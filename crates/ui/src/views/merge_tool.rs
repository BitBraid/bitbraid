//! The merge tool: a conflicted file with our side and their side next to
//! each other, and the result underneath.
//!
//! The two files are shown whole. Each side has a box in its row over it
//! that takes all of its side, and a box at the first line of each of its
//! conflicts that takes that conflict's lines of it; a click on a line
//! takes or leaves that line. The result under them shows what the choices
//! make: a conflict not decided shows what both sides started from, on a
//! tint of its own; a line taken shows the colour of its side and a tick.
//! It is put together from the choices until it is edited by hand (the
//! editor is a button away); from then on it is the user's text. Saving
//! writes it and marks the file resolved.
//!
//! The two sides have the colours of their syntax, from one parse of each
//! side's whole text (worked out off the thread that draws), and a search
//! of their own: what is typed is marked in both sides, and the matches
//! are stepped through. The result has the editor's search.
//!
//! Sources: git-merge-file(1) (what the regions are); gpui-kit
//! documentation (virtual list, code editor).

use std::rc::Rc;

use std::ops::Range;

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, VirtualListScrollHandle,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Editor, EditorState, Input, InputEvent, InputState},
    scroll::{ScrollableElement, ScrollbarAxis},
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::{Value, json};

use crate::{
    keymap::{EDIT_CONTEXT, SaveFile},
    session::{Reply, Request, SessionHandle},
    views::diff_syntax::{self, LineStyles},
};

const LINE_HEIGHT: f32 = 16.0;
/// The size of the text of the three panes: a diff's.
const TEXT_SIZE: f32 = 12.0;
/// Height of the row that says which file this is, with what is done to
/// the file as a whole.
const TITLE_HEIGHT: f32 = 38.0;
/// Height of the row over the two sides that says what each is.
const SIDES_HEIGHT: f32 = 34.0;
/// Height of the row over the result, with the steps from one conflict to
/// the next.
const RESULT_HEIGHT: f32 = 38.0;
/// Width of the column a conflict's box stands in, of a line's number,
/// and of the bar in its side's colour before a line of a conflict.
const BOX_WIDTH: f32 = 18.0;
const NUMBER_WIDTH: f32 = 36.0;
const BAR_WIDTH: f32 = 3.0;

/// Events from the merge tool.
#[derive(Debug, Clone)]
pub enum MergeEvent {
    /// The file was resolved; status and the operation are stale.
    Resolved,
    /// Resolve this file in the external merge tool instead.
    External(String),
    /// Leave the tool; what was chosen stays for when the file is opened
    /// again.
    Close,
    /// Open the file as git left it, markers and all, in the editor.
    EditByHand,
}

/// A side of a conflict.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// The side the operation is applied to.
    Ours,
    /// The side that is brought in.
    Theirs,
}

/// A stretch of the file.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Part {
    /// Lines that merge cleanly.
    Same(Vec<String>),
    /// Lines the two sides changed differently, and which of them go into
    /// the result.
    Conflict {
        ours: Vec<String>,
        base: Vec<String>,
        theirs: Vec<String>,
        take_ours: Vec<bool>,
        take_theirs: Vec<bool>,
        /// Something was chosen here, even if that was "none of it".
        decided: bool,
    },
}

/// The file as the tool holds it: its stretches and what was chosen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Model {
    parts: Vec<Part>,
}

/// One row of the two panes.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// A line both sides have.
    Same(String),
    /// A line of what both sides started from.
    Base(String),
    /// One line of each side of a conflict, and whether it is taken.
    Pair {
        conflict: usize,
        line: usize,
        ours: Option<(String, bool)>,
        theirs: Option<(String, bool)>,
    },
}

