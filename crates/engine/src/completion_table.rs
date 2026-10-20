//! What git's commands take, as the terminal's completion knows them:
//! the commands most often typed, in the order they are offered before
//! anything is typed, each with a few words of ours about it, its common
//! options, and what its other words are.
//!
//! The commands and options are git's own names, which are generic; the
//! descriptions are ours.
//!
//! Sources: git(1) (the list of commands and the global options), and
//! the manual page of each command named here (git-add(1), git-commit(1),
//! git-push(1) and so on) for the options and what their arguments are.

use bitbraid_strings::Message;

/// What an option takes after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Value {
    /// Nothing: a switch.
    None,
    /// Something that is not completed: a message, a name, a number.
    Text,
    /// Something that names a commit.
    Ref,
    /// A path of the working tree.
    Path,
    Remote,
}

/// An option of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Opt {
    /// The option as typed: `-m` or `--amend`.
    pub name: &'static str,
    /// What it does, in a few words.
    pub detail: Message,
    /// What follows it.
    pub value: Value,
}

/// Which paths are offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSet {
    /// What can be staged: changes not staged yet, new files, conflicts.
    Unstaged,
    /// What is staged.
    Staged,
    /// Files git does not track.
    Untracked,
    /// Every changed path first, then every tracked one.
    Tracked,
    /// The folders of tracked files.
    Folders,
}

/// What a command's other words are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Args {
    /// Nothing that can be completed.
    Nothing,
    /// Names of commits: branches, remote branches, tags.
    Refs,
    /// Names of commits, then paths after `--`.
    RefsThenPaths,
    /// Local branches.
    Branches,
    /// Branches to switch to: local ones, and remote ones by the name a
    /// local branch made from them gets.
    Switch,
    /// A new name first (not completed), then names of commits.
    NewThenRefs,
    Tags,
    Remotes,
    /// A remote, then local branches (what is sent).
    RemoteThenBranches,
    /// A remote, then that remote's branches (what is brought in).
    RemoteThenTheirs,
    Stashes,
    Paths(PathSet),
    /// Git's own commands.
    Commands,
    /// Words of the command's own, each with what follows it.
    Words(&'static [Command]),
}

/// A command of git's, or a word of one (`stash pop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Command {
    pub name: &'static str,
    /// What it does, in a few words.
    pub detail: Message,
    /// What its other words are.
    pub args: Args,
    /// Its common options.
    pub options: &'static [Opt],
}

const fn opt(name: &'static str, detail: Message) -> Opt {
    Opt {
        name,
        detail,
        value: Value::None,
    }
}

const fn takes(name: &'static str, detail: Message, value: Value) -> Opt {
    Opt {
        name,
        detail,
        value,
    }
}

const fn command(
    name: &'static str,
    detail: Message,
    args: Args,
    options: &'static [Opt],
) -> Command {
    Command {
        name,
        detail,
        args,
        options,
    }
}

/// The options git itself takes before a command.
pub const GLOBAL_OPTIONS: &[Opt] = &[
    opt("--version", message!("completion-git-option-version")),
    opt("--help", message!("completion-git-option-help")),
    takes("-C", message!("completion-git-option-folder"), Value::Path),
    takes("-c", message!("completion-git-option-setting"), Value::Text),
    opt("--no-pager", message!("completion-git-option-no-pager")),
    opt("--paginate", message!("completion-git-option-paginate")),
];

/// Options that leave the repository the shell is in for another: what
/// is completed after them would be of the wrong one.
pub const ELSEWHERE: &[&str] = &["-C", "--git-dir", "--work-tree", "--namespace"];

const ADD: &[Opt] = &[
    opt("--all", message!("completion-add-option-all")),
    opt("-A", message!("completion-add-option-all")),
    opt("--update", message!("completion-add-option-update")),
    opt("-u", message!("completion-add-option-update")),
    opt("--patch", message!("completion-add-option-patch")),
    opt("-p", message!("completion-add-option-patch")),
    opt(
        "--interactive",
        message!("completion-add-option-interactive"),
    ),
    opt("--force", message!("completion-add-option-force")),
    opt("--dry-run", message!("completion-add-option-dry-run")),
    opt(
        "--intent-to-add",
        message!("completion-add-option-intent-to-add"),
    ),
];

