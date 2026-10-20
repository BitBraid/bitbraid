//! What the inbox works out from what `inbox.items` answered, with
//! nothing drawn: which rows a tab and the choices above it keep, the
//! groups of the pull requests in their order, the choices the lists to
//! choose from offer, the status bar's summary, how a row's age and size
//! are written, and what a row's button does first. Every text here came
//! from another machine and is only compared and shown.
//!
//! Sources: none (original design).

use bitbraid_integrations::{
    inbox::{Inbox, InboxIssue, InboxPull, Involvement, PullGroup, Snoozed, WorkInProgress},
    model::ProviderKind,
};

/// The tabs, in their order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    /// The pull requests, in their groups.
    Pulls,
    Issues,
    /// The repositories with uncommitted work.
    Work,
    /// Pull requests and issues together.
    All,
    /// What was snoozed.
    Snoozed,
}

impl Tab {
    /// Every tab, in its order.
    pub const ALL: [Tab; 5] = [Tab::Pulls, Tab::Issues, Tab::Work, Tab::All, Tab::Snoozed];

    /// What it is known by in element ids and tests.
    pub fn key(self) -> &'static str {
        match self {
            Tab::Pulls => "pulls",
            Tab::Issues => "issues",
            Tab::Work => "work",
            Tab::All => "all",
            Tab::Snoozed => "snoozed",
        }
    }

    /// What it says.
    pub fn title(self) -> &'static str {
        match self {
            Tab::Pulls => say!("inbox-tab-pulls"),
            Tab::Issues => say!("inbox-tab-issues"),
            Tab::Work => say!("inbox-tab-work"),
            Tab::All => say!("inbox-tab-all"),
            Tab::Snoozed => say!("inbox-tab-snoozed"),
        }
    }
}

/// Whose rows the list of the tab keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Whose {
    /// Every one that is the user's business.
    Mine,
    /// Opened by the user.
    Created,
    /// Assigned to the user.
    Assigned,
    /// Waiting for the user's review (pull requests).
    ReviewRequested,
    /// Naming the user (issues).
    Mentioned,
}

impl Whose {
    /// The choices for pull requests, and for issues.
    pub const FOR_PULLS: [Whose; 4] = [
        Whose::Mine,
        Whose::Created,
        Whose::Assigned,
        Whose::ReviewRequested,
    ];
    pub const FOR_ISSUES: [Whose; 4] = [
        Whose::Mine,
        Whose::Assigned,
        Whose::Created,
        Whose::Mentioned,
    ];

    /// What it is known by in element ids and tests.
    pub fn key(self) -> &'static str {
        match self {
            Whose::Mine => "mine",
            Whose::Created => "created",
            Whose::Assigned => "assigned",
            Whose::ReviewRequested => "review_requested",
            Whose::Mentioned => "mentioned",
        }
    }

    /// What it says.
    pub fn title(self) -> &'static str {
        match self {
            Whose::Mine => say!("inbox-whose-mine"),
            Whose::Created => say!("inbox-whose-created"),
            Whose::Assigned => say!("inbox-whose-assigned"),
            Whose::ReviewRequested => say!("inbox-whose-review-requested"),
            Whose::Mentioned => say!("inbox-whose-mentioned"),
        }
    }

    fn keeps(self, involvement: &[Involvement]) -> bool {
        let wanted = match self {
            Whose::Mine => return true,
            Whose::Created => Involvement::Created,
            Whose::Assigned => Involvement::Assigned,
            Whose::ReviewRequested => Involvement::ReviewRequested,
            Whose::Mentioned => Involvement::Mentioned,
        };
        involvement.contains(&wanted)
    }
}

/// What the choices above the list say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choices {
    /// Only this account's rows.
    pub account: Option<String>,
    pub whose: Whose,
    /// Only rows with this label.
    pub label: Option<String>,
    /// Only rows of this milestone (by title).
    pub milestone: Option<String>,
    /// What is typed in the search, words that all have to be found.
    pub search: String,
}

impl Default for Choices {
    fn default() -> Self {
        Self {
            account: None,
            whose: Whose::Mine,
            label: None,
            milestone: None,
            search: String::new(),
        }
    }
}

