//! Cloning from a provider's list: an account's place in the column of
//! the dialog that clones, beside the address.
//!
//! Where to clone to; which repositories are listed (the account's own,
//! its organisations', the starred ones, or all it may see); then the
//! repository, a field that searches them with the list open under it, in
//! the column of the fields, the repositories grouped under their owners'
//! names in small capitals, a fork marked as one, a lock on a private one
//! and a book on the others. Then whether the address is the SSH or the
//! HTTPS one, which is said under the list with the address itself.
//! Picking one fills the path; the clone runs as one from an address does
//! (`repo.clone`).
//!
//! The list comes from `provider.repos`, a page at a time; what is typed
//! narrows what was read at once, and when the provider has more than it
//! sent, it is asked with the words a moment after the typing stops.
//!
//! Sources: docs.github.com "List repositories for the authenticated
//! user" (`affiliation`, starred); docs.gitlab.com "Projects API"
//! (`membership`, `owned`, `starred`, `search`).

use std::{path::PathBuf, time::Duration};

use bitbraid_integrations::model::{Account, HostedRepo, Listing};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::{ScrollableElement, ScrollbarAxis},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use crate::{
    providers::{self, Site},
    zoom::px,
};

/// The column of words before the fields, as the form has it.
const LABELS: f32 = crate::views::workspace::form::LABELS;
const LABEL_GAP: f32 = 15.;
/// A field's row, and from one row to the next.
const ROW_HEIGHT: f32 = 34.;
const ROW_GAP: f32 = 10.;
/// The pane's edges, as the form's.
const PANE_LEFT: f32 = 11.;
const PANE_RIGHT: f32 = 20.;
const PANE_TOP: f32 = 10.;
/// The heading over the rows.
const HEADING: f32 = 24.;
/// An owner's heading and a repository's row in the list.
const OWNER_HEIGHT: f32 = 26.;
const REPO_HEIGHT: f32 = 37.;
/// How long typing has to pause before the provider is asked.
const PAUSE: Duration = Duration::from_millis(400);
/// The most repositories kept.
const MOST: usize = 2000;

/// Which repositories of an account are listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Filter {
    /// The account's own.
    Own,
    /// Its organisations' or groups'.
    Organisation,
    /// The starred ones.
    Starred,
    /// Every one it may see.
    All,
}

impl Filter {
    /// In the order the buttons have them.
    pub const ALL: [Filter; 4] = [
        Filter::Own,
        Filter::Organisation,
        Filter::Starred,
        Filter::All,
    ];

    /// What `provider.repos` calls it.
    pub fn word(self) -> &'static str {
        match self {
            Filter::Own => "own",
            Filter::Organisation => "organisation",
            Filter::Starred => "starred",
            Filter::All => "all",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Filter::Own => say!("provider-clone-filter-own"),
            Filter::Organisation => say!("provider-clone-filter-organisation"),
            Filter::Starred => say!("provider-clone-filter-starred"),
            Filter::All => say!("provider-clone-filter-all"),
        }
    }
}

/// What the tab asks of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderCloneEvent {
    /// Clone from this address into this folder.
    Clone {
        url: String,
        /// The new folder.
        path: PathBuf,
    },
}

/// Where the list stands.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Read {
    /// Not asked for yet: the place was not shown.
    NotAsked,
    Loading,
    Loaded,
    Failed(String),
}

/// What the tab has on show, for the tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCloneProbe {
    /// The filter in force.
    pub filter: &'static str,
    /// The owners shown and the names under each.
    pub groups: Vec<(String, Vec<String>)>,
    /// The repository picked, `owner/name`.
    pub picked: Option<String>,
    /// The address the clone would be from.
    pub url: Option<String>,
    /// Where it would go.
    pub path: Option<PathBuf>,
    /// Whether the list is being read.
    pub loading: bool,
    /// What went wrong reading it.
    pub problem: Option<String>,
}

/// The repositories that match what is typed, grouped by owner: the
/// account's own first, then the others by name, each group by name.
/// Every word typed has to be in the full name or the description.
pub fn grouped(repos: &[HostedRepo], query: &str, login: &str) -> Vec<(String, Vec<usize>)> {
    let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut groups: Vec<(String, Vec<usize>)> = Vec::new();
    for (ix, repo) in repos.iter().enumerate() {
        let text = format!(
            "{} {}",
            repo.full_name.to_lowercase(),
            repo.description
                .as_deref()
                .unwrap_or_default()
                .to_lowercase()
        );
        if !words.iter().all(|word| text.contains(word.as_str())) {
            continue;
        }
        match groups.iter_mut().find(|(owner, _)| *owner == repo.owner) {
            Some((_, members)) => members.push(ix),
            None => groups.push((repo.owner.clone(), vec![ix])),
        }
    }
    for (_, members) in &mut groups {
        members.sort_by_key(|ix| {
            repos
                .get(*ix)
                .map(|repo| repo.name.to_lowercase())
                .unwrap_or_default()
        });
    }
    groups.sort_by_key(|(owner, _)| (!owner.eq_ignore_ascii_case(login), owner.to_lowercase()));
    groups
}

