//! Text that stays readable on whatever background a program asks for.
//!
//! A program picks colours for a terminal it has never seen: black
//! behind text in the default colour reads on a dark theme and not on a
//! light one, and the other way round. Before a cell is drawn its text
//! colour is moved towards black or white, as little as needed, until it
//! stands out from its background by a contrast ratio of at least 3 (2
//! for dim text, which is meant to stand out less). That is a floor for
//! what cannot be read, not a restyling: the colours of our themes stand
//! above it on their own backgrounds. Its background is never changed,
//! nor is text the program hid on purpose.
//!
//! Sources: WCAG 2.1, "relative luminance" and "contrast ratio"
//! (w3.org/TR/WCAG21/#dfn-relative-luminance, #dfn-contrast-ratio).

use std::sync::OnceLock;

use crate::frame::Rgb;

/// The ratio text keeps against its background.
pub const MINIMUM: f32 = 3.0;
/// The ratio dim text keeps against its background.
pub const MINIMUM_DIM: f32 = 2.0;

/// How bright a colour is to the eye, from 0 (black) to 1 (white).
pub fn luminance(colour: Rgb) -> f32 {
    let table = linear_table();
    let part = |value: u8| table.get(usize::from(value)).copied().unwrap_or(0.0);
    0.2126 * part(colour.r) + 0.7152 * part(colour.g) + 0.0722 * part(colour.b)
}

/// How far apart two colours stand, from 1 (the same) to 21 (black and
/// white).
pub fn ratio(one: Rgb, other: Rgb) -> f32 {
    let (a, b) = (luminance(one), luminance(other));
    let (light, dark) = if a >= b { (a, b) } else { (b, a) };
    (light + 0.05) / (dark + 0.05)
}

/// `text`, moved towards black or white until it stands `minimum` apart
/// from `background`, or as far as it can go.
pub fn readable(text: Rgb, background: Rgb, minimum: f32) -> Rgb {
    if ratio(text, background) >= minimum {
        return text;
    }
    let black = Rgb::new(0, 0, 0);
    let white = Rgb::new(255, 255, 255);
    let towards = if ratio(white, background) >= ratio(black, background) {
        white
    } else {
        black
    };
    if ratio(towards, background) < minimum {
        return towards;
    }
    // The least mix that is enough: the ratio grows with the mix.
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..10 {
        let middle = (low + high) / 2.0;
        if ratio(text.mix(towards, middle), background) >= minimum {
            high = middle;
        } else {
            low = middle;
        }
    }
    text.mix(towards, high)
}

/// The share of light each of the 256 values of a channel gives.
fn linear_table() -> &'static [f32; 256] {
    static TABLE: OnceLock<[f32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0.0f32; 256];
        for (value, slot) in (0u8..=255).zip(table.iter_mut()) {
            let channel = f32::from(value) / 255.0;
            *slot = if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            };
        }
        table
    })
}

/// The last pair of colours worked out, so that a run of cells of one
/// colour costs one look-up.
#[derive(Debug, Default)]
pub(crate) struct Remembered {
    last: Option<(Rgb, Rgb, u32, Rgb)>,
}

impl Remembered {
    pub(crate) fn readable(&mut self, text: Rgb, background: Rgb, minimum: f32) -> Rgb {
        let key = minimum.to_bits();
        if let Some((t, b, m, answer)) = self.last
            && t == text
            && b == background
            && m == key
        {
            return answer;
        }
        let answer = readable(text, background, minimum);
        self.last = Some((text, background, key, answer));
        answer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_and_white_are_twenty_one_apart() {
        let ratio = ratio(Rgb::new(0, 0, 0), Rgb::new(255, 255, 255));
        assert!((ratio - 21.0).abs() < 0.01, "{ratio}");
    }

    #[test]
    fn readable_text_is_left_alone() {
        let text = Rgb::hex(0x202020);
        assert_eq!(readable(text, Rgb::hex(0xffffff), MINIMUM), text);
    }

    #[test]
    fn dark_text_on_black_is_lifted_until_it_reads() {
        let background = Rgb::new(0, 0, 0);
        let lifted = readable(Rgb::hex(0x2a2a33), background, MINIMUM);
        assert!(ratio(lifted, background) >= MINIMUM);
        // As little as needed: not all the way to white.
        assert_ne!(lifted, Rgb::new(255, 255, 255));
    }

    #[test]
    fn light_text_on_white_is_darkened() {
        let background = Rgb::new(255, 255, 255);
        let darkened = readable(Rgb::hex(0xeeeeee), background, MINIMUM);
        assert!(ratio(darkened, background) >= MINIMUM);
        assert!(luminance(darkened) < luminance(Rgb::hex(0xeeeeee)));
    }

    #[test]
    fn text_in_its_own_background_colour_becomes_visible() {
        for value in [0x000000, 0x808080, 0x3465a4, 0xffffff] {
            let colour = Rgb::hex(value);
            let shown = readable(colour, colour, MINIMUM);
            let best = ratio(Rgb::hex(0xffffff), colour).max(ratio(Rgb::hex(0), colour));
            assert!(
                ratio(shown, colour) >= MINIMUM.min(best) - 0.01,
                "{value:06x}"
            );
        }
    }

    #[test]
    fn the_remembered_pair_answers_as_the_function_does() {
        let mut remembered = Remembered::default();
        let pairs = [
            (Rgb::hex(0x111111), Rgb::hex(0x000000)),
            (Rgb::hex(0x111111), Rgb::hex(0x000000)),
            (Rgb::hex(0xfafafa), Rgb::hex(0xffffff)),
            (Rgb::hex(0x111111), Rgb::hex(0x000000)),
        ];
        for (text, background) in pairs {
            assert_eq!(
                remembered.readable(text, background, MINIMUM),
                readable(text, background, MINIMUM)
            );
        }
    }
}
