//! What the window does, as `behaviour.rs` asks it, for what has to fit in
//! a small window: a menu taller than the window, the form under the
//! changes, the toolbar at a large zoom.

use bitbraid_ui::shell::STATUS_BAR_HEIGHT;
use gpui_kit::{
    ElementId, InputEvent as _, MouseMoveEvent, Pixels, Point, ScrollDelta, ScrollWheelEvent,
    point, px, test::TestWindowExt as _,
};

use crate::{
    repos,
    runner::{Shots, Test, Themes},
    stage::{HEIGHT, Stage, Start, WIDTH},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "a_branch_menu_taller_than_the_window_stays_inside_it_and_scrolls",
        run: a_branch_menu_taller_than_the_window_stays_inside_it_and_scrolls,
    },
    Test {
        name: "the_toolbar_keeps_its_parts_apart_at_a_large_zoom",
        run: the_toolbar_keeps_its_parts_apart_at_a_large_zoom,
    },
    Test {
        name: "a_narrow_window_puts_the_toolbars_middle_group_in_a_menu",
        run: a_narrow_window_puts_the_toolbars_middle_group_in_a_menu,
    },
    Test {
        name: "a_narrow_window_folds_the_references_panel_to_keep_the_graph",
        run: a_narrow_window_folds_the_references_panel_to_keep_the_graph,
    },
    Test {
        name: "at_150_percent_the_controls_over_a_diff_stay_apart_and_inside_the_pane",
        run: at_150_percent_the_controls_over_a_diff_stay_apart_and_inside_the_pane,
    },
    Test {
        name: "tabs_that_do_not_fit_are_not_drawn_over_the_bars_buttons",
        run: tabs_that_do_not_fit_are_not_drawn_over_the_bars_buttons,
    },
    Test {
        name: "the_buttons_of_a_stopped_rebase_stay_above_the_status_bar",
        run: the_buttons_of_a_stopped_rebase_stay_above_the_status_bar,
    },
    Test {
        name: "at_150_percent_the_commit_form_scrolls_into_reach",
        run: at_150_percent_the_commit_form_scrolls_into_reach,
    },
    Test {
        name: "at_150_percent_the_wheel_over_a_file_list_scrolls_the_list_before_the_panel",
        run: at_150_percent_the_wheel_over_a_file_list_scrolls_the_list_before_the_panel,
    },
    Test {
        name: "at_150_percent_a_commits_files_stay_in_reach_under_its_message",
        run: at_150_percent_a_commits_files_stay_in_reach_under_its_message,
    },
    Test {
        name: "in_a_low_window_the_details_scroll_and_their_files_take_the_wheel_first",
        run: in_a_low_window_the_details_scroll_and_their_files_take_the_wheel_first,
    },
    Test {
        name: "the_co_authors_label_keeps_to_one_line_before_their_pictures",
        run: the_co_authors_label_keeps_to_one_line_before_their_pictures,
    },
];

/// The room a window that draws its own frame keeps for its shadow, on
/// Linux: the toolkit keeps a menu that far from every edge, tiled or not.
const LINUX_INSET: f32 = 20.0;

/// Where a branch's name is drawn in the references panel, by its full
/// name.
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

/// The top and the bottom of an entry of the open menu, by its number,
/// when there is such an entry.
fn entry(stage: &mut Stage, at: usize) -> Option<(f32, f32)> {
    stage.window(|window, _| {
        window.within("popup-menu").try_find(at).map(|found| {
            let bounds = found.bounds();
            (f32::from(bounds.top()), f32::from(bounds.bottom()))
        })
    })
}

