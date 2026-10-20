//! What the pull requests section and a pull request's view work out from
//! what a provider answered, with nothing drawn: the groups of the
//! section, the mark of a row, what the search box keeps, how old a time
//! is, the colours of a label, what a custom filter asks of each
//! provider, which remotes could have an account, and which branches have
//! an open pull request. Every text here came from another machine and
//! is only compared and shown.
//!
//! Sources: docs.github.com/en/search-github (searching issues and pull
//! requests: the qualifiers); docs.gitlab.com/api (merge_requests: the
//! parameters of a list); RFC 3339 (times); W3C WCAG 2.1 (relative
//! luminance, for text on a label's colour).

use std::collections::{HashMap, HashSet};

use bitbraid_integrations::{
    model::{
        Account, CiState, Mergeable, ProviderKind, PullRequest, PullState, RepoLink, ReviewState,
        Stack,
    },
    remotes, search,
};

/// A group of the section: the pull requests a filter of the provider
/// answers with, closed and opened from its heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Group {
    /// Opened by the user.
    Mine,
    /// Assigned to the user.
    Assigned,
    /// Waiting for the user's review.
    ReviewRequested,
    /// Every open one.
    All,
    /// One of the filters the user made, by its place in the list.
    Custom(usize),
}

impl Group {
    /// The groups every repository has, in their order.
    pub const BUILT_IN: [Group; 4] = [
        Group::Mine,
        Group::Assigned,
        Group::ReviewRequested,
        Group::All,
    ];

    /// What the group is known by among the closed ones.
    pub fn key(self) -> String {
        match self {
            Group::Mine => "mine".to_owned(),
            Group::Assigned => "assigned".to_owned(),
            Group::ReviewRequested => "review_requested".to_owned(),
            Group::All => "all".to_owned(),
            Group::Custom(at) => format!("custom:{at}"),
        }
    }

    /// What `pr.list` is asked with as its `filter`; a filter of the
    /// user's adds its `query`.
    pub fn filter(self) -> &'static str {
        match self {
            Group::Mine => "mine",
            Group::Assigned => "assigned",
            Group::ReviewRequested => "review_requested",
            Group::All | Group::Custom(_) => "all",
        }
    }

    /// What its heading says.
    pub fn title(self, custom: &[SavedFilter]) -> String {
        match self {
            Group::Mine => say!("pulls-group-mine").to_owned(),
            Group::Assigned => say!("pulls-group-assigned").to_owned(),
            Group::ReviewRequested => say!("pulls-group-review-requested").to_owned(),
            Group::All => say!("pulls-group-all").to_owned(),
            Group::Custom(at) => custom
                .get(at)
                .map(|filter| filter.name.clone())
                .unwrap_or_default(),
        }
    }
}

/// A filter the user made: a name for its group, and a query in GitHub's
/// words (see [`QUALIFIERS`]). Kept with the repository's state.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct SavedFilter {
    /// What its group is called.
    pub name: String,
    /// What it asks for.
    pub query: String,
}

/// The most filters a repository keeps, and the longest name and query.
pub const MOST_FILTERS: usize = 16;
pub const LONGEST_FILTER_NAME: usize = 80;
pub const LONGEST_QUERY: usize = 512;

/// The filters that were remembered, held to a sane size: they come from a
/// file anyone can edit.
pub fn remembered_filters(filters: &[SavedFilter]) -> Vec<SavedFilter> {
    filters
        .iter()
        .filter(|filter| {
            !filter.name.trim().is_empty()
                && filter.name.len() <= LONGEST_FILTER_NAME
                && filter.query.len() <= LONGEST_QUERY
        })
        .take(MOST_FILTERS)
        .cloned()
        .collect()
}

/// What is wrong with a filter that is about to be saved, if anything.
pub fn filter_problem(name: &str, query: &str, others: &[SavedFilter]) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        return Some(say!("pulls-filter-no-name").to_owned());
    }
    if name.chars().count() > LONGEST_FILTER_NAME {
        return Some(say!(
            "pulls-filter-name-too-long",
            most = LONGEST_FILTER_NAME
        ));
    }
    if query.trim().is_empty() {
        return Some(say!("pulls-filter-no-query").to_owned());
    }
    if query.len() > LONGEST_QUERY {
        return Some(say!("pulls-filter-query-too-long", most = LONGEST_QUERY));
    }
    if let Some(why) = query_problem(query) {
        return Some(say!("pulls-filter-query-unreadable", why));
    }
    if others.iter().any(|other| other.name.trim() == name) {
        return Some(say!("pulls-filter-name-taken").to_owned());
    }
    if others.len() >= MOST_FILTERS {
        return Some(say!("pulls-filter-too-many", most = MOST_FILTERS));
    }
    None
}

