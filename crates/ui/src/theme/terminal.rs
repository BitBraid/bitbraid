//! The colours of the terminal inside the window, for each of our themes,
//! and the names a theme file of the user's gives them.
//!
//! The palettes are ours: each is made from its theme's own colours (the
//! ground a shade off the window's, the eight colours from the theme's
//! base colours, their bright kin lighter on dark and darker on light, so
//! that every one of them reads on the ground). A theme file names what
//! it changes in its `[colors]` table, `"terminal.background"` or
//! `[colors.terminal] background`, and the rest is ours of its mode.
//!
//! Sources: XTerm Control Sequences (the sixteen colours and their order).

use gpui_kit::{Global, Hsla};

/// The sixteen colours a program asks for by number, in that order: the
/// eight, then their bright kin.
pub const ANSI_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright_black",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
    "bright_white",
];

/// The colours of a terminal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalColors {
    /// The sixteen colours by number.
    pub ansi: [Hsla; 16],
    /// The text where a program asks for no colour.
    pub foreground: Hsla,
    /// The ground.
    pub background: Hsla,
    /// The cursor.
    pub cursor: Hsla,
    /// What is selected: its ground and its text.
    pub selection_background: Hsla,
    pub selection_foreground: Hsla,
    /// Text a program asks to be faint.
    pub dim_foreground: Hsla,
}

/// The terminal colours of the theme in use, a GPUI global.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalTheme(pub TerminalColors);

impl Global for TerminalTheme {}

/// A colour from its `#rrggbb`; black for what is none (only constants
/// of this file are read).
fn hex(text: &str) -> Hsla {
    gpui_kit::component::try_parse_color(text).unwrap_or_default()
}

/// One of our palettes, written out.
struct Written {
    ansi: [&'static str; 16],
    foreground: &'static str,
    background: &'static str,
    cursor: &'static str,
    selection_background: &'static str,
    selection_foreground: &'static str,
    dim_foreground: &'static str,
}

impl Written {
    fn colors(&self) -> TerminalColors {
        TerminalColors {
            ansi: self.ansi.map(hex),
            foreground: hex(self.foreground),
            background: hex(self.background),
            cursor: hex(self.cursor),
            selection_background: hex(self.selection_background),
            selection_foreground: hex(self.selection_foreground),
            dim_foreground: hex(self.dim_foreground),
        }
    }
}

/// Dark: the ground of the bars around the window, the base colours of
/// the theme, and the accent for the cursor.
const DARK: Written = Written {
    ansi: [
        "#1b222c", "#e05858", "#4fb373", "#e0b34a", "#4a90d9", "#b975d1", "#45b2c2", "#c9d1db",
        "#5d6b7a", "#f07a7a", "#6fcf8f", "#f0c96a", "#6fa9e8", "#cf95e3", "#6acbd8", "#f4f7fa",
    ],
    foreground: "#d5dde6",
    background: "#0f141a",
    cursor: "#3b8bd8",
    selection_background: "#2b4a6e",
    selection_foreground: "#f4f7fa",
    dim_foreground: "#8a97a8",
};

/// Light: a white ground under the window's grey, the base colours made
/// dark enough to read on it. Every colour is also a ground programs
/// write on: the colours read under black and under bright white, white
/// under black, black under white and bright white; bright white is the
/// light ground of the lot (as text on the white ground it is not meant
/// to be seen, as in any light terminal).
const LIGHT: Written = Written {
    ansi: [
        "#14181d", "#c0302d", "#1f7a3f", "#8a6100", "#2766bd", "#9c3fb3", "#11737f", "#6b747f",
        "#5d6b7a", "#a42523", "#186434", "#6e4d00", "#1d55a3", "#7f2f94", "#0d5f69", "#eef0f3",
    ],
    foreground: "#1d2733",
    background: "#ffffff",
    cursor: "#2f7bc8",
    selection_background: "#cfe0f5",
    selection_foreground: "#1d2733",
    dim_foreground: "#5d6b7a",
};

/// High contrast: black and white, full colours, a selection that turns
/// the text round.
const CONTRAST: Written = Written {
    ansi: [
        "#000000", "#ff6b6b", "#6cf07a", "#ffd400", "#6ea8ff", "#e08cff", "#5ce1e6", "#e6e6e6",
        "#9a9a9a", "#ff9a9a", "#a0ffaa", "#fff07a", "#a6c8ff", "#f0b8ff", "#9af0f2", "#ffffff",
    ],
    foreground: "#ffffff",
    background: "#000000",
    cursor: "#ffd400",
    selection_background: "#ffffff",
    selection_foreground: "#000000",
    dim_foreground: "#d0d0d0",
};

/// Dusk: the plum of its bars, its base colours, its violet cursor and a
/// violet selection.
const DUSK: Written = Written {
    ansi: [
        "#2a2636", "#ea6878", "#67cc9a", "#ecca6c", "#7ca5e9", "#dc82d4", "#6ecbd6", "#d1cfdd",
        "#6d6881", "#fd8994", "#87e1b2", "#f6dd90", "#9cbff8", "#eda4e6", "#90e1ea", "#f5f4fb",
    ],
    foreground: "#e8e6f0",
    background: "#141120",
    cursor: "#9575e2",
    selection_background: "#43356e",
    selection_foreground: "#f6f2fd",
    dim_foreground: "#96949d",
};

/// Ember: warm charcoal, its base colours, the amber of its accent.
const EMBER: Written = Written {
    ansi: [
        "#2b2521", "#e36650", "#83ba64", "#eac25a", "#6ba9d4", "#d981ac", "#6ebfb9", "#d8cfc7",
        "#71675f", "#f68771", "#9fd283", "#f7d883", "#8bc2e9", "#eca0c4", "#91d6d1", "#f8f5ef",
    ],
    foreground: "#eae7e4",
    background: "#18120e",
    cursor: "#eb9e41",
    selection_background: "#4d3a24",
    selection_foreground: "#f7f1ea",
    dim_foreground: "#989593",
};

/// Graphite: neutral greys, its base colours, the indigo of its accent.
const GRAPHITE: Written = Written {
    ansi: [
        "#2b2b2d", "#eb635e", "#65c67d", "#f1c45e", "#75a2ec", "#ca80db", "#5cc6d7", "#d3d4d7",
        "#707174", "#ff847d", "#86df9a", "#fcda86", "#95bcfb", "#dda1ea", "#8dddeb", "#f8f8f8",
    ],
    foreground: "#e7e7e9",
    background: "#17181a",
    cursor: "#687de0",
    selection_background: "#36406e",
    selection_foreground: "#f5f6fa",
    dim_foreground: "#959697",
};

/// Paper: a cream ground, the colours made as the light palette's are
/// (dark enough to read on it, light enough to carry black), warmer.
const PAPER: Written = Written {
    ansi: [
        "#1a1511", "#bb3924", "#3f7932", "#906405", "#2c6fae", "#9b4796", "#1d777b", "#7b7167",
        "#74685b", "#982917", "#306324", "#734f02", "#105996", "#80337b", "#156165", "#f4efe7",
    ],
    foreground: "#282623",
    background: "#fffdf8",
    cursor: "#b34f2a",
    selection_background: "#f3dccb",
    selection_foreground: "#2f2820",
    dim_foreground: "#6b6966",
};

/// Sage: a white ground, the colours made as the light palette's are,
/// with green-grey for black and white.
const SAGE: Written = Written {
    ansi: [
        "#131714", "#b83c3b", "#287c42", "#8a6700", "#1e71ac", "#904e9f", "#1e767f", "#6d766e",
        "#636e64", "#992728", "#196632", "#6e5203", "#015b91", "#753a83", "#156068", "#ecf2ed",
    ],
    foreground: "#252725",
    background: "#ffffff",
    cursor: "#187c49",
    selection_background: "#d3e8da",
    selection_foreground: "#1f2a24",
    dim_foreground: "#686a68",
};

/// Which of our palettes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Builtin {
    Dark,
    Light,
    Contrast,
    Dusk,
    Ember,
    Graphite,
    Paper,
    Sage,
}

