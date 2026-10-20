//! The panel that makes a pull request. It stands 600 wide at the left of
//! the graph, over it, opened from a branch's menu, from a branch dropped
//! on another, from the plus of the PULL REQUESTS section or from the
//! palette, and goes away with its close button or Cancel.
//!
//! It asks, each field under its words: from which remote's branch and
//! into which remote's branch, side by side with an arrow between (that
//! remote's default branch at first), a template when the repository has
//! some, the title (the branch's one commit's summary when it has one
//! commit of its own, else made from the branch's name), the description
//! (the repository's template when it has one, picked from a list when it
//! has several), reviewers, assignees and labels as chips picked from what
//! the provider allows (`pr.options`), a milestone, and at GitHub whether
//! it is a draft. When, at GitHub, the branch it goes into has an open
//! pull request of its own (`pr.for_branch`), the new one is added to that
//! stack, on top of it, or goes where that one goes. A line says when the
//! two branches would conflict (`branch.predict` between the two tips). A
//! local branch that follows nothing on that remote is pushed first and
//! set to follow what it is pushed to (the window does that, through the
//! engine's push).
//!
//! Sources: docs.github.com/en/rest/pulls/pulls ("Create a pull request");
//! docs.gitlab.com/api/merge_requests ("Create MR");
//! docs.github.com/en/communities/using-templates-to-encourage-useful-issues-and-pull-requests.

use std::sync::Arc;

use bitbraid_git::{CommitInfo, Oid, Snapshot};
use bitbraid_integrations::model::{
    ProviderKind, PullOptions, PullRequest, PullTemplate, RepoLink,
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde::Deserialize;
use serde_json::json;

use super::{
    calls,
    card::label_chip,
    facts::{PullMark, may_open, number_text, provider_name, shown_line},
    section::provider_mark,
};
use crate::{
    session::{GraphData, SessionHandle},
    views::rewrite::{Rewrite, RunRewrite, descends},
    zoom::px,
};

/// The panel's measures: 600 wide at the graph's left, over it; its
/// heading; the room inside its edges; a field's words over it, their
/// line and the room under them; a field, the description, a button; from
/// one field to the next.
pub const PANEL_WIDTH: f32 = 600.0;
const HEADING_HEIGHT: f32 = 58.0;
const PANEL_PADDING: f32 = 20.0;
const LABEL_LINE: f32 = 20.0;
const LABEL_GAP: f32 = 4.0;
const FIELD_HEIGHT: f32 = 33.0;
const DESCRIPTION_HEIGHT: f32 = 113.0;
const BUTTON_HEIGHT: f32 = 31.0;
const ROW_GAP: f32 = 12.0;
/// What the fields leave at the right for the scroll bar, beside the
/// panel's padding, and the room on either side of the arrow between
/// where a pull request comes from and where it goes.
const SCROLL_ROOM: f32 = 7.0;
const ENDS_GAP: f32 = 20.0;
/// How high a list that drops from a button is at most.
const MENU_HEIGHT: f32 = 320.0;
/// The most entries such a list has.
const MOST_LISTED: usize = 500;
/// The most files a conflict line names.
const NAMED: usize = 4;

/// What the panel asks of the window.
#[derive(Debug, Clone)]
pub enum NewPullEvent {
    /// Put the panel away: the graph comes back.
    Close,
    /// Push this local branch to this remote and have it follow what it
    /// is pushed to; then tell the panel ([`NewPullView::pushed`]).
    Push {
        /// The local branch.
        branch: String,
        remote: String,
    },
    /// It was made: show it.
    Made {
        /// The repository it is in.
        link: RepoLink,
        /// What the provider answered.
        pull: Box<PullRequest>,
    },
}

/// Which of the people and labels a chip row picks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Chips {
    /// Who is asked to review it.
    Reviewers,
    /// Who it is assigned to.
    Assignees,
    /// Its labels.
    Labels,
}

impl Chips {
    fn key(self) -> &'static str {
        match self {
            Self::Reviewers => "reviewers",
            Self::Assignees => "assignees",
            Self::Labels => "labels",
        }
    }

    /// What the button that drops the list to pick from says.
    fn choose(self) -> &'static str {
        match self {
            Self::Reviewers => say!("pulls-new-choose-reviewers"),
            Self::Assignees => say!("pulls-new-choose-assignees"),
            Self::Labels => say!("pulls-new-choose-labels"),
        }
    }
}

/// What the panel has on show, for the off-screen tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewPullProbe {
    /// The remote and branch it comes from, `remote branch`.
    pub from: String,
    /// The remote and branch it goes into, as the provider is told.
    pub into: String,
    /// The title as it stands.
    pub title: String,
    /// The description as it stands.
    pub body: String,
    /// The chips picked.
    pub reviewers: Vec<String>,
    pub assignees: Vec<String>,
    pub labels: Vec<String>,
    /// The milestone picked, by its title.
    pub milestone: Option<String>,
    /// Whether it is to be a draft.
    pub draft: bool,
    /// How many templates there are to pick from.
    pub templates: usize,
    /// The open pull request of the branch it goes into, and whether the
    /// new one goes on top of it.
    pub stack: Option<(u64, bool)>,
    /// The files the two branches would conflict in, once worked out.
    pub conflicts: Option<Vec<String>>,
    /// The branch is pushed before it is made.
    pub push_first: bool,
    /// What went wrong last.
    pub error: Option<String>,
    /// It is being pushed or made.
    pub busy: bool,
    /// The button that makes it may be pressed.
    pub ready: bool,
}

/// The branch a pull request brings.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Head {
    /// A local branch by its name, or a remote's branch by its name there.
    name: String,
    /// It is a branch of the remote it comes from, not a local one.
    remote: bool,
}

