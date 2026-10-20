//! The real client over TLS, against a server on this machine with a
//! certificate made for the test: that it is trusted when it should be,
//! refused when it should be (and that nothing was asked of a server that
//! was refused), and that the clock holds during the handshake too.
//!
//! Not covered anywhere: the system's own root store against a server it
//! does trust, which would take a real host on the network.

use std::{
    io::Write as _,
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex, atomic::AtomicBool},
    thread,
    time::{Duration, Instant},
};

use crate::common::{Heard, answer, read_head};
use bitbraid_integrations::http::{Client, Fetch, FetchError, Limits, Request, Response, Url};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};

const AGENT: &str = "BitBraid-Test/1.0";

/// A certificate for `localhost` that nobody signed but itself, and a
/// server configuration that presents it.
fn certificate() -> (Vec<u8>, Arc<ServerConfig>) {
    let made = rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let certificate: CertificateDer<'static> = made.cert.der().clone();
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(made.signing_key.serialize_der()));
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(vec![certificate.clone()], key)
        .unwrap();
    (certificate.to_vec(), Arc::new(config))
}

type Secured = StreamOwned<ServerConnection, TcpStream>;

/// A TLS server: `answer` gets each connection once its request was read.
fn serve_tls(
    config: Arc<ServerConfig>,
    answer: impl Fn(&mut Secured, &str) + Send + Sync + 'static,
) -> (SocketAddr, Heard) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let heard: Heard = Arc::new(Mutex::new(Vec::new()));
    let noted = heard.clone();
    let answer = Arc::new(answer);
    thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(socket) = socket else { continue };
            let (config, noted, answer) = (config.clone(), noted.clone(), answer.clone());
            thread::spawn(move || {
                let _ = socket.set_read_timeout(Some(Duration::from_secs(20)));
                let Ok(connection) = ServerConnection::new(config) else {
                    return;
                };
                let mut stream = StreamOwned::new(connection, socket);
                // A handshake that fails ends here: nothing was asked.
                let Some(head) = read_head(&mut stream) else {
                    return;
                };
                noted.lock().unwrap().push(head.clone());
                answer(&mut stream, &head);
            });
        }
    });
    (address, heard)
}

/// Say that this is all, as a server that ends properly does.
fn finish(stream: &mut Secured) {
    stream.conn.send_close_notify();
    let _ = stream.flush();
}

fn limits(total_ms: u64, body: usize) -> Limits {
    Limits {
        connect: Duration::from_secs(5),
        total: Duration::from_millis(total_ms),
        body,
    }
}

fn get(client: &Client, address: &str, limits: Limits) -> Result<Response, FetchError> {
    let url = Url::parse(address).unwrap();
    let request = Request {
        url: &url,
        accept: "image/png",
        limits,
    };
    client.get(&request, &AtomicBool::new(false))
}

#[test]
fn a_server_whose_certificate_is_trusted_is_asked_and_answers() {
    let (certificate, config) = certificate();
    let (address, heard) = serve_tls(config, |stream, _| {
        let _ = stream.write_all(&answer("200 OK", "image/png", b"over tls"));
        finish(stream);
    });
    let client = Client::trusting(AGENT, vec![certificate]);
    let url = format!("https://localhost:{}/avatar/abc?s=64&d=404", address.port());
    let response = get(&client, &url, limits(10_000, 100)).unwrap();
    assert_eq!(
        (response.status, response.body.as_slice()),
        (200, &b"over tls"[..])
    );
    // The same client asks again: what was set up for TLS is kept.
    assert_eq!(get(&client, &url, limits(10_000, 100)).unwrap().status, 200);
    let heard = heard.lock().unwrap().clone();
    assert_eq!(heard.len(), 2);
    assert_eq!(
        heard[0],
        format!(
            "GET /avatar/abc?s=64&d=404 HTTP/1.1\r\n\
             Host: localhost:{}\r\n\
             User-Agent: {AGENT}\r\n\
             Accept: image/png\r\n\
             Accept-Encoding: identity\r\n\
             Connection: close\r\n\r\n",
            address.port()
        )
    );
}

