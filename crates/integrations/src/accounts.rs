//! The accounts the user connected: what is not secret in
//! `<config_dir>/integrations.toml`, the tokens in the keychain.
//!
//! The file holds one `[[account]]` per account (the fields of
//! [`Account`]: no token) and one `[[host]]` per site that has settings of
//! its own: the application id of the device flow at a server of one's
//! own, and the SSH host names that stand for the site in remotes. It is
//! written beside itself and then put in its place, so that it is whole or
//! the one before; one change at a time in this process.
//!
//! The keychain holds, per account, the JSON text of a [`TokenRecord`]:
//! the token, and for GitLab's device flow the refresh token and when the
//! token runs out. [`token`] hands out a token that is still good,
//! refreshing GitLab's first when it has run out.
//!
//! Sources: docs.github.com "Users" (`GET /user`) and "Scopes for OAuth
//! apps" (`X-OAuth-Scopes`); docs.gitlab.com/api "Users", "Personal
//! access tokens" (`/personal_access_tokens/self`) and "OAuth 2.0 identity
//! provider API" (refresh tokens, token lifetime of two hours); the TOML
//! 1.0 specification (arrays of tables).

use std::{
    fmt, fs,
    path::{Path, PathBuf},
    sync::{Mutex, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};

use crate::{
    api::ProviderError,
    http::Url,
    model::{Account, ProviderKind, SignIn},
    oauth, provider,
    provider::{Identity, Session},
    secrets::Secret,
    services::Services,
};

/// The file's name in the configuration directory.
pub const FILE: &str = "integrations.toml";
/// Most accounts kept.
pub const MAX_ACCOUNTS: usize = 64;

/// One change of the file at a time in this process.
static WRITING: Mutex<()> = Mutex::new(());

/// The settings of a site.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostSettings {
    /// The site, as an account's `web` has it.
    pub web: String,
    /// The application id of the device flow there, for a server of one's
    /// own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Host names that stand for the site in remotes' addresses (aliases
    /// of the SSH configuration).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ssh_hosts: Vec<String>,
}

/// What the file holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Accounts {
    #[serde(default, rename = "account")]
    pub accounts: Vec<Account>,
    /// The settings of sites.
    #[serde(default, rename = "host")]
    pub hosts: Vec<HostSettings>,
}

impl Accounts {
    /// The account with this id.
    pub fn find(&self, id: &str) -> Option<&Account> {
        self.accounts.iter().find(|account| account.id == id)
    }

    /// The settings of a site.
    pub fn host(&self, web: &str) -> Option<&HostSettings> {
        self.hosts.iter().find(|host| host.web == web)
    }

    /// The SSH host names that stand for an account's site.
    pub fn ssh_hosts(&self, account: &Account) -> Vec<String> {
        self.host(&account.web)
            .map(|host| host.ssh_hosts.clone())
            .unwrap_or_default()
    }
}

/// What the keychain keeps for an account.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenRecord {
    /// The token.
    pub access_token: String,
    /// What a new token is asked with when it runs out (GitLab's device
    /// flow).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// When it runs out, in seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

impl fmt::Debug for TokenRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TokenRecord")
            .field("access_token", &"<hidden>")
            .field(
                "refresh_token",
                &self.refresh_token.as_ref().map(|_| "<hidden>"),
            )
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

impl TokenRecord {
    /// A token that does not run out: one made at the provider and pasted.
    pub fn token(token: &Secret) -> Self {
        Self {
            access_token: token.expose().to_owned(),
            refresh_token: None,
            expires_at: None,
        }
    }

    /// The record as the keychain keeps it.
    fn kept(&self) -> Result<Secret, ProviderError> {
        serde_json::to_string(self)
            .map(Secret::new)
            .map_err(|err| ProviderError::Refused(err.to_string()))
    }

