//! What the window does, asked of the window itself. Keys are pressed as
//! keys, not dispatched as the actions they stand for, wherever the key is
//! what is being tested.

use std::time::Duration;

use bitbraid_ui::{
    keymap,
    prefs::{Prefs, ThemeChoice},
    views::{
        refs::{ToggleHidden, TogglePinned, ToggleSolo},
        rewrite::{Rewrite, RunRewrite},
        submodules::{RunSubmodule, SubmoduleRequest},
    },
};
use gpui_kit::{ElementId, component::Theme, px, test::TestWindowExt as _};

use crate::{
    compare, repos,
    runner::Test,
    stage::{HEIGHT, Stage, Start, WIDTH},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "the_comparison_sees_what_it_is_for",
        run: compare::self_test,
    },
    Test {
        name: "a_repository_opens_and_comes_to_rest",
        run: a_repository_opens_and_comes_to_rest,
    },
    Test {
        name: "a_repository_opens_on_its_uncommitted_work",
        run: a_repository_opens_on_its_uncommitted_work,
    },
    Test {
        name: "the_commit_button_says_how_many_files_and_waits_for_them",
        run: the_commit_button_says_how_many_files_and_waits_for_them,
    },
    Test {
        name: "pictures_are_asked_for_only_while_they_are_switched_on",
        run: crate::pictures::pictures_are_asked_for_only_while_they_are_switched_on,
    },
    Test {
        name: "switching_pictures_off_drops_what_is_under_way",
        run: crate::pictures::switching_pictures_off_drops_what_is_under_way,
    },
    Test {
        name: "a_path_that_is_no_repository_is_said_and_takes_nothing_down",
        run: a_path_that_is_no_repository_is_said_and_takes_nothing_down,
    },
    Test {
        name: "a_remembered_tab_whose_folder_is_gone_is_left_out",
        run: a_remembered_tab_whose_folder_is_gone_is_left_out,
    },
    Test {
        name: "keys_work_before_the_first_click",
        run: keys_work_before_the_first_click,
    },
    Test {
        name: "the_windows_controls_are_in_the_bar_they_belong_to",
        run: the_windows_controls_are_in_the_bar_they_belong_to,
    },
    Test {
        name: "the_toolbars_middle_group_stands_in_its_place_at_any_width",
        run: the_toolbars_middle_group_stands_in_its_place_at_any_width,
    },
    Test {
        name: "a_remembered_zoom_holds_from_the_start",
        run: a_remembered_zoom_holds_from_the_start,
    },
    Test {
        name: "a_setting_is_changed_in_its_row",
        run: a_setting_is_changed_in_its_row,
    },
    Test {
        name: "ctrl_f_reaches_the_search_from_any_focus",
        run: ctrl_f_reaches_the_search_from_any_focus,
    },
    Test {
        name: "escape_closes_the_palette",
        run: escape_closes_the_palette,
    },
    Test {
        name: "ctrl_z_in_a_text_field_takes_back_typing_not_a_git_action",
        run: ctrl_z_in_a_text_field_takes_back_typing_not_a_git_action,
    },
    Test {
        name: "home_and_end_in_the_changes_panel_go_to_the_first_and_last_file",
        run: home_and_end_in_the_changes_panel_go_to_the_first_and_last_file,
    },
    Test {
        name: "gitflow_is_set_up_in_a_dialog_of_rows_and_a_bugfix_started",
        run: gitflow_is_set_up_in_a_dialog_of_rows_and_a_bugfix_started,
    },
    Test {
        name: "a_submodule_is_added_on_a_branch_and_removed_with_or_without_its_clone",
        run: a_submodule_is_added_on_a_branch_and_removed_with_or_without_its_clone,
    },
    Test {
        name: "the_key_of_a_signed_commit_is_allowed_from_its_details_and_taken_out_again",
        run: the_key_of_a_signed_commit_is_allowed_from_its_details_and_taken_out_again,
    },
    Test {
        name: "cloning_and_a_new_repository_are_asked_in_forms_of_the_windows_own",
        run: cloning_and_a_new_repository_are_asked_in_forms_of_the_windows_own,
    },
    Test {
        name: "a_file_in_conflict_is_reached_and_resolved_with_the_keys",
        run: a_file_in_conflict_is_reached_and_resolved_with_the_keys,
    },
    Test {
        name: "the_colours_of_code_follow_the_theme_there_and_back",
        run: the_colours_of_code_follow_the_theme_there_and_back,
    },
    Test {
        name: "what_the_graph_shows_of_a_reference_is_changed_from_its_label",
        run: what_the_graph_shows_of_a_reference_is_changed_from_its_label,
    },
    Test {
        name: "a_click_on_the_selected_row_takes_the_selection_away",
        run: a_click_on_the_selected_row_takes_the_selection_away,
    },
    Test {
        name: "a_long_branch_name_is_cut_inside_its_row_and_what_stands_beside_it_stays",
        run: a_long_branch_name_is_cut_inside_its_row_and_what_stands_beside_it_stays,
    },
    Test {
        name: "a_key_is_rebound_by_pressing_it_and_is_not_run_meanwhile",
        run: a_key_is_rebound_by_pressing_it_and_is_not_run_meanwhile,
    },
    Test {
        name: "files_git_was_told_not_to_look_at_are_brought_back_by_key",
        run: files_git_was_told_not_to_look_at_are_brought_back_by_key,
    },
    Test {
        name: "a_signing_key_is_chosen_with_the_keys",
        run: a_signing_key_is_chosen_with_the_keys,
    },
    Test {
        name: "the_merge_tool_starts_from_what_was_resolved_last_time",
        run: the_merge_tool_starts_from_what_was_resolved_last_time,
    },
    Test {
        name: "a_refusing_hook_is_left_out_of_a_merge_and_a_push_only_when_switched",
        run: a_refusing_hook_is_left_out_of_a_merge_and_a_push_only_when_switched,
    },
    Test {
        name: "a_file_moved_in_the_index_is_shown_as_moved_not_as_new",
        run: a_file_moved_in_the_index_is_shown_as_moved_not_as_new,
    },
    Test {
        name: "the_versions_of_a_picture_are_laid_over_each_other_by_a_switch",
        run: the_versions_of_a_picture_are_laid_over_each_other_by_a_switch,
    },
];

/// The two versions of a picture stand one above the other. A switch
/// beside the upper one's name lays one over the other, the next switch
/// does it another way, and a switch pressed again sets them apart again:
/// one of them is on at most.
fn the_versions_of_a_picture_are_laid_over_each_other_by_a_switch() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let picture = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).diff().read(cx).probe().picture)
    };
    let switch = |stage: &mut Stage, which: usize| {
        stage.click(("image-mode", which));
        picture(stage)
    };

    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("assets/logo.png");
    // The switches are there once both versions are in.
    stage.until("both versions of the picture are in", |stage| {
        picture(stage).is_some() && stage.shows(("image-mode", 0usize))
    });
    assert_eq!(picture(&mut stage), Some("apart"));
    assert_eq!(switch(&mut stage, 0), Some("overlay"));
    assert_eq!(switch(&mut stage, 1), Some("swipe"));
    assert_eq!(switch(&mut stage, 1), Some("apart"));
    assert_eq!(switch(&mut stage, 2), Some("difference"));
    assert_eq!(switch(&mut stage, 0), Some("overlay"));
    assert_eq!(switch(&mut stage, 0), Some("apart"));
}

/// A file moved in the index is compared with what it was under its old
/// name; by its new name alone, which the last commit does not have, it
/// would all be added. Nothing is added when it only moved, and only what
/// was changed when it was changed as well.
fn a_file_moved_in_the_index_is_shown_as_moved_not_as_new() {
    repos::fresh();
    let repo = repos::project();
    repos::git(&repo, &["mv", "grammar.rs", "syntax.rs"]);
    let shown = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).diff().read(cx).probe())
    };

    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("syntax.rs");
    stage.until("the moved file is compared with its old name", |stage| {
        let probe = shown(stage);
        !probe.loading && probe.moved_from.is_some()
    });
    let probe = shown(&mut stage);
    assert_eq!(probe.moved_from.as_deref(), Some("grammar.rs"));
    assert_eq!((probe.added, probe.removed), (0, 0), "it only moved");
    drop(stage);

    // Moved and given one more line, both staged: one line is new.
    let mut text = std::fs::read_to_string(repo.join("syntax.rs")).unwrap();
    text.push_str("// one more line\n");
    std::fs::write(repo.join("syntax.rs"), text).unwrap();
    repos::git(&repo, &["add", "syntax.rs"]);
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.open_changed("syntax.rs");
    stage.until("the moved and changed file is compared", |stage| {
        let probe = shown(stage);
        !probe.loading && probe.moved_from.is_some()
    });
    let probe = shown(&mut stage);
    assert_eq!(probe.moved_from.as_deref(), Some("grammar.rs"));
    assert_eq!((probe.added, probe.removed), (1, 0), "one line was added");
}

