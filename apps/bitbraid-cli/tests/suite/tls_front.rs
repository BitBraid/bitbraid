//! HTTPS in front of a plain HTTP server on this machine, for the LFS
//! tests: an authority made for the test, a certificate it signs for
//! `127.0.0.1`, and a front that ends TLS and hands each request on to
//! the server behind it.
//!
//! An LFS server writes where content is to be sent and fetched as whole
//! addresses, of its own idea of where it is: a GitLab set up for
//! `http://127.0.0.1:8929` answers with that whatever it was asked
//! through. So the front can put its own address in place of that one in
//! the server's JSON answers, and the transfers then come through it as
//! well. It can also hold the bytes to a rate, so that a transfer on the
//! loopback interface takes long enough to be stopped half way.
//!
//! One request per connection: the front asks the server to close after
//! its answer and closes towards the client after passing it on. Bodies
//! are passed on as they come, chunked or not, except a JSON answer that
//! is rewritten, which is read whole, unchunked and sent with its new
//! length. `Expect: 100-continue` is answered by the front at once.
//!
//! Sources: RFC 9112 (the lines of a request and an answer, chunked
//! transfer coding, `Connection: close`), RFC 9110 (`Expect`), RFC 5280
//! (an authority, the names of a certificate), RFC 7468 (PEM); the
//! documentation of the `rustls` and `rcgen` crates; git-lfs's
//! `docs/api/batch.md` (the `href` of an action).

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair, KeyUsagePurpose};
use rustls::{
    ServerConfig, ServerConnection, StreamOwned,
    pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
};

/// Standard base64 with padding, for PEM and for Basic credentials.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk.first().copied().unwrap_or(0),
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for (i, shift) in [18, 12, 6, 0].into_iter().enumerate() {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> shift) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A certificate in PEM.
fn pem(der: &[u8]) -> String {
    let body = base64(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for line in body.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(line).unwrap());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

/// An authority made for one test run.
pub(crate) struct Authority {
    certificate: CertificateDer<'static>,
    issuer: Issuer<'static, KeyPair>,
}

impl Authority {
    pub(crate) fn new(name: &str) -> Self {
        let key = KeyPair::generate().unwrap();
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.distinguished_name.push(DnType::CommonName, name);
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params.key_usages = vec![
            KeyUsagePurpose::KeyCertSign,
            KeyUsagePurpose::CrlSign,
            KeyUsagePurpose::DigitalSignature,
        ];
        let certificate = params.self_signed(&key).unwrap().der().clone();
        Self {
            certificate,
            issuer: Issuer::new(params, key),
        }
    }

    /// The authority's certificate in PEM, for `http.sslCAInfo`.
    pub(crate) fn pem(&self) -> String {
        pem(&self.certificate)
    }

    /// A server configuration that presents a certificate this authority
    /// signed for `127.0.0.1` and `localhost`.
    pub(crate) fn server(&self) -> Arc<ServerConfig> {
        let key = KeyPair::generate().unwrap();
        let mut params =
            CertificateParams::new(vec!["127.0.0.1".to_owned(), "localhost".to_owned()]).unwrap();
        params
            .distinguished_name
            .push(DnType::CommonName, "127.0.0.1");
        let leaf = params.signed_by(&key, &self.issuer).unwrap().der().clone();
        let private = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der()));
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(vec![leaf, self.certificate.clone()], private)
            .unwrap();
        Arc::new(config)
    }
}

type Secured = StreamOwned<ServerConnection, TcpStream>;

/// What a front was asked, a line per request.
pub(crate) type Heard = Arc<Mutex<Vec<String>>>;

/// HTTPS (or plain HTTP) on a port of this machine, handed on to a plain
/// HTTP server.
pub(crate) struct Front {
    address: SocketAddr,
    scheme: &'static str,
    stop: Arc<AtomicBool>,
    heard: Heard,
    handshakes_failed: Arc<AtomicUsize>,
}

/// How a front passes requests on.
#[derive(Clone)]
pub(crate) struct Passing {
    /// The server behind it.
    pub upstream: SocketAddr,
    /// The address the server writes for itself in its JSON answers, put
    /// in place by the front's own.
    pub rewrite: Option<String>,
    /// Bytes a second in either direction, when held to a rate.
    pub rate: Option<usize>,
}

