//! The SSH key on a connected provider's page: the key the current
//! profile connects with, its fingerprint and a button that adds it to the
//! provider; a title and a button that makes a new key and adds that; and
//! buttons that choose another private key (the profile's from then on)
//! or another public key to add.
//!
//! The key in use is the profile's key file; for a profile that uses the
//! SSH agent's keys it is the first of ssh's own default files that is
//! there. What is added is the public half: the file beside the private
//! one with `.pub` added, unless another was chosen. A new key goes to
//! `~/.ssh/id_ed25519_bitbraid` (or with a number after it when that is
//! taken), made by `ssh.key_generate`, which never writes over a file, and
//! becomes the profile's key once it is made, whether or not the provider
//! takes it.
//!
//! Sources: ssh(1) (the default identity files); docs.github.com "Adding a
//! new SSH key to your GitHub account"; docs.gitlab.com "Use SSH keys to
//! communicate with GitLab"; gpui-kit documentation (input, button).

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::{IntegrationsEvent, IntegrationsView, layout};
use crate::{
    providers::{self, Site},
    zoom::px,
};

/// ssh's own key files, in the order it tries them.
const DEFAULT_KEYS: [&str; 3] = ["~/.ssh/id_ed25519", "~/.ssh/id_ecdsa", "~/.ssh/id_rsa"];
/// Where a new key goes, before a number is put after it.
const NEW_KEY: &str = "~/.ssh/id_ed25519_bitbraid";
/// What a key is called at the provider unless something else is typed.
const DEFAULT_TITLE: &str = "BitBraid";

/// What the rows keep.
pub(super) struct Keys {
    /// The private key file the current profile connects with; none for
    /// the SSH agent's keys.
    pub profile_key: Option<String>,
    /// A public key file chosen to be added instead of the one beside the
    /// private key.
    pub public: Option<String>,
    /// What ssh-keygen said of the key in use: for which file, and its
    /// fingerprint or why there is none.
    print: Option<(String, Result<String, String>)>,
    /// The file whose fingerprint is being asked for.
    asking: Option<String>,
    /// What the key is called at the provider.
    title: Entity<InputState>,
    /// A key is being made or added.
    busy: bool,
    /// What came of it.
    said: Option<(bool, String)>,
}

impl Keys {
    pub(super) fn new(window: &mut Window, cx: &mut Context<IntegrationsView>) -> Self {
        let title = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(DEFAULT_TITLE)
                .default_value(DEFAULT_TITLE)
        });
        Self {
            profile_key: None,
            public: None,
            print: None,
            asking: None,
            title,
            busy: false,
            said: None,
        }
    }

    /// The key in use and its fingerprint, for the tests.
    pub(super) fn probe(&self) -> Option<(String, Option<String>)> {
        self.print
            .as_ref()
            .map(|(path, print)| (path.clone(), print.as_ref().ok().cloned()))
    }
}

/// A path as it was typed, with `~` for the home folder.
fn expand(path: &str) -> PathBuf {
    crate::shell::expand_home(path)
}

/// The public key file that goes with a private one.
fn public_of(private: &str) -> String {
    format!("{private}.pub")
}

/// Where a new key can go: the usual name, or that with the first number
/// after it that is free.
pub(super) fn free_key_path() -> Option<PathBuf> {
    (1..100).find_map(|number| {
        let path = if number == 1 {
            expand(NEW_KEY)
        } else {
            expand(&format!("{NEW_KEY}_{number}"))
        };
        let public = PathBuf::from(public_of(&path.to_string_lossy()));
        let free = std::fs::symlink_metadata(&path).is_err()
            && std::fs::symlink_metadata(&public).is_err();
        free.then_some(path)
    })
}

