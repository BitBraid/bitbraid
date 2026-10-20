//! Signing set-up: how this repository signs, whether that can work, the
//! keys that could be used, each with a way to choose it, and who may sign
//! with an SSH key: the allowed signers, with the file they are kept in.
//!
//! Sources: none (original design).

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::{
    session::{Reply, Request, SessionHandle},
    views::preferences::{
        page::{across, own_group},
        row::{BUTTON_HEIGHT, CONTROL_HEIGHT, CONTROL_WIDTH, Parts, row},
    },
};

/// Events from the signing view.
#[derive(Debug, Clone)]
pub enum SigningEvent {
    /// The set-up was changed; what to say about it.
    Changed(String),
    /// A change was refused; why.
    Refused(String),
    /// Ask where the allowed signers are to be kept.
    ChooseSignersFile,
    /// Ask for somebody's key and allow them to sign.
    AddSigner,
    /// Ask, then take a signer out of the allowed ones.
    RemoveSigner {
        /// The fingerprint of their key.
        fingerprint: String,
        /// Who, as the line names them.
        principals: Vec<String>,
    },
}

/// Somebody who may sign with an SSH key, as `sign.signers` lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signer {
    /// Who: usually one address.
    pub principals: Vec<String>,
    /// What the line says besides: what the key is good for, until when.
    pub options: Vec<String>,
    /// The kind of key.
    pub kind: String,
    /// Its fingerprint.
    pub fingerprint: String,
}

/// The allowed signers and where they are kept.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Signers {
    /// The file, as it is resolved; none when no setting names one.
    pub path: Option<String>,
    /// Whether that file is there.
    pub exists: bool,
    /// Whose configuration names it: `repo`, `user`, or git's own word.
    pub scope: Option<String>,
    /// Who is in it.
    pub signers: Vec<Signer>,
    /// Lines that are no signer and no comment.
    pub skipped: usize,
}

impl Signers {
    /// Read what `sign.signers` answered.
    pub fn from_json(value: &serde_json::Value) -> Self {
        let text = |value: &serde_json::Value, key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let texts = |value: &serde_json::Value, key: &str| -> Vec<String> {
            value
                .get(key)
                .and_then(|list| list.as_array())
                .into_iter()
                .flatten()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        };
        let file = value.get("file").cloned().unwrap_or_default();
        Self {
            path: text(&file, "path"),
            exists: file.get("exists").and_then(|flag| flag.as_bool()) == Some(true),
            scope: text(&file, "scope"),
            signers: value
                .get("signers")
                .and_then(|list| list.as_array())
                .into_iter()
                .flatten()
                .filter_map(|signer| {
                    let found = Signer {
                        principals: texts(signer, "principals"),
                        options: texts(signer, "options"),
                        kind: text(signer, "kind")?,
                        fingerprint: text(signer, "fingerprint")?,
                    };
                    (!found.principals.is_empty()).then_some(found)
                })
                .collect(),
            skipped: value
                .get("skipped")
                .and_then(|count| count.as_u64())
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or(0),
        }
    }

    /// Where the allowed signers are kept, in a sentence.
    pub fn place(&self) -> String {
        // Whose settings name it, as the catalog's messages choose their
        // words by it.
        let whose = match self.scope.as_deref() {
            Some("repo") => "repo",
            Some("user") => "user",
            Some(_) => "other",
            None => "none",
        };
        match (&self.path, self.exists) {
            (None, _) => say!("signing-signers-no-file").to_owned(),
            (Some(path), true) => say!("signing-signers-place", path, whose),
            (Some(path), false) => say!("signing-signers-place-missing", path, whose),
        }
    }
}

/// What a signer's key is good for, when the line says anything about it
/// beyond git's own signatures.
fn options_said(options: &[String]) -> String {
    options
        .iter()
        .filter(|option| option.as_str() != "namespaces=\"git\"")
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
}

/// How the repository signs, as `sign.status` answers it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Status {
    format: Option<String>,
    key: Option<String>,
    /// The program that signs in that format.
    program: Option<String>,
    commits: bool,
    tags: bool,
    can_sign: bool,
    reason: Option<String>,
}