/// The words a custom filter is said to know: GitHub's search knows these
/// and more; GitLab's lists [`AT_GITLAB`].
pub const QUALIFIERS: [&str; 10] = [
    "author",
    "assignee",
    "review-requested",
    "label",
    "milestone",
    "base",
    "head",
    "draft",
    "review",
    "status",
];

/// The words a list of GitLab's merge requests can be asked by (its
/// author's, assignee's and reviewer's user name, a label, a milestone,
/// the target and source branch, whether it is a draft); any other is
/// refused, and so is leaving out a branch.
const AT_GITLAB: [&str; 10] = [
    "author",
    "assignee",
    "review-requested",
    "reviewer",
    "label",
    "labels",
    "milestone",
    "base",
    "head",
    "draft",
];

/// Why a query cannot be used at all, if it cannot: what the provider
/// commands' reader of queries says of it (control characters, a key
/// without a value, too many terms or choices).
pub fn query_problem(query: &str) -> Option<String> {
    let terms = match search::parse(query, false) {
        Ok(terms) => terms,
        Err(why) => return Some(why),
    };
    search::expand(&terms).err()
}

/// What a provider refuses of a query, a line each: at GitLab the words
/// its lists cannot be asked by, which make its part of the group say so
/// in place of a list. GitHub's search takes its own words and says
/// nothing here. Empty when all of it is used.
pub fn left_out(query: &str, kind: ProviderKind) -> Vec<String> {
    if kind != ProviderKind::Gitlab {
        return Vec::new();
    }
    let Ok(terms) = search::parse(query, false) else {
        return Vec::new();
    };
    let mut said: Vec<String> = Vec::new();
    for term in terms {
        let Some(key) = term.key.as_deref() else {
            continue;
        };
        let line = if !AT_GITLAB.contains(&key) {
            say!("pulls-query-gitlab-unknown-key", key)
        } else if term.negated && matches!(key, "base" | "head") {
            say!("pulls-query-gitlab-negated-branch", key)
        } else {
            continue;
        };
        if !said.contains(&line) {
            said.push(line);
        }
    }
    said
}

/// How a row marks where a pull request stands. A draft is marked as any
/// other, by its checks and reviews.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateMark {
    /// Its checks passed (or it has none) and no review is still awaited.
    Ready,
    /// A check is still to run or running, or a review that was asked
    /// for is still to come.
    Waiting,
    /// A check failed, or a reviewer asked for changes.
    Failing,
    /// Merged.
    Merged,
    /// Closed without being merged.
    Closed,
}

impl StateMark {
    /// What the mark stands for, in words.
    pub fn words(self) -> &'static str {
        match self {
            StateMark::Ready => say!("pulls-mark-ready"),
            StateMark::Waiting => say!("pulls-mark-waiting"),
            StateMark::Failing => say!("pulls-mark-failing"),
            StateMark::Merged => say!("pulls-merged"),
            StateMark::Closed => say!("pulls-closed"),
        }
    }
}

/// Whether the checks of a commit have ended without a failure, or there
/// are none to wait for.
pub fn checks_passed(state: CiState) -> bool {
    matches!(
        state,
        CiState::Success | CiState::Neutral | CiState::Skipped | CiState::None
    )
}

/// Whether the checks of a commit failed.
pub fn checks_failed(state: CiState) -> bool {
    matches!(state, CiState::Failure | CiState::Cancelled)
}

/// Where a pull request stands, as its row marks it.
pub fn state_mark(pull: &PullRequest) -> StateMark {
    match pull.state {
        PullState::Merged => return StateMark::Merged,
        PullState::Closed => return StateMark::Closed,
        PullState::Open => {}
    }
    let asked_changes = pull
        .reviewers
        .iter()
        .any(|reviewer| reviewer.state == ReviewState::ChangesRequested);
    if asked_changes || checks_failed(pull.ci) {
        return StateMark::Failing;
    }
    let awaited = pull
        .reviewers
        .iter()
        .any(|reviewer| reviewer.state == ReviewState::Pending);
    if checks_passed(pull.ci) && !awaited {
        StateMark::Ready
    } else {
        StateMark::Waiting
    }
}

/// How a pull request's number is written: `#12` at GitHub, `!12` for a
/// merge request at GitLab.
pub fn number_text(kind: ProviderKind, number: u64) -> String {
    match kind {
        ProviderKind::Github => format!("#{number}"),
        ProviderKind::Gitlab => format!("!{number}"),
    }
}

/// What a provider calls one, for headings and menus.
pub fn provider_name(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Github => "GitHub",
        ProviderKind::Gitlab => "GitLab",
    }
}

