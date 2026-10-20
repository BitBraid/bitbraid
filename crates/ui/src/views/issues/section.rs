//! What the ISSUES section of the left panel holds: at its top, the
//! repository whose issues are shown (a list to choose from when several
//! remotes are linked to accounts), a search box and a button that opens
//! a field for the provider's own query; then two groups, all the open
//! issues and the open ones assigned to the user, each closed and opened
//! from its heading and saying how many it holds. A row is `#n` and the
//! title, cut to fit; the card of the issue shows while the pointer rests
//! on it; a click opens the issue's view in the middle; its menu makes a
//! branch for the issue (named in the bar that asks, from the template of
//! the preferences), opens it at the provider or copies its link. A
//! remote at a provider no account is connected for has a line that
//! invites to connect one, as in PULL REQUESTS.
//!
//! The panel draws the section's heading (its count, a refresh, and a
//! plus for a new issue, greyed: issues are made at the provider) and
//! gives it its height; this view is what is under the heading.
//! The lists are read with `issue.list` as a standalone command with the
//! link the section holds: when the remotes are linked, when the section
//! comes into view, every five minutes while it is in view, when the
//! query changes, and when the heading's refresh is pressed (then past
//! what was kept).
//!
//! Sources: the description of `issue.list`.

use std::{
    collections::{BTreeSet, HashMap},
    rc::Rc,
    time::{Duration, Instant},
};

use bitbraid_git::Snapshot;
use bitbraid_integrations::model::{Account, Issue, Listing, RepoLink};
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Selectable, Sizable, VirtualListScrollHandle,
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
    card::{self, Known},
    facts::{IssueGroup, first_link, matches_search, number_text, query_help, repo_name},
};
use crate::{
    prefs::Prefs,
    session::{Reply, SessionHandle, run_standalone},
    views::pulls::{
        calls,
        card::with_card,
        facts::{may_open, provider_name, shown_line, unconnected_hosts},
        section::provider_mark,
    },
    zoom::px,
};

/// How often the lists are read again while the section is in view.
pub const READ_EVERY: Duration = Duration::from_secs(5 * 60);
/// Height of a row, as the references panel's rows are.
const ROW_HEIGHT: f32 = 24.0;
/// How far each level is indented.
const INDENT: f32 = 15.0;
/// The room between the parts of a row.
const ROW_GAP: f32 = 5.5;
/// What a row's controls that show under the pointer know it by.
const ROW_GROUP: &str = "issues-row";
/// How far past the column at the left an issue's number starts: where
/// the group's arrow stands.
const ISSUE_IN: f32 = 0.0;
/// The row of the repository's list and the list's box.
const CHOOSER_ROW_HEIGHT: f32 = 40.0;
const CHOOSER_HEIGHT: f32 = 32.0;
use crate::views::pulls::section::{SEARCH_HEIGHT, SEARCH_LEFT, SEARCH_ROW_HEIGHT};
/// The longest query kept.
const LONGEST_QUERY: usize = 512;

/// What the section asks of the window.
#[derive(Debug, Clone)]
pub enum IssuesEvent {
    /// Show this issue in the middle.
    Open {
        /// Where it is.
        link: RepoLink,
        number: u64,
    },
    /// Make a branch for this issue: the bar that asks names it.
    Branch {
        /// Where it is.
        link: RepoLink,
        issue: Box<Issue>,
    },
    /// Show where accounts are connected.
    ConnectAccount,
    /// Whether the section is shown, or its count, changed.
    Counted,
    /// The remotes linked to accounts, once known.
    Linked(Vec<RepoLink>),
}

/// A list of one group, as far as it was read.
#[derive(Debug, Clone, Default)]
struct Listed {
    listing: Option<Listing<Issue>>,
    error: Option<String>,
    loading: bool,
}

/// What the off-screen tests give the section in place of what the
/// providers would answer.
#[derive(Debug, Clone, Default)]
pub struct Given {
    pub links: Vec<RepoLink>,
    /// The lists of the chosen repository, by group.
    pub lists: Vec<(IssueGroup, Listing<Issue>)>,
    /// A list that could not be read, and why.
    pub errors: Vec<(IssueGroup, String)>,
    /// The hosts without an account.
    pub invite: Vec<String>,
}

