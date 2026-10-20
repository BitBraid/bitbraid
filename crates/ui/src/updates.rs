//! The window's side of the updater: what it knows of this copy and of the
//! newest release, and what the preferences and the notices say about them.
//!
//! The updater is the integrations crate's (`bitbraid_integrations::update`),
//! reached only through its commands, which the shell runs
//! (`shell/updates.rs`): `update.status` when the window opens, which asks
//! nobody; `update.check` at the start and once a day, and when the user
//! asks, but only while updates are turned on in the preferences (they are
//! off until the user turns them on); and `update.install` when the user
//! asks for it.
//!
//! Sources: none (original design).

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use gpui_kit::{App, Global};
use serde_json::Value;

/// How often a look is taken while the window is open and updates are on.
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Longest text taken from an answer.
const MOST: usize = 600;

/// What the user asks of the updater from the preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Look for a newer version now.
    Look,
    /// Install the newer version that was found.
    Install,
    /// Stop the download.
    Stop,
    /// Start the version that was installed, in place of this one.
    Restart,
    /// Open the newest release's page.
    Page,
}

/// What this copy is as far as updates go (`update.status`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    /// The version that runs.
    pub current: String,
    /// Whether the build has an update key.
    pub key: bool,
    /// Why it has none.
    pub why_no_key: Option<String>,
    /// How this copy was installed: `appimage`, `bundle`, `installer`,
    /// `package` or `elsewhere`.
    pub installation: String,
    /// The file, folder or program that is this copy.
    pub path: String,
    /// Whether it installs a newer version itself.
    pub installs_itself: bool,
    /// Why not, when it does not.
    pub why_not: Option<String>,
}

/// What a look at the feed found (`update.check`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Look {
    /// The version that runs.
    pub current: String,
    /// The newest version.
    pub latest: String,
    /// Whether it is newer.
    pub newer: bool,
    /// The release's page.
    pub page: String,
    /// Whether this copy installs it itself.
    pub installs: bool,
    /// Why not, when it does not.
    pub why_not: Option<String>,
}

/// What failed last, and why, as a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Failed {
    /// A look at the feed.
    Look(String),
    /// An installation, which says what of it failed.
    Install(String),
}

/// A newer version that was installed, and what comes next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installed {
    /// In place: starting again starts it.
    Restart(String),
    /// Its installer runs, and the window closes for it.
    Quit(String),
}

/// What the window knows of updates.
#[derive(Debug, Clone, Default)]
pub struct Updates {
    /// What this copy is, once it was asked.
    pub status: Option<Status>,
    /// A look is under way.
    pub looking: bool,
    /// What the last look found.
    pub found: Option<Look>,
    /// When the last look ended, in Unix seconds.
    pub looked_at: Option<i64>,
    /// Whether the last look or installation failed, and why.
    pub failed: Option<Failed>,
    /// How much of the package came, in percent, while it downloads.
    pub installing: Option<u8>,
    /// What stops the download.
    pub cancel: Option<Arc<AtomicBool>>,
    /// What was installed.
    pub installed: Option<Installed>,
    /// The newest version a notice was shown for in this run: a look a day
    /// later that finds the same says nothing again.
    pub told: Option<String>,
    /// Whether updates were on the last time the preferences changed: a
    /// look is taken as soon as they are turned on.
    pub on: bool,
}

impl Global for Updates {}

/// Set up what the window knows of updates: nothing yet.
pub fn init(cx: &mut App) {
    let on = cx
        .try_global::<crate::prefs::Prefs>()
        .is_some_and(|prefs| prefs.preferences.updates);
    cx.set_global(Updates {
        on,
        ..Updates::default()
    });
}

/// A text of an answer, made fit to show: one line, bounded.
fn text(value: &Value, key: &str) -> String {
    let text = value.get(key).and_then(Value::as_str).unwrap_or_default();
    text.chars()
        .take(MOST)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .trim()
        .to_owned()
}

fn maybe(value: &Value, key: &str) -> Option<String> {
    Some(text(value, key)).filter(|text| !text.is_empty())
}

fn flag(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool) == Some(true)
}

impl Status {
    /// From what `update.status` answered.
    pub fn from_json(value: &Value) -> Self {
        Self {
            current: text(value, "current"),
            key: flag(value, "key"),
            why_no_key: maybe(value, "why_no_key"),
            installation: text(value, "installation"),
            path: text(value, "path"),
            installs_itself: flag(value, "installs_itself"),
            why_not: maybe(value, "why_not"),
        }
    }
}

