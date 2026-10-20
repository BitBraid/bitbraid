//! The LFS lock commands against a server that keeps locks.
//!
//! No LFS server is at hand where the tests run, so one stands in for it:
//! [`Server`] speaks the File Locking API of git-lfs over plain HTTP on a
//! port of this machine and keeps the locks in memory. It decides who the
//! user is, a lock of somebody else's can be planted in it, and it can be
//! told to answer badly. Everything it is asked is kept, so that a test
//! can say what a command sent as well as what it returned.
//!
//! The commands run on one scratch copy and `git lfs` itself on a twin
//! that has a stand-in of its own: what the two servers keep, what they
//! were asked and which files may be written have to come out the same.
//!
//! The commands run git as whoever runs the tests has it set up. What
//! could reach outside is pinned in each copy's own configuration: where
//! the LFS server is, no credential helper, hooks kept inside the copy.
//! The stand-in never asks who is there, so nobody is asked for a password.
//!
//! What the stand-in is not, compared with a real server:
//!
//! - It does not authenticate. Nothing is ever answered with 401, so the
//!   credential helpers, the askpass program, `lfs.<url>.access` and the
//!   `Authorization` header are not exercised, nor is the SSH way of
//!   finding the API (`git-lfs-authenticate`). Who "the user" is, is a
//!   name the test gives it.
//! - It speaks plain HTTP/1.1 and closes the connection after each answer:
//!   no TLS, no connection kept open, no chunked or compressed bodies, no
//!   redirects, no proxy.
//! - Locks are not kept per reference: the `ref` of a request and the
//!   `refspec` of a listing are recorded and otherwise ignored, where a
//!   real server may keep locks per branch and refuse a ref it does not
//!   allow.
//! - There are no access rights: whoever asks may lock, and may unlock
//!   somebody else's lock with `force`, except where it is told to answer
//!   as to a reader (`Manner::Reader`: the list, and 403 to everything
//!   else; `Manner::ReadingToken`, the same in github.com's words). A real
//!   server refuses those who may not push, and lets only some force.
//! - It pages only when told to (a page size the test sets, or the `limit`
//!   of a request, which git-lfs does not send for these commands); a real
//!   server pages by itself at a size of its own.
//! - It has no rate limits, no `Retry-After`, no `request_id` or
//!   `documentation_url` in its errors, and paths are compared byte for
//!   byte (no folding of case or of Unicode forms).
//! - The other half of the API, the objects (`/objects/batch`), is not
//!   there at all: it answers 404.
//!
//! What a real server does instead is run by the opt-in live tests of the
//! command line (`apps/bitbraid-cli/tests/suite/live_lfs.rs`), and TLS in
//! ci by `lfs_tls.rs` there.
//!
//! Sources: `docs/api/locking.md` of the git-lfs repository (the File
//! Locking API: creating a lock, listing locks, listing them for
//! verification, deleting one; the status codes and the bodies of each);
//! `docs/api/server-discovery.md` there (where the API of a remote is:
//! `lfs.url`, `remote.<name>.lfsurl`, or the remote's address followed by
//! `/info/lfs`); git-lfs-lock(1), git-lfs-locks(1), git-lfs-unlock(1),
//! git-lfs-config(5) (`lfs.url`, `lfs.activitytimeout`,
//! `lfs.setlockablereadonly`); RFC 9112 (the lines of a request and of an
//! answer over HTTP/1.1) and RFC 3986 (percent-encoding). Which requests
//! git-lfs sends for which command, and in what order, the documents leave
//! open: that was found by having a stand-in write down what git-lfs 3.8.0
//! sent it.
//!
//! Without git-lfs installed there is nothing to run: each test that
//! needs it then says that it was skipped and passes.
#![allow(clippy::print_stderr)]

use std::{
    cell::RefCell,
    io::{Read as _, Write as _},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

/// Who the stand-in takes its user for.
const USER: &str = "Ada Stand-in";
/// Somebody else.
const OTHER: &str = "Bob Other";
/// Where the API is on the stand-in, as for a remote at `/scratch.git`.
const BASE: &str = "/scratch.git/info/lfs";
/// The media type of the API.
const LFS_JSON: &str = "application/vnd.git-lfs+json";
/// The branch the copies have checked out, as git-lfs names it.
const BRANCH: &str = "refs/heads/main";
/// The files that are there to lock, all of them lockable.
const FILES: [&str; 4] = ["one.bin", "sub/two.bin", "d ir/sp ace.bin", "three.bin"];
/// No command may take longer than this against a server that answers,
/// however badly.
const PROMPT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------
// The stand-in.

/// A lock as the stand-in keeps it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Held {
    id: String,
    path: String,
    owner: String,
    locked_at: String,
}

impl Held {
    /// The lock as the API writes it.
    fn json(&self) -> Value {
        json!({
            "id": self.id,
            "path": self.path,
            "locked_at": self.locked_at,
            "owner": { "name": self.owner },
        })
    }
}

/// When the lock with this number was made: the stand-in has a clock of
/// its own, so that what it answers is the same in every run. A time as
/// the API asks for it, in RFC 3339 with seconds, in capitals.
fn stamp(number: u64) -> String {
    format!(
        "2026-10-01T10:{:02}:{:02}Z",
        (number / 60) % 60,
        number % 60
    )
}

/// Something the stand-in was asked.
#[derive(Debug, Clone, PartialEq)]
struct Asked {
    method: String,
    /// The path below the API's address; the whole path for what was
    /// asked elsewhere.
    path: String,
    /// The query, decoded, in the order it came.
    query: Vec<(String, String)>,
    /// The body; null without one, text when it was not JSON.
    body: Value,
}

fn post(path: &str, body: Value) -> Asked {
    Asked {
        method: "POST".to_owned(),
        path: path.to_owned(),
        query: Vec::new(),
        body,
    }
}

fn get(path: &str, query: &[(&str, &str)]) -> Asked {
    Asked {
        method: "GET".to_owned(),
        path: path.to_owned(),
        query: query
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: Value::Null,
    }
}

/// The request git-lfs lists locks for verification with.
fn verify() -> Asked {
    post("/locks/verify", json!({ "ref": { "name": BRANCH } }))
}

/// The same, for the page that begins at `cursor`.
fn verify_from(cursor: &str) -> Asked {
    post(
        "/locks/verify",
        json!({ "ref": { "name": BRANCH }, "cursor": cursor }),
    )
}

/// The request git-lfs looks the lock on a path up with.
fn lookup(path: &str) -> Asked {
    get("/locks", &[("path", path), ("refspec", BRANCH)])
}

/// The request git-lfs locks a path with.
fn create(path: &str) -> Asked {
    post("/locks", json!({ "path": path, "ref": { "name": BRANCH } }))
}

/// The request git-lfs gives the lock `id` up with.
fn delete(id: &str, force: bool) -> Asked {
    post(
        &format!("/locks/{id}/unlock"),
        json!({ "force": force, "ref": { "name": BRANCH } }),
    )
}

/// How the stand-in answers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Manner {
    /// As the API says.
    Proper,
    /// With the API's error for a server that failed.
    Failing,
    /// With a failure that is a page of text, as something in front of a
    /// server answers.
    FailingPage,
    /// With success and bytes that are not JSON.
    Garbage,
    /// With success and no body.
    EmptyBody,
    /// With success and JSON of another shape.
    WrongShape,
    /// With success and an empty object, whatever was asked.
    Hollow,
    /// With less of a body than it announced, and then nothing.
    CutShort,
    /// Not at all: it reads what is asked and closes.
    Hangup,
    /// Not at all: it reads what is asked and keeps the connection.
    Silent,
    /// With one more page of locks for verification each time, without
    /// end. Everything else as the API says.
    Endless,
    /// As to a user who may read and not push: the list as the API says,
    /// and everything else, verification too, refused with 403. GitLab
    /// answers a reporter so, and github.com one who may only read.
    Reader,
    /// As github.com answers one whose role may push and whose token may
    /// only read (a fine-grained token): the list as the API says,
    /// verification and a deletion refused with 403 and words that ask
    /// for push access, a new lock refused with 403 and no words, which
    /// git-lfs then says in its own.
    ReadingToken,
}

/// What the stand-in sends back.
#[derive(Debug)]
enum Reply {
    Json(u16, Value),
    Raw {
        status: u16,
        kind: &'static str,
        body: Vec<u8>,
        /// The length it announces.
        announced: usize,
    },
    Hangup,
    Silent,
}

/// What the stand-in keeps.
#[derive(Debug)]
struct Kept {
    user: String,
    locks: Vec<Held>,
    /// How many locks were made; the next one's id is one more.
    made: u64,
    asked: Vec<Asked>,
    /// What was asked in a way the API does not allow.
    complaints: Vec<String>,
    manner: Manner,
    /// At most this many locks in one answer, when set.
    page: Option<usize>,
    /// How many pages an endless listing has sent.
    pages: u64,
}

