//! The checks of commits, for the rows of the graph: what is known of
//! each commit, which commits are to be asked about, and the mark a row
//! carries with the checks in its hover. Also the marks of a check's
//! state that the pull requests' rows, card and view use.
//!
//! The graph asks for the commits of the rows in view as they are drawn,
//! a batch of at most [`BATCH`] at a time and one batch at a time, and
//! each commit at most once a [`AGAIN_AFTER`]; only while the repository
//! has a remote linked to an account ([`Checks::set_source`]). Without
//! one, nothing is asked and nothing is held, and a row costs one test of
//! an empty table.
//!
//! Sources: docs.github.com/en/graphql (statusCheckRollup);
//! docs.gitlab.com/api (pipelines, commits/statuses).

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use bitbraid_integrations::model::{Check, CiState, CommitStatus, RepoLink};
use gpui_kit::component::{ActiveTheme, Icon, IconName, h_flex, tooltip::Tooltip, v_flex};
use gpui_kit::*;

use super::facts::{age, checks_summary, ci_word, shown_line};
use crate::zoom::px;

/// The most commits asked about at once (what `ci.status` takes).
pub const BATCH: usize = 100;
/// How long before a commit is asked about again.
pub const AGAIN_AFTER: Duration = Duration::from_secs(60);
/// The most commits anything is remembered about.
const REMEMBERED: usize = 20_000;
/// The most checks a hover lists, one a line.
const MOST_LISTED: usize = 20;

/// What is known of the checks of the commits in the graph.
#[derive(Debug, Default)]
pub struct Checks {
    /// The repository at the provider they are asked of.
    source: Option<RepoLink>,
    /// The commits that have checks, by full id.
    known: HashMap<String, CommitStatus>,
    /// When each commit was last asked about.
    asked: HashMap<String, Instant>,
    /// A batch is out.
    out: bool,
    /// Which source the batch that is out was asked of: an answer for an
    /// older one is not taken.
    generation: u64,
    /// A remote of the repository is linked to an account, given lists or
    /// not: pull requests can be made from the graph.
    linked: bool,
}

/// A batch to ask about: of which source, and which commits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Batch {
    pub generation: u64,
    pub source: RepoLink,
    pub shas: Vec<String>,
}

impl Checks {
    /// Ask the repository at the provider this link names, or nothing. A
    /// change forgets what was known. Whether it changed.
    pub fn set_source(&mut self, source: Option<RepoLink>) -> bool {
        if self.source == source {
            return false;
        }
        self.source = source;
        self.known.clear();
        self.asked.clear();
        self.out = false;
        self.generation = self.generation.wrapping_add(1);
        true
    }

    /// Say whether a remote is linked to an account.
    pub fn set_linked(&mut self, linked: bool) -> bool {
        let changed = self.linked != linked;
        self.linked = linked;
        changed
    }

    /// Whether a remote is linked to an account: what makes pull requests
    /// is offered.
    pub fn linked(&self) -> bool {
        self.linked
    }

    /// Whether checks are asked for at all.
    pub fn is_on(&self) -> bool {
        self.source.is_some()
    }

