//! One running application per configuration directory: a link or a path
//! the application is started with is handed to the instance that is
//! already running, which opens it in a tab, instead of a second window.
//!
//! The running instance listens on a socket in the user's runtime
//! directory. Its name carries a hash of the configuration directory, so
//! that instances started with different configuration directories (the
//! tests, the off-screen tooling, a second profile of the user's) never
//! meet. A start with something to open connects, sends it as one line of
//! JSON, and ends when the other side says it took it.
//!
//! Only this user can reach the socket (the runtime directory is theirs),
//! and what arrives is treated as what the command line is: a link goes
//! through the same questions as one the application was started with.
//!
//! On Windows the instance listens on a port of the loopback interface
//! instead, and a small file in the configuration directory,
//! `instance.port`, names the port and a random token. Any program on the
//! machine can reach the port, but only this user can read the
//! configuration directory: a start reads the file, connects, and sends
//! the token before its line, and a connection that does not begin with
//! the token is turned away. A file whose port nobody of ours answers on
//! was left by a run that was killed, and the next instance writes its own
//! over it; a file that is not one of ours is never trusted and never
//! replaced.
//!
//! On macOS the system itself hands the running application what it is to
//! open, a link clicked anywhere, a folder dropped on its icon or opened
//! with it, as addresses ([`opened_by_system`]); they arrive where what
//! later starts hand over does, and are taken the same way, also when
//! they come before there is a window to take them.
//!
//! Sources: unix(7) for socket files and their length limit; the XDG Base
//! Directory specification for `XDG_RUNTIME_DIR`; the documentation of
//! `std::net::TcpListener` and of the `getrandom` crate; RFC 8089 (the
//! `file` URI scheme) and RFC 3986 (percent-encoding); gpui's
//! `Application::on_open_urls`.

use std::{
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
};

/// What the system asked to open, and the way to the window once there is
/// one.
struct FromSystem {
    sender: Option<futures::channel::mpsc::UnboundedSender<String>>,
    waiting: Vec<String>,
}

static FROM_SYSTEM: Mutex<FromSystem> = Mutex::new(FromSystem {
    sender: None,
    waiting: Vec::new(),
});

/// The most that waits for the window.
const MOST_WAITING: usize = 64;

/// Hand what the system asked the application to open to the window: now,
/// or once it is there. Addresses that are neither a link of ours nor a
/// local file are passed over.
pub fn opened_by_system(urls: Vec<String>) {
    let mut queue = FROM_SYSTEM.lock().unwrap_or_else(PoisonError::into_inner);
    for open in urls.iter().filter_map(|url| from_system_url(url)) {
        let sent = queue
            .sender
            .as_ref()
            .is_some_and(|sender| sender.unbounded_send(open.clone()).is_ok());
        if !sent && queue.waiting.len() < MOST_WAITING {
            queue.waiting.push(open);
        }
    }
}

/// What to open of an address the system gave: a link of ours as it is;
/// the path of a local file (`file:///…`, `file://localhost/…`), its
/// percent-escapes undone and without a slash at its end.
pub fn from_system_url(url: &str) -> Option<String> {
    if url.len() > LONGEST || url.chars().any(char::is_control) {
        return None;
    }
    if is_link(url) {
        return Some(url.to_owned());
    }
    let scheme = url.get(..7)?;
    if !scheme.eq_ignore_ascii_case("file://") {
        return None;
    }
    let rest = url.get(7..)?;
    let path = match rest.get(..9) {
        Some(host) if host.eq_ignore_ascii_case("localhost") => rest.get(9..)?,
        _ => rest,
    };
    if !path.starts_with('/') {
        return None;
    }
    let decoded = percent_decoded(path)?;
    let trimmed = decoded.trim_end_matches('/');
    let path = if trimmed.is_empty() { "/" } else { trimmed };
    (!path.chars().any(char::is_control)).then(|| path.to_owned())
}

/// `text` with its `%XX` escapes undone, when it is UTF-8 after that.
fn percent_decoded(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0usize;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'%' {
            let hex = text.get(at.checked_add(1)?..at.checked_add(3)?)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at = at.checked_add(3)?;
        } else {
            out.push(byte);
            at = at.checked_add(1)?;
        }
    }
    String::from_utf8(out).ok()
}

