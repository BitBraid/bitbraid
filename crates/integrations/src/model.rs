//! What the providers' answers are made into: accounts, repositories at a
//! provider, pull requests, issues and the checks run on commits, the same
//! shapes for GitHub and GitLab.
//!
//! These are the outputs of the commands in this crate, and what the
//! window shows. They hold what was read and nothing that signs in: no
//! token is ever a field here. Every text in them came from another
//! machine and is shown as text, never run or followed without the user.
//!
//! A merge request of GitLab is a [`PullRequest`] here, and its `iid` is the
//! `number`; a GitLab group path (`group/subgroup`) is an `owner`.
//!
//! Times are RFC 3339 texts as the provider gave them.
//!
//! Sources: docs.github.com/en/rest (pulls, issues, checks, commits/statuses,
//! repos, users, reactions left out); docs.gitlab.com/api (merge_requests,
//! issues, projects, pipelines, commits/statuses, users, keys).

use serde::{Deserialize, Serialize};

/// The longest title kept, in characters; the rest is cut.
pub const MAX_TITLE: usize = 1024;
/// The longest body or comment kept, in bytes; the rest is cut.
pub const MAX_BODY: usize = 256 * 1024;

/// Which provider an account is at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// GitHub.com, or a GitHub Enterprise Server at its own address.
    Github,
    /// GitLab.com, or a GitLab of one's own.
    Gitlab,
}

/// How an account was signed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignIn {
    /// A code typed at the provider's site (the OAuth device flow).
    Device,
    /// A token made at the provider and pasted.
    Token,
}

/// An account the user connected. Its token is in the system's keychain,
/// under `id`, and nowhere else.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    /// `<kind>:<web host>:<login>`, for example `github:github.com:octo`.
    pub id: String,
    /// The provider.
    pub kind: ProviderKind,
    /// The site as people open it: `https://github.com`,
    /// `https://gitlab.example.org`.
    pub web: String,
    /// Where its API answers: `https://api.github.com`,
    /// `https://ghe.example.org/api/v3`, `https://gitlab.com/api/v4`.
    pub api: String,
    /// The user's name at the provider.
    pub login: String,
    /// The user's full name, when they gave one.
    pub name: Option<String>,
    /// The user's picture.
    pub avatar_url: Option<String>,
    /// How it was signed in.
    pub sign_in: SignIn,
    /// What the token was granted, as the provider names it.
    pub scopes: Vec<String>,
    /// The account used for its host when several are connected there.
    pub default: bool,
}

/// A person at a provider.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct User {
    /// Their name at the provider.
    pub login: String,
    /// Their full name, when the answer had one.
    pub name: Option<String>,
    /// Their picture.
    pub avatar_url: Option<String>,
    /// Their page.
    pub web_url: Option<String>,
}

/// A repository at a provider (a project, at GitLab).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostedRepo {
    /// The provider's own id for it (GitLab's project id is needed to ask
    /// about it).
    pub id: String,
    /// The user, organisation or group it belongs to; a GitLab group path
    /// may have several parts (`group/subgroup`).
    pub owner: String,
    pub name: String,
    /// `owner/name`.
    pub full_name: String,
    /// What it says it is.
    pub description: Option<String>,
    /// Whether only some may see it.
    pub private: bool,
    /// Whether it is a fork, and of what (`owner/name`).
    pub parent: Option<String>,
    /// Whether it takes no more changes.
    pub archived: bool,
    /// Its default branch.
    pub default_branch: Option<String>,
    /// The address to clone over HTTPS.
    pub clone_https: String,
    /// The address to clone over SSH.
    pub clone_ssh: String,
    /// Its page.
    pub web_url: String,
    /// When it last changed.
    pub updated: Option<String>,
    /// How many starred it.
    pub stars: u64,
}

/// What kind of place a repository may be made in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NamespaceKind {
    /// The user's own.
    User,
    /// A GitHub organisation.
    Organisation,
    /// A GitLab group or subgroup.
    Group,
}

