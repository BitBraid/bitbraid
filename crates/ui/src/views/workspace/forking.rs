//! A repository's other copies at a provider: the tabs of the pane that
//! adds a remote, one per connected account beside the address, and
//! making a new repository at a provider out of the open one.
//!
//! An account's tab lists the account's forks of the repository a remote
//! is linked to, any of which is added as a remote named after its owner,
//! or offers to fork it there and add the fork as a remote named after
//! the user (`provider.fork`). Which remote is linked to which account is
//! `provider.links`; the forks are the account's own repositories whose
//! parent is that repository (`provider.repos`).
//!
//! Sources: docs.github.com "Create a fork"; docs.gitlab.com "Fork a
//! project".

use bitbraid_integrations::model::{HostedRepo, Listing, RepoLink};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::Workspace;
use crate::{
    providers::{self, Site},
    session::{Reply, Request},
    zoom::px,
};

/// From the pane's edges.
const SIDE: f32 = 20.;
/// A tab of the pane, and a fork's row.
const TAB_HEIGHT: f32 = 32.;
const ROW_HEIGHT: f32 = 34.;
/// The most forks listed.
const MOST_FORKS: usize = 50;

/// What the tabs of the pane keep.
#[derive(Default)]
pub(super) struct Fork {
    /// Which tab is chosen: nought for the address, else an account's,
    /// by its id.
    account: Option<String>,
    /// Which remote is linked to which account, once asked.
    links: Option<Result<Vec<RepoLink>, String>>,
    /// The account's repositories that are forks, once asked: for which
    /// account.
    forks: Option<(String, Result<Vec<HostedRepo>, String>)>,
    /// A fork is being made.
    busy: bool,
}

impl Fork {
    /// The account whose tab is chosen.
    pub(super) fn account(&self) -> Option<&str> {
        self.account.as_deref()
    }
}

/// The link of the account's to fork from: origin when it is one.
fn link_of<'a>(links: &'a [RepoLink], account: &str) -> Option<&'a RepoLink> {
    let mine = links.iter().filter(|link| link.account == account);
    mine.clone()
        .find(|link| link.remote == "origin")
        .or_else(|| links.iter().find(|link| link.account == account))
        .or(mine.into_iter().next())
}

