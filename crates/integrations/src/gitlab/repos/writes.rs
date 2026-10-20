//! Making projects and keys at GitLab: a new project of the user's or of
//! one of their groups; a fork, waited for until GitLab has copied it; a
//! public key uploaded to the user's account.
//!
//! GitLab copies a fork after it answers: the fork is read until its
//! import is done, a second apart, at most [`MOST_WAITS`] times; a copy
//! that failed is an error with what GitLab said.
//!
//! Sources: docs.gitlab.com/api "Projects" (create a project: `name`,
//! `path`, `namespace_id`, `description`, `visibility`; fork a project:
//! `namespace_path`; `import_status`, `import_error`), "Namespaces" (get
//! a namespace by its path), "User SSH keys" (add an SSH key for the
//! current user: `title`, `key`).

use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{WireProject, repo};
use crate::{
    api::{self, ProviderError},
    gitlab::project_path,
    http::Method,
    keys,
    model::{HostedRepo, SshKey},
    provider::{NewRepo, Session, Target},
};

/// How many times a new fork is read before it is answered as it is.
const MOST_WAITS: usize = 60;
/// The wait between two of those.
const WAIT: Duration = Duration::from_secs(1);

/// Whether a namespace is the user's own, which GitLab takes when none is
/// named.
fn own(session: &Session<'_>, namespace: Option<&str>) -> bool {
    namespace.is_none_or(|namespace| namespace.eq_ignore_ascii_case(&session.account.login))
}

/// Make a project: the user's, or a group's.
pub(in crate::gitlab) fn create(
    session: &Session<'_>,
    new: &NewRepo,
) -> Result<HostedRepo, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireNamespaceId {
        id: u64,
    }
    let api = session.api()?;
    let mut body = Map::new();
    body.insert("name".to_owned(), json!(new.name));
    body.insert("path".to_owned(), json!(new.name));
    body.insert(
        "visibility".to_owned(),
        json!(if new.private { "private" } else { "public" }),
    );
    if let Some(description) = &new.description {
        body.insert("description".to_owned(), json!(description));
    }
    if let Some(group) = new
        .namespace
        .as_deref()
        .filter(|group| !own(session, Some(group)))
    {
        let (namespace, _): (WireNamespaceId, _) = api.json(
            Method::Get,
            &format!("/namespaces/{}", api::encode(group)),
            None,
        )?;
        body.insert("namespace_id".to_owned(), json!(namespace.id));
    }
    let (wire, _): (WireProject, _) =
        api.json(Method::Post, "/projects", Some(&Value::Object(body)))?;
    Ok(repo(&wire))
}

/// What a fork's copy has come to.
#[derive(Debug, Deserialize)]
struct WireImport {
    #[serde(default)]
    import_status: Option<String>,
    #[serde(default)]
    import_error: Option<String>,
}

/// Fork a project into the user's namespace or a group, and wait,
/// bounded, until GitLab has copied it.
pub(in crate::gitlab) fn fork(
    session: &Session<'_>,
    target: &Target,
    namespace: Option<&str>,
) -> Result<HostedRepo, ProviderError> {
    let api = session.api()?;
    let body = match namespace {
        Some(group) if !own(session, Some(group)) => json!({ "namespace_path": group }),
        _ => json!({}),
    };
    let (wire, _): (WireProject, _) = api.json(
        Method::Post,
        &format!("{}/fork", project_path(target)),
        Some(&body),
    )?;
    let made = repo(&wire);
    let at = format!("/projects/{}", made.id);
    for _ in 0..MOST_WAITS {
        let (now, _): (WireImport, _) = api.json(Method::Get, &at, None)?;
        match now.import_status.as_deref() {
            None | Some("finished" | "none") => break,
            Some("failed") => {
                return Err(ProviderError::Refused(say!(
                    "gitlab-fork-not-copied",
                    error = api::clean_line(now.import_error.as_deref().unwrap_or_default(), 300)
                )));
            }
            Some(_) => session.pause(WAIT)?,
        }
    }
    Ok(made)
}

/// A key as GitLab answers it.
#[derive(Debug, Deserialize)]
struct WireKey {
    id: u64,
    #[serde(default)]
    title: Option<String>,
}

/// Upload a public key to the user's account.
pub(in crate::gitlab) fn key_add(
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
