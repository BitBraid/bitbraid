//! How far git-lfs is, while it moves content.
//!
//! Git-lfs says it in two ways, and each leaves out what the other has.
//!
//! Its meter, one line on standard output that it rewrites in place,
//! counts the files that are done: `Downloading LFS objects:  50% (1/2),
//! 800 KB | 183 KB/s`. One large file stands at 0 % until it is there.
//! Without a terminal the meter is not written at all unless
//! `GIT_LFS_FORCE_PROGRESS` asks for it.
//!
//! Its report, a file named by `GIT_LFS_PROGRESS` to which it appends a
//! line now and then, has the bytes of each file:
//! `<direction> <current>/<total files> <downloaded>/<total> <name>`, with
//! `download`, `upload` or `checkout` for the direction. A transfer
//! between two directories on one machine, which git-lfs does without a
//! server, writes nothing there.
//!
//! So both are asked for and read into one figure per direction, under
//! the words of the meter: the files that are done and the part of each
//! one in flight that has arrived, of all the files there are. A figure
//! never goes back, whichever of the two a line came from, and one that
//! says nothing new is not passed on.
//!
//! The lines of a checkout are different from what the page says: each is
//! one file that was put in place, with its size against the size of all
//! of them (found by running git-lfs 3.8.0). The filter's own lines
//! (`clean`, `smudge`) are about a single file each and are passed over.
//!
//! Sources: git-lfs-config(5) (`GIT_LFS_PROGRESS` and the format of its
//! lines; `GIT_LFS_FORCE_PROGRESS` and `lfs.forceprogress`); git-lfs(1);
//! git-lfs-fetch(1), git-lfs-pull(1), git-lfs-push(1),
//! git-lfs-checkout(1).

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{Progress, Repository, remote};

/// The variable that names the file git-lfs appends its report to. The
/// path has to be absolute.
pub(super) const REPORT: &str = "GIT_LFS_PROGRESS";
/// The variable that has the meter written without a terminal.
pub(super) const METER: &str = "GIT_LFS_FORCE_PROGRESS";

/// More files in flight than this are not told apart; git-lfs moves
/// eight at a time unless it is set otherwise.
const MOST_IN_FLIGHT: usize = 64;
/// More stages than this are not remembered.
const MOST_STAGES: usize = 16;

/// Told apart by a number: two commands of one process at once have two
/// files.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// The file one command reports to. Removed when this is dropped.
#[derive(Debug)]
pub(super) struct Report {
    file: PathBuf,
}

impl Report {
    /// A file for the report of one command in this repository, in a
    /// folder where nobody else writes (see [`folders`]). None where it
    /// cannot be made; the command then runs with its meter only.
    pub(super) fn begin(repo: &Repository) -> Option<Self> {
        let git_dir = std::path::absolute(repo.git_dir()).ok()?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
        let dir = folders(&git_dir, runtime.as_deref())
            .into_iter()
            .find(|dir| fs::create_dir_all(dir).is_ok())?;
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let file = dir.join(format!("lfs-{}-{n}.progress", std::process::id()));
        // One of an earlier process with this number that was left behind.
        let _ = fs::remove_file(&file);
        Some(Self { file })
    }

    /// Where it is.
    pub(super) fn path(&self) -> &Path {
        &self.file
    }
}

impl Drop for Report {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file);
    }
}

/// Where a report may go, the first that can be made: `bitbraid/` in the
/// user's runtime directory where there is one, then `bitbraid/` in the
/// repository's git directory.
///
/// Git-lfs waits for the disk with every line it appends to the report
/// (3.8.0 does): with the file on a disk a transfer can take several times
/// as long, with it in memory no longer than without a report. The runtime
/// directory (`XDG_RUNTIME_DIR`) is the user's alone, and systemd keeps it
/// in memory; one that is not an absolute path to a folder is not taken.
fn folders(git_dir: &Path, runtime: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(runtime) = runtime.filter(|dir| dir.is_absolute() && dir.is_dir()) {
        out.push(runtime.join("bitbraid"));
    }
    out.push(git_dir.join("bitbraid"));
    out
}