impl Workspace {
    /// The tabs of the pane that adds a remote: the address, and each
    /// connected account. None without accounts.
    pub(super) fn docked_tabs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let fork = &self.docked.as_ref()?.fork;
        let accounts = providers::accounts(cx);
        if accounts.is_empty() {
            return None;
        }
        let theme = cx.theme().clone();
        let chosen = fork.account.clone();
        let tab =
            |id: SharedString, icon: Icon, label: String, on: bool, account: Option<String>| {
                h_flex()
                    .id(id)
                    .h(px(TAB_HEIGHT))
                    .px(px(10.))
                    .gap(px(6.))
                    .items_center()
                    .rounded(theme.radius)
                    .text_sm()
                    .cursor_pointer()
                    .when(on, |tab| tab.bg(theme.list_active))
                    .when(!on, |tab| tab.hover(|tab| tab.bg(theme.list_hover)))
                    .child(icon.size(px(14.)))
                    .child(div().max_w(px(140.)).truncate().child(label))
                    .on_click(
                        cx.listener(move |this, _, _, cx| {
                            this.choose_fork_tab(account.clone(), cx)
                        }),
                    )
                    .test_support()
            };
        Some(
            h_flex()
                .w_full()
                .px(px(SIDE - 6.))
                .pb(px(8.))
                .gap(px(4.))
                .flex_wrap()
                .child(tab(
                    "docked-tab-url".into(),
                    Icon::new(gpui_kit::assets::IconName::Link),
                    say!("workspace-remote-url").to_owned(),
                    chosen.is_none(),
                    None,
                ))
                .children(accounts.into_iter().take(8).map(|account| {
                    let on = chosen.as_deref() == Some(account.id.as_str());
                    tab(
                        SharedString::from(format!("docked-tab-{}", account.id)),
                        Site::of(&account).icon(),
                        providers::account_label(&account),
                        on,
                        Some(account.id),
                    )
                }))
                .into_any_element(),
        )
    }

    /// Choose a tab of the pane: the address, or an account's, whose
    /// links and forks are asked for when they are not known.
    pub fn choose_fork_tab(&mut self, account: Option<String>, cx: &mut Context<Self>) {
        let Some(docked) = self.docked.as_mut() else {
            return;
        };
        docked.fork.account = account.clone();
        cx.notify();
        let Some(account) = account else {
            return;
        };
        if docked.fork.links.is_none() {
            docked.fork.links = Some(Ok(Vec::new()));
            let rx = self.session.send(Request::Command {
                id: "provider.links".to_owned(),
                params: json!({}),
            });
            cx.spawn(async move |this, cx| {
                let answer = match rx.await {
                    Ok(Reply::Command(value)) => serde_json::from_value::<Vec<RepoLink>>(value)
                        .map_err(|err| {
                            say!("dialogs-fork-links-unreadable", error = err.to_string())
                        }),
                    Ok(Reply::Error(err)) => Err(providers::said(&err)),
                    _ => Err(say!("dialogs-no-answer").to_owned()),
                };
                let _ = this.update(cx, |this, cx| {
                    if let Some(docked) = this.docked.as_mut() {
                        docked.fork.links = Some(answer);
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        let known = docked
            .fork
            .forks
            .as_ref()
            .is_some_and(|(asked, _)| *asked == account);
        if !known {
            docked.fork.forks = None;
            let answer = providers::ask::<Listing<HostedRepo>>(
                "provider.repos",
                json!({ "account": account, "filter": "own" }),
            );
            cx.spawn(async move |this, cx| {
                let answer = answer.await.map(|listing| listing.items);
                let _ = this.update(cx, |this, cx| {
                    if let Some(docked) = this.docked.as_mut()
                        && docked.fork.account.as_deref() == Some(account.as_str())
                    {
                        docked.fork.forks = Some((account, answer));
                        cx.notify();
                    }
                });
            })
            .detach();
        }
    }

    /// Show an account's tab as if these had been answered: for the
    /// tests and the pictures.
    pub fn show_fork_tab(
        &mut self,
        account: String,
        links: Vec<RepoLink>,
        forks: Result<Vec<HostedRepo>, String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(docked) = self.docked.as_mut() {
            docked.fork.account = Some(account.clone());
            docked.fork.links = Some(Ok(links));
            docked.fork.forks = Some((account, forks));
            cx.notify();
        }
    }

    /// Fork the linked repository at the account, and add the fork as a
    /// remote: an operation, as a fetch is.
    fn fork_and_add(&mut self, remote: String, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(docked) = self.docked.as_mut() {
            docked.fork.busy = true;
        }
        self.undock(window, cx);
        self.run_operation(
            "provider.fork",
            json!({ "remote": remote }),
            say!("dialogs-forking"),
            say!("dialogs-forked", remote = remote.as_str()),
            window,
            cx,
        );
    }

    /// Add a fork that is there as a remote named after its owner.
    fn add_fork(&mut self, repo: HostedRepo, window: &mut Window, cx: &mut Context<Self>) {
        let name = remote_name(&repo.owner);
        if !providers::usable_address(&repo.clone_ssh) {
            return;
        }
        let url = repo.clone_ssh.trim().to_owned();
        self.undock(window, cx);
        self.add_remote_with_push(name, url, String::new(), window, cx);
    }

    /// The body of an account's tab.
    pub(super) fn fork_body(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let fork = &self.docked.as_ref()?.fork;
        let account_id = fork.account.clone()?;
        let account = providers::account(&account_id, cx)?;
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let site = Site::of(&account);
        let mut body = v_flex().w_full().px(px(SIDE)).pt(px(2.)).gap(px(12.));
        let link = match &fork.links {
            None => None,
            Some(Err(problem)) => {
                return Some(
                    body.child(
                        div()
                            .text_sm()
                            .text_color(theme.danger)
                            .child(say!("dialogs-fork-remote-links-unreadable", problem)),
                    )
                    .into_any_element(),
                );
            }
            Some(Ok(links)) => link_of(links, &account.id).cloned(),
        };
        let Some(link) = link else {
            return Some(
                body.child(
                    div()
                        .id("docked-fork-none")
                        .text_sm()
                        .whitespace_normal()
                        .text_color(muted)
                        .child(say!(
                            "dialogs-fork-nothing-to-fork",
                            login = account.login.as_str(),
                            site = site.name()
                        ))
                        .test_support(),
                )
                .into_any_element(),
            );
        };
        let full = format!("{}/{}", link.owner, link.name);
        body = body.child(div().text_sm().whitespace_normal().child(say!(
            "dialogs-fork-repository",
            full = full.as_str(),
            site = site.name(),
            remote = link.remote.as_str()
        )));
        let forks: Vec<HostedRepo> = match &fork.forks {
            Some((_, Ok(repos))) => repos
                .iter()
                .filter(|repo| {
                    repo.parent
                        .as_deref()
                        .is_some_and(|parent| parent.eq_ignore_ascii_case(&full))
                })
                .take(MOST_FORKS)
                .cloned()
                .collect(),
            _ => Vec::new(),
        };
        let has_fork = !forks.is_empty();
        body = body.child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(muted)
                .child(say!(
                    "dialogs-forks-heading",
                    login = account.login.as_str()
                )),
        );
        match &fork.forks {
            None => {
                body = body.child(
                    h_flex()
                        .gap(px(8.))
                        .text_sm()
                        .text_color(muted)
                        .child(Spinner::new().xsmall())
                        .child(say!("dialogs-forks-asking")),
                )
            }
            Some((_, Err(problem))) => {
                body = body.child(
                    div()
                        .id("docked-fork-problem")
                        .text_xs()
                        .whitespace_normal()
                        .text_color(muted)
                        .child(say!("dialogs-forks-unlisted", problem))
                        .test_support(),
                )
            }
            Some((_, Ok(_))) if forks.is_empty() => {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(say!("dialogs-forks-none", login = account.login.as_str())),
                )
            }
            Some((_, Ok(_))) => {
                let remotes = self.remote_names();
                for (ix, repo) in forks.into_iter().enumerate() {
                    let name = remote_name(&repo.owner);
                    let there = remotes.contains(&name);
                    let shown = providers::shown(&repo.full_name);
                    body = body.child(
                        h_flex()
                            .id(("docked-fork", ix))
                            .h(px(ROW_HEIGHT))
                            .gap(px(8.))
                            .items_center()
                            .child(
                                Icon::new(gpui_kit::assets::IconName::GitFork)
                                    .size(px(14.))
                                    .text_color(muted),
                            )
                            .child(div().flex_1().min_w_0().truncate().text_sm().child(shown))
                            .child(
                                Button::new(("docked-fork-add", ix))
                                    .outline()
                                    .small()
                                    .label(if there {
                                        say!("dialogs-fork-is-remote", name = name.as_str())
                                    } else {
                                        say!("dialogs-fork-add-as", name = name.as_str())
                                    })
                                    .disabled(there)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.add_fork(repo.clone(), window, cx)
                                    })),
                            )
                            .test_support(),
                    );
                }
            }
        }
        let remote = link.remote.clone();
        let busy = fork.busy;
        // With a fork there, that is what is added; one more is not made.
        if has_fork {
            return Some(body.into_any_element());
        }
        body = body.child(
            v_flex()
                .gap(px(6.))
                .pt(px(4.))
                .child(
                    Button::new("docked-fork-go")
                        .primary()
                        .small()
                        .w_full()
                        .h(px(30.))
                        .label(say!("dialogs-fork-and-add"))
                        .loading(busy)
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.fork_and_add(remote.clone(), window, cx)
                        })),
                )
                .child(
                    div()
                        .text_xs()
                        .whitespace_normal()
                        .text_color(muted)
                        .child(say!(
                            "dialogs-fork-explained",
                            full = full.as_str(),
                            login = account.login.as_str(),
                            name = remote_name(&account.login)
                        )),
                ),
        );
        Some(body.into_any_element())
    }

    /// Ask which remote is which repository at which account, when the
    /// remotes changed since that was last asked: the pictures of their
    /// owners go by it.
    pub(super) fn look_at_links(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        if providers::accounts(cx).is_empty() {
            return;
        }
        let remotes: Vec<(String, Option<String>)> = snapshot
            .remotes
            .iter()
            .map(|remote| (remote.name.clone(), remote.fetch_url.clone()))
            .collect();
        let dir = self.session.summary.git_dir.clone();
        if providers::links_asked_for(&dir, cx).as_ref() == Some(&remotes) {
            return;
        }
        // Asked once for these remotes, whatever comes of it.
        providers::set_links(&dir, remotes.clone(), Vec::new(), cx);
        let rx = self.session.send(Request::Command {
            id: "provider.links".to_owned(),
            params: json!({}),
        });
        cx.spawn(async move |_, cx| {
            if let Ok(Reply::Command(value)) = rx.await
                && let Ok(links) = serde_json::from_value::<Vec<RepoLink>>(value)
            {
                cx.update(|cx| providers::set_links(&dir, remotes, links, cx));
            }
        })
        .detach();
    }

    /// Make a repository at a provider out of the open one, add it as the
    /// remote `origin` and push there: the second half of making a
    /// repository at a provider from the dialog that makes one.
    pub(crate) fn publish_to_provider(
        &mut self,
        params: serde_json::Value,
        label: String,
        done: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_operation("provider.repo_create", params, &label, done, window, cx);
    }
}

