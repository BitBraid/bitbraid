//! Making a repository at a provider: an account's place in the dialog that
//! makes a repository, beside "On this machine".
//!
//! Where it is made (the user's own, or an organisation or group the
//! account may make one in, from `provider.namespaces`), its name, what it
//! says it is, whether it is private, and the folder it is made in on this
//! machine. Going ahead makes it here first, with a first commit, opens
//! it, and then has `provider.repo_create` make it at the provider, add it
//! as the remote `origin` and push the first commit there.
//!
//! Sources: docs.github.com "Create a repository for the authenticated
//! user" and "Create an organization repository" (name, description,
//! private); docs.gitlab.com "Projects API" (create a project: `name`,
//! `path`, `namespace_id`, `visibility`).

use std::path::PathBuf;

use bitbraid_integrations::model::{Account, Namespace, NamespaceKind};
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::*;
use serde_json::json;

use crate::{
    providers::{self, Site},
    zoom::px,
};

/// The column of words before the fields, as the form has it.
const LABELS: f32 = crate::views::workspace::form::LABELS;
const LABEL_GAP: f32 = 15.;
const ROW_HEIGHT: f32 = 34.;
const ROW_GAP: f32 = 10.;
const PANE_LEFT: f32 = 11.;
const PANE_RIGHT: f32 = 20.;
const PANE_TOP: f32 = 10.;
const HEADING: f32 = 24.;
/// The longest name a repository is given.
const LONGEST_NAME: usize = 100;
/// The most places listed.
const MOST_NAMESPACES: usize = 200;

/// What the place asks of the window: make this repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Creation {
    /// The account it is made at.
    pub account: String,
    /// The organisation or group, `None` for the user's own.
    pub namespace: Option<String>,
    pub name: String,
    /// What it says it is.
    pub description: Option<String>,
    /// Whether only some may see it.
    pub private: bool,
    /// Where it is made on this machine.
    pub path: PathBuf,
}

/// What the place has on show, for the tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCreateProbe {
    /// The places it may be made in, by path.
    pub namespaces: Vec<String>,
    /// The one chosen; none for the user's own.
    pub namespace: Option<String>,
    /// Whether it would be private.
    pub private: bool,
    /// What it would be, when that will do; else why not.
    pub creation: Result<Creation, String>,
}

