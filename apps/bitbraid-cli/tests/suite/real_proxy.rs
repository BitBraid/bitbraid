//! Fetch, push and clone through the engine with a real proxy on the way:
//! Squid 6 in Docker (`bitbraid_testkit::real_proxy`), against the
//! smart-HTTP server of the tests, plain and over HTTPS through a tunnel
//! with a certificate from an authority of the test's own.
//!
//! - Squid with Basic, its logins checked by `basic_ncsa_auth`: the
//!   proxy's password asked through the application's askpass dialog, a
//!   wrong one, no user at all, Squid's own 407 and 403 pages, a tunnel
//!   to a port Squid does not open tunnels to, and a server Squid cannot
//!   reach; a clone through the proxy the variables name.
//! - Squid with Negotiate, its tickets checked by `negotiate_kerberos_auth`
//!   with a keytab of an MIT KDC run beside it, for a realm of the test's
//!   own: this machine's `kinit` with a configuration and a ticket cache of
//!   the test's; git told `http.proxyAuthMethod=negotiate`; no ticket;
//!   git's default way (`anyauth`), which does not offer a ticket; the
//!   proxy named by its address instead of its service's name.
//! - Squid with NTLM, checked by Samba's `ntlm_auth` against the accounts
//!   of a standalone Samba beside it (the user `BBPROXY\carol`): this
//!   machine's git, whose curl may have no NTLM, said so; the command line
//!   run in Ubuntu 24.04, whose git 2.43 has libcurl 8.5 with NTLM, the
//!   proxy's password asked through this test's dialog from inside the
//!   container: fetch and push over plain HTTP, a fetch over HTTPS, a wrong
//!   password, and git's own way with the user named.
//!
//! The proxy's name for Kerberos, `proxy.bitbraid.test`, resolves nowhere
//! on this machine: git is told where it is with `http.curloptResolve`
//! (curl's `--resolve`), which curl applies to a proxy's name as to any
//! other, so nothing needs root or a change to `/etc/hosts`. The servers
//! go by `git.example.test`, which Squid alone resolves, to the loopback
//! address, through a hosts file of its own.
//!
//! Opt-in and never in ci: run with `--ignored` and
//! `BITBRAID_REAL_PROXY=1`, with Docker that runs containers without
//! sudo; the image is built at the first run. Otherwise each test says
//! that it was skipped:
//!
//! ```sh
//! BITBRAID_REAL_PROXY=1 cargo test -p bitbraid-cli --test suite real_proxy:: -- --ignored
//! ```
//!
//! A test that needs a ticket runs the command line as a program with
//! `KRB5_CONFIG` and `KRB5CCNAME` of its own, the engine inside it, so
//! that nothing of the developer's Kerberos is read.
//!
//! Sources: git-config(1) (`http.proxy`, `http.proxyAuthMethod`,
//! `http.curloptResolve`, `http.sslCAInfo`), git(1) (`GIT_CONFIG_COUNT`,
//! `GIT_SSL_CAINFO`), curl(1) (`--resolve`, `--proxy-negotiate`,
//! "ENVIRONMENT"), squid.conf documentation (`auth_param`, `acl`,
//! `http_access`), kinit(1), krb5.conf(5), RFC 4559 (Negotiate over
//! HTTP), RFC 9110 §15.5.8 (407).
#![allow(clippy::print_stderr)]

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bitbraid_git::askpass::Kind;
use bitbraid_testkit::{
    Authority, Git, SmartHttp, SmartHttpOptions, fixtures_out_dir,
    real_proxy::{self, Kdc, Logged, NTLM_DOMAIN, Negotiate, Squid, SquidOptions},
    scratch_root,
    tls::ServerConfig,
};
use serde_json::json;

