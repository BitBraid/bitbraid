//! Git LFS: whether it is set up here, the patterns it tracks, the actions
//! on them (set up, track, untrack, fetch, pull, push, check out), and the
//! locks the server keeps.
//!
//! Sources: none (original design).

use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::InputState,
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
        row::{BUTTON_HEIGHT, Parts, field, row},
    },
};

/// Events from the LFS view.
#[derive(Debug, Clone)]
pub enum LfsEvent {
    /// Run a command that changes something here at once.
    Change {
        id: &'static str,
        params: serde_json::Value,
        /// What to say when it worked.
        done: String,
    },
    /// Run a command that talks to the remote.
    Transfer {
        id: &'static str,
        /// What the status bar says meanwhile.
        doing: &'static str,
        /// What to say when it worked.
        done: &'static str,
    },
    /// Lock a file on the server, so that nobody else changes it.
    Lock(String),
    /// Give a lock up; somebody else's when `force` says so, which is
    /// asked about first.
    Unlock {
        /// The file.
        path: String,
        /// The lock is not this user's.
        force: bool,
    },
}

/// A lock as the server lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockRow {
    /// The file.
    pub path: String,
    /// Who holds it, when the server says.
    pub owner: Option<String>,
    /// Since when, as the server writes it.
    pub since: Option<String>,
    /// It is this user's; none when the server did not say whose it is.
    pub ours: Option<bool>,
}

impl LockRow {
    /// Read what `lfs.locks` answers: a list of locks.
    pub fn list_from(value: &serde_json::Value) -> Vec<LockRow> {
        let text = |lock: &serde_json::Value, key: &str| {
            lock.get(key)
                .and_then(|text| text.as_str())
                .map(str::to_owned)
        };
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|lock| {
                Some(LockRow {
                    path: text(lock, "path")?,
                    owner: text(lock, "owner"),
                    since: text(lock, "locked_at"),
                    ours: lock.get("ours").and_then(|ours| ours.as_bool()),
                })
            })
            .take(2000)
            .collect()
    }

    /// Whose it is and since when, beside the path.
    pub fn held(&self) -> String {
        let who = match (&self.owner, self.ours) {
            (_, Some(true)) => say!("lfs-held-yours").to_owned(),
            (Some(owner), _) => say!("lfs-held-by", owner),
            (None, _) => say!("lfs-held-by-somebody").to_owned(),
        };
        match &self.since {
            Some(since) => say!("lfs-held-since", who, since),
            None => who,
        }
    }
}

/// What `lfs.status` answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LfsStatus {
    /// git-lfs is installed, and which version.
    pub installed: bool,
    version: Option<String>,
    /// The repository's hooks call git-lfs; the filter is in effect.
    hooks: bool,
    filters: bool,
    /// Tracked patterns: the pattern, where it is declared, lockable.
    patterns: Vec<(String, String, bool)>,
    /// How many files of the index are LFS files.
    files: u64,
    /// Any pattern is tracked.
    pub active: bool,
}

impl LfsStatus {
    /// Read the answer.
    pub fn from_json(value: &serde_json::Value) -> Self {
        let flag = |key: &str| value.get(key).and_then(|flag| flag.as_bool()) == Some(true);
        Self {
            installed: flag("installed"),
            version: value
                .get("version")
                .and_then(|version| version.as_str())
                .map(str::to_owned),
            hooks: flag("hooks"),
            filters: flag("filters"),
            patterns: value
                .get("patterns")
                .and_then(|list| list.as_array())
                .into_iter()
                .flatten()
                .filter_map(|pattern| {
                    Some((
                        pattern.get("pattern")?.as_str()?.to_owned(),
                        pattern
                            .get("source")
                            .and_then(|source| source.as_str())
                            .unwrap_or(".gitattributes")
                            .to_owned(),
                        pattern.get("lockable").and_then(|flag| flag.as_bool()) == Some(true),
                    ))
                })
                .collect(),
            files: value
                .get("files")
                .and_then(|count| count.as_u64())
                .unwrap_or(0),
            active: flag("active"),
        }
    }

    /// Where things stand, in a sentence.
    fn verdict(&self) -> String {
        match (self.installed, self.hooks && self.filters) {
            (false, _) => say!("lfs-not-installed").to_owned(),
            (true, true) => say!(
                "lfs-set-up-here",
                version = self.version.as_deref().unwrap_or("git-lfs")
            ),
            (true, false) => match (self.hooks, self.filters) {
                (false, false) => say!("lfs-neither-set-up"),
                (false, true) => say!("lfs-hooks-not-set-up"),
                _ => say!("lfs-filter-not-set-up"),
            }
            .to_owned(),
        }
    }
}

