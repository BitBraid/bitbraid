//! What the window does, as `behaviour.rs` asks it, for the window: its
//! bars, tabs, prompts, toasts, the palette, zoom and the page of
//! repositories.

use std::{cell::Cell, rc::Rc, time::Duration};

use bitbraid_ui::{ask::Ask, keymap, shell::STATUS_BAR_HEIGHT};

use crate::{
    repos,
    runner::Test,
    stage::{HEIGHT, Stage, Start, WIDTH},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "what_the_application_is_lies_over_the_window_and_copies_its_details",
        run: what_the_application_is_lies_over_the_window_and_copies_its_details,
    },
    Test {
        name: "a_question_is_asked_in_the_toolbars_place_and_answered_there",
        run: a_question_is_asked_in_the_toolbars_place_and_answered_there,
    },
    Test {
        name: "a_question_lies_over_a_page_without_toolbar_and_enter_goes_ahead",
        run: a_question_lies_over_a_page_without_toolbar_and_enter_goes_ahead,
    },
    Test {
        name: "a_refused_push_asks_in_the_bar_whether_to_pull_or_to_force",
        run: a_refused_push_asks_in_the_bar_whether_to_pull_or_to_force,
    },
    Test {
        name: "the_list_of_tabs_finds_a_tab_goes_to_it_and_closes_one",
        run: the_list_of_tabs_finds_a_tab_goes_to_it_and_closes_one,
    },
    Test {
        name: "the_repository_and_branch_blocks_drop_panels_to_find_one",
        run: the_repository_and_branch_blocks_drop_panels_to_find_one,
    },
    Test {
        name: "the_toolbar_greys_what_cannot_be_done_and_stashes_at_once",
        run: the_toolbar_greys_what_cannot_be_done_and_stashes_at_once,
    },
    Test {
        name: "the_large_file_control_stands_after_the_group",
        run: the_large_file_control_stands_after_the_group,
    },
    Test {
        name: "a_toast_stands_at_the_bottom_left_and_only_a_success_goes_by_itself",
        run: a_toast_stands_at_the_bottom_left_and_only_a_success_goes_by_itself,
    },
    Test {
        name: "the_history_of_notifications_hangs_from_the_bell",
        run: the_history_of_notifications_hangs_from_the_bell,
    },
    Test {
        name: "the_activity_log_is_a_dialog_over_the_window",
        run: the_activity_log_is_a_dialog_over_the_window,
    },
    Test {
        name: "the_list_of_keys_is_a_dialog_over_the_window",
        run: the_list_of_keys_is_a_dialog_over_the_window,
    },
    Test {
        name: "the_default_keys_do_what_they_say",
        run: the_default_keys_do_what_they_say,
    },
    Test {
        name: "tabs_shrink_have_one_cross_and_slide_where_they_are_dragged",
        run: tabs_shrink_have_one_cross_and_slide_where_they_are_dragged,
    },
    Test {
        name: "tabs_keep_their_width_shown_under_the_pointer_or_not",
        run: tabs_keep_their_width_shown_under_the_pointer_or_not,
    },
    Test {
        name: "the_page_of_repositories_has_a_pinned_tab_as_the_inbox_has",
        run: the_page_of_repositories_has_a_pinned_tab_as_the_inbox_has,
    },
    Test {
        name: "nothing_of_the_tabs_moves_whichever_is_shown_and_every_tab_has_its_lines",
        run: nothing_of_the_tabs_moves_whichever_is_shown_and_every_tab_has_its_lines,
    },
    Test {
        name: "a_new_tab_is_shown_from_the_inbox_and_the_page_of_repositories",
        run: a_new_tab_is_shown_from_the_inbox_and_the_page_of_repositories,
    },
    Test {
        name: "the_zoom_is_picked_from_a_list_and_branch_names_in_the_graph",
        run: the_zoom_is_picked_from_a_list_and_branch_names_in_the_graph,
    },
    Test {
        name: "the_whole_window_grows_with_the_zoom_and_a_click_finds_its_row",
        run: the_whole_window_grows_with_the_zoom_and_a_click_finds_its_row,
    },
    Test {
        name: "every_list_is_laid_out_again_at_once_whichever_way_the_zoom_changes",
        run: every_list_is_laid_out_again_at_once_whichever_way_the_zoom_changes,
    },
    Test {
        name: "the_shown_tab_and_its_commit_come_back_at_every_start",
        run: the_shown_tab_and_its_commit_come_back_at_every_start,
    },
];

/// What the bar asks, if it asks anything.
fn said(stage: &mut Stage) -> Option<String> {
    stage.shell().bar.map(|(said, _)| said)
}

/// A tab's name and a prompt of the repository's are asked in a bar that
/// takes the toolbar's place, with the tabs dimmed meanwhile: going ahead
/// waits for what it needs, a second choice and giving up are buttons
/// beside it, Escape gives up, and the toolbar is back afterwards.
fn a_question_is_asked_in_the_toolbars_place_and_answered_there() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let folder = stage.shell().tabs[0].title.clone();
    assert!(stage.shows("undo") && !stage.shows("ask-cancel"));

    // The bar instead of the toolbar, its field empty and focused.
    stage.act(keymap::RenameTab);
    assert_eq!(
        stage.shell().bar,
        Some(("Tab name".to_owned(), vec![String::new()]))
    );
    assert!(stage.shows("ask-cancel") && stage.shows(("ask-choice", 1usize)));
    assert!(!stage.shows("undo"), "the toolbar has made room");
    assert!(!stage.dialog_open(), "a bar, not a dialog");
    // Nothing to save while the name is what it was.
    stage.click(("ask-choice", 0usize));
    assert!(said(&mut stage).is_some(), "Save waits for a new name");
    stage.type_text("work");
    stage.click(("ask-choice", 0usize));
    assert_eq!(said(&mut stage), None);
    assert_eq!(stage.shell().tabs[0].title, "work");
    assert!(stage.shows("undo") && !stage.shows("ask-cancel"));

    // Asked again, the field holds the name; the second choice clears it.
    stage.act(keymap::RenameTab);
    assert_eq!(
        stage.shell().bar,
        Some(("Tab name".to_owned(), vec!["work".to_owned()]))
    );
    stage.click(("ask-choice", 1usize));
    assert_eq!(said(&mut stage), None);
    assert_eq!(stage.shell().tabs[0].title, folder);

    // Escape gives up and changes nothing; so does the last button.
    stage.act(keymap::RenameTab);
    stage.type_text("other");
    stage.keys("escape");
    assert_eq!(said(&mut stage), None);
    assert_eq!(stage.shell().tabs[0].title, folder);
    stage.act(keymap::RenameTab);
    stage.click("ask-cancel");
    assert_eq!(said(&mut stage), None);

    // A prompt of the repository's asks there too, and Enter in its field
    // goes ahead.
    stage.act(keymap::JumpTo);
    let (_, texts) = stage.shell().bar.expect("the jump is asked in the bar");
    assert_eq!(texts.len(), 1);
    assert!(stage.workspace().prompting);
    stage.type_text("main");
    stage.keys("enter");
    assert_eq!(said(&mut stage), None);
    assert!(stage.shows("undo"));
}