const COMMIT: &[Opt] = &[
    takes("-m", message!("completion-option-message"), Value::Text),
    takes(
        "--message",
        message!("completion-option-message"),
        Value::Text,
    ),
    opt("--amend", message!("completion-commit-option-amend")),
    opt("-a", message!("completion-commit-option-all")),
    opt("--all", message!("completion-commit-option-all")),
    opt("--no-edit", message!("completion-commit-option-no-edit")),
    opt("--signoff", message!("completion-commit-option-signoff")),
    opt("-S", message!("completion-commit-option-sign")),
    opt(
        "--no-verify",
        message!("completion-commit-option-no-verify"),
    ),
    takes(
        "--fixup",
        message!("completion-commit-option-fixup"),
        Value::Ref,
    ),
    opt(
        "--allow-empty",
        message!("completion-commit-option-allow-empty"),
    ),
    opt("--verbose", message!("completion-commit-option-verbose")),
    takes("-F", message!("completion-commit-option-file"), Value::Path),
    takes(
        "--author",
        message!("completion-commit-option-author"),
        Value::Text,
    ),
];

const PUSH: &[Opt] = &[
    opt("-u", message!("completion-push-option-set-upstream")),
    opt(
        "--set-upstream",
        message!("completion-push-option-set-upstream"),
    ),
    opt(
        "--force-with-lease",
        message!("completion-push-option-force-with-lease"),
    ),
    opt("--force", message!("completion-push-option-force")),
    opt("-f", message!("completion-push-option-force")),
    opt("--tags", message!("completion-push-option-tags")),
    opt(
        "--follow-tags",
        message!("completion-push-option-follow-tags"),
    ),
    opt("--all", message!("completion-push-option-all")),
    opt("--delete", message!("completion-push-option-delete")),
    opt("--dry-run", message!("completion-push-option-dry-run")),
    opt("--no-verify", message!("completion-push-option-no-verify")),
];

const PULL: &[Opt] = &[
    opt("--rebase", message!("completion-pull-option-rebase")),
    opt("--no-rebase", message!("completion-pull-option-no-rebase")),
    opt("--ff-only", message!("completion-option-ff-only")),
    opt("--autostash", message!("completion-option-autostash")),
    opt("--tags", message!("completion-pull-option-tags")),
    opt("--verbose", message!("completion-pull-option-verbose")),
];

const FETCH: &[Opt] = &[
    opt("--all", message!("completion-fetch-option-all")),
    opt("--prune", message!("completion-fetch-option-prune")),
    opt("-p", message!("completion-fetch-option-prune")),
    opt("--tags", message!("completion-fetch-option-tags")),
    takes(
        "--depth",
        message!("completion-fetch-option-depth"),
        Value::Text,
    ),
    opt("--unshallow", message!("completion-fetch-option-unshallow")),
];

const CHECKOUT: &[Opt] = &[
    takes("-b", message!("completion-option-new-branch"), Value::Text),
    takes(
        "-B",
        message!("completion-option-reset-branch"),
        Value::Text,
    ),
    opt("--detach", message!("completion-option-detach")),
    opt("--track", message!("completion-option-track")),
    opt("--force", message!("completion-option-discard-changes")),
    takes(
        "--orphan",
        message!("completion-option-orphan"),
        Value::Text,
    ),
    opt("--patch", message!("completion-option-patch")),
    opt("--ours", message!("completion-option-ours")),
    opt("--theirs", message!("completion-option-theirs")),
];

const SWITCH: &[Opt] = &[
    takes("-c", message!("completion-option-new-branch"), Value::Text),
    takes(
        "--create",
        message!("completion-option-new-branch"),
        Value::Text,
    ),
    takes(
        "-C",
        message!("completion-option-reset-branch"),
        Value::Text,
    ),
    opt("--detach", message!("completion-switch-option-detach")),
    opt(
        "--discard-changes",
        message!("completion-option-discard-changes"),
    ),
    opt("--track", message!("completion-option-track")),
    takes(
        "--orphan",
        message!("completion-option-orphan"),
        Value::Text,
    ),
];

const BRANCH: &[Opt] = &[
    opt("--all", message!("completion-branch-option-all")),
    opt("-a", message!("completion-branch-option-all")),
    opt("--remotes", message!("completion-branch-option-remotes")),
    opt("-r", message!("completion-branch-option-remotes")),
    opt("-d", message!("completion-branch-option-delete")),
    opt("--delete", message!("completion-branch-option-delete")),
    opt("-D", message!("completion-branch-option-delete-force")),
    opt("-m", message!("completion-branch-option-move")),
    opt("--move", message!("completion-branch-option-move")),
    opt("-M", message!("completion-branch-option-move-force")),
    opt("--copy", message!("completion-branch-option-copy")),
    takes(
        "--set-upstream-to",
        message!("completion-branch-option-set-upstream-to"),
        Value::Ref,
    ),
    takes(
        "-u",
        message!("completion-branch-option-set-upstream-to"),
        Value::Ref,
    ),
    opt(
        "--unset-upstream",
        message!("completion-branch-option-unset-upstream"),
    ),
    opt("--verbose", message!("completion-branch-option-verbose")),
    opt("-v", message!("completion-branch-option-verbose")),
    takes(
        "--merged",
        message!("completion-branch-option-merged"),
        Value::Ref,
    ),
    takes(
        "--no-merged",
        message!("completion-branch-option-no-merged"),
        Value::Ref,
    ),
    takes(
        "--contains",
        message!("completion-branch-option-contains"),
        Value::Ref,
    ),
    opt(
        "--show-current",
        message!("completion-branch-option-show-current"),
    ),
];

