//! The commit details panel: over it, while the working tree has changes,
//! a strip that leads to them (while an operation is stopped on
//! conflicts, it says how many files are in conflict); the id with the
//! signature's mark; the message in a frame of its own whose height is
//! dragged; who wrote it and when with the parents beside, who committed
//! it and who wrote it too; how many files changed in which way; the order
//! and the Path / Tree switch; and the files, which scroll by themselves,
//! each with a menu of what can be done with it (`file_actions`). Several
//! commits selected are listed as cards over the files they change.
//!
//! In a panel too low for all of it (a small window, a large zoom) the
//! message gives way, down to a line of its summary, and the cards of
//! several commits down to one, so that the files keep two rows; lower
//! still, the panel scrolls, and a wheel over the files or the message
//! moves them before the panel.
//!
//! Sources: git-interpret-trailers(1) for the co-authors a message names;
//! gpui-kit documentation (virtual list, scroll mask); the CSS flexible box
//! layout's resolution of flexible lengths, which the toolkit's layout
//! follows.

use std::{cell::Cell, collections::BTreeSet, rc::Rc};

use bitbraid_git::{CommitInfo, Oid};
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, VirtualListScrollHandle,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    menu::ContextMenuExt,
    scroll::{ScrollableElement, ScrollbarAxis},
    tooltip::Tooltip,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use crate::{
    prefs::Prefs,
    session::{Badge, BadgeKind, DIFF_GUARD_BYTES, Reply, Request, SessionHandle},
    views::{
        absolute_time,
        avatar::avatar,
        changes, chosen,
        drag_paths::{self, DraggedPaths},
        file_actions::{self, FileAction, RunFileAction},
        file_tree,
        graph::Comparison,
        signatures::{Level, Verification},
    },
};

/// Events from the details panel.
#[derive(Debug, Clone)]
pub enum DetailsEvent {
    /// Select and scroll to this commit.
    Reveal(Oid),
    /// Open this file of the commit in the diff viewer.
    OpenFile {
        /// Commit.
        id: Oid,
        /// Its diff.
        diff: Box<bitbraid_git::FileDiff>,
    },
    /// Open this file as it differs between two commits.
    OpenRange {
        /// The older end; none for the empty tree.
        old: Option<Oid>,
        /// The newer end.
        new: Oid,
        /// Its diff.
        diff: Box<bitbraid_git::FileDiff>,
    },
    /// Something was picked from a file's menu.
    File(FileAction),
    /// Ask whether the key this commit was signed with is to count: who
    /// committed it is whom the key would be allowed for.
    AllowSigner {
        /// The commit.
        id: Oid,
        /// The address of whoever committed it.
        address: String,
    },
    /// Show the uncommitted work instead.
    ShowUncommitted,
    /// Show this issue in the middle: an issue key of the message was
    /// clicked.
    OpenIssue {
        /// Its repository.
        link: bitbraid_integrations::model::RepoLink,
        /// Its number.
        number: u64,
    },
    /// Open this file as it differs between a commit and the working
    /// tree.
    OpenAgainstWorktree {
        /// The commit.
        old: Oid,
        /// Its diff.
        diff: Box<bitbraid_git::FileDiff>,
    },
}

/// Where the files that are listed come from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Listed {
    /// One commit against its parent.
    Commit(Oid),
    /// Two commits against each other.
    Range { old: Option<Oid>, new: Oid },
    /// A commit against the working tree.
    Worktree(Oid),
}

/// Several commits selected together.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Many {
    /// The commits, newest first.
    ids: Vec<Oid>,
    /// The two ends their files are listed between, when there are such.
    comparison: Option<Comparison>,
}

pub struct DetailsView {
    session: SessionHandle,
    id: Option<Oid>,
    /// Set instead of `id` while several commits are selected.
    many: Option<Many>,
    commit: Option<CommitInfo>,
    message: Option<String>,
    /// What git makes of the commit's signature, when it has one.
    verification: Option<Verification>,
    badges: Vec<Badge>,
    files: Vec<bitbraid_git::FileDiff>,
    /// Why the files could not be listed, said where they would be
    /// rather than an empty list.
    files_error: Option<String>,
    selected_file: Option<String>,
    /// The files listed are what differs between the commit and the
    /// working tree, not what the commit changed; and whether that list
    /// is still being worked out.
    against_worktree: bool,
    comparing: bool,
    /// How many files the working tree has changes in, for the strip over
    /// the panel.
    uncommitted: usize,
    /// How many files an operation stopped on is in conflict over, which
    /// the strip says instead.
    conflicts: usize,
    /// The commits of a selection of several, for their cards.
    cards: Vec<CommitInfo>,
    /// The folders of the tree that are open.
    open_folders: BTreeSet<String>,
    /// How much taller than its least the message's frame was dragged,
    /// and how tall it was drawn the last time.
    message_extra: f32,
    message_drawn: Rc<Cell<f32>>,
    /// Where the files' list begins, and where the panel ends, as drawn:
    /// how much the message may grow.
    files_room: Rc<Cell<(f32, f32)>>,
    files_scroll: VirtualListScrollHandle,
    cards_scroll: ScrollHandle,
    /// The message's text, which scrolls inside its frame.
    message_scroll: ScrollHandle,
    focus_handle: FocusHandle,
    /// How the checkout stands, for the menus of the folders.
    sparse: Option<super::sparse::SparseMark>,
}

impl EventEmitter<DetailsEvent> for DetailsView {}

