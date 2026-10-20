//! Provider commands about repositories and accounts' keys: which
//! repository at which account the open repository's remotes are, the
//! repositories an account may clone, making and forking one, uploading
//! a public key and the provider's pictures of people.
//!
//! Making a repository adds it as a remote of the open repository and
//! pushes the current branch to it when asked; forking adds the fork as a
//! remote named after its owner (one that is the fork already is taken as
//! it is); a key is read and checked here ([`crate::keys`]) before it is
//! sent, and a private one is refused. What touches the open repository
//! goes through the engine's own commands (`remote.add`, `remote.push`).
//!
//! Sources: git-remote(1); the providers' documents named in
//! [`crate::github`] and [`crate::gitlab`].

use bitbraid_engine::{
    CommandContext, CommandError, CommandSpec, Output, ParamKind, ParamSpec, Params, Registry,
};

use serde_json::json;

use crate::{
    accounts,
    cmds::{
        self, ACCOUNT, FREE, FRESH, NAME, OWNER, READS, WRITES, WRITES_IF_OPEN, failed, invalid,
        opt, req,
    },
    keys,
    model::{Forked, MadeRepo},
    provider::{NewRepo, RepoFilter, RepoQuery},
};

const REPOS: &[ParamSpec] = &[
    req(
        "account",
        ParamKind::Text,
        message!("provider-param-account"),
    ),
    opt(
        "filter",
        ParamKind::Choice(&["own", "organisation", "starred", "all"]),
        message!("provider-param-repos-filter"),
    ),
    opt(
        "query",
        ParamKind::Text,
        message!("provider-param-repos-query"),
    ),
    opt(
        "page",
        ParamKind::Number,
        message!("provider-param-repos-page"),
    ),
    FRESH,
];
const NAMESPACES: &[ParamSpec] = &[
    req(
        "account",
        ParamKind::Text,
        message!("provider-param-account"),
    ),
    FRESH,
];
const REPO_CREATE: &[ParamSpec] = &[
    req(
        "account",
        ParamKind::Text,
        message!("provider-param-account"),
    ),
    opt(
        "namespace",
        ParamKind::Text,
        message!("provider-param-repo-create-namespace"),
    ),
    req(
        "name",
        ParamKind::Text,
        message!("provider-param-repo-create-name"),
    ),
    opt(
        "description",
        ParamKind::Text,
        message!("provider-param-repo-create-description"),
    ),
    req(
        "private",
        ParamKind::Flag,
        message!("provider-param-repo-create-private"),
    ),
    opt(
        "remote",
        ParamKind::Text,
        message!("provider-param-repo-create-remote"),
    ),
    opt(
        "push",
        ParamKind::Flag,
        message!("provider-param-repo-create-push"),
    ),
    opt(
        "protocol",
        ParamKind::Choice(&["https", "ssh"]),
        message!("provider-param-repo-create-protocol"),
    ),
];
const FORK: &[ParamSpec] = &[
    opt(
        "remote",
        ParamKind::Text,
        message!("provider-param-fork-remote"),
    ),
    opt(
        "namespace",
        ParamKind::Text,
        message!("provider-param-fork-namespace"),
    ),
];
const SSH_KEY_ADD: &[ParamSpec] = &[
    req(
        "account",
        ParamKind::Text,
        message!("provider-param-account"),
    ),
    req(
        "path",
        ParamKind::Path,
        message!("provider-param-ssh-key-add-path"),
    ),
    req(
        "title",
        ParamKind::Text,
        message!("provider-param-ssh-key-add-title"),
    ),
];
const AVATARS: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    req(
        "emails",
        ParamKind::List,
        message!("provider-param-avatars-emails"),
    ),
    FRESH,
];

/// Most addresses asked about at once.
const MAX_EMAILS: usize = 100;

