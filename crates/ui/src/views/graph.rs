//! The commit graph: a virtual list of rows with a painted lane canvas,
//! reference labels, summary, author, date and id.
//!
//! Sources: gpui-kit documentation (paths, assets).

use std::{
    cell::{Cell, RefCell},
    collections::{BTreeSet, HashMap, HashSet},
    rc::Rc,
    sync::Arc,
};

use bitbraid_git::{CommitInfo, GraphCommit, GraphRow, Oid, Signature};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt, VirtualListScrollHandle,
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
use serde_json::json;

use crate::{
    keymap::{
        CompareWithWorktree, CopyCommitId, ExtendNext, ExtendPrev, FocusGraph, FocusSearch,
        GRAPH_CONTEXT, NextBranch, PageDown, PageUp, PrevBranch, SelectFirst, SelectLast,
        SelectNext, SelectPrev, ToggleOnlyMatches,
    },
    prefs::{Column, Prefs},
    session::{Badge, BadgeKind, GraphData, Reply, Request, SessionHandle, WorktreeMark},
    theme::lane_color,
    views::{
        absolute_time,
        avatar::paint_identicon,
        now,
        refs::{MovePinned, Pin, Seen, ToggleHidden, TogglePinned, ToggleSolo, lane_moves},
        relative_time,
        rewrite::{self, Choice, DragChip, Dragged, DropTarget, Rewrite, RunRewrite, drop_choices},
        signatures::{Level, Verification, unchecked_sentence},
        stashes::{self, RunStash, StashRequest},
        tags::{self, RunTag, TagRequest},
        worktrees::{self, RunWorktree, WorktreeRequest},
    },
};

mod access;
mod checks;
mod light;
mod probe;
mod widen;

pub use probe::GraphProbe;

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct ToggleColumn {
    column: Column,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct MoveColumn {
    column: Column,
    /// The column in whose place it goes.
    to: Column,
}

/// Show the branch and tag column, or the graph column, or put it away.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct ToggleFirstColumns {
    /// The graph column; else the branch and tag column.
    graph: bool,
}

/// A column's heading picked up to be put elsewhere among the columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HeadingDrag(Column);

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct ToggleAbsoluteDates;

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct ResetColumns;

/// Show the commit date in the rows, or the author date again.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct ToggleCommitDates;

/// Order the graph topologically, or by date again.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct ToggleTopoOrder;

/// Order the graph by the date each change was written, or by the date of
/// the commit again.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct ToggleAuthorOrder;

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct ToggleDateDividers;

/// Leave only the row the cursor is on selected.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct ClearPicked;

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct CheckoutTarget {
    target: String,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct BranchAt {
    start: String,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct CopyId {
    id: String,
}

/// Write commits to files as patches.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct SavePatches {
    commits: Vec<String>,
}

/// Put a `bitbraid://` link to a commit on the clipboard.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct CopyLink {
    commit: String,
}

/// List what differs between a commit and the working tree, in the panel
/// beside the graph: the commit is selected for it.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct CompareAt {
    commit: String,
}

/// What the menu of a label does to the branch it names, where no other
/// action says it.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct LabelAction {
    what: LabelDo,
    name: String,
}

/// The entries of a label's own menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum LabelDo {
    Pull,
    SetUpstream,
    Rename,
    Delete,
    DeleteOnRemote,
    FastForward,
    Push,
    CopyName,
}

/// What the menu of the row of uncommitted work does.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = graph, no_json)]
struct WipDo {
    what: WipAction,
}

/// Something asked of the uncommitted work as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum WipAction {
    /// Stage every change.
    StageAll,
    /// Take everything out of the index again.
    UnstageAll,
    /// Throw the unstaged changes away, after asking.
    DiscardAll,
    /// Commit what is staged.
    Commit,
    /// Stage everything and commit.
    CommitAll,
}

/// Something asked of a branch from its label in the graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchRequest {
    /// Pull it: the checked-out branch as the toolbar does, another one
    /// moved to its upstream after a fetch.
    Pull(String),
    /// Ask what it is to follow.
    SetUpstream(String),
    /// Ask for another name.
    Rename(String),
    /// Ask, then delete it.
    Delete(String),
    /// Move it to its upstream without checking it out.
    FastForward(String),
    /// Push it.
    Push(String),
    /// Ask, then delete the branch of a remote on that remote.
    DeleteOnRemote(String),
}

const ROW_HEIGHT: f32 = 28.0;
/// Height of the columns' heading, its rule under it included, and what
/// is free between it and the first row.
const HEADER_HEIGHT: f32 = 23.0;
const HEADER_GAP: f32 = 2.0;
/// Where the heading's words begin in the label column, and in any other.
const HEADER_FIRST_INSET: f32 = 16.0;
const HEADER_INSET: f32 = 5.0;
/// The strip of the heading that takes the pointer to resize a column:
/// this wide, with its middle on the border's line (one point wide, the
/// first of the next heading), so that it is grabbed on either side of
/// the line; and how far of it lies left of the line.
const GRAB_WIDTH: f32 = 9.0;
const GRAB_LEFT: f32 = 4.0;
/// What is free above and below a row's band, its bar and the fill of a
/// selected row: they are 22 high in a row of 28, so that the rows read as
/// rows and not as one block.
const BAND_INSET: f32 = 3.0;
/// The group a row of the graph is, for what shows while the pointer is on
/// it.
const ROW_GROUP: &str = "graph-row";
/// Width of the bar that closes a row's band, at the end of the graph
/// column.
const BAND_BAR: f32 = 2.0;
/// Space between that bar and what the row says.
const MESSAGE_INSET: f32 = 7.0;
/// How much of its lane's colour the band of a selected row has.
const SELECTED_BAND: f32 = 0.45;
const LANE_PITCH: f32 = 22.0;
/// Radius of the placeholder dot shown until a commit's author is known.
const NODE_RADIUS: f32 = 4.0;
/// Radius of the avatar ring that is an ordinary commit's node.
const NODE_RING: f32 = 11.0;
/// Radius of the dot that is a merge commit's node.
const MERGE_DOT: f32 = 5.0;
/// Half the side of the dotted square that is a stash's node: as large as
/// a commit's node.
const STASH_BOX: f32 = 11.0;
/// Dash and gap of a stash's square and of its line down to the commit it
/// was made on: dots.
const DOTS: [f32; 2] = [2.0, 2.0];
/// Radius of the corner where an edge turns from a node's row into a lane.
const CORNER: f32 = 8.0;
/// Width of lane lines.
const STROKE: f32 = 2.0;
/// Dash and gap of the lane of uncommitted work. Two periods fill half a
/// row, so the pattern runs unbroken from one row into the next.
const DASH: [f32; 2] = [4.0, 3.0];
/// Space left of the first lane: room for a node and the end of the line
/// that comes in from a label.
const LEFT_MARGIN: f32 = 36.0;
/// Space right of the last lane, so that its node clears the column edge.
const RIGHT_MARGIN: f32 = 6.0;
/// Height of a time divider's box, hanging under the line at the row's
/// top edge, and how far it stands from the row's right end.
const DIVIDER_H: f32 = 14.0;
const DIVIDER_RIGHT: f32 = 8.0;
/// Opacity of what rows outside the branch whose labels the pointer is
/// over say, once they have stepped back (`light`).
const FADED: f32 = 0.3;
/// How much is left of what a row says while a search is on and the row
/// is not among what it found.
const UNMATCHED: f32 = 0.35;

/// Where the search bar's top is, from the top of the columns' heading.
const SEARCH_TOP: f32 = 6.0;
/// How high the search bar is.
const SEARCH_HEIGHT: f32 = 33.0;
/// What the list keeps free over its first row while the search bar is
/// open: as far as the bar reaches down over the rows, and a little under
/// it, so that the first row, with only what was found shown the first of
/// them, is not under the bar with its message hidden; rows scrolled up
/// pass under it.
const SEARCH_INSET: f32 = SEARCH_TOP - HEADER_HEIGHT - HEADER_GAP + SEARCH_HEIGHT + 4.0;

/// What the search bar says of what it found: which of them is selected,
/// of how many, or how many there are when none of them is.
fn match_count(at: Option<usize>, found: usize) -> String {
    match (at, found) {
        (Some(at), found) if at < found => {
            say!("graph-search-match-of", at = at.saturating_add(1), found)
        }
        (_, found) => say!("graph-search-results", found),
    }
}

/// Whether what a row says is dimmed: something is searched for, the
/// search has answered, and the row is not among what it found.
fn unmatched(searched_for: &str, pending: bool, is_match: bool) -> bool {
    !searched_for.trim().is_empty() && !pending && !is_match
}

/// The widest the graph column makes itself to fit the lanes; a wider
/// graph scrolls sideways in it.
const AUTO_GRAPH_MOST: f32 = 232.0;

/// How wide the graph column makes itself for this many lanes, until its
/// border is dragged: room for every lane and its node, up to
/// [`AUTO_GRAPH_MOST`].
fn auto_graph_width(lanes: u16) -> f32 {
    (40.0 + LANE_PITCH * f32::from(lanes.max(1))).min(AUTO_GRAPH_MOST)
}

/// Width the graph needs for this many lanes.
fn graph_extent(lanes: u16) -> f32 {
    LEFT_MARGIN + LANE_PITCH * f32::from(lanes) + RIGHT_MARGIN
}

/// Horizontal placement of lanes in the graph column.
#[derive(Debug, Clone, Copy)]
struct LaneGeometry {
    /// Left edge of the column.
    left: Pixels,
    /// Width of the column.
    width: f32,
    /// Horizontal scroll offset.
    scroll_x: f32,
}

impl LaneGeometry {
    /// Centre of a lane, relative to the column's left edge.
    fn offset(&self, slot: u16) -> f32 {
        LEFT_MARGIN + LANE_PITCH * (f32::from(slot) + 0.5) - self.scroll_x
    }

    /// Centre of a lane.
    fn x(&self, slot: u16) -> Pixels {
        self.left + px(self.offset(slot))
    }

    /// Whether anything drawn between the lanes of two slots can show in
    /// the column.
    fn spans(&self, a: u16, b: u16) -> bool {
        self.offset(a.max(b)) > -NODE_RING && self.offset(a.min(b)) < self.width + NODE_RING
    }

    /// Whether a lane can show in the column.
    fn shows(&self, slot: u16) -> bool {
        self.spans(slot, slot)
    }
}

/// How wide the shade is over a side of the graph column that has lanes
/// beyond it, and how much of the background it has.
const PIN_SHADE: f32 = 28.0;
const PIN_SHADE_STRENGTH: f32 = 0.6;

/// What a node held at an edge of the graph column keeps free there: at
/// the left, from the column's edge; at the right, from the bar that
/// closes the row's band.
const PIN_MARGIN: f32 = 2.0;

/// Where the centre of a node whose lane's centre is `x` points from the
/// graph column's left edge is drawn in a column `width` points wide, and
/// whether it is held there rather than at its lane: at its lane while
/// the whole of it fits in the column (less what is kept free at either
/// edge), else at the nearer edge, its side against what is kept free
/// there. It is held from the moment it would be cut, so a column made
/// narrower point by point moves it with its lane until then and by as
/// much as the column after: it never jumps, and no part of it is cut
/// off. A column too narrow for a whole node holds it at the left.
fn node_centre(x: f32, width: f32) -> (f32, bool) {
    let least = NODE_RING + PIN_MARGIN;
    // A width that is no number leaves the least room.
    let most = (width - NODE_RING - BAND_BAR - PIN_MARGIN).max(least);
    if !x.is_finite() {
        (x, false)
    } else if x < least {
        (least, true)
    } else if x > most {
        (most, true)
    } else {
        (x, false)
    }
}

/// Where a row's node is drawn: at its lane, or, when the whole of it
/// does not fit there, at the nearer edge of the column, inside it (and
/// whether it was held there): [`node_centre`].
fn pinned_node(geometry: &LaneGeometry, lane: u16) -> (Pixels, bool) {
    let (centre, held) = node_centre(geometry.offset(lane), geometry.width);
    (geometry.left + px(centre), held)
}

/// Who a person is, as a commit names them: `Name <address>`, or what of
/// the two there is.
fn person(who: &Signature) -> String {
    match (who.name.trim(), who.email.trim()) {
        ("", "") => String::new(),
        (name, "") => name.to_owned(),
        ("", email) => format!("<{email}>"),
        (name, email) => format!("{name} <{email}>"),
    }
}

/// What the circle of a commit's author says under the pointer: the author,
/// and on a line of its own who committed it when that is someone else
/// (another name or another address).
fn circle_said(author: &Signature, committer: &Signature) -> Vec<String> {
    let mut said = vec![person(author)];
    if author.name.trim() != committer.name.trim() || author.email.trim() != committer.email.trim()
    {
        said.push(say!("graph-committed-by", person = person(committer)));
    }
    said
}

/// The tooltip of a commit's author's circle or picture: what
/// [`circle_said`] says, a line each, who committed it in the muted
/// colour.
fn person_tooltip(said: Vec<String>) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    move |window, cx| {
        let said = said.clone();
        Tooltip::element(move |_, cx| {
            let muted = cx.theme().muted_foreground;
            v_flex()
                .id("author-tip")
                .role(Role::Tooltip)
                .aria_label(said.join("\n"))
                .children(said.iter().enumerate().map(|(at, line)| {
                    div()
                        .when(at > 0, |line| line.text_color(muted))
                        .child(line.clone())
                }))
                .test_support()
        })
        .build(window, cx)
    }
}

/// How wide the tooltip of a commit's message is at most, in points:
/// about [`MESSAGE_TIP_LETTERS`] letters of its text.
const MESSAGE_TIP_WIDTH: f32 = 420.0;
/// About how many letters a line of that tooltip holds, and how many such
/// lines it has at most: a longer message is cut there, with an ellipsis,
/// so that the tooltip never covers the window.
const MESSAGE_TIP_LETTERS: usize = 60;
const MESSAGE_TIP_LINES: usize = 24;

/// What the tooltip of a commit's message says: its summary as the row
/// shows it, then, when there is one, an empty line and the body as it was
/// written, its lines as they are. Where that would be more than
/// [`MESSAGE_TIP_LINES`] lines of [`MESSAGE_TIP_LETTERS`] letters (a long
/// line taking as many as it fills), it is cut, and ends with an ellipsis.
fn message_said(summary: &str, body: &str) -> String {
    let body = body.trim();
    let mut lines = vec![summary.trim_end()];
    if !body.is_empty() {
        lines.push("");
        lines.extend(body.lines());
    }
    let mut said: Vec<String> = Vec::new();
    let mut room = MESSAGE_TIP_LINES;
    for line in lines {
        let line = line.trim_end();
        let takes = line.chars().count().div_ceil(MESSAGE_TIP_LETTERS).max(1);
        if takes > room {
            // What still fits of it, and the ellipsis.
            let kept: String = line
                .chars()
                .take(room.saturating_mul(MESSAGE_TIP_LETTERS).saturating_sub(1))
                .collect();
            said.push(format!("{}\u{2026}", kept.trim_end()));
            break;
        }
        room -= takes;
        said.push(line.to_owned());
    }
    said.join("\n")
}

/// The tooltip of a commit's message: what [`message_said`] says, its
/// lines kept and a long one wrapped at the tooltip's width.
fn message_tooltip(said: String) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let said = SharedString::from(said);
    move |window, cx| {
        let said = said.clone();
        Tooltip::element(move |_, _| {
            div()
                .id("message-tip")
                .role(Role::Tooltip)
                .aria_label(said.clone())
                .max_w(px(MESSAGE_TIP_WIDTH))
                .whitespace_normal()
                .child(said.clone())
                .test_support()
        })
        .build(window, cx)
    }
}

/// What marks a row's place in its lane.
#[derive(Debug, Clone)]
enum Node {
    /// Uncommitted work: a dashed ring.
    Wip,
    /// A merge commit: a small dot.
    Merge,
    /// A stash: a square, so that it is not taken for a commit on a branch.
    Stash,
    /// Any other commit: the author's identicon in a ring, keyed by email
    /// once the details are known; or the author's picture, where pictures
    /// are on and there is one.
    Commit(Option<String>, Option<Arc<RenderImage>>),
}

/// A commit that carries a signature: in which form, and what git made of
/// it once that has been asked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Signed {
    format: Option<String>,
    verification: Option<Verification>,
}

/// How many signatures are checked in one go: each can cost a process.
const VERIFY_AT_ONCE: usize = 64;

/// Height of a label, of the box that folds the rest, and of a field that
/// names something new in their place.
const LABEL_HEIGHT: f32 = 22.0;
/// Where the labels begin, from the column's left edge.
const LABELS_LEFT: f32 = 10.0;
/// Between a label and the box that says how many more there are.
const LABELS_GAP: f32 = 5.0;
/// The box "+N" for the labels that are folded: as wide as "+" and one
/// digit with room around them, and as much wider as a digit is for each
/// digit more; and the least of the line towards the node.
const LABELS_FOLDED_MARK: f32 = 20.0;
const LABELS_FOLDED_DIGIT: f32 = 7.0;
const LABELS_CONNECTOR: f32 = 6.0;
/// Between labels stacked in a column when a commit's are unfolded.
const LABELS_STACKED_GAP: f32 = 2.0;

/// The line from a row's labels to its node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LabelLine {
    /// The row has no labels.
    None,
    /// Thin and faint: the labels of anything but the checked-out branch.
    Thin,
    /// Bold, in the lane's colour: the checked-out branch's label.
    Bold,
}

impl LabelLine {
    /// How thick the line is and how much of the lane's colour it has.
    fn stroke(self) -> (f32, f32) {
        match self {
            LabelLine::Bold => (2.0, 1.0),
            LabelLine::Thin | LabelLine::None => (1.0, 0.4),
        }
    }
}

/// The row whose labels are unfolded, and what keeps them so: the
/// pointer on the row's labels, on the column of all of them, which lies
/// over the rows below, or on one of them widened to its whole name
/// (`widen`); or the menu of one of them opened from the column, which
/// lies over it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Unfold {
    row: Option<usize>,
    on_labels: bool,
    on_column: bool,
    on_widened: bool,
    menu_open: bool,
}

/// What keeps a row's labels unfolded ([`Unfold`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Over {
    /// The pointer on the row's labels.
    Labels,
    /// On the column of them unfolded.
    Column,
    /// On one of them widened.
    Widened,
    /// The menu of one of them, opened from the column.
    Menu,
}

/// A row's menu, opened from more than one place of the row.
type RowMenu = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// Where a row keeps its menu for what opens it from elsewhere than the
/// row itself: the column its labels unfold into, which lies over the rows
/// below and takes the pointer from them, the row included.
type KeptMenu = Rc<RefCell<Option<RowMenu>>>;

/// The row's menu `menu`, as the row opens it, kept in `kept` as well.
fn keep_menu(
    kept: &KeptMenu,
    menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    let menu: RowMenu = Rc::new(menu);
    if let Ok(mut slot) = kept.try_borrow_mut() {
        *slot = Some(menu.clone());
    }
    move |popup, window, cx| menu(popup, window, cx)
}

/// A reference label on a row: one name with every place it is in (a
/// branch here and on remotes), a tag or a stash; or another worktree
/// that has the commit checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RefLabel {
    /// What the label says: a branch's name without the remote it is on, a
    /// tag's name, "HEAD" when it is detached, a worktree's folder.
    name: String,
    /// The checked-out branch, or a detached HEAD.
    head: bool,
    local: bool,
    /// The remote branches of this name at the commit, as git names them
    /// (`origin/main`), in the order of the remotes: one cloud each.
    remotes: Vec<String>,
    tag: bool,
    stash: bool,
    /// Another worktree has this checked out: where that worktree is. On a
    /// label that is nothing else, the name is that of its folder.
    worktree: Option<String>,
    /// The upstream of the checked-out branch is among what the label
    /// stands for: the remote branch it follows, on the label of the
    /// checked-out branch itself when the two are at the same commit.
    upstream: bool,
}

impl RefLabel {
    /// Whether a remote has a branch of this name at the commit.
    fn remote(&self) -> bool {
        !self.remotes.is_empty()
    }

    /// What git calls what the label stands for: the branch here, or for a
    /// label that is only on remotes the first remote's branch; a tag by
    /// its name.
    fn git_name(&self) -> String {
        if self.local || self.head || self.tag {
            return self.name.clone();
        }
        self.remotes
            .first()
            .cloned()
            .unwrap_or_else(|| self.name.clone())
    }

    /// Whether the label names something git knows by that name: a branch,
    /// a tag or HEAD, and not a stash or a worktree's folder.
    fn names_a_ref(&self) -> bool {
        self.head || self.local || self.remote() || self.tag
    }
}

/// Group a commit's badges into labels: one for each name. A branch that
/// is here and on remotes at this commit is one label, and a remote
/// branch is called by its own name, without the remote's; the places it
/// is in are said by the label's icons.
fn ref_labels(badges: &[Badge]) -> Vec<RefLabel> {
    let mut labels: Vec<RefLabel> = Vec::new();
    for badge in badges {
        // A stash is its row; it has no label of its own.
        if badge.kind == BadgeKind::Stash {
            continue;
        }
        if badge.kind == BadgeKind::Remote {
            let branch = badge
                .name
                .split_once('/')
                .map_or(badge.name.as_str(), |(_, branch)| branch);
            let same = labels.iter_mut().find(|label| {
                !label.tag && (label.local || label.remote()) && label.name == branch
            });
            match same {
                Some(label) => label.remotes.push(badge.name.clone()),
                None => labels.push(RefLabel {
                    name: branch.to_owned(),
                    head: false,
                    local: false,
                    remotes: vec![badge.name.clone()],
                    tag: false,
                    stash: false,
                    worktree: None,
                    upstream: false,
                }),
            }
            continue;
        }
        labels.push(RefLabel {
            name: badge.name.clone(),
            head: badge.kind == BadgeKind::Head,
            // A detached HEAD is not a branch.
            local: badge.kind == BadgeKind::Local
                || (badge.kind == BadgeKind::Head && badge.name != "HEAD"),
            remotes: Vec::new(),
            tag: badge.kind == BadgeKind::Tag,
            stash: false,
            worktree: None,
            upstream: false,
        });
    }
    labels
}

/// Mark the labels of a commit with the other worktrees that have it
/// checked out: on the label of the branch a worktree is on, or as a label
/// of its own, named after its folder, for a detached one.
fn with_worktrees(mut labels: Vec<RefLabel>, marks: &[WorktreeMark], commit: Oid) -> Vec<RefLabel> {
    for mark in marks.iter().filter(|mark| mark.head == commit) {
        let on_branch = mark.branch.as_ref().and_then(|branch| {
            labels
                .iter_mut()
                .find(|label| label.local && !label.head && label.name == *branch)
        });
        match on_branch {
            Some(label) => label.worktree = Some(mark.path.clone()),
            None => labels.push(RefLabel {
                name: worktrees::folder_name(&mark.path),
                head: false,
                local: false,
                remotes: Vec::new(),
                tag: false,
                stash: false,
                worktree: Some(mark.path.clone()),
                upstream: false,
            }),
        }
    }
    labels
}

/// Mark the label that stands for the upstream of the checked-out branch.
/// `upstream` is its short name, like `origin/main`: the label that has it
/// among its remote branches.
fn mark_upstream(labels: &mut [RefLabel], upstream: Option<&str>) {
    let Some(upstream) = upstream else { return };
    if let Some(label) = labels
        .iter_mut()
        .find(|label| label.remotes.iter().any(|remote| remote == upstream))
    {
        label.upstream = true;
    }
}

/// One entry of a label's menu, or a run of entries another part builds
/// (merging and rebasing, a reset, what the graph shows of it, a tag's).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LabelEntry {
    /// Between the branch and its remote.
    Pull,
    Push,
    SetUpstream,
    FastForward,
    /// Merge it into the checked-out branch, rebase that onto it.
    Combine,
    /// A submenu: the branch, or the commit (a tag's, or the checked-out
    /// branch's: the commit alone).
    Checkout,
    /// A submenu likewise: a worktree for the branch or for the commit.
    Worktree,
    /// What is made of the label's commit or brought to it.
    NewBranch,
    CherryPick,
    CherryPickStaged,
    Reset,
    Reword,
    Revert,
    Drop,
    MoveDown,
    ApplyPatch,
    /// The branch's or the tag's own name and end.
    Rename,
    Delete,
    DeleteOnRemote,
    /// A tag's own entries: its details, pushing it, deleting it here and
    /// on the remote.
    TagEntries,
    CopyName,
    CopyId,
    SavePatch,
    /// Hide it, pin it, show it alone.
    Seen,
    Compare,
    /// A tag, plain or annotated, at the label's commit.
    NewTag,
    /// A message for a tag that has none.
    Annotate,
    /// The worktree that has it checked out.
    OpenWorktree,
    /// Make a pull request of the branch.
    StartPull,
}

/// What the menu of a label has to know beside the label.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct LabelFacts {
    /// The local branch follows a remote branch.
    follows: bool,
    /// The label's commit is the one HEAD is at.
    at_head: bool,
    /// A tag without a message of its own.
    lightweight: bool,
    /// The graph knows what the graph shows of it (hide, pin, show alone).
    seen: bool,
    /// A remote is linked to an account: pull requests can be made.
    pulls: bool,
}

/// The menu of a label as groups of entries, a rule between two: what
/// goes between the branch and its remote; what it does to the checked-out
/// branch; checking it out; a worktree; what is made of its commit or
/// brought to it; its own name and end; what is copied; what the graph
/// shows of it; comparing; a tag. The checked-out branch has nothing that
/// would combine it with itself or check it out, and has what rewrites its
/// last commit. A label that names no reference (a worktree's folder, a
/// detached HEAD) has only its worktree.
fn label_groups(label: &RefLabel, facts: LabelFacts) -> Vec<Vec<LabelEntry>> {
    use LabelEntry as E;
    let mut groups: Vec<Vec<LabelEntry>> = Vec::new();
    if label.names_a_ref() && label.name != "HEAD" && !label.stash {
        let made = vec![
            E::NewBranch,
            E::CherryPick,
            E::Reset,
            E::Revert,
            E::CherryPickStaged,
        ];
        let copies = vec![E::CopyName, E::CopyId, E::SavePatch];
        let seen = || facts.seen.then(|| vec![E::Seen]);
        if label.tag {
            groups.extend([
                vec![E::Combine],
                vec![E::Checkout],
                vec![E::Worktree],
                made,
                vec![E::TagEntries],
                vec![E::CopyName],
            ]);
            groups.extend(seen());
            if facts.lightweight {
                groups.push(vec![E::Annotate]);
            }
        } else {
            let mut sync = Vec::new();
            if label.local {
                if facts.follows {
                    sync.push(E::Pull);
                }
                sync.extend([E::Push, E::SetUpstream]);
                if facts.follows && !label.head {
                    sync.push(E::FastForward);
                }
            }
            if !sync.is_empty() {
                groups.push(sync);
            }
            if label.head {
                groups.extend([
                    vec![E::Checkout],
                    vec![E::Worktree],
                    vec![E::NewBranch, E::Reword, E::Revert],
                    vec![E::Drop, E::MoveDown],
                    vec![E::ApplyPatch, E::Rename, E::Delete],
                ]);
            } else {
                groups.extend([
                    vec![E::Combine],
                    vec![E::Checkout],
                    vec![E::Worktree],
                    made,
                    if label.local {
                        vec![E::Rename, E::Delete]
                    } else {
                        vec![E::DeleteOnRemote]
                    },
                ]);
            }
            // Making a pull request comes after what is made of its
            // commit.
            if facts.pulls && (label.local || label.remote()) {
                let at = groups
                    .iter()
                    .position(|group| group.contains(&E::NewBranch))
                    .map_or(groups.len(), |at| at + 1);
                groups.insert(at, vec![E::StartPull]);
            }
            groups.push(copies);
            groups.extend(seen());
            groups.extend([vec![E::Compare], vec![E::NewTag]]);
        }
    }
    if label.worktree.is_some() {
        groups.push(vec![E::OpenWorktree]);
    }
    groups
}

/// The reference a label names, by its full name: what hiding, showing
/// alone and pinning go by. A label that stands for a local branch and the
/// remote one it follows names the local one. None for a stash, a
/// detached HEAD and a worktree's folder.
fn label_ref(label: &RefLabel) -> Option<String> {
    if !label.names_a_ref() || label.name == "HEAD" || label.stash {
        return None;
    }
    let name = &label.git_name();
    if label.local || label.head {
        Some(format!("refs/heads/{name}"))
    } else if label.tag {
        Some(format!("refs/tags/{name}"))
    } else if label.remote() {
        Some(format!("refs/remotes/{name}"))
    } else {
        None
    }
}

/// What the graph shows of the reference a label names, for the entries
/// of its menu that change it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct LabelSeen {
    full_name: String,
    hidden: bool,
    soloed: bool,
    pin: Pin,
    /// For one that is pinned: whether its lane can move left, and right.
    moves: (bool, bool),
}

impl LabelSeen {
    /// Of `label`, with what is hidden, shown alone and pinned in this
    /// repository. A label is on a row of the graph, so its branch can be
    /// pinned unless as many are pinned as there is room for; a tag has
    /// no line of its own to pin.
    fn of(label: &RefLabel, hidden: &[String], solo: &[String], pinned: &[String]) -> Option<Self> {
        let full_name = label_ref(label)?;
        let pin = if pinned.contains(&full_name) {
            Pin::Pinned
        } else if label.tag && !label.local && !label.head && !label.remote() {
            Pin::NotOffered
        } else if pinned.len() >= crate::prefs::MAX_PINNED {
            Pin::Full
        } else {
            Pin::Free
        };
        Some(Self {
            hidden: hidden.contains(&full_name),
            soloed: solo.contains(&full_name),
            pin,
            moves: lane_moves(pinned, &full_name),
            full_name,
        })
    }
}

/// What the menu of a label is about: the label, the commit it is on,
/// and what is known around them.
struct LabelMenu<'a> {
    label: &'a RefLabel,
    /// The commit, by its full id, and whether it is a merge commit.
    commit: &'a str,
    merge: bool,
    /// What the local branch follows, as `remote/branch`.
    upstream: Option<&'a str>,
    /// The preferences keep it from being deleted.
    protected: bool,
    /// The checked-out branch, when HEAD is on one.
    current: Option<&'a str>,
    facts: LabelFacts,
    seen: Option<&'a LabelSeen>,
    /// The open pull request of its branch, if it has one.
    pull: Option<crate::views::pulls::facts::PullMark>,
}