impl Status {
    fn from_json(value: &serde_json::Value) -> Self {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let flag = |key: &str| value.get(key).and_then(|flag| flag.as_bool()) == Some(true);
        Self {
            format: text("format"),
            key: text("key"),
            program: text("program"),
            commits: flag("commits"),
            tags: flag("tags"),
            can_sign: flag("can_sign"),
            reason: text("reason"),
        }
    }

    /// Whether signing can work, in a sentence.
    fn verdict(&self) -> String {
        match (self.can_sign, &self.reason) {
            (true, _) => say!("signing-can-sign").to_owned(),
            (false, Some(reason)) => say!("signing-cannot-sign-why", reason),
            (false, None) => say!("signing-cannot-sign").to_owned(),
        }
    }
}

/// A key that could sign, as `sign.keys` answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    format: String,
    /// What `user.signingkey` takes.
    key: String,
    name: String,
    fingerprint: String,
    /// Where it was found: gpg, a file, the agent.
    source: String,
    usable: bool,
    /// When it expires, in seconds since the Unix epoch.
    expires: Option<i64>,
}

impl Key {
    /// Why it cannot be chosen, for a key that cannot sign: said beside
    /// its name, where a row that is only greyed says nothing.
    fn unusable(&self, now: i64) -> Option<&'static str> {
        match self.expires {
            _ if self.usable => None,
            Some(at) if at <= now => Some(say!("signing-key-expired")),
            _ => Some(say!("signing-key-unusable")),
        }
    }
}

fn keys_from(value: &serde_json::Value) -> Vec<Key> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|key| {
            let text = |name: &str| {
                key.get(name)
                    .and_then(|text| text.as_str())
                    .unwrap_or_default()
                    .to_owned()
            };
            let found = Key {
                format: text("format"),
                key: text("key"),
                name: text("name"),
                fingerprint: text("fingerprint"),
                source: text("source"),
                usable: key.get("usable").and_then(|flag| flag.as_bool()) != Some(false),
                expires: key.get("expires").and_then(|at| at.as_i64()),
            };
            (!found.key.is_empty() && !found.format.is_empty()).then_some(found)
        })
        .collect()
}

/// A fingerprint short enough for a row: its end, which is what people
/// compare.
fn short(fingerprint: &str) -> String {
    let count = fingerprint.chars().count();
    if count <= 20 {
        fingerprint.to_owned()
    } else {
        let tail: String = fingerprint.chars().skip(count.saturating_sub(16)).collect();
        format!("…{tail}")
    }
}

/// The signing view.
pub struct SigningView {
    session: SessionHandle,
    status: Option<Status>,
    keys: Vec<Key>,
    loading_keys: bool,
    /// Who may sign with an SSH key, once that has been asked.
    signers: Option<Signers>,
    error: Option<String>,
    focus_handle: FocusHandle,
    /// The key the arrows are on.
    cursor: Option<usize>,
}

impl EventEmitter<SigningEvent> for SigningView {}

impl Focusable for SigningView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// The key the cursor goes to from `from`, `by` keys on, among `count`:
/// the first or the last from nowhere, and no further than the ends.
fn stepped(count: usize, from: Option<usize>, by: isize) -> Option<usize> {
    let last = count.checked_sub(1)?;
    Some(match from {
        Some(at) => at.min(last).saturating_add_signed(by).min(last),
        None if by < 0 => last,
        None => 0,
    })
}

/// What choosing a key for a place is refused with, when it cannot be:
/// the key cannot sign, it is the one in use there already, or (for the
/// repository's own settings) the repository is opened read-only.
fn not_chosen(usable: bool, in_use: bool, read_only: bool) -> Option<&'static str> {
    if !usable {
        Some(say!("signing-refused-unusable"))
    } else if in_use {
        Some(say!("signing-refused-in-use"))
    } else if read_only {
        Some(say!("signing-refused-read-only"))
    } else {
        None
    }
}

