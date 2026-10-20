//! A dialog that asks more than a name: text fields, a name picked from a
//! list as it is typed, one of a few choices, switches, and a word about
//! what is typed while it is typed.
//!
//! The workspace's plain prompt ([`Workspace::prompt`]) is for one or two
//! texts; this is for the dialogs that carry options (a new branch that is
//! not checked out, what a stash takes, a signed tag, the parent of a merge
//! to pick against), and for those that ask several things at once, each
//! in a row with a word before it that says what it is (gitflow's branches
//! and prefixes, a submodule's address, folder and branch, a signer and
//! their key, where a clone goes and what it is of).
//!
//! A form belongs to whoever opens it: a repository's tab, or the window
//! itself, which asks what to clone and where a new repository goes
//! before there is any tab to ask in ([`FormHost`]). Either way it is the
//! same form with the same measures: the words before the rows
//! right-aligned in a column, the rows a fixed step apart.
//!
//! Sources: gpui-kit documentation (dialog, checkbox, radio group, input,
//! button).

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    radio::{Radio, RadioGroup},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use super::{PromptField, Workspace};

/// A switch of a form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Check {
    /// What it says.
    pub label: String,
    /// Whether it starts switched on.
    pub on: bool,
}

impl Check {
    pub(crate) fn new(label: impl Into<String>, on: bool) -> Self {
        Self {
            label: label.into(),
            on,
        }
    }
}

/// One of a few choices: each with what it is called and a word more.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Pick {
    /// The choices: label, and what there is to add in fainter print.
    pub options: Vec<(String, String)>,
    /// The one that starts chosen.
    pub picked: usize,
}

/// One of a list, or none, picked from a menu that drops from a button as
/// wide as a field: the `.gitignore` a new repository starts from.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Choose {
    /// The word before it.
    pub label: String,
    /// What the button says while none is chosen.
    pub none: String,
    /// What each is called.
    pub options: Vec<String>,
}

/// The large dialog a repository is made in: as wide as most windows
/// allow, from under the toolbar's place to near the window's bottom, a
/// title row with the cross that gives up, at the left a column of where
/// it is made from (there is one place, chosen), and at the right the
/// rows, every switch a row of its own, and one button that waits until
/// the form will do.
#[derive(Clone)]
pub(crate) struct Sheet {
    /// The first place of the column, which heads the rows as well.
    pub place: &'static str,
    pub icon: gpui_kit::assets::IconName,
    /// The places under it: each a view of its own that takes the rows'
    /// side when it is chosen (an account at a provider).
    pub others: Vec<Place>,
}

/// A place of the large dialog's column besides its first.
#[derive(Clone)]
pub(crate) struct Place {
    /// What it is known by in element ids: `form-place-<key>`.
    pub key: SharedString,
    /// What the column says.
    pub label: SharedString,
    pub icon: Icon,
    /// What is shown at the right while it is chosen; it has its own
    /// button that goes ahead.
    pub view: AnyView,
}

/// What a row that has no field shows, worked out from what the other
/// rows hold (in order, as they are answered): a full path from a folder
/// and a name.
pub(crate) type Derived = Rc<dyn Fn(&[String]) -> String>;

/// Names offered under the first field, narrowed by what is typed there.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Suggest {
    /// Every name on offer.
    pub names: Vec<String>,
    /// Only a name of the list will do: what is typed is completed to the
    /// best match, and with none the form stays open.
    pub strict: bool,
}

/// What there is to say about what is typed in the first field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// It cannot be used: the form stays open.
    Refuse(String),
    /// Worth knowing, and no reason to stop.
    Remark(String),
}

/// Looks at what is typed in the first field, with the switches as they
/// stand, and says what is wrong with it, if anything.
pub(crate) type Judge<H = Workspace> = Rc<dyn Fn(&H, &str, &[bool]) -> Option<Verdict>>;

/// Looks at everything that is answered so far, and says what is wrong
/// with it as a whole, if anything: two rows that cannot both hold what
/// they hold.
pub(crate) type Review<H = Workspace> = Rc<dyn Fn(&H, &Answers) -> Option<Verdict>>;

/// Something the form has its owner do while it is open: what the button
/// beside a row is for (choose a folder), and what follows the typing in
/// a row (a folder suggested from an address).
pub(crate) type Deed<H = Workspace> = Rc<dyn Fn(&mut H, &mut Window, &mut Context<H>)>;

/// One of a list of text fields, each with a word before it that says what
/// goes there. A form has as many as it needs.
pub(crate) struct Row<H = Workspace> {
    /// What is written before the field.
    pub label: String,
    /// Shown while the field is empty.
    pub placeholder: &'static str,
    /// What the field starts with.
    pub initial: String,
    /// The form cannot be confirmed while this is empty.
    pub required: bool,
    /// A line of small print under the field.
    pub note: Option<String>,
    /// A button at the right of the field: what it says and what it does.
    pub button: Option<(&'static str, Deed<H>)>,
    /// Shown only while this switch is on; while it is off the row
    /// answers nothing and need not be filled in.
    pub shown_by: Option<usize>,
    /// No field: a text worked out from the other rows, which is what the
    /// row answers.
    pub derived: Option<Derived>,
}

impl<H> Row<H> {
    /// A row that has to be filled in.
    pub(crate) fn required(label: impl Into<String>, initial: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            placeholder: "",
            initial: initial.into(),
            required: true,
            note: None,
            button: None,
            shown_by: None,
            derived: None,
        }
    }

    /// A row that may stay empty; `placeholder` says what that means.
    pub(crate) fn optional(
        label: impl Into<String>,
        initial: impl Into<String>,
        placeholder: &'static str,
    ) -> Self {
        Self {
            label: label.into(),
            placeholder,
            initial: initial.into(),
            required: false,
            note: None,
            button: None,
            shown_by: None,
            derived: None,
        }
    }

    /// A row without a field that shows what `derived` makes of the other
    /// rows.
    pub(crate) fn derived(
        label: impl Into<String>,
        derived: impl Fn(&[String]) -> String + 'static,
    ) -> Self {
        Self {
            derived: Some(Rc::new(derived)),
            ..Self::optional(label, "", "")
        }
    }
}

