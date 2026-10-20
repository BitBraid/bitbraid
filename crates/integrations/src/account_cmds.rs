//! Account commands: list the connected accounts, sign in by a code typed
//! at the provider's site or by a token, sign out, and choose the default
//! account of a site.
//!
//! A token is a parameter of `account.add_token` and nothing else: it goes
//! to the provider to ask whose it is and then to the keychain, and is
//! never part of an answer or an error.
//!
//! Sources: docs.github.com "Authorizing OAuth apps" (device flow) and
//! "Managing your personal access tokens"; docs.gitlab.com/api "OAuth 2.0
//! identity provider API" (device authorization grant) and "Personal
//! access tokens"; RFC 8628.

use bitbraid_engine::{
    CommandContext, CommandError, CommandSpec, Output, ParamKind, ParamSpec, Params, Registry,
};
use serde_json::json;

use crate::{
    accounts::{self, TokenRecord},
    api::ProviderError,
    cmds::{self, FREE, KIND, WEB, failed, invalid, opt, req},
    model::{DevicePoll, SignIn},
    oauth::{self, Polled},
    secrets::Secret,
};

const ID: ParamSpec = req("id", ParamKind::Text, message!("account-param-id"));
const CLIENT_ID: ParamSpec = opt(
    "client_id",
    ParamKind::Text,
    message!("account-param-client-id"),
);

const SIGN_IN_START: &[ParamSpec] = &[KIND, WEB, CLIENT_ID];
const SIGN_IN_POLL: &[ParamSpec] = &[
    KIND,
    WEB,
    req(
        "device_code",
        ParamKind::Text,
        message!("account-param-sign-in-poll-device-code"),
    ),
    opt(
        "interval",
        ParamKind::Number,
        message!("account-param-sign-in-poll-interval"),
    ),
];
const ADD_TOKEN: &[ParamSpec] = &[
    KIND,
    WEB,
    req(
        "token",
        ParamKind::Text,
        message!("account-param-add-token"),
    ),
];
const BY_ID: &[ParamSpec] = &[ID];

/// Register the account commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register_described(
        CommandSpec {
            id: "account.list",
            label: say!("account-list"),
            description: say!("account-list-description"),
        },
        list,
        FREE,
        &[],
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "account.sign_in_start",
            label: say!("account-sign-in-start"),
            description: say!("account-sign-in-start-description"),
        },
        sign_in_start,
        FREE,
        SIGN_IN_START,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "account.sign_in_poll",
            label: say!("account-sign-in-poll"),
            description: say!("account-sign-in-poll-description"),
        },
        sign_in_poll,
        FREE,
        SIGN_IN_POLL,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "account.add_token",
            label: say!("account-add-token"),
            description: say!("account-add-token-description"),
        },
        add_token,
        FREE,
        ADD_TOKEN,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "account.remove",
            label: say!("account-remove"),
            description: say!("account-remove-description"),
        },
        remove,
        FREE,
        BY_ID,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "account.set_default",
            label: say!("account-set-default"),
            description: say!("account-set-default-description"),
        },
        set_default,
        FREE,
        BY_ID,
    );
}

fn list(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let services = cmds::services(ctx)?;
    cmds::answer(&accounts::list(&services).map_err(failed)?)
}

/// The site a command names: `kind` and `web`.
fn site(params: &Params) -> Result<accounts::Site, CommandError> {
    let kind = cmds::kind(params)?;
    let web = cmds::text_with(params, "web", 2048, false)?;
    accounts::site(kind, web.as_deref()).map_err(|err| invalid("web", err.to_string()))
}

/// The device flow's application id at a site, or why there is none.
fn client_id(
    services: &crate::services::Services,
    site: &accounts::Site,
) -> Result<String, CommandError> {
    let known = accounts::load(&services.config_dir).map_err(failed)?;
    oauth::client_id(services, site, &known).ok_or_else(|| {
        failed(ProviderError::Refused(say!(
            "account-code-sign-in-unavailable",
            site = site.authority.as_str()
        )))
    })
}

fn sign_in_start(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let site = site(params)?;
    let given = cmds::text_with(params, "client_id", 256, false)?;
    let services = cmds::services(ctx)?;
    if let Some(given) = given
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
    {
        if site.public() {
            return Err(invalid(
                "client_id",
                say!("account-client-id-own-server-only"),
            ));
        }
        if !oauth::usable_id(&given) {
            return Err(invalid("client_id", say!("account-client-id-not-an-id")));
        }
        accounts::set_client_id(&services, &site.web, &given).map_err(failed)?;
    }
    let client_id = client_id(&services, &site)?;
    let code = oauth::start(&services, &site, &client_id, &ctx.cancel).map_err(failed)?;
    cmds::answer(&code)
}

fn sign_in_poll(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let site = site(params)?;
    let device_code = cmds::required(params, "device_code")?;
    if device_code.len() > 512 {
        return Err(invalid("device_code", say!("account-device-code-too-long")));
    }
    let interval = cmds::number(params, "interval", 1, 3600)?.unwrap_or(5);
    let services = cmds::services(ctx)?;
    let client_id = client_id(&services, &site)?;
    let polled = oauth::poll(
        &services,
        &site,
        &client_id,
        &device_code,
        interval,
        &ctx.cancel,
    )
    .map_err(failed)?;
    let state = match polled {
        Polled::Waiting => DevicePoll::Waiting,
        Polled::SlowDown(interval) => DevicePoll::SlowDown { interval },
        Polled::Denied => DevicePoll::Denied,
        Polled::Expired => DevicePoll::Expired,
        Polled::Token(record, granted) => {
            let (account, _) = accounts::add(
                &services,
                &site,
                record,
                SignIn::Device,
                granted,
                &ctx.cancel,
            )
            .map_err(failed)?;
            DevicePoll::Done { account }
        }
    };
    cmds::answer(&state)
}

fn add_token(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let site = site(params)?;
    let token = cmds::text_with(params, "token", 4096, false)?
        .map(|token| token.trim().to_owned())
        .filter(|token| !token.is_empty())
        .ok_or_else(|| invalid("token", say!("commands-missing")))?;
    if !token.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return Err(invalid("token", say!("account-not-a-token")));
    }
    let services = cmds::services(ctx)?;
    let (account, _) = accounts::add(
        &services,
        &site,
        TokenRecord::token(&Secret::new(token)),
        SignIn::Token,
        None,
        &ctx.cancel,
    )
    .map_err(failed)?;
    cmds::answer(&account)
}

fn remove(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = cmds::required(params, "id")?;
    let services = cmds::services(ctx)?;
    accounts::remove(&services, &id).map_err(failed)?;
    Ok(json!({ "removed": id }))
}

fn set_default(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = cmds::required(params, "id")?;
    let services = cmds::services(ctx)?;
    accounts::set_default(&services, &id).map_err(failed)?;
    Ok(json!({ "default": id }))
}