impl SigningView {
    /// Create the view.
    pub fn new(session: SessionHandle, cx: &mut Context<Self>) -> Self {
        Self {
            session,
            status: None,
            keys: Vec::new(),
            loading_keys: false,
            signers: None,
            error: None,
            focus_handle: cx.focus_handle(),
            cursor: None,
        }
    }

    /// The key the arrows are on, by what git would be given for it; for
    /// the off-screen tests.
    pub fn key_under_cursor(&self) -> Option<String> {
        self.keys.get(self.cursor?).map(|key| key.key.clone())
    }

    /// How many keys are listed; for the off-screen tests.
    pub fn key_count(&self) -> usize {
        self.keys.len()
    }

    /// Whether `key` is the one the repository signs with.
    fn in_use(&self, key: &Key) -> bool {
        self.status.as_ref().is_some_and(|status| {
            status.key.as_deref() == Some(key.key.as_str())
                && status.format.as_deref() == Some(key.format.as_str())
        })
    }

    /// Sign with the key at `ix`: in this repository, or everywhere. What
    /// cannot be is said, and nothing is written.
    fn choose(&mut self, ix: usize, everywhere: bool, cx: &mut Context<Self>) {
        let Some(key) = self.keys.get(ix) else {
            return;
        };
        // Everywhere is the user's own settings: neither a read-only
        // repository nor the key in use here is in the way of that.
        let refusal = if everywhere {
            not_chosen(key.usable, false, false)
        } else {
            not_chosen(key.usable, self.in_use(key), self.session.summary.bare)
        };
        if let Some(why) = refusal {
            cx.emit(SigningEvent::Refused(why.to_owned()));
            return;
        }
        let (scope, done) = if everywhere {
            ("user", say!("signing-key-chosen-everywhere"))
        } else {
            ("repo", say!("signing-key-chosen-here"))
        };
        let params = json!({ "format": key.format, "key": key.key, "scope": scope });
        self.configure(params, done.to_owned(), cx);
    }

    /// A key while the view has the focus: the arrows move over the keys
    /// that could sign, Enter or Space signs with the one they are on in
    /// this repository, and with Shift in every repository.
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let by = match event.keystroke.unparse().as_str() {
            "down" | "j" => 1,
            "up" | "k" => -1,
            "home" => isize::MIN,
            "end" => isize::MAX,
            "enter" | "space" | "shift-enter" | "shift-space" => {
                if let Some(ix) = self.cursor {
                    self.choose(ix, event.keystroke.modifiers.shift, cx);
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        self.cursor = stepped(self.keys.len(), self.cursor, by);
        cx.stop_propagation();
        cx.notify();
    }

    /// Who may sign, as last read; for the off-screen tests.
    pub fn signers(&self) -> Option<&Signers> {
        self.signers.as_ref()
    }

    /// Read the allowed signers again: on opening, and whenever one was
    /// added or taken out or the file was chosen.
    pub fn reload_signers(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "sign.signers".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            match reply {
                Reply::Command(value) => this.signers = Some(Signers::from_json(&value)),
                // A file that cannot be read is said where the list would
                // be; the rest of the view is about other things.
                Reply::Error(err) => {
                    this.signers = None;
                    this.error = Some(err);
                }
                _ => {}
            }
            cx.notify();
        });
    }

