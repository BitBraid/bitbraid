//! One row of a settings page, and the controls that stand in one.
//!
//! Every setting is laid out the same way, on every page: the words that
//! name it right-aligned in a column that is a quarter of the form, the
//! control beside them, and what there is to explain under the control in
//! small print. The measures are the constants below: they hold at any
//! window width, since the column is a share of the form and the form has
//! a width it does not go beyond.
//!
//! A control writes to the preferences as it is changed. A field that is
//! typed into is filled again only when the setting was changed from
//! somewhere else: what is being typed is the setting already, as far as
//! the preferences can make it out, and is left as it was typed.
//!
//! Sources: gpui-kit documentation (setting, input, checkbox, button, popup
//! menu).

use gpui_kit::component::{
    ActiveTheme,
    button::Button,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    text::TextView,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use std::rc::Rc;

use crate::{
    literal,
    prefs::{Preferences, Prefs},
};

/// The widest a form gets.
pub(crate) const FORM_WIDTH: f32 = 1200.;
/// The share of the form that is the column of words, with the gap that
/// follows them.
pub(crate) const WORDS_SHARE: f32 = 0.25;
/// From where the words end to where the controls begin.
pub(crate) const WORDS_GAP: f32 = 31.;
/// A field or a list to choose from.
pub(crate) const CONTROL_WIDTH: f32 = 300.;
/// A field, a list to choose from, and the line a box to tick stands in.
pub(crate) const CONTROL_HEIGHT: f32 = 34.;
/// A button that acts, beside a field or under the last one.
pub(crate) const BUTTON_HEIGHT: f32 = 30.;
/// The widest an explanation gets.
pub(crate) const NOTE_WIDTH: f32 = 525.;
/// From a control to what is explained under it.
const NOTE_BEFORE: f32 = 8.;
/// What a row with an explanation leaves under it, on top of the gap
/// between any two rows: together 24 from an explanation's box to the
/// next row.
const NOTE_AFTER: f32 = 16.;
/// A line of the words.
const WORDS_LINE: f32 = 20.;
/// A line of an explanation.
const NOTE_LINE: f32 = 17.5;

/// What stands beside the words of a row.
pub(crate) struct Parts {
    /// The control, in a line as high as a field. None for a row that only
    /// says something.
    pub control: Option<AnyElement>,
    /// What is under it, in small print.
    pub under: Option<AnyElement>,
}

/// The row: the words, and beside them the control with what is under it.
pub(crate) fn row(words: SharedString, parts: Parts, cx: &App) -> Div {
    let muted = cx.theme().muted_foreground;
    let Parts { control, under } = parts;
    let explained = under.is_some();
    let stands_alone = control.is_none();
    h_flex()
        .w_full()
        .max_w(px(FORM_WIDTH))
        .items_start()
        .when(explained, |row| row.pb(px(NOTE_AFTER)))
        .child(
            // The first line of the words is level with the control,
            // however many lines they take.
            div()
                .w(relative(WORDS_SHARE))
                .flex_none()
                .min_h(px(CONTROL_HEIGHT))
                .pt(px((CONTROL_HEIGHT - WORDS_LINE) / 2.))
                .pr(px(WORDS_GAP))
                .text_sm()
                .line_height(px(WORDS_LINE))
                .text_right()
                .whitespace_normal()
                .child(words),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .children(control.map(|control| {
                    h_flex()
                        .min_h(px(CONTROL_HEIGHT))
                        .items_center()
                        .child(control)
                }))
                .children(under.map(|under| {
                    div()
                        .max_w(px(NOTE_WIDTH))
                        .pt(px(if stands_alone {
                            (CONTROL_HEIGHT - NOTE_LINE) / 2.
                        } else {
                            NOTE_BEFORE
                        }))
                        .text_xs()
                        .line_height(px(NOTE_LINE))
                        .text_color(muted)
                        .whitespace_normal()
                        .child(under)
                })),
        )
}

/// What there is to explain about a setting.
#[derive(Debug, Clone)]
pub(crate) enum Note {
    /// Nothing.
    None,
    /// A sentence or two.
    Plain(SharedString),
    /// The same with commands between backquotes, which are set apart in
    /// the monospace font.
    Marked(&'static str),
}

impl Note {
    /// A sentence or two.
    pub(crate) fn plain(text: impl Into<SharedString>) -> Self {
        Self::Plain(text.into())
    }

    fn element(&self, words: &SharedString) -> Option<AnyElement> {
        match self {
            Note::None => None,
            Note::Plain(text) => Some(div().child(text.clone()).into_any_element()),
            Note::Marked(text) => Some(
                TextView::markdown(SharedString::from(format!("note-{words}")), *text)
                    .into_any_element(),
            ),
        }
    }
}

/// What makes the control of a row and what is under it, each time the
/// section is drawn.
type MakeParts = Rc<dyn Fn(&mut Window, &mut App) -> Parts>;

/// A row of a section: its words, and what stands beside them.
#[derive(Clone)]
pub(crate) struct Item {
    words: SharedString,
    parts: MakeParts,
}

impl Item {
    /// Draw the row.
    pub(crate) fn render(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        let parts = (self.parts)(window, cx);
        row(self.words.clone(), parts, cx).into_any_element()
    }
}

/// A row, whatever stands in it.
pub(crate) fn item_of(
    words: impl Into<SharedString>,
    parts: impl Fn(&mut Window, &mut App) -> Parts + 'static,
) -> Item {
    Item {
        words: words.into(),
        parts: Rc::new(parts),
    }
}

/// A setting as a row: its words, its control, and what explains it.
fn item(
    words: impl Into<SharedString>,
    note: Note,
    control: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
) -> Item {
    let words: SharedString = words.into();
    let named = words.clone();
    item_of(words, move |window, cx| Parts {
        control: Some(control(window, cx)),
        under: note.element(&named),
    })
}

/// Change the preferences from a control, bring them into range, write
/// them, and show the change in every window.
fn write(cx: &mut App, change: impl FnOnce(&mut Preferences)) {
    let prefs = cx.global_mut::<Prefs>();
    change(&mut prefs.preferences);
    prefs.preferences.sanitize();
    prefs.save_preferences();
    cx.refresh_windows();
}

/// A setting that is on or off: a box to tick.
pub(crate) fn check(
    words: impl Into<SharedString>,
    note: Note,
    get: impl Fn(&Preferences) -> bool + 'static,
    set: impl Fn(&mut Preferences, bool) + Clone + 'static,
) -> Item {
    let words: SharedString = words.into();
    let id = SharedString::from(format!("check-{words}"));
    let name = words.clone();
    item(words, note, move |_, cx| {
        let set = set.clone();
        Checkbox::new(id.clone())
            .accessibility_label(name.clone())
            .checked(get(&cx.global::<Prefs>().preferences))
            .on_click(move |on: &bool, _, cx: &mut App| {
                let on = *on;
                write(cx, |preferences| set(preferences, on));
            })
            .into_any_element()
    })
}

/// A setting that is one of a few: a list to choose from, which shows the
/// one that is chosen. Each choice is the word the preferences keep and
/// what the list calls it.
pub(crate) fn select(
    words: impl Into<SharedString>,
    note: Note,
    choices: Vec<(SharedString, SharedString)>,
    get: fn(&App) -> SharedString,
    set: fn(SharedString, &mut App),
) -> Item {
    let words: SharedString = words.into();
    let id = SharedString::from(format!("select-{words}"));
    item(words, note, move |_, cx| {
        let chosen = get(cx);
        let shown = choices
            .iter()
            .find(|(word, _)| *word == chosen)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| chosen.clone());
        let listed = choices.clone();
        Button::new(id.clone())
            .outline()
            .label(shown)
            .dropdown_caret(true)
            .w(px(CONTROL_WIDTH))
            .h(px(CONTROL_HEIGHT))
            .justify_between()
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                listed.iter().fold(menu, |menu, (word, label)| {
                    let word = word.clone();
                    menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(word == chosen)
                            .on_click(move |_, _, cx| set(word.clone(), cx)),
                    )
                })
            })
            .into_any_element()
    })
}