impl Look {
    /// From what `update.check` answered; none when it names no version.
    pub fn from_json(value: &Value) -> Option<Self> {
        let latest = maybe(value, "latest")?;
        Some(Self {
            current: text(value, "current"),
            latest,
            newer: flag(value, "newer"),
            page: text(value, "page"),
            installs: flag(value, "installs"),
            why_not: maybe(value, "why_not"),
        })
    }
}

/// An error of an updater's command as a sentence: without the area's
/// word in front, with a capital and a full stop.
pub fn said(error: &str) -> String {
    let error = error.trim();
    let bare = error.strip_prefix("update: ").unwrap_or(error);
    let bare: String = bare.chars().take(MOST).collect();
    let mut sentence = crate::shell::sentence(bare.trim_end_matches('.'));
    sentence.push('.');
    sentence
}

/// A clause as a sentence of its own.
fn whole(clause: &str) -> String {
    let mut sentence = crate::shell::sentence(clause.trim().trim_end_matches('.'));
    sentence.push('.');
    sentence
}

/// What the switch says under its name: what is sent when it is on, and
/// to whom.
pub fn switch_note() -> &'static str {
    say!("updates-switch-note")
}

/// What the row about this copy says: how it was installed and what the
/// updater does with it, then what the last look found.
pub fn copy_text(updates: &Updates, on: bool, now: i64) -> String {
    let Some(status) = &updates.status else {
        return say!("updates-copy-reading").to_owned();
    };
    if !status.key {
        return status
            .why_no_key
            .as_deref()
            .map(whole)
            .unwrap_or_else(|| say!("updates-copy-no-key").to_owned());
    }
    let copy = if status.installs_itself {
        match status.installation.as_str() {
            "appimage" => say!("updates-copy-appimage", path = &status.path),
            "bundle" => say!("updates-copy-bundle", path = &status.path),
            _ => say!("updates-copy-windows-installer").to_owned(),
        }
    } else {
        status
            .why_not
            .as_deref()
            .map(whole)
            .unwrap_or_else(|| say!("updates-copy-not-itself").to_owned())
    };
    let latest = || {
        updates
            .found
            .as_ref()
            .map(|look| look.latest.clone())
            .unwrap_or_default()
    };
    let last = if let Some(percent) = updates.installing {
        say!("updates-downloading", version = latest(), percent)
    } else if let Some(installed) = &updates.installed {
        match installed {
            Installed::Restart(version) => say!("updates-installed-restart", version),
            Installed::Quit(version) => say!("updates-installer-runs", version),
        }
    } else if updates.looking {
        say!("updates-looking").to_owned()
    } else if let Some(failed) = &updates.failed {
        match failed {
            Failed::Look(failed) => say!("updates-last-look-failed", failed),
            Failed::Install(failed) => failed.clone(),
        }
    } else if let Some(look) = &updates.found {
        let when = updates
            .looked_at
            .map(|at| {
                let when = crate::views::relative_time(at, now);
                format!(" {}", say!("updates-looked", when))
            })
            .unwrap_or_default();
        if look.newer {
            format!("{}{when}", newer_out(look))
        } else {
            let newest = say!("updates-newest", current = &look.current);
            format!("{newest}{when}")
        }
    } else if on {
        say!("updates-not-looked-yet").to_owned()
    } else {
        say!("updates-off-nothing-asked").to_owned()
    };
    format!("{copy} {last}")
}

/// That a newer version is out, and which one this is.
fn newer_out(look: &Look) -> String {
    say!(
        "updates-newer-out",
        latest = &look.latest,
        current = &look.current
    )
}

