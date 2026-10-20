//! The window's frame on macOS: its three buttons stand at the left of the
//! bar of tabs, as the applications there with tabs in their title bar have
//! them, and the empty parts of the bar move the window. Elsewhere the
//! system draws the title bar and nothing here applies.
//!
//! The buttons keep the system's size whatever the zoom: their room is in
//! the toolkit's pixels, and they are moved to the middle of the bar when
//! the zoom makes it taller or lower.
//!
//! Sources: gpui's `TitlebarOptions` (`appears_transparent`,
//! `traffic_light_position`), `Window::start_window_move`,
//! `Window::titlebar_double_click`, `Window::set_traffic_light_position`
//! and `App::set_window_appearance`; Apple's Human Interface Guidelines,
//! "Windows" (the title bar and its controls; a double click on the bar
//! does what the user chose in the system's settings).

use gpui_kit::{
    App, MouseDownEvent, Pixels, Point, SharedString, TitlebarOptions, Window, WindowAppearance,
    point, px,
};

/// Whether the window's buttons stand in the bar of tabs.
pub const IN_TABS: bool = cfg!(target_os = "macos");

/// The room the three buttons take at the left of the bar, in the
/// toolkit's pixels.
const BUTTONS_ROOM: f32 = 78.0;

/// The left edge of the first button, from the window's.
const BUTTONS_LEFT: f32 = 13.0;

/// How tall a button is.
const BUTTON_HEIGHT: f32 = 14.0;

/// The window's title bar: the system's, or on macOS one that is not drawn
/// and has its buttons where [`buttons_at`] says.
pub fn titlebar(title: SharedString, bar_height: Pixels) -> TitlebarOptions {
    TitlebarOptions {
        title: Some(title),
        appears_transparent: IN_TABS,
        traffic_light_position: IN_TABS.then(|| buttons_at(bar_height)),
    }
}

/// Where the buttons stand in a bar this tall: in its middle.
pub fn buttons_at(bar_height: Pixels) -> Point<Pixels> {
    let top = (f32::from(bar_height) - BUTTON_HEIGHT) / 2.0;
    point(px(BUTTONS_LEFT), px(top.max(0.0).round()))
}

/// Put the buttons in the middle of a bar this tall, where they are in it.
pub fn place_buttons(window: &Window, bar_height: Pixels) {
    // The toolkit has the call on macOS alone.
    #[cfg(target_os = "macos")]
    window.set_traffic_light_position(buttons_at(bar_height));
    #[cfg(not(target_os = "macos"))]
    let _ = (window, bar_height);
}

/// The room to keep for the buttons now: none where they are not in the
/// bar, and none in full screen, where the system hides them.
pub fn buttons_room(window: &Window) -> Pixels {
    if IN_TABS && !window.is_fullscreen() {
        px(BUTTONS_ROOM)
    } else {
        Pixels::ZERO
    }
}

/// A press on an empty part of the bar: a drag moves the window, and a
/// double click does what the system's settings say (zoom, or minimise).
pub fn bar_pressed(event: &MouseDownEvent, window: &mut Window) {
    if event.click_count >= 2 {
        window.titlebar_double_click();
    } else {
        window.start_window_move();
    }
}

/// Have what the system draws (the window's buttons, its menus, its
/// dialogs) as light or as dark as the theme. Only on macOS: elsewhere the
/// system's frame is left as the user has it.
pub fn follow_theme(dark: bool, cx: &App) {
    if IN_TABS {
        cx.set_window_appearance(Some(if dark {
            WindowAppearance::Dark
        } else {
            WindowAppearance::Light
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_buttons_stand_in_the_middle_of_the_bar() {
        assert_eq!(buttons_at(px(34.0)), point(px(13.0), px(10.0)));
        assert_eq!(buttons_at(px(51.0)), point(px(13.0), px(19.0)));
        // A bar lower than a button: at its top.
        assert_eq!(buttons_at(px(4.0)), point(px(13.0), px(0.0)));
        assert_eq!(buttons_at(px(f32::NAN)).x, px(13.0));
    }
}
