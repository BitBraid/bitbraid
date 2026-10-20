//! An HTTP forward proxy for tests, on a port of the loopback interface.
//!
//! It takes the two kinds of request a client sends a proxy: a `CONNECT
//! host:port`, after which it carries bytes both ways until either side
//! closes (a tunnel, for TLS or for ssh), and a request whose target is a
//! whole `http://` address, which it hands on in the form a server reads
//! and whose answer it carries back. It can ask for a user name and a
//! password (Basic), ask for a way of signing in that no test can give
//! (its 407 offering Negotiate or NTLM in place of Basic), refuse
//! everything, and it writes down every request it was asked, with what
//! it did with it.
//!
//! It never connects off the machine: whatever host a request names, the
//! connection is made to the loopback address, on the port the request
//! names. A test can therefore give a server a name that resolves nowhere
//! (`git.example.test`): a client that reaches it went through the proxy,
//! and one that tried to go around it finds nothing.
//!
//! One request to a connection: after the answer, or at the end of a
//! tunnel, the connection is closed.
//!
//! Sources: RFC 9110 §9.3.6 (CONNECT), §11.7.1 and §11.7.2
//! (`Proxy-Authenticate`, `Proxy-Authorization`), §15.5.8 (407); RFC 9112
//! §3.2.2 and §3.2.3 (absolute and authority form of a request target),
//! §7.6.1 (`Connection`); RFC 7617 (the Basic scheme); RFC 4559
//! (Negotiate).

use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use base64::Engine as _;

/// What the proxy asks for and does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ProxyOptions {
    /// The user name and password it asks for with Basic; it asks for
    /// none when absent.
    pub login: Option<(String, String)>,
    /// Refuse every request (403), after the login if one is asked for.
    pub refuse: bool,
    /// The `Proxy-Authenticate` lines of its 407, in place of Basic's
    /// (`Negotiate`, `NTLM`): a proxy that wants a way of signing in the
    /// tests cannot give. With these and no login, every request is
    /// turned away with 407; with a login, that login is still let in.
    pub challenges: Vec<String>,
}

/// What became of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Carried to the server, or a tunnel to it opened.
    Passed,
    /// Turned away with 407: no login came with it.
    WantsLogin,
    /// Turned away with 407: the login that came with it is not the one.
    WrongLogin,
    /// Turned away with 403.
    Refused,
    /// Nothing listens where it was going (502).
    Unreachable,
    /// Not a request a proxy takes (400).
    NotUnderstood,
}

/// One request the proxy was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Passage {
    pub method: String,
    /// Its target as it was written: `host:port` for a CONNECT, the whole
    /// address for another.
    pub target: String,
    /// The `Proxy-Authorization` line's value, as sent.
    pub authorization: Option<String>,
    /// What the proxy did with it.
    pub outcome: Outcome,
}

/// A running proxy. It stops when dropped.
#[derive(Debug)]
pub struct Proxy {
    port: u16,
    stop: Arc<AtomicBool>,
    passages: Arc<Mutex<Vec<Passage>>>,
    thread: Option<JoinHandle<()>>,
}

/// Longest head of a request read.
const MAX_HEAD: usize = 64 * 1024;
/// How long a quiet connection is waited on.
const PATIENCE: Duration = Duration::from_secs(60);

impl Proxy {
    /// Listen on a free port of the loopback interface.
    pub fn start(options: ProxyOptions) -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let passages = Arc::new(Mutex::new(Vec::new()));
        let options = Arc::new(options);
        let thread = {
            let (stop, passages) = (stop.clone(), passages.clone());
            std::thread::Builder::new()
                .name("test-proxy".to_owned())
                .spawn(move || {
                    for client in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let Ok(client) = client else { continue };
                        let (options, passages) = (options.clone(), passages.clone());
                        // A connection that cannot be served ends; the
                        // next one is served all the same.
                        let _ = std::thread::Builder::new()
                            .name("test-proxy-connection".to_owned())
                            .spawn(move || {
                                let _ = serve(client, &options, &passages);
                            });
                    }
                })?
        };
        Ok(Self {
            port,
            stop,
            passages,
            thread: Some(thread),
        })
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Its address: `http://127.0.0.1:<port>`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Its address with a user name, and a password when one is given:
    /// `http://user[:password]@127.0.0.1:<port>`.
    pub fn url_with(&self, user: &str, password: Option<&str>) -> String {
        match password {
            Some(password) => format!("http://{user}:{password}@127.0.0.1:{}", self.port),
            None => format!("http://{user}@127.0.0.1:{}", self.port),
        }
    }

    /// Every request asked so far, in order.
    pub fn passages(&self) -> Vec<Passage> {
        match self.passages.lock() {
            Ok(passages) => passages.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the thread out of its wait for a connection.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The value a `Proxy-Authorization` line has for this login.
pub fn basic(user: &str, password: &str) -> String {
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{password}"))
    )
}

/// The head of a request, read a byte at a time so that nothing after it
/// (a body, the first bytes of a tunnel) is taken off the connection.
fn read_head(stream: &mut TcpStream) -> io::Result<Option<String>> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 || head.len() > MAX_HEAD {
            return Ok(None);
        }
        head.extend_from_slice(&byte);
    }
    Ok(Some(String::from_utf8_lossy(&head).into_owned()))
}