/// The panel.
pub struct NewPullView {
    session: SessionHandle,
    links: Vec<RepoLink>,
    snapshot: Snapshot,
    graph: Option<Arc<GraphData>>,
    /// The link it comes from, and its branch.
    from: usize,
    head: Head,
    /// The link it goes into, and the branch there.
    to: usize,
    base: String,
    title: Entity<InputState>,
    body: Entity<TextareaState>,
    title_text: String,
    body_text: String,
    /// What the panel put in the title and the description: while they
    /// hold that, a better guess may replace it; once typed in, not.
    title_filled: String,
    body_filled: String,
    templates: Vec<PullTemplate>,
    template: Option<usize>,
    options: Option<PullOptions>,
    options_error: Option<String>,
    reviewers: Vec<String>,
    assignees: Vec<String>,
    labels: Vec<String>,
    milestone: Option<String>,
    draft: bool,
    /// The open pull request of the branch it goes into.
    stack: Option<PullRequest>,
    /// The new one goes on top of it.
    stacked: bool,
    /// The files the two tips would conflict in; none while not known.
    conflicts: Option<Vec<String>>,
    pushing: bool,
    creating: bool,
    error: Option<String>,
    /// Which pair of ends the answers belong to.
    generation: u64,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<NewPullEvent> for NewPullView {}

/// What the window knows when it opens the panel.
pub struct Opening {
    /// The open repository.
    pub session: SessionHandle,
    /// Its remotes linked to an account.
    pub links: Vec<RepoLink>,
    /// Its branches and remotes.
    pub snapshot: Snapshot,
    /// The graph shown, to tell a branch's own commits.
    pub graph: Option<Arc<GraphData>>,
    /// The branch it brings: a local one, or `remote/branch`.
    pub head: String,
    /// The branch it goes into, when one was named: a local one, or
    /// `remote/branch`.
    pub base: Option<String>,
}

/// The entry of a branch's menu that opens the panel (a local branch by
/// its name, a remote's as `remote/branch`). On another branch than the
/// one checked out it makes a pull request of the checked-out branch into
/// that one; on the checked-out branch, one of it into its remote's
/// default branch.
pub fn start_entry(menu: PopupMenu, branch: &str, checked_out: bool) -> PopupMenu {
    let (said, head, base) = if checked_out {
        (
            say!("pulls-start-from", branch = shown_line(branch, 80)),
            branch.to_owned(),
            None,
        )
    } else {
        (
            say!("pulls-start-into", branch = shown_line(branch, 80)),
            // The checked-out branch, as the section's plus has it.
            String::new(),
            Some(branch.to_owned()),
        )
    };
    menu.menu(
        said,
        Box::new(RunRewrite {
            rewrite: Rewrite::StartPull { head, base },
        }),
    )
}

/// The entries of a branch's menu for the open pull request it has,
/// before the one that starts a pull request: its page at the provider,
/// and its link.
pub fn open_pull_entries(menu: PopupMenu, mark: Option<&PullMark>) -> PopupMenu {
    let Some(mark) = mark else {
        return menu;
    };
    let named = number_text(mark.kind, mark.number);
    let (browse, copy) = (mark.web_url.clone(), mark.web_url.clone());
    menu.item(
        PopupMenuItem::new(say!(
            "pulls-menu-open-at",
            named = &named,
            provider = provider_name(mark.kind)
        ))
        .disabled(!may_open(&browse))
        .on_click(move |_, _, cx| {
            if may_open(&browse) {
                cx.open_url(&browse);
            }
        }),
    )
    .item(
        PopupMenuItem::new(say!("pulls-menu-copy-link", named = &named)).on_click(
            move |_, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(copy.clone())),
        ),
    )
}

/// The remote a name begins with, and the rest: `origin/feature/a` is
/// origin's `feature/a`. The longest remote's name that fits wins.
pub fn split_remote(remotes: &[&str], name: &str) -> Option<(String, String)> {
    remotes
        .iter()
        .filter_map(|remote| {
            let branch = name.strip_prefix(*remote)?.strip_prefix('/')?;
            (!branch.is_empty()).then(|| ((*remote).to_owned(), branch.to_owned()))
        })
        .max_by_key(|(remote, _)| remote.len())
}