/// The menu of a branch in the references panel has more entries than a
/// window 720 high has room for, at 100 % on Linux, where the toolkit keeps
/// it the window's client inset from each edge as well: it is as high as
/// what is left, inside the window, and scrolls to its last entry.
fn a_branch_menu_taller_than_the_window_stays_inside_it_and_scrolls() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    stage.window(|window, _| window.set_client_inset(px(LINUX_INSET)));
    let at = name_at(&mut stage, "refs/heads/feature/dates");
    stage.right_click_at((f32::from(at.x), f32::from(at.y)));
    assert!(stage.shows("popup-menu"), "the branch's menu is open");
    let (_, top, _, height) = stage.place("popup-menu");
    let bottom = top + height;
    assert!(
        top >= 0.0 && bottom <= HEIGHT,
        "the menu is inside the window: {top} to {bottom}"
    );

    // Its last entry is reached by scrolling.
    let last = (0..200)
        .rev()
        .find(|at| entry(&mut stage, *at).is_some())
        .expect("the menu has entries");
    assert!(
        entry(&mut stage, last).is_some_and(|(_, foot)| foot > bottom),
        "the last entry is below what shows at first"
    );
    for _ in 0..10 {
        stage.window(|window, cx| {
            window.scroll(
                "popup-menu",
                ScrollDelta::Pixels(point(px(0.), px(-200.))),
                cx,
            )
        });
        stage.frame();
    }
    let (head, foot) = entry(&mut stage, last).expect("the last entry");
    assert!(
        head >= top && foot <= bottom,
        "scrolled, the last entry shows in the menu: {head}..{foot} in {top}..{bottom}"
    );
}

/// Where the status bar begins at a zoom, in the toolkit's pixels.
fn status_top(zoom: f32) -> f32 {
    HEIGHT - STATUS_BAR_HEIGHT * zoom
}

/// The bottom of a control of the window, in the toolkit's pixels.
fn bottom_of(stage: &mut Stage, id: &'static str) -> f32 {
    assert!(stage.shows(id), "{id} is on show");
    let (_, top, _, height) = stage.place(id);
    top + height
}

/// A rebase that stopped in conflict puts its three buttons under the
/// form, all of them above the status bar in a window 720 high.
fn the_buttons_of_a_stopped_rebase_stay_above_the_status_bar() {
    repos::fresh();
    let repo = repos::project_in_rebase_conflict();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the rebase is seen", |stage| stage.shows("op-skip"));
    for id in ["op-continue", "op-abort", "op-skip"] {
        let bottom = bottom_of(&mut stage, id);
        assert!(
            bottom <= status_top(1.0),
            "{id} ends at {bottom}, under the status bar at {}",
            status_top(1.0)
        );
    }
}

/// At 100 %, 150 % and 200 % the blocks at the toolbar's left, its
/// buttons and its right end keep apart, in this order, inside the window:
/// Undo over the branch's name and Redo over its caret would take the
/// caret's clicks.
fn the_toolbar_keeps_its_parts_apart_at_a_large_zoom() {
    for zoom in [1.0f32, 1.5, 2.0] {
        repos::fresh();
        let repo = repos::project();
        let mut stage = Stage::open(Start::with(&repo).preferences(&format!("zoom = {zoom}\n")));
        stage.until("the repository is open", |stage| stage.workspace().loaded);
        let mut edge = 0.0f32;
        for id in [
            "crumb-repo",
            "crumb-branch",
            "undo",
            "redo",
            "pull",
            "push",
            "new-branch",
            "stash",
            "pop",
            "terminal",
            "palette",
            "search",
        ] {
            assert!(stage.shows(id), "{id} is on show at {zoom}");
            let (left, _, width, _) = stage.place(id);
            assert!(
                left >= edge - 0.5,
                "at {zoom}, {id} begins at {left}, over what ends at {edge}"
            );
            edge = left + width;
        }
        assert!(edge <= WIDTH + 0.5, "at {zoom} the toolbar ends at {edge}");
    }
}

/// The left, top, right and bottom edge of a control.
type Edges = (f32, f32, f32, f32);

/// The edges of a control, when it is on show.
fn edges(stage: &mut Stage, id: &'static str) -> Option<Edges> {
    stage.window(|window, _| {
        window
            .try_find(id)
            .filter(|found| found.visible())
            .map(|found| {
                let bounds = found.bounds();
                (
                    f32::from(bounds.left()),
                    f32::from(bounds.top()),
                    f32::from(bounds.right()),
                    f32::from(bounds.bottom()),
                )
            })
    })
}

