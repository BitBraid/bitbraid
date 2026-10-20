//! How the terminals look, from the preferences (the section Terminal)
//! and the theme in use (its terminal colours): what every terminal of the
//! window is drawn with, handed to the text of each session again when it
//! changes.
//!
//! Sources: none (original design).

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::{App, SharedString};

use crate::prefs::Prefs;
use crate::theme::TerminalColors;

/// The shape of the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cursor {
    Bar,
    Block,
    Underline,
}

impl Cursor {
    /// The cursor a word of the preferences stands for; a bar for any
    /// other.
    pub fn from_word(word: &str) -> Self {
        match word {
            "block" => Self::Block,
            "underline" => Self::Underline,
            _ => Self::Bar,
        }
    }
}

/// What a terminal is drawn with.
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// The family of the text.
    pub font_family: SharedString,
    /// Its size, in points (the zoom is the drawing's to apply).
    pub font_size: f32,
    /// How high a line is, as a multiple of the size.
    pub line_height: f32,
    pub cursor: Cursor,
    /// Whether the text is dimmed while the keys are elsewhere.
    pub dim_unfocused: bool,
    pub colors: TerminalColors,
}

/// The look of the terminals as the preferences and the theme say now.
pub fn current(cx: &App) -> Look {
    let prefs = &cx.global::<Prefs>().preferences.terminal_panel;
    Look {
        font_family: prefs
            .font_family
            .clone()
            .map(SharedString::from)
            .unwrap_or_else(|| cx.theme().mono_font_family.clone()),
        font_size: prefs.font_size,
        line_height: prefs.line_height,
        cursor: Cursor::from_word(&prefs.cursor),
        dim_unfocused: prefs.dim_unfocused,
        colors: crate::theme::terminal_colors(cx),
    }
}

/// The look as the text of a session takes it.
pub fn terminal_look(look: &Look) -> super::text::TerminalLook {
    use super::text::CursorStyle;
    super::text::TerminalLook {
        font_family: look.font_family.clone(),
        font_size: look.font_size,
        line_height: look.line_height,
        cursor: match look.cursor {
            Cursor::Bar => CursorStyle::Bar,
            Cursor::Block => CursorStyle::Block,
            Cursor::Underline => CursorStyle::Underline,
        },
        dim_unfocused: look.dim_unfocused,
        palette: palette(&look.colors),
    }
}

/// A colour of ours as the terminal's frame takes it.
fn rgb(color: gpui_kit::Hsla) -> bitbraid_terminal::Rgb {
    let rgba = color.to_rgb();
    // Each channel is between 0 and 1; one that is no number is 0.
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
    bitbraid_terminal::Rgb {
        r: channel(rgba.r),
        g: channel(rgba.g),
        b: channel(rgba.b),
    }
}

/// The terminal colours of a theme as the session's frame resolves them.
pub fn palette(colors: &TerminalColors) -> bitbraid_terminal::Palette {
    bitbraid_terminal::Palette {
        ansi: colors.ansi.map(rgb),
        foreground: rgb(colors.foreground),
        background: rgb(colors.background),
        cursor: rgb(colors.cursor),
        // What stands under a block cursor: the ground, so that it reads.
        cursor_text: rgb(colors.background),
        selection_foreground: Some(rgb(colors.selection_foreground)),
        selection_background: rgb(colors.selection_background),
        dim_foreground: Some(rgb(colors.dim_foreground)),
        bright_foreground: None,
    }
}

#[cfg(test)]
mod tests {
    use super::{Cursor, palette};

    #[test]
    fn the_theme_colours_reach_the_frame() {
        let colors = crate::theme::terminal_builtin(crate::theme::TerminalBuiltin::Dark);
        let palette = palette(&colors);
        // The ground of the dark theme, #0f141a.
        assert_eq!(
            (
                palette.background.r,
                palette.background.g,
                palette.background.b
            ),
            (0x0f, 0x14, 0x1a)
        );
        assert_eq!(palette.ansi.len(), 16);
        assert!(palette.selection_foreground.is_some() && palette.dim_foreground.is_some());
    }

    #[test]
    fn a_cursor_is_one_of_three_and_a_bar_otherwise() {
        assert_eq!(Cursor::from_word("block"), Cursor::Block);
        assert_eq!(Cursor::from_word("underline"), Cursor::Underline);
        assert_eq!(Cursor::from_word("bar"), Cursor::Bar);
        assert_eq!(Cursor::from_word(""), Cursor::Bar);
    }
}