/// A text from a provider as one line to show: control characters and
/// line breaks become spaces, and it is cut at `most` characters with an
/// ellipsis.
pub fn shown_line(text: &str, most: usize) -> String {
    let mut out = String::new();
    for (count, c) in text.chars().enumerate() {
        if count >= most {
            out.push('…');
            break;
        }
        out.push(if c.is_control() { ' ' } else { c });
    }
    out
}

/// Whether a pull request is one the search box keeps: every word that
/// was typed is in its title, its number, its branches or its author's
/// name; upper and lower case are the same.
pub fn matches_search(pull: &PullRequest, search: &str) -> bool {
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let number = pull.number.to_string();
    let haystacks = [
        pull.title.to_lowercase(),
        pull.head_ref.to_lowercase(),
        pull.base_ref.to_lowercase(),
        pull.author.login.to_lowercase(),
        pull.author.name.as_deref().unwrap_or("").to_lowercase(),
    ];
    words.iter().all(|word| {
        let digits = word.trim_start_matches(['#', '!']);
        (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && number == digits)
            || haystacks.iter().any(|hay| hay.contains(word.as_str()))
    })
}

/// The day a milestone is due, as it is shown: GitHub gives a time
/// (`2026-12-31T08:00:00Z`), GitLab a day (`2026-12-31`); none when it is
/// neither.
pub fn due_date(text: &str) -> Option<String> {
    if let Some(time) = parse_time(text) {
        return Some(crate::views::absolute_date(time, 0));
    }
    let text = text.trim();
    let mut parts = text.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    let digits =
        |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    let fits = parts.next().is_none()
        && digits(year, 4)
        && digits(month, 2)
        && digits(day, 2)
        && (1..=12).contains(&month.parse::<u32>().ok()?)
        && (1..=31).contains(&day.parse::<u32>().ok()?);
    fits.then(|| text.to_owned())
}

/// A time as RFC 3339 writes it (`2026-10-12T08:30:00Z`,
/// `2026-10-12T10:30:00.123+02:00`), in seconds since 1970; none when it
/// is not one.
pub fn parse_time(text: &str) -> Option<i64> {
    let text = text.trim();
    let date = text.get(..10)?;
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let day: u32 = parts.next()?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || !(1..=9999).contains(&year) {
        return None;
    }
    let rest = text.get(10..)?;
    let rest = rest.strip_prefix(['T', 't', ' '])?;
    let clock = rest.get(..8)?;
    let mut parts = clock.split(':');
    let hour: i64 = parts.next()?.parse().ok()?;
    let minute: i64 = parts.next()?.parse().ok()?;
    let second: i64 = parts.next()?.parse().ok()?;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut zone = rest.get(8..)?;
    if let Some(fraction) = zone.strip_prefix('.') {
        let digits = fraction.chars().take_while(char::is_ascii_digit).count();
        zone = fraction.get(digits..)?;
    }
    let offset: i64 = match zone {
        "Z" | "z" => 0,
        zone => {
            let sign = match zone.get(..1)? {
                "+" => 1,
                "-" => -1,
                _ => return None,
            };
            let hours: i64 = zone.get(1..3)?.parse().ok()?;
            let minutes: i64 = zone.get(4..6)?.parse().ok()?;
            if zone.get(3..4)? != ":" || zone.len() != 6 || hours > 23 || minutes > 59 {
                return None;
            }
            sign * (hours * 3600 + minutes * 60)
        }
    };
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

// `days_from_civil` of "chrono-Compatible Low-Level Date Algorithms",
// the other way round from `views::civil_from_days`.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year.rem_euclid(400);
    let month = i64::from(month);
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// How long ago a time of a provider's was, as the rest of the window
/// says it ("3 h ago"); empty when it is no time.
pub fn age(text: &str, now: i64) -> String {
    parse_time(text)
        .map(|then| crate::views::relative_time(then, now))
        .unwrap_or_default()
}

/// What a view says of what it shows when that was kept from before,
/// because the provider could not be reached: "Offline: as read 1 h
/// ago". None when it was read from the provider.
pub fn offline_said(
    freshness: &bitbraid_integrations::model::Freshness,
    now: i64,
) -> Option<String> {
    if !freshness.offline {
        return None;
    }
    let when = freshness
        .fetched
        .as_deref()
        .map(|time| age(time, now))
        .filter(|said| !said.is_empty());
    Some(match when {
        Some(when) => say!("pulls-offline-as-read", when),
        None => say!("pulls-offline-as-read-before").to_owned(),
    })
}

/// A label's colour (six hexadecimal digits, as the provider gives it,
/// with or without `#`) and the colour of text that reads on it: black
/// on a light colour, white on a dark one. None when it is no colour.
pub fn label_colours(colour: &str) -> Option<(u32, u32)> {
    let hex = colour.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let channel = |shift: u32| {
        let c = f64::from((value >> shift) & 0xff) / 255.0;
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0);
    // Black on it reads better than white once it is lighter than the
    // middle of their contrast ratios.
    let text = if luminance > 0.179 {
        0x1a1a1a
    } else {
        0xffffff
    };
    Some((value, text))
}

/// What the merge box says about whether the pull request merges, and
/// whether that is good news.
pub fn merge_line(mergeable: Mergeable, base: &str) -> (String, Option<bool>) {
    match mergeable {
        Mergeable::Clean => (say!("pulls-merges-cleanly", base), Some(true)),
        Mergeable::Conflicts => (say!("pulls-merge-conflicts", base), Some(false)),
        Mergeable::Blocked => (say!("pulls-merge-blocked").to_owned(), Some(false)),
        Mergeable::Unknown => (say!("pulls-merge-unknown").to_owned(), None),
    }
}

/// The checks of a commit summed up in a line.
pub fn checks_summary(states: &[CiState]) -> String {
    if states.is_empty() {
        return say!("pulls-checks-none").to_owned();
    }
    let count = |wanted: fn(CiState) -> bool| states.iter().filter(|state| wanted(**state)).count();
    let failed = count(checks_failed);
    let running = count(|state| matches!(state, CiState::Pending | CiState::Running));
    let passed = count(|state| state == CiState::Success);
    let all = states.len();
    if failed == 0 && running == 0 {
        if passed == all {
            return say!("pulls-checks-all-passed", all);
        }
        return say!("pulls-checks-some-said-nothing", all, passed);
    }
    let mut parts = Vec::new();
    if failed > 0 {
        parts.push(say!("pulls-checks-failed", failed));
    }
    if running > 0 {
        parts.push(say!("pulls-checks-running", running));
    }
    if passed > 0 {
        parts.push(say!("pulls-checks-passed", passed));
    }
    say!("pulls-checks-counted", all, parts = parts.join(", "))
}

/// What one check's state is, in a word.
pub fn ci_word(state: CiState) -> &'static str {
    match state {
        CiState::None => say!("pulls-ci-unknown"),
        CiState::Pending => say!("pulls-ci-waiting"),
        CiState::Running => say!("pulls-ci-running"),
        CiState::Success => say!("pulls-ci-passed"),
        CiState::Failure => say!("pulls-ci-failed"),
        CiState::Cancelled => say!("pulls-ci-cancelled"),
        CiState::Skipped => say!("pulls-ci-skipped"),
        CiState::Neutral => say!("pulls-ci-neutral"),
    }
}

