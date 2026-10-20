//! Maintenance: how large the repository is and what takes the room,
//! git's own maintenance tasks with what each freed, and a check of the
//! objects.
//!
//! A section of the repository's preferences, laid out as the others are (a
//! row of words, a control beside them, what it says under the control).
//! The size is measured the first time the section is shown and when asked
//! again; it is a read on a thread of its own that can be stopped. A task
//! and the check run as operations of the repository's tab: the status bar
//! shows how far they are, they are stopped from there or from here, and
//! what they answer comes back to this view.
//!
//! Sources: git-gc(1), git-prune(1), git-repack(1), git-commit-graph(1),
//! git-fsck(1), git-count-objects(1).

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::{Value, json};

use crate::{
    session::{Reply, Request, SessionHandle},
    views::preferences::{
        page::{across, own_group},
        row::{BUTTON_HEIGHT, Parts, row},
    },
};

/// One of git's maintenance tasks, as the engine's `repo.maintenance`
/// names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Task {
    /// `git gc`.
    Gc,
    /// `git gc --aggressive`.
    GcAggressive,
    /// `git prune`.
    Prune,
    /// `git repack`.
    Repack,
    /// `git commit-graph write`.
    CommitGraph,
}

impl Task {
    /// In the order the section lists them.
    pub const ALL: [Task; 5] = [
        Task::Gc,
        Task::Prune,
        Task::Repack,
        Task::CommitGraph,
        Task::GcAggressive,
    ];

    /// The engine's word for it.
    pub fn word(self) -> &'static str {
        match self {
            Task::Gc => "gc",
            Task::GcAggressive => "gc_aggressive",
            Task::Prune => "prune",
            Task::Repack => "repack",
            Task::CommitGraph => "commit_graph",
        }
    }

    /// What its row is called.
    fn label(self) -> &'static str {
        match self {
            Task::Gc => say!("maintenance-gc"),
            Task::GcAggressive => say!("maintenance-gc-aggressive"),
            Task::Prune => say!("maintenance-prune"),
            Task::Repack => say!("maintenance-repack"),
            Task::CommitGraph => say!("maintenance-commit-graph"),
        }
    }

    /// What it does, under its button.
    fn note(self) -> &'static str {
        match self {
            Task::Gc => say!("maintenance-gc-note"),
            Task::GcAggressive => say!("maintenance-gc-aggressive-note"),
            Task::Prune => say!("maintenance-prune-note"),
            Task::Repack => say!("maintenance-repack-note"),
            Task::CommitGraph => say!("maintenance-commit-graph-note"),
        }
    }

    /// What the status bar says while it runs.
    pub fn doing(self) -> &'static str {
        match self {
            Task::Gc | Task::GcAggressive => say!("maintenance-collecting-garbage"),
            Task::Prune => say!("maintenance-pruning"),
            Task::Repack => say!("maintenance-repacking"),
            Task::CommitGraph => say!("maintenance-writing-commit-graph"),
        }
    }
}

/// What runs or ran: a task, or the check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Job {
    /// One of the tasks.
    Task(Task),
    /// `git fsck`.
    Check,
}

/// Events from the view, which the repository's tab carries out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaintenanceEvent {
    /// Run a task or the check.
    Run(Job),
    /// Stop what runs.
    Stop,
}

/// A size in bytes, for reading: bytes, then KiB, MiB and GiB with one
/// decimal.
pub fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 3] = ["KiB", "MiB", "GiB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit = unit.saturating_add(1);
    }
    format!("{value:.1} {}", UNITS.get(unit).copied().unwrap_or("GiB"))
}

/// What a [`Tally`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Counted {
    /// The files of a worktree.
    Files,
    /// The file contents of the LFS store.
    FileContents,
}

