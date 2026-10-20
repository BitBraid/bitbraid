//! Issues in the window: what the ISSUES section, an issue's card and view,
//! a branch for an issue, issue keys as links and the trailer of the commit
//! box do, and their pictures. They are given the shapes the provider
//! commands answer with (`bitbraid_integrations::model`), made here, or
//! answers recorded from the providers, so that nothing depends on a
//! network or an account.

use std::time::Duration;

use bitbraid_integrations::model::{
    Comment, Freshness, Issue, IssueDetails, IssueState, Label, Listing, Milestone, ProviderKind,
    RepoLink, User,
};
use bitbraid_ui::{
    prefs::Prefs,
    views::{
        issues::{
            card::{self, Known},
            facts::IssueGroup,
            section::{Given, IssuesProbe},
            view::IssueViewProbe,
        },
        pulls::calls,
    },
};
use gpui_kit::{ElementId, Pixels, Point, px};
use serde_json::{Value, json};

use crate::{
    repos,
    runner::{Shots, Test, Themes},
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "the_issues_section_shows_with_a_linked_remote_and_its_groups_close",
        run: the_issues_section_shows_with_a_linked_remote_and_its_groups_close,
    },
    Test {
        name: "the_issues_search_box_keeps_the_issues_that_match",
        run: the_issues_search_box_keeps_the_issues_that_match,
    },
    Test {
        name: "the_issues_query_asks_the_provider_and_a_repository_is_chosen",
        run: the_issues_query_asks_the_provider_and_a_repository_is_chosen,
    },
    Test {
        name: "an_issue_opens_in_the_middle_with_its_branches_and_goes_back",
        run: an_issue_opens_in_the_middle_with_its_branches_and_goes_back,
    },
    Test {
        name: "a_branch_for_an_issue_is_named_in_the_bar_and_made",
        run: a_branch_for_an_issue_is_named_in_the_bar_and_made,
    },
    Test {
        name: "an_issues_card_reads_it_once_for_its_description",
        run: an_issues_card_reads_it_once_for_its_description,
    },
    Test {
        name: "issue_keys_in_messages_and_branch_names_show_their_issue",
        run: issue_keys_in_messages_and_branch_names_show_their_issue,
    },
    Test {
        name: "the_commit_box_names_the_issue_of_the_branch",
        run: the_commit_box_names_the_issue_of_the_branch,
    },
    Test {
        name: "hostile_issues_are_shown_without_harm",
        run: hostile_issues_are_shown_without_harm,
    },
    Test {
        name: "recorded_issues_of_a_gitlab_and_of_github_fill_the_section_and_a_view",
        run: recorded_issues_of_a_gitlab_and_of_github_fill_the_section_and_a_view,
    },
    Test {
        name: "an_issue_key_in_a_branch_name_is_a_link_in_the_references_panel",
        run: an_issue_key_in_a_branch_name_is_a_link_in_the_references_panel,
    },
    Test {
        name: "thousands_of_issues_draw_the_rows_in_view_and_the_same_ones_tell_nothing",
        run: thousands_of_issues_draw_the_rows_in_view_and_the_same_ones_tell_nothing,
    },
];

/// A project with three thousand open issues: the section draws only the
/// rows in view, as it is scrolled too, and reading the lists again every
/// five minutes, answered the same, does not make the references panel
/// over.
fn thousands_of_issues_draw_the_rows_in_view_and_the_same_ones_tell_nothing() {
    use bitbraid_ui::views::issues::section::{IssuesEvent, READ_EVERY};
    use gpui_kit::test::TestWindowExt as _;
    use std::{cell::Cell, rc::Rc};
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let many: Vec<Issue> = (1..=3000u64)
        .map(|n| issue(n, &format!("Problem {n}"), "bob"))
        .collect();
    let mine: Vec<Issue> = many.iter().step_by(10).cloned().collect();
    let all = serde_json::to_value(listing(many)).unwrap();
    let assigned = serde_json::to_value(listing(mine)).unwrap();
    stand_in(move |id, params| {
        (id == "issue.list").then(|| {
            Ok(if params["filter"] == "assigned" {
                assigned.clone()
            } else {
                all.clone()
            })
        })
    });
    let issues = stage.with_workspace(|workspace, _, cx| workspace.read(cx).issues());
    // How many times the panel was told the counts.
    let counted = Rc::new(Cell::new(0usize));
    let counter = counted.clone();
    stage.app(|cx| {
        cx.subscribe(&issues, move |_, event: &IssuesEvent, _| {
            if matches!(event, IssuesEvent::Counted) {
                counter.set(counter.get() + 1);
            }
        })
        .detach()
    });
    let link = links().into_iter().next().unwrap();
    stage.app(|cx| issues.update(cx, |view, cx| view.link_to(vec![link], cx)));
    toggle(&mut stage, "issues");
    stage.until("the lists are read", |stage| {
        stage.settle();
        let seen = probe(stage);
        !seen.reading && seen.rows.len() > 3000
    });
    let drawn = |stage: &mut Stage| -> Vec<u64> {
        stage.window(|window, _| {
            (1..=3000u64)
                .filter(|number| window.try_find(row("open", *number)).is_some())
                .collect()
        })
    };
    let first = drawn(&mut stage);
    assert!(
        first.first() == Some(&1) && first.len() <= 60,
        "the rows in view: {} drawn, {first:?}",
        first.len()
    );
    let (x, y, w, h) = stage.place(row("open", 1));
    crate::behaviour_fit::wheel_at(&mut stage, (x + w / 2., y + h / 2.), -24. * 1500.);
    let scrolled = drawn(&mut stage);
    assert!(
        !scrolled.contains(&1)
            && scrolled.iter().any(|number| (1400..=1600).contains(number))
            && scrolled.len() <= 60,
        "the rows in view, scrolled: {} drawn, {scrolled:?}",
        scrolled.len()
    );

    let told = counted.get();
    let reads = || {
        calls::asked()
            .iter()
            .filter(|(id, _)| id == "issue.list")
            .count()
    };
    let before = reads();
    for _ in 0..3 {
        stage.pass(READ_EVERY + Duration::from_secs(1));
        stage.settle();
    }
    assert!(reads() > before, "read again every five minutes");
    assert_eq!(
        counted.get(),
        told,
        "the same lists do not make the references panel over"
    );
    calls::stand_down();
}