/// What a list hands the choice it was given to.
pub(crate) type Pick = Rc<dyn Fn(SharedString, &mut Window, &mut App)>;

/// A list to choose from whose choices and choice are known when the
/// section is made: each choice is a word and what the list calls it.
pub(crate) fn choose(
    words: impl Into<SharedString>,
    note: Note,
    choices: Vec<(SharedString, SharedString)>,
    chosen: SharedString,
    pick: Pick,
) -> Item {
    let words: SharedString = words.into();
    let id = SharedString::from(format!("select-{words}"));
    item(words, note, move |_, _| {
        let shown = choices
            .iter()
            .find(|(word, _)| *word == chosen)
            .map(|(_, label)| label.clone())
            .unwrap_or_else(|| chosen.clone());
        let (listed, chosen, pick) = (choices.clone(), chosen.clone(), pick.clone());
        Button::new(id.clone())
            .outline()
            .label(shown)
            .dropdown_caret(true)
            .w(px(CONTROL_WIDTH))
            .h(px(CONTROL_HEIGHT))
            .justify_between()
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                listed.iter().fold(menu, |menu, (word, label)| {
                    let (word, pick) = (word.clone(), pick.clone());
                    menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(word == chosen)
                            .on_click(move |_, window, cx| pick(word.clone(), window, cx)),
                    )
                })
            })
            .into_any_element()
    })
}