    /// The record from what the keychain kept. A bare token, as another
    /// program might have left it, is a token that does not run out.
    fn from_kept(kept: &Secret) -> Self {
        serde_json::from_str(kept.expose()).unwrap_or_else(|_| Self::token(kept))
    }

    /// The token as a secret.
    pub fn secret(&self) -> Secret {
        Secret::new(self.access_token.clone())
    }
}

/// A provider's site: where people open it and where its API answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Site {
    /// The provider.
    pub kind: ProviderKind,
    /// `https://github.com`, `http://127.0.0.1:8929`.
    pub web: String,
    /// `https://api.github.com`, `https://host/api/v3`, `https://host/api/v4`.
    pub api: String,
    /// The host with its port when it is not the usual one.
    pub authority: String,
}

impl Site {
    /// The site of an account.
    pub fn of(account: &Account) -> Result<Self, ProviderError> {
        site(account.kind, Some(&account.web))
    }

    /// Whether it is the public one (github.com, gitlab.com).
    pub fn public(&self) -> bool {
        matches!(
            (self.kind, self.authority.as_str()),
            (ProviderKind::Github, "github.com") | (ProviderKind::Gitlab, "gitlab.com")
        )
    }
}

/// The site of a provider: github.com or gitlab.com when no address is
/// given, else the address read and tidied. Only `https`, or `http` to
/// this machine; no user name, query or fragment.
pub fn site(kind: ProviderKind, web: Option<&str>) -> Result<Site, ProviderError> {
    let given = web.map(str::trim).filter(|web| !web.is_empty());
    let web = given.unwrap_or(match kind {
        ProviderKind::Github => "https://github.com",
        ProviderKind::Gitlab => "https://gitlab.com",
    });
    let refuse = |why: &str| ProviderError::Refused(say!("account-address", why));
    if web.contains(['?', '#']) {
        return Err(refuse(say!("account-address-question-or-fragment")));
    }
    let url = Url::parse(web).map_err(|err| refuse(&err.to_string()))?;
    let loopback = url.host() == "localhost"
        || url
            .host()
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !url.secure() && !loopback {
        return Err(refuse(say!("http-insecure")));
    }
    let scheme = if url.secure() { "https" } else { "http" };
    let path = url.path().trim_end_matches('/');
    let web = format!("{scheme}://{}{path}", url.authority());
    let authority = url.authority();
    let api = match kind {
        ProviderKind::Github if authority == "github.com" && path.is_empty() => {
            "https://api.github.com".to_owned()
        }
        ProviderKind::Github => format!("{web}/api/v3"),
        ProviderKind::Gitlab => format!("{web}/api/v4"),
    };
    Ok(Site {
        kind,
        web,
        api,
        authority,
    })
}

/// An account's id: `<kind>:<host[:port]>:<login>`.
pub fn account_id(site: &Site, login: &str) -> String {
    let kind = match site.kind {
        ProviderKind::Github => "github",
        ProviderKind::Gitlab => "gitlab",
    };
    format!("{kind}:{}:{login}", site.authority)
}

fn file(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

fn unusable(why: impl fmt::Display) -> ProviderError {
    ProviderError::Refused(say!(
        "account-file-unusable",
        file = FILE,
        why = why.to_string()
    ))
}

/// That no account of this id is connected.
pub(crate) fn not_connected(id: &str) -> String {
    say!("account-not-connected", id = format!("{id:?}"))
}

/// What the keychain shows for an account's token.
fn keychain_label(login: &str, authority: &str) -> String {
    say!("account-keychain-label", login, site = authority)
}

/// What the file holds; nothing when there is no file.
pub fn load(dir: &Path) -> Result<Accounts, ProviderError> {
    let text = match fs::read_to_string(file(dir)) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Accounts::default()),
        Err(err) => return Err(unusable(err)),
    };
    if text.len() > 1024 * 1024 {
        return Err(unusable(say!("account-file-too-large")));
    }
    let accounts: Accounts = toml::from_str(&text).map_err(|err| unusable(err.message()))?;
    if accounts.accounts.len() > MAX_ACCOUNTS {
        return Err(unusable(say!("account-file-too-many")));
    }
    Ok(accounts)
}

