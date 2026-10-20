//! A workspace as a file to share: written by
//! `workspace.export`, read by `workspace.import`. It is TOML that can be
//! kept in a repository, so that whoever clones it can make the same
//! workspace: the name, colour, icon, description, default profile and
//! layout, and the repositories by their paths relative to the file's
//! folder (`.` for that folder itself) with the addresses to clone them
//! from. A repository outside that folder is written by its folder's name
//! and address only, and is put in the folder on import.
//!
//! A file to import is someone else's text. It is read with bounds on its
//! size and on everything in it; a repository whose path would lead out
//! of the folder it is imported into (a full path, `..` too far, or a
//! symbolic link that leads elsewhere) is refused and named, and so is
//! one that is not there and has no address; the rest are imported.
//!
//! Sources: the TOML specification (toml.io).

use std::{
    fs,
    io::Read as _,
    path::{Component, Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    command::{CommandContext, CommandError, Output, Params, param, param_or},
    workspace::{
        self, Layout, MAX_NAME, MAX_REPOSITORIES, MAX_WORKSPACES, Workspace, WorkspaceRepository,
        refused,
    },
    workspace_cmds::{checked_entry, directory, id_param, name_is_free, shown},
    workspace_scan::lexically_plain,
};

/// What the file says it is.
pub const FORMAT: &str = "bitbraid-workspace";
/// The version of the file this build writes and reads.
pub const VERSION: u32 = 1;
/// A file larger than this is not read.
pub const MAX_SHARED: u64 = 1024 * 1024;

/// The file.
#[derive(Debug, Serialize, Deserialize)]
struct Shared {
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    version: Option<u32>,
    name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    colour: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    default_profile: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    layout: Option<Layout>,
    #[serde(default)]
    repositories: Vec<SharedRepository>,
}

/// One repository in the file: by its path relative to the file's
/// folder, or by the name of its folder.
#[derive(Debug, Serialize, Deserialize)]
struct SharedRepository {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    remote_url: Option<String>,
}

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

/// `path` relative to `folder`, with `/` between its parts; `.` for the
/// folder itself; none when it is not inside it.
fn relative(path: &Path, folder: &Path) -> Option<String> {
    let rest = path.strip_prefix(folder).ok()?;
    let mut parts = Vec::new();
    for component in rest.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_str()?),
            _ => return None,
        }
    }
    if parts.is_empty() {
        return Some(".".to_owned());
    }
    Some(parts.join("/"))
}

pub(crate) fn export(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id = id_param(params)?;
    let text: String = param(params, "file")?;
    let file = workspace::full_path(&text).map_err(|why| invalid("file", why))?;
    let overwrite: bool = param_or(params, "overwrite", false)?;
    let found = workspace::find(directory(ctx)?, &id)?;
    let file_path = Path::new(&file);
    let folder = file_path
        .parent()
        .ok_or_else(|| invalid("file", say!("workspace-file-in-a-folder")))?;
    if !folder.is_dir() {
        return Err(refused(say!(
            "workspace-folder-missing",
            folder = folder.display().to_string()
        )));
    }
    if file_path.is_dir() {
        return Err(refused(say!("workspace-is-a-folder", file = file.as_str())));
    }
    if file_path.exists() && !overwrite {
        return Err(refused(say!("workspace-file-there", file = file.as_str())));
    }
    let mut by_name = Vec::new();
    let repositories: Vec<SharedRepository> = found
        .repositories
        .iter()
        .map(|entry| match relative(Path::new(&entry.path), folder) {
            Some(path) => SharedRepository {
                path: Some(path),
                name: None,
                remote_url: entry.remote_url.clone(),
            },
            None => {
                by_name.push(entry.path.clone());
                SharedRepository {
                    path: None,
                    name: Some(entry.name()),
                    remote_url: entry.remote_url.clone(),
                }
            }
        })
        .collect();
    let count = repositories.len();
    let shared = Shared {
        format: Some(FORMAT.to_owned()),
        version: Some(VERSION),
        name: found.name.clone(),
        colour: found.colour.clone(),
        icon: found.icon.clone(),
        description: found.description.clone(),
        default_profile: found.default_profile.clone(),
        layout: found.layout.clone(),
        repositories,
    };
    let body = toml::to_string_pretty(&shared).map_err(|err| {
        refused(say!(
            "workspace-one-not-written-down",
            error = err.to_string()
        ))
    })?;
    let text = format!(
        "# A workspace of BitBraid. Import it with workspace.import; the paths\n\
         # of its repositories are relative to the folder of this file.\n{body}"
    );
    workspace::write_whole(file_path, &text).map_err(|why| refused(format!("{file} {why}")))?;
    Ok(json!({
        "file": file,
        "repositories": count,
        "by_path": count.saturating_sub(by_name.len()),
        "by_name": by_name,
    }))
}

