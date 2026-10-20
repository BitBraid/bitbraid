//! What the window does, as `behaviour.rs` asks it, for the terminal inside
//! it: the toolbar's button and the key, the panel under the graph, its
//! heading, tabs and menu, its height, the terminals of each worktree, and
//! what is kept of it per repository.
//!
//! Every terminal here runs `/bin/sh` with a prompt and surroundings of
//! the test's own, never the shell of whoever runs the tests.

use bitbraid_ui::{
    keymap,
    shell::STATUS_BAR_HEIGHT,
    views::{
        rewrite::{Rewrite, RunRewrite},
        worktrees::WorktreeRequest,
    },
};

use crate::{
    repos,
    runner::Test,
    stage::{HEIGHT, Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "the_button_and_the_key_show_and_hide_the_terminal",
        run: the_button_and_the_key_show_and_hide_the_terminal,
    },
    Test {
        name: "minimised_the_terminal_is_its_heading_alone",
        run: minimised_the_terminal_is_its_heading_alone,
    },
    Test {
        name: "hidden_a_terminal_keeps_its_shell_and_ended_it_starts_afresh",
        run: hidden_a_terminal_keeps_its_shell_and_ended_it_starts_afresh,
    },
    Test {
        name: "the_terminals_top_edge_is_dragged_between_its_least_and_the_graphs_rows",
        run: the_terminals_top_edge_is_dragged_between_its_least_and_the_graphs_rows,
    },
    Test {
        name: "another_terminal_is_a_tab_of_the_heading",
        run: another_terminal_is_a_tab_of_the_heading,
    },
    Test {
        name: "each_worktree_has_terminals_of_its_own",
        run: each_worktree_has_terminals_of_its_own,
    },
    Test {
        name: "the_terminal_comes_back_as_it_was_with_the_repository",
        run: the_terminal_comes_back_as_it_was_with_the_repository,
    },
    Test {
        name: "the_terminals_settings_apply_to_the_open_terminals",
        run: the_terminals_settings_apply_to_the_open_terminals,
    },
    Test {
        name: "what_takes_the_graphs_place_puts_the_terminal_away",
        run: what_takes_the_graphs_place_puts_the_terminal_away,
    },
    Test {
        name: "the_menu_of_the_terminal_has_its_entries",
        run: the_menu_of_the_terminal_has_its_entries,
    },
    Test {
        name: "a_folder_of_the_changes_opens_a_terminal_there",
        run: a_folder_of_the_changes_opens_a_terminal_there,
    },
    Test {
        name: "a_shell_that_ends_takes_its_tab_and_the_last_the_panel",
        run: a_shell_that_ends_takes_its_tab_and_the_last_the_panel,
    },
    Test {
        name: "a_branch_made_in_the_terminal_shows_in_the_graph",
        run: a_branch_made_in_the_terminal_shows_in_the_graph,
    },
    Test {
        name: "a_shell_that_cannot_be_started_says_why",
        run: a_shell_that_cannot_be_started_says_why,
    },
    Test {
        name: "the_plan_of_a_rebase_greys_the_terminal",
        run: the_plan_of_a_rebase_greys_the_terminal,
    },
];

/// While the plan of an interactive rebase takes the graph's place the
/// terminal's button is greyed and its key does nothing; afterwards both
/// work again.
fn the_plan_of_a_rebase_greys_the_terminal() {
    repos::fresh();
    let repo = repos::project_with_files();
    let first = repos::rev(&repo, "HEAD~3");
    let mut stage = Stage::open(Start::with(&repo).preferences(SHELL));
    assert!(stage.workspace().loaded);
    stage.act(RunRewrite {
        rewrite: Rewrite::Interactive {
            first,
            reword: false,
        },
    });
    assert!(stage.workspace().planning);
    stage.click("terminal");
    stage.keys("ctrl-`");
    assert!(!stage.terminal().shown, "nothing starts during the plan");
    assert!(stage.terminal().ids.is_empty(), "and no shell was started");
    stage.click("plan-cancel");
    stage.until("the plan is put away", |stage| !stage.workspace().planning);
    stage.click("terminal");
    assert!(stage.terminal().shown, "the button works again");
}

