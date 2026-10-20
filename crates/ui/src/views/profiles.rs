//! Profiles: the identities the user commits as, which of them is the
//! current one, and the section of the preferences that keeps them.
//!
//! There is always a current profile: the one git's own identity is, else
//! the one the application was last switched to, else the first. When there
//! is none at all, one is made from git's identity the first time the
//! profiles are read, called Default; when git knows nobody yet, Default
//! stands with nobody in it until it is given a name and an address.
//!
//! The section lists the profiles in a table as wide as the form, each
//! with its picture (the pattern its commits carry in the graph), its
//! author and address, and a menu of what can be done with it; a new one
//! and a change are made in a dialog. Deleting asks first, in the bar over
//! the page (`crate::ask`). Switching to a profile is the window's: it
//! writes the identity to git's settings and brings the profile's tabs back.
//!
//! Sources: gpui-kit documentation (dialog, input, checkbox, popup menu).

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::session::{Reply, run_standalone};

/// What the profile made from git's identity is called.
pub const DEFAULT_NAME: bitbraid_strings::Message = message!("profiles-default-name");

/// One profile, as `profile.list` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Profile {
    pub name: String,
    /// The name to commit as.
    pub user_name: String,
    /// The address to commit as.
    pub user_email: String,
    /// The key to sign with.
    pub signing_key: Option<String>,
    /// How that key signs.
    pub signing_format: Option<String>,
    /// The private key file to connect with.
    pub ssh_key: Option<String>,
    /// Whether it is kept in `profiles.toml`: Default is not while git
    /// knows nobody.
    pub stored: bool,
}

impl Profile {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        Some(Self {
            name: text("name")?,
            user_name: text("user_name").unwrap_or_default(),
            user_email: text("user_email").unwrap_or_default(),
            signing_key: text("signing_key"),
            signing_format: text("signing_format"),
            ssh_key: text("ssh_key"),
            stored: true,
        })
    }

    /// What it has besides a name and an address, in a few words.
    pub fn extras(&self) -> String {
        let mut parts = Vec::new();
        if self.signing_key.is_some() {
            parts.push(match self.signing_format.as_deref() {
                Some("ssh") => say!("profiles-signs-ssh"),
                Some("x509") => say!("profiles-signs-certificate"),
                _ => say!("profiles-signs-gpg"),
            });
        }
        if self.ssh_key.is_some() {
            parts.push(say!("profiles-own-ssh-key"));
        }
        parts.join(" · ")
    }

    /// What its picture is made from: its address, as the pictures of
    /// authors are, so that it looks like its commits; its name without
    /// one.
    pub fn picture_key(&self) -> &str {
        picture_key(&self.name, Some(&self.user_email))
    }

    /// What `profile.save` keeps of it.
    pub fn as_json(&self) -> serde_json::Value {
        json!({
            "name": self.name,
            "user_name": self.user_name,
            "user_email": self.user_email,
            "signing_key": self.signing_key,
            "signing_format": self.signing_key.as_ref().map(|key| {
                self.signing_format.clone().unwrap_or_else(|| format_of(key).to_owned())
            }),
            "ssh_key": self.ssh_key,
        })
    }
}

/// What a picture is made from: the address, the name without one.
pub fn picture_key<'a>(name: &'a str, address: Option<&'a str>) -> &'a str {
    match address.map(str::trim) {
        Some(address) if !address.is_empty() => address,
        _ => name,
    }
}

/// The picture of a profile, `size` each way.
pub fn picture(key: &str, size: f32, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .w(px(size))
        .h(px(size))
        .flex_none()
        .rounded(px((size / 7.).max(2.)))
        .overflow_hidden()
        .bg(theme.muted)
        .child(crate::views::avatar::identicon(
            key,
            px(size),
            theme.is_dark(),
        ))
        .into_any_element()
}