/// Whether every word typed is in one of `texts`, or is the row's number
/// (`12`, `#12`, `!12`); upper and lower case are the same.
fn found(search: &str, number: u64, texts: &[&str]) -> bool {
    let number = number.to_string();
    let texts: Vec<String> = texts.iter().map(|text| text.to_lowercase()).collect();
    search.split_whitespace().all(|word| {
        let word = word.to_lowercase();
        let digits = word.trim_start_matches(['#', '!']);
        (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && digits == number)
            || texts.iter().any(|text| text.contains(&word))
    })
}

/// Whether a pull request is kept by the choices.
pub fn keeps_pull(pull: &InboxPull, choices: &Choices) -> bool {
    let row = &pull.pull;
    choices
        .account
        .as_ref()
        .is_none_or(|account| *account == pull.account)
        && choices.whose.keeps(&pull.involvement)
        && choices
            .label
            .as_ref()
            .is_none_or(|label| row.labels.iter().any(|known| known.name == *label))
        && choices.milestone.as_ref().is_none_or(|milestone| {
            pull.milestone
                .as_ref()
                .is_some_and(|known| known.title == *milestone)
        })
        && found(
            &choices.search,
            row.number,
            &[
                &row.title,
                &row.repo,
                &row.head_ref,
                &row.author.login,
                row.author.name.as_deref().unwrap_or(""),
            ],
        )
}

/// Whether an issue is kept by the choices.
pub fn keeps_issue(issue: &InboxIssue, choices: &Choices) -> bool {
    let row = &issue.issue;
    choices
        .account
        .as_ref()
        .is_none_or(|account| *account == issue.account)
        && choices.whose.keeps(&issue.involvement)
        && choices
            .label
            .as_ref()
            .is_none_or(|label| row.labels.iter().any(|known| known.name == *label))
        && choices.milestone.as_ref().is_none_or(|milestone| {
            row.milestone
                .as_ref()
                .is_some_and(|known| known.title == *milestone)
        })
        && found(
            &choices.search,
            row.number,
            &[
                &row.title,
                &row.repo,
                &row.author.login,
                row.author.name.as_deref().unwrap_or(""),
            ],
        )
}

/// Whether a row is snoozed at `now`.
pub fn snoozed(snoozed: Option<&Snoozed>, now: i64) -> bool {
    snoozed.is_some_and(|snoozed| bitbraid_integrations::inbox::still_snoozed(snoozed, now))
}

/// The pull requests of the pull requests' tab, in their groups: every
/// group with something in it, in the tab's order, each row in every
/// group it falls in, the pinned ones first. Snoozed ones are left out.
pub fn grouped<'a>(
    answer: &'a Inbox,
    choices: &Choices,
    now: i64,
) -> Vec<(PullGroup, Vec<&'a InboxPull>)> {
    PullGroup::ALL
        .into_iter()
        .filter_map(|group| {
            let mut rows: Vec<&InboxPull> = answer
                .pulls
                .iter()
                .filter(|pull| pull.groups.contains(&group))
                .filter(|pull| !snoozed(pull.snoozed.as_ref(), now))
                .filter(|pull| keeps_pull(pull, choices))
                .collect();
            // Stable: within the pinned and the others, the answer's
            // order, the most recently changed first.
            rows.sort_by_key(|pull| !pull.pinned);
            (!rows.is_empty()).then_some((group, rows))
        })
        .collect()
}

/// A row of a flat list: a pull request or an issue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row<'a> {
    Pull(&'a InboxPull),
    Issue(&'a InboxIssue),
}

impl Row<'_> {
    /// When it last changed, as the provider wrote it.
    fn updated(&self) -> &str {
        match self {
            Row::Pull(pull) => &pull.pull.updated,
            Row::Issue(issue) => &issue.issue.updated,
        }
    }

    fn pinned(&self) -> bool {
        match self {
            Row::Pull(pull) => pull.pinned,
            Row::Issue(issue) => issue.pinned,
        }
    }

    pub fn key(&self) -> &str {
        match self {
            Row::Pull(pull) => &pull.key,
            Row::Issue(issue) => &issue.key,
        }
    }
}

