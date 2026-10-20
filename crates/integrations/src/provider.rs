//! One face for both providers: what the commands ask of GitHub and of
//! GitLab, each operation a method of [`Provider`], answered with the
//! shapes of [`crate::model`].
//!
//! [`of`] gives the provider of an account's kind. Each method takes a
//! [`Session`] (the account, its token, the network and the wish to stop)
//! and what it is about: a [`Target`] repository and the operation's own
//! values.
//!
//! Sources: docs.github.com/en/rest and docs.gitlab.com/api, as named in
//! [`crate::github`] and [`crate::gitlab`].

use std::{collections::BTreeMap, sync::atomic::AtomicBool};

use crate::{
    api::{Api, ProviderError},
    http::Fetch,
    model::{
        Account, Comment, CommitStatus, HostedRepo, Issue, IssueDetails, Listing, MergeMethod,
        Merged, Namespace, ProviderKind, PullDetails, PullOptions, PullRequest, Review, SshKey,
        Stack,
    },
    secrets::Secret,
};

/// What a provider says about the owner of a token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Their login.
    pub login: String,
    /// The provider's id for them.
    pub id: String,
    /// Their full name.
    pub name: Option<String>,
    /// Their e-mail address, when the token may see it.
    pub email: Option<String>,
    /// Their picture.
    pub avatar_url: Option<String>,
    /// What the token was granted, when the provider says.
    pub scopes: Vec<String>,
}

/// An account, signed in, and how to reach it.
#[derive(Debug, Clone, Copy)]
pub struct Session<'a> {
    /// The account.
    pub account: &'a Account,
    /// Its token.
    pub token: &'a Secret,
    /// The network.
    pub fetch: &'a dyn Fetch,
    /// Set when the answer is no longer wanted.
    pub cancel: &'a AtomicBool,
    /// Seconds since the Unix epoch, now.
    pub now: i64,
}

impl<'a> Session<'a> {
    /// Wait a while, unless the answer is no longer wanted.
    pub fn pause(&self, wait: std::time::Duration) -> Result<(), ProviderError> {
        if self.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(ProviderError::Http(crate::http::FetchError::Cancelled));
        }
        std::thread::sleep(wait);
        Ok(())
    }

    /// The account's API, signed in.
    pub fn api(&self) -> Result<Api<'a>, ProviderError> {
        Api::new(
            self.fetch,
            &self.account.api,
            self.account.kind,
            Some(self.token),
            self.cancel,
        )
    }

    /// The account's GraphQL API, signed in.
    pub fn graphql(&self) -> Result<Api<'a>, ProviderError> {
        Api::new(
            self.fetch,
            &crate::api::graphql_address(&self.account.api),
            self.account.kind,
            Some(self.token),
            self.cancel,
        )
    }
}

/// A repository at a provider: its owner path and name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// `owner`, or a GitLab group path.
    pub owner: String,
    /// The repository's name.
    pub name: String,
}

impl Target {
    /// `owner/name`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Which pull requests a list holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PullFilter {
    /// All of the repository's.
    All,
    /// Those the user opened.
    Mine,
    /// Those assigned to the user.
    Assigned,
    /// Those the user is asked to review.
    ReviewRequested,
}

/// Which states a list holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateFilter {
    /// Open ones.
    Open,
    /// Closed without being merged.
    Closed,
    /// Merged.
    Merged,
    /// All.
    All,
}

/// What a list of pull requests is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullQuery {
    /// Whose.
    pub filter: PullFilter,
    /// In which state.
    pub state: StateFilter,
    /// The provider's search words, when given.
    pub query: Option<String>,
    /// Most items read.
    pub most: usize,
}

/// Which issues a list holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueFilter {
    /// All of the repository's.
    All,
    /// Assigned to the user.
    Assigned,
    /// Opened by the user.
    Created,
    /// Where the user is named.
    Mentioned,
}

/// What a list of issues is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueQuery {
    /// Whose.
    pub filter: IssueFilter,
    /// Open, closed or all ([`StateFilter::Merged`] is not one).
    pub state: StateFilter,
    /// The provider's search words, when given.
    pub query: Option<String>,
    /// Most items read.
    pub most: usize,
}

