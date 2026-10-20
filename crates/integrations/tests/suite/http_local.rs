//! The real client against servers on this machine, the well-behaved and
//! the hostile: one that never answers, one that answers a byte at a time,
//! one whose body or head never ends, one that sends nonsense, one that
//! goes away. Each has to end in an error within the time allowed, and
//! none in a wait without end or a buffer without bound.
//!
//! Plain HTTP throughout: it is the same code as over TLS from the first
//! byte of the request on (`tests/suite/http_tls.rs` covers what is under
//! it).

use std::{
    io::Write as _,
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::common::{answer, serve, write_forever};
use bitbraid_integrations::http::{
    Call, Client, Fetch, FetchError, Limits, Method, Request, Response, Url,
};

const AGENT: &str = "BitBraid-Test/1.0";

fn limits(total_ms: u64, body: usize) -> Limits {
    Limits {
        connect: Duration::from_secs(5),
        total: Duration::from_millis(total_ms),
        body,
    }
}

fn get(address: &str, limits: Limits) -> Result<Response, FetchError> {
    let url = Url::parse(address).unwrap();
    let request = Request {
        url: &url,
        accept: "image/png, image/jpeg",
        limits,
    };
    Client::new(AGENT).get(&request, &AtomicBool::new(false))
}

#[test]
fn an_answer_is_read_and_the_request_says_nothing_it_should_not() {
    let server = serve(|stream, _| {
        let _ = stream.write_all(&answer("200 OK", "image/png", b"the body"));
    });
    let response = get(
        &format!("{}/avatar/abc?s=64&d=404", server.base()),
        limits(5000, 100),
    )
    .unwrap();
    assert_eq!(response.status, 200);
    assert_eq!(response.content_type.as_deref(), Some("image/png"));
    assert_eq!(response.body, b"the body");

    // Everything that was sent, line by line: the address, the host, the
    // application's name, what is welcome, and how the answer should come.
    // No cookie, no referrer, no name of a person or a machine.
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0],
        format!(
            "GET /avatar/abc?s=64&d=404 HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: {AGENT}\r\n\
             Accept: image/png, image/jpeg\r\n\
             Accept-Encoding: identity\r\n\
             Connection: close\r\n\r\n",
            server.address
        )
    );
}

#[test]
fn a_body_in_chunks_and_one_that_ends_with_the_connection_are_read() {
    let chunked = serve(|stream, _| {
        let _ =
            stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n");
        let _ = stream.flush();
        thread::sleep(Duration::from_millis(30));
        let _ = stream.write_all(b"4\r\ndefg\r\n0\r\n\r\n");
    });
    assert_eq!(
        get(&chunked.base(), limits(5000, 100)).unwrap().body,
        b"abcdefg"
    );

    let closing = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.0 200 OK\r\n\r\nuntil the end");
    });
    assert_eq!(
        get(&closing.base(), limits(5000, 100)).unwrap().body,
        b"until the end"
    );
}

#[test]
fn a_redirect_is_handed_back_and_not_followed() {
    let server = serve(|stream, _| {
        let _ = stream
            .write_all(b"HTTP/1.1 302 Found\r\nLocation: /elsewhere\r\nContent-Length: 0\r\n\r\n");
    });
    let response = get(&format!("{}/first", server.base()), limits(5000, 100)).unwrap();
    assert_eq!(response.status, 302);
    assert_eq!(response.location.as_deref(), Some("/elsewhere"));
    // One request: nobody went elsewhere.
    assert_eq!(server.lines(), ["GET /first HTTP/1.1"]);
}

#[test]
fn a_server_that_never_answers_is_given_up_in_time() {
    let server = serve(|_, _| thread::sleep(Duration::from_secs(15)));
    let started = Instant::now();
    assert_eq!(
        get(&server.base(), limits(400, 100)),
        Err(FetchError::Timeout)
    );
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(350), "{took:?}");
    assert!(took < Duration::from_secs(3), "{took:?}");
}

