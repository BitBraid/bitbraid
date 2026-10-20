//! The repository's own settings as a list: reading them, telling what
//! changed between two readings, and putting a change back or doing it
//! again. This is what makes a change of settings something undo can
//! take back, whichever command made it: the signing settings, an
//! identity written to the repository, gitflow's names, a branch's
//! description, a remote.
//!
//! Only the repository's own configuration is read and written here
//! (`--local`). What a command writes to the user's configuration is not
//! the repository's to take back.
//!
//! A setting is a name with all its values, in the order they stand in:
//! `remote.origin.fetch` may have several. A name that stands without a
//! value is read as `true`, which is what git takes it for.
//!
//! Sources: git-config(1) (`--local`, `--list`, `-z` and what it prints,
//! `--add`, `--unset-all`, the exit status 5 for a name that is not set,
//! the syntax of names: a subsection keeps its case and may hold dots).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Error, Repository, Result, exe};

/// One setting that a change touched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Setting {
    /// Its name, as git lists it.
    pub key: String,
    /// Its values before the change; none when it was not set.
    pub before: Vec<String>,
    /// Its values after the change; none when it was taken away.
    pub after: Vec<String>,
}

/// The longest name and the longest value that are written back.
const LONGEST: usize = 64 * 1024;

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("settings-what-settings"),
        detail: detail.into(),
    }
}

/// What `git config --list -z` printed, as names and values in order.
fn parse(listed: &str) -> Vec<(String, String)> {
    listed
        .split('\0')
        .filter(|record| !record.is_empty())
        .map(|record| match record.split_once('\n') {
            Some((key, value)) => (key.to_owned(), value.to_owned()),
            // A name alone is a switch that is on.
            None => (record.to_owned(), "true".to_owned()),
        })
        .collect()
}

/// Every setting of the repository's own configuration: name and value,
/// in the order of the file.
pub fn local(repo: &Repository) -> Result<Vec<(String, String)>> {
    let out = exe::run(repo, ["config", "--local", "--list", "-z"], None)?;
    Ok(parse(&out.stdout))
}

fn grouped(settings: &[(String, String)]) -> BTreeMap<&str, Vec<&str>> {
    let mut by_key: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (key, value) in settings {
        by_key.entry(key.as_str()).or_default().push(value.as_str());
    }
    by_key
}

/// What differs between two readings: each name whose values are not the
/// same, with all its values before and after, in the order of names.
pub fn changed(before: &[(String, String)], after: &[(String, String)]) -> Vec<Setting> {
    let (was, is) = (grouped(before), grouped(after));
    let owned = |values: Option<&Vec<&str>>| -> Vec<String> {
        values
            .map(|values| values.iter().map(|value| (*value).to_owned()).collect())
            .unwrap_or_default()
    };
    let mut keys: Vec<&str> = was.keys().chain(is.keys()).copied().collect();
    keys.sort_unstable();
    keys.dedup();
    keys.into_iter()
        .filter(|key| was.get(key) != is.get(key))
        .map(|key| Setting {
            key: key.to_owned(),
            before: owned(was.get(key)),
            after: owned(is.get(key)),
        })
        .collect()
}

/// Give `key` exactly `values`, in that order; none takes it away.
fn write(repo: &Repository, key: &str, values: &[String]) -> Result<()> {
    let (code, out) =
        exe::run_with_code(repo, ["config", "--local", "--unset-all", "--", key], None)?;
    // 5: it was not set.
    if !matches!(code, Some(0 | 5)) {
        return Err(refused(say!(
            "settings-not-taken-away",
            key,
            stderr = out.stderr.trim()
        )));
    }
    for value in values {
        exe::run(
            repo,
            ["config", "--local", "--add", "--", key, value.as_str()],
            None,
        )?;
    }
    Ok(())
}

