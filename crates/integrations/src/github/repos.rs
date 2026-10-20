//! Repositories at GitHub and the pictures of commit authors: the
//! repositories the account may clone, a page at a time (its own, its
//! organisations', starred, or all of them); where it may make one
//! (itself and its organisations); and the picture of each author of a
//! repository's commits, by the e-mail address in the commit. What the
//! commits taught is kept for as long as the process lasts, and an
//! address learnt is not asked about again; the commits' answer itself
//! is kept on disk ([`crate::cache`]).
//!
//! GitHub's list of the user's repositories has no search: with words,
//! every page is read (up to a bound) and the repositories whose name or
//! description has them are paged instead.
//!
//! Sources: docs.github.com/en/rest "Repositories" (list repositories for
//! the authenticated user: `affiliation`, `sort`, `per_page`, `page`),
//! "Starring" (list repositories starred by the authenticated user),
//! "Organizations" (list organizations for the authenticated user),
//! "Users" (get the authenticated user), "Commits" (list commits: the
//! commit's author e-mail beside the author's account); docs.github.com
//! "Setting your commit email address" (the `ID+login@users.noreply.github.com`
//! addresses).

pub(super) mod writes;

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{WireUser, fresh, repo_path};
use crate::{
    api::{self, ProviderError},
    http::Method,
    model::{HostedRepo, Listing, Namespace, NamespaceKind},
    provider::{RepoFilter, RepoQuery, Session, Target},
};

/// Repositories on a page.
const PAGE: usize = 100;
/// Most repositories read to look for words in.
const MOST_SEARCHED: usize = 1000;
/// Most organisations read.
const MOST_ORGANISATIONS: usize = 500;
/// Most commits read to learn their authors' pictures.
const MOST_COMMITS: usize = 300;

#[derive(Debug, Deserialize)]
struct WireOwner {
    login: String,
}

#[derive(Debug, Deserialize)]
struct WireParent {
    full_name: String,
}

#[derive(Debug, Deserialize)]
struct WireRepo {
    id: u64,
    name: String,
    full_name: String,
    owner: WireOwner,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    parent: Option<WireParent>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    clone_url: Option<String>,
    #[serde(default)]
    ssh_url: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    pushed_at: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    stargazers_count: u64,
}

fn repo(wire: &WireRepo) -> HostedRepo {
    let text = |text: &Option<String>, max: usize| {
        api::clean_line(text.as_deref().unwrap_or_default(), max)
    };
    HostedRepo {
        id: wire.id.to_string(),
        owner: api::clean_line(&wire.owner.login, 256),
        name: api::clean_line(&wire.name, 256),
        full_name: api::clean_line(&wire.full_name, 512),
        description: wire
            .description
            .as_deref()
            .map(|text| api::clean_line(text, 1024))
            .filter(|text| !text.trim().is_empty()),
        private: wire.private,
        // A list names no parent: only one repository's answer does.
        parent: wire
            .parent
            .as_ref()
            .map(|parent| api::clean_line(&parent.full_name, 512)),
        archived: wire.archived,
        default_branch: wire
            .default_branch
            .as_deref()
            .map(|branch| api::clean_line(branch, 256)),
        clone_https: text(&wire.clone_url, 2048),
        clone_ssh: text(&wire.ssh_url, 2048),
        web_url: text(&wire.html_url, 2048),
        updated: wire
            .pushed_at
            .as_deref()
            .or(wire.updated_at.as_deref())
            .map(|at| api::clean_line(at, 64)),
        stars: wire.stargazers_count,
    }
}

/// One repository.
pub(super) fn get(session: &Session<'_>, target: &Target) -> Result<HostedRepo, ProviderError> {
    let (wire, _): (WireRepo, _) = session.api()?.json(Method::Get, &repo_path(target), None)?;
    Ok(repo(&wire))
}