/// The format a signing key is in, told from how it looks: a path (on
/// Windows one that begins with a drive too) or a literal SSH key is SSH,
/// anything else a GPG key id.
pub fn format_of(key: &str) -> &'static str {
    let key = key.trim();
    if key.starts_with('/')
        || std::path::Path::new(key).is_absolute()
        || key.starts_with('~')
        || key.starts_with("ssh-")
        || key.starts_with("ecdsa-")
        || key.starts_with("sk-")
        || key.starts_with("key::")
        || key.ends_with(".pub")
    {
        "ssh"
    } else {
        "openpgp"
    }
}

/// The profiles, and who is committed as.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Profiles {
    /// Every profile, by name.
    pub list: Vec<Profile>,
    /// The current one, by name.
    pub current: Option<String>,
    /// The name and address git commits as, wherever a repository says
    /// nothing else.
    pub identity: Option<(String, String)>,
    /// What went wrong reading them.
    pub problem: Option<String>,
}

impl Profiles {
    /// The current profile.
    pub fn current_profile(&self) -> Option<&Profile> {
        let name = self.current.as_deref()?;
        self.list.iter().find(|profile| profile.name == name)
    }

    /// Every profile, the current one first, as the lists to switch from
    /// have them.
    pub fn current_first(&self) -> Vec<&Profile> {
        let mut list: Vec<&Profile> = self.list.iter().collect();
        list.sort_by_key(|profile| self.current.as_deref() != Some(profile.name.as_str()));
        list
    }

    /// The names of the profiles.
    pub fn names(&self) -> Vec<String> {
        self.list
            .iter()
            .map(|profile| profile.name.clone())
            .collect()
    }
}

/// Whether a profile is the identity git commits as: the same name and
/// address, and the same keys.
fn is_identity(profile: &Profile, identity: &Profile) -> bool {
    profile.user_name == identity.user_name
        && profile
            .user_email
            .eq_ignore_ascii_case(&identity.user_email)
        && profile.signing_key == identity.signing_key
        && profile.ssh_key == identity.ssh_key
}

/// The current profile among `list`: the one switched to last when git's
/// identity is still that one (two profiles can hold the same identity),
/// else the one git's identity is, else the one switched to last, else the
/// first.
pub fn choose_current(
    list: &[Profile],
    identity: &Profile,
    matching: Option<&str>,
    remembered: Option<&str>,
) -> Option<String> {
    let find = |name: &str| list.iter().find(|profile| profile.name == name);
    let remembered = remembered.and_then(find);
    remembered
        .filter(|profile| is_identity(profile, identity))
        .or(matching.and_then(find))
        .or(remembered)
        .or(list.first())
        .map(|profile| profile.name.clone())
}

/// The profiles as `profile.list` answered, by name.
fn listed(value: &serde_json::Value) -> Vec<Profile> {
    let mut list: Vec<Profile> = value
        .get("profiles")
        .and_then(|list| list.as_array())
        .map(|list| list.iter().filter_map(Profile::from_json).collect())
        .unwrap_or_default();
    list.sort_by_key(|profile| profile.name.to_lowercase());
    list
}