/// Why a name cannot be a repository's, if it cannot.
pub fn name_refusal(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some(say!("provider-create-name-empty"));
    }
    if name.len() > LONGEST_NAME {
        return Some(say!("provider-create-name-long"));
    }
    if name.starts_with('.') || name.ends_with(".git") {
        return Some(say!("provider-create-name-dot"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return Some(say!("provider-create-name-letters"));
    }
    None
}

/// The place of one account.
pub struct ProviderCreateView {
    account: Account,
    namespaces: Vec<Namespace>,
    /// What went wrong reading them.
    namespaces_problem: Option<String>,
    /// The places were asked for.
    namespaces_asked: bool,
    namespace: Option<String>,
    name: Entity<InputState>,
    description: Entity<InputState>,
    folder: Entity<InputState>,
    private: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Creation> for ProviderCreateView {}

impl ProviderCreateView {
    /// The place of an account, making the folder in `parent` unless
    /// another is typed. The places it may make one in are asked for at
    /// first shown.
    pub fn new(
        account: Account,
        parent: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx).placeholder(say!("provider-create-name-placeholder"))
        });
        let description = cx.new(|cx| {
            InputState::new(window, cx).placeholder(say!("provider-create-description-placeholder"))
        });
        let folder = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(say!("provider-create-folder-placeholder"))
                .default_value(parent.display().to_string())
        });
        let subscriptions = [&name, &description, &folder]
            .into_iter()
            .map(|field| {
                cx.subscribe(field, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .collect();
        Self {
            account,
            namespaces: Vec::new(),
            namespaces_problem: None,
            namespaces_asked: false,
            namespace: None,
            name,
            description,
            folder,
            private: true,
            _subscriptions: subscriptions,
        }
    }

    /// Ask for the places it may be made in, the first time it is shown.
    fn ask_namespaces(&mut self, cx: &mut Context<Self>) {
        if self.namespaces_asked {
            return;
        }
        self.namespaces_asked = true;
        let answer = providers::ask::<Vec<Namespace>>(
            "provider.namespaces",
            json!({ "account": self.account.id }),
        );
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| this.take_namespaces(answer, cx));
        })
        .detach();
    }

    /// Take what `provider.namespaces` answered.
    pub fn take_namespaces(
        &mut self,
        answer: Result<Vec<Namespace>, String>,
        cx: &mut Context<Self>,
    ) {
        self.namespaces_asked = true;
        match answer {
            Ok(list) => {
                self.namespaces = list
                    .into_iter()
                    .take(MOST_NAMESPACES)
                    .map(|mut namespace| {
                        namespace.path = providers::shown(&namespace.path);
                        namespace.name = providers::shown(&namespace.name);
                        namespace
                    })
                    .filter(|namespace| !namespace.path.is_empty())
                    .collect();
                self.namespaces_problem = None;
            }
            Err(problem) => self.namespaces_problem = Some(problem),
        }
        cx.notify();
    }

    /// Make it in this organisation or group; none for the user's own.
    pub fn set_namespace(&mut self, namespace: Option<String>, cx: &mut Context<Self>) {
        self.namespace = namespace;
        cx.notify();
    }

    /// Make it private, or public.
    pub fn set_private(&mut self, private: bool, cx: &mut Context<Self>) {
        self.private = private;
        cx.notify();
    }

    /// Type the name, as the field would have it: for the tests and the
    /// pictures.
    pub fn set_name(
        &mut self,
        name: &str,
        description: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.name
            .update(cx, |input, cx| input.set_value(name.to_owned(), window, cx));
        self.description.update(cx, |input, cx| {
            input.set_value(description.to_owned(), window, cx)
        });
    }

    /// What would be made, or why nothing can be yet.
    fn creation(&self, cx: &App) -> Result<Creation, String> {
        let name = self.name.read(cx).value().trim().to_owned();
        if let Some(why) = name_refusal(&name) {
            return Err(why.to_owned());
        }
        let typed = self.folder.read(cx).value().trim().to_owned();
        let folder = crate::shell::expand_home(&typed);
        if typed.is_empty() || !folder.is_absolute() {
            return Err(say!("provider-create-folder-not-full").to_owned());
        }
        let path = folder.join(&name);
        if std::fs::symlink_metadata(&path).is_ok() {
            return Err(say!(
                "provider-create-taken",
                path = path.display().to_string()
            ));
        }
        let description = providers::shown(&self.description.read(cx).value());
        Ok(Creation {
            account: self.account.id.clone(),
            namespace: self.namespace.clone(),
            name,
            description: (!description.is_empty()).then_some(description),
            private: self.private,
            path,
        })
    }

    /// Make it.
    pub fn go(&mut self, cx: &mut Context<Self>) {
        if let Ok(creation) = self.creation(cx) {
            cx.emit(creation);
        }
    }

    /// What the place has on show.
    pub fn probe(&self, cx: &App) -> ProviderCreateProbe {
        ProviderCreateProbe {
            namespaces: self
                .namespaces
                .iter()
                .map(|namespace| namespace.path.clone())
                .collect(),
            namespace: self.namespace.clone(),
            private: self.private,
            creation: self.creation(cx),
        }
    }

    /// What the list of places says for the one chosen.
    fn namespace_label(&self) -> String {
        match &self.namespace {
            None => say!("provider-create-own", login = self.account.login.as_str()),
            Some(path) => self
                .namespaces
                .iter()
                .find(|namespace| namespace.path == *path)
                .map(|namespace| {
                    if namespace.name.is_empty() || namespace.name == namespace.path {
                        namespace.path.clone()
                    } else {
                        format!("{} ({})", namespace.path, namespace.name)
                    }
                })
                .unwrap_or_else(|| path.clone()),
        }
    }
}

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

