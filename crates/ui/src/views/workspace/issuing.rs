//! What the tab does with issues: the section's rows are made with the
//! tab and handed to the references panel; an issue opened from them, or
//! from a key in a commit's message or a summary, takes the middle in the
//! graph's place until its view is closed; a branch for an issue is named
//! in the bar that asks, from the template of the preferences, and made
//! by `issue.branch` on the repository's queue, so that undo sees it; the
//! commit box is told which branch is checked out, for the issue its name
//! holds.
//!
//! Sources: none (original design).

use bitbraid_git::Snapshot;
use bitbraid_integrations::model::{Issue, RepoLink};
use gpui_kit::*;
use serde_json::json;

use super::{Sheet, Workspace, WorkspaceEvent, branches::judge_branch_name, form::Verdict};
use crate::{
    notices::Notice,
    prefs::Prefs,
    views::issues::{
        IssueView, IssueViewEvent, IssuesEvent, IssuesView, facts::offered_branch_name,
    },
};

impl Workspace {
    /// What the issues section asked for.
    pub(super) fn issues_event(
        &mut self,
        event: &IssuesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            IssuesEvent::Open { link, number } => {
                self.open_issue(link.clone(), *number, window, cx)
            }
            IssuesEvent::Branch { link, issue } => {
                self.ask_issue_branch(link.clone(), issue, window, cx)
            }
            // Accounts are connected in the settings' Integrations section.
            IssuesEvent::ConnectAccount => cx.emit(WorkspaceEvent::OpenSettings(
                crate::views::preferences::Section::Integrations,
            )),
            // What the details and the graph draw keys as links of.
            IssuesEvent::Linked(links) => {
                if let Some(snapshot) = &self.snapshot {
                    let remotes = snapshot
                        .remotes
                        .iter()
                        .map(|remote| (remote.name.clone(), remote.fetch_url.clone()))
                        .collect();
                    let dir = self.session.summary.git_dir.clone();
                    crate::providers::set_links(&dir, remotes, links.clone(), cx);
                }
            }
            IssuesEvent::Counted => {}
        }
    }

    /// The repository's references were read again: the section asks
    /// which remotes are linked when they changed, the issue's view lists
    /// the branches made for it, and the commit box learns the branch.
    pub(super) fn issues_follow(&mut self, snapshot: &Snapshot, cx: &mut Context<Self>) {
        self.issues
            .update(cx, |view, cx| view.set_remotes(snapshot, cx));
        let local: Vec<String> = snapshot
            .branches
            .iter()
            .map(|branch| branch.name.clone())
            .collect();
        let current = snapshot
            .head
            .branch
            .clone()
            .filter(|_| !snapshot.head.detached);
        if let Some(view) = self.issue_view.clone() {
            let (local, current) = (local.clone(), current.clone());
            view.update(cx, |view, cx| view.set_branches(local, current, cx));
        }
        self.commit_box
            .update(cx, |view, cx| view.set_issue_branch(current, cx));
    }

    /// Show an issue in the middle, in the graph's place.
    pub fn open_issue(
        &mut self,
        link: RepoLink,
        number: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same = self
            .issue_view
            .as_ref()
            .is_some_and(|view| view.read(cx).shows() == (&link, number));
        if !same {
            let local = self.local_branch_names();
            let current = self.snapshot.as_ref().and_then(|snapshot| {
                snapshot
                    .head
                    .branch
                    .clone()
                    .filter(|_| !snapshot.head.detached)
            });
            let view = cx.new(|cx| IssueView::new(link, number, local, current, cx));
            self.issue_subscription = Some(cx.subscribe_in(
                &view,
                window,
                |this, _, event: &IssueViewEvent, window, cx| match event {
                    IssueViewEvent::Close => this.close_issue(cx),
                    IssueViewEvent::Branch { link, issue } => {
                        this.ask_issue_branch(link.clone(), issue, window, cx)
                    }
                    IssueViewEvent::Checkout(name) => this.checkout(name.clone(), window, cx),
                },
            ));
            self.issue_view = Some(view);
        }
        self.sheet = Some(Sheet::Issue);
        cx.notify();
    }

    /// Put the issue's view away: the graph comes back.
    pub fn close_issue(&mut self, cx: &mut Context<Self>) {
        if self.sheet == Some(Sheet::Issue) {
            self.sheet = None;
        }
        cx.notify();
    }

    /// The issues section's rows.
    pub fn issues(&self) -> Entity<IssuesView> {
        self.issues.clone()
    }

    /// The issue's view, while it has the middle.
    pub fn issue_view(&self) -> Option<Entity<IssueView>> {
        self.issue_view
            .clone()
            .filter(|_| self.sheet == Some(Sheet::Issue))
    }

    /// The middle while an issue is shown.
    pub(super) fn issue_page(&self) -> AnyElement {
        match &self.issue_view {
            Some(view) => view.clone().into_any_element(),
            None => div().into_any_element(),
        }
    }

    /// Ask in the bar for the name of a branch for an issue: what the
    /// template of the preferences makes of its number and title, to be
    /// changed at will. The branch starts at the commit checked out.
    pub fn ask_issue_branch(
        &mut self,
        link: RepoLink,
        issue: &Issue,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let template = cx.global::<Prefs>().preferences.issue_branch.clone();
        let offered = offered_branch_name(&template, issue);
        let number = issue.number;
        let workspace = cx.entity().downgrade();
        let made = move |checkout: bool| {
            let workspace = workspace.clone();
            let link = link.clone();
            move |texts: Vec<String>, window: &mut Window, cx: &mut App| {
                let name = texts.first().cloned().unwrap_or_default();
                let link = link.clone();
                let _ = workspace.update(cx, |this, cx| {
                    this.make_issue_branch(link, number, name, checkout, window, cx)
                });
            }
        };
        crate::ask::show(
            crate::ask::Ask::new(say!("dialogs-issue-branch-title", number))
                .field(say!("workspace-prompt-name"), &offered)
                .go(say!("dialogs-create-and-check-out"), made(true))
                .other(say!("dialogs-create"), made(false)),
            window,
            cx,
        );
    }

    /// Make the branch for an issue that was named in the bar. A name git
    /// would refuse, or one there is already, is said, and the bar asks
    /// again with it.
    fn make_issue_branch(
        &mut self,
        link: RepoLink,
        number: u64,
        name: String,
        checkout: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let tags: Vec<String> = self
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.tags.iter().map(|tag| tag.name.clone()).collect())
            .unwrap_or_default();
        if let Some(Verdict::Refuse(why)) =
            judge_branch_name(&name, &self.local_branch_names(), &tags)
        {
            crate::notices::push(window, cx, Notice::error(why));
            let workspace = cx.entity().downgrade();
            let again = move |checkout: bool| {
                let workspace = workspace.clone();
                let link = link.clone();
                move |texts: Vec<String>, window: &mut Window, cx: &mut App| {
                    let name = texts.first().cloned().unwrap_or_default();
                    let link = link.clone();
                    let _ = workspace.update(cx, |this, cx| {
                        this.make_issue_branch(link, number, name, checkout, window, cx)
                    });
                }
            };
            crate::ask::show(
                crate::ask::Ask::new(say!("dialogs-issue-branch-title", number))
                    .field(say!("workspace-prompt-name"), &name)
                    .go(say!("dialogs-create-and-check-out"), again(true))
                    .other(say!("dialogs-create"), again(false)),
                window,
                cx,
            );
            return;
        }
        self.run_mutation(
            "issue.branch",
            json!({
                "account": link.account,
                "owner": link.owner,
                "name": link.name,
                "number": number,
                "branch": name,
                "checkout": checkout,
            }),
            if checkout {
                say!(
                    "dialogs-issue-branch-created-checked-out",
                    name = name.as_str(),
                    number
                )
            } else {
                say!("dialogs-issue-branch-created", name = name.as_str(), number)
            },
            |_, _, _, _| false,
            window,
            cx,
        );
    }
}
