//! Making repositories: init (with the ignore templates it can start
//! with) and clone. Neither needs an open repository, and neither opens
//! the one it made: that is the caller's next step.
//!
//! Sources: git-init(1), git-clone(1).

use std::path::PathBuf;

use bitbraid_git::{CloneOptions, InitOptions, Reporter, create};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "repo.init",
            label: say!("create-repo-init"),
            description: say!("create-repo-init-description"),
        },
        init,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.ignore_templates",
            label: say!("create-repo-ignore-templates"),
            description: say!("create-repo-ignore-templates-description"),
        },
        ignore_templates,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.clone",
            label: say!("create-repo-clone"),
            description: say!("create-repo-clone-description"),
        },
        clone,
    );
}

fn init(_ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: PathBuf = param(params, "path")?;
    let options = InitOptions {
        branch: param_or(params, "branch", None)?,
        first_commit: param_or(params, "first_commit", false)?,
        gitignore: param_or(params, "gitignore", Vec::new())?,
    };
    let created = create::init(&path, &options)?;
    // As text: `json!` cannot write a path that is not UTF-8.
    Ok(json!({ "path": created.display().to_string() }))
}

fn ignore_templates(_ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let names: Vec<String> = param_or(params, "names", Vec::new())?;
    let text = bitbraid_git::ignore_templates::compose(&names)?;
    Ok(json!({
        "templates": bitbraid_git::ignore_templates::TEMPLATES,
        "text": text,
    }))
}

fn clone(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let options = CloneOptions {
        url: param(params, "url")?,
        path: param(params, "path")?,
        branch: param_or(params, "branch", None)?,
        depth: param_or(params, "depth", None)?,
        recurse_submodules: param_or(params, "recurse_submodules", false)?,
        allow_local: param_or(params, "allow_local", false)?,
        filter: param_or(params, "filter", None)?,
        sparse: param_or(params, "sparse", false)?,
    };
    let sink = ctx.progress.clone();
    let mut on_progress = |progress| {
        if let Some(sink) = &sink {
            sink(progress);
        }
    };
    let mut reporter = Reporter {
        cancel: ctx.cancel.clone(),
        on_progress: &mut on_progress,
    };
    let cloned = create::clone(&options, &mut reporter)?;
    let mut out = json!({
        "path": cloned.path.display().to_string(),
        "url": options.url,
        "commit_graph": cloned.commit_graph,
    });
    if let (Some(filter), Some(object)) = (options.filter, out.as_object_mut()) {
        // A remote that does not filter sends everything, and git only
        // warns: whether a partial clone was made is said.
        object.insert("filter".to_owned(), json!(filter.spec()));
        object.insert("partial".to_owned(), json!(cloned.partial));
    }
    Ok(out)
}
