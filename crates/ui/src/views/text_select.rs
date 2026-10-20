//! Selecting the text of a list of lines with the pointer, and copying
//! it, for the diff and the blame: a press and a drag over the lines'
//! text, two clicks for a word, three for the line, Shift with a click to
//! move the end that moves. Only the text of the lines is taken: no
//! numbers, signs or headings; with two columns, only the column the
//! selection began in.
//!
//! A line's text is drawn by [`LineText`], which paints what of it is
//! selected between the backgrounds of its runs and its letters, and
//! writes down where it was drawn and how it was laid out. The pointer is
//! turned into a place in a line from that ([`hit`]), so nothing is laid
//! out again for it, and only the rows in view are drawn again while the
//! selection grows.
//!
//! Sources: gpui's documentation of `TextLayout` (`paint_background`,
//! `paint_foreground`, `line_layouts`) and `LineLayout`
//! (`closest_index_for_x`, `x_for_index`); the terminal's selection
//! (`crates/terminal/src/session.rs`, `SelectOp`), whose clicks this
//! follows.

use std::{cell::RefCell, ops::Range, rc::Rc, sync::Arc};

use gpui_kit::*;

/// Which text of a row: its only one, or one of its two columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Column {
    /// The one text of the row.
    Only,
    /// The left column: the old side.
    Old,
    /// The right column: the new side.
    New,
}

/// A place in the text: a row of the list, and a byte of its line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct TextPoint {
    /// The row.
    pub row: usize,
    /// The byte in the row's line, on a character's boundary.
    pub offset: usize,
}

impl TextPoint {
    /// The place `offset` in `row`.
    pub fn new(row: usize, offset: usize) -> Self {
        Self { row, offset }
    }
}

/// How much a selection takes at a time: what the click that began it
/// took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Characters: a press and a drag.
    Character,
    /// Words: two clicks.
    Word,
    /// Lines: three clicks.
    Line,
}

/// Text selected in one column.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextSelection {
    /// The column it is in.
    pub column: Column,
    /// What the press took: one place, or the word or the line it was in.
    anchor: (TextPoint, TextPoint),
    /// The end that moves.
    head: TextPoint,
    reach: Reach,
}

impl TextSelection {
    /// A selection begun at a press: `unit` is what the press took (the
    /// place itself for characters, else the word or line it is in).
    pub fn new(column: Column, unit: (TextPoint, TextPoint), reach: Reach) -> Self {
        Self {
            column,
            anchor: unit,
            head: unit.1,
            reach,
        }
    }

    /// Its two ends, the first one first.
    pub fn span(&self) -> (TextPoint, TextPoint) {
        let (from, to) = self.anchor;
        if self.head < from {
            (self.head, to)
        } else if self.head > to {
            (from, self.head)
        } else {
            (from, to)
        }
    }

    /// Whether it holds nothing.
    pub fn is_empty(&self) -> bool {
        let (from, to) = self.span();
        from == to
    }

    /// How much it takes at a time.
    pub fn reach(&self) -> Reach {
        self.reach
    }

    /// Move the end that moves to `place`, whose word or line (as the
    /// selection takes them) is `unit`. Whether anything changed.
    pub fn extend(&mut self, place: TextPoint, unit: (TextPoint, TextPoint)) -> bool {
        let head = match self.reach {
            Reach::Character => place,
            Reach::Word | Reach::Line if place < self.anchor.0 => unit.0,
            Reach::Word | Reach::Line => unit.1,
        };
        let changed = head != self.head;
        self.head = head;
        changed
    }

    /// Shift with a click: the selection reaches to `place` by characters,
    /// from its far end when `place` lies outside it, else from where it
    /// began.
    pub fn extend_to(&mut self, place: TextPoint) -> bool {
        let (from, to) = self.span();
        // The end nearer the click moves; the other one stays.
        self.anchor = if place < from {
            (to, to)
        } else if place > to {
            (from, from)
        } else {
            self.anchor
        };
        self.reach = Reach::Character;
        let changed = self.head != place;
        self.head = place;
        changed
    }

