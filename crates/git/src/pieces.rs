//! A history walked in pieces, so that the first rows of a large graph can
//! be shown before the whole of it has been walked.
//!
//! The walk comes to commits newest first. After some of them, the rows
//! that are already certain are handed out as a piece: [`stable_rows`]
//! puts what was walked so far into graph order and keeps the leading part
//! of it that the rest of the walk cannot change. A piece is therefore the
//! beginning of the next piece and of the whole, row for row, and since
//! the lane layout goes from the top down, the lanes of its rows stay
//! where they are when more arrives.
//!
//! What can still change a row is a commit that is not walked yet and
//! turns out to be a child of one that is: it has to come first. The walk
//! goes by commit time, so such a child is older than everything walked,
//! give or take the clocks of the machines the commits were made on. Rows
//! of commits that are not clearly newer than the place the walk has
//! reached ([`MARGIN_SECONDS`]) are held back for the next piece. A
//! history whose clocks are further off than that may see a row of a
//! piece move when the whole arrives; the whole is always right.
//!
//! In topological order a piece also ends after the first commit that has
//! a parent the walk has not reached: that parent may come next, wherever
//! in time it lies. Pieces in that order are short when the newest line of
//! history is.
//!
//! Sources: git-rev-list(1) (`--date-order`, `--topo-order`); the
//! reasoning above is ours.

use std::collections::HashSet;

use crate::{
    GraphCommit, Oid, Order, Repository, Result,
    log::{LogOptions, commit_tips, ordered, resolve_tips, walk_by_time},
};

/// How many commits are walked before the first piece is handed out. A
/// history that has no more is handed out whole.
pub const FIRST_PIECE: usize = 50_000;

/// Each further piece is this many times the one before.
pub const GROWTH: usize = 8;

/// A commit whose time is within this of the oldest commit walked is not
/// yet certain of its row: three days, which is the clock skew that is
/// allowed for (see the module's text).
pub const MARGIN_SECONDS: i64 = 3 * 86_400;

/// A piece with fewer rows than this is not worth showing.
pub const SMALLEST_PIECE: usize = 64;

/// What the beginning of a walk gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum First {
    /// The walk ended within the first piece: every commit, in graph
    /// order.
    Whole(Vec<GraphCommit>),
    /// There is more: the rows that are certain so far, in graph order,
    /// and how many commits were walked for them.
    Piece {
        /// The leading rows of the graph.
        rows: Vec<GraphCommit>,
        /// Commits walked so far.
        walked: usize,
    },
}

/// The leading rows of the graph that `walked` already settles. `walked`
/// holds the first commits the walk came to, in that order, and more are
/// to come.
pub fn stable_rows(walked: &[GraphCommit], order: Order) -> Vec<GraphCommit> {
    // By the author's date nothing is settled before the walk is over: a
    // commit written long ago can be the next one the walk comes to. Such
    // a history comes whole.
    if order == Order::AuthorDate {
        return Vec::new();
    }
    // The walk has come down to this time; a commit without one (which
    // only a damaged commit has) settles nothing.
    let mut reached: Option<i64> = None;
    for commit in walked {
        let Some(seconds) = commit.seconds else {
            return Vec::new();
        };
        reached = Some(reached.map_or(seconds, |least: i64| least.min(seconds)));
    }
    let Some(bound) = reached.map(|least| least.saturating_add(MARGIN_SECONDS)) else {
        return Vec::new();
    };
    let known: HashSet<Oid> = match order {
        Order::Topo => walked.iter().map(|commit| commit.id).collect(),
        Order::Date | Order::AuthorDate => HashSet::new(),
    };
    let mut rows = match order {
        Order::Topo => crate::log::topological_order(walked.to_vec()),
        Order::Date | Order::AuthorDate => crate::log::topological_repair(walked.to_vec()),
    };
    let mut keep = 0usize;
    for commit in &rows {
        // It may still get a child that has to come before it.
        if commit.seconds.is_none_or(|seconds| seconds <= bound) {
            break;
        }
        keep = keep.saturating_add(1);
        // In topological order what comes next may be a parent that is
        // not walked yet.
        if order == Order::Topo && commit.parents.iter().any(|parent| !known.contains(parent)) {
            break;
        }
    }
    rows.truncate(keep);
    rows
}