/// Which repositories a list holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoFilter {
    /// The user's own.
    Own,
    /// Of the user's organisations or groups.
    Organisation,
    /// Starred.
    Starred,
    /// Every one the user may see.
    All,
}

/// What a list of repositories is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoQuery {
    /// Which.
    pub filter: RepoFilter,
    /// Words its name has, when given.
    pub query: Option<String>,
    /// The page, from 1.
    pub page: u32,
}

/// A repository to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRepo {
    /// Where: an organisation or group path; the user's own when absent.
    pub namespace: Option<String>,
    /// Its name.
    pub name: String,
    /// What it says it is.
    pub description: Option<String>,
    /// Whether only some may see it.
    pub private: bool,
}

/// A pull request to open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPull {
    /// The branch it brings (`owner:branch` for a fork at GitHub).
    pub head: String,
    /// The branch it goes into.
    pub base: String,
    /// Its title.
    pub title: String,
    /// Its description.
    pub body: String,
    /// Whether it is a draft.
    pub draft: bool,
    /// Who is asked to review it.
    pub reviewers: Vec<String>,
    /// Who it is assigned to.
    pub assignees: Vec<String>,
    /// Its labels.
    pub labels: Vec<String>,
    /// Its milestone's id.
    pub milestone: Option<String>,
}

/// What changes of a pull request; what is `None` stays.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PullChange {
    /// A new title.
    pub title: Option<String>,
    /// A new description.
    pub body: Option<String>,
    /// Who is asked to review it, in place of those before.
    pub reviewers: Option<Vec<String>>,
    /// Who it is assigned to, in place of those before.
    pub assignees: Option<Vec<String>>,
    /// Its labels, in place of those before.
    pub labels: Option<Vec<String>>,
    /// Its milestone (`Some(None)` takes it away).
    pub milestone: Option<Option<String>>,
    /// Whether it is a draft.
    pub draft: Option<bool>,
    /// The branch it goes into.
    pub base: Option<String>,
}

/// How a pull request is merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRequest {
    /// The way.
    pub method: MergeMethod,
    /// Whether its branch is removed afterwards.
    pub delete_branch: bool,
    /// The merge commit's title.
    pub title: Option<String>,
    /// The merge commit's message.
    pub message: Option<String>,
    /// Whether the open pull requests whose base is its branch go into
    /// its base afterwards, before its branch is removed.
    pub retarget_stack: bool,
}

/// A comment on lines of a file of a pull request, as they are in its
/// head: what a suggestion is sent as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineComment {
    /// The file's path in the pull request's head.
    pub path: String,
    /// The line it is on; the last one when it is on several.
    pub line: u64,
    /// The first line, when it is on several. GitLab keeps the comment on
    /// `line` alone: a suggestion there says itself how many lines above
    /// it it takes.
    pub start_line: Option<u64>,
    /// What it says, in Markdown.
    pub body: String,
}

/// What a review says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewEvent {
    /// Approve.
    Approve,
    /// Ask for changes.
    RequestChanges,
    /// Comment only.
    Comment,
}

/// What the inbox asks of one account: the open pull requests and
/// issues of the user, across every repository the account sees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboxQuery {
    /// Only those of these repositories (`owner/name`), when given: a
    /// workspace's. The provider may still answer others; the command
    /// leaves them out.
    pub repos: Option<Vec<String>>,
    /// Most of each kind read.
    pub most: usize,
}

/// A pull request the inbox found, with what its row in a list lacks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundPull {
    /// The row.
    pub pull: PullRequest,
    /// Lines, mergeability, reviews summed up, milestone.
    pub facts: crate::inbox::PullFacts,
    /// Why it is the user's.
    pub involvement: Vec<crate::inbox::Involvement>,
}

/// An issue the inbox found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundIssue {
    /// The row.
    pub issue: Issue,
    /// How many reactions it has, when the provider said.
    pub reactions: Option<u64>,
    /// Why it is the user's.
    pub involvement: Vec<crate::inbox::Involvement>,
}

/// What one account answered the inbox.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// Its open pull requests, each once.
    pub pulls: Vec<FoundPull>,
    /// Its open issues, each once.
    pub issues: Vec<FoundIssue>,
    /// Whether there was more than was read.
    pub more: bool,
    /// How fresh it is.
    pub freshness: crate::model::Freshness,
}

