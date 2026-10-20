//! What the window has on show, as plain facts.
//!
//! The off-screen tests (`tests/offscreen.rs`) open the shell without a
//! display, press keys and click, and then have to say what came of it
//! before a picture is taken or an expectation is checked. They are a
//! program of their own and see nothing that is private here, so what
//! they ask about is gathered in one place. Nothing here changes
//! anything.

use std::path::PathBuf;

use gpui_kit::{App, Entity};

use super::{Shell, TabKind, WindowSheet};
use crate::views::workspace::Workspace;

/// One tab.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TabProbe {
    /// What the tab is called.
    pub title: String,
    /// The repository it shows; none for a start page.
    pub path: Option<PathBuf>,
}

/// What the window has on show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellProbe {
    /// The tabs, in order.
    pub tabs: Vec<TabProbe>,
    /// Which of them is shown.
    pub active: usize,
    /// The view of the window's own that covers the tab: `preferences`,
    /// `shortcuts`, `keys`, `about`, `notifications`, `repositories` or
    /// `inbox`.
    pub sheet: Option<&'static str>,
    /// Whether the palette is open.
    pub palette_open: bool,
    /// What the bar in the toolbar's place asks, and what its fields hold
    /// (`crate::ask`); none while it asks nothing.
    pub bar: Option<(String, Vec<String>)>,
    /// What the toolbar's middle group can do now.
    pub toolbar: super::ToolbarState,
    /// The command whose parameters are being asked for, by id.
    pub asking_for: Option<&'static str>,
    /// The question of git's or ssh's that is on screen, as it was
    /// worded, and how many wait behind it.
    pub asking: Option<(String, usize)>,
    /// Whether a clone or a new repository is being made.
    pub creating: bool,
    /// What the rows of the window's own open form hold (what to clone
    /// and where, where a new repository goes); empty when none is open.
    pub form_rows: Vec<String>,
    /// Whether that form would do as it stands: its button is greyed
    /// while it would not.
    pub form_ready: bool,
    /// The section of the preferences that is shown, while they are.
    pub preferences_section: Option<&'static str>,
    /// The current profile, and every profile there is.
    pub profile: Option<String>,
    pub profiles: Vec<String>,
    /// What the fields of gitflow's section hold, once it was shown.
    pub flow_fields: Vec<String>,
}

impl Shell {
    /// What the window has on show.
    pub fn probe(&self, cx: &App) -> ShellProbe {
        ShellProbe {
            tabs: self
                .tabs
                .iter()
                .map(|tab| TabProbe {
                    title: tab.title.to_string(),
                    path: match tab.kind {
                        TabKind::Repo(_) => tab.path.clone(),
                        TabKind::Start(_) => None,
                    },
                })
                .collect(),
            active: self.active,
            sheet: self.sheet.map(|sheet| match sheet {
                WindowSheet::Preferences => "preferences",
                WindowSheet::Shortcuts => "shortcuts",
                WindowSheet::KeyList => "keys",
                WindowSheet::About => "about",
                WindowSheet::Notices => "notifications",
                WindowSheet::Repositories => "repositories",
                WindowSheet::Inbox => "inbox",
            }),
            palette_open: self.palette_open,
            bar: crate::ask::on_show(cx),
            toolbar: self.toolbar_state(cx),
            asking_for: self.param_prompt.as_ref().map(|prompt| prompt.id),
            asking: self
                .asking
                .as_ref()
                .map(|question| (question.prompt.clone(), self.ask_queue.len())),
            creating: self.creating.is_some(),
            form_rows: crate::views::workspace::form::row_texts(self, cx),
            form_ready: crate::views::workspace::form::sendable(self, cx),
            preferences_section: (self.sheet == Some(WindowSheet::Preferences))
                .then(|| self.preferences.read(cx).section().key()),
            profile: self.profile_now.clone(),
            profiles: self.profiles.clone(),
            flow_fields: self.preferences.read(cx).flow_values(cx),
        }
    }

    /// The preferences, for what a test does in one of their sections.
    pub fn preferences_view(&self) -> Entity<crate::views::preferences::PreferencesView> {
        self.preferences.clone()
    }

    /// The workspace of the tab that is shown, when it is a repository's.
    pub fn shown_workspace(&self) -> Option<Entity<Workspace>> {
        self.active_workspace().cloned()
    }

    /// What the terminal of the shown tab has on show.
    pub fn terminal_probe(
        &self,
        window: &gpui_kit::Window,
        cx: &App,
    ) -> crate::views::terminal::probe::TerminalProbe {
        crate::views::terminal::probe::probe(
            &self.terminals,
            self.active_path().as_deref(),
            window,
            cx,
        )
    }
}
