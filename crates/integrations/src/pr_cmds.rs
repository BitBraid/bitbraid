//! Pull request commands: list them, show one, the choices for making or
//! changing one, its templates, make, change, check out, merge, review and
//! comment on one, its stack, and the one of a branch. A merge request of
//! GitLab is a pull request here.
//!
//! The commands are the reads (`pr.list` with every filter and a query,
//! `pr.show`, `pr.options`, `pr.templates`, `pr.stack`, `pr.for_branch`)
//! and the writes (`pr.create`, `pr.update`, `pr.merge` with the stack
//! moved down when asked, `pr.review`, `pr.comment`), `pr.checkout` and
//! `pr.head` ([`crate::pr_checkout`]).
//!
//! Sources: docs.github.com/en/rest "Pull requests"; docs.gitlab.com/api
//! "Merge requests"; the documents named in [`crate::github`],
//! [`crate::gitlab`] and [`crate::templates`].

use bitbraid_engine::{
    CommandContext, CommandError, CommandSpec, Output, ParamKind, ParamSpec, Params, Registry,
};

use crate::{
    cmds::{
        self, ACCOUNT, FRESH, MAX_SENT_BODY, NAME, NUMBER, OWNER, READS, READS_IF_OPEN, WRITES,
        failed, invalid, opt, req,
    },
    model::MergeMethod,
    provider::{
        LineComment, MergeRequest, NewPull, PullChange, PullFilter, PullQuery, ReviewEvent,
        StateFilter,
    },
};

/// The highest line a comment is put on.
const MOST_LINE: u64 = 10_000_000;

/// Most pull requests a list holds.
const MOST: usize = 1000;

const LIST: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    opt(
        "filter",
        ParamKind::Choice(&["all", "mine", "assigned", "review_requested"]),
        message!("pr-param-list-filter"),
    ),
    opt("query", ParamKind::Text, message!("pr-param-list-query")),
    opt(
        "state",
        ParamKind::Choice(&["open", "closed", "merged", "all"]),
        message!("pr-param-list-state"),
    ),
    FRESH,
];
const ONE: &[ParamSpec] = &[ACCOUNT, OWNER, NAME, NUMBER, FRESH];
const OPTIONS: &[ParamSpec] = &[ACCOUNT, OWNER, NAME, FRESH];
const CREATE: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    req("head", ParamKind::Text, message!("pr-param-create-head")),
    req("base", ParamKind::Text, message!("pr-param-base")),
    req("title", ParamKind::Text, message!("pr-param-create-title")),
    opt("body", ParamKind::Text, message!("pr-param-create-body")),
    opt("draft", ParamKind::Flag, message!("pr-param-create-draft")),
    opt(
        "reviewers",
        ParamKind::List,
        message!("pr-param-create-reviewers"),
    ),
    opt(
        "assignees",
        ParamKind::List,
        message!("pr-param-create-assignees"),
    ),
    opt(
        "labels",
        ParamKind::List,
        message!("pr-param-create-labels"),
    ),
    opt(
        "milestone",
        ParamKind::Text,
        message!("pr-param-create-milestone"),
    ),
];
const UPDATE: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    NUMBER,
    opt("title", ParamKind::Text, message!("pr-param-update-title")),
    opt("body", ParamKind::Text, message!("pr-param-update-body")),
    opt(
        "reviewers",
        ParamKind::List,
        message!("pr-param-update-reviewers"),
    ),
    opt(
        "assignees",
        ParamKind::List,
        message!("pr-param-update-assignees"),
    ),
    opt(
        "labels",
        ParamKind::List,
        message!("pr-param-update-labels"),
    ),
    opt(
        "milestone",
        ParamKind::Text,
        message!("pr-param-update-milestone"),
    ),
    opt("draft", ParamKind::Flag, message!("pr-param-update-draft")),
    opt("base", ParamKind::Text, message!("pr-param-base")),
];
const MERGE: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    NUMBER,
    req(
        "method",
        ParamKind::Choice(&["merge", "squash", "rebase"]),
        message!("pr-param-merge-method"),
    ),
    opt(
        "delete_branch",
        ParamKind::Flag,
        message!("pr-param-merge-delete-branch"),
    ),
    opt("title", ParamKind::Text, message!("pr-param-merge-title")),
    opt(
        "message",
        ParamKind::Text,
        message!("pr-param-merge-message"),
    ),
    opt(
        "retarget_stack",
        ParamKind::Flag,
        message!("pr-param-merge-retarget-stack"),
    ),
];
const REVIEW: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    NUMBER,
    req(
        "event",
        ParamKind::Choice(&["approve", "request_changes", "comment"]),
        message!("pr-param-review-event"),
    ),
    opt("body", ParamKind::Text, message!("pr-param-review-body")),
];
const COMMENT: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    NUMBER,
    req("body", ParamKind::Text, message!("pr-param-comment-body")),
    opt(
        "reply_to",
        ParamKind::Text,
        message!("pr-param-comment-reply-to"),
    ),
    opt("path", ParamKind::Text, message!("pr-param-comment-path")),
    opt("line", ParamKind::Number, message!("pr-param-comment-line")),
    opt(
        "start_line",
        ParamKind::Number,
        message!("pr-param-comment-start-line"),
    ),
];
const FOR_BRANCH: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    req("branch", ParamKind::Text, message!("pr-param-for-branch")),
    FRESH,
];
const CHECKOUT: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    NUMBER,
    opt("remote", ParamKind::Text, message!("pr-param-remote")),
    opt(
        "dirty",
        ParamKind::Choice(&["refuse", "stash", "discard"]),
        message!("pr-param-checkout-dirty"),
    ),
];