/// A setting that is typed: how the preferences show it, and what they
/// make of what is typed.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Typed {
    /// The setting as a field shows it.
    pub show: fn(&Preferences) -> String,
    /// Take what a field holds.
    pub take: fn(&mut Preferences, &str),
}

impl Typed {
    /// Take what was typed. The answer is the setting as it is shown from
    /// then on: what the field holds, as far as the preferences go.
    pub(crate) fn typed(self, preferences: &mut Preferences, typed: &str) -> String {
        (self.take)(preferences, typed);
        preferences.sanitize();
        (self.show)(preferences)
    }
}

/// Whether a field has to be filled again from its setting: only when the
/// setting is no longer what the field last made of what was typed, which
/// is when it was changed from somewhere else. Putting the setting back
/// after every key would take away what the preferences tidy up (a blank
/// at the end, the comma after a name) the moment it is typed.
pub(crate) fn overtaken(held: &str, stored: &str) -> bool {
    held != stored
}

/// What a text field keeps between frames.
struct Field {
    input: Entity<InputState>,
    /// The setting as it was when the field last wrote it or was filled
    /// from it.
    held: String,
    _subscription: Subscription,
}

/// A field as wide and as high as every field of a form, with the words
/// of its row as its name for assistive technology.
pub(crate) fn field(
    id: SharedString,
    name: impl Into<SharedString>,
    input: &Entity<InputState>,
) -> Div {
    div().w(px(CONTROL_WIDTH)).flex_none().child(
        Input::new(input)
            .id(id)
            .aria_label(name)
            .min_h(px(CONTROL_HEIGHT))
            .w_full(),
    )
}

/// A setting that is typed into a field. A command line is typed in the
/// monospace font: there two hyphens are two, which the interface font
/// does not show.
pub(crate) fn text(
    words: impl Into<SharedString>,
    note: Note,
    typed: Typed,
    command: bool,
) -> Item {
    let words: SharedString = words.into();
    let key = SharedString::from(format!("field-{words}"));
    let name = words.clone();
    item(words, note, move |window, cx| {
        let stored = (typed.show)(&cx.global::<Prefs>().preferences);
        let state = window.use_keyed_state(key.clone(), cx, {
            let stored = stored.clone();
            move |window, cx| {
                let input = cx.new(|cx| InputState::new(window, cx).default_value(stored.clone()));
                let subscription = cx.subscribe(
                    &input,
                    move |field: &mut Field, input, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            let text = input.read(cx).value();
                            let prefs = cx.global_mut::<Prefs>();
                            field.held = typed.typed(&mut prefs.preferences, &text);
                            prefs.save_preferences();
                            cx.refresh_windows();
                        }
                    },
                );
                Field {
                    input,
                    held: stored,
                    _subscription: subscription,
                }
            }
        });
        let input = state.read(cx).input.clone();
        if overtaken(&state.read(cx).held, &stored) {
            state.update(cx, |field, _| field.held = stored.clone());
            input.update(cx, |input, cx| input.set_value(stored, window, cx));
        }
        let field = field(key.clone(), name.clone(), &input);
        if command {
            literal::command(cx).child(field).into_any_element()
        } else {
            field.into_any_element()
        }
    })
}

/// A setting that is a number: the least and the most it can be, and how
/// the preferences hold it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Number {
    /// The least it can be.
    pub min: f64,
    /// The most it can be.
    pub max: f64,
    /// The setting.
    pub get: fn(&Preferences) -> f64,
    /// Set it; what is given is in range.
    pub set: fn(&mut Preferences, f64),
}

/// What a number field makes of what it holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Counted {
    /// No number yet, or one under the least: more may be typed.
    Waiting,
    /// A number in range: it is the setting.
    Is(f64),
    /// One over the most. Nothing typed after it brings it back, so the
    /// setting is the most and the field says so at once.
    Most(f64),
}

/// Read what a number field holds.
pub(crate) fn counted(text: &str, min: f64, max: f64) -> Counted {
    match text.trim().parse::<f64>() {
        Ok(number) if !number.is_finite() => Counted::Waiting,
        Ok(number) if number > max => Counted::Most(max),
        Ok(number) if number < min => Counted::Waiting,
        Ok(number) => Counted::Is(number),
        Err(_) => Counted::Waiting,
    }
}

/// A number as its field shows it: without a fraction it does not have.
pub(crate) fn number_text(number: f64) -> String {
    format!("{number}")
}

