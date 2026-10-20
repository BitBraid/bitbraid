//! Suggestions from local edits: `pr.suggestions` says what the changes
//! in the working tree to the files of a pull request would be sent as,
//! so that they can be looked at first; each is then sent with
//! `pr.comment` on its lines. The pull request's branch is to be checked
//! out in the repository that is open; what is compared is the commit
//! checked out and the working tree ([`crate::suggest`]).
//!
//! Sources: [`crate::suggest`]; docs.github.com/en/rest/pulls and
//! docs.gitlab.com/api/merge_requests for what `pr.show` reads.

use bitbraid_engine::{
    CommandContext, CommandError, CommandSpec, Output, ParamKind, ParamSpec, Params, Registry,
};
use bitbraid_git::{DiffOptions, DiffSide};

use crate::{
    cmds::{self, ACCOUNT, NAME, NUMBER, OWNER, READS, failed, invalid, opt},
    suggest::{self, LocalChange, Suggestions},
};

/// The most bytes a file compared may have on either side.
const MOST_FILE_BYTES: u64 = 2 * 1024 * 1024;
/// Lines around a change when the file is read whole: as many as it has.
const WHOLE: u32 = u32::MAX;
/// The longest note put before each suggestion.
const MOST_NOTE: usize = 4096;

const SUGGESTIONS: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    NUMBER,
    opt(
        "path",
        ParamKind::Text,
        message!("suggest-param-suggestions-path"),
    ),
    opt(
        "note",
        ParamKind::Text,
        message!("suggest-param-suggestions-note"),
    ),
];

/// Register the command.
pub fn register(registry: &mut Registry) {
    let _ = registry.register_described(
        CommandSpec {
            id: "pr.suggestions",
            label: say!("suggest-pr-suggestions"),
            description: say!("suggest-pr-suggestions-description"),
        },
        suggestions,
        READS,
        SUGGESTIONS,
    );
}

/// Whether a path a provider names can be one of the working tree's.
fn plain_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && !path.starts_with('/')
        && !path.chars().any(char::is_control)
        && path.split('/').all(|part| !matches!(part, "" | "." | ".."))
        && bitbraid_core::paths::stays_inside(path)
}

fn suggestions(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    let only = cmds::text_with(params, "path", 4096, false)?;
    if only.as_deref().is_some_and(|path| !plain_path(path)) {
        return Err(invalid("path", say!("suggest-not-a-path")));
    }
    let note = cmds::text_with(params, "note", MOST_NOTE, true)?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let details = signed
        .provider()
        .pr_show(&signed.session(ctx), &target, number)
        .map_err(failed)?;
    let kind = signed.provider().kind();
    let repo = ctx.repo()?;
    if repo.workdir().is_none() {
        return Err(invalid("repository", say!("suggest-no-working-tree")));
    }
    let head = DiffSide::parse(repo, "HEAD")?;
    let checked_out = match &head {
        DiffSide::Commit(id) => Some(id.to_string()),
        _ => None,
    };
    let mut made = Vec::new();
    for file in &details.files {
        if only.as_deref().is_some_and(|path| path != file.path)
            || file.status == "removed"
            || !plain_path(&file.path)
        {
            continue;
        }
        let Some(patch) = file.patch.as_deref() else {
            continue;
        };
        let compare = |context: u32| DiffOptions {
            context,
            paths: vec![file.path.clone()],
            detect_renames: false,
            whitespace: Default::default(),
            max_bytes: Some(MOST_FILE_BYTES),
        };
        let changed =
            bitbraid_git::compare::diff_mixed(repo, &head, &DiffSide::Worktree, &compare(0))?;
        let Some(diff) = changed
            .iter()
            .find(|diff| diff.path == file.path && !diff.binary && !diff.hunks.is_empty())
        else {
            continue;
        };
        let changes: Vec<LocalChange> = diff.hunks.iter().map(LocalChange::of).collect();
        // The commit's version, whole: every line of the old side of one
        // diff with all of the file around its changes.
        let whole =
            bitbraid_git::compare::diff_mixed(repo, &head, &DiffSide::Worktree, &compare(WHOLE))?;
        let mut lines: Vec<String> = Vec::new();
        for line in whole
            .iter()
            .filter(|diff| diff.path == file.path)
            .flat_map(|diff| diff.hunks.iter())
            .flat_map(|hunk| hunk.lines.iter())
        {
            if let Some(at) = line.old_no.and_then(|at| usize::try_from(at).ok()) {
                if lines.len() < at {
                    lines.resize(at, String::new());
                }
                if let Some(slot) = at.checked_sub(1).and_then(|at| lines.get_mut(at)) {
                    slot.clone_from(&line.text);
                }
            }
        }
        made.extend(suggest::suggestions(
            &file.path,
            patch,
            &lines,
            &changes,
            kind,
            note.as_deref(),
        ));
    }
    cmds::answer(&Suggestions {
        head_sha: details.pull.head_sha.clone(),
        checked_out,
        suggestions: made,
    })
}

#[cfg(test)]
mod tests {
    use super::plain_path;

    #[test]
    fn only_paths_inside_the_working_tree_are_taken() {
        assert!(plain_path("src/a.rs"));
        assert!(plain_path("a b/ü.txt"));
        for bad in [
            "",
            "/etc/passwd",
            "../x",
            "a/../b",
            "a//b",
            "./a",
            "a\u{0}b",
            "a/",
        ] {
            assert!(!plain_path(bad), "{bad:?}");
        }
    }
}