/// One line of the report.
#[derive(Debug, PartialEq, Eq)]
struct Step<'a> {
    direction: &'a str,
    /// Which file this is, counted from one.
    file: u64,
    /// How many files there are, as far as git-lfs knows.
    files: u64,
    bytes: u64,
    of: u64,
    name: &'a str,
}

impl<'a> Step<'a> {
    /// The line as a step; none for a line that is not one. The numbers
    /// are whatever was written and are only ever compared and summed.
    fn read(line: &'a str) -> Option<Self> {
        let mut parts = line.trim_end_matches(['\r', '\n']).splitn(4, ' ');
        let direction = parts.next()?;
        let (file, files) = parts.next()?.split_once('/')?;
        let (bytes, of) = parts.next()?.split_once('/')?;
        Some(Self {
            direction,
            file: file.parse().ok()?,
            files: files.parse().ok()?,
            bytes: bytes.parse().ok()?,
            of: of.parse().ok()?,
            name: parts.next().unwrap_or(""),
        })
    }
}

/// The stages content is moved in, in the words of the meter.
const DOWNLOADING: &str = "Downloading LFS objects";
const UPLOADING: &str = "Uploading LFS objects";
const CHECKING_OUT: &str = "Checking out LFS objects";

/// What the meter of git-lfs calls a direction.
fn stage_of(direction: &str) -> Option<&'static str> {
    match direction {
        "download" => Some(DOWNLOADING),
        "upload" => Some(UPLOADING),
        "checkout" => Some(CHECKING_OUT),
        _ => None,
    }
}

/// Whether a line is the meter standing at less than everything. After a
/// command that went through, such a line is only where git-lfs stopped
/// writing, not where it stopped.
pub(super) fn is_stale_meter(line: &str) -> bool {
    remote::parse_progress(line).is_some_and(|progress| {
        [DOWNLOADING, UPLOADING, CHECKING_OUT].contains(&progress.stage.as_str())
            && progress.percent != Some(100)
    })
}

/// How far one direction is, by the report.
#[derive(Debug, Default)]
struct Stage {
    /// How many files there are.
    files: u64,
    /// How many of them are done.
    finished: u64,
    /// The ones in flight, each with how many thousandths of it arrived.
    in_flight: HashMap<String, u64>,
}

impl Stage {
    fn take(&mut self, step: &Step<'_>) {
        self.files = self.files.max(step.files).max(step.file);
        if step.direction == "checkout" {
            // One line for each file, when it is in place.
            self.finished = self.finished.max(step.file);
        } else if step.bytes >= step.of {
            self.in_flight.remove(step.name);
            self.finished = self.finished.saturating_add(1).min(self.files);
        } else if self.in_flight.len() < MOST_IN_FLIGHT || self.in_flight.contains_key(step.name) {
            let thousandths = u128::from(step.bytes).saturating_mul(1000) / u128::from(step.of);
            self.in_flight.insert(
                step.name.to_owned(),
                u64::try_from(thousandths).unwrap_or(1000).min(1000),
            );
        }
    }

    fn percent(&self) -> u8 {
        let in_flight = u64::try_from(self.in_flight.len()).unwrap_or(u64::MAX);
        let files = self
            .files
            .max(self.finished.saturating_add(in_flight))
            .max(1);
        let arrived = self
            .finished
            .saturating_mul(1000)
            .saturating_add(self.in_flight.values().sum());
        u8::try_from((arrived / files / 10).min(100)).unwrap_or(100)
    }
}

/// What git-lfs says while it runs, as progress to pass on.
#[derive(Debug, Default)]
pub(super) struct Meter {
    stages: HashMap<&'static str, Stage>,
    /// The highest figure each stage was at.
    highest: HashMap<String, Option<u8>>,
    last: Option<Progress>,
}

