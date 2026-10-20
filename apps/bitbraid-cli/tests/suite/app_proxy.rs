//! The application's own connections through the proxy its surroundings
//! name, end to end: the command line run as a program with the
//! variables set as a user's shell would have them, asking a GitLab of
//! one's own for a code to sign in with (`account.sign_in_start`, which
//! keeps nothing in the keychain).
//!
//! `HTTPS_PROXY` with a login in it; `https_proxy` before `HTTPS_PROXY`;
//! `ALL_PROXY`; git's `http.proxy` where no variable names one; a proxy
//! that wants a login and has none; one that cannot be used. The server
//! answers from the recording of a sign-in by code at a GitLab
//! (`crates/integrations/tests/recordings/gitlab/device_flow.json`) over
//! TLS, with a certificate from an authority of the test's own that the
//! program trusts through `SSL_CERT_FILE` (read by the client's source of
//! root certificates in place of the system's store). It goes by
//! `gitlab.example.test`, which resolves nowhere: what reached it went
//! through the proxy, and no name was looked up but the proxy's.
//!
//! Sources: curl(1) "ENVIRONMENT"; git-config(1) (`http.proxy`), git(1)
//! (`GIT_CONFIG_GLOBAL`); the documentation of `rustls-native-certs`
//! (`SSL_CERT_FILE`); RFC 6761 (`.test`).

