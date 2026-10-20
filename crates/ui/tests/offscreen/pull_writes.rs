//! Pull requests that change things: making one from the panel, and what
//! the view of one does to it. The provider commands are answered by
//! stand-ins given here (`views::pulls::calls`), so that nothing depends on
//! a network or an account, and what the views asked of them is looked at.

use std::time::Duration;

use bitbraid_integrations::model::{
    Comment, Label, MergeMethod, Mergeable, Merged, Milestone, PullDetails, PullOptions,
    PullRequest, PullState, PullTemplate, Review, ReviewState, Stack, StackEntry, User,
};
use bitbraid_ui::views::{
    pulls::{
        PullView, calls,
        create::{Chips, NewPullProbe, NewPullView},
        view::{Listed, PullViewProbe, Verdict},
    },
    rewrite::Rewrite,
};
use gpui_kit::{
    App, ElementId, Entity, InputEvent as _, MouseButton, MouseDownEvent, MouseUpEvent, Pixels,
    Point, Window, point, px, test::TestWindowExt as _,
};
use serde_json::{Value, json};

use crate::{
    pulls, repos,
    runner::{Shots, Test, Themes},
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "a_pull_request_is_made_of_a_branch_that_is_pushed_first",
        run: a_pull_request_is_made_of_a_branch_that_is_pushed_first,
    },
    Test {
        name: "a_branch_dropped_on_another_offers_to_start_a_pull_request",
        run: a_branch_dropped_on_another_offers_to_start_a_pull_request,
    },
    Test {
        name: "the_new_pull_request_panel_stacks_and_says_what_went_wrong",
        run: the_new_pull_request_panel_stacks_and_says_what_went_wrong,
    },
    Test {
        name: "a_pull_requests_title_description_and_lists_are_changed_in_place",
        run: a_pull_requests_title_description_and_lists_are_changed_in_place,
    },
    Test {
        name: "a_pull_request_in_a_stack_is_merged_after_asking_and_its_stack_walked",
        run: a_pull_request_in_a_stack_is_merged_after_asking_and_its_stack_walked,
    },
    Test {
        name: "comments_are_answered_and_quoted_and_reviews_given_from_the_field",
        run: comments_are_answered_and_quoted_and_reviews_given_from_the_field,
    },
    Test {
        name: "two_clicks_on_a_pull_requests_branches_check_them_out",
        run: two_clicks_on_a_pull_requests_branches_check_them_out,
    },
    Test {
        name: "the_entry_of_another_branchs_menu_makes_one_of_the_checked_out_into_it",
        run: the_entry_of_another_branchs_menu_makes_one_of_the_checked_out_into_it,
    },
    Test {
        name: "a_pull_requests_row_checks_it_out_on_a_double_click_and_has_a_menu",
        run: a_pull_requests_row_checks_it_out_on_a_double_click_and_has_a_menu,
    },
    Test {
        name: "hostile_answers_to_the_writes_are_shown_without_harm",
        run: hostile_answers_to_the_writes_are_shown_without_harm,
    },
    Test {
        name: "a_pencils_open_list_fills_in_when_what_can_be_chosen_is_read",
        run: a_pencils_open_list_fills_in_when_what_can_be_chosen_is_read,
    },
];

/// The entries of the menu that is open, by what they say.
fn menu_entries(stage: &mut Stage) -> Vec<String> {
    stage.window(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|element| element.visible() && element.role() == Some(gpui_kit::Role::MenuItem))
            .filter_map(|element| element.label().map(str::to_owned))
            .collect()
    })
}

/// A pencil's list opened while what can be chosen is still being read
/// says so, and lists the choices as soon as they come, without being
/// closed and opened again (a list once open is not built again). Opened
/// again it has them at once.
fn a_pencils_open_list_fills_in_when_what_can_be_chosen_is_read() {
    let (mut stage, _standing) = open_41(pulls::details(), false);
    calls::hold("pr.options");
    assert!(scroll_to(&mut stage, "pull-edit-labels"));
    stage.click("pull-edit-labels");
    stage.frame();
    let before = menu_entries(&mut stage);
    assert!(
        before
            .iter()
            .any(|entry| entry.starts_with("Reading what can be chosen")),
        "{before:?}"
    );
    calls::release();
    stage.settle();
    stage.frame();
    let after = menu_entries(&mut stage);
    for label in ["bug", "docs", "lexer"] {
        assert!(
            after.iter().any(|entry| entry == label),
            "the open list has {label}: {after:?}"
        );
    }
    assert!(
        !after
            .iter()
            .any(|entry| entry.starts_with("Reading what can be chosen")),
        "{after:?}"
    );
    // Opened again, it has them at once.
    stage.keys("escape");
    assert!(scroll_to(&mut stage, "pull-edit-labels"));
    stage.click("pull-edit-labels");
    stage.frame();
    let again = menu_entries(&mut stage);
    assert!(again.iter().any(|entry| entry == "lexer"), "{again:?}");
    stage.keys("escape");
}

/// The stand-ins answer while this is held, and stop when it goes, also
/// when a test fails half-way.
pub struct Standing;

impl Drop for Standing {
    fn drop(&mut self) {
        calls::stand_down();
    }
}

/// Press one of the panel's buttons, which stand under its fields: the
/// fields are scrolled to their end first, as the user would in a window
/// this low.
fn panel_button(stage: &mut Stage, id: &'static str) {
    stage.window(|window, cx| {
        window.scroll(
            "new-pull-rows",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        )
    });
    stage.frame();
    stage.click(id);
}