impl Render for ProviderCreateView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let site = Site::of(&self.account);
        self.ask_namespaces(cx);
        let creation = self.creation(cx);
        let view = cx.entity().downgrade();
        // The user's own first, then the others the provider listed.
        let mut choices: Vec<(Option<String>, String)> = vec![(
            None,
            say!("provider-create-own", login = self.account.login.as_str()),
        )];
        choices.extend(
            self.namespaces
                .iter()
                .filter(|namespace| {
                    !(namespace.kind == NamespaceKind::User
                        && namespace.path.eq_ignore_ascii_case(&self.account.login))
                })
                .map(|namespace| (Some(namespace.path.clone()), namespace.path.clone())),
        );
        let chosen = self.namespace.clone();
        let namespaces = Button::new("create-namespace")
            .outline()
            .w_full()
            .h(px(ROW_HEIGHT))
            .dropdown_caret(true)
            .label(self.namespace_label())
            .dropdown_menu(move |menu, _, _| {
                choices.iter().fold(
                    menu.scrollable(true).max_h(px(320.)),
                    |menu, (path, label)| {
                        let (path, view) = (path.clone(), view.clone());
                        menu.item(
                            PopupMenuItem::new(label.clone())
                                .checked(chosen == path)
                                .on_click(move |_, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.set_namespace(path.clone(), cx)
                                    });
                                }),
                        )
                    },
                )
            });
        let visibility = h_flex().gap(px(6.)).children([true, false].map(|private| {
            crate::views::chosen(
                Button::new(if private {
                    "create-private"
                } else {
                    "create-public"
                })
                .outline()
                .small()
                .label(if private {
                    say!("provider-create-private")
                } else {
                    say!("provider-create-public")
                }),
                private == self.private,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_private(private, cx)))
        }));
        let full_path = match &creation {
            Ok(creation) => creation.path.display().to_string(),
            Err(_) => String::new(),
        };
        let said = match &creation {
            Ok(creation) => say!(
                "provider-create-said",
                path = creation.path.display().to_string(),
                site = site.name(),
                namespace = creation.namespace.as_deref().unwrap_or(&self.account.login),
                name = creation.name.as_str()
            ),
            Err(why) => why.clone(),
        };
        v_flex()
            .id("create-provider")
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
                        "provider-create-heading",
                        login = self.account.login.as_str(),
                        site = site.name()
                    )),
            )
            .child(labelled(say!("provider-create-make-in"), muted, namespaces))
            .children(self.namespaces_problem.clone().map(|problem| {
                h_flex()
                    .gap(px(LABEL_GAP))
                    .child(div().w(px(LABELS)).flex_none())
                    .child(
                        div()
                            .id("create-namespaces-problem")
                            .text_xs()
                            .whitespace_normal()
                            .text_color(muted)
                            .child(say!("provider-create-namespaces-unread", problem))
                            .test_support(),
                    )
            }))
            .child(labelled(
                say!("provider-create-name"),
                muted,
                Input::new(&self.name).id("create-name"),
            ))
            .child(labelled(
                say!("provider-create-description"),
                muted,
                Input::new(&self.description).id("create-description"),
            ))
            .child(labelled(
                say!("provider-create-visibility"),
                muted,
                visibility,
            ))
            .child(labelled(
                say!("provider-create-in-folder"),
                muted,
                Input::new(&self.folder).id("create-folder"),
            ))
            .child(labelled(
                say!("provider-create-full-path"),
                muted,
                div().text_sm().truncate().child(full_path),
            ))
            .child(
                h_flex()
                    .gap(px(LABEL_GAP))
                    .child(div().w(px(LABELS)).flex_none())
                    .child(
                        div()
                            .id("create-said")
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .whitespace_normal()
                            .text_color(if creation.is_ok() {
                                muted
                            } else {
                                theme.danger
                            })
                            .child(said)
                            .test_support(),
                    ),
            )
            .child(
                h_flex().justify_end().child(
                    Button::new("create-go")
                        .primary()
                        .h(px(31.))
                        .label(say!("provider-create-create"))
                        .disabled(creation.is_err())
                        .on_click(cx.listener(|this, _, _, cx| this.go(cx))),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::name_refusal;

    #[test]
    fn a_repositorys_name_is_a_plain_one() {
        for good in ["engine", "notes.v2", "the_engine-2"] {
            assert_eq!(name_refusal(good), None, "{good}");
        }
        for bad in [
            "",
            ".hidden",
            "engine.git",
            "a b",
            "a/b",
            "née",
            &"x".repeat(101),
        ] {
            assert!(name_refusal(bad).is_some(), "{bad:?}");
        }
    }
}