    /// Read the set-up and the keys again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "sign.status".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    this.status = Some(Status::from_json(&value));
                    this.error = None;
                }
                Reply::Error(err) => this.error = Some(err),
                _ => {}
            }
            cx.notify();
        });
        self.reload_signers(cx);
        self.loading_keys = true;
        let request = Request::Command {
            id: "sign.keys".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            this.loading_keys = false;
            if let Reply::Command(value) = reply {
                this.keys = keys_from(&value);
                // The cursor stays on a key that is there.
                this.cursor = this
                    .cursor
                    .and_then(|at| stepped(this.keys.len(), Some(at), 0));
            }
            cx.notify();
        });
    }

    /// Write a change to the set-up and say how it went.
    fn configure(&mut self, params: serde_json::Value, done: String, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "sign.configure".to_owned(),
            params,
        };
        self.session.call(request, cx, move |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    this.status = Some(Status::from_json(&value));
                    cx.emit(SigningEvent::Changed(done));
                }
                Reply::Error(err) => cx.emit(SigningEvent::Refused(err)),
                _ => {}
            }
            cx.notify();
        });
    }

    fn render_key(&self, ix: usize, key: &Key, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let chosen = self.in_use(key);
        let under_cursor = self.cursor == Some(ix);
        h_flex()
            .id(("sign-key", ix))
            .w_full()
            .min_h(px(32.))
            .px_3()
            .gap_2()
            .items_center()
            .when(chosen, |d| d.bg(theme.success.opacity(0.18)))
            // The row the arrows are on, over the mark of the key in use.
            .when(under_cursor, |d| d.bg(theme.list_active))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.cursor = Some(ix);
                this.focus_handle.focus(window, cx);
                cx.notify();
            }))
            .child(
                div()
                    .flex_none()
                    .w(px(64.))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(key.format.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .when(!key.usable, |d| d.text_color(theme.muted_foreground))
                    .child(if key.name.is_empty() {
                        key.key.clone()
                    } else {
                        key.name.clone()
                    }),
            )
            .children(key.unusable(crate::views::now()).map(|why| {
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(why)
            }))
            .child(
                div()
                    .flex_none()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(short(&key.fingerprint)),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(48.))
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(key.source.clone()),
            )
            .child(
                Button::new(("sign-key-here", ix))
                    .ghost()
                    .xsmall()
                    .label(if chosen {
                        say!("signing-key-in-use")
                    } else {
                        say!("signing-key-use-here")
                    })
                    // A read-only repository's own settings stay as they
                    // are; the user's are not the repository's.
                    .disabled(chosen || !key.usable || self.session.summary.bare)
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(ix, false, cx))),
            )
            .child(
                Button::new(("sign-key-everywhere", ix))
                    .ghost()
                    .xsmall()
                    .label(say!("signing-key-use-everywhere"))
                    .disabled(!key.usable)
                    .on_click(cx.listener(move |this, _, _, cx| this.choose(ix, true, cx))),
            )
            .into_any_element()
    }
}

impl SigningView {
    /// One of the allowed signers: who, the kind of key, its fingerprint,
    /// what else the line says, and a way to take them out.
    fn render_signer(&self, ix: usize, signer: &Signer, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (fingerprint, principals) = (signer.fingerprint.clone(), signer.principals.clone());
        let said = options_said(&signer.options);
        h_flex()
            .w_full()
            .min_h(px(32.))
            .px_3()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(signer.principals.join(", ")),
            )
            .when(!said.is_empty(), |d| {
                d.child(
                    div()
                        .flex_none()
                        .max_w(px(260.))
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(said),
                )
            })
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(signer.kind.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(short(&signer.fingerprint)),
            )
            .child(
                Button::new(("signer-remove", ix))
                    .ghost()
                    .xsmall()
                    .label(say!("signing-signer-remove"))
                    .disabled(self.session.summary.bare)
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(SigningEvent::RemoveSigner {
                            fingerprint: fingerprint.clone(),
                            principals: principals.clone(),
                        })
                    })),
            )
            .into_any_element()
    }
}

/// The formats git signs in: what git calls each, and what the list calls
/// it.
const FORMATS: [(&str, bitbraid_strings::Message); 3] = [
    ("openpgp", message!("signing-format-openpgp")),
    ("ssh", message!("signing-format-ssh")),
    ("x509", message!("signing-format-x509")),
];