/// Where there is no toolbar the bar lies over the top of the page; a
/// question without fields goes ahead on Enter, and a new question takes
/// the place of the one on show, which counts as given up.
fn a_question_lies_over_a_page_without_toolbar_and_enter_goes_ahead() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    assert!(stage.shows("browse") && !stage.shows("undo"));
    let (yes, given_up) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
    let (for_yes, for_given_up) = (yes.clone(), given_up.clone());
    stage.with_shell(move |_, window, cx| {
        bitbraid_ui::ask::show(
            Ask::new("Is this a question?")
                .go("Yes", move |texts, _, _| {
                    assert!(texts.is_empty());
                    for_yes.set(for_yes.get() + 1);
                })
                .on_cancel(move |_, _, _| for_given_up.set(for_given_up.get() + 1)),
            window,
            cx,
        )
    });
    stage.frame();
    assert_eq!(said(&mut stage).as_deref(), Some("Is this a question?"));
    assert!(
        stage.shows("ask-cancel") && stage.shows("browse"),
        "over the page"
    );
    stage.keys("enter");
    assert_eq!((yes.get(), given_up.get()), (1, 0));
    assert_eq!(said(&mut stage), None);

    // Asked twice: the first is given up, the second stands.
    let for_given_up = given_up.clone();
    stage.with_shell(move |_, window, cx| {
        bitbraid_ui::ask::show(
            Ask::new("First").on_cancel(move |_, _, _| for_given_up.set(for_given_up.get() + 1)),
            window,
            cx,
        );
        bitbraid_ui::ask::show(Ask::new("Second"), window, cx);
    });
    stage.frame();
    assert_eq!(said(&mut stage).as_deref(), Some("Second"));
    assert_eq!(given_up.get(), 1);
    stage.keys("escape");
    assert_eq!(said(&mut stage), None);
}

/// A push the remote refuses because it has commits the branch does not
/// asks in the bar: pull them in, or force, which says first what it would
/// overwrite in a dialog of its own.
fn a_refused_push_asks_in_the_bar_whether_to_pull_or_to_force() {
    repos::fresh();
    let repo = repos::project();
    let origin = repos::work().join("origin.git");
    let origin_path = origin.to_str().expect("a path").to_owned();
    repos::git(&repo, &["init", "-q", "--bare", &origin_path]);
    repos::git(&repo, &["remote", "add", "origin", &origin_path]);
    repos::git(&repo, &["push", "-q", "-u", "origin", "main"]);
    // The remote has a commit this branch has not, and this one another.
    repos::git(&repo, &["commit", "-q", "--allow-empty", "-m", "theirs"]);
    repos::git(&repo, &["push", "-q", "origin", "main"]);
    repos::git(&repo, &["reset", "-q", "--hard", "HEAD~1"]);
    repos::git(&repo, &["commit", "-q", "--allow-empty", "-m", "ours"]);
    let mut stage = Stage::open(Start::with(&repo));

    stage.act(keymap::Push);
    stage.until("the refused push is asked about", |stage| {
        said(stage).is_some_and(|said| said.contains("pull them in"))
    });
    assert!(!stage.dialog_open());
    // Pull first, then Force push, in the colour of danger, then Cancel.
    stage.click(("ask-choice", 1usize));
    assert_eq!(said(&mut stage), None);
    assert!(
        stage.dialog_open(),
        "the forced push says what it overwrites"
    );
    stage.keys("escape");
    stage.pass(Duration::from_secs(1));
    assert_eq!(
        repos::git(&origin, &["log", "-1", "--format=%s", "main"]).trim(),
        "theirs",
        "nothing was overwritten"
    );
}

/// The chevron at the right end of the tab bar opens a list of the tabs:
/// the repositories' first, then the others; what is typed finds a tab by
/// its name, Enter goes to the first one found, a click to the one
/// clicked, and the cross of a row closes that tab.
fn the_list_of_tabs_finds_a_tab_goes_to_it_and_closes_one() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::NewTab);
    stage.act(keymap::NewTab);
    assert_eq!((stage.shell().tabs.len(), stage.shell().active), (3, 2));

    stage.click("tab-list");
    assert!(stage.shows(("tab-list-row", 0usize)) && stage.shows(("tab-list-row", 2usize)));
    let (_, repo_row, _, _) = stage.place(("tab-list-row", 0usize));
    let (_, other_row, _, _) = stage.place(("tab-list-row", 1usize));
    assert!(repo_row < other_row, "the repository's tab is listed first");
    // Found by its name; Enter goes there and the list closes.
    let name = stage.shell().tabs[0].title.clone();
    stage.type_text(&name);
    assert!(
        !stage.shows(("tab-list-row", 1usize)),
        "only what is found is listed"
    );
    stage.keys("enter");
    assert_eq!(stage.shell().active, 0);
    assert!(!stage.shows(("tab-list-row", 0usize)), "the list is closed");

    // Opened again it has nothing typed; a click goes to the row's tab.
    stage.click("tab-list");
    assert!(stage.shows(("tab-list-row", 1usize)));
    stage.click(("tab-list-row", 1usize));
    assert_eq!(stage.shell().active, 1);

    // The cross closes that tab and leaves the others listed.
    stage.click("tab-list");
    stage.click(("tab-list-close", 2usize));
    assert_eq!(stage.shell().tabs.len(), 2);
    assert!(stage.shows(("tab-list-row", 0usize)));
}

/// The repository and the branch at the toolbar's left are blocks 110
/// wide, the branch's from 123 whatever the repository is called. Each
/// drops a panel with a field to find one: the recent repositories but the
/// one on show (Enter opens the first found), the local branches with the
/// current one first (a click checks one out).
fn the_repository_and_branch_blocks_drop_panels_to_find_one() {
    repos::fresh();
    let repo = repos::project();
    let library = repos::library();
    let state = format!("recent = [{repo:?}, {library:?}]\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));

    let (repo_x, _, repo_width, _) = stage.place("crumb-repo");
    let (branch_x, _, branch_width, _) = stage.place("crumb-branch");
    assert_eq!((repo_x, repo_width), (0.0, 110.0));
    assert_eq!((branch_x, branch_width), (123.0, 110.0));

    // The branches: the current one first, then the others; what is typed
    // finds one, and a click checks it out.
    stage.click("crumb-branch");
    assert!(
        stage.shows(("branch-finder-row", 0usize)) && stage.shows(("branch-finder-row", 3usize))
    );
    let (_, top, _, _) = stage.place(("branch-finder-row", 0usize));
    assert!(top > 48.0 + 34.0, "under the toolbar and the field: {top}");
    stage.type_text("pages");
    assert!(!stage.shows(("branch-finder-row", 1usize)));
    stage.click(("branch-finder-row", 0usize));
    stage.until("the branch is checked out", |_| {
        repos::git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).trim() == "pages"
    });
    assert!(
        !stage.shows(("branch-finder-row", 0usize)),
        "the panel is closed"
    );

    // The repositories: the one on show is not listed; Enter opens the
    // first one found, in a tab of its own.
    stage.click("crumb-repo");
    assert!(stage.shows(("repo-finder-row", 0usize)));
    assert!(
        !stage.shows(("repo-finder-row", 1usize)),
        "the open one is left out"
    );
    stage.keys("enter");
    stage.until("the other repository is shown", |stage| {
        let shell = stage.shell();
        shell
            .tabs
            .get(shell.active)
            .and_then(|tab| tab.path.clone())
            == Some(library.clone())
    });
}

