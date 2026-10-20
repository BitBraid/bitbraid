//! What the window does, as `behaviour.rs` asks it, for what takes the
//! graph's place: a file's diff, history and blame, conflicts and the
//! rebase editor.

use bitbraid_ui::prefs::Prefs;
use gpui_kit::{point, px, test::TestWindowExt as _};

use crate::{
    repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "a_diff_is_shown_as_its_hunks_or_whole_and_the_file_as_it_is",
        run: a_diff_is_shown_as_its_hunks_or_whole_and_the_file_as_it_is,
    },
    Test {
        name: "the_strip_beside_a_diff_maps_the_whole_file_and_is_where_one_scrolls",
        run: the_strip_beside_a_diff_maps_the_whole_file_and_is_where_one_scrolls,
    },
    Test {
        name: "escape_closes_a_files_past_wherever_in_it_the_keys_are",
        run: escape_closes_a_files_past_wherever_in_it_the_keys_are,
    },
    Test {
        name: "a_files_past_opened_by_keys_closes_with_escape_and_its_blame_shows_the_graph",
        run: a_files_past_opened_by_keys_closes_with_escape_and_its_blame_shows_the_graph,
    },
    Test {
        name: "a_files_past_opened_from_the_palette_or_a_menu_closes_with_escape",
        run: a_files_past_opened_from_the_palette_or_a_menu_closes_with_escape,
    },
    Test {
        name: "enter_in_the_finder_opens_the_first_file_of_what_is_typed_even_before_it_is_found",
        run: enter_in_the_finder_opens_the_first_file_of_what_is_typed_even_before_it_is_found,
    },
    Test {
        name: "a_file_edited_and_not_saved_is_asked_about_before_it_closes",
        run: a_file_edited_and_not_saved_is_asked_about_before_it_closes,
    },
    Test {
        name: "the_rows_above_a_file_offer_what_it_can_have_and_escape_closes_it",
        run: the_rows_above_a_file_offer_what_it_can_have_and_escape_closes_it,
    },
    Test {
        name: "lines_are_picked_by_a_click_or_a_drag_and_staged_from_their_menu",
        run: lines_are_picked_by_a_click_or_a_drag_and_staged_from_their_menu,
    },
    Test {
        name: "the_file_in_its_past_is_blame_without_the_words_and_the_list_ends",
        run: the_file_in_its_past_is_blame_without_the_words_and_the_list_ends,
    },
    Test {
        name: "a_file_deleted_on_one_side_is_asked_about_in_the_bar",
        run: a_file_deleted_on_one_side_is_asked_about_in_the_bar,
    },
    Test {
        name: "a_conflict_is_taken_by_its_box_and_reset_takes_the_choices_back",
        run: a_conflict_is_taken_by_its_box_and_reset_takes_the_choices_back,
    },
    Test {
        name: "a_commits_details_say_what_is_in_conflict_instead_of_a_banner",
        run: a_commits_details_say_what_is_in_conflict_instead_of_a_banner,
    },
    Test {
        name: "asking_for_the_references_panel_closes_what_folded_it",
        run: asking_for_the_references_panel_closes_what_folded_it,
    },
    Test {
        name: "text_in_a_diff_is_selected_with_the_pointer_and_copied",
        run: text_in_a_diff_is_selected_with_the_pointer_and_copied,
    },
    Test {
        name: "text_in_a_blame_is_selected_with_the_pointer_and_copied",
        run: text_in_a_blame_is_selected_with_the_pointer_and_copied,
    },
];

/// A file one side of a merge changed and the other deleted: opening it
/// asks in the bar, the middle stays as it was; giving up leaves the
/// conflict, keeping the file resolves it with the changed one.
fn a_file_deleted_on_one_side_is_asked_about_in_the_bar() {
    repos::fresh();
    let repo = repos::project_with_a_deleted_conflict();
    let mut stage = Stage::open(Start::with(&repo));
    assert_eq!(stage.workspace().in_progress, Some(("merge".to_owned(), 1)));
    stage.keys("ctrl-home");
    stage.focus_changes();
    stage.keys("home");
    stage.keys("down enter");
    let asked = stage.shell().bar.map(|(said, _)| said);
    let said = asked.expect("the bar asks");
    assert!(said.contains("guide.md"), "{said}");
    assert!(
        stage.workspace().file.is_none(),
        "the middle stays as it was"
    );

    stage.click("ask-cancel");
    assert_eq!(stage.workspace().in_progress, Some(("merge".to_owned(), 1)));

    stage.focus_changes();
    stage.keys("home");
    stage.keys("down enter");
    stage.click(("ask-choice", 0usize));
    stage.until("the file is kept and resolved", |stage| {
        stage.workspace().in_progress == Some(("merge".to_owned(), 0))
    });
    assert_eq!(
        std::fs::read_to_string(repo.join("docs/guide.md")).unwrap(),
        "Search for a needle, once more.\n"
    );
}

