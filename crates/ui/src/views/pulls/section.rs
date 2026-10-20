//! What the PULL REQUESTS section of the left panel holds: a search box
//! and a button that adds a filter at its top; then the groups (mine,
//! assigned to me, waiting for my review, all, and the filters the user
//! made), each closed and opened from its heading and saying how many it
//! has; in each, the pull requests of every remote linked to an account,
//! under the remote's name with the provider's mark; a row has a mark of
//! where it stands, its number, its title cut to fit, a menu, and a card
//! over it while the pointer rests on it. A remote at a provider no
//! account is connected for has a line that invites to connect one.
//!
//! The panel draws the section's heading (with its count, a refresh and a
//! plus) and gives the section its height; this view is what is under the
//! heading. It reads the lists once when the remotes are linked (the
//! heading's count and the graph's marks need them), when the section
//! comes into view, every five minutes while it is in view, and when the
//! heading's refresh is pressed (then past what the provider's answers
//! that were kept say). A list kept from before because the provider could
//! not be reached has a quiet mark that says when it was read.
//!
//! Sources: none (original design).

use std::{
    collections::{BTreeSet, HashMap, HashSet},
    rc::Rc,
    time::{Duration, Instant},
};

use bitbraid_git::Snapshot;
use bitbraid_integrations::model::{
    Account, CommitStatus, Listing, ProviderKind, PullRequest, PullState, RepoLink,
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, VirtualListScrollHandle,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::{
    card::{card, state_icon, with_card},
    facts::{
        Group, PullMark, SavedFilter, branch_marks, matches_search, may_open, number_text,
        provider_name, remembered_filters, shown_line, state_mark, unconnected_hosts,
    },
    filter,
};
use crate::{
    prefs::Prefs,
    session::{Reply, SessionHandle, run_standalone},
    zoom::px,
};

/// The row of the search box under the heading, the box, and how far in
/// from the panel's edge it starts.
pub const SEARCH_ROW_HEIGHT: f32 = 35.0;
pub const SEARCH_HEIGHT: f32 = 30.0;
pub const SEARCH_LEFT: f32 = 25.0;
/// How often the lists are read again while the section is in view.
pub const READ_EVERY: Duration = Duration::from_secs(5 * 60);
/// Height of a row, as the references panel's rows are.
const ROW_HEIGHT: f32 = 24.0;
/// How far each level is indented.
const INDENT: f32 = 15.0;
/// The room between the parts of a row.
const ROW_GAP: f32 = 5.5;
/// What a row's controls that show under the pointer know it by.
const ROW_GROUP: &str = "pulls-row";
/// How much further in than a third level a pull request's mark stands.
const PULL_MARK_IN: f32 = 1.0;
/// The most closed groups and remotes remembered.
const MOST_CLOSED: usize = 256;

/// What the section asks of the window.
#[derive(Debug, Clone)]
pub enum PullsEvent {
    /// Show this pull request in the middle.
    Open {
        /// Where it is.
        link: RepoLink,
        number: u64,
    },
    /// Check out the branch this pull request brings (a double click on
    /// its row).
    Checkout {
        /// Where it is.
        link: RepoLink,
        number: u64,
        /// The fork it comes from, `owner/name`, when it is one.
        fork: Option<String>,
    },
    /// Select this pull request's head commit in the graph.
    Reveal {
        /// The commit, as the provider wrote it.
        sha: String,
    },
    /// Show where accounts are connected.
    ConnectAccount,
    /// Whether the section is shown, or its count, changed.
    Counted,
    /// The remote whose commits' checks the graph asks for, if any: the
    /// first linked one, `origin` first.
    Linked(Option<RepoLink>),
    /// The branches with an open pull request, by name.
    Marked(HashMap<String, PullMark>),
}

/// A list of one group of one remote, as far as it was read.
#[derive(Debug, Clone, Default)]
struct Listed {
    listing: Option<Listing<PullRequest>>,
    error: Option<String>,
    loading: bool,
}

/// What the off-screen tests and the shapes of a test give the section in
/// place of what the providers would answer.
#[derive(Debug, Clone, Default)]
pub struct Given {
    pub links: Vec<RepoLink>,
    /// The lists, each of a group of a link (by its place in `links`).
    pub lists: Vec<(Group, usize, Listing<PullRequest>)>,
    /// The checks of the pull requests' head commits.
    pub checks: Vec<CommitStatus>,
    /// The hosts without an account.
    pub invite: Vec<String>,
}

/// One row of the section.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// A group's heading.
    Group {
        group: Group,
        title: String,
        count: usize,
        open: bool,
    },
    /// A remote's heading in a group: its provider's mark and its name.
    Remote { group: Group, link: usize },
    /// A pull request: in which group, of which link, and its place in
    /// that list.
    Pull {
        group: Group,
        link: usize,
        at: usize,
    },
    /// A line about a list: it is being read, it is empty, or it could not
    /// be read.
    Note {
        text: String,
        problem: bool,
        indent: usize,
    },
    /// An invitation to connect an account for a host.
    Invite { host: String },
}