/// Have the provider commands answered by `answer` while what this
/// gives is held.
pub fn stand_in(
    answer: impl Fn(&str, &Value) -> Option<Result<Value, String>> + 'static,
) -> Standing {
    calls::stand_in(answer);
    Standing
}

fn person(login: &str) -> User {
    pulls::user(login)
}

fn label(name: &str, colour: &str) -> Label {
    pulls::label(name, colour)
}

/// What the repository at the provider allows to be chosen.
pub fn options() -> PullOptions {
    PullOptions {
        labels: vec![
            label("bug", "d73a4a"),
            label("docs", "0075ca"),
            label("lexer", "5319e7"),
        ],
        milestones: vec![
            Milestone {
                id: "3".to_owned(),
                title: "0.3".to_owned(),
                due: None,
                open: true,
            },
            Milestone {
                id: "4".to_owned(),
                title: "0.4".to_owned(),
                due: None,
                open: true,
            },
        ],
        people: ["ada", "bob", "cy", "octo"]
            .into_iter()
            .map(person)
            .collect(),
        merge_methods: vec![MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase],
    }
}

/// The repository's two templates.
fn templates() -> Vec<PullTemplate> {
    vec![
        PullTemplate {
            path: ".github/PULL_REQUEST_TEMPLATE/fix.md".to_owned(),
            name: "fix".to_owned(),
            body: "## What was wrong\n\n## How it is fixed\n".to_owned(),
        },
        PullTemplate {
            path: ".github/PULL_REQUEST_TEMPLATE/feature.md".to_owned(),
            name: "feature".to_owned(),
            body: "## What it adds\n\n## How to try it\n".to_owned(),
        },
    ]
}

/// The open pull request of `feature/lexer`, into main.
fn lexer_pull() -> PullRequest {
    let mut pull = pulls::pull(39, "feature/lexer", "A lexer for quotes", "ada");
    pull.base_ref = "main".to_owned();
    pull
}

/// What the provider answers: what can be chosen, the templates, the
/// open pull request of `feature/lexer`, a push, and `made` for a new
/// pull request (or what goes wrong).
fn answers(
    made: Result<PullRequest, String>,
) -> impl Fn(&str, &Value) -> Option<Result<Value, String>> {
    move |id, params| match id {
        "pr.options" => Some(Ok(json!(options()))),
        "pr.templates" => Some(Ok(json!(templates()))),
        "pr.for_branch" => Some(Ok(match params["branch"].as_str() {
            Some("feature/lexer") => json!(lexer_pull()),
            _ => Value::Null,
        })),
        "remote.push" => Some(Ok(json!({}))),
        "pr.create" => Some(made.clone().map(|pull| json!(pull))),
        "pr.show" => {
            let mut details = pulls::details();
            if let Ok(pull) = &made {
                details.pull = pull.clone();
            }
            Some(Ok(json!(details)))
        }
        _ => None,
    }
}

fn new_pull(stage: &mut Stage) -> Option<Entity<NewPullView>> {
    stage.with_workspace(|workspace, _, cx| workspace.read(cx).new_pull())
}

fn new_probe(stage: &mut Stage) -> NewPullProbe {
    stage.with_workspace(|workspace, _, cx| {
        workspace
            .read(cx)
            .new_pull()
            .expect("the panel is open")
            .read(cx)
            .probe()
    })
}

/// Do something with the panel, in the window.
fn with_panel(
    stage: &mut Stage,
    f: impl FnOnce(&mut NewPullView, &mut Window, &mut gpui_kit::Context<NewPullView>),
) {
    stage.with_workspace(|workspace, window, cx: &mut App| {
        let panel = workspace.read(cx).new_pull().expect("the panel is open");
        panel.update(cx, |view, cx| f(view, window, cx));
    });
    stage.settle();
}

/// Open the panel for `head` into `base`, as a branch's menu or a drop
/// does.
fn start(stage: &mut Stage, head: &str, base: Option<&str>) {
    // What the entry of a branch's menu asks of the tab.
    let rewrite = Rewrite::StartPull {
        head: head.to_owned(),
        base: base.map(str::to_owned),
    };
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| workspace.rewrite(rewrite, window, cx))
    });
    stage.settle();
    assert_eq!(
        stage.workspace().sheet,
        Some("new_pull"),
        "{:?}",
        stage.notices()
    );
    stage.until("what can be chosen is read", |stage| {
        stage.settle();
        let seen = new_probe(stage);
        seen.templates == 2 && seen.conflicts.is_some()
    });
}

/// The panel filled in: a template, a reviewer, an assignee, a label, a
/// milestone, draft.
fn fill(stage: &mut Stage) {
    with_panel(stage, |view, window, cx| {
        view.use_template(Some(1), window, cx);
        view.toggle_chip(Chips::Reviewers, "bob", cx);
        view.toggle_chip(Chips::Assignees, "octo", cx);
        view.toggle_chip(Chips::Labels, "lexer", cx);
        view.set_milestone(Some("3".to_owned()), cx);
        view.set_draft(true, cx);
    });
}

