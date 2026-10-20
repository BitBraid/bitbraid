//! Requests that move the current branch (merge, rebase, reset, cherry-pick,
//! revert): the menus of the graph and of the references panel make them,
//! the workspace asks where it has to and runs them.
//!
//! Sources: gpui-kit documentation (popup menu).

use std::sync::LazyLock;

use bitbraid_git::{MergeMode, Oid, ResetMode};
use gpui_kit::component::{ActiveTheme, menu::PopupMenu};
use gpui_kit::*;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::session::{BadgeKind, GraphData};

/// One request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum Rewrite {
    /// Merge a branch, a tag or a commit into the current branch.
    Merge {
        /// What to merge, as git names it.
        target: String,
    },
    /// Merge, after asking how: only forward, always with a merge commit,
    /// squashed, or without committing.
    MergeHow {
        /// What to merge, as git names it.
        target: String,
    },
    /// Merge in the way that was chosen.
    MergeAs {
        /// What to merge, as git names it.
        target: String,
        /// How.
        mode: MergeMode,
        /// Without the hooks that could refuse the merge commit.
        #[serde(default)]
        no_verify: bool,
    },
    /// Rebase the current branch onto a branch, a tag or a commit.
    Rebase {
        /// Where the branch's commits go.
        onto: String,
    },
    /// Rebase, after asking for the new base (when none is given), for
    /// where the range begins, and whether local changes are set aside.
    RebaseHow {
        /// The new base, when it is known already.
        onto: Option<String>,
        /// Only the commits after this one move, when that is known.
        after: Option<String>,
    },
    /// Rebase with what was chosen.
    RebaseWith {
        /// Where the commits go.
        onto: String,
        /// Only the commits after this one; all the base lacks when absent.
        after: Option<String>,
        /// Set local changes aside before and bring them back after.
        autostash: bool,
        /// Without the hook that could refuse the rebase.
        #[serde(default)]
        no_verify: bool,
    },
    /// Move the current branch to a commit.
    Reset {
        /// The commit.
        target: String,
        /// What happens to the index and the files.
        mode: ResetMode,
    },
    /// Apply a commit's changes on top of the current branch.
    CherryPick {
        /// The commit.
        commit: String,
        /// It is a merge commit: which parent its changes are taken
        /// against is asked first.
        merge: bool,
        /// As staged changes, without a commit.
        staged: bool,
    },
    /// Undo a commit's changes with a new commit.
    Revert {
        /// The commit.
        commit: String,
        /// It is a merge commit: which parent's line is kept is asked
        /// first.
        merge: bool,
    },
    /// Pick or revert a merge commit against the parent that was chosen.
    OfMerge {
        /// The merge commit.
        commit: String,
        /// The parent, counted from one.
        mainline: u32,
        /// Revert it rather than pick it.
        revert: bool,
        /// A pick as staged changes, without a commit.
        staged: bool,
    },
    /// Plan what happens to this commit and every one after it, up to the
    /// current branch: reorder, reword, squash, edit, drop.
    Interactive {
        /// The oldest commit of the plan.
        first: String,
        /// Begin with that commit set to be reworded.
        reword: bool,
    },
    /// Plan the commits the current branch has and a new base does not,
    /// to be replayed on that base: an interactive rebase onto it.
    InteractiveOnto {
        /// The new base, as git names it.
        onto: String,
    },
    /// Move a local branch that is not checked out forward to a commit.
    FastForward {
        /// The branch.
        branch: String,
        /// Where it moves to, as git names it.
        to: String,
    },
    /// Push a local branch to a branch of a remote.
    PushTo {
        /// The local branch.
        branch: String,
        /// The remote's branch, as `remote/branch`.
        onto: String,
    },
    /// Melt a commit into its parent, messages together.
    Squash {
        /// The commit.
        commit: String,
    },
    /// Take a commit out of the history of the current branch.
    Drop {
        /// The commit.
        commit: String,
    },
    /// Move a commit, or commits that follow one another, one place in
    /// the history of the current branch: up past the commit after them,
    /// or down past the one before.
    Move {
        /// The commits.
        commits: Vec<String>,
        /// Towards HEAD.
        up: bool,
    },
    /// Apply several commits on top of the current branch, oldest first.
    CherryPickMany {
        /// The commits, in the order they are applied.
        commits: Vec<String>,
    },
    /// Undo several commits with new commits, newest first.
    RevertMany {
        /// The commits, in the order they are undone.
        commits: Vec<String>,
    },
    /// Melt commits that follow one another into the oldest of them.
    SquashMany {
        /// The commits.
        commits: Vec<String>,
    },
    /// Open the panel that makes a pull request of a branch.
    StartPull {
        /// The branch it brings: a local one, or `remote/branch`.
        head: String,
        /// The branch it goes into, when one was named.
        base: Option<String>,
    },
}

/// The action a menu entry dispatches.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = rewrite, no_json)]
pub struct RunRewrite {
    /// What to do.
    pub rewrite: Rewrite,
}

/// The engine command for a request and what to say about it.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// Command id.
    pub id: &'static str,
    /// Its parameters.
    pub params: Value,
    /// Shown while it runs.
    pub running: &'static str,
    /// Shown when it worked.
    pub done: String,
}

/// A commit id cut for reading; a name stays as it is.
fn shown(rev: &str) -> &str {
    let is_id = rev.len() == 40 && rev.bytes().all(|byte| byte.is_ascii_hexdigit());
    if is_id {
        rev.get(..7).unwrap_or(rev)
    } else {
        rev
    }
}

/// What is said when a commit was picked.
fn picked(commit: &str, staged: bool) -> String {
    let commit = shown(commit);
    if staged {
        say!("rewrite-picked-staged", commit)
    } else {
        say!("rewrite-cherry-picked", commit)
    }
}

/// The ways to merge, as the engine names them, with what a dialog calls
/// them and what each does. Looked up in the catalog the first time it is
/// read, so that the code that reads it takes `&'static str`s.
pub static MERGE_MODES: LazyLock<[(MergeMode, &'static str, &'static str); 5]> =
    LazyLock::new(|| {
        [
            (
                MergeMode::Ff,
                say!("rewrite-mode-ff"),
                say!("rewrite-mode-ff-note"),
            ),
            (
                MergeMode::FfOnly,
                say!("rewrite-mode-ff-only"),
                say!("rewrite-mode-ff-only-note"),
            ),
            (
                MergeMode::NoFf,
                say!("rewrite-mode-no-ff"),
                say!("rewrite-mode-no-ff-note"),
            ),
            (
                MergeMode::Squash,
                say!("rewrite-mode-squash"),
                say!("rewrite-mode-squash-note"),
            ),
            (
                MergeMode::NoCommit,
                say!("rewrite-mode-no-commit"),
                say!("rewrite-mode-no-commit-note"),
            ),
        ]
    });

/// The parameters of a command, without the hooks that could refuse it
/// when that was asked for (`no_verify`).
fn unverified(params: Value, no_verify: bool) -> Value {
    if no_verify {
        crate::views::workspace::hooks::without(params)
    } else {
        params
    }
}

