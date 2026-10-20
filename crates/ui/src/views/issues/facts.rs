//! What the views of issues work out without drawing anything: the
//! groups of the ISSUES section, what its search box keeps, which
//! repository an issue key in a text names, the branches made for an
//! issue, the key a commit message is given from its branch, and the
//! words of a custom query at each provider.
//!
//! Sources: the commands' descriptions in
//! `crates/integrations/src/issue_cmds.rs`; docs.github.com "Autolinked
//! references and URLs"; docs.gitlab.com "GitLab Flavored Markdown";
//! git-interpret-trailers(1).

use std::ops::Range;

use bitbraid_git::issue_keys::{self, IssueKey};
use bitbraid_integrations::model::{Issue, IssueState, ProviderKind, RepoLink};

/// A group of the section, closed and opened from its heading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum IssueGroup {
    /// Every open issue.
    Open,
    /// The open issues assigned to the user.
    Mine,
}

impl IssueGroup {
    /// The groups, in their order.
    pub const ALL: [IssueGroup; 2] = [IssueGroup::Open, IssueGroup::Mine];

    /// What the group is known by among the closed ones, and in tests.
    pub fn key(self) -> &'static str {
        match self {
            IssueGroup::Open => "open",
            IssueGroup::Mine => "mine",
        }
    }

    /// What `issue.list` is asked with as its `filter`.
    pub fn filter(self) -> &'static str {
        match self {
            IssueGroup::Open => "all",
            IssueGroup::Mine => "assigned",
        }
    }

    /// What its heading says.
    pub fn title(self) -> &'static str {
        match self {
            IssueGroup::Open => say!("issues-group-open"),
            IssueGroup::Mine => say!("issues-group-mine"),
        }
    }
}

/// How an issue's number is written, at both providers.
pub fn number_text(number: u64) -> String {
    format!("#{number}")
}

/// A repository at a provider as `owner/name`.
pub fn repo_name(link: &RepoLink) -> String {
    format!("{}/{}", link.owner, link.name)
}

/// Whether an issue is one the search box keeps: every word that was
/// typed is in its title, its number, its labels or its author's name;
/// upper and lower case are the same.
pub fn matches_search(issue: &Issue, search: &str) -> bool {
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    if words.is_empty() {
        return true;
    }
    let number = issue.number.to_string();
    let mut haystacks = vec![
        issue.title.to_lowercase(),
        issue.author.login.to_lowercase(),
        issue.author.name.as_deref().unwrap_or("").to_lowercase(),
    ];
    haystacks.extend(issue.labels.iter().map(|label| label.name.to_lowercase()));
    words.iter().all(|word| {
        let digits = word.trim_start_matches('#');
        (!digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) && number == digits)
            || haystacks.iter().any(|hay| hay.contains(word.as_str()))
    })
}

/// The repository whose issues a bare key (`#12`) names: the remote
/// `origin` when it is linked to an account, else the first linked one.
pub fn first_link(links: &[RepoLink]) -> Option<&RepoLink> {
    links
        .iter()
        .find(|link| link.remote == "origin")
        .or_else(|| links.first())
}

