//! Pushing that overwrites, pushing tags, and fetching on a timer.
//!
//! A push with lease or by force first asks the engine what it would take
//! away (`remote.push_preview`) and says so before anything is sent; the
//! lease is then on the very commit that was shown, so that a fetch in
//! between cannot widen it. The timer fetches quietly: nothing is asked
//! of the user out of nowhere, and a remote that wants an answer stops it
//! for this repository, with one notification, until a fetch by hand.
//!
//! Sources: git-push(1) (`--force-with-lease`), git-fetch(1).

use std::time::{Duration, Instant};

use gpui_kit::component::{ActiveTheme, WindowExt, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Workspace, push_rejected};
use crate::{
    notices::Notice,
    prefs::Prefs,
    session::{Reply, Request},
};

/// How a push is to go.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum PushHow {
    /// Only forward.
    Plain,
    /// Overwrite what was shown, and nothing that arrived since.
    Lease,
    /// Overwrite whatever is there.
    Force,
    /// Forward, and every tag with it.
    WithTags,
    /// Forward, to this remote rather than the one the branch follows.
    To(String),
}

/// What a push that overwrites would take away, as the engine says it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Overwritten {
    /// The local branch.
    pub branch: String,
    /// The remote it goes to.
    pub remote: String,
    /// Where the branch is there, as last fetched.
    pub there: Option<String>,
    /// How many commits the remote's branch has that the local one lacks.
    pub behind: u64,
    /// The first of them, newest first: short id and summary.
    pub lost: Vec<(String, String)>,
}

impl Overwritten {
    /// Read what `remote.push_preview` answers.
    pub fn from_json(value: &Value) -> Self {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .map(str::to_owned)
        };
        Self {
            branch: text("branch").unwrap_or_default(),
            remote: text("remote").unwrap_or_default(),
            there: text("there"),
            behind: value.get("behind").and_then(|n| n.as_u64()).unwrap_or(0),
            lost: value
                .get("lost")
                .and_then(|lost| lost.as_array())
                .into_iter()
                .flatten()
                .filter_map(|commit| {
                    let id: String = commit.get("id")?.as_str()?.chars().take(7).collect();
                    Some((id, commit.get("summary")?.as_str()?.to_owned()))
                })
                .take(64)
                .collect(),
        }
    }
}

/// What is asked before a push that overwrites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The heading.
    pub title: String,
    /// What is true now.
    pub lead: String,
    /// The commits that would go, one a line.
    pub commits: Vec<String>,
    /// What the push will and will not do about it.
    pub close: String,
    /// The button that goes on.
    pub ok: &'static str,
}

/// Most commits a question lists.
const LISTED: usize = 6;

/// The question for a push with lease (`force` false) or by force. None
/// when a push with lease has nothing to overwrite as far as is known
/// here: it is then an ordinary push, and runs without a question.
pub fn overwrite_question(about: &Overwritten, force: bool) -> Option<Question> {
    let (branch, remote) = (&about.branch, &about.remote);
    let there = format!("{remote}/{branch}");
    let by_force = say!("dialogs-push-force-unseen");
    match (&about.there, about.behind, force) {
        (_, 0, false) | (None, _, false) => None,
        (None, _, true) => Some(Question {
            title: say!("dialogs-force-push-title", branch, remote),
            lead: say!("dialogs-force-push-unknown", branch, remote),
            commits: Vec::new(),
            close: String::new(),
            ok: say!("workspace-force-push"),
        }),
        (Some(_), 0, true) => Some(Question {
            title: say!("dialogs-force-push-title", branch, remote),
            lead: say!("dialogs-force-push-level", there = there.as_str(), branch),
            commits: Vec::new(),
            close: by_force.to_owned(),
            ok: say!("workspace-force-push"),
        }),
        (Some(at), behind, _) => {
            let mut commits: Vec<String> = about
                .lost
                .iter()
                .take(LISTED)
                .map(|(id, summary)| format!("{id}  {summary}"))
                .collect();
            let listed = u64::try_from(commits.len()).unwrap_or(u64::MAX);
            if behind > listed {
                commits.push(say!(
                    "dialogs-overwrite-more",
                    more = behind.saturating_sub(listed)
                ));
            }
            let short: String = at.chars().take(7).collect();
            Some(Question {
                title: say!("dialogs-overwrite-title", behind, there = there.as_str()),
                lead: say!(
                    "dialogs-overwrite-lead",
                    there = there.as_str(),
                    behind,
                    branch
                ),
                commits,
                close: if force {
                    by_force.to_owned()
                } else {
                    say!(
                        "dialogs-lease-refuses-if-moved",
                        there = there.as_str(),
                        short
                    )
                },
                ok: if force {
                    say!("workspace-force-push")
                } else {
                    say!("dialogs-push-with-lease")
                },
            })
        }
    }
}