impl IntegrationsView {
    /// The public key file of the key in use, and where it comes from.
    fn key_in_use(&self) -> Option<(String, &'static str)> {
        if let Some(public) = &self.keys.public {
            return Some((public.clone(), "chosen"));
        }
        if let Some(private) = &self.keys.profile_key {
            return Some((public_of(private), "profile"));
        }
        DEFAULT_KEYS.iter().find_map(|key| {
            let public = public_of(&expand(key).to_string_lossy());
            std::path::Path::new(&public)
                .is_file()
                .then_some((public, "ssh"))
        })
    }

    /// The account a key goes to: the site's default one.
    fn key_account(&self, cx: &App) -> Option<bitbraid_integrations::model::Account> {
        let accounts = providers::accounts_at(self.chosen, cx);
        accounts
            .iter()
            .find(|account| account.default)
            .or(accounts.first())
            .cloned()
    }

    /// Ask for the fingerprint of the key in use when it is not known.
    pub(super) fn look_at_key(&mut self, cx: &mut Context<Self>) {
        if self.key_account(cx).is_none() {
            return;
        }
        let Some((public, _)) = self.key_in_use() else {
            self.keys.print = None;
            return;
        };
        let known = self
            .keys
            .print
            .as_ref()
            .is_some_and(|(path, _)| *path == public);
        if known || self.keys.asking.as_deref() == Some(public.as_str()) {
            return;
        }
        self.keys.asking = Some(public.clone());
        let answer =
            providers::ask::<serde_json::Value>("ssh.key_fingerprint", json!({ "path": public }));
        cx.spawn(async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                if this.keys.asking.as_deref() != Some(public.as_str()) {
                    return;
                }
                this.keys.asking = None;
                let print = answer.and_then(|value| {
                    value
                        .get("fingerprint")
                        .and_then(|print| print.as_str())
                        .map(providers::shown)
                        .ok_or_else(|| say!("integrations-ssh-no-fingerprint").to_owned())
                });
                this.keys.print = Some((public, print));
                cx.notify();
            });
        })
        .detach();
    }

    /// Show a private key as the profile's, with this fingerprint,
    /// without asking anybody: for the pictures.
    pub fn show_key(
        &mut self,
        private: String,
        fingerprint: Result<String, String>,
        cx: &mut Context<Self>,
    ) {
        self.keys.public = None;
        self.keys.asking = None;
        self.keys.print = Some((public_of(&private), fingerprint));
        self.keys.profile_key = Some(private);
        cx.notify();
    }

    /// What was said of the last key made or added, for the tests.
    pub fn key_said(&self, _: &App) -> Option<String> {
        self.keys.said.as_ref().map(|(_, text)| text.clone())
    }

    /// Say what came of adding or making a key, and show the page's end
    /// where the key rows are: for the pictures.
    pub fn show_key_said(&mut self, worked: bool, text: String, cx: &mut Context<Self>) {
        self.keys.said = Some((worked, text));
        self.page_scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Add the key in use to the provider.
    fn add_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(account), Some((public, _))) = (self.key_account(cx), self.key_in_use()) else {
            return;
        };
        let title = self.key_title(cx);
        self.keys.busy = true;
        self.keys.said = None;
        let answer = providers::ask::<serde_json::Value>(
            "provider.ssh_key_add",
            json!({ "account": account.id, "path": public, "title": title }),
        );
        let site = self.chosen;
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update(cx, |this, cx| {
                this.keys.busy = false;
                this.keys.said = Some(match answer {
                    Ok(_) => (
                        true,
                        say!(
                            "integrations-ssh-added",
                            site = site.name(),
                            title = title.as_str()
                        ),
                    ),
                    Err(problem) => (false, problem),
                });
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// What the key is called at the provider.
    fn key_title(&self, cx: &App) -> String {
        let typed = providers::shown(&self.keys.title.read(cx).value());
        if typed.is_empty() {
            DEFAULT_TITLE.to_owned()
        } else {
            typed
        }
    }

    /// Make a new key, add it to the provider, and connect with it from
    /// then on.
    fn generate_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = self.key_account(cx) else {
            return;
        };
        let Some(path) = free_key_path() else {
            self.keys.said = Some((false, say!("integrations-ssh-no-free-name").to_owned()));
            cx.notify();
            return;
        };
        let title = self.key_title(cx);
        self.keys.busy = true;
        self.keys.said = None;
        let made = providers::ask::<serde_json::Value>(
            "ssh.key_generate",
            json!({ "path": path, "comment": title }),
        );
        let site = self.chosen;
        cx.spawn_in(window, async move |this, cx| {
            let made = made.await;
            let public = made.as_ref().ok().and_then(|value| {
                value
                    .get("public")
                    .and_then(|public| public.as_str())
                    .map(str::to_owned)
            });
            let added = match (&made, &public) {
                (Ok(_), Some(public)) => {
                    providers::ask::<serde_json::Value>(
                        "provider.ssh_key_add",
                        json!({ "account": account.id, "path": public, "title": title }),
                    )
                    .await
                }
                (Err(problem), _) => Err(problem.clone()),
                (Ok(_), None) => Err(say!("integrations-ssh-no-public").to_owned()),
            };
            let private = path.to_string_lossy().into_owned();
            let _ = this.update(cx, |this, cx| {
                this.keys.busy = false;
                this.keys.said = Some(match (&made, added) {
                    (Ok(_), Ok(_)) => (
                        true,
                        say!(
                            "integrations-ssh-generated",
                            private = private.as_str(),
                            site = site.name()
                        ),
                    ),
                    (Ok(_), Err(problem)) => (
                        false,
                        say!(
                            "integrations-ssh-generated-not-added",
                            private = private.as_str(),
                            problem
                        ),
                    ),
                    (Err(_), Err(problem)) => (false, problem),
                    (Err(problem), Ok(_)) => (false, problem.clone()),
                });
                if made.is_ok() {
                    this.keys.public = None;
                    cx.emit(IntegrationsEvent::UseKey(private));
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Choose a key file in the desktop's chooser: a private one becomes
    /// the profile's, a public one is what is added.
    fn choose_key(&mut self, public: bool, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(
                if public {
                    say!("integrations-ssh-choose-public")
                } else {
                    say!("integrations-ssh-choose-private")
                }
                .into(),
            ),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
                && let Some(path) = path.to_str().map(str::to_owned)
            {
                let _ = this.update(cx, |this, cx| {
                    if public {
                        this.keys.public = Some(path);
                    } else {
                        this.keys.public = None;
                        cx.emit(IntegrationsEvent::UseKey(path));
                    }
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The SSH key rows, under a provider with an account: rows of the
    /// provider's form with no heading of their own; the key in use; a
    /// title for a new key, with the button that makes and adds one under
    /// it; the buttons that choose a private and a public key, one under
    /// the other.
    pub(super) fn ssh_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(account) = self.key_account(cx) else {
            return Vec::new();
        };
        let theme = cx.theme().clone();
        let (muted, danger, success) = (theme.muted_foreground, theme.danger, theme.success);
        let site = Site::of(&account);
        let in_use = self.key_in_use();
        let print = self.keys.print.clone();
        let busy = self.keys.busy;
        let view = self.handle();
        let (for_add, for_generate, for_private, for_public) =
            (view.clone(), view.clone(), view.clone(), view);
        let to = providers::account_label(&account);
        let provider = match site.kind() {
            bitbraid_integrations::model::ProviderKind::Github => "GitHub",
            bitbraid_integrations::model::ProviderKind::Gitlab => "GitLab",
        };
        let (shown, whence) = match &in_use {
            Some((path, whence)) => (path.clone(), *whence),
            None => (String::new(), "none"),
        };
        let fingerprint = print
            .as_ref()
            .filter(|(path, _)| *path == shown)
            .map(|(_, print)| print.clone());
        // What there is to say of the key in use stands under the pointer
        // on it, so that the rows keep their places.
        let about: Vec<String> = [
            match &fingerprint {
                Some(Ok(print)) => print.clone(),
                Some(Err(why)) => why.clone(),
                None if shown.is_empty() => say!("integrations-ssh-none-here").to_owned(),
                None => String::new(),
            },
            match whence {
                "profile" => say!("integrations-ssh-from-profile"),
                "chosen" => say!("integrations-ssh-from-chosen"),
                "ssh" => say!("integrations-ssh-from-ssh"),
                _ => "",
            }
            .to_owned(),
            if shown.is_empty() {
                String::new()
            } else {
                say!("integrations-ssh-added-to", to = to.as_str())
            },
        ]
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect();
        let about = about.join("\n");
        let failed = matches!(fingerprint, Some(Err(_)));
        let mut rows = vec![layout::row(
            say!("integrations-ssh-key"),
            h_flex()
                .w_full()
                .gap(px(8.))
                .items_center()
                .child(
                    div()
                        .id("integrations-ssh-key")
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .when(failed, |text| text.text_color(danger))
                        .child(if shown.is_empty() {
                            say!("integrations-ssh-no-key").to_owned()
                        } else {
                            shown.clone()
                        })
                        .when(!about.is_empty(), |text| {
                            text.tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(about.clone())
                                    .build(window, cx)
                            })
                        })
                        .test_support(),
                )
                .child(
                    Button::new("integrations-ssh-add")
                        .outline()
                        .small()
                        .h(px(layout::BUTTON_HEIGHT))
                        .label(say!("integrations-ssh-add", provider))
                        .disabled(busy || shown.is_empty())
                        .on_click(move |_, window, cx| {
                            let _ = for_add.update(cx, |this, cx| this.add_key(window, cx));
                        }),
                )
                .into_any_element(),
            true,
            None,
            muted,
        )];
        rows.push(layout::row(
            say!("integrations-ssh-new"),
            Input::new(&self.keys.title)
                .id("integrations-ssh-title")
                .min_h(px(layout::FIELD_HEIGHT))
                .w_full()
                .into_any_element(),
            true,
            None,
            muted,
        ));
        rows.push(layout::row(
            "",
            Button::new("integrations-ssh-generate")
                .primary()
                .small()
                .h(px(layout::BUTTON_HEIGHT))
                .icon(IconName::Plus)
                .label(say!("integrations-ssh-generate", provider))
                .tooltip(say!(
                    "integrations-ssh-generate-tooltip",
                    provider,
                    to = to.as_str()
                ))
                .loading(busy)
                .disabled(busy)
                .on_click(move |_, window, cx| {
                    let _ = for_generate.update(cx, |this, cx| this.generate_key(window, cx));
                })
                .into_any_element(),
            false,
            None,
            muted,
        ));
        rows.push(layout::row(
            say!("integrations-ssh-others"),
            Button::new("integrations-ssh-private")
                .outline()
                .small()
                .h(px(layout::BUTTON_HEIGHT))
                .label(say!("integrations-ssh-private"))
                .tooltip(say!("integrations-ssh-private-tooltip"))
                .on_click(move |_, window, cx| {
                    let _ = for_private.update(cx, |this, cx| this.choose_key(false, window, cx));
                })
                .into_any_element(),
            false,
            None,
            muted,
        ));
        rows.push(layout::row(
            "",
            Button::new("integrations-ssh-public")
                .outline()
                .small()
                .h(px(layout::BUTTON_HEIGHT))
                .label(say!("integrations-ssh-public"))
                .tooltip(say!("integrations-ssh-public-tooltip"))
                .on_click(move |_, window, cx| {
                    let _ = for_public.update(cx, |this, cx| this.choose_key(true, window, cx));
                })
                .into_any_element(),
            false,
            None,
            muted,
        ));
        if let Some((worked, text)) = self.keys.said.clone() {
            rows.push(layout::row(
                "",
                div()
                    .id("integrations-ssh-said")
                    .text_sm()
                    .text_color(if worked { success } else { danger })
                    .child(text)
                    .test_support()
                    .into_any_element(),
                true,
                None,
                muted,
            ));
        }
        rows
    }
}