impl LabelMenu<'_> {
    /// Add one entry, or the run of entries it stands for.
    fn add(
        &self,
        menu: PopupMenu,
        entry: LabelEntry,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        use LabelEntry as E;
        let label = self.label;
        let name = label.git_name();
        let commit = self.commit.to_owned();
        let at_head = self.facts.at_head;
        // Another worktree has the branch checked out.
        let elsewhere = label.local && label.worktree.is_some();
        let named = |what: LabelDo| -> Box<dyn Action> {
            Box::new(LabelAction {
                what,
                name: name.clone(),
            })
        };
        let rewrite = |rewrite: Rewrite| -> Box<dyn Action> { Box::new(RunRewrite { rewrite }) };
        let pick = |menu: PopupMenu, staged: bool| {
            menu.menu_with_disabled(
                // A merge commit is asked about first: against which parent.
                match (self.merge, staged, at_head) {
                    (false, false, false) => say!("graph-cherry-pick"),
                    (false, true, false) => say!("graph-cherry-pick-staged"),
                    (true, false, false) => say!("graph-cherry-pick-merge"),
                    (true, true, false) => say!("graph-cherry-pick-merge-staged"),
                    (false, false, true) => say!("graph-cherry-pick-at-head"),
                    (false, true, true) => say!("graph-cherry-pick-staged-at-head"),
                    (true, false, true) => say!("graph-cherry-pick-merge-at-head"),
                    (true, true, true) => say!("graph-cherry-pick-merge-staged-at-head"),
                },
                rewrite(Rewrite::CherryPick {
                    commit: commit.clone(),
                    merge: self.merge,
                    staged,
                }),
                at_head,
            )
        };
        match entry {
            E::Pull => menu.menu(say!("graph-pull"), named(LabelDo::Pull)),
            E::Push => menu.menu(say!("graph-push"), named(LabelDo::Push)),
            E::SetUpstream => menu.menu(say!("graph-set-upstream"), named(LabelDo::SetUpstream)),
            E::FastForward => match self.upstream {
                Some(upstream) => menu.menu_with_disabled(
                    if elsewhere {
                        say!("graph-fast-forward-elsewhere", upstream)
                    } else {
                        say!("graph-fast-forward", upstream)
                    },
                    named(LabelDo::FastForward),
                    elsewhere,
                ),
                None => menu,
            },
            E::Combine => rewrite::combine_entries(menu, self.current, &name, &name, at_head),
            E::Checkout => {
                let detached = Box::new(CheckoutTarget {
                    target: commit.clone(),
                });
                if label.tag || label.head {
                    return menu.menu(say!("graph-checkout-detached"), detached);
                }
                let branch = name.clone();
                menu.submenu(say!("graph-checkout"), window, cx, move |menu, _, _| {
                    menu.menu_with_disabled(
                        if elsewhere {
                            say!("graph-checkout-elsewhere", branch = branch.as_str())
                        } else {
                            branch.clone()
                        },
                        Box::new(CheckoutTarget {
                            target: branch.clone(),
                        }),
                        elsewhere,
                    )
                    .menu(say!("graph-checkout-this-commit"), detached.boxed_clone())
                })
            }
            E::Worktree => {
                let new = |start: &str| -> Box<dyn Action> {
                    Box::new(RunWorktree {
                        request: WorktreeRequest::New(Some(start.to_owned())),
                    })
                };
                if label.tag || label.head {
                    return menu.menu(say!("worktrees-new-here"), new(&commit));
                }
                let (for_branch, for_commit) = (new(&name), new(&commit));
                let branch = name.clone();
                menu.submenu(
                    say!("graph-worktree-from"),
                    window,
                    cx,
                    move |menu, _, _| {
                        menu.menu(format!("{branch}…"), for_branch.boxed_clone())
                            .menu(say!("graph-worktree-from-commit"), for_commit.boxed_clone())
                    },
                )
            }
            E::NewBranch => menu.menu(
                say!("graph-new-branch"),
                Box::new(BranchAt {
                    start: commit.clone(),
                }),
            ),
            E::CherryPick => pick(menu, false),
            E::CherryPickStaged => pick(menu, true),
            E::Reset => rewrite::reset_entries(menu, self.current, &commit, at_head, window, cx),
            E::Reword => menu.menu(
                say!("graph-reword"),
                rewrite(Rewrite::Interactive {
                    first: commit.clone(),
                    reword: true,
                }),
            ),
            E::Revert => menu.menu(
                if self.merge {
                    say!("graph-revert-merge")
                } else {
                    say!("graph-revert")
                },
                rewrite(Rewrite::Revert {
                    commit: commit.clone(),
                    merge: self.merge,
                }),
            ),
            E::Drop => menu.menu(say!("graph-drop"), rewrite(Rewrite::Drop { commit })),
            E::MoveDown => menu.menu_with_disabled(
                say!("graph-move-down"),
                rewrite(Rewrite::Move {
                    commits: vec![commit],
                    up: false,
                }),
                self.merge,
            ),
            E::ApplyPatch => menu.menu(
                say!("graph-apply-patch"),
                Box::new(crate::keymap::ApplyPatch),
            ),
            E::Rename => menu.menu(say!("graph-rename"), named(LabelDo::Rename)),
            E::Delete => {
                let why = if label.head {
                    Some(say!("graph-why-checked-out"))
                } else if elsewhere {
                    Some(say!("graph-why-elsewhere"))
                } else if self.protected {
                    Some(say!("graph-why-protected"))
                } else {
                    None
                };
                match why {
                    Some(why) => menu.menu_with_disabled(
                        say!("graph-delete-not", why),
                        named(LabelDo::Delete),
                        true,
                    ),
                    None => menu.menu(say!("graph-delete"), named(LabelDo::Delete)),
                }
            }
            E::DeleteOnRemote => menu.menu(
                say!("graph-delete-on-remote", name = name.as_str()),
                named(LabelDo::DeleteOnRemote),
            ),
            E::TagEntries => tags::tag_entries(menu, &name),
            E::CopyName => menu.menu(say!("graph-copy-name"), named(LabelDo::CopyName)),
            E::CopyId => menu.menu(say!("graph-copy-id"), Box::new(CopyId { id: commit })),
            E::SavePatch => menu.menu(
                say!("graph-save-patch"),
                Box::new(SavePatches {
                    commits: vec![commit],
                }),
            ),
            E::Seen => match self.seen {
                Some(seen) => crate::views::refs::own_visibility_entries(
                    menu,
                    &seen.full_name,
                    seen.hidden,
                    seen.soloed,
                    seen.pin,
                    seen.moves,
                ),
                None => menu,
            },
            E::Compare => menu.menu(say!("graph-compare"), Box::new(CompareAt { commit })),
            E::NewTag => tags::new_tag_entry(menu, &commit),
            E::Annotate => tags::annotate_entry(menu, &label.name),
            E::OpenWorktree => match &label.worktree {
                Some(path) => worktrees::open_worktree_entry(menu, path),
                None => menu,
            },
            // The open pull request's page and link, then the entry that
            // starts one.
            E::StartPull => crate::views::pulls::create::start_entry(
                crate::views::pulls::create::open_pull_entries(menu, self.pull.as_ref()),
                &name,
                label.head,
            ),
        }
    }

    /// The whole menu: the groups of [`label_groups`], a rule between two.
    /// It is longer than a small window is high, and scrolls there.
    fn menu(&self, menu: PopupMenu, window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
        let mut menu = crate::views::menu_fit::fit(menu, window);
        for (at, group) in label_groups(self.label, self.facts).iter().enumerate() {
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

/// The nearest branch of a commit without labels: what the faint label
/// says and what two clicks on it check out, when there is something to
/// check out.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Ghost {
    name: String,
    target: Option<String>,
}

/// The faint label for `badge`, which sits on `commit`.
fn ghost_of(badge: &Badge, marks: &[WorktreeMark], commit: Oid) -> Ghost {
    let labels = with_worktrees(ref_labels(std::slice::from_ref(badge)), marks, commit);
    Ghost {
        name: badge.name.clone(),
        target: labels.first().and_then(checkout_target),
    }
}

/// The rows that carry a branch (the checked-out one, a local or a remote
/// one), in order.
fn rows_with_branches(data: &GraphData) -> Vec<usize> {
    let mut rows: Vec<usize> = data
        .badges
        .iter()
        .filter(|(_, badges)| {
            badges.iter().any(|badge| {
                matches!(
                    badge.kind,
                    BadgeKind::Head | BadgeKind::Local | BadgeKind::Remote
                )
            })
        })
        .filter_map(|(id, _)| data.index.get(id).copied())
        .collect();
    rows.sort_unstable();
    rows.dedup();
    rows
}

/// The next of `rows`, which are in order, below or above `from`; from
/// nowhere, the first one going down.
fn next_of(rows: &[usize], from: Option<usize>, down: bool) -> Option<usize> {
    match (from, down) {
        (None, true) => rows.first().copied(),
        (None, false) => None,
        (Some(from), true) => rows.get(rows.partition_point(|row| *row <= from)).copied(),
        (Some(from), false) => rows
            .partition_point(|row| *row < from)
            .checked_sub(1)
            .and_then(|at| rows.get(at))
            .copied(),
    }
}

/// Whether a search text is a query for the engine rather than a few
/// letters to look for in the rows at hand: it has a word of the form
/// `key:…`, a quote, or a word that begins with a dash.
fn is_query(text: &str) -> bool {
    text.contains('"')
        || text.split_whitespace().any(|word| {
            let (negated, word) = match word.strip_prefix('-') {
                Some(rest) => (true, rest),
                None => (false, word),
            };
            let keyed = word.split_once(':').is_some_and(|(key, _)| {
                !key.is_empty() && key.chars().all(|c| c.is_ascii_alphabetic())
            });
            keyed || (negated && !word.is_empty())
        })
}

/// What was wrong with a query, without what the engine wraps it in.
fn query_error(err: &str) -> String {
    let said = err
        .rsplit_once("\"query\": ")
        .map_or(err, |(_, said)| said)
        .trim();
    let mut said = said.to_owned();
    if let Some(first) = said.get(..1) {
        said.replace_range(..1, &first.to_uppercase());
    }
    said
}

/// What is said while a large history is still being walked.
fn walking_text(walked: usize, rows: usize) -> String {
    say!(
        "graph-walking",
        rows = grouped(rows),
        walked = grouped(walked)
    )
}

/// A count with its thousands apart: 1 234 567 (thin spaces).
fn grouped(count: usize) -> String {
    let digits = count.to_string();
    let mut out = String::with_capacity(digits.len().saturating_add(digits.len() / 3));
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len().saturating_sub(at)).is_multiple_of(3) {
            out.push('\u{2009}');
        }
        out.push(digit);
    }
    out
}

/// What a stash is called without git's "On branch:" or "WIP on branch:"
/// in front of it.
pub fn stash_summary(summary: &str) -> &str {
    let rest = summary
        .strip_prefix("On ")
        .or_else(|| summary.strip_prefix("WIP on "));
    match rest.and_then(|rest| rest.split_once(": ")) {
        Some((_, message)) if !message.is_empty() => message,
        _ => summary,
    }
}

/// What checking a label out means: the local branch when there is one,
/// `remote/branch` for a remote-only label, the tag name for a tag (which
/// detaches). The checked-out branch and stashes have nothing to check out,
/// and a branch another worktree has checked out cannot be checked out
/// here as well.
fn checkout_target(label: &RefLabel) -> Option<String> {
    if label.head || label.stash || label.worktree.is_some() {
        None
    } else {
        Some(label.git_name())
    }
}

/// What a single label is when something is dropped on it: the branch or
/// tag it names, rather than whatever the row would pick among several. A
/// local branch is somewhere to merge into; a remote branch or a tag is
/// only somewhere to rebase onto or reset to. A label that names nothing
/// git knows by that name (a stash, a worktree's folder, a detached HEAD)
/// is no target of its own: the row is.
fn label_target(label: &RefLabel, commit: Oid) -> Option<DropTarget> {
    if !label.names_a_ref() || label.stash || label.name == "HEAD" {
        return None;
    }
    let branch = label.local || label.head;
    Some(DropTarget {
        commit,
        branch: branch.then(|| label.name.clone()),
        what: label.git_name(),
        name: label.git_name(),
    })
}

/// How wide the box is that says how many labels are folded (`hidden`).
fn folded_mark_width(hidden: usize) -> f32 {
    let more_digits = hidden.max(1).ilog10();
    LABELS_FOLDED_MARK + LABELS_FOLDED_DIGIT * more_digits as f32
}

/// How wide the one label a row shows may be in a label column `width`
/// wide: as wide as it needs, up to what the column leaves beside the
/// space before the labels, the box that says how many others are folded
/// (`hidden`, none when 0) and the least of the line towards the node.
/// Its name is cut, with an ellipsis, only where that is too little; what
/// it stands for stays.
fn label_cap(width: f32, hidden: usize) -> f32 {
    let beside = LABELS_LEFT
        + LABELS_CONNECTOR
        + if hidden > 0 {
            LABELS_GAP + folded_mark_width(hidden)
        } else {
            0.0
        };
    // A width that is no number has room for nothing.
    let room = if width.is_finite() {
        width - beside
    } else {
        0.0
    };
    room.max(0.0)
}

/// What is told, as a label is laid out, whether its name is cut.
type Measure = Rc<dyn Fn(bool)>;

/// A canvas over the room a label's name has, which tells `measure`
/// whether the whole name, in the text style it is drawn in, is wider
/// than that room: whether it is cut with an ellipsis (`widen`).
fn measure_name(name: String, measure: Measure) -> impl IntoElement {
    let name = SharedString::from(name);
    canvas(
        move |bounds, window, _| {
            let style = window.text_style();
            let size = style.font_size.to_pixels(window.rem_size());
            let run = style.to_run(name.len());
            let whole = window
                .text_system()
                .shape_line(name.clone(), size, &[run], None)
                .width;
            measure(whole > bounds.size.width + gpui_kit::px(0.05));
        },
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

/// Time bucket of a commit relative to `now`: (kind, value) with kind 0 =
/// hours ago, 1 = days ago, 2 = calendar day for older commits.
fn time_bucket(seconds: i64, now: i64) -> (u8, i64) {
    let delta = now.saturating_sub(seconds).max(0);
    if delta < 86_400 {
        (0, delta / 3600)
    } else if delta < 86_400 * 30 {
        (1, delta / 86_400)
    } else {
        (2, seconds.div_euclid(86_400))
    }
}

/// Label for a time bucket.
fn bucket_label(bucket: (u8, i64)) -> String {
    match bucket {
        (0, hours) => say!("graph-date-hours-ago", hours),
        (1, days) => say!("graph-date-days-ago", days),
        (_, day) => absolute_time(day.saturating_mul(86_400), 0)
            .chars()
            .take(10)
            .collect(),
    }
}

/// The date a row shows, with the offset it was written in: the author's or
/// the committer's, as preferred. Until the commit's text has been read
/// there is only the commit date the walk knows.
fn shown_date(
    detail: Option<&CommitInfo>,
    walked: Option<i64>,
    commit_dates: bool,
) -> (Option<i64>, i32) {
    match detail {
        Some(detail) if commit_dates => (Some(detail.committer.seconds), detail.committer.offset),
        Some(detail) => (Some(detail.author.seconds), detail.author.offset),
        None => (walked, 0),
    }
}

/// Rows of the commit at `row` and of the commits its first parents lead
/// to that are the branch's own: down the first parents, without what was
/// merged in, until a commit that another branch stands on or starts from
/// (`stops`), which is no longer only this branch's.
fn first_parent_line(
    commits: &[GraphCommit],
    index: &HashMap<Oid, usize>,
    stops: impl Fn(usize) -> bool,
    row: usize,
) -> HashSet<usize> {
    let mut line = HashSet::new();
    let mut at = Some(row).filter(|row| *row < commits.len());
    while let Some(ix) = at {
        if !line.insert(ix) {
            break; // a cycle can only come from a corrupt history
        }
        at = commits
            .get(ix)
            .and_then(|commit| commit.parents.first())
            .and_then(|parent| index.get(parent).copied())
            .filter(|parent| !stops(*parent));
    }
    line
}

/// Stroke a finished path, ignoring degenerate ones.
fn stroke(window: &mut Window, path: PathBuilder, color: Hsla) {
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// How a lane line is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Line {
    Solid,
    /// The lane of uncommitted work.
    Dashed,
    /// A stash's line down to the commit it was made on.
    Dotted,
}

impl From<bool> for Line {
    fn from(dashed: bool) -> Self {
        if dashed { Line::Dashed } else { Line::Solid }
    }
}

/// A stroke for lane lines: dashed for the lane of uncommitted work,
/// dotted for a stash's.
fn lane_stroke(line: impl Into<Line>) -> PathBuilder {
    let path = PathBuilder::stroke(px(STROKE));
    match line.into() {
        Line::Solid => path,
        Line::Dashed => path.dash_array(&[px(DASH[0]), px(DASH[1])]),
        Line::Dotted => path.dash_array(&[px(DOTS[0]), px(DOTS[1])]),
    }
}

/// Paint a lane between two heights.
fn paint_lane(
    window: &mut Window,
    x: Pixels,
    from: Pixels,
    to: Pixels,
    color: Hsla,
    line: impl Into<Line>,
) {
    let mut path = lane_stroke(line);
    path.move_to(point(x, from));
    path.line_to(point(x, to));
    stroke(window, path, color);
}

/// Paint the edge between a node and a lane in another slot: the lane from
/// `end` (the row's top for a lane ending in this node, its bottom for a
/// lane leading to a parent), a rounded corner, then a level run into the
/// node at height `mid`.
fn paint_edge(
    window: &mut Window,
    node_x: Pixels,
    mid: Pixels,
    lane_x: Pixels,
    end: Pixels,
    color: Hsla,
    line: Line,
) {
    let across = crate::zoom::unpx(node_x - lane_x);
    let along = crate::zoom::unpx(end - mid);
    let radius = CORNER.min(across.abs()).min(along.abs());
    let turn_y = px(radius * along.signum());
    let turn_x = px(radius * across.signum());
    // Cubic approximation of a quarter circle.
    let ease = 1.0 - 0.5523;
    let mut path = lane_stroke(line);
    path.move_to(point(lane_x, end));
    path.line_to(point(lane_x, mid + turn_y));
    path.cubic_bezier_to(
        point(lane_x + turn_x, mid),
        point(lane_x, mid + turn_y * ease),
        point(lane_x + turn_x * ease, mid),
    );
    path.line_to(point(node_x, mid));
    stroke(window, path, color);
}

/// Paint a filled circle.
fn paint_disc(window: &mut Window, centre: Point<Pixels>, radius: Pixels, color: Hsla) {
    window.paint_quad(
        fill(
            Bounds::new(
                point(centre.x - radius, centre.y - radius),
                size(radius * 2.0, radius * 2.0),
            ),
            color,
        )
        .corner_radii(radius),
    );
}

/// Paint a filled square with rounded corners, `half` from its centre to
/// its sides.
fn paint_square(
    window: &mut Window,
    centre: Point<Pixels>,
    half: Pixels,
    corner: Pixels,
    color: Hsla,
) {
    window.paint_quad(
        fill(
            Bounds::new(
                point(centre.x - half, centre.y - half),
                size(half * 2.0, half * 2.0),
            ),
            color,
        )
        .corner_radii(corner),
    );
}

/// Paint a dotted square outline, `half` from its centre to its sides.
fn paint_dotted_square(window: &mut Window, centre: Point<Pixels>, half: Pixels, color: Hsla) {
    let (x, y) = (centre.x, centre.y);
    let mut path = lane_stroke(Line::Dotted);
    path.move_to(point(x - half, y - half));
    path.line_to(point(x + half, y - half));
    path.line_to(point(x + half, y + half));
    path.line_to(point(x - half, y + half));
    path.close();
    stroke(window, path, color);
}

/// Paint a dashed circle outline.
fn paint_dashed_ring(window: &mut Window, centre: Point<Pixels>, radius: Pixels, color: Hsla) {
    let bend = radius * 0.5523;
    let (x, y) = (centre.x, centre.y);
    let mut path = lane_stroke(true);
    path.move_to(point(x + radius, y));
    path.cubic_bezier_to(
        point(x, y + radius),
        point(x + radius, y + bend),
        point(x + bend, y + radius),
    );
    path.cubic_bezier_to(
        point(x - radius, y),
        point(x - bend, y + radius),
        point(x - radius, y + bend),
    );
    path.cubic_bezier_to(
        point(x, y - radius),
        point(x - radius, y - bend),
        point(x - bend, y - radius),
    );
    path.cubic_bezier_to(
        point(x + radius, y),
        point(x + bend, y - radius),
        point(x + radius, y - bend),
    );
    path.close();
    stroke(window, path, color);
}

/// Everything the canvas of one row needs to paint its part of the graph.
struct RowPaint {
    data: Arc<GraphData>,
    /// Commit index, or `None` for the row of uncommitted work.
    ix: Option<usize>,
    node: Node,
    /// The line from the row's labels, continued up to the node.
    label_line: LabelLine,
    /// The row is selected: its band is filled rather than washed.
    selected: bool,
    /// The lanes of stashes that pass this row on their way down to the
    /// commit they were made on, or end in its node: drawn dotted.
    stash_slots: Vec<u16>,
    /// Slot of the lane of uncommitted work while it passes this row.
    wip_slot: Option<u16>,
    /// That lane ends in this row's node.
    wip_ends_here: bool,
    /// The rows above and below are not its neighbours in the history
    /// (only what a search found is shown): the node alone, without lanes.
    alone: bool,
    colors: Vec<Hsla>,
    background: Hsla,
    is_dark: bool,
    scroll_x: f32,
}

impl RowPaint {
    fn color(&self, id: u32) -> Hsla {
        // As many colours as the theme in use has for lanes.
        let count = u32::try_from(self.colors.len()).unwrap_or(u32::MAX).max(1);
        self.colors
            .get((id % count) as usize)
            .copied()
            .unwrap_or_else(|| hsla(0.6, 0.5, 0.5, 1.0))
    }

    /// This row's layout and the layout of the row above it.
    fn rows(&self) -> (Option<&GraphRow>, Option<&GraphRow>) {
        match self.ix {
            None => (self.data.wip.as_ref(), None),
            Some(ix) => (
                self.data.rows.get(ix),
                match ix.checked_sub(1) {
                    Some(above) => self.data.rows.get(above),
                    None => self.data.wip.as_ref(),
                },
            ),
        }
    }

    fn paint(&self, bounds: Bounds<Pixels>, window: &mut Window) {
        let (Some(row), above) = self.rows() else {
            return;
        };
        let geometry = LaneGeometry {
            left: bounds.origin.x,
            width: crate::zoom::unpx(bounds.size.width),
            scroll_x: self.scroll_x,
        };
        let left = bounds.origin.x;
        let right = left + bounds.size.width;
        let top = bounds.origin.y;
        let bottom = top + bounds.size.height;
        let mid = top + bounds.size.height / 2.0;
        let node_x = geometry.x(row.lane);
        let node_color = self.color(row.color);
        // A commit whose lane is out of the column is drawn at the column's
        // nearer edge, under the shade there, so that every row has one.
        let (pinned_x, pinned) = pinned_node(&geometry, row.lane);

        // The row's band: a wash from the node to the column edge, closed by
        // a solid bar that colour-codes the message next to it. Both leave
        // the row's top and bottom free; a selected row's band is filled.
        let band_top = top + px(BAND_INSET);
        let band_height = (bounds.size.height - px(2.0 * BAND_INSET)).max(px(0.0));
        if pinned_x < right {
            let from = pinned_x.max(left);
            window.paint_quad(fill(
                Bounds::new(point(from, band_top), size(right - from, band_height)),
                node_color.opacity(if self.selected { SELECTED_BAND } else { 0.10 }),
            ));
        }
        window.paint_quad(fill(
            Bounds::new(
                point(right - px(BAND_BAR), band_top),
                size(px(BAND_BAR), band_height),
            ),
            node_color,
        ));
        // The line from the labels, continued up to the node.
        if self.label_line != LabelLine::None && pinned_x > left {
            let (thick, strength) = self.label_line.stroke();
            window.paint_quad(fill(
                Bounds::new(
                    point(left, top + px((ROW_HEIGHT - thick) / 2.0)),
                    size(pinned_x.min(right) - left, px(thick)),
                ),
                node_color.opacity(strength),
            ));
        }

        // Lanes passing through.
        let lanes = !self.alone;
        for lane in row
            .straight
            .iter()
            .filter(|l| lanes && geometry.shows(l.slot))
        {
            paint_lane(
                window,
                geometry.x(lane.slot),
                top,
                bottom,
                self.color(lane.color),
                if self.wip_slot == Some(lane.slot) {
                    Line::Dashed
                } else if self.stash_slots.contains(&lane.slot) {
                    Line::Dotted
                } else {
                    Line::Solid
                },
            );
        }
        // The node's own lane, above and below it.
        if lanes && geometry.shows(row.lane) {
            if above.is_some_and(|above| above.leaves(row.lane)) {
                paint_lane(window, node_x, top, mid, node_color, self.wip_ends_here);
            }
            if row.continues {
                let line = match self.node {
                    Node::Wip => Line::Dashed,
                    Node::Stash => Line::Dotted,
                    _ => Line::Solid,
                };
                paint_lane(window, node_x, mid, bottom, node_color, line);
            }
        }
        // Lanes ending in this node come from the top; lanes to further
        // parents leave through the bottom. The farthest go first so that
        // the corners of nearer ones stay on top.
        let mut edges: Vec<(u16, u32, Pixels)> = row
            .merges_in
            .iter()
            .map(|lane| (lane.slot, lane.color, top))
            .chain(row.forks.iter().map(|lane| (lane.slot, lane.color, bottom)))
            .filter(|(slot, _, _)| lanes && *slot != row.lane && geometry.spans(*slot, row.lane))
            .collect();
        edges.sort_by_key(|(slot, _, _)| std::cmp::Reverse(slot.abs_diff(row.lane)));
        for (slot, color, end) in edges {
            // A stash's line into the commit it was made on, or out of the
            // stash to it, is dotted.
            let dotted = (end == top && self.stash_slots.contains(&slot))
                || (end == bottom && matches!(self.node, Node::Stash));
            paint_edge(
                window,
                node_x,
                mid,
                geometry.x(slot),
                end,
                self.color(color),
                if dotted { Line::Dotted } else { Line::Solid },
            );
        }

        // The shade over a side of the column that has lanes beyond it.
        // Beyond the right edge: the outermost lane's node does not fit.
        let outermost = self.data.max_width.saturating_sub(1);
        let beyond = geometry.offset(outermost) + NODE_RING > geometry.width;
        let shade = self.background.opacity(PIN_SHADE_STRENGTH);
        if self.scroll_x > 0.0 {
            window.paint_quad(fill(
                Bounds::new(point(left, top), size(px(PIN_SHADE), bounds.size.height)),
                shade,
            ));
        }
        if beyond {
            window.paint_quad(fill(
                Bounds::new(
                    point(right - px(PIN_SHADE + BAND_BAR), top),
                    size(px(PIN_SHADE), bounds.size.height),
                ),
                shade,
            ));
        }
        if pinned && self.alone {
            return;
        }
        let centre = point(pinned_x, mid);
        match &self.node {
            Node::Wip => {
                paint_disc(window, centre, px(NODE_RING), self.background);
                paint_dashed_ring(window, centre, px(NODE_RING - STROKE / 2.0), node_color);
            }
            Node::Merge => paint_disc(window, centre, px(MERGE_DOT), node_color),
            Node::Stash => {
                // A dotted square as large as a commit's node, over the
                // band, with a small square of its colour in it.
                paint_square(window, centre, px(STASH_BOX), px(0.0), self.background);
                paint_dotted_square(window, centre, px(STASH_BOX - STROKE / 2.0), node_color);
                paint_square(window, centre, px(MERGE_DOT - 1.0), px(1.0), node_color);
            }
            Node::Commit(avatar_key, picture) => {
                paint_disc(window, centre, px(NODE_RING), node_color);
                paint_disc(window, centre, px(NODE_RING - STROKE), self.background);
                if let Some(picture) = picture {
                    crate::views::avatar::paint_picture(
                        window,
                        picture,
                        centre,
                        px(NODE_RING - STROKE),
                    );
                    return;
                }
                match avatar_key {
                    Some(key) => {
                        let cell = px(2.6);
                        paint_identicon(
                            window,
                            key,
                            point(pinned_x - cell * 2.5, mid - cell * 2.5),
                            cell,
                            self.is_dark,
                        );
                    }
                    None => paint_disc(window, centre, px(NODE_RADIUS), node_color.opacity(0.5)),
                }
            }
        }
    }
}

/// Most rows one range can add to the selection: a slip of the hand on a
/// history of a million commits is not worth a million entries.
const MAX_RANGE: usize = 5000;

/// What a click on `row` does to the selected rows. Alone it leaves that
/// one row selected; with the toggle key it adds the row or takes it out;
/// with the range key it selects everything from where the last plain or
/// toggling click was to this row. `picked` holds the rows when there are
/// several and is empty when there is at most one, which `cursor` names.
/// Returns the row the cursor is on afterwards.
fn click_rows(
    picked: &mut BTreeSet<usize>,
    anchor: &mut Option<usize>,
    cursor: Option<usize>,
    row: usize,
    toggle: bool,
    range: bool,
) -> Option<usize> {
    let landed = match (*anchor, range) {
        (Some(from), true) => {
            if !toggle {
                picked.clear();
            } else if picked.is_empty() {
                picked.extend(cursor);
            }
            // From the anchor towards the row, as far as is allowed.
            let reach = from.abs_diff(row).min(MAX_RANGE);
            let (low, high) = if row >= from {
                (from, from.saturating_add(reach))
            } else {
                (from.saturating_sub(reach), from)
            };
            picked.extend(low..=high);
            Some(if row >= from { high } else { low })
        }
        _ if toggle => {
            if picked.is_empty() {
                picked.extend(cursor);
            }
            *anchor = Some(row);
            if picked.remove(&row) {
                // The cursor stays where it was unless it was this row.
                cursor
                    .filter(|at| *at != row)
                    .or_else(|| picked.iter().next().copied())
            } else {
                picked.insert(row);
                Some(row)
            }
        }
        _ => {
            picked.clear();
            *anchor = Some(row);
            Some(row)
        }
    };
    // One row is not "several".
    if picked.len() < 2 {
        let only = picked.iter().next().copied();
        picked.clear();
        return landed.or(only);
    }
    landed
}

/// How wide the field of the row of uncommitted work is for what is typed
/// in the summary: 96 while there is nothing, then as wide as the text
/// needs, up to 176, where a longer summary is cut.
fn wip_field_width(typed: &str) -> f32 {
    const EMPTY: f32 = 96.0;
    const LETTER: f32 = 7.5;
    const PADDING: f32 = 18.0;
    let letters = typed.trim().chars().count();
    if letters == 0 {
        return EMPTY;
    }
    (PADDING + LETTER * letters as f32).clamp(EMPTY, 176.0)
}

/// The stashes of a graph: for each, its row, its lane, and the row of the
/// commit it was made on (its first parent), between which its line is
/// drawn dotted. A stash whose commit is not among the rows has none.
fn stash_lanes(data: &GraphData) -> Vec<(usize, u16, usize)> {
    let mut lanes: Vec<(usize, u16, usize)> = data
        .badges
        .iter()
        .filter(|(_, badges)| badges.iter().any(|badge| badge.kind == BadgeKind::Stash))
        .filter_map(|(id, _)| {
            let row = *data.index.get(id)?;
            let base = data.commits.get(row)?.parents.first()?;
            let base_row = *data.index.get(base)?;
            let lane = data.rows.get(row)?.lane;
            Some((row, lane, base_row))
        })
        .collect();
    lanes.sort_unstable();
    lanes
}

/// The lanes of stashes that pass the row `ix` or end in its node.
fn stash_slots_at(lanes: &[(usize, u16, usize)], ix: usize) -> Vec<u16> {
    lanes
        .iter()
        .filter(|(row, _, base)| *row < ix && ix <= *base)
        .map(|(_, lane, _)| *lane)
        .collect()
}

/// Whether `newest_first` are commits that follow one another, each the
/// first parent of the one before.
fn is_run(data: &GraphData, newest_first: &[Oid]) -> bool {
    newest_first.len() >= 2
        && newest_first.windows(2).all(|pair| match pair {
            [newer, older] => data
                .index
                .get(newer)
                .and_then(|ix| data.commits.get(*ix))
                .is_some_and(|commit| commit.parents.first() == Some(older)),
            _ => false,
        })
}

/// What a selection of several commits can be compared as. `newest_first`
/// is the selection in row order with each commit's parents. A run in
/// which every commit is the first parent of the one before is shown as
/// what those commits change together; any two commits as what differs
/// between them; anything else has no single diff.
fn comparison(newest_first: &[(Oid, Vec<Oid>)]) -> Option<Comparison> {
    let (newest, _) = newest_first.first()?;
    let (oldest, oldest_parents) = newest_first.last()?;
    let chain = newest_first.len() >= 2
        && newest_first
            .iter()
            .zip(newest_first.iter().skip(1))
            .all(|((_, parents), (below, _))| parents.first() == Some(below));
    if chain {
        return Some(Comparison {
            old: oldest_parents.first().copied(),
            new: *newest,
            together: true,
        });
    }
    (newest_first.len() == 2).then_some(Comparison {
        old: Some(*oldest),
        new: *newest,
        together: false,
    })
}

/// Two ends to diff for a selection of several commits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Comparison {
    /// The older end; none for the empty tree (the selection reaches the
    /// first commit).
    pub old: Option<Oid>,
    /// The newer end.
    pub new: Oid,
    /// The ends are around a run of commits: the diff is what those
    /// commits change together. Otherwise it is what differs between two
    /// commits that were picked.
    pub together: bool,
}

/// The uncommitted files by what happened to them, for the row that stands
/// for them: the open worktree's, or another one's.
pub use bitbraid_git::wip::Counts as WipCounts;

/// Events from the graph.
#[derive(Debug, Clone)]
pub enum GraphEvent {
    /// Selection changed.
    Selected {
        /// The commit that is selected, if one is.
        id: Option<Oid>,
        /// Whether the user selected it (a click, the keys), rather than
        /// the selection coming back by itself with a graph that was
        /// loaded or said again: what is shown of it is then theirs to
        /// have fetched in a partial clone.
        asked: bool,
    },
    /// Show this issue in the middle: an issue key of a summary was
    /// clicked.
    OpenIssue {
        /// Its repository.
        link: bitbraid_integrations::model::RepoLink,
        /// Its number.
        number: u64,
    },
    /// Several commits are selected, newest first, and what they can be
    /// compared as.
    SelectedMany {
        /// The commits in row order.
        ids: Vec<Oid>,
        /// The two ends to diff, when the selection has such.
        comparison: Option<Comparison>,
        /// Whether the user made the selection, as for `Selected`.
        asked: bool,
    },
    /// The work-in-progress row was selected.
    SelectedWip,
    /// The row of another worktree's uncommitted work was selected: the
    /// worktree's path.
    SelectedWorktree(String),
    /// Check this out: a branch, `remote/branch`, or a commit id.
    Checkout(String),
    /// Ask for a name and create a branch at this commit or reference.
    NewBranch(String),
    /// Merge, rebase, reset, pick or revert.
    Rewrite(Rewrite),
    /// Stash, or use a stash.
    Stash(StashRequest),
    /// Make a tag.
    Tag(TagRequest),
    /// Add a worktree, or open one.
    Worktree(WorktreeRequest),
    /// Write these commits (ids, oldest first) to files as patches.
    ExportPatches(Vec<String>),
    /// Something was dropped on a row and this is what was chosen.
    Dropped(Choice),
    /// Pull, push, rename, delete or otherwise act on the branch a label
    /// names.
    Branch(BranchRequest),
    /// Hide, show alone or pin the reference a label names: what the graph
    /// shows of it, which the references panel keeps.
    Seen(Seen),
    /// Something asked of the uncommitted work as a whole.
    Wip(WipAction),
    /// The field of the row of uncommitted work was clicked: the summary
    /// of the commit is typed in the commit form, which it shows.
    WriteSummary,
    /// A name was typed in the field of the label column: make what it
    /// names at `start` (HEAD when none).
    Named {
        /// What the name is for.
        what: NameFor,
        /// Where: a commit's id or a reference's name.
        start: Option<String>,
        /// The name, trimmed.
        name: String,
    },
}

/// What a name typed into the label column of a commit's row makes: a new
/// branch or tag is named there, on the commit's row, rather than in a
/// dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameFor {
    /// A branch, checked out.
    Branch,
    /// A tag without a message of its own.
    Tag,
    /// A tag with a message, which is asked for next.
    AnnotatedTag,
}