#[test]
fn a_server_that_does_not_even_take_the_request_is_given_up_in_time() {
    // It listens and never accepts: the connection is made by the system,
    // and then nothing.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let started = Instant::now();
    assert_eq!(
        get(&format!("http://{address}/"), limits(400, 100)),
        Err(FetchError::Timeout)
    );
    assert!(started.elapsed() < Duration::from_secs(3));
    drop(listener);
}

#[test]
fn an_answer_that_trickles_is_cut_off_when_the_time_is_up() {
    // A byte every 40 ms: each wait is short and something always comes,
    // and still the whole may not take longer than was allowed. Once in
    // the head and once in the body.
    let slow_head = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n");
        write_forever(stream, b"X", Duration::from_millis(40), 400);
    });
    let slow_body = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 90\r\n\r\n");
        write_forever(stream, b"x", Duration::from_millis(40), 400);
    });
    for server in [&slow_head, &slow_body] {
        let started = Instant::now();
        assert_eq!(
            get(&server.base(), limits(500, 100)),
            Err(FetchError::Timeout)
        );
        let took = started.elapsed();
        assert!(took >= Duration::from_millis(450), "{took:?}");
        assert!(took < Duration::from_secs(3), "{took:?}");
    }
}

#[test]
fn a_body_without_end_is_refused_at_the_size_that_is_taken() {
    let piece = vec![b'x'; 16 * 1024];
    // Until the connection closes, which it never does.
    let endless = {
        let piece = piece.clone();
        serve(move |stream, _| {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\n\r\n");
            write_forever(stream, &piece, Duration::ZERO, 100_000);
        })
    };
    // In chunks that never stop coming.
    let chunks = {
        let piece = piece.clone();
        serve(move |stream, _| {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
            let mut chunk = b"4000\r\n".to_vec();
            chunk.extend_from_slice(&piece);
            chunk.extend_from_slice(b"\r\n");
            write_forever(stream, &chunk, Duration::ZERO, 100_000);
        })
    };
    // A length that says so up front.
    let declared = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4000000000\r\n\r\n");
        write_forever(stream, &[b'x'; 1024], Duration::ZERO, 100_000);
    });
    for server in [&endless, &chunks, &declared] {
        let started = Instant::now();
        assert_eq!(
            get(&server.base(), limits(10_000, 200_000)),
            Err(FetchError::TooLarge)
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

#[test]
fn a_head_without_end_is_refused() {
    let server = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n");
        write_forever(
            stream,
            b"X-More: and more and more and more and more\r\n",
            Duration::ZERO,
            1_000_000,
        );
    });
    let one_line = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nX-Long: ");
        write_forever(stream, &[b'a'; 4096], Duration::ZERO, 100_000);
    });
    for server in [&server, &one_line] {
        let started = Instant::now();
        assert!(matches!(
            get(&server.base(), limits(10_000, 100)),
            Err(FetchError::Malformed(_))
        ));
        assert!(started.elapsed() < Duration::from_secs(5));
    }
}

#[test]
fn nonsense_is_refused() {
    for nonsense in [
        &b"\x00\x01\x02\x03\xff\xfe\xfd\n\n\r\n\r\n"[..],
        b"SSH-2.0-OpenSSH_9.9\r\n",
        b"<html><body>captive portal</body></html>\r\n\r\n",
        b"HTTP/1.1 two hundred\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Length: many\r\n\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nnot a size\r\n",
        b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 3\r\n\r\nabc",
    ] {
        let bytes = nonsense.to_vec();
        let server = serve(move |stream, _| {
            let _ = stream.write_all(&bytes);
        });
        let result = get(&server.base(), limits(5000, 100));
        assert!(
            matches!(result, Err(FetchError::Malformed(_))),
            "{:?} gave {result:?}",
            String::from_utf8_lossy(nonsense)
        );
    }
}