/// A hook that would refuse a merge or a push is left out only when the
/// switch where each is started says so, and what went without it says
/// which hook that was.
fn a_refusing_hook_is_left_out_of_a_merge_and_a_push_only_when_switched() {
    repos::fresh();
    let repo = repos::project_with_refusing_hooks();
    let origin = repos::work().join("origin.git");
    let mut stage = Stage::open(Start::with(&repo));
    let merge = || RunRewrite {
        rewrite: Rewrite::MergeHow {
            target: "wip".to_owned(),
        },
    };
    let parents = |repo: &std::path::Path| {
        repos::git(repo, &["rev-list", "--parents", "-1", "HEAD"])
            .split_whitespace()
            .count()
            - 1
    };
    let said = |stage: &mut Stage, what: &str| {
        stage
            .notices()
            .iter()
            .filter(|said| said.contains(what))
            .count()
    };

    // As the dialog comes, the switch is there and off: the hook holds
    // the merge up, and it is left in progress with nothing committed.
    stage.act(merge());
    assert!(stage.dialog_open() && stage.shows(("form-check", 0usize)));
    stage.click("form-ok");
    stage.until("the merge has stopped", |stage| {
        stage.workspace().in_progress.is_some()
    });
    assert_eq!(stage.workspace().in_progress, Some(("merge".to_owned(), 0)));
    assert_eq!(parents(&repo), 1);
    assert_eq!(said(&mut stage, "without running"), 0);
    repos::git(&repo, &["merge", "--abort"]);
    stage.until("the merge is given up", |stage| {
        stage.workspace().in_progress.is_none()
    });
    stage.pass(Duration::from_secs(1));

    // Ticked: merged, and said with the name of the hook it went without.
    stage.act(merge());
    assert!(stage.dialog_open());
    stage.click(("form-check", 0usize));
    stage.click("form-ok");
    stage.until("the merge commit is made", |_| parents(&repo) == 2);
    stage.until("it says which hook it went without", |stage| {
        said(stage, "without running pre-merge-commit") == 1
    });
    assert!(stage.workspace().in_progress.is_none());

    // A push as it is: its hook refuses, and nothing arrives.
    stage.act(keymap::Push);
    stage.until("the push is refused by its hook", |stage| {
        said(stage, "pre-push hook of this repository stopped") == 1
    });
    assert_eq!(repos::git(&origin, &["for-each-ref"]), "");

    // Switched off, which is said: the push goes, and says what it went
    // without.
    stage.act(keymap::SkipPushHook);
    assert_eq!(said(&mut stage, "go without the pre-push hook"), 1);
    stage.act(keymap::Push);
    stage.until("the push arrives", |_| {
        !repos::git(&origin, &["for-each-ref", "refs/heads/main"]).is_empty()
    });
    stage.until("it says which hook it went without", |stage| {
        said(stage, "without running pre-push") == 1
    });
    assert_eq!(
        repos::git(&origin, &["rev-parse", "refs/heads/main"]),
        repos::git(&repo, &["rev-parse", "HEAD"])
    );

    // Switched back: the next push is refused again.
    stage.act(keymap::SkipPushHook);
    repos::git(&repo, &["commit", "-q", "--allow-empty", "-m", "one more"]);
    stage.act(keymap::Push);
    stage.until("the push is refused again", |stage| {
        said(stage, "pre-push hook of this repository stopped") == 2
    });
    assert_ne!(
        repos::git(&origin, &["rev-parse", "refs/heads/main"]),
        repos::git(&repo, &["rev-parse", "HEAD"])
    );
    assert_eq!(said(&mut stage, "without running pre-push"), 1);
}

fn a_repository_opens_and_comes_to_rest() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let shell = stage.shell();
    assert_eq!(shell.tabs.len(), 1);
    assert_eq!(shell.tabs[0].path.as_deref(), Some(repo.as_path()));
    assert_eq!(shell.tabs[0].title, "project");
    let workspace = stage.workspace();
    assert!(workspace.loaded, "{workspace:?}");
    assert_eq!(workspace.error, None);
    assert_eq!(workspace.branches, 6);
    assert_eq!(stage.graph().commits, 16);
    assert_eq!(stage.notices(), Vec::<String>::new());
}

/// A repository with uncommitted work opens on it: its row is selected and
/// the right panel holds the lists and the commit form. A clean one opens
/// on a commit.
fn a_repository_opens_on_its_uncommitted_work() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    assert!(stage.workspace().changes_shown);

    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let graph = stage.graph();
    assert!(
        !graph.wip_row && !graph.wip_selected,
        "nothing is uncommitted"
    );
    assert!(!stage.workspace().changes_shown);
}

/// The button of the commit form waits for a summary, then says how many
/// files it commits; with nothing staged it has nothing to commit and
/// cannot be pressed, unless the last commit is being amended.
fn the_commit_button_says_how_many_files_and_waits_for_them() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    let button = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).commit_box().read(cx).button())
    };
    assert_eq!(
        button(&mut stage),
        ("Type a summary to commit".to_owned(), false)
    );

    // Asked to commit without a summary, the form takes the keys.
    stage.keys("ctrl-enter");
    stage.type_text("guide: say how to search");
    stage.until("the one staged file is counted", |stage| {
        button(stage) == ("Commit 1 file".to_owned(), true)
    });

    // Nothing staged: nothing to commit.
    stage.focus_changes();
    stage.keys("shift-u");
    stage.until("the button waits for something staged", |stage| {
        button(stage) == ("Stage a file to commit".to_owned(), false)
    });

    // Everything staged: it says how many files that is.
    stage.keys("shift-s");
    stage.until("something is staged again", |stage| button(stage).1);
    let staged = repos::git(&repo, &["diff", "--cached", "--name-only"])
        .lines()
        .count();
    assert!(staged > 1, "{staged}");
    assert_eq!(button(&mut stage), (format!("Commit {staged} files"), true));
}

/// Started with a path that is no repository, the application says so and
/// shows the start page. The notification is asked for inside
/// `Shell::new`, before there is a window to say it in.
fn a_path_that_is_no_repository_is_said_and_takes_nothing_down() {
    repos::fresh();
    let folder = repos::work().join("nothing");
    std::fs::create_dir_all(&folder).unwrap();
    let file = repos::work().join("a-file.txt");
    std::fs::write(&file, "not a repository\n").unwrap();
    let missing = repos::work().join("not-there");
    for path in [&folder, &file, &missing] {
        let mut stage = Stage::open(Start::with(path));
        let shell = stage.shell();
        assert_eq!(shell.tabs.len(), 1, "{shell:?}");
        assert_eq!(shell.tabs[0].path, None, "the start page shows instead");
        let notices = stage.notices();
        assert_eq!(notices.len(), 1, "{notices:?}");
        assert!(
            notices[0].starts_with(&format!("Cannot open {}", path.display())),
            "{notices:?}"
        );
        // The window goes on working.
        stage.keys("ctrl-p");
        assert!(stage.shell().palette_open);
    }
}

/// A tab from the last time whose folder is gone is not opened, whichever
/// tab was the one shown; one whose folder is there but is no repository
/// any more is said.
fn a_remembered_tab_whose_folder_is_gone_is_left_out() {
    repos::fresh();
    let repo = repos::project();
    let gone = repos::work().join("gone");
    let emptied = repos::work().join("emptied");
    std::fs::create_dir_all(&emptied).unwrap();

    // The tab that was shown is the one that is gone.
    let state = format!("open = [{repo:?}, {gone:?}]\nactive = 1\n");
    let mut stage = Stage::open(Start::bare().state(state));
    let shell = stage.shell();
    assert_eq!(shell.tabs.len(), 1, "{shell:?}");
    assert_eq!(shell.tabs[0].path.as_deref(), Some(repo.as_path()));
    assert_eq!(shell.active, 0);
    assert!(stage.workspace().loaded);
    assert_eq!(stage.notices(), Vec::<String>::new());
    drop(stage);

    // Every tab is gone: the start page.
    let state = format!("open = [{gone:?}]\nactive = 0\n");
    let mut stage = Stage::open(Start::bare().state(state));
    let shell = stage.shell();
    assert_eq!(shell.tabs.len(), 1);
    assert_eq!(shell.tabs[0].path, None);
    drop(stage);

    // The folder is there and holds no repository any more.
    let state = format!("open = [{emptied:?}, {repo:?}]\nactive = 1\n");
    let mut stage = Stage::open(Start::bare().state(state));
    let shell = stage.shell();
    assert_eq!(shell.tabs.len(), 1, "{shell:?}");
    assert_eq!(shell.tabs[0].path.as_deref(), Some(repo.as_path()));
    let notices = stage.notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("emptied"), "{notices:?}");
}

