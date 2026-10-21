//! One open repository: session, references panel, graph, diff viewer, and
//! either commit details or the changes panel with the commit box.

use std::sync::Arc;

use bitbraid_git::{
    Area, ChangeKind, Oid, Order, Snapshot, StatusEntry, Tips, sequence::InProgress,
};
use futures::StreamExt as _;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::{
    notices::Notice,
    prefs::{Prefs, RepoState},
    session::{Event, GraphData, Reply, Request, SessionHandle, StartupReplies, is_dirty},
    views::{
        activity::{ActivityEvent, ActivityView},
        blame::{BlameEvent, BlameView},
        changes::{ChangesEvent, ChangesView, Side},
        commit_box::{CommitBox, CommitEvent},
        details::{DetailsEvent, DetailsView},
        diff::{DiffEvent, DiffSource, DiffView},
        edit::{EditEvent, EditView},
        find_file::{FindFile, FindFileEvent},
        flow::{self, FlowRequest, FlowState},
        graph::{BranchRequest, GraphEvent, GraphView, NameFor, WipAction},
        history::{HistoryEvent, HistoryView},
        index_flags::{FlagsEvent, FlagsView},
        issues::{IssueView, IssuesEvent, IssuesView},
        journal::{JournalEvent, JournalView},
        lfs::{LfsEvent, LfsStatus, LfsView},
        merge_tool::{MergeEvent, MergeTool},
        pulls::{NewPullView, PullView, PullsEvent, PullsView},
        rebase_editor::{RebaseEditor, RebaseEvent},
        refs::{RefsEvent, RefsView, RemoteRequest},
        rerere::{RerereEvent, RerereView},
        rewrite::{Choice, Meeting, Rewrite, conflict_warning},
        signing::{SigningEvent, SigningView},
        stashes::StashRequest,
        submodules::SubmoduleRequest,
        tags::TagRequest,
        worktrees::{self, WorktreeRequest},
    },
};

mod asking;
mod branches;
mod deleted;
mod docked;
mod file_pane;
mod files;
mod flowing;
mod forking;
pub(crate) mod form;
mod from_inbox;
pub(crate) mod hooks;
mod ignoring;
mod issuing;
mod leaving;
mod looking;
mod maintaining;
mod marks;
mod modules;
mod moves;
mod naming;
mod past;
mod patches;
mod platform;
mod pulling;
mod running;
mod signers;
mod stashing;
mod sync;
mod tagging;
pub mod terminals;
mod trailers;
mod unfolding;

pub use platform::PlatformNotes;
pub use sync::PushHow;

// What the tab has on show, for the off-screen tests.
mod probe;
pub use probe::{WorkspaceProbe, WorktreeLook};

/// A border between panes that is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Split {
    /// Between the references panel and the middle.
    Left,
    /// Between the middle and the right panel.
    Right,
}

/// Nothing follows the pointer while a border is dragged.
struct NoChip;

impl Render for NoChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The least time between two reads of where the submodules stand, while
/// files keep changing in them: each is a handful of git processes.
const SUBMODULE_GAP: std::time::Duration = std::time::Duration::from_secs(2);

/// How narrow and how wide the left panel can be dragged: no narrower
/// than it starts out, and up to 793, at any window size and beside a
/// right panel of any width (see `MIDDLE_KEPT` for the one bound beyond).
const SIDE_MIN: f32 = 208.0;
const SIDE_MAX: f32 = 793.0;

/// How wide the right panel is, its rule included, until it is dragged:
/// the same in a small window and in a large one.
const RIGHT_WIDTH: f32 = 400.0;
/// How narrow the right panel can be dragged.
const RIGHT_MIN: f32 = 353.0;
/// How wide the right panel can be dragged: as wide as leaves the middle
/// this much beside a left panel at its least, whatever the left panel
/// takes (folded, at its least or wide): the window's width less 687.
const MIDDLE_LEAST: f32 = 479.0;
/// The least the middle keeps whatever the two panels were dragged to:
/// what a window 1280 wide leaves it with the left panel at its widest
/// beside the right one as it starts out. Dragged further (both wide, or
/// in a smaller window), the panel being dragged stops: the graph does
/// not go, nor does the right panel run out of the window.
const MIDDLE_KEPT: f32 = 87.0;

/// The left panel's width: between the limits of a side panel, and no
/// wider than leaves the middle what it keeps beside a right panel that
/// takes `right`, so that the two panels cannot be dragged until the graph
/// is gone, nor keep a width that leaves no graph in a smaller window.
// Not `clamp`: of a width that is no number it makes no number, and the
// clamp under it panics on one for a bound.
#[allow(clippy::manual_clamp)]
fn dragged_left_width(wanted: f32, window: f32, right: f32) -> f32 {
    let most = (window - right - MIDDLE_KEPT).max(SIDE_MIN).min(SIDE_MAX);
    if wanted.is_finite() {
        wanted.clamp(SIDE_MIN, most)
    } else {
        SIDE_MIN
    }
}

/// What the graph keeps in a narrow window before the references panel
/// gives way to its strip: its three columns wide enough for a label, the
/// lanes and the start of a message. 1280 at 160 % (800 points) leaves it
/// that beside both panels at their least; at 175 % (731) it would not.
const GRAPH_ROOM: f32 = 200.0;

/// Whether a window `window` wide is too narrow for the references panel
/// at its least, the graph at [`GRAPH_ROOM`] and the right panel at its
/// least side by side: the references panel is a strip then. A width that
/// is no number is wide enough.
fn too_narrow_for_both(window: f32) -> bool {
    window < SIDE_MIN + GRAPH_ROOM + RIGHT_MIN
}

/// Whether the row of uncommitted work is what gets selected: when the
/// first status of a repository that was just opened lists something,
/// unless the middle shows the file of a commit again, whose commit the
/// selection then stays with. Later statuses select nothing.
fn opens_on_uncommitted_work(first_status: bool, dirty: bool, commit_file: bool) -> bool {
    first_status && dirty && !commit_file
}

/// The right panel's width: what was asked for, no narrower than its least,
/// no wider than its own most in a window `window` wide, and no wider than
/// leaves the middle what it keeps beside a left panel that takes `left`.
/// In a window too narrow for both, the panel keeps its least.
fn right_width(wanted: f32, window: f32, left: f32) -> f32 {
    let most = (window - SIDE_MIN - MIDDLE_LEAST)
        .min(window - left - MIDDLE_KEPT)
        .max(RIGHT_MIN);
    if wanted.is_finite() {
        wanted.clamp(RIGHT_MIN, most)
    } else {
        RIGHT_WIDTH.min(most)
    }
}

/// What the right pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RightPane {
    /// Details of the selected commit.
    Commit,
    /// Changes and the commit box (the row of uncommitted work selected).
    Wip,
    /// What is uncommitted in another worktree (its row selected).
    Worktree,
}

/// What the file pane under the graph shows for the current file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileTab {
    /// Its diff.
    Diff,
    /// The commits that changed it.
    History,
    /// Who last changed each line.
    Blame,
    /// The worktree file itself, to change it.
    Edit,
    /// The two sides of the file while it is in conflict.
    Merge,
}

/// A view that takes the centre for as long as it is looked at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sheet {
    /// The git processes that were run.
    Activity,
    /// What can be undone and redone.
    Journal,
    /// Files git was told not to look at.
    Flags,
    /// What rerere has recorded.
    Rerere,
    /// A pull request.
    Pull,
    /// A pull request being made.
    NewPull,
    /// An issue.
    Issue,
}

/// An operation that is running: what it is, and what git last said.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Operation {
    /// What the user started, for example "Fetching".
    label: String,
    /// The engine's command, when the user started it from a control
    /// (none for what runs in the background or waited in the queue).
    command: Option<String>,
    /// The stage git reported, with a percentage when it gave one.
    stage: Option<(String, Option<u8>)>,
}

/// What goes on once a conflict warning is waved through, or was not needed.
type GoOn = std::rc::Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

/// What a confirmed prompt does with what was typed, one text per field.
type PromptAction =
    std::rc::Rc<dyn Fn(&mut Workspace, Vec<String>, &mut Window, &mut Context<Workspace>)>;

/// One text field of a prompt.
pub(crate) struct PromptField {
    /// Shown while the field is empty; says what goes there.
    placeholder: &'static str,
    /// What the field starts with.
    initial: String,
    /// The prompt cannot be confirmed while this is empty.
    required: bool,
}

impl PromptField {
    fn required(placeholder: &'static str, initial: &str) -> Self {
        Self {
            placeholder,
            initial: initial.to_owned(),
            required: true,
        }
    }
    fn optional(placeholder: &'static str) -> Self {
        Self {
            placeholder,
            initial: String::new(),
            required: false,
        }
    }
}

/// A program outside the application that the repository's folder is
/// opened in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum Outside {
    /// The system's file manager.
    FileManager,
    /// The editor the preferences name.
    Editor,
    /// The terminal the preferences name, or the one found.
    Terminal,
}

/// Moving the content of large files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
pub enum LfsTransfer {
    /// Download without touching the worktree.
    Fetch,
    /// Download what is checked out and put it in place.
    Pull,
    /// Upload the current branch's.
    Push,
    /// Put what was fetched in place of the pointers; nothing is moved
    /// over the network.
    Checkout,
}

impl LfsTransfer {
    /// The engine command, what the status bar says while it runs, and
    /// what is said when it is done.
    pub fn command(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Fetch => (
                "lfs.fetch",
                say!("workspace-lfs-fetching"),
                say!("workspace-lfs-fetched"),
            ),
            Self::Pull => (
                "lfs.pull",
                say!("workspace-lfs-pulling"),
                say!("workspace-lfs-pulled"),
            ),
            Self::Push => (
                "lfs.push",
                say!("workspace-lfs-pushing"),
                say!("workspace-lfs-pushed"),
            ),
            Self::Checkout => (
                "lfs.checkout",
                say!("workspace-lfs-checking-out"),
                say!("workspace-lfs-checked-out"),
            ),
        }
    }
}

/// What a workspace asks of the window it is a tab of.
#[derive(Debug, Clone)]
pub enum WorkspaceEvent {
    /// Show a section of the preferences: gitflow's, for one.
    OpenSettings(crate::views::preferences::Section),
    /// Open the repository at this path in a tab: a worktree, a submodule.
    Open(std::path::PathBuf),
    /// Open the repository at this path in the tab this workspace is in,
    /// in its place: another worktree of the same repository.
    OpenInPlace(std::path::PathBuf),
    /// Open the repository at this path in a tab and show a commit in it:
    /// a commit of a submodule.
    OpenAt {
        /// The repository's folder.
        path: std::path::PathBuf,
        /// The commit to show there.
        commit: String,
    },
}

impl EventEmitter<WorkspaceEvent> for Workspace {}