const MERGE: &[Opt] = &[
    opt("--no-ff", message!("completion-merge-option-no-ff")),
    opt("--ff-only", message!("completion-option-ff-only")),
    opt("--squash", message!("completion-merge-option-squash")),
    opt("--abort", message!("completion-merge-option-abort")),
    opt("--continue", message!("completion-option-continue")),
    takes("-m", message!("completion-option-message"), Value::Text),
    opt("--no-commit", message!("completion-merge-option-no-commit")),
    opt("--no-edit", message!("completion-option-no-edit")),
    takes(
        "--strategy",
        message!("completion-merge-option-strategy"),
        Value::Text,
    ),
];

const REBASE: &[Opt] = &[
    opt(
        "--interactive",
        message!("completion-rebase-option-interactive"),
    ),
    opt("-i", message!("completion-rebase-option-interactive")),
    takes(
        "--onto",
        message!("completion-rebase-option-onto"),
        Value::Ref,
    ),
    opt("--continue", message!("completion-option-continue")),
    opt("--abort", message!("completion-rebase-option-abort")),
    opt("--skip", message!("completion-option-skip")),
    opt(
        "--autosquash",
        message!("completion-rebase-option-autosquash"),
    ),
    opt("--autostash", message!("completion-option-autostash")),
    opt(
        "--update-refs",
        message!("completion-rebase-option-update-refs"),
    ),
    opt(
        "--rebase-merges",
        message!("completion-rebase-option-rebase-merges"),
    ),
];

const LOG: &[Opt] = &[
    opt("--oneline", message!("completion-log-option-oneline")),
    opt("--graph", message!("completion-log-option-graph")),
    opt("--all", message!("completion-log-option-all")),
    opt("--patch", message!("completion-log-option-patch")),
    opt("-p", message!("completion-log-option-patch")),
    opt("--stat", message!("completion-option-stat")),
    takes(
        "--author",
        message!("completion-log-option-author"),
        Value::Text,
    ),
    takes(
        "--since",
        message!("completion-log-option-since"),
        Value::Text,
    ),
    takes(
        "--until",
        message!("completion-log-option-until"),
        Value::Text,
    ),
    takes(
        "--grep",
        message!("completion-log-option-grep"),
        Value::Text,
    ),
    takes(
        "-n",
        message!("completion-log-option-max-count"),
        Value::Text,
    ),
    opt("--follow", message!("completion-log-option-follow")),
    opt(
        "--first-parent",
        message!("completion-log-option-first-parent"),
    ),
    opt("--reverse", message!("completion-log-option-reverse")),
    opt("--decorate", message!("completion-log-option-decorate")),
];

const DIFF: &[Opt] = &[
    opt("--staged", message!("completion-diff-option-staged")),
    opt("--cached", message!("completion-diff-option-staged")),
    opt("--stat", message!("completion-option-stat")),
    opt("--name-only", message!("completion-option-name-only")),
    opt(
        "--name-status",
        message!("completion-diff-option-name-status"),
    ),
    opt("--word-diff", message!("completion-diff-option-word-diff")),
    opt(
        "--ignore-all-space",
        message!("completion-option-ignore-all-space"),
    ),
    opt("-w", message!("completion-option-ignore-all-space")),
    opt("--check", message!("completion-diff-option-check")),
    opt("--no-index", message!("completion-diff-option-no-index")),
];

const SHOW: &[Opt] = &[
    opt("--stat", message!("completion-option-stat")),
    opt("--name-only", message!("completion-option-name-only")),
    opt("--oneline", message!("completion-show-option-oneline")),
    opt("--no-patch", message!("completion-show-option-no-patch")),
];

