//! What the window does, as `behaviour.rs` asks it, for the commit graph.

use bitbraid_ui::views::{
    rewrite::{Rewrite, RunRewrite},
    stashes::{RunStash, StashRequest},
    tags::{RunTag, TagRequest},
};

use crate::{
    repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "a_commits_labels_are_one_per_name_and_unfold_under_the_pointer",
        run: a_commits_labels_are_one_per_name_and_unfold_under_the_pointer,
    },
    Test {
        name: "a_new_branch_or_tag_is_named_on_its_commits_row",
        run: a_new_branch_or_tag_is_named_on_its_commits_row,
    },
    Test {
        name: "a_commit_moves_a_stash_is_renamed_and_a_tag_annotated_from_the_graph",
        run: a_commit_moves_a_stash_is_renamed_and_a_tag_annotated_from_the_graph,
    },
    Test {
        name: "dropping_a_commit_and_a_hard_reset_are_asked_in_the_bar",
        run: dropping_a_commit_and_a_hard_reset_are_asked_in_the_bar,
    },
    Test {
        name: "the_labels_a_drag_could_land_on_are_outlined_while_it_lasts",
        run: the_labels_a_drag_could_land_on_are_outlined_while_it_lasts,
    },
    Test {
        name: "the_row_of_uncommitted_work_shows_the_summary_being_written",
        run: the_row_of_uncommitted_work_shows_the_summary_being_written,
    },
    Test {
        name: "a_column_is_moved_by_dragging_its_heading_onto_another",
        run: a_column_is_moved_by_dragging_its_heading_onto_another,
    },
    Test {
        name: "a_columns_border_is_taken_on_either_side_of_its_line",
        run: a_columns_border_is_taken_on_either_side_of_its_line,
    },
    Test {
        name: "a_label_is_as_wide_as_its_column_allows",
        run: a_label_is_as_wide_as_its_column_allows,
    },
    Test {
        name: "a_branch_is_lit_once_the_pointer_rests_on_its_label_and_the_rest_fades",
        run: a_branch_is_lit_once_the_pointer_rests_on_its_label_and_the_rest_fades,
    },
    Test {
        name: "a_cut_label_widens_to_its_whole_name_while_the_pointer_rests_on_it",
        run: a_cut_label_widens_to_its_whole_name_while_the_pointer_rests_on_it,
    },
    Test {
        name: "a_label_in_the_unfolded_column_opens_its_own_menu",
        run: a_label_in_the_unfolded_column_opens_its_own_menu,
    },
    Test {
        name: "a_commits_circle_names_its_author_under_the_pointer",
        run: a_commits_circle_names_its_author_under_the_pointer,
    },
    Test {
        name: "a_commits_whole_message_shows_over_its_row_with_its_lines",
        run: a_commits_whole_message_shows_over_its_row_with_its_lines,
    },
    Test {
        name: "enter_and_shift_enter_step_through_what_a_search_found",
        run: enter_and_shift_enter_step_through_what_a_search_found,
    },
    Test {
        name: "the_search_bar_leaves_the_first_row_in_sight",
        run: the_search_bar_leaves_the_first_row_in_sight,
    },
    Test {
        name: "the_headings_follow_the_rows_when_the_window_is_made_wider",
        run: the_headings_follow_the_rows_when_the_window_is_made_wider,
    },
    Test {
        name: "the_graph_comes_back_where_it_was_scrolled_when_the_window_is_resized_at_the_start",
        run: the_graph_comes_back_where_it_was_scrolled_when_the_window_is_resized_at_the_start,
    },
    Test {
        name: "only_the_rows_in_view_of_a_long_history_are_built",
        run: only_the_rows_in_view_of_a_long_history_are_built,
    },
    Test {
        name: "a_commit_without_labels_shows_its_nearest_branch_and_two_clicks_check_it_out",
        run: a_commit_without_labels_shows_its_nearest_branch_and_two_clicks_check_it_out,
    },
];

/// A history of 5,000 commits: the rows that are built are the ones in
/// view, at the top and after a jump to the last row, never the
/// thousands between.
fn only_the_rows_in_view_of_a_long_history_are_built() {
    use bitbraid_testkit::CorpusSpec;
    let spec = CorpusSpec {
        commits: 5_000,
        lanes: 4,
        merge_every: 25,
        files: 50,
    };
    let repo = bitbraid_testkit::build_synthetic(&spec).unwrap();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the history is read", |stage| {
        stage.graph().commits >= 5_000
    });
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    let built = |stage: &mut Stage| -> Vec<usize> {
        (0..rows).filter(|row| stage.shows(("row", *row))).collect()
    };
    let top = built(&mut stage);
    assert!(
        !top.is_empty() && top.len() < 100 && top.iter().all(|row| *row < 100),
        "built at the top: {top:?}"
    );
    stage.act(bitbraid_ui::keymap::SelectLast);
    stage.frame();
    let end = built(&mut stage);
    assert!(
        !end.is_empty() && end.len() < 100 && end.iter().all(|row| *row + 100 > rows),
        "built at the end of {rows}: {end:?}"
    );
}

/// The pointer on a commit without labels shows, faintly, the nearest
/// branch that has it, and two clicks on that check the branch out;
/// the commit is then in the branch checked out.
fn a_commit_without_labels_shows_its_nearest_branch_and_two_clicks_check_it_out() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    let head = |repo: &std::path::Path| {
        repos::git(repo, &["symbolic-ref", "--short", "HEAD"])
            .trim()
            .to_owned()
    };
    assert_eq!(head(&repo), "main");
    let mut checked_out = None;
    for row in 0..rows {
        if !stage.graph_labels(row).is_empty() {
            continue;
        }
        // Selected, so that its commit is known, and with the pointer on
        // it, so that the faint label shows.
        let spot = stage.graph_spot(row, false);
        stage.click_at(spot);
        let Some(commit) = stage.graph().selected else {
            continue;
        };
        if !stage.shows(("ghost", row)) {
            continue;
        }
        let (left, top, width, height) = stage.place(("ghost", row));
        stage.clicks_at(
            (left + width / 2.0, top + height / 2.0),
            2,
            gpui_kit::Modifiers::default(),
        );
        stage.settle();
        let now = head(&repo);
        if now != "main" {
            checked_out = Some((commit, now));
            break;
        }
    }
    let (commit, branch) = checked_out.expect("a faint label checked a branch out");
    stage.until("the checkout is seen", |stage| {
        stage.settle();
        stage.workspace().loaded
    });
    // Fails, and says so, when the branch does not have the commit.
    repos::git(&repo, &["merge-base", "--is-ancestor", &commit, &branch]);
}

