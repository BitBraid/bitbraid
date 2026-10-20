//! Real proxy servers for the tests that ask for them: Squid, an MIT
//! Kerberos KDC for a Squid that signs its users in with Negotiate, and
//! Samba's winbind for one that signs them in with NTLM.
//! Each runs in a Docker container of its own on this machine's network
//! (`--network host`), listening on a port of the loopback interface, with
//! at most 1 GiB of memory; it is removed when dropped, and stops by itself
//! after an hour should the test that started it never get that far.
//!
//! Opt-in and never in ci: [`requested`] says whether the variable
//! [`VARIABLE`] is set, and a test that is not asked for says that it was
//! skipped. The image is built at the first use from a Dockerfile written
//! here (Ubuntu 24.04's `squid`, `krb5-kdc`, `samba`, `winbind`, `git` and
//! `openssl`), downloaded from Docker Hub and Ubuntu's archive, and kept by
//! Docker under a tag that changes with the Dockerfile. Containers carry
//! the label `bitbraid-test=real-proxy`, by which any left behind are
//! found.
//!
//! Squid answers as it does anywhere: its own 407 and 403 pages, a tunnel
//! (`CONNECT`) only to the ports it is told are for TLS (443, and those of
//! [`SquidOptions::tunnel_ports`]), its access log written where the test
//! reads it. The names a test gives its servers are resolved by Squid to
//! the loopback address through a hosts file of its own; no other name is
//! looked up on this machine's resolver or anywhere else. Logins:
//!
//! - Basic, checked by `basic_ncsa_auth` against a password file (the
//!   passwords hashed with `openssl passwd -apr1` inside the container);
//! - Negotiate, checked by `negotiate_kerberos_auth` against a keytab that
//!   the [`Kdc`] wrote for the proxy's service (`HTTP/<proxy's name>`);
//! - NTLM, checked by Samba's `ntlm_auth` through `winbindd` against the
//!   accounts of a standalone Samba in the same container, whose domain is
//!   its NetBIOS name, [`NTLM_DOMAIN`] (no domain controller: winbind
//!   checks a standalone server's own accounts, NTLMv2 included).
//!
//! A program can be run in a container of the image, as this user, with
//! the directories it needs at their own paths ([`run_in_ubuntu`]): the
//! command line there runs Ubuntu's git 2.43 with libcurl 8.5, which has
//! NTLM, where this machine's libcurl may have none.
//!
//! The KDC has a realm of the test's own, its users and services made with
//! `kadmin.local` when it starts. This machine's own `kinit` signs a user
//! in with a configuration and a ticket cache of the test's
//! (`KRB5_CONFIG`, `KRB5CCNAME`); neither `/etc/krb5.conf` nor the user's
//! ticket cache is read or written.
//!
//! Sources: squid.conf documentation (`http_port`, `http_access`, `acl`
//! types `port`, `method`, `dstdomain`, `proxy_auth`, `auth_param`,
//! `hosts_file`, `access_log`, `cache_log`, `umask`, `pid_filename`,
//! `shutdown_lifetime`), the manual pages of `basic_ncsa_auth`,
//! `negotiate_kerberos_auth` and Samba's `ntlm_auth`
//! (`--helper-protocol=squid-2.5-ntlmssp`), smb.conf(5) (`server role`,
//! `netbios name`, `ntlm auth`), smbpasswd(8), winbindd(8), wbinfo(1)
//! (`-p`, `-a`); Squid's native access
//! log format (wiki.squid-cache.org "SquidFaq/SquidLogs"); MIT Kerberos
//! documentation: kdc.conf(5) (`kdc_ports`, `kdc_tcp_ports`), krb5.conf(5)
//! (`default_realm`, `dns_canonicalize_hostname`, `rdns`, `[realms]`,
//! `[domain_realm]`), kdb5_util(8), kadmin(1) (`addprinc`, `ktadd`),
//! kinit(1), krb5kdc(8); docker-run(1) (`--network host`, `--memory`,
//! `--label`), docker-build(1) (a Dockerfile on standard input);
//! openssl-passwd(1).

use std::{
    io::Write as _,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, anyhow, bail};

use crate::{fnv1a64, scratch_root};

/// The variable that asks for the tests against real proxies.
pub const VARIABLE: &str = "BITBRAID_REAL_PROXY";

/// The label every container started here carries.
pub const LABEL: &str = "bitbraid-test=real-proxy";