/// The LFS view.
pub struct LfsView {
    session: SessionHandle,
    status: LfsStatus,
    pattern: Entity<InputState>,
    error: Option<String>,
    /// The locks the server has, once they were asked for; or why they
    /// could not be read.
    locks: Option<Result<Vec<LockRow>, String>>,
    /// The server is being asked.
    locks_loading: bool,
    /// The file to lock.
    lock_path: Entity<InputState>,
}

impl EventEmitter<LfsEvent> for LfsView {}

impl LfsView {
    /// Create the view.
    pub fn new(session: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            session,
            status: LfsStatus::default(),
            pattern: cx
                .new(|cx| InputState::new(window, cx).placeholder(say!("lfs-pattern-placeholder"))),
            error: None,
            locks: None,
            locks_loading: false,
            lock_path: cx
                .new(|cx| InputState::new(window, cx).placeholder(say!("lfs-lock-placeholder"))),
        }
    }

    /// Ask the server which files are locked. It goes over the network,
    /// so it is asked for and not done with every look at the view.
    fn load_locks(&mut self, cx: &mut Context<Self>) {
        if self.locks_loading {
            return;
        }
        self.locks_loading = true;
        cx.notify();
        let request = Request::Command {
            id: "lfs.locks".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            this.locks_loading = false;
            match reply {
                Reply::Command(value) => this.locks = Some(Ok(LockRow::list_from(&value))),
                Reply::Error(err) => this.locks = Some(Err(err)),
                _ => {}
            }
            cx.notify();
        });
    }

    fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let path = self.lock_path.read(cx).value().trim().to_owned();
        if path.is_empty() {
            return;
        }
        self.lock_path
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        cx.emit(LfsEvent::Lock(path));
    }

    /// Read where LFS stands again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "lfs.status".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    this.status = LfsStatus::from_json(&value);
                    this.error = None;
                }
                Reply::Error(err) => this.error = Some(err),
                _ => {}
            }
            cx.notify();
        });
        // Locks that are shown are read again with the rest: one may
        // just have been taken or given up.
        if self.locks.is_some() {
            self.load_locks(cx);
        }
    }

    fn track(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pattern = self.pattern.read(cx).value().trim().to_owned();
        if pattern.is_empty() {
            return;
        }
        self.pattern
            .update(cx, |state, cx| state.set_value(String::new(), window, cx));
        cx.emit(LfsEvent::Change {
            id: "lfs.track",
            params: json!({ "pattern": pattern }),
            done: say!("lfs-tracked-now", pattern),
        });
    }
}

