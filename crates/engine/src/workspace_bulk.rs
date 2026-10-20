//! Commands on many repositories of a workspace at once: fetch,
//! pull, and clone the missing ones.
//!
//! Each repository's command runs through the registry in a context of
//! its own, as if it were the only one: `remote.fetch`, `remote.pull` and
//! `repo.clone`, with the undo journal, the hooks and the activity log of
//! that repository. A few run at once (eight unless the caller says), on
//! threads of their own; one that fails never stops the others. Setting
//! the context's stop flag keeps those that have not started from
//! starting, and the running ones see it as they would alone.
//!
//! What each is doing goes to the context's progress sink as it happens,
//! one report per change, each naming its repository
//! ([`RepositoryOutcome::to_progress`]); the answer is a [`BulkOutcome`]
//! with what became of every one.
//!
//! Sources: git-fetch(1), git-pull(1), git-clone(1).

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};

use serde_json::{Value, json};

use crate::{
    Registry,
    command::{CommandContext, CommandError, Output, Params, ProgressSink, param_or},
    workspace::{
        self, BulkOutcome, RepositoryOutcome, RepositoryState, Workspace, WorkspaceRepository,
        refused,
    },
    workspace_cmds::{directory, id_param, position},
};

/// As many repositories as are worked on at once unless the caller says.
pub const DEFAULT_PARALLEL: usize = 8;
/// The most that are worked on at once.
pub const MAX_PARALLEL: usize = 32;

/// The commands each repository is given: the engine's own, made once.
pub(crate) fn registry() -> &'static Registry {
    static SHARED: OnceLock<Registry> = OnceLock::new();
    SHARED.get_or_init(crate::builtin_registry)
}

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

/// What one repository is given to do.
#[derive(Debug, Clone)]
pub(crate) struct Job {
    /// The repository, by the path the workspace has for it.
    pub path: String,
    /// The folder of the repository to open first; none for a command
    /// that needs none (a clone).
    pub open: Option<String>,
    /// The command, and what it is given.
    pub command: &'static str,
    pub params: Value,
}

/// One repository's turn: its command, or why it is not given one.
#[derive(Debug, Clone)]
pub(crate) enum Turn {
    Run(Job),
    Skip { path: String, why: String },
}

/// How many at once, from the parameters.
fn parallel(params: &Params) -> Result<usize, CommandError> {
    let parallel: usize = param_or(params, "parallel", DEFAULT_PARALLEL)?;
    if parallel == 0 || parallel > MAX_PARALLEL {
        return Err(invalid(
            "parallel",
            say!("workspace-from-one-to", most = MAX_PARALLEL),
        ));
    }
    Ok(parallel)
}

/// The workspace and the repositories of it that are asked for: those
/// `paths` names, in the workspace's order, or all of them.
fn chosen(
    ctx: &CommandContext,
    params: &Params,
) -> Result<(Workspace, Vec<WorkspaceRepository>), CommandError> {
    let id = id_param(params)?;
    let asked: Option<Vec<String>> = param_or(params, "paths", None)?;
    let found = workspace::find(directory(ctx)?, &id)?;
    let Some(asked) = asked else {
        let all = found.repositories.clone();
        return Ok((found, all));
    };
    if asked.len() > workspace::MAX_REPOSITORIES {
        return Err(invalid(
            "paths",
            say!(
                "workspace-at-most-at-a-time",
                most = workspace::MAX_REPOSITORIES
            ),
        ));
    }
    let mut wanted = Vec::with_capacity(asked.len());
    for text in &asked {
        let path = workspace::full_path(text)
            .map_err(|why| invalid("paths", format!("{:?}: {why}", workspace::cut(text))))?;
        if !found.repositories.iter().any(|entry| entry.path == path) {
            return Err(invalid(
                "paths",
                say!(
                    "workspace-not-one-of-its-repositories",
                    path = path.as_str()
                ),
            ));
        }
        wanted.push(path);
    }
    let picked = found
        .repositories
        .iter()
        .filter(|entry| wanted.contains(&entry.path))
        .cloned()
        .collect();
    Ok((found, picked))
}