    /// What of the line of `row` in `column` (`len` bytes long) is
    /// selected, and whether the selection goes on past its end, where its
    /// line break is taken too. None when nothing of it is.
    pub fn in_row(&self, row: usize, column: Column, len: usize) -> Option<(Range<usize>, bool)> {
        if column != self.column {
            return None;
        }
        let (from, to) = self.span();
        if from == to || row < from.row || row > to.row {
            return None;
        }
        let start = if row == from.row {
            from.offset.min(len)
        } else {
            0
        };
        let end = if row == to.row {
            to.offset.min(len)
        } else {
            len
        };
        let on = row < to.row;
        (start < end || (start == end && on)).then_some((start..end.max(start), on))
    }

    /// The text it holds, the lines of the rows read by `line` (none for a
    /// row without text in the column: a heading, the empty side of a
    /// row). A line ends with a break where the selection goes on past it.
    pub fn text<'a>(&self, line: impl Fn(usize, Column) -> Option<&'a str>) -> String {
        let (from, to) = self.span();
        let mut out = String::new();
        for row in from.row..=to.row {
            let Some(text) = line(row, self.column) else {
                continue;
            };
            let Some((range, on)) = self.in_row(row, self.column, text.len()) else {
                continue;
            };
            let whole = range.end >= text.len();
            if let Some(part) = text.get(range) {
                // A line that ends in a carriage return has it drawn as a
                // mark of its own, not as text.
                out.push_str(if whole {
                    part.strip_suffix('\r').unwrap_or(part)
                } else {
                    part
                });
            }
            if on {
                out.push('\n');
            }
        }
        out
    }
}

/// What a view keeps of the selection of its lines' text: the selection,
/// whether it grows (the button is held since it was pressed on the
/// text), and where the lines were drawn.
#[derive(Default)]
pub struct Selecting {
    selection: Option<TextSelection>,
    growing: bool,
    drawn: DrawnLines,
}

impl Selecting {
    /// Where the lines are written down as they are drawn.
    pub fn drawn(&self) -> DrawnLines {
        self.drawn.clone()
    }

    /// Let the selection go. Whether there was one.
    pub fn clear(&mut self) -> bool {
        self.growing = false;
        self.selection.take().is_some()
    }

    /// What of the line of `row` in `column` is selected (see
    /// [`TextSelection::in_row`]).
    pub fn in_row(&self, row: usize, column: Column, len: usize) -> Option<(Range<usize>, bool)> {
        self.selection.as_ref()?.in_row(row, column, len)
    }

    /// Whether some text is selected.
    pub fn has_text(&self) -> bool {
        self.selection
            .as_ref()
            .is_some_and(|selection| !selection.is_empty())
    }