/// One row of the section.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// A group's heading.
    Group {
        group: IssueGroup,
        count: usize,
        open: bool,
    },
    /// An issue: in which group, and its place in that list.
    Issue { group: IssueGroup, at: usize },
    /// A line about a list: it is being read, it is empty, or it could not
    /// be read.
    Note { text: String, problem: bool },
    /// An invitation to connect an account for a host.
    Invite { host: String },
}

/// What the section has on show, for the off-screen tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssuesProbe {
    /// Whether the panel shows the section at all.
    pub listed: bool,
    /// Whether its lists are in view.
    pub shown: bool,
    /// The repository whose issues are shown, `owner/name`.
    pub repository: Option<String>,
    /// The repositories to choose from.
    pub repositories: Vec<String>,
    /// Each group: its key, how many it shows and whether it is open.
    pub groups: Vec<(String, usize, bool)>,
    /// The issues' rows, top to bottom: number and title.
    pub rows: Vec<String>,
    /// The lines about the lists (reading, none, what went wrong).
    pub notes: Vec<String>,
    /// What the search box holds.
    pub search: String,
    /// The query the lists were asked with, and whether its field is open.
    pub query: String,
    pub query_open: bool,
    /// Whether a list is being read.
    pub reading: bool,
    /// The hosts it invites to connect an account for.
    pub invite: Vec<String>,
}