/// In the references panel, the key a branch's name holds (`1` of
/// `qa/1-the-button-does-nothing`, `#7` of `qa/#7-button`) is a link of the
/// linked remote: the pointer on the row shows the issue's card, which is
/// read for it, and a click on the key opens the issue.
fn an_issue_key_in_a_branch_name_is_a_link_in_the_references_panel() {
    repos::fresh();
    let repo = repos::project_with_everything();
    repos::git(&repo, &["branch", "qa/1-the-button-does-nothing", "main~1"]);
    repos::git(&repo, &["branch", "qa/#7-button", "main~1"]);
    // The local branches open, and nothing else, so that all have rows.
    let state = format!(
        "[repos.{repo:?}]\nclosed_sections = [\"gitflow\", \"remote\", \"worktrees\", \
         \"stashes\", \"pulls\", \"issues\", \"tags\", \"submodules\"]\n"
    );
    let mut stage = Stage::open(Start::with(&repo).state(state));
    stand_in(|id, params| {
        (id == "issue.show").then(|| {
            let number = params["number"].as_u64().unwrap_or(0);
            Ok(serde_json::to_value(details(number)).unwrap())
        })
    });
    give(&mut stage, given());
    let key = |name: &str| ElementId::Name(format!("ref-key:refs/heads/{name}").into());
    stage.until("the keys of the names are links", |stage| {
        stage.settle();
        stage.shows(key("qa/1-the-button-does-nothing")) && stage.shows(key("qa/#7-button"))
    });
    // The pointer on the row: the card of its issue is read.
    let (x, y, w, h) = stage.place(ElementId::Name("ref-name:refs/heads/qa/#7-button".into()));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    stage.until("the issue of the branch is read for its card", |stage| {
        stage.settle();
        known(stage, 7).is_some_and(|known| matches!(known, Known::Read(_)))
    });
    // A click on the key opens it.
    let (_, _, _, h) = stage.place(key("qa/1-the-button-does-nothing"));
    stage.click_in(key("qa/1-the-button-does-nothing"), (3., h / 2.));
    stage.settle();
    assert_eq!(stage.workspace().sheet, Some("issue"));
    assert_eq!(view_probe(&mut stage).map(|seen| seen.number), Some(1));
    stage.click("issue-close");
    calls::stand_down();
}

/// The clock of the tests stands at 2020-01-03 12:00 UTC: these are an
/// hour, a day and two days before.
const HOUR_AGO: &str = "2020-01-03T11:00:00Z";
const DAY_AGO: &str = "2020-01-02T12:00:00Z";
const TWO_DAYS_AGO: &str = "2020-01-01T12:00:00Z";

fn user(login: &str) -> User {
    User {
        login: login.to_owned(),
        ..User::default()
    }
}

fn label(name: &str, colour: &str) -> Label {
    Label {
        name: name.to_owned(),
        color: Some(colour.to_owned()),
        description: None,
    }
}

fn issue(number: u64, title: &str, author: &str) -> Issue {
    Issue {
        repo: "octo/tools".to_owned(),
        number,
        title: title.to_owned(),
        state: IssueState::Open,
        author: user(author),
        assignees: Vec::new(),
        labels: Vec::new(),
        milestone: None,
        created: TWO_DAYS_AGO.to_owned(),
        updated: HOUR_AGO.to_owned(),
        comments: 0,
        web_url: format!("https://github.com/octo/tools/issues/{number}"),
    }
}

/// The open issues of the GitHub remote.
fn github_issues() -> Vec<Issue> {
    let mut lexer = issue(12, "The lexer drops quotes inside a needle", "bob");
    lexer.labels = vec![label("bug", "d73a4a"), label("lexer", "5319e7")];
    lexer.assignees = vec![user("ada")];
    lexer.comments = 2;
    lexer.milestone = Some(Milestone {
        id: "3".to_owned(),
        title: "0.3".to_owned(),
        due: Some("2020-02-01T00:00:00Z".to_owned()),
        open: true,
    });
    let mut dates = issue(9, "Dates in the log are a day off", "cy");
    dates.labels = vec![label("enhancement", "a2eeef")];
    let docs = issue(7, "A landing page for the docs", "octo");
    vec![lexer, dates, docs]
}

fn listing(items: Vec<Issue>) -> Listing<Issue> {
    Listing {
        items,
        more: false,
        freshness: Freshness {
            fetched: Some("2020-01-03T12:00:00Z".to_owned()),
            ..Freshness::default()
        },
    }
}

fn links() -> Vec<RepoLink> {
    vec![
        RepoLink {
            remote: "origin".to_owned(),
            account: "github:github.com:ada".to_owned(),
            kind: ProviderKind::Github,
            owner: "octo".to_owned(),
            name: "tools".to_owned(),
        },
        RepoLink {
            remote: "lab".to_owned(),
            account: "gitlab:127.0.0.1:ada".to_owned(),
            kind: ProviderKind::Gitlab,
            owner: "group".to_owned(),
            name: "tools".to_owned(),
        },
    ]
}