fn number(value: &Value, key: &str) -> u64 {
    value.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

/// The objects as git counts them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Counts {
    /// Loose objects and their room.
    pub loose: u64,
    pub loose_bytes: u64,
    /// Objects in packs, the packs, and their room.
    pub packed: u64,
    pub packs: u64,
    pub pack_bytes: u64,
    /// Files among the objects that are none of git's, and their room.
    pub garbage: u64,
    pub garbage_bytes: u64,
}

impl Counts {
    fn from_json(value: &Value) -> Self {
        Self {
            loose: number(value, "loose"),
            loose_bytes: number(value, "loose_bytes"),
            packed: number(value, "packed"),
            packs: number(value, "packs"),
            pack_bytes: number(value, "pack_bytes"),
            garbage: number(value, "garbage"),
            garbage_bytes: number(value, "garbage_bytes"),
        }
    }

    /// All the room the objects take.
    pub fn total_bytes(&self) -> u64 {
        self.loose_bytes
            .saturating_add(self.pack_bytes)
            .saturating_add(self.garbage_bytes)
    }

    /// The objects in a sentence.
    pub fn said(&self) -> String {
        let mut out = say!(
            "maintenance-objects-said",
            objects = say!(
                "maintenance-objects-count",
                count = self.loose.saturating_add(self.packed)
            ),
            total = size_text(self.total_bytes()),
            packed = self.packed,
            packs = say!("maintenance-packs-count", count = self.packs),
            pack_size = size_text(self.pack_bytes),
            loose = self.loose,
            loose_size = size_text(self.loose_bytes),
        );
        if self.garbage > 0 {
            out.push(' ');
            out.push_str(&say!(
                "maintenance-garbage",
                count = self.garbage,
                size = size_text(self.garbage_bytes)
            ));
        }
        out
    }
}

/// How many of something and their room.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Tally {
    pub count: u64,
    pub bytes: u64,
    /// Everything was counted.
    pub complete: bool,
}

impl Tally {
    fn from_json(value: &Value) -> Self {
        Self {
            count: number(value, "count"),
            bytes: number(value, "bytes"),
            complete: value.get("complete").and_then(Value::as_bool) != Some(false),
        }
    }

    /// "12 files, 3.4 MiB", with "at least" when the count stopped short.
    fn said(&self, what: Counted) -> String {
        let count = self.count;
        let size = size_text(self.bytes);
        match (what, self.complete) {
            (Counted::Files, true) => say!("maintenance-files", count, size),
            (Counted::Files, false) => say!("maintenance-files-at-least", count, size),
            (Counted::FileContents, true) => say!("maintenance-file-contents", count, size),
            (Counted::FileContents, false) => {
                say!("maintenance-file-contents-at-least", count, size)
            }
        }
    }
}

/// A worktree and its files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRow {
    /// Where it is.
    pub path: String,
    /// What it has checked out.
    pub branch: Option<String>,
    pub files: Tally,
}

/// One of the largest blobs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobRow {
    pub size: u64,
    /// Its path where it came in.
    pub path: Option<String>,
    /// The commit it came in by, and that commit's summary.
    pub commit: Option<String>,
    pub summary: Option<String>,
}

/// What `repo.size` answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// The object store.
    pub objects: Counts,
    /// Commits, trees, blobs and tags.
    pub by_type: [Tally; 4],
    /// The LFS store, when there is one.
    pub lfs: Option<Tally>,
    pub worktrees: Vec<WorktreeRow>,
    /// The largest blobs of history.
    pub largest: Vec<BlobRow>,
    /// Commits walked, and whether that was all of them.
    pub walked: u64,
    pub walk_complete: bool,
    /// The walk stopped where a partial clone left the trees of history
    /// on its remote.
    pub walk_left_on_remote: bool,
}