impl Rewrite {
    /// What running this request means. None for the ones that are
    /// planned in an editor or asked about first.
    pub fn plan(&self) -> Option<Plan> {
        Some(match self {
            // Planned in an editor, or asked about first.
            Rewrite::Interactive { .. }
            | Rewrite::InteractiveOnto { .. }
            | Rewrite::PushTo { .. }
            | Rewrite::StartPull { .. }
            | Rewrite::MergeHow { .. }
            | Rewrite::RebaseHow { .. }
            | Rewrite::CherryPick { merge: true, .. }
            | Rewrite::Revert { merge: true, .. } => return None,
            Rewrite::FastForward { branch, to } => Plan {
                id: "branch.fast_forward",
                params: json!({ "name": branch, "to": to }),
                running: say!("rewrite-fast-forwarding"),
                done: say!("rewrite-fast-forwarded-branch", branch, to = shown(to)),
            },
            // Moving forward makes no commit, and so asks no hook.
            Rewrite::MergeAs {
                target,
                mode: MergeMode::FfOnly,
                ..
            } => Plan {
                id: "branch.merge",
                params: json!({ "target": target, "mode": "ff_only" }),
                running: say!("rewrite-fast-forwarding"),
                done: say!("rewrite-fast-forwarded", target = shown(target)),
            },
            Rewrite::MergeAs {
                target,
                mode,
                no_verify,
            } => Plan {
                id: "branch.merge",
                params: unverified(json!({ "target": target, "mode": mode }), *no_verify),
                running: say!("rewrite-merging"),
                done: match mode {
                    MergeMode::Squash => say!("rewrite-squashed-staged", target = shown(target)),
                    _ => say!("rewrite-merged", target = shown(target)),
                },
            },
            Rewrite::RebaseWith {
                onto,
                after,
                autostash,
                no_verify,
            } => Plan {
                id: "branch.rebase",
                params: unverified(
                    json!({ "onto": onto, "after": after, "autostash": autostash }),
                    *no_verify,
                ),
                running: say!("rewrite-rebasing"),
                done: say!("rewrite-rebased", onto = shown(onto)),
            },
            Rewrite::OfMerge {
                commit,
                mainline,
                revert: true,
                ..
            } => Plan {
                id: "commit.revert",
                params: json!({ "commits": [commit], "mainline": mainline }),
                running: say!("rewrite-reverting"),
                done: say!("rewrite-reverted", commit = shown(commit)),
            },
            Rewrite::OfMerge {
                commit,
                mainline,
                revert: false,
                staged,
            } => Plan {
                id: "commit.cherry_pick",
                params: json!({ "commits": [commit], "mainline": mainline, "no_commit": staged }),
                running: say!("rewrite-cherry-picking"),
                done: picked(commit, *staged),
            },
            Rewrite::CherryPickMany { commits } => Plan {
                id: "commit.cherry_pick",
                params: json!({ "commits": commits }),
                running: say!("rewrite-cherry-picking"),
                done: say!("rewrite-cherry-picked-many", count = commits.len()),
            },
            Rewrite::RevertMany { commits } => Plan {
                id: "commit.revert",
                params: json!({ "commits": commits }),
                running: say!("rewrite-reverting"),
                done: say!("rewrite-reverted-many", count = commits.len()),
            },
            Rewrite::SquashMany { commits } => Plan {
                id: "commit.squash",
                params: json!({ "commits": commits }),
                running: say!("rewrite-squashing"),
                done: say!("rewrite-squashed-many", count = commits.len()),
            },
            Rewrite::Squash { commit } => Plan {
                id: "commit.squash",
                params: json!({ "commits": [commit] }),
                running: say!("rewrite-squashing"),
                done: say!("rewrite-squashed-into-parent", commit = shown(commit)),
            },
            Rewrite::Drop { commit } => Plan {
                id: "commit.drop",
                params: json!({ "commit": commit }),
                running: say!("rewrite-dropping"),
                done: say!("rewrite-dropped", commit = shown(commit)),
            },
            Rewrite::Move { commits, up } => Plan {
                id: "commit.move",
                params: json!({ "commits": commits, "up": up }),
                running: say!("rewrite-moving"),
                done: match (commits.as_slice(), *up) {
                    ([one], true) => say!("rewrite-moved-up", commit = shown(one)),
                    ([one], false) => say!("rewrite-moved-down", commit = shown(one)),
                    (many, true) => say!("rewrite-moved-many-up", count = many.len()),
                    (many, false) => say!("rewrite-moved-many-down", count = many.len()),
                },
            },
            Rewrite::Merge { target } => Plan {
                id: "branch.merge",
                params: json!({ "target": target }),
                running: say!("rewrite-merging"),
                done: say!("rewrite-merged", target = shown(target)),
            },
            Rewrite::Rebase { onto } => Plan {
                id: "branch.rebase",
                params: json!({ "onto": onto }),
                running: say!("rewrite-rebasing"),
                done: say!("rewrite-rebased", onto = shown(onto)),
            },
            Rewrite::Reset { target, mode } => Plan {
                id: "branch.reset",
                params: json!({ "target": target, "mode": mode }),
                running: say!("rewrite-resetting"),
                done: say!("rewrite-reset-to", target = shown(target)),
            },
            Rewrite::CherryPick { commit, staged, .. } => Plan {
                id: "commit.cherry_pick",
                params: json!({ "commits": [commit], "no_commit": staged }),
                running: say!("rewrite-cherry-picking"),
                done: picked(commit, *staged),
            },
            Rewrite::Revert { commit, .. } => Plan {
                id: "commit.revert",
                params: json!({ "commits": [commit] }),
                running: say!("rewrite-reverting"),
                done: say!("rewrite-reverted", commit = shown(commit)),
            },
        })
    }

    /// Whether to ask before running: what throws local changes away, and
    /// what takes a commit out of the history.
    pub fn asks_first(&self) -> bool {
        matches!(
            self,
            Rewrite::Reset {
                mode: ResetMode::Hard,
                ..
            } | Rewrite::Drop { .. }
        )
    }
}

/// What is said before something that may meet conflicts is begun.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meeting {
    /// The other tip, as git names it.
    pub target: String,
    /// What is about to happen, for example "Merging topic".
    pub what: String,
    /// The button that goes on regardless.
    pub go_on: &'static str,
    /// What merging the two tips in memory cannot know.
    pub caveat: &'static str,
}

impl Rewrite {
    /// For a request that brings two lines of history together: what to
    /// check for conflicts first, and how to put it.
    pub fn meets(&self) -> Option<Meeting> {
        match self {
            Rewrite::Merge { target } => Some(Meeting {
                target: target.clone(),
                what: say!("rewrite-merging-target", target = shown(target)),
                go_on: say!("rewrite-merge-anyway"),
                caveat: "",
            }),
            // A merge that only moves forward cannot conflict.
            Rewrite::MergeAs { target, mode, .. } if *mode != MergeMode::FfOnly => Some(Meeting {
                target: target.clone(),
                what: say!("rewrite-merging-target", target = shown(target)),
                go_on: say!("rewrite-merge-anyway"),
                caveat: "",
            }),
            Rewrite::Rebase { onto } | Rewrite::RebaseWith { onto, .. } => Some(Meeting {
                target: onto.clone(),
                what: say!("rewrite-rebasing-onto", onto = shown(onto)),
                go_on: say!("rewrite-rebase-anyway"),
                caveat: say!("rewrite-rebase-caveat"),
            }),
            _ => None,
        }
    }
}

/// Most files a conflict warning names.
const NAMED: usize = 8;

/// The heading and the text of a conflict warning: what would conflict in
/// how many files, and which.
pub fn conflict_warning(what: &str, files: &[String], caveat: &str) -> (String, String) {
    let mut named: Vec<&str> = files.iter().take(NAMED).map(String::as_str).collect();
    let more = files.len().saturating_sub(named.len());
    let rest;
    if more > 0 {
        rest = say!("rewrite-more-files", more);
        named.push(&rest);
    }
    let mut text = say!("rewrite-in-conflict", files = named.join(", "));
    if !caveat.is_empty() {
        text.push(' ');
        text.push_str(caveat);
    }
    (
        say!("rewrite-would-conflict", what, count = files.len()),
        text,
    )
}