/// What the section is given: two remotes, GitHub's shown.
fn given() -> Given {
    let all = github_issues();
    let mine: Vec<Issue> = all
        .iter()
        .filter(|issue| issue.number == 12)
        .cloned()
        .collect();
    Given {
        links: links(),
        lists: vec![
            (IssueGroup::Open, listing(all)),
            (IssueGroup::Mine, listing(mine)),
        ],
        errors: Vec::new(),
        invite: Vec::new(),
    }
}

/// Everything the view of #12 shows.
fn details(number: u64) -> IssueDetails {
    let mut issue = github_issues()
        .into_iter()
        .find(|issue| issue.number == number)
        .unwrap_or_else(|| issue(number, "An issue a key names", "octo"));
    issue.comments = 2;
    IssueDetails {
        issue,
        body: "Quotes inside a needle are dropped when the lexer reads them.\n\n\
               ## Steps\n\n\
               1. Search for `\"a b\"`\n\
               2. See **nothing** found\n\n\
               See [the guide](https://example.org/guide).\n"
            .to_owned(),
        comments: vec![
            Comment {
                id: "c1".to_owned(),
                author: user("ada"),
                body: "I see it too: the quote ends the word.".to_owned(),
                created: DAY_AGO.to_owned(),
                updated: None,
            },
            Comment {
                id: "c2".to_owned(),
                author: user("bob"),
                body: "A branch for it is on the way.".to_owned(),
                created: HOUR_AGO.to_owned(),
                updated: None,
            },
        ],
        freshness: Default::default(),
    }
}

/// Give the section of the shown tab what it would have read.
fn give(stage: &mut Stage, given: Given) {
    stage.with_workspace(|workspace, _, cx| {
        let issues = workspace.read(cx).issues();
        issues.update(cx, |view, cx| view.give(given, cx));
    });
    stage.settle();
}

fn probe(stage: &mut Stage) -> IssuesProbe {
    stage.with_workspace(|workspace, _, cx| workspace.read(cx).issues().read(cx).probe())
}

fn view_probe(stage: &mut Stage) -> Option<IssueViewProbe> {
    stage.with_workspace(|workspace, _, cx| {
        workspace
            .read(cx)
            .issue_view()
            .map(|view| view.read(cx).probe())
    })
}

fn give_details(stage: &mut Stage, number: u64) {
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace.read(cx).issue_view().expect("an issue is shown");
        view.update(cx, |view, cx| view.give(details(number), cx));
    });
    stage.settle();
}

/// The sections of the references panel on show, with where each heading
/// is and how high its list is.
fn sections(stage: &mut Stage) -> Vec<(&'static str, Point<Pixels>, Option<f32>)> {
    stage.with_workspace(|workspace, _, cx| workspace.read(cx).refs().read(cx).sections_shown())
}

/// Open or close a section with a click on its heading.
fn toggle(stage: &mut Stage, section: &str) {
    let (_, head, _) = sections(stage)
        .into_iter()
        .find(|(key, _, _)| *key == section)
        .expect("the section is on show");
    assert!(head.y > px(0.), "the heading of {section} was drawn");
    stage.click_at((f32::from(head.x), f32::from(head.y)));
    stage.settle();
}

/// A repository with a branch made for issue 12, its issues given, and
/// their section open.
fn open_section(pictures: bool) -> Stage {
    repos::fresh();
    let repo = repos::project_with_everything();
    repos::git(&repo, &["branch", "12-lexer-quotes", "main~1"]);
    let start = Start::with(&repo);
    let mut stage = Stage::open(if pictures { start.pictures() } else { start });
    give(&mut stage, given());
    toggle(&mut stage, "issues");
    stage
}

fn row(group: &str, number: u64) -> ElementId {
    ElementId::Name(format!("issue:{group}:{number}").into())
}

/// Answer the provider commands the issues ask as `answer` says; every
/// other command runs for real.
fn stand_in(answer: impl Fn(&str, &Value) -> Option<Result<Value, String>> + 'static) {
    calls::stand_in(answer);
}

/// The section is there while a remote is linked to an account, after
/// the pull requests; it says how many issues are open, and its groups
/// close and open from their headings and stay as they were left.
fn the_issues_section_shows_with_a_linked_remote_and_its_groups_close() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    assert!(
        !sections(&mut stage)
            .iter()
            .any(|(key, _, _)| *key == "issues"),
        "its remote is a folder on this machine: nothing to link"
    );
    give(&mut stage, given());
    let keys: Vec<&str> = sections(&mut stage)
        .iter()
        .map(|(key, _, _)| *key)
        .collect();
    assert_eq!(
        keys,
        [
            "local",
            "remote",
            "worktrees",
            "stashes",
            "issues",
            "tags",
            "submodules"
        ]
    );
    assert!(!probe(&mut stage).shown, "closed, as every section starts");
    toggle(&mut stage, "issues");
    let seen = probe(&mut stage);
    assert!(seen.shown && seen.listed, "{seen:?}");
    assert_eq!(seen.repository.as_deref(), Some("octo/tools"));
    assert_eq!(seen.repositories, ["octo/tools", "group/tools"]);
    assert_eq!(
        seen.groups,
        [("open".to_owned(), 3, true), ("mine".to_owned(), 1, true)]
    );
    assert_eq!(
        seen.rows,
        [
            "#12 The lexer drops quotes inside a needle",
            "#9 Dates in the log are a day off",
            "#7 A landing page for the docs",
            "#12 The lexer drops quotes inside a needle",
        ]
    );
    assert!(stage.shows("issues-repository"), "two remotes: a choice");
    stage.click(ElementId::Name("issues-group:open".into()));
    let seen = probe(&mut stage);
    assert_eq!(seen.groups[0], ("open".to_owned(), 3, false));
    assert!(!stage.shows(row("open", 9)));
    assert!(stage.shows(row("mine", 12)));
    let closed = stage.app(|cx| cx.global::<Prefs>().repo_state(&repo).closed_issue_groups);
    assert_eq!(closed, ["open"]);
    stage.click(ElementId::Name("issues-group:open".into()));
    assert!(probe(&mut stage).groups[0].2);
    // Closing the section tells its rows they are out of view.
    toggle(&mut stage, "issues");
    assert!(!probe(&mut stage).shown);
}

