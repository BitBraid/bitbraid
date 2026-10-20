//! What the window does, as `behaviour.rs` asks it, for the right panel: a
//! commit's details, the work in progress and the commit form.

use bitbraid_ui::views::{
    commit_box::FormTab,
    rewrite::{Rewrite, RunRewrite},
};
use gpui_kit::{ScrollDelta, point, px, test::TestWindowExt as _};

use crate::{
    repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "a_list_of_changes_folds_to_its_heading_and_the_keys_go_past_it",
        run: a_list_of_changes_folds_to_its_heading_and_the_keys_go_past_it,
    },
    Test {
        name: "unstaging_a_moved_file_takes_back_the_whole_move",
        run: unstaging_a_moved_file_takes_back_the_whole_move,
    },
    Test {
        name: "discarding_everything_and_giving_a_merge_up_are_asked_in_the_bar",
        run: discarding_everything_and_giving_a_merge_up_are_asked_in_the_bar,
    },
    Test {
        name: "a_tree_of_changes_starts_closed_and_opens_all_from_its_first_row",
        run: a_tree_of_changes_starts_closed_and_opens_all_from_its_first_row,
    },
    Test {
        name: "a_commit_s_details_lead_back_to_the_uncommitted_work",
        run: a_commit_s_details_lead_back_to_the_uncommitted_work,
    },
    Test {
        name: "a_second_click_on_the_file_that_is_shown_closes_it",
        run: a_second_click_on_the_file_that_is_shown_closes_it,
    },
    Test {
        name: "several_commits_after_a_comparison_with_the_working_tree_say_what_they_are",
        run: several_commits_after_a_comparison_with_the_working_tree_say_what_they_are,
    },
    Test {
        name: "discarding_a_staged_file_from_its_menu_takes_it_back_to_the_last_commit",
        run: discarding_a_staged_file_from_its_menu_takes_it_back_to_the_last_commit,
    },
    Test {
        name: "a_stopped_merge_names_its_sides_and_resolves_all_from_its_heading",
        run: a_stopped_merge_names_its_sides_and_resolves_all_from_its_heading,
    },
    Test {
        name: "the_right_panel_is_there_while_a_row_is_selected",
        run: the_right_panel_is_there_while_a_row_is_selected,
    },
    Test {
        name: "the_side_panels_go_as_far_as_they_go_and_the_graph_stays",
        run: the_side_panels_go_as_far_as_they_go_and_the_graph_stays,
    },
    Test {
        name: "rewording_in_a_plan_opens_a_form_under_the_row",
        run: rewording_in_a_plan_opens_a_form_under_the_row,
    },
    Test {
        name: "the_stash_tab_stashes_what_is_staged_under_its_title",
        run: the_stash_tab_stashes_what_is_staged_under_its_title,
    },
    Test {
        name: "amending_shows_the_last_message_and_unticking_gives_back_what_was_typed",
        run: amending_shows_the_last_message_and_unticking_gives_back_what_was_typed,
    },
    Test {
        name: "the_message_is_dragged_taller_until_the_lists_have_their_least",
        run: the_message_is_dragged_taller_until_the_lists_have_their_least,
    },
];

/// The two lists of changes are each a heading and the rows under it. A
/// click on the heading folds the list to it: its files are out of sight
/// and out of the keys' way, and the other list is what the keys walk.
fn a_list_of_changes_folds_to_its_heading_and_the_keys_go_past_it() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let first_file = |stage: &mut Stage| {
        stage.focus_changes();
        stage.keys("home");
        stage.workspace().file.map(|(path, _)| path)
    };
    assert_eq!(first_file(&mut stage).as_deref(), Some("assets/logo.png"));
    assert!(stage.shows("unstaged-rows"));

    stage.click("heading-unstaged");
    assert!(!stage.shows("unstaged-rows"), "the unstaged list is folded");
    assert!(stage.shows("stage-all"), "its heading keeps its button");
    assert_eq!(
        first_file(&mut stage).as_deref(),
        Some("docs/guide.md"),
        "the keys begin in the staged list"
    );

    // Its button acts on the whole list all the same.
    stage.click("stage-all");
    stage.until("everything is staged", |_| {
        repos::git(&repo, &["status", "--porcelain"])
            .lines()
            .all(|line| !line.starts_with(' ') && !line.starts_with("??"))
    });

    stage.click("heading-unstaged");
    assert!(stage.shows("unstaged-rows"), "open again");
    // Nothing is left to stage: the heading has no button.
    assert!(!stage.shows("stage-all"));
}