impl Builtin {
    /// Every one of them.
    #[cfg(test)]
    pub const ALL: [Builtin; 8] = [
        Self::Dark,
        Self::Light,
        Self::Contrast,
        Self::Dusk,
        Self::Ember,
        Self::Graphite,
        Self::Paper,
        Self::Sage,
    ];

    /// Whether its ground is light.
    #[cfg(test)]
    pub fn is_light(self) -> bool {
        matches!(self, Self::Light | Self::Paper | Self::Sage)
    }
}

/// One of our palettes.
pub fn builtin(which: Builtin) -> TerminalColors {
    match which {
        Builtin::Dark => DARK.colors(),
        Builtin::Light => LIGHT.colors(),
        Builtin::Contrast => CONTRAST.colors(),
        Builtin::Dusk => DUSK.colors(),
        Builtin::Ember => EMBER.colors(),
        Builtin::Graphite => GRAPHITE.colors(),
        Builtin::Paper => PAPER.colors(),
        Builtin::Sage => SAGE.colors(),
    }
}

/// Every name a theme file can give a terminal colour, without the
/// `terminal.` in front, with the colour of `colors` it stands for.
pub fn named(colors: &TerminalColors) -> Vec<(String, Hsla)> {
    let mut out = vec![
        ("foreground".to_owned(), colors.foreground),
        ("background".to_owned(), colors.background),
        ("cursor".to_owned(), colors.cursor),
        (
            "selection.background".to_owned(),
            colors.selection_background,
        ),
        (
            "selection.foreground".to_owned(),
            colors.selection_foreground,
        ),
        ("dim.foreground".to_owned(), colors.dim_foreground),
    ];
    out.extend(
        ANSI_NAMES
            .iter()
            .zip(colors.ansi)
            .map(|(name, color)| (format!("ansi.{name}"), color)),
    );
    out
}