/// The keys work before the first click: the graph has them when the
/// window opens, the window itself when nothing is open.
fn keys_work_before_the_first_click() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    assert_eq!(stage.graph_focus(), (true, false), "the graph has the keys");
    let first = stage.graph().selected.expect("a commit is selected");
    stage.keys("down");
    let second = stage.graph().selected.expect("a commit is selected");
    assert_ne!(first, second, "Down moved the selection");
    stage.keys("k");
    assert_eq!(stage.graph().selected, Some(first));
    stage.keys("ctrl-p");
    assert!(stage.shell().palette_open);
    drop(stage);

    // With nothing open the window itself has them.
    let mut stage = Stage::open(Start::bare());
    stage.keys("ctrl-p");
    assert!(stage.shell().palette_open);
    drop(stage);

    // And a view of the window's own has them when it is what is shown.
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-,");
    assert_eq!(stage.shell().sheet, Some("preferences"));
    stage.keys("escape");
    assert_eq!(stage.shell().sheet, None);
    assert_eq!(stage.graph_focus(), (true, false), "back in the graph");
}

/// The middle of a control, in points from the window's top left.
fn middle(stage: &mut Stage, id: &'static str) -> (f32, f32) {
    let (x, y, width, height) = stage.place(id);
    (x + width / 2.0, y + height / 2.0)
}

/// Whether two measures are the same to the point.
fn same(one: f32, other: f32) -> bool {
    (one - other).abs() < 0.75
}

/// The window's own controls are in the bars they belong to: the list of
/// tabs, the bell, the preferences and the profile at the right end of the
/// tabs, in that order and nothing else there; the palette beside the
/// search in the toolbar; the activity log and the keys in the status bar.
fn the_windows_controls_are_in_the_bar_they_belong_to() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));

    // The right end of the tabs: 33 from one to the next, the profile
    // last, and nothing else there.
    let list = middle(&mut stage, "tab-list");
    let bell = middle(&mut stage, "notices");
    let gear = middle(&mut stage, "preferences");
    assert!(same(bell.0 - list.0, 33.0), "{list:?} {bell:?}");
    assert!(same(gear.0 - bell.0, 33.0), "{bell:?} {gear:?}");
    let (profile, _, _, _) = stage.place("profiles");
    let (gear_left, _, gear_width, _) = stage.place("preferences");
    assert!(profile > gear_left + gear_width, "the profile comes last");
    for end in ["tab-list", "notices", "preferences", "profiles"] {
        assert!(middle(&mut stage, end).1 < 34.0, "{end} is in the tab bar");
    }
    for moved in ["refresh", "left-panel", "right-panel", "theme"] {
        assert!(!stage.shows(moved), "{moved} has no button any more");
    }

    // The list of tabs goes to the tab that is picked in it.
    stage.act(keymap::NewTab);
    assert_eq!(stage.shell().active, 1);
    stage.click("tab-list");
    stage.keys("down");
    stage.keys("enter");
    assert_eq!(stage.shell().active, 0, "the first tab of the list");

    // The palette is opened from the toolbar, beside the search, whose
    // middle is 26 from the window's edge.
    let palette = middle(&mut stage, "palette");
    let search = middle(&mut stage, "search");
    assert!(same(search.0, WIDTH - 26.0), "{search:?}");
    assert!(same(search.0 - palette.0, 52.0), "{palette:?} {search:?}");
    assert!(palette.1 > 34.0 && palette.1 < 82.0, "in the toolbar");
    stage.click("palette");
    assert!(stage.shell().palette_open);
    stage.keys("escape");
    assert!(!stage.shell().palette_open);

    // The activity log and the keys are opened from the status bar.
    let log = middle(&mut stage, "activity");
    let keys = middle(&mut stage, "cheat-sheet");
    assert!(log.1 > HEIGHT - 26.0 && keys.1 > HEIGHT - 26.0, "{log:?}");
    assert!(same(keys.0 - log.0, 33.0), "{log:?} {keys:?}");
    stage.click("activity");
    assert_eq!(stage.workspace().sheet, Some("activity"));
    stage.click("activity");
    assert_eq!(stage.workspace().sheet, None);
    stage.click("cheat-sheet");
    assert_eq!(stage.shell().sheet, Some("keys"));
}

/// The toolbar's middle group stands in the same place whatever the
/// window's width: the middle of Undo 244 left of the window's, the buttons
/// 52 apart, the caret 31 after Pull and Push 66 after it, and nothing
/// beside Push, whose other ways are the menu on the pointer's other
/// button.
fn the_toolbars_middle_group_stands_in_its_place_at_any_width() {
    repos::fresh();
    let repo = repos::project();
    let origin = repos::work().join("origin.git");
    repos::git(
        &repo,
        &["init", "-q", "--bare", origin.to_str().expect("a path")],
    );
    repos::git(
        &repo,
        &["remote", "add", "origin", origin.to_str().expect("a path")],
    );
    let mut stage = Stage::open(Start::with(&repo));

    let undo = middle(&mut stage, "undo");
    assert!(same(undo.0, WIDTH / 2.0 - 244.0), "{undo:?}");
    let mut before = undo.0;
    for (button, step) in [
        ("redo", 52.0),
        ("pull", 52.0),
        ("pull-more", 31.0),
        ("push", 35.0),
        ("new-branch", 52.0),
        ("stash", 52.0),
        ("pop", 52.0),
    ] {
        let at = middle(&mut stage, button).0;
        assert!(
            same(at - before, step),
            "{button} is at {at}, after {before}"
        );
        before = at;
    }
    assert!(!stage.shows("push-more"), "nothing stands beside Push");

    // The menu of the button: its first entry is the push itself, which
    // arrives.
    assert_eq!(repos::git(&origin, &["for-each-ref"]), "");
    stage.right_click("push");
    stage.keys("down");
    stage.keys("enter");
    stage.until("the push from the menu arrives", |_| {
        !repos::git(&origin, &["for-each-ref", "refs/heads/main"]).is_empty()
    });
}

/// The zoom that was remembered is the window's from the start, not only
/// once it is changed.
fn a_remembered_zoom_holds_from_the_start() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 1.5\n"));
    assert_eq!(stage.window(|window, _| window.rem_size()), px(24.));
    // It follows the keys from there.
    stage.keys("ctrl-=");
    let rem = f32::from(stage.window(|window, _| window.rem_size()));
    assert!((rem - 25.6).abs() < 0.01, "{rem}");
    stage.keys("ctrl-0");
    assert_eq!(stage.window(|window, _| window.rem_size()), px(16.));
    drop(stage);

    // A number no window can be drawn at is brought into range.
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 1000000.0\n"));
    let rem = f32::from(stage.window(|window, _| window.rem_size()));
    assert!((8.0..=48.0).contains(&rem), "{rem}");
    assert!(stage.workspace().loaded);
}

/// Every setting is a row of one form, with a control of its own kind: a
/// box that is ticked for a switch, and fields that are typed into. A
/// field is not filled again from its setting after every key, or the
/// comma and the blank after a protected branch's name would be gone as
/// they are typed and the next name would run into the last.
fn a_setting_is_changed_in_its_row() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::ShowPreferences);
    assert_eq!(stage.shell().sheet, Some("preferences"));
    let stashes =
        |stage: &mut Stage| stage.app(|cx| cx.global::<Prefs>().preferences.checkout_stash);
    assert!(!stashes(&mut stage));
    assert!(stage.shows("check-Stash on checkout"));
    stage.click("check-Stash on checkout");
    assert!(stashes(&mut stage), "the box was ticked");
    stage.click("check-Stash on checkout");
    assert!(!stashes(&mut stage), "and cleared again");

    // A name after the ones that are there, key by key.
    let protected = |stage: &mut Stage| {
        stage.app(|cx| cx.global::<Prefs>().preferences.protected_branches.clone())
    };
    assert_eq!(protected(&mut stage), ["main", "master"]);
    stage.click("field-Protected branches");
    stage.keys("end");
    for key in [",", " ", "t", "r", "u", "n", "k"] {
        stage.type_text(key);
    }
    assert_eq!(protected(&mut stage), ["main", "master", "trunk"]);

    // A number is typed like anything else; past the most it is the most.
    let stale = |stage: &mut Stage| stage.app(|cx| cx.global::<Prefs>().preferences.stale_days);
    assert_eq!(stale(&mut stage), 90);
    stage.click("number-A branch is stale after, in days");
    stage.keys("end");
    stage.type_text("0");
    assert_eq!(stale(&mut stage), 900);
    stage.type_text("0");
    assert_eq!(stale(&mut stage), 3650);
}