/// At 150 % every control of the row over a file's diff is inside the
/// pane, and none is over another.
fn at_150_percent_the_controls_over_a_diff_stay_apart_and_inside_the_pane() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 1.5\n"));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    stage.open_changed("src/render.rs");
    stage.until("the diff is on show", |stage| stage.shows("view-diff"));
    let (row_left, _, row_right, _) = edges(&mut stage, "file-controls").expect("the row");
    let ids = [
        "tab-edit",
        "side-unstaged",
        "side-staged",
        "view-file",
        "view-diff",
        "tab-blame",
        "tab-history",
        "diff-wrap",
        "diff-prev",
        "diff-next",
        "diff-whitespace",
        "diff-words",
        "diff-external",
        "diff-refresh",
    ];
    let found: Vec<(&str, Edges)> = ids
        .iter()
        .filter_map(|id| edges(&mut stage, id).map(|at| (*id, at)))
        .collect();
    assert!(found.len() >= 8, "the row has its controls: {found:?}");
    for (id, (left, _, right, _)) in &found {
        assert!(
            *left >= row_left - 0.5 && *right <= row_right + 0.5,
            "{id} at {left}..{right} is outside the row {row_left}..{row_right}"
        );
    }
    for (at, (one, a)) in found.iter().enumerate() {
        for (other, b) in found.iter().skip(at + 1) {
            let apart =
                a.2 <= b.0 + 0.5 || b.2 <= a.0 + 0.5 || a.3 <= b.1 + 0.5 || b.3 <= a.1 + 0.5;
            assert!(apart, "{one} {a:?} is over {other} {b:?}");
        }
    }
}

/// At 150 % with a dozen tabs, as many tabs as fit are drawn, the shown
/// one among them, and none past the tabs' end over the plus, the bell and
/// the gear; the list of tabs under the chevron reaches the others.
fn tabs_that_do_not_fit_are_not_drawn_over_the_bars_buttons() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 1.5\n"));
    for _ in 0..12 {
        stage.act(bitbraid_ui::keymap::NewTab);
    }
    let active = stage.shell().active;
    let (_, _, end, _) = edges(&mut stage, "tabs").expect("the tabs");
    let (plus, ..) = edges(&mut stage, "new-tab").expect("the plus");
    assert!(
        end <= plus + 0.5,
        "the tabs end at {end}, the plus begins at {plus}"
    );
    let mut drawn = 0;
    for ix in 0..13usize {
        let found = stage.window(|window, _| {
            window
                .try_find(("tab", ix))
                .filter(|found| found.visible())
                .map(|found| f32::from(found.bounds().right()))
        });
        if let Some(right) = found {
            drawn += 1;
            assert!(
                right <= end + 0.5,
                "tab {ix} ends at {right}, past the tabs' end at {end}"
            );
        }
    }
    assert!(drawn < 13, "not every tab has room at 150 %: {drawn}");
    let shown = stage.window(|window, _| {
        window
            .try_find(("tab", active))
            .is_some_and(|found| found.visible())
    });
    assert!(shown, "the shown tab {active} is drawn");
}

/// At 150 % a window 1280 by 720 is about 853 by 480 points: the panel of
/// the changes has too little room for the lists at their least and the
/// whole form. It ends over the status bar and scrolls, down to the
/// button.
fn at_150_percent_the_commit_form_scrolls_into_reach() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 1.5\n"));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let (_, pane_top, _, pane_height) = stage.place("wip-pane");
    assert!(
        pane_top + pane_height <= status_top(1.5) + 0.5,
        "the panel ends over the status bar: {pane_top} + {pane_height}"
    );
    for _ in 0..10 {
        stage.window(|window, cx| {
            window.scroll(
                "wip-pane",
                ScrollDelta::Pixels(point(px(0.), px(-200.))),
                cx,
            )
        });
        stage.frame();
    }
    let bottom = bottom_of(&mut stage, "commit");
    assert!(
        bottom <= status_top(1.5),
        "scrolled, the commit button ends at {bottom}, under the status bar at {}",
        status_top(1.5)
    );
}

