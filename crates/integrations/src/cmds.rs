//! What the provider commands share: reading their parameters, finding
//! the services, the account and the repository a command is about, and
//! turning what went wrong into the engine's error.
//!
//! Every parameter is hostile until read: a text has a length it may not
//! pass and no control characters, a number a bound, a list a count; a
//! value of the wrong type is refused with the parameter's name. An owner
//! and a name are held to what a provider allows in a path.
//!
//! A command that is about a repository takes `account`, `owner` and
//! `name`; whichever is absent comes from the open repository's first link
//! (`origin` first, `provider.links`).
//!
//! Sources: none (original design).

use std::sync::Arc;

use bitbraid_engine::{
    Access, CommandContext, CommandError, Effect, Needs, Output, ParamKind, ParamSpec, Params,
};
use bitbraid_strings::Message;
use serde::Serialize;

use crate::{
    accounts::{self, Accounts},
    api::ProviderError,
    cache::Kept,
    http::FetchError,
    model::{Account, ProviderKind, RepoLink},
    provider::{self, Provider, Session, Target},
    remotes,
    secrets::Secret,
    services::Services,
};

/// Needs no repository, and changes none.
pub(crate) const FREE: Access = Access::FREE;
/// Reads the open repository when there is one (its link), and changes
/// none.
pub(crate) const READS_IF_OPEN: Access = Access::READS_IF_OPEN;
/// Reads the open repository, which it needs.
pub(crate) const READS: Access = Access::READS;
/// Changes the open repository, which it needs.
pub(crate) const WRITES: Access = Access::WRITES;
/// Changes the open repository when there is one and it is asked to.
pub(crate) const WRITES_IF_OPEN: Access = Access {
    repository: Needs::RepositoryIfOpen,
    effect: Effect::Writes,
};

/// A required parameter.
pub(crate) const fn req(name: &'static str, kind: ParamKind, description: Message) -> ParamSpec {
    ParamSpec {
        name,
        kind,
        required: true,
        description,
    }
}

/// An optional parameter.
pub(crate) const fn opt(name: &'static str, kind: ParamKind, description: Message) -> ParamSpec {
    ParamSpec {
        name,
        kind,
        required: false,
        description,
    }
}

/// `account`, for a command about a repository.
pub(crate) const ACCOUNT: ParamSpec = opt(
    "account",
    ParamKind::Text,
    message!("commands-param-account"),
);
pub(crate) const OWNER: ParamSpec = opt("owner", ParamKind::Text, message!("commands-param-owner"));
pub(crate) const NAME: ParamSpec = opt("name", ParamKind::Text, message!("commands-param-name"));
pub(crate) const FRESH: ParamSpec = opt("fresh", ParamKind::Flag, message!("commands-param-fresh"));
pub(crate) const KIND: ParamSpec = req(
    "kind",
    ParamKind::Choice(&["github", "gitlab"]),
    message!("commands-param-kind"),
);
pub(crate) const WEB: ParamSpec = opt("web", ParamKind::Text, message!("commands-param-web"));
pub(crate) const NUMBER: ParamSpec = req(
    "number",
    ParamKind::Number,
    message!("commands-param-number"),
);

/// Longest text parameter, unless a command says otherwise.
pub(crate) const MAX_TEXT: usize = 4096;
/// Longest body a command sends (a description, a comment).
pub(crate) const MAX_SENT_BODY: usize = 256 * 1024;

/// The engine's error for a provider's.
pub(crate) fn failed(err: ProviderError) -> CommandError {
    match err {
        ProviderError::Http(FetchError::Cancelled) => {
            CommandError::Git(bitbraid_git::Error::Invalid {
                what: say!("commands-what-operation"),
                detail: say!("commands-cancelled").to_owned(),
            })
        }
        other => CommandError::Git(bitbraid_git::Error::Refused {
            what: say!("commands-what-provider"),
            detail: other.to_string(),
        }),
    }
}

pub(crate) fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

/// A command's answer.
pub(crate) fn answer<T: Serialize>(value: &T) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(value)?)
}

fn given<'a>(params: &'a Params, name: &str) -> Option<&'a serde_json::Value> {
    params.get(name).filter(|value| !value.is_null())
}

/// A text parameter: at most `max` bytes, no control characters but line
/// breaks and tabs where `lines` says so.
pub(crate) fn text_with(
    params: &Params,
    name: &str,
    max: usize,
    lines: bool,
) -> Result<Option<String>, CommandError> {
    let Some(value) = given(params, name) else {
        return Ok(None);
    };
    let text = value
        .as_str()
        .ok_or_else(|| invalid(name, say!("commands-is-text")))?;
    if text.len() > max {
        return Err(invalid(name, say!("commands-too-long", max)));
    }
    if text
        .chars()
        .any(|c| c.is_control() && !(lines && matches!(c, '\n' | '\t' | '\r')))
    {
        return Err(invalid(name, say!("commands-control-characters")));
    }
    Ok(Some(text.to_owned()))
}