/// The section's view.
pub struct IssuesView {
    session: SessionHandle,
    /// How many times a row was clicked: the cards are known by it, so
    /// that a click puts the one on show away until the pointer comes
    /// back.
    cards: u64,
    /// A row was clicked and the pointer is still on it: no card shows
    /// until it leaves.
    quiet: bool,
    /// The open repository's remotes: name and address.
    remotes: Vec<(String, String)>,
    /// The remotes linked to an account, once asked.
    links: Vec<RepoLink>,
    /// The accounts, once asked.
    accounts: Vec<Account>,
    /// The hosts of remotes with no account.
    invite: Vec<String>,
    /// The remote whose issues are shown, by name; the first linked one
    /// (`origin` first) when none was chosen.
    chosen: Option<String>,
    /// The lists of the chosen repository, by group.
    lists: HashMap<IssueGroup, Listed>,
    /// The groups that are closed.
    closed: BTreeSet<String>,
    search: Entity<InputState>,
    search_text: String,
    /// The field of the provider's query, and the query the lists were
    /// asked with.
    query: Entity<InputState>,
    query_text: String,
    query_open: bool,
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
    /// The accounts the window knew when the links were last asked.
    account_ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<IssuesEvent> for IssuesView {}

impl IssuesView {
    /// Create the section for an open repository.
    pub fn new(session: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let state = cx.global::<Prefs>().repo_state(&session.path);
        let query_text: String = state
            .issue_query
            .clone()
            .unwrap_or_default()
            .chars()
            .take(LONGEST_QUERY)
            .collect();
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(say!("issues-search")));
        let query = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(say!("issues-query-placeholder"))
                .default_value(query_text.clone())
        });
        let searched =
            cx.subscribe_in(&search, window, |this, state, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.search_text = state.read(cx).value().to_string();
                    cx.emit(IssuesEvent::Counted);
                    cx.notify();
                }
            });
        let queried = cx.subscribe_in(&query, window, |this, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let typed = state.read(cx).value().to_string();
                this.set_query(&typed, cx);
            }
        });
        Self {
            session,
            cards: 0,
            quiet: false,
            remotes: Vec::new(),
            links: Vec::new(),
            accounts: Vec::new(),
            invite: Vec::new(),
            chosen: state.issue_repo.clone().filter(|name| name.len() <= 255),
            lists: HashMap::new(),
            closed: state
                .closed_issue_groups
                .iter()
                .filter(|key| {
                    IssueGroup::ALL
                        .iter()
                        .any(|group| group.key() == key.as_str())
                })
                .cloned()
                .collect(),
            search,
            search_text: String::new(),
            query,
            query_open: !query_text.is_empty(),
            query_text,
            shown: false,
            read_at: None,
            generation: 0,
            given: false,
            now: crate::views::now(),
            scroll: VirtualListScrollHandle::new(),
            ticker: None,
            account_ids: crate::providers::account_ids(cx),
            _subscriptions: vec![
                searched,
                queried,
                cx.observe_global::<crate::providers::Accounts>(Self::accounts_changed),
            ],
        }
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
        if remotes == self.remotes {
            return;
        }
        self.remotes = remotes;
        self.load_links(cx);
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
            // Given lists stay what they were given.
            if this.given {
                return;
            }
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
                cx.emit(IssuesEvent::Counted);
                cx.notify();
            },
        );
    }

    /// The linked remotes are known: the lists are read again when they
    /// changed, in view or not, since the heading counts them.
    fn set_links(&mut self, links: Vec<RepoLink>, cx: &mut Context<Self>) {
        let changed = links != self.links;
        self.links = links.into_iter().take(64).collect();
        self.invite = unconnected_hosts(&self.remotes, &self.links, &self.accounts);
        if changed {
            self.lists.clear();
            self.read_at = None;
            cx.emit(IssuesEvent::Linked(self.links.clone()));
            if self.link().is_some() {
                self.read(false, cx);
            }
        }
        cx.emit(IssuesEvent::Counted);
        cx.notify();
    }

    /// Take these as the linked remotes, as `provider.links` would have
    /// answered, and read their lists from then on: for the off-screen
    /// tests, whose stand-ins answer the provider commands.
    pub fn link_to(&mut self, links: Vec<RepoLink>, cx: &mut Context<Self>) {
        self.set_links(links, cx);
    }

    pub fn links(&self) -> &[RepoLink] {
        &self.links
    }

    /// The repository whose issues are shown: the chosen remote's when it
    /// is linked, else the first linked one.
    pub fn link(&self) -> Option<&RepoLink> {
        self.chosen
            .as_ref()
            .and_then(|chosen| self.links.iter().find(|link| &link.remote == chosen))
            .or_else(|| first_link(&self.links))
    }

    /// The provider whose issues are shown, once a remote is linked: the
    /// section's heading names it.
    pub fn provider(&self) -> Option<bitbraid_integrations::model::ProviderKind> {
        self.link().map(|link| link.kind)
    }

    /// Show the issues of another linked remote, and remember it.
    pub fn choose(&mut self, remote: &str, cx: &mut Context<Self>) {
        if !self.links.iter().any(|link| link.remote == remote)
            || self.link().is_some_and(|link| link.remote == remote)
        {
            return;
        }
        self.chosen = Some(remote.to_owned());
        let path = self.session.path.clone();
        let chosen = self.chosen.clone();
        cx.global_mut::<Prefs>()
            .update_repo_state(&path, |state| state.issue_repo = chosen);
        self.lists.clear();
        if !self.given {
            self.read(false, cx);
        }
        cx.emit(IssuesEvent::Counted);
        cx.notify();
    }

    /// Ask the lists with this query of the provider's from now on, and
    /// remember it.
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        let query: String = query.trim().chars().take(LONGEST_QUERY).collect();
        if query == self.query_text {
            return;
        }
        self.query_text = query;
        let path = self.session.path.clone();
        let kept = Some(self.query_text.clone()).filter(|query| !query.is_empty());
        cx.global_mut::<Prefs>()
            .update_repo_state(&path, |state| state.issue_query = kept);
        if !self.given {
            self.read(false, cx);
        }
        cx.emit(IssuesEvent::Counted);
        cx.notify();
    }

    /// Open or close the field of the provider's query. Closing it asks
    /// the lists without a query again.
    pub fn toggle_query(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.query_open = !self.query_open;
        if self.query_open {
            self.query.update(cx, |field, cx| field.focus(window, cx));
        } else {
            self.query
                .update(cx, |field, cx| field.set_value(String::new(), window, cx));
            self.set_query("", cx);
        }
        cx.notify();
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
        if stale && self.link().is_some() {
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

    /// Ask both lists of the chosen repository again.
    fn read(&mut self, fresh: bool, cx: &mut Context<Self>) {
        if self.given {
            return;
        }
        let Some(link) = self.link().cloned() else {
            return;
        };
        self.read_at = Some(Instant::now());
        self.now = crate::views::now();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        for group in IssueGroup::ALL {
            let mut params = json!({
                "account": link.account,
                "owner": link.owner,
                "name": link.name,
                "filter": group.filter(),
                "fresh": fresh,
            });
            if !self.query_text.is_empty()
                && let Some(object) = params.as_object_mut()
            {
                object.insert("query".to_owned(), json!(self.query_text));
            }
            self.lists.entry(group).or_default().loading = true;
            let reply = calls::run("issue.list", params);
            SessionHandle::deliver(
                reply,
                "issue.list".to_owned(),
                cx,
                move |this, reply, cx| {
                    if this.generation != generation {
                        return;
                    }
                    let listed = this.lists.entry(group).or_default();
                    listed.loading = false;
                    // The same issues as before (most readings, and every
                    // kept answer) change no count: the panel is not made
                    // over for them.
                    let error_before = listed.error.take();
                    let same = match reply {
                        Reply::Command(value) => {
                            match serde_json::from_value::<Listing<Issue>>(value) {
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
                                    listed.error = Some(say!("issues-reply-unreadable").to_owned());
                                    listed.error == error_before
                                }
                            }
                        }
                        Reply::Error(err) => {
                            let had = listed.listing.take().is_some();
                            listed.error = Some(crate::providers::said(&err));
                            !had && listed.error == error_before
                        }
                        _ => {
                            listed.error = error_before;
                            true
                        }
                    };
                    if !same {
                        cx.emit(IssuesEvent::Counted);
                    }
                    cx.notify();
                },
            );
        }
        cx.notify();
    }

    /// Show these in place of what the providers would answer, and read
    /// nothing from then on: for the off-screen tests.
    pub fn give(&mut self, given: Given, cx: &mut Context<Self>) {
        self.given = true;
        self.ticker = None;
        self.generation = self.generation.wrapping_add(1);
        self.links = given.links;
        self.invite = given.invite;
        self.lists = given
            .lists
            .into_iter()
            .map(|(group, listing)| {
                (
                    group,
                    Listed {
                        listing: Some(listing),
                        error: None,
                        loading: false,
                    },
                )
            })
            .collect();
        for (group, error) in given.errors {
            self.lists.insert(
                group,
                Listed {
                    listing: None,
                    error: Some(error),
                    loading: false,
                },
            );
        }
        self.now = crate::views::now();
        cx.emit(IssuesEvent::Linked(self.links.clone()));
        cx.emit(IssuesEvent::Counted);
        cx.notify();
    }

    /// How many open issues the heading counts, or none while the section
    /// is not to be shown: no remote is linked, and none invites to
    /// connect an account.
    pub fn heading_count(&self) -> Option<usize> {
        if self.links.is_empty() && self.invite.is_empty() {
            return None;
        }
        Some(self.found(IssueGroup::Open).len())
    }

    /// The issues of a list that the search box keeps, with their places
    /// in it.
    fn found(&self, group: IssueGroup) -> Vec<(usize, &Issue)> {
        self.lists
            .get(&group)
            .and_then(|listed| listed.listing.as_ref())
            .map(|listing| {
                listing
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, issue)| matches_search(issue, &self.search_text))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The issue a row stands for.
    fn issue_of(&self, group: IssueGroup, at: usize) -> Option<&Issue> {
        self.lists.get(&group)?.listing.as_ref()?.items.get(at)
    }

    /// The rows of the section, top to bottom.
    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        if self.link().is_some() {
            for group in IssueGroup::ALL {
                let found = self.found(group);
                let open = !self.closed.contains(group.key());
                rows.push(Row::Group {
                    group,
                    count: found.len(),
                    open,
                });
                if !open {
                    continue;
                }
                let note = |text: &str, problem: bool| Row::Note {
                    text: text.to_owned(),
                    problem,
                };
                // An empty list shows nothing under its group, whose count
                // says it.
                match self.lists.get(&group) {
                    Some(Listed {
                        error: Some(error), ..
                    }) => rows.push(note(&shown_line(error, 300), true)),
                    Some(Listed {
                        listing: Some(_), ..
                    }) => rows.extend(found.into_iter().map(|(at, _)| Row::Issue { group, at })),
                    _ => rows.push(note(say!("issues-reading"), false)),
                }
            }
        }
        rows.extend(
            self.invite
                .iter()
                .map(|host| Row::Invite { host: host.clone() }),
        );
        rows
    }

    /// Open or close a group, and remember it.
    pub fn toggle_group(&mut self, key: &str, cx: &mut Context<Self>) {
        if !IssueGroup::ALL.iter().any(|group| group.key() == key) {
            return;
        }
        if !self.closed.remove(key) {
            self.closed.insert(key.to_owned());
        }
        let closed: Vec<String> = self.closed.iter().cloned().collect();
        let path = self.session.path.clone();
        cx.global_mut::<Prefs>()
            .update_repo_state(&path, |state| state.closed_issue_groups = closed);
        cx.notify();
    }

    /// Open an issue's view by its number, from the lists: for the
    /// off-screen tests and the keyboard alike.
    pub fn open_number(&mut self, number: u64, cx: &mut Context<Self>) -> bool {
        let known = self.lists.values().any(|listed| {
            listed
                .listing
                .iter()
                .flat_map(|listing| &listing.items)
                .any(|issue| issue.number == number)
        });
        match self.link().cloned().filter(|_| known) {
            Some(link) => {
                cx.emit(IssuesEvent::Open { link, number });
                true
            }
            None => false,
        }
    }

    /// Ask for a branch for an issue of the lists, by its number: what the
    /// row's menu does.
    pub fn branch_for(&mut self, number: u64, cx: &mut Context<Self>) -> bool {
        let issue = self.lists.values().find_map(|listed| {
            listed
                .listing
                .as_ref()?
                .items
                .iter()
                .find(|issue| issue.number == number)
                .cloned()
        });
        match (self.link().cloned(), issue) {
            (Some(link), Some(issue)) => {
                cx.emit(IssuesEvent::Branch {
                    link,
                    issue: Box::new(issue),
                });
                true
            }
            _ => false,
        }
    }

    /// What the section has on show.
    pub fn probe(&self) -> IssuesProbe {
        let rows = self.rows();
        IssuesProbe {
            listed: self.heading_count().is_some(),
            shown: self.shown,
            repository: self.link().map(repo_name),
            repositories: self.links.iter().map(repo_name).collect(),
            groups: rows
                .iter()
                .filter_map(|row| match row {
                    Row::Group { group, count, open } => {
                        Some((group.key().to_owned(), *count, *open))
                    }
                    _ => None,
                })
                .collect(),
            rows: rows
                .iter()
                .filter_map(|row| match row {
                    Row::Issue { group, at } => self
                        .issue_of(*group, *at)
                        .map(|issue| format!("{} {}", number_text(issue.number), issue.title)),
                    _ => None,
                })
                .collect(),
            notes: rows
                .iter()
                .filter_map(|row| match row {
                    Row::Note { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect(),
            search: self.search_text.clone(),
            query: self.query_text.clone(),
            query_open: self.query_open,
            reading: self.lists.values().any(|listed| listed.loading),
            invite: self.invite.clone(),
        }
    }

    /// The menu of an issue's row.
    fn row_menu(
        menu: PopupMenu,
        view: WeakEntity<Self>,
        link: RepoLink,
        number: u64,
        url: String,
    ) -> PopupMenu {
        let branch = {
            let view = view.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let _ = view.update(cx, |this, cx| this.branch_for(number, cx));
            }
        };
        let open = {
            let link = link.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                let link = link.clone();
                let _ = view.update(cx, |_, cx| cx.emit(IssuesEvent::Open { link, number }));
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
        // The menu's groups: a branch for it; its page and its link;
        // then the issue shown here.
        let named = number_text(number);
        menu.item(PopupMenuItem::new(say!("issues-menu-branch")).on_click(branch))
            .separator()
            .item(
                PopupMenuItem::new(say!(
                    "issues-menu-open-at",
                    named = &named,
                    provider = provider_name(link.kind)
                ))
                .on_click(browse),
            )
            .item(PopupMenuItem::new(say!("issues-menu-copy-link", named = &named)).on_click(copy))
            .separator()
            .item(PopupMenuItem::new(say!("issues-menu-show-here", named = &named)).on_click(open))
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
        match row {
            Row::Group { group, count, open } => {
                let key = group.key();
                base(0)
                    .id(ElementId::Name(format!("issues-group:{key}").into()))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.accent))
                    // In the rows' colour.
                    .text_color(theme.foreground)
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_group(key, cx)))
                    .child(
                        Icon::new(if *open {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size_3p5()
                        .flex_none(),
                    )
                    .child(div().min_w_0().truncate().child(group.title()))
                    .child(
                        div()
                            .flex_none()
                            .ml_auto()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{count}")),
                    )
                    .test_support()
                    .into_any_element()
            }
            Row::Issue { group, at } => {
                let (Some(issue), Some(link)) = (self.issue_of(*group, *at), self.link()) else {
                    return div().into_any_element();
                };
                let kind = link.kind;
                let shown_issue = issue.clone();
                let now = self.now;
                let number = issue.number;
                let cards = self.cards;
                let link_for_click = link.clone();
                let link_for_card = link.clone();
                let view = cx.entity().downgrade();
                let (link_for_menu, url) = (link.clone(), issue.web_url.clone());
                let menu_view = view.clone();
                let (menu_link, menu_url) = (link.clone(), issue.web_url.clone());
                let row = base(0)
                    .id(ElementId::Name(
                        format!("issue:{}:{}", group.key(), issue.number).into(),
                    ))
                    .relative()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.accent))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        // A click puts the card away.
                        this.cards = this.cards.wrapping_add(1);
                        this.quiet = true;
                        cx.notify();
                        cx.emit(IssuesEvent::Open {
                            link: link_for_click.clone(),
                            number,
                        })
                    }))
                    // The number where the group's arrow stands; the rows
                    // have no mark.
                    .child(div().w(px(ISSUE_IN)).flex_none())
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme.muted_foreground)
                            .child(number_text(issue.number)),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .child(shown_line(&issue.title, 300)),
                    )
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
                                    format!("issues-more:{}:{}", group.key(), number).into(),
                                ))
                                .accessibility_label(say!("issues-row-menu"))
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
                                        )
                                    },
                                ),
                            ),
                    )
                    .test_support();
                // Its card beside it while the pointer rests on it. The
                // list has no descriptions: the issue is read for its card
                // once, as a key's card is, and its description shows when
                // that is in.
                // None after a click until the pointer leaves the row.
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
                            format!("issue-card:{}:{}:{}", group.key(), number, cards).into(),
                        ),
                        row,
                        move |_, cx| {
                            let body = match card::known(&link_for_card, shown_issue.number, cx) {
                                Known::Read(details) => Some(details.body),
                                Known::Reading | Known::Failed(_) => None,
                            };
                            card::card(&shown_issue, body.as_deref(), kind, now, cx)
                        },
                    )
                };
                // A right click opens the same menu.
                div()
                    .id(ElementId::Name(
                        format!("issue-menu:{}:{}", group.key(), number).into(),
                    ))
                    .child(row)
                    .context_menu(move |menu, _, _| {
                        Self::row_menu(
                            menu,
                            menu_view.clone(),
                            menu_link.clone(),
                            number,
                            menu_url.clone(),
                        )
                    })
                    .into_any_element()
            }
            Row::Note { text, problem } => base(1)
                .id(("issues-note", number))
                .text_xs()
                .text_color(if *problem {
                    theme.danger
                } else {
                    theme.muted_foreground
                })
                .child(div().min_w_0().truncate().child(text.clone()))
                .when(*problem, |row| {
                    let said = text.clone();
                    row.tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
                })
                .into_any_element(),
            Row::Invite { host } => {
                let host = shown_line(host, 120);
                base(0)
                    .id(("issues-invite", number))
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
                                    .child(say!("issues-no-account", host)),
                            )
                            .child(
                                div()
                                    .id(("issues-connect", number))
                                    .text_color(theme.link)
                                    .cursor_pointer()
                                    .hover(|style| style.underline())
                                    .on_click(cx.listener(|_, _, _, cx| {
                                        cx.emit(IssuesEvent::ConnectAccount)
                                    }))
                                    .child(say!("issues-connect-account"))
                                    .test_support(),
                            ),
                    )
                    .into_any_element()
            }
        }
    }

    /// The list of repositories to choose from, above the search box
    /// (with one repository too), and the button that asks the provider
    /// with a query beside it.
    fn render_chooser(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let link = self.link()?;
        let view = cx.entity().downgrade();
        let links = self.links.clone();
        let chosen = link.remote.clone();
        let theme = cx.theme().clone();
        let filtered = !self.query_text.is_empty();
        Some(
            h_flex()
                .w_full()
                .h(px(CHOOSER_ROW_HEIGHT))
                .flex_none()
                .pl(px(SEARCH_LEFT))
                .pr(px(14.))
                .gap_1()
                .items_center()
                .child(
                    div().flex_1().min_w_0().child(
                        Button::new("issues-repository")
                            .accessibility_label(say!("issues-repository"))
                            .outline()
                            .small()
                            .w_full()
                            .h(px(CHOOSER_HEIGHT))
                            .justify_start()
                            .bg(theme.input_background())
                            .child(provider_mark(link.kind, cx))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .text_sm()
                                    .child(shown_line(&link.name, 120)),
                            )
                            .child(Icon::new(IconName::ChevronDown).size_3().flex_none())
                            .tooltip(say!("issues-repository"))
                            .dropdown_menu(move |menu, _, _| {
                                links.iter().fold(menu, |menu, link| {
                                    let remote = link.remote.clone();
                                    let view = view.clone();
                                    menu.item(
                                        PopupMenuItem::new(format!(
                                            "{} ({})",
                                            shown_line(&repo_name(link), 120),
                                            shown_line(&link.remote, 60)
                                        ))
                                        .checked(link.remote == chosen)
                                        .on_click(
                                            move |_, _, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.choose(&remote, cx)
                                                });
                                            },
                                        ),
                                    )
                                })
                            }),
                    ),
                )
                .child(
                    Button::new("issues-filter")
                        .accessibility_label(say!("issues-query"))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(gpui_kit::assets::IconName::ListFilter))
                        .when(filtered, |button| button.selected(true))
                        .tooltip(if self.query_open {
                            say!("issues-query-close")
                        } else {
                            say!("issues-query-open")
                        })
                        .on_click(cx.listener(|this, _, window, cx| this.toggle_query(window, cx))),
                )
                .into_any_element(),
        )
    }
}

