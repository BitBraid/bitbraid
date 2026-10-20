//! Gitflow commands: how it stands, set it up, start and finish a
//! feature, bugfix, release or hotfix, and start a support branch.
//!
//! A finish is a sequence of ordinary steps. Each is journalled as what it
//! is (a checkout, a move of HEAD, a tag, a deleted branch) under a label
//! that names the finish, so undoing them one after another takes a
//! finish back. A merge that meets a conflict stops the finish there, the
//! way `branch.merge` stops; once it is resolved and committed, the same
//! finish goes on with what is left.
//!
//! Sources: git-switch(1), git-merge(1), git-tag(1), git-branch(1).

use bitbraid_git::{
    Dirty, ResetMode, branch,
    flow::{self, FinishPlan, InitOptions, Kind, Need, Step},
};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    journal::Change,
    rewrite_cmds,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "flow.status",
            label: say!("flow-status"),
            description: say!("flow-status-description"),
        },
        status,
    );
    let _ = registry.register(
        CommandSpec {
            id: "flow.init",
            label: say!("flow-init"),
            description: say!("flow-init-description"),
        },
        init,
    );
    let _ = registry.register(
        CommandSpec {
            id: "flow.start",
            label: say!("flow-start"),
            description: say!("flow-start-description"),
        },
        start,
    );
    let _ = registry.register(
        CommandSpec {
            id: "flow.finish",
            label: say!("flow-finish"),
            description: say!("flow-finish-description"),
        },
        finish,
    );
}

fn status(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(flow::status(ctx.repo()?)?)?)
}

fn init(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let options = InitOptions {
        main: param_or(params, "main", None)?,
        develop: param_or(params, "develop", None)?,
        feature: param_or(params, "feature", None)?,
        bugfix: param_or(params, "bugfix", None)?,
        release: param_or(params, "release", None)?,
        hotfix: param_or(params, "hotfix", None)?,
        support: param_or(params, "support", None)?,
        tag: param_or(params, "tag", None)?,
    };
    // The names it writes are settings of the repository's, and the
    // develop branch it makes goes with them: one entry for both, so that
    // undo leaves the repository as it was before gitflow was set up.
    let (done, settings) = crate::settings::watching(ctx, |repo| Ok(flow::init(repo, &options)?))?;
    let created = done.created.and(done.status.develop.clone());
    let branch = done
        .created
        .and_then(|target| Some((created.clone()?, target)));
    if branch.is_some() || !settings.is_empty() {
        let label = match &created {
            Some(develop) => say!("flow-entry-init-develop", develop = develop.as_str()),
            None => say!("flow-entry-init").to_owned(),
        };
        ctx.record(label, Change::Config { settings, branch })?;
    }
    let mut out = serde_json::to_value(&done.status)?;
    if let Some(fields) = out.as_object_mut() {
        fields.insert("created".to_owned(), json!(created));
    }
    Ok(out)
}

fn start(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let kind: Kind = param(params, "kind")?;
    let name: String = param(params, "name")?;
    let base: Option<String> = param_or(params, "base", None)?;
    let started = flow::start(ctx.repo()?, kind, &name, base.as_deref())?;
    ctx.record(
        say!("flow-entry-start", kind = kind.noun(), name = name.as_str()),
        Change::BranchCreate {
            name: started.branch.clone(),
            target: started.target,
            switched_from: Some(started.before),
        },
    )?;
    Ok(json!({
        "kind": kind,
        "name": name,
        "branch": started.branch,
        "base": started.base,
        "target": started.target,
    }))
}