/// From a branch's menu: the panel takes the middle, comes from the
/// branch's remote into its default branch, has a title, offers the two
/// templates, and what is picked; the branch follows nothing, so it is
/// pushed before the pull request is made with what was picked, and then
/// the new pull request's view takes the middle.
fn a_pull_request_is_made_of_a_branch_that_is_pushed_first() {
    let mut stage = pulls::open_section(false);
    let mut made = pulls::pull(42, "wip", "Quotes and the needle", "octo");
    made.draft = true;
    let _standing = stand_in(answers(Ok(made)));
    start(&mut stage, "wip", None);
    let seen = new_probe(&mut stage);
    assert_eq!(seen.from, "origin wip");
    assert_eq!(seen.into, "origin main");
    assert!(seen.push_first, "wip follows nothing");
    assert!(!seen.title.is_empty(), "{seen:?}");
    assert_eq!(seen.body, "", "two templates: none is put in unasked");
    assert_eq!(seen.stack, None);
    assert!(seen.ready && !seen.busy);
    fill(&mut stage);
    let seen = new_probe(&mut stage);
    assert_eq!(seen.body, "## What it adds\n\n## How to try it\n");
    assert_eq!(
        (seen.reviewers, seen.assignees, seen.labels),
        (
            vec!["bob".to_owned()],
            vec!["octo".to_owned()],
            vec!["lexer".to_owned()]
        )
    );
    assert_eq!(seen.milestone.as_deref(), Some("0.3"));
    // A chip goes with its cross.
    with_panel(&mut stage, |view, _, cx| {
        view.toggle_chip(Chips::Reviewers, "cy", cx)
    });
    stage.click(ElementId::Name("new-pull-reviewers-drop:1".into()));
    assert_eq!(new_probe(&mut stage).reviewers, ["bob"]);

    panel_button(&mut stage, "new-pull-create");
    stage.until("the new pull request is shown", |stage| {
        stage.settle();
        stage.workspace().sheet == Some("pull")
    });
    let asked = calls::asked();
    let at = |id: &str| asked.iter().position(|(asked, _)| asked == id);
    let (push, create) = (at("remote.push").unwrap(), at("pr.create").unwrap());
    assert!(push < create, "pushed first");
    assert_eq!(
        asked[push].1,
        json!({ "branch": "wip", "remote": "origin", "set_upstream": true })
    );
    let params = &asked[create].1;
    assert_eq!(
        (&params["account"], &params["owner"], &params["name"]),
        (
            &json!("github:github.com:octo"),
            &json!("octo"),
            &json!("tools")
        )
    );
    assert_eq!(
        (&params["head"], &params["base"]),
        (&json!("wip"), &json!("main"))
    );
    assert_eq!(params["reviewers"], json!(["bob"]));
    assert_eq!(params["assignees"], json!(["octo"]));
    assert_eq!(params["labels"], json!(["lexer"]));
    assert_eq!(params["milestone"], json!("3"));
    assert_eq!(params["draft"], json!(true));
    assert_eq!(
        params["body"],
        json!("## What it adds\n\n## How to try it\n")
    );
    assert_eq!(
        pulls::view_probe(&mut stage).map(|seen| seen.number),
        Some(42)
    );
    assert!(new_pull(&mut stage).is_none(), "the panel is put away");
    assert!(
        stage.notices().iter().any(|notice| notice.contains("#42")),
        "{:?}",
        stage.notices()
    );
}

/// Where a name of the references panel is drawn.
fn name_at(stage: &mut Stage, full_name: &str) -> Point<Pixels> {
    let id = ElementId::Name(format!("ref-name:{full_name}").into());
    stage
        .window(|window, _| {
            window
                .try_find(id)
                .filter(|found| found.visible())
                .map(|found| found.bounds().center())
        })
        .expect("the row is on show")
}

/// A local branch dragged onto another, in the left panel as in the
/// graph, offers to start a pull request of the one into the other, once
/// a remote is linked to an account; the panel opens with the two.
fn a_branch_dropped_on_another_offers_to_start_a_pull_request() {
    repos::fresh();
    let repo = repos::project();
    let state = format!(
        "[repos.{repo:?}]\nclosed_sections = [\"remote\", \"pulls\", \"worktrees\", \"stashes\", \"tags\", \"submodules\"]\n"
    );
    let mut stage = Stage::open(Start::with(&repo).state(state));
    pulls::give(&mut stage, pulls::given());
    let _standing = stand_in(answers(Err("not used".to_owned())));
    let wip = name_at(&mut stage, "refs/heads/wip");
    let main = name_at(&mut stage, "refs/heads/main");
    stage.drag_at(
        (f32::from(wip.x), f32::from(wip.y)),
        (f32::from(main.x), f32::from(main.y)),
    );
    stage.settle();
    assert!(stage.graph().drop_menu);
    // Merging comes first; starting a pull request next.
    stage.click(("drop-choice", 1usize));
    stage.settle();
    assert_eq!(stage.workspace().sheet, Some("new_pull"));
    let seen = new_probe(&mut stage);
    assert_eq!(
        (seen.from.as_str(), seen.into.as_str()),
        ("origin wip", "origin main")
    );
    panel_button(&mut stage, "new-pull-cancel");
    assert_eq!(stage.workspace().sheet, None);
    // From the palette: of the checked-out branch, which goes nowhere
    // into itself until another is chosen.
    stage.act(bitbraid_ui::keymap::StartPullRequest);
    stage.settle();
    assert_eq!(stage.workspace().sheet, Some("new_pull"));
    let seen = new_probe(&mut stage);
    assert_eq!(seen.from, "origin main");
    assert!(!seen.ready, "main into main");
    stage.click("new-pull-close");
    assert_eq!(stage.workspace().sheet, None);
}