impl Render for IssuesView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // Only the rows in view are made, each with its card and menu: a
        // project can have thousands of open issues, and every one of them
        // would otherwise be laid out and painted with every frame of the
        // panel. The invitations, which wrap, stand under the list; they
        // come after every other row.
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
                "issues-rows",
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
        let help = self
            .link()
            .map(|link| query_help(link.kind))
            .unwrap_or(say!("issues-query-help"));
        let chooser = self.render_chooser(cx);
        v_flex()
            .id("issues")
            .size_full()
            .children(chooser)
            .child(
                h_flex()
                    .w_full()
                    .h(px(SEARCH_ROW_HEIGHT))
                    .flex_none()
                    .pl(px(SEARCH_LEFT))
                    .pr(px(17.))
                    .items_center()
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .id("issues-search")
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
                    ),
            )
            .when(self.query_open, |section| {
                section.child(
                    h_flex()
                        .id("issues-query-row")
                        .w_full()
                        .h(px(30.))
                        .flex_none()
                        .pl(px(SEARCH_LEFT))
                        .pr(px(8.))
                        .gap_1()
                        .items_center()
                        .child(
                            div().flex_1().min_w_0().child(
                                Input::new(&self.query)
                                    .id("issues-query")
                                    .small()
                                    .cleanable(true),
                            ),
                        )
                        // What Enter does, for the pointer; as wide as the
                        // filter button above it.
                        .child(
                            Button::new("issues-query-go")
                                .accessibility_label(say!("issues-query-go"))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Check)
                                .tooltip(say!("issues-query-go"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    let typed = this.query.read(cx).value().to_string();
                                    this.set_query(&typed, cx);
                                })),
                        )
                        .tooltip(move |window, cx| {
                            Tooltip::new(say!("issues-query-tooltip", help)).build(window, cx)
                        }),
                )
            })
            .child(
                div()
                    .id("issues-room")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(v_flex().size_full().children(list).children(invites))
                    .vertical_scrollbar(&self.scroll),
            )
    }
}
