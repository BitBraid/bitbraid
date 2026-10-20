//! Patches from the interface: the file is typed or chosen in the
//! desktop's own chooser, and before a patch is applied git is asked what
//! it would do (`patch.check`): the files it touches with the lines added
//! and removed, and whether it applies as it is. One that does not can be
//! tried with a three-way merge.
//!
//! Sources: `crates/engine/src/patch_cmds.rs`; git-apply(1) (`--check`,
//! `--numstat`, `--3way`).

use gpui_kit::component::{
    WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::Input,
    v_flex,
};
use gpui_kit::*;

use crate::zoom::px;
use serde_json::{Value, json};

use super::{Workspace, sentence};
use crate::{
    notices::Notice,
    session::{Reply, Request},
};

/// How many files of a patch are named before the rest is counted.
const NAMED: usize = 8;

/// What is asked before a patch is applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    /// The question.
    pub title: String,
    /// What the patch would do.
    pub description: String,
    /// It applies as it is; otherwise only a three-way merge is offered.
    pub applies: bool,
}

/// The last part of a path, for a title.
fn name_of(file: &str) -> &str {
    file.rsplit(['/', '\\']).next().unwrap_or(file)
}

/// What `patch.check` answered about a patch file, as a question to ask
/// before it is applied.
pub fn preview(file: &str, checked: &Value) -> Preview {
    let name = name_of(file);
    let applies = checked.get("applies").and_then(|flag| flag.as_bool()) == Some(true);
    let files: Vec<String> = checked
        .get("files")
        .and_then(|files| files.as_array())
        .map(|files| {
            files
                .iter()
                .filter_map(|file| {
                    let path = file.get("path")?.as_str()?;
                    let count = |key: &str| file.get(key).and_then(|count| count.as_u64());
                    Some(match (count("added"), count("deleted")) {
                        (Some(added), Some(deleted)) => format!("{path} (+{added} −{deleted})"),
                        _ => say!("dialogs-patch-file-binary", path),
                    })
                })
                .take(NAMED)
                .collect()
        })
        .unwrap_or_default();
    let total = checked
        .get("total")
        .and_then(|total| total.as_u64())
        .and_then(|total| usize::try_from(total).ok())
        .unwrap_or(files.len())
        .max(files.len());
    let more = total.saturating_sub(files.len());
    let mut description = match (total, more) {
        (0, _) => say!("dialogs-patch-changes-none").to_owned(),
        (1, _) | (_, 0) => say!("dialogs-patch-changes", total, files = files.join(", ")),
        (total, more) => say!(
            "dialogs-patch-changes-more",
            total,
            files = files.join(", "),
            more
        ),
    };
    if !applies {
        let reason = checked
            .get("reason")
            .and_then(|reason| reason.as_str())
            .map(str::trim)
            .filter(|reason| !reason.is_empty())
            .unwrap_or_else(|| say!("dialogs-patch-no-reason"));
        description.push(' ');
        description.push_str(&say!("dialogs-patch-does-not-apply", reason));
    } else {
        description.push(' ');
        description.push_str(say!("dialogs-patch-applies"));
    }
    Preview {
        title: if applies {
            say!("dialogs-patch-apply-title", name)
        } else {
            say!("dialogs-patch-does-not-apply-title", name)
        },
        description,
        applies,
    }
}