const STASH_PUSH: &[Opt] = &[
    opt(
        "--include-untracked",
        message!("completion-stash-push-option-include-untracked"),
    ),
    opt(
        "-u",
        message!("completion-stash-push-option-include-untracked"),
    ),
    opt("--all", message!("completion-stash-push-option-all")),
    opt(
        "--keep-index",
        message!("completion-stash-push-option-keep-index"),
    ),
    opt("--staged", message!("completion-stash-push-option-staged")),
    takes(
        "-m",
        message!("completion-stash-push-option-message"),
        Value::Text,
    ),
    takes(
        "--message",
        message!("completion-stash-push-option-message"),
        Value::Text,
    ),
    opt("--patch", message!("completion-stash-push-option-patch")),
];

const STASH_APPLY: &[Opt] = &[opt(
    "--index",
    message!("completion-stash-apply-option-index"),
)];

const STASH_SHOW: &[Opt] = &[opt(
    "--patch",
    message!("completion-stash-show-option-patch"),
)];

const STASH: &[Command] = &[
    command(
        "push",
        message!("completion-stash-push"),
        Args::Paths(PathSet::Unstaged),
        STASH_PUSH,
    ),
    command(
        "pop",
        message!("completion-stash-pop"),
        Args::Stashes,
        STASH_APPLY,
    ),
    command(
        "apply",
        message!("completion-stash-apply"),
        Args::Stashes,
        STASH_APPLY,
    ),
    command(
        "list",
        message!("completion-stash-list"),
        Args::Nothing,
        &[],
    ),
    command(
        "show",
        message!("completion-stash-show"),
        Args::Stashes,
        STASH_SHOW,
    ),
    command(
        "drop",
        message!("completion-stash-drop"),
        Args::Stashes,
        &[],
    ),
    command(
        "branch",
        message!("completion-stash-branch"),
        Args::Nothing,
        &[],
    ),
    command(
        "clear",
        message!("completion-stash-clear"),
        Args::Nothing,
        &[],
    ),
];

const RESET: &[Opt] = &[
    opt("--soft", message!("completion-reset-option-soft")),
    opt("--mixed", message!("completion-reset-option-mixed")),
    opt("--hard", message!("completion-reset-option-hard")),
    opt("--keep", message!("completion-reset-option-keep")),
    opt("--patch", message!("completion-reset-option-patch")),
];

const RESTORE: &[Opt] = &[
    takes(
        "--source",
        message!("completion-restore-option-source"),
        Value::Ref,
    ),
    opt("--staged", message!("completion-restore-option-staged")),
    opt("-S", message!("completion-restore-option-staged")),
    opt("--worktree", message!("completion-restore-option-worktree")),
    opt("--patch", message!("completion-option-patch")),
    opt("--ours", message!("completion-option-ours")),
    opt("--theirs", message!("completion-option-theirs")),
];

const TAG: &[Opt] = &[
    opt("--annotate", message!("completion-tag-option-annotate")),
    opt("-a", message!("completion-tag-option-annotate")),
    takes("-m", message!("completion-tag-option-message"), Value::Text),
    opt("--delete", message!("completion-tag-option-delete")),
    opt("-d", message!("completion-tag-option-delete")),
    opt("--sign", message!("completion-tag-option-sign")),
    opt("--list", message!("completion-tag-option-list")),
    opt("--force", message!("completion-tag-option-force")),
    opt("--verify", message!("completion-tag-option-verify")),
];

const REMOTE: &[Command] = &[
    command("add", message!("completion-remote-add"), Args::Nothing, &[]),
    command(
        "remove",
        message!("completion-remote-remove"),
        Args::Remotes,
        &[],
    ),
    command(
        "rename",
        message!("completion-remote-rename"),
        Args::Remotes,
        &[],
    ),
    command(
        "set-url",
        message!("completion-remote-set-url"),
        Args::Remotes,
        &[],
    ),
    command(
        "get-url",
        message!("completion-remote-get-url"),
        Args::Remotes,
        &[],
    ),
    command(
        "show",
        message!("completion-remote-show"),
        Args::Remotes,
        &[],
    ),
    command(
        "prune",
        message!("completion-remote-prune"),
        Args::Remotes,
        &[],
    ),
    command(
        "update",
        message!("completion-remote-update"),
        Args::Remotes,
        &[],
    ),
];

const CHERRY_PICK: &[Opt] = &[
    opt(
        "--no-commit",
        message!("completion-cherry-pick-option-no-commit"),
    ),
    opt("-x", message!("completion-cherry-pick-option-note-origin")),
    opt("--edit", message!("completion-cherry-pick-option-edit")),
    opt("--continue", message!("completion-option-continue")),
    opt("--abort", message!("completion-option-abort")),
    opt("--skip", message!("completion-option-skip")),
    takes(
        "--mainline",
        message!("completion-option-mainline"),
        Value::Text,
    ),
];