/// A place where the account may make a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Namespace {
    /// Its path: a login, an organisation, `group/subgroup`.
    pub path: String,
    /// Its name as people read it.
    pub name: String,
    /// What it is.
    pub kind: NamespaceKind,
    /// Its picture.
    pub avatar_url: Option<String>,
}

/// A repository made at a provider, and what was done with it in the
/// open repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MadeRepo {
    pub repo: HostedRepo,
    /// The remote added for it, when one was asked for.
    pub remote: Option<String>,
    /// The branch pushed to it, when that was asked for.
    pub pushed: Option<String>,
}

/// A repository made by forking, and the remote that was added for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Forked {
    /// The fork.
    pub repo: HostedRepo,
    /// The remote added for it in the open repository, when one was.
    pub remote: Option<String>,
}

/// A public key uploaded to an account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshKey {
    /// The provider's id for it.
    pub id: String,
    /// What it is called there.
    pub title: String,
    /// Its fingerprint (`SHA256:…`), when known.
    pub fingerprint: Option<String>,
}

/// What may be chosen when a pull request is made or changed.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PullOptions {
    /// The repository's labels.
    pub labels: Vec<Label>,
    /// Its open milestones.
    pub milestones: Vec<Milestone>,
    /// Who may be asked to review or be assigned.
    pub people: Vec<User>,
    /// The ways it allows a pull request to be merged.
    pub merge_methods: Vec<MergeMethod>,
}

/// A pull request's template, read from the working tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullTemplate {
    /// Its path in the repository.
    pub path: String,
    /// Its name, for a list to choose from.
    pub name: String,
    /// What it says, in Markdown.
    pub body: String,
}

/// What came of a merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Merged {
    /// Whether it was merged.
    pub merged: bool,
    /// The commit the merge made, when the provider said.
    pub sha: Option<String>,
    /// What the provider said about it, and what could not be done after
    /// it was merged.
    pub message: Option<String>,
    /// The open pull requests whose base was its branch and that now go
    /// into its base, when that was asked.
    #[serde(default)]
    pub retargeted: Vec<u64>,
    /// Whether its branch was removed afterwards.
    #[serde(default)]
    pub branch_deleted: bool,
}

/// A pull request checked out in the open repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullCheckout {
    /// The local branch it is in, now checked out.
    pub branch: String,
    /// The commit at its head, as fetched.
    pub commit: String,
    /// Whether the branch was made now.
    pub created: bool,
    /// Whether a branch that was there was moved forward to the head.
    pub updated: bool,
    /// Why a branch that was there was left where it was, when it was.
    pub note: Option<String>,
    /// The remote branch it follows (`origin/topic`), when the branch it
    /// comes from can be reached.
    pub upstream: Option<String>,
    /// The remote added for the fork it comes from, when one was.
    pub remote_added: Option<String>,
}

/// A pull request's head, fetched into the open repository without
/// checking anything out: where a worktree for it starts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullHead {
    /// The commit at its head, as fetched.
    pub commit: String,
    /// The branch it comes from, as the provider names it.
    pub branch: String,
    /// The remote here that is the repository that branch is in, when one
    /// is: the pull request's own, or its fork's (none is added for this).
    pub remote: Option<String>,
    /// Where a worktree for it starts: that remote's branch when it is at
    /// the head, else the commit.
    pub start: String,
}

/// Which repository at which account the open repository's remote is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoLink {
    /// The remote's name.
    pub remote: String,
    /// The account that is asked about it ([`Account::id`]).
    pub account: String,
    /// The provider.
    pub kind: ProviderKind,
    /// The owner path, as in [`HostedRepo::owner`].
    pub owner: String,
    /// The repository's name.
    pub name: String,
}

/// Where a pull request stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullState {
    Open,
    /// Closed without being merged.
    Closed,
    Merged,
}

/// What a reviewer said last.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Approved,
    /// Asked for changes.
    ChangesRequested,
    /// Commented only.
    Commented,
    /// Asked to review and not answered yet.
    Pending,
    /// A review that was set aside.
    Dismissed,
}

