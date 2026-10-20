//! File history: the commits that changed one path, newest first, as the
//! list at the left of the view of a file's past. Choosing a commit shows
//! what it did to the file beside the list; from a row the file can be
//! put back, blamed as of that commit, or the commit shown in the graph.
//!
//! A row has two lines: what the commit says and its id, then when and by
//! whom, and where the file came from when the commit added or moved it.
//!
//! Sources: git-log(1) (`--follow`).

use std::rc::Rc;

use bitbraid_git::{HistoryEntry, Oid, history::HistoryKind};
use gpui_kit::component::{
    ActiveTheme, IconName, Sizable, VirtualListScrollHandle,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::{ScrollableElement, ScrollbarAxis},
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::{
    session::{Reply, Request, SessionHandle},
    views::{absolute_date, avatar::avatar},
};

/// Height of a row: two lines.
const ROW_HEIGHT: f32 = 44.0;
/// Height of the row that ends a list reaching where the file began.
const END_HEIGHT: f32 = 60.0;
/// Entries asked for at once; a file with more history says so.
const MAX_ENTRIES: usize = 500;

/// Events from the history view.
#[derive(Debug, Clone)]
pub enum HistoryEvent {
    /// Show what this commit did to the file.
    Open {
        /// The commit.
        id: Oid,
        /// The file's path in that commit.
        path: String,
        /// Where the file was before, when this commit moved it: the
        /// diff is then between the two places.
        previous: Option<String>,
        /// It was picked from the list, by pointer or key; otherwise it
        /// is the commit the list began with when it arrived.
        by_hand: bool,
    },
    /// Blame the file as of this commit.
    Blame {
        /// The commit.
        id: Oid,
        /// The file's path in that commit.
        path: String,
    },
    /// Select this commit in the graph.
    Reveal(Oid),
    /// The worktree file was put back to a revision; refresh status.
    Mutated,
}

/// The history view.
pub struct HistoryView {
    session: SessionHandle,
    path: Option<String>,
    /// The commit to begin at when the list arrives; the newest without.
    begin_at: Option<Oid>,
    entries: Rc<Vec<HistoryEntry>>,
    sizes: crate::zoom::Sizes,
    selected: Option<usize>,
    loading: bool,
    error: Option<String>,
    scroll: VirtualListScrollHandle,
    focus_handle: FocusHandle,
}

impl EventEmitter<HistoryEvent> for HistoryView {}

/// The second line of a row: when and by whom, and where the file came
/// from when this commit added, removed, moved or copied it.
pub(crate) fn byline(entry: &HistoryEntry, last: bool) -> String {
    let who = say!(
        "history-byline",
        date = absolute_date(entry.commit.author.seconds, entry.commit.author.offset),
        author = entry.commit.author.name.as_str()
    );
    // The last commit's part is said under the list, where it ends.
    let what = match (entry.kind, &entry.previous_path) {
        _ if last => None,
        (HistoryKind::Added, _) => Some(say!("history-added").to_owned()),
        (HistoryKind::Deleted, _) => Some(say!("history-deleted").to_owned()),
        (HistoryKind::Renamed, Some(previous)) => Some(say!("history-moved-from", previous)),
        (HistoryKind::Renamed, None) => Some(say!("history-moved").to_owned()),
        (HistoryKind::Copied, Some(previous)) => Some(say!("history-copied-from", previous)),
        (HistoryKind::Copied, None) => Some(say!("history-copied").to_owned()),
        (HistoryKind::Modified, _) => None,
    };
    match what {
        Some(what) => format!("{who}  ·  {what}"),
        None => who,
    }
}

/// What is said under the last commit of a list that reaches where the
/// file began: how it came to be there.
pub(crate) fn beginning(entry: &HistoryEntry) -> String {
    match (entry.kind, &entry.previous_path) {
        (HistoryKind::Renamed | HistoryKind::Copied, Some(previous)) => {
            say!("history-came-from", previous)
        }
        _ => say!("history-added-here").to_owned(),
    }
}

/// The row to begin at: the one of `wanted` when the list has it, else
/// the newest. None for an empty list.
pub(crate) fn begin_row(entries: &[HistoryEntry], wanted: Option<Oid>) -> Option<usize> {
    if entries.is_empty() {
        return None;
    }
    Some(
        wanted
            .and_then(|id| entries.iter().position(|entry| entry.commit.id == id))
            .unwrap_or(0),
    )
}

impl HistoryView {
    /// Create the view.
    pub fn new(session: SessionHandle, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            session,
            path: None,
            begin_at: None,
            entries: Rc::new(Vec::new()),
            sizes: crate::zoom::Sizes::default(),
            selected: None,
            loading: false,
            error: None,
            scroll: VirtualListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        }
    }

    /// The path whose history is shown or being loaded.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// How much there is, for the title of the view around the list.
    pub fn summary(&self) -> String {
        let count = self.entries.len();
        if self.loading {
            say!("history-loading").to_owned()
        } else if count >= MAX_ENTRIES {
            say!("history-latest-commits", count)
        } else {
            say!("history-commits", count)
        }
    }

    /// Load the history of `path` as HEAD sees it. When the list arrives
    /// it begins at the commit `at`, when it has it, else at the newest,
    /// and says so with [`HistoryEvent::Open`].
    pub fn show(&mut self, path: String, at: Option<Oid>, cx: &mut Context<Self>) {
        self.path = Some(path.clone());
        self.begin_at = at;
        self.entries = Rc::new(Vec::new());
        self.sizes = crate::zoom::Sizes::default();
        self.selected = None;
        self.error = None;
        self.loading = true;
        let request = Request::Command {
            id: "repo.history".to_owned(),
            params: json!({ "path": path, "max_entries": MAX_ENTRIES }),
        };
        self.session
            .call_asked(request, cx, move |this, reply, cx| {
                if this.path.as_deref() != Some(path.as_str()) {
                    return; // another file was asked for meanwhile
                }
                this.loading = false;
                match reply {
                    Reply::Command(value) => {
                        match serde_json::from_value::<Vec<HistoryEntry>>(value) {
                            Ok(entries) => {
                                // A list that reaches where the file began ends
                                // in a row that says so.
                                let mut sizes = vec![size(px(0.), px(ROW_HEIGHT)); entries.len()];
                                if !entries.is_empty() && entries.len() < MAX_ENTRIES {
                                    sizes.push(size(px(0.), px(END_HEIGHT)));
                                }
                                this.sizes = crate::zoom::Sizes::new(sizes);
                                this.entries = Rc::new(entries);
                                if let Some(row) = begin_row(&this.entries, this.begin_at) {
                                    this.choose(row, false, cx);
                                }
                            }
                            Err(err) => this.error = Some(err.to_string()),
                        }
                    }
                    Reply::Error(err) => this.error = Some(err),
                    _ => {}
                }
                cx.notify();
            });
        cx.notify();
    }

    /// Mark the row of this commit as the one shown, without saying so:
    /// for when something else already shows it.
    pub fn mark(&mut self, id: Oid, cx: &mut Context<Self>) {
        if let Some(row) = self.entries.iter().position(|entry| entry.commit.id == id) {
            self.selected = Some(row);
            self.scroll.scroll_to_item(row, ScrollStrategy::Center);
            cx.notify();
        }
    }

    /// Begin again, for a list that is there from the last time and is
    /// shown anew: at the commit `at` when the list has it, else at the
    /// newest, as a list that has just arrived does. A list that is still
    /// being read begins there when it arrives.
    pub fn begin_again(&mut self, at: Option<Oid>, cx: &mut Context<Self>) {
        self.begin_at = at;
        if let Some(row) = begin_row(&self.entries, at) {
            self.choose(row, false, cx);
        }
    }

    /// Make a row the one shown and say which commit that is.
    fn choose(&mut self, row: usize, by_hand: bool, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(row) else {
            return;
        };
        self.selected = Some(row);
        self.scroll.scroll_to_item(row, ScrollStrategy::Center);
        cx.emit(HistoryEvent::Open {
            id: entry.commit.id,
            path: entry.path.clone(),
            previous: entry.previous_path.clone(),
            by_hand,
        });
        cx.notify();
    }

    /// The arrows go through the list; each commit shows as it is reached.
    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let modifiers = event.keystroke.modifiers;
        if modifiers.control || modifiers.alt || modifiers.platform || modifiers.shift {
            return;
        }
        let last = self.entries.len().saturating_sub(1);
        let at = self.selected.unwrap_or(0);
        let to = match event.keystroke.key.as_str() {
            "up" => at.saturating_sub(1),
            "down" => at.saturating_add(1).min(last),
            "home" => 0,
            "end" => last,
            _ => return,
        };
        cx.stop_propagation();
        if self.selected != Some(to) {
            self.choose(to, true, cx);
        }
    }

    /// Put the worktree file back to what it was in `id`. Undo reverses it.
    fn restore(&mut self, id: Oid, cx: &mut Context<Self>) {
        // The file's current path: history may list it under older names.
        let Some(path) = self.path.clone() else {
            return;
        };
        self.error = None;
        let request = Request::Command {
            id: "file.restore".to_owned(),
            params: json!({ "path": path, "rev": id.to_hex() }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Error(err) = reply {
                this.error = Some(err);
            }
            cx.emit(HistoryEvent::Mutated);
            cx.notify();
        });
    }

    fn render_row(&self, ix: usize, entry: &HistoryEntry, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let id = entry.commit.id;
        let blame_path = entry.path.clone();
        let selected = self.selected == Some(ix);
        h_flex()
            .id(("history-row", ix))
            .w_full()
            .h(px(ROW_HEIGHT))
            .pl(px(10.))
            .pr_2()
            .gap(px(11.))
            .items_center()
            .border_b_1()
            .border_color(theme.border.opacity(0.6))
            .cursor_pointer()
            // The one that is shown beside the list is plainly marked.
            .when(selected, |d| d.bg(theme.primary.opacity(0.24)))
            .when(!selected, |d| d.hover(|s| s.bg(theme.list_hover)))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.focus_handle.focus(window, cx);
                this.choose(ix, true, cx);
            }))
            .child(div().flex_none().child(avatar(
                &entry.commit.author.email,
                px(32.),
                theme.is_dark(),
                cx,
            )))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        h_flex()
                            .w_full()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .child(entry.commit.summary.clone()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .font_family(theme.mono_font_family.clone())
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(id.short(7)),
                            ),
                    )
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(byline(entry, ix + 1 == self.entries.len())),
                            )
                            .child(
                                Button::new(("history-restore", ix))
                                    .ghost()
                                    .xsmall()
                                    .label(say!("history-restore"))
                                    .tooltip(say!("history-restore-tooltip"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.restore(id, cx);
                                    })),
                            )
                            .child(
                                Button::new(("history-blame", ix))
                                    .ghost()
                                    .xsmall()
                                    .label(say!("history-blame"))
                                    .tooltip(say!("history-blame-tooltip"))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(HistoryEvent::Blame {
                                            id,
                                            path: blame_path.clone(),
                                        });
                                    })),
                            )
                            .child(
                                Button::new(("history-reveal", ix))
                                    .accessibility_label(say!("history-reveal"))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ExternalLink)
                                    .tooltip(say!("history-reveal"))
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.emit(HistoryEvent::Reveal(id));
                                    })),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl HistoryView {
    /// The end of a list that reaches where the file began: under a rule,
    /// how it came to be there, and that nothing comes before.
    fn render_end(&self, last: &HistoryEntry, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .id("history-end")
            .test_support()
            .w_full()
            .h(px(END_HEIGHT))
            .px(px(10.))
            .pt_2()
            .gap_2()
            .border_t_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(div().child(beginning(last)))
            .child(
                div()
                    .w_full()
                    .text_center()
                    .child(say!("history-no-earlier")),
            )
            .into_any_element()
    }
}