impl NameFor {
    /// What the empty field says.
    fn placeholder(self) -> &'static str {
        match self {
            NameFor::Branch => say!("graph-name-branch"),
            NameFor::Tag | NameFor::AnnotatedTag => say!("graph-name-tag"),
        }
    }

    /// What the probe calls it.
    pub fn word(self) -> &'static str {
        match self {
            NameFor::Branch => "branch",
            NameFor::Tag => "tag",
            NameFor::AnnotatedTag => "annotated_tag",
        }
    }
}

/// The field in the label column of a row, while a name is typed there.
struct Naming {
    /// The row of the list it stands on.
    row: usize,
    start: Option<String>,
    what: NameFor,
    field: Entity<InputState>,
    _subscription: Subscription,
}

/// Width and height of that field.
const NAME_FIELD_WIDTH: f32 = 128.0;
const NAME_FIELD_HEIGHT: f32 = 28.0;
/// Where it begins, from the label column's left edge.
const NAME_FIELD_LEFT: f32 = 7.0;

/// A column border that can be dragged to make the column wider or
/// narrower.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Resize {
    Refs,
    Graph,
    /// One of the columns after the graph, which is not the last shown.
    Column(Column),
}

/// Nothing follows the pointer while a border is dragged.
struct NoChip;

impl Render for NoChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The row `delta` steps from `current` among `rows`, which are in order:
/// what a step is while only those rows are shown. From a row that is not
/// among them, the first step goes to its neighbour on that side; past
/// either end it stays at the end.
fn step_among(rows: &[usize], current: Option<usize>, delta: isize) -> Option<usize> {
    let last = isize::try_from(rows.len().checked_sub(1)?).unwrap_or(isize::MAX);
    let at = match current.map(|current| rows.binary_search(&current)) {
        None => 0,
        Some(Ok(at)) => isize::try_from(at)
            .unwrap_or(isize::MAX)
            .saturating_add(delta),
        Some(Err(at)) => isize::try_from(at)
            .unwrap_or(isize::MAX)
            .saturating_add(delta)
            .saturating_sub(isize::from(delta > 0)),
    };
    rows.get(usize::try_from(at.clamp(0, last)).unwrap_or(0))
        .copied()
}

/// How much of a commit's id the id column shows: six characters, at any
/// width.
const ID_LETTERS: usize = 6;

/// The least a column after the graph is: a message still says something,
/// an author is their picture, a date and an id are cut.
fn least_of(column: Column) -> f32 {
    match column {
        Column::Summary => 120.0,
        Column::Author => 40.0,
        Column::Date => 58.0,
        Column::Id => 55.0,
    }
}

/// How wide each column after the graph is, in the order `shown` has them
/// with the width each would have, when `room` is what the label and graph
/// columns leave: each its own width and the last what is left. Where that
/// is less than the last one's least, the columns before it give way from
/// the right, the message last, each down to its least. With no room known
/// (nothing drawn yet) every one has its own width.
fn fit_columns(shown: &[(Column, f32)], room: f32) -> Vec<f32> {
    let mut widths: Vec<f32> = shown
        .iter()
        .map(|(column, width)| {
            if width.is_finite() {
                width.max(least_of(*column))
            } else {
                least_of(*column)
            }
        })
        .collect();
    let Some(((last, _), rest)) = shown.split_last() else {
        return widths;
    };
    if !room.is_finite() || room <= 0.0 {
        return widths;
    }
    let others: f32 = widths.iter().rev().skip(1).sum();
    let mut left = room - others;
    let least_last = least_of(*last);
    if left < least_last {
        let mut short = least_last - left;
        // From the right, the message last.
        let mut order: Vec<usize> = (0..rest.len())
            .rev()
            .filter(|ix| {
                rest.get(*ix)
                    .is_some_and(|(column, _)| *column != Column::Summary)
            })
            .collect();
        order.extend((0..rest.len()).filter(|ix| {
            rest.get(*ix)
                .is_some_and(|(column, _)| *column == Column::Summary)
        }));
        for ix in order {
            if short <= 0.0 {
                break;
            }
            let (Some((column, _)), Some(width)) = (rest.get(ix), widths.get_mut(ix)) else {
                continue;
            };
            let give = (*width - least_of(*column)).clamp(0.0, short);
            *width -= give;
            short -= give;
        }
        left = least_last.max(least_last - short);
    }
    if let Some(width) = widths.last_mut() {
        *width = left.max(least_last);
    }
    widths
}

/// The order of the columns after `column` was put in the place of `to`:
/// the others close up behind it. Unchanged when either is not shown.
fn moved_column(order: &[Column], column: Column, to: Column) -> Vec<Column> {
    let (Some(from), Some(at)) = (
        order.iter().position(|shown| *shown == column),
        order.iter().position(|shown| *shown == to),
    ) else {
        return order.to_vec();
    };
    let mut moved = order.to_vec();
    let taken = moved.remove(from);
    moved.insert(at.min(moved.len()), taken);
    moved
}

/// The columns after the graph, in their order, each with its width.
fn shown_columns(columns: &crate::prefs::Columns) -> Vec<(Column, f32)> {
    columns
        .order
        .iter()
        .map(|column| (*column, columns.width_of(*column)))
        .collect()
}

/// The width a column gets when its right border is dragged to `x`
/// pixels from its left edge. Columns keep a size that can still be
/// grabbed and read, however far the pointer goes.
fn resized(which: Resize, x: f32) -> f32 {
    let (least, most) = match which {
        Resize::Refs => (60.0, 480.0),
        Resize::Graph => (48.0, 900.0),
        Resize::Column(column) => (least_of(column), 900.0),
    };
    if x.is_finite() {
        x.clamp(least, most)
    } else {
        least
    }
}

/// What a drop can mean, shown where it happened until one is picked.
struct DropMenu {
    position: Point<Pixels>,
    choices: Vec<Choice>,
}

pub struct GraphView {
    session: SessionHandle,
    data: Option<Arc<GraphData>>,
    details: HashMap<Oid, CommitInfo>,
    pending: HashSet<Oid>,
    selected: Option<usize>,
    hovered: Option<usize>,
    /// Nearest branch of the hovered commit row, looked up once per hover.
    ghost: Option<(usize, Option<Ghost>)>,
    /// The row whose labels are unfolded.
    unfold: Unfold,
    /// The row of the list whose unfolded labels' menu is open, with the
    /// commit it shows: they stay as they are until it is dismissed.
    labels_held: Option<(usize, usize)>,
    /// The label the pointer rests on, widened to its whole name when that
    /// is cut.
    widen: widen::Widen,
    /// The label the secondary button went down on, with its row: the menu
    /// that opens is then that label's own.
    menu_label: Option<(usize, RefLabel)>,
    /// The rows that carry a branch, found when a jump first asks for them
    /// and kept until the graph changes.
    branch_rows: Option<Rc<Vec<usize>>>,
    /// The commit of the branch the pointer is on in the references panel.
    panel_hover: Option<Oid>,
    /// The run of commits lit for the branch whose labels the pointer rests
    /// on, here or in the references panel: what that row's first parents
    /// lead to. What every other row says steps back, a moment after the
    /// pointer came and over a moment.
    light: light::Light,
    /// Time-bucket divider label above each commit row, when the bucket changes.
    dividers: Vec<Option<String>>,
    scroll: VirtualListScrollHandle,
    /// The commit at the top of the list as last seen, with how far into
    /// its row the list was scrolled, and whether that is still to be
    /// written down: it is, a moment after the scrolling stops.
    top_seen: Option<(Oid, f32)>,
    top_saving: bool,
    /// The list was put back where it was scrolled to the last time, or
    /// that is no longer to be done.
    top_restored: bool,
    /// While that place is being put back: the list holds an offset to
    /// the end it has in the window as it is, and in its first frames the
    /// window may be larger than the window manager then makes it.
    restoring: Cell<Option<Restoring>>,
    /// How wide the list was when the columns' heading was last laid out
    /// for it: the heading is laid out before the list learns its width
    /// in a frame, the rows after (`render_header`). What the rows found
    /// it to be, and whether they asked for the frame that lays the
    /// heading out again.
    heading_drawn: Cell<f32>,
    heading_pane: Cell<f32>,
    heading_asked: Cell<bool>,
    /// Where the pointer went down on a column's border to resize it, in
    /// the window, and where that border's line was, in points from the
    /// heading's left edge (`resize_handle`).
    resize_grab: Rc<Cell<Option<(Pixels, f32)>>>,
    item_sizes: crate::zoom::Sizes,
    focus_handle: FocusHandle,
    search: Entity<InputState>,
    search_text: String,
    /// A query with words like `author:` in it was handed to the engine:
    /// how many times that was done (the answer to an older one is not
    /// taken), whether the answer is still out, and what was wrong with
    /// the query when something was.
    search_asked: u64,
    search_pending: bool,
    /// The search was stopped before it answered.
    search_stopped: bool,
    search_error: Option<String>,
    matches: Vec<usize>,
    match_pos: usize,
    now: i64,
    bench: Option<Bench>,
    frames_rendered: u64,
    /// Startup milestones already logged: first render with a graph, first details.
    logged: (bool, bool),
    /// Staged and unstaged counts of the last status. The row of uncommitted
    /// work itself is part of the graph data, which lays out its lane.
    wip: Option<WipCounts>,
    /// An operation stopped part way: what it says about itself, and
    /// whether it cannot go on as things are.
    stopped: Option<(String, bool)>,
    /// The rows that are selected when there are several (the selected
    /// row among them), and the row a range is taken from.
    picked: BTreeSet<usize>,
    anchor: Option<usize>,
    /// The search bar is shown over the top of the graph.
    search_open: bool,
    /// Only the rows a search found are shown, while there is a search.
    only_matches: bool,
    /// Horizontal scroll offset of the graph column in pixels.
    scroll_x: f32,
    /// The checked-out branch, for the wording of the row menu.
    current_branch: Option<String>,
    /// What is uncommitted in each of the other worktrees, by path, as
    /// last looked at.
    worktree_changes: HashMap<String, WipCounts>,
    /// The worktrees being removed, by path.
    removing: BTreeSet<String>,
    /// The commits that carry a signature, among those asked about.
    signatures: HashMap<Oid, Signed>,
    signed_asked: HashSet<Oid>,
    /// Signed commits git has not been asked about yet, and whether it is
    /// being asked about some right now: one batch at a time.
    verify_queue: Vec<Oid>,
    verifying: bool,
    /// Open after a drop.
    drop_menu: Option<DropMenu>,
    /// The field in which a new branch or tag is named, on its row.
    naming: Option<Naming>,
    /// The tags without a message of their own.
    lightweight: HashSet<String>,
    /// The summary field of the commit form: what is typed there is what
    /// the row of uncommitted work shows where a message would be.
    summary_source: Option<Entity<InputState>>,
    /// A branch, a tag or a commit is being dragged over the graph: the
    /// labels it could be dropped on are outlined.
    label_drag: bool,
    /// For each stash in the graph: its row, its lane, and the row of the
    /// commit it was made on (see [`stash_lanes`]).
    stash_lanes: Rc<Vec<(usize, u16, usize)>>,
    /// Select the row of uncommitted work as soon as there is one.
    want_wip: bool,
    /// Nothing is selected because the selected row was clicked again: a
    /// graph that comes meanwhile selects nothing either.
    deselected: bool,
    /// The checks of the commits in view, and the branches with an open
    /// pull request (`graph/checks.rs`).
    checks: crate::views::pulls::checks::Checks,
    pull_marks: Rc<HashMap<String, crate::views::pulls::facts::PullMark>>,
    _subscriptions: Vec<Subscription>,
}

/// Auto-scroll benchmark driven by `BITBRAID_BENCH=<seconds>`. With
/// `BITBRAID_BENCH_RESIZE` set as well it drags the graph column's border
/// to and fro instead (`cargo xtask appbench --resize`).
struct Bench {
    started: std::time::Instant,
    duration: std::time::Duration,
    position: usize,
    forward: bool,
    frames: u64,
    second_start: std::time::Instant,
    second_frames: u64,
    samples: Vec<f64>,
    running: bool,
    /// The window the resizing run sends its pointer's events to, when
    /// the run resizes.
    resize: Option<AnyWindowHandle>,
    /// Where the border was when the resizing run took it, from the
    /// heading's left edge, and the graph column's width then.
    border: Option<(f32, f32)>,
    /// How far right of that the pointer is, in points.
    shift: f32,
    /// Events the resizing run has sent, and widths it brought about.
    sent: u64,
    resizes: u64,
    /// How long the main thread had run, and how many frames were drawn,
    /// when the first second (which is not counted) was over.
    counted_from: Option<(std::time::Duration, u64)>,
}

/// What the measuring run does after a step.
enum BenchStep {
    Next,
    Send(AnyWindowHandle, PlatformInput),
    Done,
}

/// How far the resizing run moves the pointer each step, and how far it
/// takes the border past where it was at most.
const BENCH_DRAG_STEP: f32 = 2.0;
const BENCH_DRAG_REACH: f32 = 160.0;

impl EventEmitter<GraphEvent> for GraphView {}

/// The row at the top of a list scrolled by `offset` (negative as the
/// toolkit has it), for rows of one height.
fn top_row(offset: f32, row_height: f32) -> usize {
    if !offset.is_finite() || row_height <= 0.0 {
        return 0;
    }
    let rows = (-offset / row_height).floor();
    if rows <= 0.0 { 0 } else { rows as usize }
}

/// How far into the row at the top (`row`) a list scrolled by `offset` is
/// scrolled, for rows of one height: from nought up to the row's height,
/// to a hundredth of a point. Nought for what is no number.
fn into_row(offset: f32, row_height: f32, row: usize) -> f32 {
    let into = -offset - row as f32 * row_height;
    if into.is_finite() && row_height > 0.0 {
        (into.clamp(0.0, row_height) * 100.0).round() / 100.0
    } else {
        0.0
    }
}

/// The place the list was scrolled to at the last start while it is put
/// back: the offset wanted, and the one the list was drawn at the last
/// time, in points (negative as the toolkit has them).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Restoring {
    wanted: f32,
    drawn: f32,
}

