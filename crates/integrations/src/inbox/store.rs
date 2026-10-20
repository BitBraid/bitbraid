//! The inbox's pins and snoozes, kept in `inbox.toml` in the
//! configuration directory: which rows the user pinned to the top, and
//! which they put out of sight and until when. They are the user's own
//! and never sent anywhere.
//!
//! ```toml
//! version = 1
//!
//! [[pin]]
//! key = "github:github.com:octo/pull/octo/tools#7"
//! since = 1792013400
//!
//! [[snooze]]
//! key = "github:github.com:octo/issue/octo/tools#9"
//! since = 1792013400
//! until = 1792017000
//! ```
//!
//! The file is anyone's to edit, so it is read with bounds: at most
//! [`MAX_FILE`] bytes, [`MAX_MARKS`] pins and as many snoozes, every key
//! as [`super::usable_key`] allows; an entry that breaks them is passed
//! over, keys it does not know are passed over too. A file that is not
//! TOML at all, or one a newer version wrote, is never written over: a
//! change is refused and says why, and the rows are shown without marks.
//! A snooze that has ended is gone the next time the file is written.
//! The file is written beside itself and moved into place, so that it is
//! whole or as it was.
//!
//! Sources: toml.io v1.0.0 (arrays of tables).

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use super::{Snoozed, usable_key};

/// The file's name in the configuration directory.
pub const FILE: &str = "inbox.toml";
/// The version this build writes.
pub const VERSION: u32 = 1;
/// Largest file read, in bytes.
pub const MAX_FILE: u64 = 1024 * 1024;
/// Most pins kept, and most snoozes.
pub const MAX_MARKS: usize = 2000;

/// A pinned row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pin {
    /// The row's key ([`super::key`]).
    pub key: String,
    /// When it was pinned, in seconds since the Unix epoch.
    #[serde(default)]
    pub since: i64,
}

/// A snoozed row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snooze {
    /// The row's key.
    pub key: String,
    /// When it was snoozed.
    #[serde(default)]
    pub since: i64,
    /// When it comes back by itself; never, when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
}

impl Snooze {
    /// As a row shows it.
    pub fn snoozed(&self) -> Snoozed {
        Snoozed {
            since: self.since,
            until: self.until,
        }
    }
}

/// What the file holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Marks {
    /// The version that wrote it.
    #[serde(default = "version")]
    pub version: u32,
    /// The pins, oldest first.
    #[serde(default, rename = "pin")]
    pub pins: Vec<Pin>,
    /// The snoozes, oldest first.
    #[serde(default, rename = "snooze")]
    pub snoozes: Vec<Snooze>,
}

fn version() -> u32 {
    VERSION
}

impl Default for Marks {
    fn default() -> Self {
        Self {
            version: VERSION,
            pins: Vec::new(),
            snoozes: Vec::new(),
        }
    }
}

impl Marks {
    /// Whether a row is pinned.
    pub fn pinned(&self, key: &str) -> bool {
        self.pins.iter().any(|pin| pin.key == key)
    }

    /// The snooze of a row that still holds at `now`.
    pub fn snoozed(&self, key: &str, now: i64) -> Option<Snoozed> {
        self.snoozes
            .iter()
            .find(|snooze| snooze.key == key)
            .map(Snooze::snoozed)
            .filter(|snoozed| super::still_snoozed(snoozed, now))
    }

    /// Without the snoozes that have ended by `now`.
    pub fn without_ended(mut self, now: i64) -> Self {
        self.snoozes
            .retain(|snooze| super::still_snoozed(&snooze.snoozed(), now));
        self
    }
}

/// An entry of the file as it is read: anything may be in it.
#[derive(Debug, Deserialize)]
struct LooseEntry {
    #[serde(default)]
    key: Option<toml::Value>,
    #[serde(default)]
    since: Option<toml::Value>,
    #[serde(default)]
    until: Option<toml::Value>,
}

/// The file as it is read.
#[derive(Debug, Deserialize)]
struct LooseFile {
    #[serde(default)]
    version: Option<toml::Value>,
    #[serde(default)]
    pin: Option<Vec<toml::Value>>,
    #[serde(default)]
    snooze: Option<Vec<toml::Value>>,
}

