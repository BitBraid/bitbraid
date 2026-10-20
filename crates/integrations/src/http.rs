//! Fetching bytes over HTTPS.
//!
//! [`Fetch`] is what the rest of the application asks for: one request
//! ([`Call`]: a method, an address, header lines and perhaps a body; or
//! the plain `GET` of a [`Request`]), answered with a status, the header
//! lines that matter and a body, or not at all. [`Client`] is the one
//! real implementation: HTTP/1.1 over TLS with the root certificates of the
//! system, one request to a connection. It sends the address asked for, a
//! `Host`, a fixed `User-Agent`, the header lines the caller gave, those
//! that frame the message (`Accept-Encoding`, `Connection`,
//! `Content-Length`), and nothing else: no cookies are kept or sent, no
//! referrer, nothing that names the user, the machine or a repository. Of
//! the answer's header lines it keeps those in [`KEPT_HEADERS`] and drops
//! the rest (a cookie among them). A request's `Debug` text never shows the
//! value of an `Authorization` or `PRIVATE-TOKEN` line, nor a body. It does
//! not follow redirects (the status and the `Location` are handed to the
//! caller, who knows where an answer may come from). An answer is read up
//! to a size the caller sets and for as long as the caller allows, in
//! slices short enough that a request that is no longer wanted ends at
//! once; a large one (a package of the updater) is written out as it comes
//! rather than kept ([`Fetch::send_to`]). The one wait that is not cut
//! short is the making of the connection itself, which the standard
//! library does in one piece: it lasts its share of the time allowed for
//! connecting at most, and a request that is no longer wanted by then ends
//! there, before anything was sent.
//!
//! Plain HTTP is spoken only to this machine (a loopback address): that is
//! what a test server or a mirror of one's own is, and nothing else is
//! ever asked without TLS.
//!
//! An HTTPS request goes through the proxy the user's surroundings name
//! (`HTTPS_PROXY` and the like, else git's `http.proxy`), in a tunnel the
//! proxy is asked for, with TLS to the host inside it; this machine is
//! never asked through one ([`proxy`] says which and how).
//!
//! Not here: HTTP/2, connections kept open, compressed answers, client
//! certificates, bodies sent in chunks.
//!
//! Sources: RFC 9110 (HTTP semantics: methods, status codes, `Location`,
//! `ETag`, `Retry-After`); RFC 8288 (`Link`); docs.github.com "Rate limits
//! for the REST API" (`X-RateLimit-*`) and "Scopes for OAuth apps"
//! (`X-OAuth-Scopes`); docs.gitlab.com "Rate limits" (`RateLimit-*`) and
//! "REST API" (pagination headers `X-Next-Page`, `X-Total`); RFC 9112
//! (HTTP/1.1: the message head, how long a body is, the chunked transfer
//! coding); RFC 3986 (the parts of an address); the documentation of
//! `rustls` (`ClientConfig`, `ClientConnection`, `StreamOwned`,
//! `crypto::ring`), of `rustls-native-certs` (`load_native_certs`) and of
//! `httparse` (`Response::parse`); the standard library's `TcpStream`.

use std::{
    fmt,
    io::{self, Read as _, Write as _},
    net::{IpAddr, SocketAddr, TcpStream, ToSocketAddrs as _},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned, pki_types};

pub mod proxy;
mod wire;

pub use proxy::{NoProxy, Proxies, ProxyAddress, Route};

/// Why a request was not answered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// The address is not one this client asks.
    #[error("{}", say!("http-address", what = *.0))]
    Address(&'static str),
    /// Plain HTTP to another machine.
    #[error("{}", say!("http-insecure"))]
    Insecure,
    /// The host's name stands for no address.
    #[error("{}", say!("http-resolve", name = .0.as_str()))]
    Resolve(String),
    /// No connection was made.
    #[error("{}", say!("http-connect", why = .0.as_str()))]
    Connect(String),
    /// The connection could not be secured, or the certificate is not one
    /// the system trusts for this host.
    #[error("{}", say!("http-tls", why = .0.as_str()))]
    Tls(String),
    /// The time allowed ran out.
    #[error("{}", say!("http-timeout"))]
    Timeout,
    /// The answer is larger than the caller takes.
    #[error("{}", say!("http-too-large"))]
    TooLarge,
    /// What came back is not an HTTP answer this client reads.
    #[error("{}", say!("http-malformed", what = *.0))]
    Malformed(&'static str),
    /// The caller no longer wants it.
    #[error("{}", say!("http-cancelled"))]
    Cancelled,
    /// The connection broke part way.
    #[error("{}", say!("http-broken", why = .0.as_str()))]
    Broken(String),
    /// The proxy that was named would not let the request through, or
    /// cannot be used: wants a login, refused one, refused the tunnel.
    #[error("{0}")]
    Proxy(String),
    /// A stand-in for the network that answers from a recording has no
    /// answer for this request ([`crate::replay`]).
    #[error("{}", say!("http-unrecorded", request = .0.as_str()))]
    Unrecorded(String),
    /// What came could not be written where the caller wanted it
    /// ([`Fetch::send_to`]): a full disk, a file that went away.
    #[error("{}", say!("http-not-stored", error = .0.as_str()))]
    Stored(String),
}

/// An address: `https://host[:port]/path?query`, or `http://` likewise.
/// No user name, no fragment; the path is sent as it is written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    secure: bool,
    host: String,
    port: u16,
    path: String,
}

