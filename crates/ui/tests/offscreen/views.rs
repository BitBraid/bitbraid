//! Every view, brought into the state that is to be looked at and handed
//! over to have its picture taken. Before each picture the view is asked
//! what it shows, so that a picture is never of something else than its
//! name says.

use std::time::Duration;

use gpui_kit::{ElementId, test::TestWindowExt as _};

use bitbraid_ui::{
    keymap,
    views::{
        flow::FlowRequest,
        rewrite::{Rewrite, RunRewrite},
        tags::{RunTag, TagRequest},
    },
};

use crate::{
    repos,
    runner::{Shots, Themes, Views},
    stage::{Stage, Start},
};

/// The views, in the order they are drawn.
pub const VIEWS: &[Views] = &[
    Views {
        name: "start",
        run: start,
        needs: &[],
    },
    Views {
        name: "window",
        run: window,
        needs: &[],
    },
    Views {
        name: "workspace",
        run: workspace,
        needs: &[],
    },
    Views {
        name: "labels",
        run: labels,
        needs: &[],
    },
    Views {
        name: "graph_tips",
        run: graph_tips,
        needs: &[],
    },
    Views {
        name: "pictures",
        run: crate::pictures::pictures,
        needs: &[],
    },
    Views {
        name: "updates",
        run: crate::updates::pictures,
        needs: &[],
    },
    Views {
        name: "references",
        run: references,
        needs: &[],
    },
    Views {
        name: "dialogs",
        run: dialogs,
        needs: &[],
    },
    Views {
        name: "changes",
        run: changes,
        needs: &[],
    },
    Views {
        name: "file",
        run: file,
        needs: &[],
    },
    Views {
        name: "line_endings",
        run: line_endings,
        needs: &[],
    },
    Views {
        name: "windows_banner",
        run: windows_banner,
        needs: &[],
    },
    Views {
        name: "conflict",
        run: conflict,
        needs: &[],
    },
    Views {
        name: "rebase",
        run: rebase,
        needs: &[],
    },
    Views {
        name: "sheets",
        run: sheets,
        needs: &[],
    },
    Views {
        name: "signing",
        run: signing,
        needs: &["gpg", "ssh-keygen"],
    },
    Views {
        name: "lfs",
        run: lfs,
        needs: &["git-lfs"],
    },
    Views {
        name: "read_only",
        run: read_only,
        needs: &[],
    },
    Views {
        name: "menus",
        run: menus,
        needs: &[],
    },
    Views {
        name: "asking",
        run: asking,
        needs: &[],
    },
    Views {
        name: "signatures",
        run: signatures,
        needs: &["ssh-keygen"],
    },
    Views {
        name: "co_authors",
        run: crate::behaviour_fit::co_author_pictures,
        needs: &[],
    },
    Views {
        name: "narrow",
        run: crate::behaviour_fit::narrow_pictures,
        needs: &[],
    },
    Views {
        name: "hooks",
        run: hooks,
        needs: &[],
    },
    Views {
        name: "repositories",
        run: repositories,
        needs: &[],
    },
    Views {
        name: "workspaces",
        run: crate::workspaces::pictures,
        needs: &[],
    },
    Views {
        name: "start_workspaces",
        run: crate::workspaces::start_pictures,
        needs: &[],
    },
    Views {
        name: "upkeep",
        run: crate::behaviour_upkeep::pictures,
        needs: &[],
    },
    Views {
        name: "inbox",
        run: crate::inbox::pictures,
        needs: &[],
    },
    Views {
        name: "docked",
        run: docked,
        needs: &[],
    },
    Views {
        name: "integrations",
        run: crate::behaviour_providers::pictures,
        needs: &[],
    },
    Views {
        name: "clone_provider",
        run: crate::behaviour_providers::clone_pictures,
        needs: &[],
    },
    Views {
        name: "fork_provider",
        run: crate::behaviour_providers::fork_pictures,
        needs: &[],
    },
    Views {
        name: "owner_pictures",
        run: crate::behaviour_providers::owner_pictures,
        needs: &[],
    },
    Views {
        name: "terminal_text",
        run: crate::behaviour_terminal_text::terminal_text_pictures,
        needs: &["sh"],
    },
    Views {
        name: "terminal",
        run: terminal,
        needs: &["sh"],
    },
    Views {
        name: "terminal_completion",
        run: crate::behaviour_terminal_completion::completion_pictures,
        needs: &["sh"],
    },
    Views {
        name: "pulls",
        run: crate::pulls::pulls,
        needs: &[],
    },
    Views {
        name: "pull_writes",
        run: crate::pull_writes::pull_writes,
        needs: &[],
    },
    Views {
        name: "pull_suggestions",
        run: crate::pull_suggestions::pull_suggestions,
        needs: &[],
    },
    Views {
        name: "pull_view_writes",
        run: crate::pull_writes::pull_view_writes,
        needs: &[],
    },
    Views {
        name: "issues",
        run: crate::issues::issues,
        needs: &[],
    },
];