impl Report {
    /// Read the answer; what is not there is nothing.
    pub fn from_json(value: &Value) -> Self {
        let by_type = value.get("by_type").cloned().unwrap_or(Value::Null);
        let tally = |key: &str| by_type.get(key).map(Tally::from_json).unwrap_or_default();
        Self {
            objects: value
                .get("objects")
                .map(Counts::from_json)
                .unwrap_or_default(),
            by_type: [
                tally("commits"),
                tally("trees"),
                tally("blobs"),
                tally("tags"),
            ],
            lfs: value
                .get("lfs")
                .filter(|lfs| lfs.is_object())
                .map(Tally::from_json),
            worktrees: value
                .get("worktrees")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|tree| {
                    Some(WorktreeRow {
                        path: text(tree, "path")?,
                        branch: text(tree, "branch"),
                        files: tree.get("files").map(Tally::from_json).unwrap_or_default(),
                    })
                })
                .take(64)
                .collect(),
            largest: value
                .get("largest")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|blob| {
                    Some(BlobRow {
                        size: blob.get("size")?.as_u64()?,
                        path: text(blob, "path"),
                        commit: text(blob, "commit"),
                        summary: text(blob, "summary"),
                    })
                })
                .take(100)
                .collect(),
            walked: number(value, "walked"),
            walk_complete: value.get("walk_complete").and_then(Value::as_bool) != Some(false),
            walk_left_on_remote: value.get("walk_left_on_remote").and_then(Value::as_bool)
                == Some(true),
        }
    }

    /// The objects by type in a line.
    fn by_type_said(&self) -> String {
        let [commits, trees, blobs, tags] = self.by_type;
        say!(
            "maintenance-by-type",
            commits = commits.count,
            commits_size = size_text(commits.bytes),
            trees = trees.count,
            trees_size = size_text(trees.bytes),
            blobs = blobs.count,
            blobs_size = size_text(blobs.bytes),
            tags = tags.count,
            tags_size = size_text(tags.bytes),
        )
    }
}

/// What a task did, in a sentence: what it freed, and what became of the
/// loose objects and the packs.
pub fn freed_said(value: &Value) -> String {
    let before = value
        .get("before")
        .map(Counts::from_json)
        .unwrap_or_default();
    let after = value
        .get("after")
        .map(Counts::from_json)
        .unwrap_or_default();
    let freed = value.get("freed").and_then(Value::as_i64).unwrap_or(0);
    let room = match freed {
        0 => say!("maintenance-room-same").to_owned(),
        n if n > 0 => say!("maintenance-room-freed", size = size_text(n.unsigned_abs())),
        n => say!("maintenance-room-grew", size = size_text(n.unsigned_abs())),
    };
    say!(
        "maintenance-task-said",
        room,
        loose_before = before.loose,
        packs_before = say!("maintenance-packs-count", count = before.packs),
        loose_after = after.loose,
        packs_after = say!("maintenance-packs-count", count = after.packs),
    )
}

/// What the check found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Checked {
    /// Nothing is wrong.
    pub ok: bool,
    /// Git checked everything.
    pub finished: bool,
    /// What is wrong, as git says it, the first of it.
    pub problems: Vec<String>,
    /// How many things are wrong, and how many objects dangle.
    pub problem_count: u64,
    pub dangling_count: u64,
}

impl Checked {
    /// Read the answer of `repo.fsck`.
    pub fn from_json(value: &Value) -> Self {
        Self {
            ok: value.get("ok").and_then(Value::as_bool) == Some(true),
            finished: value.get("finished").and_then(Value::as_bool) != Some(false),
            problems: value
                .get("problems")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|problem| text(problem, "text"))
                .take(MOST_PROBLEMS)
                .collect(),
            problem_count: number(value, "problem_count"),
            dangling_count: number(value, "dangling_count"),
        }
    }

    /// The check in a sentence.
    pub fn said(&self) -> String {
        let dangling = match self.dangling_count {
            0 => String::new(),
            count => format!(" {}", say!("maintenance-dangling", count)),
        };
        let found = if self.ok {
            say!("maintenance-nothing-wrong").to_owned()
        } else if self.problem_count == 0 {
            say!("maintenance-could-not-check").to_owned()
        } else if self.finished {
            say!("maintenance-found-wrong", count = self.problem_count)
        } else {
            say!(
                "maintenance-found-wrong-unfinished",
                count = self.problem_count
            )
        };
        format!("{found}{dangling}")
    }
}