/// Something picked up to be dropped on a graph row: a reference or a
/// commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dragged {
    /// What git is told: a branch or tag name, `remote/branch`, or a
    /// commit id.
    pub what: String,
    /// What menus call it.
    pub label: String,
    /// The commit it is at.
    pub commit: Oid,
    /// It is a local branch, which is what can be rebased or reset.
    pub local: bool,
    /// It is a reference, not a bare commit.
    pub is_ref: bool,
    /// A bare commit that is a merge.
    pub merge: bool,
}

/// The label that follows the pointer while something is dragged.
pub struct DragChip(pub String);

impl Render for DragChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px_2()
            .py_0p5()
            .rounded(theme.radius)
            .bg(theme.primary)
            .text_color(theme.primary_foreground)
            .text_xs()
            .child(self.0.clone())
    }
}

/// The graph row something was dropped on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropTarget {
    /// Its commit.
    pub commit: Oid,
    /// A local branch at that commit, the checked-out one when it is there.
    pub branch: Option<String>,
    /// What git is told for "onto here": a name on the row, or the commit.
    pub what: String,
    /// What menus call it.
    pub name: String,
}

/// One thing a drop can mean: an operation, after checking out the branch
/// it applies to when that is not the current one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Choice {
    /// The menu entry.
    pub label: String,
    /// Check this branch out first.
    pub checkout: Option<String>,
    /// Then do this.
    pub rewrite: Rewrite,
}

/// What the graph knows about a drop beyond its two ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Around {
    /// Whether where it lands descends from what was dragged, so that a
    /// branch could simply move forward there. None when that is not
    /// known: the graph that is shown does not reach from one to the
    /// other, or they are too far apart to tell at once.
    pub forward: Option<bool>,
    /// What it landed on is a branch of a remote.
    pub onto_remote: bool,
    /// A remote here is linked to an account: a pull request can be
    /// made of a branch dropped on another.
    pub pull_requests: bool,
}

/// Most commits walked to find out whether one descends from another.
const WALKED: usize = 20_000;

/// Whether `from` descends from `to` (or is it), going by the graph that
/// is shown; none when the walk is given up. Rows are in the order of the
/// graph, a commit above its parents, so nothing below `to` is looked at.
pub fn descends(graph: &GraphData, from: Oid, to: Oid) -> Option<bool> {
    let goal = *graph.index.get(&to)?;
    let mut seen: std::collections::HashSet<usize> = std::collections::HashSet::new();
    let mut stack = vec![*graph.index.get(&from)?];
    while let Some(row) = stack.pop() {
        if row == goal {
            return Some(true);
        }
        if row > goal || !seen.insert(row) {
            continue;
        }
        if seen.len() > WALKED {
            return None;
        }
        for parent in graph
            .commits
            .get(row)
            .map(|commit| commit.parents.as_slice())
            .unwrap_or_default()
        {
            // A parent that is not in the graph leads nowhere here.
            if let Some(parent) = graph.index.get(parent) {
                stack.push(*parent);
            }
        }
    }
    Some(false)
}

/// What the graph says about dropping `dragged` on `target`.
pub fn around(graph: &GraphData, dragged: &Dragged, target: &DropTarget) -> Around {
    Around {
        forward: descends(graph, target.commit, dragged.commit),
        onto_remote: graph.badges.get(&target.commit).is_some_and(|badges| {
            badges
                .iter()
                .any(|badge| badge.kind == BadgeKind::Remote && badge.name == target.what)
        }),
        pull_requests: false,
    }
}

/// What dropping `dragged` on `target` can mean, with `current` the
/// checked-out branch. Dragging goes from what moves to where it goes:
/// a dragged reference is merged into the branch it lands on, a dragged
/// branch is rebased onto or (when it is the current one) reset to where
/// it lands, a dragged commit is picked onto the branch it lands on.
pub fn drop_choices(dragged: &Dragged, target: &DropTarget, current: Option<&str>) -> Vec<Choice> {
    choices(dragged, target, current, None)
}

/// [`drop_choices`], with what the graph knows about the two ends: a
/// branch that could simply move forward to where it lands is offered
/// that first, and a branch dropped on a branch of a remote can be pushed
/// there. A branch can also be rebased with a plan.
pub fn drop_choices_around(
    dragged: &Dragged,
    target: &DropTarget,
    current: Option<&str>,
    around: &Around,
) -> Vec<Choice> {
    choices(dragged, target, current, Some(around))
}

fn choices(
    dragged: &Dragged,
    target: &DropTarget,
    current: Option<&str>,
    around: Option<&Around>,
) -> Vec<Choice> {
    let mut out = Vec::new();
    if dragged.commit == target.commit {
        return out;
    }
    let first = |branch: &str| (current != Some(branch)).then(|| branch.to_owned());
    let (forward, onto_remote) =
        around.map_or((None, false), |around| (around.forward, around.onto_remote));
    if dragged.local && forward == Some(true) {
        let checked_out = current == Some(dragged.what.as_str());
        out.push(Choice {
            label: say!(
                "rewrite-fast-forward-to",
                branch = &dragged.label,
                target = &target.name
            ),
            checkout: None,
            // The checked-out branch takes its files along, as a merge
            // that only moves forward does; another one is just moved.
            rewrite: if checked_out {
                Rewrite::MergeAs {
                    target: target.what.clone(),
                    mode: MergeMode::FfOnly,
                    no_verify: false,
                }
            } else {
                Rewrite::FastForward {
                    branch: dragged.what.clone(),
                    to: target.what.clone(),
                }
            },
        });
    }
    if dragged.local && onto_remote {
        out.push(Choice {
            label: say!(
                "rewrite-push-to",
                branch = &dragged.label,
                target = &target.name
            ),
            checkout: None,
            rewrite: Rewrite::PushTo {
                branch: dragged.what.clone(),
                onto: target.what.clone(),
            },
        });
    }
    if let Some(into) = &target.branch {
        // What the branch it lands on already has is not merged into it.
        if dragged.is_ref && forward == Some(true) {
        } else if dragged.is_ref {
            out.push(Choice {
                label: say!("rewrite-merge-into", name = &dragged.label, into),
                checkout: first(into),
                rewrite: Rewrite::Merge {
                    target: dragged.what.clone(),
                },
            });
        } else {
            out.push(Choice {
                label: say!("rewrite-cherry-pick-onto", name = &dragged.label, into),
                checkout: first(into),
                rewrite: Rewrite::CherryPick {
                    commit: dragged.what.clone(),
                    merge: dragged.merge,
                    staged: false,
                },
            });
        }
    }
    let pull_requests = around.is_some_and(|around| around.pull_requests);
    let base = target
        .branch
        .clone()
        .or_else(|| onto_remote.then(|| target.what.clone()));
    if pull_requests
        && dragged.local
        && let Some(base) = base
    {
        out.push(Choice {
            label: say!(
                "rewrite-start-pull",
                head = &dragged.label,
                base = &target.name
            ),
            checkout: None,
            rewrite: Rewrite::StartPull {
                head: dragged.what.clone(),
                base: Some(base),
            },
        });
    }
    if dragged.local {
        out.push(Choice {
            label: say!(
                "rewrite-rebase-onto",
                branch = &dragged.label,
                onto = &target.name
            ),
            checkout: first(&dragged.what),
            rewrite: Rewrite::Rebase {
                onto: target.what.clone(),
            },
        });
        if around.is_some() {
            out.push(Choice {
                label: say!(
                    "rewrite-rebase-onto-interactively",
                    branch = &dragged.label,
                    onto = &target.name
                ),
                checkout: first(&dragged.what),
                rewrite: Rewrite::InteractiveOnto {
                    onto: target.what.clone(),
                },
            });
        }
        if current == Some(dragged.what.as_str()) {
            let (branch, to) = (&dragged.label, &target.name);
            for (mode, label) in [
                (
                    ResetMode::Soft,
                    say!("rewrite-reset-to-soft", branch, target = to),
                ),
                (
                    ResetMode::Mixed,
                    say!("rewrite-reset-to-mixed", branch, target = to),
                ),
                (
                    ResetMode::Hard,
                    say!("rewrite-reset-to-hard", branch, target = to),
                ),
            ] {
                out.push(Choice {
                    label,
                    checkout: None,
                    rewrite: Rewrite::Reset {
                        target: target.commit.to_hex(),
                        mode,
                    },
                });
            }
        }
    }
    out
}