/// Register the provider commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register_described(
        CommandSpec {
            id: "provider.links",
            label: say!("provider-links"),
            description: say!("provider-links-description"),
        },
        links,
        READS,
        &[],
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "provider.repos",
            label: say!("provider-repos"),
            description: say!("provider-repos-description"),
        },
        repos,
        FREE,
        REPOS,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "provider.namespaces",
            label: say!("provider-namespaces"),
            description: say!("provider-namespaces-description"),
        },
        namespaces,
        FREE,
        NAMESPACES,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "provider.repo_create",
            label: say!("provider-repo-create"),
            description: say!("provider-repo-create-description"),
        },
        repo_create,
        WRITES_IF_OPEN,
        REPO_CREATE,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "provider.fork",
            label: say!("provider-fork"),
            description: say!("provider-fork-description"),
        },
        fork,
        WRITES,
        FORK,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "provider.ssh_key_add",
            label: say!("provider-ssh-key-add"),
            description: say!("provider-ssh-key-add-description"),
        },
        ssh_key_add,
        FREE,
        SSH_KEY_ADD,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "provider.avatars",
            label: say!("provider-avatars"),
            description: say!("provider-avatars-description"),
        },
        avatars,
        cmds::READS_IF_OPEN,
        AVATARS,
    );
}

fn links(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    ctx.repo()?;
    let services = cmds::services(ctx)?;
    let known = accounts::load(&services.config_dir).map_err(failed)?;
    cmds::answer(&cmds::links(ctx, &known)?)
}

fn repos(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let filter = match cmds::choice(
        params,
        "filter",
        &["own", "organisation", "starred", "all"],
        "all",
    )? {
        "own" => RepoFilter::Own,
        "organisation" => RepoFilter::Organisation,
        "starred" => RepoFilter::Starred,
        _ => RepoFilter::All,
    };
    let query = RepoQuery {
        filter,
        query: cmds::text_with(params, "query", 256, false)?,
        page: u32::try_from(cmds::number(params, "page", 1, 10_000)?.unwrap_or(1)).unwrap_or(1),
    };
    cmds::required(params, "account")?;
    cmds::flag(params, "fresh")?;
    let signed = cmds::signed(ctx, params)?;
    let listing = signed
        .provider()
        .repos(&signed.session(ctx), &query)
        .map_err(failed)?;
    cmds::answer(&listing)
}

fn namespaces(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    cmds::required(params, "account")?;
    cmds::flag(params, "fresh")?;
    let signed = cmds::signed(ctx, params)?;
    let found = signed
        .provider()
        .namespaces(&signed.session(ctx))
        .map_err(failed)?;
    cmds::answer(&found)
}

/// A name for a new repository or a remote: what a provider allows.
fn repository_name(params: &Params, name: &str) -> Result<Option<String>, CommandError> {
    let Some(text) = cmds::text_with(params, name, 255, false)? else {
        return Ok(None);
    };
    if !crate::remotes::path_part(&text) || text.ends_with(".git") {
        return Err(invalid(name, say!("provider-not-a-name")));
    }
    Ok(Some(text))
}

fn repo_create(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    cmds::required(params, "account")?;
    let repo = NewRepo {
        namespace: cmds::text_with(params, "namespace", 1024, false)?,
        name: repository_name(params, "name")?
            .ok_or_else(|| invalid("name", say!("provider-missing")))?,
        description: cmds::text_with(params, "description", 2048, false)?,
        private: cmds::flag(params, "private")?
            .ok_or_else(|| invalid("private", say!("provider-missing")))?,
    };
    let remote = repository_name(params, "remote")?;
    let push = cmds::flag(params, "push")?.unwrap_or(false);
    let ssh = cmds::choice(params, "protocol", &["https", "ssh"], "https")? == "ssh";
    if push && remote.is_none() {
        return Err(invalid("push", say!("provider-push-without-remote")));
    }
    if remote.is_some() && ctx.repo.is_none() {
        return Err(CommandError::NoRepository);
    }
    let (made, site) = {
        let signed = cmds::signed(ctx, params)?;
        let made = signed
            .provider()
            .repo_create(&signed.session(ctx), &repo)
            .map_err(failed)?;
        (made, crate::pr_checkout::Site::of(&signed)?)
    };
    let mut out = MadeRepo {
        repo: made,
        remote: None,
        pushed: None,
    };
    if let Some(remote) = remote {
        let address = if ssh {
            &out.repo.clone_ssh
        } else {
            &out.repo.clone_https
        };
        if !site.holds(address) {
            return Err(failed(crate::api::ProviderError::Refused(say!(
                "provider-made-elsewhere",
                repository = out.repo.full_name.as_str()
            ))));
        }
        let then = |what: &str, err: CommandError| {
            CommandError::Git(bitbraid_git::Error::Refused {
                what: say!("provider-refused"),
                detail: say!(
                    "provider-made-but",
                    repository = out.repo.full_name.as_str(),
                    what,
                    error = err.to_string()
                ),
            })
        };
        crate::shared()
            .run(
                "remote.add",
                ctx,
                &json!({ "name": remote, "url": address }),
            )
            .map_err(|err| then(say!("provider-remote-not-added"), err))?;
        out.remote = Some(remote.clone());
        if push {
            let branch = bitbraid_git::branch::head_state(ctx.repo()?)?
                .branch
                .ok_or_else(|| {
                    then(
                        say!("provider-nothing-pushed"),
                        invalid("push", say!("provider-no-branch")),
                    )
                })?;
            crate::shared()
                .run(
                    "remote.push",
                    ctx,
                    &json!({ "remote": remote, "branch": branch, "set_upstream": true }),
                )
                .map_err(|err| then(say!("provider-push-failed"), err))?;
            out.pushed = Some(branch);
        }
    }
    cmds::answer(&out)
}

