//! The references panel: local branches, remotes, worktrees, stashes, pull
//! requests, issues, tags, submodules and gitflow, each a section with a
//! heading. Every open section is a virtual list that scrolls on its own,
//! and the open ones share the panel's height, so local and remote
//! branches are seen at the same time and a repository with thousands of
//! remote branches costs no more per frame than a small one. Folded, the
//! panel is a strip with one mark and count per section.
//!
//! The sections can be put in another order (drag a heading onto another,
//! or the heading's menu), hidden and brought back (the button at the top),
//! resized against each other (drag the edge between two open ones) and
//! one of them given all the room (the others close); folders of branches
//! and tags close and open. All of it is remembered per repository. How
//! the open lists share the height is in `refs/heights.rs`; the rows
//! themselves, the filter and its ranking in `refs/rows.rs`. A section
//! with nothing in it is not shown (but for the branches, the remotes and
//! the worktrees), gitflow's only where it is set up, and every section
//! starts out closed.
//!
//! The panel has no keys of its own: a click on a row selects its commit
//! in the graph, which then has the keys; a click with Ctrl or Shift picks
//! several, and Enter in the filter selects the commit of the first
//! reference it found.
//!
//! Sources: gpui-kit documentation (virtual list, popup menu, anchored and
//! deferred elements).

mod heights;
mod rows;

pub(crate) use self::rows::under_remote;

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, BTreeSet, HashMap},
    rc::Rc,
    sync::Arc,
};

use bitbraid_git::{Oid, Snapshot};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, VirtualListScrollHandle,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenu},
    scroll::{ScrollableElement, ScrollbarAxis},
    tooltip::Tooltip,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde::Deserialize;

use self::heights::{dragged, laid_out, on_close, on_open};
use self::rows::{
    Item, Mark, Needle, Place, RowId, SECTIONS, Shape, age_text, branch_aside, branch_tip,
    build_sections, clicked, focused, known_section, moved_before, nudged, ordered, pin_moved,
    pin_moves, remote_ref, section_number, track_text,
};
use crate::{
    prefs::{Prefs, RepoState},
    session::{GraphData, SessionHandle},
    views::{
        flow::{self, BranchNote, FlowRequest, FlowState, RunFlow},
        issues::{IssuesEvent, IssuesView},
        pulls::{PullsEvent, PullsView},
        rewrite::{self, DragChip, Dragged, DropTarget, Rewrite, RunRewrite},
        stashes::{self, RunStash, StashRequest},
        submodules::{self, RunSubmodule, SubmoduleRequest},
        tags::{self, RunTag, TagRequest},
        workspace::PushHow,
        worktrees::{self, RunWorktree, WorktreeRequest},
    },
};

/// Height of a reference's row.
const ROW_HEIGHT: f32 = 24.0;
/// How much higher the row of a branch is that has a description: the
/// line it stands on, under the name.
const DESCRIBED: f32 = 14.0;
/// How far from the row's left edge a name begins, before its indent:
/// the padding, the slot for a folder's arrow and the mark of what the
/// row is, with the gaps between them.
const NAME_AT: f32 = 45.0;
/// The issue a branch's name holds: the repository of the issue, its
/// number, and where its key stands in the row's name when it is there.
type BranchIssue = (
    bitbraid_integrations::model::RepoLink,
    u64,
    Option<std::ops::Range<usize>>,
);

/// What a row keeps free at its right end, where the list's scrollbar
/// stands: how far a branch is ahead and behind ends 25 short of the
/// panel's edge.
const ROW_END: f32 = 25.0;
/// Height of a section's heading.
const HEAD_HEIGHT: f32 = 30.0;
/// How far the button that opens a row's menu stands from the row's right
/// end; it shows while the pointer is on the row.
const MORE_AT: f32 = 7.0;
/// The room between the parts of a row: the column at its left end, the
/// mark, the name and what stands beside it. A name begins 45 from the
/// panel's edge at the top level.
const ROW_GAP: f32 = 5.5;
/// What a heading's buttons that show under the pointer know it by.
const HEAD_GROUP: &str = "refs-head";
/// What a row's controls that show under the pointer know it by.
const ROW_GROUP: &str = "refs-row";
/// How far each level of a tree is indented.
const INDENT: f32 = 15.0;
/// Width of the panel when it is folded to a strip.
pub const RAIL_WIDTH: f32 = 36.0;
/// The strip's measures: the row of the button that opens the panel
/// again, the room under it, and the step from one section's mark to the
/// next.
const RAIL_FIRST: f32 = 32.0;
const RAIL_GAP: f32 = 3.0;
const RAIL_STEP: f32 = 37.0;
/// The box a mark of the strip opens.
const PEEK_WIDTH: f32 = 248.0;
const PEEK_HEIGHT: f32 = 270.0;

/// Events from the references panel.
#[derive(Debug, Clone)]
pub enum RefsEvent {
    /// Scroll the graph to this commit and select it.
    Reveal(Oid),
    /// Hidden, shown alone or pinned references changed; reload the graph.
    VisibilityChanged,
    /// Check this out: a branch, `remote/branch`, or a tag.
    Checkout(String),
    /// Ask for a name and create a branch at this reference.
    NewBranch(String),
    /// Ask for a new name for this local branch.
    Rename(String),
    /// Ask, then delete this local branch.
    Delete(String),
    /// Merge this into the current branch, or rebase the current branch
    /// onto it.
    Rewrite(Rewrite),
    /// Use a stash.
    Stash(StashRequest),
    /// Push or delete a tag.
    Tag(TagRequest),
    /// Move this local branch, which is not checked out, forward to its
    /// upstream.
    FastForward(String),
    /// Pull this local branch: the checked-out one as the toolbar does,
    /// another by fetching and moving it forward.
    Pull(String),
    /// Ask what this local branch is to follow.
    SetUpstream(String),
    /// Have this local branch follow nothing.
    ClearUpstream(String),
    /// Ask, then delete this branch on its remote (`remote/branch`).
    DeleteOnRemote(String),
    /// Fetch, add, repoint or remove a remote.
    Remote(RemoteRequest),
    /// Ask, then delete these local branches (short names).
    DeleteMany(Vec<String>),
    /// Push these local branches (short names).
    PushMany(Vec<String>),
    /// Push this local branch in a way that overwrites, after asking.
    PushWith(String, PushHow),
    /// Add, open, lock or remove a worktree.
    Worktree(WorktreeRequest),
    /// Add, open, update or remove a submodule.
    Submodule(SubmoduleRequest),
    /// Set gitflow up, start or finish one of its branches; describe a
    /// branch; delete the merged ones.
    Flow(FlowRequest),
    /// The worktrees are being looked at: count what is changed in each.
    CountWorktrees,
    /// The local branches came into view: what is worked out for them
    /// only while they show can be.
    LocalShown,
    /// The pointer came onto a branch or tag that is at this commit
    /// (true), or left it: the graph lights its history.
    Hover(Oid, bool),
    /// Select this commit, the one a branch is at, and list what differs
    /// between it and the working tree.
    Compare(Oid),
    /// A branch or tag was dropped on another one's row: offer what the
    /// two can do, as a drop on a row of the graph does.
    Dropped(Dragged, DropTarget),
    /// Read the pull requests again, past what was kept.
    RefreshPulls,
    /// Read the issues again, past what was kept.
    RefreshIssues,
    /// Show the issue a branch's name holds: its key in the name was
    /// clicked.
    OpenIssue {
        /// The repository of the issue.
        link: bitbraid_integrations::model::RepoLink,
        /// Its number.
        number: u64,
    },
    /// The strip's button was pressed while what the middle shows holds
    /// the panel folded: the tab closes that, as its own way out does.
    OpenAsked,
}

/// What to do with all the selected references at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum Batch {
    Delete,
    Push,
    Hide,
    Show,
    Solo,
    Clear,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct RunBatch {
    what: Batch,
}

/// What can be asked about a remote.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum RemoteRequest {
    /// Fetch this remote, pruning branches that are gone there.
    Fetch(String),
    /// Ask for a name and a URL, add the remote and fetch it.
    Add,
    /// Ask for this remote's URLs.
    EditUrl(String),
    /// Ask, then remove this remote.
    Remove(String),
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct RunRemote {
    request: RemoteRequest,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct FastForwardBranch {
    name: String,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct CheckoutRef {
    target: String,
}

/// What the menu of one branch asks about pushing, pulling and following.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum Sync {
    Push,
    PushLease,
    PushForce,
    Pull,
    SetUpstream,
    ClearUpstream,
    DeleteOnRemote,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct SyncBranch {
    /// A local branch by its short name; for a deletion, `remote/branch`.
    name: String,
    what: Sync,
}

/// What the user wrote about the local branch a row is for, when they
/// wrote anything: from what is known of the branches already, with no
/// question asked of git for the row.
fn description_of<'a>(notes: &'a HashMap<String, BranchNote>, item: &Item) -> Option<&'a str> {
    if item.folder.is_some() || item.remote.is_some() || item.place != Place::None {
        return None;
    }
    match RefKind::of(&item.full_name) {
        RefKind::Local(name) => notes.get(&name)?.description.as_deref(),
        _ => None,
    }
}

/// How high the row of an item is.
fn height_of(notes: &HashMap<String, BranchNote>, item: &Item) -> f32 {
    match description_of(notes, item) {
        Some(_) => ROW_HEIGHT + DESCRIBED,
        None => ROW_HEIGHT,
    }
}

/// A branch's description as its menu shows it: its first line, cut where
/// a menu would get too wide.
fn described_as(description: &str) -> String {
    const WIDEST: usize = 60;
    let line = description.lines().next().unwrap_or("").trim();
    if line.chars().count() > WIDEST {
        let cut: String = line.chars().take(WIDEST).collect();
        format!("{}…", cut.trim_end())
    } else {
        line.to_owned()
    }
}

/// What a local branch's row says when merging it into the checked-out
/// branch would conflict in `files` files.
fn conflict_mark(files: usize) -> String {
    say!("refs-conflict-mark", files)
}

/// What the four entries about pulling, pushing and following say for a
/// local branch, and whether each is greyed: pull, push, set upstream,
/// clear upstream. One that cannot run says why.
fn sync_labels(upstream: Option<&str>, is_head: bool) -> [(String, bool); 4] {
    [
        (
            match (upstream, is_head) {
                (None, _) => say!("refs-pull-nothing").to_owned(),
                (Some(upstream), true) => say!("refs-pull-from", upstream),
                (Some(upstream), false) => say!("refs-pull-fast-forward", upstream),
            },
            upstream.is_none(),
        ),
        (
            match upstream {
                Some(upstream) => say!("refs-push-to", upstream),
                None => say!("refs-push-publish").to_owned(),
            },
            false,
        ),
        (say!("refs-set-upstream").to_owned(), false),
        (
            match upstream {
                Some(upstream) => say!("refs-clear-upstream", upstream),
                None => say!("refs-clear-upstream-nothing").to_owned(),
            },
            upstream.is_none(),
        ),
    ]
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct NewBranchAt {
    start: String,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct RenameBranch {
    name: String,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct DeleteBranch {
    name: String,
}

/// Ask, then delete these local branches.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct DeleteMany {
    names: Vec<String>,
}

/// Put a text on the clipboard: a branch's name, or the id of the commit
/// it is at.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct CopyText {
    text: String,
}

/// List what differs between the commit a branch is at and the working
/// tree.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct CompareTip {
    /// The commit, by its full id.
    commit: String,
}

/// One entry of the menu of a branch's row, or a run of entries that
/// another view builds (merging and rebasing, a reset, what the graph
/// shows).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BranchEntry {
    /// Pulling, pushing and what the branch follows.
    Pull,
    Push,
    SetUpstream,
    /// The pushes that overwrite: they ask first and say what would go.
    PushLease,
    PushForce,
    ClearUpstream,
    FastForward,
    /// Merge it into the checked-out branch, rebase that onto it.
    Combine,
    Checkout,
    Worktree,
    NewBranch,
    CherryPick,
    /// Cherry-pick as staged changes, without a commit.
    CherryPickStaged,
    Reset,
    Reword,
    Revert,
    Drop,
    /// Move the commit one place down, past its parent.
    MoveDown,
    ApplyPatch,
    Rename,
    Delete,
    DeleteOnRemote,
    Describe,
    CopyName,
    CopyId,
    /// Hide it, pin it, show it alone.
    Seen,
    ShowAll,
    Focus,
    Compare,
    Tag,
    /// A tag's own: rebase the checked-out branch onto it in the editor;
    /// delete it here, or on a remote; push it; what it says about
    /// itself; give it a message.
    InteractiveOnto,
    /// Finish the branch, as gitflow does: in gitflow's section.
    Finish,
    TagDelete,
    TagDeleteRemote,
    TagPush,
    TagDetails,
    Annotate,
    /// Make a pull request of the branch.
    StartPull,
}

/// The menu of a tag's row as groups: what it does to the checked-out
/// branch; checking it out; a worktree; what is made of its commit or
/// brought to it; deleting it here or on a remote, then pushing it and
/// what it says; its name to copy; what the graph shows; and for a tag
/// without a message, giving it one.
fn tag_groups(lightweight: bool) -> Vec<Vec<BranchEntry>> {
    use BranchEntry as E;
    let mut groups = vec![
        vec![E::Combine, E::InteractiveOnto],
        vec![E::Checkout],
        vec![E::Worktree],
        vec![
            E::NewBranch,
            E::CherryPick,
            E::Reset,
            E::Revert,
            E::CherryPickStaged,
        ],
        vec![E::TagDelete, E::TagDeleteRemote, E::TagPush, E::TagDetails],
        vec![E::CopyName],
        vec![E::Seen],
        vec![E::ShowAll, E::Focus],
    ];
    if lightweight {
        groups.push(vec![E::Annotate]);
    }
    groups
}

/// The menu of a branch's row as groups of entries, a rule between two
/// groups: what goes between the branch and its remote; what it does to
/// the checked-out branch; checking it out; a worktree; what is made of
/// its commit or brought to it; its own name and end; what is copied; what
/// the graph shows of it; comparing; a tag. The checked-out branch has
/// nothing that would combine it with itself or check it out, and has
/// what rewrites its last commit.
fn branch_groups(local: bool, is_head: bool, follows: bool) -> Vec<Vec<BranchEntry>> {
    use BranchEntry as E;
    let mut sync = vec![
        E::Pull,
        E::Push,
        E::SetUpstream,
        E::PushLease,
        E::PushForce,
        E::ClearUpstream,
    ];
    if follows {
        sync.push(E::FastForward);
    }
    let seen = [vec![E::Seen], vec![E::ShowAll, E::Focus]];
    let last = [vec![E::Compare], vec![E::Tag]];
    let groups = match (local, is_head) {
        (true, true) => vec![
            sync,
            vec![E::NewBranch, E::Reset, E::Reword, E::Revert],
            vec![E::Drop, E::MoveDown],
            vec![E::ApplyPatch, E::Rename, E::Delete, E::Describe],
            vec![E::CopyName, E::CopyId],
        ],
        (true, false) => vec![
            sync,
            vec![E::Combine],
            vec![E::Checkout],
            vec![E::Worktree],
            vec![
                E::NewBranch,
                E::CherryPick,
                E::Reset,
                E::Revert,
                E::CherryPickStaged,
            ],
            vec![E::Rename, E::Delete, E::Describe],
            vec![E::CopyName, E::CopyId],
        ],
        (false, _) => vec![
            vec![E::Combine],
            vec![E::Checkout],
            vec![E::Worktree],
            vec![
                E::NewBranch,
                E::CherryPick,
                E::Reset,
                E::Revert,
                E::CherryPickStaged,
            ],
            vec![E::DeleteOnRemote],
            vec![E::CopyName, E::CopyId],
        ],
    };
    groups.into_iter().chain(seen).chain(last).collect()
}

