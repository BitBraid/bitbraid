//! Gitflow and branch housekeeping requests: the menus of the references
//! panel make them, the workspace asks where it has to and runs them.
//! Also what `flow.status` and `branch.overview` answered, as the panel
//! shows it.
//!
//! Sources: gpui-kit documentation (popup menu).

use std::collections::{BTreeMap, HashMap};

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use serde::Deserialize;

/// The kinds of branch gitflow has, in the order they are listed.
pub const KINDS: [&str; 5] = ["feature", "bugfix", "release", "hotfix", "support"];

/// Whether a branch of this kind is started from the main branch (what
/// mends or carries on something released) rather than from develop.
pub fn from_main(kind: &str) -> bool {
    matches!(kind, "hotfix" | "support")
}

/// Whether a branch of this kind is ever finished: a support branch stays.
pub fn finishes(kind: &str) -> bool {
    kind != "support"
}

/// One request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum FlowRequest {
    /// Ask for the two branch names and the prefixes, and set gitflow up
    /// or change how it is set up.
    SetUp,
    /// Ask for a name and start a branch of this kind: one of [`KINDS`].
    Start(String),
    /// Ask for a kind and a name and start that branch.
    StartAny,
    /// Finish the branch that is checked out, when it is one of gitflow's.
    FinishCurrent,
    /// Ask, then finish a branch.
    Finish {
        /// `feature`, `bugfix`, `release` or `hotfix`.
        kind: String,
        /// Its name without the prefix.
        name: String,
        /// Leave the branch there afterwards.
        keep: bool,
    },
    /// Ask for a description of this local branch.
    Describe(String),
    /// Show which local branches are merged, and delete them when that is
    /// confirmed.
    DeleteMerged,
}

/// The action a menu entry dispatches.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = flow, no_json)]
pub struct RunFlow {
    /// What to do.
    pub request: FlowRequest,
}

fn entry(request: FlowRequest) -> Box<dyn Action> {
    Box::new(RunFlow { request })
}

/// A branch of one of gitflow's kinds there is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlowBranch {
    /// One of [`KINDS`].
    pub kind: &'static str,
    /// Its name without the prefix.
    pub name: String,
    /// The branch itself.
    pub branch: String,
    /// The commit it is at.
    pub tip: Option<String>,
    /// It is checked out.
    pub current: bool,
}

/// What `flow.status` answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlowState {
    /// Gitflow is set up here.
    pub initialized: bool,
    /// The main branch, as configured.
    pub main: Option<String>,
    /// The develop branch, as configured.
    pub develop: Option<String>,
    /// What the branches of each kind begin with, by kind, and the tags
    /// under `tag`: as the engine says they are in effect.
    pub prefixes: BTreeMap<String, String>,
    /// The branches of the five kinds, features first.
    pub branches: Vec<FlowBranch>,
}

impl FlowState {
    /// What the branches of a kind begin with: what is in effect, and the
    /// usual prefix while nothing is known.
    pub fn prefix_of(&self, kind: &str) -> String {
        self.prefixes
            .get(kind)
            .cloned()
            .unwrap_or_else(|| format!("{kind}/"))
    }

    /// What the tags of releases and hotfixes begin with; often nothing.
    pub fn tag_prefix(&self) -> String {
        self.prefixes.get("tag").cloned().unwrap_or_default()
    }

    /// The branch a kind is started from: main or develop, as configured,
    /// or by their usual names while gitflow is not set up.
    pub fn base_of(&self, kind: &str) -> String {
        if from_main(kind) {
            self.main.clone().unwrap_or_else(|| "main".to_owned())
        } else {
            self.develop.clone().unwrap_or_else(|| "develop".to_owned())
        }
    }

    /// Read the answer.
    pub fn from_json(value: &serde_json::Value) -> Self {
        let text = |value: &serde_json::Value, key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let mut branches = Vec::new();
        for (key, kind) in [
            ("features", "feature"),
            ("bugfixes", "bugfix"),
            ("releases", "release"),
            ("hotfixes", "hotfix"),
            ("supports", "support"),
        ] {
            for found in value
                .get(key)
                .and_then(|list| list.as_array())
                .into_iter()
                .flatten()
            {
                let (Some(name), Some(branch)) = (text(found, "name"), text(found, "branch"))
                else {
                    continue;
                };
                branches.push(FlowBranch {
                    kind,
                    name,
                    branch,
                    tip: text(found, "tip"),
                    current: found.get("current").and_then(|flag| flag.as_bool()) == Some(true),
                });
            }
        }
        Self {
            initialized: value.get("initialized").and_then(|flag| flag.as_bool()) == Some(true),
            main: text(value, "main"),
            develop: text(value, "develop"),
            prefixes: value
                .get("prefixes")
                .and_then(|prefixes| prefixes.as_object())
                .into_iter()
                .flatten()
                .filter_map(|(key, prefix)| Some((key.clone(), prefix.as_str()?.to_owned())))
                .collect(),
            branches,
        }
    }
}