/// A file's past: the list of commits ends, where the file began, in a row
/// that says so; the file as it was is blame without the commits' words
/// (the author's picture, the number, the bar), and blame has them; both
/// have the miniature of the text.
fn the_file_in_its_past_is_blame_without_the_words_and_the_list_ends() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("src/render.rs");
    stage.click("tab-history");
    stage.until("the list is in", |stage| stage.shows("history-end"));
    let blame = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| {
            let blame = workspace.read(cx).blame();
            let blame = blame.read(cx);
            (blame.is_plain(), blame.line_count())
        })
    };
    stage.click("past-file");
    stage.until("the file as it was", |stage| blame(stage).1 > 0);
    assert!(blame(&mut stage).0, "without the commits' words");
    assert!(stage.shows("blame-minimap"));
    stage.click("past-blame");
    assert_eq!(stage.workspace().file_tab, "blame");
    stage.until("the blame", |stage| blame(stage).1 > 0);
    assert!(!blame(&mut stage).0, "with the commits' words");
    // Blamed at the chosen commit, there is no way back to offer.
    assert!(!stage.shows("blame-head"));
}

/// Lines of a diff: a click picks a changed line, a drag the changed lines
/// between its ends, and the line's menu stages them; the hunk's buttons
/// stay about the hunk.
fn lines_are_picked_by_a_click_or_a_drag_and_staged_from_their_menu() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("src/render.rs");
    stage.until("the hunks are in", |stage| {
        let probe = shown(stage);
        !probe.loading && probe.headings > 0
    });
    let list = stage.window(|window, _| window.find("diff-list").bounds());
    let (x, top) = (f32::from(list.left()) + 200.0, f32::from(list.top()));
    // The first hunk: its heading 32 high, then the removed line (16 high)
    // and the added one.
    let (removed, added) = (top + 40.0, top + 56.0);
    stage.click_at((x, removed));
    assert_eq!(shown(&mut stage).selected, 1, "a click picks the line");
    stage.drag_at((x, removed), (x, added));
    assert_eq!(shown(&mut stage).selected, 2, "a drag picks both");
    // The hunk's buttons say the hunk still.
    assert!(stage.shows(("stage-hunk", 0usize)));

    // The menu stages what is picked: the two lines, and only them.
    stage.act(bitbraid_ui::views::diff::ActOnLines {
        command: "stage.hunks".to_owned(),
        hunk: 0,
        line: 0,
    });
    stage.until("the two lines are staged", |_| {
        repos::git(
            &repo,
            &["diff", "--cached", "--numstat", "--", "src/render.rs"],
        )
        .starts_with("1\t1\t")
    });
}

/// The rows above a file: the sides of the index are two joined buttons, of
/// which only the sides the file has are there and the other is shown by a
/// click; what a file cannot have is not offered (editing a file that is
/// gone or a picture, blame for a picture, a past for a file that is new);
/// Escape closes the file wherever the keys are.
fn the_rows_above_a_file_offer_what_it_can_have_and_escape_closes_it() {
    repos::fresh();
    let repo = repos::project_with_changes();
    // A file with a change on both sides of the index.
    let guide = repo.join("docs/guide.md");
    let mut text = std::fs::read_to_string(&guide).unwrap();
    text.push_str("And one more line, not staged.\n");
    std::fs::write(&guide, text).unwrap();

    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    let offers = |stage: &mut Stage| -> [bool; 5] {
        [
            stage.shows("tab-edit"),
            stage.shows("tab-blame"),
            stage.shows("tab-history"),
            stage.shows("side-unstaged"),
            stage.shows("side-staged"),
        ]
    };
    let side_shown = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).diff().read(cx).side())
    };

    // Changed in the worktree only: everything, and one side.
    stage.open_changed("src/render.rs");
    assert_eq!(offers(&mut stage), [true, true, true, true, false]);
    // Changed on both sides: both, and a click shows the other one.
    stage.open_changed("docs/guide.md");
    assert_eq!(offers(&mut stage), [true, true, true, true, true]);
    let first = side_shown(&mut stage);
    let other = if first == Some(bitbraid_ui::views::changes::Side::Unstaged) {
        "side-staged"
    } else {
        "side-unstaged"
    };
    stage.click(other);
    assert_ne!(side_shown(&mut stage), first, "the other side is shown");
    // Gone from the worktree: nothing to edit.
    stage.open_changed("grammar.rs");
    stage.until("the diff of the file that is gone", |stage| {
        !shown(stage).loading
    });
    assert_eq!(offers(&mut stage), [false, true, true, true, false]);
    // New: no past yet.
    stage.open_changed("notes.txt");
    stage.until("the diff of the new file", |stage| !shown(stage).loading);
    assert_eq!(offers(&mut stage), [true, false, false, true, false]);
    // A picture: its history only.
    stage.open_changed("assets/logo.png");
    stage.until("the picture", |stage| shown(stage).picture.is_some());
    assert_eq!(offers(&mut stage), [false, false, true, true, false]);

    // Escape closes the file, also with the keys in the changes panel.
    stage.focus_changes();
    stage.keys("escape");
    assert!(stage.workspace().file.is_none(), "Escape closed the file");
}