/// Cut a piece that is in date order where a stash might still come in.
/// [`crate::with_stashes`] puts the row of a stash where its date says, as
/// long as the commit it was made on is in the graph. For a stash whose
/// commit the walk has not reached that is not known yet, so the piece
/// ends above the first row such a stash could go above. Stashes made on
/// commits of the piece are no trouble, and neither is any in topological
/// order, where a stash stands right above its commit.
pub fn hold_back_for_stashes(repo: &Repository, rows: &mut Vec<GraphCommit>, stashes: &[Oid]) {
    let r = repo.gix();
    // Per stash: its time and the commit it was made on.
    let mut open: Vec<(i64, Oid)> = stashes
        .iter()
        .filter_map(|id| {
            let commit = r.find_commit(*id.as_gix()).ok()?;
            let parent = commit.parent_ids().next()?.detach();
            // Without a time it sorts below every commit and never goes
            // anywhere by its date.
            let seconds = commit.time().ok()?.seconds;
            Some((seconds, Oid::from(parent)))
        })
        .collect();
    let stashes = open.clone();
    // Cutting can take away the commit another stash was made on, which
    // then is one whose place is not known either: until nothing changes.
    loop {
        open.clone_from(&stashes);
        for commit in rows.iter() {
            open.retain(|(_, parent)| *parent != commit.id);
            if open.is_empty() {
                return;
            }
        }
        let Some(newest) = open.iter().map(|(seconds, _)| *seconds).max() else {
            return;
        };
        match rows
            .iter()
            .position(|commit| commit.seconds.is_none_or(|seconds| seconds <= newest))
        {
            Some(cut) => rows.truncate(cut),
            None => return,
        }
    }
}

/// Walk at most `first` commits and say whether that was all of them.
pub fn first(repo: &Repository, options: &LogOptions, first: usize) -> Result<First> {
    let more = first.saturating_add(1);
    let limited = LogOptions {
        limit: Some(options.limit.map_or(more, |limit| limit.min(more))),
        ..options.clone()
    };
    let walked = walk_tolerant(repo, &limited, &mut |_| true)?.unwrap_or_default();
    if walked.len() <= first {
        return Ok(First::Whole(ordered(repo, walked, options.order)));
    }
    let seen = walked.get(..first).unwrap_or(&walked);
    Ok(First::Piece {
        rows: stable_rows(seen, options.order),
        walked: seen.len(),
    })
}

/// Walk everything, handing out a piece each time the number of commits
/// walked reaches the next size: `first` times [`GROWTH`], and so on.
/// `on_piece` is given the rows that are certain and how many commits were
/// walked; `go_on` is asked now and then and stops the walk by saying no,
/// in which case there is no result. Otherwise every commit, in graph
/// order.
pub fn walk_in_pieces(
    repo: &Repository,
    options: &LogOptions,
    first: usize,
    go_on: &dyn Fn() -> bool,
    on_piece: &mut dyn FnMut(Vec<GraphCommit>, usize),
) -> Result<Option<Vec<GraphCommit>>> {
    let order = options.order;
    let mut next = first.max(1).saturating_mul(GROWTH);
    let walked = walk_tolerant(repo, options, &mut |so_far: &[GraphCommit]| {
        let count = so_far.len();
        // Asked every few thousand commits: often enough to stop within
        // milliseconds, seldom enough to cost nothing.
        if count.trailing_zeros() >= 12 && !go_on() {
            return false;
        }
        if count >= next {
            next = next.saturating_mul(GROWTH);
            let rows = stable_rows(so_far, order);
            if rows.len() >= SMALLEST_PIECE {
                on_piece(rows, count);
            }
        }
        true
    })?;
    Ok(match walked {
        Some(walked) if go_on() => Some(ordered(repo, walked, order)),
        _ => None,
    })
}

/// [`crate::walk`] that gives up when `stop` is set, which it looks at
/// every few thousand commits: no result then.
pub fn walk_unless(
    repo: &Repository,
    options: &LogOptions,
    stop: &std::sync::atomic::AtomicBool,
) -> Result<Option<Vec<GraphCommit>>> {
    walk_in_pieces(
        repo,
        options,
        usize::MAX,
        &|| !stop.load(std::sync::atomic::Ordering::SeqCst),
        &mut |_, _| {},
    )
}