/// What the provider answers to the writes is another machine's: lists
/// without end, names with control characters, a stack whose place is
/// past its end, a merge that did not happen with a message of no end,
/// answers of the wrong shape. All of it is shown without harm.
fn hostile_answers_to_the_writes_are_shown_without_harm() {
    let mut odd = pulls::details();
    odd.pull.title = "\u{1b}[31m".repeat(500);
    let (mut stage, standing) = open_41(odd, false);
    drop(standing);
    let _standing = stand_in(|id, _| match id {
        "pr.options" => Some(Ok(json!(PullOptions {
            labels: (0..5000)
                .map(|n| label(&format!("{n}\u{0}\n"), "zz"))
                .collect(),
            milestones: Vec::new(),
            people: (0..5000)
                .map(|n| person(&"\u{202e}".repeat(n % 7)))
                .collect(),
            merge_methods: Vec::new(),
        }))),
        "pr.stack" => Some(Ok(json!(Stack {
            entries: Vec::new(),
            position: usize::MAX,
        }))),
        "pr.merge" => Some(Ok(json!(Merged {
            merged: false,
            sha: None,
            message: Some("\u{7}no".repeat(100_000)),
            retargeted: vec![u64::MAX; 1000],
            branch_deleted: false,
        }))),
        "pr.update" | "pr.comment" | "pr.review" => Some(Ok(json!("a text, not a pull request"))),
        "pr.show" => Some(Ok(json!({ "pull": 7 }))),
        _ => None,
    });
    stage.click("pull-edit-reviewers");
    stage.click("pull-edit-reviewers");
    stage.pass(Duration::from_millis(200));
    stage.keys("escape");
    with_view(&mut stage, |view, window, cx| {
        view.toggle_listed(Listed::Labels, "\u{0}", window, cx)
    });
    stage.until("the answer of the wrong shape is said", |stage| {
        stage.settle();
        seen(stage).write_error.is_some()
    });
    with_view(&mut stage, |view, window, cx| view.merge(window, cx));
    stage.until("the merge that did not happen is said", |stage| {
        stage.settle();
        seen(stage)
            .write_error
            .is_some_and(|error| error.starts_with("Merging"))
    });
    with_view(&mut stage, |view, _, cx| view.refresh(cx));
    stage.settle();
    stage.frame();
    with_view(&mut stage, |view, window, cx| {
        view.reply_to_thread(usize::MAX, window, cx);
        view.quote(&"\n".repeat(10_000), Some(usize::MAX), window, cx);
    });
    stage.frame();
    stage.click("pull-close");
    assert_eq!(stage.workspace().sheet, None);
}

/// Into a branch that has an open pull request of its own, the new one
/// goes on top of it, or, unticked, where that one goes. A title is
/// needed. What the provider says when it cannot make it stays in the
/// panel, which stays; Cancel puts it away.
fn the_new_pull_request_panel_stacks_and_says_what_went_wrong() {
    let mut stage = pulls::open_section(false);
    let _standing = stand_in(answers(Err(
        "opening a pull request: not built yet".to_owned()
    )));
    start(&mut stage, "feature/dates", Some("origin/feature/lexer"));
    let seen = new_probe(&mut stage);
    assert_eq!(seen.into, "origin feature/lexer");
    stage.until("the stack is known", |stage| {
        stage.settle();
        new_probe(stage).stack.is_some()
    });
    assert_eq!(new_probe(&mut stage).stack, Some((39, true)));
    stage.click("new-pull-stack");
    let seen = new_probe(&mut stage);
    assert_eq!(seen.stack, Some((39, false)));
    assert_eq!(seen.into, "origin main", "where #39 goes");
    stage.click("new-pull-stack");
    // Without a title it cannot be made.
    stage.click("new-pull-title");
    stage.keys("ctrl-a backspace");
    assert!(!new_probe(&mut stage).ready);
    stage.type_text("Dates in the log");
    assert!(new_probe(&mut stage).ready);
    panel_button(&mut stage, "new-pull-create");
    stage.until("the provider answered", |stage| {
        stage.settle();
        !new_probe(stage).busy
    });
    let seen = new_probe(&mut stage);
    assert_eq!(stage.workspace().sheet, Some("new_pull"));
    assert!(
        seen.error
            .as_deref()
            .is_some_and(|error| error.contains("not built yet")),
        "{seen:?}"
    );
    let asked = calls::asked();
    let create = asked.iter().find(|(id, _)| id == "pr.create").unwrap();
    assert_eq!(create.1["base"], json!("feature/lexer"), "on top of #39");
    assert_eq!(create.1["title"], json!("Dates in the log"));
    panel_button(&mut stage, "new-pull-cancel");
    assert_eq!(stage.workspace().sheet, None);
}