impl GraphView {
    /// Note which commit is at the top of the list, and how far into its
    /// row the list is scrolled, and when that changed, write it down a
    /// moment later: not on every frame of a scroll.
    fn note_top(&mut self, cx: &mut Context<Self>) {
        // While only what a search found is shown, the list is not where
        // the graph is scrolled to.
        if self.shown().is_some() {
            return;
        }
        let Some(data) = &self.data else {
            return;
        };
        let offset = crate::zoom::unpx(self.scroll.base_handle().offset().y);
        let row = top_row(offset, ROW_HEIGHT);
        let top = self
            .commit_ix(row.max(self.offset()))
            .and_then(|ix| data.commits.get(ix))
            .map(|commit| (commit.id, into_row(offset, ROW_HEIGHT, row)));
        if top == self.top_seen {
            return;
        }

        let first = self.top_seen.is_none();
        self.top_seen = top;
        // Where the list starts out is where it was; nothing to write.
        if first || self.top_saving {
            return;
        }
        self.top_saving = true;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(700))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.top_saving = false;
                let seen = this.top_seen;
                let path = this.session.path.clone();
                cx.global_mut::<Prefs>().update_repo_state(&path, |state| {
                    state.top = seen.map(|(id, _)| id.to_hex());
                    state.top_into = seen.map(|(_, into)| into).filter(|into| *into > 0.0);
                });
            });
        })
        .detach();
    }

    /// While the place the list was scrolled to at the last start is being
    /// put back, before the list is laid out: ask for it again, since the
    /// list holds an offset to the end it has in each frame, and the first
    /// frames may lay it out lower or higher than it settles at, in a
    /// window larger than the window manager then makes it. A frame
    /// that drew it there does not end this: a later one may hold it
    /// again. What ends it is anything else moving the list (the wheel,
    /// the scroll bar, a row brought into view).
    fn keep_restoring(&self) {
        let Some(restoring) = self.restoring.get() else {
            return;
        };
        let now = crate::zoom::unpx(self.scroll.base_handle().offset().y);
        if (now - restoring.drawn).abs() > 0.5 {
            self.restoring.set(None);
        } else if (now - restoring.wanted).abs() > 0.5 {
            self.scroll
                .base_handle()
                .set_offset(point(px(0.), px(restoring.wanted)));
        }
    }

    /// The list has been laid out with the place being put back: where it
    /// was held to is noted, to tell it from a move by something else in
    /// the next frame.
    fn note_restoring(&self) {
        let Some(restoring) = self.restoring.get() else {
            return;
        };
        let drawn = crate::zoom::unpx(self.scroll.base_handle().offset().y);
        self.restoring.set(Some(Restoring { drawn, ..restoring }));
    }

    /// Name a new branch or tag in the label column of the row of `start`
    /// (a commit's id or a reference's name; HEAD when none): the row is
    /// selected and scrolled to, and a field takes the place of its labels,
    /// with `initial` in it. Enter makes it (see [`GraphEvent::Named`]),
    /// Escape or a click elsewhere gives up. False when the commit is not
    /// among the rows on show: then the name is asked for otherwise.
    pub fn name_new(
        &mut self,
        what: NameFor,
        start: Option<String>,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(data) = self.data.clone() else {
            return false;
        };
        let commit = match start.as_deref() {
            None | Some("HEAD") => data.head,
            Some(start) => start.parse::<Oid>().ok().or_else(|| {
                data.badges.iter().find_map(|(id, badges)| {
                    badges
                        .iter()
                        .any(|badge| {
                            badge.name == start || badge.full_name.as_deref() == Some(start)
                        })
                        .then_some(*id)
                })
            }),
        };
        let Some(row) = commit
            .and_then(|id| data.index.get(&id).copied())
            .map(|ix| ix.saturating_add(self.offset()))
        else {
            return false;
        };
        if self
            .shown()
            .is_some_and(|rows| rows.binary_search(&row).is_err())
        {
            return false;
        }
        self.select(Some(row), true, window, cx);
        let field = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(what.placeholder());
            state.set_value(initial.to_owned(), window, cx);
            state
        });
        let subscription = cx.subscribe_in(
            &field,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.finish_naming(window, cx),
                InputEvent::Blur => this.stop_naming(cx),
                _ => {}
            },
        );
        self.naming = Some(Naming {
            row,
            start,
            what,
            field: field.clone(),
            _subscription: subscription,
        });
        // After whatever asked for it (a menu) has given the focus back.
        cx.defer_in(window, move |_, window, cx| {
            field.update(cx, |state, cx| state.focus(window, cx));
        });
        cx.notify();
        true
    }

    /// Enter in the naming field: say what was named, and put the field
    /// away; an empty name makes nothing.
    fn finish_naming(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(naming) = self.naming.take() else {
            return;
        };
        let name = naming.field.read(cx).value().trim().to_owned();
        self.focus_handle.focus(window, cx);
        if !name.is_empty() {
            cx.emit(GraphEvent::Named {
                what: naming.what,
                start: naming.start,
                name,
            });
        }
        cx.notify();
    }

    /// Put the naming field away without making anything.
    fn stop_naming(&mut self, cx: &mut Context<Self>) {
        if self.naming.take().is_some() {
            cx.notify();
        }
    }

    /// Show the search bar and put the cursor in it.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        self.search.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Put the search bar away and forget what was searched for.
    fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = false;
        self.search
            .update(cx, |state, cx| state.set_value("", window, cx));
        self.search_text.clear();
        let narrowed = std::mem::take(&mut self.only_matches);
        self.recompute_matches(cx);
        // Back among all the rows: where the selected one is.
        if narrowed && let Some(ix) = self.selected {
            self.scroll_to(ix, ScrollStrategy::Center);
        }
        self.focus_handle.focus(window, cx);
    }

    /// The rows on show while only what a search found is shown: the
    /// matches, in row order. None while every row shows.
    fn shown(&self) -> Option<&[usize]> {
        (self.only_matches && !self.search_text.trim().is_empty())
            .then_some(self.matches.as_slice())
    }

    /// Bring a row into view, wherever it is in the list that is on show.
    fn scroll_to(&self, list_ix: usize, strategy: ScrollStrategy) {
        // Where the list was at the last start is not put back over it.
        self.restoring.set(None);
        let at = match self.shown() {
            Some(rows) => rows.binary_search(&list_ix).ok(),
            None => Some(list_ix),
        };
        if let Some(at) = at {
            self.scroll.scroll_to_item(at, strategy);
        }
    }

    /// What the list keeps free over its first row: room for the search
    /// bar while it is open (`SEARCH_INSET`).
    pub(crate) fn list_inset(&self) -> f32 {
        if self.search_open { SEARCH_INSET } else { 0.0 }
    }

    /// Give the list as many rows as are on show.
    fn resize_list(&mut self) {
        let rows = match self.shown() {
            Some(rows) => rows.len(),
            None => self
                .data
                .as_ref()
                .map_or(0, |data| data.commits.len())
                .saturating_add(self.offset()),
        };
        if self.item_sizes.len() != rows {
            self.item_sizes = crate::zoom::Sizes::new(vec![size(px(0.), px(ROW_HEIGHT)); rows]);
        }
    }

    /// Show only the commits the search found, or every row again. The
    /// selected row stays selected and in view; among the matches the rows
    /// are no neighbours, so they show without lanes.
    pub fn toggle_only_matches(&mut self, cx: &mut Context<Self>) {
        self.only_matches = !self.only_matches;
        self.resize_list();
        self.restoring.set(None);
        self.scroll.base_handle().set_offset(point(px(0.), px(0.)));
        if let Some(ix) = self.selected {
            self.scroll_to(ix, ScrollStrategy::Center);
        }
        cx.notify();
    }

    /// Go to the match before or after the current one.
    fn step_match(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.matches.len();
        if count == 0 {
            return;
        }
        self.match_pos = if forward {
            self.match_pos.saturating_add(1) % count
        } else {
            self.match_pos
                .checked_sub(1)
                .unwrap_or(count.saturating_sub(1))
        };
        if let Some(ix) = self.matches.get(self.match_pos).copied() {
            // From the search field the keys stay there, for the next step.
            let typing = self.search.focus_handle(cx).contains_focused(window, cx);
            self.select(Some(ix), true, window, cx);
            if typing {
                self.search.update(cx, |state, cx| state.focus(window, cx));
            }
        }
    }

    pub fn new(
        session: SessionHandle,
        bench: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(say!("graph-search-placeholder")));
        let subscription = cx.subscribe_in(
            &search,
            window,
            |this, state, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    this.search_text = state.read(cx).value().to_string();
                    this.recompute_matches(cx);
                }
                // Enter goes to the next of what was found, with Shift to
                // the one before.
                InputEvent::PressEnter { shift, .. } => this.step_match(!*shift, window, cx),
                _ => {}
            },
        );
        Self {
            session,
            data: None,
            details: HashMap::new(),
            pending: HashSet::new(),
            selected: None,
            hovered: None,
            unfold: Unfold::default(),
            labels_held: None,
            widen: widen::Widen::default(),
            ghost: None,
            menu_label: None,
            branch_rows: None,
            panel_hover: None,
            light: light::Light::default(),
            dividers: Vec::new(),
            scroll: VirtualListScrollHandle::new(),
            top_seen: None,
            top_saving: false,
            top_restored: false,
            restoring: Cell::new(None),
            heading_drawn: Cell::new(f32::NAN),
            heading_pane: Cell::new(f32::NAN),
            heading_asked: Cell::new(false),
            resize_grab: Rc::new(Cell::new(None)),
            item_sizes: crate::zoom::Sizes::default(),
            focus_handle: cx.focus_handle(),
            search,
            search_text: String::new(),
            search_asked: 0,
            search_pending: false,
            search_stopped: false,
            search_error: None,
            matches: Vec::new(),
            match_pos: 0,
            now: now(),
            frames_rendered: 0,
            logged: (false, false),
            wip: None,
            stopped: None,
            picked: BTreeSet::new(),
            anchor: None,
            search_open: false,
            only_matches: false,
            scroll_x: 0.0,
            current_branch: None,
            worktree_changes: HashMap::new(),
            removing: BTreeSet::new(),
            signatures: HashMap::new(),
            signed_asked: HashSet::new(),
            verify_queue: Vec::new(),
            verifying: false,
            drop_menu: None,
            naming: None,
            lightweight: HashSet::new(),
            summary_source: None,
            label_drag: false,
            stash_lanes: Rc::new(Vec::new()),
            want_wip: false,
            deselected: false,
            checks: Default::default(),
            pull_marks: Rc::default(),
            bench: std::env::var("BITBRAID_BENCH")
                .ok()
                .filter(|_| bench)
                .and_then(|v| v.parse::<u64>().ok())
                .map(|secs| Bench {
                    started: std::time::Instant::now(),
                    duration: std::time::Duration::from_secs(secs),
                    position: 0,
                    forward: true,
                    frames: 0,
                    second_start: std::time::Instant::now(),
                    second_frames: 0,
                    samples: Vec::new(),
                    running: false,
                    resize: std::env::var_os("BITBRAID_BENCH_RESIZE")
                        .map(|_| window.window_handle()),
                    border: None,
                    shift: 0.0,
                    sent: 0,
                    resizes: 0,
                    counted_from: None,
                }),
            _subscriptions: vec![subscription],
        }
    }

    /// Start the auto-scroll benchmark once the graph is loaded.
    fn start_bench(&mut self, cx: &mut Context<Self>) {
        let Some(bench) = self.bench.as_mut() else {
            return;
        };
        if bench.running {
            return;
        }
        bench.running = true;
        bench.started = std::time::Instant::now();
        bench.second_start = bench.started;
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(4))
                    .await;
                match this
                    .update(cx, |view, cx| view.bench_step(cx))
                    .unwrap_or(BenchStep::Done)
                {
                    BenchStep::Next => {}
                    // Sent as the system sends what the pointer does, so
                    // that the drag takes the way a hand's takes.
                    BenchStep::Send(window, event) => {
                        let _ = window.update(cx, |_, window, cx| window.dispatch_event(event, cx));
                    }
                    BenchStep::Done => break,
                }
            }
        })
        .detach();
    }

    /// One step of the measuring run: count the frames drawn, end the run
    /// when its time is up, and otherwise scroll or say where the pointer
    /// goes next.
    #[allow(clippy::print_stdout)] // the benchmark reports on stdout by design
    fn bench_step(&mut self, cx: &mut Context<Self>) -> BenchStep {
        let rows = self.data.as_ref().map(|d| d.commits.len()).unwrap_or(0);
        let rendered = self.frames_rendered;
        let (refs, graph) = self.widths(cx);
        let list = self.scroll.bounds();
        let Some(bench) = self.bench.as_mut() else {
            return BenchStep::Done;
        };
        let now = std::time::Instant::now();
        let span = now.duration_since(bench.second_start);
        if span.as_secs_f64() >= 1.0 {
            let frames_this_second = rendered.saturating_sub(bench.second_frames);
            bench.frames = rendered;
            let fps = frames_this_second as f64 / span.as_secs_f64();
            println!("fps {fps:.1}");
            bench.samples.push(fps);
            bench.second_start = now;
            bench.second_frames = rendered;
            if bench.counted_from.is_none() {
                bench.counted_from =
                    crate::processor_time::this_thread().map(|run| (run, rendered));
            }
        }
        if bench.started.elapsed() >= bench.duration {
            let samples: Vec<f64> = bench.samples.iter().skip(1).copied().collect();
            let min = samples.iter().copied().fold(f64::INFINITY, f64::min);
            let avg = if samples.is_empty() {
                0.0
            } else {
                samples.iter().sum::<f64>() / samples.len() as f64
            };
            // The most memory held, with the graph open and scrolled:
            // where the system tells.
            let memory = crate::memory::resident()
                .map(|held| {
                    format!(
                        " peak_rss_mib={} rss_mib={}", // not shown: xtask appbench reads it
                        held.peak_mib(),
                        held.now_mib()
                    )
                })
                .unwrap_or_default();
            // The main thread's time on a processor for each frame counted:
            // what is left of a frame's time at the rate drawn.
            let processor = bench
                .counted_from
                .zip(crate::processor_time::this_thread())
                .and_then(|((from, frames_then), run)| {
                    let frames = rendered.checked_sub(frames_then).filter(|&n| n > 0)?;
                    let spent = run.checked_sub(from)?;
                    Some(format!(
                        " main_thread_ms_per_frame={:.2}",
                        spent.as_secs_f64() * 1000.0 / frames as f64
                    ))
                })
                .unwrap_or_default();
            // What the resizing run did: the pointer's events sent, and
            // how often the column took a width from them.
            let resizing = if bench.resize.is_some() {
                format!(
                    " resize sent={} resizes={} graph_width={graph:.0}", // not shown: as above
                    bench.sent, bench.resizes
                )
            } else {
                String::new()
            };
            println!(
                "APP_BENCH rows={rows} seconds={} frames={} min_fps={min:.1} avg_fps={avg:.1}{memory}{processor}{resizing}",
                bench.duration.as_secs(),
                bench.frames
            );
            cx.quit();
            return BenchStep::Done;
        }
        if let Some(window) = bench.resize {
            // The graph column's border, in the middle of the heading,
            // which lies above the list and is as wide.
            if list.size.width <= px(0.) || graph <= 0.0 {
                return BenchStep::Next;
            }
            let (line, width) = *bench.border.get_or_insert((refs + graph, graph));
            let at = |shift: f32| {
                point(
                    list.left() + px(line + shift),
                    list.top() - px(HEADER_GAP + HEADER_HEIGHT / 2.0),
                )
            };
            // Onto the border, the button down, then to and fro, as far
            // left as the column can still give and as far right as the
            // reach.
            let event = match bench.sent {
                0 => PlatformInput::MouseMove(MouseMoveEvent {
                    position: at(0.0),
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                }),
                1 => PlatformInput::MouseDown(MouseDownEvent {
                    button: MouseButton::Left,
                    position: at(0.0),
                    modifiers: Modifiers::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                _ => {
                    let least = -(width - 60.0).clamp(0.0, BENCH_DRAG_REACH);
                    if bench.forward {
                        bench.shift += BENCH_DRAG_STEP;
                        if bench.shift >= BENCH_DRAG_REACH {
                            bench.forward = false;
                        }
                    } else {
                        bench.shift -= BENCH_DRAG_STEP;
                        if bench.shift <= least {
                            bench.forward = true;
                        }
                    }
                    PlatformInput::MouseMove(MouseMoveEvent {
                        position: at(bench.shift),
                        pressed_button: Some(MouseButton::Left),
                        modifiers: Modifiers::default(),
                    })
                }
            };
            bench.sent = bench.sent.saturating_add(1);
            return BenchStep::Send(window, event);
        }
        let step = 3usize;
        let last = rows.saturating_sub(1);
        if bench.forward {
            bench.position = bench.position.saturating_add(step).min(last);
            if bench.position >= last {
                bench.forward = false;
            }
        } else {
            bench.position = bench.position.saturating_sub(step);
            if bench.position == 0 {
                bench.forward = true;
            }
        }
        let position = bench.position;
        self.restoring.set(None);
        self.scroll.scroll_to_item(position, ScrollStrategy::Top);
        cx.notify();
        BenchStep::Next
    }

    /// Replace the graph, keeping the selection when the commit still exists.
    pub fn set_graph(
        &mut self,
        data: Arc<GraphData>,
        remembered: Option<Oid>,
        cx: &mut Context<Self>,
    ) {
        // The session hands the same graph out again when nothing it was
        // built from has changed.
        if self.data.as_ref().is_some_and(|d| Arc::ptr_eq(d, &data)) {
            return;
        }
        let previous = self.selected_commit().and_then(|ix| {
            self.data
                .as_ref()
                .and_then(|d| d.commits.get(ix))
                .map(|c| c.id)
        });
        let was_wip = self.offset() == 1 && self.selected == Some(0);
        let first_load = self.data.is_none();
        // The selection was taken away by a click and nothing was selected
        // since: it stays away.
        self.deselected = self.deselected && self.selected.is_none() && !first_load;
        // What was on show were the leading rows of this history.
        let grew = self
            .data
            .as_ref()
            .is_some_and(|d| d.walked.is_some() && d.generation == data.generation);
        // The commits of a selection of several, to find them again.
        let picked_ids = self.picked_ids();
        self.now = now();
        // Leaving the row of uncommitted work (after a commit, say) lands on HEAD.
        let wanted = if was_wip {
            data.head
        } else {
            previous.or(remembered).or(data.head)
        };
        let offset = usize::from(data.wip.is_some());
        let asked = offset == 1 && std::mem::take(&mut self.want_wip);
        // The commit to select may be further down a history of which only
        // the leading rows are here: then nothing is selected, and nothing
        // is said about it, until the rows that hold it arrive.
        let awaited = data.walked.is_some()
            && wanted
                .filter(|_| previous.or(remembered).is_some())
                .is_some_and(|id| !data.index.contains_key(&id));
        self.selected = if offset == 1 && (was_wip || asked) {
            Some(0)
        } else if awaited || self.deselected {
            None
        } else {
            wanted
                .and_then(|id| data.index.get(&id).copied())
                .map(|ix| ix + offset)
                .or(if data.commits.is_empty() {
                    None
                } else {
                    Some(offset)
                })
        };
        let awaited = awaited && self.selected.is_none();
        self.picked = picked_ids
            .iter()
            .filter_map(|id| data.index.get(id).map(|ix| ix.saturating_add(offset)))
            .collect();
        if self.picked.len() < 2 || self.selected.is_none_or(|at| !self.picked.contains(&at)) {
            self.picked.clear();
        }
        self.anchor = self.selected;
        self.stash_lanes = Rc::new(stash_lanes(&data));
        self.data = Some(data);
        self.ghost = None;
        self.unfold = Unfold::default();
        self.labels_held = None;
        self.widen.reset();
        // The row the field stood on may be another commit's now.
        self.naming = None;
        self.menu_label = None;
        self.branch_rows = None;
        self.light.reset();
        self.rebuild_sizes();
        self.details
            .retain(|id, _| self.data.as_ref().is_some_and(|d| d.index.contains_key(id)));
        self.pending.clear();
        // A reload that keeps the selection leaves the scroll position alone.
        let now_wip = self.offset() == 1 && self.selected == Some(0);
        let current = self.selected_commit().and_then(|ix| {
            self.data
                .as_ref()
                .and_then(|d| d.commits.get(ix))
                .map(|c| c.id)
        });
        let kept = !first_load && was_wip == now_wip && previous == current;
        // The first time, the list comes back where it was scrolled to, if
        // the commit that was at its top is still in the graph. In a large
        // history that commit may come with a later piece: it is looked
        // for until it is found or the whole history is here, unless the
        // list was scrolled meanwhile.
        let untouched = crate::zoom::unpx(self.scroll.base_handle().offset().y) == 0.0;
        let look = !self.top_restored && (first_load || (grew && untouched));
        let remembered_top = look
            .then(|| {
                let state = cx.global::<Prefs>().repo_state(&self.session.path);
                Some((state.top?, state.top_into))
            })
            .flatten();
        let top = remembered_top.and_then(|(id, into)| {
            let ix = id
                .parse::<Oid>()
                .ok()
                .and_then(|id| self.data.as_ref()?.index.get(&id).copied())?;
            let into = into
                .filter(|into| into.is_finite())
                .unwrap_or(0.0)
                .clamp(0.0, ROW_HEIGHT);
            Some((ix.saturating_add(self.offset()), into))
        });
        let whole = self.data.as_ref().is_some_and(|d| d.walked.is_none());
        if top.is_some() || whole || !look {
            self.top_restored = true;
        }
        if let Some((top, into)) = top.filter(|_| self.shown().is_none()) {
            // The offset itself: asking the list to scroll to a row only
            // brings the row into view, at whichever edge is nearer. It is
            // asked for again until the list is drawn there.
            let wanted = -(top as f32 * ROW_HEIGHT + into);
            self.scroll
                .base_handle()
                .set_offset(point(px(0.), px(wanted)));
            self.restoring.set(Some(Restoring {
                wanted,
                drawn: wanted,
            }));
        }
        if !kept && let Some(ix) = self.selected {
            if top.is_none() {
                self.scroll_to(ix, ScrollStrategy::Center);
            }
            // Ask for the commits around the row the list is about to show,
            // rather than wait for the first frame to find them missing.
            let centre = self.commit_ix(ix).unwrap_or(0);
            let around: Vec<Oid> = self
                .data
                .as_ref()
                .map(|d| {
                    d.commits
                        .iter()
                        .skip(centre.saturating_sub(48))
                        .take(96)
                        .map(|c| c.id)
                        .filter(|id| !self.details.contains_key(id))
                        .collect()
                })
                .unwrap_or_default();
            self.request_details(around, cx);
        }
        // Nothing is said while the commit to select is awaited: what is
        // remembered as selected stays remembered.
        if !awaited {
            self.emit_selection(cx);
        }
        self.recompute_matches(cx);
        self.start_bench(cx);
        cx.notify();
    }

    /// Offset of the first commit in the list: 1 when the graph starts with
    /// the row of uncommitted work.
    fn offset(&self) -> usize {
        usize::from(self.data.as_ref().is_some_and(|d| d.wip.is_some()))
    }

    /// Map a list index to a commit index.
    fn commit_ix(&self, list_ix: usize) -> Option<usize> {
        list_ix.checked_sub(self.offset())
    }

    fn selected_commit(&self) -> Option<usize> {
        self.selected.and_then(|ix| self.commit_ix(ix))
    }

    fn rebuild_sizes(&mut self) {
        let offset = self.offset();
        let commits = self
            .data
            .as_ref()
            .map(|d| d.commits.as_slice())
            .unwrap_or(&[]);
        let now = self.now;
        // A divider says that what follows is older. That holds in date
        // order only: in topological order the dates go back and forth.
        let by_date = self
            .data
            .as_ref()
            .is_some_and(|d| d.order == bitbraid_git::Order::Date);
        let mut dividers: Vec<Option<String>> =
            Vec::with_capacity(if by_date { commits.len() } else { 0 });
        let mut previous: Option<(u8, i64)> = None;
        for commit in commits.iter().filter(|_| by_date) {
            let bucket = commit.seconds.map(|s| time_bucket(s, now));
            let label = match (bucket, previous) {
                (Some(b), Some(p)) if b == p => None,
                (Some(b), _) => Some(bucket_label(b)),
                (None, _) => None,
            };
            if bucket.is_some() {
                previous = bucket;
            }
            dividers.push(label);
        }
        let sizes = vec![size(px(0.), px(ROW_HEIGHT)); commits.len() + offset];
        self.dividers = dividers;
        self.item_sizes = crate::zoom::Sizes::new(sizes);
    }

    /// A column border is being dragged: the column follows the pointer.
    fn resize_column(
        &mut self,
        which: Resize,
        pointer: Pixels,
        header: Bounds<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let x = crate::zoom::unpx(pointer - header.left());
        let room = crate::zoom::unpx(header.size.width);
        let (refs, graph) = self.widths(cx);
        let columns = &mut cx.global_mut::<Prefs>().preferences.columns;
        // Where the dragged column begins: after those before it, as wide
        // as they are drawn.
        let before = match which {
            Resize::Refs => 0.0,
            Resize::Graph => refs,
            Resize::Column(column) => {
                let shown = shown_columns(columns);
                let widths = fit_columns(&shown, room - refs - graph);
                let at = shown
                    .iter()
                    .position(|(shown, _)| *shown == column)
                    .unwrap_or(0);
                refs + graph + widths.iter().take(at).sum::<f32>()
            }
        };
        // The line keeps the distance from the pointer it had where the
        // pointer took it, within the strip it was taken by.
        let grabbed = self.resize_grab.get().map_or(0.0, |(down, line)| {
            let from_line = crate::zoom::unpx(down - header.left()) - line;
            if from_line.is_finite() {
                from_line.clamp(-GRAB_LEFT, GRAB_WIDTH - GRAB_LEFT)
            } else {
                0.0
            }
        });
        let new = resized(which, x - grabbed - before);
        if let Some(bench) = self.bench.as_mut() {
            bench.resizes = bench.resizes.saturating_add(1);
        }
        match which {
            Resize::Refs => columns.refs_width = new,
            Resize::Graph => {
                columns.graph_width = new;
                columns.graph_auto = false;
            }
            Resize::Column(Column::Summary) => columns.summary_width = new,
            Resize::Column(Column::Author) => columns.author_width = new,
            Resize::Column(Column::Date) => columns.date_width = Some(new),
            Resize::Column(Column::Id) => columns.id_width = new,
        }
        cx.notify();
    }

    /// How wide the branch and tag column and the graph column are drawn:
    /// nothing for one that is put away; the graph column as wide as its
    /// lanes need until its border is dragged.
    fn widths(&self, cx: &App) -> (f32, f32) {
        let columns = &cx.global::<Prefs>().preferences.columns;
        let refs = if columns.show_refs {
            columns.refs_width
        } else {
            0.0
        };
        let graph = if !columns.show_graph {
            0.0
        } else if columns.graph_auto {
            auto_graph_width(self.data.as_ref().map_or(1, |data| data.max_width))
        } else {
            columns.graph_width
        };
        (refs, graph)
    }

    /// How wide the columns after the graph are drawn, in their order:
    /// what [`fit_columns`] makes of the room the pane has.
    fn column_widths(&self, cx: &App) -> Vec<(Column, f32)> {
        let (refs, graph) = self.widths(cx);
        let columns = &cx.global::<Prefs>().preferences.columns;
        let shown = shown_columns(columns);
        let pane = crate::zoom::unpx(self.scroll.bounds().size.width);
        let widths = fit_columns(&shown, pane - refs - graph);
        shown
            .iter()
            .map(|(column, _)| *column)
            .zip(widths)
            .collect()
    }

    /// The strip over a column's border, `line` points from the heading's
    /// left edge, that is dragged to resize the column: it lies over the
    /// headings on both sides of the line, after them, so that it takes
    /// the pointer there before they do, and within the heading, which is
    /// `room` wide when that is known. Under the pointer the line it
    /// stands for is drawn in the accent colour. Where it was taken, from
    /// the line, is kept in `grab`: the line then stays that far from the
    /// pointer instead of jumping to it.
    fn resize_handle(
        which: Resize,
        line: f32,
        room: f32,
        grab: &Rc<Cell<Option<(Pixels, f32)>>>,
        theme: &gpui_kit::component::Theme,
    ) -> AnyElement {
        let hover = theme.primary.opacity(0.6);
        let number = match which {
            Resize::Refs => 0usize,
            Resize::Graph => 1,
            Resize::Column(column) => 2 + column as usize,
        };
        let left = (line - GRAB_LEFT).max(0.0);
        let right = if room.is_finite() && room > 0.0 {
            (line - GRAB_LEFT + GRAB_WIDTH).min(room)
        } else {
            line - GRAB_LEFT + GRAB_WIDTH
        };
        let grab = grab.clone();
        div()
            .id(("resize", number))
            .group("column-resize")
            .absolute()
            .top_0()
            .h_full()
            .left(px(left))
            .w(px((right - left).max(0.0)))
            .cursor_col_resize()
            .child(
                div()
                    .absolute()
                    .top_0()
                    .h_full()
                    .left(px(line - left - 1.0))
                    .w(px(3.))
                    .group_hover("column-resize", move |style| style.bg(hover)),
            )
            .on_mouse_down(MouseButton::Left, move |event, _, _| {
                grab.set(Some((event.position.x, line)));
            })
            .on_drag(which, |_, _, _, cx| cx.new(|_| NoChip))
            .test_support()
            .into_any_element()
    }

    /// Something was dropped on a row, of the graph or of the references
    /// panel: offer what that can mean.
    pub(crate) fn dropped(
        &mut self,
        dragged: &Dragged,
        target: &DropTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // With what the graph knows of the two: whether the one could simply
        // move forward to the other, and whether that is a remote's branch.
        let current = self.current_branch.as_deref();
        let choices = match self.data.as_deref() {
            Some(graph) => {
                let mut around = rewrite::around(graph, dragged, target);
                around.pull_requests = self.checks.linked();
                rewrite::drop_choices_around(dragged, target, current, &around)
            }
            None => drop_choices(dragged, target, current),
        };
        self.drop_menu = Some(DropMenu {
            position: window.mouse_position(),
            choices,
        });
        cx.notify();
    }

    fn render_drop_menu(&self, menu: &DropMenu, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let entries: Vec<AnyElement> = if menu.choices.is_empty() {
            vec![
                div()
                    .px_2()
                    .py_1()
                    .text_color(theme.muted_foreground)
                    .child(say!("graph-drop-nothing"))
                    .into_any_element(),
            ]
        } else {
            menu.choices
                .iter()
                .enumerate()
                .map(|(ix, choice)| {
                    let picked = choice.clone();
                    // Said in the entry, since it is not what was asked for.
                    let label = match &choice.checkout {
                        Some(first) => say!(
                            "graph-drop-checks-out-first",
                            choice = choice.label.as_str(),
                            first = first.as_str()
                        ),
                        None => choice.label.clone(),
                    };
                    div()
                        .id(("drop-choice", ix))
                        .px_2()
                        .py_1()
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.accent))
                        .child(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.drop_menu = None;
                            cx.emit(GraphEvent::Dropped(picked.clone()));
                            cx.notify();
                        }))
                        .test_support()
                        .into_any_element()
                })
                .collect()
        };
        deferred(
            anchored().position(menu.position).child(
                v_flex()
                    .id("drop-menu")
                    .occlude()
                    .p_1()
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .text_color(theme.popover_foreground)
                    .text_sm()
                    .shadow_md()
                    .children(entries)
                    // A click anywhere else puts it away.
                    .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                        this.drop_menu = None;
                        cx.notify();
                    })),
            ),
        )
        .with_priority(1)
    }

    /// Select the row of uncommitted work, now or as soon as the graph has
    /// one (it arrives a moment after the status that calls for it).
    pub fn select_wip(&mut self, cx: &mut Context<Self>) {
        if self.offset() == 1 {
            self.selected = Some(0);
            self.emit_selection(cx);
            cx.notify();
        } else {
            self.want_wip = true;
        }
    }

    /// What is uncommitted in each of the other worktrees, by its path:
    /// for the marks on what they have checked out and for their rows.
    pub fn set_worktree_changes(
        &mut self,
        changes: HashMap<String, WipCounts>,
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

    /// The commit form's summary, for the row of uncommitted work to show.
    pub fn set_summary_source(&mut self, field: Entity<InputState>) {
        self.summary_source = Some(field);
    }

    /// The tags that have no message of their own: their labels offer to
    /// give them one.
    pub fn set_lightweight_tags(&mut self, tags: HashSet<String>) {
        self.lightweight = tags;
    }

    /// Tell the view which branch is checked out (none when detached).
    pub fn set_current_branch(&mut self, branch: Option<String>, cx: &mut Context<Self>) {
        if self.current_branch != branch {
            self.current_branch = branch;
            cx.notify();
        }
    }

    /// Record the staged and unstaged counts shown on the row of uncommitted
    /// work. The row appears and disappears with the graph data.
    pub fn set_wip(&mut self, counts: Option<WipCounts>, cx: &mut Context<Self>) {
        self.wip = counts.filter(|counts| counts.total() > 0);
        cx.notify();
    }

    /// An operation stopped part way, or is over: the row of uncommitted
    /// work says what it waits for (the sentence, and whether it cannot go
    /// on as things are).
    pub fn set_stopped(&mut self, stopped: Option<(String, bool)>, cx: &mut Context<Self>) {
        if self.stopped != stopped {
            self.stopped = stopped;
            cx.notify();
        }
    }

    /// The commits of a selection of several, newest first; empty when at
    /// most one row is selected.
    fn picked_ids(&self) -> Vec<Oid> {
        let Some(data) = &self.data else {
            return Vec::new();
        };
        self.picked
            .iter()
            .filter_map(|row| self.commit_ix(*row))
            .filter_map(|ix| data.commits.get(ix))
            .map(|commit| commit.id)
            .collect()
    }

    /// Whether a row is selected, alone or among several.
    fn is_selected(&self, list_ix: usize) -> bool {
        self.selected == Some(list_ix) || self.picked.contains(&list_ix)
    }

    /// A row that can be one of several: a commit, not the uncommitted
    /// work and not a stash.
    fn can_join(&self, list_ix: usize) -> bool {
        let Some(data) = &self.data else { return false };
        self.commit_ix(list_ix)
            .and_then(|ix| data.commits.get(ix))
            .is_some_and(|commit| {
                !data.worktree_rows.contains_key(&commit.id)
                    && !data.badges.get(&commit.id).is_some_and(|badges| {
                        badges.iter().any(|badge| badge.kind == BadgeKind::Stash)
                    })
            })
    }

    /// A row was clicked with these keys held.
    fn click(
        &mut self,
        list_ix: usize,
        modifiers: Modifiers,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let several = modifiers.secondary() || modifiers.shift;
        // A plain click on the one row that is selected takes the selection
        // away: nothing is selected then, until a row is clicked.
        if !several && self.picked.is_empty() && self.selected == Some(list_ix) {
            self.select(None, false, window, cx);
            self.deselected = true;
            return;
        }
        // Uncommitted work and stashes are selected alone, and nothing is
        // added to them.
        let joins = self.can_join(list_ix)
            && self
                .selected
                .is_none_or(|at| self.can_join(at) || self.picked.is_empty() && !several);
        if !several || !joins || self.selected.is_some_and(|at| !self.can_join(at)) {
            self.select(Some(list_ix), false, window, cx);
            return;
        }
        let cursor = click_rows(
            &mut self.picked,
            &mut self.anchor,
            self.selected,
            list_ix,
            modifiers.secondary(),
            modifiers.shift,
        );
        // A range may have swept over rows that cannot join, or, while
        // only what a search found is shown, over rows that are not shown.
        let picked: Vec<usize> = self.picked.iter().copied().collect();
        for row in picked {
            let hidden = self
                .shown()
                .is_some_and(|rows| rows.binary_search(&row).is_err());
            if hidden || !self.can_join(row) {
                self.picked.remove(&row);
            }
        }
        if self.picked.len() < 2 {
            self.picked.clear();
        }
        self.selected = cursor.or(self.selected);
        self.focus_handle.focus(window, cx);
        self.announce_selection(true, cx);
        cx.notify();
    }

    /// Grow or shrink the selection by one row from the keyboard.
    fn extend_selection(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(data) = &self.data else { return };
        let Some(at) = self.selected else { return };
        let last = data
            .commits
            .len()
            .saturating_add(self.offset())
            .saturating_sub(1);
        let to = if down {
            at.saturating_add(1).min(last)
        } else {
            at.saturating_sub(1)
        };
        if to == at || !self.can_join(to) || !self.can_join(at) {
            return;
        }
        if self.anchor.is_none() {
            self.anchor = Some(at);
        }
        let shift = Modifiers {
            shift: true,
            ..Modifiers::default()
        };
        self.click(to, shift, window, cx);
        self.scroll_to(to, ScrollStrategy::Center);
    }

    /// Back to one selected row: the one the cursor is on.
    fn clear_picked(&mut self, cx: &mut Context<Self>) {
        if !self.picked.is_empty() {
            self.picked.clear();
            self.announce_selection(true, cx);
            cx.notify();
        }
    }

    /// Say again what is selected, for whoever shows something about it:
    /// after something else borrowed the place it is shown in.
    pub fn say_selection(&mut self, cx: &mut Context<Self>) {
        self.emit_selection(cx);
    }

    /// Say what is selected, which came about by itself: with a graph that
    /// was loaded, or a search that found it.
    fn emit_selection(&mut self, cx: &mut Context<Self>) {
        self.announce_selection(false, cx);
    }

    /// Say what is selected, and whether the user selected it.
    fn announce_selection(&mut self, asked: bool, cx: &mut Context<Self>) {
        if self.offset() == 1 && self.selected == Some(0) {
            cx.emit(GraphEvent::SelectedWip);
            return;
        }
        if self.picked.len() >= 2
            && let Some(data) = &self.data
        {
            let with_parents: Vec<(Oid, Vec<Oid>)> = self
                .picked
                .iter()
                .filter_map(|row| self.commit_ix(*row))
                .filter_map(|ix| data.commits.get(ix))
                .map(|commit| (commit.id, commit.parents.clone()))
                .collect();
            cx.emit(GraphEvent::SelectedMany {
                ids: with_parents.iter().map(|(id, _)| *id).collect(),
                comparison: comparison(&with_parents),
                asked,
            });
            return;
        }
        let id = self.selected_commit().and_then(|ix| {
            self.data
                .as_ref()
                .and_then(|d| d.commits.get(ix))
                .map(|c| c.id)
        });
        // The row of another worktree is no commit to show.
        if let Some(path) = id.and_then(|id| self.worktree_of(id)) {
            cx.emit(GraphEvent::SelectedWorktree(path));
            return;
        }
        cx.emit(GraphEvent::Selected { id, asked });
    }

    /// The path of the worktree whose uncommitted work the row with this
    /// id stands for, when it is such a row.
    fn worktree_of(&self, id: Oid) -> Option<String> {
        let data = self.data.as_ref()?;
        let at = data.worktree_rows.get(&id)?;
        data.worktrees.get(*at).map(|mark| mark.path.clone())
    }

    /// Select and scroll to a commit; false when it is not in the graph.
    pub fn reveal(&mut self, id: Oid, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(ix) = self.data.as_ref().and_then(|d| d.index.get(&id).copied()) else {
            return false;
        };
        let ix = ix + self.offset();
        self.select(Some(ix), true, window, cx);
        true
    }

    fn select(
        &mut self,
        ix: Option<usize>,
        scroll: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected = ix;
        self.deselected = false;
        self.picked.clear();
        self.anchor = ix;
        if scroll && let Some(ix) = ix {
            self.scroll_to(ix, ScrollStrategy::Center);
        }
        self.focus_handle.focus(window, cx);
        self.announce_selection(true, cx);
        cx.notify();
    }

    fn move_selection(&mut self, delta: isize, window: &mut Window, cx: &mut Context<Self>) {
        // Among what a search found, a step is to the next of those.
        if let Some(rows) = self.shown() {
            if let Some(next) = step_among(rows, self.selected, delta) {
                self.select(Some(next), true, window, cx);
            }
            return;
        }
        let Some(data) = &self.data else { return };
        if data.commits.is_empty() {
            return;
        }
        let current = self.selected.unwrap_or(0) as isize;
        let last = (data.commits.len() + self.offset()).saturating_sub(1) as isize;
        let next = (current + delta).clamp(0, last) as usize;
        self.select(Some(next), true, window, cx);
    }

    fn request_details(&mut self, ids: Vec<Oid>, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        for id in &ids {
            self.pending.insert(*id);
        }
        self.session
            .call(Request::Details(ids), cx, |this, reply, cx| {
                if let Reply::Details(details) = reply {
                    this.take_details(details, cx);
                }
            });
    }

    /// Details of commits for the rows: asked for here, or read ahead of
    /// the window for the rows the graph shows first when it opens
    /// (`Request::FirstDetails`), which may come before the graph itself.
    pub fn take_details(&mut self, details: Vec<CommitInfo>, cx: &mut Context<Self>) {
        if details.is_empty() {
            return;
        }
        if !std::mem::replace(&mut self.logged.1, true) {
            crate::session::milestone("first commit details");
        }
        let ids: Vec<Oid> = details.iter().map(|commit| commit.id).collect();
        for commit in details {
            self.pending.remove(&commit.id);
            self.details.insert(commit.id, commit);
        }
        self.request_signed(ids, cx);
        cx.notify();
    }

    /// Ask which of these commits carry a signature: read from the commit
    /// itself, so it costs next to nothing. The signed ones are then
    /// checked, a batch at a time.
    fn request_signed(&mut self, ids: Vec<Oid>, cx: &mut Context<Self>) {
        let ids: Vec<Oid> = ids
            .into_iter()
            .filter(|id| self.signed_asked.insert(*id))
            .collect();
        if ids.is_empty() {
            return;
        }
        let request = Request::Command {
            id: "sign.signed".to_owned(),
            params: json!({ "ids": ids }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            let Reply::Command(value) = reply else { return };
            for answer in value.as_array().into_iter().flatten() {
                let id = answer
                    .get("id")
                    .and_then(|id| id.as_str())
                    .and_then(|id| id.parse::<Oid>().ok());
                let signed = answer.get("signed").and_then(|flag| flag.as_bool()) == Some(true);
                if let (Some(id), true) = (id, signed) {
                    this.signatures.insert(
                        id,
                        Signed {
                            format: answer
                                .get("format")
                                .and_then(|format| format.as_str())
                                .map(str::to_owned),
                            verification: None,
                        },
                    );
                    this.verify_queue.push(id);
                }
            }
            this.verify_some(cx);
            cx.notify();
        });
    }

    /// Have git check the signatures that wait, the ones asked for last
    /// first: those are the rows in view.
    fn verify_some(&mut self, cx: &mut Context<Self>) {
        if self.verifying || self.verify_queue.is_empty() {
            return;
        }
        let from = self.verify_queue.len().saturating_sub(VERIFY_AT_ONCE);
        let ids: Vec<Oid> = self.verify_queue.split_off(from);
        self.verifying = true;
        let request = Request::Command {
            id: "sign.verify".to_owned(),
            params: json!({ "ids": ids }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            this.verifying = false;
            if let Reply::Command(value) = reply {
                for answer in value.as_array().into_iter().flatten() {
                    let id = answer
                        .get("id")
                        .and_then(|id| id.as_str())
                        .and_then(|id| id.parse::<Oid>().ok());
                    if let Some(signed) = id.and_then(|id| this.signatures.get_mut(&id)) {
                        signed.verification = Verification::from_json(answer);
                    }
                }
            }
            this.verify_some(cx);
            cx.notify();
        });
    }

    /// Who may sign with an SSH key has changed: what was made of the
    /// signatures made with one no longer holds, and git is asked again,
    /// a batch at a time as before. Signatures of other kinds stay as
    /// they were judged; the allowed signers say nothing about them.
    pub fn check_ssh_signatures_again(&mut self, cx: &mut Context<Self>) {
        let mut again: Vec<Oid> = self
            .signatures
            .iter_mut()
            .filter(|(_, signed)| signed.format.as_deref() == Some("ssh"))
            .map(|(id, signed)| {
                signed.verification = None;
                *id
            })
            .collect();
        // One that still waits is not asked about twice.
        again.retain(|id| !self.verify_queue.contains(id));
        if again.is_empty() {
            return;
        }
        self.verify_queue.extend(again);
        self.verify_some(cx);
        cx.notify();
    }

    fn set_hover(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        if self.hovered == ix {
            return;
        }
        self.hovered = ix;
        cx.notify();
    }

    /// The pointer came onto the labels of the row `list_ix`, which shows
    /// the commit `ix`, onto the column of them or onto one of them widened,
    /// or left; or the menu of one of them was opened from the column, or
    /// dismissed (`over`): the labels are unfolded while any holds, and the
    /// branch they name is lit.
    fn set_unfold(
        &mut self,
        list_ix: usize,
        ix: usize,
        over: Over,
        on: bool,
        cx: &mut Context<Self>,
    ) {
        if self.unfold.row != Some(list_ix) {
            if !on {
                return;
            }
            self.unfold = Unfold {
                row: Some(list_ix),
                ..Unfold::default()
            };
        }
        match over {
            Over::Labels => self.unfold.on_labels = on,
            Over::Column => self.unfold.on_column = on,
            Over::Widened => self.unfold.on_widened = on,
            Over::Menu => self.unfold.menu_open = on,
        }
        let kept = self.unfold.on_labels
            || self.unfold.on_column
            || self.unfold.on_widened
            || self.unfold.menu_open;
        if !kept {
            self.unfold = Unfold::default();
        }
        if self.light.point_at_labels(kept.then_some(ix)) {
            self.ask_light(cx);
        }
        cx.notify();
    }

    /// The pointer came onto the label `key` drawn at `place` (in its row
    /// or in the unfolded column), or left it: the label is widened to its
    /// whole name once the pointer has rested on it for [`light::DELAY`],
    /// when its name is cut.
    fn point_at_label(
        &mut self,
        key: widen::Key,
        place: widen::Place,
        on: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        let then = self.widen.point(key, place, on);
        self.widen_then(then, window, cx);
    }

    /// The pointer came onto the widened label of the row `list_ix`, which
    /// shows the commit `ix`, or left it: it keeps the label widened, the
    /// row's labels unfolded and their branch lit, as the label does.
    fn point_at_widened(
        &mut self,
        list_ix: usize,
        ix: usize,
        on: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) {
        self.set_unfold(list_ix, ix, Over::Widened, on, cx);
        let then = self.widen.point_at_wide(on);
        self.widen_then(then, window, cx);
    }

    /// The menu of a row's labels was opened from the column they unfold
    /// into (`Some`: the row of the list and its commit), or dismissed: while
    /// it is open the column stays unfolded and a label widened there stays
    /// widened, the menu lying over them; dismissed, they follow the pointer
    /// again.
    fn hold_labels(&mut self, held: Option<(usize, usize)>, cx: &mut Context<Self>) {
        if let Some((list_ix, ix)) = held {
            self.labels_held = held;
            self.set_unfold(list_ix, ix, Over::Menu, true, cx);
            return;
        }
        if let Some((list_ix, ix)) = self.labels_held.take() {
            self.set_unfold(list_ix, ix, Over::Menu, false, cx);
            if self.widen.settle() {
                cx.notify();
            }
        }
    }

    /// Before a label of the column hears of the secondary button: no
    /// label is pointed at, so that the menu is the commit's where the
    /// button went down on none of them, as on the row.
    fn no_label(
        cx: &mut Context<Self>,
    ) -> impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static {
        cx.listener(|this, event: &MouseDownEvent, _, _| {
            if event.button == MouseButton::Right {
                this.menu_label = None;
            }
        })
    }

    /// The row's menu as the column of its unfolded labels opens it (and a
    /// label widened there): the menu of the label the secondary button went
    /// down on, as the row's own label opens it. It holds the column until
    /// it is dismissed.
    fn column_menu(
        view: WeakEntity<Self>,
        kept: KeptMenu,
        list_ix: usize,
        ix: usize,
    ) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
        move |popup, window, cx| {
            let _ = view.update(cx, |this, cx| this.hold_labels(Some((list_ix, ix)), cx));
            let release = view.clone();
            let menu = cx.entity();
            cx.subscribe(&menu, move |_, _, _: &DismissEvent, cx| {
                let _ = release.update(cx, |this, cx| this.hold_labels(None, cx));
            })
            .detach();
            let row_menu = kept.try_borrow().ok().and_then(|slot| slot.clone());
            match row_menu {
                Some(row_menu) => row_menu(popup, window, cx),
                None => popup,
            }
        }
    }

    /// What a change of where the pointer is among the labels asks.
    fn widen_then(&mut self, then: widen::Then, window: &Window, cx: &mut Context<Self>) {
        match then {
            widen::Then::Nothing => {}
            widen::Then::Wait(turn) => {
                cx.notify();
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(light::DELAY).await;
                    let _ = this.update(cx, |this, cx| {
                        if this.widen.due(turn) {
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            // After what the pointer came onto has said so too.
            widen::Then::Settle => cx.defer_in(window, |this, _, cx| {
                // Not while the menu of the column's labels is open over
                // them: it lets them go when it is dismissed.
                if this.labels_held.is_none() && this.widen.settle() {
                    cx.notify();
                }
            }),
        }
    }

    /// What the pointer asks to be lit changed: it is lit once nothing
    /// changed for [`light::DELAY`]. The labels it is on ask first, then
    /// the branch it is on in the references panel; where it is on
    /// neither, the light goes out.
    fn ask_light(&mut self, cx: &mut Context<Self>) {
        let turn = self.light.ask();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(light::DELAY).await;
            let _ = this.update(cx, |this, cx| {
                if !this.light.is_turn(turn) {
                    return;
                }
                let row = this.light.on_labels().or_else(|| {
                    let id = this.panel_hover?;
                    this.data.as_ref()?.index.get(&id).copied()
                });
                this.set_highlight(row, cx);
            });
        })
        .detach();
    }

    /// Light the branch whose labels the pointer rests on: the commit at
    /// `row` and the commits its first parents lead to. The other rows
    /// begin to step back.
    fn set_highlight(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        if self.light.lit().map(|(row, _)| *row) == row {
            return;
        }
        let stash_lanes = self.stash_lanes.clone();
        let lit = row.and_then(|row| {
            let data = self.data.as_ref()?;
            // A commit is another branch's too where a branch's label is
            // on it, or where a line of history begins at it (a lane ends
            // in its node; a stash's does not count).
            let stops = |ix: usize| {
                let branch = data
                    .commits
                    .get(ix)
                    .and_then(|commit| data.badges.get(&commit.id))
                    .is_some_and(|badges| {
                        badges.iter().any(|badge| {
                            matches!(
                                badge.kind,
                                BadgeKind::Head | BadgeKind::Local | BadgeKind::Remote
                            )
                        })
                    });
                let stashed = stash_slots_at(&stash_lanes, ix);
                let forked = data.rows.get(ix).is_some_and(|layout| {
                    layout
                        .merges_in
                        .iter()
                        .any(|lane| !stashed.contains(&lane.slot))
                });
                branch || forked
            };
            let line = first_parent_line(&data.commits, &data.index, stops, row);
            Some((row, Rc::new(line)))
        });
        let now = cx.background_executor().now();
        self.light.light(lit, now);
        cx.notify();
    }

    /// The pointer came onto a branch in the references panel, or left it:
    /// the graph lights that branch's history as it does for a label of its
    /// own, once the pointer has rested there as long as on a label, so
    /// that passing over a list of branches walks no history; the branch
    /// that is left only takes back what it asked for.
    pub fn highlight_from_panel(&mut self, id: Oid, on: bool, cx: &mut Context<Self>) {
        if on {
            if self.panel_hover != Some(id) {
                self.panel_hover = Some(id);
                self.ask_light(cx);
            }
        } else if self.panel_hover == Some(id) {
            self.panel_hover = None;
            self.ask_light(cx);
        }
    }

    /// How much is left of what the row of the commit `ix` says while the
    /// run of another branch is lit: its opacity, none while it has not
    /// stepped back at all; on its way to [`FADED`] while the light comes
    /// or goes.
    fn shade(&self, ix: usize) -> Option<f32> {
        let back = self.light.back(ix);
        (back > 0.0).then_some(1.0 - (1.0 - FADED) * back)
    }

    /// Stop the search the engine is still running: a content search over a
    /// long history takes as long as git takes for it. What it found so far
    /// is not kept.
    fn stop_search(&mut self, cx: &mut Context<Self>) {
        if !self.search_pending {
            return;
        }
        drop(
            self.session
                .send(Request::CancelReads("search.commits".to_owned())),
        );
        // Its answer, which says that it was stopped, is not taken.
        self.search_asked = self.search_asked.wrapping_add(1);
        self.search_pending = false;
        self.search_stopped = true;
        cx.notify();
    }

    fn recompute_matches(&mut self, cx: &mut Context<Self>) {
        // What is still being searched was asked for another text.
        if self.search_pending {
            drop(
                self.session
                    .send(Request::CancelReads("search.commits".to_owned())),
            );
        }
        self.matches.clear();
        self.match_pos = 0;
        self.search_error = None;
        self.search_pending = false;
        self.search_stopped = false;
        // Whatever is still out was asked for another text.
        self.search_asked = self.search_asked.wrapping_add(1);
        // Until an answer is in there is nothing to show of a query.
        self.resize_list();
        let needle = self.search_text.trim().to_lowercase();
        if needle.is_empty() {
            self.resize_list();
            cx.notify();
            return;
        }
        if is_query(&self.search_text) {
            self.ask_engine(cx);
            return;
        }
        if let Some(data) = &self.data {
            // The word as the digits of an id, once: every commit is asked
            // whether its id begins with them, without writing each id out
            // as a string at every key.
            let digits = bitbraid_git::oid::hex_digits(&needle);
            for (ix, commit) in data.commits.iter().enumerate() {
                if data.worktree_rows.contains_key(&commit.id) {
                    continue; // its id is made up
                }
                let id_match = digits
                    .as_deref()
                    .is_some_and(|digits| commit.id.begins_with(digits));
                let detail_match = self.details.get(&commit.id).is_some_and(|d| {
                    d.summary.to_lowercase().contains(&needle)
                        || d.author.name.to_lowercase().contains(&needle)
                        || d.author.email.to_lowercase().contains(&needle)
                });
                let badge_match = data.badges.get(&commit.id).is_some_and(|badges| {
                    badges
                        .iter()
                        .any(|b| b.name.to_lowercase().contains(&needle))
                });
                if id_match || detail_match || badge_match {
                    self.matches.push(ix + self.offset());
                }
            }
        }
        self.resize_list();
        if let Some(first) = self.matches.first().copied() {
            self.selected = Some(first);
            self.scroll_to(first, ScrollStrategy::Center);
            self.emit_selection(cx);
        }
        cx.notify();
    }

    /// Hand the search text to the engine as a query: it has words that
    /// say what to look at, which the rows that are loaded cannot answer.
    fn ask_engine(&mut self, cx: &mut Context<Self>) {
        let asked = self.search_asked;
        self.search_pending = true;
        let request = Request::Command {
            id: "search.commits".to_owned(),
            params: json!({ "query": self.search_text.trim() }),
        };
        // The user typed it: a search of what files changed reads history
        // as git would, fetching what a partial clone left out.
        self.session
            .call_asked(request, cx, move |this, reply, cx| {
                if this.search_asked != asked {
                    return; // the text has changed since
                }
                this.search_pending = false;
                match reply {
                    Reply::Command(value) => {
                        let offset = this.offset();
                        if let Some(data) = &this.data {
                            // The rows the graph has for the ids, in its order.
                            let mut rows: Vec<usize> = value
                                .get("ids")
                                .and_then(|ids| ids.as_array())
                                .into_iter()
                                .flatten()
                                .filter_map(|id| id.as_str()?.parse::<Oid>().ok())
                                .filter_map(|id| data.index.get(&id).copied())
                                .map(|row| row.saturating_add(offset))
                                .collect();
                            rows.sort_unstable();
                            this.matches = rows;
                        }
                        this.resize_list();
                        if let Some(first) = this.matches.first().copied() {
                            this.selected = Some(first);
                            this.scroll_to(first, ScrollStrategy::Center);
                            this.emit_selection(cx);
                        }
                    }
                    // A key whose value is still being typed is not a mistake.
                    Reply::Error(_) if this.search_text.trim_end().ends_with(':') => {}
                    Reply::Error(err) => this.search_error = Some(query_error(&err)),
                    _ => {}
                }
                cx.notify();
            });
        cx.notify();
    }

    /// Nearest branch badge name for a commit without badges: follow the
    /// nearest child (a row above) until a badge is found. Bounded, and the
    /// answer for the hovered row is kept until the hover moves.
    fn nearest_badge(&mut self, ix: usize) -> Option<Ghost> {
        if let Some((row, ghost)) = &self.ghost
            && *row == ix
        {
            return ghost.clone();
        }
        let ghost = self.find_nearest_badge(ix);
        self.ghost = Some((ix, ghost.clone()));
        ghost
    }

    fn find_nearest_badge(&self, ix: usize) -> Option<Ghost> {
        let data = self.data.as_ref()?;
        let mut row = ix;
        for _ in 0..5000 {
            let id = data.commits.get(row)?.id;
            // A stash is no branch to be near to.
            if let Some(badge) = data
                .badges
                .get(&id)
                .and_then(|badges| badges.iter().find(|badge| badge.kind != BadgeKind::Stash))
            {
                return Some(ghost_of(badge, &data.worktrees, id));
            }
            row = usize::try_from(*data.child.get(row)?).ok()?;
        }
        None
    }

    /// Select the next row down or up the graph that carries a branch.
    fn jump_branch(&mut self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rows = match &self.branch_rows {
            Some(rows) => rows.clone(),
            None => {
                let rows = Rc::new(
                    self.data
                        .as_ref()
                        .map(|data| rows_with_branches(data))
                        .unwrap_or_default(),
                );
                self.branch_rows = Some(rows.clone());
                rows
            }
        };
        // From the row of uncommitted work the way is down only.
        let from = self.selected_commit();
        if let Some(ix) = next_of(&rows, from, down) {
            self.select(Some(ix.saturating_add(self.offset())), true, window, cx);
        }
    }

    /// A label as [`Self::render_label`] draws it, which notes when the
    /// secondary button goes down on it: the menu that opens is then its
    /// own. `spoken` unless it is the copy of a label widened over it,
    /// which assistive technology has once.
    #[allow(clippy::too_many_arguments)]
    fn render_label_in_row(
        id: (&'static str, usize),
        list_ix: usize,
        label: &RefLabel,
        colour: Hsla,
        cap: Option<f32>,
        changed: Option<usize>,
        measure: Option<Measure>,
        spoken: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let pointed = label.clone();
        Self::render_label(label, colour, cap, changed, measure, cx)
            .id(id)
            // Two clicks check out what it names.
            .when(spoken, |chip| {
                chip.role(Role::Button)
                    .aria_label(access::label_name(label))
            })
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, _, _, _| {
                    this.menu_label = Some((list_ix, pointed.clone()));
                }),
            )
    }

    /// The canvas painting one row's part of the graph into the graph
    /// column, scrolled by `scroll_x` and clipped to the column. `ix` is the
    /// commit index, or `None` for the row of uncommitted work.
    fn render_lanes(
        &self,
        ix: Option<usize>,
        node: Node,
        label_line: LabelLine,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(data) = self.data.clone() else {
            return div().into_any_element();
        };
        // The lane of uncommitted work is dashed from its row down to HEAD.
        let head_ix = data
            .wip
            .as_ref()
            .filter(|wip| wip.continues)
            .and_then(|_| data.head)
            .and_then(|head| match data.index.get(&head) {
                Some(ix) => Some(*ix),
                // Further down a history of which the leading rows are
                // here: below every row there is.
                None => data.walked.map(|_| usize::MAX),
            });
        let wip_slot = match (ix, head_ix, &data.wip) {
            (Some(ix), Some(head_ix), Some(wip)) if ix < head_ix => Some(wip.lane),
            _ => None,
        };
        let (refs_width, graph_width) = self.widths(cx);
        let paint = RowPaint {
            data,
            ix,
            node,
            label_line,
            selected,
            stash_slots: ix
                .map(|ix| stash_slots_at(&self.stash_lanes, ix))
                .unwrap_or_default(),
            wip_slot,
            wip_ends_here: ix.is_some() && ix == head_ix,
            alone: self.shown().is_some(),
            colors: (0..crate::theme::lane_count(cx))
                .map(|c| lane_color(c, cx))
                .collect(),
            background: cx.theme().background,
            is_dark: cx.theme().is_dark(),
            scroll_x: self.scroll_x,
        };
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    paint.paint(bounds, window);
                });
            },
        )
        .absolute()
        .top_0()
        .bottom_0()
        .left(px(refs_width))
        .w(px(graph_width))
        .into_any_element()
    }

    /// A place as large as the author's circle of the row `list_ix`, where
    /// the circle is drawn in its lane (`lane`), with a tooltip that names
    /// the author, and the committer when that is someone else. It takes
    /// nothing from the row: a click, a drag, the row's tint under the
    /// pointer and its menu are the row's as anywhere else on it. None
    /// where no circle is drawn: a merge's dot, a stash, a commit whose
    /// author is not read yet, a graph put away, a node held at the edge
    /// while only what a search found is shown.
    fn author_circle(
        &self,
        list_ix: usize,
        lane: u16,
        node: &Node,
        detail: Option<&CommitInfo>,
        cx: &App,
    ) -> Option<AnyElement> {
        if !matches!(node, Node::Commit(..)) {
            return None;
        }
        let detail = detail?;
        let (refs, graph) = self.widths(cx);
        if graph <= 0.0 || !graph.is_finite() {
            return None;
        }
        let at_lane = LaneGeometry {
            left: px(0.0),
            width: graph,
            scroll_x: self.scroll_x,
        }
        .offset(lane);
        let (centre, held) = node_centre(at_lane, graph);
        if !centre.is_finite() || (held && self.shown().is_some()) {
            return None;
        }
        let said = circle_said(&detail.author, &detail.committer);
        Some(
            div()
                .id(("author-circle", list_ix))
                .absolute()
                .left(px(refs + centre - NODE_RING))
                .top(px(ROW_HEIGHT / 2.0 - NODE_RING))
                .size(px(2.0 * NODE_RING))
                .rounded_full()
                .tooltip(person_tooltip(said))
                .test_support()
                .into_any_element(),
        )
    }

    /// The fill of a selected row: from the end of the graph column to the
    /// row's end, as high as the row's band, which is filled where it is
    /// painted. The labels' column stays as it is.
    fn selected_fill(&self, cx: &App) -> Div {
        let (refs, graph) = self.widths(cx);
        div()
            .absolute()
            .top(px(BAND_INSET))
            .h(px(ROW_HEIGHT - 2.0 * BAND_INSET))
            .left(px(refs + graph))
            .right_0()
            .bg(cx.theme().list_active)
    }

    /// The tint of the row under the pointer: from the end of the graph
    /// column to the row's end, as high as the row's band. It shows while
    /// the pointer is on the row (`ROW_GROUP`).
    fn hover_fill(&self, cx: &App) -> Div {
        let (refs, graph) = self.widths(cx);
        let tint = cx.theme().accent;
        div()
            .absolute()
            .top(px(BAND_INSET))
            .h(px(ROW_HEIGHT - 2.0 * BAND_INSET))
            .left(px(refs + graph))
            .right_0()
            .group_hover(ROW_GROUP, move |style| style.bg(tint))
    }

    /// Text colour on a label filled with a lane colour.
    fn label_text(cx: &App) -> Hsla {
        if cx.theme().is_dark() {
            cx.theme().background
        } else {
            hsla(0., 0., 1., 1.)
        }
    }

    /// One reference label: the name, then an icon for each place it is in
    /// and for what else it stands for; the checked-out branch has a check
    /// mark before its name. `cap` is the widest it may be, its name cut
    /// to fit and its icons kept; none is as wide as it needs. `measure`
    /// is told, as the label is laid out, whether its name is cut.
    fn render_label(
        label: &RefLabel,
        colour: Hsla,
        cap: Option<f32>,
        changed: Option<usize>,
        measure: Option<Measure>,
        cx: &App,
    ) -> Div {
        use gpui_kit::assets::IconName;
        let text = Self::label_text(cx);
        let mut icons: Vec<IconName> = Vec::new();
        // Here: a branch of this repository, or the commit HEAD is at.
        if label.local || label.head {
            icons.push(IconName::Laptop);
        }
        // One cloud for each remote that has a branch of this name here;
        // the picture of its owner instead where there is one.
        let owners: Vec<AnyElement> = label
            .remotes
            .iter()
            .filter_map(|remote| crate::providers::remote_picture(remote, 14., cx))
            .collect();
        icons.extend(
            label
                .remotes
                .iter()
                .skip(owners.len())
                .map(|_| IconName::Cloud),
        );
        // What the checked-out branch follows.
        if label.upstream {
            icons.push(IconName::ArrowUpToLine);
        }
        if label.tag {
            icons.push(IconName::Tag);
        }
        if label.stash {
            icons.push(IconName::Archive);
        }
        if label.worktree.is_some() {
            icons.push(IconName::FolderTree);
        }
        h_flex()
            .flex_none()
            .when_some(cap, |d, cap| d.max_w(px(cap)))
            .h(px(LABEL_HEIGHT))
            .gap_1p5()
            .px(px(5.))
            .items_center()
            .rounded(px(3.))
            .bg(colour)
            .text_color(text)
            .text_xs()
            .when(label.head, |d| {
                d.font_semibold().child(
                    Icon::new(gpui_kit::component::IconName::Check)
                        .size_3()
                        .flex_none()
                        .text_color(text),
                )
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .child(label.name.clone())
                    .when_some(measure, |name, measure| {
                        name.child(measure_name(label.name.clone(), measure))
                    }),
            )
            .children(
                icons
                    .into_iter()
                    .map(|icon| Icon::new(icon).size_3p5().flex_none().text_color(text)),
            )
            .children(owners)
            // How many files are changed in the worktree that has it.
            .when_some(changed.filter(|count| *count > 0), |d, count| {
                d.child(div().flex_none().child(format!("{count}")))
            })
    }

    /// The line from the labels towards the node: bold and in the lane's
    /// colour for the checked-out branch, thin and faint for another.
    fn render_connector(colour: Hsla, line: LabelLine) -> Div {
        let (thick, strength) = line.stroke();
        div()
            .flex_1()
            .min_w(px(LABELS_CONNECTOR))
            .h(px(thick))
            .bg(colour.opacity(strength))
    }

    /// The cell under the graph column: it reserves the width and takes the
    /// horizontal wheel.
    fn render_graph_cell(&self, list_ix: usize, cx: &mut Context<Self>) -> Stateful<Div> {
        div()
            .id(("graph-cell", list_ix))
            .w(px(self.widths(cx).1))
            .h_full()
            .flex_none()
            .on_scroll_wheel(
                cx.listener(|this, event: &ScrollWheelEvent, _, cx| this.scroll_graph(event, cx)),
            )
    }

    /// One count per kind of change, each with its mark.
    fn render_counts(counts: WipCounts, theme: &gpui_kit::component::Theme) -> Vec<AnyElement> {
        use gpui_kit::assets::IconName as Extra;
        [
            (counts.modified, Icon::new(Extra::Pencil), theme.yellow),
            (
                counts.added,
                Icon::new(gpui_kit::component::IconName::Plus),
                theme.green,
            ),
            (
                counts.deleted,
                Icon::new(gpui_kit::component::IconName::Minus),
                theme.red,
            ),
            (counts.renamed, Icon::new(Extra::FilePen), theme.blue),
        ]
        .into_iter()
        .filter(|(count, ..)| *count > 0)
        .map(|(count, icon, colour)| {
            h_flex()
                .flex_none()
                .gap_1()
                .items_center()
                .child(icon.size_3().text_color(colour))
                .child(format!("{count}"))
                .into_any_element()
        })
        .collect()
    }

    /// The row of another worktree's uncommitted work: it sits right above
    /// the commit that worktree has checked out, with a dashed node like
    /// the row of the open worktree, a label that names the worktree and
    /// the counts of what is changed there. One click selects it, two open
    /// the worktree, where its changes are staged. While the worktree is
    /// being removed the row says so, faded, and offers nothing.
    fn render_worktree_row(
        &self,
        list_ix: usize,
        ix: usize,
        mark: &WorktreeMark,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let refs_width = self.widths(cx).0;
        let selected = self.selected == Some(list_ix);
        let lane_colour = self
            .data
            .as_ref()
            .and_then(|data| data.rows.get(ix))
            .map(|row| lane_color(row.color, cx))
            .unwrap_or(theme.primary);
        let counts = self
            .worktree_changes
            .get(&mark.path)
            .copied()
            .unwrap_or_default();
        let label = RefLabel {
            name: worktrees::folder_name(&mark.path),
            head: false,
            local: false,
            remotes: Vec::new(),
            tag: false,
            stash: false,
            worktree: Some(mark.path.clone()),
            upstream: false,
        };
        let (open_path, menu_path) = (mark.path.clone(), mark.path.clone());
        let on_row = self
            .data
            .as_ref()
            .and_then(|data| data.index.get(&mark.head).copied())
            .unwrap_or(ix);
        let removing = self.removing.contains(&mark.path);
        let shade = if removing {
            Some(FADED)
        } else {
            self.shade(on_row)
        };
        let said = match &mark.branch {
            _ if removing => say!("graph-worktree-removing", name = label.name.as_str()),
            Some(branch) => say!(
                "graph-worktree-changes-on",
                name = label.name.as_str(),
                branch = branch.as_str()
            ),
            None => say!("graph-worktree-changes", name = label.name.as_str()),
        };
        div()
            .id(("worktree-row", list_ix))
            .relative()
            .h(px(ROW_HEIGHT))
            .w_full()
            .children(selected.then(|| self.selected_fill(cx)))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                if event.click_count() >= 2 && !removing {
                    cx.emit(GraphEvent::Worktree(WorktreeRequest::Open(
                        open_path.clone(),
                    )));
                } else {
                    this.select(Some(list_ix), false, window, cx);
                }
            }))
            .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                this.set_hover(if *hovering { Some(list_ix) } else { None }, cx)
            }))
            .context_menu(move |menu, _, _| match removing {
                true => worktrees::removing_menu(menu),
                false => worktrees::open_worktree_entry(menu, &menu_path),
            })
            .child(self.render_lanes(Some(ix), Node::Wip, LabelLine::Thin, selected, cx))
            .child(
                h_flex()
                    .size_full()
                    .items_center()
                    .child(
                        h_flex()
                            .w(px(refs_width))
                            .flex_none()
                            .pl_1()
                            .gap_1()
                            .items_center()
                            .overflow_hidden()
                            .child(Self::render_label(
                                &label,
                                lane_colour,
                                Some(label_cap(refs_width, 0)),
                                None,
                                None,
                                cx,
                            ))
                            .child(Self::render_connector(lane_colour, LabelLine::Thin)),
                    )
                    .child(self.render_graph_cell(list_ix, cx))
                    .child(
                        h_flex()
                            // Lit with the commit it stands on: its work
                            // belongs to that branch's line.
                            .when_some(shade, |d, shade| d.opacity(shade))
                            .flex_1()
                            .min_w_0()
                            .px_1()
                            .gap_3()
                            .items_center()
                            .text_sm()
                            .child(
                                div()
                                    .id(("worktree-said", list_ix))
                                    .flex_none()
                                    .max_w_full()
                                    .h(px(20.))
                                    .px_2()
                                    .rounded(theme.radius)
                                    .bg(theme.background)
                                    .text_color(theme.muted_foreground)
                                    .line_height(px(20.))
                                    .truncate()
                                    // Its counts are of tracked files only.
                                    .tooltip(|window, cx| {
                                        Tooltip::new(worktrees::TRACKED_ONLY.text())
                                            .build(window, cx)
                                    })
                                    .child(said),
                            )
                            .children(Self::render_counts(counts, &theme)),
                    ),
            )
            .into_any_element()
    }

    fn render_wip_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let refs_width = self.widths(cx).0;
        let selected = self.selected == Some(0);
        // Uncommitted work belongs to the line of HEAD; without a HEAD
        // among the rows it is in no run, and steps back while any is lit.
        let shade = self.shade(
            self.data
                .as_ref()
                .and_then(|d| d.head.and_then(|head| d.index.get(&head).copied()))
                .unwrap_or(usize::MAX),
        );
        let kinds = Self::render_counts(self.wip.unwrap_or_default(), &theme);
        div()
            .id("wip-row")
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(access::wip_name(self.wip))
            .aria_selected(selected)
            .when(selected, |row| row.aria_active_descendant())
            .relative()
            .h(px(ROW_HEIGHT))
            .w_full()
            .children(selected.then(|| self.selected_fill(cx)))
            .on_click(cx.listener(|this, _, window, cx| this.select(Some(0), false, window, cx)))
            // What can be done with all of it at once.
            .context_menu(|menu, _, _| {
                let wip = |what: WipAction| -> Box<dyn Action> { Box::new(WipDo { what }) };
                let menu = menu
                    .menu(say!("graph-wip-stage-all"), wip(WipAction::StageAll))
                    .menu(say!("graph-wip-unstage-all"), wip(WipAction::UnstageAll))
                    .menu(say!("graph-wip-discard-all"), wip(WipAction::DiscardAll))
                    .separator();
                stashes::push_entry(menu)
                    .separator()
                    .menu(say!("graph-wip-commit"), wip(WipAction::Commit))
                    .menu(say!("graph-wip-commit-all"), wip(WipAction::CommitAll))
            })
            .child(self.render_lanes(None, Node::Wip, LabelLine::None, selected, cx))
            .child(
                h_flex()
                    .size_full()
                    .items_center()
                    .child(
                        h_flex()
                            .w(px(refs_width))
                            .flex_none()
                            .pl_1()
                            .gap_1()
                            .items_center()
                            .overflow_hidden()
                            // Uncommitted work has no label: its row says
                            // what it is.
                            .child(div().flex_1()),
                    )
                    .child(self.render_graph_cell(0, cx))
                    .child(
                        h_flex()
                            .when_some(shade, |d, shade| d.opacity(shade))
                            .flex_1()
                            .min_w_0()
                            .px_1()
                            .gap_3()
                            .items_center()
                            .text_sm()
                            // While an operation is stopped part way the
                            // row is its notice, filled from the graph to
                            // the row's end: what it is and what it waits
                            // for.
                            .when_some(self.stopped.clone(), |cell, (said, blocked)| {
                                let fill = if blocked {
                                    theme.warning
                                } else {
                                    theme.success
                                };
                                // One row high, so a long sentence is
                                // cut: it is whole where the pointer
                                // rests on it, and at the head of the
                                // changes panel.
                                let whole = said.clone();
                                cell.child(
                                    h_flex()
                                        .id("wip-stopped")
                                        .flex_1()
                                        .min_w_0()
                                        .h(px(22.))
                                        .px_2()
                                        .gap_2()
                                        .items_center()
                                        .bg(fill.opacity(0.85))
                                        .text_color(theme.background)
                                        .tooltip(move |window, cx| {
                                            Tooltip::new(whole.clone()).build(window, cx)
                                        })
                                        // A warning while it cannot go
                                        // on; a tick once it can.
                                        .child(
                                            Icon::new(if blocked {
                                                gpui_kit::component::IconName::TriangleAlert
                                            } else {
                                                gpui_kit::component::IconName::Check
                                            })
                                            .small()
                                            .flex_none(),
                                        )
                                        .child(div().flex_1().min_w_0().truncate().child(said)),
                                )
                            })
                            // Where a message would be: the summary of the
                            // commit being written, in a field that says
                            // Summary while there is none yet. A click goes
                            // to where it is typed.
                            .when(self.stopped.is_none(), |cell| {
                                let typed = self
                                    .summary_source
                                    .as_ref()
                                    .map(|field| field.read(cx).value().to_string())
                                    .unwrap_or_default();
                                let width = wip_field_width(&typed);
                                cell.child(
                                    div()
                                        .id("wip-summary")
                                        .flex_none()
                                        .w(px(width))
                                        .h(px(22.))
                                        .px_2()
                                        .rounded(px(3.))
                                        .border_1()
                                        .border_color(theme.border)
                                        .bg(theme.background)
                                        .line_height(px(20.))
                                        .truncate()
                                        .cursor_text()
                                        .map(|d| {
                                            if typed.trim().is_empty() {
                                                d.text_color(theme.muted_foreground)
                                                    .child(say!("graph-wip-summary"))
                                            } else {
                                                d.child(typed)
                                            }
                                        })
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            cx.stop_propagation();
                                            this.select(Some(0), false, window, cx);
                                            cx.emit(GraphEvent::WriteSummary);
                                        })),
                                )
                                .children(kinds)
                            }),
                    ),
            )
            .into_any_element()
    }

    fn render_row(&mut self, list_ix: usize, cx: &mut Context<Self>) -> AnyElement {
        if self.offset() == 1 && list_ix == 0 {
            return self.render_wip_row(cx);
        }
        let Some(ix) = self.commit_ix(list_ix) else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let Some(data) = self.data.clone() else {
            return div().into_any_element();
        };
        let Some(commit) = data.commits.get(ix) else {
            return div().into_any_element();
        };
        let id = commit.id;
        if let Some(mark) = data
            .worktree_rows
            .get(&id)
            .and_then(|at| data.worktrees.get(*at))
        {
            return self.render_worktree_row(list_ix, ix, mark, cx);
        }
        let is_merge = commit.parents.len() > 1;
        let selected = self.is_selected(list_ix);
        // With several selected, the menu on one of them is about all.
        let many: Option<(Vec<String>, bool)> =
            (self.picked.len() >= 2 && self.picked.contains(&list_ix)).then(|| {
                let rows: Vec<&bitbraid_git::GraphCommit> = self
                    .picked
                    .iter()
                    .filter_map(|row| self.commit_ix(*row))
                    .filter_map(|ix| data.commits.get(ix))
                    .collect();
                (
                    rows.iter().map(|commit| commit.id.to_hex()).collect(),
                    rows.iter().any(|commit| commit.parents.len() > 1),
                )
            });
        let hovered = self.hovered == Some(list_ix);
        // Matches are collected in row order. While something is searched
        // for, what a row says is dimmed unless the row is one of them:
        // the graph and the labels stay as they are.
        let is_match = self.matches.binary_search(&list_ix).is_ok();
        let unmatched = unmatched(&self.search_text, self.search_pending, is_match);
        let detail = self.details.get(&id).cloned();
        let labels = data
            .badges
            .get(&id)
            .map(|badges| ref_labels(badges))
            .unwrap_or_default();
        let mut labels = if data.worktrees.is_empty() {
            labels
        } else {
            with_worktrees(labels, &data.worktrees, id)
        };
        // The one that is the upstream of the checked-out branch says so.
        if !labels.is_empty()
            && let Some(upstream) = self
                .current_branch
                .as_ref()
                .and_then(|branch| data.upstreams.get(branch))
        {
            mark_upstream(&mut labels, Some(upstream));
        }
        // For each label, how many files are changed in the worktree that
        // has it, when one has.
        let changed: Vec<Option<usize>> = labels
            .iter()
            .map(|label| {
                label
                    .worktree
                    .as_ref()
                    .and_then(|path| self.worktree_changes.get(path))
                    .map(WipCounts::total)
            })
            .collect();
        let worktree_paths: Vec<String> = labels
            .iter()
            .filter_map(|label| label.worktree.clone())
            .collect();
        let ghost = if labels.is_empty() && hovered {
            self.nearest_badge(ix)
        } else {
            None
        };
        let lane_colour = data
            .rows
            .get(ix)
            .map(|r| lane_color(r.color, cx))
            .unwrap_or(theme.primary);
        let commit_dates = cx.global::<Prefs>().preferences.commit_dates();
        let (seconds, date_offset) = shown_date(detail.as_ref(), commit.seconds, commit_dates);
        let columns = cx.global::<Prefs>().preferences.columns.clone();
        let now_ts = self.now;
        let is_stash = data
            .badges
            .get(&id)
            .is_some_and(|badges| badges.iter().any(|badge| badge.kind == BadgeKind::Stash));
        // What the row is when picked up, and what it is when dropped on:
        // its first branch or tag, or the commit itself.
        let named = labels.iter().find(|label| label.names_a_ref());
        let is_branch = |label: &RefLabel| label.local || (label.head && label.name != "HEAD");
        let dragged = match named.filter(|label| label.name != "HEAD") {
            Some(label) => Dragged {
                what: label.git_name(),
                label: label.git_name(),
                commit: id,
                local: is_branch(label),
                is_ref: true,
                merge: false,
            },
            None => Dragged {
                what: id.to_hex(),
                label: id.short(7),
                commit: id,
                local: false,
                is_ref: false,
                merge: is_merge,
            },
        };
        let landing = labels
            .iter()
            .find(|label| label.head && is_branch(label))
            .or_else(|| labels.iter().find(|label| is_branch(label)));
        let drop_target = DropTarget {
            commit: id,
            branch: landing.map(|label| label.name.clone()),
            what: landing
                .or(named.filter(|label| label.name != "HEAD"))
                .map(RefLabel::git_name)
                .unwrap_or_else(|| id.to_hex()),
            name: landing
                .or(named.filter(|label| label.name != "HEAD"))
                .map(RefLabel::git_name)
                .unwrap_or_else(|| id.short(7)),
        };
        let node = if is_stash {
            Node::Stash
        } else if is_merge {
            Node::Merge
        } else {
            Node::Commit(
                detail.as_ref().map(|d| d.author.email.clone()),
                detail
                    .as_ref()
                    .and_then(|d| crate::avatars::picture(&d.author.email, false, cx)),
            )
        };
        let divider = self
            .dividers
            .get(ix)
            .cloned()
            .flatten()
            .filter(|_| columns.date_dividers);

        let read_only = self.session.summary.bare;
        // Branch and tag labels, filled with the lane colour.
        let has_labels = !labels.is_empty() || ghost.is_some();
        // The line from the labels to the node: bold for the checked-out
        // branch's, thin and faint for any other; none while the labels'
        // column is put away.
        let (refs_w, _) = self.widths(cx);
        let label_line = match (labels.first(), has_labels) {
            _ if refs_w <= 0.0 => LabelLine::None,
            (Some(label), _) if label.head => LabelLine::Bold,
            (_, true) => LabelLine::Thin,
            (_, false) => LabelLine::None,
        };
        // What the row's menu offers to check out, and what two clicks on
        // its labels do.
        let targets: Vec<String> = labels.iter().filter_map(checkout_target).collect();
        let current = self.current_branch.clone();
        let at_head = data.head == Some(id);
        // One label is shown; the others are folded into a box that says
        // how many, and both unfold every label while the pointer is on
        // them.
        let hidden = labels.len().saturating_sub(1);
        let cap = label_cap(refs_w, hidden);
        let unfolded = hidden > 0 && self.unfold.row == Some(list_ix);
        // A label with what it does: two clicks check out what it names
        // (or open the worktree that has it), and something dropped on it
        // is about the branch or tag it names, not about the row.
        let dragging = self.label_drag;
        // The branches with an open pull request: a mark at the end of
        // their label; none while there are none.
        let pull_marks = self.pull_marks_for_rows();
        // A branch whose name holds an issue shows the issue's card under
        // the pointer.
        let issue_links = crate::providers::drawn_links(cx);
        // The label the pointer rests on is measured: whether its name is
        // cut, so that it is widened.
        let measuring = self.widen.measuring();
        // A label: the `index`th of the row's, drawn in the row (the
        // first), in the unfolded column, or widened over what is right of
        // it, which it is as wide as its name for.
        let chip = |index: usize,
                    place: widen::Place,
                    label: &RefLabel,
                    changed: Option<usize>,
                    cx: &mut Context<Self>| {
            let (at, cap) = match place {
                widen::Place::Row => (index, Some(cap)),
                widen::Place::Column => (index.saturating_add(32), Some(label_cap(refs_w, 0))),
                widen::Place::Wide => (index.saturating_add(48), None),
            };
            let key = (list_ix, index);
            let measure = measuring
                .as_ref()
                .filter(|(asked, _)| *asked == key && place != widen::Place::Wide)
                .map(|(_, said)| {
                    let said = said.clone();
                    Rc::new(move |cut: bool| said.set(Some((key, cut)))) as Measure
                });
            let pull = pull_marks.as_ref().and_then(|marks| {
                checks::pull_of(marks, &label.name, label.local || label.remote())
            });
            let pull_colour = Self::label_text(cx);
            let chip = Self::render_label_in_row(
                ("label", list_ix.wrapping_mul(64).wrapping_add(at)),
                list_ix,
                label,
                lane_colour,
                cap,
                changed,
                measure,
                place != widen::Place::Wide,
                cx,
            )
            .when(place != widen::Place::Wide, |chip| {
                chip.on_hover(cx.listener(move |this, hovering: &bool, window, cx| {
                    this.point_at_label(key, place, *hovering, window, cx)
                }))
            })
            .when_some(pull, |chip, pull| {
                chip.child(checks::pull_mark(
                    ("label-pull", list_ix.wrapping_mul(64).wrapping_add(at)).into(),
                    pull,
                    pull_colour,
                ))
            })
            .when_some(
                (label.local || label.remote())
                    .then(|| crate::views::issues::facts::branch_issue(&issue_links, &label.name))
                    .flatten(),
                |chip, (link, number)| {
                    chip.tooltip(move |window, cx| {
                        let link = link.clone();
                        Tooltip::element(move |_, cx| {
                            crate::views::issues::card::key_card(&link, number, cx)
                        })
                        .build(window, cx)
                    })
                },
            );
            let target = checkout_target(label);
            let worktree = label.worktree.clone();
            let chip = chip.on_click(cx.listener(move |_, event: &ClickEvent, _, cx| {
                if event.click_count() < 2 {
                    return;
                }
                if let Some(target) = &target {
                    cx.stop_propagation();
                    cx.emit(GraphEvent::Checkout(target.clone()));
                } else if let Some(path) = &worktree {
                    cx.stop_propagation();
                    cx.emit(GraphEvent::Worktree(WorktreeRequest::Open(path.clone())));
                }
            }));
            match label_target(label, id).filter(|_| !read_only) {
                Some(target) => {
                    let over = theme.primary;
                    // While a branch or a commit is dragged, every label it
                    // could be dropped on says so with an outline.
                    let chip = chip.relative().when(dragging, |chip| {
                        chip.child(
                            div()
                                .absolute()
                                .inset_0()
                                .rounded(px(3.))
                                .border_2()
                                .border_color(theme.ring),
                        )
                    });
                    chip.drag_over::<Dragged>(move |style, _, _, _| style.bg(over))
                        .on_drop(cx.listener(move |this, dragged: &Dragged, window, cx| {
                            cx.stop_propagation();
                            this.dropped(dragged, &target, window, cx)
                        }))
                        .test_support()
                        .into_any_element()
                }
                None => chip.test_support().into_any_element(),
            }
        };
        let mut refs_cell = h_flex()
            .w(px(refs_w))
            .flex_none()
            .pl(px(LABELS_LEFT.min(refs_w)))
            .items_center()
            .overflow_hidden();
        // A new branch or tag is being named here: the field stands where
        // the labels are, and the labels make room for it.
        let naming = self
            .naming
            .as_ref()
            .filter(|naming| naming.row == list_ix)
            .map(|naming| naming.field.clone());
        if let Some(field) = naming {
            let width = NAME_FIELD_WIDTH.min(refs_w - NAME_FIELD_LEFT).max(0.0);
            refs_cell = refs_cell.pl(px(NAME_FIELD_LEFT)).child(
                // The frame is the field's, as high as the row leaves
                // room for; the text in it is the toolkit's input.
                h_flex()
                    .id(("name-field", list_ix))
                    .flex_none()
                    .w(px(width))
                    .h(px(NAME_FIELD_HEIGHT))
                    .items_center()
                    .rounded(px(3.))
                    .border_1()
                    .border_color(theme.ring)
                    .bg(theme.background)
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(Input::new(&field).small().appearance(false)),
            );
        } else if let Some(label) = labels.first() {
            let first = chip(
                0,
                widen::Place::Row,
                label,
                changed.first().copied().flatten(),
                cx,
            );
            refs_cell = refs_cell.child(
                h_flex()
                    .id(("labels", list_ix))
                    .flex_none()
                    .gap(px(LABELS_GAP))
                    .items_center()
                    .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                        this.set_unfold(list_ix, ix, Over::Labels, *hovering, cx)
                    }))
                    .child(first)
                    .when(hidden > 0, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .h(px(LABEL_HEIGHT))
                                .w(px(folded_mark_width(hidden)))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(px(3.))
                                .bg(lane_colour.opacity(0.35))
                                .text_xs()
                                .text_color(theme.foreground)
                                .child(format!("+{hidden}")),
                        )
                    }),
            );
        }
        if let Some(Ghost { name, target }) = ghost {
            refs_cell = refs_cell.child(
                div()
                    .id(("ghost", list_ix))
                    .test_support()
                    .min_w_0()
                    .max_w(px(cap))
                    .h(px(LABEL_HEIGHT))
                    .px(px(5.))
                    .flex()
                    .items_center()
                    .rounded(px(3.))
                    .text_xs()
                    .bg(lane_colour.opacity(0.4))
                    .text_color(theme.foreground)
                    .child(div().min_w_0().truncate().child(name))
                    // Two clicks check the nearest branch out.
                    .on_click(cx.listener(move |_, event: &ClickEvent, _, cx| {
                        if event.click_count() >= 2
                            && let Some(target) = &target
                        {
                            cx.stop_propagation();
                            cx.emit(GraphEvent::Checkout(target.clone()));
                        }
                    })),
            );
        }
        if has_labels {
            refs_cell = refs_cell.child(Self::render_connector(lane_colour, label_line));
        }
        // The row's menu, kept for the column its labels unfold into: the
        // column lies over the rows, this one's included, and opens it
        // itself.
        let kept_menu: KeptMenu = Rc::new(RefCell::new(None));
        let view_of_menu = cx.entity().downgrade();
        // Unfolded: every label of the commit, a column of them from the
        // row down over the rows below, drawn over them.
        let all_labels = unfolded.then(|| {
            let column =
                v_flex()
                    .id(("all-labels", list_ix))
                    .occlude()
                    .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                        this.set_unfold(list_ix, ix, Over::Column, *hovering, cx)
                    }))
                    .absolute()
                    .top(px(BAND_INSET))
                    .left(px(LABELS_LEFT))
                    .gap(px(LABELS_STACKED_GAP))
                    // Opaque over the rows it lies on: what they have
                    // there (their labels, the lines to their nodes) does
                    // not show in the gaps between its labels or at their
                    // round corners.
                    .rounded(px(3.))
                    .bg(theme.background)
                    .children(labels.iter().zip(&changed).enumerate().map(
                        |(at, (label, changed))| {
                            chip(at, widen::Place::Column, label, *changed, cx)
                        },
                    ))
                    .capture_any_mouse_down(Self::no_label(cx))
                    .context_menu(Self::column_menu(
                        view_of_menu.clone(),
                        kept_menu.clone(),
                        list_ix,
                        ix,
                    ));
            deferred(column).with_priority(1)
        });
        // The label the pointer rests on, when its name is cut: as wide as
        // its whole name, at its own place, over the graph and the columns
        // right of it, opaque, nothing else moving. In the unfolded column
        // it lies over the rows below and takes the pointer from them, as
        // the column does. In the row it leaves the pointer to the row (its
        // tint, a drag, the label's menu), and a click on it selects the
        // row and reaches nothing under it.
        let widened = self
            .widen
            .shown()
            .filter(|(row, index)| *row == list_ix && (unfolded || *index == 0))
            .and_then(|(_, index)| {
                let label = labels.get(index)?;
                let down = if unfolded {
                    index as f32 * (LABEL_HEIGHT + LABELS_STACKED_GAP)
                } else {
                    0.0
                };
                let changed = changed.get(index).copied().flatten();
                let wide = div()
                    .id(("label-wide", list_ix))
                    .absolute()
                    .top(px(BAND_INSET + down))
                    .left(px(LABELS_LEFT))
                    .on_hover(cx.listener(move |this, hovering: &bool, window, cx| {
                        this.point_at_widened(list_ix, ix, *hovering, window, cx)
                    }))
                    .child(chip(index, widen::Place::Wide, label, changed, cx))
                    .test_support();
                // In the column: over the rows below, and the menu its own,
                // as the column's. In the row: the row's pointer and menu.
                let wide = if unfolded {
                    wide.occlude()
                        .capture_any_mouse_down(Self::no_label(cx))
                        .context_menu(Self::column_menu(
                            view_of_menu.clone(),
                            kept_menu.clone(),
                            list_ix,
                            ix,
                        ))
                        .into_any_element()
                } else {
                    wide.on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.click(list_ix, event.modifiers(), window, cx)
                    }))
                    .into_any_element()
                };
                Some(deferred(wide).with_priority(2))
            });

        // What the row is called for assistive technology.
        let spoken = access::row_name(
            if is_stash {
                access::RowKind::Stash
            } else if is_merge {
                access::RowKind::Merge
            } else {
                access::RowKind::Commit
            },
            &detail
                .as_ref()
                .map(|d| {
                    if is_stash {
                        stash_summary(&d.summary).to_owned()
                    } else {
                        d.summary.clone()
                    }
                })
                .unwrap_or_default(),
            detail.as_ref().map(|d| d.author.name.as_str()),
            &match (seconds, columns.absolute_dates) {
                (Some(s), true) => absolute_time(s, date_offset),
                (Some(s), false) => relative_time(s, now_ts),
                (None, _) => String::new(),
            },
            &labels,
        );
        // Message: summary, then a little after it the body in muted text
        // with "|" for its line breaks.
        let summary = detail
            .as_ref()
            .map(|d| {
                if is_stash {
                    stash_summary(&d.summary).to_owned()
                } else {
                    d.summary.clone()
                }
            })
            .unwrap_or_default();
        let body_line: String = detail
            .as_ref()
            .map(|d| {
                d.body
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default();
        // The whole message, under the pointer: once the row's text is
        // read, and not while the row's label is widened over the cell.
        let widened_here = self.widen.shown().is_some_and(|(row, _)| row == list_ix);
        let message_tip = detail
            .as_ref()
            .filter(|_| !widened_here)
            .map(|d| message_said(&summary, &d.body));
        // Outside the run of commits a label's pointer lights, what a row
        // says steps back; its labels and its part of the graph stay.
        let shade = self.shade(ix);
        // Issue keys in the summary are links.
        let view = cx.entity().downgrade();
        let open: crate::views::issues::linked::Open =
            std::rc::Rc::new(move |link, number, _, cx| {
                let _ = view.update(cx, |_, cx| cx.emit(GraphEvent::OpenIssue { link, number }));
            });
        let summary = crate::views::issues::linked::text(
            ("summary", list_ix),
            summary,
            &issue_links,
            open,
            cx,
        );
        let message_cell = h_flex()
            .when_some(shade, |d, shade| d.opacity(shade))
            .id(("message", list_ix))
            .when_some(message_tip, |cell, said| {
                cell.tooltip(message_tooltip(said))
            })
            .h_full()
            .pl(px(MESSAGE_INSET))
            .pr_2()
            .gap_1()
            .items_center()
            .child(
                div()
                    .flex_none()
                    .max_w_full()
                    .truncate()
                    .text_sm()
                    .when(unmatched, |d| d.opacity(UNMATCHED))
                    .child(summary),
            )
            // The checks of the commit, when they are known.
            .children(self.check_mark(id, list_ix, cx))
            // A signed commit carries a mark; what it stands for is said
            // when the pointer rests on it.
            .when_some(self.signatures.get(&id).cloned(), |d, signed| {
                let (icon, colour, said) = match &signed.verification {
                    Some(verification) => (
                        verification.icon(),
                        match verification.level() {
                            Level::Good => theme.success,
                            Level::Doubtful => theme.warning,
                            Level::Bad => theme.danger,
                        },
                        verification.sentence(),
                    ),
                    None => (
                        gpui_kit::assets::IconName::ShieldQuestionMark,
                        theme.muted_foreground,
                        unchecked_sentence(signed.format.as_deref()),
                    ),
                };
                d.child(
                    div()
                        .id(("signature", list_ix))
                        .flex_none()
                        .child(Icon::new(icon).size_3p5().text_color(colour))
                        .tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx)),
                )
            })
            .when(!body_line.is_empty(), |d| {
                d.child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .pl_1()
                        .text_color(theme.muted_foreground)
                        .when(unmatched, |d| d.opacity(UNMATCHED))
                        .child(body_line),
                )
            });

        // The columns after the graph, in their order: each as wide as
        // the heading has it, the last one taking what is left.
        let fitted = self.column_widths(cx);
        let count = fitted.len();
        let mut message_cell = Some(message_cell);
        let mut cells: Vec<AnyElement> = Vec::new();
        for (at, (column, width)) in fitted.into_iter().enumerate() {
            let sized = |cell: Div| {
                if at + 1 == count {
                    cell.flex_1().min_w(px(least_of(column)))
                } else {
                    cell.w(px(width)).flex_none()
                }
            };
            let cell: AnyElement = match column {
                Column::Summary => match message_cell.take() {
                    Some(cell) => {
                        if at + 1 == count {
                            cell.flex_1().min_w(px(least_of(column))).into_any_element()
                        } else {
                            cell.w(px(width)).flex_none().into_any_element()
                        }
                    }
                    None => continue,
                },
                // The author's name, or only their picture where the
                // column is too narrow for a name.
                Column::Author => {
                    let narrow = at + 1 != count && width < 60.0;
                    sized(h_flex())
                        .px_2()
                        .gap_1p5()
                        .items_center()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .when(narrow, |cell| {
                            // It names them under the pointer, as the
                            // circle in the graph does.
                            cell.children(detail.as_ref().map(|d| {
                                div()
                                    .id(("author-picture", list_ix))
                                    .flex_none()
                                    .child(crate::views::avatar::avatar(
                                        &d.author.email,
                                        px(16.),
                                        theme.is_dark(),
                                        cx,
                                    ))
                                    .tooltip(person_tooltip(circle_said(&d.author, &d.committer)))
                                    .test_support()
                            }))
                        })
                        .when(!narrow, |cell| {
                            cell.child(
                                div().min_w_0().truncate().child(
                                    detail
                                        .as_ref()
                                        .map(|d| d.author.name.clone())
                                        .unwrap_or_default(),
                                ),
                            )
                        })
                        .into_any_element()
                }
                Column::Date => sized(div())
                    .truncate()
                    .px_2()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(match (seconds, columns.absolute_dates) {
                        (Some(s), true) => absolute_time(s, date_offset),
                        (Some(s), false) => relative_time(s, now_ts),
                        (None, _) => String::new(),
                    })
                    .into_any_element(),
                Column::Id => sized(div())
                    .px_1()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(id.short(ID_LETTERS))
                    .into_any_element(),
            };
            cells.push(match shade.filter(|_| column != Column::Summary) {
                Some(shade) => div().opacity(shade).child(cell).into_any_element(),
                None => cell,
            });
        }

        // A time divider sits on the row's top edge without taking height:
        // a line across the whole row, and hanging from its right end a
        // small box that says how long ago the commits under it are. The
        // first row has nothing above it to part from, and none.
        let first = list_ix == 0;
        let divider_line = divider.as_deref().filter(|_| !first).map(|_| {
            div()
                .absolute()
                .top(px(-0.5))
                .left_0()
                .right_0()
                .h(px(1.))
                .bg(theme.border)
        });
        let divider_label = divider.as_deref().filter(|_| !first).map(|label| {
            div()
                .absolute()
                .top(px(1.))
                .right(px(DIVIDER_RIGHT))
                .h(px(DIVIDER_H))
                .px_1()
                .rounded(px(2.))
                .bg(theme.secondary)
                .text_size(px(10.))
                .line_height(px(DIVIDER_H))
                .text_color(theme.muted_foreground)
                .child(label.to_owned())
        });

        let circle = data
            .rows
            .get(ix)
            .filter(|_| !widened_here)
            .and_then(|layout| {
                self.author_circle(list_ix, layout.lane, &node, detail.as_ref(), cx)
            });
        let lanes = self.render_lanes(Some(ix), node, label_line, selected, cx);
        let (view, menu_data) = (cx.entity().downgrade(), data.clone());
        div()
            .id(("row", list_ix))
            .test_support()
            .role(Role::ListBoxOption)
            .aria_label(spoken)
            .aria_selected(selected)
            .when(self.selected == Some(list_ix), |row| {
                row.aria_active_descendant()
            })
            .relative()
            .h(px(ROW_HEIGHT))
            .w_full()
            .group(ROW_GROUP)
            .child(self.hover_fill(cx))
            .children(selected.then(|| self.selected_fill(cx)))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click(list_ix, event.modifiers(), window, cx)
            }))
            .on_hover(cx.listener(move |this, hovering: &bool, _, cx| {
                this.set_hover(if *hovering { Some(list_ix) } else { None }, cx)
            }))
            // Before a label hears of the secondary button: no label is
            // pointed at, so that the menu is the row's unless one says so.
            .capture_any_mouse_down(cx.listener(|this, event: &MouseDownEvent, _, _| {
                if event.button == MouseButton::Right {
                    this.menu_label = None;
                }
            }))
            // A stash is neither picked up nor landed on, and nothing is
            // in a repository that is read-only.
            .when(!is_stash && !read_only, |row| {
                let accent = theme.accent;
                row.on_drag(dragged, |dragged, _, _, cx| {
                    cx.new(|_| DragChip(dragged.label.clone()))
                })
                .drag_over::<Dragged>(move |style, _, _, _| style.bg(accent))
                .on_drop(cx.listener(
                    move |this, dragged: &Dragged, window, cx| {
                        this.dropped(dragged, &drop_target, window, cx)
                    },
                ))
            })
            .children(divider_line)
            .child(lanes)
            .child(
                h_flex()
                    .size_full()
                    .items_center()
                    .when(refs_w > 0.0, |row| row.child(refs_cell))
                    .child(self.render_graph_cell(list_ix, cx))
                    .children(cells),
            )
            .children(circle)
            .children(divider_label)
            .children(all_labels)
            .children(widened)
            .context_menu(keep_menu(&kept_menu, move |menu, window, cx| {
                // A commit's menu is longer than a small window is high,
                // and scrolls there.
                let menu = crate::views::menu_fit::fit(menu, window);
                let hex = id.to_hex();
                // Nothing in a read-only repository changes it: what is
                // left copies something out.
                if read_only {
                    let commits = match &many {
                        Some((commits, _)) => commits.iter().rev().cloned().collect(),
                        None => vec![hex.clone()],
                    };
                    let ids = match &many {
                        Some((commits, _)) => commits.join("\n"),
                        None => hex.clone(),
                    };
                    return crate::read_only::menu(menu)
                        .separator()
                        .menu(say!("graph-save-patch"), Box::new(SavePatches { commits }))
                        .menu(say!("graph-copy-link"), Box::new(CopyLink { commit: hex }))
                        .menu(say!("graph-copy-id"), Box::new(CopyId { id: ids }));
                }
                // The secondary button went down on a label: the menu of
                // the branch or tag it names, not of the commit.
                let pointed = view
                    .upgrade()
                    .and_then(|view| view.read(cx).menu_label.clone())
                    .filter(|(row, _)| *row == list_ix)
                    .map(|(_, label)| label)
                    .filter(|label| label.name != "HEAD");
                if let Some(label) = pointed {
                    let upstream = label
                        .local
                        .then(|| menu_data.upstreams.get(&label.name).cloned())
                        .flatten();
                    let protected = cx
                        .global::<Prefs>()
                        .preferences
                        .protected_branches
                        .contains(&label.name);
                    let seen = view.upgrade().and_then(|view| {
                        let state = cx.global::<Prefs>().repo_state(&view.read(cx).session.path);
                        LabelSeen::of(&label, &state.hidden, &state.only, &state.pinned)
                    });
                    let lightweight = label.tag
                        && view
                            .upgrade()
                            .is_some_and(|view| view.read(cx).lightweight.contains(&label.name));
                    return LabelMenu {
                        label: &label,
                        commit: &hex,
                        merge: is_merge,
                        upstream: upstream.as_deref(),
                        protected,
                        current: current.as_deref(),
                        facts: LabelFacts {
                            follows: upstream.is_some(),
                            at_head,
                            lightweight,
                            seen: seen.is_some(),
                            pulls: view
                                .upgrade()
                                .is_some_and(|view| view.read(cx).checks.linked()),
                        },
                        seen: seen.as_ref(),
                        pull: view.upgrade().and_then(|view| {
                            checks::pull_of(
                                &view.read(cx).pull_marks,
                                &label.name,
                                label.local || label.remote(),
                            )
                            .cloned()
                        }),
                    }
                    .menu(menu, window, cx);
                }
                // Several selected: the commit's menu, in the groups of
                // one, with what takes several in the plural and the rest
                // for the commit the menu is on.
                if let Some((commits, any_merge)) = &many {
                    let ids: Vec<Oid> = commits.iter().filter_map(|c| c.parse().ok()).collect();
                    let oldest = ids.last().copied();
                    let run = view.upgrade().is_some_and(|view| {
                        view.read(cx)
                            .data
                            .as_ref()
                            .is_some_and(|data| is_run(data, &ids))
                    });
                    let on_current = run
                        && oldest.is_some_and(|oldest| {
                            menu_data
                                .head
                                .and_then(|head| rewrite::descends(&menu_data, head, oldest))
                                .unwrap_or(true)
                        });
                    let top_at_head = ids.first().copied() == menu_data.head;
                    let plan = rewrite::many_row_plan(
                        current.as_deref(),
                        &hex,
                        at_head,
                        commits,
                        *any_merge,
                        on_current,
                        top_at_head,
                    );
                    let menu = menu
                        .menu(
                            say!("graph-checkout-detached"),
                            Box::new(CheckoutTarget {
                                target: hex.clone(),
                            }),
                        )
                        .separator();
                    let menu = worktrees::new_worktree_entry(menu, Some(&hex))
                        .separator()
                        .menu(
                            say!("graph-new-branch"),
                            Box::new(BranchAt { start: hex.clone() }),
                        );
                    let menu = rewrite::commit_row_entries(
                        menu,
                        plan,
                        current.as_deref(),
                        &hex,
                        at_head,
                        window,
                        cx,
                    )
                    .separator()
                    .menu(
                        say!("graph-copy-ids", count = commits.len()),
                        Box::new(CopyId {
                            id: commits.join("\n"),
                        }),
                    )
                    .menu(
                        say!("graph-save-patches", count = commits.len()),
                        Box::new(SavePatches {
                            // Oldest first: the order they apply in.
                            commits: commits.iter().rev().cloned().collect(),
                        }),
                    )
                    .separator()
                    .menu(
                        say!("graph-compare"),
                        Box::new(CompareAt {
                            commit: hex.clone(),
                        }),
                    )
                    .separator();
                    return tags::new_tag_entry(menu, &hex)
                        .separator()
                        .menu(say!("graph-select-only-this"), Box::new(ClearPicked));
                }
                // A stash is not a place on a branch: it is applied, popped
                // or dropped, and nothing is merged into it or reset to it.
                if is_stash {
                    let menu = stashes::stash_entries(menu, &hex).separator();
                    return stashes::reword_entry(menu, &hex)
                        .separator()
                        .menu(say!("graph-copy-id"), Box::new(CopyId { id: hex }));
                }
                let mut menu = menu;
                for target in &targets {
                    menu = menu.menu(
                        say!("graph-checkout-target", target = target.as_str()),
                        Box::new(CheckoutTarget {
                            target: target.clone(),
                        }),
                    );
                }
                for path in &worktree_paths {
                    menu = worktrees::open_worktree_entry(menu, path);
                }
                // In groups: checking out; a worktree; what is made of the
                // commit or brought to it; for a commit of the current
                // branch, rewriting from it; copying; comparing; a tag.
                let menu = menu
                    .menu(
                        say!("graph-checkout-detached"),
                        Box::new(CheckoutTarget {
                            target: hex.clone(),
                        }),
                    )
                    .separator();
                // For the branch on the row when there is one to check out
                // there, else for the commit.
                let menu = worktrees::new_worktree_entry(
                    menu,
                    Some(targets.first().map(String::as_str).unwrap_or(&hex)),
                )
                .separator()
                .menu(
                    say!("graph-new-branch"),
                    Box::new(BranchAt { start: hex.clone() }),
                );
                // A branch on the row is merged by name, which is what the
                // merge commit then says; a bare commit by its id.
                let (target, name) = match targets.first() {
                    Some(name) => (name.as_str(), name.as_str()),
                    None => (hex.as_str(), say!("graph-this-commit")),
                };
                // Whether the commit is one of the current branch: only
                // such a commit is rewritten from. Where the graph cannot
                // tell (too far back), the entries are there and git says.
                let on_current = menu_data
                    .head
                    .and_then(|head| rewrite::descends(&menu_data, head, id))
                    .unwrap_or(true);
                let plan = rewrite::commit_row_plan(
                    current.as_deref(),
                    target,
                    name,
                    &hex,
                    is_merge,
                    at_head,
                    on_current,
                );
                let menu = rewrite::commit_row_entries(
                    menu,
                    plan,
                    current.as_deref(),
                    &hex,
                    at_head,
                    window,
                    cx,
                )
                .separator()
                .menu(say!("graph-copy-id"), Box::new(CopyId { id: hex.clone() }))
                .menu(
                    say!("graph-save-patch"),
                    Box::new(SavePatches {
                        commits: vec![hex.clone()],
                    }),
                )
                .menu(
                    say!("graph-copy-link"),
                    Box::new(CopyLink {
                        commit: hex.clone(),
                    }),
                )
                .separator()
                .menu(
                    say!("graph-compare"),
                    Box::new(CompareAt {
                        commit: hex.clone(),
                    }),
                )
                .separator();
                tags::new_tag_entry(menu, &hex)
            }))
            .into_any_element()
    }

    /// The thumb of the graph column's own scroll bar, at the column's
    /// foot, while the graph is wider than the column: how much of it is
    /// on show, and where.
    fn render_graph_thumb(&self, cx: &App) -> Option<Div> {
        let total = self
            .data
            .as_ref()
            .map(|d| graph_extent(d.max_width))
            .unwrap_or(0.0);
        let (refs, graph) = self.widths(cx);
        if total <= graph || !total.is_finite() {
            return None;
        }
        let share = (graph / total).clamp(0.05, 1.0);
        let offset = (self.scroll_x / total).clamp(0.0, 1.0);
        Some(
            div()
                .absolute()
                .bottom(px(2.))
                .left(px(refs + graph * offset))
                .w(px(graph * share))
                .h(px(6.))
                .rounded(px(3.))
                .bg(cx.theme().muted_foreground.opacity(0.45)),
        )
    }

    /// Horizontal wheel (or shift+wheel) over the graph column scrolls it.
    fn scroll_graph(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(20.));
        let dx = if event.modifiers.shift && delta.x == px(0.) {
            delta.y
        } else {
            delta.x
        };
        if dx == px(0.) {
            return;
        }
        let graph_width = self.widths(cx).1;
        let total = self
            .data
            .as_ref()
            .map(|d| graph_extent(d.max_width))
            .unwrap_or(0.0);
        let max = (total - graph_width).max(0.0);
        self.scroll_x = (self.scroll_x - crate::zoom::unpx(dx)).clamp(0.0, max);
        cx.notify();
    }

    /// Column headers aligned with the row cells.
    fn render_header(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let columns = cx.global::<Prefs>().preferences.columns.clone();
        let commit_dates = cx.global::<Prefs>().preferences.commit_dates();
        let topo_order = cx.global::<Prefs>().preferences.order() == bitbraid_git::Order::Topo;
        let author_order =
            cx.global::<Prefs>().preferences.order() == bitbraid_git::Order::AuthorDate;
        let (refs_w, graph_w) = self.widths(cx);
        let label = |text: &'static str| {
            div()
                .text_size(px(10.))
                .font_family(theme.mono_font_family.clone())
                .whitespace_nowrap()
                .overflow_hidden()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        // A heading cell: its words in the middle of its height, and a thin
        // rule before every one but the first.
        let cell = |first: bool| {
            div()
                .flex()
                .items_center()
                .flex_none()
                .h_full()
                .relative()
                .pl(px(if first {
                    HEADER_FIRST_INSET
                } else {
                    HEADER_INSET
                }))
                .pr_1()
                .when(!first, |d| d.border_l_1().border_color(theme.border))
        };
        let mut header = h_flex()
            .w_full()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.list_head)
            .when(refs_w > 0.0, |header| {
                header.child(
                    cell(true)
                        .w(px(refs_w))
                        .overflow_hidden()
                        .child(label(say!("graph-heading-refs"))),
                )
            })
            .when(graph_w > 0.0, |header| {
                header.child(
                    cell(refs_w <= 0.0)
                        .id("graph-header")
                        .w(px(graph_w))
                        .overflow_hidden()
                        .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                            this.scroll_graph(event, cx)
                        }))
                        .child(label(say!("graph-heading-graph")))
                        .test_support(),
                )
            });
        // Each column as wide as its rows have it, the last taking what is
        // left; a border is dragged by a strip over the right edge of the
        // column it sizes.
        // What the rows have is known only once the list is laid out in
        // the frame, after this: the width it is laid out for is noted,
        // and a list of another width draws the heading again.
        let fitted = self.column_widths(cx);
        let pane = crate::zoom::unpx(self.scroll.bounds().size.width);
        self.heading_drawn.set(pane);
        self.heading_pane.set(pane);
        self.heading_asked.set(false);
        let count = fitted.len();
        // Where each border is that can be dragged, from the heading's
        // left edge: the right edge of the column it sizes.
        let mut borders: Vec<(Resize, f32)> = Vec::new();
        if refs_w > 0.0 {
            borders.push((Resize::Refs, refs_w));
        }
        if graph_w > 0.0 {
            borders.push((Resize::Graph, refs_w + graph_w));
        }
        let mut along = refs_w + graph_w;
        for (at, (column, width)) in fitted.into_iter().enumerate() {
            let last = at + 1 == count;
            let word = match column {
                Column::Summary => say!("graph-heading-summary"),
                Column::Author => say!("graph-heading-author"),
                Column::Date => say!("graph-heading-date"),
                Column::Id => say!("graph-heading-id"),
            };
            let over = theme.accent;
            let cell = cell(at == 0 && refs_w <= 0.0 && graph_w <= 0.0)
                .id(("heading", column as usize))
                .test_support()
                .overflow_hidden()
                .child(label(word))
                // Picked up and let go on another heading: the column goes
                // in that one's place.
                .on_drag(HeadingDrag(column), move |_, _, _, cx| {
                    cx.new(|_| DragChip(word.to_owned()))
                })
                .drag_over::<HeadingDrag>(move |style, _, _, _| style.bg(over))
                .on_drop(cx.listener(move |_, dragged: &HeadingDrag, window, cx| {
                    if dragged.0 != column {
                        window.dispatch_action(
                            Box::new(MoveColumn {
                                column: dragged.0,
                                to: column,
                            }),
                            cx,
                        );
                    }
                }));
            header = header.child(if last {
                cell.flex_1().min_w(px(least_of(column)))
            } else {
                cell.w(px(width))
            });
            along += width;
            if !last {
                borders.push((Resize::Column(column), along));
            }
        }
        // The strips that resize the columns lie over the headings, each
        // with its middle on its border; the gear lies over them.
        let handles: Vec<AnyElement> = borders
            .into_iter()
            .map(|(which, line)| Self::resize_handle(which, line, pane, &self.resize_grab, &theme))
            .collect();
        // The gear at the end chooses the columns. It lies over the end of
        // the header rather than after its columns: the columns are then
        // where the rows have them, and the gear is there however little
        // room the columns leave. A right click anywhere on the heading
        // opens the same menu.
        let choices = ColumnChoices {
            columns,
            commit_dates,
            topo_order,
            author_order,
        };
        let for_gear = choices.clone();
        header
            .id("graph-heading")
            .relative()
            .context_menu(move |menu, _, _| choices.menu(menu))
            .children(handles)
            .child(
                div()
                    .absolute()
                    .right_0()
                    .top_0()
                    .h_full()
                    .pr_1()
                    .bg(theme.list_head)
                    .child(
                        Button::new("columns-menu")
                            .accessibility_label(say!("graph-columns"))
                            .ghost()
                            .xsmall()
                            .icon(gpui_kit::component::IconName::Settings)
                            .dropdown_menu(move |menu, _, _| for_gear.menu(menu)),
                    ),
            )
            .into_any_element()
    }
}