/// Closing a file whose text was edited and not saved asks first, in the
/// bar: giving up stays, leaving the changes throws them away, saving
/// writes the file and then closes it.
fn a_file_edited_and_not_saved_is_asked_about_before_it_closes() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let file = repo.join("notes.txt");
    let on_disk = std::fs::read_to_string(&file).unwrap();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("notes.txt");
    let edit = |stage: &mut Stage, text: &str| {
        if stage.shows("tab-edit") {
            stage.click("tab-edit");
        }
        stage.until("the file is open in the editor", |stage| {
            stage.with_workspace(|workspace, _, cx| workspace.read(cx).editor().read(cx).is_open())
        });
        // Edited where the file view is: the button gives way to a word,
        // and the row has reading again and saving.
        assert!(stage.shows("editing") && !stage.shows("tab-edit"));
        assert!(stage.shows("edit-save") && stage.shows("edit-revert"));
        stage.with_workspace(|workspace, window, cx| {
            let editor = workspace.read(cx).editor();
            editor.update(cx, |editor, cx| editor.focus_text(window, cx));
        });
        stage.keys("ctrl-home");
        stage.type_text(text);
        assert!(stage.with_workspace(|workspace, _, cx| {
            workspace.read(cx).editor().read(cx).is_dirty()
        }));
    };
    let asked = |stage: &mut Stage| stage.shell().bar.map(|(said, _)| said);

    // Giving up: the file stays open, the text as it was typed.
    edit(&mut stage, "first ");
    stage.click("close-diff");
    let said = asked(&mut stage).expect("closing asks first");
    assert!(said.contains("notes.txt"), "{said}");
    stage.click("ask-cancel");
    assert!(asked(&mut stage).is_none());
    assert!(stage.workspace().file.is_some(), "it stayed open");

    // Leaving the changes: closed, and the file opens from the disk.
    stage.click("close-diff");
    stage.click(("ask-choice", 1usize));
    assert!(stage.workspace().file.is_none(), "it closed");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), on_disk);
    stage.open_changed("notes.txt");
    stage.click("tab-edit");
    stage.until("the file is open from the disk", |stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).editor().read(cx).is_open())
    });
    assert!(
        !stage
            .with_workspace(|workspace, _, cx| { workspace.read(cx).editor().read(cx).is_dirty() })
    );

    // Saving: written, then closed.
    edit(&mut stage, "kept ");
    stage.click("close-diff");
    stage.click(("ask-choice", 0usize));
    stage.until("the file is written and closed", |stage| {
        stage.workspace().file.is_none()
    });
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!("kept {on_disk}")
    );
}

/// Escape closes the history and blame of a file wherever in them the keys
/// are, not only in their list. A click on a line of the blame shows its
/// commit in the graph and puts the past away.
fn escape_closes_a_files_past_wherever_in_it_the_keys_are() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("src/render.rs");
    let past = |stage: &mut Stage, then: &dyn Fn(&mut Stage)| {
        stage.click("tab-history");
        assert_eq!(stage.workspace().file_tab, "history");
        stage.until("the past has what the commit changed", |stage| {
            !shown(stage).loading && shown(stage).lines > 0
        });
        then(stage);
        stage.keys("escape");
        assert_eq!(
            stage.workspace().file_tab,
            "diff",
            "Escape went back to the file"
        );
        assert!(stage.shows("tab-history"));
    };
    // The keys in the list.
    past(&mut stage, &|_| {});
    // The keys in what the commit changed, beside the list.
    past(&mut stage, &|stage| {
        stage.window(|window, cx| window.click_at("diff-list", point(px(200.0), px(30.0)), cx));
        stage.frame();
    });
    // The keys in the blame.
    past(&mut stage, &|stage| {
        stage.click("past-file");
        stage.click("past-blame");
        assert_eq!(stage.workspace().file_tab, "blame");
        stage.until("the blame is in", |stage| stage.shows("blame"));
        stage.click("blame");
    });
    // A click on a line of the blame shows its commit in the graph: the
    // past goes, and the file it was opened from with it, so that the
    // graph is what is seen.
    stage.click("tab-history");
    stage.until("the past has what the commit changed", |stage| {
        !shown(stage).loading && shown(stage).lines > 0
    });
    stage.click("past-file");
    stage.click("past-blame");
    stage.until("the blame is in", |stage| stage.shows("blame"));
    stage.window(|window, cx| window.click_at("blame-list", point(px(120.0), px(10.0)), cx));
    stage.frame();
    assert!(stage.workspace().file.is_none(), "back to the graph");
    assert_eq!(stage.graph_focus(), (true, false), "the graph has the keys");
}

