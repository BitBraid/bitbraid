//! What goes on around a command that the command line runs: whether a
//! repository is opened for it first, what it reports while it runs, and
//! how it is stopped.
//!
//! Progress goes to standard error, so that standard output stays the
//! JSON result and nothing else. On a terminal the line is rewritten in
//! place; into a pipe or a file a line is written when the stage changes
//! and at every tenth of the way.
//!
//! Ctrl-C (and a request to terminate) sets the engine's cancel flag: a
//! command that watches it stops its git process and says that it was
//! stopped, and what it left unfinished it cleans up as it does in the
//! application. A command that does not watch the flag is given a moment
//! to end and is then left; a second Ctrl-C ends the process at once.
//!
//! Sources: signal(7) (what a terminal sends on Ctrl-C; an exit status of
//! 128 plus the signal's number for a process that ended on one);
//! ECMA-48 (`CSI K`, erase to the end of the line).

use std::{
    io::{IsTerminal as _, Write as _},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use bitbraid_engine::{Needs, ProgressSink, Registry, workspace::RepositoryOutcome};
use bitbraid_git::Progress;

/// The exit status of a run that was stopped by Ctrl-C: 128 + SIGINT.
pub const INTERRUPTED: u8 = 130;

/// How long a command that does not watch the cancel flag is waited for
/// after Ctrl-C.
const GRACE: Duration = Duration::from_secs(3);

/// Whether a repository is opened before a command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    /// The command cannot run without one.
    Must,
    /// The command uses one when there is one.
    IfThere,
    /// The command takes no notice of one.
    No,
}

/// What the registry says about the command `id`. One that is not
/// registered opens nothing: running it says that it is unknown.
pub fn opening(registry: &Registry, id: &str) -> Opening {
    match registry.access(id).map(|access| access.repository) {
        Some(Needs::Repository) => Opening::Must,
        Some(Needs::RepositoryIfOpen) => Opening::IfThere,
        Some(Needs::Nothing) | None => Opening::No,
    }
}

#[derive(Debug, Default)]
struct Shown {
    stage: String,
    percent: Option<u8>,
    /// A line on the terminal that has not been ended yet.
    open: bool,
}

/// Where a running command's progress is written.
#[derive(Debug)]
pub struct Meter {
    /// Standard error is a terminal: one line, rewritten in place.
    live: bool,
    shown: Mutex<Shown>,
}

/// One progress report as a line of text. A report of one repository of
/// several (the workspace commands) starts with the repository.
fn line(progress: &Progress) -> String {
    if let Some(outcome) = RepositoryOutcome::from_progress(progress) {
        let what = match (&outcome.error, &outcome.progress) {
            (Some(error), _) => format!("{}: {error}", outcome.state.as_str()),
            (None, Some(stage)) => stage.clone(),
            (None, None) => outcome.state.as_str().to_owned(),
        };
        return match outcome.percent {
            Some(percent) => format!("{}: {what}: {percent}%", outcome.path),
            None => format!("{}: {what}", outcome.path),
        };
    }
    match progress.percent {
        Some(percent) => format!("{}: {percent}%", progress.stage),
        None => progress.stage.clone(),
    }
}

/// Whether a report is worth a line of its own after the one before it,
/// where every line stays: a new stage, the next tenth, or the end.
fn worth_a_line(before: &(String, Option<u8>), progress: &Progress) -> bool {
    if before.0 != progress.stage {
        return true;
    }
    if before.1 == progress.percent {
        return false;
    }
    let tenth = |percent: Option<u8>| percent.map(|percent| percent / 10);
    progress.percent == Some(100) || tenth(before.1) != tenth(progress.percent)
}

impl Meter {
    /// A meter for standard error as it is now.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            live: std::io::stderr().is_terminal(),
            shown: Mutex::new(Shown::default()),
        })
    }

    /// What the engine reports to.
    pub fn sink(self: &Arc<Self>) -> ProgressSink {
        let meter = self.clone();
        Arc::new(move |progress| meter.show(&progress))
    }

    fn show(&self, progress: &Progress) {
        // A poisoned lock: the line is not shown, the command goes on.
        let Ok(mut shown) = self.shown.lock() else {
            return;
        };
        let before = (std::mem::take(&mut shown.stage), shown.percent);
        let mut stderr = std::io::stderr().lock();
        if self.live {
            if before.0 != progress.stage || before.1 != progress.percent {
                let _ = write!(stderr, "\r\x1b[K{}", line(progress));
                let _ = stderr.flush();
                shown.open = true;
            }
        } else if worth_a_line(&before, progress) {
            let _ = writeln!(stderr, "{}", line(progress));
        }
        shown.stage.clone_from(&progress.stage);
        shown.percent = progress.percent;
    }

    /// End the line that is being rewritten, so that what is written next
    /// starts on one of its own.
    pub fn finish(&self) {
        if let Ok(mut shown) = self.shown.lock()
            && std::mem::take(&mut shown.open)
        {
            let _ = writeln!(std::io::stderr().lock());
        }
    }
}