/// A repository as it is shown: its texts tidied.
fn tidy(mut repo: HostedRepo) -> HostedRepo {
    repo.owner = providers::shown(&repo.owner);
    repo.name = providers::shown(&repo.name);
    repo.full_name = providers::shown(&repo.full_name);
    repo.description = repo
        .description
        .as_deref()
        .map(providers::shown)
        .filter(|text| !text.is_empty());
    repo
}

/// The tab of one account.
pub struct ProviderCloneView {
    account: Account,
    folder: Entity<InputState>,
    search: Entity<InputState>,
    filter: Filter,
    /// Clone over SSH rather than HTTPS.
    ssh: bool,
    repos: Vec<HostedRepo>,
    /// The provider has more than it sent.
    more: bool,
    read: Read,
    /// Which asking is the latest.
    asked: u64,
    /// The query the list was last asked with.
    asked_query: String,
    picked: Option<String>,
    scroll: ScrollHandle,
    /// The pause after typing, before the provider is asked.
    pausing: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ProviderCloneEvent> for ProviderCloneView {}

impl ProviderCloneView {
    /// The tab of an account, cloning into `parent` unless another folder
    /// is chosen. The account's own repositories are asked for when the
    /// tab is first shown.
    pub fn new(
        account: Account,
        parent: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let folder = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(say!("provider-clone-folder-placeholder"))
                .default_value(parent.display().to_string())
        });
        let search = cx.new(|cx| {
            InputState::new(window, cx).placeholder(say!("provider-clone-search-placeholder"))
        });
        let subscriptions = vec![
            cx.subscribe_in(
                &search,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.typed(window, cx);
                    }
                },
            ),
            cx.subscribe(&folder, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        Self {
            account,
            folder,
            search,
            filter: Filter::Own,
            ssh: true,
            repos: Vec::new(),
            more: false,
            read: Read::NotAsked,
            asked: 0,
            asked_query: String::new(),
            picked: None,
            scroll: ScrollHandle::new(),
            pausing: None,
            _subscriptions: subscriptions,
        }
    }

    /// Ask the provider for the list under the filter in force.
    fn load(&mut self, query: String, cx: &mut Context<Self>) {
        self.asked = self.asked.wrapping_add(1);
        let asked = self.asked;
        self.read = Read::Loading;
        self.asked_query = query.clone();
        let mut params = json!({ "account": self.account.id, "filter": self.filter.word() });
        if !query.is_empty()
            && let Some(params) = params.as_object_mut()
        {
            params.insert("query".to_owned(), query.into());
        }
        let answer = providers::ask::<Listing<HostedRepo>>("provider.repos", params);
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                if this.asked == asked {
                    this.take(answer, cx);
                }
            });
        })
        .detach();
        cx.notify();
    }

    /// Take what `provider.repos` answered.
    fn take(&mut self, answer: Result<Listing<HostedRepo>, String>, cx: &mut Context<Self>) {
        match answer {
            Ok(listing) => {
                self.repos = listing.items.into_iter().take(MOST).map(tidy).collect();
                self.more = listing.more;
                self.read = Read::Loaded;
                if self
                    .picked
                    .as_ref()
                    .is_some_and(|picked| !self.repos.iter().any(|repo| repo.full_name == *picked))
                {
                    self.picked = None;
                }
            }
            Err(problem) => self.read = Read::Failed(problem),
        }
        cx.notify();
    }

    /// Show this list as if the provider had answered it: for the tests
    /// and the pictures.
    pub fn show_listing(&mut self, listing: Listing<HostedRepo>, cx: &mut Context<Self>) {
        self.asked = self.asked.wrapping_add(1);
        self.take(Ok(listing), cx);
    }

    /// Show what went wrong reading the list: for the tests.
    pub fn show_problem(&mut self, problem: String, cx: &mut Context<Self>) {
        self.asked = self.asked.wrapping_add(1);
        self.take(Err(problem), cx);
    }

    /// List another kind of repositories.
    pub fn set_filter(&mut self, filter: Filter, cx: &mut Context<Self>) {
        if self.filter != filter {
            self.filter = filter;
            self.picked = None;
            let query = self.query(cx);
            self.load(if self.more { query } else { String::new() }, cx);
        }
    }

    /// Clone over SSH, or over HTTPS.
    pub fn set_ssh(&mut self, ssh: bool, cx: &mut Context<Self>) {
        self.ssh = ssh;
        cx.notify();
    }

    /// Pick a repository by its full name.
    pub fn pick(&mut self, full_name: &str, cx: &mut Context<Self>) {
        if self.repos.iter().any(|repo| repo.full_name == full_name) {
            self.picked = Some(full_name.to_owned());
            cx.notify();
        }
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_owned()
    }

    /// What is typed narrows the list now; when the provider has more, it
    /// is asked once the typing pauses.
    fn typed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        cx.notify();
        if !self.more && self.asked_query.is_empty() {
            self.pausing = None;
            return;
        }
        self.pausing = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(PAUSE).await;
            let _ = this.update(cx, |this, cx| {
                let query = this.query(cx);
                if query != this.asked_query {
                    this.load(query, cx);
                }
            });
        }));
    }

    /// The repository picked, when it is among those shown.
    fn picked_repo(&self) -> Option<&HostedRepo> {
        let picked = self.picked.as_ref()?;
        self.repos.iter().find(|repo| repo.full_name == *picked)
    }

    /// The address the clone is from, and where it goes, when both will do.
    fn target(&self, cx: &App) -> (Option<String>, Result<PathBuf, String>) {
        let Some(repo) = self.picked_repo() else {
            return (None, Err(say!("provider-clone-pick").to_owned()));
        };
        let url = if self.ssh {
            repo.clone_ssh.clone()
        } else {
            repo.clone_https.clone()
        };
        let typed = self.folder.read(cx).value().trim().to_owned();
        let folder = crate::shell::expand_home(&typed);
        let name = repo.name.trim();
        let path = if typed.is_empty() || !folder.is_absolute() {
            Err(say!("provider-clone-folder-not-full").to_owned())
        } else if name.is_empty() || name.contains(['/', '\\']) || name == "." || name == ".." {
            Err(say!("provider-clone-bad-name").to_owned())
        } else {
            let path = folder.join(name);
            let taken = std::fs::read_dir(&path)
                .map(|mut entries| entries.next().is_some())
                .unwrap_or_else(|_| path.exists());
            if taken {
                Err(say!(
                    "provider-clone-taken",
                    path = path.display().to_string()
                ))
            } else {
                Ok(path)
            }
        };
        let url = providers::usable_address(&url).then(|| url.trim().to_owned());
        (url, path)
    }

    /// Clone what is picked.
    pub fn go(&mut self, cx: &mut Context<Self>) {
        if let (Some(url), Ok(path)) = self.target(cx) {
            cx.emit(ProviderCloneEvent::Clone { url, path });
        }
    }

    /// Choose the folder in the desktop's chooser.
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(say!("provider-clone-browse-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.folder.update(cx, |input, cx| {
                        input.set_value(path.display().to_string(), window, cx)
                    });
                });
            }
        })
        .detach();
    }

    /// What the tab has on show.
    pub fn probe(&self, cx: &App) -> ProviderCloneProbe {
        let groups = grouped(&self.repos, &self.query(cx), &self.account.login)
            .into_iter()
            .map(|(owner, members)| {
                let names = members
                    .iter()
                    .filter_map(|ix| self.repos.get(*ix).map(|repo| repo.name.clone()))
                    .collect();
                (owner, names)
            })
            .collect();
        let (url, path) = self.target(cx);
        ProviderCloneProbe {
            filter: self.filter.word(),
            groups,
            picked: self.picked.clone(),
            url,
            path: path.ok(),
            loading: matches!(self.read, Read::Loading | Read::NotAsked),
            problem: match &self.read {
                Read::Failed(problem) => Some(problem.clone()),
                _ => None,
            },
        }
    }
}

