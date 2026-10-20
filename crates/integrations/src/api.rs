//! Asking a provider's API: one address it answers at, the token that
//! signs in, JSON in and out, the errors it answers with, and the pages
//! of a list.
//!
//! Every answer is hostile until read: a body is taken up to a size, read
//! into the shapes of the reader that asked (a wrong shape is an error,
//! never a guess), and a `Link` to the next page is followed only when it
//! is on the same API address, never to the same page twice, and at most
//! [`MAX_PAGES`] times. The token goes only to the account's API address
//! or its site, as the caller says; no address an answer names is asked
//! with it otherwise.
//!
//! Sources: docs.github.com "Getting started with the REST API" (the
//! `Accept` and `X-GitHub-Api-Version` lines, `Authorization: Bearer`),
//! "Using pagination in the REST API" (`Link` with `rel="next"`), "Rate
//! limits for the REST API" (`X-RateLimit-Remaining`, `X-RateLimit-Reset`,
//! `Retry-After`), "Troubleshooting the REST API" (the error body's
//! `message`); docs.gitlab.com "REST API" (`Authorization: Bearer`,
//! pagination with `Link`, status codes, the error body's `message` and
//! `error`), "Rate limits" (`RateLimit-Remaining`, `RateLimit-Reset`);
//! RFC 8288 (`Link`); RFC 3339 (the time written for a reset).

use std::{collections::BTreeSet, sync::atomic::AtomicBool, time::Duration};

use serde::de::DeserializeOwned;

use crate::{
    http::{Call, Fetch, FetchError, Limits, Method, Response, Url},
    model::{Freshness, MAX_BODY, MAX_TITLE, ProviderKind, User},
    secrets::Secret,
};

/// Most pages of one list read.
pub const MAX_PAGES: usize = 50;
/// Largest answer read, per page.
pub const MAX_ANSWER: usize = 16 * 1024 * 1024;

/// What it may cost to ask once.
pub fn limits() -> Limits {
    Limits {
        connect: Duration::from_secs(10),
        total: Duration::from_secs(60),
        body: MAX_ANSWER,
    }
}

/// Why a provider did not answer with what was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderError {
    /// Nothing came back, or not an HTTP answer.
    #[error("{0}")]
    Http(#[from] FetchError),
    /// The token was refused.
    #[error("{}", say!("api-unauthorized", said = .0.as_str()))]
    Unauthorized(String),
    /// Not there, or not visible to this account.
    #[error("{}", say!("api-not-found"))]
    NotFound,
    /// The allowance of requests is spent.
    #[error("{}", limited(.0))]
    Limited(Option<String>),
    /// Another status, with what the provider said.
    #[error("{}", say!("api-status", status = *.status, message = .message.as_str()))]
    Status {
        status: u16,
        /// What it said, cut and without control characters.
        message: String,
    },
    /// The answer is not of the shape it should have.
    #[error("{}", say!("api-malformed", error = .0.as_str()))]
    Malformed(String),
    /// What is asked of it cannot be asked.
    #[error("{0}")]
    Refused(String),
}

fn limited(reset: &Option<String>) -> String {
    match reset {
        Some(reset) => say!("api-limited-until", reset = reset.as_str()),
        None => say!("api-limited").to_owned(),
    }
}

/// A text from another machine made fit to show: no control characters
/// (a line break becomes a space), at most `max` characters.
pub fn clean_line(text: &str, max: usize) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(max)
        .collect()
}

/// A title, as [`clean_line`] with [`MAX_TITLE`].
pub fn title(text: &str) -> String {
    clean_line(text, MAX_TITLE)
}

