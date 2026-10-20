//! What the toolbar keeps when the window is narrow for it: at 150 % a
//! window 1280 wide is 853 points, too few for the middle group and the
//! two blocks at the left side by side.
//!
//! The room the middle group keeps free after it gives way first; then the
//! words under the buttons go, as they go when the setting switches them
//! off; then the two blocks grow narrower, their names cut short, down to a
//! least; then the middle group gives way to a menu: Branch, Stash, Pop,
//! the terminal and the large-file control go into a button with a menu
//! after Push, and in a window narrower still the whole group is that one
//! button. The left part is never drawn over the middle, nor the middle
//! over the right.
//!
//! Sources: the CSS flexible box layout's resolution of flexible lengths,
//! which the toolkit's layout follows.

/// Width of one button of the middle group, with its words and without.
pub(super) const BUTTON_WIDTH: f32 = 48.0;
pub(super) const BUTTON_NARROW: f32 = 36.0;
/// The caret beside Pull.
const CARET_WIDTH: f32 = 14.0;
/// The terminal's button, with its words: wider than the others.
pub(super) const TERMINAL_WIDTH: f32 = 52.0;
/// Between two parts of the middle group, and of the right one.
const GAP: f32 = 4.0;
/// The large-file control, when there is one, and its margin.
const LFS_SPAN: f32 = 6.0 + 46.0;
/// The two blocks at the left and the chevron between them, at their
/// width and at their least.
const BLOCK_WIDTH: f32 = 110.0;
const BLOCK_LEAST: f32 = 72.0;
pub(super) const CHEVRON_WIDTH: f32 = 13.0;
/// What the bar keeps at its right end.
const END: f32 = 2.0;

/// How much of the middle group stands as buttons of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Middle {
    /// Every button.
    All,
    /// Undo, Redo, Pull and Push, then a button whose menu has the rest.
    Some,
    /// One button whose menu has every one.
    Menu,
}

impl Middle {
    /// Whether the button of the middle group known by `id` stands in the
    /// bar (and is not in the menu).
    pub(super) fn shows(self, id: &str) -> bool {
        match self {
            Middle::All => true,
            Middle::Some => matches!(id, "undo" | "redo" | "pull" | "push"),
            Middle::Menu => false,
        }
    }
}

/// How the toolbar is laid out in a window `width` points wide.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ToolbarFit {
    /// Whether the buttons say what they are under their icons.
    pub labels: bool,
    /// How much of the middle group stands as buttons.
    pub middle: Middle,
    /// How wide each of the two blocks at the left is.
    pub block: f32,
    /// How wide the left part is at least: the two blocks.
    pub left: f32,
    /// How wide the right part is at least: its two buttons.
    pub right: f32,
}

/// How wide the middle group is: seven buttons (Pull with its caret), a
/// rule that takes no room, the terminal's button, and the large-file
/// control when there is one; or Undo, Redo, Pull with its caret, Push and
/// the menu's button; or the menu's button alone.
fn middle(labels: bool, lfs: bool, kept: Middle) -> f32 {
    let button = if labels { BUTTON_WIDTH } else { BUTTON_NARROW };
    match kept {
        Middle::All => {
            let terminal = if labels {
                TERMINAL_WIDTH
            } else {
                BUTTON_NARROW
            };
            let parts = if lfs { 10.0 } else { 9.0 };
            7.0 * button
                + CARET_WIDTH
                + terminal
                + (parts - 1.0) * GAP
                + if lfs { LFS_SPAN } else { 0.0 }
        }
        Middle::Some => 5.0 * button + CARET_WIDTH + 4.0 * GAP,
        Middle::Menu => button,
    }
}

/// How wide the right part is: two buttons and the gap between.
fn right(labels: bool) -> f32 {
    let button = if labels { BUTTON_WIDTH } else { BUTTON_NARROW };
    2.0 * button + GAP
}

/// The two blocks and the chevron, each block `block` wide.
fn crumbs(block: f32) -> f32 {
    2.0 * block + CHEVRON_WIDTH
}

