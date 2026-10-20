//! `bitbraid-cli`: the command-line surface over the engine. Every
//! subcommand but `commands` is sugar over `run <command-id>`; output is
//! JSON unless a flag asks for text.
#![allow(clippy::print_stdout, clippy::print_stderr)]

use std::process::ExitCode;

use anyhow::{Context as _, Result};
use bitbraid_engine::{CommandContext, Registry};
use clap::{Parser, Subcommand};
use serde_json::{Value, json};

mod catalog;
mod running;

use running::Opening;

#[derive(Debug, Parser)]
#[command(
    name = "bitbraid-cli",
    version,
    about = "BitBraid command-line surface"
)]
struct Cli {
    /// Repository path (any directory inside it). Defaults to the current
    /// directory. A command that needs no repository opens none.
    #[arg(long, global = true)]
    repo: Option<String>,
    /// Compact JSON instead of pretty-printed.
    #[arg(long, global = true)]
    compact: bool,
    /// Do not report progress on standard error while a command runs.
    #[arg(long, global = true)]
    no_progress: bool,
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Debug, Subcommand)]
enum Cmd {
    /// List the registered engine commands.
    Commands {
        /// Emit a JSON array instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Run any command by id with JSON parameters.
    Run {
        /// Command id, for example `repo.log`.
        id: String,
        /// Parameters as a JSON object.
        #[arg(long, default_value = "{}")]
        params: String,
    },
    /// Repository summary (`repo.summary`).
    Summary,
    /// References: HEAD, branches, remotes, tags, stashes, worktrees, submodules (`repo.refs`).
    Refs,
    /// Commits in graph order (`repo.log`).
    Log {
        /// Stop after this many commits.
        #[arg(long)]
        limit: Option<usize>,
        /// Walk from HEAD only instead of all references.
        #[arg(long)]
        head: bool,
        /// Follow first parents only.
        #[arg(long)]
        first_parent: bool,
    },
    /// Commits with lane layout (`repo.graph`).
    Graph {
        /// Stop after this many commits.
        #[arg(long)]
        limit: Option<usize>,
        /// Walk from HEAD only instead of all references.
        #[arg(long)]
        head: bool,
        /// Print ASCII art instead of JSON.
        #[arg(long)]
        ascii: bool,
    },
    /// Working tree status (`repo.status`).
    Status,
    /// Diff between two sides (`repo.diff`): revisions, `index`, `worktree` or `empty`.
    Diff {
        /// Old side (default HEAD).
        #[arg(default_value = "HEAD")]
        old: String,
        /// New side (default worktree).
        #[arg(default_value = "worktree")]
        new: String,
        /// Restrict to paths (files or directories).
        #[arg(long = "path")]
        paths: Vec<String>,
        /// Context lines.
        #[arg(long, default_value_t = 3)]
        context: u32,
        /// Print unified diff text instead of JSON.
        #[arg(long)]
        text: bool,
    },
    /// Blame a file (`repo.blame`).
    Blame {
        /// Path relative to the worktree root.
        path: String,
        /// Revision (default HEAD).
        #[arg(long)]
        rev: Option<String>,
    },
    /// History of a file (`repo.history`).
    History {
        /// Path relative to the worktree root.
        path: String,
        /// Do not follow renames.
        #[arg(long)]
        no_follow: bool,
        /// Stop after this many entries.
        #[arg(long)]
        limit: Option<usize>,
    },
}

static NULL: Value = Value::Null;

/// Field access that never panics on missing keys.
fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.get(key).unwrap_or(&NULL)
}

fn render_diff_text(files: &Value) -> String {
    let mut out = String::new();
    for file in files.as_array().into_iter().flatten() {
        let path = field(file, "path").as_str().unwrap_or_default();
        let old_path = field(file, "old_path").as_str().unwrap_or(path);
        out.push_str(&format!("diff --git a/{old_path} b/{path}\n"));
        if field(file, "binary").as_bool().unwrap_or(false) {
            out.push_str("Binary files differ\n");
            continue;
        }
        out.push_str(&format!("--- a/{old_path}\n+++ b/{path}\n"));
        for hunk in field(file, "hunks").as_array().into_iter().flatten() {
            let side = |start: &Value, lines: &Value| {
                let (s, l) = (start.as_u64().unwrap_or(0), lines.as_u64().unwrap_or(0));
                if l == 1 {
                    s.to_string()
                } else {
                    format!("{s},{l}")
                }
            };
            out.push_str(&format!(
                "@@ -{} +{} @@\n",
                side(field(hunk, "old_start"), field(hunk, "old_lines")),
                side(field(hunk, "new_start"), field(hunk, "new_lines"))
            ));
            for line in field(hunk, "lines").as_array().into_iter().flatten() {
                let prefix = match field(line, "kind").as_str() {
                    Some("added") => '+',
                    Some("removed") => '-',
                    _ => ' ',
                };
                out.push(prefix);
                out.push_str(field(line, "text").as_str().unwrap_or_default());
                out.push('\n');
                if field(line, "no_newline").as_bool().unwrap_or(false) {
                    out.push_str("\\ No newline at end of file\n");
                }
            }
        }
    }
    out
}

/// Whether a repository is opened before the subcommand runs. For `run`
/// the registry says what the command needs; the subcommands of their own
/// all read a repository.
fn opening(registry: &Registry, cmd: &Cmd) -> Opening {
    match cmd {
        Cmd::Commands { .. } => Opening::No,
        Cmd::Run { id, .. } => running::opening(registry, id),
        _ => Opening::Must,
    }
}

