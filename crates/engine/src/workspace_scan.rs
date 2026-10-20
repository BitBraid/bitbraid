//! Where a workspace's repositories come from besides single folders: the
//! repositories in a folder, looked for a few levels down, and the
//! folders of a VS Code `.code-workspace` file.
//!
//! A scan never leaves the folder it was given: it follows no symbolic
//! link, does not go into a `.git` or into a repository it found (its
//! submodules and whatever else is inside are the repository's), stops at
//! a depth and a count, and says what it passed over and why. Nothing it
//! looks at is changed.
//!
//! A `.code-workspace` file is JSON with comments and trailing commas, as
//! VS Code writes it. Its `folders` are read for their `path` (relative to
//! the file, or full) or their `uri` when that is a `file://` one; the
//! rest of the file is passed over.
//!
//! Sources: gitrepository-layout(5); VS Code's documentation, "What is a
//! VS Code workspace?"
//! (code.visualstudio.com/docs/editing/workspaces/workspaces, the
//! multi-root workspace file and its `folders`); RFC 8259 (JSON) and
//! RFC 8089 (the `file` URI scheme).

use std::{
    collections::VecDeque,
    fs,
    io::Read as _,
    path::{Component, Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::Serialize;

use crate::workspace;

/// How many levels below the folder a scan looks: its own subfolders are
/// the first.
pub const MAX_DEPTH: usize = 3;
/// As many repositories as one scan finds.
pub const MAX_FOUND: usize = 500;
/// As many folders as one scan looks into, whatever their depth.
pub const MAX_VISITED: usize = 20_000;
/// As many things passed over as are listed; the rest are counted.
pub const MAX_SKIPPED: usize = 200;
/// A `.code-workspace` file larger than this is not read.
pub const MAX_CODE_WORKSPACE: u64 = 1024 * 1024;
/// As many folders of a `.code-workspace` file as are read.
pub const MAX_CODE_FOLDERS: usize = 1000;

/// A repository that was found.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Found {
    /// Its folder, by its full path.
    pub path: String,
    /// Where it is cloned from, as `workspace.add` would keep it.
    pub remote_url: Option<String>,
    /// Whether it is a bare repository.
    pub bare: bool,
}

/// Something that was passed over, and why.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Skipped {
    /// What it is: a path, or what a file said.
    pub path: String,
    /// Why it was passed over.
    pub why: String,
}

/// What a scan or a `.code-workspace` file came to.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Scanned {
    /// The repositories, in the order they were found.
    pub found: Vec<Found>,
    /// What was passed over, at most [`MAX_SKIPPED`].
    pub skipped: Vec<Skipped>,
    /// How many more were passed over than are listed.
    pub more_skipped: usize,
    /// Whether everything there was to look at was looked at: not when a
    /// bound or a stop ended it early.
    pub complete: bool,
}

impl Scanned {
    fn skip(&mut self, path: &Path, why: impl Into<String>) {
        self.skip_text(path.to_string_lossy().into_owned(), why);
    }

    fn skip_text(&mut self, path: String, why: impl Into<String>) {
        if self.skipped.len() < MAX_SKIPPED {
            self.skipped.push(Skipped {
                path,
                why: why.into(),
            });
        } else {
            self.more_skipped = self.more_skipped.saturating_add(1);
        }
    }

    fn find(&mut self, path: &Path) {
        let Some(text) = path.to_str() else {
            self.skip(path, say!("workspace-path-not-text"));
            return;
        };
        let Ok(text) = workspace::full_path(text) else {
            self.skip(path, say!("workspace-skip-not-kept"));
            return;
        };
        if self.found.iter().any(|found| found.path == text) {
            return;
        }
        let remote_url =
            bitbraid_git::glance::remote_url(path).and_then(|url| workspace::remote_url(&url).ok());
        self.found.push(Found {
            path: text,
            remote_url,
            bare: !path.join(".git").exists(),
        });
    }
}