    /// The selected text, the lines read by `line`; none when nothing is.
    pub fn text<'a>(&self, line: impl Fn(usize, Column) -> Option<&'a str>) -> Option<String> {
        let selection = self.selection.as_ref().filter(|s| !s.is_empty())?;
        Some(selection.text(line))
    }

    /// The pointer went down: on a line's text a selection begins there
    /// (two clicks take a word, three the line; with Shift the end of the
    /// one there is moves there); anywhere else the selection goes.
    /// Whether anything changed.
    pub fn press<'a>(
        &mut self,
        event: &MouseDownEvent,
        line: impl Fn(usize, Column) -> Option<&'a str>,
    ) -> bool {
        let Some((column, place)) = hit(&self.drawn, event.position, None) else {
            return self.clear();
        };
        let extended = event.modifiers.shift
            && self
                .selection
                .as_mut()
                .filter(|selection| selection.column == column)
                .map(|selection| selection.extend_to(place))
                .is_some();
        if !extended {
            let reach = match event.click_count {
                2 => Reach::Word,
                count if count >= 3 => Reach::Line,
                _ => Reach::Character,
            };
            let text = line(place.row, column).unwrap_or_default();
            self.selection = Some(TextSelection::new(
                column,
                unit_at(text, place, reach),
                reach,
            ));
        }
        self.growing = true;
        true
    }

    /// The pointer is at `position` with its button held: a selection
    /// that grows follows it. Whether anything changed.
    pub fn drag<'a>(
        &mut self,
        position: Point<Pixels>,
        line: impl Fn(usize, Column) -> Option<&'a str>,
    ) -> bool {
        if !self.growing {
            return false;
        }
        let Some(selection) = self.selection.as_mut() else {
            return false;
        };
        let Some((_, place)) = hit(&self.drawn, position, Some(selection.column)) else {
            return false;
        };
        let text = line(place.row, selection.column).unwrap_or_default();
        selection.extend(place, unit_at(text, place, selection.reach()))
    }

    /// Whether a selection grows: the pointer's button is held since it
    /// went down on the text.
    pub fn is_growing(&self) -> bool {
        self.growing
    }

    /// The pointer's button went up. Whether a selection was growing.
    pub fn release(&mut self) -> bool {
        std::mem::take(&mut self.growing)
    }
}

/// What a press or a drag at `place` takes as it reaches: the place
/// itself, the word there, or the whole line (`text`).
fn unit_at(text: &str, place: TextPoint, reach: Reach) -> (TextPoint, TextPoint) {
    let range = match reach {
        Reach::Character => place.offset..place.offset,
        Reach::Word => word_at(text, place.offset),
        Reach::Line => 0..text.len(),
    };
    (
        TextPoint::new(place.row, range.start),
        TextPoint::new(place.row, range.end),
    )
}

/// Put text where a middle click pastes it elsewhere on the desktop, on
/// the systems that have such a place.
pub fn offer_primary(text: Option<String>, cx: &mut App) {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    if let Some(text) = text {
        cx.write_to_primary(ClipboardItem::new_string(text));
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    let _ = (text, cx);
}

/// The kind of a character, for what two clicks take: a run of letters,
/// digits and underscores; a run of blanks; or a run of anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Word,
    Blank,
    Other,
}

fn kind_of(ch: char) -> Kind {
    if ch.is_alphanumeric() || ch == '_' {
        Kind::Word
    } else if ch.is_whitespace() {
        Kind::Blank
    } else {
        Kind::Other
    }
}

/// The word at `offset` of `text`, as two clicks take it: the run of
/// characters of one kind the character there is in (the one before it
/// at the end of the line). Empty for an empty line.
pub fn word_at(text: &str, offset: usize) -> Range<usize> {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset = offset.saturating_sub(1);
    }
    let after = text.get(offset..).and_then(|rest| rest.chars().next());
    let before = text.get(..offset).and_then(|head| head.chars().next_back());
    let Some(kind) = after.or(before).map(kind_of) else {
        return offset..offset;
    };
    // At the end of a line, or between two kinds where the one after is
    // blank and the one before is not, the word is the one before.
    let at = match (before, after) {
        (Some(b), Some(a)) if kind_of(a) == Kind::Blank && kind_of(b) != Kind::Blank => {
            offset.saturating_sub(b.len_utf8())
        }
        (Some(b), None) => offset.saturating_sub(b.len_utf8()),
        _ => offset,
    };
    let kind = text
        .get(at..)
        .and_then(|rest| rest.chars().next())
        .map(kind_of)
        .unwrap_or(kind);
    let start = text
        .get(..at)
        .map(|head| {
            head.char_indices()
                .rev()
                .take_while(|(_, ch)| kind_of(*ch) == kind)
                .last()
                .map_or(at, |(ix, _)| ix)
        })
        .unwrap_or(at);
    let end = text
        .get(at..)
        .map(|rest| {
            rest.char_indices()
                .find(|(_, ch)| kind_of(*ch) != kind)
                .map_or(text.len(), |(ix, _)| at.saturating_add(ix))
        })
        .unwrap_or(at);
    start..end
}

