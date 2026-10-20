//! Blame that passes over revisions: commits that only reformatted or
//! renamed are listed in a file, and blame then names the commit that
//! changed a line before them.
//!
//! The file is the one `blame.ignoreRevsFile` names, or, where that is not
//! set, `.git-blame-ignore-revs` at the top of the worktree, the name
//! projects have settled on. Gitoxide's blame does not pass over
//! revisions, so this one is git's own (`git blame --porcelain
//! --ignore-revs-file`), read back into the same lines.
//!
//! Sources: git-blame(1) (`--ignore-revs-file`, `--porcelain` and "THE
//! PORCELAIN FORMAT"), git-config(1) (`blame.ignoreRevsFile`).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{BlameLine, Error, Oid, Repository, Result, activity::lossy, exe, stage::check_rel};

/// The conventional name of the file, at the top of the worktree.
pub const CONVENTIONAL: &str = ".git-blame-ignore-revs";

/// The file of revisions blame passes over in a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct IgnoreRevs {
    /// The file.
    #[serde(serialize_with = "lossy")]
    pub file: PathBuf,
    /// Git's configuration names it (`blame.ignoreRevsFile`), so `git
    /// blame` passes over its revisions by itself. Otherwise it is the
    /// conventional file, which git reads only when told to.
    pub configured: bool,
    /// It is there to be read.
    pub exists: bool,
    /// How many revisions it lists.
    pub revisions: usize,
}

/// How many lines of a file of revisions name one: not blank, not a
/// comment.
pub fn count_revisions(text: &str) -> usize {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .count()
}

/// The file of revisions to pass over here, when there is one: the
/// configured one, else the conventional one when it is there.
pub fn ignore_revs(repo: &Repository) -> Option<IgnoreRevs> {
    let describe = |file: PathBuf, configured: bool| {
        // Not read whole when it is no file of revisions at all.
        let text = std::fs::metadata(&file)
            .ok()
            .filter(|meta| meta.is_file() && meta.len() <= 16 * 1024 * 1024)
            .and_then(|_| std::fs::read(&file).ok())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        IgnoreRevs {
            configured,
            exists: text.is_some(),
            revisions: text.as_deref().map(count_revisions).unwrap_or(0),
            file,
        }
    };
    if let Some(file) = exe::config_path(repo, "blame.ignoreRevsFile") {
        return Some(describe(file, true));
    }
    let conventional = repo.workdir()?.join(CONVENTIONAL);
    conventional
        .is_file()
        .then(|| describe(conventional, false))
}

fn unreadable(detail: impl Into<String>) -> Error {
    Error::Invalid {
        what: say!("blame-what"),
        detail: detail.into(),
    }
}

/// Read `git blame --porcelain` of `path`: a header line per line of the
/// file (the commit, the line's number there, its number here), details
/// of a commit the first time it is named, and the line itself after a
/// tab. Among the details is the file's name in the commit, given at
/// least once for each commit and again where its lines come from
/// another name; where it is not `path`, the lines say it.
pub fn parse_porcelain(text: &str, path: &str) -> Result<Vec<BlameLine>> {
    let mut out = Vec::new();
    let mut current: Option<(Oid, u32, u32)> = None;
    let mut names: std::collections::HashMap<Oid, String> = std::collections::HashMap::new();
    for raw in text.split('\n') {
        if let Some(content) = raw.strip_prefix('\t') {
            let Some((commit, source_line, line)) = current.take() else {
                return Err(unreadable(say!("blame-line-before-commit")));
            };
            out.push(BlameLine {
                line,
                commit,
                source_line,
                text: content.to_owned(),
                source_path: names.get(&commit).filter(|name| *name != path).cloned(),
            });
            continue;
        }
        if let Some((commit, _, _)) = current {
            // A detail of the commit: author, summary, filename, …
            if let Some(name) = raw.strip_prefix("filename ") {
                // Quoted the way C writes a string where it has to be.
                let name = match name.strip_prefix('"') {
                    Some(quoted) => crate::lfs::unquote(quoted)
                        .map(|(name, _)| name)
                        .ok_or_else(|| {
                            unreadable(say!("blame-not-file-name", name = format!("{name:?}")))
                        })?,
                    None => name.to_owned(),
                };
                names.insert(commit, name);
            }
            continue;
        }
        if raw.is_empty() {
            continue;
        }
        let mut words = raw.split(' ');
        let (Some(id), Some(source), Some(line)) = (words.next(), words.next(), words.next())
        else {
            return Err(unreadable(say!(
                "blame-not-header",
                raw = format!("{raw:?}")
            )));
        };
        let commit = id
            .parse::<Oid>()
            .map_err(|_| unreadable(say!("blame-not-commit-id", id = format!("{id:?}"))))?;
        let number = |word: &str| {
            word.parse::<u32>()
                .map_err(|_| unreadable(say!("blame-not-line-number", word = format!("{word:?}"))))
        };
        current = Some((commit, number(source)?, number(line)?));
    }
    Ok(out)
}

