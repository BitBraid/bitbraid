//! Projects at GitLab and the pictures of addresses: the projects the
//! account may clone, a page at a time (its own, its groups', starred, or
//! all it is a member of), with GitLab's own search; where it may make
//! one (its namespaces: itself and its groups); and the picture GitLab
//! shows for an e-mail address.
//!
//! Sources: docs.gitlab.com/api "Projects" (list all projects: `owned`,
//! `membership`, `starred`, `search`, `order_by`, `page`, `per_page`;
//! `namespace.kind`, `forked_from_project`), "Namespaces" (list),
//! "Avatar" (get the avatar URL of a user, `email`, `size`).

pub(super) mod writes;

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{at_once, fresh};
use crate::{
    api::{self, ProviderError},
    http::Method,
    model::{HostedRepo, Listing, Namespace, NamespaceKind},
    provider::{RepoFilter, RepoQuery, Session, Target},
};

/// Projects on a page.
const PAGE: usize = 100;
/// Most namespaces read.
const MOST_NAMESPACES: usize = 500;
/// The side of the pictures asked for, as the avatars of authors are.
const SIDE: u32 = crate::avatars::ASKED_SIDE;

#[derive(Debug, Deserialize)]
struct WireNamespace {
    #[serde(default)]
    kind: Option<String>,
    full_path: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireParent {
    path_with_namespace: String,
}

#[derive(Debug, Deserialize)]
struct WireProject {
    id: u64,
    path: String,
    path_with_namespace: String,
    namespace: WireNamespace,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    forked_from_project: Option<WireParent>,
    #[serde(default)]
    archived: bool,
    #[serde(default)]
    default_branch: Option<String>,
    #[serde(default)]
    http_url_to_repo: Option<String>,
    #[serde(default)]
    ssh_url_to_repo: Option<String>,
    #[serde(default)]
    web_url: Option<String>,
    #[serde(default)]
    last_activity_at: Option<String>,
    #[serde(default)]
    star_count: u64,
}

fn repo(wire: &WireProject) -> HostedRepo {
    let text = |text: &Option<String>, max: usize| {
        api::clean_line(text.as_deref().unwrap_or_default(), max)
    };
    HostedRepo {
        id: wire.id.to_string(),
        owner: api::clean_line(&wire.namespace.full_path, 1024),
        name: api::clean_line(&wire.path, 256),
        full_name: api::clean_line(&wire.path_with_namespace, 1024),
        description: wire
            .description
            .as_deref()
            .map(|text| api::clean_line(text, 1024))
            .filter(|text| !text.trim().is_empty()),
        private: wire.visibility.as_deref() != Some("public"),
        parent: wire
            .forked_from_project
            .as_ref()
            .map(|parent| api::clean_line(&parent.path_with_namespace, 1024)),
        archived: wire.archived,
        default_branch: wire
            .default_branch
            .as_deref()
            .map(|branch| api::clean_line(branch, 256)),
        clone_https: text(&wire.http_url_to_repo, 2048),
        clone_ssh: text(&wire.ssh_url_to_repo, 2048),
        web_url: text(&wire.web_url, 2048),
        updated: wire
            .last_activity_at
            .as_deref()
            .map(|at| api::clean_line(at, 64)),
        stars: wire.star_count,
    }
}

/// One project.
pub(super) fn get(session: &Session<'_>, target: &Target) -> Result<HostedRepo, ProviderError> {
    let (wire, _): (WireProject, _) =
        session
            .api()?
            .json(Method::Get, &super::project_path(target), None)?;
    Ok(repo(&wire))
}

/// The projects the account may clone.
pub(super) fn list(
    session: &Session<'_>,
    query: &RepoQuery,
) -> Result<Listing<HostedRepo>, ProviderError> {
    let which = match query.filter {
        RepoFilter::Own => "owned=true",
        RepoFilter::Starred => "starred=true",
        RepoFilter::Organisation | RepoFilter::All => "membership=true",
    };
    let mut path = format!("/projects?{which}&order_by=last_activity_at&simple=false");
    if let Some(words) = query
        .query
        .as_deref()
        .filter(|words| !words.trim().is_empty())
    {
        path.push_str(&format!("&search={}", api::encode(words.trim())));
    }
    path.push_str(&format!("&per_page={PAGE}&page={}", query.page.max(1)));
    let (wire, response): (Vec<WireProject>, _) = session.api()?.json(Method::Get, &path, None)?;
    let more = response
        .header("x-next-page")
        .is_some_and(|next| !next.trim().is_empty())
        || response.header("link").and_then(api::next_link).is_some();
    Ok(Listing {
        items: wire
            .iter()
            // Of the groups: those whose namespace is a group.
            .filter(|project| {
                query.filter != RepoFilter::Organisation
                    || project.namespace.kind.as_deref() == Some("group")
            })
            .map(repo)
            .take(PAGE)
            .collect(),
        more,
        freshness: fresh(session, &response),
    })
}

/// Where the account may make a project: its namespaces.
pub(super) fn namespaces(session: &Session<'_>) -> Result<Vec<Namespace>, ProviderError> {
    let (found, _, _) = session
        .api()?
        .pages::<WireNamespace>("/namespaces?per_page=100", MOST_NAMESPACES)?;
    Ok(found
        .iter()
        .map(|wire| {
            let path = api::clean_line(&wire.full_path, 1024);
            Namespace {
                name: wire
                    .name
                    .as_deref()
                    .map(|name| api::clean_line(name, 256))
                    .filter(|name| !name.trim().is_empty())
                    .unwrap_or_else(|| path.clone()),
                path,
                kind: if wire.kind.as_deref() == Some("user") {
                    NamespaceKind::User
                } else {
                    NamespaceKind::Group
                },
                avatar_url: wire
                    .avatar_url
                    .as_deref()
                    .map(|url| api::clean_line(url, 2048)),
            }
        })
        .collect())
}

/// The pictures GitLab shows for these addresses, a few asked at once.
pub(super) fn avatars(
    session: &Session<'_>,
    emails: &[String],
) -> Result<BTreeMap<String, String>, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireAvatar {
        #[serde(default)]
        avatar_url: Option<String>,
    }
    let api = session.api()?;
    let found = at_once(emails, |email| {
        let path = format!("/avatar?email={}&size={SIDE}", api::encode(email));
        let (wire, _): (WireAvatar, _) = api.json(Method::Get, &path, None)?;
        Ok(wire
            .avatar_url
            .map(|url| api::clean_line(&url, 2048))
            .filter(|url| url.starts_with("https://") || url.starts_with("http://")))
    })?;
    Ok(emails
        .iter()
        .zip(found)
        .filter_map(|(email, url)| Some((email.clone(), url?)))
        .collect())
}