/// What the menu of a branch's row needs to know of the branch.
#[derive(Debug, Clone)]
struct BranchRow {
    /// A local branch by its short name, a remote one as `remote/branch`,
    /// a tag by its name.
    name: String,
    full_name: String,
    local: bool,
    /// It is a tag, and whether that has no message of its own.
    tag: Option<bool>,
    /// A branch of gitflow's that its row in gitflow's section finishes:
    /// its kind and its name without the prefix.
    finish: Option<(&'static str, String)>,
    /// The commit it is at, by its full id.
    commit: String,
    /// That commit is a merge commit.
    merge: bool,
    is_head: bool,
    /// It is at the commit HEAD is at.
    at_head: bool,
    protected: bool,
    upstream: Option<String>,
    described: bool,
    /// The checked-out branch, when HEAD is on one.
    current: Option<String>,
    hidden: bool,
    soloed: bool,
    pin: Pin,
    moves: (bool, bool),
    /// What the entry for focused visibility says.
    focus: String,
    /// A remote is linked to an account: pull requests can be made.
    pulls: bool,
    /// The open pull request of the branch, if it has one.
    pull: Option<crate::views::pulls::facts::PullMark>,
}

impl BranchRow {
    /// Add one entry of the menu, or the run of entries it stands for.
    fn add(
        &self,
        menu: PopupMenu,
        entry: BranchEntry,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        use BranchEntry as E;
        let name = &self.name;
        let sync = |what| -> Box<dyn Action> {
            Box::new(SyncBranch {
                name: name.clone(),
                what,
            })
        };
        let rewrite = |rewrite| -> Box<dyn Action> { Box::new(RunRewrite { rewrite }) };
        let [pull, push, follow, unfollow] = sync_labels(self.upstream.as_deref(), self.is_head);
        // A merge commit is asked about first: against which parent.
        let pick = |menu: PopupMenu, staged: bool| {
            menu.menu_with_disabled(
                match (self.merge, staged, self.at_head) {
                    (false, false, false) => say!("refs-cherry-pick"),
                    (false, true, false) => say!("refs-cherry-pick-staged"),
                    (true, false, false) => say!("refs-cherry-pick-merge"),
                    (true, true, false) => say!("refs-cherry-pick-merge-staged"),
                    (false, false, true) => say!("refs-cherry-pick-at-head"),
                    (false, true, true) => say!("refs-cherry-pick-staged-at-head"),
                    (true, false, true) => say!("refs-cherry-pick-merge-at-head"),
                    (true, true, true) => say!("refs-cherry-pick-merge-staged-at-head"),
                },
                rewrite(Rewrite::CherryPick {
                    commit: self.commit.clone(),
                    merge: self.merge,
                    staged,
                }),
                self.at_head,
            )
        };
        match entry {
            E::Pull => menu.menu_with_disabled(pull.0, sync(Sync::Pull), pull.1),
            E::Push => menu.menu_with_disabled(push.0, sync(Sync::Push), push.1),
            E::SetUpstream => menu.menu_with_disabled(follow.0, sync(Sync::SetUpstream), follow.1),
            E::PushLease => menu.menu(say!("refs-push-lease"), sync(Sync::PushLease)),
            E::PushForce => menu.menu(say!("refs-push-force"), sync(Sync::PushForce)),
            E::ClearUpstream => {
                menu.menu_with_disabled(unfollow.0, sync(Sync::ClearUpstream), unfollow.1)
            }
            E::FastForward => match &self.upstream {
                Some(upstream) => menu.menu_with_disabled(
                    if self.is_head {
                        say!(
                            "refs-fast-forward-checked-out",
                            upstream = upstream.as_str()
                        )
                    } else {
                        say!("refs-fast-forward", upstream = upstream.as_str())
                    },
                    Box::new(FastForwardBranch { name: name.clone() }),
                    self.is_head,
                ),
                None => menu,
            },
            E::Combine => {
                rewrite::combine_entries(menu, self.current.as_deref(), name, name, self.at_head)
            }
            E::Checkout => menu.menu_with_disabled(
                if self.tag.is_some() {
                    say!("refs-checkout-detached")
                } else if self.is_head {
                    say!("refs-checkout-checked-out")
                } else {
                    say!("refs-checkout")
                },
                Box::new(CheckoutRef {
                    target: name.clone(),
                }),
                self.is_head,
            ),
            E::Worktree => worktrees::new_worktree_entry(menu, Some(name)),
            E::NewBranch => menu.menu(
                say!("refs-new-branch"),
                Box::new(NewBranchAt {
                    start: name.clone(),
                }),
            ),
            E::CherryPick => pick(menu, false),
            E::CherryPickStaged => pick(menu, true),
            E::Reset => rewrite::reset_entries(
                menu,
                self.current.as_deref(),
                &self.commit,
                self.at_head,
                window,
                cx,
            ),
            E::Reword => menu.menu(
                say!("refs-reword"),
                rewrite(Rewrite::Interactive {
                    first: self.commit.clone(),
                    reword: true,
                }),
            ),
            E::Revert => menu.menu(
                if self.merge {
                    say!("refs-revert-merge")
                } else {
                    say!("refs-revert")
                },
                rewrite(Rewrite::Revert {
                    commit: self.commit.clone(),
                    merge: self.merge,
                }),
            ),
            E::Drop => menu.menu(
                say!("refs-drop"),
                rewrite(Rewrite::Drop {
                    commit: self.commit.clone(),
                }),
            ),
            E::MoveDown => menu.menu_with_disabled(
                say!("refs-move-down"),
                rewrite(Rewrite::Move {
                    commits: vec![self.commit.clone()],
                    up: false,
                }),
                self.merge,
            ),
            E::ApplyPatch => menu.menu(
                say!("refs-apply-patch"),
                Box::new(crate::keymap::ApplyPatch),
            ),
            E::Rename => menu.menu(
                say!("refs-rename"),
                Box::new(RenameBranch { name: name.clone() }),
            ),
            E::Delete => menu.menu_with_disabled(
                if self.is_head {
                    say!("refs-delete-checked-out")
                } else if self.protected {
                    say!("refs-delete-protected")
                } else {
                    say!("refs-delete")
                },
                Box::new(DeleteBranch { name: name.clone() }),
                self.is_head || self.protected,
            ),
            E::DeleteOnRemote => {
                menu.menu(say!("refs-delete-on-remote"), sync(Sync::DeleteOnRemote))
            }
            E::Describe => flow::describe_entry(menu, name, self.described),
            E::CopyName => menu.menu(
                say!("refs-copy-name"),
                Box::new(CopyText { text: name.clone() }),
            ),
            E::CopyId => menu.menu(
                say!("refs-copy-id"),
                Box::new(CopyText {
                    text: self.commit.clone(),
                }),
            ),
            E::Seen => own_visibility_entries(
                menu,
                &self.full_name,
                self.hidden,
                self.soloed,
                self.pin,
                self.moves,
            ),
            E::ShowAll => menu.menu(say!("refs-show-all"), Box::new(ShowAll)),
            E::Focus => menu.menu(self.focus.clone(), Box::new(FocusedVisibility)),
            E::Compare => menu.menu(
                say!("refs-compare"),
                Box::new(CompareTip {
                    commit: self.commit.clone(),
                }),
            ),
            E::Tag => tags::new_tag_entry(menu, name),
            E::InteractiveOnto => match &self.current {
                Some(current) => menu.menu(
                    say!(
                        "refs-rebase-onto",
                        current = current.as_str(),
                        name = name.as_str()
                    ),
                    rewrite(Rewrite::InteractiveOnto { onto: name.clone() }),
                ),
                None => menu.menu_with_disabled(
                    say!("refs-rebase-onto-detached"),
                    rewrite(Rewrite::InteractiveOnto { onto: name.clone() }),
                    true,
                ),
            },
            E::TagDelete => menu.menu(
                say!("tags-delete"),
                Box::new(RunTag {
                    request: TagRequest::Delete(name.clone()),
                }),
            ),
            E::TagDeleteRemote => menu.menu(
                say!("tags-delete-remote"),
                Box::new(RunTag {
                    request: TagRequest::DeleteRemote(name.clone()),
                }),
            ),
            E::TagPush => menu.menu(
                say!("tags-push"),
                Box::new(RunTag {
                    request: TagRequest::Push(name.clone()),
                }),
            ),
            E::TagDetails => menu.menu(
                say!("tags-details"),
                Box::new(RunTag {
                    request: TagRequest::Details(name.clone()),
                }),
            ),
            E::Annotate => tags::annotate_entry(menu, name),
            E::Finish => match &self.finish {
                Some((kind, short)) => flow::branch_entries(menu, kind, short),
                None => menu,
            },
            // The open pull request's page and link, then the entry that
            // starts one.
            E::StartPull => crate::views::pulls::create::start_entry(
                crate::views::pulls::create::open_pull_entries(menu, self.pull.as_ref()),
                name,
                self.is_head,
            ),
        }
    }

    /// The whole menu: the groups of [`branch_groups`] (`tag_groups` for a
    /// tag), a rule between two.
    /// It is longer than a small window is high, and scrolls there.
    fn menu(&self, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
        let mut groups = match self.tag {
            Some(lightweight) => tag_groups(lightweight),
            None => branch_groups(self.local, self.is_head, self.upstream.is_some()),
        };
        // In gitflow's section, finishing it comes after what goes between
        // it and its remote.
        if self.finish.is_some() {
            let at = usize::from(!groups.is_empty());
            groups.insert(at.min(groups.len()), vec![BranchEntry::Finish]);
        }
        // Making a pull request comes after what is made of its commit.
        if self.pulls && self.tag.is_none() {
            let at = groups
                .iter()
                .position(|group| group.contains(&BranchEntry::NewBranch))
                .map_or(groups.len(), |at| at + 1);
            groups.insert(at.min(groups.len()), vec![BranchEntry::StartPull]);
        }
        let mut menu = crate::views::menu_fit::fit(menu, window);
        for (at, group) in groups.iter().enumerate() {
            if at > 0 {
                menu = menu.separator();
            }
            for entry in group {
                menu = self.add(menu, *entry, window, cx);
            }
        }
        menu
    }
}

/// What a row of the panel names, as the branch commands want it: a local
/// branch by its short name, a remote branch as `remote/branch`, a tag by
/// its name. Stashes, worktrees and submodules are none of these.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RefKind {
    Local(String),
    Remote(String),
    Tag(String),
    Other,
}

impl RefKind {
    fn of(full_name: &str) -> Self {
        if let Some(name) = full_name.strip_prefix("refs/heads/") {
            Self::Local(name.to_owned())
        } else if let Some(name) = full_name.strip_prefix("refs/remotes/") {
            Self::Remote(name.to_owned())
        } else if let Some(name) = full_name.strip_prefix("refs/tags/") {
            Self::Tag(name.to_owned())
        } else {
            Self::Other
        }
    }

    /// What to hand to checkout and to "new branch at".
    fn target(&self) -> Option<&str> {
        match self {
            Self::Local(name) | Self::Remote(name) | Self::Tag(name) => Some(name),
            Self::Other => None,
        }
    }
}

/// Hide a reference from the graph, or show it again. The menu of a row
/// here and the menu of a label in the graph both dispatch it.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
pub struct ToggleHidden {
    /// The reference, by its full name.
    pub full_name: String,
}

/// Show only this reference (and the others shown alone) in the graph,
/// or stop doing so.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
pub struct ToggleSolo {
    /// The reference, by its full name.
    pub full_name: String,
}

/// Keep a branch's line at the left of the graph, or let it go.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
pub struct TogglePinned {
    /// The reference, by its full name.
    pub full_name: String,
}

/// A change to what the graph shows of one reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Seen {
    /// Hide it, or show it again.
    Hidden(String),
    /// Show only it, or stop.
    Solo(String),
    /// Pin it, or unpin it.
    Pinned(String),
    /// Move its pinned lane by one: to the right, or to the left.
    Moved(String, bool),
}

/// Move a pinned reference one lane to the left or to the right.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
pub struct MovePinned {
    /// The reference, by its full name.
    pub full_name: String,
    /// To the right; else to the left.
    pub right: bool,
}

/// Whether a pinned reference has a lane to its left to move to, and one
/// to its right: for the menu of a label in the graph, which offers what
/// a row here offers.
pub(crate) fn lane_moves(pinned: &[String], full_name: &str) -> (bool, bool) {
    pin_moves(pinned, full_name)
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct ShowAll;

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct FocusedVisibility;

/// What a section's heading can be asked to do with its section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum SectionVerb {
    Maximise,
    /// Hide every reference of the section from the graph, or show them
    /// all again.
    HideAll,
    ShowAll,
    MoveUp,
    MoveDown,
    CloseFolders,
    OpenFolders,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct SectionDo {
    key: String,
    what: SectionVerb,
}

/// Show a section that is hidden, or hide one that shows.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct ToggleSection {
    key: String,
}

/// Every section back where and how it starts out.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = refs, no_json)]
struct ResetSections;

/// A heading that is being dragged to another place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DraggedSection(&'static str);

/// The edge between two open sections that is being dragged: the list
/// above it, the list below it, and how many headings lie between the
/// bottom of the one above and the edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SectionSplit {
    above: &'static str,
    below: &'static str,
    between: usize,
}

/// Nothing follows the pointer while an edge is dragged.
struct NoChip;

impl Render for NoChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// What builds a menu: for the pointer and for the keyboard alike.
type MenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

impl Mark {
    fn icon(self, open: bool) -> Icon {
        use gpui_kit::assets::IconName as Extra;
        match self {
            Mark::Branch => Icon::new(Extra::GitBranch),
            // An open folder is a folder; a closed one is drawn shut.
            Mark::Folder if open => Icon::new(IconName::Folder),
            Mark::Folder => Icon::new(IconName::FolderClosed),
            Mark::Remote => Icon::new(IconName::Globe),
            Mark::Tag => Icon::new(Extra::Tag),
            Mark::Stash => Icon::new(Extra::Archive),
            Mark::Worktree => Icon::new(Extra::FolderTree),
            Mark::Submodule => Icon::new(Extra::Package),
        }
    }
}

/// The mark of a section, in its heading and on the folded strip.
fn section_icon(key: &str) -> Icon {
    use gpui_kit::assets::IconName as Extra;
    match key {
        "local" => Icon::new(Extra::Laptop),
        "remote" => Icon::new(Extra::Cloud),
        "worktrees" => Icon::new(Extra::FolderTree),
        "stashes" => Icon::new(Extra::Archive),
        "tags" => Icon::new(Extra::Tag),
        "gitflow" => Icon::new(Extra::Workflow),
        "pulls" => Icon::new(Extra::GitPullRequest),
        "issues" => Icon::new(Extra::CircleDot),
        _ => Icon::new(Extra::Package),
    }
}

/// How high the first row of the panel's top is: the button that folds the
/// panel, and beside it the line of what is seen. As high as the strip's
/// row, so that the button stands where the strip's button that opens
/// the panel again stands. The filter is under it, and this much room
/// under the filter before the sections.
const TOP_ROW: f32 = RAIL_FIRST;
const TOP_BELOW: f32 = 8.0;

/// What narrows what the panel and the graph show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Narrowing {
    Filter,
    Solo,
    Hidden,
}

/// The tint of the top's block while something narrows what is seen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tint {
    Filter,
    Solo,
}

/// What the line at the top of the panel says.
struct TopLine {
    word: &'static str,
    count: String,
    undo: Option<(&'static str, Narrowing)>,
    tint: Option<Tint>,
}

/// What the names of a section's references begin with, for the
/// sections whose references the graph shows: hidden as a whole, the
/// section is that among the hidden ones.
fn section_space(key: &str) -> Option<&'static str> {
    match key {
        "local" => Some("refs/heads/"),
        "remote" => Some("refs/remotes/"),
        "tags" => Some("refs/tags/"),
        _ => None,
    }
}

/// The buttons a heading has under the pointer: adding a remote; counting
/// what is changed in the worktrees again and adding one; adding a
/// submodule; reading the pull requests or the issues again and creating
/// one. An icon, what its tooltip says, and what it asks for (none:
/// greyed).
fn head_buttons(key: &str) -> Vec<(IconName, &'static str, Option<RefsEvent>)> {
    match key {
        "remote" => vec![(
            IconName::Plus,
            say!("refs-head-add-remote"),
            Some(RefsEvent::Remote(RemoteRequest::Add)),
        )],
        "pulls" => vec![
            (
                IconName::RefreshCw,
                say!("refs-head-refresh-pulls"),
                Some(RefsEvent::RefreshPulls),
            ),
            (
                IconName::Plus,
                say!("refs-head-create-pull"),
                Some(RefsEvent::Rewrite(Rewrite::StartPull {
                    head: String::new(),
                    base: None,
                })),
            ),
        ],
        "issues" => vec![
            (
                IconName::RefreshCw,
                say!("refs-head-refresh-issues"),
                Some(RefsEvent::RefreshIssues),
            ),
            (IconName::Plus, say!("refs-head-create-issue"), None),
        ],
        "worktrees" => vec![
            (
                IconName::RefreshCw,
                say!("refs-head-count-worktrees"),
                Some(RefsEvent::CountWorktrees),
            ),
            (
                IconName::Plus,
                say!("refs-head-add-worktree"),
                Some(RefsEvent::Worktree(WorktreeRequest::New(None))),
            ),
        ],
        "submodules" => vec![(
            IconName::Plus,
            say!("refs-head-add-submodule"),
            Some(RefsEvent::Submodule(SubmoduleRequest::Add)),
        )],
        _ => Vec::new(),
    }
}

/// A heading's name as a menu entry: "BRANCHES" as "Branches".
fn title_case(title: &str) -> String {
    let lower = title.to_lowercase();
    let mut letters = lower.chars();
    match letters.next() {
        Some(first) => first.to_uppercase().chain(letters).collect(),
        None => String::new(),
    }
}

/// A section as the view shows it: its rows and where it is scrolled to.
struct Section {
    key: &'static str,
    title: &'static str,
    count: usize,
    items: Rc<Vec<Item>>,
    sizes: crate::zoom::Sizes,
    scroll: VirtualListScrollHandle,
}

