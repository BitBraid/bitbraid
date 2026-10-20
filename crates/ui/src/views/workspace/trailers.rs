//! Asking for a trailer the commit box is to add: its value, and its key
//! when none was chosen from the menu.

use gpui_kit::*;

use super::{PromptField, RightPane, Workspace};

impl Workspace {
    /// Show the commit box and ask for a trailer, key and value (from the
    /// palette).
    pub fn ask_trailer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.right = RightPane::Wip;
        self.graph_view.update(cx, |view, cx| view.select_wip(cx));
        self.prompt_trailer(String::new(), window, cx);
        cx.notify();
    }

    /// Ask for the value of a trailer, and for its key when there is none
    /// yet, and add it to the message in the commit box.
    pub(super) fn prompt_trailer(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = key.trim().to_owned();
        if key.is_empty() {
            self.prompt(
                say!("dialogs-trailer-title").to_owned(),
                say!("dialogs-add"),
                vec![
                    PromptField::required(say!("dialogs-trailer-key-placeholder"), ""),
                    PromptField::required(say!("dialogs-trailer-value-placeholder"), ""),
                ],
                |this, texts, _, cx| {
                    let mut texts = texts.into_iter();
                    let key = texts.next().unwrap_or_default();
                    let value = texts.next().unwrap_or_default();
                    this.commit_box
                        .update(cx, |view, cx| view.add_trailer(&key, &value, cx));
                },
                window,
                cx,
            );
            return;
        }
        self.prompt(
            say!("dialogs-trailer-key-title", key = key.as_str()),
            say!("dialogs-add"),
            vec![PromptField::required(
                if crate::views::commit_box::names_a_person(&key) {
                    say!("dialogs-trailer-person-placeholder")
                } else {
                    say!("dialogs-trailer-value-placeholder")
                },
                "",
            )],
            move |this, texts, _, cx| {
                let value = texts.into_iter().next().unwrap_or_default();
                this.commit_box
                    .update(cx, |view, cx| view.add_trailer(&key, &value, cx));
            },
            window,
            cx,
        );
    }
}
