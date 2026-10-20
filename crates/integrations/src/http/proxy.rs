//! The proxy the client goes through, where it is named, and the tunnel
//! through it.
//!
//! The client asks HTTPS addresses through a proxy when one is named, as
//! curl does: in `https_proxy` or `HTTPS_PROXY`, else in `all_proxy` or
//! `ALL_PROXY` (the small letters first, an empty value as none), else, as
//! a fallback of the application's own, in git's `http.proxy` of the
//! user's and the system's configuration. `no_proxy` or `NO_PROXY` names
//! hosts that are asked directly whatever names a proxy: `*` for all, a
//! name for itself and every name under it (a dot before it changes
//! nothing), an address, or a range of addresses in CIDR notation. The
//! variables are read as the programs the application starts see them
//! (with the login shell's environment when it was adopted,
//! [`bitbraid_core::process::var_os`]), once, at the first request that
//! could go through a proxy.
//!
//! What is never asked through one: plain HTTP, which the client speaks to
//! this machine only, and this machine itself (`localhost` and the
//! loopback addresses), which a proxy on another machine could not reach;
//! so `http_proxy` is not read. Go and Python's `requests` keep the
//! loopback out of proxies the same way; curl does not, and a proxy named
//! for everything there has the user add `localhost` to `NO_PROXY`.
//!
//! A proxy is an HTTP proxy, spoken to in plain HTTP:
//! `[http://][user[:password]@]host[:port]`, port 1080 when none is
//! given, as curl has it. Through it the client asks for a tunnel to the
//! host (`CONNECT host:port`) and speaks TLS with the host inside it, so
//! the proxy sees which host and port, and nothing of what is asked or
//! answered. A login in the proxy's address is sent with the `CONNECT`
//! (Basic); the proxy's own answer is all there is to it, nothing is
//! asked of the user. A proxy that wants a login and has none, one that
//! refuses the login or the tunnel, one that is not there and one that
//! cannot be used (a SOCKS proxy, a proxy spoken to over TLS, an address
//! that is not one) end the request with an error that says so; a request
//! never goes around a proxy that was named.
//!
//! Not read: `http.<url>.proxy`, `http.proxyAuthMethod`, a proxy's
//! certificate settings; the proxy is asked for Basic only. A proxy whose
//! 407 offers no Basic (one that takes Negotiate, which is Kerberos, or
//! NTLM only, as a proxy of an Active Directory domain may) ends the
//! request with an error that names what it asked for and says that the
//! application's own connections give Basic only.
//!
//! Sources: curl(1) "ENVIRONMENT" and "-x, --proxy" (the variables, which
//! case is read, `NO_PROXY` with names, addresses and CIDR ranges, port
//! 1080), git-config(1) (`http.proxy`), RFC 9110 §9.3.6 (CONNECT), §11.7
//! (`Proxy-Authorization`, `Proxy-Authenticate`), §15.5.8 (407); RFC 9112
//! §3.2.3 (the authority form of a request target); RFC 7617 (Basic); RFC
//! 3986 §3.2 (the authority of an address, percent-encoding); RFC 9110
//! §5.6 and §11.6.1 (tokens, quoted strings, challenges); RFC 4559
//! (Negotiate over HTTP); RFC 4648 §4
//! (base64); RFC 4632 (CIDR); the documentation of Go's
//! `httpproxy.Config` and of `httparse`.

use std::{
    fmt,
    io::{Read as _, Write as _},
    net::{IpAddr, Ipv6Addr, TcpStream},
};

use bitbraid_strings::Message;

use super::{Clock, FetchError, Url, broken, only_waiting};

/// Port of a proxy whose address names none, as curl takes it.
pub const DEFAULT_PORT: u16 = 1080;

/// Longest proxy address or `NO_PROXY` list read.
const LONGEST: usize = 4096;

/// Longest answer to a `CONNECT` read, head only.
const ANSWER_LIMIT: usize = 8 * 1024;

/// How long git is given to say what its `http.proxy` is.
const GIT_PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// An HTTP proxy.
#[derive(Clone, PartialEq, Eq)]
pub struct ProxyAddress {
    host: String,
    port: u16,
    login: Option<(String, String)>,
}

impl fmt::Debug for ProxyAddress {
    /// Without the password.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProxyAddress")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.login.as_ref().map(|(user, _)| user))
            .finish()
    }
}

/// `%XX` decoded; `None` for a `%` not followed by two hex digits, for
/// what is not UTF-8 then, and for a control character.
fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while let Some(byte) = bytes.get(at) {
        if *byte == b'%' {
            let pair = bytes.get(at.saturating_add(1)..at.saturating_add(3))?;
            let pair = std::str::from_utf8(pair).ok()?;
            out.push(u8::from_str_radix(pair, 16).ok()?);
            at = at.saturating_add(3);
        } else {
            out.push(*byte);
            at = at.saturating_add(1);
        }
    }
    let text = String::from_utf8(out).ok()?;
    (!text.chars().any(char::is_control)).then_some(text)
}

