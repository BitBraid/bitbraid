//! Every key there is, searchable: what it does, where it applies, and
//! which keys are the user's own. A key is changed here by pressing it: a
//! row is opened (a click, or the arrows and Enter from the search field),
//! the new key is pressed, what it would take from another action is said
//! before anything is saved, and saving writes `keybindings.toml`, the same
//! file that can be edited by hand. A row goes back to its default, and so
//! do all of them at once.
//!
//! It also says what the application is, which is where people look for
//! that.
//!
//! Sources: gpui's keymap documentation (`Keystroke`, keystroke
//! interceptors).

use gpui_kit::component::{
    ActiveTheme, IconName, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use crate::{
    brand,
    keymap::{
        self, BINDINGS, Binding, Bound, KeyChange, Keys, POINTER_NOTES, Resolved, display_key,
    },
    notices::Notice,
};

/// What the sheet asks of the window.
#[derive(Debug, Clone)]
pub enum ShortcutsEvent {
    /// Put the sheet away.
    Close,
    /// The sheet wrote the user's file and read it again: the window need
    /// not do so a second time.
    Written,
}

impl EventEmitter<ShortcutsEvent> for ShortcutsView {}

/// What is being done to the keys of a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Editing {
    /// The row is open and what to do with its keys is being chosen.
    Choosing {
        /// The row of the table.
        index: usize,
    },
    /// The next key that is pressed is taken for the row.
    Recording {
        /// The row of the table.
        index: usize,
        /// Whether the key is one more, or takes the place of the others.
        add: bool,
        /// The key that was pressed, normalised.
        captured: Option<String>,
    },
}

impl Editing {
    fn index(&self) -> usize {
        match self {
            Self::Choosing { index } | Self::Recording { index, .. } => *index,
        }
    }
}

/// What a key does while a row is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Step {
    /// Wait for the key that is to be the row's.
    Record {
        /// As one more key.
        add: bool,
    },
    /// The row is to have no key.
    Unbind,
    /// The row is to have its default keys again.
    Reset,
    /// Leave the row as it is.
    Close,
    /// This is the key that was pressed for the row.
    Capture(String),
    /// Write down what was pressed.
    Save,
    /// Forget what was pressed and wait again.
    Again,
    /// The key does nothing here.
    Nothing,
}

/// What the key `key` (as the toolkit writes it) does to a row that is
/// open. While a key is waited for, whatever is pressed is that key: Enter
/// and Escape too, so that they can be given to an action. Once one was
/// pressed, Enter saves it, Escape leaves, Backspace waits again and any
/// other key takes its place.
pub(crate) fn step(editing: &Editing, key: &str, own: bool) -> Step {
    match editing {
        Editing::Choosing { .. } => match key {
            "enter" => Step::Record { add: false },
            "a" => Step::Record { add: true },
            "n" => Step::Unbind,
            "r" if own => Step::Reset,
            "escape" => Step::Close,
            _ => Step::Nothing,
        },
        Editing::Recording { captured: None, .. } => match keymap::normal_key(key) {
            Some(key) => Step::Capture(key),
            None => Step::Nothing,
        },
        Editing::Recording {
            captured: Some(_), ..
        } => match key {
            "enter" => Step::Save,
            "escape" => Step::Close,
            "backspace" => Step::Again,
            other => match keymap::normal_key(other) {
                Some(key) => Step::Capture(key),
                None => Step::Nothing,
            },
        },
    }
}