const HEAD: &[ParamSpec] = &[
    ACCOUNT,
    OWNER,
    NAME,
    NUMBER,
    opt("remote", ParamKind::Text, message!("pr-param-remote")),
];

/// Register the pull request commands.
pub fn register(registry: &mut Registry) {
    let entries: [(
        CommandSpec,
        bitbraid_engine::Handler,
        bitbraid_engine::Access,
        &'static [ParamSpec],
    ); 13] = [
        (
            CommandSpec {
                id: "pr.list",
                label: say!("pr-list"),
                description: say!("pr-list-description"),
            },
            list,
            READS_IF_OPEN,
            LIST,
        ),
        (
            CommandSpec {
                id: "pr.show",
                label: say!("pr-show"),
                description: say!("pr-show-description"),
            },
            show,
            READS_IF_OPEN,
            ONE,
        ),
        (
            CommandSpec {
                id: "pr.options",
                label: say!("pr-options"),
                description: say!("pr-options-description"),
            },
            options,
            READS_IF_OPEN,
            OPTIONS,
        ),
        (
            CommandSpec {
                id: "pr.templates",
                label: say!("pr-templates"),
                description: say!("pr-templates-description"),
            },
            templates,
            READS,
            &[],
        ),
        (
            CommandSpec {
                id: "pr.create",
                label: say!("pr-create"),
                description: say!("pr-create-description"),
            },
            create,
            READS_IF_OPEN,
            CREATE,
        ),
        (
            CommandSpec {
                id: "pr.update",
                label: say!("pr-update"),
                description: say!("pr-update-description"),
            },
            update,
            READS_IF_OPEN,
            UPDATE,
        ),
        (
            CommandSpec {
                id: "pr.checkout",
                label: say!("pr-checkout"),
                description: say!("pr-checkout-description"),
            },
            checkout,
            WRITES,
            CHECKOUT,
        ),
        (
            CommandSpec {
                id: "pr.head",
                label: say!("pr-head"),
                description: say!("pr-head-description"),
            },
            head,
            WRITES,
            HEAD,
        ),
        (
            CommandSpec {
                id: "pr.merge",
                label: say!("pr-merge"),
                description: say!("pr-merge-description"),
            },
            merge,
            READS_IF_OPEN,
            MERGE,
        ),
        (
            CommandSpec {
                id: "pr.review",
                label: say!("pr-review"),
                description: say!("pr-review-description"),
            },
            review,
            READS_IF_OPEN,
            REVIEW,
        ),
        (
            CommandSpec {
                id: "pr.comment",
                label: say!("pr-comment"),
                description: say!("pr-comment-description"),
            },
            comment,
            READS_IF_OPEN,
            COMMENT,
        ),
        (
            CommandSpec {
                id: "pr.stack",
                label: say!("pr-stack"),
                description: say!("pr-stack-description"),
            },
            stack,
            READS_IF_OPEN,
            ONE,
        ),
        (
            CommandSpec {
                id: "pr.for_branch",
                label: say!("pr-for-branch"),
                description: say!("pr-for-branch-description"),
            },
            for_branch,
            READS_IF_OPEN,
            FOR_BRANCH,
        ),
    ];
    for (spec, handler, access, params) in entries {
        let _ = registry.register_described(spec, handler, access, params);
    }
}

