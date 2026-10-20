//! The real client through a proxy on this machine: a tunnel asked for by
//! `CONNECT` and TLS to the host inside it; the login of the proxy's
//! address sent, and a proxy that wants one and has none, or refuses it,
//! said; a proxy that refuses the tunnel, cannot reach the host, is not
//! there or cannot be used; this machine asked directly whatever the
//! proxy; a proxy that never answers, answers nonsense or without end,
//! each ending within the time allowed. Then the provider commands, the
//! sign-in by code and the pictures through it, against a server that
//! answers from the recordings of `tests/recordings/`.
//!
//! The proxy (`bitbraid_testkit::proxy`) connects to the loopback address
//! whatever host it is asked for. The servers go by names under
//! `example.test`, which resolve nowhere: a request that reached one went
//! through the proxy, and the client looked no name up but the proxy's.
//! Where a request must not go through the proxy, the server is
//! `localhost`, so that no name leaves the machine to be looked up.
//!
//! Which variables and which setting name the proxy is held in the unit
//! tests of `http::proxy` (they cannot be set for one test of a process
//! whose tests run side by side) and end to end by the command line run
//! as a program (`apps/bitbraid-cli/tests/suite/app_proxy.rs`).

use std::{
    io::Write as _,
    net::TcpListener,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use crate::common::{serve, write_forever};
use bitbraid_engine::CommandContext;
use bitbraid_integrations::{
    Services,
    avatars::{Avatars, Outcome as Picture, Service},
    http::{
        Call, Client, Fetch, FetchError, Limits, Method, NoProxy, Proxies, ProxyAddress, Request,
        Response, Url,
    },
    replay::Replay,
    secrets::MemoryStore,
};
use bitbraid_testkit::{
    Answer, Asked, Authority, Outcome, Proxy, ProxyOptions, StandIn, proxy::basic,
};
use serde_json::{Value, json};

pub(crate) const AGENT: &str = "BitBraid-Test/1.0";
static COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) fn limits(total_ms: u64) -> Limits {
    Limits {
        connect: Duration::from_secs(5),
        total: Duration::from_millis(total_ms),
        body: 1024 * 1024,
    }
}

pub(crate) fn through(address: &str) -> Proxies {
    Proxies::through(ProxyAddress::parse(address).unwrap(), "the test")
}

pub(crate) fn get(client: &Client, address: &str, total_ms: u64) -> Result<Response, FetchError> {
    let url = Url::parse(address).unwrap();
    let request = Request {
        url: &url,
        accept: "*/*",
        limits: limits(total_ms),
    };
    client.get(&request, &AtomicBool::new(false))
}

/// A TLS server for `names`, answering with `answer`, and the authority
/// a client has to trust for it.
pub(crate) fn tls_stand_in(
    names: &[&str],
    answer: impl Fn(&Asked) -> Answer + Send + Sync + 'static,
) -> (StandIn, Vec<u8>) {
    let authority = Authority::new().unwrap();
    let config = authority.server_config(names).unwrap();
    (
        StandIn::start(Some(config), answer).unwrap(),
        authority.certificate_der(),
    )
}

pub(crate) fn plain_answer(body: &str) -> impl Fn(&Asked) -> Answer + Send + Sync + 'static {
    let body = body.as_bytes().to_vec();
    move |_| Answer {
        status: 200,
        headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        body: body.clone(),
    }
}

fn login_proxy() -> Proxy {
    Proxy::start(ProxyOptions {
        login: Some(("carol".to_owned(), "proxy-secret".to_owned())),
        refuse: false,
        challenges: Vec::new(),
    })
    .unwrap()
}