/// What a reviewer's state is, in words.
pub fn review_word(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Approved => say!("pulls-review-approved"),
        ReviewState::ChangesRequested => say!("pulls-review-changes-requested"),
        ReviewState::Commented => say!("pulls-review-commented"),
        ReviewState::Pending => say!("pulls-review-pending"),
        ReviewState::Dismissed => say!("pulls-review-dismissed"),
    }
}

/// The host of a remote's address: `github.com` of
/// `git@github.com:owner/name.git`, `https://github.com/owner/name`,
/// `ssh://git@gitlab.example.org:2222/group/name.git`. None for a path on
/// this machine.
pub fn host_of(url: &str) -> Option<String> {
    let url = url.trim();
    let host = if let Some((scheme, rest)) = url.split_once("://") {
        if scheme.eq_ignore_ascii_case("file") {
            return None;
        }
        let authority = rest.split('/').next()?;
        let authority = authority.rsplit('@').next()?;
        // An address in brackets keeps its colons.
        if let Some(inner) = authority.strip_prefix('[') {
            inner.split(']').next()?
        } else {
            authority.split(':').next()?
        }
    } else {
        // `user@host:path`, scp's way, with no slash before the colon.
        let (before, _) = url.split_once(':')?;
        if before.contains('/') || before.is_empty() {
            return None;
        }
        before.rsplit('@').next()?
    };
    let host = host.trim().to_lowercase();
    (!host.is_empty()).then_some(host)
}

/// The remotes whose host is a provider's and that no account can be
/// asked about: those the section invites to connect an account for, by
/// host, each once. A host is taken for a provider's when it is
/// github.com or gitlab.com, or its name says github or gitlab.
pub fn unconnected_hosts(
    remotes: &[(String, String)],
    links: &[RepoLink],
    accounts: &[Account],
) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for (name, url) in remotes {
        if links.iter().any(|link| &link.remote == name) {
            continue;
        }
        let Some(address) = remotes::parse(url) else {
            continue;
        };
        let host = address.host;
        let provider_like = host == "github.com"
            || host == "gitlab.com"
            || host.contains("github")
            || host.contains("gitlab");
        let known = accounts
            .iter()
            .any(|account| remotes::same_site(account, &host, &[]));
        if provider_like && !known && !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
}