/// A title made from a branch's name: its last part, words apart, the
/// first one capitalised (`feature/add-login` is "Add login").
pub fn title_from_branch(branch: &str) -> String {
    let last = branch.rsplit('/').next().unwrap_or(branch);
    let words: String = last
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();
    let words = words.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The branch a remote's pull requests go into when none is named: the
/// one its `HEAD` names, else `main`, `master`, else its first.
pub fn default_branch(remote: &bitbraid_git::snapshot::Remote) -> Option<String> {
    let has = |name: &str| remote.branches.iter().any(|branch| branch.name == name);
    remote
        .head
        .clone()
        .filter(|head| has(head))
        .or_else(|| {
            ["main", "master"]
                .into_iter()
                .find(|name| has(name))
                .map(str::to_owned)
        })
        .or_else(|| {
            remote
                .branches
                .iter()
                .map(|branch| branch.name.clone())
                .find(|name| name != "HEAD")
        })
}

/// What the provider is told the branch it brings is: its name, with its
/// owner in front when it is in another repository than the one the pull
/// request goes into (a fork's).
pub fn head_parameter(from: &RepoLink, to: &RepoLink, branch: &str) -> String {
    if from.owner == to.owner && from.name == to.name {
        branch.to_owned()
    } else {
        format!("{}:{branch}", from.owner)
    }
}

/// The conflict line's text: in which files, the first few named.
fn conflict_text(files: &[String]) -> String {
    let named: Vec<String> = files
        .iter()
        .take(NAMED)
        .map(|file| shown_line(file, 120))
        .collect();
    let more = files.len().saturating_sub(named.len());
    say!(
        "pulls-new-conflicts",
        files = say!("pulls-new-conflict-files", count = files.len()),
        named = named.join(", "),
        more
    )
}

/// What `branch.predict` answers.
#[derive(Debug, Clone, Deserialize)]
struct Prediction {
    known: bool,
    #[serde(default)]
    conflicts: Vec<String>,
}

impl NewPullView {
    /// The panel for a pull request of `opening.head`.
    pub fn new(opening: Opening, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let title = cx.new(|cx| InputState::new(window, cx).placeholder(say!("pulls-title")));
        let body = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(say!("pulls-description-placeholder"))
                // Tab goes on to the next control: a text that grows,
                // unlike one of fixed rows, is not indented.
                .auto_grow(8, 8)
        });
        let titled = cx.subscribe_in(&title, window, |this, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.title_text = state.read(cx).value().to_string();
                cx.notify();
            }
        });
        let described = cx.subscribe_in(&body, window, |this, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.body_text = state.read(cx).value().to_string();
                cx.notify();
            }
        });
        let Opening {
            session,
            links,
            snapshot,
            graph,
            head,
            base,
        } = opening;
        let mut view = Self {
            session,
            links,
            snapshot,
            graph,
            from: 0,
            head: Head {
                name: String::new(),
                remote: false,
            },
            to: 0,
            base: String::new(),
            title,
            body,
            title_text: String::new(),
            body_text: String::new(),
            title_filled: String::new(),
            body_filled: String::new(),
            templates: Vec::new(),
            template: None,
            options: None,
            options_error: None,
            reviewers: Vec::new(),
            assignees: Vec::new(),
            labels: Vec::new(),
            milestone: None,
            draft: false,
            stack: None,
            stacked: true,
            conflicts: None,
            pushing: false,
            creating: false,
            error: None,
            generation: 0,
            scroll: ScrollHandle::new(),
            _subscriptions: vec![titled, described],
        };
        view.place_ends(&head, base.as_deref());
        view.read_templates(window, cx);
        view.read_options(cx);
        view.ends_changed(window, cx);
        view
    }

    fn remote_names(&self) -> Vec<&str> {
        self.snapshot
            .remotes
            .iter()
            .map(|remote| remote.name.as_str())
            .collect()
    }

    fn link_of(&self, remote: &str) -> Option<usize> {
        self.links.iter().position(|link| link.remote == remote)
    }

    /// The link a remote's name means, else `origin`'s, else the first.
    fn link_or_first(&self, remote: Option<&str>) -> usize {
        remote
            .and_then(|remote| self.link_of(remote))
            .or_else(|| self.link_of("origin"))
            .unwrap_or(0)
    }

    /// Where the two ends are at first, from what was named.
    fn place_ends(&mut self, head: &str, base: Option<&str>) {
        let remotes: Vec<String> = self.remote_names().into_iter().map(str::to_owned).collect();
        let names: Vec<&str> = remotes.iter().map(String::as_str).collect();
        let local = self
            .snapshot
            .branches
            .iter()
            .find(|branch| branch.name == head)
            .cloned();
        match (local, split_remote(&names, head)) {
            (Some(branch), _) => {
                self.from = self.link_or_first(branch.remote.as_deref());
                self.head = Head {
                    name: branch.name,
                    remote: false,
                };
            }
            (None, Some((remote, name))) => {
                self.from = self.link_or_first(Some(&remote));
                self.head = Head { name, remote: true };
            }
            (None, None) => {
                self.from = self.link_or_first(None);
                self.head = Head {
                    name: head.to_owned(),
                    remote: false,
                };
            }
        }
        // Into what was named; else into the parent of a fork (a remote
        // called `upstream`), else where it comes from.
        let named = base.and_then(|base| {
            if let Some((remote, name)) = split_remote(&names, base) {
                return Some((remote, name));
            }
            let local = self.snapshot.branches.iter().find(|b| b.name == base)?;
            match (&local.upstream, &local.remote) {
                (Some(upstream), Some(remote)) => split_remote(&[remote.as_str()], upstream)
                    .or(Some((remote.clone(), base.to_owned()))),
                _ => Some((self.links.get(self.from)?.remote.clone(), base.to_owned())),
            }
        });
        match named {
            Some((remote, name)) => {
                self.to = self.link_or_first(Some(&remote));
                self.base = name;
            }
            None => {
                self.to = self.link_of("upstream").unwrap_or(self.from);
                self.base = self.default_base();
            }
        }
    }

    /// The default branch of the remote it goes into.
    fn default_base(&self) -> String {
        self.links
            .get(self.to)
            .and_then(|link| {
                self.snapshot
                    .remotes
                    .iter()
                    .find(|remote| remote.name == link.remote)
            })
            .and_then(default_branch)
            .unwrap_or_else(|| "main".to_owned())
    }

    fn source_link(&self) -> Option<&RepoLink> {
        self.links.get(self.from)
    }

    fn to_link(&self) -> Option<&RepoLink> {
        self.links.get(self.to)
    }

    /// The provider the pull request is made at.
    fn kind(&self) -> ProviderKind {
        self.to_link()
            .map_or(ProviderKind::Github, |link| link.kind)
    }

    /// The local branch it brings, when it is one.
    fn local_head(&self) -> Option<&bitbraid_git::snapshot::Branch> {
        if self.head.remote {
            return None;
        }
        self.snapshot
            .branches
            .iter()
            .find(|branch| branch.name == self.head.name)
    }

    /// The name of the branch it brings at the remote it comes from: a
    /// remote's branch as it is; a local one by what it follows there,
    /// else by its own name, as it is pushed.
    fn head_at_remote(&self) -> String {
        let Some(link) = self.source_link() else {
            return self.head.name.clone();
        };
        match self.local_head() {
            Some(branch) => branch
                .upstream
                .as_deref()
                .filter(|_| branch.remote.as_deref() == Some(link.remote.as_str()))
                .and_then(|upstream| split_remote(&[link.remote.as_str()], upstream))
                .map_or_else(|| branch.name.clone(), |(_, name)| name),
            None => self.head.name.clone(),
        }
    }

    /// Whether the branch has to be pushed first: a local one that
    /// follows nothing on the remote it comes from.
    fn push_first(&self) -> bool {
        let Some(link) = self.source_link() else {
            return false;
        };
        self.local_head().is_some_and(|branch| {
            branch.upstream.is_none() || branch.remote.as_deref() != Some(link.remote.as_str())
        })
    }

    /// The commit at the tip of the branch it brings.
    fn head_tip(&self) -> Option<Oid> {
        match self.local_head() {
            Some(branch) => Some(branch.target),
            None => self.remote_tip(self.source_link()?.remote.as_str(), &self.head.name),
        }
    }

    fn remote_tip(&self, remote: &str, branch: &str) -> Option<Oid> {
        self.snapshot
            .remotes
            .iter()
            .find(|known| known.name == remote)?
            .branches
            .iter()
            .find(|known| known.name == branch)
            .map(|known| known.target)
    }

    /// The branch the provider is told it goes into: below the stack's
    /// top when it is not added to it.
    fn base_parameter(&self) -> String {
        match &self.stack {
            Some(pull) if !self.stacked && self.kind() == ProviderKind::Github => {
                pull.base_ref.clone()
            }
            _ => self.base.clone(),
        }
    }

    /// Whether what is asked is enough to make it.
    fn ready(&self) -> bool {
        let same_repository = self.from == self.to;
        !self.title_text.trim().is_empty()
            && !self.head.name.is_empty()
            && !self.base.is_empty()
            && !(same_repository && self.head_at_remote() == self.base_parameter())
            && self.source_link().is_some()
            && self.to_link().is_some()
            && !self.pushing
            && !self.creating
    }

    /// The branches changed: what depends on them is asked again.
    fn ends_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.conflicts = None;
        self.stack = None;
        self.stacked = true;
        self.error = None;
        self.read_conflicts(cx);
        self.read_stack(cx);
        self.fill_title(window, cx);
        cx.notify();
    }

    /// The conflicts merging the branch into the other would meet, worked
    /// out between the two tips as fetched, writing nothing.
    fn read_conflicts(&mut self, cx: &mut Context<Self>) {
        let (Some(head), Some(link)) = (self.head_tip(), self.to_link()) else {
            return;
        };
        let Some(base) = self.remote_tip(&link.remote, &self.base) else {
            return;
        };
        let generation = self.generation;
        let asked = calls::ask_queued::<Prediction>(
            &self.session,
            "branch.predict",
            json!({ "ours": base.to_hex(), "target": head.to_hex(), "aside": true }),
        );
        cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                if let Ok(prediction) = answer
                    && prediction.known
                {
                    this.conflicts = Some(prediction.conflicts);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Whether the branch it goes into has an open pull request of its
    /// own (stacks are GitHub's).
    fn read_stack(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self
            .to_link()
            .filter(|link| link.kind == ProviderKind::Github)
        else {
            return;
        };
        let generation = self.generation;
        let asked = calls::ask::<Option<PullRequest>>(
            "pr.for_branch",
            json!({
                "account": link.account,
                "owner": link.owner,
                "name": link.name,
                "branch": self.base,
            }),
        );
        cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.stack = answer.ok().flatten();
                cx.notify();
            });
        })
        .detach();
    }

    /// The labels, milestones and people of the repository it goes into.
    fn read_options(&mut self, cx: &mut Context<Self>) {
        let Some(link) = self.to_link().cloned() else {
            return;
        };
        self.options = None;
        self.options_error = None;
        let to = self.to;
        let asked = calls::ask::<PullOptions>(
            "pr.options",
            json!({ "account": link.account, "owner": link.owner, "name": link.name }),
        );
        cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                if this.to != to {
                    return;
                }
                match answer {
                    Ok(options) => this.options = Some(options),
                    Err(error) => this.options_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The repository's templates: one is put in the description at once.
    fn read_templates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let asked =
            calls::ask_queued::<Vec<PullTemplate>>(&self.session, "pr.templates", json!({}));
        cx.spawn_in(window, async move |this, cx| {
            let answer = asked.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.templates = answer.unwrap_or_default();
                this.templates.truncate(MOST_LISTED);
                if this.templates.len() == 1 {
                    this.use_template(Some(0), window, cx);
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Put a template in the description, or take it out (none), unless
    /// something else was typed there.
    pub fn use_template(&mut self, at: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        if at.is_some_and(|at| at >= self.templates.len()) {
            return;
        }
        let typed = self.body_text != self.body_filled && !self.body_text.trim().is_empty();
        let text = at
            .and_then(|at| self.templates.get(at))
            .map(|template| template.body.clone())
            .unwrap_or_default();
        self.template = at;
        if typed && at.is_some() {
            // What was typed stays; the template goes under it.
            let joined = format!("{}\n\n{text}", self.body_text.trim_end());
            self.set_body(joined, window, cx);
        } else if !typed {
            self.set_body(text, window, cx);
        }
        cx.notify();
    }

    fn set_body(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.body_filled = text.clone();
        self.body_text = text.clone();
        self.body
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    /// Put the best guess of a title in the title, unless the user typed
    /// one: the branch's one commit's summary, else its name in words.
    fn fill_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.title_text != self.title_filled {
            return;
        }
        let guess = title_from_branch(&self.head.name);
        self.set_title(guess, window, cx);
        let Some(single) = self.single_commit() else {
            return;
        };
        let generation = self.generation;
        let asked = calls::ask_queued::<Vec<CommitInfo>>(
            &self.session,
            "repo.commits",
            json!({ "ids": [single.to_hex()] }),
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = asked.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if this.generation != generation || this.title_text != this.title_filled {
                    return;
                }
                if let Some(summary) = answer
                    .ok()
                    .and_then(|commits| commits.into_iter().next())
                    .map(|commit| shown_line(&commit.summary, 300))
                    .filter(|summary| !summary.is_empty())
                {
                    this.set_title(summary, window, cx);
                }
            });
        })
        .detach();
    }

    fn set_title(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.title_filled = text.clone();
        self.title_text = text.clone();
        self.title
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    /// The branch's one commit, when it has exactly one that the branch
    /// it goes into lacks, going by the graph that is shown.
    fn single_commit(&self) -> Option<Oid> {
        let graph = self.graph.as_deref()?;
        let tip = self.head_tip()?;
        let base = self.remote_tip(&self.to_link()?.remote, &self.base)?;
        let parent = *graph
            .commits
            .get(*graph.index.get(&tip)?)?
            .parents
            .first()?;
        (descends(graph, base, tip) == Some(false) && descends(graph, base, parent) == Some(true))
            .then_some(tip)
    }

    /// Choose where it comes from.
    pub fn set_from(
        &mut self,
        link: usize,
        head: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.links.get(link).is_none() {
            return;
        }
        let remote = self
            .snapshot
            .branches
            .iter()
            .all(|branch| branch.name != head);
        self.from = link;
        self.head = Head { name: head, remote };
        self.ends_changed(window, cx);
    }

    /// Choose where it goes.
    pub fn set_into(
        &mut self,
        link: usize,
        base: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.links.get(link).is_none() {
            return;
        }
        let moved = link != self.to;
        self.to = link;
        self.base = base;
        if moved {
            self.reviewers.clear();
            self.assignees.clear();
            self.labels.clear();
            self.milestone = None;
            self.read_options(cx);
        }
        self.ends_changed(window, cx);
    }

    /// Pick or drop a chip.
    pub fn toggle_chip(&mut self, chips: Chips, name: &str, cx: &mut Context<Self>) {
        let list = match chips {
            Chips::Reviewers => &mut self.reviewers,
            Chips::Assignees => &mut self.assignees,
            Chips::Labels => &mut self.labels,
        };
        match list.iter().position(|known| known == name) {
            Some(at) => {
                list.remove(at);
            }
            None if list.len() < 100 => list.push(name.to_owned()),
            None => {}
        }
        cx.notify();
    }

    /// Pick a milestone by its id, or none.
    pub fn set_milestone(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        self.milestone = id;
        cx.notify();
    }

    /// Whether it is to be a draft.
    pub fn set_draft(&mut self, draft: bool, cx: &mut Context<Self>) {
        self.draft = draft;
        cx.notify();
    }

    /// Whether it goes on top of the open pull request of the branch it
    /// goes into.
    pub fn set_stacked(&mut self, stacked: bool, cx: &mut Context<Self>) {
        self.stacked = stacked;
        cx.notify();
    }

    /// Make it: push the branch first where it has to be.
    pub fn create(&mut self, cx: &mut Context<Self>) {
        if !self.ready() {
            return;
        }
        self.error = None;
        if self.push_first()
            && let Some(remote) = self.source_link().map(|link| link.remote.clone())
        {
            self.pushing = true;
            cx.emit(NewPullEvent::Push {
                branch: self.head.name.clone(),
                remote,
            });
            cx.notify();
            return;
        }
        self.send(cx);
    }

    /// The window pushed the branch, or could not: with the branches as
    /// they are now, it is made, or the panel says why not.
    pub fn pushed(&mut self, done: Result<Snapshot, String>, cx: &mut Context<Self>) {
        if !self.pushing {
            return;
        }
        self.pushing = false;
        match done {
            Ok(snapshot) => {
                self.snapshot = snapshot;
                self.send(cx);
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    fn send(&mut self, cx: &mut Context<Self>) {
        let (Some(from), Some(to)) = (self.source_link().cloned(), self.to_link().cloned()) else {
            return;
        };
        let milestone = self.milestone.clone();
        let mut params = json!({
            "account": to.account,
            "owner": to.owner,
            "name": to.name,
            "head": head_parameter(&from, &to, &self.head_at_remote()),
            "base": self.base_parameter(),
            "title": self.title_text.trim(),
            "body": self.body_text,
            "draft": self.draft && to.kind == ProviderKind::Github,
            "reviewers": self.reviewers,
            "assignees": self.assignees,
            "labels": self.labels,
        });
        if let (Some(milestone), Some(object)) = (milestone, params.as_object_mut()) {
            object.insert("milestone".to_owned(), json!(milestone));
        }
        self.creating = true;
        let asked = calls::ask::<PullRequest>("pr.create", params);
        cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                this.creating = false;
                match answer {
                    Ok(pull) => cx.emit(NewPullEvent::Made {
                        link: to,
                        pull: Box::new(pull),
                    }),
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// What the panel has on show.
    pub fn probe(&self) -> NewPullProbe {
        let remote = |link: usize| {
            self.links
                .get(link)
                .map(|link| link.remote.clone())
                .unwrap_or_default()
        };
        let milestone = self.milestone.as_ref().and_then(|id| {
            self.options
                .as_ref()?
                .milestones
                .iter()
                .find(|milestone| milestone.id == *id)
                .map(|milestone| milestone.title.clone())
        });
        NewPullProbe {
            from: format!("{} {}", remote(self.from), self.head.name),
            into: format!("{} {}", remote(self.to), self.base_parameter()),
            title: self.title_text.clone(),
            body: self.body_text.clone(),
            reviewers: self.reviewers.clone(),
            assignees: self.assignees.clone(),
            labels: self.labels.clone(),
            milestone,
            draft: self.draft,
            templates: self.templates.len(),
            stack: self.stack.as_ref().map(|pull| (pull.number, self.stacked)),
            conflicts: self.conflicts.clone(),
            push_first: self.push_first(),
            error: self.error.clone(),
            busy: self.pushing || self.creating,
            ready: self.ready(),
        }
    }

    /// A field of the form: its words over it, then what it holds, as
    /// wide as the panel lets it be.
    fn field(label: &'static str, field: AnyElement, cx: &App) -> AnyElement {
        v_flex()
            .w_full()
            .gap(px(LABEL_GAP))
            .child(Self::words(label, cx))
            .child(div().w_full().child(field))
            .into_any_element()
    }

    /// The words over a field.
    fn words(label: &'static str, cx: &App) -> Div {
        div()
            .h(px(LABEL_LINE))
            .text_sm()
            .line_height(px(LABEL_LINE))
            .text_color(cx.theme().foreground)
            .child(label)
    }

    /// A line under the fields: a mark and what it says.
    fn note(id: &'static str, icon: Icon, colour: Hsla, text: String, cx: &App) -> AnyElement {
        h_flex()
            .id(id)
            .w_full()
            .gap_1p5()
            .items_start()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(icon.size_3p5().flex_none().mt(px(1.)).text_color(colour))
            .child(div().flex_1().min_w_0().whitespace_normal().child(text))
            .test_support()
            .into_any_element()
    }

    /// A button that drops a list of remotes, and one that drops the
    /// branches of the remote chosen.
    fn ends_row(&self, into: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (link_at, branch) = if into {
            (self.to, self.base.clone())
        } else {
            (self.from, self.head.name.clone())
        };
        let link = self.links.get(link_at);
        let side = if into { "into" } else { "from" };
        let view = cx.entity().downgrade();
        let links: Vec<(usize, String, String)> = self
            .links
            .iter()
            .enumerate()
            .map(|(at, link)| (at, link.remote.clone(), repository_name(link)))
            .collect();
        let remote_menu = {
            let view = view.clone();
            let snapshot_remotes: Vec<(String, Option<String>)> = self
                .snapshot
                .remotes
                .iter()
                .map(|remote| (remote.name.clone(), default_branch(remote)))
                .collect();
            let head = self.head.name.clone();
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let mut menu = menu;
                for (at, remote, repository) in &links {
                    let (at, view) = (*at, view.clone());
                    let fallback = snapshot_remotes
                        .iter()
                        .find(|(name, _)| name == remote)
                        .and_then(|(_, default)| default.clone())
                        .unwrap_or_else(|| "main".to_owned());
                    let head = head.clone();
                    menu = menu.item(
                        PopupMenuItem::new(format!("{repository} ({})", shown_line(remote, 80)))
                            .checked(at == link_at)
                            .on_click(move |_, window, cx| {
                                let (head, fallback) = (head.clone(), fallback.clone());
                                let _ = view.update(cx, |this, cx| {
                                    if into {
                                        this.set_into(at, fallback, window, cx)
                                    } else {
                                        this.set_from(at, head, window, cx)
                                    }
                                });
                            }),
                    );
                }
                menu
            }
        };
        let branches: Vec<String> = if into {
            link.and_then(|link| {
                self.snapshot
                    .remotes
                    .iter()
                    .find(|remote| remote.name == link.remote)
            })
            .map(|remote| {
                remote
                    .branches
                    .iter()
                    .map(|branch| branch.name.clone())
                    .filter(|name| name != "HEAD")
                    .take(MOST_LISTED)
                    .collect()
            })
            .unwrap_or_default()
        } else {
            self.snapshot
                .branches
                .iter()
                .map(|branch| branch.name.clone())
                .take(MOST_LISTED)
                .collect()
        };
        let branch_menu = {
            let chosen = branch.clone();
            move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
                let mut menu = menu.scrollable(true).max_h(px(MENU_HEIGHT));
                for name in &branches {
                    let (view, name) = (view.clone(), name.clone());
                    menu = menu.item(
                        PopupMenuItem::new(shown_line(&name, 120))
                            .checked(name == chosen)
                            .on_click(move |_, window, cx| {
                                let name = name.clone();
                                let _ = view.update(cx, |this, cx| {
                                    if into {
                                        this.set_into(link_at, name, window, cx)
                                    } else {
                                        this.set_from(link_at, name, window, cx)
                                    }
                                });
                            }),
                    );
                }
                menu
            }
        };
        let said = link.map(|link| {
            say!(
                "pulls-new-remote-tooltip",
                remote = shown_line(&link.remote, 80),
                repository = repository_name(link),
                provider = provider_name(link.kind)
            )
        });
        // One column of the two: the remote with its provider's mark, and
        // under it the branch, each a list to choose from across the
        // column.
        let mark = link.map(|link| provider_mark(link.kind, cx));
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(LABEL_GAP))
            .child(Self::words(
                if into {
                    say!("pulls-new-into")
                } else {
                    say!("pulls-new-from")
                },
                cx,
            ))
            .child(
                Button::new(ElementId::Name(format!("new-pull-{side}-remote").into()))
                    .accessibility_label(if into {
                        say!("pulls-new-into-repository")
                    } else {
                        say!("pulls-new-from-repository")
                    })
                    .outline()
                    .w_full()
                    .h(px(FIELD_HEIGHT))
                    .justify_start()
                    .bg(theme.input_background())
                    .children(mark)
                    // The repository as the provider names it; the remote
                    // in the tooltip.
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(link.map_or_else(String::new, repository_name)),
                    )
                    .child(Icon::new(IconName::ChevronDown).size_3p5().flex_none())
                    .when_some(said, |button, said| button.tooltip(said))
                    .dropdown_menu(remote_menu),
            )
            .child(div().h(px(ROW_GAP - LABEL_GAP)))
            .child(Self::words(say!("pulls-branch"), cx))
            .child(
                Button::new(ElementId::Name(format!("new-pull-{side}-branch").into()))
                    .accessibility_label(say!("pulls-new-branch-named", branch = &branch))
                    .outline()
                    .w_full()
                    .h(px(FIELD_HEIGHT))
                    .justify_start()
                    .bg(theme.input_background())
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(shown_line(&branch, 120)),
                    )
                    .child(Icon::new(IconName::ChevronDown).size_3p5().flex_none())
                    .dropdown_menu(branch_menu),
            )
            .when(!into && self.head.remote, |column| {
                column.child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(say!("pulls-new-remote-branch")),
                )
            })
            .into_any_element()
    }

    /// The chips picked, each with a cross that drops it, and a button
    /// that drops the list to pick from.
    fn chips_row(&self, chips: Chips, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let picked = match chips {
            Chips::Reviewers => &self.reviewers,
            Chips::Assignees => &self.assignees,
            Chips::Labels => &self.labels,
        };
        let choices: Vec<(String, Option<bitbraid_integrations::model::Label>)> =
            match (&self.options, chips) {
                (Some(options), Chips::Labels) => options
                    .labels
                    .iter()
                    .take(MOST_LISTED)
                    .map(|label| (label.name.clone(), Some(label.clone())))
                    .collect(),
                (Some(options), _) => options
                    .people
                    .iter()
                    .take(MOST_LISTED)
                    .map(|user| (user.login.clone(), None))
                    .collect(),
                (None, _) => Vec::new(),
            };
        let view = cx.entity().downgrade();
        let chosen = picked.clone();
        let menu_choices = choices.clone();
        let menu = move |menu: PopupMenu, _: &mut Window, _: &mut Context<PopupMenu>| {
            let mut menu = menu.scrollable(true).max_h(px(MENU_HEIGHT));
            for (name, _) in &menu_choices {
                let (view, name) = (view.clone(), name.clone());
                menu = menu.item(
                    PopupMenuItem::new(shown_line(&name, 80))
                        .checked(chosen.contains(&name))
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| this.toggle_chip(chips, &name, cx));
                        }),
                );
            }
            menu
        };
        let reason = match (&self.options, &self.options_error) {
            (Some(_), _) if choices.is_empty() => Some(say!("pulls-nothing-to-choose").to_owned()),
            (Some(_), _) => None,
            (None, Some(error)) => Some(error.clone()),
            (None, None) => Some(say!("pulls-reading-choices").to_owned()),
        };
        let view = cx.entity().downgrade();
        h_flex()
            .id(ElementId::Name(format!("new-pull-{}", chips.key()).into()))
            .w_full()
            .min_h(px(FIELD_HEIGHT))
            .pl(px(10.))
            .pr(px(4.))
            .py(px(4.))
            .gap_1()
            .flex_wrap()
            .items_center()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.input)
            .bg(theme.input_background())
            .children(picked.iter().enumerate().map(|(at, name)| {
                let label = choices
                    .iter()
                    .find(|(known, _)| known == name)
                    .and_then(|(_, label)| label.clone());
                let (view, drop_name) = (view.clone(), name.clone());
                let cross = div()
                    .id(ElementId::Name(
                        format!("new-pull-{}-drop:{at}", chips.key()).into(),
                    ))
                    .flex_none()
                    .cursor_pointer()
                    .child(Icon::new(IconName::Close).size_3())
                    .on_click(move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| this.toggle_chip(chips, &drop_name, cx));
                    })
                    .test_support();
                match label {
                    Some(label) => h_flex()
                        .gap_0p5()
                        .items_center()
                        .child(label_chip(&label, cx))
                        .child(cross)
                        .into_any_element(),
                    None => h_flex()
                        .h(px(22.))
                        .px_1p5()
                        .gap_1()
                        .items_center()
                        .rounded(px(11.))
                        .border_1()
                        .border_color(theme.border)
                        .text_xs()
                        .child(super::card::picture(
                            &bitbraid_integrations::model::User {
                                login: name.clone(),
                                ..Default::default()
                            },
                            16.,
                            cx,
                        ))
                        .child(shown_line(name, 60))
                        .child(cross)
                        .into_any_element(),
                }
            }))
            .when(picked.is_empty(), |row| {
                row.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(say!("pulls-none")),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new(ElementId::Name(
                    format!("new-pull-{}-add", chips.key()).into(),
                ))
                .ghost()
                .xsmall()
                .icon(IconName::ChevronDown)
                .tooltip(match &reason {
                    Some(reason) => reason.clone(),
                    None => chips.choose().to_owned(),
                })
                .disabled(reason.is_some())
                .dropdown_menu(menu),
            )
            .into_any_element()
    }

    fn milestone_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let milestones = self
            .options
            .as_ref()
            .map(|options| options.milestones.clone())
            .unwrap_or_default();
        let shown = self
            .milestone
            .as_ref()
            .and_then(|id| milestones.iter().find(|milestone| milestone.id == *id))
            .map_or_else(
                || say!("pulls-none").to_owned(),
                |milestone| shown_line(&milestone.title, 80),
            );
        let view = cx.entity().downgrade();
        let chosen = self.milestone.clone();
        let empty = milestones.is_empty();
        Button::new("new-pull-milestone")
            .accessibility_label(say!("pulls-milestone"))
            .outline()
            .w_full()
            .h(px(FIELD_HEIGHT))
            .justify_start()
            .bg(cx.theme().input_background())
            .child(div().flex_1().min_w_0().truncate().child(shown))
            .child(Icon::new(IconName::ChevronDown).size_3p5().flex_none())
            .disabled(empty)
            .when(empty, |button| {
                button.tooltip(say!("pulls-new-no-milestone"))
            })
            .dropdown_menu(move |menu: PopupMenu, _, _| {
                let none_view = view.clone();
                let mut menu = menu.scrollable(true).max_h(px(MENU_HEIGHT)).item(
                    PopupMenuItem::new(say!("pulls-none"))
                        .checked(chosen.is_none())
                        .on_click(move |_, _, cx| {
                            let _ = none_view.update(cx, |this, cx| this.set_milestone(None, cx));
                        }),
                );
                for milestone in milestones.iter().take(MOST_LISTED) {
                    let (view, id) = (view.clone(), milestone.id.clone());
                    menu = menu.item(
                        PopupMenuItem::new(shown_line(&milestone.title, 80))
                            .checked(chosen.as_deref() == Some(id.as_str()))
                            .on_click(move |_, _, cx| {
                                let id = id.clone();
                                let _ =
                                    view.update(cx, |this, cx| this.set_milestone(Some(id), cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    fn template_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let names: Vec<String> = self
            .templates
            .iter()
            .map(|template| shown_line(&template.name, 80))
            .collect();
        let shown = self
            .template
            .and_then(|at| names.get(at).cloned())
            .unwrap_or_else(|| say!("pulls-none").to_owned());
        let view = cx.entity().downgrade();
        let chosen = self.template;
        Button::new("new-pull-template")
            .accessibility_label(say!("pulls-new-template-named", shown = &shown))
            .outline()
            .w_full()
            .h(px(FIELD_HEIGHT))
            .justify_start()
            .bg(cx.theme().input_background())
            .child(div().flex_1().min_w_0().truncate().child(shown))
            .child(Icon::new(IconName::ChevronDown).size_3p5().flex_none())
            .dropdown_menu(move |menu: PopupMenu, _, _| {
                let none_view = view.clone();
                let mut menu = menu.scrollable(true).max_h(px(MENU_HEIGHT)).item(
                    PopupMenuItem::new(say!("pulls-none"))
                        .checked(chosen.is_none())
                        .on_click(move |_, window, cx| {
                            let _ = none_view
                                .update(cx, |this, cx| this.use_template(None, window, cx));
                        }),
                );
                for (at, name) in names.iter().enumerate() {
                    let view = view.clone();
                    menu = menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(chosen == Some(at))
                            .on_click(move |_, window, cx| {
                                let _ = view
                                    .update(cx, |this, cx| this.use_template(Some(at), window, cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }
}

impl Render for NewPullView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let kind = self.kind();
        let (new, push_and_create, create) = match kind {
            ProviderKind::Github => (
                say!("pulls-new-pull-request"),
                say!("pulls-push-and-create-pull-request"),
                say!("pulls-create-pull-request"),
            ),
            ProviderKind::Gitlab => (
                say!("pulls-new-merge-request"),
                say!("pulls-push-and-create-merge-request"),
                say!("pulls-create-merge-request"),
            ),
        };
        let heading = h_flex()
            .w_full()
            .h(px(HEADING_HEIGHT))
            .flex_none()
            .pl(px(PANEL_PADDING))
            .pr(px(12.))
            .gap(px(10.))
            .items_center()
            .child(provider_mark(kind, cx))
            .child(div().text_lg().child(new))
            .child(div().flex_1())
            .child(
                Button::new("new-pull-close")
                    .accessibility_label(say!("pulls-back-to-graph"))
                    .ghost()
                    .icon(IconName::Close)
                    .tooltip(say!("pulls-back-to-graph"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(NewPullEvent::Close))),
            );
        let mut rows: Vec<AnyElement> = Vec::new();
        // Where it comes from and where it goes, side by side with an
        // arrow between.
        rows.push(
            h_flex()
                .w_full()
                .items_start()
                .gap(px(ENDS_GAP))
                .child(self.ends_row(false, cx))
                .child(
                    div()
                        .flex_none()
                        .pt(px(LABEL_LINE + LABEL_GAP + (FIELD_HEIGHT - 16.) / 2.))
                        .child(
                            Icon::new(IconName::ArrowRight)
                                .size(px(16.))
                                .text_color(theme.muted_foreground),
                        ),
                )
                .child(self.ends_row(true, cx))
                .into_any_element(),
        );
        if self.push_first()
            && let Some(link) = self.source_link()
        {
            rows.push(Self::note(
                "new-pull-push-first",
                Icon::new(IconName::Info),
                theme.muted_foreground,
                say!(
                    "pulls-new-push-first",
                    branch = shown_line(&self.head.name, 120),
                    remote = shown_line(&link.remote, 80)
                ),
                cx,
            ));
        }
        if let Some(pull) = &self.stack {
            let below = shown_line(&pull.base_ref, 120);
            rows.push(
                h_flex()
                    .w_full()
                    .child(
                        Checkbox::new("new-pull-stack")
                            .label(say!(
                                "pulls-new-stack",
                                named = number_text(kind, pull.number),
                                title = shown_line(&pull.title, 120)
                            ))
                            .checked(self.stacked)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_stacked(*checked, cx)
                            })),
                    )
                    .into_any_element(),
            );
            if !self.stacked {
                rows.push(Self::note(
                    "new-pull-beside",
                    Icon::new(IconName::Info),
                    theme.muted_foreground,
                    say!("pulls-new-beside", below),
                    cx,
                ));
            }
        }
        match &self.conflicts {
            Some(files) if !files.is_empty() => rows.push(Self::note(
                "new-pull-conflicts",
                Icon::new(IconName::TriangleAlert),
                theme.warning,
                conflict_text(files),
                cx,
            )),
            Some(_) => rows.push(Self::note(
                "new-pull-clean",
                Icon::new(IconName::CircleCheck),
                theme.success,
                say!("pulls-new-clean").to_owned(),
                cx,
            )),
            None => {}
        }
        // The template before the title.
        if !self.templates.is_empty() {
            rows.push(Self::field(
                say!("pulls-new-template"),
                self.template_button(cx),
                cx,
            ));
        }
        rows.push(Self::field(
            say!("pulls-title"),
            Input::new(&self.title)
                .id("new-pull-title")
                .h(px(FIELD_HEIGHT))
                .into_any_element(),
            cx,
        ));
        rows.push(Self::field(
            say!("pulls-description"),
            div()
                .id("new-pull-body")
                .w_full()
                .h(px(DESCRIPTION_HEIGHT))
                .overflow_hidden()
                .child(Textarea::new(&self.body).h_full())
                .into_any_element(),
            cx,
        ));
        rows.push(Self::field(
            say!("pulls-reviewers"),
            self.chips_row(Chips::Reviewers, cx),
            cx,
        ));
        rows.push(Self::field(
            say!("pulls-assignees"),
            self.chips_row(Chips::Assignees, cx),
            cx,
        ));
        rows.push(Self::field(
            say!("pulls-labels"),
            self.chips_row(Chips::Labels, cx),
            cx,
        ));
        rows.push(Self::field(
            say!("pulls-milestone"),
            self.milestone_button(cx),
            cx,
        ));
        if kind == ProviderKind::Github {
            rows.push(
                h_flex()
                    .w_full()
                    .h(px(22.))
                    .child(
                        Checkbox::new("new-pull-draft")
                            .label(say!("pulls-new-draft"))
                            .checked(self.draft)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_draft(*checked, cx)
                            })),
                    )
                    .into_any_element(),
            );
        }
        let go = if self.pushing {
            say!("pulls-new-pushing")
        } else if self.creating {
            say!("pulls-new-creating")
        } else if self.push_first() {
            push_and_create
        } else {
            create
        };
        // What went wrong last, over the buttons.
        rows.push(
            div()
                .id("new-pull-error")
                .w_full()
                .text_xs()
                .text_color(theme.danger)
                .line_clamp(3)
                .whitespace_normal()
                .children(self.error.as_ref().map(|error| shown_line(error, 600)))
                .test_support()
                .into_any_element(),
        );
        // Cancel and create, each half the width, under the form.
        rows.push(
            h_flex()
                .w_full()
                .gap(px(10.))
                .child(
                    div().flex_1().child(
                        Button::new("new-pull-cancel")
                            .outline()
                            .w_full()
                            .h(px(BUTTON_HEIGHT))
                            .label(say!("pulls-cancel"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(NewPullEvent::Close))),
                    ),
                )
                .child(
                    div().flex_1().child(
                        Button::new("new-pull-create")
                            .primary()
                            .w_full()
                            .h(px(BUTTON_HEIGHT))
                            .label(go)
                            .loading(self.pushing || self.creating)
                            .disabled(!self.ready())
                            .on_click(cx.listener(|this, _, _, cx| this.create(cx))),
                    ),
                )
                .into_any_element(),
        );
        v_flex()
            .id("new-pull")
            .size_full()
            .bg(theme.sidebar)
            .border_r_1()
            .border_color(theme.border)
            .shadow_lg()
            .child(heading)
            .child(
                div()
                    .id("new-pull-room")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        v_flex()
                            .id("new-pull-rows")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .pl(px(PANEL_PADDING))
                            .pr(px(PANEL_PADDING + SCROLL_ROOM))
                            .pt(px(6.))
                            .pb(px(PANEL_PADDING))
                            .gap(px(ROW_GAP))
                            .children(rows)
                            .test_support(),
                    )
                    .vertical_scrollbar(&self.scroll),
            )
            .test_support()
    }
}

/// A repository as its provider names it: `owner/name`.
fn repository_name(link: &RepoLink) -> String {
    format!(
        "{}/{}",
        shown_line(&link.owner, 120),
        shown_line(&link.name, 120)
    )
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use bitbraid_integrations::model::{ProviderKind, RepoLink};

    use super::{conflict_text, head_parameter, split_remote, title_from_branch};

    #[test]
    fn a_title_is_made_from_the_branch_name() {
        assert_eq!(title_from_branch("feature/add-login"), "Add login");
        assert_eq!(title_from_branch("fix_the__parser"), "Fix the parser");
        assert_eq!(title_from_branch("éclair"), "Éclair");
        assert_eq!(title_from_branch("a/"), "");
        assert_eq!(title_from_branch(""), "");
    }

    #[test]
    fn the_longest_remote_that_fits_names_the_branch() {
        let remotes = ["origin", "origin/fork"];
        assert_eq!(
            split_remote(&remotes, "origin/fork/main"),
            Some(("origin/fork".to_owned(), "main".to_owned()))
        );
        assert_eq!(
            split_remote(&remotes, "origin/a/b"),
            Some(("origin".to_owned(), "a/b".to_owned()))
        );
        assert_eq!(split_remote(&remotes, "origin/"), None);
        assert_eq!(split_remote(&remotes, "main"), None);
    }

    #[test]
    fn a_forks_branch_is_named_with_its_owner() {
        let link = |owner: &str| RepoLink {
            remote: owner.to_owned(),
            account: "github:github.com:octo".to_owned(),
            kind: ProviderKind::Github,
            owner: owner.to_owned(),
            name: "tools".to_owned(),
        };
        assert_eq!(head_parameter(&link("octo"), &link("octo"), "wip"), "wip");
        assert_eq!(
            head_parameter(&link("ada"), &link("octo"), "wip"),
            "ada:wip"
        );
    }

    #[test]
    fn a_conflict_line_names_the_first_files() {
        let files: Vec<String> = (0..6).map(|n| format!("f{n}")).collect();
        let text = conflict_text(&files);
        assert!(
            text.starts_with("The two branches conflict in 6 files: f0, f1, f2, f3 and 2 more.")
        );
        assert!(conflict_text(&files[..1]).contains("in 1 file: f0."));
    }
}
