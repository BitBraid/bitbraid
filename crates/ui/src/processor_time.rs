//! How long the calling thread has run on a processor, for the measuring
//! runs (`BITBRAID_BENCH`, `cargo xtask appbench`): a headless compositor
//! asks for no more than its 60 frames a second, so the frame rate says
//! only whether the application keeps up, and the time the main thread
//! spends on each frame says by how much.
//!
//! Linux, where the kernel keeps the figure in `/proc`, and macOS, where
//! the thread's own clock of processor time says it; elsewhere there is
//! none and the measuring run says so by leaving it out.
//!
//! Sources: proc(5) (`/proc/thread-self`, `/proc/[pid]/schedstat`);
//! clock_gettime(2) (`CLOCK_THREAD_CPUTIME_ID`) and the documentation of
//! rustix 1 (`time::clock_gettime`).

use std::time::Duration;

/// How long the calling thread has run, where the system tells.
pub fn this_thread() -> Option<Duration> {
    #[cfg(target_os = "macos")]
    {
        let spent = rustix::time::clock_gettime(rustix::time::ClockId::ThreadCPUTime);
        let seconds = u64::try_from(spent.tv_sec).ok()?;
        let nanoseconds = u32::try_from(spent.tv_nsec).ok()?;
        Some(Duration::new(seconds, nanoseconds))
    }
    #[cfg(not(target_os = "macos"))]
    {
        // One short line; anything else at that name is not read.
        let text = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
        parse(&text)
    }
}

/// The time on a processor out of the text of a `schedstat` file: its
/// first figure, in nanoseconds.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn parse(text: &str) -> Option<Duration> {
    let nanoseconds = text.split_whitespace().next()?.parse::<u64>().ok()?;
    Some(Duration::from_nanos(nanoseconds))
}

#[cfg(test)]
mod tests {
    use super::parse;
    use std::time::Duration;

    #[test]
    fn the_time_run_is_the_first_figure() {
        assert_eq!(
            parse("1234567890 45678 321\n"),
            Some(Duration::from_nanos(1_234_567_890))
        );
    }

    #[test]
    fn text_that_is_not_a_schedstat_gives_nothing() {
        for hostile in [
            "",
            "\n",
            "-5 1 1",
            "many 1 1",
            "99999999999999999999999 1 1",
        ] {
            assert_eq!(parse(hostile), None, "{hostile:?}");
        }
    }

    #[test]
    fn the_system_tells_on_linux_and_macos() {
        // The figure itself is brought up to date at the scheduler's
        // ticks, so a thread that has just begun may read nought.
        if cfg!(any(target_os = "linux", target_os = "macos")) {
            assert!(super::this_thread().is_some());
        }
        // macOS's clock moves with the work done.
        if cfg!(target_os = "macos") {
            let before = super::this_thread().unwrap();
            let mut sum = 0u64;
            for n in 0..20_000_000u64 {
                sum = sum.wrapping_add(n * n);
            }
            assert!(sum > 0);
            assert!(super::this_thread().unwrap() > before);
        }
    }
}