/// Ctrl+F opens the search wherever the focus is.
fn ctrl_f_reaches_the_search_from_any_focus() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    let searching = |stage: &mut Stage, from: &str| {
        stage.keys("ctrl-f");
        let graph = stage.graph();
        assert!(graph.search_open, "from {from}: {graph:?}");
        assert_eq!(stage.graph_focus(), (false, true), "from {from}");
        // Typing lands in the field.
        stage.type_text("lexer");
        stage.pass(Duration::from_millis(500));
        let graph = stage.graph();
        assert_eq!(graph.search_text, "lexer", "from {from}");
        assert!(graph.matches >= 2, "from {from}: {graph:?}");
        // Escape gives the keys back to the graph and leaves what was
        // found on show; pressed again there, it puts the bar away.
        stage.keys("escape");
        assert_eq!(stage.graph_focus(), (true, false), "from {from}");
        assert!(stage.graph().search_open, "from {from}");
        stage.keys("escape");
        let graph = stage.graph();
        assert!(
            !graph.search_open && graph.search_text.is_empty(),
            "from {from}: {graph:?}"
        );
        assert_eq!(stage.graph_focus(), (true, false), "from {from}");
    };

    searching(&mut stage, "the graph");
    // And so does its button.
    stage.keys("ctrl-f");
    stage.type_text("lexer");
    stage.pass(Duration::from_millis(500));
    stage.click("search-close");
    let graph = stage.graph();
    assert!(
        !graph.search_open && graph.search_text.is_empty(),
        "{graph:?}"
    );

    stage.keys("ctrl-alt-f");
    assert_eq!(stage.graph_focus(), (false, false));
    searching(&mut stage, "the filter of the references panel");

    stage.keys("ctrl-home");
    assert!(stage.graph().wip_selected);
    stage.focus_changes();
    assert_eq!(
        stage.graph_focus(),
        (false, false),
        "the changes have the keys"
    );
    searching(&mut stage, "the changes panel");
}

/// Escape closes the palette.
fn escape_closes_the_palette() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let first = stage.graph().selected;
    stage.keys("ctrl-p");
    assert!(stage.shell().palette_open);
    stage.type_text("theme");
    stage.keys("escape");
    assert!(!stage.shell().palette_open, "Escape closes it");
    // The keys are the graph's again.
    assert_eq!(stage.graph_focus(), (true, false));
    stage.keys("down");
    assert_ne!(stage.graph().selected, first);
    // And it opens again, empty.
    stage.keys("ctrl-p");
    assert!(stage.shell().palette_open);
    // Chosen with Enter, an entry runs and the palette goes.
    stage.type_text("Keyboard shortcuts");
    stage.keys("enter");
    let shell = stage.shell();
    assert!(!shell.palette_open, "{shell:?}");
    assert_eq!(shell.sheet, Some("keys"));
}

/// Ctrl+Z in a text field takes back what was typed, not the last git
/// action.
fn ctrl_z_in_a_text_field_takes_back_typing_not_a_git_action() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    // Something git did that undo would take back: a new branch.
    stage.act(keymap::NewBranch);
    assert!(stage.graph().naming.is_some(), "named in the graph");
    stage.type_text("feature/wrap");
    stage.keys("enter");
    stage.until("the new branch is among the references", |stage| {
        stage.workspace().branches == 7
    });
    assert!(repos::git(&repo, &["branch", "--list", "feature/wrap"]).contains("feature/wrap"));
    // The dialog that closed hands the focus back a moment later.
    stage.pass(Duration::from_secs(1));

    // In the search field, Ctrl+Z is about the text.
    stage.keys("ctrl-f");
    assert_eq!(stage.graph_focus(), (false, true));
    stage.type_text("lexer");
    stage.pass(Duration::from_secs(1));
    assert_eq!(
        stage.graph_focus(),
        (false, true),
        "the cursor stays in the field"
    );
    assert_eq!(stage.graph().search_text, "lexer");
    stage.keys("ctrl-z");
    let graph = stage.graph();
    let text = &graph.search_field;
    assert!(
        text.len() < "lexer".len() && "lexer".starts_with(text.as_str()),
        "typing was taken back: {graph:?}"
    );
    assert_eq!(&graph.search_text, text, "the search follows the field");
    assert_eq!(stage.workspace().branches, 7, "and the branch was not");
    assert!(repos::git(&repo, &["branch", "--list", "feature/wrap"]).contains("feature/wrap"));

    // In the graph it is about git: the branch goes.
    stage.keys("escape");
    assert_eq!(stage.graph_focus(), (true, false));
    stage.keys("ctrl-z");
    stage.until("the branch is gone again", |stage| {
        stage.workspace().branches == 6
    });
    assert_eq!(repos::git(&repo, &["branch", "--list", "feature/wrap"]), "");
}

/// With no file under the cursor yet, Home goes to the first file and End
/// to the last.
fn home_and_end_in_the_changes_panel_go_to_the_first_and_last_file() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let opened = |keys: &str| {
        let mut stage = Stage::open(Start::with(&repo));
        stage.keys("ctrl-home");
        assert!(stage.graph().wip_selected);
        stage.focus_changes();
        stage.keys(keys);
        stage.keys("enter");
        stage.workspace().file.map(|(path, _)| path)
    };
    // The unstaged files come first, by path; the staged one is last.
    assert_eq!(opened("home").as_deref(), Some("assets/logo.png"));
    assert_eq!(opened("end").as_deref(), Some("docs/guide.md"));
    assert_eq!(opened("down").as_deref(), Some("assets/logo.png"));
    assert_eq!(opened("up").as_deref(), Some("docs/guide.md"));
    assert_eq!(opened("down down").as_deref(), Some("grammar.rs"));
    assert_eq!(opened("end home").as_deref(), Some("assets/logo.png"));
}

/// Gitflow's two branches and six prefixes are a section of the
/// preferences, each a row. What is typed is looked at as a whole, the
/// button waits while it will not do, and the branch of a kind chosen in
/// the dialog that starts one is started under its new prefix.
fn gitflow_is_set_up_in_a_dialog_of_rows_and_a_bugfix_started() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::FlowSettings);
    assert_eq!(stage.shell().preferences_section, Some("gitflow"));
    assert_eq!(
        stage.shell().flow_fields,
        [
            "main", "develop", "feature/", "bugfix/", "release/", "hotfix/", "support/", ""
        ],
        "the settings as they would usually be"
    );

    // The fourth field is the bugfix prefix. Two kinds cannot share a
    // prefix: the button waits and nothing is written.
    stage.click("flow-3");
    stage.keys("ctrl-a");
    stage.type_text("feature/");
    assert_eq!(stage.shell().flow_fields[3], "feature/");
    stage.click("flow-save");
    stage.pass(Duration::from_millis(300));
    assert_eq!(stage.workspace().branches, 6, "no develop branch yet");
    stage.click("flow-3");
    stage.keys("ctrl-a");
    stage.type_text("fix/");
    // The last row may stay empty; it is given something all the same.
    stage.click("flow-7");
    stage.type_text("v");
    let fields = stage.shell().flow_fields;
    assert_eq!((fields[3].as_str(), fields[7].as_str()), ("fix/", "v"));
    stage.click("flow-save");
    stage.until("the develop branch is made", |stage| {
        stage.workspace().branches == 7
    });
    assert_eq!(
        repos::git(&repo, &["config", "--get-regexp", r"^gitflow\."]),
        "gitflow.branch.master main\n\
         gitflow.branch.develop develop\n\
         gitflow.prefix.feature feature/\n\
         gitflow.prefix.bugfix fix/\n\
         gitflow.prefix.release release/\n\
         gitflow.prefix.hotfix hotfix/\n\
         gitflow.prefix.support support/\n\
         gitflow.prefix.versiontag v\n"
    );
    stage.pass(Duration::from_secs(1));

    // The next time the section says what is set.
    stage.click("preferences-close");
    stage.act(keymap::FlowSettings);
    stage.until("the settings are read back", |stage| {
        stage.shell().flow_fields.get(3).map(String::as_str) == Some("fix/")
    });
    assert_eq!(stage.shell().flow_fields[7], "v");
    stage.click("preferences-close");
    stage.pass(Duration::from_secs(1));

    stage.act(keymap::FlowStart);
    assert!(stage.dialog_open(), "the dialog that starts a branch");
    stage.type_text("crash");
    // The second of the five kinds is the bugfix.
    stage.click_within("form-pick", 1usize);
    stage.click("form-ok");
    stage.until("the bugfix is among the branches", |stage| {
        stage.workspace().branches == 8
    });
    assert_eq!(
        repos::git(&repo, &["symbolic-ref", "--short", "HEAD"]),
        "fix/crash\n",
        "under the prefix that was set, and checked out"
    );
    assert_eq!(
        repos::git(&repo, &["rev-parse", "fix/crash"]),
        repos::git(&repo, &["rev-parse", "develop"]),
        "from develop"
    );
}

