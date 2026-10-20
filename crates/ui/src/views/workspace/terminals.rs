//! The terminal of a repository's tab: where its panel stands in the
//! middle column, how high it is, and how a terminal is started.
//!
//! The panel stands at the bottom of the graph's column, as wide as the
//! column and down to the status bar. It starts 275 high and is dragged
//! by its top edge, no lower than that and no higher than leaves the graph
//! its heading and about two rows; minimised it is its heading alone. How
//! high it is, whether it is on show and whether it is minimised is kept
//! per repository. Whatever takes the graph's place (a file, its past,
//! the merge tool, the plan of a rebase, a page of the centre) puts the
//! panel away until the graph is back; the shells keep running.
//!
//! A terminal is started in the repository's folder or in a folder inside
//! it: the engine says what to start there (`terminal.shell`), with the
//! shell the preferences name. When a command ends in it, the repository
//! is read again, at most once a second, so that what it did shows in the
//! graph even where the watcher misses it.
//!
//! What git commands typed in them are completed with is read when a
//! terminal starts and after each command typed in one
//! (`terminal.completion_words`, for the tracked paths); the branches,
//! remotes, tags, stashes and changes in it follow what the tab reads
//! for its own lists, and the panel hands them to each terminal.
//!
//! Sources: `AGENTS.md` section 2 (the terminal).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::*;

use serde_json::json;

use super::Workspace;
use crate::prefs::Prefs;
use crate::session::{Reply, Request};
use crate::views::terminal::panel::{
    GRAPH_KEPT, LEAST_HEIGHT, MINIMISED_HEIGHT, PanelDo, PanelEvent, TerminalPanel,
};

use crate::views::terminal::sessions::{self, Terminals, TerminalsEvent};
use crate::zoom::px;

/// The least time between two readings of the repository that commands
/// in its terminals asked for.
const REFRESH_GAP: Duration = Duration::from_secs(1);

/// What a tab keeps of the window's terminals.
#[derive(Default)]
pub(super) struct TerminalLink {
    /// The window's terminals, once the window handed them over.
    terminals: Option<Entity<Terminals>>,
    /// The names git commands typed in the terminals are completed with,
    /// as the engine read them last; the references and the
    /// changes in them follow the tab's own.
    words: Option<bitbraid_engine::completion::Words>,
    /// They are being read.
    words_asked: bool,
    /// When a command last had the repository read again, and whether a
    /// reading waits for the gap to pass.
    refreshed: Option<Instant>,
    refresh_waiting: bool,
}

/// What is dragged while the panel's top edge is: nothing follows the
/// pointer.
#[derive(Debug, Clone, Copy)]
struct PanelEdge;

struct NoChip;

impl Render for NoChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// How high the panel is, from how high it was dragged and what is left
/// of the column for it: no lower than its least and no higher than
/// leaves the graph what it keeps. A column too low for both keeps the
/// least.
// Not `clamp`: of a height that is no number it makes no number.
#[allow(clippy::manual_clamp)]
pub fn panel_height(wanted: f32, column: f32) -> f32 {
    let most = (column - GRAPH_KEPT).max(LEAST_HEIGHT);
    if wanted.is_finite() {
        wanted.max(LEAST_HEIGHT).min(most)
    } else {
        LEAST_HEIGHT
    }
}

/// The folder a terminal asked for in `wanted` starts in: that folder when
/// it is inside the repository's `root` and there; the nearest folder
/// above it that is there when it is not (a folder of a commit the working
/// tree no longer has); the repository's own otherwise.
pub fn start_in(
    wanted: Option<PathBuf>,
    root: &std::path::Path,
    is_dir: impl Fn(&std::path::Path) -> bool,
) -> PathBuf {
    let Some(wanted) = wanted else {
        return root.to_path_buf();
    };
    // `..` is not followed out of the repository.
    let inside = wanted.starts_with(root)
        && !wanted
            .components()
            .any(|part| part == std::path::Component::ParentDir);
    if !inside {
        return root.to_path_buf();
    }
    wanted
        .ancestors()
        .take_while(|folder| folder.starts_with(root))
        .find(|folder| is_dir(folder))
        .map_or_else(|| root.to_path_buf(), std::path::Path::to_path_buf)
}

/// How high the panel is in a measuring run of the terminal.
const BENCH_HEIGHT: f32 = 480.0;