/// Whether `host` is a name an address can have, or an IPv6 address.
fn host_name(host: &str) -> bool {
    let named = !host.is_empty()
        && host.len() <= 253
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && !host.starts_with(['.', '-'])
        && !host.contains("..");
    named || host.parse::<Ipv6Addr>().is_ok()
}

impl ProxyAddress {
    /// Read `[http://][user[:password]@]host[:port][/]`. What cannot be
    /// used is said in words, without the password.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.is_empty() {
            return Err(say!("http-proxy-address-empty").to_owned());
        }
        if text.len() > LONGEST {
            return Err(say!("http-proxy-address-too-long").to_owned());
        }
        if text.chars().any(|c| c.is_control() || c.is_whitespace()) {
            return Err(say!("http-proxy-address-space").to_owned());
        }
        let rest = match text.split_once("://") {
            Some((scheme, rest)) => {
                let scheme = scheme.to_ascii_lowercase();
                match scheme.as_str() {
                    "http" => rest,
                    "https" => {
                        return Err(say!("http-proxy-over-tls").into());
                    }
                    "socks4" | "socks4a" | "socks5" | "socks5h" => {
                        return Err(say!("http-proxy-socks", scheme = scheme.as_str()));
                    }
                    _ => return Err(say!("http-proxy-scheme").to_owned()),
                }
            }
            None => text,
        };
        // A path after the host says nothing to a proxy.
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let (login, place) = match authority.rsplit_once('@') {
            Some((userinfo, place)) => {
                let (user, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
                let user =
                    percent_decoded(user).ok_or_else(|| say!("http-proxy-user-name").to_owned())?;
                let password = percent_decoded(password)
                    .ok_or_else(|| say!("http-proxy-password").to_owned())?;
                (Some((user, password)), place)
            }
            None => (None, authority),
        };
        let (host, port) = if let Some(inner) = place.strip_prefix('[') {
            let (host, after) = inner
                .split_once(']')
                .ok_or_else(|| say!("http-proxy-bracket").to_owned())?;
            match after.strip_prefix(':') {
                Some(port) => (host, Some(port)),
                None if after.is_empty() => (host, None),
                None => return Err(say!("http-proxy-port").to_owned()),
            }
        } else {
            match place.rsplit_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (place, None),
            }
        };
        if !host_name(host) {
            return Err(say!("http-proxy-host").to_owned());
        }
        let port = match port {
            None | Some("") => DEFAULT_PORT,
            Some(port) => port
                .parse::<u16>()
                .ok()
                .filter(|port| *port != 0)
                .ok_or_else(|| say!("http-proxy-port").to_owned())?,
        };
        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
            login,
        })
    }

    /// The host, in lower case; an IPv6 address without its brackets.
    pub fn host(&self) -> &str {
        &self.host
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Whether the address gives a login.
    pub fn has_login(&self) -> bool {
        self.login.is_some()
    }

    /// The host and port as text, an IPv6 address in brackets: what an
    /// error names it by.
    pub fn place(&self) -> String {
        if self.host.contains(':') {
            format!("[{}]:{}", self.host, self.port)
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    /// The `Proxy-Authorization` line's value for the login, if any.
    fn authorization(&self) -> Option<String> {
        self.login.as_ref().map(|(user, password)| {
            format!("Basic {}", base64(format!("{user}:{password}").as_bytes()))
        })
    }
}

/// Base64 of RFC 4648 §4, with padding.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let at = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (at(0) << 16) | (at(1) << 8) | at(2);
        for i in 0..4usize {
            if i <= chunk.len() {
                let index = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap_or(0);
                out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'A')));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// One entry of `NO_PROXY`.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Bypass {
    /// `*`: every host.
    All,
    /// A name, for itself and every name under it.
    Name(String),
    /// An address, or the first `bits` of one.
    Range(IpAddr, u8),
}

/// The hosts that are asked directly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoProxy(Vec<Bypass>);

impl NoProxy {
    /// Read a list as `NO_PROXY` has it: entries split by commas or
    /// spaces. An entry that is none of the kinds is left out.
    pub fn parse(text: &str) -> Self {
        let mut end = LONGEST.min(text.len());
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        let text = text.get(..end).unwrap_or_default();
        let entries = text
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|entry| !entry.is_empty())
            .filter_map(|entry| {
                if entry == "*" {
                    return Some(Bypass::All);
                }
                let (address, bits) = match entry.split_once('/') {
                    Some((address, bits)) => (address, Some(bits)),
                    None => (entry, None),
                };
                let address = address
                    .strip_prefix('[')
                    .and_then(|inner| inner.strip_suffix(']'))
                    .unwrap_or(address);
                if let Ok(ip) = address.parse::<IpAddr>() {
                    let most: u8 = if ip.is_ipv4() { 32 } else { 128 };
                    let bits = match bits {
                        None => most,
                        Some(bits) => bits.parse::<u8>().ok().filter(|bits| *bits <= most)?,
                    };
                    return Some(Bypass::Range(ip, bits));
                }
                if bits.is_some() {
                    return None;
                }
                let name = entry.trim_start_matches('.').trim_end_matches('.');
                host_name(name).then(|| Bypass::Name(name.to_ascii_lowercase()))
            })
            .collect();
        Self(entries)
    }

    /// Whether `host` (a name in lower case, or an address without
    /// brackets) is asked directly.
    pub fn bypasses(&self, host: &str) -> bool {
        let host = host.trim_end_matches('.');
        let ip = host.parse::<IpAddr>().ok();
        self.0.iter().any(|entry| match entry {
            Bypass::All => true,
            Bypass::Name(name) => {
                ip.is_none()
                    && (host == name
                        || host
                            .strip_suffix(name.as_str())
                            .is_some_and(|before| before.ends_with('.')))
            }
            Bypass::Range(range, bits) => ip.is_some_and(|ip| within(ip, *range, *bits)),
        })
    }
}