/// Write the file: beside it first, then in its place, readable by the
/// user only.
fn store(dir: &Path, accounts: &Accounts) -> Result<(), ProviderError> {
    let text = toml::to_string_pretty(accounts).map_err(unusable)?;
    fs::create_dir_all(dir).map_err(unusable)?;
    let path = file(dir);
    let beside = dir.join(format!("{FILE}.tmp"));
    fs::write(&beside, text).map_err(unusable)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = fs::set_permissions(&beside, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&beside, &path).map_err(|err| {
        let _ = fs::remove_file(&beside);
        unusable(err)
    })
}

/// Change the file under the lock: read it, let `change` do its part,
/// write it back.
fn change<T>(
    dir: &Path,
    change: impl FnOnce(&mut Accounts) -> Result<T, ProviderError>,
) -> Result<T, ProviderError> {
    let _writing = WRITING
        .lock()
        .map_err(|_| unusable(say!("account-change-failed-half-way")))?;
    let mut accounts = load(dir)?;
    let answer = change(&mut accounts)?;
    store(dir, &accounts)?;
    Ok(answer)
}

/// The connected accounts, in the order they were added.
pub fn list(services: &Services) -> Result<Vec<Account>, ProviderError> {
    Ok(load(&services.config_dir)?.accounts)
}

/// The account with this id.
pub fn find(services: &Services, id: &str) -> Result<Account, ProviderError> {
    load(&services.config_dir)?
        .find(id)
        .cloned()
        .ok_or_else(|| ProviderError::Refused(not_connected(id)))
}

/// Who a token belongs to, asked of the provider.
pub fn whoami(
    services: &Services,
    site: &Site,
    token: &Secret,
    cancel: &AtomicBool,
) -> Result<Identity, ProviderError> {
    let probe = Account {
        id: String::new(),
        kind: site.kind,
        web: site.web.clone(),
        api: site.api.clone(),
        login: String::new(),
        name: None,
        avatar_url: None,
        sign_in: SignIn::Token,
        scopes: Vec::new(),
        default: false,
    };
    let session = Session {
        account: &probe,
        token,
        fetch: services.fetch.as_ref(),
        cancel,
        now: (services.now)(),
    };
    let identity = provider::of(site.kind).whoami(&session)?;
    if identity.login.is_empty()
        || !identity
            .login
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Err(ProviderError::Malformed(
            say!("account-odd-login").to_owned(),
        ));
    }
    Ok(identity)
}

/// Connect an account: ask the provider whose the token is, keep the
/// token in the keychain and the account in the file. The first account
/// at a site is its default. Connecting the same account again replaces
/// its token and what is known of it.
pub fn add(
    services: &Services,
    site: &Site,
    record: TokenRecord,
    sign_in: SignIn,
    granted: Option<Vec<String>>,
    cancel: &AtomicBool,
) -> Result<(Account, Identity), ProviderError> {
    if record.access_token.trim().is_empty() {
        return Err(ProviderError::Refused(
            say!("account-token-empty").to_owned(),
        ));
    }
    let identity = whoami(services, site, &record.secret(), cancel)?;
    let scopes = if identity.scopes.is_empty() {
        granted.unwrap_or_default()
    } else {
        identity.scopes.clone()
    };
    let mut account = Account {
        id: account_id(site, &identity.login),
        kind: site.kind,
        web: site.web.clone(),
        api: site.api.clone(),
        login: identity.login.clone(),
        name: identity.name.clone(),
        avatar_url: identity.avatar_url.clone(),
        sign_in,
        scopes,
        default: false,
    };
    let label = keychain_label(&account.login, &site.authority);
    let added = change(&services.config_dir, |accounts| {
        let before = accounts.find(&account.id).map(|known| known.default);
        let others_default = accounts
            .accounts
            .iter()
            .any(|known| known.web == account.web && known.default && known.id != account.id);
        account.default = before.unwrap_or(false) || !others_default;
        if before.is_none() && accounts.accounts.len() >= MAX_ACCOUNTS {
            return Err(ProviderError::Refused(say!(
                "account-too-many",
                most = MAX_ACCOUNTS
            )));
        }
        // The token first: an account without one in the keychain is never
        // written down.
        services
            .secrets
            .set(&account.id, &label, &record.kept()?)
            .map_err(|err| ProviderError::Refused(err.to_string()))?;
        match accounts
            .accounts
            .iter_mut()
            .find(|known| known.id == account.id)
        {
            Some(known) => *known = account.clone(),
            None => accounts.accounts.push(account.clone()),
        }
        Ok(account.clone())
    })?;
    Ok((added, identity))
}

