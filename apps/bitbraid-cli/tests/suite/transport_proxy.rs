//! Fetch, push and clone through the engine with a proxy on the way: the
//! proxy named in `http.proxy`, in git's global configuration and in the
//! `http_proxy`, `https_proxy` and `no_proxy` variables; a proxy that
//! asks for a login, answered through the application's askpass dialog,
//! and a login it refuses; a proxy that wants a Kerberos ticket
//! (Negotiate), said by how git was told to sign in to it
//! (`http.proxyAuthMethod`); a proxy that refuses or is not there, said
//! in the application's words; HTTPS through a `CONNECT` tunnel, with a
//! certificate from an authority of the test's own that git is told to
//! trust (`http.sslCAInfo`, `GIT_SSL_CAINFO`); and ssh through the proxy
//! as a `ProxyCommand`.
//!
//! Everything runs on this machine: the proxy and the servers listen on
//! the loopback interface, and the proxy connects nowhere else
//! (`bitbraid_testkit::proxy`). The servers are given a name that resolves
//! nowhere, `git.example.test` (RFC 6761 keeps `.test` for this), so a
//! request that reached one went through the proxy; tests that go around
//! it name the loopback address instead, so that no name is ever looked
//! up.
//!
//! The variables are a child's, never this process's: a test that needs
//! them runs the command line as a program with them set. A test that
//! needs the dialog runs the engine here, with `http.proxy` naming the
//! proxy, which is where git takes the user name it asks the password
//! for, whichever of the two named the proxy.
//!
//! Where `git http-backend` is not installed the tests say so and pass
//! without having run; the ssh test also needs `sshd` and `ncat` or
//! `socat`, and says so where they are missing (a hosted runner).
//!
//! Sources: git-config(1) (`http.proxy`, `http.sslCAInfo`,
//! `http.sslBackend`, `core.sshCommand`), git(1) (`GIT_SSL_CAINFO`,
//! `GIT_CONFIG_GLOBAL`), gitcredentials(7), curl(1) "ENVIRONMENT"
//! (`http_proxy`, `HTTPS_PROXY`, `NO_PROXY`), ssh_config(5)
//! (`ProxyCommand`, `%h`, `%p`), ncat(1) (`--proxy`, `--proxy-type`),
//! socat(1) (`PROXY:`), RFC 9110 §9.3.6 (CONNECT), RFC 6761 (`.test`).
#![allow(clippy::print_stderr)]

use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bitbraid_git::askpass::Kind;
use bitbraid_testkit::{
    Authority, Git, Outcome, Proxy, ProxyOptions, SmartHttp, SmartHttpOptions, fixtures_out_dir,
    proxy::basic, scratch_root, tls::ServerConfig,
};
use serde_json::json;

use crate::{
    ASKPASS_SERVER as ONE,
    transport_auth::{Asking, PASSPHRASE, Site, free_port, served_path, sshd_or_skip},
};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The name the servers go by: it resolves nowhere.
const NAME: &str = "git.example.test";
const PROXY_USER: &str = "carol";
const PROXY_PASSWORD: &str = "proxy-secret";

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

fn proxy() -> Proxy {
    Proxy::start(ProxyOptions::default()).unwrap()
}

fn proxy_with_login() -> Proxy {
    Proxy::start(ProxyOptions {
        login: Some((PROXY_USER.to_owned(), PROXY_PASSWORD.to_owned())),
        refuse: false,
        challenges: Vec::new(),
    })
    .unwrap()
}