/// Where a piece of a line's text was drawn in the last frame.
#[derive(Clone)]
pub struct Drawn {
    /// Its row and column.
    pub row: usize,
    pub column: Column,
    /// Where in the line the piece begins: a wrapped line is drawn in
    /// pieces, one under the other.
    pub start: usize,
    /// The text's own box, and the part of the window it is seen in (the
    /// box of its column, cut by the list).
    pub bounds: Bounds<Pixels>,
    pub seen: Bounds<Pixels>,
    /// How it was laid out: where each character stands.
    pub layout: Option<Arc<LineLayout>>,
}

impl Drawn {
    /// How long the piece is.
    fn len(&self) -> usize {
        self.layout.as_ref().map_or(0, |layout| layout.len)
    }

    /// The place in the line nearest to `x`.
    fn offset_at(&self, x: Pixels) -> usize {
        let within = self.layout.as_ref().map_or(0, |layout| {
            layout.closest_index_for_x(x - self.bounds.left())
        });
        self.start.saturating_add(within.min(self.len()))
    }
}

/// What was drawn of the lines in the last frame, emptied as the list is
/// drawn anew and filled by its lines.
pub type DrawnLines = Rc<RefCell<Vec<Drawn>>>;

/// Forget what was drawn, at the start of a frame.
pub fn forget_drawn(drawn: &DrawnLines) {
    if let Ok(mut drawn) = drawn.try_borrow_mut() {
        drawn.clear();
    }
}

/// The place under the pointer at `position`: for a press (`column`
/// none), only on a line's text, in the column it is in; while a
/// selection grows, in its column, the nearest line above or below
/// where the pointer is between lines or outside the list.
pub fn hit(
    drawn: &DrawnLines,
    position: Point<Pixels>,
    column: Option<Column>,
) -> Option<(Column, TextPoint)> {
    let drawn = drawn.try_borrow().ok()?;
    let within =
        |piece: &&Drawn| position.y >= piece.bounds.top() && position.y < piece.bounds.bottom();
    let Some(column) = column else {
        let piece = drawn
            .iter()
            .filter(within)
            .find(|piece| position.x >= piece.seen.left() && position.x < piece.seen.right())?;
        return Some((
            piece.column,
            TextPoint::new(piece.row, piece.offset_at(position.x)),
        ));
    };
    let mine = || drawn.iter().filter(|piece| piece.column == column);
    if let Some(piece) = mine().find(within) {
        return Some((
            column,
            TextPoint::new(piece.row, piece.offset_at(position.x)),
        ));
    }
    // Between lines, or past the first or the last one drawn: the
    // nearest line below, at its start, else the nearest above, at its
    // end.
    let below = mine()
        .filter(|piece| piece.bounds.top() >= position.y)
        .min_by(|a, b| {
            a.bounds
                .top()
                .partial_cmp(&b.bounds.top())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    if let Some(piece) = below {
        return Some((column, TextPoint::new(piece.row, piece.start)));
    }
    let above = mine()
        .filter(|piece| piece.bounds.bottom() <= position.y)
        .max_by(|a, b| {
            (a.bounds.bottom(), a.start)
                .partial_cmp(&(b.bounds.bottom(), b.start))
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    Some((
        column,
        TextPoint::new(above.row, above.start.saturating_add(above.len())),
    ))
}

/// Hear the pointer's moves with its button held, and its release,
/// wherever they are in the window: a selection goes on growing while the
/// pointer is outside the list. Called while the list is painted.
pub fn follow_pointer<V: 'static>(
    view: WeakEntity<V>,
    moved: fn(&mut V, Point<Pixels>, &mut Context<V>),
    released: fn(&mut V, &mut Context<V>),
    window: &mut Window,
) {
    let moving = view.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, _, cx| {
        if phase != DispatchPhase::Bubble || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        let _ = moving.update(cx, |view, cx| moved(view, event.position, cx));
    });
    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
        if phase != DispatchPhase::Bubble {
            return;
        }
        let _ = view.update(cx, released);
    });
}