/// A port nothing listens on: taken and let go again.
pub(crate) fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn through_a_proxy_a_request_goes_in_a_tunnel_with_tls_to_the_host_inside() {
    let (server, authority) = tls_stand_in(&["api.example.test"], plain_answer("inside"));
    let proxy = Proxy::start(ProxyOptions::default()).unwrap();
    let client = Client::trusting(AGENT, vec![authority]).with_proxies(through(&proxy.url()));
    let address = format!("https://api.example.test:{}/user?page=2", server.port());
    let response = get(&client, &address, 10_000).unwrap();
    assert_eq!(
        (response.status, response.body.as_slice()),
        (200, &b"inside"[..])
    );

    // The proxy was asked for a tunnel to the host and port, and saw
    // nothing of the request inside it.
    let passages = proxy.passages();
    assert_eq!(passages.len(), 1, "{passages:?}");
    assert_eq!(passages[0].method, "CONNECT");
    assert_eq!(
        passages[0].target,
        format!("api.example.test:{}", server.port())
    );
    assert_eq!(passages[0].authorization, None);
    assert_eq!(passages[0].outcome, Outcome::Passed);
    // The server got the request as it would have directly.
    let asked = server.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(asked[0].method, "GET");
    assert_eq!(asked[0].target, "/user?page=2");
    assert_eq!(
        asked[0].header("host"),
        Some(format!("api.example.test:{}", server.port()).as_str())
    );
    assert_eq!(asked[0].header("user-agent"), Some(AGENT));
    assert_eq!(asked[0].header("proxy-authorization"), None);

    // A request with a body goes the same way.
    let call = Call::new(Method::Post, Url::parse(&address).unwrap(), limits(10_000))
        .with_body("application/json", b"{\"a\":1}".to_vec());
    assert_eq!(
        client.send(&call, &AtomicBool::new(false)).unwrap().status,
        200
    );
    assert_eq!(server.asked()[1].body, b"{\"a\":1}");
    assert_eq!(proxy.passages().len(), 2);
}

#[test]
fn a_login_in_the_proxys_address_is_sent_and_a_missing_or_wrong_one_is_said() {
    let (server, authority) = tls_stand_in(&["api.example.test"], plain_answer("inside"));
    let proxy = login_proxy();
    let address = format!("https://api.example.test:{}/user", server.port());
    let client = |proxy_address: &str| {
        Client::trusting(AGENT, vec![authority.clone()]).with_proxies(through(proxy_address))
    };

    let right = client(&proxy.url_with("carol", Some("proxy-secret")));
    assert_eq!(get(&right, &address, 10_000).unwrap().status, 200);
    let passages = proxy.passages();
    assert_eq!(
        passages[0].authorization.as_deref(),
        Some(basic("carol", "proxy-secret").as_str())
    );
    assert_eq!(passages[0].outcome, Outcome::Passed);

    let err = get(&client(&proxy.url()), &address, 10_000).unwrap_err();
    let FetchError::Proxy(said) = &err else {
        panic!("{err:?}")
    };
    assert!(
        said.contains("asks for a user name and a password"),
        "{said}"
    );
    assert!(
        said.contains(&format!("127.0.0.1:{}", proxy.port())),
        "{said}"
    );

    let err = get(
        &client(&proxy.url_with("carol", Some("not-it"))),
        &address,
        10_000,
    )
    .unwrap_err();
    assert!(
        matches!(&err, FetchError::Proxy(said) if said.contains("refused the user name and password")),
        "{err:?}"
    );
    assert!(!err.to_string().contains("not-it"));
    assert!(!err.to_string().contains("proxy-secret"));
    // Neither reached the server.
    assert_eq!(server.asked().len(), 1);
    let outcomes: Vec<Outcome> = proxy.passages().iter().map(|p| p.outcome).collect();
    assert_eq!(
        outcomes,
        [Outcome::Passed, Outcome::WantsLogin, Outcome::WrongLogin]
    );
}

#[test]
fn a_proxy_that_takes_negotiate_or_ntlm_only_is_said_to_want_what_the_client_cannot_give() {
    let (server, authority) = tls_stand_in(&["api.example.test"], plain_answer("inside"));
    let address = format!("https://api.example.test:{}/user", server.port());
    let client = |proxy_address: &str| {
        Client::trusting(AGENT, vec![authority.clone()]).with_proxies(through(proxy_address))
    };
    for (challenges, named) in [
        (&["Negotiate"][..], "Negotiate (Kerberos)"),
        (&["NTLM"], "NTLM"),
        (&["Negotiate", "NTLM"], "Negotiate (Kerberos) or NTLM"),
    ] {
        let proxy = Proxy::start(ProxyOptions {
            challenges: challenges.iter().map(|c| (*c).to_owned()).collect(),
            ..ProxyOptions::default()
        })
        .unwrap();
        // With a login in its address and without: the same, for the
        // login goes as Basic, which the proxy does not take.
        for address_of_proxy in [proxy.url(), proxy.url_with("carol", Some("proxy-secret"))] {
            let err = get(&client(&address_of_proxy), &address, 10_000).unwrap_err();
            assert_eq!(
                err,
                FetchError::Proxy(format!(
                    "the proxy at 127.0.0.1:{} asks to be signed in to by {named}, which the \
                     application's own connections do not do: they give a proxy a user name \
                     and a password (Basic), from its address, and nothing else",
                    proxy.port()
                )),
                "{challenges:?}"
            );
            assert!(!err.to_string().contains("proxy-secret"));
        }
    }
    assert!(server.asked().is_empty(), "nothing reached the server");

    // One that offers Basic beside them: the login of its address passes,
    // and without one the address is said to need it.
    let proxy = Proxy::start(ProxyOptions {
        login: Some(("carol".to_owned(), "proxy-secret".to_owned())),
        challenges: vec![
            "Negotiate".to_owned(),
            "NTLM".to_owned(),
            "Basic realm=\"corporate\"".to_owned(),
        ],
        ..ProxyOptions::default()
    })
    .unwrap();
    let right = client(&proxy.url_with("carol", Some("proxy-secret")));
    assert_eq!(get(&right, &address, 10_000).unwrap().status, 200);
    let err = get(&client(&proxy.url()), &address, 10_000).unwrap_err();
    assert!(
        matches!(&err, FetchError::Proxy(said) if said.contains("asks for a user name and a password")),
        "{err:?}"
    );
}

