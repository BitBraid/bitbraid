//! Workspace commands: named groups of repositories, what the page of
//! repositories shows them with, and their defaults. They are kept in
//! `workspaces.toml` (`crate::workspace`).
//!
//! Making, changing and deleting a workspace never touches one of its
//! repositories: a repository that is added is only looked at, for
//! whether it is one and where it is cloned from.
//!
//! Sources: gitrepository-layout(5) (what makes a folder a repository);
//! git-remote(1).

use std::path::Path;

use serde_json::{Value, json};

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    workspace::{
        self, Layout, MAX_NAME, MAX_REPOSITORIES, MAX_WORKSPACES, Workspace, WorkspaceRepository,
        folded, refused,
    },
    workspace_scan,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "workspace.list",
            label: say!("workspace-list"),
            description: say!("workspace-list-description"),
        },
        list,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.create",
            label: say!("workspace-create"),
            description: say!("workspace-create-description"),
        },
        create,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.update",
            label: say!("workspace-update"),
            description: say!("workspace-update-description"),
        },
        update,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.delete",
            label: say!("workspace-delete"),
            description: say!("workspace-delete-description"),
        },
        delete,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.reorder",
            label: say!("workspace-reorder"),
            description: say!("workspace-reorder-description"),
        },
        reorder,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.add",
            label: say!("workspace-add"),
            description: say!("workspace-add-description"),
        },
        add,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.remove",
            label: say!("workspace-remove"),
            description: say!("workspace-remove-description"),
        },
        remove,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.scan",
            label: say!("workspace-scan"),
            description: say!("workspace-scan-description"),
        },
        scan,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.sync_folder",
            label: say!("workspace-sync-folder"),
            description: say!("workspace-sync-folder-description"),
        },
        sync_folder,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.import_code_workspace",
            label: say!("workspace-import-code-workspace"),
            description: say!("workspace-import-code-workspace-description"),
        },
        import_code_workspace,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.fetch",
            label: say!("workspace-fetch"),
            description: say!("workspace-fetch-description"),
        },
        crate::workspace_bulk::fetch,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.pull",
            label: say!("workspace-pull"),
            description: say!("workspace-pull-description"),
        },
        crate::workspace_bulk::pull,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.clone_missing",
            label: say!("workspace-clone-missing"),
            description: say!("workspace-clone-missing-description"),
        },
        crate::workspace_bulk::clone_missing,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.locate",
            label: say!("workspace-locate"),
            description: say!("workspace-locate-description"),
        },
        locate,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.defaults",
            label: say!("workspace-defaults"),
            description: say!("workspace-defaults-description"),
        },
        defaults,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.export",
            label: say!("workspace-export"),
            description: say!("workspace-export-description"),
        },
        crate::workspace_file::export,
    );
    let _ = registry.register(
        CommandSpec {
            id: "workspace.import",
            label: say!("workspace-import"),
            description: say!("workspace-import-description"),
        },
        crate::workspace_file::import,
    );
}

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

/// Where the workspaces are kept, or the refusal of every workspace
/// command when the application has not said.
pub(crate) fn directory(ctx: &CommandContext) -> Result<&Path, CommandError> {
    ctx.config_dir
        .as_deref()
        .ok_or_else(|| refused(say!("workspace-no-directory")))
}

/// The id of a workspace as a parameter.
pub(crate) fn id_param(params: &Params) -> Result<String, CommandError> {
    let text: String = param(params, "id")?;
    workspace::id(&text).map_err(|why| invalid("id", why))
}

/// Where the workspace of an id is among the others.
pub(crate) fn position(workspaces: &[Workspace], id: &str) -> Result<usize, CommandError> {
    workspaces
        .iter()
        .position(|workspace| workspace.id == id)
        .ok_or_else(|| refused(say!("workspace-no-such", id = format!("{id:?}"))))
}

/// A workspace as a command answers with it: its fields, and the paths of
/// its repositories that are not there.
pub(crate) fn shown(workspace: &Workspace) -> Result<Value, CommandError> {
    let mut out = serde_json::to_value(workspace)?;
    let missing: Vec<&str> = workspace
        .repositories
        .iter()
        .filter(|entry| !entry.is_present())
        .map(|entry| entry.path.as_str())
        .collect();
    if let Some(object) = out.as_object_mut() {
        object.insert("missing".to_owned(), json!(missing));
    }
    Ok(out)
}

