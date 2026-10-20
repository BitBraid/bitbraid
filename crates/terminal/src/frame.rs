//! What is on show: a copy of the screen with every colour worked out,
//! made while the screen is locked and drawn after it is let go.
//!
//! Colours come from the program as one of the 16 named colours, an index
//! into the 256 (the 16, a 6×6×6 cube and 24 greys), or red, green and
//! blue. A program may also change a colour of the palette (OSC 4, 10, 11,
//! 12); what it changed counts before the [`Palette`] the window gives.
//! Inverse swaps the two colours of a cell, hidden draws its text in its
//! background, dim mixes the text a third of the way into the background.
//! A selected cell takes the selection's colours. Text is then moved
//! towards black or white as far as it must be to read on its background
//! (`contrast`).
//!
//! [`Row::runs`] and [`Row::backgrounds`] group a row for drawing: text in
//! runs of one colour and style, backgrounds in spans of one colour with
//! the default left out, so a full screen is a few dozen shapes rather
//! than thousands.
//!
//! Sources: XTerm Control Sequences, "Character Attributes (SGR)" and
//! "Operating System Commands" (invisible-island.net/xterm/ctlseqs);
//! `alacritty_terminal` 0.26, `term::RenderableContent` and
//! `term::cell::Flags`; the 256-colour cube and grey ramp as xterm's
//! `256colres.pl` defines them.

use crate::contrast::{self, Remembered};

use alacritty_terminal::{
    event::EventListener,
    grid::Dimensions as _,
    term::{
        Term, TermMode,
        cell::{Cell as GridCell, Flags},
        color::Colors,
    },
    vte::ansi::{Color, CursorShape as ProgramShape, NamedColor},
};

/// A colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Rgb {
    /// Red.
    pub r: u8,
    /// Green.
    pub g: u8,
    /// Blue.
    pub b: u8,
}

impl Rgb {
    /// A colour from its three parts.
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// A colour from `0xRRGGBB`.
    pub const fn hex(value: u32) -> Self {
        Self {
            r: ((value >> 16) & 0xff) as u8,
            g: ((value >> 8) & 0xff) as u8,
            b: (value & 0xff) as u8,
        }
    }

    /// This colour mixed with `other`: none of it at 0, all of it at 1.
    pub fn mix(self, other: Rgb, amount: f32) -> Rgb {
        let amount = if amount.is_finite() {
            amount.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let part = |from: u8, to: u8| -> u8 {
            let value = f32::from(from) + (f32::from(to) - f32::from(from)) * amount;
            value.round().clamp(0.0, 255.0) as u8
        };
        Rgb::new(
            part(self.r, other.r),
            part(self.g, other.g),
            part(self.b, other.b),
        )
    }

    /// As `0xRRGGBB`.
    pub fn to_hex(self) -> u32 {
        (u32::from(self.r) << 16) | (u32::from(self.g) << 8) | u32::from(self.b)
    }
}

/// The colours a terminal is drawn in.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Palette {
    /// The 16 named colours: black, red, green, yellow, blue, magenta,
    /// cyan, white, then their bright forms.
    pub ansi: [Rgb; 16],
    /// Text that has no colour of its own.
    pub foreground: Rgb,
    /// The background of cells that have none of their own.
    pub background: Rgb,
    pub cursor: Rgb,
    /// The character under a block cursor.
    pub cursor_text: Rgb,
    /// Selected text; its own colour stays when none.
    pub selection_foreground: Option<Rgb>,
    /// Behind selected text.
    pub selection_background: Rgb,
    /// Dim text without a colour of its own; the foreground mixed into
    /// the background when none.
    pub dim_foreground: Option<Rgb>,
    /// Bold text without a colour of its own; the foreground when none.
    pub bright_foreground: Option<Rgb>,
}

impl Default for Palette {
    fn default() -> Self {
        Self::dark()
    }
}