#[test]
fn a_proxy_that_refuses_cannot_reach_is_not_there_or_cannot_be_used_is_said() {
    let (server, authority) = tls_stand_in(&["api.example.test"], plain_answer("inside"));
    let client =
        |proxies: Proxies| Client::trusting(AGENT, vec![authority.clone()]).with_proxies(proxies);
    let address = format!("https://api.example.test:{}/user", server.port());

    let refusing = Proxy::start(ProxyOptions {
        login: None,
        refuse: true,
        challenges: Vec::new(),
    })
    .unwrap();
    let err = get(&client(through(&refusing.url())), &address, 10_000).unwrap_err();
    assert_eq!(
        err,
        FetchError::Proxy(format!(
            "the proxy at 127.0.0.1:{} refused the connection to api.example.test:{} (403)",
            refusing.port(),
            server.port()
        ))
    );

    // The proxy is there; the host's port has nothing behind it.
    let proxy = Proxy::start(ProxyOptions::default()).unwrap();
    let nowhere = format!("https://api.example.test:{}/user", free_port());
    let err = get(&client(through(&proxy.url())), &nowhere, 10_000).unwrap_err();
    assert!(
        matches!(&err, FetchError::Connect(said) if said.contains("could not reach") && said.contains("(502)")),
        "{err:?}"
    );

    // No proxy where one was named: no connection, and no going around.
    let gone = free_port();
    let started = Instant::now();
    let err = get(
        &client(through(&format!("http://127.0.0.1:{gone}"))),
        &address,
        10_000,
    )
    .unwrap_err();
    assert!(
        matches!(&err, FetchError::Connect(said) if said.starts_with(&format!("the proxy at 127.0.0.1:{gone}"))),
        "{err:?}"
    );
    assert!(started.elapsed() < Duration::from_secs(5));

    // A proxy that cannot be used is never asked, nor is the host.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let socks = Proxies::from_values(
        |name| {
            (name == "HTTPS_PROXY").then(|| format!("socks5://{}", listener.local_addr().unwrap()))
        },
        || None,
    );
    let err = get(&client(socks), &address, 10_000).unwrap_err();
    assert!(
        matches!(&err, FetchError::Proxy(said) if said.starts_with("the proxy named in HTTPS_PROXY cannot be used") && said.contains("SOCKS")),
        "{err:?}"
    );
    assert!(listener.accept().is_err(), "nothing connected");
    assert_eq!(server.asked().len(), 0);
}

#[test]
fn this_machine_and_the_hosts_of_no_proxy_are_asked_directly() {
    let (server, authority) = tls_stand_in(&["localhost"], plain_answer("direct"));
    let proxy = Proxy::start(ProxyOptions::default()).unwrap();
    let client = Client::trusting(AGENT, vec![authority]).with_proxies(through(&proxy.url()));
    let address = format!("https://localhost:{}/x", server.port());
    assert_eq!(get(&client, &address, 10_000).unwrap().body, b"direct");
    let plain = serve(|stream, _| {
        let _ = stream.write_all(&crate::common::answer("200 OK", "text/plain", b"plain"));
    });
    assert_eq!(
        get(&client, &format!("{}/y", plain.base()), 10_000)
            .unwrap()
            .body,
        b"plain"
    );
    assert!(proxy.passages().is_empty(), "{:?}", proxy.passages());

    // A host of no_proxy: which route it takes, without a name looked up.
    let bypassing = through(&proxy.url()).bypassing(NoProxy::parse(".example.test"));
    let url = Url::parse("https://api.example.test/").unwrap();
    assert_eq!(
        bypassing.route(&url),
        bitbraid_integrations::http::Route::Direct
    );
    let other = Url::parse("https://api.example.org/").unwrap();
    assert!(matches!(
        bypassing.route(&other),
        bitbraid_integrations::http::Route::Through(_)
    ));
}