/// Stash and Pop are greyed while there is nothing for them; Stash in the
/// toolbar stashes every change, untracked files too, without asking; and
/// while what a button started runs, the whole middle group is greyed and
/// that button spins.
fn the_toolbar_greys_what_cannot_be_done_and_stashes_at_once() {
    repos::fresh();
    let repo = repos::project();
    let origin = repos::work().join("origin.git");
    let origin_path = origin.to_str().expect("a path").to_owned();
    repos::git(&repo, &["init", "-q", "--bare", &origin_path]);
    repos::git(&repo, &["remote", "add", "origin", &origin_path]);
    let mut stage = Stage::open(Start::with(&repo));
    let toolbar = stage.shell().toolbar;
    assert!(
        !toolbar.busy && !toolbar.can_stash && !toolbar.can_pop,
        "{toolbar:?}"
    );

    // Something to stash: a file git does not know yet.
    std::fs::write(repo.join("scratch.txt"), "a note\n").expect("written");
    stage.until("there is something to stash", |stage| {
        stage.shell().toolbar.can_stash
    });
    stage.click("stash");
    assert!(
        !stage.dialog_open() && stage.shell().bar.is_none(),
        "nothing is asked"
    );
    stage.until("the change is stashed", |stage| {
        stage.shell().toolbar.can_pop
    });
    assert!(
        !repo.join("scratch.txt").exists(),
        "untracked files went too"
    );
    assert_eq!(repos::git(&repo, &["stash", "list"]).lines().count(), 1);
    stage.until("nothing is left to stash", |stage| {
        !stage.shell().toolbar.can_stash
    });

    // While the push runs, the group waits and Push spins: as it starts,
    // before its answer is in.
    let started = stage.with_shell(|shell, window, cx| {
        let workspace = shell.shown_workspace().expect("a repository is shown");
        workspace.update(cx, |workspace, cx| workspace.push(window, cx));
        shell.toolbar_state(cx)
    });
    assert!(started.busy, "{started:?}");
    assert_eq!(started.spin, Some("push"));
    stage.until("the push is done", |stage| !stage.shell().toolbar.busy);
    assert_eq!(stage.shell().toolbar.spin, None);
    assert!(!repos::git(&origin, &["for-each-ref", "refs/heads/main"]).is_empty());
}

/// While a pattern of large files is tracked, one control stands 70 after
/// Pop, and the group stands 19 further left: the middle of Undo 263 left
/// of the window's.
fn the_large_file_control_stands_after_the_group() {
    repos::fresh();
    let repo = repos::project();
    std::fs::write(
        repo.join(".gitattributes"),
        "*.bin filter=lfs diff=lfs merge=lfs -text\n",
    )
    .expect("written");
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the pattern is seen", |stage| stage.shows("lfs"));
    let (undo_x, _, undo_width, _) = stage.place("undo");
    assert!(
        (undo_x + undo_width / 2.0 - (WIDTH / 2.0 - 263.0)).abs() < 0.75,
        "Undo at {undo_x}"
    );
    let (pop_x, _, pop_width, _) = stage.place("pop");
    let (lfs_x, _, lfs_width, _) = stage.place("lfs");
    assert!(
        (lfs_x - (pop_x + pop_width) - 70.0).abs() < 0.75,
        "LFS at {lfs_x}"
    );
    assert!((lfs_width - 46.0).abs() < 0.75);
    // Its menu has what can be done with large files.
    stage.click("lfs");
    stage.keys("down");
    stage.keys("escape");
}

/// A toast stands at the bottom left, 16 from the window's edge and 7
/// above the status bar, 352 wide. What succeeded goes by itself; what
/// failed stays until its cross puts it away. Fetching says nothing.
fn a_toast_stands_at_the_bottom_left_and_only_a_success_goes_by_itself() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.with_shell(|_, window, cx| {
        bitbraid_ui::notices::push(window, cx, bitbraid_ui::notices::Notice::success("It went"))
    });
    stage.frame();
    let (x, y, width, height) = stage.place("notification");
    assert!(
        (x - 16.0 - 36.0).abs() < 1.0 || (x - 16.0).abs() < 1.0,
        "at {x}"
    );
    assert!(
        ((y + height) - (HEIGHT - STATUS_BAR_HEIGHT - 7.0)).abs() < 2.0,
        "{y} {height}"
    );
    assert!((width - 352.0).abs() < 1.0, "{width}");
    stage.pass(Duration::from_secs(8));
    assert!(!stage.shows("notice-close"), "a success goes by itself");

    stage.with_shell(|_, window, cx| {
        bitbraid_ui::notices::push(window, cx, bitbraid_ui::notices::Notice::error("It broke"))
    });
    stage.frame();
    stage.pass(Duration::from_secs(30));
    assert!(stage.shows("notice-close"), "a failure stays");
    stage.click("notice-close");
    stage.pass(Duration::from_secs(1));
    assert!(!stage.shows("notice-close"), "the cross puts it away");

    // A fetch that goes well says nothing.
    let said = stage.notices().len();
    stage.act(keymap::Fetch);
    stage.pass(Duration::from_secs(1));
    stage.until("the fetch has run", |stage| !stage.shell().toolbar.busy);
    stage.pass(Duration::from_secs(1));
    assert_eq!(stage.notices().len(), said, "{:?}", stage.notices());
}

/// The history of notifications hangs from the bell: a panel 450 wide,
/// right-aligned to the bell, from under the tabs to 75 above the window's
/// bottom, over the repository, which stays as it was under it.
fn the_history_of_notifications_hangs_from_the_bell() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.with_shell(|_, window, cx| {
        bitbraid_ui::notices::push(window, cx, bitbraid_ui::notices::Notice::error("It broke"))
    });
    stage.frame();
    stage.click("notices");
    assert_eq!(stage.shell().sheet, Some("notifications"));
    let (bell_x, _, bell_width, _) = stage.place("notices");
    let (x, y, width, height) = stage.place("notices-history");
    assert!((width - 450.0).abs() < 1.0, "{width}");
    assert!(
        (x + width - (bell_x + bell_width)).abs() < 2.0,
        "{x} {bell_x}"
    );
    assert!((34.0..42.0).contains(&y), "{y}");
    assert!(((y + height) - (HEIGHT - 75.0)).abs() < 3.0, "{y} {height}");
    assert!(stage.shows("undo"), "the toolbar is still there under it");
    // The bell puts it away again, and so does Escape.
    stage.click("notices");
    assert_eq!(stage.shell().sheet, None);
    stage.click("notices");
    stage.keys("escape");
    assert_eq!(stage.shell().sheet, None);
}

/// The activity log is a dialog over the dimmed window, 51 from its left,
/// right and top edge and 70 from its bottom, the repository unchanged
/// under it; its cross, a click beside it and Escape put it away.
fn the_activity_log_is_a_dialog_over_the_window() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let panes_before = stage.place("fold-refs");
    stage.act(keymap::ShowActivity);
    assert_eq!(stage.workspace().sheet, Some("activity"));
    let (x, y, width, height) = stage.place("activity-dialog");
    assert!((x - 51.0).abs() < 1.0 && (y - 51.0).abs() < 1.0, "{x} {y}");
    assert!((width - (WIDTH - 102.0)).abs() < 1.0, "{width}");
    assert!((height - (HEIGHT - 51.0 - 70.0)).abs() < 1.0, "{height}");
    assert_eq!(
        stage.place("fold-refs"),
        panes_before,
        "the repository stays under it"
    );

    stage.click("activity-close");
    assert_eq!(stage.workspace().sheet, None);
    stage.act(keymap::ShowActivity);
    stage.click_in("activity-backdrop", (20.0, 20.0));
    assert_eq!(stage.workspace().sheet, None, "a click beside it");
    stage.act(keymap::ShowActivity);
    stage.keys("escape");
    assert_eq!(stage.workspace().sheet, None, "Escape");
}