const REVERT: &[Opt] = &[
    opt("--no-edit", message!("completion-option-no-edit")),
    opt(
        "--no-commit",
        message!("completion-revert-option-no-commit"),
    ),
    opt("--continue", message!("completion-option-continue")),
    opt("--abort", message!("completion-option-abort")),
    takes(
        "--mainline",
        message!("completion-option-mainline"),
        Value::Text,
    ),
];

const CLONE: &[Opt] = &[
    takes(
        "--depth",
        message!("completion-clone-option-depth"),
        Value::Text,
    ),
    takes(
        "--branch",
        message!("completion-clone-option-branch"),
        Value::Text,
    ),
    opt(
        "--recurse-submodules",
        message!("completion-clone-option-recurse-submodules"),
    ),
    opt("--bare", message!("completion-clone-option-bare")),
    takes(
        "--filter",
        message!("completion-clone-option-filter"),
        Value::Text,
    ),
    opt(
        "--single-branch",
        message!("completion-clone-option-single-branch"),
    ),
];

const INIT: &[Opt] = &[
    opt("--bare", message!("completion-init-option-bare")),
    takes(
        "--initial-branch",
        message!("completion-init-option-initial-branch"),
        Value::Text,
    ),
];

const CONFIG: &[Opt] = &[
    opt("--global", message!("completion-config-option-global")),
    opt("--local", message!("completion-config-option-local")),
    opt("--list", message!("completion-config-option-list")),
    opt(
        "--show-origin",
        message!("completion-config-option-show-origin"),
    ),
    opt("--edit", message!("completion-config-option-edit")),
    takes(
        "--unset",
        message!("completion-config-option-unset"),
        Value::Text,
    ),
];

const WORKTREE_ADD: &[Opt] = &[
    takes(
        "-b",
        message!("completion-worktree-add-option-new-branch"),
        Value::Text,
    ),
    opt("--detach", message!("completion-option-detach")),
];

const WORKTREE_REMOVE: &[Opt] = &[opt(
    "--force",
    message!("completion-worktree-remove-option-force"),
)];

const WORKTREE: &[Command] = &[
    command(
        "add",
        message!("completion-worktree-add"),
        Args::NewThenRefs,
        WORKTREE_ADD,
    ),
    command(
        "list",
        message!("completion-worktree-list"),
        Args::Nothing,
        &[],
    ),
    command(
        "remove",
        message!("completion-worktree-remove"),
        Args::Nothing,
        WORKTREE_REMOVE,
    ),
    command(
        "move",
        message!("completion-worktree-move"),
        Args::Nothing,
        &[],
    ),
    command(
        "lock",
        message!("completion-worktree-lock"),
        Args::Nothing,
        &[],
    ),
    command(
        "unlock",
        message!("completion-worktree-unlock"),
        Args::Nothing,
        &[],
    ),
    command(
        "prune",
        message!("completion-worktree-prune"),
        Args::Nothing,
        &[],
    ),
    command(
        "repair",
        message!("completion-worktree-repair"),
        Args::Nothing,
        &[],
    ),
];

const SUBMODULE_UPDATE: &[Opt] = &[
    opt(
        "--init",
        message!("completion-submodule-update-option-init"),
    ),
    opt(
        "--recursive",
        message!("completion-submodule-update-option-recursive"),
    ),
    opt(
        "--remote",
        message!("completion-submodule-update-option-remote"),
    ),
];

const SUBMODULE: &[Command] = &[
    command(
        "status",
        message!("completion-submodule-status"),
        Args::Nothing,
        &[],
    ),
    command(
        "update",
        message!("completion-submodule-update"),
        Args::Nothing,
        SUBMODULE_UPDATE,
    ),
    command(
        "init",
        message!("completion-submodule-init"),
        Args::Nothing,
        &[],
    ),
    command(
        "add",
        message!("completion-submodule-add"),
        Args::Nothing,
        &[],
    ),
    command(
        "deinit",
        message!("completion-submodule-deinit"),
        Args::Nothing,
        &[],
    ),
    command(
        "sync",
        message!("completion-submodule-sync"),
        Args::Nothing,
        &[],
    ),
    command(
        "foreach",
        message!("completion-submodule-foreach"),
        Args::Nothing,
        &[],
    ),
    command(
        "summary",
        message!("completion-submodule-summary"),
        Args::Nothing,
        &[],
    ),
];