/// What reading the file found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Loaded {
    /// The marks that could be used.
    pub marks: Marks,
    /// Why the file is not to be written over, when it is not.
    pub read_only: Option<String>,
}

/// Where the file is.
pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

fn entry(value: &toml::Value) -> Option<(String, i64, Option<i64>)> {
    let entry: LooseEntry = value.clone().try_into().ok()?;
    let key = entry.key?.as_str()?.to_owned();
    if !usable_key(&key) {
        return None;
    }
    let since = entry
        .since
        .as_ref()
        .and_then(toml::Value::as_integer)
        .unwrap_or(0);
    let until = match entry.until {
        None => None,
        Some(value) => Some(value.as_integer()?),
    };
    Some((key, since, until))
}

/// Read the marks of a text: the entries that can be used, the first of
/// each key, up to [`MAX_MARKS`] of each kind.
pub fn read(text: &str) -> Loaded {
    let file: LooseFile = match toml::from_str(text) {
        Ok(file) => file,
        Err(err) => {
            return Loaded {
                marks: Marks::default(),
                read_only: Some(say!(
                    "inbox-file-unreadable",
                    file = FILE,
                    error = err.message().trim()
                )),
            };
        }
    };
    let written_by = file
        .version
        .as_ref()
        .and_then(toml::Value::as_integer)
        .unwrap_or(i64::from(VERSION));
    let read_only = (written_by > i64::from(VERSION))
        .then(|| say!("inbox-file-newer", file = FILE, version = written_by));
    let mut marks = Marks::default();
    for (key, since, _) in file.pin.iter().flatten().filter_map(entry) {
        if marks.pins.len() >= MAX_MARKS {
            break;
        }
        if !marks.pinned(&key) {
            marks.pins.push(Pin { key, since });
        }
    }
    for (key, since, until) in file.snooze.iter().flatten().filter_map(entry) {
        if marks.snoozes.len() >= MAX_MARKS {
            break;
        }
        if !marks.snoozes.iter().any(|snooze| snooze.key == key) {
            marks.snoozes.push(Snooze { key, since, until });
        }
    }
    Loaded { marks, read_only }
}

/// Read the file of a configuration directory; nothing when there is
/// none.
pub fn load(dir: &Path) -> Loaded {
    let file = path(dir);
    match fs::metadata(&file) {
        Ok(meta) if meta.len() > MAX_FILE => {
            return Loaded {
                marks: Marks::default(),
                read_only: Some(say!("inbox-file-too-large", file = FILE, most = MAX_FILE)),
            };
        }
        Ok(_) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Loaded::default(),
        Err(err) => {
            return Loaded {
                marks: Marks::default(),
                read_only: Some(say!(
                    "inbox-file-unopenable",
                    file = FILE,
                    error = err.to_string()
                )),
            };
        }
    }
    match fs::read_to_string(&file) {
        Ok(text) => read(&text),
        Err(err) => Loaded {
            marks: Marks::default(),
            read_only: Some(say!(
                "inbox-file-unreadable",
                file = FILE,
                error = err.to_string()
            )),
        },
    }
}

/// Write the marks: beside the file first, then in its place.
fn store(dir: &Path, marks: &Marks) -> Result<(), String> {
    let text = toml::to_string_pretty(marks).map_err(|err| err.to_string())?;
    fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let beside = dir.join(format!("{FILE}.tmp-{}", std::process::id()));
    fs::write(&beside, text).map_err(|err| err.to_string())?;
    fs::rename(&beside, path(dir)).map_err(|err| {
        let _ = fs::remove_file(&beside);
        err.to_string()
    })
}

/// One change at a time in this process.
static WRITING: Mutex<()> = Mutex::new(());