/// Opened by keys alone (the finder: Ctrl+Shift+H, a path, Enter), a file's
/// past closes with Escape, and the graph has the keys again, though the
/// finder's dialog gives them back to the graph hidden behind the past a
/// moment after it closes. A click on a line of its blame puts the past
/// away and selects that line's commit in the graph, which has the keys.
fn a_files_past_opened_by_keys_closes_with_escape_and_its_blame_shows_the_graph() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    let open = |stage: &mut Stage| {
        stage.keys("ctrl-shift-h");
        assert!(stage.dialog_open(), "the finder is open");
        stage.type_text("render.rs");
        stage.pass(std::time::Duration::from_secs(1));
        stage.keys("enter");
        stage.until("the file's past is open", |stage| {
            !stage.dialog_open() && stage.workspace().file_tab == "history"
        });
        stage.until("the past has what the commit changed", |stage| {
            !shown(stage).loading && shown(stage).lines > 0
        });
        // As in the window, where one waits to see it before the next
        // key: the toolkit's timers run meanwhile, and the finder's
        // dialog, closed by its own Enter, gives the keys back to the
        // graph behind the past a quarter of a second later.
        stage.pass(std::time::Duration::from_secs(1));
    };

    open(&mut stage);
    stage.keys("escape");
    assert!(
        stage.workspace().file.is_none(),
        "Escape closed the past: {:?}",
        stage.workspace().file
    );
    assert_eq!(stage.graph_focus(), (true, false), "the graph has the keys");

    open(&mut stage);
    stage.click("past-file");
    stage.click("past-blame");
    assert_eq!(stage.workspace().file_tab, "blame");
    stage.until("the blame is in", |stage| stage.shows("blame"));
    let blamed = repos::git(
        &repo,
        &["blame", "-l", "-L1,1", "HEAD", "--", "src/render.rs"],
    );
    let first = blamed
        .trim_start_matches('^')
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    stage.window(|window, cx| window.click_at("blame-list", point(px(120.0), px(10.0)), cx));
    stage.frame();
    assert!(
        stage.workspace().file.is_none(),
        "the past went: {:?}",
        stage.workspace().file
    );
    let selected = stage.graph().selected.unwrap_or_default();
    assert!(
        !first.is_empty() && first.starts_with(&selected) || selected.starts_with(&first),
        "the line's commit {first} is selected: {selected}"
    );
    assert_eq!(stage.graph_focus(), (true, false), "the graph has the keys");
}

/// The other ways to a file's past without the pointer, or with it only
/// to open a menu, close with Escape too, once one has waited to see the
/// past: the finder reached from the palette, the menu key on a
/// changed file, and the menu of a commit's file walked with the keys.
fn a_files_past_opened_from_the_palette_or_a_menu_closes_with_escape() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let closes = |stage: &mut Stage, how: &str| {
        stage.until("the file's past is open", |stage| {
            !stage.dialog_open() && stage.workspace().file_tab == "history"
        });
        stage.until("the past has what the commit changed", |stage| {
            !shown(stage).loading && shown(stage).lines > 0
        });
        stage.pass(std::time::Duration::from_secs(1));
        stage.keys("escape");
        assert_ne!(
            stage.workspace().file_tab,
            "history",
            "Escape closed the past opened {how}"
        );
        // Back to the file when the pane was about it already, and the
        // keys are there: Escape closes that too.
        if stage.workspace().file.is_some() {
            stage.keys("escape");
        }
        assert!(
            stage.workspace().file.is_none(),
            "back to the graph: {:?}",
            stage.workspace().file
        );
        assert_eq!(stage.graph_focus(), (true, false), "the graph has the keys");
    };

    // The palette's entry for the finder, then the finder.
    stage.keys("ctrl-p");
    stage.type_text("find a file by its path");
    stage.pass(std::time::Duration::from_secs(1));
    stage.keys("enter");
    stage.until("the finder is open", |stage| stage.dialog_open());
    stage.pass(std::time::Duration::from_secs(1));
    stage.type_text("render.rs");
    stage.pass(std::time::Duration::from_secs(1));
    stage.keys("enter");
    closes(&mut stage, "from the palette");

    // The menu key on the staged file, then its fourth entry, past
    // unstaging, discarding and ignoring it: its history.
    stage.focus_changes();
    stage.keys("end shift-f10");
    stage.until("its menu is open", |stage| {
        stage.with_workspace(|workspace, _, cx| {
            workspace.read(cx).changes().read(cx).key_menu_open()
        })
    });
    stage.keys("down down down down enter");
    closes(&mut stage, "from the menu key");

    // A commit's file: its menu, then the first entry, its history.
    let commit = stage.graph_spot(1, false);
    stage.click_at(commit);
    stage.until("a commit is selected", |stage| !stage.graph().wip_selected);
    stage.until("its files are listed", |stage| stage.shows("commit-files"));
    let (x, y, _, _) = stage.place("commit-files");
    stage.right_click_at((x + 80., y + 16.));
    stage.pass(std::time::Duration::from_millis(300));
    stage.keys("down enter");
    closes(&mut stage, "from a commit's file");
}