/// The repositories in `folder`, `depth` levels down at most (no more
/// than [`MAX_DEPTH`]), the first `limit` of them (no more than
/// [`MAX_FOUND`]), shallow ones first. `folder` itself may be one; then
/// it is the only one.
pub fn scan(folder: &Path, depth: usize, limit: usize, cancel: &Arc<AtomicBool>) -> Scanned {
    let depth = depth.min(MAX_DEPTH);
    let limit = limit.clamp(1, MAX_FOUND);
    let mut out = Scanned {
        complete: true,
        ..Scanned::default()
    };
    let mut waiting = VecDeque::from([(folder.to_path_buf(), 0usize)]);
    let mut visited = 0usize;
    while let Some((dir, level)) = waiting.pop_front() {
        if cancel.load(Ordering::SeqCst) {
            out.complete = false;
            out.skip(&dir, say!("workspace-skip-stopped"));
            break;
        }
        if bitbraid_git::glance::is_repository_folder(&dir) {
            if out.found.len() >= limit {
                out.complete = false;
                out.skip(&dir, say!("workspace-skip-limit", limit));
                break;
            }
            out.find(&dir);
            continue;
        }
        if level >= depth {
            // The folders at the last level are looked at, not into.
            if has_folders(&dir) {
                out.skip(&dir, say!("workspace-skip-too-deep", depth));
            }
            continue;
        }
        visited = visited.saturating_add(1);
        if visited > MAX_VISITED {
            out.complete = false;
            out.skip(
                &dir,
                say!("workspace-skip-too-many-folders", most = MAX_VISITED),
            );
            break;
        }
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(err) => {
                out.skip(
                    &dir,
                    say!("workspace-cannot-be-read", error = err.to_string()),
                );
                continue;
            }
        };
        let mut inside: Vec<(PathBuf, fs::FileType)> = entries
            .filter_map(Result::ok)
            .filter_map(|entry| Some((entry.path(), entry.file_type().ok()?)))
            .collect();
        inside.sort_by(|a, b| a.0.cmp(&b.0));
        for (path, kind) in inside {
            if path.file_name().is_some_and(|name| name == ".git") {
                continue;
            }
            if kind.is_symlink() {
                if path.is_dir() {
                    out.skip(&path, say!("workspace-skip-link"));
                }
                continue;
            }
            if kind.is_dir() {
                waiting.push_back((path, level.saturating_add(1)));
            }
        }
    }
    out
}

/// Whether `dir` has a folder in it (a link to one is not one).
fn has_folders(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .filter_map(Result::ok)
            .take(10_000)
            .any(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
    })
}

/// JSON as VS Code writes its own files, with `//` and `/* */` comments
/// and commas before a closing bracket, made plain JSON. What is inside
/// strings is left as it is; line breaks are kept, so that a line number
/// of the JSON is one of the file.
pub fn plain_json(text: &str) -> String {
    #[derive(PartialEq)]
    enum In {
        Code,
        Text,
        Line,
        Block,
    }
    let mut out = String::with_capacity(text.len());
    let mut state = In::Code;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match state {
            In::Code => match c {
                '"' => {
                    state = In::Text;
                    out.push(c);
                }
                '/' if chars.peek() == Some(&'/') => {
                    chars.next();
                    state = In::Line;
                    out.push_str("  ");
                }
                '/' if chars.peek() == Some(&'*') => {
                    chars.next();
                    state = In::Block;
                    out.push_str("  ");
                }
                _ => out.push(c),
            },
            In::Text => {
                out.push(c);
                if c == '\\' {
                    if let Some(next) = chars.next() {
                        out.push(next);
                    }
                } else if c == '"' {
                    state = In::Code;
                }
            }
            In::Line => {
                if c == '\n' {
                    state = In::Code;
                    out.push(c);
                } else {
                    out.push(' ');
                }
            }
            In::Block => {
                if c == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    state = In::Code;
                    out.push_str("  ");
                } else if c == '\n' {
                    out.push(c);
                } else {
                    out.push(' ');
                }
            }
        }
    }
    without_trailing_commas(&out)
}