/// Change the marks: read the file, let `change` do its part, drop the
/// snoozes that ended by `now`, and write it back. Refused when the file
/// is not to be written over, or the change would keep more than
/// [`MAX_MARKS`] of a kind.
pub fn change(dir: &Path, now: i64, change: impl FnOnce(&mut Marks)) -> Result<Marks, String> {
    let _writing = WRITING
        .lock()
        .map_err(|_| say!("inbox-marks-poisoned").to_owned())?;
    let loaded = load(dir);
    if let Some(why) = loaded.read_only {
        return Err(why);
    }
    let mut marks = loaded.marks.without_ended(now);
    change(&mut marks);
    if marks.pins.len() > MAX_MARKS || marks.snoozes.len() > MAX_MARKS {
        return Err(say!("inbox-marks-too-many", most = MAX_MARKS));
    }
    marks.version = VERSION;
    store(dir, &marks)?;
    Ok(marks)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bitbraid-pins-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn pins_and_snoozes_are_kept_and_read_back() {
        let dir = scratch("kept");
        assert_eq!(load(&dir), Loaded::default());
        let marks = change(&dir, 100, |marks| {
            marks.pins.push(Pin {
                key: "a/pull/o/r#1".into(),
                since: 100,
            });
            marks.snoozes.push(Snooze {
                key: "a/issue/o/r#2".into(),
                since: 100,
                until: Some(200),
            });
            marks.snoozes.push(Snooze {
                key: "a/issue/o/r#3".into(),
                since: 100,
                until: None,
            });
        })
        .unwrap();
        let loaded = load(&dir);
        assert_eq!(loaded.read_only, None);
        assert_eq!(loaded.marks, marks);
        assert!(loaded.marks.pinned("a/pull/o/r#1"));
        assert!(loaded.marks.snoozed("a/issue/o/r#2", 199).is_some());
        assert!(loaded.marks.snoozed("a/issue/o/r#2", 200).is_none());
        assert!(loaded.marks.snoozed("a/issue/o/r#3", i64::MAX).is_some());
        // A snooze that ended is gone with the next change.
        let after = change(&dir, 250, |_| {}).unwrap();
        assert_eq!(after.snoozes.len(), 1);
        assert!(!fs::read_dir(&dir).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp")
        }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_cannot_be_used_is_passed_over() {
        let long = "k".repeat(super::super::MAX_KEY + 1);
        let text = format!(
            r#"
version = 1
other = "kept for nobody"
[[pin]]
key = "good"
since = 5
[[pin]]
key = "good"
since = 6
[[pin]]
key = "{long}"
[[pin]]
key = 12
[[pin]]
since = 3
[[pin]]
key = "with space"
[[snooze]]
key = "s"
until = "tomorrow"
[[snooze]]
key = "t"
until = 9
since = "x"
"#
        );
        let loaded = read(&text);
        assert_eq!(loaded.read_only, None);
        assert_eq!(
            loaded.marks.pins,
            [Pin {
                key: "good".into(),
                since: 5
            }]
        );
        assert_eq!(
            loaded.marks.snoozes,
            [Snooze {
                key: "t".into(),
                since: 0,
                until: Some(9)
            }]
        );
        // Not a list of tables: nothing, and nothing breaks.
        assert_eq!(read("pin = 3").marks, Marks::default());
        assert_eq!(read("pin = [1, \"a\"]").marks, Marks::default());
    }

    #[test]
    fn a_file_that_cannot_be_read_or_is_newer_is_never_written_over() {
        let dir = scratch("broken");
        fs::create_dir_all(&dir).unwrap();
        for text in ["[[pin]\nkey = ", "version = 2\n[[pin]]\nkey = \"a\"\n"] {
            fs::write(path(&dir), text).unwrap();
            let loaded = load(&dir);
            assert!(loaded.read_only.is_some(), "{text:?}");
            assert!(change(&dir, 0, |marks| marks.pins.clear()).is_err());
            assert_eq!(fs::read_to_string(path(&dir)).unwrap(), text);
        }
        // One too large is not even read.
        let big = vec![b'#'; usize::try_from(MAX_FILE).unwrap() + 1];
        fs::write(path(&dir), &big).unwrap();
        assert!(load(&dir).read_only.is_some());
        assert!(change(&dir, 0, |_| {}).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_more_than_so_many_marks_are_read_or_kept() {
        let mut text = String::new();
        for at in 0..=MAX_MARKS {
            text.push_str(&format!("[[pin]]\nkey = \"k{at}\"\n"));
        }
        assert_eq!(read(&text).marks.pins.len(), MAX_MARKS);
        let dir = scratch("many");
        fs::create_dir_all(&dir).unwrap();
        fs::write(path(&dir), &text).unwrap();
        assert!(
            change(&dir, 0, |marks| marks.pins.push(Pin {
                key: "one more".into(),
                since: 0
            }))
            .is_err()
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