/// The text of one line, or of a piece of a wrapped one, as the toolkit
/// draws it, with what of it is selected painted under its letters.
pub struct LineText {
    text: StyledText,
    row: usize,
    column: Column,
    start: usize,
    /// What of the piece is selected, counted from its start, and whether
    /// the selection goes on past its end (its line break is shown).
    selected: Option<(Range<usize>, bool)>,
    colour: Hsla,
    drawn: DrawnLines,
}

impl LineText {
    /// The piece of the line of `row` in `column` that begins at `start`.
    pub fn new(
        text: StyledText,
        (row, column, start): (usize, Column, usize),
        selected: Option<(Range<usize>, bool)>,
        colour: Hsla,
        drawn: DrawnLines,
    ) -> Self {
        Self {
            text,
            row,
            column,
            start,
            selected,
            colour,
            drawn,
        }
    }
}

impl IntoElement for LineText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for LineText {
    type RequestLayoutState = ();
    type PrepaintState = Option<Arc<LineLayout>>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.text.request_layout(id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        state: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.text
            .prepaint(id, inspector_id, bounds, state, window, cx);
        // Laid out now: the toolkit measured it before it was placed.
        let layout = self
            .text
            .layout()
            .line_layouts()
            .first()
            .map(|line| line.unwrapped_layout.clone());
        if let Ok(mut drawn) = self.drawn.try_borrow_mut() {
            drawn.push(Drawn {
                row: self.row,
                column: self.column,
                start: self.start,
                bounds,
                seen: window.content_mask().bounds,
                layout: layout.clone(),
            });
        }
        layout
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        layout: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let text = self.text.layout().clone();
        let _ = text.paint_background(window, cx);
        if let Some((range, on)) = self.selected.clone() {
            let x = |index: usize| {
                layout
                    .as_ref()
                    .map_or(px(0.), |layout| layout.x_for_index(index))
            };
            // The line break is a character's width past the end.
            let size = layout.as_ref().map_or(px(0.), |layout| layout.font_size);
            let left = bounds.left() + x(range.start);
            let mut right = bounds.left() + x(range.end);
            if on {
                right += size * 0.6;
            }
            if right > left {
                window.paint_quad(fill(
                    Bounds::from_corners(point(left, bounds.top()), point(right, bounds.bottom())),
                    self.colour,
                ));
            }
        }
        let _ = text.paint_foreground(window, cx);
    }
}

/// What of a piece of a line (`piece`, bytes of the line) is selected,
/// counted from the piece's start, given what of the line is; the line
/// break is shown after the last piece only.
pub fn in_piece(
    line: &Option<(Range<usize>, bool)>,
    piece: &Range<usize>,
    last: bool,
) -> Option<(Range<usize>, bool)> {
    let (range, on) = line.as_ref()?;
    let on = *on && last;
    let start = range.start.max(piece.start);
    let end = range.end.min(piece.end);
    if start > end || (start == end && !(on && start == piece.end)) {
        return None;
    }
    Some((
        start.saturating_sub(piece.start)..end.saturating_sub(piece.start),
        on,
    ))
}

#[cfg(test)]
mod tests {
    use super::{Column, Reach, TextPoint, TextSelection, in_piece, word_at};

    fn place(row: usize, offset: usize) -> TextPoint {
        TextPoint::new(row, offset)
    }