/// `exit` in a shell ends it: its tab goes, and with the last one the
/// panel; the button then starts a new one.
fn a_shell_that_ends_takes_its_tab_and_the_last_the_panel() {
    let (repo, mut stage) = opened();
    stage.click("terminal");
    at_prompt(&mut stage);
    let screen = stage.terminal().screen.unwrap();
    assert_eq!(screen.trim(), "$", "the test's own prompt: {screen:?}");
    // It starts in the repository.
    run(&mut stage, "pwd");
    stage.until("the folder is said", |stage| {
        stage
            .terminal()
            .screen
            .is_some_and(|screen| screen.contains(&*repo.to_string_lossy()))
    });
    stage.click("terminal-new");
    at_prompt(&mut stage);
    assert_eq!(stage.terminal().names.len(), 2);
    run(&mut stage, "exit");
    stage.until("the second shell's tab is gone", |stage| {
        stage.terminal().names.len() == 1
    });
    assert!(stage.terminal().shown);
    assert!(stage.terminal().focused, "the keys go to the one left");
    run(&mut stage, "exit");
    stage.until("the panel goes with the last shell", |stage| {
        !stage.terminal().shown
    });
    assert!(stage.terminal().ids.is_empty());
    assert!(!stage.shows("terminal-panel"));
    stage.click("terminal");
    at_prompt(&mut stage);
    assert_eq!(stage.terminal().names, ["Terminal 1"]);
}

/// What a command typed in the terminal does shows in the graph once it
/// is done.
fn a_branch_made_in_the_terminal_shows_in_the_graph() {
    let (_, mut stage) = opened();
    stage.click("terminal");
    at_prompt(&mut stage);
    let before = stage.workspace().branches;
    run(&mut stage, "git branch typed-in-the-terminal");
    stage.until("the new branch is in the references", |stage| {
        stage.workspace().branches == before + 1
    });
}

/// A shell the preferences name and the system does not have is said in
/// the panel; nothing is started.
fn a_shell_that_cannot_be_started_says_why() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(
        Start::with(&repo).preferences("[terminal_panel]\nshell = \"no-such-shell --login\"\n"),
    );
    stage.click("terminal");
    let failed = stage.terminal();
    assert!(failed.shown, "{failed:?}");
    let why = failed.failed.expect("it says why");
    assert!(why.contains("no-such-shell"), "{why}");
    assert_eq!(failed.running, 0);
    // A quote that is not closed is said before anything is asked.
    stage.app(|cx| {
        cx.global_mut::<bitbraid_ui::prefs::Prefs>()
            .preferences
            .terminal_panel
            .shell = Some("'open".to_owned());
    });
    stage.click("terminal-new");
    let why = stage.terminal().failed.expect("it says why");
    assert!(why.contains("quote"), "{why}");
}

/// The preferences every test here starts with: the shell is `/bin/sh`
/// with a prompt of the test's own, in the test's surroundings (a scratch
/// home without a profile to read).
#[cfg(not(windows))]
pub const SHELL: &str = "[terminal_panel]\nshell = \"env 'PS1=$ ' /bin/sh\"\n";
/// On Windows the `sh` of Git for Windows (and its `env`), found on
/// `PATH`, which has no `/bin/sh`.
#[cfg(windows)]
pub const SHELL: &str = "[terminal_panel]\nshell = \"env 'PS1=$ ' sh\"\n";

/// Wait until the terminal on show is at its prompt.
pub fn at_prompt(stage: &mut Stage) {
    stage.until("the shell is at its prompt", |stage| {
        stage.terminal().screen.is_some_and(|screen| {
            screen
                .trim_end()
                .lines()
                .last()
                .is_some_and(|line| line.trim_end() == "$")
        })
    });
}

/// Type a line into the terminal on show, and Enter.
pub fn run(stage: &mut Stage, line: &str) {
    let ids = stage.terminal().ids;
    let active = stage.terminal().active;
    assert!(
        stage.terminal().focused,
        "the terminal {:?} has the keys",
        ids.get(active)
    );
    stage.type_text(line);
    stage.keys("enter");
}

/// The project, opened with the test's shell.
fn opened() -> (std::path::PathBuf, Stage) {
    repos::fresh();
    let repo = repos::project();
    let stage = Stage::open(Start::with(&repo).preferences(SHELL));
    (repo, stage)
}