/// A text parameter that may be absent: `None` when it is, the text
/// otherwise (an empty one included: it takes the field away).
fn text_param(params: &Params, name: &str) -> Result<Option<String>, CommandError> {
    param_or(params, name, None)
}

/// A field as a parameter gives it: absent stays absent, an empty text is
/// a field taken away, anything else is checked by `check`.
fn field(
    params: &Params,
    name: &str,
    check: impl Fn(&str) -> Result<String, String>,
) -> Result<Option<Option<String>>, CommandError> {
    match text_param(params, name)? {
        None => Ok(None),
        Some(text) if text.trim().is_empty() => Ok(Some(None)),
        Some(text) => check(&text)
            .map(|kept| Some(Some(kept)))
            .map_err(|why| invalid(name, why)),
    }
}

/// What a workspace's own fields are to become, from the parameters of
/// `workspace.create` or `workspace.update`.
#[derive(Default)]
struct Fields {
    name: Option<String>,
    colour: Option<Option<String>>,
    icon: Option<Option<String>>,
    description: Option<Option<String>>,
    default_profile: Option<Option<String>>,
    follows_folder: Option<Option<String>>,
    layout: Option<Option<Layout>>,
}

impl Fields {
    fn read(params: &Params) -> Result<Self, CommandError> {
        let name = match text_param(params, "name")? {
            Some(text) => {
                Some(workspace::short_text(&text, MAX_NAME).map_err(|why| invalid("name", why))?)
            }
            None => None,
        };
        let description = match text_param(params, "description")? {
            None => None,
            Some(text) => {
                Some(workspace::description(&text).map_err(|why| invalid("description", why))?)
            }
        };
        let layout = match params.get("layout") {
            None | Some(Value::Null) => None,
            Some(_) => {
                let layout: Layout = param(params, "layout")?;
                let layout = workspace::layout(layout).map_err(|why| invalid("layout", why))?;
                Some(Some(layout).filter(|layout| !layout.is_empty()))
            }
        };
        Ok(Self {
            name,
            colour: field(params, "colour", workspace::colour)?,
            icon: field(params, "icon", workspace::icon)?,
            description,
            default_profile: field(params, "default_profile", |text| {
                workspace::short_text(text, crate::profile_cmds::MAX_NAME)
            })?,
            follows_folder: field(params, "follows_folder", workspace::full_path)?,
            layout,
        })
    }

    fn apply(self, workspace: &mut Workspace) {
        if let Some(name) = self.name {
            workspace.name = name;
        }
        if let Some(colour) = self.colour {
            workspace.colour = colour;
        }
        if let Some(icon) = self.icon {
            workspace.icon = icon;
        }
        if let Some(description) = self.description {
            workspace.description = description;
        }
        if let Some(profile) = self.default_profile {
            workspace.default_profile = profile;
        }
        if let Some(folder) = self.follows_folder {
            workspace.follows_folder = folder;
        }
        if let Some(layout) = self.layout {
            workspace.layout = layout;
        }
    }
}

/// Refuse a name that another workspace has.
pub(crate) fn name_is_free(
    workspaces: &[Workspace],
    name: &str,
    own: Option<&str>,
) -> Result<(), CommandError> {
    let key = folded(name);
    let taken = workspaces
        .iter()
        .any(|other| folded(&other.name) == key && Some(other.id.as_str()) != own);
    if taken {
        return Err(refused(say!(
            "workspace-name-taken",
            name = format!("{name:?}")
        )));
    }
    Ok(())
}