/// The list of keys is a dialog 673 by 517, centred, 83 under the window's
/// top; it lists what has a key, finds by what is typed, and its cross,
/// a click beside it and Escape put it away. Changing a key is the page
/// the settings open.
fn the_list_of_keys_is_a_dialog_over_the_window() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.keys("ctrl-/");
    assert_eq!(stage.shell().sheet, Some("keys"));
    let (x, y, width, height) = stage.place("key-list");
    assert!((width - 673.0).abs() < 1.0 && (height - 517.0).abs() < 1.0);
    assert!(
        (x - (WIDTH - 673.0) / 2.0).abs() < 1.0 && (y - 83.0).abs() < 1.0,
        "{x} {y}"
    );
    assert!(stage.shows("undo"), "the window stays under it");
    stage.click("key-list-close");
    assert_eq!(stage.shell().sheet, None);
    stage.click("cheat-sheet");
    stage.click_in("key-list-backdrop", (10.0, 300.0));
    assert_eq!(stage.shell().sheet, None, "a click beside it");
    stage.keys("ctrl-/");
    stage.keys("escape");
    assert_eq!(stage.shell().sheet, None, "Escape");
    stage.act(keymap::ChangeKeys);
    assert_eq!(stage.shell().sheet, Some("shortcuts"));
}

/// The default keys for what is here: Ctrl+N clones, Ctrl+I starts a
/// repository, Ctrl+Shift+A lists the tabs, Ctrl+Shift+H finds a file for
/// its history, Ctrl+Y redoes, Ctrl+Shift+F is full screen; the next theme
/// and the stash are not on Ctrl+Shift+T and Ctrl+Shift+S.
fn the_default_keys_do_what_they_say() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let keys_of = |stage: &mut Stage, id: &'static str| {
        stage.app(|cx| cx.global::<keymap::Keys>().resolved.keys_of(id).to_vec())
    };
    assert_eq!(
        keys_of(&mut stage, "redo.last").first().map(String::as_str),
        Some("ctrl-y")
    );
    assert_eq!(
        keys_of(&mut stage, "view.fullscreen")
            .first()
            .map(String::as_str),
        Some("ctrl-shift-f")
    );
    assert!(!keys_of(&mut stage, "theme.cycle").contains(&"ctrl-shift-t".to_owned()));
    assert!(!keys_of(&mut stage, "stash.push").contains(&"ctrl-shift-s".to_owned()));

    stage.keys("ctrl-n");
    assert!(stage.dialog_open(), "Ctrl+N asks what to clone");
    stage.keys("escape");
    assert!(!stage.dialog_open());
    stage.keys("ctrl-i");
    assert!(
        stage.dialog_open(),
        "Ctrl+I asks where the new repository goes"
    );
    stage.keys("escape");
    assert!(!stage.dialog_open());
    stage.keys("ctrl-shift-a");
    assert!(
        stage.shows(("tab-list-row", 0usize)),
        "Ctrl+Shift+A lists the tabs"
    );
    stage.keys("escape");
    stage.keys("ctrl-shift-h");
    assert!(stage.dialog_open(), "Ctrl+Shift+H finds a file");
}

/// A tab is called "New tab" until a repository is in it; only the shown
/// tab has a cross; past what fits the tabs grow narrower and the plus
/// stays in the window; a tab dragged onto another's place lands there.
fn tabs_shrink_have_one_cross_and_slide_where_they_are_dragged() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::NewTab);
    stage.act(keymap::NewTab);
    let shell = stage.shell();
    assert_eq!(shell.tabs[1].title, "New tab");
    assert_eq!(shell.active, 2);
    assert!(
        stage.shows(("close-tab", 2usize)),
        "the shown tab has a cross"
    );
    assert!(!stage.shows(("close-tab", 0usize)) && !stage.shows(("close-tab", 1usize)));

    // The repository's tab dragged onto the last place lands there.
    let (x0, y0, w0, h0) = stage.place(("tab-name", 0usize));
    let (x2, y2, w2, h2) = stage.place(("tab-name", 2usize));
    stage.drag_at(
        (x0 + w0 / 2.0, y0 + h0 / 2.0),
        (x2 + w2 / 2.0, y2 + h2 / 2.0),
    );
    let shell = stage.shell();
    assert_eq!(
        shell.tabs[2].path.as_deref(),
        Some(repo.as_path()),
        "{shell:?}"
    );

    // Many tabs: they grow narrower, with the plus after them; past what
    // fits at their least, as many as fit, the shown one the last of them.
    for _ in 0..20 {
        stage.act(keymap::NewTab);
    }
    let (plus_x, _, plus_width, _) = stage.place("new-tab");
    assert!(
        plus_x + plus_width < WIDTH - 100.0,
        "the plus is in the window: {plus_x}"
    );
    let last = stage.shell().active;
    let (_, _, narrow, _) = stage.place(("tab-name", last));
    assert!(narrow < w0, "{narrow} {w0}");
    assert!(!stage.shows(("tab", 0usize)), "the first has no room");
}

/// A tab is as wide shown as not, and under the pointer as not: the
/// cross's place is every tab's (the cross is drawn on the shown tab and
/// the one the pointer is over), and a name takes the room of its bold.
/// Showing another tab and passing over one move no tab; closing one
/// moves only the tabs after it, into its place, at their own widths.
fn tabs_keep_their_width_shown_under_the_pointer_or_not() {
    for zoom in [1.0f32, 1.5] {
        repos::fresh();
        let repo = repos::project();
        let library = repos::library();
        let mut stage = Stage::open(Start::with(&repo).preferences(&format!("zoom = {zoom}\n")));
        // Three names of three lengths: the repository, another one opened
        // in a new tab, and a new tab.
        stage.act(keymap::NewTab);
        stage.with_shell(|shell, window, cx| shell.open_repository(library.clone(), window, cx));
        stage.act(keymap::NewTab);
        stage.settle();
        assert_eq!(stage.shell().tabs.len(), 3);
        assert_eq!(stage.shell().active, 2);
        let places = |stage: &mut Stage, count: usize| -> Vec<(f32, f32)> {
            (0..count)
                .map(|ix| {
                    let (x, _, wide, _) = stage.place(("tab", ix));
                    (x, wide)
                })
                .collect()
        };
        let same = |seen: &[(f32, f32)], before: &[(f32, f32)], what: &str| {
            assert_eq!(seen.len(), before.len(), "{what}");
            for (now, then) in seen.iter().zip(before) {
                assert!(
                    (now.0 - then.0).abs() < 0.01 && (now.1 - then.1).abs() < 0.01,
                    "at {zoom}: {what}: the tabs (left, width) are {seen:?}, were {before:?}"
                );
            }
        };
        let (plus, ..) = stage.place("new-tab");
        let before = places(&mut stage, 3);

        // Another tab shown, and another.
        for shown in [0usize, 1] {
            stage.click(("tab-name", shown));
            assert_eq!(stage.shell().active, shown);
            same(
                &places(&mut stage, 3),
                &before,
                &format!("tab {shown} shown"),
            );
            assert_eq!(stage.place("new-tab").0, plus, "the plus stays");
        }

        // The pointer over a tab that is not shown: its cross, nothing moves.
        stage.click(("tab-name", 0usize));
        let (x, y, wide, high) = stage.place(("tab", 1usize));
        stage.point_at((x + wide / 2.0, y + high / 2.0));
        assert!(
            stage.shows(("close-tab", 1usize)),
            "the tab under the pointer shows its cross"
        );
        assert!(!stage.shows(("close-tab", 2usize)));
        same(&places(&mut stage, 3), &before, "under the pointer");
        // Away from the tabs, its cross goes, and nothing moves.
        stage.point_at((x + wide / 2.0, y + high * 8.0));
        assert!(!stage.shows(("close-tab", 1usize)));
        assert!(stage.shows(("close-tab", 0usize)), "the shown tab's cross");
        same(&places(&mut stage, 3), &before, "the pointer gone");

        // The middle tab closed by its cross: the first stays, the last
        // takes its place at its own width.
        stage.point_at((x + wide / 2.0, y + high / 2.0));
        stage.click(("close-tab", 1usize));
        assert_eq!(stage.shell().tabs.len(), 2);
        assert_eq!(stage.shell().active, 0);
        let after = places(&mut stage, 2);
        let (first, middle, last) = (before[0], before[1], before[2]);
        same(
            &after,
            &[first, (middle.0, last.1)],
            "the middle one closed",
        );
    }
}