/// Answer with a status and nothing else, and close.
fn refuse(client: &mut TcpStream, status: &str, extra: &str) -> io::Result<()> {
    client.write_all(
        format!("HTTP/1.1 {status}\r\n{extra}Content-Length: 0\r\nConnection: close\r\n\r\n")
            .as_bytes(),
    )?;
    let _ = client.flush();
    let _ = client.shutdown(Shutdown::Write);
    Ok(())
}

/// The port of `host[:port]`, or `usual` without one. An IPv6 address is
/// in brackets.
fn port_of(authority: &str, usual: u16) -> Option<u16> {
    let port = match authority.strip_prefix('[') {
        Some(inner) => {
            let after = inner.split_once(']')?.1;
            if after.is_empty() {
                return Some(usual);
            }
            after.strip_prefix(':')?
        }
        None => match authority.rsplit_once(':') {
            Some((_, port)) => port,
            None => return Some(usual),
        },
    };
    port.parse().ok().filter(|port| *port != 0)
}

fn serve(
    mut client: TcpStream,
    options: &ProxyOptions,
    passages: &Mutex<Vec<Passage>>,
) -> io::Result<()> {
    client.set_read_timeout(Some(PATIENCE))?;
    let Some(head) = read_head(&mut client)? else {
        return Ok(()); // the wake-up call, or a client that went away
    };
    let mut lines = head.split("\r\n");
    let request_line = lines.next().unwrap_or_default();
    let mut words = request_line.split(' ');
    let method = words.next().unwrap_or_default().to_owned();
    let target = words.next().unwrap_or_default().to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
        .collect();
    let authorization = headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("proxy-authorization"))
        .map(|(_, value)| value.clone());
    let note = |outcome: Outcome| {
        let passage = Passage {
            method: method.clone(),
            target: target.clone(),
            authorization: authorization.clone(),
            outcome,
        };
        match passages.lock() {
            Ok(mut passages) => passages.push(passage),
            Err(poisoned) => poisoned.into_inner().push(passage),
        }
    };

    if options.login.is_some() || !options.challenges.is_empty() {
        let wanted = options
            .login
            .as_ref()
            .map(|(user, password)| basic(user, password));
        let outcome = match (&authorization, &wanted) {
            (None, _) => Some(Outcome::WantsLogin),
            (Some(given), Some(wanted)) if given == wanted => None,
            (Some(_), _) => Some(Outcome::WrongLogin),
        };
        if let Some(outcome) = outcome {
            note(outcome);
            let challenges = if options.challenges.is_empty() {
                "Proxy-Authenticate: Basic realm=\"bitbraid test proxy\"\r\n".to_owned()
            } else {
                options
                    .challenges
                    .iter()
                    .map(|challenge| format!("Proxy-Authenticate: {challenge}\r\n"))
                    .collect()
            };
            return refuse(
                &mut client,
                "407 Proxy Authentication Required",
                &challenges,
            );
        }
    }
    if options.refuse {
        note(Outcome::Refused);
        return refuse(&mut client, "403 Forbidden", "");
    }

    // Where it goes, always on this machine, and what the server is sent.
    let (port, forwarded) = if method.eq_ignore_ascii_case("CONNECT") {
        (port_of(&target, 443), None)
    } else if let Some(rest) = target
        .get(..7)
        .filter(|scheme| scheme.eq_ignore_ascii_case("http://"))
        .and_then(|_| target.get(7..))
    {
        let (authority, path) = match rest.find(['/', '?']) {
            Some(at) => rest.split_at_checked(at).unwrap_or((rest, "/")),
            None => (rest, "/"),
        };
        let path = if path.starts_with('?') {
            format!("/{path}")
        } else {
            path.to_owned()
        };
        let mut head = format!("{method} {path} HTTP/1.1\r\n");
        for (name, value) in &headers {
            let lower = name.to_ascii_lowercase();
            if !matches!(
                lower.as_str(),
                "proxy-authorization" | "proxy-connection" | "connection" | "keep-alive"
            ) {
                head.push_str(&format!("{name}: {value}\r\n"));
            }
        }
        head.push_str("Connection: close\r\n\r\n");
        (port_of(authority, 80), Some(head))
    } else {
        (None, None)
    };
    let Some(port) = port else {
        note(Outcome::NotUnderstood);
        return refuse(&mut client, "400 Bad Request", "");
    };
    let Ok(mut server) = TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        Duration::from_secs(5),
    ) else {
        note(Outcome::Unreachable);
        return refuse(&mut client, "502 Bad Gateway", "");
    };
    server.set_read_timeout(Some(PATIENCE))?;
    note(Outcome::Passed);
    match forwarded {
        Some(head) => server.write_all(head.as_bytes())?,
        None => client.write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")?,
    }
    tunnel(client, server)
}