/// What a branch's label in the graph says of its open pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullMark {
    /// The provider.
    pub kind: ProviderKind,
    pub number: u64,
    /// Its title, as one line.
    pub title: String,
    /// Where it stands.
    pub mark: StateMark,
    /// Its place in a stack of pull requests, counted from the one on the
    /// base branch, and how many the stack has; none when it is alone.
    pub stack: Option<(usize, usize)>,
    /// Its page at the provider.
    pub web_url: String,
}

impl PullMark {
    /// What the mark's hover says.
    pub fn words(&self) -> String {
        let mut words = format!(
            "{} {}: {}",
            number_text(self.kind, self.number),
            self.title,
            self.mark.words()
        );
        if let Some((place, of)) = self.stack {
            words.push_str(" · ");
            words.push_str(&say!("pulls-mark-stack-place", place, of));
        }
        words
    }
}

/// Where a stack has a pull request: its place counted from one at the
/// bottom, and how many there are; none when it stands alone.
pub fn stack_place(stack: &Stack) -> Option<(usize, usize)> {
    place_in_stack(stack.position, stack.entries.len())
}

/// [`stack_place`] of a stack of `of` with the one asked at `position`.
fn place_in_stack(position: usize, of: usize) -> Option<(usize, usize)> {
    (of > 1 && position < of).then_some((position.saturating_add(1), of))
}

/// For each branch of the repository that has an open pull request in the
/// lists held, that pull request, with its place in a stack: by the
/// branch's name. A pull request from a fork names a branch of the fork,
/// and marks nothing here. The stacks are made of every list of a remote
/// at once, since one group may hold only part of one.
pub fn branch_marks<'a>(
    lists: impl Iterator<Item = (&'a RepoLink, &'a [PullRequest])>,
) -> HashMap<String, PullMark> {
    // Each remote's pull requests, each once (the first held of a
    // number), by reference: several hundred open ones are usual, and
    // copying them whole for each of them would be slow.
    let mut by_link: Vec<(&RepoLink, Vec<&PullRequest>, HashSet<u64>)> = Vec::new();
    for (link, pulls) in lists {
        let at = match by_link.iter().position(|(known, _, _)| *known == link) {
            Some(at) => at,
            None => {
                by_link.push((link, Vec::new(), HashSet::new()));
                by_link.len().saturating_sub(1)
            }
        };
        let Some((_, held, numbers)) = by_link.get_mut(at) else {
            continue;
        };
        for pull in pulls {
            if numbers.insert(pull.number) {
                held.push(pull);
            }
        }
    }
    let mut marks: HashMap<String, PullMark> = HashMap::new();
    for (link, pulls, _) in &mut by_link {
        pulls.sort_by_key(|pull| pull.number);
        let stacks = bitbraid_integrations::provider::Stacks::new(pulls.iter().copied());
        for pull in pulls.iter() {
            if pull.state != PullState::Open || pull.head_repo.is_some() {
                continue;
            }
            if marks.contains_key(&pull.head_ref) {
                continue;
            }
            let stack = stacks
                .place(pull.number)
                .and_then(|(position, of)| place_in_stack(position, of));
            marks.insert(
                pull.head_ref.clone(),
                PullMark {
                    kind: link.kind,
                    number: pull.number,
                    title: shown_line(&pull.title, 200),
                    mark: state_mark(pull),
                    stack,
                    web_url: pull.web_url.clone(),
                },
            );
        }
    }
    marks
}

/// Whether an address from a provider may be opened in the system's
/// browser: `https`, or `http` to this machine (a GitLab of one's own run
/// here). Anything else (`javascript:`, `file:`, a program's own scheme)
/// is not opened.
pub fn may_open(url: &str) -> bool {
    let url = url.trim();
    if url.chars().any(char::is_control) {
        return false;
    }
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    if scheme.eq_ignore_ascii_case("https") {
        return !rest.is_empty();
    }
    if scheme.eq_ignore_ascii_case("http") {
        return matches!(
            host_of(url).as_deref(),
            Some("127.0.0.1" | "localhost" | "::1")
        );
    }
    false
}

#[cfg(test)]
mod tests {
    use bitbraid_integrations::model::{Reviewer, User};

    use super::*;

    #[test]
    fn a_due_day_is_shown_from_a_time_or_a_day() {
        assert_eq!(
            due_date("2026-12-31T08:00:00Z").as_deref(),
            Some("2026-12-31")
        );
        assert_eq!(due_date("2026-12-31").as_deref(), Some("2026-12-31"));
        for odd in [
            "",
            "2026-13-01",
            "2026-12-32",
            "26-12-31",
            "2026-12-31-1",
            "２０２６-12-31",
        ] {
            assert_eq!(due_date(odd), None, "{odd}");
        }
    }

