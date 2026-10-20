//! The section of the preferences for the terminal inside the window: the
//! font, its size, the height of a line, the cursor, dimming while the
//! keys are elsewhere, and the shell to start. A change shows at once in
//! the terminals that are open; the shell is for those started afterwards.
//! Under a heading of their own, the settings of completion: whether git
//! commands are completed, and what Tab does while completions are on
//! show.
//!
//! Sources: the toolkit's text system (`all_font_names`, `resolve_font`,
//! `advance`).

use gpui_kit::*;

use super::{Group, Note, Number, PreferencesView, Typed, change, row};
use crate::prefs::{
    COMPLETION_TABS, Prefs, TERMINAL_CURSORS, TERMINAL_LINE_HEIGHTS, TERMINAL_SIZES,
};

/// The monospace families of the machine, once they are found: every font
/// has to be loaded to be measured, which is done once, off the window's
/// thread, the first time the section is shown.
#[derive(Debug, Default)]
struct MonoFonts {
    found: Option<Vec<String>>,
    asked: bool,
}

impl Global for MonoFonts {}

/// Whether a family sets every character in a cell of one width: a
/// narrow letter, a wide one, a digit and a mark as wide as each other.
fn is_monospace(family: &str, system: &TextSystem) -> bool {
    let id = system.resolve_font(&font(SharedString::from(family.to_owned())));
    let size = gpui_kit::px(12.);
    let widths: Vec<f32> = ['i', 'M', '0', '.']
        .iter()
        .filter_map(|c| system.advance(id, size, *c).ok())
        .map(|advance| f32::from(advance.width))
        .collect();
    let Some(first) = widths.first().copied() else {
        return false;
    };
    widths.len() == 4
        && widths
            .iter()
            .all(|width| width.is_finite() && *width > 0.0 && (width - first).abs() < 0.01)
}

/// A number the preferences keep as an `f32`, as its field shows it: to
/// the hundredth, so that 1.2 is not 1.2000000476837158.
fn shown(value: f32) -> f64 {
    (f64::from(value) * 100.0).round() / 100.0
}

/// The monospace families of a text system, by name.
fn find_mono_fonts(system: &TextSystem) -> Vec<String> {
    let mut names: Vec<String> = system
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('.') && !name.trim().is_empty())
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names.dedup();
    // Only so many are measured: a machine with thousands of fonts is not
    // made to wait for all of them.
    names.truncate(2000);
    names
        .into_iter()
        .filter(|name| is_monospace(name, system))
        .collect()
}

/// The monospace families there are, by name; none until they are found,
/// which the first call starts (the section is drawn again then).
fn mono_fonts(cx: &mut App) -> Vec<String> {
    let known = cx.default_global::<MonoFonts>();
    if let Some(found) = &known.found {
        return found.clone();
    }
    if !known.asked {
        known.asked = true;
        let system = cx.text_system().clone();
        let finding = cx
            .background_executor()
            .spawn(async move { find_mono_fonts(&system) });
        cx.spawn(async move |cx| {
            let found = finding.await;
            cx.update(|cx| {
                cx.default_global::<MonoFonts>().found = Some(found);
                cx.refresh_windows();
            });
        })
        .detach();
    }
    Vec::new()
}