/// The identity in what `profile.current` answered, as a profile would
/// hold it, and the profile it is.
fn identity_in(value: &serde_json::Value) -> (Profile, Option<String>) {
    let identity = value.get("identity");
    let text = |key: &str| {
        identity
            .and_then(|identity| identity.get(key))
            .and_then(|text| text.as_str())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let profile = Profile {
        name: DEFAULT_NAME.text().to_owned(),
        user_name: text("user_name").unwrap_or_default(),
        user_email: text("user_email").unwrap_or_default(),
        signing_key: text("signing_key"),
        signing_format: text("signing_format"),
        ssh_key: text("ssh_key"),
        stored: false,
    };
    let matching = value
        .get("profile")
        .and_then(|name| name.as_str())
        .map(str::to_owned);
    (profile, matching)
}

/// Read the profiles and who is committed as, and see that there is a
/// current one: the first time there are none, Default is made from git's
/// identity. `remembered` is the profile last switched to.
pub async fn read(remembered: Option<String>) -> Profiles {
    let (list, current) = (
        run_standalone("profile.list", json!({})),
        run_standalone("profile.current", json!({ "scope": "user" })),
    );
    let (mut list, mut problem) = match list.reply.await {
        Ok(Reply::Command(value)) => (listed(&value), None),
        Ok(Reply::Error(err)) => (Vec::new(), Some(err)),
        _ => (Vec::new(), None),
    };
    let (identity, matching) = match current.reply.await {
        Ok(Reply::Command(value)) => identity_in(&value),
        _ => (identity_in(&serde_json::Value::Null).0, None),
    };
    let committing = (!identity.user_name.is_empty() || !identity.user_email.is_empty())
        .then(|| (identity.user_name.clone(), identity.user_email.clone()));
    if list.is_empty() && problem.is_none() {
        let complete = !identity.user_name.is_empty() && !identity.user_email.is_empty();
        let mut made = identity.clone();
        if complete {
            let saved = run_standalone(
                "profile.save",
                json!({ "profile": identity.as_json(), "rename_from": null }),
            );
            match saved.reply.await {
                Ok(Reply::Command(_)) => made.stored = true,
                Ok(Reply::Error(err)) => problem = Some(err),
                _ => {}
            }
        }
        list.push(made);
    }
    let current = choose_current(&list, &identity, matching.as_deref(), remembered.as_deref());
    Profiles {
        list,
        current,
        identity: committing,
        problem,
    }
}

/// What can be done with a profile from the section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Act {
    /// Make a new one.
    Add,
    /// Change one.
    Edit(String),
    /// Delete one, once that is confirmed.
    Delete(String),
    /// Make it the current one.
    Switch(String),
    /// Commit as it in the open repository only.
    UseHere(String),
}

/// What the section's controls hand an act to.
pub type OnAct = Rc<dyn Fn(Act, &mut Window, &mut App)>;

/// The table's columns: profile, author, address, and what else it has.
const COLUMNS: f32 = 4.;
/// A row of the table.
const ROW_HEIGHT: f32 = 50.;
/// From one row to the next.
const ROW_GAP: f32 = 10.;
/// The picture in a row.
const ROW_PICTURE: f32 = 30.;

/// The button that makes a new profile, over the table.
pub fn add_button(on: OnAct) -> AnyElement {
    h_flex()
        .pt(px(12.))
        .pb(px(2.))
        .child(
            Button::new("profile-add")
                .outline()
                .small()
                .h(px(30.))
                .icon(Icon::new(IconName::Plus))
                .label(say!("profiles-add"))
                .on_click(move |_, window, cx| on(Act::Add, window, cx)),
        )
        .into_any_element()
}