/// Blame `path` as of `rev` (HEAD when absent), passing over the revisions
/// the file lists.
pub fn blame_ignoring(
    repo: &Repository,
    path: &str,
    rev: Option<Oid>,
    revisions: &Path,
) -> Result<Vec<BlameLine>> {
    check_rel(path)?;
    let rev = rev
        .map(|id| id.to_hex())
        .unwrap_or_else(|| "HEAD".to_owned());
    let mut ignore = std::ffi::OsString::from("--ignore-revs-file=");
    ignore.push(revisions.as_os_str());
    let args: Vec<std::ffi::OsString> = vec![
        "--literal-pathspecs".into(),
        "blame".into(),
        "--porcelain".into(),
        // Root commits are commits like the others, not a boundary.
        "--root".into(),
        ignore,
        rev.into(),
        "--".into(),
        path.into(),
    ];
    let out = exe::run(repo, &args, None)?;
    parse_porcelain(&out.stdout, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn porcelain_is_read_into_lines() {
        let a = "a".repeat(40);
        let b = "b".repeat(40);
        // The file was renamed after `a`: its name there, as git quotes
        // it, is said on a's lines.
        let text = format!(
            "{a} 1 1 2\nauthor Ada\nauthor-mail <ada@x.invalid>\nsummary first\nfilename \"\\303\\241 old.txt\"\n\tone\n\
             {a} 2 2\n\t\ttabbed two\n\
             {b} 7 3 1\nauthor Bob\nprevious {a} f.txt\nfilename f.txt\n\tthree\n\
             {a} 4 4 1\n\t\n"
        );
        let lines = parse_porcelain(&text, "f.txt").unwrap();
        let said: Vec<(u32, String, u32, &str, Option<&str>)> = lines
            .iter()
            .map(|line| {
                (
                    line.line,
                    line.commit.short(1),
                    line.source_line,
                    line.text.as_str(),
                    line.source_path.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            said,
            [
                (1, "a".to_owned(), 1, "one", Some("á old.txt")),
                (2, "a".to_owned(), 2, "\ttabbed two", Some("á old.txt")),
                (3, "b".to_owned(), 7, "three", None),
                (4, "a".to_owned(), 4, "", Some("á old.txt")),
            ]
        );
        assert!(parse_porcelain("", "f.txt").unwrap().is_empty());
    }

    #[test]
    fn what_is_not_porcelain_is_refused_not_guessed() {
        let a = "a".repeat(40);
        for bad in [
            "\tcontent without a header\n".to_owned(),
            "nonsense\n".to_owned(),
            format!("{a} x 1\n\tline\n"),
            format!("{a} 1\n\tline\n"),
            "zz 1 1\n\tline\n".to_owned(),
            format!("{a} 1 1 1\nfilename \"unterminated\n\tline\n"),
            format!("{a} 1 1 1\nfilename \"\\39\"\n\tline\n"),
        ] {
            assert!(parse_porcelain(&bad, "f.txt").is_err(), "{bad:?}");
        }
    }

    #[test]
    fn revisions_are_the_lines_that_are_not_comments() {
        assert_eq!(count_revisions(""), 0);
        assert_eq!(
            count_revisions("# reformatted\nabc123\n\n  def456  \n# end\n"),
            2
        );
    }
}