/// Most rows a form shows: more would not fit the window.
pub(crate) const MOST_ROWS: usize = 10;

/// What a confirmed form does with the answers.
type Action<H> = Rc<dyn Fn(&mut H, Answers, &mut Window, &mut Context<H>)>;

/// Everything a form asks.
pub(crate) struct Form<H = Workspace> {
    /// The heading.
    pub title: String,
    /// A sentence above everything else.
    pub lead: Option<String>,
    /// Up to two text fields.
    pub fields: Vec<PromptField>,
    /// Labelled text fields under those: as many as are needed, up to
    /// [`MOST_ROWS`].
    pub rows: Vec<Row<H>>,
    /// The row the keyboard starts in, when there are no fields above.
    pub begin: usize,
    /// What follows the typing in a row.
    pub typed: Option<Deed<H>>,
    /// What is said about all that is answered, while it is answered.
    pub review: Option<Review<H>>,
    /// Names offered for the first field.
    pub suggest: Option<Suggest>,
    /// What is said about the first field while it is typed.
    pub judge: Option<Judge<H>>,
    /// One of a list, or none.
    pub choose: Option<Choose>,
    /// One of a few choices.
    pub pick: Option<Pick>,
    /// Switches.
    pub checks: Vec<Check>,
    /// How wide the dialog is, when it is not the toolkit's usual width.
    pub width: Option<f32>,
    /// Drawn as the large dialog a repository is made in.
    pub sheet: Option<Sheet>,
    /// The button that confirms.
    pub ok_text: &'static str,
}

impl<H> Form<H> {
    /// A form with a heading and a confirming button, and nothing else yet.
    pub(crate) fn new(title: impl Into<String>, ok_text: &'static str) -> Self {
        Self {
            title: title.into(),
            lead: None,
            fields: Vec::new(),
            rows: Vec::new(),
            begin: 0,
            typed: None,
            review: None,
            suggest: None,
            judge: None,
            choose: None,
            pick: None,
            checks: Vec::new(),
            width: None,
            sheet: None,
            ok_text,
        }
    }
}

/// What was answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct Answers {
    /// What was typed, trimmed, one text per field.
    pub texts: Vec<String>,
    /// What was typed in the labelled rows, trimmed, one text per row.
    pub rows: Vec<String>,
    /// The switches, in the order they were given.
    pub checks: Vec<bool>,
    /// Which of the list was chosen, if one was.
    pub chosen: Option<usize>,
    /// Which choice was made; nought without choices.
    pub picked: usize,
}

impl Answers {
    /// The text of field `ix`, empty when there is no such field.
    pub(crate) fn text(&self, ix: usize) -> String {
        self.texts.get(ix).cloned().unwrap_or_default()
    }

    /// The text of row `ix`, empty when there is no such row.
    pub(crate) fn row(&self, ix: usize) -> String {
        self.rows.get(ix).cloned().unwrap_or_default()
    }

    /// Whether switch `ix` is on.
    pub(crate) fn check(&self, ix: usize) -> bool {
        self.checks.get(ix).copied().unwrap_or(false)
    }
}

/// The form that is open: what cannot live in the dialog's own closure.
pub(crate) struct Open<H> {
    required: Vec<bool>,
    rows_required: Vec<bool>,
    review: Option<Review<H>>,
    typed: Option<Deed<H>>,
    suggest: Option<Suggest>,
    judge: Option<Judge<H>>,
    pick: Option<Pick>,
    checks: Vec<Check>,
    rows_shown_by: Vec<Option<usize>>,
    rows_derived: Vec<Option<Derived>>,
    chosen: Option<usize>,
    /// Which place of the large dialog's column is chosen: nought for
    /// the form's own.
    place: usize,
    action: Action<H>,
}

impl<H> Open<H> {
    /// Whether row `ix` is on show: it is, unless a switch that is off
    /// hides it.
    fn shown(&self, ix: usize) -> bool {
        self.rows_shown_by
            .get(ix)
            .copied()
            .flatten()
            .is_none_or(|check| self.checks.get(check).is_some_and(|check| check.on))
    }

    /// Which rows have to be filled in as the form stands: those that
    /// have to be and are on show.
    fn rows_required_now(&self) -> Vec<bool> {
        self.rows_required
            .iter()
            .enumerate()
            .map(|(ix, required)| *required && self.shown(ix))
            .collect()
    }
}

/// What somebody who opens forms keeps for them: the text fields, which
/// are made once and used by one form after the other, and the form that
/// is open.
pub(crate) struct Forms<H> {
    /// The fields without a word before them: a repository's tab lends
    /// the two of its plain prompt.
    plain: Vec<Entity<InputState>>,
    /// The fields of the rows, made as forms need them.
    rows: Vec<Entity<InputState>>,
    open: Option<Open<H>>,
    subscriptions: Vec<Subscription>,
}

impl<H> Forms<H> {
    /// With these as the fields that have no word before them.
    pub(crate) fn new(plain: Vec<Entity<InputState>>) -> Self {
        Self {
            plain,
            rows: Vec::new(),
            open: None,
            subscriptions: Vec::new(),
        }
    }

    /// Whether a form is open, or was and has not been answered.
    pub(crate) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Forget the form that was open: something else asks now.
    pub(crate) fn close(&mut self) {
        self.open = None;
    }
}

/// Somebody who opens forms.
pub(crate) trait FormHost: Sized + 'static {
    /// What it keeps for them.
    fn forms(&self) -> &Forms<Self>;
    /// The same, to change.
    fn forms_mut(&mut self) -> &mut Forms<Self>;
}

/// How wide the column of words before the rows' fields is, in points;
/// the words stand right-aligned in it, so that the fields begin under
/// one another.
pub(crate) const LABELS: f32 = 145.0;
/// The space between that column and the fields.
const LABEL_GAP: f32 = 15.0;
/// How high a row is at least, and the space between two: 44 from one
/// row to the next.
const ROW_HEIGHT: f32 = 34.0;
const ROW_GAP: f32 = 10.0;
/// How far under its field a row's note begins.
const NOTE_LIFT: f32 = 4.0;
/// A row that holds a switch or a line of text, not a field: 32 from it
/// to the next.
const LINE_ROW_HEIGHT: f32 = 22.0;
/// Most of the menu that a list to choose one from drops.
const CHOOSE_MENU_HEIGHT: f32 = 320.0;

