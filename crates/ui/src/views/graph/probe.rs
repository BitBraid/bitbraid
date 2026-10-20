//! What the graph has on show, as plain facts, for the off-screen tests
//! (see `shell/probe.rs`). Nothing here changes anything.

use gpui_kit::{App, Focusable as _, Window};

use crate::zoom::px;

use super::GraphView;

/// What the graph has on show.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GraphProbe {
    /// How many commits are listed.
    pub commits: usize,
    /// Whether the row of uncommitted work is there, above them.
    pub wip_row: bool,
    /// Whether that row is the one selected.
    pub wip_selected: bool,
    /// The selected commit, by its id in full.
    pub selected: Option<String>,
    /// Its summary, once its text was read.
    pub selected_summary: Option<String>,
    /// How many rows are selected together.
    pub picked: usize,
    /// Whether the search bar is shown, what is searched for and how many
    /// rows match.
    pub search_open: bool,
    pub search_text: String,
    pub matches: usize,
    /// What the search field itself holds; what is searched for follows
    /// it.
    pub search_field: String,
    /// Whether the menu of what a drop can do is open.
    pub drop_menu: bool,
    /// What the field of the row of uncommitted work says: the summary
    /// typed in the commit form.
    pub wip_summary: String,
    /// Whether something is being dragged over the graph, so that the
    /// labels it could be dropped on are outlined.
    pub label_drag: bool,
    /// The row of the list whose labels are unfolded over the rows below.
    pub unfolded: Option<usize>,
    /// The label widened to its whole name over what is right of it: the
    /// row of the list and its place among the row's labels.
    pub widened: Option<(usize, usize)>,
    /// The row whose label column has a field for a new name, and what
    /// the name is for.
    pub naming: Option<(usize, &'static str)>,
    /// Whether the columns' heading was laid out for another width than
    /// the list has now, and whether the graph asked for the frame that
    /// lays it out again.
    pub heading_behind: bool,
    pub heading_asked: bool,
    /// The commits lit by the highlight of a branch, by their ids in full,
    /// the one it is lit for first; empty while nothing is.
    pub lit: Vec<String>,
    /// How far the other rows have gone, in percent, from how they were
    /// before the last change of what is lit to how `lit` has them: 100
    /// while nothing changes.
    pub fade: u8,
}

impl GraphView {
    /// What the graph has on show.
    pub fn probe(&self, cx: &App) -> GraphProbe {
        let commits = self.data.as_ref().map(|data| data.commits.as_slice());
        let selected = self
            .selected_commit()
            .and_then(|ix| commits.and_then(|commits| commits.get(ix)))
            .map(|commit| commit.id);
        GraphProbe {
            commits: commits.map(<[_]>::len).unwrap_or(0),
            wip_row: self.offset() == 1,
            wip_selected: self.offset() == 1 && self.selected == Some(0),
            selected: selected.map(|id| id.to_hex()),
            selected_summary: selected
                .and_then(|id| self.details.get(&id))
                .map(|info| info.summary.clone()),
            picked: self.picked.len(),
            search_open: self.search_open,
            search_text: self.search_text.clone(),
            matches: self.matches.len(),
            search_field: self.search.read(cx).value().to_string(),
            drop_menu: self.drop_menu.is_some(),
            label_drag: self.label_drag,
            wip_summary: self
                .summary_source
                .as_ref()
                .map(|field| field.read(cx).value().to_string())
                .unwrap_or_default(),
            unfolded: self.unfold.row,
            widened: self.widen.shown(),
            naming: self
                .naming
                .as_ref()
                .map(|naming| (naming.row, naming.what.word())),
            heading_behind: (self.heading_drawn.get()
                - crate::zoom::unpx(self.scroll.bounds().size.width))
            .abs()
                > 0.5,
            heading_asked: self.heading_asked.get(),
            lit: self
                .light
                .lit()
                .map(|(row, line)| {
                    let commits = commits.unwrap_or_default();
                    let mut lit: Vec<usize> = line.iter().copied().filter(|ix| ix != row).collect();
                    lit.sort_unstable();
                    std::iter::once(*row)
                        .chain(lit)
                        .filter_map(|ix| commits.get(ix).map(|commit| commit.id.to_hex()))
                        .collect()
                })
                .unwrap_or_default(),
            fade: (self
                .light
                .progress_at(cx.background_executor().now(), cx.reduce_motion())
                * 100.0)
                .round() as u8,
        }
    }

    /// What the labels of the row `row` of the list say, in order: one
    /// for each name, as the row shows them (the first) and folds them
    /// (the rest).
    pub fn label_names(&self, row: usize) -> Vec<String> {
        let Some(data) = &self.data else {
            return Vec::new();
        };
        let Some(commit) = self.commit_ix(row).and_then(|ix| data.commits.get(ix)) else {
            return Vec::new();
        };
        let labels = data
            .badges
            .get(&commit.id)
            .map(|badges| super::ref_labels(badges))
            .unwrap_or_default();
        super::with_worktrees(labels, &data.worktrees, commit.id)
            .into_iter()
            .map(|label| label.name)
            .collect()
    }

    /// Where in the window the row `row` of the list is, as it was drawn
    /// last: a place on its labels (the first of them) when `on_label`,
    /// else a place on its message. For a test that points at a row: its
    /// rows are a virtual list's and have no ids to be found by.
    pub fn spot(&self, row: usize, on_label: bool) -> (f32, f32) {
        let list = self.scroll.bounds();
        let down =
            px(super::ROW_HEIGHT * (row as f32 + 0.5) + self.list_inset()) + self.scroll.offset().y;
        let across = if on_label {
            // Inside the first label of the column at the left.
            px(40.)
        } else {
            list.size.width * 0.7
        };
        (
            f32::from(list.origin.x + across),
            f32::from(list.origin.y + down),
        )
    }

    /// Where in the window the heading of a column after the graph is: its
    /// middle. None when the column is not shown.
    pub fn heading_spot(&self, column: crate::prefs::Column, cx: &App) -> Option<(f32, f32)> {
        let (refs, graph) = self.widths(cx);
        let list = self.scroll.bounds();
        // Along the list in points, and in the window at the zoom.
        let zoom = crate::zoom::scale();
        let mut along = refs + graph;
        for (shown, width) in self.column_widths(cx) {
            if shown == column {
                let x = f32::from(list.origin.x) + (along + width.min(80.0) / 2.0) * zoom;
                let y = f32::from(list.origin.y)
                    - (super::HEADER_GAP + super::HEADER_HEIGHT / 2.0) * zoom;
                return Some((x, y));
            }
            along += width;
        }
        None
    }

    /// Where in the window the field of the row of uncommitted work is: a
    /// little into it, past the label and graph columns.
    pub fn wip_field_spot(&self, cx: &App) -> (f32, f32) {
        let (refs, graph) = self.widths(cx);
        let list = self.scroll.bounds();
        let down = px(super::ROW_HEIGHT * 0.5) + self.scroll.offset().y;
        (
            f32::from(list.origin.x) + (refs + graph + 24.0) * crate::zoom::scale(),
            f32::from(list.origin.y + down),
        )
    }

    /// Whether the cursor is in the search field.
    pub fn search_focused(&self, window: &Window, cx: &App) -> bool {
        self.search.read(cx).focus_handle(cx).is_focused(window)
    }

    /// Whether the keys go to the rows of the graph.
    pub fn rows_focused(&self, window: &Window, _cx: &App) -> bool {
        self.focus_handle.is_focused(window)
    }
}
