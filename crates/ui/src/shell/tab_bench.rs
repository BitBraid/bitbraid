//! The measuring run of a tab switch (`BITBRAID_BENCH_TABS=<seconds>`,
//! `cargo xtask appbench --tabs`): once the repository the application was
//! started on has its graph drawn and has been left alone for a while, a
//! tab of the start page is opened beside it, and the window goes from one
//! tab to the other every half second. Each switch to the repository's tab
//! is measured by the main thread's time on a processor from the switch
//! until a quarter of a second later, by which the window has drawn it and
//! is quiet again: what a person waits for when coming back to a busy
//! repository.
//!
//! Sources: `crate::processor_time` for the main thread's time.

use std::time::{Duration, Instant};

use gpui_kit::*;

use super::Shell;

/// The variable that asks for the run, with its length in seconds.
pub const VARIABLE: &str = "BITBRAID_BENCH_TABS";

/// How long the repository is left alone before the first switch: its
/// first reads (status, references, the other worktrees) are done by then.
const SETTLE: Duration = Duration::from_secs(5);

/// From one switch to the next, and how long after a switch its time is
/// taken.
const STEP: Duration = Duration::from_millis(500);
const TAKEN_AFTER: Duration = Duration::from_millis(250);

/// The run's length, when one was asked for.
pub fn asked() -> Option<Duration> {
    std::env::var(VARIABLE)
        .ok()
        .and_then(|seconds| seconds.parse::<u64>().ok())
        .filter(|seconds| (1..=3600).contains(seconds))
        .map(Duration::from_secs)
}

/// What the switches measured: each one's main-thread time, in ms.
fn summary(samples: &[f64]) -> String {
    if samples.is_empty() {
        return "tab_switches=0".to_owned();
    }
    let mut sorted = samples.to_vec();
    sorted.sort_by(f64::total_cmp);
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let at = |share: f64| {
        let ix = ((sorted.len() as f64 - 1.0) * share).round() as usize;
        sorted.get(ix).copied().unwrap_or(0.0)
    };
    // not shown: xtask appbench reads it
    format!(
        "tab_switches={} tab_switch_ms_min={:.1} tab_switch_ms_median={:.1} tab_switch_ms_mean={mean:.1} tab_switch_ms_max={:.1}",
        sorted.len(),
        at(0.0),
        at(0.5),
        at(1.0)
    )
}

impl Shell {
    /// Start the measuring run of tab switches, when one was asked for.
    pub(super) fn start_tab_bench(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(length) = asked() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            let wait = |cx: &mut AsyncWindowContext, time| cx.background_executor().timer(time);
            // The graph of the repository it was started on, drawn.
            let started = Instant::now();
            loop {
                wait(cx, Duration::from_millis(100)).await;
                let loaded = this
                    .update(cx, |shell, cx| {
                        shell
                            .active_workspace()
                            .is_some_and(|workspace| workspace.read(cx).commit_count().is_some())
                    })
                    .unwrap_or(false);
                if loaded {
                    break;
                }
                if started.elapsed() > Duration::from_secs(600) {
                    return;
                }
            }
            wait(cx, SETTLE).await;
            let Ok(repository) = this.update_in(cx, |shell, window, cx| {
                let repository = shell.active;
                shell.new_tab(window, cx);
                repository
            }) else {
                return;
            };
            let start_page = this.update(cx, |shell, _| shell.active).unwrap_or(0);
            wait(cx, STEP).await;
            let mut samples = Vec::new();
            let begun = Instant::now();
            while begun.elapsed() < length {
                let before = crate::processor_time::this_thread();
                if this
                    .update(cx, |shell, cx| shell.activate(repository, cx))
                    .is_err()
                {
                    return;
                }
                wait(cx, TAKEN_AFTER).await;
                if let (Some(before), Some(after)) = (before, crate::processor_time::this_thread())
                {
                    samples.push(after.saturating_sub(before).as_secs_f64() * 1000.0);
                }
                let _ = this.update(cx, |shell, cx| shell.activate(start_page, cx));
                wait(cx, STEP).await;
            }
            let memory = crate::memory::resident()
                .map(|held| {
                    format!(
                        " peak_rss_mib={} rss_mib={}", // not shown: xtask appbench reads it
                        held.peak_mib(),
                        held.now_mib()
                    )
                })
                .unwrap_or_default();
            #[allow(clippy::print_stdout)] // the measuring run reports on stdout by design
            {
                println!(
                    "APP_BENCH seconds={} {}{memory}",
                    length.as_secs(),
                    summary(&samples)
                );
            }
            let _ = this.update(cx, |_, cx| cx.quit());
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::summary;

    #[test]
    fn the_switches_are_summed_up_in_order() {
        assert_eq!(summary(&[]), "tab_switches=0");
        assert_eq!(
            summary(&[30.0, 10.0, 20.0]),
            "tab_switches=3 tab_switch_ms_min=10.0 tab_switch_ms_median=20.0 tab_switch_ms_mean=20.0 tab_switch_ms_max=30.0"
        );
    }
}
