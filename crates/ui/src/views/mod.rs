//! Views of the application.

pub mod about;
pub mod activity;
pub mod avatar;
pub mod blame;
pub mod changes;
pub mod commit_box;
pub mod details;
pub mod diff;
pub mod diff_doc;
pub mod diff_syntax;
pub mod drag_paths;
pub mod edit;
pub mod file_actions;
pub mod file_tree;
pub mod find_file;
pub mod flow;
pub mod graph;
pub mod history;
pub mod image_diff;
pub mod inbox;
pub mod index_flags;
pub mod integrations;
pub mod issues;
pub mod journal;
pub mod key_list;
pub mod lfs;
pub mod maintenance;
pub mod menu_fit;
pub mod merge_tool;
pub mod notices;
pub mod preferences;
pub mod profiles;
pub mod provider_clone;
pub mod provider_create;
pub mod pulls;
pub mod rebase_editor;
pub mod refs;
pub mod repositories;
pub mod rerere;
pub mod rewrite;
pub mod shortcuts;
pub mod signatures;
pub mod signing;
pub mod sparse;
pub mod start;
pub mod stashes;
pub mod stopped;
pub mod submodules;
pub mod tags;
pub mod terminal;
pub mod text_select;
pub mod workspace;
pub mod worktrees;

/// One of a row of joined buttons, or a switch: marked as the chosen one
/// when it is, by being filled where the others are only outlined.
pub fn chosen(
    button: gpui_kit::component::button::Button,
    selected: bool,
    cx: &gpui_kit::App,
) -> gpui_kit::component::button::Button {
    use gpui_kit::Styled as _;
    use gpui_kit::component::{ActiveTheme as _, Selectable as _};
    let button = button.selected(selected);
    if selected {
        button.bg(cx.theme().primary.opacity(CHOSEN_FILL))
    } else {
        button
    }
}

/// How strongly the chosen one of a row of joined buttons is filled with
/// the theme's own accent.
const CHOSEN_FILL: f32 = 0.34;

/// Relative time such as "3 h ago" for a Unix timestamp, against `now`.
pub fn relative_time(seconds: i64, now: i64) -> String {
    let delta = now.saturating_sub(seconds);
    if delta < 0 {
        return say!("views-in-the-future").to_owned();
    }
    if delta < 60 {
        say!("views-just-now").to_owned()
    } else if delta < 3600 {
        say!("views-minutes-ago", count = delta / 60)
    } else if delta < 86_400 {
        say!("views-hours-ago", count = delta / 3600)
    } else if delta < 86_400 * 30 {
        say!("views-days-ago", count = delta / 86_400)
    } else if delta < 86_400 * 365 {
        say!("views-months-ago", count = delta / (86_400 * 30))
    } else {
        say!("views-years-ago", count = delta / (86_400 * 365))
    }
}

/// The day of a Unix timestamp where it was made, `YYYY-MM-DD`.
pub fn absolute_date(seconds: i64, offset: i32) -> String {
    let local = seconds.saturating_add(i64::from(offset));
    let (y, m, d) = civil_from_days(local.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Absolute local-looking date from a Unix timestamp and offset, `YYYY-MM-DD HH:MM`.
pub fn absolute_time(seconds: i64, offset: i32) -> String {
    let local = seconds.saturating_add(i64::from(offset));
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60
    )
}

// `civil_from_days` of "chrono-Compatible Low-Level Date Algorithms".
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Unix time now, by the clock the journal and the activity log go by
/// too (`BITBRAID_NOW` holds it still, see `bitbraid_core::clock`).
pub fn now() -> i64 {
    bitbraid_core::clock::unix_seconds()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        assert_eq!(relative_time(100, 130), "just now");
        assert_eq!(relative_time(0, 3600 * 5), "5 h ago");
        assert_eq!(relative_time(0, 86_400 * 3), "3 d ago");
        assert_eq!(relative_time(0, 86_400 * 400), "1 y ago");
    }

    #[test]
    fn absolute_times() {
        assert_eq!(absolute_time(1_577_836_800, 0), "2020-01-01 00:00");
        assert_eq!(absolute_time(1_577_836_800, 3600), "2020-01-01 01:00");
        assert_eq!(absolute_time(0, 0), "1970-01-01 00:00");
    }
}