impl DetailsView {
    pub fn new(session: SessionHandle, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            session,
            id: None,
            many: None,
            commit: None,
            message: None,
            verification: None,
            badges: Vec::new(),
            files: Vec::new(),
            files_error: None,
            selected_file: None,
            against_worktree: false,
            comparing: false,
            uncommitted: 0,
            conflicts: 0,
            cards: Vec::new(),
            open_folders: BTreeSet::new(),
            message_extra: 0.,
            message_drawn: Rc::new(Cell::new(0.)),
            files_room: Rc::new(Cell::new((0., f32::INFINITY))),
            files_scroll: VirtualListScrollHandle::new(),
            cards_scroll: ScrollHandle::new(),
            message_scroll: ScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            sparse: None,
        }
    }

    /// Whether a commit, or several, is shown.
    pub fn shows_something(&self) -> bool {
        self.id.is_some() || self.many.is_some()
    }

    /// No file of the list is marked as the one shown any more.
    pub fn unmark_file(&mut self, cx: &mut Context<Self>) {
        if self.selected_file.take().is_some() {
            cx.notify();
        }
    }

    /// How many files the working tree has changes in.
    pub fn set_uncommitted(&mut self, files: usize, cx: &mut Context<Self>) {
        if self.uncommitted != files {
            self.uncommitted = files;
            cx.notify();
        }
    }

    /// How many files the operation in progress is in conflict over.
    pub fn set_conflicts(&mut self, files: usize, cx: &mut Context<Self>) {
        if self.conflicts != files {
            self.conflicts = files;
            cx.notify();
        }
    }

    /// Show a selection of several commits: how many, and the files that
    /// differ between the two ends they can be compared as, if any. Those
    /// are fetched in a partial clone only when the user made the
    /// selection (`asked`): a selection that came back by itself lists
    /// what is here, or says that the rest was left on the remote.
    pub fn show_many(
        &mut self,
        ids: Vec<Oid>,
        comparison: Option<Comparison>,
        asked: bool,
        cx: &mut Context<Self>,
    ) {
        let many = Many { ids, comparison };
        if self.many.as_ref() == Some(&many) {
            return;
        }
        self.id = None;
        self.many = Some(many.clone());
        // Several commits are never compared with the working tree; one
        // that was must not leave that heading over their files.
        self.against_worktree = false;
        self.comparing = false;
        self.badges.clear();
        self.commit = None;
        self.message = None;
        self.files.clear();
        self.files_error = None;
        self.selected_file = None;
        self.cards.clear();
        self.open_folders.clear();
        cx.notify();
        // Their cards: what each says, who wrote it, and when.
        let ids = many.ids.clone();
        let wanted = many;
        self.session
            .call(Request::Details(ids), cx, move |this, reply, cx| {
                if this.many.as_ref() != Some(&wanted) {
                    return;
                }
                if let Reply::Details(details) = reply {
                    this.cards = details;
                    cx.notify();
                }
            });
        self.list_files(asked, cx);
    }

    /// Ask for the files of what is shown: one commit's against its
    /// parent, or those between the ends of several. In a partial clone
    /// what they need is fetched only when `asked`.
    fn list_files(&mut self, asked: bool, cx: &mut Context<Self>) {
        if let Some(many) = self.many.clone() {
            let Some(comparison) = many.comparison else {
                return;
            };
            let request = Request::RangeFiles {
                old: comparison.old,
                new: comparison.new,
            };
            self.session
                .call_as(asked, request, cx, move |this, reply, cx| {
                    if this.many.as_ref() != Some(&many) {
                        return;
                    }
                    this.take_files(reply, cx);
                });
            return;
        }
        let Some(id) = self.id else { return };
        self.session.call_as(
            asked,
            Request::CommitFiles(id),
            cx,
            move |this, reply, cx| {
                if this.id != Some(id) || this.against_worktree {
                    return;
                }
                this.take_files(reply, cx);
            },
        );
    }

    /// Why the files could not be listed, when they could not.
    pub fn files_error(&self) -> Option<&str> {
        self.files_error.as_deref()
    }

    /// How many files are listed.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Whether the files could not be listed because a partial clone left
    /// what they need on its remote, and nobody had asked for it yet.
    pub fn files_left_on_remote(&self) -> bool {
        self.files.is_empty()
            && self
                .files_error
                .as_deref()
                .is_some_and(|error| error.contains(bitbraid_git::promisor::left_on_remote()))
    }

    /// Fetch what the files of what is shown need, which a partial clone
    /// left on its remote: the user asks for it.
    pub fn fetch_files(&mut self, cx: &mut Context<Self>) {
        if !self.files_left_on_remote() {
            return;
        }
        self.files_error = None;
        cx.notify();
        self.list_files(true, cx);
    }

    /// Show a commit (or nothing). Its files are fetched in a partial
    /// clone only when the user selected it (`asked`), as for
    /// [`Self::show_many`].
    pub fn show(
        &mut self,
        id: Option<Oid>,
        badges: Vec<Badge>,
        asked: bool,
        cx: &mut Context<Self>,
    ) {
        self.many = None;
        self.id = id;
        self.badges = badges;
        self.commit = None;
        self.message = None;
        self.verification = None;
        self.files.clear();
        self.files_error = None;
        self.selected_file = None;
        self.against_worktree = false;
        self.comparing = false;
        self.cards.clear();
        self.open_folders.clear();
        cx.notify();
        let Some(id) = id else { return };
        self.verify(id, cx);
        self.list_files(asked, cx);
        self.session
            .call(Request::Details(vec![id]), cx, move |this, reply, cx| {
                if this.id != Some(id) {
                    return;
                }
                if let Reply::Details(mut details) = reply {
                    this.commit = details.pop();
                    cx.notify();
                }
            });
        self.session
            .call(Request::Message(id), cx, move |this, reply, cx| {
                if this.id != Some(id) {
                    return;
                }
                if let Reply::Message(message) = reply {
                    this.message = Some(message);
                    cx.notify();
                }
            });
    }

    /// The files a list of them was asked for answered with, or why
    /// there are none to be had, which is said instead of nothing.
    fn take_files(&mut self, reply: Reply, cx: &mut Context<Self>) {
        match reply {
            Reply::CommitFiles(files) => {
                self.files = files;
                self.files_error = None;
            }
            Reply::Error(error) => {
                self.files.clear();
                self.files_error = Some(error);
            }
            _ => return,
        }
        cx.notify();
    }

    /// Whether what is listed is what differs from the shown commit to the
    /// working tree, which the heading over the files then says.
    pub fn compared_with_worktree(&self) -> bool {
        self.against_worktree
    }

    /// Whether one commit is shown, which can be compared with the
    /// working tree.
    pub fn has_commit(&self) -> bool {
        self.id.is_some()
    }

    /// List what differs between the commit that is shown and the
    /// working tree, or go back to what the commit itself changed. The
    /// user asked for either: in a partial clone what they need is fetched.
    pub fn toggle_against_worktree(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.id else { return };
        self.against_worktree = !self.against_worktree;
        self.files.clear();
        self.files_error = None;
        self.selected_file = None;
        self.comparing = self.against_worktree;
        cx.notify();
        if !self.against_worktree {
            self.list_files(true, cx);
            return;
        }
        let request = Request::Command {
            id: "repo.diff".to_owned(),
            params: serde_json::json!({
                "old": id.to_hex(),
                "new": "worktree",
                "max_bytes": DIFF_GUARD_BYTES,
            }),
        };
        self.session
            .call_asked(request, cx, move |this, reply, cx| {
                if this.id != Some(id) || !this.against_worktree {
                    return;
                }
                if let Reply::Command(value) = reply {
                    this.files = serde_json::from_value(value).unwrap_or_default();
                }
                this.comparing = false;
                cx.notify();
            });
    }

    /// Ask what git makes of the commit's signature; a commit without
    /// one costs nothing to ask about.
    fn verify(&mut self, id: Oid, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "sign.verify".to_owned(),
            params: serde_json::json!({ "ids": [id] }),
        };
        self.session.call(request, cx, move |this, reply, cx| {
            if this.id != Some(id) {
                return;
            }
            if let Reply::Command(value) = reply {
                this.verification = value
                    .as_array()
                    .and_then(|answers| answers.first())
                    .and_then(Verification::from_json);
                cx.notify();
            }
        });
    }

    /// How the checkout stands.
    pub fn set_sparse(&mut self, mark: Option<super::sparse::SparseMark>, cx: &mut Context<Self>) {
        if self.sparse != mark {
            self.sparse = mark;
            cx.notify();
        }
    }

    /// Ask again what the shown commit's signature is worth: who may sign
    /// has changed.
    pub fn verify_again(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.id.filter(|_| self.many.is_none()) {
            self.verify(id, cx);
        }
    }

    /// What is said of the shown commit's signature: git's word for its
    /// state and who signed, for the off-screen tests.
    pub fn signature(&self) -> Option<(String, Option<String>)> {
        self.verification
            .as_ref()
            .map(|verification| (verification.state.clone(), verification.signer.clone()))
    }
}