/// Put a colour a theme file names (without `terminal.` in front) in its
/// place. False when there is no terminal colour of that name.
pub fn set(colors: &mut TerminalColors, name: &str, color: Hsla) -> bool {
    let slot = match name {
        "foreground" => &mut colors.foreground,
        "background" => &mut colors.background,
        "cursor" => &mut colors.cursor,
        "selection.background" => &mut colors.selection_background,
        "selection.foreground" => &mut colors.selection_foreground,
        "dim.foreground" => &mut colors.dim_foreground,
        other => {
            let Some(at) = other
                .strip_prefix("ansi.")
                .and_then(|ansi| ANSI_NAMES.iter().position(|known| *known == ansi))
            else {
                return false;
            };
            match colors.ansi.get_mut(at) {
                Some(slot) => slot,
                None => return false,
            }
        }
    };
    *slot = color;
    true
}

/// The contrast of two colours as WCAG counts it, from 1 to 21.
#[cfg(test)]
pub(super) fn contrast(a: Hsla, b: Hsla) -> f32 {
    fn luminance(color: Hsla) -> f32 {
        let rgb = color.to_rgb();
        let channel = |c: f32| {
            if c <= 0.039_28 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(rgb.r) + 0.7152 * channel(rgb.g) + 0.0722 * channel(rgb.b)
    }
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::{Builtin, builtin, contrast, named, set};

    #[test]
    fn every_colour_reads_on_its_ground() {
        for which in Builtin::ALL {
            let colors = builtin(which);
            assert!(
                contrast(colors.foreground, colors.background) >= 7.0,
                "{which:?}: the text"
            );
            assert!(
                contrast(colors.selection_foreground, colors.selection_background) >= 4.5,
                "{which:?}: the selection"
            );
            assert!(
                contrast(colors.dim_foreground, colors.background) >= 3.0,
                "{which:?}: faint text"
            );
            assert!(
                contrast(colors.cursor, colors.background) >= 3.0,
                "{which:?}: the cursor"
            );
            // Every colour but the one that is the ground's own (black on
            // dark, the bright white on light) reads as text.
            for (at, color) in colors.ansi.iter().enumerate() {
                let ground_like = if which.is_light() { at == 15 } else { at == 0 };
                if !ground_like {
                    assert!(
                        contrast(*color, colors.background) >= 3.0,
                        "{which:?}: colour {at} on the ground ({:.2})",
                        contrast(*color, colors.background)
                    );
                }
            }
        }
    }

    /// What programs write on a light ground: every colour as text, and
    /// the pairs they use as text on a ground of a colour (a status line
    /// in bright white on blue, `ls` in black on green, a selection in
    /// black on white).
    #[test]
    fn the_light_colours_read_as_text_and_as_grounds() {
        for which in Builtin::ALL.into_iter().filter(|which| which.is_light()) {
            light_colours_read(which);
        }
    }

    fn light_colours_read(which: Builtin) {
        let colors = builtin(which);
        let at = |n: usize| colors.ansi[n];
        // As text on the ground: the colours, white and black.
        for n in (1..=14).filter(|n| *n != 8) {
            assert!(
                contrast(at(n), colors.background) >= 4.5,
                "{which:?}: colour {n} on the ground ({:.2})",
                contrast(at(n), colors.background)
            );
        }
        assert!(
            contrast(at(0), colors.background) >= 7.0,
            "{which:?}: black"
        );
        // As grounds: black and bright white under each other and the
        // colours, white under black, black under white.
        assert!(
            contrast(at(15), at(0)) >= 7.0,
            "{which:?}: bright white on black"
        );
        assert!(contrast(at(7), at(0)) >= 3.5, "{which:?}: white on black");
        assert!(contrast(at(0), at(7)) >= 3.5, "{which:?}: black on white");
        for n in (1..=6).chain(9..=14) {
            assert!(
                contrast(at(15), at(n)) >= 4.5,
                "{which:?}: bright white on colour {n} ({:.2})",
                contrast(at(15), at(n))
            );
        }
        // Black is written on the six (`ls` writes on green and yellow);
        // on the bright ones, which are the darker here, bright white is.
        for n in 1..=6 {
            assert!(
                contrast(at(0), at(n)) >= 3.0,
                "{which:?}: black on colour {n} ({:.2})",
                contrast(at(0), at(n))
            );
        }
        // Text the shell gives no colour reads on bright white.
        assert!(
            contrast(colors.foreground, at(15)) >= 7.0,
            "{which:?}: text on bright white"
        );
    }

    #[test]
    fn every_name_is_a_colour_that_can_be_set() {
        let mut colors = builtin(Builtin::Dark);
        let names = named(&colors);
        assert_eq!(names.len(), 22);
        let white = gpui_kit::hsla(0., 0., 1., 1.);
        for (name, _) in &names {
            assert!(set(&mut colors, name, white), "{name}");
        }
        assert!(named(&colors).iter().all(|(_, color)| *color == white));
        for nothing in ["", "ansi.", "ansi.orange", "selection", "ansi.bright_", "x"] {
            assert!(!set(&mut colors, nothing, white), "{nothing}");
        }
    }
}
