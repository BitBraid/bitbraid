//! What each command takes: for every parameter its name, what kind of
//! value it is, whether it has to be given, and a few words about it.
//!
//! The table is what a surface without a form of its own goes by: the
//! palette's prompt and `commands --json`. [`from_text`] turns what was
//! typed into such a prompt into the JSON a handler reads.
//!
//! The schemas live here, in one table, rather than beside each
//! `CommandSpec`: the registry attaches them when a command is registered
//! (`Registry::params`). A test holds the table to the handlers: every
//! registered command has a schema, no schema names a command that does
//! not exist, and what is marked required is what a handler refuses to do
//! without. A command registered from a higher layer brings its own row
//! (`Registry::register_described`), so a surface asks the registry
//! (`Registry::params`) rather than this table.
//!
//! Sources: the handlers in this crate (`*_cmds.rs`), which are what is
//! described.

use std::collections::BTreeMap;

use bitbraid_strings::Message;
use serde::Serialize;

use crate::command::{CommandError, Params};

/// What kind of value a parameter is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ParamKind {
    /// Any text: a name, a message, a URL, a pattern.
    Text,
    /// Yes or no.
    Flag,
    /// A whole number.
    Number,
    /// A file or folder: relative to the worktree, or a full path, as the
    /// description says.
    Path,
    /// Something that names a commit: a branch, a tag, an id, `HEAD~2`.
    Revision,
    /// Several texts.
    List,
    /// One of a fixed set of words.
    Choice(&'static [&'static str]),
    /// Something with a shape of its own, given as JSON.
    Structured,
}

impl ParamKind {
    /// The kind as one word, as the catalog names it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Flag => "flag",
            Self::Number => "number",
            Self::Path => "path",
            Self::Revision => "revision",
            Self::List => "list",
            Self::Choice(_) => "choice",
            Self::Structured => "structured",
        }
    }

    /// The words a choice is among; empty for every other kind.
    pub fn choices(&self) -> &'static [&'static str] {
        match self {
            Self::Choice(choices) => choices,
            _ => &[],
        }
    }
}

/// One parameter of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamSpec {
    /// Its key in the parameters object.
    pub name: &'static str,
    /// What kind of value it is.
    pub kind: ParamKind,
    /// Whether the command refuses to run without it.
    pub required: bool,
    /// A few words about it.
    pub description: Message,
}

const fn req(name: &'static str, kind: ParamKind, description: Message) -> ParamSpec {
    ParamSpec {
        name,
        kind,
        required: true,
        description,
    }
}

const fn opt(name: &'static str, kind: ParamKind, description: Message) -> ParamSpec {
    ParamSpec {
        name,
        kind,
        required: false,
        description,
    }
}

use ParamKind::{Choice, Flag, List, Number, Path, Revision, Structured, Text};

const DIRTY: ParamKind = Choice(&["refuse", "stash", "discard"]);
const SIDES: Message = message!("params-diff-side");
const TIPS: ParamSpec = opt("tips", Structured, message!("params-tips"));
const ZONE: ParamSpec = opt("zone", Text, message!("params-zone"));
const ORDER: ParamSpec = opt(
    "order",
    Choice(&["date", "topo", "author_date"]),
    message!("params-order"),
);
const FLOW_KIND: ParamKind = Choice(&["feature", "bugfix", "release", "hotfix", "support"]);
/// The kinds that are finished: a support branch never is.
const FLOW_FINISHED: ParamKind = Choice(&["feature", "bugfix", "release", "hotfix"]);
const CONFIG_SCOPE: ParamKind = Choice(&["repo", "user"]);
const WORKSPACE_ID: ParamSpec = req("id", Text, message!("params-workspace-id"));
const WORKSPACE_PATHS: ParamSpec = opt("paths", List, message!("params-workspace-paths"));
const WORKSPACE_REPOSITORIES: ParamSpec = opt(
    "repositories",
    Structured,
    message!("params-workspace-repositories"),
);
const WORKSPACE_CHOSEN: ParamSpec = opt("paths", List, message!("params-workspace-chosen"));
const WORKSPACE_PARALLEL: ParamSpec =
    opt("parallel", Number, message!("params-workspace-parallel"));
const WORKSPACE_LAYOUT: ParamSpec = opt("layout", Structured, message!("params-workspace-layout"));

const NONE: &[ParamSpec] = &[];