fn entry(rewrite: Rewrite) -> Box<dyn Action> {
    Box::new(RunRewrite { rewrite })
}

/// Menu entries that combine `target` with the current branch: merge it in,
/// rebase onto it. `name` is how the menu calls the target, `current` the
/// checked-out branch (none when HEAD is detached), and `same` says the
/// target is the commit HEAD is at, where there is nothing to do.
pub fn combine_entries(
    menu: PopupMenu,
    current: Option<&str>,
    target: &str,
    name: &str,
    same: bool,
) -> PopupMenu {
    let merge = Rewrite::Merge {
        target: target.to_owned(),
    };
    let rebase = Rewrite::Rebase {
        onto: target.to_owned(),
    };
    let here = current.unwrap_or("HEAD");
    let menu = menu.menu_with_disabled(
        if same {
            say!("rewrite-merge-already-there", name)
        } else {
            say!("rewrite-merge-into", name, into = here)
        },
        entry(merge),
        same,
    );
    // The other ways to merge and to rebase are asked for in a dialog.
    let menu = if same {
        menu
    } else {
        menu.menu(
            say!("rewrite-merge-into-with-options", name, into = here),
            entry(Rewrite::MergeHow {
                target: target.to_owned(),
            }),
        )
    };
    match current {
        Some(branch) => {
            let menu = menu.menu_with_disabled(
                if same {
                    say!("rewrite-rebase-onto-already-there", name)
                } else {
                    say!("rewrite-rebase-onto", branch, onto = name)
                },
                entry(rebase),
                same,
            );
            if same {
                menu
            } else {
                menu.menu(
                    say!("rewrite-rebase-onto-with-options", branch, onto = name),
                    entry(Rewrite::RebaseHow {
                        onto: Some(target.to_owned()),
                        after: None,
                    }),
                )
            }
        }
        None => menu.menu_with_disabled(
            say!("rewrite-rebase-onto-detached", name),
            entry(rebase),
            true,
        ),
    }
}

/// The submenu that moves the current branch to `target`, one entry per
/// kind of reset.
pub fn reset_entries(
    menu: PopupMenu,
    current: Option<&str>,
    target: &str,
    same: bool,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let here = current.unwrap_or("HEAD").to_owned();
    if same {
        return menu.menu_with_disabled(
            say!("rewrite-reset-here-already-there", here = &here),
            entry(Rewrite::Reset {
                target: target.to_owned(),
                mode: ResetMode::Mixed,
            }),
            true,
        );
    }
    let target = target.to_owned();
    menu.submenu(
        say!("rewrite-reset-here", here),
        window,
        cx,
        move |menu, _, _| {
            let reset = |mode| {
                entry(Rewrite::Reset {
                    target: target.clone(),
                    mode,
                })
            };
            menu.menu(say!("rewrite-reset-soft"), reset(ResetMode::Soft))
                .menu(say!("rewrite-reset-mixed"), reset(ResetMode::Mixed))
                .menu(say!("rewrite-reset-hard"), reset(ResetMode::Hard))
        },
    )
}

/// One step of the part of a commit row's menu that is about the current
/// branch: an entry, or the rule between two groups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowEntry {
    /// The rule between two groups.
    Rule,
    /// An entry that asks for something; one that cannot run says why.
    Run {
        /// What the entry says.
        text: String,
        /// What it asks for.
        rewrite: Rewrite,
        /// It is there to say why not.
        disabled: bool,
    },
    /// The submenu that moves the current branch to the commit, one entry
    /// per kind of reset (see [`reset_entries`]).
    Reset,
}

/// What the row of one commit offers, in its menu, that brings the commit
/// to the current branch or rewrites the branch from it, in groups.
///
/// First what is made of the commit: picking it, rebasing the current
/// branch onto it, resetting to it, reverting it; then picking without
/// committing, merging, and merging and rebasing with options. Then, for
/// a commit of the current branch only (`on_current`), rewriting from it:
/// the plan, a new message, dropping it; then moving it one place up or
/// down, moving it and what follows elsewhere, and squashing it into its
/// parent.
///
/// `target` is what is merged and rebased onto, as git names it (a branch
/// on the row, else the commit), `name` how the menu calls it, `commit`
/// the commit's id, `merge` whether it is a merge commit, and `same`
/// whether HEAD is at it, where there is nothing to bring.
pub fn commit_row_plan(
    current: Option<&str>,
    target: &str,
    name: &str,
    commit: &str,
    merge: bool,
    same: bool,
    on_current: bool,
) -> Vec<RowEntry> {
    let run = |text: String, rewrite: Rewrite, disabled: bool| RowEntry::Run {
        text,
        rewrite,
        disabled,
    };
    let here = current.unwrap_or("HEAD");
    // A merge commit is asked about first: against which parent. The
    // entries' messages choose by this.
    let kind = if merge { "merge" } else { "commit" };
    let pick = |staged| Rewrite::CherryPick {
        commit: commit.to_owned(),
        merge,
        staged,
    };
    let rebase = Rewrite::Rebase {
        onto: target.to_owned(),
    };
    let mut plan = vec![
        run(
            if same {
                say!("rewrite-cherry-pick-this-already-there", kind)
            } else {
                say!("rewrite-cherry-pick-this", kind)
            },
            pick(false),
            same,
        ),
        match current {
            Some(_) if same => run(
                say!("rewrite-rebase-onto-already-there", name),
                rebase,
                true,
            ),
            Some(branch) => run(
                say!("rewrite-rebase-onto", branch, onto = name),
                rebase,
                false,
            ),
            None => run(say!("rewrite-rebase-onto-detached", name), rebase, true),
        },
        RowEntry::Reset,
        run(
            say!("rewrite-revert-this", kind),
            Rewrite::Revert {
                commit: commit.to_owned(),
                merge,
            },
            false,
        ),
        run(
            if same {
                say!("rewrite-cherry-pick-this-staged-already-there", kind)
            } else {
                say!("rewrite-cherry-pick-this-staged", kind)
            },
            pick(true),
            same,
        ),
        run(
            if same {
                say!("rewrite-merge-already-there", name)
            } else {
                say!("rewrite-merge-into", name, into = here)
            },
            Rewrite::Merge {
                target: target.to_owned(),
            },
            same,
        ),
    ];
    // The other ways to merge and to rebase are asked for in a dialog.
    if !same {
        plan.push(run(
            say!("rewrite-merge-into-with-options", name, into = here),
            Rewrite::MergeHow {
                target: target.to_owned(),
            },
            false,
        ));
        if let Some(branch) = current {
            plan.push(run(
                say!("rewrite-rebase-onto-with-options", branch, onto = name),
                Rewrite::RebaseHow {
                    onto: Some(target.to_owned()),
                    after: None,
                },
                false,
            ));
        }
    }
    if !on_current {
        return plan;
    }
    // A merge commit cannot be part of a plan, but it is reworded and
    // dropped like any commit; squashed into its parent it cannot be (that
    // would be the merge undone and its changes kept), and the entry says
    // so.
    plan.extend([
        RowEntry::Rule,
        run(
            say!("rewrite-rebase-interactively").to_owned(),
            Rewrite::Interactive {
                first: commit.to_owned(),
                reword: false,
            },
            merge,
        ),
        run(
            say!("rewrite-reword").to_owned(),
            Rewrite::Interactive {
                first: commit.to_owned(),
                reword: true,
            },
            false,
        ),
        run(
            say!("rewrite-drop").to_owned(),
            Rewrite::Drop {
                commit: commit.to_owned(),
            },
            false,
        ),
    ]);
    // One place up, past the commit after it: not for the newest, which
    // has none; and down, past the one before it.
    if !same {
        plan.push(run(
            say!("rewrite-move-up").to_owned(),
            Rewrite::Move {
                commits: vec![commit.to_owned()],
                up: true,
            },
            merge,
        ));
    }
    plan.extend([
        run(
            say!("rewrite-move-down").to_owned(),
            Rewrite::Move {
                commits: vec![commit.to_owned()],
                up: false,
            },
            merge,
        ),
        // This commit and what follows it, moved somewhere else: the
        // range begins after its parent.
        run(
            say!("rewrite-rebase-elsewhere").to_owned(),
            Rewrite::RebaseHow {
                onto: None,
                after: Some(format!("{commit}^")),
            },
            merge,
        ),
        run(
            squash_label(merge).to_owned(),
            Rewrite::Squash {
                commit: commit.to_owned(),
            },
            merge,
        ),
    ]);
    plan
}