/// The terminal under the graph: with a shell at its prompt, with two
/// tabs, minimised, and the section of the preferences.
fn terminal(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(
        Start::with(&repo)
            .preferences(crate::behaviour_terminal::SHELL)
            .pictures(),
    );
    stage.click("terminal");
    let shown = stage.terminal();
    assert!(shown.shown && shown.names.len() == 1, "{shown:?}");
    crate::behaviour_terminal::at_prompt(&mut stage);
    crate::behaviour_terminal::run(&mut stage, "git log --oneline -3");
    stage.until("git has answered", |stage| {
        stage
            .terminal()
            .screen
            .is_some_and(|screen| screen.trim_end().lines().count() >= 5)
    });
    crate::behaviour_terminal::at_prompt(&mut stage);
    shots.take(&mut stage, "terminal", Themes::All);

    stage.click("terminal-new");
    assert_eq!(stage.terminal().names.len(), 2);
    crate::behaviour_terminal::at_prompt(&mut stage);
    shots.take(&mut stage, "terminal_tabs", Themes::Two);

    stage.click("terminal-minimise");
    assert!(stage.terminal().minimised);
    shots.take(&mut stage, "terminal_minimised", Themes::Two);

    stage.act(keymap::ShowPreferences);
    stage.click("section-terminal");
    assert_eq!(stage.shell().preferences_section, Some("terminal"));
    shots.take(&mut stage, "preferences_terminal", Themes::Two);
}

/// The window as it is with nothing to open, and the two dialogs that
/// make a repository.
fn start(shots: &mut Shots) {
    repos::fresh();
    // Something in the lists: a repository that was open before.
    let known = repos::project();
    let state = format!(
        "recent = [{known:?}, \"/tmp/bitbraid-offscreen/work/gone\"]\nfavourites = [{known:?}]\n"
    );
    let mut stage = Stage::open(Start::bare().state(state).pictures());
    let shell = stage.shell();
    assert_eq!(shell.tabs.len(), 1);
    assert_eq!(shell.tabs[0].path, None, "a start page");
    // The page begins under the tabs: the toolbar is a repository's.
    assert!(stage.shows("browse") && stage.shows("clone") && stage.shows("init"));
    assert!(!stage.shows("pull-more"), "no toolbar on the start page");
    shots.take(&mut stage, "start", Themes::Three);

    stage.click("clone");
    assert!(stage.dialog_open(), "the clone dialog");
    stage.type_text("https://example.invalid/team/library.git");
    shots.take(&mut stage, "dialog_clone", Themes::Two);
    stage.click("form-close");
    assert!(!stage.dialog_open());

    stage.click("init");
    assert!(stage.dialog_open(), "the dialog for a new repository");
    shots.take(&mut stage, "dialog_new_repository", Themes::Two);
}

/// The views of the window's own: preferences, keys, notifications.
fn window(shots: &mut Shots) {
    repos::fresh();
    // Started with a folder that is no repository: that is said in a
    // notification, which the history then has to show.
    let nothing = repos::work().join("nothing");
    std::fs::create_dir_all(&nothing).unwrap();
    let mut stage = Stage::open(Start::with(&nothing).pictures());
    assert_eq!(stage.notices().len(), 1, "{:?}", stage.notices());
    // Under the pointer, with its cross.
    stage.hover("notification");
    shots.take(&mut stage, "notification", Themes::Two);
    // Something to know stays until it is put away; what was said is
    // kept.
    stage.pass(Duration::from_secs(30));
    assert!(stage.shows_toast(), "it stays");
    stage.close_toast();
    stage.pass(Duration::from_secs(1));
    assert!(!stage.shows_toast());
    // Where that button was depends on how many lines the notice took.
    stage.pointer_away();

    stage.act(keymap::ShowPreferences);
    assert_eq!(stage.shell().sheet, Some("preferences"));
    shots.take(&mut stage, "preferences", Themes::Three);

    stage.act(keymap::ChangeKeys);
    assert_eq!(stage.shell().sheet, Some("shortcuts"));
    shots.take(&mut stage, "shortcuts", Themes::Two);

    // The list of keys, a dialog over the window.
    stage.act(keymap::ShowShortcuts);
    assert_eq!(stage.shell().sheet, Some("keys"));
    shots.take(&mut stage, "key_list", Themes::Two);

    stage.act(keymap::ShowNotifications);
    assert_eq!(stage.shell().sheet, Some("notifications"));
    shots.take(&mut stage, "notifications", Themes::Two);
}

/// A repository: references, graph and the details of a commit; the
/// palette, the search and several commits selected.
fn workspace(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    let workspace = stage.workspace();
    assert!(
        workspace.loaded && workspace.error.is_none(),
        "{workspace:?}"
    );
    let graph = stage.graph();
    assert_eq!(graph.commits, 16);
    assert!(
        graph.selected.is_some(),
        "a commit is selected at the start"
    );
    shots.take(&mut stage, "workspace", Themes::All);

    // The same at 120 %: everything in the window drawn larger.
    stage.act(keymap::ZoomIn);
    stage.act(keymap::ZoomIn);
    shots.take(&mut stage, "workspace_zoom_120", Themes::Two);
    stage.act(keymap::ZoomReset);

    stage.act(keymap::TogglePalette);
    assert!(stage.shell().palette_open);
    stage.type_text("branch");
    shots.take(&mut stage, "palette", Themes::Two);
    stage.keys("escape");
    assert!(!stage.shell().palette_open);

    // The menu beside Pull, from the toolbar.
    stage.click("pull-more");
    shots.take(&mut stage, "menu_pull", Themes::Two);
    stage.keys("escape");

    // The list of the tabs, from the chevron at the right end of the tabs.
    stage.click("tab-list");
    assert!(stage.shows(("tab-list-row", 0usize)));
    shots.take(&mut stage, "tab_list", Themes::Two);
    stage.keys("escape");
    assert!(!stage.shows(("tab-list-row", 0usize)));

    // The branches, from the branch's block at the toolbar's left.
    stage.click("crumb-branch");
    assert!(stage.shows(("branch-finder-row", 0usize)));
    shots.take(&mut stage, "branch_finder", Themes::Two);
    stage.keys("escape");
    assert!(!stage.shows(("branch-finder-row", 0usize)));

    stage.act(keymap::FocusSearch);
    stage.type_text("lexer");
    let graph = stage.graph();
    assert!(graph.search_open && graph.matches >= 2, "{graph:?}");
    shots.take(&mut stage, "search", Themes::Two);
    stage.keys("escape");
    assert_eq!(
        stage.graph_focus(),
        (true, false),
        "Escape gives the keys back to the graph"
    );
    stage.click("search-close");
    assert!(!stage.graph().search_open);

    // Two commits together, the oldest two: the right pane says what
    // they change between them.
    stage.keys("ctrl-end shift-up");
    assert_eq!(stage.graph().picked, 2);
    shots.take(&mut stage, "commits_compared", Themes::Two);
}

