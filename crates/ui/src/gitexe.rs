//! Which git the application runs: what the preferences chose, and what
//! that executable said about itself when it was asked.
//!
//! The choice is handed to the engine before any repository is opened;
//! asking for the version starts a process, so that happens once the window
//! is up and never holds it back.

use gpui_kit::{App, Global};
use serde_json::{Value, json};

use crate::prefs::Prefs;

/// What is known about the git in use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitExecutable {
    /// Whether it has been asked yet.
    pub asked: bool,
    /// Where it is, when it was found.
    pub path: Option<String>,
    /// Its version, as `2.56.0`.
    pub version: Option<String>,
    /// Whether everything works with that version.
    pub supported: bool,
    /// The oldest version everything works with.
    pub minimum: String,
    /// What this version does without.
    pub lacks: Vec<String>,
    /// Why it could not be asked: there is no such program, or it is not git.
    pub error: Option<String>,
    /// Whether it is the one shipped with the application, run because
    /// `PATH` has none (or chosen by its path).
    pub shipped: bool,
}

impl Global for GitExecutable {}

impl GitExecutable {
    /// Read what `git.info` (or the `info` of `git.use`) answered.
    pub fn from_json(value: &Value) -> Self {
        let number = |version: &Value| {
            let part = |key: &str| version.get(key).and_then(Value::as_u64).unwrap_or(0);
            format!("{}.{}.{}", part("major"), part("minor"), part("patch"))
        };
        Self {
            asked: true,
            path: value.get("path").and_then(Value::as_str).map(str::to_owned),
            version: value.get("version").filter(|v| v.is_object()).map(number),
            supported: value.get("supported").and_then(Value::as_bool) == Some(true),
            minimum: value
                .get("minimum")
                .filter(|v| v.is_object())
                .map(number)
                .unwrap_or_default(),
            lacks: value
                .get("lacks")
                .and_then(Value::as_array)
                .map(|lacks| {
                    lacks
                        .iter()
                        .filter_map(|what| what.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default(),
            error: None,
            shipped: value.get("shipped").and_then(Value::as_bool) == Some(true),
        }
    }

    /// The executable could not be asked.
    pub fn failed(error: String) -> Self {
        Self {
            asked: true,
            error: Some(error),
            ..Self::default()
        }
    }

    /// What to warn about, if anything: no git, or one that is too old.
    pub fn warning(&self) -> Option<String> {
        if !self.asked {
            return None;
        }
        if let Some(error) = &self.error {
            return Some(say!("app-git-cannot-run-warning", error));
        }
        if self.supported && self.lacks.is_empty() {
            return None;
        }
        let version = self
            .version
            .as_deref()
            .unwrap_or(say!("app-git-version-unknown"));
        let lacks = self.lacks.join("; ");
        Some(if self.supported {
            say!("app-git-does-without", version, lacks)
        } else {
            say!("app-git-too-old", version, minimum = &self.minimum, lacks)
        })
    }

    /// A few words for the status bar.
    pub fn short(&self) -> Option<String> {
        match (&self.version, &self.error) {
            (Some(version), _) if self.shipped => Some(say!("app-git-short-shipped", version)),
            (Some(version), _) => Some(say!("app-git-short", version)),
            (None, Some(_)) => Some(say!("app-git-short-none").to_owned()),
            (None, None) => None,
        }
    }

    /// Which git is in use, in a line for the preferences.
    pub fn in_use(&self) -> String {
        match (&self.version, &self.path, &self.error) {
            (Some(version), Some(path), _) if self.shipped => {
                say!("app-git-in-use-shipped-at", version, path)
            }
            (Some(version), None, _) if self.shipped => say!("app-git-in-use-shipped", version),
            (Some(version), Some(path), _) => say!("app-git-in-use-at", version, path),
            (Some(version), None, _) => say!("app-git-in-use", version),
            (None, _, Some(error)) => say!("app-git-cannot-run", error),
            (None, _, None) => say!("app-git-asking").to_owned(),
        }
    }
}

/// Hand the executable the preferences name to the engine, as it is: it is
/// asked what it is later, once there is a window.
pub fn init(cx: &mut App) {
    cx.set_global(GitExecutable::default());
    let chosen = cx.global::<Prefs>().preferences.git_executable.clone();
    if let Some(program) = chosen {
        let registry = bitbraid_integrations::registry();
        let mut ctx = bitbraid_engine::CommandContext::default();
        let _ = registry.run(
            "git.use",
            &mut ctx,
            &json!({ "program": program, "verify": false }),
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::GitExecutable;

    #[test]
    fn what_the_engine_answers_is_read_and_warned_about() {
        let current = GitExecutable::from_json(&json!({
            "program": "git", "path": "/usr/bin/git", "chosen": false,
            "version_text": "git version 2.56.0",
            "version": { "major": 2, "minor": 56, "patch": 0 },
            "supported": true,
            "minimum": { "major": 2, "minor": 38, "patch": 0 },
            "lacks": [],
        }));
        assert_eq!(current.version.as_deref(), Some("2.56.0"));
        assert_eq!(current.short().as_deref(), Some("git 2.56.0"));
        assert_eq!(current.warning(), None);
        assert!(!current.shipped);
        assert_eq!(current.in_use(), "In use: git 2.56.0 at /usr/bin/git");

        // The git shipped with the application, run where PATH has none.
        let shipped = GitExecutable::from_json(&json!({
            "program": "/opt/bitbraid/libexec/bitbraid/git/bin/git",
            "path": "/opt/bitbraid/libexec/bitbraid/git/bin/git",
            "chosen": false, "shipped": true,
            "version": { "major": 2, "minor": 56, "patch": 0 },
            "supported": true,
            "minimum": { "major": 2, "minor": 38, "patch": 0 },
            "lacks": [],
        }));
        assert!(shipped.shipped);
        assert_eq!(shipped.short().as_deref(), Some("git 2.56.0 (shipped)"));
        assert_eq!(
            shipped.in_use(),
            "In use: git 2.56.0, shipped with BitBraid, at \
             /opt/bitbraid/libexec/bitbraid/git/bin/git"
        );
        assert_eq!(shipped.warning(), None);
        let unplaced = GitExecutable {
            path: None,
            ..shipped.clone()
        };
        assert_eq!(
            unplaced.in_use(),
            "In use: git 2.56.0, shipped with BitBraid"
        );
        assert_eq!(
            GitExecutable::default().in_use(),
            "Asking git what version it is…"
        );
        assert!(
            GitExecutable::failed("not found".to_owned())
                .in_use()
                .starts_with("Git cannot be run: not found")
        );

        let old = GitExecutable::from_json(&json!({
            "path": "/opt/old/git",
            "version": { "major": 2, "minor": 30, "patch": 1 },
            "supported": false,
            "minimum": { "major": 2, "minor": 38, "patch": 0 },
            "lacks": ["listing worktrees", "something else"],
        }));
        let warning = old.warning().unwrap();
        assert!(warning.contains("2.30.1") && warning.contains("older than 2.38.0"));
        assert!(warning.contains("listing worktrees; something else"));

        let nearly = GitExecutable::from_json(&json!({
            "version": { "major": 2, "minor": 40, "patch": 0 },
            "supported": true,
            "lacks": ["one thing"],
        }));
        assert_eq!(
            nearly.warning().as_deref(),
            Some("Git 2.40.0 does without: one thing.")
        );

        let none = GitExecutable::failed("cannot run git: not found".to_owned());
        assert!(none.warning().unwrap().starts_with("Git cannot be run"));
        assert_eq!(none.short().as_deref(), Some("no git"));
        // Before it was asked there is nothing to say.
        assert_eq!(GitExecutable::default().warning(), None);
        assert_eq!(GitExecutable::default().short(), None);
        // Nonsense is read as nothing known, not as a crash.
        let odd = GitExecutable::from_json(&json!(["x"]));
        assert_eq!(odd.version, None);
        assert!(!odd.supported);
    }
}