/// A request as it came.
#[derive(Debug)]
struct Request {
    method: String,
    path: String,
    query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.as_str())
    }
    fn asked_for(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.as_str())
    }
    /// The path below the API's address.
    fn below(&self) -> Option<&str> {
        self.path.strip_prefix(BASE)
    }
    fn json(&self) -> Value {
        if self.body.is_empty() {
            return Value::Null;
        }
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&self.body).into_owned()))
    }
}

fn message(text: &str) -> Value {
    json!({ "message": text })
}

impl Kept {
    /// Write down what was asked, and what was wrong with how.
    fn note(&mut self, request: &Request) {
        let path = request.below().unwrap_or(&request.path).to_owned();
        // Every request names the API's media type as what it accepts,
        // and one with a body as what that body is.
        if !request
            .header("accept")
            .is_some_and(|kind| kind.starts_with(LFS_JSON))
        {
            self.complaints.push(format!(
                "{} {path}: Accept is not {LFS_JSON}",
                request.method
            ));
        }
        if request.method == "POST"
            && !request
                .header("content-type")
                .is_some_and(|kind| kind.starts_with(LFS_JSON))
        {
            self.complaints.push(format!(
                "{} {path}: Content-Type is not {LFS_JSON}",
                request.method
            ));
        }
        self.asked.push(Asked {
            method: request.method.clone(),
            path,
            query: request.query.clone(),
            body: request.json(),
        });
    }

    /// One page of `found`, which is in the order the locks were made,
    /// and where the next page begins: the id of its first lock.
    fn paged(
        &self,
        found: Vec<Held>,
        cursor: Option<&str>,
        limit: Option<usize>,
    ) -> (Vec<Held>, Option<String>) {
        let number = |id: &str| id.parse::<u64>().unwrap_or(0);
        let from = cursor.map(number).unwrap_or(0);
        let mut rest = found.into_iter().filter(|lock| number(&lock.id) >= from);
        let size = match (limit, self.page) {
            (Some(asked), Some(own)) => asked.min(own),
            (asked, own) => asked.or(own).unwrap_or(usize::MAX),
        };
        let page: Vec<Held> = rest.by_ref().take(size.max(1)).collect();
        (page, rest.next().map(|lock| lock.id))
    }

    /// The answer the API gives.
    fn proper(&mut self, request: &Request) -> (u16, Value) {
        let Some(below) = request.below() else {
            return (404, message("there is nothing at this address"));
        };
        let said = request.json();
        let limit = |value: Option<&Value>| {
            value
                .and_then(Value::as_u64)
                .and_then(|limit| usize::try_from(limit).ok())
        };
        match (request.method.as_str(), below) {
            // Create a lock: 201 with it, or 409 with the one that is in
            // the way.
            ("POST", "/locks") => {
                let Some(path) = said
                    .get("path")
                    .and_then(Value::as_str)
                    .filter(|path| !path.is_empty())
                else {
                    // The document does not say what a request without a
                    // path gets.
                    return (400, message("no path to lock was given"));
                };
                if let Some(held) = self.locks.iter().find(|lock| lock.path == path) {
                    return (
                        409,
                        json!({ "lock": held.json(), "message": "the path is locked already" }),
                    );
                }
                self.made += 1;
                let lock = Held {
                    id: self.made.to_string(),
                    path: path.to_owned(),
                    owner: self.user.clone(),
                    locked_at: stamp(self.made),
                };
                self.locks.push(lock.clone());
                (201, json!({ "lock": lock.json() }))
            }
            // List locks, all or those of a path or with an id.
            ("GET", "/locks") => {
                let found: Vec<Held> = self
                    .locks
                    .iter()
                    .filter(|lock| {
                        request
                            .asked_for("path")
                            .is_none_or(|path| lock.path == path)
                    })
                    .filter(|lock| request.asked_for("id").is_none_or(|id| lock.id == id))
                    .cloned()
                    .collect();
                let limit = request
                    .asked_for("limit")
                    .and_then(|limit| limit.parse().ok());
                let (page, next) = self.paged(found, request.asked_for("cursor"), limit);
                let mut answer =
                    json!({ "locks": page.iter().map(Held::json).collect::<Vec<_>>() });
                if let Some(next) = next {
                    answer["next_cursor"] = Value::String(next);
                }
                (200, answer)
            }
            // List locks for verification: the user's and the others'.
            ("POST", "/locks/verify") => {
                let (page, next) = self.paged(
                    self.locks.clone(),
                    said.get("cursor").and_then(Value::as_str),
                    limit(said.get("limit")),
                );
                let of = |ours: bool| -> Vec<Value> {
                    page.iter()
                        .filter(|lock| (lock.owner == self.user) == ours)
                        .map(Held::json)
                        .collect()
                };
                let mut answer = json!({ "ours": of(true), "theirs": of(false) });
                if let Some(next) = next {
                    answer["next_cursor"] = Value::String(next);
                }
                (200, answer)
            }
            // Delete a lock: the user's own, or anybody's with `force`.
            ("POST", unlock) if unlock.ends_with("/unlock") => {
                let id = unlock
                    .strip_prefix("/locks/")
                    .and_then(|rest| rest.strip_suffix("/unlock"))
                    .unwrap_or("");
                let Some(at) = self.locks.iter().position(|lock| lock.id == id) else {
                    return (404, message("there is no lock with this id"));
                };
                let force = said.get("force").and_then(Value::as_bool).unwrap_or(false);
                if self.locks[at].owner != self.user && !force {
                    return (
                        403,
                        message("the lock is somebody else's and is given up only with force"),
                    );
                }
                let lock = self.locks.remove(at);
                (200, json!({ "lock": lock.json() }))
            }
            _ => (404, message("the stand-in has nothing of that kind")),
        }
    }

    fn answer(&mut self, request: &Request) -> Reply {
        let raw = |status: u16, kind: &'static str, body: &[u8]| Reply::Raw {
            status,
            kind,
            body: body.to_vec(),
            announced: body.len(),
        };
        match self.manner {
            Manner::Proper => {
                let (status, body) = self.proper(request);
                Reply::Json(status, body)
            }
            Manner::Failing => Reply::Json(500, message("the stand-in fails on purpose")),
            Manner::FailingPage => raw(
                500,
                "text/html",
                b"<html><body><h1>Bad gateway</h1></body></html>",
            ),
            Manner::Garbage => raw(200, LFS_JSON, b"\x00\xff<html>not what was asked for{{{"),
            Manner::EmptyBody => raw(200, LFS_JSON, b""),
            Manner::WrongShape => Reply::Json(
                200,
                json!({ "lock": [], "locks": "many", "ours": 7, "theirs": { "a": 1 } }),
            ),
            Manner::Hollow => {
                let created = request.method == "POST" && request.below() == Some("/locks");
                Reply::Json(if created { 201 } else { 200 }, json!({}))
            }
            Manner::CutShort => Reply::Raw {
                status: 200,
                kind: LFS_JSON,
                body: b"{\"locks\": [{\"id\": \"1\", \"pa".to_vec(),
                announced: 4000,
            },
            Manner::Hangup => Reply::Hangup,
            Manner::Silent => Reply::Silent,
            Manner::Endless => {
                if request.method == "POST" && request.below() == Some("/locks/verify") {
                    self.pages += 1;
                    let lock = Held {
                        id: self.pages.to_string(),
                        path: format!("endless/{}.bin", self.pages),
                        owner: OTHER.to_owned(),
                        locked_at: stamp(self.pages),
                    };
                    return Reply::Json(
                        200,
                        json!({
                            "ours": [],
                            "theirs": [lock.json()],
                            "next_cursor": (self.pages + 1).to_string(),
                        }),
                    );
                }
                let (status, body) = self.proper(request);
                Reply::Json(status, body)
            }
            Manner::Reader => {
                if request.method == "GET" && request.below() == Some("/locks") {
                    let (status, body) = self.proper(request);
                    return Reply::Json(status, body);
                }
                Reply::Json(403, message("this user may read and not push"))
            }
            Manner::ReadingToken => match (request.method.as_str(), request.below()) {
                ("GET", Some("/locks")) => {
                    let (status, body) = self.proper(request);
                    Reply::Json(status, body)
                }
                ("POST", Some("/locks")) => Reply::Json(403, json!({})),
                ("POST", Some("/locks/verify")) => {
                    Reply::Json(403, message("You must have push access to verify locks"))
                }
                _ => Reply::Json(403, message("You must have push access to delete a lock")),
            },
        }
    }
}