/// Rows of the graph with more labels than their column has room for at
/// its default width: as many as are readable, the rest folded into a
/// "+N". The first row's two labels are whole, not "m…" and "v…".
fn labels(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::fixture("branches-merges", "project");
    // The checked-out branch and a tag are there; beside them a long name
    // with a short one, three short ones, and two of two letters.
    repos::git(&repo, &["tag", "v2", "feature/b"]);
    for (kind, name) in [("branch", "wip"), ("branch", "pages"), ("tag", "v0.2")] {
        repos::git(&repo, &[kind, name, "feature/a~1"]);
    }
    repos::git(&repo, &["branch", "ab", "main~2"]);
    repos::git(&repo, &["tag", "cd", "main~2"]);
    repos::git(
        &repo,
        &["branch", "feature/a-very-long-name-indeed", "main~3"],
    );
    let mut stage = Stage::open(Start::with(&repo).pictures());
    let workspace = stage.workspace();
    assert!(
        workspace.loaded && workspace.error.is_none(),
        "{workspace:?}"
    );
    assert_eq!(stage.graph().commits, 7);
    shots.take(&mut stage, "graph_labels", Themes::Two);

    // The pointer on the labels of the commit with three: they unfold in
    // a column down over the rows below, over everything there, the labels
    // of those rows too, which do not show through; the branch's run is
    // lit, and what the other rows say has stepped back. In every theme.
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    let Some(row) = (0..rows).find(|row| stage.graph_labels(*row).len() == 3) else {
        panic!("no commit with three labels");
    };
    let on = stage.graph_spot(row, true);
    stage.point_at(on);
    stage.pass(Duration::from_millis(300));
    let graph = stage.graph();
    assert_eq!(graph.unfolded, Some(row), "{graph:?}");
    assert!(!graph.lit.is_empty() && graph.fade == 100, "{graph:?}");
    shots.take(&mut stage, "graph_labels_unfolded", Themes::All);

    // The pointer resting on a label whose name is cut: the label widened
    // to its whole name over the graph and the messages, opaque, nothing
    // else moved, the branch lit.
    let long = "feature/a-very-long-name-indeed";
    let Some((row, index)) = (0..rows).find_map(|row| {
        let names = stage.graph_labels(row);
        names
            .iter()
            .position(|name| name == long)
            .map(|at| (row, at))
    }) else {
        panic!("no row with {long}");
    };
    stage.point_at((640.0, 700.0));
    let (x, y) = stage.graph_spot(row, true);
    stage.point_at((x, y));
    // In the column the row's labels unfold into, when it is not the
    // first of them.
    stage.point_at((x, y + 24.0 * index as f32));
    stage.pass(Duration::from_millis(300));
    let graph = stage.graph();
    assert_eq!(graph.widened, Some((row, index)), "{graph:?}");
    shots.take(&mut stage, "graph_labels_widened", Themes::Two);
}

/// What the rows of the graph say under the pointer: the circle of a
/// commit whose committer is not its author names both.
fn graph_tips(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let guest = repos::git(
        &repo,
        &["log", "-1", "--format=%H", ":/Empty commit by a guest"],
    )
    .trim()
    .to_owned();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert_eq!(stage.graph().commits, 16);
    // Its row, found by selecting the rows one after the other.
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    let row = (0..rows)
        .find(|row| {
            let spot = stage.graph_spot(*row, false);
            stage.click_at(spot);
            stage.graph().selected.as_deref() == Some(guest.as_str())
        })
        .expect("the guest's commit is listed");
    let (x, y, wide, high) = stage.place(("author-circle", row));
    stage.point_at((x + wide / 2.0, y + high / 2.0));
    stage.pass(Duration::from_millis(700));
    let said = stage.window(|window, _| {
        window
            .try_find("author-tip")
            .filter(|tip| tip.visible())
            .and_then(|tip| tip.label().map(str::to_owned))
    });
    assert!(
        said.as_deref()
            .is_some_and(|said| said.contains("\nCommitted by ")),
        "{said:?}"
    );
    shots.take(&mut stage, "graph_author_tip", Themes::Two);
    drop(stage);

    // A commit's whole message over its row: the summary, an empty line
    // and the body with its own lines, a long one wrapped.
    repos::fresh();
    let repo = repos::project();
    repos::git(
        &repo,
        &[
            "commit",
            "--allow-empty",
            "-m",
            "Explain how the parser reads a line",
            "-m",
            "It reads words and quotes.\nEscapes are read as the shell reads them, \
             which is a longer line than the tooltip is wide, so that it wraps.",
            "-m",
            "- one\n- two",
        ],
    );
    let head = repos::git(&repo, &["rev-parse", "HEAD"]).trim().to_owned();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert_eq!(stage.graph().commits, 17);
    let rows = stage.graph().commits + usize::from(stage.graph().wip_row);
    let row = (0..rows)
        .find(|row| {
            let spot = stage.graph_spot(*row, false);
            stage.click_at(spot);
            stage.graph().selected.as_deref() == Some(head.as_str())
        })
        .expect("the new commit is listed");
    let on = stage.graph_spot(row, false);
    stage.point_at(on);
    stage.pass(Duration::from_millis(700));
    let said = stage.window(|window, _| {
        window
            .try_find("message-tip")
            .filter(|tip| tip.visible())
            .and_then(|tip| tip.label().map(str::to_owned))
    });
    assert!(
        said.as_deref()
            .is_some_and(|said| said.starts_with("Explain how the parser reads a line\n\n")),
        "{said:?}"
    );
    shots.take(&mut stage, "graph_message_tip", Themes::Two);
}

