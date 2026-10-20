//! The references panel asked for while what the middle shows holds it
//! folded to its strip (a file's diff and its editor) or has taken its
//! place (a file's history and blame, the merge tool, the plan of a
//! rebase). Those fold the panel when they open and bring it back when
//! they close; asking for the panel (the strip's button, its key, the
//! palette, the keys that reach its filter) is closing them, as their own
//! way out does (an edited file is asked about first), and the panel is
//! open when the graph is back, whatever it was before. Opened beside
//! them, the panel would be drawn into the strip's width.
//!
//! Sources: none (original design).

use gpui_kit::*;

use super::{FileTab, Workspace};
use crate::prefs::Prefs;

impl Workspace {
    /// Whether what the middle shows holds the references panel folded,
    /// or has taken its place.
    pub fn holds_references(&self) -> bool {
        self.show_diff || self.planning
    }

    /// The references panel is asked for: what holds it folded is closed
    /// as its own way out closes it, and the panel is open; the keys go
    /// to the graph, as after Escape.
    pub fn open_references(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.unfold_references(window, cx);
        cx.defer_in(window, |this, window, cx| {
            this.focus_graph(window, cx);
        });
    }

    /// Close what holds the references panel folded, and open the panel.
    pub(super) fn unfold_references(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.session.path.clone();
        if !cx.global::<Prefs>().left_panel_shown(Some(&path)) {
            cx.global_mut::<Prefs>()
                .set_panel_shown(Some(&path), true, true);
            cx.refresh_windows();
        }
        if self.planning {
            // As Escape in the plan: given up, nothing was started.
            self.planning = false;
            self.graph_view
                .update(cx, |view, cx| view.say_selection(cx));
        }
        if self.show_diff {
            // A file's past goes back to the graph, not to the file it
            // was opened from, which would fold the panel again; unless
            // edited text is asked about first, and may stay.
            if self.shows_past() && !self.edit_view.read(cx).is_dirty() {
                self.past_back = None;
                self.past_entry = None;
                self.file_tab = FileTab::Diff;
            }
            self.close_file(window, cx);
        }
        cx.notify();
    }
}