    /// Whether nothing is known: a row has no mark to look for.
    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    /// Of these commits (the rows in view), the batch to ask about now:
    /// none while one is out, without a source, or when each was asked
    /// about less than [`AGAIN_AFTER`] ago.
    pub fn wanted<'a>(
        &mut self,
        shas: impl IntoIterator<Item = &'a str>,
        now: Instant,
    ) -> Option<Batch> {
        if self.out {
            return None;
        }
        let source = self.source.clone()?;
        let mut batch: Vec<String> = Vec::new();
        for sha in shas {
            if batch.len() >= BATCH {
                break;
            }
            let due = self
                .asked
                .get(sha)
                .is_none_or(|then| now.saturating_duration_since(*then) >= AGAIN_AFTER);
            if due && !batch.iter().any(|known| known == sha) {
                batch.push(sha.to_owned());
            }
        }
        if batch.is_empty() {
            return None;
        }
        if self.asked.len().saturating_add(batch.len()) > REMEMBERED {
            self.asked.clear();
        }
        for sha in &batch {
            self.asked.insert(sha.clone(), now);
        }
        self.out = true;
        Some(Batch {
            generation: self.generation,
            source,
            shas: batch,
        })
    }

    /// The answer to a batch. Whether anything shown changed.
    pub fn answered(&mut self, generation: u64, statuses: Vec<CommitStatus>) -> bool {
        if generation != self.generation {
            return false;
        }
        self.out = false;
        let mut changed = false;
        for status in statuses {
            let shown = status.state != CiState::None || !status.checks.is_empty();
            if !shown {
                changed |= self.known.remove(&status.sha).is_some();
                continue;
            }
            if self.known.len() >= REMEMBERED && !self.known.contains_key(&status.sha) {
                continue;
            }
            if self.known.get(&status.sha) != Some(&status) {
                self.known.insert(status.sha.clone(), status);
                changed = true;
            }
        }
        changed
    }

    /// A batch got no answer: the commits in it are asked about again
    /// after [`AGAIN_AFTER`], like the others.
    pub fn failed(&mut self, generation: u64) {
        if generation == self.generation {
            self.out = false;
        }
    }

    /// What is known of a commit's checks, when it has any.
    pub fn of(&self, sha: &str) -> Option<&CommitStatus> {
        self.known.get(sha)
    }

    /// Hold these as known, without asking: for the off-screen tests.
    pub fn give(&mut self, statuses: Vec<CommitStatus>) {
        for status in statuses {
            self.known.insert(status.sha.clone(), status);
        }
    }

    /// How many commits have a mark: for the off-screen tests.
    pub fn marked(&self) -> usize {
        self.known.len()
    }
}

/// The mark of a state of checks, and its colour.
pub fn ci_icon(state: CiState, cx: &App) -> (Icon, Hsla) {
    use gpui_kit::assets::IconName as Extra;
    let theme = cx.theme();
    match state {
        CiState::Success => (Icon::new(IconName::CircleCheck), theme.success),
        CiState::Failure | CiState::Cancelled => (Icon::new(IconName::CircleX), theme.danger),
        CiState::Pending | CiState::Running => (Icon::new(Extra::CircleDot), theme.warning),
        CiState::Skipped | CiState::Neutral | CiState::None => {
            (Icon::new(Extra::CircleDashed), theme.muted_foreground)
        }
    }
}

/// When a check last did something: when it ended, or began.
fn check_age(check: &Check, now: i64) -> String {
    check
        .finished
        .as_deref()
        .or(check.started.as_deref())
        .map(|time| age(time, now))
        .unwrap_or_default()
}

/// The lines of a list of checks: a mark, the name and how long ago each
/// ran, at most [`MOST_LISTED`] of them and a line for the rest.
pub fn check_lines(checks: &[Check], now: i64, cx: &App) -> Vec<AnyElement> {
    let theme = cx.theme();
    let mut lines: Vec<AnyElement> = checks
        .iter()
        .take(MOST_LISTED)
        .map(|check| {
            let (icon, colour) = ci_icon(check.state, cx);
            h_flex()
                .gap_1p5()
                .items_center()
                .text_xs()
                .child(icon.size_3p5().flex_none().text_color(colour))
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(shown_line(&check.name, 120)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_color(theme.muted_foreground)
                        .child(say!(
                            "pulls-check-state-age",
                            state = ci_word(check.state),
                            age = check_age(check, now)
                        )),
                )
                .into_any_element()
        })
        .collect();
    if checks.len() > MOST_LISTED {
        lines.push(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(say!(
                    "pulls-checks-more",
                    count = checks.len() - MOST_LISTED
                ))
                .into_any_element(),
        );
    }
    lines
}