/// `%41` and the like as the bytes they stand for; in a query a `+` is a
/// space as well.
fn decoded(text: &str, query: bool) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let hex = |at: usize| {
            bytes
                .get(at)
                .and_then(|digit| char::from(*digit).to_digit(16))
        };
        match bytes[at] {
            b'%' => match (hex(at + 1), hex(at + 2)) {
                (Some(high), Some(low)) => {
                    out.push(u8::try_from(high * 16 + low).unwrap_or(b'?'));
                    at += 2;
                }
                _ => out.push(b'%'),
            },
            b'+' if query => out.push(b' '),
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn position(bytes: &[u8], wanted: &[u8]) -> Option<usize> {
    bytes
        .windows(wanted.len())
        .position(|window| window == wanted)
}

/// Read one request whole: its first line, its headers, and as much of a
/// body as it announces. None when it is not one, or too large.
fn read_request(stream: &mut TcpStream) -> Option<Request> {
    const MOST_HEAD: usize = 64 * 1024;
    const MOST_BODY: usize = 1024 * 1024;
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    let head_end = loop {
        if let Some(at) = position(&bytes, b"\r\n\r\n") {
            break at;
        }
        if bytes.len() > MOST_HEAD {
            return None;
        }
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return None,
            Ok(read) => bytes.extend_from_slice(&buffer[..read]),
        }
    };
    let head = String::from_utf8_lossy(&bytes[..head_end]).into_owned();
    let mut lines = head.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let method = first.next()?.to_owned();
    let target = first.next()?.to_owned();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_owned()))
        .collect();
    let length: usize = headers
        .iter()
        .find(|(name, _)| name == "content-length")
        .and_then(|(_, value)| value.parse().ok())
        .unwrap_or(0);
    if length > MOST_BODY {
        return None;
    }
    let mut body = bytes[head_end + 4..].to_vec();
    while body.len() < length {
        match stream.read(&mut buffer) {
            Ok(0) | Err(_) => return None,
            Ok(read) => body.extend_from_slice(&buffer[..read]),
        }
    }
    body.truncate(length);
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    let query = query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (decoded(name, true), decoded(value, true))
        })
        .collect();
    Some(Request {
        method,
        path: decoded(path, false),
        query,
        headers,
        body,
    })
}

fn phrase(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        409 => "Conflict",
        _ => "Internal Server Error",
    }
}

fn send(stream: &mut TcpStream, status: u16, kind: &str, body: &[u8], announced: usize) {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {kind}\r\nContent-Length: {announced}\r\nConnection: close\r\n\r\n",
        phrase(status)
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// Answer the one request of a connection.
fn serve(mut stream: TcpStream, kept: &Mutex<Kept>, stop: &AtomicBool) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Some(request) = read_request(&mut stream) else {
        return;
    };
    let reply = {
        let mut kept = kept.lock().unwrap();
        kept.note(&request);
        kept.answer(&request)
    };
    match reply {
        Reply::Json(status, body) => {
            let body = serde_json::to_vec(&body).unwrap();
            send(&mut stream, status, LFS_JSON, &body, body.len());
        }
        Reply::Raw {
            status,
            kind,
            body,
            announced,
        } => send(&mut stream, status, kind, &body, announced),
        Reply::Hangup => {}
        Reply::Silent => {
            // Until it is told to answer again, or to end; never for
            // longer than a test may take.
            let began = Instant::now();
            while !stop.load(Ordering::SeqCst)
                && kept.lock().unwrap().manner == Manner::Silent
                && began.elapsed() < Duration::from_secs(120)
            {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

/// The stand-in: it listens on a port of this machine that the system
/// picks, and answers each connection on a thread of its own.
#[derive(Debug)]
struct Server {
    port: u16,
    kept: Arc<Mutex<Kept>>,
    stop: Arc<AtomicBool>,
    listening: Option<JoinHandle<()>>,
}

impl Server {
    /// Start one that takes whoever asks for `user`.
    fn start(user: &str) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let kept = Arc::new(Mutex::new(Kept {
            user: user.to_owned(),
            locks: Vec::new(),
            made: 0,
            asked: Vec::new(),
            complaints: Vec::new(),
            manner: Manner::Proper,
            page: None,
            pages: 0,
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (shared, stopping) = (kept.clone(), stop.clone());
        let listening = std::thread::Builder::new()
            .name("lfs-stand-in".to_owned())
            .spawn(move || {
                while !stopping.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((stream, _)) => {
                            let (kept, stop) = (shared.clone(), stopping.clone());
                            std::thread::spawn(move || serve(stream, &kept, &stop));
                        }
                        Err(_) => std::thread::sleep(Duration::from_millis(5)),
                    }
                }
            })
            .unwrap();
        Self {
            port,
            kept,
            stop,
            listening: Some(listening),
        }
    }

    /// The address of a remote that has its API here.
    fn origin(&self) -> String {
        format!("http://127.0.0.1:{}/scratch.git", self.port)
    }

    /// The address of the API.
    fn url(&self) -> String {
        format!("http://127.0.0.1:{}{BASE}", self.port)
    }

    /// Put a lock on `path` that `owner` holds.
    fn plant(&self, path: &str, owner: &str) -> Held {
        let mut kept = self.kept.lock().unwrap();
        kept.made += 1;
        let lock = Held {
            id: kept.made.to_string(),
            path: path.to_owned(),
            owner: owner.to_owned(),
            locked_at: stamp(kept.made),
        };
        kept.locks.push(lock.clone());
        lock
    }

    /// The locks it keeps: path and owner, in the order they were made.
    fn held(&self) -> Vec<(String, String)> {
        self.kept
            .lock()
            .unwrap()
            .locks
            .iter()
            .map(|lock| (lock.path.clone(), lock.owner.clone()))
            .collect()
    }

    /// What it was asked since this was last called.
    fn take_asked(&self) -> Vec<Asked> {
        std::mem::take(&mut self.kept.lock().unwrap().asked)
    }

    /// What was asked in a way the API does not allow.
    fn complaints(&self) -> Vec<String> {
        self.kept.lock().unwrap().complaints.clone()
    }

    fn answer(&self, manner: Manner) {
        self.kept.lock().unwrap().manner = manner;
    }

    fn page(&self, size: Option<usize>) {
        self.kept.lock().unwrap().page = size;
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(listening) = self.listening.take() {
            let _ = listening.join();
        }
    }
}

// ---------------------------------------------------------------------
// The copies.

/// Whether the test called `test` has to be skipped: git has no git-lfs.
fn skipped(test: &str) -> bool {
    let found = Command::new("git")
        .args(["lfs", "version"])
        .output()
        .is_ok_and(|out| out.status.success());
    if !found {
        eprintln!("skipped {test}: git-lfs is not installed");
    }
    !found
}

fn git_at(dir: impl Into<PathBuf>) -> Git {
    Git::new(dir, fixtures_out_dir().join(".home"))
}

/// Whoever runs the tests may keep hooks in a place of their own, for all
/// their repositories, and git-lfs writes its hooks where git looks for
/// them whenever its filter runs. So every repository a command is run in
/// has that place set to its own directory, and theirs is left alone.
fn keep_hooks_here(dir: &Path, git_dir: &Path) {
    let theirs = Command::new("git")
        .current_dir(dir)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .is_ok_and(|out| out.status.success() && !out.stdout.is_empty());
    if theirs {
        let hooks = git_dir.join("hooks");
        git_at(dir)
            .run(["config", "core.hooksPath", hooks.to_str().unwrap()])
            .unwrap();
    }
}

/// How a copy is told where its LFS server is.
#[derive(Debug, Clone, Copy)]
enum Found {
    /// `lfs.url`.
    LfsUrl,
    /// A remote at an address that the API is below.
    Remote,
    /// A remote that is elsewhere, with `remote.<name>.lfsurl`.
    RemoteSetting,
}

/// What every copy has in its own configuration, so that nothing of
/// whoever runs the tests is asked or changed: no credential helper, and
/// lockable files read-only as git-lfs has them by default.
fn pin(git: &Git) {
    for (key, value) in [
        ("credential.helper", ""),
        ("lfs.setlockablereadonly", "true"),
    ] {
        git.run(["config", "--local", key, value]).unwrap();
    }
}

/// A scratch copy with LFS set up in it, [`FILES`] committed as lockable
/// files, and `server` as its LFS server.
fn prepared(server: &Server, found: Found) -> Scratch {
    let copy = Scratch::of("linear").unwrap();
    keep_hooks_here(copy.path(), &copy.path().join(".git"));
    let git = copy.git();
    git.run(["lfs", "install", "--local"]).unwrap();
    git.run(["lfs", "track", "--lockable", "*.bin"]).unwrap();
    for file in FILES {
        copy.write(file, &format!("content of {file}\n")).unwrap();
    }
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "lockable files"]).unwrap();
    pin(&git);
    let elsewhere = copy.path().with_extension("nowhere.git");
    match found {
        Found::LfsUrl => {
            git.run(["config", "--local", "lfs.url", &server.url()])
                .unwrap();
        }
        Found::Remote => {
            git.run(["remote", "add", "origin", &server.origin()])
                .unwrap();
        }
        Found::RemoteSetting => {
            git.run(["remote", "add", "origin", elsewhere.to_str().unwrap()])
                .unwrap();
            git.run(["config", "--local", "remote.origin.lfsurl", &server.url()])
                .unwrap();
        }
    }
    copy
}

fn writable(copy: &Scratch, file: &str) -> bool {
    !std::fs::metadata(copy.path().join(file))
        .unwrap()
        .permissions()
        .readonly()
}

/// The locks a server keeps (path and owner), and for each of [`FILES`]
/// whether it may be written.
type Standing = (Vec<(String, String)>, Vec<(String, bool)>);

/// What a lock command can change: the locks the server keeps, and which
/// of the files may be written (git-lfs keeps a lockable file read-only
/// while it is not locked by the user).
fn standing(server: &Server, copy: &Scratch) -> Standing {
    (
        server.held(),
        FILES
            .iter()
            .map(|file| ((*file).to_owned(), writable(copy, file)))
            .collect(),
    )
}

/// A copy the commands run on, with its stand-in.
struct Bench {
    server: Server,
    copy: Scratch,
    registry: Registry,
    ctx: RefCell<CommandContext>,
    /// The stages the commands reported as their progress.
    reported: Arc<Mutex<Vec<String>>>,
}

impl Bench {
    fn new() -> Self {
        Self::found(Found::LfsUrl)
    }
    fn found(found: Found) -> Self {
        let server = Server::start(USER);
        let copy = prepared(&server, found);
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
            .expect("open scratch");
        assert_eq!(server.take_asked(), [], "setting up asked the server");
        let reported: Arc<Mutex<Vec<String>>> = Arc::default();
        let sink = reported.clone();
        ctx.progress = Some(Arc::new(move |progress| {
            sink.lock().unwrap().push(progress.stage);
        }));
        Self {
            server,
            copy,
            registry,
            ctx: RefCell::new(ctx),
            reported,
        }
    }
    /// What the commands reported as their progress so far. A lock is
    /// asked for and answered: there is nothing to be part of the way
    /// through.
    fn reported(&self) -> Vec<String> {
        self.reported.lock().unwrap().clone()
    }
    fn run(&self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx.borrow_mut(), &params)
            .unwrap_or_else(|err| panic!("{id} {params}: {err}"))
    }
    fn try_run(&self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx.borrow_mut(), &params)
            .map_err(|err| err.to_string())
    }
    /// The error of a command that has to fail, which has to say `why`.
    fn refused(&self, id: &str, params: Value, why: &str) -> String {
        let err = self
            .try_run(id, params.clone())
            .expect_err(&format!("{id} {params} should be refused"));
        assert!(err.contains(why), "{id} {params}: {err}");
        err
    }
    fn standing(&self) -> Standing {
        standing(&self.server, &self.copy)
    }
}