impl Render for LfsView {
    /// The section of the preferences about large files: setting LFS up,
    /// or once it is, a pattern to track and the patterns tracked; then
    /// the transfers and the locks.
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let status = self.status.clone();
        // In a read-only repository nothing here can be done: the
        // engine would refuse each of them.
        let read_only = self.session.summary.bare;
        let ready = status.installed && !read_only;
        let set_up = status.installed && status.hooks && status.filters;
        let muted = theme.muted_foreground;
        let mut groups: Vec<AnyElement> = Vec::new();
        if !set_up {
            let install = Button::new("lfs-install")
                .primary()
                .small()
                .h(px(BUTTON_HEIGHT))
                .label(say!("lfs-install"))
                .disabled(!ready)
                .on_click(cx.listener(|_, _, _, cx| {
                    cx.emit(LfsEvent::Change {
                        id: "lfs.install",
                        params: json!({}),
                        done: say!("lfs-installed").to_owned(),
                    })
                }))
                .into_any_element();
            groups.push(own_group(
                Some(say!("lfs-set-up-group").into()),
                true,
                vec![
                    row(
                        say!("lfs-row").into(),
                        Parts {
                            control: Some(install),
                            under: Some(div().child(status.verdict()).into_any_element()),
                        },
                        cx,
                    )
                    .into_any_element(),
                ],
                cx,
            ));
        } else {
            let patterns: Vec<AnyElement> = status
                .patterns
                .iter()
                .enumerate()
                .map(|(ix, (pattern, source, lockable))| {
                    let untrack = pattern.clone();
                    h_flex()
                        .w_full()
                        .h(px(30.))
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_font_family.clone())
                                .child(pattern.clone()),
                        )
                        .when(*lockable, |d| {
                            d.child(
                                div()
                                    .flex_none()
                                    .text_xs()
                                    .text_color(muted)
                                    .child(say!("lfs-lockable")),
                            )
                        })
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(muted)
                                .child(source.clone()),
                        )
                        .child(
                            Button::new(("lfs-untrack", ix))
                                .ghost()
                                .xsmall()
                                .label(say!("lfs-untrack"))
                                .disabled(!ready)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(LfsEvent::Change {
                                        id: "lfs.untrack",
                                        params: json!({ "pattern": untrack }),
                                        done: say!("lfs-untracked-now", pattern = &untrack),
                                    })
                                })),
                        )
                        .into_any_element()
                })
                .collect();
            let none = patterns.is_empty();
            let track = Button::new("lfs-track")
                .outline()
                .small()
                .h(px(BUTTON_HEIGHT))
                .label(say!("lfs-track"))
                .disabled(!ready)
                .on_click(cx.listener(|this, _, window, cx| this.track(window, cx)));
            groups.push(own_group(
                Some(say!("lfs-tracking-group").into()),
                true,
                vec![
                    row(
                        say!("lfs-new-pattern").into(),
                        Parts {
                            control: Some(
                                field("lfs-pattern".into(), say!("lfs-new-pattern"), &self.pattern)
                                    .into_any_element(),
                            ),
                            under: Some(
                                v_flex()
                                    .gap_1()
                                    .child(h_flex().pb(px(4.)).child(track))
                                    .child(say!("lfs-track-explained"))
                                    .into_any_element(),
                            ),
                        },
                        cx,
                    )
                    .into_any_element(),
                    // The patterns stand where the controls do.
                    row(
                        say!("lfs-tracked").into(),
                        Parts {
                            control: None,
                            under: Some(
                                v_flex()
                                    .w_full()
                                    .text_sm()
                                    .text_color(theme.foreground)
                                    .children(patterns)
                                    .when(none, |d| {
                                        d.child(
                                            div().text_color(muted).child(say!("lfs-no-pattern")),
                                        )
                                    })
                                    .into_any_element(),
                            ),
                        },
                        cx,
                    )
                    .into_any_element(),
                ],
                cx,
            ));
        }
        let transfer = |id: &'static str,
                        command: &'static str,
                        label: &'static str,
                        doing: &'static str,
                        done: &'static str| {
            Button::new(id)
                .outline()
                .small()
                .h(px(BUTTON_HEIGHT))
                .label(label)
                .disabled(!ready)
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(LfsEvent::Transfer {
                        id: command,
                        doing,
                        done,
                    })
                }))
        };
        let transfers = h_flex()
            .gap(px(5.))
            .child(transfer(
                "lfs-fetch",
                "lfs.fetch",
                say!("lfs-fetch"),
                say!("lfs-fetching"),
                say!("lfs-fetched"),
            ))
            .child(transfer(
                "lfs-pull",
                "lfs.pull",
                say!("lfs-pull"),
                say!("lfs-pulling"),
                say!("lfs-pulled"),
            ))
            .child(transfer(
                "lfs-push",
                "lfs.push",
                say!("lfs-push"),
                say!("lfs-pushing"),
                say!("lfs-pushed"),
            ))
            // What was fetched, put in place of the pointers.
            .child(transfer(
                "lfs-checkout",
                "lfs.checkout",
                say!("lfs-check-out"),
                say!("lfs-checking-out"),
                say!("lfs-checked-out"),
            ))
            .into_any_element();
        let files = say!("lfs-files", count = status.files);
        groups.push(own_group(
            Some(say!("lfs-large-files-group").into()),
            false,
            vec![
                row(
                    say!("lfs-transfers").into(),
                    Parts {
                        control: Some(transfers),
                        under: Some(
                            v_flex()
                                .child(files)
                                .when(set_up, |d| d.child(status.verdict()))
                                .into_any_element(),
                        ),
                    },
                    cx,
                )
                .into_any_element(),
            ],
            cx,
        ));
        groups.push(self.render_locks(ready, cx));
        v_flex()
            .id("lfs")
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
            .children(groups)
    }
}