/// Longest address read.
const MAX_URL: usize = 2048;

fn without_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let start = text.get(..prefix.len())?;
    start
        .eq_ignore_ascii_case(prefix)
        .then(|| text.get(prefix.len()..))
        .flatten()
}

impl Url {
    /// Read an address. Anything that could put a line break, a space or a
    /// byte outside printable ASCII into a request is refused, so what is
    /// sent is the address and nothing smuggled in with it.
    pub fn parse(text: &str) -> Result<Self, FetchError> {
        let text = text.trim();
        if text.len() > MAX_URL {
            return Err(FetchError::Address(say!("http-address-too-long")));
        }
        let (secure, rest) = if let Some(rest) = without_prefix(text, "https://") {
            (true, rest)
        } else if let Some(rest) = without_prefix(text, "http://") {
            (false, rest)
        } else {
            return Err(FetchError::Address(say!("http-address-scheme")));
        };
        // A fragment is for the reader of the answer and is never sent.
        let rest = rest.split('#').next().unwrap_or_default();
        let (authority, path) = match rest.find(['/', '?']) {
            Some(at) => rest.split_at_checked(at).unwrap_or((rest, "")),
            None => (rest, ""),
        };
        if authority.contains('@') {
            return Err(FetchError::Address(say!("http-address-user-name")));
        }
        let (host, port) = if let Some(inner) = authority.strip_prefix('[') {
            let (host, after) = inner
                .split_once(']')
                .ok_or(FetchError::Address(say!("http-address-unclosed-bracket")))?;
            let port = match after.strip_prefix(':') {
                Some(port) => Some(port),
                None if after.is_empty() => None,
                None => return Err(FetchError::Address(say!("http-address-port"))),
            };
            if host.parse::<std::net::Ipv6Addr>().is_err() {
                return Err(FetchError::Address(say!("http-address-host")));
            }
            (host, port)
        } else {
            match authority.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            }
        };
        let named = !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
            && !host.starts_with(['.', '-'])
            && !host.contains("..");
        if !named && host.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(FetchError::Address(say!("http-address-host")));
        }
        let port = match port {
            None if secure => 443,
            None => 80,
            Some(port) => port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or(FetchError::Address(say!("http-address-port")))?,
        };
        if !path.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
            return Err(FetchError::Address(say!("http-address-path")));
        }
        let path = match path.chars().next() {
            Some('/') => path.to_owned(),
            _ => format!("/{path}"),
        };
        Ok(Self {
            secure,
            host: host.to_ascii_lowercase(),
            port,
            path,
        })
    }

    /// Whether it is asked over TLS.
    pub fn secure(&self) -> bool {
        self.secure
    }

    /// The host, in lower case; an IPv6 address without its brackets.
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// The path with its query, as it is sent.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// The host as the `Host` line names it: with the port when it is not
    /// the usual one, an IPv6 address in brackets.
    pub fn authority(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        let usual = if self.secure { 443 } else { 80 };
        if self.port == usual {
            host
        } else {
            format!("{host}:{}", self.port)
        }
    }

    /// Where a `Location` leads from here: a whole address, one that
    /// keeps the scheme (`//host/path`), a path on the same host, or a
    /// path beside this one.
    pub fn join(&self, location: &str) -> Result<Self, FetchError> {
        let location = location.trim();
        let scheme = if self.secure { "https" } else { "http" };
        if location.contains("://") {
            Self::parse(location)
        } else if location.starts_with("//") {
            Self::parse(&format!("{scheme}:{location}"))
        } else if location.starts_with('/') {
            Self::parse(&format!("{scheme}://{}{location}", self.authority()))
        } else {
            let folder = self.path.split('?').next().unwrap_or_default();
            let folder = folder.rsplit_once('/').map_or("", |(folder, _)| folder);
            Self::parse(&format!(
                "{scheme}://{}{folder}/{location}",
                self.authority()
            ))
        }
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let scheme = if self.secure { "https" } else { "http" };
        write!(f, "{scheme}://{}{}", self.authority(), self.path)
    }
}

