//! The inbox: the pull requests and issues that wait for the user, from
//! every connected account at once, the repositories on this machine
//! with uncommitted work, and the rows the user pinned or put out of
//! sight for a while. These are the shapes `inbox.items` answers with,
//! what decides the groups a pull request is shown in, and when a snooze
//! ends. [`store`] keeps the pins and snoozes in `inbox.toml`.
//!
//! A pull request is shown in every group whose rule it meets, in this
//! order:
//!
//! - ready to merge: open, not a draft, the provider says it merges
//!   without conflicts, its reviews are settled (the provider says it is
//!   approved, or every reviewer's last word is an approval, or nobody
//!   was asked; a review still awaited, a comment or changes asked keep
//!   it out, and so does a review the provider still requires), and its
//!   checks did not fail (checks that wait for someone to let them run,
//!   as a fork's do, do not keep it out);
//! - waiting for my review: someone else's, and the user is asked to
//!   review it and has not;
//! - no reviewers: nobody was asked to review it and nobody did;
//! - failing checks: its checks summed up failed;
//! - conflicts: its changes conflict with its base;
//! - changes asked: a reviewer's last word asked for changes;
//! - commented: a reviewer's last word was a comment;
//! - waiting for review: a reviewer was asked and has not answered (the
//!   user's own pull requests and those of others the user is not asked
//!   about);
//! - draft: it is a draft;
//! - other: none of these (a pull request whose state the provider has
//!   not worked out yet).
//!
//! Sources: docs.github.com/en/graphql "Objects" (`PullRequest`:
//! `mergeable`, `reviewDecision`, `additions`, `deletions`), "Enums"
//! (`MergeableState`, `PullRequestReviewDecision`); docs.gitlab.com/api
//! "Merge requests" (`detailed_merge_status`, `has_conflicts`).

pub mod store;

use serde::{Deserialize, Serialize};

use crate::model::{
    CiState, Freshness, Issue, Mergeable, Milestone, ProviderKind, PullRequest, PullState,
    ReviewState,
};

/// Most repositories one look takes: what a workspace may hold.
pub const MAX_PATHS: usize = 1000;
/// Longest key of a row, in bytes.
pub const MAX_KEY: usize = 600;

/// Why a pull request or an issue is the user's business.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Involvement {
    /// The user opened it.
    Created,
    /// It is assigned to the user.
    Assigned,
    /// The user is asked to review it.
    ReviewRequested,
    /// It names the user, or the user took part in it otherwise.
    Mentioned,
}

/// Whether a pull request's reviews let it be merged, as the provider
/// sums them up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Approval {
    /// The provider did not say, or no review is required.
    #[default]
    Unknown,
    /// The reviews it needs were given.
    Approved,
    /// A reviewer asked for changes.
    ChangesRequested,
    /// A review is required and was not given yet.
    Required,
}

/// The groups of the pull requests' tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullGroup {
    /// It can be merged now.
    ReadyToMerge,
    /// Someone else's, waiting for the user's review.
    ReviewMine,
    /// Nobody was asked to review it.
    NoReviewers,
    /// Its checks failed.
    FailingChecks,
    /// It conflicts with its base.
    Conflicts,
    /// A reviewer asked for changes.
    ChangesRequested,
    /// A reviewer commented.
    Commented,
    /// A reviewer was asked and has not answered.
    WaitingForReview,
    Draft,
    /// None of the others.
    Other,
}

impl PullGroup {
    /// Every group, in the order the tab shows them.
    pub const ALL: [PullGroup; 10] = [
        PullGroup::ReadyToMerge,
        PullGroup::ReviewMine,
        PullGroup::NoReviewers,
        PullGroup::FailingChecks,
        PullGroup::Conflicts,
        PullGroup::ChangesRequested,
        PullGroup::Commented,
        PullGroup::WaitingForReview,
        PullGroup::Draft,
        PullGroup::Other,
    ];

    /// The groups the status bar sums up: what the user can act on now.
    pub const URGENT: [PullGroup; 5] = [
        PullGroup::ReadyToMerge,
        PullGroup::ReviewMine,
        PullGroup::Conflicts,
        PullGroup::FailingChecks,
        PullGroup::ChangesRequested,
    ];