/// Ends the meter's line when it is dropped: on every way out of a run.
#[derive(Debug)]
pub struct EndsLine(pub Arc<Meter>);

impl Drop for EndsLine {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// Have Ctrl-C set `cancel` instead of ending the process, and see to it
/// that the process ends all the same.
pub fn stop_on_interrupt(cancel: &Arc<AtomicBool>) {
    use signal_hook::consts::{SIGINT, SIGTERM};
    for signal in [SIGINT, SIGTERM] {
        // In this order: with the flag set already, the signal does what
        // it does by default, which ends the process.
        let _ = signal_hook::flag::register_conditional_default(signal, cancel.clone());
        let _ = signal_hook::flag::register(signal, cancel.clone());
    }
    let cancel = cancel.clone();
    let _ = std::thread::Builder::new()
        .name("bitbraid-cli-interrupt".to_owned())
        .spawn(move || {
            while !cancel.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(50));
            }
            // A command that watches the flag has ended long before this.
            std::thread::sleep(GRACE);
            let _ = writeln!(
                std::io::stderr().lock(),
                "error: stopped (the command did not end by itself)"
            );
            std::process::exit(i32::from(INTERRUPTED));
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(stage: &str, percent: Option<u8>) -> Progress {
        Progress {
            stage: stage.to_owned(),
            percent,
        }
    }

    #[test]
    fn a_report_is_a_stage_and_how_far_it_is() {
        assert_eq!(
            line(&report("Receiving objects", Some(42))),
            "Receiving objects: 42%"
        );
        assert_eq!(
            line(&report("Enumerating objects", None)),
            "Enumerating objects"
        );
    }

    #[test]
    fn a_report_of_one_repository_of_several_names_it_first() {
        assert_eq!(
            line(&report("/src/a\trunning\tReceiving objects", Some(42))),
            "/src/a: Receiving objects: 42%"
        );
        assert_eq!(line(&report("/src/a\tqueued\t", None)), "/src/a: queued");
        assert_eq!(line(&report("/src/a\tdone\t", None)), "/src/a: done");
        assert_eq!(
            line(&report("/src/a\tfailed\tfatal: gone", None)),
            "/src/a: failed: fatal: gone"
        );
    }

    #[test]
    fn lines_that_stay_are_few() {
        let mut before = (String::new(), None);
        let mut written = Vec::new();
        let reports = (0..=100u8)
            .map(|percent| report("Receiving objects", Some(percent)))
            .chain([report("Receiving objects", Some(100))])
            .chain([report("Resolving deltas", None)])
            .chain([report("Resolving deltas", None)])
            .chain(
                (0..=100u8)
                    .step_by(50)
                    .map(|p| report("Resolving deltas", Some(p))),
            );
        for progress in reports {
            if worth_a_line(&before, &progress) {
                written.push(line(&progress));
            }
            before = (progress.stage.clone(), progress.percent);
        }
        assert_eq!(
            written,
            [
                "Receiving objects: 0%",
                "Receiving objects: 10%",
                "Receiving objects: 20%",
                "Receiving objects: 30%",
                "Receiving objects: 40%",
                "Receiving objects: 50%",
                "Receiving objects: 60%",
                "Receiving objects: 70%",
                "Receiving objects: 80%",
                "Receiving objects: 90%",
                "Receiving objects: 100%",
                "Resolving deltas",
                "Resolving deltas: 0%",
                "Resolving deltas: 50%",
                "Resolving deltas: 100%",
            ]
        );
    }

    #[test]
    fn the_registry_decides_what_is_opened() {
        let registry = bitbraid_integrations::registry();
        for (id, want) in [
            ("repo.log", Opening::Must),
            ("branch.create", Opening::Must),
            ("repo.init", Opening::No),
            ("repo.clone", Opening::No),
            ("link.parse", Opening::No),
            ("profile.list", Opening::No),
            ("link.make", Opening::IfThere),
            ("profile.current", Opening::IfThere),
            ("profile.use", Opening::IfThere),
            // The provider commands bring their own lines.
            ("account.list", Opening::No),
            ("pr.list", Opening::IfThere),
            ("provider.links", Opening::Must),
            ("pr.checkout", Opening::Must),
            ("no.such", Opening::No),
        ] {
            assert_eq!(opening(&registry, id), want, "{id}");
        }
    }
}
