//! Asking in a bar: a name, a word or a yes, asked where the toolbar is.
//!
//! Most things are asked in a bar that takes the toolbar's place for as
//! long as the question stands, as wide as the window and as high as the
//! toolbar, its content centred as one row: what is asked (a word or a
//! sentence), the fields, then the buttons, the one that goes ahead first,
//! a destructive one in the colour of danger, and the one that gives up
//! last. The tab bar is dimmed meanwhile. Where there is no toolbar (a new
//! tab, a page of the window's own) the bar lies over the page's top. A
//! dialog stays only where a confirmation lists what would be lost.
//!
//! Any view hands its question to [`show`], built with [`Ask`]: what it
//! says, its fields, and for each button what to do with what was typed.
//! One question stands at a time: a new one takes the place of the one on
//! show, which counts as given up. Enter in a field (or, without fields,
//! anywhere in the bar) is the first button that goes ahead; Escape gives
//! up. The window's root draws the bar ([`render`]); nothing here runs
//! git.
//!
//! ```ignore
//! crate::ask::show(
//!     Ask::new("Name of the branch")
//!         .field("name", "")
//!         .go("Rename", move |texts, window, cx| { /* texts[0] */ }),
//!     window,
//!     cx,
//! );
//! ```
//!
//! Sources: none (original design).

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
};
use gpui_kit::*;

use crate::zoom::px;

/// Height of the bar: the toolbar's.
pub const HEIGHT: f32 = 48.0;
/// Width of a field in the bar.
const FIELD_WIDTH: f32 = 210.0;
/// Height of a field in the bar.
const FIELD_HEIGHT: f32 = 34.0;
/// Height of a button in the bar.
const BUTTON_HEIGHT: f32 = 22.0;
/// Space between two buttons, and between the fields and the first one.
const BUTTON_GAP: f32 = 12.0;
/// Space between what is asked and the first field.
const SAID_GAP: f32 = 9.0;

/// What a button does with what was typed: one text per field, trimmed.
pub type Run = Rc<dyn Fn(Vec<String>, &mut Window, &mut App)>;

/// What a button of the bar is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// Goes ahead with what was typed: waits until every field that must
    /// be filled in is, and every field that must change has.
    Go,
    /// Another choice, beside going ahead (clearing a name, pulling first).
    Other,
    /// Goes ahead with something that cannot be had back without undo: in
    /// the colour of danger.
    Danger,
    /// Gives up. Always the last button.
    Cancel,
}

/// A field of the bar.
#[derive(Debug, Clone)]
struct Field {
    placeholder: SharedString,
    initial: String,
    /// Going ahead waits until it holds something.
    required: bool,
    /// Going ahead waits until it holds something else than it started
    /// with.
    must_change: bool,
}

/// A button of the bar.
struct Choice {
    label: SharedString,
    tone: Tone,
    run: Option<Run>,
}

/// A question for the bar: what it says, its fields and its buttons.
pub struct Ask {
    said: SharedString,
    fields: Vec<Field>,
    choices: Vec<Choice>,
    cancel_label: SharedString,
    on_cancel: Option<Run>,
}

impl Ask {
    /// A question that says this: a word for a field ("Name"), or a
    /// sentence for a confirmation.
    pub fn new(said: impl Into<SharedString>) -> Self {
        Self {
            said: said.into(),
            fields: Vec::new(),
            choices: Vec::new(),
            cancel_label: say!("ask-cancel").into(),
            on_cancel: None,
        }
    }

    /// A field that has to be filled in: what it says while it is empty,
    /// and what it holds at first.
    pub fn field(mut self, placeholder: impl Into<SharedString>, initial: &str) -> Self {
        self.fields.push(Field {
            placeholder: placeholder.into(),
            initial: initial.to_owned(),
            required: true,
            must_change: false,
        });
        self
    }

    /// A field that may be left empty.
    pub fn optional_field(mut self, placeholder: impl Into<SharedString>, initial: &str) -> Self {
        self.fields.push(Field {
            placeholder: placeholder.into(),
            initial: initial.to_owned(),
            required: false,
            must_change: false,
        });
        self
    }

    /// The field added last may be empty, but going ahead waits until it
    /// holds something else than it started with (a new name for an old
    /// one).
    pub fn changed(mut self) -> Self {
        if let Some(field) = self.fields.last_mut() {
            field.required = false;
            field.must_change = true;
        }
        self
    }