/// What is asked when gitflow is set up or its settings are changed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FlowSettings {
    /// The branch releases are made on.
    pub main: String,
    /// The branch work comes together on.
    pub develop: String,
    /// What the branches of each kind begin with, in the order of
    /// [`KINDS`].
    pub prefixes: [String; 5],
    /// What the tags of releases and hotfixes begin with; may be empty.
    pub tag: String,
}

impl FlowSettings {
    /// The settings as they are, or as they would usually be while there
    /// are none: the main branch is the one of the two usual names that
    /// is there.
    pub fn of(state: &FlowState, local: &[String]) -> Self {
        let has = |name: &str| local.iter().any(|branch| branch == name);
        Self {
            main: state.main.clone().unwrap_or_else(|| {
                if has("master") && !has("main") {
                    "master".to_owned()
                } else {
                    "main".to_owned()
                }
            }),
            develop: state
                .develop
                .clone()
                .unwrap_or_else(|| "develop".to_owned()),
            prefixes: KINDS.map(|kind| state.prefix_of(kind)),
            tag: state.tag_prefix(),
        }
    }

    /// The parameters of `flow.init` that write all of it.
    pub fn params(&self) -> serde_json::Value {
        let mut params = serde_json::json!({
            "main": self.main,
            "develop": self.develop,
            "tag": self.tag,
        });
        if let Some(fields) = params.as_object_mut() {
            for (kind, prefix) in KINDS.iter().zip(&self.prefixes) {
                fields.insert((*kind).to_owned(), serde_json::json!(prefix));
            }
        }
        params
    }

    /// What there is to say about these settings before they are written:
    /// `Err` with why they cannot be, `Ok` with what is worth knowing, if
    /// anything. The engine checks all of it again; this is what can be
    /// told while it is typed.
    pub fn reviewed(&self, state: &FlowState, local: &[String]) -> Result<Option<String>, String> {
        let has = |name: &str| local.iter().any(|branch| branch == name);
        if self.main.is_empty() || self.develop.is_empty() {
            return Ok(None);
        }
        if self.main == self.develop {
            return Err(say!("flow-same-branches", branch = &self.main));
        }
        if !has(&self.main) {
            return Err(say!("flow-no-such-branch", branch = &self.main));
        }
        for (ix, (kind, prefix)) in KINDS.iter().zip(&self.prefixes).enumerate() {
            if prefix.is_empty() {
                return Ok(None);
            }
            if !begins_a_name(prefix) {
                return Err(say!(
                    "flow-prefix-cannot-begin",
                    prefix = format!("{prefix:?}"),
                    kind = *kind
                ));
            }
            let shared = KINDS
                .iter()
                .zip(&self.prefixes)
                .skip(ix.saturating_add(1))
                .find(|(_, other)| *other == prefix);
            if let Some((other, _)) = shared {
                let mut first = (*kind).to_owned();
                if let Some(initial) = first.get_mut(..1) {
                    initial.make_ascii_uppercase();
                }
                return Err(say!("flow-prefix-shared", first, other = *other, prefix));
            }
        }
        if !self.tag.is_empty() && !begins_a_name(&self.tag) {
            return Err(say!(
                "flow-tag-prefix-cannot-begin",
                prefix = format!("{:?}", self.tag)
            ));
        }
        let mut remarks = Vec::new();
        if !has(&self.develop) {
            remarks.push(say!(
                "flow-develop-made",
                develop = &self.develop,
                main = &self.main
            ));
        }
        // Branches there are under a prefix that is given up are no
        // longer of their kind.
        for (kind, prefix) in KINDS.iter().zip(&self.prefixes) {
            let old = state.prefix_of(kind);
            let left = state
                .branches
                .iter()
                .filter(|branch| branch.kind == *kind && !branch.branch.starts_with(prefix))
                .count();
            if left > 0 && old != *prefix {
                remarks.push(say!("flow-branches-left", count = left, old, kind = *kind));
            }
        }
        Ok((!remarks.is_empty()).then(|| remarks.join(" ")))
    }
}