/// The remote a fork of an owner's is added as: the owner's name, as a
/// remote's name may be.
fn remote_name(owner: &str) -> String {
    let name: String = owner
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches(['-', '.']).to_owned();
    if name.is_empty() {
        "fork".to_owned()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_integrations::model::{ProviderKind, RepoLink};

    use super::{link_of, remote_name};

    fn link(remote: &str, account: &str) -> RepoLink {
        RepoLink {
            remote: remote.to_owned(),
            account: account.to_owned(),
            kind: ProviderKind::Github,
            owner: "team".to_owned(),
            name: "engine".to_owned(),
        }
    }

    #[test]
    fn origin_is_forked_from_when_it_is_the_accounts() {
        let links = vec![
            link("upstream", "a"),
            link("origin", "a"),
            link("mine", "b"),
        ];
        assert_eq!(link_of(&links, "a").unwrap().remote, "origin");
        assert_eq!(link_of(&links, "b").unwrap().remote, "mine");
        assert!(link_of(&links, "c").is_none());
    }

    #[test]
    fn a_forks_remote_is_named_after_its_owner() {
        assert_eq!(remote_name("ada-lovelace"), "ada-lovelace");
        assert_eq!(remote_name("group/sub"), "group-sub");
        assert_eq!(remote_name("../"), "fork");
    }
}