/// Whether the first `bits` of `ip` are those of `range`.
fn within(ip: IpAddr, range: IpAddr, bits: u8) -> bool {
    let (ip, range, width) = match (ip, range) {
        (IpAddr::V4(ip), IpAddr::V4(range)) => (
            u128::from(u32::from(ip)),
            u128::from(u32::from(range)),
            32u32,
        ),
        (IpAddr::V6(ip), IpAddr::V6(range)) => (u128::from(ip), u128::from(range), 128u32),
        _ => return false,
    };
    let bits = u32::from(bits).min(width);
    if bits == 0 {
        return true;
    }
    let shift = width.saturating_sub(bits);
    ip.checked_shr(shift) == range.checked_shr(shift)
}

/// A proxy that was named, and where.
#[derive(Clone, PartialEq, Eq)]
struct Named {
    /// Where it was named: a variable, or git's setting.
    from: String,
    /// The proxy, or why the one named cannot be used.
    proxy: Result<ProxyAddress, String>,
}

impl fmt::Debug for Named {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Named")
            .field("from", &self.from)
            .field("proxy", &self.proxy)
            .finish()
    }
}

/// Which proxy the client's HTTPS requests go through, and which hosts
/// they do not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Proxies {
    https: Option<Named>,
    no_proxy: NoProxy,
}

/// How one request goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route<'a> {
    /// Straight to the host.
    Direct,
    /// Through this proxy.
    Through(&'a ProxyAddress),
    /// Not at all: the proxy named cannot be used. Where it was named,
    /// and why.
    Unusable(&'a str, &'a str),
}

/// Where git's own setting for a proxy is said to come from.
pub const GIT_SETTING: Message = message!("http-proxy-git-setting");

/// The variables that name the proxy of HTTPS, in the order they count.
const HTTPS_VARIABLES: &[&str] = &["https_proxy", "HTTPS_PROXY", "all_proxy", "ALL_PROXY"];

/// How the environment writes the variable `name`, of
/// [`HTTPS_VARIABLES`], among the names `written` (Windows, where a name
/// is found whatever its case). None for any other name, git's setting
/// among them.
fn spelled_as(name: &str, mut written: impl Iterator<Item = String>) -> Option<String> {
    if !HTTPS_VARIABLES.contains(&name) {
        return None;
    }
    written.find(|written| written.eq_ignore_ascii_case(name))
}

impl Proxies {
    /// No proxy: every request goes straight to its host.
    pub fn none() -> Self {
        Self::default()
    }

    /// This proxy for every HTTPS request, as though a setting named it.
    pub fn through(proxy: ProxyAddress, from: &str) -> Self {
        Self {
            https: Some(Named {
                from: from.to_owned(),
                proxy: Ok(proxy),
            }),
            no_proxy: NoProxy::default(),
        }
    }

    /// The same, with these hosts asked directly.
    pub fn bypassing(mut self, no_proxy: NoProxy) -> Self {
        self.no_proxy = no_proxy;
        self
    }