/// A branch that is here and on a remote at the same commit is one label,
/// called by its own name; a commit with more than one label shows the
/// first and folds the others, and the pointer on them unfolds them all
/// until it goes.
fn a_commits_labels_are_one_per_name_and_unfold_under_the_pointer() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    let labels: Vec<Vec<String>> = (0..rows).map(|row| stage.graph_labels(row)).collect();
    // The remote has every branch, and most of them where they are here:
    // no label is the remote's own, and no name is twice on a commit.
    for names in &labels {
        assert!(
            names.iter().all(|name| !name.starts_with("origin/")),
            "{names:?}"
        );
        let mut seen = names.clone();
        seen.dedup();
        assert_eq!(&seen, names, "a name is one label");
    }
    assert!(labels.iter().any(|names| names.iter().any(|n| n == "wip")));

    // The commit the second worktree has checked out has a tag as well.
    let Some(row) = labels.iter().position(|names| names.len() > 1) else {
        panic!("no commit with two labels: {labels:?}");
    };
    assert_eq!(stage.graph().unfolded, None);
    let on = stage.graph_spot(row, true);
    stage.point_at(on);
    assert_eq!(
        stage.graph().unfolded,
        Some(row),
        "the pointer unfolds them"
    );
    // Away from them, onto the message of another row: folded again.
    let away = stage.graph_spot(row.saturating_sub(1).max(1), false);
    stage.point_at(away);
    assert_eq!(stage.graph().unfolded, None);
}

/// A new branch is named in a field that stands in the label column of
/// its commit's row, and is made and checked out with Enter; Escape puts
/// the field away and makes nothing. A tag is named the same way; an
/// annotated one is asked its message next, in the bar.
fn a_new_branch_or_tag_is_named_on_its_commits_row() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let branches = stage.workspace().branches;
    let head = repos::git(&repo, &["rev-parse", "HEAD"]);

    // From where the toolbar's button asks: at HEAD, the first row.
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |w, cx| w.prompt_new_branch(None, window, cx))
    });
    stage.frame();
    assert_eq!(stage.graph().naming, Some((0, "branch")));
    assert!(!stage.dialog_open() && !stage.workspace().prompting);
    // Given up: nothing is made.
    stage.keys("escape");
    assert_eq!(stage.graph().naming, None);
    assert_eq!(stage.workspace().branches, branches);

    // Again, and typed: made at HEAD and checked out.
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |w, cx| w.prompt_new_branch(None, window, cx))
    });
    stage.frame();
    stage.type_text("named-here");
    stage.keys("enter");
    stage.until("the branch is made", |stage| {
        stage.workspace().branches == branches + 1
    });
    assert_eq!(stage.graph().naming, None);
    assert_eq!(
        repos::git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).trim(),
        "named-here"
    );
    assert_eq!(
        repos::git(&repo, &["rev-parse", "named-here"]).trim(),
        head.trim()
    );

    // A tag on the third row, from the row's menu.
    let third = stage.graph_spot(2, false);
    stage.click_at(third);
    let at = stage.graph().selected.expect("the row is selected");
    stage.act(RunTag {
        request: TagRequest::New(at.clone()),
    });
    stage.frame();
    assert_eq!(stage.graph().naming, Some((2, "tag")));
    stage.type_text("v-light");
    stage.keys("enter");
    stage.until("the tag is made", |stage| {
        let _ = stage;
        repos::git(&repo, &["tag", "--list", "v-light"]).trim() == "v-light"
    });
    assert_eq!(repos::git(&repo, &["rev-parse", "v-light^{}"]).trim(), at);
    assert_eq!(
        repos::git(&repo, &["cat-file", "-t", "v-light"]).trim(),
        "commit",
        "a tag without a message of its own"
    );

    // An annotated one: the name in the row, then the message in the bar.
    stage.act(RunTag {
        request: TagRequest::NewAnnotated(at.clone()),
    });
    stage.frame();
    assert_eq!(stage.graph().naming, Some((2, "annotated_tag")));
    stage.type_text("v-told");
    stage.keys("enter");
    stage.frame();
    assert!(stage.shell().bar.is_some(), "the message is asked for");
    stage.type_text("Said about it");
    stage.click(("ask-choice", 0usize));
    stage.until("the annotated tag is made", |_| {
        repos::git(&repo, &["tag", "--list", "v-told"]).trim() == "v-told"
    });
    assert_eq!(
        repos::git(&repo, &["cat-file", "-t", "v-told"]).trim(),
        "tag"
    );
    assert!(repos::git(&repo, &["cat-file", "-p", "v-told"]).contains("Said about it"));
    assert_eq!(
        stage.notices(),
        [
            "Created branch named-here and checked it out",
            "Created tag v-light",
            "Created tag v-told"
        ]
    );
}

/// What the graph's menus ask of the three commands that only it has:
/// a commit moved one place, a stash given another message in the bar,
/// and a lightweight tag given one; each undone and done as git would.
fn a_commit_moves_a_stash_is_renamed_and_a_tag_annotated_from_the_graph() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let subjects = |repo: &std::path::Path| -> Vec<String> {
        repos::git(repo, &["log", "--format=%s", "-3", "HEAD"])
            .lines()
            .map(str::to_owned)
            .collect()
    };
    let before = subjects(&repo);

    // The commit under HEAD, one place up: it is the newest then.
    let second = repos::git(&repo, &["rev-parse", "HEAD~1"])
        .trim()
        .to_owned();
    stage.act(RunRewrite {
        rewrite: Rewrite::Move {
            commits: vec![second],
            up: true,
        },
    });
    stage.until("the two newest change places", |_| {
        let now = subjects(&repo);
        now.len() == 3 && now[0] == before[1] && now[1] == before[0]
    });
    assert_eq!(subjects(&repo)[2], before[2]);

    // A stash, renamed in the bar, keeps its place and its changes.
    let stash = repos::git(&repo, &["rev-parse", "stash@{0}"])
        .trim()
        .to_owned();
    let tree = repos::git(&repo, &["rev-parse", "stash@{0}^{tree}"]);
    stage.act(RunStash {
        request: StashRequest::Reword(stash),
    });
    stage.frame();
    let (said, texts) = stage.shell().bar.expect("the message is asked in the bar");
    assert!(said.contains("stash"), "{said}");
    assert_eq!(texts, ["notes for later"], "it starts from its message");
    stage.keys("ctrl-a");
    stage.type_text("wrap the notes");
    stage.click(("ask-choice", 0usize));
    stage.until("the stash has its new message", |_| {
        repos::git(&repo, &["stash", "list", "--format=%gs"]).trim() == "On main: wrap the notes"
    });
    assert_eq!(repos::git(&repo, &["rev-parse", "stash@{0}^{tree}"]), tree);

    // The lightweight tag, given a message.
    assert_eq!(
        repos::git(&repo, &["cat-file", "-t", "v0.1"]).trim(),
        "commit"
    );
    stage.act(RunTag {
        request: TagRequest::Annotate("v0.1".to_owned()),
    });
    stage.frame();
    assert!(
        stage.shell().bar.is_some(),
        "the message is asked in the bar"
    );
    stage.type_text("the first one");
    stage.click(("ask-choice", 0usize));
    stage.until("the tag has a message", |_| {
        repos::git(&repo, &["cat-file", "-t", "v0.1"]).trim() == "tag"
    });
    assert!(repos::git(&repo, &["cat-file", "-p", "v0.1"]).contains("the first one"));
}