/// A row with its word in the column at the left.
fn labelled(label: &'static str, muted: Hsla, field: impl IntoElement) -> Div {
    h_flex()
        .w_full()
        .min_h(px(ROW_HEIGHT))
        .gap(px(LABEL_GAP))
        .items_center()
        .child(
            div()
                .w(px(LABELS))
                .flex_none()
                .text_right()
                .text_sm()
                .text_color(muted)
                .child(label),
        )
        .child(div().flex_1().min_w_0().child(field))
}

impl Render for ProviderCloneView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.read == Read::NotAsked {
            self.load(String::new(), cx);
        }
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let site = Site::of(&self.account);
        let query = self.query(cx);
        let groups = grouped(&self.repos, &query, &self.account.login);
        let (url, path) = self.target(cx);
        let ready = url.is_some() && path.is_ok();
        let filters = h_flex().gap(px(6.)).children(Filter::ALL.map(|filter| {
            crate::views::chosen(
                Button::new(SharedString::from(format!(
                    "clone-filter-{}",
                    filter.word()
                )))
                .outline()
                .small()
                .label(filter.label()),
                filter == self.filter,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter, cx)))
        }));
        let protocol = h_flex().gap(px(6.)).children([true, false].map(|ssh| {
            crate::views::chosen(
                Button::new(if ssh {
                    "clone-over-ssh"
                } else {
                    "clone-over-https"
                })
                .outline()
                .small()
                .label(if ssh { "SSH" } else { "HTTPS" }),
                ssh == self.ssh,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_ssh(ssh, cx)))
        }));
        let mut list = v_flex().w_full().pb(px(8.));
        for (owner, members) in &groups {
            // The owner in small capitals over its repositories.
            list = list.child(
                div()
                    .h(px(OWNER_HEIGHT))
                    .flex_none()
                    .px(px(12.))
                    .pt(px(8.))
                    .text_xs()
                    .text_color(muted)
                    .truncate()
                    .child(owner.to_uppercase()),
            );
            for ix in members {
                let Some(repo) = self.repos.get(*ix) else {
                    continue;
                };
                let picked = self.picked.as_deref() == Some(repo.full_name.as_str());
                let full_name = repo.full_name.clone();
                list = list.child(
                    h_flex()
                        .id(SharedString::from(format!("clone-repo-{}", repo.full_name)))
                        .h(px(REPO_HEIGHT))
                        .flex_none()
                        .px(px(12.))
                        .gap(px(8.))
                        .items_center()
                        .border_b_1()
                        .border_color(theme.border)
                        .cursor_pointer()
                        .text_sm()
                        .when(picked, |row| row.bg(theme.list_active))
                        .when(!picked, |row| row.hover(|row| row.bg(theme.list_hover)))
                        // A fork's mark for a fork, a lock for a private
                        // one, a book for the others.
                        .child(
                            Icon::new(if repo.parent.is_some() {
                                gpui_kit::assets::IconName::GitFork
                            } else if repo.private {
                                gpui_kit::assets::IconName::Lock
                            } else {
                                gpui_kit::assets::IconName::Book
                            })
                            .size(px(14.))
                            .flex_none()
                            .text_color(muted),
                        )
                        .child(
                            div()
                                .flex_none()
                                .max_w(px(260.))
                                .truncate()
                                .child(repo.name.clone()),
                        )
                        .children(repo.description.clone().map(|text| {
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(muted)
                                .child(text)
                        }))
                        .on_click(cx.listener(move |this, event: &ClickEvent, _, cx| {
                            this.pick(&full_name, cx);
                            if event.click_count() >= 2 {
                                this.go(cx);
                            }
                        }))
                        .test_support(),
                );
            }
        }
        let status: Option<AnyElement> = match &self.read {
            Read::Loading | Read::NotAsked => Some(
                h_flex()
                    .gap(px(8.))
                    .items_center()
                    .text_sm()
                    .text_color(muted)
                    .child(Spinner::new().xsmall())
                    .child(say!("provider-clone-asking", site = site.name()))
                    .into_any_element(),
            ),
            Read::Failed(problem) => Some(
                div()
                    .id("clone-problem")
                    .text_sm()
                    .whitespace_normal()
                    .text_color(theme.danger)
                    .child(say!("provider-clone-unread", problem = problem.as_str()))
                    .test_support()
                    .into_any_element(),
            ),
            Read::Loaded if groups.is_empty() => Some(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(if query.is_empty() {
                        say!("provider-clone-none")
                    } else {
                        say!("provider-clone-no-match")
                    })
                    .into_any_element(),
            ),
            Read::Loaded if self.more => Some(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(say!("provider-clone-more", site = site.name()))
                    .into_any_element(),
            ),
            Read::Loaded => None,
        };
        let said = match (&url, &path) {
            (Some(url), Ok(path)) => say!(
                "provider-clone-over",
                protocol = if self.ssh { "SSH" } else { "HTTPS" },
                url,
                path = path.display().to_string()
            ),
            (_, Err(why)) => why.clone(),
            (None, Ok(_)) => say!("provider-clone-no-address").to_owned(),
        };
        v_flex()
            .id("clone-provider")
            .size_full()
            .pl(px(PANE_LEFT))
            .pr(px(PANE_RIGHT))
            .pt(px(PANE_TOP))
            .pb(px(PANE_TOP))
            .gap(px(ROW_GAP))
            .child(
                h_flex()
                    .flex_none()
                    .h(px(HEADING))
                    .mb(px(1.))
                    .gap(px(8.))
                    .items_center()
                    .text_lg()
                    .child(site.icon().size(px(18.)))
                    .child(say!(
                        "provider-clone-heading",
                        login = self.account.login.as_str(),
                        site = match site {
                            Site::Github | Site::Gitlab => site.name().to_owned(),
                            _ => providers::host_of(&self.account.web).to_owned(),
                        }
                    )),
            )
            .child(labelled(
                say!("provider-clone-into-folder"),
                muted,
                h_flex()
                    .gap(px(11.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.folder).id("clone-folder")),
                    )
                    .child(
                        Button::new("clone-browse")
                            .outline()
                            .label(say!("provider-clone-browse"))
                            .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx))),
                    ),
            ))
            .child(labelled(say!("provider-clone-show"), muted, filters))
            .child(labelled(
                say!("provider-clone-repository"),
                muted,
                Input::new(&self.search)
                    .id("clone-search")
                    .prefix(Icon::new(IconName::Search).size(px(14.)).text_color(muted))
                    .suffix(
                        Icon::new(IconName::ChevronDown)
                            .size(px(14.))
                            .text_color(muted),
                    ),
            ))
            .child(
                h_flex()
                    .flex_1()
                    .min_h(px(120.))
                    .w_full()
                    .items_stretch()
                    .mt(px(-ROW_GAP + 2.))
                    .child(div().w(px(LABELS + LABEL_GAP)).flex_none())
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_w_0()
                            .min_h_0()
                            .rounded(theme.radius)
                            .border_1()
                            .border_color(theme.border)
                            .bg(theme.popover)
                            .child(
                                v_flex()
                                    .id("clone-list")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.scroll)
                                    .children(status.map(|status| div().p(px(10.)).child(status)))
                                    .child(list),
                            )
                            .scrollbar(&self.scroll, ScrollbarAxis::Vertical),
                    ),
            )
            .child(labelled(say!("provider-clone-clone-over"), muted, protocol))
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(11.))
                    .items_center()
                    .child(
                        div()
                            .id("clone-said")
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(if ready {
                                muted
                            } else {
                                theme.danger.opacity(0.9)
                            })
                            .when(self.picked.is_none(), |line| line.text_color(muted))
                            .child(said)
                            .test_support(),
                    )
                    .child(
                        Button::new("clone-go")
                            .primary()
                            .h(px(31.))
                            .label(say!("provider-clone-clone"))
                            .disabled(!ready)
                            .on_click(cx.listener(|this, _, _, cx| this.go(cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_integrations::model::HostedRepo;

    use super::grouped;

    fn repo(owner: &str, name: &str, description: Option<&str>) -> HostedRepo {
        HostedRepo {
            id: format!("{owner}/{name}"),
            owner: owner.to_owned(),
            name: name.to_owned(),
            full_name: format!("{owner}/{name}"),
            description: description.map(str::to_owned),
            private: false,
            parent: None,
            archived: false,
            default_branch: Some("main".to_owned()),
            clone_https: format!("https://example.org/{owner}/{name}.git"),
            clone_ssh: format!("git@example.org:{owner}/{name}.git"),
            web_url: format!("https://example.org/{owner}/{name}"),
            updated: None,
            stars: 0,
        }
    }

    #[test]
    fn repositories_are_grouped_by_owner_the_accounts_own_first() {
        let repos = vec![
            repo("zeta-org", "tools", None),
            repo("ada", "notes", Some("Lecture notes")),
            repo("babbage-org", "engine", Some("The difference engine")),
            repo("ada", "analytical", None),
            repo("babbage-org", "Cards", None),
        ];
        let groups = grouped(&repos, "", "Ada");
        let names: Vec<(&str, Vec<&str>)> = groups
            .iter()
            .map(|(owner, members)| {
                (
                    owner.as_str(),
                    members.iter().map(|ix| repos[*ix].name.as_str()).collect(),
                )
            })
            .collect();
        assert_eq!(
            names,
            [
                ("ada", vec!["analytical", "notes"]),
                ("babbage-org", vec!["Cards", "engine"]),
                ("zeta-org", vec!["tools"]),
            ]
        );
        // Every word, in the name or the description, whatever the case.
        let found = grouped(&repos, "ENGINE difference", "ada");
        assert_eq!(found, [("babbage-org".to_owned(), vec![2])]);
        let found = grouped(&repos, "ada/", "ada");
        assert_eq!(found.len(), 1);
        assert!(grouped(&repos, "nothing like it", "ada").is_empty());
    }
}
