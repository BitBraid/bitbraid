//! Ending sessions: killed, dropped, or a shell that will not hang up.
//! Afterwards no shell is left, none is a zombie, and no thread of a
//! session is left running. One test in a binary of its own, so that the
//! threads counted are only its own.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
#![cfg(target_os = "linux")]

mod common;

use std::time::{Duration, Instant};

use bitbraid_terminal::{GridSize, SessionEvent};
use common::{Home, Running, spec, start};

/// The threads of this process a session runs.
fn session_threads() -> usize {
    std::fs::read_dir("/proc/self/task")
        .unwrap()
        .filter_map(|task| std::fs::read_to_string(task.ok()?.path().join("comm")).ok())
        .filter(|name| name.starts_with("PTY reader") || name.starts_with("bitbraid-term"))
        .count()
}

/// Whether a process is there at all, a zombie included.
fn there(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

fn until(what: &str, test: impl Fn() -> bool) {
    let started = Instant::now();
    while !test() {
        if started.elapsed() > Duration::from_secs(10) {
            panic!("never: {what}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn ended_sessions_leave_nothing_behind() {
    let home = Home::new("ending");
    assert_eq!(session_threads(), 0);

    // Killed: the shell and the job it runs go, Exited is said. The job
    // goes because the shell hangs up on it, as bash does; dash (the sh
    // of Debian and Ubuntu) leaves its jobs running, in any terminal.
    let running = Running::bash(&home);
    let shell = running.session.pid().unwrap();
    running.type_line("sleep 1000 & echo JOB $!");
    let frame = running.until("the job", |frame| {
        frame.rows.iter().any(|row| row.text().starts_with("JOB "))
    });
    let job: u32 = frame
        .rows
        .iter()
        .find_map(|row| row.text().strip_prefix("JOB ")?.trim().parse().ok())
        .unwrap();
    assert_eq!(session_threads(), 2);
    running.session.kill();
    assert_eq!(running.wait_exit(), None, "hung up on, not exited");
    until("the shell is reaped", || !there(shell));
    until("its job is gone", || !there(job));
    until("its threads end", || session_threads() == 0);
    assert!(running.session.exited().is_some());
    // Killing again, typing, asking for frames: nothing happens.
    running.session.kill();
    running.type_line("echo nobody");
    let _ = running.frame();

    // Dropped: the same, without a word to it.
    let running = Running::shell(&home);
    let shell = running.session.pid().unwrap();
    let events = running.events.clone();
    let clone = running.session.clone();
    drop(running);
    assert!(there(shell), "a clone still holds it");
    drop(clone);
    until("the dropped shell is reaped", || !there(shell));
    until("its threads end", || session_threads() == 0);
    assert!(events.lock().unwrap().contains(&SessionEvent::Exited(None)));

    // A shell that does not hang up is killed a moment later.
    let stubborn = start(
        &spec(
            &home,
            &[
                "-c",
                "trap '' HUP; echo READY; while :; do sleep 0.05; done",
            ],
        ),
        GridSize::default(),
    );
    stubborn.until("it to start", |frame| frame.text().contains("READY"));
    let shell = stubborn.session.pid().unwrap();
    let asked = Instant::now();
    stubborn.session.kill();
    stubborn.wait_exit();
    let took = asked.elapsed();
    assert!(took >= Duration::from_millis(900), "{took:?}");
    assert!(took < Duration::from_secs(5), "{took:?}");
    until("the stubborn shell is reaped", || !there(shell));
    until("its threads end", || session_threads() == 0);
    assert_eq!(stubborn.session.exited().unwrap().signal, Some(9));

    // A shell that exits by itself leaves nothing either.
    let running = Running::shell(&home);
    let shell = running.session.pid().unwrap();
    running.type_line("exit");
    assert_eq!(running.wait_exit(), Some(0));
    until("it is reaped", || !there(shell));
    until("its threads end", || session_threads() == 0);
}