/// The repositories a parameter names, checked: `paths` (full paths of
/// folders that are repositories) and `repositories` (`{path,
/// remote_url}`, where a folder that is not there needs the address).
/// Nothing is taken when one of them cannot be. A repository that is
/// there and was given no address gets the one of its `origin`.
pub(crate) fn entries(params: &Params) -> Result<Vec<WorkspaceRepository>, CommandError> {
    let paths: Vec<String> = param_or(params, "paths", Vec::new())?;
    let given: Vec<WorkspaceRepository> = param_or(params, "repositories", Vec::new())?;
    if paths.len().saturating_add(given.len()) > MAX_REPOSITORIES {
        return Err(invalid(
            "paths",
            say!(
                "workspace-at-most-repositories-at-a-time",
                most = MAX_REPOSITORIES
            ),
        ));
    }
    let mut out = Vec::with_capacity(paths.len().saturating_add(given.len()));
    let named = paths
        .into_iter()
        .map(|path| {
            (
                "paths",
                WorkspaceRepository {
                    path,
                    remote_url: None,
                },
            )
        })
        .chain(given.into_iter().map(|entry| ("repositories", entry)));
    for (parameter, entry) in named {
        let shown = workspace::cut(&entry.path);
        let entry = workspace::repository(entry)
            .map_err(|why| invalid(parameter, format!("{shown:?}: {why}")))?;
        let checked = checked_entry(entry).map_err(|why| invalid(parameter, why))?;
        if !out
            .iter()
            .any(|kept: &WorkspaceRepository| kept.path == checked.path)
        {
            out.push(checked);
        }
    }
    Ok(out)
}

/// A repository entry as it is added: a folder that is there has to be a
/// repository, and one that is not has to have an address to clone from.
pub(crate) fn checked_entry(entry: WorkspaceRepository) -> Result<WorkspaceRepository, String> {
    let path = Path::new(&entry.path);
    match std::fs::symlink_metadata(path) {
        Err(_) => {
            if entry.remote_url.is_none() {
                return Err(say!(
                    "workspace-not-there-no-address",
                    path = entry.path.as_str()
                ));
            }
            Ok(entry)
        }
        Ok(_) if !path.is_dir() => Err(say!(
            "workspace-path-not-a-folder",
            path = entry.path.as_str()
        )),
        Ok(_) if !bitbraid_git::glance::is_repository_folder(path) => Err(say!(
            "workspace-not-a-repository-folder",
            path = entry.path.as_str()
        )),
        Ok(_) => {
            let remote_url = entry.remote_url.or_else(|| {
                bitbraid_git::glance::remote_url(path)
                    .and_then(|url| workspace::remote_url(&url).ok())
            });
            Ok(WorkspaceRepository {
                path: entry.path,
                remote_url,
            })
        }
    }
}

/// Put `entries` into a workspace: those it has already are left as they
/// are, except that one without an address gets the one given now.
/// Answers the paths that were added and those it had.
pub(crate) fn merge(
    workspace: &mut Workspace,
    entries: Vec<WorkspaceRepository>,
) -> Result<(Vec<String>, Vec<String>), CommandError> {
    let mut added = Vec::new();
    let mut already = Vec::new();
    for entry in entries {
        match workspace
            .repositories
            .iter_mut()
            .find(|kept| kept.path == entry.path)
        {
            Some(kept) => {
                if kept.remote_url.is_none() {
                    kept.remote_url = entry.remote_url;
                }
                already.push(entry.path);
            }
            None => {
                if workspace.repositories.len() >= MAX_REPOSITORIES {
                    return Err(refused(say!(
                        "workspace-holds-at-most",
                        most = MAX_REPOSITORIES
                    )));
                }
                added.push(entry.path.clone());
                workspace.repositories.push(entry);
            }
        }
    }
    Ok((added, already))
}

fn list(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let only = match params.get("id") {
        None | Some(Value::Null) => None,
        Some(_) => Some(id_param(params)?),
    };
    let loaded = workspace::load(directory(ctx)?)?;
    let mut shown_all = Vec::new();
    for workspace in &loaded.workspaces {
        if only.as_ref().is_none_or(|id| *id == workspace.id) {
            shown_all.push(shown(workspace)?);
        }
    }
    if let Some(id) = &only
        && shown_all.is_empty()
    {
        return Err(refused(say!("workspace-no-such", id = format!("{id:?}"))));
    }
    Ok(json!({ "workspaces": shown_all, "read_only": loaded.read_only }))
}