/// Every section of the references panel with something in it, and open:
/// a remote, a second worktree, a stash, tags, a submodule.
fn references(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project_with_everything();
    // What somebody wrote about a branch stands under its name: of a
    // branch the filter further down finds, where its row has room.
    repos::git(
        &repo,
        &[
            "config",
            "branch.feature/lexer.description",
            "Quotes and escapes; Bob reviews",
        ],
    );
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state).pictures());
    let workspace = stage.workspace();
    assert!(
        workspace.loaded && workspace.error.is_none(),
        "{workspace:?}"
    );
    assert_eq!(
        workspace.branches, 7,
        "the fixture's six and the worktree's"
    );
    shots.take(&mut stage, "references", Themes::Three);

    // The menu of a branch, opened with a click of the other button on
    // its name.
    let name = stage.window(|window, _| {
        window
            .try_find(ElementId::Name("ref-name:refs/heads/feature/dates".into()))
            .filter(|found| found.visible())
            .map(|found| found.bounds().center())
    });
    let name = name.expect("the branch's row is on show");
    stage.right_click_at((f32::from(name.x), f32::from(name.y)));
    stage.settle();
    shots.take(&mut stage, "menu_branch", Themes::Two);
    stage.keys("escape");

    // The panel filtered down to what was typed.
    stage.act(keymap::FilterReferences);
    stage.type_text("lex");
    shots.take(&mut stage, "references_filtered", Themes::Two);
}

/// What is asked and said over a repository: a prompt, the files found
/// by their path, the details of a tag, an alert, a command's parameters.
fn dialogs(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert!(stage.workspace().loaded);

    // A new branch is named in the graph, in the label column of the
    // commit's row.
    stage.act(keymap::NewBranch);
    assert!(
        stage.graph().naming.is_some(),
        "the field for a branch name"
    );
    stage.type_text("feature/wrap");
    shots.take(&mut stage, "graph_naming", Themes::Two);
    stage.keys("escape");
    assert!(stage.graph().naming.is_none());
    stage.pass(Duration::from_secs(1));

    // A question in the bar that takes the toolbar's place: a tab's name.
    stage.act(keymap::RenameTab);
    stage.type_text("work");
    assert!(
        stage.shell().bar.is_some(),
        "the tab's name is asked in the bar"
    );
    shots.take(&mut stage, "bar_prompt", Themes::Two);
    stage.keys("escape");
    assert!(stage.shell().bar.is_none());

    // A file found by its path, with its key.
    stage.keys("alt-p");
    assert!(stage.dialog_open(), "the dialog that finds a file");
    stage.type_text("lex");
    stage.pass(Duration::from_secs(1));
    shots.take(&mut stage, "find_file", Themes::Two);
    stage.keys("escape");
    assert!(!stage.dialog_open());
    stage.pass(Duration::from_secs(1));

    stage.act(RunTag {
        request: TagRequest::Details("v0.2".to_owned()),
    });
    assert!(stage.dialog_open(), "the details of a tag");
    shots.take(&mut stage, "tag_details", Themes::Two);
    stage.keys("escape");
    assert!(!stage.dialog_open());

    // A confirmation in the bar: before a tag is deleted.
    stage.act(RunTag {
        request: TagRequest::Delete("v0.1".to_owned()),
    });
    assert!(
        stage.shell().bar.is_some() && !stage.dialog_open(),
        "the question before a tag is deleted is asked in the bar"
    );
    shots.take(&mut stage, "bar_confirm", Themes::Two);
    stage.keys("escape");
    assert!(stage.shell().bar.is_none());
    stage.pass(Duration::from_secs(1));

    // A confirmation that lists what would go stays a dialog: the
    // branches that are merged.
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.flow(FlowRequest::DeleteMerged, window, cx)
        })
    });
    stage.until("the question before merged branches go", |stage| {
        stage.dialog_open()
    });
    shots.take(&mut stage, "dialog_alert", Themes::Three);
    stage.keys("escape");
    assert!(!stage.dialog_open());
    stage.pass(Duration::from_secs(1));

    // (A dialog that asks several things, each in a row of its own, is
    // the clone dialog of the start page; a submodule's is asked in the
    // pane beside the panel: `docked`.)

    // A command of the engine's, chosen in the palette by its id: the
    // prompt for what it takes is made from what the engine says.
    stage.act(keymap::TogglePalette);
    stage.type_text("tag.create");
    stage.keys("enter");
    assert_eq!(stage.shell().asking_for, Some("tag.create"));
    shots.take(&mut stage, "dialog_command", Themes::Two);
}