/// What the menu of the columns' heading offers, and what is on.
#[derive(Clone)]
struct ColumnChoices {
    columns: crate::prefs::Columns,
    commit_dates: bool,
    topo_order: bool,
    author_order: bool,
}

impl ColumnChoices {
    /// The menu, in groups: the columns, then how the graph is shown (the
    /// dates, the order), then going back to the first layout. A column is
    /// moved by dragging its heading.
    fn menu(&self, menu: PopupMenu) -> PopupMenu {
        let mut menu = menu
            .menu_with_check(
                say!("graph-columns-refs"),
                self.columns.show_refs,
                Box::new(ToggleFirstColumns { graph: false }),
            )
            .menu_with_check(
                say!("graph-columns-graph"),
                self.columns.show_graph,
                Box::new(ToggleFirstColumns { graph: true }),
            );
        for column in Column::ALL {
            menu = menu.menu_with_check(
                if column == Column::Summary {
                    say!("graph-columns-summary")
                } else {
                    column.label()
                },
                self.columns.order.contains(&column),
                Box::new(ToggleColumn { column }),
            );
        }
        menu.separator()
            .menu_with_check(
                say!("graph-columns-absolute-dates"),
                self.columns.absolute_dates,
                Box::new(ToggleAbsoluteDates),
            )
            .menu_with_check(
                say!("graph-columns-date-dividers"),
                self.columns.date_dividers,
                Box::new(ToggleDateDividers),
            )
            .menu_with_check(
                say!("graph-columns-commit-dates"),
                self.commit_dates,
                Box::new(ToggleCommitDates),
            )
            .menu_with_check(
                say!("graph-columns-topological"),
                self.topo_order,
                Box::new(ToggleTopoOrder),
            )
            .menu_with_check(
                say!("graph-columns-author-order"),
                self.author_order,
                Box::new(ToggleAuthorOrder),
            )
            .separator()
            .menu(say!("graph-columns-reset"), Box::new(ResetColumns))
    }
}

