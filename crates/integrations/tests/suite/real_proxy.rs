//! The application's own client through a real proxy: Squid 6 in Docker
//! on this machine (`bitbraid_testkit::real_proxy`), with the recorded
//! GitLab and a picture service served over TLS behind it by an
//! authority of the test's own.
//!
//! - Squid with Basic (`basic_ncsa_auth`): the provider commands and the
//!   sign-in by code, and a picture, through tunnels Squid opens with the
//!   login of the proxy's address; no login, a wrong one, a port Squid
//!   opens no tunnel to, a host it refuses and a server it cannot reach,
//!   each said in the client's words.
//! - Squid that offers Negotiate (with a KDC of the test's own), NTLM
//!   (checked by Samba's `ntlm_auth`), or both, and no Basic: what the
//!   client says, which is that the proxy wants what it does not give
//!   (the client signs in to a proxy with Basic alone); and one that
//!   offers Basic beside them, which the login of its address passes.
//!
//! The servers go by names under `example.test`, which Squid alone
//! resolves, to the loopback address. Opt-in and never in ci: run with
//! `--ignored` and `BITBRAID_REAL_PROXY=1`, with Docker that runs
//! containers without sudo (the image, Ubuntu 24.04 with Squid, MIT
//! Kerberos and Samba, is built at the first run); otherwise each test
//! says that it was skipped:
//!
//! ```sh
//! BITBRAID_REAL_PROXY=1 \
//!     cargo test -p bitbraid-integrations --test suite real_proxy:: -- --ignored
//! ```
//!
//! Sources: squid.conf documentation (`auth_param`, `acl`, `http_access`),
//! RFC 9110 §9.3.6 (CONNECT), §11.7.1 (`Proxy-Authenticate`), §15.5.8
//! (407); RFC 4559 (Negotiate).

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use crate::http_proxy::{
    AGENT, free_port, get, plain_answer, recording, replayed, run, through, tls_stand_in,
};
use bitbraid_integrations::{
    Services,
    avatars::{Avatars, Outcome as Picture, Service},
    http::{Client, FetchError},
    replay::Replay,
    secrets::MemoryStore,
};
use bitbraid_testkit::{
    Answer, StandIn,
    real_proxy::{self, Kdc, Negotiate, Squid, SquidOptions},
};
use serde_json::json;

static COUNTER: AtomicU64 = AtomicU64::new(0);

const GITLAB: &str = "gitlab.example.test";
const PICTURES: &str = "pictures.example.test";
const API: &str = "api.example.test";
const REFUSED: &str = "refused.example.test";
const USER: &str = "carol";
const PASSWORD: &str = "proxy-secret";

