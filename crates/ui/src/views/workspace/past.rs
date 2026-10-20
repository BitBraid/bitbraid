//! A file's past, which takes the whole width under the toolbar: the
//! commits that changed the file in a list at the left, and beside it what
//! the chosen one did to the file, the file as it was then, or that file
//! blamed. History and blame are this one view; closing it goes back to
//! where it was opened from.
//!
//! Sources: `crates/ui/src/views/history.rs`, `blame.rs`, `diff.rs`.

use bitbraid_git::Oid;
use gpui_kit::component::{
    ActiveTheme, IconName, Sizable, StyledExt,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::*;

use crate::zoom::px;

use super::{FileTab, Workspace};
use crate::views::{
    chosen,
    diff::{CONTROL_GAP, CONTROL_HEIGHT, DiffSource},
};

/// Height of the row that says which file this is, and of the row of
/// controls under it.
const TITLE_HEIGHT: f32 = 36.0;
const CONTROLS_HEIGHT: f32 = 53.0;
/// Width of the list of commits, whatever the window's.
const LIST_WIDTH: f32 = 443.0;
/// In a window too narrow for that, the list leaves this share of the
/// width to what stands beside it.
const BESIDE_SHARE: f32 = 0.4;

/// A commit of the list: its id, where the file is in it, and where the
/// file was before when that commit moved it.
pub(super) type PastEntry = (Oid, String, Option<String>);

/// What stands beside the list for a tab and a way of showing the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Beside {
    /// What the commit changed in the file.
    Changes,
    /// The whole file as of the commit.
    File,
    /// The whole file with the commit each line is from.
    Blamed,
}

/// What is beside the list: blame when that is the tab, else the diff,
/// as changes or as the whole file.
pub(super) fn beside(tab: FileTab, file_view: bool) -> Beside {
    match (tab, file_view) {
        (FileTab::Blame, _) => Beside::Blamed,
        (_, true) => Beside::File,
        (_, false) => Beside::Changes,
    }
}

/// Which panels make room for what the middle shows: the references at
/// the left, the details at the right. A file's past and the merge tool
/// take the whole width; the plan of an interactive rebase takes the
/// place of the references and the graph and leaves the details; a sheet,
/// a file's diff and the graph leave both.
pub(super) fn panels_taken(sheet: bool, planning: bool, file: Option<FileTab>) -> (bool, bool) {
    if sheet {
        return (false, false);
    }
    if planning {
        return (true, false);
    }
    match file {
        Some(FileTab::History | FileTab::Blame | FileTab::Merge) => (true, true),
        Some(FileTab::Diff | FileTab::Edit) | None => (false, false),
    }
}

/// What becomes of a commit the list of a file's past names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Heard {
    /// The view is not on show: nothing.
    Nothing,
    /// It is remembered as the chosen one; what is beside the list stays.
    Noted,
    /// It is remembered and shown beside the list.
    Shown,
}

/// What to do with a commit the list names. A list that arrives after its
/// view was closed says where it begins to nobody, and must not bring the
/// view back. The commit a list begins with (`by_hand` false) leaves a
/// blame that is already about the file alone; one picked from the list
/// is always shown.
pub(super) fn heard(shows_past: bool, tab: FileTab, by_hand: bool) -> Heard {
    if !shows_past {
        Heard::Nothing
    } else if tab == FileTab::Blame && !by_hand {
        Heard::Noted
    } else {
        Heard::Shown
    }
}

impl Workspace {
    /// Whether the middle shows a file's past, which takes the panels'
    /// room as well.
    pub(super) fn shows_past(&self) -> bool {
        self.show_diff && matches!(self.file_tab, FileTab::History | FileTab::Blame)
    }

    /// Which panels make room for the middle as it is now: the references,
    /// the details.
    pub(super) fn centre_takes(&self) -> (bool, bool) {
        // An issue's view and a pull request's take the right panel's
        // place too.
        if matches!(self.sheet, Some(super::Sheet::Issue | super::Sheet::Pull)) {
            return (false, true);
        }
        panels_taken(
            // The activity log lies over the window and takes no panel.
            self.sheet
                .is_some_and(|sheet| sheet != super::Sheet::Activity),
            self.planning,
            self.show_diff.then_some(self.file_tab),
        )
    }

