//! Changing a pull request in place: a pencil beside its title and its
//! description turns them into fields with Save and Cancel; the pencils
//! of the reviewers, the assignees, the labels and the milestone drop a
//! list of what the provider allows (`pr.options`, read the first time
//! one is opened), where a click picks or drops one and the change is
//! sent at once. Every change is a `pr.update`, whose answer is the pull
//! request as the view shows it.
//!
//! Sources: docs.github.com/en/rest/pulls/pulls ("Update a pull
//! request"), docs.github.com/en/rest/pulls/review-requests,
//! docs.github.com/en/rest/issues (assignees, labels, milestones);
//! docs.gitlab.com/api/merge_requests ("Update MR").

use bitbraid_integrations::model::{PullDetails, PullOptions};
use gpui_kit::component::{
    Disableable, Icon, Sizable,
    button::{Button, ButtonVariants},
    input::{InputEvent, InputState, TextareaState},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
};
use gpui_kit::*;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{PullView, PullViewEvent};
use crate::views::pulls::{calls, facts::shown_line};
use crate::zoom::px;

/// How high a list that drops from a pencil is at most.
const MENU_HEIGHT: f32 = 320.0;
/// The most entries such a list has.
const MOST_LISTED: usize = 500;

/// What is being edited in place.
pub(super) enum Editing {
    /// The title, in a field.
    Title(Entity<InputState>),
    /// The description, in a field of several lines.
    Body(Entity<TextareaState>),
}

/// Which list a pencil of the right column changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listed {
    /// Who is asked to review it.
    Reviewers,
    /// Who it is assigned to.
    Assignees,
    /// Its labels.
    Labels,
    /// Its milestone.
    Milestone,
}

impl Listed {
    fn key(self) -> &'static str {
        match self {
            Self::Reviewers => "reviewers",
            Self::Assignees => "assignees",
            Self::Labels => "labels",
            Self::Milestone => "milestone",
        }
    }
}