/// How high the panel is drawn.
fn panel_height(stage: &mut Stage) -> f32 {
    stage.place("terminal-panel").3
}

/// The toolbar's button and Ctrl+` show the terminal, starting a shell the
/// first time, and put it away again; its shell stays.
fn the_button_and_the_key_show_and_hide_the_terminal() {
    let (repo, mut stage) = opened();
    assert!(!stage.terminal().shown);
    assert!(!stage.shows("terminal-panel"));
    stage.click("terminal");
    let shown = stage.terminal();
    assert!(shown.shown, "{shown:?}");
    assert_eq!(shown.names, ["Terminal 1"]);
    assert_eq!(shown.folders, std::slice::from_ref(&repo));
    assert!(stage.shows("terminal-panel"));
    assert!(stage.terminal().focused, "the keys go to the new terminal");
    // As high as it starts, at the bottom of the graph's column.
    assert_eq!(panel_height(&mut stage), 275.);
    let (_, top, _, height) = stage.place("terminal-panel");
    assert_eq!(
        top + height,
        HEIGHT - STATUS_BAR_HEIGHT,
        "down to the status bar"
    );
    let ids = stage.terminal().ids;

    stage.keys(keymap::binding("terminal.toggle").unwrap().keys[0]);
    assert!(!stage.terminal().shown);
    assert!(!stage.shows("terminal-panel"));
    assert_eq!(stage.terminal().ids, ids, "its shell stays");
    stage.keys("ctrl-`");
    assert!(stage.terminal().shown);
    assert_eq!(stage.terminal().ids, ids, "the same shell comes back");
    stage.click("terminal");
    assert!(!stage.terminal().shown);
}

/// Minimised, the panel is its heading at the bottom of the column, and the
/// chevron brings the rest back.
fn minimised_the_terminal_is_its_heading_alone() {
    let (_, mut stage) = opened();
    stage.click("terminal");
    stage.click("terminal-minimise");
    let minimised = stage.terminal();
    assert!(minimised.shown && minimised.minimised, "{minimised:?}");
    assert_eq!(panel_height(&mut stage), 30.);
    let (_, top, _, height) = stage.place("terminal-panel");
    assert_eq!(top + height, HEIGHT - STATUS_BAR_HEIGHT);
    stage.click("terminal-minimise");
    assert!(!stage.terminal().minimised);
    assert_eq!(panel_height(&mut stage), 275.);
}

/// The cross puts the panel away with its shell running, and the button
/// brings that one back; the bin ends the shell, the panel goes with its
/// last one, and the button then starts a new shell.
fn hidden_a_terminal_keeps_its_shell_and_ended_it_starts_afresh() {
    let (_, mut stage) = opened();
    stage.click("terminal");
    let first = stage.terminal().ids;
    stage.click("terminal-hide");
    assert!(!stage.terminal().shown);
    assert_eq!(stage.terminal().ids, first);
    stage.click("terminal");
    assert_eq!(stage.terminal().ids, first);
    stage.click("terminal-end");
    let ended = stage.terminal();
    assert!(!ended.shown && ended.ids.is_empty(), "{ended:?}");
    assert!(!stage.shows("terminal-panel"));
    stage.click("terminal");
    let again = stage.terminal();
    assert_eq!(again.names, ["Terminal 1"], "numbered from one again");
    assert_eq!(again.ids.len(), 1);
    assert_ne!(again.ids, first, "a new shell");
}

/// Dragged by its top edge, the panel grows until the graph keeps its
/// heading and about two rows, and gets no lower than it starts.
fn the_terminals_top_edge_is_dragged_between_its_least_and_the_graphs_rows() {
    let (repo, mut stage) = opened();
    stage.click("terminal");
    let (x, top, width, _) = stage.place("terminal-panel");
    let (_, column_top, _, column_height) = stage.place("terminal-column");
    let middle = x + width / 2.;
    stage.press_and_move((middle, top), (middle, column_top + 5.));
    stage.release_at((middle, column_top + 5.));
    let most = column_height - 70.;
    assert_eq!(panel_height(&mut stage), most);
    assert_eq!(stage.terminal().height, Some(most));
    // Down past its least.
    let (_, top, _, _) = stage.place("terminal-panel");
    stage.press_and_move((middle, top), (middle, HEIGHT - 30.));
    stage.release_at((middle, HEIGHT - 30.));
    assert_eq!(panel_height(&mut stage), 275.);
    // Somewhere between, and kept for the repository.
    let (_, top, _, _) = stage.place("terminal-panel");
    stage.press_and_move((middle, top), (middle, top - 100.));
    stage.release_at((middle, top - 100.));
    assert_eq!(panel_height(&mut stage), 375.);
    let kept = stage.app(|cx| {
        cx.global::<bitbraid_ui::prefs::Prefs>()
            .repo_state(&repo)
            .terminal_height
    });
    assert_eq!(kept, Some(375.));
}