/// How much of the caller's time and memory one request may take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Longest the name is looked up and the connection made.
    pub connect: Duration,
    /// Longest the whole request takes, from the first step to the last
    /// byte, however the answer trickles in.
    pub total: Duration,
    /// Largest body taken, in bytes.
    pub body: usize,
}

/// One `GET`.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// What is asked.
    pub url: &'a Url,
    /// The `Accept` line: the kinds of answer that are welcome.
    pub accept: &'a str,
    /// What it may cost.
    pub limits: Limits,
}

/// What a request does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Method {
    /// Reads.
    Get,
    /// Makes something, or asks something that is not a read.
    Post,
    /// Puts something in place of what is there.
    Put,
    /// Changes part of something.
    Patch,
    /// Removes something.
    Delete,
}

impl Method {
    /// The method as the request line names it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
        }
    }
}

/// Header lines whose value is a secret: a request's `Debug` text writes
/// `<hidden>` for them.
const SECRET_HEADERS: &[&str] = &["authorization", "private-token"];

/// Header lines the client writes itself, or never: a caller who gives
/// one is refused.
const OWN_HEADERS: &[&str] = &[
    "host",
    "user-agent",
    "content-length",
    "transfer-encoding",
    "connection",
    "accept-encoding",
    "cookie",
    "te",
    "upgrade",
    "expect",
    "keep-alive",
    "proxy-authorization",
    "proxy-connection",
];

/// Most header lines a caller gives.
const MAX_HEADERS: usize = 32;
/// Longest value of a header line a caller gives.
const MAX_HEADER_VALUE: usize = 8 * 1024;
/// Largest body sent.
pub const MAX_SENT: usize = 16 * 1024 * 1024;

/// The header lines of an answer that are kept, in lower case: what tells
/// a reader where the next page is, how much more may be asked, what a
/// token was granted, and whether what it has is still current. Every
/// other line is dropped as it is read.
pub const KEPT_HEADERS: &[&str] = &[
    "content-type",
    "date",
    "etag",
    "last-modified",
    "link",
    "location",
    "ratelimit-limit",
    "ratelimit-remaining",
    "ratelimit-reset",
    "retry-after",
    "www-authenticate",
    "x-accepted-oauth-scopes",
    "x-next-page",
    "x-oauth-scopes",
    "x-page",
    "x-per-page",
    "x-ratelimit-limit",
    "x-ratelimit-remaining",
    "x-ratelimit-reset",
    "x-ratelimit-resource",
    "x-ratelimit-used",
    "x-total",
    "x-total-pages",
];

/// One request with everything it may carry: a method, header lines and
/// perhaps a body.
#[derive(Clone, PartialEq, Eq)]
pub struct Call {
    /// What it does.
    pub method: Method,
    /// What it is asked of.
    pub url: Url,
    /// Its header lines, in the order they are sent.
    pub headers: Vec<(String, String)>,
    /// What is sent with it.
    pub body: Option<Vec<u8>>,
    /// What it may cost.
    pub limits: Limits,
}

impl Call {
    /// A request without header lines or body.
    pub fn new(method: Method, url: Url, limits: Limits) -> Self {
        Self {
            method,
            url,
            headers: Vec::new(),
            body: None,
            limits,
        }
    }

    /// The same with a header line added.
    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    /// The same with a body of this kind.
    pub fn with_body(mut self, content_type: &str, body: Vec<u8>) -> Self {
        self.headers
            .push(("Content-Type".to_owned(), content_type.to_owned()));
        self.body = Some(body);
        self
    }

    /// The value of a header line, by its name in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The `GET` a [`Request`] stands for.
    pub fn from_request(request: &Request<'_>) -> Self {
        Self::new(Method::Get, request.url.clone(), request.limits)
            .with_header("Accept", request.accept)
    }
}

impl fmt::Debug for Call {
    /// Everything but the secrets: the value of a header line that signs
    /// in is `<hidden>`, and of the body only its length is written.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| {
                let secret = SECRET_HEADERS
                    .iter()
                    .any(|secret| name.trim().eq_ignore_ascii_case(secret));
                (
                    name.as_str(),
                    if secret { "<hidden>" } else { value.as_str() },
                )
            })
            .collect();
        f.debug_struct("Call")
            .field("method", &self.method)
            .field("url", &self.url.to_string())
            .field("headers", &headers)
            .field(
                "body",
                &self
                    .body
                    .as_ref()
                    .map(|body| format!("<{} bytes>", body.len())),
            )
            .field("limits", &self.limits)
            .finish()
    }
}