/// What the section has on show, for the off-screen tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullsProbe {
    /// Whether the panel shows the section at all.
    pub listed: bool,
    /// Whether its lists are in view.
    pub shown: bool,
    /// Each group: its key, how many it shows and whether it is open.
    pub groups: Vec<(String, usize, bool)>,
    /// The pull requests' rows, top to bottom: number and title.
    pub rows: Vec<String>,
    /// What the search box holds.
    pub search: String,
    /// Whether a list was kept from before, the provider out of reach.
    pub offline: bool,
    /// The hosts it invites to connect an account for.
    pub invite: Vec<String>,
    /// The filters the user made, by name.
    pub filters: Vec<String>,
    /// Whether a list is being read.
    pub reading: bool,
    /// How many head commits of the pull requests listed have their
    /// checks known.
    pub checks: usize,
}

/// The section's view.
pub struct PullsView {
    session: SessionHandle,
    /// How many times a row was clicked: the cards are known by it, so
    /// that a click puts the one on show away until the pointer comes
    /// back.
    cards: u64,
    /// A row was clicked and the pointer is still on it: no card shows
    /// until it leaves.
    quiet: bool,
    /// The branches with an open pull request, as the graph was told.
    marks: HashMap<String, PullMark>,
    /// The open repository's remotes: name and address.
    remotes: Vec<(String, String)>,
    /// The remotes were read once.
    remotes_read: bool,
    /// The remotes linked to an account, once asked.
    links: Vec<RepoLink>,
    /// Which remotes are linked was answered once (or given).
    links_answered: bool,
    /// The accounts, once asked.
    accounts: Vec<Account>,
    /// The hosts of remotes with no account.
    invite: Vec<String>,
    /// The filters the user made here.
    custom: Vec<SavedFilter>,
    /// The lists, by group and link.
    lists: HashMap<(Group, usize), Listed>,
    /// The checks of the head commits of the pull requests listed.
    head_checks: HashMap<String, CommitStatus>,
    /// The head commits asked about since the lists were last read.
    head_asked: HashSet<String>,
    /// The groups and remotes that are closed.
    closed: BTreeSet<String>,
    search: Entity<InputState>,
    search_text: String,
    /// The lists are in view.
    shown: bool,
    /// When the lists were last asked for.
    read_at: Option<Instant>,
    /// Which reading the answers belong to: an older one's are dropped.
    generation: u64,
    /// The lists were given (tests): nothing is read.
    given: bool,
    now: i64,
    scroll: VirtualListScrollHandle,
    /// Reads the lists again every [`READ_EVERY`] while they are in view.
    ticker: Option<Task<()>>,
    /// What redraws the dialog that adds a filter as it is typed in.
    dialog_subscriptions: Vec<Subscription>,
    /// The accounts the window knew when the links were last asked.
    account_ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PullsEvent> for PullsView {}

/// What a group and a remote in it are known by among the closed ones.
fn remote_key(group: Group, remote: &str) -> String {
    format!("{}/{remote}", group.key())
}

impl PullsView {
    /// Create the section for an open repository.
    pub fn new(session: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.global::<Prefs>().repo_state(&session.path);
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(say!("pulls-search")));
        let subscription =
            cx.subscribe_in(&search, window, |this, state, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search_text = state.read(cx).value().to_string();
                    cx.emit(PullsEvent::Counted);
                    cx.notify();
                }
            });
        Self {
            session,
            cards: 0,
            quiet: false,
            marks: HashMap::new(),
            remotes: Vec::new(),
            remotes_read: false,
            links: Vec::new(),
            links_answered: false,
            accounts: Vec::new(),
            invite: Vec::new(),
            custom: remembered_filters(&state.pull_filters),
            lists: HashMap::new(),
            head_checks: HashMap::new(),
            head_asked: HashSet::new(),
            closed: state
                .closed_pull_groups
                .iter()
                .filter(|key| !key.is_empty() && key.len() <= 1024)
                .take(MOST_CLOSED)
                .cloned()
                .collect(),
            search,
            search_text: String::new(),
            shown: false,
            read_at: None,
            generation: 0,
            given: false,
            now: crate::views::now(),
            scroll: VirtualListScrollHandle::new(),
            ticker: None,
            dialog_subscriptions: Vec::new(),
            account_ids: crate::providers::account_ids(cx),
            _subscriptions: vec![
                subscription,
                cx.observe_global::<crate::providers::Accounts>(Self::accounts_changed),
            ],
        }
    }

    /// The groups, in their order: the four every repository has, then the
    /// user's.
    fn groups(&self) -> Vec<Group> {
        Group::BUILT_IN
            .into_iter()
            .chain((0..self.custom.len()).map(Group::Custom))
            .collect()
    }

    /// The repository's remotes changed, or were read for the first time:
    /// which are linked to an account is asked again when they changed.
    pub fn set_remotes(&mut self, snapshot: &Snapshot, cx: &mut Context<Self>) {
        let remotes: Vec<(String, String)> = snapshot
            .remotes
            .iter()
            .map(|remote| {
                (
                    remote.name.clone(),
                    remote.fetch_url.clone().unwrap_or_default(),
                )
            })
            .collect();
        self.remotes_read = true;
        if remotes == self.remotes {
            return;
        }
        self.remotes = remotes;
        self.load_links(cx);
    }

    /// Whether it is known which remotes are linked to an account: it was
    /// answered, or the repository has no remote to link.
    pub fn links_known(&self) -> bool {
        self.links_answered || (self.remotes_read && self.remotes.is_empty())
    }

    /// An account was connected or disconnected (the accounts the window
    /// knows changed): which remotes are linked is asked again, so that
    /// the lists come without opening the repository again.
    fn accounts_changed(&mut self, cx: &mut Context<Self>) {
        let ids = crate::providers::account_ids(cx);
        if ids == self.account_ids {
            return;
        }
        self.account_ids = ids;
        if !self.remotes.is_empty() {
            self.load_links(cx);
        }
    }

    /// Ask which remotes are linked to an account, and which accounts
    /// there are.
    fn load_links(&mut self, cx: &mut Context<Self>) {
        if self.given {
            return;
        }
        let request = crate::session::Request::Command {
            id: "provider.links".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            let links: Vec<RepoLink> = match reply {
                Reply::Command(value) => serde_json::from_value(value).unwrap_or_default(),
                _ => Vec::new(),
            };
            this.set_links(links, cx);
        });
        let running = run_standalone("account.list", json!({}));
        SessionHandle::deliver(
            running.reply,
            "account.list".to_owned(),
            cx,
            |this, reply, cx| {
                this.accounts = match reply {
                    Reply::Command(value) => serde_json::from_value(value).unwrap_or_default(),
                    _ => Vec::new(),
                };
                this.invite = unconnected_hosts(&this.remotes, &this.links, &this.accounts);
                cx.emit(PullsEvent::Counted);
                cx.notify();
            },
        );
    }

    /// The linked remotes are known: when they changed, the lists held are
    /// dropped and read again.
    fn set_links(&mut self, links: Vec<RepoLink>, cx: &mut Context<Self>) {
        let changed = links != self.links;
        self.links = links;
        self.links_answered = true;
        self.invite = unconnected_hosts(&self.remotes, &self.links, &self.accounts);
        if changed {
            self.lists.clear();
            self.head_checks.clear();
            self.read_at = None;
            cx.emit(PullsEvent::Linked(self.first_link()));
            self.marks_changed(cx);
            // The heading's count and the graph's marks of branches with a
            // pull request need the lists: they are read once as soon as
            // the remotes are linked, and then while the section is in
            // view.
            if !self.links.is_empty() {
                self.read(false, cx);
            }
        }
        cx.emit(PullsEvent::Counted);
        cx.notify();
    }

    /// Take these as the linked remotes, as `provider.links` would have
    /// answered, and read their lists from then on: for the off-screen
    /// tests, whose stand-ins answer the provider commands.
    pub fn link_to(&mut self, links: Vec<RepoLink>, cx: &mut Context<Self>) {
        self.set_links(links, cx);
    }

    /// The remote the graph's checks are asked of: `origin` if it is
    /// linked, else the first that is.
    fn first_link(&self) -> Option<RepoLink> {
        self.links
            .iter()
            .find(|link| link.remote == "origin")
            .or_else(|| self.links.first())
            .cloned()
    }

    /// Whether the lists are in view: they are read when they come into
    /// view and have not been for a while, and again every [`READ_EVERY`]
    /// while they stay.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if shown == self.shown {
            return;
        }
        self.shown = shown;
        if !shown {
            self.ticker = None;
            return;
        }
        let stale = self.read_at.is_none_or(|at| at.elapsed() >= READ_EVERY);
        if stale {
            self.read(false, cx);
        }
        if !self.given {
            self.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(READ_EVERY).await;
                    let ticked = this.update(cx, |this, cx| {
                        if this.shown {
                            this.read(false, cx);
                        }
                    });
                    if ticked.is_err() {
                        break;
                    }
                }
            }));
        }
    }

    /// Read again, past what was kept: the heading's refresh. Which remotes
    /// are linked is asked again too, in case an account was connected.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load_links(cx);
        self.read(true, cx);
    }

    /// Ask every list again: each group of each linked remote.
    fn read(&mut self, fresh: bool, cx: &mut Context<Self>) {
        if self.given {
            return;
        }
        self.read_at = Some(Instant::now());
        self.now = crate::views::now();
        self.generation = self.generation.wrapping_add(1);
        self.head_asked.clear();
        let generation = self.generation;
        for group in self.groups() {
            for (at, link) in self.links.iter().enumerate() {
                let mut params = json!({
                    "account": link.account,
                    "owner": link.owner,
                    "name": link.name,
                    "filter": group.filter(),
                    "fresh": fresh,
                });
                if let Group::Custom(custom) = group
                    && let Some(filter) = self.custom.get(custom)
                    && let Some(object) = params.as_object_mut()
                {
                    object.insert("query".to_owned(), json!(filter.query));
                }
                self.lists.entry((group, at)).or_default().loading = true;
                let reply = super::calls::run("pr.list", params);
                SessionHandle::deliver(reply, "pr.list".to_owned(), cx, move |this, reply, cx| {
                    if this.generation != generation {
                        return;
                    }
                    let listed = this.lists.entry((group, at)).or_default();
                    listed.loading = false;
                    // The same pull requests as before (most readings,
                    // and every kept answer) change no mark and no count:
                    // nothing else is told, nor made over.
                    let error_before = listed.error.take();
                    let same = match reply {
                        Reply::Command(value) => {
                            match serde_json::from_value::<Listing<PullRequest>>(value) {
                                Ok(listing) => {
                                    let same = error_before.is_none()
                                        && listed
                                            .listing
                                            .as_ref()
                                            .is_some_and(|held| held.items == listing.items);
                                    listed.listing = Some(listing);
                                    same
                                }
                                Err(_) => {
                                    listed.error = Some(say!("pulls-reply-unreadable").to_owned());
                                    listed.error == error_before
                                }
                            }
                        }
                        Reply::Error(err) => {
                            listed.error = Some(err);
                            listed.error == error_before
                        }
                        _ => {
                            listed.error = error_before;
                            true
                        }
                    };
                    this.ask_head_checks(at, cx);
                    if !same {
                        this.marks_changed(cx);
                        cx.emit(PullsEvent::Counted);
                    }
                    cx.notify();
                });
            }
        }
        cx.notify();
    }

    /// Ask for the checks of the head commits of the open pull requests of
    /// a remote that are not known yet: a batch at a time.
    fn ask_head_checks(&mut self, at: usize, cx: &mut Context<Self>) {
        let Some(link) = self.links.get(at).cloned() else {
            return;
        };
        let mut shas: Vec<String> = Vec::new();
        for ((_, link_at), listed) in &self.lists {
            if *link_at != at {
                continue;
            }
            for pull in listed.listing.iter().flat_map(|listing| &listing.items) {
                if pull.state == PullState::Open
                    && !pull.head_sha.is_empty()
                    && !self.head_asked.contains(&pull.head_sha)
                    && !shas.contains(&pull.head_sha)
                    && shas.len() < super::checks::BATCH
                {
                    shas.push(pull.head_sha.clone());
                }
            }
        }
        if shas.is_empty() {
            return;
        }
        self.head_asked.extend(shas.iter().cloned());
        let generation = self.generation;
        let reply = super::calls::run(
            "ci.status",
            json!({
                "account": link.account,
                "owner": link.owner,
                "name": link.name,
                "shas": shas,
            }),
        );
        SessionHandle::deliver(reply, "ci.status".to_owned(), cx, move |this, reply, cx| {
            if this.generation != generation {
                return;
            }
            if let Reply::Command(value) = reply {
                let statuses: Vec<CommitStatus> = serde_json::from_value(value).unwrap_or_default();
                for status in statuses {
                    this.head_checks.insert(status.sha.clone(), status);
                }
                cx.notify();
            }
        });
    }

    /// Show these in place of what the providers would answer, and read
    /// nothing from then on: for the off-screen tests.
    pub fn give(&mut self, given: Given, cx: &mut Context<Self>) {
        self.given = true;
        self.ticker = None;
        self.generation = self.generation.wrapping_add(1);
        self.links = given.links;
        self.links_answered = true;
        self.invite = given.invite;
        self.lists = given
            .lists
            .into_iter()
            .map(|(group, at, listing)| {
                (
                    (group, at),
                    Listed {
                        listing: Some(listing),
                        error: None,
                        loading: false,
                    },
                )
            })
            .collect();
        self.head_checks = given
            .checks
            .into_iter()
            .map(|status| (status.sha.clone(), status))
            .collect();
        self.now = crate::views::now();
        // Given lists ask nothing of a provider: nor does the graph for them.
        cx.emit(PullsEvent::Linked(None));
        self.marks_changed(cx);
        cx.emit(PullsEvent::Counted);
        cx.notify();
    }

    /// Tell the graph which branches have an open pull request.
    fn marks_changed(&mut self, cx: &mut Context<Self>) {
        let lists = self.lists.iter().filter_map(|((_, at), listed)| {
            let link = self.links.get(*at)?;
            let listing = listed.listing.as_ref()?;
            Some((link, listing.items.as_slice()))
        });
        let marks = branch_marks(lists);
        // The graph holds what it was told last (nothing, at first).
        if marks == self.marks {
            return;
        }
        self.marks = marks.clone();
        cx.emit(PullsEvent::Marked(marks));
    }

    /// The open pull request of a branch by its name here (a remote's as
    /// `remote/branch`), if the lists have one: what its menu offers.
    pub fn mark_of(&self, branch: &str) -> Option<PullMark> {
        let named = self
            .remotes
            .iter()
            .find_map(|(remote, _)| branch.strip_prefix(remote.as_str())?.strip_prefix('/'))
            .unwrap_or(branch);
        self.marks.get(named).cloned()
    }

    /// The remotes linked to an account, `origin` first: where pull
    /// requests can be made.
    pub fn links(&self) -> Vec<RepoLink> {
        let mut links = self.links.clone();
        links.sort_by_key(|link| link.remote != "origin");
        links
    }

    /// How many open pull requests the heading counts, or none while the
    /// section is not to be shown: no remote is linked, and none invites
    /// to connect an account.
    pub fn heading_count(&self) -> Option<usize> {
        if self.links.is_empty() && self.invite.is_empty() {
            return None;
        }
        let mut seen: HashSet<(usize, u64)> = HashSet::new();
        for ((_, at), listed) in &self.lists {
            for pull in listed.listing.iter().flat_map(|listing| &listing.items) {
                if matches_search(pull, &self.search_text) {
                    seen.insert((*at, pull.number));
                }
            }
        }
        Some(seen.len())
    }

    /// The pull requests of a list that the search box keeps, with their
    /// places in it.
    fn found(&self, group: Group, at: usize) -> Vec<(usize, &PullRequest)> {
        self.lists
            .get(&(group, at))
            .and_then(|listed| listed.listing.as_ref())
            .map(|listing| {
                listing
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, pull)| matches_search(pull, &self.search_text))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The rows of the section, top to bottom.
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        // Without a linked remote there is nothing to group: only the
        // invitations.
        let groups = if self.links.is_empty() {
            Vec::new()
        } else {
            self.groups()
        };
        for group in groups {
            let count = (0..self.links.len())
                .map(|at| self.found(group, at).len())
                .sum();
            let open = !self.closed.contains(&group.key());
            rows.push(Row::Group {
                group,
                title: group.title(&self.custom),
                count,
                open,
            });
            if !open {
                continue;
            }
            // A remote's heading stands only over something: an empty
            // list shows nothing under the group, whose count says it.
            for at in 0..self.links.len() {
                let found = self.found(group, at);
                let listed = self.lists.get(&(group, at));
                let note = |text: &str, problem: bool| Row::Note {
                    text: text.to_owned(),
                    problem,
                    indent: 2,
                };
                let under: Vec<Row> = match listed {
                    Some(Listed {
                        error: Some(error), ..
                    }) => vec![note(&shown_line(error, 300), true)],
                    Some(Listed {
                        listing: Some(_), ..
                    }) => found
                        .into_iter()
                        .map(|(place, _)| Row::Pull {
                            group,
                            link: at,
                            at: place,
                        })
                        .collect(),
                    _ => vec![note(say!("pulls-reading"), false)],
                };
                if under.is_empty() {
                    continue;
                }
                rows.push(Row::Remote { group, link: at });
                rows.extend(under);
            }
        }
        rows.extend(
            self.invite
                .iter()
                .map(|host| Row::Invite { host: host.clone() }),
        );
        rows
    }

    /// The pull request a row stands for.
    fn pull_of(&self, group: Group, link: usize, at: usize) -> Option<&PullRequest> {
        self.lists
            .get(&(group, link))?
            .listing
            .as_ref()?
            .items
            .get(at)
    }

    /// The freshness of the lists held: whether one was kept from before,
    /// and the oldest time one of those was read.
    fn offline(&self) -> Option<Option<String>> {
        let kept: Vec<&Listing<PullRequest>> = self
            .lists
            .values()
            .filter_map(|listed| listed.listing.as_ref())
            .filter(|listing| listing.freshness.offline)
            .collect();
        if kept.is_empty() {
            return None;
        }
        Some(
            kept.iter()
                .filter_map(|listing| listing.freshness.fetched.clone())
                .min(),
        )
    }

    /// Open or close a group, or a remote in a group, and remember it.
    fn toggle(&mut self, key: String, cx: &mut Context<Self>) {
        if !self.closed.remove(&key) && self.closed.len() < MOST_CLOSED {
            self.closed.insert(key);
        }
        let closed: Vec<String> = self.closed.iter().cloned().collect();
        let path = self.session.path.clone();
        cx.global_mut::<Prefs>()
            .update_repo_state(&path, |state| state.closed_pull_groups = closed);
        cx.notify();
    }

    /// Open the dialog that adds a filter.
    fn add_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let kinds: Vec<ProviderKind> = self.links.iter().map(|link| link.kind).collect();
        let view = cx.entity().downgrade();
        self.dialog_subscriptions = filter::open(view, kinds, self.custom.clone(), window, cx);
    }

    /// Keep a filter the dialog made, and read its lists.
    pub fn save_filter(&mut self, filter: SavedFilter, cx: &mut Context<Self>) {
        if super::facts::filter_problem(&filter.name, &filter.query, &self.custom).is_some() {
            return;
        }
        self.custom.push(SavedFilter {
            name: filter.name.trim().to_owned(),
            query: filter.query.trim().to_owned(),
        });
        self.remember_filters(cx);
        if self.shown {
            self.read(false, cx);
        }
        cx.emit(PullsEvent::Counted);
        cx.notify();
    }

    /// Forget a filter the user made.
    fn remove_filter(&mut self, at: usize, cx: &mut Context<Self>) {
        if at >= self.custom.len() {
            return;
        }
        self.custom.remove(at);
        // The lists of the filters after it move up a place.
        let lists = std::mem::take(&mut self.lists);
        self.lists = lists
            .into_iter()
            .filter_map(|((group, link), listed)| match group {
                Group::Custom(n) if n == at => None,
                Group::Custom(n) if n > at => Some(((Group::Custom(n - 1), link), listed)),
                group => Some(((group, link), listed)),
            })
            .collect();
        self.remember_filters(cx);
        cx.emit(PullsEvent::Counted);
        cx.notify();
    }

    fn remember_filters(&self, cx: &mut Context<Self>) {
        let filters = self.custom.clone();
        let path = self.session.path.clone();
        cx.global_mut::<Prefs>()
            .update_repo_state(&path, |state| state.pull_filters = filters);
    }

    /// What the section has on show.
    pub fn probe(&self) -> PullsProbe {
        let rows = self.rows();
        let kind_of = |at: usize| {
            self.links
                .get(at)
                .map_or(ProviderKind::Github, |link| link.kind)
        };
        PullsProbe {
            listed: self.heading_count().is_some(),
            shown: self.shown,
            groups: rows
                .iter()
                .filter_map(|row| match row {
                    Row::Group {
                        group, count, open, ..
                    } => Some((group.key(), *count, *open)),
                    _ => None,
                })
                .collect(),
            rows: rows
                .iter()
                .filter_map(|row| match row {
                    Row::Pull { group, link, at } => self.pull_of(*group, *link, *at).map(|pull| {
                        format!(
                            "{} {}",
                            number_text(kind_of(*link), pull.number),
                            pull.title
                        )
                    }),
                    _ => None,
                })
                .collect(),
            search: self.search_text.clone(),
            offline: self.offline().is_some(),
            invite: self.invite.clone(),
            filters: self
                .custom
                .iter()
                .map(|filter| filter.name.clone())
                .collect(),
            reading: self.lists.values().any(|listed| listed.loading),
            checks: self.head_checks.len(),
        }
    }

    /// Open a pull request's view by its number, from the first list that
    /// has it: for the off-screen tests and the keyboard alike.
    pub fn open_number(&mut self, number: u64, cx: &mut Context<Self>) -> bool {
        let found = self.lists.iter().find_map(|((_, at), listed)| {
            listed
                .listing
                .as_ref()?
                .items
                .iter()
                .find(|pull| pull.number == number)
                .and(self.links.get(*at))
        });
        match found.cloned() {
            Some(link) => {
                cx.emit(PullsEvent::Open { link, number });
                true
            }
            None => false,
        }
    }

    /// The menu of a pull request's row.
    fn row_menu(
        menu: PopupMenu,
        view: WeakEntity<Self>,
        link: RepoLink,
        number: u64,
        url: String,
        (head_ref, head_sha): (String, String),
    ) -> PopupMenu {
        let open = {
            let view = view.clone();
            let link = link.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let link = link.clone();
                let _ = view.update(cx, |_, cx| cx.emit(PullsEvent::Open { link, number }));
            }
        };
        let checkout = {
            let view = view.clone();
            let link = link.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let link = link.clone();
                let _ = view.update(cx, |this, cx| {
                    let fork = this.fork_of(&link, number);
                    cx.emit(PullsEvent::Checkout { link, number, fork })
                });
            }
        };
        let reveal = {
            let view = view.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let sha = head_sha.clone();
                let _ = view.update(cx, |_, cx| cx.emit(PullsEvent::Reveal { sha }));
            }
        };
        let browse = {
            let url = url.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                if may_open(&url) {
                    cx.open_url(&url);
                }
            }
        };
        let copy = move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
            cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
        };
        // The menu's groups: the provider's page and the link; then what
        // is done here.
        let named = number_text(link.kind, number);
        menu.item(
            PopupMenuItem::new(say!(
                "pulls-menu-open-at",
                named = &named,
                provider = provider_name(link.kind)
            ))
            .on_click(browse),
        )
        .item(PopupMenuItem::new(say!("pulls-menu-copy-link", named = &named)).on_click(copy))
        .separator()
        .item(PopupMenuItem::new(say!("pulls-menu-show-here", named = &named)).on_click(open))
        .item(PopupMenuItem::new(say!("pulls-menu-check-out")).on_click(checkout))
        .item(
            PopupMenuItem::new(say!(
                "pulls-menu-select-head",
                branch = shown_line(&head_ref, 60)
            ))
            .on_click(reveal),
        )
    }

    /// The fork a listed pull request comes from, `owner/name`, if it is
    /// one.
    fn fork_of(&self, link: &RepoLink, number: u64) -> Option<String> {
        let at = self.links.iter().position(|known| known == link)?;
        self.lists
            .iter()
            .filter(|((_, of), _)| *of == at)
            .filter_map(|(_, listed)| listed.listing.as_ref())
            .flat_map(|listing| &listing.items)
            .find(|pull| pull.number == number)
            .and_then(|pull| pull.head_repo.clone())
    }

    fn render_row(&self, number: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let base = |indent: usize| {
            h_flex()
                .group(ROW_GROUP)
                .w_full()
                .h(px(ROW_HEIGHT))
                .flex_none()
                .pl(px(6.))
                // Clear of the list's scroll bar, as the references panel's
                // rows.
                .pr(px(25.))
                .gap(px(ROW_GAP))
                .items_center()
                .text_size(px(13.))
                // The column at the left end, as the references panel's
                // rows have it.
                .child(div().w(px(14.)).flex_none())
                .when(indent > 0, |row| {
                    row.child(
                        div()
                            .flex_none()
                            .w(px((INDENT * indent as f32 - ROW_GAP).max(0.))),
                    )
                })
        };
        let count = |count: usize| {
            div()
                .flex_none()
                .ml_auto()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(format!("{count}"))
        };
        match row {
            Row::Group {
                group,
                title,
                count: shown,
                open,
            } => {
                let key = group.key();
                let custom = match group {
                    Group::Custom(at) => Some(*at),
                    _ => None,
                };
                let view = cx.entity().downgrade();
                let row = base(0)
                    .id(ElementId::Name(format!("pulls-group:{key}").into()))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.accent))
                    // In the rows' colour.
                    .text_color(theme.foreground)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(key.clone(), cx)))
                    .child(
                        Icon::new(if *open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size_3p5()
                        .flex_none(),
                    )
                    .child(div().min_w_0().truncate().child(shown_line(title, 80)))
                    .child(count(*shown));
                match custom {
                    Some(at) => div()
                        .id(("pulls-group-menu", number))
                        .child(row.test_support())
                        .context_menu(move |menu, _, _| {
                            let view = view.clone();
                            menu.item(PopupMenuItem::new(say!("pulls-remove-filter")).on_click(
                                move |_, _, cx| {
                                    let _ = view.update(cx, |this, cx| this.remove_filter(at, cx));
                                },
                            ))
                        })
                        .into_any_element(),
                    None => row.test_support().into_any_element(),
                }
            }
            Row::Remote { group, link } => {
                let Some(link) = self.links.get(*link) else {
                    return div().into_any_element();
                };
                let key = remote_key(*group, &link.remote);
                base(1)
                    .id(ElementId::Name(format!("pulls-remote:{key}").into()))
                    .child(provider_mark(link.kind, cx))
                    // The remote's name; the repository is in the
                    // tooltip.
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(shown_line(&link.remote, 80)),
                    )
                    .tooltip({
                        let said = say!(
                            "pulls-remote-tooltip",
                            remote = shown_line(&link.remote, 80),
                            provider = provider_name(link.kind),
                            owner = shown_line(&link.owner, 120),
                            name = shown_line(&link.name, 120)
                        );
                        move |window, cx| Tooltip::new(said.clone()).build(window, cx)
                    })
                    .test_support()
                    .into_any_element()
            }
            Row::Pull { group, link, at } => {
                let (Some(pull), Some(linked)) =
                    (self.pull_of(*group, *link, *at), self.links.get(*link))
                else {
                    return div().into_any_element();
                };
                let (icon, colour) = state_icon(state_mark(pull), cx);
                let kind = linked.kind;
                let shown_pull = pull.clone();
                let checks = self.head_checks.get(&pull.head_sha).cloned();
                let now = self.now;
                let (link_for_click, number) = (linked.clone(), pull.number);
                let cards = self.cards;
                let fork = pull.head_repo.clone();
                let view = cx.entity().downgrade();
                let (link_for_menu, url) = (linked.clone(), pull.web_url.clone());
                let menu_view = view.clone();
                let (menu_link, menu_url) = (linked.clone(), pull.web_url.clone());
                let head = (pull.head_ref.clone(), pull.head_sha.clone());
                let menu_head = head.clone();
                let row = base(2)
                    .id(ElementId::Name(
                        format!("pull:{}:{}:{}", group.key(), link, pull.number).into(),
                    ))
                    .relative()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.accent))
                    .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                        // A click puts the card away.
                        this.cards = this.cards.wrapping_add(1);
                        this.quiet = true;
                        cx.notify();
                        // A double click checks the branch out; one click
                        // shows it here.
                        if event.click_count() >= 2 {
                            cx.emit(PullsEvent::Checkout {
                                link: link_for_click.clone(),
                                number,
                                fork: fork.clone(),
                            })
                        } else {
                            cx.emit(PullsEvent::Open {
                                link: link_for_click.clone(),
                                number,
                            })
                        }
                    }))
                    // The mark stands a little further in than a third
                    // level would put it, and is smaller.
                    .child(div().w(px(PULL_MARK_IN)).flex_none())
                    .child(icon.size_3().flex_none().text_color(colour))
                    .child(div().min_w_0().flex_1().truncate().child(format!(
                        "{} {}",
                        number_text(kind, pull.number),
                        shown_line(&pull.title, 300)
                    )))
                    .child(
                        div()
                            .absolute()
                            .right(px(7.))
                            .top_0()
                            .h(px(ROW_HEIGHT))
                            .flex()
                            .items_center()
                            .invisible()
                            .group_hover(ROW_GROUP, |style| style.visible())
                            .child(
                                Button::new(ElementId::Name(
                                    format!("pulls-more:{}:{}:{}", group.key(), link, number)
                                        .into(),
                                ))
                                .accessibility_label(say!("pulls-row-menu"))
                                .ghost()
                                .xsmall()
                                .icon(IconName::EllipsisVertical)
                                .dropdown_menu(
                                    move |menu, _, _| {
                                        Self::row_menu(
                                            menu,
                                            view.clone(),
                                            link_for_menu.clone(),
                                            number,
                                            url.clone(),
                                            head.clone(),
                                        )
                                    },
                                ),
                            ),
                    )
                    .test_support();
                // Its card beside it while the pointer rests on it; none
                // after a click until the pointer leaves the row.
                let row = row.on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                    if !*hovered && this.quiet {
                        this.quiet = false;
                        cx.notify();
                    }
                }));
                let row = if self.quiet {
                    row.into_any_element()
                } else {
                    with_card(
                        ElementId::Name(
                            format!("pull-card:{}:{}:{}:{}", group.key(), link, number, cards)
                                .into(),
                        ),
                        row,
                        move |_, cx| card(&shown_pull, kind, checks.as_ref(), now, cx),
                    )
                };
                // A right click opens the same menu.
                div()
                    .id(ElementId::Name(
                        format!("pull-menu:{}:{}:{}", group.key(), link, number).into(),
                    ))
                    .child(row)
                    .context_menu(move |menu, _, _| {
                        Self::row_menu(
                            menu,
                            menu_view.clone(),
                            menu_link.clone(),
                            number,
                            menu_url.clone(),
                            menu_head.clone(),
                        )
                    })
                    .into_any_element()
            }
            Row::Note {
                text,
                problem,
                indent,
            } => base(*indent)
                .id(("pulls-note", number))
                .text_xs()
                .text_color(if *problem {
                    theme.danger
                } else {
                    theme.muted_foreground
                })
                .child(div().w(px(14.)).flex_none())
                .child(div().min_w_0().truncate().child(text.clone()))
                .when(*problem, |row| {
                    let said = text.clone();
                    row.tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
                })
                .into_any_element(),
            Row::Invite { host } => {
                let host = shown_line(host, 120);
                base(0)
                    .id(("pulls-invite", number))
                    .h_auto()
                    .min_h(px(ROW_HEIGHT))
                    .py(px(4.))
                    .items_start()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::Info).size_3p5().flex_none().mt(px(1.)))
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_0p5()
                            .child(
                                div()
                                    .whitespace_normal()
                                    .child(say!("pulls-no-account", host)),
                            )
                            .child(
                                div()
                                    .id(("pulls-connect", number))
                                    .text_color(theme.link)
                                    .cursor_pointer()
                                    .hover(|style| style.underline())
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(PullsEvent::ConnectAccount)
                                    }))
                                    .child(say!("pulls-connect-account"))
                                    .test_support(),
                            ),
                    )
                    .into_any_element()
            }
        }
    }
}

