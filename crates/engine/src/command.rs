//! Command descriptors, handlers, the execution context and the registry.

use std::{
    any::{Any, TypeId},
    collections::BTreeMap,
    sync::Arc,
};

use serde::Serialize;
use serde::de::DeserializeOwned;

/// Static description of a command.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CommandSpec {
    /// Stable, dotted identifier, for example `repo.open`.
    pub id: &'static str,
    /// Human-readable label shown in menus and the palette.
    pub label: &'static str,
    /// One-line description for the palette and CLI help.
    pub description: &'static str,
}

/// JSON parameters of a command.
pub type Params = serde_json::Value;
/// JSON result of a command.
pub type Output = serde_json::Value;

/// Where a long-running command reports what it is doing.
pub type ProgressSink = std::sync::Arc<dyn Fn(bitbraid_git::Progress) + Send + Sync>;

/// Values a higher layer keeps in the context, one per type, which the
/// engine never looks into: the integrations crate keeps the
/// services its commands use here, and its tests keep stand-ins.
#[derive(Clone, Default)]
pub struct Extensions {
    values: BTreeMap<TypeId, (&'static str, Arc<dyn Any + Send + Sync>)>,
}

impl Extensions {
    /// Keep `value`, in place of one of the same type kept before.
    pub fn insert<T: Send + Sync + 'static>(&mut self, value: Arc<T>) {
        self.values
            .insert(TypeId::of::<T>(), (std::any::type_name::<T>(), value));
    }

    /// The value of this type, when one is kept.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Arc<T>> {
        let (_, value) = self.values.get(&TypeId::of::<T>())?;
        value.clone().downcast::<T>().ok()
    }

    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

impl std::fmt::Debug for Extensions {
    /// The names of the types kept, and nothing of what they hold.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_set()
            .entries(self.values.values().map(|(name, _)| name))
            .finish()
    }
}

/// Everything a command may touch.
#[derive(Default)]
pub struct CommandContext {
    /// The repository the session works on, once opened.
    pub repo: Option<bitbraid_git::Repository>,
    /// The undo journal of that repository.
    pub journal: Option<crate::journal::Journal>,
    /// Where commands that take a while report their progress.
    pub progress: Option<ProgressSink>,
    /// Set from outside to stop the command that is running.
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Where the application keeps its own files (`profiles.toml`). The
    /// application sets it; commands that need it refuse without it.
    pub config_dir: Option<std::path::PathBuf>,
    /// What commands registered by a higher layer need besides their
    /// parameters; empty unless the application or a test put it there.
    pub extensions: Extensions,
}

impl std::fmt::Debug for CommandContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CommandContext")
            .field("repo", &self.repo)
            .field("journal", &self.journal)
            .field("progress", &self.progress.is_some())
            .field("cancel", &self.cancel)
            .field("config_dir", &self.config_dir)
            .field("extensions", &self.extensions)
            .finish()
    }
}

impl CommandContext {
    /// The open repository, or the error every repository command reports.
    pub fn repo(&self) -> Result<&bitbraid_git::Repository, CommandError> {
        self.repo.as_ref().ok_or(CommandError::NoRepository)
    }

    /// Open a repository and load its journal.
    pub fn open(&mut self, path: &str) -> Result<bitbraid_git::RepoSummary, CommandError> {
        let repo = bitbraid_git::Repository::open(path)?;
        let summary = repo.summary()?;
        self.journal = Some(crate::journal::Journal::load(&repo));
        self.repo = Some(repo);
        Ok(summary)
    }

    /// Open the repository again when another git executable was chosen
    /// since it was opened, so that what is read here comes from the same
    /// configuration as what the git children read. Only a comparison when
    /// nothing changed; a repository that cannot be opened again is kept
    /// as it is.
    pub fn follow_git(&mut self) {
        if let Some(repo) = &mut self.repo {
            let _ = repo.follow_git();
        }
    }

    /// Read the journal from disk again. Another context on the same
    /// repository (a second thread running operations) writes it too.
    pub fn reload_journal(&mut self) {
        if let Some(repo) = &self.repo {
            self.journal = Some(crate::journal::Journal::load(repo));
        }
    }