/// What is staged, as git says it: the first column of its short status.
fn staged_lines(repo: &std::path::Path) -> Vec<String> {
    repos::git(repo, &["status", "--porcelain"])
        .lines()
        .filter(|line| !line.starts_with(' ') && !line.starts_with('?'))
        .map(str::to_owned)
        .collect()
}

/// A file moved and staged is one row, the old path an arrow before the
/// new. Unstaging it takes back the whole move, not only the new path,
/// which would leave the old one's deletion staged.
fn unstaging_a_moved_file_takes_back_the_whole_move() {
    repos::fresh();
    let repo = repos::project();
    let file = repos::git(&repo, &["ls-files", "src"])
        .lines()
        .next()
        .map(str::to_owned)
        .expect("the project has sources");
    repos::git(&repo, &["mv", &file, "moved.rs"]);
    assert_eq!(staged_lines(&repo), [format!("R  {file} -> moved.rs")]);

    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    stage.focus_changes();
    stage.keys("shift-u");
    stage.until("nothing is staged", |_| staged_lines(&repo).is_empty());
    // The file is where it was moved to, and gone from where it was.
    assert!(repo.join("moved.rs").exists() && !repo.join(&file).exists());
}

/// What is changed and not staged, as git says it.
fn unstaged_lines(repo: &std::path::Path) -> Vec<String> {
    repos::git(repo, &["status", "--porcelain"])
        .lines()
        .filter(|line| line.chars().nth(1).is_some_and(|second| second != ' '))
        .map(str::to_owned)
        .collect()
}

/// Throwing away every unstaged change, and giving a stopped merge up,
/// are asked first in the bar in the toolbar's place; nothing happens
/// until the bar's button that goes ahead is pressed.
fn discarding_everything_and_giving_a_merge_up_are_asked_in_the_bar() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let before = unstaged_lines(&repo);
    assert!(before.len() > 1, "{before:?}");

    stage.click("discard-all");
    let (said, _) = stage.shell().bar.expect("the bar asks");
    assert!(said.contains("unstaged changes"), "{said}");
    stage.click("ask-cancel");
    assert_eq!(stage.shell().bar, None);
    assert_eq!(unstaged_lines(&repo), before, "given up: nothing went");

    stage.click("discard-all");
    stage.click(("ask-choice", 0usize));
    // A submodule that is at another commit stays there: putting a file
    // back does not move what a submodule has checked out.
    stage.until("nothing unstaged is left but the submodule", |_| {
        unstaged_lines(&repo) == [" M vendor/library"]
    });
    assert!(
        !staged_lines(&repo).is_empty(),
        "what is staged stays: {:?}",
        staged_lines(&repo)
    );

    repos::fresh();
    let repo = repos::project_in_conflict();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the merge is seen", |stage| {
        stage.workspace().in_progress.is_some()
    });
    stage.click("op-abort");
    let (said, _) = stage.shell().bar.expect("the bar asks");
    assert!(said.contains("merge"), "{said}");
    assert!(stage.workspace().in_progress.is_some(), "not yet");
    stage.click(("ask-choice", 0usize));
    stage.until("the merge is given up", |stage| {
        stage.workspace().in_progress.is_none()
    });
}

/// A repository that opened on its uncommitted work, with the keys in the
/// commit form's summary.
fn at_the_summary(repo: &std::path::Path) -> Stage {
    let mut stage = Stage::open(Start::with(repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    // Asked to commit without a summary, the form takes the keys.
    stage.keys("ctrl-enter");
    stage
}

/// What the commit form's button and summary say, and which tab is on.
fn form(stage: &mut Stage) -> ((String, bool), (String, bool), String, FormTab) {
    stage.with_workspace(|workspace, _, cx| {
        let form = workspace.read(cx).commit_box();
        let form = form.read(cx);
        (
            form.button(),
            form.stash_button(),
            form.summary().to_owned(),
            form.tab(),
        )
    })
}

/// The second tab over the form sets what is staged aside, under the
/// title typed in it; its button says how many files that is.
fn the_stash_tab_stashes_what_is_staged_under_its_title() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = at_the_summary(&repo);
    stage.type_text("guide: set aside");
    stage.click("tab-stash");
    let (_, stash, summary, tab) = form(&mut stage);
    assert_eq!(tab, FormTab::Stash);
    assert_eq!(summary, "guide: set aside", "the title is the summary");
    assert_eq!(stash, ("Stash 1 file".to_owned(), true));

    stage.click("stash-staged");
    stage.until("the stash is made", |_| {
        repos::git(&repo, &["stash", "list"]).contains("guide: set aside")
    });
    stage.until("nothing is staged", |_| staged_lines(&repo).is_empty());
    stage.until("the form is empty again", |stage| form(stage).2.is_empty());
    // What was not staged is where it was.
    assert!(!unstaged_lines(&repo).is_empty());
    assert_eq!(
        form(&mut stage).1,
        ("Stage a file to stash".to_owned(), false)
    );

    stage.click("tab-commit");
    assert_eq!(form(&mut stage).3, FormTab::Commit);
}

/// Ticking amend puts the last commit's message in the form, whatever was
/// typed; unticking gives back what was typed.
fn amending_shows_the_last_message_and_unticking_gives_back_what_was_typed() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let last = repos::git(&repo, &["log", "-1", "--format=%s"])
        .trim()
        .to_owned();
    let mut stage = at_the_summary(&repo);
    stage.type_text("my own summary");
    stage.click("amend");
    stage.until("the last message is in the form", |stage| {
        form(stage).2 == last
    });
    assert_eq!(form(&mut stage).0, ("Amend".to_owned(), true));
    stage.click("amend");
    stage.until("what was typed is back", |stage| {
        form(stage).2 == "my own summary"
    });
    assert_eq!(form(&mut stage).0, ("Commit 1 file".to_owned(), true));

    // The options are folded under the message until asked for.
    assert!(!stage.shows("push-after"));
    stage.click("commit-options-fold");
    assert!(stage.shows("push-after"));
}