    /// Entering the view of the past from the file pane: what the pane
    /// showed is what closing goes back to, and no commit is chosen yet.
    pub(super) fn enter_past(&mut self, cx: &mut Context<Self>) {
        if !self.shows_past() {
            self.past_back = self.diff_view.read(cx).source().cloned();
            self.past_file_view = self.diff_view.read(cx).file_view();
            self.past_entry = None;
        }
    }

    /// A commit of the list was chosen: show it beside the list. One that
    /// only came with the list (`by_hand` false) leaves a blame that is
    /// already about the file alone.
    pub(super) fn past_chosen(&mut self, entry: PastEntry, by_hand: bool, cx: &mut Context<Self>) {
        match heard(self.shows_past(), self.file_tab, by_hand) {
            Heard::Nothing => {}
            Heard::Noted => self.past_entry = Some(entry),
            Heard::Shown => {
                self.past_entry = Some(entry);
                self.past_show(cx);
                cx.notify();
            }
        }
    }

    /// Bring what is beside the list in line with the chosen commit.
    fn past_show(&mut self, cx: &mut Context<Self>) {
        let Some((id, path, previous)) = self.past_entry.clone() else {
            return;
        };
        // The file as it was is blame without the commit's words: the
        // author's picture and a bar per line.
        let file = self.file_tab == FileTab::History && self.diff_view.read(cx).file_view();
        match self.file_tab {
            FileTab::Blame => self.blame_view.update(cx, |view, cx| {
                view.set_plain(false, cx);
                if view.target() != Some((path.as_str(), Some(id))) {
                    view.show(path, Some(id), cx);
                }
            }),
            _ if file => self.blame_view.update(cx, |view, cx| {
                view.set_plain(true, cx);
                if view.target() != Some((path.as_str(), Some(id))) {
                    view.show(path, Some(id), cx);
                }
            }),
            _ => {
                let source = DiffSource::Revision {
                    id,
                    path,
                    old_path: previous,
                };
                self.diff_view.update(cx, |view, cx| {
                    if view.source() != Some(&source) {
                        view.show(Some(source), cx);
                    }
                });
            }
        }
    }

    /// Blame the file as of a commit of the list (the row's own button).
    pub(super) fn past_blame(&mut self, id: Oid, path: String, cx: &mut Context<Self>) {
        self.past_entry = Some((id, path, None));
        self.file_tab = FileTab::Blame;
        self.set_show_file(true, cx);
        self.history_view.update(cx, |view, cx| view.mark(id, cx));
        self.past_show(cx);
        cx.notify();
    }

    /// What a commit did to the file, asked for from the blame: the diff
    /// beside the list, and the commit marked in it.
    pub(super) fn past_changes(&mut self, id: Oid, path: String, cx: &mut Context<Self>) {
        self.past_entry = Some((id, path, None));
        self.file_tab = FileTab::History;
        self.set_show_file(true, cx);
        self.diff_view
            .update(cx, |view, cx| view.set_file_view(false, cx));
        self.history_view.update(cx, |view, cx| view.mark(id, cx));
        self.past_show(cx);
        cx.notify();
    }

    /// Choose what stands beside the list.
    fn set_beside(&mut self, wanted: Beside, cx: &mut Context<Self>) {
        self.file_tab = match wanted {
            Beside::Blamed => FileTab::Blame,
            Beside::Changes | Beside::File => FileTab::History,
        };
        if wanted != Beside::Blamed {
            self.diff_view.update(cx, |view, cx| {
                view.set_file_view(wanted == Beside::File, cx)
            });
        }
        if self.past_entry.is_some() {
            self.past_show(cx);
        } else if wanted != Beside::Changes {
            // No commit chosen yet: the file as the pane has it.
            if let Some((path, rev)) = self.file.clone() {
                self.blame_view.update(cx, |view, cx| {
                    view.set_plain(wanted == Beside::File, cx);
                    if view.target() != Some((path.as_str(), rev)) {
                        view.show(path, rev, cx);
                    }
                });
            }
        }
        cx.notify();
    }