    /// Record a journal entry and persist it.
    pub fn record(
        &mut self,
        label: String,
        change: crate::journal::Change,
    ) -> Result<(), CommandError> {
        let repo = self.repo.as_ref().ok_or(CommandError::NoRepository)?;
        let journal = self.journal.get_or_insert_with(Default::default);
        journal.record(label, change);
        journal.save(repo);
        Ok(())
    }
}

/// Errors while running a command.
#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    /// No command with this id.
    #[error("{}", say!("command-unknown", id = format!("{:?}", .0)))]
    Unknown(String),
    /// The command needs an open repository and there is none.
    #[error("{}", say!("command-no-repository"))]
    NoRepository,
    /// A parameter is missing or has the wrong shape.
    #[error("{}", say!("command-invalid-param", name = format!("{:?}", .name), detail = .detail.as_str()))]
    InvalidParam {
        /// Parameter name.
        name: String,
        /// What is wrong.
        detail: String,
    },
    /// The open repository is bare, which is opened read-only, and the
    /// command would change it. Nothing was done.
    #[error("{}", say!("command-read-only", command = .0.as_str()))]
    ReadOnly(String),
    /// The repository model failed.
    #[error(transparent)]
    Git(#[from] bitbraid_git::Error),
    /// Serialising the result failed.
    #[error("{}", say!("command-serialize", error = .0.to_string()))]
    Serialize(#[from] serde_json::Error),
}

/// A command implementation.
pub type Handler = fn(&mut CommandContext, &Params) -> Result<Output, CommandError>;

/// Errors from registering commands.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RegistryError {
    /// Two commands share one id.
    #[error("{}", say!("command-duplicate", id = format!("{:?}", .0)))]
    Duplicate(String),
    /// An id does not follow the `area.verb` convention.
    #[error("{}", say!("command-malformed-id", id = format!("{:?}", .0)))]
    MalformedId(String),
}

struct Entry {
    spec: CommandSpec,
    handler: Handler,
    /// What the command needs of a repository and may do to one: from
    /// `access::of` for the engine's own, as given for one that brought it.
    access: crate::access::Access,
    /// What the command takes, from `params::schema`, looked up once, or
    /// as the command brought it.
    params: Option<&'static [crate::params::ParamSpec]>,
}

