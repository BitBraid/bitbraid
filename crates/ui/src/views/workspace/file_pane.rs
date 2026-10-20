//! A file in the place of the graph, under two rows.
//!
//! The first row, 36 high: what happened to the file as a sign and its
//! path (a moved one says where from), the short id of the commit it is
//! from, then at the right how many changes, the words that changed, the
//! external tool and reading it again, the button that puts it into the
//! index or takes it out (in the colour of going ahead and of danger),
//! and the way out, a dot while edited text is not saved.
//!
//! The second row, 53 high, its buttons 23 high from 10 under the first:
//! three parts. At the left editing the file (or a word that says it is
//! edited) and resolving it while it is in conflict; in the middle which
//! side of the index, as two joined buttons of which only the sides the
//! file has are there, and the file as it is or how it changed; at the
//! right the file's past, then the diff's own groups. The left and the
//! right part share what is left of the width, so the middle stands in
//! the middle of the pane when there is room and gives way to the left
//! when the right part needs it.
//!
//! What a file cannot have is not offered: no editing a file that is
//! gone, binary, a picture or a submodule; no blame for the last three,
//! and no past at all for a file new in the worktree. Escape closes the
//! file.
//!
//! Sources: `crates/ui/src/views/diff.rs`.

use bitbraid_git::diff::FileChangeKind;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Selectable, Sizable, StyledExt,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use super::{FileTab, Workspace};
use crate::views::{
    changes::Side,
    chosen,
    diff::{CONTROL_GAP, CONTROL_HEIGHT, DiffSource, FileShape},
};

/// Height of the first row, and of the second.
const FIRST_ROW: f32 = 36.0;
const SECOND_ROW: f32 = 53.0;
/// How far under the first row the buttons of the second begin.
const SECOND_ROW_TOP: f32 = 10.0;
/// What a button of words has before and after them.
const TEXT_PADDING: f32 = 10.0;

/// What the rows above a file offer for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Offered {
    pub edit: bool,
    pub history: bool,
    /// Who wrote its lines.
    pub blame: bool,
}

/// What can be done with a file of this shape: one of the worktree
/// (`worktree`) or of a commit.
pub(crate) fn offered(shape: &FileShape, worktree: bool) -> Offered {
    let gone = shape.kind == Some(FileChangeKind::Deleted);
    // A file that is only now added has no past yet.
    let new = worktree && shape.kind == Some(FileChangeKind::Added);
    Offered {
        edit: shape.text && !gone,
        history: !new,
        blame: !new && shape.text,
    }
}

impl Workspace {
    /// Whether Escape closes the file: while the file pane is what the
    /// middle shows, and the keys are in it, in the list of changes, or
    /// nowhere. A menu or a field that has them takes Escape itself.
    pub(super) fn escape_closes_file(&self, window: &Window, cx: &App) -> bool {
        let pane = self.sheet.is_none()
            && !self.planning
            && self.show_diff
            && !self.shows_past()
            && !matches!(self.file_tab, FileTab::Merge);
        let keys_here = window.focused(cx).is_none()
            || self
                .diff_view
                .read(cx)
                .focus_handle(cx)
                .contains_focused(window, cx)
            || self
                .changes_view
                .read(cx)
                .focus_handle(cx)
                .is_focused(window);
        pane && keys_here
    }