impl PullView {
    /// Send a command about this pull request, with `extra` beside what
    /// names it; `done` takes the answer. One at a time: while one is out,
    /// another is not sent.
    pub(super) fn send<T: DeserializeOwned + 'static>(
        &mut self,
        id: &'static str,
        extra: Value,
        what: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
        done: impl FnOnce(&mut Self, T, &mut Window, &mut Context<Self>) + 'static,
    ) {
        if self.sending.is_some() {
            return;
        }
        let mut params = json!({
            "account": self.link.account,
            "owner": self.link.owner,
            "name": self.link.name,
            "number": self.number,
        });
        if let (Some(params), Value::Object(extra)) = (params.as_object_mut(), extra) {
            params.extend(extra);
        }
        self.sending = Some(what);
        self.write_error = None;
        let asked = calls::ask::<T>(id, params);
        cx.spawn_in(window, async move |this, cx| {
            let answer = asked.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.sending = None;
                match answer {
                    Ok(value) => done(this, value, window, cx),
                    Err(error) => this.write_error = Some(format!("{what}: {error}")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Change it: `change` is what `pr.update` takes beside its number.
    pub(super) fn change(
        &mut self,
        change: Value,
        what: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.send::<PullDetails>(
            "pr.update",
            change,
            what,
            window,
            cx,
            |this, details, _, cx| {
                this.editing = None;
                this.take(details);
                cx.emit(PullViewEvent::Changed);
            },
        );
    }

    /// Edit the title in place.
    pub fn edit_title(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self
            .details
            .as_ref()
            .map(|details| details.pull.title.clone())
        else {
            return;
        };
        let field = cx.new(|cx| InputState::new(window, cx).placeholder(say!("pulls-title")));
        field.update(cx, |state, cx| {
            state.set_value(shown_line(&title, 1024), window, cx);
            state.focus(window, cx);
        });
        let saved = cx.subscribe_in(&field, window, |this, _, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.save_edit(window, cx);
            }
        });
        self.edit_subscription = Some(saved);
        self.editing = Some(Editing::Title(field));
        cx.notify();
    }

    /// Edit the description in place.
    pub fn edit_body(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(body) = self.details.as_ref().map(|details| details.body.clone()) else {
            return;
        };
        let field = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(say!("pulls-description-placeholder"))
                .rows(10)
        });
        field.update(cx, |state, cx| {
            state.set_value(body, window, cx);
            state.focus(window, cx);
        });
        self.edit_subscription = None;
        self.editing = Some(Editing::Body(field));
        cx.notify();
    }

    /// Put the field away without changing anything.
    pub fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.editing = None;
        self.edit_subscription = None;
        cx.notify();
    }

    /// Send what the field holds.
    pub fn save_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match &self.editing {
            Some(Editing::Title(field)) => {
                let title = field.read(cx).value().trim().to_owned();
                if title.is_empty() {
                    self.write_error = Some(say!("pulls-title-needed").to_owned());
                    cx.notify();
                    return;
                }
                self.change(
                    json!({ "title": title }),
                    say!("pulls-changing-title"),
                    window,
                    cx,
                );
            }
            Some(Editing::Body(field)) => {
                let body = field.read(cx).value().to_string();
                self.change(
                    json!({ "body": body }),
                    say!("pulls-changing-description"),
                    window,
                    cx,
                );
            }
            None => {}
        }
    }

    /// What can be chosen, read once for the pencils.
    pub(super) fn read_options(&mut self, cx: &mut Context<Self>) {
        if self.options.is_some() || self.options_asked {
            return;
        }
        self.options_asked = true;
        let asked = calls::ask::<PullOptions>(
            "pr.options",
            json!({
                "account": self.link.account,
                "owner": self.link.owner,
                "name": self.link.name,
            }),
        );
        cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                match answer {
                    Ok(options) => this.options = Some(options),
                    Err(error) => {
                        this.options_asked = false;
                        this.write_error = Some(say!("pulls-choices-unread-why", error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Pick or drop a reviewer, an assignee or a label, and send the list
    /// as it then is; or pick a milestone (none takes it away).
    pub fn toggle_listed(
        &mut self,
        listed: Listed,
        name: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(details) = &self.details else {
            return;
        };
        let pull = &details.pull;
        let flip = |mut names: Vec<String>| {
            match names.iter().position(|known| known == name) {
                Some(at) => {
                    names.remove(at);
                }
                None => names.push(name.to_owned()),
            }
            names
        };
        let (change, what) = match listed {
            Listed::Reviewers => (
                json!({ "reviewers": flip(pull.reviewers.iter().map(|r| r.user.login.clone()).collect()) }),
                say!("pulls-changing-reviewers"),
            ),
            Listed::Assignees => (
                json!({ "assignees": flip(pull.assignees.iter().map(|u| u.login.clone()).collect()) }),
                say!("pulls-changing-assignees"),
            ),
            Listed::Labels => (
                json!({ "labels": flip(pull.labels.iter().map(|l| l.name.clone()).collect()) }),
                say!("pulls-changing-labels"),
            ),
            Listed::Milestone => (
                json!({ "milestone": name }),
                say!("pulls-changing-milestone"),
            ),
        };
        self.change(change, what, window, cx);
    }

    /// What is chosen now in a list of the right column, by what is sent.
    fn chosen(&self, listed: Listed) -> Vec<String> {
        let Some(details) = &self.details else {
            return Vec::new();
        };
        let pull = &details.pull;
        match listed {
            Listed::Reviewers => pull
                .reviewers
                .iter()
                .map(|r| r.user.login.clone())
                .collect(),
            Listed::Assignees => pull.assignees.iter().map(|u| u.login.clone()).collect(),
            Listed::Labels => pull.labels.iter().map(|l| l.name.clone()).collect(),
            Listed::Milestone => details.milestone.iter().map(|m| m.id.clone()).collect(),
        }
    }

    /// What can be chosen in a list, (what is sent, what is shown); none
    /// while it has not been read.
    fn choices(&self, listed: Listed) -> Option<Vec<(String, String)>> {
        self.options.as_ref().map(|options| match listed {
            Listed::Reviewers | Listed::Assignees => options
                .people
                .iter()
                .map(|user| (user.login.clone(), shown_line(&user.login, 60)))
                .collect(),
            Listed::Labels => options
                .labels
                .iter()
                .map(|label| (label.name.clone(), shown_line(&label.name, 60)))
                .collect(),
            Listed::Milestone => std::iter::once((String::new(), say!("pulls-none").to_owned()))
                .chain(
                    options
                        .milestones
                        .iter()
                        .map(|m| (m.id.clone(), shown_line(&m.title, 60))),
                )
                .collect(),
        })
    }

    /// The pencil of a part of the right column: a list of what can be
    /// chosen, the chosen ones ticked.
    pub(super) fn pencil(&self, listed: Listed, cx: &mut Context<Self>) -> AnyElement {
        if self.details.is_none() {
            return div().into_any_element();
        }
        let view = cx.entity().downgrade();
        let busy = self.sending.is_some();
        Button::new(ElementId::Name(
            format!("pull-edit-{}", listed.key()).into(),
        ))
        .accessibility_label(say!("pulls-change"))
        .ghost()
        .xsmall()
        .icon(Icon::new(gpui_kit::assets::IconName::Pencil))
        .disabled(busy)
        .on_click(cx.listener(|this, _, _, cx| this.read_options(cx)))
        .dropdown_menu(move |menu: PopupMenu, window, cx| {
            listed_menu(menu, listed, &view, true, window, cx)
        })
        .into_any_element()
    }
}

/// The entries of a pencil's list, from what the view knows as the list
/// is opened. While what can be chosen is being read the list says so,
/// and fills in once when the answer comes, without being closed and
/// opened again (a list once open is not built again).
/// `opening` is the first building of the list, which asks what can be
/// chosen when that is not known yet; filling it in does not ask again.
fn listed_menu(
    menu: PopupMenu,
    listed: Listed,
    view: &WeakEntity<PullView>,
    opening: bool,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let menu = menu.scrollable(true).max_h(px(MENU_HEIGHT));
    let Some(this) = view.upgrade() else {
        return menu;
    };
    if opening {
        this.update(cx, |this, cx| this.read_options(cx));
    }
    let (choices, chosen, asking) = {
        let this = this.read(cx);
        (
            this.choices(listed),
            this.chosen(listed),
            this.options_asked,
        )
    };
    let Some(choices) = choices else {
        if !asking {
            // Asking failed; the view says why.
            return menu.item(PopupMenuItem::new(say!("pulls-choices-unread")).disabled(true));
        }
        let mut filled = false;
        cx.observe_in(&this, window, move |menu, this, window, cx| {
            let answered = {
                let this = this.read(cx);
                this.options.is_some() || !this.options_asked
            };
            if filled || !answered {
                return;
            }
            filled = true;
            let view = this.downgrade();
            menu.rebuild(window, cx, |menu, window, cx| {
                listed_menu(menu, listed, &view, false, window, cx)
            });
        })
        .detach();
        return menu.item(PopupMenuItem::new(say!("pulls-reading-choices")).disabled(true));
    };
    if choices.is_empty() {
        return menu.item(PopupMenuItem::new(say!("pulls-nothing-to-choose")).disabled(true));
    }
    let mut menu = menu;
    for (sent, shown) in choices.into_iter().take(MOST_LISTED) {
        let picked = if listed == Listed::Milestone && sent.is_empty() {
            chosen.is_empty()
        } else {
            chosen.contains(&sent)
        };
        let view = view.clone();
        menu = menu.item(PopupMenuItem::new(shown).checked(picked).on_click(
            move |_, window, cx| {
                let _ = view.update(cx, |this, cx| this.toggle_listed(listed, &sent, window, cx));
            },
        ));
    }
    menu
}