#[test]
fn a_server_that_goes_away_is_an_error() {
    // Closes without a word.
    let silent = serve(|_, _| {});
    assert_eq!(
        get(&silent.base(), limits(5000, 100)),
        Err(FetchError::Malformed("nothing came back"))
    );
    // Closes in the middle of what it promised.
    let short = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 50\r\n\r\nonly this");
    });
    assert_eq!(
        get(&short.base(), limits(5000, 100)),
        Err(FetchError::Malformed("the body ends early"))
    );
    // Nobody listens there at all: a port that was just given back.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    drop(listener);
    assert!(matches!(
        get(&format!("http://{address}/"), limits(5000, 100)),
        Err(FetchError::Connect(_))
    ));
}

#[test]
fn a_request_that_is_no_longer_wanted_ends_at_once() {
    let server = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1000\r\n\r\n");
        write_forever(stream, b"x", Duration::from_millis(50), 400);
    });
    let url = Url::parse(&server.base()).unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        stop.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    let request = Request {
        url: &url,
        accept: "*/*",
        limits: limits(20_000, 2000),
    };
    assert_eq!(
        Client::new(AGENT).get(&request, &cancel),
        Err(FetchError::Cancelled)
    );
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(250), "{took:?}");
    assert!(took < Duration::from_millis(1500), "{took:?}");
}

#[test]
fn localhost_by_name_is_this_machine_too() {
    let server = serve(|stream, _| {
        let _ = stream.write_all(&answer("200 OK", "text/plain", b"by name"));
    });
    let address = format!("http://localhost:{}/", server.address.port());
    assert_eq!(get(&address, limits(5000, 100)).unwrap().body, b"by name");
    assert_eq!(
        server.requests()[0].lines().nth(1),
        Some(format!("Host: localhost:{}", server.address.port()).as_str())
    );
}

fn call(method: Method, address: &str) -> Call {
    Call::new(method, Url::parse(address).unwrap(), limits(5000, 10_000))
}