/// Where something that runs is: what git says it does, and how far.
type Stage = (String, Option<u8>);

/// How many of the problems are listed.
const MOST_PROBLEMS: usize = 20;
/// How high a row of a list is.
const LIST_ROW: f32 = 24.;
/// How wide the sizes of the largest blobs are.
const SIZE_COLUMN: f32 = 72.;

/// The maintenance view.
pub struct MaintenanceView {
    session: SessionHandle,
    /// The last measure, or why it failed.
    report: Option<Result<Report, String>>,
    /// The size is being measured.
    measuring: bool,
    /// It was measured, or asked to be, at least once.
    asked: bool,
    /// What runs, and where it is.
    running: Option<(Job, Option<Stage>)>,
    /// What each task said the last time it ran.
    done: Vec<(Task, Result<String, String>)>,
    /// What the check found the last time.
    checked: Option<Result<Checked, String>>,
}

impl EventEmitter<MaintenanceEvent> for MaintenanceView {}

impl MaintenanceView {
    /// Create the view; nothing is measured until it is shown.
    pub fn new(session: SessionHandle) -> Self {
        Self {
            session,
            report: None,
            measuring: false,
            asked: false,
            running: None,
            done: Vec::new(),
            checked: None,
        }
    }

    /// Measure the repository, unless that is under way.
    pub fn measure(&mut self, cx: &mut Context<Self>) {
        if self.measuring {
            return;
        }
        self.measuring = true;
        self.asked = true;
        cx.notify();
        let request = Request::Command {
            id: "repo.size".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            this.measuring = false;
            match reply {
                Reply::Command(value) => this.report = Some(Ok(Report::from_json(&value))),
                Reply::Error(err) if err.contains("cancelled") => {}
                Reply::Error(err) => this.report = Some(Err(err)),
                _ => {}
            }
            cx.notify();
        });
    }

    /// Stop measuring.
    fn stop_measuring(&mut self, cx: &mut Context<Self>) {
        drop(
            self.session
                .send(Request::CancelReads("repo.size".to_owned())),
        );
        cx.notify();
    }

    /// Take a measure that was made elsewhere: the off-screen tests give
    /// one of their own, so that what is drawn does not hang on the file
    /// system's block size.
    pub fn take_report(&mut self, value: &Value, cx: &mut Context<Self>) {
        self.report = Some(Ok(Report::from_json(value)));
        self.measuring = false;
        self.asked = true;
        cx.notify();
    }

    /// What the last measure said, for the tests.
    pub fn report(&self) -> Option<&Report> {
        self.report.as_ref().and_then(|report| report.as_ref().ok())
    }

    /// Whether it is measuring.
    pub fn measuring(&self) -> bool {
        self.measuring
    }

    /// A task or the check began.
    pub fn began(&mut self, job: Job, cx: &mut Context<Self>) {
        self.running = Some((job, None));
        cx.notify();
    }

    /// Where what runs is.
    pub fn progress(&mut self, stage: String, percent: Option<u8>, cx: &mut Context<Self>) {
        if let Some((_, at)) = &mut self.running {
            *at = Some((stage, percent));
            cx.notify();
        }
    }

    /// What runs, for the tests.
    pub fn running(&self) -> Option<Job> {
        self.running.as_ref().map(|(job, _)| *job)
    }

    /// What a task or the check answered. After a task the size is
    /// measured again, since that is what it changed.
    pub fn finished(&mut self, job: Job, reply: Result<Value, String>, cx: &mut Context<Self>) {
        self.running = None;
        let stopped = matches!(&reply, Err(err) if err.contains("cancelled"));
        match job {
            Job::Task(task) => {
                if !stopped {
                    let said = reply.map(|value| freed_said(&value));
                    self.done.retain(|(known, _)| *known != task);
                    self.done.push((task, said));
                }
                self.measure(cx);
            }
            Job::Check => {
                if !stopped {
                    self.checked = Some(reply.map(|value| Checked::from_json(&value)));
                }
            }
        }
        cx.notify();
    }

    /// What a task said the last time, for the tests.
    pub fn task_said(&self, task: Task) -> Option<&Result<String, String>> {
        self.done
            .iter()
            .find(|(known, _)| *known == task)
            .map(|(_, said)| said)
    }

    /// What the check found, for the tests.
    pub fn checked(&self) -> Option<&Result<Checked, String>> {
        self.checked.as_ref()
    }

    /// What runs, under the row of the job: where it is, and a button
    /// that stops it.
    fn running_line(&self, job: Job, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (running, at) = self.running.as_ref()?;
        if *running != job {
            return None;
        }
        let said = match at {
            Some((stage, Some(percent))) => format!("{stage}: {percent} %"),
            Some((stage, None)) => format!("{stage}…"),
            None => say!("maintenance-starting").to_owned(),
        };
        Some(
            h_flex()
                .gap_2()
                .items_center()
                .text_color(cx.theme().foreground)
                .child(said)
                .child(
                    Button::new("maintenance-stop")
                        .ghost()
                        .xsmall()
                        .label(say!("maintenance-stop"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(MaintenanceEvent::Stop))),
                )
                .into_any_element(),
        )
    }

    /// The size: the objects, the LFS store, the worktrees and the largest
    /// blobs.
    fn render_size(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let measure = if self.measuring {
            Button::new("maintenance-measure-stop")
                .outline()
                .small()
                .h(px(BUTTON_HEIGHT))
                .label(say!("maintenance-stop"))
                .on_click(cx.listener(|this, _, _, cx| this.stop_measuring(cx)))
        } else {
            Button::new("maintenance-measure")
                .outline()
                .small()
                .h(px(BUTTON_HEIGHT))
                .label(if self.report.is_some() {
                    say!("maintenance-measure-again")
                } else {
                    say!("maintenance-measure")
                })
                .on_click(cx.listener(|this, _, _, cx| this.measure(cx)))
        };
        let mut rows: Vec<AnyElement> = Vec::new();
        let objects_said: AnyElement = match (&self.report, self.measuring) {
            (_, true) => div()
                .child(say!("maintenance-measuring"))
                .into_any_element(),
            (None, false) => div()
                .child(say!("maintenance-not-measured"))
                .into_any_element(),
            (Some(Err(err)), false) => div()
                .text_color(theme.danger)
                .child(say!("maintenance-measure-failed", error = err))
                .into_any_element(),
            (Some(Ok(report)), false) => v_flex()
                .gap_1()
                .child(report.objects.said())
                .child(report.by_type_said())
                .into_any_element(),
        };
        rows.push(
            row(
                say!("maintenance-objects").into(),
                Parts {
                    control: Some(measure.into_any_element()),
                    under: Some(objects_said),
                },
                cx,
            )
            .into_any_element(),
        );
        let Some(Ok(report)) = &self.report else {
            return own_group(Some(say!("maintenance-size-group").into()), true, rows, cx);
        };
        if let Some(lfs) = report.lfs {
            rows.push(
                row(
                    say!("maintenance-lfs").into(),
                    Parts {
                        control: None,
                        under: Some(
                            div()
                                .child(say!(
                                    "maintenance-lfs-kept",
                                    tally = lfs.said(Counted::FileContents)
                                ))
                                .into_any_element(),
                        ),
                    },
                    cx,
                )
                .into_any_element(),
            );
        }
        let worktrees: Vec<AnyElement> = report
            .worktrees
            .iter()
            .map(|tree| {
                let name = match &tree.branch {
                    Some(branch) => format!("{} ({branch})", tree.path),
                    None => tree.path.clone(),
                };
                h_flex()
                    .w_full()
                    .h(px(LIST_ROW))
                    .gap_3()
                    .items_center()
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted)
                            .child(tree.files.said(Counted::Files)),
                    )
                    .into_any_element()
            })
            .collect();
        rows.push(
            row(
                say!("maintenance-worktrees").into(),
                Parts {
                    control: None,
                    under: Some(
                        v_flex()
                            .w_full()
                            .text_sm()
                            .text_color(theme.foreground)
                            .children(worktrees)
                            .into_any_element(),
                    ),
                },
                cx,
            )
            .into_any_element(),
        );
        let largest: Vec<AnyElement> = report
            .largest
            .iter()
            .enumerate()
            .map(|(ix, blob)| {
                let commit = blob
                    .commit
                    .as_deref()
                    .map(|id| id.get(..7).unwrap_or(id).to_owned())
                    .unwrap_or_default();
                let tip = match (&blob.commit, &blob.summary) {
                    (Some(id), Some(summary)) => say!(
                        "maintenance-came-in-by",
                        commit = id.get(..10).unwrap_or(id),
                        summary
                    ),
                    _ => say!("maintenance-not-seen").to_owned(),
                };
                h_flex()
                    .id(("maintenance-blob", ix))
                    .w_full()
                    .h(px(LIST_ROW))
                    .gap_3()
                    .items_center()
                    .tooltip(move |window, cx| {
                        gpui_kit::component::tooltip::Tooltip::new(tip.clone()).build(window, cx)
                    })
                    .child(
                        div()
                            .w(px(SIZE_COLUMN))
                            .flex_none()
                            .text_right()
                            .child(size_text(blob.size)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme.mono_font_family.clone())
                            .child(blob.path.clone().unwrap_or_else(|| "?".to_owned())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(muted)
                            .font_family(theme.mono_font_family.clone())
                            .child(commit),
                    )
                    // Asked for in the tests.
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let none = largest.is_empty();
        let walk = if report.walk_left_on_remote {
            // Measured without fetching what a partial clone left out.
            say!("maintenance-walk-left-on-remote", walked = report.walked)
        } else if report.walk_complete {
            say!("maintenance-walk-complete", walked = report.walked)
        } else {
            say!("maintenance-walk-incomplete", walked = report.walked)
        };
        rows.push(
            row(
                say!("maintenance-largest").into(),
                Parts {
                    control: None,
                    under: Some(
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                v_flex()
                                    .w_full()
                                    .text_sm()
                                    .text_color(theme.foreground)
                                    .children(largest),
                            )
                            .child(if none {
                                say!("maintenance-no-files").to_owned()
                            } else {
                                walk
                            })
                            .into_any_element(),
                    ),
                },
                cx,
            )
            .into_any_element(),
        );
        own_group(Some(say!("maintenance-size-group").into()), true, rows, cx)
    }

    /// The tasks, each a row with its button, what it does, and what it
    /// said the last time.
    fn render_tasks(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        // A bare repository is opened read-only: every task would be
        // refused.
        let read_only = self.session.summary.bare;
        let busy = self.running.is_some();
        let rows: Vec<AnyElement> = Task::ALL
            .into_iter()
            .map(|task| {
                let button = Button::new(SharedString::from(format!(
                    "maintenance-run-{}",
                    task.word()
                )))
                .outline()
                .small()
                .h(px(BUTTON_HEIGHT))
                .label(say!("maintenance-run"))
                .disabled(read_only || busy)
                .on_click(
                    cx.listener(move |_, _, _, cx| cx.emit(MaintenanceEvent::Run(Job::Task(task)))),
                );
                let last =
                    self.done.iter().find(|(known, _)| *known == task).map(
                        |(_, said)| match said {
                            Ok(said) => div()
                                .text_color(theme.foreground)
                                .child(said.clone())
                                .into_any_element(),
                            Err(err) => div()
                                .text_color(theme.danger)
                                .child(err.clone())
                                .into_any_element(),
                        },
                    );
                let running = self.running_line(Job::Task(task), cx);
                row(
                    task.label().into(),
                    Parts {
                        control: Some(button.into_any_element()),
                        under: Some(
                            v_flex()
                                .gap_1()
                                .child(task.note())
                                .children(running)
                                .children(last)
                                .into_any_element(),
                        ),
                    },
                    cx,
                )
                .into_any_element()
            })
            .collect();
        let mut group = vec![];
        if read_only {
            group.push(across(
                div()
                    .pb_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(say!("maintenance-read-only"))
                    .into_any_element(),
            ));
        }
        group.extend(rows);
        own_group(
            Some(say!("maintenance-tasks-group").into()),
            false,
            group,
            cx,
        )
    }

    /// The check: its button, and what it found.
    fn render_check(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let busy = self.running.is_some();
        let button = Button::new("maintenance-check")
            .outline()
            .small()
            .h(px(BUTTON_HEIGHT))
            .label(say!("maintenance-check"))
            .disabled(busy)
            .on_click(cx.listener(|_, _, _, cx| cx.emit(MaintenanceEvent::Run(Job::Check))));
        let found: Option<AnyElement> = self.checked.as_ref().map(|checked| match checked {
            Ok(checked) => v_flex()
                .gap_1()
                .text_color(if checked.ok {
                    theme.foreground
                } else {
                    theme.danger
                })
                .child(checked.said())
                .children(checked.problems.iter().map(|problem| {
                    div()
                        .truncate()
                        .font_family(theme.mono_font_family.clone())
                        .child(problem.clone())
                }))
                .when(
                    checked.problem_count > checked.problems.len() as u64,
                    |list| {
                        list.child(say!(
                            "maintenance-more-problems",
                            count = checked
                                .problem_count
                                .saturating_sub(checked.problems.len() as u64)
                        ))
                    },
                )
                .into_any_element(),
            Err(err) => div()
                .text_color(theme.danger)
                .child(say!("maintenance-check-failed", error = err))
                .into_any_element(),
        });
        let running = self.running_line(Job::Check, cx);
        own_group(
            Some(say!("maintenance-check-group").into()),
            false,
            vec![
                row(
                    say!("maintenance-objects-and-links").into(),
                    Parts {
                        control: Some(button.into_any_element()),
                        under: Some(
                            v_flex()
                                .gap_1()
                                .child(say!("maintenance-check-note"))
                                .children(running)
                                .children(found)
                                .into_any_element(),
                        ),
                    },
                    cx,
                )
                .into_any_element(),
            ],
            cx,
        )
    }
}