/// Whether a text can be the start of a branch's or a tag's name, as far
/// as can be told without asking git: no blank, no control character, no
/// leading dash, slash or dot, and none of the signs git keeps for itself.
fn begins_a_name(prefix: &str) -> bool {
    !(prefix.starts_with(['-', '/', '.'])
        || prefix.contains("..")
        || prefix.contains("//")
        || prefix.contains("@{")
        || prefix.contains("/.")
        || prefix
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || "~^:?*[\\".contains(c)))
}

/// What starting a branch will do, said while its name is typed: `Err`
/// when there is such a branch already.
pub fn start_explained(
    kind: &str,
    name: &str,
    state: &FlowState,
    local: &[String],
) -> Result<String, String> {
    let branch = format!("{}{name}", state.prefix_of(kind));
    let base = state.base_of(kind);
    if name.is_empty() {
        return Ok(say!(
            "flow-start-any-name",
            prefix = state.prefix_of(kind),
            base
        ));
    }
    if local.contains(&branch) {
        return Err(say!("flow-branch-exists", branch));
    }
    Ok(say!("flow-start-explained", branch, base))
}

/// What is worth saying about a local branch beside its name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BranchNote {
    /// What the user wrote about it.
    pub description: Option<String>,
    /// Its last commit is old.
    pub stale: bool,
    /// The branch it followed on a remote is gone.
    pub gone: bool,
}

/// Read `branch.overview` into notes by branch name; a branch about which
/// there is nothing to say is left out.
pub fn notes_from(value: &serde_json::Value) -> HashMap<String, BranchNote> {
    value
        .get("branches")
        .and_then(|branches| branches.as_array())
        .into_iter()
        .flatten()
        .filter_map(|branch| {
            let flag = |key: &str| branch.get(key).and_then(|flag| flag.as_bool()) == Some(true);
            let note = BranchNote {
                description: branch
                    .get("description")
                    .and_then(|text| text.as_str())
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(str::to_owned),
                stale: flag("stale"),
                gone: flag("gone"),
            };
            let name = branch.get("name")?.as_str()?.to_owned();
            (note != BranchNote::default()).then_some((name, note))
        })
        .collect()
}

/// The entries under the heading of the gitflow section.
pub fn heading_entries(menu: PopupMenu, initialized: bool) -> PopupMenu {
    let start = |menu: PopupMenu, label: &'static str, kind: &str| {
        menu.menu_with_disabled(
            if initialized {
                label.to_owned()
            } else {
                say!("flow-set-up-first", label = label.trim_end_matches('…'))
            },
            entry(FlowRequest::Start(kind.to_owned())),
            !initialized,
        )
    };
    let menu = start(menu, say!("flow-start-feature"), "feature");
    let menu = start(menu, say!("flow-start-bugfix"), "bugfix");
    let menu = start(menu, say!("flow-start-release"), "release");
    let menu = start(menu, say!("flow-start-hotfix"), "hotfix");
    let menu = start(menu, say!("flow-start-support"), "support");
    menu.separator().menu(
        if initialized {
            say!("flow-branches-and-prefixes")
        } else {
            say!("flow-set-up")
        },
        entry(FlowRequest::SetUp),
    )
}

/// Menu entries for one branch of gitflow's.
pub fn branch_entries(menu: PopupMenu, kind: &str, name: &str) -> PopupMenu {
    if !finishes(kind) {
        return menu.label(say!("flow-support-stays"));
    }
    let finish = |keep| {
        entry(FlowRequest::Finish {
            kind: kind.to_owned(),
            name: name.to_owned(),
            keep,
        })
    };
    menu.menu(say!("flow-finish-branch", kind, name), finish(false))
        .menu(say!("flow-finish-and-keep"), finish(true))
}

/// The entry that asks for a branch's description.
pub fn describe_entry(menu: PopupMenu, name: &str, described: bool) -> PopupMenu {
    menu.menu(
        if described {
            say!("flow-change-description")
        } else {
            say!("flow-describe")
        },
        entry(FlowRequest::Describe(name.to_owned())),
    )
}

/// The entry that cleans merged branches up.
pub fn delete_merged_entry(menu: PopupMenu) -> PopupMenu {
    menu.menu(say!("flow-delete-merged"), entry(FlowRequest::DeleteMerged))
}