/// The large dialog a repository is made in (see [`Sheet`]): its width,
/// where it begins, how high it is at most, how far short of the window's
/// bottom it ends when the window is lower than that, and how high it is
/// at least: 852 high where there is room (at 1920x1080), down near the
/// bottom where there is not (at 1280x720: to the 16 the toolkit leaves).
const SHEET_WIDTH: f32 = 998.0;
const SHEET_TOP: f32 = 83.0;
const SHEET_MOST: f32 = 852.0;
const SHEET_BOTTOM: f32 = 16.0;
const SHEET_LEAST: f32 = 400.0;
/// Its title row, the title's place in it and the cross's.
const SHEET_TITLE: f32 = 52.0;
const SHEET_TITLE_LEFT: f32 = 20.0;
const SHEET_CLOSE_RIGHT: f32 = 16.0;
/// The column of places at the left, and a place in it.
const SHEET_PLACES: f32 = 249.0;
const SHEET_PLACE_HEIGHT: f32 = 40.0;
const SHEET_PLACE_SIDE: f32 = 11.0;
const SHEET_PLACE_GAP: f32 = 12.0;
/// The rows' side of it: from its edges, the heading over the rows and
/// the space under that.
const SHEET_PANE_LEFT: f32 = 11.0;
const SHEET_PANE_RIGHT: f32 = 20.0;
const SHEET_PANE_TOP: f32 = 10.0;
const SHEET_HEADING: f32 = 24.0;
const SHEET_HEADING_GAP: f32 = 11.0;
/// The button that goes ahead, under the rows, and the one beside a
/// field.
const SHEET_BUTTON_HEIGHT: f32 = 31.0;
const SHEET_BUTTON_GAP: f32 = 11.0;
const BUTTON_GAP: f32 = 11.0;

/// A row of a form: the word for it right-aligned in the column at the
/// left, what is filled in beside it. A row without a word is a note, a
/// choice or a switch that stands under the fields.
fn labelled(label: impl Into<SharedString>, muted: Hsla, field: impl IntoElement) -> Div {
    h_flex()
        .w_full()
        .min_h(px(ROW_HEIGHT))
        .gap(px(LABEL_GAP))
        .items_center()
        .child(
            div()
                .w(px(LABELS))
                .flex_none()
                .text_right()
                .text_sm()
                .whitespace_normal()
                .text_color(muted)
                .child(label.into()),
        )
        .child(div().flex_1().min_w_0().child(field))
}

/// What stands under a row and belongs to it, or to the rows as a whole:
/// a note, what is said about the answers, a choice. It begins where the
/// fields begin and is as high as it is, no more.
fn beneath(field: impl IntoElement) -> Div {
    h_flex()
        .w_full()
        .gap(px(LABEL_GAP))
        .items_start()
        .child(div().w(px(LABELS)).flex_none())
        .child(div().flex_1().min_w_0().child(field))
}

/// A line of small print in a form.
fn small_print(text: impl Into<SharedString>, muted: Hsla) -> Div {
    div()
        .text_xs()
        .whitespace_normal()
        .text_color(muted)
        .child(text.into())
}

/// Most names offered at a time.
const OFFERED: usize = 8;

/// The names that go with what is typed, best first, and how many more
/// there are: one that is what was typed, then those that begin with it,
/// then those whose last part after a `/` does, then those that have
/// every word of it somewhere, whatever the case. With nothing typed, the
/// first of the list.
pub(crate) fn suggestions(names: &[String], typed: &str, limit: usize) -> (Vec<String>, usize) {
    let typed = typed.trim().to_lowercase();
    let words: Vec<&str> = typed.split_whitespace().collect();
    let mut ranked: Vec<(u8, &String)> = names
        .iter()
        .filter_map(|name| {
            let lower = name.to_lowercase();
            let rank = if lower == typed {
                0
            } else if lower.starts_with(&typed) {
                1
            } else if lower
                .rsplit('/')
                .next()
                .is_some_and(|last| last.starts_with(&typed))
            {
                2
            } else if words.iter().all(|word| lower.contains(word)) {
                3
            } else {
                return None;
            };
            Some((rank, name))
        })
        .collect();
    // Stable: names of one rank stay in the order they were given.
    ranked.sort_by_key(|(rank, _)| *rank);
    let more = ranked.len().saturating_sub(limit);
    (
        ranked
            .into_iter()
            .take(limit)
            .map(|(_, name)| name.clone())
            .collect(),
        more,
    )
}

/// The choice after an arrow key: one further down or up, and no further
/// than the ends.
pub(crate) fn stepped(picked: usize, down: bool, last: usize) -> usize {
    if down {
        picked.saturating_add(1).min(last)
    } else {
        picked.saturating_sub(1).min(last)
    }
}

/// What a strict list makes of what was typed: the name itself when it is
/// one of the list, else the best match, else nothing.
pub(crate) fn completed(names: &[String], typed: &str) -> Option<String> {
    if names.iter().any(|name| name == typed) {
        return Some(typed.to_owned());
    }
    if typed.trim().is_empty() {
        return None;
    }
    suggestions(names, typed, 1).0.into_iter().next()
}

/// The dialog is drawn again as a field without a word before it is typed
/// in: what is said about the text, and the names on offer, follow it.
pub(crate) fn follow_typing<H: FormHost>(host: &mut H, window: &mut Window, cx: &mut Context<H>) {
    for field in host.forms().plain.clone() {
        let typed = cx.subscribe_in(&field, window, |this, _, event: &InputEvent, window, _| {
            if matches!(event, InputEvent::Change) && this.forms().is_open() {
                window.refresh();
            }
        });
        host.forms_mut().subscriptions.push(typed);
    }
}

