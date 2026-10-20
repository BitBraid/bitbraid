//! The dialogs about tags that carry a choice: a new tag that can be
//! signed, and which remote a tag goes to, or goes from, when there is
//! more than one.
//!
//! Sources: git-tag(1) (`-a`, `-s`), git-push(1).

use gpui_kit::*;
use serde_json::{Value, json};

use super::{
    PromptField, Workspace,
    form::{Check, Form, Pick, Verdict},
};

/// What there is to say about `typed` as the name of a new tag, with the
/// tags there are: git's rules for the names of references, then a name
/// that is taken.
pub(super) fn judge_tag_name(typed: &str, existing: &[String]) -> Option<Verdict> {
    if let Some(problem) = bitbraid_git::branch::name_problem(typed) {
        return Some(Verdict::Refuse(problem));
    }
    if existing.iter().any(|name| name == typed) {
        return Some(Verdict::Refuse(say!("dialogs-tag-exists", name = typed)));
    }
    None
}

/// The parameters of `tag.create` for what the dialog says. A signed tag
/// needs a message, as an annotated one does: without one it gets its
/// own name, which is what is customary.
pub(super) fn tag_params(name: &str, target: &str, message: &str, sign: bool) -> Value {
    let message = if sign && message.is_empty() {
        name
    } else {
        message
    };
    json!({ "name": name, "target": target, "message": message, "sign": sign })
}

/// The remotes to choose among, the one a tag goes to by default first
/// chosen: `origin` when there is one.
pub(super) fn remote_pick(remotes: &[String]) -> Pick {
    Pick {
        options: remotes
            .iter()
            .map(|remote| (remote.clone(), String::new()))
            .collect(),
        picked: remotes
            .iter()
            .position(|remote| remote == "origin")
            .unwrap_or(0),
    }
}

impl Workspace {
    /// Ask for a name and a message and make a tag at `target`; with a
    /// message it is an annotated tag, and it can be signed.
    pub(super) fn prompt_new_tag(
        &mut self,
        target: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let at = if target.len() == 40 && target.chars().all(|c| c.is_ascii_hexdigit()) {
            target.chars().take(7).collect::<String>()
        } else {
            target.clone()
        };
        let mut form = Form::new(say!("dialogs-new-tag-at", at), say!("workspace-create-tag"));
        form.fields = vec![
            PromptField::required(say!("workspace-prompt-name"), ""),
            PromptField::optional(say!("dialogs-tag-message-placeholder")),
        ];
        form.judge = Some(std::rc::Rc::new(|this: &Self, typed: &str, _: &[bool]| {
            let tags: Vec<String> = this
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.tags.iter().map(|tag| tag.name.clone()).collect())
                .unwrap_or_default();
            judge_tag_name(typed, &tags)
        }));
        form.checks = vec![Check::new(say!("dialogs-tag-sign"), false)];
        self.form(
            form,
            move |this, answers, window, cx| {
                let (name, sign) = (answers.text(0), answers.check(0));
                this.run_mutation(
                    "tag.create",
                    tag_params(&name, &target, &answers.text(1), sign),
                    if sign {
                        say!("dialogs-created-signed-tag", name = name.as_str())
                    } else {
                        say!("workspace-created-tag", name = name.as_str())
                    },
                    |_, _, _, _| false,
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Go on with the remote something is about: asked for when there is
    /// more than one, and otherwise none is named, which is the only one.
    pub(super) fn with_remote(
        &mut self,
        title: String,
        ok_text: &'static str,
        then: impl Fn(&mut Self, Option<String>, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remotes = self.remote_names();
        if remotes.len() < 2 {
            then(self, None, window, cx);
            return;
        }
        let mut form = Form::new(title, ok_text);
        form.pick = Some(remote_pick(&remotes));
        self.form(
            form,
            move |this, answers, window, cx| {
                let remote = remotes.get(answers.picked).cloned();
                then(this, remote, window, cx);
            },
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{judge_tag_name, remote_pick, tag_params};
    use crate::views::workspace::form::Verdict;

    #[test]
    fn a_new_tag_name_is_judged_as_it_is_typed() {
        let tags = vec!["v1.0".to_owned()];
        assert_eq!(judge_tag_name("", &tags), None);
        assert_eq!(judge_tag_name("v1.1", &tags), None);
        assert_eq!(
            judge_tag_name("v1.0", &tags),
            Some(Verdict::Refuse("There is a tag v1.0 already.".to_owned()))
        );
        assert!(matches!(
            judge_tag_name("v 2", &tags),
            Some(Verdict::Refuse(said)) if said.contains("a space")
        ));
    }

    #[test]
    fn a_signed_tag_without_a_message_gets_its_name() {
        assert_eq!(
            tag_params("v2", "main", "", false),
            json!({ "name": "v2", "target": "main", "message": "", "sign": false })
        );
        assert_eq!(
            tag_params("v2", "main", "", true),
            json!({ "name": "v2", "target": "main", "message": "v2", "sign": true })
        );
        assert_eq!(
            tag_params("v2", "main", "second release", true)["message"],
            "second release"
        );
    }

    #[test]
    fn origin_is_the_remote_that_starts_chosen() {
        let remotes = |names: &[&str]| -> Vec<String> {
            names.iter().map(|name| (*name).to_owned()).collect()
        };
        assert_eq!(remote_pick(&remotes(&["backup", "origin"])).picked, 1);
        assert_eq!(remote_pick(&remotes(&["backup", "mirror"])).picked, 0);
        assert_eq!(remote_pick(&[]).options.len(), 0);
    }
}