/// Make a workspace called `name` with `fields` and `entries`, and keep it.
fn make(
    dir: &Path,
    name: &str,
    fields: Fields,
    entries: Vec<WorkspaceRepository>,
) -> Result<Workspace, CommandError> {
    workspace::changing(dir, |workspaces| {
        if workspaces.len() >= MAX_WORKSPACES {
            return Err(refused(say!(
                "workspace-too-many-workspaces",
                most = MAX_WORKSPACES
            )));
        }
        name_is_free(workspaces, name, None)?;
        let mut made = Workspace {
            id: workspace::new_id(name, workspaces),
            name: name.to_owned(),
            colour: None,
            icon: None,
            description: None,
            default_profile: None,
            follows_folder: None,
            layout: None,
            repositories: Vec::new(),
        };
        fields.apply(&mut made);
        merge(&mut made, entries)?;
        workspaces.push(made.clone());
        Ok(made)
    })
}

/// The repositories found by a scan or in a file, as entries of a
/// workspace.
fn found_entries(scanned: &workspace_scan::Scanned) -> Vec<WorkspaceRepository> {
    scanned
        .found
        .iter()
        .map(|found| WorkspaceRepository {
            path: found.path.clone(),
            remote_url: found.remote_url.clone(),
        })
        .collect()
}

/// The repositories of the folder a workspace follows, scanned as
/// `workspace.scan` does by default.
fn scan_followed(
    ctx: &CommandContext,
    folder: &str,
) -> Result<workspace_scan::Scanned, CommandError> {
    let path = Path::new(folder);
    if !path.is_dir() {
        return Err(refused(say!("workspace-folder-missing", folder)));
    }
    Ok(workspace_scan::scan(
        path,
        workspace_scan::MAX_DEPTH,
        workspace_scan::MAX_FOUND,
        &ctx.cancel,
    ))
}

fn create(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let fields = Fields::read(params)?;
    let name = fields
        .name
        .clone()
        .ok_or_else(|| invalid("name", say!("workspace-missing")))?;
    let mut entries = entries(params)?;
    let dir = directory(ctx)?;
    // One that follows a folder starts with every repository in it.
    let followed = match &fields.follows_folder {
        Some(Some(folder)) => Some(scan_followed(ctx, folder)?),
        _ => None,
    };
    if let Some(scanned) = &followed {
        for entry in found_entries(scanned) {
            if !entries.iter().any(|kept| kept.path == entry.path) {
                entries.push(entry);
            }
        }
    }
    let made = make(dir, &name, fields, entries)?;
    Ok(json!({
        "workspace": shown(&made)?,
        "created": true,
        "scanned": followed,
    }))
}

fn scan(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let text: String = param(params, "path")?;
    let folder = workspace::full_path(&text).map_err(|why| invalid("path", why))?;
    let depth: usize = param_or(params, "depth", workspace_scan::MAX_DEPTH)?;
    if depth > workspace_scan::MAX_DEPTH {
        return Err(invalid(
            "depth",
            say!("workspace-at-most", most = workspace_scan::MAX_DEPTH),
        ));
    }
    let limit: usize = param_or(params, "limit", workspace_scan::MAX_FOUND)?;
    if limit == 0 || limit > workspace_scan::MAX_FOUND {
        return Err(invalid(
            "limit",
            say!("workspace-from-one-to", most = workspace_scan::MAX_FOUND),
        ));
    }
    if !Path::new(&folder).is_dir() {
        return Err(refused(say!(
            "workspace-folder-missing",
            folder = folder.as_str()
        )));
    }
    let scanned = workspace_scan::scan(Path::new(&folder), depth, limit, &ctx.cancel);
    let mut out = serde_json::to_value(&scanned)?;
    if let Some(object) = out.as_object_mut() {
        object.insert("folder".to_owned(), json!(folder));
    }
    Ok(out)
}

fn sync_folder(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = id_param(params)?;
    let dir = directory(ctx)?;
    let folder = workspace::find(dir, &id)?
        .follows_folder
        .ok_or_else(|| refused(say!("workspace-follows-no-folder", id = format!("{id:?}"))))?;
    let scanned = scan_followed(ctx, &folder)?;
    let entries = found_entries(&scanned);
    let (changed, added) = workspace::changing(dir, |workspaces| {
        let at = position(workspaces, &id)?;
        let kept = workspaces
            .get_mut(at)
            .ok_or_else(|| refused(say!("workspace-no-such", id = format!("{id:?}"))))?;
        let (added, _) = merge(kept, entries)?;
        Ok((kept.clone(), added))
    })?;
    Ok(json!({
        "workspace": shown(&changed)?,
        "added": added,
        "skipped": scanned.skipped,
        "more_skipped": scanned.more_skipped,
        "complete": scanned.complete,
    }))
}