/// The rows of a flat tab: the issues, everything together, or what is
/// snoozed; pinned first, then the most recently changed.
pub fn flat<'a>(answer: &'a Inbox, tab: Tab, choices: &Choices, now: i64) -> Vec<Row<'a>> {
    let wanted = |snoozed_now: bool| match tab {
        Tab::Snoozed => snoozed_now,
        _ => !snoozed_now,
    };
    let mut rows: Vec<Row<'a>> = Vec::new();
    if matches!(tab, Tab::All | Tab::Snoozed) {
        rows.extend(
            answer
                .pulls
                .iter()
                .filter(|pull| wanted(snoozed(pull.snoozed.as_ref(), now)))
                .filter(|pull| keeps_pull(pull, choices))
                .map(Row::Pull),
        );
    }
    if matches!(tab, Tab::Issues | Tab::All | Tab::Snoozed) {
        rows.extend(
            answer
                .issues
                .iter()
                .filter(|issue| wanted(snoozed(issue.snoozed.as_ref(), now)))
                .filter(|issue| keeps_issue(issue, choices))
                .map(Row::Issue),
        );
    }
    rows.sort_by(|a, b| {
        (b.pinned(), b.updated())
            .cmp(&(a.pinned(), a.updated()))
            .then_with(|| a.key().cmp(b.key()))
    });
    rows
}

/// The repositories with work in progress that the search keeps, by
/// their name, path or branch.
pub fn work<'a>(answer: &'a Inbox, choices: &Choices) -> Vec<&'a WorkInProgress> {
    answer
        .work
        .iter()
        .filter(|wip| {
            found(
                &choices.search,
                0,
                &[&wip.name, &wip.path, wip.branch.as_deref().unwrap_or("")],
            )
        })
        .collect()
}

/// How many rows each tab holds, as its count says.
pub fn counts(answer: &Inbox, choices: &Choices, now: i64) -> Vec<(Tab, usize)> {
    Tab::ALL
        .into_iter()
        .map(|tab| {
            let count = match tab {
                Tab::Pulls => answer
                    .pulls
                    .iter()
                    .filter(|pull| !snoozed(pull.snoozed.as_ref(), now))
                    .filter(|pull| keeps_pull(pull, choices))
                    .count(),
                Tab::Work => work(answer, choices).len(),
                other => flat(answer, other, choices, now).len(),
            };
            (tab, count)
        })
        .collect()
}

/// What a group's heading says.
pub fn group_title(group: PullGroup) -> &'static str {
    match group {
        PullGroup::ReadyToMerge => say!("inbox-group-ready-to-merge"),
        PullGroup::ReviewMine => say!("inbox-group-review-mine"),
        PullGroup::NoReviewers => say!("inbox-group-no-reviewers"),
        PullGroup::FailingChecks => say!("inbox-group-failing-checks"),
        PullGroup::Conflicts => say!("inbox-group-conflicts"),
        PullGroup::ChangesRequested => say!("inbox-group-changes-requested"),
        PullGroup::Commented => say!("inbox-group-commented"),
        PullGroup::WaitingForReview => say!("inbox-group-waiting-for-review"),
        PullGroup::Draft => say!("inbox-group-draft"),
        PullGroup::Other => say!("inbox-group-other"),
    }
}

/// What a group says of one pull request, after its name in the status
/// bar.
fn group_sentence(group: PullGroup) -> &'static str {
    match group {
        PullGroup::ReadyToMerge => say!("inbox-says-ready-to-merge"),
        PullGroup::ReviewMine => say!("inbox-says-review-mine"),
        PullGroup::NoReviewers => say!("inbox-says-no-reviewers"),
        PullGroup::FailingChecks => say!("inbox-says-failing-checks"),
        PullGroup::Conflicts => say!("inbox-says-conflicts"),
        PullGroup::ChangesRequested => say!("inbox-says-changes-requested"),
        PullGroup::Commented => say!("inbox-says-commented"),
        PullGroup::WaitingForReview => say!("inbox-says-waiting-for-review"),
        PullGroup::Draft => say!("inbox-says-draft"),
        PullGroup::Other => say!("inbox-says-other"),
    }
}

/// A pull request as the status bar names it: `owner/name#12`, `!12` at
/// GitLab.
pub fn short_name(pull: &InboxPull) -> String {
    let mark = match pull.kind {
        ProviderKind::Github => '#',
        ProviderKind::Gitlab => '!',
    };
    format!(
        "{}{mark}{}",
        crate::views::pulls::facts::shown_line(&pull.pull.repo, 80),
        pull.pull.number
    )
}

