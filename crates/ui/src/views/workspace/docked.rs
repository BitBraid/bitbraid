//! The pane that docks at the right edge of the references panel, over
//! the graph, for what is added to a repository from the panel: a
//! remote, a worktree and a submodule. Each is a form of fields one under
//! the other, each field under the word that says what goes in it, and
//! one button that goes ahead; the graph stays in sight to the right of
//! it.
//!
//! The pane is 500 wide for a remote, 600 for a worktree and 400 for a
//! submodule at every window size, from the top of the centre to its
//! bottom; a heading row with the pane's icon, its title and a cross that
//! puts it away; the fields 34 high and 20 from the pane's edges; a
//! remote's button as wide as the fields, the others' at the left. Escape
//! puts the pane away, Enter goes ahead, Tab goes to the next field.
//!
//! Sources: git-remote(1) (`add`, `set-url --push`); git-worktree(1)
//! (`add`, `-b`, `--detach`); git-submodule(1) (`add`, `--branch`).

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use super::{Workspace, form::Verdict, modules::Adding, worktrees};
use crate::{
    notices::Notice,
    session::{Reply, Request},
};

/// From the pane's edges to its fields.
const SIDE: f32 = 20.;
/// The heading row and its icon.
const HEADING_HEIGHT: f32 = 58.;
const HEADING_ICON: f32 = 18.;
/// From the heading row to the first word.
const FIRST_TOP: f32 = 2.;
/// A word over its field, the space between the two, the field, and the
/// space from a field to the next word.
const WORD_HEIGHT: f32 = 20.;
const WORD_GAP: f32 = 6.;
const FIELD_HEIGHT: f32 = 34.;
const FIELD_GAP: f32 = 15.;
/// The button that goes ahead, and the one beside the folder.
const BUTTON_HEIGHT: f32 = 30.;
const BROWSE_WIDTH: f32 = 70.;
/// Most references the list under the worktree's first field offers.
const MOST_OFFERED: usize = 200;

/// What the pane adds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Adds {
    Remote,
    /// A worktree, starting from this when it is given.
    Worktree(Option<String>),
    Submodule,
}

impl Adds {
    /// The word for it in the probe and in element ids.
    pub(crate) fn key(&self) -> &'static str {
        match self {
            Adds::Remote => "remote",
            Adds::Worktree(_) => "worktree",
            Adds::Submodule => "submodule",
        }
    }

    fn width(&self) -> f32 {
        match self {
            Adds::Remote => 500.,
            Adds::Worktree(_) => 600.,
            Adds::Submodule => 400.,
        }
    }

    fn title(&self) -> &'static str {
        match self {
            Adds::Remote => say!("dialogs-add-remote-title"),
            Adds::Worktree(_) => say!("dialogs-new-worktree-title"),
            Adds::Submodule => say!("dialogs-add-submodule-title"),
        }
    }

    fn icon(&self) -> Icon {
        Icon::new(match self {
            Adds::Remote => gpui_kit::assets::IconName::Cloud,
            Adds::Worktree(_) => gpui_kit::assets::IconName::FolderTree,
            Adds::Submodule => gpui_kit::assets::IconName::Package,
        })
    }

    fn ok_text(&self) -> &'static str {
        match self {
            Adds::Remote => say!("dialogs-add-and-fetch"),
            Adds::Worktree(_) => say!("dialogs-add-worktree"),
            Adds::Submodule => say!("dialogs-add-submodule"),
        }
    }

    /// Each field: the word over it, what it says while it is empty.
    fn fields(&self) -> [(&'static str, &'static str); 3] {
        match self {
            Adds::Remote => [
                (
                    say!("dialogs-remote-name"),
                    say!("dialogs-remote-name-placeholder"),
                ),
                (
                    say!("workspace-remote-url"),
                    say!("dialogs-remote-url-placeholder"),
                ),
                (
                    say!("dialogs-remote-push-url"),
                    say!("dialogs-remote-push-url-placeholder"),
                ),
            ],
            Adds::Worktree(_) => [
                (say!("dialogs-worktree-check-out"), "HEAD"),
                (
                    say!("dialogs-worktree-new-branch"),
                    say!("dialogs-worktree-new-branch-placeholder"),
                ),
                (say!("dialogs-folder"), ""),
            ],
            Adds::Submodule => [
                (
                    say!("dialogs-submodule-repository"),
                    say!("dialogs-submodule-repository-placeholder"),
                ),
                (
                    say!("dialogs-folder"),
                    say!("dialogs-submodule-folder-placeholder"),
                ),
                (
                    say!("dialogs-submodule-branch"),
                    say!("dialogs-submodule-branch-placeholder"),
                ),
            ],
        }
    }
}

