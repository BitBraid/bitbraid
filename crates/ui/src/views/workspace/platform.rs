//! What the system has to say of a repository. On Windows: files whose
//! paths are longer than it takes while git's long paths are off, which
//! git cannot check out or read (`repo.long_paths`); and a repository
//! inside a WSL distribution, for which the distribution's git is run
//! (`repo.wsl`). A banner over the graph says it until it is put away,
//! and turns long paths on from there. Elsewhere there is nothing to say,
//! and nothing is asked.
//!
//! Sources: Git for Windows' documentation of `core.longpaths`; Microsoft
//! Learn, "Working across Windows and Linux file systems".

use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::Workspace;
use crate::session::{Reply, Request};

/// What the banner says.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlatformNotes {
    /// While git's long paths are off: how many tracked files are beyond
    /// what Windows takes, and how long the longest path is.
    pub long_paths: Option<(usize, usize)>,
    /// The WSL distribution the repository is inside.
    pub wsl: Option<String>,
    /// The banner was put away.
    pub hidden: bool,
}

impl PlatformNotes {
    /// Whether the banner has anything to say.
    pub fn shown(&self) -> bool {
        !self.hidden && (self.long_paths.is_some() || self.wsl.is_some())
    }
}

/// The banner's heading.
const HEADING: &str = "Windows";

/// What the banner says, a line for each thing it knows.
pub fn note_texts(notes: &PlatformNotes) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some((files, longest)) = notes.long_paths {
        lines.push(say!("workspace-platform-long-paths", files, longest));
    }
    if let Some(distribution) = &notes.wsl {
        lines.push(say!("workspace-platform-wsl", distribution));
    }
    lines
}

impl Workspace {
    /// Ask what the system has to say of the repository: only on Windows.
    pub(super) fn look_at_platform(&mut self, cx: &mut Context<Self>) {
        if !cfg!(windows) {
            return;
        }
        self.session.call(
            Request::Command {
                id: "repo.wsl".to_owned(),
                params: json!({}),
            },
            cx,
            |this, reply, cx| {
                if let Reply::Command(found) = reply {
                    this.platform.wsl = found
                        .get("distribution")
                        .and_then(|name| name.as_str())
                        .map(str::to_owned);
                    cx.notify();
                }
            },
        );
        self.ask_long_paths(cx);
    }

    /// Ask which files are too long for Windows while long paths are off.
    fn ask_long_paths(&mut self, cx: &mut Context<Self>) {
        self.session.call(
            Request::Command {
                id: "repo.long_paths".to_owned(),
                params: json!({}),
            },
            cx,
            |this, reply, cx| {
                if let Reply::Command(found) = reply {
                    let blocks = found.get("blocks_git").and_then(|b| b.as_bool()) == Some(true);
                    let count = |name: &str| {
                        found
                            .get(name)
                            .and_then(|n| n.as_u64())
                            .and_then(|n| usize::try_from(n).ok())
                            .unwrap_or(0)
                    };
                    this.platform.long_paths =
                        blocks.then(|| (count("beyond"), count("longest_length")));
                    cx.notify();
                }
            },
        );
    }

    /// Show these notes in the banner, as Windows would have them: what
    /// the off-screen tests look at on a system that has nothing to say.
    pub fn show_platform_notes(&mut self, notes: PlatformNotes, cx: &mut Context<Self>) {
        self.platform = notes;
        cx.notify();
    }

    /// Turn git's long paths on for this repository, and ask again: the
    /// session answers in turn, so the answer is from after the change,
    /// and the line goes when it worked.
    fn turn_on_long_paths(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_mutation(
            "repo.long_paths_enable",
            json!({}),
            say!("workspace-platform-long-paths-on").to_owned(),
            |_, _, _, _| false,
            window,
            cx,
        );
        self.ask_long_paths(cx);
    }

    /// The banner, when it has something to say.
    pub(super) fn render_platform_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.platform.shown() {
            return None;
        }
        let theme = cx.theme().clone();
        let long_paths = self.platform.long_paths.is_some();
        Some(
            h_flex()
                .id("platform-banner")
                .w_full()
                .px_3()
                .py_1()
                .gap_2()
                .items_start()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.muted)
                .text_sm()
                .child(div().flex_none().font_semibold().child(HEADING))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .text_color(theme.muted_foreground)
                        .children(
                            note_texts(&self.platform)
                                .into_iter()
                                .map(|line| div().whitespace_normal().child(line)),
                        ),
                )
                .when(long_paths, |banner| {
                    banner.child(
                        Button::new("platform-long-paths")
                            .small()
                            .outline()
                            .label(say!("workspace-platform-turn-on-long-paths"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.turn_on_long_paths(window, cx)
                            })),
                    )
                })
                .child(
                    Button::new("platform-hide")
                        .small()
                        .ghost()
                        .label(say!("workspace-platform-hide"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.platform.hidden = true;
                            cx.notify();
                        })),
                )
                .test_support()
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    // Not a glob: `gpui_kit::*` carries a `test` attribute of its own.
    use super::{PlatformNotes, note_texts};

    #[test]
    fn the_banner_says_what_it_knows() {
        let mut notes = PlatformNotes::default();
        assert!(!notes.shown());
        assert!(note_texts(&notes).is_empty());
        notes.long_paths = Some((1, 301));
        let lines = note_texts(&notes);
        assert!(
            lines[0].starts_with("One file here has a path longer"),
            "{lines:?}"
        );
        assert!(lines[0].contains("301 characters"));
        notes.long_paths = Some((12, 280));
        notes.wsl = Some("Ubuntu".to_owned());
        let lines = note_texts(&notes);
        assert!(lines[0].starts_with("12 files here have paths"));
        assert!(lines[1].contains("inside WSL (Ubuntu)"));
        assert!(notes.shown());
        notes.hidden = true;
        assert!(!notes.shown());
    }
}
