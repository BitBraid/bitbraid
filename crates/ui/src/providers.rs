//! The accounts at GitHub and GitLab as the window knows them, and what
//! every view that shows a provider needs: which of the four sites an
//! account is at, how a provider command is asked and its answer read, and
//! how a text that came from a provider is made fit to show.
//!
//! The accounts are read with `account.list` when the window starts and
//! again whenever one is added, removed or made the default. No token
//! passes through here: the commands keep tokens in the system's keychain,
//! and a token typed into a field goes from the field to
//! `account.add_token` and nowhere else.
//!
//! A provider command runs on a thread of its own (`session::run_standalone`),
//! never behind the open repository's queue: a slow provider holds up
//! nothing else.
//!
//! Sources: docs.github.com "Scopes for OAuth apps" and "Managing your
//! personal access tokens" (the page that makes a token, with its
//! description and scopes filled in); docs.gitlab.com "Personal access
//! tokens" (the scopes, and the page's `name` and `scopes` parameters).

use std::future::Future;

use bitbraid_integrations::model::{self, Account, ProviderKind, RepoLink};
use gpui_kit::*;
use serde::de::DeserializeOwned;

use crate::session::Reply;

/// One of the sites the integrations section lists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Site {
    /// github.com.
    Github,
    /// A GitHub Enterprise Server at an address of its own.
    GithubServer,
    /// gitlab.com.
    Gitlab,
    /// A GitLab of one's own.
    GitlabServer,
}

/// What a GitHub token is asked to be allowed, as GitHub names it.
pub const GITHUB_SCOPES: &[&str] = &[
    "repo",
    "read:org",
    "admin:public_key",
    "read:user",
    "user:email",
    "workflow",
];

/// What a GitLab token is asked to be allowed.
pub const GITLAB_SCOPES: &[&str] = &["api", "read_user"];

impl Site {
    /// In the order the section lists them.
    pub const ALL: [Site; 4] = [
        Site::Github,
        Site::GithubServer,
        Site::Gitlab,
        Site::GitlabServer,
    ];

    /// What it is called.
    pub fn name(self) -> &'static str {
        match self {
            Site::Github => say!("providers-site-github"),
            Site::GithubServer => say!("providers-site-github-server"),
            Site::Gitlab => say!("providers-site-gitlab"),
            Site::GitlabServer => say!("providers-site-gitlab-server"),
        }
    }

    /// What it is called in element ids and tests.
    pub fn key(self) -> &'static str {
        match self {
            Site::Github => "github",
            Site::GithubServer => "github_server",
            Site::Gitlab => "gitlab",
            Site::GitlabServer => "gitlab_server",
        }
    }

    /// The provider it is.
    pub fn kind(self) -> ProviderKind {
        match self {
            Site::Github | Site::GithubServer => ProviderKind::Github,
            Site::Gitlab | Site::GitlabServer => ProviderKind::Gitlab,
        }
    }

    /// What the commands call the provider.
    pub fn kind_word(self) -> &'static str {
        kind_word(self.kind())
    }

    /// The site's own address; none for one at an address of its own.
    pub fn public_web(self) -> Option<&'static str> {
        match self {
            Site::Github => Some("https://github.com"),
            Site::Gitlab => Some("https://gitlab.com"),
            Site::GithubServer | Site::GitlabServer => None,
        }
    }

    /// Whether it is at an address the user gives.
    pub fn self_hosted(self) -> bool {
        self.public_web().is_none()
    }

    /// The site an account is at.
    pub fn of(account: &Account) -> Site {
        let public = same_web(&account.web, "https://github.com")
            || same_web(&account.web, "https://gitlab.com");
        match (account.kind, public) {
            (ProviderKind::Github, true) => Site::Github,
            (ProviderKind::Github, false) => Site::GithubServer,
            (ProviderKind::Gitlab, true) => Site::Gitlab,
            (ProviderKind::Gitlab, false) => Site::GitlabServer,
        }
    }

    /// What a token has to be allowed.
    pub fn scopes(self) -> &'static [&'static str] {
        match self.kind() {
            ProviderKind::Github => GITHUB_SCOPES,
            ProviderKind::Gitlab => GITLAB_SCOPES,
        }
    }

    /// The page of the site at `web` where a token is made, with its name
    /// and what it is allowed filled in.
    pub fn token_page(self, web: &str) -> String {
        let web = web.trim_end_matches('/');
        let scopes = self.scopes().join(",");
        match self.kind() {
            ProviderKind::Github => {
                format!("{web}/settings/tokens/new?description=BitBraid&scopes={scopes}")
            }
            ProviderKind::Gitlab => format!(
                "{web}/-/user_settings/personal_access_tokens?name=BitBraid&scopes={scopes}"
            ),
        }
    }

    /// Its mark: our own, from the toolkit's set.
    pub fn icon(self) -> gpui_kit::component::Icon {
        use gpui_kit::assets::IconName as Extra;
        use gpui_kit::component::{Icon, IconName};
        match self {
            Site::Github => Icon::new(IconName::Github),
            Site::GithubServer => Icon::new(IconName::Building2),
            Site::Gitlab => Icon::new(Extra::FolderGit2),
            Site::GitlabServer => Icon::new(Extra::Server),
        }
    }
}