/// What other starts of the application hand over to this one, a GPUI
/// global: the window takes the receiving end when it is built.
pub struct Incoming {
    /// Links and paths as they arrive.
    pub events: Option<futures::channel::mpsc::UnboundedReceiver<String>>,
    /// Keeps the socket (on Windows, the file that names the port) for as
    /// long as the application runs.
    _listening: Option<Listening>,
}

impl gpui_kit::Global for Incoming {}

impl std::fmt::Debug for Incoming {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // not shown: the type's name, for developers
        f.debug_struct("Incoming")
            .field("listening", &self._listening.is_some())
            .finish()
    }
}

/// Start listening for what later starts hand over. Without a place for
/// the socket, or with another instance there already, nothing listens
/// and every start is its own.
pub fn init(cx: &mut gpui_kit::App) {
    let (tx, rx) = futures::channel::mpsc::unbounded::<String>();
    // What the system asked to open goes the same way, what came already
    // first.
    {
        let mut queue = FROM_SYSTEM.lock().unwrap_or_else(PoisonError::into_inner);
        for open in queue.waiting.drain(..) {
            let _ = tx.unbounded_send(open);
        }
        queue.sender = Some(tx.clone());
    }
    let listening = own_socket().and_then(|socket| {
        listen(&socket, move |open| {
            let _ = tx.unbounded_send(open);
        })
    });
    // The socket file goes when the application is quit, with the
    // listening end; one left by a run that was killed is replaced by the
    // next.
    if listening.is_some() {
        cx.on_app_quit(|cx| {
            if cx.try_global::<Incoming>().is_some() {
                cx.global_mut::<Incoming>()._listening = None;
            }
            async {}
        })
        .detach();
    }
    cx.set_global(Incoming {
        events: Some(rx),
        _listening: listening,
    });
}

/// The longest message that is taken: a link or a path, with room.
const LONGEST: usize = 16 * 1024;

/// The name of the file in the configuration directory that says, on
/// Windows, where the instance listens.
const PORT_FILE: &str = "instance.port";

/// Where the instance for a configuration directory listens on Unix: in
/// the runtime directory when there is one, else in the configuration
/// directory itself.
pub fn socket_path(runtime_dir: Option<&Path>, config_dir: &Path) -> PathBuf {
    match runtime_dir.filter(|dir| dir.is_absolute() && dir.is_dir()) {
        Some(dir) => dir.join(format!(
            "bitbraid-{:016x}.sock",
            fnv1a64(config_dir.to_string_lossy().as_bytes())
        )),
        None => config_dir.join("instance.sock"),
    }
}