/// What the graph's menus ask before it is done (dropping a commit, a
/// hard reset) is asked in the bar in the toolbar's place, not in a
/// dialog: given up, nothing changes; gone ahead with, it is done.
fn dropping_a_commit_and_a_hard_reset_are_asked_in_the_bar() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let head = repos::git(&repo, &["rev-parse", "HEAD"]);
    let below = repos::git(&repo, &["rev-parse", "HEAD~1"])
        .trim()
        .to_owned();
    let reset = RunRewrite {
        rewrite: Rewrite::Reset {
            target: below.clone(),
            mode: bitbraid_git::ResetMode::Hard,
        },
    };

    stage.act(reset.clone());
    stage.frame();
    let (said, _) = stage.shell().bar.expect("the reset is asked in the bar");
    assert!(said.starts_with("Reset main"), "{said}");
    assert!(!stage.dialog_open());
    stage.click("ask-cancel");
    assert!(stage.shell().bar.is_none());
    assert_eq!(repos::git(&repo, &["rev-parse", "HEAD"]), head);

    // The commit on top dropped: asked, then done.
    let top = head.trim().to_owned();
    stage.act(RunRewrite {
        rewrite: Rewrite::Drop {
            commit: top.clone(),
        },
    });
    stage.frame();
    let (said, _) = stage.shell().bar.expect("the drop is asked in the bar");
    assert!(
        said.starts_with(&format!("Drop commit {}", &top[..7])),
        "{said}"
    );
    assert!(!stage.dialog_open());
    stage.click(("ask-choice", 0usize));
    stage.until("the commit is gone", |_| {
        repos::git(&repo, &["rev-parse", "HEAD"]).trim() == below
    });

    // And the reset goes ahead when asked to.
    let further = repos::git(&repo, &["rev-parse", "HEAD~1"])
        .trim()
        .to_owned();
    stage.act(RunRewrite {
        rewrite: Rewrite::Reset {
            target: further.clone(),
            mode: bitbraid_git::ResetMode::Hard,
        },
    });
    stage.frame();
    assert!(stage.shell().bar.is_some());
    stage.click(("ask-choice", 0usize));
    stage.until("the branch is reset", |_| {
        repos::git(&repo, &["rev-parse", "HEAD"]).trim() == further
    });
}

/// While a branch is dragged over the graph, the labels it could be
/// dropped on are outlined; let go, they are not.
fn the_labels_a_drag_could_land_on_are_outlined_while_it_lasts() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    assert!(!stage.graph().label_drag);
    let (from, to) = (stage.graph_spot(1, true), stage.graph_spot(5, false));
    stage.press_and_move(from, to);
    assert!(stage.graph().label_drag, "the drag is under way");
    stage.release_at(to);
    stage.keys("escape");
    stage.frame();
    assert!(!stage.graph().label_drag, "it ended");
}

/// The field where the row of uncommitted work has its message: a click
/// on it selects the row and puts the cursor in the commit form's summary,
/// and what is typed there is what the field says.
fn the_row_of_uncommitted_work_shows_the_summary_being_written() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is there", |stage| {
        stage.graph().wip_row
    });
    assert_eq!(stage.graph().wip_summary, "");
    // Away from it first, so that the click is what selects it.
    let row = stage.graph_spot(2, false);
    stage.click_at(row);
    assert!(!stage.graph().wip_selected);
    let field = stage.graph_wip_field();
    stage.click_at(field);
    assert!(stage.graph().wip_selected, "the click selects the row");
    stage.type_text("Wrap at the width");
    assert_eq!(stage.graph().wip_summary, "Wrap at the width");
}

/// A column's heading picked up and let go on another heading: the column
/// takes that one's place and the others close up; the order is kept.
fn a_column_is_moved_by_dragging_its_heading_onto_another() {
    use bitbraid_ui::prefs::{Column, Prefs};
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(
        Start::with(&repo).preferences("[columns]\norder = [\"summary\", \"author\", \"id\"]\n"),
    );
    let order =
        |stage: &mut Stage| stage.app(|cx| cx.global::<Prefs>().preferences.columns.order.clone());
    assert_eq!(
        order(&mut stage),
        [Column::Summary, Column::Author, Column::Id]
    );
    let (from, to) = (
        stage.graph_heading(Column::Id),
        stage.graph_heading(Column::Author),
    );
    stage.drag_at(from, to);
    assert_eq!(
        order(&mut stage),
        [Column::Summary, Column::Id, Column::Author]
    );
}

/// A column's border is taken by a strip a few points wide with its
/// middle on the line: a little right of the line as well as a little
/// left of it, the pointer resizes the column the line ends, not the next
/// column's heading, and the line keeps the distance from the pointer it
/// was taken at. A heading's own drag still moves a column
/// (`a_column_is_moved_by_dragging_its_heading_onto_another`), and no
/// strip reaches out of the heading.
fn a_columns_border_is_taken_on_either_side_of_its_line() {
    use bitbraid_ui::prefs::{Column, Prefs};
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(
        Start::with(&repo).preferences("[columns]\norder = [\"summary\", \"author\", \"id\"]\n"),
    );
    // Room for every column as wide as it is set: none gives way.
    stage.resize(1920.0, 1080.0);
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let order =
        |stage: &mut Stage| stage.app(|cx| cx.global::<Prefs>().preferences.columns.order.clone());
    let before = order(&mut stage);
    // The border between the message and the author: its line is the
    // first point of the author's heading.
    let message_line = |stage: &mut Stage| stage.place(("heading", Column::Author as usize)).0;
    let (line, top, _, height) = stage.place(("heading", Column::Author as usize));
    let y = top + height / 2.0;

    // Three points right of the line, dragged forty to the right: the
    // line goes forty to the right, and no column is moved.
    stage.drag_at((line + 3.0, y), (line + 43.0, y));
    assert_eq!(order(&mut stage), before, "no column was moved");
    let moved = message_line(&mut stage) - line;
    assert!(
        (moved - 40.0).abs() < 0.5,
        "taken right of its line, the message's border followed the pointer by {moved}"
    );
    // Three and a half points left of it, thirty to the left.
    let line = message_line(&mut stage);
    stage.drag_at((line - 3.5, y), (line - 33.5, y));
    let moved = message_line(&mut stage) - line;
    assert!(
        (moved + 30.0).abs() < 0.5,
        "taken left of its line, it followed by {moved}"
    );

    // The border of the branch and tag column, right of its line: on the
    // graph column's heading.
    let refs_line = |stage: &mut Stage| stage.place("graph-header").0;
    let line = refs_line(&mut stage);
    stage.drag_at((line + 3.0, y), (line + 23.0, y));
    let moved = refs_line(&mut stage) - line;
    assert!(
        (moved - 20.0).abs() < 0.5,
        "the label column's border followed by {moved}"
    );

    // Every strip is a few points wide and lies within the heading, up
    // to the gear at its end; and on its line.
    let (gear, _, gear_width, _) = stage.place("columns-menu");
    let lines = [
        ("refs", stage.place("graph-header").0, 0usize),
        (
            "graph",
            stage.place(("heading", Column::Summary as usize)).0,
            1,
        ),
        (
            "message",
            stage.place(("heading", Column::Author as usize)).0,
            2 + Column::Summary as usize,
        ),
        (
            "author",
            stage.place(("heading", Column::Id as usize)).0,
            2 + Column::Author as usize,
        ),
    ];
    for (what, line, strip) in lines {
        let (left, _, width, _) = stage.place(("resize", strip));
        assert!(
            (8.0..=10.0).contains(&width),
            "the {what} strip is {width} wide"
        );
        assert!(
            left < line - 3.0 && left + width > line + 4.0,
            "the {what} strip, {left} and {width} wide, lies on its line at {line}"
        );
        assert!(
            left >= 0.0 && left + width <= gear + gear_width,
            "the {what} strip is within the heading"
        );
    }
}