/// What the hover of a commit's mark shows: the checks summed up, then one
/// a line.
fn commit_tip(status: &CommitStatus, now: i64, cx: &App) -> AnyElement {
    let states: Vec<CiState> = status.checks.iter().map(|check| check.state).collect();
    let summary = if states.is_empty() {
        say!("pulls-checks-word", word = ci_word(status.state))
    } else {
        checks_summary(&states)
    };
    v_flex()
        .max_w(px(420.))
        .gap_1()
        .py_1()
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(summary),
        )
        .children(check_lines(&status.checks, now, cx))
        .into_any_element()
}

/// The mark a row of the graph carries for its commit's checks, with
/// them in its hover.
pub fn commit_mark(
    id: ElementId,
    status: &CommitStatus,
    now: i64,
    cx: &App,
) -> impl IntoElement + use<> {
    let (icon, colour) = ci_icon(status.state, cx);
    let status = status.clone();
    div()
        .id(id)
        .flex_none()
        .child(icon.size_3p5().text_color(colour))
        .tooltip(move |window, cx| {
            let status = status.clone();
            Tooltip::element(move |_, cx| commit_tip(&status, now, cx)).build(window, cx)
        })
        .test_support()
}

#[cfg(test)]
mod tests {
    use bitbraid_integrations::model::ProviderKind;

    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use std::time::Instant;

    use bitbraid_integrations::model::{CiState, CommitStatus, RepoLink};

    use super::{AGAIN_AFTER, BATCH, Checks};

    fn link(name: &str) -> RepoLink {
        RepoLink {
            remote: "origin".to_owned(),
            account: "github:github.com:octo".to_owned(),
            kind: ProviderKind::Github,
            owner: "octo".to_owned(),
            name: name.to_owned(),
        }
    }

    fn status(sha: &str, state: CiState) -> CommitStatus {
        CommitStatus {
            sha: sha.to_owned(),
            state,
            checks: Vec::new(),
        }
    }

    #[test]
    fn nothing_is_asked_without_a_linked_remote() {
        let mut checks = Checks::default();
        assert!(!checks.is_on());
        assert_eq!(checks.wanted(["a", "b"], Instant::now()), None);
    }

    #[test]
    fn commits_are_asked_about_a_batch_at_a_time_and_once_a_minute() {
        let mut checks = Checks::default();
        assert!(checks.set_source(Some(link("tools"))));
        assert!(!checks.set_source(Some(link("tools"))));
        let start = Instant::now();
        let shas: Vec<String> = (0..150).map(|n| format!("{n:040}")).collect();
        let first = checks
            .wanted(shas.iter().map(String::as_str), start)
            .unwrap();
        assert_eq!(first.shas.len(), BATCH);
        // One batch at a time.
        assert_eq!(checks.wanted(shas.iter().map(String::as_str), start), None);
        assert!(checks.answered(
            first.generation,
            vec![
                status(&shas[0], CiState::Success),
                status(&shas[1], CiState::None)
            ]
        ));
        assert!(checks.of(&shas[0]).is_some());
        assert!(checks.of(&shas[1]).is_none(), "nothing to show");
        // The rest come next; the first hundred not again within a minute.
        let second = checks
            .wanted(shas.iter().map(String::as_str), start)
            .unwrap();
        assert_eq!(second.shas.len(), 50);
        checks.failed(second.generation);
        assert_eq!(checks.wanted(shas.iter().map(String::as_str), start), None);
        let later = start + AGAIN_AFTER;
        assert_eq!(
            checks
                .wanted(shas.iter().map(String::as_str), later)
                .map(|batch| batch.shas.len()),
            Some(BATCH)
        );
    }

    #[test]
    fn an_answer_for_another_repository_is_not_taken() {
        let mut checks = Checks::default();
        checks.set_source(Some(link("tools")));
        let batch = checks.wanted(["a"], Instant::now()).unwrap();
        checks.set_source(Some(link("other")));
        assert!(!checks.answered(batch.generation, vec![status("a", CiState::Failure)]));
        assert!(checks.is_empty());
        // And the new one can ask at once.
        assert!(checks.wanted(["a"], Instant::now()).is_some());
        checks.set_source(None);
        assert!(!checks.is_on());
    }
}