    /// Leave the past: back to the file as the pane showed it, or, when
    /// the view was opened from a list, back to the graph.
    pub(super) fn close_past(&mut self, cx: &mut Context<Self>) {
        self.past_entry = None;
        match self.past_back.take() {
            Some(source) => {
                self.file_tab = FileTab::Diff;
                let file_view = self.past_file_view;
                self.diff_view
                    .update(cx, |view, cx| view.show_as(Some(source), file_view, cx));
            }
            None => {
                self.file_tab = FileTab::Diff;
                self.set_show_file(false, cx);
                self.diff_view.update(cx, |view, cx| view.show(None, cx));
                self.remember_file(cx);
            }
        }
        cx.notify();
    }

    /// Escape in a file's past, wherever in the window's middle the keys
    /// are: the view closes and the keys go to what is shown again, the
    /// graph or the file. So it does when the view was opened by keys
    /// alone (the finder) and the keys are still the hidden graph's.
    pub(super) fn escape_past(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_past(cx);
        cx.defer_in(window, |this, window, cx| {
            if !this.focus_graph(window, cx) && this.show_diff {
                this.diff_view.read(cx).focus_handle(cx).focus(window, cx);
            }
        });
    }

    /// A commit of the list or of the blame is to be shown in the graph
    /// (the row's button and the gutter's click): the past is put away,
    /// back to the graph rather than to the file it was opened from, so
    /// that the commit is seen selected there and the graph has the keys.
    pub(super) fn reveal_from_past(
        &mut self,
        id: Oid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shows_past() {
            self.past_back = None;
            self.close_past(cx);
        }
        self.reveal(id, window, cx);
    }