/// A body: at most [`MAX_BODY`] bytes, without control characters other
/// than line breaks and tabs.
pub fn body(text: &str) -> String {
    crate::model::bounded(text, MAX_BODY)
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

/// A person as an answer gave them, cleaned.
pub fn user(
    login: &str,
    name: Option<&str>,
    avatar_url: Option<&str>,
    web_url: Option<&str>,
) -> User {
    let some = |text: Option<&str>, max: usize| {
        text.map(|text| clean_line(text, max))
            .filter(|text| !text.trim().is_empty())
    };
    User {
        login: clean_line(login, 256),
        name: some(name, 256),
        avatar_url: some(avatar_url, 2048),
        web_url: some(web_url, 2048),
    }
}

/// `2026-10-12T08:30:00Z` for seconds since the Unix epoch.
pub fn rfc3339(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let rest = seconds.rem_euclid(86_400);
    // Days to a civil date: `civil_from_days` of "chrono-Compatible
    // Low-Level Date Algorithms".
    let shifted = days.saturating_add(719_468);
    let era = shifted.div_euclid(146_097);
    let of_era = shifted.rem_euclid(146_097);
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096).div_euclid(365);
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era
        .saturating_add(era.saturating_mul(400))
        .saturating_add(i64::from(month <= 2));
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// What the header lines of an answer say about the allowance left, and
/// whether it is one that was kept and given because the provider could
/// not be asked ([`crate::cache`]): then it was read when it was kept.
pub fn freshness(response: &Response, now: i64) -> Freshness {
    let number = |names: &[&str]| {
        names
            .iter()
            .find_map(|name| response.header(name))
            .and_then(|value| value.trim().parse::<u64>().ok())
    };
    let remaining = number(&["x-ratelimit-remaining", "ratelimit-remaining"]);
    let reset = number(&["x-ratelimit-reset", "ratelimit-reset"])
        .and_then(|reset| i64::try_from(reset).ok())
        .map(rfc3339);
    let kept = response
        .header(crate::cache::OFFLINE)
        .and_then(|at| at.trim().parse::<i64>().ok());
    Freshness {
        fetched: Some(rfc3339(kept.unwrap_or(now))),
        offline: kept.is_some(),
        remaining,
        reset,
    }
}

/// How fresh a list read in several answers is: as the latest answer
/// says, with the allowance of the latest one that named it; kept answers
/// in it make all of it kept.
pub fn latest(kept: Freshness, newer: Freshness) -> Freshness {
    let named = newer.remaining.is_some();
    Freshness {
        fetched: newer.fetched.or(kept.fetched),
        offline: kept.offline || newer.offline,
        remaining: newer.remaining.or(kept.remaining),
        reset: if named {
            newer.reset
        } else {
            kept.reset.or(newer.reset)
        },
    }
}

/// The address of the next page, from a `Link` line.
pub fn next_link(link: &str) -> Option<String> {
    link.split(',').find_map(|part| {
        let (target, parameters) = part.split_once(';')?;
        let target = target.trim().strip_prefix('<')?.strip_suffix('>')?;
        let next = parameters.split(';').any(|parameter| {
            let parameter = parameter.trim();
            parameter.eq_ignore_ascii_case("rel=\"next\"")
                || parameter.eq_ignore_ascii_case("rel=next")
        });
        next.then(|| target.to_owned())
    })
}

/// Percent-encode a path's part: GitLab asks for a project by its path
/// with every `/` as `%2F`.
pub fn encode(part: &str) -> String {
    let mut out = String::with_capacity(part.len());
    for byte in part.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A form's body: `a=1&b=two%20words`.
pub fn form(pairs: &[(&str, &str)]) -> Vec<u8> {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
        .into_bytes()
}

/// One provider's API, asked with one token.
#[derive(Debug)]
pub struct Api<'a> {
    fetch: &'a dyn Fetch,
    base: Url,
    kind: ProviderKind,
    token: Option<&'a Secret>,
    cancel: &'a AtomicBool,
}

impl<'a> Api<'a> {
    /// The API at `base` (an account's `api`), signed in with `token`
    /// when there is one.
    pub fn new(
        fetch: &'a dyn Fetch,
        base: &str,
        kind: ProviderKind,
        token: Option<&'a Secret>,
        cancel: &'a AtomicBool,
    ) -> Result<Self, ProviderError> {
        let base = Url::parse(base.trim_end_matches('/'))?;
        if base.path().contains('?') {
            return Err(ProviderError::Refused(
                say!("api-address-with-question").to_owned(),
            ));
        }
        Ok(Self {
            fetch,
            base,
            kind,
            token,
            cancel,
        })
    }

    /// The base path, without a slash at the end.
    fn base_path(&self) -> &str {
        self.base.path().trim_end_matches('/')
    }

    /// The address of `path` (which starts with `/`) on this API.
    pub fn url(&self, path: &str) -> Result<Url, ProviderError> {
        let base = self.base.to_string();
        let base = base.trim_end_matches('/');
        Ok(Url::parse(&format!("{base}{path}"))?)
    }

    /// Whether `url` is on this API: same scheme, host, port and under
    /// its path.
    pub fn owns(&self, url: &Url) -> bool {
        let base = self.base_path();
        url.secure() == self.base.secure()
            && url.host() == self.base.host()
            && url.port() == self.base.port()
            && (base.is_empty()
                || url.path() == base
                || url
                    .path()
                    .strip_prefix(base)
                    .is_some_and(|rest| rest.starts_with(['/', '?'])))
    }

    fn call(
        &self,
        method: Method,
        url: Url,
        body: Option<&serde_json::Value>,
    ) -> Result<Call, ProviderError> {
        let mut call = Call::new(method, url, limits());
        call = match self.kind {
            ProviderKind::Github => call
                .with_header("Accept", "application/vnd.github+json")
                .with_header("X-GitHub-Api-Version", "2022-11-28"),
            ProviderKind::Gitlab => call.with_header("Accept", "application/json"),
        };
        if let Some(token) = self.token {
            call = call.with_header("Authorization", &format!("Bearer {}", token.expose()));
        }
        if let Some(body) = body {
            let bytes =
                serde_json::to_vec(body).map_err(|err| ProviderError::Refused(err.to_string()))?;
            call = call.with_body("application/json", bytes);
        }
        Ok(call)
    }

    /// Ask, and turn an answer that is not a success into its error.
    pub fn send(
        &self,
        method: Method,
        url: Url,
        body: Option<&serde_json::Value>,
    ) -> Result<Response, ProviderError> {
        if !self.owns(&url) {
            return Err(ProviderError::Refused(
                say!("api-address-not-on-api").to_owned(),
            ));
        }
        let call = self.call(method, url, body)?;
        let response = self.fetch.send(&call, self.cancel)?;
        checked(response)
    }

    /// Ask `path` and read the answer as `T`.
    pub fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&serde_json::Value>,
    ) -> Result<(T, Response), ProviderError> {
        let response = self.send(method, self.url(path)?, body)?;
        let value = read(&response.body)?;
        Ok((value, response))
    }

    /// Read the pages of a list from `path`, up to `most` items: the
    /// items, whether there were more, and the last answer.
    pub fn pages<T: DeserializeOwned>(
        &self,
        path: &str,
        most: usize,
    ) -> Result<(Vec<T>, bool, Response), ProviderError> {
        self.pages_where(path, most, |_| true)
    }

    /// Read the pages of a list from `path`, keeping the items `keep`
    /// takes, up to `most` of them: the items, whether there were more,
    /// and the last answer.
    pub fn pages_where<T: DeserializeOwned>(
        &self,
        path: &str,
        most: usize,
        keep: impl Fn(&T) -> bool,
    ) -> Result<(Vec<T>, bool, Response), ProviderError> {
        let mut url = self.url(path)?;
        let mut seen = BTreeSet::new();
        let mut items: Vec<T> = Vec::new();
        // The allowance as the latest page that said it.
        let mut allowance: Vec<(String, String)> = Vec::new();
        let mut pages = 0usize;
        loop {
            seen.insert(url.to_string());
            let mut response = self.send(Method::Get, url.clone(), None)?;
            pages = pages.saturating_add(1);
            let page: Vec<T> = read(&response.body)?;
            items.extend(page.into_iter().filter(|item| keep(item)));
            let said: Vec<(String, String)> = response
                .headers
                .iter()
                .filter(|(name, _)| name.contains("ratelimit"))
                .cloned()
                .collect();
            if said.is_empty() {
                response.headers.extend(allowance.iter().cloned());
            } else {
                allowance = said;
            }
            if items.len() >= most {
                items.truncate(most);
                return Ok((items, true, response));
            }
            let next = response
                .header("link")
                .and_then(next_link)
                .and_then(|next| Url::parse(&next).ok())
                .filter(|next| self.owns(next) && !seen.contains(&next.to_string()));
            match next {
                // At most so many pages: the rest is said to be there.
                Some(_) if pages >= MAX_PAGES => return Ok((items, true, response)),
                Some(next) => url = next,
                None => {
                    let more = response.header("link").and_then(next_link).is_some();
                    return Ok((items, more, response));
                }
            }
        }
    }
}

