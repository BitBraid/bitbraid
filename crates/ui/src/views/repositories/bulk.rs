//! What is done to many repositories of a workspace at once: fetch, pull
//! with the way of pulling chosen at the heading, clone the missing ones.
//! Ticking rows turns a workspace's heading into a bar whose buttons name
//! how many they act on (fetch, pull, open in tabs, open in the editor,
//! locate, clone, remove); without a tick the heading's buttons act on
//! every repository.
//!
//! The engine runs the command on each repository in a context of its
//! own, a few at a time (`workspace.fetch`, `workspace.pull`,
//! `workspace.clone_missing`), and says where each one is through its
//! progress (`RepositoryOutcome::from_progress`). Each row shows that:
//! waiting, what git is doing and how far, then how it ended. One that
//! failed says why and keeps saying it until its cross takes it away; one
//! that ended well says so until the page is shown again. A stop button
//! stops what has not started and asks what runs to stop.
//!
//! Sources: none (original design).

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use bitbraid_engine::workspace::{BulkOutcome, RepositoryOutcome, RepositoryState};
use futures::StreamExt as _;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    spinner::Spinner,
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::{ROW_BUTTON, ROW_BUTTON_GAP, ROW_RIGHT, RepositoriesView, chip, workspaces::Listed};
use crate::{
    notices::Notice,
    session::{Reply, run_standalone},
    zoom::px,
};

/// The widest a failure is said on its row; the rest is under the
/// pointer.
const SAID_WIDTH: f32 = 420.;

/// What is done to many repositories at once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Fetch from their remotes.
    Fetch,
    /// Pull what they follow, the way the heading says.
    Pull,
    /// Clone the ones that are not there.
    Clone,
}

impl Kind {
    /// The engine's command.
    fn command(self) -> &'static str {
        match self {
            Kind::Fetch => "workspace.fetch",
            Kind::Pull => "workspace.pull",
            Kind::Clone => "workspace.clone_missing",
        }
    }

    /// What a row says when its repository's command ended well.
    fn done(self) -> &'static str {
        match self {
            Kind::Fetch => say!("repositories-bulk-fetched"),
            Kind::Pull => say!("repositories-bulk-pulled"),
            Kind::Clone => say!("repositories-bulk-cloned"),
        }
    }

    /// Which it is, as the catalog's messages choose their words by it.
    fn selector(self) -> &'static str {
        match self {
            Kind::Fetch => "fetch",
            Kind::Pull => "pull",
            Kind::Clone => "clone",
        }
    }
}

/// A command running on many repositories of one workspace.
pub(super) struct Run {
    kind: Kind,
    /// Set to stop it.
    cancel: Arc<AtomicBool>,
    /// How many repositories it was given.
    total: usize,
}