impl Workspace {
    /// Ask for a patch file, typed or chosen in the desktop's chooser, and
    /// go on with it: a mailbox is applied as commits, a patch is looked
    /// at first.
    pub(super) fn prompt_patch_file(
        &mut self,
        mailbox: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.refused_here("patch.apply", window, cx) {
            return;
        }
        self.prompt_action = Some(std::rc::Rc::new(move |this, texts, window, cx| {
            let typed = texts.into_iter().next().unwrap_or_default();
            let file = crate::shell::expand_home(&typed).display().to_string();
            this.patch_chosen(file, mailbox, window, cx);
        }));
        self.prompt_required = vec![true];
        self.prompt.update(cx, |state, cx| {
            state.set_placeholder(say!("dialogs-patch-file-placeholder"), window, cx);
            state.set_value("", window, cx);
        });
        let input = self.prompt.clone();
        let workspace = cx.entity().downgrade();
        // A mailbox makes commits, and two hooks can refuse a patch of
        // it: where one of them is there, it can be left out, for this
        // mailbox. A patch that is only applied to the files asks none.
        self.skip_mailbox_hooks = false;
        let hooks = if mailbox {
            self.hooks_there(bitbraid_git::hooks::Unverified::Mailbox, cx)
        } else {
            Vec::new()
        };
        let title = if mailbox {
            say!("dialogs-mailbox-title")
        } else {
            say!("dialogs-patch-title")
        };
        window.open_dialog(cx, move |dialog, _, cx| {
            let (for_browse, for_ok) = (workspace.clone(), workspace.clone());
            let for_switch = workspace.clone();
            let skipping = workspace
                .upgrade()
                .is_some_and(|workspace| workspace.read(cx).skip_mailbox_hooks);
            let switch = (!hooks.is_empty()).then(|| {
                Checkbox::new("patch-skip-hooks")
                    .label(super::hooks::switch_label(&hooks))
                    .checked(skipping)
                    .on_click(move |checked: &bool, window, cx| {
                        let _ = for_switch.update(cx, |this, cx| {
                            this.skip_mailbox_hooks = *checked;
                            cx.notify();
                        });
                        window.refresh();
                    })
            });
            dialog
                .title(title)
                .width(px(560.))
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&input)))
                                .child(
                                    Button::new("patch-browse")
                                        .outline()
                                        .label(say!("dialogs-patch-choose"))
                                        .on_click(move |_, window, cx| {
                                            let _ = for_browse.update(cx, |this, cx| {
                                                this.browse_for_patch(window, cx)
                                            });
                                        }),
                                ),
                        )
                        .children(switch),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("patch-cancel")
                                .ghost()
                                .label(say!("workspace-cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("patch-ok")
                                .primary()
                                .label(match (mailbox, skipping) {
                                    (true, true) => say!("dialogs-apply-without-hooks"),
                                    (true, false) => say!("dialogs-apply"),
                                    (false, _) => say!("dialogs-patch-look"),
                                })
                                .on_click(move |_, window, cx| {
                                    let done = for_ok
                                        .update(cx, |this, cx| this.confirm_prompt(window, cx))
                                        .unwrap_or(true);
                                    if done {
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
        });
        cx.defer_in(window, |this, window, cx| {
            this.prompt.update(cx, |state, cx| state.focus(window, cx));
        });
    }

    /// Choose the patch file in the desktop's chooser; what is chosen goes
    /// into the field, to be looked at before anything is done with it.
    fn browse_for_patch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(say!("dialogs-patch-choose-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.prompt.update(cx, |state, cx| {
                        state.set_value(path.display().to_string(), window, cx)
                    });
                });
            }
        })
        .detach();
    }

    /// A patch file was named: a mailbox is applied as commits; a patch is
    /// checked first, and applied when that is confirmed.
    fn patch_chosen(
        &mut self,
        file: String,
        mailbox: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if mailbox {
            // It makes commits, runs hooks and can stop half-way. A patch
            // that does not apply as it is is merged in (`--3way`), so
            // that what it wanted and what is here stand in the file as a
            // conflict, with everything there is for resolving one;
            // without that the series stops with nothing to look at.
            let params = json!({ "file": file, "three_way": true });
            // Ticked in the dialog, for this mailbox.
            let without = std::mem::take(&mut self.skip_mailbox_hooks);
            self.run_operation(
                "patch.mailbox",
                if without {
                    super::hooks::without(params)
                } else {
                    params
                },
                say!("dialogs-mailbox-applying"),
                say!("dialogs-mailbox-applied").to_owned(),
                window,
                cx,
            );
            return;
        }
        let reply = self.session.send(Request::Command {
            id: "patch.check".to_owned(),
            params: json!({ "file": file }),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else {
                return;
            };
            let _ = this.update_in(cx, |_, window, cx| {
                let checked = match reply {
                    Reply::Error(err) => {
                        crate::notices::push(window, cx, Notice::error(sentence(&err)));
                        return;
                    }
                    Reply::Command(checked) => checked,
                    _ => return,
                };
                let asked = preview(&file, &checked);
                let workspace = cx.entity().downgrade();
                window.open_alert_dialog(cx, move |alert, _, cx| {
                    let (workspace, file) = (workspace.clone(), file.clone());
                    let three_way = !asked.applies;
                    alert
                        .title(asked.title.clone())
                        // What git says of the patch, file names and all.
                        .description(crate::literal::said(asked.description.clone(), cx))
                        .width(px(560.))
                        .ok_text(if three_way {
                            say!("dialogs-patch-try-three-way")
                        } else {
                            say!("dialogs-apply")
                        })
                        .show_cancel(true)
                        .on_ok(move |_, window, cx| {
                            let file = file.clone();
                            let _ = workspace.update(cx, |this, cx| {
                                this.apply_patch(file, three_way, window, cx)
                            });
                            true
                        })
                });
            });
        })
        .detach();
    }

    /// Apply a patch file to the files here.
    fn apply_patch(
        &mut self,
        file: String,
        three_way: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_mutation(
            "patch.apply",
            json!({ "file": file, "three_way": three_way }),
            if three_way {
                say!("dialogs-patch-applied-three-way").to_owned()
            } else {
                say!("dialogs-patch-applied").to_owned()
            },
            |_, _, _, _| false,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::preview;

    #[test]
    fn a_patch_that_applies_is_asked_about_with_what_it_changes() {
        let checked = json!({
            "applies": true, "reason": null, "total": 3,
            "files": [
                { "path": "src/a.rs", "added": 3, "deleted": 1, "binary": false },
                { "path": "b.txt", "added": 10, "deleted": 0, "binary": false },
                { "path": "img.png", "added": null, "deleted": null, "binary": true },
            ],
        });
        let asked = preview("/tmp/fixes/0001-fix.patch", &checked);
        assert_eq!(asked.title, "Apply 0001-fix.patch?");
        assert_eq!(
            asked.description,
            "It changes 3 files: src/a.rs (+3 −1), b.txt (+10 −0), img.png (binary). It \
             applies as it is; undo takes it out again."
        );
        assert!(asked.applies);
        let one = json!({
            "applies": true, "total": 1,
            "files": [{ "path": "a", "added": 1, "deleted": 1 }],
        });
        assert!(
            preview("p", &one)
                .description
                .starts_with("It changes 1 file: a (+1 −1).")
        );
    }

    #[test]
    fn a_patch_that_does_not_apply_says_why_and_offers_a_merge() {
        let checked = json!({
            "applies": false,
            "reason": "error: patch failed: a.txt:1\\nerror: a.txt: patch does not apply\\n",
            "total": 1,
            "files": [{ "path": "a.txt", "added": 1, "deleted": 1, "binary": false }],
        });
        let asked = preview("fix.patch", &checked);
        assert_eq!(asked.title, "fix.patch does not apply as it is");
        assert!(!asked.applies);
        assert!(
            asked.description.contains("patch failed: a.txt:1"),
            "{}",
            asked.description
        );
        assert!(asked.description.contains("three-way merge"));
    }

    #[test]
    fn many_files_are_counted_and_an_answer_that_is_none_says_little() {
        let files: Vec<_> = (0..30)
            .map(|n| json!({ "path": format!("f{n}"), "added": 1, "deleted": 0 }))
            .collect();
        let asked = preview(
            "big.patch",
            &json!({ "applies": true, "total": 412, "files": files }),
        );
        assert!(
            asked
                .description
                .starts_with("It changes 412 files: f0 (+1 −0), ")
        );
        assert!(
            asked.description.contains("f7 (+1 −0), and 404 more."),
            "{}",
            asked.description
        );
        for odd in [json!(null), json!([]), json!({ "files": 3, "total": "x" })] {
            let asked = preview("odd.patch", &odd);
            assert!(!asked.applies, "{odd}");
            assert!(
                asked.description.starts_with("Git finds no file in it"),
                "{odd}"
            );
        }
    }
}