const BISECT: &[Command] = &[
    command(
        "start",
        message!("completion-bisect-start"),
        Args::Refs,
        &[],
    ),
    command("good", message!("completion-bisect-good"), Args::Refs, &[]),
    command("bad", message!("completion-bisect-bad"), Args::Refs, &[]),
    command("skip", message!("completion-bisect-skip"), Args::Refs, &[]),
    command(
        "reset",
        message!("completion-bisect-reset"),
        Args::Refs,
        &[],
    ),
    command("log", message!("completion-bisect-log"), Args::Nothing, &[]),
    command("run", message!("completion-bisect-run"), Args::Nothing, &[]),
];

const SPARSE_SET: &[Opt] = &[
    opt(
        "--cone",
        message!("completion-sparse-checkout-set-option-cone"),
    ),
    opt(
        "--no-cone",
        message!("completion-sparse-checkout-set-option-no-cone"),
    ),
];

const SPARSE: &[Command] = &[
    command(
        "set",
        message!("completion-sparse-checkout-set"),
        Args::Paths(PathSet::Folders),
        SPARSE_SET,
    ),
    command(
        "add",
        message!("completion-sparse-checkout-add"),
        Args::Paths(PathSet::Folders),
        &[],
    ),
    command(
        "list",
        message!("completion-sparse-checkout-list"),
        Args::Nothing,
        &[],
    ),
    command(
        "disable",
        message!("completion-sparse-checkout-disable"),
        Args::Nothing,
        &[],
    ),
    command(
        "reapply",
        message!("completion-sparse-checkout-reapply"),
        Args::Nothing,
        &[],
    ),
];

const MAINTENANCE_RUN: &[Opt] = &[takes(
    "--task",
    message!("completion-maintenance-run-option-task"),
    Value::Text,
)];

const MAINTENANCE: &[Command] = &[
    command(
        "run",
        message!("completion-maintenance-run"),
        Args::Nothing,
        MAINTENANCE_RUN,
    ),
    command(
        "start",
        message!("completion-maintenance-start"),
        Args::Nothing,
        &[],
    ),
    command(
        "stop",
        message!("completion-maintenance-stop"),
        Args::Nothing,
        &[],
    ),
];

const NOTES: &[Command] = &[
    command("add", message!("completion-notes-add"), Args::Refs, &[]),
    command("show", message!("completion-notes-show"), Args::Refs, &[]),
    command(
        "list",
        message!("completion-notes-list"),
        Args::Nothing,
        &[],
    ),
    command("edit", message!("completion-notes-edit"), Args::Refs, &[]),
    command(
        "remove",
        message!("completion-notes-remove"),
        Args::Refs,
        &[],
    ),
];

const CLEAN: &[Opt] = &[
    opt("--dry-run", message!("completion-clean-option-dry-run")),
    opt("-n", message!("completion-clean-option-dry-run")),
    opt("--force", message!("completion-clean-option-force")),
    opt("-f", message!("completion-clean-option-force")),
    opt("-d", message!("completion-clean-option-folders")),
    opt("-x", message!("completion-clean-option-ignored")),
    opt(
        "--interactive",
        message!("completion-clean-option-interactive"),
    ),
];

const RM: &[Opt] = &[
    opt("--cached", message!("completion-rm-option-cached")),
    opt("-r", message!("completion-rm-option-recursive")),
    opt("--force", message!("completion-rm-option-force")),
    opt("--dry-run", message!("completion-rm-option-dry-run")),
];

const BLAME: &[Opt] = &[
    takes("-L", message!("completion-blame-option-lines"), Value::Text),
    opt("-w", message!("completion-option-ignore-all-space")),
    opt(
        "--show-email",
        message!("completion-blame-option-show-email"),
    ),
];

const GREP: &[Opt] = &[
    opt(
        "--ignore-case",
        message!("completion-grep-option-ignore-case"),
    ),
    opt(
        "--line-number",
        message!("completion-grep-option-line-number"),
    ),
    opt(
        "--word-regexp",
        message!("completion-grep-option-word-regexp"),
    ),
    opt(
        "--files-with-matches",
        message!("completion-option-name-only"),
    ),
    opt("--cached", message!("completion-grep-option-cached")),
];

const DESCRIBE: &[Opt] = &[
    opt("--tags", message!("completion-describe-option-tags")),
    opt("--all", message!("completion-describe-option-all")),
    opt("--always", message!("completion-describe-option-always")),
    opt("--dirty", message!("completion-describe-option-dirty")),
];

const SHORTLOG: &[Opt] = &[
    opt("--summary", message!("completion-shortlog-option-summary")),
    opt(
        "--numbered",
        message!("completion-shortlog-option-numbered"),
    ),
    opt("--email", message!("completion-shortlog-option-email")),
];