/// One row of a commit's files.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FileRow {
    /// The first row of the tree: opens every folder, or closes them all.
    Expand { all_open: bool },
    /// A folder of the tree.
    Folder {
        path: String,
        name: String,
        depth: usize,
        open: bool,
        counts: file_tree::Counts,
    },
    /// A file, by its place among the files; with the folder apart in the
    /// list of paths.
    File {
        file: usize,
        depth: usize,
        folder: Option<String>,
        name: String,
    },
}

/// Height of a commit's file, and of a folder of its tree.
const FILE_HEIGHT: f32 = 32.;
const FOLDER_HEIGHT: f32 = 25.;
/// The message's frame: how high it is undragged, and how low it can be
/// dragged.
const MESSAGE_HEIGHT: f32 = 133.;
const MESSAGE_LEAST: f32 = 103.;
/// How low the frame gives way in a panel too low for everything: a line
/// of the summary between the frame's margins.
const MESSAGE_SQUEEZED: f32 = 56.;
/// The box of the cards of several commits: as high as they are, up to
/// six or so, and down to one in a panel too low for everything.
const CARDS_MOST: f32 = 270.;
const CARDS_LEAST: f32 = 54.;
/// What the dragged frame leaves under it at the least: the people, the
/// counts, the row of controls and two and a half files.
const UNDER_MESSAGE: f32 = 230.;

/// What happened to a file, as a folder counts it.
fn kind_of(kind: bitbraid_git::diff::FileChangeKind) -> file_tree::Kind {
    use bitbraid_git::diff::FileChangeKind as Change;
    match kind {
        Change::Added => file_tree::Kind::Added,
        Change::Deleted => file_tree::Kind::Deleted,
        Change::Renamed | Change::Copied => file_tree::Kind::Renamed,
        Change::Modified | Change::TypeChanged => file_tree::Kind::Modified,
    }
}

/// The rows of the files: as paths or as a tree, in the order of the
/// lists, with the folders in `open` open.
fn file_rows(
    files: &[bitbraid_git::FileDiff],
    tree: bool,
    open: &BTreeSet<String>,
    reversed: bool,
) -> Vec<FileRow> {
    let paths: Vec<&str> = files.iter().map(|file| file.path.as_str()).collect();
    let order = file_tree::order(&paths, reversed);
    if !tree {
        return order
            .into_iter()
            .filter_map(|ix| {
                let file = files.get(ix)?;
                let (folder, name) = match file.path.rsplit_once('/') {
                    Some((folder, name)) => (Some(folder.to_owned()), name.to_owned()),
                    None => (None, file.path.clone()),
                };
                Some(FileRow::File {
                    file: ix,
                    depth: 0,
                    folder,
                    name,
                })
            })
            .collect();
    }
    let items: Vec<(usize, &str, file_tree::Kind)> = order
        .into_iter()
        .filter_map(|ix| {
            let file = files.get(ix)?;
            Some((ix, file.path.as_str(), kind_of(file.kind)))
        })
        .collect();
    let folders = file_tree::folders(paths.iter().copied());
    let mut rows = Vec::new();
    if !folders.is_empty() {
        rows.push(FileRow::Expand {
            all_open: folders.is_subset(open),
        });
    }
    for node in file_tree::tree(&items, &|path| open.contains(path)) {
        rows.push(match node {
            file_tree::Node::Folder {
                path,
                name,
                depth,
                open,
                counts,
            } => FileRow::Folder {
                path,
                name,
                depth,
                open,
                counts,
            },
            file_tree::Node::File { item, depth } => FileRow::File {
                file: item,
                depth,
                folder: None,
                name: files
                    .get(item)
                    .map(|file| {
                        file.path
                            .rsplit('/')
                            .next()
                            .unwrap_or(&file.path)
                            .to_owned()
                    })
                    .unwrap_or_default(),
            },
        });
    }
    rows
}

/// What a stash's message says without the words git puts before it
/// (`On main: `, `WIP on main: `).
fn stash_summary(summary: &str) -> &str {
    let rest = summary
        .strip_prefix("WIP on ")
        .or_else(|| summary.strip_prefix("On "));
    match rest.and_then(|rest| rest.split_once(": ")) {
        Some((branch, said)) if !branch.contains(' ') => said,
        _ => summary,
    }
}

/// The people a message names as having written it too: its
/// `Co-authored-by` trailers, by name and address.
fn co_authors(message: &str) -> Vec<(String, String)> {
    message
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            if !key.trim().eq_ignore_ascii_case("co-authored-by") {
                return None;
            }
            let value = value.trim();
            let (name, address) = match value.split_once('<') {
                Some((name, rest)) => (name.trim(), rest.trim_end_matches('>').trim()),
                None => (value, value),
            };
            (!address.is_empty()).then(|| (name.to_owned(), address.to_owned()))
        })
        .collect()
}

impl DetailsView {
    /// The rows of the files that are listed, as the preferences have them.
    fn rows(&self, cx: &App) -> Vec<FileRow> {
        let preferences = &cx.global::<Prefs>().preferences;
        file_rows(
            &self.files,
            preferences.changes_tree,
            &self.open_folders,
            preferences.changes_reversed,
        )
    }