/// The grip over the form makes the message taller and the lists shorter,
/// down to their least: a heading and two rows each.
fn the_message_is_dragged_taller_until_the_lists_have_their_least() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = at_the_summary(&repo);
    let (_, _, _, before) = stage.place("commit-message");
    let (_, button_top, _, _) = stage.place("commit");
    let grip = |stage: &mut Stage| {
        let (x, y, width, _) = stage.place("form-grip");
        (x + width / 2., y + 2.)
    };
    // As far up as it goes: the message is taller, the lists keep their
    // least (the window of the tests is 720 high: they have little more
    // to give), and the button is where it was.
    let from = grip(&mut stage);
    stage.drag_at(from, (from.0, 0.));
    for list in ["unstaged-rows", "staged-rows"] {
        let (_, _, _, height) = stage.place(list);
        assert!(height >= 64., "{list}: {height}");
    }
    let (_, _, _, tallest) = stage.place("commit-message");
    assert!(tallest > before, "{before} -> {tallest}");
    // Down again by less than it went up: shorter by that much.
    let from = grip(&mut stage);
    let back = ((tallest - before) / 2.).floor();
    stage.drag_at(from, (from.0, from.1 + back));
    let (_, _, _, height) = stage.place("commit-message");
    assert!(
        (tallest - height - back).abs() < 1.5,
        "{tallest} - {back} -> {height}"
    );
    // And as far down as it goes: its least, which is where it began.
    let from = grip(&mut stage);
    stage.drag_at(from, (from.0, 719.));
    let (_, _, _, height) = stage.place("commit-message");
    assert!((height - before).abs() < 1., "{before} -> {height}");
    assert!((stage.place("commit").1 - button_top).abs() < 1.);
}

/// The tree of changed files starts with its folders closed, files of the
/// top folder after them; its first row opens every folder. The order is
/// by path without regard to case, and turned round by the sort button.
fn a_tree_of_changes_starts_closed_and_opens_all_from_its_first_row() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let first_file = |stage: &mut Stage| {
        stage.focus_changes();
        stage.keys("home");
        stage.workspace().file.map(|(path, _)| path)
    };
    stage.click("layout-tree");
    // Closed: assets/, src/ and vendor/ first, then grammar.rs.
    assert_eq!(first_file(&mut stage).as_deref(), Some("grammar.rs"));
    // The first row of the unstaged list opens every folder. The keys
    // scrolled the list to the file; back to its top first.
    stage.window(|window, cx| {
        window.scroll(
            "unstaged-rows",
            ScrollDelta::Pixels(point(px(0.), px(1000.))),
            cx,
        )
    });
    stage.frame();
    let (x, y, _, _) = stage.place("unstaged-rows");
    stage.click_at((x + 40., y + 15.));
    assert_eq!(first_file(&mut stage).as_deref(), Some("assets/logo.png"));

    // Z to A: vendor/ is the last folder to come first.
    stage.click("changes-sort");
    stage.focus_changes();
    stage.keys("home");
    assert_eq!(
        stage.workspace().file.map(|(path, _)| path).as_deref(),
        Some("vendor/library")
    );
    stage.click("changes-sort");
    stage.click("layout-path");
    assert_eq!(first_file(&mut stage).as_deref(), Some("assets/logo.png"));
}