/// The table of profiles, across the form.
pub fn table(profiles: &Profiles, repo_open: bool, on: OnAct, cx: &App) -> AnyElement {
    let theme = cx.theme().clone();
    let cell = |child: AnyElement| {
        div()
            .w(relative(1. / COLUMNS))
            .min_w_0()
            .pl(px(10.))
            .truncate()
            .child(child)
    };
    let heading = h_flex()
        .w_full()
        .h(px(40.))
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .text_sm()
        .child(cell(
            h_flex()
                .gap_1()
                .child(say!("profiles-profile-name"))
                .child(
                    Icon::new(IconName::ArrowUp)
                        .size(px(12.))
                        .text_color(theme.muted_foreground),
                )
                .into_any_element(),
        ))
        .child(cell(
            div().child(say!("profiles-author-name")).into_any_element(),
        ))
        .child(cell(
            div()
                .child(say!("profiles-author-email"))
                .into_any_element(),
        ))
        .child(cell(div().child(say!("profiles-keys")).into_any_element()));
    let rows = profiles.list.iter().enumerate().map(|(ix, profile)| {
        let current = profiles.current.as_deref() == Some(profile.name.as_str());
        let on = on.clone();
        let name = profile.name.clone();
        let menu = Button::new(("profile-menu", ix))
            .accessibility_label(say!("profiles-menu"))
            .ghost()
            .small()
            .icon(Icon::new(IconName::EllipsisVertical))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let act = |act: Act| {
                    let on = on.clone();
                    move |_: &ClickEvent, window: &mut Window, cx: &mut App| {
                        on(act.clone(), window, cx)
                    }
                };
                menu.item(
                    PopupMenuItem::new(say!("profiles-use-everywhere"))
                        .disabled(current)
                        .on_click(act(Act::Switch(name.clone()))),
                )
                .item(
                    PopupMenuItem::new(say!("profiles-use-here"))
                        .disabled(!repo_open)
                        .on_click(act(Act::UseHere(name.clone()))),
                )
                .item(
                    PopupMenuItem::new(say!("profiles-change"))
                        .on_click(act(Act::Edit(name.clone()))),
                )
                .separator()
                .item(
                    PopupMenuItem::new(say!("profiles-delete"))
                        .disabled(current)
                        .on_click(act(Act::Delete(name.clone()))),
                )
            });
        div()
            .relative()
            .w_full()
            .child(
                h_flex()
                    .id(("profile-row", ix))
                    .w_full()
                    .h(px(ROW_HEIGHT))
                    .items_center()
                    .rounded(theme.radius)
                    .bg(theme.secondary)
                    .border_2()
                    .border_color(if current { theme.ring } else { theme.secondary })
                    .text_sm()
                    .child(cell(
                        h_flex()
                            .gap(px(11.))
                            .child(picture(profile.picture_key(), ROW_PICTURE, cx))
                            .child(div().min_w_0().truncate().child(profile.name.clone()))
                            .into_any_element(),
                    ))
                    .child(cell(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(profile.user_name.clone())
                            .into_any_element(),
                    ))
                    .child(cell(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(profile.user_email.clone())
                            .into_any_element(),
                    ))
                    .child(
                        h_flex()
                            .w(relative(1. / COLUMNS))
                            .min_w_0()
                            .pl(px(10.))
                            .pr(px(22.))
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(profile.extras()),
                            )
                            .child(menu),
                    )
                    .test_support(),
            )
            .when(current, |row| {
                row.child(
                    div()
                        .absolute()
                        .top(px(-6.))
                        .left(px(-6.))
                        .size(px(14.))
                        .rounded_full()
                        .bg(theme.ring)
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            Icon::new(IconName::Check)
                                .size(px(10.))
                                .text_color(theme.primary_foreground),
                        ),
                )
            })
    });
    v_flex()
        .w_full()
        .gap(px(ROW_GAP))
        .child(heading)
        .children(rows)
        .into_any_element()
}

/// What a dialog for a profile keeps while it is open.
pub struct Dialog {
    /// The profile it changes; none for a new one.
    editing: Option<String>,
    /// The current profile, to take settings from.
    from: Option<Profile>,
    /// Every profile's name, so that a name is not taken twice.
    taken: Vec<String>,
    name: Entity<InputState>,
    user_name: Entity<InputState>,
    user_email: Entity<InputState>,
    signing_key: Entity<InputState>,
    ssh_key: Entity<InputState>,
    /// Take the current profile's settings for what is left empty.
    copy: bool,
    /// Why it was not saved.
    refused: Option<String>,
    /// A save is under way.
    busy: bool,
}