    /// A person with their part in the commit and when: the picture, the
    /// name, and under it "authored" or "committed" with the date, which
    /// wraps rather than be cut.
    fn person(committed: bool, sig: &bitbraid_git::Signature, cx: &App) -> Div {
        let dark = cx.theme().is_dark();
        let time = absolute_time(sig.seconds, sig.offset);
        let part = if committed {
            say!("details-committed", time)
        } else {
            say!("details-authored", time)
        };
        h_flex()
            .min_w_0()
            .gap(px(8.))
            .items_center()
            .child(avatar(&sig.email, px(40.), dark, cx))
            .child(
                v_flex()
                    .min_w_0()
                    .child(div().text_base().truncate().child(sig.name.clone()))
                    .child(
                        div()
                            .text_sm()
                            .whitespace_normal()
                            .text_color(cx.theme().muted_foreground)
                            .child(part),
                    ),
            )
    }

    /// The id row: the signature's mark at the left, what is shown in the
    /// middle, and at the right what can be done about the key.
    fn id_row(&self, middle: AnyElement, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let mark = self.verification.clone().map(|verification| {
            let colour = match verification.level() {
                Level::Good => theme.success,
                Level::Doubtful => theme.warning,
                Level::Bad => theme.danger,
            };
            let said: SharedString = match &verification.key {
                Some(key) => say!(
                    "details-signature-with-key",
                    sentence = verification.sentence(),
                    key
                ),
                None => verification.sentence(),
            }
            .into();
            div()
                .id("signature-mark")
                .absolute()
                .left(px(10.))
                .top(px(9.))
                .child(Icon::new(verification.icon()).size_4().text_color(colour))
                .tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
        });
        // A key that holds and that nothing vouches for can be vouched
        // for from here. Not in a repository that is only read: its
        // settings may be what names the file.
        let allow = self
            .verification
            .as_ref()
            .filter(|verification| verification.key_can_be_allowed())
            .filter(|_| !self.session.summary.bare)
            .and(self.commit.as_ref())
            .map(|commit| (commit.id, commit.committer.email.clone()));
        h_flex()
            .relative()
            .w_full()
            .h(px(36.))
            .flex_none()
            .px(px(10.))
            .items_center()
            .justify_center()
            .border_b_1()
            .border_color(theme.border)
            .children(mark)
            .child(middle)
            .children(allow.map(|(id, address)| {
                div().absolute().right(px(10.)).top(px(6.)).child(
                    Button::new("allow-signer")
                        .ghost()
                        .xsmall()
                        .label(say!("details-allow-key"))
                        .tooltip(say!("details-allow-key-tooltip"))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(DetailsEvent::AllowSigner {
                                id,
                                address: address.clone(),
                            })
                        })),
                )
            }))
    }

    /// How many files changed in which way, the order and the Path / Tree
    /// switch, and under them the files, which scroll by themselves.
    fn render_files(&self, listed: Listed, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let tree = cx.global::<Prefs>().preferences.changes_tree;
        let mut out: Vec<AnyElement> = Vec::new();
        use bitbraid_git::diff::FileChangeKind as Change;
        let count =
            |wanted: fn(Change) -> bool| self.files.iter().filter(|file| wanted(file.kind)).count();
        let counts = [
            (
                count(|k| matches!(k, Change::Modified | Change::TypeChanged)),
                file_tree::Kind::Modified,
            ),
            (count(|k| k == Change::Added), file_tree::Kind::Added),
            (count(|k| k == Change::Deleted), file_tree::Kind::Deleted),
            (
                count(|k| matches!(k, Change::Renamed | Change::Copied)),
                file_tree::Kind::Renamed,
            ),
        ];
        // What the files are when they are not what the commit changed.
        let against = self.against_worktree;
        if against {
            out.push(
                div()
                    .mt(px(10.))
                    .px(px(12.))
                    .flex_none()
                    .text_xs()
                    .text_color(muted)
                    .whitespace_normal()
                    .child(match (self.comparing, self.files.is_empty()) {
                        (true, _) => say!("details-against-comparing"),
                        (false, true) => say!("details-against-same"),
                        (false, false) => say!("details-against-differs"),
                    })
                    .into_any_element(),
            );
        }
        out.push(
            h_flex()
                .mt(px(if against { 4. } else { 27. }))
                .h(px(24.))
                .flex_none()
                .pl(px(12.))
                .gap(px(12.))
                .items_center()
                .text_sm()
                .children(
                    counts
                        .into_iter()
                        .filter(|(n, ..)| *n > 0)
                        .map(|(count, kind)| {
                            let (icon, colour) = changes::tree_mark(kind, &theme);
                            let said = match kind {
                                file_tree::Kind::Modified => say!("details-count-modified", count),
                                file_tree::Kind::Added => say!("details-count-added", count),
                                file_tree::Kind::Deleted => say!("details-count-deleted", count),
                                file_tree::Kind::Renamed => say!("details-count-renamed", count),
                            };
                            h_flex()
                                .gap(px(4.))
                                .items_center()
                                .child(icon.size_3().text_color(colour))
                                .child(said)
                        }),
                )
                .into_any_element(),
        );
        // The order, the Path / Tree switch in the middle, and at the
        // right what the files are compared with.
        let has_commit = matches!(listed, Listed::Commit(_) | Listed::Worktree(_));
        out.push(
            h_flex()
                .relative()
                .mt(px(8.))
                .h(px(24.))
                .flex_none()
                .w_full()
                .justify_center()
                .child(
                    div().absolute().left(px(18.)).top_0().child(
                        changes::sort_button("details-sort", cx)
                            .on_click(cx.listener(|_, _, _, cx| changes::toggle_order(cx))),
                    ),
                )
                .child(
                    ButtonGroup::new("details-layout")
                        .outline()
                        .small()
                        .child(
                            chosen(
                                Button::new("details-path")
                                    .icon(Icon::new(gpui_kit::assets::IconName::List))
                                    .label(say!("details-layout-path")),
                                !tree,
                                cx,
                            )
                            .on_click(cx.listener(|_, _, _, cx| set_tree(false, cx))),
                        )
                        .child(
                            chosen(
                                Button::new("details-tree")
                                    .icon(Icon::new(gpui_kit::assets::IconName::ListTree))
                                    .label(say!("details-layout-tree")),
                                tree,
                                cx,
                            )
                            .on_click(cx.listener(|_, _, _, cx| set_tree(true, cx))),
                        ),
                )
                .when(has_commit, |row| {
                    row.child(
                        div().absolute().right(px(10.)).top_0().child(
                            chosen(
                                Button::new("against-worktree")
                                    .accessibility_label(say!("details-against"))
                                    .outline()
                                    .xsmall()
                                    .h(px(24.))
                                    .icon(Icon::new(gpui_kit::assets::IconName::FileDiff))
                                    .tooltip(if against {
                                        say!("details-against-off")
                                    } else {
                                        say!("details-against")
                                    }),
                                against,
                                cx,
                            )
                            .on_click(
                                cx.listener(|this, _, _, cx| this.toggle_against_worktree(cx)),
                            ),
                        ),
                    )
                })
                .into_any_element(),
        );
        // A rule 6 under the switch; the files right under it.
        out.push(
            div()
                .mt(px(6.))
                .mx(px(10.))
                .h(px(1.))
                .flex_none()
                .bg(theme.border)
                .into_any_element(),
        );
        if self.files.is_empty() {
            // A partial clone left what the files need on its remote, and
            // nobody asked for it yet: said plainly, with the way to have
            // it.
            if self.files_left_on_remote() {
                out.push(
                    v_flex()
                        .mt(px(8.))
                        .px(px(12.))
                        .gap_2()
                        .flex_none()
                        .items_start()
                        .child(
                            div()
                                .text_xs()
                                .text_color(muted)
                                .whitespace_normal()
                                .child(say!("details-files-on-remote")),
                        )
                        .child(
                            Button::new("details-fetch-files")
                                .outline()
                                .xsmall()
                                .label(say!("details-fetch-files"))
                                .tooltip(say!("details-fetch-files-tooltip"))
                                .on_click(cx.listener(|this, _, _, cx| this.fetch_files(cx))),
                        )
                        .into_any_element(),
                );
                return out;
            }
            if let Some(error) = &self.files_error {
                out.push(
                    div()
                        .mt(px(8.))
                        .px(px(12.))
                        .flex_none()
                        .text_xs()
                        .text_color(muted)
                        .whitespace_normal()
                        .child(SharedString::from(say!("details-files-error", error)))
                        .into_any_element(),
                );
            }
            return out;
        }
        let rows = Rc::new(self.rows(cx));
        let sizes = Rc::new(
            rows.iter()
                .map(|row| {
                    size(
                        px(0.),
                        px(match row {
                            FileRow::Expand { .. } => 30.,
                            FileRow::Folder { .. } => FOLDER_HEIGHT,
                            FileRow::File { .. } => FILE_HEIGHT,
                        }),
                    )
                })
                .collect::<Vec<_>>(),
        );
        let files_room = self.files_room.clone();
        out.push(
            div()
                .id("commit-files")
                .relative()
                .flex_1()
                .min_h(px(FILE_HEIGHT * 2.))
                .child(
                    canvas(
                        move |bounds, _, _| {
                            files_room.set((crate::zoom::unpx(bounds.top()), files_room.get().1))
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(
                    v_virtual_list(
                        cx.entity().clone(),
                        "commit-file-rows",
                        sizes,
                        move |this, range, _, cx| {
                            range
                                .filter_map(|ix| {
                                    Some(this.render_file_row(ix, rows.get(ix)?, listed, cx))
                                })
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.files_scroll),
                )
                .scrollbar(&self.files_scroll, ScrollbarAxis::Vertical)
                // The wheel moves the files before the panel around them
                // (`changes::wheel_first`).
                .child(changes::wheel_first(
                    "commit-files-wheel",
                    &self.files_scroll,
                ))
                .test_support()
                .into_any_element(),
        );
        out
    }

    /// What dragging the row of a file or folder of the list picks up: its
    /// full path in the worktree. None without a worktree.
    fn dragged(&self, path: &str) -> Option<DraggedPaths> {
        DraggedPaths::in_worktree(self.session.summary.workdir.as_deref(), [path])
    }

    /// One row of the files.
    fn render_file_row(
        &self,
        ix: usize,
        row: &FileRow,
        listed: Listed,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        match row {
            FileRow::Expand { all_open } => {
                let all_open = *all_open;
                h_flex()
                    .id(("commit-file", ix))
                    .h(px(30.))
                    .pl(px(20.))
                    .items_center()
                    .text_xs()
                    .cursor_pointer()
                    .child(if all_open {
                        say!("details-collapse-all")
                    } else {
                        say!("details-expand-all")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if all_open {
                            this.open_folders.clear();
                        } else {
                            this.open_folders = file_tree::folders(
                                this.files.iter().map(|file| file.path.as_str()),
                            );
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            }
            FileRow::Folder {
                path,
                name,
                depth,
                open,
                counts,
            } => {
                let (open, counts, folder) = (*open, *counts, path.clone());
                let tip = SharedString::from(format!("{path}/"));
                let (menu_folder, menu_focus) = (path.clone(), self.focus_handle.clone());
                let sparse = self.sparse.clone();
                h_flex()
                    .id(("commit-file", ix))
                    .h(px(FOLDER_HEIGHT))
                    .pl(px(15. + 15. * *depth as f32))
                    .pr(px(10.))
                    .gap(px(4.))
                    .items_center()
                    .text_sm()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.accent))
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if open {
                            this.open_folders.remove(&folder);
                        } else {
                            let paths: Vec<&str> =
                                this.files.iter().map(|file| file.path.as_str()).collect();
                            let opened = file_tree::opens_with(&folder, paths.iter().copied());
                            this.open_folders.extend(opened);
                        }
                        cx.notify();
                    }))
                    // Dragged out as its folder in the worktree.
                    .when_some(self.dragged(path), |row, dragged| {
                        row.on_drag(dragged, drag_paths::chip)
                    })
                    .child(
                        Icon::new(if open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size_3()
                        .flex_none()
                        .text_color(muted),
                    )
                    .child(
                        div()
                            .flex_none()
                            .truncate()
                            .text_color(muted)
                            .child(name.clone()),
                    )
                    .when(!open, |row| {
                        row.child(div().w(px(18.)).flex_none()).children(
                            file_tree::Kind::ALL
                                .into_iter()
                                .filter(|kind| file_tree::count_of(&counts, *kind) > 0)
                                .map(|kind| {
                                    let (icon, colour) = changes::tree_mark(kind, &theme);
                                    h_flex()
                                        .flex_none()
                                        .gap_1()
                                        .mr_2()
                                        .items_center()
                                        .text_xs()
                                        .child(icon.size_3().text_color(colour))
                                        .child(file_tree::count_of(&counts, kind).to_string())
                                }),
                        )
                    })
                    // The folder as the working tree has it: a terminal
                    // there.
                    .context_menu(move |menu, window, cx| {
                        let menu = menu.action_context(menu_focus.clone()).menu(
                            say!("details-terminal"),
                            Box::new(file_actions::RunFileAction {
                                action: FileAction::Terminal {
                                    path: menu_folder.clone(),
                                },
                            }),
                        );
                        // Only this folder, or this one too.
                        super::sparse::folder_entries(
                            menu,
                            &menu_folder,
                            sparse.as_ref(),
                            window,
                            cx,
                        )
                    })
                    .into_any_element()
            }
            FileRow::File {
                file,
                depth,
                folder,
                name,
            } => match self.files.get(*file) {
                Some(diff) => self.render_file(ix, diff, *depth, folder.clone(), name, listed, cx),
                None => div().h(px(FILE_HEIGHT)).into_any_element(),
            },
        }
    }

    /// A file of the commit: its kind as an icon, the folder dimmed and cut
    /// first, the name, and at the right the lines it added and took out.
    #[allow(clippy::too_many_arguments)]
    fn render_file(
        &self,
        ix: usize,
        file: &bitbraid_git::FileDiff,
        depth: usize,
        folder: Option<String>,
        name: &str,
        listed: Listed,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use bitbraid_git::diff::FileChangeKind as Kind;
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        // What a file's menu is about: the commit that has the file, or
        // for a file that was deleted the commit before, which still does.
        let (at, before) = match listed {
            Listed::Commit(id) => (
                id,
                self.commit
                    .as_ref()
                    .filter(|commit| commit.id == id)
                    .and_then(|commit| commit.parents.first().copied()),
            ),
            Listed::Range { old, new } => (new, old),
            Listed::Worktree(id) => (id, Some(id)),
        };
        let selected = self.selected_file.as_deref() == Some(file.path.as_str());
        let (icon, colour) = changes::tree_mark(kind_of(file.kind), &theme);
        let path = file.path.clone();
        let open = file.clone();
        let menu_path = file.path.clone();
        let menu_focus = self.focus_handle.clone();
        let tip = SharedString::from(file.path.clone());
        let from = match listed {
            // Against the working tree: a file that is there is the
            // worktree's; one that is gone is still in the commit.
            Listed::Worktree(id) if file.kind == Kind::Deleted => file_actions::Listed::Commit(id),
            Listed::Worktree(_) => file_actions::Listed::Worktree,
            _ if file.kind == Kind::Deleted => {
                file_actions::Listed::DeletedBy { commit: at, before }
            }
            _ => file_actions::Listed::Commit(at),
        };
        // A file that was moved: its old place, in the commit before.
        let moved_from = file
            .old_path
            .clone()
            .filter(|_| !matches!(listed, Listed::Worktree(_)));
        h_flex()
            .id(("commit-file", ix))
            .test_support()
            .w_full()
            .h(px(FILE_HEIGHT))
            .pl(px(22. + 15. * depth as f32))
            .pr(px(10.))
            .gap(px(4.))
            .items_center()
            .text_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(theme.list_active))
            .when(!selected, |d| d.hover(|s| s.bg(theme.accent)))
            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_file = Some(path.clone());
                let diff = Box::new(open.clone());
                cx.emit(match listed {
                    Listed::Commit(id) => DetailsEvent::OpenFile { id, diff },
                    Listed::Range { old, new } => DetailsEvent::OpenRange { old, new, diff },
                    Listed::Worktree(old) => DetailsEvent::OpenAgainstWorktree { old, diff },
                });
                cx.notify();
            }))
            // Dragged out by its path in the worktree.
            .when_some(self.dragged(&file.path), |row, dragged| {
                row.on_drag(dragged, drag_paths::chip)
            })
            .child(icon.size_3().flex_none().text_color(colour))
            // The folder is cut first, and the slash goes with the name.
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .children(folder.map(|folder| {
                        div()
                            .flex_shrink(1.0)
                            .min_w_0()
                            .truncate()
                            .text_color(muted)
                            .child(folder)
                    }))
                    .child(div().flex_none().max_w_full().truncate().child(
                        if depth == 0 && file.path.contains('/') {
                            format!("/{name}")
                        } else {
                            name.to_owned()
                        },
                    )),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(muted)
                    .child(if file.binary {
                        say!("details-file-binary").to_owned()
                    } else if file.too_large {
                        // Listed, and compared when it is opened and asked for.
                        say!("details-file-large").to_owned()
                    } else {
                        format!("+{} −{}", file.additions, file.deletions)
                    }),
            )
            // What is picked is handed to this view, wherever the focus
            // was when the menu opened.
            .context_menu(move |menu, window, _| {
                // A file's menu is longer than a small window is high.
                let menu = crate::views::menu_fit::fit(menu, window);
                let menu = file_actions::entries(
                    menu.action_context(menu_focus.clone()),
                    &menu_path,
                    from,
                );
                match &moved_from {
                    Some(old_path) => file_actions::old_path_entries(menu, old_path, before),
                    None => menu,
                }
            })
            .into_any_element()
    }

    /// Several commits selected together: a card for each, what their
    /// files are listed as, and the files.
    fn render_many(&self, many: &Many, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let dark = theme.is_dark();
        let what = match many.comparison {
            Some(Comparison { together: true, .. }) => say!("details-many-together").to_owned(),
            Some(Comparison { old, new, .. }) => match old {
                Some(old) => say!(
                    "details-many-differs",
                    old = old.short(7),
                    new = new.short(7)
                ),
                None => say!("details-many-differs-from-nothing", new = new.short(7)),
            },
            None => say!("details-many-not-compared").to_owned(),
        };
        let cards: Vec<AnyElement> = many
            .ids
            .iter()
            .filter_map(|id| self.cards.iter().find(|card| card.id == *id))
            .map(|card| {
                h_flex()
                    .h(px(44.))
                    .flex_none()
                    .px(px(7.))
                    .gap(px(8.))
                    .items_center()
                    .rounded(theme.radius)
                    .bg(theme.secondary)
                    .child(avatar(&card.author.email, px(30.), dark, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_sm().truncate().child(card.summary.clone()))
                            .child(div().text_xs().truncate().text_color(muted).child(format!(
                                "{}, {}",
                                card.author.name,
                                absolute_time(card.author.seconds, card.author.offset)
                            ))),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .child(card.id.short(6)),
                    )
                    .into_any_element()
            })
            .collect();
        let middle = div()
            .text_sm()
            .child(say!("details-many-selected", count = many.ids.len()))
            .into_any_element();
        let mut out = vec![
            self.id_row(middle, cx).into_any_element(),
            div()
                .mt(px(10.))
                .px(px(10.))
                .flex_none()
                .text_base()
                .whitespace_normal()
                .child(what)
                .into_any_element(),
            // As high as the cards, up to its most; in a panel too low for
            // everything it gives way, down to a card, before the files.
            div()
                .id("commit-cards")
                .mt(px(8.))
                .mx(px(10.))
                .p(px(4.))
                .flex_shrink(1.)
                .min_h(px(CARDS_LEAST))
                .max_h(px(CARDS_MOST))
                .overflow_y_scroll()
                .track_scroll(&self.cards_scroll)
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .child(v_flex().gap(px(4.)).children(cards))
                .test_support()
                .into_any_element(),
        ];
        if let Some(comparison) = many.comparison {
            out.extend(self.render_files(
                Listed::Range {
                    old: comparison.old,
                    new: comparison.new,
                },
                cx,
            ));
        }
        out
    }
}