fn import_code_workspace(
    ctx: &mut CommandContext,
    params: &Params,
) -> Result<Output, CommandError> {
    let text: String = param(params, "file")?;
    let file = workspace::full_path(&text).map_err(|why| invalid("file", why))?;
    let into = match params.get("id") {
        None | Some(Value::Null) => None,
        Some(_) => Some(id_param(params)?),
    };
    let name = match text_param(params, "name")? {
        Some(text) => {
            Some(workspace::short_text(&text, MAX_NAME).map_err(|why| invalid("name", why))?)
        }
        None => None,
    };
    if into.is_some() && name.is_some() {
        return Err(invalid("name", say!("workspace-id-or-name")));
    }
    let scanned = workspace_scan::code_workspace(Path::new(&file))
        .map_err(|why| refused(format!("{file} {why}")))?;
    let entries = found_entries(&scanned);
    let mut out = serde_json::to_value(&scanned)?;
    let (workspace, added) = match (into, name) {
        (Some(id), _) => {
            let dir = directory(ctx)?;
            let (changed, added) = workspace::changing(dir, |workspaces| {
                let at = position(workspaces, &id)?;
                let kept = workspaces
                    .get_mut(at)
                    .ok_or_else(|| refused(say!("workspace-no-such", id = format!("{id:?}"))))?;
                let (added, _) = merge(kept, entries)?;
                Ok((kept.clone(), added))
            })?;
            (Some(shown(&changed)?), Some(added))
        }
        (None, Some(name)) => {
            let dir = directory(ctx)?;
            let added: Vec<String> = entries.iter().map(|entry| entry.path.clone()).collect();
            let made = make(dir, &name, Fields::default(), entries)?;
            (Some(shown(&made)?), Some(added))
        }
        (None, None) => (None, None),
    };
    if let Some(object) = out.as_object_mut() {
        object.insert("file".to_owned(), json!(file));
        object.insert("workspace".to_owned(), json!(workspace));
        object.insert("added".to_owned(), json!(added));
    }
    Ok(out)
}

fn update(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = id_param(params)?;
    let fields = Fields::read(params)?;
    let dir = directory(ctx)?;
    // A folder to follow from now on: its repositories come in at once,
    // as when a workspace is made with it.
    let followed = match &fields.follows_folder {
        Some(Some(folder)) => Some(scan_followed(ctx, folder)?),
        _ => None,
    };
    let found = followed.as_ref().map(found_entries).unwrap_or_default();
    let (changed, renamed_from, added) = workspace::changing(dir, |workspaces| {
        let at = position(workspaces, &id)?;
        if let Some(name) = &fields.name {
            name_is_free(workspaces, name, Some(&id))?;
        }
        let kept = workspaces
            .get_mut(at)
            .ok_or_else(|| refused(say!("workspace-no-such", id = format!("{id:?}"))))?;
        let before = kept.name.clone();
        fields.apply(kept);
        let (added, _) = merge(kept, found)?;
        let renamed_from = (before != kept.name).then_some(before);
        Ok((kept.clone(), renamed_from, added))
    })?;
    Ok(json!({
        "workspace": shown(&changed)?,
        "renamed_from": renamed_from,
        "added": added,
        "scanned": followed,
    }))
}

fn delete(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = id_param(params)?;
    let dir = directory(ctx)?;
    let gone = workspace::changing(dir, |workspaces| {
        let at = position(workspaces, &id)?;
        Ok(workspaces.remove(at))
    })?;
    Ok(json!({ "deleted": shown(&gone)? }))
}