/// What is wrong with what a dialog holds, if anything: which field, and
/// why. Only what git would refuse or change is said; the engine checks
/// all of it again.
pub fn judged(
    name: &str,
    user_name: &str,
    user_email: &str,
    editing: Option<&str>,
    taken: &[String],
) -> Option<(usize, &'static str)> {
    let name = name.trim();
    if name.is_empty() {
        // Nothing to say yet: the button waits for a name.
        return Some((0, ""));
    }
    let same = |other: &String| other.to_lowercase() == name.to_lowercase();
    let renamed_to_itself =
        editing.is_some_and(|editing| editing.to_lowercase() == name.to_lowercase());
    if !renamed_to_itself && taken.iter().any(same) {
        return Some((0, say!("profiles-name-taken")));
    }
    if user_name.contains(['<', '>']) {
        return Some((1, say!("profiles-author-name-invalid")));
    }
    let address = user_email.trim();
    if address.contains(['<', '>']) || address.chars().any(char::is_whitespace) {
        return Some((2, say!("profiles-author-email-invalid")));
    }
    None
}

/// The profile a dialog would save: what is typed, and for a new profile
/// with the box ticked, the current profile's for what is left empty.
fn assembled(dialog: &Dialog, cx: &App) -> Profile {
    let read = |input: &Entity<InputState>| input.read(cx).value().trim().to_owned();
    let mut profile = Profile {
        name: read(&dialog.name),
        user_name: read(&dialog.user_name),
        user_email: read(&dialog.user_email),
        signing_key: Some(read(&dialog.signing_key)).filter(|key| !key.is_empty()),
        signing_format: None,
        ssh_key: Some(read(&dialog.ssh_key))
            .filter(|key| !key.is_empty())
            .and_then(|key| crate::shell::expand_home(&key).to_str().map(str::to_owned)),
        stored: true,
    };
    profile.signing_format = profile
        .signing_key
        .as_deref()
        .map(|key| format_of(key).to_owned());
    if dialog.editing.is_none()
        && dialog.copy
        && let Some(from) = &dialog.from
    {
        if profile.user_name.is_empty() {
            profile.user_name = from.user_name.clone();
        }
        if profile.user_email.is_empty() {
            profile.user_email = from.user_email.clone();
        }
        if profile.signing_key.is_none() {
            profile.signing_key = from.signing_key.clone();
            profile.signing_format = from.signing_format.clone();
        }
        if profile.ssh_key.is_none() {
            profile.ssh_key = from.ssh_key.clone();
        }
    }
    profile
}

/// What the window is told when a profile was saved: its name, and
/// whether it is new.
pub type OnSaved = Rc<dyn Fn(String, bool, &mut Window, &mut App)>;