/// A command of the terminal's that the keys and the palette run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminalKey {
    /// Show or hide the panel, starting a terminal when there is none.
    Toggle,
    /// Start another terminal.
    New,
    /// Show the next terminal, or the previous one.
    Next,
    Previous,
    /// End the terminal on show.
    Close,
}

impl Workspace {
    /// Take the window's terminals: the panel of this repository is drawn
    /// from them, and what they say about it is heard here.
    pub fn set_terminals(
        &mut self,
        terminals: Entity<Terminals>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let heard = cx.subscribe_in(
            &terminals,
            window,
            |this, _, event: &TerminalsEvent, window, cx| {
                if event.path != this.session.path {
                    return;
                }
                match &event.what {
                    PanelEvent::Changed => cx.notify(),
                    PanelEvent::Wanted(folder) => this.start_terminal(folder.clone(), window, cx),
                    PanelEvent::CommandFinished => this.refresh_after_command(cx),
                    PanelEvent::Left => {
                        this.focus_graph(window, cx);
                    }
                }
            },
        );
        self._subscriptions.push(heard);
        self.terminal.terminals = Some(terminals);
        // Shown when the repository was last closed: shown again, with a
        // shell of its own when the window has none for it.
        // A measuring run of the terminal (`text::BENCH_VARIABLE`) opens
        // one, about as high as it goes in a window of 1280 by 720.
        let measuring = crate::views::terminal::text::bench_seconds().is_some();
        if measuring {
            cx.global_mut::<Prefs>()
                .update_repo_state(&self.session.path, |state| {
                    state.terminal_height = Some(BENCH_HEIGHT)
                });
        }
        let state = cx.global::<Prefs>().repo_state(&self.session.path);
        if (state.terminal_shown || measuring)
            && self
                .terminal_panel(cx)
                .is_none_or(|p| p.read(cx).is_empty())
        {
            self.start_terminal(None, window, cx);
            // As it was: a terminal asked for comes up whole, one brought
            // back keeps being minimised.
            if state.terminal_minimised
                && let Some(panel) = self.terminal_panel(cx)
            {
                panel.update(cx, |panel, cx| panel.set_minimised(true, cx));
            }
        }
    }

    /// The panel of this repository's folder, when it has one.
    pub fn terminal_panel(&self, cx: &App) -> Option<Entity<TerminalPanel>> {
        self.terminal
            .terminals
            .as_ref()
            .and_then(|terminals| terminals.read(cx).panel(&self.session.path))
    }

    /// Whether the terminal can be shown and started now: not while the
    /// plan of an interactive rebase takes the graph's place.
    pub fn terminal_offered(&self) -> bool {
        !self.planning
    }

    /// Run one of the terminal's commands.
    pub fn terminal_key(&mut self, key: TerminalKey, window: &mut Window, cx: &mut Context<Self>) {
        if !self.terminal_offered() {
            return;
        }
        let panel = self.terminal_panel(cx);
        let has_terminals = panel.as_ref().is_some_and(|p| !p.read(cx).is_empty());
        match key {
            TerminalKey::Toggle => {
                let Some(panel) = panel.filter(|_| has_terminals) else {
                    self.start_terminal(None, window, cx);
                    return;
                };
                let shown = panel.read(cx).shown(cx);
                panel.update(cx, |panel, cx| {
                    if shown {
                        panel.run(PanelDo::Hide, window, cx);
                    } else {
                        panel.set_shown(true, cx);
                        panel.focus_active(window, cx);
                    }
                });
            }
            TerminalKey::New => self.start_terminal(None, window, cx),
            TerminalKey::Next | TerminalKey::Previous => {
                if let Some(panel) = panel {
                    panel.update(cx, |panel, cx| {
                        panel.step(key == TerminalKey::Next, window, cx)
                    });
                }
            }
            TerminalKey::Close => {
                if let Some(panel) = panel {
                    panel.update(cx, |panel, cx| panel.run(PanelDo::End, window, cx));
                }
            }
        }
        cx.notify();
    }