/// A row's label is as wide as its name needs, as far as the label column
/// allows: in a column of the width the window starts with, a long name is
/// cut and the label fills what the column leaves; in a column made wider
/// the label grows with it, up to the whole name.
fn a_label_is_as_wide_as_its_column_allows() {
    repos::fresh();
    let repo = repos::project();
    let long = "feature/a-name-long-enough-to-need-a-good-part-of-a-wide-column";
    let root = repos::git(&repo, &["rev-list", "--max-parents=0", "HEAD"]);
    repos::git(&repo, &["branch", long, root.trim()]);
    // How wide the label of that branch is drawn.
    let width = |stage: &mut Stage| -> f32 {
        stage
            .window(|window, _| {
                gpui_kit::base::test_support::snapshots(window)
                    .into_iter()
                    .filter(|found| found.visible() && found.role() == Some(gpui_kit::Role::Button))
                    .find(|found| found.label().is_some_and(|label| label.contains(long)))
                    .map(|found| f32::from(found.bounds().size.width))
            })
            .expect("the branch's label is on show")
    };
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let start = width(&mut stage);
    // The column is 140 wide at the start; 10 before the labels and 6 of
    // line towards the node are not the label's.
    assert!(
        (start - (140.0 - 16.0)).abs() < 1.0,
        "the label fills what the column leaves: {start}"
    );

    let mut stage = Stage::open(Start::with(&repo).preferences("[columns]\nrefs_width = 480.0\n"));
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let wide = width(&mut stage);
    assert!(
        wide > start + 120.0,
        "the label grows with its column: {start} then {wide}"
    );
    assert!(
        wide < 480.0 - 16.0 - 20.0,
        "and is no wider than its name needs: {wide}"
    );
}

/// The pointer on a branch's labels lights the branch's own run of
/// commits once it has rested there a moment (100 ms): a label the pointer
/// only passes over lights nothing. What the other rows say steps back
/// over a moment (100 ms) rather than at once, and comes forward the same
/// way a moment after the pointer left; from one label to the next the
/// rows stay back in between, the light going over to the other run once
/// the pointer rests again. With reduced motion the moment of rest is the
/// same and the rows step back at once.
fn a_branch_is_lit_once_the_pointer_rests_on_its_label_and_the_rest_fades() {
    use std::time::Duration;
    let ms = Duration::from_millis;
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    // The windows of the tests start with reduced motion: this one moves.
    stage.app(|cx| cx.set_reduce_motion(false));
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    // Two rows with labels, the lower one first: its labels unfold over
    // the rows below it, not over the other's.
    let labelled: Vec<usize> = (0..rows)
        .filter(|row| !stage.graph_labels(*row).is_empty())
        .collect();
    let (Some(&upper), Some(&lower)) = (labelled.first(), labelled.get(1)) else {
        panic!("two rows with labels: {labelled:?}");
    };
    let (on_lower, on_upper) = (stage.graph_spot(lower, true), stage.graph_spot(upper, true));
    // Away from every label: on a row's message.
    let away = stage.graph_spot(lower, false);
    let lit = |stage: &mut Stage| stage.graph().lit.first().cloned();
    let fade = |stage: &mut Stage| stage.graph().fade;

    // Passed over: both labels, each for less than the moment.
    stage.point_at(on_lower);
    stage.pass(ms(60));
    assert_eq!(lit(&mut stage), None, "not yet");
    stage.point_at(on_upper);
    stage.pass(ms(60));
    stage.point_at(away);
    stage.pass(ms(300));
    assert_eq!(lit(&mut stage), None, "a pass over labels lights nothing");
    assert_eq!(fade(&mut stage), 100, "and nothing moved");

    // Rested on: lit after the moment, the other rows stepping back.
    stage.point_at(on_lower);
    stage.pass(ms(90));
    assert_eq!(lit(&mut stage), None, "within the moment");
    stage.pass(ms(20));
    let lower_lit = lit(&mut stage).expect("lit after the moment");
    let begun = fade(&mut stage);
    assert!(
        (5..=15).contains(&begun),
        "the rows begin to step back: {begun}"
    );
    stage.pass(ms(50));
    let half = fade(&mut stage);
    assert!((55..=65).contains(&half), "half way: {half}");
    stage.pass(ms(60));
    assert_eq!(fade(&mut stage), 100, "all the way back");

    // To the other label, over a row's message: the rows stay back in
    // between, and the light goes over once the pointer rests again.
    stage.point_at(away);
    stage.pass(ms(40));
    stage.point_at(on_upper);
    stage.pass(ms(40));
    assert_eq!(
        lit(&mut stage).as_ref(),
        Some(&lower_lit),
        "still the first"
    );
    assert_eq!(fade(&mut stage), 100, "no row came forward in between");
    stage.pass(ms(70));
    let upper_lit = lit(&mut stage).expect("lit");
    assert_ne!(upper_lit, lower_lit, "the other branch is lit now");
    assert!(fade(&mut stage) < 100, "the rows go over to it");
    stage.pass(ms(100));
    assert_eq!(fade(&mut stage), 100);

    // Away: lit for the moment, then put out, the rows coming forward.
    stage.point_at(away);
    stage.pass(ms(90));
    assert_eq!(
        lit(&mut stage).as_ref(),
        Some(&upper_lit),
        "within the moment"
    );
    assert_eq!(fade(&mut stage), 100);
    stage.pass(ms(20));
    assert_eq!(lit(&mut stage), None, "put out");
    assert!(fade(&mut stage) < 100, "the rows come forward");
    stage.pass(ms(100));
    assert_eq!(fade(&mut stage), 100);

    // With reduced motion: after the same moment, and at once.
    stage.app(|cx| cx.set_reduce_motion(true));
    stage.point_at(on_lower);
    stage.pass(ms(90));
    assert_eq!(lit(&mut stage), None, "the moment of rest stays");
    stage.pass(ms(20));
    assert_eq!(lit(&mut stage).as_ref(), Some(&lower_lit));
    assert_eq!(fade(&mut stage), 100, "back at once");
    stage.point_at(away);
    stage.pass(ms(110));
    assert_eq!(lit(&mut stage), None);
    assert_eq!(fade(&mut stage), 100, "forward at once");
}