/// One error of a GraphQL answer.
#[derive(Debug, serde::Deserialize)]
struct GraphqlError {
    #[serde(default)]
    message: Option<String>,
    /// GitHub's kind of error: `NOT_FOUND`, `RATE_LIMITED`, `FORBIDDEN`.
    #[serde(default, rename = "type")]
    kind: Option<String>,
}

/// A GraphQL answer: its data, its errors, or both.
#[derive(Debug, serde::Deserialize)]
struct GraphqlAnswer<T> {
    #[serde(default = "Option::default")]
    data: Option<T>,
    #[serde(default)]
    errors: Option<Vec<GraphqlError>>,
}

impl Api<'_> {
    /// Ask a GraphQL API (this one's address is where it answers) with
    /// `query` and its `variables`, and read the answer's `data` as `T`.
    /// An error that says the thing is not there, or that the allowance
    /// is spent, is that error. Data that came with other errors is taken
    /// as it is (its missing parts are `null`); errors without data are
    /// the provider's first message.
    pub fn graphql<T: DeserializeOwned>(
        &self,
        query: &str,
        variables: &serde_json::Value,
    ) -> Result<(T, Response), ProviderError> {
        let body = serde_json::json!({ "query": query, "variables": variables });
        let response = self.send(Method::Post, self.url("")?, Some(&body))?;
        let answer: GraphqlAnswer<T> = read(&response.body)?;
        let errors = answer.errors.unwrap_or_default();
        let kind = |wanted: &str| {
            errors
                .iter()
                .any(|error| error.kind.as_deref() == Some(wanted))
        };
        if kind("RATE_LIMITED") {
            let reset = response
                .header("x-ratelimit-reset")
                .and_then(|reset| reset.trim().parse::<i64>().ok())
                .map(rfc3339);
            return Err(ProviderError::Limited(reset));
        }
        if kind("NOT_FOUND") {
            return Err(ProviderError::NotFound);
        }
        match answer.data {
            Some(data) => Ok((data, response)),
            None => {
                let message = errors
                    .iter()
                    .find_map(|error| error.message.as_deref())
                    .map(|message| clean_line(message, 300))
                    .unwrap_or_else(|| say!("api-answer-without-data").to_owned());
                Err(ProviderError::Status {
                    status: response.status,
                    message,
                })
            }
        }
    }
}