/// What a provider does. Each method is one operation the provider
/// commands ask of it.
pub trait Provider: Sync {
    /// Which provider it is.
    fn kind(&self) -> ProviderKind;

    /// Who the token belongs to and what it was granted.
    fn whoami(&self, session: &Session<'_>) -> Result<Identity, ProviderError>;

    /// The reference a pull request's head is fetched from
    /// (`refs/pull/7/head`, `refs/merge-requests/7/head`).
    fn pull_ref(&self, number: u64) -> String;

    /// The repositories the account may clone.
    fn repos(
        &self,
        session: &Session<'_>,
        query: &RepoQuery,
    ) -> Result<Listing<HostedRepo>, ProviderError>;

    /// Where the account may make a repository.
    fn namespaces(&self, session: &Session<'_>) -> Result<Vec<Namespace>, ProviderError>;

    /// One repository, as a list shows it.
    fn repo_get(&self, session: &Session<'_>, target: &Target)
    -> Result<HostedRepo, ProviderError>;

    /// Make a repository.
    fn repo_create(
        &self,
        session: &Session<'_>,
        repo: &NewRepo,
    ) -> Result<HostedRepo, ProviderError>;

    /// Fork a repository.
    fn fork(
        &self,
        session: &Session<'_>,
        target: &Target,
        namespace: Option<&str>,
    ) -> Result<HostedRepo, ProviderError>;

    /// Upload a public key.
    fn ssh_key_add(
        &self,
        session: &Session<'_>,
        title: &str,
        key: &str,
    ) -> Result<SshKey, ProviderError>;

    /// The pictures of the people with these e-mail addresses, as far as
    /// the provider knows them: from the commits of `target` where the
    /// provider learns them that way.
    fn avatars(
        &self,
        session: &Session<'_>,
        target: Option<&Target>,
        emails: &[String],
    ) -> Result<BTreeMap<String, String>, ProviderError>;

    /// A repository's pull requests.
    fn pr_list(
        &self,
        session: &Session<'_>,
        target: &Target,
        query: &PullQuery,
    ) -> Result<Listing<PullRequest>, ProviderError>;

    /// One pull request with everything its view shows.
    fn pr_show(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
    ) -> Result<PullDetails, ProviderError>;

    /// One pull request as a row of the list shows it, without what only
    /// its view shows.
    fn pr_get(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
    ) -> Result<PullRequest, ProviderError>;

    /// What may be chosen for a pull request.
    fn pr_options(
        &self,
        session: &Session<'_>,
        target: &Target,
    ) -> Result<PullOptions, ProviderError>;

    /// Open a pull request.
    fn pr_create(
        &self,
        session: &Session<'_>,
        target: &Target,
        pull: &NewPull,
    ) -> Result<PullRequest, ProviderError>;

    /// Change a pull request.
    fn pr_update(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        change: &PullChange,
    ) -> Result<PullDetails, ProviderError>;

    /// Merge a pull request.
    fn pr_merge(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        merge: &MergeRequest,
    ) -> Result<Merged, ProviderError>;

    /// Review a pull request.
    fn pr_review(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        event: ReviewEvent,
        body: Option<&str>,
    ) -> Result<Review, ProviderError>;

    /// Comment on a pull request, or answer a comment.
    fn pr_comment(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        body: &str,
        reply_to: Option<&str>,
    ) -> Result<Comment, ProviderError>;

    /// Comment on lines of a file of a pull request.
    fn pr_line_comment(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        comment: &LineComment,
    ) -> Result<Comment, ProviderError>;

    /// The stack a pull request is in.
    fn pr_stack(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
    ) -> Result<Stack, ProviderError>;

    /// The open pull request of a branch, if there is one.
    fn pr_for_branch(
        &self,
        session: &Session<'_>,
        target: &Target,
        branch: &str,
    ) -> Result<Option<PullRequest>, ProviderError>;

    /// A repository's issues.
    fn issue_list(
        &self,
        session: &Session<'_>,
        target: &Target,
        query: &IssueQuery,
    ) -> Result<Listing<Issue>, ProviderError>;

    /// One issue with its comments.
    fn issue_show(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
    ) -> Result<IssueDetails, ProviderError>;

    /// The user's open pull requests and issues across the account, for
    /// the inbox.
    fn inbox(&self, session: &Session<'_>, query: &InboxQuery) -> Result<Found, ProviderError>;

    /// The checks of these commits.
    fn ci_status(
        &self,
        session: &Session<'_>,
        target: &Target,
        shas: &[String],
    ) -> Result<Vec<CommitStatus>, ProviderError>;
}

/// Most open pull requests read to find a stack.
pub const MOST_STACKED: usize = 1000;

/// The stack pull request `number` is in, among `pulls` (the open ones
/// of its repository, and it): below it, the open one whose branch is its
/// base, and so on down; above it, the open one whose base is its branch
/// (the lowest number when several are), and so on up. Pull requests
/// from forks are in no stack but their own. `None` when `number` is not
/// among them.
pub fn stack_of(pulls: &[PullRequest], number: u64) -> Option<Stack> {
    let asked = pulls.iter().find(|pull| pull.number == number)?;
    let stacked = |pull: &&PullRequest| {
        pull.state == crate::model::PullState::Open && pull.head_repo.is_none()
    };
    let mut seen = std::collections::BTreeSet::from([asked.number]);
    let mut below = Vec::new();
    let mut at = asked;
    while let Some(next) = pulls
        .iter()
        .filter(stacked)
        .filter(|pull| !seen.contains(&pull.number) && pull.head_ref == at.base_ref)
        .min_by_key(|pull| pull.number)
    {
        seen.insert(next.number);
        below.push(next);
        at = next;
    }
    let mut above = Vec::new();
    let mut at = asked;
    while at.head_repo.is_none() {
        let Some(next) = pulls
            .iter()
            .filter(stacked)
            .filter(|pull| !seen.contains(&pull.number) && pull.base_ref == at.head_ref)
            .min_by_key(|pull| pull.number)
        else {
            break;
        };
        seen.insert(next.number);
        above.push(next);
        at = next;
    }
    let entry = |pull: &PullRequest| crate::model::StackEntry {
        number: pull.number,
        title: pull.title.clone(),
        head_ref: pull.head_ref.clone(),
        base_ref: pull.base_ref.clone(),
        state: pull.state,
    };
    let position = below.len();
    let entries = below
        .into_iter()
        .rev()
        .chain(std::iter::once(asked))
        .chain(above)
        .map(entry)
        .collect();
    Some(Stack { entries, position })
}

/// A walk from one pull request down its stack and up: those under it
/// (nearest first), it, and those over it (nearest first).
type Walked<'a> = (Vec<&'a PullRequest>, &'a PullRequest, Vec<&'a PullRequest>);

/// The pull requests of a repository made ready to find many stacks in:
/// the stacks [`stack_of`] finds, each step of a walk a look-up by branch
/// instead of a pass over every pull request, which for several hundred
/// open ones would hold up the window's thread.
#[derive(Debug)]
pub struct Stacks<'a> {
    /// Each number's pull request: the first given, as [`stack_of`] finds.
    by_number: std::collections::HashMap<u64, &'a PullRequest>,
    /// The open pull requests of the repository's own branches by their
    /// branch, and by the branch they go into, each list lowest number
    /// first and otherwise in the order given.
    by_head: std::collections::HashMap<&'a str, Vec<&'a PullRequest>>,
    by_base: std::collections::HashMap<&'a str, Vec<&'a PullRequest>>,
}

impl<'a> Stacks<'a> {
    /// Index `pulls`: the open ones of a repository, and others.
    pub fn new(pulls: impl IntoIterator<Item = &'a PullRequest>) -> Self {
        let mut by_number = std::collections::HashMap::new();
        let mut by_head: std::collections::HashMap<&'a str, Vec<&'a PullRequest>> =
            std::collections::HashMap::new();
        let mut by_base: std::collections::HashMap<&'a str, Vec<&'a PullRequest>> =
            std::collections::HashMap::new();
        for pull in pulls {
            by_number.entry(pull.number).or_insert(pull);
            if pull.state == crate::model::PullState::Open && pull.head_repo.is_none() {
                by_head
                    .entry(pull.head_ref.as_str())
                    .or_default()
                    .push(pull);
                by_base
                    .entry(pull.base_ref.as_str())
                    .or_default()
                    .push(pull);
            }
        }
        // Stable: of two with one number, the first given comes first, as
        // `min_by_key` takes it.
        for list in by_head.values_mut().chain(by_base.values_mut()) {
            list.sort_by_key(|pull| pull.number);
        }
        Self {
            by_number,
            by_head,
            by_base,
        }
    }

    /// The stack pull request `number` is in, as [`stack_of`] has it.
    #[cfg(test)]
    pub fn stack_of(&self, number: u64) -> Option<Stack> {
        let (below, asked, above) = self.walk(number)?;
        let entry = |pull: &PullRequest| crate::model::StackEntry {
            number: pull.number,
            title: pull.title.clone(),
            head_ref: pull.head_ref.clone(),
            base_ref: pull.base_ref.clone(),
            state: pull.state,
        };
        let position = below.len();
        let entries = below
            .into_iter()
            .rev()
            .chain(std::iter::once(asked))
            .chain(above)
            .map(entry)
            .collect();
        Some(Stack { entries, position })
    }

    /// Where pull request `number` stands in its stack and how many the
    /// stack has, without making the stack's entries: `(position, count)`
    /// of [`Stacks::stack_of`]'s answer.
    pub fn place(&self, number: u64) -> Option<(usize, usize)> {
        let (below, _, above) = self.walk(number)?;
        Some((
            below.len(),
            below.len().saturating_add(above.len()).saturating_add(1),
        ))
    }

    /// The pull requests under `number` (nearest first), it, and those
    /// over it (nearest first).
    fn walk(&self, number: u64) -> Option<Walked<'a>> {
        let asked = *self.by_number.get(&number)?;
        let mut seen = std::collections::BTreeSet::from([asked.number]);
        let first_unseen = |list: Option<&Vec<&'a PullRequest>>,
                            seen: &std::collections::BTreeSet<u64>| {
            list.and_then(|list| {
                list.iter()
                    .copied()
                    .find(|pull| !seen.contains(&pull.number))
            })
        };
        let mut below = Vec::new();
        let mut at = asked;
        while let Some(next) = first_unseen(self.by_head.get(at.base_ref.as_str()), &seen) {
            seen.insert(next.number);
            below.push(next);
            at = next;
        }
        let mut above = Vec::new();
        let mut at = asked;
        while at.head_repo.is_none() {
            let Some(next) = first_unseen(self.by_base.get(at.head_ref.as_str()), &seen) else {
                break;
            };
            seen.insert(next.number);
            above.push(next);
            at = next;
        }
        Some((below, asked, above))
    }
}

/// The provider of a kind.
pub fn of(kind: ProviderKind) -> &'static dyn Provider {
    match kind {
        ProviderKind::Github => &crate::github::Github,
        ProviderKind::Gitlab => &crate::gitlab::Gitlab,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CiState, PullState, User};

