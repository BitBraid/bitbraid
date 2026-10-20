//! The LFS commands over TLS, on this machine: an LFS server's
//! certificate is trusted when the repository names the authority that
//! signed it (`http.sslCAInfo`), and refused, in the application's words,
//! when it does not: with the system's authorities alone, and with
//! another authority's certificate named. Git's own fetch from the same
//! address is refused in the same words.
//!
//! The server is a stand-in of a few lines that keeps one lock, behind
//! the HTTPS front of `tls_front`; the live test against a GitLab
//! (`live_lfs`) runs the same through a real one. A refused handshake
//! never reaches the stand-in, which the front counts.
//!
//! Without git-lfs installed the LFS half says that it was skipped. The
//! words of git's own refusal are curl's with OpenSSL, held to on Linux.
//!
//! Sources: git-config(1) (`http.sslCAInfo`, `lfs.url` in
//! git-lfs-config(5)); git-lfs's `docs/api/locking.md` (listing locks, and
//! listing them for verification); RFC 9112.
#![allow(clippy::print_stderr)]

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, scratch_root};
use serde_json::{Value, json};

use crate::ASKPASS_SERVER as ONE;
use crate::tls_front::{Authority, Front, Passing};

const LOCK: &str =
    r#"{"id":"7","path":"a.bin","owner":{"name":"Bea Holder"},"locked_at":"2026-10-17T06:00:00Z"}"#;