    #[test]
    fn a_drag_selects_from_where_it_began_to_where_it_is_either_way() {
        let mut selection =
            TextSelection::new(Column::Only, (place(2, 3), place(2, 3)), Reach::Character);
        assert!(selection.is_empty());
        assert!(selection.extend(place(4, 1), (place(4, 1), place(4, 1))));
        assert_eq!(selection.span(), (place(2, 3), place(4, 1)));
        selection.extend(place(1, 5), (place(1, 5), place(1, 5)));
        assert_eq!(selection.span(), (place(1, 5), place(2, 3)));
        assert_eq!(selection.in_row(1, Column::Only, 10), Some((5..10, true)));
        assert_eq!(selection.in_row(2, Column::Only, 10), Some((0..3, false)));
        assert_eq!(selection.in_row(3, Column::Only, 10), None);
        assert_eq!(selection.in_row(2, Column::New, 10), None, "one column");
    }

    #[test]
    fn the_text_is_the_lines_without_what_has_none_and_breaks_where_it_goes_on() {
        let lines = ["fn main() {", "", "    body();\r", "}"];
        let line = |row: usize, _| match row {
            // A heading: nothing of its own.
            1 => None,
            row => lines.get(row).copied(),
        };
        let mut selection =
            TextSelection::new(Column::Only, (place(0, 3), place(0, 3)), Reach::Character);
        selection.extend(place(3, 1), (place(3, 1), place(3, 1)));
        assert_eq!(selection.text(line), "main() {\n    body();\n}");
        // To the start of the next line: the break is taken, nothing more.
        selection.extend(place(3, 0), (place(3, 0), place(3, 0)));
        assert_eq!(selection.text(line), "main() {\n    body();\n");
    }

    #[test]
    fn two_clicks_take_a_word_and_a_drag_goes_on_by_words() {
        let text = "let value_5 = 5 * 2;";
        assert_eq!(word_at(text, 6), 4..11, "letters, digits, underscores");
        assert_eq!(word_at(text, 4), 4..11);
        assert_eq!(word_at(text, 11), 4..11, "right after it");
        assert_eq!(word_at(text, 12), 12..13, "a sign");
        assert_eq!(word_at(text, text.len()), 19..20, "the end of the line");
        assert_eq!(word_at("", 0), 0..0);
        assert_eq!(word_at("a  b", 2), 1..3, "blanks");
        assert_eq!(word_at("añb c", 2), 0..4, "on a character's boundary");

        let unit =
            |row, range: std::ops::Range<usize>| (place(row, range.start), place(row, range.end));
        let mut selection = TextSelection::new(Column::Only, unit(0, 4..11), Reach::Word);
        assert_eq!(selection.span(), (place(0, 4), place(0, 11)));
        selection.extend(place(1, 2), unit(1, 0..3));
        assert_eq!(selection.span(), (place(0, 4), place(1, 3)));
        selection.extend(place(0, 1), unit(0, 0..3));
        assert_eq!(
            selection.span(),
            (place(0, 0), place(0, 11)),
            "back past it"
        );
    }

    #[test]
    fn shift_moves_the_nearer_end() {
        let mut selection =
            TextSelection::new(Column::Old, (place(3, 2), place(3, 2)), Reach::Character);
        selection.extend(place(5, 0), (place(5, 0), place(5, 0)));
        selection.extend_to(place(6, 4));
        assert_eq!(selection.span(), (place(3, 2), place(6, 4)));
        selection.extend_to(place(1, 0));
        assert_eq!(selection.span(), (place(1, 0), place(6, 4)));
    }

    #[test]
    fn a_piece_of_a_wrapped_line_has_its_part_and_the_last_the_break() {
        let line = Some((3..12, true));
        assert_eq!(in_piece(&line, &(0..5), false), Some((3..5, false)));
        assert_eq!(in_piece(&line, &(5..10), false), Some((0..5, false)));
        assert_eq!(in_piece(&line, &(10..14), true), Some((0..2, true)));
        assert_eq!(
            in_piece(&Some((3..14, true)), &(14..14), true),
            Some((0..0, true))
        );
        assert_eq!(in_piece(&Some((0..3, false)), &(5..10), false), None);
        assert_eq!(in_piece(&None, &(0..5), true), None);
    }
}
