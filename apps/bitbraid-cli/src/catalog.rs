//! Command catalog export: one JSON document describing every registered
//! command (`bitbraid-cli commands --json`).
//!
//! Each row says what the command is, what it takes, and what it needs of
//! a repository and may do to one, so that whoever drives the engine from
//! outside can tell what is safe to run where before running it.

use bitbraid_engine::{Access, Registry};
use serde::Serialize;

/// One catalog row.
#[derive(Debug, Serialize)]
pub struct Entry<'a> {
    pub id: &'a str,
    /// Label shown to users.
    pub label: &'a str,
    /// One-line description.
    pub description: &'a str,
    /// What the command needs and may do, as the registry has it (the
    /// engine's table, `bitbraid_engine::access`, or what a command of a
    /// higher layer brought): `repository` is `repository` (it
    /// cannot run without an open one), `repository_if_open` or `nothing`;
    /// `effect` is `reads`, `writes`, or `writes_scope` (it writes the
    /// configuration its `scope` parameter names: the repository's unless
    /// that is `"user"`). A command the table does not know is taken for
    /// one that needs a repository and changes it.
    pub access: Access,
    /// What the command takes, one row per parameter; absent when nothing
    /// is known about them.
    pub params: Option<Vec<Param>>,
}

/// One parameter of a command.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Param {
    /// Its key in the parameters object.
    pub name: &'static str,
    /// `text`, `flag`, `number`, `path`, `revision`, `list`, `choice` or
    /// `structured`.
    pub kind: &'static str,
    /// The words a choice is among.
    #[serde(skip_serializing_if = "<[_]>::is_empty")]
    pub choices: &'static [&'static str],
    /// Whether the command refuses to run without it.
    pub required: bool,
    /// A few words about it.
    pub description: &'static str,
}

/// Catalog rows in id order.
pub fn entries(registry: &Registry) -> Vec<Entry<'_>> {
    registry
        .iter()
        .map(|spec| Entry {
            id: spec.id,
            label: spec.label,
            description: spec.description,
            access: registry.access(spec.id).unwrap_or(Access::WRITES),
            params: registry.params(spec.id).map(|params| {
                params
                    .iter()
                    .map(|param| Param {
                        name: param.name,
                        kind: param.kind.name(),
                        choices: param.kind.choices(),
                        required: param.required,
                        description: param.description.text(),
                    })
                    .collect()
            }),
        })
        .collect()
}

/// Pretty-printed JSON array of the catalog.
pub fn to_json(registry: &Registry) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&entries(registry))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]
mod tests {
    use bitbraid_engine::{CommandContext, CommandError, CommandSpec, Output, Params};

    use super::*;

    fn noop(_: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
        Ok(Output::Null)
    }

    #[test]
    fn exports_registered_commands() {
        let mut reg = Registry::new();
        reg.register(
            CommandSpec {
                id: "repo.open",
                label: "Open",
                description: "Open a repository",
            },
            noop,
        )
        .unwrap();
        let json = to_json(&reg).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed[0]["id"], "repo.open");
        // `repo.open` is in the engine's table of parameters.
        assert_eq!(parsed[0]["params"][0]["name"], "path");
        assert_eq!(parsed[0]["params"][0]["kind"], "path");
        assert_eq!(parsed[0]["params"][0]["required"], false);
        assert!(parsed[0]["params"][0].get("choices").is_none());
        // And in its table of what commands need and do: opening changes
        // which repository is open, and no repository.
        assert_eq!(
            parsed[0]["access"],
            serde_json::json!({ "repository": "nothing", "effect": "reads" })
        );
    }

    #[test]
    fn the_built_in_catalog_says_what_each_command_needs_and_does() {
        let registry = bitbraid_integrations::registry();
        let rows = entries(&registry);
        assert_eq!(rows.len(), registry.len());
        // Every row says the same as the registry does.
        for row in &rows {
            assert_eq!(Some(row.access), registry.access(row.id), "{}", row.id);
        }
        let listed: serde_json::Value = serde_json::from_str(&to_json(&registry).unwrap()).unwrap();
        let access = |id: &str| {
            listed
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["id"] == id)
                .unwrap_or_else(|| panic!("{id} is not listed"))["access"]
                .clone()
        };
        let says = |repository: &str, effect: &str| serde_json::json!({ "repository": repository, "effect": effect });
        assert_eq!(access("repo.init"), says("nothing", "reads"));
        assert_eq!(access("repo.log"), says("repository", "reads"));
        assert_eq!(access("branch.create"), says("repository", "writes"));
        assert_eq!(access("link.make"), says("repository_if_open", "reads"));
        assert_eq!(access("sign.configure"), says("repository", "writes_scope"));
        assert_eq!(
            access("profile.use"),
            says("repository_if_open", "writes_scope")
        );
        // One made up for a test is not in the table, and is taken for
        // one that needs a repository and changes it.
        let mut reg = Registry::new();
        reg.register(
            CommandSpec {
                id: "made.up",
                label: "Made up",
                description: "Nothing is known about it",
            },
            noop,
        )
        .unwrap();
        assert_eq!(entries(&reg)[0].access, Access::WRITES);
    }

    #[test]
    fn the_built_in_catalog_describes_parameters() {
        let registry = bitbraid_integrations::registry();
        let rows = entries(&registry);
        let pull = rows.iter().find(|row| row.id == "remote.pull").unwrap();
        let params = pull.params.as_ref().unwrap();
        assert_eq!(params[0].name, "mode");
        assert_eq!(params[0].kind, "choice");
        assert_eq!(params[0].choices, ["ff_only", "ff", "rebase", "merge"]);
        // A command that takes nothing says so with an empty list.
        let summary = rows.iter().find(|row| row.id == "repo.summary").unwrap();
        assert_eq!(summary.params.as_deref(), Some(&[][..]));
        // One made up for a test is not in the table.
        let mut reg = Registry::new();
        reg.register(
            CommandSpec {
                id: "made.up",
                label: "Made up",
                description: "Nothing is known about it",
            },
            noop,
        )
        .unwrap();
        assert!(entries(&reg)[0].params.is_none());
        assert!(to_json(&reg).unwrap().contains("\"params\": null"));
    }

    #[test]
    fn the_provider_commands_are_in_the_catalog_with_their_own_description() {
        let registry = bitbraid_integrations::registry();
        let rows = entries(&registry);
        let row = |id: &str| rows.iter().find(|row| row.id == id).unwrap();
        // They bring what they need and take: no repository for an
        // account, the open one's link for a pull request when there is
        // one.
        assert_eq!(row("account.add_token").access, Access::FREE);
        assert_eq!(row("pr.list").access, Access::READS_IF_OPEN);
        assert_eq!(row("pr.checkout").access, Access::WRITES);
        let takes = row("account.add_token").params.as_ref().unwrap();
        let names: Vec<&str> = takes.iter().map(|param| param.name).collect();
        assert_eq!(names, ["kind", "web", "token"]);
        assert_eq!(takes[0].choices, ["github", "gitlab"]);
        for id in [
            "account.list",
            "provider.links",
            "pr.list",
            "issue.list",
            "ci.status",
        ] {
            assert!(row(id).params.is_some(), "{id}");
        }
    }

    #[test]
    fn empty_registry_exports_empty_array() {
        assert_eq!(to_json(&Registry::new()).unwrap(), "[]");
    }
}