/// Uncommitted work: its row in the graph, the changes panel and the
/// commit box.
fn changes(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    let workspace = stage.workspace();
    assert!(
        workspace.loaded && workspace.status_paths >= 5,
        "{workspace:?}"
    );
    assert!(stage.graph().wip_row);
    stage.keys("ctrl-home");
    assert!(stage.graph().wip_selected);
    assert!(stage.workspace().changes_shown);
    shots.take(&mut stage, "changes", Themes::Three);

    // The same as a tree of folders.
    stage.focus_changes();
    stage.keys("t");
    shots.take(&mut stage, "changes_tree", Themes::Two);
}

/// A file in the centre: its hunks, the whole file in one column and in
/// two, the file as it is, a picture, a submodule, its history, who wrote
/// its lines, and the editor.
fn file(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    stage.keys("ctrl-home");
    assert!(stage.graph().wip_selected);

    stage.open_changed("src/render.rs");
    let workspace = stage.workspace();
    assert_eq!(workspace.file_tab, "diff");
    assert!(stage.shows("mode-split"));
    shots.take(&mut stage, "diff", Themes::All);

    stage.click("mode-inline");
    shots.take(&mut stage, "diff_one_column", Themes::Two);
    stage.click("mode-split");
    shots.take(&mut stage, "diff_side_by_side", Themes::Two);
    stage.click("mode-hunks");

    // The file as it is: one number column, nothing marked.
    stage.click("view-file");
    assert!(!stage.shows("mode-split"));
    shots.take(&mut stage, "diff_whole_file", Themes::Two);
    stage.click("view-diff");
    assert!(stage.shows("mode-split"));

    // The file's past takes the window's width: the commits at the left,
    // what the first of them changed beside them.
    stage.click("tab-history");
    assert_eq!(stage.workspace().file_tab, "history");
    assert!(stage.shows("past-diff") && stage.shows("close-past"));
    assert!(!stage.shows("tab-history"), "the file pane made room");
    shots.take(&mut stage, "file_history", Themes::Two);

    // Blame is the same view: the file whole, and the switch on.
    stage.click("past-file");
    assert!(stage.shows("past-blame"));
    stage.click("past-blame");
    assert_eq!(stage.workspace().file_tab, "blame");
    shots.take(&mut stage, "blame", Themes::Two);

    // Closing it goes back to the file as the pane had it.
    stage.click("close-past");
    assert_eq!(stage.workspace().file_tab, "diff");
    assert!(stage.shows("tab-history"));

    stage.click("tab-edit");
    assert_eq!(stage.workspace().file_tab, "edit");
    // The colours of the text may be put off (see `PUT_OFF`): on a busy
    // machine the text is drawn plain until then.
    stage.pass(crate::stage::PUT_OFF);
    shots.take(&mut stage, "editor", Themes::Two);

    stage.open_changed("assets/logo.png");
    assert_eq!(stage.workspace().file_tab, "diff");
    shots.take(&mut stage, "diff_image", Themes::Two);
    // The last of the switches that lay one over the other: what differs.
    stage.click(("image-mode", 2usize));
    shots.take(&mut stage, "diff_image_difference", Themes::Two);

    stage.open_changed("vendor/library");
    shots.take(&mut stage, "diff_submodule", Themes::Two);
}

/// Line endings: a file whose line endings alone changed says so in place
/// of its lines, which come when asked for; the header says how the lines
/// end where that matters, and a changed line whose ending is not the
/// other side's is marked.
fn line_endings(shots: &mut Shots) {
    let shown = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).diff().read(cx).probe())
    };
    repos::fresh();
    let repo = repos::project_with_line_endings();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    stage.keys("ctrl-home");
    assert!(stage.graph().wip_selected);

    stage.open_changed("docs/notes.txt");
    stage.until("the notes are compared", |stage| {
        let probe = shown(stage);
        !probe.loading && probe.endings_only
    });
    let probe = shown(&mut stage);
    assert_eq!(
        probe.line_endings.as_deref(),
        Some("LF → CRLF"),
        "{probe:?}"
    );
    assert_eq!((probe.added, probe.removed, probe.lines), (3, 3, 0));
    assert!(stage.shows("diff-endings-lines") && stage.shows("diff-line-endings"));
    shots.take(&mut stage, "diff_line_endings_only", Themes::Two);
    // Asked for, the lines are there, every one of them changed.
    stage.click("diff-endings-lines");
    let probe = shown(&mut stage);
    assert!(!probe.endings_only && probe.lines > 0, "{probe:?}");

    stage.open_changed("scripts/build.cmd");
    stage.until("the script is compared", |stage| {
        let probe = shown(stage);
        !probe.loading && probe.added == 2
    });
    let probe = shown(&mut stage);
    assert_eq!(probe.line_endings.as_deref(), Some("CRLF → LF + CRLF"));
    assert!(!probe.endings_only);
    shots.take(&mut stage, "diff_line_endings_marked", Themes::Two);
}

/// What Windows has to say of a repository: paths longer than it takes
/// while git's long paths are off, and a repository inside WSL. Nothing is
/// said on this system, so the notes are handed to the tab as Windows
/// would have them. What its buttons do is a test of behaviour
/// (`behaviour_upkeep.rs`): run here, the command would be in the
/// activity log of a later picture.
fn windows_banner(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert!(!stage.shows("platform-banner"), "nothing to say here");
    crate::behaviour_upkeep::show_windows_notes(&mut stage);
    assert!(stage.shows("platform-banner") && stage.shows("platform-long-paths"));
    shots.take(&mut stage, "banner_windows", Themes::Two);
}