/// What a number field keeps between frames.
struct NumberField {
    input: Entity<InputState>,
    /// The setting as it was when the field last wrote it or was filled
    /// from it.
    held: f64,
    _subscription: Subscription,
}

/// A setting that is a number, typed into a field like any other.
pub(crate) fn number(words: impl Into<SharedString>, note: Note, number: Number) -> Item {
    let words: SharedString = words.into();
    let key = SharedString::from(format!("number-{words}"));
    let name = words.clone();
    item(words, note, move |window, cx| {
        let stored = (number.get)(&cx.global::<Prefs>().preferences);
        let state = window.use_keyed_state(key.clone(), cx, move |window, cx| {
            let input = cx.new(|cx| InputState::new(window, cx).default_value(number_text(stored)));
            let subscription = cx.subscribe_in(
                &input,
                window,
                move |field: &mut NumberField, input, event: &InputEvent, window, cx| {
                    let text = input.read(cx).value();
                    match event {
                        InputEvent::Change => {
                            let read = counted(&text, number.min, number.max);
                            let (Counted::Is(value) | Counted::Most(value)) = read else {
                                return;
                            };
                            let prefs = cx.global_mut::<Prefs>();
                            (number.set)(&mut prefs.preferences, value);
                            prefs.preferences.sanitize();
                            prefs.save_preferences();
                            field.held = (number.get)(&prefs.preferences);
                            cx.refresh_windows();
                            if matches!(read, Counted::Most(_)) {
                                let most = number_text(field.held);
                                input.update(cx, |input, cx| input.set_value(most, window, cx));
                            }
                        }
                        // Left with something that is no setting: the field
                        // says what the setting is.
                        InputEvent::Blur => {
                            let shown = number_text(field.held);
                            if *text != *shown {
                                input.update(cx, |input, cx| input.set_value(shown, window, cx));
                            }
                        }
                        _ => {}
                    }
                },
            );
            NumberField {
                input,
                held: stored,
                _subscription: subscription,
            }
        });
        let input = state.read(cx).input.clone();
        if state.read(cx).held != stored {
            state.update(cx, |field, _| field.held = stored);
            input.update(cx, |input, cx| {
                input.set_value(number_text(stored), window, cx)
            });
        }
        field(key.clone(), name.clone(), &input).into_any_element()
    })
}

#[cfg(test)]
mod tests {
    use super::{Counted, Typed, counted, number_text, overtaken};
    use crate::prefs::Preferences;

    const NAMES: Typed = Typed {
        show: |preferences| preferences.protected_branches.join(", "),
        take: |preferences, typed| {
            preferences.protected_branches = crate::views::preferences::names_from(typed)
        },
    };

    /// Filled again from the setting after every key, the field for
    /// protected branches would lose the comma after a name, and the blank
    /// after it, as soon as they were typed: a second name could not be
    /// typed at all.
    #[test]
    fn a_list_of_names_can_be_typed_name_after_name() {
        let mut preferences = Preferences {
            protected_branches: vec!["main".to_owned()],
            ..Preferences::default()
        };
        let mut held = (NAMES.show)(&preferences);
        assert_eq!(held, "main");
        for typed in ["main,", "main, ", "main, t", "main, trunk"] {
            held = NAMES.typed(&mut preferences, typed);
            let stored = (NAMES.show)(&preferences);
            assert!(!overtaken(&held, &stored), "{typed:?} was put back");
        }
        assert_eq!(preferences.protected_branches, ["main", "trunk"]);
        // Changed from somewhere else: the field follows.
        preferences.protected_branches = vec!["release".to_owned()];
        assert!(overtaken(&held, &(NAMES.show)(&preferences)));
    }

    #[test]
    fn a_number_is_the_setting_only_when_it_is_in_range() {
        // On the way to 150, with 50 the least: nothing is set, and
        // nothing is put back.
        assert_eq!(counted("1", 50.0, 300.0), Counted::Waiting);
        assert_eq!(counted("15", 50.0, 300.0), Counted::Waiting);
        assert_eq!(counted("150", 50.0, 300.0), Counted::Is(150.0));
        assert_eq!(counted(" 90 ", 1.0, 3650.0), Counted::Is(90.0));
        // Past the most it is the most, at once.
        assert_eq!(counted("1500", 50.0, 300.0), Counted::Most(300.0));
        assert_eq!(counted("1e999", 50.0, 300.0), Counted::Waiting);
        for nothing in ["", "-", "x", "NaN", "inf"] {
            assert_eq!(counted(nothing, 1.0, 10.0), Counted::Waiting, "{nothing}");
        }
        assert_eq!(number_text(90.0), "90");
        assert_eq!(number_text(2.5), "2.5");
    }
}
