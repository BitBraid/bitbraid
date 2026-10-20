//! What the application is: a dialog over the window, from the menu bar's
//! "About BitBraid" on macOS and from the palette everywhere.
//!
//! A card 440 wide, centred, as far under the window's top as the list of
//! keys. On top a rule in the three strands' colours, in the order they
//! enter the mark; under it the theme's logo, 240 wide, with more than a
//! strand's width clear around it, on the icon's tile in a dark theme and
//! on the title bar's colour in a light one. Then the version, the git it
//! runs, the licence and the note on Git's marks, and two buttons: one
//! copies the version, the git and the system, for a report of a fault;
//! one closes. Escape and a click beside the card put it away too.
//!
//! Sources: the drawings in `brand/` and the terms for them in `LICENSE`;
//! Apple's Human Interface Guidelines, "The menu bar" (the application's
//! menu and its About item).

use gpui_kit::component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::*;

use crate::{brand, gitexe::GitExecutable, zoom::px};

/// Width of the card.
pub const WIDTH: f32 = 440.0;
/// Width of the logo on it.
const LOGO_WIDTH: f32 = 240.0;
/// Height of the rule in the strands' colours.
const RULE_HEIGHT: f32 = 4.0;

/// What the dialog asks of the window.
#[derive(Debug, Clone)]
pub enum AboutEvent {
    /// Put the dialog away.
    Close,
}

impl EventEmitter<AboutEvent> for AboutView {}

/// The dialog.
pub struct AboutView {
    focus_handle: FocusHandle,
    /// Whether the details were copied since it was shown.
    copied: bool,
}

impl AboutView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            copied: false,
        }
    }

    /// Shown anew: nothing copied yet.
    pub fn shown(&mut self, cx: &mut Context<Self>) {
        self.copied = false;
        cx.notify();
    }

    /// Whether the details were copied since it was shown; for the tests.
    pub fn copied(&self) -> bool {
        self.copied
    }
}

impl Focusable for AboutView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The git that runs, in a few words: its version and where it is, or
/// that it is the one shipped with the application, or why there is none.
pub fn git_line(git: Option<&GitExecutable>) -> String {
    match git {
        Some(git) => match (&git.version, &git.path) {
            (Some(version), _) if git.shipped => say!("about-git-shipped", version),
            (Some(version), Some(path)) => say!("about-git-at", version, path),
            (Some(version), None) => say!("about-git", version),
            (None, _) if git.asked => say!("about-git-none").to_owned(),
            (None, _) => say!("about-git-looking").to_owned(),
        },
        None => say!("about-git-looking").to_owned(),
    }
}

/// The system, as people name it, and the processor.
fn system() -> String {
    let os = match std::env::consts::OS {
        "macos" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        "freebsd" => "FreeBSD",
        other => other,
    };
    format!("{os}, {}", std::env::consts::ARCH)
}

/// What the copy button puts on the clipboard: the application and its
/// version, the git it runs, the system.
pub fn details(git: Option<&GitExecutable>) -> String {
    format!(
        "{} {}\n{}\n{}",
        brand::NAME,
        env!("CARGO_PKG_VERSION"),
        git_line(git),
        system()
    )
}