/// The pictures: the panel filled in, and into a branch with a pull
/// request of its own after the provider refused; the drop menu with the
/// entry that starts one.
pub fn pull_writes(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    pulls::give(&mut stage, pulls::given());
    {
        let _standing = stand_in(answers(Err(
            "opening a pull request: not built yet".to_owned()
        )));
        start(&mut stage, "wip", None);
        fill(&mut stage);
        shots.take(&mut stage, "new_pull", Themes::Two);
        panel_button(&mut stage, "new-pull-cancel");

        start(&mut stage, "feature/dates", Some("origin/feature/lexer"));
        stage.until("the stack is known", |stage| {
            stage.settle();
            new_probe(stage).stack.is_some()
        });
        panel_button(&mut stage, "new-pull-create");
        stage.until("the provider answered", |stage| {
            stage.settle();
            new_probe(stage).error.is_some()
        });
        shots.take(&mut stage, "new_pull_stack", Themes::Two);
        panel_button(&mut stage, "new-pull-cancel");
    }
    stage.pass(Duration::from_millis(100));
    drop(stage);

    // Where it starts: a branch's menu in the left panel, and a branch
    // dropped on another there.
    repos::fresh();
    let repo = repos::project();
    let state = format!(
        "[repos.{repo:?}]\nclosed_sections = [\"remote\", \"pulls\", \"worktrees\", \"stashes\", \"tags\", \"submodules\"]\n"
    );
    let mut stage = Stage::open(Start::with(&repo).state(state).pictures());
    pulls::give(&mut stage, pulls::given());
    let wip = name_at(&mut stage, "refs/heads/wip");
    stage.right_click_at((f32::from(wip.x), f32::from(wip.y)));
    stage.pass(Duration::from_millis(300));
    shots.take(&mut stage, "menu_branch_pull", Themes::Two);
    stage.keys("escape");
    stage.pass(Duration::from_millis(300));
    let main = name_at(&mut stage, "refs/heads/main");
    stage.drag_at(
        (f32::from(wip.x), f32::from(wip.y)),
        (f32::from(main.x), f32::from(main.y)),
    );
    stage.settle();
    shots.take(&mut stage, "menu_drop_pull", Themes::Two);
    stage.keys("escape");
}

/// The stack #40 (bottom), #41, #42 (top), into main; `at` is the place
/// of the one asked about.
fn stack(at: usize) -> Stack {
    let entry = |number: u64, head: &str, base: &str, title: &str| StackEntry {
        number,
        title: title.to_owned(),
        head_ref: head.to_owned(),
        base_ref: base.to_owned(),
        state: PullState::Open,
    };
    Stack {
        entries: vec![
            entry(40, "pages", "main", "A landing page for the docs"),
            entry(41, "wip", "pages", "Quotes and the needle in one pass"),
            entry(42, "printer", "wip", "Print what was lexed"),
        ],
        position: at,
    }
}

/// What the provider answers for a pull request's view: `details` for
/// pr.show (and pr.update with the change made), its stack, and what each
/// write answers.
fn view_answers(details: PullDetails) -> impl Fn(&str, &Value) -> Option<Result<Value, String>> {
    let held = std::rc::Rc::new(std::cell::RefCell::new(details));
    move |id, params| match id {
        "pr.show" => {
            let mut shown = held.borrow().clone();
            if let Some(number) = params["number"].as_u64() {
                shown.pull.number = number;
            }
            Some(Ok(json!(shown)))
        }
        "pr.options" => Some(Ok(json!(options()))),
        "pr.stack" => {
            let at = match params["number"].as_u64() {
                Some(40) => 0,
                Some(42) => 2,
                _ => 1,
            };
            Some(Ok(json!(stack(at))))
        }
        "pr.update" => {
            let mut changed = held.borrow_mut();
            if let Some(title) = params["title"].as_str() {
                if title == "refuse" {
                    return Some(Err("the provider said no".to_owned()));
                }
                changed.pull.title = title.to_owned();
            }
            if let Some(body) = params["body"].as_str() {
                changed.body = body.to_owned();
            }
            if let Some(reviewers) = params["reviewers"].as_array() {
                changed.pull.reviewers = reviewers
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|login| bitbraid_integrations::model::Reviewer {
                        user: person(login),
                        state: ReviewState::Pending,
                    })
                    .collect();
            }
            if let Some(labels) = params["labels"].as_array() {
                changed.pull.labels = labels
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|name| label(name, "5319e7"))
                    .collect();
            }
            if let Some(milestone) = params["milestone"].as_str() {
                changed.milestone = options()
                    .milestones
                    .into_iter()
                    .find(|known| known.id == milestone);
            }
            Some(Ok(json!(*changed)))
        }
        "pr.merge" => Some(Ok(json!(Merged {
            merged: true,
            sha: Some("abc".to_owned()),
            message: None,
            retargeted: vec![42],
            branch_deleted: params["delete_branch"] == json!(true),
        }))),
        "pr.comment" => Some(Ok(json!(Comment {
            id: "c9".to_owned(),
            author: person("octo"),
            body: params["body"].as_str().unwrap_or_default().to_owned(),
            created: "2020-01-03T12:00:00Z".to_owned(),
            updated: None,
        }))),
        "pr.review" => Some(Ok(json!(Review {
            id: "r9".to_owned(),
            author: person("octo"),
            state: ReviewState::Approved,
            body: String::new(),
            submitted: None,
        }))),
        "pr.checkout" => Some(Ok(json!({
            "branch": "wip",
            "commit": "0".repeat(40),
            "created": false,
            "updated": true,
            "note": null,
            "upstream": "origin/wip",
            "remote_added": null,
        }))),
        _ => None,
    }
}