impl Palette {
    /// A neutral palette on a dark background.
    pub fn dark() -> Self {
        Self {
            ansi: [
                Rgb::hex(0x2b2d33),
                Rgb::hex(0xdc6a64),
                Rgb::hex(0x8fbf6f),
                Rgb::hex(0xd8b45e),
                Rgb::hex(0x689fe0),
                Rgb::hex(0xb984d4),
                Rgb::hex(0x56b6bd),
                Rgb::hex(0xc8cad0),
                Rgb::hex(0x6c707a),
                Rgb::hex(0xee8580),
                Rgb::hex(0xa8d68c),
                Rgb::hex(0xeccb7c),
                Rgb::hex(0x88b6ef),
                Rgb::hex(0xcd9fe3),
                Rgb::hex(0x75ccd2),
                Rgb::hex(0xf1f2f4),
            ],
            foreground: Rgb::hex(0xd6d8dd),
            background: Rgb::hex(0x1b1c20),
            cursor: Rgb::hex(0xd6d8dd),
            cursor_text: Rgb::hex(0x1b1c20),
            selection_foreground: None,
            selection_background: Rgb::hex(0x3b4658),
            dim_foreground: None,
            bright_foreground: None,
        }
    }

    /// A neutral palette on a light background.
    pub fn light() -> Self {
        Self {
            ansi: [
                Rgb::hex(0x24262b),
                Rgb::hex(0xb73c35),
                Rgb::hex(0x3f7d2a),
                Rgb::hex(0x8a6a12),
                Rgb::hex(0x2c63b0),
                Rgb::hex(0x8a43a8),
                Rgb::hex(0x1f7c83),
                Rgb::hex(0xb9bcc3),
                Rgb::hex(0x666a73),
                Rgb::hex(0xd3534b),
                Rgb::hex(0x55993c),
                Rgb::hex(0xa8831d),
                Rgb::hex(0x4479c7),
                Rgb::hex(0xa35bc2),
                Rgb::hex(0x2c959d),
                Rgb::hex(0xe4e6ea),
            ],
            foreground: Rgb::hex(0x24262b),
            background: Rgb::hex(0xfbfbfc),
            cursor: Rgb::hex(0x24262b),
            cursor_text: Rgb::hex(0xfbfbfc),
            selection_foreground: None,
            selection_background: Rgb::hex(0xc9d7ee),
            dim_foreground: None,
            bright_foreground: None,
        }
    }

    /// Colour `index` of the 256, as the palette has it: the 16 named
    /// ones, the cube and the greys.
    pub fn indexed(&self, index: u8) -> Rgb {
        if let Some(named) = self.ansi.get(usize::from(index)) {
            return *named;
        }
        if index >= 232 {
            let grey = 8u8.saturating_add((index - 232).saturating_mul(10));
            return Rgb::new(grey, grey, grey);
        }
        let cube = index - 16;
        let level = |step: u8| -> u8 {
            if step == 0 {
                0
            } else {
                55u8.saturating_add(step.saturating_mul(40))
            }
        };
        Rgb::new(level(cube / 36), level((cube / 6) % 6), level(cube % 6))
    }
}

/// How a cell's text is underlined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Underline {
    /// Not at all.
    #[default]
    None,
    /// Once; also dotted and dashed lines, drawn as one.
    Single,
    /// Twice.
    Double,
    /// A wavy line.
    Curly,
}

/// How a cell's text is set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: Underline,
    /// The underline's own colour (SGR 58); the text's when none.
    pub underline_color: Option<Rgb>,
    /// Struck through.
    pub strikeout: bool,
}

/// How wide a cell's character is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CellWidth {
    /// One cell.
    #[default]
    Normal,
    /// Two cells: this one and the spacer after it.
    Wide,
    /// The second cell of a wide character, or the empty cell at the end
    /// of a row that a wide character did not fit in; draws no text.
    Spacer,
}

/// One cell of the screen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Cell {
    /// Its character; a space when it has none.
    pub character: char,
    /// Characters of no width drawn on it (combining marks, joiners).
    pub zero_width: Option<Box<str>>,
    /// The colour of its text.
    pub foreground: Rgb,
    pub background: Rgb,
    /// How its text is set.
    pub style: Style,
    /// How wide it is.
    pub width: CellWidth,
    /// Whether it is selected.
    pub selected: bool,
}