/// The entries of [`commit_row_plan`], put into a menu.
pub fn commit_row_entries(
    menu: PopupMenu,
    plan: Vec<RowEntry>,
    current: Option<&str>,
    commit: &str,
    same: bool,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let mut menu = menu;
    for step in plan {
        menu = match step {
            RowEntry::Rule => menu.separator(),
            RowEntry::Run {
                text,
                rewrite,
                disabled,
            } => menu.menu_with_disabled(text, entry(rewrite), disabled),
            RowEntry::Reset => reset_entries(menu, current, commit, same, window, cx),
        };
    }
    menu
}

/// What the row of one of several selected commits offers, in its menu,
/// about the current branch: the entries for one commit where they take
/// several (picking, reverting) in the plural, and the others for the
/// commit the menu is on (`commit`: rebasing onto it, resetting to it). For
/// a run of commits of the current branch (`run_on_current`) then what
/// rewrites them: moving them one place up (not when the newest is HEAD's,
/// `top_at_head`) or down, squashing them. A merge commit among them rules
/// out what would have to choose a parent for it. `newest_first` is the
/// selection as the graph lists it.
#[allow(clippy::too_many_arguments)]
pub fn many_row_plan(
    current: Option<&str>,
    commit: &str,
    same: bool,
    newest_first: &[String],
    any_merge: bool,
    run_on_current: bool,
    top_at_head: bool,
) -> Vec<RowEntry> {
    let count = newest_first.len();
    let oldest_first: Vec<String> = newest_first.iter().rev().cloned().collect();
    let run = |text: String, rewrite: Rewrite, disabled: bool| RowEntry::Run {
        text,
        rewrite,
        disabled,
    };
    // The entries' messages choose by whether a merge commit is among them.
    let among = if any_merge { "merge" } else { "none" };
    let rebase = Rewrite::Rebase {
        onto: commit.to_owned(),
    };
    let mut plan = vec![
        run(
            say!("rewrite-cherry-pick-many", count, among),
            Rewrite::CherryPickMany {
                commits: oldest_first.clone(),
            },
            any_merge,
        ),
        match current {
            Some(_) if same => run(
                say!("rewrite-rebase-onto-this-already-there").to_owned(),
                rebase,
                true,
            ),
            Some(branch) => run(say!("rewrite-rebase-onto-this", branch), rebase, false),
            None => run(
                say!("rewrite-rebase-onto-this-detached").to_owned(),
                rebase,
                true,
            ),
        },
        RowEntry::Reset,
        run(
            say!("rewrite-revert-many", count, among),
            Rewrite::RevertMany {
                commits: newest_first.to_vec(),
            },
            any_merge,
        ),
    ];
    if run_on_current {
        plan.push(RowEntry::Rule);
        if !top_at_head {
            plan.push(run(
                say!("rewrite-move-many-up", count, among),
                Rewrite::Move {
                    commits: oldest_first.clone(),
                    up: true,
                },
                any_merge,
            ));
        }
        plan.extend([
            run(
                say!("rewrite-move-many-down", count, among),
                Rewrite::Move {
                    commits: oldest_first.clone(),
                    up: false,
                },
                any_merge,
            ),
            run(
                say!("rewrite-squash-many", count, among),
                Rewrite::SquashMany {
                    commits: oldest_first,
                },
                any_merge,
            ),
        ]);
    }
    plan
}