/// A one-line text parameter.
pub(crate) fn text(params: &Params, name: &str) -> Result<Option<String>, CommandError> {
    text_with(params, name, MAX_TEXT, false)
}

/// A one-line text parameter that has to be given and not be blank.
pub(crate) fn required(params: &Params, name: &str) -> Result<String, CommandError> {
    text(params, name)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| invalid(name, say!("commands-missing")))
}

/// A yes or no.
pub(crate) fn flag(params: &Params, name: &str) -> Result<Option<bool>, CommandError> {
    match given(params, name) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| invalid(name, say!("commands-is-yes-or-no"))),
    }
}

/// A whole number from `min` to `max`.
pub(crate) fn number(
    params: &Params,
    name: &str,
    min: u64,
    max: u64,
) -> Result<Option<u64>, CommandError> {
    let Some(value) = given(params, name) else {
        return Ok(None);
    };
    let number = value
        .as_u64()
        .ok_or_else(|| invalid(name, say!("commands-is-whole-number")))?;
    if !(min..=max).contains(&number) {
        return Err(invalid(name, say!("commands-out-of-range", min, max)));
    }
    Ok(Some(number))
}

/// The number of a pull request or an issue.
pub(crate) fn item_number(params: &Params) -> Result<u64, CommandError> {
    number(params, "number", 1, u64::from(u32::MAX))?
        .ok_or_else(|| invalid("number", say!("commands-missing")))
}

/// One of a few words; `default` when absent.
pub(crate) fn choice<'a>(
    params: &Params,
    name: &str,
    choices: &[&'a str],
    default: &'a str,
) -> Result<&'a str, CommandError> {
    let Some(value) = given(params, name) else {
        return Ok(default);
    };
    let word = value
        .as_str()
        .ok_or_else(|| invalid(name, say!("commands-is-text")))?;
    choices
        .iter()
        .find(|choice| **choice == word)
        .copied()
        .ok_or_else(|| {
            invalid(
                name,
                say!("commands-is-one-of", choices = choices.join(", ")),
            )
        })
}

/// A list of one-line texts: at most `most` of them.
pub(crate) fn list(
    params: &Params,
    name: &str,
    most: usize,
) -> Result<Option<Vec<String>>, CommandError> {
    let Some(value) = given(params, name) else {
        return Ok(None);
    };
    let items = value
        .as_array()
        .ok_or_else(|| invalid(name, say!("commands-is-list")))?;
    if items.len() > most {
        return Err(invalid(name, say!("commands-too-many-items", most)));
    }
    items
        .iter()
        .map(|item| {
            let text = item
                .as_str()
                .ok_or_else(|| invalid(name, say!("commands-is-list-of-texts")))?;
            if text.is_empty() || text.len() > 512 || text.chars().any(char::is_control) {
                return Err(invalid(name, say!("commands-unusable-item")));
            }
            Ok(text.to_owned())
        })
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

/// The provider a command names.
pub(crate) fn kind(params: &Params) -> Result<ProviderKind, CommandError> {
    match choice(params, "kind", &["github", "gitlab"], "")? {
        "github" => Ok(ProviderKind::Github),
        "gitlab" => Ok(ProviderKind::Gitlab),
        _ => Err(invalid("kind", say!("commands-missing"))),
    }
}

/// The services of a command's context.
pub(crate) fn services(ctx: &CommandContext) -> Result<Arc<Services>, CommandError> {
    Services::of(ctx).map_err(failed)
}

/// The links of the open repository's remotes to the accounts.
pub(crate) fn links(
    ctx: &CommandContext,
    accounts: &Accounts,
) -> Result<Vec<RepoLink>, CommandError> {
    let repo = ctx.repo()?;
    // The remotes' addresses alone: a whole snapshot reads every reference.
    let remotes: Vec<(String, String)> = bitbraid_git::remote_fetch_urls(repo)
        .into_iter()
        .filter_map(|(name, url)| Some((name, url?)))
        .collect();
    Ok(remotes::links(&remotes, &accounts.accounts, |account| {
        accounts.ssh_hosts(account)
    }))
}

/// An owner path as a provider allows it: parts of letters, digits, `-`,
/// `_` and `.`, joined by `/`.
fn owner_path(name: &str, text: &str) -> Result<String, CommandError> {
    let parts: Vec<&str> = text.split('/').collect();
    if parts.len() > 20 || !parts.iter().all(|part| remotes::path_part(part)) {
        return Err(invalid(name, say!("commands-not-a-path")));
    }
    Ok(text.to_owned())
}

/// The account a command is about, from `account` or the open
/// repository's link, and that link's repository when there is one.
fn account_and_link(
    ctx: &CommandContext,
    params: &Params,
    accounts: &Accounts,
) -> Result<(Account, Option<RepoLink>), CommandError> {
    let named = text(params, "account")?;
    let link = if ctx.repo.is_some() {
        let all = links(ctx, accounts)?;
        match &named {
            Some(id) => all.into_iter().find(|link| &link.account == id),
            None => all.into_iter().next(),
        }
    } else {
        None
    };
    let id = match (&named, &link) {
        (Some(id), _) => id.clone(),
        (None, Some(link)) => link.account.clone(),
        (None, None) => {
            return Err(CommandError::Git(bitbraid_git::Error::Refused {
                what: say!("commands-what-provider"),
                detail: say!("commands-no-account").to_owned(),
            }));
        }
    };
    let account = accounts
        .find(&id)
        .cloned()
        .ok_or_else(|| invalid("account", accounts::not_connected(&id)))?;
    Ok((account, link))
}

/// What a command about an account works with.
pub(crate) struct Signed {
    pub services: Arc<Services>,
    pub account: Account,
    pub token: Secret,
    /// The network, with the account's kept answers in front of it.
    pub fetch: Kept,
}

impl Signed {
    /// The account, signed in with `token`; its reads kept in the cache
    /// directory, and asked without offering what was kept when the
    /// command's `fresh` says so.
    fn new(
        services: Arc<Services>,
        account: Account,
        token: Secret,
        params: &Params,
    ) -> Result<Self, CommandError> {
        let fresh = flag(params, "fresh")?.unwrap_or(false);
        let fetch = Kept::new(
            services.fetch.clone(),
            services.cache_dir.as_deref(),
            &account.id,
            fresh,
            services.now,
        );
        Ok(Self {
            services,
            account,
            token,
            fetch,
        })
    }

    /// A session of the account.
    pub fn session<'a>(&'a self, ctx: &'a CommandContext) -> Session<'a> {
        Session {
            account: &self.account,
            token: &self.token,
            fetch: &self.fetch,
            cancel: &ctx.cancel,
            now: (self.services.now)(),
        }
    }

    /// The account's provider.
    pub fn provider(&self) -> &'static dyn Provider {
        provider::of(self.account.kind)
    }
}