impl Render for SigningView {
    /// The section of the preferences that says how this repository
    /// signs: the format, the program, the key, who may sign with an SSH
    /// key, and whether commits and tags are signed when nothing else is
    /// said; then what could sign.
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let status = self.status.clone().unwrap_or_default();
        let signers = self.signers.clone().unwrap_or_default();
        let read_only = self.session.summary.bare;
        let format = status
            .format
            .clone()
            .unwrap_or_else(|| "openpgp".to_owned());
        let format_label = FORMATS
            .iter()
            .find(|(word, _)| *word == format)
            .map_or(format.clone(), |(_, label)| label.text().to_owned());
        let view = cx.entity().downgrade();
        let format_list = {
            let view = view.clone();
            let format = format.clone();
            Button::new("select-signing-format")
                .outline()
                .label(format_label)
                .dropdown_caret(true)
                .disabled(read_only)
                .w(px(CONTROL_WIDTH))
                .h(px(CONTROL_HEIGHT))
                .justify_between()
                .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                    FORMATS.iter().fold(menu, |menu, (word, label)| {
                        let view = view.clone();
                        menu.item(
                            PopupMenuItem::new(label.text())
                                .checked(*word == format)
                                .on_click(move |_, _, cx| {
                                    let _ = view.update(cx, |this, cx| {
                                        this.configure(
                                            json!({ "format": word, "scope": "repo" }),
                                            say!("signing-format-chosen", label = label.text()),
                                            cx,
                                        )
                                    });
                                }),
                        )
                    })
                })
                .into_any_element()
        };
        // The keys of this format, and which of them is in use.
        let listed: Vec<(usize, String)> = self
            .keys
            .iter()
            .enumerate()
            .filter(|(_, key)| key.format == format)
            .map(|(ix, key)| {
                let name = if key.name.is_empty() {
                    key.key.clone()
                } else {
                    key.name.clone()
                };
                (ix, format!("{name} - {}", short(&key.fingerprint)))
            })
            .collect();
        let in_use = self
            .keys
            .iter()
            .enumerate()
            .find(|(_, key)| key.format == format && self.in_use(key))
            .map(|(ix, _)| ix);
        let key_shown = match (
            in_use.and_then(|ix| listed.iter().find(|(at, _)| *at == ix)),
            &status.key,
        ) {
            (Some((_, label)), _) => label.clone(),
            (None, Some(key)) => key.clone(),
            (None, None) => say!("signing-key-none").to_owned(),
        };
        let key_list = {
            let view = view.clone();
            let listed = listed.clone();
            h_flex()
                .w(px(CONTROL_WIDTH))
                .gap(px(5.))
                .child(
                    Button::new("select-signing-key")
                        .outline()
                        .label(key_shown)
                        .dropdown_caret(true)
                        .disabled(read_only || listed.is_empty())
                        .w(px(CONTROL_WIDTH - CONTROL_HEIGHT - 5.))
                        .h(px(CONTROL_HEIGHT))
                        .justify_between()
                        .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                            listed.iter().fold(menu, |menu, (ix, label)| {
                                let (view, ix) = (view.clone(), *ix);
                                menu.item(
                                    PopupMenuItem::new(label.clone())
                                        .checked(in_use == Some(ix))
                                        .on_click(move |_, _, cx| {
                                            let _ = view
                                                .update(cx, |this, cx| this.choose(ix, false, cx));
                                        }),
                                )
                            })
                        }),
                )
                .child(
                    Button::new("signing-reload")
                        .accessibility_label(say!("signing-reload"))
                        .outline()
                        .small()
                        .h(px(CONTROL_HEIGHT))
                        .w(px(CONTROL_HEIGHT))
                        .icon(IconName::RefreshCw)
                        .tooltip(say!("signing-reload"))
                        .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                )
                .into_any_element()
        };
        let verdict = div()
            .flex()
            .gap_1()
            .child(
                Icon::new(if status.can_sign {
                    gpui_kit::assets::IconName::ShieldCheck
                } else {
                    gpui_kit::assets::IconName::ShieldAlert
                })
                .size_3()
                .text_color(if status.can_sign {
                    theme.success
                } else {
                    theme.warning
                }),
            )
            // The reason can be long: it wraps under the control.
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .whitespace_normal()
                    .child(status.verdict()),
            );
        let toggle = |id: &'static str, on: bool, key: &'static str| {
            Checkbox::new(id)
                .accessibility_label(if key == "commits" {
                    say!("signing-sign-commits")
                } else {
                    say!("signing-sign-tags")
                })
                .checked(on)
                .disabled(read_only)
                .on_click(cx.listener(move |this, on: &bool, _, cx| {
                    let said = match (key, *on) {
                        ("commits", true) => say!("signing-commits-signed"),
                        ("commits", false) => say!("signing-commits-not-signed"),
                        (_, true) => say!("signing-tags-signed"),
                        (_, false) => say!("signing-tags-not-signed"),
                    };
                    this.configure(json!({ key: *on, "scope": "repo" }), said.to_owned(), cx)
                }))
                .into_any_element()
        };
        let mut set_up = vec![
            row(
                say!("signing-row-format").into(),
                Parts {
                    control: Some(format_list),
                    under: None,
                },
                cx,
            )
            .into_any_element(),
            row(
                say!("signing-row-program").into(),
                Parts {
                    control: Some(
                        div()
                            .font_family(theme.mono_font_family.clone())
                            .text_sm()
                            .child(
                                status
                                    .program
                                    .clone()
                                    .unwrap_or_else(|| say!("signing-none").to_owned()),
                            )
                            .into_any_element(),
                    ),
                    under: Some(
                        div()
                            .child(say!("signing-program-under"))
                            .into_any_element(),
                    ),
                },
                cx,
            )
            .into_any_element(),
            row(
                say!("signing-row-key").into(),
                Parts {
                    control: Some(key_list),
                    under: Some(
                        v_flex()
                            .gap_1()
                            .child(verdict)
                            .child(say!("signing-key-under"))
                            .into_any_element(),
                    ),
                },
                cx,
            )
            .into_any_element(),
        ];
        if format == "ssh" {
            set_up.push(
                row(
                    say!("signing-row-signers-file").into(),
                    Parts {
                        control: Some(
                            h_flex()
                                .w(px(CONTROL_WIDTH))
                                .gap(px(5.))
                                .child(
                                    div()
                                        .flex_1()
                                        .min_w_0()
                                        .truncate()
                                        .font_family(theme.mono_font_family.clone())
                                        .text_sm()
                                        .child(
                                            signers
                                                .path
                                                .clone()
                                                .unwrap_or_else(|| say!("signing-none").to_owned()),
                                        ),
                                )
                                .child(
                                    Button::new("signers-file")
                                        .outline()
                                        .small()
                                        .h(px(BUTTON_HEIGHT))
                                        .label(say!("signing-signers-file-browse"))
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            cx.emit(SigningEvent::ChooseSignersFile)
                                        })),
                                )
                                .into_any_element(),
                        ),
                        under: Some(div().child(signers.place()).into_any_element()),
                    },
                    cx,
                )
                .into_any_element(),
            );
        }
        set_up.push(
            row(
                say!("signing-row-sign-commits").into(),
                Parts {
                    control: Some(toggle("sign-commits", status.commits, "commits")),
                    under: None,
                },
                cx,
            )
            .into_any_element(),
        );
        set_up.push(
            row(
                say!("signing-row-sign-tags").into(),
                Parts {
                    control: Some(toggle("sign-tags", status.tags, "tags")),
                    under: None,
                },
                cx,
            )
            .into_any_element(),
        );
        // Who may sign with an SSH key: the button, then the signers.
        let mut who = vec![
            row(
                say!("signing-row-signers").into(),
                Parts {
                    control: Some(
                        Button::new("signers-add")
                            .outline()
                            .small()
                            .h(px(BUTTON_HEIGHT))
                            .label(say!("signing-signer-add"))
                            .disabled(read_only)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(SigningEvent::AddSigner)))
                            .into_any_element(),
                    ),
                    under: Some(
                        div()
                            .child(say!("signing-signers-under"))
                            .into_any_element(),
                    ),
                },
                cx,
            )
            .into_any_element(),
        ];
        let allowed: Vec<AnyElement> = signers
            .signers
            .iter()
            .enumerate()
            .map(|(ix, signer)| self.render_signer(ix, signer, cx))
            .collect();
        let nobody = allowed.is_empty() && signers.path.is_some();
        let skipped = signers.skipped;
        who.push(across(
            v_flex()
                .w_full()
                .text_sm()
                .children(allowed)
                .when(nobody, |d| {
                    d.child(
                        div()
                            .py_1()
                            .text_color(theme.muted_foreground)
                            .child(say!("signing-signers-nobody")),
                    )
                })
                .when(skipped > 0, |d| {
                    d.child(
                        div()
                            .py_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(say!("signing-signers-skipped", skipped)),
                    )
                })
                .into_any_element(),
        ));
        let keys: Vec<AnyElement> = self
            .keys
            .iter()
            .enumerate()
            .map(|(ix, key)| self.render_key(ix, key, cx))
            .collect();
        let none = keys.is_empty();
        let could = vec![across(
            v_flex()
                .w_full()
                .text_sm()
                .children(keys)
                .when(none, |d| {
                    d.child(div().py_1().text_color(theme.muted_foreground).child(
                        if self.loading_keys {
                            say!("signing-keys-looking")
                        } else {
                            say!("signing-keys-none")
                        },
                    ))
                })
                .into_any_element(),
        )];
        v_flex()
            .id("signing")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .w_full()
            .when_some(self.error.clone(), |d, err| {
                d.child(across(
                    div()
                        .pb_2()
                        .text_sm()
                        .text_color(theme.danger)
                        .child(err)
                        .into_any_element(),
                ))
            })
            .child(own_group(None, true, set_up, cx))
            .child(own_group(
                Some(say!("signing-group-signers").into()),
                false,
                who,
                cx,
            ))
            .child(own_group(
                Some(say!("signing-group-keys").into()),
                false,
                could,
                cx,
            ))
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{Signers, Status, keys_from, not_chosen, options_said, short, stepped};

    #[test]
    fn the_cursor_walks_the_keys_and_stops_at_the_ends() {
        assert_eq!(stepped(0, None, 1), None);
        assert_eq!(stepped(3, None, 1), Some(0));
        assert_eq!(stepped(3, None, -1), Some(2));
        assert_eq!(stepped(3, Some(0), 1), Some(1));
        assert_eq!(stepped(3, Some(2), 1), Some(2));
        assert_eq!(stepped(3, Some(0), -1), Some(0));
        assert_eq!(stepped(3, Some(1), isize::MIN), Some(0));
        assert_eq!(stepped(3, Some(1), isize::MAX), Some(2));
        // A cursor beyond a list that got shorter comes back onto it.
        assert_eq!(stepped(2, Some(9), 0), Some(1));
        assert_eq!(stepped(0, Some(9), 0), None);
    }

    #[test]
    fn a_key_that_cannot_be_chosen_says_why() {
        assert_eq!(not_chosen(true, false, false), None);
        assert!(
            not_chosen(false, false, false)
                .unwrap()
                .contains("cannot sign")
        );
        assert!(not_chosen(true, true, false).unwrap().contains("in use"));
        assert!(not_chosen(true, false, true).unwrap().contains("read-only"));
        // What is wrong with the key comes first.
        assert!(
            not_chosen(false, true, true)
                .unwrap()
                .contains("cannot sign")
        );
    }

    #[test]
    fn the_set_up_is_read_and_judged() {
        let status = Status::from_json(&json!({
            "format": "ssh",
            "key": "/home/ada/.ssh/id_ed25519.pub",
            "commits": true,
            "tags": false,
            "allowed_signers": null,
            "can_sign": false,
            "reason": "the key file does not exist",
        }));
        assert!(status.commits && !status.tags);
        assert_eq!(
            status.verdict(),
            "Signing cannot work yet: the key file does not exist"
        );
        let fine = Status::from_json(&json!({ "can_sign": true }));
        assert_eq!(fine.verdict(), "Signing can work with what is set up.");
        assert_eq!(Status::from_json(&json!("nonsense")), Status::default());
    }

    #[test]
    fn the_allowed_signers_are_read_and_their_place_is_said() {
        let signers = Signers::from_json(&json!({
            "file": {
                "configured": "~/.config/git/allowed_signers",
                "path": "/home/ada/.config/git/allowed_signers",
                "exists": true, "scope": "user",
            },
            "signers": [
                { "line": 1, "principals": ["ada@fixture.invalid"], "options": ["namespaces=\"git\""],
                  "kind": "ssh-ed25519", "key": "AAAA", "fingerprint": "SHA256:abc", "comment": null },
                { "line": 3, "principals": ["bob@fixture.invalid", "*@ops.invalid"],
                  "options": ["namespaces=\"git\"", "valid-before=\"20300101\""],
                  "kind": "ssh-rsa", "key": "BBBB", "fingerprint": "SHA256:def" },
                { "principals": [], "kind": "ssh-rsa", "fingerprint": "SHA256:none" },
                { "principals": ["no-key@fixture.invalid"] },
                "nonsense",
            ],
            "skipped": 2, "more": 0,
        }));
        assert_eq!(signers.signers.len(), 2);
        assert_eq!(signers.skipped, 2);
        assert_eq!(
            signers.place(),
            "/home/ada/.config/git/allowed_signers (your own git settings name it, for every \
             repository)"
        );
        // What a key is good for is said only where it is more than git.
        assert_eq!(options_said(&signers.signers[0].options), "");
        assert_eq!(
            options_said(&signers.signers[1].options),
            "valid-before=\"20300101\""
        );

        let none = Signers::from_json(&json!({
            "file": { "configured": null, "path": null, "exists": false, "scope": null },
            "signers": [], "skipped": 0,
        }));
        assert!(none.place().starts_with("No file is chosen"));
        let missing = Signers::from_json(&json!({
            "file": { "path": "/repo/.git/allowed_signers", "exists": false, "scope": "repo" },
        }));
        assert_eq!(
            missing.place(),
            "/repo/.git/allowed_signers (this repository's settings name it); it is not there \
             yet, and is made with the first signer"
        );
        assert_eq!(Signers::from_json(&json!("nonsense")), Signers::default());
    }

    #[test]
    fn keys_are_read_and_odd_ones_left_out() {
        let keys = keys_from(&json!([
            { "format": "openpgp", "key": "ABCD", "name": "Ada <ada@fixture.invalid>",
              "fingerprint": "0123456789ABCDEF0123456789ABCDEF01234567", "source": "gpg", "usable": true },
            { "format": "ssh", "key": "/k.pub", "name": "", "fingerprint": "SHA256:x", "source": "file", "usable": false },
            { "format": "ssh" },
            "nonsense",
        ]));
        assert_eq!(keys.len(), 2);
        assert_eq!(short(&keys[0].fingerprint), "…89ABCDEF01234567");
        assert_eq!(short(&keys[1].fingerprint), "SHA256:x");
        assert!(keys[0].usable && !keys[1].usable);
        assert!(keys_from(&json!({})).is_empty());
    }

    #[test]
    fn a_key_that_cannot_sign_says_why() {
        let keys = keys_from(&json!([
            { "format": "openpgp", "key": "A", "usable": true, "expires": 50 },
            { "format": "openpgp", "key": "B", "usable": false, "expires": 50 },
            { "format": "openpgp", "key": "C", "usable": false, "expires": 500 },
            { "format": "openpgp", "key": "D", "usable": false },
            { "format": "openpgp", "key": "E", "usable": false, "expires": "soon" },
        ]));
        let now = 100;
        // One that can sign says nothing, whatever its date.
        assert_eq!(keys[0].unusable(now), None);
        assert_eq!(keys[1].unusable(now), Some("expired"));
        // Not expired and still no good: revoked or disabled.
        for key in &keys[2..] {
            assert_eq!(
                key.unusable(now),
                Some("cannot sign: revoked or disabled"),
                "{}",
                key.key
            );
        }
        assert_eq!(
            keys[1].unusable(i64::MIN),
            Some("cannot sign: revoked or disabled")
        );
        assert_eq!(keys[1].unusable(i64::MAX), Some("expired"));
    }
}