/// What a notice says of a newer version that was found.
pub fn found_text(look: &Look) -> String {
    let out = newer_out(look);
    match (&look.why_not, look.installs) {
        (Some(why), false) => format!("{out} {}", whole(why)),
        _ => out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn status(installation: &str, installs_itself: bool) -> Status {
        Status {
            current: "1.0.0".to_owned(),
            key: true,
            why_no_key: None,
            installation: installation.to_owned(),
            path: "/home/u/Apps/BitBraid.AppImage".to_owned(),
            installs_itself,
            why_not: (!installs_itself)
                .then(|| "BitBraid was installed by the system's package manager".to_owned()),
        }
    }

    #[test]
    fn answers_are_read_and_bounded() {
        let read = Status::from_json(&json!({
            "current": "1.0.0", "key": true, "installation": "appimage",
            "path": "/a/b\nc", "installs_itself": true, "why_not": null,
        }));
        assert_eq!(read.path, "/a/b c");
        assert!(read.key && read.installs_itself);
        assert_eq!(read.why_not, None);
        assert_eq!(Status::from_json(&json!([])), Status::default());
        assert_eq!(Look::from_json(&json!({"newer": true})), None);
        let look = Look::from_json(&json!({
            "current": "1.0.0", "latest": "x".repeat(5000), "newer": true, "installs": "yes",
        }))
        .unwrap();
        assert_eq!(look.latest.chars().count(), MOST);
        assert!(look.newer && !look.installs);
    }

    #[test]
    fn errors_are_said_as_sentences() {
        assert_eq!(
            said("update: this build of BitBraid has no update key"),
            "This build of BitBraid has no update key."
        );
        assert_eq!(said("stopped."), "Stopped.");
    }

    #[test]
    fn the_row_says_what_the_copy_is_and_what_was_found() {
        let mut updates = Updates::default();
        assert!(copy_text(&updates, false, 0).starts_with("Reading"));
        updates.status = Some(Status {
            key: false,
            why_no_key: Some("this build of BitBraid has no update key".to_owned()),
            ..status("appimage", true)
        });
        assert_eq!(
            copy_text(&updates, true, 0),
            "This build of BitBraid has no update key."
        );
        updates.status = Some(status("appimage", true));
        let off = copy_text(&updates, false, 0);
        assert!(
            off.contains("the AppImage /home/u/Apps/BitBraid.AppImage"),
            "{off}"
        );
        assert!(off.ends_with("Updates are off: nothing is asked."), "{off}");
        updates.found = Some(Look {
            current: "1.0.0".to_owned(),
            latest: "1.2.0".to_owned(),
            newer: true,
            page: "https://example.org".to_owned(),
            installs: true,
            why_not: None,
        });
        updates.looked_at = Some(100);
        assert!(
            copy_text(&updates, true, 130)
                .ends_with("BitBraid 1.2.0 is out; this is 1.0.0. Looked just now.")
        );
        updates.installing = Some(42);
        assert!(copy_text(&updates, true, 130).ends_with("Downloading BitBraid 1.2.0: 42 %."));
        updates.installing = None;
        // A look that failed says so; an installation that failed says what
        // of it did, and not that the look failed.
        updates.failed = Some(Failed::Look(said("update: the feed could not be read")));
        assert!(
            copy_text(&updates, true, 130)
                .ends_with("The last look failed. The feed could not be read.")
        );
        let refused = said(
            "update: the download of BitBraid 1.2.0 was not installed: its SHA-256 is not the one the feed gives",
        );
        updates.failed = Some(Failed::Install(refused));
        let row = copy_text(&updates, true, 130);
        assert!(
            row.ends_with(
                "kept until the new one starts. The download of BitBraid 1.2.0 was not installed: \
                 its SHA-256 is not the one the feed gives."
            ),
            "{row}"
        );
        updates.failed = None;
        updates.installed = Some(Installed::Restart("1.2.0".to_owned()));
        assert!(copy_text(&updates, true, 130).ends_with("restart to use it."));
        updates.status = Some(status("package", false));
        assert!(
            copy_text(&updates, true, 130)
                .starts_with("BitBraid was installed by the system's package manager.")
        );
    }

    #[test]
    fn a_notice_of_a_version_says_why_it_is_not_installed_here() {
        let look = Look {
            current: "1.0.0".to_owned(),
            latest: "1.2.0".to_owned(),
            newer: true,
            page: String::new(),
            installs: false,
            why_not: Some("install it with the package manager".to_owned()),
        };
        assert_eq!(
            found_text(&look),
            "BitBraid 1.2.0 is out; this is 1.0.0. Install it with the package manager."
        );
        let here = Look {
            installs: true,
            why_not: None,
            ..look
        };
        assert_eq!(found_text(&here), "BitBraid 1.2.0 is out; this is 1.0.0.");
        // A name with capitals of its own keeps them at the start of a
        // sentence ("macOS", not "MacOS").
        let translocated = Look {
            installs: false,
            why_not: Some("macOS runs this copy from a place of its own".to_owned()),
            ..here
        };
        assert_eq!(
            found_text(&translocated),
            "BitBraid 1.2.0 is out; this is 1.0.0. macOS runs this copy from a place of its own."
        );
    }
}