    /// Start a terminal in a folder of the repository (none: its own
    /// folder), and show it.
    pub fn start_terminal(
        &mut self,
        folder: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(terminals) = self.terminal.terminals.clone() else {
            return;
        };
        let path = self.session.path.clone();
        let root = self
            .session
            .summary
            .workdir
            .clone()
            .unwrap_or_else(|| path.clone());
        let folder = start_in(folder, &root, |folder| folder.is_dir());
        let panel = terminals.update(cx, |terminals, cx| terminals.panel_or_new(&path, cx));
        let id = panel.update(cx, |panel, cx| panel.add(folder.clone(), cx));
        if panel.read(cx).words().is_none() {
            self.ask_completion_words(cx);
        }
        cx.notify();
        // What runs in it: the shell the engine names for this folder,
        // with the one the preferences name.
        let chosen = cx
            .global::<Prefs>()
            .preferences
            .terminal_panel
            .shell
            .clone();
        let (program, args) = match chosen.as_deref().map(sessions::command_words) {
            None => (None, Vec::new()),
            Some(Some(mut words)) if !words.is_empty() => {
                let program = words.remove(0);
                (Some(program), words)
            }
            Some(_) => {
                let why = say!("workspace-terminal-unclosed-quote");
                panel.update(cx, |panel, cx| panel.failed(id, why.to_owned(), cx));
                return;
            }
        };
        let reply = self.session.send(Request::Command {
            id: "terminal.shell".to_owned(),
            params: json!({
                "path": folder.to_string_lossy(),
                "program": program,
                "args": args,
            }),
        });
        let weak = panel.downgrade();
        cx.spawn_in(window, async move |_, cx| {
            let reply = reply.await;
            let _ = cx.update(|window, cx| {
                let started = match reply {
                    Ok(Reply::Command(value)) => match sessions::spec_of(&value) {
                        Some(spec) => sessions::open_text(&spec, window, cx),
                        None => Err(say!("workspace-terminal-no-shell-named").to_owned()),
                    },
                    Ok(Reply::Error(err)) => Err(crate::shell::sentence(&err)),
                    Ok(_) | Err(_) => Err(say!("workspace-terminal-no-shell").to_owned()),
                };
                let _ = weak.update(cx, |panel, cx| match started {
                    Ok(running) => {
                        panel.attach(id, running, window, cx);
                        panel.focus_active(window, cx);
                    }
                    Err(why) => panel.failed(id, why, cx),
                });
            });
        })
        .detach();
    }

    /// A command ended in one of the repository's terminals: read the
    /// repository again, unless that was done less than a second ago, in
    /// which case once the second has passed.
    fn refresh_after_command(&mut self, cx: &mut Context<Self>) {
        let since = self.terminal.refreshed.map(|at| at.elapsed());
        match since {
            Some(since) if since < REFRESH_GAP => {
                if self.terminal.refresh_waiting {
                    return;
                }
                self.terminal.refresh_waiting = true;
                let wait = REFRESH_GAP.saturating_sub(since);
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(wait).await;
                    let _ = this.update(cx, |this, cx| {
                        this.terminal.refresh_waiting = false;
                        this.read_after_command(cx);
                    });
                })
                .detach();
            }
            _ => self.read_after_command(cx),
        }
    }

    fn read_after_command(&mut self, cx: &mut Context<Self>) {
        self.terminal.refreshed = Some(Instant::now());
        self.ask_completion_words(cx);
        self.reload(cx);
        self.load_status(cx);
    }

    /// The graph with the terminal's panel under it, while the panel is on
    /// show.
    pub(super) fn with_terminal(&self, above: AnyElement, cx: &mut Context<Self>) -> AnyElement {
        let Some(panel) = self.terminal_panel(cx).filter(|p| p.read(cx).shown(cx)) else {
            return above;
        };
        let path = self.session.path.clone();
        let state = cx.global::<Prefs>().repo_state(&path);
        let minimised = state.terminal_minimised;
        let wanted = state.terminal_height.unwrap_or(LEAST_HEIGHT);
        let hover = cx.theme().primary.opacity(0.6);
        let place = if minimised {
            div().h(px(MINIMISED_HEIGHT)).flex_none().child(panel)
        } else {
            div()
                .relative()
                .h(px(panel_height(wanted, f32::INFINITY)))
                .min_h(px(LEAST_HEIGHT))
                .flex_shrink(1.)
                .child(panel)
                // Its top edge is dragged to make it higher or lower.
                .child(
                    div()
                        .id("terminal-edge")
                        .absolute()
                        .top(px(-2.))
                        .left_0()
                        .w_full()
                        .h(px(5.))
                        .cursor_row_resize()
                        .hover(move |style| style.bg(hover))
                        .on_drag(PanelEdge, |_, _, _, cx| cx.new(|_| NoChip))
                        .test_support(),
                )
        };
        v_flex()
            .id("terminal-column")
            .size_full()
            .on_drag_move(
                cx.listener(move |_, event: &DragMoveEvent<PanelEdge>, _, cx| {
                    let y = crate::zoom::unpx(event.event.position.y);
                    let top = crate::zoom::unpx(event.bounds.top());
                    let bottom = crate::zoom::unpx(event.bounds.bottom());
                    let height = panel_height(bottom - y, bottom - top);
                    cx.global_mut::<Prefs>()
                        .update_repo_state(&path, |state| state.terminal_height = Some(height));
                    cx.notify();
                }),
            )
            .child(
                div()
                    .flex_1()
                    .min_h(px(if minimised { 0. } else { GRAPH_KEPT }))
                    .overflow_hidden()
                    .child(above),
            )
            .child(place)
            .test_support()
            .into_any_element()
    }
}