/// Open the dialog that makes a profile, or changes `editing`. The
/// subscriptions that redraw it as it is typed in belong to whoever opens
/// it, for as long as it is open.
pub fn open_dialog<V: 'static>(
    profiles: &Profiles,
    editing: Option<&Profile>,
    on_saved: OnSaved,
    window: &mut Window,
    cx: &mut Context<V>,
) -> Vec<Subscription> {
    let mut input = |value: &str, placeholder: &str| {
        let value = value.to_owned();
        let placeholder = placeholder.to_owned();
        cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(value)
        })
    };
    let from = profiles.current_profile().cloned();
    let (copy_name, copy_email) = from
        .as_ref()
        .filter(|_| editing.is_none())
        .map(|from| (from.user_name.clone(), from.user_email.clone()))
        .unwrap_or_default();
    let edited = editing.cloned().unwrap_or_default();
    let name = input(&edited.name, say!("profiles-name-placeholder"));
    let user_name = input(&edited.user_name, &copy_name);
    let user_email = input(&edited.user_email, &copy_email);
    let signing_key = input(
        edited.signing_key.as_deref().unwrap_or(""),
        say!("profiles-signing-key-placeholder"),
    );
    let ssh_key = input(
        edited.ssh_key.as_deref().unwrap_or(""),
        say!("profiles-ssh-key-placeholder"),
    );
    let state = cx.new(|_| Dialog {
        editing: editing
            .filter(|profile| profile.stored)
            .map(|profile| profile.name.clone()),
        from,
        taken: profiles.names(),
        name: name.clone(),
        user_name: user_name.clone(),
        user_email: user_email.clone(),
        signing_key: signing_key.clone(),
        ssh_key: ssh_key.clone(),
        copy: true,
        refused: None,
        busy: false,
    });
    let subscriptions = [&name, &user_name, &user_email, &signing_key, &ssh_key]
        .into_iter()
        .map(|input| {
            cx.subscribe_in(input, window, |_, _, event: &InputEvent, window, _| {
                if matches!(event, InputEvent::Change) {
                    window.refresh();
                }
            })
        })
        .collect();
    let adding = editing.is_none();
    let first = name.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let theme = cx.theme().clone();
        let this = state.read(cx);
        let typed = |input: &Entity<InputState>| input.read(cx).value().to_string();
        let shown_key = picture_key(&typed(&this.name), Some(&typed(&this.user_email))).to_owned();
        let verdict = judged(
            &typed(&this.name),
            &typed(&this.user_name),
            &typed(&this.user_email),
            this.editing.as_deref(),
            &this.taken,
        );
        let refused = this.refused.clone();
        let (busy, copy) = (this.busy, this.copy);
        let words =
            |text: &'static str| div().text_sm().line_height(px(20.)).pb(px(4.)).child(text);
        let said = |field: usize| {
            verdict
                .filter(|(at, why)| *at == field && !why.is_empty())
                .map(|(_, why)| {
                    div()
                        .pt(px(4.))
                        .text_xs()
                        .text_color(theme.danger)
                        .child(why)
                })
        };
        let field = |id: &'static str, input: &Entity<InputState>| {
            Input::new(input).id(id).min_h(px(34.)).w_full()
        };
        let body = v_flex()
            .w_full()
            .gap(px(14.))
            .child(
                h_flex()
                    .w_full()
                    .items_end()
                    .gap(px(10.))
                    .child(picture(&shown_key, 56., cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(words(say!("profiles-profile-name")))
                            .child(field("profile-name", &this.name)),
                    ),
            )
            .children(said(0))
            .child(
                v_flex()
                    .child(words(say!("profiles-author-name")))
                    .child(field("profile-user-name", &this.user_name))
                    .children(said(1)),
            )
            .child(
                v_flex()
                    .child(words(say!("profiles-author-email")))
                    .child(field("profile-user-email", &this.user_email))
                    .children(said(2)),
            )
            .child(
                v_flex()
                    .child(words(say!("profiles-signing-key")))
                    .child(field("profile-signing-key", &this.signing_key)),
            )
            .child(
                v_flex()
                    .child(words(say!("profiles-ssh-key")))
                    .child(field("profile-ssh-key", &this.ssh_key)),
            )
            .children(refused.map(|why| {
                div()
                    .text_sm()
                    .text_color(theme.danger)
                    .whitespace_normal()
                    .child(why)
            }));
        let (for_copy, for_save) = (state.clone(), state.clone());
        let on_saved = on_saved.clone();
        dialog
            .title(
                h_flex()
                    .gap_2()
                    .child(Icon::new(IconName::User).size(px(18.)))
                    .child(if adding {
                        say!("profiles-add")
                    } else {
                        say!("profiles-dialog-change")
                    }),
            )
            .width(px(748.))
            .child(body)
            .footer(
                h_flex()
                    .w_full()
                    .gap(px(10.))
                    .justify_end()
                    .items_center()
                    .when(adding, |row| {
                        row.child(
                            Checkbox::new("profile-copy")
                                .label(say!("profiles-copy-current"))
                                .checked(copy)
                                .on_click(move |on: &bool, window, cx| {
                                    let on = *on;
                                    for_copy.update(cx, |dialog, _| dialog.copy = on);
                                    window.refresh();
                                }),
                        )
                    })
                    .child(
                        Button::new("profile-cancel")
                            .outline()
                            .small()
                            .label(say!("profiles-cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("profile-save")
                            .primary()
                            .small()
                            .label(if adding {
                                say!("profiles-dialog-add-button")
                            } else {
                                say!("profiles-dialog-save")
                            })
                            .disabled(verdict.is_some() || busy)
                            .on_click(move |_, window, cx| {
                                save(&for_save, on_saved.clone(), window, cx)
                            }),
                    ),
            )
    });
    cx.defer_in(window, move |_, window, cx| {
        first.update(cx, |input, cx| input.focus(window, cx));
    });
    subscriptions
}