/// Put every setting of a change back to what it was (`back`), or to
/// what the change made of it. Only when each still has the values the
/// other end of the change left it with: a setting that was changed
/// since is somebody's later decision, and then nothing is written at
/// all. When a write fails, those before it are taken back.
pub fn put(repo: &Repository, settings: &[Setting], back: bool) -> Result<()> {
    let ends = |setting: &'_ Setting| -> (Vec<String>, Vec<String>) {
        if back {
            (setting.after.clone(), setting.before.clone())
        } else {
            (setting.before.clone(), setting.after.clone())
        }
    };
    // What a journal says is not trusted further than a name and values
    // that git could have listed.
    for setting in settings {
        let odd = |text: &str| text.len() > LONGEST || text.contains('\0');
        let named = setting.key.contains('.')
            && !setting.key.starts_with('-')
            && !setting.key.chars().any(char::is_control);
        if !named || odd(&setting.key) || ends(setting).1.iter().any(|value| odd(value)) {
            return Err(refused(say!(
                "settings-not-writable",
                key = format!("{:?}", setting.key.chars().take(80).collect::<String>())
            )));
        }
    }
    let now = local(repo)?;
    let now = grouped(&now);
    for setting in settings {
        let (from, _) = ends(setting);
        let current: Vec<&str> = now.get(setting.key.as_str()).cloned().unwrap_or_default();
        if current != from.iter().map(String::as_str).collect::<Vec<_>>() {
            return Err(refused(say!(
                "settings-not-as-left",
                key = setting.key.as_str()
            )));
        }
    }
    for (done, setting) in settings.iter().enumerate() {
        let (_, to) = ends(setting);
        if let Err(err) = write(repo, &setting.key, &to) {
            // Half done is worse than not done.
            for earlier in settings.iter().take(done.saturating_add(1)) {
                let _ = write(repo, &earlier.key, &ends(earlier).0);
            }
            return Err(err);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn a_listing_is_names_and_values_in_order() {
        let listed = "core.bare\nfalse\0remote.origin.fetch\n+refs/heads/*:refs/remotes/origin/*\0\
                      branch.feature/x.y.description\nline one\nline two\n\0core.switch\0user.name\n\0";
        assert_eq!(
            parse(listed),
            pairs(&[
                ("core.bare", "false"),
                ("remote.origin.fetch", "+refs/heads/*:refs/remotes/origin/*"),
                ("branch.feature/x.y.description", "line one\nline two\n"),
                ("core.switch", "true"),
                ("user.name", ""),
            ])
        );
        assert!(parse("").is_empty());
        assert!(parse("\0\0").is_empty());
    }

    #[test]
    fn what_changed_is_each_name_with_all_its_values() {
        let before = pairs(&[
            ("core.bare", "false"),
            ("remote.origin.url", "/a"),
            ("remote.origin.fetch", "one"),
            ("remote.origin.fetch", "two"),
            ("user.name", "Ada"),
            ("gone.key", "x"),
        ]);
        let after = pairs(&[
            ("core.bare", "false"),
            ("remote.origin.url", "/b"),
            // The same two, the other way round: that is a change.
            ("remote.origin.fetch", "two"),
            ("remote.origin.fetch", "one"),
            ("user.name", "Ada"),
            ("new.key", "y"),
            ("new.key", "z"),
        ]);
        let found = changed(&before, &after);
        let brief: Vec<(&str, Vec<&str>, Vec<&str>)> = found
            .iter()
            .map(|setting| {
                (
                    setting.key.as_str(),
                    setting.before.iter().map(String::as_str).collect(),
                    setting.after.iter().map(String::as_str).collect(),
                )
            })
            .collect();
        assert_eq!(
            brief,
            [
                ("gone.key", vec!["x"], vec![]),
                ("new.key", vec![], vec!["y", "z"]),
                (
                    "remote.origin.fetch",
                    vec!["one", "two"],
                    vec!["two", "one"]
                ),
                ("remote.origin.url", vec!["/a"], vec!["/b"]),
            ]
        );
        assert!(changed(&before, &before).is_empty());
        assert!(changed(&[], &[]).is_empty());
    }
}