/// The parameters of `remote.push` for a push of `branch` that overwrites.
/// A lease is on the commit that was shown; with nothing known of the
/// branch on the remote it is an ordinary push.
pub fn overwrite_params(about: &Overwritten, force: bool, publish: bool) -> Value {
    let mut params = json!({
        "branch": about.branch,
        "remote": about.remote,
        "set_upstream": publish,
    });
    let how = match (force, &about.there) {
        (true, _) => Some("force"),
        (false, Some(_)) => Some("lease"),
        (false, None) => None,
    };
    if let (Some(how), Some(params)) = (how, params.as_object_mut()) {
        params.insert("force".to_owned(), json!(how));
        if let (false, Some(at)) = (force, &about.there) {
            params.insert("expect".to_owned(), json!(at));
        }
    }
    params
}

/// Whether git said that a lease did not hold.
pub fn lease_broken(err: &str) -> bool {
    err.contains("stale info")
}

/// When the timer last fetched or was told that someone did, and whether
/// it has stopped.
#[derive(Debug, Clone, Copy)]
pub struct AutoFetch {
    /// The last fetch, by the timer or by hand; when the tab was opened
    /// before there was one.
    since: Instant,
    /// A remote wanted an answer: nothing more until a fetch by hand.
    stopped: bool,
}

impl AutoFetch {
    /// A timer that starts counting now.
    pub fn new() -> Self {
        Self {
            since: Instant::now(),
            stopped: false,
        }
    }

    /// Someone fetched by hand: the wait begins again, and so does the
    /// timer when it had stopped.
    pub fn fetched_by_hand(&mut self) {
        self.since = Instant::now();
        self.stopped = false;
    }
}

impl Default for AutoFetch {
    fn default() -> Self {
        Self::new()
    }
}

/// What keeps the timer from fetching, whatever the time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Held {
    /// An operation is running or waiting, or a merge, rebase, pick or
    /// revert is unfinished.
    pub busy: bool,
    /// A remote wanted an answer.
    pub stopped: bool,
    /// There is no remote to fetch.
    pub no_remote: bool,
    /// The repository is opened read-only.
    pub read_only: bool,
}

/// Whether the timer fetches now: every `minutes` (never when that is
/// nought), counted from the last fetch, and only when nothing holds it.
pub fn auto_fetch_due(minutes: u32, since: Instant, now: Instant, held: Held) -> bool {
    if minutes == 0 || held.busy || held.stopped || held.no_remote || held.read_only {
        return false;
    }
    let every = Duration::from_secs(u64::from(minutes).saturating_mul(60));
    now.saturating_duration_since(since) >= every
}

/// The parameters of `remote.fetch` for one remote, or for all of them,
/// as the preferences have fetching.
pub fn fetch_params(remote: Option<&str>, preferences: &crate::prefs::Preferences) -> Value {
    json!({
        "remote": remote,
        "prune": preferences.fetch_prune,
        "tags": preferences.fetch_tags,
    })
}

/// How often the timer looks at the clock.
const TICK: Duration = Duration::from_secs(15);

/// What the engine's quiet fetch says when a remote could only be
/// fetched by asking (`bitbraid_git::remote::WANTS_AN_ANSWER`).
// not shown: the engine's words, looked for
const WANTS_AN_ANSWER: &str = "wants an answer";