fn list(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let filter = match cmds::choice(
        params,
        "filter",
        &["all", "mine", "assigned", "review_requested"],
        "all",
    )? {
        "mine" => PullFilter::Mine,
        "assigned" => PullFilter::Assigned,
        "review_requested" => PullFilter::ReviewRequested,
        _ => PullFilter::All,
    };
    let state = match cmds::choice(
        params,
        "state",
        &["open", "closed", "merged", "all"],
        "open",
    )? {
        "closed" => StateFilter::Closed,
        "merged" => StateFilter::Merged,
        "all" => StateFilter::All,
        _ => StateFilter::Open,
    };
    let query = PullQuery {
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
        .pr_list(&signed.session(ctx), &target, &query)
        .map_err(failed)?;
    cmds::answer(&listing)
}

fn show(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    cmds::flag(params, "fresh")?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let details = signed
        .provider()
        .pr_show(&signed.session(ctx), &target, number)
        .map_err(failed)?;
    cmds::answer(&details)
}

fn options(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    cmds::flag(params, "fresh")?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let options = signed
        .provider()
        .pr_options(&signed.session(ctx), &target)
        .map_err(failed)?;
    cmds::answer(&options)
}

fn templates(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let found = match ctx.repo()?.workdir() {
        Some(root) => crate::templates::read(root),
        // A bare repository has no working tree to hold them.
        None => Vec::new(),
    };
    cmds::answer(&found)
}

/// A branch's name as a parameter: what git allows, roughly; the provider
/// says the rest.
fn branch(params: &Params, name: &str) -> Result<Option<String>, CommandError> {
    let Some(text) = cmds::text_with(params, name, 255, false)? else {
        return Ok(None);
    };
    if text.trim().is_empty()
        || text.starts_with('-')
        || text.contains("..")
        || text.contains(['~', '^', '?', '*', '[', '\\', ' '])
    {
        return Err(invalid(name, say!("pr-not-a-branch")));
    }
    Ok(Some(text))
}

fn create(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let pull = NewPull {
        head: branch(params, "head")?.ok_or_else(|| invalid("head", say!("pr-missing")))?,
        base: branch(params, "base")?.ok_or_else(|| invalid("base", say!("pr-missing")))?,
        title: cmds::required(params, "title")?,
        body: cmds::text_with(params, "body", MAX_SENT_BODY, true)?.unwrap_or_default(),
        draft: cmds::flag(params, "draft")?.unwrap_or(false),
        reviewers: cmds::list(params, "reviewers", 100)?.unwrap_or_default(),
        assignees: cmds::list(params, "assignees", 100)?.unwrap_or_default(),
        labels: cmds::list(params, "labels", 100)?.unwrap_or_default(),
        milestone: cmds::text_with(params, "milestone", 64, false)?,
    };
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let made = signed
        .provider()
        .pr_create(&signed.session(ctx), &target, &pull)
        .map_err(failed)?;
    cmds::answer(&made)
}

fn update(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    let change = PullChange {
        title: cmds::text(params, "title")?,
        body: cmds::text_with(params, "body", MAX_SENT_BODY, true)?,
        reviewers: cmds::list(params, "reviewers", 100)?,
        assignees: cmds::list(params, "assignees", 100)?,
        labels: cmds::list(params, "labels", 100)?,
        milestone: cmds::text_with(params, "milestone", 64, false)?
            .map(|milestone| Some(milestone).filter(|id| !id.is_empty())),
        draft: cmds::flag(params, "draft")?,
        base: branch(params, "base")?,
    };
    if change == PullChange::default() {
        return Err(invalid("title", say!("pr-nothing-to-change")));
    }
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let details = signed
        .provider()
        .pr_update(&signed.session(ctx), &target, number, &change)
        .map_err(failed)?;
    cmds::answer(&details)
}

fn checkout(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    crate::pr_checkout::checkout(ctx, params)
}

fn head(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    crate::pr_checkout::head(ctx, params)
}

fn merge(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    let method = match cmds::choice(params, "method", &["merge", "squash", "rebase"], "")? {
        "merge" => MergeMethod::Merge,
        "squash" => MergeMethod::Squash,
        "rebase" => MergeMethod::Rebase,
        _ => return Err(invalid("method", say!("pr-missing"))),
    };
    let request = MergeRequest {
        method,
        delete_branch: cmds::flag(params, "delete_branch")?.unwrap_or(false),
        title: cmds::text_with(params, "title", 1024, false)?,
        message: cmds::text_with(params, "message", MAX_SENT_BODY, true)?,
        retarget_stack: cmds::flag(params, "retarget_stack")?.unwrap_or(false),
    };
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let merged = signed
        .provider()
        .pr_merge(&signed.session(ctx), &target, number, &request)
        .map_err(failed)?;
    cmds::answer(&merged)
}

fn review(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    let event = match cmds::choice(
        params,
        "event",
        &["approve", "request_changes", "comment"],
        "",
    )? {
        "approve" => ReviewEvent::Approve,
        "request_changes" => ReviewEvent::RequestChanges,
        "comment" => ReviewEvent::Comment,
        _ => return Err(invalid("event", say!("pr-missing"))),
    };
    let body = cmds::text_with(params, "body", MAX_SENT_BODY, true)?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let given = signed
        .provider()
        .pr_review(
            &signed.session(ctx),
            &target,
            number,
            event,
            body.as_deref(),
        )
        .map_err(failed)?;
    cmds::answer(&given)
}

fn comment(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    let body = cmds::text_with(params, "body", MAX_SENT_BODY, true)?
        .filter(|body| !body.trim().is_empty())
        .ok_or_else(|| invalid("body", say!("pr-missing")))?;
    let reply_to = cmds::text_with(params, "reply_to", 128, false)?;
    if reply_to.as_deref().is_some_and(|id| {
        !id.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'='))
    }) {
        return Err(invalid("reply_to", say!("pr-not-a-comment")));
    }
    // On lines of a file.
    let path = cmds::text_with(params, "path", 4096, false)?;
    let line = cmds::number(params, "line", 1, MOST_LINE)?;
    let start_line = cmds::number(params, "start_line", 1, MOST_LINE)?;
    if let Some(path) = path {
        if reply_to.is_some() {
            return Err(invalid("path", say!("pr-path-with-reply")));
        }
        if path.trim().is_empty() || path.starts_with('/') {
            return Err(invalid("path", say!("pr-not-a-file")));
        }
        let line = line.ok_or_else(|| invalid("line", say!("pr-missing")))?;
        if start_line.is_some_and(|start| start > line) {
            return Err(invalid("start_line", say!("pr-start-after-line")));
        }
        let (signed, target) = cmds::on_repository(ctx, params)?;
        let made = signed
            .provider()
            .pr_line_comment(
                &signed.session(ctx),
                &target,
                number,
                &LineComment {
                    path,
                    line,
                    start_line,
                    body,
                },
            )
            .map_err(failed)?;
        return cmds::answer(&made);
    }
    if line.is_some() || start_line.is_some() {
        return Err(invalid("line", say!("pr-line-without-path")));
    }
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let made = signed
        .provider()
        .pr_comment(
            &signed.session(ctx),
            &target,
            number,
            &body,
            reply_to.as_deref(),
        )
        .map_err(failed)?;
    cmds::answer(&made)
}

fn stack(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    cmds::flag(params, "fresh")?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let found = signed
        .provider()
        .pr_stack(&signed.session(ctx), &target, number)
        .map_err(failed)?;
    cmds::answer(&found)
}

fn for_branch(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name = branch(params, "branch")?.ok_or_else(|| invalid("branch", say!("pr-missing")))?;
    cmds::flag(params, "fresh")?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let found = signed
        .provider()
        .pr_for_branch(&signed.session(ctx), &target, &name)
        .map_err(failed)?;
    cmds::answer(&found)
}