    /// What it is called in parameters and files.
    pub fn key(self) -> &'static str {
        match self {
            PullGroup::ReadyToMerge => "ready_to_merge",
            PullGroup::ReviewMine => "review_mine",
            PullGroup::NoReviewers => "no_reviewers",
            PullGroup::FailingChecks => "failing_checks",
            PullGroup::Conflicts => "conflicts",
            PullGroup::ChangesRequested => "changes_requested",
            PullGroup::Commented => "commented",
            PullGroup::WaitingForReview => "waiting_for_review",
            PullGroup::Draft => "draft",
            PullGroup::Other => "other",
        }
    }
}

/// What a pull request was found with, besides its row of the lists.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PullFacts {
    /// Lines added, when the provider said.
    pub additions: Option<u64>,
    /// Lines removed, when the provider said.
    pub deletions: Option<u64>,
    /// Whether it merges as it stands.
    pub mergeable: Mergeable,
    /// What its reviews add up to.
    pub approval: Approval,
    pub milestone: Option<Milestone>,
}

/// The groups a pull request is shown in, for the user `me` (a login at
/// its provider): every group whose rule it meets, in the tab's order,
/// [`PullGroup::Other`] when it meets none.
pub fn groups(pull: &PullRequest, facts: &PullFacts, me: &str) -> Vec<PullGroup> {
    let mine = pull.author.login.eq_ignore_ascii_case(me);
    let state_of = |login: &str| {
        pull.reviewers
            .iter()
            .find(|reviewer| reviewer.user.login.eq_ignore_ascii_case(login))
            .map(|reviewer| reviewer.state)
    };
    let asked_of_me = !mine && state_of(me) == Some(ReviewState::Pending);
    let any = |state: ReviewState| {
        pull.reviewers
            .iter()
            .any(|reviewer| reviewer.state == state)
    };
    let changes =
        any(ReviewState::ChangesRequested) || facts.approval == Approval::ChangesRequested;
    let mut out = Vec::new();
    let mut put = |group: PullGroup, rule: bool| {
        if rule {
            out.push(group);
        }
    };
    let open = pull.state == PullState::Open;
    let settled = facts.approval == Approval::Approved
        || (facts.approval != Approval::Required
            && pull
                .reviewers
                .iter()
                .all(|reviewer| reviewer.state == ReviewState::Approved));
    put(
        PullGroup::ReadyToMerge,
        open && !pull.draft
            && facts.mergeable == Mergeable::Clean
            && !changes
            && settled
            && pull.ci != CiState::Failure,
    );
    put(PullGroup::ReviewMine, open && asked_of_me);
    put(PullGroup::NoReviewers, open && pull.reviewers.is_empty());
    put(
        PullGroup::FailingChecks,
        open && pull.ci == CiState::Failure,
    );
    put(
        PullGroup::Conflicts,
        open && facts.mergeable == Mergeable::Conflicts,
    );
    put(PullGroup::ChangesRequested, open && changes);
    put(PullGroup::Commented, open && any(ReviewState::Commented));
    put(
        PullGroup::WaitingForReview,
        open && !asked_of_me && any(ReviewState::Pending),
    );
    put(PullGroup::Draft, open && pull.draft);
    if out.is_empty() {
        out.push(PullGroup::Other);
    }
    out
}

/// How a row is snoozed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snoozed {
    /// When it was snoozed, in seconds since the Unix epoch.
    pub since: i64,
    /// When it comes back by itself; never, until it is brought back,
    /// when absent.
    pub until: Option<i64>,
}

/// A pull request as a row of the inbox shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxPull {
    /// What its pin and snooze are kept by ([`key`]).
    pub key: String,
    /// The account it was found with ([`crate::model::Account::id`]).
    pub account: String,
    /// The account's provider.
    pub kind: ProviderKind,
    /// Its row of the lists of pull requests: its repository, number,
    /// title, state, author, branches, reviewers, assignees, labels,
    /// checks.
    pub pull: PullRequest,
    /// Lines added, when the provider said.
    pub additions: Option<u64>,
    /// Lines removed, when the provider said.
    pub deletions: Option<u64>,
    /// Whether it merges as it stands.
    pub mergeable: Mergeable,
    /// What its reviews add up to.
    pub approval: Approval,
    pub milestone: Option<Milestone>,
    /// Why it is the user's business.
    pub involvement: Vec<Involvement>,
    /// The groups it is shown in ([`groups`]).
    pub groups: Vec<PullGroup>,
    /// The repository on this machine it belongs to, when one of those
    /// looked at has a remote that is its repository (or its fork's).
    pub path: Option<String>,
    /// That remote's name there.
    pub remote: Option<String>,
    /// Whether the user pinned it to the top.
    pub pinned: bool,
    /// Whether it is snoozed, and until when.
    pub snoozed: Option<Snoozed>,
}