/// How the checks of a commit stand, summed up or one by one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiState {
    /// Nothing was run, or nothing is known.
    #[default]
    None,
    /// Waiting to run.
    Pending,
    Running,
    /// Passed.
    Success,
    /// Failed, or ended in an error.
    Failure,
    /// Stopped before it ended.
    Cancelled,
    /// Not run, on purpose.
    Skipped,
    /// Ended without saying pass or fail.
    Neutral,
}

impl CiState {
    /// How much a state weighs when checks are summed up: the heaviest
    /// one is the sum.
    fn weight(self) -> u8 {
        match self {
            Self::None => 0,
            Self::Skipped => 1,
            Self::Neutral => 2,
            Self::Success => 3,
            Self::Cancelled => 4,
            Self::Pending => 5,
            Self::Running => 6,
            Self::Failure => 7,
        }
    }

    /// Checks summed up, as [`CommitStatus::state`] says.
    pub fn sum(states: impl IntoIterator<Item = Self>) -> Self {
        states
            .into_iter()
            .max_by_key(|state| state.weight())
            .unwrap_or_default()
    }
}

/// A label on a pull request or an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub name: String,
    /// Its colour as six hexadecimal digits, without `#`.
    pub color: Option<String>,
    /// What it means.
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Milestone {
    /// The provider's id (GitHub's number, GitLab's id).
    pub id: String,
    pub title: String,
    /// When it is due.
    pub due: Option<String>,
    /// Whether it is still open.
    pub open: bool,
}

/// A reviewer of a pull request, asked or not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reviewer {
    /// Who.
    pub user: User,
    /// What they said last; [`ReviewState::Pending`] while asked and
    /// silent.
    pub state: ReviewState,
}

/// A pull request as a row of the list shows it, with what its hover
/// card adds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullRequest {
    /// The repository it is in, `owner/name`.
    pub repo: String,
    /// Its number (GitLab's `iid`).
    pub number: u64,
    pub title: String,
    /// Open, closed or merged.
    pub state: PullState,
    /// Whether it is a draft.
    pub draft: bool,
    /// Who opened it.
    pub author: User,
    /// The branch it brings.
    pub head_ref: String,
    /// The repository that branch is in, `owner/name`, when it is a fork;
    /// `None` when it is the same repository, or the fork is gone or the
    /// account cannot see it.
    pub head_repo: Option<String>,
    /// The commit at the head of that branch.
    pub head_sha: String,
    /// The branch it goes into.
    pub base_ref: String,
    /// When it was opened.
    pub created: String,
    /// When it last changed.
    pub updated: String,
    pub labels: Vec<Label>,
    /// Who was asked to review it and what each said.
    pub reviewers: Vec<Reviewer>,
    /// Who it is assigned to.
    pub assignees: Vec<User>,
    /// Its checks summed up.
    pub ci: CiState,
    /// How many comments it has, when the answer said.
    pub comments: Option<u64>,
    /// Its page.
    pub web_url: String,
}

/// Whether a pull request can be merged as it stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mergeable {
    /// The provider has not worked it out yet.
    #[default]
    Unknown,
    /// It merges without conflicts and nothing holds it back.
    Clean,
    /// Its changes conflict with the base.
    Conflicts,
    /// Something the repository requires is missing (reviews, checks).
    Blocked,
}

/// How a pull request is merged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    /// A merge commit.
    Merge,
    /// One commit with all its changes.
    Squash,
    /// Its commits replayed onto the base.
    Rebase,
}

/// A check run on a commit: a job of GitHub Actions or GitLab CI, or a
/// status another service set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Check {
    pub name: String,
    /// How it stands.
    pub state: CiState,
    /// Where its output is.
    pub url: Option<String>,
    /// What it says about itself.
    pub description: Option<String>,
    /// When it started.
    pub started: Option<String>,
    /// When it ended.
    pub finished: Option<String>,
}

