//! A small editor for a worktree file: the quick fix that does not need
//! another program. The file is edited where its file view is, with no bar
//! of its own (the row above the file has reading it again and saving).
//! It saves through `file.write`, which keeps the file's mode, refuses
//! when the file changed on disk meanwhile, and can be undone.
//!
//! Sources: gpui-kit documentation (code editor).

use gpui_kit::component::{
    ActiveTheme,
    input::{Editor, EditorState, InputEvent},
    v_flex,
};
use gpui_kit::*;
use serde_json::json;

use crate::{
    keymap::{EDIT_CONTEXT, SaveFile},
    session::{Reply, Request, SessionHandle},
    views::diff_syntax,
};

/// Largest file opened for editing; beyond it another editor is the tool.
const MAX_TEXT: usize = 2 * 1024 * 1024;

/// Events from the editor.
#[derive(Debug, Clone)]
pub enum EditEvent {
    /// The file was written; status and diffs are stale.
    Saved,
}

/// What the pane holds.
enum State {
    /// Nothing asked for yet.
    Empty,
    /// Waiting for the file.
    Loading,
    /// A file that cannot be edited here, and why.
    Unsuitable(String),
    /// A file being edited.
    Open {
        editor: Entity<EditorState>,
        /// The text as it was read or last saved.
        saved: String,
        /// The blob id of that text on disk, which saving checks.
        base: Option<String>,
        /// The file ends its lines with carriage return and line feed.
        crlf: bool,
        _subscription: Subscription,
    },
}

pub struct EditView {
    session: SessionHandle,
    path: Option<String>,
    state: State,
    dirty: bool,
    saving: bool,
    error: Option<String>,
    focus_handle: FocusHandle,
}

impl EventEmitter<EditEvent> for EditView {}

/// The text of a file as the editor shows it, and whether its lines end in
/// carriage return and line feed. Not text, or too much of it: why not.
fn editable(bytes: Vec<u8>) -> Result<(String, bool), String> {
    if bytes.len() > MAX_TEXT {
        return Err(say!("edit-too-large").to_owned());
    }
    if bytes.contains(&0) {
        return Err(say!("edit-not-text").to_owned());
    }
    let text = String::from_utf8(bytes).map_err(|_| say!("edit-not-utf8").to_owned())?;
    // Only when every line ends that way: a mixed file is left as it is,
    // carriage returns and all.
    let breaks = text.matches('\n').count();
    let crlf = breaks > 0 && text.matches("\r\n").count() == breaks;
    Ok(if crlf {
        (text.replace("\r\n", "\n"), true)
    } else {
        (text, false)
    })
}

/// The text to write for what the editor holds.
pub(crate) fn to_disk(text: &str, crlf: bool) -> String {
    if crlf {
        text.replace('\n', "\r\n")
    } else {
        text.to_owned()
    }
}

