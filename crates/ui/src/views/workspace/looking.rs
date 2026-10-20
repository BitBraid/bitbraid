//! When the other worktrees are looked at for what is uncommitted in them:
//! for their rows in the graph and their marks in the worktrees section.
//!
//! A look is a `git status` in a worktree ([`bitbraid_git::wip::LOOK`]),
//! which in a large working tree takes a second or more of a processor.
//! With editors or coding tools at work in several worktrees all the
//! time, a look at each whenever its index is written would have a git
//! status running nearly all the time. In the background, therefore:
//!
//! - a worktree is looked at when its stamp (its HEAD, the size and time
//!   of its index) is not what it was, or while the worktrees section is
//!   open (an edit that writes no index shows there too), and then at most
//!   once every [`EVERY`], whichever tab looks;
//! - one look runs at a time in the whole application, at one worktree,
//!   with a pause after it as long as it took and [`PAUSE`] at least, so
//!   that looks at several worktrees never run back to back;
//! - what a look found is for every tab to use: the tab of one worktree
//!   and the tab of another of the same repository do not each look at a
//!   third.
//!
//! When the user asks (F5, the section's button, the section coming into
//! view), every other worktree is looked at at once, in one go, though
//! never while another look runs. The tab's own worktree is not looked at
//! here: its status is read as soon as anything changes in it, untracked
//! files and all.
//!
//! Sources: git-status(1) (`--untracked-files`), git-worktree(1).

use std::{
    collections::{BTreeSet, HashMap, VecDeque},
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use bitbraid_git::wip::Counts;
use gpui_kit::*;
use serde_json::json;

use super::Workspace;
use crate::{
    prefs::{Prefs, SeenWorktree},
    session::{Reply, Request},
};

/// The least time between two looks at one worktree in the background.
pub const EVERY: Duration = Duration::from_secs(60);

/// The least pause after a look before the next one begins in the
/// background; after a look that took longer, as long as it took.
pub const PAUSE: Duration = Duration::from_secs(2);

/// How many looks are kept for the probes, and how many worktrees are
/// remembered at most.
const KEPT: usize = 256;
const REMEMBERED: usize = 4096;

/// What a look at a worktree found: under which stamp, and how many files
/// of each kind are changed. Nothing when git could not say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    pub stamp: String,
    pub counts: Counts,
}

/// One look, as the probes are told of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Look {
    /// The worktrees it looked at.
    pub paths: Vec<String>,
    pub began: Instant,
    pub ended: Instant,
    /// Whether the user asked for it.
    pub asked: bool,
}

/// The look that is under way: since when, and whether the tab that
/// started it is still waiting for it (a tab that is closed while its look
/// runs never says it ended).
#[derive(Debug)]
struct Running {
    began: Instant,
    alive: Weak<()>,
}

/// The application's record of looks at worktrees, shared by its tabs.
#[derive(Debug, Default)]
pub struct Looks {
    /// Per worktree, by path: when it was last looked at, and what that
    /// found.
    last: HashMap<String, (Instant, Option<Found>)>,
    running: Option<Running>,
    /// No look begins in the background before this.
    rest_until: Option<Instant>,
    /// The last looks, oldest first.
    kept: VecDeque<Look>,
    /// How many looks began while another was under way: none.
    overlaps: u64,
}

impl Global for Looks {}

/// What a tab is to do about the other worktrees now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Plan {
    /// Nothing: nothing changed that is not seen, or a look of its own is
    /// on its way.
    Nothing,
    /// Look now, so.
    Now(Ask),
    /// Plan again after this long.
    After(Duration),
}

/// One look, as the engine is asked for it (`worktree.changes`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ask {
    /// Worktrees not to be looked at now: they were a moment ago.
    pub skip: Vec<String>,
    /// How many to look at; all that changed when absent.
    pub at_most: Option<usize>,
    /// Worktrees looked at whatever their stamp says: all of them, or
    /// these.
    pub regardless: Regardless,
    /// Whether the user asked.
    pub asked: bool,
}

/// Which worktrees are looked at whatever their stamp says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Regardless {
    All,
    These(Vec<String>),
}