/// Enter in the finder is for what is typed: pressed before the files that
/// match it have come, it waits for them and opens the first, not the
/// first of those found for the text before. From the palette's entry for
/// the finder too.
fn enter_in_the_finder_opens_the_first_file_of_what_is_typed_even_before_it_is_found() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    let typed_fast = |stage: &mut Stage, how: &str| {
        assert!(stage.dialog_open(), "the finder is open");
        // A text, and the files it finds are in.
        stage.type_text("guide");
        stage.pass(std::time::Duration::from_secs(1));
        // Another in its place, and Enter at once: before what it finds
        // has come.
        stage.window(|window, cx| {
            window.press("ctrl-a", cx);
            window.input("render.rs", cx);
            window.press("enter", cx);
        });
        stage.until("the file's past is open", |stage| {
            !stage.dialog_open() && stage.workspace().file_tab == "history"
        });
        assert_eq!(
            stage.workspace().file.map(|(path, _)| path).as_deref(),
            Some("src/render.rs"),
            "the first file the typed text finds, {how}"
        );
        stage.pass(std::time::Duration::from_secs(1));
        stage.keys("escape");
        assert!(stage.workspace().file.is_none(), "closed again");
    };

    stage.keys("ctrl-shift-h");
    typed_fast(&mut stage, "with its key");

    stage.keys("ctrl-p");
    stage.type_text("find a file by its path");
    stage.pass(std::time::Duration::from_secs(1));
    stage.keys("enter");
    stage.until("the finder is open", |stage| stage.dialog_open());
    stage.pass(std::time::Duration::from_secs(1));
    typed_fast(&mut stage, "from the palette");

    // Enter for a text that finds nothing leaves the finder open, and it
    // still is when what was found for the text before was something.
    stage.keys("ctrl-shift-h");
    stage.type_text("guide");
    stage.pass(std::time::Duration::from_secs(1));
    stage.window(|window, cx| {
        window.press("ctrl-a", cx);
        window.input("qqqzzz", cx);
        window.press("enter", cx);
    });
    stage.pass(std::time::Duration::from_secs(1));
    assert!(stage.dialog_open(), "nothing found, nothing opened");
    assert!(stage.workspace().file.is_none(), "nothing opened");
    stage.keys("escape");
    assert!(!stage.dialog_open());
}

/// The strip at the right edge of a diff: 14 wide beside the hunks, 44 with
/// the map beside the whole file, 134 with the miniature of the text beside
/// the file as it is. A click on a line puts the cursor there, whose mark
/// the strip carries; the pointer over the strip shows what is in view; a
/// click low on it scrolls down.
fn the_strip_beside_a_diff_maps_the_whole_file_and_is_where_one_scrolls() {
    repos::fresh();
    let repo = repos::project_with_changes();
    // A file long enough to scroll, changed near its top and its end.
    let long: String = (1..=400).map(|n| format!("line {n}\n")).collect();
    std::fs::write(repo.join("long.txt"), &long).unwrap();
    repos::git(&repo, &["add", "long.txt"]);
    repos::git(&repo, &["commit", "-q", "-m", "a long file"]);
    let changed = long
        .replace("line 3\n", "line three\n")
        .replace("line 390\n", "line three hundred and ninety\n");
    std::fs::write(repo.join("long.txt"), changed).unwrap();

    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("long.txt");
    stage.until("the hunks are in", |stage| {
        let probe = shown(stage);
        !probe.loading && probe.headings > 0
    });
    // What the strip is as last drawn: a frame first, for what came in.
    let width = |stage: &mut Stage| {
        stage.frame();
        stage.window(|window, _| f32::from(window.find("diff-minimap").bounds().size.width))
    };
    assert_eq!(width(&mut stage), 14.0, "beside the hunks: the strip alone");

    // A line clicked is where the cursor is.
    stage.window(|window, cx| window.click_at("diff-list", point(px(200.0), px(40.0)), cx));
    stage.frame();
    // The heading is 32 high and a line 16: 40 down is the first line.
    assert_eq!(shown(&mut stage).cursor, Some(1));
    // Over the strip it shows what is in view.
    stage.window(|window, cx| window.hover("diff-minimap", cx));
    stage.frame();
    assert!(shown(&mut stage).map_hovered);

    // The whole file: the strip and the map, and a click low on them goes
    // down the file.
    stage.click("mode-inline");
    stage.until("the whole file is in", |stage| {
        let probe = shown(stage);
        !probe.loading && probe.headings == 0 && probe.lines > 400
    });
    assert_eq!(
        width(&mut stage),
        44.0,
        "beside the whole file: strip and map"
    );
    assert_eq!(shown(&mut stage).scrolled, 0);
    let height =
        stage.window(|window, _| f32::from(window.find("diff-minimap").bounds().size.height));
    stage.window(|window, cx| {
        window.click_at("diff-minimap", point(px(30.0), px(height - 4.0)), cx)
    });
    stage.frame();
    assert!(shown(&mut stage).scrolled > 0, "{:?}", shown(&mut stage));

    // The file as it is: a miniature of its text left of the strip.
    stage.click("view-file");
    stage.until("the file as it is", |stage| shown(stage).file_view);
    assert_eq!(width(&mut stage), 134.0, "the miniature and the strip");
    stage.click("view-diff");
    stage.click("mode-hunks");
}

/// What the viewer in the centre shows.
fn shown(stage: &mut Stage) -> bitbraid_ui::views::diff::DiffProbe {
    stage.with_workspace(|workspace, _, cx| workspace.read(cx).diff().read(cx).probe())
}