/// The per-repository view.
pub struct Workspace {
    session: SessionHandle,
    snapshot: Option<Snapshot>,
    graph: Option<Arc<GraphData>>,
    /// Graph requests the session has not answered yet.
    graph_loads: usize,
    /// A larger piece of a history whose first piece is still on its way
    /// here as the answer to a request.
    graph_ahead: Option<Arc<GraphData>>,
    status: Vec<StatusEntry>,
    refs_view: Entity<RefsView>,
    graph_view: Entity<GraphView>,
    details_view: Entity<DetailsView>,
    changes_view: Entity<ChangesView>,
    commit_box: Entity<CommitBox>,
    diff_view: Entity<DiffView>,
    history_view: Entity<HistoryView>,
    /// The finder of files by their path, shown in a dialog.
    find_file: Entity<FindFile>,
    blame_view: Entity<BlameView>,
    edit_view: Entity<EditView>,
    merge_tool: Entity<MergeTool>,
    rebase_editor: Entity<RebaseEditor>,
    activity_view: Entity<ActivityView>,
    journal_view: Entity<JournalView>,
    rerere_view: Entity<RerereView>,
    signing_view: Entity<SigningView>,
    lfs_view: Entity<LfsView>,
    /// The maintenance section and the sparse checkout (`maintaining`).
    upkeep: maintaining::Upkeep,
    flags_view: Entity<FlagsView>,
    /// Where LFS stands here, as last asked: the toolbar shows its button
    /// while a pattern is tracked.
    lfs_status: LfsStatus,
    /// A view that has the centre at present, in place of the graph.
    sheet: Option<Sheet>,
    /// The rows of the pull requests section, and the pull request shown
    /// in the middle while the sheet is its (`workspace/pulling.rs`).
    pulls: Entity<PullsView>,
    pull_view: Option<Entity<PullView>>,
    pull_subscription: Option<Subscription>,
    new_pull: Option<Entity<NewPullView>>,
    new_pull_subscription: Option<Subscription>,
    /// The same for the issues section and an issue
    /// (`workspace/issuing.rs`).
    issues: Entity<IssuesView>,
    issue_view: Option<Entity<IssueView>>,
    issue_subscription: Option<Subscription>,
    /// The pane over the left of the centre that adds a remote, a
    /// worktree or a submodule (`docked.rs`).
    docked: Option<docked::Docked>,
    /// An interactive rebase is being planned: the editor has the centre.
    planning: bool,
    /// The merge, rebase, cherry-pick or revert that is unfinished.
    in_progress: Option<InProgress>,
    /// The gitflow branch whose finish stopped in a merge that conflicts:
    /// its kind, its name and whether it is kept. When that merge is
    /// concluded the finish goes on with the steps that are left.
    finishing: Option<(String, String, bool)>,
    /// Pushes go without the `pre-push` hook. Kept while the tab is open,
    /// said on the button and in its menu for as long as it is on, and
    /// never written anywhere: a new start runs the hook again.
    skip_push_hook: bool,
    /// The mailbox that is being asked for is applied without the hooks
    /// that could refuse a patch: ticked in its dialog, for that one.
    skip_mailbox_hooks: bool,
    /// The file the pane under the graph is about: its path, and the commit
    /// it was opened from (absent for a file of the worktree).
    file: Option<(String, Option<Oid>)>,
    file_tab: FileTab,
    /// The first text field of the prompts (new branch, rename), and what to
    /// do with the name once it is confirmed.
    prompt: Entity<InputState>,
    /// The field of prompts that may stay empty (a stash message), which
    /// says so itself.
    prompt_optional: Entity<InputState>,
    /// The forms this tab asks in: the fields of their rows, and the one
    /// that is open (`workspace/form.rs`).
    forms: form::Forms<Workspace>,
    prompt_action: Option<PromptAction>,
    /// For each field of the open prompt, whether it has to be filled in.
    prompt_required: Vec<bool>,
    /// Gitflow as it is set up here, as last asked.
    flow_state: FlowState,
    /// Whether the preferences show this repository's sections.
    settings_shown: bool,
    /// When the things around the references were last asked for (changes
    /// in other worktrees, submodules, gitflow, branch notes, LFS). Each is
    /// a git process or several, so they are not asked again with every
    /// reload; none when they are to be asked whatever the time.
    places_loaded: Option<std::time::Instant>,
    /// Keeps the reads of where the submodules stand apart while files
    /// change in them.
    submodule_reads: crate::views::submodules::Paced,
    /// The operation on the session's operations thread, if any, and those
    /// waiting behind it.
    operation: Option<Operation>,
    operations_waiting: std::collections::VecDeque<Operation>,
    /// The command that changes the repository on the session thread.
    busy: running::Busy,
    /// The worktrees being removed, by path.
    removing: std::collections::BTreeSet<String>,
    right: RightPane,
    /// The last status held as many paths as are taken and there are more.
    status_cut: bool,
    /// The banner that says the repository is a large one was put away.
    large_hidden: bool,
    /// What the system has to say of the repository.
    platform: PlatformNotes,
    /// The other worktree whose row of uncommitted work is selected.
    selected_worktree: Option<String>,
    /// Whether the other worktrees were asked for their changes in this
    /// session.
    worktrees_asked: bool,
    /// Ask every worktree afresh the next time: the user asked.
    worktrees_afresh: bool,
    /// What this tab wants of the looks at the other worktrees (see
    /// `looking.rs`).
    worktree_wants: looking::Wants,
    /// How many times the references and the history were read again.
    reloads: u64,
    show_diff: bool,
    /// In the view of a file's past: the commit of the list that is shown
    /// beside it, and what the file pane showed before, which closing the
    /// view goes back to (none: back to the graph).
    past_entry: Option<past::PastEntry>,
    past_back: Option<DiffSource>,
    /// Whether the file pane showed the file as it is when the past was
    /// opened: it does again when the past is closed.
    past_file_view: bool,
    /// The file in the centre closes when the editor has written it: what
    /// was asked when it was closed with changes not saved.
    close_after_save: bool,
    /// How far the checked-out branch is ahead of and behind its upstream,
    /// when it is not level with it.
    ahead_behind: Option<(u32, u32)>,
    /// The file this tab had open the last time, until it is open again or
    /// found to be gone; and whether the first status is in, which says
    /// whether a file of the worktree is still changed.
    file_to_restore: Option<crate::prefs::OpenFile>,
    status_loaded: bool,
    undo_label: Option<String>,
    redo_label: Option<String>,
    /// Which journal entry undo would take, as last asked.
    undo_id: Option<u64>,
    /// Whether undo, or redo, would talk to a remote (a branch deleted
    /// there is pushed back): it is then run where it can take its time.
    undo_remote: bool,
    redo_remote: bool,
    /// The timer that fetches in the background.
    auto_fetch: sync::AutoFetch,
    /// The window's terminals, as far as this tab draws them.
    terminal: terminals::TerminalLink,
    /// Which local branches would conflict with the checked-out one.
    conflict_marks: marks::ConflictMarks,
    loading: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

/// The graph request for a repository's remembered visibility. References
/// shown alone are looked up in `snapshot`; without one the request is only
/// known when nothing is shown alone.
pub fn graph_request(
    repo_state: &RepoState,
    snapshot: Option<&Snapshot>,
    order: Order,
) -> Option<Request> {
    let all = || Request::Graph {
        tips: Tips::All,
        exclude: repo_state.hidden.clone(),
        pinned: repo_state.pinned.clone(),
        order,
        worktree_rows: repo_state.worktree_rows(),
    };
    if repo_state.only.is_empty() {
        return Some(all());
    }
    let snapshot = snapshot?;
    let mut ids = Vec::new();
    // By itself, or with its folder as a whole.
    let soloed = |full: &str| {
        repo_state.only.iter().any(|name| name == full)
            || crate::views::refs::under_remote(&repo_state.only, full)
    };
    for b in &snapshot.branches {
        if soloed(&b.full_name) {
            ids.push(b.target);
        }
    }
    for r in &snapshot.remotes {
        for b in &r.branches {
            // By itself, or with its remote or folder as a whole.
            if soloed(&b.full_name) {
                ids.push(b.target);
            }
        }
    }
    for t in &snapshot.tags {
        if soloed(&format!("refs/tags/{}", t.name)) {
            ids.push(t.target);
        }
    }
    Some(if ids.is_empty() {
        all()
    } else {
        // What is pinned stays pinned while something is shown alone: a pin
        // holds whenever its branch is in the graph that is shown.
        Request::Graph {
            tips: Tips::Ids(ids),
            exclude: Vec::new(),
            pinned: repo_state.pinned.clone(),
            order,
            worktree_rows: repo_state.worktree_rows(),
        }
    })
}

impl Workspace {
    /// Build the views for an open session. `startup` holds the replies to
    /// the first status, references and graph requests when they were sent
    /// ahead of the window.
    pub fn new(
        session: SessionHandle,
        mut events: futures::channel::mpsc::UnboundedReceiver<Event>,
        startup: Option<StartupReplies>,
        bench: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let refs_view = cx.new(|cx| RefsView::new(session.clone(), window, cx));
        let pulls = cx.new(|cx| PullsView::new(session.clone(), window, cx));
        refs_view.update(cx, |view, cx| view.set_pulls(pulls.clone(), cx));
        let issues = cx.new(|cx| IssuesView::new(session.clone(), window, cx));
        refs_view.update(cx, |view, cx| view.set_issues(issues.clone(), cx));
        let graph_view = cx.new(|cx| GraphView::new(session.clone(), bench, window, cx));
        let details_view = cx.new(|cx| DetailsView::new(session.clone(), window, cx));
        let changes_view = cx.new(|cx| ChangesView::new(session.clone(), window, cx));
        let commit_box = cx.new(|cx| CommitBox::new(session.clone(), window, cx));
        // The row of uncommitted work shows the summary that is typed.
        let summary = commit_box.read(cx).summary_field();
        graph_view.update(cx, |view, _| view.set_summary_source(summary));
        let diff_view = cx.new(|cx| DiffView::new(session.clone(), window, cx));
        let history_view = cx.new(|cx| HistoryView::new(session.clone(), window, cx));
        let find_file = cx.new(|cx| FindFile::new(session.clone(), window, cx));
        let blame_view = cx.new(|cx| BlameView::new(session.clone(), window, cx));
        let edit_view = cx.new(|cx| EditView::new(session.clone(), cx));
        let merge_tool = cx.new(|cx| MergeTool::new(session.clone(), window, cx));
        let rerere_view = cx.new(|_| RerereView::new(session.clone()));
        let rebase_editor = cx.new(|cx| RebaseEditor::new(session.clone(), window, cx));
        let activity_view = cx.new(|cx| ActivityView::new(session.clone(), window, cx));
        let journal_view = cx.new(|_| JournalView::new(session.clone()));
        let signing_view = cx.new(|cx| SigningView::new(session.clone(), cx));
        let lfs_view = cx.new(|cx| LfsView::new(session.clone(), window, cx));
        let upkeep = maintaining::Upkeep::new(&session, window, cx);
        let flags_view = cx.new(|cx| FlagsView::new(session.clone(), cx));
        let prompt =
            cx.new(|cx| InputState::new(window, cx).placeholder(say!("workspace-prompt-name")));
        let prompt_optional = cx.new(|cx| {
            InputState::new(window, cx).placeholder(say!("workspace-prompt-message-optional"))
        });

        let subscriptions = vec![
            // What another tab found in a worktree this one shows.
            cx.observe_global::<looking::Looks>(|this, cx| this.adopt_shared_looks(cx)),
            cx.subscribe_in(&pulls, window, |this, _, event: &PullsEvent, window, cx| {
                this.pulls_event(event, window, cx)
            }),
            cx.subscribe_in(
                &issues,
                window,
                |this, _, event: &IssuesEvent, window, cx| this.issues_event(event, window, cx),
            ),
            cx.subscribe_in(
                &graph_view,
                window,
                |this, _, event: &GraphEvent, window, cx| match event {
                    GraphEvent::Selected { id, asked } => this.on_selected(*id, *asked, window, cx),
                    GraphEvent::SelectedMany {
                        ids,
                        comparison,
                        asked,
                    } => {
                        this.right = RightPane::Commit;
                        this.details_view.update(cx, |view, cx| {
                            view.show_many(ids.clone(), *comparison, *asked, cx)
                        });
                        cx.notify();
                    }
                    GraphEvent::SelectedWip => {
                        this.right = RightPane::Wip;
                        cx.notify();
                    }
                    GraphEvent::SelectedWorktree(path) => {
                        this.right = RightPane::Worktree;
                        this.selected_worktree = Some(path.clone());
                        cx.notify();
                    }
                    GraphEvent::Checkout(target) => this.checkout(target.clone(), window, cx),
                    GraphEvent::OpenIssue { link, number } => {
                        this.open_issue(link.clone(), *number, window, cx)
                    }
                    GraphEvent::NewBranch(start) => {
                        this.prompt_new_branch(Some(start.clone()), window, cx)
                    }
                    GraphEvent::Rewrite(rewrite) => this.rewrite(rewrite.clone(), window, cx),
                    GraphEvent::Stash(request) => this.stash(request.clone(), window, cx),
                    GraphEvent::Tag(request) => this.tag(request.clone(), window, cx),
                    GraphEvent::Worktree(request) => this.worktree(request.clone(), window, cx),
                    GraphEvent::ExportPatches(commits) => {
                        this.export_patches(commits.clone(), window, cx)
                    }
                    GraphEvent::Dropped(choice) => this.run_choice(choice.clone(), window, cx),
                    GraphEvent::Branch(request) => this.branch_from_label(request, window, cx),
                    GraphEvent::Seen(change) => {
                        let change = change.clone();
                        this.refs_view.update(cx, |view, cx| view.see(change, cx));
                    }
                    GraphEvent::Wip(action) => this.wip_action(*action, window, cx),
                    GraphEvent::WriteSummary => {
                        this.right = RightPane::Wip;
                        this.commit_box
                            .update(cx, |view, cx| view.focus_summary(window, cx));
                        cx.notify();
                    }
                    GraphEvent::Named { what, start, name } => {
                        this.named(*what, start.clone(), name.clone(), window, cx)
                    }
                },
            ),
            cx.subscribe_in(
                &refs_view,
                window,
                |this, _, event: &RefsEvent, window, cx| match event {
                    RefsEvent::Reveal(id) => this.reveal(*id, window, cx),
                    RefsEvent::VisibilityChanged => this.load_graph(cx),
                    RefsEvent::Checkout(target) => this.checkout(target.clone(), window, cx),
                    RefsEvent::NewBranch(start) => {
                        this.prompt_new_branch(Some(start.clone()), window, cx)
                    }
                    RefsEvent::Rename(name) => this.prompt_rename_branch(name.clone(), window, cx),
                    RefsEvent::Delete(name) => this.confirm_delete_branch(name.clone(), window, cx),
                    RefsEvent::Rewrite(rewrite) => this.rewrite(rewrite.clone(), window, cx),
                    RefsEvent::Stash(request) => this.stash(request.clone(), window, cx),
                    RefsEvent::Tag(request) => this.tag(request.clone(), window, cx),
                    RefsEvent::Remote(request) => this.remote(request.clone(), window, cx),
                    RefsEvent::DeleteMany(names) => {
                        this.confirm_delete_branches(names.clone(), window, cx)
                    }
                    RefsEvent::PushMany(names) => this.push_branches(names, window, cx),
                    RefsEvent::PushWith(name, how) => {
                        this.push_how(Some(name.clone()), how.clone(), window, cx)
                    }
                    RefsEvent::Pull(name) => this.pull_branch(name.clone(), window, cx),
                    RefsEvent::SetUpstream(name) => this.prompt_upstream(name.clone(), window, cx),
                    RefsEvent::ClearUpstream(name) => this.run_mutation(
                        "branch.set_upstream",
                        json!({ "name": name, "upstream": null }),
                        say!("workspace-upstream-cleared", name),
                        |_, _, _, _| false,
                        window,
                        cx,
                    ),
                    RefsEvent::DeleteOnRemote(name) => {
                        this.confirm_delete_on_remote(name.clone(), window, cx)
                    }
                    RefsEvent::Worktree(request) => this.worktree(request.clone(), window, cx),
                    RefsEvent::Submodule(request) => this.submodule(request.clone(), window, cx),
                    RefsEvent::Flow(request) => this.flow(request.clone(), window, cx),
                    RefsEvent::CountWorktrees => this.want_worktree_looks(true, cx),
                    RefsEvent::LocalShown => this.mark_conflicts(cx),
                    RefsEvent::Compare(id) => this.compare_commit_with_worktree(*id, window, cx),
                    // What a drop on a row of the panel can mean is offered
                    // where a drop on a row of the graph offers it.
                    RefsEvent::Dropped(dragged, target) => this
                        .graph_view
                        .update(cx, |view, cx| view.dropped(dragged, target, window, cx)),
                    RefsEvent::Hover(id, on) => this
                        .graph_view
                        .update(cx, |view, cx| view.highlight_from_panel(*id, *on, cx)),
                    RefsEvent::RefreshPulls => this.pulls.update(cx, |view, cx| view.refresh(cx)),
                    RefsEvent::RefreshIssues => this.issues.update(cx, |view, cx| view.refresh(cx)),
                    RefsEvent::OpenIssue { link, number } => {
                        this.open_issue(link.clone(), *number, window, cx)
                    }
                    RefsEvent::OpenAsked => this.open_references(window, cx),
                    RefsEvent::FastForward(name) => this.run_mutation(
                        "branch.fast_forward",
                        json!({ "name": name }),
                        say!("workspace-fast-forwarded", name),
                        |_, _, _, _| false,
                        window,
                        cx,
                    ),
                },
            ),
            cx.subscribe_in(
                &details_view,
                window,
                |this, _, event: &DetailsEvent, window, cx| match event {
                    DetailsEvent::Reveal(id) => this.reveal(*id, window, cx),
                    DetailsEvent::OpenIssue { link, number } => {
                        this.open_issue(link.clone(), *number, window, cx)
                    }
                    DetailsEvent::ShowUncommitted => {
                        this.right = RightPane::Wip;
                        this.graph_view.update(cx, |view, cx| view.select_wip(cx));
                        cx.notify();
                    }
                    DetailsEvent::OpenFile { id, diff } => {
                        let source = DiffSource::Commit {
                            id: *id,
                            path: diff.path.clone(),
                            diff: diff.clone(),
                        };
                        if this.toggle_file(diff.path.clone(), Some(*id), source, window, cx) {
                            this.details_view
                                .update(cx, |view, cx| view.unmark_file(cx));
                        }
                    }
                    DetailsEvent::OpenRange { old, new, diff } => {
                        let source = DiffSource::Range {
                            old: *old,
                            new: *new,
                            path: diff.path.clone(),
                            diff: diff.clone(),
                        };
                        if this.toggle_file(diff.path.clone(), Some(*new), source, window, cx) {
                            this.details_view
                                .update(cx, |view, cx| view.unmark_file(cx));
                        }
                    }
                    DetailsEvent::File(action) => this.file_action(action.clone(), window, cx),
                    DetailsEvent::AllowSigner { id, address } => {
                        this.prompt_allow_signer_of(*id, address.clone(), window, cx)
                    }
                    DetailsEvent::OpenAgainstWorktree { old, diff } => {
                        let source = DiffSource::AgainstWorktree {
                            old: *old,
                            path: diff.path.clone(),
                            diff: diff.clone(),
                        };
                        // A file of the worktree: its history and blame
                        // are the ones of what is checked out.
                        if this.toggle_file(diff.path.clone(), None, source, window, cx) {
                            this.details_view
                                .update(cx, |view, cx| view.unmark_file(cx));
                        }
                    }
                },
            ),
            cx.subscribe_in(
                &changes_view,
                window,
                |this, _, event: &ChangesEvent, window, cx| match event {
                    // The middle goes back to the graph when several files
                    // are selected, unless it shows something else.
                    ChangesEvent::Several => {
                        let working = matches!(
                            this.diff_view.read(cx).source(),
                            Some(DiffSource::Working { .. })
                        );
                        if this.show_diff && working {
                            this.close_file(window, cx);
                        }
                    }
                    ChangesEvent::Clicked { path, side } => {
                        let source = DiffSource::Working {
                            path: path.clone(),
                            side: *side,
                        };
                        if this.toggle_file(path.clone(), None, source, window, cx) {
                            this.changes_view.update(cx, |view, cx| view.unmark(cx));
                        }
                    }
                    ChangesEvent::Open { path, side } => {
                        let source = DiffSource::Working {
                            path: path.clone(),
                            side: *side,
                        };
                        this.open_file(path.clone(), None, source, cx);
                    }
                    ChangesEvent::Mutated => this.after_mutation(cx),
                    ChangesEvent::OpenConflict { path } => {
                        this.open_conflict(path.clone(), window, cx)
                    }
                    ChangesEvent::Ignore(request) => this.ignore(request.clone(), window, cx),
                    ChangesEvent::File(action) => this.file_action(action.clone(), window, cx),
                    ChangesEvent::Discard(paths) => this.ask_discard(paths.clone(), window, cx),
                },
            ),
            cx.subscribe_in(
                &diff_view,
                window,
                |this, _, event: &DiffEvent, window, cx| match event {
                    DiffEvent::Mutated => this.after_mutation(cx),
                    DiffEvent::OpenSubmoduleCommit { path, id } => {
                        this.open_submodule_commit(path, id, window, cx)
                    }
                },
            ),
            cx.subscribe_in(
                &prompt,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    // Enter in the name field confirms the prompt.
                    if matches!(event, InputEvent::PressEnter { .. })
                        && this.confirm_prompt(window, cx)
                    {
                        window.close_dialog(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &prompt_optional,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. })
                        && this.confirm_prompt(window, cx)
                    {
                        window.close_dialog(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &edit_view,
                window,
                |this, _, event: &EditEvent, _, cx| match event {
                    // The file changed under the diff and the status.
                    EditEvent::Saved => {
                        this.after_mutation(cx);
                        this.peek_undo(cx);
                        this.saved(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &rebase_editor,
                window,
                |this, _, event: &RebaseEvent, window, cx| {
                    // While the plan is made, the details beside it are
                    // about the commit that is selected in it.
                    if let RebaseEvent::Selected(id) = event {
                        let badges = this
                            .graph
                            .as_ref()
                            .and_then(|graph| graph.badges.get(id).cloned())
                            .unwrap_or_default();
                        this.right = RightPane::Commit;
                        this.details_view
                            .update(cx, |view, cx| view.show(Some(*id), badges, true, cx));
                        cx.notify();
                        return;
                    }
                    this.planning = false;
                    // The details are the graph's again.
                    this.graph_view
                        .update(cx, |view, cx| view.say_selection(cx));
                    if let RebaseEvent::Run {
                        first,
                        steps,
                        onto,
                        no_verify,
                    } = event
                    {
                        let params = json!({ "first": first, "steps": steps, "onto": onto });
                        this.run_operation(
                            "rebase.interactive",
                            if *no_verify {
                                hooks::without(params)
                            } else {
                                params
                            },
                            say!("workspace-rebasing"),
                            say!("workspace-rebased").to_owned(),
                            window,
                            cx,
                        );
                    }
                    if let RebaseEvent::Reword { commit, message } = event {
                        let short: String = commit.chars().take(7).collect();
                        this.run_operation(
                            "commit.reword",
                            json!({ "commit": commit, "message": message }),
                            say!("workspace-rewording"),
                            say!("workspace-reworded", short),
                            window,
                            cx,
                        );
                    }
                    cx.notify();
                },
            ),
            cx.subscribe_in(
                &activity_view,
                window,
                |this, _, event: &ActivityEvent, _, cx| match event {
                    ActivityEvent::Close => {
                        this.sheet = None;
                        cx.notify();
                    }
                },
            ),
            cx.subscribe_in(
                &rerere_view,
                window,
                |this, _, event: &RerereEvent, window, cx| match event {
                    RerereEvent::Close => {
                        this.sheet = None;
                        cx.notify();
                    }
                    RerereEvent::Said { text, ok } => {
                        crate::notices::push(
                            window,
                            cx,
                            if *ok {
                                Notice::success(text.clone())
                            } else {
                                Notice::error(sentence(text))
                            },
                        );
                        // A file that rerere had resolved may be one it
                        // has not resolved now.
                        this.after_mutation(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &journal_view,
                window,
                |this, _, event: &JournalEvent, window, cx| match event {
                    JournalEvent::Close => {
                        this.sheet = None;
                        cx.notify();
                    }
                    // The repository is somewhere else in its history now.
                    JournalEvent::Walked { said, complete } => {
                        crate::notices::push(
                            window,
                            cx,
                            if *complete {
                                Notice::success(said.clone())
                            } else {
                                Notice::warning(sentence(said))
                            },
                        );
                        this.after_mutation(cx);
                        this.reload(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &signing_view,
                window,
                |this, _, event: &SigningEvent, window, cx| match event {
                    SigningEvent::Changed(said) => {
                        crate::notices::push(window, cx, Notice::success(said.clone()));
                        // The commit box says whether it will sign.
                        this.commit_box.update(cx, |view, cx| view.load_signing(cx));
                    }
                    SigningEvent::Refused(err) => {
                        crate::notices::push(window, cx, Notice::error(sentence(err)))
                    }
                    SigningEvent::ChooseSignersFile => this.prompt_signers_file(None, window, cx),
                    SigningEvent::AddSigner => this.prompt_add_signer(window, cx),
                    SigningEvent::RemoveSigner {
                        fingerprint,
                        principals,
                    } => {
                        this.ask_remove_signer(fingerprint.clone(), principals.clone(), window, cx)
                    }
                },
            ),
            cx.subscribe_in(
                &lfs_view,
                window,
                |this, _, event: &LfsEvent, window, cx| match event {
                    LfsEvent::Change { id, params, done } => {
                        this.places_loaded = None;
                        this.run_mutation(
                            id,
                            params.clone(),
                            done.clone(),
                            |_, _, _, _| false,
                            window,
                            cx,
                        )
                    }
                    LfsEvent::Transfer { id, doing, done } => {
                        this.run_operation(id, json!({}), doing, (*done).to_owned(), window, cx)
                    }
                    // The server is asked: where it can take its time and
                    // be stopped.
                    LfsEvent::Lock(path) => this.run_operation(
                        "lfs.lock",
                        json!({ "path": path }),
                        say!("workspace-lfs-locking"),
                        say!("workspace-lfs-locked", path),
                        window,
                        cx,
                    ),
                    LfsEvent::Unlock { path, force } => {
                        this.unlock_file(path.clone(), *force, window, cx)
                    }
                },
            ),
            cx.subscribe_in(
                &flags_view,
                window,
                |this, _, event: &FlagsEvent, window, cx| match event {
                    FlagsEvent::Close => {
                        this.sheet = None;
                        cx.notify();
                    }
                    FlagsEvent::Changed(said) => {
                        crate::notices::push(window, cx, Notice::success(said.clone()));
                        this.load_status(cx);
                    }
                    FlagsEvent::Refused(err) => {
                        crate::notices::push(window, cx, Notice::error(sentence(err)))
                    }
                },
            ),
            cx.subscribe_in(
                &merge_tool,
                window,
                |this, _, event: &MergeEvent, window, cx| match event {
                    // The file is no longer in conflict: show what it became.
                    MergeEvent::Resolved => {
                        if this.file_tab == FileTab::Merge {
                            this.file_tab = FileTab::Diff;
                            if let Some((path, None)) = this.file.clone() {
                                let source = DiffSource::Working {
                                    path,
                                    side: Side::Staged,
                                };
                                this.diff_view
                                    .update(cx, |view, cx| view.show(Some(source), cx));
                            }
                        }
                        this.after_mutation(cx);
                        cx.notify();
                    }
                    // Waited for where it can be stopped; the list of
                    // conflicts is read again when it ends.
                    MergeEvent::External(path) => this.external_merge(path.clone(), window, cx),
                    // Back to the graph; the tool keeps what was chosen
                    // for when the file is opened again.
                    MergeEvent::Close => {
                        this.set_show_file(false, cx);
                        this.diff_view.update(cx, |view, cx| view.show(None, cx));
                        this.remember_file(cx);
                        cx.notify();
                    }
                    MergeEvent::EditByHand => this.edit_file(window, cx),
                },
            ),
            cx.subscribe_in(
                &history_view,
                window,
                |this, _, event: &HistoryEvent, window, cx| match event {
                    // Beside the list, which stays: the file keeps the
                    // name it was opened under, whatever it was called
                    // in the commit that is looked at.
                    HistoryEvent::Open {
                        id,
                        path,
                        previous,
                        by_hand,
                    } => this.past_chosen((*id, path.clone(), previous.clone()), *by_hand, cx),
                    HistoryEvent::Blame { id, path } => this.past_blame(*id, path.clone(), cx),
                    HistoryEvent::Reveal(id) => this.reveal_from_past(*id, window, cx),
                    HistoryEvent::Mutated => this.after_mutation(cx),
                },
            ),
            cx.subscribe_in(
                &find_file,
                window,
                |this, _, event: &FindFileEvent, window, cx| match event {
                    // The file's history, in the pane under the graph.
                    FindFileEvent::Open(path) => {
                        window.close_dialog(cx);
                        // The file as the last commit has it, which is
                        // where it was found; its history is what is shown.
                        let head = this
                            .snapshot
                            .as_ref()
                            .and_then(|snapshot| snapshot.head.target);
                        let source = match head {
                            Some(id) => DiffSource::Revision {
                                id,
                                path: path.clone(),
                                old_path: None,
                            },
                            None => DiffSource::Working {
                                path: path.clone(),
                                side: Side::Unstaged,
                            },
                        };
                        this.open_file(path.clone(), head, source, cx);
                        this.set_file_tab(FileTab::History, cx);
                        // Closing it goes back to the graph it came from.
                        this.past_back = None;
                        // The keys go into the list, as a click on the
                        // history's tab puts them: after the finder, closed
                        // above, has given them back to where they were
                        // (see `find_file`).
                        cx.defer_in(window, |this, window, cx| {
                            this.history_view
                                .read(cx)
                                .focus_handle(cx)
                                .focus(window, cx);
                        });
                    }
                },
            ),
            cx.subscribe_in(
                &blame_view,
                window,
                |this, _, event: &BlameEvent, window, cx| match event {
                    BlameEvent::Reveal(id) => this.reveal_from_past(*id, window, cx),
                    BlameEvent::Open { id, path } => this.past_changes(*id, path.clone(), cx),
                },
            ),
            cx.subscribe_in(
                &commit_box,
                window,
                |this, _, event: &CommitEvent, window, cx| match event {
                    CommitEvent::Committed {
                        push,
                        skipped_hooks,
                    } => {
                        // A commit that was made without its checks says so.
                        let notice = if skipped_hooks.is_empty() {
                            Notice::success(say!("workspace-committed"))
                        } else {
                            Notice::warning(say!(
                                "workspace-committed-without",
                                hooks = crate::views::commit_box::name_list(skipped_hooks)
                            ))
                        };
                        crate::notices::push(window, cx, notice);
                        this.after_mutation(cx);
                        this.reload(cx);
                        if *push {
                            this.push(window, cx);
                        }
                    }
                    CommitEvent::Stashed => {
                        crate::notices::push(
                            window,
                            cx,
                            Notice::success(say!("workspace-stashed-staged")),
                        );
                        this.after_mutation(cx);
                        this.reload(cx);
                    }
                    CommitEvent::AskTrailer { key } => this.prompt_trailer(key.clone(), window, cx),
                    // The ways out of an operation that stopped part way.
                    CommitEvent::Continue => this.step_in_progress(false, window, cx),
                    CommitEvent::Skip => this.step_in_progress(true, window, cx),
                    CommitEvent::Abort => this.abort_in_progress(window, cx),
                },
            ),
        ];

        cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this
                    .update(cx, |workspace, cx| workspace.on_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        Self::watch_the_clock(window, cx);
        let file_to_restore = cx.global::<Prefs>().repo_state(&session.path).open_file;
        let mut workspace = Self {
            session,
            snapshot: None,
            graph: None,
            graph_loads: 0,
            graph_ahead: None,
            status: Vec::new(),
            refs_view,
            graph_view,
            details_view,
            changes_view,
            commit_box,
            diff_view,
            history_view,
            find_file,
            blame_view,
            edit_view,
            merge_tool,
            rebase_editor,
            activity_view,
            journal_view,
            rerere_view,
            signing_view,
            lfs_view,
            upkeep,
            flags_view,
            lfs_status: LfsStatus::default(),
            sheet: None,
            pulls,
            pull_view: None,
            pull_subscription: None,
            new_pull: None,
            new_pull_subscription: None,
            issues,
            issue_view: None,
            issue_subscription: None,
            docked: None,
            planning: false,
            in_progress: None,
            finishing: None,
            skip_push_hook: false,
            skip_mailbox_hooks: false,
            file: None,
            file_tab: FileTab::Diff,
            // The two fields of the plain prompt are lent to the forms.
            forms: form::Forms::new(vec![prompt.clone(), prompt_optional.clone()]),
            prompt,
            prompt_optional,
            prompt_action: None,
            prompt_required: Vec::new(),
            flow_state: FlowState::default(),
            settings_shown: false,
            places_loaded: None,
            submodule_reads: crate::views::submodules::Paced::new(SUBMODULE_GAP),
            operation: None,
            operations_waiting: std::collections::VecDeque::new(),
            busy: running::Busy::default(),
            removing: std::collections::BTreeSet::new(),
            right: RightPane::Commit,
            status_cut: false,
            large_hidden: false,
            platform: PlatformNotes::default(),
            selected_worktree: None,
            worktrees_asked: false,
            worktrees_afresh: false,
            worktree_wants: Default::default(),
            reloads: 0,
            show_diff: false,
            past_entry: None,
            past_back: None,
            past_file_view: false,
            close_after_save: false,
            ahead_behind: None,
            file_to_restore,
            status_loaded: false,
            undo_label: None,
            redo_label: None,
            undo_id: None,
            undo_remote: false,
            redo_remote: false,
            auto_fetch: sync::AutoFetch::new(),
            terminal: terminals::TerminalLink::default(),
            conflict_marks: marks::ConflictMarks::default(),
            loading: true,
            error: None,
            _subscriptions: subscriptions,
        };
        workspace.follow_form_typing(window, cx);
        // What was known about the other worktrees the last time: their
        // rows are in the first graph, and their counts are shown at once.
        let seen = cx
            .global::<Prefs>()
            .repo_state(&workspace.session.path)
            .worktrees_seen;
        if !seen.is_empty() {
            workspace.show_worktrees_seen(&seen, cx);
        }
        match startup {
            Some(replies) => {
                // Replies that are already in go straight into the views, so
                // that the first frame shows them; the rest arrive as usual.
                let take = |workspace: &mut Self,
                            mut rx: futures::channel::oneshot::Receiver<Reply>,
                            name: &str,
                            on_reply: fn(&mut Self, Reply, &mut Context<Self>),
                            cx: &mut Context<Self>| {
                    match rx.try_recv() {
                        Ok(Some(reply)) => on_reply(workspace, reply, cx),
                        _ => SessionHandle::deliver(rx, name.to_owned(), cx, on_reply),
                    }
                };
                take(
                    &mut workspace,
                    replies.status,
                    "status",
                    Self::on_status,
                    cx,
                );
                take(&mut workspace, replies.refs, "refs", Self::on_refs, cx);
                // The details of the rows the graph shows first, read after
                // it: when they are in, so is the graph, and they go in
                // first, so that the graph does not ask for them again.
                take(
                    &mut workspace,
                    replies.details,
                    "first_details",
                    Self::on_first_details,
                    cx,
                );
                workspace.graph_loads = 1;
                take(&mut workspace, replies.graph, "graph", Self::on_graph, cx);
                workspace.peek_undo(cx);
                // What the status goes with when it is asked for here: the
                // operation in progress, or the conflicts there are without
                // one.
                workspace.load_in_progress(cx);
            }
            None => {
                // Status first: the session lays the graph out with a row for
                // uncommitted work when the last status it saw was dirty.
                workspace.load_status(cx);
                workspace.reload(cx);
            }
        }
        workspace.look_at_platform(cx);
        workspace
    }

    /// Show a file's diff in the pane under the graph and remember the file
    /// for the pane's history and blame tabs.
    fn open_file(
        &mut self,
        path: String,
        rev: Option<Oid>,
        source: DiffSource,
        cx: &mut Context<Self>,
    ) {
        self.file = Some((path, rev));
        self.file_tab = FileTab::Diff;
        self.diff_view
            .update(cx, |view, cx| view.show_as(Some(source), false, cx));
        self.set_show_file(true, cx);
        self.remember_file(cx);
        cx.notify();
    }

    /// Write down which file is open in the middle, or that none is, so
    /// that the tab comes back with it.
    fn remember_file(&mut self, cx: &mut Context<Self>) {
        let open = self
            .file
            .clone()
            .filter(|_| self.show_diff)
            .map(|(path, rev)| crate::prefs::OpenFile {
                path,
                rev: rev.map(|id| id.to_hex()),
            });
        let path = self.session.path.clone();
        if cx.global::<Prefs>().repo_state(&path).open_file != open {
            cx.global_mut::<Prefs>()
                .update_repo_state(&path, |state| state.open_file = open);
        }
    }

    /// Open the file the tab had open the last time, once: a file of the
    /// worktree when it is still changed, a file of a commit when the
    /// commit is still in the graph.
    fn restore_file(&mut self, cx: &mut Context<Self>) {
        let Some(open) = self.file_to_restore.clone() else {
            return;
        };
        let source = match &open.rev {
            None => {
                // The status says whether it is still changed, and where.
                if !self.status_loaded {
                    return;
                }
                self.status
                    .iter()
                    .find(|entry| entry.path == open.path)
                    .map(|entry| DiffSource::Working {
                        path: open.path.clone(),
                        side: if entry.unstaged.is_some() {
                            Side::Unstaged
                        } else {
                            Side::Staged
                        },
                    })
            }
            Some(rev) => {
                let Some(graph) = &self.graph else {
                    return;
                };
                rev.parse::<Oid>()
                    .ok()
                    .filter(|id| graph.index.contains_key(id))
                    .map(|id| DiffSource::Revision {
                        id,
                        path: open.path.clone(),
                        old_path: None,
                    })
            }
        };
        self.file_to_restore = None;
        match source {
            Some(source) => {
                let rev = open.rev.as_deref().and_then(|rev| rev.parse::<Oid>().ok());
                self.open_file(open.path, rev, source, cx);
            }
            // It is no longer there to be shown: forget it.
            None => self.remember_file(cx),
        }
    }

    /// Put the file that is shown into the index, or take it out, and go
    /// on showing it from the other side.
    fn move_file(&mut self, stage: bool, cx: &mut Context<Self>) {
        let Some((path, None)) = self.file.clone() else {
            return;
        };
        let request = Request::Command {
            id: if stage {
                "stage.paths"
            } else {
                "unstage.paths"
            }
            .to_owned(),
            params: json!({ "paths": [path] }),
        };
        self.session.call(request, cx, move |this, reply, cx| {
            if let Reply::Error(err) = reply {
                this.fail(err, cx);
                return;
            }
            let source = DiffSource::Working {
                path: path.clone(),
                side: if stage { Side::Staged } else { Side::Unstaged },
            };
            this.diff_view
                .update(cx, |view, cx| view.show(Some(source), cx));
            this.after_mutation(cx);
        });
    }

    /// Whether a row of the graph is selected, so that the right panel has
    /// something to show.
    fn something_selected(&self, cx: &App) -> bool {
        self.right != RightPane::Commit || self.details_view.read(cx).shows_something()
    }

    /// Open a file that was clicked in a list of the right panel; the one
    /// that is shown already is closed again by the click (the list takes
    /// its mark away). Whether it was closed.
    fn toggle_file(
        &mut self,
        path: String,
        rev: Option<Oid>,
        source: DiffSource,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let shown = self.show_diff
            && self
                .diff_view
                .read(cx)
                .source()
                .is_some_and(|shown| same_file(shown, &source));
        if shown {
            self.close_file(window, cx);
        } else {
            self.open_file(path, rev, source, cx);
        }
        shown
    }

    /// Show or leave the file in the middle. While one is shown, the
    /// references panel makes room and is a strip.
    fn set_show_file(&mut self, show: bool, cx: &mut Context<Self>) {
        self.show_diff = show;
        self.refs_view
            .update(cx, |view, cx| view.force_fold(show, cx));
    }

    /// Open a conflicted file in the merge tool: our side, their side and
    /// the result. The file as git left it, markers and all, stays one tab
    /// away in the editor.
    fn open_conflict(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        // Changed on one side and deleted on the other: asked in the bar,
        // staying on what is shown.
        if self.ask_about_deleted(&path, window, cx) {
            return;
        }
        let source = DiffSource::Working {
            path: path.clone(),
            side: Side::Unstaged,
        };
        self.open_file(path.clone(), None, source, cx);
        // A file that holds a resolution remembered from last time is shown
        // as it is, where it can be changed: there is no conflict left in
        // it to look at first. The merge tool is a tab away, and starts
        // from that resolution too.
        let reused = self.in_progress.as_ref().is_some_and(|operation| {
            operation
                .conflicts
                .iter()
                .any(|conflict| conflict.path == path && conflict.reused)
        });
        if reused {
            self.edit_file(window, cx);
            return;
        }
        self.show_merge_tool(path, window, cx);
    }

    /// Resolve a file in the external merge tool, waited for where it can
    /// be stopped; the list of conflicts is read again when it ends.
    pub(super) fn external_merge(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let command = cx.global::<Prefs>().preferences.external_merge.clone();
        self.run_operation(
            "tool.merge",
            json!({ "path": path, "command": command }),
            say!("workspace-merge-tool-waiting"),
            say!("workspace-merge-tool-ended").to_owned(),
            window,
            cx,
        );
    }

    /// The merge tool on the conflicted file the pane is about, asked for
    /// by name: also for a file resolved as last time, whose result then
    /// starts from what git put into it.
    fn show_merge_tool(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        let (ours, theirs) = self
            .in_progress
            .as_ref()
            .map(|operation| (operation.ours.clone(), operation.theirs.clone()))
            .unwrap_or_default();
        self.merge_tool.update(cx, |view, cx| {
            if view.path() != Some(path.as_str()) {
                view.open(path, ours, theirs, window, cx);
            }
        });
        self.set_file_tab(FileTab::Merge, cx);
    }

    /// Open the worktree file the pane is about in the editor, unless it is
    /// open there already (with or without unsaved changes).
    fn edit_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // A file of a commit is edited as it is in the worktree.
        if let Some((path, _)) = self.file.clone() {
            self.edit_view.update(cx, |view, cx| {
                if view.path() != Some(path.as_str()) {
                    view.open(path, window, cx);
                }
            });
            self.set_file_tab(FileTab::Edit, cx);
        }
    }

    /// Switch the file pane, loading history or blame for the current file
    /// unless that is what the tab already holds.
    fn set_file_tab(&mut self, tab: FileTab, cx: &mut Context<Self>) {
        // History and blame are one view with the commits in a list at
        // its left: both want the list.
        let past = matches!(tab, FileTab::History | FileTab::Blame);
        let entering = past && !self.shows_past();
        if past {
            self.enter_past(cx);
        }
        self.file_tab = tab;
        self.set_show_file(true, cx);
        if let Some((path, rev)) = self.file.clone() {
            if past {
                let list_path = path.clone();
                self.history_view.update(cx, |view, cx| {
                    if view.path() != Some(list_path.as_str()) {
                        view.show(list_path, rev, cx);
                    } else if entering {
                        // The list is there from the last time: it
                        // begins again where a new one would.
                        view.begin_again(rev, cx);
                    }
                });
            }
            match tab {
                FileTab::Diff | FileTab::History => {}
                FileTab::Blame => self.blame_view.update(cx, |view, cx| {
                    if view.target() != Some((path.as_str(), rev)) {
                        view.show(path, rev, cx);
                    }
                }),
                // Opened where there is a window to do it in (the tab's
                // button); here the pane only switches.
                FileTab::Edit | FileTab::Merge => {}
            }
        }
        cx.notify();
    }

    /// Run a command that changes the repository, say so when it worked,
    /// and refresh. `on_error` gets the first look at a failure and returns
    /// true when it took care of it.
    fn run_mutation(
        &mut self,
        id: &str,
        params: serde_json::Value,
        done: String,
        on_error: impl FnOnce(&mut Self, &str, &mut Window, &mut Context<Self>) -> bool + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.refused_here(id, window, cx) {
            return;
        }
        // Always the user's: a patch that is written out reads what a
        // partial clone may have left on its remote.
        let reply = self.session.ask(Request::Command {
            id: id.to_owned(),
            params,
        });
        let before = self.undo_id;
        let quiet = quiet_when_done(id);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else {
                return;
            };
            let made = Self::journalled(&this, cx, before).await;
            let _ =
                this.update_in(cx, |this, window, cx| {
                    match reply {
                        Reply::Error(err) => {
                            if !on_error(this, &err, window, cx) {
                                let notice = this.failed_notice(&err, cx);
                                crate::notices::push(window, cx, notice);
                            }
                        }
                        Reply::Command(value)
                            if value.get("stash_kept").and_then(|kept| kept.as_bool())
                                == Some(true) =>
                        {
                            // The checkout happened, the stashed changes stayed.
                            crate::notices::push(
                                window,
                                cx,
                                Notice::warning(say!(
                                    "workspace-done-stash-kept",
                                    done = done.as_str()
                                )),
                            );
                        }
                        // Done, and a hook that ran afterwards ended badly
                        // (`post-checkout`): said, with the way to its words.
                        Reply::Command(value)
                            if value.get("hook").is_some_and(|hook| hook.is_string()) =>
                        {
                            let said = value
                                .get("hook")
                                .and_then(|hook| hook.as_str())
                                .unwrap_or_default();
                            let workspace = cx.entity().downgrade();
                            let notice = Notice::warning(format!("{done}. {}", sentence(said)))
                                .action(say!("workspace-show-log"), move |_, cx| {
                                    let _ = workspace.update(cx, |this, cx| this.show_activity(cx));
                                });
                            crate::notices::push(window, cx, notice);
                        }
                        _ if quiet => {}
                        _ => {
                            let notice = this.done_notice(done, made, cx);
                            crate::notices::push(window, cx, notice)
                        }
                    }
                    this.after_mutation(cx);
                    this.reload(cx);
                });
        })
        .detach();
    }

    /// In a repository that is opened read-only, a command that would
    /// change it is not sent: the engine would refuse it. Says so and
    /// returns true then. The views do not offer such commands; this is
    /// for what reaches here all the same (two clicks on a label, a key).
    fn refused_here(&self, id: &str, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let refused = self.read_only() && crate::read_only::refuses(id);
        if refused {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!(
                    "workspace-read-only-refused",
                    in_short = crate::read_only::IN_SHORT.text()
                )),
            );
        }
        refused
    }

    /// The journal entry a command that just ended made, if it made one:
    /// what undo would take now, when that is not what it would have taken
    /// before the command ran.
    async fn journalled(
        this: &WeakEntity<Self>,
        cx: &mut AsyncWindowContext,
        before: Option<u64>,
    ) -> Option<u64> {
        let peek = this
            .update(cx, |this, _| {
                this.session.send(Request::Command {
                    id: "undo.peek".to_owned(),
                    params: json!({}),
                })
            })
            .ok()?;
        let Ok(Reply::Command(value)) = peek.await else {
            return None;
        };
        made_entry(before, value.get("undo_id").and_then(|id| id.as_u64()))
    }

    /// A notice that something worked, with a way to undo it when it went
    /// into the journal.
    fn done_notice(&self, text: String, made: Option<u64>, cx: &mut Context<Self>) -> Notice {
        self.with_undo(Notice::success(text), made, cx)
    }

    /// A notice about something that was done, with a way to undo it when
    /// it made an entry in the journal.
    fn with_undo(&self, notice: Notice, made: Option<u64>, cx: &mut Context<Self>) -> Notice {
        match made {
            Some(id) => {
                let workspace = cx.entity().downgrade();
                notice.action(say!("workspace-undo"), move |window, cx| {
                    let _ = workspace.update(cx, |this, cx| this.undo_entry(id, window, cx));
                })
            }
            None => notice,
        }
    }

    /// A notice that something failed, with a way to what git said.
    fn failed_notice(&self, err: &str, cx: &mut Context<Self>) -> Notice {
        let workspace = cx.entity().downgrade();
        Notice::error(sentence(err)).action(say!("workspace-show-log"), move |_, cx| {
            let _ = workspace.update(cx, |this, cx| this.show_activity(cx));
        })
    }

    /// Undo the journal entry a notice was about, when it is still the
    /// last thing that was done; otherwise show the undo history, where
    /// going back further is a choice of its own.
    pub fn undo_entry(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if self.undo_id == Some(id) {
            self.undo(window, cx);
            return;
        }
        crate::notices::push(window, cx, Notice::info(say!("workspace-undo-not-last")));
        if self.sheet != Some(Sheet::Journal) {
            self.toggle_sheet(Sheet::Journal, cx);
        }
    }

    /// Open the repository's folder in another program: the file manager,
    /// the editor or a terminal. It says something only when it failed.
    pub fn open_outside(&mut self, what: Outside, window: &mut Window, cx: &mut Context<Self>) {
        let prefs = &cx.global::<Prefs>().preferences;
        let (id, params) = match what {
            Outside::FileManager => ("repo.open_folder", json!({ "command": prefs.file_manager })),
            Outside::Editor => ("repo.open_editor", json!({ "command": prefs.editor })),
            Outside::Terminal => ("repo.open_terminal", json!({ "command": prefs.terminal })),
        };
        let reply = self.session.send(Request::Command {
            id: id.to_owned(),
            params,
        });
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Reply::Error(err)) = reply.await {
                let _ = cx.update(|window, cx| {
                    let said = if what == Outside::Editor && err.contains("no editor is named") {
                        say!("workspace-no-editor").to_owned()
                    } else {
                        sentence(&err)
                    };
                    crate::notices::push(window, cx, Notice::error(said));
                });
            }
        })
        .detach();
    }

    /// Show the activity log, if it is not what is shown.
    pub fn show_activity(&mut self, cx: &mut Context<Self>) {
        if self.sheet != Some(Sheet::Activity) {
            self.toggle_sheet(Sheet::Activity, cx);
        }
    }

    /// Run a command that can take a while as an operation: it shows in the
    /// status bar with what git reports, can be stopped, and refreshes
    /// everything when it ends.
    fn run_operation(
        &mut self,
        id: &str,
        params: serde_json::Value,
        label: &str,
        done: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_operation_with(id, params, label, done, |_, _, _, _| false, window, cx);
    }

    /// [`Self::run_operation`], with `on_error` getting the first look at a
    /// failure; it returns true when it took care of it.
    #[allow(clippy::too_many_arguments)]
    fn run_operation_with(
        &mut self,
        id: &str,
        params: serde_json::Value,
        label: &str,
        done: String,
        on_error: impl FnOnce(&mut Self, &str, &mut Window, &mut Context<Self>) -> bool + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.refused_here(id, window, cx) {
            return;
        }
        // What goes without the hooks that could refuse it says so for
        // as long as it runs, and when it is done.
        let params = self.with_switches(id, params, cx);
        let label = if hooks::asked(id, &params) {
            hooks::running(label)
        } else {
            label.to_owned()
        };
        let reply = self.session.send(Request::Operation {
            id: id.to_owned(),
            params,
        });
        self.operation_began(&label, Some(id));
        let command = id.to_owned();
        let before = self.undo_id;
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let made = Self::journalled(&this, cx, before).await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.operation_ended();
                let ended = match &reply {
                    Ok(Reply::Command(value)) if stopped(value).is_some() => Ended::Stopped,
                    Ok(Reply::Error(_)) | Err(_) => Ended::Failed,
                    Ok(_) => Ended::Done,
                };
                match reply {
                    Ok(Reply::Error(err)) if err.contains("operation: cancelled") => {
                        crate::notices::push(
                            window,
                            cx,
                            Notice::info(say!(
                                "workspace-operation-stopped",
                                label = label.as_str()
                            )),
                        )
                    }
                    // The question git asked was declined; that is not news.
                    Ok(Reply::Error(err)) if err.contains("unable to read askpass response") => {
                        crate::notices::push(
                            window,
                            cx,
                            Notice::info(say!(
                                "workspace-operation-not-answered",
                                label = label.as_str()
                            )),
                        )
                    }
                    Ok(Reply::Error(err)) => {
                        if !on_error(this, &err, window, cx) {
                            let notice = this.failed_notice(&err, cx);
                            crate::notices::push(window, cx, notice)
                        }
                    }
                    // It began, or went on, and stopped on something to
                    // resolve: that is in the banner now.
                    Ok(Reply::Command(value)) if stopped(&value).is_some() => {
                        let files = stopped(&value).unwrap_or(0);
                        let said = value.get("stopped").and_then(|said| said.as_str());
                        crate::notices::push(
                            window,
                            cx,
                            Notice::warning(match (files, said) {
                                // Stopped where it was asked to: at a
                                // commit to amend, before a merge commit.
                                (0, Some(said))
                                    if said.starts_with("stopped at")
                                        || said.starts_with("stopped before") =>
                                {
                                    format!("{label}: {said}")
                                }
                                (0, _) => say!(
                                    "workspace-operation-stopped-half-way",
                                    label = label.as_str()
                                ),
                                (count, _) if this.finish_follows(&command) => say!(
                                    "workspace-finish-stopped-on-conflicts",
                                    label = label.as_str(),
                                    count
                                ),
                                (count, _) => say!(
                                    "workspace-operation-stopped-on-conflicts",
                                    label = label.as_str(),
                                    count
                                ),
                            }),
                        )
                    }
                    // Done, and the changes that were set aside for it
                    // did not fit back in (a rebase with autostash).
                    Ok(Reply::Command(value))
                        if value.get("stash_kept").and_then(|kept| kept.as_bool())
                            == Some(true) =>
                    {
                        crate::notices::push(
                            window,
                            cx,
                            Notice::warning(say!(
                                "workspace-done-autostash-kept",
                                done = done.as_str()
                            )),
                        )
                    }
                    // Done without the hooks that could have refused it:
                    // which ones, as a warning.
                    Ok(Reply::Command(value)) if hooks::skipped_said(&done, &value).is_some() => {
                        let said = hooks::skipped_said(&done, &value).unwrap_or(done);
                        let notice = this.with_undo(Notice::warning(said), made, cx);
                        crate::notices::push(window, cx, notice)
                    }
                    Ok(_) if quiet_when_done(&command) => {}
                    Ok(_) => {
                        let notice = this.done_notice(done, made, cx);
                        crate::notices::push(window, cx, notice)
                    }
                    Err(_) => {}
                }
                this.finish_goes_on(&command, ended, window, cx);
                this.after_mutation(cx);
                this.reload(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// What a merge is part of when it is the one a gitflow finish stopped
    /// in: the sentence about it begins with this.
    fn finishing_said(&self) -> Option<String> {
        self.finishing
            .as_ref()
            .map(|(kind, name, _)| say!("workspace-flow-finishing-branch", kind, name))
    }

    /// Whether "stopped on a conflict" is a gitflow finish that stopped,
    /// whose rest follows by itself: the sentence then says so.
    fn finish_follows(&self, command: &str) -> bool {
        command == "flow.finish" && self.finishing.is_some()
    }

    /// A gitflow finish is several steps, and a merge among them that
    /// conflicts stops it. When that merge is concluded with Continue, the
    /// finish is run again and does what is left (the engine's finish takes
    /// up wherever it stopped). It can stop again, in the next merge of a
    /// release or a hotfix, and goes on the same way.
    fn finish_goes_on(
        &mut self,
        command: &str,
        ended: Ended,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match finish_after(command, ended) {
            Waiting::Keep => {}
            Waiting::Forget => self.finishing = None,
            Waiting::GoOn => {
                if let Some((kind, name, keep)) = self.finishing.clone() {
                    self.run_operation(
                        "flow.finish",
                        json!({ "kind": kind, "name": name, "keep": keep }),
                        say!("workspace-flow-finishing"),
                        say!("workspace-flow-finished", kind, name),
                        window,
                        cx,
                    );
                }
            }
        }
    }

    /// An operation was handed to the operations thread: it is the one
    /// running, or one more waiting behind it.
    fn operation_began(&mut self, label: &str, command: Option<&str>) {
        let operation = Operation {
            label: label.to_owned(),
            command: command.map(str::to_owned),
            stage: None,
        };
        match &self.operation {
            Some(_) => self.operations_waiting.push_back(operation),
            None => self.operation = Some(operation),
        }
    }

    /// The engine's command that runs now, when the user started it from
    /// a control: the toolbar is greyed meanwhile.
    pub fn running_command(&self) -> Option<String> {
        self.operation.as_ref()?.command.clone()
    }

    /// Whether there is anything to stash: a change of any kind.
    pub fn can_stash(&self) -> bool {
        !self.status.is_empty()
    }

    /// Whether there is a stash to pop.
    pub fn can_pop(&self) -> bool {
        self.snapshot
            .as_ref()
            .is_some_and(|snapshot| !snapshot.stashes.is_empty())
    }

    /// An operation ended: the next one in the queue is running now, if
    /// there is one. The thread takes them in the order they were handed
    /// to it.
    fn operation_ended(&mut self) {
        self.operation = self.operations_waiting.pop_front();
    }

    /// Run any engine command with parameters that were put together
    /// elsewhere (the palette's prompt), where it can be stopped, and show
    /// what it answers.
    pub fn run_generic(
        &mut self,
        id: &str,
        label: String,
        params: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply = self.session.send(Request::Operation {
            id: id.to_owned(),
            params,
        });
        self.operation_began(&label, Some(id));
        let id = id.to_owned();
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.operation_ended();
                match reply {
                    Ok(Reply::Error(err)) if err.contains("operation: cancelled") => {
                        crate::notices::push(
                            window,
                            cx,
                            Notice::info(say!(
                                "workspace-operation-stopped",
                                label = label.as_str()
                            )),
                        )
                    }
                    Ok(Reply::Error(err)) => {
                        crate::notices::push(window, cx, Notice::error(sentence(&err)))
                    }
                    Ok(Reply::Command(value)) => {
                        crate::shell::show_command_output(&label, &value, window, cx);
                        // What lies outside the repository may have
                        // changed too.
                        crate::shell::command_ran(&id, cx);
                    }
                    _ => {}
                }
                this.places_loaded = None;
                this.after_mutation(cx);
                this.reload(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Commit what is staged, or everything, with the message in the
    /// commit box. Without a message the box is shown and waits for one.
    pub fn commit_now(&mut self, stage_all: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.right = RightPane::Wip;
        self.graph_view.update(cx, |view, cx| view.select_wip(cx));
        self.commit_box
            .update(cx, |view, cx| view.commit_now(stage_all, window, cx));
        cx.notify();
    }

    /// What the menu of a branch's label in the graph asked for: what the
    /// references panel does for the same entries.
    fn branch_from_label(
        &mut self,
        request: &BranchRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            BranchRequest::Pull(name) => self.pull_branch(name.clone(), window, cx),
            BranchRequest::SetUpstream(name) => self.prompt_upstream(name.clone(), window, cx),
            BranchRequest::Rename(name) => self.prompt_rename_branch(name.clone(), window, cx),
            BranchRequest::Delete(name) => self.confirm_delete_branch(name.clone(), window, cx),
            BranchRequest::FastForward(name) => self.run_mutation(
                "branch.fast_forward",
                json!({ "name": name }),
                say!("workspace-fast-forwarded", name),
                |_, _, _, _| false,
                window,
                cx,
            ),
            BranchRequest::Push(name) => self.push_branches(std::slice::from_ref(name), window, cx),
            BranchRequest::DeleteOnRemote(name) => {
                self.confirm_delete_on_remote(name.clone(), window, cx)
            }
        }
    }

    /// What the menu of the row of uncommitted work asked for, about all
    /// of it at once.
    fn wip_action(&mut self, action: WipAction, window: &mut Window, cx: &mut Context<Self>) {
        let (staged, unstaged) = wip_paths(&self.status);
        match action {
            WipAction::StageAll if unstaged.is_empty() => {
                crate::notices::push(window, cx, Notice::info(say!("workspace-nothing-to-stage")));
            }
            WipAction::StageAll => self.run_mutation(
                "stage.paths",
                json!({ "paths": unstaged }),
                say!("workspace-staged-files", count = unstaged.len()),
                |_, _, _, _| false,
                window,
                cx,
            ),
            WipAction::UnstageAll if staged.is_empty() => {
                crate::notices::push(window, cx, Notice::info(say!("workspace-nothing-staged")));
            }
            WipAction::UnstageAll => self.run_mutation(
                "unstage.paths",
                json!({ "paths": staged }),
                say!("workspace-unstaged-files", count = staged.len()),
                |_, _, _, _| false,
                window,
                cx,
            ),
            WipAction::DiscardAll if unstaged.is_empty() => {
                crate::notices::push(
                    window,
                    cx,
                    Notice::info(say!("workspace-nothing-to-discard")),
                );
            }
            WipAction::DiscardAll => self.ask_discard(unstaged, window, cx),
            WipAction::Commit => self.commit_now(false, window, cx),
            WipAction::CommitAll => self.commit_now(true, window, cx),
        }
    }

    /// Ask in the bar, then throw away what is not staged of these files.
    /// Undo brings it back; the question is asked all the same.
    fn ask_discard(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let count = paths.len();
        let workspace = cx.entity().downgrade();
        crate::ask::show(
            crate::ask::Ask::new(say!("workspace-discard-question", count)).danger(
                say!("workspace-discard"),
                move |_, window, cx| {
                    let _ = workspace.update(cx, |this, cx| {
                        this.run_mutation(
                            "discard.paths",
                            json!({ "paths": paths.clone() }),
                            say!("workspace-discarded", count),
                            |_, _, _, _| false,
                            window,
                            cx,
                        )
                    });
                },
            ),
            window,
            cx,
        );
    }

    /// Ask for a date or a reference and show the commit it names.
    pub fn prompt_jump(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt(
            say!("workspace-jump-title").to_owned(),
            say!("workspace-jump"),
            vec![PromptField::required(
                say!("workspace-jump-placeholder"),
                "",
            )],
            |this, texts, window, cx| {
                let text = texts.into_iter().next().unwrap_or_default();
                this.jump_to(text, window, cx);
            },
            window,
            cx,
        );
    }

    /// Show the commit `text` names: as a revision when it is one (a
    /// branch called 2024 is a branch), else as a date, where it is the
    /// newest commit at or before it.
    pub fn jump_to(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        let locate = |params: serde_json::Value| {
            self.session.send(Request::Command {
                id: "search.locate".to_owned(),
                params,
            })
        };
        let by_rev = locate(json!({ "rev": text }));
        let by_date = locate(json!({ "date": text }));
        cx.spawn_in(window, async move |this, cx| {
            let found = |reply| match reply {
                Ok(Reply::Command(value)) => value
                    .get("id")
                    .and_then(|id| id.as_str())
                    .and_then(|id| id.parse::<Oid>().ok()),
                _ => None,
            };
            let id = match found(by_rev.await) {
                Some(id) => Some(id),
                None => found(by_date.await),
            };
            let _ = this.update_in(cx, |this, window, cx| match id {
                Some(id) => this.reveal(id, window, cx),
                None => crate::notices::push(
                    window,
                    cx,
                    Notice::warning(say!("workspace-jump-nothing", text = text.as_str())),
                ),
            });
        })
        .detach();
    }

    /// Whether the repository has been read: until then its tab spins.
    pub fn is_loaded(&self) -> bool {
        self.snapshot.is_some()
    }

    /// The activity log, while it is open: the window shows it as a
    /// dialog over everything.
    pub fn activity_dialog(&self) -> Option<Entity<ActivityView>> {
        (self.sheet == Some(Sheet::Activity)).then(|| self.activity_view.clone())
    }

    /// Put the activity log away.
    pub fn close_activity(&mut self, cx: &mut Context<Self>) {
        if self.sheet == Some(Sheet::Activity) {
            self.sheet = None;
            cx.notify();
        }
    }

    /// Show the activity log, or put it away when it is what is shown.
    pub fn toggle_activity(&mut self, cx: &mut Context<Self>) {
        self.toggle_sheet(Sheet::Activity, cx);
    }

    /// Escape puts away a list shown in the middle (the undo history,
    /// rerere's records, the index flags) and gives the keys back to the
    /// graph. Answers whether there was one.
    pub fn escape_closes_list(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !matches!(
            self.sheet,
            Some(Sheet::Journal | Sheet::Rerere | Sheet::Flags)
        ) {
            return false;
        }
        self.sheet = None;
        cx.notify();
        // Once the graph is drawn again.
        cx.defer_in(window, |this, window, cx| {
            this.focus_graph(window, cx);
        });
        true
    }

    /// Escape that no action took in a side panel: the keys go
    /// back to the graph; from the right panel, a file in the middle is
    /// closed first. What in the panel has a use of its own for the key (a
    /// menu, a field's popup) took it before it comes here.
    fn escape_in_panel(
        &mut self,
        event: &KeyDownEvent,
        right: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.keystroke.key != "escape" || event.keystroke.modifiers.modified() {
            return;
        }
        let file_pane = self.sheet.is_none()
            && !self.planning
            && self.show_diff
            && !self.shows_past()
            && !matches!(self.file_tab, FileTab::Merge);
        if right && file_pane {
            cx.stop_propagation();
            self.close_file(window, cx);
            cx.defer_in(window, |this, window, cx| {
                this.focus_graph(window, cx);
            });
        } else if self.focus_graph(window, cx) {
            cx.stop_propagation();
        }
    }

    /// Show the undo history in the centre, or put it away.
    pub fn toggle_journal(&mut self, cx: &mut Context<Self>) {
        self.toggle_sheet(Sheet::Journal, cx);
    }

    /// Show what rerere has recorded in the centre, or put it away.
    pub fn toggle_rerere_records(&mut self, cx: &mut Context<Self>) {
        self.toggle_sheet(Sheet::Rerere, cx);
    }

    /// The sections of the preferences that are this repository's: how it
    /// signs, its large files, its session for the rest.
    pub fn signing_view(&self) -> Entity<SigningView> {
        self.signing_view.clone()
    }

    /// See `signing_view`.
    pub fn lfs_view(&self) -> Entity<LfsView> {
        self.lfs_view.clone()
    }

    /// See `signing_view`.
    pub fn session(&self) -> SessionHandle {
        self.session.clone()
    }

    /// Gitflow's set-up as it was last read.
    pub fn flow_state(&self) -> &FlowState {
        &self.flow_state
    }

    /// Read again what a section of the preferences about this repository
    /// shows: how it signs and its keys, where LFS stands.
    pub fn reload_settings(&mut self, cx: &mut Context<Self>) {
        self.signing_view.update(cx, |view, cx| view.reload(cx));
        self.lfs_view.update(cx, |view, cx| view.reload(cx));
    }

    /// Write gitflow's branches and prefixes, as its section has them.
    pub fn save_flow_settings(
        &mut self,
        settings: crate::views::flow::FlowSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let set_up = self.flow_state.initialized;
        self.places_loaded = None;
        self.run_mutation(
            "flow.init",
            settings.params(),
            if set_up {
                say!("workspace-flow-saved").to_owned()
            } else {
                say!(
                    "workspace-flow-set-up",
                    main = &settings.main,
                    develop = &settings.develop
                )
            },
            |_, _, _, _| false,
            window,
            cx,
        );
    }

    /// Whether a pattern is tracked by LFS here: the toolbar then shows
    /// its button.
    pub fn lfs_active(&self) -> bool {
        self.lfs_status.active
    }

    /// Fetch, pull or push the content of large files, as the toolbar's
    /// LFS group and the LFS view both do.
    pub fn lfs_transfer(&mut self, what: LfsTransfer, window: &mut Window, cx: &mut Context<Self>) {
        let (id, doing, done) = what.command();
        self.run_operation(id, json!({}), doing, done.to_owned(), window, cx);
    }

    /// Give an LFS lock up. Somebody else's is taken from them, which is
    /// asked about first.
    fn unlock_file(
        &mut self,
        path: String,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let run =
            move |this: &mut Self, path: String, window: &mut Window, cx: &mut Context<Self>| {
                this.run_operation(
                    "lfs.unlock",
                    json!({ "path": path, "force": force }),
                    say!("workspace-lfs-unlocking"),
                    say!("workspace-lfs-unlocked", path = path.as_str()),
                    window,
                    cx,
                )
            };
        if !force {
            run(self, path, window, cx);
            return;
        }
        let workspace = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (workspace, path) = (workspace.clone(), path.clone());
            alert
                .title(say!(
                    "workspace-lfs-force-unlock-title",
                    path = path.as_str()
                ))
                .description(say!("workspace-lfs-force-unlock-description"))
                .ok_text(say!("workspace-lfs-force-unlock"))
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let path = path.clone();
                    let _ = workspace.update(cx, |this, cx| run(this, path, window, cx));
                    true
                })
        });
    }

    /// Show the files git was told not to look at, or put that away. The
    /// view takes the keys while it is shown: its rows are walked with the
    /// arrows.
    pub fn toggle_flags(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.toggle_sheet(Sheet::Flags, cx);
        if self.sheet == Some(Sheet::Flags) {
            self.flags_view.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    /// Ask for a folder and write these commits there as patches, one
    /// file each.
    pub fn export_patches(
        &mut self,
        commits: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let folder = self
            .main_folder()
            .parent()
            .map(|parent| parent.join("patches").display().to_string())
            .unwrap_or_else(|| "patches".to_owned());
        let count = commits.len();
        self.prompt(
            say!("workspace-patches-save-title", count),
            say!("workspace-save"),
            vec![PromptField::required(
                say!("workspace-patches-folder"),
                &folder,
            )],
            move |this, texts, window, cx| {
                let typed = texts.into_iter().next().unwrap_or_default();
                let dir = crate::shell::expand_home(&typed).display().to_string();
                this.run_mutation(
                    "patch.export",
                    json!({ "commits": commits, "dir": dir }),
                    say!("workspace-patches-written", count, dir = dir.as_str()),
                    |_, _, _, _| false,
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Ask for a file and write changes there as a patch: the ones that
    /// are staged, or the ones that are not.
    pub fn save_changes_as_patch(
        &mut self,
        staged: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = if staged {
            "staged.patch"
        } else {
            "changes.patch"
        };
        let file = self
            .main_folder()
            .parent()
            .map(|parent| parent.join(name).display().to_string())
            .unwrap_or_else(|| name.to_owned());
        self.prompt(
            if staged {
                say!("workspace-patch-save-staged").to_owned()
            } else {
                say!("workspace-patch-save-unstaged").to_owned()
            },
            say!("workspace-save"),
            vec![PromptField::required(say!("workspace-patch-file"), &file)],
            move |this, texts, window, cx| {
                let typed = texts.into_iter().next().unwrap_or_default();
                let file = crate::shell::expand_home(&typed).display().to_string();
                let source = if staged { "staged" } else { "worktree" };
                this.run_mutation(
                    "patch.export",
                    json!({ source: true, "file": file }),
                    say!("workspace-patch-written", file = file.as_str()),
                    |_, _, _, _| false,
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Ask for a patch file and apply it to the files here; or, for a
    /// mailbox, make its commits (`git am`).
    pub fn prompt_apply_patch(
        &mut self,
        mailbox: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Typed or chosen; a patch is looked at before it is applied
        // (`workspace/patches.rs`).
        self.prompt_patch_file(mailbox, window, cx);
    }

    /// Switch remembering how conflicts were resolved (rerere) on or off
    /// for this repository, and say which it is now.
    pub fn toggle_rerere(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.session.send(Request::Command {
            id: "rerere.status".to_owned(),
            params: json!({}),
        });
        cx.spawn_in(window, async move |this, cx| {
            let enabled = match reply.await {
                Ok(Reply::Command(value)) => {
                    value.get("enabled").and_then(|flag| flag.as_bool()) == Some(true)
                }
                _ => false,
            };
            let _ = this.update_in(cx, |this, window, cx| {
                this.run_mutation(
                    "rerere.enable",
                    json!({ "on": !enabled }),
                    if enabled {
                        say!("workspace-rerere-off").to_owned()
                    } else {
                        say!("workspace-rerere-on").to_owned()
                    },
                    |_, _, _, _| false,
                    window,
                    cx,
                )
            });
        })
        .detach();
    }

    /// Who commits changed outside this tab (a profile was switched to):
    /// read again what says so here.
    pub fn identity_changed(&mut self, cx: &mut Context<Self>) {
        self.commit_box.update(cx, |view, cx| view.load_signing(cx));
    }

    /// Commit as a profile in this repository only: its identity is
    /// written to the repository's own settings.
    pub fn use_profile_here(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.run_mutation(
            "profile.use",
            json!({ "name": name, "scope": "repo" }),
            say!("workspace-profile-used", name = name.as_str()),
            |_, _, _, _| false,
            window,
            cx,
        );
        self.identity_changed(cx);
    }

    /// Make this application what opens `bitbraid://` links on this
    /// desktop.
    pub fn register_links(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_mutation(
            "link.register",
            json!({}),
            say!("workspace-links-registered").to_owned(),
            |_, _, _, _| false,
            window,
            cx,
        );
    }

    fn toggle_sheet(&mut self, sheet: Sheet, cx: &mut Context<Self>) {
        self.sheet = (self.sheet != Some(sheet)).then_some(sheet);
        self.refresh_sheet(cx);
        cx.notify();
    }

    /// Read again what the sheet that is shown lists.
    fn refresh_sheet(&mut self, cx: &mut Context<Self>) {
        match self.sheet {
            Some(Sheet::Activity) => self.activity_view.update(cx, |view, cx| view.reload(cx)),
            Some(Sheet::Journal) => self.journal_view.update(cx, |view, cx| view.reload(cx)),
            Some(Sheet::Rerere) => self.rerere_view.update(cx, |view, cx| view.reload(cx)),
            Some(Sheet::Flags) => self.flags_view.update(cx, |view, cx| view.reload(cx)),
            // Read from the provider when it is opened and when asked.
            Some(Sheet::Pull | Sheet::NewPull | Sheet::Issue) | None => {}
        }
        // The preferences show where LFS stands here. How it signs is not
        // read again after every action: looking for keys asks gpg and the
        // agent.
        if self.settings_shown {
            self.lfs_view.update(cx, |view, cx| view.reload(cx));
        }
    }

    /// Whether the preferences show this repository's sections.
    pub fn settings_shown(&self) -> bool {
        self.settings_shown
    }

    /// Whether the preferences show this repository's sections: what they
    /// show is read again with the rest.
    pub fn set_settings_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if shown && !self.settings_shown {
            self.reload_settings(cx);
        }
        self.settings_shown = shown;
    }

    /// What the status bar says about the running operation, or about a
    /// command that changes the repository and has run for a moment.
    pub fn operation_text(&self) -> Option<String> {
        let Some(operation) = self.operation.as_ref() else {
            return self.busy_text();
        };
        let mut text = operation.label.clone();
        match &operation.stage {
            Some((stage, Some(percent))) => text.push_str(&format!(": {stage} {percent} %")),
            Some((stage, None)) => text.push_str(&format!(": {stage}")),
            None => text.push('…'),
        }
        if !self.operations_waiting.is_empty() {
            text.push_str(&format!(
                "  {}",
                say!(
                    "workspace-operations-waiting",
                    count = self.operations_waiting.len()
                )
            ));
        }
        Some(text)
    }

    /// Stop the running operation.
    pub fn cancel_operation(&mut self, cx: &mut Context<Self>) {
        self.session
            .call(Request::CancelOperation, cx, |_, _, _| {});
    }

    /// Fetch from every remote, pruning branches that are gone there.
    pub fn fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.auto_fetch.fetched_by_hand();
        let params = sync::fetch_params(None, &cx.global::<Prefs>().preferences);
        self.run_operation(
            "remote.fetch",
            params,
            say!("workspace-fetching"),
            say!("workspace-fetched").to_owned(),
            window,
            cx,
        );
    }

    /// How this repository pulls: its own choice when it has made one, the
    /// preference otherwise.
    pub fn pull_mode(&self, cx: &App) -> String {
        let prefs = cx.global::<Prefs>();
        prefs
            .repo_state(&self.session.path)
            .pull_mode
            .filter(|mode| PULL_MODES.iter().any(|(id, _)| id == mode))
            .unwrap_or_else(|| prefs.preferences.pull_mode.clone())
    }

    /// What the toolbar's Pull button does in this repository: one of the
    /// ways to pull, or [`FETCH_MODE`].
    pub fn pull_button(&self, cx: &App) -> String {
        let prefs = cx.global::<Prefs>();
        prefs
            .repo_state(&self.session.path)
            .pull_mode
            .filter(|mode| mode == FETCH_MODE || PULL_MODES.iter().any(|(id, _)| id == mode))
            .unwrap_or_else(|| prefs.preferences.pull_mode.clone())
    }

    /// Make the Pull button do this here from now on (one of the ways to
    /// pull, or [`FETCH_MODE`]). Nothing is run.
    pub fn choose_pull(&mut self, mode: &str, cx: &mut Context<Self>) {
        if mode != FETCH_MODE && !PULL_MODES.iter().any(|(id, _)| *id == mode) {
            return;
        }
        let path = self.session.path.clone();
        cx.global_mut::<Prefs>()
            .update_repo_state(&path, |state| state.pull_mode = Some(mode.to_owned()));
        cx.notify();
    }

    /// What the toolbar's Pull button does: pull as chosen here, or fetch.
    pub fn pull_or_fetch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pull_button(cx) == FETCH_MODE {
            self.fetch(window, cx);
        } else {
            self.pull(window, cx);
        }
    }

    /// Names of the remotes, in the order the panel shows them.
    pub fn remote_names(&self) -> Vec<String> {
        self.snapshot
            .as_ref()
            .map(|snapshot| snapshot.remotes.iter().map(|r| r.name.clone()).collect())
            .unwrap_or_default()
    }

    /// Fetch, add, repoint or remove a remote.
    pub fn remote(&mut self, request: RemoteRequest, window: &mut Window, cx: &mut Context<Self>) {
        let quiet = |_: &mut Self, _: &str, _: &mut Window, _: &mut Context<Self>| false;
        match request {
            RemoteRequest::Fetch(name) => self.run_operation(
                "remote.fetch",
                sync::fetch_params(Some(&name), &cx.global::<Prefs>().preferences),
                say!("workspace-fetching"),
                say!("workspace-fetched-remote", name = name.as_str()),
                window,
                cx,
            ),
            RemoteRequest::Add => self.dock(docked::Adds::Remote, window, cx),
            RemoteRequest::EditUrl(name) => {
                let remote = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| snapshot.remotes.iter().find(|r| r.name == name));
                let fetch = remote.and_then(|r| r.fetch_url.clone()).unwrap_or_default();
                let push = remote.and_then(|r| r.push_url.clone()).unwrap_or_default();
                self.prompt(
                    say!("workspace-remote-urls-title", name = name.as_str()),
                    say!("workspace-save"),
                    vec![
                        PromptField::required(say!("workspace-remote-url"), &fetch),
                        PromptField {
                            placeholder: say!("workspace-remote-push-url"),
                            initial: push.clone(),
                            required: false,
                        },
                    ],
                    move |this, texts, window, cx| {
                        let mut texts = texts.into_iter();
                        let url = texts.next().unwrap_or_default();
                        let new_push = texts.next().unwrap_or_default();
                        // A push URL that was there and is emptied goes back
                        // to being the same as the URL.
                        let push_to = if new_push.is_empty() && !push.is_empty() {
                            Some(url.clone())
                        } else if new_push != push {
                            Some(new_push)
                        } else {
                            None
                        };
                        this.set_remote_urls(name.clone(), url, push_to, window, cx);
                    },
                    window,
                    cx,
                );
            }
            RemoteRequest::Remove(name) => self.ask_in_bar(
                say!("workspace-remote-remove-question", name = name.as_str()),
                say!("workspace-remove"),
                true,
                move |this, window, cx| {
                    this.run_mutation(
                        "remote.remove",
                        json!({ "name": name }),
                        say!("workspace-remote-removed", name = name.as_str()),
                        quiet,
                        window,
                        cx,
                    )
                },
                window,
                cx,
            ),
        }
    }

    /// Point a remote somewhere else: its URL, and its push URL when given.
    fn set_remote_urls(
        &mut self,
        name: String,
        url: String,
        push: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut replies = vec![self.session.send(Request::Command {
            id: "remote.set_url".to_owned(),
            params: json!({ "name": name, "url": url }),
        })];
        if let Some(push) = push {
            replies.push(self.session.send(Request::Command {
                id: "remote.set_url".to_owned(),
                params: json!({ "name": name, "url": push, "push": true }),
            }));
        }
        cx.spawn_in(window, async move |this, cx| {
            let mut failed = None;
            for reply in replies {
                if let Ok(Reply::Error(err)) = reply.await {
                    failed.get_or_insert(err);
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                match failed {
                    Some(err) => crate::notices::push(window, cx, Notice::error(sentence(&err))),
                    None => crate::notices::push(
                        window,
                        cx,
                        Notice::success(say!("workspace-remote-urls-saved", name = name.as_str())),
                    ),
                }
                this.reload(cx);
            });
        })
        .detach();
    }

    /// Pull the current branch the way this repository pulls. When the
    /// upstream as it was last fetched already conflicts with the branch,
    /// say so first.
    pub fn pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.pull_mode(cx);
        let upstream = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.branches.iter().find(|branch| branch.is_head))
            .and_then(|branch| branch.upstream.clone());
        match upstream {
            // A fast-forward cannot conflict.
            Some(upstream) if mode != "ff_only" => {
                let meeting = Meeting {
                    what: say!("workspace-pulling-upstream", upstream = upstream.as_str()),
                    target: upstream,
                    go_on: say!("workspace-pull-anyway"),
                    caveat: say!("workspace-pull-caveat"),
                };
                let go: GoOn = std::rc::Rc::new(|this, window, cx| this.run_pull(window, cx));
                self.foretell(meeting, go, window, cx);
            }
            _ => self.run_pull(window, cx),
        }
    }

    fn run_pull(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.pull_mode(cx);
        self.run_operation(
            "remote.pull",
            json!({ "mode": mode }),
            say!("workspace-pulling"),
            say!("workspace-pulled").to_owned(),
            window,
            cx,
        );
    }

    /// Push the current branch; the first push of a branch makes the pushed
    /// branch its upstream.
    pub fn push(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let head = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.branches.iter().find(|branch| branch.is_head));
        let params = match head {
            Some(branch) if branch.upstream.is_none() => {
                json!({ "branch": branch.name, "set_upstream": true })
            }
            _ => json!({}),
        };
        let what = head
            .map(|branch| branch.name.clone())
            .unwrap_or_else(|| "HEAD".to_owned());
        self.run_operation_with(
            "remote.push",
            params,
            say!("workspace-pushing"),
            say!("workspace-pushed", what = what.as_str()),
            |this, err, window, cx| {
                if !push_rejected(err) {
                    return false;
                }
                // The remote has what this branch has not: asked in the
                // bar, pull first or overwrite (which lists what it would
                // overwrite before it does).
                let (for_pull, for_force) = (cx.entity().downgrade(), cx.entity().downgrade());
                let _ = this;
                crate::ask::show(
                    crate::ask::Ask::new(say!("workspace-push-rejected"))
                        .go(say!("workspace-pull"), move |_, window, cx| {
                            let _ = for_pull.update(cx, |this, cx| this.pull(window, cx));
                        })
                        .danger(say!("workspace-force-push"), move |_, window, cx| {
                            let _ = for_force.update(cx, |this, cx| {
                                this.push_how(None, PushHow::Force, window, cx)
                            });
                        }),
                    window,
                    cx,
                );
                true
            },
            window,
            cx,
        );
    }

    /// Check out a branch, a remote branch or a commit. When local changes
    /// are in the way, ask what to do with them.
    pub fn checkout(&mut self, target: String, window: &mut Window, cx: &mut Context<Self>) {
        let again = target.clone();
        self.run_mutation(
            "branch.checkout",
            json!({ "target": target }),
            say!("workspace-checked-out", target = target.as_str()),
            move |this, err, window, cx| {
                // git names the files a checkout would overwrite.
                if !err.contains("would be overwritten") {
                    return false;
                }
                // Set aside and brought back without asking, when the
                // preferences say so.
                if cx.global::<Prefs>().preferences.checkout_stash {
                    this.checkout_with(again, "stash", window, cx);
                } else {
                    this.ask_about_local_changes(again, window, cx);
                }
                true
            },
            window,
            cx,
        );
    }

    fn checkout_with(
        &mut self,
        target: String,
        dirty: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_mutation(
            "branch.checkout",
            json!({ "target": target, "dirty": dirty }),
            say!("workspace-checked-out", target = target.as_str()),
            |_, _, _, _| false,
            window,
            cx,
        );
    }

    fn ask_about_local_changes(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let (stash_target, discard_target) = (target.clone(), target.clone());
            let (for_stash, for_discard) = (workspace.clone(), workspace.clone());
            dialog
                .title(say!("workspace-dirty-title"))
                .child(
                    v_flex()
                        .gap_2()
                        .text_sm()
                        .child(say!("workspace-dirty-overwrite", target = target.as_str()))
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(say!("workspace-dirty-explained")),
                        ),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("dirty-cancel")
                                .ghost()
                                .label(say!("workspace-cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("dirty-discard")
                                .ghost()
                                .label(say!("workspace-dirty-discard"))
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let target = discard_target.clone();
                                    let _ = for_discard.update(cx, |this, cx| {
                                        this.checkout_with(target, "discard", window, cx)
                                    });
                                }),
                        )
                        .child(
                            Button::new("dirty-stash")
                                .primary()
                                .label(say!("workspace-dirty-stash"))
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let target = stash_target.clone();
                                    let _ = for_stash.update(cx, |this, cx| {
                                        this.checkout_with(target, "stash", window, cx)
                                    });
                                }),
                        ),
                )
        });
    }