/// What a tab wants of the looks at its other worktrees.
#[derive(Debug, Default)]
pub struct Wants {
    /// Something may have changed in them since its last look began: a
    /// write in their own files, a reload.
    pub wanted: bool,
    /// The user asked: every one of them, now.
    pub afresh: bool,
    /// Its look is on its way.
    mine: bool,
    /// When the timer it set goes off, when one is set.
    timer: Option<Instant>,
    /// Those that changed and are not looked at yet, by path.
    waiting: BTreeSet<String>,
    /// How many other worktrees the references listed when the last
    /// answer came: one more is one that was never looked at.
    answered_for: Option<usize>,
}

impl Looks {
    /// Whether the worktree at `path` was looked at less than [`EVERY`]
    /// ago.
    fn recent(&self, path: &str, now: Instant) -> bool {
        self.last
            .get(path)
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) < EVERY)
    }

    /// Whether a look is under way.
    fn busy(&self) -> bool {
        self.running
            .as_ref()
            .is_some_and(|running| running.alive.strong_count() > 0)
    }

    /// What a tab is to do now. `seen` are the other worktrees it knows of
    /// (by path, as the engine names them), `others` how many the
    /// references list, and `open` whether the worktrees section is open.
    pub fn plan(
        &self,
        now: Instant,
        wants: &Wants,
        seen: &[String],
        others: usize,
        open: bool,
    ) -> Plan {
        if wants.mine {
            return Plan::Nothing;
        }
        let something = wants.afresh || wants.wanted || !wants.waiting.is_empty();
        if !something {
            return Plan::Nothing;
        }
        // Never two at once: not even when the user asks.
        if self.busy() {
            return Plan::After(PAUSE);
        }
        if wants.afresh {
            return Plan::Now(Ask {
                skip: Vec::new(),
                at_most: None,
                regardless: Regardless::All,
                asked: true,
            });
        }
        if let Some(until) = self.rest_until.filter(|until| *until > now) {
            return Plan::After(until.saturating_duration_since(now));
        }
        // After a write somewhere, any of them may have changed; else
        // those that did and were left for later.
        let candidates: Vec<&String> = if wants.wanted {
            seen.iter().collect()
        } else {
            wants.waiting.iter().collect()
        };
        let unknown = wants.wanted && wants.answered_for != Some(others);
        let due: Vec<String> = candidates
            .iter()
            .filter(|path| !self.recent(path, now))
            .map(|path| (*path).clone())
            .collect();
        if unknown || !due.is_empty() {
            let skip = seen
                .iter()
                .filter(|path| self.recent(path, now))
                .cloned()
                .collect();
            return Plan::Now(Ask {
                skip,
                at_most: Some(1),
                regardless: Regardless::These(if open { due } else { Vec::new() }),
                asked: false,
            });
        }
        // Each was looked at a moment ago: when the first may be again.
        candidates
            .iter()
            .filter_map(|path| self.last.get(path.as_str()))
            .filter_map(|(at, _)| at.checked_add(EVERY))
            .min()
            .map(|at| Plan::After(at.saturating_duration_since(now)))
            .unwrap_or(Plan::Nothing)
    }

    /// A look begins: what the tab holds until it has ended.
    pub fn began(&mut self, now: Instant) -> Arc<()> {
        if self.busy() {
            self.overlaps = self.overlaps.saturating_add(1);
        }
        let token = Arc::new(());
        self.running = Some(Running {
            began: now,
            alive: Arc::downgrade(&token),
        });
        token
    }

    /// The look that began at `began` has ended, having looked at these.
    pub fn ended(
        &mut self,
        began: Instant,
        now: Instant,
        looked: Vec<(String, Option<Found>)>,
        asked: bool,
    ) {
        if self
            .running
            .as_ref()
            .is_some_and(|running| running.began == began)
        {
            self.running = None;
        }
        let took = now.saturating_duration_since(began);
        self.rest_until = now.checked_add(took.max(PAUSE));
        if self.last.len() > REMEMBERED {
            self.last
                .retain(|_, (at, _)| now.saturating_duration_since(*at) < EVERY);
        }
        let paths = looked.iter().map(|(path, _)| path.clone()).collect();
        for (path, found) in looked {
            self.last.insert(path, (began, found));
        }
        if self.kept.len() >= KEPT {
            self.kept.pop_front();
        }
        self.kept.push_back(Look {
            paths,
            began,
            ended: now,
            asked,
        });
    }

    /// What the last look at the worktree at `path` found.
    fn found(&self, path: &str) -> Option<&Found> {
        self.last.get(path).and_then(|(_, found)| found.as_ref())
    }

    /// The last looks, oldest first, and how many began while another ran.
    pub fn looks(&self) -> (Vec<Look>, u64) {
        (self.kept.iter().cloned().collect(), self.overlaps)
    }
}