/// The repository a key names: the open repository's own for `#12`; for
/// `owner/name#12` the linked remote of that path, letters in any case,
/// or else that path at the account of the first linked remote. None
/// without a linked remote, or for a path its provider cannot have
/// (GitHub has no groups).
pub fn link_for_key(links: &[RepoLink], key: &IssueKey) -> Option<RepoLink> {
    let first = first_link(links)?;
    let Some(path) = key.repo.as_deref() else {
        return Some(first.clone());
    };
    if let Some(linked) = links
        .iter()
        .find(|link| repo_name(link).eq_ignore_ascii_case(path))
    {
        return Some(linked.clone());
    }
    let (owner, name) = path.rsplit_once('/')?;
    if first.kind == ProviderKind::Github && owner.contains('/') {
        return None;
    }
    Some(RepoLink {
        remote: String::new(),
        account: first.account.clone(),
        kind: first.kind,
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// An issue key in a text, drawn as a link to its issue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyLink {
    /// Where the key is in the text, in bytes.
    pub range: Range<usize>,
    /// The repository of its issue.
    pub link: RepoLink,
    /// The issue's number.
    pub number: u64,
}

/// The keys of a text that name an issue of a repository the open one is
/// linked to (or of another repository at the same account), in order;
/// none without a linked remote.
pub fn key_links(text: &str, links: &[RepoLink]) -> Vec<KeyLink> {
    if links.is_empty() || !text.contains('#') {
        return Vec::new();
    }
    issue_keys::keys_in(text)
        .into_iter()
        .filter_map(|found| {
            let link = link_for_key(links, &found.key)?;
            Some(KeyLink {
                range: found.start..found.end,
                link,
                number: found.key.number,
            })
        })
        .collect()
}

/// The issue a branch's name holds, as a link: of the open repository's
/// first linked remote. None when the name holds none or nothing is
/// linked.
pub fn branch_issue(links: &[RepoLink], branch: &str) -> Option<(RepoLink, u64)> {
    let number = issue_keys::branch_number(branch)?;
    Some((first_link(links)?.clone(), number))
}

/// The same, with where the key stands in `shown`, the end of the
/// branch's name that a row shows (its last parts, in a folder): the bytes
/// of the word that holds the number, when they are in what is shown.
pub fn branch_issue_in(
    links: &[RepoLink],
    branch: &str,
    shown: &str,
) -> Option<(RepoLink, u64, Option<std::ops::Range<usize>>)> {
    let (number, at) = issue_keys::branch_key(branch)?;
    let link = first_link(links)?.clone();
    let in_shown = branch
        .len()
        .checked_sub(shown.len())
        .filter(|cut| branch.ends_with(shown) && at.start >= *cut)
        .map(|cut| at.start.saturating_sub(cut)..at.end.saturating_sub(cut))
        .filter(|range| shown.get(range.clone()).is_some());
    Some((link, number, in_shown))
}

/// The branches among `names` that were made for issue `number`: whose
/// name holds that number, by the rule of `issue.key_from_branch`.
pub fn branches_for(names: &[String], number: u64) -> Vec<String> {
    names
        .iter()
        .filter(|name| issue_keys::branch_number(name) == Some(number))
        .cloned()
        .collect()
}

/// The name the naming bar offers for a branch for an issue: what the
/// template makes of its number and title, or the number alone when the
/// template makes nothing git would take.
pub fn offered_branch_name(template: &str, issue: &Issue) -> String {
    let template = Some(template).filter(|template| !template.trim().is_empty());
    issue_keys::branch_for_issue(template, issue.number, &issue.title)
        .or_else(|_| issue_keys::branch_for_issue(None, issue.number, &issue.title))
        .unwrap_or_else(|_| issue.number.to_string())
}

/// The key of the trailer that names a branch's issue in a commit
/// message: it refers to the issue without closing it, at either
/// provider.
// not shown: written into the commit message, as git's trailers are
pub const TRAILER_KEY: &str = "Refs";

/// Whether a message already names issue `number` of the open
/// repository: as `#12`, wherever it stands, outside code.
pub fn names_issue(message: &str, number: u64) -> bool {
    issue_keys::keys_in(message)
        .iter()
        .any(|found| found.key.repo.is_none() && found.key.number == number)
}

/// The first lines of a description, for a card: at most `lines` of the
/// lines that say something, each at most `width` characters, Markdown's
/// marks of headings, quotes, code and emphasis left out.
pub fn first_lines(body: &str, lines: usize, width: usize) -> Vec<String> {
    body.lines()
        .map(|line| line.trim().trim_start_matches(['#', '>']).trim())
        .filter(|line| !line.is_empty() && !line.starts_with("```") && !line.starts_with("<!--"))
        .take(lines)
        .map(|line| {
            let plain = line
                .replace("**", "")
                .replace("__", "")
                .replace("~~", "")
                .replace('`', "");
            crate::views::pulls::facts::shown_line(&plain, width)
        })
        .collect()
}

/// The word of where an issue stands.
pub fn state_word(state: IssueState) -> &'static str {
    match state {
        IssueState::Open => say!("issues-state-open"),
        IssueState::Closed => say!("issues-state-closed"),
    }
}

/// What the filter's query may say at a provider, for its tooltip.
pub fn query_help(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Github => say!("issues-query-help-github"),
        ProviderKind::Gitlab => say!("issues-query-help-gitlab"),
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` is not here, but the views' modules
    // are kept alike.
    use bitbraid_git::issue_keys::IssueKey;
    use bitbraid_integrations::model::{Issue, IssueState, Label, ProviderKind, RepoLink, User};

    use super::{
        branch_issue, branches_for, first_lines, first_link, key_links, link_for_key,
        matches_search, names_issue, offered_branch_name,
    };

    fn link(remote: &str, kind: ProviderKind, owner: &str, name: &str) -> RepoLink {
        RepoLink {
            remote: remote.to_owned(),
            account: format!("{kind:?}:me"),
            kind,
            owner: owner.to_owned(),
            name: name.to_owned(),
        }
    }

    fn issue(number: u64, title: &str) -> Issue {
        Issue {
            repo: "octo/tools".to_owned(),
            number,
            title: title.to_owned(),
            state: IssueState::Open,
            author: User {
                login: "ada".to_owned(),
                ..User::default()
            },
            assignees: Vec::new(),
            labels: vec![Label {
                name: "bug".to_owned(),
                color: None,
                description: None,
            }],
            milestone: None,
            created: String::new(),
            updated: String::new(),
            comments: 0,
            web_url: String::new(),
        }
    }

    #[test]
    fn the_search_box_keeps_titles_numbers_labels_and_authors() {
        let lexer = issue(12, "The lexer drops quotes");
        assert!(matches_search(&lexer, ""));
        assert!(matches_search(&lexer, "LEXER quotes"));
        assert!(matches_search(&lexer, "#12"));
        assert!(matches_search(&lexer, "12"));
        assert!(matches_search(&lexer, "bug"));
        assert!(matches_search(&lexer, "ada"));
        assert!(!matches_search(&lexer, "#1"));
        assert!(!matches_search(&lexer, "parser"));
    }

    #[test]
    fn a_key_names_the_repository_it_says_or_the_open_ones() {
        let links = vec![
            link("upstream", ProviderKind::Github, "octo", "tools"),
            link("origin", ProviderKind::Github, "ada", "tools"),
        ];
        assert_eq!(first_link(&links).unwrap().remote, "origin");
        let named = |text: &str| {
            link_for_key(&links, &IssueKey::parse(text).unwrap())
                .map(|link| format!("{}/{}@{}", link.owner, link.name, link.remote))
        };
        assert_eq!(named("#3").as_deref(), Some("ada/tools@origin"));
        assert_eq!(
            named("OCTO/Tools#3").as_deref(),
            Some("octo/tools@upstream")
        );
        assert_eq!(named("other/thing#3").as_deref(), Some("other/thing@"));
        assert_eq!(named("group/sub/thing#3"), None);
        let lab = vec![link("origin", ProviderKind::Gitlab, "group", "tools")];
        let nested = link_for_key(&lab, &IssueKey::parse("group/sub/thing#3").unwrap()).unwrap();
        assert_eq!(
            (nested.owner.as_str(), nested.name.as_str()),
            ("group/sub", "thing")
        );
        assert_eq!(link_for_key(&[], &IssueKey::parse("#3").unwrap()), None);
    }

    #[test]
    fn keys_in_a_message_are_links_only_with_a_linked_remote() {
        let links = vec![link("origin", ProviderKind::Github, "octo", "tools")];
        let text = "Fix the lexer (#12), see octo/tools#7 and `#8`";
        let found = key_links(text, &links);
        let shown: Vec<(&str, u64)> = found
            .iter()
            .map(|key| (text.get(key.range.clone()).unwrap(), key.number))
            .collect();
        assert_eq!(shown, [("#12", 12), ("octo/tools#7", 7)]);
        assert!(key_links(text, &[]).is_empty());
        assert!(key_links("no keys", &links).is_empty());
    }

    #[test]
    fn branches_made_for_an_issue_are_found_and_named() {
        let names = vec![
            "main".to_owned(),
            "12-fix-lexer".to_owned(),
            "feature/12-x".to_owned(),
            "fix-123".to_owned(),
        ];
        assert_eq!(branches_for(&names, 12), ["12-fix-lexer", "feature/12-x"]);
        assert!(branches_for(&names, 7).is_empty());
        let lexer = issue(12, "The lexer: drops \"quotes\"!");
        assert_eq!(
            offered_branch_name("{number}-{title}", &lexer),
            "12-the-lexer-drops-quotes"
        );
        assert_eq!(offered_branch_name("fix/{number}", &lexer), "fix/12");
        // A template git would refuse falls back to the plain one.
        assert_eq!(
            offered_branch_name("{title}..", &lexer),
            "12-the-lexer-drops-quotes"
        );
        assert_eq!(offered_branch_name("", &lexer), "12-the-lexer-drops-quotes");
        let links = vec![link("origin", ProviderKind::Gitlab, "g", "p")];
        assert_eq!(branch_issue(&links, "12-fix").map(|(_, n)| n), Some(12));
        assert_eq!(branch_issue(&links, "main"), None);
        assert_eq!(branch_issue(&[], "12-fix"), None);
        // Where the key stands in what a row shows of the name.
        let at = |branch: &str, shown: &str| {
            super::branch_issue_in(&links, branch, shown)
                .map(|(_, number, at)| (number, at.and_then(|at| shown.get(at)).map(str::to_owned)))
        };
        let at = |branch: &str, shown: &str| {
            at(branch, shown).map(|(number, key)| (number, key.unwrap_or_default()))
        };
        assert_eq!(
            at("qa/1-the-button-does-nothing", "1-the-button-does-nothing"),
            Some((1, "1".to_owned()))
        );
        assert_eq!(at("qa/#1-button", "#1-button"), Some((1, "#1".to_owned())));
        assert_eq!(
            at("qa/#1-button", "qa/#1-button"),
            Some((1, "#1".to_owned()))
        );
        // Not in what is shown: the card all the same, no link.
        assert_eq!(at("fix-7", "something else"), Some((7, String::new())));
        assert_eq!(at("main", "main"), None);
        assert_eq!(super::branch_issue_in(&[], "12-fix", "12-fix"), None);
    }

    #[test]
    fn a_message_names_an_issue_once() {
        assert!(names_issue("Fix it\n\nRefs: #12", 12));
        assert!(!names_issue("Fix it\n\nRefs: #123", 12));
        assert!(!names_issue("Fix `#12`", 12));
        assert!(!names_issue("Fix octo/tools#12", 12));
    }

    #[test]
    fn a_card_takes_the_first_lines_that_say_something() {
        let body = "## Steps\n\n```\ncode\n```\n> quoted\n<!-- hidden -->\nthird\nfourth";
        assert_eq!(first_lines(body, 3, 80), ["Steps", "code", "quoted"]);
        assert_eq!(
            first_lines("See **nothing** in `\"a b\"`, ~~gone~~ snake_case", 1, 80),
            ["See nothing in \"a b\", gone snake_case"]
        );
        assert_eq!(first_lines("", 3, 80), Vec::<String>::new());
        assert_eq!(
            first_lines(&"x".repeat(200), 1, 10),
            [format!("{}…", "x".repeat(10))]
        );
    }
}