const ARCHIVE: &[Opt] = &[
    takes(
        "--format",
        message!("completion-archive-option-format"),
        Value::Text,
    ),
    takes(
        "--output",
        message!("completion-archive-option-output"),
        Value::Text,
    ),
    takes(
        "--prefix",
        message!("completion-archive-option-prefix"),
        Value::Text,
    ),
];

const FORMAT_PATCH: &[Opt] = &[
    opt(
        "--stdout",
        message!("completion-format-patch-option-stdout"),
    ),
    opt(
        "--cover-letter",
        message!("completion-format-patch-option-cover-letter"),
    ),
    takes(
        "-o",
        message!("completion-format-patch-option-output-directory"),
        Value::Text,
    ),
];

const AM: &[Opt] = &[
    opt("--continue", message!("completion-option-continue")),
    opt("--skip", message!("completion-am-option-skip")),
    opt("--abort", message!("completion-option-abort")),
    opt("--3way", message!("completion-am-option-3way")),
];

const APPLY: &[Opt] = &[
    opt("--check", message!("completion-apply-option-check")),
    opt("--stat", message!("completion-apply-option-stat")),
    opt("--cached", message!("completion-apply-option-cached")),
    opt("--reverse", message!("completion-apply-option-reverse")),
    opt("--3way", message!("completion-apply-option-3way")),
];

const GC: &[Opt] = &[
    opt("--aggressive", message!("completion-gc-option-aggressive")),
    opt("--auto", message!("completion-gc-option-auto")),
];

const STATUS: &[Opt] = &[
    opt("--short", message!("completion-status-option-short")),
    opt("-s", message!("completion-status-option-short")),
    opt("--branch", message!("completion-status-option-branch")),
    opt("--ignored", message!("completion-status-option-ignored")),
    takes(
        "--untracked-files",
        message!("completion-status-option-untracked-files"),
        Value::Text,
    ),
];

const HELP: &[Opt] = &[
    opt("--all", message!("completion-help-option-all")),
    opt("--guides", message!("completion-help-option-guides")),
];

const REMOTE_OPTIONS: &[Opt] = &[
    opt("--verbose", message!("completion-remote-option-verbose")),
    opt("-v", message!("completion-remote-option-verbose")),
];

const MV: &[Opt] = &[opt("--force", message!("completion-mv-option-force"))];