fn the_view(stage: &mut Stage) -> Entity<PullView> {
    stage.with_workspace(|workspace, _, cx| {
        workspace
            .read(cx)
            .pull_view()
            .expect("a pull request is shown")
    })
}

fn seen(stage: &mut Stage) -> PullViewProbe {
    pulls::view_probe(stage).expect("a pull request is shown")
}

/// Do something with the view, in the window.
fn with_view(
    stage: &mut Stage,
    f: impl FnOnce(&mut PullView, &mut Window, &mut gpui_kit::Context<PullView>),
) {
    let view = the_view(stage);
    stage.window(|window, cx| view.update(cx, |view, cx| f(view, window, cx)));
    stage.settle();
}

/// The section given, and #41 opened from its row, read from the
/// stand-ins.
fn open_41(details: PullDetails, pictures: bool) -> (Stage, Standing) {
    repos::fresh();
    let repo = repos::project_with_everything();
    let start = Start::with(&repo);
    let mut stage = Stage::open(if pictures { start.pictures() } else { start });
    pulls::give(&mut stage, pulls::given());
    pulls::toggle(&mut stage, "pulls");
    let standing = stand_in(view_answers(details));
    stage.click(pulls::row("assigned", 0, 41));
    stage.until("the pull request is read", |stage| {
        stage.settle();
        pulls::view_probe(stage).is_some_and(|seen| !seen.loading && seen.title.is_some())
    });
    (stage, standing)
}

/// Two clicks on a control, as a person makes them.
pub fn double_click(stage: &mut Stage, id: impl Into<ElementId>) {
    let (x, y, w, h) = stage.place(id);
    let position = point(px(x + w / 2.), px(y + h / 2.));
    stage.window(|window, cx| {
        for count in [1, 2] {
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Default::default(),
                    click_count: count,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Default::default(),
                    click_count: count,
                }
                .to_platform_input(),
                cx,
            );
        }
    });
    stage.settle();
}

/// The last time a command was asked, with what.
fn last_asked(id: &str) -> Option<Value> {
    calls::asked()
        .into_iter()
        .rev()
        .find(|(asked, _)| asked == id)
        .map(|(_, params)| params)
}

/// The pencils change the title and the description in fields of their
/// own, and the lists of the right column from what can be chosen; each
/// change is sent at once and shown as the provider answers it. What the
/// provider refuses is said, and the field stays.
fn a_pull_requests_title_description_and_lists_are_changed_in_place() {
    let (mut stage, _standing) = open_41(pulls::details(), false);
    stage.click("pull-title-edit");
    assert_eq!(seen(&mut stage).editing, Some("title"));
    stage.keys("ctrl-a backspace");
    stage.type_text("One pass for quotes and the needle");
    stage.keys("enter");
    stage.until("the title is changed", |stage| {
        stage.settle();
        seen(stage).editing.is_none()
    });
    assert_eq!(
        last_asked("pr.update").unwrap()["title"],
        json!("One pass for quotes and the needle")
    );
    assert_eq!(
        seen(&mut stage).title.as_deref(),
        Some("One pass for quotes and the needle")
    );
    // What the provider refuses stays in the field, and is said.
    stage.click("pull-title-edit");
    stage.keys("ctrl-a backspace");
    stage.type_text("refuse");
    stage.click("pull-title-save");
    stage.until("the provider answered", |stage| {
        stage.settle();
        seen(stage).sending.is_none()
    });
    let now = seen(&mut stage);
    assert_eq!(now.editing, Some("title"));
    assert!(
        now.write_error
            .as_deref()
            .is_some_and(|error| error.contains("said no")),
        "{now:?}"
    );
    stage.click("pull-title-cancel");
    assert_eq!(seen(&mut stage).editing, None);

    stage.click("pull-edit");
    assert_eq!(seen(&mut stage).editing, Some("body"));
    stage.keys("ctrl-a backspace");
    stage.type_text("Lexes once.");
    stage.click("pull-body-save");
    stage.until("the description is changed", |stage| {
        stage.settle();
        seen(stage).editing.is_none()
    });
    assert_eq!(
        last_asked("pr.update").unwrap()["body"],
        json!("Lexes once.")
    );

    // The lists: the pencil reads what can be chosen; a pick is sent at once.
    stage.click("pull-edit-reviewers");
    stage.keys("escape");
    with_view(&mut stage, |view, window, cx| {
        view.toggle_listed(Listed::Reviewers, "cy", window, cx)
    });
    assert_eq!(
        last_asked("pr.update").unwrap()["reviewers"],
        json!(["bob", "cy"])
    );
    with_view(&mut stage, |view, window, cx| {
        view.toggle_listed(Listed::Labels, "enhancement", window, cx)
    });
    assert_eq!(last_asked("pr.update").unwrap()["labels"], json!(["lexer"]));
    with_view(&mut stage, |view, window, cx| {
        view.toggle_listed(Listed::Milestone, "", window, cx)
    });
    assert_eq!(last_asked("pr.update").unwrap()["milestone"], json!(""));
    assert!(
        last_asked("pr.options").is_some(),
        "the pencil asked what can be chosen"
    );
}