/// All registered commands, keyed by id.
#[derive(Default)]
pub struct Registry {
    entries: BTreeMap<&'static str, Entry>,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.entries.keys()).finish()
    }
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a command with its handler. Fails on duplicate or malformed ids.
    pub fn register(&mut self, spec: CommandSpec, handler: Handler) -> Result<(), RegistryError> {
        let access = crate::access::of(spec.id);
        let params = crate::params::schema(spec.id);
        self.insert(spec, handler, access, params)
    }

    /// Register a command that brings what it needs and may do, and what
    /// it takes, rather than having lines in the engine's tables: one
    /// registered from a higher layer (`AGENTS.md` section 2). Fails as
    /// [`Registry::register`] does.
    pub fn register_described(
        &mut self,
        spec: CommandSpec,
        handler: Handler,
        access: crate::access::Access,
        params: &'static [crate::params::ParamSpec],
    ) -> Result<(), RegistryError> {
        self.insert(spec, handler, access, Some(params))
    }

    fn insert(
        &mut self,
        spec: CommandSpec,
        handler: Handler,
        access: crate::access::Access,
        params: Option<&'static [crate::params::ParamSpec]>,
    ) -> Result<(), RegistryError> {
        if !well_formed_id(spec.id) {
            return Err(RegistryError::MalformedId(spec.id.to_owned()));
        }
        if self.entries.contains_key(spec.id) {
            return Err(RegistryError::Duplicate(spec.id.to_owned()));
        }
        self.entries.insert(
            spec.id,
            Entry {
                spec,
                handler,
                access,
                params,
            },
        );
        Ok(())
    }

    /// Look up a command by id.
    pub fn get(&self, id: &str) -> Option<&CommandSpec> {
        self.entries.get(id).map(|entry| &entry.spec)
    }

    /// What a command takes: one row per parameter. Empty for a command
    /// that takes nothing; none for a command that is not registered or
    /// that `params::schema` has no row for and that brought none.
    pub fn params(&self, id: &str) -> Option<&'static [crate::params::ParamSpec]> {
        self.entries.get(id).and_then(|entry| entry.params)
    }

    /// All commands in id order.
    pub fn iter(&self) -> impl Iterator<Item = &CommandSpec> {
        self.entries.values().map(|entry| &entry.spec)
    }

    /// What a registered command needs of a repository and may do to one
    /// (see [`crate::access`]); none for an id that is not registered.
    pub fn access(&self, id: &str) -> Option<crate::access::Access> {
        self.entries.get(id).map(|entry| entry.access)
    }

    /// Number of registered commands.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether nothing is registered.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Run a command by id.
    pub fn run(
        &self,
        id: &str,
        ctx: &mut CommandContext,
        params: &Params,
    ) -> Result<Output, CommandError> {
        let entry = self
            .entries
            .get(id)
            .ok_or_else(|| CommandError::Unknown(id.to_owned()))?;
        // A repository that was open when another git was chosen follows
        // it before anything is read.
        ctx.follow_git();
        // A bare repository is read-only: this is the one place where what
        // would change it is refused, before anything runs.
        if ctx.repo.as_ref().is_some_and(|repo| repo.is_bare())
            && entry.access.changes_repository(params)
        {
            return Err(CommandError::ReadOnly(format!(
                "\"{}\" ({})",
                entry.spec.label, entry.spec.id
            )));
        }
        // What git is started for meanwhile is this command's in the
        // activity log, and this repository's wherever git runs.
        let _named = crate::activity_cmds::during(entry.spec.id, ctx);
        let ran = (entry.handler)(ctx, params);
        // Git added more packs while the repository was open than the
        // handle has room for (`bitbraid_git::repo::is_out_of_room`): it
        // is opened again, and what only reads is asked once more. What
        // may have changed something is not done twice; it fails as it
        // did, and the next command has the room.
        match ran {
            Err(err) if bitbraid_git::repo::came_from_out_of_room(&err) => {
                let reopened = ctx.repo.as_mut().is_some_and(|repo| repo.reopen().is_ok());
                if reopened && !entry.access.changes_repository(params) {
                    (entry.handler)(ctx, params)
                } else {
                    Err(err)
                }
            }
            ran => ran,
        }
    }

    /// Run a command nobody asked for: one an interface runs to keep what
    /// it shows current, or to look a repository over in the background.
    /// It reads only what is on this machine (`bitbraid_git::exe::local_only`):
    /// git fetches nothing a partial clone left on its remote and reaches no
    /// remote at all, and a read that would need what was left there fails
    /// saying so, or answers that it cannot tell.
    /// Otherwise as [`Registry::run`].
    pub fn run_unasked(
        &self,
        id: &str,
        ctx: &mut CommandContext,
        params: &Params,
    ) -> Result<Output, CommandError> {
        let _here = bitbraid_git::exe::local_only();
        self.run(id, ctx, params)
    }
}

fn well_formed_id(id: &str) -> bool {
    let mut count = 0usize;
    for part in id.split('.') {
        count = count.saturating_add(1);
        if part.is_empty()
            || !part
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return false;
        }
    }
    count >= 2
}

/// Read an optional parameter, falling back to `default` when absent or null.
pub fn param_or<T: DeserializeOwned>(
    params: &Params,
    name: &str,
    default: T,
) -> Result<T, CommandError> {
    match params.get(name) {
        None | Some(serde_json::Value::Null) => Ok(default),
        Some(value) => {
            serde_json::from_value(value.clone()).map_err(|err| CommandError::InvalidParam {
                name: name.to_owned(),
                detail: err.to_string(),
            })
        }
    }
}