/// The page of repositories has a pinned tab, as the inbox has: the first
/// at the bar's left end, the inbox's next to it, and the repositories'
/// tabs right after both, with no room between them. As high and, the inbox
/// counting nothing, as wide as the inbox's; a tab to assistive technology
/// in the pinned tabs' list, selected while its page is on show, when the
/// repository's tab and the inbox's are not; no cross. Clicked again it
/// leaves the page shown, as a shown tab does, and its key shows the page
/// from anywhere.
fn the_page_of_repositories_has_a_pinned_tab_as_the_inbox_has() {
    use gpui_kit::{Role, test::TestWindowExt};
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let found = |stage: &mut Stage, id: &'static str| stage.window(|window, _| window.find(id));
    let selected = |stage: &mut Stage| {
        let repository = stage.window(|window, _| window.find(("tab", 0usize)));
        (
            found(stage, "repositories-tab").selected(),
            found(stage, "inbox-tab").selected(),
            repository.selected(),
        )
    };
    let (pages_x, pages_y, pages_wide, pages_high) = stage.place("repositories-tab");
    let (inbox_x, inbox_y, inbox_wide, inbox_high) = stage.place("inbox-tab");
    let (first_x, ..) = stage.place(("tab", 0usize));
    assert!(pages_x.abs() < 0.01, "at the bar's left end: {pages_x}");
    assert!(
        (inbox_x - (pages_x + pages_wide)).abs() < 0.01,
        "the inbox's tab right after it: {inbox_x}"
    );
    assert!(
        (first_x - (inbox_x + inbox_wide)).abs() < 0.01,
        "the first tab right after both: {first_x}"
    );
    assert_eq!((pages_y, pages_high), (inbox_y, inbox_high), "as high");
    assert!(
        (pages_wide - inbox_wide).abs() < 0.01 && (34.0..=40.0).contains(&pages_wide),
        "as wide as the inbox's mark alone: {pages_wide}, {inbox_wide}"
    );
    let tab = found(&mut stage, "repositories-tab");
    assert_eq!(tab.role(), Some(Role::Tab));
    assert_eq!(tab.label(), Some("Repositories"));
    let pinned = found(&mut stage, "pinned-tabs");
    assert_eq!(pinned.role(), Some(Role::TabList));
    assert_eq!(pinned.label(), Some("Pinned tabs"));
    assert_eq!(selected(&mut stage), (Some(false), Some(false), Some(true)));

    // Shown: its tab is the selected one, as wide as before; the
    // repository's tab is drawn as not shown, without its cross.
    stage.click("repositories-tab");
    assert_eq!(stage.shell().sheet, Some("repositories"));
    assert_eq!(selected(&mut stage), (Some(true), Some(false), Some(false)));
    assert!((stage.place("repositories-tab").2 - pages_wide).abs() < 0.01);
    assert!(!stage.shows(("close-tab", 0usize)));
    // Clicked again, it stays.
    stage.click("repositories-tab");
    assert_eq!(stage.shell().sheet, Some("repositories"));

    // The inbox's tab takes over, and the page's key brings it back.
    stage.click("inbox-tab");
    assert_eq!(stage.shell().sheet, Some("inbox"));
    assert_eq!(selected(&mut stage), (Some(false), Some(true), Some(false)));
    stage.keys("ctrl-alt-o");
    assert_eq!(stage.shell().sheet, Some("repositories"));
    assert_eq!(selected(&mut stage), (Some(true), Some(false), Some(false)));

    // The repository's tab puts it away.
    stage.click(("tab-name", 0usize));
    assert_eq!(stage.shell().sheet, None);
    assert_eq!(selected(&mut stage), (Some(false), Some(false), Some(true)));
}

/// A place in the window, in points: left, top, width, height.
type Place = (f32, f32, f32, f32);

/// What a click on a tab must not move in the tab bar.
#[derive(Debug, Clone)]
struct TabBar {
    /// The pinned tabs and their marks: the page of repositories', its
    /// mark, the inbox's, its mark.
    pinned: Vec<Place>,
    /// The tabs drawn, by index: their box, their icon's place (none for
    /// a new tab) and their cross's.
    tabs: Vec<(usize, Place, Option<Place>, Place)>,
    /// The lines down the tabs' sides, from the left.
    lines: Vec<Place>,
    /// The shown tab's accent: where each one is drawn.
    accents: Vec<Place>,
}

/// The places of what an element was drawn as, in points.
fn place_of(bounds: gpui_kit::Bounds<gpui_kit::Pixels>) -> Place {
    (
        f32::from(bounds.origin.x),
        f32::from(bounds.origin.y),
        f32::from(bounds.size.width),
        f32::from(bounds.size.height),
    )
}

/// Where the tab bar has everything, `count` tabs open.
fn tab_bar(stage: &mut Stage, count: usize) -> TabBar {
    use gpui_kit::{ElementId, base::test_support::snapshots, test::TestWindowExt};
    let pinned = [
        "repositories-tab",
        "repositories-tab-mark",
        "inbox-tab",
        "inbox-tab-mark",
    ]
    .map(|id| stage.place(id))
    .to_vec();
    let mut tabs = Vec::new();
    for ix in 0..count {
        if !stage.shows(("tab", ix)) {
            continue;
        }
        let icon = stage.window(|window, _| {
            window
                .try_find(("tab-icon", ix))
                .map(|icon| place_of(icon.bounds()))
        });
        tabs.push((
            ix,
            stage.place(("tab", ix)),
            icon,
            stage.place(("tab-cross", ix)),
        ));
    }
    let named = |stage: &mut Stage, wanted: &'static str| {
        let mut found: Vec<Place> = stage.window(|window, _| {
            snapshots(window)
                .into_iter()
                .filter(|element| element.visible())
                .filter(|element| match element.path().last() {
                    Some(ElementId::NamedInteger(name, _) | ElementId::Name(name)) => {
                        name.as_ref() == wanted
                    }
                    _ => false,
                })
                .map(|element| place_of(element.bounds()))
                .collect()
        });
        found.sort_by(|a, b| a.0.total_cmp(&b.0));
        found
    };
    let lines = named(stage, "tab-side");
    let accents = named(stage, "tab-accent");
    TabBar {
        pinned,
        tabs,
        lines,
        accents,
    }
}

/// Whether two places are the same, to a hundredth of a point.
fn same_place(a: Place, b: Place) -> bool {
    [(a.0, b.0), (a.1, b.1), (a.2, b.2), (a.3, b.3)]
        .iter()
        .all(|(a, b)| (a - b).abs() < 0.01)
}

/// The tab bar is as it was: every tab, icon, cross and line where it was.
fn bar_as_before(now: &TabBar, before: &TabBar, what: &str) {
    let drawn = |bar: &TabBar| bar.tabs.iter().map(|tab| tab.0).collect::<Vec<_>>();
    assert_eq!(drawn(now), drawn(before), "{what}: the tabs drawn");
    for (now, then) in now.pinned.iter().zip(&before.pinned) {
        assert!(
            same_place(*now, *then),
            "{what}: a pinned tab or its mark is at {now:?}, was at {then:?}"
        );
    }
    for (now, then) in now.tabs.iter().zip(&before.tabs) {
        let ix = now.0;
        assert!(
            same_place(now.1, then.1),
            "{what}: tab {ix} is at {:?}, was at {:?}",
            now.1,
            then.1
        );
        match (now.2, then.2) {
            (Some(icon), Some(was)) => assert!(
                same_place(icon, was),
                "{what}: tab {ix}'s icon is at {icon:?}, was at {was:?}"
            ),
            (None, None) => {}
            (icon, was) => panic!("{what}: tab {ix}'s icon {icon:?}, was {was:?}"),
        }
        assert!(
            same_place(now.3, then.3),
            "{what}: tab {ix}'s cross's place is at {:?}, was at {:?}",
            now.3,
            then.3
        );
    }
    assert_eq!(
        now.lines.len(),
        before.lines.len(),
        "{what}: the lines are {:?}, were {:?}",
        now.lines,
        before.lines
    );
    for (now, then) in now.lines.iter().zip(&before.lines) {
        assert!(
            same_place(*now, *then),
            "{what}: a line is at {now:?}, was at {then:?}"
        );
    }
}