#[test]
fn a_request_with_a_method_header_lines_and_a_body_is_sent_whole() {
    let bodies: Arc<std::sync::Mutex<Vec<Vec<u8>>>> = Arc::default();
    let kept = bodies.clone();
    let server = serve(move |stream, head| {
        let length: usize = head
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .map_or(0, |length| length.trim().parse().unwrap());
        let mut body = vec![0u8; length];
        std::io::Read::read_exact(stream, &mut body).unwrap();
        kept.lock().unwrap().push(body);
        let _ = stream.write_all(
            b"HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nETag: \"v1\"\r\n\
              Link: </next?page=2>; rel=\"next\"\r\nX-RateLimit-Remaining: 12\r\n\
              Set-Cookie: tracking=1\r\nContent-Length: 2\r\n\r\n{}",
        );
    });
    let posted = call(
        Method::Post,
        &format!("{}/api/v4/things?x=1", server.base()),
    )
    .with_header("Accept", "application/json")
    .with_header("Authorization", "Bearer not-a-real-token")
    .with_body("application/json", br#"{"title":"one"}"#.to_vec());
    let response = Client::new(AGENT)
        .send(&posted, &AtomicBool::new(false))
        .unwrap();
    assert_eq!(response.status, 201);
    assert_eq!(response.body, b"{}");
    assert_eq!(response.header("etag"), Some("\"v1\""));
    assert_eq!(
        response.header("link"),
        Some("</next?page=2>; rel=\"next\"")
    );
    assert_eq!(response.header("x-ratelimit-remaining"), Some("12"));
    assert_eq!(response.header("set-cookie"), None);
    assert_eq!(*bodies.lock().unwrap(), [br#"{"title":"one"}"#.to_vec()]);
    assert_eq!(
        server.requests()[0],
        format!(
            "POST /api/v4/things?x=1 HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: {AGENT}\r\n\
             Accept: application/json\r\n\
             Authorization: Bearer not-a-real-token\r\n\
             Content-Type: application/json\r\n\
             Accept-Encoding: identity\r\n\
             Connection: close\r\n\
             Content-Length: 15\r\n\r\n",
            server.address
        )
    );

    // Each method by its name; one that sends nothing still says so.
    let deleted = serve(|stream, _| {
        let _ = stream.write_all(b"HTTP/1.1 204 No Content\r\n\r\n");
    });
    for method in [Method::Delete, Method::Put, Method::Patch, Method::Get] {
        let response = Client::new(AGENT)
            .send(&call(method, &deleted.base()), &AtomicBool::new(false))
            .unwrap();
        assert_eq!(response.status, 204);
    }
    let heads = deleted.requests();
    assert!(heads[0].starts_with("DELETE / HTTP/1.1\r\n"));
    assert!(!heads[0].contains("Content-Length"));
    assert!(heads[1].starts_with("PUT / HTTP/1.1\r\n"));
    assert!(heads[1].contains("Content-Length: 0\r\n"));
    assert!(heads[2].starts_with("PATCH / HTTP/1.1\r\n"));
    assert!(heads[3].starts_with("GET / HTTP/1.1\r\n"));
}

#[test]
fn header_lines_that_could_break_a_request_are_refused_before_anything_is_sent() {
    let server = serve(|stream, _| {
        let _ = stream.write_all(&answer("200 OK", "text/plain", b"no"));
    });
    for (name, value) in [
        ("Host", "elsewhere.example.org"),
        ("cookie", "a=b"),
        ("Content-Length", "5"),
        ("Transfer-Encoding", "chunked"),
        ("Connection", "keep-alive"),
        ("X-Ok", "a\r\nInjected: yes"),
        ("X-Ok", "line\nbreak"),
        ("X-Ok", ""),
        ("X-Ok", "caf\u{e9}"),
        ("Bad Name", "x"),
        ("Bad:Name", "x"),
        ("", "x"),
    ] {
        let refused = call(Method::Get, &server.base()).with_header(name, value);
        assert!(
            matches!(
                Client::new(AGENT).send(&refused, &AtomicBool::new(false)),
                Err(FetchError::Address(_))
            ),
            "{name:?}: {value:?}"
        );
    }
    let long = "v".repeat(9000);
    let refused = call(Method::Get, &server.base()).with_header("X-Long", &long);
    assert!(
        Client::new(AGENT)
            .send(&refused, &AtomicBool::new(false))
            .is_err()
    );
    let mut many = call(Method::Get, &server.base());
    for at in 0..40 {
        many = many.with_header(&format!("X-{at}"), "1");
    }
    assert!(
        Client::new(AGENT)
            .send(&many, &AtomicBool::new(false))
            .is_err()
    );
    // None of it reached the server.
    assert!(server.requests().is_empty());
}

#[test]
fn a_request_never_shows_its_secrets() {
    let secret = call(Method::Post, "https://gitlab.example.org/oauth/token")
        .with_header("Authorization", "Bearer glpat-very-secret")
        .with_header("PRIVATE-TOKEN", "glpat-also-secret")
        .with_header(" authorization", "token ghp_third_secret")
        .with_header("Accept", "application/json")
        .with_body(
            "application/x-www-form-urlencoded",
            b"refresh_token=refresh-secret".to_vec(),
        );
    let text = format!("{secret:?}");
    for hidden in [
        "very-secret",
        "also-secret",
        "third_secret",
        "refresh-secret",
    ] {
        assert!(!text.contains(hidden), "{text}");
    }
    assert!(text.contains("<hidden>") && text.contains("application/json"));
    assert!(text.contains("<28 bytes>"), "{text}");
    let response = Response {
        status: 200,
        body: br#"{"access_token":"gho_secret"}"#.to_vec(),
        ..Default::default()
    };
    assert!(!format!("{response:?}").contains("gho_secret"));
}