    fn pull(number: u64, title: &str) -> PullRequest {
        PullRequest {
            repo: "octo/tools".to_owned(),
            number,
            title: title.to_owned(),
            state: PullState::Open,
            draft: false,
            author: User {
                login: "octo".to_owned(),
                name: Some("Octo Cat".to_owned()),
                ..User::default()
            },
            head_ref: "feature/lexer".to_owned(),
            head_repo: None,
            head_sha: "a".repeat(40),
            base_ref: "main".to_owned(),
            created: "2026-10-10T08:00:00Z".to_owned(),
            updated: "2026-10-12T08:00:00Z".to_owned(),
            labels: Vec::new(),
            reviewers: Vec::new(),
            assignees: Vec::new(),
            ci: CiState::None,
            comments: None,
            web_url: "https://github.com/octo/tools/pull/1".to_owned(),
        }
    }

    fn reviewer(state: ReviewState) -> Reviewer {
        Reviewer {
            user: User::default(),
            state,
        }
    }

    #[test]
    fn a_row_is_marked_by_its_checks_and_reviews() {
        // Nothing to wait for is a tick, a review asked for and not given
        // yet or a check still running is a hollow circle, a failed check
        // or changes asked for a cross; a draft is marked the same way.
        let mut open = pull(1, "x");
        assert_eq!(state_mark(&open), StateMark::Ready);
        open.reviewers = vec![reviewer(ReviewState::Pending)];
        assert_eq!(state_mark(&open), StateMark::Waiting);
        open.reviewers = vec![reviewer(ReviewState::Commented)];
        open.ci = CiState::Success;
        assert_eq!(state_mark(&open), StateMark::Ready, "commented is answered");
        open.reviewers = vec![reviewer(ReviewState::Approved)];
        assert_eq!(state_mark(&open), StateMark::Ready);
        open.ci = CiState::Running;
        assert_eq!(state_mark(&open), StateMark::Waiting);
        open.ci = CiState::Pending;
        open.draft = true;
        assert_eq!(
            state_mark(&open),
            StateMark::Waiting,
            "a draft as any other"
        );
        open.draft = false;
        open.ci = CiState::Failure;
        assert_eq!(state_mark(&open), StateMark::Failing);
        open.ci = CiState::Success;
        open.reviewers.push(reviewer(ReviewState::ChangesRequested));
        assert_eq!(state_mark(&open), StateMark::Failing);
        open.state = PullState::Merged;
        assert_eq!(state_mark(&open), StateMark::Merged);
        open.state = PullState::Closed;
        assert_eq!(state_mark(&open), StateMark::Closed);
    }

    #[test]
    fn the_search_box_keeps_what_has_every_word() {
        let found = pull(42, "Teach the lexer quotes");
        for kept in [
            "",
            "lexer",
            "LEXER quotes",
            "#42",
            "!42",
            "42",
            "octo",
            "cat",
            "main",
            "feature/",
        ] {
            assert!(matches_search(&found, kept), "{kept}");
        }
        for gone in ["parser", "#4", "lexer parser", "43"] {
            assert!(!matches_search(&found, gone), "{gone}");
        }
    }