/// The three layouts and the file view: the hunks under their
/// headings; the whole file in one column or in two, without headings;
/// the file as it is, one line a row. The layout is the preferences', so
/// it holds for the next file; the file view is the file's.
fn a_diff_is_shown_as_its_hunks_or_whole_and_the_file_as_it_is() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let lines = std::fs::read_to_string(repo.join("src/render.rs"))
        .unwrap()
        .lines()
        .count();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("src/render.rs");
    let at_rest = |stage: &mut Stage, layout: &str| {
        stage.until("the diff is in", |stage| {
            let probe = shown(stage);
            !probe.loading && probe.layout == layout && probe.lines > 0
        });
        shown(stage)
    };
    let layout =
        |stage: &mut Stage| stage.app(|cx| cx.global::<Prefs>().preferences.diff.layout.clone());

    // The hunks: a heading over each, three lines around a change.
    let hunks = at_rest(&mut stage, "hunks");
    assert!(hunks.headings > 0, "{hunks:?}");
    assert!(
        hunks.lines < lines,
        "only the parts that changed: {hunks:?}"
    );

    // One column: every line of the file, and what was taken out of it.
    stage.click("mode-inline");
    let inline = at_rest(&mut stage, "one_column");
    assert_eq!(inline.headings, 0, "{inline:?}");
    assert_eq!(inline.lines, lines + inline.removed as usize, "{inline:?}");
    assert_eq!(layout(&mut stage), "one_column");

    // Two columns: a row for every pair, still no heading.
    stage.click("mode-split");
    let split = at_rest(&mut stage, "two_columns");
    assert_eq!(split.headings, 0, "{split:?}");
    assert!(
        split.lines >= lines && split.lines < inline.lines,
        "{split:?}"
    );

    // The file as it is: a row a line, and the layouts are not offered.
    stage.click("view-file");
    stage.until("the file as it is", |stage| {
        let probe = shown(stage);
        probe.file_view && !probe.loading && probe.lines == lines
    });
    assert!(!stage.shows("mode-split") && !stage.shows("diff-next"));
    assert!(stage.shows("diff-wrap"));
    stage.click("view-diff");
    assert!(!shown(&mut stage).file_view);

    // Back to the hunks, which the next file opens in too.
    stage.click("mode-hunks");
    let back = at_rest(&mut stage, "hunks");
    assert_eq!(back.headings, hunks.headings);
    assert_eq!(layout(&mut stage), "hunks");
}

/// The merge tool: both files whole with a box at each conflict's first
/// line and one over each side; a box takes its side's lines into the
/// result, the box over a side takes that side everywhere, and Reset, there
/// only once something was chosen, takes the choices back.
fn a_conflict_is_taken_by_its_box_and_reset_takes_the_choices_back() {
    repos::fresh();
    let repo = repos::project_in_conflict();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.focus_changes();
    stage.keys("down enter");
    let text = |stage: &mut Stage| -> Option<String> {
        stage.with_workspace(|workspace, _, cx| {
            let tool = workspace.read(cx).merge_tool();
            tool.read(cx).result_text(cx)
        })
    };
    stage.until("the merge tool has the file", |stage| text(stage).is_some());
    assert!(text(&mut stage).unwrap().contains("<<<<<<<"));
    assert!(!stage.shows("merge-rebuild"), "nothing chosen yet");

    stage.click(("take-ours", 0usize));
    assert_eq!(
        text(&mut stage).unwrap(),
        "Search for a needle.\nUse single quotes around several words.\n"
    );
    assert!(stage.shows("merge-rebuild"));

    // Clicked again, the box leaves the side out: the conflict is decided
    // for neither.
    stage.click(("take-ours", 0usize));
    assert_eq!(text(&mut stage).unwrap(), "Search for a needle.\n");
    // The box over the other side takes that side everywhere.
    stage.click("take-all-theirs");
    assert_eq!(
        text(&mut stage).unwrap(),
        "Search for a needle.\nUse double quotes around several words.\nSee the README.\n"
    );

    stage.click("merge-rebuild");
    assert!(text(&mut stage).unwrap().contains("<<<<<<<"));
    assert!(!stage.shows("merge-rebuild"));
    assert_eq!(stage.workspace().in_progress, Some(("merge".to_owned(), 1)));
}

/// An operation stopped on conflicts while a commit is selected: there is
/// no banner over the graph; the strip over the commit's details says how
/// many files are in conflict, and its button goes to them, where the ways
/// out are.
fn a_commits_details_say_what_is_in_conflict_instead_of_a_banner() {
    repos::fresh();
    let repo = repos::project_in_conflict();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    assert!(stage.workspace().changes_shown);
    assert!(!stage.shows("progress-continue"));
    stage.keys("down");
    assert!(!stage.graph().wip_selected);
    assert!(!stage.workspace().changes_shown, "a commit's details");
    assert!(stage.shows("view-changes"));
    assert!(!stage.shows("progress-continue"), "no banner");

    stage.click("view-changes");
    assert!(stage.workspace().changes_shown);
    assert!(stage.shows("op-continue") && stage.shows("op-abort"));
    assert_eq!(stage.workspace().in_progress, Some(("merge".to_owned(), 1)));
}