fn scratch(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("bb-l-real-proxy-{name}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// A server for `GITLAB` answering from the recording `file`, and the
/// authority that vouches for it.
fn recorded(file: &str) -> (StandIn, Vec<u8>) {
    let replay = Arc::new(Replay::load(&recording(file)).unwrap());
    tls_stand_in(&[GITLAB], move |asked| replayed(&replay, asked))
}

/// Services whose client trusts `authority` and goes through `proxy`.
fn services(authority: Vec<u8>, proxy: &str, dir: &std::path::Path) -> Arc<Services> {
    let client = Client::trusting(AGENT, vec![authority]).with_proxies(through(proxy));
    Arc::new(Services::new(
        Arc::new(client),
        Arc::new(MemoryStore::new()),
        dir,
    ))
}

fn said(err: &FetchError) -> String {
    match err {
        FetchError::Proxy(said) | FetchError::Connect(said) => said.clone(),
        other => panic!("another error: {other:?}"),
    }
}

#[test]
#[ignore = "real proxy: needs BITBRAID_REAL_PROXY and Docker"]
fn squid_with_basic_carries_the_providers_and_the_pictures() {
    if !real_proxy::requested("the client through squid with basic") {
        return;
    }
    let (gitlab, gitlab_authority) = recorded("gitlab/add_token_and_pr_list.json");
    let (device, device_authority) = recorded("gitlab/device_flow.json");
    let (pictures, pictures_authority) = tls_stand_in(&[PICTURES], |_| Answer {
        status: 404,
        headers: vec![("Content-Type".to_owned(), "text/plain".to_owned())],
        body: b"no such picture".to_vec(),
    });
    let (api, api_authority) = tls_stand_in(&[API, REFUSED], plain_answer("inside"));
    // A port Squid opens tunnels to, with nothing behind it.
    let nowhere = free_port();
    let squid = Squid::start(&SquidOptions {
        basic: vec![(USER.to_owned(), PASSWORD.to_owned())],
        tunnel_ports: vec![
            gitlab.port(),
            device.port(),
            pictures.port(),
            api.port(),
            nowhere,
        ],
        refused_hosts: vec![REFUSED.to_owned()],
        names: vec![
            GITLAB.to_owned(),
            PICTURES.to_owned(),
            API.to_owned(),
            REFUSED.to_owned(),
        ],
        ..SquidOptions::default()
    })
    .unwrap();
    let with_login = squid.url_with(USER, Some(PASSWORD));

    // A provider's reads: an account by its token, then its merge
    // requests.
    let dir = scratch("providers");
    let services_here = services(gitlab_authority.clone(), &with_login, &dir);
    let web = format!("https://{GITLAB}:{}", gitlab.port());
    let account = run(
        &services_here,
        &dir,
        "account.add_token",
        json!({ "kind": "gitlab", "web": web, "token": "glpat-replayed" }),
    )
    .unwrap_or_else(|err| panic!("{err}\n{}", squid.cache_log()));
    assert_eq!(account["name"], "User 1");
    let listing = run(
        &services_here,
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
    let tunnel = format!("{GITLAB}:{}", gitlab.port());
    let log = squid.access_log();
    assert!(log.len() >= 2, "{log:#?}");
    // One request to a connection, the login sent with the CONNECT before
    // Squid asks: every tunnel opened at once, as carol.
    assert!(
        log.iter().all(|l| l.method == "CONNECT"
            && l.target == tunnel
            && l.result == "TCP_TUNNEL"
            && l.status == 200
            && l.user.as_deref() == Some(USER)),
        "{log:#?}"
    );
    assert!(gitlab.asked().iter().any(|a| a.target == "/api/v4/user"));
    let _ = std::fs::remove_dir_all(&dir);

    // Signing in by code.
    let dir = scratch("device");
    let services_here = services(device_authority, &with_login, &dir);
    let code = run(
        &services_here,
        &dir,
        "account.sign_in_start",
        json!({ "kind": "gitlab", "web": format!("https://{GITLAB}:{}", device.port()),
                "client_id": "f4d2cf3c9a724e6b34744588be1d48b12f0badbfd455d0cd63f2e53195e5315c" }),
    )
    .unwrap();
    assert_eq!(code["user_code"], "LE4SP3QG");
    let _ = std::fs::remove_dir_all(&dir);

    // A picture.
    let before = squid.access_log().len();
    let client =
        Client::trusting(AGENT, vec![pictures_authority]).with_proxies(through(&with_login));
    let service = Service::new(Some(&format!(
        "https://{PICTURES}:{}/avatar",
        pictures.port()
    )))
    .unwrap();
    let avatars = Avatars::new(Arc::new(client), service, None);
    assert_eq!(
        avatars.picture("ada@example.org", &[32], &AtomicBool::new(false)),
        Picture::Absent
    );
    assert_eq!(pictures.asked().len(), 1);
    let log: Vec<_> = squid.access_log().into_iter().skip(before).collect();
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(log[0].target, format!("{PICTURES}:{}", pictures.port()));
    assert_eq!(log[0].user.as_deref(), Some(USER));

    // What turns the client away, said.
    let client = |proxy: &str| {
        Client::trusting(AGENT, vec![api_authority.clone()]).with_proxies(through(proxy))
    };
    let address = |host: &str, port: u16| format!("https://{host}:{port}/user");
    let place = format!("127.0.0.1:{}", squid.port());
    let right = get(&client(&with_login), &address(API, api.port()), 10_000).unwrap();
    assert_eq!((right.status, right.body.as_slice()), (200, &b"inside"[..]));

    let err = get(&client(&squid.url()), &address(API, api.port()), 10_000).unwrap_err();
    assert_eq!(
        said(&err),
        format!(
            "the proxy at {place} asks for a user name and a password, and its address gives \
             none (http://name:password@host:port)"
        )
    );
    let err = get(
        &client(&squid.url_with(USER, Some("not-it"))),
        &address(API, api.port()),
        10_000,
    )
    .unwrap_err();
    assert_eq!(
        said(&err),
        format!("the proxy at {place} refused the user name and password its address gives")
    );
    assert!(!err.to_string().contains("not-it"));
    // A port Squid opens no tunnel to (refused before the login is asked
    // for, so nothing need listen there); a host it refuses; a port it
    // opens tunnels to with nothing behind it.
    let unlisted = free_port();
    let err = get(&client(&with_login), &address(API, unlisted), 10_000).unwrap_err();
    assert_eq!(
        err,
        FetchError::Proxy(format!(
            "the proxy at {place} refused the connection to {API}:{unlisted} (403)"
        ))
    );
    let err = get(&client(&with_login), &address(REFUSED, api.port()), 10_000).unwrap_err();
    assert_eq!(
        err,
        FetchError::Proxy(format!(
            "the proxy at {place} refused the connection to {REFUSED}:{} (403)",
            api.port()
        ))
    );
    let err = get(&client(&with_login), &address(API, nowhere), 10_000).unwrap_err();
    assert_eq!(
        err,
        FetchError::Connect(format!(
            "the proxy at {place} could not reach {API}:{nowhere} (503)"
        ))
    );
    // Squid's own pages, as its log has them.
    let results: Vec<(String, u16, Option<String>)> = squid
        .access_log()
        .into_iter()
        .filter(|l| l.target.starts_with(API) || l.target.starts_with(REFUSED))
        .map(|l| (l.result, l.status, l.user))
        .collect();
    let carol = Some(USER.to_owned());
    assert_eq!(
        results,
        [
            ("TCP_TUNNEL".to_owned(), 200, carol.clone()),
            ("TCP_DENIED".to_owned(), 407, None),
            ("TCP_DENIED".to_owned(), 407, carol.clone()),
            ("TCP_DENIED".to_owned(), 403, None),
            ("TCP_DENIED".to_owned(), 403, carol.clone()),
            ("TCP_TUNNEL".to_owned(), 503, carol),
        ]
    );
}

#[test]
#[ignore = "real proxy: needs BITBRAID_REAL_PROXY and Docker"]
fn squid_that_takes_negotiate_or_ntlm_only_is_said_to_want_what_the_client_does_not_give() {
    if !real_proxy::requested("the client through squid with negotiate or ntlm") {
        return;
    }
    let (api, authority) = tls_stand_in(&[API], plain_answer("inside"));
    let address = format!("https://{API}:{}/user", api.port());
    let kdc = Kdc::start("BITBRAID.TEST", &[], &["HTTP/proxy.bitbraid.test"]).unwrap();
    let negotiate = Negotiate {
        keytab: kdc.keytab(),
        principal: "HTTP/proxy.bitbraid.test@BITBRAID.TEST".to_owned(),
        krb5_conf: kdc.config(),
    };
    let base = SquidOptions {
        tunnel_ports: vec![api.port()],
        names: vec![API.to_owned()],
        ..SquidOptions::default()
    };
    for (options, named) in [
        (
            SquidOptions {
                negotiate: Some(negotiate.clone()),
                ..base.clone()
            },
            "Negotiate (Kerberos)",
        ),
        (
            SquidOptions {
                ntlm: vec![(USER.to_owned(), PASSWORD.to_owned())],
                ..base.clone()
            },
            "NTLM",
        ),
        (
            SquidOptions {
                negotiate: Some(negotiate.clone()),
                ntlm: vec![(USER.to_owned(), PASSWORD.to_owned())],
                ..base.clone()
            },
            "Negotiate (Kerberos) or NTLM",
        ),
    ] {
        let squid = Squid::start(&options).unwrap();
        for proxy in [squid.url(), squid.url_with(USER, Some(PASSWORD))] {
            let client =
                Client::trusting(AGENT, vec![authority.clone()]).with_proxies(through(&proxy));
            let err = get(&client, &address, 10_000).unwrap_err();
            assert_eq!(
                err,
                FetchError::Proxy(format!(
                    "the proxy at 127.0.0.1:{} asks to be signed in to by {named}, which the \
                     application's own connections do not do: they give a proxy a user name \
                     and a password (Basic), from its address, and nothing else",
                    squid.port()
                )),
                "{named}"
            );
        }
        let log = squid.access_log();
        assert_eq!(log.len(), 2, "{log:#?}");
        assert!(log.iter().all(|l| l.status == 407), "{log:#?}");
    }
    assert!(api.asked().is_empty());

    // All three offered, as a proxy of an Active Directory domain often
    // does: the login of the proxy's address passes, as Basic.
    let squid = Squid::start(&SquidOptions {
        negotiate: Some(negotiate),
        ntlm: vec![(USER.to_owned(), PASSWORD.to_owned())],
        basic: vec![(USER.to_owned(), PASSWORD.to_owned())],
        ..base
    })
    .unwrap();
    let client = Client::trusting(AGENT, vec![authority])
        .with_proxies(through(&squid.url_with(USER, Some(PASSWORD))));
    assert_eq!(get(&client, &address, 10_000).unwrap().status, 200);
    let log = squid.access_log();
    assert_eq!(log.len(), 1, "{log:#?}");
    assert_eq!(log[0].user.as_deref(), Some(USER));
}