    /// What the variables say, as `var` gives them, and else what git's
    /// `http.proxy` says, as `git_setting` gives it; `git_setting` is
    /// asked only when no variable names a proxy.
    pub fn from_values(
        var: impl Fn(&str) -> Option<String>,
        git_setting: impl FnOnce() -> Option<String>,
    ) -> Self {
        let set = |name: &str| var(name).filter(|value| !value.trim().is_empty());
        let named = HTTPS_VARIABLES
            .iter()
            .find_map(|name| set(name).map(|value| (name.to_string(), value)))
            .or_else(|| {
                git_setting()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| (GIT_SETTING.text().to_owned(), value))
            });
        let no_proxy = set("no_proxy")
            .or_else(|| set("NO_PROXY"))
            .map(|list| NoProxy::parse(&list))
            .unwrap_or_default();
        Self {
            https: named.map(|(from, value)| Named {
                proxy: ProxyAddress::parse(&value),
                from,
            }),
            no_proxy,
        }
    }

    /// What the user's surroundings say: the variables as the programs
    /// the application starts see them, and git's own configuration
    /// outside any repository (the user's and the system's files).
    pub fn from_surroundings() -> Self {
        let mut proxies = Self::from_values(
            |name| bitbraid_core::process::var_os(name).and_then(|value| value.into_string().ok()),
            // Its value stays out of the activity log: it may hold the
            // proxy's password.
            || bitbraid_git::exe::user_setting("http.proxy", GIT_PATIENCE),
        );
        // On Windows a variable's name has no case: `https_proxy` is asked
        // for first and finds HTTPS_PROXY. It is named as it is written.
        if cfg!(windows)
            && let Some(named) = proxies.https.as_mut()
            && let Some(written) = spelled_as(
                &named.from,
                std::env::vars_os().filter_map(|(name, _)| name.into_string().ok()),
            )
        {
            named.from = written;
        }
        proxies
    }

    /// Whether a proxy is named at all, usable or not.
    pub fn named(&self) -> bool {
        self.https.is_some()
    }

    /// How a request for `url` goes.
    pub fn route(&self, url: &Url) -> Route<'_> {
        if stays_here(url) || self.no_proxy.bypasses(url.host()) {
            return Route::Direct;
        }
        match &self.https {
            None => Route::Direct,
            Some(Named {
                proxy: Ok(proxy), ..
            }) => Route::Through(proxy),
            Some(Named {
                from,
                proxy: Err(why),
            }) => Route::Unusable(from, why),
        }
    }
}

/// Whether a request never goes through a proxy: plain HTTP (to this
/// machine only), and this machine by its name or address.
pub(super) fn stays_here(url: &Url) -> bool {
    let host = url.host().trim_end_matches('.');
    !url.secure()
        || host == "localhost"
        || host.ends_with(".localhost")
        || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// What is asked for in the `CONNECT`: the host and the port, always, an
/// IPv6 address in brackets.
fn tunnel_target(url: &Url) -> String {
    if url.host().contains(':') {
        format!("[{}]:{}", url.host(), url.port())
    } else {
        format!("{}:{}", url.host(), url.port())
    }
}

/// Ask the proxy on `socket` for a tunnel to `url`'s host and read its
/// answer. On success the connection is the tunnel: what is written next
/// reaches the host.
pub(super) fn open_tunnel(
    socket: &mut TcpStream,
    proxy: &ProxyAddress,
    url: &Url,
    agent: &str,
    clock: &Clock<'_>,
) -> Result<(), FetchError> {
    let target = tunnel_target(url);
    let mut head =
        format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nUser-Agent: {agent}\r\n");
    if let Some(authorization) = proxy.authorization() {
        head.push_str("Proxy-Authorization: ");
        head.push_str(&authorization);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    let mut rest = head.as_bytes();
    while !rest.is_empty() {
        socket
            .set_write_timeout(Some(clock.slice()?))
            .map_err(broken)?;
        match socket.write(rest) {
            Ok(0) => {
                return Err(FetchError::Broken(
                    say!("http-proxy-takes-nothing-more").to_owned(),
                ));
            }
            Ok(sent) => rest = rest.get(sent..).unwrap_or_default(),
            Err(err) if only_waiting(&err) => {}
            Err(err) => return Err(broken(err)),
        }
    }
    let answer = read_answer(socket, clock)?;
    let (status, schemes) = status_of(&answer)?;
    match status {
        200..=299 => Ok(()),
        // A proxy that offers no Basic: what it wants is said, whatever the
        // address gives (a login it gives went as Basic, of no use to it).
        407 if !offers_basic(&schemes) => Err(FetchError::Proxy(say!(
            "http-proxy-wants-other-sign-in",
            place = proxy.place(),
            schemes = said_schemes(&schemes)
        ))),
        407 if proxy.has_login() => Err(FetchError::Proxy(say!(
            "http-proxy-login-refused",
            place = proxy.place()
        ))),
        407 => Err(FetchError::Proxy(say!(
            "http-proxy-login-wanted",
            place = proxy.place()
        ))),
        // The proxy is there and could not reach the host: as though the
        // host could not be reached.
        500..=599 => Err(FetchError::Connect(say!(
            "http-proxy-unreachable",
            place = proxy.place(),
            target = target.as_str(),
            status
        ))),
        _ => Err(FetchError::Proxy(say!(
            "http-proxy-refused",
            place = proxy.place(),
            target = target.as_str(),
            status
        ))),
    }
}

/// The head of the proxy's answer, read a byte at a time: after a head
/// that opens a tunnel, what comes is the host's, and none of it may be
/// taken here.
fn read_answer(socket: &mut TcpStream, clock: &Clock<'_>) -> Result<Vec<u8>, FetchError> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if head.len() >= ANSWER_LIMIT {
            return Err(FetchError::Malformed(say!("http-proxy-answer-too-long")));
        }
        socket
            .set_read_timeout(Some(clock.slice()?))
            .map_err(broken)?;
        match socket.read(&mut byte) {
            Ok(0) => {
                return Err(FetchError::Proxy(say!("http-proxy-closed").to_owned()));
            }
            Ok(_) => head.extend_from_slice(&byte),
            Err(err) if only_waiting(&err) => {}
            Err(err) => return Err(broken(err)),
        }
    }
    Ok(head)
}