/// Turn the wheel by `dy` pixels with the pointer at a place in the
/// window, in its pixels: down the content for less than nought.
pub fn wheel_at(stage: &mut Stage, (x, y): (f32, f32), dy: f32) {
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

/// The top and the bottom of a row of the changes, by its number, when it
/// is on show.
fn change_row(stage: &mut Stage, ix: usize) -> Option<(f32, f32)> {
    stage.window(|window, _| {
        window
            .try_find(("changes-row", ix))
            .filter(|found| found.visible())
            .map(|found| {
                let bounds = found.bounds();
                (f32::from(bounds.top()), f32::from(bounds.bottom()))
            })
    })
}

/// The number of the row of the changes under a height of the window.
fn change_row_at(stage: &mut Stage, y: f32) -> Option<usize> {
    (0..200).find(|ix| change_row(stage, *ix).is_some_and(|(top, bottom)| top <= y && y < bottom))
}

/// Where the panel of the changes and the form is scrolled to, told by
/// where the changes' top is drawn.
fn panel_at(stage: &mut Stage) -> f32 {
    stage.place("changes").1
}

/// At 150 % in a window 1280 by 720 the panel of the changes and the form
/// scrolls as a whole, and the toolkit hands a wheel to every scrolling
/// area under the pointer. Over one of its lists the list takes the wheel
/// first; the panel only once the list is at its end, or over the panel
/// outside its lists.
fn at_150_percent_the_wheel_over_a_file_list_scrolls_the_list_before_the_panel() {
    repos::fresh();
    let repo = repos::project_with_changes();
    for n in 0..40 {
        std::fs::write(repo.join(format!("note-{n:02}.txt")), "a note\n").unwrap();
    }
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 1.5\n"));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    stage.until("every changed file is listed", |stage| {
        stage.workspace().status_paths >= 46
    });
    let (left, top, width, height) = stage.place("unstaged-rows");
    let (_, pane_top, _, pane_height) = stage.place("wip-pane");
    let centre = (left + width / 2.0, top + height / 2.0);
    assert!(
        centre.1 > pane_top && centre.1 < pane_top + pane_height,
        "the middle of the list {centre:?} is in the panel at {pane_top} + {pane_height}"
    );
    let ix = change_row_at(&mut stage, centre.1).expect("a row under the list's middle");

    // Over the list: the list moves, the panel does not.
    let (panel, (row, _)) = (panel_at(&mut stage), change_row(&mut stage, ix).unwrap());
    wheel_at(&mut stage, centre, -10.0);
    let moved = panel - panel_at(&mut stage);
    assert!(
        moved.abs() < 0.5,
        "a wheel over the list scrolled the panel by {moved}"
    );
    let scrolled = row - change_row(&mut stage, ix).expect("the row is on show").0;
    assert!(
        (scrolled - 10.0).abs() < 0.5,
        "a wheel of 10 over the list moved its row by {scrolled}"
    );

    // Over the list's heading, outside the list: the panel moves, and the
    // list with it, but not inside it.
    let (panel, (row, _)) = (panel_at(&mut stage), change_row(&mut stage, ix).unwrap());
    wheel_at(&mut stage, (centre.0, top - 20.0), -10.0);
    let moved = panel - panel_at(&mut stage);
    assert!(
        (moved - 10.0).abs() < 0.5,
        "a wheel of 10 over the heading scrolled the panel by {moved}"
    );
    let scrolled = row - change_row(&mut stage, ix).expect("the row is on show").0;
    assert!(
        (scrolled - moved).abs() < 0.5,
        "the panel moved by {moved}, the row by {scrolled}"
    );

    // Over the list again, on and on: the list goes to its end, and only
    // then the panel moves.
    let (left, top, width, height) = stage.place("unstaged-rows");
    let centre = (left + width / 2.0, top + height / 2.0);
    let panel = panel_at(&mut stage);
    let turns = (0..60)
        .find(|_| {
            wheel_at(&mut stage, centre, -100.0);
            (panel - panel_at(&mut stage)).abs() > 0.5
        })
        .expect("at the list's end the wheel scrolls the panel");
    assert!(turns > 0, "the first turn over the list scrolled the panel");
    // The list was at its end: its last row ends where the list ends.
    let (_, top, _, height) = stage.place("unstaged-rows");
    let bottom = top + height;
    let last = (0..200)
        .filter(|ix| {
            change_row(&mut stage, *ix)
                .is_some_and(|(head, foot)| head >= top - 0.5 && foot <= bottom + 0.5)
        })
        .max()
        .expect("rows of the list on show");
    let (_, foot) = change_row(&mut stage, last).unwrap();
    assert!(
        (foot - bottom).abs() < 1.0,
        "the list's last row on show ends at {foot}, the list at {bottom}"
    );
    assert!(
        change_row(&mut stage, last + 1).is_none_or(|(head, _)| head >= bottom - 0.5),
        "row {last} is the list's last"
    );

    // And back up: the list first again.
    let (left, top, width, height) = stage.place("unstaged-rows");
    let centre = (left + width / 2.0, top + height / 2.0);
    let panel = panel_at(&mut stage);
    wheel_at(&mut stage, centre, 10.0);
    let moved = panel - panel_at(&mut stage);
    assert!(
        moved.abs() < 0.5,
        "a wheel up over the list scrolled the panel by {moved}"
    );
}