impl Focusable for HistoryView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for HistoryView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let entries = self.entries.clone();
        let list = v_flex()
            .id("history-list")
            .relative()
            .size_full()
            .child(
                v_virtual_list(
                    cx.entity().clone(),
                    "history-rows",
                    self.sizes.get(),
                    move |this, range, _window, cx| {
                        range
                            .filter_map(|ix| match entries.get(ix) {
                                Some(entry) => Some(this.render_row(ix, entry, cx)),
                                None => Some(this.render_end(entries.last()?, cx)),
                            })
                            .collect::<Vec<_>>()
                    },
                )
                .track_scroll(&self.scroll),
            )
            .scrollbar(&self.scroll, ScrollbarAxis::Vertical);

        v_flex()
            .id("history")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .bg(theme.sidebar)
            .when_some(self.error.clone(), |d, err| {
                d.child(div().p_2().text_sm().text_color(theme.danger).child(err))
            })
            .when(self.loading, |d| {
                d.child(
                    div()
                        .p_2()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(say!("history-loading")),
                )
            })
            .child(div().flex_1().min_h_0().child(list))
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_git::{CommitInfo, HistoryEntry, Oid, Signature, history::HistoryKind};

    use super::{begin_row, beginning, byline};

    fn entry(byte: u8, kind: HistoryKind, previous: Option<&str>) -> HistoryEntry {
        let id: Oid = format!("{byte:02x}").repeat(20).parse().unwrap();
        let who = Signature {
            name: "Ada Scratch".to_owned(),
            email: "ada@scratch.invalid".to_owned(),
            seconds: 1_000,
            offset: 0,
        };
        HistoryEntry {
            commit: CommitInfo {
                id,
                parents: Vec::new(),
                author: who.clone(),
                committer: who,
                summary: "A change".to_owned(),
                body: String::new(),
            },
            path: "src/new.rs".to_owned(),
            kind,
            previous_path: previous.map(str::to_owned),
            blob: None,
        }
    }

    #[test]
    fn the_second_line_says_when_by_whom_and_where_the_file_came_from() {
        // The day it was made, as a date.
        assert_eq!(
            byline(&entry(1, HistoryKind::Modified, None), false),
            "1970-01-01 by Ada Scratch"
        );
        assert_eq!(
            byline(&entry(1, HistoryKind::Renamed, Some("src/old.rs")), false),
            "1970-01-01 by Ada Scratch  ·  moved from src/old.rs"
        );
        assert_eq!(
            byline(&entry(1, HistoryKind::Copied, Some("src/old.rs")), false),
            "1970-01-01 by Ada Scratch  ·  copied from src/old.rs"
        );
        assert_eq!(
            byline(&entry(1, HistoryKind::Deleted, None), false),
            "1970-01-01 by Ada Scratch  ·  deleted"
        );
        // The last one's part is said under the list instead.
        let added = entry(1, HistoryKind::Added, None);
        assert_eq!(byline(&added, true), "1970-01-01 by Ada Scratch");
        assert_eq!(beginning(&added), "The file was added here.");
        let moved = entry(1, HistoryKind::Renamed, Some("src/old.rs"));
        assert!(beginning(&moved).contains("src/old.rs"));
    }

    #[test]
    fn the_list_begins_at_the_commit_asked_for_or_at_the_newest() {
        let entries = vec![
            entry(1, HistoryKind::Modified, None),
            entry(2, HistoryKind::Modified, None),
            entry(3, HistoryKind::Added, None),
        ];
        let id = |byte: u8| entries[usize::from(byte) - 1].commit.id;
        assert_eq!(begin_row(&entries, None), Some(0));
        assert_eq!(begin_row(&entries, Some(id(2))), Some(1));
        // A commit that did not touch the file: the newest that did.
        let other: Oid = "ff".repeat(20).parse().unwrap();
        assert_eq!(begin_row(&entries, Some(other)), Some(0));
        assert_eq!(begin_row(&[], Some(id(1))), None);
    }
}