/// The keys a row has once `key` was saved for it: that key alone, or the
/// keys it has with that one behind them.
pub(crate) fn keys_after(resolved: &Resolved, index: usize, key: &str, add: bool) -> Vec<String> {
    let mut keys: Vec<String> = if add {
        resolved
            .bound
            .iter()
            .find(|bound| bound.index == index)
            .map(|bound| bound.keys.clone())
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    if !keys.iter().any(|held| held == key) {
        keys.push(key.to_owned());
    }
    keys
}

/// The row `by` rows on from `from` among the rows on show; the first or
/// the last when nothing is marked yet, or what was marked is not on show.
pub(crate) fn moved(shown: &[usize], from: Option<usize>, by: isize) -> Option<usize> {
    let at = from.and_then(|index| shown.iter().position(|row| *row == index));
    let next = match at {
        Some(at) => at
            .saturating_add_signed(by)
            .min(shown.len().saturating_sub(1)),
        None if by < 0 => shown.len().saturating_sub(1),
        None => 0,
    };
    shown.get(next).copied()
}

/// Whether a row is kept for a search: every word of it has to be in the
/// label, the id, the place or one of the keys as they are shown.
pub(crate) fn matches(binding: &Binding, keys: &[String], query: &str) -> bool {
    // As shown, and as written with words (`cmd+p` for macOS's `⌘P`).
    let shown: Vec<String> = keys
        .iter()
        .flat_map(|key| [display_key(key).to_lowercase(), key.replace('-', "+")])
        .collect();
    let haystack = format!(
        "{} {} {} {} {}",
        binding.label.text().to_lowercase(),
        binding.id,
        binding.place().to_lowercase(),
        binding.keywords.map_or("", |words| words.text()),
        shown.join(" ")
    );
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| haystack.contains(word))
}

/// The rows a search keeps, by their place in the table.
pub(crate) fn shown_rows(resolved: &Resolved, query: &str) -> Vec<usize> {
    resolved
        .bound
        .iter()
        .filter(|bound| {
            BINDINGS.get(bound.index).is_some_and(|binding| {
                let all: Vec<String> = bound.keys.iter().chain(&bound.lost).cloned().collect();
                matches(binding, &all, query)
            })
        })
        .map(|bound| bound.index)
        .collect()
}

/// The sheet of shortcuts.
pub struct ShortcutsView {
    focus_handle: FocusHandle,
    filter: Entity<InputState>,
    query: String,
    /// The row the arrows are on, by its place in the table.
    cursor: Option<usize>,
    /// The row that is open, and what is being done with it.
    editing: Option<Editing>,
    /// Why the last change was not written.
    failed: Option<String>,
    scroll: ScrollHandle,
    /// The cursor moved: bring its row into view when it is drawn.
    reveal: bool,
    _subscriptions: Vec<Subscription>,
}