/// While the working tree has changes, a commit's details have a strip
/// over them whose button selects the row of uncommitted work again.
fn a_commit_s_details_lead_back_to_the_uncommitted_work() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    stage.keys("ctrl-home down");
    stage.until("a commit is selected", |stage| !stage.graph().wip_selected);
    assert!(!stage.workspace().changes_shown);
    stage.click("view-changes");
    stage.until("the uncommitted work is selected again", |stage| {
        stage.graph().wip_selected
    });
    assert!(stage.workspace().changes_shown);
}

/// One commit compared with the working tree, then several selected:
/// their details no longer say they are compared with the working tree.
fn several_commits_after_a_comparison_with_the_working_tree_say_what_they_are() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let first = stage.graph_spot(1, false);
    stage.click_at(first);
    stage.until("a commit's details are shown", |stage| {
        stage.shows("against-worktree")
    });
    stage.click("against-worktree");
    stage.until("it is compared with the working tree", |stage| {
        stage.workspace().compared_with_worktree
    });
    let second = stage.graph_spot(2, false);
    stage.click_at_with(second, gpui_kit::Modifiers::shift());
    stage.until("two commits are selected", |stage| {
        !stage.shows("against-worktree")
    });
    assert!(
        !stage.workspace().compared_with_worktree,
        "two commits are said to be compared with the working tree"
    );
}

/// A click on a file of the lists of changes, or of a commit, shows it in
/// the middle; a second click on the same row closes it again, and the
/// row is no longer marked.
fn a_second_click_on_the_file_that_is_shown_closes_it() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let (x, y, _, _) = stage.place("unstaged-rows");
    let first = (x + 80., y + 16.);
    stage.click_at(first);
    assert_eq!(
        stage.workspace().file.map(|(path, _)| path).as_deref(),
        Some("assets/logo.png")
    );
    stage.click_at(first);
    assert_eq!(stage.workspace().file, None, "closed by the second click");
    let keys =
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).changes().read(cx).keys_on());
    assert_eq!(keys, None, "and no file is marked");

    // Two files selected: the middle goes back to the graph.
    stage.click_at(first);
    assert!(stage.workspace().file.is_some());
    let second = (first.0, first.1 + 32.);
    stage.click_at_with(second, gpui_kit::Modifiers::secondary_key());
    assert_eq!(stage.workspace().file, None, "two are selected");

    // The same in a commit's files.
    let commit = stage.graph_spot(1, false);
    stage.click_at(commit);
    stage.until("a commit is selected", |stage| !stage.graph().wip_selected);
    stage.until("its files are listed", |stage| stage.shows("commit-files"));
    let (x, y, _, _) = stage.place("commit-files");
    let first = (x + 80., y + 16.);
    stage.click_at(first);
    let shown = stage.workspace().file;
    assert!(
        shown.as_ref().is_some_and(|(_, rev)| rev.is_some()),
        "{shown:?}"
    );
    stage.click_at(first);
    assert_eq!(stage.workspace().file, None);
}

/// A staged file's menu has "Discard changes" second, as an unstaged
/// file's has: it takes the file out of the index and throws its changes
/// away, so that it is as the last commit has it.
fn discarding_a_staged_file_from_its_menu_takes_it_back_to_the_last_commit() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let status = |repo: &std::path::Path| {
        repos::git(repo, &["status", "--porcelain", "--", "docs/guide.md"])
    };
    assert!(!status(&repo).is_empty(), "the guide is changed and staged");
    stage.focus_changes();
    // The last file is the staged one; its menu, then the second entry.
    stage.keys("end shift-f10");
    stage.until("its menu is open", |stage| {
        stage.with_workspace(|workspace, _, cx| {
            workspace.read(cx).changes().read(cx).key_menu_open()
        })
    });
    stage.keys("down down enter");
    stage.until("the guide is as the last commit has it", |_| {
        status(&repo).is_empty()
    });
}

/// While a merge is stopped, the line under the heading names the branch
/// that comes in and the one it comes into, and the heading of the files
/// in conflict has a button that marks them all resolved as they are.
fn a_stopped_merge_names_its_sides_and_resolves_all_from_its_heading() {
    repos::fresh();
    let repo = repos::project_in_conflict();
    // The conflict is settled in the file; git has not been told.
    std::fs::write(
        repo.join("docs/guide.md"),
        "Search for a needle, settled.\n",
    )
    .expect("the file is written");
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the merge and its conflict are seen", |stage| {
        stage.workspace().in_progress == Some(("merge".to_owned(), 1))
    });
    let names =
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).changes().read(cx).side_names());
    assert_eq!(names, Some(("feature/guide".to_owned(), "main".to_owned())));
    stage.click("resolve-all");
    stage.until("nothing is in conflict", |stage| {
        stage.workspace().in_progress == Some(("merge".to_owned(), 0))
    });
}

