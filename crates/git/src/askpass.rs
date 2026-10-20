//! Answering git's questions without a terminal.
//!
//! Git and ssh ask for user names, passwords and passphrases through an
//! "askpass" program: they run it with the question as its argument and
//! read the answer from its output. This module is both ends of ours. The
//! application starts a [`Server`] and names its own executable as the
//! askpass program of the git processes it runs; started that way (see
//! [`client_main`]) the executable hands the question to the server and
//! prints what comes back.
//!
//! Ours is the last resort. Git looks at `GIT_ASKPASS` and `core.askPass`
//! first, and those are left alone; `SSH_ASKPASS`, which git falls back to
//! and ssh uses, is only set when the user has none. Credential helpers and
//! the ssh agent come before any of it.
//!
//! Answers are kept in memory for as long as it takes to hand them over.
//! They are never stored and never logged.
//!
//! Where the server listens depends on the system. On Unix it is a socket
//! in a new directory only this user can enter. On Windows it is a port of
//! the loopback interface, which only programs on this machine can reach
//! and which the firewall asks nothing about. Either way the askpass
//! program brings a token along: a random number the server makes when it
//! starts and that only its children are given. On Windows, where any
//! program on the machine can connect, the token is what keeps the others
//! out. Other systems get no askpass program, and a git that has to ask
//! fails with its own message.
//!
//! Sources: gitcredentials(7), git-config(1) (`core.askPass`), ssh(1)
//! (`SSH_ASKPASS`, `SSH_ASKPASS_REQUIRE`, `SSH_ASKPASS_PROMPT`), unix(7);
//! the documentation of `std::net::TcpListener` and of the `getrandom`
//! crate.

use std::{
    ffi::OsString,
    path::PathBuf,
    sync::{RwLock, mpsc},
};

/// Tells the askpass program where the server is: the socket's path on
/// Unix, `127.0.0.1:<port>` on Windows.
pub const SOCKET_ENV: &str = "BITBRAID_ASKPASS_SOCKET";
/// Proves to the server that the asker is a child of the application.
pub const TOKEN_ENV: &str = "BITBRAID_ASKPASS_TOKEN";

/// Longest question taken, in bytes; the rest is cut.
const MAX_PROMPT: usize = 4096;
/// Longest answer handed back, in bytes.
const MAX_ANSWER: usize = 4096;

/// What kind of answer a question wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A password or passphrase: typed without being shown.
    Secret,
    /// Something that may be shown: a user name, "yes" to a host key.
    Text,
    /// Yes or no, with nothing to type.
    Confirm,
}

impl Kind {
    /// Tell from what ssh says about the prompt (`SSH_ASKPASS_PROMPT`) and
    /// from the question itself. When in doubt it is a secret.
    pub fn of(hint: &str, prompt: &str) -> Kind {
        let lower = prompt.trim_start().to_lowercase();
        if hint == "confirm" || hint == "none" {
            Kind::Confirm
        } else if lower.starts_with("username") || lower.contains("(yes/no") {
            Kind::Text
        } else {
            Kind::Secret
        }
    }
}

/// A question waiting for its answer.
#[derive(Debug)]
pub struct Question {
    /// Identifies the question in a later [`Event::Gone`].
    pub id: u64,
    /// The question as git or ssh worded it, on one line.
    pub prompt: String,
    /// What kind of answer it wants.
    pub kind: Kind,
    answer: mpsc::Sender<Option<String>>,
}

impl Question {
    /// A question with nobody behind it but whoever holds the other end
    /// that comes with it: what is answered arrives there. For whatever
    /// shows questions to be shown one without a server and a process
    /// that asks.
    pub fn standalone(
        id: u64,
        prompt: impl Into<String>,
        kind: Kind,
    ) -> (Self, mpsc::Receiver<Option<String>>) {
        let (answer, answered) = mpsc::channel();
        (
            Self {
                id,
                prompt: prompt.into(),
                kind,
                answer,
            },
            answered,
        )
    }

    /// Answer, or decline with `None`. Whoever asked may be gone by now;
    /// that is not an error.
    pub fn answer(self, text: Option<String>) {
        let _ = self.answer.send(text);
    }
}