/// Every command's parameters: the ids a row is for, separated by spaces,
/// and what they take. A command that takes nothing has an empty row.
static SCHEMAS: &[(&str, &[ParamSpec])] = &[
    // Read-only: the repository.
    (
        "repo.open",
        &[opt("path", Path, message!("params-repo-open-path"))],
    ),
    ("repo.summary repo.refs", NONE),
    (
        "repo.status",
        &[opt("limit", Number, message!("params-repo-status-limit"))],
    ),
    (
        "repo.log",
        &[
            TIPS,
            opt("limit", Number, message!("params-limit-commits")),
            opt("first_parent", Flag, message!("params-first-parent")),
            ORDER,
        ],
    ),
    (
        "repo.graph",
        &[
            TIPS,
            opt("limit", Number, message!("params-limit-commits")),
            opt("first_parent", Flag, message!("params-first-parent")),
            ORDER,
            opt("details", Flag, message!("params-repo-graph-details")),
            opt("ascii", Flag, message!("params-repo-graph-ascii")),
            opt("pinned", List, message!("params-repo-graph-pinned")),
            opt("stashes", Flag, message!("params-repo-graph-stashes")),
        ],
    ),
    (
        "repo.commits",
        &[req("ids", List, message!("params-commit-ids"))],
    ),
    (
        "repo.diff",
        &[
            opt("old", Revision, message!("params-repo-diff-old")),
            opt("new", Revision, message!("params-repo-diff-new")),
            opt("paths", List, message!("params-repo-diff-paths")),
            opt("context", Number, message!("params-repo-diff-context")),
            opt(
                "detect_renames",
                Flag,
                message!("params-repo-diff-detect-renames"),
            ),
            opt(
                "whitespace",
                Choice(&["exact", "ignore_trailing", "ignore_all", "ignore_cr_at_eol"]),
                message!("params-repo-diff-whitespace"),
            ),
            opt("max_bytes", Number, message!("params-repo-diff-max-bytes")),
        ],
    ),
    (
        "repo.glance",
        &[req("paths", List, message!("params-repo-glance-paths"))],
    ),
    (
        "repo.blame",
        &[
            req("path", Path, message!("params-file-in-worktree")),
            opt("rev", Revision, message!("params-repo-blame-rev")),
            opt(
                "ignore_revs",
                Flag,
                message!("params-repo-blame-ignore-revs"),
            ),
        ],
    ),
    ("repo.blame_ignores", NONE),
    (
        "repo.blame_previous",
        &[
            req("path", Path, message!("params-repo-blame-previous-path")),
            req("rev", Revision, message!("params-repo-blame-previous-rev")),
        ],
    ),
    (
        "repo.history",
        &[
            req("path", Path, message!("params-file-in-worktree")),
            opt("follow", Flag, message!("params-repo-history-follow")),
            TIPS,
            opt("limit", Number, message!("params-repo-history-limit")),
            opt("first_parent", Flag, message!("params-first-parent")),
            opt(
                "max_entries",
                Number,
                message!("params-repo-history-max-entries"),
            ),
        ],
    ),
    // Making repositories.
    (
        "repo.init",
        &[
            req("path", Path, message!("params-repo-init-path")),
            opt("branch", Text, message!("params-repo-init-branch")),
            opt(
                "first_commit",
                Flag,
                message!("params-repo-init-first-commit"),
            ),
            opt("gitignore", List, message!("params-repo-init-gitignore")),
        ],
    ),
    (
        "repo.ignore_templates",
        &[opt(
            "names",
            List,
            message!("params-repo-ignore-templates-names"),
        )],
    ),
    (
        "repo.clone",
        &[
            req("url", Text, message!("params-repo-clone-url")),
            req("path", Path, message!("params-repo-clone-path")),
            opt("branch", Text, message!("params-repo-clone-branch")),
            opt("depth", Number, message!("params-repo-clone-depth")),
            opt(
                "filter",
                Choice(&["blob:none", "tree:0"]),
                message!("params-repo-clone-filter"),
            ),
            opt("sparse", Flag, message!("params-repo-clone-sparse")),
            opt(
                "recurse_submodules",
                Flag,
                message!("params-repo-clone-recurse-submodules"),
            ),
            opt(
                "allow_local",
                Flag,
                message!("params-repo-clone-allow-local"),
            ),
        ],
    ),
    // The folder in other programs.
    (
        "repo.open_folder",
        &[
            opt("path", Path, message!("params-folder-to-open")),
            opt("command", Text, message!("params-repo-open-folder-command")),
        ],
    ),
    (
        "repo.open_editor",
        &[
            opt("command", Text, message!("params-repo-open-editor-command")),
            opt("path", Path, message!("params-folder-to-open")),
        ],
    ),
    (
        "repo.open_terminal",
        &[
            opt(
                "command",
                Text,
                message!("params-repo-open-terminal-command"),
            ),
            opt("path", Path, message!("params-folder-to-open")),
        ],
    ),
    ("repo.terminals", NONE),
    // SSH keys for the providers.
    (
        "ssh.key_generate",
        &[
            req("path", Path, message!("params-ssh-key-generate-path")),
            opt("comment", Text, message!("params-ssh-key-generate-comment")),
        ],
    ),
    (
        "ssh.key_fingerprint",
        &[req(
            "path",
            Path,
            message!("params-ssh-key-fingerprint-path"),
        )],
    ),
    // The embedded terminal.
    (
        "terminal.shell",
        &[
            opt("path", Path, message!("params-terminal-shell-path")),
            opt("program", Path, message!("params-terminal-shell-program")),
            opt("args", List, message!("params-terminal-shell-args")),
        ],
    ),
    (
        "terminal.completion_words",
        &[
            opt(
                "changes",
                Flag,
                message!("params-terminal-completion-words-changes"),
            ),
            opt(
                "most_paths",
                Number,
                message!("params-terminal-completion-words-most-paths"),
            ),
        ],
    ),
    (
        "terminal.complete",
        &[
            req("line", Text, message!("params-terminal-complete-line")),
            opt("folder", Text, message!("params-terminal-complete-folder")),
            opt("shell", Text, message!("params-terminal-complete-shell")),
        ],
    ),
    (
        "terminal.paste_paths",
        &[
            req("paths", List, message!("params-terminal-paste-paths-paths")),
            opt("shell", Text, message!("params-terminal-paste-paths-shell")),
        ],
    ),
    // The git executable.
    (
        "git.info",
        &[opt("program", Path, message!("params-git-info-program"))],
    ),
    (
        "git.use",
        &[
            opt("program", Path, message!("params-git-use-program")),
            opt("verify", Flag, message!("params-git-use-verify")),
        ],
    ),
    // Search.
    (
        "search.parse",
        &[
            req("query", Text, message!("params-search-query")),
            opt("now", Number, message!("params-search-parse-now")),
            ZONE,
        ],
    ),
    (
        "search.commits",
        &[
            req("query", Text, message!("params-search-query")),
            TIPS,
            opt("limit", Number, message!("params-search-commits-limit")),
            opt("now", Number, message!("params-search-now")),
            ZONE,
        ],
    ),
    (
        "search.content",
        &[
            req("text", Text, message!("params-search-content-text")),
            opt("regex", Flag, message!("params-search-content-regex")),
            opt("paths", List, message!("params-search-content-paths")),
            TIPS,
            opt("limit", Number, message!("params-search-content-limit")),
        ],
    ),
    (
        "search.files",
        &[
            req("query", Text, message!("params-search-files-query")),
            opt("rev", Revision, message!("params-search-files-rev")),
            opt("limit", Number, message!("params-search-files-limit")),
        ],
    ),
    (
        "search.locate",
        &[
            opt("rev", Revision, message!("params-search-locate-rev")),
            opt("date", Text, message!("params-search-locate-date")),
            TIPS,
            opt("now", Number, message!("params-search-now")),
            ZONE,
        ],
    ),
    // Staging.
    (
        "stage.paths unstage.paths discard.paths",
        &[req("paths", List, message!("params-stage-paths-paths"))],
    ),
    (
        "stage.hunks unstage.hunks discard.hunks",
        &[
            req("path", Path, message!("params-file-in-worktree")),
            req(
                "selection",
                Structured,
                message!("params-stage-hunks-selection"),
            ),
            opt("context", Number, message!("params-stage-hunks-context")),
        ],
    ),
    (
        "ignore.add",
        &[
            opt("pattern", Text, message!("params-ignore-add-pattern")),
            opt("path", Path, message!("params-ignore-add-path")),
            opt(
                "what",
                Choice(&["file", "folder", "extension"]),
                message!("params-ignore-add-what"),
            ),
            opt(
                "target",
                Choice(&["repository", "local", "global"]),
                message!("params-ignore-add-target"),
            ),
        ],
    ),
    (
        "ignore.tracked",
        &[opt(
            "limit",
            Number,
            message!("params-ignore-tracked-limit"),
        )],
    ),
    (
        "ignore.untrack",
        &[req("paths", List, message!("params-ignore-untrack-paths"))],
    ),
    ("index.flags", NONE),
    (
        "index.set_flag",
        &[
            req("paths", List, message!("params-index-set-flag-paths")),
            req(
                "flag",
                Choice(&["intent_to_add", "skip_worktree", "assume_unchanged"]),
                message!("params-index-set-flag-flag"),
            ),
            req("on", Flag, message!("params-index-set-flag-on")),
        ],
    ),
    // Commits.
    (
        "commit.create",
        &[
            req("message", Text, message!("params-commit-create-message")),
            opt("stage_all", Flag, message!("params-stage-all")),
            opt("no_verify", Flag, message!("params-skip-commit-hooks")),
            opt("sign", Flag, message!("params-commit-create-sign")),
            opt(
                "allow_empty",
                Flag,
                message!("params-commit-create-allow-empty"),
            ),
        ],
    ),
    (
        "commit.amend",
        &[
            req("message", Text, message!("params-commit-amend-message")),
            opt(
                "keep_author",
                Flag,
                message!("params-commit-amend-keep-author"),
            ),
            opt("stage_all", Flag, message!("params-stage-all")),
            opt("no_verify", Flag, message!("params-skip-commit-hooks")),
            opt("sign", Flag, message!("params-commit-amend-sign")),
        ],
    ),
    (
        "commit.messages",
        &[opt(
            "limit",
            Number,
            message!("params-commit-messages-limit"),
        )],
    ),
    (
        "commit.trailer",
        &[
            req("message", Text, message!("params-commit-trailer-message")),
            opt(
                "trailers",
                Structured,
                message!("params-commit-trailer-trailers"),
            ),
            opt("sign_off", Flag, message!("params-commit-trailer-sign-off")),
        ],
    ),
    (
        "commit.trailer_keys",
        &[opt(
            "limit",
            Number,
            message!("params-commit-trailer-keys-limit"),
        )],
    ),
    (
        "commit.template undo.last redo.last undo.peek undo.journal",
        NONE,
    ),
    (
        "commit.people",
        &[opt("limit", Number, message!("params-commit-people-limit"))],
    ),
    (
        "undo.to redo.to",
        &[req("id", Number, message!("params-undo-to-id"))],
    ),
    (
        "commit.reword",
        &[
            req("commit", Revision, message!("params-commit-reword-commit")),
            req("message", Text, message!("params-commit-reword-message")),
        ],
    ),
    (
        "commit.drop",
        &[req(
            "commit",
            Revision,
            message!("params-commit-drop-commit"),
        )],
    ),
    (
        "commit.squash",
        &[req(
            "commits",
            List,
            message!("params-commit-squash-commits"),
        )],
    ),
    (
        "commit.move",
        &[
            req("commits", List, message!("params-commit-move-commits")),
            req("up", Flag, message!("params-commit-move-up")),
        ],
    ),
    (
        "commit.rewrite_way",
        &[req(
            "commit",
            Revision,
            message!("params-commit-rewrite-way-commit"),
        )],
    ),
    (
        "commit.cherry_pick",
        &[
            req(
                "commits",
                List,
                message!("params-commit-cherry-pick-commits"),
            ),
            opt(
                "no_commit",
                Flag,
                message!("params-commit-cherry-pick-no-commit"),
            ),
            opt("mainline", Number, message!("params-mainline")),
        ],
    ),
    (
        "commit.revert",
        &[
            req("commits", List, message!("params-commit-revert-commits")),
            opt("mainline", Number, message!("params-mainline")),
        ],
    ),
    // Files.
    (
        "file.open",
        &[
            req("path", Path, message!("params-file-in-worktree")),
            opt("command", Text, message!("params-file-open-command")),
            opt("line", Number, message!("params-file-open-line")),
            opt("rev", Revision, message!("params-file-open-rev")),
        ],
    ),
    (
        "file.reveal",
        &[
            req("path", Path, message!("params-file-in-worktree")),
            opt("command", Text, message!("params-file-reveal-command")),
        ],
    ),
    (
        "file.restore",
        &[
            req("path", Path, message!("params-file-in-worktree")),
            req("rev", Revision, message!("params-file-restore-rev")),
        ],
    ),
    (
        "file.bytes",
        &[
            req("path", Path, message!("params-file-in-worktree")),
            opt("side", Revision, message!("params-file-bytes-side")),
            opt("max", Number, message!("params-file-bytes-max")),
        ],
    ),
    (
        "file.write",
        &[
            req("path", Path, message!("params-file-in-worktree")),
            req("text", Text, message!("params-file-write-text")),
            opt("base", Text, message!("params-file-write-base")),
        ],
    ),
    // Branches.
    (
        "branch.create",
        &[
            req("name", Text, message!("params-new-branch-name")),
            opt("start", Revision, message!("params-branch-create-start")),
            opt("checkout", Flag, message!("params-branch-create-checkout")),
            opt("dirty", DIRTY, message!("params-dirty")),
        ],
    ),
    (
        "branch.checkout",
        &[
            req(
                "target",
                Revision,
                message!("params-branch-checkout-target"),
            ),
            opt("dirty", DIRTY, message!("params-dirty")),
        ],
    ),
    (
        "branch.rename",
        &[
            req("name", Text, message!("params-branch")),
            req("new_name", Text, message!("params-branch-rename-new-name")),
        ],
    ),
    (
        "branch.delete",
        &[
            req("name", Text, message!("params-branch")),
            opt("force", Flag, message!("params-branch-delete-force")),
            opt("protected", List, message!("params-protected")),
        ],
    ),
    (
        "branch.delete_remote",
        &[
            req("remote", Text, message!("params-remote")),
            req(
                "branch",
                Text,
                message!("params-branch-delete-remote-branch"),
            ),
            opt("protected", List, message!("params-protected")),
        ],
    ),
    (
        "branch.set_upstream",
        &[
            req("name", Text, message!("params-local-branch")),
            opt(
                "upstream",
                Text,
                message!("params-branch-set-upstream-upstream"),
            ),
        ],
    ),
    (
        "branch.fast_forward",
        &[
            req("name", Text, message!("params-branch-fast-forward-name")),
            opt("to", Revision, message!("params-branch-fast-forward-to")),
        ],
    ),
    ("branch.tracking", NONE),
    (
        "branch.overview",
        &[
            opt("base", Revision, message!("params-branch-overview-base")),
            opt(
                "stale_days",
                Number,
                message!("params-branch-overview-stale-days"),
            ),
            opt("now", Number, message!("params-branch-overview-now")),
        ],
    ),
    (
        "branch.describe",
        &[
            req("name", Text, message!("params-local-branch")),
            opt(
                "description",
                Text,
                message!("params-branch-describe-description"),
            ),
        ],
    ),
    (
        "branch.delete_merged",
        &[
            opt(
                "base",
                Revision,
                message!("params-branch-delete-merged-base"),
            ),
            opt(
                "protected",
                List,
                message!("params-branch-delete-merged-protected"),
            ),
            opt(
                "dry_run",
                Flag,
                message!("params-branch-delete-merged-dry-run"),
            ),
        ],
    ),
    (
        "branch.merge",
        &[
            req("target", Revision, message!("params-branch-merge-target")),
            opt(
                "mode",
                Choice(&["ff_only", "ff", "no_ff", "squash", "no_commit"]),
                message!("params-branch-merge-mode"),
            ),
            opt("no_verify", Flag, message!("params-branch-merge-no-verify")),
        ],
    ),
    (
        "branch.rebase",
        &[
            req("onto", Revision, message!("params-branch-rebase-onto")),
            opt("after", Revision, message!("params-branch-rebase-after")),
            opt(
                "autostash",
                Flag,
                message!("params-branch-rebase-autostash"),
            ),
            opt("no_verify", Flag, message!("params-skip-rebase-hook")),
        ],
    ),
    (
        "branch.predict",
        &[
            req("target", Revision, message!("params-branch-predict-target")),
            opt("ours", Revision, message!("params-branch-predict-ours")),
            opt("aside", Flag, message!("params-branch-predict-aside")),
        ],
    ),
    (
        "branch.reset",
        &[
            req("target", Revision, message!("params-branch-reset-target")),
            opt(
                "mode",
                Choice(&["soft", "mixed", "hard", "keep"]),
                message!("params-branch-reset-mode"),
            ),
        ],
    ),
    // Remotes.
    (
        "remote.fetch",
        &[
            opt("remote", Text, message!("params-remote-fetch-remote")),
            opt("prune", Flag, message!("params-prune")),
            opt("tags", Flag, message!("params-all-tags")),
            opt("quiet", Flag, message!("params-quiet")),
            opt("reference", Text, message!("params-remote-fetch-reference")),
        ],
    ),
    (
        "remote.pull",
        &[opt(
            "mode",
            Choice(&["ff_only", "ff", "rebase", "merge"]),
            message!("params-pull-mode"),
        )],
    ),
    (
        "remote.push",
        &[
            opt("remote", Text, message!("params-push-remote")),
            opt("branch", Text, message!("params-push-branch")),
            opt(
                "set_upstream",
                Flag,
                message!("params-remote-push-set-upstream"),
            ),
            opt(
                "force",
                Choice(&["no", "lease", "force"]),
                message!("params-remote-push-force"),
            ),
            opt("tags", Flag, message!("params-all-tags")),
            opt("expect", Revision, message!("params-remote-push-expect")),
            opt("to", Text, message!("params-remote-push-to")),
            opt("no_verify", Flag, message!("params-remote-push-no-verify")),
        ],
    ),
    (
        "remote.push_preview",
        &[
            opt("remote", Text, message!("params-push-remote")),
            opt("branch", Text, message!("params-push-branch")),
        ],
    ),
    (
        "remote.add",
        &[
            req("name", Text, message!("params-remote-add-name")),
            req("url", Text, message!("params-remote-add-url")),
        ],
    ),
    (
        "remote.remove",
        &[req("name", Text, message!("params-remote"))],
    ),
    (
        "remote.set_url",
        &[
            req("name", Text, message!("params-remote")),
            req("url", Text, message!("params-remote-set-url-url")),
            opt("push", Flag, message!("params-remote-set-url-push")),
        ],
    ),
    // Stashes and tags.
    ("stash.list", NONE),
    (
        "stash.push",
        &[
            opt("message", Text, message!("params-stash-push-message")),
            opt(
                "include_untracked",
                Flag,
                message!("params-stash-push-include-untracked"),
            ),
            opt("keep_index", Flag, message!("params-stash-push-keep-index")),
            opt(
                "staged_only",
                Flag,
                message!("params-stash-push-staged-only"),
            ),
        ],
    ),
    (
        "stash.apply stash.pop",
        &[
            opt("stash", Revision, message!("params-stash")),
            opt("index", Flag, message!("params-stash-apply-index")),
        ],
    ),
    (
        "stash.show",
        &[
            opt("stash", Revision, message!("params-stash")),
            opt("context", Number, message!("params-stash-show-context")),
        ],
    ),
    (
        "stash.drop",
        &[opt("stash", Revision, message!("params-stash"))],
    ),
    (
        "stash.reword",
        &[
            opt("stash", Revision, message!("params-stash")),
            req("message", Text, message!("params-stash-reword-message")),
        ],
    ),
    (
        "stash.branch",
        &[
            req("name", Text, message!("params-new-branch-name")),
            opt("stash", Revision, message!("params-stash")),
        ],
    ),
    (
        "tag.create",
        &[
            req("name", Text, message!("params-tag-create-name")),
            opt("target", Revision, message!("params-tag-create-target")),
            opt("message", Text, message!("params-tag-create-message")),
            opt("sign", Flag, message!("params-tag-create-sign")),
        ],
    ),
    (
        "tag.delete tag.details",
        &[req("name", Text, message!("params-tag"))],
    ),
    (
        "tag.annotate",
        &[
            req("name", Text, message!("params-tag-annotate-name")),
            req("message", Text, message!("params-tag-annotate-message")),
        ],
    ),
    (
        "tag.push",
        &[
            opt("name", Text, message!("params-tag-push-name")),
            opt("remote", Text, message!("params-tag-push-remote")),
        ],
    ),
    (
        "tag.delete_remote",
        &[
            req("name", Text, message!("params-tag")),
            opt("remote", Text, message!("params-tag-delete-remote-remote")),
        ],
    ),
    // Operations in progress and conflicts.
    ("op.status op.continue op.skip op.abort conflict.list", NONE),
    (
        "conflict.take",
        &[
            req("path", Path, message!("params-file-in-conflict")),
            req(
                "side",
                Choice(&["ours", "theirs"]),
                message!("params-conflict-take-side"),
            ),
        ],
    ),
    (
        "conflict.resolve",
        &[
            req("path", Path, message!("params-file-in-conflict")),
            opt("allow_markers", Flag, message!("params-allow-markers")),
        ],
    ),
    (
        "conflict.detail conflict.reopen rerere.forget",
        &[req("path", Path, message!("params-file-in-worktree"))],
    ),
    (
        "conflict.write",
        &[
            req("path", Path, message!("params-file-in-conflict")),
            req("text", Text, message!("params-conflict-write-text")),
            opt("allow_markers", Flag, message!("params-allow-markers")),
        ],
    ),
    ("rerere.status", NONE),
    ("conflict.resolved", NONE),
    (
        "rerere.records",
        &[opt(
            "limit",
            Number,
            message!("params-rerere-records-limit"),
        )],
    ),
    (
        "rerere.forget_record",
        &[req("id", Text, message!("params-rerere-forget-record-id"))],
    ),
    (
        "rerere.enable",
        &[
            req("on", Flag, message!("params-rerere-enable-on")),
            opt("scope", CONFIG_SCOPE, message!("params-config-scope")),
        ],
    ),
    // Interactive rebase.
    (
        "rebase.plan",
        &[
            opt("first", Revision, message!("params-rebase-plan-first")),
            opt("onto", Revision, message!("params-rebase-plan-onto")),
        ],
    ),
    (
        "rebase.outline",
        &[
            req("first", Revision, message!("params-plan-first")),
            req("steps", Structured, message!("params-plan-steps")),
        ],
    ),
    (
        "rebase.interactive",
        &[
            req("first", Revision, message!("params-plan-first")),
            req("steps", Structured, message!("params-plan-steps")),
            opt("onto", Revision, message!("params-rebase-interactive-onto")),
            opt("no_verify", Flag, message!("params-skip-rebase-hook")),
        ],
    ),
    // External tools.
    ("tool.list", NONE),
    (
        "tool.known",
        &[opt("all", Flag, message!("params-tool-known-all"))],
    ),
    (
        "tool.diff",
        &[
            req("path", Path, message!("params-file-in-worktree")),
            opt("staged", Flag, message!("params-tool-diff-staged")),
            opt("commit", Revision, message!("params-tool-diff-commit")),
            opt("to", Revision, message!("params-tool-diff-to")),
            opt("from", Revision, message!("params-tool-diff-from")),
            opt("command", Text, message!("params-tool-diff-command")),
        ],
    ),
    (
        "tool.merge",
        &[
            req("path", Path, message!("params-file-in-conflict")),
            opt("command", Text, message!("params-tool-merge-command")),
        ],
    ),
    // Worktrees and submodules.
    (
        "worktree.list",
        &[opt(
            "changes",
            Flag,
            message!("params-worktree-list-changes"),
        )],
    ),
    (
        "worktree.changes",
        &[
            opt(
                "known",
                Structured,
                message!("params-worktree-changes-known"),
            ),
            opt("skip", List, message!("params-worktree-changes-skip")),
            opt(
                "at_most",
                Number,
                message!("params-worktree-changes-at-most"),
            ),
        ],
    ),
    (
        "worktree.add",
        &[
            req("path", Path, message!("params-worktree-add-path")),
            opt("branch", Text, message!("params-worktree-add-branch")),
            opt(
                "new_branch",
                Text,
                message!("params-worktree-add-new-branch"),
            ),
            opt("start", Revision, message!("params-worktree-add-start")),
            opt("detach", Flag, message!("params-worktree-add-detach")),
        ],
    ),
    (
        "worktree.remove",
        &[
            req("path", Path, message!("params-worktree")),
            opt("force", Flag, message!("params-worktree-remove-force")),
            opt(
                "delete_branch",
                Flag,
                message!("params-worktree-remove-delete-branch"),
            ),
        ],
    ),
    (
        "worktree.lock",
        &[
            req("path", Path, message!("params-worktree")),
            opt("reason", Text, message!("params-worktree-lock-reason")),
        ],
    ),
    (
        "worktree.move",
        &[
            req("path", Path, message!("params-worktree")),
            req("to", Path, message!("params-worktree-move-to")),
        ],
    ),
    (
        "worktree.unlock",
        &[req("path", Path, message!("params-worktree"))],
    ),
    ("worktree.prune submodule.list", NONE),
    (
        "submodule.add",
        &[
            req("url", Text, message!("params-submodule-add-url")),
            req("path", Path, message!("params-submodule-add-path")),
            opt("branch", Text, message!("params-submodule-add-branch")),
            opt(
                "allow_local",
                Flag,
                message!("params-submodule-allow-local"),
            ),
        ],
    ),
    (
        "submodule.update",
        &[
            opt("paths", List, message!("params-submodule-paths")),
            opt("init", Flag, message!("params-submodule-update-init")),
            opt("recursive", Flag, message!("params-submodule-recursive")),
            opt(
                "allow_local",
                Flag,
                message!("params-submodule-allow-local"),
            ),
        ],
    ),
    (
        "submodule.sync",
        &[
            opt("paths", List, message!("params-submodule-paths")),
            opt("recursive", Flag, message!("params-submodule-recursive")),
        ],
    ),
    (
        "submodule.remove",
        &[
            req("path", Path, message!("params-submodule")),
            opt("force", Flag, message!("params-submodule-remove-force")),
            opt("purge", Flag, message!("params-submodule-remove-purge")),
        ],
    ),
    (
        "submodule.forget",
        &[req("name", Text, message!("params-submodule-forget-name"))],
    ),
    (
        "submodule.changes",
        &[
            req("path", Path, message!("params-submodule")),
            opt("old", Revision, SIDES),
            opt("new", Revision, SIDES),
        ],
    ),
    // Activity, hooks.
    (
        "activity.list",
        &[
            opt("limit", Number, message!("params-activity-list-limit")),
            opt(
                "questions",
                Flag,
                message!("params-activity-list-questions"),
            ),
            opt("text", Text, message!("params-activity-list-text")),
        ],
    ),
    ("activity.clear hook.list", NONE),
    // Gitflow.
    ("flow.status", NONE),
    (
        "flow.init",
        &[
            opt("main", Text, message!("params-flow-init-main")),
            opt("develop", Text, message!("params-flow-init-develop")),
            opt("feature", Text, message!("params-flow-init-feature")),
            opt("bugfix", Text, message!("params-flow-init-bugfix")),
            opt("release", Text, message!("params-flow-init-release")),
            opt("hotfix", Text, message!("params-flow-init-hotfix")),
            opt("support", Text, message!("params-flow-init-support")),
            opt("tag", Text, message!("params-flow-init-tag")),
        ],
    ),
    (
        "flow.start",
        &[
            req("kind", FLOW_KIND, message!("params-flow-start-kind")),
            req("name", Text, message!("params-flow-name")),
            opt("base", Revision, message!("params-flow-start-base")),
        ],
    ),
    (
        "flow.finish",
        &[
            req("kind", FLOW_FINISHED, message!("params-flow-finish-kind")),
            req("name", Text, message!("params-flow-name")),
            opt("message", Text, message!("params-flow-finish-message")),
            opt("keep", Flag, message!("params-flow-finish-keep")),
        ],
    ),
    // Signing.
    ("sign.status", NONE),
    (
        "sign.keys",
        &[
            opt("ssh_dir", Path, message!("params-sign-keys-ssh-dir")),
            opt("agent", Flag, message!("params-sign-keys-agent")),
        ],
    ),
    (
        "sign.configure",
        &[
            opt(
                "format",
                Choice(&["openpgp", "ssh", "x509"]),
                message!("params-sign-configure-format"),
            ),
            opt("key", Text, message!("params-sign-configure-key")),
            opt("commits", Flag, message!("params-sign-configure-commits")),
            opt("tags", Flag, message!("params-sign-configure-tags")),
            opt("scope", CONFIG_SCOPE, message!("params-config-scope")),
        ],
    ),
    (
        "sign.verify",
        &[req("ids", List, message!("params-sign-verify-ids"))],
    ),
    (
        "sign.signed",
        &[req("ids", List, message!("params-commit-ids"))],
    ),
    ("sign.signers", NONE),
    (
        "sign.signers_file",
        &[
            opt("path", Path, message!("params-sign-signers-file-path")),
            opt(
                "scope",
                Structured,
                message!("params-sign-signers-file-scope"),
            ),
        ],
    ),
    (
        "sign.allow",
        &[
            opt("principals", List, message!("params-sign-allow-principals")),
            opt("key", Text, message!("params-sign-allow-key")),
            opt("key_file", Path, message!("params-sign-allow-key-file")),
            opt("commit", Revision, message!("params-sign-allow-commit")),
        ],
    ),
    (
        "sign.disallow",
        &[
            req("key", Text, message!("params-sign-disallow-key")),
            opt(
                "principals",
                List,
                message!("params-sign-disallow-principals"),
            ),
        ],
    ),
    // Profiles and links.
    ("profile.list", NONE),
    (
        "profile.save",
        &[
            req(
                "profile",
                Structured,
                message!("params-profile-save-profile"),
            ),
            opt(
                "rename_from",
                Text,
                message!("params-profile-save-rename-from"),
            ),
        ],
    ),
    (
        "profile.delete",
        &[req("name", Text, message!("params-profile"))],
    ),
    (
        "profile.current",
        &[opt(
            "scope",
            Structured,
            message!("params-profile-current-scope"),
        )],
    ),
    (
        "profile.use",
        &[
            req("name", Text, message!("params-profile")),
            req("scope", Structured, message!("params-profile-use-scope")),
        ],
    ),
    // Workspaces.
    (
        "workspace.list",
        &[opt("id", Text, message!("params-workspace-list-id"))],
    ),
    (
        "workspace.create",
        &[
            req("name", Text, message!("params-workspace-create-name")),
            opt("colour", Text, message!("params-workspace-create-colour")),
            opt("icon", Text, message!("params-workspace-create-icon")),
            opt(
                "description",
                Text,
                message!("params-workspace-description"),
            ),
            opt(
                "default_profile",
                Text,
                message!("params-workspace-default-profile"),
            ),
            opt(
                "follows_folder",
                Path,
                message!("params-workspace-create-follows-folder"),
            ),
            WORKSPACE_LAYOUT,
            WORKSPACE_PATHS,
            WORKSPACE_REPOSITORIES,
        ],
    ),
    (
        "workspace.update",
        &[
            WORKSPACE_ID,
            opt("name", Text, message!("params-workspace-update-name")),
            opt("colour", Text, message!("params-workspace-update-colour")),
            opt("icon", Text, message!("params-workspace-update-icon")),
            opt(
                "description",
                Text,
                message!("params-workspace-description"),
            ),
            opt(
                "default_profile",
                Text,
                message!("params-workspace-default-profile"),
            ),
            opt(
                "follows_folder",
                Path,
                message!("params-workspace-update-follows-folder"),
            ),
            WORKSPACE_LAYOUT,
        ],
    ),
    ("workspace.delete", &[WORKSPACE_ID]),
    (
        "workspace.reorder",
        &[req("ids", List, message!("params-workspace-reorder-ids"))],
    ),
    (
        "workspace.add",
        &[WORKSPACE_ID, WORKSPACE_PATHS, WORKSPACE_REPOSITORIES],
    ),
    (
        "workspace.remove",
        &[
            WORKSPACE_ID,
            req("paths", List, message!("params-workspace-remove-paths")),
        ],
    ),
    (
        "workspace.scan",
        &[
            req("path", Path, message!("params-workspace-scan-path")),
            opt("depth", Number, message!("params-workspace-scan-depth")),
            opt("limit", Number, message!("params-workspace-scan-limit")),
        ],
    ),
    ("workspace.sync_folder", &[WORKSPACE_ID]),
    (
        "workspace.import_code_workspace",
        &[
            req(
                "file",
                Path,
                message!("params-workspace-import-code-workspace-file"),
            ),
            opt(
                "id",
                Text,
                message!("params-workspace-import-code-workspace-id"),
            ),
            opt(
                "name",
                Text,
                message!("params-workspace-import-code-workspace-name"),
            ),
        ],
    ),
    (
        "workspace.fetch",
        &[
            WORKSPACE_ID,
            WORKSPACE_CHOSEN,
            WORKSPACE_PARALLEL,
            opt("prune", Flag, message!("params-prune")),
            opt("tags", Flag, message!("params-all-tags")),
            opt("quiet", Flag, message!("params-quiet")),
        ],
    ),
    (
        "workspace.pull",
        &[
            WORKSPACE_ID,
            WORKSPACE_CHOSEN,
            WORKSPACE_PARALLEL,
            opt(
                "mode",
                Choice(&["ff_only", "ff", "rebase", "merge"]),
                message!("params-pull-mode"),
            ),
        ],
    ),
    (
        "workspace.clone_missing",
        &[
            WORKSPACE_ID,
            WORKSPACE_CHOSEN,
            WORKSPACE_PARALLEL,
            opt(
                "folder",
                Path,
                message!("params-workspace-clone-missing-folder"),
            ),
        ],
    ),
    (
        "workspace.locate",
        &[
            WORKSPACE_ID,
            req("path", Path, message!("params-workspace-locate-path")),
            req("to", Path, message!("params-workspace-locate-to")),
        ],
    ),
    (
        "workspace.defaults",
        &[
            WORKSPACE_ID,
            req("path", Path, message!("params-workspace-defaults-path")),
            opt(
                "apply_profile",
                Flag,
                message!("params-workspace-defaults-apply-profile"),
            ),
        ],
    ),
    (
        "workspace.export",
        &[
            WORKSPACE_ID,
            req("file", Path, message!("params-workspace-export-file")),
            opt(
                "overwrite",
                Flag,
                message!("params-workspace-export-overwrite"),
            ),
        ],
    ),
    (
        "workspace.import",
        &[
            req("file", Path, message!("params-workspace-import-file")),
            opt("folder", Path, message!("params-workspace-import-folder")),
            opt("name", Text, message!("params-workspace-import-name")),
        ],
    ),
    (
        "link.parse",
        &[req("url", Text, message!("params-link-parse-url"))],
    ),
    (
        "link.make",
        &[
            opt("path", Path, message!("params-link-make-path")),
            opt("url", Text, message!("params-link-make-url")),
            opt("branch", Text, message!("params-link-make-branch")),
            opt("tag", Text, message!("params-link-make-tag")),
            opt("commit", Revision, message!("params-link-make-commit")),
        ],
    ),
    (
        "link.register",
        &[
            opt("exe", Path, message!("params-link-register-exe")),
            opt(
                "applications_dir",
                Path,
                message!("params-link-register-applications-dir"),
            ),
            opt("dry_run", Flag, message!("params-link-register-dry-run")),
            opt("no_mime", Flag, message!("params-link-register-no-mime")),
        ],
    ),
    // Patches.
    (
        "patch.export",
        &[
            opt("commits", List, message!("params-patch-export-commits")),
            opt("from", Revision, message!("params-patch-export-from")),
            opt("to", Revision, message!("params-patch-export-to")),
            opt("staged", Flag, message!("params-patch-export-staged")),
            opt("worktree", Flag, message!("params-patch-export-worktree")),
            opt("dir", Path, message!("params-patch-export-dir")),
            opt("file", Path, message!("params-patch-export-file")),
            opt("overwrite", Flag, message!("params-patch-export-overwrite")),
        ],
    ),
    (
        "patch.check patch.apply",
        &[
            req("file", Path, message!("params-patch-check-file")),
            opt("index", Flag, message!("params-patch-check-index")),
            opt("three_way", Flag, message!("params-patch-check-three-way")),
            opt("reverse", Flag, message!("params-patch-check-reverse")),
        ],
    ),
    (
        "patch.mailbox",
        &[
            req("file", Path, message!("params-patch-mailbox-file")),
            opt(
                "three_way",
                Flag,
                message!("params-patch-mailbox-three-way"),
            ),
            opt(
                "no_verify",
                Flag,
                message!("params-patch-mailbox-no-verify"),
            ),
        ],
    ),
    // Large files.
    ("lfs.status lfs.install lfs.locks lfs.checkout", NONE),
    (
        "lfs.track",
        &[
            req("pattern", Text, message!("params-lfs-track-pattern")),
            opt("lockable", Flag, message!("params-lfs-track-lockable")),
        ],
    ),
    (
        "lfs.untrack",
        &[req("pattern", Text, message!("params-lfs-untrack-pattern"))],
    ),
    (
        "lfs.files",
        &[opt("rev", Revision, message!("params-lfs-files-rev"))],
    ),
    (
        "lfs.fetch lfs.push",
        &[
            opt("remote", Text, message!("params-lfs-remote")),
            opt("all", Flag, message!("params-lfs-fetch-all")),
        ],
    ),
    (
        "lfs.pull",
        &[opt("remote", Text, message!("params-lfs-remote"))],
    ),
    (
        "lfs.lock",
        &[req("path", Path, message!("params-lfs-lock-path"))],
    ),
    (
        "lfs.unlock",
        &[
            req("path", Path, message!("params-lfs-unlock-path")),
            opt("force", Flag, message!("params-lfs-unlock-force")),
        ],
    ),
    // Maintenance.
    (
        "repo.maintenance",
        &[req(
            "task",
            Choice(&["gc", "gc_aggressive", "prune", "repack", "commit_graph"]),
            message!("params-repo-maintenance-task"),
        )],
    ),
    ("repo.fsck", NONE),
    (
        "repo.size",
        &[
            opt("blobs", Number, message!("params-repo-size-blobs")),
            opt("commits", Number, message!("params-repo-size-commits")),
        ],
    ),
    // Sparse checkout.
    ("sparse.status sparse.disable", NONE),
    (
        "sparse.set sparse.add",
        &[
            opt(
                "directories",
                List,
                message!("params-sparse-set-directories"),
            ),
            opt("patterns", List, message!("params-sparse-set-patterns")),
        ],
    ),
    // Windows specifics.
    ("repo.long_paths repo.wsl", NONE),
    (
        "repo.long_paths_enable",
        &[opt("scope", CONFIG_SCOPE, message!("params-config-scope"))],
    ),
];

