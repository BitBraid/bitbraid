//! A server on this machine for the tests: it listens on a port of the
//! loopback address the system picks, notes what each request said, and
//! answers however the test tells it to. Nothing here leaves the machine.
#![allow(dead_code, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

/// What the server heard: the head of each request, in the order they came.
pub type Heard = Arc<Mutex<Vec<String>>>;

/// A running server.
pub struct Server {
    /// Where it listens.
    pub address: SocketAddr,
    /// What it heard so far.
    pub heard: Heard,
}

impl Server {
    /// `http://127.0.0.1:<port>`.
    pub fn base(&self) -> String {
        format!("http://{}", self.address)
    }

    /// The heads of the requests heard so far.
    pub fn requests(&self) -> Vec<String> {
        self.heard.lock().unwrap().clone()
    }

    /// The first line of each request heard so far.
    pub fn lines(&self) -> Vec<String> {
        self.requests()
            .iter()
            .map(|head| head.lines().next().unwrap_or_default().to_owned())
            .collect()
    }
}

/// Read a request's head off a connection: up to the empty line. None
/// when the other side went away first.
pub fn read_head<S: Read>(stream: &mut S) -> Option<String> {
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(1) => head.push(byte[0]),
            _ => return None,
        }
        if head.len() > 64 * 1024 {
            return None;
        }
    }
    Some(String::from_utf8_lossy(&head).into_owned())
}

/// Start a server. `answer` is given each connection after its request's
/// head was read and noted, together with that head; it writes what it
/// likes and the connection closes when it returns.
pub fn serve(answer: impl Fn(&mut TcpStream, &str) + Send + Sync + 'static) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let heard: Heard = Arc::default();
    let answer = Arc::new(answer);
    let noted = heard.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let (answer, noted) = (answer.clone(), noted.clone());
            thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
                let Some(head) = read_head(&mut stream) else {
                    return;
                };
                noted.lock().unwrap().push(head.clone());
                answer(&mut stream, &head);
            });
        }
    });
    Server { address, heard }
}

/// A whole answer with a body of a known length.
pub fn answer(status: &str, kind: &str, body: &[u8]) -> Vec<u8> {
    let mut bytes = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    bytes.extend_from_slice(body);
    bytes
}

/// The path of a request, from its head: `GET <path> HTTP/1.1`.
pub fn path_of(head: &str) -> &str {
    head.split(' ').nth(1).unwrap_or_default()
}

/// A PNG of one colour sliding into another, `side` pixels each way.
pub fn png(side: u32, red: u8, green: u8, blue: u8) -> Vec<u8> {
    let picture = image::RgbaImage::from_fn(side, side, |x, y| {
        let shade = ((x + y) * 60 / side.max(1)) as u8;
        image::Rgba([
            red.saturating_sub(shade),
            green.saturating_sub(shade),
            blue.saturating_sub(shade),
            255,
        ])
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(picture)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

/// Keep writing until the other side stops taking it, or `most` writes
/// were made: what a server that never ends does.
pub fn write_forever(stream: &mut TcpStream, piece: &[u8], pause: Duration, most: usize) {
    for _ in 0..most {
        if stream.write_all(piece).is_err() || stream.flush().is_err() {
            return;
        }
        if !pause.is_zero() {
            thread::sleep(pause);
        }
    }
}