impl Focusable for GraphView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GraphView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frames_rendered = self.frames_rendered.wrapping_add(1);
        // While the rows step back or come forward, a frame after each.
        if self
            .light
            .advance(cx.background_executor().now(), cx.reduce_motion())
        {
            window.request_animation_frame();
        }
        // A drag that has ended, dropped or not, outlines nothing.
        if self.label_drag && !cx.has_active_drag() {
            self.label_drag = false;
        }
        // Before the place is noted: while the list is being put back
        // where it was, that is where it is.
        self.keep_restoring();
        self.note_top(cx);
        if self.data.is_some() && !std::mem::replace(&mut self.logged.0, true) {
            crate::session::milestone("first graph render");
        }
        let theme = cx.theme().clone();
        // The search bar floats over the top right corner of the graph
        // while it is open.
        let search_bar = self.search_open.then(|| {
            let found = self.matches.len();
            // Which of them is the selected row, when it is one of them.
            let at = self
                .selected
                .and_then(|row| self.matches.binary_search(&row).ok());
            h_flex()
                .id("graph-search")
                .test_support()
                .absolute()
                // Over the right end of the columns' heading.
                .top(px(SEARCH_TOP - HEADER_HEIGHT - HEADER_GAP))
                .right(px(12.))
                .w(px(412.))
                .h(px(SEARCH_HEIGHT))
                .px_2()
                .gap_2()
                .items_center()
                .rounded(theme.radius)
                .bg(theme.popover)
                .border_1()
                .border_color(theme.border)
                .shadow_md()
                .occlude()
                .child(
                    Icon::new(gpui_kit::component::IconName::Search)
                        .size_4()
                        .flex_none()
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Input::new(&self.search).small()),
                )
                .child(
                    div()
                        .flex_none()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(match found {
                            _ if self.search_pending => say!("graph-search-pending").to_owned(),
                            _ if self.search_stopped => say!("graph-search-stopped").to_owned(),
                            _ if self.search_error.is_some() => "?".to_owned(),
                            n => match_count(at, n),
                        }),
                )
                // A search that runs can be stopped.
                .when(self.search_pending, |bar| {
                    bar.child(
                        Button::new("search-stop")
                            .accessibility_label(say!("graph-search-stop"))
                            .ghost()
                            .xsmall()
                            .icon(gpui_kit::assets::IconName::CircleStop)
                            .tooltip(say!("graph-search-stop"))
                            .on_click(cx.listener(|this, _, _, cx| this.stop_search(cx))),
                    )
                })
                .child(
                    Button::new("search-only")
                        .accessibility_label(say!("graph-search-only"))
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::assets::IconName::ListFilter)
                        .selected(self.only_matches)
                        .tooltip(if self.only_matches {
                            say!("graph-search-only-off")
                        } else {
                            say!("graph-search-only-on")
                        })
                        .on_click(cx.listener(|this, _, _, cx| this.toggle_only_matches(cx))),
                )
                .child(
                    Button::new("search-prev")
                        .accessibility_label(say!("graph-search-previous"))
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::component::IconName::ArrowUp)
                        .disabled(found == 0)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.step_match(false, window, cx)),
                        ),
                )
                .child(
                    Button::new("search-next")
                        .accessibility_label(say!("graph-search-next"))
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::component::IconName::ArrowDown)
                        .disabled(found == 0)
                        .on_click(
                            cx.listener(|this, _, window, cx| this.step_match(true, window, cx)),
                        ),
                )
                .child(
                    Button::new("search-close")
                        .accessibility_label(say!("graph-search-close"))
                        .ghost()
                        .xsmall()
                        .icon(gpui_kit::component::IconName::Close)
                        .on_click(cx.listener(|this, _, window, cx| this.close_search(window, cx))),
                )
        });
        // What is wrong with the query stands under the bar.
        let search_note = self
            .search_error
            .clone()
            .filter(|_| self.search_open)
            .map(|said| {
                div()
                    .absolute()
                    .top(px(40.))
                    .right(px(14.))
                    .w(px(410.))
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .bg(theme.popover)
                    .border_1()
                    .border_color(theme.danger)
                    .text_xs()
                    .text_color(theme.danger)
                    .occlude()
                    .child(said)
            });

        // A large history is shown from its first rows while the rest is
        // walked: said at the foot of the list until all of it is here.
        let walking = self
            .data
            .as_ref()
            .and_then(|data| Some((data.walked?, data.commits.len())))
            .map(|(walked, rows)| {
                div()
                    .absolute()
                    .bottom(px(6.))
                    .right(px(18.))
                    .px_2()
                    .py_1()
                    .rounded(theme.radius)
                    .bg(theme.popover)
                    .border_1()
                    .border_color(theme.border)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(walking_text(walked, rows))
            });
        let list = v_flex()
            .id("graph-list")
            .relative()
            .size_full()
            .child(
                v_virtual_list(
                    cx.entity().clone(),
                    "graph-rows",
                    self.item_sizes.get(),
                    move |this, range, window, cx| {
                        // The window was made wider or narrower: the rows
                        // are laid out for the list's new width, the
                        // heading was for the old one. It is drawn again
                        // with the next frame, rather than stay where it
                        // was, over other columns' values.
                        let pane = crate::zoom::unpx(this.scroll.bounds().size.width);
                        if (pane - this.heading_pane.get()).abs() > 0.5 {
                            this.heading_pane.set(pane);
                            this.heading_asked.set(true);
                            window.request_animation_frame();
                        }
                        // The list has held its offset to its end: what
                        // it made of a place being put back.
                        this.note_restoring();
                        // While only what a search found is shown, the
                        // rows of the list are the matches.
                        let rows: Vec<usize> = match this.shown() {
                            Some(shown) => range.filter_map(|at| shown.get(at).copied()).collect(),
                            None => range.collect(),
                        };
                        let mut missing = Vec::new();
                        if let Some(data) = this.data.clone() {
                            for list_ix in rows.iter().copied() {
                                if let Some(ix) = this.commit_ix(list_ix)
                                    && let Some(c) = data.commits.get(ix)
                                    && !this.details.contains_key(&c.id)
                                    && !this.pending.contains(&c.id)
                                    && !data.worktree_rows.contains_key(&c.id)
                                {
                                    missing.push(c.id);
                                }
                            }
                        }
                        this.request_details(missing, cx);
                        this.want_checks(&rows, cx);
                        rows.into_iter()
                            .map(|ix| this.render_row(ix, cx))
                            .collect::<Vec<_>>()
                    },
                )
                .track_scroll(&self.scroll)
                .pt(px(self.list_inset())),
            )
            .scrollbar(&self.scroll, ScrollbarAxis::Vertical);

        v_flex()
            .id("graph")
            .test_support()
            .key_context(GRAPH_CONTEXT)
            .track_focus(&self.focus_handle)
            // A list box of commits for assistive technology.
            .role(Role::ListBox)
            .aria_label(access::list_name())
            .on_action(
                cx.listener(|this, _: &SelectNext, window, cx| this.move_selection(1, window, cx)),
            )
            .on_action(cx.listener(|this, _: &ExtendNext, window, cx| {
                this.extend_selection(true, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ExtendPrev, window, cx| {
                this.extend_selection(false, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ClearPicked, _, cx| this.clear_picked(cx)))
            .on_action(
                cx.listener(|this, _: &SelectPrev, window, cx| this.move_selection(-1, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PageDown, window, cx| this.move_selection(20, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PageUp, window, cx| this.move_selection(-20, window, cx)),
            )
            .on_action(cx.listener(|this, _: &SelectFirst, window, cx| {
                this.move_selection(isize::MIN / 2, window, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectLast, window, cx| {
                this.move_selection(isize::MAX / 2, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CopyCommitId, _, cx| {
                if let Some(id) = this.selected_commit().and_then(|ix| {
                    this.data
                        .as_ref()
                        .and_then(|d| d.commits.get(ix))
                        .map(|c| c.id)
                }) {
                    cx.write_to_clipboard(ClipboardItem::new_string(id.to_hex()));
                }
            }))
            .on_action(cx.listener(|_, action: &CheckoutTarget, _, cx| {
                cx.emit(GraphEvent::Checkout(action.target.clone()))
            }))
            .on_action(cx.listener(|_, action: &BranchAt, _, cx| {
                cx.emit(GraphEvent::NewBranch(action.start.clone()))
            }))
            .on_action(cx.listener(|_, action: &RunRewrite, _, cx| {
                cx.emit(GraphEvent::Rewrite(action.rewrite.clone()))
            }))
            .on_action(cx.listener(|_, action: &RunStash, _, cx| {
                cx.emit(GraphEvent::Stash(action.request.clone()))
            }))
            .on_action(cx.listener(|_, action: &RunTag, _, cx| {
                cx.emit(GraphEvent::Tag(action.request.clone()))
            }))
            .on_action(cx.listener(|_, action: &RunWorktree, _, cx| {
                cx.emit(GraphEvent::Worktree(action.request.clone()))
            }))
            .on_action(cx.listener(|_, action: &CopyId, _, cx| {
                cx.write_to_clipboard(ClipboardItem::new_string(action.id.clone()));
            }))
            .on_action(cx.listener(|_, action: &SavePatches, _, cx| {
                cx.emit(GraphEvent::ExportPatches(action.commits.clone()))
            }))
            // What the graph shows of the reference a label names is
            // kept by the references panel: hand it on.
            .on_action(cx.listener(|_, action: &ToggleHidden, _, cx| {
                cx.emit(GraphEvent::Seen(Seen::Hidden(action.full_name.clone())))
            }))
            .on_action(cx.listener(|_, action: &ToggleSolo, _, cx| {
                cx.emit(GraphEvent::Seen(Seen::Solo(action.full_name.clone())))
            }))
            .on_action(cx.listener(|_, action: &TogglePinned, _, cx| {
                cx.emit(GraphEvent::Seen(Seen::Pinned(action.full_name.clone())))
            }))
            .on_action(cx.listener(|_, action: &MovePinned, _, cx| {
                cx.emit(GraphEvent::Seen(Seen::Moved(
                    action.full_name.clone(),
                    action.right,
                )))
            }))
            .on_action(cx.listener(|_, action: &LabelAction, _, cx| {
                let name = action.name.clone();
                match action.what {
                    LabelDo::Rename => cx.emit(GraphEvent::Branch(BranchRequest::Rename(name))),
                    LabelDo::Delete => cx.emit(GraphEvent::Branch(BranchRequest::Delete(name))),
                    LabelDo::DeleteOnRemote => {
                        cx.emit(GraphEvent::Branch(BranchRequest::DeleteOnRemote(name)))
                    }
                    LabelDo::FastForward => {
                        cx.emit(GraphEvent::Branch(BranchRequest::FastForward(name)))
                    }
                    LabelDo::Push => cx.emit(GraphEvent::Branch(BranchRequest::Push(name))),
                    LabelDo::CopyName => cx.write_to_clipboard(ClipboardItem::new_string(name)),
                    LabelDo::Pull => cx.emit(GraphEvent::Branch(BranchRequest::Pull(name))),
                    LabelDo::SetUpstream => {
                        cx.emit(GraphEvent::Branch(BranchRequest::SetUpstream(name)))
                    }
                }
            }))
            .on_action(
                cx.listener(|_, action: &WipDo, _, cx| cx.emit(GraphEvent::Wip(action.what))),
            )
            .on_action(
                cx.listener(|this, _: &NextBranch, window, cx| this.jump_branch(true, window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &PrevBranch, window, cx| this.jump_branch(false, window, cx)),
            )
            .on_action(cx.listener(|this, action: &CopyLink, _, cx| {
                let request = Request::Command {
                    id: "link.make".to_owned(),
                    params: json!({ "commit": action.commit }),
                };
                this.session.call(request, cx, |_, reply, cx| {
                    if let Reply::Command(value) = reply
                        && let Some(link) = value.get("link").and_then(|link| link.as_str())
                    {
                        cx.write_to_clipboard(ClipboardItem::new_string(link.to_owned()));
                    }
                });
            }))
            .on_action(cx.listener(|this, action: &CompareAt, window, cx| {
                let Ok(id) = action.commit.parse::<Oid>() else {
                    return;
                };
                let Some(row) = this
                    .data
                    .as_ref()
                    .and_then(|data| data.index.get(&id).copied())
                    .map(|ix| ix.saturating_add(this.offset()))
                else {
                    return;
                };
                // The row is where the pointer is: nothing scrolls.
                this.select(Some(row), false, window, cx);
                // Once the panel has heard which commit it is about.
                cx.defer_in(window, |_, window, cx| {
                    window.dispatch_action(Box::new(CompareWithWorktree), cx);
                });
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                this.focus_search(window, cx);
            }))
            .on_action(cx.listener(|this, _: &ToggleOnlyMatches, _, cx| {
                this.toggle_only_matches(cx);
            }))
            .on_action(cx.listener(|this, _: &FocusGraph, window, cx| {
                // Escape in the field that names something: given up.
                if let Some(naming) = &this.naming
                    && naming.field.focus_handle(cx).contains_focused(window, cx)
                {
                    this.naming = None;
                    this.focus_handle.focus(window, cx);
                    cx.notify();
                    return;
                }
                // From the search field, back to the rows, with what was
                // found still on show; asked again there, the bar is put
                // away. Whatever else has the keyboard inside the graph
                // (the menu of a row) has its own use for the key that
                // asks this: it closes.
                if this.search.focus_handle(cx).contains_focused(window, cx) {
                    this.focus_handle.focus(window, cx);
                } else if this.search_open && this.focus_handle.is_focused(window) {
                    this.close_search(window, cx);
                } else {
                    cx.propagate();
                }
            }))
            .size_full()
            .child(self.render_header(cx))
            .child(div().flex_none().h(px(HEADER_GAP)))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(list)
                    .children(self.render_graph_thumb(cx))
                    .children(walking)
                    .children(search_bar)
                    .children(search_note),
            )
            .children(
                self.drop_menu
                    .as_ref()
                    .map(|menu| self.render_drop_menu(menu, cx)),
            )
            // A branch, a tag or a commit being dragged: the labels it could
            // be dropped on are outlined.
            .on_drag_move(cx.listener(|this, _: &DragMoveEvent<Dragged>, _, cx| {
                if !this.label_drag {
                    this.label_drag = true;
                    cx.notify();
                }
            }))
            // A column border being dragged, wherever over the view the
            // pointer is; the widths are stored when it is let go.
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<Resize>, _, cx| {
                if let Some(which) = event.dragged_item().downcast_ref::<Resize>() {
                    this.resize_column(*which, event.event.position.x, event.bounds, cx);
                }
            }))
            .on_drop(cx.listener(|this, _: &Resize, _, cx| {
                this.resize_grab.set(None);
                cx.global::<Prefs>().save_preferences();
            }))
            .on_action(cx.listener(|_, action: &ToggleFirstColumns, _, cx| {
                let prefs = cx.global_mut::<Prefs>();
                let columns = &mut prefs.preferences.columns;
                if action.graph {
                    columns.show_graph = !columns.show_graph;
                } else {
                    columns.show_refs = !columns.show_refs;
                }
                prefs.save_preferences();
                cx.notify();
            }))
            .on_action(cx.listener(|_, action: &ToggleColumn, _, cx| {
                let prefs = cx.global_mut::<Prefs>();
                let order = &mut prefs.preferences.columns.order;
                if order.contains(&action.column) {
                    // Not the last one shown: something stays after the
                    // graph.
                    if order.len() > 1 {
                        order.retain(|c| *c != action.column);
                    }
                } else if action.column == Column::Summary {
                    // The message comes back where it was at first.
                    order.insert(0, action.column);
                } else {
                    order.push(action.column);
                }
                prefs.save_preferences();
                cx.notify();
            }))
            .on_action(cx.listener(|_, action: &MoveColumn, _, cx| {
                let prefs = cx.global_mut::<Prefs>();
                let order = &mut prefs.preferences.columns.order;
                *order = moved_column(order, action.column, action.to);
                prefs.save_preferences();
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &ToggleAbsoluteDates, _, cx| {
                let prefs = cx.global_mut::<Prefs>();
                prefs.preferences.columns.absolute_dates =
                    !prefs.preferences.columns.absolute_dates;
                prefs.save_preferences();
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &ToggleDateDividers, _, cx| {
                let prefs = cx.global_mut::<Prefs>();
                prefs.preferences.columns.date_dividers = !prefs.preferences.columns.date_dividers;
                prefs.save_preferences();
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &ToggleCommitDates, _, cx| {
                let prefs = cx.global_mut::<Prefs>();
                let commit = !prefs.preferences.commit_dates();
                prefs.preferences.graph_date = if commit { "commit" } else { "author" }.to_owned();
                prefs.save_preferences();
                cx.refresh_windows();
            }))
            .on_action(cx.listener(|_, _: &ToggleAuthorOrder, _, cx| {
                let prefs = cx.global_mut::<Prefs>();
                let author = prefs.preferences.order() != bitbraid_git::Order::AuthorDate;
                prefs.preferences.graph_order = if author { "author" } else { "date" }.to_owned();
                prefs.save_preferences();
                // The workspace sees the other order and asks for the graph.
                cx.refresh_windows();
            }))
            .on_action(cx.listener(|_, _: &ToggleTopoOrder, _, cx| {
                let prefs = cx.global_mut::<Prefs>();
                let topo = prefs.preferences.order() != bitbraid_git::Order::Topo;
                prefs.preferences.graph_order = if topo { "topo" } else { "date" }.to_owned();
                prefs.save_preferences();
                // The workspace sees the other order and asks for the graph.
                cx.refresh_windows();
            }))
            .on_action(cx.listener(|_, _: &ResetColumns, _, cx| {
                let prefs = cx.global_mut::<Prefs>();
                prefs.preferences.columns = crate::prefs::Columns::default();
                prefs.save_preferences();
                cx.notify();
            }))
            .when(self.data.is_none(), |this| {
                this.child(
                    div()
                        .p_2()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(say!("graph-loading")),
                )
            })
    }
}

#[cfg(test)]
mod scroll_tests {
    use super::{into_row, top_row};

    #[test]
    fn the_row_at_the_top_follows_the_scroll_offset() {
        assert_eq!(top_row(0.0, 28.0), 0);
        assert_eq!(top_row(-27.9, 28.0), 0);
        assert_eq!(top_row(-28.0, 28.0), 1);
        assert_eq!(top_row(-2800.0, 28.0), 100);
        // Pulled past the top, or nonsense from somewhere: the first row.
        assert_eq!(top_row(40.0, 28.0), 0);
        assert_eq!(top_row(f32::NAN, 28.0), 0);
        assert_eq!(top_row(f32::NEG_INFINITY, 28.0), 0);
        assert_eq!(top_row(-100.0, 0.0), 0);
    }

    #[test]
    fn how_far_into_the_top_row_is_a_part_of_one_row() {
        assert_eq!(into_row(0.0, 28.0, 0), 0.0);
        assert_eq!(into_row(-710.0, 28.0, top_row(-710.0, 28.0)), 10.0);
        assert_eq!(into_row(-2212.0, 28.0, 79), 0.0);
        // To a hundredth.
        assert_eq!(into_row(-28.333_33, 28.0, 1), 0.33);
        // Never outside the row, and nought for nonsense.
        assert_eq!(into_row(-100.0, 28.0, 0), 28.0);
        assert_eq!(into_row(40.0, 28.0, 0), 0.0);
        assert_eq!(into_row(f32::NAN, 28.0, 0), 0.0);
        assert_eq!(into_row(-100.0, 0.0, 0), 0.0);
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use gpui_kit::px;

    use super::Column;
    use super::{
        Comparison, LaneGeometry, MAX_RANGE, NODE_RING, RefLabel, bucket_label, checkout_target,
        click_rows, comparison, first_parent_line, graph_extent, label_target, ref_labels,
        stash_summary, time_bucket, unmatched, with_worktrees,
    };

    #[test]
    fn a_search_dims_what_it_did_not_find_once_it_has_answered() {
        // Nothing is searched for: every row is as it is.
        assert!(!unmatched("", false, false));
        assert!(!unmatched("   ", false, false));
        // A search that has answered dims the rows it did not find.
        assert!(unmatched("word", false, false));
        assert!(!unmatched("word", false, true));
        // While it is still looking nothing is dimmed: the rows would
        // flicker with every letter typed.
        assert!(!unmatched("word", true, false));
    }

    #[test]
    fn a_label_is_a_place_to_drop_on_by_itself() {
        use crate::views::rewrite::{Dragged, DropTarget, drop_choices};
        let commit: bitbraid_git::Oid = "11".repeat(20).parse().unwrap();
        let elsewhere: bitbraid_git::Oid = "22".repeat(20).parse().unwrap();
        let label = |name: &str| RefLabel {
            name: name.to_owned(),
            head: false,
            local: false,
            remotes: Vec::new(),
            tag: false,
            stash: false,
            worktree: None,
            upstream: false,
        };
        let local = RefLabel {
            local: true,
            ..label("feature")
        };
        let remote = RefLabel {
            remotes: vec!["origin/main".to_owned()],
            ..label("main")
        };
        let tag = RefLabel {
            tag: true,
            ..label("v1")
        };
        // A local branch is somewhere to merge into; a remote branch and a
        // tag are named, and nothing is merged into them.
        assert_eq!(
            label_target(&local, commit),
            Some(DropTarget {
                commit,
                branch: Some("feature".to_owned()),
                what: "feature".to_owned(),
                name: "feature".to_owned(),
            })
        );
        for other in [&remote, &tag] {
            let target = label_target(other, commit).unwrap();
            assert_eq!(target.branch, None);
            // By the name git gives it: on a remote, with the remote's.
            assert_eq!(target.what, other.git_name());
            assert_eq!(target.name, other.git_name());
        }
        // The checked-out branch is a branch; a detached HEAD, a stash and
        // the folder of a worktree are no targets of their own.
        let head = RefLabel {
            head: true,
            ..label("main")
        };
        assert_eq!(
            label_target(&head, commit).and_then(|target| target.branch),
            Some("main".to_owned())
        );
        let detached = RefLabel {
            head: true,
            ..label("HEAD")
        };
        let stash = RefLabel {
            stash: true,
            ..label("stash@{0}")
        };
        let folder = RefLabel {
            worktree: Some("/work/other".to_owned()),
            ..label("other")
        };
        for none in [&detached, &stash, &folder] {
            assert_eq!(label_target(none, commit), None, "{}", none.name);
        }

        // What a branch dropped on each means: on the row's other branch,
        // a merge into that one; on the remote branch beside it, none.
        let dragged = Dragged {
            what: "topic".to_owned(),
            label: "topic".to_owned(),
            commit: elsewhere,
            local: true,
            is_ref: true,
            merge: false,
        };
        let said = |target: &RefLabel| -> Vec<String> {
            let target = label_target(target, commit).unwrap();
            drop_choices(&dragged, &target, Some("main"))
                .into_iter()
                .map(|choice| choice.label)
                .collect()
        };
        assert_eq!(
            said(&local),
            ["Merge topic into feature", "Rebase topic onto feature"]
        );
        assert_eq!(said(&remote), ["Rebase topic onto origin/main"]);
        assert_eq!(said(&tag), ["Rebase topic onto v1"]);
    }
    use crate::session::{Badge, BadgeKind, WorktreeMark};

    fn badge(kind: BadgeKind, name: &str) -> Badge {
        Badge {
            kind,
            name: name.to_owned(),
            full_name: None,
        }
    }

    #[test]
    fn a_search_text_with_keys_quotes_or_a_dash_is_a_query() {
        use super::{is_query, query_error};
        for plain in [
            "fix",
            "fix lanes",
            "ada@fixture.invalid",
            "fix-up",
            "1.2",
            "a - b",
            ":x",
        ] {
            assert!(!is_query(plain), "{plain:?}");
        }
        for query in [
            "author:ada",
            "fix author:ada",
            "-wip",
            "fix -merge",
            "\"two words\"",
            "is:merge",
            "PATH:src",
            "-author:bob",
        ] {
            assert!(is_query(query), "{query:?}");
        }
        assert_eq!(
            query_error("invalid parameter \"query\": at byte 7: expected a value after `author:`"),
            "At byte 7: expected a value after `author:`"
        );
        assert_eq!(query_error("something else"), "Something else");
        assert_eq!(query_error(""), "");
    }

    #[test]
    fn other_worktrees_are_marked_on_what_they_have_checked_out() {
        let commit: bitbraid_git::Oid = "11".repeat(20).parse().unwrap();
        let other: bitbraid_git::Oid = "22".repeat(20).parse().unwrap();
        let mark = |path: &str, head, branch: Option<&str>| WorktreeMark {
            path: path.to_owned(),
            head,
            branch: branch.map(str::to_owned),
        };
        let marks = [
            mark("/work/on-topic", commit, Some("topic")),
            mark("/work/detached", commit, None),
            mark("/work/elsewhere", other, Some("main")),
        ];
        let labels = with_worktrees(
            ref_labels(&[
                badge(BadgeKind::Head, "main"),
                badge(BadgeKind::Local, "topic"),
                badge(BadgeKind::Tag, "v1"),
            ]),
            &marks,
            commit,
        );
        let seen: Vec<(&str, Option<&str>)> = labels
            .iter()
            .map(|label| (label.name.as_str(), label.worktree.as_deref()))
            .collect();
        // On the label of its branch; a detached one as a label of its
        // own, by its folder; one at another commit not at all.
        assert_eq!(
            seen,
            [
                ("main", None),
                ("topic", Some("/work/on-topic")),
                ("v1", None),
                ("detached", Some("/work/detached")),
            ]
        );
        // A branch another worktree has cannot be checked out here, and a
        // folder is no reference to merge or drag.
        assert_eq!(checkout_target(&labels[1]), None);
        assert_eq!(checkout_target(&labels[3]), None);
        assert_eq!(checkout_target(&labels[2]).as_deref(), Some("v1"));
        assert!(labels[1].names_a_ref() && !labels[3].names_a_ref());
        // Its branch is not on the row (hidden, or moved since): a label of
        // its own all the same.
        let alone = with_worktrees(Vec::new(), &marks, other);
        assert_eq!(alone.len(), 1);
        assert_eq!(alone[0].name, "elsewhere");
        assert!(with_worktrees(Vec::new(), &[], commit).is_empty());
    }

    #[test]
    fn the_graph_column_is_as_wide_as_its_lanes_need_up_to_a_limit() {
        use super::{AUTO_GRAPH_MOST, auto_graph_width};
        // 40 and 22 for each lane.
        assert_eq!(auto_graph_width(6), 172.0);
        assert_eq!(auto_graph_width(7), 194.0);
        assert_eq!(auto_graph_width(0), 62.0);
        assert_eq!(auto_graph_width(1), 62.0);
        assert_eq!(auto_graph_width(40), AUTO_GRAPH_MOST);
        assert_eq!(auto_graph_width(u16::MAX), AUTO_GRAPH_MOST);
    }

    #[test]
    fn a_heading_let_go_on_another_puts_its_column_in_that_place() {
        use super::moved_column;
        let order = [Column::Summary, Column::Author, Column::Date, Column::Id];
        assert_eq!(
            moved_column(&order, Column::Id, Column::Author),
            [Column::Summary, Column::Id, Column::Author, Column::Date]
        );
        assert_eq!(
            moved_column(&order, Column::Summary, Column::Id),
            [Column::Author, Column::Date, Column::Id, Column::Summary]
        );
        assert_eq!(moved_column(&order, Column::Date, Column::Date), order);
        assert_eq!(
            moved_column(&[Column::Summary], Column::Id, Column::Summary),
            [Column::Summary]
        );
    }

    #[test]
    fn a_dragged_border_resizes_within_limits() {
        use super::{Resize, resized};
        // A column ends where the pointer is, from its own left edge.
        assert_eq!(resized(Resize::Refs, 220.0), 220.0);
        assert_eq!(resized(Resize::Graph, 300.0), 300.0);
        assert_eq!(resized(Resize::Column(Column::Author), 216.0), 216.0);
        // Not below what it can still show.
        assert_eq!(resized(Resize::Column(Column::Id), 10.0), 55.0);
        assert_eq!(resized(Resize::Column(Column::Summary), 10.0), 120.0);
        // Dragged off either end, or to nowhere: still a column.
        for which in [
            Resize::Refs,
            Resize::Graph,
            Resize::Column(Column::Summary),
            Resize::Column(Column::Author),
            Resize::Column(Column::Date),
            Resize::Column(Column::Id),
        ] {
            for x in [
                -5000.0,
                0.0,
                1e9,
                f32::NAN,
                f32::INFINITY,
                f32::NEG_INFINITY,
            ] {
                let width = resized(which, x);
                assert!((40.0..=900.0).contains(&width), "{which:?} at {x}: {width}");
            }
        }
    }

    #[test]
    fn the_last_column_takes_what_is_left_and_the_others_give_way_from_the_right() {
        use super::fit_columns;
        let all = [
            (Column::Summary, 400.0),
            (Column::Author, 175.0),
            (Column::Date, 175.0),
            (Column::Id, 72.0),
        ];
        // Room enough: each its own width, the last the rest.
        assert_eq!(fit_columns(&all, 1000.0), [400.0, 175.0, 175.0, 250.0]);
        // The message alone takes all of it.
        assert_eq!(fit_columns(&all[..1], 700.0), [700.0]);
        // Too little: date, then author give way to their least, then the
        // message, so that the last keeps its least.
        assert_eq!(fit_columns(&all, 700.0), [400.0, 175.0, 70.0, 55.0]);
        assert_eq!(fit_columns(&all, 360.0), [207.0, 40.0, 58.0, 55.0]);
        // Less than every least: each its least.
        assert_eq!(fit_columns(&all, 10.0), [120.0, 40.0, 58.0, 55.0]);
        // Nothing drawn yet, a width that is no number, or no columns.
        assert_eq!(fit_columns(&all, 0.0), [400.0, 175.0, 175.0, 72.0]);
        assert_eq!(fit_columns(&all, f32::NAN), [400.0, 175.0, 175.0, 72.0]);
        assert!(fit_columns(&[], 500.0).is_empty());
        let odd = [(Column::Summary, f32::NAN), (Column::Id, -4.0)];
        assert_eq!(fit_columns(&odd, 500.0), [120.0, 380.0]);
    }

    #[test]
    fn one_label_stands_for_a_name_wherever_it_is() {
        let labels = ref_labels(&[
            badge(BadgeKind::Head, "main"),
            badge(BadgeKind::Local, "release/1.0"),
            badge(BadgeKind::Remote, "origin/main"),
            badge(BadgeKind::Remote, "origin/release/1.0"),
            badge(BadgeKind::Remote, "upstream/release/1.0"),
            badge(BadgeKind::Remote, "origin/topic"),
            badge(BadgeKind::Remote, "upstream/topic"),
            badge(BadgeKind::Remote, "upstream/hotfix"),
            badge(BadgeKind::Tag, "v1.0"),
            badge(BadgeKind::Remote, "origin/v1.0"),
        ]);
        let names: Vec<&str> = labels.iter().map(|l| l.name.as_str()).collect();
        // A remote branch is called by its own name, and one name on a
        // commit is one label, however many places have it; a tag and a
        // branch of the same name are two.
        assert_eq!(
            names,
            ["main", "release/1.0", "topic", "hotfix", "v1.0", "v1.0"]
        );
        let remotes =
            |at: usize| -> Vec<&str> { labels[at].remotes.iter().map(String::as_str).collect() };
        assert!(labels[0].head && labels[0].local);
        assert_eq!(remotes(0), ["origin/main"]);
        assert!(!labels[1].head && labels[1].local);
        assert_eq!(remotes(1), ["origin/release/1.0", "upstream/release/1.0"]);
        assert!(!labels[2].local && !labels[3].local);
        assert_eq!(remotes(2), ["origin/topic", "upstream/topic"]);
        assert_eq!(remotes(3), ["upstream/hotfix"]);
        assert!(labels[4].tag && !labels[4].local && !labels[4].remote());
        assert!(!labels[5].tag && remotes(5) == ["origin/v1.0"]);
        // What git calls each: the branch here, else the first remote's.
        let git: Vec<String> = labels.iter().map(RefLabel::git_name).collect();
        assert_eq!(
            git,
            [
                "main",
                "release/1.0",
                "origin/topic",
                "upstream/hotfix",
                "v1.0",
                "origin/v1.0"
            ]
        );
    }

    #[test]
    fn labels_offer_what_makes_sense_to_check_out() {
        let labels = ref_labels(&[
            badge(BadgeKind::Head, "main"),
            badge(BadgeKind::Local, "topic"),
            badge(BadgeKind::Remote, "origin/topic"),
            badge(BadgeKind::Remote, "origin/other"),
            badge(BadgeKind::Tag, "v1"),
            badge(BadgeKind::Stash, "stash@{0}"),
        ]);
        let targets: Vec<String> = labels.iter().filter_map(checkout_target).collect();
        // Not the checked-out branch and not a stash; a branch with both
        // sides once, by its local name; one only on a remote by the name
        // git gives it there.
        assert_eq!(targets, ["topic", "origin/other", "v1"]);
    }

    #[test]
    fn a_detached_head_is_not_a_local_branch() {
        let labels = ref_labels(&[
            badge(BadgeKind::Head, "HEAD"),
            badge(BadgeKind::Remote, "origin/HEAD"),
        ]);
        assert_eq!(labels.len(), 2);
        assert!(labels[0].head && !labels[0].local && !labels[0].remote());
    }

    #[test]
    fn a_label_is_as_wide_as_it_needs_up_to_what_its_column_leaves_beside_the_rest() {
        use super::{folded_mark_width, label_cap};
        // In the column of the window's start: all of it but the space
        // before the labels and the least of the line towards the node,
        // and the box of the folded ones and its gap where there is one.
        assert_eq!(label_cap(140.0, 0), 140.0 - 10.0 - 6.0);
        assert_eq!(label_cap(140.0, 2), 140.0 - 10.0 - 6.0 - 5.0 - 20.0);
        // A wider column, a wider label: no other bound.
        assert_eq!(label_cap(400.0, 0), 400.0 - 16.0);
        assert_eq!(label_cap(400.0, 3), 400.0 - 16.0 - 25.0);
        // The box grows with its digits.
        assert_eq!(folded_mark_width(9), 20.0);
        assert_eq!(folded_mark_width(12), 27.0);
        assert_eq!(folded_mark_width(0), 20.0);
        assert_eq!(label_cap(400.0, 12), 400.0 - 16.0 - 5.0 - 27.0);
        assert!(folded_mark_width(usize::MAX).is_finite());
        // A column dragged narrow leaves less, and nothing below none.
        assert!(label_cap(80.0, 0) < label_cap(140.0, 0));
        assert_eq!(label_cap(10.0, 1), 0.0);
        assert_eq!(label_cap(-5.0, 0), 0.0);
        assert_eq!(label_cap(f32::NAN, 0), 0.0);
        assert_eq!(label_cap(f32::INFINITY, 1), 0.0);
    }

    #[test]
    fn the_graph_scrolls_far_enough_to_show_its_outermost_nodes() {
        let width = 160.0;
        for lanes in [1u16, 9, 189, u16::MAX] {
            let last = lanes.saturating_sub(1);
            let start = LaneGeometry {
                left: px(0.),
                width,
                scroll_x: 0.0,
            };
            assert!(start.offset(0) - NODE_RING >= 0.0);
            let end = LaneGeometry {
                left: px(0.),
                width,
                scroll_x: (graph_extent(lanes) - width).max(0.0),
            };
            assert!(end.offset(last) + NODE_RING <= width);
            assert!(end.shows(last));
        }
    }

    #[test]
    fn an_edge_shows_when_it_crosses_the_column_between_hidden_lanes() {
        let geometry = LaneGeometry {
            left: px(0.),
            width: 160.0,
            scroll_x: 900.0,
        };
        assert!(!geometry.shows(0) && !geometry.shows(180));
        assert!(geometry.spans(0, 180) && geometry.spans(180, 0));
        assert!(!geometry.spans(0, 10));
        assert!(!geometry.spans(150, 180));
    }

    #[test]
    fn a_branch_line_follows_first_parents_only() {
        use std::collections::{HashMap, HashSet};

        use bitbraid_git::{GraphCommit, Oid};

        let oid = |b: u8| -> Oid { format!("{b:02x}").repeat(20).parse().unwrap() };
        // Rows: 0 = merge(1, 2), 1 -> 3, 2 -> 3, 3 = root; plus a commit whose
        // parent is outside the graph and one that is its own parent.
        let commits: Vec<GraphCommit> = [
            (10, vec![11, 12]),
            (11, vec![13]),
            (12, vec![13]),
            (13, vec![]),
            (20, vec![99]),
            (30, vec![30]),
        ]
        .into_iter()
        .map(|(id, parents)| GraphCommit {
            id: oid(id),
            parents: parents.into_iter().map(oid).collect(),
            seconds: None,
        })
        .collect();
        let index: HashMap<Oid, usize> =
            commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        let line = |row| first_parent_line(&commits, &index, |_| false, row);
        assert_eq!(line(0), HashSet::from([0, 1, 3]));
        assert_eq!(line(2), HashSet::from([2, 3]));
        assert_eq!(line(4), HashSet::from([4]));
        assert_eq!(line(5), HashSet::from([5]), "a cycle ends the line");
        assert!(line(600).is_empty(), "a row outside the graph has no line");
        // It ends before a commit another branch has a part in.
        let own = |row, stop: usize| first_parent_line(&commits, &index, |ix| ix == stop, row);
        assert_eq!(own(0, 3), HashSet::from([0, 1]));
        assert_eq!(own(0, 1), HashSet::from([0]));
        // The commit the label is on is lit whatever else stands on it.
        assert_eq!(own(1, 1), HashSet::from([1, 3]));
    }

    #[test]
    fn time_buckets_coarsen_with_age() {
        let now = 1_800_000_000;
        assert_eq!(time_bucket(now - 10, now), (0, 0));
        assert_eq!(time_bucket(now - 3 * 3600, now), (0, 3));
        assert_eq!(time_bucket(now - 2 * 86_400, now), (1, 2));
        assert_eq!(time_bucket(now - 40 * 86_400, now).0, 2);
        assert_eq!(bucket_label((0, 1)), "1 hour ago");
        assert_eq!(bucket_label((1, 4)), "4 days ago");
        assert_eq!(bucket_label((2, 18_262)), "2020-01-01");
    }

    fn oid(byte: u8) -> bitbraid_git::Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    #[test]
    fn clicks_select_one_row_add_one_or_take_a_range() {
        use std::collections::BTreeSet;
        let mut picked = BTreeSet::new();
        let mut anchor = None;
        let rows = |picked: &BTreeSet<usize>| picked.iter().copied().collect::<Vec<_>>();

        // A plain click is one row, and "several" stays empty.
        let cursor = click_rows(&mut picked, &mut anchor, None, 4, false, false);
        assert_eq!((cursor, rows(&picked), anchor), (Some(4), vec![], Some(4)));
        // Adding a second row keeps the first, which only the cursor knew.
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 7, true, false);
        assert_eq!(
            (cursor, rows(&picked), anchor),
            (Some(7), vec![4, 7], Some(7))
        );
        // A range from the last click, upwards as well as downwards.
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 9, false, true);
        assert_eq!((cursor, rows(&picked)), (Some(9), vec![7, 8, 9]));
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 5, false, true);
        assert_eq!(
            (cursor, rows(&picked), anchor),
            (Some(5), vec![5, 6, 7], Some(7))
        );
        // With both keys the range is added to what is there.
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 2, true, false);
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 3, true, true);
        assert_eq!((cursor, rows(&picked)), (Some(3), vec![2, 3, 5, 6, 7]));
        // Taking rows out again: the cursor moves off a row that goes.
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 3, true, false);
        assert_eq!((cursor, rows(&picked)), (Some(2), vec![2, 5, 6, 7]));
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 6, true, false);
        assert_eq!((cursor, rows(&picked)), (Some(2), vec![2, 5, 7]));
        // Down to one row is one row again.
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 5, true, false);
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 7, true, false);
        assert_eq!((cursor, rows(&picked)), (Some(2), vec![]));
        // And a plain click forgets the rest.
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 1, true, false);
        assert_eq!(rows(&picked), vec![1, 2]);
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 30, false, false);
        assert_eq!(
            (cursor, rows(&picked), anchor),
            (Some(30), vec![], Some(30))
        );

        // A range without anywhere to start from is a plain click.
        let (mut picked, mut anchor) = (BTreeSet::new(), None);
        let cursor = click_rows(&mut picked, &mut anchor, None, 3, false, true);
        assert_eq!((cursor, rows(&picked), anchor), (Some(3), vec![], Some(3)));
        // A range on the row it starts from is that row.
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 3, false, true);
        assert_eq!((cursor, rows(&picked)), (Some(3), vec![]));
        // However far away the other end is, a range is bounded, and the
        // cursor is where it stopped.
        let cursor = click_rows(&mut picked, &mut anchor, cursor, usize::MAX, false, true);
        assert_eq!(picked.len(), MAX_RANGE + 1);
        assert_eq!(cursor, Some(3 + MAX_RANGE));
        let cursor = click_rows(&mut picked, &mut anchor, cursor, 0, false, true);
        assert_eq!((cursor, rows(&picked)), (Some(0), vec![0, 1, 2, 3]));
    }

    #[test]
    fn several_commits_are_compared_as_a_run_or_as_two_ends() {
        let commit = |id: u8, parents: &[u8]| (oid(id), parents.iter().map(|p| oid(*p)).collect());
        // Newest first: 4 on 3 on 2, whose parent 1 is not selected.
        let run: Vec<(bitbraid_git::Oid, Vec<bitbraid_git::Oid>)> =
            vec![commit(4, &[3]), commit(3, &[2]), commit(2, &[1])];
        assert_eq!(
            comparison(&run),
            Some(Comparison {
                old: Some(oid(1)),
                new: oid(4),
                together: true,
            })
        );
        // Down to the first commit: against nothing.
        assert_eq!(
            comparison(&[commit(2, &[1]), commit(1, &[])]),
            Some(Comparison {
                old: None,
                new: oid(2),
                together: true,
            })
        );
        // A merge follows its first parent only.
        assert_eq!(
            comparison(&[commit(5, &[4, 9]), commit(4, &[3])]),
            Some(Comparison {
                old: Some(oid(3)),
                new: oid(5),
                together: true,
            })
        );
        // Two commits that do not follow one another: one against the
        // other.
        assert_eq!(
            comparison(&[commit(7, &[6]), commit(2, &[1])]),
            Some(Comparison {
                old: Some(oid(2)),
                new: oid(7),
                together: false,
            })
        );
        // Three that are not a run have no single diff; one or none is
        // not a selection of several.
        assert_eq!(
            comparison(&[commit(7, &[6]), commit(5, &[4]), commit(2, &[1])]),
            None
        );
        assert_eq!(comparison(&[commit(7, &[6])]), None);
        assert_eq!(comparison(&[]), None);
    }

    #[test]
    fn a_row_shows_the_author_date_or_the_commit_date() {
        use bitbraid_git::{CommitInfo, Signature};

        use super::shown_date;
        let commit = CommitInfo {
            id: oid(1),
            parents: Vec::new(),
            author: Signature {
                name: "Ada".to_owned(),
                email: "ada@fixture.invalid".to_owned(),
                seconds: 1_000,
                offset: 3_600,
            },
            committer: Signature {
                name: "Bob".to_owned(),
                email: "bob@fixture.invalid".to_owned(),
                seconds: 9_000,
                offset: -7_200,
            },
            summary: String::new(),
            body: String::new(),
        };
        assert_eq!(
            shown_date(Some(&commit), Some(9_000), false),
            (Some(1_000), 3_600)
        );
        assert_eq!(
            shown_date(Some(&commit), Some(9_000), true),
            (Some(9_000), -7_200)
        );
        // Before the commit is read: what the walk knows, either way.
        assert_eq!(shown_date(None, Some(9_000), false), (Some(9_000), 0));
        assert_eq!(shown_date(None, None, true), (None, 0));
    }

    #[test]
    fn the_upstream_of_the_checked_out_branch_is_marked() {
        use super::mark_upstream;
        let marked = |badges: &[Badge], upstream: Option<&str>| -> Vec<(String, bool)> {
            let mut labels = ref_labels(badges);
            mark_upstream(&mut labels, upstream);
            labels
                .into_iter()
                .map(|label| (label.name, label.upstream))
                .collect()
        };
        let label = |name: &str, upstream: bool| (name.to_owned(), upstream);
        // Only on remotes: the one label of that name has it.
        let behind = [
            badge(BadgeKind::Remote, "origin/main"),
            badge(BadgeKind::Remote, "fork/main"),
            badge(BadgeKind::Tag, "v1"),
        ];
        assert_eq!(
            marked(&behind, Some("origin/main")),
            [label("main", true), label("v1", false)]
        );
        assert_eq!(
            marked(&behind, Some("fork/main")),
            [label("main", true), label("v1", false)]
        );
        // It is where the branch is and shares its label: that label.
        let level = [
            badge(BadgeKind::Head, "main"),
            badge(BadgeKind::Local, "topic"),
            badge(BadgeKind::Remote, "origin/main"),
            badge(BadgeKind::Remote, "origin/topic"),
        ];
        assert_eq!(
            marked(&level, Some("origin/main")),
            [label("main", true), label("topic", false)]
        );
        // Another remote's branch of the same name shares the label, and
        // the upstream is elsewhere: nothing here is the upstream.
        let other = [
            badge(BadgeKind::Head, "main"),
            badge(BadgeKind::Remote, "fork/main"),
        ];
        assert_eq!(marked(&other, Some("origin/main")), [label("main", false)]);
        // Nothing is followed, or what is followed is not on this commit.
        assert_eq!(
            marked(&behind, None),
            [label("main", false), label("v1", false)]
        );
        assert_eq!(
            marked(&behind, Some("origin/topic"))[0],
            label("main", false)
        );
        assert!(marked(&[], Some("origin/main")).is_empty());
        // A name without a remote in it, or an empty one, marks nothing.
        assert_eq!(
            marked(&level, Some("main")),
            [label("main", false), label("topic", false)]
        );
        assert_eq!(marked(&level, Some(""))[0], label("main", false));
    }

    #[test]
    fn a_labels_menu_has_its_groups_for_each_kind_of_label() {
        use super::{LabelEntry as E, LabelFacts, label_groups};
        let labels = with_worktrees(
            ref_labels(&[
                badge(BadgeKind::Head, "main"),
                badge(BadgeKind::Local, "topic"),
                badge(BadgeKind::Remote, "origin/other"),
                badge(BadgeKind::Tag, "v1"),
            ]),
            &[WorktreeMark {
                path: "/work/detached".to_owned(),
                head: oid(1),
                branch: None,
            }],
            oid(1),
        );
        let facts = |follows: bool, lightweight: bool| LabelFacts {
            follows,
            at_head: false,
            lightweight,
            seen: true,
            pulls: false,
        };
        // The checked-out branch: nothing that combines it with itself
        // or checks it out; what rewrites its last commit.
        assert_eq!(
            label_groups(&labels[0], facts(true, false)),
            [
                vec![E::Pull, E::Push, E::SetUpstream],
                vec![E::Checkout],
                vec![E::Worktree],
                vec![E::NewBranch, E::Reword, E::Revert],
                vec![E::Drop, E::MoveDown],
                vec![E::ApplyPatch, E::Rename, E::Delete],
                vec![E::CopyName, E::CopyId, E::SavePatch],
                vec![E::Seen],
                vec![E::Compare],
                vec![E::NewTag],
            ]
        );
        // Another local branch that follows one: fast-forward after the
        // rest of its group; without an upstream no pull either.
        let made = vec![
            E::NewBranch,
            E::CherryPick,
            E::Reset,
            E::Revert,
            E::CherryPickStaged,
        ];
        let topic = label_groups(&labels[1], facts(true, false));
        assert_eq!(
            topic,
            [
                vec![E::Pull, E::Push, E::SetUpstream, E::FastForward],
                vec![E::Combine],
                vec![E::Checkout],
                vec![E::Worktree],
                made.clone(),
                vec![E::Rename, E::Delete],
                vec![E::CopyName, E::CopyId, E::SavePatch],
                vec![E::Seen],
                vec![E::Compare],
                vec![E::NewTag],
            ]
        );
        assert_eq!(
            label_groups(&labels[1], facts(false, false))[0],
            [E::Push, E::SetUpstream]
        );
        // A branch only on a remote: no pushing or following, deleted on
        // its remote.
        assert_eq!(
            label_groups(&labels[2], facts(false, false)),
            [
                vec![E::Combine],
                vec![E::Checkout],
                vec![E::Worktree],
                made.clone(),
                vec![E::DeleteOnRemote],
                vec![E::CopyName, E::CopyId, E::SavePatch],
                vec![E::Seen],
                vec![E::Compare],
                vec![E::NewTag],
            ]
        );
        // A tag: its own entries; one without a message can be given one.
        let tag = |lightweight| label_groups(&labels[3], facts(false, lightweight));
        assert_eq!(
            tag(false),
            [
                vec![E::Combine],
                vec![E::Checkout],
                vec![E::Worktree],
                made,
                vec![E::TagEntries],
                vec![E::CopyName],
                vec![E::Seen],
            ]
        );
        assert_eq!(tag(true).last(), Some(&vec![E::Annotate]));
        // What the graph does not know it cannot change: no visibility.
        assert!(
            !label_groups(&labels[1], LabelFacts::default())
                .iter()
                .any(|group| group.contains(&E::Seen))
        );
        // The folder of a detached worktree names no reference: only its
        // worktree; a detached HEAD has nothing of its own.
        assert_eq!(labels[4].name, "detached");
        assert_eq!(
            label_groups(&labels[4], facts(false, false)),
            [vec![E::OpenWorktree]]
        );
        let detached = ref_labels(&[badge(BadgeKind::Head, "HEAD")]);
        assert!(label_groups(&detached[0], facts(false, false)).is_empty());
    }

    #[test]
    fn a_label_names_the_reference_whose_place_in_the_graph_its_menu_changes() {
        use super::{LabelSeen, Pin, label_ref};
        let labels = ref_labels(&[
            badge(BadgeKind::Head, "main"),
            badge(BadgeKind::Local, "topic"),
            badge(BadgeKind::Remote, "origin/topic"),
            badge(BadgeKind::Remote, "origin/other"),
            badge(BadgeKind::Tag, "v1"),
        ]);
        let named: Vec<(String, Option<String>)> = labels
            .iter()
            .map(|label| (label.name.clone(), label_ref(label)))
            .collect();
        let full = |name: &str| {
            named
                .iter()
                .find(|(label, _)| label == name)
                .unwrap_or_else(|| panic!("no label {name}: {named:?}"))
                .1
                .clone()
        };
        assert_eq!(full("main").as_deref(), Some("refs/heads/main"));
        // A local branch and the remote one that shares its label: the
        // local one.
        assert_eq!(full("topic").as_deref(), Some("refs/heads/topic"));
        // A branch only on a remote is labelled by its own name and is
        // the remote's branch.
        assert_eq!(full("other").as_deref(), Some("refs/remotes/origin/other"));
        assert_eq!(full("v1").as_deref(), Some("refs/tags/v1"));
        // A stash and a detached HEAD have no place of their own to change.
        let stash = super::RefLabel {
            name: "stash@{0}".to_owned(),
            head: false,
            local: false,
            remotes: Vec::new(),
            tag: false,
            stash: true,
            worktree: None,
            upstream: false,
        };
        assert_eq!(label_ref(&stash), None);
        let detached = ref_labels(&[badge(BadgeKind::Head, "HEAD")]);
        assert_eq!(label_ref(&detached[0]), None);

        let label = |name: &str| labels.iter().find(|label| label.name == name).unwrap();
        let names =
            |list: &[&str]| -> Vec<String> { list.iter().map(|n| (*n).to_owned()).collect() };
        // Hidden, shown alone and pinned are read off what the repository
        // keeps.
        let seen = LabelSeen::of(
            label("topic"),
            &names(&["refs/heads/topic"]),
            &names(&["refs/heads/main"]),
            &names(&["refs/heads/topic"]),
        )
        .unwrap();
        assert_eq!(seen.full_name, "refs/heads/topic");
        assert!(seen.hidden && !seen.soloed);
        assert_eq!(seen.pin, Pin::Pinned);
        // The only one pinned has no lane beside it to move to; of two,
        // each can move towards the other.
        assert_eq!(seen.moves, (false, false));
        let two = names(&["refs/heads/main", "refs/heads/topic"]);
        assert_eq!(
            LabelSeen::of(label("topic"), &[], &[], &two).unwrap().moves,
            (true, false)
        );
        assert_eq!(
            LabelSeen::of(label("main"), &[], &[], &two).unwrap().moves,
            (false, true)
        );
        // A branch that is not pinned can be, while there is room.
        let free = LabelSeen::of(label("other"), &[], &[], &names(&["refs/heads/topic"]));
        assert_eq!(free.unwrap().pin, Pin::Free);
        let full_up: Vec<String> = (0..crate::prefs::MAX_PINNED)
            .map(|n| format!("refs/heads/b{n}"))
            .collect();
        assert_eq!(
            LabelSeen::of(label("main"), &[], &[], &full_up)
                .unwrap()
                .pin,
            Pin::Full
        );
        // A tag has no line of its own to pin; it is hidden and shown alone
        // like any other.
        let tag = LabelSeen::of(label("v1"), &[], &names(&["refs/tags/v1"]), &[]).unwrap();
        assert_eq!(tag.pin, Pin::NotOffered);
        assert!(tag.soloed && !tag.hidden);
        assert_eq!(LabelSeen::of(&stash, &[], &[], &[]), None);
    }

    #[test]
    fn the_nearest_branch_is_checked_out_when_it_can_be() {
        use super::{Ghost, ghost_of};
        let ghost = |kind: BadgeKind, name: &str, marks: &[WorktreeMark]| {
            ghost_of(&badge(kind, name), marks, oid(1))
        };
        let offers = |name: &str| Ghost {
            name: name.to_owned(),
            target: Some(name.to_owned()),
        };
        assert_eq!(ghost(BadgeKind::Local, "topic", &[]), offers("topic"));
        assert_eq!(
            ghost(BadgeKind::Remote, "origin/topic", &[]),
            offers("origin/topic")
        );
        assert_eq!(ghost(BadgeKind::Tag, "v1", &[]), offers("v1"));
        // The checked-out branch is where one is already, and a branch
        // another worktree has cannot be had here as well.
        assert_eq!(ghost(BadgeKind::Head, "main", &[]).target, None);
        let held = [WorktreeMark {
            path: "/work/topic".to_owned(),
            head: oid(1),
            branch: Some("topic".to_owned()),
        }];
        let taken = ghost(BadgeKind::Local, "topic", &held);
        assert_eq!((taken.name.as_str(), taken.target), ("topic", None));
    }

    #[test]
    fn a_jump_goes_to_the_next_row_with_a_branch() {
        use super::next_of;
        let rows = [2usize, 5, 9, 40];
        assert_eq!(next_of(&rows, None, true), Some(2));
        assert_eq!(next_of(&rows, None, false), None);
        assert_eq!(next_of(&rows, Some(0), true), Some(2));
        assert_eq!(next_of(&rows, Some(2), true), Some(5));
        assert_eq!(next_of(&rows, Some(6), true), Some(9));
        assert_eq!(next_of(&rows, Some(40), true), None);
        assert_eq!(next_of(&rows, Some(usize::MAX), true), None);
        assert_eq!(next_of(&rows, Some(40), false), Some(9));
        assert_eq!(next_of(&rows, Some(6), false), Some(5));
        assert_eq!(next_of(&rows, Some(2), false), None);
        assert_eq!(next_of(&rows, Some(0), false), None);
        assert_eq!(next_of(&rows, Some(usize::MAX), false), Some(40));
        assert_eq!(next_of(&[], Some(3), true), None);
        assert_eq!(next_of(&[], None, true), None);
    }

    #[test]
    fn a_step_among_what_was_found_goes_to_the_next_of_those() {
        use super::step_among;
        let rows = [3usize, 8, 20, 21];
        // On one of them: the next and the one before, and no further
        // than the ends.
        assert_eq!(step_among(&rows, Some(8), 1), Some(20));
        assert_eq!(step_among(&rows, Some(8), -1), Some(3));
        assert_eq!(step_among(&rows, Some(3), -1), Some(3));
        assert_eq!(step_among(&rows, Some(21), 1), Some(21));
        assert_eq!(step_among(&rows, Some(8), 20), Some(21));
        assert_eq!(step_among(&rows, Some(8), isize::MIN / 2), Some(3));
        assert_eq!(step_among(&rows, Some(8), isize::MAX), Some(21));
        assert_eq!(step_among(&rows, Some(8), isize::MIN), Some(3));
        // From a row that is not among them: its neighbour on that side.
        assert_eq!(step_among(&rows, Some(10), 1), Some(20));
        assert_eq!(step_among(&rows, Some(10), -1), Some(8));
        assert_eq!(step_among(&rows, Some(0), -1), Some(3));
        assert_eq!(step_among(&rows, Some(0), 1), Some(3));
        assert_eq!(step_among(&rows, Some(99), 1), Some(21));
        assert_eq!(step_among(&rows, Some(99), -1), Some(21));
        assert_eq!(step_among(&rows, Some(10), 2), Some(21));
        // From nowhere: the first. Among none: nowhere.
        assert_eq!(step_among(&rows, None, 5), Some(3));
        assert_eq!(step_among(&[], Some(3), 1), None);
        assert_eq!(step_among(&[], None, -1), None);
    }

    #[test]
    fn a_stashs_line_is_dotted_from_it_down_to_the_commit_it_was_made_on() {
        use super::stash_slots_at;
        // A stash on row 1 in lane 2 made on row 4; another on row 3 in
        // lane 1 made on row 5.
        let lanes = [(1, 2, 4), (3, 1, 5)];
        assert!(stash_slots_at(&lanes, 0).is_empty());
        assert!(stash_slots_at(&lanes, 1).is_empty(), "not its own row");
        assert_eq!(stash_slots_at(&lanes, 2), [2]);
        assert_eq!(stash_slots_at(&lanes, 4), [2, 1], "it ends in the base");
        assert_eq!(stash_slots_at(&lanes, 5), [1]);
        assert!(stash_slots_at(&lanes, 6).is_empty());
        assert!(stash_slots_at(&[], usize::MAX).is_empty());
    }

    #[test]
    fn the_field_of_uncommitted_work_grows_with_the_summary_up_to_a_limit() {
        use super::wip_field_width;
        assert_eq!(wip_field_width(""), 96.0);
        assert_eq!(wip_field_width("   "), 96.0);
        assert_eq!(wip_field_width("fix"), 96.0);
        let longer = wip_field_width("fix: the crash when the list is empty");
        assert!(longer > 96.0, "{longer}");
        assert_eq!(wip_field_width("fix: crashes"), 18.0 + 7.5 * 12.0);
        assert_eq!(wip_field_width(&"x".repeat(10_000)), 176.0);
    }

    #[test]
    fn a_message_is_said_whole_with_its_lines_and_cut_when_long() {
        use super::{MESSAGE_TIP_LETTERS, MESSAGE_TIP_LINES, message_said};
        assert_eq!(message_said("Fix it", ""), "Fix it");
        assert_eq!(message_said("Fix it", "  \n "), "Fix it");
        assert_eq!(
            message_said("Fix it", "First line\nsecond line\n\nAnother paragraph\n"),
            "Fix it\n\nFirst line\nsecond line\n\nAnother paragraph"
        );
        assert_eq!(message_said("Fix it", "a\r\nb"), "Fix it\n\na\nb");
        // Many lines: as many as there is room for, then an ellipsis.
        let many: Vec<String> = (0..100).map(|n| format!("line {n}")).collect();
        let said = message_said("Many", &many.join("\n"));
        let lines: Vec<&str> = said.lines().collect();
        assert_eq!(lines.len(), MESSAGE_TIP_LINES + 1, "{said}");
        assert_eq!(lines.last(), Some(&"\u{2026}"));
        // One very long line fills the room it has and is cut.
        let long = "word ".repeat(10_000);
        let said = message_said("Long", &long);
        assert!(said.ends_with('\u{2026}'), "{said}");
        let letters = said.chars().count();
        assert!(
            letters <= MESSAGE_TIP_LINES * MESSAGE_TIP_LETTERS + 8,
            "{letters} letters"
        );
        // Nothing at all, or letters of every kind, is no fault.
        assert_eq!(message_said("", ""), "");
        let _ = message_said("\u{1F600}", &"\u{0301}".repeat(5000));
    }

    #[test]
    fn a_circle_names_its_author_and_another_committer() {
        use super::circle_said;
        use bitbraid_git::Signature;
        let who = |name: &str, email: &str| Signature {
            name: name.to_owned(),
            email: email.to_owned(),
            seconds: 0,
            offset: 0,
        };
        let ada = who("Ada Example", "ada@example.invalid");
        assert_eq!(
            circle_said(&ada, &ada),
            ["Ada Example <ada@example.invalid>"]
        );
        assert_eq!(
            circle_said(&ada, &who("Cy Applier", "cy@example.invalid")),
            [
                "Ada Example <ada@example.invalid>",
                "Committed by Cy Applier <cy@example.invalid>"
            ]
        );
        // Another address is someone else too; spaces around are not.
        assert_eq!(
            circle_said(&ada, &who(" Ada Example ", "ada@work.invalid")).len(),
            2
        );
        assert_eq!(
            circle_said(&ada, &who("Ada Example ", " ada@example.invalid")).len(),
            1
        );
        // What there is of a name and an address.
        assert_eq!(circle_said(&who("Ada", ""), &who("Ada", "")), ["Ada"]);
        assert_eq!(circle_said(&who("", "a@b"), &who("", "a@b")), ["<a@b>"]);
        assert_eq!(circle_said(&who("", ""), &who("", "")), [""]);
    }

    #[test]
    fn a_node_out_of_the_column_is_drawn_at_its_nearer_edge() {
        use super::{LaneGeometry, pinned_node};
        let geometry = LaneGeometry {
            left: px(100.0),
            width: 172.0,
            scroll_x: 0.0,
        };
        // In the column: where its lane is.
        let (x, pinned) = pinned_node(&geometry, 2);
        assert!(!pinned);
        assert_eq!(x, geometry.x(2));
        // Beyond its right edge: at the edge, inside.
        let (x, pinned) = pinned_node(&geometry, 30);
        assert!(pinned);
        assert!(f32::from(x) < 100.0 + 172.0 && f32::from(x) > 100.0 + 172.0 - 30.0);
        // Scrolled past on the left: at the left edge.
        let scrolled = LaneGeometry {
            scroll_x: 400.0,
            ..geometry
        };
        let (x, pinned) = pinned_node(&scrolled, 0);
        assert!(pinned);
        assert!(f32::from(x) > 100.0 && f32::from(x) < 130.0);
        // A column too narrow for a whole node: held at the left.
        let narrow = LaneGeometry {
            width: 10.0,
            ..geometry
        };
        let (x, pinned) = pinned_node(&narrow, 30);
        assert!(pinned);
        assert_eq!(f32::from(x), 100.0 + super::NODE_RING + super::PIN_MARGIN);
        // No number for a width or a scroll: nothing to hold to.
        let lost = LaneGeometry {
            width: f32::NAN,
            scroll_x: f32::INFINITY,
            ..geometry
        };
        let _ = pinned_node(&lost, 3);
    }

    /// A node goes from its lane to the column's edge without a jump and
    /// is never cut off: while the column is made narrower a quarter of a
    /// point at a time, or the lanes are scrolled sideways as finely, every
    /// lane's node stays whole in the column, stays at its lane while the
    /// whole of it fits there, and moves by no more than the column's edge
    /// or the lanes did.
    #[test]
    fn a_node_is_held_at_the_edge_from_the_moment_it_would_be_cut() {
        use super::{BAND_BAR, LaneGeometry, NODE_RING, PIN_MARGIN, pinned_node};
        let step = 0.25f32;
        let left = 100.0f32;
        let whole = |x: f32, width: f32, what: &str| {
            assert!(
                x - NODE_RING >= -0.001 && x + NODE_RING <= width - BAND_BAR + 0.001,
                "{what}: the node at {x} is cut in a column {width} wide"
            );
        };
        for lane in 0..12u16 {
            // Narrower, from 400 points to 40.
            let mut last: Option<(f32, f32)> = None;
            let mut width = 400.0f32;
            while width >= 40.0 {
                let geometry = LaneGeometry {
                    left: px(left),
                    width,
                    scroll_x: 0.0,
                };
                let (x, held) = pinned_node(&geometry, lane);
                let x = f32::from(x) - left;
                let what = format!("lane {lane}, a column {width} wide");
                whole(x, width, &what);
                let at_lane = geometry.offset(lane);
                if at_lane + NODE_RING <= width - BAND_BAR - PIN_MARGIN {
                    assert!(
                        !held && (x - at_lane).abs() < 0.001,
                        "{what}: it fits at its lane"
                    );
                } else {
                    assert!(held, "{what}: held at the edge");
                }
                if let Some((was_width, was_x)) = last {
                    assert!(
                        (x - was_x).abs() <= (was_width - width) + 0.001,
                        "{what}: the node went from {was_x} to {x}"
                    );
                }
                last = Some((width, x));
                width -= step;
            }
            // Scrolled sideways in a column 120 wide, to the right as far
            // as past the lane.
            let mut last: Option<(f32, f32)> = None;
            let mut scroll = 0.0f32;
            while scroll <= 400.0 {
                let geometry = LaneGeometry {
                    left: px(left),
                    width: 120.0,
                    scroll_x: scroll,
                };
                let (x, _) = pinned_node(&geometry, lane);
                let x = f32::from(x) - left;
                let what = format!("lane {lane}, scrolled {scroll}");
                whole(x, 120.0, &what);
                if let Some((was_scroll, was_x)) = last {
                    assert!(
                        (x - was_x).abs() <= (scroll - was_scroll) + 0.001,
                        "{what}: the node went from {was_x} to {x}"
                    );
                }
                last = Some((scroll, x));
                scroll += step;
            }
        }
    }

    #[test]
    fn the_search_says_which_of_what_it_found_is_selected() {
        use super::match_count;
        assert_eq!(match_count(Some(0), 6), "1 of 6");
        assert_eq!(match_count(Some(5), 6), "6 of 6");
        assert_eq!(match_count(None, 6), "6 results");
        assert_eq!(match_count(None, 0), "0 results");
        assert_eq!(match_count(None, 1), "1 result");
        assert_eq!(match_count(Some(9), 6), "6 results", "not one of them");
    }

    #[test]
    fn a_stash_is_called_by_its_message() {
        assert_eq!(stash_summary("On main: an idea"), "an idea");
        assert_eq!(
            stash_summary("WIP on topic/x: 1a2b3c4 subject"),
            "1a2b3c4 subject"
        );
        assert_eq!(stash_summary("On main: "), "On main: ");
        assert_eq!(
            stash_summary("Onwards: nothing to cut"),
            "Onwards: nothing to cut"
        );
        assert_eq!(stash_summary(""), "");
    }
}