impl EditView {
    pub fn new(session: SessionHandle, cx: &mut Context<Self>) -> Self {
        Self {
            session,
            path: None,
            state: State::Empty,
            dirty: false,
            saving: false,
            error: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The path being edited.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Whether there are changes that have not been saved.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Open the worktree file at `path`.
    pub fn open(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.path = Some(path.clone());
        self.state = State::Loading;
        self.dirty = false;
        self.saving = false;
        self.error = None;
        let request = Request::Command {
            id: "file.bytes".to_owned(),
            params: json!({ "path": path, "side": "worktree" }),
        };
        let reply = self.session.send(request);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.path.as_deref() != Some(path.as_str()) {
                    return;
                }
                this.loaded(&path, reply, window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn loaded(&mut self, path: &str, reply: Reply, window: &mut Window, cx: &mut Context<Self>) {
        use base64::Engine as _;
        let value = match reply {
            Reply::Command(value) => value,
            Reply::Error(err) => {
                self.state = State::Unsuitable(err);
                return;
            }
            _ => return,
        };
        let Some(encoded) = value.get("base64").and_then(|text| text.as_str()) else {
            self.state = State::Unsuitable(say!("edit-not-in-worktree").to_owned());
            return;
        };
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap_or_default();
        let (text, crlf) = match editable(bytes) {
            Ok(found) => found,
            Err(why) => {
                self.state = State::Unsuitable(why);
                return;
            }
        };
        let language = diff_syntax::language_for(path).unwrap_or("plaintext");
        let shown = text.clone();
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language.to_owned())
                .line_number(true)
                .soft_wrap(false)
                .searchable(true)
                .default_value(shown)
        });
        let subscription = cx.subscribe(&editor, |this, editor, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let now = editor.read(cx).value();
                if let State::Open { saved, .. } = &this.state {
                    let dirty = now.as_ref() != saved.as_str();
                    if dirty != this.dirty {
                        this.dirty = dirty;
                        cx.notify();
                    }
                }
            }
        });
        self.state = State::Open {
            editor,
            saved: text,
            base: value
                .get("id")
                .and_then(|id| id.as_str())
                .map(str::to_owned),
            crlf,
            _subscription: subscription,
        };
    }

    /// Put the keys in the text, once the file is open in the editor.
    pub fn focus_text(&self, window: &mut Window, cx: &mut App) {
        if let State::Open { editor, .. } = &self.state {
            editor.read(cx).focus_handle(cx).focus(window, cx);
        }
    }

    /// Read the file from the disk again, leaving what was typed.
    pub fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(path) = self.path.clone() {
            self.open(path, window, cx);
        }
    }

    /// Whether the text is being written.
    pub fn is_saving(&self) -> bool {
        self.saving
    }

    /// Whether the file is open in the editor (read, and text).
    pub fn is_open(&self) -> bool {
        matches!(self.state, State::Open { .. })
    }

    /// Leave what was typed and not saved: the file opens from the disk
    /// the next time it is edited.
    pub fn forget(&mut self, cx: &mut Context<Self>) {
        self.path = None;
        self.state = State::Empty;
        self.dirty = false;
        self.error = None;
        cx.notify();
    }

    /// Write what the editor holds to the file.
    pub fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (
            Some(path),
            State::Open {
                editor, base, crlf, ..
            },
        ) = (self.path.clone(), &self.state)
        else {
            return;
        };
        if self.saving || !self.dirty {
            return;
        }
        let text = editor.read(cx).value().to_string();
        let request = Request::Command {
            id: "file.write".to_owned(),
            params: json!({ "path": path, "text": to_disk(&text, *crlf), "base": base }),
        };
        self.saving = true;
        self.error = None;
        let reply = self.session.send(request);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                if this.path.as_deref() != Some(path.as_str()) {
                    return;
                }
                match reply {
                    Reply::Command(value) => {
                        if let State::Open { saved, base, .. } = &mut this.state {
                            *saved = text;
                            *base = value
                                .get("id")
                                .and_then(|id| id.as_str())
                                .map(str::to_owned);
                        }
                        this.dirty = false;
                        cx.emit(EditEvent::Saved);
                    }
                    Reply::Error(err) => this.error = Some(err),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl Focusable for EditView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for EditView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let note = |text: String, color: Hsla| {
            div()
                .p_2()
                .text_sm()
                .text_color(color)
                .child(text)
                .into_any_element()
        };
        let body: AnyElement = match &self.state {
            State::Empty => note(
                say!("edit-select-a-file").to_owned(),
                theme.muted_foreground,
            ),
            State::Loading => note(say!("edit-loading").to_owned(), theme.muted_foreground),
            State::Unsuitable(why) => note(why.clone(), theme.muted_foreground),
            State::Open { editor, .. } => div()
                .size_full()
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.mono_font_size)
                .child(Editor::new(editor).appearance(false).size_full())
                .into_any_element(),
        };
        v_flex()
            .id("edit")
            .key_context(EDIT_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &SaveFile, window, cx| this.save(window, cx)))
            .size_full()
            .bg(theme.background)
            // The file is edited where it is shown: no bar of its own. What
            // went wrong in writing it gets a line, over the text.
            .children(self.error.clone().map(|err| {
                div()
                    .w_full()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.danger.opacity(0.15))
                    .text_xs()
                    .text_color(theme.danger)
                    .child(err)
            }))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::{MAX_TEXT, editable, to_disk};

    #[test]
    fn text_goes_to_the_editor_and_back_unchanged() {
        for text in [
            "",
            "one line, no break",
            "unix\nlines\n",
            "windows\r\nlines\r\n",
            "windows\r\nwithout a last break",
            "mixed\r\nendings\nstay\r\n",
            "a lone \r carriage return\n",
            "ünïcödé ✓\n",
        ] {
            let (shown, crlf) = editable(text.as_bytes().to_vec()).unwrap();
            assert_eq!(to_disk(&shown, crlf), text, "{text:?}");
            // The editor never holds a carriage return it would have to
            // remember, unless the file mixes its endings.
            if crlf {
                assert!(!shown.contains('\r'), "{text:?}");
            }
        }
        // An edit in a file with Windows endings is written with them.
        let (shown, crlf) = editable(b"a\r\nb\r\n".to_vec()).unwrap();
        assert_eq!(to_disk(&format!("{shown}c\n"), crlf), "a\r\nb\r\nc\r\n");
    }

    #[test]
    fn what_is_not_text_is_not_opened() {
        assert!(editable(vec![0x89, b'P', b'N', b'G', 0, 1]).is_err());
        assert!(editable(vec![0xff, 0xfe, b'a']).is_err());
        assert!(editable(vec![b'a'; MAX_TEXT + 1]).is_err());
        assert!(editable(vec![b'a'; MAX_TEXT]).is_ok());
    }
}