#[test]
fn a_hostile_proxy_ends_the_request_within_the_time_allowed() {
    let client = |port: u16| {
        Client::trusting(AGENT, vec![b"unused".to_vec()])
            .with_proxies(through(&format!("http://127.0.0.1:{port}")))
    };
    let address = "https://api.example.test:443/x";
    let check = |name: &str, port: u16, total_ms: u64, wanted: &dyn Fn(&FetchError) -> bool| {
        let started = Instant::now();
        let err = get(&client(port), address, total_ms).unwrap_err();
        assert!(wanted(&err), "{name}: {err:?}");
        assert!(
            started.elapsed() < Duration::from_millis(total_ms + 2000),
            "{name}: took {:?}",
            started.elapsed()
        );
    };

    let silent = serve(|_, _| std::thread::sleep(Duration::from_secs(5)));
    check("silent", silent.address.port(), 700, &|err| {
        *err == FetchError::Timeout
    });

    let nonsense = serve(|stream, _| {
        let _ = stream.write_all(b"nonsense\r\n\r\n");
    });
    check("nonsense", nonsense.address.port(), 5000, &|err| {
        matches!(err, FetchError::Malformed(_))
    });

    let endless = serve(|stream, _| {
        write_forever(
            stream,
            b"HTTP/1.1 200 OK\r\nX: yyyyyyyy\r\n",
            Duration::ZERO,
            100_000,
        );
    });
    check("endless", endless.address.port(), 5000, &|err| {
        matches!(err, FetchError::Malformed(_) | FetchError::Broken(_))
    });

    let trickle = serve(|stream, _| {
        write_forever(stream, b"H", Duration::from_millis(50), 1000);
    });
    check("trickle", trickle.address.port(), 800, &|err| {
        *err == FetchError::Timeout
    });

    let closing = serve(|_, _| {});
    check("closing", closing.address.port(), 5000, &|err| {
        matches!(err, FetchError::Proxy(said) if said.contains("without an answer"))
            || matches!(err, FetchError::Broken(_))
    });

    // A tunnel that opens onto something that is not TLS.
    let not_tls = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.1 200 Connection established\r\n\r\nthis is not tls\r\n");
    });
    check("not tls", not_tls.address.port(), 5000, &|err| {
        matches!(err, FetchError::Tls(_) | FetchError::Broken(_))
    });

    // A request nobody wants any more is not made.
    let stop = AtomicBool::new(true);
    let url = Url::parse(address).unwrap();
    let request = Request {
        url: &url,
        accept: "*/*",
        limits: limits(5000),
    };
    assert_eq!(
        client(silent.address.port()).get(&request, &stop),
        Err(FetchError::Cancelled)
    );
}

// --------------------------------------- what talks to the providers

pub(crate) fn recording(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/recordings")
        .join(name)
}