/// Read the file, or say why it cannot be read.
fn read_shared(file: &Path) -> Result<Shared, String> {
    let meta = fs::metadata(file)
        .map_err(|err| say!("workspace-cannot-be-read", error = err.to_string()))?;
    if !meta.is_file() {
        return Err(say!("workspace-not-a-file").to_owned());
    }
    let too_large = || say!("workspace-larger-than", kib = MAX_SHARED / 1024);
    if meta.len() > MAX_SHARED {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    fs::File::open(file)
        .and_then(|opened| {
            opened
                .take(MAX_SHARED.saturating_add(1))
                .read_to_end(&mut bytes)
        })
        .map_err(|err| say!("workspace-cannot-be-read", error = err.to_string()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SHARED {
        return Err(too_large());
    }
    let text = String::from_utf8(bytes).map_err(|_| say!("workspace-not-text").to_owned())?;
    let shared: Shared = toml::from_str(&text).map_err(|err| {
        let line = err
            .span()
            .and_then(|span| text.get(..span.start))
            .map(|before| before.matches('\n').count().saturating_add(1));
        let said = workspace::cut(err.message());
        match line {
            Some(line) => say!("workspace-not-understood-at", line, said),
            None => say!("workspace-not-understood", said),
        }
    })?;
    if shared.format.as_deref() != Some(FORMAT) {
        return Err(say!("workspace-not-a-workspace-file", format = FORMAT));
    }
    if shared.version.is_some_and(|version| version > VERSION) {
        return Err(say!("workspace-written-by-newer").to_owned());
    }
    if shared.repositories.len() > MAX_REPOSITORIES {
        return Err(say!(
            "workspace-file-too-many-repositories",
            most = MAX_REPOSITORIES
        ));
    }
    Ok(shared)
}

/// The longest part of `path` that is there, as the file system has it
/// (links followed).
fn real_ancestor(path: &Path) -> Option<PathBuf> {
    let mut at = Some(path);
    while let Some(here) = at {
        if let Ok(real) = bitbraid_core::paths::canonical(here) {
            return Some(real);
        }
        at = here.parent();
    }
    None
}

/// Where a repository of the file goes under `folder` (whose real path is
/// `real_folder`), or why it is refused.
fn place(entry: &SharedRepository, folder: &Path, real_folder: &Path) -> Result<String, String> {
    let given = match (&entry.path, &entry.name) {
        (Some(path), _) => {
            if path.is_empty()
                || path.len() > workspace::MAX_PATH
                || path.chars().any(char::is_control)
            {
                return Err(say!("workspace-place-path-unusable").to_owned());
            }
            let relative = Path::new(path);
            if relative.has_root()
                || relative
                    .components()
                    .any(|part| matches!(part, Component::Prefix(_) | Component::RootDir))
            {
                return Err(say!(
                    "workspace-place-full-path",
                    folder = folder.display().to_string()
                ));
            }
            lexically_plain(&folder.join(relative))
        }
        (None, Some(name)) => {
            let mut parts = Path::new(name).components();
            let single = matches!(
                (parts.next(), parts.next()),
                (Some(Component::Normal(_)), None)
            );
            if !single || name.chars().any(char::is_control) || name.contains(['/', '\\']) {
                return Err(say!("workspace-place-name-unusable").to_owned());
            }
            folder.join(name)
        }
        (None, None) => return Err(say!("workspace-place-neither").to_owned()),
    };
    if !given.starts_with(folder) {
        return Err(say!(
            "workspace-place-leads-out",
            folder = folder.display().to_string()
        ));
    }
    let real =
        real_ancestor(&given).ok_or_else(|| say!("workspace-place-not-followed").to_owned())?;
    if !real.starts_with(real_folder) {
        return Err(say!(
            "workspace-place-through-link",
            folder = folder.display().to_string()
        ));
    }
    let text = given
        .to_str()
        .ok_or_else(|| say!("workspace-path-not-text").to_owned())?;
    workspace::full_path(text)
}

pub(crate) fn import(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let text: String = param(params, "file")?;
    let file = workspace::full_path(&text).map_err(|why| invalid("file", why))?;
    let folder: Option<String> = param_or(params, "folder", None)?;
    let folder = match folder {
        Some(text) => workspace::full_path(&text).map_err(|why| invalid("folder", why))?,
        None => Path::new(&file)
            .parent()
            .and_then(Path::to_str)
            .map(str::to_owned)
            .ok_or_else(|| invalid("file", say!("workspace-file-in-a-folder")))?,
    };
    let name: Option<String> = param_or(params, "name", None)?;
    let name = name
        .map(|text| workspace::short_text(&text, MAX_NAME).map_err(|why| invalid("name", why)))
        .transpose()?;
    let dir = directory(ctx)?.to_path_buf();
    let folder_path = PathBuf::from(&folder);
    if !folder_path.is_dir() {
        return Err(refused(say!(
            "workspace-folder-missing",
            folder = folder.as_str()
        )));
    }
    let real_folder = bitbraid_core::paths::canonical(&folder_path).map_err(|err| {
        refused(say!(
            "workspace-folder-not-followed",
            folder = folder.as_str(),
            error = err.to_string()
        ))
    })?;
    let shared = read_shared(Path::new(&file)).map_err(|why| refused(format!("{file} {why}")))?;

    // The workspace's own fields, checked as any workspace's are.
    let unchecked = Workspace {
        id: "imported".to_owned(),
        name: name.unwrap_or(shared.name),
        colour: shared.colour,
        icon: shared.icon,
        description: shared.description,
        default_profile: shared.default_profile,
        follows_folder: None,
        layout: shared.layout,
        repositories: Vec::new(),
    };
    let mut made = workspace::tidy(unchecked).map_err(|why| refused(format!("{file}: {why}")))?;

    let mut refused_entries = Vec::new();
    for (index, entry) in shared.repositories.iter().enumerate() {
        let said = entry
            .path
            .as_deref()
            .or(entry.name.as_deref())
            .map(workspace::cut)
            .unwrap_or_else(|| say!("workspace-entry-number", number = index.saturating_add(1)));
        let placed = place(entry, &folder_path, &real_folder).and_then(|path| {
            let remote_url = entry
                .remote_url
                .as_deref()
                .filter(|url| !url.trim().is_empty())
                .map(|url| {
                    workspace::remote_url(url).map_err(|why| say!("workspace-its-remote-url", why))
                })
                .transpose()?;
            checked_entry(WorkspaceRepository { path, remote_url })
        });
        match placed {
            Ok(checked) => {
                if !made
                    .repositories
                    .iter()
                    .any(|kept| kept.path == checked.path)
                {
                    made.repositories.push(checked);
                }
            }
            Err(why) => refused_entries.push(json!({ "repository": said, "why": why })),
        }
    }
    let added: Vec<String> = made
        .repositories
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    let made = workspace::changing(&dir, |workspaces| {
        if workspaces.len() >= MAX_WORKSPACES {
            return Err(refused(say!(
                "workspace-too-many-workspaces",
                most = MAX_WORKSPACES
            )));
        }
        name_is_free(workspaces, &made.name, None)?;
        made.id = workspace::new_id(&made.name, workspaces);
        workspaces.push(made.clone());
        Ok(made)
    })?;
    Ok(json!({
        "workspace": shown(&made)?,
        "added": added,
        "refused": refused_entries,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_inside_the_folder_is_written_relative_to_it() {
        let folder = Path::new("/src/team");
        assert_eq!(
            relative(Path::new("/src/team/a/b"), folder).as_deref(),
            Some("a/b")
        );
        assert_eq!(relative(folder, folder).as_deref(), Some("."));
        assert_eq!(relative(Path::new("/src/other"), folder), None);
        assert_eq!(relative(Path::new("/src/teamwork"), folder), None);
    }

    #[test]
    fn a_place_never_leads_out_of_the_folder() {
        let folder = Path::new("/nowhere/team");
        let real = Path::new("/nowhere/team");
        let entry = |path: Option<&str>, name: Option<&str>| SharedRepository {
            path: path.map(str::to_owned),
            name: name.map(str::to_owned),
            remote_url: None,
        };
        for (path, name, why) in [
            (Some("/etc"), None, "full path"),
            (Some("../other"), None, "leads out"),
            (Some("a/../../other"), None, "leads out"),
            (Some(""), None, "cannot be used"),
            (Some("a\nb"), None, "cannot be used"),
            (None, Some(".."), "not a folder's name"),
            (None, Some("a/b"), "not a folder's name"),
            (None, Some("."), "not a folder's name"),
            (None, Some(""), "not a folder's name"),
            (None, None, "neither"),
        ] {
            let said = place(&entry(path, name), folder, real).unwrap_err();
            assert!(said.contains(why), "{path:?} {name:?}: {said}");
        }
    }
}
