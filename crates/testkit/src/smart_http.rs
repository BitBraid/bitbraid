//! A smart-HTTP git server for tests, on a port of the loopback interface:
//! it reads each request, checks a Basic login when it was given one, and
//! hands what it lets in to `git http-backend`, as a web server hands a
//! CGI program a request. Plain HTTP, or HTTPS with a certificate from a
//! [`crate::tls::Authority`]. It writes down every request it was sent.
//!
//! One request to a connection: every answer says `Connection: close`.
//!
//! Sources: git-http-backend(1) (`GIT_PROJECT_ROOT`,
//! `GIT_HTTP_EXPORT_ALL`, the CGI variables it reads); RFC 3875 (CGI: the
//! variables, the `Status` line of a script's answer); RFC 9112 (the
//! message head, `Content-Length`, the chunked transfer coding, `Expect:
//! 100-continue`); RFC 7617 (Basic); the documentation of `rustls`
//! (`ServerConnection`, `StreamOwned`).

use std::{
    io::{self, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

use rustls::{ServerConfig, ServerConnection, StreamOwned};

use crate::{
    oracle::git_binary,
    proxy::basic,
    stand_in::{Asked, read_request},
};

/// What the server asks for and how it speaks.
#[derive(Debug, Clone, Default)]
pub struct SmartHttpOptions {
    /// The user name and password it asks for with Basic; none when
    /// absent.
    pub login: Option<(String, String)>,
    /// Speak TLS with this configuration; plain HTTP when absent.
    pub tls: Option<Arc<ServerConfig>>,
}

/// One request the server was sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Served {
    pub method: String,
    /// Its target, as sent.
    pub target: String,
    /// Its `Host` line.
    pub host: Option<String>,
    /// Its `Authorization` line, as sent.
    pub authorization: Option<String>,
    /// Whether it was let in.
    pub let_in: bool,
}

/// A running server. It stops when dropped.
#[derive(Debug)]
pub struct SmartHttp {
    port: u16,
    secure: bool,
    stop: Arc<AtomicBool>,
    served: Arc<Mutex<Vec<Served>>>,
    thread: Option<JoinHandle<()>>,
}

/// Where git keeps its own programs, when it has `http-backend` there.
fn backend_dir() -> Option<PathBuf> {
    let out = Command::new(git_binary())
        .arg("--exec-path")
        .stdin(Stdio::null())
        .output()
        .ok()?;
    let dir = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim());
    let backend = if cfg!(windows) {
        "git-http-backend.exe"
    } else {
        "git-http-backend"
    };
    dir.join(backend).is_file().then_some(dir)
}

impl SmartHttp {
    /// Serve the repositories under `root`. `Ok(None)` when git has no
    /// `http-backend` here.
    pub fn start(root: &Path, options: SmartHttpOptions) -> io::Result<Option<Self>> {
        if backend_dir().is_none() {
            return Ok(None);
        }
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let stop = Arc::new(AtomicBool::new(false));
        let served = Arc::new(Mutex::new(Vec::new()));
        let secure = options.tls.is_some();
        let thread = {
            let (stop, served, root) = (stop.clone(), served.clone(), root.to_path_buf());
            let options = Arc::new(options);
            std::thread::Builder::new()
                .name("test-smart-http".to_owned())
                .spawn(move || {
                    for stream in listener.incoming() {
                        if stop.load(Ordering::SeqCst) {
                            break;
                        }
                        let Ok(stream) = stream else { continue };
                        let (served, root, options) =
                            (served.clone(), root.clone(), options.clone());
                        // A request that cannot be read is the client's to
                        // notice; the next one is served all the same.
                        let _ = std::thread::Builder::new()
                            .name("test-smart-http-request".to_owned())
                            .spawn(move || {
                                let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
                                let _ = match &options.tls {
                                    Some(config) => {
                                        let Ok(connection) = ServerConnection::new(config.clone())
                                        else {
                                            return;
                                        };
                                        let mut secured = StreamOwned::new(connection, stream);
                                        let done = answer(&mut secured, &root, &options, &served);
                                        secured.conn.send_close_notify();
                                        let _ = secured.flush();
                                        done
                                    }
                                    None => {
                                        let mut stream = stream;
                                        answer(&mut stream, &root, &options, &served)
                                    }
                                };
                            });
                    }
                })?
        };
        Ok(Some(Self {
            port,
            secure,
            stop,
            served,
            thread: Some(thread),
        }))
    }