/// A plain HTTP stand-in for an LFS server that keeps one lock of
/// somebody else's, on a free port; one answer per connection.
fn stand_in() -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    thread::spawn(move || {
        for socket in listener.incoming() {
            let Ok(socket) = socket else { continue };
            thread::spawn(move || {
                let _ = socket.set_read_timeout(Some(Duration::from_secs(10)));
                let mut reader = BufReader::new(socket);
                let mut first = String::new();
                if reader.read_line(&mut first).is_err() {
                    return;
                }
                let mut length = 0usize;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let line = line.trim_end();
                    if line.is_empty() {
                        break;
                    }
                    if let Some((name, value)) = line.split_once(':')
                        && name.eq_ignore_ascii_case("content-length")
                    {
                        length = value.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length.min(1 << 20)];
                let _ = reader.read_exact(&mut body);
                let (status, answer) =
                    if first.starts_with("POST ") && first.contains("/locks/verify") {
                        ("200 OK", format!(r#"{{"ours":[],"theirs":[{LOCK}]}}"#))
                    } else if first.starts_with("GET ") && first.contains("/locks") {
                        ("200 OK", format!(r#"{{"locks":[{LOCK}]}}"#))
                    } else {
                        ("404 Not Found", r#"{"message":"not here"}"#.to_owned())
                    };
                let mut socket = reader.into_inner();
                let _ = socket.write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/vnd.git-lfs+json\r\n\
                         Content-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                        answer.len()
                    )
                    .as_bytes(),
                );
            });
        }
    });
    address
}

fn has_lfs() -> bool {
    Command::new("git")
        .args(["lfs", "version"])
        .output()
        .is_ok_and(|out| out.status.success())
}

struct Site {
    dir: PathBuf,
    registry: Registry,
    ctx: CommandContext,
}

impl Site {
    /// A repository whose LFS server and remote are at `origin`, with an
    /// LFS file's pointer committed (its content is nowhere).
    fn new(origin: &str) -> Self {
        let dir = scratch_root().join(format!(
            "lfs-tls-{}-{}",
            std::process::id(),
            crate::live_lfs::fresh("lfs-tls")
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = Git::new(&dir, dir.join(".home"));
        git.ensure_home().unwrap();
        git.run(["init", "-q", "-b", "main"]).unwrap();
        crate::live_lfs::pin(&git, None);
        for (key, value) in [
            ("lfs.url", format!("{origin}/r.git/info/lfs")),
            ("lfs.activitytimeout", "20".to_owned()),
        ] {
            git.run(["config", key, value.as_str()]).unwrap();
        }
        git.run(["remote", "add", "origin", &format!("{origin}/r.git")])
            .unwrap();
        std::fs::write(
            dir.join(".gitattributes"),
            "*.bin filter=lfs diff=lfs merge=lfs -text lockable\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("a.bin"),
            "version https://git-lfs.github.com/spec/v1\n\
             oid sha256:dace82371525b81729329d6d579bc783ef95cd6a25f635cf1efc594862dbe52c\n\
             size 11\n",
        )
        .unwrap();
        git.run(["add", ".gitattributes", "a.bin"]).unwrap();
        git.run(["commit", "-q", "-m", "a pointer"]).unwrap();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": dir }))
            .unwrap();
        Self { dir, registry, ctx }
    }

    fn git(&self) -> Git {
        Git::new(&self.dir, self.dir.join(".home"))
    }

    fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }

    fn trust(&self, authority: Option<&Path>) {
        let git = self.git();
        match authority {
            Some(file) => git
                .run(["config", "http.sslCAInfo", file.to_str().unwrap()])
                .unwrap(),
            None => git
                .run(["config", "--unset-all", "http.sslCAInfo"])
                .unwrap_or_default(),
        };
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const REFUSED: &str = "The server's certificate was not accepted, so nothing was sent to it";

#[test]
fn an_lfs_server_over_tls_is_trusted_through_the_repository_and_refused_otherwise() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let authority = Authority::new("BitBraid test authority");
    let front = Front::start(
        Some(authority.server()),
        Passing {
            upstream: stand_in(),
            rewrite: None,
            rate: None,
        },
    );
    let mut site = Site::new(&front.origin());
    let ca = site.dir.join("authority.pem");
    std::fs::write(&ca, authority.pem()).unwrap();
    let other = Authority::new("Another authority");
    let other_ca = site.dir.join("other.pem");
    std::fs::write(&other_ca, other.pem()).unwrap();
    let lfs = has_lfs();
    if !lfs {
        eprintln!("the LFS half is skipped: git-lfs is not installed");
    }

    // Trusted: the list comes, somebody else's lock in it.
    if lfs {
        site.trust(Some(&ca));
        let listed = site.run("lfs.locks", json!({})).unwrap();
        assert_eq!(listed[0]["path"], "a.bin", "{listed}");
        assert_eq!(listed[0]["owner"], "Bea Holder", "{listed}");
        assert_eq!(listed[0]["ours"], false, "{listed}");
        assert!(
            front
                .heard()
                .iter()
                .any(|line| line.contains("/locks/verify")),
            "{:?}",
            front.heard()
        );
        assert_eq!(front.handshakes_failed(), 0);
    }

    // Not trusted: refused in words, and nothing reaches the server.
    for authority in [None, Some(other_ca.as_path())] {
        site.trust(authority);
        let heard = front.heard().len();
        let failed = front.handshakes_failed();
        let mut asked: Vec<(&str, Value)> = Vec::new();
        if lfs {
            asked.extend([
                ("lfs.locks", json!({})),
                ("lfs.lock", json!({ "path": "a.bin" })),
                ("lfs.unlock", json!({ "path": "a.bin", "force": true })),
                ("lfs.fetch", json!({})),
            ]);
        }
        if cfg!(target_os = "linux") {
            asked.push(("remote.fetch", json!({ "remote": "origin" })));
        }
        let any = !asked.is_empty();
        for (id, params) in asked {
            let err = site
                .run(id, params)
                .expect_err("an untrusted server was let through");
            assert!(err.contains(REFUSED), "{id} ({authority:?}): {err}");
            assert!(err.contains("http.sslCAInfo"), "{id}: {err}");
        }
        assert_eq!(front.heard().len(), heard, "{:?}", front.heard());
        if any {
            assert!(front.handshakes_failed() > failed);
        }
    }
}