/// One line of the status bar's summary: a group, how many it holds, and
/// the first of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Urgent {
    pub group: PullGroup,
    /// How many pull requests it holds.
    pub count: usize,
    /// The first of them, as the bar names it.
    pub first: String,
}

/// The groups the status bar sums up that hold something, in their order;
/// snoozed rows are left out.
pub fn urgent(answer: &Inbox, now: i64) -> Vec<Urgent> {
    let all = Choices::default();
    grouped(answer, &all, now)
        .into_iter()
        .filter(|(group, _)| PullGroup::URGENT.contains(group))
        .filter_map(|(group, rows)| {
            Some(Urgent {
                group,
                count: rows.len(),
                first: short_name(rows.first()?),
            })
        })
        .collect()
}

/// What the status bar says: the first urgent group's first pull request
/// and what it waits for, and how many more there are in it.
pub fn status_line(urgent: &[Urgent]) -> Option<String> {
    let first = urgent.first()?;
    Some(say!(
        "inbox-status",
        first = first.first.as_str(),
        sentence = group_sentence(first.group),
        more = first.count.saturating_sub(1)
    ))
}

/// What a summary line of the status bar's panel says of its group.
pub fn urgent_line(urgent: &Urgent) -> String {
    let count = urgent.count;
    match urgent.group {
        PullGroup::ReadyToMerge => say!("inbox-urgent-ready-to-merge", count),
        PullGroup::ReviewMine => say!("inbox-urgent-review-mine", count),
        PullGroup::Conflicts => say!("inbox-urgent-conflicts", count),
        PullGroup::FailingChecks => say!("inbox-urgent-failing-checks", count),
        PullGroup::ChangesRequested => say!("inbox-urgent-changes-requested", count),
        _ => say!("inbox-urgent-other", count),
    }
}

/// How long ago a time was, in the fewest letters: `now`, `5m`, `3h`,
/// `2d`, `3w`, `4mo`, `1y`.
pub fn short_age(seconds: i64, now: i64) -> String {
    let delta = now.saturating_sub(seconds).max(0);
    match delta {
        0..60 => say!("inbox-age-now").to_owned(),
        60..3600 => say!("inbox-age-minutes", count = delta / 60),
        3600..86_400 => say!("inbox-age-hours", count = delta / 3600),
        86_400..604_800 => say!("inbox-age-days", count = delta / 86_400),
        604_800..2_629_746 => say!("inbox-age-weeks", count = delta / 604_800),
        2_629_746..31_556_952 => say!("inbox-age-months", count = delta / 2_629_746),
        _ => say!("inbox-age-years", count = delta / 31_556_952),
    }
}

/// How many of the four small blocks after the lines a pull request
/// changes fill: one for under ten lines, four for five hundred or more.
pub fn size_blocks(additions: Option<u64>, deletions: Option<u64>) -> Option<usize> {
    let lines = additions?.saturating_add(deletions.unwrap_or(0));
    Some(match lines {
        0..=9 => 1,
        10..=99 => 2,
        100..=499 => 3,
        _ => 4,
    })
}

/// When a snooze ends, as a row of the snoozed tab says it: a time of
/// day today, a day and a time later, or never; `offset` is how far east
/// of UTC the local clock is, in seconds.
pub fn snooze_end(snoozed: &Snoozed, now: i64, offset: i32) -> String {
    let Some(until) = snoozed.until else {
        return say!("inbox-snooze-ends-never").to_owned();
    };
    let local = |at: i64| at.saturating_add(i64::from(offset));
    let day = |at: i64| local(at).div_euclid(86_400);
    let clock = |at: i64| {
        let seconds = local(at).rem_euclid(86_400);
        format!("{:02}:{:02}", seconds / 3600, seconds % 3600 / 60)
    };
    if day(until) == day(now) {
        say!("inbox-snooze-ends-today", time = clock(until))
    } else if day(until) == day(now).saturating_add(1) {
        say!("inbox-snooze-ends-tomorrow", time = clock(until))
    } else {
        say!(
            "inbox-snooze-ends-on",
            date = crate::views::absolute_date(until, offset),
            time = clock(until)
        )
    }
}