/// Most header lines of a proxy's answer read.
const MOST_HEADERS: usize = 64;

/// Most ways of signing in kept from a proxy's answer.
const MOST_SCHEMES: usize = 8;

/// The status of an answer's head, and the ways of signing in its
/// `Proxy-Authenticate` lines offer.
fn status_of(head: &[u8]) -> Result<(u16, Vec<String>), FetchError> {
    let mut headers = [httparse::EMPTY_HEADER; MOST_HEADERS];
    let mut parsed = httparse::Response::new(&mut headers);
    match parsed.parse(head) {
        Ok(httparse::Status::Complete(_)) => {}
        _ => return Err(FetchError::Malformed(say!("http-proxy-answer"))),
    }
    let status = parsed
        .code
        .filter(|code| (100..=599).contains(code))
        .ok_or(FetchError::Malformed(say!("http-proxy-answer")))?;
    let mut schemes: Vec<String> = Vec::new();
    for header in parsed.headers.iter() {
        if !header.name.eq_ignore_ascii_case("proxy-authenticate") {
            continue;
        }
        let Ok(value) = std::str::from_utf8(header.value) else {
            continue;
        };
        for scheme in challenge_schemes(value) {
            if schemes.len() < MOST_SCHEMES
                && !schemes
                    .iter()
                    .any(|known| known.eq_ignore_ascii_case(&scheme))
            {
                schemes.push(scheme);
            }
        }
    }
    Ok((status, schemes))
}

/// Whether `c` may be in a token (RFC 9110 §5.6.2).
fn token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || "!#$%&'*+-.^_`|~".contains(c)
}

/// The schemes of the challenges in a `Proxy-Authenticate` value
/// (RFC 9110 §11.6.1): its elements split at the commas that are not in a
/// quoted string, and of each the first word, when that is a token and not
/// a parameter (`realm="..."`) of the challenge before it.
fn challenge_schemes(value: &str) -> Vec<String> {
    let mut elements = Vec::new();
    let mut current = String::new();
    let (mut quoted, mut escaped) = (false, false);
    for c in value.chars() {
        if escaped {
            escaped = false;
        } else if quoted && c == '\\' {
            escaped = true;
        } else if c == '"' {
            quoted = !quoted;
        } else if c == ',' && !quoted {
            elements.push(std::mem::take(&mut current));
            continue;
        }
        current.push(c);
    }
    elements.push(current);
    elements
        .iter()
        .filter_map(|element| {
            let word = element.split_whitespace().next()?;
            (word.len() <= 32 && word.chars().all(token_char)).then(|| word.to_owned())
        })
        .collect()
}

/// Whether a proxy that answered 407 takes Basic: it says so, or says
/// nothing of what it takes.
fn offers_basic(schemes: &[String]) -> bool {
    schemes.is_empty()
        || schemes
            .iter()
            .any(|scheme| scheme.eq_ignore_ascii_case("basic"))
}