#[test]
fn a_body_of_many_records_is_read_and_one_too_large_refused() {
    let (certificate, config) = certificate();
    let (address, _) = serve_tls(config, |stream, head| {
        if head.starts_with("GET /large") {
            let body = vec![b'p'; 150_000];
            let _ = stream.write_all(&answer("200 OK", "image/png", &body));
        } else if head.starts_with("GET /chunked") {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n");
            for _ in 0..40 {
                let _ = stream.write_all(b"1000\r\n");
                let _ = stream.write_all(&[b'c'; 0x1000]);
                let _ = stream.write_all(b"\r\n");
                let _ = stream.flush();
            }
            let _ = stream.write_all(b"0\r\n\r\n");
        } else {
            let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n");
            for _ in 0..100_000 {
                if stream.write_all(&[b'e'; 4096]).is_err() {
                    break;
                }
            }
        }
        finish(stream);
    });
    let client = Client::trusting(AGENT, vec![certificate]);
    let at = |path: &str| format!("https://localhost:{}{path}", address.port());
    let large = get(&client, &at("/large"), limits(10_000, 200_000)).unwrap();
    assert_eq!(large.body.len(), 150_000);
    assert!(large.body.iter().all(|byte| *byte == b'p'));
    let chunked = get(&client, &at("/chunked"), limits(10_000, 200_000)).unwrap();
    assert_eq!(chunked.body.len(), 40 * 0x1000);
    assert_eq!(
        get(&client, &at("/large"), limits(10_000, 100_000)),
        Err(FetchError::TooLarge)
    );
    let started = Instant::now();
    assert_eq!(
        get(&client, &at("/endless"), limits(10_000, 100_000)),
        Err(FetchError::TooLarge)
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_certificate_nobody_vouches_for_is_refused_before_anything_is_asked() {
    let (_, config) = certificate();
    let (address, heard) = serve_tls(config, |stream, _| {
        let _ = stream.write_all(&answer("200 OK", "image/png", b"should not be read"));
        finish(stream);
    });
    // The client that trusts what the system trusts: this certificate is
    // not among that.
    let client = Client::new(AGENT);
    let url = format!("https://localhost:{}/avatar/secret-hash", address.port());
    let result = get(&client, &url, limits(10_000, 100));
    assert!(matches!(result, Err(FetchError::Tls(_))), "{result:?}");
    // Nor one that trusts another certificate of the same name.
    let (other, _) = certificate();
    let client = Client::trusting(AGENT, vec![other]);
    let result = get(&client, &url, limits(10_000, 100));
    assert!(matches!(result, Err(FetchError::Tls(_))), "{result:?}");
    // The server never saw a request: the address asked for, with the
    // hash in it, did not leave before the server was known.
    thread::sleep(Duration::from_millis(200));
    assert!(heard.lock().unwrap().is_empty());
}

#[test]
fn a_certificate_for_another_name_is_refused() {
    let (certificate, config) = certificate();
    let (address, heard) = serve_tls(config, |stream, _| {
        let _ = stream.write_all(&answer("200 OK", "image/png", b"x"));
        finish(stream);
    });
    // Trusted, and made out to `localhost`: asked for by its number it is
    // not the server that was meant.
    let client = Client::trusting(AGENT, vec![certificate]);
    let result = get(
        &client,
        &format!("https://127.0.0.1:{}/", address.port()),
        limits(10_000, 100),
    );
    assert!(matches!(result, Err(FetchError::Tls(_))), "{result:?}");
    thread::sleep(Duration::from_millis(200));
    assert!(heard.lock().unwrap().is_empty());
}

#[test]
fn a_handshake_that_never_comes_is_given_up_in_time() {
    // It takes the connection and says nothing, ever.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        let mut held = Vec::new();
        for socket in listener.incoming() {
            held.push(socket);
        }
    });
    let (certificate, _) = certificate();
    let client = Client::trusting(AGENT, vec![certificate]);
    let started = Instant::now();
    assert_eq!(
        get(
            &client,
            &format!("https://localhost:{}/", address.port()),
            limits(500, 100)
        ),
        Err(FetchError::Timeout)
    );
    let took = started.elapsed();
    assert!(took >= Duration::from_millis(450), "{took:?}");
    assert!(took < Duration::from_secs(3), "{took:?}");
}

#[test]
fn a_server_that_does_not_speak_tls_is_refused() {
    // Whatever it makes of a TLS greeting, what it says back is no
    // handshake.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(mut socket) = socket else { continue };
            let _ = socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\n\r\n");
        }
    });
    let (certificate, _) = certificate();
    let client = Client::trusting(AGENT, vec![certificate]);
    let result = get(
        &client,
        &format!("https://localhost:{}/", address.port()),
        limits(5000, 100),
    );
    assert!(matches!(result, Err(FetchError::Tls(_))), "{result:?}");
}

#[test]
fn a_connection_cut_without_a_goodbye_is_not_taken_for_the_end() {
    let (certificate, config) = certificate();
    let (address, _) = serve_tls(config, |stream, _| {
        // No length, so the body ends with the connection; and the
        // connection ends without TLS saying that it does. Somebody in
        // between could have cut it there.
        let _ = stream.write_all(b"HTTP/1.1 200 OK\r\n\r\nhalf a pictu");
        let _ = stream.flush();
    });
    let client = Client::trusting(AGENT, vec![certificate]);
    let result = get(
        &client,
        &format!("https://localhost:{}/", address.port()),
        limits(5000, 100),
    );
    assert!(matches!(result, Err(FetchError::Broken(_))), "{result:?}");
}

#[test]
fn a_client_with_certificates_that_are_none_asks_nothing() {
    // Windows takes a second or two to say that nothing listens on a port
    // of this machine (it asks again before it believes the refusal):
    // time enough for that, so that the refusal is what is seen.
    let client = Client::trusting(AGENT, vec![b"not a certificate".to_vec()]);
    let result = get(&client, "https://localhost:9/", limits(10_000, 100));
    assert!(
        matches!(
            result,
            Err(FetchError::Tls(_)) | Err(FetchError::Connect(_))
        ),
        "{result:?}"
    );
    let client = Client::trusting(AGENT, Vec::new());
    let result = get(&client, "https://localhost:9/", limits(10_000, 100));
    assert!(
        matches!(
            result,
            Err(FetchError::Tls(_)) | Err(FetchError::Connect(_))
        ),
        "{result:?}"
    );
}