/// The right panel is there while a row of the graph is selected: a click
/// on the selected row takes the selection away, and the panel with it;
/// another click brings both back.
fn the_right_panel_is_there_while_a_row_is_selected() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("a commit is selected", |stage| {
        stage.graph().selected.is_some()
    });
    assert!(stage.workspace().right_panel);
    // The first row: selected by the first click unless it was already,
    // and then the click that takes it away.
    let row = stage.graph_spot(0, false);
    stage.click_at(row);
    if stage.graph().selected.is_some() {
        stage.click_at(row);
    }
    stage.until("nothing is selected", |stage| {
        stage.graph().selected.is_none()
    });
    assert!(!stage.workspace().right_panel, "no selection, no panel");
    stage.click_at(row);
    stage.until("the row is selected again", |stage| {
        stage.graph().selected.is_some()
    });
    assert!(stage.workspace().right_panel);
}

/// In the plan of an interactive rebase, rewording a commit opens a small
/// form under its row for the new message: giving it up puts the commit
/// back as it was, taking it keeps the message for when the plan is
/// started.
fn rewording_in_a_plan_opens_a_form_under_the_row() {
    repos::fresh();
    let repo = repos::project_with_files();
    let first = repos::rev(&repo, "HEAD~3");
    let summary = repos::git(&repo, &["log", "-1", "--format=%s"])
        .trim()
        .to_owned();
    let mut stage = Stage::open(Start::with(&repo));
    assert!(stage.workspace().loaded);
    stage.act(RunRewrite {
        rewrite: Rewrite::Interactive {
            first,
            reword: false,
        },
    });
    assert!(stage.workspace().planning);
    assert!(!stage.shows("reword-form"));

    // The newest commit is the selected one: R rewords it.
    stage.keys("r");
    assert!(stage.shows("reword-form"), "the form is under the row");
    stage.type_text(" again");
    stage.click("reword-cancel");
    assert!(!stage.shows("reword-form"));

    stage.keys("r");
    stage.type_text(" again");
    stage.click("reword-update");
    assert!(!stage.shows("reword-form"));
    stage.click("plan-start");
    stage.until("the commit has its new message", |_| {
        repos::git(&repo, &["log", "-1", "--format=%s"]).trim() == format!("{summary} again")
    });
}

/// The two side panels are dragged as far as they go, in the window of the
/// tests (1280 wide): the left one up to 793 beside the right one as it
/// starts out, the right one up to 593 beside the left one at its least.
/// Both wide, the panel being dragged stops where the graph between them
/// still has 87.
fn the_side_panels_go_as_far_as_they_go_and_the_graph_stays() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("a commit is selected", |stage| {
        stage.graph().selected.is_some()
    });
    assert!(stage.workspace().right_panel);
    // Where each panel ends: the left one's edge stands over its right
    // end, the right one's over its left end.
    let ends = |stage: &mut Stage| {
        let (left, ..) = stage.place(("split", 0usize));
        let (right, ..) = stage.place(("split", 1usize));
        (left + 3., right + 2.)
    };
    let drag = |stage: &mut Stage, side: usize, to: f32| {
        let (x, y, width, height) = stage.place(("split", side));
        stage.drag_at((x + width / 2., y + height / 2.), (to, y + height / 2.));
    };
    assert_eq!(ends(&mut stage), (208., 880.));

    // The left one to the window's right end: 793, the right one as it
    // was, the middle 87.
    drag(&mut stage, 0, 1270.);
    assert_eq!(ends(&mut stage), (793., 880.));
    // The right one cannot take any of that.
    drag(&mut stage, 1, 10.);
    assert_eq!(ends(&mut stage), (793., 880.));

    // The left one back to its least, then the right one as wide as it
    // goes: 593 with its rule, the middle 479.
    drag(&mut stage, 0, 10.);
    drag(&mut stage, 1, 10.);
    assert_eq!(ends(&mut stage), (208., 687.));
    // The left one now stops where the middle keeps 87.
    drag(&mut stage, 0, 1270.);
    assert_eq!(ends(&mut stage), (600., 687.));
    // And the right one narrow again: as narrow as 353 with its rule.
    drag(&mut stage, 1, 1279.);
    assert_eq!(ends(&mut stage), (600., 927.));
}