/// One row of the screen.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    /// Its cells, one per column.
    pub cells: Vec<Cell>,
}

/// Text of one colour and style, to be drawn from a column on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    /// The column it starts in.
    pub column: u16,
    /// How many columns it covers.
    pub columns: u16,
    /// Its text, with the zero-width characters after the ones they go
    /// with.
    pub text: String,
    /// The colour of its text.
    pub foreground: Rgb,
    /// How it is set.
    pub style: Style,
    /// Whether it is one wide character, which is centred over its two
    /// columns rather than set from the first.
    pub wide: bool,
}

/// Columns of one background colour.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    /// The first column.
    pub column: u16,
    /// How many columns.
    pub columns: u16,
    pub color: Rgb,
}

impl Row {
    /// The row's text in runs of one colour and style, left to right.
    /// Blanks that draw nothing (no underline, no strike) are left out, a
    /// wide character is a run of its own, and a spacer is skipped.
    pub fn runs(&self) -> Vec<Run> {
        let mut out: Vec<Run> = Vec::new();
        let mut current: Option<Run> = None;
        let flush = |run: Option<Run>, out: &mut Vec<Run>| {
            if let Some(run) = run {
                let visible = run.style.underline != Underline::None
                    || run.style.strikeout
                    || run.text.chars().any(|c| c != ' ');
                if visible {
                    out.push(run);
                }
            }
        };
        for (column, cell) in self.cells.iter().enumerate() {
            let Ok(column) = u16::try_from(column) else {
                break;
            };
            match cell.width {
                CellWidth::Spacer => {
                    flush(current.take(), &mut out);
                    continue;
                }
                CellWidth::Wide => {
                    flush(current.take(), &mut out);
                    let mut text = String::new();
                    text.push(cell.character);
                    if let Some(extra) = &cell.zero_width {
                        text.push_str(extra);
                    }
                    flush(
                        Some(Run {
                            column,
                            columns: 2,
                            text,
                            foreground: cell.foreground,
                            style: cell.style,
                            wide: true,
                        }),
                        &mut out,
                    );
                    continue;
                }
                CellWidth::Normal => {}
            }
            let same = current
                .as_ref()
                .is_some_and(|run| run.foreground == cell.foreground && run.style == cell.style);
            if !same {
                flush(current.take(), &mut out);
                current = Some(Run {
                    column,
                    columns: 0,
                    text: String::new(),
                    foreground: cell.foreground,
                    style: cell.style,
                    wide: false,
                });
            }
            if let Some(run) = current.as_mut() {
                run.columns = run.columns.saturating_add(1);
                run.text.push(cell.character);
                if let Some(extra) = &cell.zero_width {
                    run.text.push_str(extra);
                }
            }
        }
        flush(current.take(), &mut out);
        out
    }

    /// The backgrounds of the row that are not `default`, in spans of one
    /// colour, left to right.
    pub fn backgrounds(&self, default: Rgb) -> Vec<Span> {
        let mut out: Vec<Span> = Vec::new();
        for (column, cell) in self.cells.iter().enumerate() {
            let Ok(column) = u16::try_from(column) else {
                break;
            };
            if cell.background == default {
                continue;
            }
            match out.last_mut() {
                Some(span)
                    if span.color == cell.background
                        && span.column.saturating_add(span.columns) == column =>
                {
                    span.columns = span.columns.saturating_add(1);
                }
                _ => out.push(Span {
                    column,
                    columns: 1,
                    color: cell.background,
                }),
            }
        }
        out
    }

    /// The row's characters, with blanks at the end left out.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for cell in &self.cells {
            if cell.width == CellWidth::Spacer {
                continue;
            }
            out.push(cell.character);
            if let Some(extra) = &cell.zero_width {
                out.push_str(extra);
            }
        }
        out.truncate(out.trim_end().len());
        out
    }
}