/// Height of a commit's file in the details, in points; and of their
/// message's frame, undragged, and at the least it gives way to.
const FILE_ROW: f32 = 32.0;
const MESSAGE_FRAME: f32 = 133.0;
const MESSAGE_SQUEEZED: f32 = 56.0;
/// Height of a card of several commits selected, and the gap after it.
const CARD_ROW: f32 = 44.0 + 4.0;

/// Select the commit on the second row of the graph (the first is the
/// row of uncommitted work) and wait for its files.
fn select_the_first_commit(stage: &mut Stage) {
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let commit = stage.graph_spot(1, false);
    stage.click_at(commit);
    stage.until("a commit is selected", |stage| !stage.graph().wip_selected);
    // Listed, on show or not.
    stage.until("its files are listed", |stage| {
        stage.window(|window, _| window.try_find("commit-files").is_some())
    });
}

/// At 150 % in a window 1280 by 720 a commit's files stay in reach: the
/// message gives way, down to a line of its summary, and the files keep
/// two rows above the status bar; with several commits selected, their
/// cards give way too.
fn at_150_percent_a_commits_files_stay_in_reach_under_its_message() {
    repos::fresh();
    let repo = repos::project();
    // Commits like these, the newest of the history: a file each, by
    // whoever committed them, when they were committed; and a change in
    // the working tree, which puts the strip over the details.
    for (tick, n) in (1000..).zip(["one", "two", "three", "four", "five", "six"]) {
        std::fs::write(repo.join(format!("note-{n}.txt")), "a note\n").unwrap();
        repos::git(&repo, &["add", "-A"]);
        repos::git_at(&repo, tick, &["commit", "-q", "-m", &format!("note {n}")]);
    }
    std::fs::write(repo.join("note-one.txt"), "a changed note\n").unwrap();
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 1.5\n"));
    select_the_first_commit(&mut stage);
    stage.until("the newest commit is selected", |stage| {
        stage.graph().selected_summary.as_deref() == Some("note six")
    });
    assert!(
        stage.shows("view-changes"),
        "the strip over the details says the working tree has changes"
    );
    // The first file's row is above the status bar, and the files have
    // their two rows' room.
    let in_reach = |stage: &mut Stage, what: &str| {
        let (_, _, _, height) = stage.place("commit-files");
        assert!(
            height >= 2.0 * FILE_ROW * 1.5 - 0.5,
            "{what}: the files have {height}, less than two rows"
        );
        let (_, top, _, row) = stage.place(("commit-file", 0usize));
        assert!(
            top + row <= status_top(1.5) + 0.5,
            "{what}: the first file ends at {}, under the status bar at {}",
            top + row,
            status_top(1.5)
        );
    };
    in_reach(&mut stage, "a commit");
    let (_, _, _, message) = stage.place("commit-message");
    assert!(
        (MESSAGE_SQUEEZED * 1.5 - 0.5..MESSAGE_FRAME * 1.5 - 0.5).contains(&message),
        "the message gave way to {message}"
    );

    // Six commits: a card for each, over the files they change together;
    // the cards give way, down to one, and scroll.
    for _ in 0..5 {
        stage.keys("shift-down");
    }
    assert_eq!(stage.graph().picked, 6, "six commits are selected");
    stage.until("their cards and files are listed", |stage| {
        stage.shows("commit-cards")
            && stage.window(|window, _| window.try_find("commit-files").is_some())
    });
    in_reach(&mut stage, "six commits");
    // Only as far as the files need: they have their two rows and no
    // more.
    let (_, _, _, cards) = stage.place("commit-cards");
    assert!(
        (CARD_ROW * 1.5..6.0 * CARD_ROW * 1.5).contains(&cards),
        "the cards gave way to {cards}"
    );
    let (_, _, _, files) = stage.place("commit-files");
    assert!(
        (files - 2.0 * FILE_ROW * 1.5).abs() < 1.0,
        "the files have {files} under cards that gave way to {cards}"
    );
}