/// What a pull request's button does first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstAction {
    /// Merge it, after asking how.
    Merge,
    /// Show it in its repository's tab, to review it.
    Review,
    /// Open its repository in a tab.
    OpenRepository,
    /// Open it at the provider.
    OpenAtProvider,
}

impl FirstAction {
    /// What the button says; `provider` is GitHub or GitLab.
    pub fn label(self, provider: &str) -> String {
        match self {
            FirstAction::Merge => say!("inbox-action-merge").to_owned(),
            FirstAction::Review => say!("inbox-action-review").to_owned(),
            FirstAction::OpenRepository => say!("inbox-action-open-repository").to_owned(),
            FirstAction::OpenAtProvider => say!("inbox-action-open-at", provider),
        }
    }
}

/// What a pull request's button does first in a group (or in a flat
/// list, when `group` is none): merge it when it can be, review it when
/// that is asked of the user, else open its repository when it is here,
/// else its page at the provider.
pub fn first_action(pull: &InboxPull, group: Option<PullGroup>) -> FirstAction {
    let in_group = |wanted: PullGroup| match group {
        Some(group) => group == wanted,
        None => pull.groups.contains(&wanted),
    };
    if in_group(PullGroup::ReadyToMerge) {
        FirstAction::Merge
    } else if in_group(PullGroup::ReviewMine) && pull.path.is_some() {
        FirstAction::Review
    } else if pull.path.is_some() {
        FirstAction::OpenRepository
    } else {
        FirstAction::OpenAtProvider
    }
}

/// The names of the labels the rows have, each once, in order: what the
/// list of labels offers.
pub fn labels(answer: &Inbox) -> Vec<String> {
    let mut out: Vec<String> = answer
        .pulls
        .iter()
        .flat_map(|pull| pull.pull.labels.iter())
        .chain(
            answer
                .issues
                .iter()
                .flat_map(|issue| issue.issue.labels.iter()),
        )
        .map(|label| crate::views::pulls::facts::shown_line(&label.name, 60))
        .collect();
    out.sort_by_key(|name| name.to_lowercase());
    out.dedup();
    out.truncate(200);
    out
}