/// The checks of one commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitStatus {
    /// The commit.
    pub sha: String,
    /// The checks summed up ([`CiState::sum`]): a failure if one failed;
    /// else running while one runs; else pending while one waits; else
    /// cancelled if one was stopped; else a success if one passed; else
    /// neutral, skipped, or none when there are no checks.
    pub state: CiState,
    /// Each check.
    pub checks: Vec<Check>,
}

/// A commit of a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullCommit {
    /// Its id.
    pub sha: String,
    /// The first line of its message.
    pub summary: String,
    /// Its author at the provider, when the provider knows them.
    pub author: Option<User>,
    /// The name in the commit.
    pub author_name: String,
    /// When it was written.
    pub date: String,
}

/// A file a pull request changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullFile {
    pub path: String,
    /// Its path before, when it was renamed.
    pub previous_path: Option<String>,
    /// `added`, `removed`, `modified`, `renamed`, `copied`, `changed`.
    pub status: String,
    /// Lines added.
    pub additions: u64,
    /// Lines removed.
    pub deletions: u64,
    /// The unified diff, when the provider sent one (it leaves out large
    /// and binary files).
    pub patch: Option<String>,
}

/// A comment on a pull request or an issue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    /// The provider's id, which a reply names.
    pub id: String,
    /// Who wrote it.
    pub author: User,
    /// What it says, in Markdown.
    pub body: String,
    /// When it was written.
    pub created: String,
    /// When it was last changed, when it was.
    pub updated: Option<String>,
}

/// A conversation on a pull request: on the whole of it, or on a line of
/// a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    /// The provider's id (GitHub's first comment, GitLab's discussion).
    pub id: String,
    /// The file it is on, if it is on one.
    pub path: Option<String>,
    /// The line it is on, in the new version of the file.
    pub line: Option<u64>,
    /// The lines of the diff it is about, when the provider sent them.
    pub diff_hunk: Option<String>,
    /// Whether it was marked resolved.
    pub resolved: bool,
    /// Its comments, oldest first.
    pub comments: Vec<Comment>,
}

/// A review: a verdict with what was said with it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    /// The provider's id.
    pub id: String,
    /// Who reviewed.
    pub author: User,
    /// What they decided.
    pub state: ReviewState,
    /// What they wrote with it.
    pub body: String,
    /// When.
    pub submitted: Option<String>,
}

/// A pull request with everything its view shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullDetails {
    /// What the list shows of it.
    pub pull: PullRequest,
    /// Its description, in Markdown.
    pub body: String,
    pub milestone: Option<Milestone>,
    /// Whether it can be merged as it stands.
    pub mergeable: Mergeable,
    /// The ways the repository allows it to be merged.
    pub merge_methods: Vec<MergeMethod>,
    /// The checks of its head commit.
    pub checks: Vec<Check>,
    /// Its commits, oldest first.
    pub commits: Vec<PullCommit>,
    /// The files it changes.
    pub files: Vec<PullFile>,
    /// The reviews given, oldest first.
    pub reviews: Vec<Review>,
    /// The conversations on it, oldest first.
    pub threads: Vec<Thread>,
    /// How fresh it is: whether it was kept from before because the
    /// provider could not be asked, and when it was read.
    #[serde(default)]
    pub freshness: Freshness,
}

/// Open or closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IssueState {
    Open,
    Closed,
}

/// An issue as a row of the list shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Issue {
    /// The repository it is in, `owner/name`.
    pub repo: String,
    /// Its number (GitLab's `iid`).
    pub number: u64,
    pub title: String,
    /// Open or closed.
    pub state: IssueState,
    /// Who opened it.
    pub author: User,
    /// Who it is assigned to.
    pub assignees: Vec<User>,
    pub labels: Vec<Label>,
    pub milestone: Option<Milestone>,
    /// When it was opened.
    pub created: String,
    /// When it last changed.
    pub updated: String,
    /// How many comments it has.
    pub comments: u64,
    /// Its page.
    pub web_url: String,
}