/// The panel view.
pub struct RefsView {
    session: SessionHandle,
    snapshot: Option<Snapshot>,
    filter: Entity<InputState>,
    filter_text: String,
    /// The sections that are closed.
    collapsed: BTreeSet<String>,
    /// The folders that are closed, by `section:path`.
    closed_folders: BTreeSet<String>,
    /// Every section there can be, in the order they are shown in.
    order: Vec<&'static str>,
    /// The sections that are not shown at all.
    hidden_sections: BTreeSet<String>,
    /// How high an open section's list wants to be, for each that was
    /// dragged or given another's room (`refs/heights.rs`).
    sizes: HashMap<&'static str, f32>,
    /// How high the sections were drawn together last, their headings
    /// included: what the lists share.
    room: Rc<Cell<f32>>,
    /// Where the sections as a whole are scrolled to, when the panel is too
    /// low for every open list to have the least.
    body_scroll: ScrollHandle,
    /// The sections to show, rebuilt when the snapshot, the filter or what
    /// is open changes; rendering only looks at the rows in view.
    sections: Vec<Section>,
    /// Where each section was scrolled to, kept across rebuilds.
    scrolls: HashMap<&'static str, VirtualListScrollHandle>,
    /// Where each heading was drawn last, for a menu opened by key.
    head_bounds: Rc<RefCell<HashMap<&'static str, Bounds<Pixels>>>>,
    /// How far each local branch is ahead of its upstream and behind it,
    /// by short name.
    tracking: HashMap<String, (u32, u32)>,
    /// How many files are changed in each worktree, by its path.
    worktree_changes: HashMap<String, usize>,
    /// The worktrees being removed, by path.
    removing: BTreeSet<String>,
    /// Whether anything is changed in the worktree this tab shows.
    here_changed: bool,
    /// Where each submodule stands, by its path: the state, and whether
    /// tracked and untracked files of its own are changed.
    submodule_states: HashMap<String, (String, bool, bool)>,
    /// Gitflow as it is set up here, once that has been asked.
    flow: Option<FlowState>,
    /// The graph as it is shown: which commits are in it, and when each
    /// was made.
    graph: Option<Arc<GraphData>>,
    /// What there is to say about local branches, by short name.
    notes: HashMap<String, BranchNote>,
    /// Something else needs the room (a file is open): show the strip.
    forced_fold: bool,
    /// The window is too narrow for the panel beside the graph and the
    /// right panel: show the strip.
    narrow_fold: bool,
    /// The section the folded strip shows in a box beside it, and where its
    /// mark is among those of the strip.
    peek: Option<&'static str>,
    peek_at: usize,
    /// The repository is opened read-only: the menus offer what changes
    /// the view and nothing that would change the repository.
    read_only: bool,
    /// The selected references by full name, and the one a range starts at.
    selected: BTreeSet<String>,
    anchor: Option<String>,
    /// Whether the worktrees were listed open when the layout last
    /// changed: counting what is changed in each is asked for when they
    /// come into view, not with every folder that is opened.
    worktrees_shown: bool,
    /// The same for the local branches, and what would conflict with the
    /// checked-out one among them: by short name, in how many files.
    local_shown: bool,
    conflicts: HashMap<String, usize>,
    /// The pull requests section's rows, and whether they were last told
    /// they are in view.
    pulls: Option<Entity<PullsView>>,
    pulls_shown: bool,
    /// The same for the issues section.
    issues: Option<Entity<IssuesView>>,
    issues_shown: bool,
    /// Where the keyboard is.
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// The sections that start out closed: what was remembered for the
/// repository, else every one. What is remembered comes from a file
/// anyone can edit, so it is held to a sane size.
fn remembered_sections(closed: Option<Vec<String>>) -> BTreeSet<String> {
    match closed {
        Some(closed) => closed
            .into_iter()
            .filter(|key| !key.is_empty() && key.len() <= 40)
            .take(32)
            .collect(),
        None => SECTIONS.iter().map(|(key, _)| (*key).to_owned()).collect(),
    }
}

/// The most closed folders that are remembered, and the longest key.
const MOST_FOLDERS: usize = 4096;
const LONGEST_FOLDER: usize = 1024;

/// The closed folders that were remembered, held to a sane size.
fn remembered_folders(closed: &[String]) -> BTreeSet<String> {
    closed
        .iter()
        .filter(|key| !key.is_empty() && key.len() <= LONGEST_FOLDER)
        .take(MOST_FOLDERS)
        .cloned()
        .collect()
}

/// The heights that were remembered for sections that exist, as numbers
/// that can be laid out with (`heights` holds them to sane ones).
fn remembered_sizes(sizes: &BTreeMap<String, f32>) -> HashMap<&'static str, f32> {
    sizes
        .iter()
        .filter_map(|(key, size)| {
            let key = known_section(key)?;
            (size.is_finite() && *size >= 1.0).then(|| (key, size.min(100_000.0)))
        })
        .collect()
}

impl EventEmitter<RefsEvent> for RefsView {}

/// Whether a row can be pinned to the left of the graph, and what stands
/// in the way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pin {
    /// Not a branch: a tag or a stash has no line of its own to keep a
    /// lane for.
    NotOffered,
    /// It is pinned; unpinning always works.
    Pinned,
    /// It can be pinned.
    Free,
    /// Its commit is not in the graph as it is shown (it is hidden, or
    /// outside what is shown alone), so there is nothing to move.
    NotInGraph,
    /// As many are pinned as there is room for.
    Full,
}

impl Pin {
    /// What the menu says, and whether the entry is greyed.
    fn entry(self) -> Option<(&'static str, bool)> {
        match self {
            Self::NotOffered => None,
            Self::Pinned => Some((say!("refs-unpin"), false)),
            Self::Free => Some((say!("refs-pin"), false)),
            Self::NotInGraph => Some((say!("refs-pin-not-in-graph"), true)),
            Self::Full => Some((say!("refs-pin-full"), true)),
        }
    }
}

/// What a reference's menu offers about what the graph shows of that one
/// reference: hide it, pin it, show it alone. The same entries for a row of
/// the panel and for a label in the graph. They change the view, not the
/// repository.
pub(crate) fn own_visibility_entries(
    menu: PopupMenu,
    full_name: &str,
    hidden: bool,
    soloed: bool,
    pin: Pin,
    // For one that is pinned: whether there is a lane to its left to move
    // to, and one to its right.
    moves: (bool, bool),
) -> PopupMenu {
    let full_name = || full_name.to_owned();
    menu.menu(
        if hidden {
            say!("refs-show-in-graph")
        } else {
            say!("refs-hide-from-graph")
        },
        Box::new(ToggleHidden {
            full_name: full_name(),
        }),
    )
    .when_some(pin.entry(), |menu, (label, greyed)| {
        menu.menu_with_disabled(
            label,
            Box::new(TogglePinned {
                full_name: full_name(),
            }),
            greyed,
        )
    })
    // Among several pinned, which lane it has is changed here; the end it
    // stands at is greyed.
    .when(pin == Pin::Pinned && (moves.0 || moves.1), |menu| {
        menu.menu_with_disabled(
            say!("refs-lane-left"),
            Box::new(MovePinned {
                full_name: full_name(),
                right: false,
            }),
            !moves.0,
        )
        .menu_with_disabled(
            say!("refs-lane-right"),
            Box::new(MovePinned {
                full_name: full_name(),
                right: true,
            }),
            !moves.1,
        )
    })
    .menu(
        if soloed {
            say!("refs-solo-stop")
        } else {
            say!("refs-solo")
        },
        Box::new(ToggleSolo {
            full_name: full_name(),
        }),
    )
}

/// The entries of a reference's menu that are about what the graph shows:
/// of this reference, and of all of them.
fn visibility_entries(
    menu: PopupMenu,
    full_name: &str,
    hidden: bool,
    soloed: bool,
    pin: Pin,
    moves: (bool, bool),
    focus: &str,
) -> PopupMenu {
    own_visibility_entries(menu, full_name, hidden, soloed, pin, moves)
        .separator()
        .menu(say!("refs-show-all"), Box::new(ShowAll))
        .menu(focus.to_owned(), Box::new(FocusedVisibility))
}

/// What the entry for focused visibility says: which branches it keeps.
fn focus_label(kept: &[String]) -> String {
    let short = |full: &String| {
        full.strip_prefix("refs/heads/")
            .or_else(|| full.strip_prefix("refs/remotes/"))
            .unwrap_or(full)
            .to_owned()
    };
    match kept {
        [] => say!("refs-focus-detached").to_owned(),
        [head] => say!("refs-focus-alone", branch = short(head)),
        kept => say!(
            "refs-focus",
            branches = kept.iter().map(short).collect::<Vec<_>>().join(", ")
        ),
    }
}

