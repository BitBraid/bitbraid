//! What the inbox has on show, as plain facts, for the off-screen tests:
//! the tab, the counts, the groups with their rows' keys, the rows of a
//! flat tab, the choices, what is being merged. Only `choose` changes
//! anything.

use bitbraid_integrations::inbox::PullGroup;
use gpui_kit::App;

use super::{InboxView, Tab, facts};

/// What the inbox has on show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxProbe {
    /// The tab on show.
    pub tab: Tab,
    /// How many rows each tab holds.
    pub counts: Vec<(Tab, usize)>,
    /// The groups of the pull requests' tab: each with whether it is
    /// open and the keys of its rows.
    pub groups: Vec<(PullGroup, bool, Vec<String>)>,
    /// The keys of the rows of a flat tab, in their order.
    pub rows: Vec<String>,
    /// The repositories with work in progress.
    pub work: Vec<String>,
    /// The keys of the pinned rows, and of the snoozed ones.
    pub pinned: Vec<String>,
    pub snoozed: Vec<String>,
    /// The choices above the list.
    pub choices: facts::Choices,
    /// The workspace chosen, and those that can be chosen (id, name).
    pub workspace: Option<String>,
    pub workspaces: Vec<(String, String)>,
    /// Whether it is being read, and why it could not be.
    pub reading: bool,
    pub error: Option<String>,
    /// The pull request whose merge is asked about.
    pub merging: Option<String>,
    /// What the status bar says.
    pub status: Option<String>,
}

impl InboxView {
    /// What the inbox has on show.
    pub fn probe(&self, _cx: &App) -> InboxProbe {
        let now = crate::views::now();
        let answer = self.answer.clone().unwrap_or_default();
        let groups = facts::grouped(&answer, &self.choices, now)
            .into_iter()
            .map(|(group, rows)| {
                (
                    group,
                    !self.folded.contains(&group),
                    rows.iter().map(|pull| pull.key.clone()).collect(),
                )
            })
            .collect();
        let rows = match self.tab {
            Tab::Pulls | Tab::Work => Vec::new(),
            tab => facts::flat(&answer, tab, &self.choices, now)
                .iter()
                .map(|row| row.key().to_owned())
                .collect(),
        };
        let marked = |pinned: bool| {
            answer
                .pulls
                .iter()
                .map(|pull| (&pull.key, pull.pinned, pull.snoozed.as_ref()))
                .chain(
                    answer
                        .issues
                        .iter()
                        .map(|issue| (&issue.key, issue.pinned, issue.snoozed.as_ref())),
                )
                .filter(|(_, pin, snoozed)| {
                    if pinned {
                        *pin
                    } else {
                        facts::snoozed(*snoozed, now)
                    }
                })
                .map(|(key, _, _)| key.clone())
                .collect::<Vec<_>>()
        };
        InboxProbe {
            tab: self.tab,
            counts: facts::counts(&answer, &self.choices, now),
            groups,
            rows,
            work: facts::work(&answer, &self.choices)
                .iter()
                .map(|wip| wip.path.clone())
                .collect(),
            pinned: marked(true),
            snoozed: marked(false),
            choices: self.choices.clone(),
            workspace: self.workspace.clone(),
            workspaces: self.workspaces.clone(),
            reading: self.reading,
            error: self.error.clone(),
            merging: self.merging.as_ref().map(|merging| merging.key.clone()),
            status: facts::status_line(&facts::urgent(&answer, now)),
        }
    }

    /// Change the choices above the list, as their menus do.
    pub fn choose(
        &mut self,
        change: impl FnOnce(&mut facts::Choices),
        cx: &mut gpui_kit::Context<Self>,
    ) {
        change(&mut self.choices);
        cx.notify();
    }
}
