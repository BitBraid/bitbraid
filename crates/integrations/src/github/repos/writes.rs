//! Making repositories and keys at GitHub: a new repository of the user's
//! or of one of their organisations; a fork, waited for until GitHub has
//! made its branches; a public key uploaded to the user's account.
//!
//! GitHub makes a fork after it answers: the fork's branches are read
//! until there are some, a second apart, at most [`MOST_WAITS`] times;
//! then it is answered as it is.
//!
//! Sources: docs.github.com/en/rest "Repositories" (create a repository
//! for the authenticated user, create an organization repository:
//! `name`, `description`, `private`), "Forks" (create a fork:
//! `organization`, "Forking a Repository happens asynchronously"),
//! "Branches" (list branches), "Git SSH keys" (create a public SSH key for
//! the authenticated user: `title`, `key`).

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{WireRepo, repo};
use crate::{
    api::{self, ProviderError},
    github::repo_path,
    http::Method,
    keys,
    model::{HostedRepo, SshKey},
    provider::{NewRepo, Session, Target},
};

/// How many times a new fork's branches are read before it is answered
/// as it is.
const MOST_WAITS: usize = 30;
/// The wait between two of those.
const WAIT: Duration = Duration::from_secs(1);

/// Whether a namespace is the user's own, which GitHub names by leaving it
/// out.
fn own(session: &Session<'_>, namespace: Option<&str>) -> bool {
    namespace.is_none_or(|namespace| namespace.eq_ignore_ascii_case(&session.account.login))
}

/// Make a repository: the user's, or an organisation's.
pub(in crate::github) fn create(
    session: &Session<'_>,
    new: &NewRepo,
) -> Result<HostedRepo, ProviderError> {
    let path = match new.namespace.as_deref() {
        Some(organisation) if !own(session, Some(organisation)) => {
            format!("/orgs/{}/repos", api::encode(organisation))
        }
        _ => "/user/repos".to_owned(),
    };
    let mut body = Map::new();
    body.insert("name".to_owned(), json!(new.name));
    body.insert("private".to_owned(), json!(new.private));
    if let Some(description) = &new.description {
        body.insert("description".to_owned(), json!(description));
    }
    let (wire, _): (WireRepo, _) =
        session
            .api()?
            .json(Method::Post, &path, Some(&Value::Object(body)))?;
    Ok(repo(&wire))
}

/// Fork a repository into the user's account or an organisation, and
/// wait, bounded, until the fork has its branches.
pub(in crate::github) fn fork(
    session: &Session<'_>,
    target: &Target,
    namespace: Option<&str>,
) -> Result<HostedRepo, ProviderError> {
    let api = session.api()?;
    let body = match namespace {
        Some(organisation) if !own(session, Some(organisation)) => {
            json!({ "organization": organisation })
        }
        _ => json!({}),
    };
    let (wire, _): (WireRepo, _) = api.json(
        Method::Post,
        &format!("{}/forks", repo_path(target)),
        Some(&body),
    )?;
    let made = repo(&wire);
    let at = Target {
        owner: made.owner.clone(),
        name: made.name.clone(),
    };
    let branches = format!("{}/branches?per_page=1", repo_path(&at));
    for _ in 0..MOST_WAITS {
        match api.json::<Vec<Value>>(Method::Get, &branches, None) {
            Ok((found, _)) if !found.is_empty() => break,
            // Not made yet, or not filled yet.
            Ok(_) | Err(ProviderError::NotFound | ProviderError::Status { status: 409, .. }) => {
                session.pause(WAIT)?;
            }
            Err(err) => return Err(err),
        }
    }
    Ok(made)
}

/// A key as GitHub answers it.
#[derive(Debug, Deserialize)]
struct WireKey {
    id: u64,
    #[serde(default)]
    title: Option<String>,
}

/// Upload a public key to the user's account.
pub(in crate::github) fn key_add(
    session: &Session<'_>,
    title: &str,
    key: &str,
) -> Result<SshKey, ProviderError> {
    let read = keys::read(key).map_err(ProviderError::Refused)?;
    let (wire, _): (WireKey, _) = session.api()?.json(
        Method::Post,
        "/user/keys",
        Some(&json!({ "title": title, "key": read.line })),
    )?;
    Ok(SshKey {
        id: wire.id.to_string(),
        title: api::clean_line(wire.title.as_deref().unwrap_or(title), 256),
        fingerprint: Some(read.fingerprint),
    })
}