/// A copy that git-lfs itself is run on, with a stand-in of its own.
struct Twin {
    server: Server,
    copy: Scratch,
}

impl Twin {
    fn new() -> Self {
        let server = Server::start(USER);
        let copy = prepared(&server, Found::LfsUrl);
        Self { server, copy }
    }
    /// Run `git lfs` with `args`; what it printed, or how it failed.
    fn lfs(&self, args: &[&str]) -> Result<String, String> {
        let mut all = vec!["lfs"];
        all.extend(args);
        self.copy.git().run(all).map_err(|err| err.to_string())
    }
    fn standing(&self) -> Standing {
        standing(&self.server, &self.copy)
    }
}

/// A lock as the commands return it.
fn lock(id: u64, path: &str, owner: &str, ours: bool) -> Value {
    json!({
        "id": id.to_string(),
        "path": path,
        "owner": owner,
        "locked_at": stamp(id),
        "ours": ours,
    })
}

/// The locks git-lfs printed for a verified listing, as the commands
/// return them: the user's first.
fn as_returned(printed: &str) -> Value {
    let said: Value = serde_json::from_str(printed).unwrap();
    let of = |name: &str, ours: bool| -> Vec<Value> {
        said[name]
            .as_array()
            .unwrap()
            .iter()
            .map(|lock| {
                json!({
                    "id": lock["id"],
                    "path": lock["path"],
                    "owner": lock["owner"]["name"],
                    "locked_at": lock["locked_at"],
                    "ours": ours,
                })
            })
            .collect()
    };
    let mut all = of("ours", true);
    all.extend(of("theirs", false));
    Value::Array(all)
}

// ---------------------------------------------------------------------
// The stand-in by itself.