/// A submodule is added from the pane that also asks for the branch it
/// follows; removing it asks whether the clone git keeps of it goes too,
/// and a clone that was kept is offered to be deleted when it is in the
/// way of the next submodule at that place.
fn a_submodule_is_added_on_a_branch_and_removed_with_or_without_its_clone() {
    repos::fresh();
    let repo = repos::project();
    let library = repos::library();
    let stable = repos::rev(&library, "stable");
    let newest = repos::rev(&library, "main");
    assert_ne!(stable, newest);
    let kept = repo.join(".git/modules/vendor/library");
    let mut stage = Stage::open(Start::with(&repo));
    let ask = |stage: &mut Stage, request: SubmoduleRequest| {
        // The requests are the references panel's: its menus make them.
        stage.act(keymap::FocusReferences);
        let adds = request == SubmoduleRequest::Add;
        stage.act(RunSubmodule { request });
        if adds {
            assert_eq!(stage.workspace().docked, Some("submodule"), "the pane");
        } else {
            assert!(stage.dialog_open(), "the dialog of the request");
        }
    };
    let recorded = |repo: &std::path::Path| repos::rev(&repo.join("vendor/library"), "HEAD");

    // Added on a branch: three fields, the last of which may stay empty.
    ask(&mut stage, SubmoduleRequest::Add);
    assert_eq!(stage.workspace().docked_fields, ["", "", ""]);
    stage.type_text(library.to_str().unwrap());
    stage.keys("tab");
    stage.type_text("vendor/library/");
    stage.keys("tab");
    stage.type_text("stable");
    stage.keys("enter");
    stage.until("the submodule is cloned", |_| {
        repo.join("vendor/library/lib.txt").is_file()
    });
    stage.settle();
    assert_eq!(recorded(&repo), stable, "at the branch that was named");
    assert_eq!(
        repos::git(
            &repo,
            &[
                "config",
                "-f",
                ".gitmodules",
                "submodule.vendor/library.branch"
            ]
        ),
        "stable\n"
    );
    repos::git(&repo, &["commit", "-q", "-m", "take the library in"]);
    stage.pass(Duration::from_secs(1));

    // Removed as git removes it: the switch is off, the clone stays.
    ask(
        &mut stage,
        SubmoduleRequest::Remove("vendor/library".to_owned()),
    );
    stage.click("form-ok");
    stage.until("the submodule is gone", |_| {
        !repo.join("vendor/library").exists()
    });
    stage.settle();
    assert!(kept.join("HEAD").is_file(), "the clone is kept");
    repos::git(&repo, &["commit", "-q", "-m", "without the library"]);
    stage.pass(Duration::from_secs(1));

    // Another one at that place: the kept clone is in the way, which is
    // asked about, and with a yes it is deleted and the new one added.
    ask(&mut stage, SubmoduleRequest::Add);
    stage.type_text(library.to_str().unwrap());
    stage.keys("tab");
    stage.type_text("vendor/library");
    stage.keys("enter");
    stage.until("the question about the kept clone, in the bar", |stage| {
        stage.shell().bar.is_some() && stage.workspace().docked.is_none()
    });
    assert!(
        !repo.join("vendor/library").exists(),
        "nothing was added yet"
    );
    // Enter goes ahead in the bar.
    stage.keys("enter");
    stage.until("the submodule is cloned anew", |_| {
        repo.join("vendor/library/lib.txt").is_file()
    });
    stage.settle();
    assert_eq!(
        recorded(&repo),
        newest,
        "a new clone, at the default branch"
    );
    repos::git(&repo, &["commit", "-q", "-m", "the library again"]);
    stage.pass(Duration::from_secs(1));

    // Removed for good: with the switch on, the clone goes too.
    ask(
        &mut stage,
        SubmoduleRequest::Remove("vendor/library".to_owned()),
    );
    stage.click(("form-check", 0usize));
    stage.click("form-ok");
    stage.until("the submodule and its clone are gone", |_| {
        !repo.join("vendor/library").exists() && !kept.exists()
    });
    stage.settle();
    assert!(
        repos::git(&repo, &["status", "--porcelain"]).contains("D  vendor/library"),
        "the removal is staged"
    );
    assert_eq!(
        stage.notices().last().map(String::as_str),
        Some("Removed the submodule vendor/library and the clone git kept of it; that is staged")
    );
}

/// A commit signed with an SSH key nobody vouches for offers, in its
/// details, to allow that key. With no allowed-signers file chosen the file
/// is asked for first; then the signature is good, the signing view lists
/// the signer, and taken out there the signature is what it was. The key is
/// made here, beside the repository, and no agent and no file of anybody's
/// home is asked.
fn the_key_of_a_signed_commit_is_allowed_from_its_details_and_taken_out_again() {
    if !repos::on_path("ssh-keygen") {
        println!("skipped: ssh-keygen is not installed");
        return;
    }
    repos::fresh();
    let repo = repos::project();
    let key = repos::ssh_key("ada");
    repos::git(&repo, &["config", "gpg.format", "ssh"]);
    repos::git(
        &repo,
        &[
            "config",
            "user.signingkey",
            &format!("{}.pub", key.display()),
        ],
    );
    repos::git(
        &repo,
        &["commit", "-q", "--allow-empty", "-S", "-m", "Signed by Ada"],
    );
    let signed = repos::rev(&repo, "HEAD");
    let file = repos::work().join("trust/allowed_signers");

    let mut stage = Stage::open(Start::with(&repo));
    assert_eq!(stage.graph().selected.as_deref(), Some(signed.as_str()));
    // No file says whose key is whose: git cannot check it, and says so.
    stage.until("the signature has been looked at", |stage| {
        stage.workspace().signature.is_some()
    });
    let (state, signer) = stage.workspace().signature.unwrap();
    assert_eq!((state.as_str(), signer), ("cannot_check", None));
    assert!(stage.shows("allow-signer"), "the offer to allow the key");

    // Asked for from the details: first where the signers are kept.
    stage.click("allow-signer");
    stage.until("the dialog for the file", |stage| {
        stage.dialog_open() && stage.workspace().form_rows == [""]
    });
    stage.type_text(file.to_str().unwrap());
    stage.keys("enter");
    // Then for whom the key counts: whoever committed, unless said.
    stage.until("the dialog for the signer", |stage| {
        stage.dialog_open() && stage.workspace().form_rows == ["ada@fixture.invalid"]
    });
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "", "made, empty");
    assert_eq!(
        repos::git(&repo, &["config", "--local", "gpg.ssh.allowedSignersFile"]),
        format!("{}\n", file.display())
    );
    stage.keys("enter");
    stage.until("the signature is good", |stage| {
        stage.workspace().signature
            == Some(("good".to_owned(), Some("ada@fixture.invalid".to_owned())))
    });
    assert!(!stage.shows("allow-signer"), "nothing left to allow");
    assert_eq!(
        repos::git(&repo, &["log", "-1", "--format=%G? %GS"]),
        "G ada@fixture.invalid\n",
        "git's own verdict"
    );
    stage.pass(Duration::from_secs(1));

    // The signing view lists who may sign, and takes them out again.
    stage.act(keymap::ShowSigning);
    stage.until("the signer is listed", |stage| {
        stage.workspace().allowed_signers == ["ada@fixture.invalid"]
    });
    stage.click(("signer-remove", 0usize));
    assert!(stage.dialog_open(), "the question before a signer goes");
    stage.keys("enter");
    stage.until("nobody is allowed any more", |stage| {
        let workspace = stage.workspace();
        workspace.allowed_signers.is_empty()
            && workspace.signature.map(|(state, _)| state) == Some("untrusted".to_owned())
    });
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "");
    stage.pass(Duration::from_secs(1));

    // From the view's own button: a key file, and who it belongs to.
    stage.click("signers-add");
    stage.until("the dialog for a signer and a key", |stage| {
        stage.dialog_open() && stage.workspace().form_rows.len() == 3
    });
    stage.type_text("ada@fixture.invalid");
    // Neither a file nor a key yet: it stays open.
    stage.keys("enter");
    assert!(stage.dialog_open());
    stage.keys("tab");
    stage.type_text(&format!("{}.pub", key.display()));
    stage.keys("enter");
    stage.until("allowed again, from the file", |stage| {
        let workspace = stage.workspace();
        workspace.allowed_signers == ["ada@fixture.invalid"]
            && workspace.signature.map(|(state, _)| state) == Some("good".to_owned())
    });
}