fn reorder(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let ids: Vec<String> = param(params, "ids")?;
    if ids.len() > MAX_WORKSPACES {
        return Err(invalid(
            "ids",
            say!("workspace-at-most", most = MAX_WORKSPACES),
        ));
    }
    let mut wanted: Vec<String> = Vec::with_capacity(ids.len());
    for text in &ids {
        let id = workspace::id(text)
            .map_err(|why| invalid("ids", format!("{:?} {why}", workspace::cut(text))))?;
        if wanted.contains(&id) {
            return Err(invalid(
                "ids",
                say!("workspace-named-twice", id = format!("{id:?}")),
            ));
        }
        wanted.push(id);
    }
    let dir = directory(ctx)?;
    let order = workspace::changing(dir, |workspaces| {
        for id in &wanted {
            position(workspaces, id)?;
        }
        let (mut named, rest): (Vec<Workspace>, Vec<Workspace>) = std::mem::take(workspaces)
            .into_iter()
            .partition(|workspace| wanted.contains(&workspace.id));
        named.sort_by_key(|workspace| {
            wanted
                .iter()
                .position(|id| *id == workspace.id)
                .unwrap_or(usize::MAX)
        });
        named.extend(rest);
        *workspaces = named;
        Ok(workspaces
            .iter()
            .map(|workspace| workspace.id.clone())
            .collect::<Vec<_>>())
    })?;
    Ok(json!({ "order": order }))
}

fn add(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = id_param(params)?;
    let entries = entries(params)?;
    if entries.is_empty() {
        return Err(invalid("paths", say!("workspace-name-at-least-one")));
    }
    let dir = directory(ctx)?;
    let (changed, added, already) = workspace::changing(dir, |workspaces| {
        let at = position(workspaces, &id)?;
        let kept = workspaces
            .get_mut(at)
            .ok_or_else(|| refused(say!("workspace-no-such", id = format!("{id:?}"))))?;
        let (added, already) = merge(kept, entries)?;
        Ok((kept.clone(), added, already))
    })?;
    Ok(json!({ "workspace": shown(&changed)?, "added": added, "already": already }))
}