/// A merge that stopped: the ways out under the commit form, the strip
/// over a commit's details, and the file in the merge tool.
fn conflict(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project_in_conflict();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    let workspace = stage.workspace();
    assert_eq!(
        workspace.in_progress,
        Some(("merge".to_owned(), 1)),
        "{workspace:?}"
    );
    stage.keys("ctrl-home");
    assert!(stage.graph().wip_selected);
    // With the panel of uncommitted work on show the two ways out are
    // under its commit form, and there is no banner to say it twice.
    assert!(stage.shows("op-continue") && stage.shows("op-abort"));
    assert!(!stage.shows("progress-continue"));
    shots.take(&mut stage, "conflict_banner", Themes::Three);
    // With a commit selected, the strip over its details says how many
    // files are in conflict, and there is no banner either.
    stage.keys("down");
    assert!(stage.shows("view-changes") && !stage.shows("progress-continue"));
    shots.take(&mut stage, "conflict_details", Themes::Two);
    stage.keys("ctrl-home");
    assert!(stage.graph().wip_selected);

    // The file in conflict is reached with the keys of the changes panel:
    // the menu key opens its menu at the row, and Enter opens the file to
    // be resolved, in the merge tool.
    stage.focus_changes();
    stage.keys("home shift-f10");
    shots.take(&mut stage, "menu_conflict", Themes::Two);
    stage.keys("escape");
    stage.pass(Duration::from_secs(1));
    stage.keys("enter");
    let workspace = stage.workspace();
    assert_eq!(
        workspace.file,
        Some(("docs/guide.md".to_owned(), None)),
        "{workspace:?}"
    );
    assert_eq!(workspace.file_tab, "merge");
    // The tool has the window's width and its own way out; the steps
    // from conflict to conflict stand over the result.
    assert!(stage.shows("merge-close") && stage.shows("merge-save"));
    // Nothing chosen yet: no choices to take back.
    assert!(stage.shows("merge-next") && !stage.shows("merge-rebuild"));
    assert!(!stage.shows("tab-merge"), "the file pane made room");
    // The result's colours may have been put off, as the editor's.
    stage.pass(crate::stage::PUT_OFF);
    shots.take(&mut stage, "merge_tool", Themes::Three);
    // Leaving it by hand goes to the file in the editor, where the way
    // back is.
    stage.click("merge-by-hand");
    assert_eq!(stage.workspace().file_tab, "edit");
    assert!(stage.shows("tab-merge"));
}

/// The interactive rebase being planned.
fn rebase(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project_with_files();
    let first = repos::rev(&repo, "HEAD~3");
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert!(stage.workspace().loaded);
    stage.act(RunRewrite {
        rewrite: Rewrite::Interactive {
            first,
            reword: false,
        },
    });
    assert!(stage.workspace().planning, "the plan of the rebase");
    // Every commit has its choice; the three buttons are under the list.
    assert!(stage.shows(("plan-choice", 0usize)) && stage.shows(("plan-choice", 2usize)));
    assert!(
        stage.shows("plan-reset") && stage.shows("plan-cancel-foot") && stage.shows("plan-start")
    );
    shots.take(&mut stage, "rebase_editor", Themes::Three);
}

/// The switch that leaves out the hooks that could refuse it, where a
/// merge, a rebase, a push and a mailbox are started. It is there because
/// the repository has such hooks.
fn hooks(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project_with_refusing_hooks();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert!(stage.workspace().loaded);

    // How to merge: the switch is the last line, and off.
    stage.act(RunRewrite {
        rewrite: Rewrite::MergeHow {
            target: "wip".to_owned(),
        },
    });
    assert!(stage.dialog_open() && stage.shows(("form-check", 0usize)));
    shots.take(&mut stage, "dialog_merge_hooks", Themes::Two);
    stage.keys("escape");
    assert!(!stage.dialog_open());
    stage.pass(Duration::from_secs(1));

    // What to rebase: under the switch for local changes.
    stage.act(RunRewrite {
        rewrite: Rewrite::RebaseHow {
            onto: Some("wip".to_owned()),
            after: None,
        },
    });
    assert!(stage.dialog_open() && stage.shows(("form-check", 1usize)));
    stage.click(("form-check", 1usize));
    shots.take(&mut stage, "dialog_rebase_hooks", Themes::Two);
    stage.keys("escape");
    assert!(!stage.dialog_open());
    stage.pass(Duration::from_secs(1));

    // A mailbox: ticked, and the button says so.
    stage.act(keymap::ApplyMailbox);
    assert!(stage.dialog_open() && stage.shows("patch-skip-hooks"));
    stage.click("patch-skip-hooks");
    shots.take(&mut stage, "dialog_mailbox_hooks", Themes::Two);
    stage.keys("escape");
    assert!(!stage.dialog_open());
    stage.pass(Duration::from_secs(1));

    // Pushing: switched off in the menu of the button (on the pointer's
    // other button), which says so for as long, as the menu's first entry
    // does.
    stage.act(keymap::SkipPushHook);
    assert_eq!(stage.notices().len(), 1, "{:?}", stage.notices());
    // A warning stays until it is put away.
    stage.close_toast();
    stage.pass(Duration::from_secs(1));
    assert!(!stage.shows("push-more"), "nothing stands beside Push");
    stage.right_click("push");
    shots.take(&mut stage, "menu_push_hook", Themes::Two);
    stage.keys("escape");

    // The plan of an interactive rebase: beside its buttons.
    let first = repos::rev(&repo, "HEAD~3");
    stage.act(RunRewrite {
        rewrite: Rewrite::Interactive {
            first,
            reword: false,
        },
    });
    stage.until("the plan is there", |stage| stage.shows("plan-start"));
    assert!(stage.shows("plan-skip-hooks"));
    stage.click("plan-skip-hooks");
    shots.take(&mut stage, "rebase_editor_hooks", Themes::Two);
}