/// An answer: read whole, within the limits.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// Where the answer says to look instead, when it says so.
    pub location: Option<String>,
    /// What the body is said to be.
    pub content_type: Option<String>,
    /// The header lines of [`KEPT_HEADERS`] it had, names in lower case,
    /// a line given twice joined with `, `.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Response {
    /// The value of a kept header line, by its name in any case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

impl fmt::Debug for Response {
    /// Without the body, which may hold a token: only its length.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Response")
            .field("status", &self.status)
            .field("location", &self.location)
            .field("content_type", &self.content_type)
            .field("headers", &self.headers)
            .field("body", &format_args!("<{} bytes>", self.body.len()))
            .finish()
    }
}

/// Something that fetches bytes: the real client, or a stand-in in a test.
pub trait Fetch: Send + Sync + fmt::Debug {
    /// Send one request and read the answer whole. `cancel`, once set,
    /// ends the request at its next step.
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError>;

    /// Ask once with a `GET` and read the answer whole.
    fn get(&self, request: &Request<'_>, cancel: &AtomicBool) -> Result<Response, FetchError> {
        self.send(&Call::from_request(request), cancel)
    }

    /// Send one request; when the answer is what was asked for (a status
    /// from 200 to 299) its body is written to `into` as it comes, up to
    /// the call's limit, and the answer is handed back without it. Another
    /// answer (a redirect, an error) keeps its body, up to
    /// [`KEPT_OTHERWISE`] bytes, and nothing is written. The real client
    /// never holds such a body in memory; a stand-in may read it whole
    /// first.
    fn send_to(
        &self,
        call: &Call,
        cancel: &AtomicBool,
        into: &mut dyn io::Write,
    ) -> Result<Response, FetchError> {
        let mut response = self.send(call, cancel)?;
        if (200..300).contains(&response.status) {
            into.write_all(&response.body)
                .map_err(|err| FetchError::Stored(err.to_string()))?;
            response.body = Vec::new();
        } else if response.body.len() > KEPT_OTHERWISE {
            return Err(FetchError::TooLarge);
        }
        Ok(response)
    }
}

/// Most bytes of the body of an answer that was not what was asked for
/// that [`Fetch::send_to`] keeps.
pub const KEPT_OTHERWISE: usize = wire::KEPT_OTHERWISE;

/// The longest any one wait on the network lasts before the clock and the
/// caller's wish to stop are looked at again.
const SLICE: Duration = Duration::from_millis(100);

/// Addresses of one name that are tried.
const MAX_ADDRESSES: usize = 4;

/// The time a request has left, and whether it is still wanted.
#[derive(Debug)]
pub(crate) struct Clock<'a> {
    end: Instant,
    cancel: &'a AtomicBool,
}

impl<'a> Clock<'a> {
    fn new(allowed: Duration, cancel: &'a AtomicBool) -> Self {
        let now = Instant::now();
        Self {
            end: now.checked_add(allowed).unwrap_or(now),
            cancel,
        }
    }

    /// The time left, or why there is none.
    fn left(&self) -> Result<Duration, FetchError> {
        if self.cancel.load(Ordering::Relaxed) {
            return Err(FetchError::Cancelled);
        }
        let left = self.end.saturating_duration_since(Instant::now());
        if left.is_zero() {
            Err(FetchError::Timeout)
        } else {
            Ok(left)
        }
    }

    /// How long the next wait may last.
    fn slice(&self) -> Result<Duration, FetchError> {
        // A timeout of nothing is refused by the socket.
        Ok(self.left()?.clamp(Duration::from_millis(1), SLICE))
    }
}

/// A wait that ended because its slice did, not because something broke.
fn only_waiting(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

fn broken(err: io::Error) -> FetchError {
    FetchError::Broken(err.to_string())
}

/// A connection, secured or not.
enum Stream {
    Plain(TcpStream),
    Tls(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Stream {
    fn socket(&self) -> &TcpStream {
        match self {
            Stream::Plain(socket) => socket,
            Stream::Tls(tls) => &tls.sock,
        }
    }

    fn wait_at_most(&self, slice: Duration) -> Result<(), FetchError> {
        let socket = self.socket();
        socket.set_read_timeout(Some(slice)).map_err(broken)?;
        socket.set_write_timeout(Some(slice)).map_err(broken)
    }

    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(socket) => socket.read(buffer),
            Stream::Tls(tls) => tls.read(buffer),
        }
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Plain(socket) => socket.write(bytes),
            Stream::Tls(tls) => tls.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Plain(socket) => socket.flush(),
            Stream::Tls(tls) => tls.flush(),
        }
    }
}