/// What is typed in the search box keeps the issues whose title, number,
/// labels or author has it, in both groups, and the counts say so.
fn the_issues_search_box_keeps_the_issues_that_match() {
    let mut stage = open_section(false);
    stage.click("issues-search");
    stage.type_text("lexer");
    let seen = probe(&mut stage);
    assert_eq!(seen.search, "lexer");
    assert_eq!(
        seen.groups,
        [("open".to_owned(), 1, true), ("mine".to_owned(), 1, true)]
    );
    stage.keys("ctrl-a backspace");
    stage.type_text("#9");
    assert_eq!(
        probe(&mut stage).rows,
        ["#9 Dates in the log are a day off"]
    );
    stage.keys("ctrl-a backspace");
    stage.type_text("enhancement");
    assert_eq!(probe(&mut stage).rows.len(), 1);
    stage.keys("ctrl-a backspace");
    stage.type_text("nothing like it");
    let seen = probe(&mut stage);
    assert!(seen.rows.is_empty());
    assert!(
        seen.notes.is_empty(),
        "an empty group shows nothing under it: {:?}",
        seen.notes
    );
}

/// With remotes linked, the lists are asked of `issue.list` with the
/// chosen repository's link and each group's filter; a query typed in
/// the field the filter button opens asks again with it, and is kept with
/// the repository; another repository is chosen from the list above.
fn the_issues_query_asks_the_provider_and_a_repository_is_chosen() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    stand_in(|id, params| {
        (id == "issue.list").then(|| {
            let mut items = github_issues();
            if params["filter"] == "assigned" {
                items.retain(|issue| issue.number == 12);
            }
            if params["query"] == "label:bug" {
                items.retain(|issue| issue.labels.iter().any(|label| label.name == "bug"));
            }
            if params["owner"] == "group" {
                items.truncate(1);
                items
                    .iter_mut()
                    .for_each(|issue| issue.repo = "group/tools".to_owned());
            }
            Ok(serde_json::to_value(listing(items)).unwrap())
        })
    });
    stage.with_workspace(|workspace, _, cx| {
        let issues = workspace.read(cx).issues();
        issues.update(cx, |view, cx| view.link_to(links(), cx));
    });
    toggle(&mut stage, "issues");
    stage.until("the lists are read", |stage| {
        stage.settle();
        let seen = probe(stage);
        !seen.reading && seen.rows.len() == 4
    });
    let asked: Vec<(String, String, String)> = calls::asked()
        .into_iter()
        .filter(|(id, _)| id == "issue.list")
        .map(|(_, params)| {
            (
                params["account"].as_str().unwrap_or("").to_owned(),
                format!(
                    "{}/{}",
                    params["owner"].as_str().unwrap_or(""),
                    params["name"].as_str().unwrap_or("")
                ),
                params["filter"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect();
    assert_eq!(
        asked,
        [
            (
                "github:github.com:ada".to_owned(),
                "octo/tools".to_owned(),
                "all".to_owned()
            ),
            (
                "github:github.com:ada".to_owned(),
                "octo/tools".to_owned(),
                "assigned".to_owned()
            ),
        ]
    );

    // The provider's query.
    stage.click("issues-filter");
    assert!(probe(&mut stage).query_open);
    stage.click("issues-query");
    stage.type_text("label:bug");
    stage.keys("enter");
    stage.until("asked with the query", |stage| {
        stage.settle();
        let seen = probe(stage);
        !seen.reading && seen.groups[0].1 == 1
    });
    assert_eq!(probe(&mut stage).query, "label:bug");
    assert!(
        calls::asked()
            .iter()
            .any(|(id, params)| id == "issue.list" && params["query"] == "label:bug")
    );
    let kept = stage.app(|cx| cx.global::<Prefs>().repo_state(&repo).issue_query);
    assert_eq!(kept.as_deref(), Some("label:bug"));
    // The button again: no query.
    stage.click("issues-filter");
    stage.until("asked without it", |stage| {
        stage.settle();
        let seen = probe(stage);
        !seen.reading && seen.groups[0].1 == 3
    });
    assert!(!probe(&mut stage).query_open);
    // The button beside the field asks as Enter does.
    stage.click("issues-filter");
    stage.click("issues-query");
    stage.type_text("label:bug");
    stage.click("issues-query-go");
    stage.until("asked with the query again", |stage| {
        stage.settle();
        let seen = probe(stage);
        !seen.reading && seen.query == "label:bug" && seen.groups[0].1 == 1
    });
    stage.click("issues-filter");
    stage.until("asked without it again", |stage| {
        stage.settle();
        let seen = probe(stage);
        !seen.reading && seen.groups[0].1 == 3
    });

    // The other repository.
    stage.with_workspace(|workspace, _, cx| {
        let issues = workspace.read(cx).issues();
        issues.update(cx, |view, cx| view.choose("lab", cx));
    });
    stage.until("its lists are read", |stage| {
        stage.settle();
        let seen = probe(stage);
        !seen.reading && seen.groups[0].1 == 1
    });
    assert_eq!(probe(&mut stage).repository.as_deref(), Some("group/tools"));
    let chosen = stage.app(|cx| cx.global::<Prefs>().repo_state(&repo).issue_repo);
    assert_eq!(chosen.as_deref(), Some("lab"));
    calls::stand_down();
}

/// A click on a row shows the issue in the middle, with the branches
/// made for it; the close button brings the graph back.
fn an_issue_opens_in_the_middle_with_its_branches_and_goes_back() {
    let mut stage = open_section(false);
    assert_eq!(stage.workspace().sheet, None);
    stage.click(row("open", 12));
    stage.settle();
    assert_eq!(stage.workspace().sheet, Some("issue"));
    give_details(&mut stage, 12);
    let seen = view_probe(&mut stage).expect("the view is there");
    assert_eq!(seen.number, 12);
    assert_eq!(seen.repository, "octo/tools");
    assert_eq!(
        seen.title.as_deref(),
        Some("The lexer drops quotes inside a needle")
    );
    assert_eq!(seen.state.as_deref(), Some("Open"));
    assert_eq!(seen.comments, 2);
    assert_eq!(seen.branches, ["12-lexer-quotes"]);
    assert!(stage.shows("issue-title") && stage.shows("issue-description"));
    assert_eq!(seen.offline, None);
    assert!(!stage.shows("issue-offline"));
    // Kept from an hour ago, its provider away: the heading says so.
    let mut kept = details(12);
    kept.freshness.offline = true;
    kept.freshness.fetched = Some("2020-01-03T11:00:00Z".to_owned());
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace.read(cx).issue_view().expect("an issue is shown");
        view.update(cx, |view, cx| view.give(kept, cx));
    });
    stage.settle();
    stage.frame();
    let seen = view_probe(&mut stage).expect("the view is there");
    assert_eq!(seen.offline.as_deref(), Some("Offline: as read 1 h ago"));
    assert!(stage.shows("issue-offline"));
    give_details(&mut stage, 12);
    // The view takes the right panel's place too: it reaches the window's
    // right edge.
    let (x, _, w, _) = stage.place("issue-view");
    let window_width = stage.window(|window, _| window.viewport_size().width);
    assert!(
        (x + w - f32::from(window_width)).abs() < 1.,
        "the view ends at {} in a window {} wide",
        x + w,
        f32::from(window_width)
    );
    stage.click("issue-close");
    assert_eq!(stage.workspace().sheet, None);
    assert!(view_probe(&mut stage).is_none());
    // Opened again from its number, as the keyboard would.
    let opened = stage.with_workspace(|workspace, _, cx| {
        let issues = workspace.read(cx).issues();
        issues.update(cx, |view, cx| view.open_number(9, cx))
    });
    assert!(opened);
    stage.settle();
    assert_eq!(view_probe(&mut stage).map(|seen| seen.number), Some(9));
}

/// A row's menu asks in the bar for the name of a branch for the issue,
/// made by the template of the preferences; going ahead makes it and
/// checks it out with `issue.branch`, which undo can take back; a name
/// there is already is said, and asked again.
fn a_branch_for_an_issue_is_named_in_the_bar_and_made() {
    let mut stage = open_section(false);
    let repo = stage.with_workspace(|workspace, _, cx| workspace.read(cx).session().path);
    let ask = |stage: &mut Stage, number: u64| {
        let asked = stage.with_workspace(|workspace, _, cx| {
            let issues = workspace.read(cx).issues();
            issues.update(cx, |view, cx| view.branch_for(number, cx))
        });
        assert!(asked);
        stage.settle();
    };
    ask(&mut stage, 12);
    let (said, fields) = stage.shell().bar.expect("the bar asks");
    assert_eq!(said, "Branch for #12");
    assert_eq!(fields, ["12-the-lexer-drops-quotes-inside-a-needle"]);
    stage.click(("ask-choice", 0usize));
    stage.until("the branch is checked out", |stage| {
        stage.settle();
        repos::git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).trim()
            == "12-the-lexer-drops-quotes-inside-a-needle"
    });
    // Undo would take it back: the journal the repository keeps says so.
    let mut ctx = bitbraid_engine::CommandContext::default();
    let registry = bitbraid_integrations::registry();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": repo }))
        .unwrap();
    let peek = registry.run("undo.peek", &mut ctx, &json!({})).unwrap();
    assert_eq!(
        peek["undo"],
        "Create branch 12-the-lexer-drops-quotes-inside-a-needle"
    );

    // The template of the preferences, and a name there is already.
    stage.app(|cx| {
        cx.global_mut::<Prefs>().preferences.issue_branch = "fix/{number}".to_owned();
    });
    ask(&mut stage, 9);
    let (_, fields) = stage.shell().bar.expect("the bar asks");
    assert_eq!(fields, ["fix/9"]);
    stage.keys("ctrl-a backspace");
    stage.type_text("12-lexer-quotes");
    stage.click(("ask-choice", 1usize));
    stage.settle();
    assert!(
        stage
            .notices()
            .iter()
            .any(|notice| notice.contains("There is a branch 12-lexer-quotes")),
        "{:?}",
        stage.notices()
    );
    let (_, fields) = stage.shell().bar.expect("asked again");
    assert_eq!(fields, ["12-lexer-quotes"]);
    stage.click("ask-cancel");
    // The view's button asks too. (The issue is opened as a click on its
    // row would: with the repository's list and the search box above the
    // rows, #7 is under the section's fold here.)
    let opened = stage.with_workspace(|workspace, _, cx| {
        let issues = workspace.read(cx).issues();
        issues.update(cx, |view, cx| view.open_number(7, cx))
    });
    assert!(opened);
    stage.settle();
    give_details(&mut stage, 7);
    stage.click("issue-create-branch");
    let (said, _) = stage.shell().bar.expect("the bar asks");
    assert_eq!(said, "Branch for #7");
    stage.click("ask-cancel");
}

