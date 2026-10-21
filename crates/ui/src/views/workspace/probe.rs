//! What a repository's tab has on show, as plain facts, for the off-screen
//! tests (see `shell/probe.rs`). Nothing here changes anything.

use gpui_kit::{App, Entity};

use super::{FileTab, RightPane, Sheet, Workspace};
use crate::views::{changes::ChangesView, commit_box::CommitBox, graph::GraphView};

/// What a repository's tab has on show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProbe {
    /// Whether the references, the graph and the first status are in.
    pub loaded: bool,
    /// What went wrong, when the tab says so in place of its content.
    pub error: Option<String>,
    /// The view that has the centre in place of the graph: `activity`,
    /// `undo_history`, `index_flags`, `rerere`, `pull`, `new_pull` or
    /// `issue`.
    pub sheet: Option<&'static str>,
    /// Whether the interactive rebase is being planned in the centre.
    pub planning: bool,
    /// The file in the centre: its path and the commit it is from.
    pub file: Option<(String, Option<String>)>,
    /// What is shown of it: `diff`, `history`, `blame`, `edit` or `merge`.
    pub file_tab: &'static str,
    /// Whether the right pane is the changes and the commit box (else the
    /// details of a commit).
    pub changes_shown: bool,
    /// How many paths the last status listed.
    pub status_paths: usize,
    /// The operation that is unfinished (`merge`, `rebase`, …), and how
    /// many paths are in conflict in it.
    pub in_progress: Option<(String, usize)>,
    /// Whether a prompt is open: for a name or a message, or one of the
    /// dialogs with choices.
    pub prompting: bool,
    /// What the labelled rows of the open dialog hold, in order; empty
    /// when the dialog has none or none is open.
    pub form_rows: Vec<String>,
    /// What the pane beside the left panel adds while it is open
    /// (`remote`, `worktree`, `submodule`), and what its fields hold.
    pub docked: Option<&'static str>,
    pub docked_fields: Vec<String>,
    /// What the details say of the shown commit's signature: git's word
    /// for its state, and who signed. None for a commit without one, and
    /// until it has been asked.
    pub signature: Option<(String, Option<String>)>,
    /// Who may sign with an SSH key, as the signing view lists them: the
    /// names of each signer, joined. Empty until that view has asked.
    pub allowed_signers: Vec<String>,
    /// How many keys the signing view lists as able to sign.
    pub signing_keys: usize,
    /// The key the arrows are on in the signing view, as git would be
    /// given it.
    pub signing_cursor: Option<String>,
    /// Whether the repository is opened read-only.
    pub read_only: bool,
    /// How many branches the references panel was given.
    pub branches: usize,
    /// Whether the right panel is on show: it is switched on and a row of
    /// the graph is selected.
    pub right_panel: bool,
    /// Whether the details list what differs from the shown commit to the
    /// working tree, and say so over the files.
    pub compared_with_worktree: bool,
    /// How many files the details list.
    pub commit_files: usize,
    /// Whether the details say that a partial clone left what their files
    /// need on its remote, with the button that fetches it.
    pub files_left_on_remote: bool,
    /// Why the details could not list the files, when they could not.
    pub files_error: Option<String>,
    /// Whether the trial merges that mark the local branches that would
    /// conflict are still under way, and how many branches they have
    /// answered for.
    pub conflict_marks: (bool, usize),
    /// How many times the references and the history were read again.
    pub reloads: u64,
    /// The other worktrees as last looked at: their paths and how many
    /// files are changed in each.
    pub other_worktrees: Vec<(String, usize)>,
    /// The looks at other worktrees so far, in the whole application.
    pub worktree_looks: Vec<WorktreeLook>,
    /// How many of those began while another was under way.
    pub worktree_look_overlaps: u64,
    /// What the status bar says is running, and whether it offers to stop
    /// it.
    pub operation: Option<(String, bool)>,
    /// The command that changes the repository on the session thread, by
    /// its label, whether the status bar shows it yet or not.
    pub busy: Option<String>,
    /// The worktrees being removed, by path.
    pub removing: Vec<String>,
}

/// One look at other worktrees, as the probes tell of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeLook {
    /// The worktrees it looked at.
    pub paths: Vec<String>,
    /// When it began and when it ended, on the scheduler's clock, from
    /// the first look.
    pub began: std::time::Duration,
    pub ended: std::time::Duration,
    /// Whether the user asked for it.
    pub asked: bool,
}