/// A connection read against the clock.
struct Timed<'a> {
    stream: &'a mut Stream,
    clock: &'a Clock<'a>,
}

impl wire::Source for Timed<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> Result<usize, FetchError> {
        loop {
            self.stream.wait_at_most(self.clock.slice()?)?;
            match self.stream.read(buffer) {
                Ok(read) => return Ok(read),
                Err(err) if only_waiting(&err) => {}
                Err(err) => return Err(broken(err)),
            }
        }
    }
}

/// Where the root certificates come from.
#[derive(Debug)]
enum Roots {
    /// The system's store.
    System,
    /// These, in DER, and no others.
    Given(Vec<Vec<u8>>),
}

/// Where the proxies a client goes through come from.
#[derive(Debug)]
enum ProxySource {
    /// The user's surroundings, read when the first request that could
    /// go through a proxy needs them: looking at git's configuration
    /// starts a program, and is no work for the thread that creates the
    /// client.
    Surroundings(OnceLock<Proxies>),
    /// These.
    Given(Proxies),
}

/// The client: HTTP/1.1, over TLS to anywhere and plainly to this machine.
#[derive(Debug)]
pub struct Client {
    agent: String,
    roots: Roots,
    /// Made when the first secure request needs it: reading the system's
    /// certificates takes a while and is no work for the thread that
    /// creates the client.
    tls: OnceLock<Result<Arc<ClientConfig>, String>>,
    proxies: ProxySource,
}

/// What a header's value may hold.
fn header_value(text: &str) -> bool {
    !text.is_empty() && text.len() <= 256 && text.bytes().all(|b| (0x20..=0x7e).contains(&b))
}

impl Client {
    /// A client that trusts what the system trusts and goes through the
    /// proxy the user's surroundings name ([`Proxies::from_surroundings`]).
    /// `agent` is what it calls itself in every request.
    pub fn new(agent: &str) -> Self {
        Self::build(agent, Roots::System)
    }

    /// A client that trusts these certificates (DER) and no others: for a
    /// server of one's own, as in the tests. It goes through the proxy
    /// the surroundings name, as [`Client::new`]'s does.
    pub fn trusting(agent: &str, certificates: Vec<Vec<u8>>) -> Self {
        Self::build(agent, Roots::Given(certificates))
    }

    /// The same client going through these proxies, whatever the
    /// surroundings name.
    pub fn with_proxies(mut self, proxies: Proxies) -> Self {
        self.proxies = ProxySource::Given(proxies);
        self
    }

    fn build(agent: &str, roots: Roots) -> Self {
        let agent = if header_value(agent) {
            agent.to_owned()
        } else {
            "BitBraid".to_owned()
        };
        Self {
            agent,
            roots,
            tls: OnceLock::new(),
            proxies: ProxySource::Surroundings(OnceLock::new()),
        }
    }

    /// The proxies the client goes through.
    pub fn proxies(&self) -> &Proxies {
        match &self.proxies {
            ProxySource::Surroundings(read) => read.get_or_init(Proxies::from_surroundings),
            ProxySource::Given(proxies) => proxies,
        }
    }

    fn tls(&self) -> Result<Arc<ClientConfig>, FetchError> {
        self.tls
            .get_or_init(|| {
                let mut store = RootCertStore::empty();
                match &self.roots {
                    Roots::System => {
                        // A certificate of the store that cannot be read is
                        // left out; the others still count.
                        for certificate in rustls_native_certs::load_native_certs().certs {
                            let _ = store.add(certificate);
                        }
                    }
                    Roots::Given(certificates) => {
                        for der in certificates {
                            store
                                .add(pki_types::CertificateDer::from(der.clone()))
                                .map_err(|err| err.to_string())?;
                        }
                    }
                }
                if store.is_empty() {
                    return Err(say!("http-no-roots").to_owned());
                }
                let provider = Arc::new(rustls::crypto::ring::default_provider());
                let config = ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .map_err(|err| err.to_string())?
                    .with_root_certificates(store)
                    .with_no_client_auth();
                Ok(Arc::new(config))
            })
            .clone()
            .map_err(FetchError::Tls)
    }

