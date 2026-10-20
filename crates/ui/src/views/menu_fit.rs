//! How tall a long menu may be: what the window leaves of its height once
//! the toolkit has kept its distance from the window's edges. A menu that
//! is longer scrolls inside that height, rather than running out of the
//! window where what is at its end cannot be reached.
//!
//! The toolkit keeps a menu opened by the pointer's other button 8 pixels
//! from each edge, and with them the window's client inset, the room a
//! window that draws its own frame keeps for its shadow. That inset is
//! kept on every side even where the window is tiled against the screen
//! and draws no shadow there (a maximised window, a tiling compositor):
//! on Linux it is 20, so a menu that left the window only the 8 would run
//! 24 pixels out of a window 720 high.
//!
//! Sources: gpui's `anchored` element (`snap_to_window_with_margin`, which
//! adds `Window::client_inset` to the margin); gpui-component's
//! `ContextMenu` (its margin of 8) and `PopupMenu::max_h`/`scrollable`.

use gpui_kit::Window;
use gpui_kit::component::menu::PopupMenu;

/// What the toolkit keeps between a menu and each edge of the window, in
/// its pixels, before the client inset.
const EDGE: f32 = 8.0;
/// The menu's own frame around what scrolls: its border, top and bottom,
/// and a pixel to spare for rounding.
const FRAME: f32 = 4.0;
/// The least a menu is given however small the window, in the toolkit's
/// pixels: a few entries.
const LEAST: f32 = 120.0;

/// How high a menu may be in a window `height` high whose client inset is
/// `inset`, both in the toolkit's pixels. A height or an inset that is no
/// number counts as nothing.
pub fn room(height: f32, inset: f32) -> f32 {
    let height = if height.is_finite() { height } else { 0.0 };
    let inset = if inset.is_finite() {
        inset.max(0.0)
    } else {
        0.0
    };
    (height - 2.0 * (EDGE + inset) - FRAME).max(LEAST)
}

/// A menu that scrolls inside what the window leaves it: for every menu
/// that can be longer than a small window is high. Applied to the menu a
/// builder is handed, before its entries, it holds for all of them.
pub fn fit(menu: PopupMenu, window: &Window) -> PopupMenu {
    let height = f32::from(window.viewport_size().height);
    let inset = window.client_inset().map_or(0.0, f32::from);
    menu.scrollable(true)
        .max_h(gpui_kit::px(room(height, inset)))
}

#[cfg(test)]
mod tests {
    use super::room;

    #[test]
    fn a_menu_keeps_clear_of_both_edges_and_the_inset_on_each() {
        // No frame of the window's own: 8 above, 8 below, the frame.
        assert_eq!(room(720.0, 0.0), 700.0);
        // Linux's client inset of 20 is kept on each side as well.
        assert_eq!(room(720.0, 20.0), 660.0);
        // However small the window, a few entries.
        assert_eq!(room(100.0, 20.0), 120.0);
        // Hostile numbers are none.
        assert_eq!(room(f32::NAN, 20.0), 120.0);
        assert_eq!(room(720.0, f32::INFINITY), 700.0);
        assert_eq!(room(720.0, -50.0), 700.0);
    }
}
