//! The inbox: the pull requests and issues that wait for the user at
//! every connected account, and the repositories here with uncommitted
//! work, in a tab of the window's own at the left end of the tab bar.
//!
//! What is shown is what `inbox.items` answers, read off the
//! window's thread (`views::pulls::calls`), when the window starts with an
//! account connected, every five minutes, when the tab is shown and its
//! answer is older than a minute, and when its refresh is pressed (past
//! what the providers' answers kept). The repositories it looks at are
//! the open, favourite and recent ones, or a workspace's when one is
//! chosen. Pins and snoozes go through `inbox.pin`, `.unpin`,
//! `.snooze` and `.unsnooze` and show at once; merging through
//! `pr.merge`, after asking how. What touches a repository (checking a
//! pull request out, a worktree, a branch for an issue) is the window's:
//! the view says so with a [`InboxEvent`].
//!
//! Laid out as a heading, the tabs, a row with folding, refresh and the
//! search, a row of choices, then the list.
//!
//! Sources: the documents named in `bitbraid_integrations::inbox`.

pub mod facts;
mod merging;
mod probe;
mod rows;
mod top;

use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};

use bitbraid_integrations::{
    inbox::{Inbox, InboxPull, PullGroup},
    model::{Issue, MergeMethod, RepoLink},
};
use gpui_kit::component::{
    ActiveTheme,
    input::{InputEvent, InputState},
    scroll::{ScrollableElement, ScrollbarAxis},
    v_flex,
};
use gpui_kit::*;
use serde_json::json;

use crate::{notices::Notice, prefs::Prefs, views::pulls::calls, zoom::px};

pub use facts::{Choices, Tab, Whose};
pub use probe::InboxProbe;

/// What the tab and the status bar call it.
pub const NAME: bitbraid_strings::Message = message!("inbox-name");
/// How often it is read while the window is open.
pub const READ_EVERY: Duration = Duration::from_secs(5 * 60);
/// How old an answer may be when the tab is shown before it is read again.
const STALE_ON_SHOW: Duration = Duration::from_secs(60);

/// What the window is asked to do for the inbox.
#[derive(Debug, Clone, PartialEq)]
pub enum InboxEvent {
    /// Open this repository in a tab, or show its tab.
    OpenRepository(PathBuf),
    /// Check out a pull request in its repository (`pr.checkout`).
    CheckOut {
        /// The repository here.
        path: PathBuf,
        /// Which repository at which account, by which remote.
        link: RepoLink,
        number: u64,
        /// The fork it comes from, when it does.
        fork: Option<String>,
    },
    /// Make a worktree for a pull request: the pane that adds one, filled
    /// in with its head once that is fetched (`pr.head`).
    Worktree {
        /// The repository here.
        path: PathBuf,
        /// Which repository at which account.
        link: RepoLink,
        number: u64,
    },
    /// Show a pull request in its repository's tab, to review it.
    Review {
        /// The repository here.
        path: PathBuf,
        /// Which repository at which account.
        link: RepoLink,
        number: u64,
    },
    /// Show an issue in its repository's tab.
    ShowIssue {
        /// The repository here.
        path: PathBuf,
        /// Which repository at which account.
        link: RepoLink,
        number: u64,
    },
    /// Make a branch for an issue in its repository.
    IssueBranch {
        /// The repository here.
        path: PathBuf,
        /// Which repository at which account.
        link: RepoLink,
        issue: Box<Issue>,
    },
    /// Clone a repository that is not here.
    Clone {
        /// From where.
        url: String,
    },
    /// Point at the folder of a repository that is here somewhere.
    Locate,
    /// Show the integrations section of the settings.
    OpenSettings,
}

/// The merge that is being asked about.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Merging {
    /// The pull request's key.
    key: String,
    /// How.
    method: MergeMethod,
    /// Whether its branch goes afterwards.
    delete_branch: bool,
    /// Whether the provider was asked and has not answered.
    busy: bool,
    /// What the provider said against it.
    error: Option<String>,
}