/// Whether the folder of an entry is a repository to run a command in,
/// or why not.
fn openable(entry: &WorkspaceRepository) -> Result<String, String> {
    let path = Path::new(&entry.path);
    if !path.exists() {
        return Err(say!("workspace-bulk-not-there").to_owned());
    }
    if !bitbraid_git::glance::is_repository_folder(path) {
        return Err(say!("workspace-bulk-not-a-repository").to_owned());
    }
    Ok(entry.path.clone())
}

/// What every repository's context takes over from the one of the
/// command: what threads can share of it.
#[derive(Debug, Clone)]
struct Shared {
    cancel: Arc<std::sync::atomic::AtomicBool>,
    config_dir: Option<PathBuf>,
    extensions: crate::command::Extensions,
}

/// Run every turn, `parallel` at once, and say what became of each.
pub(crate) fn run_all(
    ctx: &CommandContext,
    workspace_id: &str,
    command: &str,
    turns: Vec<Turn>,
    parallel: usize,
) -> BulkOutcome {
    let started = Instant::now();
    let sink = ctx.progress.clone();
    let shared = Shared {
        cancel: ctx.cancel.clone(),
        config_dir: ctx.config_dir.clone(),
        extensions: ctx.extensions.clone(),
    };
    let report = |outcome: &RepositoryOutcome| {
        if let Some(sink) = &sink {
            sink(outcome.to_progress());
        }
    };
    let mut outcomes: Vec<Option<RepositoryOutcome>> = vec![None; turns.len()];
    let paths: Vec<String> = turns
        .iter()
        .map(|turn| match turn {
            Turn::Run(job) => job.path.clone(),
            Turn::Skip { path, .. } => path.clone(),
        })
        .collect();
    let mut jobs: Vec<(usize, Job)> = Vec::new();
    for (index, turn) in turns.into_iter().enumerate() {
        match turn {
            Turn::Run(job) => {
                report(&RepositoryOutcome::queued(&job.path));
                jobs.push((index, job));
            }
            Turn::Skip { path, why } => {
                let skipped = RepositoryOutcome {
                    state: RepositoryState::Skipped,
                    error: Some(why),
                    ..RepositoryOutcome::queued(&path)
                };
                report(&skipped);
                if let Some(slot) = outcomes.get_mut(index) {
                    *slot = Some(skipped);
                }
            }
        }
    }
    let at_once = parallel.clamp(1, MAX_PARALLEL).min(jobs.len().max(1));
    let next = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, RepositoryOutcome)>> = Mutex::new(Vec::new());
    let work = || {
        loop {
            let taken = next.fetch_add(1, Ordering::SeqCst);
            let Some((index, job)) = jobs.get(taken) else {
                break;
            };
            let outcome = run_one(&shared, job, sink.as_ref());
            results
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((*index, outcome));
        }
    };
    std::thread::scope(|scope| {
        let mut handles = Vec::new();
        // This thread is one of them, so that the work is done even when
        // no thread can be had.
        for number in 1..at_once {
            let spawned = std::thread::Builder::new()
                .name(format!("bitbraid-workspace-{number}"))
                .spawn_scoped(scope, work);
            match spawned {
                Ok(handle) => handles.push(handle),
                Err(_) => break,
            }
        }
        work();
        for handle in handles {
            // A thread that ended in a panic left its repository without
            // an outcome, which is said below.
            let _ = handle.join();
        }
    });
    for (index, outcome) in results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
    {
        if let Some(slot) = outcomes.get_mut(index) {
            *slot = Some(outcome);
        }
    }
    let stopped = shared.cancel.load(Ordering::SeqCst);
    let repositories: Vec<RepositoryOutcome> = outcomes
        .into_iter()
        .zip(paths)
        .map(|(outcome, path)| {
            outcome.unwrap_or_else(|| RepositoryOutcome {
                state: RepositoryState::Failed,
                error: Some(say!("workspace-bulk-no-answer").to_owned()),
                ..RepositoryOutcome::queued(&path)
            })
        })
        .collect();
    let count = |state| {
        repositories
            .iter()
            .filter(|outcome| outcome.state == state)
            .count()
    };
    BulkOutcome {
        workspace: workspace_id.to_owned(),
        command: command.to_owned(),
        done: count(RepositoryState::Done),
        failed: count(RepositoryState::Failed),
        skipped: count(RepositoryState::Skipped),
        repositories,
        stopped,
        parallel: at_once,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

/// Run one repository's command in a context of its own.
fn run_one(shared: &Shared, job: &Job, sink: Option<&ProgressSink>) -> RepositoryOutcome {
    let report = |outcome: &RepositoryOutcome| {
        if let Some(sink) = sink {
            sink(outcome.to_progress());
        }
    };
    let mut outcome = RepositoryOutcome::queued(&job.path);
    if shared.cancel.load(Ordering::SeqCst) {
        outcome.state = RepositoryState::Skipped;
        outcome.error = Some(say!("workspace-bulk-stopped").to_owned());
        report(&outcome);
        return outcome;
    }
    outcome.state = RepositoryState::Running;
    report(&outcome);
    // What git says it is doing, named, and the last of it kept for the
    // outcome.
    let last: Arc<Mutex<Option<bitbraid_git::Progress>>> = Arc::new(Mutex::new(None));
    let inner_sink: ProgressSink = {
        let (path, outer, last) = (job.path.clone(), sink.cloned(), last.clone());
        Arc::new(move |progress: bitbraid_git::Progress| {
            let line = RepositoryOutcome {
                state: RepositoryState::Running,
                progress: Some(progress.stage.clone()),
                percent: progress.percent,
                ..RepositoryOutcome::queued(&path)
            };
            if let Some(outer) = &outer {
                outer(line.to_progress());
            }
            if let Ok(mut kept) = last.lock() {
                *kept = Some(progress);
            }
        })
    };
    let mut inner = CommandContext {
        cancel: shared.cancel.clone(),
        config_dir: shared.config_dir.clone(),
        extensions: shared.extensions.clone(),
        progress: Some(inner_sink),
        ..CommandContext::default()
    };
    let registry = registry();
    let ran = match &job.open {
        Some(folder) => registry
            .run("repo.open", &mut inner, &json!({ "path": folder }))
            .and_then(|_| registry.run(job.command, &mut inner, &job.params)),
        None => registry.run(job.command, &mut inner, &job.params),
    };
    if let Ok(kept) = last.lock()
        && let Some(progress) = kept.as_ref()
    {
        outcome.progress = Some(progress.stage.clone());
        outcome.percent = progress.percent;
    }
    match ran {
        // A pull whose merge or rebase stopped on a conflict is in
        // progress in that repository: not done.
        Ok(output)
            if output
                .get("stopped")
                .is_some_and(|stopped| !stopped.is_null()) =>
        {
            let said = output
                .get("stopped")
                .and_then(Value::as_str)
                .unwrap_or(say!("workspace-bulk-stopped-half-way"))
                .to_owned();
            outcome.state = RepositoryState::Failed;
            outcome.error = Some(say!("workspace-bulk-conflicts", said));
            outcome.output = Some(output);
        }
        Ok(output) => {
            outcome.state = RepositoryState::Done;
            outcome.output = Some(output);
        }
        Err(err) => {
            outcome.state = RepositoryState::Failed;
            outcome.error = Some(err.to_string());
        }
    }
    report(&outcome);
    outcome
}

/// The answer of a command on many repositories.
fn answered(outcome: &BulkOutcome) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(outcome)?)
}