/// The window asks what to clone and where a new repository goes before
/// there is a tab to ask in, in the large form of its own: the folder
/// follows the address until it is edited, a shallow clone's branch and
/// depth show once it is asked for, what cannot be is said in the form
/// and keeps it open, and what is made opens in a tab. A new repository
/// is a name in a folder, and the form shows the full path the two make.
fn cloning_and_a_new_repository_are_asked_in_forms_of_the_windows_own() {
    repos::fresh();
    let library = repos::library();
    // Where clones go when nothing was said before: a folder of this run.
    let into = repos::work().join("clones");
    std::fs::create_dir_all(&into).unwrap();
    let state = format!("clone_dir = {into:?}\n");
    let mut stage = Stage::open(Start::bare().state(state));

    stage.click("clone");
    assert!(stage.dialog_open(), "the form that clones");
    // Branch and depth are a shallow clone's, and stand hidden.
    assert_eq!(stage.shell().form_rows, ["", "", "", ""]);
    assert!(!stage.shows(("form-row", 2usize)) && !stage.shows(("form-row", 3usize)));
    assert!(stage.shows("form-close") && !stage.shows("form-cancel"));
    // The keyboard starts where the address goes, and the folder is
    // suggested from it.
    stage.type_text(library.to_str().unwrap());
    let cloned = into.join("library");
    let rows = stage.shell().form_rows;
    assert_eq!(rows[1], library.to_str().unwrap());
    assert_eq!(rows[0], cloned.to_str().unwrap(), "the folder follows");
    // Shallow: the depth starts at 50. A depth that is no number is said
    // and keeps the form open.
    stage.click(("form-check", 0usize));
    assert!(stage.shows(("form-row", 2usize)) && stage.shows(("form-row", 3usize)));
    assert_eq!(stage.shell().form_rows[3], "50");
    stage.click(("form-row", 3usize));
    stage.keys("ctrl-a");
    stage.type_text("deep");
    assert_eq!(stage.shell().form_rows[3], "deep");
    stage.keys("enter");
    assert!(stage.dialog_open(), "it stays while it will not do");
    assert!(!cloned.exists() && !stage.shell().creating);
    // Not shallow after all: what the hidden rows hold is not asked for.
    stage.click(("form-check", 0usize));
    assert_eq!(stage.shell().form_rows[3], "");
    // A folder that was typed is left alone by what is typed after it.
    stage.click(("form-row", 0usize));
    stage.keys("ctrl-a");
    let elsewhere = into.join("the-library");
    stage.type_text(elsewhere.to_str().unwrap());
    assert_eq!(stage.shell().form_rows[0], elsewhere.to_str().unwrap());
    stage.click(("form-row", 1usize));
    stage.keys("ctrl-a");
    stage.type_text(&format!("{}/", library.display()));
    assert_eq!(stage.shell().form_rows[0], elsewhere.to_str().unwrap());
    stage.keys("enter");
    stage.until("the clone is open in a tab", |stage| {
        let shell = stage.shell();
        !shell.creating
            && shell
                .tabs
                .iter()
                .any(|tab| tab.path.as_deref() == Some(elsewhere.as_path()))
    });
    assert!(!stage.dialog_open());
    assert_eq!(
        repos::rev(&elsewhere, "HEAD"),
        repos::rev(&library, "HEAD"),
        "the same history"
    );
    stage.pass(Duration::from_secs(1));
    drop(stage);

    // A new repository: a name in the folder clones go to, the full path
    // shown as it is typed, a template chosen from the list, and a first
    // commit.
    let state = format!("clone_dir = {into:?}\n");
    let mut stage = Stage::open(Start::bare().state(state));
    stage.click("init");
    assert!(stage.dialog_open(), "the form for a new repository");
    assert_eq!(
        stage.shell().form_rows,
        ["", into.to_str().unwrap(), "", ""]
    );
    // A name that is no folder's name is said, and nothing is made.
    stage.type_text("a/b");
    assert_eq!(stage.shell().form_rows[2], "");
    stage.keys("enter");
    assert!(stage.dialog_open());
    stage.keys("ctrl-a");
    stage.type_text("new-repository");
    let fresh = into.join("new-repository");
    assert_eq!(stage.shell().form_rows[2], fresh.to_str().unwrap());
    stage.click(("form-row", 3usize));
    stage.type_text("trunk");
    assert_eq!(stage.shell().form_rows[3], "trunk");
    // The templates: none, then each by its name.
    stage.click("form-choose");
    assert!(stage.shows("popup-menu"), "the templates to start from");
    stage.keys("down down enter");
    stage.click("form-ok");
    stage.until("the new repository is open in a tab", |stage| {
        let shell = stage.shell();
        !shell.creating
            && shell
                .tabs
                .iter()
                .any(|tab| tab.path.as_deref() == Some(fresh.as_path()))
    });
    assert_eq!(
        repos::git(&fresh, &["symbolic-ref", "--short", "HEAD"]),
        "trunk\n"
    );
    assert_eq!(
        repos::git(&fresh, &["ls-files"]),
        ".gitignore\n",
        "a first commit with what the template starts it from"
    );
    assert!(
        !std::fs::read_to_string(fresh.join(".gitignore"))
            .unwrap()
            .is_empty()
    );
}

/// The row of a file in conflict is reached with the keys of the changes
/// panel like the rows under it, and has keys of its own: Enter opens it
/// to be resolved, the menu key opens its menu, and it is resolved with
/// our side, with theirs, or as it is.
fn a_file_in_conflict_is_reached_and_resolved_with_the_keys() {
    let ours = "Search for a needle.\nUse single quotes around several words.\n";
    let theirs = "Search for a needle.\nUse double quotes around several words.\nSee the README.\n";
    let keys_on = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).changes().read(cx).keys_on())
    };
    let menu_open = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| {
            workspace.read(cx).changes().read(cx).key_menu_open()
        })
    };
    let refusal = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| {
            workspace
                .read(cx)
                .changes()
                .read(cx)
                .refusal()
                .map(str::to_owned)
        })
    };
    let in_conflict = Some((true, "docs/guide.md".to_owned()));

    repos::fresh();
    let repo = repos::project_in_conflict();
    let guide = repo.join("docs/guide.md");
    // A file that is in no conflict, for the list under the conflicts.
    std::fs::write(repo.join("notes.txt"), "to do\n").unwrap();
    let mut stage = Stage::open(Start::with(&repo));
    assert_eq!(stage.workspace().in_progress, Some(("merge".to_owned(), 1)));
    stage.keys("ctrl-home");
    assert!(stage.graph().wip_selected);
    stage.focus_changes();
    assert_eq!(keys_on(&mut stage), None);
    // Down from nowhere is the first row there is: the file in conflict.
    stage.keys("down");
    assert_eq!(keys_on(&mut stage), in_conflict);
    // No further up than that; End is the last of the files under it,
    // and Home is the conflict again.
    stage.keys("up");
    assert_eq!(keys_on(&mut stage), in_conflict);
    stage.keys("end");
    assert_eq!(keys_on(&mut stage), Some((false, "notes.txt".to_owned())));
    stage.focus_changes();
    stage.keys("home");
    assert_eq!(keys_on(&mut stage), in_conflict);

    // The menu key opens its menu at the row; Escape puts it away and
    // the keys are the panel's again.
    stage.keys("shift-f10");
    assert!(menu_open(&mut stage), "the menu of the file in conflict");
    stage.keys("escape");
    stage.pass(Duration::from_secs(1));
    assert!(!menu_open(&mut stage));
    assert_eq!(
        keys_on(&mut stage),
        in_conflict,
        "the keys are where they were"
    );

    // Marked resolved as it is, while the markers are still in it: that
    // is refused, and said.
    assert_eq!(refusal(&mut stage), None);
    stage.keys("r");
    stage.until("the refusal is said", |stage| refusal(stage).is_some());
    assert!(std::fs::read_to_string(&guide).unwrap().contains("<<<<<<<"));
    assert_eq!(stage.workspace().in_progress, Some(("merge".to_owned(), 1)));

    // Enter opens it to be resolved: the merge tool, which has the
    // window's width until it is left.
    stage.focus_changes();
    stage.keys("enter");
    let workspace = stage.workspace();
    assert_eq!(workspace.file, Some(("docs/guide.md".to_owned(), None)));
    assert_eq!(workspace.file_tab, "merge");
    stage.click("merge-close");
    stage.pass(Duration::from_secs(1));

    // Our side.
    stage.focus_changes();
    stage.keys("o");
    stage.until("nothing is in conflict any more", |stage| {
        stage.workspace().in_progress == Some(("merge".to_owned(), 0))
    });
    assert_eq!(std::fs::read_to_string(&guide).unwrap(), ours);
    assert_eq!(
        keys_on(&mut stage).map(|(conflict, _)| conflict),
        None,
        "no file in conflict is left for the keys to be on"
    );
    drop(stage);

    // And theirs, in a merge that stopped the same way.
    repos::fresh();
    let repo = repos::project_in_conflict();
    let guide = repo.join("docs/guide.md");
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.focus_changes();
    stage.keys("home h");
    stage.until("nothing is in conflict any more", |stage| {
        stage.workspace().in_progress == Some(("merge".to_owned(), 0))
    });
    assert_eq!(std::fs::read_to_string(&guide).unwrap(), theirs);
    // The keys that are about a conflict do nothing on another file.
    stage.focus_changes();
    stage.keys("down");
    assert_eq!(
        keys_on(&mut stage).map(|(conflict, _)| conflict),
        Some(false)
    );
    stage.keys("o h r");
    assert_eq!(std::fs::read_to_string(&guide).unwrap(), theirs);
    assert_eq!(refusal(&mut stage), None);
}