impl RefsView {
    /// Create the panel.
    pub fn new(session: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.global::<Prefs>().repo_state(&session.path);
        // The field says which key puts the cursor in it.
        let key = cx
            .try_global::<crate::keymap::Keys>()
            .and_then(|keys| keys.resolved.keys_of("refs.filter").first().cloned())
            .map(|key| crate::keymap::display_key(&key));
        let placeholder = match key {
            Some(key) => say!("refs-filter-key", key),
            None => say!("refs-filter").to_owned(),
        };
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subscription = cx.subscribe_in(
            &filter,
            window,
            |this, state, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.filter_text = state.read(cx).value().to_string();
                    this.rebuild(cx);
                }
                // Enter goes on to what was found: the commit of the first
                // reference, selected in the graph, which takes the keys.
                InputEvent::PressEnter { .. } => {
                    let first = this
                        .sections
                        .iter()
                        .filter(|section| this.shows_rows(section.key))
                        .flat_map(|section| section.items.iter())
                        .find(|item| item.folder.is_none() && item.remote.is_none())
                        .map(|item| item.target);
                    if let Some(target) = first {
                        cx.emit(RefsEvent::Reveal(target));
                    }
                    let _ = window;
                }
                _ => {}
            },
        );
        let read_only = session.summary.bare;
        let mut view = Self {
            session,
            read_only,
            snapshot: None,
            filter,
            filter_text: String::new(),
            // Every section closed at first; once one was opened or closed
            // here, the panel comes back as it was left.
            collapsed: remembered_sections(state.closed_sections),
            closed_folders: remembered_folders(&state.closed_folders),
            order: ordered(&state.section_order),
            hidden_sections: state
                .hidden_sections
                .iter()
                .filter_map(|key| known_section(key))
                .map(str::to_owned)
                .collect(),
            sizes: remembered_sizes(&state.section_sizes),
            room: Rc::default(),
            body_scroll: ScrollHandle::new(),
            sections: Vec::new(),
            scrolls: HashMap::new(),
            head_bounds: Rc::default(),
            tracking: HashMap::new(),
            worktree_changes: HashMap::new(),
            removing: BTreeSet::new(),
            here_changed: false,
            submodule_states: HashMap::new(),
            flow: None,
            graph: None,
            notes: HashMap::new(),
            forced_fold: false,
            narrow_fold: false,
            peek: None,
            peek_at: 0,
            selected: BTreeSet::new(),
            anchor: None,
            worktrees_shown: false,
            local_shown: false,
            conflicts: HashMap::new(),
            pulls: None,
            pulls_shown: false,
            issues: None,
            issues_shown: false,
            focus_handle: cx.focus_handle(),
            _subscriptions: vec![subscription],
        };
        // Open from the start, they are counted with everything else that
        // is asked on open.
        view.worktrees_shown = view.worktrees_open();
        view.local_shown = view.local_open();
        view
    }

    /// Give the pull requests section its rows: the section is listed
    /// while they say so, with their count.
    pub fn set_pulls(&mut self, pulls: Entity<PullsView>, cx: &mut Context<Self>) {
        self._subscriptions
            .push(cx.subscribe(&pulls, |this, _, event: &PullsEvent, cx| {
                if matches!(event, PullsEvent::Counted) {
                    this.rebuild(cx);
                }
            }));
        self.pulls = Some(pulls);
        self.rebuild(cx);
    }

    /// Give the issues section its rows: the section is listed while they
    /// say so, with their count.
    pub fn set_issues(&mut self, issues: Entity<IssuesView>, cx: &mut Context<Self>) {
        self._subscriptions
            .push(cx.subscribe(&issues, |this, _, event: &IssuesEvent, cx| {
                if matches!(event, IssuesEvent::Counted) {
                    this.rebuild(cx);
                }
            }));
        self.issues = Some(issues);
        self.rebuild(cx);
    }

    /// Replace the snapshot.
    pub fn set_snapshot(&mut self, snapshot: Snapshot, cx: &mut Context<Self>) {
        if self.snapshot.as_ref() == Some(&snapshot) {
            return;
        }
        self.snapshot = Some(snapshot);
        self.rebuild(cx);
    }

    /// How far the local branches are ahead of their upstreams and behind
    /// them, by short name.
    pub fn set_tracking(&mut self, tracking: HashMap<String, (u32, u32)>, cx: &mut Context<Self>) {
        if self.tracking != tracking {
            self.tracking = tracking;
            cx.notify();
        }
    }

    /// How many files are changed in each worktree, by its path.
    pub fn set_worktree_changes(
        &mut self,
        changes: HashMap<String, usize>,
        cx: &mut Context<Self>,
    ) {
        if self.worktree_changes != changes {
            self.worktree_changes = changes;
            cx.notify();
        }
    }

    /// The worktrees being removed, by path: their rows are greyed and say
    /// so, and offer nothing.
    pub fn set_removing(&mut self, removing: BTreeSet<String>, cx: &mut Context<Self>) {
        if self.removing != removing {
            self.removing = removing;
            cx.notify();
        }
    }

    /// Whether anything is changed in the worktree this tab shows.
    pub fn set_here_changed(&mut self, changed: bool, cx: &mut Context<Self>) {
        if self.here_changed != changed {
            self.here_changed = changed;
            cx.notify();
        }
    }

    /// Where each submodule stands, by its path.
    pub fn set_submodule_states(
        &mut self,
        states: HashMap<String, (String, bool, bool)>,
        cx: &mut Context<Self>,
    ) {
        if self.submodule_states != states {
            self.submodule_states = states;
            cx.notify();
        }
    }

    /// The graph as it is shown now: what is in it decides what can be
    /// pinned, and it knows when each commit was made.
    pub fn set_graph(&mut self, graph: Arc<GraphData>, cx: &mut Context<Self>) {
        let same = self
            .graph
            .as_ref()
            .is_some_and(|known| Arc::ptr_eq(known, &graph));
        if !same {
            self.graph = Some(graph);
            cx.notify();
        }
    }

    /// Gitflow as it is set up here, and the branches it has.
    pub fn set_flow(&mut self, flow: FlowState, cx: &mut Context<Self>) {
        if self.flow.as_ref() != Some(&flow) {
            self.flow = Some(flow);
            self.rebuild(cx);
        }
    }

    /// What there is to say about local branches: a description, an old
    /// last commit, an upstream that is gone.
    pub fn set_notes(&mut self, notes: HashMap<String, BranchNote>, cx: &mut Context<Self>) {
        if self.notes != notes {
            self.notes = notes;
            // A row with a description is higher than one without.
            self.size_rows();
            cx.notify();
        }
    }

    /// Work out how high each row is.
    fn size_rows(&mut self) {
        for section in &mut self.sections {
            let heights: Vec<f32> = section
                .items
                .iter()
                .map(|item| height_of(&self.notes, item))
                .collect();
            section.sizes = crate::zoom::Sizes::new(
                heights
                    .into_iter()
                    .map(|height| size(px(0.), px(height)))
                    .collect(),
            );
        }
    }

    /// Whether the local branches are listed open.
    pub fn local_open(&self) -> bool {
        self.shows_rows("local") && !self.hidden_sections.contains("local")
    }

    /// Which local branches would conflict with the checked-out one, and
    /// in how many files.
    pub fn set_conflicts(&mut self, conflicts: HashMap<String, usize>, cx: &mut Context<Self>) {
        if self.conflicts != conflicts {
            self.conflicts = conflicts;
            cx.notify();
        }
    }

    /// Whether the worktrees are listed open, so that counts for them are
    /// worth asking for.
    pub fn worktrees_open(&self) -> bool {
        self.shows_rows("worktrees") && !self.hidden_sections.contains("worktrees")
    }

    /// Show the strip instead of the panel while something else needs the
    /// room, whatever the preference says.
    pub fn force_fold(&mut self, fold: bool, cx: &mut Context<Self>) {
        if self.forced_fold != fold {
            self.forced_fold = fold;
            self.peek = None;
            cx.notify();
        }
    }

    /// Show the strip instead of the panel while the window is too narrow
    /// for it beside the graph and the right panel, whatever the
    /// preference says; the panel comes back when there is room.
    pub fn fold_for_width(&mut self, fold: bool, cx: &mut Context<Self>) {
        if self.narrow_fold != fold {
            self.narrow_fold = fold;
            self.peek = None;
            cx.notify();
        }
    }

    /// Whether the panel is a strip because the window is narrow.
    pub fn folded_for_width(&self) -> bool {
        self.narrow_fold
    }

    /// Put the cursor in the filter.
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Give the panel the keyboard: it has no keys of its own, so they go
    /// to its filter.
    pub fn focus_rows(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_filter(window, cx);
    }

    /// Whether the rows of a section show: it is open.
    fn shows_rows(&self, key: &str) -> bool {
        !self.collapsed.contains(key)
    }

    /// Every reference the panel shows, in order, by full name.
    fn shown(&self) -> Vec<String> {
        self.sections
            .iter()
            .filter(|section| self.shows_rows(section.key))
            .flat_map(|section| section.items.iter())
            .filter(|item| !item.full_name.is_empty())
            .map(|item| item.full_name.clone())
            .collect()
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let started = std::time::Instant::now();
        let needle = Needle::new(&self.filter_text);
        let built = match &self.snapshot {
            Some(snapshot) => {
                // Rows are built for what shows them: a section that is
                // closed or hidden is counted.
                let wanted =
                    |key: &str| !self.hidden_sections.contains(key) && self.shows_rows(key);
                build_sections(
                    snapshot,
                    &Shape {
                        needle: &needle,
                        flow: self.flow.as_ref(),
                        closed: &self.closed_folders,
                        wanted: &wanted,
                    },
                )
            }
            None => Vec::new(),
        };
        let mut by_key: HashMap<&'static str, rows::SectionRows> = built
            .into_iter()
            .map(|section| (section.key, section))
            .collect();
        let mut sections = Vec::new();
        for key in &self.order {
            if self.hidden_sections.contains(*key) {
                continue;
            }
            // The pull requests are not references: their rows are their
            // own view's, which says whether the section is listed.
            if *key == "pulls" {
                let count = self
                    .pulls
                    .as_ref()
                    .and_then(|pulls| pulls.read(cx).heading_count());
                if let Some(count) = count {
                    sections.push(Section {
                        key,
                        title: say!("refs-section-pulls"),
                        count,
                        sizes: crate::zoom::Sizes::default(),
                        items: Rc::new(Vec::new()),
                        scroll: self
                            .scrolls
                            .entry(key)
                            .or_insert_with(VirtualListScrollHandle::new)
                            .clone(),
                    });
                }
                continue;
            }
            if *key == "issues" {
                let count = self
                    .issues
                    .as_ref()
                    .and_then(|issues| issues.read(cx).heading_count());
                // Named after the provider whose issues it shows, once a
                // tracker is chosen.
                let title = match self
                    .issues
                    .as_ref()
                    .and_then(|issues| issues.read(cx).provider())
                {
                    Some(bitbraid_integrations::model::ProviderKind::Github) => {
                        say!("refs-section-github-issues")
                    }
                    Some(bitbraid_integrations::model::ProviderKind::Gitlab) => {
                        say!("refs-section-gitlab-issues")
                    }
                    None => say!("refs-section-issues"),
                };
                if let Some(count) = count {
                    sections.push(Section {
                        key,
                        title,
                        count,
                        sizes: crate::zoom::Sizes::default(),
                        items: Rc::new(Vec::new()),
                        scroll: self
                            .scrolls
                            .entry(key)
                            .or_insert_with(VirtualListScrollHandle::new)
                            .clone(),
                    });
                }
                continue;
            }
            let Some(section) = by_key.remove(key) else {
                continue;
            };
            // Stashes, tags and submodules are not shown while there are
            // none.
            if section.total == 0 && matches!(section.key, "stashes" | "tags" | "submodules") {
                continue;
            }
            sections.push(Section {
                key: section.key,
                title: section.title,
                count: section.count,
                // Worked out below, once the rows are in place.
                sizes: crate::zoom::Sizes::default(),
                items: Rc::new(section.items),
                scroll: self
                    .scrolls
                    .entry(section.key)
                    .or_insert_with(VirtualListScrollHandle::new)
                    .clone(),
            });
        }
        self.sections = sections;
        self.size_rows();
        // What is no longer shown is no longer selected.
        let shown: BTreeSet<String> = self.shown().into_iter().collect();
        self.selected.retain(|name| shown.contains(name));
        if self
            .anchor
            .as_ref()
            .is_some_and(|name| !shown.contains(name))
        {
            self.anchor = None;
        }
        let rows: usize = self.sections.iter().map(|s| s.items.len()).sum();
        crate::session::debug(|| {
            format!(
                "refs panel: {rows} rows of {} references built in {:?}",
                self.sections.iter().map(|s| s.count).sum::<usize>(),
                started.elapsed()
            )
        });
        cx.notify();
    }

    /// A reference was clicked: update the selection.
    fn select(&mut self, name: &str, modifiers: Modifiers, cx: &mut Context<Self>) {
        let shown = self.shown();
        let order: Vec<&str> = shown.iter().map(String::as_str).collect();
        clicked(
            &mut self.selected,
            &mut self.anchor,
            &order,
            name,
            modifiers.secondary(),
            modifiers.shift,
        );
        cx.notify();
    }

    /// The selected local branches by short name, without the checked-out
    /// one and, when `deletable`, without protected ones.
    fn selected_branches(&self, deletable: bool, cx: &App) -> Vec<String> {
        let protected = &cx.global::<Prefs>().preferences.protected_branches;
        let head = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.head.branch.clone());
        self.selected
            .iter()
            .filter_map(|full| match RefKind::of(full) {
                RefKind::Local(name) => Some(name),
                _ => None,
            })
            .filter(|name| !deletable || head.as_deref() != Some(name.as_str()))
            .filter(|name| !deletable || !protected.contains(name))
            .collect()
    }

    fn run_batch(&mut self, what: Batch, cx: &mut Context<Self>) {
        match what {
            Batch::Delete => {
                let names = self.selected_branches(true, cx);
                if !names.is_empty() {
                    cx.emit(RefsEvent::DeleteMany(names));
                }
            }
            Batch::Push => {
                let names = self.selected_branches(false, cx);
                if !names.is_empty() {
                    cx.emit(RefsEvent::PushMany(names));
                }
            }
            Batch::Hide | Batch::Show => {
                let names: Vec<String> = self.selected.iter().cloned().collect();
                let hide = what == Batch::Hide;
                self.change_visibility(
                    move |state| {
                        state.hidden.retain(|hidden| !names.contains(hidden));
                        if hide {
                            state.hidden.extend(names);
                        }
                    },
                    cx,
                );
            }
            // Only these, in the graph.
            Batch::Solo => {
                let names: Vec<String> = self.selected.iter().cloned().collect();
                self.change_visibility(move |state| state.only = names, cx);
            }
            Batch::Clear => {
                self.selected.clear();
                self.anchor = None;
                cx.notify();
            }
        }
    }

    /// Remember how the panel is laid out, for the next time this
    /// repository is opened: which sections and folders are closed, the
    /// order, what is hidden and how high the lists want to be.
    fn remember_layout(&self, cx: &mut Context<Self>) {
        let closed: Vec<String> = self.collapsed.iter().cloned().collect();
        let folders: Vec<String> = self.closed_folders.iter().cloned().collect();
        let order: Vec<String> = self.order.iter().map(|key| (*key).to_owned()).collect();
        let hidden: Vec<String> = self.hidden_sections.iter().cloned().collect();
        let sizes: BTreeMap<String, f32> = self
            .sizes
            .iter()
            .map(|(key, size)| ((*key).to_owned(), *size))
            .collect();
        let path = self.session.path.clone();
        cx.global_mut::<Prefs>().update_repo_state(&path, |state| {
            state.closed_sections = Some(closed);
            state.closed_folders = folders;
            state.section_order = order;
            state.hidden_sections = hidden;
            state.section_sizes = sizes;
            // Giving one section all the room closes the others: nothing
            // is remembered as maximised.
            state.maximised_section = None;
        });
    }

    /// The layout changed in a way that changes which rows there are.
    fn layout_changed(&mut self, cx: &mut Context<Self>) {
        self.remember_layout(cx);
        self.rebuild(cx);
        // Counting is a `git status` in each worktree, which is only worth
        // it while they are looked at, and is asked for when they come
        // into view: not again for every folder or section that is opened
        // while they show.
        let shown = self.worktrees_open();
        if shown && !self.worktrees_shown {
            cx.emit(RefsEvent::CountWorktrees);
        }
        self.worktrees_shown = shown;
        let local = self.local_open();
        if local && !self.local_shown {
            cx.emit(RefsEvent::LocalShown);
        }
        self.local_shown = local;
    }

    /// Open or close a section: a list that closes gives its room to the
    /// open list under it, one that opens takes it back from there
    /// (`refs/heights.rs`).
    fn toggle_section(&mut self, key: &'static str, cx: &mut Context<Self>) {
        let lists = self.layout();
        let heights: Vec<f32> = lists.iter().map(|(_, height)| *height).collect();
        if let Some(at) = lists.iter().position(|(open, _)| *open == key) {
            self.collapsed.insert(key.to_owned());
            let staying: Vec<&'static str> = lists
                .iter()
                .map(|(open, _)| *open)
                .filter(|open| *open != key)
                .collect();
            if let Some((under, want)) = on_close(&heights, at)
                && let Some(under) = staying.get(under)
            {
                self.sizes.insert(under, want);
            }
        } else {
            // Where it comes among the open lists: after those before it.
            let place = |key: &str| self.sections.iter().position(|section| section.key == key);
            let at = lists
                .iter()
                .filter(|(open, _)| place(open) < place(key))
                .count();
            if let Some((under, want)) = on_open(&heights, at, self.sizes.get(key).copied())
                && let Some((under, _)) = lists.get(under)
            {
                self.sizes.insert(under, want);
            }
            self.collapsed.remove(key);
        }
        self.layout_changed(cx);
    }

    /// Open or close a folder, or the row of a remote.
    fn toggle_folder(&mut self, key: &str, cx: &mut Context<Self>) {
        if !self.closed_folders.remove(key) && self.closed_folders.len() < MOST_FOLDERS {
            self.closed_folders.insert(key.to_owned());
        }
        self.layout_changed(cx);
    }

    /// What a heading's menu asked for.
    fn section_do(&mut self, key: &str, what: SectionVerb, cx: &mut Context<Self>) {
        let Some(key) = known_section(key) else {
            return;
        };
        match what {
            SectionVerb::HideAll | SectionVerb::ShowAll => {
                let Some(space) = section_space(key) else {
                    return;
                };
                let hide = what == SectionVerb::HideAll;
                return self.change_visibility(
                    |state| {
                        state.hidden.retain(|name| !name.starts_with(space));
                        if hide {
                            state.hidden.push(space.to_owned());
                        }
                    },
                    cx,
                );
            }
            // All the room: every other section closes, and keeps the
            // height it wants for when it is opened again.
            SectionVerb::Maximise => {
                self.collapsed.remove(key);
                for section in &self.sections {
                    if section.key != key {
                        self.collapsed.insert(section.key.to_owned());
                    }
                }
            }
            SectionVerb::MoveUp | SectionVerb::MoveDown => {
                let hidden = &self.hidden_sections;
                self.order = nudged(&self.order, key, what == SectionVerb::MoveDown, |key| {
                    !hidden.contains(key)
                });
            }
            SectionVerb::CloseFolders => {
                // Every folder that shows now; what is inside them is
                // closed with them, and stays as it was underneath.
                let showing: Vec<String> = self
                    .sections
                    .iter()
                    .filter(|section| section.key == key)
                    .flat_map(|section| section.items.iter())
                    .filter_map(|item| item.folder.as_ref())
                    .filter(|folder| folder.open)
                    .map(|folder| folder.key.clone())
                    .collect();
                for folder in showing {
                    if self.closed_folders.len() < MOST_FOLDERS {
                        self.closed_folders.insert(folder);
                    }
                }
            }
            SectionVerb::OpenFolders => {
                let own = format!("{key}:");
                self.closed_folders
                    .retain(|folder| !folder.starts_with(&own));
            }
        }
        self.layout_changed(cx);
    }

    /// Put a section in front of another, or at the end.
    fn move_section(&mut self, moved: &str, before: Option<&str>, cx: &mut Context<Self>) {
        self.order = moved_before(&self.order, moved, before);
        self.layout_changed(cx);
    }

    /// The edge between two open lists is being dragged: the pointer is
    /// where the upper one should end. Every open list wants the height it
    /// has after that (`refs/heights.rs`).
    fn drag_split(&mut self, split: SectionSplit, y: Pixels, cx: &mut Context<Self>) {
        let lists = self.layout();
        let Some(below) = lists.iter().position(|(key, _)| *key == split.below) else {
            return;
        };
        let Some((_, height)) = below.checked_sub(1).and_then(|above| lists.get(above)) else {
            return;
        };
        let Some(above) = self
            .sections
            .iter()
            .find(|section| section.key == split.above)
        else {
            return;
        };
        let top = crate::zoom::unpx(above.scroll.bounds().origin.y);
        let wanted = crate::zoom::unpx(y) - top - HEAD_HEIGHT * split.between as f32;
        let heights: Vec<f32> = lists.iter().map(|(_, height)| *height).collect();
        let heights = dragged(&heights, below, wanted - height);
        let mut sizes = self.sizes.clone();
        for ((key, _), height) in lists.iter().zip(heights) {
            sizes.insert(key, height);
        }
        if self.sizes != sizes {
            self.sizes = sizes;
            self.remember_layout(cx);
            cx.notify();
        }
    }

    /// For the off-screen tests: the section the folded strip shows in a
    /// box beside it.
    pub fn peeking(&self) -> Option<&'static str> {
        self.peek
    }

    /// For the off-screen tests: how many of a section's references the
    /// graph shows and how many there are, while that is not all.
    pub fn seen_of(&self, key: &str, cx: &App) -> Option<(usize, usize)> {
        let state = cx.global::<Prefs>().repo_state(&self.session.path);
        let snapshot = self.snapshot.as_ref()?;
        rows::sections_seen(snapshot, &state.hidden, &state.only)
            .get(key)
            .copied()
    }

    /// For the off-screen tests: what the row of the worktree at `path`
    /// says at its right, and whether it opens; none while it has no row.
    pub fn worktree_said(&self, path: &str) -> Option<(Option<String>, bool)> {
        let item = self
            .sections
            .iter()
            .flat_map(|section| section.items.iter())
            .find(|item| matches!(&item.place, Place::Worktree { path: at, .. } if at == path))?;
        Some((self.aside(item), self.opens(item).is_some()))
    }

    /// For the off-screen tests: the sections on show, top to bottom, each
    /// with where the middle of its heading was drawn last (in the window,
    /// for a click) and how high its list is (none while it is closed).
    pub fn sections_shown(&self) -> Vec<(&'static str, Point<Pixels>, Option<f32>)> {
        let lists: HashMap<&'static str, f32> = self.layout().into_iter().collect();
        let heads = self.head_bounds.borrow();
        self.sections
            .iter()
            .map(|section| {
                let head = heads
                    .get(section.key)
                    .map(|head| head.center())
                    .unwrap_or_default();
                (section.key, head, lists.get(section.key).copied())
            })
            .collect()
    }

    /// For the off-screen tests: what the row of a reference (by its full
    /// name) says at its right and in its tooltip, when it has a row.
    pub fn row_said(&self, full_name: &str) -> Option<(Option<String>, Option<String>)> {
        let item = self
            .sections
            .iter()
            .flat_map(|section| section.items.iter())
            .find(|item| item.folder.is_none() && item.full_name == full_name)?;
        Some((self.aside(item), self.tip(item)))
    }

    /// The open lists, top to bottom, and how high each is drawn: what
    /// they share is the room of the sections less their headings.
    fn layout(&self) -> Vec<(&'static str, f32)> {
        let open: Vec<&'static str> = self
            .sections
            .iter()
            .map(|section| section.key)
            .filter(|key| self.shows_rows(key))
            .collect();
        let wants: Vec<Option<f32>> = open
            .iter()
            .map(|key| self.sizes.get(key).copied())
            .collect();
        let room = self.room.get() - HEAD_HEIGHT * self.sections.len() as f32;
        open.into_iter().zip(laid_out(room, &wants)).collect()
    }

    /// Move a pinned reference one lane to the left or to the right.
    fn move_pinned(&mut self, full_name: &str, right: bool, cx: &mut Context<Self>) {
        let name = full_name.to_owned();
        self.change_visibility(
            |state| {
                let _ = pin_moved(&mut state.pinned, &name, right);
            },
            cx,
        );
    }

    /// What two clicks do on a row that is no reference: open
    /// the worktree or the submodule, when there is one to open.
    fn opens(&self, item: &Item) -> Option<RefsEvent> {
        match &item.place {
            Place::Worktree { path, row } if !row.current && !self.removing.contains(path) => {
                Some(RefsEvent::Worktree(WorktreeRequest::Open(path.clone())))
            }
            Place::Submodule { path }
                if self
                    .submodule_states
                    .get(path)
                    .is_some_and(|(state, _, _)| state != "not_initialised") =>
            {
                Some(RefsEvent::Submodule(SubmoduleRequest::Open(path.clone())))
            }
            _ => None,
        }
    }

    /// The menu of a heading: how the sections are laid out, which is the
    /// view's own, and then what is added from that heading, which is not
    /// offered in a read-only repository.
    fn head_menu(&self, key: &'static str, cx: &App) -> MenuBuilder {
        // Giving it all the room is greyed where it has it already.
        let alone = self.shows_rows(key)
            && self
                .sections
                .iter()
                .all(|section| section.key == key || !self.shows_rows(section.key));
        let has_folders = matches!(key, "local" | "remote" | "tags");
        let read_only = self.read_only;
        let flow_initialized = self.flow.as_ref().is_some_and(|flow| flow.initialized);
        // Whether any of the section's references is hidden, so that they
        // can all be shown again.
        let some_hidden = section_space(key).is_some_and(|space| {
            cx.global::<Prefs>()
                .repo_state(&self.session.path)
                .hidden
                .iter()
                .any(|name| name.starts_with(space))
        });
        // Every section there can be, and whether it is shown: gitflow's
        // only where gitflow is set up.
        let entries: Vec<(&'static str, &'static str, bool)> = self
            .order
            .iter()
            .filter(|known| **known != "gitflow" || flow_initialized)
            .filter_map(|known| {
                let (_, title) = SECTIONS.iter().find(|(k, _)| k == known)?;
                Some((*known, title.text(), !self.hidden_sections.contains(*known)))
            })
            .collect();
        Rc::new(move |menu, _, _| {
            let run = |what| -> Box<dyn Action> {
                Box::new(SectionDo {
                    key: key.to_owned(),
                    what,
                })
            };
            // In groups: what the graph shows of the section's references;
            // all the room for it; which sections are shown (the branches
            // and gitflow always are); their order, the folders, and what
            // is added from that heading.
            let menu = if section_space(key).is_some() {
                menu.menu(say!("refs-section-hide-all"), run(SectionVerb::HideAll))
                    .menu_with_disabled(
                        say!("refs-section-show-all"),
                        run(SectionVerb::ShowAll),
                        !some_hidden,
                    )
                    .separator()
            } else {
                menu
            };
            let mut menu = menu
                .menu_with_disabled(
                    say!("refs-section-maximise"),
                    run(SectionVerb::Maximise),
                    alone,
                )
                .separator();
            for (known, title, shown) in &entries {
                menu = menu.menu_with_check_and_disabled(
                    title_case(title),
                    *shown,
                    Box::new(ToggleSection {
                        key: (*known).to_owned(),
                    }),
                    matches!(*known, "local" | "gitflow"),
                );
            }
            let menu = menu
                .menu(say!("refs-section-reset"), Box::new(ResetSections))
                .separator()
                .menu(say!("refs-move-up"), run(SectionVerb::MoveUp))
                .menu(say!("refs-move-down"), run(SectionVerb::MoveDown));
            let menu = if has_folders {
                menu.separator()
                    .menu(say!("refs-folders-close"), run(SectionVerb::CloseFolders))
                    .menu(say!("refs-folders-open"), run(SectionVerb::OpenFolders))
            } else {
                menu
            };
            if read_only {
                return menu;
            }
            // Remotes, worktrees and submodules are added from the heading
            // they will appear under.
            match key {
                "remote" => menu.separator().menu(
                    say!("refs-add-remote"),
                    Box::new(RunRemote {
                        request: RemoteRequest::Add,
                    }),
                ),
                "worktrees" => worktrees::new_worktree_entry(menu.separator(), None).menu(
                    say!("worktrees-prune"),
                    Box::new(RunWorktree {
                        request: WorktreeRequest::Prune,
                    }),
                ),
                "submodules" => submodules::all_entries(menu.separator()),
                "gitflow" => flow::heading_entries(menu.separator(), flow_initialized),
                "local" => flow::delete_merged_entry(menu.separator()),
                "tags" => tags::push_all_entry(menu.separator()),
                _ => menu,
            }
        })
    }

    /// The heading of a section: a chevron, its mark, its name and how
    /// many references it has. `split` is the edge over it that can be
    /// dragged, when there is a list above and one below to trade height.
    fn render_head(
        &self,
        section: &Section,
        split: Option<SectionSplit>,
        seen: Option<(usize, usize)>,
        // The sections as a whole scroll: their scroll bar has the room at
        // the right end, the count stands clear of it.
        scrolling: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let open = self.shows_rows(section.key);
        let key = section.key;
        let number = section_number(key);
        // Where the count ends: 11 short of the panel's edge; 6 further
        // in while the sections' scroll bar stands there.
        let head_end = if scrolling { 17. } else { 11. };
        let filtering = !self.filter_text.trim().is_empty();
        let bounds = self.head_bounds.clone();
        let accent = theme.accent;
        let builder = self.head_menu(key, cx);
        // What is added from the heading, as buttons in the count's place
        // while the pointer is on it.
        let adds = if self.read_only {
            Vec::new()
        } else {
            head_buttons(key)
        };
        let has_adds = !adds.is_empty();
        let bright = theme.foreground;
        h_flex()
            .id(("refs-head", number))
            .test_support()
            // A heading that opens and closes its section.
            .role(Role::Button)
            .aria_label(crate::access::section_heading(section.title, section.count))
            .aria_expanded(open)
            .group(HEAD_GROUP)
            .relative()
            .w_full()
            .h(px(HEAD_HEIGHT))
            .flex_none()
            .pl(px(6.))
            .pr(px(head_end))
            .gap(px(5.))
            .items_center()
            .cursor_pointer()
            .border_t_1()
            .border_color(theme.sidebar_border)
            .text_color(theme.muted_foreground)
            // No fill under the pointer: the words brighten.
            .hover(move |style| style.text_color(bright))
            .on_click(cx.listener(move |this, _, _, cx| this.toggle_section(key, cx)))
            // A heading is dragged onto another to stand in front of it, or
            // onto the lower half of it to stand after it.
            .on_drag(DraggedSection(key), |dragged, _, _, cx| {
                let title = SECTIONS
                    .iter()
                    .find(|(known, _)| *known == dragged.0)
                    .map_or("", |(_, title)| title.text());
                cx.new(|_| DragChip(title.to_owned()))
            })
            .drag_over::<DraggedSection>(move |style, _, _, _| style.bg(accent))
            .on_drop(
                cx.listener(move |this, dragged: &DraggedSection, window, cx| {
                    let after = this
                        .head_bounds
                        .borrow()
                        .get(key)
                        .is_some_and(|head| window.mouse_position().y > head.center().y);
                    let before = if after {
                        this.order
                            .iter()
                            .skip_while(|known| **known != key)
                            .nth(1)
                            .copied()
                    } else {
                        Some(key)
                    };
                    this.move_section(dragged.0, before, cx)
                }),
            )
            // Where it is, for a menu opened by key.
            .child(
                canvas(
                    move |head, _, _| {
                        bounds.borrow_mut().insert(key, head);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size_3p5()
                .flex_none(),
            )
            .child(
                // The provider's mark beside its issues.
                if key == "issues" && section.title == say!("refs-section-github-issues") {
                    Icon::new(IconName::Github)
                } else {
                    section_icon(key)
                }
                .size_4()
                .flex_none(),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.5))
                    .font_semibold()
                    .child(section.title),
            )
            .when(has_adds, |head| {
                // Over the count, out of the flow, so that the name keeps
                // its room while they are hidden.
                head.child(
                    h_flex()
                        .absolute()
                        .right(px(head_end))
                        .top_0()
                        .h_full()
                        .items_center()
                        .gap_1()
                        .invisible()
                        .group_hover(HEAD_GROUP, |style| style.visible())
                        .children(
                            adds.into_iter()
                                .enumerate()
                                .map(|(at, (icon, tip, asked))| {
                                    Button::new(("refs-add", number * 10 + at))
                                        .accessibility_label(tip)
                                        .outline()
                                        .xsmall()
                                        .icon(icon)
                                        .tooltip(tip)
                                        .disabled(asked.is_none())
                                        .on_click(cx.listener(move |_, _, _, cx| {
                                            cx.stop_propagation();
                                            if let Some(asked) = &asked {
                                                cx.emit(asked.clone());
                                            }
                                        }))
                                }),
                        ),
                )
            })
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.primary)
                    // The issues' heading has no count (outside a filter,
                    // which counts).
                    .when(key == "issues" && !filtering, |count| count.invisible())
                    // Where buttons stand in its place under the pointer.
                    .when(has_adds, |count| {
                        count.group_hover(HEAD_GROUP, |style| style.invisible())
                    })
                    // While filtering, a funnel and how many it found.
                    .when(filtering, |count| {
                        count
                            .flex()
                            .items_center()
                            .gap_0p5()
                            .child(Icon::new(gpui_kit::assets::IconName::ListFilter).size_3())
                    })
                    .child(if filtering {
                        format!("{}", section.count)
                    } else if let Some((shown, all)) = seen {
                        // What the graph shows of them, while that is
                        // not all.
                        format!("{shown}/{all}")
                    } else {
                        format!("{}", section.count)
                    }),
            )
            .children(split.map(|split| {
                let hover = theme.primary.opacity(0.6);
                div()
                    .id(("refs-split", number))
                    .absolute()
                    .top(px(-5.))
                    .left_0()
                    .w_full()
                    .h(px(10.))
                    .cursor_row_resize()
                    .hover(move |style| style.bg(hover))
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .on_drag(split, |_, _, _, cx| cx.new(|_| NoChip))
            }))
            .context_menu(move |menu, window, cx| {
                builder(crate::views::menu_fit::fit(menu, window), window, cx)
            })
            .into_any_element()
    }

    /// What stands at the right of a row.
    fn aside(&self, item: &Item) -> Option<String> {
        // A row a filter found: the folders it is in, which the flat list
        // has no rows for.
        if let Some(path) = &item.path {
            return Some(path.clone());
        }
        match RefKind::of(&item.full_name) {
            // How far it is ahead of what it follows and behind it, and
            // whether that is gone or the branch stale: the rest of what
            // there is to say about a branch is in its row's tooltip
            // (`tip`).
            RefKind::Local(name) => {
                let note = self.notes.get(&name);
                branch_aside(
                    self.tracking.get(&name).copied(),
                    note.is_some_and(|note| note.gone),
                    note.is_some_and(|note| note.stale),
                )
            }
            RefKind::Remote(_) => None,
            _ => match (&item.folder, &item.place) {
                // A closed folder says how much it holds.
                (Some(folder), _) if !folder.open => Some(format!("{}", folder.held)),
                (Some(_), _) => None,
                (None, Place::Worktree { path, .. }) if self.removing.contains(path) => {
                    Some(say!("refs-worktree-removing").to_owned())
                }
                // Of the one this tab shows, how far its branch is ahead
                // and behind, and a dot when something is changed there;
                // and that it is locked. Its folder is in its tooltip.
                (None, Place::Worktree { path, row }) => {
                    let track = row
                        .current
                        .then(|| self.tracking.get(&item.label))
                        .flatten()
                        .and_then(|(ahead, behind)| track_text(*ahead, *behind));
                    let changed = if row.current {
                        self.here_changed
                    } else {
                        self.worktree_changes
                            .get(path)
                            .is_some_and(|count| *count > 0)
                    };
                    let changed = changed.then(|| "●".to_owned());
                    let locked = row.locked.then(|| say!("refs-worktree-locked").to_owned());
                    let parts: Vec<String> =
                        [locked, track, changed].into_iter().flatten().collect();
                    (!parts.is_empty()).then(|| parts.join(" "))
                }
                // Where it stands when that is not where it should be; its
                // address is in its tooltip.
                (None, Place::Submodule { path }) => self
                    .submodule_states
                    .get(path)
                    .and_then(|(state, modified, untracked)| {
                        submodules::state_text(state, *modified, *untracked)
                    })
                    .map(str::to_owned),
                (None, Place::None) => item.hint.clone(),
            },
        }
    }

    /// What the tooltip of a row says, on every row but a folder: what it
    /// names in full, then what else there is to say of it: of a branch
    /// what the user wrote about it, a merge that would conflict, what it
    /// follows and how old its last commit is; of a tag its message; of a
    /// stash all it says; of a worktree its folder; of a submodule its
    /// address; of a remote its address.
    fn tip(&self, item: &Item) -> Option<String> {
        if item.folder.is_some() {
            return None;
        }
        if let Some(remote) = &item.remote {
            return Some(match &item.hint {
                Some(url) => format!("{remote}\n{url}"),
                None => remote.clone(),
            });
        }
        let with = |first: String, rest: Option<String>| match rest {
            Some(rest) => format!("{first}\n{rest}"),
            None => first,
        };
        let age = self.age(item.target);
        match (&item.place, RefKind::of(&item.full_name)) {
            // Another worktree's dot is of its tracked files alone: it is
            // looked at without the search for untracked files, which is
            // what costs most in a large one.
            (Place::Worktree { path, row }, _) if !row.current => Some(with(
                item.label.clone(),
                Some(format!("{path}\n{}", worktrees::TRACKED_ONLY.text())),
            )),
            (Place::Worktree { path, .. }, _) => Some(with(item.label.clone(), Some(path.clone()))),
            (Place::Submodule { path }, _) => Some(with(path.clone(), item.hint.clone())),
            (Place::None, RefKind::Local(name)) => {
                let note = self.notes.get(&name);
                let conflicts = self.conflicts.get(&name).map(|files| conflict_mark(*files));
                let rest = branch_tip(
                    note.and_then(|note| note.description.as_deref()),
                    conflicts.as_deref(),
                    item.hint.as_deref(),
                    note.is_some_and(|note| note.gone),
                    age.as_deref(),
                    note.is_some_and(|note| note.stale),
                );
                Some(with(name, rest))
            }
            (Place::None, RefKind::Remote(name)) => Some(with(
                name,
                branch_tip(None, None, None, false, age.as_deref(), false),
            )),
            (Place::None, RefKind::Tag(name)) => {
                let message = self.snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .tags
                        .iter()
                        .find(|tag| tag.name == name)
                        .and_then(|tag| tag.message.as_deref())
                        .map(str::trim)
                        .filter(|message| !message.is_empty())
                        .map(str::to_owned)
                });
                Some(with(name, message))
            }
            (Place::None, RefKind::Other) => {
                let stash = self.snapshot.as_ref().and_then(|snapshot| {
                    snapshot
                        .stashes
                        .iter()
                        .find(|stash| format!("stash@{{{}}}", stash.index) == item.full_name)
                        .map(|stash| stash.message.clone())
                });
                stash.or_else(|| Some(item.label.clone()))
            }
        }
    }

    /// The issue a branch's row names: the number its name holds, of the
    /// first remote the repository is linked to, and where the key stands
    /// in what the row shows. None for a row that is no branch, a name that
    /// holds no number, or nothing linked.
    fn issue_of(&self, item: &Item, cx: &App) -> Option<BranchIssue> {
        if item.folder.is_some() || item.place != Place::None {
            return None;
        }
        let name = match RefKind::of(&item.full_name) {
            RefKind::Local(name) | RefKind::Remote(name) => name,
            RefKind::Tag(_) | RefKind::Other => return None,
        };
        let links = crate::providers::drawn_links(cx);
        crate::views::issues::facts::branch_issue_in(&links, &name, &item.label)
    }

    /// A row's name: with the letters a filter found standing out, or
    /// with the key of the issue it holds drawn as a link that shows the
    /// issue when clicked; else as it is.
    fn name_text(
        &self,
        item: &Item,
        issue: Option<&BranchIssue>,
        same_twice: &str,
        view: WeakEntity<Self>,
        cx: &App,
    ) -> AnyElement {
        let keyed = issue
            .filter(|_| item.marks.is_empty())
            .and_then(|(link, number, at)| Some((link.clone(), *number, at.clone()?)));
        let Some((link, number, at)) = keyed else {
            return crate::fuzzy::marked(item.label.clone(), &item.marks, cx.theme().primary)
                .into_any_element();
        };
        let colour = cx.theme().link;
        let style = HighlightStyle {
            color: Some(colour),
            underline: Some(UnderlineStyle {
                thickness: px(1.),
                color: Some(colour),
                wavy: false,
            }),
            ..HighlightStyle::default()
        };
        let text = InteractiveText::new(
            ElementId::Name(format!("ref-key-text:{}{}", same_twice, item.full_name).into()),
            StyledText::new(item.label.clone()).with_highlights(vec![(at.clone(), style)]),
        )
        .on_click(vec![at], move |_, _, cx| {
            let link = link.clone();
            let _ = view.update(cx, |_, cx| cx.emit(RefsEvent::OpenIssue { link, number }));
        });
        div()
            .id(ElementId::Name(
                format!("ref-key:{}{}", same_twice, item.full_name).into(),
            ))
            .min_w_0()
            .truncate()
            .child(text)
            .test_support()
            .into_any_element()
    }

    /// How old a commit is, when it is in the graph: the walk that made
    /// the graph read its date, so this asks git nothing.
    fn age(&self, commit: Oid) -> Option<String> {
        let graph = self.graph.as_ref()?;
        let seconds = graph
            .index
            .get(&commit)
            .and_then(|row| graph.commits.get(*row))?
            .seconds?;
        age_text(seconds, crate::views::now())
    }

    /// Whether a row can be pinned, and what stands in the way.
    fn pin_of(&self, item: &Item, repo_state: &RepoState) -> Pin {
        match RefKind::of(&item.full_name) {
            _ if repo_state.pinned.contains(&item.full_name) => Pin::Pinned,
            RefKind::Local(_) | RefKind::Remote(_) => {
                let shown = self
                    .graph
                    .as_ref()
                    .is_none_or(|graph| graph.index.contains_key(&item.target));
                if !shown {
                    Pin::NotInGraph
                } else if repo_state.pinned.len() >= crate::prefs::MAX_PINNED {
                    Pin::Full
                } else {
                    Pin::Free
                }
            }
            RefKind::Tag(_) | RefKind::Other => Pin::NotOffered,
        }
    }

    /// The menu of a row; none for a folder.
    fn row_menu(
        &self,
        section: &str,
        item: &Item,
        repo_state: &RepoState,
        cx: &App,
    ) -> Option<MenuBuilder> {
        let read_only = self.read_only;
        // The row of a remote is where the remote itself is reached, and
        // where all its branches are hidden or shown alone at once.
        if let Some(name) = item.remote.clone() {
            let whole = remote_ref(&name);
            let hidden = repo_state.hidden.contains(&whole);
            let soloed = repo_state.only.contains(&whole);
            return Some(Rc::new(move |menu, _, _| {
                let run = |request| -> Box<dyn Action> { Box::new(RunRemote { request }) };
                // Fetch | edit, remove | hide, show alone | everything
                // shown again | another remote.
                let menu = if read_only {
                    crate::read_only::menu(menu).separator()
                } else {
                    menu.menu(
                        say!("refs-remote-fetch", name = name.as_str()),
                        run(RemoteRequest::Fetch(name.clone())),
                    )
                    .separator()
                    .menu(
                        say!("refs-remote-edit-urls"),
                        run(RemoteRequest::EditUrl(name.clone())),
                    )
                    .menu(
                        say!("refs-remote-remove", name = name.as_str()),
                        run(RemoteRequest::Remove(name.clone())),
                    )
                    .separator()
                };
                let menu = menu
                    .menu(
                        if hidden {
                            say!("refs-remote-show", name = name.as_str())
                        } else {
                            say!("refs-remote-hide", name = name.as_str())
                        },
                        Box::new(ToggleHidden {
                            full_name: whole.clone(),
                        }),
                    )
                    .menu(
                        if soloed {
                            say!("refs-remote-solo-stop", name = name.as_str())
                        } else {
                            say!("refs-remote-solo", name = name.as_str())
                        },
                        Box::new(ToggleSolo {
                            full_name: whole.clone(),
                        }),
                    )
                    .separator()
                    .menu(say!("refs-show-all"), Box::new(ShowAll));
                if read_only {
                    menu
                } else {
                    menu.separator()
                        .menu(say!("refs-add-remote"), run(RemoteRequest::Add))
                }
            }));
        }
        if let Some(folder) = &item.folder {
            return self.folder_menu(&folder.key, repo_state);
        }
        match &item.place {
            Place::Worktree { path, .. } if self.removing.contains(path) => {
                return Some(Rc::new(|menu, _, _| worktrees::removing_menu(menu)));
            }
            Place::Worktree { path, row: about } => {
                let (path, about) = (path.clone(), *about);
                return Some(Rc::new(move |menu, _, _| {
                    if read_only {
                        let menu = crate::read_only::menu(menu);
                        return if about.current {
                            menu
                        } else {
                            worktrees::open_worktree_entry(menu.separator(), &path)
                        };
                    }
                    worktrees::worktree_entries(menu, &path, about)
                }));
            }
            Place::Submodule { path } => {
                let there = self
                    .submodule_states
                    .get(path)
                    .is_none_or(|(state, _, _)| state != "not_initialised");
                let path = path.clone();
                return Some(Rc::new(move |menu, _, _| {
                    if read_only {
                        return crate::read_only::menu(menu);
                    }
                    submodules::submodule_entries(menu, &path, there)
                }));
            }
            Place::None => {}
        }
        let kind = RefKind::of(&item.full_name);
        let hidden = repo_state.hidden.contains(&item.full_name);
        let soloed = repo_state.only.contains(&item.full_name);
        let pin = self.pin_of(item, repo_state);
        let moves = pin_moves(&repo_state.pinned, &item.full_name);
        let target = item.target;
        let is_head = item.is_head;
        let protected = matches!(&kind, RefKind::Local(name)
            if cx.global::<Prefs>().preferences.protected_branches.contains(name));
        let head = self.snapshot.as_ref().map(|snapshot| &snapshot.head);
        let current = head.and_then(|head| head.branch.clone());
        let at_head = head.and_then(|head| head.target) == Some(target);
        let is_stash = item.full_name.starts_with("stash@{");
        let is_selected = self.selected.contains(&item.full_name);
        // With several selected, a menu on one of them is about all of them.
        let batch = (is_selected && self.selected.len() > 1).then(|| {
            (
                self.selected.len(),
                self.selected_branches(true, cx).len(),
                self.selected_branches(false, cx).len(),
            )
        });
        // What the branch is for, as its own menu says first.
        let description = match &kind {
            RefKind::Local(name) => self
                .notes
                .get(name)
                .and_then(|note| note.description.as_deref())
                .map(described_as),
            _ => None,
        };
        let menu_name = item.full_name.clone();
        let focus = focus_label(
            &self
                .snapshot
                .as_ref()
                .map(|snapshot| focused(snapshot, self.flow.as_ref()))
                .unwrap_or_default(),
        );
        // A tag that has no message of its own can be given one.
        let lightweight = match &kind {
            RefKind::Tag(name) => self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .tags
                    .iter()
                    .any(|tag| tag.name == *name && !tag.annotated)
            }),
            _ => false,
        };
        // A branch has a menu of its own, in groups (`branch_groups`).
        let branch = match &kind {
            RefKind::Local(name) | RefKind::Remote(name) | RefKind::Tag(name) => {
                let local = matches!(kind, RefKind::Local(_));
                Some(BranchRow {
                    name: name.clone(),
                    full_name: item.full_name.clone(),
                    local,
                    tag: matches!(kind, RefKind::Tag(_)).then_some(lightweight),
                    finish: (section == "gitflow" && local)
                        .then(|| {
                            self.flow.as_ref()?.branches.iter().find_map(|branch| {
                                (branch.branch == *name).then(|| (branch.kind, branch.name.clone()))
                            })
                        })
                        .flatten(),
                    commit: target.to_hex(),
                    merge: self.is_merge(target),
                    is_head,
                    at_head,
                    protected,
                    // What a local branch follows, for the entries that go
                    // between the two.
                    upstream: item.hint.clone().filter(|_| local),
                    described: description.is_some(),
                    current: current.clone(),
                    hidden,
                    soloed,
                    pin,
                    moves,
                    focus: focus.clone(),
                    pulls: self
                        .pulls
                        .as_ref()
                        .is_some_and(|pulls| !pulls.read(cx).links().is_empty()),
                    pull: self
                        .pulls
                        .as_ref()
                        .filter(|_| !matches!(kind, RefKind::Tag(_)))
                        .and_then(|pulls| pulls.read(cx).mark_of(name)),
                })
            }
            RefKind::Other => None,
        };
        Some(Rc::new(move |menu, window, cx| {
            // In a read-only repository: what the graph shows, which is
            // the view's own, and nothing else.
            if read_only {
                let menu = crate::read_only::menu(menu).separator();
                if let Some((all, _, _)) = batch {
                    let run = |what| -> Box<dyn Action> { Box::new(RunBatch { what }) };
                    return menu
                        .menu(say!("refs-batch-hide", count = all), run(Batch::Hide))
                        .menu(say!("refs-batch-show", count = all), run(Batch::Show))
                        .separator()
                        .menu(say!("refs-batch-select-none"), run(Batch::Clear));
                }
                return visibility_entries(menu, &menu_name, hidden, soloed, pin, moves, &focus);
            }
            if let Some((all, deletable, pushable)) = batch {
                let run = |what| -> Box<dyn Action> { Box::new(RunBatch { what }) };
                // Delete, push | hide, show only these, show all | select
                // none.
                return menu
                    .menu_with_disabled(
                        if deletable == 0 {
                            say!("refs-batch-delete-none").to_owned()
                        } else {
                            say!("refs-batch-delete", count = deletable)
                        },
                        run(Batch::Delete),
                        deletable == 0,
                    )
                    .menu_with_disabled(
                        if pushable == 0 {
                            say!("refs-batch-push-none").to_owned()
                        } else {
                            say!("refs-batch-push", count = pushable)
                        },
                        run(Batch::Push),
                        pushable == 0,
                    )
                    .separator()
                    .menu(say!("refs-batch-hide", count = all), run(Batch::Hide))
                    .menu(say!("refs-batch-solo", count = all), run(Batch::Solo))
                    .menu(say!("refs-batch-show", count = all), run(Batch::Show))
                    .separator()
                    .menu(say!("refs-batch-select-none"), run(Batch::Clear));
            }
            // A branch: its groups, under what the user wrote about it.
            // An entry that cannot run says why.
            if let Some(branch) = &branch {
                let menu = menu.when_some(description.clone(), |menu, text| {
                    menu.label(text).separator()
                });
                return branch.menu(menu, window, cx);
            }
            // A stash: what is done with it, then its message; then what
            // the graph shows.
            let menu = if is_stash {
                let id = target.to_hex();
                stashes::reword_entry(stashes::stash_entries(menu, &id).separator(), &id)
                    .separator()
            } else {
                menu
            };
            visibility_entries(menu, &menu_name, hidden, soloed, pin, moves, &focus)
        }))
    }

    /// The menu of a folder: for a folder of branches deleting the
    /// branches in it; then hiding all of it or showing it alone. None for
    /// a folder of no references.
    fn folder_menu(&self, key: &str, repo_state: &RepoState) -> Option<MenuBuilder> {
        let whole = rows::folder_ref(key)?;
        let hidden = repo_state.hidden.contains(&whole);
        let soloed = repo_state.only.contains(&whole);
        let read_only = self.read_only;
        let head = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.head.branch.clone());
        // The local branches in it that may be deleted: not the one
        // checked out.
        let branches: Vec<String> = match whole.strip_prefix("refs/heads/") {
            Some(prefix) => self
                .snapshot
                .as_ref()
                .map(|snapshot| {
                    snapshot
                        .branches
                        .iter()
                        .filter(|branch| branch.name.starts_with(prefix))
                        .filter(|branch| head.as_deref() != Some(branch.name.as_str()))
                        .map(|branch| branch.name.clone())
                        .collect()
                })
                .unwrap_or_default(),
            None => Vec::new(),
        };
        let local = whole.starts_with("refs/heads/");
        Some(Rc::new(move |menu, _, _| {
            let menu = if local && !read_only {
                let count = branches.len();
                menu.menu_with_disabled(
                    say!("refs-folder-delete", count),
                    Box::new(DeleteMany {
                        names: branches.clone(),
                    }),
                    count == 0,
                )
                .separator()
            } else {
                menu
            };
            menu.menu(
                if hidden {
                    say!("refs-folder-show")
                } else {
                    say!("refs-folder-hide")
                },
                Box::new(ToggleHidden {
                    full_name: whole.clone(),
                }),
            )
            .menu(
                if soloed {
                    say!("refs-solo-stop")
                } else {
                    say!("refs-folder-solo")
                },
                Box::new(ToggleSolo {
                    full_name: whole.clone(),
                }),
            )
        }))
    }

    /// Whether a commit is a merge commit, as far as the graph that is
    /// shown knows: one that is not in it counts as none.
    fn is_merge(&self, commit: Oid) -> bool {
        self.graph.as_ref().is_some_and(|graph| {
            graph
                .index
                .get(&commit)
                .and_then(|row| graph.commits.get(*row))
                .is_some_and(|commit| commit.parents.len() > 1)
        })
    }

    fn render_item(
        &self,
        section: &'static str,
        ix: usize,
        item: &Item,
        repo_state: &RepoState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let folder = item.folder.clone();
        let is_folder = folder.is_some();
        let aside = self.aside(item);
        let target = item.target;
        let kind = RefKind::of(&item.full_name);
        // What the graph shows of the row is changed from its left end: a
        // reference or a stash by its name, a remote's row for all its
        // branches, a folder for all that is in it. What is hidden or
        // shown alone with what holds it is greyed with it.
        let seen_name = match (&item.folder, &item.remote) {
            (Some(folder), _) => rows::folder_ref(&folder.key),
            (None, Some(remote)) => Some(remote_ref(remote)),
            (None, None)
                if item.place == Place::None
                    && (!matches!(kind, RefKind::Other)
                        || item.full_name.starts_with("stash@{")) =>
            {
                Some(item.full_name.clone())
            }
            (None, None) => None,
        };
        let is_named = |list: &[String]| {
            seen_name
                .as_ref()
                .is_some_and(|name| list.contains(name) || under_remote(list, name))
        };
        let hidden_itself = seen_name
            .as_ref()
            .is_some_and(|name| repo_state.hidden.contains(name));
        let hidden = is_named(&repo_state.hidden);
        let soloing = !repo_state.only.is_empty() && seen_name.is_some();
        let soloed = is_named(&repo_state.only);
        let removing =
            matches!(&item.place, Place::Worktree { path, .. } if self.removing.contains(path));
        let greyed = hidden || (soloing && !soloed) || removing;
        let checkout_target = kind.target().map(str::to_owned);
        // A plain click leaves no mark on its row: the rows picked with
        // the toggle or range key do.
        let is_selected =
            !is_folder && self.selected.len() > 1 && self.selected.contains(&item.full_name);
        let opens = self.opens(item);
        let read_only = self.read_only;
        let id = item.id();
        let is_stash_row = item.full_name.starts_with("stash@{");
        // A branch of gitflow's has a row there and one among the branches:
        // the one in gitflow's section is known apart.
        let same_twice = if section == "gitflow" { "gitflow:" } else { "" };
        // A submodule that is where it should be has a tick.
        let submodule_fine = match &item.place {
            Place::Submodule { path } => {
                self.submodule_states
                    .get(path)
                    .is_some_and(|(state, modified, untracked)| {
                        submodules::state_text(state, *modified, *untracked).is_none()
                    })
            }
            _ => false,
        };
        let slot = self.seen_slot(
            (section, ix),
            item.is_head || submodule_fine,
            seen_name.clone(),
            (hidden, hidden_itself),
            (soloing, soloed),
            cx,
        );
        let this_view = cx.entity().downgrade();
        let theme = cx.theme();
        let theme_accent = theme.accent;
        // What the user wrote about the branch stands under its name; all
        // of it, and what else there is to say about a branch, is said
        // when the pointer rests on the row.
        let description = description_of(&self.notes, item).map(str::to_owned);
        // How far a branch is ahead and behind is short and stays whole
        // beside a name that is cut; anything else at the right of a row
        // gets what the name leaves.
        let aside_stays = item.path.is_none() && matches!(kind, RefKind::Local(_));
        // A tree item for assistive technology: the name, said with what
        // the pointer would show.
        let spoken = if item.is_head {
            say!("refs-row-checked-out", name = item.label.as_str())
        } else {
            item.label.clone()
        };
        let described = self
            .tip(item)
            .map(|tip| tip.lines().collect::<Vec<_>>().join(", "));
        // The issue a branch's name holds, of the remote the repository is
        // linked to: its key in the name is a link, and the row's tooltip
        // carries the issue's card, as the graph's labels do.
        let issue = self.issue_of(item, cx);
        let tip = self.tip(item);
        let row = h_flex()
            .id((section, ix))
            .test_support()
            .role(Role::TreeItem)
            .aria_label(spoken)
            .when_some(described, |row, text| row.aria_description(text))
            .aria_selected(is_selected)
            .when_some(
                item.folder.as_ref().map(|folder| folder.open),
                |row, open| row.aria_expanded(open),
            )
            .group(ROW_GROUP)
            .w_full()
            .h(px(height_of(&self.notes, item)))
            .map(|row| match (tip, issue.clone()) {
                (tip, Some((link, number, _))) => row.tooltip(move |window, cx| {
                    let (tip, link) = (tip.clone(), link.clone());
                    Tooltip::element(move |_, cx| {
                        v_flex()
                            .gap_1()
                            .children(
                                tip.iter()
                                    .flat_map(|tip| tip.lines())
                                    .map(|line| div().child(line.to_owned())),
                            )
                            .child(crate::views::issues::card::key_card(&link, number, cx))
                    })
                    .build(window, cx)
                }),
                (Some(text), None) => {
                    row.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
                }
                (None, None) => row,
            })
            .pl(px(6.))
            .pr(px(ROW_END))
            .gap(px(ROW_GAP))
            .items_center()
            // With a line under the name, the name and what stands beside
            // it keep to the row's first line, and the line under them
            // begins where the name begins and takes no room from them.
            .when_some(description.clone(), |row, text| {
                row.relative().pb(px(DESCRIBED)).child(
                    div()
                        .absolute()
                        .left(px(NAME_AT + INDENT * item.indent as f32))
                        .right(px(12.))
                        .bottom(px(2.))
                        .text_xs()
                        .line_height(px(DESCRIBED))
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child(described_as(&text)),
                )
            })
            .text_size(px(13.))
            .cursor_pointer()
            .hover(|s| s.bg(theme.accent))
            .when(greyed, |row| row.text_color(theme.muted_foreground))
            // The checked-out branch stands out as a whole row, and so do
            // the references shown alone while anything is.
            .when(item.is_head, |row| row.bg(theme.success.opacity(0.22)))
            .when(soloing && soloed, |row| row.bg(theme.warning.opacity(0.18)))
            .when(is_selected, |row| row.bg(theme.list_active))
            // The graph lights the history of the branch or tag the pointer
            // is on.
            .when(!is_folder && !matches!(kind, RefKind::Other), |row| {
                row.on_hover(cx.listener(move |_, hovering: &bool, _, cx| {
                    cx.emit(RefsEvent::Hover(target, *hovering))
                }))
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                // A folder, and the row of a remote, opens and closes.
                if let Some(folder) = &folder {
                    this.selected.clear();
                    return this.toggle_folder(&folder.key, cx);
                }
                // One click selects and shows the commit, two check the
                // reference out; the graph has the keys then. With the toggle
                // or range key held, a click only changes the selection.
                let RowId::Named(name) = &id else {
                    return;
                };
                let modifiers = event.modifiers();
                this.select(name, modifiers, cx);
                if modifiers.secondary() || modifiers.shift {
                    return;
                }
                // Checking out needs a worktree; opening does not.
                let twice = event.click_count() >= 2;
                let checkout_target = checkout_target.as_ref().filter(|_| !read_only);
                match (&checkout_target, &opens) {
                    (Some(name), _) if twice => cx.emit(RefsEvent::Checkout((*name).clone())),
                    (None, Some(request)) if twice => cx.emit(request.clone()),
                    _ => cx.emit(RefsEvent::Reveal(target)),
                }
            }))
            // At the left end, in one column: the check mark of the
            // checked-out branch; else what the graph shows of the row:
            // while anything is shown alone, a mark that shows it alone or
            // stops; a crossed eye on what is hidden, which shows it again;
            // and on the row under the pointer an eye that hides it.
            .child(slot)
            // The column at the left end is not indented, the rest is: as
            // far as a level is, less the gap a child stands apart by.
            .when(item.indent > 0, |row| {
                row.child(
                    div()
                        .flex_none()
                        .w(px((INDENT * item.indent as f32 - ROW_GAP).max(0.))),
                )
            })
            // A remote linked to an account: its owner's picture, when
            // pictures from the providers are on.
            .child(
                match item
                    .remote
                    .as_deref()
                    .and_then(|remote| crate::providers::remote_picture(remote, 14., cx))
                {
                    Some(picture) => picture,
                    None => item
                        .mark
                        .icon(item.folder.as_ref().is_some_and(|folder| folder.open))
                        .size_3p5()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .into_any_element(),
                },
            )
            // The name is cut, with an ellipsis, where the row ends or
            // where what must stay beside it begins: it may shrink, or a
            // long name would run out of the panel and take what stands at
            // its right with it.
            .child(
                div()
                    .id(ElementId::Name(
                        format!("ref-name:{}{}", same_twice, item.full_name).into(),
                    ))
                    .min_w_0()
                    .truncate()
                    // The letters a filter found stand out; else an issue's
                    // key in it is a link.
                    .child(self.name_text(item, issue.as_ref(), same_twice, this_view, cx))
                    .test_support(),
            )
            .when_some(aside, |row, aside| {
                row.child(
                    div()
                        .id(ElementId::Name(
                            format!("ref-aside:{}{}", same_twice, item.full_name).into(),
                        ))
                        .map(|at_right| {
                            if aside_stays {
                                at_right.flex_none().ml_auto()
                            } else {
                                at_right.flex_1().min_w_0()
                            }
                        })
                        .truncate()
                        // A stash's branch follows its message.
                        .when(!is_stash_row, |aside| aside.text_right())
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(crate::fuzzy::marked(
                            aside,
                            if item.path.is_some() {
                                &item.path_marks
                            } else {
                                &[]
                            },
                            theme.primary,
                        ))
                        .test_support(),
                )
            });
        // A branch or tag can be dragged onto a row of the graph, or onto
        // another branch or tag here, where dropping it offers what the two
        // can do together.
        let accent = theme_accent;
        let row = match &kind {
            _ if read_only => row,
            RefKind::Local(name) | RefKind::Remote(name) | RefKind::Tag(name) => {
                let target_here = DropTarget {
                    commit: target,
                    branch: matches!(kind, RefKind::Local(_)).then(|| name.clone()),
                    what: name.clone(),
                    name: name.clone(),
                };
                row.on_drag(
                    Dragged {
                        what: name.clone(),
                        label: name.clone(),
                        commit: target,
                        local: matches!(kind, RefKind::Local(_)),
                        is_ref: true,
                        merge: false,
                    },
                    |dragged, _, _, cx| cx.new(|_| DragChip(dragged.label.clone())),
                )
                .drag_over::<Dragged>(move |style, _, _, _| style.bg(accent))
                .on_drop(cx.listener(move |_, dragged: &Dragged, _, cx| {
                    // Onto itself is nothing.
                    if dragged.what != target_here.what {
                        cx.emit(RefsEvent::Dropped(dragged.clone(), target_here.clone()));
                    }
                }))
            }
            RefKind::Other => row,
        };
        match self.row_menu(section, item, repo_state, cx) {
            Some(builder) => {
                // The row's menu also opens from a button at its right end,
                // which shows while the pointer is on the row.
                let from_button = builder.clone();
                row.relative()
                    .child(
                        div()
                            .absolute()
                            .right(px(MORE_AT))
                            .top_0()
                            .h(px(ROW_HEIGHT))
                            .flex()
                            .items_center()
                            .invisible()
                            .group_hover(ROW_GROUP, |style| style.visible())
                            .child(
                                Button::new((
                                    "refs-more",
                                    section_number(section) * 1_000_000 + ix,
                                ))
                                .accessibility_label(say!(
                                    "refs-row-menu",
                                    name = item.label.as_str()
                                ))
                                .ghost()
                                .xsmall()
                                .icon(IconName::EllipsisVertical)
                                .dropdown_menu(
                                    move |menu, window, cx| {
                                        from_button(
                                            crate::views::menu_fit::fit(menu, window),
                                            window,
                                            cx,
                                        )
                                    },
                                ),
                            ),
                    )
                    .context_menu(move |menu, window, cx| {
                        builder(crate::views::menu_fit::fit(menu, window), window, cx)
                    })
                    .into_any_element()
            }
            None => row.into_any_element(),
        }
    }

    /// The column at the left end of a row: the check mark of the
    /// checked-out branch, or what the graph shows of the row and the
    /// control that changes it (`seen_name` is what it hides or shows
    /// alone).
    fn seen_slot(
        &self,
        row: (&'static str, usize),
        is_head: bool,
        seen_name: Option<String>,
        (hidden, hidden_itself): (bool, bool),
        (soloing, soloed): (bool, bool),
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let slot = div()
            .id(("refs-seen", section_number(row.0) * 1_000_000 + row.1))
            .flex_none()
            .w(px(14.))
            .h(px(ROW_HEIGHT))
            .flex()
            .items_center();
        if is_head {
            return slot
                .child(
                    Icon::new(IconName::Check)
                        .size_3p5()
                        .text_color(theme.success),
                )
                .into_any_element();
        }
        let Some(name) = seen_name else {
            return slot.into_any_element();
        };
        let (icon, color, on_hover_only, change) = if soloing {
            let color = if soloed {
                theme.warning
            } else {
                theme.muted_foreground.opacity(0.5)
            };
            (
                Icon::new(gpui_kit::assets::IconName::Target),
                color,
                false,
                Seen::Solo(name),
            )
        } else if hidden {
            // Hidden with what holds it (its remote, its folder): shown
            // again from there.
            let change = hidden_itself.then_some(Seen::Hidden(name));
            return slot
                .child(
                    Icon::new(IconName::EyeOff)
                        .size_3p5()
                        .text_color(theme.muted_foreground),
                )
                .when_some(change, |slot, change| {
                    slot.cursor_pointer()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.see(change.clone(), cx)
                        }))
                })
                .into_any_element();
        } else {
            (
                Icon::new(IconName::Eye),
                theme.success,
                true,
                Seen::Hidden(name),
            )
        };
        slot.cursor_pointer()
            .child(
                div()
                    .when(on_hover_only, |mark| {
                        mark.invisible()
                            .group_hover(ROW_GROUP, |style| style.visible())
                    })
                    .child(icon.size_3p5().text_color(color)),
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                cx.stop_propagation();
                this.see(change.clone(), cx)
            }))
            .into_any_element()
    }

    /// Change what the graph shows of one reference: asked for from a row
    /// here, or from a label in the graph.
    pub fn see(&mut self, change: Seen, cx: &mut Context<Self>) {
        if let Seen::Moved(name, right) = &change {
            self.move_pinned(name, *right, cx);
            return;
        }
        self.change_visibility(
            |state| match change {
                Seen::Hidden(name) => {
                    if state.hidden.contains(&name) {
                        state.hidden.retain(|n| n != &name);
                    } else {
                        state.hidden.push(name);
                    }
                }
                Seen::Solo(name) => {
                    if state.only.contains(&name) {
                        state.only.retain(|n| n != &name);
                    } else {
                        state.only.push(name);
                    }
                }
                // The menu does not offer one more than fit.
                Seen::Pinned(name) => {
                    let _ = state.toggle_pinned(&name);
                }
                // Done above.
                Seen::Moved(..) => {}
            },
            cx,
        );
    }

    fn change_visibility(
        &mut self,
        f: impl FnOnce(&mut crate::prefs::RepoState),
        cx: &mut Context<Self>,
    ) {
        let path = self.session.path.clone();
        cx.global_mut::<Prefs>().update_repo_state(&path, f);
        cx.emit(RefsEvent::VisibilityChanged);
        cx.notify();
    }
}