/// Where each label of the graph on show is, by what it is called.
fn buttons_drawn(stage: &mut Stage) -> Vec<(String, [f32; 4])> {
    let mut found: Vec<(String, [f32; 4])> = stage.window(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|found| found.visible() && found.role() == Some(gpui_kit::Role::Button))
            .filter_map(|found| {
                let bounds = found.bounds();
                let said = found.label()?;
                if !(said.starts_with("branch ") || said.starts_with("tag ")) {
                    return None;
                }
                Some((
                    said.to_owned(),
                    [
                        f32::from(bounds.origin.x),
                        f32::from(bounds.origin.y),
                        f32::from(bounds.size.width),
                        f32::from(bounds.size.height),
                    ],
                ))
            })
            .collect()
    });
    found.sort_by(|a, b| a.0.cmp(&b.0).then(a.1[1].total_cmp(&b.1[1])));
    found
}

/// A label whose name is cut where it stands widens to its whole name
/// once the pointer has rested on it as long as the highlight of a branch
/// waits (100 ms), at its own place, over what is right of it: nothing
/// else moves. It stays while the pointer is on it, past the label's own
/// room too, and goes back as soon as the pointer leaves it. A label whose
/// name fits does not change. In the column a row's labels unfold into,
/// the label under the pointer widens the same way, over the rows below,
/// and the column stays unfolded while the pointer is on it. The branch is
/// lit as from the label, and a click on a widened label in its row
/// selects the row.
fn a_cut_label_widens_to_its_whole_name_while_the_pointer_rests_on_it() {
    use std::time::Duration;
    let ms = Duration::from_millis;
    repos::fresh();
    let repo = repos::project();
    // Alone on the first commit, and two on a commit of their own; each
    // too long for the 124 points a label has in the column at its width
    // at the start.
    let long = "feature/a-name-long-enough-to-be-cut-in-its-column";
    let (branch, tag) = (
        "feature/another-name-long-enough-to-be-cut",
        "v9.9-a-tag-whose-name-is-long-enough-to-be-cut",
    );
    let root = repos::git(&repo, &["rev-list", "--max-parents=0", "HEAD"])
        .trim()
        .to_owned();
    repos::git(&repo, &["branch", long, &root]);
    repos::git(&repo, &["branch", branch, ":/Lexer: split on spaces"]);
    repos::git(&repo, &["tag", tag, ":/Lexer: split on spaces"]);
    let mut stage = Stage::open(Start::with(&repo));
    stage.app(|cx| cx.set_reduce_motion(false));
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    let labels: Vec<Vec<String>> = (0..rows).map(|row| stage.graph_labels(row)).collect();
    let row_of = |wanted: &str| {
        labels
            .iter()
            .position(|names| names.first().is_some_and(|name| name == wanted))
            .unwrap_or_else(|| panic!("no row with {wanted} first: {labels:?}"))
    };
    let (lone, pair, short) = (row_of(long), row_of(branch), row_of("wip"));
    assert_eq!(labels[pair].len(), 2, "{labels:?}");
    let widened = |stage: &mut Stage| stage.graph().widened;
    let away = stage.graph_spot(short, false);
    let before = buttons_drawn(&mut stage);
    let label = before
        .iter()
        .find(|(said, _)| said.contains(long))
        .map(|(_, at)| *at)
        .expect("the long label is on show");

    // Rested on: widened after the moment, at its place, nothing moved.
    let on = stage.graph_spot(lone, true);
    stage.point_at(on);
    stage.pass(ms(90));
    assert_eq!(widened(&mut stage), None, "not within the moment");
    stage.pass(ms(20));
    assert_eq!(widened(&mut stage), Some((lone, 0)));
    let (x, y, wide, high) = stage.place(("label-wide", lone));
    assert!(
        (x - label[0]).abs() < 0.5 && (y - label[1]).abs() < 0.5 && (high - label[3]).abs() < 0.5,
        "at the label's place: {:?}, the label {label:?}",
        (x, y, wide, high)
    );
    assert!(
        wide > label[2] + 100.0,
        "as wide as the whole name: {wide}, the label {}",
        label[2]
    );
    assert_eq!(buttons_drawn(&mut stage), before, "nothing else moved");
    // Past the label's own room, on the widened label: it stays, and the
    // branch is lit as from the label.
    let past = (x + wide - 8.0, y + high / 2.0);
    stage.point_at(past);
    stage.pass(ms(300));
    assert_eq!(widened(&mut stage), Some((lone, 0)), "stays");
    assert!(!stage.graph().lit.is_empty(), "the branch is lit");
    // A click on it selects its row.
    stage.click_at(past);
    assert_eq!(stage.graph().selected.as_deref(), Some(root.as_str()));
    // Away: back at once.
    stage.point_at(away);
    assert_eq!(widened(&mut stage), None, "back at once");
    assert_eq!(buttons_drawn(&mut stage), before);

    // A label whose name fits does not change.
    let fits = stage.graph_spot(short, true);
    stage.point_at(fits);
    stage.pass(ms(300));
    assert_eq!(widened(&mut stage), None, "a name that fits");
    stage.point_at(away);

    // In the unfolded column: the label under the pointer, one after the
    // other.
    let on = stage.graph_spot(pair, true);
    stage.point_at(on);
    assert_eq!(stage.graph().unfolded, Some(pair));
    stage.pass(ms(110));
    assert_eq!(widened(&mut stage), Some((pair, 0)));
    let second = (on.0, on.1 + 24.0);
    stage.point_at(second);
    assert_eq!(widened(&mut stage), None, "the first went back at once");
    stage.pass(ms(110));
    assert_eq!(widened(&mut stage), Some((pair, 1)));
    let (x, y, wide, high) = stage.place(("label-wide", pair));
    assert!(
        (y - (on.1 - 11.0 + 24.0)).abs() < 0.5 && wide > 200.0,
        "the second label's place, as wide as its name: {:?}",
        (x, y, wide, high)
    );
    // Past its room, over the rows below: it stays, and so does the
    // column.
    stage.point_at((x + wide - 8.0, y + high / 2.0));
    stage.pass(ms(300));
    let graph = stage.graph();
    assert_eq!(graph.widened, Some((pair, 1)), "{graph:?}");
    assert_eq!(graph.unfolded, Some(pair), "{graph:?}");
    stage.point_at(away);
    let graph = stage.graph();
    assert_eq!((graph.widened, graph.unfolded), (None, None), "{graph:?}");
}

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