impl LfsView {
    /// The locks the server keeps: asked for, listed, taken and given up.
    fn render_locks(&self, ready: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        let rows: Vec<AnyElement> = match &self.locks {
            Some(Ok(locks)) => locks
                .iter()
                .enumerate()
                .map(|(ix, lock)| {
                    let path = lock.path.clone();
                    // A lock that is not this user's is taken by force,
                    // and that is asked about first.
                    let force = lock.ours != Some(true);
                    h_flex()
                        .w_full()
                        .py_0p5()
                        .gap_3()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_font_family.clone())
                                .child(lock.path.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(muted)
                                .child(lock.held()),
                        )
                        .child(
                            Button::new(("lfs-unlock", ix))
                                .ghost()
                                .xsmall()
                                .label(if force {
                                    say!("lfs-unlock-by-force")
                                } else {
                                    say!("lfs-unlock")
                                })
                                .disabled(!ready)
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(LfsEvent::Unlock {
                                        path: path.clone(),
                                        force,
                                    })
                                })),
                        )
                        .into_any_element()
                })
                .collect(),
            _ => Vec::new(),
        };
        let said = match (&self.locks, self.locks_loading) {
            (_, true) => Some((say!("lfs-asking").to_owned(), muted)),
            (None, false) => Some((say!("lfs-locks-not-asked").to_owned(), muted)),
            (Some(Err(err)), false) => {
                Some((say!("lfs-locks-unreadable", error = err), theme.danger))
            }
            (Some(Ok(locks)), false) if locks.is_empty() => {
                Some((say!("lfs-no-locks").to_owned(), muted))
            }
            (Some(Ok(_)), false) => None,
        };
        let lock = Button::new("lfs-lock")
            .outline()
            .small()
            .h(px(BUTTON_HEIGHT))
            .label(say!("lfs-lock"))
            .disabled(!ready)
            .on_click(cx.listener(|this, _, window, cx| this.lock(window, cx)));
        let ask = Button::new("lfs-locks-load")
            .outline()
            .small()
            .h(px(BUTTON_HEIGHT))
            .label(if self.locks.is_some() {
                say!("lfs-ask-again")
            } else {
                say!("lfs-show-locks")
            })
            .disabled(self.locks_loading || !self.status.installed)
            .on_click(cx.listener(|this, _, _, cx| this.load_locks(cx)));
        own_group(
            Some(say!("lfs-locks-group").into()),
            false,
            vec![
                row(
                    say!("lfs-lock-a-file").into(),
                    Parts {
                        control: Some(
                            field(
                                "lfs-lock-path".into(),
                                say!("lfs-lock-a-file"),
                                &self.lock_path,
                            )
                            .into_any_element(),
                        ),
                        under: Some(
                            v_flex()
                                .gap_1()
                                .child(h_flex().pb(px(4.)).child(lock))
                                .child(say!("lfs-lock-explained"))
                                .into_any_element(),
                        ),
                    },
                    cx,
                )
                .into_any_element(),
                row(
                    say!("lfs-locks-held").into(),
                    Parts {
                        control: Some(ask.into_any_element()),
                        under: Some(
                            v_flex()
                                .w_full()
                                .children(
                                    said.map(|(text, colour)| div().text_color(colour).child(text)),
                                )
                                .child(
                                    v_flex()
                                        .w_full()
                                        .text_sm()
                                        .text_color(theme.foreground)
                                        .children(rows),
                                )
                                .into_any_element(),
                        ),
                    },
                    cx,
                )
                .into_any_element(),
            ],
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{LfsStatus, LockRow};

    #[test]
    fn the_locks_are_read_and_say_whose_they_are() {
        let locks = LockRow::list_from(&json!([
            { "id": "1", "path": "art/big.psd", "owner": "Ada", "locked_at": "2026-10-10T10:00:00Z", "ours": true },
            { "id": "2", "path": "art/other.psd", "owner": "Grace", "locked_at": null, "ours": false },
            { "id": "3", "path": "art/whose.psd" },
            { "id": "4" },
            "nonsense",
        ]));
        assert_eq!(locks.len(), 3);
        assert_eq!(locks[0].held(), "yours, since 2026-10-10T10:00:00Z");
        assert_eq!(locks[1].held(), "held by Grace");
        assert_eq!(locks[2].held(), "held by somebody");
        assert_eq!(locks[2].ours, None);
        assert!(LockRow::list_from(&json!({ "not": "a list" })).is_empty());
    }

    #[test]
    fn the_status_is_read_and_says_where_lfs_stands() {
        let set_up = LfsStatus::from_json(&json!({
            "installed": true,
            "version": "git-lfs/3.8.0",
            "hooks": true,
            "filters": true,
            "patterns": [
                { "pattern": "*.psd", "source": ".gitattributes", "lockable": true },
                { "pattern": "assets/*.bin", "source": "assets/.gitattributes", "lockable": false },
                { "source": "no pattern" },
            ],
            "files": 3,
            "active": true,
        }));
        assert!(set_up.active && set_up.installed);
        assert_eq!(set_up.patterns.len(), 2);
        assert_eq!(set_up.verdict(), "LFS is set up here (git-lfs/3.8.0).");
        let half =
            LfsStatus::from_json(&json!({ "installed": true, "hooks": false, "filters": true }));
        assert_eq!(
            half.verdict(),
            "git-lfs is installed, and its hooks are not set up in this repository."
        );
        let absent = LfsStatus::from_json(&json!("nonsense"));
        assert!(!absent.installed && !absent.active);
        assert!(absent.verdict().starts_with("git-lfs is not installed"));
    }
}
