//! Signing in by a code typed at the provider's site: the OAuth 2.0
//! device authorization grant, and the renewal of GitLab's tokens,
//! which last two hours.
//!
//! [`start`] asks the site for a code and answers with what the user is
//! to do ([`DeviceCode`]); [`poll`] asks, at the interval given, whether
//! the user did it, and in the end gets the token. Both speak to the site
//! itself (`github.com/login/…`, `<gitlab>/oauth/…`), not its API, with a
//! form for a body and JSON for an answer. The application's id is public
//! (the application is not confidential and has no secret): built in for
//! github.com and gitlab.com, given by the user for a server of one's own,
//! and replaced while developing by `BITBRAID_GITHUB_CLIENT_ID` and
//! `BITBRAID_GITLAB_CLIENT_ID`. The built-in ones are the project's own
//! applications; for a server of one's own the device flow is offered
//! only where an id was given.
//!
//! What the site answers is checked: the place the user is sent to has to
//! be on the site's own host, the codes are short printable text, the
//! numbers are kept within sense.
//!
//! Sources: RFC 8628 (OAuth 2.0 Device Authorization Grant: the request
//! and answer of §3.1–3.2, the polling of §3.4–3.5 with
//! `authorization_pending`, `slow_down` adding five seconds,
//! `access_denied`, `expired_token`); RFC 6749 §6 (refreshing a token);
//! docs.github.com "Authorizing OAuth apps" (device flow: `POST
//! /login/device/code`, `POST /login/oauth/access_token`, errors answered
//! with status 200, `Accept: application/json`) and "Scopes for OAuth
//! apps"; docs.gitlab.com/api "OAuth 2.0 identity provider API" (device
//! authorization grant, `POST /oauth/authorize_device`, `POST
//! /oauth/token`, refresh tokens), docs.gitlab.com "Configure GitLab as an
//! OAuth 2.0 authentication identity provider" (an application's "Device
//! authorization grant", off unless turned on in its form; an
//! application made through the API has it off).

use std::sync::atomic::AtomicBool;

use serde::Deserialize;

use crate::{
    accounts::{Accounts, Site, TokenRecord},
    api::{self, ProviderError},
    http::{Call, Method, Url},
    model::{DeviceCode, ProviderKind},
    secrets::Secret,
    services::Services,
};

/// The application id of the device flow at github.com: the project's
/// OAuth application, registered with the device flow on. Public: the
/// application has no secret.
pub const GITHUB_CLIENT_ID: &str = "Ov23liPgVA7Yjwt3STdA";
/// The application id of the device flow at gitlab.com: the project's
/// application, not confidential, with its device authorization grant
/// on. Public: the application has no secret.
pub const GITLAB_CLIENT_ID: &str =
    "e564fd7dd39d984f4ed6a9cc4f6a5ed042fc1c39a726fbce8101352732f60d31";

/// What is asked of GitHub: the repositories, the organisations, the
/// public keys, the user and their addresses, and the workflows.
pub const GITHUB_SCOPES: &str = "repo read:org admin:public_key read:user user:email workflow";
/// What is asked of a GitLab of one's own: its API, and the user.
pub const GITLAB_SCOPES: &str = "api read_user";
/// What is asked of gitlab.com: its API, which reads the user too. The
/// built-in application was registered for `api` alone, and gitlab.com
/// refuses a code asked for with a scope the application does not have
/// (`invalid_scope`).
pub const GITLAB_COM_SCOPES: &str = "api";

const GRANT: &str = "urn:ietf:params:oauth:grant-type:device_code";

/// Longest a code may last, in seconds.
const MAX_EXPIRES: u64 = 3600;
/// Longest wait between asks, in seconds.
const MAX_INTERVAL: u64 = 120;

/// The application id of the device flow at a site, when there is one.
pub fn client_id(services: &Services, site: &Site, accounts: &Accounts) -> Option<String> {
    let id = match (site.kind, site.public()) {
        (ProviderKind::Github, true) => Some(services.client_ids.github.clone()),
        (ProviderKind::Gitlab, true) => Some(services.client_ids.gitlab.clone()),
        (_, false) => accounts
            .host(&site.web)
            .and_then(|host| host.client_id.clone()),
    };
    id.map(|id| id.trim().to_owned()).filter(|id| usable_id(id))
}