/// The mark of a provider beside a remote's name.
pub fn provider_mark(kind: ProviderKind, cx: &App) -> AnyElement {
    let theme = cx.theme();
    match kind {
        ProviderKind::Github => Icon::new(IconName::Github)
            .size_3p5()
            .flex_none()
            .text_color(theme.muted_foreground)
            .into_any_element(),
        // The icon set has no mark of GitLab's: its initials, small.
        ProviderKind::Gitlab => div()
            .flex_none()
            .h(px(14.))
            .px(px(2.))
            .rounded(px(3.))
            .border_1()
            .border_color(theme.muted_foreground)
            .text_color(theme.muted_foreground)
            .text_size(px(8.))
            .line_height(px(12.))
            .font_semibold()
            .child("GL")
            .into_any_element(),
    }
}

impl Render for PullsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let offline = self.offline();
        let now = self.now;
        // Only the rows in view are made, each with its card and menus: a
        // project can have hundreds of open pull requests, and every one of
        // them would otherwise be laid out and painted with every frame of
        // the panel. The invitations, which wrap, stand under the list;
        // they come after every other row.
        let (listed, invites): (Vec<Row>, Vec<Row>) = self
            .rows()
            .into_iter()
            .partition(|row| !matches!(row, Row::Invite { .. }));
        let first_invite = listed.len();
        let invites: Vec<AnyElement> = invites
            .iter()
            .enumerate()
            .map(|(at, row)| self.render_row(first_invite.saturating_add(at), row, cx))
            .collect();
        let sizes = Rc::new(
            listed
                .iter()
                .map(|_| size(px(0.), px(ROW_HEIGHT)))
                .collect::<Vec<_>>(),
        );
        let list = (!listed.is_empty()).then(|| {
            let listed = Rc::new(listed);
            v_virtual_list(
                cx.entity(),
                "pulls-rows",
                sizes,
                move |this, range, _, cx| {
                    range
                        .filter_map(|number| Some(this.render_row(number, listed.get(number)?, cx)))
                        .collect::<Vec<_>>()
                },
            )
            .track_scroll(&self.scroll)
            .flex_1()
            .min_h_0()
        });
        let searching = !self.search_text.trim().is_empty();
        v_flex()
            .id("pulls")
            .size_full()
            .child(
                h_flex()
                    .w_full()
                    .h(px(SEARCH_ROW_HEIGHT))
                    .flex_none()
                    .pl(px(SEARCH_LEFT))
                    .pr(px(14.))
                    .gap_1()
                    .items_center()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .id("pulls-search")
                                .small()
                                .min_h(px(SEARCH_HEIGHT))
                                .cleanable(true)
                                .when(!searching, |field| {
                                    field.suffix(
                                        Icon::new(IconName::Search)
                                            .size_3p5()
                                            .text_color(theme.muted_foreground),
                                    )
                                }),
                        ),
                    )
                    .child(
                        Button::new("pulls-add-filter")
                            .accessibility_label(say!("pulls-add-filter"))
                            .ghost()
                            .xsmall()
                            .icon(Icon::new(gpui_kit::assets::IconName::ListFilterPlus))
                            .tooltip(say!("pulls-add-filter"))
                            .disabled(self.links.is_empty())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_filter(window, cx)),
                            ),
                    ),
            )
            .when_some(offline, |section, fetched| {
                let when = fetched
                    .as_deref()
                    .map(|time| super::facts::age(time, now))
                    .filter(|said| !said.is_empty());
                section.child(
                    h_flex()
                        .id("pulls-offline")
                        .w_full()
                        .h(px(20.))
                        .flex_none()
                        .px(px(10.))
                        .gap_1()
                        .items_center()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(gpui_kit::assets::IconName::CloudOff).size_3())
                        .child(match when {
                            Some(when) => say!("pulls-offline-as-read", when),
                            None => say!("pulls-offline-as-read-before").to_owned(),
                        })
                        .tooltip(|window, cx| {
                            Tooltip::new(say!("pulls-offline-tooltip")).build(window, cx)
                        }),
                )
            })
            .child(
                div()
                    .id("pulls-room")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(v_flex().size_full().children(list).children(invites))
                    .vertical_scrollbar(&self.scroll),
            )
    }
}