/// The address of a list of the user's repositories.
fn source(filter: RepoFilter) -> &'static str {
    match filter {
        RepoFilter::Own => "/user/repos?affiliation=owner&sort=updated",
        RepoFilter::Organisation => "/user/repos?affiliation=organization_member&sort=updated",
        RepoFilter::Starred => "/user/starred?sort=updated",
        RepoFilter::All => {
            "/user/repos?affiliation=owner,collaborator,organization_member&sort=updated"
        }
    }
}

/// The repositories the account may clone.
pub(super) fn list(
    session: &Session<'_>,
    query: &RepoQuery,
) -> Result<Listing<HostedRepo>, ProviderError> {
    let api = session.api()?;
    let page = usize::try_from(query.page.max(1)).unwrap_or(1);
    let words: Vec<String> = query
        .query
        .as_deref()
        .unwrap_or_default()
        .split_whitespace()
        .map(str::to_lowercase)
        .collect();
    if words.is_empty() {
        let path = format!("{}&per_page={PAGE}&page={page}", source(query.filter));
        let (wire, response): (Vec<WireRepo>, _) = api.json(Method::Get, &path, None)?;
        let more = response.header("link").and_then(api::next_link).is_some();
        return Ok(Listing {
            items: wire.iter().map(repo).take(PAGE).collect(),
            more,
            freshness: fresh(session, &response),
        });
    }
    let path = format!("{}&per_page={PAGE}", source(query.filter));
    let (found, cut, last) = api.pages_where::<WireRepo>(&path, MOST_SEARCHED, |wire| {
        let text = format!(
            "{} {}",
            wire.full_name,
            wire.description.as_deref().unwrap_or_default()
        )
        .to_lowercase();
        words.iter().all(|word| text.contains(word.as_str()))
    })?;
    let start = page.saturating_sub(1).saturating_mul(PAGE);
    let items: Vec<HostedRepo> = found.iter().skip(start).take(PAGE).map(repo).collect();
    Ok(Listing {
        more: cut || found.len() > start.saturating_add(PAGE),
        items,
        freshness: fresh(session, &last),
    })
}

/// Where the account may make a repository: itself, and the
/// organisations it belongs to.
pub(super) fn namespaces(session: &Session<'_>) -> Result<Vec<Namespace>, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireOrganisation {
        login: String,
        #[serde(default)]
        avatar_url: Option<String>,
    }
    let api = session.api()?;
    let (me, _): (WireUser, _) = api.json(Method::Get, "/user", None)?;
    let me = me.user();
    let mut out = vec![Namespace {
        path: me.login.clone(),
        name: me.name.clone().unwrap_or_else(|| me.login.clone()),
        kind: NamespaceKind::User,
        avatar_url: me.avatar_url,
    }];
    let (organisations, _, _) =
        api.pages::<WireOrganisation>("/user/orgs?per_page=100", MOST_ORGANISATIONS)?;
    // The list names an organisation by its login only.
    out.extend(organisations.iter().map(|organisation| {
        let path = api::clean_line(&organisation.login, 256);
        Namespace {
            name: path.clone(),
            path,
            kind: NamespaceKind::Organisation,
            avatar_url: organisation
                .avatar_url
                .as_deref()
                .map(|url| api::clean_line(url, 2048)),
        }
    }));
    Ok(out)
}

/// The picture of the GitHub account an address of the form
/// `ID+login@users.noreply.github.com` stands for, without asking.
fn noreply(email: &str) -> Option<String> {
    let local = email
        .to_ascii_lowercase()
        .strip_suffix("@users.noreply.github.com")?
        .to_owned();
    let (id, _) = local.split_once('+')?;
    (!id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit()))
        .then(|| format!("https://avatars.githubusercontent.com/u/{id}?v=4"))
}