/// What a recording answers to a request, as a server would send it.
pub(crate) fn replayed(replay: &Replay, asked: &Asked) -> Answer {
    let host = asked.header("host").unwrap_or("example.test");
    let Ok(url) = Url::parse(&format!("https://{host}{}", asked.target)) else {
        return Answer {
            status: 400,
            ..Answer::default()
        };
    };
    let method: Method = serde_json::from_value(json!(asked.method)).unwrap();
    let mut call = Call::new(method, url, limits(10_000));
    if !asked.body.is_empty() {
        call = call.with_body(
            asked.header("content-type").unwrap_or("application/json"),
            asked.body.clone(),
        );
    }
    match replay.send(&call, &AtomicBool::new(false)) {
        // The recordings' GitLab is at 127.0.0.1:8929; its addresses in
        // the answers become this server's.
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

/// A server for `name` that answers from the recording `file`, the
/// services of a scratch directory whose client goes through `proxy`.
fn recorded_through(name: &str, file: &str, proxy: &Proxy) -> (StandIn, Arc<Services>, PathBuf) {
    let replay = Arc::new(Replay::load(&recording(file)).unwrap());
    let (server, authority) = tls_stand_in(&[name], move |asked| replayed(&replay, asked));
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("bb-l-proxied-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let client = Client::trusting(AGENT, vec![authority]).with_proxies(through(&proxy.url()));
    let services = Services::new(Arc::new(client), Arc::new(MemoryStore::new()), &dir);
    (server, Arc::new(services), dir)
}

pub(crate) fn run(
    services: &Arc<Services>,
    dir: &Path,
    id: &str,
    params: Value,
) -> Result<Value, String> {
    let mut ctx = CommandContext {
        config_dir: Some(dir.to_path_buf()),
        ..Default::default()
    };
    ctx.extensions.insert(services.clone());
    bitbraid_integrations::registry()
        .run(id, &mut ctx, &params)
        .map_err(|err| err.to_string())
}

#[test]
fn provider_commands_and_signing_in_go_through_the_proxy_to_a_recorded_server() {
    let proxy = Proxy::start(ProxyOptions::default()).unwrap();
    let name = "gitlab.example.test";
    let (server, services, dir) =
        recorded_through(name, "gitlab/add_token_and_pr_list.json", &proxy);
    let web = format!("https://{name}:{}", server.port());
    let account = run(
        &services,
        &dir,
        "account.add_token",
        json!({ "kind": "gitlab", "web": web, "token": "glpat-replayed" }),
    )
    .unwrap();
    assert_eq!(account["name"], "User 1");
    assert_eq!(account["api"], format!("{web}/api/v4"));
    let listing = run(
        &services,
        &dir,
        "pr.list",
        json!({ "account": account["id"], "owner": "bb-p-group",
                "name": "bb-p-project", "state": "all" }),
    )
    .unwrap();
    assert!(
        !listing["items"].as_array().unwrap().is_empty(),
        "{listing}"
    );
    let tunnel = format!("{name}:{}", server.port());
    let passages = proxy.passages();
    assert!(passages.len() >= 2, "{passages:?}");
    assert!(
        passages
            .iter()
            .all(|p| p.method == "CONNECT" && p.target == tunnel && p.outcome == Outcome::Passed),
        "{passages:?}"
    );
    assert!(server.asked().iter().any(|a| a.target == "/api/v4/user"));
    let _ = std::fs::remove_dir_all(&dir);

    // Signing in by code, at a server of one's own.
    let (server, services, dir) = recorded_through(name, "gitlab/device_flow.json", &proxy);
    let web = format!("https://{name}:{}", server.port());
    let id = "f4d2cf3c9a724e6b34744588be1d48b12f0badbfd455d0cd63f2e53195e5315c";
    let code = run(
        &services,
        &dir,
        "account.sign_in_start",
        json!({ "kind": "gitlab", "web": web, "client_id": id }),
    )
    .unwrap();
    assert_eq!(code["user_code"], "LE4SP3QG");
    assert_eq!(
        server.asked().last().map(|a| a.target.as_str()),
        Some("/oauth/authorize_device")
    );
    assert!(
        proxy
            .passages()
            .last()
            .is_some_and(|p| p.target == format!("{name}:{}", server.port()))
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn pictures_are_asked_through_the_proxy() {
    let proxy = Proxy::start(ProxyOptions::default()).unwrap();
    let (server, authority) = tls_stand_in(&["pictures.example.test"], |_| Answer {
        status: 404,
        headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        body: b"no such picture".to_vec(),
    });
    let client = Client::trusting(AGENT, vec![authority]).with_proxies(through(&proxy.url()));
    let service = Service::new(Some(&format!(
        "https://pictures.example.test:{}/avatar",
        server.port()
    )))
    .unwrap();
    let avatars = Avatars::new(Arc::new(client), service, None);
    assert_eq!(
        avatars.picture("ada@example.org", &[32], &AtomicBool::new(false)),
        Picture::Absent
    );
    let asked = server.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].target.starts_with("/avatar/"), "{asked:?}");
    assert!(!asked[0].target.contains("ada"), "only the hash is sent");
    let passages = proxy.passages();
    assert_eq!(passages.len(), 1);
    assert_eq!(
        passages[0].target,
        format!("pictures.example.test:{}", server.port())
    );
}