/// Do one step of a finish and journal it. A merge that stopped on a
/// conflict is not an error: its answer is returned, and the finish ends
/// there for now.
fn run_step(
    ctx: &mut CommandContext,
    plan: &FinishPlan,
    step: Step,
    what: &str,
) -> Result<Option<Output>, CommandError> {
    let onto = plan.onto(step);
    match step {
        Step::CheckoutMain | Step::CheckoutDevelop => {
            let outcome = branch::checkout(ctx.repo()?, onto, Dirty::Refuse)?;
            if outcome.before != outcome.after {
                ctx.record(
                    say!("flow-entry-finish-checkout", what, onto),
                    Change::Checkout {
                        target: onto.to_owned(),
                        dirty: Dirty::Refuse,
                        before: outcome.before,
                        after: outcome.after,
                        created: outcome.created,
                        backups: outcome.backups,
                    },
                )?;
            }
            Ok(None)
        }
        Step::MergeMain | Step::MergeDevelop => {
            let moved = flow::finish_merge(ctx.repo()?, plan, step)?;
            let stopped = moved.stopped.is_some();
            let answer = rewrite_cmds::finish(
                ctx,
                say!("flow-entry-finish-merge", what, onto),
                moved,
                ResetMode::Keep,
            )?;
            Ok(stopped.then_some(answer))
        }
        Step::Tag => {
            let (name, value) = flow::finish_tag(ctx.repo()?, plan)?;
            ctx.record(
                say!("flow-entry-finish-tag", what, name = name.as_str()),
                Change::TagCreate { name, value },
            )?;
            Ok(None)
        }
        Step::Delete => {
            let deleted = flow::finish_delete(ctx.repo()?, plan)?;
            ctx.record(
                say!(
                    "flow-entry-finish-delete",
                    what,
                    name = deleted.name.as_str()
                ),
                Change::BranchDelete {
                    name: deleted.name,
                    target: deleted.target,
                    upstream: deleted.upstream,
                },
            )?;
            Ok(None)
        }
    }
}

/// A step failed after others were done: say which one, since what was
/// done before it stays done and is in the journal.
fn failed_at(step: Step, err: CommandError) -> CommandError {
    let context = match step {
        Step::CheckoutMain => say!("flow-failed-checkout-main"),
        Step::MergeMain => say!("flow-failed-merge-main"),
        Step::Tag => say!("flow-failed-tag"),
        Step::CheckoutDevelop => say!("flow-failed-checkout-develop"),
        Step::MergeDevelop => say!("flow-failed-merge-develop"),
        Step::Delete => say!("flow-failed-delete"),
    };
    CommandError::Git(bitbraid_git::Error::Git {
        context,
        source: Box::new(err),
    })
}

fn finish(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let kind: Kind = param(params, "kind")?;
    let name: String = param(params, "name")?;
    let message: Option<String> = param_or(params, "message", None)?;
    let keep: bool = param_or(params, "keep", false)?;
    // Everything that refuses does so here, before the first step.
    let plan = flow::finish_plan(ctx.repo()?, kind, &name, message.as_deref(), keep)?;
    let what = say!(
        "flow-entry-finish",
        kind = kind.noun(),
        name = name.as_str()
    );

    let mut steps = Vec::new();
    let mut changed = false;
    let mut stop: Option<(Step, Output)> = None;
    for planned in &plan.steps {
        let state = match planned.need {
            Need::Already => "already",
            Need::Skip => "skipped",
            Need::Do if stop.is_some() => "pending",
            Need::Do => match run_step(ctx, &plan, planned.step, &what) {
                Ok(None) => {
                    changed = true;
                    "done"
                }
                Ok(Some(answer)) => {
                    stop = Some((planned.step, answer));
                    "stopped"
                }
                Err(err) if changed => return Err(failed_at(planned.step, err)),
                Err(err) => return Err(err),
            },
        };
        steps.push(json!({ "step": planned.step, "state": state }));
    }

    let mut out = json!({
        "kind": kind,
        "name": name,
        "branch": plan.branch,
        "main": plan.main,
        "develop": plan.develop,
        "tag": plan.tag,
        "steps": steps,
        "finished": stop.is_none(),
        "changed": changed || stop.is_some(),
        "stopped": null,
        "stopped_at": null,
        "conflicts": [],
    });
    // Stopped: answer as the merge itself does, and say which step it was.
    if let (Some((step, answer)), Some(fields)) = (stop, out.as_object_mut()) {
        fields.insert("stopped_at".to_owned(), json!(step));
        for key in ["stopped", "conflicts", "before", "after"] {
            if let Some(value) = answer.get(key) {
                fields.insert(key.to_owned(), value.clone());
            }
        }
    }
    Ok(out)
}