fn lines_of(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|lines| {
            lines
                .iter()
                .filter_map(|line| line.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// What the result field starts with: what rerere left in the file when
/// `conflict.detail` says it resolved it (and that it is so), else the
/// text put together from the two sides. Line endings are the field's own
/// either way.
fn starting_text(detail: &Value, generated: &str, crlf: bool) -> (String, bool) {
    match detail.get("reused").and_then(|text| text.as_str()) {
        Some(text) if crlf => (text.replace("\r\n", "\n"), true),
        Some(text) => (text.to_owned(), true),
        None => (generated.to_owned(), false),
    }
}

/// Where a line of the result comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// What both sides have.
    Same,
    /// What both sides started from, standing for a conflict not decided
    /// yet.
    Undecided(usize),
    /// A line taken from a side in a conflict.
    Taken(usize, Side),
}

/// A line as it is shown: without its line ending.
fn shown(line: &str) -> String {
    line.trim_end_matches(['\n', '\r']).to_owned()
}

impl Model {
    /// From what `conflict.detail` returns.
    pub fn from_detail(detail: &Value) -> Self {
        let parts = detail
            .get("regions")
            .and_then(|regions| regions.as_array())
            .map(|regions| {
                regions
                    .iter()
                    .map(|region| {
                        if region.get("kind").and_then(|kind| kind.as_str()) == Some("same") {
                            Part::Same(lines_of(&region["lines"]))
                        } else {
                            let ours = lines_of(&region["ours"]);
                            let theirs = lines_of(&region["theirs"]);
                            Part::Conflict {
                                take_ours: vec![false; ours.len()],
                                take_theirs: vec![false; theirs.len()],
                                base: lines_of(&region["base"]),
                                ours,
                                theirs,
                                decided: false,
                            }
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self { parts }
    }

    fn conflict_mut(&mut self, conflict: usize) -> Option<&mut Part> {
        self.parts
            .iter_mut()
            .filter(|part| matches!(part, Part::Conflict { .. }))
            .nth(conflict)
    }

    /// Every line of the file that has an ending ends in carriage return
    /// and line feed.
    pub fn crlf(&self) -> bool {
        let mut any = false;
        for part in &self.parts {
            let lines: [&Vec<String>; 2] = match part {
                Part::Same(lines) => [lines, lines],
                Part::Conflict { ours, theirs, .. } => [ours, theirs],
            };
            for line in lines.into_iter().flatten() {
                if line.ends_with("\r\n") {
                    any = true;
                } else if line.ends_with('\n') {
                    return false;
                }
            }
        }
        any
    }

    /// How many conflicts there are, and how many have been decided.
    pub fn counts(&self) -> (usize, usize) {
        let mut total = 0usize;
        let mut done = 0usize;
        for part in &self.parts {
            if let Part::Conflict { decided, .. } = part {
                total = total.saturating_add(1);
                done = done.saturating_add(usize::from(*decided));
            }
        }
        (total, done)
    }

    /// Take a whole conflict from our side, their side, both or neither.
    #[cfg(test)]
    pub fn take(&mut self, conflict: usize, from_ours: bool, from_theirs: bool) {
        if let Some(Part::Conflict {
            take_ours,
            take_theirs,
            decided,
            ..
        }) = self.conflict_mut(conflict)
        {
            take_ours.iter_mut().for_each(|taken| *taken = from_ours);
            take_theirs
                .iter_mut()
                .for_each(|taken| *taken = from_theirs);
            *decided = true;
        }
    }

    /// Take all of one side's lines of a conflict, or none of them; the
    /// other side's stay as they were chosen.
    pub fn take_side(&mut self, conflict: usize, side: Side, on: bool) {
        if let Some(Part::Conflict {
            take_ours,
            take_theirs,
            decided,
            ..
        }) = self.conflict_mut(conflict)
        {
            let lines = match side {
                Side::Ours => take_ours,
                Side::Theirs => take_theirs,
            };
            lines.iter_mut().for_each(|taken| *taken = on);
            *decided = true;
        }
    }

    /// Whether every line of one side of a conflict is taken (a side with
    /// no lines counts when the conflict was decided).
    pub fn side_taken(&self, conflict: usize, side: Side) -> bool {
        match self
            .parts
            .iter()
            .filter(|part| matches!(part, Part::Conflict { .. }))
            .nth(conflict)
        {
            Some(Part::Conflict {
                take_ours,
                take_theirs,
                decided,
                ..
            }) => {
                let lines = match side {
                    Side::Ours => take_ours,
                    Side::Theirs => take_theirs,
                };
                *decided && lines.iter().all(|taken| *taken)
            }
            _ => false,
        }
    }

    /// Take one side whole in every conflict, or leave it everywhere.
    pub fn take_side_everywhere(&mut self, side: Side, on: bool) {
        let (total, _) = self.counts();
        for conflict in 0..total {
            self.take_side(conflict, side, on);
        }
    }

    /// Whether one side is taken whole in every conflict.
    pub fn side_taken_everywhere(&self, side: Side) -> bool {
        let (total, _) = self.counts();
        total > 0 && (0..total).all(|conflict| self.side_taken(conflict, side))
    }

    /// Take every choice back: no conflict is decided.
    pub fn reset(&mut self) {
        for part in &mut self.parts {
            if let Part::Conflict {
                take_ours,
                take_theirs,
                decided,
                ..
            } = part
            {
                take_ours.iter_mut().for_each(|taken| *taken = false);
                take_theirs.iter_mut().for_each(|taken| *taken = false);
                *decided = false;
            }
        }
    }

    /// The lines of the result as the result pane shows them, each with
    /// where it comes from: a conflict not decided stands as what both
    /// sides started from (or one empty line when they started from
    /// nothing), so that it is seen.
    pub fn result_rows(&self) -> Vec<(String, Origin)> {
        let mut rows = Vec::new();
        let mut conflict = 0usize;
        for part in &self.parts {
            match part {
                Part::Same(lines) => {
                    rows.extend(lines.iter().map(|line| (shown(line), Origin::Same)));
                }
                Part::Conflict {
                    ours,
                    base,
                    theirs,
                    take_ours,
                    take_theirs,
                    decided,
                } => {
                    if !*decided {
                        if base.is_empty() {
                            rows.push((String::new(), Origin::Undecided(conflict)));
                        }
                        rows.extend(
                            base.iter()
                                .map(|line| (shown(line), Origin::Undecided(conflict))),
                        );
                    } else {
                        for (side, lines, taken) in [
                            (Side::Ours, ours, take_ours),
                            (Side::Theirs, theirs, take_theirs),
                        ] {
                            rows.extend(
                                lines
                                    .iter()
                                    .zip(taken)
                                    .filter(|(_, taken)| **taken)
                                    .map(|(line, _)| (shown(line), Origin::Taken(conflict, side))),
                            );
                        }
                    }
                    conflict = conflict.saturating_add(1);
                }
            }
        }
        rows
    }

    /// Take or leave one line of a conflict.
    pub fn toggle(&mut self, conflict: usize, side: Side, line: usize) {
        if let Some(Part::Conflict {
            take_ours,
            take_theirs,
            decided,
            ..
        }) = self.conflict_mut(conflict)
        {
            let lines = match side {
                Side::Ours => take_ours,
                Side::Theirs => take_theirs,
            };
            if let Some(taken) = lines.get_mut(line) {
                *taken = !*taken;
                *decided = true;
            }
        }
    }

    /// The file put together from the choices. A conflict nothing was
    /// chosen for is written the way git writes one, so that it is seen and
    /// so that saving it by accident is refused.
    pub fn result(&self) -> String {
        let mut out = String::new();
        for part in &self.parts {
            match part {
                Part::Same(lines) => lines.iter().for_each(|line| out.push_str(line)),
                Part::Conflict {
                    ours,
                    theirs,
                    take_ours,
                    take_theirs,
                    decided,
                    ..
                } => {
                    if !*decided {
                        // Each block starts on a line of its own.
                        let closed = |text: &Vec<String>| {
                            text.last().is_none_or(|line| line.ends_with('\n'))
                        };
                        // not shown: git's conflict markers, written into the file
                        out.push_str("<<<<<<< ours\n");
                        ours.iter().for_each(|line| out.push_str(line));
                        if !closed(ours) {
                            out.push('\n');
                        }
                        out.push_str("=======\n");
                        theirs.iter().for_each(|line| out.push_str(line));
                        if !closed(theirs) {
                            out.push('\n');
                        }
                        // not shown: as above
                        out.push_str(">>>>>>> theirs\n");
                        continue;
                    }
                    for (lines, taken) in [(ours, take_ours), (theirs, take_theirs)] {
                        for (line, taken) in lines.iter().zip(taken) {
                            if *taken {
                                // A last line without its break, followed by
                                // more lines, gets one.
                                if !out.is_empty() && !out.ends_with('\n') {
                                    out.push('\n');
                                }
                                out.push_str(line);
                            }
                        }
                    }
                }
            }
        }
        out
    }

    /// The rows of the two panes: both files whole.
    #[cfg(test)]
    fn rows(&self, show_base: bool) -> Vec<Row> {
        self.placed(show_base)
            .into_iter()
            .map(|(row, ..)| row)
            .collect()
    }

    /// The two sides as texts of their own: our whole file and theirs,
    /// line by line, as the panes show them.
    pub fn sides(&self) -> (Vec<String>, Vec<String>) {
        let (mut ours_text, mut theirs_text) = (Vec::new(), Vec::new());
        for part in &self.parts {
            match part {
                Part::Same(lines) => {
                    ours_text.extend(lines.iter().map(|line| shown(line)));
                    theirs_text.extend(lines.iter().map(|line| shown(line)));
                }
                Part::Conflict { ours, theirs, .. } => {
                    ours_text.extend(ours.iter().map(|line| shown(line)));
                    theirs_text.extend(theirs.iter().map(|line| shown(line)));
                }
            }
        }
        (ours_text, theirs_text)
    }

    /// The rows of the two panes, each with the line of our side's text
    /// and of their side's text it shows ([`Self::sides`]): both files
    /// whole.
    fn placed(&self, show_base: bool) -> Vec<(Row, Option<usize>, Option<usize>)> {
        let mut rows = Vec::new();
        let mut conflict = 0usize;
        // How many lines of each side lie before the part at hand.
        let (mut ours_at, mut theirs_at) = (0usize, 0usize);
        for part in &self.parts {
            match part {
                Part::Same(lines) => {
                    rows.extend(lines.iter().enumerate().map(|(offset, line)| {
                        (
                            Row::Same(shown(line)),
                            Some(ours_at.saturating_add(offset)),
                            Some(theirs_at.saturating_add(offset)),
                        )
                    }));
                    ours_at = ours_at.saturating_add(lines.len());
                    theirs_at = theirs_at.saturating_add(lines.len());
                }
                Part::Conflict {
                    ours,
                    base,
                    theirs,
                    take_ours,
                    take_theirs,
                    ..
                } => {
                    if show_base {
                        rows.extend(base.iter().map(|line| (Row::Base(shown(line)), None, None)));
                    }
                    // A conflict with no line on either side still has a
                    // row, for its boxes.
                    for line in 0..ours.len().max(theirs.len()).max(1) {
                        let cell = |lines: &Vec<String>, taken: &Vec<bool>| {
                            lines.get(line).map(|text| {
                                (shown(text), taken.get(line).copied().unwrap_or(false))
                            })
                        };
                        rows.push((
                            Row::Pair {
                                conflict,
                                line,
                                ours: cell(ours, take_ours),
                                theirs: cell(theirs, take_theirs),
                            },
                            (line < ours.len()).then(|| ours_at.saturating_add(line)),
                            (line < theirs.len()).then(|| theirs_at.saturating_add(line)),
                        ));
                    }
                    ours_at = ours_at.saturating_add(ours.len());
                    theirs_at = theirs_at.saturating_add(theirs.len());
                    conflict = conflict.saturating_add(1);
                }
            }
        }
        rows
    }
}

/// Where a text has what is searched for, whatever the case of its
/// letters (of the ASCII ones): byte ranges, in order.
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }
    // Lowering ASCII keeps every byte where it was.
    let (text_low, query_low) = (text.to_ascii_lowercase(), query.to_ascii_lowercase());
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(found) = text_low.get(from..).and_then(|rest| rest.find(&query_low)) {
        let start = from.saturating_add(found);
        let end = start.saturating_add(query_low.len());
        if text.is_char_boundary(start) && text.is_char_boundary(end) {
            out.push(start..end);
        }
        from = end.max(start.saturating_add(1));
    }
    out
}

/// The rows that have what is searched for, on either side.
fn matching_rows(rows: &[Row], query: &str) -> Vec<usize> {
    let has = |text: &str| !match_ranges(text, query).is_empty();
    rows.iter()
        .enumerate()
        .filter(|(_, row)| match row {
            Row::Same(text) | Row::Base(text) => has(text),
            Row::Pair { ours, theirs, .. } => [ours, theirs]
                .into_iter()
                .flatten()
                .any(|(text, _)| has(text)),
        })
        .map(|(ix, _)| ix)
        .collect()
}

/// The box that takes a conflict's lines of a side, or all of a side.
fn tick(on: bool, colour: Hsla, theme: &gpui_kit::component::Theme) -> Div {
    div()
        .size(px(12.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(2.))
        .border_1()
        .border_color(if on { colour } else { theme.muted_foreground })
        .when(on, |mark| {
            mark.bg(colour).child(
                Icon::new(IconName::Check)
                    .size(px(10.))
                    .text_color(theme.background),
            )
        })
}

/// What the tool holds.
enum State {
    Empty,
    Loading,
    /// A file that is resolved as a whole, and why.
    Whole(String),
    Open {
        editor: Entity<EditorState>,
        /// What the editor was last given, to tell a hand edit from that.
        generated: String,
        /// Lines end in carriage return and line feed on disk.
        crlf: bool,
        _subscription: Subscription,
    },
}

/// The merge tool view.
pub struct MergeTool {
    session: SessionHandle,
    path: Option<String>,
    /// What the two sides are, in words.
    sides: (String, String),
    model: Model,
    state: State,
    /// The result was edited by hand: choices no longer overwrite it.
    manual: bool,
    /// The result is the resolution remembered from last time (rerere),
    /// as git put it into the file: untouched so far.
    reused: bool,
    /// That resolution, as the result field holds it.
    reused_text: Option<String>,
    show_base: bool,
    /// The conflict that "next" and "previous" count from.
    cursor: Option<usize>,
    rows: Rc<Vec<Row>>,
    /// For each row, the line of our side's text and of theirs it shows.
    row_lines: Rc<Vec<(Option<usize>, Option<usize>)>>,
    sizes: crate::zoom::Sizes,
    scroll: VirtualListScrollHandle,
    /// The lines of the result as its pane shows them, and its scroll.
    result_rows: Rc<Vec<(String, Origin)>>,
    result_sizes: crate::zoom::Sizes,
    result_scroll: VirtualListScrollHandle,
    /// The result is shown in the editor, to be typed into: asked for, or
    /// because it was edited by hand.
    editing: bool,
    /// The syntax colours of our side's text and of theirs, line by line,
    /// and which run of working them out is the current one.
    colours: (Rc<Vec<LineStyles>>, Rc<Vec<LineStyles>>),
    colours_dark: bool,
    colour_run: u64,
    /// What is searched for in the two sides, the rows that have it and
    /// the one last gone to.
    search: Entity<InputState>,
    query: String,
    matches: Rc<Vec<usize>>,
    match_at: Option<usize>,
    _search: Subscription,
    saving: bool,
    error: Option<String>,
    focus_handle: FocusHandle,
}

impl EventEmitter<MergeEvent> for MergeTool {}

impl MergeTool {
    /// Create the view.
    pub fn new(session: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(say!("merge-tool-search")));
        let subscription = cx.subscribe_in(
            &search,
            window,
            |this, state, event: &InputEvent, _, cx| match event {
                InputEvent::Change => {
                    let query = state.read(cx).value().to_string();
                    this.set_query(query, cx);
                }
                InputEvent::PressEnter { .. } => this.step_match(true, cx),
                _ => {}
            },
        );
        Self {
            session,
            path: None,
            sides: (String::new(), String::new()),
            model: Model::default(),
            state: State::Empty,
            manual: false,
            reused: false,
            reused_text: None,
            show_base: false,
            cursor: None,
            rows: Rc::new(Vec::new()),
            row_lines: Rc::new(Vec::new()),
            sizes: crate::zoom::Sizes::default(),
            scroll: VirtualListScrollHandle::new(),
            result_rows: Rc::new(Vec::new()),
            result_sizes: crate::zoom::Sizes::default(),
            result_scroll: VirtualListScrollHandle::new(),
            editing: false,
            colours: (Rc::new(Vec::new()), Rc::new(Vec::new())),
            colours_dark: cx.theme().is_dark(),
            colour_run: 0,
            search,
            query: String::new(),
            matches: Rc::new(Vec::new()),
            match_at: None,
            _search: subscription,
            saving: false,
            error: None,
            focus_handle: cx.focus_handle(),
        }
    }

    /// The path being resolved.
    pub fn path(&self) -> Option<&str> {
        self.path.as_deref()
    }

    /// Open the conflicted file at `path`; `ours` and `theirs` say what the
    /// two sides are.
    pub fn open(
        &mut self,
        path: String,
        ours: String,
        theirs: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.path = Some(path.clone());
        self.sides = (ours, theirs);
        self.state = State::Loading;
        self.model = Model::default();
        self.manual = false;
        self.editing = false;
        self.reused = false;
        self.reused_text = None;
        self.cursor = None;
        self.saving = false;
        self.error = None;
        self.rebuild_rows();
        let reply = self.session.send(Request::Command {
            id: "conflict.detail".to_owned(),
            params: json!({ "path": path }),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.path.as_deref() != Some(path.as_str()) {
                    return;
                }
                match reply {
                    Reply::Command(detail) => this.loaded(&path, &detail, window, cx),
                    Reply::Error(err) => this.state = State::Whole(err),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn loaded(&mut self, path: &str, detail: &Value, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(why) = detail.get("whole_only").and_then(|why| why.as_str()) {
            self.state = State::Whole(why.to_owned());
            return;
        }
        self.model = Model::from_detail(detail);
        self.rebuild_rows();
        self.colour(cx);
        let crlf = self.model.crlf();
        let text = if crlf {
            self.model.result().replace("\r\n", "\n")
        } else {
            self.model.result()
        };
        let language = diff_syntax::language_for(path).unwrap_or("plaintext");
        // A file rerere resolved from its record starts from what it left
        // there, not from the two sides again: that is the resolution, to
        // be accepted or changed. The two sides are one button away.
        let (initial, reused) = starting_text(detail, &text, crlf);
        self.reused = reused;
        self.manual = reused;
        self.reused_text = reused.then(|| initial.clone());
        let editor = cx.new(|cx| {
            EditorState::new(window, cx)
                .language(language.to_owned())
                .line_number(true)
                .soft_wrap(false)
                .searchable(true)
                .default_value(initial)
        });
        let subscription = cx.subscribe(&editor, |this, editor, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change)
                && let State::Open { generated, .. } = &this.state
            {
                let manual = editor.read(cx).value().as_ref() != generated.as_str();
                // The first change to what was remembered makes it the
                // user's own text.
                let reused = this.reused && !this.touched_reused(editor.read(cx).value().as_ref());
                if manual != this.manual || reused != this.reused {
                    this.manual = manual;
                    this.reused = reused;
                    cx.notify();
                }
            }
        });
        self.state = State::Open {
            editor,
            generated: text,
            crlf,
            _subscription: subscription,
        };
    }

    /// Work out the syntax colours of the two sides, away from the thread
    /// that draws; the lines show plain until they are in.
    fn colour(&mut self, cx: &mut Context<Self>) {
        self.colours_dark = cx.theme().is_dark();
        self.colour_run = self.colour_run.wrapping_add(1);
        let run = self.colour_run;
        let Some(language) = self.path.as_deref().and_then(diff_syntax::language_for) else {
            self.colours = (Rc::new(Vec::new()), Rc::new(Vec::new()));
            return;
        };
        let (ours, theirs) = self.model.sides();
        let theme = cx.theme().highlight_theme.clone();
        let colours = cx.background_spawn(async move {
            (
                diff_syntax::highlight_lines(&ours, language, &theme),
                diff_syntax::highlight_lines(&theirs, language, &theme),
            )
        });
        cx.spawn(async move |this, cx| {
            let (ours, theirs) = colours.await;
            let _ = this.update(cx, |this, cx| {
                if this.colour_run == run {
                    this.colours = (Rc::new(ours), Rc::new(theirs));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Search the two sides for a text.
    fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        if query == self.query {
            return;
        }
        self.query = query;
        self.match_at = None;
        // The rows that have it are found again.
        self.rebuild_rows();
        cx.notify();
    }

    /// Go to the next or the previous row that has what is searched for.
    fn step_match(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(last) = self.matches.len().checked_sub(1) else {
            return;
        };
        let next = match (self.match_at, forward) {
            (None, true) => 0,
            (None, false) => last,
            (Some(at), true) if at >= last => 0,
            (Some(at), true) => at.saturating_add(1),
            (Some(0), false) => last,
            (Some(at), false) => at.saturating_sub(1),
        };
        self.match_at = Some(next);
        if let Some(row) = self.matches.get(next).copied() {
            self.scroll.scroll_to_item(row, ScrollStrategy::Center);
        }
        cx.notify();
    }

    fn rebuild_rows(&mut self) {
        let placed = self.model.placed(self.show_base);
        self.row_lines = Rc::new(
            placed
                .iter()
                .map(|(_, ours, theirs)| (*ours, *theirs))
                .collect(),
        );
        let rows: Vec<Row> = placed.into_iter().map(|(row, ..)| row).collect();
        self.matches = Rc::new(matching_rows(&rows, &self.query));
        if self.match_at.is_some_and(|at| at >= self.matches.len()) {
            self.match_at = None;
        }
        self.sizes = crate::zoom::Sizes::new(vec![size(px(0.), px(LINE_HEIGHT)); rows.len()]);
        self.rows = Rc::new(rows);
        let result = self.model.result_rows();
        self.result_sizes =
            crate::zoom::Sizes::new(vec![size(px(0.), px(LINE_HEIGHT)); result.len()]);
        self.result_rows = Rc::new(result);
    }

    /// A choice changed: lay the panes out again and, unless the result is
    /// the user's own text by now, put it together again.
    fn chose(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.error = None;
        self.rebuild_rows();
        if !self.manual {
            self.regenerate(window, cx);
        }
        cx.notify();
    }

    fn regenerate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.model.result();
        if let State::Open {
            editor,
            generated,
            crlf,
            ..
        } = &mut self.state
        {
            let text = if *crlf {
                text.replace("\r\n", "\n")
            } else {
                text
            };
            *generated = text.clone();
            editor.update(cx, |state, cx| state.set_value(text, window, cx));
        }
        self.manual = false;
        self.reused = false;
    }

    /// Take every choice back and put the result together from the two
    /// sides again (the result row's reset).
    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.model.reset();
        self.editing = false;
        self.error = None;
        self.rebuild_rows();
        self.regenerate(window, cx);
        cx.notify();
    }

    /// Whether the result is no longer the text that was remembered.
    fn touched_reused(&self, now: &str) -> bool {
        self.reused_text.as_deref() != Some(now)
    }

    /// Whether the result is the resolution remembered from last time; for
    /// the off-screen tests.
    pub fn starts_from_reused(&self) -> bool {
        self.reused
    }

    /// The text of the result as it stands; for the off-screen tests.
    pub fn result_text(&self, cx: &App) -> Option<String> {
        match &self.state {
            State::Open { editor, .. } => Some(editor.read(cx).value().to_string()),
            _ => None,
        }
    }

    /// Go to the next or previous conflict and bring it to the middle of
    /// the two sides and of the result.
    fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let (total, _) = self.model.counts();
        let Some(last) = total.checked_sub(1) else {
            return;
        };
        self.cursor = match (self.cursor, forward) {
            (None, true) => Some(0),
            (None, false) => Some(last),
            (Some(at), true) => Some(at.saturating_add(1).min(last)),
            (Some(at), false) => Some(at.saturating_sub(1)),
        };
        let cursor = self.cursor;
        let side_row = self.rows.iter().position(
            |row| matches!(row, Row::Pair { conflict, line: 0, .. } if Some(*conflict) == cursor),
        );
        let result_row = self
            .result_rows
            .iter()
            .position(|(_, origin)| match origin {
                Origin::Undecided(conflict) | Origin::Taken(conflict, _) => {
                    Some(*conflict) == cursor
                }
                Origin::Same => false,
            });
        // To the middle even when it is in view already, which the list's
        // own scrolling to an item leaves alone.
        let centre = |scroll: &VirtualListScrollHandle, rows: usize, row: usize| {
            let view = crate::zoom::unpx(scroll.base_handle().bounds().size.height);
            let all = rows as f32 * LINE_HEIGHT;
            let top = (row as f32 * LINE_HEIGHT + LINE_HEIGHT / 2.0 - view / 2.0)
                .clamp(0.0, (all - view).max(0.0));
            scroll.set_offset(point(px(0.), px(-top)));
        };
        if let Some(row) = side_row {
            centre(&self.scroll, self.rows.len(), row);
        }
        if let Some(row) = result_row {
            centre(&self.result_scroll, self.result_rows.len(), row);
        }
        cx.notify();
    }

    /// Resolve the file as a whole with one side.
    fn take_whole(&mut self, side: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.path.clone() else {
            return;
        };
        self.finish(
            Request::Command {
                id: "conflict.take".to_owned(),
                params: json!({ "path": path, "side": side }),
            },
            window,
            cx,
        );
    }

    /// Write the result and mark the file resolved.
    fn save(&mut self, allow_markers: bool, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(path), State::Open { editor, crlf, .. }) = (self.path.clone(), &self.state)
        else {
            return;
        };
        let text = crate::views::edit::to_disk(&editor.read(cx).value(), *crlf);
        self.finish(
            Request::Command {
                id: "conflict.write".to_owned(),
                params: json!({ "path": path, "text": text, "allow_markers": allow_markers }),
            },
            window,
            cx,
        );
    }

    fn finish(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        if self.saving {
            return;
        }
        self.saving = true;
        self.error = None;
        let reply = self.session.send(request);
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else { return };
            let _ = this.update(cx, |this, cx| {
                this.saving = false;
                match reply {
                    Reply::Error(err) => this.error = Some(err),
                    _ => {
                        this.state = State::Empty;
                        this.path = None;
                        cx.emit(MergeEvent::Resolved);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn render_row(&self, ix: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (ours_line, theirs_line) = self.row_lines.get(ix).copied().unwrap_or((None, None));
        let current = self
            .match_at
            .and_then(|at| self.matches.get(at))
            .is_some_and(|row| *row == ix);
        let found = if current {
            theme.yellow.opacity(0.7)
        } else {
            theme.yellow.opacity(0.35)
        };
        // A line with the colours of its syntax (from its side's text)
        // and what is searched for marked in it.
        let line = |text: &str, of: Option<(&Rc<Vec<LineStyles>>, usize)>| {
            let colours: LineStyles = of
                .and_then(|(side, number)| side.get(number))
                .map(|styles| {
                    styles
                        .iter()
                        .filter(|(range, _)| {
                            range.end <= text.len()
                                && text.is_char_boundary(range.start)
                                && text.is_char_boundary(range.end)
                        })
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let marks: LineStyles = match_ranges(text, &self.query)
                .into_iter()
                .map(|range| {
                    (
                        range,
                        HighlightStyle {
                            background_color: Some(found),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            let highlights: LineStyles = combine_highlights(colours, marks).collect();
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .whitespace_nowrap()
                .child(if text.is_empty() {
                    StyledText::new(" ")
                } else {
                    StyledText::new(text.to_owned()).with_highlights(highlights)
                })
        };
        let (ours_colours, theirs_colours) = (&self.colours.0, &self.colours.1);
        let ours_of = ours_line.map(|number| (ours_colours, number));
        let theirs_of = theirs_line.map(|number| (theirs_colours, number));
        // The number of a line in its side's text, after the column the
        // boxes of the conflicts stand in.
        let number = |of: Option<usize>| {
            div()
                .w(px(NUMBER_WIDTH))
                .flex_none()
                .pr_2()
                .text_right()
                .text_color(theme.muted_foreground.opacity(0.7))
                .child(match of {
                    Some(line) => line.saturating_add(1).to_string(),
                    None => String::new(),
                })
        };
        let divider = || div().flex_none().w(px(1.)).h_full().bg(theme.border);
        let gap = || div().w(px(BOX_WIDTH)).flex_none();
        let no_bar = || div().w(px(BAR_WIDTH)).flex_none();
        match row {
            // What both sides have stands in both, each with its number.
            Row::Same(text) => {
                let side = |of: Option<(&Rc<Vec<LineStyles>>, usize)>, at: Option<usize>| {
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center()
                        .child(gap())
                        .child(number(at))
                        .child(no_bar())
                        .child(div().w(px(16.)).flex_none())
                        .child(line(text, of))
                };
                h_flex()
                    .w_full()
                    .h(px(LINE_HEIGHT))
                    .child(side(ours_of, ours_line))
                    .child(divider())
                    .child(side(theirs_of, theirs_line))
                    .into_any_element()
            }
            Row::Base(text) => h_flex()
                .w_full()
                .h(px(LINE_HEIGHT))
                .px_2()
                .items_center()
                .bg(theme.muted)
                .text_color(theme.muted_foreground)
                .gap_2()
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .child(say!("merge-tool-base-row")),
                )
                .child(line(text, None))
                .into_any_element(),
            Row::Pair {
                conflict,
                line: index,
                ours,
                theirs,
            } => {
                let (conflict, index) = (*conflict, *index);
                let current = self.cursor == Some(conflict);
                // A line of a conflict: in its side's colour, with a bar
                // before it and a tick when it is taken; its first line
                // has the box that takes the conflict's lines of that
                // side. A click on a line takes it or leaves it.
                let cell = |id: &'static str,
                            side: Side,
                            content: &Option<(String, bool)>,
                            of: Option<(&Rc<Vec<LineStyles>>, usize)>,
                            color: Hsla| {
                    let taken_whole = self.model.side_taken(conflict, side);
                    let base = h_flex()
                        .id((id, conflict.saturating_mul(100_000).saturating_add(index)))
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .items_center();
                    let tick_box = (index == 0).then(|| {
                        div()
                            .id((
                                match side {
                                    Side::Ours => "take-ours",
                                    Side::Theirs => "take-theirs",
                                },
                                conflict,
                            ))
                            .w(px(BOX_WIDTH))
                            .h_full()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.model.take_side(conflict, side, !taken_whole);
                                this.cursor = Some(conflict);
                                this.chose(window, cx);
                            }))
                            .child(tick(taken_whole, color, &theme))
                            .test_support()
                    });
                    let base = base
                        .children(tick_box)
                        .when(index > 0, |row| row.child(gap()));
                    match content {
                        Some((text, taken)) => base
                            .cursor_pointer()
                            .bg(color.opacity(if *taken {
                                0.3
                            } else if current {
                                0.18
                            } else {
                                0.12
                            }))
                            .hover(|style| style.bg(color.opacity(0.4)))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.model.toggle(conflict, side, index);
                                this.cursor = Some(conflict);
                                this.chose(window, cx);
                            }))
                            .child(number(of.map(|(_, at)| at)))
                            .child(div().w(px(BAR_WIDTH)).h_full().flex_none().bg(color))
                            .child(div().w(px(16.)).flex_none().flex().justify_center().when(
                                *taken,
                                |mark| {
                                    mark.child(
                                        Icon::new(IconName::CircleCheck)
                                            .size(px(12.))
                                            .text_color(theme.success),
                                    )
                                },
                            ))
                            .child(line(text, of)),
                        None => base,
                    }
                };
                h_flex()
                    .w_full()
                    .h(px(LINE_HEIGHT))
                    .child(cell("ours-line", Side::Ours, ours, ours_of, theme.blue))
                    .child(divider())
                    .child(cell(
                        "theirs-line",
                        Side::Theirs,
                        theirs,
                        theirs_of,
                        theme.yellow,
                    ))
                    .into_any_element()
            }
        }
    }

    /// A line of the result: its number, and where it comes from as a bar
    /// and a tint: the side it was taken from with a tick, or a conflict
    /// not decided yet.
    fn render_result_row(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some((text, origin)) = self.result_rows.get(ix) else {
            return div().h(px(LINE_HEIGHT)).into_any_element();
        };
        let (colour, taken) = match origin {
            Origin::Same => (None, false),
            Origin::Undecided(_) => (Some(theme.magenta), false),
            Origin::Taken(_, Side::Ours) => (Some(theme.blue), true),
            Origin::Taken(_, Side::Theirs) => (Some(theme.yellow), true),
        };
        h_flex()
            .w_full()
            .h(px(LINE_HEIGHT))
            .items_center()
            .when_some(colour, |row, colour| row.bg(colour.opacity(0.18)))
            .child(div().w(px(BOX_WIDTH)).flex_none())
            .child(
                div()
                    .w(px(NUMBER_WIDTH))
                    .flex_none()
                    .pr_2()
                    .text_right()
                    .text_color(theme.muted_foreground.opacity(0.7))
                    .child(ix.saturating_add(1).to_string()),
            )
            .child(
                div()
                    .w(px(BAR_WIDTH))
                    .h_full()
                    .flex_none()
                    .when_some(colour, |bar, colour| bar.bg(colour)),
            )
            .child(
                div()
                    .w(px(16.))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .when(taken, |mark| {
                        mark.child(
                            Icon::new(IconName::CircleCheck)
                                .size(px(12.))
                                .text_color(theme.success),
                        )
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .whitespace_nowrap()
                    .child(if text.is_empty() {
                        " ".to_owned()
                    } else {
                        text.clone()
                    }),
            )
            .into_any_element()
    }
}

impl Focusable for MergeTool {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MergeTool {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.colours_dark != cx.theme().is_dark() && matches!(self.state, State::Open { .. }) {
            self.colour(cx);
        }
        let theme = cx.theme().clone();
        let path = self.path.clone().unwrap_or_default();
        let (total, done) = self.model.counts();
        let found = match (self.query.is_empty(), self.matches.len(), self.match_at) {
            (true, ..) => String::new(),
            (false, 0, _) => say!("merge-tool-not-found").to_owned(),
            (false, count, Some(at)) => {
                say!("merge-tool-found-at", at = at.saturating_add(1), count)
            }
            (false, count, None) => say!("merge-tool-found", count),
        };
        let note = |text: String| {
            div()
                .p_2()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        let body: AnyElement = match &self.state {
            State::Empty => note(say!("merge-tool-choose-a-file").to_owned()),
            State::Loading => note(say!("merge-tool-loading").to_owned()),
            State::Whole(why) => v_flex()
                .p_2()
                .gap_2()
                .text_sm()
                .child(why.clone())
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("whole-ours")
                                .small()
                                .outline()
                                .label(say!("merge-tool-take-ours", side = &self.sides.0))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.take_whole("ours", window, cx)
                                })),
                        )
                        .child(
                            Button::new("whole-theirs")
                                .small()
                                .outline()
                                .label(say!("merge-tool-take-theirs", side = &self.sides.1))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.take_whole("theirs", window, cx)
                                })),
                        )
                        .child(
                            Button::new("whole-external")
                                .small()
                                .ghost()
                                .label(say!("merge-tool-external"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(path) = this.path.clone() {
                                        cx.emit(MergeEvent::External(path));
                                    }
                                })),
                        ),
                )
                .into_any_element(),
            State::Open { editor, .. } => {
                let rows = self.rows.clone();
                // Over each side: the box that takes all of it, its
                // colour, its name and what it is.
                let title = |name: &'static str, what: &str, side: Side, colour: Hsla| {
                    let whole = self.model.side_taken_everywhere(side);
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .pr_2()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .id(match side {
                                    Side::Ours => "take-all-ours",
                                    Side::Theirs => "take-all-theirs",
                                })
                                .w(px(BOX_WIDTH))
                                .h_full()
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .cursor_pointer()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.model.take_side_everywhere(side, !whole);
                                    this.chose(window, cx);
                                }))
                                .child(tick(whole, colour, &theme))
                                .test_support(),
                        )
                        .child(
                            div()
                                .flex_none()
                                .size(px(14.))
                                .rounded(px(3.))
                                .bg(colour.opacity(0.85)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_sm()
                                .font_semibold()
                                .text_color(colour)
                                .child(name),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(what.to_owned()),
                        )
                };
                // Which conflict the steps count from, of how many.
                let at = match (self.cursor, total) {
                    (_, 0) => say!("merge-tool-no-conflicts").to_owned(),
                    (Some(at), _) => {
                        say!("merge-tool-conflict-at", at = at.saturating_add(1), total)
                    }
                    (None, _) => say!("merge-tool-conflicts", total),
                };
                // The choices can be taken back once there are some (or
                // the result was typed into, or is what was remembered).
                let chosen_any = done > 0 || self.manual;
                let in_editor = self.editing || self.manual;
                let result_rows = self.result_rows.len();
                let result: AnyElement = if in_editor {
                    div()
                        .flex_1()
                        .min_h_0()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(TEXT_SIZE))
                        .child(Editor::new(editor).size_full())
                        .into_any_element()
                } else {
                    v_flex()
                        .id("merge-result")
                        .flex_1()
                        .min_h_0()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(TEXT_SIZE))
                        .line_height(px(LINE_HEIGHT))
                        .child(
                            v_virtual_list(
                                cx.entity().clone(),
                                "merge-result-rows",
                                self.result_sizes.get(),
                                move |this, range, _, cx| {
                                    range
                                        .filter(|ix| *ix < result_rows)
                                        .map(|ix| this.render_result_row(ix, cx))
                                        .collect::<Vec<_>>()
                                },
                            )
                            .track_scroll(&self.result_scroll),
                        )
                        .into_any_element()
                };
                v_flex()
                    .size_full()
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(SIDES_HEIGHT))
                            .flex_none()
                            .items_center()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(title(
                                say!("merge-tool-ours"),
                                &self.sides.0,
                                Side::Ours,
                                theme.blue,
                            ))
                            .child(div().flex_none().w(px(1.)).h_full().bg(theme.border))
                            .child(title(
                                say!("merge-tool-theirs"),
                                &self.sides.1,
                                Side::Theirs,
                                theme.yellow,
                            )),
                    )
                    .child(
                        v_flex()
                            .id("merge-sides")
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .font_family(theme.mono_font_family.clone())
                            .text_size(px(TEXT_SIZE))
                            .line_height(px(LINE_HEIGHT))
                            .child(
                                v_virtual_list(
                                    cx.entity().clone(),
                                    "merge-rows",
                                    self.sizes.get(),
                                    move |this, range, _, cx| {
                                        range
                                            .filter_map(|ix| {
                                                Some(this.render_row(ix, rows.get(ix)?, cx))
                                            })
                                            .collect::<Vec<_>>()
                                    },
                                )
                                .track_scroll(&self.scroll),
                            )
                            .scrollbar(&self.scroll, ScrollbarAxis::Vertical),
                    )
                    // Over the result: its name at the left, the steps
                    // from conflict to conflict in the middle, and at the
                    // right the way back to no choices at all.
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(RESULT_HEIGHT))
                            .flex_none()
                            .pl(px(10.))
                            .pr_2()
                            .gap_2()
                            .items_center()
                            .border_t_1()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(h_flex().flex_1().min_w_0().child(
                                div().flex_none().text_lg().child(say!("merge-tool-result")),
                            ))
                            .child(
                                h_flex()
                                    .flex_none()
                                    .gap_2()
                                    .items_center()
                                    .child(div().text_sm().text_color(theme.link).child(at))
                                    .child(
                                        Button::new("merge-prev")
                                            .outline()
                                            .xsmall()
                                            .icon(IconName::ChevronUp)
                                            .tooltip(say!("merge-tool-previous-conflict"))
                                            .disabled(total == 0)
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.step(false, cx)),
                                            ),
                                    )
                                    .child(
                                        Button::new("merge-next")
                                            .outline()
                                            .xsmall()
                                            .icon(IconName::ChevronDown)
                                            .tooltip(say!("merge-tool-next-conflict"))
                                            .disabled(total == 0)
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.step(true, cx)),
                                            ),
                                    ),
                            )
                            .child(
                                h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .gap_2()
                                    .justify_end()
                                    // The result as text to type into.
                                    .when(!self.manual, |row| {
                                        row.child(
                                            crate::views::chosen(
                                                Button::new("merge-edit-result")
                                                    .outline()
                                                    .xsmall()
                                                    .icon(Icon::new(
                                                        gpui_kit::assets::IconName::Pencil,
                                                    ))
                                                    .tooltip(say!("merge-tool-type-result")),
                                                self.editing,
                                                cx,
                                            )
                                            .on_click(
                                                cx.listener(|this, _, _, cx| {
                                                    this.editing = !this.editing;
                                                    cx.notify();
                                                }),
                                            ),
                                        )
                                    })
                                    .when(chosen_any, |row| {
                                        row.child(
                                            Button::new("merge-rebuild")
                                                .outline()
                                                .xsmall()
                                                .label(say!("merge-tool-reset"))
                                                .tooltip(if self.reused {
                                                    say!("merge-tool-reset-remembered")
                                                } else {
                                                    say!("merge-tool-reset-choices")
                                                })
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.reset(window, cx)
                                                })),
                                        )
                                    }),
                            ),
                    )
                    .child(result)
                    .into_any_element()
            }
        };
        let open = matches!(self.state, State::Open { .. });
        let markers_left = self
            .error
            .as_ref()
            .is_some_and(|err| err.contains("conflict markers"));
        v_flex()
            .id("merge-tool")
            .key_context(EDIT_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &SaveFile, window, cx| this.save(false, window, cx)))
            .size_full()
            .bg(theme.background)
            // Which file, and how far it is; at the right what is done to
            // the file as a whole, saving last, and the way out.
            .child(
                h_flex()
                    .w_full()
                    .h(px(TITLE_HEIGHT))
                    .flex_none()
                    .pl(px(10.))
                    .pr_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .small()
                            .text_color(theme.warning),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .font_semibold()
                            .child(path),
                    )
                    .when(open, |header| {
                        header.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(match total {
                                    0 => say!("merge-tool-none-left").to_owned(),
                                    _ => say!("merge-tool-decided", done, total),
                                }),
                        )
                    })
                    .child(div().flex_1())
                    // The search of the two sides: what is typed is marked
                    // in both, Enter and the arrows go from line to line.
                    .when(open, |header| {
                        header
                            // It gives way first when the pane is narrow:
                            // the buttons after it must stay whole.
                            .child(
                                div()
                                    .w(px(190.))
                                    .min_w(px(64.))
                                    // Far sooner than the path beside
                                    // it, which says which file this is.
                                    .flex_shrink(12.0)
                                    .child(Input::new(&self.search).xsmall()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .min_w(px(56.))
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(found),
                            )
                            .child(
                                Button::new("merge-find-prev")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowUp)
                                    .tooltip(say!("merge-tool-find-previous"))
                                    .disabled(self.matches.is_empty())
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.step_match(false, cx)),
                                    ),
                            )
                            .child(
                                Button::new("merge-find-next")
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::ArrowDown)
                                    .tooltip(say!("merge-tool-find-next"))
                                    .disabled(self.matches.is_empty())
                                    .on_click(
                                        cx.listener(|this, _, _, cx| this.step_match(true, cx)),
                                    ),
                            )
                    })
                    .when(open, |header| {
                        header
                            .child(
                                crate::views::chosen(
                                    Button::new("merge-base")
                                        .outline()
                                        .xsmall()
                                        .label(say!("merge-tool-base"))
                                        .tooltip(say!("merge-tool-base-tooltip")),
                                    self.show_base,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        this.show_base = !this.show_base;
                                        this.rebuild_rows();
                                        cx.notify();
                                    },
                                )),
                            )
                            .child(
                                Button::new("merge-by-hand")
                                    .outline()
                                    .xsmall()
                                    .label(say!("merge-tool-by-hand"))
                                    .tooltip(say!("merge-tool-by-hand-tooltip"))
                                    .on_click(
                                        cx.listener(|_, _, _, cx| cx.emit(MergeEvent::EditByHand)),
                                    ),
                            )
                            .child(
                                Button::new("merge-external")
                                    .outline()
                                    .xsmall()
                                    .icon(IconName::ExternalLink)
                                    .label(say!("merge-tool-external"))
                                    .tooltip(say!("merge-tool-external-tooltip"))
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(path) = this.path.clone() {
                                            cx.emit(MergeEvent::External(path));
                                        }
                                    })),
                            )
                            .child(
                                Button::new("merge-save")
                                    .outline()
                                    .success()
                                    .xsmall()
                                    .icon(Icon::new(gpui_kit::assets::IconName::Save))
                                    .label(say!("merge-tool-save"))
                                    .tooltip(say!("merge-tool-save-tooltip"))
                                    .disabled(self.saving)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save(false, window, cx)
                                    })),
                            )
                    })
                    .child(
                        Button::new("merge-close")
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .tooltip(say!("merge-tool-close"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(MergeEvent::Close))),
                    ),
            )
            // What went wrong gets a line of its own: it does not fit
            // beside the buttons.
            .children(self.error.clone().map(|err| {
                h_flex()
                    .w_full()
                    .px_2()
                    .py_1()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .bg(theme.danger.opacity(0.15))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.danger)
                            .child(err),
                    )
                    .when(markers_left, |row| {
                        row.child(
                            Button::new("merge-save-anyway")
                                .outline()
                                .xsmall()
                                .label(say!("merge-tool-save-anyway"))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.save(true, window, cx)),
                                ),
                        )
                    })
            }))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests {
    use super::{Model, Origin, Row, Side, match_ranges, matching_rows};
    use serde_json::json;

    #[test]
    fn each_row_knows_which_line_of_each_side_it_shows() {
        let m = model();
        let (ours, theirs) = m.sides();
        // Every row's line is the text the row shows, on its side.
        for (row, ours_line, theirs_line) in m.placed(true) {
            match row {
                Row::Same(text) => {
                    assert_eq!(ours[ours_line.unwrap()], text);
                    assert_eq!(theirs[theirs_line.unwrap()], text);
                }
                Row::Pair {
                    ours: left,
                    theirs: right,
                    ..
                } => {
                    assert_eq!(
                        left.map(|(text, _)| text),
                        ours_line.map(|n| ours[n].clone())
                    );
                    assert_eq!(
                        right.map(|(text, _)| text),
                        theirs_line.map(|n| theirs[n].clone())
                    );
                }
                Row::Base(_) => assert_eq!((ours_line, theirs_line), (None, None)),
            }
        }
        // A long stretch is shown whole, with its numbers.
        let lines: Vec<String> = (0..20).map(|n| format!("l{n}\n")).collect();
        let long = Model::from_detail(&json!({ "regions": [{ "kind": "same", "lines": lines }] }));
        let whole = long.placed(false);
        assert_eq!(whole.len(), 20);
        assert_eq!(whole[9], (Row::Same("l9".to_owned()), Some(9), Some(9)));
    }

    #[test]
    fn what_is_searched_for_is_found_whatever_its_case() {
        assert_eq!(
            match_ranges("let Foo = foo(FOO);", "foo"),
            [4..7, 10..13, 14..17]
        );
        assert_eq!(match_ranges("aaaa", "aa"), [0..2, 2..4]);
        assert!(match_ranges("anything", "").is_empty());
        assert!(match_ranges("", "x").is_empty());
        // Letters outside ASCII keep their case, and their bytes.
        assert_eq!(match_ranges("größe GRÖSSE", "größe"), vec![0..7; 1]);
        assert!(match_ranges("größe", "SSE").is_empty());
        assert_eq!(match_ranges("a🦑b🦑", "🦑"), [1..5, 6..10]);

        let m = model();
        let rows = m.rows(true);
        // On our side only, on theirs only, in the base, in what agrees.
        assert_eq!(matching_rows(&rows, "o1").len(), 1);
        assert_eq!(matching_rows(&rows, "T1").len(), 1);
        assert_eq!(matching_rows(&rows, "top"), [0]);
        assert!(matching_rows(&rows, "nowhere").is_empty());
        assert!(matching_rows(&rows, "").is_empty());
        // Words the panes add of their own are not text of the file.
        assert!(matching_rows(&rows, "Conflict").is_empty());
    }

    fn model() -> Model {
        Model::from_detail(&json!({
            "regions": [
                { "kind": "same", "lines": ["top\n"] },
                { "kind": "conflict", "ours": ["o1\n", "o2\n"], "base": ["b\n"], "theirs": ["t1\n"] },
                { "kind": "same", "lines": ["mid\n"] },
                { "kind": "conflict", "ours": [], "base": [], "theirs": ["t2"] },
            ]
        }))
    }

    #[test]
    fn the_result_follows_the_choices() {
        let mut m = model();
        assert_eq!(m.counts(), (2, 0));
        // Undecided conflicts are written as git writes them.
        assert_eq!(
            m.result(),
            "top\n<<<<<<< ours\no1\no2\n=======\nt1\n>>>>>>> theirs\nmid\n<<<<<<< ours\n=======\nt2\n>>>>>>> theirs\n"
        );
        m.take(0, true, false);
        m.take(1, false, true);
        assert_eq!(m.counts(), (2, 2));
        assert_eq!(m.result(), "top\no1\no2\nmid\nt2");
        // Both sides, ours first.
        m.take(0, true, true);
        assert_eq!(m.result(), "top\no1\no2\nt1\nmid\nt2");
        // Line by line.
        m.toggle(0, Side::Ours, 0);
        assert_eq!(m.result(), "top\no2\nt1\nmid\nt2");
        m.toggle(0, Side::Theirs, 0);
        m.toggle(0, Side::Ours, 1);
        assert_eq!(m.result(), "top\nmid\nt2", "neither side of the first");
        // Neither is a decision too.
        m.take(1, false, false);
        assert_eq!(m.result(), "top\nmid\n");
        assert_eq!(m.counts(), (2, 2));
        // A conflict or a line that is not there changes nothing.
        let before = m.clone();
        m.take(9, true, true);
        m.toggle(0, Side::Ours, 99);
        m.toggle(7, Side::Theirs, 0);
        assert_eq!(m, before);
    }

    #[test]
    fn a_last_line_without_its_break_gets_one_when_more_follows() {
        let mut m = Model::from_detail(&json!({
            "regions": [
                { "kind": "conflict", "ours": ["no break"], "base": [], "theirs": ["after\n"] },
            ]
        }));
        m.take(0, true, true);
        assert_eq!(m.result(), "no break\nafter\n");
    }

    #[test]
    fn line_endings_are_told_from_the_lines_not_from_the_markers() {
        let crlf = Model::from_detail(&json!({
            "regions": [
                { "kind": "same", "lines": ["a\r\n"] },
                { "kind": "conflict", "ours": ["o\r\n"], "base": ["b\n"], "theirs": ["t"] },
            ]
        }));
        assert!(
            crlf.crlf(),
            "the base and a last line without a break do not count"
        );
        let mixed = Model::from_detail(&json!({
            "regions": [
                { "kind": "same", "lines": ["a\r\n"] },
                { "kind": "conflict", "ours": ["o\n"], "base": [], "theirs": [] },
            ]
        }));
        assert!(!mixed.crlf());
        assert!(!Model::default().crlf());
        assert!(!model().crlf());
    }

    #[test]
    fn rows_pair_the_sides_and_show_both_files_whole() {
        let m = model();
        let rows = m.rows(false);
        // top, two rows of the first conflict, mid, one of the second.
        assert_eq!(rows.len(), 1 + 2 + 1 + 1);
        assert_eq!(
            rows[1],
            Row::Pair {
                conflict: 0,
                line: 0,
                ours: Some(("o1".to_owned(), false)),
                theirs: Some(("t1".to_owned(), false)),
            }
        );
        // Their side is shorter: nothing beside our second line.
        assert!(matches!(
            &rows[2],
            Row::Pair {
                line: 1,
                theirs: None,
                ..
            }
        ));
        // The base is shown on request, before the conflict's lines.
        let with_base = m.rows(true);
        assert_eq!(with_base[1], Row::Base("b".to_owned()));
        assert_eq!(with_base.len(), rows.len() + 1);

        // A long stretch both agree on is shown whole.
        let lines: Vec<String> = (0..20).map(|n| format!("l{n}\n")).collect();
        let long = Model::from_detail(&json!({ "regions": [{ "kind": "same", "lines": lines }] }));
        assert_eq!(long.rows(false).len(), 20);
        assert_eq!(long.result().lines().count(), 20);

        // Nothing to show for nothing, or for what is not a detail.
        assert!(Model::from_detail(&json!({})).rows(true).is_empty());
        assert!(
            Model::from_detail(&json!({ "regions": "no" }))
                .result()
                .is_empty()
        );
        assert!(
            Model::from_detail(&json!({ "regions": [{ "kind": 3 }, 7, null] }))
                .rows(false)
                .len()
                <= 3
        );
    }

    #[test]
    fn a_side_is_taken_whole_by_its_box_and_the_result_says_where_lines_are_from() {
        let mut m = model();
        // Not decided: what both sides started from stands for it.
        assert_eq!(
            m.result_rows(),
            [
                ("top".to_owned(), Origin::Same),
                ("b".to_owned(), Origin::Undecided(0)),
                ("mid".to_owned(), Origin::Same),
                (String::new(), Origin::Undecided(1)),
            ]
        );
        // The box of our side of the first conflict, then of all theirs.
        m.take_side(0, Side::Ours, true);
        assert!(m.side_taken(0, Side::Ours) && !m.side_taken(0, Side::Theirs));
        m.take_side_everywhere(Side::Theirs, true);
        assert!(m.side_taken_everywhere(Side::Theirs));
        // The second has no line of ours: decided, it counts as taken.
        assert!(m.side_taken_everywhere(Side::Ours));
        assert_eq!(
            m.result_rows(),
            [
                ("top".to_owned(), Origin::Same),
                ("o1".to_owned(), Origin::Taken(0, Side::Ours)),
                ("o2".to_owned(), Origin::Taken(0, Side::Ours)),
                ("t1".to_owned(), Origin::Taken(0, Side::Theirs)),
                ("mid".to_owned(), Origin::Same),
                ("t2".to_owned(), Origin::Taken(1, Side::Theirs)),
            ]
        );
        // Leaving a side again, and taking every choice back.
        m.take_side(0, Side::Ours, false);
        assert!(!m.side_taken(0, Side::Ours));
        m.reset();
        assert_eq!(m.counts(), (2, 0));
        assert_eq!(m, model());
    }

    #[test]
    fn the_result_starts_from_what_was_resolved_last_time_when_there_is_such() {
        use super::starting_text;
        let generated = "<<<<<<< ours\na\n=======\nb\n>>>>>>> theirs\n";
        // Nothing remembered: the text put together from the two sides.
        for detail in [json!({}), json!({ "reused": null }), json!({ "reused": 7 })] {
            assert_eq!(
                starting_text(&detail, generated, false),
                (generated.to_owned(), false),
                "{detail}"
            );
        }
        let detail = json!({ "reused": "both\r\nby hand\r\n" });
        assert_eq!(
            starting_text(&detail, generated, false),
            ("both\r\nby hand\r\n".to_owned(), true)
        );
        // In a file of the other line endings the field holds plain ones,
        // as it does for the text from the two sides.
        assert_eq!(
            starting_text(&detail, generated, true),
            ("both\nby hand\n".to_owned(), true)
        );
        // An empty resolution is one: the file resolved to nothing.
        assert_eq!(
            starting_text(&json!({ "reused": "" }), generated, false),
            (String::new(), true)
        );
    }
}