/// The name of the repository the servers hand out for `site`.
fn served_name(site: &Site) -> String {
    site.bare
        .file_name()
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

/// A smart-HTTP server for `site`'s repository, or `None` (said) when git
/// has no `http-backend` here.
fn server_or_skip(test: &str, site: &Site, options: SmartHttpOptions) -> Option<SmartHttp> {
    // An askpass program for git alone in the developer's environment
    // comes before everything a test can set, and would be asked on their
    // screen.
    if std::env::var_os("GIT_ASKPASS").is_some() {
        eprintln!("{test}: skipped, GIT_ASKPASS is set and would be asked instead");
        return None;
    }
    let server = SmartHttp::start(site.bare.parent().unwrap(), options).unwrap();
    if server.is_none() {
        eprintln!("{test}: skipped, git has no http-backend here");
    }
    server
}

/// Point `site` at `url` through the proxy at `proxy`.
fn through(site: &Site, url: &str, proxy: &str) {
    let git = site.git();
    git.run(["remote", "add", "origin", url]).unwrap();
    git.run(["config", "http.proxy", proxy]).unwrap();
}

/// A scratch directory of the test's own.
fn scratch(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = scratch_root().join(format!("proxy-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A test authority's certificate written where git can be told to read
/// it, and a server configuration for `NAME` signed by it.
fn authority(dir: &Path) -> (PathBuf, Arc<ServerConfig>) {
    let authority = Authority::new().unwrap();
    let file = dir.join("authority.pem");
    authority.write_pem(&file).unwrap();
    (file, authority.server_config(&[NAME]).unwrap())
}

/// Have git trust the test authority for HTTPS, in `site`'s configuration.
fn trust(site: &Site, authority: &Path) {
    let git = site.git();
    git.run(["config", "http.sslCAInfo", authority.to_str().unwrap()])
        .unwrap();
    // Git for Windows asks the system's store unless told to use OpenSSL,
    // which reads the file.
    if cfg!(windows) {
        git.run(["config", "http.sslBackend", "openssl"]).unwrap();
    }
}

// ------------------------------------------------- the engine, here

#[test]
fn through_the_proxy_of_http_proxy_a_fetch_and_a_push_reach_the_server() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let Some(server) = server_or_skip(
        "fetch and push through http.proxy",
        &site,
        SmartHttpOptions::default(),
    ) else {
        return;
    };
    let proxy = proxy();
    let url = server.url(NAME, &served_name(&site));
    through(&site, &url, &proxy.url());
    let upstream = site.upstream_commit("upstream.txt");

    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}"));
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

    // Every request went through the proxy, as a whole address with the
    // name, and came to the server with the name as its host.
    let passages = proxy.passages();
    assert!(passages.len() >= 4, "{passages:?}");
    let prefix = format!("http://{NAME}:{}/", server.port());
    assert!(
        passages
            .iter()
            .all(|p| p.outcome == Outcome::Passed && p.target.starts_with(&prefix)),
        "{passages:?}"
    );
    assert!(
        passages
            .iter()
            .any(|p| p.method == "POST" && p.target.ends_with("/git-receive-pack"))
    );
    let served = server.served();
    assert_eq!(served.len(), passages.len(), "{served:?}");
    let host = format!("{NAME}:{}", server.port());
    assert!(
        served
            .iter()
            .all(|s| s.host.as_deref() == Some(host.as_str()))
    );
}

#[test]
fn a_proxy_that_asks_for_a_login_gets_its_password_through_the_dialog() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let Some(server) = server_or_skip(
        "a proxy login through the dialog",
        &site,
        SmartHttpOptions::default(),
    ) else {
        return;
    };
    let proxy = proxy_with_login();
    let named = proxy.url_with(PROXY_USER, None);
    through(&site, &server.url(NAME, &served_name(&site)), &named);
    let upstream = site.upstream_commit("upstream.txt");
    let asking = Asking::with(|kind, prompt| {
        Some(match kind {
            Kind::Secret if prompt.starts_with("Password for") => PROXY_PASSWORD.to_owned(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    });

    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}"));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    // One question: the password of the user the proxy's address names.
    let asked = asking.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(
        asked[0].1.starts_with(&format!("Password for '{named}'")),
        "{asked:?}"
    );
    let passages = proxy.passages();
    let wanted = basic(PROXY_USER, PROXY_PASSWORD);
    assert!(
        passages
            .iter()
            .filter(|p| p.outcome == Outcome::Passed)
            .all(|p| p.authorization.as_deref() == Some(wanted.as_str())),
        "{passages:?}"
    );
    assert!(passages.iter().any(|p| p.outcome == Outcome::Passed));
    assert!(passages.iter().all(|p| p.outcome != Outcome::WrongLogin));

    // A push asks again: nothing was kept.
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
    assert_eq!(asking.count("Password for"), 2);
    let config = std::fs::read_to_string(site.copy.path().join(".git/config")).unwrap();
    assert!(!config.contains(PROXY_PASSWORD), "{config}");
}

#[test]
fn a_proxy_login_that_is_wrong_or_missing_is_refused_and_said() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let Some(server) = server_or_skip("a wrong proxy login", &site, SmartHttpOptions::default())
    else {
        return;
    };
    let proxy = proxy_with_login();
    through(
        &site,
        &server.url(NAME, &served_name(&site)),
        &proxy.url_with(PROXY_USER, None),
    );
    site.upstream_commit("upstream.txt");
    let asking = Asking::with(|_, _| Some("not-the-password".to_owned()));
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("the login is wrong");
    assert!(err.contains("407"), "{err}");
    assert!(
        err.contains("The proxy refused the request")
            && err.contains("it wants to know who is asking"),
        "{err}"
    );
    assert!(!err.contains("not-the-password"), "{err}");
    assert_eq!(asking.asked().len(), 1, "asked once: {:?}", asking.asked());
    assert_eq!(site.tracking(), None);
    let passages = proxy.passages();
    assert!(
        passages.iter().any(|p| p.outcome == Outcome::WrongLogin),
        "{passages:?}"
    );
    assert!(passages.iter().all(|p| p.outcome != Outcome::Passed));
    assert!(server.served().is_empty(), "nothing reached the server");
    drop(asking);

    // No user in the proxy's address: git has nobody to ask the password
    // of, asks nothing, and is turned away.
    site.git()
        .run(["config", "http.proxy", &proxy.url()])
        .unwrap();
    let asking = Asking::with(|_, prompt| panic!("nothing should be asked: {prompt}"));
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("no login");
    assert!(
        err.contains("The proxy refused the request")
            && err.contains("it wants to know who is asking"),
        "{err}"
    );
    assert!(asking.asked().is_empty());
    assert!(server.served().is_empty());
}

#[test]
fn a_proxy_that_is_not_there_is_said() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let Some(server) = server_or_skip(
        "a proxy that is not there",
        &site,
        SmartHttpOptions::default(),
    ) else {
        return;
    };
    // A port taken and let go again: nothing listens on it.
    let nowhere = format!("http://127.0.0.1:{}", free_port());
    through(&site, &server.url(NAME, &served_name(&site)), &nowhere);
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("no proxy there");
    // curl says that it was the proxy that was not reached since 8.x;
    // older curl names the address only, and nothing more can be said.
    if err.contains("over proxy") {
        assert!(
            err.contains("Nothing answered at the proxy's address"),
            "{err}"
        );
    } else {
        eprintln!("this git's curl does not say that the proxy was not reached: {err}");
        assert!(err.contains("Failed to connect"), "{err}");
    }
    assert!(server.served().is_empty());
    assert_eq!(site.tracking(), None);
}

#[test]
fn over_https_git_goes_through_a_tunnel_and_a_refusing_proxy_is_said() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let dir = scratch("https");
    let (authority, config) = authority(&dir);
    let Some(server) = server_or_skip(
        "https through a tunnel",
        &site,
        SmartHttpOptions {
            login: None,
            tls: Some(config),
        },
    ) else {
        return;
    };
    let proxy = proxy();
    let url = server.url(NAME, &served_name(&site));
    assert!(url.starts_with("https://"));
    through(&site, &url, &proxy.url());
    trust(&site, &authority);
    let upstream = site.upstream_commit("upstream.txt");

    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}"));
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
    // The proxy opened tunnels to the name and saw nothing of what went
    // through them.
    let passages = proxy.passages();
    let tunnel = format!("{NAME}:{}", server.port());
    assert!(!passages.is_empty());
    assert!(
        passages
            .iter()
            .all(|p| p.method == "CONNECT" && p.target == tunnel && p.outcome == Outcome::Passed),
        "{passages:?}"
    );
    assert!(server.served().iter().any(|s| s.method == "POST"));

    // A proxy with a login, over the tunnel: asked through the dialog.
    let with_login = proxy_with_login();
    site.git()
        .run([
            "config",
            "http.proxy",
            &with_login.url_with(PROXY_USER, None),
        ])
        .unwrap();
    let asking = Asking::with(|kind, prompt| {
        assert_eq!(kind, Kind::Secret, "{prompt}");
        Some(PROXY_PASSWORD.to_owned())
    });
    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch with a login: {err}"));
    assert_eq!(asking.asked().len(), 1);
    assert!(
        with_login
            .passages()
            .iter()
            .any(|p| p.method == "CONNECT" && p.outcome == Outcome::Passed)
    );
    drop(asking);

    // A proxy that refuses to open the tunnel.
    let refusing = Proxy::start(ProxyOptions {
        login: None,
        refuse: true,
        challenges: Vec::new(),
    })
    .unwrap();
    site.git()
        .run(["config", "http.proxy", &refusing.url()])
        .unwrap();
    let before = server.served().len();
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("refused");
    assert!(err.contains("403"), "{err}");
    assert!(
        err.contains("The proxy did not open a connection to the server"),
        "{err}"
    );
    assert_eq!(refusing.passages()[0].outcome, Outcome::Refused);
    assert_eq!(server.served().len(), before);
    let _ = std::fs::remove_dir_all(&dir);
}