/// The views that take the centre in place of the graph.
fn sheets(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert!(stage.workspace().loaded);

    // Something to undo: a branch made here.
    stage.act(keymap::NewBranch);
    stage.type_text("feature/wrap");
    stage.keys("enter");
    stage.until("the new branch is among the references", |stage| {
        stage.workspace().branches == 7
    });
    stage.pass(Duration::from_secs(30));

    stage.act(keymap::ShowUndoHistory);
    assert_eq!(stage.workspace().sheet, Some("undo_history"));
    shots.take(&mut stage, "undo_history", Themes::Two);

    // The profiles are a section of the preferences; the first one is
    // made from git's identity.
    stage.act(keymap::ShowProfiles);
    assert_eq!(stage.shell().preferences_section, Some("profiles"));
    stage.until("the profile made from git's identity is listed", |stage| {
        stage.shows(("profile-row", 0usize))
    });
    shots.take(&mut stage, "profiles", Themes::Two);
    stage.act(keymap::ShowPreferences);
    assert_eq!(stage.shell().sheet, None);

    stage.act(keymap::ShowIndexFlags);
    assert_eq!(stage.workspace().sheet, Some("index_flags"));
    shots.take(&mut stage, "index_flags", Themes::Two);

    stage.act(keymap::ShowRerereRecords);
    assert_eq!(stage.workspace().sheet, Some("rerere"));
    shots.take(&mut stage, "rerere", Themes::Two);

    // What git was run for the branch made above, and how long each took:
    // under the clock these pictures are taken by no time passes, so the
    // log reads the same on every run (`bitbraid_git::activity`).
    stage.act(keymap::ShowActivity);
    assert_eq!(stage.workspace().sheet, Some("activity"));
    shots.take(&mut stage, "activity", Themes::Two);
}

/// How the repository signs: it asks gpg and ssh for the keys there are,
/// and in the scratch surroundings there are none.
fn signing(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    stage.act(keymap::ShowSigning);
    assert_eq!(stage.shell().preferences_section, Some("signing"));
    shots.take(&mut stage, "signing", Themes::Two);
}

/// Large files.
fn lfs(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    stage.act(keymap::ShowLfs);
    assert_eq!(stage.shell().preferences_section, Some("lfs"));
    shots.take(&mut stage, "lfs", Themes::Two);
}

/// A bare repository, which is opened read-only.
fn read_only(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::archive();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    let workspace = stage.workspace();
    assert!(workspace.loaded && workspace.read_only, "{workspace:?}");
    shots.take(&mut stage, "read_only", Themes::Three);

    // What would change it is not on offer, and says so when asked for.
    stage.act(keymap::NewBranch);
    assert!(!stage.workspace().prompting);
    assert_eq!(stage.notices().len(), 1, "{:?}", stage.notices());
    shots.take(&mut stage, "read_only_refused", Themes::Two);
}

/// The menus that open where the pointer is: of a commit's row, of a
/// label, of a tab, and of what a drop can do. None of these has an id
/// to be found by, so the pointer goes to where the graph says its rows
/// are, and to where the first tab is.
fn menus(shots: &mut Shots) {
    /// The middle of the first tab, in points from the window's corner.
    const FIRST_TAB: (f32, f32) = (68.0, 16.0);
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert!(stage.workspace().loaded);
    let put_away = |stage: &mut Stage| {
        stage.keys("escape");
        stage.pass(Duration::from_secs(1));
    };

    // A commit: the fourth row from the top.
    let row = stage.graph_spot(3, false);
    stage.right_click_at(row);
    shots.take(&mut stage, "menu_commit", Themes::Two);
    put_away(&mut stage);

    // A label, of a branch that is not the one checked out: what goes
    // for the branch, and what the graph shows of it.
    let label = stage.graph_spot(1, true);
    stage.right_click_at(label);
    shots.take(&mut stage, "menu_label", Themes::Two);
    put_away(&mut stage);

    // A tab.
    stage.right_click_at(FIRST_TAB);
    shots.take(&mut stage, "menu_tab", Themes::Two);
    put_away(&mut stage);

    // A branch picked up by its label and let go on another commit:
    // what can be done there is asked.
    let (from, to) = (stage.graph_spot(1, true), stage.graph_spot(5, false));
    stage.drag_at(from, to);
    assert!(stage.graph().drop_menu, "what the drop can do is on offer");
    shots.take(&mut stage, "menu_drop", Themes::Two);
}

/// A question of git's or ssh's: a password that is not shown as it is
/// typed, and behind it a question with yes and no. Nobody asks them
/// here: they are handed to the window as the askpass server would.
fn asking(shots: &mut Shots) {
    use bitbraid_git::askpass::{Event, Kind, Question};
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert!(stage.workspace().loaded);
    let (password, typed) = Question::standalone(
        1,
        "Password for 'https://ada@git.example.invalid': ",
        Kind::Secret,
    );
    let (host, said) = Question::standalone(
        2,
        "The authenticity of host 'git.example.invalid (203.0.113.7)' can't be established. \
         ED25519 key fingerprint is SHA256:kBRHA1v6pDMQeYzXnAAkh0zcA0h0fuGnPQ1vH3pCZqI. Are you \
         sure you want to continue connecting (yes/no/[fingerprint])?",
        Kind::Confirm,
    );
    stage.with_shell(|shell, window, cx| {
        shell.on_ask_event(Event::Asked(password), window, cx);
        shell.on_ask_event(Event::Asked(host), window, cx);
    });
    stage.frame();
    let asked = stage.shell().asking.expect("a question is on screen");
    assert!(asked.0.starts_with("Password for"), "{asked:?}");
    assert_eq!(asked.1, 1, "one more waits behind it");
    stage.type_text("not shown");
    shots.take(&mut stage, "dialog_password", Themes::Two);
    // Answered with Enter: what was typed goes to whoever asked, and the
    // next question comes up.
    stage.keys("enter");
    assert_eq!(
        typed.try_recv().ok().flatten().as_deref(),
        Some("not shown")
    );
    stage.until("the question about the host", |stage| {
        stage
            .shell()
            .asking
            .is_some_and(|(prompt, behind)| prompt.starts_with("The authenticity") && behind == 0)
    });
    // The dialog that came up takes the keys a moment after it opened.
    stage.pass(Duration::from_secs(1));
    shots.take(&mut stage, "dialog_host_key", Themes::Two);
    // Its other button declines it.
    stage.click("ask-cancel");
    stage.pass(Duration::from_secs(1));
    assert_eq!(said.try_recv().ok(), Some(None), "declined");
    assert_eq!(stage.shell().asking, None);
}

