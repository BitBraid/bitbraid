//! Submodule requests: the menus of the references panel make them, the
//! workspace asks where it has to and runs them.
//!
//! Where a submodule stands is read again as files change in it:
//! [`touches`] says whether a batch of changes can have moved it, and
//! [`Paced`] keeps the reads apart, so that a build writing a thousand
//! files in a submodule is a read every few seconds and one at the end,
//! not a thousand.
//!
//! Sources: gpui-kit documentation (popup menu); gitsubmodules(7) (a
//! submodule's repository lives under the superproject's `.git/modules`).

use std::time::{Duration, Instant};

use bitbraid_git::{Area, ChangeSet};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use serde::Deserialize;

/// One request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum SubmoduleRequest {
    /// Ask for a URL and a folder and add a submodule.
    Add,
    /// Open the submodule at this path in a tab of its own.
    Open(String),
    /// Check out what is recorded: for the one at this path, or for all;
    /// those not cloned yet are cloned.
    Update(Option<String>),
    /// Take the URLs from `.gitmodules` over: for one, or for all.
    Sync(Option<String>),
    /// Ask, then remove the submodule at this path.
    Remove(String),
}

/// Whether a batch of changes can have changed where a submodule stands:
/// something under `.git` (a submodule's own repository is in there, and
/// what the index records for it is too), or a file inside one of the
/// submodules' folders. A batch that names no paths, or as many as it can
/// hold, may have left some out and counts.
pub fn touches(set: &ChangeSet, submodules: &[&str]) -> bool {
    /// As many paths as a batch names at most.
    const MOST_PATHS: usize = 64;
    if submodules.is_empty() {
        return false;
    }
    let inside_git = set
        .areas
        .iter()
        .any(|area| matches!(area, Area::GitDir | Area::Refs | Area::Index));
    if inside_git {
        return true;
    }
    if !set.areas.contains(&Area::Worktree) {
        return false;
    }
    if set.paths.is_empty() || set.paths.len() >= MOST_PATHS {
        return true;
    }
    set.paths.iter().any(|path| {
        submodules
            .iter()
            .any(|submodule| path.starts_with(submodule))
    })
}

/// What to do about a read that follows changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Nothing: one is running or waiting, and will see the change.
    Nothing,
    /// Read now.
    Now,
    /// Read when this much time has passed.
    After(Duration),
}

/// Reads that follow a stream of changes, kept apart: never two at once,
/// never one sooner than `gap` after the one before began, and always one
/// after the last change.
#[derive(Debug)]
pub struct Paced {
    gap: Duration,
    /// A read is on its way.
    running: bool,
    /// A read is to start when its time has come.
    waiting: bool,
    /// Something changed since the last read began.
    pending: bool,
    /// When the last read began.
    last: Option<Instant>,
}

impl Paced {
    /// Reads at least `gap` apart.
    pub fn new(gap: Duration) -> Self {
        Self {
            gap,
            running: false,
            waiting: false,
            pending: false,
            last: None,
        }
    }

    fn due(&mut self, now: Instant) -> Next {
        let since = self.last.map(|last| now.saturating_duration_since(last));
        match since {
            Some(since) if since < self.gap => {
                self.waiting = true;
                Next::After(self.gap.saturating_sub(since))
            }
            _ => Next::Now,
        }
    }

    /// Something changed.
    pub fn asked(&mut self, now: Instant) -> Next {
        self.pending = true;
        if self.running || self.waiting {
            return Next::Nothing;
        }
        self.due(now)
    }

    /// A read begins: at once, or after the wait it was told.
    pub fn started(&mut self, now: Instant) {
        self.waiting = false;
        self.pending = false;
        self.running = true;
        self.last = Some(now);
    }

    /// The read that was running is answered.
    pub fn ended(&mut self, now: Instant) -> Next {
        self.running = false;
        if self.pending && !self.waiting {
            self.due(now)
        } else {
            Next::Nothing
        }
    }
}

/// The action a menu entry dispatches.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = submodule, no_json)]
pub struct RunSubmodule {
    /// What to do.
    pub request: SubmoduleRequest,
}

fn entry(request: SubmoduleRequest) -> Box<dyn Action> {
    Box::new(RunSubmodule { request })
}

/// The entries that are about all submodules: under the heading of their
/// section, and at the end of each one's menu.
pub fn all_entries(menu: PopupMenu) -> PopupMenu {
    menu.menu(say!("submodules-add"), entry(SubmoduleRequest::Add))
        .menu(
            say!("submodules-update-all"),
            entry(SubmoduleRequest::Update(None)),
        )
        .menu(
            say!("submodules-sync-all"),
            entry(SubmoduleRequest::Sync(None)),
        )
}