/// The fields of `count` rows: kept from one form to the next, and made
/// when a form has more rows than any before it. Enter in one confirms
/// the form, and what is said about the answers follows the typing.
fn row_fields<H: FormHost>(
    host: &mut H,
    count: usize,
    window: &mut Window,
    cx: &mut Context<H>,
) -> Vec<Entity<InputState>> {
    while host.forms().rows.len() < count.min(MOST_ROWS) {
        let field = cx.new(|cx| InputState::new(window, cx));
        let typed = cx.subscribe_in(
            &field,
            window,
            |this: &mut H, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => {
                    if this.forms().is_open() && confirm(this, window, cx) {
                        window.close_dialog(cx);
                    }
                }
                InputEvent::Change if this.forms().is_open() => {
                    let typed = this
                        .forms()
                        .open
                        .as_ref()
                        .and_then(|open| open.typed.clone());
                    if let Some(typed) = typed {
                        typed(this, window, cx);
                    }
                    window.refresh();
                }
                _ => {}
            },
        );
        let forms = host.forms_mut();
        forms.subscriptions.push(typed);
        forms.rows.push(field);
    }
    host.forms().rows.iter().take(count).cloned().collect()
}

/// What the labelled rows of the open form hold right now, in order;
/// nothing without such a form.
pub(crate) fn row_texts<H: FormHost>(host: &H, cx: &App) -> Vec<String> {
    host.forms()
        .open
        .as_ref()
        .map(|open| answers(host, open, cx).rows)
        .unwrap_or_default()
}

/// Put a text into row `ix` of the open form: what a button beside the
/// row chose, or what follows from another row.
pub(crate) fn set_row<H: FormHost>(
    host: &mut H,
    ix: usize,
    text: impl Into<SharedString>,
    window: &mut Window,
    cx: &mut Context<H>,
) {
    if !host.forms().is_open() {
        return;
    }
    if let Some(field) = host.forms().rows.get(ix).cloned() {
        let text = text.into();
        field.update(cx, |state, cx| state.set_value(text, window, cx));
        window.refresh();
    }
}

/// What the open form holds in its fields and rows right now, with its
/// switches and its choice.
fn answers<H: FormHost>(host: &H, open: &Open<H>, cx: &App) -> Answers {
    let read = |field: &Entity<InputState>| field.read(cx).value().trim().to_owned();
    let forms = host.forms();
    // A row a switch hides answers nothing; one without a field answers
    // what it shows.
    let mut rows: Vec<String> = forms
        .rows
        .iter()
        .take(open.rows_required.len())
        .enumerate()
        .map(|(ix, field)| {
            if open.shown(ix) {
                read(field)
            } else {
                String::new()
            }
        })
        .collect();
    for (ix, derived) in open.rows_derived.iter().enumerate() {
        if let Some(derived) = derived {
            let text = derived(&rows);
            if let Some(row) = rows.get_mut(ix) {
                *row = text;
            }
        }
    }
    Answers {
        texts: forms
            .plain
            .iter()
            .take(open.required.len())
            .map(read)
            .collect(),
        rows,
        checks: open.checks.iter().map(|check| check.on).collect(),
        chosen: open.chosen,
        picked: open.pick.as_ref().map_or(0, |pick| pick.picked),
    }
}