/// [`walk_by_time`] from the tips the options name, without the ones that
/// are no commits when those stop it, as [`crate::walk`] does.
fn walk_tolerant(
    repo: &Repository,
    options: &LogOptions,
    each: &mut dyn FnMut(&[GraphCommit]) -> bool,
) -> Result<Option<Vec<GraphCommit>>> {
    let tips = resolve_tips(repo, &options.tips)?;
    match walk_by_time(repo, &tips, options, each) {
        Ok(walked) => Ok(walked),
        Err(first) => match commit_tips(repo, &tips) {
            Some(commits_only) => walk_by_time(repo, &commits_only, options, each),
            None => Err(first),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    /// The two orders that need nothing but the commits themselves.
    fn plain(commits: Vec<GraphCommit>, order: Order) -> Vec<GraphCommit> {
        match order {
            Order::Topo => crate::log::topological_order(commits),
            Order::Date | Order::AuthorDate => crate::log::topological_repair(commits),
        }
    }

    fn oid(n: u32) -> Oid {
        format!("{n:08x}").repeat(5).parse().unwrap()
    }

    /// A commit with a time in days, far from zero.
    fn commit(id: u32, parents: &[u32], day: i64) -> GraphCommit {
        GraphCommit {
            id: oid(id),
            parents: parents.iter().map(|p| oid(*p)).collect(),
            seconds: Some(1_000_000_000 + day * DAY),
        }
    }

    fn ids(commits: &[GraphCommit]) -> Vec<Oid> {
        commits.iter().map(|commit| commit.id).collect()
    }

    /// A history of two lines that take turns, one commit a day, newest
    /// first as a walk comes to it: `n` commits on each line over a root.
    fn two_lines(n: u32) -> Vec<GraphCommit> {
        let mut out = Vec::new();
        for step in (1..=n).rev() {
            let day = i64::from(step) * 2;
            let below = |line: u32| if step == 1 { 0 } else { line + (step - 1) * 10 };
            out.push(commit(1 + step * 10, &[below(1)], day + 1));
            out.push(commit(2 + step * 10, &[below(2)], day));
        }
        out.push(commit(0, &[], 0));
        out
    }

    #[test]
    fn every_piece_is_the_beginning_of_the_whole() {
        let all = two_lines(40);
        for order in [Order::Date, Order::Topo] {
            let whole = plain(all.clone(), order);
            let mut longest = 0;
            for walked in 1..all.len() {
                let rows = stable_rows(&all[..walked], order);
                assert!(rows.len() <= walked);
                assert_eq!(
                    ids(&rows),
                    ids(&whole[..rows.len()]),
                    "{order:?}: the piece after {walked} commits"
                );
                longest = longest.max(rows.len());
            }
            // By date nearly everything walked is certain; in topological
            // order a piece ends where the newest line leaves what is
            // walked.
            match order {
                Order::Date => assert!(longest > all.len() - 10, "{longest}"),
                Order::Topo => assert!(longest > 30, "{longest}"),
                Order::AuthorDate => assert_eq!(longest, 0),
            }
        }
    }

    #[test]
    fn rows_near_where_the_walk_has_reached_are_held_back() {
        // One commit a day, linear. What is within the margin of the
        // oldest commit walked is not handed out.
        let all: Vec<GraphCommit> = (1..=20u32)
            .rev()
            .map(|n| commit(n, &[n - 1], i64::from(n)))
            .collect();
        let rows = stable_rows(&all[..10], Order::Date);
        // Walked: days 20 down to 11. Certain: newer than day 11 + 3.
        assert_eq!(ids(&rows), ids(&all[..6]));
        // A clock that is a day off does no harm: a commit that claims to
        // be a day older than its parent is walked late, and its parent
        // was held back for it. Commit 99 is a child of commit 15, dated
        // like commit 13.
        let mut skewed = all.clone();
        skewed.insert(8, commit(99, &[15], 13));
        let whole = plain(skewed.clone(), Order::Date);
        for walked in 1..skewed.len() {
            let rows = stable_rows(&skewed[..walked], Order::Date);
            assert_eq!(ids(&rows), ids(&whole[..rows.len()]), "after {walked}");
        }
    }

    #[test]
    fn nothing_is_certain_without_times_or_without_commits() {
        assert!(stable_rows(&[], Order::Date).is_empty());
        let mut all = two_lines(5);
        all[3].seconds = None;
        assert!(stable_rows(&all, Order::Date).is_empty());
        assert!(stable_rows(&all, Order::Topo).is_empty());
        // Times at the ends of what a number holds do not overflow.
        let far = vec![
            GraphCommit {
                id: oid(2),
                parents: vec![oid(1)],
                seconds: Some(i64::MAX),
            },
            GraphCommit {
                id: oid(1),
                parents: vec![],
                seconds: Some(i64::MAX - 1),
            },
        ];
        assert!(stable_rows(&far, Order::Date).is_empty());
        let near = vec![GraphCommit {
            id: oid(1),
            parents: vec![],
            seconds: Some(i64::MIN),
        }];
        assert!(stable_rows(&near, Order::Topo).is_empty());
    }
}