/// Save what the dialog holds; it closes when that is done, and stays
/// open, with what was typed and why, when it is refused.
fn save(state: &Entity<Dialog>, on_saved: OnSaved, window: &mut Window, cx: &mut App) {
    let profile = assembled(state.read(cx), cx);
    let editing = state.read(cx).editing.clone();
    state.update(cx, |dialog, _| {
        dialog.busy = true;
        dialog.refused = None;
    });
    let run = run_standalone(
        "profile.save",
        json!({ "profile": profile.as_json(), "rename_from": editing }),
    );
    let state = state.clone();
    let name = profile.name.clone();
    let new = editing.is_none();
    window
        .spawn(cx, async move |cx| {
            let reply = run.reply.await;
            let _ = cx.update(|window, cx| {
                state.update(cx, |dialog, _| dialog.busy = false);
                match reply {
                    Ok(Reply::Command(value)) => {
                        // Renamed: what was kept under its old name (its
                        // tabs, that it is the one switched to) goes with
                        // it. The engine moved its workspaces; what kept
                        // them from following is said.
                        if let Some(old) = editing.as_deref().filter(|old| *old != name) {
                            let prefs = cx.global_mut::<crate::prefs::Prefs>();
                            prefs.state.rename_profile(old, &name);
                            prefs.save_state();
                        }
                        if let Some(why) = value
                            .get("workspaces_error")
                            .and_then(serde_json::Value::as_str)
                        {
                            crate::notices::push(
                                window,
                                cx,
                                crate::notices::Notice::warning(say!(
                                    "profiles-workspaces-not-renamed",
                                    why = crate::shell::sentence(why)
                                )),
                            );
                        }
                        window.close_dialog(cx);
                        on_saved(name, new, window, cx);
                    }
                    Ok(Reply::Error(err)) => {
                        state.update(cx, |dialog, _| {
                            dialog.refused = Some(crate::shell::sentence(&err))
                        });
                        window.refresh();
                    }
                    _ => {}
                }
            });
        })
        .detach();
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{Profile, choose_current, format_of, identity_in, judged, listed};

    fn named(name: &str) -> Profile {
        Profile {
            name: name.to_owned(),
            stored: true,
            ..Profile::default()
        }
    }

    #[test]
    fn a_profile_is_read_and_says_what_else_it_has() {
        let listed = listed(&json!({ "profiles": [
            { "name": "Work", "user_name": "Ada Fixture", "user_email": "ada@work.invalid",
              "signing_key": "/home/ada/.ssh/work.pub", "signing_format": "ssh",
              "ssh_key": "/home/ada/.ssh/work" },
            { "name": "home", "user_name": "Ada", "user_email": "ada@home.invalid" },
            { "name": "G", "signing_key": "ABCD" },
            { "user_name": "nameless" },
        ]}));
        let names: Vec<&str> = listed.iter().map(|profile| profile.name.as_str()).collect();
        assert_eq!(names, ["G", "home", "Work"], "by name, whatever the case");
        assert_eq!(listed[2].extras(), "signs with SSH · own SSH key");
        assert_eq!(listed[1].extras(), "");
        assert_eq!(listed[0].extras(), "signs with GPG");
        assert_eq!(listed[2].picture_key(), "ada@work.invalid");
        assert_eq!(listed[0].picture_key(), "G", "no address: the name");
        assert!(super::listed(&json!("nonsense")).is_empty());
    }

    #[test]
    fn there_is_always_a_current_profile_when_there_is_one_at_all() {
        let person = |name: &str, email: &str| Profile {
            user_name: name.to_owned(),
            user_email: email.to_owned(),
            ..named(name)
        };
        let (home, work) = (person("Home", "h@x.invalid"), person("Work", "w@x.invalid"));
        let list = [home.clone(), work.clone()];
        // Git's identity is a profile: that one.
        assert_eq!(
            choose_current(&list, &work, Some("Work"), Some("Home")).as_deref(),
            Some("Work")
        );
        // It is none: the one switched to last.
        assert_eq!(
            choose_current(&list, &named("x"), None, Some("Work")).as_deref(),
            Some("Work")
        );
        // Neither, or one that is gone: the first.
        assert_eq!(
            choose_current(&list, &named("x"), Some("Gone"), Some("Gone")).as_deref(),
            Some("Home")
        );
        assert_eq!(choose_current(&[], &work, Some("Work"), None), None);
        // A profile made as a copy of another holds the same identity: the
        // one switched to is the current one, not the first of the two.
        let copy = Profile {
            name: "Copy".to_owned(),
            ..home.clone()
        };
        let list = [copy, home.clone()];
        assert_eq!(
            choose_current(&list, &home, Some("Copy"), Some("Home")).as_deref(),
            Some("Home")
        );
    }

    #[test]
    fn git_s_identity_is_what_the_default_profile_is_made_of() {
        let (made, matching) = identity_in(&json!({
            "identity": { "user_name": " Ada Fixture ", "user_email": "ada@fixture.invalid",
                          "signing_key": null, "ssh_key": "/home/ada/.ssh/id" },
            "profile": null,
        }));
        assert_eq!(made.name, "Default");
        assert_eq!(made.user_name, "Ada Fixture");
        assert_eq!(made.ssh_key.as_deref(), Some("/home/ada/.ssh/id"));
        assert!(!made.stored);
        assert_eq!(matching, None);
        let (nobody, _) = identity_in(&json!(null));
        assert_eq!(
            (nobody.user_name.as_str(), nobody.user_email.as_str()),
            ("", "")
        );
    }

    /// What would be refused is said at its field before it is saved, and
    /// nothing is emptied.
    #[test]
    fn what_cannot_be_saved_is_said_at_its_field() {
        let taken = ["Work".to_owned(), "Home".to_owned()];
        let at = |name: &str, user: &str, email: &str, editing: Option<&str>| {
            judged(name, user, email, editing, &taken).map(|(field, _)| field)
        };
        assert_eq!(at("", "Ada", "ada@x.invalid", None), Some(0));
        assert_eq!(at("work", "Ada", "ada@x.invalid", None), Some(0), "taken");
        assert_eq!(
            at("work", "Ada", "ada@x.invalid", Some("Work")),
            None,
            "itself"
        );
        assert_eq!(at("Club", "Ada <x>", "ada@x.invalid", None), Some(1));
        assert_eq!(at("Club", "Ada", "ada@x.invalid<x>", None), Some(2));
        assert_eq!(at("Club", "Ada", "ada @x.invalid", None), Some(2));
        assert_eq!(at("Club", "", "", None), None, "taken from the current one");
    }

    #[test]
    fn a_signing_keys_format_is_told_from_how_it_looks() {
        for ssh in [
            "/home/ada/.ssh/id_ed25519.pub",
            "~/.ssh/id.pub",
            "ssh-ed25519 AAAAC3Nza",
            "key::ssh-ed25519 AAAA",
            "work.pub",
        ] {
            assert_eq!(format_of(ssh), "ssh", "{ssh}");
        }
        for gpg in ["ABCD1234", "0x1234ABCD", "ada@fixture.invalid"] {
            assert_eq!(format_of(gpg), "openpgp", "{gpg}");
        }
        if cfg!(windows) {
            for ssh in [
                r"C:\Users\ada\.ssh\id_ed25519",
                "C:/Users/ada/.ssh/id_ed25519",
            ] {
                assert_eq!(format_of(ssh), "ssh", "{ssh}");
            }
        }
    }
}