/// What the commands call a provider.
pub fn kind_word(kind: ProviderKind) -> &'static str {
    match kind {
        ProviderKind::Github => "github",
        ProviderKind::Gitlab => "gitlab",
    }
}

/// Whether two site addresses are the same site: case and a trailing
/// slash aside.
pub fn same_web(one: &str, other: &str) -> bool {
    one.trim_end_matches('/')
        .eq_ignore_ascii_case(other.trim_end_matches('/'))
}

/// The host of a site address, with its port: what a card says.
pub fn host_of(web: &str) -> &str {
    let rest = web
        .strip_prefix("https://")
        .or_else(|| web.strip_prefix("http://"))
        .unwrap_or(web);
    rest.split('/').next().unwrap_or(rest)
}

/// Whether a host is this machine: the one place a site may be spoken to
/// without TLS (a GitLab of one's own run for trying things out).
fn is_loopback(host: &str) -> bool {
    let name = match host.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest),
        None => host.rsplit_once(':').map_or(host, |(name, _)| name),
    };
    name == "localhost"
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// The address of a site of one's own, from what was typed: `host`,
/// `host:port`, `https://host/path`. Plain HTTP only for this machine. A
/// name and password, a question or a fragment are refused, and so is
/// anything with a space or a control character in it.
pub fn site_address(typed: &str) -> Result<String, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Err(say!("providers-address-empty").to_owned());
    }
    if typed.len() > 2048
        || typed
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || matches!(c, '?' | '#' | '\\'))
    {
        return Err(say!("providers-address-invalid").to_owned());
    }
    let (scheme, rest) = match typed.split_once("://") {
        Some((scheme, rest)) => (scheme.to_ascii_lowercase(), rest),
        None => ("https".to_owned(), typed),
    };
    let rest = rest.trim_end_matches('/');
    let host = rest.split('/').next().unwrap_or_default();
    if host.is_empty() || host.contains('@') || host.starts_with('.') || host.starts_with(':') {
        return Err(say!("providers-address-invalid").to_owned());
    }
    match scheme.as_str() {
        "https" => {}
        "http" if is_loopback(host) => {}
        "http" => {
            return Err(say!("providers-address-plain-http").to_owned());
        }
        _ => return Err(say!("providers-address-scheme").to_owned()),
    }
    Ok(format!("{scheme}://{rest}"))
}

/// A text from a provider as it is shown: one line, no control
/// characters, cut at [`model::MAX_TITLE`].
pub fn shown(text: &str) -> String {
    let cut = model::bounded(text, model::MAX_TITLE);
    cut.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Whether an address a provider gave to clone or fetch from may be
/// handed on: one line, and nothing that could be taken for an option.
pub fn usable_address(url: &str) -> bool {
    let url = url.trim();
    !url.is_empty()
        && url.len() <= 4096
        && !url.starts_with('-')
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// What an error of a provider command says, as a sentence.
pub fn said(error: &str) -> String {
    // A parameter the command would not take: which, and why.
    let error = match error
        .strip_prefix("invalid parameter \"")
        .and_then(|rest| rest.split_once("\": "))
    {
        Some((name, why)) => say!("providers-invalid-parameter", name, why),
        None => error.to_owned(),
    };
    let mut text = crate::shell::sentence(error.trim());
    if !text.ends_with(['.', '!', '?']) {
        text.push('.');
    }
    text
}

/// Ask a provider command on a thread of its own, and read its answer as
/// a `T`. An error is the sentence to show.
pub fn ask<T: DeserializeOwned + 'static>(
    id: &'static str,
    params: serde_json::Value,
) -> impl Future<Output = Result<T, String>> + 'static {
    let run = crate::session::run_standalone(id, params);
    async move {
        match run.reply.await {
            Ok(Reply::Command(value)) => serde_json::from_value(value)
                .map_err(|err| say!("providers-answer-unreadable", id, error = err.to_string())),
            Ok(Reply::Error(err)) => Err(said(&err)),
            Ok(_) | Err(_) => Err(say!("providers-no-answer").to_owned()),
        }
    }
}