fn remove(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = id_param(params)?;
    let paths: Vec<String> = param(params, "paths")?;
    if paths.len() > MAX_REPOSITORIES {
        return Err(invalid(
            "paths",
            say!("workspace-at-most-at-a-time", most = MAX_REPOSITORIES),
        ));
    }
    // The paths as they are kept, so that `/a/b/` takes out `/a/b`.
    let wanted = paths
        .iter()
        .map(|path| {
            workspace::full_path(path)
                .map_err(|why| invalid("paths", format!("{:?}: {why}", workspace::cut(path))))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let dir = directory(ctx)?;
    let (changed, removed, not_there) = workspace::changing(dir, |workspaces| {
        let at = position(workspaces, &id)?;
        let kept = workspaces
            .get_mut(at)
            .ok_or_else(|| refused(say!("workspace-no-such", id = format!("{id:?}"))))?;
        let mut removed = Vec::new();
        let mut not_there = Vec::new();
        for path in wanted {
            match kept
                .repositories
                .iter()
                .position(|entry| entry.path == path)
            {
                Some(index) => {
                    kept.repositories.remove(index);
                    removed.push(path);
                }
                None => not_there.push(path),
            }
        }
        Ok((kept.clone(), removed, not_there))
    })?;
    Ok(json!({ "workspace": shown(&changed)?, "removed": removed, "not_there": not_there }))
}

fn locate(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = id_param(params)?;
    let from: String = param(params, "path")?;
    let to: String = param(params, "to")?;
    let from = workspace::full_path(&from).map_err(|why| invalid("path", why))?;
    let to = workspace::full_path(&to).map_err(|why| invalid("to", why))?;
    let target = Path::new(&to);
    if !target.is_dir() || !bitbraid_git::glance::is_repository_folder(target) {
        return Err(refused(say!(
            "workspace-not-a-repository-folder",
            path = to.as_str()
        )));
    }
    let found_url =
        bitbraid_git::glance::remote_url(target).and_then(|url| workspace::remote_url(&url).ok());
    let dir = directory(ctx)?;
    let (changed, same_remote) = workspace::changing(dir, |workspaces| {
        let at = position(workspaces, &id)?;
        let kept = workspaces
            .get_mut(at)
            .ok_or_else(|| refused(say!("workspace-no-such", id = format!("{id:?}"))))?;
        let entry = kept
            .repositories
            .iter_mut()
            .find(|entry| entry.path == from)
            .ok_or_else(|| refused(say!("workspace-no-repository-at", path = from.as_str())))?;
        let same_remote = match (&entry.remote_url, &found_url) {
            (Some(kept_url), Some(found_url)) => Some(kept_url == found_url),
            _ => None,
        };
        if entry.remote_url.is_none() {
            entry.remote_url = found_url.clone();
        }
        crate::workspace_bulk::point_at(kept, &from, &to);
        Ok((kept.clone(), same_remote))
    })?;
    Ok(json!({
        "workspace": shown(&changed)?,
        "from": from,
        "to": to,
        "same_remote": same_remote,
    }))
}

fn defaults(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = id_param(params)?;
    let text: String = param(params, "path")?;
    let path = workspace::full_path(&text).map_err(|why| invalid("path", why))?;
    let apply: bool = param_or(params, "apply_profile", true)?;
    let found = workspace::find(directory(ctx)?, &id)?;
    if !found.repositories.iter().any(|entry| entry.path == path) {
        return Err(invalid(
            "path",
            say!(
                "workspace-not-one-of-its-repositories",
                path = path.as_str()
            ),
        ));
    }
    let profile = match &found.default_profile {
        Some(name) if apply => Some(apply_profile(ctx, &path, name)),
        Some(name) => Some(json!({
            "name": name, "applied": false, "already": null, "error": null,
        })),
        None => None,
    };
    Ok(json!({
        "workspace": found.id,
        "path": path,
        "profile": profile,
        "layout": found.layout,
    }))
}

/// Write the profile `name` to the configuration of the repository at
/// `path` with `profile.use`, in a context of its own, unless the
/// identity in effect there is that profile's already. Says what was done
/// or why it could not be.
fn apply_profile(ctx: &CommandContext, path: &str, name: &str) -> Value {
    let registry = crate::workspace_bulk::registry();
    let mut inner = CommandContext {
        cancel: ctx.cancel.clone(),
        config_dir: ctx.config_dir.clone(),
        extensions: ctx.extensions.clone(),
        ..CommandContext::default()
    };
    let mut run = || -> Result<bool, CommandError> {
        if !bitbraid_git::glance::is_repository_folder(Path::new(path)) {
            return Err(refused(say!("workspace-not-a-repository-there", path)));
        }
        registry.run("repo.open", &mut inner, &json!({ "path": path }))?;
        let listed = registry.run("profile.list", &mut inner, &json!({}))?;
        let key = folded(name);
        let profile = listed
            .get("profiles")
            .and_then(Value::as_array)
            .and_then(|all| {
                all.iter().find(|profile| {
                    profile
                        .get("name")
                        .and_then(Value::as_str)
                        .is_some_and(|other| folded(other) == key)
                })
            })
            .cloned()
            .ok_or_else(|| refused(say!("workspace-no-profile", name = format!("{name:?}"))))?;
        let current = registry.run("profile.current", &mut inner, &json!({}))?;
        if is_in_effect(&profile, current.get("identity").unwrap_or(&Value::Null)) {
            return Ok(false);
        }
        registry.run(
            "profile.use",
            &mut inner,
            &json!({ "name": name, "scope": "repo" }),
        )?;
        Ok(true)
    };
    match run() {
        Ok(applied) => json!({
            "name": name, "applied": applied, "already": !applied, "error": null,
        }),
        Err(err) => json!({
            "name": name, "applied": false, "already": false, "error": err.to_string(),
        }),
    }
}

/// Whether the identity in effect is the profile's: the same name and
/// address, the same signing key (a key without a format is a GPG one, as
/// it is to git), and the same key to connect with.
fn is_in_effect(profile: &Value, identity: &Value) -> bool {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    let same_email = match (text(profile, "user_email"), text(identity, "user_email")) {
        (Some(wanted), Some(found)) => wanted.eq_ignore_ascii_case(&found),
        _ => false,
    };
    let key = text(profile, "signing_key");
    let format = |value: &Value| {
        text(value, "signing_key")
            .map(|_| text(value, "signing_format").unwrap_or_else(|| "openpgp".to_owned()))
    };
    text(profile, "user_name") == text(identity, "user_name")
        && same_email
        && key == text(identity, "signing_key")
        && format(profile) == format(identity)
        && text(profile, "ssh_key") == text(identity, "ssh_key")
}