/// The shape a program asked the cursor to have.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CursorShape {
    /// A filled cell.
    Block,
    /// A line under the cell.
    Underline,
    /// A thin line before the cell.
    Bar,
    /// The outline of a cell.
    HollowBlock,
}

/// The cursor, when it is on show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    /// Its row on screen.
    pub row: u16,
    pub column: u16,
    /// The shape the program asked for; none when it asked for none, and
    /// the user's choice applies.
    pub shape: Option<CursorShape>,
    /// Whether it stands on a wide character (two cells).
    pub wide: bool,
    pub color: Rgb,
    /// The colour of the character under it when it is a block.
    pub text_color: Rgb,
    /// The character under it.
    pub character: char,
}

/// What the program asked of the terminal that the window needs to know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modes {
    /// The alternate screen is on show (full-screen programs).
    pub alternate_screen: bool,
    /// Cursor keys send their application form (DECCKM).
    pub application_cursor: bool,
    /// The keypad sends its application form (DECKPAM).
    pub application_keypad: bool,
    /// Pasted text is to be bracketed (2004).
    pub bracketed_paste: bool,
    /// The mouse is reported (1000, 1002 or 1003).
    pub mouse_reporting: bool,
    /// Motion with a button held is reported (1002).
    pub mouse_drag: bool,
    /// All motion is reported (1003).
    pub mouse_motion: bool,
    /// Reports are written in SGR form (1006).
    pub sgr_mouse: bool,
    /// Reports are written in UTF-8 (1005).
    pub utf8_mouse: bool,
    /// The wheel in the alternate screen sends arrow keys (1007).
    pub alternate_scroll: bool,
    /// Focus changes are reported (1004).
    pub focus_reporting: bool,
    /// Return sends a line feed too (LNM).
    pub line_feed_new_line: bool,
    /// The cursor is on show (DECTCEM).
    pub show_cursor: bool,
}

impl Modes {
    pub(crate) fn of(mode: TermMode) -> Self {
        Self {
            alternate_screen: mode.contains(TermMode::ALT_SCREEN),
            application_cursor: mode.contains(TermMode::APP_CURSOR),
            application_keypad: mode.contains(TermMode::APP_KEYPAD),
            bracketed_paste: mode.contains(TermMode::BRACKETED_PASTE),
            mouse_reporting: mode.intersects(TermMode::MOUSE_MODE),
            mouse_drag: mode.contains(TermMode::MOUSE_DRAG),
            mouse_motion: mode.contains(TermMode::MOUSE_MOTION),
            sgr_mouse: mode.contains(TermMode::SGR_MOUSE),
            utf8_mouse: mode.contains(TermMode::UTF8_MOUSE),
            alternate_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            focus_reporting: mode.contains(TermMode::FOCUS_IN_OUT),
            line_feed_new_line: mode.contains(TermMode::LINE_FEED_NEW_LINE),
            show_cursor: mode.contains(TermMode::SHOW_CURSOR),
        }
    }
}

/// What is on show.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frame {
    /// The rows from the top.
    pub rows: Vec<Row>,
    /// Columns per row.
    pub columns: u16,
    /// The cursor, when it is on show: not when the program hid it or the
    /// view is scrolled back past it.
    pub cursor: Option<Cursor>,
    /// How many lines the view is scrolled back into history.
    pub display_offset: usize,
    /// How many lines of history there are.
    pub history: usize,
    /// The modes the program set.
    pub mode: Modes,
    /// The background of cells that have none of their own (the palette's,
    /// or what the program set it to).
    pub background: Rgb,
    /// The text colour of cells that have none of their own.
    pub foreground: Rgb,
}

impl Frame {
    /// The text on show, a line per row, blanks at the ends left out.
    pub fn text(&self) -> String {
        let lines: Vec<String> = self.rows.iter().map(Row::text).collect();
        lines.join("\n")
    }
}

