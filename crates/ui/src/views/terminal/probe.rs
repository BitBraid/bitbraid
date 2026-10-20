//! What the terminal of the shown tab has on show, as plain facts, for the
//! off-screen tests (see `shell/probe.rs`). Nothing here changes anything.

use std::path::PathBuf;

use gpui_kit::{App, Entity, Window};

use super::panel::TerminalPanel;
use super::sessions::Terminals;

/// What the terminal of the shown tab has on show.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TerminalProbe {
    /// Whether the repository's state says the panel is on show.
    pub shown: bool,
    /// Whether it shows its heading only.
    pub minimised: bool,
    /// How high it was dragged; none while it is as high as it starts.
    pub height: Option<f32>,
    /// What the tabs say, one per terminal, and which is on show.
    pub names: Vec<String>,
    pub active: usize,
    /// The folders they were started in.
    pub folders: Vec<PathBuf>,
    /// How many of them run.
    pub running: usize,
    /// Whether the terminal on show has the keys.
    pub focused: bool,
    /// The entries of the menu over the text, while it is open.
    pub menu: Vec<&'static str>,
    /// The terminals by their ids: a terminal started afresh has another.
    pub ids: Vec<u64>,
    /// What the terminals were last drawn with.
    pub look: Option<super::look::Look>,
    /// What the text of the terminal on show shows, a line per row.
    pub screen: Option<String>,
    /// Why the terminal on show could not be started, when it could not.
    pub failed: Option<String>,
    /// Every folder of the window that has a terminal.
    pub window_folders: Vec<PathBuf>,
    /// The completions on show in the terminal on show.
    pub completions: Vec<String>,
    /// How many branches the terminals complete with, once their names
    /// came.
    pub completion_branches: Option<usize>,
}

/// What the panel of `path` has on show.
pub fn probe(
    terminals: &Entity<Terminals>,
    path: Option<&std::path::Path>,
    window: &Window,
    cx: &App,
) -> TerminalProbe {
    let window_folders = terminals.read(cx).folders(cx);
    let Some(path) = path else {
        return TerminalProbe {
            window_folders,
            ..TerminalProbe::default()
        };
    };
    let state = cx.global::<crate::prefs::Prefs>().repo_state(path);
    let panel: Option<Entity<TerminalPanel>> = terminals.read(cx).panel(path);
    let panel = panel.as_ref().map(|panel| panel.read(cx));
    TerminalProbe {
        shown: state.terminal_shown,
        minimised: state.terminal_minimised,
        height: state.terminal_height,
        names: panel
            .map(|panel| panel.terminals().iter().map(|t| t.name()).collect())
            .unwrap_or_default(),
        active: panel.map_or(0, TerminalPanel::active),
        folders: panel
            .map(|panel| {
                panel
                    .terminals()
                    .iter()
                    .map(|t| t.folder().to_path_buf())
                    .collect()
            })
            .unwrap_or_default(),
        running: panel.map_or(0, |panel| {
            panel.terminals().iter().filter(|t| t.running(cx)).count()
        }),
        focused: panel.is_some_and(|panel| panel.active_has_keys(window, cx)),
        menu: panel.map(TerminalPanel::menu_labels).unwrap_or_default(),
        ids: panel
            .map(|panel| panel.terminals().iter().map(|t| t.id()).collect())
            .unwrap_or_default(),
        look: panel.and_then(|panel| panel.look().cloned()),
        screen: panel
            .and_then(|panel| panel.terminals().get(panel.active()))
            .and_then(|terminal| terminal.text())
            .map(|text| text.read(cx).probe(window).text),
        failed: panel
            .and_then(|panel| panel.terminals().get(panel.active()))
            .and_then(|terminal| terminal.failed().map(str::to_owned)),
        completions: panel
            .and_then(|panel| panel.terminals().get(panel.active()))
            .and_then(|terminal| terminal.text())
            .map(|text| text.read(cx).probe(window).completions)
            .unwrap_or_default(),
        completion_branches: panel
            .and_then(|panel| panel.words())
            .map(|words| words.branches.len()),

        window_folders,
    }
}