use crate::{
    ASKPASS_SERVER as ONE,
    transport_auth::{Asking, Site},
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The name the servers go by: Squid alone resolves it.
const NAME: &str = "git.example.test";
/// A host Squid refuses to everybody, with its 403 page.
const REFUSED: &str = "refused.example.test";
const PROXY_USER: &str = "carol";
const PROXY_PASSWORD: &str = "proxy-secret";

const REALM: &str = "BITBRAID.TEST";
/// The proxy's name, under which its service is known to the realm.
const PROXY_NAME: &str = "proxy.bitbraid.test";
const KERBEROS_USER: &str = "alice";
const KERBEROS_PASSWORD: &str = "alice-secret";

/// Every variable that names a proxy, in both cases.
const PROXY_VARIABLES: &[&str] = &[
    "http_proxy",
    "HTTP_PROXY",
    "https_proxy",
    "HTTPS_PROXY",
    "all_proxy",
    "ALL_PROXY",
    "no_proxy",
    "NO_PROXY",
];

fn scratch(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = scratch_root().join(format!("real-proxy-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn served_name(site: &Site) -> String {
    site.bare
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

fn server(site: &Site, options: SmartHttpOptions) -> SmartHttp {
    SmartHttp::start(site.bare.parent().unwrap(), options)
        .unwrap()
        .expect("git http-backend is installed")
}

fn authority(dir: &Path, names: &[&str]) -> (PathBuf, Arc<ServerConfig>) {
    let authority = Authority::new().unwrap();
    let file = dir.join("authority.pem");
    authority.write_pem(&file).unwrap();
    (file, authority.server_config(names).unwrap())
}

fn config(site: &Site, key: &str, value: &str) {
    site.git().run(["config", key, value]).unwrap();
}

/// What Squid logged since `from`.
fn logged_since(squid: &Squid, from: usize) -> Vec<Logged> {
    squid.access_log().into_iter().skip(from).collect()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// What the application says before git when a proxy turns git away
/// (`bitbraid_git::remote`), in part.
const SAID_LOGIN: &str = "The proxy refused the request, so nothing was sent to the server: \
                          it wants to know who is asking";
const SAID_TICKET: &str = "git was told to sign in to it with a Kerberos ticket";
const SAID_TUNNEL: &str = "The proxy did not open a connection to the server";
const SAID_NTLM: &str = "git was told to sign in to it with NTLM";

/// Whether the test may run: asked for, and git has an http-backend.
fn may_run(test: &str) -> bool {
    if !real_proxy::requested(test) {
        return false;
    }
    if std::env::var_os("GIT_ASKPASS").is_some() {
        eprintln!("{test}: skipped, GIT_ASKPASS is set and would be asked instead");
        return false;
    }
    true
}

// ------------------------------------------------------------- Basic

#[test]
#[ignore = "real proxy: needs BITBRAID_REAL_PROXY and Docker"]
fn squid_with_basic_carries_fetch_and_push_and_its_password_comes_from_the_dialog() {
    if !may_run("squid with basic") {
        return;
    }
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = scratch("basic");
    let mut site = Site::new();
    let plain = server(&site, SmartHttpOptions::default());
    let (authority, tls) = authority(&dir, &[NAME, REFUSED]);
    let secure = server(
        &site,
        SmartHttpOptions {
            login: None,
            tls: Some(tls.clone()),
        },
    );
    // A second TLS server, on a port Squid opens no tunnel to.
    let shut_out = server(
        &site,
        SmartHttpOptions {
            login: None,
            tls: Some(tls),
        },
    );
    let squid = Squid::start(&SquidOptions {
        basic: vec![(PROXY_USER.to_owned(), PROXY_PASSWORD.to_owned())],
        tunnel_ports: vec![secure.port()],
        refused_hosts: vec![REFUSED.to_owned()],
        names: vec![NAME.to_owned(), REFUSED.to_owned()],
        ..SquidOptions::default()
    })
    .unwrap();
    let repo = served_name(&site);
    let named = squid.url_with(PROXY_USER, None);
    site.git()
        .run(["remote", "add", "origin", &plain.url(NAME, &repo)])
        .unwrap();
    config(&site, "http.proxy", &named);
    config(&site, "http.sslCAInfo", authority.to_str().unwrap());
    let upstream = site.upstream_commit("upstream.txt");

    // Plain HTTP: the password of the user the proxy's address names,
    // asked through the dialog, once for each operation.
    let asking = Asking::with(|kind, prompt| {
        Some(match kind {
            Kind::Secret if prompt.starts_with("Password for") => PROXY_PASSWORD.to_owned(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    });
    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}\n{}", squid.cache_log()));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    site.git()
        .run(["merge", "-q", "--ff-only", "origin/main"])
        .unwrap();
    let local = site.local_commit("local.txt");
    site.run(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
    )
    .unwrap_or_else(|err| panic!("push: {err}"));
    assert_eq!(site.served().rev_parse("refs/heads/main").unwrap(), local);
    let asked = asking.asked();
    assert_eq!(
        asked.len(),
        2,
        "once for the fetch, once for the push: {asked:?}"
    );
    assert!(
        asked
            .iter()
            .all(|(_, prompt)| prompt.starts_with(&format!("Password for '{named}'"))),
        "{asked:?}"
    );
    // Every request went through Squid by the server's name: first
    // without a login (curl does not send one before it is asked), then
    // with carol's, which Squid let through.
    let log = squid.access_log();
    let prefix = format!("http://{NAME}:{}/", plain.port());
    assert!(
        log.iter().all(|l| l.target.starts_with(&prefix)),
        "{log:#?}"
    );
    assert!(
        log.iter().all(|l| match l.user.as_deref() {
            Some(user) => user == PROXY_USER && l.status == 200,
            None => l.result == "TCP_DENIED" && l.status == 407,
        }),
        "{log:#?}"
    );
    assert!(
        log.iter()
            .any(|l| l.method == "POST" && l.target.ends_with("/git-receive-pack")),
        "{log:#?}"
    );
    drop(asking);

    // A wrong password: Squid's 407.
    let before = squid.access_log().len();
    let asking = Asking::with(|_, _| Some("not-the-password".to_owned()));
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("the password is wrong");
    assert!(err.contains(SAID_LOGIN) && err.contains("407"), "{err}");
    assert!(!err.contains("not-the-password"), "{err}");
    assert_eq!(asking.asked().len(), 1, "{:?}", asking.asked());
    // Squid's own 407, for carol's wrong password.
    let log = logged_since(&squid, before);
    assert!(
        log.iter()
            .any(|l| l.status == 407 && l.user.as_deref() == Some(PROXY_USER)),
        "{log:#?}"
    );
    assert!(log.iter().all(|l| l.status == 407), "{log:#?}");
    drop(asking);

    // No user in the proxy's address: nothing to ask, Squid's 407.
    config(&site, "http.proxy", &squid.url());
    let before = squid.access_log().len();
    let asking = Asking::with(|_, prompt| panic!("nothing should be asked: {prompt}"));
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("no login");
    assert!(err.contains(SAID_LOGIN) && err.contains("407"), "{err}");
    let log = logged_since(&squid, before);
    assert!(!log.is_empty());
    assert!(
        log.iter().all(|l| l.status == 407 && l.user.is_none()),
        "{log:#?}"
    );
    drop(asking);

    // HTTPS through a tunnel Squid opens, with the password asked.
    config(&site, "http.proxy", &named);
    site.git()
        .run(["remote", "set-url", "origin", &secure.url(NAME, &repo)])
        .unwrap();
    let upstream = site.upstream_commit("upstream-2.txt");
    let asking = Asking::with(|_, _| Some(PROXY_PASSWORD.to_owned()));
    let before = squid.access_log().len();
    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch over https: {err}"));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    site.git()
        .run(["merge", "-q", "--ff-only", "origin/main"])
        .unwrap();
    let local = site.local_commit("local-2.txt");
    site.run(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
    )
    .unwrap_or_else(|err| panic!("push over https: {err}"));
    assert_eq!(site.served().rev_parse("refs/heads/main").unwrap(), local);
    assert_eq!(asking.asked().len(), 2, "{:?}", asking.asked());
    // Tunnels to the server's name and port, carol's let through; Squid
    // saw nothing of what went through them.
    let log = logged_since(&squid, before);
    let tunnel = format!("{NAME}:{}", secure.port());
    assert!(
        log.iter()
            .all(|l| l.method == "CONNECT" && l.target == tunnel),
        "{log:#?}"
    );
    assert!(
        log.iter().any(|l| l.result == "TCP_TUNNEL"
            && l.status == 200
            && l.user.as_deref() == Some(PROXY_USER)),
        "{log:#?}"
    );

    // A tunnel to a port Squid opens none to: its 403.
    site.git()
        .run(["remote", "set-url", "origin", &shut_out.url(NAME, &repo)])
        .unwrap();
    let before = squid.access_log().len();
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("no tunnel to that port");
    assert!(
        err.contains(SAID_TUNNEL) && err.contains("response 403"),
        "{err}"
    );
    // Refused before the login is asked for: Squid opens tunnels to the
    // ports of TLS alone (SSL_ports).
    let log = logged_since(&squid, before);
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(
        (
            log[0].method.as_str(),
            log[0].result.as_str(),
            log[0].status
        ),
        ("CONNECT", "TCP_DENIED", 403)
    );
    assert_eq!(log[0].target, format!("{NAME}:{}", shut_out.port()));

    // A host Squid refuses, over plain HTTP: its 403 page, in the
    // server's place.
    site.git()
        .run(["remote", "set-url", "origin", &plain.url(REFUSED, &repo)])
        .unwrap();
    let before = squid.access_log().len();
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("refused host");
    // Over plain HTTP the proxy answers in the server's place, and git's
    // words do not tell the two apart: said as git says it.
    assert!(
        err.contains("The requested URL returned error: 403"),
        "{err}"
    );
    assert!(!err.contains("The proxy"), "{err}");
    let log = logged_since(&squid, before);
    assert!(
        log.iter().any(|l| l.status == 403
            && l.user.as_deref() == Some(PROXY_USER)
            && l.target.starts_with(&format!("http://{REFUSED}:"))),
        "{log:#?}"
    );

    // The same over HTTPS: the tunnel refused.
    site.git()
        .run(["remote", "set-url", "origin", &secure.url(REFUSED, &repo)])
        .unwrap();
    let before = squid.access_log().len();
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("refused host");
    assert!(
        err.contains(SAID_TUNNEL) && err.contains("response 403"),
        "{err}"
    );
    let log = logged_since(&squid, before);
    assert!(
        log.iter().any(|l| l.method == "CONNECT"
            && l.status == 403
            && l.user.as_deref() == Some(PROXY_USER)),
        "{log:#?}"
    );

    // A server that is not there, over both.
    let gone = secure.port();
    drop(secure);
    site.git()
        .run([
            "remote",
            "set-url",
            "origin",
            &format!("https://{NAME}:{gone}/{repo}"),
        ])
        .unwrap();
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("no server");
    assert!(
        err.contains(SAID_TUNNEL) && err.contains("response 503"),
        "{err}"
    );
    let gone = plain.port();
    drop(plain);
    site.git()
        .run([
            "remote",
            "set-url",
            "origin",
            &format!("http://{NAME}:{gone}/{repo}"),
        ])
        .unwrap();
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("no server");
    assert!(
        err.contains("The requested URL returned error: 503"),
        "{err}"
    );
    drop(asking);
    let _ = std::fs::remove_dir_all(&dir);
}

// --------------------------------- the command line, as a program

/// The command line as a program in `dir`, seeing the fixtures' home and
/// configuration, no proxy variable of whoever runs the tests, and
/// Kerberos as `kerberos` says (configuration and ticket cache).
fn cli(dir: &Path, kerberos: Option<(&Path, &Path)>) -> Command {
    let home = fixtures_out_dir().join(".home");
    Git::new(fixtures_out_dir(), &home).ensure_home().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bitbraid-cli"));
    cmd.current_dir(dir)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("BITBRAID_CONFIG_DIR", dir.join("bitbraid-config"))
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .env_remove("GIT_SSL_CAINFO")
        .env_remove("GIT_HTTP_PROXY_AUTHMETHOD")
        .env_remove("KRB5_TRACE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in PROXY_VARIABLES {
        cmd.env_remove(name);
    }
    if let Some((config, cache)) = kerberos {
        cmd.env("KRB5_CONFIG", config)
            .env("KRB5CCNAME", format!("FILE:{}", cache.display()));
    }
    cmd
}

/// `id` with `params` run by the command line in the repository `repo`.
fn run_cli(
    repo: &Path,
    id: &str,
    params: serde_json::Value,
    kerberos: Option<(&Path, &Path)>,
    env: &[(&str, &str)],
) -> Output {
    let mut cmd = cli(repo, kerberos);
    cmd.args([
        "--repo",
        repo.to_str().unwrap(),
        "run",
        id,
        "--params",
        &params.to_string(),
    ]);
    for (name, value) in env {
        cmd.env(name, value);
    }
    cmd.output().unwrap()
}

#[test]
#[ignore = "real proxy: needs BITBRAID_REAL_PROXY and Docker"]
fn squid_with_basic_carries_a_clone_named_by_the_variables() {
    if !may_run("a clone through squid by the variables") {
        return;
    }
    let dir = scratch("clone");
    let site = Site::new();
    let plain = server(&site, SmartHttpOptions::default());
    let (authority, tls) = authority(&dir, &[NAME]);
    let secure = server(
        &site,
        SmartHttpOptions {
            login: None,
            tls: Some(tls),
        },
    );
    let squid = Squid::start(&SquidOptions {
        basic: vec![(PROXY_USER.to_owned(), PROXY_PASSWORD.to_owned())],
        tunnel_ports: vec![secure.port()],
        names: vec![NAME.to_owned()],
        ..SquidOptions::default()
    })
    .unwrap();
    let upstream = site.served().rev_parse("refs/heads/main").unwrap();
    let repo = served_name(&site);
    let with_login = squid.url_with(PROXY_USER, Some(PROXY_PASSWORD));
    for (variable, url, folder) in [
        ("http_proxy", plain.url(NAME, &repo), "plain"),
        ("HTTPS_PROXY", secure.url(NAME, &repo), "secure"),
    ] {
        let into = dir.join(folder);
        let params = json!({ "url": url, "path": into });
        let mut cmd = cli(&dir, None);
        cmd.args(["run", "repo.clone", "--params", &params.to_string()])
            .env(variable, &with_login)
            .env("GIT_SSL_CAINFO", &authority);
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "{variable}: {}", text(&out.stderr));
        let cloned = Git::new(&into, fixtures_out_dir().join(".home"));
        assert_eq!(cloned.rev_parse("HEAD").unwrap(), upstream);
    }
    // The login of the variable's address went with every request Squid
    // let through, a plain one and a tunnel.
    let log = squid.access_log();
    assert!(
        log.iter()
            .filter(|l| l.status == 200)
            .all(|l| l.user.as_deref() == Some(PROXY_USER)),
        "{log:#?}"
    );
    assert!(log.iter().any(|l| l.method == "GET" && l.status == 200));
    assert!(log.iter().any(|l| l.method == "CONNECT" && l.status == 200));
    let _ = std::fs::remove_dir_all(&dir);
}

// -------------------------------------------------------------- NTLM

/// Whether this machine's curl has NTLM (`curl -V` lists its features).
fn curl_has_ntlm() -> bool {
    Command::new("curl").arg("-V").output().is_ok_and(|out| {
        text(&out.stdout)
            .lines()
            .any(|line| line.starts_with("Features:") && line.split(' ').any(|f| f == "NTLM"))
    })
}

#[test]
#[ignore = "real proxy: needs BITBRAID_REAL_PROXY and Docker"]
fn squid_with_ntlm_lets_a_git_whose_curl_has_it_through() {
    if !may_run("squid with ntlm") {
        return;
    }
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = scratch("ntlm");
    let mut site = Site::new();
    let plain = server(&site, SmartHttpOptions::default());
    let (authority, tls) = authority(&dir, &[NAME]);
    let secure = server(
        &site,
        SmartHttpOptions {
            login: None,
            tls: Some(tls),
        },
    );
    let squid = Squid::start(&SquidOptions {
        ntlm: vec![(PROXY_USER.to_owned(), PROXY_PASSWORD.to_owned())],
        tunnel_ports: vec![secure.port()],
        names: vec![NAME.to_owned()],
        ..SquidOptions::default()
    })
    .unwrap();
    let repo = served_name(&site);
    // DOMAIN\user, the backslash written %5C in the address.
    let named = format!(
        "http://{NTLM_DOMAIN}%5C{PROXY_USER}@127.0.0.1:{}",
        squid.port()
    );
    site.git()
        .run(["remote", "add", "origin", &plain.url(NAME, &repo)])
        .unwrap();
    config(&site, "http.proxy", &named);
    config(&site, "http.proxyAuthMethod", "ntlm");
    config(&site, "http.sslCAInfo", authority.to_str().unwrap());
    let upstream = site.upstream_commit("upstream.txt");
    let asked_for = format!("Password for '{named}'");

    // This machine's git, in the engine here.
    let asking = Asking::with(|_, _| Some(PROXY_PASSWORD.to_owned()));
    let fetched = site.run("remote.fetch", json!({ "remote": "origin" }));
    if curl_has_ntlm() {
        fetched.unwrap_or_else(|err| panic!("fetch with this machine's git: {err}"));
    } else {
        // Its curl has no NTLM: git asks the password all the same, sends
        // nothing Squid takes, and is told why that may be.
        let err = fetched.expect_err("this machine's curl has no NTLM");
        assert!(
            err.contains(SAID_NTLM) && err.contains("curl -V") && err.contains("407"),
            "{err}"
        );
        assert_eq!(asking.count("Password for"), 1, "{:?}", asking.asked());
        assert_eq!(site.tracking(), None);
    }
    drop(asking);

    // The command line in Ubuntu: its git's curl has NTLM. Its git asks
    // the password of this test's dialog, from inside the container.
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap();
    let gitconfig = home.join(".gitconfig");
    std::fs::write(&gitconfig, "").unwrap();
    let program = PathBuf::from(env!("CARGO_BIN_EXE_bitbraid-cli"));
    let path = site.copy.path().to_path_buf();
    // The binary is the askpass program wherever the dialog's variables
    // are set, so the command line itself must not see them: git is given
    // a program that sets them and runs the binary as its askpass.
    let in_ubuntu = |id: &str, params: serde_json::Value| -> Output {
        let dialog: Vec<(&str, OsString)> = bitbraid_git::askpass::child_env();
        let value = |wanted: &str| {
            dialog
                .iter()
                .find(|(name, _)| *name == wanted)
                .map(|(_, value)| value.to_string_lossy().into_owned())
                .expect("the dialog's variables")
        };
        let socket = PathBuf::from(value(bitbraid_git::askpass::SOCKET_ENV));
        let askpass = bitbraid_testkit::script(
            &dir,
            "askpass.sh",
            &format!(
                "#!/bin/sh\nexport {}='{}' {}='{}'\nexec '{}' \"$@\"\n",
                bitbraid_git::askpass::SOCKET_ENV,
                socket.display(),
                bitbraid_git::askpass::TOKEN_ENV,
                value(bitbraid_git::askpass::TOKEN_ENV),
                program.display()
            ),
        )
        .unwrap();
        let env: Vec<(&str, OsString)> = vec![
            ("GIT_ASKPASS", askpass.into_os_string()),
            ("HOME", home.clone().into_os_string()),
            ("GIT_CONFIG_GLOBAL", gitconfig.clone().into_os_string()),
            ("GIT_CONFIG_NOSYSTEM", OsString::from("1")),
            ("BITBRAID_CONFIG_DIR", dir.join("config").into_os_string()),
        ];
        let env: Vec<(&str, &std::ffi::OsStr)> = env
            .iter()
            .map(|(name, value)| (*name, value.as_os_str()))
            .collect();
        let socket_dir = socket.parent().unwrap().to_path_buf();
        let program_dir = program.parent().unwrap().to_path_buf();
        let params = params.to_string();
        real_proxy::run_in_ubuntu(
            &program,
            &[
                "--repo",
                path.to_str().unwrap(),
                "run",
                id,
                "--params",
                &params,
            ],
            &[&path, &dir, &socket_dir, &program_dir],
            &env,
        )
        .unwrap()
    };
    let password_only = |kind: Kind, prompt: &str| {
        assert_eq!(kind, Kind::Secret, "{prompt}");
        assert!(prompt.starts_with("Password for '"), "{prompt}");
        Some(PROXY_PASSWORD.to_owned())
    };
    let asking = Asking::with(password_only);
    let before = squid.access_log().len();
    let out = in_ubuntu("remote.fetch", json!({ "remote": "origin" }));
    assert!(
        out.status.success(),
        "{}\n{}",
        text(&out.stderr),
        squid.cache_log()
    );
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    let asked = asking.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].1.starts_with(&asked_for), "{asked:?}");
    site.git()
        .run(["merge", "-q", "--ff-only", "origin/main"])
        .unwrap();
    let local = site.local_commit("local.txt");
    let out = in_ubuntu(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(site.served().rev_parse("refs/heads/main").unwrap(), local);
    // Squid took the requests as the domain's carol, after the NTLM
    // handshake's 407s.
    let log = logged_since(&squid, before);
    let carol = |l: &Logged| {
        l.user
            .as_deref()
            .is_some_and(|user| user.starts_with(NTLM_DOMAIN) && user.ends_with(PROXY_USER))
    };
    assert!(log.iter().all(|l| carol(l) || l.status == 407), "{log:#?}");
    assert!(
        log.iter().any(|l| carol(l)
            && l.method == "POST"
            && l.target.ends_with("/git-receive-pack")
            && l.status == 200),
        "{log:#?}"
    );

    // Over HTTPS, through a tunnel opened after the handshake.
    site.git()
        .run(["remote", "set-url", "origin", &secure.url(NAME, &repo)])
        .unwrap();
    let upstream = site.upstream_commit("upstream-2.txt");
    let before = squid.access_log().len();
    let out = in_ubuntu("remote.fetch", json!({ "remote": "origin" }));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    let log = logged_since(&squid, before);
    assert!(
        log.iter()
            .any(|l| carol(l) && l.method == "CONNECT" && l.result == "TCP_TUNNEL"),
        "{log:#?}"
    );

    // git's own way, the user named: curl takes the one way Squid offers.
    site.git()
        .run(["config", "--unset", "http.proxyAuthMethod"])
        .unwrap();
    let upstream = site.upstream_commit("upstream-3.txt");
    let out = in_ubuntu("remote.fetch", json!({ "remote": "origin" }));
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    config(&site, "http.proxyAuthMethod", "ntlm");
    drop(asking);

    // A wrong password: Squid's 407, and why it may be.
    let asking = Asking::with(|_, _| Some("not-the-password".to_owned()));
    let out = in_ubuntu("remote.fetch", json!({ "remote": "origin" }));
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains(SAID_NTLM) && err.contains("407"), "{err}");
    assert!(!err.contains("not-the-password"), "{err}");
    assert_eq!(asking.asked().len(), 1);
    drop(asking);
    let _ = std::fs::remove_dir_all(&dir);
}

// --------------------------------------------------------- Negotiate

#[test]
#[ignore = "real proxy: needs BITBRAID_REAL_PROXY and Docker"]
fn squid_with_negotiate_lets_git_through_with_a_kerberos_ticket() {
    if !may_run("squid with negotiate") {
        return;
    }
    if !real_proxy::have_kinit() {
        eprintln!("squid with negotiate: skipped, MIT Kerberos (kinit) is not installed");
        return;
    }
    let dir = scratch("negotiate");
    let site = Site::new();
    let plain = server(&site, SmartHttpOptions::default());
    let (authority, tls) = authority(&dir, &[NAME]);
    let secure = server(
        &site,
        SmartHttpOptions {
            login: None,
            tls: Some(tls),
        },
    );
    let service = format!("HTTP/{PROXY_NAME}");
    let kdc = Kdc::start(
        REALM,
        &[(KERBEROS_USER, KERBEROS_PASSWORD)],
        &[service.as_str()],
    )
    .unwrap();
    let squid = Squid::start(&SquidOptions {
        negotiate: Some(Negotiate {
            keytab: kdc.keytab(),
            principal: format!("{service}@{REALM}"),
            krb5_conf: kdc.config(),
        }),
        tunnel_ports: vec![secure.port()],
        names: vec![NAME.to_owned()],
        ..SquidOptions::default()
    })
    .unwrap();
    let krb5_conf = dir.join("krb5.conf");
    std::fs::write(&krb5_conf, kdc.config()).unwrap();
    let cache = dir.join("ccache");
    let kerberos = Some((krb5_conf.as_path(), cache.as_path()));

    let repo = served_name(&site);
    let proxy = format!("http://{PROXY_NAME}:{}", squid.port());
    let resolve = format!("{PROXY_NAME}:{}:127.0.0.1", squid.port());
    site.git()
        .run(["remote", "add", "origin", &plain.url(NAME, &repo)])
        .unwrap();
    config(&site, "http.proxy", &proxy);
    config(&site, "http.proxyAuthMethod", "negotiate");
    config(&site, "http.curloptResolve", &resolve);
    config(&site, "http.sslCAInfo", authority.to_str().unwrap());
    let upstream = site.upstream_commit("upstream.txt");
    let path = site.copy.path().to_path_buf();
    let fetch = |kerberos: Option<(&Path, &Path)>| {
        run_cli(
            &path,
            "remote.fetch",
            json!({ "remote": "origin" }),
            kerberos,
            &[],
        )
    };

    // No ticket.
    let out = fetch(kerberos);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(
        err.contains(SAID_TICKET) && err.contains("kinit") && err.contains("407"),
        "{err}"
    );
    let log = squid.access_log();
    assert!(!log.is_empty());
    assert!(
        log.iter().all(|l| l.status == 407 && l.user.is_none()),
        "{log:#?}"
    );

    // A wrong password at kinit, then the right one.
    let wrong = kdc
        .kinit(&krb5_conf, &cache, KERBEROS_USER, "not-the-password")
        .expect_err("a wrong password");
    assert!(wrong.contains("Password incorrect"), "{wrong}");
    kdc.kinit(&krb5_conf, &cache, KERBEROS_USER, KERBEROS_PASSWORD)
        .unwrap();

    // With the ticket: fetch and push over plain HTTP, then HTTPS.
    let before = squid.access_log().len();
    let out = fetch(kerberos);
    assert!(
        out.status.success(),
        "{}\n{}",
        text(&out.stderr),
        squid.cache_log()
    );
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    site.git()
        .run(["merge", "-q", "--ff-only", "origin/main"])
        .unwrap();
    let local = site.local_commit("local.txt");
    let out = run_cli(
        &path,
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
        kerberos,
        &[],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(site.served().rev_parse("refs/heads/main").unwrap(), local);
    let user = format!("{KERBEROS_USER}@{REALM}");
    let log = logged_since(&squid, before);
    assert!(
        log.iter()
            .all(|l| l.user.is_none() || l.user.as_deref() == Some(&user)),
        "{log:#?}"
    );
    assert!(
        log.iter().any(|l| l.method == "POST"
            && l.target.ends_with("/git-receive-pack")
            && l.status == 200
            && l.user.as_deref() == Some(&user)),
        "{log:#?}"
    );
    // The ticket curl asked the KDC for is the proxy's service's, by the
    // name the proxy was given.
    let tickets = Command::new("klist")
        .arg("-c")
        .arg(format!("FILE:{}", cache.display()))
        .env("KRB5_CONFIG", &krb5_conf)
        .output()
        .unwrap();
    assert!(
        text(&tickets.stdout).contains(&format!("{service}@{REALM}")),
        "{}",
        text(&tickets.stdout)
    );

    site.git()
        .run(["remote", "set-url", "origin", &secure.url(NAME, &repo)])
        .unwrap();
    let upstream = site.upstream_commit("upstream-2.txt");
    let before = squid.access_log().len();
    let out = fetch(kerberos);
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    let log = logged_since(&squid, before);
    assert!(
        log.iter().any(|l| l.method == "CONNECT"
            && l.result == "TCP_TUNNEL"
            && l.status == 200
            && l.user.as_deref() == Some(&user)),
        "{log:#?}"
    );

    // A clone, the proxy given to git by the environment.
    let into = dir.join("cloned");
    let out = {
        let mut cmd = cli(&dir, kerberos);
        cmd.args([
            "run",
            "repo.clone",
            "--params",
            &json!({ "url": secure.url(NAME, &repo), "path": into }).to_string(),
        ])
        .env("GIT_SSL_CAINFO", &authority)
        .env("GIT_CONFIG_COUNT", "3")
        .env("GIT_CONFIG_KEY_0", "http.proxy")
        .env("GIT_CONFIG_VALUE_0", &proxy)
        .env("GIT_CONFIG_KEY_1", "http.proxyAuthMethod")
        .env("GIT_CONFIG_VALUE_1", "negotiate")
        .env("GIT_CONFIG_KEY_2", "http.curloptResolve")
        .env("GIT_CONFIG_VALUE_2", &resolve);
        cmd.output().unwrap()
    };
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(into.join(".git").is_dir());

    // git's own default (anyauth) with the ticket at hand: it offers none
    // to a proxy whose address names no user.
    site.git()
        .run(["config", "--unset", "http.proxyAuthMethod"])
        .unwrap();
    let out = fetch(kerberos);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(
        err.contains(SAID_LOGIN) && err.contains("http.proxyAuthMethod set to negotiate"),
        "{err}"
    );
    config(&site, "http.proxyAuthMethod", "negotiate");

    // The proxy by its address: the ticket asked for is not its
    // service's.
    config(&site, "http.proxy", &squid.url());
    let out = fetch(kerberos);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(
        err.contains(SAID_TICKET) && err.contains("not by its address"),
        "{err}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