/// Whether an application id can be sent as it is.
pub fn usable_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Where a sign-in by code stands, with the token when it is done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Polled {
    /// The user has not answered yet.
    Waiting,
    /// Asked too often: wait this many seconds between asks.
    SlowDown(u64),
    /// The user said no.
    Denied,
    /// The code ran out.
    Expired,
    /// The token, and the scopes it was granted when the site said.
    Token(TokenRecord, Option<Vec<String>>),
}

fn endpoint(site: &Site, path: &str) -> Result<Url, ProviderError> {
    Ok(Url::parse(&format!("{}{path}", site.web))?)
}

/// Ask the site with a form, and read the JSON it answers with whatever
/// its status (the errors of the device flow are such answers).
fn post_form(
    services: &Services,
    url: Url,
    pairs: &[(&str, &str)],
    cancel: &AtomicBool,
) -> Result<(u16, serde_json::Value), ProviderError> {
    let call = Call::new(Method::Post, url, api::limits())
        .with_header("Accept", "application/json")
        .with_body("application/x-www-form-urlencoded", api::form(pairs));
    let response = services.fetch.send(&call, cancel)?;
    let value = serde_json::from_slice(&response.body).map_err(|_| {
        if (200..300).contains(&response.status) {
            ProviderError::Malformed(say!("oauth-not-json").to_owned())
        } else {
            match api::checked(response.clone()) {
                Err(err) => err,
                Ok(_) => ProviderError::Malformed(say!("oauth-not-json").to_owned()),
            }
        }
    })?;
    Ok((response.status, value))
}

/// A short printable text of an answer.
fn short_text(value: &serde_json::Value, key: &str, max: usize) -> Option<String> {
    value
        .get(key)?
        .as_str()
        .map(str::trim)
        .filter(|text| !text.is_empty() && text.len() <= max)
        .filter(|text| text.bytes().all(|b| (0x21..=0x7e).contains(&b)))
        .map(str::to_owned)
}

#[derive(Debug, Deserialize)]
struct WireCode {
    device_code: String,
    user_code: String,
    verification_uri: String,
    #[serde(default)]
    expires_in: Option<u64>,
    #[serde(default)]
    interval: Option<u64>,
}

/// Ask the site for a code the user types there.
pub fn start(
    services: &Services,
    site: &Site,
    client_id: &str,
    cancel: &AtomicBool,
) -> Result<DeviceCode, ProviderError> {
    if !usable_id(client_id) {
        return Err(ProviderError::Refused(say!("oauth-unusable-id").to_owned()));
    }
    let (path, scopes) = match site.kind {
        ProviderKind::Github => ("/login/device/code", GITHUB_SCOPES),
        ProviderKind::Gitlab if site.public() => ("/oauth/authorize_device", GITLAB_COM_SCOPES),
        ProviderKind::Gitlab => ("/oauth/authorize_device", GITLAB_SCOPES),
    };
    let (status, value) = post_form(
        services,
        endpoint(site, path)?,
        &[("client_id", client_id), ("scope", scopes)],
        cancel,
    )?;
    if !(200..300).contains(&status) || value.get("error").is_some() {
        let mut message = api::said(value.to_string().as_bytes());
        // GitLab 18.10 and later make an application with its device grant
        // off, and say no more than this when asked.
        if site.kind == ProviderKind::Gitlab
            && value.get("error").and_then(|error| error.as_str()) == Some("access_denied")
        {
            message.push(' ');
            message.push_str(say!("oauth-device-grant-off"));
        }
        return Err(ProviderError::Status { status, message });
    }
    let wire: WireCode = serde_json::from_value(value)
        .map_err(|err| ProviderError::Malformed(api::clean_line(&err.to_string(), 200)))?;
    let printable = |text: &str, max: usize| {
        text.len() <= max && text.bytes().all(|b| (0x21..=0x7e).contains(&b))
    };
    if !printable(&wire.device_code, 512) || wire.device_code.is_empty() {
        return Err(ProviderError::Malformed(
            say!("oauth-device-code").to_owned(),
        ));
    }
    if !printable(&wire.user_code, 64) || wire.user_code.is_empty() {
        return Err(ProviderError::Malformed(say!("oauth-user-code").to_owned()));
    }
    // The user is sent there: it has to be the site itself.
    let verification = Url::parse(&wire.verification_uri)
        .map_err(|_| ProviderError::Malformed(say!("oauth-verification").to_owned()))?;
    let home = Url::parse(&site.web)?;
    if verification.host() != home.host() || verification.secure() != home.secure() {
        return Err(ProviderError::Malformed(
            say!("oauth-verification-elsewhere").to_owned(),
        ));
    }
    Ok(DeviceCode {
        user_code: wire.user_code,
        verification_uri: verification.to_string(),
        device_code: wire.device_code,
        expires_in: wire.expires_in.unwrap_or(900).clamp(1, MAX_EXPIRES),
        interval: wire.interval.unwrap_or(5).clamp(1, MAX_INTERVAL),
    })
}

