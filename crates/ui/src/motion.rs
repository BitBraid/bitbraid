//! Reduced motion: a setting that stops what moves.
//!
//! Of the window's own only the graph's highlight of a branch moves: the
//! rows outside the branch's run step back and come forward over a tenth
//! of a second. The rest of what moves is the toolkit's:
//! dialogs, notifications, menus and popovers that slide or fade in,
//! sections that open, the marks that turn while something runs, scroll
//! bars that fade. All of it asks the toolkit's flag
//! (`App::reduce_motion`) first, and with the flag set it appears at once
//! and stands still. The toolkit sets the flag from the system's own
//! setting (the desktop portal's `reduced-motion` on Linux) and follows
//! it while the application runs.
//!
//! `reduce_motion` in the preferences sets the flag whatever the system
//! says. Turned off again, the flag goes back to the system's setting as
//! far as the toolkit lets it: it hands the flag back only while it holds
//! what the toolkit last wrote, so a system that asks for reduced motion
//! is followed again from the next start.
//!
//! What the toolkit does not let go of: the caret of a text field blinks
//! whatever the flag says.
//!
//! Sources: the toolkit's `gpui_base::apply_system_reduce_motion` and
//! `App::set_reduce_motion` documentation; the XDG desktop portal's
//! Settings interface (`org.freedesktop.appearance`, `reduced-motion`).

use gpui_kit::{App, Global};

use crate::prefs::Prefs;

/// What the preference was when it was last applied.
#[derive(Debug, Clone, Copy, Default)]
struct Applied(bool);

impl Global for Applied {}

/// Apply the preference as it was loaded, and follow its changes. Off,
/// the flag is left to the toolkit (and to whoever set it before).
pub fn init(cx: &mut App) {
    cx.set_global(Applied(false));
    sync(cx);
    cx.observe_global::<Prefs>(sync).detach();
}

/// Bring the toolkit's flag in line with the preference when the
/// preference changed.
pub fn sync(cx: &mut App) {
    let Some(wanted) = cx
        .try_global::<Prefs>()
        .map(|prefs| prefs.preferences.reduce_motion)
    else {
        return;
    };
    let applied = cx.try_global::<Applied>().copied().unwrap_or_default();
    if applied.0 == wanted {
        return;
    }
    cx.set_global(Applied(wanted));
    if wanted {
        cx.set_reduce_motion(true);
    } else {
        cx.set_reduce_motion(false);
        gpui_kit::base::apply_system_reduce_motion(cx);
    }
    cx.refresh_windows();
}