impl Focusable for RefsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RefsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loading = self.snapshot.is_none();
        let theme = cx.theme().clone();
        let folded = self.forced_fold
            || self.narrow_fold
            || !cx
                .global::<Prefs>()
                .left_panel_shown(Some(&self.session.path));
        // The pull requests are read while their rows are in view: told
        // after this frame, whatever opened, closed or folded them.
        let pulls_shown = self.peek == Some("pulls")
            || (!folded
                && self.shows_rows("pulls")
                && self.sections.iter().any(|section| section.key == "pulls"));
        if pulls_shown != self.pulls_shown
            && let Some(pulls) = self.pulls.clone()
        {
            self.pulls_shown = pulls_shown;
            cx.defer(move |cx| pulls.update(cx, |pulls, cx| pulls.set_shown(pulls_shown, cx)));
        }
        let issues_shown = self.peek == Some("issues")
            || (!folded
                && self.shows_rows("issues")
                && self.sections.iter().any(|section| section.key == "issues"));
        if issues_shown != self.issues_shown
            && let Some(issues) = self.issues.clone()
        {
            self.issues_shown = issues_shown;
            cx.defer(move |cx| issues.update(cx, |issues, cx| issues.set_shown(issues_shown, cx)));
        }
        if folded {
            return self.render_rail(cx).into_any_element();
        }
        // Every open section is a list of its own, as high as
        // `refs/heights.rs` has it, an empty one too.
        let heights: HashMap<&'static str, f32> = self.layout().into_iter().collect();
        // Whether the sections are taller than their room, so that they
        // scroll as a whole.
        let scrolling = heights.values().sum::<f32>() + HEAD_HEIGHT * self.sections.len() as f32
            > self.room.get() + 0.5;
        // What the graph shows of each section, while not all of it.
        let seen = {
            let state = cx.global::<Prefs>().repo_state(&self.session.path);
            match &self.snapshot {
                Some(snapshot) => rows::sections_seen(snapshot, &state.hidden, &state.only),
                None => HashMap::new(),
            }
        };
        let mut body = v_flex()
            .id("refs-body")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.body_scroll)
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<SectionSplit>, _, cx| {
                    let Some(split) = event.dragged_item().downcast_ref::<SectionSplit>().copied()
                    else {
                        return;
                    };
                    this.drag_split(split, event.event.position.y, cx);
                }),
            );
        // The nearest list above, and how many headings came since.
        let mut above: Option<(&'static str, usize)> = None;
        for (at, section) in self.sections.iter().enumerate() {
            let below = self
                .sections
                .iter()
                .skip(at)
                .find(|section| heights.contains_key(section.key))
                .map(|section| section.key);
            let split = above
                .zip(below)
                .map(|((above, between), below)| SectionSplit {
                    above,
                    below,
                    between,
                });
            body = body.child(self.render_head(
                section,
                split,
                seen.get(section.key).copied(),
                scrolling,
                cx,
            ));
            let Some(height) = heights.get(section.key).copied() else {
                if let Some((_, between)) = &mut above {
                    *between = between.saturating_add(1);
                }
                continue;
            };
            above = Some((section.key, 0));
            let items = section.items.clone();
            let key = section.key;
            if key == "pulls"
                && let Some(pulls) = self.pulls.clone()
            {
                body = body.child(
                    v_flex()
                        .id(("refs-list", section_number(key)))
                        .flex_none()
                        .h(px(height))
                        .child(pulls),
                );
                continue;
            }
            if key == "issues"
                && let Some(issues) = self.issues.clone()
            {
                body = body.child(
                    v_flex()
                        .id(("refs-list", section_number(key)))
                        .flex_none()
                        .h(px(height))
                        .child(issues),
                );
                continue;
            }
            body = body.child(
                v_flex()
                    .id(("refs-list", section_number(key)))
                    .relative()
                    .flex_none()
                    .h(px(height))
                    .child(
                        v_virtual_list(
                            cx.entity().clone(),
                            ("refs-rows", section_number(key)),
                            section.sizes.get(),
                            move |this, range, _, cx| {
                                let repo_state =
                                    cx.global::<Prefs>().repo_state(&this.session.path);
                                range
                                    .filter_map(|ix| {
                                        Some(this.render_item(
                                            key,
                                            ix,
                                            items.get(ix)?,
                                            &repo_state,
                                            cx,
                                        ))
                                    })
                                    .collect::<Vec<_>>()
                            },
                        )
                        .track_scroll(&section.scroll),
                    )
                    .scrollbar(&section.scroll, ScrollbarAxis::Vertical),
            );
        }
        let body = body.when(loading, |this| {
            this.child(
                div()
                    .p_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(say!("refs-loading")),
            )
        });
        // How high the sections are together: measured as they are drawn,
        // and the lists laid out again when it changed (the panel is drawn
        // from a cache, so the view itself is told).
        let room = self.room.clone();
        let view = cx.entity_id();
        let body = div()
            .id("refs-room")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(
                canvas(
                    move |bounds, _, cx| {
                        let height = crate::zoom::unpx(bounds.size.height);
                        if height.is_finite() && (room.get() - height).abs() >= 0.5 {
                            room.set(height);
                            cx.notify(view);
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(body)
            .vertical_scrollbar(&self.body_scroll);
        let top = self.top_line(cx);
        let filtering = !self.filter_text.trim().is_empty();
        self.with_actions(v_flex().id("refs"), cx)
            .size_full()
            .test_support()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            // The top: a row with the button that folds the panel away
            // (where the strip has the one that opens it again), how many
            // references the graph shows (or how many a filter found, or
            // are shown alone) and at its end the button that undoes that;
            // then the filter. Tinted while a filter or showing only some
            // narrows what is seen.
            .child(
                v_flex()
                    .w_full()
                    .flex_none()
                    .pb(px(TOP_BELOW))
                    .when(top.tint == Some(Tint::Solo), |block| {
                        block.bg(theme.warning.opacity(0.25))
                    })
                    .when(top.tint == Some(Tint::Filter), |block| {
                        block.bg(theme.primary.opacity(0.25))
                    })
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(TOP_ROW))
                            .pl_2()
                            .pr(px(6.))
                            .gap(px(6.))
                            .items_center()
                            .text_sm()
                            .child(
                                Button::new("fold-refs")
                                    .accessibility_label(say!("refs-fold"))
                                    .ghost()
                                    .xsmall()
                                    .icon(Icon::new(gpui_kit::assets::IconName::CircleChevronLeft))
                                    .tooltip(say!("refs-fold"))
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.set_folded(true, cx)),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_1p5()
                                    .items_center()
                                    .child(
                                        div()
                                            .flex_none()
                                            .text_color(theme.muted_foreground)
                                            .child(top.word),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_color(theme.primary)
                                            .child(top.count),
                                    ),
                            )
                            .when_some(top.undo, |line, (label, undo)| {
                                line.child(
                                    Button::new("refs-undo-narrowing")
                                        .ghost()
                                        .xsmall()
                                        .flex_none()
                                        .label(label)
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.undo_narrowing(undo, window, cx)
                                        })),
                                )
                            }),
                    )
                    .child(div().px(px(10.)).child(
                        Input::new(&self.filter).small().cleanable(true).when(
                            !filtering,
                            |field| {
                                field.suffix(
                                    Icon::new(IconName::Search)
                                        .size_3p5()
                                        .text_color(theme.muted_foreground),
                                )
                            },
                        ),
                    )),
            )
            .child(body)
            .into_any_element()
    }
}