impl Render for AboutView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let dark = theme.is_dark();
        let git = cx.try_global::<GitExecutable>().cloned();
        let strands = if dark {
            brand::STRANDS_ON_DARK
        } else {
            brand::STRANDS_ON_LIGHT
        };
        let [tile_top, tile_bottom] = brand::TILE;
        let header = v_flex()
            .id("about-header")
            .w_full()
            .items_center()
            .py(px(32.))
            .px(px(32.));
        let header = if dark {
            header.bg(linear_gradient(
                180.,
                linear_color_stop(rgb(tile_top), 0.),
                linear_color_stop(rgb(tile_bottom), 1.),
            ))
        } else {
            header.bg(theme.title_bar)
        };
        let copy_label = if self.copied {
            say!("about-copied")
        } else {
            say!("about-copy-details")
        };
        v_flex()
            .id("about")
            .track_focus(&self.focus_handle)
            .w(px(WIDTH))
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .overflow_hidden()
            .shadow_lg()
            .child(
                h_flex()
                    .w_full()
                    .h(px(RULE_HEIGHT))
                    .flex_none()
                    .children(strands.map(|colour| div().flex_1().h_full().bg(rgb(colour)))),
            )
            .child(
                header
                    .child(brand::logo_image(dark, px(LOGO_WIDTH), window, cx))
                    .test_support(),
            )
            .child(
                v_flex()
                    .w_full()
                    .items_center()
                    .gap_1()
                    .px(px(32.))
                    .pt(px(18.))
                    .pb(px(14.))
                    .text_sm()
                    .child(
                        div()
                            .id("about-version")
                            .text_base()
                            .font_semibold()
                            .child(say!("about-version", version = env!("CARGO_PKG_VERSION")))
                            .test_support(),
                    )
                    .child(
                        div()
                            .id("about-git")
                            .text_center()
                            .whitespace_normal()
                            .text_color(theme.muted_foreground)
                            .child(git_line(git.as_ref()))
                            .test_support(),
                    )
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(say!("about-licence")),
                    )
                    .child(
                        div()
                            .pt(px(12.))
                            .text_xs()
                            .text_center()
                            .whitespace_normal()
                            .text_color(theme.muted_foreground)
                            .child(say!("about-marks")),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .px(px(16.))
                    .py(px(12.))
                    .border_t_1()
                    .border_color(theme.border)
                    .child(
                        Button::new("about-copy")
                            .outline()
                            .small()
                            .label(copy_label)
                            .on_click(cx.listener(|this, _, _, cx| {
                                let git = cx.try_global::<GitExecutable>().cloned();
                                cx.write_to_clipboard(ClipboardItem::new_string(details(
                                    git.as_ref(),
                                )));
                                this.copied = true;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("about-close")
                            .primary()
                            .small()
                            .label(say!("about-close"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(AboutEvent::Close))),
                    ),
            )
            // Its place is asked for in the tests.
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::{details, git_line};
    use crate::gitexe::GitExecutable;

    fn git(version: Option<&str>, path: Option<&str>, shipped: bool) -> GitExecutable {
        GitExecutable {
            asked: true,
            path: path.map(str::to_owned),
            version: version.map(str::to_owned),
            shipped,
            ..GitExecutable::default()
        }
    }

    #[test]
    fn the_git_that_runs_is_said_in_a_few_words() {
        assert_eq!(git_line(None), "Looking for git…");
        assert_eq!(
            git_line(Some(&GitExecutable::default())),
            "Looking for git…"
        );
        assert_eq!(
            git_line(Some(&git(Some("2.50.1"), Some("/usr/bin/git"), false))),
            "Runs git 2.50.1 at /usr/bin/git"
        );
        assert_eq!(
            git_line(Some(&git(Some("2.56.0"), Some("/x/git"), true))),
            "Runs git 2.56.0, the one shipped with it"
        );
        assert_eq!(
            git_line(Some(&git(Some("2.56.0"), None, false))),
            "Runs git 2.56.0"
        );
        assert_eq!(git_line(Some(&git(None, None, false))), "No git can be run");
    }

    #[test]
    fn the_details_name_the_application_its_git_and_the_system() {
        let said = details(Some(&git(Some("2.50.1"), Some("/usr/bin/git"), false)));
        let lines: Vec<&str> = said.lines().collect();
        assert_eq!(lines.len(), 3, "{said}");
        assert_eq!(
            lines.first().copied(),
            Some(format!("BitBraid {}", env!("CARGO_PKG_VERSION")).as_str())
        );
        assert_eq!(
            lines.get(1).copied(),
            Some("Runs git 2.50.1 at /usr/bin/git")
        );
        assert!(
            lines
                .get(2)
                .is_some_and(|system| system.contains(std::env::consts::ARCH)),
            "{said}"
        );
    }
}