impl Workspace {
    /// Read the names git commands typed in the terminals are completed
    /// with, unless that is under way; what changed comes from the tab's
    /// own list of changes.
    pub(super) fn ask_completion_words(&mut self, cx: &mut Context<Self>) {
        if self.terminal.words_asked {
            return;
        }
        self.terminal.words_asked = true;
        let request = Request::Command {
            id: "terminal.completion_words".to_owned(),
            params: json!({ "changes": false }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            this.terminal.words_asked = false;
            let Reply::Command(value) = reply else {
                return;
            };
            if let Ok(words) = serde_json::from_value(value) {
                this.terminal.words = Some(words);
                this.completion_words_follow(cx);
            }
        });
    }

    /// The tab read its references or its changes again: the terminals
    /// complete with them from now on.
    pub(super) fn completion_words_follow(&mut self, cx: &mut Context<Self>) {
        let Some(panel) = self.terminal_panel(cx) else {
            return;
        };
        let Some(words) = self.terminal.words.as_mut() else {
            return;
        };
        if let Some(snapshot) = self.snapshot.as_ref() {
            bitbraid_engine::completion_cmds::set_names(words, snapshot);
        }
        words.changes = self
            .status
            .iter()
            .map(bitbraid_engine::completion_cmds::changed_path)
            .collect();
        let words = std::sync::Arc::new(words.clone());
        panel.update(cx, |panel, cx| panel.set_words(words, cx));
    }
}

#[cfg(test)]
mod tests {
    use super::{panel_height, start_in};
    use std::path::{Path, PathBuf};

    #[test]
    fn a_terminal_starts_in_a_folder_of_the_repository_that_is_there() {
        let root = Path::new("/work/project");
        let there = |folder: &Path| {
            [
                "/work/project",
                "/work/project/src",
                "/work/project/src/ui",
                "/work",
            ]
            .contains(&folder.to_str().unwrap_or(""))
        };
        let start = |wanted: Option<&str>| start_in(wanted.map(PathBuf::from), root, there);
        assert_eq!(start(None), root);
        assert_eq!(
            start(Some("/work/project/src/ui")),
            Path::new("/work/project/src/ui")
        );
        // Gone from the working tree: the nearest folder above that is there.
        assert_eq!(
            start(Some("/work/project/src/gone/deeper")),
            Path::new("/work/project/src")
        );
        // Outside the repository, or climbing out of it: the repository's.
        assert_eq!(start(Some("/work")), root);
        assert_eq!(start(Some("/etc")), root);
        assert_eq!(start(Some("/work/project/../../etc")), root);
        assert_eq!(start(Some("relative/path")), root);
    }
    use crate::views::terminal::panel::{GRAPH_KEPT, LEAST_HEIGHT};

    #[test]
    fn the_panel_is_dragged_between_its_least_and_what_leaves_the_graph_its_rows() {
        // A column of 613 (a window 720 high): from 275 to 543.
        assert_eq!(panel_height(300., 613.), 300.);
        assert_eq!(panel_height(100., 613.), LEAST_HEIGHT);
        assert_eq!(panel_height(5000., 613.), 613. - GRAPH_KEPT);
        // Too low a column for both: the least.
        assert_eq!(panel_height(500., 200.), LEAST_HEIGHT);
        for nothing in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(panel_height(nothing, 613.), LEAST_HEIGHT);
        }
        assert_eq!(panel_height(400., f32::NAN), LEAST_HEIGHT);
    }
}