    /// The port it listens on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The address of the repository `name` (a folder under the root), at
    /// `host`: `http[s]://<host>:<port>/<name>`. The host is only a name
    /// in the address; the server is on the loopback interface, and a
    /// name other than an address of it is reached through a
    /// [`crate::proxy::Proxy`].
    pub fn url(&self, host: &str, name: &str) -> String {
        let scheme = if self.secure { "https" } else { "http" };
        format!("{scheme}://{host}:{}/{name}", self.port)
    }

    /// Every request sent so far, in order.
    pub fn served(&self) -> Vec<Served> {
        match self.served.lock() {
            Ok(served) => served.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }
}

impl Drop for SmartHttp {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the thread out of its wait for a connection.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Read one request off `stream` and answer it.
fn answer<S: Read + Write>(
    stream: &mut S,
    root: &Path,
    options: &SmartHttpOptions,
    served: &Mutex<Vec<Served>>,
) -> io::Result<()> {
    let mut reader = BufReader::new(stream);
    let Some(Asked {
        method,
        target,
        headers,
        body,
    }) = read_request(&mut reader)?
    else {
        return Ok(()); // the wake-up call, or nothing a request starts with
    };
    let header = |name: &str| {
        headers
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.clone())
    };
    let stream = reader.into_inner();

    let authorization = header("authorization");
    let let_in = match &options.login {
        None => true,
        Some((user, password)) => authorization.as_deref() == Some(&basic(user, password)),
    };
    let noted = Served {
        method: method.clone(),
        target: target.clone(),
        host: header("host"),
        authorization,
        let_in,
    };
    match served.lock() {
        Ok(mut served) => served.push(noted),
        Err(poisoned) => poisoned.into_inner().push(noted),
    }
    if !let_in {
        return stream.write_all(
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"bitbraid test\"\r\n\
              Content-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }

    // Let in: git's own program does the rest, as a CGI program does.
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    let mut cgi = Command::new(git_binary());
    cgi.arg("http-backend")
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        // Pushes are taken without a login too: http-backend takes them
        // from a user who signed in only, unless told otherwise.
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.receivepack")
        .env("GIT_CONFIG_VALUE_0", "true")
        .env("REQUEST_METHOD", &method)
        .env("PATH_INFO", path)
        .env("QUERY_STRING", query)
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("CONTENT_TYPE", header("content-type").unwrap_or_default())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some((user, _)) = &options.login {
        cgi.env("REMOTE_USER", user);
    }
    if let Some(encoding) = header("content-encoding") {
        cgi.env("HTTP_CONTENT_ENCODING", encoding);
    }
    if let Some(protocol) = header("git-protocol") {
        cgi.env("GIT_PROTOCOL", protocol);
    }
    let mut child = cgi.spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| io::Error::other("no input to git http-backend"))?;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&body);
    });
    let out = child.wait_with_output()?;
    let _ = writer.join();
    let split = out
        .stdout
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|at| (at, at.saturating_add(4)))
        .or_else(|| {
            out.stdout
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|at| (at, at.saturating_add(2)))
        });
    let Some((head_end, body_start)) = split else {
        return stream.write_all(
            b"HTTP/1.1 500 No Answer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    };
    let head = String::from_utf8_lossy(out.stdout.get(..head_end).unwrap_or_default());
    let payload = out.stdout.get(body_start..).unwrap_or_default();
    let mut status = "200 OK".to_owned();
    let mut lines = String::new();
    for line in head.lines() {
        match line.split_once(':') {
            Some((name, value)) if name.eq_ignore_ascii_case("status") => {
                status = value.trim().to_owned();
            }
            _ => {
                lines.push_str(line.trim_end());
                lines.push_str("\r\n");
            }
        }
    }
    stream.write_all(
        format!(
            "HTTP/1.1 {status}\r\n{lines}Content-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        )
        .as_bytes(),
    )?;
    stream.write_all(payload)?;
    stream.flush()
}