/// Open the repository the subcommand works on, when it works on one.
fn open_repository(registry: &Registry, ctx: &mut CommandContext, cli: &Cli) -> Result<()> {
    let wanted = opening(registry, &cli.command);
    if wanted == Opening::No {
        return Ok(());
    }
    let path = cli.repo.as_deref().unwrap_or(".");
    match registry.run("repo.open", ctx, &json!({ "path": path })) {
        Ok(_) => Ok(()),
        // A command that can do without runs without, unless a repository
        // was named: then that one was meant, and it is not there.
        Err(_) if wanted == Opening::IfThere && cli.repo.is_none() => Ok(()),
        Err(err) => Err(err).with_context(|| format!("opening repository at {path}")),
    }
}

/// Whether an error is the engine saying that it was stopped on request.
fn was_stopped(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<bitbraid_engine::CommandError>(),
            Some(bitbraid_engine::CommandError::Git(bitbraid_git::Error::Invalid { what, detail }))
                if *what == "operation" && detail == "cancelled"
        )
    })
}

fn run(cli: Cli, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) -> Result<()> {
    let registry: Registry = bitbraid_integrations::registry();
    // Where the application keeps its own files (profiles): the same
    // directory the desktop application uses, unless another is named.
    let mut ctx = CommandContext {
        config_dir: std::env::var_os("BITBRAID_CONFIG_DIR")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                directories::ProjectDirs::from("", "", "bitbraid")
                    .map(|dirs| dirs.config_dir().to_path_buf())
            }),
        cancel,
        ..Default::default()
    };
    // What a command reports while it runs goes to standard error, and the
    // line it leaves there is ended before anything else is written.
    let meter = running::Meter::new();
    if !cli.no_progress {
        ctx.progress = Some(meter.sink());
    }
    let _line = running::EndsLine(meter);
    open_repository(&registry, &mut ctx, &cli)?;
    let print = |value: &Value| -> Result<()> {
        if cli.compact {
            println!("{}", serde_json::to_string(value)?);
        } else {
            println!("{}", serde_json::to_string_pretty(value)?);
        }
        Ok(())
    };
    match cli.command {
        Cmd::Commands { json } => {
            if json {
                println!("{}", catalog::to_json(&registry)?);
            } else {
                for spec in registry.iter() {
                    println!("{:<16} {:<24} {}", spec.id, spec.label, spec.description);
                }
            }
        }
        Cmd::Run { id, params } => {
            let params: Value =
                serde_json::from_str(&params).context("--params must be a JSON object")?;
            print(&registry.run(&id, &mut ctx, &params)?)?;
        }
        Cmd::Summary => print(&registry.run("repo.summary", &mut ctx, &Value::Null)?)?,
        Cmd::Refs => print(&registry.run("repo.refs", &mut ctx, &Value::Null)?)?,
        Cmd::Log {
            limit,
            head,
            first_parent,
        } => {
            let params = json!({ "limit": limit, "tips": if head { "head" } else { "all" }, "first_parent": first_parent });
            print(&registry.run("repo.log", &mut ctx, &params)?)?;
        }
        Cmd::Graph { limit, head, ascii } => {
            let params = json!({ "limit": limit, "tips": if head { "head" } else { "all" }, "ascii": ascii });
            let out = registry.run("repo.graph", &mut ctx, &params)?;
            if ascii {
                print!("{}", field(&out, "ascii").as_str().unwrap_or_default());
            } else {
                print(&out)?;
            }
        }
        Cmd::Status => print(&registry.run("repo.status", &mut ctx, &Value::Null)?)?,
        Cmd::Diff {
            old,
            new,
            paths,
            context,
            text,
        } => {
            let params = json!({ "old": old, "new": new, "paths": paths, "context": context });
            let out = registry.run("repo.diff", &mut ctx, &params)?;
            if text {
                print!("{}", render_diff_text(&out));
            } else {
                print(&out)?;
            }
        }
        Cmd::Blame { path, rev } => {
            print(&registry.run("repo.blame", &mut ctx, &json!({ "path": path, "rev": rev }))?)?;
        }
        Cmd::History {
            path,
            no_follow,
            limit,
        } => {
            let params = json!({ "path": path, "follow": !no_follow, "max_entries": limit });
            print(&registry.run("repo.history", &mut ctx, &params)?)?;
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    // Started by git or ssh as the askpass program of a running BitBraid:
    // pass the question on and print the answer, nothing else.
    if let Some(code) = bitbraid_git::askpass::client_main() {
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    let cli = Cli::parse();
    // Ctrl-C stops the command that is running through the engine's flag.
    let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    running::stop_on_interrupt(&cancel);
    let done = run(cli, cancel.clone());
    // What was run goes into the repository's activity log on a thread of
    // its own, and this process ends as soon as it has answered: wait for
    // the entries to be written, which takes a moment and no longer.
    bitbraid_git::activity::flush(std::time::Duration::from_secs(2));
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) if was_stopped(&err) => {
            eprintln!("error: stopped");
            ExitCode::from(running::INTERRUPTED)
        }
        // Git ended on the same Ctrl-C before the engine stopped it: the
        // terminal sends the signal to every process in the foreground.
        Err(err) if cancel.load(std::sync::atomic::Ordering::SeqCst) => {
            eprintln!("error: stopped ({err:#})");
            ExitCode::from(running::INTERRUPTED)
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}