/// Every tab, pinned or not, has a line down its left edge and one down
/// its right edge, as high as it is; one line between two tabs; and the
/// first tab drawn follows the inbox's at once, the two sharing a line.
fn every_tab_has_its_lines(bar: &TabBar, what: &str) {
    let inbox = bar.pinned[2];
    if let Some((ix, first, _, _)) = bar.tabs.first() {
        assert!(
            (first.0 - (inbox.0 + inbox.2)).abs() < 0.01,
            "{what}: tab {ix} begins at {}, the inbox's ends at {}",
            first.0,
            inbox.0 + inbox.2
        );
    }
    let boxes = [bar.pinned[0], bar.pinned[2]]
        .into_iter()
        .chain(bar.tabs.iter().map(|tab| tab.1));
    for (x, y, wide, high) in boxes {
        let at = |edge: f32| {
            bar.lines.iter().any(|line| {
                ((line.0 - edge).abs() < 0.01 || (line.0 + line.2 - edge).abs() < 0.01)
                    && (line.1 - y).abs() < 0.01
                    && (line.3 - high).abs() < 0.01
            })
        };
        assert!(
            at(x),
            "{what}: the tab from {x} has no line at its left: {:?}",
            bar.lines
        );
        assert!(
            at(x + wide),
            "{what}: the tab to {} has no line at its right: {:?}",
            x + wide,
            bar.lines
        );
    }
    for pair in bar.lines.windows(2) {
        if let [a, b] = pair {
            assert!(
                b.0 - a.0 > 0.99,
                "{what}: two lines where one should be: {a:?} {b:?}"
            );
        }
    }
}

/// The shown tab's accent is drawn once, inside its box.
fn accent_in(bar: &TabBar, shown: Place, what: &str) {
    assert_eq!(bar.accents.len(), 1, "{what}: {:?}", bar.accents);
    let accent = bar.accents[0];
    assert!(
        accent.0 >= shown.0 - 0.01
            && accent.0 + accent.2 <= shown.0 + shown.2 + 0.01
            && (accent.1 - shown.1).abs() < 0.01,
        "{what}: the accent {accent:?} is not along the top of {shown:?}"
    );
}

/// Showing a tab, passing over one, and showing the inbox or the page of
/// repositories change colours in the tab bar, never where anything is:
/// with each tab shown in turn, with each pinned tab's page shown and with
/// the pointer over each tab, every tab's box, its icon, its cross's place,
/// the cross itself and every line down a tab's side stay where they were;
/// every tab, pinned or not, first or last, has a line at its left and one
/// at its right; the shown tab's accent is inside its box. The first tab
/// drawn follows the inbox's with no room between them. With three tabs and
/// with more than fit, at 100 % and at 150 %.
fn nothing_of_the_tabs_moves_whichever_is_shown_and_every_tab_has_its_lines() {
    for (zoom, new_tabs) in [(1.0f32, 1usize), (1.5, 1), (1.0, 18), (1.5, 18)] {
        repos::fresh();
        let repo = repos::project();
        let library = repos::library();
        let mut stage = Stage::open(Start::with(&repo).preferences(&format!("zoom = {zoom}\n")));
        stage.with_shell(|shell, window, cx| shell.open_repository(library.clone(), window, cx));
        for _ in 0..new_tabs {
            stage.click("new-tab");
        }
        stage.settle();
        let count = stage.shell().tabs.len();
        let at = format!("{count} tabs at {zoom}");
        let before = tab_bar(&mut stage, count);
        every_tab_has_its_lines(&before, &at);
        let drawn: Vec<usize> = before.tabs.iter().map(|tab| tab.0).collect();
        assert!(drawn.len() >= 3, "{at}: {drawn:?}");
        if new_tabs > 1 {
            assert!(drawn.len() < count, "{at}: more than fit: {drawn:?}");
        }

        // Each tab shown in turn.
        let mut crosses = Vec::new();
        for (pos, ix) in drawn.iter().copied().enumerate() {
            stage.click(("tab-name", ix));
            assert_eq!(stage.shell().active, ix, "{at}");
            let what = format!("{at}, tab {ix} shown");
            let now = tab_bar(&mut stage, count);
            bar_as_before(&now, &before, &what);
            every_tab_has_its_lines(&now, &what);
            accent_in(&now, before.tabs[pos].1, &what);
            crosses.push(stage.place(("close-tab", ix)));
        }

        // The inbox and the page of repositories shown: no tab is.
        for (pinned, sheet, place) in [
            ("inbox-tab", "inbox", 2),
            ("repositories-tab", "repositories", 0),
        ] {
            stage.click(pinned);
            assert_eq!(stage.shell().sheet, Some(sheet), "{at}");
            let what = format!("{at}, {sheet} shown");
            let now = tab_bar(&mut stage, count);
            bar_as_before(&now, &before, &what);
            every_tab_has_its_lines(&now, &what);
            accent_in(&now, before.pinned[place], &what);
        }

        // The pointer over each tab that is not shown: its cross where
        // the shown tab's is, nothing else moved.
        let shown = drawn[0];
        stage.click(("tab-name", shown));
        for (pos, ix) in drawn.iter().copied().enumerate().skip(1) {
            let (x, y, wide, high) = before.tabs[pos].1;
            stage.point_at((x + wide / 2.0, y + high / 2.0));
            let what = format!("{at}, the pointer over tab {ix}");
            assert!(stage.shows(("close-tab", ix)), "{what}");
            let cross = stage.place(("close-tab", ix));
            assert!(
                same_place(cross, crosses[pos]),
                "{what}: its cross is at {cross:?}, shown at {:?}",
                crosses[pos]
            );
            bar_as_before(&tab_bar(&mut stage, count), &before, &what);
        }
        stage.point_at((WIDTH / 2.0, HEIGHT / 2.0));
    }
}