/// The pane that is open.
pub(crate) struct Docked {
    adds: Adds,
    fields: Vec<Entity<InputState>>,
    /// The folder the worktree's field was given, while nobody changed
    /// it: it follows the branch then.
    suggested: String,
    /// The first field is to have the keyboard when the pane is drawn.
    focus_first: bool,
    /// Something was typed, or going ahead was tried: until then what
    /// will not do is not said yet.
    touched: bool,
    /// For a remote: the tab of each connected account (`forking.rs`).
    pub(super) fork: super::forking::Fork,
    _subscriptions: Vec<Subscription>,
}

/// The fields of a worktree's form, by their place.
const CHECK_OUT: usize = 0;
const NEW_BRANCH: usize = 1;
const FOLDER: usize = 2;

impl Workspace {
    /// Open the pane for what is to be added, in place of the one that is
    /// open; the first field has the keyboard.
    pub(super) fn dock(&mut self, adds: Adds, window: &mut Window, cx: &mut Context<Self>) {
        let mut initial = [String::new(), String::new(), String::new()];
        let mut suggested = String::new();
        if let Adds::Worktree(start) = &adds {
            suggested = worktrees::suggested_folder(&self.main_folder(), start.as_deref());
            initial = [
                start.clone().unwrap_or_default(),
                String::new(),
                suggested.clone(),
            ];
        }
        let mut fields = Vec::new();
        let mut subscriptions = Vec::new();
        for ((_, placeholder), initial) in adds.fields().into_iter().zip(initial) {
            let field = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(initial)
            });
            subscriptions.push(cx.subscribe_in(
                &field,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::PressEnter { .. } => this.docked_go(window, cx),
                    InputEvent::Change => {
                        if let Some(docked) = this.docked.as_mut() {
                            docked.touched = true;
                        }
                        this.docked_follow(window, cx);
                        cx.notify();
                    }
                    _ => {}
                },
            ));
            fields.push(field);
        }
        self.docked = Some(Docked {
            adds,
            fields,
            suggested,
            focus_first: true,
            touched: false,
            fork: Default::default(),
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Give the pane's first field the keyboard as the pane is drawn the
    /// first time: given any earlier, while what asked for the pane still
    /// has it, it would be taken back.
    pub(super) fn focus_docked(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(docked) = self.docked.as_mut().filter(|docked| docked.focus_first) else {
            return;
        };
        docked.focus_first = false;
        if let Some(first) = docked.fields.first().cloned() {
            first.update(cx, |state, cx| state.focus(window, cx));
        }
    }

    /// Put the pane away; the graph has the keyboard again.
    pub(super) fn undock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.docked.take().is_some() {
            self.focus_graph(window, cx);
            cx.notify();
        }
    }

    /// What the pane adds, while it is open.
    pub(super) fn docked_adds(&self) -> Option<&Adds> {
        self.docked.as_ref().map(|docked| &docked.adds)
    }

    /// What the pane's fields hold, in order, trimmed.
    pub(super) fn docked_texts(&self, cx: &App) -> Vec<String> {
        self.docked
            .as_ref()
            .map(|docked| {
                docked
                    .fields
                    .iter()
                    .map(|field| field.read(cx).value().trim().to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    fn docked_text(&self, ix: usize, cx: &App) -> String {
        self.docked_texts(cx).get(ix).cloned().unwrap_or_default()
    }

    fn set_docked_text(&mut self, ix: usize, text: String, window: &mut Window, cx: &mut App) {
        if let Some(field) = self
            .docked
            .as_ref()
            .and_then(|docked| docked.fields.get(ix))
            .cloned()
        {
            field.update(cx, |state, cx| state.set_value(text, window, cx));
        }
    }

    /// A worktree's folder follows its branch while nobody changed it.
    fn docked_follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Docked {
            adds: Adds::Worktree(_),
            suggested,
            ..
        }) = &self.docked
        else {
            return;
        };
        let suggested = suggested.clone();
        if self.docked_text(FOLDER, cx) != suggested {
            return;
        }
        let branch = self.docked_text(NEW_BRANCH, cx);
        let start = self.docked_text(CHECK_OUT, cx);
        let named = [branch, start].into_iter().find(|name| !name.is_empty());
        let wanted = worktrees::suggested_folder(&self.main_folder(), named.as_deref());
        if wanted != suggested {
            if let Some(docked) = self.docked.as_mut() {
                docked.suggested = wanted.clone();
            }
            self.set_docked_text(FOLDER, wanted, window, cx);
        }
    }

    /// The references a worktree can check out: the local branches, then
    /// the remotes' branches, then the tags.
    fn checkout_names(&self) -> Vec<String> {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return Vec::new();
        };
        snapshot
            .branches
            .iter()
            .map(|branch| branch.name.clone())
            .chain(snapshot.remotes.iter().flat_map(|remote| {
                remote
                    .branches
                    .iter()
                    .map(move |branch| format!("{}/{}", remote.name, branch.name))
            }))
            .chain(snapshot.tags.iter().map(|tag| tag.name.clone()))
            .take(MOST_OFFERED)
            .collect()
    }

    /// The local branches checked out in a worktree.
    fn checked_out_branches(&self) -> Vec<String> {
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .worktrees
                    .iter()
                    .filter_map(|worktree| worktree.branch.as_deref())
                    .map(|branch| branch.trim_start_matches("refs/heads/").to_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What there is to say about what the pane holds: a refusal keeps it
    /// open, a remark is worth knowing.
    fn docked_verdict(&self, cx: &App) -> Option<Verdict> {
        let adds = self.docked_adds()?;
        let texts = self.docked_texts(cx);
        let text = |ix: usize| texts.get(ix).cloned().unwrap_or_default();
        match adds {
            Adds::Remote => {
                let name = text(0);
                if name.chars().any(char::is_whitespace) || name.starts_with('-') {
                    Some(Verdict::Refuse(say!(
                        "dialogs-remote-name-refused",
                        name = format!("{name:?}")
                    )))
                } else if self.remote_names().contains(&name) {
                    Some(Verdict::Refuse(say!("dialogs-remote-exists", name)))
                } else {
                    None
                }
            }
            Adds::Worktree(_) => {
                let (start, branch) = (text(CHECK_OUT), text(NEW_BRANCH));
                let locals = self.local_branch_names();
                if !branch.is_empty() && locals.contains(&branch) {
                    Some(Verdict::Refuse(say!(
                        "dialogs-branch-exists",
                        name = branch.as_str()
                    )))
                } else if branch.is_empty() && start.is_empty() {
                    Some(Verdict::Refuse(
                        say!("dialogs-worktree-needs-branch").to_owned(),
                    ))
                } else if branch.is_empty() && self.checked_out_branches().contains(&start) {
                    Some(Verdict::Refuse(say!(
                        "dialogs-worktree-checked-out-already",
                        start = start.as_str()
                    )))
                } else if branch.is_empty() && !locals.contains(&start) {
                    Some(Verdict::Remark(say!(
                        "dialogs-worktree-detached",
                        start = start.as_str()
                    )))
                } else {
                    None
                }
            }
            Adds::Submodule => {
                let adding = submodule_adding(&texts);
                let there = self.submodule_paths();
                match adding.reviewed(&there) {
                    Err(refusal) => Some(Verdict::Refuse(refusal)),
                    Ok(Some(remark)) => Some(Verdict::Remark(remark)),
                    Ok(None) => None,
                }
            }
        }
    }

    /// Go ahead with what the pane holds, when it will do: the pane goes
    /// away and the work begins. Else the pane stays as it is.
    pub(super) fn docked_go(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(adds) = self.docked_adds().cloned() else {
            return;
        };
        // An account's tab goes ahead with its own buttons.
        if self
            .docked
            .as_ref()
            .is_some_and(|docked| docked.fork.account().is_some())
        {
            return;
        }
        if let Some(docked) = self.docked.as_mut() {
            docked.touched = true;
        }
        cx.notify();
        if matches!(self.docked_verdict(cx), Some(Verdict::Refuse(_))) {
            return;
        }
        let texts = self.docked_texts(cx);
        let text = |ix: usize| texts.get(ix).cloned().unwrap_or_default();
        match adds {
            Adds::Remote => {
                let (name, url, push) = (text(0), text(1), text(2));
                if name.is_empty() || url.is_empty() {
                    return;
                }
                self.undock(window, cx);
                self.add_remote_with_push(name, url, push, window, cx);
            }
            Adds::Worktree(_) => {
                let (start, branch, folder) = (text(CHECK_OUT), text(NEW_BRANCH), text(FOLDER));
                if folder.is_empty() {
                    return;
                }
                let path = crate::shell::expand_home(&folder).display().to_string();
                let start = Some(start).filter(|start| !start.is_empty());
                let local = start
                    .as_ref()
                    .is_some_and(|start| self.local_branch_names().contains(start));
                let params = match (&start, branch.is_empty()) {
                    (_, false) => json!({ "path": path, "new_branch": branch, "start": start }),
                    (Some(start), true) if local => json!({ "path": path, "branch": start }),
                    (Some(start), true) => json!({ "path": path, "detach": true, "start": start }),
                    (None, true) => return,
                };
                self.undock(window, cx);
                self.run_operation(
                    "worktree.add",
                    params,
                    say!("dialogs-worktree-adding"),
                    say!(
                        "dialogs-worktree-added",
                        name = worktrees::folder_name(&path)
                    ),
                    window,
                    cx,
                );
            }
            Adds::Submodule => {
                let adding = submodule_adding(&texts);
                if adding.url.is_empty() || adding.path.is_empty() {
                    return;
                }
                self.undock(window, cx);
                self.add_submodule(adding, false, window, cx);
            }
        }
    }

    /// Add a remote, point its pushes elsewhere when a push URL is given,
    /// and fetch it once that worked.
    pub(super) fn add_remote_with_push(
        &mut self,
        name: String,
        url: String,
        push: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let added = self.session.send(Request::Command {
            id: "remote.add".to_owned(),
            params: json!({ "name": name, "url": url }),
        });
        let session = self.session.clone();
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = added.await else {
                return;
            };
            let mut failed = match reply {
                Reply::Error(err) => Some(err),
                _ => None,
            };
            if failed.is_none() && !push.is_empty() && push != url {
                let pointed = session.send(Request::Command {
                    id: "remote.set_url".to_owned(),
                    params: json!({ "name": name, "url": push, "push": true }),
                });
                if let Ok(Reply::Error(err)) = pointed.await {
                    failed = Some(err);
                }
            }
            let _ = this.update_in(cx, |this, window, cx| match failed {
                Some(err) => {
                    this.reload(cx);
                    crate::notices::push(window, cx, Notice::error(super::sentence(&err)));
                }
                None => {
                    this.reload(cx);
                    this.remote(super::RemoteRequest::Fetch(name.clone()), window, cx);
                }
            });
        })
        .detach();
    }

    /// Choose the worktree's folder in the desktop's chooser.
    fn browse_docked_folder(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(say!("dialogs-worktree-folder-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.set_docked_text(FOLDER, path.display().to_string(), window, cx);
                });
            }
        })
        .detach();
    }

    /// The pane, when one is open: it lies over the left edge of the
    /// centre, from its top to its bottom.
    pub(super) fn render_docked(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let docked = self.docked.as_ref()?;
        let theme = cx.theme().clone();
        let adds = docked.adds.clone();
        // A remark from the start; a refusal once something was typed or
        // tried.
        let verdict = self
            .docked_verdict(cx)
            .filter(|verdict| docked.touched || matches!(verdict, Verdict::Remark(_)));
        let names = match adds {
            Adds::Worktree(_) => self.checkout_names(),
            _ => Vec::new(),
        };
        let wide_button = adds == Adds::Remote;
        let tabs = (adds == Adds::Remote)
            .then(|| self.docked_tabs(cx))
            .flatten();
        let fork_body = (adds == Adds::Remote).then(|| self.fork_body(cx)).flatten();
        let mut body = v_flex()
            .w_full()
            .px(px(SIDE))
            .pt(px(FIRST_TOP))
            .gap(px(FIELD_GAP));
        for (ix, ((word, _), field)) in adds.fields().into_iter().zip(&docked.fields).enumerate() {
            let input = Input::new(field)
                .id(("docked-field", ix))
                .min_h(px(FIELD_HEIGHT))
                .w_full();
            let input = if matches!(adds, Adds::Worktree(_)) && ix == CHECK_OUT {
                let (field, names) = (field.clone(), names.clone());
                input.suffix(
                    Button::new("docked-refs")
                        .accessibility_label(say!("dialogs-branches-and-tags"))
                        .ghost()
                        .xsmall()
                        .tab_stop(false)
                        .icon(IconName::ChevronDown)
                        .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                            names.iter().fold(
                                menu.scrollable(true).max_h(px(320.)),
                                |menu, name| {
                                    let (field, name) = (field.clone(), name.clone());
                                    menu.item(PopupMenuItem::new(name.clone()).on_click(
                                        move |_, window, cx| {
                                            let name = name.clone();
                                            field.update(cx, |state, cx| {
                                                state.set_value(name, window, cx)
                                            });
                                        },
                                    ))
                                },
                            )
                        }),
                )
            } else {
                input
            };
            let control = if matches!(adds, Adds::Worktree(_)) && ix == FOLDER {
                h_flex()
                    .w_full()
                    .gap(px(6.))
                    .items_center()
                    .child(div().flex_1().min_w_0().child(input))
                    .child(
                        Button::new("docked-browse")
                            .outline()
                            .small()
                            .tab_stop(false)
                            .w(px(BROWSE_WIDTH))
                            .h(px(BUTTON_HEIGHT))
                            .label(say!("dialogs-browse"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.browse_docked_folder(window, cx)
                            })),
                    )
                    .into_any_element()
            } else {
                input.into_any_element()
            };
            body = body.child(
                v_flex()
                    .w_full()
                    .gap(px(WORD_GAP))
                    .child(
                        div()
                            .h(px(WORD_HEIGHT))
                            .line_height(px(WORD_HEIGHT))
                            .text_sm()
                            .child(word),
                    )
                    .child(control),
            );
        }
        if let Some(verdict) = verdict {
            let (text, colour) = match verdict {
                Verdict::Refuse(text) => (text, theme.danger),
                Verdict::Remark(text) => (text, theme.muted_foreground),
            };
            body = body.child(
                div()
                    .id("docked-said")
                    .text_xs()
                    .whitespace_normal()
                    .text_color(colour)
                    .child(text)
                    .test_support(),
            );
        }
        body = body.child(
            h_flex().w_full().child(
                Button::new("docked-ok")
                    .primary()
                    .small()
                    .h(px(BUTTON_HEIGHT))
                    .when(wide_button, |button| button.w_full())
                    .label(adds.ok_text())
                    .on_click(cx.listener(|this, _, window, cx| this.docked_go(window, cx))),
            ),
        );
        Some(
            v_flex()
                .id(SharedString::from(format!("docked-{}", adds.key())))
                .absolute()
                .top_0()
                .left_0()
                .bottom_0()
                .w(px(adds.width()))
                .max_w_full()
                .bg(theme.sidebar)
                .border_r_1()
                .border_color(theme.border)
                .occlude()
                // Tab is the window's: from one field to the next, the
                // buttons beside them left out.
                .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    if event.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.undock(window, cx);
                    }
                }))
                .child(
                    h_flex()
                        .w_full()
                        .h(px(HEADING_HEIGHT))
                        .flex_none()
                        .pl(px(SIDE))
                        .pr(px(17.))
                        .gap(px(8.))
                        .items_center()
                        .child(
                            adds.icon()
                                .size(px(HEADING_ICON))
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_lg()
                                .child(adds.title()),
                        )
                        .child(
                            Button::new("docked-close")
                                .accessibility_label(say!("workspace-close"))
                                .ghost()
                                .small()
                                .tab_stop(false)
                                .icon(IconName::Close)
                                .tooltip(say!("workspace-close"))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.undock(window, cx)),
                                ),
                        ),
                )
                .children(tabs)
                .child(match fork_body {
                    Some(fork_body) => fork_body,
                    None => body.into_any_element(),
                })
                .test_support()
                .into_any_element(),
        )
    }
}

/// A submodule as the pane holds it: where it comes from, the folder it
/// goes to, the branch it follows.
fn submodule_adding(texts: &[String]) -> Adding {
    let text = |ix: usize| texts.get(ix).cloned().unwrap_or_default();
    Adding {
        url: text(0),
        path: text(1).trim_end_matches('/').to_owned(),
        branch: text(2),
    }
}