/// The colour `color` stands for, with what the program changed first.
fn resolve(color: Color, colors: &Colors, palette: &Palette) -> Rgb {
    match color {
        Color::Spec(rgb) => Rgb::new(rgb.r, rgb.g, rgb.b),
        Color::Indexed(index) => {
            overridden(colors, usize::from(index)).unwrap_or_else(|| palette.indexed(index))
        }
        Color::Named(named) => named_color(named, colors, palette),
    }
}

fn overridden(colors: &Colors, index: usize) -> Option<Rgb> {
    // `Colors` panics on an index past its end: keep within it.
    if index >= alacritty_terminal::term::color::COUNT {
        return None;
    }
    colors[index].map(|rgb| Rgb::new(rgb.r, rgb.g, rgb.b))
}

fn named_color(named: NamedColor, colors: &Colors, palette: &Palette) -> Rgb {
    if let Some(rgb) = overridden(colors, named as usize) {
        return rgb;
    }
    let foreground =
        || overridden(colors, NamedColor::Foreground as usize).unwrap_or(palette.foreground);
    let background =
        || overridden(colors, NamedColor::Background as usize).unwrap_or(palette.background);
    match named {
        NamedColor::Foreground => palette.foreground,
        NamedColor::Background => palette.background,
        NamedColor::Cursor => palette.cursor,
        NamedColor::BrightForeground => palette.bright_foreground.unwrap_or_else(foreground),
        NamedColor::DimForeground => palette
            .dim_foreground
            .unwrap_or_else(|| foreground().mix(background(), DIM)),
        NamedColor::DimBlack
        | NamedColor::DimRed
        | NamedColor::DimGreen
        | NamedColor::DimYellow
        | NamedColor::DimBlue
        | NamedColor::DimMagenta
        | NamedColor::DimCyan
        | NamedColor::DimWhite => {
            let bright = named.to_bright();
            named_color(bright, colors, palette).mix(background(), DIM)
        }
        other => palette.indexed(u8::try_from(other as usize).unwrap_or(7)),
    }
}

/// How far dim text is mixed into the background.
const DIM: f32 = 1.0 / 3.0;

/// The frame of a screen.
pub(crate) fn of<T: EventListener>(term: &Term<T>, palette: &Palette) -> Frame {
    let content = term.renderable_content();
    let colors = content.colors;
    let columns = term.columns();
    let lines = term.screen_lines();
    let display_offset = content.display_offset;
    let background = named_color(NamedColor::Background, colors, palette);
    let foreground = named_color(NamedColor::Foreground, colors, palette);
    let selection = content.selection;
    let mut rows: Vec<Row> = (0..lines)
        .map(|_| Row {
            cells: Vec::with_capacity(columns),
        })
        .collect();
    let offset = i32::try_from(display_offset).unwrap_or(i32::MAX);
    let mut remembered = Remembered::default();
    for indexed in content.display_iter {
        let row = indexed.point.line.0.saturating_add(offset);
        let Some(target) = usize::try_from(row).ok().and_then(|row| rows.get_mut(row)) else {
            continue;
        };
        let selected = selection.is_some_and(|range| range.contains(indexed.point))
            || (indexed.cell.flags.contains(Flags::WIDE_CHAR)
                && selection.is_some_and(|range| {
                    range.contains(alacritty_terminal::index::Point::new(
                        indexed.point.line,
                        indexed.point.column + 1,
                    ))
                }));
        target.cells.push(cell(
            indexed.cell,
            colors,
            palette,
            background,
            selected,
            &mut remembered,
        ));
    }
    let cursor = cursor(term, display_offset, colors, palette, &rows);
    Frame {
        rows,
        columns: u16::try_from(columns).unwrap_or(u16::MAX),
        cursor,
        display_offset,
        history: term.history_size(),
        mode: Modes::of(*term.mode()),
        background,
        foreground,
    }
}