impl Front {
    /// A front on a free port: HTTPS with `tls`, plain HTTP without.
    pub(crate) fn start(tls: Option<Arc<ServerConfig>>, passing: Passing) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let heard: Heard = Arc::default();
        let handshakes_failed = Arc::new(AtomicUsize::new(0));
        let scheme = if tls.is_some() { "https" } else { "http" };
        let own = format!("{scheme}://{address}");
        let (ended, noted, failed) = (stop.clone(), heard.clone(), handshakes_failed.clone());
        thread::spawn(move || {
            while !ended.load(Ordering::SeqCst) {
                let Ok((socket, _)) = listener.accept() else {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                };
                let (tls, passing, own, noted, failed) = (
                    tls.clone(),
                    passing.clone(),
                    own.clone(),
                    noted.clone(),
                    failed.clone(),
                );
                thread::spawn(move || {
                    let _ = socket.set_nonblocking(false);
                    let _ = socket.set_read_timeout(Some(Duration::from_secs(60)));
                    let Some(config) = tls else {
                        serve(socket, &passing, &own, &noted, |_| {});
                        return;
                    };
                    let Ok(connection) = ServerConnection::new(config) else {
                        return;
                    };
                    let mut stream = StreamOwned::new(connection, socket);
                    // The handshake, before anything else: one the client
                    // gives up on is counted.
                    while stream.conn.is_handshaking() {
                        if stream.conn.complete_io(&mut stream.sock).is_err() {
                            failed.fetch_add(1, Ordering::SeqCst);
                            return;
                        }
                    }
                    serve(stream, &passing, &own, &noted, |stream: &mut Secured| {
                        stream.conn.send_close_notify();
                        let _ = stream.flush();
                    });
                });
            }
        });
        Self {
            address,
            scheme,
            stop,
            heard,
            handshakes_failed,
        }
    }

    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    /// `https://127.0.0.1:<port>`, or `http://` for a plain one.
    pub(crate) fn origin(&self) -> String {
        format!("{}://{}", self.scheme, self.address)
    }

    /// The request lines it was asked, in order.
    pub(crate) fn heard(&self) -> Vec<String> {
        self.heard.lock().unwrap().clone()
    }

    /// How many clients gave up during the handshake.
    pub(crate) fn handshakes_failed(&self) -> usize {
        self.handshakes_failed.load(Ordering::SeqCst)
    }
}

impl Drop for Front {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

/// A head of a request or an answer: its first line and its fields.
struct Head {
    line: String,
    fields: Vec<(String, String)>,
}

impl Head {
    fn get(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(field, _)| field.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn chunked(&self) -> bool {
        self.get("Transfer-Encoding")
            .is_some_and(|coding| coding.to_ascii_lowercase().contains("chunked"))
    }

    fn length(&self) -> Option<usize> {
        self.get("Content-Length")
            .and_then(|length| length.trim().parse().ok())
    }

    fn write(&self, leave_out: &[&str], add: &[(&str, String)]) -> Vec<u8> {
        let mut out = format!("{}\r\n", self.line);
        for (name, value) in &self.fields {
            if !leave_out.iter().any(|out| name.eq_ignore_ascii_case(out)) {
                out.push_str(&format!("{name}: {value}\r\n"));
            }
        }
        for (name, value) in add {
            out.push_str(&format!("{name}: {value}\r\n"));
        }
        out.push_str("\r\n");
        out.into_bytes()
    }
}

fn read_head(from: &mut impl BufRead) -> Option<Head> {
    let mut line = String::new();
    let mut lines = Vec::new();
    loop {
        line.clear();
        let read = from.read_line(&mut line).ok()?;
        if read == 0 || lines.len() > 200 {
            return None;
        }
        let trimmed = line.trim_end_matches(['\r', '\n']).to_owned();
        if trimmed.is_empty() {
            break;
        }
        lines.push(trimmed);
    }
    let mut lines = lines.into_iter();
    let first = lines.next()?;
    let fields = lines
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.trim().to_owned(), value.trim().to_owned()))
        })
        .collect();
    Some(Head {
        line: first,
        fields,
    })
}

/// Writes at most `rate` bytes a second, when given one.
struct Paced {
    rate: Option<usize>,
    began: Instant,
    sent: usize,
}

impl Paced {
    fn new(rate: Option<usize>) -> Self {
        Self {
            rate,
            began: Instant::now(),
            sent: 0,
        }
    }

    fn write(&mut self, to: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
        let Some(rate) = self.rate else {
            return to.write_all(bytes);
        };
        for piece in bytes.chunks(16 * 1024) {
            to.write_all(piece)?;
            to.flush()?;
            self.sent += piece.len();
            let due = Duration::from_secs_f64(self.sent as f64 / rate as f64);
            if let Some(wait) = due.checked_sub(self.began.elapsed()) {
                thread::sleep(wait);
            }
        }
        Ok(())
    }
}

/// Copy a body of `length` bytes, or what is left until the end.
fn copy_body(
    from: &mut impl Read,
    to: &mut impl Write,
    length: Option<usize>,
    paced: &mut Paced,
) -> std::io::Result<()> {
    let mut left = length.unwrap_or(usize::MAX);
    let mut buffer = vec![0u8; 64 * 1024];
    while left > 0 {
        let want = buffer.len().min(left);
        let read = match from.read(&mut buffer[..want]) {
            Ok(0) => break,
            Ok(read) => read,
            // A TLS client that closes without saying so ends the body
            // as well.
            Err(_) if length.is_none() => break,
            Err(err) => return Err(err),
        };
        paced.write(to, &buffer[..read])?;
        left -= read;
    }
    to.flush()
}