/// Ask the stand-in as a client of the API would, and read its answer:
/// the status and the body.
fn ask(server: &Server, method: &str, target: &str, body: Option<Value>) -> (u16, Value) {
    let mut stream = TcpStream::connect(("127.0.0.1", server.port)).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let body = body.map(|body| serde_json::to_vec(&body).unwrap());
    let mut request = format!(
        "{method} {target} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: {LFS_JSON}\r\n",
        server.port
    );
    if let Some(body) = &body {
        request.push_str(&format!(
            "Content-Type: {LFS_JSON}\r\nContent-Length: {}\r\n",
            body.len()
        ));
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    if let Some(body) = &body {
        stream.write_all(body).unwrap();
    }
    let mut answer = Vec::new();
    let _ = stream.read_to_end(&mut answer);
    let head_end = position(&answer, b"\r\n\r\n").expect("an answer with a head");
    let head = String::from_utf8_lossy(&answer[..head_end]).into_owned();
    let status: u16 = head.split(' ').nth(1).unwrap().parse().unwrap();
    assert!(
        head.to_ascii_lowercase()
            .contains(&format!("content-type: {LFS_JSON}")),
        "{head}"
    );
    (
        status,
        serde_json::from_slice(&answer[head_end + 4..]).unwrap_or(Value::Null),
    )
}

/// The stand-in does what the document says of a server, asked without
/// git-lfs in between. This runs also where git-lfs is not installed.
#[test]
fn the_stand_in_answers_as_the_locking_api_says() {
    let server = Server::start(USER);
    let at = |below: &str| format!("{BASE}{below}");
    let branch = json!({ "name": BRANCH });

    // Nothing is locked.
    assert_eq!(
        ask(&server, "GET", &at("/locks"), None),
        (200, json!({ "locks": [] }))
    );
    assert_eq!(
        ask(
            &server,
            "POST",
            &at("/locks/verify"),
            Some(json!({ "ref": branch }))
        ),
        (200, json!({ "ours": [], "theirs": [] }))
    );

    // A lock is made for the user: 201 and the lock.
    let first = json!({
        "id": "1",
        "path": "one.bin",
        "locked_at": "2026-10-01T10:00:01Z",
        "owner": { "name": USER },
    });
    assert_eq!(
        ask(
            &server,
            "POST",
            &at("/locks"),
            Some(json!({ "path": "one.bin", "ref": branch }))
        ),
        (201, json!({ "lock": first }))
    );
    // A second one on the same path: 409 and the lock that is in the way.
    let (status, said) = ask(
        &server,
        "POST",
        &at("/locks"),
        Some(json!({ "path": "one.bin", "ref": branch })),
    );
    assert_eq!(status, 409);
    assert_eq!(said["lock"], first);
    assert!(said["message"].is_string(), "{said}");
    // One without a path is none.
    assert_eq!(ask(&server, "POST", &at("/locks"), Some(json!({}))).0, 400);

    // Somebody else's, and one more of the user's.
    let theirs = server.plant("sub/two.bin", OTHER).json();
    let (status, said) = ask(
        &server,
        "POST",
        &at("/locks"),
        Some(json!({ "path": "d ir/sp ace.bin" })),
    );
    assert_eq!(status, 201);
    let third = said["lock"].clone();
    assert_eq!(third["id"], "3");

    // Listed whole, by path, by id, and by both.
    let listed = |query: &str| {
        let (status, said) = ask(&server, "GET", &format!("{}{query}", at("/locks")), None);
        assert_eq!(status, 200, "{query}");
        said
    };
    assert_eq!(listed(""), json!({ "locks": [first, theirs, third] }));
    assert_eq!(
        listed("?path=sub%2Ftwo.bin&refspec=refs%2Fheads%2Fmain"),
        json!({ "locks": [theirs] })
    );
    assert_eq!(
        listed("?path=d+ir%2Fsp+ace.bin"),
        json!({ "locks": [third] })
    );
    assert_eq!(listed("?id=1"), json!({ "locks": [first] }));
    assert_eq!(listed("?id=1&path=sub/two.bin"), json!({ "locks": [] }));
    assert_eq!(listed("?path=nowhere.bin"), json!({ "locks": [] }));
    // In pages: the cursor of the next page comes with each but the last.
    assert_eq!(
        listed("?limit=2"),
        json!({ "locks": [first, theirs], "next_cursor": "3" })
    );
    assert_eq!(listed("?limit=2&cursor=3"), json!({ "locks": [third] }));

    // For verification: whose each is, also in pages.
    let verified = |body: Value| {
        let (status, said) = ask(&server, "POST", &at("/locks/verify"), Some(body));
        assert_eq!(status, 200);
        said
    };
    assert_eq!(
        verified(json!({ "ref": branch })),
        json!({ "ours": [first, third], "theirs": [theirs] })
    );
    assert_eq!(
        verified(json!({ "limit": 1 })),
        json!({ "ours": [first], "theirs": [], "next_cursor": "2" })
    );
    assert_eq!(
        verified(json!({ "limit": 1, "cursor": "2" })),
        json!({ "ours": [], "theirs": [theirs], "next_cursor": "3" })
    );
    assert_eq!(
        verified(json!({ "limit": 1, "cursor": "3" })),
        json!({ "ours": [third], "theirs": [] })
    );

    // Somebody else's lock is given up only with force: 403 without.
    let unlock = |id: &str, body: Value| {
        ask(
            &server,
            "POST",
            &at(&format!("/locks/{id}/unlock")),
            Some(body),
        )
    };
    let (status, said) = unlock("2", json!({ "ref": branch }));
    assert_eq!(status, 403);
    assert!(said["message"].is_string(), "{said}");
    assert_eq!(unlock("2", json!({ "force": false })).0, 403);
    assert_eq!(server.held().len(), 3);
    assert_eq!(
        unlock("2", json!({ "force": true, "ref": branch })),
        (200, json!({ "lock": theirs }))
    );
    // The user's own without.
    assert_eq!(unlock("1", json!({})), (200, json!({ "lock": first })));
    // One that is not there.
    assert_eq!(unlock("1", json!({})).0, 404);
    assert_eq!(unlock("nothing", json!({ "force": true })).0, 404);
    assert_eq!(
        server.held(),
        [("d ir/sp ace.bin".to_owned(), USER.to_owned())]
    );

    // Whatever else is asked is not there: the objects, another address.
    assert_eq!(
        ask(&server, "POST", &at("/objects/batch"), Some(json!({}))).0,
        404
    );
    assert_eq!(ask(&server, "GET", "/elsewhere/locks", None).0, 404);
    assert_eq!(ask(&server, "DELETE", &at("/locks/3"), None).0, 404);
    assert_eq!(server.complaints(), Vec::<String>::new());
    // And everything that was asked was kept.
    let asked = server.take_asked();
    assert_eq!(asked.len(), 27);
    assert_eq!(asked[0], get("/locks", &[]));
    assert_eq!(asked[1], verify());
    assert_eq!(asked[2], create("one.bin"));
    assert_eq!(asked[7], lookup("sub/two.bin"));
    assert_eq!(asked[8], get("/locks", &[("path", "d ir/sp ace.bin")]));
    assert_eq!(asked[20], delete("2", true));
    assert_eq!(asked[25].path, "/elsewhere/locks");
}

// ---------------------------------------------------------------------
// The commands.

#[test]
fn locks_are_made_listed_and_given_up_as_git_lfs_does_it() {
    if skipped("locks_are_made_listed_and_given_up_as_git_lfs_does_it") {
        return;
    }
    let ours = Bench::new();
    let twin = Twin::new();
    let same = |what: &str| assert_eq!(ours.standing(), twin.standing(), "{what}");
    same("at first");

    // Nothing is locked. Git-lfs prints an empty list also when it could
    // not ask, so an empty one is asked for a second time, in plain text.
    assert_eq!(ours.run("lfs.locks", json!({})), json!([]));
    assert_eq!(ours.server.take_asked(), [verify(), verify()]);
    assert_eq!(
        as_returned(&twin.lfs(&["locks", "--verify", "--json"]).unwrap()),
        json!([])
    );
    assert_eq!(twin.server.take_asked(), [verify()]);

    // A file is locked: for the user, and it may be written now.
    let made = ours.run("lfs.lock", json!({ "path": "one.bin" }));
    assert_eq!(made, lock(1, "one.bin", USER, true));
    assert_eq!(ours.server.take_asked(), [create("one.bin")]);
    twin.lfs(&["lock", "one.bin"]).unwrap();
    assert_eq!(twin.server.take_asked(), [create("one.bin")]);
    assert_eq!(
        ours.server.held(),
        [("one.bin".to_owned(), USER.to_owned())]
    );
    assert!(writable(&ours.copy, "one.bin"));
    same("one locked");

    // It is listed as the user's, and asked for once.
    let listed = ours.run("lfs.locks", json!({}));
    assert_eq!(listed, json!([lock(1, "one.bin", USER, true)]));
    assert_eq!(ours.server.take_asked(), [verify()]);
    assert_eq!(
        listed,
        as_returned(&twin.lfs(&["locks", "--verify", "--json"]).unwrap())
    );
    assert_eq!(twin.server.take_asked(), [verify()]);

    // Somebody else holds a lock: it is listed as theirs.
    for server in [&ours.server, &twin.server] {
        server.plant("sub/two.bin", OTHER);
    }
    let listed = ours.run("lfs.locks", json!({}));
    assert_eq!(
        listed,
        json!([
            lock(1, "one.bin", USER, true),
            lock(2, "sub/two.bin", OTHER, false),
        ])
    );
    assert_eq!(
        listed,
        as_returned(&twin.lfs(&["locks", "--verify", "--json"]).unwrap())
    );
    ours.server.take_asked();
    twin.server.take_asked();

    // Their lock is not given up without force: the server refuses, and
    // what it said is passed on.
    let err = ours.refused(
        "lfs.unlock",
        json!({ "path": "sub/two.bin" }),
        "sub/two.bin was not unlocked",
    );
    assert!(err.contains("somebody else's"), "{err}");
    let refused = [lookup("sub/two.bin"), delete("2", false)];
    assert_eq!(ours.server.take_asked(), refused);
    twin.lfs(&["unlock", "sub/two.bin"]).unwrap_err();
    assert_eq!(twin.server.take_asked(), refused);
    assert_eq!(ours.server.held().len(), 2);
    same("their lock refused");

    // With force it is.
    assert_eq!(
        ours.run(
            "lfs.unlock",
            json!({ "path": "sub/two.bin", "force": true })
        ),
        json!({ "path": "sub/two.bin", "unlocked": true })
    );
    let forced = [lookup("sub/two.bin"), delete("2", true)];
    assert_eq!(ours.server.take_asked(), forced);
    twin.lfs(&["unlock", "--force", "sub/two.bin"]).unwrap();
    assert_eq!(twin.server.take_asked(), forced);
    assert_eq!(
        ours.server.held(),
        [("one.bin".to_owned(), USER.to_owned())]
    );
    same("their lock forced");

    // The user's own is given up without, and the file is read-only
    // again.
    assert_eq!(
        ours.run("lfs.unlock", json!({ "path": "one.bin" })),
        json!({ "path": "one.bin", "unlocked": true })
    );
    let given_up = [lookup("one.bin"), delete("1", false)];
    assert_eq!(ours.server.take_asked(), given_up);
    twin.lfs(&["unlock", "one.bin"]).unwrap();
    assert_eq!(twin.server.take_asked(), given_up);
    assert_eq!(ours.server.held(), []);
    assert!(!writable(&ours.copy, "one.bin"));
    same("all given up");

    // And nothing is locked.
    assert_eq!(ours.run("lfs.locks", json!({})), json!([]));
    assert_eq!(ours.server.take_asked(), [verify(), verify()]);

    // With HEAD on no branch, that is what the server is told.
    ours.copy.git().run(["checkout", "-q", "--detach"]).unwrap();
    let made = ours.run("lfs.lock", json!({ "path": "three.bin" }));
    assert_eq!(made, lock(3, "three.bin", USER, true));
    assert_eq!(
        ours.server.take_asked(),
        [post(
            "/locks",
            json!({ "path": "three.bin", "ref": { "name": "HEAD" } })
        )]
    );

    // Every request was made the way the API asks for.
    assert_eq!(ours.server.complaints(), Vec::<String>::new());
    assert_eq!(twin.server.complaints(), Vec::<String>::new());
    // Nothing of what git-lfs said was taken for progress.
    assert_eq!(ours.reported(), Vec::<String>::new());
    // None of it is something to undo.
    assert!(ours.run("undo.peek", json!({}))["undo"].is_null());
}

/// A lock on a file that is locked already says whose the lock is, which
/// the server's words do not.
#[test]
fn locking_what_is_locked_already_says_who_holds_it() {
    if skipped("locking_what_is_locked_already_says_who_holds_it") {
        return;
    }
    let ours = Bench::new();
    ours.run("lfs.lock", json!({ "path": "one.bin" }));
    ours.server.plant("sub/two.bin", OTHER);
    ours.server.take_asked();
    let before = ours.standing();

    // The user's own: the server refuses, and is asked for the lock.
    let err = ours.refused(
        "lfs.lock",
        json!({ "path": "one.bin" }),
        "one.bin is already locked by Ada Stand-in since 2026-10-01T10:00:01Z",
    );
    assert!(!err.contains("exit status"), "{err}");
    assert_eq!(
        ours.server.take_asked(),
        [create("one.bin"), lookup("one.bin")]
    );
    // Somebody else's.
    ours.refused(
        "lfs.lock",
        json!({ "path": "sub/two.bin" }),
        "sub/two.bin is already locked by Bob Other since 2026-10-01T10:00:02Z",
    );
    assert_eq!(
        ours.server.take_asked(),
        [create("sub/two.bin"), lookup("sub/two.bin")]
    );
    assert_eq!(ours.standing(), before);

    // Git-lfs says no more than the server's words.
    let twin = Twin::new();
    twin.lfs(&["lock", "one.bin"]).unwrap();
    let said = twin.lfs(&["lock", "one.bin"]).unwrap_err();
    assert!(said.contains("the path is locked already"), "{said}");
    assert!(!said.contains(USER), "{said}");

    // A lock that is refused for another reason says that reason.
    ours.server.answer(Manner::Failing);
    let err = ours.refused(
        "lfs.lock",
        json!({ "path": "three.bin" }),
        "three.bin was not locked: the stand-in fails on purpose",
    );
    assert!(!err.contains("already locked"), "{err}");
    ours.server.answer(Manner::Proper);
    assert_eq!(ours.standing(), before);
    assert_eq!(ours.server.complaints(), Vec::<String>::new());
}

/// A user who may read and not push, a reporter at GitLab, may not verify
/// whose the locks are, which takes the right to push. The list itself
/// takes only the right to read: it is shown, without whose each lock is.
/// A lock is refused in the server's words.
#[test]
fn a_reader_is_shown_the_locks_without_whose_they_are() {
    if skipped("a_reader_is_shown_the_locks_without_whose_they_are") {
        return;
    }
    let ours = Bench::new();
    ours.server.plant("sub/two.bin", OTHER);
    ours.server.plant("one.bin", USER);
    ours.server.answer(Manner::Reader);
    let before = ours.standing();
    let listed = ours.run("lfs.locks", json!({}));
    assert_eq!(
        listed,
        json!([
            { "id": "1", "path": "sub/two.bin", "owner": OTHER, "locked_at": stamp(1), "ours": null },
            { "id": "2", "path": "one.bin", "owner": USER, "locked_at": stamp(2), "ours": null },
        ])
    );
    let asked = ours.server.take_asked();
    assert_eq!(asked.first(), Some(&verify()), "{asked:?}");
    assert_eq!(
        asked.last(),
        Some(&get("/locks", &[("refspec", BRANCH)])),
        "{asked:?}"
    );
    ours.refused(
        "lfs.lock",
        json!({ "path": "three.bin" }),
        "three.bin was not locked",
    );
    ours.refused(
        "lfs.unlock",
        json!({ "path": "sub/two.bin", "force": true }),
        "sub/two.bin was not unlocked",
    );
    assert_eq!(ours.standing(), before);

    // A server that refuses the list as well: why, as the first refusal
    // said it.
    ours.server.answer(Manner::Failing);
    ours.refused(
        "lfs.locks",
        json!({}),
        "the locks could not be listed: the stand-in fails on purpose",
    );
    assert_eq!(ours.server.complaints(), Vec::<String>::new());
}

/// github.com refuses a lock and an unlock to one whose role may push and
/// whose token may only read, the unlock in words that ask for push access
/// (which the role has), the lock in none. The refusals say what they
/// mean; the list, whose verification is refused in the same words, is
/// shown, without whose each lock is.
#[test]
fn a_token_that_may_only_read_is_shown_the_locks_and_told_why_not_more() {
    if skipped("a_token_that_may_only_read_is_shown_the_locks_and_told_why_not_more") {
        return;
    }
    let not_allowed = "The server did not let the account that signed in do this";
    let ours = Bench::new();
    ours.server.plant("sub/two.bin", OTHER);
    ours.server.answer(Manner::ReadingToken);
    let before = ours.standing();
    let listed = ours.run("lfs.locks", json!({}));
    assert_eq!(
        listed,
        json!([
            { "id": "1", "path": "sub/two.bin", "owner": OTHER, "locked_at": stamp(1), "ours": null },
        ])
    );
    let asked = ours.server.take_asked();
    assert_eq!(asked.first(), Some(&verify()), "{asked:?}");
    assert_eq!(
        asked.last(),
        Some(&get("/locks", &[("refspec", BRANCH)])),
        "{asked:?}"
    );
    let err = ours.refused(
        "lfs.lock",
        json!({ "path": "three.bin" }),
        &format!("three.bin was not locked: {not_allowed}"),
    );
    assert!(err.contains("Git-lfs said: Authorization error"), "{err}");
    // Understood, the refusal is not taken for a lock in the way: the
    // path is not looked up.
    assert_eq!(ours.server.take_asked(), [create("three.bin")]);
    let err = ours.refused(
        "lfs.unlock",
        json!({ "path": "sub/two.bin", "force": true }),
        &format!("sub/two.bin was not unlocked: {not_allowed}"),
    );
    assert!(
        err.ends_with("Git-lfs said: You must have push access to delete a lock"),
        "{err}"
    );
    assert_eq!(ours.standing(), before);
    assert_eq!(ours.server.complaints(), Vec::<String>::new());
}

/// git-lfs ends badly when it did not unlock: the reason it gives is not
/// lost behind the command line that failed, and a file that is not locked
/// is not said to be "still locked".
#[test]
fn a_file_that_is_not_unlocked_says_why() {
    if skipped("a_file_that_is_not_unlocked_says_why") {
        return;
    }
    let ours = Bench::new();
    let twin = Twin::new();
    let same = |what: &str| assert_eq!(ours.standing(), twin.standing(), "{what}");

    // A file nobody locked: the server is asked for its lock, and has
    // none.
    for force in [false, true] {
        let err = ours.refused(
            "lfs.unlock",
            json!({ "path": "one.bin", "force": force }),
            "one.bin is not locked",
        );
        assert!(!err.contains("exit status"), "{err}");
        assert_eq!(ours.server.take_asked(), [lookup("one.bin")]);
    }
    twin.lfs(&["unlock", "one.bin"]).unwrap_err();
    assert_eq!(twin.server.take_asked(), [lookup("one.bin")]);
    same("nothing to unlock");

    // A locked file with changes that are not committed: git-lfs refuses
    // by itself, before it asks the server.
    for (side, server) in [(&ours.copy, &ours.server), (&twin.copy, &twin.server)] {
        side.git().run(["lfs", "lock", "one.bin"]).unwrap();
        side.write("one.bin", "changed while it was locked\n")
            .unwrap();
        server.take_asked();
    }
    let err = ours.refused(
        "lfs.unlock",
        json!({ "path": "one.bin" }),
        "one.bin was not unlocked",
    );
    assert!(err.contains("uncommitted changes"), "{err}");
    assert_eq!(ours.server.take_asked(), []);
    twin.lfs(&["unlock", "one.bin"]).unwrap_err();
    assert_eq!(twin.server.take_asked(), []);
    assert_eq!(
        ours.server.held(),
        [("one.bin".to_owned(), USER.to_owned())]
    );
    same("changes in the way");

    // With force it is given up all the same, and the changes stay.
    assert_eq!(
        ours.run("lfs.unlock", json!({ "path": "one.bin", "force": true })),
        json!({ "path": "one.bin", "unlocked": true })
    );
    assert_eq!(
        ours.server.take_asked(),
        [lookup("one.bin"), delete("1", true)]
    );
    twin.lfs(&["unlock", "--force", "one.bin"]).unwrap();
    assert_eq!(ours.server.held(), []);
    assert_eq!(
        std::fs::read_to_string(ours.copy.path().join("one.bin")).unwrap(),
        "changed while it was locked\n"
    );
    same("forced with changes");
    assert_eq!(ours.server.complaints(), Vec::<String>::new());
}

/// Some of the names here are ones that only some file systems take:
/// Windows takes no `?` and no `"` in a name, and there two others that an
/// address has to quote stand in for them.
#[test]
fn paths_arrive_whole_and_many_locks_come_in_pages() {
    if skipped("paths_arrive_whole_and_many_locks_come_in_pages") {
        return;
    }
    let ours = Bench::new();
    // Paths that an address has to quote.
    let windows = cfg!(windows);
    let odd = [
        "spa ce/in a name.bin",
        "per%cent 100%.bin",
        "plus+and&equals=.bin",
        if windows {
            "semi;colon#hash.bin"
        } else {
            "quest?ion#hash.bin"
        },
        "ünï/cödé ✓.bin",
        if windows {
            "quo'te's.bin"
        } else {
            "quo\"te's.bin"
        },
    ];
    // Committed, or git-lfs would not give their locks up again: a file
    // it does not know has changes that are not committed.
    for path in odd {
        ours.copy.write(path, "odd\n").unwrap();
    }
    ours.copy.git().run(["add", "-A"]).unwrap();
    ours.copy
        .git()
        .run(["commit", "-q", "-m", "files with odd names"])
        .unwrap();
    assert_eq!(ours.server.take_asked(), []);
    for (at, path) in odd.iter().enumerate() {
        let id = u64::try_from(at).unwrap() + 1;
        assert_eq!(
            ours.run("lfs.lock", json!({ "path": path })),
            lock(id, path, USER, true),
            "{path}"
        );
        assert_eq!(ours.server.take_asked(), [create(path)], "{path}");
        // Locked again, it is found by its path.
        ours.refused(
            "lfs.lock",
            json!({ "path": path }),
            &format!("{path} is already locked by {USER}"),
        );
        assert_eq!(
            ours.server.take_asked(),
            [create(path), lookup(path)],
            "{path}"
        );
    }
    let all: Vec<Value> = odd
        .iter()
        .enumerate()
        .map(|(at, path)| lock(u64::try_from(at).unwrap() + 1, path, USER, true))
        .collect();
    assert_eq!(ours.run("lfs.locks", json!({})), Value::Array(all.clone()));
    assert_eq!(ours.server.take_asked(), [verify()]);

    // The server gives two locks at a time: every page is asked for, and
    // the user's locks come before the others'.
    ours.server.plant("theirs/first.bin", OTHER);
    ours.server.plant("theirs/second.bin", OTHER);
    ours.run("lfs.lock", json!({ "path": "one.bin" }));
    ours.server.take_asked();
    ours.server.page(Some(2));
    let mut expected = all;
    expected.push(lock(9, "one.bin", USER, true));
    expected.push(lock(7, "theirs/first.bin", OTHER, false));
    expected.push(lock(8, "theirs/second.bin", OTHER, false));
    assert_eq!(ours.run("lfs.locks", json!({})), Value::Array(expected));
    assert_eq!(
        ours.server.take_asked(),
        [
            verify(),
            verify_from("3"),
            verify_from("5"),
            verify_from("7"),
            verify_from("9"),
        ]
    );

    // Each is given up by its path, also while the server pages.
    for (at, path) in odd.iter().enumerate() {
        assert_eq!(
            ours.run("lfs.unlock", json!({ "path": path })),
            json!({ "path": path, "unlocked": true }),
            "{path}"
        );
        let id = (at + 1).to_string();
        assert_eq!(
            ours.server.take_asked(),
            [lookup(path), delete(&id, false)],
            "{path}"
        );
    }
    // A lock on a file that is not here is somebody's who has it: it is
    // given up with force like any other.
    assert_eq!(
        ours.run(
            "lfs.unlock",
            json!({ "path": "theirs/first.bin", "force": true })
        ),
        json!({ "path": "theirs/first.bin", "unlocked": true })
    );
    assert_eq!(
        ours.server.held(),
        [
            ("theirs/second.bin".to_owned(), OTHER.to_owned()),
            ("one.bin".to_owned(), USER.to_owned()),
        ]
    );
    assert_eq!(ours.server.complaints(), Vec::<String>::new());
}

#[test]
fn the_server_is_found_through_a_remote() {
    if skipped("the_server_is_found_through_a_remote") {
        return;
    }
    for found in [Found::Remote, Found::RemoteSetting] {
        let ours = Bench::found(found);
        assert_eq!(
            ours.run("lfs.lock", json!({ "path": "one.bin" })),
            lock(1, "one.bin", USER, true),
            "{found:?}"
        );
        ours.server.plant("three.bin", OTHER);
        assert_eq!(
            ours.run("lfs.locks", json!({})),
            json!([
                lock(1, "one.bin", USER, true),
                lock(2, "three.bin", OTHER, false),
            ]),
            "{found:?}"
        );
        assert_eq!(
            ours.run("lfs.unlock", json!({ "path": "one.bin" })),
            json!({ "path": "one.bin", "unlocked": true }),
            "{found:?}"
        );
        // All of it below the API's address, where the stand-in answers.
        assert_eq!(
            ours.server.take_asked(),
            [
                create("one.bin"),
                verify(),
                lookup("one.bin"),
                delete("1", false),
            ],
            "{found:?}"
        );
        assert_eq!(ours.server.complaints(), Vec::<String>::new());
    }
}

// ---------------------------------------------------------------------
// What must not get through, and what must not bring it down.

#[test]
fn hostile_lock_parameters_never_reach_the_server() {
    if skipped("hostile_lock_parameters_never_reach_the_server") {
        return;
    }
    let ours = Bench::new();
    ours.run("lfs.lock", json!({ "path": "one.bin" }));
    ours.server.plant("sub/two.bin", OTHER);
    ours.server.take_asked();
    let before = ours.standing();
    let outside = ours.copy.path().with_extension("outside");
    let long = "p".repeat(5000);

    for path in [
        "",
        "-f",
        "--force",
        "--json",
        "--id=1",
        "--path=one.bin",
        "--remote=elsewhere",
        "/",
        "/abs",
        "..",
        "../up",
        "a/../../up",
        "./one.bin",
        "one.bin/",
        "a//b",
        "a\nb",
        "a\rb",
        "nul\u{0}",
        ".git",
        ".git/config",
        ".GIT/config",
        outside.to_str().unwrap(),
        long.as_str(),
    ] {
        for (command, params) in [
            ("lfs.lock", json!({ "path": path })),
            ("lfs.unlock", json!({ "path": path })),
            ("lfs.unlock", json!({ "path": path, "force": true })),
        ] {
            assert!(
                ours.try_run(command, params).is_err(),
                "{command} accepted the path {path:?}"
            );
            assert_eq!(ours.server.take_asked(), [], "{command} {path:?}");
        }
    }
    // What is not a file here is not locked: nothing, a folder, a link.
    #[cfg(unix)]
    std::os::unix::fs::symlink("one.bin", ours.copy.path().join("link.bin")).unwrap();
    for path in ["nowhere.bin", "sub", "sub/nowhere.bin", "link.bin"] {
        if path == "link.bin" && !cfg!(unix) {
            continue;
        }
        ours.refused("lfs.lock", json!({ "path": path }), "is not a file here");
        assert_eq!(ours.server.take_asked(), [], "{path}");
    }
    for (command, params) in [
        ("lfs.lock", json!({})),
        ("lfs.lock", json!(null)),
        ("lfs.lock", json!({ "path": null })),
        ("lfs.lock", json!({ "path": 3 })),
        ("lfs.lock", json!({ "path": ["one.bin"] })),
        ("lfs.lock", json!({ "path": { "path": "one.bin" } })),
        ("lfs.unlock", json!({})),
        ("lfs.unlock", json!({ "force": true })),
        ("lfs.unlock", json!({ "path": true })),
        ("lfs.unlock", json!({ "path": "one.bin", "force": "yes" })),
        ("lfs.unlock", json!({ "path": "one.bin", "force": 1 })),
        ("lfs.unlock", json!({ "path": "one.bin", "force": [true] })),
    ] {
        assert!(
            ours.try_run(command, params.clone()).is_err(),
            "{command} {params}"
        );
        assert_eq!(ours.server.take_asked(), [], "{command} {params}");
    }
    // Parameters a command does not have are not in its way.
    assert_eq!(
        ours.run("lfs.locks", json!({ "path": "../up", "force": "yes" })),
        json!([
            lock(1, "one.bin", USER, true),
            lock(2, "sub/two.bin", OTHER, false),
        ])
    );
    assert_eq!(ours.server.take_asked(), [verify()]);

    assert_eq!(ours.standing(), before);
    assert!(!outside.exists());
    assert!(ours.run("undo.peek", json!({}))["undo"].is_null());
}

#[test]
fn a_server_that_answers_badly_gets_an_error_each_time() {
    if skipped("a_server_that_answers_badly_gets_an_error_each_time") {
        return;
    }
    let ours = Bench::new();
    ours.run("lfs.lock", json!({ "path": "one.bin" }));
    ours.server.plant("sub/two.bin", OTHER);
    ours.server.take_asked();
    let before = ours.standing();

    for manner in [
        Manner::Failing,
        Manner::FailingPage,
        Manner::Garbage,
        Manner::EmptyBody,
        Manner::WrongShape,
        Manner::CutShort,
        Manner::Hangup,
    ] {
        ours.server.answer(manner);
        for (command, params, names) in [
            ("lfs.locks", json!({}), "the locks could not be listed: "),
            (
                "lfs.lock",
                json!({ "path": "three.bin" }),
                "three.bin was not locked: ",
            ),
            (
                "lfs.unlock",
                json!({ "path": "one.bin" }),
                "one.bin was not unlocked: ",
            ),
            (
                "lfs.unlock",
                json!({ "path": "sub/two.bin", "force": true }),
                "sub/two.bin was not unlocked: ",
            ),
        ] {
            let began = Instant::now();
            let err = ours
                .try_run(command, params.clone())
                .expect_err(&format!("{manner:?}: {command} {params} went well"));
            assert!(began.elapsed() < PROMPT, "{manner:?}: {command} took long");
            // It says what was not done and something about why, and not
            // the command line that failed.
            let (_, why) = err
                .split_once(names)
                .unwrap_or_else(|| panic!("{manner:?}: {command}: {err}"));
            assert!(!why.trim().is_empty(), "{manner:?}: {command}: {err}");
            assert!(!err.contains("exit status"), "{manner:?}: {command}: {err}");
            assert!(!err.contains("cancelled"), "{manner:?}: {command}: {err}");
            if manner == Manner::Failing {
                // What the server gave as the reason is passed on.
                assert!(
                    err.contains("the stand-in fails on purpose"),
                    "{command}: {err}"
                );
            }
            // The server was asked, and that is what failed.
            assert!(
                !ours.server.take_asked().is_empty(),
                "{manner:?}: {command} did not ask"
            );
        }
        assert_eq!(ours.standing(), before, "{manner:?}");
    }

    // Why git-lfs failed is not a stage of progress, even when it reads
    // like one ("Error while retrieving locks: …").
    assert_eq!(ours.reported(), Vec::<String>::new());

    // A server that answers everything with an empty object keeps no
    // locks, as far as anyone can tell: none are listed, none is made,
    // and there is none to give up.
    ours.server.answer(Manner::Hollow);
    assert_eq!(ours.run("lfs.locks", json!({})), json!([]));
    ours.refused(
        "lfs.lock",
        json!({ "path": "three.bin" }),
        "three.bin was not locked: ",
    );
    ours.refused(
        "lfs.unlock",
        json!({ "path": "one.bin" }),
        "one.bin is not locked",
    );
    assert_eq!(ours.standing(), before);

    // Once it answers again, everything works as before.
    ours.server.answer(Manner::Proper);
    ours.server.take_asked();
    assert_eq!(
        ours.run("lfs.locks", json!({})),
        json!([
            lock(1, "one.bin", USER, true),
            lock(2, "sub/two.bin", OTHER, false),
        ])
    );
    assert_eq!(
        ours.run("lfs.lock", json!({ "path": "three.bin" })),
        lock(3, "three.bin", USER, true)
    );
    assert_eq!(
        ours.run("lfs.unlock", json!({ "path": "one.bin" })),
        json!({ "path": "one.bin", "unlocked": true })
    );
}

/// Set the flag that stops a command once the server has been asked
/// `after` times, or when that takes too long to come.
fn stop_once_asked(ours: &Bench, after: usize) -> JoinHandle<bool> {
    let cancel = ours.ctx.borrow().cancel.clone();
    let kept = ours.server.kept.clone();
    std::thread::spawn(move || {
        let began = Instant::now();
        let mut asked = false;
        while !asked && began.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(10));
            asked = kept.lock().unwrap().asked.len() >= after;
        }
        cancel.store(true, Ordering::SeqCst);
        asked
    })
}