    /// Open the name prompt: a title, a text field and what to do with the
    /// name once it is confirmed.
    fn prompt_name(
        &mut self,
        title: String,
        initial: &str,
        ok_text: &'static str,
        on_name: impl Fn(&mut Self, String, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.prompt(
            title,
            ok_text,
            vec![PromptField::required(
                say!("workspace-prompt-name"),
                initial,
            )],
            move |this, mut texts, window, cx| {
                on_name(this, texts.pop().unwrap_or_default(), window, cx)
            },
            window,
            cx,
        );
    }

    /// Ask in the bar for one or two texts (`crate::ask`). `on_ok` gets
    /// what was typed, trimmed, one text per field; the bar waits while a
    /// required field is empty.
    fn prompt(
        &mut self,
        title: String,
        ok_text: &'static str,
        fields: Vec<PromptField>,
        on_ok: impl Fn(&mut Self, Vec<String>, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.forms.close();
        let mut ask = crate::ask::Ask::new(title);
        for field in fields {
            ask = if field.required {
                ask.field(field.placeholder, &field.initial)
            } else {
                ask.optional_field(field.placeholder, &field.initial)
            };
        }
        let workspace = cx.entity().downgrade();
        crate::ask::show(
            ask.go(ok_text, move |texts, window, cx| {
                let _ = workspace.update(cx, |this, cx| on_ok(this, texts, window, cx));
            }),
            window,
            cx,
        );
    }

    /// Run what the open prompt is for. False while a required field is
    /// empty, which keeps the prompt open.
    fn confirm_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        // A dialog with options borrows the fields; Enter is its own.
        if self.forms.is_open() {
            return self.confirm_form(window, cx);
        }
        let texts: Vec<String> = [&self.prompt, &self.prompt_optional]
            .into_iter()
            .take(self.prompt_required.len())
            .map(|input| input.read(cx).value().trim().to_owned())
            .collect();
        let missing = texts
            .iter()
            .zip(&self.prompt_required)
            .any(|(text, required)| *required && text.is_empty());
        if missing {
            return false;
        }
        if let Some(action) = self.prompt_action.take() {
            action(self, texts, window, cx);
        }
        true
    }

    fn prompt_rename_branch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let old = name.clone();
        self.prompt_name(
            say!("workspace-rename-branch-title", name = name.as_str()),
            &name,
            say!("workspace-rename"),
            move |this, new_name, window, cx| {
                this.run_mutation(
                    "branch.rename",
                    json!({ "name": old, "new_name": new_name }),
                    say!(
                        "workspace-renamed-branch",
                        old = old.as_str(),
                        new_name = new_name.as_str()
                    ),
                    |_, _, _, _| false,
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// The main worktree's folder: where a new worktree is suggested
    /// beside, and what a submodule's path is taken from.
    fn main_folder(&self) -> std::path::PathBuf {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.worktrees.iter().find(|w| w.is_main))
            .and_then(|main| main.path.clone())
            .unwrap_or_else(|| self.session.path.clone())
    }

    /// Add, open, move, lock, unlock, remove or prune a worktree, or open a
    /// terminal in one.
    pub fn worktree(
        &mut self,
        request: WorktreeRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // What this changes is to show as soon as it is done.
        self.places_loaded = None;
        let quiet = |_: &mut Self, _: &str, _: &mut Window, _: &mut Context<Self>| false;
        match request {
            WorktreeRequest::Open(path) => cx.emit(WorkspaceEvent::Open(path.into())),
            WorktreeRequest::OpenHere(path) => cx.emit(WorkspaceEvent::OpenInPlace(path.into())),
            // The terminal the preferences name, or the one found, started
            // in that worktree's folder. It says something only when it
            // could not be started.
            WorktreeRequest::Terminal(path) => {
                let command = cx.global::<Prefs>().preferences.terminal.clone();
                let reply = self.session.send(Request::Command {
                    id: "repo.open_terminal".to_owned(),
                    params: json!({ "command": command, "path": path }),
                });
                cx.spawn_in(window, async move |_, cx| {
                    if let Ok(Reply::Error(err)) = reply.await {
                        let _ = cx.update(|window, cx| {
                            crate::notices::push(window, cx, Notice::error(sentence(&err)));
                        });
                    }
                })
                .detach();
            }
            WorktreeRequest::New(start) => self.dock(docked::Adds::Worktree(start), window, cx),
            WorktreeRequest::Lock(path) => {
                let name = worktrees::folder_name(&path);
                self.prompt(
                    say!("workspace-worktree-lock-title", name),
                    say!("workspace-worktree-lock"),
                    vec![PromptField::optional(say!("workspace-worktree-lock-why"))],
                    move |this, texts, window, cx| {
                        let reason = texts.into_iter().next().unwrap_or_default();
                        this.run_mutation(
                            "worktree.lock",
                            json!({ "path": path, "reason": reason }),
                            say!(
                                "workspace-worktree-locked",
                                name = worktrees::folder_name(&path)
                            ),
                            quiet,
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            }
            WorktreeRequest::Move(path) => {
                let name = worktrees::folder_name(&path);
                self.prompt(
                    say!("workspace-worktree-move-title", name),
                    say!("workspace-worktree-move"),
                    vec![PromptField::required(
                        say!("workspace-worktree-move-to"),
                        &path,
                    )],
                    move |this, texts, window, cx| {
                        let typed = texts.into_iter().next().unwrap_or_default();
                        let to = crate::shell::expand_home(&typed).display().to_string();
                        // A move across file systems copies every file.
                        this.run_operation(
                            "worktree.move",
                            json!({ "path": path, "to": to }),
                            say!("workspace-worktree-moving"),
                            say!(
                                "workspace-worktree-moved",
                                name = worktrees::folder_name(&path),
                                to = to.as_str()
                            ),
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            }
            WorktreeRequest::Unlock(path) => self.run_mutation(
                "worktree.unlock",
                json!({ "path": path }),
                say!(
                    "workspace-worktree-unlocked",
                    name = worktrees::folder_name(&path)
                ),
                quiet,
                window,
                cx,
            ),
            WorktreeRequest::Remove { path, with_branch } => {
                let name = worktrees::folder_name(&path);
                self.ask_in_bar(
                    if with_branch {
                        say!("workspace-worktree-remove-with-branch-question", name)
                    } else {
                        say!("workspace-worktree-remove-question", name)
                    },
                    say!("workspace-remove"),
                    true,
                    move |this, window, cx| {
                        this.remove_worktree(path.clone(), with_branch, false, window, cx)
                    },
                    window,
                    cx,
                );
            }
            WorktreeRequest::Prune => self.run_mutation(
                "worktree.prune",
                json!({}),
                say!("workspace-worktrees-pruned").to_owned(),
                quiet,
                window,
                cx,
            ),
        }
    }

    /// Remove a worktree; one with changes is asked about once more. An
    /// operation, since deleting a large one's files takes a while: in the
    /// status bar, and marked in its rows, until it is done.
    fn remove_worktree(
        &mut self,
        path: String,
        with_branch: bool,
        force: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = worktrees::folder_name(&path);
        let reply = self.session.send(Request::Operation {
            id: "worktree.remove".to_owned(),
            params: json!({ "path": path, "force": force, "delete_branch": with_branch }),
        });
        self.operation_began(
            &say!("workspace-worktree-removing", name = name.as_str()),
            Some("worktree.remove"),
        );
        self.set_removing(&path, true, cx);
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.operation_ended();
                this.set_removing(&path, false, cx);
                cx.notify();
                match reply {
                    // Git, and the engine before it, refuse one with work
                    // in it.
                    Ok(Reply::Error(err)) if !force && err.contains("they would be lost") => {
                        this.ask_in_bar(
                            say!(
                                "workspace-worktree-remove-anyway-question",
                                name = name.as_str()
                            ),
                            say!("workspace-remove"),
                            true,
                            move |this, window, cx| {
                                this.remove_worktree(path.clone(), with_branch, true, window, cx)
                            },
                            window,
                            cx,
                        );
                        return;
                    }
                    Ok(Reply::Error(err)) => {
                        crate::notices::push(window, cx, Notice::error(sentence(&err)))
                    }
                    Ok(Reply::Command(value)) => {
                        // The folder is gone; the branch may have stayed.
                        match value.get("branch_kept").and_then(|kept| kept.as_str()) {
                            Some(why) => crate::notices::push(
                                window,
                                cx,
                                Notice::warning(say!(
                                    "workspace-worktree-removed-branch-kept",
                                    name = name.as_str(),
                                    why = sentence(why)
                                )),
                            ),
                            None => crate::notices::push(
                                window,
                                cx,
                                Notice::success(say!(
                                    "workspace-worktree-removed",
                                    name = name.as_str()
                                )),
                            ),
                        }
                    }
                    _ => {}
                }
                this.after_mutation(cx);
                this.reload(cx);
            });
        })
        .detach();
    }

    /// Add, open, update, sync or remove a submodule.
    pub fn submodule(
        &mut self,
        request: SubmoduleRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.places_loaded = None;
        let quiet = |_: &mut Self, _: &str, _: &mut Window, _: &mut Context<Self>| false;
        match request {
            SubmoduleRequest::Open(path) => {
                let folder = self
                    .session
                    .summary
                    .workdir
                    .clone()
                    .unwrap_or_else(|| self.session.path.clone());
                cx.emit(WorkspaceEvent::Open(folder.join(path)));
            }
            SubmoduleRequest::Add => self.dock(docked::Adds::Submodule, window, cx),
            SubmoduleRequest::Update(path) => self.update_submodules(path, false, window, cx),
            SubmoduleRequest::Sync(path) => {
                let paths: Vec<String> = path.iter().cloned().collect();
                self.run_mutation(
                    "submodule.sync",
                    json!({ "paths": paths, "recursive": true }),
                    match &path {
                        Some(path) => say!("workspace-submodule-synced", path),
                        None => say!("workspace-submodules-synced").to_owned(),
                    },
                    quiet,
                    window,
                    cx,
                );
            }
            SubmoduleRequest::Remove(path) => self.prompt_remove_submodule(path, window, cx),
        }
    }

    /// Ask whether git may clone from a local path, which it does not do
    /// for a submodule by itself, and go on when it may.
    fn ask_local_clone(
        &mut self,
        go_on: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let go_on = std::rc::Rc::new(go_on);
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (workspace, go_on) = (workspace.clone(), go_on.clone());
            alert
                .title(say!("workspace-local-clone-title"))
                .description(say!("workspace-local-clone-description"))
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let go_on = go_on.clone();
                    let _ = workspace.update(cx, |this, cx| go_on(this, window, cx));
                    true
                })
        });
    }

    fn update_submodules(
        &mut self,
        path: Option<String>,
        allow_local: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<String> = path.iter().cloned().collect();
        let again = path.clone();
        self.run_operation_with(
            "submodule.update",
            json!({ "paths": paths, "init": true, "recursive": true, "allow_local": allow_local }),
            say!("workspace-submodules-updating"),
            match &path {
                Some(path) => say!("workspace-submodule-updated", path),
                None => say!("workspace-submodules-updated").to_owned(),
            },
            move |this, err, window, cx| {
                if allow_local || !refuses_local_clone(err) {
                    return false;
                }
                let path = again.clone();
                this.ask_local_clone(
                    move |this, window, cx| this.update_submodules(path.clone(), true, window, cx),
                    window,
                    cx,
                );
                true
            },
            window,
            cx,
        );
    }

    /// Set gitflow up, start or finish one of its branches, describe a
    /// branch, or delete the merged ones.
    pub fn flow(&mut self, request: FlowRequest, window: &mut Window, cx: &mut Context<Self>) {
        self.places_loaded = None;
        let quiet = |_: &mut Self, _: &str, _: &mut Window, _: &mut Context<Self>| false;
        match request {
            FlowRequest::SetUp => cx.emit(WorkspaceEvent::OpenSettings(
                crate::views::preferences::Section::Gitflow,
            )),
            FlowRequest::Start(kind) => self.flow_start(Some(kind), window, cx),
            FlowRequest::StartAny => self.flow_start(None, window, cx),
            FlowRequest::FinishCurrent => self.flow_finish_current(window, cx),
            FlowRequest::Finish { kind, name, keep } => {
                let explained = flow::finish_explained(&kind, &self.flow_state, keep);
                self.ask_in_bar(
                    say!(
                        "workspace-flow-finish-question",
                        kind = kind.as_str(),
                        name = name.as_str(),
                        explained
                    ),
                    say!("workspace-flow-finish"),
                    false,
                    move |this, window, cx| {
                        // Merges run hooks and can stop on a conflict: an
                        // operation, like any merge. Should it stop, it is
                        // taken up again when the merge is concluded.
                        this.finishing = Some((kind.clone(), name.clone(), keep));
                        this.run_operation(
                            "flow.finish",
                            json!({ "kind": kind, "name": name, "keep": keep }),
                            say!("workspace-flow-finishing"),
                            say!(
                                "workspace-flow-finished",
                                kind = kind.as_str(),
                                name = name.as_str()
                            ),
                            window,
                            cx,
                        )
                    },
                    window,
                    cx,
                );
            }
            FlowRequest::Describe(name) => {
                let request = Request::Command {
                    id: "branch.overview".to_owned(),
                    params: json!({}),
                };
                // What it says now is what the prompt starts with.
                let reply = self.session.send(request);
                cx.spawn_in(window, async move |this, cx| {
                    let current = match reply.await {
                        Ok(Reply::Command(value)) => flow::notes_from(&value)
                            .remove(&name)
                            .and_then(|note| note.description)
                            .unwrap_or_default(),
                        _ => String::new(),
                    };
                    let _ = this.update_in(cx, |this, window, cx| {
                        let branch = name.clone();
                        this.prompt(
                            say!("workspace-describe-title", name = name.as_str()),
                            say!("workspace-save"),
                            vec![PromptField {
                                placeholder: say!("workspace-describe-placeholder"),
                                initial: current,
                                required: false,
                            }],
                            move |this, texts, window, cx| {
                                let description = texts.into_iter().next().unwrap_or_default();
                                this.run_mutation(
                                    "branch.describe",
                                    json!({ "name": branch, "description": description }),
                                    if description.is_empty() {
                                        say!(
                                            "workspace-description-removed",
                                            branch = branch.as_str()
                                        )
                                    } else {
                                        say!("workspace-described", branch = branch.as_str())
                                    },
                                    quiet,
                                    window,
                                    cx,
                                );
                            },
                            window,
                            cx,
                        );
                    });
                })
                .detach();
            }
            FlowRequest::DeleteMerged => {
                let protected = cx.global::<Prefs>().preferences.protected_branches.clone();
                let reply = self.session.send(Request::Command {
                    id: "branch.delete_merged".to_owned(),
                    params: json!({ "protected": protected, "dry_run": true }),
                });
                cx.spawn_in(window, async move |this, cx| {
                    let Ok(reply) = reply.await else { return };
                    let _ = this.update_in(cx, |_, window, cx| {
                        let value = match reply {
                            Reply::Command(value) => value,
                            Reply::Error(err) => {
                                crate::notices::push(window, cx, Notice::error(sentence(&err)));
                                return;
                            }
                            _ => return,
                        };
                        let base = value
                            .get("base")
                            .and_then(|base| base.as_str())
                            .unwrap_or(say!("workspace-base-branch"))
                            .to_owned();
                        let names: Vec<String> = value
                            .get("deleted")
                            .and_then(|list| list.as_array())
                            .into_iter()
                            .flatten()
                            .filter_map(|branch| branch.get("name")?.as_str().map(str::to_owned))
                            .collect();
                        if names.is_empty() {
                            crate::notices::push(
                                window,
                                cx,
                                Notice::info(say!(
                                    "workspace-delete-merged-none",
                                    base = base.as_str()
                                )),
                            );
                            return;
                        }
                        let workspace = cx.entity().downgrade();
                        let listed = if names.len() > 12 {
                            say!(
                                "workspace-names-and-more",
                                names = names
                                    .iter()
                                    .take(12)
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                more = names.len().saturating_sub(12)
                            )
                        } else {
                            names.join(", ")
                        };
                        let count = names.len();
                        window.open_alert_dialog(cx, move |alert, _, _| {
                            let (workspace, protected) = (workspace.clone(), protected.clone());
                            alert
                                .title(say!(
                                    "workspace-delete-merged-title",
                                    count,
                                    base = base.as_str()
                                ))
                                .description(say!(
                                    "workspace-delete-merged-description",
                                    listed = listed.as_str()
                                ))
                                .show_cancel(true)
                                .on_ok(move |_, window, cx| {
                                    let protected = protected.clone();
                                    let _ = workspace.update(cx, |this, cx| {
                                        this.run_mutation(
                                            "branch.delete_merged",
                                            json!({ "protected": protected }),
                                            say!("workspace-deleted-merged", count),
                                            |_, _, _, _| false,
                                            window,
                                            cx,
                                        )
                                    });
                                    true
                                })
                        });
                    });
                })
                .detach();
            }
        }
    }

    /// Make a tag, push one, or delete one here or on the remote.
    pub fn tag(&mut self, request: TagRequest, window: &mut Window, cx: &mut Context<Self>) {
        let quiet = |_: &mut Self, _: &str, _: &mut Window, _: &mut Context<Self>| false;
        match request {
            // Named in the graph's label column, where it shows the
            // commit; asked in a dialog where it does not.
            TagRequest::New(target) => {
                if !self.name_in_graph(NameFor::Tag, Some(target.clone()), "", window, cx) {
                    self.prompt_new_tag(target, window, cx)
                }
            }
            TagRequest::NewAnnotated(target) => {
                if !self.name_in_graph(NameFor::AnnotatedTag, Some(target.clone()), "", window, cx)
                {
                    self.prompt_new_tag(target, window, cx)
                }
            }
            // Every tag, to a remote that is asked for when there are
            // several.
            TagRequest::PushAll => self.with_remote(
                say!("workspace-push-tags-title").to_owned(),
                say!("workspace-push-tags"),
                |this, remote, window, cx| this.push_all_tags(remote, window, cx),
                window,
                cx,
            ),
            TagRequest::Annotate(name) => self.prompt(
                say!("workspace-annotate-title", name = name.as_str()),
                say!("workspace-annotate"),
                vec![PromptField::required(say!("workspace-prompt-message"), "")],
                move |this, mut texts, window, cx| {
                    let message = texts.pop().unwrap_or_default();
                    this.run_mutation(
                        "tag.annotate",
                        json!({ "name": name, "message": message }),
                        say!("workspace-annotated", name = name.as_str()),
                        quiet,
                        window,
                        cx,
                    );
                },
                window,
                cx,
            ),
            TagRequest::Details(name) => {
                let reply = self.session.send(Request::Command {
                    id: "tag.details".to_owned(),
                    params: json!({ "name": name }),
                });
                cx.spawn_in(window, async move |this, cx| {
                    let Ok(reply) = reply.await else { return };
                    let _ = this.update_in(cx, |_, window, cx| match reply {
                        Reply::Command(value) => {
                            let lines = crate::views::tags::details_lines(&value);
                            window.open_dialog(cx, move |dialog, _, cx| {
                                let muted = cx.theme().muted_foreground;
                                dialog
                                    .title(say!("workspace-tag-title", name = name.as_str()))
                                    .child(v_flex().gap_1().text_sm().children(
                                        lines.iter().enumerate().map(|(ix, line)| {
                                            div()
                                                .min_h(px(8.))
                                                .whitespace_normal()
                                                // What it is and who made it,
                                                // then the message as written.
                                                .when(ix > 0 && ix < 3, |d| d.text_color(muted))
                                                .child(line.clone())
                                        }),
                                    ))
                            });
                        }
                        Reply::Error(err) => {
                            crate::notices::push(window, cx, Notice::error(sentence(&err)))
                        }
                        _ => {}
                    });
                })
                .detach();
            }
            TagRequest::Push(name) => self.with_remote(
                say!("workspace-push-tag-title", name = name.as_str()),
                say!("workspace-push-tag"),
                move |this, remote, window, cx| {
                    this.run_operation(
                        "tag.push",
                        json!({ "name": name, "remote": remote }),
                        say!("workspace-pushing-tag"),
                        match &remote {
                            Some(remote) => {
                                say!("workspace-pushed-tag-to", name = name.as_str(), remote)
                            }
                            None => say!("workspace-pushed-tag", name = name.as_str()),
                        },
                        window,
                        cx,
                    )
                },
                window,
                cx,
            ),
            TagRequest::Delete(name) => self.ask_in_bar(
                say!("workspace-delete-tag-question", name = name.as_str()),
                say!("workspace-delete"),
                true,
                move |this, window, cx| {
                    this.run_mutation(
                        "tag.delete",
                        json!({ "name": name }),
                        say!("workspace-deleted-tag", name = name.as_str()),
                        quiet,
                        window,
                        cx,
                    )
                },
                window,
                cx,
            ),
            TagRequest::DeleteRemote(name) => self.with_remote(
                say!("workspace-delete-remote-tag-title", name = name.as_str()),
                say!("workspace-go-on"),
                move |this, remote, window, cx| {
                    this.confirm_delete_remote_tag(name.clone(), remote, window, cx)
                },
                window,
                cx,
            ),
        }
    }

    /// Ask, then delete a tag on a remote: the one named, or the only one.
    fn confirm_delete_remote_tag(
        &mut self,
        name: String,
        remote: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let there = remote
            .clone()
            .unwrap_or_else(|| say!("workspace-the-remote").to_owned());
        self.ask_in_bar(
            say!(
                "workspace-delete-remote-tag-question",
                name = name.as_str(),
                there = there.as_str()
            ),
            say!("workspace-delete"),
            true,
            move |this, window, cx| {
                this.run_operation(
                    "tag.delete_remote",
                    json!({ "name": name, "remote": remote }),
                    say!("workspace-deleting-remote-tag"),
                    say!(
                        "workspace-deleted-remote-tag",
                        name = name.as_str(),
                        there = there.as_str()
                    ),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
    }

    /// Stash the local changes, or apply, pop or drop a stash.
    pub fn stash(&mut self, request: StashRequest, window: &mut Window, cx: &mut Context<Self>) {
        let quiet = |_: &mut Self, _: &str, _: &mut Window, _: &mut Context<Self>| false;
        match request {
            StashRequest::Push => self.prompt_stash(window, cx),
            StashRequest::PushNow => self.stash_now(window, cx),
            StashRequest::Apply(stash) => self.run_mutation(
                "stash.apply",
                json!({ "stash": stash }),
                say!("workspace-stash-applied").to_owned(),
                quiet,
                window,
                cx,
            ),
            StashRequest::Pop(stash) => self.run_mutation(
                "stash.pop",
                json!({ "stash": stash }),
                say!("workspace-stash-popped").to_owned(),
                quiet,
                window,
                cx,
            ),
            StashRequest::Reword(stash) => {
                // What it is called now, without git's words before it.
                let now = self
                    .snapshot
                    .as_ref()
                    .and_then(|snapshot| {
                        snapshot
                            .stashes
                            .iter()
                            .find(|entry| entry.target.to_hex() == stash)
                    })
                    .map(|entry| crate::views::graph::stash_summary(&entry.message).to_owned())
                    .unwrap_or_default();
                self.prompt(
                    say!("workspace-stash-message-title").to_owned(),
                    say!("workspace-save"),
                    vec![PromptField::required(
                        say!("workspace-prompt-message"),
                        &now,
                    )],
                    move |this, mut texts, window, cx| {
                        let message = texts.pop().unwrap_or_default();
                        this.run_mutation(
                            "stash.reword",
                            json!({ "stash": stash, "message": message }),
                            say!("workspace-stash-reworded").to_owned(),
                            quiet,
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            }
            StashRequest::Branch(stash) => self.prompt_name(
                say!("workspace-stash-branch-title").to_owned(),
                "",
                say!("workspace-stash-branch"),
                move |this, name, window, cx| {
                    this.run_mutation(
                        "stash.branch",
                        json!({ "name": name, "stash": stash }),
                        say!("workspace-stash-branched", name = name.as_str()),
                        quiet,
                        window,
                        cx,
                    );
                },
                window,
                cx,
            ),
            StashRequest::Drop(stash) => self.ask_in_bar(
                say!("workspace-stash-drop-question"),
                say!("workspace-drop"),
                true,
                move |this, window, cx| {
                    this.run_mutation(
                        "stash.drop",
                        json!({ "stash": stash }),
                        say!("workspace-stash-dropped").to_owned(),
                        quiet,
                        window,
                        cx,
                    )
                },
                window,
                cx,
            ),
        }
    }

    /// Do what was picked after a drop: check out the branch it applies to
    /// when that is not the current one, then the operation itself. The
    /// two are separate steps for undo as well.
    pub fn run_choice(&mut self, choice: Choice, window: &mut Window, cx: &mut Context<Self>) {
        let Some(branch) = choice.checkout.clone() else {
            self.rewrite(choice.rewrite, window, cx);
            return;
        };
        let reply = self.session.send(Request::Command {
            id: "branch.checkout".to_owned(),
            params: json!({ "target": branch }),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| match reply {
                Reply::Error(err) if err.contains("would be overwritten") => {
                    crate::notices::push(
                        window,
                        cx,
                        Notice::error(say!(
                            "workspace-dirty-before-drop",
                            branch = branch.as_str()
                        )),
                    );
                }
                Reply::Error(err) => {
                    crate::notices::push(window, cx, Notice::error(sentence(&err)))
                }
                _ => {
                    this.after_mutation(cx);
                    this.reload(cx);
                    this.rewrite(choice.rewrite.clone(), window, cx);
                }
            });
        })
        .detach();
    }

    /// Merge, rebase, reset, cherry-pick or revert, as an operation. What
    /// throws local changes away asks first, and says how to get them back.
    pub fn rewrite(&mut self, rewrite: Rewrite, window: &mut Window, cx: &mut Context<Self>) {
        // How to merge, what to rebase, which parent of a merge commit:
        // asked in a dialog, which comes back here with the answer.
        if self.ask_how(&rewrite, window, cx) {
            return;
        }
        if let Some(meeting) = rewrite.meets() {
            let go: GoOn =
                std::rc::Rc::new(move |this, window, cx| this.run_rewrite(&rewrite, window, cx));
            self.foretell(meeting, go, window, cx);
            return;
        }
        if let Rewrite::Interactive { first, reword } = &rewrite {
            // Planned in the editor; nothing happens until it is started.
            let (first, reword) = (first.clone(), *reword);
            self.planning = true;
            let hooks = self.hooks_there(bitbraid_git::hooks::Unverified::Rebase, cx);
            self.rebase_editor.update(cx, |editor, cx| {
                editor.set_refusing_hooks(hooks);
                editor.open(first, reword, window, cx)
            });
            cx.notify();
            return;
        }
        if !rewrite.asks_first() {
            self.run_rewrite(&rewrite, window, cx);
            return;
        }
        let branch = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.head.branch.clone())
            .unwrap_or_else(|| "HEAD".to_owned());
        // Asked in the bar that takes the toolbar's place: nothing that
        // would be lost needs listing, and undo brings it back.
        let dropped: Option<String> = match &rewrite {
            Rewrite::Drop { commit } => Some(commit.chars().take(7).collect()),
            _ => None,
        };
        let run = move |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.run_rewrite(&rewrite, window, cx)
        };
        if let Some(short) = dropped {
            self.ask_in_bar(
                say!(
                    "workspace-drop-commit-question",
                    short,
                    branch = branch.as_str()
                ),
                say!("workspace-drop"),
                true,
                run,
                window,
                cx,
            );
            return;
        }
        // Untracked files are not touched by a reset.
        let changes = self
            .status
            .iter()
            .filter(|entry| entry.unstaged != Some(ChangeKind::Untracked))
            .count();
        let said = say!("workspace-reset-question", branch, changes);
        self.ask_in_bar(said, say!("workspace-reset"), true, run, window, cx);
    }

    /// Merge the two tips in memory before they are brought together for
    /// real. When that conflicts, name the files and ask; when it does not,
    /// or there is no telling, go on at once.
    fn foretell(
        &mut self,
        meeting: Meeting,
        go: GoOn,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let reply = self.session.send(Request::Command {
            id: "branch.predict".to_owned(),
            params: json!({ "target": meeting.target }),
        });
        cx.spawn_in(window, async move |this, cx| {
            let conflicts: Vec<String> = match reply.await {
                Ok(Reply::Command(value)) => value
                    .get("conflicts")
                    .and_then(|conflicts| conflicts.as_array())
                    .map(|conflicts| {
                        conflicts
                            .iter()
                            .filter_map(|path| path.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default(),
                // What cannot be merged in memory is for git to refuse in
                // its own words.
                _ => Vec::new(),
            };
            let _ = this.update_in(cx, |this, window, cx| {
                if conflicts.is_empty() {
                    go(this, window, cx);
                    return;
                }
                let (title, text) = conflict_warning(&meeting.what, &conflicts, meeting.caveat);
                let workspace = cx.entity().downgrade();
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let (workspace, go) = (workspace.clone(), go.clone());
                    alert
                        .title(title.clone())
                        .description(text.clone())
                        .ok_text(meeting.go_on)
                        .show_cancel(true)
                        .on_ok(move |_, window, cx| {
                            let _ = workspace.update(cx, |this, cx| go(this, window, cx));
                            true
                        })
                });
            });
        })
        .detach();
    }

    fn run_rewrite(&mut self, rewrite: &Rewrite, window: &mut Window, cx: &mut Context<Self>) {
        let Some(plan) = rewrite.plan() else {
            return;
        };
        self.run_operation(plan.id, plan.params, plan.running, plan.done, window, cx);
    }

    /// Ask, then delete several branches one after the other. Each is its
    /// own step for undo; what git refuses is left and named.
    fn confirm_delete_branches(
        &mut self,
        names: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let listed = {
            let mut shown: Vec<&str> = names.iter().take(8).map(String::as_str).collect();
            if names.len() > shown.len() {
                shown.push("…");
            }
            shown.join(", ")
        };
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (workspace, names) = (workspace.clone(), names.clone());
            alert
                .title(say!("workspace-delete-branches-title", count = names.len()))
                .description(say!(
                    "workspace-delete-branches-description",
                    listed = listed.as_str()
                ))
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let names = names.clone();
                    let _ =
                        workspace.update(cx, |this, cx| this.delete_branches(names, window, cx));
                    true
                })
        });
    }

    fn delete_branches(&mut self, names: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        let protected = cx.global::<Prefs>().preferences.protected_branches.clone();
        let replies: Vec<_> = names
            .iter()
            .map(|name| {
                self.session.send(Request::Command {
                    id: "branch.delete".to_owned(),
                    params: json!({ "name": name, "force": false, "protected": protected }),
                })
            })
            .collect();
        cx.spawn_in(window, async move |this, cx| {
            let mut left = Vec::new();
            for (name, reply) in names.iter().zip(replies) {
                if let Ok(Reply::Error(_)) = reply.await {
                    left.push(name.clone());
                }
            }
            let _ = this.update_in(cx, |this, window, cx| {
                let deleted = names.len().saturating_sub(left.len());
                let note = if left.is_empty() {
                    Notice::success(say!("workspace-deleted-branches", deleted))
                } else {
                    Notice::warning(say!(
                        "workspace-deleted-branches-some-left",
                        deleted,
                        left = left.join(", ")
                    ))
                };
                crate::notices::push(window, cx, note);
                this.after_mutation(cx);
                this.reload(cx);
            });
        })
        .detach();
    }

    /// Push several branches, one operation each; the first push of a
    /// branch makes the pushed branch its upstream.
    fn push_branches(&mut self, names: &[String], window: &mut Window, cx: &mut Context<Self>) {
        for name in names {
            let has_upstream = self
                .snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.branches.iter().find(|b| b.name == *name))
                .is_some_and(|branch| branch.upstream.is_some());
            self.run_operation(
                "remote.push",
                json!({ "branch": name, "set_upstream": !has_upstream }),
                say!("workspace-pushing"),
                say!("workspace-pushed", what = name),
                window,
                cx,
            );
        }
    }