/// A right click on a label in the column a row's labels unfold into
/// opens that label's menu, as a right click on the row's own label does;
/// the menu stays while the pointer goes into it, the column unfolded
/// under it, and Escape closes it. A label widened in the column opens
/// its menu the same way.
fn a_label_in_the_unfolded_column_opens_its_own_menu() {
    use std::time::Duration;
    let ms = Duration::from_millis;
    repos::fresh();
    let repo = repos::project();
    // Two branches on a commit of their own, both too long for the column.
    for name in [
        "feature/aa-the-first-of-two-names-long-enough-to-be-cut",
        "feature/zz-the-second-of-two-names-long-enough-to-be-cut",
    ] {
        repos::git(&repo, &["branch", name, ":/Lexer: split on spaces"]);
    }
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    let Some((row, names)) = (0..rows)
        .map(|row| (row, stage.graph_labels(row)))
        .find(|(_, names)| names.iter().any(|name| name.starts_with("feature/aa-")))
    else {
        panic!("no row with the two branches");
    };
    assert_eq!(names.len(), 2, "{names:?}");
    let (shown_first, lower) = (names[0].clone(), names[1].clone());
    let menu_of = |stage: &mut Stage, name: &str, what: &str| {
        let entries = menu_entries(stage);
        assert!(
            entries.iter().any(|entry| entry.contains(name)),
            "{what}: the menu of {name}: {entries:?}"
        );
        entries
    };

    // The lower label of the column: its menu.
    let on = stage.graph_spot(row, true);
    stage.point_at(on);
    assert_eq!(stage.graph().unfolded, Some(row));
    let on_lower = (on.0, on.1 + 24.0);
    stage.point_at(on_lower);
    stage.right_click_at(on_lower);
    stage.pass(ms(300));
    let entries = menu_of(&mut stage, &lower, "the column's lower label");
    assert!(
        !entries.iter().any(|entry| entry.contains(&shown_first)),
        "not the other label's: {entries:?}"
    );
    // Into the menu: it stays, and so does the column under it.
    stage.point_at((on_lower.0 + 60.0, on_lower.1 + 40.0));
    stage.pass(ms(300));
    menu_of(&mut stage, &lower, "with the pointer in the menu");
    assert_eq!(stage.graph().unfolded, Some(row));
    stage.keys("escape");
    stage.pass(ms(300));
    assert!(menu_entries(&mut stage).is_empty(), "closed");

    // The first label, in the column over the row's own: its menu.
    stage.point_at(on);
    stage.right_click_at(on);
    stage.pass(ms(300));
    menu_of(&mut stage, &shown_first, "the column's first label");
    stage.keys("escape");
    stage.pass(ms(300));

    // The lower label widened: its menu, from past its own room.
    stage.point_at(on);
    stage.point_at(on_lower);
    stage.pass(ms(110));
    assert_eq!(stage.graph().widened, Some((row, 1)));
    let (x, y, wide, high) = stage.place(("label-wide", row));
    let past = (x + wide - 8.0, y + high / 2.0);
    stage.point_at(past);
    stage.right_click_at(past);
    stage.pass(ms(300));
    menu_of(&mut stage, &lower, "the widened label");
    assert_eq!(stage.graph().widened, Some((row, 1)), "it stays widened");
    stage.keys("escape");
    stage.pass(ms(300));
    assert!(menu_entries(&mut stage).is_empty(), "closed");
}

/// The row of the list that shows the commit `id`, found by selecting
/// the rows one after the other with a click on their message.
fn row_of(stage: &mut Stage, id: &str) -> usize {
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    (0..rows)
        .find(|row| {
            let spot = stage.graph_spot(*row, false);
            stage.click_at(spot);
            stage.graph().selected.as_deref() == Some(id)
        })
        .expect("the commit is listed")
}

/// What the tooltip of a commit's circle says, when it is on show.
fn circle_tip(stage: &mut Stage) -> Option<String> {
    use gpui_kit::test::TestWindowExt;
    stage.window(|window, _| {
        window
            .try_find("author-tip")
            .filter(|tip| tip.visible())
            .and_then(|tip| tip.label().map(str::to_owned))
    })
}

/// The circle of a commit's author names the author once the pointer has
/// rested on it for the toolkit's moment of a tooltip, and on a line of its
/// own who committed it when that is someone else. It changes nothing else:
/// resting there selects nothing and lights nothing, a click on the circle
/// selects its row as a click anywhere on the row does, the row is dragged
/// from it as from anywhere on it, and the tooltip goes when the pointer
/// leaves.
fn a_commits_circle_names_its_author_under_the_pointer() {
    use std::time::Duration;
    repos::fresh();
    let repo = repos::project();
    let said_by_git = |format: &str, rev: &str| {
        repos::git(&repo, &["log", "-1", &format!("--format={format}"), rev])
            .trim()
            .to_owned()
    };
    let guest = said_by_git("%H", ":/Empty commit by a guest");
    let (author, committer) = (
        said_by_git("%an <%ae>", &guest),
        said_by_git("%cn <%ce>", &guest),
    );
    assert_ne!(author, committer, "the fixture's guest");
    // A commit its author committed (HEAD is a merge: a dot, no circle).
    let own = said_by_git("%H", ":/docs: explain needle search");
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let (row, own_row) = (row_of(&mut stage, &guest), row_of(&mut stage, &own));
    stage.until("the authors are known", |stage| {
        stage.shows(("author-circle", row)) && stage.shows(("author-circle", own_row))
    });
    let middle = |stage: &mut Stage, row: usize| {
        let (x, y, wide, high) = stage.place(("author-circle", row));
        (x + wide / 2.0, y + high / 2.0)
    };

    // The guest's: the author, then who committed it.
    let on = middle(&mut stage, row);
    stage.point_at(on);
    stage.pass(Duration::from_millis(300));
    assert_eq!(circle_tip(&mut stage), None, "not at once");
    stage.pass(Duration::from_millis(400));
    assert_eq!(
        circle_tip(&mut stage),
        Some(format!("{author}\nCommitted by {committer}"))
    );
    let graph = stage.graph();
    assert_eq!(graph.selected.as_deref(), Some(own.as_str()), "{graph:?}");
    assert!(graph.lit.is_empty(), "nothing is lit: {graph:?}");
    // A click on the circle selects its row.
    stage.click_at(on);
    assert_eq!(stage.graph().selected.as_deref(), Some(guest.as_str()));
    // Away: the tooltip goes.
    let away = stage.graph_spot(own_row, false);
    stage.point_at(away);
    stage.pass(Duration::from_millis(700));
    assert_eq!(circle_tip(&mut stage), None, "gone with the pointer");

    // Committed by its author: the author alone.
    let on = middle(&mut stage, own_row);
    stage.point_at(on);
    stage.pass(Duration::from_millis(700));
    assert_eq!(circle_tip(&mut stage), Some(said_by_git("%an <%ae>", &own)));
    // The row is dragged from its circle as from anywhere on it.
    let to = stage.graph_spot(row, false);
    stage.press_and_move(on, to);
    assert!(stage.graph().label_drag, "the drag is under way");
    stage.release_at(to);
    stage.keys("escape");
    stage.frame();
    assert!(!stage.graph().label_drag, "it ended");

    // The author's picture in an author column too narrow for names says
    // the same.
    let narrow = "[columns]\norder = [\"author\", \"summary\"]\nauthor_width = 40.0\n";
    let mut stage = Stage::open(Start::with(&repo).preferences(narrow));
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let row = row_of(&mut stage, &guest);
    stage.until("its author is known", |stage| {
        stage.shows(("author-picture", row))
    });
    let (x, y, wide, high) = stage.place(("author-picture", row));
    stage.point_at((x + wide / 2.0, y + high / 2.0));
    stage.pass(Duration::from_millis(700));
    assert_eq!(
        circle_tip(&mut stage),
        Some(format!("{author}\nCommitted by {committer}"))
    );
}