/// An issue key in a commit's message is a link of the linked repository:
/// the pointer on it shows the issue's card, read once with `issue.show`
/// and kept; a click opens the issue's view. A branch whose name holds an
/// issue shows that issue's card on its label.
fn issue_keys_in_messages_and_branch_names_show_their_issue() {
    repos::fresh();
    let repo = repos::project_with_everything();
    repos::git(
        &repo,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "#7 a second look at the docs",
        ],
    );
    repos::git(&repo, &["branch", "12-lexer-quotes", "main~1"]);
    let mut stage = Stage::open(Start::with(&repo));
    stand_in(|id, params| {
        (id == "issue.show").then(|| {
            let number = params["number"].as_u64().unwrap_or(0);
            Ok(serde_json::to_value(details(number)).unwrap())
        })
    });
    give(&mut stage, given());
    select(&mut stage, "#7 a second look at the docs");
    let drawn = stage.app(|cx| bitbraid_ui::providers::drawn_links(cx).len());
    assert_eq!(drawn, 2, "the details draw keys as links of the remotes");
    // The pointer on the key: its card is read.
    let (x, y, _, h) = stage.place("commit-summary");
    stage.point_at((x + 6., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    stage.until("the issue is read for its card", |stage| {
        stage.settle();
        known(stage, 7).is_some_and(|known| matches!(known, Known::Read(_)))
    });
    let asked = calls::asked();
    assert!(
        asked.iter().any(|(id, params)| id == "issue.show"
            && params["owner"] == "octo"
            && params["name"] == "tools"
            && params["number"] == 7),
        "{asked:?}"
    );
    // A click opens it.
    stage.click_in("commit-summary", (6., h / 2.));
    stage.settle();
    assert_eq!(stage.workspace().sheet, Some("issue"));
    assert_eq!(view_probe(&mut stage).map(|seen| seen.number), Some(7));
    stage.click("issue-close");

    // The label of the branch made for #12.
    let row = (1..40)
        .find(|row| {
            stage
                .graph_labels(*row)
                .iter()
                .any(|name| name == "12-lexer-quotes")
        })
        .expect("the branch has a label");
    let spot = stage.graph_spot(row, true);
    stage.point_at(spot);
    stage.pass(Duration::from_secs(2));
    stage.until("the issue of the branch is read", |stage| {
        stage.settle();
        known(stage, 12).is_some()
    });
    calls::stand_down();
}

/// A row's card has the issue's description: the list has none, so the
/// issue is read once for its card (`issue.show`) and kept; the next time
/// the card needs no reading.
fn an_issues_card_reads_it_once_for_its_description() {
    let mut stage = open_section(false);
    stand_in(|id, params| {
        (id == "issue.show").then(|| {
            let number = params["number"].as_u64().unwrap_or(0);
            Ok(serde_json::to_value(details(number)).unwrap())
        })
    });
    let before = calls::asked().len();
    let (x, y, w, h) = stage.place(row("open", 12));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    assert!(stage.shows("hover-card"));
    stage.until("the issue is read for its card", |stage| {
        stage.settle();
        known(stage, 12).is_some_and(|known| matches!(known, Known::Read(_)))
    });
    let read = || {
        calls::asked()
            .into_iter()
            .skip(before)
            .filter(|(id, params)| id == "issue.show" && params["number"] == 12)
            .count()
    };
    assert_eq!(read(), 1, "read once for the card");
    stage.point_at((x + w + 700., y));
    stage.pass(Duration::from_secs(1));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    assert!(stage.shows("hover-card"));
    assert_eq!(read(), 1, "kept: not read again");
    calls::stand_down();
}

/// Select the commit of this summary in the graph, and wait for its
/// message in the details.
fn select(stage: &mut Stage, summary: &str) {
    let found = (0..40).find(|row| {
        let spot = stage.graph_spot(*row, false);
        stage.click_at(spot);
        stage.settle();
        stage.graph().selected_summary.as_deref() == Some(summary)
    });
    assert!(found.is_some(), "{summary} is in the graph");
    stage.until("the commit's message is shown", |stage| {
        stage.shows("commit-summary")
    });
}

/// What the cards know of an issue of `octo/tools` at the GitHub account.
fn known(stage: &mut Stage, number: u64) -> Option<Known> {
    let link = links().remove(0);
    stage.app(|cx| {
        cx.try_global::<card::Kept>()
            .and_then(|kept| kept.get(&card::issue_ref(&link, number)).cloned())
    })
}

/// On a branch whose name holds an issue, the commit box's description
/// starts as `Refs: #12`; on another branch, or with the preference off,
/// it starts empty.
fn the_commit_box_names_the_issue_of_the_branch() {
    let description = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| {
            workspace
                .read(cx)
                .commit_box()
                .read(cx)
                .description()
                .to_owned()
        })
    };
    repos::fresh();
    let repo = repos::project_with_changes();
    repos::git(&repo, &["checkout", "-q", "-b", "12-fix-quotes"]);
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the trailer is in", |stage| {
        stage.settle();
        description(stage) == "Refs: #12"
    });
    let number = stage
        .with_workspace(|workspace, _, cx| workspace.read(cx).commit_box().read(cx).issue_number());
    assert_eq!(number, Some(12));
    drop(stage);

    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.settle();
    stage.pass(Duration::from_millis(300));
    stage.settle();
    assert_eq!(description(&mut stage), "", "main names no issue");
    drop(stage);

    repos::fresh();
    let repo = repos::project_with_changes();
    repos::git(&repo, &["checkout", "-q", "-b", "fix-12"]);
    let mut stage = Stage::open(Start::with(&repo).preferences("issue_trailer = false\n"));
    stage.until("the issue is known", |stage| {
        stage.settle();
        stage.with_workspace(|workspace, _, cx| {
            workspace.read(cx).commit_box().read(cx).issue_number()
        }) == Some(12)
    });
    stage.settle();
    assert_eq!(description(&mut stage), "", "the preference is off");
    // Turned on with the tab open, the empty description takes the
    // trailer at once; off again, the trailer it put there goes.
    let wanted = |stage: &mut Stage, on: bool| {
        stage.app(|cx| {
            cx.global_mut::<bitbraid_ui::prefs::Prefs>()
                .preferences
                .issue_trailer = on;
        });
        stage.frame();
    };
    wanted(&mut stage, true);
    stage.until("the trailer follows the preference", |stage| {
        stage.settle();
        description(stage) == "Refs: #12"
    });
    wanted(&mut stage, false);
    stage.until("and goes with it", |stage| {
        stage.settle();
        description(stage).is_empty()
    });
}