/// What the entry that squashes a commit into its parent says: for a merge
/// commit, that it is not for one.
pub fn squash_label(merge: bool) -> &'static str {
    if merge {
        say!("rewrite-squash-into-parent-not-merge")
    } else {
        say!("rewrite-squash-into-parent")
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Dragged, DropTarget, Rewrite, RowEntry, commit_row_plan, conflict_warning, drop_choices,
        many_row_plan, shown, squash_label,
    };
    use bitbraid_git::{Oid, ResetMode};
    use serde_json::json;

    fn id(byte: u8) -> Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    fn branch(name: &str, at: u8) -> Dragged {
        Dragged {
            what: name.to_owned(),
            label: name.to_owned(),
            commit: id(at),
            local: !name.contains('/'),
            is_ref: true,
            merge: false,
        }
    }

    fn row(branch: Option<&str>, at: u8) -> DropTarget {
        DropTarget {
            commit: id(at),
            branch: branch.map(str::to_owned),
            what: branch.map(str::to_owned).unwrap_or_else(|| id(at).to_hex()),
            name: branch.map(str::to_owned).unwrap_or_else(|| id(at).short(7)),
        }
    }

    #[test]
    fn a_drop_means_what_moves_goes_where_it_lands() {
        let labels = |dragged: &Dragged, target: &DropTarget, current: Option<&str>| {
            drop_choices(dragged, target, current)
                .into_iter()
                .map(|choice| match choice.checkout {
                    Some(first) => format!("[{first}] {}", choice.label),
                    None => choice.label,
                })
                .collect::<Vec<_>>()
        };
        // A branch onto the current branch: merged in at once; rebasing it
        // means checking it out first.
        assert_eq!(
            labels(&branch("topic", 1), &row(Some("main"), 2), Some("main")),
            ["Merge topic into main", "[topic] Rebase topic onto main"]
        );
        // The current branch onto another: the other way round, and it can
        // be reset there.
        assert_eq!(
            labels(&branch("main", 2), &row(Some("topic"), 1), Some("main")),
            [
                "[topic] Merge main into topic",
                "Rebase main onto topic",
                "Reset main to topic (soft: keep every change, staged)",
                "Reset main to topic (mixed: keep every change, unstaged)",
                "Reset main to topic (hard: drop local changes…)",
            ]
        );
        // A remote branch can be merged, not rebased; onto a bare commit
        // there is nothing to merge into.
        assert_eq!(
            labels(
                &branch("origin/topic", 1),
                &row(Some("main"), 2),
                Some("main")
            ),
            ["Merge origin/topic into main"]
        );
        assert!(labels(&branch("origin/topic", 1), &row(None, 3), Some("main")).is_empty());
        assert_eq!(
            labels(&branch("topic", 1), &row(None, 3), Some("main")),
            ["[topic] Rebase topic onto 0303030"]
        );
        // A commit is picked onto the branch it lands on.
        let commit = Dragged {
            what: id(4).to_hex(),
            label: id(4).short(7),
            commit: id(4),
            local: false,
            is_ref: false,
            merge: true,
        };
        let picked = drop_choices(&commit, &row(Some("topic"), 1), Some("main"));
        assert_eq!(picked.len(), 1);
        assert_eq!(picked[0].label, "Cherry-pick 0404040 onto topic");
        assert_eq!(picked[0].checkout.as_deref(), Some("topic"));
        assert_eq!(
            picked[0].rewrite,
            Rewrite::CherryPick {
                commit: id(4).to_hex(),
                merge: true,
                staged: false,
            }
        );
        // Onto itself, or with HEAD detached and nothing to land on: nothing.
        assert!(labels(&branch("topic", 1), &row(Some("main"), 1), Some("main")).is_empty());
        assert!(labels(&commit, &row(None, 3), None).is_empty());
        // Detached: every branch involved has to be checked out first.
        assert_eq!(
            labels(&branch("topic", 1), &row(Some("main"), 2), None),
            [
                "[main] Merge topic into main",
                "[topic] Rebase topic onto main"
            ]
        );
    }

    #[test]
    fn a_drop_offers_what_the_graph_says_is_possible() {
        use super::{Around, drop_choices_around};
        let labels = |dragged: &Dragged, target: &DropTarget, current, around: &Around| {
            drop_choices_around(dragged, target, current, around)
                .into_iter()
                .map(|choice| match choice.checkout {
                    Some(first) => format!("[{first}] {}", choice.label),
                    None => choice.label,
                })
                .collect::<Vec<_>>()
        };
        let forward = Around {
            forward: Some(true),
            onto_remote: false,
            pull_requests: false,
        };
        // A branch that is behind where it lands: moved forward first of
        // all, without a checkout when it is not the checked-out one.
        let choices =
            drop_choices_around(&branch("topic", 1), &row(None, 3), Some("main"), &forward);
        assert_eq!(choices[0].label, "Fast-forward topic to 0303030");
        assert_eq!(choices[0].checkout, None);
        assert_eq!(
            choices[0].rewrite,
            Rewrite::FastForward {
                branch: "topic".to_owned(),
                to: id(3).to_hex(),
            }
        );
        // The checked-out one moves as a merge that only goes forward.
        let choices = drop_choices_around(
            &branch("main", 2),
            &row(Some("topic"), 1),
            Some("main"),
            &forward,
        );
        assert_eq!(choices[0].label, "Fast-forward main to topic");
        // Topic has everything main has: there is nothing to merge into it.
        assert!(
            !choices
                .iter()
                .any(|choice| choice.label.starts_with("Merge"))
        );
        assert_eq!(
            choices[0].rewrite.plan().unwrap().params,
            json!({ "target": "topic", "mode": "ff_only" })
        );
        // A plan for the rebase is on offer wherever a rebase is, and it
        // opens the editor rather than run anything.
        let plain = drop_choices_around(
            &branch("topic", 1),
            &row(Some("main"), 2),
            Some("main"),
            &Around::default(),
        );
        assert_eq!(
            plain
                .iter()
                .map(|choice| choice.label.as_str())
                .collect::<Vec<_>>(),
            [
                "Merge topic into main",
                "Rebase topic onto main",
                "Rebase topic onto main interactively…",
            ]
        );
        assert_eq!(plain[2].checkout.as_deref(), Some("topic"));
        assert_eq!(
            plain[2].rewrite,
            Rewrite::InteractiveOnto {
                onto: "main".to_owned()
            }
        );
        assert!(plain[2].rewrite.plan().is_none());
        // Not known, or known not to be: not offered.
        for around in [
            Around::default(),
            Around {
                forward: Some(false),
                onto_remote: false,
                pull_requests: false,
            },
        ] {
            assert!(
                !labels(&branch("topic", 1), &row(None, 3), Some("main"), &around)
                    .iter()
                    .any(|label| label.starts_with("Fast-forward"))
            );
        }
        // A remote branch and a commit do not move forward by a drop.
        assert!(
            !labels(
                &branch("origin/topic", 1),
                &row(Some("main"), 2),
                Some("main"),
                &forward
            )
            .iter()
            .any(|label| label.starts_with("Fast-forward"))
        );
        // Dropped on a branch of a remote, a local branch can be pushed
        // there, whatever it is called.
        let remote = DropTarget {
            commit: id(5),
            branch: None,
            what: "origin/release".to_owned(),
            name: "origin/release".to_owned(),
        };
        let onto_remote = Around {
            forward: None,
            onto_remote: true,
            pull_requests: false,
        };
        assert_eq!(
            labels(&branch("topic", 1), &remote, Some("main"), &onto_remote),
            [
                "Push topic to origin/release",
                "[topic] Rebase topic onto origin/release",
                "[topic] Rebase topic onto origin/release interactively…",
            ]
        );
        let push =
            &drop_choices_around(&branch("topic", 1), &remote, Some("main"), &onto_remote)[0];
        assert_eq!(
            push.rewrite,
            Rewrite::PushTo {
                branch: "topic".to_owned(),
                onto: "origin/release".to_owned(),
            }
        );
        assert!(push.rewrite.plan().is_none());
        // A remote branch is not pushed to a remote branch.
        assert!(
            labels(
                &branch("origin/topic", 1),
                &remote,
                Some("main"),
                &onto_remote
            )
            .is_empty()
        );
    }

    #[test]
    fn descent_is_read_off_the_graph_that_is_shown() {
        use super::{around, descends};
        use crate::session::{Badge, BadgeKind, GraphData};
        use bitbraid_git::GraphCommit;
        // 1 is the tip; 1 -> 2 -> 4 and a side line 3 -> 4; 5 is apart.
        let commit = |at: u8, parents: &[u8]| GraphCommit {
            id: id(at),
            parents: parents.iter().map(|parent| id(*parent)).collect(),
            seconds: None,
        };
        let commits = vec![
            commit(1, &[2]),
            commit(2, &[4]),
            commit(3, &[4]),
            commit(4, &[9]),
            commit(5, &[]),
        ];
        let mut graph = GraphData {
            index: commits
                .iter()
                .enumerate()
                .map(|(row, commit)| (commit.id, row))
                .collect(),
            commits,
            rows: Vec::new(),
            wip: None,
            child: Vec::new(),
            badges: Default::default(),
            worktrees: Vec::new(),
            worktree_rows: Default::default(),
            upstreams: Default::default(),
            head: None,
            max_width: 1,
            order: Default::default(),
            walked: None,
            generation: 0,
        };
        assert_eq!(descends(&graph, id(1), id(4)), Some(true));
        assert_eq!(descends(&graph, id(1), id(1)), Some(true));
        assert_eq!(descends(&graph, id(3), id(4)), Some(true));
        // Sideways, backwards, and apart.
        assert_eq!(descends(&graph, id(1), id(3)), Some(false));
        assert_eq!(descends(&graph, id(4), id(1)), Some(false));
        assert_eq!(descends(&graph, id(1), id(5)), Some(false));
        // A commit the graph does not show: not known.
        assert_eq!(descends(&graph, id(1), id(9)), None);
        assert_eq!(descends(&graph, id(8), id(1)), None);

        graph.badges.insert(
            id(1),
            vec![Badge {
                kind: BadgeKind::Remote,
                name: "origin/main".to_owned(),
                full_name: Some("refs/remotes/origin/main".to_owned()),
            }],
        );
        let onto = DropTarget {
            commit: id(1),
            branch: None,
            what: "origin/main".to_owned(),
            name: "origin/main".to_owned(),
        };
        let about = around(&graph, &branch("topic", 4), &onto);
        assert_eq!(about.forward, Some(true));
        assert!(about.onto_remote);
        // The same commit under a local name is no branch of a remote.
        let local = DropTarget {
            what: "main".to_owned(),
            name: "main".to_owned(),
            ..onto
        };
        assert!(!around(&graph, &branch("topic", 4), &local).onto_remote);
    }

    #[test]
    fn plans_name_the_command_and_its_parameters() {
        let id = "0123456789abcdef0123456789abcdef01234567";
        let plan = Rewrite::Merge {
            target: "origin/topic".to_owned(),
        }
        .plan()
        .unwrap();
        assert_eq!(plan.id, "branch.merge");
        assert_eq!(plan.params, json!({ "target": "origin/topic" }));
        assert_eq!(plan.done, "Merged origin/topic");

        let plan = Rewrite::Reset {
            target: id.to_owned(),
            mode: ResetMode::Hard,
        }
        .plan()
        .unwrap();
        assert_eq!(plan.params, json!({ "target": id, "mode": "hard" }));
        assert_eq!(plan.done, "Reset to 0123456");

        // A merge commit is asked about first: there is nothing to run
        // until a parent is chosen.
        assert!(
            Rewrite::Revert {
                commit: id.to_owned(),
                merge: true,
            }
            .plan()
            .is_none()
        );
        assert!(
            Rewrite::CherryPick {
                commit: id.to_owned(),
                merge: true,
                staged: true,
            }
            .plan()
            .is_none()
        );
        let plan = Rewrite::OfMerge {
            commit: id.to_owned(),
            mainline: 2,
            revert: true,
            staged: false,
        }
        .plan()
        .unwrap();
        assert_eq!(plan.id, "commit.revert");
        assert_eq!(plan.params, json!({ "commits": [id], "mainline": 2 }));
        let plan = Rewrite::OfMerge {
            commit: id.to_owned(),
            mainline: 1,
            revert: false,
            staged: true,
        }
        .plan()
        .unwrap();
        assert_eq!(plan.id, "commit.cherry_pick");
        assert_eq!(
            plan.params,
            json!({ "commits": [id], "mainline": 1, "no_commit": true })
        );
        assert!(plan.done.contains("nothing is committed"));
        let plan = Rewrite::CherryPick {
            commit: id.to_owned(),
            merge: false,
            staged: false,
        }
        .plan()
        .unwrap();
        assert_eq!(plan.params, json!({ "commits": [id], "no_commit": false }));
        assert_eq!(plan.done, "Cherry-picked 0123456");
    }

    #[test]
    fn the_ways_to_merge_and_rebase_name_their_parameters() {
        use super::MERGE_MODES;
        use bitbraid_git::MergeMode;
        // What is asked about first runs nothing by itself.
        let ask = Rewrite::MergeHow {
            target: "topic".to_owned(),
        };
        assert!(ask.plan().is_none() && ask.meets().is_none() && !ask.asks_first());
        let ask = Rewrite::RebaseHow {
            onto: None,
            after: Some("abc^".to_owned()),
        };
        assert!(ask.plan().is_none() && ask.meets().is_none());
        // Every way to merge is on offer once, and names the engine's word.
        let words: Vec<String> = MERGE_MODES
            .iter()
            .map(|(mode, _, _)| {
                Rewrite::MergeAs {
                    target: "topic".to_owned(),
                    mode: *mode,
                    no_verify: false,
                }
                .plan()
                .unwrap()
                .params["mode"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(words, ["ff", "ff_only", "no_ff", "squash", "no_commit"]);
        // Only a fast-forward needs no look at conflicts first.
        for (mode, _, _) in *MERGE_MODES {
            let merge = Rewrite::MergeAs {
                target: "topic".to_owned(),
                mode,
                no_verify: false,
            };
            assert_eq!(merge.meets().is_none(), mode == MergeMode::FfOnly);
            // Without its hooks only when asked, and never where no
            // commit is made for a hook to refuse.
            assert!(merge.plan().unwrap().params.get("no_verify").is_none());
            let unverified = Rewrite::MergeAs {
                target: "topic".to_owned(),
                mode,
                no_verify: true,
            };
            assert_eq!(
                unverified.plan().unwrap().params.get("no_verify"),
                (mode != MergeMode::FfOnly).then_some(&json!(true))
            );
        }
        let rebase = Rewrite::RebaseWith {
            onto: "main".to_owned(),
            after: Some("topic~2".to_owned()),
            autostash: true,
            no_verify: false,
        };
        let plan = rebase.plan().unwrap();
        assert_eq!(plan.id, "branch.rebase");
        assert_eq!(
            plan.params,
            json!({ "onto": "main", "after": "topic~2", "autostash": true })
        );
        assert_eq!(rebase.meets().unwrap().target, "main");
        let unverified = Rewrite::RebaseWith {
            onto: "main".to_owned(),
            after: None,
            autostash: false,
            no_verify: true,
        };
        assert_eq!(
            unverified.plan().unwrap().params,
            json!({ "onto": "main", "after": null, "autostash": false, "no_verify": true })
        );
    }

    #[test]
    fn only_a_hard_reset_asks_first() {
        let reset = |mode| Rewrite::Reset {
            target: "main".to_owned(),
            mode,
        };
        assert!(reset(ResetMode::Hard).asks_first());
        assert!(!reset(ResetMode::Soft).asks_first());
        assert!(!reset(ResetMode::Mixed).asks_first());
        assert!(
            !Rewrite::Rebase {
                onto: "main".to_owned()
            }
            .asks_first()
        );
        // Taking a commit out asks; melting it into its parent keeps its
        // changes and does not.
        let commit = "ab".repeat(20);
        let drop = Rewrite::Drop {
            commit: commit.clone(),
        };
        assert!(drop.asks_first());
        let plan = drop.plan().unwrap();
        assert_eq!(plan.id, "commit.drop");
        assert_eq!(plan.done, "Dropped abababa");
        let squash = Rewrite::Squash {
            commit: commit.clone(),
        };
        assert!(!squash.asks_first());
        let plan = squash.plan().unwrap();
        assert_eq!(plan.id, "commit.squash");
        assert_eq!(plan.params, json!({ "commits": [commit] }));
        // The editor is opened instead of a command run.
        let planned = Rewrite::Interactive {
            first: commit,
            reword: true,
        };
        assert!(planned.plan().is_none());
        assert!(!planned.asks_first());
        assert!(planned.meets().is_none());
        // Several at once name the same commands with all of them.
        let ids = vec!["a".repeat(40), "b".repeat(40)];
        for (rewrite, id, done) in [
            (
                Rewrite::CherryPickMany {
                    commits: ids.clone(),
                },
                "commit.cherry_pick",
                "Cherry-picked 2 commits",
            ),
            (
                Rewrite::RevertMany {
                    commits: ids.clone(),
                },
                "commit.revert",
                "Reverted 2 commits",
            ),
            (
                Rewrite::SquashMany {
                    commits: ids.clone(),
                },
                "commit.squash",
                "Squashed 2 commits",
            ),
        ] {
            let plan = rewrite.plan().unwrap();
            assert_eq!(plan.id, id);
            assert_eq!(plan.params, json!({ "commits": ids }));
            assert_eq!(plan.done, done);
            assert!(!rewrite.asks_first());
            assert!(rewrite.meets().is_none());
        }
    }

    #[test]
    fn ids_are_cut_and_names_are_not() {
        assert_eq!(shown("0123456789abcdef0123456789abcdef01234567"), "0123456");
        assert_eq!(
            shown("feature/long-name-that-stays"),
            "feature/long-name-that-stays"
        );
        // Forty characters that are not an id, with a wide one at the cut.
        let name = "ééééééééééééééééééééééééééééééééééééééé";
        assert_eq!(shown(name), name);
    }

    #[test]
    fn a_conflict_warning_names_the_files_up_to_a_point() {
        let merge = Rewrite::Merge {
            target: "topic".to_owned(),
        }
        .meets()
        .unwrap();
        assert_eq!(merge.target, "topic");
        let (title, text) = conflict_warning(&merge.what, &["a.txt".to_owned()], merge.caveat);
        assert_eq!(title, "Merging topic would conflict in 1 file");
        assert_eq!(
            text,
            "In conflict: a.txt. It stops there and waits for the files to be resolved, or to \
             be aborted."
        );

        let rebase = Rewrite::Rebase {
            onto: "ab".repeat(20),
        }
        .meets()
        .unwrap();
        assert_eq!(rebase.what, "Rebasing onto abababa");
        let files: Vec<String> = (0..11).map(|n| format!("f{n}")).collect();
        let (title, text) = conflict_warning(&rebase.what, &files, rebase.caveat);
        assert_eq!(title, "Rebasing onto abababa would conflict in 11 files");
        assert!(text.starts_with("In conflict: f0, f1, f2, f3, f4, f5, f6, f7, 3 more. It stops"));
        assert!(text.ends_with("may stop on other files as well."));

        // What moves the branch without bringing two lines together.
        for rewrite in [
            Rewrite::Reset {
                target: "x".to_owned(),
                mode: ResetMode::Hard,
            },
            Rewrite::CherryPick {
                commit: "x".to_owned(),
                merge: false,
                staged: false,
            },
            Rewrite::Revert {
                commit: "x".to_owned(),
                merge: true,
            },
        ] {
            assert!(rewrite.meets().is_none());
        }
    }
    #[test]
    fn the_squash_entry_says_what_it_is_not_for() {
        assert_eq!(squash_label(false), "Squash into its parent");
        assert!(squash_label(true).contains("not for a merge commit"));
    }

    /// What a plan says, one line an entry: a rule is `---`, the reset
    /// submenu `reset`, and an entry that cannot run ends in `(off)`.
    fn said(plan: &[RowEntry]) -> Vec<String> {
        plan.iter()
            .map(|step| match step {
                RowEntry::Rule => "---".to_owned(),
                RowEntry::Reset => "reset".to_owned(),
                RowEntry::Run { text, disabled, .. } if *disabled => format!("{text} (off)"),
                RowEntry::Run { text, .. } => text.clone(),
            })
            .collect()
    }

    #[test]
    fn a_commit_rows_menu_has_its_groups_in_order() {
        let commit = "ab".repeat(20);
        // A commit of the current branch, further down: what is made of
        // it, then rewriting from it, each group in its order.
        let plan = commit_row_plan(
            Some("main"),
            &commit,
            "this commit",
            &commit,
            false,
            false,
            true,
        );
        assert_eq!(
            said(&plan),
            [
                "Cherry-pick this commit",
                "Rebase main onto this commit",
                "reset",
                "Revert this commit",
                "Cherry-pick this commit without committing",
                "Merge this commit into main",
                "Merge this commit into main with options…",
                "Rebase main onto this commit with options…",
                "---",
                "Rebase interactively from here…",
                "Reword…",
                "Drop…",
                "Move up",
                "Move down",
                "Rebase from here onto another base…",
                "Squash into its parent",
            ]
        );
        // What the entries ask for is about that commit.
        assert!(plan.contains(&RowEntry::Run {
            text: "Drop…".to_owned(),
            rewrite: Rewrite::Drop {
                commit: commit.clone()
            },
            disabled: false,
        }));
        assert!(plan.contains(&RowEntry::Run {
            text: "Rebase from here onto another base…".to_owned(),
            rewrite: Rewrite::RebaseHow {
                onto: None,
                after: Some(format!("{commit}^")),
            },
            disabled: false,
        }));

        // A commit of another branch, which carries that branch: it is
        // merged and rebased onto by name, and nothing is rewritten from it.
        let plan = commit_row_plan(Some("main"), "topic", "topic", &commit, false, false, false);
        assert_eq!(
            said(&plan),
            [
                "Cherry-pick this commit",
                "Rebase main onto topic",
                "reset",
                "Revert this commit",
                "Cherry-pick this commit without committing",
                "Merge topic into main",
                "Merge topic into main with options…",
                "Rebase main onto topic with options…",
            ]
        );
        assert!(plan.contains(&RowEntry::Run {
            text: "Merge topic into main".to_owned(),
            rewrite: Rewrite::Merge {
                target: "topic".to_owned()
            },
            disabled: false,
        }));

        // The commit HEAD is at: there is nothing to bring, and the
        // entries say so; it is rewritten like any commit of the branch.
        let plan = commit_row_plan(
            Some("main"),
            &commit,
            "this commit",
            &commit,
            false,
            true,
            true,
        );
        assert_eq!(
            said(&plan),
            [
                "Cherry-pick this commit (HEAD is already there) (off)",
                "Rebase onto this commit (HEAD is already there) (off)",
                "reset",
                "Revert this commit",
                "Cherry-pick this commit without committing (HEAD is already there) (off)",
                "Merge this commit (HEAD is already there) (off)",
                "---",
                "Rebase interactively from here…",
                "Reword…",
                "Drop…",
                "Move down",
                "Rebase from here onto another base…",
                "Squash into its parent",
            ]
        );

        // A merge commit is asked about first, and is no part of a plan;
        // with HEAD on no branch there is nothing to rebase.
        let plan = commit_row_plan(None, &commit, "this commit", &commit, true, false, true);
        assert_eq!(
            said(&plan),
            [
                "Cherry-pick this merge commit…",
                "Rebase onto this commit (HEAD is not on a branch) (off)",
                "reset",
                "Revert this merge commit…",
                "Cherry-pick this merge commit without committing…",
                "Merge this commit into HEAD",
                "Merge this commit into HEAD with options…",
                "---",
                "Rebase interactively from here… (off)",
                "Reword…",
                "Drop…",
                "Move up (off)",
                "Move down (off)",
                "Rebase from here onto another base… (off)",
                "Squash into its parent (not for a merge commit) (off)",
            ]
        );
    }

    #[test]
    fn several_commits_have_the_one_commit_entries_in_the_plural() {
        let commits: Vec<String> = ["cc", "bb", "aa"].iter().map(|b| b.repeat(20)).collect();
        let here = "bb".repeat(20);
        let plan = many_row_plan(Some("main"), &here, false, &commits, false, true, false);
        assert_eq!(
            said(&plan),
            [
                "Cherry-pick 3 commits",
                "Rebase main onto this commit",
                "reset",
                "Revert 3 commits",
                "---",
                "Move up 3 commits",
                "Move down 3 commits",
                "Squash 3 commits",
            ]
        );
        // Picked and moved oldest first, reverted newest first.
        let oldest_first: Vec<String> = commits.iter().rev().cloned().collect();
        assert!(plan.contains(&RowEntry::Run {
            text: "Move down 3 commits".to_owned(),
            rewrite: Rewrite::Move {
                commits: oldest_first.clone(),
                up: false,
            },
            disabled: false,
        }));
        assert!(plan.contains(&RowEntry::Run {
            text: "Revert 3 commits".to_owned(),
            rewrite: Rewrite::RevertMany {
                commits: commits.clone()
            },
            disabled: false,
        }));
        // Not a run of the current branch, a merge among them, the newest
        // at HEAD.
        let apart = many_row_plan(Some("main"), &here, false, &commits, false, false, false);
        assert_eq!(said(&apart).len(), 4);
        let merged = many_row_plan(Some("main"), &here, false, &commits, true, true, true);
        assert_eq!(
            said(&merged),
            [
                "Cherry-pick 3 commits (a merge commit is among them) (off)",
                "Rebase main onto this commit",
                "reset",
                "Revert 3 commits (a merge commit is among them) (off)",
                "---",
                "Move down 3 commits (a merge commit is among them) (off)",
                "Squash 3 commits (a merge commit is among them) (off)",
            ]
        );
    }
}