impl ShortcutsView {
    /// Create the view.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| {
            InputState::new(window, cx).placeholder(say!(
                "shortcuts-search",
                key = crate::keymap::here("ctrl-p").replace('-', "+")
            ))
        });
        let typed = cx.subscribe_in(&filter, window, |this, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = state.read(cx).value().to_string();
                cx.notify();
            }
        });
        // Keys are looked at before what they are bound to runs: a key that
        // is pressed to be given to an action must not do what it does now.
        let view = cx.weak_entity();
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            let taken = view
                .update(cx, |view, cx| {
                    view.on_keystroke(&event.keystroke, window, cx)
                })
                .unwrap_or(false);
            if taken {
                cx.stop_propagation();
            }
        });
        Self {
            focus_handle: cx.focus_handle(),
            filter,
            query: String::new(),
            cursor: None,
            editing: None,
            failed: None,
            scroll: ScrollHandle::new(),
            reveal: false,
            _subscriptions: vec![typed, keys],
        }
    }

    /// Put the cursor in the search field.
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self.failed = None;
        self.filter.update(cx, |state, cx| state.focus(window, cx));
    }

    /// A key was pressed somewhere in the application. Whether it was the
    /// sheet's to take: the arrows and Enter while the search field has the
    /// focus, and every key while a row is open.
    fn on_keystroke(
        &mut self,
        keystroke: &Keystroke,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = keystroke.unparse();
        let Some(editing) = self.editing.clone() else {
            if !self.filter.read(cx).focus_handle(cx).is_focused(window) {
                return false;
            }
            let by = match key.as_str() {
                "down" => 1,
                "up" => -1,
                "enter" => {
                    let shown = shown_rows(&cx.global::<Keys>().resolved, &self.query);
                    return match self.cursor.filter(|index| shown.contains(index)) {
                        Some(index) => {
                            self.open_row(index, window, cx);
                            true
                        }
                        None => false,
                    };
                }
                _ => return false,
            };
            let shown = shown_rows(&cx.global::<Keys>().resolved, &self.query);
            self.cursor = moved(&shown, self.cursor, by);
            self.reveal = true;
            cx.notify();
            return true;
        };
        // Only for as long as the sheet is what keys go to.
        if !self.focus_handle.is_focused(window) {
            self.editing = None;
            cx.notify();
            return false;
        }
        let index = editing.index();
        let own = cx
            .global::<Keys>()
            .resolved
            .bound
            .iter()
            .any(|bound| bound.index == index && bound.own);
        match step(&editing, &key, own) {
            Step::Record { add } => self.record(index, add, cx),
            Step::Unbind => self.unbind(index, window, cx),
            Step::Reset => self.reset(index, window, cx),
            Step::Close => self.close_row(window, cx),
            Step::Capture(key) => {
                if let Editing::Recording { add, .. } = editing {
                    self.editing = Some(Editing::Recording {
                        index,
                        add,
                        captured: Some(key),
                    });
                    self.failed = None;
                    cx.notify();
                }
            }
            Step::Save => self.save(window, cx),
            Step::Again => {
                if let Editing::Recording { add, .. } = editing {
                    self.record(index, add, cx);
                }
            }
            Step::Nothing => {}
        }
        true
    }

    /// Open a row: what to do with its keys is asked.
    fn open_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.cursor = Some(index);
        self.editing = Some(Editing::Choosing { index });
        self.failed = None;
        self.reveal = true;
        // The keys are the sheet's from here on, not the search field's.
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// Put the open row away and go back to the search field.
    fn close_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editing = None;
        self.failed = None;
        self.filter.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    /// Wait for the key that is to be the row's.
    fn record(&mut self, index: usize, add: bool, cx: &mut Context<Self>) {
        self.editing = Some(Editing::Recording {
            index,
            add,
            captured: None,
        });
        self.failed = None;
        cx.notify();
    }

    /// Write changes into the user's file, read it again and say what was
    /// done. What cannot be written is said in the row, which stays open.
    fn write(
        &mut self,
        changes: Vec<KeyChange>,
        done: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(file) = cx.global::<Keys>().file.clone() else {
            self.failed = Some(say!("shortcuts-no-place").to_owned());
            cx.notify();
            return;
        };
        match keymap::edit_user_keys(&file, &changes) {
            Ok(()) => {
                keymap::reload(cx);
                cx.emit(ShortcutsEvent::Written);
                crate::notices::push(window, cx, Notice::success(done));
                self.close_row(window, cx);
            }
            Err(why) => {
                self.failed = Some(why);
                cx.notify();
            }
        }
    }

    /// Save the key that was pressed for the open row.
    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Editing::Recording {
            index,
            add,
            captured: Some(key),
        }) = self.editing.clone()
        else {
            return;
        };
        let Some(binding) = BINDINGS.get(index) else {
            return;
        };
        let resolved = &cx.global::<Keys>().resolved;
        let keys = keys_after(resolved, index, &key, add);
        let changes = keymap::changes_for(resolved, index, &keys);
        let done = say!(
            "shortcuts-saved",
            key = display_key(&key),
            count = keys.len(),
            action = binding.label.text()
        );
        self.write(changes, done, window, cx);
    }

    /// Leave the row without a key.
    fn unbind(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binding) = BINDINGS.get(index) else {
            return;
        };
        self.write(
            vec![(binding.id.to_owned(), Some(Vec::new()))],
            say!("shortcuts-unbound", action = binding.label.text()),
            window,
            cx,
        );
    }

    /// Give the row its default keys again.
    fn reset(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(binding) = BINDINGS.get(index) else {
            return;
        };
        self.write(
            vec![(binding.id.to_owned(), None)],
            say!(
                "shortcuts-reset-done",
                action = binding.label.text(),
                count = binding.keys.len()
            ),
            window,
            cx,
        );
    }

    /// Ask whether every key is to be the default again, and do it.
    pub fn confirm_reset_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let keys = cx.global::<Keys>();
        let own = keys.resolved.bound.iter().filter(|bound| bound.own).count();
        let Some(file) = keys.file.clone() else {
            return;
        };
        if own == 0 && keys.resolved.problems.is_empty() {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("shortcuts-all-default-already")),
            );
            return;
        }
        let view = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (view, file) = (view.clone(), file.clone());
            alert
                .title(say!("shortcuts-reset-all-title"))
                .description(say!(
                    "shortcuts-reset-all-description",
                    count = own,
                    file = file.display().to_string()
                ))
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let file = file.clone();
                    let _ = view.update(cx, |view, cx| {
                        match keymap::clear_user_keys(&file) {
                            Ok(()) => {
                                keymap::reload(cx);
                                cx.emit(ShortcutsEvent::Written);
                                view.editing = None;
                                view.failed = None;
                                crate::notices::push(
                                    window,
                                    cx,
                                    Notice::success(say!("shortcuts-all-default-again")),
                                );
                            }
                            Err(why) => {
                                crate::notices::push(window, cx, Notice::error(why));
                            }
                        }
                        cx.notify();
                    });
                    true
                })
        });
    }

    fn row(&self, binding: &Binding, bound: &Bound, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let index = bound.index;
        let marked = self.cursor == Some(index);
        let chip = |text: String, struck: bool| {
            div()
                .flex_none()
                .px_1p5()
                .h(px(20.))
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.secondary)
                .font_family(theme.mono_font_family.clone())
                .text_xs()
                .line_height(px(18.))
                .when(struck, |chip| {
                    chip.line_through().text_color(theme.muted_foreground)
                })
                .child(text)
        };
        h_flex()
            .id(("shortcut-row", index))
            .w_full()
            .min_h(px(28.))
            .px_3()
            .gap_3()
            .items_center()
            .cursor_pointer()
            .hover(|row| row.bg(theme.list_hover))
            .when(marked, |row| row.bg(theme.list_active))
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.editing.as_ref().map(Editing::index) == Some(index) {
                    this.close_row(window, cx);
                } else {
                    this.open_row(index, window, cx);
                }
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(binding.label.text()),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(190.))
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(binding.id),
            )
            .child(
                h_flex()
                    .flex_none()
                    .w(px(300.))
                    .gap_1()
                    .justify_end()
                    .flex_wrap()
                    .when(bound.own, |keys| {
                        keys.child(div().text_xs().text_color(theme.primary).child(
                            if bound.keys.is_empty() {
                                say!("shortcuts-yours-no-key")
                            } else {
                                say!("shortcuts-yours")
                            },
                        ))
                    })
                    .children(bound.keys.iter().map(|key| chip(display_key(key), false)))
                    .children(bound.lost.iter().map(|key| chip(display_key(key), true))),
            )
            .into_any_element()
    }

    /// What stands under the row that is open: what can be done with its
    /// keys, or the key that was pressed and what saving it would do.
    fn editor(
        &self,
        editing: &Editing,
        binding: &Binding,
        bound: &Bound,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let index = bound.index;
        let leave = |label: &'static str, cx: &mut Context<Self>| {
            Button::new("key-leave")
                .ghost()
                .small()
                .label(label)
                .on_click(cx.listener(|this, _, window, cx| this.close_row(window, cx)))
        };
        let mut panel = v_flex()
            .mx_3()
            .mb_1()
            .p_2()
            .gap_2()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.primary)
            .bg(theme.secondary);
        match editing {
            Editing::Choosing { .. } => {
                panel = panel
                    .child(
                        div()
                            .whitespace_normal()
                            .child(say!("shortcuts-choosing", action = binding.label.text())),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("key-new")
                                    .primary()
                                    .small()
                                    .label(say!("shortcuts-new-key"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.record(index, false, cx)
                                    })),
                            )
                            .child(
                                Button::new("key-add")
                                    .outline()
                                    .small()
                                    .label(say!("shortcuts-one-more-key"))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.record(index, true, cx)
                                    })),
                            )
                            .child(
                                Button::new("key-none")
                                    .outline()
                                    .small()
                                    .label(say!("shortcuts-no-key"))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.unbind(index, window, cx)
                                    })),
                            )
                            .when(bound.own, |row| {
                                row.child(
                                    Button::new("key-default")
                                        .outline()
                                        .small()
                                        .label(say!("shortcuts-default-again"))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.reset(index, window, cx)
                                        })),
                                )
                            })
                            .child(leave(say!("shortcuts-leave-esc"), cx)),
                    );
            }
            Editing::Recording {
                add,
                captured: None,
                ..
            } => {
                panel = panel
                    .child(
                        div()
                            .font_semibold()
                            .whitespace_normal()
                            .child(say!("shortcuts-press-now", action = binding.label.text())),
                    )
                    .child(
                        div()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(theme.muted_foreground)
                            .child(if *add {
                                say!("shortcuts-waiting-add")
                            } else {
                                say!("shortcuts-waiting-replace")
                            }),
                    )
                    .child(h_flex().child(leave(say!("shortcuts-leave"), cx)));
            }
            Editing::Recording {
                add,
                captured: Some(key),
                ..
            } => {
                let resolved = &cx.global::<Keys>().resolved;
                let check = keymap::check_key(resolved, index, key);
                let already = bound.keys.iter().any(|held| held == key);
                let conflict = !check.taken_from.is_empty();
                let mut said = check.said(key);
                if already {
                    said.insert(0, say!("shortcuts-already-this").to_owned());
                } else if !conflict {
                    said.insert(0, say!("shortcuts-free").to_owned());
                }
                panel = panel
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .flex_none()
                                    .px_2()
                                    .h(px(24.))
                                    .rounded(theme.radius)
                                    .border_1()
                                    .border_color(if conflict {
                                        theme.warning
                                    } else {
                                        theme.success
                                    })
                                    .bg(theme.background)
                                    .font_family(theme.mono_font_family.clone())
                                    .line_height(px(22.))
                                    .child(display_key(key)),
                            )
                            .child(div().flex_1().min_w_0().whitespace_normal().child(if *add {
                                say!("shortcuts-pressed-add", action = binding.label.text())
                            } else {
                                say!("shortcuts-pressed-replace", action = binding.label.text())
                            })),
                    )
                    .children(said.into_iter().enumerate().map(|(at, sentence)| {
                        div()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(if conflict && at == 0 {
                                theme.warning
                            } else {
                                theme.muted_foreground
                            })
                            .child(sentence)
                    }))
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("key-save")
                                    .primary()
                                    .small()
                                    .label(if conflict {
                                        say!("shortcuts-save-take")
                                    } else {
                                        say!("shortcuts-save")
                                    })
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                                    ),
                            )
                            .child(
                                Button::new("key-again")
                                    .outline()
                                    .small()
                                    .label(say!("shortcuts-press-again"))
                                    .on_click({
                                        let add = *add;
                                        cx.listener(move |this, _, _, cx| {
                                            this.record(index, add, cx)
                                        })
                                    }),
                            )
                            .child(leave(say!("shortcuts-leave-esc"), cx)),
                    );
            }
        }
        panel
            .when_some(self.failed.clone(), |panel, why| {
                panel.child(
                    div()
                        .text_xs()
                        .whitespace_normal()
                        .text_color(theme.danger)
                        .child(why),
                )
            })
            .into_any_element()
    }
}