#[test]
fn a_server_that_never_answers_is_given_up_on_or_stopped() {
    if skipped("a_server_that_never_answers_is_given_up_on_or_stopped") {
        return;
    }
    let ours = Bench::new();
    ours.run("lfs.lock", json!({ "path": "one.bin" }));
    ours.server.take_asked();
    let before = ours.standing();
    let commands = [
        ("lfs.locks", json!({})),
        ("lfs.lock", json!({ "path": "three.bin" })),
        ("lfs.unlock", json!({ "path": "one.bin" })),
    ];

    // Asked to stop while the server keeps silent, a command ends at
    // once, with git-lfs: it would wait half a minute by itself.
    ours.server.answer(Manner::Silent);
    for (command, params) in commands.clone() {
        let stopper = stop_once_asked(&ours, 1);
        let began = Instant::now();
        let err = ours.try_run(command, params).unwrap_err();
        assert!(stopper.join().unwrap(), "{command} never asked the server");
        assert!(err.contains("cancelled"), "{command}: {err}");
        assert!(
            began.elapsed() < Duration::from_secs(20),
            "{command} was not stopped: {:?}",
            began.elapsed()
        );
        ours.ctx.borrow().cancel.store(false, Ordering::SeqCst);
        assert_eq!(ours.server.take_asked().len(), 1, "{command}");
    }

    // Left alone, git-lfs gives up when nothing has come for as long as
    // it is set to wait, and the command fails with what it said.
    ours.copy
        .git()
        .run(["config", "--local", "lfs.activitytimeout", "1"])
        .unwrap();
    for (command, params) in commands.clone() {
        let began = Instant::now();
        let err = ours.try_run(command, params).unwrap_err();
        assert!(!err.contains("cancelled"), "{command}: {err}");
        assert!(
            began.elapsed() < PROMPT,
            "{command} took {:?}",
            began.elapsed()
        );
        assert!(!ours.server.take_asked().is_empty(), "{command}");
    }
    ours.copy
        .git()
        .run(["config", "--local", "--unset", "lfs.activitytimeout"])
        .unwrap();

    // A server that has one more page of locks each time it is asked
    // would be listed for ever: that is stopped as well.
    ours.server.answer(Manner::Endless);
    let stopper = stop_once_asked(&ours, 5);
    let began = Instant::now();
    let err = ours.try_run("lfs.locks", json!({})).unwrap_err();
    assert!(stopper.join().unwrap(), "the pages were not asked for");
    assert!(err.contains("cancelled"), "{err}");
    assert!(began.elapsed() < Duration::from_secs(20));
    ours.ctx.borrow().cancel.store(false, Ordering::SeqCst);

    // Nothing was changed by any of it, and it works again afterwards.
    ours.server.answer(Manner::Proper);
    ours.server.take_asked();
    assert_eq!(ours.standing(), before);
    assert_eq!(
        ours.run("lfs.locks", json!({})),
        json!([lock(1, "one.bin", USER, true)])
    );
}