/// A row as the dialog draws it: its word, its field, and what is beside,
/// under and over it.
struct Drawn<H> {
    label: String,
    input: Entity<InputState>,
    note: Option<String>,
    button: Option<(&'static str, Deed<H>)>,
    shown_by: Option<usize>,
    derived: bool,
}

/// Open a form. `on_ok` gets the answers once they will do.
pub(crate) fn open<H: FormHost>(
    host: &mut H,
    form: Form<H>,
    on_ok: impl Fn(&mut H, Answers, &mut Window, &mut Context<H>) + 'static,
    window: &mut Window,
    cx: &mut Context<H>,
) {
    let Form {
        title,
        lead,
        fields,
        rows,
        begin,
        typed,
        review,
        suggest,
        judge,
        choose,
        pick,
        checks,
        width,
        sheet,
        ok_text,
    } = form;
    let plain = host.forms().plain.clone();
    let required: Vec<bool> = fields
        .iter()
        .take(plain.len())
        .map(|field| field.required)
        .collect();
    let inputs: Vec<Entity<InputState>> = plain
        .iter()
        .zip(fields)
        .map(|(input, field)| {
            input.update(cx, |state, cx| {
                state.set_placeholder(field.placeholder, window, cx);
                state.set_value(field.initial, window, cx);
            });
            input.clone()
        })
        .collect();
    // No form is open while its fields are filled in: what follows the
    // typing is not for the texts a form starts with.
    host.forms_mut().open = None;
    let row_inputs = row_fields(host, rows.len(), window, cx);
    for (input, row) in row_inputs.iter().zip(&rows) {
        input.update(cx, |state, cx| {
            state.set_placeholder(row.placeholder, window, cx);
            state.set_value(row.initial.clone(), window, cx);
        });
    }
    let rows_required: Vec<bool> = rows
        .iter()
        .take(row_inputs.len())
        .map(|row| row.required && row.derived.is_none())
        .collect();
    let rows_shown_by: Vec<Option<usize>> = rows.iter().map(|row| row.shown_by).collect();
    let rows_derived: Vec<Option<Derived>> = rows.iter().map(|row| row.derived.clone()).collect();
    let drawn: Vec<Drawn<H>> = rows
        .into_iter()
        .zip(row_inputs)
        .map(|(row, input)| Drawn {
            derived: row.derived.is_some(),
            label: row.label,
            input,
            note: row.note,
            button: row.button,
            shown_by: row.shown_by,
        })
        .collect();
    let has_rows = !drawn.is_empty() || choose.is_some() || sheet.is_some();
    // The keyboard starts in the first thing there is to type in.
    let start = inputs
        .first()
        .or_else(|| drawn.get(begin).or(drawn.first()).map(|row| &row.input))
        .cloned();
    host.forms_mut().open = Some(Open {
        required,
        rows_required,
        review,
        typed,
        suggest,
        judge,
        pick,
        checks,
        rows_shown_by,
        rows_derived,
        chosen: None,
        place: 0,
        action: Rc::new(on_ok),
    });
    let owner = cx.entity().downgrade();
    let first = inputs.first().cloned();
    window.open_dialog(cx, move |dialog, window, cx| {
        let theme = cx.theme();
        let (muted, danger) = (theme.muted_foreground, theme.danger);
        let (for_ok, for_enter) = (owner.clone(), owner.clone());
        // What the open form holds right now.
        let typed = first
            .as_ref()
            .map(|first| first.read(cx).value().to_string())
            .unwrap_or_default();
        let state = owner.upgrade().and_then(|owner| {
            let this = owner.read(cx);
            let open = this.forms().open.as_ref()?;
            let switches: Vec<bool> = open.checks.iter().map(|check| check.on).collect();
            let verdict = open
                .judge
                .as_ref()
                .and_then(|judge| judge(this, typed.trim(), &switches));
            // A name of the list in the field is a choice made, not a
            // search: the others stay on offer around it.
            let offered = open.suggest.as_ref().map(|suggest| {
                let chosen = suggest.names.iter().any(|name| name == typed.trim());
                let query = if chosen { "" } else { typed.as_str() };
                let (names, more) = suggestions(&suggest.names, query, OFFERED);
                (names, more, chosen)
            });
            let answered = answers(this, open, cx);
            let reviewed = open
                .review
                .as_ref()
                .and_then(|review| review(this, &answered));
            Some(Now {
                place: open.place,
                checks: open.checks.clone(),
                pick: open.pick.clone(),
                chosen: open.chosen,
                verdict,
                offered,
                reviewed,
                rows: answered.rows,
                sendable: ready(this, open, cx).is_some(),
            })
        });
        let Now {
            place: chosen_place,
            checks,
            pick,
            chosen,
            verdict,
            offered,
            reviewed,
            rows: answered_rows,
            sendable,
        } = state.unwrap_or_default();
        let said = |verdict: Verdict| {
            let (text, colour) = match verdict {
                Verdict::Refuse(text) => (text, danger),
                Verdict::Remark(text) => (text, muted),
            };
            div()
                .text_xs()
                .whitespace_normal()
                .text_color(colour)
                .child(text)
        };
        // Under rows, what has no word of its own stands in the column
        // of the fields.
        let under = |element: AnyElement| -> AnyElement {
            if has_rows {
                beneath(element).into_any_element()
            } else {
                element
            }
        };
        // A switch, which in the large dialog is a row of its own: its
        // words in the column at the left, the box where the fields are.
        let switch = |ix: usize, check: &Check, in_sheet: bool| -> AnyElement {
            let for_check = owner.clone();
            let box_ = Checkbox::new(("form-check", ix))
                .accessibility_label(check.label.clone())
                .checked(check.on)
                .when(!in_sheet, |box_| box_.label(check.label.clone()))
                .on_click(move |on, window, cx| {
                    let on = *on;
                    let _ = for_check.update(cx, |this, _| {
                        if let Some(check) = this
                            .forms_mut()
                            .open
                            .as_mut()
                            .and_then(|open| open.checks.get_mut(ix))
                        {
                            check.on = on;
                        }
                    });
                    window.refresh();
                });
            if in_sheet {
                labelled(check.label.clone(), muted, box_)
                    .min_h(px(LINE_ROW_HEIGHT))
                    .into_any_element()
            } else {
                under(box_.into_any_element())
            }
        };
        let choose_row = choose.as_ref().map(|choose| {
            let says = chosen
                .and_then(|ix| choose.options.get(ix))
                .cloned()
                .unwrap_or_else(|| choose.none.clone());
            let (options, none, for_menu) =
                (choose.options.clone(), choose.none.clone(), owner.clone());
            let menu_button = Button::new("form-choose")
                .outline()
                .w_full()
                .h(px(ROW_HEIGHT))
                .dropdown_caret(true)
                .label(says)
                .dropdown_menu(move |menu, _, _| {
                    let entry = |ix: Option<usize>, label: String| {
                        let owner = for_menu.clone();
                        PopupMenuItem::new(label).checked(chosen == ix).on_click(
                            move |_, window, cx| {
                                let _ = owner.update(cx, |this, _| {
                                    if let Some(open) = this.forms_mut().open.as_mut() {
                                        open.chosen = ix;
                                    }
                                });
                                window.refresh();
                            },
                        )
                    };
                    options.iter().enumerate().fold(
                        menu.scrollable(true)
                            .max_h(px(CHOOSE_MENU_HEIGHT))
                            .item(entry(None, none.clone())),
                        |menu, (ix, name)| menu.item(entry(Some(ix), name.clone())),
                    )
                });
            labelled(choose.label.clone(), muted, menu_button)
        });
        // A row's field, with the button beside it when it has one.
        let row_field = |ix: usize, row: &Drawn<H>| -> AnyElement {
            match &row.button {
                Some((says, deed)) => {
                    let (for_deed, deed) = (owner.clone(), deed.clone());
                    h_flex()
                        .gap(px(if sheet.is_some() { BUTTON_GAP } else { 8. }))
                        .child(div().flex_1().min_w_0().child(Input::new(&row.input)))
                        .child(
                            Button::new(("form-row-button", ix))
                                .outline()
                                .label(*says)
                                .on_click(move |_, window, cx| {
                                    let _ = for_deed.update(cx, |this, cx| deed(this, window, cx));
                                }),
                        )
                        .into_any_element()
                }
                None => Input::new(&row.input).into_any_element(),
            }
        };
        let mut body = v_flex().gap(px(if has_rows { ROW_GAP } else { 8.0 }));

        if let Some(sheet) = &sheet {
            // The rows, every switch where the rows it shows begin (or
            // after the rows), and the button under them all.
            body = body.child(
                div()
                    .flex_none()
                    .h(px(SHEET_HEADING))
                    .mb(px(SHEET_HEADING_GAP))
                    .text_lg()
                    .line_height(px(SHEET_HEADING))
                    .child(sheet.place),
            );
            let mut placed = vec![false; checks.len()];
            for (ix, row) in drawn.iter().enumerate() {
                if let Some(by) = row.shown_by
                    && let Some(check) = checks.get(by)
                    && placed.get(by) == Some(&false)
                {
                    body = body.child(switch(by, check, true));
                    if let Some(done) = placed.get_mut(by) {
                        *done = true;
                    }
                }
                if !row
                    .shown_by
                    .is_none_or(|by| checks.get(by).is_some_and(|check| check.on))
                {
                    continue;
                }
                if row.derived {
                    let shows = answered_rows.get(ix).cloned().unwrap_or_default();
                    body = body.child(
                        labelled(
                            row.label.clone(),
                            muted,
                            div().text_sm().truncate().child(shows),
                        )
                        .min_h(px(LINE_ROW_HEIGHT))
                        .id(("form-row", ix))
                        .test_support(),
                    );
                    continue;
                }
                body = body.child(
                    labelled(row.label.clone(), muted, row_field(ix, row))
                        .id(("form-row", ix))
                        .test_support(),
                );
                if let Some(note) = &row.note {
                    body = body.child(
                        beneath(small_print(note.clone(), muted)).mt(px(NOTE_LIFT - ROW_GAP)),
                    );
                }
            }
            if let Some(reviewed) = reviewed {
                body = body.child(under(said(reviewed).into_any_element()));
            }
            body = body.children(choose_row);
            for (ix, check) in checks.iter().enumerate() {
                if placed.get(ix) != Some(&true) {
                    body = body.child(switch(ix, check, true));
                }
            }
            let button = Button::new("form-ok")
                .primary()
                .h(px(SHEET_BUTTON_HEIGHT))
                .label(ok_text)
                .disabled(!sendable)
                .on_click(move |_, window, cx| {
                    let done = for_ok
                        .update(cx, |this, cx| confirm(this, window, cx))
                        .unwrap_or(true);
                    if done {
                        window.close_dialog(cx);
                    }
                });
            body = body.child(
                h_flex()
                    .mt(px(SHEET_BUTTON_GAP))
                    .justify_end()
                    .child(button),
            );
            let height = (window.viewport_size().height.as_f32() - SHEET_TOP - SHEET_BOTTOM)
                .clamp(SHEET_LEAST, SHEET_MOST);
            let theme = cx.theme();
            let title_row = h_flex()
                .flex_none()
                .h(px(SHEET_TITLE))
                .pl(px(SHEET_TITLE_LEFT))
                .pr(px(SHEET_CLOSE_RIGHT))
                .items_center()
                .justify_between()
                .rounded_t(theme.radius_lg)
                .bg(theme.sidebar)
                .border_b_1()
                .border_color(theme.border)
                .child(div().text_lg().child(title.clone()))
                .child(
                    Button::new("form-close")
                        .accessibility_label(say!("workspace-close"))
                        .ghost()
                        .small()
                        .icon(IconName::Close)
                        .tooltip(say!("workspace-close"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                );
            // A place of the column: the form's own (nought) or another.
            let place_row = |ix: usize, id: ElementId, icon: Icon, label: SharedString| {
                let for_place = owner.clone();
                h_flex()
                    .id(id)
                    .h(px(SHEET_PLACE_HEIGHT))
                    .px(px(SHEET_PLACE_SIDE))
                    .gap(px(SHEET_PLACE_GAP))
                    .items_center()
                    .text_sm()
                    .when(ix == chosen_place, |row| row.bg(theme.list_active))
                    .when(ix != chosen_place, |row| {
                        row.cursor_pointer().hover(|row| row.bg(theme.list_hover))
                    })
                    .child(icon.size(px(16.)).flex_none())
                    .child(div().min_w_0().truncate().child(label))
                    .on_click(move |_, window, cx| {
                        let _ = for_place.update(cx, |this, _| {
                            if let Some(open) = this.forms_mut().open.as_mut() {
                                open.place = ix;
                            }
                        });
                        window.refresh();
                    })
                    .test_support()
            };
            let places = v_flex()
                .flex_none()
                .w(px(SHEET_PLACES))
                .h_full()
                .bg(theme.sidebar)
                .border_r_1()
                .border_color(theme.border)
                .child(place_row(
                    0,
                    "form-place".into(),
                    Icon::new(sheet.icon),
                    sheet.place.into(),
                ))
                .children(sheet.others.iter().enumerate().map(|(ix, other)| {
                    place_row(
                        ix.saturating_add(1),
                        SharedString::from(format!("form-place-{}", other.key)).into(),
                        other.icon.clone(),
                        other.label.clone(),
                    )
                }));
            let other = chosen_place
                .checked_sub(1)
                .and_then(|ix| sheet.others.get(ix))
                .map(|other| other.view.clone());
            let pane = match other {
                Some(view) => div()
                    .id("form-pane")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .child(view)
                    .into_any_element(),
                None => div()
                    .id("form-pane")
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .overflow_y_scroll()
                    .pl(px(SHEET_PANE_LEFT))
                    .pr(px(SHEET_PANE_RIGHT))
                    .pt(px(SHEET_PANE_TOP))
                    .pb(px(SHEET_PANE_TOP))
                    .child(body)
                    .into_any_element(),
            };
            return dialog
                .close_button(false)
                .p_0()
                .gap_0()
                .width(px(SHEET_WIDTH))
                .margin_top(px(SHEET_TOP))
                .h(px(height))
                // Enter confirms; a form that will not do stays open, and
                // so does one whose other place goes ahead by itself.
                .on_ok(move |_, window, cx| {
                    for_enter
                        .update(cx, |this, cx| {
                            let other = this
                                .forms()
                                .open
                                .as_ref()
                                .is_some_and(|open| open.place > 0);
                            !other && confirm(this, window, cx)
                        })
                        .unwrap_or(true)
                })
                .child(
                    v_flex().size_full().child(title_row).child(
                        h_flex()
                            .flex_1()
                            .min_h_0()
                            .items_start()
                            .child(places)
                            .child(pane),
                    ),
                );
        }

        if let Some(lead) = &lead {
            body = body.child(div().text_sm().whitespace_normal().child(lead.clone()));
        }
        body = body.children(inputs.iter().take(1).map(Input::new));
        if let Some(verdict) = verdict {
            body = body.child(said(verdict));
        }
        if let Some((names, more, chosen)) = offered
            && let Some(first) = &first
        {
            let none = names.is_empty();
            let mut list = v_flex().text_sm();
            for (ix, name) in names.into_iter().enumerate() {
                let (for_pick, field, value) = (owner.clone(), first.clone(), name.clone());
                let exact = name == typed.trim();
                list = list.child(
                    div()
                        .id(("form-offer", ix))
                        .px_2()
                        .py_0p5()
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .truncate()
                        // What Enter takes: the name in the field,
                        // else the first.
                        .when(if chosen { exact } else { ix == 0 }, |row| {
                            row.bg(theme.list_hover)
                        })
                        .hover(|row| row.bg(theme.list_active))
                        .on_click(move |_, window, cx| {
                            let value = value.clone();
                            field.update(cx, |state, cx| state.set_value(value, window, cx));
                            let _ = for_pick.update(cx, |_, cx| cx.notify());
                            window.refresh();
                        })
                        .child(name),
                );
            }
            if none {
                list = list.child(
                    div()
                        .px_2()
                        .text_color(muted)
                        .child(say!("workspace-form-no-match")),
                );
            } else if more > 0 {
                list = list.child(
                    div()
                        .px_2()
                        .text_xs()
                        .text_color(muted)
                        .child(say!("workspace-form-more-matches", more)),
                );
            }
            body = body.child(list);
        }
        body = body.children(inputs.iter().skip(1).map(Input::new));
        // The rows: what each is, and the field for it beside that.
        for (ix, row) in drawn.iter().enumerate() {
            body = body.child(
                labelled(row.label.clone(), muted, row_field(ix, row))
                    .id(("form-row", ix))
                    .test_support(),
            );
            if let Some(note) = &row.note {
                // Close under its field: the step to the next row is the
                // rows' own all the same.
                body = body
                    .child(beneath(small_print(note.clone(), muted)).mt(px(NOTE_LIFT - ROW_GAP)));
            }
        }
        if let Some(reviewed) = reviewed {
            body = body.child(under(said(reviewed).into_any_element()));
        }
        body = body.children(choose_row);
        if let Some(pick) = pick {
            // With the keyboard in the list, the arrows move the choice.
            let (for_keys, last) = (owner.clone(), pick.options.len().saturating_sub(1));
            body = body.on_key_down(move |event: &KeyDownEvent, window, cx| {
                let down = match event.keystroke.key.as_str() {
                    "down" => true,
                    "up" => false,
                    _ => return,
                };
                let _ = for_keys.update(cx, |this, _| {
                    if let Some(pick) = this
                        .forms_mut()
                        .open
                        .as_mut()
                        .and_then(|open| open.pick.as_mut())
                    {
                        pick.picked = stepped(pick.picked, down, last);
                    }
                });
                cx.stop_propagation();
                window.refresh();
            });
            let for_pick = owner.clone();
            body = body.child(under(
                RadioGroup::vertical("form-pick")
                    .selected_index(Some(pick.picked))
                    .on_click(move |ix, window, cx| {
                        let ix = *ix;
                        let _ = for_pick.update(cx, |this, _| {
                            if let Some(pick) = this
                                .forms_mut()
                                .open
                                .as_mut()
                                .and_then(|open| open.pick.as_mut())
                            {
                                pick.picked = ix;
                            }
                        });
                        window.refresh();
                    })
                    .children(pick.options.iter().enumerate().map(|(ix, (label, more))| {
                        Radio::new(("form-option", ix)).label(if more.is_empty() {
                            label.clone()
                        } else {
                            format!("{label}  ·  {more}")
                        })
                    }))
                    .into_any_element(),
            ));
        }
        for (ix, check) in checks.iter().enumerate() {
            body = body.child(switch(ix, check, false));
        }
        dialog
            .title(title.clone())
            .when_some(width, |dialog, width| dialog.width(px(width)))
            // Enter confirms; a form that will not do stays open.
            .on_ok(move |_, window, cx| {
                for_enter
                    .update(cx, |this, cx| confirm(this, window, cx))
                    .unwrap_or(true)
            })
            .child(body)
            .footer(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("form-cancel")
                            .ghost()
                            .label(say!("workspace-cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("form-ok").primary().label(ok_text).on_click(
                        move |_, window, cx| {
                            let done = for_ok
                                .update(cx, |this, cx| confirm(this, window, cx))
                                .unwrap_or(true);
                            if done {
                                window.close_dialog(cx);
                            }
                        },
                    )),
            )
    });
    if let Some(start) = start {
        cx.defer_in(window, move |_, window, cx| {
            start.update(cx, |state, cx| state.focus(window, cx));
        });
    }
}

/// What the open form is, as it is drawn.
#[derive(Default)]
struct Now {
    place: usize,
    checks: Vec<Check>,
    pick: Option<Pick>,
    chosen: Option<usize>,
    verdict: Option<Verdict>,
    offered: Option<(Vec<String>, usize, bool)>,
    reviewed: Option<Verdict>,
    rows: Vec<String>,
    sendable: bool,
}

/// Whether a field or a row that has to be filled in is still empty.
fn unfilled(answers: &Answers, required: &[bool], rows_required: &[bool]) -> bool {
    answers
        .texts
        .iter()
        .zip(required)
        .chain(answers.rows.iter().zip(rows_required))
        .any(|(text, required)| *required && text.is_empty())
}

/// What the open form answers, when that will do: nothing while a
/// required field is empty, a name is none of the list, or what is typed
/// is refused.
fn ready<H: FormHost>(host: &H, open: &Open<H>, cx: &App) -> Option<Answers> {
    let mut answers = answers(host, open, cx);
    if unfilled(&answers, &open.required, &open.rows_required_now()) {
        return None;
    }
    if let (Some(suggest), Some(first)) = (&open.suggest, answers.texts.first_mut())
        && suggest.strict
    {
        *first = completed(&suggest.names, first)?;
    }
    if let Some(judge) = &open.judge
        && let Some(Verdict::Refuse(_)) = judge(
            host,
            answers.texts.first().map_or("", String::as_str),
            &answers.checks,
        )
    {
        return None;
    }
    if let Some(review) = &open.review
        && let Some(Verdict::Refuse(_)) = review(host, &answers)
    {
        return None;
    }
    Some(answers)
}

/// Whether the open form would do as it stands; false without one. The
/// large dialog greys its button while it would not.
pub(crate) fn sendable<H: FormHost>(host: &H, cx: &App) -> bool {
    host.forms()
        .open
        .as_ref()
        .is_some_and(|open| ready(host, open, cx).is_some())
}

/// Run what the open form is for. False while it will not do (see
/// [`ready`]): the form then stays open.
pub(crate) fn confirm<H: FormHost>(host: &mut H, window: &mut Window, cx: &mut Context<H>) -> bool {
    let Some(open) = &host.forms().open else {
        return true;
    };
    let Some(answers) = ready(host, open, cx) else {
        return false;
    };
    let Some(open) = host.forms_mut().open.take() else {
        return true;
    };
    (open.action)(host, answers, window, cx);
    true
}

impl FormHost for Workspace {
    fn forms(&self) -> &Forms<Self> {
        &self.forms
    }

    fn forms_mut(&mut self) -> &mut Forms<Self> {
        &mut self.forms
    }
}

impl Workspace {
    /// The dialog is drawn again as its field is typed in.
    pub(super) fn follow_form_typing(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        follow_typing(self, window, cx);
    }

    /// What the labelled rows of the open form hold right now, in order.
    pub(super) fn form_row_texts(&self, cx: &App) -> Vec<String> {
        row_texts(self, cx)
    }

    /// Open a form. `on_ok` gets the answers once they will do.
    pub(super) fn form(
        &mut self,
        form: Form,
        on_ok: impl Fn(&mut Self, Answers, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The plain prompt's fields are borrowed; its own action is not
        // what Enter runs while a form is open.
        self.prompt_action = None;
        self.prompt_required = Vec::new();
        open(self, form, on_ok, window, cx);
    }

    /// Run what the open form is for; false while it will not do.
    pub(super) fn confirm_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        confirm(self, window, cx)
    }
}

#[cfg(test)]
mod tests {
    use super::{Answers, completed, suggestions, unfilled};

    fn names() -> Vec<String> {
        [
            "origin/main",
            "origin/topic/login",
            "origin/Topic-2",
            "backup/main",
            "backup/maintenance",
            "origin/release/1.x",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    #[test]
    fn names_are_offered_best_first() {
        let names = names();
        // Nothing typed: the list as it is, up to the limit.
        let (first, more) = suggestions(&names, "", 4);
        assert_eq!(
            first,
            [
                "origin/main",
                "origin/topic/login",
                "origin/Topic-2",
                "backup/main"
            ]
        );
        assert_eq!(more, 2);
        // The name itself, then what begins with it, then what has it as
        // the start of its last part, then what has it anywhere.
        assert_eq!(
            suggestions(&names, "backup/main", 8).0,
            ["backup/main", "backup/maintenance"]
        );
        assert_eq!(
            suggestions(&names, "main", 8).0,
            ["origin/main", "backup/main", "backup/maintenance"]
        );
        assert_eq!(
            suggestions(&names, "TOPIC", 8).0,
            ["origin/Topic-2", "origin/topic/login"]
        );
        assert_eq!(
            suggestions(&names, "origin 1.x", 8).0,
            ["origin/release/1.x"]
        );
        assert_eq!(suggestions(&names, "nothing", 8), (Vec::new(), 0));
        assert_eq!(suggestions(&[], "", 8), (Vec::new(), 0));
        // No limit is no names, and says how many there would be.
        assert_eq!(suggestions(&names, "main", 0), (Vec::new(), 3));
    }

    #[test]
    fn what_is_typed_is_completed_to_a_name_of_the_list() {
        let names = names();
        assert_eq!(
            completed(&names, "origin/main").as_deref(),
            Some("origin/main")
        );
        assert_eq!(
            completed(&names, "login").as_deref(),
            Some("origin/topic/login")
        );
        assert_eq!(completed(&names, "back").as_deref(), Some("backup/main"));
        assert_eq!(completed(&names, "nothing"), None);
        assert_eq!(completed(&names, "  "), None);
        assert_eq!(completed(&[], "main"), None);
    }

    #[test]
    fn the_arrows_move_the_choice_and_stop_at_the_ends() {
        use super::stepped;
        assert_eq!(stepped(0, true, 2), 1);
        assert_eq!(stepped(2, true, 2), 2);
        assert_eq!(stepped(1, false, 2), 0);
        assert_eq!(stepped(0, false, 2), 0);
        // A choice that is past the end, and a list of one or none.
        assert_eq!(stepped(9, false, 2), 2);
        assert_eq!(stepped(usize::MAX, true, 2), 2);
        assert_eq!(stepped(0, true, 0), 0);
    }

    #[test]
    fn answers_that_are_not_there_are_empty() {
        let answers = Answers {
            texts: vec!["a".to_owned()],
            rows: vec!["first".to_owned(), "second".to_owned()],
            checks: vec![true],
            chosen: Some(1),
            picked: 2,
        };
        assert_eq!(answers.text(0), "a");
        assert_eq!(answers.text(1), "");
        assert_eq!(answers.row(1), "second");
        assert_eq!(answers.row(2), "");
        assert!(answers.check(0));
        assert!(!answers.check(7));
    }

    #[test]
    fn a_form_waits_for_what_has_to_be_filled_in() {
        let answers = |texts: &[&str], rows: &[&str]| Answers {
            texts: texts.iter().map(|text| (*text).to_owned()).collect(),
            rows: rows.iter().map(|text| (*text).to_owned()).collect(),
            ..Answers::default()
        };
        // Only what is required counts, in the fields and in the rows.
        assert!(!unfilled(
            &answers(&["a", ""], &["b", ""]),
            &[true, false],
            &[true, false]
        ));
        assert!(unfilled(
            &answers(&["", "x"], &["b"]),
            &[true, false],
            &[true]
        ));
        assert!(unfilled(
            &answers(&["a"], &["b", ""]),
            &[true],
            &[false, true]
        ));
        // Nothing asked, nothing missing.
        assert!(!unfilled(&answers(&[], &[]), &[], &[]));
        assert!(!unfilled(&answers(&[""], &[""]), &[false], &[false]));
    }
}