/// What a provider answers is another machine's: titles with control
/// characters and no end, numbers at their largest, labels of no colour,
/// a description that nests without end, comments by nobody. All of it
/// is drawn, opened and closed without harm.
fn hostile_issues_are_shown_without_harm() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let mut odd = issue(u64::MAX, &"\u{202e}long\n\u{0}".repeat(5000), "\t");
    odd.labels = vec![
        label("x", "zzzzzz"),
        label(&"y".repeat(10_000), "#"),
        Label {
            name: String::new(),
            color: None,
            description: None,
        },
    ];
    odd.assignees = (0..1000).map(|n| user(&format!("{n}\u{7}"))).collect();
    odd.created = "9999-99-99T99:99:99Z".to_owned();
    odd.updated = String::new();
    odd.web_url = "javascript:alert(1)".to_owned();
    odd.milestone = Some(Milestone {
        id: String::new(),
        title: "\n".repeat(100),
        due: Some("-1".to_owned()),
        open: false,
    });
    let mut given = given();
    given.links.truncate(1);
    given.lists = vec![(IssueGroup::Open, listing(vec![odd.clone()]))];
    given.errors = vec![(IssueGroup::Mine, "\u{0}".repeat(100_000))];
    give(&mut stage, given);
    toggle(&mut stage, "issues");
    let seen = probe(&mut stage);
    assert_eq!(seen.rows.len(), 1, "{seen:?}");
    assert_eq!(seen.notes.len(), 1, "{seen:?}");
    let (x, y, w, h) = stage.place(row("open", u64::MAX));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    stage.click(row("open", u64::MAX));
    stage.settle();
    let details = IssueDetails {
        issue: odd,
        body: format!("{}{}", ">".repeat(100_000), "[".repeat(50_000)),
        comments: (0..500)
            .map(|n| Comment {
                id: String::new(),
                author: user(""),
                body: "`".repeat(n * 10),
                created: "yesterday".to_owned(),
                updated: None,
            })
            .collect(),
        freshness: Default::default(),
    };
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace.read(cx).issue_view().expect("an issue is shown");
        view.update(cx, |view, cx| view.give(details, cx));
    });
    stage.settle();
    stage.frame();
    let seen = view_probe(&mut stage).unwrap();
    assert_eq!(seen.comments, 500);
    // Its button is far down the right column, under a thousand people.
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace.read(cx).issue_view().expect("an issue is shown");
        view.update(cx, |view, cx| view.ask_branch(cx));
    });
    stage.settle();
    let (_, fields) = stage.shell().bar.expect("the bar asks");
    assert!(
        fields
            .first()
            .is_some_and(|name| name.starts_with("18446744073709551615")),
        "{fields:?}"
    );
    stage.click("ask-cancel");
    stage.click("issue-close");
    assert_eq!(stage.workspace().sheet, None);
}