/// The inbox.
pub struct InboxView {
    focus_handle: FocusHandle,
    /// The last answer, and when it came.
    answer: Option<Inbox>,
    read_at: Option<Instant>,
    /// Whether it is being read.
    reading: bool,
    /// Why it could not be read, the last time.
    error: Option<String>,
    /// Which reading is the latest: an older answer is not taken.
    asked: u64,
    /// Whether what is shown was given by a test, which is then never
    /// read.
    given: bool,
    /// The tab on show.
    tab: Tab,
    /// The choices above the list.
    choices: Choices,
    /// The workspace chosen, by id, and the local workspaces there are
    /// (id and name).
    workspace: Option<String>,
    workspaces: Vec<(String, String)>,
    /// The groups that are folded.
    folded: HashSet<PullGroup>,
    search: Entity<InputState>,
    merging: Option<Merging>,
    /// The rows whose pin or snooze is being changed.
    marking: HashSet<String>,
    /// Whether the tab is on show.
    shown: bool,
    /// The accounts known when it was last read, by id.
    accounts_read: Vec<String>,
    /// Seconds since 1970, as of the last drawing.
    now: i64,
    scroll: ScrollHandle,
    _ticker: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<InboxEvent> for InboxView {}

impl InboxView {
    /// Make the inbox; it is read as soon as an account is known.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(say!("inbox-search-placeholder")));
        let typed = cx.subscribe_in(&search, window, |this, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.choices.search = state.read(cx).value().trim().to_owned();
                cx.notify();
            }
        });
        // An account connected or taken away: read again with them.
        let accounts = cx.observe_global::<crate::providers::Accounts>(|this, cx| {
            let now = crate::providers::account_ids(cx);
            if now != this.accounts_read {
                this.accounts_read = now;
                if !this.given && !this.accounts_read.is_empty() {
                    this.read(false, cx);
                }
            }
            cx.notify();
        });
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(READ_EVERY).await;
                let ticked = this.update(cx, |this, cx| {
                    if !this.given && !crate::providers::accounts(cx).is_empty() {
                        this.read(false, cx);
                    }
                });
                if ticked.is_err() {
                    break;
                }
            }
        });
        let mut view = Self {
            focus_handle: cx.focus_handle(),
            answer: None,
            read_at: None,
            reading: false,
            error: None,
            asked: 0,
            given: false,
            tab: Tab::Pulls,
            choices: Choices::default(),
            workspace: None,
            workspaces: Vec::new(),
            folded: HashSet::new(),
            search,
            merging: None,
            marking: HashSet::new(),
            shown: false,
            accounts_read: crate::providers::account_ids(cx),
            now: crate::views::now(),
            scroll: ScrollHandle::new(),
            _ticker: Some(ticker),
            _subscriptions: vec![typed, accounts],
        };
        if !crate::providers::accounts(cx).is_empty() {
            view.read(false, cx);
        }
        view
    }

    /// The tab is shown or put away: shown, it is read again when its
    /// answer is old, and the local workspaces are asked for.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.shown = shown;
        if !shown || self.given {
            return;
        }
        self.load_workspaces(cx);
        let stale = self.read_at.is_none_or(|at| at.elapsed() >= STALE_ON_SHOW);
        if stale && !self.reading && !crate::providers::accounts(cx).is_empty() {
            self.read(false, cx);
        }
        cx.notify();
    }

    /// Put the focus in the search, as the tab is shown.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Show the pull requests' tab with one group open and the others
    /// folded: what the status bar's summary opens.
    pub fn open_group(&mut self, group: PullGroup, cx: &mut Context<Self>) {
        self.tab = Tab::Pulls;
        self.folded = PullGroup::ALL
            .into_iter()
            .filter(|known| *known != group)
            .collect();
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    /// Show what a test made, and never read: the pictures and the tests
    /// of behaviour give the shapes `inbox.items` answers with.
    pub fn give(&mut self, answer: Inbox, cx: &mut Context<Self>) {
        self.given = true;
        self.answer = Some(answer);
        self.reading = false;
        self.error = None;
        self.read_at = Some(Instant::now());
        cx.notify();
    }

    /// Give the local workspaces there are, for a test.
    pub fn give_workspaces(&mut self, workspaces: Vec<(String, String)>, cx: &mut Context<Self>) {
        self.workspaces = workspaces;
        cx.notify();
    }

    /// The last answer, for the status bar.
    pub fn answer(&self) -> Option<&Inbox> {
        self.answer.as_ref()
    }

    /// The repositories looked at: the chosen workspace's are the
    /// command's to find; these are the open, favourite and recent ones.
    fn paths(cx: &App) -> Vec<String> {
        let state = &cx.global::<Prefs>().state;
        let mut out: Vec<String> = Vec::new();
        for path in state
            .open
            .iter()
            .chain(state.favourites.iter())
            .chain(state.recent.iter())
        {
            let text = path.to_string_lossy().into_owned();
            if path.is_absolute() && !out.contains(&text) && out.len() < 200 {
                out.push(text);
            }
        }
        out
    }

    /// Read the inbox: `fresh` asks past what was kept.
    pub fn read(&mut self, fresh: bool, cx: &mut Context<Self>) {
        if self.given {
            return;
        }
        self.asked = self.asked.wrapping_add(1);
        let asked = self.asked;
        self.reading = true;
        let params = match &self.workspace {
            Some(workspace) => {
                json!({ "paths": Self::paths(cx), "fresh": fresh, "workspace": workspace })
            }
            None => json!({ "paths": Self::paths(cx), "fresh": fresh }),
        };
        let answer = calls::ask_in_background::<Inbox>("inbox.items", params);
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                if this.asked != asked {
                    return;
                }
                this.reading = false;
                this.read_at = Some(Instant::now());
                match answer {
                    Ok(answer) => {
                        this.answer = Some(answer);
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Ask which local workspaces there are (`workspace.list`, which
    /// answers `{ "workspaces": [{ "id", "name", … }], … }`; one that does
    /// not read is passed over).
    pub(crate) fn load_workspaces(&mut self, cx: &mut Context<Self>) {
        #[derive(serde::Deserialize)]
        struct Listed {
            id: String,
            name: String,
        }
        #[derive(serde::Deserialize)]
        struct Answer {
            #[serde(default)]
            workspaces: Vec<serde_json::Value>,
        }
        // What a test gave is kept.
        if self.given {
            return;
        }
        let listed = calls::ask_in_background::<Answer>("workspace.list", json!({}));
        cx.spawn(async move |this, cx| {
            let Ok(listed) = listed.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.workspaces = listed
                    .workspaces
                    .into_iter()
                    .filter_map(|one| serde_json::from_value::<Listed>(one).ok())
                    .take(256)
                    .map(|known| {
                        (
                            known.id,
                            crate::views::pulls::facts::shown_line(&known.name, 64),
                        )
                    })
                    .collect();
                if let Some(chosen) = &this.workspace
                    && !this.workspaces.iter().any(|(id, _)| id == chosen)
                {
                    this.workspace = None;
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        self.tab = tab;
        // Whose a row is means other things for issues and pull requests.
        if matches!(
            (tab, self.choices.whose),
            (Tab::Issues, Whose::ReviewRequested) | (Tab::Pulls, Whose::Mentioned)
        ) {
            self.choices.whose = Whose::Mine;
        }
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    fn set_workspace(&mut self, workspace: Option<String>, cx: &mut Context<Self>) {
        if self.workspace != workspace {
            self.workspace = workspace;
            self.read(false, cx);
        }
    }

    fn fold(&mut self, group: PullGroup, cx: &mut Context<Self>) {
        if !self.folded.remove(&group) {
            self.folded.insert(group);
        }
        cx.notify();
    }

    fn fold_all(&mut self, fold: bool, cx: &mut Context<Self>) {
        self.folded = if fold {
            PullGroup::ALL.into_iter().collect()
        } else {
            HashSet::new()
        };
        cx.notify();
    }

    fn pull(&self, key: &str) -> Option<&InboxPull> {
        self.answer
            .as_ref()?
            .pulls
            .iter()
            .find(|pull| pull.key == key)
    }

    /// Pin, unpin, snooze or bring back a row: shown at once, and kept by
    /// the command; what the command refuses is said and undone.
    fn mark(
        &mut self,
        key: String,
        command: &'static str,
        length: Option<&'static str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = crate::views::now();
        let local =
            |pinned: &mut bool, snoozed: &mut Option<bitbraid_integrations::inbox::Snoozed>| {
                match command {
                    "inbox.pin" => *pinned = true,
                    "inbox.unpin" => *pinned = false,
                    "inbox.unsnooze" => *snoozed = None,
                    _ => {
                        let until = match length {
                            Some("hour") => Some(now.saturating_add(3600)),
                            Some("four_hours") => Some(now.saturating_add(4 * 3600)),
                            // Nine tomorrow: the command works it out; until
                            // it answers, a day.
                            Some("tomorrow") => Some(now.saturating_add(86_400)),
                            _ => None,
                        };
                        *snoozed =
                            Some(bitbraid_integrations::inbox::Snoozed { since: now, until });
                    }
                }
            };
        if let Some(answer) = self.answer.as_mut() {
            for pull in answer.pulls.iter_mut().filter(|pull| pull.key == key) {
                local(&mut pull.pinned, &mut pull.snoozed);
            }
            for issue in answer.issues.iter_mut().filter(|issue| issue.key == key) {
                local(&mut issue.pinned, &mut issue.snoozed);
            }
        }
        let params = match length {
            Some(length) => json!({ "key": key, "for": length }),
            None => json!({ "key": key }),
        };
        self.marking.insert(key.clone());
        let asked = calls::ask::<bitbraid_integrations::inbox::store::Marks>(command, params);
        cx.spawn_in(window, async move |this, cx| {
            let marks = asked.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.marking.remove(&key);
                match marks {
                    Ok(marks) => this.take_marks(&marks, cx),
                    Err(error) => {
                        crate::notices::push(
                            window,
                            cx,
                            Notice::error(say!("inbox-keep-failed", error)),
                        );
                        this.read(false, cx);
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The pins and snoozes as the command kept them.
    fn take_marks(
        &mut self,
        marks: &bitbraid_integrations::inbox::store::Marks,
        cx: &mut Context<Self>,
    ) {
        let now = crate::views::now();
        if let Some(answer) = self.answer.as_mut() {
            for pull in &mut answer.pulls {
                pull.pinned = marks.pinned(&pull.key);
                pull.snoozed = marks.snoozed(&pull.key, now);
            }
            for issue in &mut answer.issues {
                issue.pinned = marks.pinned(&issue.key);
                issue.snoozed = marks.snoozed(&issue.key, now);
            }
            let pinned_first = |a: &InboxPull, b: &InboxPull| {
                (b.pinned, &b.pull.updated).cmp(&(a.pinned, &a.pull.updated))
            };
            answer.pulls.sort_by(pinned_first);
        }
        cx.notify();
    }

    /// Ask how a pull request is to be merged.
    fn ask_merge(&mut self, key: String, cx: &mut Context<Self>) {
        self.merging = Some(Merging {
            key,
            method: MergeMethod::Merge,
            delete_branch: false,
            busy: false,
            error: None,
        });
        cx.notify();
    }

    /// Merge the pull request asked about (`pr.merge`), at its provider.
    fn merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(merging) = self.merging.as_mut() else {
            return;
        };
        let Some(pull) = self
            .answer
            .as_ref()
            .and_then(|answer| answer.pulls.iter().find(|pull| pull.key == merging.key))
        else {
            self.merging = None;
            return;
        };
        let Some((owner, name)) = pull.pull.repo.rsplit_once('/') else {
            return;
        };
        merging.busy = true;
        merging.error = None;
        let method = match merging.method {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        };
        let params = json!({
            "account": pull.account, "owner": owner, "name": name,
            "number": pull.pull.number, "method": method,
            "delete_branch": merging.delete_branch,
        });
        let named = facts::short_name(pull);
        let merged = calls::ask::<bitbraid_integrations::model::Merged>("pr.merge", params);
        cx.spawn_in(window, async move |this, cx| {
            let merged = merged.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match merged {
                    Ok(merged) if merged.merged => {
                        this.merging = None;
                        crate::notices::push(
                            window,
                            cx,
                            Notice::success(say!("inbox-merged", named = named.as_str())),
                        );
                        this.read(true, cx);
                    }
                    Ok(merged) => {
                        if let Some(merging) = this.merging.as_mut() {
                            merging.busy = false;
                            merging.error = Some(
                                merged
                                    .message
                                    .unwrap_or_else(|| say!("inbox-not-merged").to_owned()),
                            );
                        }
                    }
                    Err(error) => {
                        if let Some(merging) = this.merging.as_mut() {
                            merging.busy = false;
                            merging.error = Some(error);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The link of a row to its repository: which account, which remote
    /// here.
    fn link(
        account: &str,
        kind: bitbraid_integrations::model::ProviderKind,
        repo: &str,
        remote: Option<&str>,
    ) -> Option<RepoLink> {
        let (owner, name) = repo.rsplit_once('/')?;
        Some(RepoLink {
            remote: remote.unwrap_or("origin").to_owned(),
            account: account.to_owned(),
            kind,
            owner: owner.to_owned(),
            name: name.to_owned(),
        })
    }

    /// Open a page at the provider, through the system.
    fn open_page(&self, url: &str, window: &mut Window, cx: &mut Context<Self>) {
        if !crate::providers::open_link(url, cx) {
            crate::notices::push(window, cx, Notice::info(say!("inbox-page-cannot-open")));
        }
    }
}

impl Focusable for InboxView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for InboxView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.now = crate::views::now();
        let body = self.body(window, cx);
        let merging = self.merging.as_ref().map(|_| self.merge_dialog(cx));
        v_flex()
            .id("inbox-page")
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .child(self.top(cx))
            .child(
                div()
                    .relative()
                    .mt(px(top::LIST_TOP))
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        v_flex()
                            .id("inbox-list")
                            .size_full()
                            // Down, and sideways in a window narrower than
                            // the tables' columns (`rows::TABLE_LEAST`).
                            .overflow_scroll()
                            .track_scroll(&self.scroll)
                            .pl(px(rows::LIST_LEFT))
                            .pr(px(rows::LIST_RIGHT))
                            .pb(px(24.))
                            .child(div().min_w(px(rows::TABLE_LEAST)).child(body))
                            .test_support(),
                    )
                    .scrollbar(&self.scroll, ScrollbarAxis::Both),
            )
            .children(merging)
            .test_support()
    }
}