/// An issue with what its view shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueDetails {
    /// What the list shows of it.
    pub issue: Issue,
    /// Its description, in Markdown.
    pub body: String,
    /// Its comments, oldest first.
    pub comments: Vec<Comment>,
    /// How fresh it is: whether it was kept from before because the
    /// provider could not be asked, and when it was read.
    #[serde(default)]
    pub freshness: Freshness,
}

/// Pull requests stacked on one another: each one's base is the branch of
/// the one below.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stack {
    /// From the one on the base branch to the top.
    pub entries: Vec<StackEntry>,
    /// Which entry was asked about.
    pub position: usize,
}

/// One pull request of a stack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StackEntry {
    pub number: u64,
    pub title: String,
    /// Its branch.
    pub head_ref: String,
    /// The branch it goes into.
    pub base_ref: String,
    /// Open, closed or merged.
    pub state: PullState,
}

/// How fresh a list is, and how much more may be asked.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Freshness {
    /// When it was read from the provider.
    pub fetched: Option<String>,
    /// Whether it came from what was kept because the provider could not
    /// be reached.
    pub offline: bool,
    /// How many requests are left until `reset`, when the provider said.
    pub remaining: Option<u64>,
    /// When the allowance is renewed.
    pub reset: Option<String>,
}

/// A list read from a provider, with how fresh it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing<T> {
    /// What was found.
    pub items: Vec<T>,
    /// Whether there is more than was read.
    pub more: bool,
    /// How fresh it is.
    pub freshness: Freshness,
}

/// What a sign-in by code asks the user to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCode {
    /// The code the user types at the provider.
    pub user_code: String,
    /// Where they type it.
    pub verification_uri: String,
    /// What the application asks the provider with until the user is
    /// done. Short-lived, and worth nothing without the user's approval.
    pub device_code: String,
    /// How many seconds the code lasts.
    pub expires_in: u64,
    /// How many seconds to wait between asks.
    pub interval: u64,
}

/// Where a sign-in by code stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum DevicePoll {
    /// The user has not answered yet.
    Waiting,
    /// Asked too often: wait this many seconds between asks from now on.
    SlowDown {
        /// The new wait.
        interval: u64,
    },
    /// The user said no.
    Denied,
    /// The code ran out.
    Expired,
    /// Signed in; the token is in the keychain.
    Done {
        /// The account.
        account: Account,
    },
}

/// Cut a text to at most `max` bytes, at a character's edge.
pub fn bounded(text: &str, max: usize) -> &str {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_cuts_at_a_character_edge() {
        assert_eq!(bounded("abc", 5), "abc");
        assert_eq!(bounded("abc", 2), "ab");
        assert_eq!(bounded("aé", 2), "a");
        assert_eq!(bounded("é", 0), "");
    }

    #[test]
    fn checks_are_summed_by_the_heaviest() {
        use CiState::*;
        assert_eq!(CiState::sum([]), None);
        assert_eq!(CiState::sum([Success, Skipped, Neutral]), Success);
        assert_eq!(CiState::sum([Success, Pending]), Pending);
        assert_eq!(CiState::sum([Pending, Running, Success]), Running);
        assert_eq!(CiState::sum([Running, Failure, Success]), Failure);
        assert_eq!(CiState::sum([Success, Cancelled]), Cancelled);
        assert_eq!(CiState::sum([Skipped, Neutral]), Neutral);
    }

    #[test]
    fn states_read_as_written() {
        let poll: DevicePoll =
            serde_json::from_str(r#"{"state":"slow_down","interval":10}"#).unwrap();
        assert_eq!(poll, DevicePoll::SlowDown { interval: 10 });
        assert!(serde_json::from_str::<CiState>(r#""changes_requested""#).is_err());
        assert_eq!(
            serde_json::to_string(&ReviewState::ChangesRequested).unwrap(),
            r#""changes_requested""#
        );
    }
}