/// What finishing a branch will do, for the question that is asked first.
pub fn finish_explained(kind: &str, state: &FlowState, keep: bool) -> String {
    let main = state.main.as_deref().unwrap_or("main");
    let develop = state.develop.as_deref().unwrap_or("develop");
    // Short: it is asked in the bar, one line with its buttons.
    match (matches!(kind, "feature" | "bugfix"), keep) {
        (true, true) => say!("flow-finish-into-develop-kept", develop),
        (true, false) => say!("flow-finish-into-develop-deleted", develop),
        (false, true) => say!("flow-finish-released-kept", main, develop),
        (false, false) => say!("flow-finish-released-deleted", main, develop),
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{
        BranchNote, FlowSettings, FlowState, KINDS, begins_a_name, finish_explained, finishes,
        from_main, notes_from, start_explained,
    };

    #[test]
    fn the_flow_status_is_read_into_branches_by_kind() {
        let state = FlowState::from_json(&json!({
            "initialized": true,
            "main": "main",
            "develop": "develop",
            "features": [
                { "name": "login", "branch": "feature/login", "tip": "abc", "current": true },
                { "name": "", "branch": "feature/" },
            ],
            "bugfixes": [{ "name": "typo", "branch": "fix/typo", "tip": "123" }],
            "releases": [{ "name": "1.2", "branch": "release/1.2", "tip": "def", "current": false }],
            "hotfixes": [],
            "supports": [{ "name": "1.x", "branch": "support/1.x" }],
            "prefixes": { "feature": "feature/", "bugfix": "fix/", "tag": "v", "odd": 7 },
        }));
        assert!(state.initialized);
        let seen: Vec<(&str, &str, bool)> = state
            .branches
            .iter()
            .map(|branch| (branch.kind, branch.branch.as_str(), branch.current))
            .collect();
        assert_eq!(
            seen,
            [
                ("feature", "feature/login", true),
                ("bugfix", "fix/typo", false),
                ("release", "release/1.2", false),
                ("support", "support/1.x", false),
            ]
        );
        // The prefixes as they are in effect; the usual one where the
        // answer has none.
        assert_eq!(state.prefix_of("bugfix"), "fix/");
        assert_eq!(state.prefix_of("hotfix"), "hotfix/");
        assert_eq!(state.tag_prefix(), "v");
        assert_eq!(state.base_of("feature"), "develop");
        assert_eq!(state.base_of("support"), "main");
        assert_eq!(
            finish_explained("bugfix", &state, false),
            finish_explained("feature", &state, false)
        );
        assert_eq!(
            finish_explained("feature", &state, false),
            "It is merged into develop, then deleted."
        );
        assert_eq!(
            finish_explained("release", &state, true),
            "It is merged into main, tagged, merged into develop, then kept."
        );
        assert_eq!(
            FlowState::from_json(&json!("nonsense")),
            FlowState::default()
        );
    }

    #[test]
    fn each_kind_starts_where_it_belongs_and_all_but_support_finish() {
        let from: Vec<&str> = KINDS.into_iter().filter(|kind| from_main(kind)).collect();
        assert_eq!(from, ["hotfix", "support"]);
        let never: Vec<&str> = KINDS.into_iter().filter(|kind| !finishes(kind)).collect();
        assert_eq!(never, ["support"]);

        let state = FlowState::from_json(&json!({
            "initialized": true, "main": "trunk", "develop": "next",
            "prefixes": { "bugfix": "fix/" },
        }));
        let local = ["trunk", "next", "fix/typo"].map(str::to_owned);
        assert_eq!(
            start_explained("bugfix", "crash", &state, &local).as_deref(),
            Ok("fix/crash is made from next and checked out.")
        );
        assert_eq!(
            start_explained("support", "1.x", &state, &local).as_deref(),
            Ok("support/1.x is made from trunk and checked out.")
        );
        assert_eq!(
            start_explained("bugfix", "typo", &state, &local),
            Err("There is a branch fix/typo already.".to_owned())
        );
        // Nothing typed yet: where it will go.
        assert_eq!(
            start_explained("hotfix", "", &state, &local).as_deref(),
            Ok("A branch that begins with hotfix/ is made from trunk and checked out.")
        );
    }

    #[test]
    fn the_settings_are_looked_at_while_they_are_typed() {
        let state = FlowState::from_json(&json!({
            "initialized": true, "main": "main", "develop": "develop",
            "features": [
                { "name": "a", "branch": "feature/a" },
                { "name": "b", "branch": "feature/b" },
            ],
            "bugfixes": [{ "name": "c", "branch": "bugfix/c" }],
            "prefixes": {
                "feature": "feature/", "bugfix": "bugfix/", "release": "release/",
                "hotfix": "hotfix/", "support": "support/", "tag": "",
            },
        }));
        let local = ["main", "develop", "feature/a", "feature/b", "bugfix/c"].map(str::to_owned);
        let as_they_are = FlowSettings::of(&state, &local);
        assert_eq!(as_they_are.main, "main");
        assert_eq!(
            as_they_are.prefixes,
            ["feature/", "bugfix/", "release/", "hotfix/", "support/"].map(str::to_owned)
        );
        // Unchanged: nothing to say.
        assert_eq!(as_they_are.reviewed(&state, &local), Ok(None));
        assert_eq!(
            as_they_are.params(),
            json!({
                "main": "main", "develop": "develop", "tag": "",
                "feature": "feature/", "bugfix": "bugfix/", "release": "release/",
                "hotfix": "hotfix/", "support": "support/",
            })
        );

        let with = |change: &dyn Fn(&mut FlowSettings)| {
            let mut settings = as_they_are.clone();
            change(&mut settings);
            settings.reviewed(&state, &local)
        };
        assert_eq!(
            with(&|s| s.develop = "main".to_owned()),
            Err("The main and the develop branch cannot both be main.".to_owned())
        );
        assert_eq!(
            with(&|s| s.main = "trunk".to_owned()),
            Err("There is no branch trunk here.".to_owned())
        );
        assert_eq!(
            with(&|s| s.prefixes[1] = "feature/".to_owned()),
            Err("Feature and bugfix branches cannot both begin with feature/.".to_owned())
        );
        assert_eq!(
            with(&|s| s.prefixes[4] = "hotfix/".to_owned()),
            Err("Hotfix and support branches cannot both begin with hotfix/.".to_owned())
        );
        assert_eq!(
            with(&|s| s.prefixes[2] = "rel ease/".to_owned()),
            Err("\"rel ease/\" cannot begin the name of a release branch.".to_owned())
        );
        assert_eq!(
            with(&|s| s.tag = "v 1".to_owned()),
            Err("\"v 1\" cannot begin the name of a tag.".to_owned())
        );
        // Being typed: a field that is empty for now is not a fault yet.
        assert_eq!(with(&|s| s.prefixes[0] = String::new()), Ok(None));
        assert_eq!(with(&|s| s.main = String::new()), Ok(None));
        // What is worth knowing: a branch that will be made, and branches
        // that stop being of their kind.
        assert_eq!(
            with(&|s| s.develop = "next".to_owned()),
            Ok(Some("The branch next is made from main.".to_owned()))
        );
        assert_eq!(
            with(&|s| {
                s.prefixes[0] = "feat/".to_owned();
                s.prefixes[1] = "fix/".to_owned();
            }),
            Ok(Some(
                "2 branches that begin with feature/ are then no feature branches any more. 1 \
                 branch that begins with bugfix/ is then no bugfix branch any more."
                    .to_owned()
            ))
        );
        assert_eq!(with(&|s| s.tag = "v".to_owned()), Ok(None));

        // Not set up yet: the usual names, and master where that is what
        // there is.
        let fresh = FlowSettings::of(&FlowState::default(), &["master".to_owned()]);
        assert_eq!(
            (fresh.main.as_str(), fresh.develop.as_str()),
            ("master", "develop")
        );
        assert_eq!(fresh.prefixes[3], "hotfix/");
        assert_eq!(fresh.tag, "");
    }

    #[test]
    fn what_cannot_begin_a_name_is_told_from_what_can() {
        for good in ["feature/", "f-", "team/feature/", "v", "release-1."] {
            assert!(begins_a_name(good), "{good:?}");
        }
        for bad in [
            "-x/", "/x/", ".x/", "a b/", "a\tb", "a..b/", "a//b/", "x~/", "x^", "a:b", "a?", "a*",
            "a[", "a\\b/", "@{", "a/.b/", "a\nb",
        ] {
            assert!(!begins_a_name(bad), "{bad:?}");
        }
    }

    #[test]
    fn only_branches_with_something_to_say_get_a_note() {
        let notes = notes_from(&json!({
            "branches": [
                { "name": "main", "description": null, "stale": false, "gone": false },
                { "name": "old", "description": "  parked  ", "stale": true, "gone": false },
                { "name": "orphan", "description": "", "stale": false, "gone": true },
                { "description": "no name" },
            ],
        }));
        assert_eq!(notes.len(), 2);
        assert_eq!(
            notes["old"],
            BranchNote {
                description: Some("parked".to_owned()),
                stale: true,
                gone: false,
            }
        );
        assert!(notes["orphan"].gone && notes["orphan"].description.is_none());
        assert!(notes_from(&json!({})).is_empty());
    }
}