/// Disconnect an account: out of the file and out of the keychain. When
/// it was its site's default, the next one there becomes it.
pub fn remove(services: &Services, id: &str) -> Result<(), ProviderError> {
    change(&services.config_dir, |accounts| {
        let at = accounts
            .accounts
            .iter()
            .position(|account| account.id == id)
            .ok_or_else(|| ProviderError::Refused(not_connected(id)))?;
        let gone = accounts.accounts.remove(at);
        if gone.default
            && let Some(next) = accounts
                .accounts
                .iter_mut()
                .find(|account| account.web == gone.web)
        {
            next.default = true;
        }
        services
            .secrets
            .delete(id)
            .map_err(|err| ProviderError::Refused(err.to_string()))
    })
}

/// Make an account its site's default.
pub fn set_default(services: &Services, id: &str) -> Result<(), ProviderError> {
    change(&services.config_dir, |accounts| {
        let web = accounts
            .find(id)
            .map(|account| account.web.clone())
            .ok_or_else(|| ProviderError::Refused(not_connected(id)))?;
        for account in accounts
            .accounts
            .iter_mut()
            .filter(|account| account.web == web)
        {
            account.default = account.id == id;
        }
        Ok(())
    })
}

/// Keep the device flow's application id for a server of one's own.
pub fn set_client_id(services: &Services, web: &str, client_id: &str) -> Result<(), ProviderError> {
    change(&services.config_dir, |accounts| {
        match accounts.hosts.iter_mut().find(|host| host.web == web) {
            Some(host) => host.client_id = Some(client_id.to_owned()),
            None => accounts.hosts.push(HostSettings {
                web: web.to_owned(),
                client_id: Some(client_id.to_owned()),
                ssh_hosts: Vec::new(),
            }),
        }
        Ok(())
    })
}

/// How long before it runs out a token is refreshed.
const MARGIN: i64 = 60;