/// An issue as a row of the inbox shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxIssue {
    /// What its pin and snooze are kept by ([`key`]).
    pub key: String,
    /// The account it was found with.
    pub account: String,
    /// The account's provider.
    pub kind: ProviderKind,
    /// Its row of the lists of issues.
    pub issue: Issue,
    /// How many reactions it has, when the provider said.
    pub reactions: Option<u64>,
    /// Why it is the user's business.
    pub involvement: Vec<Involvement>,
    /// The repository on this machine it belongs to, when one is.
    pub path: Option<String>,
    /// The remote there that is its repository.
    pub remote: Option<String>,
    /// Whether the user pinned it.
    pub pinned: bool,
    /// Whether it is snoozed, and until when.
    pub snoozed: Option<Snoozed>,
}

/// A repository on this machine with uncommitted changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkInProgress {
    /// Its folder.
    pub path: String,
    /// What it is called: the folder's name.
    pub name: String,
    /// The branch checked out; none on a detached HEAD.
    pub branch: Option<String>,
    /// How far the branch is ahead of what it follows, and behind.
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    /// Its uncommitted files by kind.
    pub counts: bitbraid_git::wip::Counts,
}

/// What became of asking one account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountState {
    /// The account's id.
    pub account: String,
    /// Its provider.
    pub kind: ProviderKind,
    /// Its site.
    pub web: String,
    /// The user's name there.
    pub login: String,
    /// How fresh its answer is.
    pub freshness: Freshness,
    /// Whether it had more than was read.
    pub more: bool,
    /// Why it could not be asked, when it could not; the others are
    /// answered all the same.
    pub error: Option<String>,
}

/// What `inbox.items` answers.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Inbox {
    /// The open pull requests, pinned ones first, then the most recently
    /// changed; snoozed ones are among them, marked.
    pub pulls: Vec<InboxPull>,
    /// The open issues, in the same order.
    pub issues: Vec<InboxIssue>,
    /// The repositories with uncommitted changes.
    pub work: Vec<WorkInProgress>,
    /// Each account asked, and how it went.
    pub accounts: Vec<AccountState>,
    /// Why the pins and snoozes could not be read, when they could not
    /// (the rows are answered without them).
    pub marks_problem: Option<String>,
}

/// The key a row's pin and snooze are kept by:
/// `<account>/<pull|issue>/<owner/name>#<number>`.
pub fn key(account: &str, issue: bool, repo: &str, number: u64) -> String {
    let what = if issue { "issue" } else { "pull" };
    format!("{account}/{what}/{repo}#{number}")
}

/// Whether a text may be a row's key: not empty, at most [`MAX_KEY`]
/// bytes, without control characters or blanks.
pub fn usable_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY
        && !key.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// How long a snooze lasts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnoozeFor {
    /// Until it is brought back.
    Unsnoozed,
    /// An hour.
    Hour,
    /// Four hours.
    FourHours,
    /// Until nine in the morning tomorrow, by the local clock.
    Tomorrow,
    /// Until this time, in seconds since the Unix epoch.
    Until(i64),
}

/// When a snooze made at `now` ends: `None` when it lasts until it is
/// brought back. `offset` says how far east of UTC the local clocks are
/// at an instant, in seconds.
pub fn snooze_end(length: SnoozeFor, now: i64, offset: impl Fn(i64) -> i32) -> Option<i64> {
    match length {
        SnoozeFor::Unsnoozed => None,
        SnoozeFor::Hour => Some(now.saturating_add(3600)),
        SnoozeFor::FourHours => Some(now.saturating_add(4 * 3600)),
        SnoozeFor::Until(at) => Some(at),
        SnoozeFor::Tomorrow => {
            let local = now.saturating_add(i64::from(offset(now)));
            let today = local.div_euclid(86_400).saturating_mul(86_400);
            let wanted = today.saturating_add(86_400 + 9 * 3600);
            // The offset at nine tomorrow, which a change of the clocks
            // overnight makes another than tonight's.
            let guess = wanted.saturating_sub(i64::from(offset(now)));
            Some(wanted.saturating_sub(i64::from(offset(guess))))
        }
    }
}