/// In a window lower still, the details of a commit do not fit even with
/// the message given way: the panel scrolls to the files, and a wheel over
/// the files moves them before the panel, as over the changes.
fn in_a_low_window_the_details_scroll_and_their_files_take_the_wheel_first() {
    repos::fresh();
    let repo = repos::project();
    std::fs::create_dir_all(repo.join("notes")).unwrap();
    for n in 0..30 {
        std::fs::write(repo.join(format!("notes/note-{n:02}.txt")), "a note\n").unwrap();
    }
    repos::git(&repo, &["add", "-A"]);
    repos::git_at(&repo, 1000, &["commit", "-q", "-m", "thirty notes"]);
    // A change in the working tree: the strip over the details.
    std::fs::write(repo.join("notes/note-00.txt"), "a changed note\n").unwrap();
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 1.5\n"));
    select_the_first_commit(&mut stage);
    stage.until("the newest commit is selected", |stage| {
        stage.graph().selected_summary.as_deref() == Some("thirty notes")
    });
    let height = 600.0;
    stage.resize(WIDTH, height);
    let status = height - STATUS_BAR_HEIGHT * 1.5;
    // Where the panel is scrolled to, told by where the message is drawn.
    let panel_at = |stage: &mut Stage| stage.place("commit-message").1;
    let (_, top, _, files) = stage.place("commit-files");
    assert!(
        top + files > status + 0.5,
        "the panel has less room than it needs: the files end at {}, the status bar is at {status}",
        top + files
    );

    // Over the message, which has nothing to scroll: the panel moves, to
    // its end, where the files' rows are above the status bar.
    let (left, top, width, message) = stage.place("commit-message");
    let over_message = (left + width / 2.0, top + message / 2.0);
    let panel = panel_at(&mut stage);
    wheel_at(&mut stage, over_message, -400.0);
    let moved = panel - panel_at(&mut stage);
    assert!(
        moved > 0.5,
        "a wheel over the message scrolled the panel by {moved}"
    );
    let (left, top, width, files) = stage.place("commit-files");
    assert!(
        top + files <= status + 0.5,
        "scrolled, the files end at {}, under the status bar at {status}",
        top + files
    );

    // Over the files: they move, the panel does not.
    let centre = (left + width / 2.0, top + files / 2.0);
    let row = |stage: &mut Stage| stage.place(("commit-file", 0usize)).1;
    let (panel, first) = (panel_at(&mut stage), row(&mut stage));
    wheel_at(&mut stage, centre, -10.0);
    let moved = panel - panel_at(&mut stage);
    assert!(
        moved.abs() < 0.5,
        "a wheel over the files scrolled the panel by {moved}"
    );
    let scrolled = first - row(&mut stage);
    assert!(
        (scrolled - 10.0).abs() < 0.5,
        "a wheel of 10 over the files moved their first row by {scrolled}"
    );

    // Back up over the files: they move first, and at their top the panel.
    wheel_at(&mut stage, centre, 10.0);
    let moved = panel - panel_at(&mut stage);
    assert!(
        moved.abs() < 0.5,
        "a wheel up over the files scrolled the panel by {moved}"
    );
    assert!(
        (row(&mut stage) - first).abs() < 0.5,
        "the files are at their top"
    );
    wheel_at(&mut stage, centre, 10.0);
    let moved = panel_at(&mut stage) - panel;
    assert!(
        (moved - 10.0).abs() < 0.5,
        "at the files' top a wheel of 10 scrolled the panel by {moved}"
    );
}