impl Workspace {
    /// Look at the clock every few seconds for as long as the tab lives.
    pub(super) fn watch_the_clock(window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this.update_in(cx, |this, window, cx| this.auto_fetch_tick(window, cx));
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// Fetch in the background when it is time and nothing holds it.
    fn auto_fetch_tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (minutes, mut params) = {
            let preferences = &cx.global::<Prefs>().preferences;
            (
                preferences.auto_fetch_minutes,
                fetch_params(None, preferences),
            )
        };
        let held = Held {
            busy: self.operation.is_some() || self.in_progress.is_some(),
            stopped: self.auto_fetch.stopped,
            no_remote: self
                .snapshot
                .as_ref()
                .is_none_or(|snapshot| snapshot.remotes.is_empty()),
            read_only: self.read_only(),
        };
        let now = Instant::now();
        if !auto_fetch_due(minutes, self.auto_fetch.since, now, held) {
            return;
        }
        self.auto_fetch.since = now;
        if let Some(params) = params.as_object_mut() {
            params.insert("quiet".to_owned(), json!(true));
        }
        let reply = self.session.send(Request::Operation {
            id: "remote.fetch".to_owned(),
            params,
        });
        self.operation_began(say!("dialogs-fetching-background"), None);
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.operation_ended();
                match reply {
                    // Said once; the timer waits for a fetch by hand.
                    Ok(Reply::Error(err)) if err.contains(WANTS_AN_ANSWER) => {
                        this.auto_fetch.stopped = true;
                        let workspace = cx.entity().downgrade();
                        let who = err
                            .split(WANTS_AN_ANSWER)
                            .next()
                            .and_then(|before| before.rsplit(": ").next())
                            .map(str::trim)
                            .filter(|who| !who.is_empty())
                            .unwrap_or_else(|| say!("dialogs-a-remote"))
                            .to_owned();
                        let notice = Notice::warning(say!("dialogs-fetch-stopped", who))
                            .autohide(false)
                            .action(say!("dialogs-fetch"), move |window, cx| {
                                let _ = workspace.update(cx, |this, cx| this.fetch(window, cx));
                            });
                        crate::notices::push(window, cx, notice);
                    }
                    // Anything else that went wrong is in the activity
                    // log: a machine that is offline says nothing.
                    Ok(Reply::Error(_)) | Err(_) => {}
                    Ok(_) => this.reload(cx),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The name of the branch a push is about: the one named, else the
    /// one that is checked out.
    fn push_branch(&self, branch: Option<String>) -> Option<String> {
        branch.or_else(|| {
            self.snapshot
                .as_ref()
                .filter(|snapshot| !snapshot.head.detached)
                .and_then(|snapshot| snapshot.head.branch.clone())
        })
    }

    /// Whether a local branch follows nothing yet, so that a push
    /// publishes it.
    fn unpublished(&self, branch: &str) -> bool {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.branches.iter().find(|b| b.name == branch))
            .is_none_or(|branch| branch.upstream.is_none())
    }

    /// Push `branch` (the checked-out one when none is named) in one of
    /// the ways there are. What overwrites asks first and says what goes.
    pub fn push_how(
        &mut self,
        branch: Option<String>,
        how: PushHow,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.refused_here("remote.push", window, cx) {
            return;
        }
        let Some(name) = self.push_branch(branch.clone()) else {
            crate::notices::push(window, cx, Notice::info(say!("dialogs-push-detached")));
            return;
        };
        let publish = self.unpublished(&name);
        match how {
            PushHow::Plain if branch.is_none() => self.push(window, cx),
            PushHow::Plain => self.push_branches(&[name], window, cx),
            PushHow::WithTags => self.run_operation(
                "remote.push",
                json!({ "branch": name, "set_upstream": publish, "tags": true }),
                say!("workspace-pushing"),
                say!("dialogs-pushed-with-tags", branch = name.as_str()),
                window,
                cx,
            ),
            PushHow::To(remote) => self.run_operation(
                "remote.push",
                json!({ "branch": name, "remote": remote, "set_upstream": publish }),
                say!("workspace-pushing"),
                say!(
                    "dialogs-pushed-to",
                    branch = name.as_str(),
                    remote = remote.as_str()
                ),
                window,
                cx,
            ),
            PushHow::Lease | PushHow::Force => {
                let force = how == PushHow::Force;
                let reply = self.session.send(Request::Command {
                    id: "remote.push_preview".to_owned(),
                    params: json!({ "branch": name }),
                });
                cx.spawn_in(window, async move |this, cx| {
                    let Ok(reply) = reply.await else { return };
                    let _ = this.update_in(cx, |this, window, cx| match reply {
                        Reply::Command(value) => {
                            let about = Overwritten::from_json(&value);
                            this.ask_then_overwrite(about, force, publish, window, cx);
                        }
                        Reply::Error(err) => {
                            let notice = this.failed_notice(&err, cx);
                            crate::notices::push(window, cx, notice);
                        }
                        _ => {}
                    });
                })
                .detach();
            }
        }
    }

    /// Say what a push that overwrites takes away and, when that is
    /// accepted (or there is nothing to accept), push.
    fn ask_then_overwrite(
        &mut self,
        about: Overwritten,
        force: bool,
        publish: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(question) = overwrite_question(&about, force) else {
            self.overwrite(about, force, publish, window, cx);
            return;
        };
        let workspace = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let (workspace, about) = (workspace.clone(), about.clone());
            let theme = cx.theme();
            let (muted, mono) = (theme.muted_foreground, theme.mono_font_family.clone());
            let question = question.clone();
            alert
                .title(question.title)
                .description(
                    v_flex()
                        .gap_2()
                        .text_sm()
                        .child(div().whitespace_normal().child(question.lead))
                        .when(!question.commits.is_empty(), |text| {
                            text.child(
                                v_flex().font_family(mono).text_xs().children(
                                    question
                                        .commits
                                        .into_iter()
                                        .map(|commit| div().truncate().child(commit)),
                                ),
                            )
                        })
                        .when(!question.close.is_empty(), |text| {
                            text.child(
                                div()
                                    .whitespace_normal()
                                    .text_color(muted)
                                    .child(question.close),
                            )
                        }),
                )
                .ok_text(question.ok)
                // What overwrites is in the colour of danger.
                .ok_variant(gpui_kit::component::button::ButtonVariant::Danger)
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let about = about.clone();
                    let _ = workspace.update(cx, |this, cx| {
                        this.overwrite(about, force, publish, window, cx)
                    });
                    true
                })
        });
    }

    fn overwrite(
        &mut self,
        about: Overwritten,
        force: bool,
        publish: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let params = overwrite_params(&about, force, publish);
        let (branch, remote) = (about.branch.clone(), about.remote.clone());
        let done = match (force, &about.there) {
            (true, _) => say!(
                "dialogs-pushed-by-force",
                branch = branch.as_str(),
                remote = remote.as_str()
            ),
            (false, Some(_)) => say!(
                "dialogs-pushed-with-lease",
                branch = branch.as_str(),
                remote = remote.as_str()
            ),
            (false, None) => say!(
                "dialogs-pushed-to",
                branch = branch.as_str(),
                remote = remote.as_str()
            ),
        };
        self.run_operation_with(
            "remote.push",
            params,
            say!("workspace-pushing"),
            done,
            move |this, err, window, cx| {
                let said = if lease_broken(err) {
                    say!(
                        "dialogs-lease-broken",
                        remote = remote.as_str(),
                        branch = branch.as_str()
                    )
                } else if push_rejected(err) {
                    say!(
                        "dialogs-push-unknown-branch",
                        remote = remote.as_str(),
                        branch = branch.as_str()
                    )
                } else {
                    return false;
                };
                let (for_fetch, for_log) = (cx.entity().downgrade(), cx.entity().downgrade());
                let notice = Notice::error(said)
                    .action(say!("dialogs-fetch"), move |window, cx| {
                        let _ = for_fetch.update(cx, |this, cx| this.fetch(window, cx));
                    })
                    .action(say!("workspace-show-log"), move |_, cx| {
                        let _ = for_log.update(cx, |this, cx| this.show_activity(cx));
                    });
                let _ = this;
                crate::notices::push(window, cx, notice);
                true
            },
            window,
            cx,
        );
    }

    /// Push every tag to `remote`, or to `origin` or the only remote when
    /// none is named.
    pub fn push_all_tags(
        &mut self,
        remote: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let done = match &remote {
            Some(remote) => say!("dialogs-pushed-tags-to", remote),
            None => say!("dialogs-pushed-tags").to_owned(),
        };
        self.run_operation(
            "tag.push",
            json!({ "remote": remote }),
            say!("dialogs-pushing-tags"),
            done,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use serde_json::json;

    use super::{
        Held, Overwritten, auto_fetch_due, lease_broken, overwrite_params, overwrite_question,
    };

    fn about(there: bool, behind: u64) -> Overwritten {
        Overwritten::from_json(&json!({
            "branch": "topic",
            "remote": "origin",
            "there": there.then_some("0123456789abcdef0123456789abcdef01234567"),
            "ahead": 2,
            "behind": behind,
            "lost": (0..behind.min(20))
                .map(|n| json!({ "id": format!("{n:040x}"), "summary": format!("change {n}") }))
                .collect::<Vec<_>>(),
        }))
    }

    #[test]
    fn a_push_with_lease_asks_only_when_something_would_go() {
        assert_eq!(overwrite_question(&about(true, 0), false), None);
        assert_eq!(overwrite_question(&about(false, 0), false), None);
        let question = overwrite_question(&about(true, 1), false).unwrap();
        assert_eq!(question.title, "Overwrite 1 commit on origin/topic?");
        assert_eq!(
            question.lead,
            "origin/topic has 1 commit that topic here does not have. Pushing replaces it:"
        );
        assert_eq!(question.commits, ["0000000  change 0"]);
        assert_eq!(
            question.close,
            "The push is refused if origin/topic is no longer at 0123456: nothing that arrived \
             since is lost."
        );
        assert_eq!(question.ok, "Push with lease");
    }

    #[test]
    fn a_forced_push_always_asks_and_says_what_is_not_known() {
        let nothing_known = overwrite_question(&about(false, 0), true).unwrap();
        assert_eq!(nothing_known.title, "Force push topic to origin?");
        assert!(nothing_known.lead.contains("Nothing is known here"));
        assert!(nothing_known.commits.is_empty());

        let level = overwrite_question(&about(true, 0), true).unwrap();
        assert!(
            level
                .lead
                .starts_with("As last fetched, origin/topic has nothing")
        );
        assert!(level.close.contains("goes too, unseen"));
        assert_eq!(level.ok, "Force push");

        // Many commits: the first few and how many more.
        let many = overwrite_question(&about(true, 30), true).unwrap();
        assert_eq!(many.title, "Overwrite 30 commits on origin/topic?");
        assert_eq!(many.commits.len(), 7);
        assert_eq!(many.commits.last().unwrap(), "and 24 more");
        assert!(many.lead.ends_with("Pushing replaces them:"));
        assert!(many.close.contains("a push with lease would refuse"));
    }

    #[test]
    fn a_lease_is_on_the_commit_that_was_shown() {
        let id = "0123456789abcdef0123456789abcdef01234567";
        assert_eq!(
            overwrite_params(&about(true, 2), false, false),
            json!({
                "branch": "topic", "remote": "origin", "set_upstream": false,
                "force": "lease", "expect": id,
            })
        );
        assert_eq!(
            overwrite_params(&about(true, 2), true, false),
            json!({
                "branch": "topic", "remote": "origin", "set_upstream": false, "force": "force",
            })
        );
        // Nothing known of it there: with lease it is an ordinary push
        // that publishes.
        assert_eq!(
            overwrite_params(&about(false, 0), false, true),
            json!({ "branch": "topic", "remote": "origin", "set_upstream": true })
        );
        assert!(lease_broken(
            " ! [rejected]        main -> main (stale info)\nerror: failed to push some refs"
        ));
        assert!(!lease_broken(
            " ! [rejected]        main -> main (fetch first)"
        ));
    }

    #[test]
    fn a_fetch_goes_as_the_preferences_say() {
        let mut preferences = crate::prefs::Preferences::default();
        assert_eq!(
            super::fetch_params(None, &preferences),
            json!({ "remote": null, "prune": true, "tags": false })
        );
        preferences.fetch_prune = false;
        preferences.fetch_tags = true;
        assert_eq!(
            super::fetch_params(Some("origin"), &preferences),
            json!({ "remote": "origin", "prune": false, "tags": true })
        );
    }

    #[test]
    fn what_the_engine_answers_is_read_with_care() {
        let nothing = Overwritten::from_json(&json!("nonsense"));
        assert_eq!(nothing, Overwritten::default());
        let odd = Overwritten::from_json(&json!({
            "branch": 7, "remote": "origin", "there": null, "behind": -3,
            "lost": [{ "id": "é".repeat(40), "summary": "wide" }, { "id": 1 }, "x"],
        }));
        assert_eq!(odd.branch, "");
        assert_eq!(odd.behind, 0);
        assert_eq!(odd.lost, [("é".repeat(7), "wide".to_owned())]);
    }

    #[test]
    fn the_timer_fetches_when_it_is_time_and_nothing_holds_it() {
        let start = Instant::now();
        let after = |minutes: u64| start + Duration::from_secs(minutes * 60);
        let free = Held::default();
        // Off.
        assert!(!auto_fetch_due(0, start, after(10_000), free));
        // Not yet, then due.
        assert!(!auto_fetch_due(5, start, after(4), free));
        assert!(auto_fetch_due(5, start, after(5), free));
        assert!(auto_fetch_due(5, start, after(600), free));
        // A clock that went backwards is not a reason to fetch.
        assert!(!auto_fetch_due(1, after(10), start, free));
        // Held, whatever the time.
        for held in [
            Held { busy: true, ..free },
            Held {
                stopped: true,
                ..free
            },
            Held {
                no_remote: true,
                ..free
            },
            Held {
                read_only: true,
                ..free
            },
        ] {
            assert!(!auto_fetch_due(1, start, after(600), held), "{held:?}");
        }
        // The longest interval a file could name does not overflow.
        assert!(!auto_fetch_due(u32::MAX, start, after(600), free));
    }
}