/// Read a required parameter.
pub fn param<T: DeserializeOwned>(params: &Params, name: &str) -> Result<T, CommandError> {
    match params.get(name) {
        None | Some(serde_json::Value::Null) => Err(CommandError::InvalidParam {
            name: name.to_owned(),
            detail: say!("command-param-missing").to_owned(),
        }),
        Some(value) => {
            serde_json::from_value(value.clone()).map_err(|err| CommandError::InvalidParam {
                name: name.to_owned(),
                detail: err.to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OPEN: CommandSpec = CommandSpec {
        id: "test.echo",
        label: "Echo",
        description: "Returns its parameters",
    };

    fn echo(_: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
        Ok(params.clone())
    }

    #[test]
    fn registers_runs_and_lists_in_id_order() {
        let mut reg = Registry::new();
        reg.register(
            CommandSpec {
                id: "z.last",
                ..OPEN
            },
            echo,
        )
        .unwrap();
        reg.register(OPEN, echo).unwrap();
        let ids: Vec<&str> = reg.iter().map(|s| s.id).collect();
        assert_eq!(ids, ["test.echo", "z.last"]);
        assert_eq!(reg.get("test.echo"), Some(&OPEN));
        let mut ctx = CommandContext::default();
        let out = reg
            .run("test.echo", &mut ctx, &serde_json::json!({"a": 1}))
            .unwrap();
        assert_eq!(out["a"], 1);
        assert!(matches!(
            reg.run("nope.x", &mut ctx, &Params::Null),
            Err(CommandError::Unknown(_))
        ));
    }

    #[test]
    fn rejects_duplicates_and_malformed_ids() {
        let mut reg = Registry::new();
        reg.register(OPEN, echo).unwrap();
        assert_eq!(
            reg.register(OPEN, echo),
            Err(RegistryError::Duplicate("test.echo".into()))
        );
        for bad in ["Open", "repo", "repo..open", "repo.Open", ""] {
            assert_eq!(
                reg.register(CommandSpec { id: bad, ..OPEN }, echo),
                Err(RegistryError::MalformedId(bad.into())),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn params_helpers_validate_shape() {
        let params = serde_json::json!({"limit": 5, "name": "x"});
        assert_eq!(param_or::<usize>(&params, "limit", 1).unwrap(), 5);
        assert_eq!(param_or::<usize>(&params, "missing", 1).unwrap(), 1);
        assert!(param::<usize>(&params, "name").is_err());
        assert!(param::<String>(&params, "missing").is_err());
    }

    #[test]
    fn a_command_from_elsewhere_brings_its_own_description() {
        use crate::access::Access;
        use crate::params::{ParamKind, ParamSpec};
        const TAKES: &[ParamSpec] = &[ParamSpec {
            name: "number",
            kind: ParamKind::Number,
            required: true,
            description: message!("params-commit-ids"),
        }];
        let mut reg = Registry::new();
        reg.register_described(
            CommandSpec {
                id: "elsewhere.show",
                ..OPEN
            },
            echo,
            Access::FREE,
            TAKES,
        )
        .unwrap();
        assert_eq!(reg.access("elsewhere.show"), Some(Access::FREE));
        assert_eq!(reg.params("elsewhere.show"), Some(TAKES));
        // One the engine's tables do not know, registered plainly, is
        // taken for a change and described by nothing.
        reg.register(OPEN, echo).unwrap();
        assert_eq!(reg.access("test.echo"), Some(Access::WRITES));
        assert_eq!(reg.params("test.echo"), None);
        // The same rules for ids hold.
        assert_eq!(
            reg.register_described(OPEN, echo, Access::FREE, TAKES),
            Err(RegistryError::Duplicate("test.echo".into()))
        );
        assert_eq!(
            reg.register_described(CommandSpec { id: "Bad", ..OPEN }, echo, Access::FREE, &[]),
            Err(RegistryError::MalformedId("Bad".into()))
        );
    }

    #[test]
    fn extensions_keep_one_value_per_type() {
        #[derive(Debug, PartialEq)]
        struct Secretive(&'static str);
        let mut kept = Extensions::default();
        assert!(kept.is_empty());
        assert!(kept.get::<Secretive>().is_none());
        kept.insert(Arc::new(Secretive("first")));
        kept.insert(Arc::new(7u32));
        kept.insert(Arc::new(Secretive("hidden value")));
        assert_eq!(kept.get::<Secretive>().unwrap().0, "hidden value");
        assert_eq!(*kept.get::<u32>().unwrap(), 7);
        assert!(kept.get::<u64>().is_none());
        // Its text names the types and shows nothing they hold.
        let ctx = CommandContext {
            extensions: kept,
            ..Default::default()
        };
        let text = format!("{ctx:?}");
        assert!(text.contains("Secretive") && text.contains("u32"), "{text}");
        assert!(!text.contains("hidden value"), "{text}");
    }

    #[test]
    fn repo_commands_need_a_repository() {
        let ctx = CommandContext::default();
        assert!(matches!(ctx.repo(), Err(CommandError::NoRepository)));
    }
}