impl PreferencesView {
    /// How the terminal looks, and what it starts.
    pub(super) fn terminal_section(&self, cx: &mut Context<Self>) -> Vec<Group> {
        use gpui_kit::component::ActiveTheme as _;
        let theirs = cx.theme().mono_font_family.to_string();
        let mut fonts: Vec<(SharedString, SharedString)> = vec![(
            SharedString::default(),
            say!("preferences-terminal-font-theirs", font = &theirs).into(),
        )];
        fonts.extend(
            mono_fonts(cx)
                .into_iter()
                .filter(|name| *name != theirs)
                .map(|name| (name.clone().into(), name.into())),
        );
        let chosen: SharedString = cx
            .global::<Prefs>()
            .preferences
            .terminal_panel
            .font_family
            .clone()
            .unwrap_or_default()
            .into();
        // A family that was chosen and is not here any more is listed all
        // the same, so that the list says what is in force.
        if !chosen.is_empty() && !fonts.iter().any(|(word, _)| *word == chosen) {
            fonts.push((
                chosen.clone(),
                say!("preferences-terminal-font-missing", font = chosen.as_str()).into(),
            ));
        }
        let cursors: Vec<(SharedString, SharedString)> = TERMINAL_CURSORS
            .iter()
            .map(|(word, label)| ((*word).into(), label.text().into()))
            .collect();
        vec![
            Group::new()
                .item(row::choose(
                    say!("preferences-terminal-font"),
                    Note::plain(say!("preferences-terminal-font-note")),
                    fonts,
                    chosen,
                    std::rc::Rc::new(|value, _, cx| {
                        change(cx, |preferences| {
                            preferences.terminal_panel.font_family =
                                (!value.is_empty()).then(|| value.to_string())
                        })
                    }),
                ))
                .item(row::number(
                    say!("preferences-terminal-font-size"),
                    Note::None,
                    Number {
                        min: f64::from(TERMINAL_SIZES.0),
                        max: f64::from(TERMINAL_SIZES.1),
                        get: |preferences| shown(preferences.terminal_panel.font_size),
                        set: |preferences, size| preferences.terminal_panel.font_size = size as f32,
                    },
                ))
                .item(row::number(
                    say!("preferences-terminal-line-height"),
                    Note::plain(say!("preferences-terminal-line-height-note")),
                    Number {
                        min: f64::from(TERMINAL_LINE_HEIGHTS.0),
                        max: f64::from(TERMINAL_LINE_HEIGHTS.1),
                        get: |preferences| shown(preferences.terminal_panel.line_height),
                        set: |preferences, height| {
                            preferences.terminal_panel.line_height = height as f32
                        },
                    },
                ))
                .item(row::select(
                    say!("preferences-terminal-cursor"),
                    Note::None,
                    cursors,
                    |cx| {
                        cx.global::<Prefs>()
                            .preferences
                            .terminal_panel
                            .cursor
                            .clone()
                            .into()
                    },
                    |value, cx| {
                        change(cx, |preferences| {
                            preferences.terminal_panel.cursor = value.to_string()
                        })
                    },
                ))
                .item(row::check(
                    say!("preferences-terminal-dim"),
                    Note::plain(say!("preferences-terminal-dim-note")),
                    |preferences| preferences.terminal_panel.dim_unfocused,
                    |preferences, on| preferences.terminal_panel.dim_unfocused = on,
                ))
                .item(row::text(
                    say!("preferences-terminal-shell"),
                    Note::Marked(say!("preferences-terminal-shell-note")),
                    Typed {
                        show: |preferences| {
                            preferences.terminal_panel.shell.clone().unwrap_or_default()
                        },
                        take: |preferences, typed| {
                            let typed = typed.trim();
                            preferences.terminal_panel.shell =
                                (!typed.is_empty()).then(|| typed.to_owned())
                        },
                    },
                    true,
                )),
            Group::titled(say!("preferences-terminal-completion"))
                .item(row::check(
                    say!("preferences-terminal-git-completion"),
                    Note::plain(say!("preferences-terminal-git-completion-note")),
                    |preferences| preferences.terminal_panel.completion,
                    |preferences, on| preferences.terminal_panel.completion = on,
                ))
                .item(row::select(
                    say!("preferences-terminal-tab"),
                    Note::plain(say!("preferences-terminal-tab-note")),
                    COMPLETION_TABS
                        .iter()
                        .map(|(word, label)| ((*word).into(), label.text().into()))
                        .collect(),
                    |cx| {
                        cx.global::<Prefs>()
                            .preferences
                            .terminal_panel
                            .completion_tab
                            .clone()
                            .into()
                    },
                    |value, cx| {
                        change(cx, |preferences| {
                            preferences.terminal_panel.completion_tab = value.to_string()
                        })
                    },
                )),
        ]
    }
}

#[cfg(test)]
mod tests {
    use crate::prefs::Preferences;

    /// A number kept as an `f32` shows to the hundredth, as it was typed.
    #[test]
    fn a_number_of_the_section_shows_as_it_was_typed() {
        assert_eq!(super::shown(1.2), 1.2);
        assert_eq!(super::shown(12.0), 12.0);
        assert_eq!(super::shown(1.25), 1.25);
        assert_eq!(
            crate::views::preferences::row::number_text(super::shown(1.2)),
            "1.2"
        );
    }

    #[test]
    fn the_terminals_settings_are_kept_in_range() {
        let mut preferences = Preferences::default();
        assert_eq!(preferences.terminal_panel.font_size, 12.0);
        assert_eq!(preferences.terminal_panel.line_height, 1.2);
        assert_eq!(preferences.terminal_panel.cursor, "bar");
        assert!(preferences.terminal_panel.dim_unfocused);
        preferences.terminal_panel.font_size = 400.0;
        preferences.terminal_panel.line_height = f32::NAN;
        preferences.terminal_panel.cursor = "beam".to_owned();
        preferences.terminal_panel.shell = Some("   ".to_owned());
        preferences.terminal_panel.font_family = Some(String::new());
        preferences.terminal_panel.completion_tab = "sometimes".to_owned();
        preferences.sanitize();
        assert_eq!(preferences.terminal_panel.completion_tab, "shell");
        assert_eq!(preferences.terminal_panel.font_size, 48.0);
        assert_eq!(preferences.terminal_panel.line_height, 1.2);
        assert_eq!(preferences.terminal_panel.cursor, "bar");
        assert_eq!(preferences.terminal_panel.shell, None);
        assert_eq!(preferences.terminal_panel.font_family, None);
        // A file without the terminal's settings reads as the defaults.
        let read: Preferences = toml::from_str("theme = \"light\"\n").unwrap();
        assert_eq!(read.terminal_panel, crate::prefs::TerminalPrefs::default());
        let read: Preferences =
            toml::from_str("[terminal_panel]\nshell = \"fish\"\ncursor = \"block\"\n").unwrap();
        assert_eq!(read.terminal_panel.shell.as_deref(), Some("fish"));
        assert_eq!(read.terminal_panel.cursor, "block");
        assert_eq!(read.terminal_panel.font_size, 12.0);
        // Completion is on, and Tab goes to the shell, until changed.
        assert!(read.terminal_panel.completion);
        assert_eq!(read.terminal_panel.completion_tab, "shell");
    }
}
