//! Closing the file in the centre while its text was edited and not
//! saved: the window asks first, in the bar where the toolbar is, with
//! saving, leaving the changes, and giving up. Saving closes the file once
//! it is written; leaving the changes throws them away, so that the file
//! opens from the disk the next time; giving up stays.
//!
//! Sources: `crates/ui/src/ask.rs`.

use gpui_kit::*;

use super::Workspace;
use crate::views::diff::DiffSource;

impl Workspace {
    /// Close the file in the centre, asking first when its edited text
    /// was not saved.
    pub(super) fn close_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let unsaved = self.edit_view.read(cx).is_dirty();
        if !unsaved {
            self.close_file_now(cx);
            return;
        }
        let name = self
            .edit_view
            .read(cx)
            .path()
            .map(|path| path.rsplit('/').next().unwrap_or(path).to_owned())
            .unwrap_or_default();
        let (for_save, for_discard) = (cx.entity().downgrade(), cx.entity().downgrade());
        crate::ask::show(
            crate::ask::Ask::new(say!("workspace-file-unsaved-question", name))
                .go(say!("workspace-save"), move |_, window, cx| {
                    let _ = for_save.update(cx, |this, cx| {
                        // It closes when it is written (see `saved`): a
                        // file that cannot be written stays open, saying so.
                        this.close_after_save = true;
                        this.edit_view.update(cx, |view, cx| view.save(window, cx));
                    });
                })
                .danger(say!("workspace-discard"), move |_, _, cx| {
                    let _ = for_discard.update(cx, |this, cx| {
                        this.edit_view.update(cx, |view, cx| view.forget(cx));
                        this.close_file_now(cx);
                    });
                }),
            window,
            cx,
        );
    }

    /// The editor wrote the file: close it if that was what was waited for.
    pub(super) fn saved(&mut self, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.close_after_save) {
            self.close_file_now(cx);
        }
    }

    /// Close the file in the centre and go back to the graph.
    fn close_file_now(&mut self, cx: &mut Context<Self>) {
        self.close_after_save = false;
        self.set_show_file(false, cx);
        self.diff_view
            .update(cx, |view, cx| view.show(None::<DiffSource>, cx));
        self.remember_file(cx);
        cx.notify();
    }
}