/// What a tab knows of its other worktrees after an answer of the engine,
/// and which of them changed without being looked at. One that was looked
/// at has what was found; one that was passed over keeps what it had; one
/// that is waiting has what the last look at it found, from any tab, and
/// keeps that look's stamp, so that it is asked for again.
pub fn merged(
    value: &serde_json::Value,
    before: &[SeenWorktree],
    looks: &Looks,
) -> (Vec<SeenWorktree>, BTreeSet<String>) {
    let mut waiting = BTreeSet::new();
    let seen = value
        .as_array()
        .into_iter()
        .flatten()
        .take(bitbraid_git::wip::MAX_SCANNED)
        .filter_map(|entry| {
            let path = entry.get("path")?.as_str()?.to_owned();
            let stamp = entry.get("stamp").and_then(|stamp| stamp.as_str())?;
            let flag = |key: &str| entry.get(key).and_then(|flag| flag.as_bool()) == Some(true);
            let counted: Option<Counts> = entry
                .get("counts")
                .filter(|counts| counts.is_object())
                .and_then(|counts| serde_json::from_value(counts.clone()).ok());
            let was = before.iter().find(|seen| seen.path == path);
            let shared = looks.found(&path);
            let kept = |found: Option<(&str, Counts)>| SeenWorktree {
                path: path.clone(),
                stamp: found.map(|(stamp, _)| stamp.to_owned()).unwrap_or_default(),
                counts: found.map(|(_, counts)| counts).unwrap_or_default(),
            };
            Some(match (counted, flag("scanned"), flag("waiting")) {
                (Some(counts), true, _) => kept(Some((stamp, counts))),
                // Looked at and git could not say: no stamp, to be asked
                // again.
                (None, true, _) => kept(None),
                // Passed over: what it had, under the stamp it still has.
                (_, false, false) => kept(Some((
                    stamp,
                    was.map(|seen| seen.counts).unwrap_or_default(),
                ))),
                // Another tab looked since it changed.
                (_, false, true) if shared.is_some_and(|found| found.stamp == stamp) => {
                    kept(shared.map(|found| (found.stamp.as_str(), found.counts)))
                }
                (_, false, true) => {
                    waiting.insert(path.clone());
                    let last = shared
                        .map(|found| (found.stamp.as_str(), found.counts))
                        .or_else(|| was.map(|seen| (seen.stamp.as_str(), seen.counts)));
                    kept(last)
                }
            })
        })
        .collect();
    (seen, waiting)
}

/// The worktrees an answer says were looked at, with what was found.
fn looked_in(value: &serde_json::Value) -> Vec<(String, Option<Found>)> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .take(bitbraid_git::wip::MAX_SCANNED)
        .filter(|entry| entry.get("scanned").and_then(|flag| flag.as_bool()) == Some(true))
        .filter_map(|entry| {
            let path = entry.get("path")?.as_str()?.to_owned();
            let stamp = entry.get("stamp")?.as_str()?.to_owned();
            let counts: Option<Counts> = entry
                .get("counts")
                .filter(|counts| counts.is_object())
                .and_then(|counts| serde_json::from_value(counts.clone()).ok());
            Some((path, counts.map(|counts| Found { stamp, counts })))
        })
        .collect()
}

/// The worktrees that have a row in the graph: those with changes.
pub fn rows_of(seen: &[SeenWorktree]) -> Vec<&str> {
    seen.iter()
        .filter(|seen| seen.counts.total() > 0)
        .map(|seen| seen.path.as_str())
        .collect()
}

impl Workspace {
    /// How many worktrees other than this one the references list.
    fn other_worktrees(&self) -> usize {
        self.snapshot.as_ref().map_or(0, |snapshot| {
            snapshot
                .worktrees
                .iter()
                .filter(|worktree| !worktree.current && worktree.path.is_some())
                .count()
        })
    }