impl Focusable for ShortcutsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ShortcutsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let keys = cx.global::<Keys>();
        let complaints = keys.complaints();
        let file = keys.file.as_ref().map(|file| file.display().to_string());
        let bound = keys.resolved.bound.clone();
        let own = bound.iter().filter(|bound| bound.own).count();
        let git = cx
            .try_global::<crate::gitexe::GitExecutable>()
            .and_then(|git| match (&git.version, &git.path) {
                (Some(version), Some(path)) => Some(say!("shortcuts-git-at", version, path)),
                (Some(version), None) => Some(say!("shortcuts-git", version)),
                _ => None,
            });
        let heading = |text: &'static str| {
            div()
                .px_3()
                .pt_3()
                .pb_1()
                .text_xs()
                .font_semibold()
                .text_color(theme.muted_foreground)
                .child(text.to_uppercase())
                .into_any_element()
        };

        // One list of everything that scrolls, so that a row can be brought
        // into view by its place in it. What this is comes first, before
        // how to use it.
        let mut list: Vec<AnyElement> = vec![
            h_flex()
                .px_3()
                .pt_3()
                .gap_3()
                .items_center()
                .child(brand::mark_image(theme.is_dark(), px(40.), window, cx))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .child(div().font_semibold().child(say!(
                            "shortcuts-product-version",
                            version = env!("CARGO_PKG_VERSION")
                        )))
                        .child(
                            div()
                                .text_xs()
                                .whitespace_normal()
                                .text_color(theme.muted_foreground)
                                .child(match git {
                                    Some(git) => say!("shortcuts-what-runs", git),
                                    None => say!("shortcuts-what").to_owned(),
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .whitespace_normal()
                                .text_color(theme.muted_foreground)
                                .child(say!("shortcuts-trademarks")),
                        ),
                )
                .into_any_element(),
        ];
        if !complaints.is_empty() {
            list.push(heading(say!("shortcuts-complaints")));
            list.extend(complaints.into_iter().map(|said| {
                div()
                    .px_3()
                    .py_0p5()
                    .whitespace_normal()
                    .text_color(theme.danger)
                    .child(said)
                    .into_any_element()
            }));
        }
        // Rows by the place they apply in, in the table's order.
        let mut shown = 0usize;
        let mut place: Option<&'static str> = None;
        let mut cursor_at = None;
        for bound in &bound {
            let Some(binding) = BINDINGS.get(bound.index) else {
                continue;
            };
            let all: Vec<String> = bound.keys.iter().chain(&bound.lost).cloned().collect();
            if !matches(binding, &all, &self.query) {
                continue;
            }
            shown += 1;
            if place != Some(binding.place()) {
                place = Some(binding.place());
                list.push(heading(binding.place()));
            }
            if self.cursor == Some(bound.index) {
                cursor_at = Some(list.len());
            }
            list.push(self.row(binding, bound, cx));
            if let Some(editing) = self
                .editing
                .clone()
                .filter(|editing| editing.index() == bound.index)
            {
                list.push(self.editor(&editing, binding, bound, cx));
            }
        }
        if std::mem::take(&mut self.reveal)
            && let Some(at) = cursor_at
        {
            self.scroll.scroll_to_item(at);
        }
        let query = self.query.to_lowercase();
        let notes: Vec<AnyElement> = POINTER_NOTES
            .iter()
            .filter(|(how, what)| {
                query.split_whitespace().all(|word| {
                    how.text().to_lowercase().contains(word)
                        || what.text().to_lowercase().contains(word)
                })
            })
            .map(|(how, what)| {
                h_flex()
                    .w_full()
                    .min_h(px(28.))
                    .px_3()
                    .gap_3()
                    .child(div().flex_1().min_w_0().truncate().child(what.text()))
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(how.text()),
                    )
                    .into_any_element()
            })
            .collect();
        if !notes.is_empty() {
            list.push(heading(say!("shortcuts-pointer")));
            list.extend(notes);
        }
        if shown == 0 && !self.query.trim().is_empty() {
            list.push(
                div()
                    .p_3()
                    .text_color(theme.muted_foreground)
                    .child(say!("shortcuts-nothing-found"))
                    .into_any_element(),
            );
        }
        list.push(
            div()
                .p_3()
                .text_xs()
                .whitespace_normal()
                .text_color(theme.muted_foreground)
                .child(match file {
                    Some(file) => say!(
                        "shortcuts-how-to-change",
                        file,
                        key = crate::keymap::here("ctrl-alt-p")
                    ),
                    None => say!("shortcuts-no-place").to_owned(),
                })
                .into_any_element(),
        );

        v_flex()
            .id("shortcuts")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .w_full()
                    .h(px(44.))
                    .flex_none()
                    .px_3()
                    .gap_3()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_none()
                            .font_semibold()
                            .child(say!("shortcuts-title")),
                    )
                    .child(div().flex_1().child(Input::new(&self.filter).small()))
                    .when(own > 0, |bar| {
                        bar.child(
                            Button::new("shortcuts-reset")
                                .outline()
                                .small()
                                .label(say!("shortcuts-reset-all"))
                                .tooltip(say!("shortcuts-reset-all-tooltip"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm_reset_all(window, cx)
                                })),
                        )
                    })
                    .child(
                        Button::new("shortcuts-close")
                            .accessibility_label(say!("shortcuts-close"))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ShortcutsEvent::Close))),
                    ),
            )
            .child(
                v_flex()
                    .id("shortcut-rows")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .text_sm()
                    .children(list),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{Editing, Step, keys_after, matches, moved, shown_rows, step};
    use crate::keymap::{BINDINGS, UserKeys, binding, resolve};

    #[test]
    fn rows_are_found_by_what_they_do_where_and_by_key() {
        let pull = binding("remote.pull").unwrap();
        let keys = vec!["ctrl-shift-down".to_owned()];
        assert!(matches(pull, &keys, ""));
        assert!(matches(pull, &keys, "pull"));
        assert!(matches(pull, &keys, "PULL branch"));
        assert!(matches(pull, &keys, "remote.pull"));
        assert!(matches(pull, &keys, "ctrl+shift"));
        assert!(matches(pull, &keys, "everywhere"));
        assert!(!matches(pull, &keys, "push"));
        assert!(!matches(pull, &keys, "pull stash"));
        let stage = binding("changes.stage").unwrap();
        assert!(matches(stage, &["s".to_owned()], "changes stage"));
        assert!(!matches(stage, &["s".to_owned()], "graph"));
    }

    #[test]
    fn the_arrows_walk_the_rows_a_search_keeps() {
        let resolved = resolve(&UserKeys::default());
        let all = shown_rows(&resolved, "");
        assert_eq!(all.len(), BINDINGS.len());
        let few = shown_rows(&resolved, "zoom");
        assert_eq!(few.len(), 3, "{few:?}");
        // From nothing: the first going down, the last going up.
        assert_eq!(moved(&few, None, 1), few.first().copied());
        assert_eq!(moved(&few, None, -1), few.last().copied());
        assert_eq!(moved(&few, few.first().copied(), 1), few.get(1).copied());
        // The ends hold.
        assert_eq!(moved(&few, few.last().copied(), 1), few.last().copied());
        assert_eq!(moved(&few, few.first().copied(), -1), few.first().copied());
        // A row that the search no longer keeps counts as none.
        assert_eq!(moved(&few, Some(usize::MAX), 1), few.first().copied());
        assert_eq!(moved(&[], Some(3), 1), None);
        assert_eq!(
            moved(&few, few.first().copied(), isize::MAX),
            few.last().copied()
        );
        assert_eq!(
            moved(&few, few.last().copied(), isize::MIN),
            few.first().copied()
        );
    }

    #[test]
    fn keys_do_what_the_open_row_says_they_do() {
        let choosing = Editing::Choosing { index: 0 };
        assert_eq!(step(&choosing, "enter", false), Step::Record { add: false });
        assert_eq!(step(&choosing, "a", false), Step::Record { add: true });
        assert_eq!(step(&choosing, "n", false), Step::Unbind);
        // The default is only gone back to from keys of the user's own.
        assert_eq!(step(&choosing, "r", false), Step::Nothing);
        assert_eq!(step(&choosing, "r", true), Step::Reset);
        assert_eq!(step(&choosing, "escape", true), Step::Close);
        assert_eq!(step(&choosing, "ctrl-p", true), Step::Nothing);

        // While a key is waited for, every key is that key.
        let waiting = Editing::Recording {
            index: 0,
            add: false,
            captured: None,
        };
        for key in ["enter", "escape", "backspace", "ctrl-p", "f9", "a"] {
            assert_eq!(step(&waiting, key, true), Step::Capture(key.to_owned()));
        }
        assert_eq!(
            step(&waiting, "shift-ctrl-p", true),
            Step::Capture("ctrl-shift-p".to_owned())
        );
        // What is no key is not taken.
        for odd in ["", "ctrl-", "shift", "\u{0}"] {
            assert_eq!(step(&waiting, odd, true), Step::Nothing, "{odd:?}");
        }

        let pressed = Editing::Recording {
            index: 0,
            add: false,
            captured: Some("ctrl-alt-p".to_owned()),
        };
        assert_eq!(step(&pressed, "enter", true), Step::Save);
        assert_eq!(step(&pressed, "escape", true), Step::Close);
        assert_eq!(step(&pressed, "backspace", true), Step::Again);
        assert_eq!(step(&pressed, "f9", true), Step::Capture("f9".to_owned()));
    }

    #[test]
    fn a_saved_key_replaces_or_joins_the_keys_there_are() {
        let resolved = resolve(&UserKeys::default());
        let redo = BINDINGS
            .iter()
            .position(|row| row.id == "redo.last")
            .unwrap();
        assert_eq!(keys_after(&resolved, redo, "f9", false), ["f9"]);
        assert_eq!(
            keys_after(&resolved, redo, "f9", true),
            ["ctrl-y", "ctrl-shift-z", "f9"]
        );
        // One it has already is not there twice.
        assert_eq!(
            keys_after(&resolved, redo, "ctrl-y", true),
            ["ctrl-y", "ctrl-shift-z"]
        );
        assert_eq!(keys_after(&resolved, usize::MAX, "f9", true), ["f9"]);
    }
}
