//! Issue commands and the checks of commits: list a repository's
//! issues, show one, make a branch for one, read an issue's key from a
//! branch's name, and the checks of commits.
//!
//! A branch for an issue is named by a template (`{number}-{title}`
//! unless another is given), its title made a slug, as
//! [`bitbraid_git::issue_keys::branch_for_issue`] says, or by the name
//! the user settled on; it is made, and checked out when asked, by the
//! engine's `branch.create`, so undo sees it like any other branch. The
//! title is asked of the provider only when no name is given. The key in
//! a branch's name is read by [`bitbraid_git::issue_keys::branch_number`]
//! (`123-fix`, `fix-123`, `feature/123-x`, `issue-123`, `gh-123`, `#123`),
//! without a network.
//!
//! Sources: docs.github.com/en/rest "Issues", "Commit statuses", "Check
//! runs"; docs.gitlab.com/api "Issues", "Pipelines", "Commits" (statuses);
//! the documents named in [`crate::github`] and [`crate::gitlab`];
//! git-check-ref-format(1).

use bitbraid_engine::{
    CommandContext, CommandError, CommandSpec, Output, ParamKind, ParamSpec, Params, Registry,
};

use bitbraid_git::issue_keys;
use serde_json::json;

use crate::{
    cmds::{
        self, ACCOUNT, FREE, FRESH, NAME, OWNER, READS_IF_OPEN, WRITES, failed, invalid, opt, req,
    },
    provider::{IssueFilter, IssueQuery, StateFilter},
};

/// Most issues a list holds.
const MOST: usize = 1000;
/// The longest branch name taken, in bytes.
const MAX_BRANCH_BYTES: usize = 1024;
/// Most commits asked about at once.
const MAX_SHAS: usize = 100;

const ISSUE_NUMBER: ParamSpec = req(
    "number",
    ParamKind::Number,
    message!("issue-param-issue-number"),
);

const LIST: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    opt(
        "filter",
        ParamKind::Choice(&["all", "assigned", "created", "mentioned"]),
        message!("issue-param-list-filter"),
    ),
    opt("query", ParamKind::Text, message!("issue-param-list-query")),
    opt(
        "state",
        ParamKind::Choice(&["open", "closed", "all"]),
        message!("issue-param-list-state"),
    ),
    FRESH,
];
const SHOW: &[ParamSpec] = &[ACCOUNT, OWNER, NAME, ISSUE_NUMBER, FRESH];
const BRANCH: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    ISSUE_NUMBER,
    opt(
        "template",
        ParamKind::Text,
        message!("issue-param-branch-template"),
    ),
    opt("branch", ParamKind::Text, message!("issue-param-branch")),
    opt(
        "start",
        ParamKind::Revision,
        message!("issue-param-branch-start"),
    ),
    opt(
        "checkout",
        ParamKind::Flag,
        message!("issue-param-branch-checkout"),
    ),
];
const KEY: &[ParamSpec] = &[req(
    "branch",
    ParamKind::Text,
    message!("issue-param-key-branch"),
)];
const CI: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    req("shas", ParamKind::List, message!("issue-param-ci-shas")),
    FRESH,
];

/// Register the issue commands and the checks of commits.
pub fn register(registry: &mut Registry) {
    let _ = registry.register_described(
        CommandSpec {
            id: "issue.list",
            label: say!("issue-list"),
            description: say!("issue-list-description"),
        },
        list,
        READS_IF_OPEN,
        LIST,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "issue.show",
            label: say!("issue-show"),
            description: say!("issue-show-description"),
        },
        show,
        READS_IF_OPEN,
        SHOW,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "issue.branch",
            label: say!("issue-branch"),
            description: say!("issue-branch-description"),
        },
        branch,
        WRITES,
        BRANCH,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "issue.key_from_branch",
            label: say!("issue-key-from-branch"),
            description: say!("issue-key-from-branch-description"),
        },
        key_from_branch,
        FREE,
        KEY,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "ci.status",
            label: say!("issue-ci-status"),
            description: say!("issue-ci-status-description"),
        },
        ci_status,
        READS_IF_OPEN,
        CI,
    );
}

fn list(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let filter = match cmds::choice(
        params,
        "filter",
        &["all", "assigned", "created", "mentioned"],
        "all",
    )? {
        "assigned" => IssueFilter::Assigned,
        "created" => IssueFilter::Created,
        "mentioned" => IssueFilter::Mentioned,
        _ => IssueFilter::All,
    };
    let state = match cmds::choice(params, "state", &["open", "closed", "all"], "open")? {
        "closed" => StateFilter::Closed,
        "all" => StateFilter::All,
        _ => StateFilter::Open,
    };
    let query = IssueQuery {
        filter,
        state,
        query: cmds::text_with(params, "query", 1024, false)?
            .filter(|query| !query.trim().is_empty()),
        most: MOST,
    };
    cmds::flag(params, "fresh")?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let listing = signed
        .provider()
        .issue_list(&signed.session(ctx), &target, &query)
        .map_err(failed)?;
    cmds::answer(&listing)
}

fn show(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    cmds::flag(params, "fresh")?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let details = signed
        .provider()
        .issue_show(&signed.session(ctx), &target, number)
        .map_err(failed)?;
    cmds::answer(&details)
}

fn branch(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    let template = cmds::text_with(params, "template", issue_keys::MAX_TEMPLATE_BYTES, false)?;
    let settled = cmds::text_with(params, "branch", MAX_BRANCH_BYTES, false)?
        .filter(|name| !name.trim().is_empty());
    let start = cmds::text(params, "start")?;
    let checkout = cmds::flag(params, "checkout")?.unwrap_or(false);
    ctx.repo()?;
    let name = match settled {
        Some(name) => {
            if let Some(problem) = bitbraid_git::branch::name_problem(&name) {
                return Err(invalid("branch", problem));
            }
            name
        }
        None => {
            let (signed, target) = cmds::on_repository(ctx, params)?;
            let details = signed
                .provider()
                .issue_show(&signed.session(ctx), &target, number)
                .map_err(failed)?;
            issue_keys::branch_for_issue(template.as_deref(), number, &details.issue.title)
                .map_err(|problem| invalid("template", problem))?
        }
    };
    let mut made = crate::shared().run(
        "branch.create",
        ctx,
        &json!({ "name": name, "start": start, "checkout": checkout }),
    )?;
    if let Some(fields) = made.as_object_mut() {
        fields.insert("issue".to_owned(), json!(number));
    }
    Ok(made)
}

fn key_from_branch(_: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let branch = cmds::text_with(params, "branch", MAX_BRANCH_BYTES, false)?
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| invalid("branch", say!("issue-missing")))?;
    let number = issue_keys::branch_number(&branch);
    Ok(json!({
        "number": number,
        "key": number.map(|number| format!("#{number}")),
    }))
}

fn ci_status(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let shas = cmds::list(params, "shas", MAX_SHAS)?
        .ok_or_else(|| invalid("shas", say!("issue-missing")))?;
    if let Some(odd) = shas
        .iter()
        .find(|sha| !(4..=64).contains(&sha.len()) || !sha.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(invalid(
            "shas",
            say!("issue-not-a-commit", value = format!("{odd:?}")),
        ));
    }
    cmds::flag(params, "fresh")?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let statuses = signed
        .provider()
        .ci_status(&signed.session(ctx), &target, &shas)
        .map_err(failed)?;
    cmds::answer(&statuses)
}