impl Workspace {
    /// What the tab has on show.
    pub fn probe(&self, cx: &App) -> WorkspaceProbe {
        let (worktree_looks, worktree_look_overlaps) = Self::worktree_looks(cx);
        WorkspaceProbe {
            worktree_looks,
            worktree_look_overlaps,
            signature: self.details_view.read(cx).signature(),
            compared_with_worktree: self.details_view.read(cx).compared_with_worktree(),
            commit_files: self.details_view.read(cx).file_count(),
            files_left_on_remote: self.details_view.read(cx).files_left_on_remote(),
            files_error: self.details_view.read(cx).files_error().map(str::to_owned),
            conflict_marks: (
                self.conflict_marks.under_way(),
                self.conflict_marks.answered(),
            ),
            reloads: self.reloads,
            other_worktrees: cx
                .global::<crate::prefs::Prefs>()
                .repo_state(&self.session.path)
                .worktrees_seen
                .iter()
                .map(|seen| (seen.path.clone(), seen.counts.total()))
                .collect(),
            allowed_signers: self
                .signing_view
                .read(cx)
                .signers()
                .map(|signers| {
                    signers
                        .signers
                        .iter()
                        .map(|signer| signer.principals.join(","))
                        .collect()
                })
                .unwrap_or_default(),
            signing_keys: self.signing_view.read(cx).key_count(),
            signing_cursor: self.signing_view.read(cx).key_under_cursor(),
            loaded: self.snapshot.is_some()
                && self.graph.is_some()
                && self.status_loaded
                && self.graph_loads == 0,
            error: self.error.clone(),
            sheet: self.sheet.map(|sheet| match sheet {
                Sheet::Activity => "activity",
                Sheet::Journal => "undo_history",
                Sheet::Flags => "index_flags",
                Sheet::Rerere => "rerere",
                Sheet::Pull => "pull",
                Sheet::NewPull => "new_pull",
                Sheet::Issue => "issue",
            }),
            planning: self.planning,
            file: self
                .file
                .clone()
                .filter(|_| self.show_diff)
                .map(|(path, rev)| (path, rev.map(|id| id.to_hex()))),
            file_tab: match self.file_tab {
                FileTab::Diff => "diff",
                FileTab::History => "history",
                FileTab::Blame => "blame",
                FileTab::Edit => "edit",
                FileTab::Merge => "merge",
            },
            changes_shown: self.right == RightPane::Wip,
            status_paths: self.status.len(),
            in_progress: self.in_progress.as_ref().map(|operation| {
                (
                    format!("{:?}", operation.kind).to_lowercase(),
                    operation.conflicts.len(),
                )
            }),
            prompting: self.prompt_action.is_some()
                || self.forms.is_open()
                || crate::ask::is_open(cx),
            form_rows: self.form_row_texts(cx),
            docked: self.docked_adds().map(|adds| adds.key()),
            docked_fields: self.docked_texts(cx),
            read_only: self.read_only(),
            branches: self
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.branches.len())
                .unwrap_or(0),
            right_panel: cx
                .global::<crate::prefs::Prefs>()
                .right_panel_shown(Some(&self.session.path))
                && self.something_selected(cx),
            operation: self
                .operation_text()
                .map(|text| (text, self.operation_stoppable())),
            busy: self.busy_label().map(str::to_owned),
            removing: self.removing.iter().cloned().collect(),
        }
    }

    /// The graph of the tab.
    pub fn graph(&self) -> Entity<GraphView> {
        self.graph_view.clone()
    }

    /// The references panel of the tab, to ask how it is laid out.
    pub fn refs(&self) -> Entity<crate::views::refs::RefsView> {
        self.refs_view.clone()
    }

    /// The changes panel of the tab, to give it the focus: its rows are
    /// then walked with its own keys.
    pub fn changes(&self) -> Entity<ChangesView> {
        self.changes_view.clone()
    }

    /// The merge tool of the tab, to ask what its result holds.
    pub fn merge_tool(&self) -> Entity<crate::views::merge_tool::MergeTool> {
        self.merge_tool.clone()
    }

    /// The commit box of the tab, to give it the focus.
    pub fn commit_box(&self) -> Entity<CommitBox> {
        self.commit_box.clone()
    }

    /// The blame of the file in the centre, and of the file in its past.
    pub fn blame(&self) -> Entity<crate::views::blame::BlameView> {
        self.blame_view.clone()
    }

    /// The editor of the file in the centre.
    pub fn editor(&self) -> Entity<crate::views::edit::EditView> {
        self.edit_view.clone()
    }

    /// The viewer of the file in the centre, to ask what it shows.
    pub fn diff(&self) -> Entity<crate::views::diff::DiffView> {
        self.diff_view.clone()
    }
}