    /// The addresses a host stands for. A name is looked up on a thread of
    /// its own, because the system's lookup cannot be given a time limit.
    fn addresses(
        host: &str,
        port: u16,
        clock: &Clock<'_>,
        until: Instant,
    ) -> Result<Vec<SocketAddr>, FetchError> {
        if let Ok(ip) = host.parse::<IpAddr>() {
            return Ok(vec![SocketAddr::new(ip, port)]);
        }
        let (answer, found) = mpsc::channel();
        let name = host;
        let host = host.to_owned();
        std::thread::Builder::new()
            .name("bitbraid-resolve".to_owned())
            .spawn(move || {
                let addresses = (host.as_str(), port)
                    .to_socket_addrs()
                    .map(|addresses| addresses.collect::<Vec<_>>())
                    .map_err(|err| err.to_string());
                // Nobody waits any more when the time ran out.
                let _ = answer.send(addresses);
            })
            .map_err(|err| FetchError::Resolve(err.to_string()))?;
        loop {
            let slice = clock.slice()?;
            if Instant::now() >= until {
                return Err(FetchError::Timeout);
            }
            match found.recv_timeout(slice) {
                Ok(Ok(addresses)) if addresses.is_empty() => {
                    return Err(FetchError::Resolve(name.to_owned()));
                }
                Ok(Ok(addresses)) => return Ok(addresses),
                Ok(Err(err)) => return Err(FetchError::Resolve(err)),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(FetchError::Resolve(name.to_owned()));
                }
            }
        }
    }

    /// A connection to `host`, on this machine only when `here_only`.
    fn open(
        host: &str,
        port: u16,
        here_only: bool,
        limits: &Limits,
        clock: &Clock<'_>,
    ) -> Result<TcpStream, FetchError> {
        let now = Instant::now();
        let until = now.checked_add(limits.connect).unwrap_or(now);
        let addresses = Self::addresses(host, port, clock, until)?;
        // Without TLS only this machine is spoken to, whatever the name
        // was: it is the address that says where the bytes go.
        let allowed: Vec<SocketAddr> = addresses
            .into_iter()
            .filter(|address| !here_only || address.ip().is_loopback())
            .take(MAX_ADDRESSES)
            .collect();
        if allowed.is_empty() {
            return Err(FetchError::Insecure);
        }
        let mut failure = FetchError::Timeout;
        let mut socket = None;
        for (tried, address) in allowed.iter().enumerate() {
            let left = clock
                .left()?
                .min(until.saturating_duration_since(Instant::now()));
            if left.is_zero() {
                return Err(FetchError::Timeout);
            }
            // Each address gets its share, so that one that never answers
            // does not use up the time of the next.
            let shares = u32::try_from(allowed.len().saturating_sub(tried)).unwrap_or(1);
            let share = (left / shares.max(1)).max(Duration::from_millis(1));
            match TcpStream::connect_timeout(address, share) {
                Ok(connected) => {
                    socket = Some(connected);
                    break;
                }
                Err(err) if err.kind() == io::ErrorKind::TimedOut => failure = FetchError::Timeout,
                Err(err) => failure = FetchError::Connect(err.to_string()),
            }
        }
        let Some(socket) = socket else {
            return Err(failure);
        };
        let _ = socket.set_nodelay(true);
        clock.left()?;
        Ok(socket)
    }

    fn connect(&self, url: &Url, limits: &Limits, clock: &Clock<'_>) -> Result<Stream, FetchError> {
        // This machine, and plain HTTP (spoken to it alone), are never
        // asked through a proxy: nothing is read for them.
        let route = if proxy::stays_here(url) {
            Route::Direct
        } else {
            self.proxies().route(url)
        };
        let socket = match route {
            Route::Direct => Self::open(url.host(), url.port(), !url.secure(), limits, clock)?,
            Route::Through(proxy) => {
                let mut socket = Self::open(proxy.host(), proxy.port(), false, limits, clock)
                    .map_err(|err| match err {
                        FetchError::Resolve(_) => {
                            FetchError::Resolve(say!("http-proxy-not-found", host = proxy.host()))
                        }
                        FetchError::Connect(why) => FetchError::Connect(say!(
                            "http-proxy-connect",
                            place = proxy.place(),
                            why
                        )),
                        other => other,
                    })?;
                proxy::open_tunnel(&mut socket, proxy, url, &self.agent, clock)?;
                socket
            }
            Route::Unusable(from, why) => {
                return Err(FetchError::Proxy(say!("http-proxy-unusable", from, why)));
            }
        };
        if !url.secure() {
            return Ok(Stream::Plain(socket));
        }
        let config = self.tls()?;
        let name = pki_types::ServerName::try_from(url.host().to_owned())
            .map_err(|_| FetchError::Address(say!("http-address-host")))?;
        let connection =
            ClientConnection::new(config, name).map_err(|err| FetchError::Tls(err.to_string()))?;
        let mut tls = StreamOwned::new(connection, socket);
        // The handshake, against the clock like everything else.
        while tls.conn.is_handshaking() {
            let slice = clock.slice()?;
            tls.sock.set_read_timeout(Some(slice)).map_err(broken)?;
            tls.sock.set_write_timeout(Some(slice)).map_err(broken)?;
            match tls.conn.complete_io(&mut tls.sock) {
                Ok(_) => {}
                Err(err) if only_waiting(&err) => {}
                Err(err) => return Err(FetchError::Tls(err.to_string())),
            }
        }
        Ok(Stream::Tls(Box::new(tls)))
    }

    fn write_whole(stream: &mut Stream, bytes: &[u8], clock: &Clock<'_>) -> Result<(), FetchError> {
        let mut rest = bytes;
        while !rest.is_empty() {
            stream.wait_at_most(clock.slice()?)?;
            match stream.write(rest) {
                Ok(0) => {
                    return Err(FetchError::Broken(
                        say!("http-nothing-more-taken").to_owned(),
                    ));
                }
                Ok(sent) => rest = rest.get(sent..).unwrap_or_default(),
                Err(err) if only_waiting(&err) => {}
                Err(err) => return Err(broken(err)),
            }
        }
        loop {
            stream.wait_at_most(clock.slice()?)?;
            match stream.flush() {
                Ok(()) => return Ok(()),
                Err(err) if only_waiting(&err) => {}
                Err(err) => return Err(broken(err)),
            }
        }
    }
}