/// A new tab is made and shown wherever it is asked for: with the inbox
/// or the page of repositories on show, the plus, its key and the
/// palette's entry each make a tab and show it, the page put away as a tab
/// chosen puts it away. So does every other way of opening a tab from the
/// page: a repository of its lists, a clone and a new repository.
fn a_new_tab_is_shown_from_the_inbox_and_the_page_of_repositories() {
    use gpui_kit::test::TestWindowExt;
    repos::fresh();
    let repo = repos::project();
    let library = repos::library();
    let into = repos::work().join("made");
    std::fs::create_dir_all(&into).unwrap();
    let state = format!("recent = [{library:?}]\nclone_dir = {into:?}\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    // The tab shown is the one drawn as shown, and is the one asked for.
    let shown = |stage: &mut Stage, what: &str| {
        let shell = stage.shell();
        assert_eq!(shell.sheet, None, "{what}: the page put away");
        let active = shell.active;
        let tab = stage.window(|window, _| window.find(("tab", active)));
        assert_eq!(tab.selected(), Some(true), "{what}: drawn as shown");
        shell
    };
    type Way = (&'static str, fn(&mut Stage));
    let ways: [Way; 3] = [
        ("the plus", |stage| stage.click("new-tab")),
        ("its key", |stage| stage.keys("ctrl-t")),
        ("the palette", |stage| {
            stage.act(keymap::TogglePalette);
            stage.type_text("New tab");
            stage.keys("enter");
        }),
    ];
    for (pinned, sheet) in [("inbox-tab", "inbox"), ("repositories-tab", "repositories")] {
        for (way, make) in ways {
            stage.click(pinned);
            assert_eq!(stage.shell().sheet, Some(sheet));
            let before = stage.shell().tabs.len();
            make(&mut stage);
            let what = format!("{way} over the {sheet}");
            let shell = shown(&mut stage, &what);
            assert_eq!(shell.tabs.len(), before + 1, "{what}: a tab made");
            assert_eq!(shell.active, before, "{what}: the new one shown");
            assert_eq!(shell.tabs[before].path, None, "{what}: a start page");
        }
    }

    // A repository of the page's lists, in a tab of its own, shown.
    let lib = library.file_name().unwrap().to_str().unwrap().to_owned();
    stage.click("repositories-tab");
    stage.until("the page lists the library", |stage| {
        stage.shows(format!("open-recent-{lib}"))
    });
    stage.click(format!("open-recent-{lib}"));
    let shell = shown(&mut stage, "a repository opened from the page");
    assert_eq!(
        shell.tabs[shell.active].path.as_deref(),
        Some(library.as_path())
    );

    // A clone asked for on the page.
    stage.click("repositories-tab");
    stage.click("repositories-clone");
    assert!(stage.dialog_open(), "the form that clones");
    stage.type_text(library.to_str().unwrap());
    let cloned = into.join("library");
    assert_eq!(stage.shell().form_rows[0], cloned.to_str().unwrap());
    stage.keys("enter");
    stage.until("the clone is open", |stage| {
        let shell = stage.shell();
        !shell.creating
            && shell
                .tabs
                .iter()
                .any(|tab| tab.path.as_deref() == Some(cloned.as_path()))
    });
    let shell = shown(&mut stage, "a clone made from the page");
    assert_eq!(
        shell.tabs[shell.active].path.as_deref(),
        Some(cloned.as_path())
    );

    // A new repository made from the page.
    stage.click("repositories-tab");
    stage.click("repositories-init");
    assert!(stage.dialog_open(), "the form for a new repository");
    stage.type_text("made-here");
    let fresh = into.join("made-here");
    assert_eq!(stage.shell().form_rows[2], fresh.to_str().unwrap());
    stage.click("form-ok");
    stage.until("the new repository is open", |stage| {
        let shell = stage.shell();
        !shell.creating
            && shell
                .tabs
                .iter()
                .any(|tab| tab.path.as_deref() == Some(fresh.as_path()))
    });
    let shell = shown(&mut stage, "a new repository made from the page");
    assert_eq!(
        shell.tabs[shell.active].path.as_deref(),
        Some(fresh.as_path())
    );
    stage.pass(Duration::from_secs(1));
}

/// The zoom in the status bar opens a list of sizes; one picked is the
/// zoom from then on. The toolbar's Branch names the new branch in the
/// graph, on the row of HEAD.
fn the_zoom_is_picked_from_a_list_and_branch_names_in_the_graph() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.click("zoom");
    // From the top: 300, 250, 200, 175, 150, 140, 130, 120.
    stage.keys("down down down down down down down down enter");
    let zoom = stage.app(|cx| cx.global::<bitbraid_ui::prefs::Prefs>().preferences.zoom);
    assert!((zoom - 1.2).abs() < 0.001, "{zoom}");
    stage.act(keymap::ZoomReset);

    stage.click("new-branch");
    assert_eq!(stage.graph().naming.map(|(_, kind)| kind), Some("branch"));
    assert!(!stage.dialog_open() && stage.shell().bar.is_none());
    stage.keys("escape");
    assert_eq!(stage.graph().naming, None);
}

/// Whichever way the zoom changes, every list is laid out again at once:
/// each row of the graph, the local branches, the changes and a diff as
/// high as the zoom makes it, the next right under it.
fn every_list_is_laid_out_again_at_once_whichever_way_the_zoom_changes() {
    use gpui_kit::{ElementId, base::test_support::snapshots};
    repos::fresh();
    let repo = repos::project();
    // Uncommitted work: the changes panel lists it, and one file's diff
    // has rows.
    let text: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        std::fs::write(repo.join(name), &text).unwrap();
    }
    // The references panel's sections open.
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    stage.until("the uncommitted work is read", |stage| {
        stage.graph().wip_row && stage.shows(("changes-row", 1usize))
    });
    stage.click("wip-row");
    // The rows of a list on show, by their number: those called `name`.
    let rows = |stage: &mut Stage, name: &str| -> Vec<(u64, Place)> {
        let mut found: Vec<(u64, Place)> = stage.window(|window, _| {
            snapshots(window)
                .into_iter()
                .filter(|found| found.visible())
                .filter_map(|found| match found.path().last() {
                    Some(ElementId::NamedInteger(called, n)) if called.as_ref() == name => {
                        Some((*n, place_of(found.bounds())))
                    }
                    _ => None,
                })
                .collect()
        });
        found.sort_by_key(|(n, _)| *n);
        found.dedup_by_key(|(n, _)| *n);
        found
    };
    // Each row as high as it is at the zoom, at least, and the next one
    // right under it: none drawn over another, none with room between.
    let laid_out = |stage: &mut Stage, name: &str, least: f32, zoom: f32, what: &str| {
        let found = rows(stage, name);
        assert!(found.len() >= 2, "{what}: the rows of {name}: {found:?}");
        for (n, row) in &found {
            assert!(
                row.3 >= least * zoom - 0.6,
                "{what}: {name} row {n} is {} high, at least {} at {zoom}",
                row.3,
                least * zoom
            );
        }
        for pair in found.windows(2) {
            if let [(a, above), (b, below)] = pair
                && *b == a + 1
            {
                assert!(
                    (below.1 - (above.1 + above.3)).abs() < 0.6,
                    "{what}: {name} row {b} begins at {}, row {a} ends at {}",
                    below.1,
                    above.1 + above.3
                );
            }
        }
    };
    let zoom = |stage: &mut Stage| {
        stage.app(|cx| cx.global::<bitbraid_ui::prefs::Prefs>().preferences.zoom)
    };
    let every_list = |stage: &mut Stage, what: &str| {
        let at = zoom(stage);
        // The graph's rows, the references panel's local branches, the
        // changes panel's rows.
        laid_out(stage, "row", 28.0, at, what);
        laid_out(stage, "local", 24.0, at, what);
        laid_out(stage, "changes-row", 25.0, at, what);
    };
    every_list(&mut stage, "at the start");

    type Way = (&'static str, f32, fn(&mut Stage));
    let ways: [Way; 5] = [
        ("the status bar's list", 1.5, |stage| {
            stage.click("zoom");
            // From the top: 300, 250, 200, 175, 150.
            stage.keys("down down down down down enter");
        }),
        ("Ctrl+=", 1.6, |stage| stage.keys("ctrl-=")),
        ("Ctrl+-", 1.5, |stage| stage.keys("ctrl--")),
        ("Ctrl+0", 1.0, |stage| stage.keys("ctrl-0")),
        ("the preferences", 1.3, |stage| {
            stage.act(keymap::ShowPreferences);
            stage.click("section-interface");
            stage.click("number-Zoom, in per cent");
            stage.keys("ctrl-a");
            stage.type_text("130");
            stage.click("preferences-close");
        }),
    ];
    for (way, wanted, change) in ways {
        change(&mut stage);
        let at = zoom(&mut stage);
        assert!((at - wanted).abs() < 0.001, "{way}: the zoom is {at}");
        every_list(&mut stage, way);
    }

    // A file's diff, and the zoom changed while it is shown.
    stage.open_changed("a.txt");
    let at = zoom(&mut stage);
    laid_out(&mut stage, "diff-row", 16.0, at, "the diff");
    stage.keys("ctrl-=");
    let at = zoom(&mut stage);
    laid_out(&mut stage, "diff-row", 16.0, at, "the diff, zoomed");
    laid_out(&mut stage, "changes-row", 25.0, at, "the changes, zoomed");
    stage.keys("ctrl-0");
}

/// At 120 % the bars, the graph's rows and the side panels are 1.2 times
/// what they are at 100 %, the window as large as it was; a click lands on
/// the row that is drawn under it, and back at 100 % all is as it was.
fn the_whole_window_grows_with_the_zoom_and_a_click_finds_its_row() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the graph is read", |stage| stage.graph().commits > 2);
    let measures = |stage: &mut Stage| {
        let (_, _, _, tab) = stage.place(("tab-name", 0usize));
        let (_, below_tabs, _, tool) = stage.place("new-branch");
        let (_, status_top, _, status) = stage.place("zoom");
        let (_, first) = stage.graph_spot(0, false);
        let (_, second) = stage.graph_spot(1, false);
        // A heading's button stands at the left panel's right edge, less a
        // margin; the right panel's list of files is as wide as the panel,
        // less a margin.
        let (add_x, _, add, _) = stage.place(("refs-add", 20usize));
        let refs = add_x + add;
        let (_, _, details, _) = stage.place("commit-files");
        [
            tab,
            below_tabs,
            tool,
            HEIGHT - status_top,
            status,
            second - first,
            refs,
            details,
        ]
    };
    let spot = stage.graph_spot(2, false);
    stage.click_at(spot);
    let third = stage.graph().selected.expect("the row is selected");
    let at_100 = measures(&mut stage);

    stage.act(keymap::ZoomIn);
    stage.act(keymap::ZoomIn);
    stage.frame();
    let zoom = stage.app(|cx| cx.global::<bitbraid_ui::prefs::Prefs>().preferences.zoom);
    assert!((zoom - 1.2).abs() < 0.001, "{zoom}");
    let at_120 = measures(&mut stage);
    let (right_was, right_is) = (at_100[7], at_120[7]);
    for (ix, (was, is)) in at_100.iter().zip(at_120.iter()).enumerate().take(7) {
        assert!(
            (is - was * 1.2).abs() <= 1.0,
            "measure {ix}: {was} at 100 %, {is} at 120 %"
        );
    }
    // The right panel grows as well, but this window is 1067 points wide
    // at 120 %: the panel gives way to leave the graph its least, and is
    // 380 points of the 400 it is at 100 %.
    assert!(
        right_is > right_was * 1.1 && right_is <= right_was * 1.2 + 1.0,
        "the right panel: {right_was} at 100 %, {right_is} at 120 %"
    );
    // The window keeps its size: what is in it grows.
    let (x, _, width, _) = stage.place("zoom");
    assert!(x + width <= WIDTH, "{x} {width}");
    // A click goes to the row drawn under it.
    let spot = stage.graph_spot(0, false);
    stage.click_at(spot);
    assert_ne!(stage.graph().selected.as_deref(), Some(third.as_str()));
    let spot = stage.graph_spot(2, false);
    stage.click_at(spot);
    assert_eq!(stage.graph().selected.as_deref(), Some(third.as_str()));

    stage.act(keymap::ZoomReset);
    stage.frame();
    assert_eq!(measures(&mut stage), at_100, "100 % is as it was");
}

