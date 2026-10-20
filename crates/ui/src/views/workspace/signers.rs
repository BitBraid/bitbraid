//! The dialogs of the allowed signers: where the file is to be, somebody's
//! key and who they are, the key a commit was signed with, and the
//! question before a signer is taken out.
//!
//! What they change is neither a reference nor a file of the worktree, so
//! nothing of the graph moves; what changes is what git makes of the
//! signatures, and that is asked again where it is on show.
//!
//! Sources: ssh-keygen(1) ("ALLOWED SIGNERS").

use gpui_kit::component::WindowExt;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{
    Workspace,
    form::{Answers, Form, Pick, Row, Verdict},
};
use crate::{
    notices::Notice,
    session::{Reply, Request},
    views::signing::Signers,
};

/// The names typed for a signer: one, or several parted by commas or
/// blanks.
pub(super) fn principals_from(typed: &str) -> Vec<String> {
    typed
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Why what is typed cannot name a signer, if it cannot. The engine
/// checks again; this is what can be said while it is typed.
pub(super) fn principals_refusal(typed: &str) -> Option<String> {
    let names = principals_from(typed);
    let odd = names.iter().find(|name| {
        name.starts_with(['#', '-']) || name.contains('"') || name.chars().any(char::is_control)
    })?;
    Some(say!(
        "dialogs-signer-name-refused",
        odd = format!("{odd:?}")
    ))
}

/// The key of a new signer, from the two rows that take one: the path of
/// a public key file, or the key itself. `Err` says what is wrong, `Ok`
/// with nothing that neither is filled in yet.
pub(super) fn key_given(file: &str, key: &str) -> Result<Option<Value>, String> {
    match (file.is_empty(), key.is_empty()) {
        (true, true) => Ok(None),
        (false, false) => Err(say!("dialogs-signer-key-both").to_owned()),
        (false, true) => {
            let path = crate::shell::expand_home(file);
            if !path.is_absolute() {
                return Err(say!("dialogs-signer-key-path").to_owned());
            }
            Ok(Some(json!({ "key_file": path.to_string_lossy() })))
        }
        (true, false) => {
            if key.contains("PRIVATE KEY") {
                return Err(say!("dialogs-signer-key-private").to_owned());
            }
            if key.split_whitespace().count() < 2 {
                return Err(say!("dialogs-signer-key-incomplete").to_owned());
            }
            Ok(Some(json!({ "key": key })))
        }
    }
}

/// The parameters of `sign.signers_file` for what the dialog says: the
/// file, when one is named, and whose configuration names it.
pub(super) fn file_params(path: &str, for_everything: bool) -> Value {
    let mut params = json!({ "scope": if for_everything { "user" } else { "repo" } });
    if !path.is_empty()
        && let Some(fields) = params.as_object_mut()
    {
        fields.insert("path".to_owned(), json!(path));
    }
    params
}

/// What is said once the file is chosen.
pub(super) fn chosen_text(answer: &Value) -> String {
    let path = answer
        .get("path")
        .and_then(|path| path.as_str())
        .unwrap_or_else(|| say!("dialogs-signers-the-file"));
    if answer.get("created").and_then(|made| made.as_bool()) == Some(true) {
        say!("dialogs-signers-file-new", path)
    } else {
        say!("dialogs-signers-file-chosen", path)
    }
}

/// What is said once a signer is allowed.
pub(super) fn allowed_text(answer: &Value) -> String {
    let who = answer
        .get("principals")
        .and_then(|names| names.as_array())
        .map(|names| {
            names
                .iter()
                .filter_map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|who| !who.is_empty())
        .unwrap_or_else(|| say!("dialogs-signer-the-signer").to_owned());
    if answer.get("added").and_then(|added| added.as_bool()) == Some(false) {
        say!("dialogs-signer-allowed-already", who)
    } else {
        say!("dialogs-signer-allowed", who)
    }
}

/// What goes on once a file is chosen.
type Then = std::rc::Rc<dyn Fn(&mut Workspace, &mut Window, &mut Context<Workspace>)>;

/// Whether a step of the journal, by what it is called, changed who may
/// sign: a signer put in or taken out, or another file chosen. Taken back
/// or made again, the signatures on show are to be looked at again.
pub(super) fn about_signers(label: &str) -> bool {
    // not shown: how the engine's journal names these, which is compared with
    let labels = [
        "Allow signer ",
        "Remove signer ",
        "Choose the allowed-signers file",
    ];
    labels.iter().any(|start| label.starts_with(start))
}

impl Workspace {
    /// Who may sign has changed: what the signing view lists, and what
    /// git makes of the signatures that are on show.
    pub(super) fn signers_changed(&mut self, cx: &mut Context<Self>) {
        self.signing_view
            .update(cx, |view, cx| view.reload_signers(cx));
        self.graph_view
            .update(cx, |view, cx| view.check_ssh_signatures_again(cx));
        self.details_view
            .update(cx, |view, cx| view.verify_again(cx));
        self.commit_box.update(cx, |view, cx| view.load_signing(cx));
        self.peek_undo(cx);
    }

    /// Run one of the commands that change who may sign, say how it went
    /// with what `said` makes of the answer, and go on with `then`.
    fn run_signers(
        &mut self,
        id: &'static str,
        params: Value,
        said: impl Fn(&Value) -> String + 'static,
        then: Option<Then>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.refused_here(id, window, cx) {
            return;
        }
        let reply = self.session.send(Request::Command {
            id: id.to_owned(),
            params,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else { return };
            let _ = this.update_in(cx, |this, window, cx| match reply {
                Reply::Error(err) => {
                    let notice = this.failed_notice(&err, cx);
                    crate::notices::push(window, cx, notice);
                }
                Reply::Command(answer) => {
                    crate::notices::push(window, cx, Notice::success(said(&answer)));
                    this.signers_changed(cx);
                    if let Some(then) = &then {
                        then(this, window, cx);
                    }
                }
                _ => {}
            });
        })
        .detach();
    }

    /// Ask where the allowed signers are to be kept and whose settings
    /// say so; `then` goes on once that is done.
    pub(super) fn prompt_signers_file(
        &mut self,
        then: Option<Then>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut form = Form::new(say!("dialogs-signers-file-title"), say!("dialogs-choose"));
        form.lead = Some(say!("dialogs-signers-file-lead").to_owned());
        form.rows = vec![Row::optional(
            say!("dialogs-signers-file"),
            "",
            say!("dialogs-signers-file-placeholder"),
        )];
        // A repository that is only read keeps its settings as they are.
        let read_only = self.read_only();
        if !read_only {
            form.pick = Some(Pick {
                options: vec![
                    (
                        say!("dialogs-signers-for-repository").to_owned(),
                        say!("dialogs-signers-for-repository-more").to_owned(),
                    ),
                    (
                        say!("dialogs-signers-for-all").to_owned(),
                        say!("dialogs-signers-for-all-more").to_owned(),
                    ),
                ],
                picked: 0,
            });
        }
        self.form(
            form,
            move |this, answers, window, cx| {
                let everything = read_only || answers.picked == 1;
                this.run_signers(
                    "sign.signers_file",
                    file_params(&answers.row(0), everything),
                    chosen_text,
                    then.clone(),
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Do `go_on` once there is an allowed-signers file: at once when one
    /// is chosen, after asking for one when none is.
    fn with_signers_file(&mut self, go_on: Then, window: &mut Window, cx: &mut Context<Self>) {
        let reply = self.session.send(Request::Command {
            id: "sign.signers".to_owned(),
            params: json!({}),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else { return };
            let _ = this.update_in(cx, |this, window, cx| match reply {
                Reply::Command(answer) if Signers::from_json(&answer).path.is_some() => {
                    go_on(this, window, cx)
                }
                Reply::Command(_) => this.prompt_signers_file(Some(go_on), window, cx),
                Reply::Error(err) => {
                    let notice = this.failed_notice(&err, cx);
                    crate::notices::push(window, cx, notice);
                }
                _ => {}
            });
        })
        .detach();
    }

    /// Ask for whom the key a commit was signed with is to count, and
    /// allow it; the file is asked for first when none is chosen.
    pub(super) fn prompt_allow_signer_of(
        &mut self,
        commit: bitbraid_git::Oid,
        address: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let go_on: Then = std::rc::Rc::new(move |this, window, cx| {
            let mut form = Form::new(
                say!("dialogs-signer-allow-key-of", id = commit.short(7)),
                say!("dialogs-allow"),
            );
            form.lead = Some(say!("dialogs-signer-allow-key-lead").to_owned());
            form.rows = vec![Row::required(say!("dialogs-signer"), address.clone())];
            form.review = Some(std::rc::Rc::new(|_: &Workspace, answers: &Answers| {
                principals_refusal(&answers.row(0)).map(Verdict::Refuse)
            }));
            this.form(
                form,
                move |this, answers, window, cx| {
                    this.run_signers(
                        "sign.allow",
                        json!({
                            "commit": commit.to_hex(),
                            "principals": principals_from(&answers.row(0)),
                        }),
                        allowed_text,
                        None,
                        window,
                        cx,
                    );
                },
                window,
                cx,
            );
        });
        self.with_signers_file(go_on, window, cx);
    }

    /// The palette's way to allow a signer.
    pub fn allow_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_add_signer(window, cx);
    }

    /// The palette's way to say where the allowed signers are kept.
    pub fn choose_signers_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prompt_signers_file(None, window, cx);
    }

    /// Ask for somebody's public key and who they are, and allow them.
    pub(super) fn prompt_add_signer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let go_on: Then = std::rc::Rc::new(|this, window, cx| {
            let mut form = Form::new(say!("dialogs-signer-add-title"), say!("dialogs-allow"));
            form.lead = Some(say!("dialogs-signer-add-lead").to_owned());
            form.rows = vec![
                Row {
                    placeholder: say!("dialogs-signer-placeholder"),
                    ..Row::required(say!("dialogs-signer"), "")
                },
                Row::optional(
                    say!("dialogs-signer-key-file"),
                    "",
                    say!("dialogs-signer-key-file-placeholder"),
                ),
                Row::optional(
                    say!("dialogs-signer-key"),
                    "",
                    say!("dialogs-signer-key-placeholder"),
                ),
            ];
            form.review = Some(std::rc::Rc::new(|_: &Workspace, answers: &Answers| {
                if let Some(refusal) = principals_refusal(&answers.row(0)) {
                    return Some(Verdict::Refuse(refusal));
                }
                match key_given(&answers.row(1), &answers.row(2)) {
                    Err(refusal) => Some(Verdict::Refuse(refusal)),
                    Ok(None) => Some(Verdict::Refuse(
                        say!("dialogs-signer-key-missing").to_owned(),
                    )),
                    Ok(Some(_)) => None,
                }
            }));
            this.form(
                form,
                |this, answers, window, cx| {
                    let Ok(Some(mut params)) = key_given(&answers.row(1), &answers.row(2)) else {
                        return;
                    };
                    if let Some(fields) = params.as_object_mut() {
                        fields.insert(
                            "principals".to_owned(),
                            json!(principals_from(&answers.row(0))),
                        );
                    }
                    this.run_signers("sign.allow", params, allowed_text, None, window, cx);
                },
                window,
                cx,
            );
        });
        self.with_signers_file(go_on, window, cx);
    }

    /// Ask, then take a signer out of the allowed ones.
    pub(super) fn ask_remove_signer(
        &mut self,
        fingerprint: String,
        principals: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let who = principals.join(", ");
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (workspace, fingerprint, principals, who) = (
                workspace.clone(),
                fingerprint.clone(),
                principals.clone(),
                who.clone(),
            );
            alert
                .title(say!("dialogs-signer-remove-title", who = who.as_str()))
                .description(say!(
                    "dialogs-signer-remove-description",
                    fingerprint = fingerprint.as_str(),
                    who = who.as_str()
                ))
                .show_cancel(true)
                .on_ok(move |_, window, cx| {
                    let (fingerprint, principals, who) =
                        (fingerprint.clone(), principals.clone(), who.clone());
                    let _ = workspace.update(cx, |this, cx| {
                        this.run_signers(
                            "sign.disallow",
                            json!({ "key": fingerprint, "principals": principals }),
                            move |_| say!("dialogs-signer-removed", who = who.as_str()),
                            None,
                            window,
                            cx,
                        )
                    });
                    true
                })
        });
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        about_signers, allowed_text, chosen_text, file_params, key_given, principals_from,
        principals_refusal,
    };

    #[test]
    fn steps_that_change_who_may_sign_are_told_by_their_names() {
        // As the engine calls them (`signers_cmds.rs`).
        assert!(about_signers("Allow signer ada@fixture.invalid"));
        assert!(about_signers(
            "Remove signer ada@fixture.invalid, ada@home.invalid"
        ));
        assert!(about_signers("Choose the allowed-signers file"));
        assert!(!about_signers("Commit: Allow signer keys"));
        assert!(!about_signers("Merge topic"));
        assert!(!about_signers(""));
    }

    #[test]
    fn names_are_parted_by_commas_or_blanks() {
        assert_eq!(
            principals_from("ada@fixture.invalid"),
            ["ada@fixture.invalid"]
        );
        assert_eq!(
            principals_from(" ada@fixture.invalid, ada@home.invalid\t*@ops.invalid ,, "),
            ["ada@fixture.invalid", "ada@home.invalid", "*@ops.invalid"]
        );
        assert!(principals_from(" , ").is_empty());
        assert_eq!(
            principals_refusal("ada@fixture.invalid, *@ops.invalid"),
            None
        );
        for odd in ["#ada", "-ada", "\"ada\"", "ada@x.invalid \"b\""] {
            assert!(principals_refusal(odd).is_some(), "{odd:?}");
        }
    }

    #[test]
    fn a_key_is_a_file_or_the_key_itself() {
        assert_eq!(key_given("", ""), Ok(None));
        // A full path begins with a drive on Windows.
        let file = if cfg!(windows) {
            r"C:\keys\ada.pub"
        } else {
            "/keys/ada.pub"
        };
        assert_eq!(key_given(file, ""), Ok(Some(json!({ "key_file": file }))));
        assert_eq!(
            key_given("", "ssh-ed25519 AAAAC3Nz ada"),
            Ok(Some(json!({ "key": "ssh-ed25519 AAAAC3Nz ada" })))
        );
        assert!(key_given(file, "ssh-ed25519 AAAA").is_err());
        assert!(key_given("ada.pub", "").is_err());
        assert!(key_given("", "ssh-ed25519").is_err());
        let err = key_given("", "-----BEGIN OPENSSH PRIVATE KEY-----").unwrap_err();
        assert!(err.contains("public half"), "{err}");
    }

    #[test]
    fn the_file_is_named_for_one_repository_or_for_all() {
        assert_eq!(file_params("", false), json!({ "scope": "repo" }));
        assert_eq!(
            file_params("~/.ssh/allowed_signers", true),
            json!({ "scope": "user", "path": "~/.ssh/allowed_signers" })
        );
        assert_eq!(
            chosen_text(&json!({ "path": "/x/allowed", "created": true })),
            "SSH signatures are checked against /x/allowed now; it is new, and nobody is in it \
             yet"
        );
        assert_eq!(
            chosen_text(&json!({ "path": "/x/allowed", "created": false })),
            "SSH signatures are checked against /x/allowed now"
        );
        assert_eq!(
            allowed_text(&json!({ "principals": ["ada@fixture.invalid"], "added": true })),
            "ada@fixture.invalid may sign with that key now"
        );
        assert_eq!(
            allowed_text(&json!({ "principals": ["a@x.invalid", "b@x.invalid"], "added": false })),
            "That key was allowed for a@x.invalid, b@x.invalid already"
        );
        assert_eq!(
            allowed_text(&json!({})),
            "The signer may sign with that key now"
        );
    }
}
