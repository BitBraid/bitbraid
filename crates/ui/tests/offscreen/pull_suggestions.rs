//! Suggestions from local edits in a pull request's view: the
//! button over a file's changes, the look at what would be sent, sending
//! it, and giving up. The provider commands and `pr.suggestions` are
//! answered by stand-ins (`views::pulls::calls`), so nothing depends on a
//! network, an account or the working tree; the engine's side is tested
//! in the integrations crate, live and replayed.

use bitbraid_integrations::{
    model::{Comment, PullDetails},
    suggest::{Suggestion, Suggestions},
};
use bitbraid_ui::views::pulls::{calls, view::PullViewProbe};
use serde_json::{Value, json};

use crate::{
    pull_writes::{Standing, stand_in},
    pulls, repos,
    runner::{Shots, Test, Themes},
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "a_files_edits_are_shown_first_and_sent_as_suggestions",
        run: a_files_edits_are_shown_first_and_sent_as_suggestions,
    },
    Test {
        name: "suggestions_need_the_branch_checked_out_and_can_be_given_up",
        run: suggestions_need_the_branch_checked_out_and_can_be_given_up,
    },
];

/// What the engine says the edits to `src/lexer.rs` would be sent as: one
/// on a line of the pull request's changes, one that is not.
fn offered() -> Suggestions {
    Suggestions {
        head_sha: "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".to_owned(),
        checked_out: Some("a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".to_owned()),
        suggestions: vec![
            Suggestion {
                path: "src/lexer.rs".to_owned(),
                start_line: 2,
                line: 3,
                replaced: vec!["    quotes();".to_owned(), "    needle();".to_owned()],
                suggested: vec!["    quotes_and_needle();".to_owned()],
                body: "```suggestion\n    quotes_and_needle();\n```".to_owned(),
                why_not: None,
            },
            Suggestion {
                path: "src/lexer.rs".to_owned(),
                start_line: 9,
                line: 9,
                replaced: vec!["fn rest() {}".to_owned()],
                suggested: vec!["fn rest() { done(); }".to_owned()],
                body: "```suggestion\nfn rest() { done(); }\n```".to_owned(),
                why_not: Some(
                    "Its lines are not all in one part of the pull request's changes, where \
                     comments on lines go."
                        .to_owned(),
                ),
            },
        ],
    }
}

/// #41 with its branch named `head`, read from the stand-ins.
fn details(head: &str) -> PullDetails {
    let mut details = pulls::details();
    details.pull.head_ref = head.to_owned();
    details.pull.base_ref = "next".to_owned();
    details
}

fn answers(details: PullDetails) -> impl Fn(&str, &Value) -> Option<Result<Value, String>> {
    move |id, params| match id {
        "pr.show" => Some(Ok(json!(details))),
        "pr.stack" | "pr.options" => Some(Err("not here".to_owned())),
        "pr.suggestions" => Some(Ok(json!(offered()))),
        "pr.comment" => Some(Ok(json!(Comment {
            id: "c9".to_owned(),
            author: pulls::user("octo"),
            body: params["body"].as_str().unwrap_or_default().to_owned(),
            created: "2020-01-03T12:00:00Z".to_owned(),
            updated: None,
        }))),
        _ => None,
    }
}

/// The project on its `main`, #41 opened from its row with the changes
/// of its first file on show.
fn open_file(head: &str, pictures: bool) -> (Stage, Standing) {
    repos::fresh();
    let repo = repos::project_with_everything();
    let start = Start::with(&repo);
    let mut stage = Stage::open(if pictures { start.pictures() } else { start });
    pulls::give(&mut stage, pulls::given());
    pulls::toggle(&mut stage, "pulls");
    let standing = stand_in(answers(details(head)));
    stage.click(pulls::row("assigned", 0, 41));
    stage.until("the pull request is read", |stage| {
        stage.settle();
        pulls::view_probe(stage).is_some_and(|seen| !seen.loading && seen.title.is_some())
    });
    stage.click(("pull-file-row", 0usize));
    stage.settle();
    assert_eq!(seen(&mut stage).file_open.as_deref(), Some("src/lexer.rs"));
    (stage, standing)
}