/// The token of an answer that has one.
fn token_of(value: &serde_json::Value, now: i64) -> Result<Polled, ProviderError> {
    let access_token = short_text(value, "access_token", 4096)
        .ok_or_else(|| ProviderError::Malformed(say!("oauth-no-token").to_owned()))?;
    let refresh_token = short_text(value, "refresh_token", 4096);
    let expires_at = value
        .get("expires_in")
        .and_then(serde_json::Value::as_u64)
        .map(|seconds| seconds.min(10 * 365 * 86_400))
        .and_then(|seconds| i64::try_from(seconds).ok())
        .map(|seconds| now.saturating_add(seconds));
    let scopes = value
        .get("scope")
        .and_then(serde_json::Value::as_str)
        .map(|scope| {
            scope
                .split([' ', ','])
                .map(str::trim)
                .filter(|scope| !scope.is_empty())
                .take(64)
                .map(|scope| api::clean_line(scope, 64))
                .collect()
        });
    Ok(Polled::Token(
        TokenRecord {
            access_token,
            refresh_token,
            expires_at,
        },
        scopes,
    ))
}

/// Ask whether the user typed the code; the token when they did.
/// `interval` is the wait so far, which a `slow_down` without a number
/// makes five seconds longer.
pub fn poll(
    services: &Services,
    site: &Site,
    client_id: &str,
    device_code: &str,
    interval: u64,
    cancel: &AtomicBool,
) -> Result<Polled, ProviderError> {
    if !usable_id(client_id) {
        return Err(ProviderError::Refused(say!("oauth-unusable-id").to_owned()));
    }
    let path = match site.kind {
        ProviderKind::Github => "/login/oauth/access_token",
        ProviderKind::Gitlab => "/oauth/token",
    };
    let (status, value) = post_form(
        services,
        endpoint(site, path)?,
        &[
            ("client_id", client_id),
            ("device_code", device_code),
            ("grant_type", GRANT),
        ],
        cancel,
    )?;
    if let Some(error) = value.get("error").and_then(serde_json::Value::as_str) {
        return Ok(match error {
            "authorization_pending" => Polled::Waiting,
            "slow_down" => Polled::SlowDown(
                value
                    .get("interval")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_else(|| interval.saturating_add(5))
                    .clamp(1, MAX_INTERVAL),
            ),
            "access_denied" => Polled::Denied,
            "expired_token" => Polled::Expired,
            _ => {
                return Err(ProviderError::Status {
                    status,
                    message: api::said(value.to_string().as_bytes()),
                });
            }
        });
    }
    if !(200..300).contains(&status) {
        return Err(ProviderError::Status {
            status,
            message: api::said(value.to_string().as_bytes()),
        });
    }
    token_of(&value, (services.now)())
}

/// Ask for a new token with a refresh token (GitLab).
pub fn refresh(
    services: &Services,
    site: &Site,
    client_id: &str,
    refresh_token: &Secret,
    cancel: &AtomicBool,
) -> Result<TokenRecord, ProviderError> {
    if site.kind != ProviderKind::Gitlab {
        return Err(ProviderError::Refused(
            say!("oauth-only-gitlab-renews").to_owned(),
        ));
    }
    let (status, value) = post_form(
        services,
        endpoint(site, "/oauth/token")?,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.expose()),
            ("client_id", client_id),
        ],
        cancel,
    )?;
    if !(200..300).contains(&status) || value.get("error").is_some() {
        return Err(ProviderError::Unauthorized(api::said(
            value.to_string().as_bytes(),
        )));
    }
    match token_of(&value, (services.now)())? {
        Polled::Token(record, _) => Ok(record),
        _ => Err(ProviderError::Malformed(say!("oauth-no-token").to_owned())),
    }
}