/// The heading says where in its stack it is and goes to the one below
/// and above; the merge box lists the stack, merges in the way chosen
/// after asking in the bar, removes the branch when asked, moves the one
/// above onto its base, and says so.
fn a_pull_request_in_a_stack_is_merged_after_asking_and_its_stack_walked() {
    let (mut stage, _standing) = open_41(pulls::details(), false);
    stage.until("the stack is read", |stage| {
        stage.settle();
        seen(stage).stack.is_some()
    });
    let now = seen(&mut stage);
    assert_eq!(now.stack, Some((2, 3)));
    assert_eq!(
        now.method,
        Some(MergeMethod::Merge),
        "the first the repository allows"
    );
    assert!(now.retarget && !now.delete_branch);
    assert!(
        scroll_to(&mut stage, ("pull-stack-entry", 2usize)),
        "the top of the stack is listed"
    );
    with_view(&mut stage, |view, _, cx| {
        view.choose_method(MergeMethod::Squash, cx)
    });
    assert!(scroll_to(&mut stage, "pull-merge-delete"));
    stage.click("pull-merge-delete");
    assert!(seen(&mut stage).delete_branch);
    stage.click("pull-merge");
    let asked = stage.app(|cx| bitbraid_ui::ask::on_show(cx).map(|(said, _)| said));
    assert!(
        asked
            .as_deref()
            .is_some_and(|said| said.contains("squashed")),
        "{asked:?}"
    );
    assert!(
        last_asked("pr.merge").is_none(),
        "nothing is merged before the bar is answered"
    );
    stage.click(("ask-choice", 0usize));
    stage.until("it is merged", |stage| {
        stage.settle();
        seen(stage).sending.is_none() && last_asked("pr.merge").is_some()
    });
    let merge = last_asked("pr.merge").unwrap();
    assert_eq!(merge["method"], json!("squash"));
    assert_eq!(merge["delete_branch"], json!(true));
    assert_eq!(merge["retarget_stack"], json!(true));
    assert_eq!(merge["number"], json!(41));
    assert!(
        stage
            .notices()
            .iter()
            .any(|notice| notice.contains("Merged #41") && notice.contains("#42 now goes")),
        "{:?}",
        stage.notices()
    );
    // Up the stack: #42.
    assert!(scroll_to(&mut stage, "pull-stack-up"));
    stage.click("pull-stack-up");
    stage.until("#42 is shown", |stage| {
        stage.settle();
        pulls::view_probe(stage).is_some_and(|seen| seen.number == 42 && !seen.loading)
    });
    stage.until("its stack is read", |stage| {
        stage.settle();
        seen(stage).stack == Some((3, 3))
    });
    assert!(scroll_to(&mut stage, "pull-stack-down"));
    stage.click("pull-stack-down");
    stage.until("#41 is shown again", |stage| {
        stage.settle();
        pulls::view_probe(stage).is_some_and(|seen| seen.number == 41)
    });
}

/// Scroll the view's page from its top down until `id` is on show, and
/// a little further; whether it came.
fn scroll_to(stage: &mut Stage, id: impl Into<gpui_kit::ElementId>) -> bool {
    let id = id.into();
    let by = |stage: &mut Stage, y: f32| {
        stage.window(|window, cx| {
            window.scroll(
                "pull-page",
                gpui_kit::ScrollDelta::Pixels(point(px(0.), px(y))),
                cx,
            )
        });
        stage.frame();
    };
    by(stage, 4000.);
    if stage.shows(id.clone()) {
        return true;
    }
    for _ in 0..80 {
        by(stage, -40.);
        if stage.shows(id.clone()) {
            // Well inside the page, not at its foot.
            by(stage, -160.);
            return stage.shows(id);
        }
    }
    false
}

/// The field at the bottom comments on the whole of it; a comment's menu
/// answers in its conversation, or quotes it; the Review button sends a
/// verdict with what the field holds; what was sent is read again.
fn comments_are_answered_and_quoted_and_reviews_given_from_the_field() {
    let (mut stage, _standing) = open_41(pulls::details(), false);
    stage.window(|window, cx| {
        window.scroll(
            "pull-page",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        )
    });
    stage.frame();
    stage.click("pull-comment-field");
    stage.type_text("Looks right to me");
    stage.click("pull-comment");
    stage.until("the comment is sent", |stage| {
        stage.settle();
        seen(stage).composer.is_empty()
    });
    let comment = last_asked("pr.comment").unwrap();
    assert_eq!(comment["body"], json!("Looks right to me"));
    assert!(comment.get("reply_to").is_none());

    with_view(&mut stage, |view, window, cx| {
        view.reply_to_thread(1, window, cx)
    });
    assert_eq!(seen(&mut stage).reply_to.as_deref(), Some("t2"));
    stage.type_text("Quotes first, then.");
    stage.click("pull-comment");
    stage.until("the answer is sent", |stage| {
        stage.settle();
        seen(stage).composer.is_empty()
    });
    let answer = last_asked("pr.comment").unwrap();
    assert_eq!(answer["reply_to"], json!("t2"));
    assert_eq!(seen(&mut stage).reply_to, None);

    with_view(&mut stage, |view, window, cx| {
        view.quote("Should `needle()` come first?", Some(1), window, cx)
    });
    let now = seen(&mut stage);
    assert!(now.composer.starts_with("> Should"), "{now:?}");
    assert_eq!(now.reply_to.as_deref(), Some("t2"));
    with_view(&mut stage, |view, _, cx| view.clear_reply(cx));
    stage.keys("ctrl-a backspace");

    // At GitHub, asking for changes needs words; approving does not.
    with_view(&mut stage, |view, window, cx| {
        view.send_review(Verdict::RequestChanges, window, cx)
    });
    assert!(last_asked("pr.review").is_none());
    with_view(&mut stage, |view, window, cx| {
        view.send_review(Verdict::Approve, window, cx)
    });
    stage.until("the review is sent", |stage| {
        stage.settle();
        seen(stage).sending.is_none()
    });
    assert_eq!(last_asked("pr.review").unwrap()["event"], json!("approve"));
}