impl Meter {
    /// A line of the report file.
    pub(super) fn reported(&mut self, line: &str) -> Option<Progress> {
        let step = Step::read(line)?;
        let name = stage_of(step.direction)?;
        let stage = self.stages.entry(name).or_default();
        stage.take(&step);
        let percent = stage.percent();
        self.pass(name, Some(percent))
    }

    /// A line git-lfs wrote: its meter, or something else it is doing.
    pub(super) fn said(&mut self, line: &str) -> Option<Progress> {
        let progress = remote::parse_progress(line)?;
        self.pass(&progress.stage, progress.percent)
    }

    /// What is left to say when the command has ended well: a stage that
    /// content was moved in is at its end, also where git-lfs did not get
    /// to say so. It does not always write the last state of its meter
    /// (3.8.0 does not after a fetch that was quick), and the report has
    /// a line for what a transfer program reported and none for a file
    /// being done.
    pub(super) fn ended_well(&mut self) -> Vec<Progress> {
        let mut left = Vec::new();
        for stage in [DOWNLOADING, UPLOADING, CHECKING_OUT] {
            let short = self.highest.get(stage).is_some_and(|at| *at != Some(100));
            if short && let Some(progress) = self.pass(stage, Some(100)) {
                left.push(progress);
            }
        }
        left
    }