pub(crate) fn fetch(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let parallel = parallel(params)?;
    let prune: bool = param_or(params, "prune", false)?;
    let tags: bool = param_or(params, "tags", false)?;
    let quiet: bool = param_or(params, "quiet", false)?;
    let (found, chosen) = chosen(ctx, params)?;
    let turns = chosen
        .iter()
        .map(|entry| match openable(entry) {
            Ok(folder) => Turn::Run(Job {
                path: entry.path.clone(),
                open: Some(folder),
                command: "remote.fetch",
                params: json!({ "prune": prune, "tags": tags, "quiet": quiet }),
            }),
            Err(why) => Turn::Skip {
                path: entry.path.clone(),
                why,
            },
        })
        .collect();
    answered(&run_all(ctx, &found.id, "remote.fetch", turns, parallel))
}

pub(crate) fn pull(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let parallel = parallel(params)?;
    let mode: bitbraid_git::PullMode = param_or(params, "mode", Default::default())?;
    let (found, chosen) = chosen(ctx, params)?;
    let turns = chosen
        .iter()
        .map(|entry| match openable(entry) {
            Ok(folder) => Turn::Run(Job {
                path: entry.path.clone(),
                open: Some(folder),
                command: "remote.pull",
                params: json!({ "mode": mode }),
            }),
            Err(why) => Turn::Skip {
                path: entry.path.clone(),
                why,
            },
        })
        .collect();
    answered(&run_all(ctx, &found.id, "remote.pull", turns, parallel))
}