fn cell(
    source: &GridCell,
    colors: &Colors,
    palette: &Palette,
    background: Rgb,
    selected: bool,
    remembered: &mut Remembered,
) -> Cell {
    let flags = source.flags;
    let mut fg_color = source.fg;
    // Bold text in the default colour may have a brighter one of its own.
    if flags.contains(Flags::BOLD) && fg_color == Color::Named(NamedColor::Foreground) {
        fg_color = Color::Named(NamedColor::BrightForeground);
    }
    let mut foreground = resolve(fg_color, colors, palette);
    let mut back = resolve(source.bg, colors, palette);
    if flags.contains(Flags::DIM) {
        foreground = if source.fg == Color::Named(NamedColor::Foreground) {
            named_color(NamedColor::DimForeground, colors, palette)
        } else {
            foreground.mix(background, DIM)
        };
    }
    if flags.contains(Flags::INVERSE) {
        std::mem::swap(&mut foreground, &mut back);
    }
    if flags.contains(Flags::HIDDEN) {
        foreground = back;
    }
    if selected {
        back = palette.selection_background;
        if let Some(selected_text) = palette.selection_foreground {
            foreground = selected_text;
        }
    }
    // Hidden text keeps its background's colour; everything else is moved
    // until it reads on the background it got (`contrast`).
    if !flags.contains(Flags::HIDDEN) {
        let minimum = if flags.contains(Flags::DIM) {
            contrast::MINIMUM_DIM
        } else {
            contrast::MINIMUM
        };
        foreground = remembered.readable(foreground, back, minimum);
    }
    let underline = if flags.contains(Flags::DOUBLE_UNDERLINE) {
        Underline::Double
    } else if flags.contains(Flags::UNDERCURL) {
        Underline::Curly
    } else if flags.intersects(Flags::UNDERLINE | Flags::DOTTED_UNDERLINE | Flags::DASHED_UNDERLINE)
    {
        Underline::Single
    } else {
        Underline::None
    };
    let width = if flags.contains(Flags::WIDE_CHAR) {
        CellWidth::Wide
    } else if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
        CellWidth::Spacer
    } else {
        CellWidth::Normal
    };
    let character = match source.c {
        // A tab is kept in the grid where it was written; it shows blank.
        '\t' => ' ',
        c if c.is_control() => ' ',
        c => c,
    };
    let zero_width = source
        .zerowidth()
        .filter(|chars| !chars.is_empty())
        .map(|chars| chars.iter().filter(|c| !c.is_control()).collect::<String>())
        .filter(|text| !text.is_empty())
        .map(String::into_boxed_str);
    Cell {
        character,
        zero_width,
        foreground,
        background: back,
        style: Style {
            bold: flags.contains(Flags::BOLD),
            italic: flags.contains(Flags::ITALIC),
            underline,
            underline_color: source
                .underline_color()
                .map(|color| resolve(color, colors, palette)),
            strikeout: flags.contains(Flags::STRIKEOUT),
        },
        width,
        selected,
    }
}