    /// Pull a local branch from its menu: the checked-out one as the
    /// toolbar does; another by fetching its remote and moving it forward,
    /// which is all a pull can be without a worktree to merge in.
    fn pull_branch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let branch = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.branches.iter().find(|b| b.name == name));
        let Some(branch) = branch else {
            return;
        };
        if branch.is_head {
            return self.pull(window, cx);
        }
        let Some(remote) = branch.remote.clone() else {
            return;
        };
        if self.refused_here("remote.fetch", window, cx) {
            return;
        }
        let reply = self.session.send(Request::Operation {
            id: "remote.fetch".to_owned(),
            params: sync::fetch_params(Some(&remote), &cx.global::<Prefs>().preferences),
        });
        self.operation_began(say!("workspace-pulling"), Some("remote.pull"));
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.operation_ended();
                match reply {
                    Ok(Reply::Error(err)) if err.contains("operation: cancelled") => {
                        crate::notices::push(
                            window,
                            cx,
                            Notice::info(say!(
                                "workspace-operation-stopped",
                                label = say!("workspace-pulling")
                            )),
                        )
                    }
                    Ok(Reply::Error(err)) => {
                        let notice = this.failed_notice(&err, cx);
                        crate::notices::push(window, cx, notice)
                    }
                    // Fetched: the branch moves forward, or says why not.
                    Ok(_) => this.run_mutation(
                        "branch.fast_forward",
                        json!({ "name": name }),
                        say!("workspace-pulled-branch", name = name.as_str()),
                        |_, _, _, _| false,
                        window,
                        cx,
                    ),
                    Err(_) => {}
                }
                this.reload(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Ask, then delete a branch on its remote. `name` is `remote/branch`.
    fn confirm_delete_on_remote(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remotes = self.remote_names();
        let Some((remote, branch)) = split_remote(&remotes, &name) else {
            return;
        };
        let protected = cx.global::<Prefs>().preferences.protected_branches.clone();
        self.ask_in_bar(
            say!(
                "workspace-delete-on-remote-question",
                branch = branch.as_str(),
                remote = remote.as_str()
            ),
            say!("workspace-delete"),
            true,
            move |this, window, cx| {
                this.run_operation(
                    "branch.delete_remote",
                    json!({ "remote": remote, "branch": branch, "protected": protected }),
                    say!("workspace-deleting-on-remote"),
                    say!(
                        "workspace-deleted-on-remote",
                        branch = branch.as_str(),
                        remote = remote.as_str()
                    ),
                    window,
                    cx,
                )
            },
            window,
            cx,
        );
    }

    /// Reload as [`Self::reload`] does and ask again for everything around
    /// the references, however lately it was asked for: the user asked.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.places_loaded = None;
        self.worktrees_afresh = true;
        // A hook that was put there or taken away since: the switches
        // that leave hooks out are about the ones that are there.
        self.commit_box
            .update(cx, |commit_box, cx| commit_box.load_hooks(cx));
        self.reload(cx);
    }