/// Whether a header line's name is a token (RFC 9110 §5.1) of a sane
/// length.
fn header_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

/// The head of a request, whole, or why it cannot be sent.
fn request_head(call: &Call, agent: &str) -> Result<String, FetchError> {
    if call.headers.len() > MAX_HEADERS {
        return Err(FetchError::Address(say!("http-address-too-many-headers")));
    }
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: {agent}\r\n",
        call.method.as_str(),
        call.url.path(),
        call.url.authority(),
    );
    for (name, value) in &call.headers {
        let lower = name.to_ascii_lowercase();
        if !header_name(name) || OWN_HEADERS.contains(&lower.as_str()) {
            return Err(FetchError::Address(say!("http-address-header-name")));
        }
        // Printable, without a line break: nothing smuggled in with it.
        if value.is_empty()
            || value.len() > MAX_HEADER_VALUE
            || !value.bytes().all(|b| (0x20..=0x7e).contains(&b))
        {
            return Err(FetchError::Address(say!("http-address-header-value")));
        }
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("Accept-Encoding: identity\r\nConnection: close\r\n");
    let sends = matches!(call.method, Method::Post | Method::Put | Method::Patch);
    match &call.body {
        Some(body) if body.len() > MAX_SENT => return Err(FetchError::TooLarge),
        Some(body) => head.push_str(&format!("Content-Length: {}\r\n", body.len())),
        None if sends => head.push_str("Content-Length: 0\r\n"),
        None => {}
    }
    head.push_str("\r\n");
    Ok(head)
}

impl Client {
    /// Send one request and read its answer, its body written to `into`
    /// when there is one and the answer is what was asked for.
    fn exchange(
        &self,
        call: &Call,
        cancel: &AtomicBool,
        into: Option<&mut dyn io::Write>,
    ) -> Result<Response, FetchError> {
        let head = request_head(call, &self.agent)?;
        let clock = Clock::new(call.limits.total, cancel);
        clock.left()?;
        let mut stream = self.connect(&call.url, &call.limits, &clock)?;
        // Everything that is sent, whole.
        Self::write_whole(&mut stream, head.as_bytes(), &clock)?;
        if let Some(body) = &call.body {
            Self::write_whole(&mut stream, body, &clock)?;
        }
        let mut source = Timed {
            stream: &mut stream,
            clock: &clock,
        };
        match into {
            Some(into) => wire::read_response_to(&mut source, call.limits.body, into),
            None => wire::read_response(&mut source, call.limits.body),
        }
    }
}

impl Fetch for Client {
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError> {
        self.exchange(call, cancel, None)
    }

    fn send_to(
        &self,
        call: &Call,
        cancel: &AtomicBool,
        into: &mut dyn io::Write,
    ) -> Result<Response, FetchError> {
        self.exchange(call, cancel, Some(into))
    }