/// What the server tells the application.
#[derive(Debug)]
pub enum Event {
    /// Somebody asks.
    Asked(Question),
    /// The process that asked this question ended before it was answered.
    Gone(u64),
}

/// What git children need to find the server.
struct Active {
    program: PathBuf,
    /// Where the server is, as [`SOCKET_ENV`] tells it.
    address: OsString,
    token: String,
}

static ACTIVE: RwLock<Option<Active>> = RwLock::new(None);

/// Environment for a git child: where the server is and, when the user has
/// no askpass program for ssh, ours. Empty when no server runs.
pub fn child_env() -> Vec<(&'static str, OsString)> {
    let Ok(active) = ACTIVE.read() else {
        return Vec::new();
    };
    let Some(active) = active.as_ref() else {
        return Vec::new();
    };
    let mut env = vec![
        (SOCKET_ENV, active.address.clone()),
        (TOKEN_ENV, OsString::from(&active.token)),
    ];
    if std::env::var_os("SSH_ASKPASS").is_none() {
        env.push(("SSH_ASKPASS", active.program.clone().into_os_string()));
        // Without this ssh only uses the program when it has no terminal.
        env.push(("SSH_ASKPASS_REQUIRE", OsString::from("force")));
    }
    env
}

/// One line for the wire and for a dialog: no line breaks, no control
/// characters, cut at a character boundary.
fn one_line(text: &str, max: usize) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        let ch = if ch.is_control() { ' ' } else { ch };
        if out.len().saturating_add(ch.len_utf8()) > max {
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(unix)]
pub use unix::Server;
#[cfg(windows)]
pub use windows::Server;
#[cfg(any(unix, windows))]
pub use wire::client_main;

#[cfg(not(any(unix, windows)))]
pub use other::{Server, client_main};

#[cfg(not(any(unix, windows)))]
mod other {
    use super::Event;
    use std::path::PathBuf;

    /// No askpass server on this system.
    #[derive(Debug)]
    pub struct Server;

    impl Server {
        /// Always fails: git children get no askpass program.
        pub fn start(
            _program: PathBuf,
            _on_event: impl Fn(Event) + Send + Sync + 'static,
        ) -> crate::Result<Server> {
            Err(crate::Error::Invalid {
                what: say!("askpass-what"),
                detail: say!("askpass-unavailable").to_owned(),
            })
        }
    }

    /// This process is never the askpass program here.
    pub fn client_main() -> Option<i32> {
        None
    }
}