    fn pull(number: u64, head: &str, base: &str, state: PullState) -> PullRequest {
        PullRequest {
            repo: "o/r".into(),
            number,
            title: format!("#{number}"),
            state,
            draft: false,
            author: User::default(),
            head_ref: head.into(),
            head_repo: None,
            head_sha: String::new(),
            base_ref: base.into(),
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

    fn numbers(stack: &Stack) -> Vec<u64> {
        stack.entries.iter().map(|entry| entry.number).collect()
    }

    #[test]
    fn a_stack_is_found_down_to_the_base_and_up_to_the_top() {
        let pulls = [
            pull(1, "one", "main", PullState::Open),
            pull(2, "two", "one", PullState::Open),
            pull(3, "three", "two", PullState::Open),
            pull(4, "other", "main", PullState::Open),
            pull(5, "four", "three", PullState::Closed),
            pull(6, "also-on-two", "two", PullState::Open),
        ];
        let stack = stack_of(&pulls, 2).unwrap();
        assert_eq!(numbers(&stack), [1, 2, 3]);
        assert_eq!(stack.position, 1);
        assert_eq!(stack.entries[0].base_ref, "main");
        let top = stack_of(&pulls, 3).unwrap();
        assert_eq!((numbers(&top), top.position), (vec![1, 2, 3], 2));
        let alone = stack_of(&pulls, 4).unwrap();
        assert_eq!((numbers(&alone), alone.position), (vec![4], 0));
        // A closed one is shown where it was asked, and stacks nothing.
        let closed = stack_of(&pulls, 5).unwrap();
        assert_eq!((numbers(&closed), closed.position), (vec![1, 2, 3, 5], 3));
        assert!(stack_of(&pulls, 9).is_none());
    }

    #[test]
    fn a_stack_that_loops_or_comes_from_a_fork_ends() {
        let pulls = [
            pull(1, "a", "b", PullState::Open),
            pull(2, "b", "a", PullState::Open),
        ];
        let stack = stack_of(&pulls, 1).unwrap();
        assert_eq!(numbers(&stack), [2, 1]);
        // A fork's branch is not the repository's: nothing stacks on it,
        // and it stacks on nothing but the repository's own.
        let mut pulls = vec![
            pull(1, "one", "main", PullState::Open),
            pull(2, "two", "one", PullState::Open),
            pull(3, "one", "two", PullState::Open),
        ];
        pulls[1].head_repo = Some("fork/r".into());
        let fork = stack_of(&pulls, 2).unwrap();
        assert_eq!((numbers(&fork), fork.position), (vec![1, 2], 1));
        let on_fork = stack_of(&pulls, 1).unwrap();
        assert_eq!((numbers(&on_fork), on_fork.position), (vec![1], 0));
    }

    /// Pull requests made up from a seed: heads and bases from a few
    /// branches (so that stacks, forks, loops, closed ones and numbers
    /// given twice all come up), in no order.
    fn made_up(seed: u64, count: usize) -> Vec<PullRequest> {
        let mut state = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let mut next = move |below: u64| {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) % below.max(1)
        };
        let branches = (count as u64 / 2).max(3);
        (0..count)
            .map(|_| {
                let number = next(count as u64 + 5) + 1;
                let head = format!("b{}", next(branches));
                let base = match next(4) {
                    0 => "main".to_owned(),
                    _ => format!("b{}", next(branches)),
                };
                let state = match next(10) {
                    0 => PullState::Closed,
                    1 => PullState::Merged,
                    _ => PullState::Open,
                };
                let mut made = pull(number, &head, &base, state);
                if next(12) == 0 {
                    made.head_repo = Some("fork/r".into());
                }
                made
            })
            .collect()
    }

    #[test]
    fn the_indexed_stacks_are_the_stacks_found_one_by_one() {
        for seed in 0..60 {
            for count in [1, 2, 5, 12, 40, 150] {
                let pulls = made_up(seed, count);
                let stacks = Stacks::new(&pulls);
                for number in 0..(count as u64 + 7) {
                    let one = stack_of(&pulls, number);
                    assert_eq!(
                        stacks.stack_of(number),
                        one,
                        "seed {seed}, {count} pull requests, number {number}"
                    );
                    assert_eq!(
                        stacks.place(number),
                        one.map(|stack| (stack.position, stack.entries.len())),
                        "seed {seed}, {count} pull requests, number {number}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_stacks_of_a_thousand_and_their_chains_are_found_at_once() {
        // A chain of a thousand on top of one another, and a thousand on
        // their own: every place is found in a few look-ups each.
        let mut pulls: Vec<PullRequest> = (1..=1000u64)
            .map(|n| {
                let base = if n == 1 {
                    "main".to_owned()
                } else {
                    format!("c{}", n - 1)
                };
                pull(n, &format!("c{n}"), &base, PullState::Open)
            })
            .collect();
        pulls.extend((1001..=2000u64).map(|n| pull(n, &format!("s{n}"), "main", PullState::Open)));
        let started = std::time::Instant::now();
        let stacks = Stacks::new(&pulls);
        let alone: usize = (1001..=2000u64)
            .filter(|n| stacks.place(*n) == Some((0, 1)))
            .count();
        assert_eq!(alone, 1000);
        assert_eq!(stacks.place(1), Some((0, 1000)));
        assert_eq!(stacks.place(1000), Some((999, 1000)));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "{:?}",
            started.elapsed()
        );
    }
}