/// Whether a snooze still holds at `now`.
pub fn still_snoozed(snoozed: &Snoozed, now: i64) -> bool {
    snoozed.until.is_none_or(|until| until > now)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Reviewer, User};

    fn pull(author: &str) -> PullRequest {
        PullRequest {
            repo: "o/r".into(),
            number: 1,
            title: "t".into(),
            state: PullState::Open,
            draft: false,
            author: User {
                login: author.into(),
                ..User::default()
            },
            head_ref: "topic".into(),
            head_repo: None,
            head_sha: String::new(),
            base_ref: "main".into(),
            created: String::new(),
            updated: String::new(),
            labels: Vec::new(),
            reviewers: Vec::new(),
            assignees: Vec::new(),
            ci: CiState::None,
            comments: None,
            web_url: String::new(),
        }
    }

    fn reviewer(login: &str, state: ReviewState) -> Reviewer {
        Reviewer {
            user: User {
                login: login.into(),
                ..User::default()
            },
            state,
        }
    }

    fn clean() -> PullFacts {
        PullFacts {
            mergeable: Mergeable::Clean,
            ..PullFacts::default()
        }
    }

    #[test]
    fn a_pull_request_is_in_every_group_whose_rule_it_meets() {
        use PullGroup::*;
        // Mine, clean, nobody asked: ready, and without reviewers.
        assert_eq!(
            groups(&pull("me"), &clean(), "me"),
            [ReadyToMerge, NoReviewers]
        );
        // Failed checks keep it out.
        let mut failing = pull("me");
        failing.ci = CiState::Failure;
        assert_eq!(
            groups(&failing, &clean(), "me"),
            [NoReviewers, FailingChecks]
        );
        // Checks that wait, or that nobody let run, do not.
        failing.ci = CiState::Pending;
        assert_eq!(
            groups(&failing, &clean(), "me"),
            [ReadyToMerge, NoReviewers]
        );
        // A review awaited or a comment keeps it out; approvals let it in.
        let mut reviewed = pull("me");
        reviewed.reviewers = vec![
            reviewer("ann", ReviewState::Approved),
            reviewer("bob", ReviewState::Commented),
        ];
        assert_eq!(groups(&reviewed, &clean(), "me"), [Commented]);
        reviewed.reviewers.truncate(1);
        assert_eq!(groups(&reviewed, &clean(), "me"), [ReadyToMerge]);
        reviewed
            .reviewers
            .push(reviewer("cy", ReviewState::Pending));
        assert_eq!(groups(&reviewed, &clean(), "me"), [WaitingForReview]);
        // What the provider sums up as approved is settled.
        let approved = PullFacts {
            approval: Approval::Approved,
            ..clean()
        };
        assert_eq!(
            groups(&reviewed, &approved, "me"),
            [ReadyToMerge, WaitingForReview]
        );
        // A review that is required and missing keeps it out.
        let required = PullFacts {
            approval: Approval::Required,
            ..clean()
        };
        let mut asked = pull("me");
        asked.reviewers = vec![reviewer("ann", ReviewState::Pending)];
        assert_eq!(groups(&asked, &required, "me"), [WaitingForReview]);
        // Changes asked, by a reviewer or by the sum.
        let mut changes = pull("me");
        changes.reviewers = vec![
            reviewer("ann", ReviewState::ChangesRequested),
            reviewer("bob", ReviewState::Commented),
        ];
        assert_eq!(
            groups(&changes, &clean(), "me"),
            [ChangesRequested, Commented]
        );
        let summed = PullFacts {
            approval: Approval::ChangesRequested,
            mergeable: Mergeable::Conflicts,
            ..PullFacts::default()
        };
        assert_eq!(
            groups(&pull("me"), &summed, "me"),
            [NoReviewers, Conflicts, ChangesRequested]
        );
        // A draft is never ready.
        let mut draft = pull("me");
        draft.draft = true;
        assert_eq!(groups(&draft, &clean(), "me"), [NoReviewers, Draft]);
    }

    #[test]
    fn someone_elses_pull_request_waits_for_my_review_and_not_for_review() {
        use PullGroup::*;
        let mut theirs = pull("ann");
        theirs.reviewers = vec![reviewer("Me", ReviewState::Pending)];
        let unknown = PullFacts::default();
        assert_eq!(groups(&theirs, &unknown, "me"), [ReviewMine]);
        // Once answered, it waits for nobody's review.
        theirs.reviewers = vec![reviewer("me", ReviewState::Approved)];
        assert_eq!(groups(&theirs, &unknown, "me"), [Other]);
        // My own, with me among its reviewers, does not ask me.
        let mut own = pull("me");
        own.reviewers = vec![reviewer("me", ReviewState::Pending)];
        assert_eq!(groups(&own, &unknown, "me"), [WaitingForReview]);
        // A closed one is in no group but the last.
        let mut closed = pull("me");
        closed.state = PullState::Closed;
        assert_eq!(groups(&closed, &clean(), "me"), [Other]);
    }

    #[test]
    fn groups_are_known_by_their_keys() {
        for group in PullGroup::ALL {
            assert_eq!(
                serde_json::to_value(group).unwrap(),
                serde_json::Value::from(group.key())
            );
        }
    }

    #[test]
    fn keys_name_the_account_the_kind_the_repository_and_the_number() {
        let made = key("github:github.com:me", false, "o/r", 7);
        assert_eq!(made, "github:github.com:me/pull/o/r#7");
        assert!(usable_key(&made));
        assert_ne!(made, key("github:github.com:me", true, "o/r", 7));
        for bad in ["", "a b", "a\nb", &"k".repeat(MAX_KEY + 1)] {
            assert!(!usable_key(bad), "{bad:?}");
        }
        assert!(usable_key(&"k".repeat(MAX_KEY)));
    }

    #[test]
    fn a_snooze_ends_after_its_hours_or_at_nine_tomorrow_by_the_local_clock() {
        // 2026-10-14T21:30:00Z.
        let now = 1_792_013_400;
        let utc = |_| 0;
        assert_eq!(snooze_end(SnoozeFor::Unsnoozed, now, utc), None);
        assert_eq!(snooze_end(SnoozeFor::Hour, now, utc), Some(now + 3600));
        assert_eq!(
            snooze_end(SnoozeFor::FourHours, now, utc),
            Some(now + 14_400)
        );
        assert_eq!(snooze_end(SnoozeFor::Until(5), now, utc), Some(5));
        // UTC: nine on the 15th.
        assert_eq!(
            snooze_end(SnoozeFor::Tomorrow, now, utc).map(crate::api::rfc3339),
            Some("2026-10-15T09:00:00Z".to_owned())
        );
        // Two hours east it is half past eleven on the 14th still: nine
        // on the 15th there is seven in UTC.
        assert_eq!(
            snooze_end(SnoozeFor::Tomorrow, now, |_| 7200).map(crate::api::rfc3339),
            Some("2026-10-15T07:00:00Z".to_owned())
        );
        // Three hours east it is the 15th already: nine on the 16th.
        assert_eq!(
            snooze_end(SnoozeFor::Tomorrow, now, |_| 3 * 3600).map(crate::api::rfc3339),
            Some("2026-10-16T06:00:00Z".to_owned())
        );
        // The clocks go back an hour overnight: tomorrow's offset counts.
        let changing = |at: i64| if at < now + 3 * 3600 { 7200 } else { 3600 };
        assert_eq!(
            snooze_end(SnoozeFor::Tomorrow, now, changing).map(crate::api::rfc3339),
            Some("2026-10-15T08:00:00Z".to_owned())
        );
        // At the ends of time nothing overflows.
        for at in [i64::MIN, i64::MAX] {
            for length in [SnoozeFor::Hour, SnoozeFor::FourHours, SnoozeFor::Tomorrow] {
                let _ = snooze_end(length, at, |_| i32::MAX);
                let _ = snooze_end(length, at, |_| i32::MIN);
            }
        }
    }

    #[test]
    fn a_snooze_holds_until_its_end() {
        let until = Snoozed {
            since: 0,
            until: Some(100),
        };
        assert!(still_snoozed(&until, 99));
        assert!(!still_snoozed(&until, 100));
        let ever = Snoozed {
            since: 0,
            until: None,
        };
        assert!(still_snoozed(&ever, i64::MAX));
    }
}