/// What Unix and Windows share: the thread that takes connections, the
/// three lines a question is and the line its answer is, and the askpass
/// program's side of it. Only how the two ends find each other differs.
#[cfg(any(unix, windows))]
mod wire {
    use std::{
        ffi::{OsStr, OsString},
        io::{self, Read, Write},
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicU64, Ordering},
            mpsc,
        },
        time::Duration,
    };

    #[cfg(unix)]
    use super::unix::connect;
    #[cfg(windows)]
    use super::windows::connect;
    use super::{
        ACTIVE, Active, Event, Kind, MAX_ANSWER, MAX_PROMPT, Question, SOCKET_ENV, TOKEN_ENV,
        one_line,
    };
    use crate::{Error, Result};

    /// One end of a connection: a Unix socket or a TCP one.
    pub(super) trait Connection: Read + Write + Send + 'static {
        /// How long a read waits for something to read; `None` for ever.
        fn set_read_timeout(&self, wait: Option<Duration>) -> io::Result<()>;
        /// Whether a read with nothing to read returns at once.
        fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()>;
    }

    /// Where the server takes its connections from.
    pub(super) trait Listener: Send + 'static {
        type Connection: Connection;
        /// The next connection, or `WouldBlock` when none is waiting.
        fn accept(&self) -> io::Result<Self::Connection>;
    }

    pub(super) fn failed(context: &'static str, err: io::Error) -> Error {
        Error::Git {
            context,
            source: Box::new(err),
        }
    }

    /// Sixteen bytes from the system's random source, as hexadecimal.
    pub(super) fn random_hex() -> Result<String> {
        let mut bytes = [0u8; 16];
        #[cfg(unix)]
        std::fs::File::open("/dev/urandom")
            .and_then(|mut file| file.read_exact(&mut bytes))
            .map_err(|err| failed(say!("askpass-reading-random"), err))?;
        #[cfg(windows)]
        getrandom::fill(&mut bytes).map_err(|err| {
            failed(
                say!("askpass-reading-random"),
                io::Error::other(err.to_string()),
            )
        })?;
        Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    /// What starting a server comes to once it listens, on either system:
    /// a thread of its own takes the questions from `listener`, and git
    /// children are told to ask at `address` with `token`. Returns the flag
    /// that stops the thread.
    pub(super) fn run(
        listener: impl Listener,
        address: OsString,
        token: String,
        program: PathBuf,
        on_event: impl Fn(Event) + Send + Sync + 'static,
    ) -> Result<Arc<AtomicBool>> {
        let stop = Arc::new(AtomicBool::new(false));
        let on_event: Arc<dyn Fn(Event) + Send + Sync> = Arc::new(on_event);
        let next_id = Arc::new(AtomicU64::new(1));
        {
            let (stop, token) = (stop.clone(), token.clone());
            std::thread::Builder::new()
                .name("bitbraid-askpass".to_owned())
                .spawn(move || accept_loop(&listener, &stop, &token, &on_event, &next_id))
                .map_err(|err| failed(say!("askpass-starting-thread"), err))?;
        }
        if let Ok(mut active) = ACTIVE.write() {
            *active = Some(Active {
                program,
                address,
                token,
            });
        }
        Ok(stop)
    }

    /// What stopping a server comes to on either system: its thread stops,
    /// and git children are no longer told about it.
    pub(super) fn end(stop: &AtomicBool, address: &OsStr) {
        stop.store(true, Ordering::SeqCst);
        if let Ok(mut active) = ACTIVE.write()
            && active
                .as_ref()
                .is_some_and(|active| active.address == address)
        {
            *active = None;
        }
    }

    fn accept_loop<L: Listener>(
        listener: &L,
        stop: &Arc<AtomicBool>,
        token: &str,
        on_event: &Arc<dyn Fn(Event) + Send + Sync>,
        next_id: &Arc<AtomicU64>,
    ) {
        while !stop.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok(stream) => {
                    let (token, on_event, stop) =
                        (token.to_owned(), on_event.clone(), stop.clone());
                    let id = next_id.fetch_add(1, Ordering::SeqCst);
                    // Each asker waits for a person; they must not wait for
                    // each other.
                    let _ = std::thread::Builder::new()
                        .name("bitbraid-askpass-question".to_owned())
                        .spawn(move || serve(stream, id, &token, &on_event, &stop));
                }
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(200)),
            }
        }
    }

    /// Read the three lines of a request: token, ssh's hint, question.
    /// Bounded in size and in time, whoever is on the other end.
    fn read_request(stream: &mut impl Connection) -> Option<(String, String, String)> {
        stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
        let limit = MAX_PROMPT.saturating_mul(4).saturating_add(512);
        let mut bytes = Vec::new();
        let mut buffer = [0u8; 1024];
        while bytes.iter().filter(|byte| **byte == b'\n').count() < 3 {
            let read = stream.read(&mut buffer).ok()?;
            if read == 0 || bytes.len().saturating_add(read) > limit {
                return None;
            }
            bytes.extend(buffer.iter().take(read));
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut lines = text.split('\n');
        let token = lines.next()?.to_owned();
        let hint = lines.next()?.to_owned();
        let prompt = one_line(lines.next()?, MAX_PROMPT);
        Some((token, hint, prompt))
    }

    fn serve<C: Connection>(
        mut stream: C,
        id: u64,
        token: &str,
        on_event: &Arc<dyn Fn(Event) + Send + Sync>,
        stop: &Arc<AtomicBool>,
    ) {
        // The socket was inherited non-blocking from the listener on some
        // systems.
        let _ = stream.set_nonblocking(false);
        let decline = |stream: &mut C| {
            let _ = stream.write_all(b"n\n");
        };
        let Some((given, hint, prompt)) = read_request(&mut stream) else {
            return decline(&mut stream);
        };
        // Same length always, so comparing takes the same time.
        let same = given.len() == token.len()
            && given
                .bytes()
                .zip(token.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0;
        if !same {
            return decline(&mut stream);
        }

        let (answer, answered) = mpsc::channel();
        on_event(Event::Asked(Question {
            id,
            kind: Kind::of(&hint, &prompt),
            prompt,
            answer,
        }));
        // Wait for the person, and notice when the asker goes away (the
        // operation was stopped, ssh gave up).
        let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
        let reply = loop {
            match answered.recv_timeout(Duration::from_millis(50)) {
                Ok(reply) => break reply,
                Err(mpsc::RecvTimeoutError::Disconnected) => break None,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            let mut probe = [0u8; 1];
            let gone = match stream.read(&mut probe) {
                Ok(read) => read == 0,
                // TCP may say so when the asker was killed.
                Err(err) => matches!(
                    err.kind(),
                    io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
                ),
            };
            if gone || stop.load(Ordering::SeqCst) {
                on_event(Event::Gone(id));
                return;
            }
        };
        match reply {
            Some(text) => {
                let mut line = String::from("y");
                line.push_str(&one_line(&text, MAX_ANSWER));
                line.push('\n');
                let _ = stream.write_all(line.as_bytes());
            }
            None => decline(&mut stream),
        }
    }

    /// When this process was started as the askpass program, do that job:
    /// hand the question (the first argument) to the server, print the
    /// answer, and return the exit code to end with. `None` means this is an
    /// ordinary start. Call it before anything else in `main`.
    pub fn client_main() -> Option<i32> {
        let address = std::env::var_os(SOCKET_ENV)?;
        let token = std::env::var(TOKEN_ENV).ok()?;
        // Only when run the way an askpass program is: with a question.
        let prompt = std::env::args_os().nth(1)?;
        let hint = std::env::var("SSH_ASKPASS_PROMPT").unwrap_or_default();
        let Some(answer) = ask(&address, &token, &hint, &prompt.to_string_lossy()) else {
            return Some(1);
        };
        // To whatever the asker reads from, a pipe: no console is needed,
        // and a pipe nobody reads any more is a failure, not a crash.
        let mut out = io::stdout().lock();
        let printed = writeln!(out, "{answer}").and_then(|()| out.flush());
        Some(if printed.is_ok() { 0 } else { 1 })
    }

    /// The client's side of one question.
    fn ask(address: &OsStr, token: &str, hint: &str, prompt: &str) -> Option<String> {
        let mut stream = connect(address).ok()?;
        let request = format!(
            "{}\n{}\n{}\n",
            one_line(token, 256),
            one_line(hint, 64),
            one_line(prompt, MAX_PROMPT)
        );
        stream.write_all(request.as_bytes()).ok()?;
        let mut reply = Vec::new();
        stream
            .take(u64::try_from(MAX_ANSWER).unwrap_or(4096).saturating_add(16))
            .read_to_end(&mut reply)
            .ok()?;
        let reply = String::from_utf8_lossy(&reply);
        let line = reply.lines().next()?;
        line.strip_prefix('y').map(str::to_owned)
    }

    #[cfg(test)]
    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    mod tests {
        use super::super::{Server, child_env};
        use super::*;
        use std::sync::Mutex;

        /// One server at a time: the place git children look is global.
        static ONE: Mutex<()> = Mutex::new(());

        fn answering(
            reply: impl Fn(&Question) -> Option<Option<String>> + Send + Sync + 'static,
        ) -> (Server, Arc<Mutex<Vec<String>>>) {
            let seen = Arc::new(Mutex::new(Vec::new()));
            let log = seen.clone();
            let server = Server::start(PathBuf::from("/nonexistent/askpass"), move |event| {
                match event {
                    Event::Asked(question) => {
                        log.lock()
                            .unwrap()
                            .push(format!("{:?} {}", question.kind, question.prompt));
                        if let Some(text) = reply(&question) {
                            question.answer(text);
                        } else {
                            // Never answered: keep the question alive.
                            std::mem::forget(question);
                        }
                    }
                    Event::Gone(id) => log.lock().unwrap().push(format!("gone {id}")),
                }
            })
            .unwrap();
            (server, seen)
        }

        fn token() -> String {
            ACTIVE.read().unwrap().as_ref().unwrap().token.clone()
        }

        /// Where the server is, as the askpass program is told.
        fn address() -> OsString {
            ACTIVE.read().unwrap().as_ref().unwrap().address.clone()
        }

        #[test]
        fn a_question_is_answered_declined_or_refused() {
            let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let (_server, seen) = answering(|question| {
                Some(match question.kind {
                    Kind::Text => Some("ada".to_owned()),
                    Kind::Secret if question.prompt.contains("nope") => None,
                    Kind::Secret => Some("s3cret with spaces".to_owned()),
                    Kind::Confirm => Some(String::new()),
                })
            });
            let (token, address) = (token(), address());
            let ask = |hint: &str, prompt: &str| ask(&address, &token, hint, prompt);
            assert_eq!(
                ask("", "Username for 'https://example.invalid': ").as_deref(),
                Some("ada")
            );
            assert_eq!(
                ask("", "Password for 'https://ada@example.invalid': ").as_deref(),
                Some("s3cret with spaces")
            );
            assert_eq!(ask("", "Password for nope: "), None, "declined");
            assert_eq!(ask("confirm", "Allow this key?").as_deref(), Some(""));
            // Not one of ours: refused without anybody being asked.
            assert_eq!(
                super::ask(&address, "not the token", "", "Password: "),
                None
            );
            assert_eq!(
                super::ask(&address, &"0".repeat(token.len()), "", "Password: "),
                None
            );
            let seen = seen.lock().unwrap().clone();
            assert_eq!(
                seen,
                [
                    "Text Username for 'https://example.invalid': ",
                    "Secret Password for 'https://ada@example.invalid': ",
                    "Secret Password for nope: ",
                    "Confirm Allow this key?",
                ]
            );
        }

        #[test]
        fn hostile_requests_are_bounded_and_harmless() {
            let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let (_server, seen) = answering(|_| Some(Some("line one\nline two\u{7}".to_owned())));
            let (token, address) = (token(), address());
            // A question with line breaks and far too long: one line, cut.
            let long = format!("first\nsecond\r\n{}", "é".repeat(MAX_PROMPT));
            let answer = ask(&address, &token, "", &long).unwrap();
            // The answer is one line too, whatever was typed.
            assert_eq!(answer, "line one line two ");
            let asked = seen.lock().unwrap().first().cloned().unwrap();
            assert!(asked.starts_with("Secret first second  é"), "{asked}");
            assert!(asked.len() <= MAX_PROMPT + "Secret ".len());

            // Bytes that are not text, nothing at all, and a flood.
            for bytes in [
                &b"\xff\xfe\n\xff\n\xff\n"[..],
                &b""[..],
                &b"no newline at all"[..],
            ] {
                let mut stream = connect(&address).unwrap();
                stream.write_all(bytes).unwrap();
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let mut reply = String::new();
                let _ = stream.read_to_string(&mut reply);
                assert_eq!(reply, "n\n", "{bytes:?}");
            }
            let mut stream = connect(&address).unwrap();
            let flood = vec![b'x'; MAX_PROMPT * 8];
            let _ = stream.write_all(&flood);
            let mut reply = String::new();
            let _ = stream.read_to_string(&mut reply);
            // Over TCP the server's end, closed with most of the flood
            // unread, resets the connection, and Windows then drops what
            // had arrived and was not read yet: the "n" may be lost there.
            assert!(
                reply == "n\n" || (cfg!(windows) && reply.is_empty()),
                "{reply:?}"
            );
            assert_eq!(seen.lock().unwrap().len(), 1, "none of those was asked");
        }

        #[test]
        fn an_asker_that_goes_away_is_reported() {
            let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let (_server, seen) = answering(|_| None);
            let token = token();
            let mut stream = connect(&address()).unwrap();
            stream
                .write_all(format!("{token}\n\nPassphrase: \n").as_bytes())
                .unwrap();
            // Asked, then gone.
            let wait = |want: usize| {
                for _ in 0..200 {
                    if seen.lock().unwrap().len() >= want {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                panic!("waited for {want} events, saw {:?}", seen.lock().unwrap());
            };
            wait(1);
            drop(stream);
            wait(2);
            let seen = seen.lock().unwrap().clone();
            assert_eq!(
                seen.first().map(String::as_str),
                Some("Secret Passphrase: ")
            );
            assert!(seen.get(1).is_some_and(|line| line.starts_with("gone ")));
        }

        #[test]
        fn children_are_told_where_to_ask_only_while_a_server_runs() {
            let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            assert!(child_env().is_empty());
            let (server, _) = answering(|_| Some(None));
            let env = child_env();
            let get = |key: &str| {
                env.iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| value.clone())
            };
            assert_eq!(get(SOCKET_ENV), Some(address()));
            assert!(get(TOKEN_ENV).is_some());
            // On Unix the socket is in a directory nobody else can enter.
            #[cfg(unix)]
            let (path, dir) = {
                use std::os::unix::fs::PermissionsExt as _;
                assert_eq!(
                    get(SOCKET_ENV).as_deref(),
                    Some(server.socket().as_os_str())
                );
                let dir = server.socket().parent().unwrap().to_path_buf();
                let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o700);
                (server.socket().to_path_buf(), dir)
            };
            // On Windows it is a port of the loopback interface.
            #[cfg(windows)]
            {
                let address = get(SOCKET_ENV).unwrap();
                let address: std::net::SocketAddr = address.to_str().unwrap().parse().unwrap();
                assert!(address.ip().is_loopback() && address.port() != 0);
            }
            drop(server);
            assert!(child_env().is_empty());
            #[cfg(unix)]
            {
                assert!(!path.exists());
                assert!(!dir.exists());
            }
        }
    }
}

#[cfg(unix)]
mod unix {
    use std::{
        ffi::OsStr,
        fs, io,
        os::unix::{
            fs::{DirBuilderExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
        path::{Path, PathBuf},
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };

    use super::{
        Event,
        wire::{self, failed},
    };
    use crate::Result;

    impl wire::Connection for UnixStream {
        fn set_read_timeout(&self, wait: Option<Duration>) -> io::Result<()> {
            UnixStream::set_read_timeout(self, wait)
        }

        fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
            UnixStream::set_nonblocking(self, nonblocking)
        }
    }

    impl wire::Listener for UnixListener {
        type Connection = UnixStream;

        fn accept(&self) -> io::Result<UnixStream> {
            UnixListener::accept(self).map(|(stream, _)| stream)
        }
    }

    /// The askpass program's way to the server: the socket at `address`.
    pub(super) fn connect(address: &OsStr) -> io::Result<UnixStream> {
        UnixStream::connect(address)
    }

    /// A new directory only this user can enter, under the runtime
    /// directory or the temporary one. Creating fails when the name is
    /// taken, so nobody can have put something there first.
    fn private_dir(name: &str) -> Result<PathBuf> {
        let roots = [
            std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from),
            Some(std::env::temp_dir()),
            Some(PathBuf::from("/tmp")),
        ];
        let mut last = None;
        for root in roots.into_iter().flatten() {
            sweep(&root);
            let dir = root.join(name);
            // A socket path has to fit in about a hundred bytes.
            if dir.as_os_str().len() > 80 {
                continue;
            }
            match fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => return Ok(dir),
                Err(err) => last = Some(err),
            }
        }
        Err(failed(
            say!("askpass-creating-directory"),
            last.unwrap_or_else(|| io::Error::other(say!("askpass-no-directory"))),
        ))
    }

    /// Remove what earlier runs left behind when they were killed: our
    /// directories (`bitbraid-<pid>-<random>`) whose process is gone. Only
    /// the socket inside is removed and then the directory, which fails if
    /// anything else is in it.
    fn sweep(root: &Path) {
        // Without /proc there is no telling whether a process is alive.
        if !Path::new("/proc/self").exists() {
            return;
        }
        let Ok(entries) = fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid) = name
                .to_str()
                .and_then(|name| name.strip_prefix("bitbraid-"))
                .and_then(|rest| rest.split_once('-'))
                .filter(|(_, random)| {
                    random.len() == 8 && random.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .and_then(|(pid, _)| pid.parse::<u32>().ok())
            else {
                continue;
            };
            if pid == std::process::id() || Path::new("/proc").join(pid.to_string()).exists() {
                continue;
            }
            let dir = entry.path();
            let is_dir = fs::symlink_metadata(&dir).is_ok_and(|meta| meta.is_dir());
            if is_dir {
                let _ = fs::remove_file(dir.join("askpass"));
                let _ = fs::remove_dir(&dir);
            }
        }
    }

    /// The askpass server: a socket in a private directory and a thread
    /// that takes questions from it. Dropping it stops both.
    #[derive(Debug)]
    pub struct Server {
        dir: PathBuf,
        socket: PathBuf,
        stop: Arc<AtomicBool>,
    }

    impl Server {
        /// Start listening. `program` is the executable git children are
        /// told to run as their askpass program (it has to call
        /// [`client_main`](super::client_main) first thing); `on_event`
        /// gets every question, on a thread of the server's.
        pub fn start(
            program: PathBuf,
            on_event: impl Fn(Event) + Send + Sync + 'static,
        ) -> Result<Server> {
            let token = wire::random_hex()?;
            let dir = private_dir(&format!(
                "bitbraid-{}-{}",
                std::process::id(),
                token.get(..8).unwrap_or("0")
            ))?;
            let socket = dir.join("askpass");
            let listener = UnixListener::bind(&socket).map_err(|err| {
                let _ = fs::remove_dir(&dir);
                failed(say!("askpass-binding-socket"), err)
            })?;
            let _ = fs::set_permissions(&socket, fs::Permissions::from_mode(0o600));
            // Wake up now and then to see whether to stop.
            listener
                .set_nonblocking(true)
                .map_err(|err| failed(say!("askpass-preparing-socket"), err))?;
            let address = socket.clone().into_os_string();
            let stop = wire::run(listener, address, token, program, on_event)?;
            Ok(Server { dir, socket, stop })
        }

        /// Where the server listens.
        pub fn socket(&self) -> &Path {
            &self.socket
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            wire::end(&self.stop, self.socket.as_os_str());
            let _ = fs::remove_file(&self.socket);
            let _ = fs::remove_dir(&self.dir);
        }
    }

    #[cfg(test)]
    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    mod tests {
        use super::*;

        #[test]
        fn what_a_killed_run_left_behind_is_swept_and_nothing_else() {
            let root = std::env::temp_dir().join(format!("bitbraid-sweep-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            // No process has this id: above the largest the kernel gives out.
            let stale = root.join("bitbraid-4194999-0123abcd");
            fs::create_dir(&stale).unwrap();
            fs::write(stale.join("askpass"), "").unwrap();
            // Ours by name, but with something else inside: left as it is.
            let kept = root.join("bitbraid-4194998-0123abcd");
            fs::create_dir(&kept).unwrap();
            fs::write(kept.join("notes.txt"), "mine").unwrap();
            // A live process, and names that are not ours.
            let alive = root.join(format!("bitbraid-{}-0123abcd", std::process::id()));
            fs::create_dir(&alive).unwrap();
            let others = [
                "bitbraid-4194999-short",
                "bitbraid-notapid-0123abcd",
                "bitbraid",
                "other-4194999-0123abcd",
            ];
            for name in others {
                fs::create_dir(root.join(name)).unwrap();
            }
            fs::write(root.join("bitbraid-4194997-0123abcd"), "a file").unwrap();

            sweep(&root);
            if Path::new("/proc/self").exists() {
                assert!(!stale.exists());
            }
            assert!(kept.join("notes.txt").exists());
            assert!(alive.exists());
            for name in others {
                assert!(root.join(name).exists(), "{name}");
            }
            assert!(root.join("bitbraid-4194997-0123abcd").exists());
            // A root that is not there is nothing to sweep.
            sweep(&root.join("missing"));
            fs::remove_dir_all(&root).unwrap();
        }
    }
}

#[cfg(windows)]
mod windows {
    use std::{
        ffi::{OsStr, OsString},
        io,
        net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
        path::PathBuf,
        sync::{Arc, atomic::AtomicBool},
        time::Duration,
    };

    use super::{
        Event,
        wire::{self, failed},
    };
    use crate::Result;

    impl wire::Connection for TcpStream {
        fn set_read_timeout(&self, wait: Option<Duration>) -> io::Result<()> {
            TcpStream::set_read_timeout(self, wait)
        }

        fn set_nonblocking(&self, nonblocking: bool) -> io::Result<()> {
            TcpStream::set_nonblocking(self, nonblocking)
        }
    }

    impl wire::Listener for TcpListener {
        type Connection = TcpStream;

        fn accept(&self) -> io::Result<TcpStream> {
            TcpListener::accept(self).map(|(stream, _)| stream)
        }
    }

    /// The askpass program's way to the server: the port that `address`
    /// names, and only ever one of this machine's loopback interface,
    /// whatever the variable says.
    pub(super) fn connect(address: &OsStr) -> io::Result<TcpStream> {
        let address = address
            .to_str()
            .and_then(|text| text.parse::<SocketAddr>().ok())
            .filter(|address| address.ip().is_loopback())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, say!("askpass-not-local"))
            })?;
        TcpStream::connect(address)
    }

    /// The askpass server: a port of the loopback interface and a thread
    /// that takes questions from it. Dropping it stops both.
    #[derive(Debug)]
    pub struct Server {
        address: OsString,
        stop: Arc<AtomicBool>,
    }

    impl Server {
        /// Start listening. `program` is the executable git children are
        /// told to run as their askpass program (it has to call
        /// [`client_main`](super::client_main) first thing); `on_event`
        /// gets every question, on a thread of the server's.
        pub fn start(
            program: PathBuf,
            on_event: impl Fn(Event) + Send + Sync + 'static,
        ) -> Result<Server> {
            let token = wire::random_hex()?;
            // A free port, chosen by the system.
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
                .map_err(|err| failed(say!("askpass-listening"), err))?;
            let port = listener
                .local_addr()
                .map_err(|err| failed(say!("askpass-listening"), err))?
                .port();
            // Wake up now and then to see whether to stop.
            listener
                .set_nonblocking(true)
                .map_err(|err| failed(say!("askpass-preparing-port"), err))?;
            let address = OsString::from(format!("127.0.0.1:{port}"));
            let stop = wire::run(listener, address.clone(), token, program, on_event)?;
            Ok(Server { address, stop })
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            // The thread stops within a moment, and the port goes with it.
            wire::end(&self.stop, &self.address);
        }
    }

    #[cfg(test)]
    #[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    mod tests {
        use super::*;

        #[test]
        fn the_askpass_program_only_ever_connects_to_this_machine() {
            for address in [
                "10.0.0.1:80",
                "0.0.0.0:80",
                "example.invalid:80",
                "localhost",
                "/nonexistent/askpass",
                "",
            ] {
                let err = connect(OsStr::new(address)).unwrap_err();
                assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{address}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Kind, one_line};

    #[test]
    fn kinds_are_told_from_hint_and_wording() {
        assert_eq!(Kind::of("", "Username for 'https://host': "), Kind::Text);
        assert_eq!(
            Kind::of("", "Password for 'https://u@host': "),
            Kind::Secret
        );
        assert_eq!(
            Kind::of("", "Enter passphrase for key '/home/u/.ssh/id': "),
            Kind::Secret
        );
        assert_eq!(
            Kind::of(
                "",
                "Are you sure you want to continue connecting (yes/no/[fingerprint])? "
            ),
            Kind::Text
        );
        assert_eq!(Kind::of("confirm", "Allow use of key?"), Kind::Confirm);
        assert_eq!(Kind::of("none", "Touch your security key"), Kind::Confirm);
        // Unknown wording is treated as a secret, which shows nothing.
        assert_eq!(Kind::of("", "PIN: "), Kind::Secret);
        assert_eq!(Kind::of("", ""), Kind::Secret);
    }

    #[test]
    fn one_line_cuts_at_characters_and_drops_control_characters() {
        assert_eq!(one_line("a\nb\tc\u{0}", 100), "a b c ");
        assert_eq!(one_line("ééé", 5), "éé");
        assert_eq!(one_line("", 5), "");
        assert_eq!(one_line("abc", 0), "");
    }
}