fn cursor<T>(
    term: &Term<T>,
    display_offset: usize,
    colors: &Colors,
    palette: &Palette,
    rows: &[Row],
) -> Option<Cursor> {
    let mode = term.mode();
    let vi = mode.contains(TermMode::VI);
    if !vi && !mode.contains(TermMode::SHOW_CURSOR) {
        return None;
    }
    let point = if vi {
        term.vi_mode_cursor.point
    } else {
        term.grid().cursor.point
    };
    let row = point
        .line
        .0
        .saturating_add(i32::try_from(display_offset).unwrap_or(i32::MAX));
    let row = usize::try_from(row).ok().filter(|row| *row < rows.len())?;
    let mut column = point.column.0;
    let cells = &rows.get(row)?.cells;
    if cells
        .get(column)
        .is_some_and(|cell| cell.width == CellWidth::Spacer)
        && column > 0
    {
        column -= 1;
    }
    let under = cells.get(column)?;
    // The default style is a hollow block no program can ask for: that
    // one means "not asked".
    let shape = match term.cursor_style().shape {
        ProgramShape::Hidden => return None,
        ProgramShape::HollowBlock => None,
        ProgramShape::Block => Some(CursorShape::Block),
        ProgramShape::Underline => Some(CursorShape::Underline),
        ProgramShape::Beam => Some(CursorShape::Bar),
    };
    Some(Cursor {
        row: u16::try_from(row).ok()?,
        column: u16::try_from(column).ok()?,
        shape,
        wide: under.width == CellWidth::Wide,
        color: overridden(colors, NamedColor::Cursor as usize).unwrap_or(palette.cursor),
        text_color: palette.cursor_text,
        character: under.character,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(character: char, foreground: Rgb) -> Cell {
        Cell {
            character,
            foreground,
            background: Rgb::default(),
            ..Cell::default()
        }
    }

    #[test]
    fn the_256_colours() {
        let palette = Palette::default();
        assert_eq!(palette.indexed(1), palette.ansi[1]);
        assert_eq!(palette.indexed(16), Rgb::new(0, 0, 0));
        assert_eq!(palette.indexed(21), Rgb::new(0, 0, 255));
        assert_eq!(palette.indexed(196), Rgb::new(255, 0, 0));
        assert_eq!(palette.indexed(110), Rgb::new(135, 175, 215));
        assert_eq!(palette.indexed(231), Rgb::new(255, 255, 255));
        assert_eq!(palette.indexed(232), Rgb::new(8, 8, 8));
        assert_eq!(palette.indexed(255), Rgb::new(238, 238, 238));
    }

    #[test]
    fn mixing_stays_in_range() {
        let black = Rgb::new(0, 0, 0);
        let white = Rgb::new(255, 255, 255);
        assert_eq!(black.mix(white, 0.5), Rgb::new(128, 128, 128));
        assert_eq!(black.mix(white, 7.0), white);
        assert_eq!(black.mix(white, f32::NAN), black);
        assert_eq!(Rgb::hex(0x123456).to_hex(), 0x123456);
    }

    #[test]
    fn runs_group_text_and_backgrounds_merge() {
        let red = Rgb::new(255, 0, 0);
        let white = Rgb::new(255, 255, 255);
        let mut cells: Vec<Cell> = "ab cd".chars().map(|c| plain(c, white)).collect();
        cells.push(plain('e', red));
        cells.push(plain(' ', red));
        cells.push(Cell {
            width: CellWidth::Wide,
            ..plain('中', white)
        });
        cells.push(Cell {
            width: CellWidth::Spacer,
            ..plain(' ', white)
        });
        cells.push(Cell {
            zero_width: Some("\u{301}".into()),
            ..plain('e', white)
        });
        cells[1].background = red;
        cells[2].background = red;
        cells[4].background = white;
        let row = Row { cells };
        let runs = row.runs();
        let texts: Vec<(&str, u16, u16, bool)> = runs
            .iter()
            .map(|run| (run.text.as_str(), run.column, run.columns, run.wide))
            .collect();
        assert_eq!(
            texts,
            [
                ("ab cd", 0, 5, false),
                ("e ", 5, 2, false),
                ("中", 7, 2, true),
                ("e\u{301}", 9, 1, false)
            ]
        );
        let spans = row.backgrounds(Rgb::default());
        assert_eq!(
            spans,
            [
                Span {
                    column: 1,
                    columns: 2,
                    color: red
                },
                Span {
                    column: 4,
                    columns: 1,
                    color: white
                }
            ]
        );
        assert_eq!(row.text(), "ab cde 中e\u{301}");
    }

    #[test]
    fn blank_runs_draw_nothing_unless_underlined() {
        let white = Rgb::new(255, 255, 255);
        let mut cells: Vec<Cell> = "   ".chars().map(|c| plain(c, white)).collect();
        assert!(
            Row {
                cells: cells.clone()
            }
            .runs()
            .is_empty()
        );
        cells[1].style.underline = Underline::Single;
        let runs = Row { cells }.runs();
        assert_eq!(runs.len(), 1);
        assert_eq!((runs[0].column, runs[0].columns), (1, 1));
    }
}