/// Carry bytes both ways until the server is done, then close both.
fn tunnel(mut client: TcpStream, mut server: TcpStream) -> io::Result<()> {
    let mut client_read = client.try_clone()?;
    let mut server_write = server.try_clone()?;
    let up = std::thread::Builder::new()
        .name("test-proxy-up".to_owned())
        .spawn(move || {
            let _ = io::copy(&mut client_read, &mut server_write);
            let _ = server_write.shutdown(Shutdown::Write);
        })?;
    let _ = io::copy(&mut server, &mut client);
    let _ = client.shutdown(Shutdown::Both);
    let _ = server.shutdown(Shutdown::Both);
    let _ = up.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A server that answers every connection with what it was sent.
    fn echo() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || {
                    let mut stream = stream;
                    let head = read_head(&mut stream).unwrap().unwrap_or_default();
                    let body = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{head}",
                        head.len()
                    );
                    let _ = stream.write_all(body.as_bytes());
                });
            }
        });
        port
    }

    fn ask(proxy: &Proxy, request: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", proxy.port())).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut answer = String::new();
        let _ = stream.read_to_string(&mut answer);
        answer
    }

    #[test]
    fn a_whole_address_is_handed_on_in_the_form_a_server_reads() {
        let port = echo();
        let proxy = Proxy::start(ProxyOptions::default()).unwrap();
        let answer = ask(
            &proxy,
            &format!(
                "GET http://git.example.test:{port}/a?b=c HTTP/1.1\r\n\
                 Host: git.example.test:{port}\r\nProxy-Connection: keep-alive\r\n\r\n"
            ),
        );
        assert!(answer.starts_with("HTTP/1.1 200 OK"), "{answer}");
        assert!(answer.contains("GET /a?b=c HTTP/1.1\r\n"), "{answer}");
        assert!(answer.contains(&format!("Host: git.example.test:{port}\r\n")));
        assert!(!answer.contains("Proxy-Connection"), "{answer}");
        let passages = proxy.passages();
        assert_eq!(passages.len(), 1);
        assert_eq!(passages[0].outcome, Outcome::Passed);
        assert_eq!(
            passages[0].target,
            format!("http://git.example.test:{port}/a?b=c")
        );
    }

    #[test]
    fn a_tunnel_carries_bytes_both_ways() {
        let port = echo();
        let proxy = Proxy::start(ProxyOptions::default()).unwrap();
        let answer = ask(
            &proxy,
            &format!(
                "CONNECT api.example.test:{port} HTTP/1.1\r\nHost: api.example.test:{port}\r\n\r\n\
                 GET /inside HTTP/1.1\r\n\r\n"
            ),
        );
        assert!(
            answer.starts_with("HTTP/1.1 200 Connection established\r\n\r\nHTTP/1.1 200 OK"),
            "{answer}"
        );
        assert!(answer.contains("GET /inside HTTP/1.1"), "{answer}");
        assert_eq!(proxy.passages()[0].method, "CONNECT");
    }

    #[test]
    fn a_login_is_asked_for_and_checked_and_a_refusal_is_a_refusal() {
        let port = echo();
        let proxy = Proxy::start(ProxyOptions {
            login: Some(("alice".to_owned(), "secret".to_owned())),
            refuse: false,
            challenges: Vec::new(),
        })
        .unwrap();
        let connect = |login: Option<&str>| {
            let line = login
                .map(|value| format!("Proxy-Authorization: {value}\r\n"))
                .unwrap_or_default();
            ask(
                &proxy,
                &format!("CONNECT h.example.test:{port} HTTP/1.1\r\n{line}\r\n"),
            )
        };
        let none = connect(None);
        assert!(none.starts_with("HTTP/1.1 407"), "{none}");
        assert!(none.contains("Proxy-Authenticate: Basic"), "{none}");
        assert!(connect(Some(&basic("alice", "wrong"))).starts_with("HTTP/1.1 407"));
        assert!(connect(Some(&basic("alice", "secret"))).starts_with("HTTP/1.1 200"));
        let outcomes: Vec<Outcome> = proxy.passages().iter().map(|p| p.outcome).collect();
        assert_eq!(
            outcomes,
            [Outcome::WantsLogin, Outcome::WrongLogin, Outcome::Passed]
        );

        let refusing = Proxy::start(ProxyOptions {
            login: None,
            refuse: true,
            challenges: Vec::new(),
        })
        .unwrap();
        let answer = ask(&refusing, &format!("CONNECT h:{port} HTTP/1.1\r\n\r\n"));
        assert!(answer.starts_with("HTTP/1.1 403"), "{answer}");
        assert_eq!(refusing.passages()[0].outcome, Outcome::Refused);
    }

    #[test]
    fn a_proxy_can_ask_for_ways_of_signing_in_nobody_can_give() {
        let port = echo();
        let negotiate = Proxy::start(ProxyOptions {
            challenges: vec!["Negotiate".to_owned(), "NTLM".to_owned()],
            ..ProxyOptions::default()
        })
        .unwrap();
        let connect = |proxy: &Proxy, login: Option<&str>| {
            let line = login
                .map(|value| format!("Proxy-Authorization: {value}\r\n"))
                .unwrap_or_default();
            ask(
                proxy,
                &format!("CONNECT h.example.test:{port} HTTP/1.1\r\n{line}\r\n"),
            )
        };
        let none = connect(&negotiate, None);
        assert!(none.starts_with("HTTP/1.1 407"), "{none}");
        assert!(
            none.contains("Proxy-Authenticate: Negotiate\r\nProxy-Authenticate: NTLM\r\n"),
            "{none}"
        );
        assert!(!none.contains("Basic"), "{none}");
        let some = connect(&negotiate, Some(&basic("alice", "secret")));
        assert!(some.starts_with("HTTP/1.1 407"), "{some}");
        let outcomes: Vec<Outcome> = negotiate.passages().iter().map(|p| p.outcome).collect();
        assert_eq!(outcomes, [Outcome::WantsLogin, Outcome::WrongLogin]);

        // Its challenges, and a login it still lets in.
        let both = Proxy::start(ProxyOptions {
            login: Some(("alice".to_owned(), "secret".to_owned())),
            challenges: vec!["Negotiate".to_owned(), "Basic realm=\"x\"".to_owned()],
            ..ProxyOptions::default()
        })
        .unwrap();
        assert!(connect(&both, None).contains("Proxy-Authenticate: Negotiate\r\n"));
        assert!(connect(&both, Some(&basic("alice", "secret"))).starts_with("HTTP/1.1 200"));
    }

    #[test]
    fn what_is_not_a_proxy_request_or_goes_nowhere_is_answered_so() {
        let proxy = Proxy::start(ProxyOptions::default()).unwrap();
        assert!(ask(&proxy, "GET /plain HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 400"));
        assert!(ask(&proxy, "CONNECT h:0 HTTP/1.1\r\n\r\n").starts_with("HTTP/1.1 400"));
        // A port nothing listens on: taken and let go again.
        let free = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        assert!(
            ask(&proxy, &format!("CONNECT h:{free} HTTP/1.1\r\n\r\n")).starts_with("HTTP/1.1 502")
        );
        let outcomes: Vec<Outcome> = proxy.passages().iter().map(|p| p.outcome).collect();
        assert_eq!(
            outcomes,
            [
                Outcome::NotUnderstood,
                Outcome::NotUnderstood,
                Outcome::Unreachable
            ]
        );
    }

    #[test]
    fn a_port_is_read_from_a_host_and_a_port() {
        assert_eq!(port_of("h.example.test:8080", 443), Some(8080));
        assert_eq!(port_of("h.example.test", 443), Some(443));
        assert_eq!(port_of("[::1]:22", 443), Some(22));
        assert_eq!(port_of("[::1]", 80), Some(80));
        assert_eq!(port_of("h:0", 80), None);
        assert_eq!(port_of("h:x", 80), None);
    }
}