/// The whole way, with no network: the window's provider commands answer
/// from what the integrations crate's live tests recorded, at a GitLab of
/// one's own and at GitHub; the groups hold what `issue.list` answers for
/// each filter, and a click on an issue shows what `issue.show` read.
fn recorded_issues_of_a_gitlab_and_of_github_fill_the_section_and_a_view() {
    // The real commands answer, from the recordings.
    calls::stand_down();
    for (kind, web, remote, file, number, owner, name) in [
        (
            "gitlab",
            "http://127.0.0.1:8929",
            "http://127.0.0.1:8929/bb-p-group/bb-p-project.git",
            "gitlab/issues.json",
            1u64,
            "bb-p-group",
            "bb-p-project",
        ),
        (
            "github",
            "https://github.com",
            "https://github.com/user-1/bitbraid-test-core.git",
            "github/issues.json",
            7,
            "user-1",
            "bitbraid-test-core",
        ),
    ] {
        let (mut stage, account) = recorded(kind, web, remote, file);
        let seen = probe(&mut stage);
        assert_eq!(
            seen.repository.as_deref(),
            Some(format!("{owner}/{name}").as_str())
        );
        let config = crate::hermetic::inside().unwrap().join("config/bitbraid");
        let by_command: Vec<(String, usize, bool)> = [("open", "all"), ("mine", "assigned")]
            .into_iter()
            .map(|(key, filter)| {
                let listing = bitbraid_integrations::registry()
                    .run(
                        "issue.list",
                        &mut context(&config),
                        &json!({ "account": account, "owner": owner, "name": name, "filter": filter }),
                    )
                    .unwrap();
                let count = listing["items"].as_array().map_or(0, Vec::len);
                (key.to_owned(), count, true)
            })
            .collect();
        assert_eq!(seen.groups, by_command, "{kind}: {seen:?}");
        assert!(seen.groups[0].1 > 0, "{kind}: {seen:?}");
        let opened = stage.with_workspace(|workspace, _, cx| {
            let issues = workspace.read(cx).issues();
            issues.update(cx, |view, cx| view.open_number(number, cx))
        });
        assert!(opened, "{kind}");
        stage.until("the issue is read", |stage| {
            stage.settle();
            view_probe(stage).is_some_and(|seen| !seen.loading)
        });
        let seen = view_probe(&mut stage).unwrap();
        assert_eq!(seen.error, None, "{kind}");
        assert_eq!(
            seen.title.as_deref(),
            Some("The button does nothing"),
            "{kind}"
        );
        assert_eq!(seen.comments, 2, "{kind}");
        stage.click("issue-close");
    }
}

