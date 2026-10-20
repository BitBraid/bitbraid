//! How fast a session reads a stream, and what a frame of a full screen
//! costs. Not in the gate (`--ignored`): it prints its numbers.
//!
//! `cargo test -p bitbraid-terminal --release --test suite -- --ignored
//! --nocapture throughput::`
#![allow(clippy::print_stdout)]
#![cfg(unix)]

use std::{
    fmt::Write as _,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crate::common::{Home, Running, spec, start};
use bitbraid_terminal::{GridSize, Palette};

const STREAM: usize = 100 << 20;

#[test]
#[ignore = "a measurement: run with --ignored --nocapture"]
fn a_stream_of_100_mb_and_a_full_frame() {
    let home = Home::new("throughput");
    let size = GridSize::new(200, 60, 8, 16);

    // 100 MB of lines through a real PTY, with a frame taken 60 times a
    // second meanwhile, as the window does while output streams.
    let command = format!(
        "yes 'the quick brown fox jumps over the lazy dog 0123456789 \u{1b}[32mgreen\u{1b}[0m' | head -c {STREAM}"
    );
    let running = start(&spec(&home, &["-c", &command]), size);
    let session = running.session.clone();
    let done = Arc::new(AtomicBool::new(false));
    let drawing = {
        let done = done.clone();
        std::thread::spawn(move || {
            let palette = Palette::default();
            let mut frames = 0u32;
            let mut spent = Duration::ZERO;
            while !done.load(Ordering::Acquire) {
                let started = Instant::now();
                let frame = session.frame(&palette);
                std::hint::black_box(&frame);
                spent += started.elapsed();
                frames += 1;
                std::thread::sleep(Duration::from_millis(16));
            }
            (frames, spent)
        })
    };
    let started = Instant::now();
    let deadline = Duration::from_secs(300);
    while running.exited().is_none() {
        assert!(started.elapsed() < deadline, "the stream never ended");
        std::thread::sleep(Duration::from_millis(5));
    }
    let took = started.elapsed();
    done.store(true, Ordering::Release);
    let (frames, spent) = drawing.join().unwrap();
    let mb = STREAM as f64 / f64::from(1 << 20);
    println!(
        "stream: {mb:.0} MiB in {:.2} s = {:.1} MiB/s ({frames} frames meanwhile, {:.2} ms each)",
        took.as_secs_f64(),
        mb / took.as_secs_f64(),
        spent.as_secs_f64() * 1000.0 / f64::from(frames.max(1))
    );

    // A full screen of coloured text: a colour, a style or a background
    // changes every four cells.
    let mut screen = String::new();
    for row in 0..60u32 {
        for cell in 0..50u32 {
            let n = row * 50 + cell;
            let _ = write!(
                screen,
                "\u{1b}[{};38;5;{};48;2;{};{};{}m{:04}",
                if n % 3 == 0 { 1 } else { 3 },
                n % 256,
                n % 7,
                n % 11,
                n % 13,
                n % 10_000
            );
        }
        screen.push_str("\u{1b}[0m");
        if row < 59 {
            screen.push_str("\r\n");
        }
    }
    std::fs::write(home.0.join("screen"), &screen).unwrap();
    let full = Running::shell_sized(&home, size);
    full.type_line("clear; cat screen; sleep 1000");
    full.until("the full screen", |frame| {
        frame.rows[59].text().len() == 200 && frame.rows[0].text().starts_with("0000")
    });
    let palette = Palette::default();
    let rounds = 500u32;
    let started = Instant::now();
    for _ in 0..rounds {
        std::hint::black_box(full.session.frame(&palette));
    }
    let frame_cost = started.elapsed() / rounds;
    let frame = full.session.frame(&palette);
    let started = Instant::now();
    let mut runs = 0usize;
    let mut spans = 0usize;
    for _ in 0..rounds {
        for row in &frame.rows {
            runs += row.runs().len();
            spans += row.backgrounds(frame.background).len();
        }
    }
    let group_cost = started.elapsed() / rounds;
    println!(
        "frame of 200x60: {:.3} ms to copy, {:.3} ms to group into {} runs and {} spans",
        frame_cost.as_secs_f64() * 1000.0,
        group_cost.as_secs_f64() * 1000.0,
        runs / rounds as usize,
        spans / rounds as usize
    );
    full.session.kill();
}