/// Copy a chunked body as it is, chunk by chunk, up to its end.
fn copy_chunked(
    from: &mut impl BufRead,
    to: &mut impl Write,
    paced: &mut Paced,
) -> std::io::Result<()> {
    loop {
        let mut line = String::new();
        if from.read_line(&mut line)? == 0 {
            return Ok(());
        }
        paced.write(to, line.as_bytes())?;
        let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or("0").trim(), 16)
            .unwrap_or(0);
        if size == 0 {
            // Trailers, up to an empty line.
            loop {
                let mut trailer = String::new();
                if from.read_line(&mut trailer)? == 0 {
                    return Ok(());
                }
                paced.write(to, trailer.as_bytes())?;
                if trailer.trim().is_empty() {
                    return to.flush();
                }
            }
        }
        let mut chunk = vec![0u8; size + 2];
        from.read_exact(&mut chunk)?;
        paced.write(to, &chunk)?;
    }
}

/// A chunked body, read whole and unchunked.
fn read_chunked(from: &mut impl BufRead) -> std::io::Result<Vec<u8>> {
    let mut sink = Vec::new();
    copy_chunked(from, &mut sink, &mut Paced::new(None))?;
    let mut out = Vec::new();
    let mut rest = sink.as_slice();
    while let Some(end) = rest.windows(2).position(|pair| pair == b"\r\n") {
        let size = usize::from_str_radix(
            String::from_utf8_lossy(&rest[..end])
                .split(';')
                .next()
                .unwrap_or("0")
                .trim(),
            16,
        )
        .unwrap_or(0);
        rest = &rest[end + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = rest.get(size + 2..).unwrap_or(&[]);
    }
    Ok(out)
}

/// The fields a front does not pass on as they are.
const HOP: [&str; 6] = [
    "Host",
    "Connection",
    "Keep-Alive",
    "Expect",
    "Accept-Encoding",
    "Proxy-Connection",
];

fn serve<S: Read + Write>(
    stream: S,
    passing: &Passing,
    own: &str,
    heard: &Heard,
    finish: impl FnOnce(&mut S),
) {
    let mut client = BufReader::new(stream);
    let Some(request) = read_head(&mut client) else {
        return;
    };
    heard.lock().unwrap().push(request.line.clone());
    if request
        .get("Expect")
        .is_some_and(|expect| expect.eq_ignore_ascii_case("100-continue"))
    {
        let _ = client.get_mut().write_all(b"HTTP/1.1 100 Continue\r\n\r\n");
        let _ = client.get_mut().flush();
    }
    let Ok(upstream) = TcpStream::connect(passing.upstream) else {
        let _ = client.get_mut().write_all(
            b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        return;
    };
    let _ = upstream.set_read_timeout(Some(Duration::from_secs(120)));
    let mut server = BufReader::new(upstream);
    let head = request.write(
        &HOP,
        &[
            ("Host", passing.upstream.to_string()),
            ("Connection", "close".to_owned()),
            ("X-Forwarded-Proto", "https".to_owned()),
        ],
    );
    if server.get_mut().write_all(&head).is_err() {
        return;
    }
    let mut paced = Paced::new(passing.rate);
    let sent = if request.chunked() {
        copy_chunked(&mut client, server.get_mut(), &mut paced)
    } else {
        copy_body(
            &mut client,
            server.get_mut(),
            Some(request.length().unwrap_or(0)),
            &mut paced,
        )
    };
    if sent.is_err() {
        return;
    }
    let Some(answer) = read_head(&mut server) else {
        return;
    };
    let json = answer
        .get("Content-Type")
        .is_some_and(|kind| kind.contains("json"));
    let out = client.get_mut();
    let mut paced = Paced::new(passing.rate);
    match &passing.rewrite {
        Some(from) if json => {
            let body = if answer.chunked() {
                read_chunked(&mut server).unwrap_or_default()
            } else {
                let mut body = Vec::new();
                let _ = copy_body(
                    &mut server,
                    &mut body,
                    answer.length(),
                    &mut Paced::new(None),
                );
                body
            };
            let body = String::from_utf8_lossy(&body).replace(from.as_str(), own);
            let head = answer.write(
                &["Content-Length", "Transfer-Encoding", "Connection"],
                &[
                    ("Content-Length", body.len().to_string()),
                    ("Connection", "close".to_owned()),
                ],
            );
            let _ = out.write_all(&head);
            let _ = paced.write(out, body.as_bytes());
        }
        _ => {
            let head = answer.write(&["Connection"], &[("Connection", "close".to_owned())]);
            if out.write_all(&head).is_err() {
                return;
            }
            let _ = if answer.chunked() {
                copy_chunked(&mut server, out, &mut paced)
            } else {
                copy_body(&mut server, out, answer.length(), &mut paced)
            };
        }
    }
    let _ = out.flush();
    finish(client.get_mut());
}