use std::{
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use bitbraid_integrations::{
    http::{Call, Fetch as _, Limits, Method, Url},
    replay::Replay,
};
use bitbraid_testkit::{
    Answer, Asked, Authority, Git, Outcome, Proxy, ProxyOptions, StandIn, fixtures_out_dir,
    proxy::basic, scratch_root,
};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const NAME: &str = "gitlab.example.test";
/// The application id the recording's sign-in was made with.
const CLIENT_ID: &str = "f4d2cf3c9a724e6b34744588be1d48b12f0badbfd455d0cd63f2e53195e5315c";

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
    let dir = scratch_root().join(format!("app-proxy-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// What the recording answers to a request, the GitLab of the recording
/// (`127.0.0.1:8929`) named as the host that was asked.
fn replayed(replay: &Replay, asked: &Asked) -> Answer {
    let host = asked.header("host").unwrap_or(NAME);
    let limits = Limits {
        connect: Duration::from_secs(5),
        total: Duration::from_secs(10),
        body: 1024 * 1024,
    };
    let (Ok(url), Ok(method)) = (
        Url::parse(&format!("https://{host}{}", asked.target)),
        serde_json::from_value::<Method>(json!(asked.method)),
    ) else {
        return Answer {
            status: 400,
            ..Answer::default()
        };
    };
    let mut call = Call::new(method, url, limits);
    if !asked.body.is_empty() {
        call = call.with_body(
            asked
                .header("content-type")
                .unwrap_or("application/x-www-form-urlencoded"),
            asked.body.clone(),
        );
    }
    match replay.send(&call, &AtomicBool::new(false)) {
        Ok(response) => Answer {
            status: response.status,
            headers: response.headers.clone(),
            body: String::from_utf8_lossy(&response.body)
                .replace("http://127.0.0.1:8929", &format!("https://{host}"))
                .into_bytes(),
        },
        Err(err) => Answer {
            status: 404,
            headers: Vec::new(),
            body: err.to_string().into_bytes(),
        },
    }
}

/// The recorded GitLab over TLS, and the file of the authority that
/// vouches for it.
struct Site {
    dir: PathBuf,
    server: StandIn,
    authority: PathBuf,
}

impl Site {
    fn new(name: &str) -> Self {
        let dir = scratch(name);
        let authority = Authority::new().unwrap();
        let file = dir.join("authority.pem");
        authority.write_pem(&file).unwrap();
        let replay = Arc::new(
            Replay::load(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../crates/integrations/tests/recordings/gitlab/device_flow.json"),
            )
            .unwrap(),
        );
        let server = StandIn::start(
            Some(authority.server_config(&[NAME]).unwrap()),
            move |asked| replayed(&replay, asked),
        )
        .unwrap();
        Self {
            dir,
            server,
            authority: file,
        }
    }

    fn tunnel(&self) -> String {
        format!("{NAME}:{}", self.server.port())
    }

    /// `account.sign_in_start` at the site, run as a program with `env`
    /// over a scrubbed environment: no proxy variable of whoever runs the
    /// tests, the fixtures' home and git configuration.
    fn sign_in(&self, env: &[(&str, &str)]) -> Output {
        let home = fixtures_out_dir().join(".home");
        Git::new(fixtures_out_dir(), &home).ensure_home().unwrap();
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let params = json!({
            "kind": "gitlab",
            "web": format!("https://{NAME}:{}", self.server.port()),
            "client_id": CLIENT_ID,
        });
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_bitbraid-cli"));
        cmd.current_dir(&self.dir)
            .args([
                "run",
                "account.sign_in_start",
                "--params",
                &params.to_string(),
            ])
            .env("HOME", &home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("BITBRAID_CONFIG_DIR", self.dir.join(format!("config-{n}")))
            .env("SSL_CERT_FILE", &self.authority)
            .env_remove("SSL_CERT_DIR")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in PROXY_VARIABLES {
            cmd.env_remove(name);
        }
        for (name, value) in env {
            cmd.env(name, value);
        }
        cmd.output().unwrap()
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// The code the program printed, after it said it succeeded.
fn code_of(out: &Output) -> Value {
    assert!(out.status.success(), "{}", text(&out.stderr));
    serde_json::from_slice::<Value>(&out.stdout).unwrap()["user_code"].clone()
}

fn passed_through(proxy: &Proxy, site: &Site) {
    let passages = proxy.passages();
    assert!(!passages.is_empty());
    assert!(
        passages.iter().all(|p| p.method == "CONNECT"
            && p.target == site.tunnel()
            && p.outcome == Outcome::Passed),
        "{passages:?}"
    );
}

#[test]
fn the_variables_name_the_proxy_the_application_goes_through() {
    let site = Site::new("variables");

    // HTTPS_PROXY, with a login.
    let proxy = Proxy::start(ProxyOptions {
        login: Some(("carol".to_owned(), "proxy-secret".to_owned())),
        refuse: false,
        challenges: Vec::new(),
    })
    .unwrap();
    let out = site.sign_in(&[(
        "HTTPS_PROXY",
        &proxy.url_with("carol", Some("proxy-secret")),
    )]);
    assert_eq!(code_of(&out), "LE4SP3QG");
    passed_through(&proxy, &site);
    assert_eq!(
        proxy.passages()[0].authorization.as_deref(),
        Some(basic("carol", "proxy-secret").as_str())
    );
    assert!(
        site.server
            .asked()
            .iter()
            .any(|asked| asked.target == "/oauth/authorize_device")
    );

    // The same proxy, no login in its address: said, and nothing sent.
    let before = site.server.asked().len();
    let out = site.sign_in(&[("HTTPS_PROXY", &proxy.url())]);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    assert!(err.contains("asks for a user name and a password"), "{err}");
    assert_eq!(site.server.asked().len(), before);

    // https_proxy before HTTPS_PROXY (where the case of a name counts).
    if !cfg!(windows) {
        let small = Proxy::start(ProxyOptions::default()).unwrap();
        let dead = format!("http://127.0.0.1:{}", crate::transport_auth::free_port());
        let out = site.sign_in(&[("https_proxy", &small.url()), ("HTTPS_PROXY", &dead)]);
        assert_eq!(code_of(&out), "LE4SP3QG");
        passed_through(&small, &site);
    }

    // ALL_PROXY, when nothing names a proxy for HTTPS.
    let all = Proxy::start(ProxyOptions::default()).unwrap();
    let out = site.sign_in(&[("ALL_PROXY", &all.url())]);
    assert_eq!(code_of(&out), "LE4SP3QG");
    passed_through(&all, &site);

    // A proxy that cannot be used stops the request: no going around it.
    let before = site.server.asked().len();
    let out = site.sign_in(&[("HTTPS_PROXY", "socks5://127.0.0.1:1080")]);
    assert!(!out.status.success());
    let err = text(&out.stderr);
    // On Windows a name has no case, and the variable is named as the
    // environment writes it: the first spelling this test's command was
    // given (it took `https_proxy` away before it set HTTPS_PROXY).
    let named = if cfg!(windows) {
        err.to_lowercase()
            .contains("the proxy named in https_proxy cannot be used")
    } else {
        err.contains("the proxy named in HTTPS_PROXY cannot be used")
    };
    assert!(named && err.contains("SOCKS"), "{err}");
    assert_eq!(site.server.asked().len(), before);
}

#[test]
fn gits_own_setting_names_the_proxy_where_no_variable_does() {
    let site = Site::new("git-setting");
    let proxy = Proxy::start(ProxyOptions::default()).unwrap();
    let global = site.dir.join("global.gitconfig");
    std::fs::write(
        &global,
        format!(
            "[user]\n\tname = Ada Fixture\n\temail = ada@fixture.invalid\n\
             [http]\n\tproxy = {}\n",
            proxy.url()
        ),
    )
    .unwrap();
    let out = site.sign_in(&[("GIT_CONFIG_GLOBAL", global.to_str().unwrap())]);
    assert_eq!(code_of(&out), "LE4SP3QG");
    passed_through(&proxy, &site);

    // A variable comes first: the setting is not asked.
    let named = Proxy::start(ProxyOptions::default()).unwrap();
    let before = proxy.passages().len();
    let out = site.sign_in(&[
        ("GIT_CONFIG_GLOBAL", global.to_str().unwrap()),
        ("HTTPS_PROXY", &named.url()),
    ]);
    assert_eq!(code_of(&out), "LE4SP3QG");
    passed_through(&named, &site);
    assert_eq!(proxy.passages().len(), before);
}