/// What git makes of the signatures on commits: the marks in the graph
/// and the sentence in the details, for a good one, one by a key nobody
/// vouches for, and one that does not match; and the signing view with
/// who may sign. The commits and the key are the fixed ones of
/// `signed.rs`; no key is made and no agent is asked.
fn signatures(shots: &mut Shots) {
    use crate::signed;
    repos::fresh();
    let repo = repos::project();
    signed::add_to(&repo);
    let (good, untrusted, bad) = (
        signed::COMMITS[0].1,
        signed::COMMITS[1].1,
        signed::COMMITS[2].1,
    );
    // The newest of the three is the newest commit there is.
    let state = format!("[repos.{repo:?}]\nselected = \"{bad}\"\n");
    let mut stage = Stage::open(Start::with(&repo).state(state).pictures());
    assert_eq!(
        stage.graph().commits,
        19,
        "the fixture's sixteen and the three"
    );
    let seen = |stage: &mut Stage, id: &str, state: &str, signer: Option<&str>| {
        stage.until(
            &format!("the verdict on the commit that is {state}"),
            |stage| {
                stage.graph().selected.as_deref() == Some(id)
                    && stage.workspace().signature
                        == Some((state.to_owned(), signer.map(str::to_owned)))
            },
        );
    };
    // Git names nobody for a signature that does not match.
    seen(&mut stage, bad, "bad", None);
    assert!(
        !stage.shows("allow-signer"),
        "no key mends what does not match"
    );
    stage.keys("down down");
    seen(&mut stage, good, "good", Some(signed::SIGNER));
    assert!(!stage.shows("allow-signer"));
    // The picture is of the one in the middle: the three marks are in
    // the graph whichever is selected, and this one's details also offer
    // to allow its key.
    stage.keys("up");
    seen(&mut stage, untrusted, "untrusted", None);
    assert!(stage.shows("allow-signer"), "its key can be allowed");
    shots.take(&mut stage, "signatures", Themes::Two);

    // Who may sign, in the signing view.
    stage.act(keymap::ShowSigning);
    stage.until("the signer is listed", |stage| {
        stage.workspace().allowed_signers == [signed::SIGNER]
    });
    shots.take(&mut stage, "signing_signers", Themes::Two);
}

/// The page of repositories behind its pinned tab: one open with a
/// remote it is ahead of and something uncommitted, a favourite that
/// follows nothing, and one whose folder is gone.
fn repositories(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project_with_everything();
    let library = repos::work().join("library");
    std::fs::write(repo.join("a-new-note.txt"), "new\n").unwrap();
    let gone = repos::work().join("gone");
    let state = format!("recent = [{repo:?}, {library:?}, {gone:?}]\nfavourites = [{library:?}]\n");
    let mut stage = Stage::open(Start::with(&repo).state(state).pictures());
    stage.click("repositories-tab");
    assert_eq!(stage.shell().sheet, Some("repositories"));
    stage.until("git has been asked about each repository", |stage| {
        stage.shows("changes-open-project")
            && stage.shows("branch-favourites-library")
            && stage.shows("problem-recent-gone")
    });
    shots.take(&mut stage, "repositories", Themes::Two);
}

/// The pane beside the references panel, over the graph: what adds a
/// remote, a worktree and a submodule.
fn docked(shots: &mut Shots) {
    use bitbraid_ui::views::{
        refs::RemoteRequest,
        submodules::{RunSubmodule, SubmoduleRequest},
        worktrees::{RunWorktree, WorktreeRequest},
    };
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    assert!(stage.workspace().loaded);

    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.remote(RemoteRequest::Add, window, cx)
        })
    });
    stage.frame();
    assert_eq!(stage.workspace().docked, Some("remote"));
    shots.take(&mut stage, "docked_remote", Themes::Two);
    stage.keys("escape");

    stage.act(keymap::FocusReferences);
    stage.act(RunWorktree {
        request: WorktreeRequest::New(None),
    });
    assert_eq!(stage.workspace().docked, Some("worktree"));
    shots.take(&mut stage, "docked_worktree", Themes::Two);
    stage.keys("escape");

    stage.act(keymap::FocusReferences);
    stage.act(RunSubmodule {
        request: SubmoduleRequest::Add,
    });
    assert_eq!(stage.workspace().docked, Some("submodule"));
    // What is typed is looked at as it is typed.
    stage.type_text("https://example.invalid/team/library.git");
    assert!(stage.shows("docked-said"));
    shots.take(&mut stage, "docked_submodule", Themes::Two);
}