/// The socket of this run: for the configuration directory in use. On
/// Windows, the file in the configuration directory that names the port.
pub fn own_socket() -> Option<PathBuf> {
    let config = crate::prefs::config_dir()?;
    if cfg!(windows) {
        return Some(config.join(PORT_FILE));
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    Some(socket_path(runtime.as_deref(), &config))
}

/// FNV-1a, 64 bits: a name for a path, not a secret.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// What is sent: one line.
fn encode(open: &str) -> Option<String> {
    let line = serde_json::to_string(&serde_json::json!({ "open": open })).ok()?;
    (line.len() <= LONGEST).then(|| format!("{line}\n"))
}

/// What a line that arrived asks to be opened, when it is one of ours.
fn decode(line: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(line.trim()).ok()?;
    let open = value.get("open")?.as_str()?;
    (!open.trim().is_empty() && !open.chars().any(char::is_control)).then(|| open.to_owned())
}

/// What Unix and Windows share: how long either side waits, what one
/// connection carries each way, and the threads that listen. Only how a
/// start finds the instance differs.
#[cfg(any(unix, windows))]
mod wire {
    use std::{
        io::{self, BufRead, BufReader, Read, Write},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use super::{LONGEST, decode};

    /// How long either side waits for the other.
    pub(super) const PATIENCE: Duration = Duration::from_secs(2);

    /// How many connections are attended to at once; more than that are
    /// turned away, and a start turned away opens a window of its own.
    const AT_ONCE: usize = 16;

    /// The longest first line where a token is wanted: the token, with
    /// room.
    const TOKEN_LINE: usize = 128;

    /// One end of a connection: a Unix socket or a TCP one.
    pub(super) trait Connection: Read + Write + Send + 'static {
        /// Wait no longer than [`PATIENCE`] for the other side, reading or
        /// writing.
        fn be_patient(&self);
    }

    /// Where connections come from.
    pub(super) trait Listener: Send + 'static {
        type Connection: Connection;
        /// The next connection, once there is one.
        fn accept(&self) -> io::Result<Self::Connection>;
    }

    /// Send `message` over a connection that was just made, and say
    /// whether the other side took it.
    pub(super) fn exchange(mut stream: impl Connection, message: &str) -> bool {
        stream.be_patient();
        if stream.write_all(message.as_bytes()).is_err() {
            return false;
        }
        let mut answer = String::new();
        let _ = BufReader::new(stream.take(64)).read_line(&mut answer);
        answer.trim() == "ok"
    }

    /// One line of at most `longest` bytes, its end included, as text.
    fn line(reader: &mut impl BufRead, longest: usize) -> Option<String> {
        let mut bytes = Vec::new();
        let read = reader
            .by_ref()
            .take(longest as u64 + 1)
            .read_until(b'\n', &mut bytes);
        let taken = read.is_ok() && bytes.len() <= longest && bytes.last() == Some(&b'\n');
        taken.then(|| String::from_utf8(bytes).ok()).flatten()
    }

    /// Whether `given` is `token`, in the same time however much of it is
    /// right.
    fn same(given: &str, token: &str) -> bool {
        given.len() == token.len()
            && given
                .bytes()
                .zip(token.bytes())
                .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                == 0
    }

    /// Read what one connection sends and answer it. Where a `token` is
    /// wanted (Windows) the first line has to be the token, and an empty
    /// line after it only asks whether anybody is there.
    fn attend(
        mut stream: impl Connection,
        token: Option<&str>,
        on_open: &(dyn Fn(String) + Send + Sync),
    ) {
        stream.be_patient();
        let message = {
            let mut reader = BufReader::new(&mut stream);
            let known = token.is_none_or(|token| {
                line(&mut reader, TOKEN_LINE).is_some_and(|given| {
                    given
                        .strip_suffix('\n')
                        .is_some_and(|given| same(given, token))
                })
            });
            // One line, of a length that can be a link or a path; more
            // than that is not from us. Read after a wrong token too, so
            // that the start hears "no" rather than a connection cut short.
            let message = line(&mut reader, LONGEST);
            known.then_some(message).flatten()
        };
        let open = match message.as_deref() {
            Some("\n") if token.is_some() => {
                let _ = stream.write_all(b"ok\n");
                return;
            }
            Some(line) => decode(line),
            None => None,
        };
        match open {
            Some(open) => {
                let _ = stream.write_all(b"ok\n");
                on_open(open);
            }
            None => {
                let _ = stream.write_all(b"no\n");
            }
        }
    }

    /// Take connections from `listener` on a thread of its own and pass
    /// what arrives to `on_open`. Each is attended to on a thread of its
    /// own, so that one that connects and says nothing holds nobody else
    /// up.
    pub(super) fn serve(
        listener: impl Listener,
        token: Option<String>,
        on_open: impl Fn(String) + Send + Sync + 'static,
    ) -> io::Result<()> {
        let on_open: Arc<dyn Fn(String) + Send + Sync> = Arc::new(on_open);
        let busy = Arc::new(AtomicUsize::new(0));
        std::thread::Builder::new()
            .name("bitbraid-instance".to_owned())
            .spawn(move || {
                for stream in std::iter::from_fn(|| Some(listener.accept())) {
                    let Ok(stream) = stream else {
                        continue;
                    };
                    if busy.fetch_add(1, Ordering::SeqCst) >= AT_ONCE {
                        busy.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                    let (on_open, done, token) = (on_open.clone(), busy.clone(), token.clone());
                    let attended = std::thread::Builder::new()
                        .name("bitbraid-instance-connection".to_owned())
                        .spawn(move || {
                            attend(stream, token.as_deref(), on_open.as_ref());
                            done.fetch_sub(1, Ordering::SeqCst);
                        });
                    if attended.is_err() {
                        busy.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            })
            .map(|_| ())
    }
}

#[cfg(unix)]
mod unix {
    use std::{
        io,
        os::unix::net::{UnixListener, UnixStream},
        path::{Path, PathBuf},
    };

    use super::{encode, wire};

    impl wire::Connection for UnixStream {
        fn be_patient(&self) {
            let _ = self.set_read_timeout(Some(wire::PATIENCE));
            let _ = self.set_write_timeout(Some(wire::PATIENCE));
        }
    }

    impl wire::Listener for UnixListener {
        type Connection = UnixStream;

        fn accept(&self) -> io::Result<UnixStream> {
            UnixListener::accept(self).map(|(stream, _)| stream)
        }
    }

    /// Hand something to open to the instance listening at `socket`. True
    /// when it said that it took it; false when nobody is there.
    pub fn hand_over(socket: &Path, open: &str) -> bool {
        let Some(line) = encode(open) else {
            return false;
        };
        let Ok(stream) = UnixStream::connect(socket) else {
            return false;
        };
        wire::exchange(stream, &line)
    }

    /// The listening end. Dropping it takes the socket file away.
    #[derive(Debug)]
    pub struct Listening {
        socket: PathBuf,
    }

    impl Drop for Listening {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.socket);
        }
    }

    /// Listen at `socket` and pass what arrives to `on_open`. None when
    /// another instance is listening there already, or the socket cannot
    /// be made. A socket file nobody listens on (left by a run that was
    /// killed) is replaced.
    pub fn listen(
        socket: &Path,
        on_open: impl Fn(String) + Send + Sync + 'static,
    ) -> Option<Listening> {
        if UnixStream::connect(socket).is_ok() {
            return None;
        }
        // Nobody answers: whatever is there is a leftover. Only a socket is
        // taken away; anything else at that name is somebody else's.
        if let Ok(meta) = std::fs::symlink_metadata(socket) {
            use std::os::unix::fs::FileTypeExt as _;
            if !meta.file_type().is_socket() {
                return None;
            }
            let _ = std::fs::remove_file(socket);
        }
        if let Some(parent) = socket.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let listener = UnixListener::bind(socket).ok()?;
        if wire::serve(listener, None, on_open).is_err() {
            let _ = std::fs::remove_file(socket);
            return None;
        }
        Some(Listening {
            socket: socket.to_path_buf(),
        })
    }
}

#[cfg(unix)]
pub use unix::{Listening, hand_over, listen};

#[cfg(windows)]
mod windows {
    use std::{
        io::{self, Read as _},
        net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
        path::{Path, PathBuf},
    };

    use super::{encode, wire};

    impl wire::Connection for TcpStream {
        fn be_patient(&self) {
            let _ = self.set_read_timeout(Some(wire::PATIENCE));
            let _ = self.set_write_timeout(Some(wire::PATIENCE));
        }
    }

    impl wire::Listener for TcpListener {
        type Connection = TcpStream;

        fn accept(&self) -> io::Result<TcpStream> {
            TcpListener::accept(self).map(|(stream, _)| stream)
        }
    }

    /// The first line of the file, which says that it is one of ours.
    // not shown: the first line of the file, which another start reads
    const HEADER: &str = "bitbraid instance";

    /// What the file says: the port the instance listens on, and the token
    /// it wants before anything else.
    #[cfg_attr(test, derive(Debug, PartialEq, Eq))]
    pub(super) struct Place {
        pub(super) port: u16,
        pub(super) token: String,
    }

    impl Place {
        /// What a file of ours says. None when there is no file, or when it
        /// is not one of ours.
        pub(super) fn read(file: &Path) -> Option<Place> {
            let mut text = String::new();
            std::fs::File::open(file)
                .ok()?
                .take(256)
                .read_to_string(&mut text)
                .ok()?;
            Place::parse(&text)
        }

        /// The three lines [`Place::text`] writes, and nothing else.
        pub(super) fn parse(text: &str) -> Option<Place> {
            let mut lines = text.split('\n');
            let (header, port, token, end) =
                (lines.next()?, lines.next()?, lines.next()?, lines.next()?);
            let port = port.strip_prefix("port ")?.parse::<u16>().ok()?;
            let token = token.strip_prefix("token ")?;
            let ours = header == HEADER
                && port != 0
                && token.len() == 32
                && token
                    .bytes()
                    .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
                && end.is_empty()
                && lines.next().is_none();
            ours.then(|| Place {
                port,
                token: token.to_owned(),
            })
        }

        pub(super) fn text(&self) -> String {
            // not shown: written into the file, for another start to read
            format!("{HEADER}\nport {}\ntoken {}\n", self.port, self.token)
        }

        /// Send `message` to the instance after the token, and say whether
        /// it took it. A message that is nothing but its line end only asks
        /// whether anybody is there.
        fn send(&self, message: &str) -> bool {
            // A port that can be taken has nobody on it, which this says
            // at once; Windows takes two seconds and more to refuse a
            // connection.
            if TcpListener::bind((Ipv4Addr::LOCALHOST, self.port)).is_ok() {
                return false;
            }
            let address = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
            let Ok(stream) = TcpStream::connect_timeout(&address, wire::PATIENCE) else {
                return false;
            };
            wire::exchange(stream, &format!("{}\n{message}", self.token))
        }
    }

    /// Sixteen bytes from the system's random source, as hexadecimal.
    fn random_hex() -> Option<String> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes).ok()?;
        Some(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    /// Hand something to open to the instance that `file` names. True when
    /// it said that it took it; false when there is no such file, when it
    /// is not one of ours, or when nobody of ours answers on its port.
    pub fn hand_over(file: &Path, open: &str) -> bool {
        let Some(line) = encode(open) else {
            return false;
        };
        Place::read(file).is_some_and(|place| place.send(&line))
    }

    /// The listening end. Dropping it takes the file away, as long as the
    /// file still names this instance.
    pub struct Listening {
        file: PathBuf,
        token: String,
    }

    impl std::fmt::Debug for Listening {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            // not shown: the type's name, for developers
            f.debug_struct("Listening")
                .field("file", &self.file)
                .finish_non_exhaustive()
        }
    }

    impl Drop for Listening {
        fn drop(&mut self) {
            if Place::read(&self.file).is_some_and(|place| place.token == self.token) {
                let _ = std::fs::remove_file(&self.file);
            }
        }
    }

    /// Listen on a port of the loopback interface, name it in `file`, and
    /// pass what arrives to `on_open`. None when the instance the file
    /// names answers, when the file is not one of ours, or when the port
    /// or the file cannot be made. A file of ours whose port nobody of ours
    /// answers on (left by a run that was killed) is replaced.
    pub fn listen(
        file: &Path,
        on_open: impl Fn(String) + Send + Sync + 'static,
    ) -> Option<Listening> {
        if std::fs::symlink_metadata(file).is_ok() {
            let there = Place::read(file)?;
            if there.send("\n") {
                return None;
            }
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
        let place = Place {
            port: listener.local_addr().ok()?.port(),
            token: random_hex()?,
        };
        if let Some(parent) = file.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Written beside and moved into place, so that a start never reads
        // half of it.
        let mut beside = file.as_os_str().to_owned();
        beside.push(format!(".{}", std::process::id()));
        let beside = PathBuf::from(beside);
        let written =
            std::fs::write(&beside, place.text()).and_then(|()| std::fs::rename(&beside, file));
        if written.is_err() {
            let _ = std::fs::remove_file(&beside);
            return None;
        }
        let token = place.token.clone();
        if wire::serve(listener, Some(place.token), on_open).is_err() {
            let _ = std::fs::remove_file(file);
            return None;
        }
        Some(Listening {
            file: file.to_path_buf(),
            token,
        })
    }
}

#[cfg(windows)]
pub use windows::{Listening, hand_over, listen};

/// Where nothing can listen every start is an instance of its own.
#[cfg(not(any(unix, windows)))]
pub fn hand_over(_socket: &Path, _open: &str) -> bool {
    false
}

/// The listening end, where there is none.
#[cfg(not(any(unix, windows)))]
#[derive(Debug)]
pub struct Listening;

/// Where nothing can listen nothing listens.
#[cfg(not(any(unix, windows)))]
pub fn listen(
    _socket: &Path,
    _on_open: impl Fn(String) + Send + Sync + 'static,
) -> Option<Listening> {
    None
}

/// What the application was started with, as it is handed over: a link as
/// it is, a path made whole, since the instance that takes it was started
/// somewhere else.
pub fn message_for(link: Option<&str>, path: Option<&Path>) -> Option<String> {
    if let Some(link) = link {
        return Some(link.to_owned());
    }
    let path = path?;
    let whole = bitbraid_core::paths::canonical(path)
        .or_else(|_| std::path::absolute(path))
        .ok()?;
    Some(whole.to_string_lossy().into_owned())
}

/// Whether something that arrived is a link rather than a path.
pub fn is_link(open: &str) -> bool {
    let scheme = bitbraid_engine::link_cmds::SCHEME;
    open.get(..scheme.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
        && open
            .get(scheme.len()..)
            .is_some_and(|rest| rest.starts_with(':'))
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        sync::mpsc,
        time::Duration,
    };

    use super::{LONGEST, decode, encode, hand_over, is_link, listen, message_for, socket_path};

    fn playground(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bitbraid-inst-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn each_configuration_directory_has_a_socket_of_its_own() {
        let dir = playground("paths");
        let (one, two) = (
            Path::new("/home/ada/.config/bitbraid"),
            Path::new("/tmp/scratch/bitbraid"),
        );
        let a = socket_path(Some(&dir), one);
        let b = socket_path(Some(&dir), two);
        assert_ne!(a, b);
        assert_eq!(a, socket_path(Some(&dir), one), "the same every time");
        assert_eq!(a.parent(), Some(dir.as_path()));
        // A socket's path is short: its length is limited to about a hundred bytes.
        assert!(a.file_name().unwrap().len() < 40);
        // No runtime directory, or one that is not there: beside the configuration.
        assert_eq!(socket_path(None, one), one.join("instance.sock"));
        assert_eq!(
            socket_path(Some(Path::new("/definitely/not/there")), one),
            one.join("instance.sock")
        );
        assert_eq!(
            socket_path(Some(Path::new("relative")), one),
            one.join("instance.sock")
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn what_the_system_asks_to_open_is_a_link_or_a_local_path() {
        use super::from_system_url as url;
        let link = "bitbraid://open?path=/work/project";
        assert_eq!(url(link).as_deref(), Some(link));
        assert_eq!(
            url("file:///Users/ada/My%20Project/").as_deref(),
            Some("/Users/ada/My Project")
        );
        assert_eq!(
            url("FILE://localhost/Users/ada/caf%C3%A9").as_deref(),
            Some("/Users/ada/café")
        );
        assert_eq!(url("file:///").as_deref(), Some("/"));
        for passed_over in [
            "",
            "https://example.com/",
            "file://server/share/x",
            "file:relative",
            "file:///bad%zzescape",
            "file:///half%2",
            "file:///not%FFutf8",
            "file:///a%0Ab",
            "file:///a\nb",
        ] {
            assert_eq!(url(passed_over), None, "{passed_over:?}");
        }
        assert_eq!(
            url(&format!("file:///{}", "a".repeat(super::LONGEST))),
            None
        );
    }

    #[test]
    fn messages_are_one_line_and_read_back() {
        let line = encode("bitbraid://open?path=/work/my%20project&branch=main").unwrap();
        assert!(line.ends_with('\n') && line.matches('\n').count() == 1);
        assert_eq!(
            decode(&line).as_deref(),
            Some("bitbraid://open?path=/work/my%20project&branch=main")
        );
        // A path with a quote and a backslash in it survives.
        let odd = "/work/a \"b\" \\c";
        assert_eq!(decode(&encode(odd).unwrap()).as_deref(), Some(odd));
        assert!(encode(&"x".repeat(LONGEST)).is_none(), "too long to be one");
        for hostile in [
            "",
            "{}",
            "{\"open\": 7}",
            "{\"open\": \"\"}",
            "{\"open\": \"a\\nb\"}",
            "{\"open\": \"a\\u0000b\"}",
            "not json",
            "[\"open\"]",
        ] {
            assert_eq!(decode(hostile), None, "{hostile:?}");
        }
        assert!(is_link("bitbraid://open?path=/x") && is_link("BitBraid:x"));
        assert!(!is_link("/work/project") && !is_link("git") && !is_link("éééééé"));
    }

    #[test]
    fn a_start_argument_is_handed_over_whole() {
        assert_eq!(message_for(None, None), None);
        assert_eq!(
            message_for(Some("bitbraid://open?path=/x"), Some(Path::new("ignored"))).as_deref(),
            Some("bitbraid://open?path=/x")
        );
        let here = std::env::current_dir().unwrap();
        let whole = message_for(None, Some(Path::new("."))).unwrap();
        assert_eq!(
            Path::new(&whole),
            bitbraid_core::paths::canonical(&here).unwrap()
        );
        // One that is not there is still made whole.
        let gone = message_for(None, Some(Path::new("not-there-at-all"))).unwrap();
        assert!(Path::new(&gone).is_absolute() && gone.ends_with("not-there-at-all"));
    }

    /// A connection to the instance for `socket`, as a start makes it.
    #[cfg(unix)]
    fn connect(socket: &Path) -> std::os::unix::net::UnixStream {
        std::os::unix::net::UnixStream::connect(socket).unwrap()
    }

    /// What a start sends before its line: nothing on Unix.
    #[cfg(unix)]
    fn token_line(_socket: &Path) -> String {
        String::new()
    }

    /// A connection to the instance the file names, as a start makes it.
    #[cfg(windows)]
    fn connect(file: &Path) -> std::net::TcpStream {
        let place = super::windows::Place::read(file).unwrap();
        std::net::TcpStream::connect(("127.0.0.1", place.port)).unwrap()
    }

    /// What a start sends before its line: the token the file names.
    #[cfg(windows)]
    fn token_line(file: &Path) -> String {
        format!("{}\n", super::windows::Place::read(file).unwrap().token)
    }

    /// What a run that was killed leaves behind: a socket file nobody
    /// listens on.
    #[cfg(unix)]
    fn leave_behind(socket: &Path) {
        let leftover = std::os::unix::net::UnixListener::bind(socket).unwrap();
        drop(leftover);
    }

    /// What a run that was killed leaves behind: a file of ours that names
    /// a port nobody listens on.
    #[cfg(windows)]
    fn leave_behind(file: &Path) {
        let free = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = free.local_addr().unwrap().port();
        drop(free);
        let token = "0123456789abcdef0123456789abcdef".to_owned();
        std::fs::write(file, super::windows::Place { port, token }.text()).unwrap();
    }

    #[test]
    fn what_is_handed_over_arrives_and_only_one_listens() {
        let dir = playground("round");
        let socket = dir.join(if cfg!(windows) {
            "instance.port"
        } else {
            "i.sock"
        });
        // Nobody there: nothing is handed over.
        assert!(!hand_over(&socket, "bitbraid://open?path=/x"));

        let (tx, rx) = mpsc::channel();
        let listening = listen(&socket, move |open| {
            let _ = tx.send(open);
        })
        .unwrap();
        assert!(hand_over(&socket, "bitbraid://open?path=/work/project"));
        assert!(hand_over(&socket, "/work/other project"));
        let wait = Duration::from_secs(5);
        // Both arrive; each connection is attended to on a thread of its
        // own, so which of two is passed on first is not promised.
        let mut arrived = [
            rx.recv_timeout(wait).unwrap(),
            rx.recv_timeout(wait).unwrap(),
        ];
        arrived.sort();
        assert_eq!(
            arrived,
            ["/work/other project", "bitbraid://open?path=/work/project"]
        );

        // A second one does not take the place of the first.
        assert!(listen(&socket, |_| {}).is_none());
        assert!(hand_over(&socket, "/still/the/first"));
        assert_eq!(rx.recv_timeout(wait).unwrap(), "/still/the/first");

        // Things that are not ours are answered "no" and passed nowhere.
        {
            use std::io::{Read as _, Write as _};
            let answer_to = |first: &str, bytes: &[u8]| {
                let mut stream = connect(&socket);
                stream.set_read_timeout(Some(wait)).unwrap();
                let _ = stream.write_all(&[first.as_bytes(), bytes].concat());
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let mut answer = String::new();
                let _ = stream.read_to_string(&mut answer);
                answer
            };
            let first = token_line(&socket);
            for hostile in [
                &b"not json\n"[..],
                b"{\"open\": 7}\n",
                b"\xff\xfe\n",
                b"{\"open\": \"no line end\"}",
                &vec![b'x'; LONGEST * 2][..],
            ] {
                let answer = answer_to(&first, hostile);
                // Over TCP (Windows) the listener's end, closed with most
                // of a flood unread, resets the connection, and Windows
                // then drops what had arrived and was not read yet: the
                // "no" may be lost there.
                let flood = hostile.len() > LONGEST;
                assert!(
                    answer == "no\n" || (cfg!(windows) && flood && answer.is_empty()),
                    "{:?}: {answer:?}",
                    &hostile[..hostile.len().min(20)]
                );
            }
            // On Windows a line of ours without the token, or with
            // another, is not taken either.
            if cfg!(windows) {
                let line = encode("/not/without/the/token").unwrap();
                for first in ["", "\n", "0123456789abcdef0123456789abcdef\n"] {
                    assert_eq!(answer_to(first, line.as_bytes()), "no\n", "{first:?}");
                }
            }
            // One that connects and says nothing does not hold the rest up
            // for good.
            let silent = connect(&socket);
            assert!(hand_over(&socket, "/after/the/silent/one"));
            drop(silent);
        }
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            "/after/the/silent/one"
        );
        assert!(rx.try_recv().is_err(), "nothing hostile was passed on");

        // Gone with the listener; a leftover is replaced, a file of
        // somebody else's is not.
        drop(listening);
        assert!(!socket.exists());
        leave_behind(&socket);
        assert!(socket.exists(), "a leftover is there");
        assert!(!hand_over(&socket, "/x"));
        let again = listen(&socket, |_| {}).unwrap();
        assert!(hand_over(&socket, "/x"));
        drop(again);
        std::fs::write(&socket, "a file of somebody else's").unwrap();
        assert!(listen(&socket, |_| {}).is_none());
        assert!(!hand_over(&socket, "/x"));
        assert_eq!(
            std::fs::read_to_string(&socket).unwrap(),
            "a file of somebody else's"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_only_a_file_of_ours_is_trusted_and_it_is_kept_whole() {
        use super::windows::Place;
        let token = "0123456789abcdef0123456789abcdef";
        let place = Place {
            port: 49731,
            token: token.to_owned(),
        };
        assert_eq!(
            place.text(),
            format!("bitbraid instance\nport 49731\ntoken {token}\n")
        );
        assert_eq!(Place::parse(&place.text()), Some(place));
        for hostile in [
            String::new(),
            "bitbraid instance\n".to_owned(),
            format!("bitbraid instance\nport 49731\ntoken {token}"),
            format!("bitbraid instance\nport 49731\ntoken {token}\nmore\n"),
            format!("bitbraid instance\r\nport 49731\r\ntoken {token}\r\n"),
            format!("something else\nport 49731\ntoken {token}\n"),
            format!("bitbraid instance\nport 0\ntoken {token}\n"),
            format!("bitbraid instance\nport 65536\ntoken {token}\n"),
            format!("bitbraid instance\nport -1\ntoken {token}\n"),
            format!(
                "bitbraid instance\nport 49731\ntoken {}\n",
                token.to_uppercase()
            ),
            "bitbraid instance\nport 49731\ntoken short\n".to_owned(),
            format!("bitbraid instance\nport 49731\ntoken {token}{token}\n"),
        ] {
            assert_eq!(Place::parse(&hostile), None, "{hostile:?}");
        }

        let dir = playground("windows-file");
        let file = dir.join("instance.port");
        // A file of ours whose port somebody else took since, who does
        // not answer as an instance does: it is replaced.
        let foreign = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = foreign.local_addr().unwrap().port();
        let leftover = Place {
            port: taken,
            token: token.to_owned(),
        };
        std::fs::write(&file, leftover.text()).unwrap();
        // It closes what it is sent at once, unread.
        std::thread::spawn(move || {
            for stream in foreign.incoming() {
                drop(stream);
            }
        });
        assert!(!hand_over(&file, "/x"));
        let listening = listen(&file, |_| {}).unwrap();
        let ours = Place::read(&file).unwrap();
        assert_ne!(ours.port, taken);
        assert_ne!(ours.token, token);
        // Written whole, and nothing is left beside it.
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["instance.port"]);
        // A file that names another instance by now is not taken away
        // with this one.
        let later = Place {
            port: ours.port,
            token: "fedcba9876543210fedcba9876543210".to_owned(),
        };
        std::fs::write(&file, later.text()).unwrap();
        drop(listening);
        assert_eq!(Place::read(&file), Some(later));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