/// The layout of the toolbar in a window `width` points wide; `labels` is
/// what the setting asks for, `lfs` whether the large-file control is
/// there. A width that is no number is taken as wide enough.
pub(super) fn fit(width: f32, labels: bool, lfs: bool) -> ToolbarFit {
    let width = if width.is_finite() { width } else { f32::MAX };
    let full = crumbs(BLOCK_WIDTH) + middle(true, lfs, Middle::All) + right(true) + END;
    let labels = labels && width >= full;
    // The most of the middle group that leaves the blocks their least.
    let fits =
        |kept| crumbs(BLOCK_LEAST) + middle(labels, lfs, kept) + right(labels) + END <= width;
    let kept = [Middle::All, Middle::Some]
        .into_iter()
        .find(|kept| fits(*kept))
        .unwrap_or(Middle::Menu);
    let room = width - middle(labels, lfs, kept) - right(labels) - END;
    let block = ((room - CHEVRON_WIDTH) / 2.0).clamp(BLOCK_LEAST, BLOCK_WIDTH);
    ToolbarFit {
        labels,
        middle: kept,
        block,
        left: crumbs(block),
        right: right(labels),
    }
}

#[cfg(test)]
mod tests {
    use super::{BLOCK_WIDTH, Middle, fit};

    #[test]
    fn the_toolbar_keeps_its_words_and_blocks_where_there_is_room() {
        // 1280 at 100 %, and 1280 at 150 %: everything, the room after the
        // middle group gives way at 853.
        for width in [1280.0, 853.3] {
            let fit = fit(width, true, false);
            assert!(fit.labels, "{width}");
            assert_eq!(fit.middle, Middle::All);
            assert_eq!(fit.block, BLOCK_WIDTH, "{width}");
            assert_eq!(fit.left, 233.0);
            assert_eq!(fit.right, 100.0);
        }
        // Words switched off stay off.
        assert!(!fit(1280.0, false, false).labels);
    }

    #[test]
    fn a_narrower_window_drops_the_words_then_narrows_the_blocks() {
        // 1280 at 175 %: no room for the words.
        let at_175 = fit(731.4, true, false);
        assert!(!at_175.labels);
        assert_eq!(at_175.block, BLOCK_WIDTH);
        // At 200 %: the blocks give way too, to what is left.
        let at_200 = fit(640.0, true, false);
        assert!(!at_200.labels);
        assert_eq!(at_200.middle, Middle::All);
        assert!(
            at_200.block < BLOCK_WIDTH && at_200.block > 72.0,
            "{at_200:?}"
        );
        assert!(at_200.left + 334.0 + at_200.right + 2.0 <= 640.0 + 0.01);
        // The large-file control takes room of its own.
        assert!(fit(800.0, true, false).labels);
        assert!(!fit(800.0, true, true).labels);
        // A width that is no number is wide enough.
        assert!(fit(f32::NAN, true, false).labels);
    }

    #[test]
    fn a_narrower_window_still_puts_the_middle_group_in_a_menu() {
        // The blocks at their least and every button: 569 points.
        assert_eq!(fit(569.0, true, false).middle, Middle::All);
        assert_eq!(fit(568.0, true, false).middle, Middle::Some);
        // With the large-file control the whole group is wider.
        assert_eq!(fit(600.0, true, true).middle, Middle::Some);
        // Undo, Redo, Pull and Push with the menu: 445 points.
        assert_eq!(fit(445.0, true, false).middle, Middle::Some);
        assert_eq!(fit(444.0, true, false).middle, Middle::Menu);
        // Whatever is kept, the parts fit side by side down to the least.
        for width in [640.0, 568.0, 500.0, 445.0, 444.0, 271.0] {
            for lfs in [false, true] {
                let fit = fit(width, true, lfs);
                let middle = super::middle(false, lfs, fit.middle);
                assert!(
                    fit.left + middle + fit.right + 2.0 <= width + 0.01,
                    "{width} {lfs}: {fit:?}"
                );
            }
        }
        // With the menu alone the blocks have room again; however narrow,
        // they keep their least.
        assert_eq!(fit(300.0, true, true).middle, Middle::Menu);
        assert_eq!(fit(300.0, true, true).block, 86.5);
        assert_eq!(fit(200.0, true, true).block, 72.0);
        // What stands in the bar, and what is in the menu.
        assert!(Middle::All.shows("stash"));
        assert!(Middle::Some.shows("push") && !Middle::Some.shows("stash"));
        assert!(!Middle::Menu.shows("undo"));
    }
}