/// List files as a tree, or as paths, everywhere.
fn set_tree(tree: bool, cx: &mut App) {
    let prefs = cx.global_mut::<Prefs>();
    if prefs.preferences.changes_tree != tree {
        prefs.preferences.changes_tree = tree;
        prefs.save_preferences();
    }
}

/// The message's grip, while it is dragged.
#[derive(Debug, Clone, Copy)]
struct MessageGrip;

/// Nothing follows the pointer while the grip is dragged.
struct GripChip;

impl Render for GripChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

impl Focusable for DetailsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DetailsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let room = self.files_room.clone();
        let mut container = v_flex()
            .id("details")
            .relative()
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|_, run: &RunFileAction, _, cx| {
                cx.emit(DetailsEvent::File(run.action.clone()))
            }))
            // The message's grip: the frame follows the pointer, between
            // its least and what leaves the blocks under it their room.
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<MessageGrip>, _, cx| {
                    let (top, grab) = (this.message_drawn.get(), 6.);
                    let bottom = crate::zoom::unpx(event.bounds.bottom());
                    let wanted = crate::zoom::unpx(event.event.position.y) - grab - top;
                    let most = (bottom - top - UNDER_MESSAGE).max(MESSAGE_LEAST);
                    let height = wanted.clamp(MESSAGE_LEAST, most);
                    let extra = height - MESSAGE_HEIGHT;
                    if (extra - this.message_extra).abs() >= 0.5 {
                        this.message_extra = extra;
                        cx.notify();
                    }
                }),
            )
            .size_full()
            // A panel too low for everything even with the message and the
            // cards given way scrolls, rather than leave the files under
            // the status bar out of reach. Across, it cuts off what is
            // wider.
            .overflow_y_scroll()
            .overflow_x_hidden()
            .test_support()
            .bg(theme.background)
            .border_l_1()
            .border_color(theme.border)
            .child(
                canvas(
                    move |bounds, _, _| {
                        room.set((room.get().0, crate::zoom::unpx(bounds.bottom())))
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            );
        // While the working tree has changes, a strip over it all that
        // leads to them; while an operation is stopped on conflicts, the
        // files in conflict, which stand in for the banner over the graph.
        if self.uncommitted > 0 || self.conflicts > 0 {
            let (said, tint, label) = match (self.conflicts, self.uncommitted) {
                (0, count) => (
                    say!("details-uncommitted", count),
                    theme.primary,
                    say!("details-view-changes"),
                ),
                (count, _) => (
                    say!("details-conflicts", count),
                    theme.warning,
                    say!("details-view-conflicts"),
                ),
            };
            container = container.child(
                h_flex()
                    .w_full()
                    .h(px(36.))
                    .flex_none()
                    .gap(px(12.))
                    .items_center()
                    .justify_center()
                    .bg(tint.opacity(0.22))
                    .text_sm()
                    .child(said)
                    .child(
                        Button::new("view-changes")
                            .outline()
                            .xsmall()
                            .label(label)
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(DetailsEvent::ShowUncommitted)),
                            ),
                    ),
            );
        }
        if let Some(many) = self.many.clone() {
            return container.children(self.render_many(&many, cx));
        }
        let Some(id) = self.id else {
            return container.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(muted)
                    .child(say!("details-nothing-selected")),
            );
        };
        // The id, short, with the whole of it one click away.
        let copy_hex = id.to_hex();
        let middle = h_flex()
            .gap_2()
            .items_center()
            .text_sm()
            .child(div().text_color(muted).child(say!("details-commit-label")))
            .child(div().child(id.short(6)))
            .child(
                Button::new("copy-id")
                    .accessibility_label(say!("details-copy-id"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Copy)
                    .tooltip(say!("details-copy-id"))
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.write_to_clipboard(ClipboardItem::new_string(copy_hex.clone()));
                    })),
            )
            .into_any_element();
        container = container.child(self.id_row(middle, cx));
        let Some(commit) = self.commit.clone() else {
            return container.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(muted)
                    .child(say!("details-loading")),
            );
        };
        let stash = self
            .badges
            .iter()
            .any(|badge| badge.kind == BadgeKind::Stash);
        // The message in a frame of its own: the summary, then the rest of
        // it, scrolling inside the frame; the frame as tall as it was
        // dragged.
        let body = self
            .message
            .as_deref()
            .and_then(|message| message.split_once('\n'))
            .map(|(_, rest)| rest.trim().to_owned())
            .filter(|rest| !rest.is_empty());
        let summary = if stash {
            stash_summary(&commit.summary).to_owned()
        } else {
            commit.summary.clone()
        };
        // Issue keys in it are links of the repository's linked remotes:
        // a click shows the issue in the middle.
        let links = crate::providers::drawn_links(cx);
        let view = cx.entity().downgrade();
        let open: crate::views::issues::linked::Open =
            std::rc::Rc::new(move |link, number, _, cx| {
                let _ = view.update(cx, |_, cx| {
                    cx.emit(DetailsEvent::OpenIssue { link, number })
                });
            });
        let summary = crate::views::issues::linked::text(
            "commit-summary-text",
            summary,
            &links,
            open.clone(),
            cx,
        );
        let body = body.map(|body| {
            crate::views::issues::linked::text("commit-body-text", body, &links, open, cx)
        });
        let message_drawn = self.message_drawn.clone();
        let height = (MESSAGE_HEIGHT + self.message_extra).max(MESSAGE_LEAST);
        container = container.child(
            div()
                .id("commit-message")
                .relative()
                .mt(px(10.))
                .mx(px(10.))
                .h(px(height))
                // What gives way first in a panel too low for everything:
                // the files keep their two rows.
                .flex_shrink(1.)
                .min_h(px(MESSAGE_SQUEEZED))
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .child(
                    canvas(
                        move |bounds, _, _| message_drawn.set(crate::zoom::unpx(bounds.top())),
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full(),
                )
                .child(
                    v_flex()
                        .id("commit-message-text")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.message_scroll)
                        .p(px(12.))
                        .gap_2()
                        .child(
                            div()
                                .id("commit-summary")
                                .text_lg()
                                .whitespace_normal()
                                .child(summary)
                                .test_support(),
                        )
                        .children(body.map(|body| {
                            div()
                                .text_sm()
                                .text_color(muted)
                                .whitespace_normal()
                                .child(body)
                        })),
                )
                // A long message moves before the panel under the wheel.
                .child(changes::wheel_first(
                    "commit-message-wheel",
                    &self.message_scroll,
                ))
                .test_support(),
        );
        // Under it, the grip that makes it taller or lower.
        container = container.child(
            div()
                .id("message-grip")
                .mt(px(4.))
                .h(px(12.))
                .w_full()
                .flex_none()
                .flex()
                .justify_center()
                .items_center()
                .cursor_row_resize()
                .on_drag(MessageGrip, |_, _, _, cx| cx.new(|_| GripChip))
                .child(div().w(px(100.)).h(px(4.)).rounded_full().bg(theme.border))
                .test_support(),
        );
        // Who and when at the left, where it comes from at the right: a
        // stash comes from the commit it was made on.
        let parents: Vec<Oid> = if stash {
            commit.parents.first().copied().into_iter().collect()
        } else {
            commit.parents.clone()
        };
        let mut parent_ids: Vec<AnyElement> = Vec::new();
        for (n, parent) in parents.iter().copied().enumerate() {
            if n > 0 {
                parent_ids.push(div().mr_1().child(",").into_any_element());
            }
            parent_ids.push(
                div()
                    .id(SharedString::from(format!("parent-{parent}")))
                    .cursor_pointer()
                    .hover(|style| style.underline())
                    .child(parent.short(6))
                    .on_click(cx.listener(move |_, _, _, cx| cx.emit(DetailsEvent::Reveal(parent))))
                    .into_any_element(),
            );
        }
        container = container.child(
            h_flex()
                .mt(px(2.))
                .px(px(10.))
                .flex_none()
                .gap_2()
                .items_start()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(Self::person(false, &commit.author, cx)),
                )
                .when(!parent_ids.is_empty(), |row| {
                    row.child(
                        h_flex()
                            .flex_none()
                            .pt(px(2.))
                            .gap_1()
                            .text_sm()
                            .child(div().text_color(muted).child(say!("details-parent-label")))
                            .child(h_flex().children(parent_ids)),
                    )
                }),
        );
        if commit.committer.email != commit.author.email
            || commit.committer.seconds != commit.author.seconds
        {
            container = container.child(
                div()
                    .mt(px(10.))
                    .px(px(10.))
                    .flex_none()
                    .child(Self::person(true, &commit.committer, cx)),
            );
        }
        // Whoever else the message says wrote it.
        let others = self.message.as_deref().map(co_authors).unwrap_or_default();
        if !others.is_empty() {
            let dark = theme.is_dark();
            container = container.child(
                h_flex()
                    .mt(px(10.))
                    .px(px(10.))
                    .h(px(20.))
                    .flex_none()
                    .gap(px(6.))
                    .items_center()
                    .text_sm()
                    .child(
                        div()
                            .id("co-authors-label")
                            .flex_none()
                            .whitespace_nowrap()
                            .text_color(muted)
                            .child(say!("details-co-authors-label"))
                            .test_support(),
                    )
                    .children(others.into_iter().enumerate().map(|(n, (name, address))| {
                        let tip = SharedString::from(format!("{name} <{address}>"));
                        div()
                            .id(("co-author", n))
                            .child(avatar(&address, px(20.), dark, cx))
                            .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                            .test_support()
                    })),
            );
        }
        let listed = if self.against_worktree {
            Listed::Worktree(id)
        } else {
            Listed::Commit(id)
        };
        container.children(self.render_files(listed, cx))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{FileRow, co_authors, file_rows, stash_summary};

    fn file(path: &str) -> bitbraid_git::FileDiff {
        bitbraid_git::FileDiff {
            kind: bitbraid_git::diff::FileChangeKind::Modified,
            path: path.to_owned(),
            old_path: None,
            old_id: None,
            new_id: None,
            old_mode: None,
            new_mode: None,
            binary: false,
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
            old_size: None,
            new_size: None,
            too_large: false,
            line_endings: Default::default(),
        }
    }

    #[test]
    fn a_stash_says_what_it_is_without_the_branch_git_puts_before() {
        assert_eq!(
            stash_summary("On main: notes and a new file"),
            "notes and a new file"
        );
        assert_eq!(
            stash_summary("WIP on fix/crash: 1a2b3c4 fix it"),
            "1a2b3c4 fix it"
        );
        // What is not of that shape is said as it is.
        assert_eq!(stash_summary("notes: on top"), "notes: on top");
        assert_eq!(stash_summary("On the road: again"), "On the road: again");
        assert_eq!(stash_summary(""), "");
    }

    #[test]
    fn the_co_authors_are_read_from_the_trailers() {
        let message = "fix: crash\n\nFixes it.\n\nCo-authored-by: Ben Okafor <ben@example.invalid>\nco-authored-by:cleo@example.invalid\nSigned-off-by: Ada <ada@example.invalid>\nCo-authored-by:   \n";
        assert_eq!(
            co_authors(message),
            [
                ("Ben Okafor".to_owned(), "ben@example.invalid".to_owned()),
                (
                    "cleo@example.invalid".to_owned(),
                    "cleo@example.invalid".to_owned()
                ),
            ]
        );
        assert!(co_authors("no trailers").is_empty());
    }

    #[test]
    fn the_files_of_a_commit_are_paths_or_a_tree_in_the_order_of_the_lists() {
        let files = vec![
            file("src/main.rs"),
            file("README.md"),
            file("docs/logo.png"),
        ];
        let names = |rows: &[FileRow]| -> Vec<String> {
            rows.iter()
                .map(|row| match row {
                    FileRow::Expand { all_open } => format!("expand {all_open}"),
                    FileRow::Folder { path, open, .. } => format!("{path}/ {open}"),
                    FileRow::File { file, depth, .. } => {
                        format!("{}{}", "  ".repeat(*depth), files[*file].path)
                    }
                })
                .collect()
        };
        let rows = file_rows(&files, false, &BTreeSet::new(), false);
        assert_eq!(names(&rows), ["docs/logo.png", "README.md", "src/main.rs"]);
        let rows = file_rows(&files, false, &BTreeSet::new(), true);
        assert_eq!(names(&rows), ["src/main.rs", "README.md", "docs/logo.png"]);
        let rows = file_rows(&files, true, &BTreeSet::new(), false);
        assert_eq!(
            names(&rows),
            ["expand false", "docs/ false", "src/ false", "README.md"]
        );
        let open: BTreeSet<String> = ["docs".to_owned(), "src".to_owned()].into();
        let rows = file_rows(&files, true, &open, false);
        assert_eq!(
            names(&rows),
            [
                "expand true",
                "docs/ true",
                "  docs/logo.png",
                "src/ true",
                "  src/main.rs",
                "README.md"
            ]
        );
        // A file at the top of the tree has no folder row.
        let rows = file_rows(&[file("a.rs")], true, &BTreeSet::new(), false);
        assert_eq!(
            rows,
            [FileRow::File {
                file: 0,
                depth: 0,
                folder: None,
                name: "a.rs".to_owned()
            }]
        );
    }
}