/// A file in the middle folds the references panel to its strip, and its
/// past and the plan of a rebase take the panel's place; closing them
/// brings it back. Asking for the panel meanwhile is closing them, as
/// their own way out does: the strip's button, the panel's key, the
/// palette, the key of its filter.
fn asking_for_the_references_panel_closes_what_folded_it() {
    use bitbraid_ui::{
        keymap,
        views::rewrite::{Rewrite, RunRewrite},
    };
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    let open = |stage: &mut Stage| {
        stage.open_changed("src/render.rs");
        assert!(
            stage.shows("unfold-refs") && !stage.shows("fold-refs"),
            "the file folds the panel to its strip"
        );
    };
    let back = |stage: &mut Stage, how: &str| {
        stage.settle();
        assert!(
            stage.workspace().file.is_none(),
            "{how}: the file is closed: {:?}",
            stage.workspace().file
        );
        assert!(!stage.workspace().planning, "{how}: the plan is put away");
        assert!(
            stage.shows("fold-refs") && !stage.shows("unfold-refs"),
            "{how}: the panel is open"
        );
        let (_, _, width, _) = stage.place("refs");
        assert!(width > 200.0, "{how}: the panel has its width: {width}");
    };

    // The strip's button.
    open(&mut stage);
    stage.click("unfold-refs");
    back(&mut stage, "the strip's button");
    // The panel's key.
    open(&mut stage);
    stage.keys("ctrl-j");
    back(&mut stage, "the key");
    // The palette's entry.
    open(&mut stage);
    stage.keys("ctrl-p");
    stage.type_text("show or hide the references panel");
    stage.pass(std::time::Duration::from_secs(1));
    stage.keys("enter");
    back(&mut stage, "the palette");
    // The key of the filter: the panel opens with the keys in its filter.
    open(&mut stage);
    stage.act(keymap::FilterReferences);
    back(&mut stage, "the filter's key");

    // A file's past takes the panel's place: back to the graph, not to
    // the file it was opened from, which would fold the panel again.
    open(&mut stage);
    stage.click("tab-history");
    assert_eq!(stage.workspace().file_tab, "history");
    assert!(!stage.shows("unfold-refs") && !stage.shows("fold-refs"));
    stage.act(keymap::ToggleLeftPanel);
    back(&mut stage, "the key in the file's past");

    // Folded by hand before the file was opened: asked for, it opens.
    stage.click("fold-refs");
    open(&mut stage);
    stage.click("unfold-refs");
    back(&mut stage, "folded by hand");

    // The plan of a rebase takes the panel's place: given up, as Escape
    // in it does.
    let first = repos::rev(&repo, "HEAD~1");
    stage.act(RunRewrite {
        rewrite: Rewrite::Interactive {
            first,
            reword: false,
        },
    });
    assert!(stage.workspace().planning);
    stage.act(keymap::ToggleLeftPanel);
    back(&mut stage, "the key in the plan");

    // Edited text is asked about first; given up, the file stays, and
    // the panel stays its strip.
    stage.open_changed("notes.txt");
    stage.click("tab-edit");
    stage.until("the file is open in the editor", |stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).editor().read(cx).is_open())
    });
    stage.with_workspace(|workspace, window, cx| {
        let editor = workspace.read(cx).editor();
        editor.update(cx, |editor, cx| editor.focus_text(window, cx));
    });
    stage.keys("ctrl-home");
    stage.type_text("edited ");
    stage.click("unfold-refs");
    assert!(stage.shell().bar.is_some(), "asked first");
    stage.click("ask-cancel");
    assert!(stage.workspace().file.is_some(), "it stayed open");
    assert!(
        stage.shows("unfold-refs") && !stage.shows("fold-refs"),
        "and the panel its strip"
    );
    stage.click("unfold-refs");
    stage.click(("ask-choice", 1usize));
    back(&mut stage, "the edited text left");

    // Nothing in the middle: the key folds the panel and opens it again.
    stage.keys("ctrl-j");
    assert!(stage.shows("unfold-refs"), "folded");
    stage.keys("ctrl-j");
    assert!(stage.shows("fold-refs"), "open");
}