    #[test]
    fn times_are_read_as_rfc_3339_writes_them() {
        assert_eq!(parse_time("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_time("2026-10-12T08:30:00Z"), Some(1_791_793_800));
        assert_eq!(
            parse_time("2026-10-12T10:30:00.123+02:00"),
            Some(1_791_793_800)
        );
        assert_eq!(parse_time("2026-10-12T06:30:00-02:00"), Some(1_791_793_800));
        for odd in [
            "",
            "2026",
            "2026-13-01T00:00:00Z",
            "2026-10-12",
            "2026-10-12T08:30:00",
            "2026-10-12T08:30:00+0200",
            "2026-10-12T25:00:00Z",
            "x026-10-12T08:30:00Z",
            "2026-10-12T08:30:00Zjunk",
            "2026-10-é2T08:30:00Z",
        ] {
            assert_eq!(parse_time(odd), None, "{odd}");
        }
        assert_eq!(age("2026-10-12T08:30:00Z", 1_791_793_800 + 7200), "2 h ago");
        assert_eq!(age("not a time", 0), "");
    }

    #[test]
    fn text_on_a_label_reads_on_its_colour() {
        assert_eq!(label_colours("ffffff"), Some((0xffffff, 0x1a1a1a)));
        assert_eq!(label_colours("#000000"), Some((0, 0xffffff)));
        assert_eq!(label_colours("b60205"), Some((0xb60205, 0xffffff)));
        assert_eq!(label_colours("fbca04"), Some((0xfbca04, 0x1a1a1a)));
        for odd in ["", "fff", "gggggg", "1234567", "#12345"] {
            assert_eq!(label_colours(odd), None, "{odd}");
        }
    }

    #[test]
    fn what_a_provider_refuses_of_a_query_is_said() {
        let query = "author:octo review:approved status:success label:a,b -base:main colour:red";
        assert!(left_out(query, ProviderKind::Github).is_empty());
        assert_eq!(
            left_out(query, ProviderKind::Gitlab),
            vec![
                "review: is not a word GitLab's lists take",
                "status: is not a word GitLab's lists take",
                "-base: GitLab cannot leave a branch out",
                "colour: is not a word GitLab's lists take",
            ]
        );
        assert!(left_out("author:@me -draft:true milestone:v1", ProviderKind::Gitlab).is_empty());
        assert!(query_problem("author:octo -label:wip,blocked").is_none());
        assert!(query_problem("author:").is_some());
        assert!(query_problem("a\u{7}b").is_some());
    }

    #[test]
    fn a_filter_is_saved_only_when_it_is_whole_and_new() {
        let saved = vec![SavedFilter {
            name: "Bugs".to_owned(),
            query: "label:bug".to_owned(),
        }];
        assert!(filter_problem("Mine to merge", "author:@me", &saved).is_none());
        assert!(filter_problem(" ", "label:x", &saved).is_some());
        assert!(filter_problem("New", "  ", &saved).is_some());
        assert!(filter_problem("Bugs", "label:y", &saved).is_some());
        assert!(filter_problem(&"n".repeat(81), "label:y", &saved).is_some());
        assert!(filter_problem("Long", &"q".repeat(513), &saved).is_some());
        let full = vec![saved[0].clone(); MOST_FILTERS];
        assert!(filter_problem("One more", "label:z", &full).is_some());
        let odd = vec![
            SavedFilter::default(),
            SavedFilter {
                name: "x".repeat(81),
                query: String::new(),
            },
            saved[0].clone(),
        ];
        assert_eq!(remembered_filters(&odd), saved);
        assert_eq!(
            remembered_filters(&vec![saved[0].clone(); 40]).len(),
            MOST_FILTERS
        );
    }

    #[test]
    fn a_remotes_host_is_read_from_every_kind_of_address() {
        for (url, host) in [
            ("git@github.com:octo/tools.git", Some("github.com")),
            ("https://github.com/octo/tools", Some("github.com")),
            (
                "https://user:pw@GitLab.example.org/group/sub/x.git",
                Some("gitlab.example.org"),
            ),
            ("ssh://git@127.0.0.1:2224/group/x.git", Some("127.0.0.1")),
            ("ssh://git@[::1]:22/x.git", Some("::1")),
            ("/srv/git/x.git", None),
            ("./x", None),
            ("file:///srv/x.git", None),
            ("", None),
        ] {
            assert_eq!(host_of(url).as_deref(), host, "{url}");
        }
    }

    #[test]
    fn a_remote_at_a_provider_without_an_account_is_invited_once() {
        let remotes = vec![
            (
                "origin".to_owned(),
                "git@github.com:octo/tools.git".to_owned(),
            ),
            ("fork".to_owned(), "https://github.com/me/tools".to_owned()),
            (
                "lab".to_owned(),
                "https://gitlab.example.org/g/x".to_owned(),
            ),
            ("local".to_owned(), "/srv/x.git".to_owned()),
            ("other".to_owned(), "https://example.org/x.git".to_owned()),
        ];
        assert_eq!(
            unconnected_hosts(&remotes, &[], &[]),
            vec!["github.com", "gitlab.example.org"]
        );
        let link = RepoLink {
            remote: "lab".to_owned(),
            account: "gitlab:gitlab.example.org:me".to_owned(),
            kind: ProviderKind::Gitlab,
            owner: "g".to_owned(),
            name: "x".to_owned(),
        };
        assert_eq!(
            unconnected_hosts(&remotes, &[link], &[]),
            vec!["github.com"]
        );
    }

    #[test]
    fn a_branch_with_an_open_pull_request_is_marked_once() {
        let link = RepoLink {
            remote: "origin".to_owned(),
            account: "github:github.com:octo".to_owned(),
            kind: ProviderKind::Github,
            owner: "octo".to_owned(),
            name: "tools".to_owned(),
        };
        let mut from_fork = pull(3, "From a fork");
        from_fork.head_repo = Some("me/tools".to_owned());
        from_fork.head_ref = "fork-branch".to_owned();
        let mut merged = pull(4, "Merged");
        merged.state = PullState::Merged;
        merged.head_ref = "done".to_owned();
        let pulls = vec![pull(1, "First\nline"), pull(2, "Second"), from_fork, merged];
        let marks = branch_marks([(&link, pulls.as_slice())].into_iter());
        assert_eq!(marks.len(), 1);
        let mark = marks.get("feature/lexer").unwrap();
        assert_eq!(mark.number, 1);
        assert_eq!(
            mark.words(),
            "#1 First line: Checks passed, no review awaited"
        );
    }

    #[test]
    fn a_branch_in_a_stack_says_its_place_whatever_list_it_came_from() {
        let link = RepoLink {
            remote: "origin".to_owned(),
            account: "github:github.com:octo".to_owned(),
            kind: ProviderKind::Github,
            owner: "octo".to_owned(),
            name: "tools".to_owned(),
        };
        let mut bottom = pull(5, "Lexer");
        bottom.head_ref = "lexer".to_owned();
        let mut middle = pull(6, "Parser");
        middle.head_ref = "parser".to_owned();
        middle.base_ref = "lexer".to_owned();
        let mut top = pull(9, "Printer");
        top.head_ref = "printer".to_owned();
        top.base_ref = "parser".to_owned();
        // The group of mine holds the top one alone; all of them, all.
        let mine = vec![top.clone()];
        let all = vec![top, middle, bottom];
        let marks = branch_marks([(&link, mine.as_slice()), (&link, all.as_slice())].into_iter());
        let place = |branch: &str| marks.get(branch).and_then(|mark| mark.stack);
        assert_eq!(place("lexer"), Some((1, 3)));
        assert_eq!(place("parser"), Some((2, 3)));
        assert_eq!(place("printer"), Some((3, 3)));
        assert!(
            marks
                .get("parser")
                .unwrap()
                .words()
                .ends_with("· 2/3 in a stack")
        );
        let alone = Stack {
            entries: Vec::new(),
            position: 0,
        };
        assert_eq!(stack_place(&alone), None);
    }

    #[test]
    fn the_marks_of_thousands_of_pull_requests_are_worked_out_at_once() {
        // A project can have hundreds of open merge requests, marked on
        // the window's thread for every list that is answered, so none of
        // them may copy all of them or walk its stack through all of them.
        // Here three thousand, in stacks of three, with a list of some of
        // them given twice, mark as each stack found alone has it.
        let link = RepoLink {
            remote: "origin".to_owned(),
            account: "gitlab:gitlab.com:octo".to_owned(),
            kind: ProviderKind::Gitlab,
            owner: "octo".to_owned(),
            name: "tools".to_owned(),
        };
        let all: Vec<PullRequest> = (1..=3000u64)
            .map(|n| {
                let mut made = pull(n, &format!("Change {n}"));
                made.head_ref = format!("topic/{n}");
                made.base_ref = if n % 3 == 1 {
                    "main".to_owned()
                } else {
                    format!("topic/{}", n - 1)
                };
                made
            })
            .collect();
        let mine: Vec<PullRequest> = all.iter().step_by(7).cloned().collect();
        let started = std::time::Instant::now();
        let marks = branch_marks([(&link, mine.as_slice()), (&link, all.as_slice())].into_iter());
        let took = started.elapsed();
        assert_eq!(marks.len(), 3000);
        for n in [1u64, 2, 3, 1500, 2998, 2999, 3000] {
            let found = bitbraid_integrations::provider::stack_of(&all, n)
                .as_ref()
                .and_then(stack_place);
            assert_eq!(
                marks.get(&format!("topic/{n}")).and_then(|mark| mark.stack),
                found,
                "#{n}"
            );
        }
        assert_eq!(
            marks.get("topic/2").and_then(|mark| mark.stack),
            Some((2, 3))
        );
        // Milliseconds; copying all of them for each of them would take
        // seconds.
        assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    }

    #[test]
    fn only_safe_addresses_are_opened() {
        for yes in [
            "https://github.com/octo/tools/pull/1",
            "HTTPS://example.org",
            "http://127.0.0.1:8929/g/x",
            "http://localhost/x",
        ] {
            assert!(may_open(yes), "{yes}");
        }
        for no in [
            "http://example.org",
            "javascript:alert(1)",
            "file:///etc/passwd",
            "https://",
            "vscode://open",
            "https://exa\nmple.org",
            "",
        ] {
            assert!(!may_open(no), "{no}");
        }
    }

    #[test]
    fn lines_and_summaries_are_short_and_plain() {
        assert_eq!(shown_line("a\tb\nc", 10), "a b c");
        assert_eq!(shown_line("abcdef", 3), "abc…");
        assert_eq!(number_text(ProviderKind::Gitlab, 7), "!7");
        assert_eq!(checks_summary(&[]), "No checks");
        assert_eq!(
            checks_summary(&[CiState::Success, CiState::Success]),
            "All 2 checks passed"
        );
        assert_eq!(
            checks_summary(&[CiState::Success, CiState::Failure, CiState::Running]),
            "3 checks: 1 failed, 1 running or waiting, 1 passed"
        );
        assert_eq!(merge_line(Mergeable::Conflicts, "main").1, Some(false));
    }
}