// --------------------------------- the command line, with variables

/// The command line as a program in `dir`, seeing the fixtures' home and
/// configuration and no proxy variable of whoever runs the tests.
fn cli(dir: &Path) -> Command {
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
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for name in PROXY_VARIABLES {
        cmd.env_remove(name);
    }
    cmd
}

fn clone_with(dir: &Path, url: &str, into: &Path, env: &[(&str, &str)]) -> Output {
    let params = json!({ "url": url, "path": into });
    let mut cmd = cli(dir);
    cmd.args(["run", "repo.clone", "--params", &params.to_string()]);
    for (name, value) in env {
        cmd.env(name, value);
    }
    cmd.output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn the_variables_and_the_global_configuration_name_the_proxy_of_a_clone() {
    let site = Site::new();
    let Some(server) = server_or_skip(
        "a clone through the variables",
        &site,
        SmartHttpOptions::default(),
    ) else {
        return;
    };
    let upstream = site.served().rev_parse("refs/heads/main").unwrap();
    let dir = scratch("clone");
    let url = server.url(NAME, &served_name(&site));

    // http_proxy, for an http:// address.
    let proxy = proxy();
    let out = clone_with(
        &dir,
        &url,
        &dir.join("by-variable"),
        &[("http_proxy", &proxy.url())],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let cloned = Git::new(dir.join("by-variable"), fixtures_out_dir().join(".home"));
    assert_eq!(cloned.rev_parse("HEAD").unwrap(), upstream);
    let passages = proxy.passages();
    assert!(!passages.is_empty());
    assert!(passages.iter().all(|p| p.outcome == Outcome::Passed));

    // HTTP_PROXY in capitals is not read for http:// (curl reads only the
    // small letters for it): the clone goes direct, to the loopback
    // address, and the proxy sees nothing. On Windows the names of
    // variables are the same in either case.
    let direct = server.url("127.0.0.1", &served_name(&site));
    if !cfg!(windows) {
        let ignored = self::proxy();
        let out = clone_with(
            &dir,
            &direct,
            &dir.join("capitals"),
            &[("HTTP_PROXY", &ignored.url())],
        );
        assert!(out.status.success(), "{}", text(&out.stderr));
        assert!(ignored.passages().is_empty());
    }

    // no_proxy names the host: direct, though http_proxy names a proxy.
    let bypassed = self::proxy();
    let out = clone_with(
        &dir,
        &direct,
        &dir.join("no-proxy"),
        &[("http_proxy", &bypassed.url()), ("no_proxy", "127.0.0.1")],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(bypassed.passages().is_empty());

    // git's global configuration, where the variables say nothing.
    let configured = self::proxy();
    let global = dir.join("global.gitconfig");
    std::fs::write(
        &global,
        format!(
            "[user]\n\tname = Ada Fixture\n\temail = ada@fixture.invalid\n\
             [http]\n\tproxy = {}\n",
            configured.url()
        ),
    )
    .unwrap();
    let out = clone_with(
        &dir,
        &url,
        &dir.join("by-configuration"),
        &[("GIT_CONFIG_GLOBAL", global.to_str().unwrap())],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(!configured.passages().is_empty());

    // A proxy that is not there stops the clone, leaves nothing, and is
    // said in our words before git's (with a curl that names it).
    let nowhere = format!("http://127.0.0.1:{}", free_port());
    let out = clone_with(
        &dir,
        &url,
        &dir.join("nowhere"),
        &[("http_proxy", &nowhere)],
    );
    assert!(!out.status.success());
    let err = text(&out.stderr);
    if err.contains("over proxy") {
        assert!(
            err.contains("Nothing answered at the proxy's address"),
            "{err}"
        );
    }
    assert!(!dir.join("nowhere").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_proxy_that_wants_kerberos_is_said_by_how_git_was_told_to_sign_in() {
    let site = Site::new();
    let Some(server) = server_or_skip(
        "a proxy that wants kerberos",
        &site,
        SmartHttpOptions::default(),
    ) else {
        return;
    };
    let proxy = Proxy::start(ProxyOptions {
        challenges: vec!["Negotiate".to_owned()],
        ..ProxyOptions::default()
    })
    .unwrap();
    through(&site, &server.url(NAME, &served_name(&site)), &proxy.url());
    let dir = scratch("kerberos");
    // Kerberos of the test's own: no realm, no ticket.
    let config = dir.join("krb5.conf");
    std::fs::write(&config, "").unwrap();
    let fetch = || {
        let mut cmd = cli(&dir);
        cmd.args([
            "--repo",
            site.copy.path().to_str().unwrap(),
            "run",
            "remote.fetch",
            "--params",
            &json!({ "remote": "origin" }).to_string(),
        ])
        .env("KRB5_CONFIG", &config)
        .env(
            "KRB5CCNAME",
            format!("FILE:{}", dir.join("no-ticket").display()),
        )
        .env_remove("GIT_HTTP_PROXY_AUTHMETHOD");
        cmd.output().unwrap()
    };

    // Git's own way: it offers no ticket, and is told how to.
    let out = fetch();
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(
        err.contains("it wants to know who is asking")
            && err.contains("http.proxyAuthMethod set to negotiate"),
        "{err}"
    );
    // Told to sign in with a ticket, and without one.
    site.git()
        .run(["config", "http.proxyAuthMethod", "negotiate"])
        .unwrap();
    let out = fetch();
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(
        err.contains("git was told to sign in to it with a Kerberos ticket")
            && err.contains("kinit")
            && err.contains("407"),
        "{err}"
    );
    assert!(!err.contains("it wants to know who is asking"), "{err}");
    assert!(server.served().is_empty());
    assert!(
        proxy
            .passages()
            .iter()
            .all(|p| p.outcome == Outcome::WantsLogin)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn https_proxy_in_either_case_carries_a_clone_through_a_tunnel() {
    let site = Site::new();
    let dir = scratch("https-clone");
    let (authority, config) = authority(&dir);
    let Some(server) = server_or_skip(
        "an https clone through the variables",
        &site,
        SmartHttpOptions {
            login: None,
            tls: Some(config),
        },
    ) else {
        return;
    };
    let url = server.url(NAME, &served_name(&site));
    let mut trust_env = vec![("GIT_SSL_CAINFO", authority.to_str().unwrap().to_owned())];
    // Git for Windows asks the system's store unless told to use OpenSSL,
    // which reads the file; a clone has no configuration of its own yet.
    if cfg!(windows) {
        trust_env.extend([
            ("GIT_CONFIG_COUNT", "1".to_owned()),
            ("GIT_CONFIG_KEY_0", "http.sslBackend".to_owned()),
            ("GIT_CONFIG_VALUE_0", "openssl".to_owned()),
        ]);
    }
    for (variable, folder) in [("https_proxy", "small"), ("HTTPS_PROXY", "capitals")] {
        let proxy = proxy();
        let mut env: Vec<(&str, &str)> = trust_env
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        let address = proxy.url();
        env.push((variable, &address));
        let out = clone_with(&dir, &url, &dir.join(folder), &env);
        assert!(out.status.success(), "{variable}: {}", text(&out.stderr));
        let passages = proxy.passages();
        assert!(
            !passages.is_empty()
                && passages
                    .iter()
                    .all(|p| p.method == "CONNECT"
                        && p.target == format!("{NAME}:{}", server.port())),
            "{variable}: {passages:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

// --------------------------------------------------------------- ssh

/// How to reach `%h:%p` through the HTTP proxy at `port` from ssh, with
/// the tools that are installed: ncat, else socat.
fn proxy_command(port: u16) -> Option<String> {
    let have = |program: &str| {
        Command::new(program)
            .arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
    };
    if have("ncat") {
        Some(format!(
            "ncat --proxy 127.0.0.1:{port} --proxy-type http %h %p"
        ))
    } else if Command::new("socat")
        .arg("-V")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
    {
        Some(format!("socat - PROXY:127.0.0.1:%h:%p,proxyport={port}"))
    } else {
        None
    }
}

#[test]
fn over_ssh_a_proxy_command_reaches_the_server_through_the_proxy() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(sshd) = sshd_or_skip("ssh through a proxy") else {
        return;
    };
    let proxy = proxy();
    let Some(command) = proxy_command(proxy.port()) else {
        eprintln!("ssh through a proxy: skipped, neither ncat nor socat is installed");
        return;
    };
    let mut site = Site::new();
    sshd.serve(&site);
    // The server by the name, reached by ssh through the proxy alone.
    let git = site.git();
    let ssh = git.run(["config", "core.sshCommand"]).unwrap();
    git.run([
        "config",
        "core.sshCommand",
        &format!("{} -o 'ProxyCommand={command}'", ssh.trim()),
    ])
    .unwrap();
    let url = format!(
        "ssh://{}@{NAME}:{}{}",
        sshd.user,
        sshd.port,
        served_path(&site.bare)
    );
    git.run(["remote", "set-url", "origin", &url]).unwrap();
    let upstream = site.upstream_commit("upstream.txt");
    let asking = Asking::with(|kind, prompt| {
        Some(match kind {
            Kind::Secret if prompt.contains("passphrase") => PASSPHRASE.to_owned(),
            _ if prompt.contains("(yes/no") => "yes".to_owned(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    });

    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}\n{}", sshd.log()));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    // The host is known by its name and port.
    assert!(
        sshd.known_hosts()
            .contains(&format!("[{NAME}]:{}", sshd.port)),
        "{}",
        sshd.known_hosts()
    );
    assert_eq!(asking.count("Enter passphrase"), 1);
    let passages = proxy.passages();
    assert_eq!(passages.len(), 1, "{passages:?}");
    assert_eq!(passages[0].method, "CONNECT");
    assert_eq!(passages[0].target, format!("{NAME}:{}", sshd.port));
    assert_eq!(passages[0].outcome, Outcome::Passed);
    assert!(sshd.log().contains("Accepted publickey"));
}
