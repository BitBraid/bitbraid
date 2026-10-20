//! The window's updater: what it runs and says.
//!
//! When the window opens it asks `update.status` what this copy is, which
//! asks nobody. Only while updates are on (Preferences, General, Updates;
//! off until the user turns them on) does it look for a newer version with
//! `update.check`: at the start, once a day while it runs, as soon as they
//! are turned on, and when the user asks (the palette's "Check for
//! updates", the preferences' "Look now"). Asked while they are off, it
//! says so and offers to turn them on; nothing is sent.
//!
//! A newer version found is said once a run in a notice that stays, with
//! a button that installs it when this copy installs it itself
//! (`update.install`, its download shown in the preferences and stoppable
//! there) and its page otherwise. Installed, the window offers to start the
//! new version in its place, or closes for the Windows installer.
//!
//! Sources: none (original design).

use futures::StreamExt as _;
use gpui_kit::*;
use serde_json::json;

use super::Shell;
use crate::{
    notices::Notice,
    prefs::Prefs,
    session::Reply,
    updates::{self, Ask, Failed, Installed, Look, Status, Updates},
};

impl Shell {
    /// Ask what this copy is, look when updates are on, and again once a
    /// day; look as soon as they are turned on.
    pub(super) fn start_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let status = crate::session::run_standalone_unasked("update.status", json!({}));
        cx.spawn_in(window, async move |_, cx| {
            if let Ok(Reply::Command(value)) = status.reply.await {
                let _ = cx.update(|_, cx| {
                    cx.global_mut::<Updates>().status = Some(Status::from_json(&value));
                    cx.refresh_windows();
                });
            }
        })
        .detach();
        let turned = cx.observe_global_in::<Prefs>(window, |shell, window, cx| {
            let on = cx.global::<Prefs>().preferences.updates;
            let was = std::mem::replace(&mut cx.global_mut::<Updates>().on, on);
            if on && !was {
                shell.look_for_updates(false, window, cx);
            }
        });
        self._subscriptions.push(turned);
        if cx.global::<Prefs>().preferences.updates {
            cx.defer_in(window, |shell, window, cx| {
                shell.look_for_updates(false, window, cx)
            });
        }
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(updates::EVERY).await;
                let alive = this.update_in(cx, |shell, window, cx| {
                    if cx.global::<Prefs>().preferences.updates {
                        shell.look_for_updates(false, window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// What the preferences ask of the updater.
    pub(super) fn on_update_ask(&mut self, ask: Ask, window: &mut Window, cx: &mut Context<Self>) {
        match ask {
            Ask::Look => self.look_for_updates(true, window, cx),
            Ask::Install => self.install_update(window, cx),
            Ask::Stop => {
                if let Some(cancel) = &cx.global::<Updates>().cancel {
                    cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                }
            }
            Ask::Restart => restart(window, cx),
            Ask::Page => {
                if let Some(look) = &cx.global::<Updates>().found {
                    crate::providers::open_link(&look.page, cx);
                }
            }
        }
    }

    /// Asked while updates are off: say so, and offer to turn them on.
    fn updates_are_off(window: &mut Window, cx: &mut App) {
        let notice = Notice::info(say!("updates-off-notice")).action(
            say!("updates-turn-on-and-look"),
            |_, cx| {
                let prefs = cx.global_mut::<Prefs>();
                prefs.preferences.updates = true;
                prefs.save_preferences();
                cx.refresh_windows();
            },
        );
        crate::notices::push(window, cx, notice);
    }

    /// Look for a newer version: when the user `asked`, everything that
    /// comes of it is said; otherwise only a newer version, once a run.
    pub(crate) fn look_for_updates(
        &mut self,
        asked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !cx.global::<Prefs>().preferences.updates {
            if asked {
                Self::updates_are_off(window, cx);
            }
            return;
        }
        let updates = cx.global_mut::<Updates>();
        if updates.looking || updates.installing.is_some() {
            return;
        }
        updates.looking = true;
        cx.refresh_windows();
        let run = if asked {
            crate::session::run_standalone("update.check", json!({}))
        } else {
            crate::session::run_standalone_unasked("update.check", json!({}))
        };
        cx.spawn_in(window, async move |this, cx| {
            let reply = run.reply.await;
            let _ = this.update_in(cx, |shell, window, cx| {
                shell.looked(reply.ok(), asked, window, cx)
            });
        })
        .detach();
    }

    fn looked(
        &mut self,
        reply: Option<Reply>,
        asked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let now = crate::views::now();
        let shell = cx.entity().downgrade();
        let updates = cx.global_mut::<Updates>();
        updates.looking = false;
        updates.looked_at = Some(now);
        let mut say = None;
        match reply {
            Some(Reply::Command(value)) => match Look::from_json(&value) {
                Some(look) => {
                    updates.failed = None;
                    if look.newer && (asked || updates.told.as_deref() != Some(&look.latest)) {
                        updates.told = Some(look.latest.clone());
                        say = Some(found_notice(&look, shell));
                    } else if asked && !look.newer {
                        say = Some(Notice::success(say!(
                            "updates-newest",
                            current = &look.current
                        )));
                    }
                    updates.found = Some(look);
                }
                None => {
                    let why = say!("updates-feed-unreadable").to_owned();
                    updates.failed = Some(Failed::Look(why.clone()));
                    say = asked.then(|| Notice::error(why));
                }
            },
            Some(Reply::Error(err)) => {
                let why = updates::said(&err);
                updates.failed = Some(Failed::Look(why.clone()));
                say = asked.then(|| Notice::error(why));
            }
            _ => {}
        }
        cx.refresh_windows();
        if let Some(notice) = say {
            crate::notices::push(window, cx, notice);
        }
    }

    /// Install the newer version that was found (or, with none found yet,
    /// whatever the feed names when it is newer).
    pub(crate) fn install_update(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !cx.global::<Prefs>().preferences.updates {
            Self::updates_are_off(window, cx);
            return;
        }
        let updates = cx.global_mut::<Updates>();
        if updates.looking || updates.installing.is_some() {
            return;
        }
        let offered = updates
            .found
            .as_ref()
            .filter(|look| look.newer)
            .map(|look| look.latest.clone());
        let crate::session::Standalone {
            reply,
            mut progress,
            cancel,
        } = crate::session::run_standalone("update.install", json!({ "version": offered }));
        updates.installing = Some(0);
        updates.failed = None;
        updates.cancel = Some(cancel);
        cx.refresh_windows();
        cx.spawn_in(window, async move |_, cx| {
            while let Some(step) = progress.next().await {
                let alive = cx.update(|_, cx| {
                    let updates = cx.global_mut::<Updates>();
                    if updates.installing.is_some() {
                        updates.installing = Some(step.percent.unwrap_or(0).min(100));
                        cx.refresh_windows();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let _ = this.update_in(cx, |_, window, cx| installed(reply.ok(), window, cx));
        })
        .detach();
    }
}

/// The notice of a newer version: what it offers depends on whether this
/// copy installs it itself.
fn found_notice(look: &Look, shell: WeakEntity<Shell>) -> Notice {
    let notice = Notice::info(updates::found_text(look)).autohide(false);
    let page = look.page.clone();
    if look.installs {
        notice
            .action(say!("updates-install"), move |window, cx| {
                let _ = shell.update(cx, |shell, cx| shell.install_update(window, cx));
            })
            .action(say!("updates-what-is-new"), move |_, cx| {
                crate::providers::open_link(&page, cx);
            })
    } else {
        notice.action(say!("updates-open-page"), move |_, cx| {
            crate::providers::open_link(&page, cx);
        })
    }
}

/// What came of an installation.
fn installed(reply: Option<Reply>, window: &mut Window, cx: &mut App) {
    let updates = cx.global_mut::<Updates>();
    updates.installing = None;
    updates.cancel = None;
    let notice = match reply {
        Some(Reply::Command(value)) => {
            let version = value
                .get("installed")
                .and_then(|version| version.as_str())
                .unwrap_or_default()
                .chars()
                .take(64)
                .collect::<String>();
            if value.get("then").and_then(|then| then.as_str()) == Some("quit") {
                updates.installed = Some(Installed::Quit(version));
                cx.refresh_windows();
                // The installer replaces what runs: it needs it closed.
                cx.quit();
                return;
            }
            updates.installed = Some(Installed::Restart(version.clone()));
            Notice::success(say!("updates-installed-notice", version))
                .autohide(false)
                .action(say!("updates-restart-now"), restart)
        }
        Some(Reply::Error(err)) if err.contains("cancelled") => {
            Notice::info(say!("updates-download-stopped"))
        }
        Some(Reply::Error(err)) => {
            let why = updates::said(&err);
            updates.failed = Some(Failed::Install(why.clone()));
            Notice::error(why)
        }
        _ => return,
    };
    cx.refresh_windows();
    crate::notices::push(window, cx, notice);
}

/// Start the version that was installed and close this one.
fn restart(window: &mut Window, cx: &mut App) {
    let updater = bitbraid_integrations::update::Updater::this_copy();
    match bitbraid_integrations::update::start_again(&updater.installation) {
        Ok(()) => cx.quit(),
        Err(why) => crate::notices::push(window, cx, Notice::error(updates::said(&why))),
    }
}