    /// The progress to pass on for a stage that is at `percent` by one of
    /// the two: none when that is nothing new.
    fn pass(&mut self, stage: &str, percent: Option<u8>) -> Option<Progress> {
        let percent = match self.highest.get_mut(stage) {
            Some(highest) => {
                // No figure is less than any figure.
                *highest = (*highest).max(percent);
                *highest
            }
            None => {
                if self.highest.len() < MOST_STAGES {
                    self.highest.insert(stage.to_owned(), percent);
                }
                percent
            }
        };
        let progress = Progress {
            stage: stage.to_owned(),
            percent,
        };
        if self.last.as_ref() == Some(&progress) {
            return None;
        }
        self.last = Some(progress.clone());
        Some(progress)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passed(meter: &mut Meter, lines: &[&str]) -> Vec<(String, Option<u8>)> {
        lines
            .iter()
            .filter_map(|line| meter.reported(line))
            .map(|progress| (progress.stage, progress.percent))
            .collect()
    }

    #[test]
    fn the_report_goes_where_writing_it_is_cheap() {
        let git_dir = Path::new("/somewhere/repo/.git");
        let runtime = std::env::temp_dir();
        assert_eq!(
            folders(git_dir, Some(&runtime)),
            [runtime.join("bitbraid"), git_dir.join("bitbraid")]
        );
        // None, a relative one, one that is not there: the git directory.
        let missing = runtime.join("bitbraid-no-such-folder-for-the-report");
        for runtime in [
            None,
            Some(Path::new("run/user/1000")),
            Some(missing.as_path()),
        ] {
            assert_eq!(folders(git_dir, runtime), [git_dir.join("bitbraid")]);
        }
    }

    #[test]
    fn a_line_of_the_report_is_read_or_passed_over() {
        assert_eq!(
            Step::read("download 1/2 250000/500000 sub dir/two.bin"),
            Some(Step {
                direction: "download",
                file: 1,
                files: 2,
                bytes: 250_000,
                of: 500_000,
                name: "sub dir/two.bin",
            })
        );
        assert_eq!(
            Step::read("checkout 2/2 15/26 two.bin\r\n").map(|step| step.name),
            Some("two.bin")
        );
        // Without a name it is still a step.
        assert_eq!(Step::read("upload 1/1 0/0").map(|step| step.of), Some(0));
        for line in [
            "",
            " ",
            "download",
            "download 1 2 3",
            "download 1/2",
            "download 1/2 3",
            "download a/2 3/4 x",
            "download 1/2 -3/4 x",
            "download 1/2 3/99999999999999999999999 x",
            "download  1/2 3/4 x",
            "\u{0}\u{1}/\u{2} /",
            "Downloading LFS objects: 100% (2/2), 26 B | 0 B/s",
        ] {
            assert_eq!(Step::read(line), None, "{line:?}");
        }
    }

    #[test]
    fn a_transfer_is_as_far_as_its_bytes_are() {
        // As git-lfs 3.8.0 wrote it for two files through a transfer
        // program that reports twice for each.
        let mut meter = Meter::default();
        assert_eq!(
            passed(
                &mut meter,
                &[
                    "upload 1/2 250000/500000 two.bin",
                    "upload 1/2 250000/500000 two.bin",
                    "upload 1/2 500000/500000 two.bin",
                    "upload 2/2 150000/300000 one.bin",
                    "upload 2/2 300000/300000 one.bin",
                ]
            ),
            [
                ("Uploading LFS objects".to_owned(), Some(25)),
                ("Uploading LFS objects".to_owned(), Some(50)),
                ("Uploading LFS objects".to_owned(), Some(75)),
                ("Uploading LFS objects".to_owned(), Some(100)),
            ]
        );
        // The filter's lines in between are not the transfer's.
        let mut meter = Meter::default();
        assert_eq!(
            passed(
                &mut meter,
                &[
                    "download 1/2 250000/500000 two.bin",
                    "download 1/2 500000/500000 two.bin",
                    "download 2/2 150000/300000 one.bin",
                    "clean 1/1 500000/500000 two.bin",
                    "smudge 1/1 1/500000 two.bin",
                    "download 2/2 300000/300000 one.bin",
                    "clean 1/1 300000/300000 one.bin",
                ]
            )
            .into_iter()
            .map(|(stage, percent)| {
                assert_eq!(stage, "Downloading LFS objects");
                percent
            })
            .collect::<Vec<_>>(),
            [Some(25), Some(50), Some(75), Some(100)]
        );
    }

    #[test]
    fn files_in_flight_together_are_summed_and_the_figure_never_goes_back() {
        let mut meter = Meter::default();
        let percents: Vec<Option<u8>> = passed(
            &mut meter,
            &[
                "download 1/4 50/100 a",
                "download 2/4 10/100 b",
                "download 3/4 0/100 c",
                "download 1/4 90/100 a",
                "download 2/4 100/100 b",
                // More files turn up than were known of.
                "download 5/8 1/100 e",
                "download 1/4 100/100 a",
                "download 3/4 100/100 c",
            ],
        )
        .into_iter()
        .map(|(_, percent)| percent)
        .collect();
        // Of four files: half of one, then 0.6, the same again, 1.0 and
        // 1.9. Then there are eight, and of eight less is there than was
        // said of four (1.91, 2.01 and 3.01), so the figure stands.
        assert_eq!(percents, [Some(12), Some(15), Some(25), Some(47)]);
        assert_eq!(
            meter.highest.get("Downloading LFS objects"),
            Some(&Some(47))
        );
        // A file with nothing in it is done when it is named.
        let mut meter = Meter::default();
        assert_eq!(
            passed(&mut meter, &["upload 1/1 0/0 empty"]),
            [("Uploading LFS objects".to_owned(), Some(100))]
        );
    }

    #[test]
    fn a_checkout_counts_files() {
        let mut meter = Meter::default();
        assert_eq!(
            passed(
                &mut meter,
                &[
                    "checkout 1/3 11/40 one.bin",
                    "checkout 2/3 15/40 two.bin",
                    "checkout 3/3 14/40 three.bin",
                ]
            ),
            [
                ("Checking out LFS objects".to_owned(), Some(33)),
                ("Checking out LFS objects".to_owned(), Some(66)),
                ("Checking out LFS objects".to_owned(), Some(100)),
            ]
        );
    }

    #[test]
    fn the_meter_and_the_report_make_one_figure() {
        let mut meter = Meter::default();
        let mut seen = Vec::new();
        let mut take = |progress: Option<Progress>| {
            if let Some(progress) = progress {
                seen.push((progress.stage, progress.percent));
            }
        };
        take(meter.said("Fetching reference refs/heads/main"));
        take(meter.said("Downloading LFS objects:   0% (0/1), 0 B | 0 B/s"));
        take(meter.reported("download 1/2 250000/500000 two.bin"));
        // The meter still counts no file as done.
        take(meter.said("Downloading LFS objects:   0% (0/2), 500 KB | 125 KB/s"));
        take(meter.reported("download 1/2 500000/500000 two.bin"));
        take(meter.said("Downloading LFS objects:  50% (1/2), 800 KB | 183 KB/s"));
        take(meter.reported("download 2/2 300000/300000 one.bin"));
        take(meter.said("Downloading LFS objects: 100% (2/2), 800 KB | 183 KB/s"));
        take(meter.said("error: failed to fetch some objects"));
        let stage = "Downloading LFS objects".to_owned();
        assert_eq!(
            seen,
            [
                (stage.clone(), Some(0)),
                (stage.clone(), Some(25)),
                (stage.clone(), Some(50)),
                (stage, Some(100)),
            ]
        );
    }

    #[test]
    fn a_command_that_ended_well_is_at_the_end_of_what_it_moved() {
        // As git-lfs 3.8.0 left it after a quick fetch of two files
        // through a transfer program that reports half of each.
        let mut meter = Meter::default();
        let _ = meter.said("Downloading LFS objects:   0% (0/1), 0 B | 0 B/s");
        let _ = meter.reported("download 1/2 7/15 sub/two.bin");
        let _ = meter.reported("download 2/2 5/11 one.bin");
        let _ = meter.said("Fetching reference refs/heads/main");
        assert_eq!(
            meter.ended_well(),
            [Progress {
                stage: "Downloading LFS objects".to_owned(),
                percent: Some(100),
            }]
        );
        // Said once.
        assert_eq!(meter.ended_well(), Vec::<Progress>::new());
        // Nothing was moved, or all of it was said: nothing to add.
        let mut meter = Meter::default();
        let _ = meter.said("Receiving objects:  50% (1/2)");
        let _ = meter.said("Uploading LFS objects: 100% (2/2), 26 B | 0 B/s, done.");
        assert_eq!(meter.ended_well(), Vec::<Progress>::new());

        for (line, stale) in [
            ("Downloading LFS objects:   0% (0/1), 0 B | 0 B/s", true),
            ("Uploading LFS objects:  50% (1/2), 11 B | 0 B/s", true),
            ("Checking out LFS objects", false),
            (
                "Checking out LFS objects: 100% (2/2), 26 B | 0 B/s, done.",
                false,
            ),
            ("Fetching reference refs/heads/main", false),
            ("Receiving objects:  50% (1/2)", false),
            ("", false),
        ] {
            assert_eq!(is_stale_meter(line), stale, "{line:?}");
        }
    }

    #[test]
    fn hostile_reports_are_only_reports() {
        let mut meter = Meter::default();
        let most = u64::MAX;
        for line in [
            format!("download {most}/{most} {most}/{most} x"),
            format!("download 0/0 {most}/1 x"),
            format!("upload {most}/1 1/{most} y"),
            "checkout 0/0 0/0 ".to_owned(),
            format!("checkout {most}/3 1/2 z"),
            "download 1/1 5/0 nothing".to_owned(),
        ] {
            if let Some(progress) = meter.reported(&line) {
                assert!(progress.percent.is_some_and(|percent| percent <= 100));
            }
        }
        // Names without end are not all remembered.
        for n in 0..10 * MOST_IN_FLIGHT {
            let _ = meter.reported(&format!("download 1/9 1/2 file-{n}"));
        }
        let stage = meter.stages.get("Downloading LFS objects").unwrap();
        assert!(stage.in_flight.len() <= MOST_IN_FLIGHT);
        // Nor are stages.
        for n in 0..10 * MOST_STAGES {
            let stage = "A".repeat(n % 40 + 1);
            let _ = meter.said(&format!("{stage}: 5% (1/20)"));
        }
        assert!(meter.highest.len() <= MOST_STAGES);
    }
}
