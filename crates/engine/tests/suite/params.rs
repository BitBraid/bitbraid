//! The table of parameters against the registry and the handlers: every
//! command is described, nothing is described that does not exist, and
//! what the table marks as required is what a handler refuses to do
//! without.

use std::collections::{BTreeMap, BTreeSet};

use bitbraid_engine::{CommandContext, ParamKind, builtin_registry, params};
use bitbraid_testkit::Scratch;
use serde_json::json;

#[test]
fn every_command_is_described_and_nothing_else_is() {
    let registry = builtin_registry();
    let registered: BTreeSet<&str> = registry.iter().map(|spec| spec.id).collect();
    let described: Vec<&str> = params::described().collect();
    let unique: BTreeSet<&str> = described.iter().copied().collect();
    assert_eq!(unique.len(), described.len(), "an id has two rows");
    let missing: Vec<&&str> = registered.difference(&unique).collect();
    assert!(
        missing.is_empty(),
        "commands without parameters described: {missing:?}"
    );
    let stale: Vec<&&str> = unique.difference(&registered).collect();
    assert!(
        stale.is_empty(),
        "rows for commands that do not exist: {stale:?}"
    );

    for spec in registry.iter() {
        let schema = registry.params(spec.id).unwrap();
        let mut names = BTreeSet::new();
        for param in schema {
            assert!(
                names.insert(param.name),
                "{}: {} twice",
                spec.id,
                param.name
            );
            assert!(
                !param.name.is_empty()
                    && param
                        .name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{}: {:?}",
                spec.id,
                param.name
            );
            assert!(
                param.description.text().len() >= 3 && param.description.text().len() <= 120,
                "{}.{}: a few words, not {:?}",
                spec.id,
                param.name,
                param.description.text()
            );
            if let ParamKind::Choice(choices) = param.kind {
                assert!(choices.len() >= 2, "{}.{}", spec.id, param.name);
            }
        }
    }
}

#[test]
fn what_is_marked_required_is_what_a_handler_refuses_to_do_without() {
    let registry = builtin_registry();
    let copy = Scratch::of("linear").unwrap();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
        .unwrap();
    let head_before = copy.git().rev_parse("HEAD").unwrap();
    let mut checked = 0;
    for spec in registry.iter() {
        let schema = registry.params(spec.id).unwrap();
        let required: Vec<&str> = schema
            .iter()
            .filter(|param| param.required)
            .map(|param| param.name)
            .collect();
        if required.is_empty() {
            continue;
        }
        // With nothing given, the command says which parameter it misses.
        let err = registry
            .run(spec.id, &mut ctx, &json!({}))
            .expect_err(&format!("{} ran without {required:?}", spec.id))
            .to_string();
        assert!(
            required.iter().any(|name| err.contains(name)),
            "{}: {err:?} names none of {required:?}",
            spec.id
        );
        // And the prompt's own check agrees.
        let refused = params::from_text(schema, &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(refused.contains("missing"), "{}: {refused}", spec.id);
        checked += 1;
    }
    assert!(
        checked > 60,
        "only {checked} commands have required parameters"
    );
    // Nothing happened to the repository meanwhile.
    assert_eq!(copy.git().rev_parse("HEAD").unwrap(), head_before);
    assert_eq!(copy.git().run(["status", "--porcelain"]).unwrap(), "");
}

#[test]
fn a_prompt_can_run_a_command() {
    // What is typed into a prompt, one text per parameter, runs the command.
    let registry = builtin_registry();
    let copy = Scratch::of("linear").unwrap();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
        .unwrap();
    let typed: BTreeMap<String, String> = [
        ("name", "from-the-prompt"),
        ("start", "HEAD~1"),
        ("checkout", "no"),
    ]
    .into_iter()
    .map(|(name, text)| (name.to_owned(), text.to_owned()))
    .collect();
    let schema = registry.params("branch.create").unwrap();
    let built = params::from_text(schema, &typed).unwrap();
    registry.run("branch.create", &mut ctx, &built).unwrap();
    let git = copy.git();
    assert_eq!(
        git.rev_parse("from-the-prompt").unwrap(),
        git.rev_parse("HEAD~1").unwrap()
    );
    // A list and a number.
    let typed: BTreeMap<String, String> = [("tips", "head"), ("limit", "2")]
        .into_iter()
        .map(|(name, text)| (name.to_owned(), text.to_owned()))
        .collect();
    let built = params::from_text(registry.params("repo.log").unwrap(), &typed).unwrap();
    let log = registry.run("repo.log", &mut ctx, &built).unwrap();
    assert_eq!(log.as_array().unwrap().len(), 2);
}