/// The parameters of the command with this id; none for an id nothing is
/// known about. A command that takes nothing has an empty schema.
pub fn schema(id: &str) -> Option<&'static [ParamSpec]> {
    SCHEMAS
        .iter()
        .find(|(ids, _)| ids.split(' ').any(|known| known == id))
        .map(|(_, params)| *params)
}

/// Every command id the table has a row for.
pub fn described() -> impl Iterator<Item = &'static str> {
    SCHEMAS.iter().flat_map(|(ids, _)| ids.split(' '))
}

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

/// What "yes" and "no" are typed as.
fn flag(text: &str) -> Option<bool> {
    match text.trim().to_lowercase().as_str() {
        "yes" | "y" | "true" | "on" | "1" => Some(true),
        "no" | "n" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// The parameters object for what was typed, one text per parameter: what
/// a prompt hands over. An empty text leaves an optional parameter out and
/// is refused for a required one. Flags take yes and no; a list takes its
/// items separated by commas or line breaks; something structured takes
/// JSON, or a bare word. Names the schema does not have are refused.
pub fn from_text(
    schema: &[ParamSpec],
    typed: &BTreeMap<String, String>,
) -> Result<Params, CommandError> {
    for name in typed.keys() {
        if !schema.iter().any(|param| param.name == name) {
            return Err(invalid(name, say!("params-no-such-parameter")));
        }
    }
    let mut out = serde_json::Map::new();
    for param in schema {
        let text = typed.get(param.name).map(String::as_str).unwrap_or("");
        let trimmed = text.trim();
        if trimmed.is_empty() {
            if param.required {
                return Err(invalid(param.name, say!("params-missing")));
            }
            continue;
        }
        let value = match param.kind {
            // A message keeps its line breaks and its indentation.
            Text => serde_json::Value::String(text.trim_end().to_owned()),
            Path | Revision => serde_json::Value::String(trimmed.to_owned()),
            Flag => serde_json::Value::Bool(
                flag(trimmed).ok_or_else(|| invalid(param.name, say!("params-is-yes-or-no")))?,
            ),
            Number => match trimmed.parse::<i64>() {
                Ok(number) => serde_json::Value::from(number),
                Err(_) => match trimmed.parse::<u64>() {
                    Ok(number) => serde_json::Value::from(number),
                    Err(_) => return Err(invalid(param.name, say!("params-is-a-whole-number"))),
                },
            },
            List => serde_json::Value::Array(
                trimmed
                    .split([',', '\n'])
                    .map(str::trim)
                    .filter(|item| !item.is_empty())
                    .map(|item| serde_json::Value::String(item.to_owned()))
                    .collect(),
            ),
            Choice(choices) => {
                if !choices.contains(&trimmed) {
                    return Err(invalid(
                        param.name,
                        say!("params-is-one-of", choices = choices.join(", ")),
                    ));
                }
                serde_json::Value::String(trimmed.to_owned())
            }
            Structured => serde_json::from_str(trimmed)
                .unwrap_or_else(|_| serde_json::Value::String(trimmed.to_owned())),
        };
        out.insert(param.name.to_owned(), value);
    }
    Ok(serde_json::Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(name, text)| ((*name).to_owned(), (*text).to_owned()))
            .collect()
    }

    #[test]
    fn what_is_typed_becomes_parameters() {
        let schema = schema("branch.create").unwrap();
        let params = from_text(
            schema,
            &typed(&[
                ("name", "  topic/x "),
                ("checkout", "Yes"),
                ("dirty", "stash"),
            ]),
        )
        .unwrap();
        assert_eq!(
            params,
            serde_json::json!({ "name": "  topic/x", "checkout": true, "dirty": "stash" })
        );
        // Left empty, an optional parameter is not sent at all.
        assert_eq!(
            from_text(schema, &typed(&[("name", "x"), ("start", "  ")])).unwrap(),
            serde_json::json!({ "name": "x" })
        );

        let picks = from_text(
            super::schema("commit.cherry_pick").unwrap(),
            &typed(&[("commits", "abc, def\n 123 ,,"), ("mainline", "2")]),
        )
        .unwrap();
        assert_eq!(
            picks,
            serde_json::json!({ "commits": ["abc", "def", "123"], "mainline": 2 })
        );

        // Something structured is JSON, or a word.
        let log = super::schema("repo.log").unwrap();
        assert_eq!(
            from_text(log, &typed(&[("tips", "head")])).unwrap(),
            serde_json::json!({ "tips": "head" })
        );
        assert_eq!(
            from_text(log, &typed(&[("tips", "{\"ids\": [\"abc\"]}")])).unwrap(),
            serde_json::json!({ "tips": { "ids": ["abc"] } })
        );
        // A commit message keeps its paragraphs.
        let message = from_text(
            super::schema("commit.create").unwrap(),
            &typed(&[("message", "summary\n\n  indented body\n")]),
        )
        .unwrap();
        assert_eq!(message["message"], "summary\n\n  indented body");
    }

    #[test]
    fn what_cannot_be_right_is_refused() {
        let schema = schema("branch.create").unwrap();
        let refused =
            |pairs: &[(&str, &str)]| from_text(schema, &typed(pairs)).unwrap_err().to_string();
        assert!(refused(&[]).contains("\"name\": missing"));
        assert!(refused(&[("name", "   ")]).contains("missing"));
        assert!(refused(&[("name", "x"), ("checkout", "maybe")]).contains("yes or no"));
        assert!(
            refused(&[("name", "x"), ("dirty", "burn")]).contains("one of refuse, stash, discard")
        );
        assert!(refused(&[("name", "x"), ("nonsense", "1")]).contains("no such parameter"));
        let number = |text: &str| {
            from_text(
                super::schema("repo.log").unwrap(),
                &typed(&[("limit", text)]),
            )
        };
        assert_eq!(number("12").unwrap(), serde_json::json!({ "limit": 12 }));
        assert_eq!(number("18446744073709551615").unwrap()["limit"], u64::MAX);
        for bad in [
            "twelve",
            "1.5",
            "1e9",
            "99999999999999999999999",
            "-",
            "0x10",
        ] {
            assert!(number(bad).is_err(), "{bad}");
        }
        assert_eq!(number("-3").unwrap(), serde_json::json!({ "limit": -3 }));
    }

    #[test]
    fn every_schema_is_well_formed() {
        // Ids are checked against the registry in `tests/suite/params.rs`;
        // here the rows themselves.
        for id in ["repo.log", "branch.create", "patch.export", "flow.finish"] {
            let schema = schema(id).unwrap();
            let mut names: Vec<&str> = schema.iter().map(|param| param.name).collect();
            names.sort_unstable();
            names.dedup();
            assert_eq!(names.len(), schema.len(), "{id} names a parameter twice");
        }
        assert_eq!(schema("no.such"), None);
        assert_eq!(ParamKind::Choice(&["a"]).name(), "choice");
        assert_eq!(ParamKind::Choice(&["a", "b"]).choices(), ["a", "b"]);
        assert!(ParamKind::Text.choices().is_empty());
    }
}
