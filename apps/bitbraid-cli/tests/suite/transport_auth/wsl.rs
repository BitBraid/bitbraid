//! A repository inside a WSL distribution, opened through the
//! distribution's share and worked with by the distribution's own git
//! through `wsl.exe`. Here, beside the smart-HTTP server and the askpass
//! of the module above, because the first thing it does is a fetch that
//! asks who is there.
//!
//! The repository is cloned inside the distribution, under its `/tmp`,
//! from a bare repository on the Windows side; its remote is the test's
//! server on the loopback interface, which a distribution shares with
//! Windows. It has a symbolic link, an executable file and a `post-commit`
//! hook, all of them Linux's. Through the share (`\\wsl$\<name>\tmp\…`)
//! the engine:
//!
//! - fetches, with the user and the token asked through the binary run by
//!   the distribution's git as its askpass program, and nothing kept;
//! - carries out an interactive rebase (a reword and a drop), its todo
//!   list and the new message handed to the distribution's git;
//! - commits, and the hook runs inside the distribution;
//! - reads the status, from the distribution's git (gitoxide through the
//!   share is over ten times as slow for these two thousand files): the
//!   link and the executable file are not changes, a changed file is one,
//!   and it takes less than a second;
//! - fetches from a server that never answers, is cancelled, and leaves
//!   no git running inside the distribution;
//! - opens the same repository by the share's other name
//!   (`\\wsl.localhost\…`) and commits there.
//!
//! WSL 2 runs a machine of its own behind Windows' address translation:
//! there the servers listen on Windows' end of its virtual network (the
//! way out of the distribution's default route), which Windows' firewall
//! has to let it reach; WSL 1 shares Windows' network and the loopback.
//!
//! Opt-in: `BITBRAID_WSL_DISTRIBUTION` names a distribution that has git
//! and a POSIX sh. Without it the test says it was skipped. Nothing is
//! made outside the scratch folders and the distribution's
//! `/tmp/bitbraid-wsl-<pid>`, which is removed afterwards.
//!
//! Sources: Microsoft Learn, "Working across Windows and Linux file
//! systems" (`\\wsl$`), "Basic commands for WSL" (`--distribution`,
//! `--exec`), "Share environment variables between WSL and Windows"
//! (`WSLENV`); git-rebase(1), githooks(5), gitcredentials(7).
#![cfg(windows)]
#![allow(clippy::print_stdout)]

use std::{
    io::Read as _,
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_git::askpass::Kind;
use serde_json::{Value, json};

use super::{Asking, Http, LOOPBACK, ONE, Site, TOKEN, USER, listening_at};

/// The distribution the test is asked to run in.
fn distribution() -> Option<String> {
    std::env::var("BITBRAID_WSL_DISTRIBUTION")
        .ok()
        .filter(|name| !name.trim().is_empty())
}

/// What sh prints for `script` inside the distribution; a failure fails
/// the test.
fn inside(distribution: &str, script: &str) -> String {
    let out = Command::new("wsl.exe")
        .args(["--distribution", distribution, "--exec", "sh", "-c", script])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{script}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A single-quoted word for sh.
fn sh(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// The distribution's folder of the test, removed when it ends.
struct Place {
    distribution: String,
    root: String,
}

impl Drop for Place {
    fn drop(&mut self) {
        let _ = Command::new("wsl.exe")
            .args(["--distribution", &self.distribution, "--exec", "rm", "-rf"])
            .arg(&self.root)
            .stdin(Stdio::null())
            .output();
    }
}

/// Where the distribution reaches Windows: on WSL 2, a machine of its own
/// behind Windows' address translation, the way out of its default route
/// (Windows' end of the virtual network, where a server of the test has
/// to listen); on WSL 1, which shares Windows' network, the loopback.
fn windows_seen_from(distribution: &str) -> std::net::IpAddr {
    if !inside(distribution, "uname -r").contains("WSL2") {
        return LOOPBACK;
    }
    let route = inside(distribution, "ip route show default");
    route
        .split_whitespace()
        .skip_while(|word| *word != "via")
        .nth(1)
        .and_then(|address| address.parse().ok())
        .unwrap_or_else(|| panic!("no way out in {route:?}"))
}

/// A server that takes connections and never answers.
struct Silent {
    host: std::net::IpAddr,
    port: u16,
    stop: Arc<AtomicBool>,
}

impl Silent {
    fn start(host: std::net::IpAddr) -> Self {
        let listener = TcpListener::bind((host, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for stream in listener.incoming() {
                if stopped.load(Ordering::SeqCst) {
                    break;
                }
                if let Ok(mut stream) = stream {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(50)));
                    let _ = stream.read(&mut [0; 4096]);
                    held.push(stream);
                }
            }
        });
        Self { host, port, stop }
    }
}

impl Drop for Silent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = std::net::TcpStream::connect((self.host, self.port));
    }
}