/// Menu entries for one submodule, in groups: update it; open it; remove
/// it; its address synced; and what is done to all of them.
pub fn submodule_entries(menu: PopupMenu, path: &str, there: bool) -> PopupMenu {
    let at = || path.to_owned();
    let menu = menu
        .menu(
            if there {
                say!("submodules-update")
            } else {
                say!("submodules-clone")
            },
            entry(SubmoduleRequest::Update(Some(at()))),
        )
        .separator()
        .menu_with_disabled(
            if there {
                say!("submodules-open-in-tab")
            } else {
                say!("submodules-open-in-tab-not-there")
            },
            entry(SubmoduleRequest::Open(at())),
            !there,
        )
        .separator()
        .menu(
            say!("submodules-remove"),
            entry(SubmoduleRequest::Remove(at())),
        )
        .separator()
        .menu(
            say!("submodules-sync"),
            entry(SubmoduleRequest::Sync(Some(at()))),
        )
        .separator();
    all_entries(menu)
}

/// What a submodule's state is called beside its name; nothing when it is
/// where it should be.
pub fn state_text(state: &str, modified: bool, untracked: bool) -> Option<&'static str> {
    match state {
        "not_initialised" => Some(say!("submodules-state-not-there")),
        "conflicted" => Some(say!("submodules-state-conflicted")),
        "moved" => Some(say!("submodules-state-moved")),
        _ if modified || untracked => Some(say!("submodules-state-changed")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bitbraid_git::{Area, ChangeSet};

    use super::{Next, Paced, state_text, touches};

    fn changes(areas: &[Area], paths: &[&str]) -> ChangeSet {
        ChangeSet {
            areas: areas.to_vec(),
            paths: paths.iter().map(std::path::PathBuf::from).collect(),
        }
    }

    #[test]
    fn a_change_inside_a_submodule_or_under_git_can_move_it() {
        let submodules = ["deps/lib", "vendor/other"];
        let moved = |areas: &[Area], paths: &[&str]| touches(&changes(areas, paths), &submodules);
        // A file in one of them, or the folder itself.
        assert!(moved(
            &[Area::Worktree],
            &["README.md", "deps/lib/src/a.rs"]
        ));
        assert!(moved(&[Area::Worktree], &["vendor/other"]));
        // Files elsewhere, also in a folder whose name starts the same.
        assert!(!moved(
            &[Area::Worktree],
            &["README.md", "deps/library/a.rs", "deps"]
        ));
        // Under `.git`: the submodule's own repository, the index, HEAD.
        for area in [Area::GitDir, Area::Refs, Area::Index] {
            assert!(moved(&[area], &[]), "{area:?}");
        }
        // A batch that may have left paths out counts.
        assert!(moved(&[Area::Worktree], &[]));
        let many: Vec<String> = (0..64).map(|n| format!("src/f{n}.rs")).collect();
        let many: Vec<&str> = many.iter().map(String::as_str).collect();
        assert!(moved(&[Area::Worktree], &many));
        // Nothing changed, or there are no submodules to move.
        assert!(!moved(&[], &["deps/lib/a"]));
        assert!(!touches(&changes(&[Area::GitDir], &[]), &[]));
    }

    #[test]
    fn reads_that_follow_changes_are_kept_apart_and_one_comes_last() {
        let gap = Duration::from_secs(2);
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut paced = Paced::new(gap);
        // The first change is read at once.
        assert_eq!(paced.asked(at(0)), Next::Now);
        paced.started(at(0));
        // A thousand more while it runs start nothing.
        for n in 1..1000 {
            assert_eq!(paced.asked(at(n / 10)), Next::Nothing);
        }
        // Answered after 300 ms: what changed meanwhile is read when the
        // gap is over, and until then nothing else is started.
        assert_eq!(
            paced.ended(at(300)),
            Next::After(Duration::from_millis(1700))
        );
        assert_eq!(paced.asked(at(400)), Next::Nothing);
        assert_eq!(paced.asked(at(1900)), Next::Nothing);
        paced.started(at(2000));
        // Nothing changed while that one ran: it was the last.
        assert_eq!(paced.ended(at(2200)), Next::Nothing);
        // A change soon after waits for the gap; one long after does not.
        assert_eq!(
            paced.asked(at(2500)),
            Next::After(Duration::from_millis(1500))
        );
        paced.started(at(4000));
        assert_eq!(paced.ended(at(4100)), Next::Nothing);
        assert_eq!(paced.asked(at(60_000)), Next::Now);
        paced.started(at(60_000));
        // A read that takes longer than the gap: the next begins at once.
        assert_eq!(paced.asked(at(61_000)), Next::Nothing);
        assert_eq!(paced.ended(at(65_000)), Next::Now);
    }

    #[test]
    fn a_submodule_in_step_says_nothing_unless_it_has_changes() {
        assert_eq!(state_text("in_step", false, false), None);
        assert_eq!(state_text("in_step", true, false), Some("changed"));
        assert_eq!(state_text("in_step", false, true), Some("changed"));
        // Where it stands comes before what is changed in it.
        assert_eq!(state_text("moved", true, true), Some("moved"));
        assert_eq!(
            state_text("not_initialised", false, false),
            Some("not checked out")
        );
        assert_eq!(state_text("conflicted", false, false), Some("in conflict"));
        assert_eq!(state_text("something new", false, false), None);
    }
}
