//! A server for tests that answers each request as the test says, plain
//! or over TLS, on a port of the loopback interface: a stand-in for a
//! provider's API or a picture service, reached directly or through a
//! [`crate::proxy::Proxy`] by a name. It writes down every request.
//!
//! One request to a connection: every answer says `Connection: close`.
//!
//! Sources: RFC 9112 (the message head, `Content-Length`, the chunked
//! transfer coding, `Expect: 100-continue`); the documentation of
//! `rustls` (`ServerConnection`, `StreamOwned`).

use std::{
    io::{self, BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use rustls::{ServerConfig, ServerConnection, StreamOwned};

/// One request, read whole.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub method: String,
    /// Its target, as sent.
    pub target: String,
    /// Its header lines, names in lower case, in order.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Asked {
    /// The value of a header line, by its name in lower case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.as_str())
    }
}

/// What a request is answered with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Answer {
    /// The status code.
    pub status: u16,
    /// Header lines besides `Content-Length` and `Connection`.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// Longest head read.
const MAX_HEAD: usize = 64 * 1024;
/// Largest body read.
const MAX_BODY: usize = 64 * 1024 * 1024;

/// Read one request off `reader`: `None` when the other side said
/// nothing, or went away before its head was whole. An `Expect:
/// 100-continue` is answered as it is read.
pub fn read_request<S: Read + Write>(reader: &mut BufReader<S>) -> io::Result<Option<Asked>> {
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut words = line.split_whitespace();
    let (Some(method), Some(target)) = (words.next(), words.next()) else {
        return Ok(None);
    };
    let (method, target) = (method.to_owned(), target.to_owned());
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut read = line.len();
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            return Ok(None);
        }
        read = read.saturating_add(line.len());
        if read > MAX_HEAD {
            return Err(io::Error::other("the head is too long"));
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.clone())
    };
    if header("expect").is_some_and(|value| value.eq_ignore_ascii_case("100-continue")) {
        reader
            .get_mut()
            .write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
        reader.get_mut().flush()?;
    }
    let mut body = Vec::new();
    if header("transfer-encoding").is_some_and(|value| value.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size)?;
            let size = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(io::Error::other)?;
            if body.len().saturating_add(size) > MAX_BODY {
                return Err(io::Error::other("the body is too large"));
            }
            let mut chunk = vec![0u8; size];
            reader.read_exact(&mut chunk)?;
            body.extend_from_slice(&chunk);
            let mut end = String::new();
            reader.read_line(&mut end)?;
            if size == 0 {
                break;
            }
        }
    } else if let Some(length) = header("content-length").and_then(|value| value.parse().ok()) {
        if length > MAX_BODY {
            return Err(io::Error::other("the body is too large"));
        }
        body = vec![0u8; length];
        reader.read_exact(&mut body)?;
    }
    Ok(Some(Asked {
        method,
        target,
        headers,
        body,
    }))
}

/// The reason phrase a status is sent with.
fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        422 => "Unprocessable Content",
        _ => "Answer",
    }
}

/// Write `answer` whole.
pub fn write_answer<S: Write>(stream: &mut S, answer: &Answer) -> io::Result<()> {
    let mut head = format!("HTTP/1.1 {} {}\r\n", answer.status, reason(answer.status));
    for (name, value) in &answer.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str(&format!(
        "Content-Length: {}\r\nConnection: close\r\n\r\n",
        answer.body.len()
    ));
    stream.write_all(head.as_bytes())?;
    stream.write_all(&answer.body)?;
    stream.flush()
}

type Answering = dyn Fn(&Asked) -> Answer + Send + Sync;

/// A running stand-in. It stops when dropped.
pub struct StandIn {
    port: u16,
    stop: Arc<AtomicBool>,
    asked: Arc<Mutex<Vec<Asked>>>,
    thread: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for StandIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StandIn")
            .field("port", &self.port)
            .finish_non_exhaustive()
    }
}

impl StandIn {
    /// Answer every request with what `answer` makes of it; over TLS with
    /// `tls`, else in plain HTTP.
    pub fn start(
        tls: Option<Arc<ServerConfig>>,
        answer: impl Fn(&Asked) -> Answer + Send + Sync + 'static,
    ) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let answer: Arc<Answering> = Arc::new(answer);
        let thread = {
            let (stop, asked) = (stop.clone(), asked.clone());
            std::thread::Builder::new()
                .name("test-stand-in".to_owned())
                .spawn(move || {
                    for stream in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let Ok(stream) = stream else { continue };
                        let (tls, asked, answer) = (tls.clone(), asked.clone(), answer.clone());
                        let _ = std::thread::Builder::new()
                            .name("test-stand-in-request".to_owned())
                            .spawn(move || {
                                let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                                let _ = match tls {
                                    Some(config) => {
                                        let Ok(connection) = ServerConnection::new(config) else {
                                            return;
                                        };
                                        let mut secured = StreamOwned::new(connection, stream);
                                        let done = serve(&mut secured, &asked, answer.as_ref());
                                        secured.conn.send_close_notify();
                                        let _ = secured.flush();
                                        done
                                    }
                                    None => {
                                        let mut stream = stream;
                                        serve(&mut stream, &asked, answer.as_ref())
                                    }
                                };
                            });
                    }
                })?
        };
        Ok(Self {
            port,
            stop,
            asked,
            thread: Some(thread),
        })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Every request so far, in order.
    pub fn asked(&self) -> Vec<Asked> {
        match self.asked.lock() {
            Ok(asked) => asked.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Drop for StandIn {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the thread out of its wait for a connection.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn serve<S: Read + Write>(
    stream: &mut S,
    asked: &Mutex<Vec<Asked>>,
    answer: &Answering,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let Some(request) = read_request(&mut reader)? else {
        return Ok(());
    };
    let answered = answer(&request);
    match asked.lock() {
        Ok(mut asked) => asked.push(request),
        Err(poisoned) => poisoned.into_inner().push(request),
    }
    write_answer(reader.into_inner(), &answered)
}