impl Render for MaintenanceView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Measured the first time it is drawn, which is when its section
        // is first shown: on the next turn, not while it is drawn.
        if !self.asked {
            self.asked = true;
            cx.spawn(async move |this, cx| {
                let _ = this.update(cx, |this, cx| this.measure(cx));
            })
            .detach();
        }
        let size = self.render_size(cx);
        let tasks = self.render_tasks(cx);
        let check = self.render_check(cx);
        v_flex()
            .id("maintenance")
            .w_full()
            .child(size)
            .child(tasks)
            .child(check)
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{Checked, Counted, Report, freed_said, size_text};

    #[test]
    fn sizes_read_as_people_read_them() {
        assert_eq!(size_text(0), "0 B");
        assert_eq!(size_text(1023), "1023 B");
        assert_eq!(size_text(1024), "1.0 KiB");
        assert_eq!(size_text(1536), "1.5 KiB");
        assert_eq!(size_text(3 * 1024 * 1024), "3.0 MiB");
        assert_eq!(size_text(5 * 1024 * 1024 * 1024), "5.0 GiB");
        assert_eq!(size_text(u64::MAX), "17179869184.0 GiB");
    }

    #[test]
    fn a_measure_is_read_and_said() {
        let report = Report::from_json(&json!({
            "objects": {
                "loose": 3, "loose_bytes": 12288, "packed": 20, "packs": 1,
                "pack_bytes": 1048576, "garbage": 1, "garbage_bytes": 1024,
            },
            "by_type": {
                "commits": { "count": 5, "bytes": 800, "complete": true },
                "blobs": { "count": 9, "bytes": 4096, "complete": true },
            },
            "lfs": { "count": 2, "bytes": 500, "complete": true },
            "worktrees": [
                { "path": "/work/one", "branch": "main", "is_main": true,
                  "files": { "count": 12, "bytes": 2048, "complete": false } },
                { "branch": "no path" },
            ],
            "largest": [
                { "id": "ab", "size": 5000, "path": "big.bin", "commit": "1234567890abcdef",
                  "summary": "add it" },
                { "id": "cd", "size": 10 },
                { "id": "ef" },
            ],
            "walked": 6,
            "walk_complete": true,
        }));
        assert_eq!(
            report.objects.said(),
            "23 objects take 1.0 MiB: 20 in 1 pack (1.0 MiB), 3 loose (12.0 KiB). 1 file \
             among them is none of git's (1.0 KiB)."
        );
        assert_eq!(report.by_type[0].count, 5);
        assert_eq!(report.by_type[1].count, 0);
        assert_eq!(report.lfs.map(|lfs| lfs.count), Some(2));
        assert_eq!(report.worktrees.len(), 1);
        assert_eq!(
            report.worktrees[0].files.said(Counted::Files),
            "at least 12 files, 2.0 KiB"
        );
        assert_eq!(report.largest.len(), 2);
        assert_eq!(report.largest[1].path, None);
        // Nonsense is an empty measure, and no LFS store.
        let nothing = Report::from_json(&json!("nonsense"));
        assert_eq!(nothing, Report::default().with_complete_walk());
        assert!(Report::from_json(&json!({ "lfs": null })).lfs.is_none());
    }

    impl Report {
        fn with_complete_walk(mut self) -> Self {
            self.walk_complete = true;
            self
        }
    }

    #[test]
    fn what_a_task_freed_is_said() {
        let said = freed_said(&json!({
            "before": { "loose": 40, "packs": 2 },
            "after": { "loose": 0, "packs": 1 },
            "freed": 2048,
        }));
        assert_eq!(
            said,
            "Freed 2.0 KiB: 40 loose and 2 packs before, 0 loose and 1 pack after."
        );
        assert!(
            freed_said(&json!({ "freed": -1024 })).starts_with("The objects take 1.0 KiB more")
        );
        assert!(freed_said(&json!({})).starts_with("The objects take as much room as before"));
    }

    #[test]
    fn what_the_check_found_is_said() {
        let sound = Checked::from_json(&json!({
            "ok": true, "finished": true, "problems": [], "problem_count": 0,
            "dangling_count": 3,
        }));
        assert!(sound.ok);
        assert_eq!(
            sound.said(),
            "Nothing is wrong. 3 objects dangle: nothing refers to them, and gc removes them \
             once they are old enough."
        );
        let broken = Checked::from_json(&json!({
            "ok": false, "finished": false,
            "problems": [{ "text": "missing blob 12" }, { "kind": "error" }],
            "problem_count": 30, "dangling_count": 1,
        }));
        assert_eq!(broken.problems, ["missing blob 12"]);
        assert!(
            broken
                .said()
                .starts_with("Git found 30 things wrong, and could not check the rest.")
        );
        assert!(!Checked::from_json(&json!(null)).ok);
    }
}