/// The colours of code follow the theme. The first three of our themes
/// bring no colours of code of their own, and the toolkit keeps the last
/// it had: after dark, light and dark again an editor would stand white in
/// a dark window. The others bring theirs, and a theme without them after
/// one of those has those of its mode again.
fn the_colours_of_code_follow_the_theme_there_and_back() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    // The ground an editor stands on, and whether the window is dark.
    let dark = |stage: &mut Stage| {
        stage.app(|cx| {
            let theme = cx.global::<Theme>();
            let ground = theme
                .highlight_theme
                .style
                .editor_background
                .map(|colour| colour.l < 0.5);
            (
                theme.is_dark(),
                theme.highlight_theme.appearance.is_dark(),
                ground,
            )
        })
    };
    assert_eq!(dark(&mut stage), (true, true, Some(true)), "as it starts");
    for (theme, is_dark, own) in [
        (ThemeChoice::Light, false, false),
        (ThemeChoice::Dark, true, false),
        (ThemeChoice::Light, false, false),
        (ThemeChoice::Contrast, true, false),
        (ThemeChoice::Dark, true, false),
        (ThemeChoice::Paper, false, true),
        (ThemeChoice::Ember, true, true),
        (ThemeChoice::Light, false, false),
        (ThemeChoice::Sage, false, true),
        (ThemeChoice::Dark, true, false),
    ] {
        stage.use_theme(&theme);
        stage.frame();
        assert_eq!(
            dark(&mut stage),
            (is_dark, is_dark, Some(is_dark)),
            "after going to {theme:?}"
        );
        let name = stage.app(|cx| cx.global::<Theme>().highlight_theme.name.clone());
        assert_eq!(
            name.starts_with("BitBraid"),
            own,
            "after going to {theme:?}: the colours of code are {name}"
        );
    }
}

/// The menu of a label in the graph hides, shows alone and pins the
/// reference it names: its entries are the references panel's, and what
/// they do arrives there from the graph, which has the keys when a label's
/// menu is used. The graph is walked again each time.
fn what_the_graph_shows_of_a_reference_is_changed_from_its_label() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    assert_eq!(stage.graph().commits, 16);
    assert_eq!(stage.graph_focus(), (true, false), "the graph has the keys");
    let kept = |stage: &mut Stage| {
        let repo = repo.clone();
        stage.app(move |cx| {
            let state = cx.global::<Prefs>().repo_state(&repo);
            (
                state.hidden.clone(),
                state.only.clone(),
                state.pinned.clone(),
            )
        })
    };
    let wip = "refs/heads/wip".to_owned();
    let pages = "refs/heads/pages".to_owned();

    // Hidden: the commit that is only on that branch goes with it.
    stage.act(ToggleHidden {
        full_name: wip.clone(),
    });
    stage.until("the branch is out of the graph", |stage| {
        stage.graph().commits == 15
    });
    assert_eq!(kept(&mut stage).0, std::slice::from_ref(&wip));
    stage.act(ToggleHidden {
        full_name: wip.clone(),
    });
    stage.until("and in it again", |stage| stage.graph().commits == 16);
    assert!(kept(&mut stage).0.is_empty());

    // Shown alone: only what leads to that branch is left.
    stage.act(ToggleSolo {
        full_name: pages.clone(),
    });
    stage.until("only its history is shown", |stage| {
        let commits = stage.graph().commits;
        commits > 0 && commits < 16
    });
    assert_eq!(kept(&mut stage).1, std::slice::from_ref(&pages));
    stage.act(ToggleSolo {
        full_name: pages.clone(),
    });
    stage.until("everything is shown again", |stage| {
        stage.graph().commits == 16
    });

    // Pinned, and let go.
    stage.act(TogglePinned {
        full_name: pages.clone(),
    });
    stage.until("the branch is pinned", |stage| {
        kept(stage).2 == [pages.clone()]
    });
    assert_eq!(
        stage.graph().commits,
        16,
        "a pin moves a line, it hides none"
    );
    stage.act(TogglePinned {
        full_name: pages.clone(),
    });
    stage.until("and let go", |stage| kept(stage).2.is_empty());
}

/// A click on a commit's row selects it; a click on the row that is
/// selected takes the selection away again, and it stays away when the
/// graph is read anew. Nothing is selected then: the window goes on, and
/// the next click selects.
fn a_click_on_the_selected_row_takes_the_selection_away() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    assert_eq!(stage.graph().commits, 16);

    // A row is selected by one click (by two, had it been the one that
    // was selected when the repository opened).
    let row = stage.graph_spot(3, false);
    stage.click_at(row);
    if stage.graph().selected.is_none() {
        stage.click_at(row);
    }
    let third = stage.graph().selected;
    assert!(third.is_some(), "a click selects the row");

    // The same row again: nothing is selected.
    stage.click_at(row);
    let graph = stage.graph();
    assert_eq!((graph.selected, graph.picked), (None, 0));
    assert!(!graph.wip_selected);
    assert!(stage.workspace().loaded);
    assert_eq!(stage.notices(), Vec::<String>::new());

    // The graph is read again (a branch is hidden and shown): still nothing.
    let wip = "refs/heads/wip".to_owned();
    stage.act(ToggleHidden {
        full_name: wip.clone(),
    });
    stage.until("the branch is out of the graph", |stage| {
        stage.graph().commits == 15
    });
    assert_eq!(stage.graph().selected, None);
    stage.act(ToggleHidden { full_name: wip });
    stage.until("and in it again", |stage| stage.graph().commits == 16);
    assert_eq!(stage.graph().selected, None);

    // And a click selects again, the same row as before.
    let row = stage.graph_spot(3, false);
    stage.click_at(row);
    assert_eq!(stage.graph().selected, third);
}

/// A branch whose name is longer than the panel has room for is cut
/// inside its row, and how far it is ahead and behind stays whole beside
/// it.
fn a_long_branch_name_is_cut_inside_its_row_and_what_stands_beside_it_stays() {
    const LONG: &str = "a-very-long-branch-name-that-does-not-fit-in-the-panel";
    repos::fresh();
    let repo = repos::project_with_everything();
    // One commit behind what it follows, so that there is something to
    // say at the right of its row.
    repos::git(&repo, &["branch", LONG, "origin/main~1"]);
    repos::git(
        &repo,
        &["branch", "-q", "--set-upstream-to=origin/main", LONG],
    );
    // Every section starts out closed: the branches are opened.
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    let bounds = |stage: &mut Stage, id: &str| {
        let id = ElementId::Name(id.to_owned().into());
        stage.window(|window, _| {
            window
                .try_find(id)
                .filter(|found| found.visible())
                .map(|found| found.bounds())
        })
    };
    let name = format!("ref-name:refs/heads/{LONG}");
    let aside = format!("ref-aside:refs/heads/{LONG}");
    stage.until("the row says how far the branch is behind", |stage| {
        bounds(stage, &aside).is_some_and(|aside| aside.size.width > px(0.))
    });
    let name = bounds(&mut stage, &name).expect("the name of the branch is on show");
    let aside = bounds(&mut stage, &aside).expect("and what stands beside it");
    // The panel is 208 wide at first, and a row ends short of its edge.
    assert!(
        aside.right() <= px(208.),
        "what stands beside the name is inside the panel: {aside:?}"
    );
    assert!(
        name.right() <= aside.left(),
        "the name ends before it: {name:?}, {aside:?}"
    );
    assert!(
        name.size.width > px(40.),
        "and is not squeezed away: {name:?}"
    );
}

/// The keys an action has now, by its name in `keybindings.toml`.
fn keys_of(stage: &mut Stage, id: &str) -> Vec<String> {
    stage.app(|cx| cx.global::<keymap::Keys>().resolved.keys_of(id).to_vec())
}

/// A key is given to an action in the sheet of shortcuts by pressing it.
/// The key that is pressed belongs to something already: it has to be
/// taken, not run, nothing is written before it is confirmed, and then it
/// does its new thing at once.
fn a_key_is_rebound_by_pressing_it_and_is_not_run_meanwhile() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::ChangeKeys);
    assert_eq!(stage.shell().sheet, Some("shortcuts"));
    // From the search field: the third row is "New tab"; Enter opens it,
    // Enter again waits for the key.
    stage.keys("down down down enter enter");
    // Ctrl+P opens the palette everywhere else.
    stage.keys("ctrl-p");
    assert!(
        !stage.shell().palette_open,
        "the key that is pressed for an action must not do what it does now"
    );
    assert_eq!(
        keys_of(&mut stage, "tab.new"),
        ["ctrl-t"],
        "nothing changes before it is saved"
    );
    assert_eq!(keys_of(&mut stage, "palette.open"), ["ctrl-p"]);
    // Enter saves: the action that had the key loses it.
    stage.keys("enter");
    assert_eq!(keys_of(&mut stage, "tab.new"), ["ctrl-p"]);
    assert!(keys_of(&mut stage, "palette.open").is_empty());
    let said = stage.notices();
    assert!(
        said.iter()
            .any(|said| said.contains("Ctrl+P") && said.contains("New tab")),
        "{said:?}"
    );
    // It is in the file, where it can be read and changed by hand.
    let file = stage
        .app(|cx| cx.global::<keymap::Keys>().file.clone())
        .expect("the tests have a place for files");
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(
        written.contains("\"tab.new\" = \"ctrl-p\"") && written.contains("\"palette.open\" = []"),
        "{written}"
    );
    // And the key does its new thing at once.
    let tabs = stage.shell().tabs.len();
    stage.keys("ctrl-p");
    let shell = stage.shell();
    assert!(!shell.palette_open, "{shell:?}");
    assert_eq!(shell.tabs.len(), tabs + 1, "Ctrl+P opens a tab now");

    // Every key back to its default, after the question.
    stage.act(keymap::ResetKeys);
    assert!(stage.dialog_open(), "it asks first");
    stage.keys("enter");
    stage.until("the defaults are back", |stage| {
        keys_of(stage, "palette.open") == ["ctrl-p"]
    });
    assert_eq!(keys_of(&mut stage, "tab.new"), ["ctrl-t"]);
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(!written.contains("tab.new"), "{written}");
}