/// The pictures of the people with these addresses: an address GitHub
/// made for an account says whose it is; the others are learnt from the
/// commits of `target`, whose authors GitHub names beside the address in
/// each commit.
pub(super) fn avatars(
    session: &Session<'_>,
    target: Option<&Target>,
    emails: &[String],
) -> Result<BTreeMap<String, String>, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireGitPerson {
        #[serde(default)]
        email: Option<String>,
    }
    #[derive(Debug, Deserialize)]
    struct WireGitCommit {
        #[serde(default)]
        author: Option<WireGitPerson>,
        #[serde(default)]
        committer: Option<WireGitPerson>,
    }
    #[derive(Debug, Deserialize)]
    struct WireCommit {
        commit: WireGitCommit,
        #[serde(default)]
        author: Option<WireUser>,
        #[serde(default)]
        committer: Option<WireUser>,
    }
    let site = session.account.api.clone();
    let mut found = BTreeMap::new();
    let github_com = session.account.web.trim_end_matches('/') == "https://github.com";
    for email in emails {
        if let Some(picture) = noreply(email)
            .filter(|_| github_com)
            .or_else(|| learnt(&site, email))
        {
            found.insert(email.clone(), picture);
        }
    }
    let Some(target) = target else {
        return Ok(found);
    };
    if found.len() == emails.len() {
        return Ok(found);
    }
    let (commits, _, _) = session.api()?.pages::<WireCommit>(
        &format!("{}/commits?per_page=100", repo_path(target)),
        MOST_COMMITS,
    )?;
    for commit in &commits {
        let pairs = [
            (commit.commit.author.as_ref(), commit.author.as_ref()),
            (commit.commit.committer.as_ref(), commit.committer.as_ref()),
        ];
        for (person, account) in pairs {
            if let (Some(email), Some(picture)) = (
                person.and_then(|person| person.email.as_deref()),
                account.and_then(|account| account.user().avatar_url),
            ) {
                learn(&site, email, &picture);
            }
        }
    }
    for email in emails {
        if !found.contains_key(email)
            && let Some(picture) = learnt(&site, email)
        {
            found.insert(email.clone(), picture);
        }
    }
    Ok(found)
}

/// Most addresses learnt and kept for as long as the process lasts.
const MOST_LEARNT: usize = 10_000;

/// The pictures learnt from commits: by the API's address and the e-mail
/// address in lower case.
fn pictures() -> &'static std::sync::Mutex<BTreeMap<(String, String), String>> {
    static LEARNT: std::sync::OnceLock<std::sync::Mutex<BTreeMap<(String, String), String>>> =
        std::sync::OnceLock::new();
    LEARNT.get_or_init(Default::default)
}

/// Keep the picture of an address, as a site's commits named it.
fn learn(site: &str, email: &str, picture: &str) {
    if email.len() > 254 || picture.len() > 2048 {
        return;
    }
    if let Ok(mut learnt) = pictures().lock() {
        if learnt.len() >= MOST_LEARNT {
            learnt.clear();
        }
        learnt.insert(
            (site.to_owned(), email.to_ascii_lowercase()),
            picture.to_owned(),
        );
    }
}

/// The picture of an address learnt from a site's commits before.
fn learnt(site: &str, email: &str) -> Option<String> {
    pictures()
        .lock()
        .ok()?
        .get(&(site.to_owned(), email.to_ascii_lowercase()))
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_learnt_is_kept_for_its_site() {
        let site = "https://learnt.example.invalid/api/v3";
        assert_eq!(learnt(site, "A@Example.org"), None);
        learn(site, "a@example.org", "https://pictures.example.invalid/a");
        assert_eq!(
            learnt(site, "A@Example.org").as_deref(),
            Some("https://pictures.example.invalid/a")
        );
        assert_eq!(
            learnt("https://elsewhere.example.invalid", "a@example.org"),
            None
        );
        learn(site, &"x".repeat(300), "https://pictures.example.invalid/x");
        assert_eq!(learnt(site, &"x".repeat(300)), None);
    }

    #[test]
    fn an_address_github_made_says_whose_it_is() {
        assert_eq!(
            noreply("583231+octocat@users.noreply.github.com").as_deref(),
            Some("https://avatars.githubusercontent.com/u/583231?v=4")
        );
        assert_eq!(noreply("octocat@users.noreply.github.com"), None);
        assert_eq!(noreply("x+octocat@users.noreply.github.com"), None);
        assert_eq!(noreply("583231+octocat@example.org"), None);
    }
}