/// Two clicks on the branch it brings check it out with `pr.checkout`;
/// from a fork none of the remotes is, after asking. Two clicks on the
/// branch it goes into check that out.
fn two_clicks_on_a_pull_requests_branches_check_them_out() {
    let (mut stage, _standing) = open_41(pulls::details(), false);
    double_click(&mut stage, "pull-head-branch");
    let checkout = last_asked("pr.checkout").expect("checked out");
    assert_eq!(checkout["number"], json!(41));
    assert!(
        stage
            .notices()
            .iter()
            .any(|notice| notice.contains("Checked out #41 in wip")),
        "{:?}",
        stage.notices()
    );
    drop(_standing);

    // From a fork: asked first.
    let mut forked = pulls::details();
    forked.pull.head_repo = Some("ada/tools".to_owned());
    let (mut stage, _standing) = open_41(forked, false);
    double_click(&mut stage, "pull-head-branch");
    assert!(
        last_asked("pr.checkout").is_none(),
        "not before the bar is answered"
    );
    let asked = stage.app(|cx| bitbraid_ui::ask::on_show(cx).map(|(said, _)| said));
    assert!(
        asked
            .as_deref()
            .is_some_and(|said| said.contains("ada/tools")),
        "{asked:?}"
    );
    stage.click(("ask-choice", 0usize));
    assert!(last_asked("pr.checkout").is_some());

    // The branch it goes into: main, checked out already, stays so.
    double_click(&mut stage, "pull-base-branch");
    stage.settle();
    assert_eq!(stage.workspace().sheet, Some("pull"));
}

/// The entry of the menu of a branch that is not the checked-out one makes
/// a pull request of the checked-out branch into it (no branch named: the
/// checked-out one), and the panel stands at the graph's left, 600 wide,
/// the graph beside it.
fn the_entry_of_another_branchs_menu_makes_one_of_the_checked_out_into_it() {
    let mut stage = pulls::open_section(false);
    let _standing = stand_in(answers(Err("not used".to_owned())));
    start(&mut stage, "", Some("origin/feature/lexer"));
    let seen = new_probe(&mut stage);
    assert_eq!(seen.from, "origin main", "the checked-out branch");
    assert_eq!(seen.into, "origin feature/lexer");
    let (x, _, w, _) = stage.place("new-pull");
    assert_eq!(w, 600.);
    let width = stage.window(|window, _| f32::from(window.viewport_size().width));
    assert!(
        x > 100. && x + w < width - 200.,
        "at the middle's left, the rest of the graph beside it: {x} {w} in {width}"
    );
}

/// Two clicks on a pull request's row check out the branch it brings
/// (`pr.checkout`, as the view's branch does), and the other button opens
/// the row's menu, which checks out too.
fn a_pull_requests_row_checks_it_out_on_a_double_click_and_has_a_menu() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    pulls::give(&mut stage, pulls::given());
    pulls::toggle(&mut stage, "pulls");
    let _standing = stand_in(view_answers(pulls::details()));
    double_click(&mut stage, pulls::row("assigned", 0, 41));
    stage.settle();
    let asked = last_asked("pr.checkout").expect("checked out from the row");
    assert_eq!(asked["number"], json!(41));
    let before = calls::asked().len();
    stage.right_click(pulls::row("assigned", 0, 41));
    assert!(stage.shows("popup-menu"), "the row's menu");
    // Its page, its link; shown here, checked out, its head in the graph.
    stage.keys("down down down down enter");
    stage.settle();
    let after: Vec<String> = calls::asked()
        .into_iter()
        .skip(before)
        .map(|(id, _)| id)
        .collect();
    assert!(
        after.iter().any(|id| id == "pr.checkout"),
        "checked out from the menu: {after:?}"
    );
}

/// The pictures of a pull request's view while it is changed: the title
/// and the description in their fields, the stack in the heading and the
/// merge box, the field answering a conversation.
pub fn pull_view_writes(shots: &mut Shots) {
    let mut details = pulls::details();
    details.mergeable = Mergeable::Blocked;
    let (mut stage, standing) = open_41(details, true);
    stage.until("the stack is read", |stage| {
        stage.settle();
        seen(stage).stack.is_some()
    });
    stage.click("pull-edit");
    shots.take(&mut stage, "pull_view_editing", Themes::Two);
    stage.click("pull-body-cancel");
    with_view(&mut stage, |view, window, cx| {
        view.choose_method(MergeMethod::Squash, cx);
        view.reply_to_thread(1, window, cx);
    });
    stage.type_text("Quotes first, then.");
    stage.window(|window, cx| {
        window.scroll(
            "pull-page",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        )
    });
    stage.frame();
    shots.take(&mut stage, "pull_view_merge", Themes::Two);
    // Merging is asked in the bar first.
    stage.click("pull-merge");
    shots.take(&mut stage, "pull_merge_ask", Themes::Two);
    stage.click("ask-cancel");
    drop(standing);
    stage.pass(Duration::from_millis(100));
}