/// The accounts the window knows, a GPUI global.
#[derive(Debug, Default)]
pub struct Accounts {
    list: Vec<Account>,
    /// Whether `account.list` has answered once.
    read: bool,
    /// Why they could not be read, when they could not.
    problem: Option<String>,
    /// Which reading is the latest: an older answer is not taken.
    asked: u64,
}

impl Global for Accounts {}

/// The ids of every account, as last read: what the sections of the left
/// panel compare to know that one was connected or disconnected.
pub fn account_ids(cx: &App) -> Vec<String> {
    cx.try_global::<Accounts>()
        .map(|known| {
            known
                .list
                .iter()
                .map(|account| account.id.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// Every account, as last read.
pub fn accounts(cx: &App) -> Vec<Account> {
    cx.try_global::<Accounts>()
        .map(|known| known.list.clone())
        .unwrap_or_default()
}

/// The accounts at one site.
pub fn accounts_at(site: Site, cx: &App) -> Vec<Account> {
    accounts(cx)
        .into_iter()
        .filter(|account| Site::of(account) == site)
        .collect()
}

/// The account of this id.
pub fn account(id: &str, cx: &App) -> Option<Account> {
    accounts(cx).into_iter().find(|account| account.id == id)
}

/// Whether the accounts were read, and why not when they could not be.
pub fn read_state(cx: &App) -> (bool, Option<String>) {
    cx.try_global::<Accounts>()
        .map(|known| (known.read, known.problem.clone()))
        .unwrap_or_default()
}

/// Take a list of accounts as what is known, and draw the windows again.
/// What came from a provider is cut to size first.
pub fn set_accounts(list: Vec<Account>, cx: &mut App) {
    let known = cx.default_global::<Accounts>();
    known.list = list.into_iter().take(64).map(tidy).collect();
    known.read = true;
    known.problem = None;
    known.asked = known.asked.wrapping_add(1);
    cx.refresh_windows();
}

/// An account's texts as they are shown.
fn tidy(mut account: Account) -> Account {
    account.login = shown(&account.login);
    account.name = account.name.as_deref().map(shown).filter(|n| !n.is_empty());
    account
}

/// Read the accounts again with `account.list`. The windows are drawn
/// again when they are in.
pub fn reload(cx: &mut App) {
    let asked = {
        let known = cx.default_global::<Accounts>();
        known.asked = known.asked.wrapping_add(1);
        known.asked
    };
    let answer = ask::<Vec<Account>>("account.list", serde_json::json!({}));
    cx.spawn(async move |cx| {
        let answer = answer.await;
        cx.update(|cx| {
            if cx.default_global::<Accounts>().asked != asked {
                return;
            }
            match answer {
                Ok(list) => {
                    set_accounts(list, cx);
                    // Taking them counted as asking again.
                    cx.default_global::<Accounts>().asked = asked;
                }
                Err(problem) => {
                    let known = cx.default_global::<Accounts>();
                    known.read = true;
                    known.problem = Some(problem);
                    cx.refresh_windows();
                }
            }
        });
    })
    .detach();
}

/// Install the global and read the accounts once.
pub fn init(cx: &mut App) {
    cx.set_global(Accounts::default());
    reload(cx);
}

/// Which remote of which repository is which repository at which account,
/// as `provider.links` answered for each open repository (by its git
/// directory), and which repository the window is drawing now.
#[derive(Debug, Default)]
pub struct Links {
    by_repo: std::collections::HashMap<std::path::PathBuf, RepoLinks>,
    drawing: Option<std::path::PathBuf>,
}

/// The links of one repository, and the remotes they were asked for.
#[derive(Debug, Default, Clone)]
struct RepoLinks {
    asked_for: Vec<(String, Option<String>)>,
    links: Vec<RepoLink>,
}

impl Global for Links {}

/// The remotes (name and address) the links of a repository were last
/// asked for.
pub fn links_asked_for(
    git_dir: &std::path::Path,
    cx: &App,
) -> Option<Vec<(String, Option<String>)>> {
    cx.try_global::<Links>()?
        .by_repo
        .get(git_dir)
        .map(|repo| repo.asked_for.clone())
}

/// Take the links of a repository, for these remotes.
pub fn set_links(
    git_dir: &std::path::Path,
    asked_for: Vec<(String, Option<String>)>,
    links: Vec<RepoLink>,
    cx: &mut App,
) {
    let known = cx.default_global::<Links>();
    known.by_repo.insert(
        git_dir.to_path_buf(),
        RepoLinks {
            asked_for,
            links: links.into_iter().take(64).collect(),
        },
    );
    cx.refresh_windows();
}

/// The repository the window is drawing: what [`remote_picture`] answers
/// for.
pub fn drawing(git_dir: &std::path::Path, cx: &mut App) {
    let known = cx.default_global::<Links>();
    if known.drawing.as_deref() != Some(git_dir) {
        known.drawing = Some(git_dir.to_path_buf());
    }
}

/// The repository the window drew last, by its git directory.
pub fn drawing_dir(cx: &App) -> Option<std::path::PathBuf> {
    cx.try_global::<Links>()?.drawing.clone()
}

/// The links of the repository being drawn: what issue keys in its
/// messages and branch names are links of.
pub fn drawn_links(cx: &App) -> Vec<RepoLink> {
    cx.try_global::<Links>()
        .and_then(|known| known.by_repo.get(known.drawing.as_ref()?))
        .map(|repo| repo.links.clone())
        .unwrap_or_default()
}

/// The address of the picture of a remote's owner in the repository being
/// drawn: the account's own when it owns the repository; at GitHub, the
/// owner's picture by name.
pub fn owner_picture_address(remote: &str, cx: &App) -> Option<String> {
    let known = cx.try_global::<Links>()?;
    let repo = known.by_repo.get(known.drawing.as_ref()?)?;
    let link = repo.links.iter().find(|link| link.remote == remote)?;
    let account = account(&link.account, cx)?;
    if link.owner.eq_ignore_ascii_case(&account.login) {
        return account.avatar_url.clone();
    }
    let owner_plain = !link.owner.is_empty()
        && link
            .owner
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-');
    (account.kind == ProviderKind::Github && owner_plain)
        .then(|| format!("{}/{}.png", account.web.trim_end_matches('/'), link.owner))
}

/// Whom e-mail addresses are looked up with, for the pictures of authors:
/// each account a remote of the repository being drawn is linked to, with
/// that remote's repository (origin first; GitHub knows its authors by
/// their commits there), then each site's default account that is not
/// among them, without one.
pub fn lookup_targets(cx: &App) -> Vec<(String, Option<(String, String)>)> {
    let mut targets: Vec<(String, Option<(String, String)>)> = Vec::new();
    if let Some(known) = cx.try_global::<Links>()
        && let Some(repo) = known
            .drawing
            .as_ref()
            .and_then(|dir| known.by_repo.get(dir))
    {
        for link in &repo.links {
            if !targets.iter().any(|(account, _)| *account == link.account) {
                targets.push((
                    link.account.clone(),
                    Some((link.owner.clone(), link.name.clone())),
                ));
            }
        }
    }
    for account in accounts(cx).into_iter().filter(|account| account.default) {
        if !targets.iter().any(|(id, _)| *id == account.id) {
            targets.push((account.id, None));
        }
    }
    targets
}

/// The picture of a remote's owner, drawn `size` each way, when pictures
/// from the providers are on and it is in memory. `remote` is a remote's
/// name, or a remote branch's (`origin/main`).
pub fn remote_picture(remote: &str, size: f32, cx: &App) -> Option<AnyElement> {
    if !crate::avatars::from_providers(cx) {
        return None;
    }
    let name = remote.split_once('/').map_or(remote, |(name, _)| name);
    let address = owner_picture_address(name, cx)?;
    let picture = crate::avatars::picture_at(&address, false, cx)?;
    Some(
        img(picture)
            .size(crate::zoom::px(size))
            .flex_none()
            .rounded_full()
            .into_any_element(),
    )
}

/// What an account is called on a tab or in a list: the site's mark goes
/// beside it, so the login alone, and the host where the site is not the
/// usual one.
pub fn account_label(account: &Account) -> String {
    match Site::of(account) {
        Site::Github | Site::Gitlab => account.login.clone(),
        Site::GithubServer | Site::GitlabServer => {
            say!(
                "providers-account-at",
                login = &account.login,
                host = host_of(&account.web)
            )
        }
    }
}

/// Whether a link may be opened in the system's browser: HTTPS, or HTTP
/// to this machine, and nothing that could be taken for something else.
pub fn openable(link: &str) -> bool {
    let link = link.trim();
    if link.len() > 4096 || link.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return false;
    }
    if let Some(rest) = link.strip_prefix("https://") {
        return !rest.is_empty() && !host_of(link).contains('@');
    }
    link.strip_prefix("http://")
        .is_some_and(|_| is_loopback(host_of(link)) && !host_of(link).contains('@'))
}

/// Open a link of a provider's in the system's browser, when it may be.
pub fn open_link(link: &str, cx: &App) -> bool {
    let may = openable(link);
    if may {
        cx.open_url(link.trim());
    }
    may
}

#[cfg(test)]
mod tests {
    use super::{Site, host_of, openable, said, same_web, shown, site_address};
    use bitbraid_integrations::model::{Account, ProviderKind, SignIn};

    fn account(kind: ProviderKind, web: &str) -> Account {
        Account {
            id: format!("x:{web}:me"),
            kind,
            web: web.to_owned(),
            api: String::new(),
            login: "me".to_owned(),
            name: None,
            avatar_url: None,
            sign_in: SignIn::Token,
            scopes: Vec::new(),
            default: true,
        }
    }

    #[test]
    fn an_account_belongs_to_the_site_of_its_address() {
        assert_eq!(
            Site::of(&account(ProviderKind::Github, "https://github.com")),
            Site::Github
        );
        assert_eq!(
            Site::of(&account(ProviderKind::Github, "https://GitHub.com/")),
            Site::Github
        );
        assert_eq!(
            Site::of(&account(ProviderKind::Github, "https://ghe.example.org")),
            Site::GithubServer
        );
        assert_eq!(
            Site::of(&account(ProviderKind::Gitlab, "https://gitlab.com")),
            Site::Gitlab
        );
        assert_eq!(
            Site::of(&account(ProviderKind::Gitlab, "http://127.0.0.1:8929")),
            Site::GitlabServer
        );
        // A GitLab at github.com's address is still a GitLab.
        assert_eq!(
            Site::of(&account(ProviderKind::Gitlab, "https://github.com")),
            Site::Gitlab
        );
        assert!(same_web("https://a.org/", "https://A.org"));
    }

    #[test]
    fn the_token_page_has_the_name_and_scopes_filled_in() {
        assert_eq!(
            Site::GitlabServer.token_page("https://gitlab.example.org/"),
            "https://gitlab.example.org/-/user_settings/personal_access_tokens?name=BitBraid&scopes=api,read_user"
        );
        assert_eq!(
            Site::GithubServer.token_page("https://ghe.example.org"),
            "https://ghe.example.org/settings/tokens/new?description=BitBraid&scopes=repo,read:org,admin:public_key,read:user,user:email,workflow"
        );
    }

    #[test]
    fn a_servers_address_is_read_carefully() {
        assert_eq!(
            site_address(" gitlab.example.org ").unwrap(),
            "https://gitlab.example.org"
        );
        assert_eq!(
            site_address("https://code.example.org/gitlab/").unwrap(),
            "https://code.example.org/gitlab"
        );
        assert_eq!(
            site_address("http://127.0.0.1:8929").unwrap(),
            "http://127.0.0.1:8929"
        );
        assert_eq!(
            site_address("HTTP://localhost:8080/").unwrap(),
            "http://localhost:8080"
        );
        for bad in [
            "",
            "http://gitlab.example.org",
            "ftp://gitlab.example.org",
            "https://user:pw@gitlab.example.org",
            "https://gitlab.example.org/?q=1",
            "https://gitlab.example.org/#x",
            "gitlab example org",
            "https://",
            "https://:443",
            "gitlab.exa\nmple.org",
            "http://127.0.0.1.evil.org",
        ] {
            assert!(site_address(bad).is_err(), "{bad:?} was taken");
        }
        assert_eq!(
            host_of("https://gitlab.example.org:8443/x"),
            "gitlab.example.org:8443"
        );
    }

    #[test]
    fn texts_from_a_provider_are_one_tidy_line() {
        assert_eq!(shown(" a\nb\tc\u{7} "), "a b c");
        assert_eq!(shown(&"x".repeat(5000)).len(), 1024);
        assert_eq!(said("not built yet"), "Not built yet.");
        assert_eq!(
            said("invalid parameter \"token\": is not a token"),
            "The token is not a token."
        );
    }

    #[test]
    fn only_web_pages_are_opened() {
        assert!(openable("https://github.com/login/device"));
        assert!(openable("http://127.0.0.1:8929/oauth/device"));
        assert!(!openable("http://gitlab.example.org/oauth/device"));
        assert!(!openable("file:///etc/passwd"));
        assert!(!openable("javascript:alert(1)"));
        assert!(!openable("https://a b"));
        assert!(!openable("https://"));
        assert!(!openable("https://user@evil.org/"));
        assert!(super::usable_address("ssh://host.example.org/a/b.git"));
        assert!(!super::usable_address("--upload-pack=touch x"));
        assert!(!super::usable_address("https://a.org/b c"));
        assert!(!super::usable_address(""));
    }
}