/// A context of commands for the window's configuration directory.
fn context(config: &std::path::Path) -> bitbraid_engine::CommandContext {
    bitbraid_engine::CommandContext {
        config_dir: Some(config.to_path_buf()),
        ..Default::default()
    }
}

/// A repository whose `origin` is `remote`, opened in a window whose
/// provider commands answer from the recording `file`, with a keychain in
/// memory and an account connected at `web` by a stand-in token. Gives the
/// window once the section has read its lists, and the account's id.
fn recorded(kind: &str, web: &str, remote: &str, file: &str) -> (Stage, String) {
    use std::sync::Arc;

    use bitbraid_integrations::{
        replay::{Recording, Replay},
        secrets::MemoryStore,
        services::Services,
    };

    repos::fresh();
    let repo = repos::project();
    repos::git(&repo, &["remote", "add", "origin", remote]);
    let recordings =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../integrations/tests/recordings");
    let text = std::fs::read_to_string(recordings.join(file)).unwrap();
    let recording: Recording = serde_json::from_str(&text).unwrap();
    let config = crate::hermetic::inside().unwrap().join("config/bitbraid");
    Services::install(
        &config,
        Arc::new(Services::new(
            Arc::new(Replay::new(recording)),
            Arc::new(MemoryStore::new()),
            &config,
        )),
    );
    let mut stage = Stage::open(Start::with(&repo));
    let account = bitbraid_integrations::registry()
        .run(
            "account.add_token",
            &mut context(&config),
            &json!({ "kind": kind, "web": web, "token": "stand-in-token" }),
        )
        .unwrap();
    let account = account["id"].as_str().unwrap().to_owned();
    stage.with_workspace(|workspace, _, cx| {
        let issues = workspace.read(cx).issues();
        issues.update(cx, |view, cx| view.refresh(cx));
    });
    stage.until("the section is listed", |stage| {
        stage.settle();
        probe(stage).listed
    });
    toggle(&mut stage, "issues");
    stage.until("the recorded lists are in", |stage| {
        stage.settle();
        let seen = probe(stage);
        !seen.reading && seen.notes.iter().all(|note| note != "Reading…")
    });
    (stage, account)
}

/// The pictures: the section, a row's card, the field of the provider's
/// query, an issue's view, the bar that names a branch for it, a key in
/// a commit's message with its card, and the commit box with the trailer.
pub fn issues(shots: &mut Shots) {
    let mut stage = open_section(true);
    shots.take(&mut stage, "issues_section", Themes::Two);

    // The card of a row, while the pointer rests on it, with the
    // description it reads.
    stand_in(|id, params| {
        (id == "issue.show").then(|| {
            let number = params["number"].as_u64().unwrap_or(0);
            Ok(serde_json::to_value(details(number)).unwrap())
        })
    });
    let (x, y, w, h) = stage.place(row("open", 12));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    stage.until("the issue is read for its card", |stage| {
        stage.settle();
        known(stage, 12).is_some_and(|known| matches!(known, Known::Read(_)))
    });
    stage.settle();
    shots.take(&mut stage, "issues_card", Themes::Two);
    calls::stand_down();
    stage.point_at((x + w + 400., y));

    stage.click("issues-filter");
    stage.click("issues-query");
    stage.type_text("label:bug");
    shots.take(&mut stage, "issues_query", Themes::Two);
    stage.click("issues-filter");
    stage.settle();

    stage.click(row("open", 12));
    stage.settle();
    give_details(&mut stage, 12);
    shots.take(&mut stage, "issue_view", Themes::Two);
    stage.click("issue-create-branch");
    stage.settle();
    shots.take(&mut stage, "issue_branch", Themes::Two);
    stage.click("ask-cancel");
    stage.click("issue-close");
    stage.settle();
    drop(stage);

    // A key in a commit's message, its card under the pointer.
    repos::fresh();
    let repo = repos::project_with_everything();
    repos::git(
        &repo,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "#12 count quotes in the lexer\n\nThe needle is found inside quotes now.\n\nRefs: #12",
        ],
    );
    let mut stage = Stage::open(Start::with(&repo).pictures());
    stand_in(|id, params| {
        (id == "issue.show").then(|| {
            let number = params["number"].as_u64().unwrap_or(0);
            Ok(serde_json::to_value(details(number)).unwrap())
        })
    });
    give(&mut stage, given());
    select(&mut stage, "#12 count quotes in the lexer");
    let (x, y, _, h) = stage.place("commit-summary");
    stage.point_at((x + 6., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    stage.until("the card is read", |stage| {
        stage.settle();
        known(stage, 12).is_some_and(|known| matches!(known, Known::Read(_)))
    });
    stage.frame();
    shots.take(&mut stage, "commit_issue_key", Themes::Two);
    calls::stand_down();
    drop(stage);

    // The commit box on a branch made for #12.
    repos::fresh();
    let repo = repos::project_with_changes();
    repos::git(&repo, &["checkout", "-q", "-b", "12-fix-quotes"]);
    let mut stage = Stage::open(Start::with(&repo).pictures());
    stage.until("the trailer is in", |stage| {
        stage.settle();
        stage.with_workspace(|workspace, _, cx| {
            workspace
                .read(cx)
                .commit_box()
                .read(cx)
                .description()
                .to_owned()
        }) == "Refs: #12"
    });
    shots.take(&mut stage, "commit_issue_trailer", Themes::Two);
}