    /// The view of a file's past.
    pub(super) fn render_past(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (path, _) = self.file.clone().unwrap_or_default();
        let (folder, name) = match path.rsplit_once('/') {
            Some((folder, name)) => (format!("{folder}/"), name.to_owned()),
            None => (String::new(), path.clone()),
        };
        let file_view = self.diff_view.read(cx).file_view();
        let shown = beside(self.file_tab, file_view);
        let count = self.history_view.read(cx).summary();
        let title = h_flex()
            .w_full()
            .h(px(TITLE_HEIGHT))
            .flex_none()
            .pl(px(10.))
            .pr_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .items_center()
                    .text_sm()
                    .child(
                        div()
                            .flex_none()
                            .pr_2()
                            .font_semibold()
                            .child(say!("workspace-past-heading")),
                    )
                    .child(
                        div()
                            .flex_none()
                            .max_w(relative(0.6))
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(folder),
                    )
                    .child(div().min_w_0().truncate().child(name)),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(count),
            )
            .child(
                Button::new("close-past")
                    .accessibility_label(say!("workspace-close"))
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip(say!("workspace-past-close-tooltip"))
                    .on_click(cx.listener(|this, _, _, cx| this.close_past(cx))),
            );
        // The pair in the middle; what belongs to the choice at the right.
        let text_button =
            |id: &'static str| Button::new(id).small().h(px(CONTROL_HEIGHT)).px(px(10.));
        let pair = ButtonGroup::new("past-view")
            .outline()
            .small()
            .child(
                chosen(
                    text_button("past-file").label(say!("workspace-file-view")),
                    shown != Beside::Changes,
                    cx,
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    if shown == Beside::Changes {
                        this.set_beside(Beside::File, cx);
                    }
                })),
            )
            .child(
                chosen(
                    text_button("past-diff").label(say!("workspace-diff-view")),
                    shown == Beside::Changes,
                    cx,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_beside(Beside::Changes, cx))),
            );
        // The diff's controls are for what it changed; the file as it was
        // is the blame's.
        let diff_controls = (shown == Beside::Changes).then(|| {
            self.diff_view
                .update(cx, |view, cx| view.render_controls(cx))
        });
        let blame_switch = (shown != Beside::Changes).then(|| {
            chosen(
                text_button("past-blame")
                    .outline()
                    .label(say!("workspace-blame"))
                    .tooltip(say!("workspace-past-blame-tooltip")),
                shown == Beside::Blamed,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| {
                this.set_beside(
                    if shown == Beside::Blamed {
                        Beside::File
                    } else {
                        Beside::Blamed
                    },
                    cx,
                )
            }))
        });
        let (groups, extras) = match diff_controls {
            Some(controls) => (Some(controls.groups), Some(controls.extras)),
            None => (None, None),
        };
        // As over a file in the middle: 53 high, the buttons from 10 under
        // the title row, the pair in the middle.
        let controls = h_flex()
            .w_full()
            .h(px(CONTROLS_HEIGHT))
            .flex_none()
            .px_3()
            .pt(px(10.))
            .gap(px(CONTROL_GAP))
            .items_start()
            .child(div().flex_1().flex_basis(px(0.)).min_w_0())
            .child(pair)
            // What is at the right is kept whole: when it is wide the pair
            // gives way to the left rather than be written over.
            .child(
                h_flex()
                    .flex_1()
                    .flex_basis(px(0.))
                    .gap(px(CONTROL_GAP))
                    .items_center()
                    .justify_end()
                    .children(extras)
                    .children(groups)
                    .children(blame_switch),
            );
        let content = match shown {
            Beside::Blamed | Beside::File => self.blame_view.clone().into_any_element(),
            Beside::Changes => self.diff_view.clone().into_any_element(),
        };
        v_flex()
            .id("past")
            .size_full()
            .bg(theme.background)
            .on_action(
                cx.listener(|this, _: &crate::keymap::FocusGraph, window, cx| {
                    this.escape_past(window, cx)
                }),
            )
            .child(title)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .w(px(LIST_WIDTH))
                            .max_w(relative(1.0 - BESIDE_SHARE))
                            .h_full()
                            .flex_none()
                            .border_r_1()
                            .border_color(theme.border)
                            .child(self.history_view.clone()),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .h_full()
                            .child(controls)
                            .child(div().flex_1().min_h_0().child(content)),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::super::FileTab;
    use super::{Beside, Heard, beside, heard, panels_taken};

    #[test]
    fn a_list_that_arrives_late_does_not_bring_its_view_back() {
        // The view was closed while the list was still being read.
        assert_eq!(heard(false, FileTab::History, false), Heard::Nothing);
        assert_eq!(heard(false, FileTab::Diff, true), Heard::Nothing);
        // On show: the commit the list begins with is shown beside it,
        // unless a blame of the file stands there already.
        assert_eq!(heard(true, FileTab::History, false), Heard::Shown);
        assert_eq!(heard(true, FileTab::Blame, false), Heard::Noted);
        // One picked from the list is shown, blamed when blame is on.
        assert_eq!(heard(true, FileTab::History, true), Heard::Shown);
        assert_eq!(heard(true, FileTab::Blame, true), Heard::Shown);
    }

    #[test]
    fn the_panels_make_room_for_what_takes_the_width() {
        // The graph and a file's diff leave both panels.
        assert_eq!(panels_taken(false, false, None), (false, false));
        assert_eq!(
            panels_taken(false, false, Some(FileTab::Diff)),
            (false, false)
        );
        assert_eq!(
            panels_taken(false, false, Some(FileTab::Edit)),
            (false, false)
        );
        // History, blame and the merge tool take the whole width.
        for tab in [FileTab::History, FileTab::Blame, FileTab::Merge] {
            assert_eq!(panels_taken(false, false, Some(tab)), (true, true));
        }
        // The plan of a rebase leaves the details, whatever file was open.
        assert_eq!(panels_taken(false, true, None), (true, false));
        assert_eq!(
            panels_taken(false, true, Some(FileTab::History)),
            (true, false)
        );
        // A sheet is shown between the panels.
        assert_eq!(
            panels_taken(true, true, Some(FileTab::Merge)),
            (false, false)
        );
    }

    #[test]
    fn what_is_beside_the_list_follows_the_tab_and_the_way_the_file_is_shown() {
        assert_eq!(beside(FileTab::History, false), Beside::Changes);
        assert_eq!(beside(FileTab::History, true), Beside::File);
        // Blame is the whole file whatever the diff was last asked for.
        assert_eq!(beside(FileTab::Blame, false), Beside::Blamed);
        assert_eq!(beside(FileTab::Blame, true), Beside::Blamed);
    }
}