/// In the view of index flags a file git was told not to look at is
/// brought back with the keys; its button brings the rest back at once.
fn files_git_was_told_not_to_look_at_are_brought_back_by_key() {
    repos::fresh();
    let repo = repos::project_with_files();
    let tracked: Vec<String> = repos::git(&repo, &["ls-files"])
        .lines()
        .take(3)
        .map(str::to_owned)
        .collect();
    assert_eq!(tracked.len(), 3, "{tracked:?}");
    repos::git(&repo, &["update-index", "--assume-unchanged", &tracked[0]]);
    repos::git(
        &repo,
        &["update-index", "--skip-worktree", &tracked[1], &tracked[2]],
    );
    // As git lists them: `h` for assumed unchanged, `S` for skipped.
    let hidden = |repo: &std::path::Path| {
        repos::git(repo, &["ls-files", "-v"])
            .lines()
            .filter(|line| line.starts_with('h') || line.starts_with('S'))
            .count()
    };
    assert_eq!(hidden(&repo), 3);

    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::ShowIndexFlags);
    stage.until("the three files are listed", |stage| {
        stage.shows(("flag-clear", 2usize))
    });
    // The view has the keys as soon as it is open: down to the first row,
    // Enter takes its mark off.
    stage.keys("down enter");
    stage.until("git looks at the first file again", |_| hidden(&repo) == 2);
    stage.until("and the list is one shorter", |stage| {
        !stage.shows(("flag-clear", 2usize))
    });
    // The others at once.
    stage.click("flags-clear-all");
    stage.until("git looks at every file again", |_| hidden(&repo) == 0);
    stage.until("and nothing is listed", |stage| {
        !stage.shows(("flag-clear", 0usize))
    });
}

/// The signing view has a cursor: a key that could sign is chosen with
/// the arrows and Enter or Space. The keys are made here, in the `.ssh` of
/// the scratch home, which is where the view looks, and taken away again:
/// the pictures of the view show a home without any.
fn a_signing_key_is_chosen_with_the_keys() {
    if !repos::on_path("ssh-keygen") {
        println!("skipped: ssh-keygen is not installed");
        return;
    }
    repos::fresh();
    let repo = repos::project();
    let home = std::path::PathBuf::from(std::env::var_os("HOME").unwrap());
    assert!(
        home.starts_with(crate::hermetic::inside().unwrap()),
        "the home of the scratch surroundings"
    );
    let ssh = home.join(".ssh");
    let _ = std::fs::remove_dir_all(&ssh);
    std::fs::create_dir_all(&ssh).unwrap();
    for name in ["id_ada", "id_bob"] {
        let status = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f"])
            .arg(ssh.join(name))
            .stdin(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "ssh-keygen made a key");
    }
    // What the repository itself says it signs with: nothing yet.
    let set = |repo: &std::path::Path, name: &str| {
        repos::git(repo, &["config", "--local", "--default", "", "--get", name])
            .trim()
            .to_owned()
    };
    assert_eq!(set(&repo, "user.signingkey"), "");

    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::ShowSigning);
    stage.until("the two keys are listed", |stage| {
        stage.workspace().signing_keys == 2
    });
    assert_eq!(stage.workspace().signing_cursor, None);
    // The view has the keys as soon as it is open. Enter with the cursor
    // nowhere chooses nothing.
    stage.keys("enter");
    stage.pass(Duration::from_millis(300));
    assert_eq!(set(&repo, "user.signingkey"), "");
    // Down to the first key, on to the second and no further; back up.
    stage.keys("down");
    let first = stage.workspace().signing_cursor.expect("the first key");
    stage.keys("down");
    let second = stage.workspace().signing_cursor.expect("the second key");
    assert_ne!(first, second);
    stage.keys("down");
    assert_eq!(stage.workspace().signing_cursor.as_ref(), Some(&second));
    stage.keys("home");
    assert_eq!(stage.workspace().signing_cursor.as_ref(), Some(&first));
    stage.keys("end");
    assert_eq!(stage.workspace().signing_cursor.as_ref(), Some(&second));
    stage.keys("up");
    assert_eq!(stage.workspace().signing_cursor.as_ref(), Some(&first));

    // Enter: the repository signs with the key the cursor is on.
    stage.keys("enter");
    stage.until("the repository signs with the first key", |_| {
        set(&repo, "user.signingkey") == first
    });
    assert_eq!(set(&repo, "gpg.format"), "ssh");
    stage.until("and that is said", |stage| {
        stage
            .notices()
            .iter()
            .any(|notice| notice.contains("signs with that key now"))
    });
    // Enter again on the key in use: said, and nothing is written.
    stage.keys("enter");
    stage.until("the key in use is said to be", |stage| {
        stage
            .notices()
            .iter()
            .any(|notice| notice.contains("in use here"))
    });
    assert_eq!(set(&repo, "user.signingkey"), first);
    // Space chooses as Enter does.
    stage.keys("down space");
    stage.until("the repository signs with the second key", |_| {
        set(&repo, "user.signingkey") == second
    });
    // The cursor is still where it was after the view read the set-up
    // again.
    assert_eq!(stage.workspace().signing_cursor.as_ref(), Some(&second));
    drop(stage);
    std::fs::remove_dir_all(&ssh).unwrap();
}

/// Opened on a file git resolved as last time (rerere), the merge tool
/// starts from what git left in the file, so that saving keeps the
/// remembered resolution; the two sides are one button away.
fn the_merge_tool_starts_from_what_was_resolved_last_time() {
    let resolution = "Search for a needle.\nUse quotes of either kind around several words.\n";
    repos::fresh();
    let repo = repos::project_resolved_as_last_time(resolution);
    let guide = repo.join("docs/guide.md");
    let tool = |stage: &mut Stage| -> (bool, Option<String>) {
        stage.with_workspace(|workspace, _, cx| {
            let tool = workspace.read(cx).merge_tool();
            let tool = tool.read(cx);
            (tool.starts_from_reused(), tool.result_text(cx))
        })
    };

    let mut stage = Stage::open(Start::with(&repo));
    assert_eq!(stage.workspace().in_progress, Some(("merge".to_owned(), 1)));
    stage.keys("ctrl-home");
    stage.focus_changes();
    // Opened from the list it is shown as it is, to be changed there; the
    // merge tool is asked for by its tab.
    stage.keys("down enter");
    assert_eq!(stage.workspace().file_tab, "edit");
    stage.click("tab-merge");
    stage.until("the merge tool has the file", |stage| {
        stage.workspace().file_tab == "merge" && tool(stage).1.is_some()
    });
    // The result is what was resolved last time, and says so.
    assert_eq!(tool(&mut stage), (true, Some(resolution.to_owned())));
    assert!(stage.shows("merge-rebuild"));
    // Nothing was written by looking.
    assert_eq!(std::fs::read_to_string(&guide).unwrap(), resolution);
    assert_eq!(stage.workspace().in_progress, Some(("merge".to_owned(), 1)));

    // The way back to the two sides: the result is put together from
    // them, and is the remembered one no longer.
    stage.click("merge-rebuild");
    stage.until("the result is from the two sides", |stage| !tool(stage).0);
    let (_, from_sides) = tool(&mut stage);
    let from_sides = from_sides.unwrap();
    assert!(from_sides.contains("<<<<<<<"), "{from_sides}");
    assert!(from_sides.contains("single quotes"), "{from_sides}");
    assert!(from_sides.contains("double quotes"), "{from_sides}");
    stage.click("merge-close");
    stage.pass(Duration::from_secs(1));
    drop(stage);

    // Saved as it starts, the file is resolved to what was remembered.
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.focus_changes();
    stage.keys("down enter");
    stage.click("tab-merge");
    stage.until("the merge tool has the file again", |stage| {
        tool(stage) == (true, Some(resolution.to_owned()))
    });
    stage.click("merge-save");
    stage.until("nothing is in conflict any more", |stage| {
        stage.workspace().in_progress == Some(("merge".to_owned(), 0))
    });
    assert_eq!(std::fs::read_to_string(&guide).unwrap(), resolution);
    assert_eq!(
        repos::git(&repo, &["diff", "--cached", "--name-only"]).trim(),
        "docs/guide.md",
        "staged as resolved"
    );

    // A file that was not resolved by rerere starts from the two sides.
    drop(stage);
    repos::fresh();
    let repo = repos::project_in_conflict();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-home");
    stage.focus_changes();
    stage.keys("down enter");
    stage.until("the merge tool has the file", |stage| {
        stage.workspace().file_tab == "merge" && tool(stage).1.is_some()
    });
    let (reused, text) = tool(&mut stage);
    assert!(!reused);
    assert!(text.unwrap().contains("<<<<<<<"));
}