fn seen(stage: &mut Stage) -> PullViewProbe {
    pulls::view_probe(stage).expect("a pull request is shown")
}

fn asked(id: &str) -> Vec<Value> {
    calls::asked()
        .into_iter()
        .filter(|(asked, _)| asked == id)
        .map(|(_, params)| params)
        .collect()
}

/// Show what the edits would be sent as.
fn look(stage: &mut Stage) {
    stage.click("pull-file-suggest");
    stage.until("the suggestions are read", |stage| {
        stage.settle();
        seen(stage)
            .suggesting
            .is_some_and(|suggesting| suggesting.answer.is_some())
    });
    stage.frame();
    assert!(stage.shows("pull-suggesting"), "the look is drawn");
}

/// With the pull request's branch checked out, the button over a file's
/// changes asks for that file's suggestions on the repository's queue and
/// shows them in the file's place, each with its lines, what it replaces
/// and what it offers, one that cannot be sent with why; Send sends each
/// that can be on its lines, and the conversation comes back.
fn a_files_edits_are_shown_first_and_sent_as_suggestions() {
    let (mut stage, _standing) = open_file("main", false);
    assert_eq!(seen(&mut stage).suggest_refused, None);
    look(&mut stage);
    let wanted = asked("pr.suggestions");
    assert_eq!(wanted.len(), 1, "{wanted:?}");
    assert_eq!(wanted[0]["path"], "src/lexer.rs");
    assert_eq!(wanted[0]["number"], 41);
    assert!(stage.shows(("pull-suggestion", 0usize)));
    assert!(stage.shows(("pull-suggestion", 1usize)));
    // Nothing is sent before Send.
    assert!(asked("pr.comment").is_empty());
    stage.click("pull-suggest-send");
    stage.until("the suggestions are sent", |stage| {
        stage.settle();
        seen(stage).suggesting.is_none()
    });
    let sent = asked("pr.comment");
    assert_eq!(sent.len(), 1, "only the one that can be: {sent:?}");
    assert_eq!(sent[0]["path"], "src/lexer.rs");
    assert_eq!(sent[0]["start_line"], 2);
    assert_eq!(sent[0]["line"], 3);
    assert_eq!(
        sent[0]["body"],
        "```suggestion\n    quotes_and_needle();\n```"
    );
    let after = seen(&mut stage);
    assert_eq!(after.write_error, None);
    assert_eq!(after.file_open, None, "the conversation is on show again");
}

/// On another branch the button says why it cannot be used, and asks
/// nothing; with the branch checked out, Cancel and the arrow give the
/// look up without sending.
fn suggestions_need_the_branch_checked_out_and_can_be_given_up() {
    let (mut stage, standing) = open_file("wip", false);
    let refused = seen(&mut stage).suggest_refused;
    assert!(
        refused.is_some_and(|why| why.contains("Check out its branch")),
        "{refused:?}"
    );
    stage.click("pull-file-suggest");
    stage.settle();
    assert!(asked("pr.suggestions").is_empty());
    assert!(seen(&mut stage).suggesting.is_none());
    drop(standing);
    drop(stage);
    let (mut stage, _standing) = open_file("main", false);
    look(&mut stage);
    stage.click("pull-suggest-cancel");
    stage.settle();
    assert!(seen(&mut stage).suggesting.is_none());
    assert_eq!(
        seen(&mut stage).file_open.as_deref(),
        Some("src/lexer.rs"),
        "back to the file's changes"
    );
    look(&mut stage);
    stage.click("pull-suggest-back");
    stage.settle();
    assert!(seen(&mut stage).suggesting.is_none());
    assert!(asked("pr.comment").is_empty());
}

/// The file's changes with the button, and the look at what would be
/// sent, in both themes.
pub fn pull_suggestions(shots: &mut Shots) {
    let (mut stage, _standing) = open_file("main", true);
    shots.take(&mut stage, "pull_file_suggest", Themes::Two);
    look(&mut stage);
    shots.take(&mut stage, "pull_suggestions", Themes::Two);
}