    /// Reload references and the graph.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.reloads = self.reloads.saturating_add(1);
        self.loading = true;
        self.error = None;
        self.session.call(Request::Refs, cx, Self::on_refs);
        self.load_graph(cx);
        self.peek_undo(cx);
        cx.notify();
    }

    fn on_refs(&mut self, reply: Reply, cx: &mut Context<Self>) {
        match reply {
            Reply::Refs(snapshot) => {
                self.refs_view
                    .update(cx, |view, cx| view.set_snapshot(snapshot.clone(), cx));
                self.pulls
                    .update(cx, |view, cx| view.set_remotes(&snapshot, cx));
                self.issues_follow(&snapshot, cx);
                let branch = snapshot
                    .head
                    .branch
                    .clone()
                    .filter(|_| !snapshot.head.detached);
                self.changes_view.update(cx, |view, cx| {
                    view.set_branch(snapshot.head.branch.clone(), cx)
                });
                let lightweight = snapshot
                    .tags
                    .iter()
                    .filter(|tag| !tag.annotated)
                    .map(|tag| tag.name.clone())
                    .collect();
                self.graph_view.update(cx, |view, cx| {
                    view.set_current_branch(branch, cx);
                    view.set_lightweight_tags(lightweight);
                });
                self.snapshot = Some(snapshot);
                self.completion_words_follow(cx);
                self.pull_view_follows(cx);
                // The sides of a stopped operation are named by branches.
                let names = self
                    .in_progress
                    .as_ref()
                    .zip(self.snapshot.as_ref())
                    .map(|(operation, snapshot)| side_names(operation, snapshot));
                self.changes_view
                    .update(cx, |view, cx| view.set_side_names(names, cx));
                self.load_tracking(cx);
                self.load_places(cx);
                self.look_at_links(cx);
                self.mark_conflicts(cx);
                cx.notify();
            }
            Reply::Error(err) => self.fail(err, cx),
            _ => {}
        }
    }

    /// Ask how far the local branches are from their upstreams, for the
    /// references panel. It comes after the references themselves, which
    /// are not held up by it.
    fn load_tracking(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "branch.tracking".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            let Reply::Command(value) = reply else { return };
            let tracking: std::collections::HashMap<String, (u32, u32)> = value
                .get("branches")
                .and_then(|branches| branches.as_array())
                .map(|branches| {
                    branches
                        .iter()
                        .filter_map(|branch| {
                            let count = |key: &str| {
                                branch
                                    .get(key)
                                    .and_then(|n| n.as_u64())
                                    .and_then(|n| u32::try_from(n).ok())
                            };
                            Some((
                                branch.get("name")?.as_str()?.to_owned(),
                                (count("ahead")?, count("behind")?),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            // The status bar says it for the branch that is checked out.
            let head = this
                .snapshot
                .as_ref()
                .filter(|snapshot| !snapshot.head.detached)
                .and_then(|snapshot| snapshot.head.branch.clone());
            let ahead_behind = head.and_then(|name| tracking.get(&name).copied());
            if this.ahead_behind != ahead_behind {
                this.ahead_behind = ahead_behind;
                cx.notify();
            }
            this.refs_view
                .update(cx, |view, cx| view.set_tracking(tracking, cx));
        });
    }

    /// Ask how gitflow is set up, whether the checkout is sparse, where LFS
    /// stands and what there is to say about the local branches; and, where
    /// the repository has another worktree or a submodule, how much is
    /// changed in each worktree and where each submodule stands.
    fn load_places(&mut self, cx: &mut Context<Self>) {
        // A repository in the middle of a rebase reloads many times a
        // second. (The other worktrees have a pace of their own, see
        // `looking.rs`.)
        const EVERY: std::time::Duration = std::time::Duration::from_secs(15);
        if self.places_loaded.is_some_and(|at| at.elapsed() < EVERY) {
            return;
        }
        self.places_loaded = Some(std::time::Instant::now());
        let ask = |id: &str| Request::Command {
            id: id.to_owned(),
            params: json!({}),
        };
        self.session
            .call(ask("flow.status"), cx, |this, reply, cx| {
                if let Reply::Command(value) = reply {
                    this.flow_state = FlowState::from_json(&value);
                    let flow = this.flow_state.clone();
                    this.refs_view
                        .update(cx, |view, cx| view.set_flow(flow, cx));
                }
            });
        // Whether only part of the files is checked out.
        self.load_sparse(cx);
        self.session.call(ask("lfs.status"), cx, |this, reply, cx| {
            if let Reply::Command(value) = reply {
                let status = LfsStatus::from_json(&value);
                if this.lfs_status != status {
                    this.lfs_status = status;
                    cx.notify();
                }
            }
        });
        // Stale by the clock the panel's ages are told by.
        let overview = Request::Command {
            id: "branch.overview".to_owned(),
            params: json!({
                "stale_days": cx.global::<Prefs>().preferences.stale_days,
                "now": crate::views::now(),
            }),
        };
        self.session.call(overview, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply {
                let notes = flow::notes_from(&value);
                this.refs_view
                    .update(cx, |view, cx| view.set_notes(notes, cx));
            }
        });
        let Some((worktrees, submodules)) = self
            .snapshot
            .as_ref()
            .map(|snapshot| (snapshot.worktrees.len(), snapshot.submodules.len()))
        else {
            return;
        };
        // What is uncommitted in the other worktrees, for their rows in
        // the graph and their counts in the panel. A `git status` in each
        // takes seconds where there are many of them, so a worktree is
        // looked at again only when its HEAD or its index is not what it
        // was (which costs a `stat`) or its section is open, and at most
        // once a minute (`looking.rs`); all of them when the user
        // refreshes; and none while the repository is being opened: the
        // first time waits a moment, and what was known the last time is
        // shown until then.
        if worktrees > 1 {
            let afresh = std::mem::take(&mut self.worktrees_afresh);
            if std::mem::replace(&mut self.worktrees_asked, true) {
                self.want_worktree_looks(afresh, cx);
            } else {
                cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(std::time::Duration::from_secs(2))
                        .await;
                    let _ = this.update(cx, |this, cx| this.want_worktree_looks(afresh, cx));
                })
                .detach();
            }
        }
        if submodules > 0 {
            self.submodules_changed(cx);
        }
    }

    /// Where the submodules stand may have changed: read it again, now or
    /// when the last read is far enough back, unless a read that will see
    /// the change is on its way.
    fn submodules_changed(&mut self, cx: &mut Context<Self>) {
        let next = self.submodule_reads.asked(std::time::Instant::now());
        self.follow_submodules(next, cx);
    }

    fn follow_submodules(&mut self, next: crate::views::submodules::Next, cx: &mut Context<Self>) {
        use crate::views::submodules::Next;
        match next {
            Next::Nothing => {}
            Next::Now => self.load_submodules(cx),
            Next::After(wait) => {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |this, cx| this.load_submodules(cx));
                })
                .detach();
            }
        }
    }

    /// Ask where each submodule stands. A read on a thread of its own that
    /// writes nothing under `.git` (git is run with optional locks off),
    /// so the watcher does not see it and nothing loops.
    fn load_submodules(&mut self, cx: &mut Context<Self>) {
        self.submodule_reads.started(std::time::Instant::now());
        let request = Request::Command {
            id: "submodule.list".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply {
                let states: std::collections::HashMap<String, (String, bool, bool)> = value
                    .as_array()
                    .map(|submodules| {
                        submodules
                            .iter()
                            .filter_map(|submodule| {
                                let flag =
                                    |key: &str| submodule.get(key).and_then(|flag| flag.as_bool());
                                Some((
                                    submodule.get("path")?.as_str()?.to_owned(),
                                    (
                                        submodule.get("state")?.as_str()?.to_owned(),
                                        flag("modified")?,
                                        flag("untracked")?,
                                    ),
                                ))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                this.refs_view
                    .update(cx, |view, cx| view.set_submodule_states(states, cx));
            }
            // What changed while it ran is read next.
            let next = this.submodule_reads.ended(std::time::Instant::now());
            this.follow_submodules(next, cx);
        });
    }

    /// Give the views what is known about the other worktrees.
    fn show_worktrees_seen(&mut self, seen: &[crate::prefs::SeenWorktree], cx: &mut Context<Self>) {
        let totals: std::collections::HashMap<String, usize> = seen
            .iter()
            .map(|seen| (seen.path.clone(), seen.counts.total()))
            .collect();
        let counts: std::collections::HashMap<String, bitbraid_git::wip::Counts> = seen
            .iter()
            .map(|seen| (seen.path.clone(), seen.counts))
            .collect();
        self.refs_view
            .update(cx, |view, cx| view.set_worktree_changes(totals, cx));
        self.graph_view
            .update(cx, |view, cx| view.set_worktree_changes(counts, cx));
        cx.notify();
    }

    fn load_status(&mut self, cx: &mut Context<Self>) {
        self.session.call(Request::Status, cx, Self::on_status);
        self.load_in_progress(cx);
    }

    /// Ask what operation is unfinished, with its conflicts. It travels
    /// with the status: whatever changes one can change the other.
    fn load_in_progress(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "op.status".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            let Reply::Command(value) = reply else { return };
            let operation: Option<InProgress> = value
                .get("operation")
                .cloned()
                .and_then(|operation| serde_json::from_value(operation).ok());
            if this.in_progress != operation {
                let part_of = this.finishing_said();
                let names = operation
                    .as_ref()
                    .zip(this.snapshot.as_ref())
                    .map(|(operation, snapshot)| side_names(operation, snapshot));
                this.changes_view.update(cx, |view, cx| {
                    view.set_conflicts(operation.as_ref(), part_of.as_deref(), cx);
                    view.set_side_names(names, cx);
                });
                // An operation that has just stopped on conflicts: bring up
                // the panel they are listed in.
                let began = this.in_progress.is_none()
                    && operation
                        .as_ref()
                        .is_some_and(|operation| !operation.conflicts.is_empty());
                if began {
                    this.right = RightPane::Wip;
                    this.graph_view.update(cx, |view, cx| view.select_wip(cx));
                }
                this.in_progress = operation;
                let conflicts = this
                    .in_progress
                    .as_ref()
                    .map_or(0, |operation| operation.conflicts.len());
                this.details_view
                    .update(cx, |view, cx| view.set_conflicts(conflicts, cx));
                let stopped =
                    this.in_progress
                        .as_ref()
                        .map(|operation| crate::views::commit_box::Stopped {
                            noun: operation.kind.noun().to_owned(),
                            conflicts: operation.conflicts.len(),
                            can_skip: operation.can_skip,
                        });
                this.commit_box
                    .update(cx, |view, cx| view.set_stopped(stopped, cx));
                // The row of uncommitted work says what it waits for.
                let said = this.in_progress.as_ref().map(|operation| {
                    (
                        crate::views::stopped::sentence(operation, part_of.as_deref()),
                        crate::views::stopped::blocked(operation),
                    )
                });
                this.graph_view
                    .update(cx, |view, cx| view.set_stopped(said, cx));
                this.leave_resolved(cx);
                cx.notify();
            }
            // Nothing is in progress and nothing of ours is running: a
            // finish that waited for a merge has nothing to wait for (the
            // merge was concluded or given up some other way). Finishing
            // again from the menu does what is left.
            if this.in_progress.is_none() && this.operation.is_none() {
                this.finishing = None;
            }
            // Without an operation the index may hold conflicts all the
            // same (a patch applied with a three-way merge): they are
            // listed to be resolved too.
            if this.in_progress.is_none() {
                this.load_loose_conflicts(cx);
            }
        });
    }

    /// Ask which files are in conflict while no operation is in progress,
    /// for the panel of changes.
    fn load_loose_conflicts(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "conflict.list".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if this.in_progress.is_some() {
                return;
            }
            let Reply::Command(value) = reply else { return };
            let conflicts: Vec<bitbraid_git::sequence::Conflict> = value
                .get("conflicts")
                .cloned()
                .and_then(|conflicts| serde_json::from_value(conflicts).ok())
                .unwrap_or_default();
            this.changes_view
                .update(cx, |view, cx| view.set_loose_conflicts(conflicts, cx));
        });
    }

    /// The file the merge tool is open on is no longer in conflict (it was
    /// resolved elsewhere, or the operation is over): show what it became.
    fn leave_resolved(&mut self, cx: &mut Context<Self>) {
        if self.file_tab != FileTab::Merge {
            return;
        }
        let Some((path, None)) = self.file.clone() else {
            return;
        };
        let still = self.in_progress.as_ref().is_some_and(|operation| {
            operation
                .conflicts
                .iter()
                .any(|conflict| conflict.path == path)
        });
        if !still {
            self.file_tab = FileTab::Diff;
            let source = DiffSource::Working {
                path,
                side: Side::Staged,
            };
            self.diff_view
                .update(cx, |view, cx| view.show(Some(source), cx));
        }
    }

    /// What the banner of an operation in progress offers, asked for by
    /// key or from the palette: go on (`Some(false)`), skip (`Some(true)`)
    /// or abort (`None`). With nothing in progress, or nothing to skip,
    /// that is said instead.
    pub fn in_progress_asked(
        &mut self,
        step: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(operation) = &self.in_progress else {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("workspace-nothing-in-progress")),
            );
            return;
        };
        match step {
            Some(true) if !operation.can_skip => crate::notices::push(
                window,
                cx,
                Notice::info(say!(
                    "workspace-nothing-to-skip",
                    noun = operation.kind.noun()
                )),
            ),
            Some(false) if !operation.conflicts.is_empty() => crate::notices::push(
                window,
                cx,
                Notice::info(say!(
                    "workspace-still-in-conflict",
                    count = operation.conflicts.len()
                )),
            ),
            Some(skip) => self.step_in_progress(skip, window, cx),
            None => self.abort_in_progress(window, cx),
        }
    }

    /// Go on with the operation in progress, or skip the commit it is on.
    pub fn step_in_progress(&mut self, skip: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(operation) = &self.in_progress else {
            return;
        };
        let noun = operation.kind.noun();
        self.run_operation(
            if skip { "op.skip" } else { "op.continue" },
            json!({}),
            if skip {
                say!("workspace-skipping")
            } else {
                say!("workspace-continuing")
            },
            say!("workspace-operation-finished", noun),
            window,
            cx,
        );
    }

    /// Ask, then give the operation in progress up.
    pub fn abort_in_progress(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(operation) = &self.in_progress else {
            return;
        };
        let noun = operation.kind.noun();
        let workspace = cx.entity().downgrade();
        crate::ask::show(
            crate::ask::Ask::new(say!("workspace-abort-question", noun)).danger(
                say!("workspace-abort"),
                move |_, window, cx| {
                    let _ = workspace.update(cx, |this, cx| {
                        // A finish that stopped in this merge is given up
                        // with it.
                        this.finishing = None;
                        this.run_mutation(
                            "op.abort",
                            json!({}),
                            say!("workspace-aborted", noun),
                            |_, _, _, _| false,
                            window,
                            cx,
                        )
                    });
                },
            ),
            window,
            cx,
        );
    }

    fn on_status(&mut self, reply: Reply, cx: &mut Context<Self>) {
        match reply {
            Reply::Status(entries, more) => {
                self.status_cut = more;
                // Every file once, by what happened to it: the staged side
                // says it when there is one.
                let mut counts = crate::views::graph::WipCounts::default();
                for entry in &entries {
                    match entry.staged.or(entry.unstaged) {
                        Some(
                            ChangeKind::Added | ChangeKind::Untracked | ChangeKind::IntentToAdd,
                        ) => counts.added = counts.added.saturating_add(1),
                        Some(ChangeKind::Deleted) => {
                            counts.deleted = counts.deleted.saturating_add(1)
                        }
                        Some(ChangeKind::Renamed | ChangeKind::Copied) => {
                            counts.renamed = counts.renamed.saturating_add(1)
                        }
                        Some(_) => counts.modified = counts.modified.saturating_add(1),
                        None => {}
                    }
                }
                let first = !self.status_loaded;
                // The worktree this tab shows has a mark of it in the
                // references panel.
                let changed = !entries.is_empty();
                self.refs_view
                    .update(cx, |view, cx| view.set_here_changed(changed, cx));
                self.status = entries.clone();
                self.status_loaded = true;
                self.completion_words_follow(cx);
                // A file the index holds under a new name is compared with
                // what it was under the old one.
                let moves = crate::views::diff::staged_moves(&self.status);
                self.diff_view
                    .update(cx, |view, _| view.set_staged_moves(moves));
                self.restore_file(cx);
                // A repository opens on its uncommitted work when it has
                // any, whatever was selected the last time: unless the file
                // of a commit is brought back in the middle, which goes
                // with that commit.
                let commit_file = self
                    .file_to_restore
                    .as_ref()
                    .is_some_and(|open| open.rev.is_some())
                    || (self.show_diff && self.file.as_ref().is_some_and(|(_, rev)| rev.is_some()));
                if opens_on_uncommitted_work(first, is_dirty(&self.status), commit_file) {
                    // The row is the graph's first: where the graph was
                    // scrolled to the last time would leave it out of
                    // sight.
                    let path = self.session.path.clone();
                    cx.global_mut::<Prefs>()
                        .update_repo_state(&path, |state| state.top = None);
                    self.right = RightPane::Wip;
                    self.graph_view.update(cx, |view, cx| view.select_wip(cx));
                }
                // What the commit button says it commits: the staged list.
                let staged = entries
                    .iter()
                    .filter(|entry| !entry.conflict && entry.staged.is_some())
                    .count();
                self.commit_box
                    .update(cx, |view, cx| view.set_staged(staged, cx));
                let changed = entries.len();
                self.details_view
                    .update(cx, |view, cx| view.set_uncommitted(changed, cx));
                self.changes_view
                    .update(cx, |view, cx| view.set_status(entries, cx));
                self.graph_view
                    .update(cx, |view, cx| view.set_wip(Some(counts), cx));
                self.sync_wip_row(cx);
                self.diff_view.update(cx, |view, cx| view.refresh(cx));
                cx.notify();
            }
            Reply::Error(err) => self.fail(err, cx),
            _ => {}
        }
    }

    fn after_mutation(&mut self, cx: &mut Context<Self>) {
        self.load_status(cx);
        self.peek_undo(cx);
        // What was just done is what the open sheet is about.
        self.refresh_sheet(cx);
    }

    fn peek_undo(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "undo.peek".into(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply {
                this.undo_label = value
                    .get("undo")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                this.redo_label = value
                    .get("redo")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                this.undo_id = value.get("undo_id").and_then(|v| v.as_u64());
                let flag = |key: &str| value.get(key).and_then(|v| v.as_bool()) == Some(true);
                this.undo_remote = flag("undo_remote");
                this.redo_remote = flag("redo_remote");
                cx.notify();
            }
        });
    }

    /// Labels of what undo and redo would do.
    pub fn undo_labels(&self) -> (Option<String>, Option<String>) {
        (self.undo_label.clone(), self.redo_label.clone())
    }

    /// Reverse the last journalled action.
    pub fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_journal("undo.last", "undone", window, cx);
    }

    /// Replay the last undone action.
    pub fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_journal("redo.last", "redone", window, cx);
    }

    fn run_journal(
        &mut self,
        id: &str,
        key: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // What talks to a remote is an operation: it shows in the status
        // bar and holds nothing else up while it pushes.
        let undo = id == "undo.last";
        if (undo && self.undo_remote) || (!undo && self.redo_remote) {
            let reply = self.session.send(Request::Operation {
                id: id.to_owned(),
                params: json!({}),
            });
            self.operation_began(
                if undo {
                    say!("workspace-undoing")
                } else {
                    say!("workspace-redoing")
                },
                Some(if undo { "undo.last" } else { "redo.last" }),
            );
            cx.spawn_in(window, async move |this, cx| {
                let reply = reply.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    this.operation_ended();
                    if let Ok(Reply::Error(err)) = reply {
                        let notice = this.failed_notice(&err, cx);
                        crate::notices::push(window, cx, notice);
                    }
                    this.after_mutation(cx);
                    this.reload(cx);
                });
            })
            .detach();
            cx.notify();
            return;
        }
        let reply = self.session.send(Request::Command {
            id: id.into(),
            params: json!({}),
        });
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match reply {
                    // Said where it is seen: the reload that follows
                    // takes the panel's error away again.
                    Ok(Reply::Error(err)) => {
                        let notice = this.failed_notice(&err, cx);
                        crate::notices::push(window, cx, notice);
                    }
                    Ok(Reply::Command(value)) => {
                        if let Some(label) = value.get(key).and_then(|v| v.as_str()) {
                            this.error = None;
                            // Who may sign is what it was before, or after:
                            // the list and the signatures on show follow.
                            if signers::about_signers(label) {
                                this.signers_changed(cx);
                            }
                            // Back, and the local changes carried with it
                            // did not fit there: they wait in the stash.
                            if value.get("stash_kept").and_then(|kept| kept.as_bool()) == Some(true)
                            {
                                crate::notices::push(
                                    window,
                                    cx,
                                    Notice::warning(if undo {
                                        say!("workspace-undone-stash-kept", label)
                                    } else {
                                        say!("workspace-redone-stash-kept", label)
                                    }),
                                );
                            }
                        }
                    }
                    _ => {}
                }
                this.after_mutation(cx);
                this.reload(cx);
            });
        })
        .detach();
    }

    fn load_graph(&mut self, cx: &mut Context<Self>) {
        let repo_state = cx.global::<Prefs>().repo_state(&self.session.path);
        let order = cx.global::<Prefs>().preferences.order();
        // References shown alone that the snapshot does not know yet fall
        // back to everything; the reload after the references arrive narrows
        // it.
        let request =
            graph_request(&repo_state, self.snapshot.as_ref(), order).unwrap_or(Request::Graph {
                tips: Tips::All,
                exclude: repo_state.hidden.clone(),
                pinned: repo_state.pinned.clone(),
                order,
                worktree_rows: repo_state.worktree_rows(),
            });
        self.graph_loads = self.graph_loads.saturating_add(1);
        self.session.call(request, cx, Self::on_graph);
    }

    fn on_graph(&mut self, reply: Reply, cx: &mut Context<Self>) {
        self.graph_loads = self.graph_loads.saturating_sub(1);
        match reply {
            Reply::Graph(graph) => {
                self.loading = false;
                // More of the same history may have come already, as an
                // event that overtook this answer.
                let graph = match self.graph_ahead.take() {
                    Some(ahead)
                        if ahead.generation == graph.generation && ahead.follows(&graph) =>
                    {
                        ahead
                    }
                    _ => graph,
                };
                // And a late answer with a smaller part of the history
                // that is on show changes nothing.
                if self.graph.as_ref().is_none_or(|shown| graph.follows(shown)) {
                    self.show_graph(graph, cx);
                }
                self.restore_file(cx);
                self.sync_wip_row(cx);
                cx.notify();
            }
            Reply::Error(err) => self.fail(err, cx),
            _ => {}
        }
    }

    /// The details of the rows the first graph shows first, read ahead of
    /// the window: for the graph's rows.
    fn on_first_details(&mut self, reply: Reply, cx: &mut Context<Self>) {
        if let Reply::Details(details) = reply {
            self.graph_view
                .update(cx, |view, cx| view.take_details(details, cx));
        }
    }

    /// Put a graph into the views.
    fn show_graph(&mut self, graph: Arc<GraphData>, cx: &mut Context<Self>) {
        self.graph = Some(graph.clone());
        let remembered = cx.global::<Prefs>().repo_state(&self.session.path).selected;
        let remembered: Option<Oid> = remembered.and_then(|s| s.parse().ok());
        self.refs_view
            .update(cx, |view, cx| view.set_graph(graph.clone(), cx));
        self.graph_view
            .update(cx, |view, cx| view.set_graph(graph, remembered, cx));
    }

    /// More of a large history has been walked: a larger piece of the
    /// graph, or all of it. It is for the views when it belongs to
    /// the graph they show; one that belongs to an answer that is still on
    /// its way waits for it.
    fn on_graph_piece(&mut self, graph: Arc<GraphData>, cx: &mut Context<Self>) {
        match &self.graph {
            Some(shown) if shown.generation == graph.generation => {
                if graph.follows(shown) {
                    self.show_graph(graph, cx);
                    self.restore_file(cx);
                    cx.notify();
                }
            }
            _ if self.graph_loads > 0 => self.graph_ahead = Some(graph),
            _ => {}
        }
    }

    /// The graph carries the row of uncommitted work, which the session lays
    /// out from the last status it computed. Ask for the graph again when it
    /// disagrees with the status shown here, unless one is already on its way.
    fn sync_wip_row(&mut self, cx: &mut Context<Self>) {
        let dirty = is_dirty(&self.status);
        if self.graph_loads == 0
            && self
                .graph
                .as_ref()
                .is_some_and(|graph| graph.wip.is_some() != dirty)
        {
            self.load_graph(cx);
        }
    }

    fn fail(&mut self, err: String, cx: &mut Context<Self>) {
        self.loading = false;
        self.error = Some(err);
        cx.notify();
    }

    fn on_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Progress {
                command,
                stage,
                percent,
            } => {
                self.upkeep_progress(&command, &stage, percent, cx);
                if let Some(operation) = &mut self.operation {
                    operation.stage = Some((stage, percent));
                    cx.notify();
                }
            }
            Event::Busy(label) => self.on_busy(label, cx),
            Event::Graph(graph) => self.on_graph_piece(graph, cx),
            Event::GraphFailed { generation, error } => {
                if self
                    .graph
                    .as_ref()
                    .is_some_and(|shown| shown.generation == generation)
                {
                    self.fail(error, cx);
                }
            }
            Event::Changed(set) => {
                // A submodule's state follows what changes in it.
                let touched = self.snapshot.as_ref().is_some_and(|snapshot| {
                    let paths: Vec<&str> = snapshot
                        .submodules
                        .iter()
                        .map(|submodule| submodule.path.as_str())
                        .collect();
                    crate::views::submodules::touches(&set, &paths)
                });
                if touched {
                    self.submodules_changed(cx);
                }
                if set
                    .areas
                    .iter()
                    .any(|a| matches!(a, Area::Refs | Area::GitDir))
                {
                    self.reload(cx);
                }
                if set
                    .areas
                    .iter()
                    .any(|a| matches!(a, Area::Index | Area::Worktree))
                {
                    self.load_status(cx);
                }
                if set.areas.contains(&Area::OtherWorktree) {
                    self.want_worktree_looks(false, cx);
                }
            }
        }
    }

    fn on_selected(
        &mut self,
        id: Option<Oid>,
        asked: bool,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.right = RightPane::Commit;
        let badges = id
            .and_then(|id| self.graph.as_ref().and_then(|g| g.badges.get(&id).cloned()))
            .unwrap_or_default();
        self.details_view
            .update(cx, |view, cx| view.show(id, badges, asked, cx));
        let path = self.session.path.clone();
        cx.global_mut::<Prefs>()
            .update_repo_state(&path, |state| state.selected = id.map(|id| id.to_hex()));
        cx.notify();
    }

    fn reveal(&mut self, id: Oid, window: &mut Window, cx: &mut Context<Self>) {
        // The graph takes the keys when it selects; while something else
        // has its place (a file's past, whose blame asked) they stay where
        // they were, or Escape and the arrows would go to what is hidden.
        let kept = (self.show_diff || self.shows_past())
            .then(|| window.focused(cx))
            .flatten();
        let found = self
            .graph_view
            .update(cx, |view, cx| view.reveal(id, window, cx));
        if let Some(kept) = kept {
            kept.focus(window, cx);
        }
        if !found {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("workspace-not-in-graph", id = id.short(10))),
            );
        }
    }

    /// Whether the repository is opened read-only (a bare one):
    /// the engine refuses what would change it, and the interface does not
    /// offer it (see [`crate::read_only`]).
    pub fn read_only(&self) -> bool {
        self.session.summary.bare
    }

    /// The repository's name: the name of its folder.
    pub fn repo_name(&self) -> String {
        crate::shell::repo_title(&self.session.path)
    }

    /// What HEAD is on: the branch, or that it is detached.
    pub fn head_label(&self) -> String {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.head.branch.clone())
            .or_else(|| self.session.summary.head_branch.clone())
            .unwrap_or_else(|| {
                if self.session.summary.detached {
                    say!("workspace-head-detached").to_owned()
                } else {
                    say!("workspace-head-unborn").to_owned()
                }
            })
    }

    /// The local branches by name, as the snapshot lists them.
    pub fn local_branch_names(&self) -> Vec<String> {
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .branches
                    .iter()
                    .map(|branch| branch.name.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Give the graph the focus, when it is what the centre shows. False
    /// when something else has the centre.
    pub fn focus_graph(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let shown = self.sheet.is_none() && !self.planning && !self.show_diff;
        if shown {
            self.graph_view.read(cx).focus_handle(cx).focus(window, cx);
        }
        shown
    }

    /// Put the keyboard in the references panel: in its filter, or on its
    /// rows. The panel is opened when it was folded away.
    pub fn focus_references(&mut self, filter: bool, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.session.path.clone();
        if !cx.global::<Prefs>().left_panel_shown(Some(&path)) {
            cx.global_mut::<Prefs>()
                .set_panel_shown(Some(&path), true, true);
            cx.refresh_windows();
        }
        // A file in the middle folds the panel to its strip, and its past,
        // the merge tool and the plan of a rebase take its place: the
        // panel is what was asked for, which closes them
        // (`unfolding.rs`). Edited text is asked about first, and the keys
        // stay where they are meanwhile.
        if self.holds_references() {
            self.unfold_references(window, cx);
            if self.holds_references() {
                return;
            }
        }
        self.refs_view.update(cx, |view, cx| {
            if filter {
                view.focus_filter(window, cx)
            } else {
                view.focus_rows(window, cx)
            }
        });
        cx.notify();
    }

    /// Find a file by its path and show its history: a dialog with a
    /// field and the files that match what is typed.
    pub fn find_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.find_file.update(cx, |view, cx| view.open(window, cx));
        let view = self.find_file.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            dialog
                .title(say!("workspace-find-file-title"))
                .width(px(640.))
                // Enter is the field's: it opens the file the cursor is on,
                // and the dialog is closed then, at once. The dialog's own
                // Enter would close it as well, and give the keys back a
                // quarter of a second later, when its closing is drawn, to
                // what had them before, the graph: by then the file's past
                // stands in its place, and every key would go nowhere.
                .on_ok(|_, _, _| false)
                .child(view.clone())
        });
        // The dialog takes the focus when it opens; hand it to the field.
        cx.defer_in(window, |this, window, cx| {
            this.find_file.update(cx, |view, cx| view.focus(window, cx));
        });
    }

    /// Show only the commits the graph's search found, or all again.
    pub fn toggle_only_matches(&mut self, cx: &mut Context<Self>) {
        self.graph_view
            .update(cx, |graph, cx| graph.toggle_only_matches(cx));
    }

    /// Put the cursor in the graph's search field.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.graph_view
            .update(cx, |graph, cx| graph.focus_search(window, cx));
    }

    /// Text for the status bar.
    pub fn status_line(&self) -> String {
        let branch = self
            .snapshot
            .as_ref()
            .and_then(|s| s.head.branch.clone())
            .or_else(|| self.session.summary.head_branch.clone())
            .unwrap_or_else(|| {
                if self.session.summary.detached {
                    say!("workspace-head-detached").to_owned()
                } else {
                    say!("workspace-head-unborn").to_owned()
                }
            });
        let text = status_text(
            &self.session.path.display().to_string(),
            &branch,
            self.ahead_behind,
            self.status.len(),
            self.status_cut,
        );
        if self.read_only() {
            say!("workspace-status-read-only", text)
        } else {
            text
        }
    }

    /// Commit count once loaded. Rows that stand for the uncommitted work
    /// of other worktrees are no commits.
    pub fn commit_count(&self) -> Option<usize> {
        self.graph
            .as_ref()
            .map(|g| g.commits.len().saturating_sub(g.worktree_rows.len()))
    }

    /// The right pane while the row of another worktree's uncommitted work
    /// is selected: which worktree, what is changed in it, and the way to
    /// where that is staged and committed.
    fn render_worktree_pane(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let path = self.selected_worktree.clone().unwrap_or_default();
        let seen = cx
            .global::<Prefs>()
            .repo_state(&self.session.path)
            .worktrees_seen
            .into_iter()
            .find(|seen| seen.path == path);
        let branch = self.snapshot.as_ref().and_then(|snapshot| {
            snapshot
                .worktrees
                .iter()
                .find(|worktree| {
                    worktree
                        .path
                        .as_ref()
                        .is_some_and(|at| at.display().to_string() == path)
                })
                .map(|worktree| worktree.branch.clone())
        });
        let on = match branch {
            Some(Some(branch)) => say!("workspace-other-worktree-on", branch),
            Some(None) => say!("workspace-other-worktree-detached").to_owned(),
            None => say!("workspace-other-worktree-gone").to_owned(),
        };
        let counts = seen.map(|seen| seen.counts).unwrap_or_default();
        let line =
            |count: usize, said: fn(usize) -> String| (count > 0).then(|| div().child(said(count)));
        let open = path.clone();
        v_flex()
            .size_full()
            .border_l_1()
            .border_color(theme.border)
            .p_3()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(say!("workspace-other-worktree-heading")),
            )
            .child(
                div()
                    .text_sm()
                    .child(crate::views::worktrees::folder_name(&path)),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(path.clone()),
            )
            .child(div().text_xs().text_color(theme.muted_foreground).child(on))
            .child(
                v_flex()
                    .text_sm()
                    .children(line(counts.modified, |count| {
                        say!("workspace-other-worktree-changed", count)
                    }))
                    .children(line(counts.added, |count| {
                        say!("workspace-other-worktree-new", count)
                    }))
                    .children(line(counts.deleted, |count| {
                        say!("workspace-other-worktree-removed", count)
                    }))
                    .children(line(counts.renamed, |count| {
                        say!("workspace-other-worktree-moved", count)
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(say!("workspace-other-worktree-explained")),
            )
            .child(
                h_flex().child(
                    Button::new("open-worktree")
                        .small()
                        .label(say!("workspace-other-worktree-open"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.worktree(WorktreeRequest::Open(open.clone()), window, cx)
                        })),
                ),
            )
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The pictures of its remotes' owners are this repository's.
        crate::providers::drawing(&self.session.summary.git_dir, cx);
        self.focus_docked(window, cx);
        let repo_state = cx.global::<Prefs>().repo_state(&self.session.path);
        let left_w = repo_state.left.unwrap_or(208.);
        let right_w = repo_state.right.unwrap_or(RIGHT_WIDTH);
        let window_w = crate::zoom::unpx(window.viewport_size().width);
        let path = self.session.path.clone();
        let theme = cx.theme().clone();
        let file_tab = self.file_tab;
        // What takes the whole width leaves neither panel; what takes the
        // place of the references and the graph leaves the right one.
        let (no_left, no_right) = self.centre_takes();
        let (left_shown, right_shown) = {
            let prefs = cx.global::<Prefs>();
            (
                prefs.left_panel_shown(Some(&path)),
                prefs.right_panel_shown(Some(&path)),
            )
        };
        // The right panel is there while a row of the graph is selected:
        // with nothing selected the graph has the whole width.
        let right_shown = right_shown && self.something_selected(cx);
        // The order of the graph is a preference: when it is another than
        // the one the graph on show was walked in, the graph is asked for
        // again (a change of preferences draws every window anew).
        let order = cx.global::<Prefs>().preferences.order();
        if self.graph_loads == 0
            && self.error.is_none()
            && self.graph.as_ref().is_some_and(|g| g.order != order)
        {
            self.load_graph(cx);
        }

        // A detached HEAD is said out loud, with the two ways out of it.
        let detached = self
            .snapshot
            .as_ref()
            .filter(|snapshot| snapshot.head.detached)
            .map(|snapshot| (snapshot.head.target, snapshot.head.previous.clone()));
        let detached_banner = detached.map(|(at, previous)| {
            let hex = at.map(|id| id.to_hex());
            h_flex()
                .w_full()
                .px_3()
                .py_1()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.muted)
                .text_sm()
                .child(div().flex_1().min_w_0().truncate().child(match at {
                    Some(id) => say!("workspace-detached-at", id = id.short(7)),
                    None => say!("workspace-detached").to_owned(),
                }))
                .children(previous.map(|branch| {
                    let label = say!("workspace-detached-back", branch = branch.as_str());
                    Button::new("detached-back")
                        .small()
                        .outline()
                        .label(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.checkout(branch.clone(), window, cx)
                        }))
                }))
                .child(
                    Button::new("detached-branch")
                        .small()
                        .primary()
                        .label(say!("workspace-detached-new-branch"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.prompt_new_branch(hex.clone(), window, cx)
                        })),
                )
        });

        // An operation in progress comes first: HEAD is detached during a
        // rebase, and that is not what needs saying then.
        let progress_banner = self.in_progress.as_ref().map(|operation| {
            let conflicts = operation.conflicts.len();
            let blocked = crate::views::stopped::blocked(operation);
            // The merge a gitflow finish stopped in says what it is part of.
            let said = crate::views::stopped::sentence(operation, self.finishing_said().as_deref());
            let mailbox = operation.kind == bitbraid_git::sequence::Kind::Mailbox;
            h_flex()
                .w_full()
                .px_3()
                .py_1()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(theme.border)
                .bg(if blocked {
                    theme.danger.opacity(0.15)
                } else {
                    theme.success.opacity(0.15)
                })
                .text_sm()
                // What to do next is the end of the sentence: it goes on a
                // second line rather than be cut off.
                .child(div().flex_1().min_w_0().whitespace_normal().child(said))
                .child(
                    Button::new("progress-abort")
                        .small()
                        .ghost()
                        .label(say!("workspace-abort"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.abort_in_progress(window, cx)),
                        ),
                )
                .when(operation.can_skip, |banner| {
                    banner.child(
                        Button::new("progress-skip")
                            .small()
                            .outline()
                            .label(if mailbox {
                                say!("workspace-skip-patch")
                            } else {
                                say!("workspace-skip-commit")
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.step_in_progress(true, window, cx)
                            })),
                    )
                })
                .child(
                    Button::new("progress-continue")
                        .small()
                        .primary()
                        .label(say!("workspace-continue"))
                        .disabled(conflicts > 0)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.step_in_progress(false, window, cx)
                        })),
                )
        });
        let detached_banner = detached_banner.filter(|_| progress_banner.is_none());
        // The panel of uncommitted work says the same and has the same
        // buttons under its commit form, the details of a commit say how
        // many files are in conflict over them and lead there, and the
        // merge tool is about one file of it: the banner is for when none
        // is on show.
        let merging =
            self.sheet.is_none() && !self.planning && self.show_diff && file_tab == FileTab::Merge;
        let conflicted = self
            .in_progress
            .as_ref()
            .is_some_and(|operation| !operation.conflicts.is_empty());
        let panel_says = right_shown
            && !no_right
            && (self.right == RightPane::Wip || (self.right == RightPane::Commit && conflicted));
        let progress_banner = progress_banner.filter(|_| !merging && !panel_says);

        // A bare repository is opened read-only, and says so for
        // as long as it is open: the engine refuses what would change it.
        let bare_banner = self.session.summary.bare.then(|| {
            h_flex()
                .id("bare-banner")
                .w_full()
                .px_3()
                .py_1()
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.muted)
                .text_sm()
                .child(
                    div()
                        .font_semibold()
                        .child(crate::read_only::HEADING.text()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child(crate::read_only::EXPLANATION.text()),
                )
        });

        // A repository beyond what is handled well says so, once, until
        // the banner is put away.
        let references = self.snapshot.as_ref().map_or(0, |snapshot| {
            snapshot
                .branches
                .len()
                .saturating_add(snapshot.tags.len())
                .saturating_add(
                    snapshot
                        .remotes
                        .iter()
                        .map(|remote| remote.branches.len())
                        .sum(),
                )
        });
        let commits = self
            .graph
            .as_ref()
            .map(|graph| graph.walked.unwrap_or(graph.commits.len()));
        let notes = if self.large_hidden {
            Vec::new()
        } else {
            large_notes(commits, references, self.status_cut)
        };
        let large_banner = (!notes.is_empty()).then(|| {
            h_flex()
                .id("large-banner")
                .w_full()
                .px_3()
                .py_1()
                .gap_2()
                .items_start()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.muted)
                .text_sm()
                .child(
                    div()
                        .flex_none()
                        .font_semibold()
                        .child(say!("workspace-large-heading")),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .text_color(theme.muted_foreground)
                        .children(notes.into_iter().map(|note| div().child(note))),
                )
                .child(
                    Button::new("large-hide")
                        .small()
                        .ghost()
                        .label(say!("workspace-large-hide"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.large_hidden = true;
                            cx.notify();
                        })),
                )
        });
        // What Windows has to say of it.
        let platform_banner = self.render_platform_banner(cx);

        let graph_pane = v_flex()
            .size_full()
            .child(div().flex_1().min_h_0().child(self.graph_view.clone()))
            .when_some(self.error.clone(), |this, err| {
                this.child(div().p_2().text_sm().text_color(theme.danger).child(err))
            });

        // The activity log is a dialog over the window (`activity_dialog`):
        // what is under it stays as it was.
        let page_sheet = self.sheet.filter(|sheet| *sheet != Sheet::Activity);
        let centre: AnyElement = if let Some(sheet) = page_sheet {
            match sheet {
                Sheet::Activity => div().into_any_element(),
                Sheet::Journal => self.journal_view.clone().into_any_element(),
                Sheet::Rerere => self.rerere_view.clone().into_any_element(),
                Sheet::Flags => self.flags_view.clone().into_any_element(),
                Sheet::Pull => self.pull_page(),
                // The panel stands over the graph's left, the graph beside
                // it.
                Sheet::NewPull => self.new_pull_over(graph_pane.into_any_element()),
                Sheet::Issue => self.issue_page(),
            }
        } else if self.planning {
            self.rebase_editor.clone().into_any_element()
        } else if self.shows_past() {
            // A file's history and its blame: one view, as wide as the
            // window (the panels make room below).
            self.render_past(cx)
        } else if self.show_diff && file_tab == FileTab::Merge {
            // The merge tool is as wide as the window too, under a row of
            // its own that says which file it is about.
            self.merge_tool.clone().into_any_element()
        } else if self.show_diff {
            // A file takes the place of the graph, under two rows.
            self.render_file_pane(cx)
        } else {
            // The terminal's panel stands under the graph only.
            self.with_terminal(graph_pane.into_any_element(), cx)
        };
        let centre = v_flex()
            .size_full()
            .children(bare_banner)
            .children(large_banner)
            .children(progress_banner)
            .children(detached_banner)
            .children(platform_banner)
            .child(div().flex_1().min_h_0().child(centre));

        // The side panels fill their pane and repaint only when they change
        // themselves, so scrolling the graph does not lay them out again.
        let fill = || StyleRefinement::default().size_full();
        let right_pane: AnyElement = match self.right {
            RightPane::Commit => self.details_view.clone().cached(fill()).into_any_element(),
            RightPane::Wip => v_flex()
                .id("wip-pane")
                .relative()
                .size_full()
                // A window too low for the lists at their least and the
                // whole form (a stopped operation's buttons under it, a
                // large zoom) scrolls the two, rather than putting the
                // form's end under the status bar out of reach.
                // A list in it takes the wheel first, and hands it on at
                // its end (`changes::wheel_first`).
                .overflow_y_scroll()
                .test_support()
                .border_l_1()
                .border_color(theme.border)
                // Where the panel ends, for the form's room.
                .child({
                    let room = self.commit_box.read(cx).room();
                    canvas(
                        move |bounds, _, _| {
                            room.set((room.get().0, crate::zoom::unpx(bounds.bottom())))
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full()
                })
                // The form's grip goes no higher than leaves the lists their
                // least, and a form dragged taller gives way first when
                // the window is made smaller.
                .child({
                    let (least, floor) = {
                        let changes = self.changes_view.read(cx);
                        (changes.least_height(2.), changes.least_height(1.))
                    };
                    let room = self.commit_box.read(cx).room();
                    let tightest = self.commit_box.read(cx).tightest();
                    div()
                        .relative()
                        .flex_1()
                        .min_h(px(floor))
                        .child(
                            canvas(
                                move |bounds, _, _| {
                                    let top = crate::zoom::unpx(bounds.top());
                                    room.set((top + least, room.get().1));
                                    tightest.set(top + floor);
                                },
                                |_, _, _, _| {},
                            )
                            .absolute()
                            .size_full(),
                        )
                        .child(self.changes_view.clone().cached(fill()))
                })
                .child(self.commit_box.clone())
                .into_any_element(),
            RightPane::Worktree => self.render_worktree_pane(cx),
        };

        // The side panels keep the width they were given; the middle takes
        // the rest. Folded, or with a file open in the middle, the
        // references panel is a strip; so it is in a window too narrow for
        // it at its least beside the graph at what it keeps and the right
        // panel at its least (200 % in 1280 by 720 is 640 points).
        let narrow =
            left_shown && !no_left && right_shown && !no_right && too_narrow_for_both(window_w);
        if self.refs_view.read(cx).folded_for_width() != narrow {
            let refs = self.refs_view.clone();
            cx.defer(move |cx| refs.update(cx, |view, cx| view.fold_for_width(narrow, cx)));
        }
        let left_open = left_shown && !self.show_diff && !narrow;
        // What the left panel takes of the width, which bounds how wide the
        // right one can be.
        // Open, the left panel leaves the middle its least beside the
        // right panel at its least: a width it was dragged to in a larger
        // window gives way when the window is smaller.
        let right_least = if right_shown && !no_right {
            RIGHT_MIN
        } else {
            0.
        };
        let left_drawn = dragged_left_width(left_w, window_w, right_least);
        let left_taken = if no_left {
            0.
        } else if left_open {
            left_drawn
        } else {
            crate::views::refs::RAIL_WIDTH
        };
        let right_taken = if right_shown && !no_right {
            right_width(right_w, window_w, left_taken)
        } else {
            0.
        };
        let edge = |side: Split| {
            let hover = theme.primary.opacity(0.6);
            div()
                .id(("split", side as usize))
                .absolute()
                .top_0()
                .h_full()
                .w(px(5.))
                .map(|handle| match side {
                    Split::Left => handle.right(px(-2.)),
                    Split::Right => handle.left(px(-2.)),
                })
                .cursor_col_resize()
                .hover(move |style| style.bg(hover))
                .on_drag(side, |_, _, _, cx| cx.new(|_| NoChip))
                .test_support()
        };
        let left = if no_left {
            None
        } else if left_open {
            Some(
                div()
                    .relative()
                    .w(px(left_drawn))
                    .h_full()
                    .flex_none()
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        this.escape_in_panel(event, false, window, cx)
                    }))
                    .child(self.refs_view.clone().cached(fill()))
                    .child(edge(Split::Left)),
            )
        } else {
            Some(
                div()
                    .w(px(crate::views::refs::RAIL_WIDTH))
                    .h_full()
                    .flex_none()
                    .child(self.refs_view.clone()),
            )
        };
        h_flex()
            .id("workspace-panes")
            .size_full()
            // Escape closes the file in the middle, wherever the keys are
            // (it asks first when edited text is not saved).
            .on_action(
                cx.listener(|this, _: &crate::keymap::FocusGraph, window, cx| {
                    if this.escape_closes_file(window, cx) {
                        this.close_file(window, cx);
                    } else if this.escape_closes_list(window, cx) {
                        // The list over the middle is put away.
                    } else if this.shows_past() && this.sheet.is_none() {
                        // A file's past, with the keys elsewhere in the
                        // tab. Keys left with the graph hidden behind it
                        // come to nothing here: a view that is not drawn
                        // is on no key's way.
                        this.escape_past(window, cx);
                    } else {
                        cx.propagate();
                    }
                }),
            )
            .on_drag_move(
                cx.listener(move |_, event: &DragMoveEvent<Split>, window, cx| {
                    let Some(side) = event.dragged_item().downcast_ref::<Split>().copied() else {
                        return;
                    };
                    let x = crate::zoom::unpx(event.event.position.x);
                    let width = crate::zoom::unpx(window.viewport_size().width);
                    let wanted = match side {
                        Split::Left => dragged_left_width(x, width, right_taken),
                        Split::Right => right_width(width - x, width, left_taken),
                    };
                    cx.global_mut::<Prefs>()
                        .update_repo_state(&path, |state| match side {
                            Split::Left => state.left = Some(wanted),
                            Split::Right => state.right = Some(wanted),
                        });
                    cx.notify();
                }),
            )
            .children(left)
            .child(
                // Squeezed by the panels to what it keeps, the middle cuts
                // what is wider than it (the graph's headings) at its edge.
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_hidden()
                    .child(centre)
                    .children(self.render_docked(cx)),
            )
            .when(right_shown && !no_right, |row| {
                row.child(
                    div()
                        .relative()
                        .w(px(right_taken))
                        .h_full()
                        .flex_none()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            this.escape_in_panel(event, true, window, cx)
                        }))
                        .child(right_pane)
                        .child(edge(Split::Right)),
                )
            })
    }
}