impl RepositoriesView {
    /// Run a command on repositories of a workspace, each row saying
    /// where its repository is. One command at a time in a workspace.
    pub fn start_bulk(
        &mut self,
        id: &str,
        kind: Kind,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() || self.runs.contains_key(id) {
            return;
        }
        let mut params = json!({ "id": id, "paths": paths });
        if kind == Kind::Pull
            && let Some(object) = params.as_object_mut()
        {
            object.insert("mode".to_owned(), super::workspaces::pull_mode(cx).into());
        }
        for path in &paths {
            self.outcomes.insert(
                (id.to_owned(), path.clone()),
                RepositoryOutcome::queued(path),
            );
        }
        let crate::session::Standalone {
            reply,
            mut progress,
            cancel,
        } = run_standalone(kind.command(), params);
        self.bulk_kinds.insert(id.to_owned(), kind);
        self.runs.insert(
            id.to_owned(),
            Run {
                kind,
                cancel,
                total: paths.len(),
            },
        );
        cx.notify();
        let id = id.to_owned();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(step) = progress.next().await {
                let Some(outcome) = RepositoryOutcome::from_progress(&step) else {
                    continue;
                };
                let alive = this.update(cx, |this, cx| {
                    this.outcomes
                        .insert((id.clone(), outcome.path.clone()), outcome);
                    cx.notify();
                });
                if alive.is_err() {
                    return;
                }
            }
            let reply = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.runs.remove(&id);
                match reply {
                    Ok(Reply::Command(value)) => match serde_json::from_value::<BulkOutcome>(value)
                    {
                        Ok(outcome) => this.bulk_ended(&id, kind, outcome, window, cx),
                        Err(err) => crate::notices::push(
                            window,
                            cx,
                            Notice::error(say!(
                                "repositories-answer-not-understood",
                                error = err.to_string()
                            )),
                        ),
                    },
                    Ok(Reply::Error(err)) => {
                        // Nothing ran: no row is waiting any more.
                        this.outcomes.retain(|(workspace, _), outcome| {
                            *workspace != id || outcome.state.is_final()
                        });
                        crate::notices::push(
                            window,
                            cx,
                            Notice::error(super::workspaces::sentence(&err)),
                        );
                    }
                    _ => {}
                }
                this.look(window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// Take in how each repository ended, and say how it went.
    pub(super) fn bulk_ended(
        &mut self,
        id: &str,
        kind: Kind,
        outcome: BulkOutcome,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for repository in &outcome.repositories {
            self.outcomes
                .insert((id.to_owned(), repository.path.clone()), repository.clone());
        }
        // A row that never heard of its end did not run.
        self.outcomes
            .retain(|(workspace, _), one| *workspace != id || one.state.is_final());
        let name = self
            .workspace(id)
            .map(|listed| listed.workspace.name.clone())
            .unwrap_or_else(|| id.to_owned());
        let all = outcome.repositories.len();
        let notice = if outcome.stopped {
            Notice::info(say!(
                "repositories-bulk-stopped",
                done = outcome.done,
                all,
                name = name.as_str()
            ))
        } else if outcome.failed > 0 || outcome.skipped > 0 {
            Notice::warning(say!(
                "repositories-bulk-partly",
                done = outcome.done,
                all,
                name = name.as_str(),
                failed = outcome.failed.saturating_add(outcome.skipped)
            ))
        } else {
            Notice::success(say!(
                "repositories-bulk-went-well",
                kind = kind.selector(),
                all,
                name = name.as_str()
            ))
        };
        crate::notices::push(window, cx, notice);
    }

    /// Show where each of these repositories is, as a command on many
    /// would have said it, in place of what the workspace's rows said,
    /// and, with `running`, a command still at work on them: what a
    /// picture of the page starts from, without git.
    pub fn show_outcomes(
        &mut self,
        id: &str,
        kind: Kind,
        outcomes: Vec<RepositoryOutcome>,
        running: bool,
        cx: &mut Context<Self>,
    ) {
        self.outcomes.retain(|(workspace, _), _| workspace != id);
        self.runs.remove(id);
        if running {
            self.runs.insert(
                id.to_owned(),
                Run {
                    kind,
                    cancel: Arc::new(AtomicBool::new(false)),
                    total: outcomes.len(),
                },
            );
        }
        self.bulk_kinds.insert(id.to_owned(), kind);
        for outcome in outcomes {
            self.outcomes
                .insert((id.to_owned(), outcome.path.clone()), outcome);
        }
        cx.notify();
    }

    /// Stop the command running on a workspace's repositories.
    pub fn stop_bulk(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(run) = self.runs.get(id) {
            run.cancel.store(true, Ordering::SeqCst);
        }
        cx.notify();
    }

    /// Whether a command runs on a workspace's repositories.
    pub fn is_running(&self, id: &str) -> bool {
        self.runs.contains_key(id)
    }

    /// How a repository's command ended, or where it is, as the page
    /// has it.
    pub fn outcome(&self, id: &str, path: &str) -> Option<&RepositoryOutcome> {
        self.outcomes.get(&(id.to_owned(), path.to_owned()))
    }

    /// Take away what a row says of how its repository's command ended.
    pub fn dismiss(&mut self, id: &str, path: &str, cx: &mut Context<Self>) {
        self.outcomes.remove(&(id.to_owned(), path.to_owned()));
        cx.notify();
    }

    /// What ended well is not said any more once the page is shown again;
    /// what failed is, until its cross is clicked.
    pub(super) fn forget_done(&mut self) {
        let runs = &self.runs;
        self.outcomes.retain(|(workspace, _), outcome| {
            runs.contains_key(workspace) || outcome.state != RepositoryState::Done
        });
    }

    /// What a row says of its repository's command: where it is, or how
    /// it ended; none when it has not been given one.
    pub(super) fn outcome_chips(
        &self,
        id: &str,
        path: &str,
        key: &str,
        cx: &mut Context<Self>,
    ) -> Option<Vec<AnyElement>> {
        let outcome = self.outcomes.get(&(id.to_owned(), path.to_owned()))?;
        let theme = cx.theme().clone();
        let kind = self.bulk_kinds.get(id).copied().unwrap_or(Kind::Fetch);
        let state = chip("state", key, cx);
        let mut chips = Vec::new();
        match outcome.state {
            RepositoryState::Queued => chips.push(
                state
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(gpui_kit::assets::IconName::Clock).size(px(11.)))
                    .child(say!("repositories-bulk-waiting"))
                    .into_any_element(),
            ),
            RepositoryState::Running => {
                let said = match (&outcome.progress, outcome.percent) {
                    (Some(line), Some(percent)) => format!("{line} {percent}%"),
                    (Some(line), None) => line.clone(),
                    (None, _) => say!("repositories-bulk-starting", kind = kind.selector()),
                };
                chips.push(
                    state
                        .child(Spinner::new().xsmall())
                        .child(div().max_w(px(SAID_WIDTH)).truncate().child(said))
                        .into_any_element(),
                );
            }
            RepositoryState::Done => {
                let done = outcome
                    .output
                    .as_ref()
                    .and_then(said_of)
                    .unwrap_or_else(|| kind.done().to_owned());
                chips.push(
                    state
                        .text_color(theme.success)
                        .child(Icon::new(IconName::CircleCheck).size(px(11.)))
                        .child(done)
                        .into_any_element(),
                );
                chips.extend(self.chips(key, std::path::Path::new(path), cx));
            }
            RepositoryState::Failed | RepositoryState::Skipped => {
                let error = outcome
                    .error
                    .clone()
                    .unwrap_or_else(|| say!("repositories-bulk-did-not-run").to_owned());
                let shown = super::workspaces::sentence(&error);
                let (page, id, path) = (cx.entity().downgrade(), id.to_owned(), path.to_owned());
                chips.push(
                    state
                        .text_color(theme.danger)
                        .bg(theme.danger.opacity(0.12))
                        .child(Icon::new(IconName::CircleX).size(px(11.)))
                        .child(div().max_w(px(SAID_WIDTH)).truncate().child(shown.clone()))
                        .tooltip(move |window, cx| Tooltip::new(shown.clone()).build(window, cx))
                        .into_any_element(),
                );
                chips.push(
                    Button::new(SharedString::from(format!("dismiss-{key}")))
                        .accessibility_label(say!("repositories-take-away"))
                        .ghost()
                        .xsmall()
                        .icon(Icon::new(IconName::Close).size(px(11.)))
                        .tooltip(say!("repositories-take-away"))
                        .on_click(move |_, _, cx| {
                            let _ = page.update(cx, |this, cx| this.dismiss(&id, &path, cx));
                        })
                        .into_any_element(),
                );
            }
        }
        Some(chips)
    }

    /// The bar over a workspace's section while rows are ticked or a
    /// command runs: its buttons name how many they act on.
    pub(super) fn bulk_bar(&self, listed: &Listed, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let id = listed.workspace.id.clone();
        if let Some(run) = self.runs.get(&id) {
            let ended = self
                .outcomes
                .iter()
                .filter(|((workspace, _), outcome)| *workspace == id && outcome.state.is_final())
                .count();
            let stopping = run.cancel.load(Ordering::SeqCst);
            let for_stop = id.clone();
            return h_flex()
                .flex_none()
                .gap(px(ROW_BUTTON_GAP))
                .pr(px(ROW_RIGHT))
                .items_center()
                .child(Spinner::new().xsmall())
                .child(
                    div()
                        .id(SharedString::from(format!("running-{id}")))
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(if stopping {
                            say!("repositories-bulk-stopping", ended, total = run.total)
                        } else {
                            say!(
                                "repositories-bulk-running",
                                kind = run.kind.selector(),
                                ended,
                                total = run.total
                            )
                        }),
                )
                .child(
                    Button::new(SharedString::from(format!("stop-{id}")))
                        .outline()
                        .danger()
                        .xsmall()
                        .icon(gpui_kit::assets::IconName::CircleStop)
                        .label(say!("repositories-bulk-stop"))
                        .disabled(stopping)
                        .on_click(cx.listener(move |this, _, _, cx| this.stop_bulk(&for_stop, cx))),
                )
                .into_any_element();
        }
        let ticked = self.ticked_in(&id);
        let count = ticked.len();
        let missing: Vec<String> = ticked
            .iter()
            .filter(|path| listed.missing.contains(path))
            .cloned()
            .collect();
        let clonable: Vec<String> = listed
            .workspace
            .repositories
            .iter()
            .filter(|repository| {
                missing.contains(&repository.path) && repository.remote_url.is_some()
            })
            .map(|repository| repository.path.clone())
            .collect();
        let button = |what: &str, label: String, icon: Icon| {
            Button::new(SharedString::from(format!("{what}-{id}")))
                .outline()
                .xsmall()
                .icon(icon)
                .label(label)
        };
        let (fetch_id, open_id, editor_id, locate_id, clone_id, remove_id, none_id) = (
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
            id.clone(),
        );
        let present = count.saturating_sub(missing.len());
        let first_missing = missing.first().cloned();
        h_flex()
            .flex_none()
            .gap(px(8.))
            .pr(px(ROW_RIGHT))
            .items_center()
            .child(
                button(
                    "fetch",
                    say!("repositories-bulk-fetch", count = present),
                    Icon::new(IconName::RefreshCw),
                )
                .disabled(present == 0)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let paths = this.present_ticked(&fetch_id);
                    this.start_bulk(&fetch_id, Kind::Fetch, paths, window, cx)
                })),
            )
            .child(self.pull_buttons(&id, Some(present), cx))
            .child(
                button(
                    "open-all",
                    say!("repositories-bulk-open", count = present),
                    Icon::new(IconName::ExternalLink),
                )
                .disabled(present == 0)
                .on_click(cx.listener(move |this, _, _, cx| {
                    let paths = this.present_ticked(&open_id);
                    this.open_workspace_repositories(&open_id, paths, cx)
                })),
            )
            .child(
                button(
                    "editor-all",
                    say!("repositories-bulk-editor", count = present),
                    Icon::new(gpui_kit::assets::IconName::CodeXml),
                )
                .tooltip(say!("repositories-bulk-editor-tip"))
                .disabled(present == 0)
                .on_click(cx.listener(move |this, _, window, cx| {
                    let paths = this
                        .present_ticked(&editor_id)
                        .into_iter()
                        .map(std::path::PathBuf::from)
                        .collect();
                    this.open_in_editor(paths, window, cx)
                })),
            )
            .when_some(first_missing, |bar, first| {
                bar.child(
                    button(
                        "locate",
                        say!("repositories-bulk-locate", count = missing.len()),
                        Icon::new(IconName::Search),
                    )
                    .tooltip(say!("repositories-bulk-locate-tip"))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.locate(&locate_id, &first, window, cx)
                    })),
                )
            })
            .when(!clonable.is_empty(), |bar| {
                let count = clonable.len();
                bar.child(
                    button(
                        "clone",
                        say!("repositories-bulk-clone", count),
                        Icon::new(gpui_kit::assets::IconName::ArrowDownToLine),
                    )
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_bulk(&clone_id, Kind::Clone, clonable.clone(), window, cx)
                    })),
                )
            })
            .child(
                button(
                    "remove",
                    say!("repositories-bulk-remove", count),
                    Icon::new(gpui_kit::assets::IconName::Trash),
                )
                .tooltip(say!("repositories-bulk-remove-tip"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    let paths = this.ticked_in(&remove_id);
                    this.remove_from_workspace(&remove_id, paths, window, cx)
                })),
            )
            .child(
                Button::new(SharedString::from(format!("untick-{id}")))
                    .accessibility_label(say!("repositories-tick-none"))
                    .ghost()
                    .xsmall()
                    .w(px(ROW_BUTTON))
                    .icon(IconName::Close)
                    .tooltip(say!("repositories-tick-none"))
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.tick_all(&none_id, false, cx)),
                    ),
            )
            .child(self.workspace_menu(listed, cx))
            .into_any_element()
    }

    /// The ticked rows of a workspace whose repositories are there.
    fn present_ticked(&self, id: &str) -> Vec<String> {
        let missing = self
            .workspace(id)
            .map(|listed| listed.missing.clone())
            .unwrap_or_default();
        self.ticked_in(id)
            .into_iter()
            .filter(|path| !missing.contains(path))
            .collect()
    }
}