/// What the application wrote down for the next start, as it would read
/// it then.
fn saved_state() -> String {
    let root = crate::hermetic::inside().expect("the tests run in scratch surroundings");
    std::fs::read_to_string(root.join("config/bitbraid/state.toml")).expect("the state is written")
}

/// Two tabs, the first shown, the window closed without anything being
/// switched to: the next start shows the first, and so does the start
/// after it, since opening the tabs again must not write down the last one
/// opened as the one shown. And a clean repository comes back on the commit
/// that was selected, with its graph where it was scrolled to, each time.
fn the_shown_tab_and_its_commit_come_back_at_every_start() {
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
    let library = repos::library();
    let state = format!("open = [{project:?}, {library:?}]\nactive = 0\n");
    let mut stage = Stage::open(Start::bare().state(state));
    assert_eq!(stage.shell().active, 0);
    stage.until("the graph is read", |stage| stage.graph().commits > 50);
    stage.keys("ctrl-home");
    for _ in 0..45 {
        stage.keys("down");
    }
    let selected = stage.graph().selected.expect("a commit is selected");
    let top = stage.graph_spot(0, false);
    // A moment for the place the graph is scrolled to to be written down.
    stage.pass(Duration::from_secs(2));
    let mut saved = saved_state();
    drop(stage);

    for start in ["the next start", "the start after it"] {
        let mut stage = Stage::open(Start::bare().state(saved.clone()));
        assert_eq!(stage.shell().active, 0, "{start} shows the first tab");
        stage.until("the graph is read", |stage| stage.graph().commits > 50);
        assert_eq!(
            stage.graph().selected.as_deref(),
            Some(selected.as_str()),
            "{start} selects the commit"
        );
        // Scrolled to the row that was at the top: to within a row.
        let (_, again) = stage.graph_spot(0, false);
        assert!(
            (again - top.1).abs() < 28.0,
            "{start} scrolls where it was: {again} for {}",
            top.1
        );
        stage.pass(Duration::from_secs(2));
        saved = saved_state();
        drop(stage);
    }
}

/// "About BitBraid" (the menu bar's, and the palette's): the logo and what
/// the application is, its version and the git it runs, over the window;
/// the details copied for a report; put away by Escape, by its button and
/// by a click beside it, the keys the graph's again each time.
fn what_the_application_is_lies_over_the_window_and_copies_its_details() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::ShowAbout);
    assert_eq!(stage.shell().sheet, Some("about"));
    for id in [
        "about",
        "about-header",
        "about-version",
        "about-git",
        "about-copy",
    ] {
        assert!(stage.shows(id), "{id} is not on show");
    }
    // The details, as the button copies them: the version, the git, the
    // system.
    stage.click("about-copy");
    let copied = stage
        .app(|cx| cx.read_from_clipboard().and_then(|item| item.text()))
        .unwrap_or_default();
    let lines: Vec<&str> = copied.lines().collect();
    assert_eq!(lines.len(), 3, "{copied:?}");
    assert_eq!(
        lines.first().copied(),
        Some(format!("BitBraid {}", env!("CARGO_PKG_VERSION")).as_str())
    );
    assert!(
        lines.get(1).is_some_and(|git| git.starts_with("Runs git ")),
        "{copied:?}"
    );
    assert_eq!(stage.shell().sheet, Some("about"), "copying leaves it open");

    // Escape puts it away, and the keys are the graph's.
    stage.keys("escape");
    assert_eq!(stage.shell().sheet, None);
    assert_eq!(stage.graph_focus(), (true, false));
    // So does its button.
    stage.act(keymap::ShowAbout);
    assert_eq!(stage.shell().sheet, Some("about"));
    stage.click("about-close");
    assert_eq!(stage.shell().sheet, None);
    assert_eq!(stage.graph_focus(), (true, false));
    // And a click beside it.
    stage.act(keymap::ShowAbout);
    stage.click_in("about-backdrop", (8., 8.));
    assert_eq!(stage.shell().sheet, None);
    // Asked for while it is on show, it goes, as the other dialogs do.
    stage.act(keymap::ShowAbout);
    stage.act(keymap::ShowAbout);
    assert_eq!(stage.shell().sheet, None);
}