/// The plus starts another terminal in the repository: the terminals are
/// then tabs of the heading, a click shows one, the keys go round them,
/// and closing the one on show shows another.
fn another_terminal_is_a_tab_of_the_heading() {
    let (_, mut stage) = opened();
    stage.click("terminal");
    assert!(stage.shows("terminal-new"));
    assert!(!stage.shows("terminal-tabs"), "no tabs for one terminal");
    stage.click("terminal-new");
    let two = stage.terminal();
    assert_eq!(two.names, ["Terminal 1", "Terminal 2"]);
    assert_eq!(two.active, 1, "the new one is on show");
    assert!(stage.shows("terminal-tabs"));
    let first = two.ids[0];
    stage.click(("terminal-tab", first));
    assert_eq!(stage.terminal().active, 0);
    stage.act(keymap::NextTerminal);
    assert_eq!(stage.terminal().active, 1);
    stage.act(keymap::NextTerminal);
    assert_eq!(stage.terminal().active, 0, "round again");
    stage.act(keymap::PrevTerminal);
    assert_eq!(stage.terminal().active, 1);
    stage.act(keymap::NewTerminal);
    assert_eq!(
        stage.terminal().names,
        ["Terminal 1", "Terminal 2", "Terminal 3"]
    );
    // The cross of a tab shows while the pointer is on it.
    let (x, y, w, h) = stage.place(("terminal-tab", first));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.click(("terminal-tab-close", first));
    let closed = stage.terminal();
    assert_eq!(closed.names, ["Terminal 2", "Terminal 3"]);
    assert!(closed.shown);
    stage.act(keymap::CloseTerminal);
    assert_eq!(stage.terminal().names.len(), 1);
    assert!(!stage.shows("terminal-tabs"));
}

/// A worktree opened in the tab's place has terminals of its own, started
/// in its folder; going back finds the first one as it was. Closing the
/// tab ends both.
fn each_worktree_has_terminals_of_its_own() {
    let (repo, mut stage) = opened();
    let other = repos::work().join("project-other");
    repos::git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "other",
            other.to_str().unwrap(),
        ],
    );
    stage.click("terminal");
    let main_ids = stage.terminal().ids;
    let open_here = |stage: &mut Stage, path: &std::path::Path| {
        let path = path.to_string_lossy().into_owned();
        stage.with_workspace(|workspace, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.worktree(WorktreeRequest::OpenHere(path), window, cx)
            })
        });
        stage.frame();
    };
    open_here(&mut stage, &other);
    stage.until("the worktree is open in the tab", |stage| {
        stage.shell().tabs[0].path.as_deref() == Some(other.as_path()) && stage.workspace().loaded
    });
    assert_eq!(stage.shell().tabs.len(), 1);
    let theirs = stage.terminal();
    // It looks as the main one did: shown, with a shell of its own there.
    assert!(theirs.shown, "{theirs:?}");
    assert_eq!(theirs.folders, std::slice::from_ref(&other));
    assert_ne!(theirs.ids, main_ids);
    assert_eq!(theirs.window_folders.len(), 2);
    open_here(&mut stage, &repo);
    stage.until("the main worktree is back", |stage| {
        stage.shell().tabs[0].path.as_deref() == Some(repo.as_path()) && stage.workspace().loaded
    });
    assert_eq!(stage.terminal().ids, main_ids, "the first one, as it was");
    stage.act(keymap::CloseTab);
    assert!(stage.terminal().window_folders.is_empty());
}