/// A repository whose newest commit names two co-authors, opened with that
/// commit selected.
fn co_authored(start: impl FnOnce(Start) -> Start) -> Stage {
    repos::fresh();
    let repo = repos::project();
    repos::git_at(
        &repo,
        1000,
        &[
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "Read escapes in quotes",
            "-m",
            "Co-authored-by: Ben Okafor <ben@example.invalid>\n\
             Co-authored-by: Cleo Marsh <cleo@example.invalid>",
        ],
    );
    let head = repos::rev(&repo, "HEAD");
    let state = format!("[repos.{repo:?}]\nselected = \"{head}\"\n");
    let mut stage = Stage::open(start(Start::with(&repo).state(state)));
    stage.until("the co-authors are shown", |stage| {
        stage.shows(("co-author", 1usize))
    });
    stage
}

/// "Co-authors:" keeps to one line, before the first picture, in a box as
/// wide as its words.
fn the_co_authors_label_keeps_to_one_line_before_their_pictures() {
    let mut stage = co_authored(|start| start);
    let (left, _, width, height) = stage.place("co-authors-label");
    let (picture_left, _, _, picture) = stage.place(("co-author", 0usize));
    assert!(
        height < picture * 1.5,
        "the label is {height} high beside a picture {picture} high: more than a line"
    );
    assert!(
        left + width <= picture_left + 0.5,
        "the label ends at {}, over the first picture at {picture_left}",
        left + width
    );
}

/// The row of a commit's co-authors, under its author.
pub fn co_author_pictures(shots: &mut Shots) {
    let mut stage = co_authored(Start::pictures);
    shots.take(&mut stage, "details_co_authors", Themes::Two);
}

/// The window at 200 % in 1280 by 720, which is 640 by 360 points: the
/// toolbar and the graph in a narrow window, the references panel a
/// strip; and at 250 % (512 points), the toolbar's menu of what it has
/// no room for.
pub fn narrow_pictures(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 2.0\n").pictures());
    stage.until("the repository is open", |stage| stage.workspace().loaded);
    stage.until("the references panel is a strip", |stage| {
        stage.shows("unfold-refs")
    });
    assert!(stage.shows("stash") && !stage.shows("toolbar-more"));
    shots.take(&mut stage, "narrow_window", Themes::Two);
    drop(stage);

    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 2.5\n").pictures());
    stage.until("the repository is open", |stage| stage.workspace().loaded);
    assert!(!stage.shows("stash"));
    stage.click("toolbar-more");
    stage.frame();
    assert!(
        menu_entries(&mut stage)
            .iter()
            .any(|entry| entry == "Stash"),
        "{:?}",
        menu_entries(&mut stage)
    );
    shots.take(&mut stage, "narrow_window_menu", Themes::Two);
    stage.keys("escape");
}

/// The entries of the menu that is open, by what they say, with where
/// each one's middle is.
fn menu_places(stage: &mut Stage) -> Vec<(String, (f32, f32))> {
    stage.window(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|element| element.visible() && element.role() == Some(gpui_kit::Role::MenuItem))
            .filter_map(|element| {
                let bounds = element.bounds();
                let middle = (
                    f32::from(bounds.left() + bounds.size.width / 2.0),
                    f32::from(bounds.top() + bounds.size.height / 2.0),
                );
                element.label().map(|label| (label.to_owned(), middle))
            })
            .collect()
    })
}

/// The entries of the menu that is open, by what they say.
fn menu_entries(stage: &mut Stage) -> Vec<String> {
    menu_places(stage)
        .into_iter()
        .map(|(label, _)| label)
        .collect()
}