    /// The button that goes ahead.
    pub fn go(
        mut self,
        label: impl Into<SharedString>,
        run: impl Fn(Vec<String>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.choices.push(Choice {
            label: label.into(),
            tone: Tone::Go,
            run: Some(Rc::new(run)),
        });
        self
    }

    /// Another choice, which runs with the fields as they are.
    pub fn other(
        mut self,
        label: impl Into<SharedString>,
        run: impl Fn(Vec<String>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.choices.push(Choice {
            label: label.into(),
            tone: Tone::Other,
            run: Some(Rc::new(run)),
        });
        self
    }

    /// A destructive choice, in the colour of danger.
    pub fn danger(
        mut self,
        label: impl Into<SharedString>,
        run: impl Fn(Vec<String>, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.choices.push(Choice {
            label: label.into(),
            tone: Tone::Danger,
            run: Some(Rc::new(run)),
        });
        self
    }

    /// What the button that gives up says ("Cancel" when not told).
    pub fn cancel_label(mut self, label: impl Into<SharedString>) -> Self {
        self.cancel_label = label.into();
        self
    }

    /// What to do when the question is given up: the cancel button,
    /// Escape, or another question taking its place.
    pub fn on_cancel(mut self, run: impl Fn(Vec<String>, &mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Some(Rc::new(run));
        self
    }
}

/// The question on show.
struct Shown {
    said: SharedString,
    fields: Vec<Field>,
    inputs: Vec<Entity<InputState>>,
    choices: Vec<Choice>,
    cancel_label: SharedString,
    on_cancel: Option<Run>,
    /// The bar's own, for when it has no field to hold the keys.
    focus: FocusHandle,
    /// Where the keys were before; they go back there.
    before: Option<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl Shown {
    fn texts(&self, cx: &App) -> Vec<String> {
        self.inputs
            .iter()
            .map(|input| input.read(cx).value().trim().to_owned())
            .collect()
    }

    /// Whether what is typed lets the bar go ahead.
    fn ready(&self, cx: &App) -> bool {
        self.fields.iter().zip(self.texts(cx)).all(|(field, text)| {
            (!field.required || !text.is_empty())
                && (!field.must_change || text != field.initial.trim())
        })
    }
}

/// The question the window is asking, if it is asking one.
#[derive(Default)]
pub struct Asking {
    shown: Option<Shown>,
}

impl Global for Asking {}

/// Ask this, in the bar. A question that was on show is given up.
pub fn show(ask: Ask, window: &mut Window, cx: &mut App) {
    give_up(window, cx);
    let Ask {
        said,
        fields,
        choices,
        cancel_label,
        on_cancel,
    } = ask;
    let mut subscriptions = Vec::new();
    let inputs: Vec<Entity<InputState>> = fields
        .iter()
        .map(|field| {
            let (placeholder, initial) = (field.placeholder.clone(), field.initial.clone());
            let input = cx.new(|cx| {
                let mut state = InputState::new(window, cx).placeholder(placeholder);
                state.set_value(initial, window, cx);
                state
            });
            subscriptions.push(window.subscribe(
                &input,
                cx,
                |_, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => go_ahead(window, cx),
                    // The buttons wait for what is typed.
                    InputEvent::Change => window.refresh(),
                    _ => {}
                },
            ));
            input
        })
        .collect();
    let focus = cx.focus_handle();
    let before = window.focused(cx);
    match inputs.first() {
        Some(first) => first.update(cx, |state, cx| state.focus(window, cx)),
        None => focus.focus(window, cx),
    }
    cx.default_global::<Asking>().shown = Some(Shown {
        said,
        fields,
        inputs,
        choices,
        cancel_label,
        on_cancel,
        focus,
        before,
        _subscriptions: subscriptions,
    });
    window.refresh();
}

/// Whether a question is on show.
pub fn is_open(cx: &App) -> bool {
    cx.try_global::<Asking>()
        .is_some_and(|asking| asking.shown.is_some())
}

/// What the question on show says, and what its fields hold.
pub fn on_show(cx: &App) -> Option<(String, Vec<String>)> {
    let shown = cx.try_global::<Asking>()?.shown.as_ref()?;
    Some((shown.said.to_string(), shown.texts(cx)))
}

/// Take the question away, and give the keys back to where they were.
fn take(window: &mut Window, cx: &mut App) -> Option<Shown> {
    if !cx.has_global::<Asking>() {
        return None;
    }
    let shown = cx.global_mut::<Asking>().shown.take()?;
    if let Some(before) = &shown.before {
        before.focus(window, cx);
    }
    window.refresh();
    Some(shown)
}

/// Give the question up, if one is on show.
pub fn give_up(window: &mut Window, cx: &mut App) {
    if let Some(shown) = take(window, cx) {
        let texts = shown.texts(cx);
        if let Some(run) = shown.on_cancel {
            run(texts, window, cx);
        }
    }
}

/// Press the button of this index, when it can be pressed.
fn press(index: usize, window: &mut Window, cx: &mut App) {
    let Some(shown) = cx.try_global::<Asking>().and_then(|a| a.shown.as_ref()) else {
        return;
    };
    let Some(choice) = shown.choices.get(index) else {
        return;
    };
    if choice.tone == Tone::Go && !shown.ready(cx) {
        return;
    }
    let run = choice.run.clone();
    if let Some(shown) = take(window, cx) {
        let texts = shown.texts(cx);
        if let Some(run) = run {
            run(texts, window, cx);
        }
    }
}

/// Go ahead: the first button that does, when what is typed lets it.
fn go_ahead(window: &mut Window, cx: &mut App) {
    let first = cx
        .try_global::<Asking>()
        .and_then(|asking| asking.shown.as_ref())
        .and_then(|shown| {
            shown
                .choices
                .iter()
                .position(|choice| matches!(choice.tone, Tone::Go | Tone::Danger))
        });
    if let Some(index) = first {
        press(index, window, cx);
    }
}

/// The bar, when a question is on show: as wide as the window and as high
/// as the toolbar, everything in it centred as one row.
pub fn render(window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let _ = window;
    let theme = cx.theme().clone();
    let shown = cx.try_global::<Asking>()?.shown.as_ref()?;
    let ready = shown.ready(cx);
    let fields = shown.inputs.iter().map(|input| {
        div()
            .w(px(FIELD_WIDTH))
            .h(px(FIELD_HEIGHT))
            .flex_none()
            .child(Input::new(input).h(px(FIELD_HEIGHT)))
    });
    let buttons = shown.choices.iter().enumerate().map(|(index, choice)| {
        let button = Button::new(("ask-choice", index))
            .outline()
            .xsmall()
            .h(px(BUTTON_HEIGHT))
            .label(choice.label.clone())
            .on_click(move |_, window, cx| press(index, window, cx));
        match choice.tone {
            Tone::Go => button.success().disabled(!ready),
            Tone::Other => button.info(),
            Tone::Danger => button.danger(),
            Tone::Cancel => button,
        }
        .into_any_element()
    });
    let cancel = Button::new("ask-cancel")
        .outline()
        .xsmall()
        .h(px(BUTTON_HEIGHT))
        .label(shown.cancel_label.clone())
        .on_click(|_, window, cx| give_up(window, cx));
    Some(
        h_flex()
            .id("ask-bar")
            .key_context("AskBar")
            .track_focus(&shown.focus)
            .w_full()
            .h(px(HEIGHT))
            .flex_none()
            .px_4()
            .justify_center()
            .items_center()
            .bg(theme.title_bar.blend(theme.primary.opacity(0.18)))
            .border_b_1()
            .border_color(theme.border)
            // Escape gives up, wherever in the bar the keys are; it is
            // the graph's otherwise.
            .on_action(|_: &crate::keymap::FocusGraph, window, cx| give_up(window, cx))
            // Without a field, Enter goes ahead as well.
            .on_key_down(|event, window, cx| {
                if event.keystroke.key == "enter" {
                    go_ahead(window, cx);
                }
            })
            .child(
                div()
                    .flex_none()
                    .min_w_0()
                    .mr(px(SAID_GAP))
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(shown.said.clone()),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(BUTTON_GAP))
                    .items_center()
                    .children(fields)
                    .children(buttons)
                    .child(cancel),
            )
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    use super::{Ask, Tone};

    #[test]
    fn a_question_keeps_its_fields_and_buttons_in_the_order_given() {
        let ask = Ask::new("Name")
            .field("name", "old")
            .changed()
            .go("Save", |_, _, _| {})
            .other("Clear", |_, _, _| {})
            .danger("Delete", |_, _, _| {})
            .cancel_label("Keep");
        assert_eq!(ask.said.as_ref(), "Name");
        assert_eq!(ask.fields.len(), 1);
        let field = &ask.fields[0];
        assert!(field.must_change && !field.required);
        let tones: Vec<Tone> = ask.choices.iter().map(|choice| choice.tone).collect();
        assert_eq!(tones, [Tone::Go, Tone::Other, Tone::Danger]);
        assert_eq!(ask.cancel_label.as_ref(), "Keep");
        // Changed with no field is nothing.
        assert!(Ask::new("Sure?").changed().fields.is_empty());
    }
}