/// Shown, minimised or not, and how high: kept for the repository and as
/// it was when it is opened again, with a new shell.
fn the_terminal_comes_back_as_it_was_with_the_repository() {
    repos::fresh();
    let repo = repos::project();
    let state = format!("[repos.{repo:?}]\nterminal_shown = true\nterminal_height = 400.0\n");
    let mut stage = Stage::open(Start::with(&repo).preferences(SHELL).state(state));
    let back = stage.terminal();
    assert!(back.shown, "{back:?}");
    assert_eq!(back.names, ["Terminal 1"]);
    assert_eq!(panel_height(&mut stage), 400.);
    at_prompt(&mut stage);
    // Minimised, it comes back minimised.
    stage.click("terminal-minimise");
    drop(stage);
    let state = std::fs::read_to_string(
        crate::hermetic::inside()
            .unwrap()
            .join("config/bitbraid/state.toml"),
    )
    .unwrap();
    let mut stage = Stage::open(Start::with(&repo).preferences(SHELL).state(state));
    let back = stage.terminal();
    assert!(back.shown && back.minimised, "{back:?}");
    assert_eq!(panel_height(&mut stage), 30.);
}

/// What the section of the preferences says is what the open terminals
/// are drawn with, at once.
fn the_terminals_settings_apply_to_the_open_terminals() {
    let (_, mut stage) = opened();
    stage.click("terminal");
    let look = stage.terminal().look.expect("drawn");
    assert_eq!(look.font_size, 12.);
    assert_eq!(look.line_height, 1.2);
    stage.act(keymap::ShowPreferences);
    stage.click("section-terminal");
    assert_eq!(stage.shell().preferences_section, Some("terminal"));
    assert!(stage.shows("select-Cursor"));
    assert!(stage.shows("field-Shell"));
    stage.app(|cx| {
        let prefs = cx.global_mut::<bitbraid_ui::prefs::Prefs>();
        prefs.preferences.terminal_panel.font_size = 15.;
        prefs.preferences.terminal_panel.cursor = "block".to_owned();
        cx.refresh_windows();
    });
    stage.click("preferences-close");
    let look = stage.terminal().look.expect("drawn");
    assert_eq!(look.font_size, 15.);
    assert_eq!(
        look.cursor,
        bitbraid_ui::views::terminal::look::Cursor::Block
    );
}

/// A file in the middle takes the graph's place and the panel with it;
/// closing the file brings both back.
fn what_takes_the_graphs_place_puts_the_terminal_away() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo).preferences(SHELL));
    stage.click("terminal");
    assert!(stage.shows("terminal-panel"));
    stage.open_changed("src/render.rs");
    assert!(stage.workspace().file.is_some());
    assert!(!stage.shows("terminal-panel"));
    assert!(stage.terminal().shown, "still on show under the file");
    stage.act(keymap::FocusGraph);
    assert!(stage.workspace().file.is_none());
    assert!(stage.shows("terminal-panel"));
}

/// A click of the other button in the text opens the terminal's menu, in
/// its groups.
fn the_menu_of_the_terminal_has_its_entries() {
    let (_, mut stage) = opened();
    stage.click("terminal");
    let id = stage.terminal().ids[0];
    stage.right_click(("terminal-text", id));
    assert_eq!(
        stage.terminal().menu,
        [
            "Copy",
            "Paste",
            "Select All",
            "Delete Line",
            "Clear",
            "Minimise",
            "End the session",
            "Hide"
        ]
    );
    stage.keys("escape");
    assert!(stage.terminal().menu.is_empty());
}

/// A folder of the changes, shown as a tree, starts a terminal in itself.
fn a_folder_of_the_changes_opens_a_terminal_there() {
    use bitbraid_ui::views::file_actions::FileAction;
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo).preferences(SHELL));
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.file_action(
                FileAction::Terminal {
                    path: "src".to_owned(),
                },
                window,
                cx,
            )
        })
    });
    stage.frame();
    let opened = stage.terminal();
    assert!(opened.shown);
    assert_eq!(opened.folders, [repo.join("src")]);
    // One that is not there starts where the nearest folder above is.
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.file_action(
                FileAction::Terminal {
                    path: "gone/away".to_owned(),
                },
                window,
                cx,
            )
        })
    });
    stage.frame();
    assert_eq!(stage.terminal().folders, [repo.join("src"), repo.clone()]);
}