    /// The file in the middle, under its two rows.
    pub(super) fn render_file_pane(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let file_tab = self.file_tab;
        let (path, rev) = self.file.clone().unwrap_or_default();
        let worktree = rev.is_none();
        let unsaved = self.edit_view.read(cx).is_dirty();
        let in_conflict = worktree
            && self.in_progress.as_ref().is_some_and(|operation| {
                operation
                    .conflicts
                    .iter()
                    .any(|conflict| conflict.path == path)
            });
        let shape = self.diff_view.read(cx).shape();
        let can = offered(&shape, worktree);
        let side = self.diff_view.read(cx).side().filter(|_| worktree);
        // The sides of the index the file has, from the status.
        let sides: Vec<Side> = self
            .status
            .iter()
            .find(|entry| entry.path == path)
            .filter(|_| worktree && !in_conflict)
            .map(|entry| {
                [
                    (entry.unstaged.is_some(), Side::Unstaged),
                    (entry.staged.is_some(), Side::Staged),
                ]
                .into_iter()
                .filter(|(has, _)| *has)
                .map(|(_, side)| side)
                .collect()
            })
            .unwrap_or_default();
        let file_view = self.diff_view.read(cx).file_view();
        let controls = (file_tab == FileTab::Diff).then(|| {
            self.diff_view
                .update(cx, |view, cx| view.render_controls(cx))
        });
        let (groups, extras) = match controls {
            Some(controls) => (Some(controls.groups), Some(controls.extras)),
            None => (None, None),
        };

        // A button of words in the second row.
        let text_button = |id: &'static str| {
            Button::new(id)
                .small()
                .h(px(CONTROL_HEIGHT))
                .px(px(TEXT_PADDING))
        };