/// What the clipboard holds as text.
fn clipboard(stage: &mut Stage) -> Option<String> {
    stage.app(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
}

/// How wide a character of the diff's text is: the font of code at 12,
/// six tenths of it.
const CHARACTER: f32 = 7.2;

/// Text in a diff is selected with the pointer and copied: a drag across
/// lines and Ctrl+C put the text of the lines between its ends on the
/// clipboard, without numbers or signs; two clicks take a word, three the
/// line, Shift with a click moves the end; "Copy" is in a line's menu; in
/// two columns only the column the drag began in is taken; a press beside
/// the text lets it go. Lines for staging are picked on the numbers: the
/// press there picks the line, and its menu stages it.
fn text_in_a_diff_is_selected_with_the_pointer_and_copied() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("src/render.rs");
    stage.until("the hunks are in", |stage| {
        let probe = shown(stage);
        !probe.loading && probe.headings > 0
    });
    // The lines of the first hunk, as git has them, without their signs.
    let patch = repos::git(&repo, &["diff", "--", "src/render.rs"]);
    let lines: Vec<String> = patch
        .lines()
        .skip_while(|line| !line.starts_with("@@"))
        .skip(1)
        .take_while(|line| !line.starts_with("@@"))
        .map(|line| line.get(1..).unwrap_or_default().to_owned())
        .collect();
    assert!(lines.len() >= 3, "{lines:?}");
    let list = stage.window(|window, _| window.find("diff-list").bounds());
    let (left, top, right) = (
        f32::from(list.left()),
        f32::from(list.top()),
        f32::from(list.right()),
    );
    // The first hunk: its heading 32 high, then a row of 16 a line; the
    // text begins after the two numbers and the sign, 70 in.
    let row = |n: usize| top + 32.0 + 16.0 * n as f32 + 8.0;
    let at = |column: usize| left + 70.0 + CHARACTER * column as f32 + 1.0;

    // From the third character of the first line to past the end of the
    // third: what is between, the lines' breaks with it.
    stage.drag_at((at(2), row(0)), (right - 20.0, row(2)));
    stage.keys("ctrl-c");
    let wanted = format!(
        "{}\n{}\n{}",
        lines[0].get(2..).unwrap_or_default(),
        lines[1],
        lines[2]
    );
    assert_eq!(clipboard(&mut stage).as_deref(), Some(wanted.as_str()));
    assert_eq!(shown(&mut stage).text_selected, wanted);
    // The drag picked the changed lines it went over too.
    assert_eq!(shown(&mut stage).selected, 2, "the two changed lines");

    // Two clicks take a word, three the line.
    let word = lines[1]
        .find("parsed")
        .expect("the line says what the fixture made it say");
    stage.clicks_at((at(word + 2), row(1)), 2, Default::default());
    assert_eq!(shown(&mut stage).text_selected, "parsed");
    stage.clicks_at((at(word + 2), row(1)), 3, Default::default());
    assert_eq!(shown(&mut stage).text_selected, lines[1]);
    // Shift with a click moves the end: from the fifth character of the
    // first line to the end of the second.
    stage.clicks_at(
        (at(4), row(0)),
        1,
        gpui_kit::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    let wanted = format!("{}\n{}", lines[0].get(4..).unwrap_or_default(), lines[1]);
    assert_eq!(shown(&mut stage).text_selected, wanted);
    // The line's menu copies it too.
    stage.right_click_at((at(6), row(1)));
    stage.pass(std::time::Duration::from_millis(300));
    stage.app(|cx| cx.write_to_clipboard(gpui_kit::ClipboardItem::new_string(String::new())));
    stage.keys("down enter");
    assert_eq!(clipboard(&mut stage).as_deref(), Some(wanted.as_str()));

    // A press on the numbers lets the text go and picks the line, which
    // its menu stages, alone.
    stage.click_at((left + 10.0, row(0)));
    assert_eq!(shown(&mut stage).text_selected, "");
    assert_eq!(shown(&mut stage).selected, 1, "the press picked the line");
    stage.act(bitbraid_ui::views::diff::ActOnLines {
        command: "stage.hunks".to_owned(),
        hunk: 0,
        line: 0,
    });
    stage.until("the one line is staged", |_| {
        repos::git(
            &repo,
            &["diff", "--cached", "--numstat", "--", "src/render.rs"],
        )
        .starts_with("0\t1\t")
    });

    // Two columns: the new side's first three lines, though the drag
    // ends over the old side.
    stage.open_changed("src/render.rs");
    stage.click("mode-split");
    stage.until("the two columns are in", |stage| {
        let probe = shown(stage);
        !probe.loading && probe.layout == "two_columns" && probe.lines > 0
    });
    let list = stage.window(|window, _| window.find("diff-list").bounds());
    let (left, top, width) = (
        f32::from(list.left()),
        f32::from(list.top()),
        f32::from(list.size.width),
    );
    let half = (width - 14.0) / 2.0;
    let new_text = left + half + 14.0 + 40.0 + 1.0;
    let row = |n: usize| top + 16.0 * n as f32 + 8.0;
    stage.drag_at((new_text, row(0)), (left + 60.0, row(2)));
    stage.keys("ctrl-c");
    let new_side: Vec<String> = std::fs::read_to_string(repo.join("src/render.rs"))
        .unwrap()
        .lines()
        .take(3)
        .map(str::to_owned)
        .collect();
    // The drag ended left of the new side's third line: up to its start.
    let wanted = format!("{}\n{}\n", new_side[0], new_side[1]);
    assert_eq!(clipboard(&mut stage).as_deref(), Some(wanted.as_str()));
    stage.click("mode-hunks");
}

/// Text in a blame, and in the file as it was in a file's past, is
/// selected and copied as in a diff.
fn text_in_a_blame_is_selected_with_the_pointer_and_copied() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("src/render.rs");
    stage.click("tab-blame");
    let count = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).blame().read(cx).line_count())
    };
    stage.until("the blame", |stage| count(stage) > 0);
    let list = stage.window(|window, _| window.find("blame-list").bounds());
    let (left, top, right) = (
        f32::from(list.left()),
        f32::from(list.top()),
        f32::from(list.right()),
    );
    // The text begins 407 in; a row is 16 high.
    let row = |n: usize| top + 16.0 * n as f32 + 8.0;
    stage.drag_at((left + 408.0, row(0)), (right - 20.0, row(2)));
    stage.keys("ctrl-c");
    let copied = clipboard(&mut stage).unwrap_or_default();
    let in_worktree: Vec<String> = std::fs::read_to_string(repo.join("src/render.rs"))
        .unwrap()
        .lines()
        .take(3)
        .map(str::to_owned)
        .collect();
    let at_head: Vec<String> = repos::git(&repo, &["show", "HEAD:src/render.rs"])
        .lines()
        .take(3)
        .map(str::to_owned)
        .collect();
    assert!(
        copied == in_worktree.join("\n") || copied == at_head.join("\n"),
        "{copied:?}"
    );
}