fn fork(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    ctx.repo()?;
    let remote = cmds::text_with(params, "remote", 255, false)?;
    let namespace = cmds::text_with(params, "namespace", 1024, false)?;
    let services = cmds::services(ctx)?;
    let known = accounts::load(&services.config_dir).map_err(failed)?;
    let all = cmds::links(ctx, &known)?;
    let link = match &remote {
        Some(remote) => all.iter().find(|link| &link.remote == remote),
        None => all.first(),
    }
    .ok_or_else(|| invalid("remote", say!("provider-not-linked")))?;
    let by_link = serde_json::json!({
        "account": link.account,
        "owner": link.owner,
        "name": link.name,
    });
    let base_address = bitbraid_git::snapshot(ctx.repo()?)?
        .remotes
        .into_iter()
        .find(|known| known.name == link.remote)
        .and_then(|known| known.fetch_url)
        .unwrap_or_default();
    let (made, site) = {
        let (signed, target) = cmds::on_repository(ctx, &by_link)?;
        let made = signed
            .provider()
            .fork(&signed.session(ctx), &target, namespace.as_deref())
            .map_err(failed)?;
        (made, crate::pr_checkout::Site::of(&signed)?)
    };
    let (name, _) =
        crate::pr_checkout::fork_remote(ctx, &made, &base_address, &site)?.ok_or_else(|| {
            failed(crate::api::ProviderError::Refused(say!(
                "provider-made-elsewhere",
                repository = made.full_name.as_str()
            )))
        })?;
    cmds::answer(&Forked {
        repo: made,
        remote: Some(name),
    })
}

fn ssh_key_add(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    cmds::required(params, "account")?;
    let path = cmds::required(params, "path")?;
    let title = cmds::text_with(params, "title", 255, false)?
        .filter(|title| !title.trim().is_empty())
        .ok_or_else(|| invalid("title", say!("provider-missing")))?;
    let path = std::path::PathBuf::from(path);
    if !path.is_absolute() || path.extension().and_then(|ext| ext.to_str()) != Some("pub") {
        return Err(invalid("path", say!("provider-not-a-key-path")));
    }
    let metadata = std::fs::metadata(&path).map_err(|err| invalid("path", err.to_string()))?;
    if !metadata.is_file() || metadata.len() > u64::try_from(keys::MAX_LINE).unwrap_or(u64::MAX) + 2
    {
        return Err(invalid("path", say!("provider-not-a-key-file")));
    }
    let text = std::fs::read_to_string(&path).map_err(|err| invalid("path", err.to_string()))?;
    let key = keys::read(&text).map_err(|problem| invalid("path", problem))?;
    let signed = cmds::signed(ctx, params)?;
    let added = signed
        .provider()
        .ssh_key_add(&signed.session(ctx), &title, &key.line)
        .map_err(failed)?;
    cmds::answer(&added)
}

fn avatars(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let emails = cmds::list(params, "emails", MAX_EMAILS)?
        .ok_or_else(|| invalid("emails", say!("provider-missing")))?;
    if let Some(odd) = emails
        .iter()
        .find(|email| !email.contains('@') || email.len() > 254)
    {
        return Err(invalid(
            "emails",
            say!("provider-not-an-address", value = format!("{odd:?}")),
        ));
    }
    cmds::flag(params, "fresh")?;
    let (signed, target) = cmds::on_account(ctx, params)?;
    let found = signed
        .provider()
        .avatars(&signed.session(ctx), target.as_ref(), &emails)
        .map_err(failed)?;
    cmds::answer(&found)
}