/// What the tooltip of a row's message says, when it is on show.
fn message_tip(stage: &mut Stage) -> Option<String> {
    use gpui_kit::test::TestWindowExt;
    stage.window(|window, _| {
        window
            .try_find("message-tip")
            .filter(|tip| tip.visible())
            .and_then(|tip| tip.label().map(str::to_owned))
    })
}

/// The pointer resting on a row's message shows the commit's whole
/// message in a tooltip, after the toolkit's moment: the summary, an empty
/// line and the body with its own line breaks (the row shows the body on
/// one line, its lines joined by "|"); a commit without a body, its
/// summary alone. Resting there selects nothing, a click selects the row,
/// and the tooltip goes when the pointer leaves.
fn a_commits_whole_message_shows_over_its_row_with_its_lines() {
    use std::time::Duration;
    repos::fresh();
    let repo = repos::project();
    repos::git(
        &repo,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "Explain the parser",
            "-m",
            "It reads words.\nIt reads quotes, too.",
            "-m",
            "- one\n- two",
        ],
    );
    let explained = repos::git(&repo, &["rev-parse", "HEAD"]).trim().to_owned();
    let bare = repos::git(&repo, &["rev-parse", ":/FIX parser errors"])
        .trim()
        .to_owned();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let (row, bare_row) = (row_of(&mut stage, &explained), row_of(&mut stage, &bare));

    let on = stage.graph_spot(row, false);
    stage.point_at(on);
    stage.pass(Duration::from_millis(300));
    assert_eq!(message_tip(&mut stage), None, "not at once");
    stage.pass(Duration::from_millis(400));
    assert_eq!(
        message_tip(&mut stage).as_deref(),
        Some("Explain the parser\n\nIt reads words.\nIt reads quotes, too.\n\n- one\n- two")
    );
    assert_eq!(
        stage.graph().selected.as_deref(),
        Some(bare.as_str()),
        "resting selects nothing"
    );
    stage.click_at(on);
    assert_eq!(stage.graph().selected.as_deref(), Some(explained.as_str()));
    let away = stage.graph_spot(row, true);
    stage.point_at(away);
    stage.pass(Duration::from_millis(700));
    assert_eq!(message_tip(&mut stage), None, "gone with the pointer");

    // Without a body: the summary alone.
    let on = stage.graph_spot(bare_row, false);
    stage.point_at(on);
    stage.pass(Duration::from_millis(700));
    assert_eq!(
        message_tip(&mut stage).as_deref(),
        Some("FIX parser errors")
    );

    // A label widened across the row's message: the pointer resting on it
    // there shows neither the message's tooltip nor the author's.
    let long = "feature/a-name-long-enough-to-reach-over-the-messages";
    repos::git(&repo, &["branch", long, &bare]);
    stage.until("the new label is on show", |stage| {
        stage.graph_labels(bare_row).first().map(String::as_str) == Some(long)
    });
    let neutral = stage.graph_spot(row, false);
    let rest = |stage: &mut Stage, at: (f32, f32), long: u64| {
        stage.point_at(at);
        stage.pass(Duration::from_millis(long));
    };
    rest(&mut stage, neutral, 700);
    let on = stage.graph_spot(bare_row, true);
    rest(&mut stage, on, 110);
    assert_eq!(stage.graph().widened, Some((bare_row, 0)));
    let (x, y, wide, high) = stage.place(("label-wide", bare_row));
    let over = (x + wide - 8.0, y + high / 2.0);
    // Without the label there, that is the message's place.
    rest(&mut stage, neutral, 700);
    rest(&mut stage, over, 700);
    assert_eq!(stage.graph().widened, None);
    assert_eq!(
        message_tip(&mut stage).as_deref(),
        Some("FIX parser errors"),
        "the widened label reaches over the message"
    );
    rest(&mut stage, neutral, 700);
    rest(&mut stage, on, 110);
    rest(&mut stage, over, 700);
    assert_eq!(stage.graph().widened, Some((bare_row, 0)));
    assert_eq!(message_tip(&mut stage), None, "the message's tooltip rests");
    assert_eq!(circle_tip(&mut stage), None, "the author's tooltip rests");
}

/// In the search field, Enter goes to the next commit it found and
/// Shift+Enter to the one before, round the ends.
fn enter_and_shift_enter_step_through_what_a_search_found() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-f");
    stage.type_text("lexer");
    stage.until("the search has answered", |stage| {
        stage.graph().matches >= 2
    });
    let first = stage.graph().selected;
    assert!(first.is_some(), "the first that was found is selected");
    stage.keys("enter");
    let second = stage.graph().selected;
    assert!(second.is_some() && second != first, "on to the next");
    stage.keys("shift-enter");
    assert_eq!(stage.graph().selected, first, "back to the one before");
    stage.keys("shift-enter");
    assert_ne!(stage.graph().selected, first, "round to the last");
}

/// Made wider, the window lays the headings of the columns out before the
/// list learns its new width in a frame, and the rows after: the headings
/// would stand over other columns' values until something else drew the
/// window again. The graph asks for the next frame itself when the two
/// differ, and in it the headings stand over their values.
fn the_headings_follow_the_rows_when_the_window_is_made_wider() {
    use bitbraid_ui::prefs::Column;
    use gpui_kit::{px, size, test::TestWindowExt as _};
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(
        Start::with(&repo)
            .preferences("[columns]\norder = [\"summary\", \"author\", \"date\", \"id\"]\n"),
    );
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    // Where the heading is drawn, and where the rows have its column, as
    // the probe says it: inside it, 40 from its left or in its middle.
    let heading = |stage: &mut Stage, column: Column| {
        let (left, _, width, _) = stage.place(("heading", column as usize));
        left + width.min(80.0) / 2.0
    };
    let columns = [Column::Author, Column::Date, Column::Id];
    let before: Vec<f32> = columns
        .into_iter()
        .map(|column| stage.graph_heading(column).0)
        .collect();
    for column in columns {
        let (drawn, rows) = (heading(&mut stage, column), stage.graph_heading(column).0);
        assert!(
            (drawn - rows).abs() < 1.0,
            "{column:?} at first: {drawn} {rows}"
        );
    }
    // The window is made wider, and drawn once, as the platform draws it:
    // the heading is behind the rows, and the graph asks for the frame
    // that puts it right (on a small repository nothing else asks for
    // one).
    stage.window(|window, cx| {
        window.resize(size(px(1920.), px(1080.)));
        window.bounds_changed(cx);
        window.render_frame(cx);
    });
    stage.settle();
    let graph = stage.graph();
    assert!(
        graph.heading_behind,
        "the heading was laid out for the old width"
    );
    assert!(graph.heading_asked, "the graph asked for the next frame");
    // The frame after it.
    stage.window(|window, cx| {
        window.simulate_next_frame(cx);
        window.render_frame(cx);
    });
    stage.settle();
    assert!(!stage.graph().heading_behind);
    let mut moved = 0.0f32;
    for (column, rows_were) in columns.into_iter().zip(before) {
        let (drawn, rows) = (heading(&mut stage, column), stage.graph_heading(column).0);
        moved = moved.max((rows - rows_were).abs());
        assert!(
            (drawn - rows).abs() < 1.0,
            "{column:?}: the rows have it at {rows}, its heading is at {drawn}"
        );
    }
    assert!(moved > 10.0, "the columns moved with the width: {moved}");
}