        // The path: the folder dimmed, the name, and for a moved file
        // where it was.
        let split = |path: &str| match path.rsplit_once('/') {
            Some((folder, name)) => (format!("{folder}/"), name.to_owned()),
            None => (String::new(), path.to_owned()),
        };
        let named = |path: &str| {
            let (folder, name) = split(path);
            h_flex()
                .min_w_0()
                .child(
                    div()
                        .flex_none()
                        .max_w(px(480.))
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child(folder),
                )
                .child(div().min_w_0().truncate().font_semibold().child(name))
        };
        let (mark, colour) = match shape.kind {
            Some(FileChangeKind::Added) => (Icon::new(IconName::Plus), theme.green),
            Some(FileChangeKind::Deleted) => (Icon::new(IconName::Minus), theme.red),
            Some(FileChangeKind::Renamed | FileChangeKind::Copied) => {
                (Icon::new(gpui_kit::assets::IconName::FilePen), theme.blue)
            }
            _ => (Icon::new(gpui_kit::assets::IconName::Pencil), theme.yellow),
        };
        let first_row = h_flex()
            .w_full()
            .h(px(FIRST_ROW))
            .flex_none()
            .px_3()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .when(shape.kind.is_some(), |row| {
                        row.child(mark.small().flex_none().text_color(colour))
                    })
                    .when_some(shape.moved_from.clone(), |row, from| {
                        row.child(named(&from)).child(
                            div()
                                .flex_none()
                                .text_color(theme.muted_foreground)
                                .child("→"),
                        )
                    })
                    .child(named(&path))
                    .when_some(rev, |row, rev| {
                        row.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .font_family(theme.mono_font_family.clone())
                                .text_color(theme.muted_foreground)
                                .child(rev.short(7)),
                        )
                    }),
            )
            .children(extras)
            .when_some(side.filter(|_| !in_conflict), |row, side| {
                let stage = side == Side::Unstaged;
                let button = Button::new("file-stage").outline();
                row.child(
                    if stage {
                        button.success()
                    } else {
                        button.danger()
                    }
                    .xsmall()
                    .px(px(8.))
                    .label(if stage {
                        say!("workspace-file-stage")
                    } else {
                        say!("workspace-file-unstage")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.move_file(stage, cx))),
                )
            })
            .child(
                // A dot while the edited text is not saved; closing then
                // asks first.
                Button::new("close-diff")
                    .accessibility_label(say!("workspace-file-close"))
                    .ghost()
                    .xsmall()
                    .icon(if unsaved {
                        Icon::new(gpui_kit::assets::IconName::Dot)
                    } else {
                        Icon::new(IconName::Close)
                    })
                    .tooltip(if unsaved {
                        say!("workspace-file-close-unsaved")
                    } else {
                        say!("workspace-close")
                    })
                    .on_click(cx.listener(|this, _, window, cx| this.close_file(window, cx))),
            );

        let showing = |tab: FileTab, whole: bool| {
            move |this: &mut Self, _: &ClickEvent, _: &mut Window, cx: &mut Context<Self>| {
                this.diff_view
                    .update(cx, |view, cx| view.set_file_view(whole, cx));
                this.set_file_tab(tab, cx);
            }
        };
        // At the left: editing, or the word that says it is edited, and
        // resolving while the file is in conflict. Never narrower than
        // what it has, so that a narrow pane puts the groups on more
        // lines rather than one over another.
        let left = h_flex()
            .flex_1()
            .flex_basis(px(0.))
            .gap(px(CONTROL_GAP))
            .items_center()
            .when(can.edit && file_tab != FileTab::Edit, |row| {
                row.child(
                    text_button("tab-edit")
                        .outline()
                        .icon(Icon::new(gpui_kit::assets::IconName::Pencil))
                        .label(say!("workspace-file-edit"))
                        .on_click(cx.listener(|this, _, window, cx| this.edit_file(window, cx))),
                )
            })
            .when(file_tab == FileTab::Edit, |row| {
                row.child(
                    h_flex()
                        .id("editing")
                        .test_support()
                        .flex_none()
                        .h(px(CONTROL_HEIGHT))
                        .gap_1()
                        .items_center()
                        .text_sm()
                        .italic()
                        .text_color(theme.muted_foreground)
                        .child(
                            Icon::new(gpui_kit::assets::IconName::Pencil)
                                .small()
                                .text_color(theme.muted_foreground),
                        )
                        .child(say!("workspace-file-editing")),
                )
            })
            .when(in_conflict, |row| {
                row.child(
                    text_button("tab-merge")
                        .outline()
                        .label(say!("workspace-file-resolve"))
                        .selected(file_tab == FileTab::Merge)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some((path, None)) = this.file.clone() {
                                this.show_merge_tool(path, window, cx);
                            }
                        })),
                )
            });
        // In the middle: the sides of the index, and the file as it is or
        // how it changed.
        let middle = h_flex()
            .flex_none()
            .gap(px(CONTROL_GAP))
            .items_center()
            .when(!sides.is_empty(), |row| {
                row.child(ButtonGroup::new("file-side").outline().small().children(
                    sides.iter().map(|shown| {
                        let shown = *shown;
                        let (id, label) = match shown {
                            Side::Unstaged => ("side-unstaged", say!("workspace-file-unstaged")),
                            Side::Staged => ("side-staged", say!("workspace-file-staged")),
                        };
                        chosen(
                            text_button(id).label(label),
                            side == Some(shown) && file_tab == FileTab::Diff,
                            cx,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.show_side(shown, cx)))
                    }),
                ))
            })
            .child(
                ButtonGroup::new("file-view")
                    .outline()
                    .small()
                    .child(
                        chosen(
                            text_button("view-file").label(say!("workspace-file-view")),
                            (file_tab == FileTab::Diff && file_view) || file_tab == FileTab::Edit,
                            cx,
                        )
                        // The file being edited is the file view.
                        .when(file_tab != FileTab::Edit, |button| {
                            button.on_click(cx.listener(showing(FileTab::Diff, true)))
                        }),
                    )
                    .child(
                        chosen(
                            text_button("view-diff").label(say!("workspace-diff-view")),
                            file_tab == FileTab::Diff && !file_view,
                            cx,
                        )
                        .on_click(cx.listener(showing(FileTab::Diff, false))),
                    ),
            );
        // At the right: the file's past, then what the diff has.
        let past = (can.history || can.blame).then(|| {
            ButtonGroup::new("file-past")
                .outline()
                .small()
                .when(can.blame, |group| {
                    group.child(
                        text_button("tab-blame")
                            .label(say!("workspace-blame"))
                            .selected(file_tab == FileTab::Blame)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_file_tab(FileTab::Blame, cx);
                                this.history_view
                                    .read(cx)
                                    .focus_handle(cx)
                                    .focus(window, cx);
                            })),
                    )
                })
                .when(can.history, |group| {
                    group.child(
                        text_button("tab-history")
                            .label(say!("workspace-history"))
                            .selected(file_tab == FileTab::History)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.set_file_tab(FileTab::History, cx);
                                this.history_view
                                    .read(cx)
                                    .focus_handle(cx)
                                    .focus(window, cx);
                            })),
                    )
                })
        });
        let saving = self.edit_view.read(cx).is_saving();
        let right = h_flex()
            .flex_1()
            .flex_basis(px(0.))
            .gap(px(CONTROL_GAP))
            .items_center()
            .justify_end()
            .children(past)
            .children(groups)
            // While editing: reading the file again and saving it (Ctrl+S
            // saves too, and closing asks).
            .when(file_tab == FileTab::Edit, |row| {
                row.child(
                    Button::new("edit-revert")
                        .accessibility_label(say!("workspace-file-reload"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::RefreshCw)
                        .tooltip(say!("workspace-file-reload-tooltip"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.edit_view
                                .update(cx, |view, cx| view.reload(window, cx))
                        })),
                )
                .child(
                    Button::new("edit-save")
                        .accessibility_label(say!("workspace-save"))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(gpui_kit::assets::IconName::Save))
                        .tooltip(say!("workspace-file-save-tooltip"))
                        .disabled(!unsaved || saving)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.edit_view.update(cx, |view, cx| view.save(window, cx))
                        })),
                )
            });
        // A pane too narrow for the three groups on one line (a large
        // zoom) has them on two or three, each line as high as one.
        let second_row = h_flex()
            .id("file-controls")
            .test_support()
            .w_full()
            .min_h(px(SECOND_ROW))
            .flex_none()
            .flex_wrap()
            .px_3()
            .pt(px(SECOND_ROW_TOP))
            .pb(px(SECOND_ROW - SECOND_ROW_TOP - CONTROL_HEIGHT))
            .gap(px(CONTROL_GAP))
            .items_start()
            .child(left)
            .child(middle)
            .child(right);
        v_flex()
            .size_full()
            .child(first_row)
            .child(second_row)
            .child(div().flex_1().min_h_0().child(match file_tab {
                FileTab::Diff => self.diff_view.clone().into_any_element(),
                FileTab::History => self.history_view.clone().into_any_element(),
                FileTab::Blame => self.blame_view.clone().into_any_element(),
                FileTab::Edit => self.edit_view.clone().into_any_element(),
                FileTab::Merge => self.merge_tool.clone().into_any_element(),
            }))
            .into_any_element()
    }

    /// Show the other side of the index of the file in the middle.
    fn show_side(&mut self, side: Side, cx: &mut Context<Self>) {
        let Some((path, None)) = self.file.clone() else {
            return;
        };
        self.file_tab = FileTab::Diff;
        self.diff_view.update(cx, |view, cx| {
            view.show(Some(DiffSource::Working { path, side }), cx)
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_git::diff::FileChangeKind;

    use super::{Offered, offered};
    use crate::views::diff::FileShape;

    #[test]
    fn what_a_file_cannot_have_is_not_offered() {
        let shape = |kind, text| FileShape {
            kind: Some(kind),
            text,
            moved_from: None,
        };
        let all = Offered {
            edit: true,
            history: true,
            blame: true,
        };
        assert_eq!(offered(&shape(FileChangeKind::Modified, true), true), all);
        // Gone: nothing to edit; its past is there.
        assert_eq!(
            offered(&shape(FileChangeKind::Deleted, true), true),
            Offered { edit: false, ..all }
        );
        // New in the worktree: no past yet.
        assert_eq!(
            offered(&shape(FileChangeKind::Added, true), true),
            Offered {
                edit: true,
                history: false,
                blame: false,
            }
        );
        // Added by a commit: that commit is its past.
        assert_eq!(offered(&shape(FileChangeKind::Added, true), false), all);
        // Binary, a picture, a submodule: its history only.
        assert_eq!(
            offered(&shape(FileChangeKind::Modified, false), true),
            Offered {
                edit: false,
                history: true,
                blame: false,
            }
        );
    }
}