fn subjects(distribution: &str, work: &str, count: usize) -> Vec<String> {
    inside(
        distribution,
        &format!("git -C {} log --format=%s -{count}", sh(work)),
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

#[test]
fn a_repository_inside_wsl_is_worked_with_by_the_distributions_git() {
    let Some(distribution) = distribution() else {
        println!("skipped: BITBRAID_WSL_DISTRIBUTION names no distribution");
        return;
    };
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if std::env::var_os("GIT_ASKPASS").is_some() {
        println!("skipped: GIT_ASKPASS is set and would be asked instead");
        return;
    }
    let site = Site::new();
    let host = windows_seen_from(&distribution);
    let Some(http) = Http::start_on(site.bare.parent().unwrap(), host) else {
        println!("skipped: git has no http-backend here");
        return;
    };
    assert!(listening_at(host, http.port, Duration::from_secs(5)));
    let place = Place {
        distribution: distribution.clone(),
        root: format!("/tmp/bitbraid-wsl-{}", std::process::id()),
    };
    let work = format!("{}/work", place.root);
    let bare = bitbraid_core::wsl::linux_view(site.bare.to_str().unwrap(), &distribution)
        .expect("the bare repository is on a drive");
    let made = format!(
        "set -e\n\
         rm -rf {root}\nmkdir -p {root}\ncd {root}\n\
         git -c safe.directory='*' clone -q {bare} work\ncd work\n\
         git remote set-url origin {url}\n\
         git config user.name 'Ada Fixture'\n\
         git config user.email ada@fixture.invalid\n\
         git config credential.helper ''\n\
         git config http.proxy ''\n\
         printf 'target\\n' > target.txt\nln -s target.txt link\n\
         printf '#!/bin/sh\\necho tool\\n' > tool.sh\nchmod +x tool.sh\n\
         mkdir many\ni=0\nwhile [ $i -lt 2000 ]; do printf '%s\\n' $i > many/f$i.txt; i=$((i+1)); done\n\
         git add -A\ngit commit -q -m 'a link, a tool and many files'\n\
         for n in 1 2 3; do printf '%s\\n' $n >> steps.txt; git add steps.txt; git commit -q -m \"step $n\"; done\n\
         printf '#!/bin/sh\\nuname -s > \"$(git rev-parse --git-dir)/hook-said\"\\n' > .git/hooks/post-commit\n\
         chmod +x .git/hooks/post-commit\n",
        root = sh(&place.root),
        bare = sh(&bare),
        url = sh(&http.url(&site.bare)),
    );
    inside(&distribution, &made);
    let upstream = site.upstream_commit("upstream.txt");

    let share = PathBuf::from(format!(r"\\wsl$\{distribution}{}", work.replace('/', "\\")));
    let registry: Registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": share }))
        .unwrap_or_else(|err| panic!("open {}: {err}", share.display()));
    let mut run = |id: &str, params: Value| registry.run(id, &mut ctx, &params);

    // Fetch: asked through the binary, which the distribution's git runs.
    let asking = Asking::with(|kind, prompt| {
        Some(match kind {
            Kind::Text if prompt.starts_with("Username for") => USER.to_owned(),
            Kind::Secret if prompt.starts_with("Password for") => TOKEN.to_owned(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    });
    run("remote.fetch", json!({ "remote": "origin" })).unwrap_or_else(|err| panic!("fetch: {err}"));
    let tracking = inside(
        &distribution,
        &format!("git -C {} rev-parse refs/remotes/origin/main", sh(&work)),
    );
    assert_eq!(tracking.trim(), upstream);
    assert_eq!(asking.count("Username for"), 1, "{:?}", asking.asked());
    assert_eq!(asking.count("Password for"), 1, "{:?}", asking.asked());
    let config = inside(&distribution, &format!("cat {}/.git/config", sh(&work)));
    assert!(!config.contains(TOKEN), "{config}");

    // An interactive rebase: the second step reworded, the third dropped.
    let ids: Vec<String> = inside(
        &distribution,
        &format!("git -C {} rev-list --reverse -3 HEAD", sh(&work)),
    )
    .lines()
    .map(str::to_owned)
    .collect();
    assert_eq!(ids.len(), 3, "{ids:?}");
    run(
        "rebase.interactive",
        json!({
            "first": ids[0],
            "steps": [
                { "id": ids[0], "action": "pick" },
                { "id": ids[1], "action": "reword", "message": "step two, said anew\n" },
                { "id": ids[2], "action": "drop" },
            ],
        }),
    )
    .unwrap_or_else(|err| panic!("rebase: {err}"));
    assert_eq!(
        subjects(&distribution, &work, 3),
        [
            "step two, said anew",
            "step 1",
            "a link, a tool and many files"
        ]
    );
    let steps = inside(&distribution, &format!("cat {}/steps.txt", sh(&work)));
    assert_eq!(steps, "1\n2\n");

    // A commit, and its hook run by the distribution.
    inside(
        &distribution,
        &format!(
            "rm -f {work}/.git/hook-said; printf 'new\\n' > {work}/new.txt",
            work = sh(&work)
        ),
    );
    run(
        "commit.create",
        json!({ "message": "with a hook", "stage_all": true }),
    )
    .unwrap_or_else(|err| panic!("commit: {err}"));
    assert_eq!(subjects(&distribution, &work, 1), ["with a hook"]);
    let said = inside(&distribution, &format!("cat {}/.git/hook-said", sh(&work)));
    assert_eq!(said.trim(), "Linux");

    // The status through the share: the link and the executable file are
    // no changes; a changed file is one.
    let paths = |status: &Value| -> Vec<String> {
        status
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .filter_map(|entry| entry["path"].as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let status = run("repo.status", json!({})).unwrap_or_else(|err| panic!("status: {err}"));
    assert_eq!(paths(&status), Vec::<String>::new(), "{status}");
    inside(
        &distribution,
        &format!("printf 'changed\\n' >> {}/target.txt", sh(&work)),
    );
    let status = run("repo.status", json!({})).unwrap_or_else(|err| panic!("status: {err}"));
    assert_eq!(paths(&status), ["target.txt"], "{status}");
    let mut took = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        run("repo.status", json!({})).unwrap();
        took.push(started.elapsed());
    }
    took.sort();
    let started = Instant::now();
    inside(
        &distribution,
        &format!("git -C {} status --porcelain >/dev/null", sh(&work)),
    );
    let by_git = started.elapsed();
    let median = took.get(2).copied().unwrap_or_default();
    println!(
        "status of {} files: {:.0} ms (the median of five); \
         git status inside, wsl.exe included: {:.0} ms",
        2000 + 5,
        median.as_secs_f64() * 1000.0,
        by_git.as_secs_f64() * 1000.0
    );
    assert!(median < Duration::from_secs(1), "{median:?}");

    // A fetch from a server that never answers, cancelled.
    let silent = Silent::start(host);
    let at = format!("{host}:{}", silent.port);
    inside(
        &distribution,
        &format!(
            "git -C {} remote add silent http://{at}/never.git",
            sh(&work)
        ),
    );
    drop(run);
    let flag = ctx.cancel.clone();
    let pulled = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        flag.store(true, Ordering::SeqCst);
    });
    let started = Instant::now();
    let fetched = registry.run("remote.fetch", &mut ctx, &json!({ "remote": "silent" }));
    let waited = started.elapsed();
    pulled.join().unwrap();
    assert!(fetched.is_err(), "{fetched:?}");
    assert!(waited < Duration::from_secs(30), "{waited:?}");
    // Nothing of it is left running inside.
    let mut left = String::new();
    for _ in 0..50 {
        left = inside(&distribution, "ps -o args 2>/dev/null || ps");
        if !left.contains(&at) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(!left.contains(&at), "{left}");
    ctx.cancel.store(false, Ordering::SeqCst);

    // The same repository by the share's other name: read, and a commit
    // made by the distribution's git.
    let other = PathBuf::from(format!(
        r"\\wsl.localhost\{distribution}{}",
        work.replace('/', "\\")
    ));
    let mut again = CommandContext::default();
    registry
        .run("repo.open", &mut again, &json!({ "path": other }))
        .unwrap_or_else(|err| panic!("open {}: {err}", other.display()));
    registry
        .run("repo.status", &mut again, &json!({}))
        .unwrap_or_else(|err| panic!("status: {err}"));
    registry
        .run(
            "commit.create",
            &mut again,
            &json!({ "message": "by the share's other name", "stage_all": true }),
        )
        .unwrap_or_else(|err| panic!("commit: {err}"));
    assert_eq!(
        subjects(&distribution, &work, 1),
        ["by the share's other name"]
    );
    drop(place);
}