/// Where a provider's GraphQL API answers, from where its REST API does:
/// `https://api.github.com/graphql`, `https://host/api/graphql` beside
/// `https://host/api/v3` or `https://host/api/v4`.
pub fn graphql_address(api: &str) -> String {
    let api = api.trim_end_matches('/');
    for rest in ["/api/v3", "/api/v4"] {
        if let Some(base) = api.strip_suffix(rest) {
            return format!("{base}/api/graphql");
        }
    }
    format!("{api}/graphql")
}

/// Read a body as `T`.
pub fn read<T: DeserializeOwned>(body: &[u8]) -> Result<T, ProviderError> {
    serde_json::from_slice(body)
        .map_err(|err| ProviderError::Malformed(clean_line(&err.to_string(), 200)))
}

/// What an error's body says: GitHub's and GitLab's `message`, OAuth's
/// `error_description` or `error`.
pub fn said(body: &[u8]) -> String {
    let value: serde_json::Value = serde_json::from_slice(body).unwrap_or_default();
    let text = ["message", "error_description", "error"]
        .iter()
        .find_map(|key| match value.get(key) {
            Some(serde_json::Value::String(text)) => Some(text.clone()),
            Some(other @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => {
                Some(other.to_string())
            }
            _ => None,
        })
        .unwrap_or_else(|| String::from_utf8_lossy(body.get(..200).unwrap_or(body)).into_owned());
    clean_line(&text, 300).trim().to_owned()
}

/// An answer that is a success as it is, or its error.
pub fn checked(response: Response) -> Result<Response, ProviderError> {
    let status = response.status;
    if (200..300).contains(&status) || status == 304 {
        return Ok(response);
    }
    let spent = response
        .header("x-ratelimit-remaining")
        .or_else(|| response.header("ratelimit-remaining"))
        .is_some_and(|left| left.trim() == "0");
    let message = said(&response.body);
    Err(match status {
        401 => ProviderError::Unauthorized(message),
        403 | 429 if spent || status == 429 => {
            let reset = response
                .header("x-ratelimit-reset")
                .or_else(|| response.header("ratelimit-reset"))
                .and_then(|reset| reset.trim().parse::<i64>().ok())
                .map(rfc3339);
            ProviderError::Limited(reset)
        }
        404 => ProviderError::NotFound,
        _ => ProviderError::Status { status, message },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn times_are_written_as_rfc_3339() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(rfc3339(1_791_801_000), "2026-10-12T10:30:00Z");
        assert_eq!(rfc3339(-1), "1969-12-31T23:59:59Z");
        // Nothing that panics, whatever the number.
        let _ = rfc3339(i64::MAX);
        let _ = rfc3339(i64::MIN);
    }

    #[test]
    fn the_next_page_is_read_from_a_link() {
        assert_eq!(
            next_link(
                "<https://api.github.com/x?page=1>; rel=\"prev\", \
                 <https://api.github.com/x?page=3>; rel=\"next\", \
                 <https://api.github.com/x?page=9>; rel=\"last\""
            )
            .as_deref(),
            Some("https://api.github.com/x?page=3")
        );
        assert_eq!(next_link("<https://a/x>; rel=\"last\""), None);
        assert_eq!(next_link("garbage"), None);
        assert_eq!(next_link(""), None);
    }

    #[test]
    fn texts_from_elsewhere_are_made_fit_to_show() {
        assert_eq!(title("a\nb\u{7}c\u{202e}"), "a b c\u{202e}");
        assert_eq!(title(&"x".repeat(5000)).chars().count(), MAX_TITLE);
        assert_eq!(body("line\nnext\u{0}\ttab"), "line\nnext\ttab");
        assert_eq!(encode("group/sub project"), "group%2Fsub%20project");
        assert_eq!(
            form(&[("client_id", "a b"), ("scope", "api read_user")]),
            b"client_id=a%20b&scope=api%20read_user"
        );
    }

    #[test]
    fn graphql_answers_beside_the_rest_api() {
        assert_eq!(
            graphql_address("https://api.github.com"),
            "https://api.github.com/graphql"
        );
        assert_eq!(
            graphql_address("https://ghe.example.org/api/v3/"),
            "https://ghe.example.org/api/graphql"
        );
        assert_eq!(
            graphql_address("http://127.0.0.1:8929/api/v4"),
            "http://127.0.0.1:8929/api/graphql"
        );
    }

    #[test]
    fn errors_say_what_the_provider_said() {
        let answer = |status: u16, headers: &[(&str, &str)], body: &str| Response {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
            ..Default::default()
        };
        assert!(matches!(
            checked(answer(401, &[], r#"{"message":"Bad credentials"}"#)),
            Err(ProviderError::Unauthorized(message)) if message == "Bad credentials"
        ));
        assert_eq!(
            checked(answer(
                403,
                &[("x-ratelimit-remaining", "0"), ("x-ratelimit-reset", "0")],
                "{}"
            )),
            Err(ProviderError::Limited(Some("1970-01-01T00:00:00Z".into())))
        );
        assert_eq!(
            checked(answer(429, &[], "")),
            Err(ProviderError::Limited(None))
        );
        assert_eq!(checked(answer(404, &[], "")), Err(ProviderError::NotFound));
        assert!(matches!(
            checked(answer(422, &[], r#"{"message":{"title":["is too long"]}}"#)),
            Err(ProviderError::Status { status: 422, message }) if message.contains("too long")
        ));
        assert!(matches!(
            checked(answer(500, &[], "\u{0}\u{1}oops\n")),
            Err(ProviderError::Status { status: 500, message }) if message == "oops"
        ));
        assert!(checked(answer(204, &[], "")).is_ok());
    }
}