/// What the two sides of a stopped operation are called: what comes in
/// and what it comes into, by the names of their branches where they have
/// them (the branch being merged, rebased; the branch it goes into or
/// onto), else by their short ids.
fn side_names(operation: &InProgress, snapshot: &bitbraid_git::Snapshot) -> (String, String) {
    use bitbraid_git::sequence::Kind;
    let named = |id: Option<Oid>| -> Option<String> {
        let id = id?;
        snapshot
            .branches
            .iter()
            .find(|branch| branch.target == id)
            .map(|branch| branch.name.clone())
            .or_else(|| {
                snapshot
                    .remotes
                    .iter()
                    .flat_map(|remote| remote.branches.iter())
                    .find(|branch| branch.target == id)
                    .map(|branch| branch.name.clone())
            })
            .or_else(|| Some(id.short(7)))
    };
    let current = snapshot
        .head
        .branch
        .clone()
        .or_else(|| snapshot.head.target.map(|id| id.short(7)))
        .unwrap_or_else(|| operation.ours.clone());
    match operation.kind {
        Kind::Rebase => (
            operation
                .branch
                .clone()
                .or_else(|| named(operation.original_head))
                .unwrap_or_else(|| operation.theirs.clone()),
            named(operation.onto).unwrap_or_else(|| operation.ours.clone()),
        ),
        Kind::Merge => (
            named(operation.current).unwrap_or_else(|| operation.theirs.clone()),
            current,
        ),
        Kind::CherryPick | Kind::Revert => (
            operation
                .current
                .map(|id| id.short(7))
                .unwrap_or_else(|| operation.theirs.clone()),
            current,
        ),
        Kind::Mailbox => (operation.theirs.clone(), current),
    }
}