/// What the image is made of.
const DOCKERFILE: &str = "FROM ubuntu:24.04
RUN apt-get update \\
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \\
      squid krb5-kdc krb5-admin-server krb5-user openssl ca-certificates \\
      samba winbind git \\
 && rm -rf /var/lib/apt/lists/*
";

/// The domain of the accounts NTLM is checked against: the NetBIOS name of
/// the standalone Samba beside Squid.
pub const NTLM_DOMAIN: &str = "BBPROXY";

/// How long a container lives at most, in seconds.
const LIFETIME: u64 = 3600;

/// How long a server is given to start.
const PATIENCE: Duration = Duration::from_secs(90);

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Whether the tests against real proxies were asked for; when not, it
/// says that `test` was skipped and how to ask for it.
#[allow(clippy::print_stderr)]
pub fn requested(test: &str) -> bool {
    if std::env::var_os(VARIABLE).is_some_and(|value| !value.is_empty()) {
        return true;
    }
    eprintln!("{test}: skipped, set {VARIABLE}=1 to run it against Squid in Docker");
    false
}

/// Whether this machine's MIT Kerberos programs are there.
pub fn have_kinit() -> bool {
    Command::new("klist")
        .arg("-V")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// A port of the loopback interface nothing listens on: taken and let go.
fn free_port() -> Result<u16> {
    Ok(TcpListener::bind("127.0.0.1:0")?.local_addr()?.port())
}

/// What a command printed, or why it failed, with what it said.
fn ran(cmd: &mut Command) -> Result<String> {
    let out = cmd
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("running {cmd:?}"))?;
    if !out.status.success() {
        bail!(
            "{cmd:?} failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// The image's tag, built when Docker does not have it yet, once a run.
fn image() -> Result<String> {
    static IMAGE: OnceLock<std::result::Result<String, String>> = OnceLock::new();
    IMAGE
        .get_or_init(|| build_image().map_err(|err| format!("{err:#}")))
        .clone()
        .map_err(|err| anyhow!(err))
}

fn build_image() -> Result<String> {
    let tag = format!(
        "bitbraid-test-proxy:{:016x}",
        fnv1a64(DOCKERFILE.as_bytes())
    );
    let have = Command::new("docker")
        .args(["image", "inspect", "--format", "{{.Id}}", &tag])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .context("running docker")?;
    if have.success() {
        return Ok(tag);
    }
    let mut child = Command::new("docker")
        .args(["build", "-q", "-t", &tag, "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("running docker build")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(DOCKERFILE.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        bail!(
            "docker build failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(tag)
}

/// Whether `text` can go into a shell script and a configuration file of
/// the containers as it is.
fn plain(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 200
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'/' | b'@'))
}

/// A directory of the test's own that a container can write into.
fn scratch(what: &str) -> Result<PathBuf> {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = scratch_root().join(format!("real-proxy-{what}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// A directory everybody can write into: the containers' programs run as
/// users of their own.
fn open_dir(dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o777))?;
    }
    Ok(())
}

/// A running container. It is removed when dropped.
#[derive(Debug)]
pub struct Container {
    name: String,
}

impl Container {
    /// Start the image with `mounts` (`host:container[:ro]`), `env`
    /// (`NAME=value`) and `command`, on this machine's network.
    fn start(what: &str, mounts: &[String], env: &[String], command: &[&str]) -> Result<Self> {
        let image = image()?;
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("bitbraid-test-{what}-{}-{n}", std::process::id());
        let mut cmd = Command::new("docker");
        cmd.args([
            "run",
            "-d",
            "--rm",
            "--name",
            &name,
            "--label",
            LABEL,
            "--network",
            "host",
            "--memory",
            "1g",
            "--memory-swap",
            "1g",
        ]);
        for mount in mounts {
            cmd.args(["-v", mount]);
        }
        for pair in env {
            cmd.args(["-e", pair]);
        }
        cmd.arg(&image);
        cmd.args(["timeout", &LIFETIME.to_string()]);
        cmd.args(command);
        ran(&mut cmd)?;
        Ok(Self { name })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether it still runs.
    fn running(&self) -> bool {
        ran(Command::new("docker").args(["inspect", "--format", "{{.State.Running}}", &self.name]))
            .is_ok_and(|said| said == "true")
    }

    /// What it wrote on its standard output and error.
    pub fn logs(&self) -> String {
        Command::new("docker")
            .args(["logs", &self.name])
            .stdin(Stdio::null())
            .output()
            .map(|out| {
                format!(
                    "{}{}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr)
                )
            })
            .unwrap_or_default()
    }

    /// Wait until something listens on `port`, or say why not.
    fn wait_for(&self, port: u16, also: impl Fn() -> bool, log: impl Fn() -> String) -> Result<()> {
        let started = Instant::now();
        while started.elapsed() < PATIENCE {
            if also() && TcpStream::connect(("127.0.0.1", port)).is_ok() {
                return Ok(());
            }
            if !self.running() {
                bail!(
                    "the container {} stopped: {}\n{}",
                    self.name,
                    self.logs(),
                    log()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        bail!(
            "nothing listened on port {port} of {} after {PATIENCE:?}: {}",
            self.name,
            log()
        )
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", &self.name])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Negotiate (Kerberos) as Squid checks it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Negotiate {
    /// The keytab holding the key of the proxy's service.
    pub keytab: PathBuf,
    /// The service's principal (`HTTP/<name>@REALM`).
    pub principal: String,
    /// The realm's configuration, as [`Kdc::config`] gives it.
    pub krb5_conf: String,
}

/// How a Squid is set up. It asks for a login when it is given a way to
/// check one, offering the ways in the order Negotiate, NTLM, Basic (as a
/// proxy of Active Directory's usually does); with none it lets everybody
/// through.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SquidOptions {
    /// Basic, with these users and passwords.
    pub basic: Vec<(String, String)>,
    /// Negotiate, for this service.
    pub negotiate: Option<Negotiate>,
    /// NTLM, with these users of [`NTLM_DOMAIN`] and their passwords.
    pub ntlm: Vec<(String, String)>,
    /// The ports, beside 443, that it opens a tunnel to.
    pub tunnel_ports: Vec<u16>,
    /// Hosts it refuses whoever asks, after the login (its 403 page).
    pub refused_hosts: Vec<String>,
    /// Names it resolves to the loopback address.
    pub names: Vec<String>,
}

impl SquidOptions {
    /// Whether it asks for a login.
    fn asks(&self) -> bool {
        !self.basic.is_empty() || self.negotiate.is_some() || !self.ntlm.is_empty()
    }
}

/// One line of Squid's access log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Logged {
    /// The request's method.
    pub method: String,
    /// Its target: the address, or `host:port` for a tunnel.
    pub target: String,
    /// The user Squid took it to be from, when one was given.
    pub user: Option<String>,
    /// What Squid did, as it says it (`TCP_DENIED`, `TCP_TUNNEL`, ...).
    pub result: String,
    /// The status it answered with.
    pub status: u16,
}

/// A running Squid. It is removed when dropped.
#[derive(Debug)]
pub struct Squid {
    container: Container,
    port: u16,
    dir: PathBuf,
}

/// Squid's configuration for `options`, listening on `port`.
fn squid_conf(port: u16, options: &SquidOptions) -> String {
    let mut conf = format!(
        "http_port 127.0.0.1:{port}\n\
         pid_filename none\n\
         cache deny all\n\
         umask 022\n\
         access_log stdio:/var/log/bitbraid/access.log squid\n\
         cache_log /var/log/bitbraid/cache.log\n\
         logfile_rotate 0\n\
         hosts_file /etc/bitbraid/hosts\n\
         dns_nameservers 127.0.0.1\n\
         shutdown_lifetime 0 seconds\n"
    );
    if let Some(negotiate) = &options.negotiate {
        conf.push_str(&format!(
            "auth_param negotiate program /usr/lib/squid/negotiate_kerberos_auth \
             -k /etc/bitbraid/http.keytab -s {}\n\
             auth_param negotiate children 2\n\
             auth_param negotiate keep_alive on\n",
            negotiate.principal
        ));
    }
    if !options.ntlm.is_empty() {
        conf.push_str(
            "auth_param ntlm program /usr/bin/ntlm_auth --helper-protocol=squid-2.5-ntlmssp\n\
             auth_param ntlm children 2\n\
             auth_param ntlm keep_alive on\n",
        );
    }
    if !options.basic.is_empty() {
        conf.push_str(
            "auth_param basic program /usr/lib/squid/basic_ncsa_auth /run/bitbraid/htpasswd\n\
             auth_param basic realm BitBraid test proxy\n\
             auth_param basic children 2\n",
        );
    }
    let mut ports = String::from("443");
    for port in &options.tunnel_ports {
        ports.push_str(&format!(" {port}"));
    }
    conf.push_str(&format!(
        "acl SSL_ports port {ports}\n\
         acl Safe_ports port 80 443 1025-65535\n\
         acl CONNECT method CONNECT\n\
         http_access deny !Safe_ports\n\
         http_access deny CONNECT !SSL_ports\n"
    ));
    if options.asks() {
        conf.push_str(
            "acl authenticated proxy_auth REQUIRED\n\
             http_access deny !authenticated\n",
        );
    }
    if !options.refused_hosts.is_empty() {
        conf.push_str(&format!(
            "acl refused dstdomain {}\nhttp_access deny refused\n",
            options.refused_hosts.join(" ")
        ));
    }
    conf.push_str("http_access allow all\n");
    conf
}

/// What the container runs: the password file made, the accounts of
/// Samba and its winbind for NTLM, then Squid. Samba keeps its logs inside
/// the container: it makes directories of root's beside them (`cores`),
/// which the test could not remove from the directory it reads Squid's
/// logs in.
const SQUID_START: &str = "set -e
mkdir -p /run/bitbraid
: > /run/bitbraid/htpasswd
if [ -f /etc/bitbraid/logins ]; then
  while IFS=' ' read -r user password; do
    printf '%s:%s\\n' \"$user\" \"$(openssl passwd -apr1 \"$password\")\" >> /run/bitbraid/htpasswd
  done < /etc/bitbraid/logins
fi
chmod 644 /run/bitbraid/htpasswd
if [ -f /etc/bitbraid/ntlm-logins ]; then
  cat > /etc/samba/smb.conf <<EOF
[global]
  workgroup = BITBRAID
  netbios name = $NTLM_DOMAIN
  server role = standalone server
  passdb backend = tdbsam
  ntlm auth = yes
  log file = /var/log/samba/log.%m
EOF
  while IFS=' ' read -r user password; do
    useradd -M -s /usr/sbin/nologin \"$user\"
    printf '%s\\n%s\\n' \"$password\" \"$password\" | smbpasswd -s -a \"$user\" >/dev/null
  done < /etc/bitbraid/ntlm-logins
  winbindd -D
  i=0
  until wbinfo -p >/dev/null 2>&1; do
    i=$((i + 1)); [ $i -gt 100 ] && exit 1; sleep 0.1
  done
  usermod -a -G winbindd_priv proxy
fi
exec squid -N -f /etc/bitbraid/squid.conf
";

impl Squid {
    /// Start one set up by `options`.
    pub fn start(options: &SquidOptions) -> Result<Self> {
        let dir = scratch("squid")?;
        let port = free_port()?;
        let conf = dir.join("conf");
        std::fs::create_dir_all(&conf)?;
        let log = dir.join("log");
        open_dir(&log)?;
        let mut hosts = String::new();
        for name in &options.names {
            if !plain(name) {
                bail!("the name {name:?} cannot be given to Squid");
            }
            hosts.push_str(&format!("127.0.0.1 {name}\n"));
        }
        for host in &options.refused_hosts {
            if !plain(host) {
                bail!("the host {host:?} cannot be given to Squid");
            }
        }
        std::fs::write(conf.join("hosts"), hosts)?;
        std::fs::write(conf.join("squid.conf"), squid_conf(port, options))?;
        std::fs::write(conf.join("start.sh"), SQUID_START)?;
        let mut env = Vec::new();
        if !options.ntlm.is_empty() {
            let mut lines = String::new();
            for (user, password) in &options.ntlm {
                if !plain(user) || !plain(password) || user.contains(['/', '@']) {
                    bail!("the NTLM login {user:?} cannot be given to Squid");
                }
                lines.push_str(&format!("{user} {password}\n"));
            }
            std::fs::write(conf.join("ntlm-logins"), lines)?;
            env.push(format!("NTLM_DOMAIN={NTLM_DOMAIN}"));
        }
        if !options.basic.is_empty() {
            let mut lines = String::new();
            for (user, password) in &options.basic {
                if !plain(user) || !plain(password) {
                    bail!("the login {user:?} cannot be given to Squid");
                }
                lines.push_str(&format!("{user} {password}\n"));
            }
            std::fs::write(conf.join("logins"), lines)?;
        }
        if let Some(Negotiate {
            keytab,
            principal,
            krb5_conf,
        }) = &options.negotiate
        {
            if !plain(principal) {
                bail!("the principal {principal:?} cannot be given to Squid");
            }
            std::fs::copy(keytab, conf.join("http.keytab"))
                .with_context(|| format!("copying {}", keytab.display()))?;
            std::fs::write(conf.join("krb5.conf"), krb5_conf)?;
            env.push("KRB5_CONFIG=/etc/bitbraid/krb5.conf".to_owned());
            env.push("KRB5_KTNAME=/etc/bitbraid/http.keytab".to_owned());
        }
        let mounts = [
            format!("{}:/etc/bitbraid:ro", conf.display()),
            format!("{}:/var/log/bitbraid", log.display()),
        ];
        let container =
            Container::start("squid", &mounts, &env, &["sh", "/etc/bitbraid/start.sh"])?;
        let cache_log = log.join("cache.log");
        container.wait_for(
            port,
            || true,
            || std::fs::read_to_string(&cache_log).unwrap_or_default(),
        )?;
        Ok(Self {
            container,
            port,
            dir,
        })
    }

    /// The port it listens on, of the loopback interface.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Its address: `http://127.0.0.1:<port>`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Its address with a user name, and a password when one is given.
    pub fn url_with(&self, user: &str, password: Option<&str>) -> String {
        match password {
            Some(password) => format!("http://{user}:{password}@127.0.0.1:{}", self.port),
            None => format!("http://{user}@127.0.0.1:{}", self.port),
        }
    }

    pub fn container(&self) -> &Container {
        &self.container
    }

    /// Every request in its access log so far, in order; not the
    /// connections that sent none (the look made at the start whether it
    /// listens). Squid writes a request down when it is done with it,
    /// which can be after the client has its answer: the log is read when
    /// it has not changed for a fifth of a second, or after five seconds.
    pub fn access_log(&self) -> Vec<Logged> {
        let file = self.dir.join("log/access.log");
        let read = || std::fs::read_to_string(&file).unwrap_or_default();
        let started = Instant::now();
        let mut text = read();
        while started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(200));
            let again = read();
            if again == text {
                break;
            }
            text = again;
        }
        text.lines()
            .filter_map(logged)
            .filter(|line| line.method != "-")
            .collect()
    }

    /// What it wrote in its own log.
    pub fn cache_log(&self) -> String {
        std::fs::read_to_string(self.dir.join("log/cache.log")).unwrap_or_default()
    }
}

impl Drop for Squid {
    fn drop(&mut self) {
        // The container first, then what it wrote.
        let _ = Command::new("docker")
            .args(["rm", "-f", self.container.name()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A line of Squid's native access log: time, elapsed, client,
/// result/status, bytes, method, target, user, hierarchy, type.
fn logged(line: &str) -> Option<Logged> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let (result, status) = fields.get(3)?.split_once('/')?;
    let user = fields.get(7).copied().filter(|user| *user != "-");
    Some(Logged {
        method: (*fields.get(5)?).to_owned(),
        target: (*fields.get(6)?).to_owned(),
        user: user.map(str::to_owned),
        result: result.to_owned(),
        status: status.parse().ok()?,
    })
}

/// Run `program` with `args` in a container of the image (Ubuntu 24.04,
/// its git and libcurl), on this machine's network, as the user and group
/// that own the first of `dirs`, which is the working directory; each of
/// `dirs` is there at its own path, and `env` is all of the environment
/// beside `PATH`. What it printed and how it ended.
pub fn run_in_ubuntu(
    program: &Path,
    args: &[&str],
    dirs: &[&Path],
    env: &[(&str, &std::ffi::OsStr)],
) -> Result<std::process::Output> {
    let image = image()?;
    let first = dirs.first().context("a working directory")?;
    #[cfg(unix)]
    let owner = {
        use std::os::unix::fs::MetadataExt as _;
        let meta = std::fs::metadata(first)?;
        format!("{}:{}", meta.uid(), meta.gid())
    };
    #[cfg(not(unix))]
    let owner = String::from("0:0");
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let name = format!("bitbraid-test-ubuntu-{}-{n}", std::process::id());
    let mut cmd = Command::new("docker");
    cmd.args([
        "run",
        "--rm",
        "--name",
        &name,
        "--label",
        LABEL,
        "--network",
        "host",
        "--memory",
        "1g",
        "--memory-swap",
        "1g",
        "--user",
        &owner,
        "-w",
    ]);
    cmd.arg(first);
    for dir in dirs {
        let mut mount = dir.as_os_str().to_owned();
        mount.push(":");
        mount.push(dir.as_os_str());
        cmd.arg("-v").arg(mount);
    }
    for (name, value) in env {
        let mut pair = std::ffi::OsString::from(name);
        pair.push("=");
        pair.push(value);
        cmd.arg("-e").arg(pair);
    }
    cmd.arg(&image)
        .args(["timeout", &LIFETIME.to_string()])
        .arg(program)
        .args(args)
        .stdin(Stdio::null());
    cmd.output().with_context(|| format!("running {cmd:?}"))
}

/// A running KDC. It is removed when dropped.
#[derive(Debug)]
pub struct Kdc {
    container: Container,
    port: u16,
    dir: PathBuf,
    realm: String,
}

impl Kdc {
    /// Start one for `realm` (capitals), with `users` (name and password)
    /// and `services` (`HTTP/host`, their keys written to [`Kdc::keytab`]).
    pub fn start(realm: &str, users: &[(&str, &str)], services: &[&str]) -> Result<Self> {
        if !plain(realm) {
            bail!("the realm {realm:?} cannot be made");
        }
        let dir = scratch("kdc")?;
        let out = dir.join("out");
        open_dir(&out)?;
        let port = free_port()?;
        let mut script = format!(
            "set -e
cat > /etc/krb5.conf <<EOF
[libdefaults]
  default_realm = {realm}
  dns_lookup_kdc = false
  dns_lookup_realm = false
[realms]
  {realm} = {{
    kdc = 127.0.0.1:{port}
  }}
EOF
cat > /etc/krb5kdc/kdc.conf <<EOF
[kdcdefaults]
  kdc_ports = {port}
  kdc_tcp_ports = {port}
[realms]
  {realm} = {{
    database_name = /var/lib/krb5kdc/principal
    key_stash_file = /etc/krb5kdc/stash
    max_life = 1h
    supported_enctypes = aes256-cts-hmac-sha1-96:normal aes128-cts-hmac-sha1-96:normal
  }}
[logging]
  kdc = FILE:/out/kdc.log
EOF
kdb5_util create -s -r {realm} -P bitbraid-master-key >/dev/null
"
        );
        for (user, password) in users {
            if !plain(user) || !plain(password) {
                bail!("the user {user:?} cannot be made");
            }
            script.push_str(&format!(
                "kadmin.local -q 'addprinc -pw {password} {user}' >/dev/null\n"
            ));
        }
        for service in services {
            if !plain(service) {
                bail!("the service {service:?} cannot be made");
            }
            script.push_str(&format!(
                "kadmin.local -q 'addprinc -randkey {service}' >/dev/null\n\
                 kadmin.local -q 'ktadd -k /out/services.keytab {service}' >/dev/null\n"
            ));
        }
        script.push_str(
            "chmod 644 /out/services.keytab /out/kdc.log 2>/dev/null || true\n\
             touch /out/ready\n\
             exec krb5kdc -n\n",
        );
        std::fs::write(dir.join("start.sh"), script)?;
        let mounts = [
            format!("{}:/start.sh:ro", dir.join("start.sh").display()),
            format!("{}:/out", out.display()),
        ];
        let container = Container::start("kdc", &mounts, &[], &["sh", "/start.sh"])?;
        let ready = out.join("ready");
        container.wait_for(port, || ready.exists(), String::new)?;
        Ok(Self {
            container,
            port,
            dir,
            realm: realm.to_owned(),
        })
    }

    pub fn realm(&self) -> &str {
        &self.realm
    }

    /// The port it listens on, of the loopback interface.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The keytab holding the keys of the services.
    pub fn keytab(&self) -> PathBuf {
        self.dir.join("out/services.keytab")
    }

    pub fn container(&self) -> &Container {
        &self.container
    }

    /// A `krb5.conf` for the realm: the KDC by its address, no name looked
    /// up in DNS, and a service's host name taken as it is written (no
    /// canonical name asked of DNS, no reverse lookup), every name under
    /// the realm's domain (its name in small letters) in the realm.
    pub fn config(&self) -> String {
        let realm = &self.realm;
        let domain = realm.to_ascii_lowercase();
        format!(
            "[libdefaults]
  default_realm = {realm}
  dns_lookup_kdc = false
  dns_lookup_realm = false
  dns_canonicalize_hostname = false
  rdns = false
[realms]
  {realm} = {{
    kdc = 127.0.0.1:{port}
  }}
[domain_realm]
  .{domain} = {realm}
  {domain} = {realm}
",
            port = self.port
        )
    }

    /// Sign `user` in with `password` by this machine's `kinit`, with the
    /// configuration file `config` and the ticket cache `cache`; what
    /// `kinit` said when it failed.
    pub fn kinit(
        &self,
        config: &Path,
        cache: &Path,
        user: &str,
        password: &str,
    ) -> std::result::Result<(), String> {
        let mut child = Command::new("kinit")
            .arg(format!("{user}@{}", self.realm))
            .env("KRB5_CONFIG", config)
            .env("KRB5CCNAME", format!("FILE:{}", cache.display()))
            .env_remove("KRB5_TRACE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| format!("kinit could not be run: {err}"))?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(format!("{password}\n").as_bytes());
        }
        let out = child
            .wait_with_output()
            .map_err(|err| format!("kinit: {err}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
        }
    }
}

impl Drop for Kdc {
    fn drop(&mut self) {
        let _ = Command::new("docker")
            .args(["rm", "-f", self.container.name()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_of_the_access_log_is_read() {
        let line = "1792232452.164     23 127.0.0.1 TCP_DENIED/407 3490 GET \
                    http://git.example.test:18080/ carol HIER_NONE/- text/html";
        assert_eq!(
            logged(line),
            Some(Logged {
                method: "GET".to_owned(),
                target: "http://git.example.test:18080/".to_owned(),
                user: Some("carol".to_owned()),
                result: "TCP_DENIED".to_owned(),
                status: 407,
            })
        );
        let tunnel = "1792232452.209 0 127.0.0.1 TCP_TUNNEL/503 0 CONNECT \
                      git.example.test:18443 - HIER_DIRECT/127.0.0.1 -";
        let read = logged(tunnel).unwrap();
        assert_eq!((read.user, read.status), (None, 503));
        assert_eq!(logged(""), None);
        assert_eq!(logged("a b c d"), None);
        assert_eq!(logged("a b c X/y e f g"), None);
    }

    #[test]
    fn the_configuration_follows_the_options() {
        let conf = squid_conf(
            3128,
            &SquidOptions {
                basic: vec![("carol".into(), "secret".into())],
                tunnel_ports: vec![8443, 9443],
                refused_hosts: vec!["refused.example.test".into()],
                ..SquidOptions::default()
            },
        );
        assert!(conf.contains("http_port 127.0.0.1:3128\n"));
        assert!(conf.contains("acl SSL_ports port 443 8443 9443\n"));
        assert!(conf.contains("basic_ncsa_auth"));
        // The login before the refusal, so that a refused host is refused
        // to a user who signed in, with Squid's 403.
        let login = conf.find("http_access deny !authenticated").unwrap();
        let refused = conf.find("http_access deny refused").unwrap();
        assert!(login < refused);
        assert!(conf.ends_with("http_access allow all\n"));
        let open = squid_conf(1, &SquidOptions::default());
        assert!(!open.contains("auth_param") && !open.contains("authenticated"));
        // Negotiate, then NTLM, then Basic, as they are offered.
        let all = squid_conf(
            1,
            &SquidOptions {
                basic: vec![("carol".into(), "secret".into())],
                negotiate: Some(Negotiate {
                    keytab: PathBuf::from("k"),
                    principal: "HTTP/p.bitbraid.test@BITBRAID.TEST".into(),
                    krb5_conf: String::new(),
                }),
                ntlm: vec![("dave".into(), "secret".into())],
                ..SquidOptions::default()
            },
        );
        let at = |scheme: &str| all.find(&format!("auth_param {scheme} program")).unwrap();
        assert!(at("negotiate") < at("ntlm") && at("ntlm") < at("basic"));
        assert!(all.contains("ntlm_auth --helper-protocol=squid-2.5-ntlmssp"));
        assert!(all.contains("http_access deny !authenticated"));
        assert!(plain("HTTP/proxy.bitbraid.test@BITBRAID.TEST"));
        assert!(!plain("a b") && !plain("a'b") && !plain("") && !plain("$(x)"));
    }
}