/// The commands, in the order they are offered before anything is typed.
pub const COMMANDS: &[Command] = &[
    command(
        "status",
        message!("completion-command-status"),
        Args::Nothing,
        STATUS,
    ),
    command(
        "add",
        message!("completion-command-add"),
        Args::Paths(PathSet::Unstaged),
        ADD,
    ),
    command(
        "commit",
        message!("completion-command-commit"),
        Args::Paths(PathSet::Tracked),
        COMMIT,
    ),
    command(
        "push",
        message!("completion-command-push"),
        Args::RemoteThenBranches,
        PUSH,
    ),
    command(
        "pull",
        message!("completion-command-pull"),
        Args::RemoteThenTheirs,
        PULL,
    ),
    command(
        "fetch",
        message!("completion-command-fetch"),
        Args::RemoteThenTheirs,
        FETCH,
    ),
    command(
        "checkout",
        message!("completion-command-checkout"),
        Args::RefsThenPaths,
        CHECKOUT,
    ),
    command(
        "switch",
        message!("completion-command-switch"),
        Args::Switch,
        SWITCH,
    ),
    command(
        "branch",
        message!("completion-command-branch"),
        Args::NewThenRefs,
        BRANCH,
    ),
    command(
        "merge",
        message!("completion-command-merge"),
        Args::Refs,
        MERGE,
    ),
    command(
        "rebase",
        message!("completion-command-rebase"),
        Args::Refs,
        REBASE,
    ),
    command(
        "log",
        message!("completion-command-log"),
        Args::RefsThenPaths,
        LOG,
    ),
    command(
        "diff",
        message!("completion-command-diff"),
        Args::RefsThenPaths,
        DIFF,
    ),
    command(
        "show",
        message!("completion-command-show"),
        Args::RefsThenPaths,
        SHOW,
    ),
    command(
        "stash",
        message!("completion-command-stash"),
        Args::Words(STASH),
        STASH_PUSH,
    ),
    command(
        "reset",
        message!("completion-command-reset"),
        Args::RefsThenPaths,
        RESET,
    ),
    command(
        "restore",
        message!("completion-command-restore"),
        Args::Paths(PathSet::Unstaged),
        RESTORE,
    ),
    command(
        "tag",
        message!("completion-command-tag"),
        Args::NewThenRefs,
        TAG,
    ),
    command(
        "remote",
        message!("completion-command-remote"),
        Args::Words(REMOTE),
        REMOTE_OPTIONS,
    ),
    command(
        "cherry-pick",
        message!("completion-command-cherry-pick"),
        Args::Refs,
        CHERRY_PICK,
    ),
    command(
        "revert",
        message!("completion-command-revert"),
        Args::Refs,
        REVERT,
    ),
    command(
        "clone",
        message!("completion-command-clone"),
        Args::Nothing,
        CLONE,
    ),
    command(
        "init",
        message!("completion-command-init"),
        Args::Nothing,
        INIT,
    ),
    command(
        "config",
        message!("completion-command-config"),
        Args::Nothing,
        CONFIG,
    ),
    command(
        "help",
        message!("completion-command-help"),
        Args::Commands,
        HELP,
    ),
    command(
        "worktree",
        message!("completion-command-worktree"),
        Args::Words(WORKTREE),
        &[],
    ),
    command(
        "submodule",
        message!("completion-command-submodule"),
        Args::Words(SUBMODULE),
        &[],
    ),
    command(
        "bisect",
        message!("completion-command-bisect"),
        Args::Words(BISECT),
        &[],
    ),
    command(
        "blame",
        message!("completion-command-blame"),
        Args::Paths(PathSet::Tracked),
        BLAME,
    ),
    command(
        "clean",
        message!("completion-command-clean"),
        Args::Paths(PathSet::Untracked),
        CLEAN,
    ),
    command(
        "rm",
        message!("completion-command-rm"),
        Args::Paths(PathSet::Tracked),
        RM,
    ),
    command(
        "mv",
        message!("completion-command-mv"),
        Args::Paths(PathSet::Tracked),
        MV,
    ),
    command(
        "grep",
        message!("completion-command-grep"),
        Args::Nothing,
        GREP,
    ),
    command(
        "describe",
        message!("completion-command-describe"),
        Args::Refs,
        DESCRIBE,
    ),
    command(
        "reflog",
        message!("completion-command-reflog"),
        Args::Refs,
        &[],
    ),
    command(
        "shortlog",
        message!("completion-command-shortlog"),
        Args::Refs,
        SHORTLOG,
    ),
    command(
        "sparse-checkout",
        message!("completion-command-sparse-checkout"),
        Args::Words(SPARSE),
        &[],
    ),
    command(
        "maintenance",
        message!("completion-command-maintenance"),
        Args::Words(MAINTENANCE),
        &[],
    ),
    command("gc", message!("completion-command-gc"), Args::Nothing, GC),
    command(
        "notes",
        message!("completion-command-notes"),
        Args::Words(NOTES),
        &[],
    ),
    command(
        "archive",
        message!("completion-command-archive"),
        Args::Refs,
        ARCHIVE,
    ),
    command(
        "format-patch",
        message!("completion-command-format-patch"),
        Args::Refs,
        FORMAT_PATCH,
    ),
    command(
        "am",
        message!("completion-command-am"),
        Args::Paths(PathSet::Untracked),
        AM,
    ),
    command(
        "apply",
        message!("completion-command-apply"),
        Args::Paths(PathSet::Untracked),
        APPLY,
    ),
    command(
        "range-diff",
        message!("completion-command-range-diff"),
        Args::Refs,
        &[],
    ),
];

/// The command of this name.
pub fn command_named(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|command| command.name == name)
}

/// The option of a command (or git's own) of this name.
pub fn option_named(options: &'static [Opt], name: &str) -> Option<&'static Opt> {
    options.iter().find(|option| option.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_is_there_once_and_said_in_a_few_words() {
        let mut seen = std::collections::BTreeSet::new();
        for command in COMMANDS {
            assert!(seen.insert(command.name), "{} twice", command.name);
            let detail = command.detail.text();
            assert!(!detail.is_empty() && detail.len() <= 48, "{detail}");
            let mut options = std::collections::BTreeSet::new();
            for option in command.options {
                assert!(option.name.starts_with('-'), "{}", option.name);
                assert!(
                    options.insert(option.name),
                    "{} {}",
                    command.name,
                    option.name
                );
                let detail = option.detail.text();
                assert!(!detail.is_empty() && detail.len() <= 48, "{detail}");
            }
            if let Args::Words(words) = command.args {
                let mut own = std::collections::BTreeSet::new();
                for word in words {
                    assert!(own.insert(word.name), "{} {}", command.name, word.name);
                }
            }
        }
        assert!(command_named("checkout").is_some());
        assert!(command_named("frobnicate").is_none());
    }
}
