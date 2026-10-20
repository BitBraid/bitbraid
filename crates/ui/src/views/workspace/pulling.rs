//! What the tab does with the pull requests: the section's rows are made
//! with the tab and handed to the references panel; a pull request opened
//! from them, or from a `bitbraid://` link that names one, takes the
//! middle in the graph's place until its view is closed, and the panel
//! that makes one lies over the graph; the graph is told which repository
//! at a provider to ask its commits' checks of, which branches have an
//! open pull request, and whether pull requests can be made here at all.
//! A branch that is to be pushed before its pull request is made is pushed
//! here, as an operation, and the panel is told when it is.
//!
//! Sources: none (original design).

use bitbraid_integrations::model::{ProviderKind, RepoLink};
use gpui_kit::*;
use serde_json::json;

use super::{Sheet, Workspace, WorkspaceEvent};
use crate::{
    notices::Notice,
    session::{Reply, Request},
    views::pulls::{
        NewPullEvent, NewPullView, PullView, PullViewEvent, PullsEvent, PullsView, calls,
        create::Opening,
        facts::{number_text, provider_name},
    },
};

impl Workspace {
    /// What the pull requests section asked for.
    pub(super) fn pulls_event(
        &mut self,
        event: &PullsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PullsEvent::Open { link, number } => self.open_pull(link.clone(), *number, window, cx),
            PullsEvent::Checkout { link, number, fork } => {
                self.checkout_pull(link.clone(), *number, fork.clone(), window, cx)
            }
            PullsEvent::Reveal { sha } => match sha.parse::<bitbraid_git::Oid>() {
                Ok(id) => self.reveal(id, window, cx),
                Err(_) => {
                    crate::notices::push(window, cx, Notice::info(say!("dialogs-pull-no-head")))
                }
            },
            // Accounts are connected in the settings' Integrations section.
            PullsEvent::ConnectAccount => cx.emit(WorkspaceEvent::OpenSettings(
                crate::views::preferences::Section::Integrations,
            )),
            PullsEvent::Linked(link) => {
                let link = link.clone();
                self.graph_view
                    .update(cx, |view, cx| view.set_checks_source(link, cx));
            }
            PullsEvent::Marked(marks) => {
                let marks = marks.clone();
                self.graph_view
                    .update(cx, |view, cx| view.set_pull_marks(marks, cx));
            }
            // Whether pull requests can be made here: the graph's menus
            // and drops offer it then.
            PullsEvent::Counted => {
                let linked = !self.pulls.read(cx).links().is_empty();
                self.graph_view
                    .update(cx, |view, cx| view.set_pulls_linked(linked, cx));
            }
        }
    }

    /// Open the panel that makes a pull request of `head` (a local branch,
    /// `remote/branch`, or empty for the checked-out one), into `base`
    /// when one is named.
    pub fn start_pull(
        &mut self,
        head: String,
        base: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let links = self.pulls.read(cx).links();
        if links.is_empty() {
            crate::notices::push(window, cx, Notice::info(say!("dialogs-pull-not-linked")));
            return;
        }
        let Some(snapshot) = self.snapshot.clone() else {
            return;
        };
        let head = if head.is_empty() {
            snapshot
                .head
                .branch
                .clone()
                .filter(|_| !snapshot.head.detached)
        } else {
            Some(head)
        };
        let Some(head) = head else {
            crate::notices::push(window, cx, Notice::info(say!("dialogs-pull-detached")));
            return;
        };
        let opening = Opening {
            session: self.session.clone(),
            links,
            snapshot,
            graph: self.graph_view.read(cx).graph_data(),
            head,
            base,
        };
        let view = cx.new(|cx| NewPullView::new(opening, window, cx));
        self.new_pull_subscription = Some(cx.subscribe_in(
            &view,
            window,
            |this, view, event: &NewPullEvent, window, cx| match event {
                NewPullEvent::Close => this.close_new_pull(cx),
                NewPullEvent::Push { branch, remote } => {
                    this.push_for_pull(view.clone(), branch.clone(), remote.clone(), window, cx)
                }
                NewPullEvent::Made { link, pull } => {
                    crate::notices::push(
                        window,
                        cx,
                        Notice::success(say!(
                            "dialogs-pull-opened",
                            named = number_text(link.kind, pull.number)
                        )),
                    );
                    this.new_pull = None;
                    this.pulls.update(cx, |view, cx| view.refresh(cx));
                    this.open_pull(link.clone(), pull.number, window, cx);
                }
            },
        ));
        self.new_pull = Some(view);
        self.sheet = Some(Sheet::NewPull);
        cx.notify();
    }

    /// Put the panel that makes a pull request away: the graph comes back.
    pub fn close_new_pull(&mut self, cx: &mut Context<Self>) {
        if self.sheet == Some(Sheet::NewPull) {
            self.sheet = None;
        }
        self.new_pull = None;
        self.new_pull_subscription = None;
        cx.notify();
    }

    /// The panel that makes a pull request, while it has the middle.
    pub fn new_pull(&self) -> Option<Entity<NewPullView>> {
        self.new_pull
            .clone()
            .filter(|_| self.sheet == Some(Sheet::NewPull))
    }

    /// The panel that makes a pull request over the left of `graph`: 600
    /// wide, or what the middle has when it is narrower.
    pub(super) fn new_pull_over(&self, graph: AnyElement) -> AnyElement {
        div()
            .relative()
            .size_full()
            .child(graph)
            .children(self.new_pull.clone().map(|view| {
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .h_full()
                    .w(crate::zoom::px(crate::views::pulls::create::PANEL_WIDTH))
                    .max_w_full()
                    // What is under it gets none of the pointer's events.
                    .occlude()
                    .child(view)
            }))
            .into_any_element()
    }

    /// Push a branch before its pull request is made, setting it to follow
    /// what it is pushed to; then hand the panel the branches as they are.
    fn push_for_pull(
        &mut self,
        panel: Entity<NewPullView>,
        branch: String,
        remote: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let params = json!({ "branch": branch, "remote": remote, "set_upstream": true });
        let reply = match calls::answered("remote.push", &params) {
            Some(Ok(value)) => {
                let (sender, receiver) = futures::channel::oneshot::channel();
                let _ = sender.send(Reply::Command(value));
                receiver
            }
            Some(Err(error)) => {
                panel.update(cx, |panel, cx| panel.pushed(Err(error), cx));
                return;
            }
            None => {
                if self.refused_here("remote.push", window, cx) {
                    panel.update(cx, |panel, cx| {
                        panel.pushed(Err(say!("dialogs-pull-push-refused").to_owned()), cx)
                    });
                    return;
                }
                self.session.send(Request::Operation {
                    id: "remote.push".to_owned(),
                    params,
                })
            }
        };
        self.operation_began(say!("workspace-pushing"), Some("remote.push"));
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.operation_ended();
                let failed = match reply {
                    Ok(Reply::Error(error)) => Some(super::sentence(&error)),
                    Ok(_) => None,
                    Err(_) => Some(say!("dialogs-pull-push-no-answer").to_owned()),
                };
                this.after_mutation(cx);
                this.reload(cx);
                match failed {
                    Some(error) => panel.update(cx, |panel, cx| panel.pushed(Err(error), cx)),
                    // The branches again, now that it follows its remote's.
                    None => this.session.call(Request::Refs, cx, move |_, reply, cx| {
                        let done = match reply {
                            Reply::Refs(snapshot) => Ok(snapshot),
                            _ => Err(say!("dialogs-pull-branches-unreadable").to_owned()),
                        };
                        panel.update(cx, |panel, cx| panel.pushed(done, cx));
                    }),
                }
            });
        })
        .detach();
    }

    /// Show a pull request in the middle, in the graph's place.
    pub fn open_pull(
        &mut self,
        link: RepoLink,
        number: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let same = self
            .pull_view
            .as_ref()
            .is_some_and(|view| view.read(cx).shows() == (&link, number));
        if !same {
            let view = cx.new(|cx| PullView::new(link.clone(), number, cx));
            self.pull_subscription = Some(cx.subscribe_in(
                &view,
                window,
                move |this, _, event: &PullViewEvent, window, cx| match event {
                    PullViewEvent::Close => this.close_pull(cx),
                    PullViewEvent::Open(number) => {
                        this.open_pull(link.clone(), *number, window, cx)
                    }
                    PullViewEvent::Changed => this.pulls.update(cx, |view, cx| view.refresh(cx)),
                    PullViewEvent::Checkout { number, fork } => {
                        this.checkout_pull(link.clone(), *number, fork.clone(), window, cx)
                    }
                    // The branch it goes into, of the remote it is at: as
                    // two clicks on that branch's row would.
                    PullViewEvent::CheckoutBase(branch) => {
                        this.checkout(format!("{}/{branch}", link.remote), window, cx)
                    }
                },
            ));
            self.pull_view = Some(view);
        }
        self.pull_view_follows(cx);
        self.sheet = Some(Sheet::Pull);
        cx.notify();
    }

    /// Show the pull request a link names, once it is known
    /// which remotes are linked to an account: of the remote at the
    /// address the link gave, when it gave one and that remote is linked,
    /// else of the first linked remote, `origin` first. A notice says when
    /// no remote is linked, and when the provider cannot give it (a
    /// number it does not have among them); the view says that too.
    pub fn open_linked_pull(
        &mut self,
        number: u64,
        url: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let links = self.pulls.read(cx).links();
        let named = url.and_then(|url| self.remote_at(url));
        let link = named
            .and_then(|remote| links.iter().find(|link| link.remote == remote))
            .or_else(|| links.first())
            .cloned();
        let Some(link) = link else {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("dialogs-pull-link-not-linked", number)),
            );
            return;
        };
        self.open_pull(link.clone(), number, window, cx);
        let Some(view) = self.pull_view.clone() else {
            return;
        };
        cx.spawn_in(window, async move |this, cx| {
            // The provider is asked on another thread; its own limits end
            // the asking within a minute.
            for _ in 0..1_300 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let read = view.read_with(cx, |view, _| {
                    let (loading, error) = view.reading();
                    (loading, error.map(str::to_owned), view.shows().1)
                });
                let (loading, error, shown) = read;
                if shown != number {
                    return;
                }
                if loading {
                    continue;
                }
                if let Some(error) = error {
                    let kind = match link.kind {
                        ProviderKind::Github => "github",
                        ProviderKind::Gitlab => "gitlab",
                    };
                    let _ = this.update_in(cx, |_, window, cx| {
                        crate::notices::push(
                            window,
                            cx,
                            Notice::error(say!(
                                "dialogs-pull-link-unopened",
                                kind,
                                named = number_text(link.kind, number),
                                provider = provider_name(link.kind),
                                error,
                            )),
                        )
                    });
                }
                return;
            }
        })
        .detach();
    }

    /// The remote whose address is `url`, when one is (a trailing slash or
    /// `.git` aside).
    fn remote_at(&self, url: &str) -> Option<String> {
        let plain = |text: &str| {
            let text = text.trim_end_matches('/');
            text.strip_suffix(".git").unwrap_or(text).to_owned()
        };
        let wanted = plain(url);
        self.snapshot.as_ref()?.remotes.iter().find_map(|remote| {
            [&remote.fetch_url, &remote.push_url]
                .into_iter()
                .flatten()
                .any(|found| plain(found) == wanted)
                .then(|| remote.name.clone())
        })
    }

    /// Tell the pull request's view which branch is checked out here, for
    /// suggestions from local edits.
    pub(super) fn pull_view_follows(&mut self, cx: &mut Context<Self>) {
        let Some(view) = self.pull_view.clone() else {
            return;
        };
        let branch = self
            .snapshot
            .as_ref()
            .filter(|snapshot| !snapshot.head.detached)
            .and_then(|snapshot| snapshot.head.branch.clone());
        let session = self.session.clone();
        view.update(cx, |view, cx| view.set_here(Some(session), branch, cx));
    }

    /// Check out the branch a pull request brings (`pr.checkout`): from a
    /// fork none of the remotes here is, after asking, since that adds
    /// the fork as a remote.
    pub(super) fn checkout_pull(
        &mut self,
        link: RepoLink,
        number: u64,
        fork: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let params = json!({
            "account": link.account,
            "owner": link.owner,
            "name": link.name,
            "number": number,
        });
        let named = number_text(link.kind, number);
        let known = |fork: &str| {
            self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.remotes.iter().any(|remote| {
                    remote.fetch_url.as_deref().is_some_and(|url| {
                        let url = url.trim_end_matches('/').trim_end_matches(".git");
                        url.ends_with(&format!("/{fork}")) || url.ends_with(&format!(":{fork}"))
                    })
                })
            })
        };
        match fork.filter(|fork| !known(fork)) {
            Some(fork) => {
                let workspace = cx.entity().downgrade();
                crate::ask::show(
                    crate::ask::Ask::new(say!(
                        "dialogs-pull-from-fork-question",
                        named = named.as_str(),
                        fork = crate::views::pulls::facts::shown_line(&fork, 120)
                    ))
                    .go(
                        say!("dialogs-add-and-check-out"),
                        move |_, window, cx| {
                            let (params, named) = (params.clone(), named.clone());
                            let _ = workspace.update(cx, |this, cx| {
                                this.run_pull_operation("pr.checkout", params, named, window, cx)
                            });
                        },
                    ),
                    window,
                    cx,
                );
            }
            None => self.run_pull_operation("pr.checkout", params, named, window, cx),
        }
    }

    /// Run a command of a pull request that changes the repository, as
    /// an operation; what a stand-in of the off-screen tests answers
    /// stands for it there.
    fn run_pull_operation(
        &mut self,
        id: &str,
        params: serde_json::Value,
        named: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match calls::answered(id, &params) {
            Some(Ok(value)) => {
                let branch = value
                    .get("branch")
                    .and_then(|branch| branch.as_str())
                    .map(|branch| crate::views::pulls::facts::shown_line(branch, 120))
                    .unwrap_or_default();
                crate::notices::push(
                    window,
                    cx,
                    Notice::success(say!("dialogs-pull-checked-out-in", named, branch)),
                );
                self.after_mutation(cx);
                self.reload(cx);
            }
            Some(Err(error)) => {
                let notice = self.failed_notice(&error, cx);
                crate::notices::push(window, cx, notice);
            }
            None => self.run_operation(
                id,
                params,
                say!("dialogs-checking-out"),
                say!("workspace-checked-out", target = named),
                window,
                cx,
            ),
        }
    }

    /// Put the pull request's view away: the graph comes back.
    pub fn close_pull(&mut self, cx: &mut Context<Self>) {
        if self.sheet == Some(Sheet::Pull) {
            self.sheet = None;
        }
        cx.notify();
    }

    /// The pull requests section's rows.
    pub fn pulls(&self) -> Entity<PullsView> {
        self.pulls.clone()
    }

    /// The pull request's view, while it has the middle.
    pub fn pull_view(&self) -> Option<Entity<PullView>> {
        self.pull_view
            .clone()
            .filter(|_| self.sheet == Some(Sheet::Pull))
    }

    /// The middle while a pull request is shown.
    pub(super) fn pull_page(&self) -> AnyElement {
        match &self.pull_view {
            Some(view) => view.clone().into_any_element(),
            None => div().into_any_element(),
        }
    }
}