/// The ways of signing in, as a reader knows them: "Negotiate (Kerberos)
/// or NTLM".
fn said_schemes(schemes: &[String]) -> String {
    let named: Vec<String> = schemes
        .iter()
        .map(|scheme| match scheme.to_ascii_lowercase().as_str() {
            "negotiate" => "Negotiate (Kerberos)".to_owned(),
            "kerberos" => "Kerberos".to_owned(),
            "ntlm" => "NTLM".to_owned(),
            "digest" => "Digest".to_owned(),
            "bearer" => "Bearer".to_owned(),
            _ => scheme.clone(),
        })
        .collect();
    match named.split_last() {
        None => say!("http-proxy-scheme-unnamed").to_owned(),
        Some((last, [])) => last.clone(),
        Some((last, rest)) => say!(
            "http-proxy-schemes-or",
            rest = rest.join(", "),
            last = last.as_str()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> ProxyAddress {
        ProxyAddress::parse(text).unwrap()
    }

    #[test]
    fn a_variable_is_named_as_the_environment_writes_it() {
        let written = || {
            ["Path", "HTTPS_PROXY", "no_proxy"]
                .map(str::to_owned)
                .into_iter()
        };
        assert_eq!(
            spelled_as("https_proxy", written()).as_deref(),
            Some("HTTPS_PROXY")
        );
        assert_eq!(spelled_as("all_proxy", written()), None);
        // Only the proxy's own variables: git's setting keeps its words.
        assert_eq!(spelled_as("path", written()), None);
        assert_eq!(spelled_as(GIT_SETTING.text(), written()), None);
    }

    #[test]
    fn a_proxy_address_is_read_as_curl_and_git_write_it() {
        let plain = address("proxy.example.org:3128");
        assert_eq!(
            (plain.host(), plain.port(), plain.has_login()),
            ("proxy.example.org", 3128, false)
        );
        let scheme = address(" HTTP://Proxy.Example.org:8080/ ");
        assert_eq!((scheme.host(), scheme.port()), ("proxy.example.org", 8080));
        assert_eq!(address("http://proxy.example.org").port(), DEFAULT_PORT);
        assert_eq!(address("proxy.example.org:").port(), DEFAULT_PORT);
        let six = address("http://[::1]:3128");
        assert_eq!((six.host(), six.port()), ("::1", 3128));
        assert_eq!(six.place(), "[::1]:3128");
        assert_eq!(address("[fe80::1]").port(), DEFAULT_PORT);

        let login = address("http://carol:pa%3Ass%40word@10.0.0.1:3128");
        assert_eq!(
            login.login,
            Some(("carol".to_owned(), "pa:ss@word".to_owned()))
        );
        assert_eq!(login.host(), "10.0.0.1");
        // Basic of "carol:pa:ss@word".
        assert_eq!(
            login.authorization().as_deref(),
            Some("Basic Y2Fyb2w6cGE6c3NAd29yZA==")
        );
        let user_only = address("http://carol@proxy.example.org");
        assert_eq!(user_only.login, Some(("carol".to_owned(), String::new())));
        // The last @ ends the login.
        assert_eq!(
            address("a@b@proxy.example.org").login,
            Some(("a@b".to_owned(), String::new()))
        );
    }

    #[test]
    fn what_cannot_be_used_as_a_proxy_is_said_and_never_shows_the_password() {
        for (text, said) in [
            ("", "empty"),
            ("socks5://127.0.0.1:1080", "SOCKS"),
            ("socks5h://user:secret@proxy:1080", "SOCKS"),
            ("https://proxy.example.org", "TLS"),
            ("ftp://proxy.example.org", "scheme"),
            ("http://", "host"),
            ("http://:3128", "host"),
            ("http://proxy.example.org:0", "port"),
            ("http://proxy.example.org:99999", "port"),
            ("http://proxy.example.org:http", "port"),
            ("http://pro xy.example.org", "space"),
            ("http://proxy.example.org\r\nX: y", "control"),
            ("http://[::1", "bracket"),
            ("http://[::1]x", "port"),
            ("http://user:%zz@proxy", "password"),
            ("http://%0a@proxy", "user name"),
            ("http://.proxy", "host"),
            ("http://proxy..example", "host"),
        ] {
            let err = ProxyAddress::parse(text).unwrap_err();
            assert!(err.contains(said), "{text:?}: {err}");
            assert!(!err.contains("secret"), "{err}");
        }
        assert!(ProxyAddress::parse(&"a".repeat(LONGEST + 1)).is_err());
        let debug = format!("{:?}", address("http://carol:secret@proxy:1"));
        assert!(
            debug.contains("carol") && !debug.contains("secret"),
            "{debug}"
        );
    }

    #[test]
    fn no_proxy_takes_names_with_what_is_under_them_addresses_and_ranges() {
        let list = NoProxy::parse(
            "example.org, .internal.test ,10.0.0.0/8 192.168.1.7,[::1],fd00::/8,bad/8,,",
        );
        for (host, direct) in [
            ("example.org", true),
            ("api.example.org", true),
            ("EXAMPLE.org".to_ascii_lowercase().as_str(), true),
            ("example.org.", true),
            ("badexample.org", false),
            ("example.org.evil.test", false),
            ("internal.test", true),
            ("git.internal.test", true),
            ("10.1.2.3", true),
            ("11.1.2.3", false),
            ("192.168.1.7", true),
            ("192.168.1.8", false),
            ("::1", true),
            ("fd12:3456::1", true),
            ("fe80::1", false),
            ("github.com", false),
        ] {
            assert_eq!(list.bypasses(host), direct, "{host}");
        }
        assert!(NoProxy::parse("*").bypasses("anything.example"));
        assert!(NoProxy::parse("*").bypasses("192.0.2.1"));
        assert!(!NoProxy::parse("").bypasses("example.org"));
        // A name never takes an address: `0.0.1` does not take 10.0.0.1.
        assert!(!NoProxy::parse("0.0.1").bypasses("10.0.0.1"));
        assert!(NoProxy::parse("0.0.0.0/0").bypasses("203.0.113.9"));
        assert!(!NoProxy::parse("0.0.0.0/0").bypasses("::1"));
        assert!(NoProxy::parse("1.2.3.4/33").0.is_empty());
        assert!(NoProxy::parse(&"a,".repeat(10_000)).0.len() <= LONGEST);
    }

    fn values<'a>(set: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            set.iter()
                .find(|(known, _)| *known == name)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    fn from(proxies: &Proxies) -> Option<(&str, &str)> {
        proxies.https.as_ref().map(|named| {
            (
                named.from.as_str(),
                named.proxy.as_ref().map_or("", |p| p.host()),
            )
        })
    }

    #[test]
    fn the_variables_count_in_curls_order_and_git_only_when_they_name_none() {
        let git = || Some("http://git-proxy.example:3128".to_owned());
        let all = [
            ("https_proxy", "small.example:1"),
            ("HTTPS_PROXY", "capital.example:1"),
            ("all_proxy", "all-small.example:1"),
            ("ALL_PROXY", "all-capital.example:1"),
            ("http_proxy", "never.example:1"),
            ("HTTP_PROXY", "never.example:1"),
        ];
        let order = [
            ("https_proxy", "small.example"),
            ("HTTPS_PROXY", "capital.example"),
            ("all_proxy", "all-small.example"),
            ("ALL_PROXY", "all-capital.example"),
        ];
        for skip in 0..order.len() {
            let set: Vec<(&str, &str)> = all
                .iter()
                .filter(|(name, _)| !order[..skip].iter().any(|(gone, _)| gone == name))
                .copied()
                .collect();
            let proxies = Proxies::from_values(values(&set), || panic!("git is not asked"));
            assert_eq!(from(&proxies), Some(order[skip]), "{set:?}");
        }
        // http_proxy says nothing about HTTPS: git's setting counts.
        let proxies = Proxies::from_values(values(&all[4..]), git);
        assert_eq!(
            from(&proxies),
            Some((GIT_SETTING.text(), "git-proxy.example"))
        );
        // An empty variable is none; an empty setting too.
        let proxies = Proxies::from_values(values(&[("https_proxy", " ")]), git);
        assert_eq!(
            from(&proxies),
            Some((GIT_SETTING.text(), "git-proxy.example"))
        );
        let proxies = Proxies::from_values(values(&[]), || Some(String::new()));
        assert_eq!(proxies, Proxies::none());
        assert!(!proxies.named());
        // A proxy named that cannot be used is kept with why.
        let proxies = Proxies::from_values(values(&[("HTTPS_PROXY", "socks5://p:1")]), git);
        let url = Url::parse("https://api.github.com/user").unwrap();
        assert!(matches!(
            proxies.route(&url),
            Route::Unusable("HTTPS_PROXY", why) if why.contains("SOCKS")
        ));
        // no_proxy before NO_PROXY.
        let proxies = Proxies::from_values(
            values(&[
                ("https_proxy", "p:1"),
                ("no_proxy", "github.com"),
                ("NO_PROXY", "gitlab.com"),
            ]),
            git,
        );
        assert_eq!(proxies.route(&url), Route::Direct);
        let gitlab = Url::parse("https://gitlab.com/api/v4/user").unwrap();
        assert!(matches!(proxies.route(&gitlab), Route::Through(_)));
    }

    #[test]
    fn plain_http_and_this_machine_never_go_through_a_proxy() {
        let proxies = Proxies::through(address("proxy.example:3128"), "test");
        for (text, direct) in [
            ("http://127.0.0.1:8080/x", true),
            ("https://127.0.0.1:8443/x", true),
            ("https://127.1.2.3/x", true),
            ("https://[::1]:8443/x", true),
            ("https://localhost:8443/x", true),
            ("https://pictures.localhost/x", true),
            ("https://localhost./x", true),
            ("https://example.org/x", false),
            ("https://192.0.2.1/x", false),
            ("https://notlocalhost/x", false),
        ] {
            let url = Url::parse(text).unwrap();
            assert_eq!(proxies.route(&url) == Route::Direct, direct, "{text}");
        }
        assert_eq!(
            Proxies::none().route(&Url::parse("https://example.org/").unwrap()),
            Route::Direct
        );
        let bypassing = proxies.bypassing(NoProxy::parse("example.org"));
        assert_eq!(
            bypassing.route(&Url::parse("https://www.example.org/").unwrap()),
            Route::Direct
        );
    }

    #[test]
    fn a_tunnel_is_asked_for_by_host_and_port() {
        assert_eq!(
            tunnel_target(&Url::parse("https://api.github.com/user").unwrap()),
            "api.github.com:443"
        );
        assert_eq!(
            tunnel_target(&Url::parse("https://[2001:db8::1]:8443/").unwrap()),
            "[2001:db8::1]:8443"
        );
    }

    #[test]
    fn the_status_of_a_proxys_answer_is_read_or_refused() {
        assert_eq!(
            status_of(b"HTTP/1.1 200 Connection established\r\n\r\n"),
            Ok((200, vec![]))
        );
        assert_eq!(
            status_of(
                b"HTTP/1.0 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic\r\n\r\n"
            ),
            Ok((407, vec!["Basic".to_owned()]))
        );
        for nonsense in [
            &b"nonsense\r\n\r\n"[..],
            b"HTTP/1.1 999 What\r\n\r\n",
            b"HTTP/1.1 20",
        ] {
            assert!(status_of(nonsense).is_err(), "{nonsense:?}");
        }
    }

    #[test]
    fn hostile_text_in_the_variables_never_panics() {
        // Pieces a hostile or broken setting is made of, put together in
        // every order a simple generator of our own comes to.
        const PIECES: &[&str] = &[
            "http://",
            "https://",
            "socks5h://",
            "://",
            "@",
            ":",
            "[",
            "]",
            "/",
            "%",
            "%4",
            "%zz",
            "%00",
            "%e9",
            "*",
            ",",
            " ",
            ".",
            "..",
            "-",
            "0",
            "65535",
            "65536",
            "::1",
            "10.0.0.0",
            "/33",
            "/-1",
            "a",
            "proxy",
            "\u{e9}",
            "\u{1F600}",
            "\r\n",
            "\0",
            "localhost",
        ];
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let url = Url::parse("https://api.example.org/").unwrap();
        for _ in 0..20_000 {
            let mut text = String::new();
            for _ in 0..(next() % 12) {
                let at = usize::try_from(next() % PIECES.len() as u64).unwrap();
                text.push_str(PIECES[at]);
            }
            let _ = ProxyAddress::parse(&text).map(|proxy| (proxy.place(), proxy.authorization()));
            let list = NoProxy::parse(&text);
            let _ = list.bypasses(&text.to_ascii_lowercase());
            let _ = list.bypasses("api.example.org");
            let proxies = Proxies::from_values(
                |name| (name == "ALL_PROXY" || name == "NO_PROXY").then(|| text.clone()),
                || None,
            );
            let _ = proxies.route(&url);
        }
    }

    #[test]
    fn the_ways_a_proxy_asks_to_be_signed_in_to_are_read_from_its_407() {
        // Squid 6 with Negotiate, NTLM and Basic, a line each.
        let squid = b"HTTP/1.1 407 Proxy Authentication Required\r\n\
            Server: squid/6.14\r\n\
            Proxy-Authenticate: Negotiate\r\n\
            Proxy-Authenticate: NTLM\r\n\
            Proxy-Authenticate: Basic realm=\"BitBraid test proxy\"\r\n\
            Connection: keep-alive\r\n\r\n";
        let (status, schemes) = status_of(squid).unwrap();
        assert_eq!(status, 407);
        assert_eq!(schemes, ["Negotiate", "NTLM", "Basic"]);
        assert!(offers_basic(&schemes));
        // Several in one line, with parameters and commas inside quotes.
        for (value, wanted) in [
            (
                "Basic realm=\"a, NTLM b\", Negotiate",
                &["Basic", "Negotiate"][..],
            ),
            (
                "Digest realm=\"x\", qop=\"auth,auth-int\", nonce=\"1\", NTLM",
                &["Digest", "NTLM"],
            ),
            ("Negotiate abc+/==", &["Negotiate"]),
            ("  ,, Basic", &["Basic"]),
            ("realm=\"only a parameter\"", &[]),
            ("\"quoted\" Basic", &[]),
            ("Basic realm=\"open \\\" quote, NTLM\"", &["Basic"]),
            ("", &[]),
        ] {
            assert_eq!(challenge_schemes(value), wanted, "{value:?}");
        }
        assert!(offers_basic(&[]));
        assert!(!offers_basic(&["Negotiate".to_owned()]));
        assert_eq!(
            said_schemes(&["negotiate".to_owned()]),
            "Negotiate (Kerberos)"
        );
        assert_eq!(
            said_schemes(&["Negotiate".to_owned(), "NTLM".to_owned()]),
            "Negotiate (Kerberos) or NTLM"
        );
        assert_eq!(
            said_schemes(&["A".to_owned(), "b".to_owned(), "ntlm".to_owned()]),
            "A, b or NTLM"
        );
        // No more than a few kept, each once.
        let many: String = (0..40).map(|n| format!("S{n}, s{n}, ")).collect();
        let head = format!("HTTP/1.1 407 X\r\nProxy-Authenticate: {many}\r\n\r\n");
        assert_eq!(status_of(head.as_bytes()).unwrap().1.len(), MOST_SCHEMES);
    }

    #[test]
    fn hostile_challenges_never_panic() {
        const PIECES: &[&str] = &[
            "Basic",
            "NTLM",
            "Negotiate",
            " ",
            ",",
            "\"",
            "\\",
            "=",
            "realm",
            "\u{e9}",
            "\u{1F600}",
            "\t",
            "==",
            "a",
        ];
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..20_000 {
            let mut value = String::new();
            for _ in 0..(next() % 16) {
                let at = usize::try_from(next() % PIECES.len() as u64).unwrap();
                value.push_str(PIECES[at]);
            }
            let schemes = challenge_schemes(&value);
            let _ = (offers_basic(&schemes), said_schemes(&schemes));
            let head = format!("HTTP/1.1 407 X\r\nProxy-Authenticate: {value}\r\n\r\n");
            let _ = status_of(head.as_bytes());
        }
    }

    #[test]
    fn base64_is_rfc_4648s() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain.as_bytes()), coded);
        }
    }
}