/// The titles of the milestones the rows have, each once, in order.
pub fn milestones(answer: &Inbox) -> Vec<String> {
    let mut out: Vec<String> = answer
        .pulls
        .iter()
        .filter_map(|pull| pull.milestone.as_ref())
        .chain(
            answer
                .issues
                .iter()
                .filter_map(|issue| issue.issue.milestone.as_ref()),
        )
        .map(|milestone| crate::views::pulls::facts::shown_line(&milestone.title, 60))
        .collect();
    out.sort_by_key(|title| title.to_lowercase());
    out.dedup();
    out.truncate(200);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitbraid_integrations::{
        inbox::Approval,
        model::{
            CiState, Issue, IssueState, Label, Mergeable, Milestone, PullRequest, PullState, User,
        },
    };

    fn user(login: &str) -> User {
        User {
            login: login.into(),
            ..User::default()
        }
    }

    fn pull(number: u64, groups: &[PullGroup], updated: &str) -> InboxPull {
        InboxPull {
            key: format!("a/pull/o/r#{number}"),
            account: "a".into(),
            kind: ProviderKind::Github,
            pull: PullRequest {
                repo: "o/r".into(),
                number,
                title: format!("Title {number}"),
                state: PullState::Open,
                draft: false,
                author: user("me"),
                head_ref: format!("topic-{number}"),
                head_repo: None,
                head_sha: String::new(),
                base_ref: "main".into(),
                created: String::new(),
                updated: updated.into(),
                labels: vec![Label {
                    name: "bug".into(),
                    color: None,
                    description: None,
                }],
                reviewers: Vec::new(),
                assignees: Vec::new(),
                ci: CiState::None,
                comments: None,
                web_url: String::new(),
            },
            additions: Some(3),
            deletions: Some(1),
            mergeable: Mergeable::Clean,
            approval: Approval::Unknown,
            milestone: None,
            involvement: vec![Involvement::Created],
            groups: groups.to_vec(),
            path: None,
            remote: None,
            pinned: false,
            snoozed: None,
        }
    }

    fn issue(number: u64, updated: &str) -> InboxIssue {
        InboxIssue {
            key: format!("a/issue/o/r#{number}"),
            account: "b".into(),
            kind: ProviderKind::Gitlab,
            issue: Issue {
                repo: "g/p".into(),
                number,
                title: format!("Issue {number}"),
                state: IssueState::Open,
                author: user("ann"),
                assignees: Vec::new(),
                labels: Vec::new(),
                milestone: Some(Milestone {
                    id: "1".into(),
                    title: "v2".into(),
                    due: None,
                    open: true,
                }),
                created: String::new(),
                updated: updated.into(),
                comments: 0,
                web_url: String::new(),
            },
            reactions: None,
            involvement: vec![Involvement::Mentioned],
            path: None,
            remote: None,
            pinned: false,
            snoozed: None,
        }
    }

    fn answer() -> Inbox {
        let mut pinned = pull(
            3,
            &[PullGroup::ReadyToMerge, PullGroup::Draft],
            "2026-10-01",
        );
        pinned.pinned = true;
        let mut asleep = pull(4, &[PullGroup::ReadyToMerge], "2026-10-09");
        asleep.snoozed = Some(Snoozed {
            since: 0,
            until: Some(200),
        });
        Inbox {
            pulls: vec![
                pull(
                    1,
                    &[PullGroup::ReadyToMerge, PullGroup::NoReviewers],
                    "2026-10-05",
                ),
                pull(2, &[PullGroup::Conflicts], "2026-10-04"),
                pinned,
                asleep,
            ],
            issues: vec![issue(7, "2026-10-06")],
            ..Inbox::default()
        }
    }

    #[test]
    fn pull_requests_are_in_every_group_they_fall_in_the_pinned_first() {
        let answer = answer();
        let groups: Vec<(PullGroup, Vec<u64>)> = grouped(&answer, &Choices::default(), 100)
            .into_iter()
            .map(|(group, rows)| (group, rows.iter().map(|row| row.pull.number).collect()))
            .collect();
        assert_eq!(
            groups,
            [
                (PullGroup::ReadyToMerge, vec![3, 1]),
                (PullGroup::NoReviewers, vec![1]),
                (PullGroup::Conflicts, vec![2]),
                (PullGroup::Draft, vec![3]),
            ],
            "the snoozed one is out until its time"
        );
        // Its time up, it is back.
        let later = grouped(&answer, &Choices::default(), 200);
        assert_eq!(later[0].1.len(), 3);
    }

    #[test]
    fn the_flat_tabs_hold_their_rows_pinned_first_then_the_latest() {
        let answer = answer();
        let keys = |tab: Tab| -> Vec<String> {
            flat(&answer, tab, &Choices::default(), 100)
                .iter()
                .map(|row| row.key().to_owned())
                .collect()
        };
        assert_eq!(keys(Tab::Issues), ["a/issue/o/r#7"]);
        assert_eq!(
            keys(Tab::All),
            [
                "a/pull/o/r#3",
                "a/issue/o/r#7",
                "a/pull/o/r#1",
                "a/pull/o/r#2"
            ]
        );
        assert_eq!(keys(Tab::Snoozed), ["a/pull/o/r#4"]);
        let counted = counts(&answer, &Choices::default(), 100);
        assert_eq!(
            counted,
            [
                (Tab::Pulls, 3),
                (Tab::Issues, 1),
                (Tab::Work, 0),
                (Tab::All, 4),
                (Tab::Snoozed, 1)
            ]
        );
    }

    #[test]
    fn the_choices_keep_what_they_name_and_the_search_takes_numbers() {
        let answer = answer();
        let kept = |choices: Choices| -> Vec<String> {
            flat(&answer, Tab::All, &choices, 100)
                .iter()
                .map(|row| row.key().to_owned())
                .collect()
        };
        let choose = |change: &dyn Fn(&mut Choices)| {
            let mut choices = Choices::default();
            change(&mut choices);
            choices
        };
        assert_eq!(
            kept(choose(&|c| c.account = Some("b".into()))),
            ["a/issue/o/r#7"]
        );
        assert_eq!(
            kept(choose(&|c| c.whose = Whose::Mentioned)),
            ["a/issue/o/r#7"]
        );
        assert_eq!(kept(choose(&|c| c.whose = Whose::Assigned)).len(), 0);
        assert_eq!(kept(choose(&|c| c.label = Some("bug".into()))).len(), 3);
        assert_eq!(
            kept(choose(&|c| c.milestone = Some("v2".into()))),
            ["a/issue/o/r#7"]
        );
        for search in ["#2", "2", "TITLE 2", "topic-2"] {
            assert_eq!(
                kept(choose(&|c| c.search = search.into())),
                ["a/pull/o/r#2"],
                "{search}"
            );
        }
        assert_eq!(kept(choose(&|c| c.search = "o/r title".into())).len(), 3);
        assert_eq!(kept(choose(&|c| c.search = "nothing".into())).len(), 0);
        assert_eq!(labels(&answer), ["bug"]);
        assert_eq!(milestones(&answer), ["v2"]);
    }

    #[test]
    fn the_status_bar_sums_up_the_urgent_groups() {
        let answer = answer();
        let urgent = urgent(&answer, 100);
        assert_eq!(
            urgent,
            [
                Urgent {
                    group: PullGroup::ReadyToMerge,
                    count: 2,
                    first: "o/r#3".into()
                },
                Urgent {
                    group: PullGroup::Conflicts,
                    count: 1,
                    first: "o/r#2".into()
                },
            ]
        );
        assert_eq!(
            status_line(&urgent).as_deref(),
            Some("o/r#3 can be merged, and 1 more")
        );
        assert_eq!(urgent_line(&urgent[0]), "2 pull requests can be merged");
        assert_eq!(urgent_line(&urgent[1]), "1 pull request has conflicts");
        assert_eq!(status_line(&[]), None);
    }

    #[test]
    fn ages_sizes_and_the_ends_of_snoozes_are_written_short() {
        let now = 1_000_000_000;
        assert_eq!(short_age(now, now), "now");
        assert_eq!(short_age(now - 300, now), "5m");
        assert_eq!(short_age(now - 5 * 3600, now), "5h");
        assert_eq!(short_age(now - 3 * 86_400, now), "3d");
        assert_eq!(short_age(now - 15 * 86_400, now), "2w");
        assert_eq!(short_age(now - 100 * 86_400, now), "3mo");
        assert_eq!(short_age(now - 800 * 86_400, now), "2y");
        assert_eq!(short_age(now + 50, now), "now");
        assert_eq!(short_age(i64::MIN, i64::MAX), "292277024626y");
        assert_eq!(size_blocks(Some(3), Some(1)), Some(1));
        assert_eq!(size_blocks(Some(30), None), Some(2));
        assert_eq!(size_blocks(Some(u64::MAX), Some(u64::MAX)), Some(4));
        assert_eq!(size_blocks(None, Some(4)), None);
        // 2001-09-09T01:46:40Z.
        let at = |until| Snoozed {
            since: 0,
            until: Some(until),
        };
        assert_eq!(snooze_end(&at(now + 3600), now, 0), "until 02:46");
        assert_eq!(
            snooze_end(&at(now + 86_400), now, 0),
            "until tomorrow 01:46"
        );
        assert_eq!(snooze_end(&at(now + 3600), now, 7200), "until 04:46");
        assert!(snooze_end(&at(now + 9 * 86_400), now, 0).starts_with("until 2001-09-18"));
        let ever = Snoozed {
            since: 0,
            until: None,
        };
        assert_eq!(snooze_end(&ever, now, 0), "until unsnoozed");
        let _ = snooze_end(&at(i64::MAX), i64::MIN, i32::MAX);
    }

    #[test]
    fn a_rows_button_merges_reviews_or_opens() {
        let mut ready = pull(1, &[PullGroup::ReadyToMerge, PullGroup::ReviewMine], "");
        assert_eq!(first_action(&ready, None), FirstAction::Merge);
        assert_eq!(
            first_action(&ready, Some(PullGroup::ReviewMine)),
            FirstAction::OpenAtProvider
        );
        ready.path = Some("/r".into());
        assert_eq!(
            first_action(&ready, Some(PullGroup::ReviewMine)),
            FirstAction::Review
        );
        assert_eq!(
            first_action(&ready, Some(PullGroup::Conflicts)),
            FirstAction::OpenRepository
        );
        assert_eq!(
            FirstAction::OpenAtProvider.label("GitLab"),
            "Open at GitLab"
        );
    }
}