impl RefsView {
    /// What the panel's menus ask for, wherever the menu was opened: in the
    /// panel, or in the box the folded strip opens.
    fn with_actions(&self, element: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        element
            .on_action(cx.listener(|_, action: &CheckoutRef, _, cx| {
                cx.emit(RefsEvent::Checkout(action.target.clone()))
            }))
            .on_action(cx.listener(|_, action: &NewBranchAt, _, cx| {
                cx.emit(RefsEvent::NewBranch(action.start.clone()))
            }))
            .on_action(cx.listener(|_, action: &RenameBranch, _, cx| {
                cx.emit(RefsEvent::Rename(action.name.clone()))
            }))
            .on_action(cx.listener(|_, action: &DeleteBranch, _, cx| {
                cx.emit(RefsEvent::Delete(action.name.clone()))
            }))
            .on_action(cx.listener(|_, action: &DeleteMany, _, cx| {
                if !action.names.is_empty() {
                    cx.emit(RefsEvent::DeleteMany(action.names.clone()))
                }
            }))
            .on_action(cx.listener(|_, action: &RunRewrite, _, cx| {
                cx.emit(RefsEvent::Rewrite(action.rewrite.clone()))
            }))
            .on_action(cx.listener(|_, action: &CopyText, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(action.text.clone()));
            }))
            .on_action(cx.listener(|_, action: &CompareTip, _, cx| {
                if let Ok(commit) = action.commit.parse() {
                    cx.emit(RefsEvent::Compare(commit));
                }
            }))
            .on_action(cx.listener(|_, action: &RunStash, _, cx| {
                cx.emit(RefsEvent::Stash(action.request.clone()))
            }))
            .on_action(cx.listener(|_, action: &RunTag, _, cx| {
                cx.emit(RefsEvent::Tag(action.request.clone()))
            }))
            .on_action(cx.listener(|_, action: &FastForwardBranch, _, cx| {
                cx.emit(RefsEvent::FastForward(action.name.clone()))
            }))
            .on_action(cx.listener(|_, action: &RunRemote, _, cx| {
                cx.emit(RefsEvent::Remote(action.request.clone()))
            }))
            .on_action(cx.listener(|_, action: &SyncBranch, _, cx| {
                let name = action.name.clone();
                cx.emit(match action.what {
                    Sync::Push => RefsEvent::PushMany(vec![name]),
                    Sync::PushLease => RefsEvent::PushWith(name, PushHow::Lease),
                    Sync::PushForce => RefsEvent::PushWith(name, PushHow::Force),
                    Sync::Pull => RefsEvent::Pull(name),
                    Sync::SetUpstream => RefsEvent::SetUpstream(name),
                    Sync::ClearUpstream => RefsEvent::ClearUpstream(name),
                    Sync::DeleteOnRemote => RefsEvent::DeleteOnRemote(name),
                })
            }))
            .on_action(cx.listener(|_, action: &RunWorktree, _, cx| {
                cx.emit(RefsEvent::Worktree(action.request.clone()))
            }))
            .on_action(cx.listener(|_, action: &RunSubmodule, _, cx| {
                cx.emit(RefsEvent::Submodule(action.request.clone()))
            }))
            .on_action(cx.listener(|_, action: &RunFlow, _, cx| {
                cx.emit(RefsEvent::Flow(action.request.clone()))
            }))
            .on_action(
                cx.listener(|this, action: &RunBatch, _, cx| this.run_batch(action.what, cx)),
            )
            .on_action(cx.listener(|this, action: &SectionDo, _, cx| {
                this.section_do(&action.key, action.what, cx)
            }))
            .on_action(cx.listener(|this, action: &ToggleSection, _, cx| {
                // The branches, and gitflow's where it is set up, are always
                // shown.
                if matches!(action.key.as_str(), "local" | "gitflow") {
                    return;
                }
                if !this.hidden_sections.remove(&action.key)
                    && let Some(key) = known_section(&action.key)
                {
                    this.hidden_sections.insert(key.to_owned());
                }
                this.layout_changed(cx);
            }))
            .on_action(cx.listener(|this, _: &ResetSections, _, cx| {
                this.order = ordered(&[]);
                this.hidden_sections.clear();
                this.sizes.clear();
                this.collapsed = remembered_sections(None);
                this.layout_changed(cx);
            }))
            .on_action(cx.listener(|this, action: &ToggleHidden, _, cx| {
                this.see(Seen::Hidden(action.full_name.clone()), cx)
            }))
            .on_action(cx.listener(|this, action: &ToggleSolo, _, cx| {
                this.see(Seen::Solo(action.full_name.clone()), cx)
            }))
            .on_action(cx.listener(|this, action: &TogglePinned, _, cx| {
                this.see(Seen::Pinned(action.full_name.clone()), cx)
            }))
            .on_action(cx.listener(|this, action: &MovePinned, _, cx| {
                this.move_pinned(&action.full_name, action.right, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowAll, _, cx| {
                this.change_visibility(
                    |state| {
                        state.hidden.clear();
                        state.only.clear();
                    },
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &FocusedVisibility, _, cx| {
                let Some(snapshot) = this.snapshot.clone() else {
                    return;
                };
                // The checked-out branch, what it follows, and its base.
                let solo = rows::focused(&snapshot, this.flow.as_ref());
                this.change_visibility(|state| state.only = solo, cx);
            }))
    }
}

impl RefsView {
    /// Fold the panel to a strip or open it again, and remember it.
    fn set_folded(&mut self, folded: bool, cx: &mut Context<Self>) {
        let path = self.session.path.clone();
        cx.global_mut::<Prefs>()
            .set_panel_shown(Some(&path), true, !folded);
        // The window's own switch for the panel shows the same state.
        cx.refresh_windows();
        cx.notify();
    }

    /// The line at the top of the panel: what narrows what is seen, how
    /// many there are of all, and the button that undoes it. A filter
    /// comes before showing only some, that before what is hidden.
    fn top_line(&self, cx: &App) -> TopLine {
        let state = cx.global::<Prefs>().repo_state(&self.session.path);
        let (shown, narrowed, all) = match &self.snapshot {
            Some(snapshot) => {
                let (shown, narrowed) = rows::viewing(snapshot, &state.hidden, &state.only);
                (shown, narrowed, rows::reference_count(snapshot))
            }
            None => (0, false, 0),
        };
        if !self.filter_text.trim().is_empty() {
            // What the filter found of the references the graph can show.
            let found: usize = self
                .sections
                .iter()
                .filter(|section| section_space(section.key).is_some())
                .map(|section| section.count)
                .sum();
            return TopLine {
                word: say!("refs-top-filtering"),
                count: format!("{found}/{all}"),
                undo: Some((say!("refs-top-clear-filter"), Narrowing::Filter)),
                tint: Some(Tint::Filter),
            };
        }
        if !state.only.is_empty() && narrowed {
            return TopLine {
                word: say!("refs-top-solo"),
                count: format!("{shown}/{all}"),
                undo: Some((say!("refs-top-show-all"), Narrowing::Solo)),
                tint: Some(Tint::Solo),
            };
        }
        if narrowed {
            return TopLine {
                word: say!("refs-top-viewing"),
                count: format!("{shown}/{all}"),
                undo: Some((say!("refs-top-show-all"), Narrowing::Hidden)),
                tint: None,
            };
        }
        TopLine {
            word: say!("refs-top-viewing"),
            count: format!("{shown}"),
            undo: None,
            tint: None,
        }
    }

    /// For the off-screen tests: what the line at the top says, the count,
    /// and the label of its button when it has one.
    pub fn top_said(&self, cx: &App) -> (&'static str, String, Option<&'static str>) {
        let top = self.top_line(cx);
        (top.word, top.count, top.undo.map(|(label, _)| label))
    }

    /// The button of the top line: clear the filter, stop soloing, or
    /// show every reference again.
    fn undo_narrowing(&mut self, what: Narrowing, window: &mut Window, cx: &mut Context<Self>) {
        match what {
            // Setting the field's text says nothing of it to the panel.
            Narrowing::Filter => {
                self.filter
                    .update(cx, |state, cx| state.set_value("", window, cx));
                self.filter_text.clear();
                self.rebuild(cx);
            }
            Narrowing::Solo => self.change_visibility(|state| state.only.clear(), cx),
            Narrowing::Hidden => self.change_visibility(|state| state.hidden.clear(), cx),
        }
    }

    /// The panel folded to a strip: a button to open it, then one mark and
    /// count per section, which shows that section in a box beside the
    /// strip.
    fn render_rail(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let peek = self.peek.and_then(|key| {
            let section = self.sections.iter().find(|section| section.key == key)?;
            Some(self.render_peek(section, cx))
        });
        let rail = v_flex()
            .id("refs-rail")
            .w(px(RAIL_WIDTH))
            .h_full()
            .flex_none()
            .items_center()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.sidebar_border)
            // The button 16 under the top, the first mark 37 under it, 37
            // from one mark to the next, each count right under its mark.
            .child(
                div().h(px(RAIL_FIRST)).flex().items_center().child(
                    Button::new("unfold-refs")
                        .accessibility_label(say!("refs-unfold"))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(gpui_kit::assets::IconName::CircleChevronRight))
                        // Folded by the window's width alone, it opens
                        // when there is room; its sections open from
                        // their marks meanwhile.
                        .when(self.narrow_fold && !self.forced_fold, |button| {
                            button.disabled(true)
                        })
                        .tooltip(if self.narrow_fold && !self.forced_fold {
                            say!("refs-unfold-narrow")
                        } else {
                            say!("refs-unfold")
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.peek = None;
                            // Folded by what the middle shows: opening the
                            // panel is closing that (the tab does it, and
                            // the panel comes back with the graph). Opened
                            // beside it, the panel would be drawn into the
                            // strip's width.
                            if this.forced_fold {
                                cx.emit(RefsEvent::OpenAsked);
                                cx.notify();
                            } else {
                                this.set_folded(false, cx)
                            }
                        })),
                ),
            )
            .child(div().h(px(RAIL_GAP)))
            .children(self.sections.iter().enumerate().map(|(at, section)| {
                let key = section.key;
                let peeking = self.peek == Some(key);
                v_flex()
                    .id(("refs-rail", section_number(key)))
                    .w_full()
                    .h(px(RAIL_STEP))
                    .items_center()
                    .cursor_pointer()
                    .text_color(theme.muted_foreground)
                    .hover(|style| style.bg(theme.accent))
                    .when(peeking, |mark| mark.bg(theme.accent))
                    // The section opens in a box beside the strip; the
                    // panel stays folded.
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if this.peek == Some(key) {
                            this.peek = None;
                            return cx.notify();
                        }
                        this.peek = Some(key);
                        this.peek_at = at;
                        if !this.shows_rows(key) {
                            this.toggle_section(key, cx);
                        }
                        cx.notify();
                    }))
                    .child(section_icon(key).size_4())
                    .child(
                        div()
                            .mt(px(2.))
                            .text_size(px(10.))
                            .line_height(px(12.))
                            .text_color(theme.primary)
                            .child(format!("{}", section.count)),
                    )
                    .test_support()
            }));
        // What the menus of the box's rows ask for is the panel's to do.
        self.with_actions(div().id("refs-folded"), cx)
            .relative()
            .h_full()
            .child(rail)
            .children(peek)
    }

    /// The box a mark of the folded strip opens beside it, over the graph:
    /// the section's heading and its rows, its middle level with the mark.
    /// A click anywhere else puts it away.
    fn render_peek(&self, section: &Section, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let key = section.key;
        let items = section.items.clone();
        let middle = RAIL_FIRST + RAIL_GAP + RAIL_STEP * (self.peek_at as f32 + 0.5);
        let top = (middle - PEEK_HEIGHT / 2.).max(0.);
        let list = v_virtual_list(
            cx.entity().clone(),
            ("refs-peek-rows", section_number(key)),
            section.sizes.get(),
            move |this, range, _, cx| {
                let repo_state = cx.global::<Prefs>().repo_state(&this.session.path);
                range
                    .filter_map(|ix| {
                        Some(this.render_item(key, ix, items.get(ix)?, &repo_state, cx))
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&section.scroll);
        deferred(
            div()
                .id("refs-peek")
                .absolute()
                .left(px(RAIL_WIDTH))
                .top(px(top))
                .w(px(PEEK_WIDTH))
                .h(px(PEEK_HEIGHT))
                .flex()
                .flex_col()
                .bg(theme.sidebar)
                .border_1()
                .border_color(theme.sidebar_border)
                .rounded(theme.radius)
                .shadow_lg()
                .occlude()
                .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                    this.peek = None;
                    cx.notify();
                }))
                .child(
                    h_flex()
                        .h(px(HEAD_HEIGHT))
                        .flex_none()
                        .pl(px(6.))
                        .pr(px(11.))
                        .gap(px(5.))
                        .items_center()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::ChevronDown).size_3p5())
                        .child(section_icon(key).size_4())
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.5))
                                .font_semibold()
                                .child(section.title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.primary)
                                .child(format!("{}", section.count)),
                        ),
                )
                .child(
                    match (
                        self.pulls.clone().filter(|_| key == "pulls"),
                        self.issues.clone().filter(|_| key == "issues"),
                    ) {
                        (Some(pulls), _) => v_flex()
                            .id(("refs-peek-list", section_number(key)))
                            .flex_1()
                            .min_h_0()
                            .child(pulls)
                            .into_any_element(),
                        (None, Some(issues)) => v_flex()
                            .id(("refs-peek-list", section_number(key)))
                            .flex_1()
                            .min_h_0()
                            .child(issues)
                            .into_any_element(),
                        (None, None) => v_flex()
                            .id(("refs-peek-list", section_number(key)))
                            .flex_1()
                            .min_h_0()
                            .child(list)
                            .scrollbar(&section.scroll, ScrollbarAxis::Vertical)
                            .into_any_element(),
                    },
                )
                .test_support(),
        )
        .with_priority(1)
        .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use std::collections::BTreeMap;

    use super::{
        MOST_FOLDERS, Pin, RefKind, remembered_folders, remembered_sections, remembered_sizes,
    };

    /// The menu of a tag's row: its groups in their order, and giving a
    /// message only to a tag that has none.
    #[test]
    fn a_tags_menu_has_its_groups_in_order() {
        use super::{BranchEntry as E, tag_groups};
        let annotated = tag_groups(false);
        assert_eq!(
            annotated,
            [
                vec![E::Combine, E::InteractiveOnto],
                vec![E::Checkout],
                vec![E::Worktree],
                vec![
                    E::NewBranch,
                    E::CherryPick,
                    E::Reset,
                    E::Revert,
                    E::CherryPickStaged
                ],
                vec![E::TagDelete, E::TagDeleteRemote, E::TagPush, E::TagDetails],
                vec![E::CopyName],
                vec![E::Seen],
                vec![E::ShowAll, E::Focus],
            ]
        );
        let lightweight = tag_groups(true);
        assert_eq!(lightweight.last(), Some(&vec![E::Annotate]));
        assert_eq!(lightweight.len(), annotated.len() + 1);
    }

    /// The menu of a branch's row: its groups in their order.
    #[test]
    fn a_branchs_menu_has_its_groups_in_order_for_each_kind_of_branch() {
        use super::{BranchEntry as E, branch_groups};

        let pick = vec![
            E::NewBranch,
            E::CherryPick,
            E::Reset,
            E::Revert,
            E::CherryPickStaged,
        ];
        let seen = [
            vec![E::CopyName, E::CopyId],
            vec![E::Seen],
            vec![E::ShowAll, E::Focus],
            vec![E::Compare],
            vec![E::Tag],
        ];
        let with =
            |own: Vec<Vec<E>>| -> Vec<Vec<E>> { own.into_iter().chain(seen.clone()).collect() };
        let sync = |follows: bool| {
            let mut sync = vec![
                E::Pull,
                E::Push,
                E::SetUpstream,
                E::PushLease,
                E::PushForce,
                E::ClearUpstream,
            ];
            if follows {
                sync.push(E::FastForward);
            }
            sync
        };

        // A local branch that is not checked out.
        assert_eq!(
            branch_groups(true, false, true),
            with(vec![
                sync(true),
                vec![E::Combine],
                vec![E::Checkout],
                vec![E::Worktree],
                pick.clone(),
                vec![E::Rename, E::Delete, E::Describe],
            ])
        );
        // One that follows nothing has nothing to move forward to.
        assert_eq!(
            branch_groups(true, false, false).first(),
            Some(&sync(false))
        );

        // The checked-out branch: nothing that would combine it with
        // itself, check it out or give it a second worktree; and what
        // rewrites its last commit.
        let head = branch_groups(true, true, true);
        assert_eq!(
            head,
            with(vec![
                sync(true),
                vec![E::NewBranch, E::Reset, E::Reword, E::Revert],
                vec![E::Drop, E::MoveDown],
                vec![E::ApplyPatch, E::Rename, E::Delete, E::Describe],
            ])
        );
        let has = |groups: &[Vec<E>], entry: E| groups.iter().flatten().any(|e| *e == entry);
        for absent in [E::Combine, E::Checkout, E::Worktree, E::CherryPick] {
            assert!(!has(&head, absent), "{absent:?} on the checked-out branch");
        }

        // A branch of a remote: nothing goes between it and a remote, it
        // is not renamed, and it is deleted where it is.
        let remote = branch_groups(false, false, false);
        assert_eq!(
            remote,
            with(vec![
                vec![E::Combine],
                vec![E::Checkout],
                vec![E::Worktree],
                pick,
                vec![E::DeleteOnRemote],
            ])
        );
        for absent in [E::Pull, E::Push, E::Rename, E::Delete, E::Reword, E::Drop] {
            assert!(!has(&remote, absent), "{absent:?} on a remote branch");
        }

        // Whatever the branch: no group is empty, and no entry is there
        // twice.
        for groups in [&branch_groups(true, false, true), &head, &remote] {
            assert!(groups.iter().all(|group| !group.is_empty()));
            let all: Vec<E> = groups.iter().flatten().copied().collect();
            for (at, entry) in all.iter().enumerate() {
                assert!(
                    !all.iter().skip(at + 1).any(|e| e == entry),
                    "{entry:?} twice"
                );
            }
        }
    }

    #[test]
    fn a_described_branch_has_a_higher_row_and_nothing_else_has() {
        use std::collections::HashMap;

        use super::{
            BranchNote, DESCRIBED, Item, ROW_HEIGHT, description_of, height_of,
            rows::{Folder, Mark, Place},
        };
        let row = |full_name: &str| Item {
            label: full_name.rsplit('/').next().unwrap_or_default().to_owned(),
            full_name: full_name.to_owned(),
            target: bitbraid_git::Oid::zero(),
            indent: 0,
            is_head: false,
            hint: None,
            mark: Mark::Branch,
            remote: None,
            place: Place::None,
            folder: None,
            path: None,
            marks: Vec::new(),
            path_marks: Vec::new(),
        };
        let note = |description: Option<&str>| BranchNote {
            description: description.map(str::to_owned),
            ..BranchNote::default()
        };
        let notes: HashMap<String, BranchNote> = [
            (
                "wip".to_owned(),
                note(Some("parked until the parser lands")),
            ),
            ("old".to_owned(), note(None)),
            // A folder and a remote's branch of the same name as a
            // described local branch are not that branch.
            ("origin/wip".to_owned(), note(Some("not a local branch"))),
            (
                "feature".to_owned(),
                note(Some("a branch called like a folder")),
            ),
        ]
        .into_iter()
        .collect();

        let described = row("refs/heads/wip");
        assert_eq!(
            description_of(&notes, &described),
            Some("parked until the parser lands")
        );
        assert_eq!(height_of(&notes, &described), ROW_HEIGHT + DESCRIBED);
        for other in [
            row("refs/heads/old"),
            row("refs/heads/unknown"),
            row("refs/remotes/origin/wip"),
            row("refs/tags/wip"),
            Item {
                folder: Some(Folder {
                    key: "local:feature".to_owned(),
                    open: true,
                    held: 2,
                }),
                ..row("")
            },
            Item {
                remote: Some("origin".to_owned()),
                ..row("")
            },
            Item {
                place: Place::Submodule {
                    path: "wip".to_owned(),
                },
                ..row("refs/heads/wip")
            },
        ] {
            assert_eq!(description_of(&notes, &other), None, "{other:?}");
            assert_eq!(height_of(&notes, &other), ROW_HEIGHT, "{other:?}");
        }
        // The branch that is called like a folder has its line.
        assert!(description_of(&notes, &row("refs/heads/feature")).is_some());
        // Nothing known about any branch: every row is as high as before.
        assert_eq!(height_of(&HashMap::new(), &described), ROW_HEIGHT);
    }

    #[test]
    fn rows_know_what_can_be_checked_out() {
        assert_eq!(
            RefKind::of("refs/heads/feature/a"),
            RefKind::Local("feature/a".to_owned())
        );
        assert_eq!(
            RefKind::of("refs/remotes/origin/feature/a").target(),
            Some("origin/feature/a")
        );
        assert_eq!(RefKind::of("refs/tags/v1.0").target(), Some("v1.0"));
        for other in [
            "stash@{0}",
            "worktree:main",
            "submodule:x",
            "",
            "refs/heads",
        ] {
            assert_eq!(RefKind::of(other).target(), None, "{other:?}");
        }
    }

    #[test]
    fn a_branch_is_pushed_pulled_and_told_what_to_follow_from_its_menu() {
        use super::sync_labels;
        let texts = |upstream, is_head| {
            sync_labels(upstream, is_head).map(|(label, greyed)| (label, greyed))
        };
        // Nothing followed: a push publishes, and there is nothing to pull
        // or to clear.
        let alone = texts(None, false);
        assert_eq!(alone[0], ("Pull (it follows nothing)".to_owned(), true));
        assert!(alone[1].0.starts_with("Push (publishes it") && !alone[1].1);
        assert_eq!(alone[2], ("Set upstream…".to_owned(), false));
        assert_eq!(
            alone[3],
            ("Clear upstream (it follows nothing)".to_owned(), true)
        );
        // Following: the checked-out one pulls, another is fetched and
        // moved forward.
        let head = texts(Some("origin/main"), true);
        assert_eq!(head[0], ("Pull from origin/main".to_owned(), false));
        assert_eq!(head[1], ("Push to origin/main".to_owned(), false));
        assert_eq!(
            head[3],
            (
                "Clear upstream (stop following origin/main)".to_owned(),
                false
            )
        );
        let other = texts(Some("origin/topic"), false);
        assert_eq!(
            other[0],
            (
                "Pull (fetch, then fast-forward to origin/topic)".to_owned(),
                false
            )
        );
    }

    #[test]
    fn the_entry_for_focused_visibility_says_what_it_keeps() {
        use super::focus_label;
        let names = |list: &[&str]| {
            list.iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            focus_label(&names(&[
                "refs/heads/topic",
                "refs/remotes/origin/topic",
                "refs/heads/main"
            ])),
            "Only topic, origin/topic, main"
        );
        assert_eq!(
            focus_label(&names(&["refs/heads/main"])),
            "Only main (it follows nothing and has no base)"
        );
        assert!(focus_label(&[]).contains("HEAD is on none"));
    }

    #[test]
    fn pinning_is_offered_where_it_can_work_and_says_why_not() {
        assert_eq!(Pin::NotOffered.entry(), None);
        assert_eq!(Pin::Pinned.entry(), Some(("Unpin", false)));
        assert_eq!(Pin::Free.entry(), Some(("Pin to the left", false)));
        for blocked in [Pin::NotInGraph, Pin::Full] {
            let (label, greyed) = blocked.entry().unwrap();
            assert!(greyed && label.starts_with("Pin to the left ("), "{label}");
        }
    }

    #[test]
    fn what_is_remembered_about_the_layout_is_held_to_a_sane_size() {
        // Nothing remembered: every section closed.
        let start = remembered_sections(None);
        for key in [
            "gitflow",
            "local",
            "remote",
            "worktrees",
            "stashes",
            "tags",
            "submodules",
        ] {
            assert!(start.contains(key), "{key} starts out closed");
        }
        // Remembered as all open.
        assert!(remembered_sections(Some(Vec::new())).is_empty());
        let hostile: Vec<String> = (0..500).map(|n| format!("s{n}")).collect();
        assert_eq!(remembered_sections(Some(hostile)).len(), 32);

        let folders: Vec<String> = (0..10_000)
            .map(|n| format!("remote:origin/team-{n}"))
            .chain(["".to_owned(), "x".repeat(5000)])
            .collect();
        let kept = remembered_folders(&folders);
        assert_eq!(kept.len(), MOST_FOLDERS);
        assert!(kept.iter().all(|key| !key.is_empty() && key.len() < 100));

        let sizes: BTreeMap<String, f32> = [
            ("local", 120.0),
            ("remote", f32::NAN),
            ("tags", -5.0),
            ("stashes", f32::INFINITY),
            ("worktrees", 1e30),
            ("nonsense", 50.0),
        ]
        .into_iter()
        .map(|(key, size)| (key.to_owned(), size))
        .collect();
        let kept = remembered_sizes(&sizes);
        assert_eq!(kept.get("local"), Some(&120.0));
        assert_eq!(kept.get("worktrees"), Some(&100_000.0));
        assert_eq!(kept.len(), 2, "{kept:?}");
    }
}