/// The top of the highest row of the graph on show, among the first
/// `rows` of the list.
fn first_row_top(stage: &mut Stage, rows: usize) -> f32 {
    use gpui_kit::test::TestWindowExt as _;
    stage.window(|window, _| {
        (0..rows)
            .filter_map(|ix| window.try_find(("row", ix)))
            .filter(|found| found.visible())
            .map(|found| f32::from(found.bounds().top()))
            .fold(f32::INFINITY, f32::min)
    })
}

/// While the floating search bar is open the rows begin under it, all of
/// them or only the ones found, so that it hides neither the first row nor
/// the first commit found.
fn the_search_bar_leaves_the_first_row_in_sight() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let id = repos::rev(&repo, "HEAD~3");
    stage.keys("ctrl-f");
    stage.type_text(&id[..7]);
    stage.until("the search has answered", |stage| {
        stage.graph().matches == 1
    });
    let (_, bar_top, _, bar_height) = stage.place("graph-search");
    let under = bar_top + bar_height;
    let all = first_row_top(&mut stage, 40);
    assert!(
        all >= under,
        "the first row begins at {all}, under the bar at {under}"
    );
    stage.act(bitbraid_ui::keymap::ToggleOnlyMatches);
    let found = first_row_top(&mut stage, 40);
    assert!(
        found >= under,
        "the commit found begins at {found}, under the bar at {under}"
    );
    // Put away, the rows are where they were.
    stage.keys("escape");
    stage.keys("escape");
    assert!(!stage.graph().search_open);
    assert!(first_row_top(&mut stage, 40) < under);
}

/// The application's state file as it was last written.
fn saved_state() -> String {
    let root = crate::hermetic::inside().expect("the tests run in scratch surroundings");
    std::fs::read_to_string(root.join("config/bitbraid/state.toml")).expect("the state is written")
}

/// What the state file says of where the graph was scrolled to.
fn saved_place(state: &str) -> Vec<String> {
    state
        .lines()
        .filter(|line| line.starts_with("top"))
        .map(str::to_owned)
        .collect()
}

/// Turn the wheel by `dy` pixels with the pointer at a place in the
/// window: down the content for less than nought.
fn wheel_at(stage: &mut Stage, (x, y): (f32, f32), dy: f32) {
    use gpui_kit::{
        InputEvent as _, MouseMoveEvent, ScrollDelta, ScrollWheelEvent, point, px,
        test::TestWindowExt as _,
    };
    let position = point(px(x), px(y));
    stage.window(|window, cx| {
        window.dispatch_event(
            MouseMoveEvent {
                position,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
        window.render_frame(cx);
        window.dispatch_event(
            ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.), px(dy))),
                ..Default::default()
            }
            .to_platform_input(),
            cx,
        );
    });
    stage.frame();
}

/// The application opens its window larger than the window manager then
/// makes it: put back in the first frame, in the larger window, the place
/// the graph was scrolled to would be held to the end the list has there,
/// and a history scrolled to its end would come back seven rows higher.
/// The rows on show come back as they were, at the end of the history and
/// in its middle, between two rows; in a window lower than the one they
/// were seen in, the row at the top comes back; and what is written down
/// of the place stays as it was.
fn the_graph_comes_back_where_it_was_scrolled_when_the_window_is_resized_at_the_start() {
    repos::fresh();
    let project = repos::project();
    // More commits than the window has rows for, so that it scrolls.
    for step in 0..40 {
        repos::git(
            &project,
            &[
                "commit",
                "-q",
                "--allow-empty",
                "-m",
                &format!("step {step}"),
            ],
        );
    }
    let state = format!("open = [{project:?}]\nactive = 0\n");
    for at_the_end in [true, false] {
        let place = if at_the_end { "the end" } else { "the middle" };
        let mut stage = Stage::open(Start::bare().state(state.clone()));
        stage.until("the graph is read", |stage| stage.graph().commits > 50);
        let commits = stage.graph().commits;
        if at_the_end {
            // To the end, and a row near it picked with the pointer,
            // which leaves the list where it is.
            stage.keys("ctrl-end");
            let end = stage.graph_spot(0, false).1;
            let row = stage.graph_spot(commits - 11, false);
            stage.click_at(row);
            assert_eq!(stage.graph_spot(0, false).1, end, "the click scrolled");
        } else {
            // Into the middle, between two rows.
            stage.keys("ctrl-home");
            let over = stage.graph_spot(4, false);
            wheel_at(&mut stage, over, -710.0);
            let row = stage.graph_spot(30, false);
            stage.click_at(row);
        }
        let selected = stage.graph().selected.expect("a commit is selected");
        let first = stage.graph_spot(0, false).1;
        assert!(
            first < -300.0,
            "at {place} the graph is scrolled: its first row at {first}"
        );
        // A moment for the place the graph is scrolled to to be written.
        stage.pass(std::time::Duration::from_secs(2));
        let saved = saved_state();
        assert!(
            !saved_place(&saved).is_empty(),
            "at {place} the place is written: {saved}"
        );
        drop(stage);

        // Opened larger, as the application asks, and made smaller by the
        // window manager after its first frames: to the size it had, and
        // to a lower one.
        for (width, height) in [(1280.0, 720.0), (1280.0, 600.0)] {
            let mut stage = Stage::open(Start::bare().state(saved.clone()).size(1440.0, 920.0));
            stage.until("the graph is read", |stage| stage.graph().commits > 50);
            stage.resize(width, height);
            assert_eq!(
                stage.graph().selected.as_deref(),
                Some(selected.as_str()),
                "at {place}, {width}x{height}: the commit is selected again"
            );
            let again = stage.graph_spot(0, false).1;
            assert!(
                (again - first).abs() < 0.5,
                "at {place}, {width}x{height}: the first row is at {again}, it was at {first}"
            );
            stage.pass(std::time::Duration::from_secs(2));
            assert_eq!(
                saved_place(&saved_state()),
                saved_place(&saved),
                "at {place}, {width}x{height}: the place written down stays"
            );
        }
    }
}