    fn get(&self, request: &Request<'_>, cancel: &AtomicBool) -> Result<Response, FetchError> {
        if !header_value(request.accept) {
            return Err(FetchError::Address(say!("http-address-accept")));
        }
        self.send(&Call::from_request(request), cancel)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_read_into_its_parts() {
        let url = Url::parse("https://Example.org/avatar/abc?s=64&d=404").unwrap();
        assert!(url.secure());
        assert_eq!(url.host(), "example.org");
        assert_eq!(url.port(), 443);
        assert_eq!(url.path(), "/avatar/abc?s=64&d=404");
        assert_eq!(url.authority(), "example.org");
        assert_eq!(url.to_string(), "https://example.org/avatar/abc?s=64&d=404");

        let local = Url::parse(" http://127.0.0.1:8080 ").unwrap();
        assert!(!local.secure());
        assert_eq!(
            (local.host(), local.port(), local.path()),
            ("127.0.0.1", 8080, "/")
        );
        assert_eq!(local.authority(), "127.0.0.1:8080");

        let six = Url::parse("http://[::1]:9000/x#top").unwrap();
        assert_eq!((six.host(), six.port(), six.path()), ("::1", 9000, "/x"));
        assert_eq!(six.authority(), "[::1]:9000");
        assert_eq!(Url::parse("https://[::1]").unwrap().authority(), "[::1]");

        let query = Url::parse("https://example.org?x=1").unwrap();
        assert_eq!(query.path(), "/?x=1");
    }

    #[test]
    fn what_is_not_an_address_is_refused() {
        for text in [
            "",
            "example.org",
            "ftp://example.org/",
            "https://",
            "https:///path",
            "https://user@example.org/",
            "https://user:secret@example.org/",
            "https://example.org:0/",
            "https://example.org:99999/",
            "https://example.org:http/",
            "https://exa mple.org/",
            "https://example.org/a b",
            "https://example.org/a\r\nHost: other",
            "https://example.org/\u{e9}",
            "https://ex\u{e9}mple.org/",
            "https://.example.org/",
            "https://example..org/",
            "https://[::1/",
            "https://[nonsense]/",
            "https://[::1]x/",
        ] {
            assert!(Url::parse(text).is_err(), "{text:?} was taken");
        }
        let long = format!("https://example.org/{}", "a".repeat(MAX_URL));
        assert_eq!(Url::parse(&long), Err(FetchError::Address("too long")));
    }

    #[test]
    fn a_location_is_followed_from_where_it_was_given() {
        let from = Url::parse("https://example.org/avatar/abc?s=64").unwrap();
        let join = |location: &str| from.join(location).map(|url| url.to_string());
        assert_eq!(
            join("https://www.example.org/a/b").unwrap(),
            "https://www.example.org/a/b"
        );
        assert_eq!(
            join("//cdn.example.org/x").unwrap(),
            "https://cdn.example.org/x"
        );
        assert_eq!(join("/other?y=2").unwrap(), "https://example.org/other?y=2");
        assert_eq!(join("def").unwrap(), "https://example.org/avatar/def");
        assert!(join("/a b").is_err());
        assert!(join("gopher://example.org/").is_err());
        let local = Url::parse("http://127.0.0.1:8080/a").unwrap();
        assert_eq!(
            local.join("/b").unwrap().to_string(),
            "http://127.0.0.1:8080/b"
        );
    }

    #[test]
    fn a_request_that_is_not_wanted_or_has_no_time_is_not_made() {
        let client = Client::new("Test/1");
        let url = Url::parse("http://127.0.0.1:9/").unwrap();
        let limits = Limits {
            connect: Duration::from_secs(1),
            total: Duration::from_secs(1),
            body: 16,
        };
        let request = Request {
            url: &url,
            accept: "*/*",
            limits,
        };
        let stop = AtomicBool::new(true);
        assert_eq!(client.get(&request, &stop), Err(FetchError::Cancelled));
        let go = AtomicBool::new(false);
        let none = Request {
            limits: Limits {
                total: Duration::ZERO,
                ..limits
            },
            ..request
        };
        assert_eq!(client.get(&none, &go), Err(FetchError::Timeout));
        let odd = Request {
            accept: "a\r\nX: y",
            ..request
        };
        assert!(matches!(client.get(&odd, &go), Err(FetchError::Address(_))));
    }

    #[test]
    fn plain_http_goes_to_this_machine_only() {
        let client = Client::new("Test/1");
        let go = AtomicBool::new(false);
        let limits = Limits {
            connect: Duration::from_secs(2),
            total: Duration::from_secs(2),
            body: 16,
        };
        // An address of the documentation range: nothing is sent to it,
        // the refusal comes before any connection.
        let url = Url::parse("http://192.0.2.1/").unwrap();
        let request = Request {
            url: &url,
            accept: "*/*",
            limits,
        };
        assert_eq!(client.get(&request, &go), Err(FetchError::Insecure));
    }

    #[test]
    fn an_agent_that_could_break_a_request_is_not_used() {
        assert_eq!(Client::new("BitBraid/0.0.1").agent, "BitBraid/0.0.1");
        assert_eq!(Client::new("a\r\nb").agent, "BitBraid");
        assert_eq!(Client::new("").agent, "BitBraid");
    }
}