/// This account, signed in with the services of the command: for a
/// command that asks several accounts in turn (the inbox).
pub(crate) fn signed_as(
    ctx: &CommandContext,
    params: &Params,
    services: &Arc<Services>,
    account: Account,
) -> Result<Signed, CommandError> {
    let token = accounts::token(services, &account, &ctx.cancel).map_err(failed)?;
    Signed::new(services.clone(), account, token, params)
}

/// The account a command is about, signed in.
pub(crate) fn signed(ctx: &CommandContext, params: &Params) -> Result<Signed, CommandError> {
    let services = services(ctx)?;
    let accounts = accounts::load(&services.config_dir).map_err(failed)?;
    let (account, _) = account_and_link(ctx, params, &accounts)?;
    let token = accounts::token(&services, &account, &ctx.cancel).map_err(failed)?;
    Signed::new(services, account, token, params)
}

/// The account and repository a command is about, signed in.
pub(crate) fn on_repository(
    ctx: &CommandContext,
    params: &Params,
) -> Result<(Signed, Target), CommandError> {
    let (signed, target) = on_account(ctx, params)?;
    let target = target.ok_or_else(|| {
        CommandError::Git(bitbraid_git::Error::Refused {
            what: say!("commands-what-provider"),
            detail: say!("commands-which-repository").to_owned(),
        })
    })?;
    Ok((signed, target))
}

/// The account a command is about, signed in, and the repository it is
/// about when one is named or the open repository is linked.
pub(crate) fn on_account(
    ctx: &CommandContext,
    params: &Params,
) -> Result<(Signed, Option<Target>), CommandError> {
    let services = services(ctx)?;
    let accounts = accounts::load(&services.config_dir).map_err(failed)?;
    let owner = text(params, "owner")?
        .map(|owner| owner_path("owner", &owner))
        .transpose()?;
    let name = text(params, "name")?
        .map(|name| {
            if remotes::path_part(&name) {
                Ok(name)
            } else {
                Err(invalid("name", say!("commands-not-a-name")))
            }
        })
        .transpose()?;
    let (account, link) = account_and_link(ctx, params, &accounts)?;
    let target = match (owner, name, link) {
        (Some(owner), Some(name), _) => Some(Target { owner, name }),
        (owner, name, Some(link)) => Some(Target {
            owner: owner.unwrap_or(link.owner),
            name: name.unwrap_or(link.name),
        }),
        _ => None,
    };
    if account.kind == ProviderKind::Github
        && target
            .as_ref()
            .is_some_and(|target| target.owner.contains('/'))
    {
        return Err(invalid("owner", say!("commands-one-name-at-github")));
    }
    let token = accounts::token(&services, &account, &ctx.cancel).map_err(failed)?;
    Ok((Signed::new(services, account, token, params)?, target))
}