/// Every file under `dir` with its bytes.
fn files_under(dir: &Path) -> Vec<(String, Vec<u8>)> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, Vec<u8>)>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                out.push((
                    path.strip_prefix(base).unwrap().display().to_string(),
                    std::fs::read(&path).unwrap_or_default(),
                ));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// A directory beside a copy that is removed when this goes.
struct Beside(PathBuf);

impl Drop for Beside {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Listing locks is a read, and a bare repository is opened read-only, but
/// git-lfs keeps what the server answered in the repository each time it
/// lists them (`lfs/lockcache.db` and a file under `lfs/cache/locks`; 3.8.0
/// does). So a bare repository is not asked.
#[test]
fn listing_locks_leaves_a_bare_repository_as_it_is() {
    if skipped("listing_locks_leaves_a_bare_repository_as_it_is") {
        return;
    }
    let server = Server::start(USER);
    let copy = prepared(&server, Found::LfsUrl);
    // A bare copy that has the stand-in as its LFS server, and in which
    // git-lfs has been before, so that it has no settings left to write.
    let bare_copy = |name: &str| {
        let bare = Beside(copy.path().with_extension(name));
        let _ = std::fs::remove_dir_all(&bare.0);
        copy.git()
            .run([
                "clone",
                "-q",
                "--bare",
                copy.path().to_str().unwrap(),
                bare.0.to_str().unwrap(),
            ])
            .unwrap();
        keep_hooks_here(&bare.0, &bare.0);
        let git = git_at(&bare.0);
        pin(&git);
        git.run(["config", "--local", "lfs.url", &server.url()])
            .unwrap();
        git.run(["config", "--local", "lfs.repositoryformatversion", "0"])
            .unwrap();
        bare
    };
    let bare = bare_copy("bare.git");
    server.plant("one.bin", OTHER);
    let before = files_under(&bare.0);

    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": bare.0 }))
        .unwrap();
    let err = registry
        .run("lfs.locks", &mut ctx, &json!({}))
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("bare repository") && err.contains("read-only"),
        "{err}"
    );
    assert_eq!(server.take_asked(), []);
    let after = files_under(&bare.0);
    assert_eq!(
        after.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        before.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );
    assert!(after == before, "a file of the bare repository changed");
    // There is no file to lock or to unlock in one either.
    for (command, params) in [
        ("lfs.lock", json!({ "path": "one.bin" })),
        ("lfs.unlock", json!({ "path": "one.bin", "force": true })),
    ] {
        let err = registry
            .run(command, &mut ctx, &params)
            .unwrap_err()
            .to_string();
        assert!(err.contains("bare repository"), "{command}: {err}");
    }
    assert_eq!(server.take_asked(), []);
    assert!(files_under(&bare.0) == before);
    assert_eq!(server.held(), [("one.bin".to_owned(), OTHER.to_owned())]);

    // Why it is not asked: git-lfs itself, asked the same in a bare copy
    // of its own, leaves files in it. Should a later git-lfs no longer do
    // so, this fails, and the refusal can go.
    let twin = bare_copy("twin.git");
    let names = |dir: &Path| -> Vec<String> {
        files_under(dir).into_iter().map(|(name, _)| name).collect()
    };
    let had = names(&twin.0);
    git_at(&twin.0)
        .run(["lfs", "locks", "--verify", "--json"])
        .unwrap();
    assert_eq!(server.take_asked(), [verify()]);
    let left: Vec<String> = names(&twin.0)
        .into_iter()
        .filter(|name| !had.contains(name))
        .collect();
    assert!(
        !left.is_empty() && left.iter().all(|name| Path::new(name).starts_with("lfs")),
        "git-lfs left {left:?} in a bare repository"
    );

    // In the copy that has files, the same server is listed.
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
        .unwrap();
    assert_eq!(
        registry.run("lfs.locks", &mut ctx, &json!({})).unwrap(),
        json!([lock(1, "one.bin", OTHER, false)])
    );
}