    /// Something may have changed in the other worktrees: look at them
    /// again when it may be. `afresh`: the user asked, every one now.
    pub(super) fn want_worktree_looks(&mut self, afresh: bool, cx: &mut Context<Self>) {
        if self.other_worktrees() == 0 {
            return;
        }
        self.worktree_wants.wanted = true;
        self.worktree_wants.afresh |= afresh;
        self.look_at_worktrees(cx);
    }

    /// Do what the plan says.
    fn look_at_worktrees(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let seen = cx
            .global::<Prefs>()
            .repo_state(&self.session.path)
            .worktrees_seen;
        let paths: Vec<String> = seen.iter().map(|seen| seen.path.clone()).collect();
        let open = self.refs_view.read(cx).worktrees_open();
        let others = self.other_worktrees();
        // Read without being written, which would tell every tab.
        let wants = &self.worktree_wants;
        let plan = match cx.try_global::<Looks>() {
            Some(looks) => looks.plan(now, wants, &paths, others, open),
            None => Looks::default().plan(now, wants, &paths, others, open),
        };
        match plan {
            Plan::Nothing => {}
            Plan::After(wait) => self.look_later(now, wait, cx),
            Plan::Now(ask) => self.look_now(ask, &seen, now, cx),
        }
    }

    /// Plan again after `wait`, unless a timer that goes off sooner is set.
    fn look_later(&mut self, now: Instant, wait: Duration, cx: &mut Context<Self>) {
        let Some(at) = now.checked_add(wait) else {
            return;
        };
        if self.worktree_wants.timer.is_some_and(|set| set <= at) {
            return;
        }
        self.worktree_wants.timer = Some(at);
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            let _ = this.update(cx, |this, cx| {
                if this.worktree_wants.timer == Some(at) {
                    this.worktree_wants.timer = None;
                }
                this.look_at_worktrees(cx);
            });
        })
        .detach();
    }

    /// Ask the engine to look, as planned.
    fn look_now(&mut self, ask: Ask, seen: &[SeenWorktree], now: Instant, cx: &mut Context<Self>) {
        let wants = &mut self.worktree_wants;
        wants.mine = true;
        wants.wanted = false;
        if ask.asked {
            wants.afresh = false;
        }
        let token = cx.default_global::<Looks>().began(now);
        let known: serde_json::Map<String, serde_json::Value> = match &ask.regardless {
            Regardless::All => Default::default(),
            Regardless::These(paths) => seen
                .iter()
                .filter(|seen| !seen.stamp.is_empty() && !paths.contains(&seen.path))
                .map(|seen| (seen.path.clone(), json!(seen.stamp)))
                .collect(),
        };
        let mut params = json!({ "known": known, "skip": ask.skip });
        if let (Some(at_most), Some(object)) = (ask.at_most, params.as_object_mut()) {
            object.insert("at_most".to_owned(), json!(at_most));
        }
        let request = Request::Command {
            id: "worktree.changes".to_owned(),
            params,
        };
        let others = self.other_worktrees();
        let asked = ask.asked;
        self.session.call(request, cx, move |this, reply, cx| {
            let ended = cx.background_executor().now();
            this.worktree_wants.mine = false;
            this.worktree_wants.answered_for = Some(others);
            let value = match reply {
                Reply::Command(value) => Some(value),
                _ => None,
            };
            let looked = value.as_ref().map(looked_in).unwrap_or_default();
            cx.default_global::<Looks>()
                .ended(now, ended, looked, asked);
            drop(token);
            match value {
                Some(value) => this.take_worktree_answer(&value, cx),
                // Asked again when something changes, not at once.
                None => this.worktree_wants.waiting.clear(),
            }
            this.look_at_worktrees(cx);
        });
    }

    /// Keep and show what an answer says about the other worktrees.
    fn take_worktree_answer(&mut self, value: &serde_json::Value, cx: &mut Context<Self>) {
        let before = cx
            .global::<Prefs>()
            .repo_state(&self.session.path)
            .worktrees_seen;
        let (now, waiting) = match cx.try_global::<Looks>() {
            Some(looks) => merged(value, &before, looks),
            None => merged(value, &before, &Looks::default()),
        };
        self.worktree_wants.waiting = waiting;
        self.keep_worktrees_seen(&before, now, cx);
    }

    /// What a look of another tab found, taken over: of a worktree this
    /// tab knows from an older look, the newer one's stamp and counts.
    pub(super) fn adopt_shared_looks(&mut self, cx: &mut Context<Self>) {
        let Some(looks) = cx.try_global::<Looks>() else {
            return;
        };
        let before = cx
            .global::<Prefs>()
            .repo_state(&self.session.path)
            .worktrees_seen;
        let now: Vec<SeenWorktree> = before
            .iter()
            .map(|seen| match looks.found(&seen.path) {
                Some(found) if found.stamp != seen.stamp => SeenWorktree {
                    path: seen.path.clone(),
                    stamp: found.stamp.clone(),
                    counts: found.counts,
                },
                _ => seen.clone(),
            })
            .collect();
        self.keep_worktrees_seen(&before, now, cx);
    }

    /// Keep what is now known of the other worktrees, and show it.
    fn keep_worktrees_seen(
        &mut self,
        before: &[SeenWorktree],
        now: Vec<SeenWorktree>,
        cx: &mut Context<Self>,
    ) {
        if now == before {
            return;
        }
        let rows_changed = rows_of(&now) != rows_of(before);
        let path = self.session.path.clone();
        let kept = now.clone();
        cx.global_mut::<Prefs>()
            .update_repo_state(&path, |state| state.worktrees_seen = kept);
        self.show_worktrees_seen(&now, cx);
        // Another set of worktrees has rows in the graph.
        if rows_changed {
            self.load_graph(cx);
        }
    }

    /// The looks so far, for the probes, timed from the first one; and how
    /// many began while another ran.
    pub(super) fn worktree_looks(cx: &App) -> (Vec<super::WorktreeLook>, u64) {
        let Some(looks) = cx.try_global::<Looks>() else {
            return (Vec::new(), 0);
        };
        let (kept, overlaps) = looks.looks();
        let Some(origin) = kept.first().map(|look| look.began) else {
            return (Vec::new(), overlaps);
        };
        let facts = kept
            .into_iter()
            .map(|look| super::WorktreeLook {
                paths: look.paths,
                began: look.began.saturating_duration_since(origin),
                ended: look.ended.saturating_duration_since(origin),
                asked: look.asked,
            })
            .collect();
        (facts, overlaps)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::time::{Duration, Instant};

    use bitbraid_git::wip::Counts;
    use serde_json::json;

    // Named one by one: the toolkit's names, which `super` has, include a
    // `test` of its own.
    use super::{
        Ask, EVERY, Found, KEPT, Looks, PAUSE, Plan, REMEMBERED, Regardless, Wants, looked_in,
        merged, rows_of,
    };
    use crate::prefs::SeenWorktree;

    fn wants() -> Wants {
        Wants::default()
    }

    fn seen(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|path| (*path).to_owned()).collect()
    }

    fn found(stamp: &str, modified: usize) -> Option<Found> {
        Some(Found {
            stamp: stamp.to_owned(),
            counts: Counts {
                modified,
                ..Counts::default()
            },
        })
    }

    #[test]
    fn nothing_is_looked_at_while_nothing_changed() {
        let looks = Looks::default();
        let now = Instant::now();
        assert_eq!(
            looks.plan(now, &wants(), &seen(&["/a"]), 1, false),
            Plan::Nothing
        );
        let mine = Wants {
            wanted: true,
            mine: true,
            ..wants()
        };
        assert_eq!(
            looks.plan(now, &mine, &seen(&["/a"]), 1, false),
            Plan::Nothing
        );
    }

    #[test]
    fn a_worktree_is_looked_at_once_a_minute_one_at_a_time_with_a_pause() {
        let mut looks = Looks::default();
        let start = Instant::now();
        let paths = seen(&["/a", "/b"]);
        let mut wants = Wants {
            wanted: true,
            ..wants()
        };
        // Never looked at: now, one of them, the stamps told.
        let Plan::Now(ask) = looks.plan(start, &wants, &paths, 2, false) else {
            panic!("a look now");
        };
        assert_eq!(ask.at_most, Some(1));
        assert!(ask.skip.is_empty());
        assert_eq!(ask.regardless, Regardless::These(Vec::new()));
        // While it runs nothing else begins, in any tab.
        let token = looks.began(start);
        let other = Wants {
            wanted: true,
            ..Wants::default()
        };
        assert_eq!(
            looks.plan(start, &other, &paths, 2, false),
            Plan::After(PAUSE)
        );
        let afresh = Wants {
            afresh: true,
            ..Wants::default()
        };
        assert_eq!(
            looks.plan(start, &afresh, &paths, 2, false),
            Plan::After(PAUSE)
        );
        // It took three seconds and looked at `/a`; `/b` changed too.
        let ended = start + Duration::from_secs(3);
        looks.ended(start, ended, vec![("/a".to_owned(), found("s1", 1))], false);
        drop(token);
        wants.wanted = false;
        wants.answered_for = Some(2);
        wants.waiting = ["/b".to_owned()].into();
        // A pause as long as it took, then `/b`, with `/a` left out.
        assert_eq!(
            looks.plan(ended, &wants, &paths, 2, false),
            Plan::After(Duration::from_secs(3))
        );
        let later = ended + Duration::from_secs(3);
        let Plan::Now(ask) = looks.plan(later, &wants, &paths, 2, false) else {
            panic!("a look at the one that waits");
        };
        assert_eq!(ask.skip, ["/a"]);
        let token = looks.began(later);
        looks.ended(later, later, vec![("/b".to_owned(), found("s1", 0))], false);
        drop(token);
        // Both changed again at once: not before a minute after each look.
        wants.waiting = ["/a".to_owned(), "/b".to_owned()].into();
        let soon = later + PAUSE;
        assert_eq!(
            looks.plan(soon, &wants, &paths, 2, false),
            Plan::After(start + EVERY - soon)
        );
        wants.waiting.clear();
        wants.wanted = true;
        assert_eq!(
            looks.plan(soon, &wants, &paths, 2, false),
            Plan::After(start + EVERY - soon)
        );
        // A minute after the first look: `/a` again, `/b` still left out.
        let Plan::Now(ask) = looks.plan(start + EVERY, &wants, &paths, 2, false) else {
            panic!("a look a minute later");
        };
        assert_eq!(ask.skip, ["/b"]);
        assert_eq!(looks.looks().1, 0);
    }

    #[test]
    fn the_user_asks_for_all_at_once_and_the_open_section_for_each_regardless() {
        let mut looks = Looks::default();
        let now = Instant::now();
        let paths = seen(&["/a", "/b"]);
        let afresh = Wants {
            afresh: true,
            ..wants()
        };
        // Even right after another look: every one, whatever the stamps.
        let token = looks.began(now);
        looks.ended(now, now, vec![("/a".to_owned(), found("s", 0))], false);
        drop(token);
        assert_eq!(
            looks.plan(now, &afresh, &paths, 2, false),
            Plan::Now(Ask {
                skip: Vec::new(),
                at_most: None,
                regardless: Regardless::All,
                asked: true,
            })
        );
        // The section open: those not looked at for a minute are looked
        // at whatever their stamps say.
        let wanted = Wants {
            wanted: true,
            answered_for: Some(2),
            ..wants()
        };
        let Plan::Now(ask) = looks.plan(now + PAUSE, &wanted, &paths, 2, true) else {
            panic!("a look");
        };
        assert_eq!(ask.regardless, Regardless::These(seen(&["/b"])));
        assert_eq!(ask.skip, ["/a"]);
        // Only `/a` known, and looked at a moment ago: nothing before a
        // minute is up, unless the references list a worktree more than
        // when the last answer came, which is looked for at once.
        let known = seen(&["/a"]);
        assert_eq!(
            looks.plan(now + PAUSE, &wanted, &known, 2, false),
            Plan::After(EVERY - PAUSE)
        );
        let Plan::Now(ask) = looks.plan(now + PAUSE, &wanted, &known, 3, false) else {
            panic!("a look for the new one");
        };
        assert_eq!(ask.skip, ["/a"]);
    }

    #[test]
    fn a_look_whose_tab_is_gone_holds_nothing_up() {
        let mut looks = Looks::default();
        let now = Instant::now();
        let token = looks.began(now);
        let wanted = Wants {
            wanted: true,
            ..wants()
        };
        assert_eq!(looks.plan(now, &wanted, &[], 1, false), Plan::After(PAUSE));
        drop(token);
        assert!(matches!(
            looks.plan(now, &wanted, &[], 1, false),
            Plan::Now(_)
        ));
        // An end that is not of the look that runs leaves that one running.
        let token = looks.began(now);
        looks.ended(now - Duration::from_secs(1), now, Vec::new(), false);
        assert!(looks.busy());
        looks.ended(now, now, Vec::new(), false);
        assert!(!looks.busy());
        drop(token);
    }

    #[test]
    fn what_is_known_follows_what_was_looked_at_by_any_tab() {
        let entry = |path: &str, stamp: &str, scanned: bool, waiting: bool| json!({ "path": path, "stamp": stamp, "scanned": scanned, "waiting": waiting, "counts": null });
        let was = |path: &str, stamp: &str, modified: usize| SeenWorktree {
            path: path.to_owned(),
            stamp: stamp.to_owned(),
            counts: Counts {
                modified,
                ..Counts::default()
            },
        };
        let before = vec![
            was("/a", "s1", 2),
            was("/b", "s1", 0),
            was("/c", "s1", 4),
            was("/gone", "s1", 5),
        ];
        let mut looks = Looks::default();
        let now = Instant::now();
        // Another tab looked at `/c` under the stamp it has now.
        looks.ended(now, now, vec![("/c".to_owned(), found("s3", 1))], false);
        let answer = json!([
            { "path": "/a", "stamp": "s2", "scanned": true, "waiting": false,
              "counts": { "modified": 0, "added": 1, "deleted": 0, "renamed": 0 } },
            entry("/b", "s1", false, false),
            entry("/c", "s3", false, true),
            entry("/d", "s9", false, true),
            entry("/e", "s9", true, false),
            { "path": 7 },
            "noise",
        ]);
        let (seen, waiting) = merged(&answer, &before, &looks);
        assert_eq!(
            seen,
            [
                SeenWorktree {
                    path: "/a".to_owned(),
                    stamp: "s2".to_owned(),
                    counts: Counts {
                        added: 1,
                        ..Counts::default()
                    },
                },
                was("/b", "s1", 0),
                was("/c", "s3", 1),
                // Waiting and never looked at: nothing, no stamp.
                was("/d", "", 0),
                // Looked at, git could not say: asked again.
                was("/e", "", 0),
            ]
        );
        assert_eq!(waiting, ["/d".to_owned()].into());
        // Waiting, last looked at by this tab: what it had, under the
        // stamp it had, so that it stays changed.
        let (seen, waiting) = merged(&json!([entry("/a", "s5", false, true)]), &before, &looks);
        assert_eq!(seen, [was("/a", "s1", 2)]);
        assert_eq!(waiting, ["/a".to_owned()].into());
        // A row for each worktree that has changes, in the order listed.
        assert_eq!(rows_of(&before), ["/a", "/c", "/gone"]);
        // No list, or a very long one: nothing, or as many as are ever
        // looked at.
        assert!(
            merged(&json!({ "path": "/a" }), &before, &looks)
                .0
                .is_empty()
        );
        let many: Vec<serde_json::Value> = (0..500)
            .map(|n| entry(&format!("/w/{n}"), "s", false, false))
            .collect();
        assert_eq!(
            merged(&json!(many), &[], &looks).0.len(),
            bitbraid_git::wip::MAX_SCANNED
        );
        assert_eq!(
            looked_in(&answer),
            [
                (
                    "/a".to_owned(),
                    Some(Found {
                        stamp: "s2".to_owned(),
                        counts: Counts {
                            added: 1,
                            ..Counts::default()
                        }
                    })
                ),
                ("/e".to_owned(), None)
            ]
        );
    }

    #[test]
    fn the_record_stays_bounded() {
        let mut looks = Looks::default();
        let start = Instant::now();
        for n in 0..(REMEMBERED + KEPT) {
            let at = start + Duration::from_secs(n as u64);
            looks.ended(at, at, vec![(format!("/w/{n}"), None)], false);
        }
        assert!(looks.kept.len() <= KEPT);
        assert!(looks.last.len() <= REMEMBERED + 1);
    }
}