pub(crate) fn clone_missing(
    ctx: &mut CommandContext,
    params: &Params,
) -> Result<Output, CommandError> {
    let parallel = parallel(params)?;
    let into: Option<String> = param_or(params, "folder", None)?;
    let into = into
        .map(|text| workspace::full_path(&text).map_err(|why| invalid("folder", why)))
        .transpose()?;
    if let Some(folder) = &into
        && !Path::new(folder).is_dir()
    {
        return Err(refused(say!(
            "workspace-folder-missing",
            folder = folder.as_str()
        )));
    }
    let named = params.get("paths").is_some_and(|paths| !paths.is_null());
    let (found, chosen) = chosen(ctx, params)?;
    let mut turns = Vec::new();
    // Where each clone goes, for the entries that move.
    let mut moved: Vec<(String, String)> = Vec::new();
    for entry in &chosen {
        if Path::new(&entry.path).exists() {
            // Only what was asked for by name is said to be there.
            if named {
                turns.push(Turn::Skip {
                    path: entry.path.clone(),
                    why: say!("workspace-bulk-there-already").to_owned(),
                });
            }
            continue;
        }
        let Some(url) = &entry.remote_url else {
            turns.push(Turn::Skip {
                path: entry.path.clone(),
                why: say!("workspace-bulk-no-address").to_owned(),
            });
            continue;
        };
        let destination = match &into {
            Some(folder) => {
                let destination = PathBuf::from(folder).join(entry.name());
                let Some(text) = destination.to_str().map(str::to_owned) else {
                    turns.push(Turn::Skip {
                        path: entry.path.clone(),
                        why: say!("workspace-bulk-name-not-a-folder").to_owned(),
                    });
                    continue;
                };
                if text != entry.path {
                    moved.push((entry.path.clone(), text.clone()));
                }
                text
            }
            None => entry.path.clone(),
        };
        turns.push(Turn::Run(Job {
            path: entry.path.clone(),
            open: None,
            command: "repo.clone",
            params: json!({ "url": url, "path": destination }),
        }));
    }
    let outcome = run_all(ctx, &found.id, "repo.clone", turns, parallel);
    // The entries of those cloned elsewhere point at where they are now.
    let cloned: Vec<(String, String)> = moved
        .into_iter()
        .filter(|(from, _)| {
            outcome
                .repositories
                .iter()
                .any(|done| done.path == *from && done.state == RepositoryState::Done)
        })
        .collect();
    if !cloned.is_empty() {
        let dir = directory(ctx)?;
        workspace::changing(dir, |workspaces| {
            let at = position(workspaces, &found.id)?;
            if let Some(kept) = workspaces.get_mut(at) {
                for (from, to) in &cloned {
                    point_at(kept, from, to);
                }
            }
            Ok(())
        })?;
    }
    let mut out = answered(&outcome)?;
    if let Some(object) = out.as_object_mut() {
        let moved: Vec<Value> = cloned
            .iter()
            .map(|(from, to)| json!({ "from": from, "to": to }))
            .collect();
        object.insert("moved".to_owned(), json!(moved));
    }
    Ok(out)
}

/// Point the entry at `from` to `to`; when the workspace has an entry at
/// `to` already, that one stays and the other goes.
pub(crate) fn point_at(workspace: &mut Workspace, from: &str, to: &str) {
    if workspace.repositories.iter().any(|entry| entry.path == to) {
        let url = workspace
            .repositories
            .iter()
            .find(|entry| entry.path == from)
            .and_then(|entry| entry.remote_url.clone());
        workspace.repositories.retain(|entry| entry.path != from);
        if let Some(kept) = workspace
            .repositories
            .iter_mut()
            .find(|entry| entry.path == to)
            && kept.remote_url.is_none()
        {
            kept.remote_url = url;
        }
        return;
    }
    if let Some(entry) = workspace
        .repositories
        .iter_mut()
        .find(|entry| entry.path == from)
    {
        entry.path = to.to_owned();
    }
}