/// Whether two things to show in the middle are the same file seen the
/// same way, whatever was loaded of it.
fn same_file(a: &DiffSource, b: &DiffSource) -> bool {
    match (a, b) {
        (
            DiffSource::Working { path, side },
            DiffSource::Working {
                path: other,
                side: other_side,
            },
        ) => path == other && side == other_side,
        (
            DiffSource::Commit { id, path, .. },
            DiffSource::Commit {
                id: other_id,
                path: other,
                ..
            },
        ) => id == other_id && path == other,
        (
            DiffSource::Range { old, new, path, .. },
            DiffSource::Range {
                old: other_old,
                new: other_new,
                path: other,
                ..
            },
        ) => old == other_old && new == other_new && path == other,
        (
            DiffSource::AgainstWorktree { old, path, .. },
            DiffSource::AgainstWorktree {
                old: other_old,
                path: other,
                ..
            },
        ) => old == other_old && path == other,
        _ => a == b,
    }
}

/// A remote branch's short name parted into the remote and the branch
/// there. Remotes can have slashes in their names, so the longest name
/// that fits decides.
fn split_remote(remotes: &[String], name: &str) -> Option<(String, String)> {
    remotes
        .iter()
        .filter_map(|remote| {
            let branch = name.strip_prefix(remote.as_str())?.strip_prefix('/')?;
            (!branch.is_empty()).then(|| (remote.clone(), branch.to_owned()))
        })
        .max_by_key(|(remote, _)| remote.len())
}