/// A token of the account that is still good: the one kept, or, when it
/// has run out and there is a refresh token, a new one asked for with it
/// and kept in its place.
pub fn token(
    services: &Services,
    account: &Account,
    cancel: &AtomicBool,
) -> Result<Secret, ProviderError> {
    let kept = services
        .secrets
        .get(&account.id)
        .map_err(|err| ProviderError::Refused(err.to_string()))?
        .ok_or_else(|| {
            ProviderError::Unauthorized(say!("account-no-token", id = account.id.as_str()))
        })?;
    let record = TokenRecord::from_kept(&kept);
    let now = (services.now)();
    let ran_out = record
        .expires_at
        .is_some_and(|at| at <= now.saturating_add(MARGIN));
    let Some(refresh) = record.refresh_token.as_deref().filter(|_| ran_out) else {
        return Ok(record.secret());
    };
    let site = Site::of(account)?;
    let accounts = load(&services.config_dir)?;
    let client_id = oauth::client_id(services, &site, &accounts)
        .ok_or_else(|| ProviderError::Unauthorized(say!("account-token-ran-out").to_owned()))?;
    let fresh = oauth::refresh(services, &site, &client_id, &Secret::new(refresh), cancel)?;
    let label = keychain_label(&account.login, &site.authority);
    services
        .secrets
        .set(&account.id, &label, &fresh.kept()?)
        .map_err(|err| ProviderError::Refused(err.to_string()))?;
    Ok(fresh.secret())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sites_are_read_and_their_api_found() {
        let github = site(ProviderKind::Github, None).unwrap();
        assert_eq!(
            (
                github.web.as_str(),
                github.api.as_str(),
                github.authority.as_str()
            ),
            ("https://github.com", "https://api.github.com", "github.com")
        );
        assert!(github.public());
        let enterprise = site(ProviderKind::Github, Some(" https://GHE.example.org/ ")).unwrap();
        assert_eq!(enterprise.api, "https://ghe.example.org/api/v3");
        assert!(!enterprise.public());
        let local = site(ProviderKind::Gitlab, Some("http://127.0.0.1:8929")).unwrap();
        assert_eq!(local.web, "http://127.0.0.1:8929");
        assert_eq!(local.api, "http://127.0.0.1:8929/api/v4");
        assert_eq!(account_id(&local, "root"), "gitlab:127.0.0.1:8929:root");
        let under = site(ProviderKind::Gitlab, Some("https://example.org/gitlab/")).unwrap();
        assert_eq!(under.api, "https://example.org/gitlab/api/v4");
        assert_eq!(
            site(ProviderKind::Gitlab, Some("")).unwrap().web,
            "https://gitlab.com"
        );
        for odd in [
            "http://gitlab.example.org",
            "ftp://example.org",
            "https://user@example.org",
            "https://example.org/?x=1",
            "https://example.org/#x",
            "not an address",
        ] {
            assert!(site(ProviderKind::Gitlab, Some(odd)).is_err(), "{odd}");
        }
    }

    #[test]
    fn a_token_record_never_shows_its_tokens() {
        let record = TokenRecord {
            access_token: "access-secret".into(),
            refresh_token: Some("refresh-secret".into()),
            expires_at: Some(5),
        };
        let text = format!("{record:?}");
        assert!(!text.contains("secret"), "{text}");
        let kept = record.kept().unwrap();
        assert_eq!(TokenRecord::from_kept(&kept), record);
        // A bare token is one that does not run out.
        let bare = TokenRecord::from_kept(&Secret::new("glpat-abc"));
        assert_eq!(bare.access_token, "glpat-abc");
        assert_eq!(bare.expires_at, None);
    }

    #[test]
    fn the_file_holds_no_token() {
        let dir = std::env::temp_dir().join(format!("bitbraid-accounts-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let accounts = Accounts {
            accounts: vec![Account {
                id: "github:github.com:octo".into(),
                kind: ProviderKind::Github,
                web: "https://github.com".into(),
                api: "https://api.github.com".into(),
                login: "octo".into(),
                name: None,
                avatar_url: Some("https://avatars.example/1".into()),
                sign_in: SignIn::Token,
                scopes: vec!["repo".into()],
                default: true,
            }],
            hosts: vec![HostSettings {
                web: "https://gitlab.example.org".into(),
                client_id: Some("abc".into()),
                ssh_hosts: vec!["work".into()],
            }],
        };
        store(&dir, &accounts).unwrap();
        assert_eq!(load(&dir).unwrap(), accounts);
        let text = fs::read_to_string(dir.join(FILE)).unwrap();
        assert!(
            text.contains("[[account]]") && text.contains("[[host]]"),
            "{text}"
        );
        assert!(
            !text.contains("access_token") && !text.contains("refresh"),
            "{text}"
        );
        fs::write(dir.join(FILE), "[[account]]\nid = 7\n").unwrap();
        assert!(load(&dir).is_err());
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(load(&dir).unwrap(), Accounts::default());
    }
}
