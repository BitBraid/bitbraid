//! The terminal inside the window: a panel at the bottom of the graph's
//! column with one or more shells of the repository or worktree on show,
//! kept by the window per worktree (`AGENTS.md` section 2).
//!
//! `panel` is the panel of one worktree (its heading, its tabs, its menu),
//! `sessions` the window's list of them, `text` what a session shows and
//! the keys and the pointer it takes.
//!
//! Sources: none (original design).

pub mod completion;
pub mod look;
pub mod panel;
pub mod probe;
pub mod sessions;
pub mod text;

/// The id of the row of the keymap that shows and hides the terminal.
pub const TOGGLE_ID: &str = "terminal.toggle";

/// The key that shows and hides the terminal until the user gives it
/// another: Ctrl+`.
#[cfg(test)]
pub const TOGGLE_KEY: &str = "ctrl-`";

/// The rows of the keymap that are the terminal panel's own: their keys
/// are for the window even while the shell has the keys (the text's
/// `WINDOW_KEYS` are these).
pub const PANEL_IDS: [&str; 5] = [
    TOGGLE_ID,
    "terminal.new",
    "terminal.next",
    "terminal.previous",
    "terminal.close",
];

#[cfg(test)]
mod tests {
    use super::{PANEL_IDS, TOGGLE_ID, TOGGLE_KEY};
    use crate::keymap::{BINDINGS, binding, normal_key, resolve};

    #[test]
    fn the_terminals_keys_are_rows_of_the_keymap_the_palette_finds() {
        let toggle = binding(TOGGLE_ID).unwrap();
        assert_eq!(toggle.keys, [TOGGLE_KEY]);
        assert_eq!(normal_key(TOGGLE_KEY).as_deref(), Some(TOGGLE_KEY));
        for id in PANEL_IDS {
            let row = binding(id).unwrap_or_else(|| panic!("{id}"));
            assert!(row.palette, "{id} is offered by the palette");
            assert!(row.context.is_none(), "{id} applies everywhere");
            assert!(
                row.label.text().starts_with("Terminal")
                    && row
                        .keywords
                        .is_some_and(|words| words.text().contains("terminal")),
                "{id}: typing terminal finds it"
            );
        }
        // Their keys are taken by nothing else.
        let resolved = resolve(&Default::default());
        assert!(resolved.conflicts.is_empty(), "{:?}", resolved.conflicts);
        assert_eq!(resolved.keys_of(TOGGLE_ID), [TOGGLE_KEY]);
        // The system's terminal keeps its own row, told apart.
        let outside = binding("repo.open_terminal").unwrap();
        assert!(outside.label.text().contains("system's"));
        assert_eq!(BINDINGS.iter().filter(|row| row.id == TOGGLE_ID).count(), 1);
    }
}