/// The paths of a status that have something staged, and the ones that
/// have something that is not: what "all" means for the row of uncommitted
/// work. A file with both is in both.
fn wip_paths(status: &[StatusEntry]) -> (Vec<String>, Vec<String>) {
    let paths = |side: fn(&StatusEntry) -> bool| -> Vec<String> {
        status
            .iter()
            .filter(|entry| side(entry))
            .map(|entry| entry.path.clone())
            .collect()
    };
    (
        paths(|entry| entry.staged.is_some()),
        paths(|entry| entry.unstaged.is_some()),
    )
}

/// The journal entry a command made: what undo would take after it, when
/// that is not what undo would have taken before it.
fn made_entry(before: Option<u64>, after: Option<u64>) -> Option<u64> {
    after.filter(|id| Some(*id) != before)
}

/// Whether git said that a push was refused because the remote is ahead.
fn push_rejected(err: &str) -> bool {
    err.contains("[rejected]") || err.contains("fetch first") || err.contains("non-fast-forward")
}

/// The status bar's words about a repository: where it is, the branch, how
/// far that is ahead of and behind its upstream, how many files are changed.
fn status_text(
    path: &str,
    branch: &str,
    ahead_behind: Option<(u32, u32)>,
    changes: usize,
    cut: bool,
) -> String {
    let mut text = format!("{path}  ·  {branch}");
    match ahead_behind {
        Some((ahead, 0)) if ahead > 0 => text.push_str(&format!(" ↑{ahead}")),
        Some((0, behind)) if behind > 0 => text.push_str(&format!(" ↓{behind}")),
        Some((ahead, behind)) if ahead > 0 => text.push_str(&format!(" ↑{ahead} ↓{behind}")),
        _ => {}
    }
    if changes > 0 {
        // A status that was cut holds the first of more.
        let changed = if cut {
            say!("workspace-status-changed-or-more", changes)
        } else {
            say!("workspace-status-changed", changes)
        };
        text.push_str(&format!("  ·  {changed}"));
    }
    text
}

/// More commits than the graph is measured on (up to a million): beyond
/// this a repository is said to be large.
pub const MANY_COMMITS: usize = 1_500_000;
/// More references than the panel and the labels are measured on (4,400).
pub const MANY_REFERENCES: usize = 20_000;

/// What there is to say about a repository that is beyond what is handled
/// well: one sentence for each thing that is, none when it is within
/// what is measured.
fn large_notes(commits: Option<usize>, references: usize, status_cut: bool) -> Vec<String> {
    let mut notes = Vec::new();
    if let Some(commits) = commits.filter(|commits| *commits > MANY_COMMITS) {
        notes.push(say!("workspace-large-commits", commits));
    }
    if references > MANY_REFERENCES {
        notes.push(say!("workspace-large-references", references));
    }
    if status_cut {
        notes.push(say!(
            "workspace-large-status",
            limit = bitbraid_git::STATUS_LIMIT
        ));
    }
    notes
}

/// Whether git said that it does not clone from a local path.
fn refuses_local_clone(err: &str) -> bool {
    err.contains("transport 'file' not allowed")
}

/// How an operation ended, for what may follow it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ended {
    /// It did what it was for.
    Done,
    /// It stopped on something to resolve.
    Stopped,
    /// It failed, or was cancelled.
    Failed,
}

/// What becomes of a gitflow finish that waits for its merge, when a
/// command has ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Waiting {
    /// It goes on waiting.
    Keep,
    /// There is nothing to wait for any more.
    Forget,
    /// The merge is concluded: the finish is run again for what is left.
    GoOn,
}

/// What a command that ended means for a gitflow finish that stopped in a
/// merge. The finish itself stopping keeps it waiting; the finish ending
/// any other way (done, refused, cancelled) is the end of it; the merge
/// continued to its end is where it goes on. A continue that stops again
/// (more conflicts) or fails changes nothing, and neither does any other
/// command.
fn finish_after(command: &str, ended: Ended) -> Waiting {
    match (command, ended) {
        ("flow.finish", Ended::Stopped) => Waiting::Keep,
        ("flow.finish", _) => Waiting::Forget,
        ("op.continue", Ended::Done) => Waiting::GoOn,
        _ => Waiting::Keep,
    }
}

/// When a command's result says the operation is in progress rather than
/// done: how many files are in conflict.
fn stopped(value: &serde_json::Value) -> Option<usize> {
    let began = value.get("stopped").is_some_and(|v| v.is_string());
    let went_on = value.get("finished").and_then(|v| v.as_bool()) == Some(false);
    if !began && !went_on {
        return None;
    }
    let listed = |value: &serde_json::Value| value.as_array().map(Vec::len);
    Some(
        value
            .get("conflicts")
            .and_then(listed)
            .or_else(|| {
                value
                    .get("operation")
                    .and_then(|operation| operation.get("conflicts"))
                    .and_then(listed)
            })
            .unwrap_or(0),
    )
}

/// Whether a command that went well says nothing: what the toolbar's
/// buttons do and shows for itself (the graph, the stash in the panel).
fn quiet_when_done(command: &str) -> bool {
    matches!(command, "remote.fetch" | "stash.push" | "stash.pop")
}

/// What the Pull button does when it fetches instead of pulling.
pub const FETCH_MODE: &str = "fetch";

/// What the toolbar's Pull button can be made to do, in the order its
/// caret lists them: fetching, then the ways to pull.
pub const PULL_BUTTON: [(&str, bitbraid_strings::Message); 5] = [
    (FETCH_MODE, message!("workspace-pull-button-fetch")),
    ("ff", message!("workspace-pull-button-ff")),
    ("ff_only", message!("workspace-pull-button-ff-only")),
    ("rebase", message!("workspace-pull-button-rebase")),
    ("merge", message!("workspace-pull-button-merge")),
];

/// The four ways to pull, as the engine names them and as menus do.
pub const PULL_MODES: [(&str, bitbraid_strings::Message); 4] = [
    ("ff_only", message!("workspace-pull-mode-ff-only")),
    ("ff", message!("workspace-pull-mode-ff")),
    ("rebase", message!("workspace-pull-mode-rebase")),
    ("merge", message!("workspace-pull-mode-merge")),
];

/// An error as a notification shows it: starting with a capital.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use bitbraid_git::{Oid, Order, Snapshot, Tips};

    use super::graph_request;
    use crate::{prefs::RepoState, session::Request};

    fn oid(byte: u8) -> Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    #[test]
    fn a_narrow_window_folds_the_references_panel_to_keep_the_graph() {
        use super::too_narrow_for_both;
        // 1280 at 100 %, 150 % (853 points) and 160 % (800): room for all
        // three.
        assert!(!too_narrow_for_both(1280.));
        assert!(!too_narrow_for_both(853.3));
        assert!(!too_narrow_for_both(800.));
        assert!(!too_narrow_for_both(761.));
        // At 175 % and 200 % the graph would keep 170 and 79.
        assert!(too_narrow_for_both(760.9));
        assert!(too_narrow_for_both(731.4));
        assert!(too_narrow_for_both(640.));
        assert!(!too_narrow_for_both(f32::NAN));
    }

    #[test]
    fn the_right_panel_is_as_wide_as_asked_between_its_least_and_what_the_middle_leaves() {
        use super::{RIGHT_WIDTH, right_width};
        // Undragged it is the same in a small window and in a large one.
        assert_eq!(right_width(RIGHT_WIDTH, 1280., 208.), 400.);
        assert_eq!(right_width(RIGHT_WIDTH, 1920., 208.), 400.);
        // Dragged narrow it stops at its least.
        assert_eq!(right_width(10., 1280., 208.), 353.);
        assert_eq!(right_width(-50., 1920., 208.), 353.);
        // Dragged wide it stops at the window's width less 687, whatever
        // the left panel takes: at its least, folded to its strip, wider.
        assert_eq!(right_width(5000., 1280., 208.), 593.);
        assert_eq!(right_width(5000., 1920., 208.), 1233.);
        assert_eq!(right_width(5000., 1280., 36.), 593.);
        assert_eq!(right_width(5000., 1920., 36.), 1233.);
        assert_eq!(right_width(5000., 1920., 400.), 1233.);
        // Beside a left panel wide enough it leaves the middle what it
        // keeps: a width that was fine before the left panel grew gives way.
        assert_eq!(right_width(5000., 1920., 793.), 1040.);
        assert_eq!(right_width(593., 1280., 793.), 400.);
        // Where there is not room for both, it keeps its least.
        assert_eq!(right_width(400., 1000., 793.), 353.);
        assert_eq!(right_width(400., 300., 208.), 353.);
        // Nothing that is no number gets through.
        assert_eq!(right_width(f32::NAN, 1280., 208.), 400.);
        assert_eq!(right_width(f32::INFINITY, 1280., 208.), 400.);
        assert_eq!(right_width(400., f32::NAN, 208.), 353.);
    }

    #[test]
    fn the_two_panels_cannot_be_dragged_until_the_middle_is_gone() {
        use super::{dragged_left_width, right_width};
        // The left one at its widest beside the right one as it starts
        // out, or at its least, at 1280 and at 1920: its own limit holds,
        // and so does the right one's after it.
        for (window, right) in [(1280., 400.), (1280., 353.), (1920., 400.), (1920., 353.)] {
            assert_eq!(dragged_left_width(5000., window, right), 793.);
        }
        let left = dragged_left_width(5000., 1280., 400.);
        assert_eq!(right_width(5000., 1280., left), 400.);
        assert_eq!(1280. - left - 400., 87.);
        // The right one at its widest first: the left one stops where the
        // middle has what it keeps.
        let right = right_width(5000., 1280., 208.);
        assert_eq!(dragged_left_width(5000., 1280., right), 600.);
        assert_eq!(1280. - 600. - right, 87.);
        let right = right_width(5000., 1920., 208.);
        assert_eq!(dragged_left_width(5000., 1920., right), 600.);
        // Without a right panel its own limit holds; narrow, its least.
        assert_eq!(dragged_left_width(5000., 1280., 0.), 793.);
        assert_eq!(dragged_left_width(10., 1920., 400.), 208.);
        // No room at all, or no number: its least.
        assert_eq!(dragged_left_width(500., 400., 353.), 208.);
        assert_eq!(dragged_left_width(f32::NAN, 1280., 400.), 208.);
        // Dragged wide in a large window, then drawn in a smaller one: it
        // stays at 1280, gives way in a window smaller still beside a right
        // panel at its least, and the middle keeps what it keeps.
        assert_eq!(dragged_left_width(793., 1280., 353.), 793.);
        let left = dragged_left_width(793., 1000., 353.);
        assert_eq!(left, 560.);
        assert_eq!(right_width(641., 1000., left), 353.);
        assert_eq!(1000. - left - 353., 87.);
        assert_eq!(dragged_left_width(500., f32::NAN, 400.), 208.);
    }

    #[test]
    fn a_repository_opens_on_its_uncommitted_work_once() {
        use super::opens_on_uncommitted_work as opens;
        assert!(opens(true, true, false));
        // A clean one does not.
        assert!(!opens(true, false, false));
        // The file of a commit that is brought back keeps its commit.
        assert!(!opens(true, true, true));
        // What changes later moves no selection.
        assert!(!opens(false, true, false));
    }

    #[test]
    fn a_command_is_known_to_be_journalled_by_what_undo_takes_next() {
        use super::made_entry;
        assert_eq!(made_entry(None, Some(1)), Some(1), "the first entry");
        assert_eq!(made_entry(Some(4), Some(5)), Some(5));
        assert_eq!(made_entry(Some(5), Some(5)), None, "nothing was recorded");
        assert_eq!(made_entry(Some(5), None), None);
        assert_eq!(made_entry(None, None), None);
    }

    #[test]
    fn a_remote_branch_is_parted_into_its_remote_and_its_name() {
        use super::split_remote;
        let remotes = [
            "origin".to_owned(),
            "origin/fork".to_owned(),
            "up".to_owned(),
        ];
        let parted = |name: &str| split_remote(&remotes, name);
        assert_eq!(
            parted("origin/team/topic"),
            Some(("origin".to_owned(), "team/topic".to_owned()))
        );
        // The longest remote that fits.
        assert_eq!(
            parted("origin/fork/main"),
            Some(("origin/fork".to_owned(), "main".to_owned()))
        );
        assert_eq!(parted("up/x"), Some(("up".to_owned(), "x".to_owned())));
        for none in ["", "origin", "origin/", "upstream/x", "nowhere/x", "/x"] {
            assert_eq!(parted(none), None, "{none:?}");
        }
    }

    #[test]
    fn a_finish_that_stopped_goes_on_when_its_merge_is_concluded() {
        use super::{Ended, Waiting, finish_after};
        // The finish stopped in a merge: it waits.
        assert_eq!(finish_after("flow.finish", Ended::Stopped), Waiting::Keep);
        // The merge was continued to its end: the rest of the finish.
        assert_eq!(finish_after("op.continue", Ended::Done), Waiting::GoOn);
        // Continued into more conflicts, or not at all: still waiting.
        assert_eq!(finish_after("op.continue", Ended::Stopped), Waiting::Keep);
        assert_eq!(finish_after("op.continue", Ended::Failed), Waiting::Keep);
        // The finish ended, whichever way: nothing waits.
        assert_eq!(finish_after("flow.finish", Ended::Done), Waiting::Forget);
        assert_eq!(finish_after("flow.finish", Ended::Failed), Waiting::Forget);
        // Nothing else has a say: a fetch that ends while the merge is
        // being resolved does not set the finish off.
        for command in ["remote.fetch", "op.skip", "commit.create", "flow.start", ""] {
            for ended in [Ended::Done, Ended::Stopped, Ended::Failed] {
                assert_eq!(finish_after(command, ended), Waiting::Keep, "{command}");
            }
        }
    }

    #[test]
    fn a_rejected_push_is_told_from_other_failures() {
        use super::push_rejected;
        assert!(push_rejected(
            "git push failed (exit status: 1): ! [rejected] main -> main (fetch first)"
        ));
        assert!(push_rejected(
            " ! [rejected]        main -> main (non-fast-forward)"
        ));
        assert!(!push_rejected("fatal: could not read Username"));
        assert!(!push_rejected(""));
    }

    #[test]
    fn the_status_bar_says_where_the_branch_stands() {
        use super::status_text;
        assert_eq!(status_text("/r", "main", None, 0, false), "/r  ·  main");
        assert_eq!(
            status_text("/r", "main", Some((0, 0)), 0, false),
            "/r  ·  main"
        );
        assert_eq!(
            status_text("/r", "main", Some((2, 0)), 0, false),
            "/r  ·  main ↑2"
        );
        assert_eq!(
            status_text("/r", "main", Some((0, 3)), 0, false),
            "/r  ·  main ↓3"
        );
        assert_eq!(
            status_text("/r", "main", None, 20_000, true),
            "/r  ·  main  ·  20000 or more changed"
        );
        assert_eq!(
            status_text("/r", "main", Some((2, 3)), 4, false),
            "/r  ·  main ↑2 ↓3  ·  4 changed"
        );
        assert_eq!(
            status_text("/r", "main", None, 1, false),
            "/r  ·  main  ·  1 changed"
        );
    }

    #[test]
    fn all_of_the_uncommitted_work_is_what_the_status_lists() {
        use bitbraid_git::{ChangeKind, StatusEntry};

        use super::wip_paths;
        let entry =
            |path: &str, staged: Option<ChangeKind>, unstaged: Option<ChangeKind>| StatusEntry {
                path: path.to_owned(),
                original_path: None,
                staged,
                unstaged,
                conflict: false,
            };
        let status = [
            entry("staged.txt", Some(ChangeKind::Modified), None),
            entry(
                "both.txt",
                Some(ChangeKind::Added),
                Some(ChangeKind::Modified),
            ),
            entry("changed.txt", None, Some(ChangeKind::Modified)),
            entry("new.txt", None, Some(ChangeKind::Untracked)),
            entry("neither.txt", None, None),
        ];
        let (staged, unstaged) = wip_paths(&status);
        assert_eq!(staged, ["staged.txt", "both.txt"]);
        assert_eq!(unstaged, ["both.txt", "changed.txt", "new.txt"]);
        assert_eq!(wip_paths(&[]), (Vec::new(), Vec::new()));
    }

    #[test]
    fn a_repository_beyond_what_is_measured_says_so() {
        use super::{MANY_COMMITS, MANY_REFERENCES, large_notes};
        // Within what is measured, and while nothing is known: nothing.
        assert!(large_notes(None, 0, false).is_empty());
        assert!(large_notes(Some(1_000_000), 4_400, false).is_empty());
        assert!(large_notes(Some(MANY_COMMITS), MANY_REFERENCES, false).is_empty());
        // One sentence for each thing that is beyond it.
        let commits = large_notes(Some(MANY_COMMITS + 1), 0, false);
        assert_eq!(commits.len(), 1);
        assert!(commits[0].starts_with("1500001 commits"));
        let all = large_notes(Some(usize::MAX), usize::MAX, true);
        assert_eq!(all.len(), 3);
        assert!(all[1].contains("references"));
        assert!(all[2].contains("20000"));
        assert_eq!(large_notes(None, 0, true).len(), 1);
    }

    #[test]
    fn what_is_known_about_the_other_worktrees_follows_what_was_asked() {
        use bitbraid_git::wip::Counts;
        use serde_json::json;

        use super::looking::{Looks, merged, rows_of};
        use crate::prefs::SeenWorktree;
        let worktrees_seen = |value: &serde_json::Value, before: &[SeenWorktree]| {
            merged(value, before, &Looks::default()).0
        };
        let seen = |path: &str, stamp: &str, modified: usize| SeenWorktree {
            path: path.to_owned(),
            stamp: stamp.to_owned(),
            counts: Counts {
                modified,
                ..Counts::default()
            },
        };
        let before = vec![
            seen("/w/a", "s1", 2),
            seen("/w/b", "s1", 0),
            seen("/w/gone", "s1", 5),
        ];
        let answer = json!([
            // Looked at: what was counted.
            { "path": "/w/a", "stamp": "s2", "scanned": true,
              "counts": { "modified": 0, "added": 1, "deleted": 0, "renamed": 0 } },
            // Passed over: what it had.
            { "path": "/w/b", "stamp": "s1", "scanned": false, "counts": null },
            // New, passed over although nothing is known: nothing.
            { "path": "/w/c", "stamp": "s1", "scanned": false, "counts": null },
            // Looked at and git could not say: no stamp, to be asked again.
            { "path": "/w/d", "stamp": "s9", "scanned": true, "counts": null },
            // Not an entry.
            { "path": 7 },
            "noise",
        ]);
        let now = worktrees_seen(&answer, &before);
        let added = SeenWorktree {
            path: "/w/a".to_owned(),
            stamp: "s2".to_owned(),
            counts: Counts {
                added: 1,
                ..Counts::default()
            },
        };
        assert_eq!(
            now,
            [
                added,
                seen("/w/b", "s1", 0),
                seen("/w/c", "s1", 0),
                seen("/w/d", "", 0)
            ]
        );
        // A row for each worktree that has changes, in the order listed.
        assert_eq!(rows_of(&before), ["/w/a", "/w/gone"]);
        assert_eq!(rows_of(&now), ["/w/a"]);
        // An answer that is no list, or a very long one: nothing, or as
        // many as are ever looked at.
        assert!(worktrees_seen(&json!({ "path": "/w/a" }), &before).is_empty());
        let many: Vec<serde_json::Value> = (0..500)
            .map(|n| json!({ "path": format!("/w/{n}"), "stamp": "s", "scanned": false }))
            .collect();
        assert_eq!(
            worktrees_seen(&json!(many), &[]).len(),
            bitbraid_git::wip::MAX_SCANNED
        );
    }

    #[test]
    fn without_solo_the_graph_request_needs_no_references() {
        use crate::prefs::SeenWorktree;
        let seen = |path: &str, modified: usize| SeenWorktree {
            path: path.to_owned(),
            stamp: "s".to_owned(),
            counts: bitbraid_git::wip::Counts {
                modified,
                ..Default::default()
            },
        };
        let state = RepoState {
            hidden: vec!["refs/heads/old".to_owned()],
            pinned: vec!["refs/heads/main".to_owned()],
            worktrees_seen: vec![seen("/w/clean", 0), seen("/w/dirty", 3)],
            ..RepoState::default()
        };
        let Some(Request::Graph {
            tips,
            exclude,
            pinned,
            order,
            worktree_rows,
        }) = graph_request(&state, None, Order::Topo)
        else {
            panic!("a graph request");
        };
        assert_eq!(tips, Tips::All);
        assert_eq!(exclude, ["refs/heads/old"]);
        assert_eq!(pinned, ["refs/heads/main"]);
        // The order that is preferred, and a row for each worktree that
        // had changes when it was last looked at.
        assert_eq!(order, Order::Topo);
        assert_eq!(worktree_rows, ["/w/dirty"]);
    }

    #[test]
    fn soloed_references_wait_for_the_snapshot() {
        let state = RepoState {
            only: vec!["refs/heads/topic".to_owned()],
            ..RepoState::default()
        };
        assert!(graph_request(&state, None, Order::Date).is_none());

        let mut snapshot = Snapshot::default();
        snapshot.branches.push(bitbraid_git::snapshot::Branch {
            name: "topic".to_owned(),
            full_name: "refs/heads/topic".to_owned(),
            target: oid(7),
            upstream: None,
            remote: None,
            is_head: false,
        });
        let Some(Request::Graph { tips, .. }) = graph_request(&state, Some(&snapshot), Order::Date)
        else {
            panic!("a graph request");
        };
        assert_eq!(tips, Tips::Ids(vec![oid(7)]));
        // What is pinned is asked for with what is shown alone.
        let pinning = RepoState {
            pinned: vec!["refs/heads/main".to_owned()],
            ..state.clone()
        };
        let Some(Request::Graph { pinned, .. }) =
            graph_request(&pinning, Some(&snapshot), Order::Topo)
        else {
            panic!("a graph request");
        };
        assert_eq!(pinned, ["refs/heads/main"]);

        // A remote shown alone as a whole stands for every branch it has.
        snapshot.remotes.push(bitbraid_git::snapshot::Remote {
            name: "origin".to_owned(),
            fetch_url: None,
            push_url: None,
            head: None,
            branches: [(8, "a"), (9, "team/b")]
                .into_iter()
                .map(|(byte, name)| bitbraid_git::snapshot::RemoteBranch {
                    name: name.to_owned(),
                    full_name: format!("refs/remotes/origin/{name}"),
                    target: oid(byte),
                })
                .collect(),
        });
        let whole = RepoState {
            only: vec!["refs/remotes/origin".to_owned()],
            ..RepoState::default()
        };
        let Some(Request::Graph { tips, .. }) = graph_request(&whole, Some(&snapshot), Order::Date)
        else {
            panic!("a graph request");
        };
        assert_eq!(tips, Tips::Ids(vec![oid(8), oid(9)]));

        // A reference shown alone that no longer exists shows everything.
        let Some(Request::Graph { tips, .. }) =
            graph_request(&state, Some(&Snapshot::default()), Order::Date)
        else {
            panic!("a graph request");
        };
        assert_eq!(tips, Tips::All);
    }
}