/// `[1, 2,]` and `{"a": 1,}` without the last comma; strings are left as
/// they are.
fn without_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_text = false;
    let mut chars = text.char_indices().peekable();
    while let Some((at, c)) = chars.next() {
        if in_text {
            out.push(c);
            if c == '\\' {
                if let Some((_, next)) = chars.next() {
                    out.push(next);
                }
            } else if c == '"' {
                in_text = false;
            }
            continue;
        }
        match c {
            '"' => {
                in_text = true;
                out.push(c);
            }
            ',' => {
                let rest = text.get(at.saturating_add(1)..).unwrap_or("");
                let next = rest.trim_start().chars().next();
                if !matches!(next, Some('}' | ']')) {
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// A path made plain without looking at the disk: `.` parts gone, and a
/// `..` taking away the part before it (and nothing above the root).
pub fn lexically_plain(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                // At the root, there is nothing above to go to.
                let at_root = out.parent().is_none();
                if !at_root {
                    out.pop();
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// The path of a `file://` address: what is after the host, with `%XX`
/// read as the byte it stands for. None for another kind of address, or
/// one on another host.
fn file_uri_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let path = match rest.strip_prefix("localhost") {
        Some(path) => path,
        None => rest,
    };
    if !path.starts_with('/') {
        return None;
    }
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0usize;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'%' {
            let digits = bytes.get(at.saturating_add(1)..at.saturating_add(3))?;
            let text = std::str::from_utf8(digits).ok()?;
            out.push(u8::from_str_radix(text, 16).ok()?);
            at = at.saturating_add(3);
        } else {
            out.push(byte);
            at = at.saturating_add(1);
        }
    }
    let path = String::from_utf8(out).ok()?;
    // On Windows the drive comes after the slash: `file:///c%3A/x` is
    // `c:/x`.
    if cfg!(windows) {
        let bytes = path.as_bytes();
        let drive = bytes.get(1).is_some_and(u8::is_ascii_alphabetic)
            && bytes.get(2) == Some(&b':')
            && matches!(bytes.get(3), None | Some(b'/'));
        if drive {
            return path.get(1..).map(str::to_owned);
        }
    }
    Some(path)
}

/// The repositories a `.code-workspace` file lists: each of its folders
/// that is a repository's own folder. The others, and whatever the file
/// says that cannot be a folder here, are passed over with why. An error
/// when the file cannot be read or is not such a file.
pub fn code_workspace(file: &Path) -> Result<Scanned, String> {
    let meta = fs::metadata(file)
        .map_err(|err| say!("workspace-cannot-be-read", error = err.to_string()))?;
    if !meta.is_file() {
        return Err(say!("workspace-not-a-file").to_owned());
    }
    if meta.len() > MAX_CODE_WORKSPACE {
        return Err(say!(
            "workspace-larger-than",
            kib = MAX_CODE_WORKSPACE / 1024
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(file)
        .and_then(|opened| {
            opened
                .take(MAX_CODE_WORKSPACE.saturating_add(1))
                .read_to_end(&mut bytes)
        })
        .map_err(|err| say!("workspace-cannot-be-read", error = err.to_string()))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CODE_WORKSPACE {
        return Err(say!(
            "workspace-larger-than",
            kib = MAX_CODE_WORKSPACE / 1024
        ));
    }
    let text = String::from_utf8(bytes).map_err(|_| say!("workspace-not-text").to_owned())?;
    // A byte order mark is allowed before the JSON.
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let base = file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("/"));
    folders_of(&plain_json(text), &base)
}

/// The repositories of the `folders` of a `.code-workspace` file's JSON,
/// its relative paths read from `base`.
pub fn folders_of(json: &str, base: &Path) -> Result<Scanned, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|err| {
        say!(
            "workspace-not-understood-at",
            line = err.line(),
            said = workspace::cut(&err.to_string())
        )
    })?;
    let folders = match value.get("folders") {
        Some(serde_json::Value::Array(folders)) => folders,
        Some(_) => return Err(say!("workspace-code-folders-not-a-list").to_owned()),
        None => return Err(say!("workspace-code-no-folders").to_owned()),
    };
    if folders.len() > MAX_CODE_FOLDERS {
        return Err(say!(
            "workspace-code-too-many-folders",
            count = folders.len(),
            most = MAX_CODE_FOLDERS
        ));
    }
    let mut out = Scanned {
        complete: true,
        ..Scanned::default()
    };
    for (index, folder) in folders.iter().enumerate() {
        let number = index.saturating_add(1);
        let said = |what: &str| format!("{}{what}", say!("workspace-folder-number", number));
        let path = folder.get("path").and_then(serde_json::Value::as_str);
        let uri = folder.get("uri").and_then(serde_json::Value::as_str);
        let text = match (path, uri) {
            (Some(path), _) => path.to_owned(),
            (None, Some(uri)) => match file_uri_path(uri) {
                Some(path) => path,
                None => {
                    out.skip_text(
                        said(&format!(": {}", workspace::cut(uri))),
                        say!("workspace-skip-not-local"),
                    );
                    continue;
                }
            },
            (None, None) => {
                out.skip_text(said(""), say!("workspace-skip-neither"));
                continue;
            }
        };
        if text.is_empty() || text.len() > workspace::MAX_PATH || text.chars().any(char::is_control)
        {
            out.skip_text(
                said(&format!(": {:?}", workspace::cut(&text))),
                say!("workspace-skip-path-unusable"),
            );
            continue;
        }
        let given = Path::new(&text);
        let full = if given.is_absolute() {
            lexically_plain(given)
        } else {
            lexically_plain(&base.join(given))
        };
        if bitbraid_git::glance::is_repository_folder(&full) {
            out.find(&full);
        } else if full.is_dir() {
            out.skip(&full, say!("workspace-skip-not-a-repository"));
        } else {
            out.skip(&full, say!("workspace-skip-not-there"));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comments_and_trailing_commas_are_taken_out() {
        let text = "{\n  // the folders\n  \"folders\": [\n    { \"path\": \"a//b\" }, /* one */\n    \
                    { \"path\": \"c/*d*/\", },\n  ],\n  \"x\": \"q\\\"//,]\",\n}\n";
        let plain = plain_json(text);
        let value: serde_json::Value = serde_json::from_str(&plain).unwrap();
        assert_eq!(value["folders"][0]["path"], "a//b");
        assert_eq!(value["folders"][1]["path"], "c/*d*/");
        assert_eq!(value["x"], "q\"//,]");
        assert_eq!(plain.lines().count(), text.lines().count());
        // What is left open is the end of the text.
        assert_eq!(plain_json("[1 /* open"), format!("[1{}", " ".repeat(8)));
        assert_eq!(plain_json("\"open \\"), "\"open \\");
        assert_eq!(plain_json("[1,\n// c\n]"), "[1\n    \n]");
    }

    #[test]
    fn paths_are_made_plain_without_the_disk() {
        assert_eq!(
            lexically_plain(Path::new("/a/b/../c/./d")),
            Path::new("/a/c/d")
        );
        assert_eq!(lexically_plain(Path::new("/../../x")), Path::new("/x"));
        assert_eq!(lexically_plain(Path::new("/a/b/")), Path::new("/a/b"));
    }

    #[test]
    fn a_file_address_is_a_path_here() {
        assert_eq!(
            file_uri_path("file:///home/a%20b/c").as_deref(),
            Some("/home/a b/c")
        );
        assert_eq!(file_uri_path("file://localhost/x").as_deref(), Some("/x"));
        // How VS Code writes a folder on a drive of Windows.
        let on_drive = file_uri_path("file:///c%3A/work/a%20b");
        if cfg!(windows) {
            assert_eq!(on_drive.as_deref(), Some("c:/work/a b"));
            assert_eq!(file_uri_path("file:///D:").as_deref(), Some("D:"));
        } else {
            assert_eq!(on_drive.as_deref(), Some("/c:/work/a b"));
        }
        for other in [
            "vscode-remote://ssh-remote+host/x",
            "file://elsewhere/x",
            "file:///bad%zz",
            "file:///cut%2",
            "file:///not-text%ff",
            "/plain",
        ] {
            assert_eq!(file_uri_path(other), None, "{other}");
        }
    }

    #[test]
    fn what_is_not_a_code_workspace_says_why() {
        let base = Path::new("/nowhere");
        for (json, why) in [
            ("[]", "no \"folders\""),
            ("{\"folders\": {}}", "not a list"),
            ("{", "line 1"),
            (
                "{\"folders\": [1, {\"path\": 7}, {\"uri\": \"https://x\"}]}",
                "",
            ),
        ] {
            match folders_of(json, base) {
                Ok(scanned) => {
                    assert!(why.is_empty(), "{json}");
                    assert!(scanned.found.is_empty());
                    assert_eq!(scanned.skipped.len(), 3, "{scanned:?}");
                }
                Err(said) => assert!(said.contains(why), "{json}: {said}"),
            }
        }
        let many = format!(
            "{{\"folders\": [{}]}}",
            vec!["{\"path\": \"a\"}"; MAX_CODE_FOLDERS + 1].join(",")
        );
        assert!(
            folders_of(&many, base)
                .unwrap_err()
                .contains("no more than")
        );
    }
}