/// What a command's answer says it did, when it says something short: a
/// fetch that brought nothing, a pull that was up to date.
fn said_of(output: &serde_json::Value) -> Option<String> {
    let said = output
        .get("summary")
        .or_else(|| output.get("said"))
        .and_then(serde_json::Value::as_str)?
        .trim();
    (!said.is_empty() && said.chars().count() <= 80).then(|| super::workspaces::sentence(said))
}

#[cfg(test)]
mod tests {
    use super::{Kind, said_of};
    use serde_json::json;

    #[test]
    fn each_kind_runs_its_engine_command() {
        assert_eq!(Kind::Fetch.command(), "workspace.fetch");
        assert_eq!(Kind::Pull.command(), "workspace.pull");
        assert_eq!(Kind::Clone.command(), "workspace.clone_missing");
    }

    #[test]
    fn a_short_summary_of_an_answer_is_said_and_a_long_one_is_not() {
        assert_eq!(
            said_of(&json!({ "summary": "up to date" })).as_deref(),
            Some("Up to date")
        );
        assert_eq!(said_of(&json!({ "summary": "x".repeat(81) })), None);
        assert_eq!(said_of(&json!({ "summary": "  " })), None);
        assert_eq!(said_of(&json!({})), None);
        assert_eq!(said_of(&json!(null)), None);
    }
}