/// The parts of the toolbar, left to right, that are on show: none
/// begins before the one before it ends, and the last ends inside the
/// window.
fn toolbar_apart(stage: &mut Stage, what: &str) -> Vec<&'static str> {
    let mut edge = 0.0f32;
    let mut shown = Vec::new();
    for id in [
        "crumb-repo",
        "crumb-branch",
        "undo",
        "redo",
        "pull",
        "push",
        "new-branch",
        "stash",
        "pop",
        "terminal",
        "toolbar-more",
        "palette",
        "search",
    ] {
        if !stage.shows(id) {
            continue;
        }
        let (left, _, width, _) = stage.place(id);
        assert!(
            left >= edge - 0.5,
            "{what}: {id} begins at {left}, over what ends at {edge}"
        );
        edge = left + width;
        shown.push(id);
    }
    assert!(edge <= WIDTH + 0.5, "{what}: the toolbar ends at {edge}");
    shown
}

/// In a window narrower than the whole toolbar at its least (569 points:
/// 1280 at 250 % is 512), Branch, Stash, Pop and the terminal go into the
/// menu of a button after Push, and narrower still (1280 at 300 % is 427)
/// the whole middle group is that button; the parts keep apart, and what
/// the menu has does what its button does.
fn a_narrow_window_puts_the_toolbars_middle_group_in_a_menu() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 2.5\n"));
    stage.until("the repository is open", |stage| stage.workspace().loaded);
    let shown = toolbar_apart(&mut stage, "at 250 %");
    assert_eq!(
        shown,
        [
            "crumb-repo",
            "crumb-branch",
            "undo",
            "redo",
            "pull",
            "push",
            "toolbar-more",
            "palette",
            "search"
        ]
    );
    stage.click("toolbar-more");
    stage.frame();
    let entries = menu_entries(&mut stage);
    for entry in ["Branch", "Stash", "Pop", "Terminal"] {
        assert!(entries.iter().any(|seen| seen == entry), "{entries:?}");
    }
    assert!(!entries.iter().any(|seen| seen == "Undo"), "{entries:?}");
    // Stash, from the menu: the changes are stashed, as by its button.
    let (_, stash) = menu_places(&mut stage)
        .into_iter()
        .find(|(label, _)| label == "Stash")
        .expect("Stash is in the menu");
    stage.click_at(stash);
    stage.until("the changes are stashed", |stage| {
        stage.settle();
        !repos::git(&repo, &["stash", "list"]).trim().is_empty()
    });
    drop(stage);

    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 3.0\n"));
    stage.until("the repository is open", |stage| stage.workspace().loaded);
    let shown = toolbar_apart(&mut stage, "at 300 %");
    assert_eq!(
        shown,
        [
            "crumb-repo",
            "crumb-branch",
            "toolbar-more",
            "palette",
            "search"
        ]
    );
    stage.click("toolbar-more");
    stage.frame();
    let entries = menu_entries(&mut stage);
    for entry in ["Undo", "Redo", "Pull", "Push", "Branch", "Pop", "Terminal"] {
        assert!(entries.iter().any(|seen| seen == entry), "{entries:?}");
    }
    stage.keys("escape");
}

/// At 200 % (640 points) the references panel is its strip while a
/// commit's details are beside the graph, which keeps at least 200; between
/// the two panels at their least it would keep 79. At 150 % and 160 % (853
/// and 800 points) the panel stays open.
fn a_narrow_window_folds_the_references_panel_to_keep_the_graph() {
    repos::fresh();
    let repo = repos::project();
    for zoom in ["1.5", "1.6"] {
        let mut stage = Stage::open(Start::with(&repo).preferences(&format!("zoom = {zoom}\n")));
        stage.until("a commit is selected", |stage| {
            stage.workspace().loaded && stage.graph().selected.is_some()
        });
        assert!(!stage.shows("unfold-refs"), "at {zoom} the panel is open");
        drop(stage);
    }

    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 2.0\n"));
    stage.until("a commit is selected", |stage| {
        stage.workspace().loaded && stage.graph().selected.is_some()
    });
    stage.until("the references panel is a strip", |stage| {
        stage.shows("unfold-refs")
    });
    // The strip is 36 points wide; the right panel's edge is 2 left of
    // its border. In points: the window is 1280 wide at 200 %.
    let zoom = 2.0;
    let (split_left, ..) = stage.place(("split", 1usize));
    let graph = (split_left + 2.0) / zoom - 36.0;
    assert!(graph >= 200.0, "the graph keeps {graph} points");
}
