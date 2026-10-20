//! What the window does, as `behaviour.rs` asks it, for the references
//! panel at the left.

use bitbraid_ui::views::{
    refs::RefsView,
    stashes::{RunStash, StashRequest},
    tags::{RunTag, TagRequest},
};
use bitbraid_ui::{
    keymap,
    prefs::Prefs,
    views::refs::{ToggleHidden, ToggleSolo},
};
use gpui_kit::{
    ElementId, Entity, InputEvent as _, MouseMoveEvent, Pixels, Point, point, px,
    test::TestWindowExt as _,
};

use crate::{
    repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "every_section_starts_closed_and_the_open_ones_share_the_height",
        run: every_section_starts_closed_and_the_open_ones_share_the_height,
    },
    Test {
        name: "deleting_a_tag_and_dropping_a_stash_are_asked_in_the_bar",
        run: deleting_a_tag_and_dropping_a_stash_are_asked_in_the_bar,
    },
    Test {
        name: "a_click_on_a_branch_selects_its_commit_and_the_graph_takes_the_keys",
        run: a_click_on_a_branch_selects_its_commit_and_the_graph_takes_the_keys,
    },
    Test {
        name: "an_old_branch_says_it_is_stale_on_its_row_and_in_its_tooltip",
        run: an_old_branch_says_it_is_stale_on_its_row_and_in_its_tooltip,
    },
    Test {
        name: "the_eye_at_the_left_of_a_row_hides_a_branch_or_a_folder_and_shows_it_again",
        run: the_eye_at_the_left_of_a_row_hides_a_branch_or_a_folder_and_shows_it_again,
    },
    Test {
        name: "the_top_line_says_what_narrows_what_is_seen_and_undoes_it",
        run: the_top_line_says_what_narrows_what_is_seen_and_undoes_it,
    },
    Test {
        name: "a_heading_under_the_pointer_has_its_add_button_in_its_counts_place",
        run: a_heading_under_the_pointer_has_its_add_button_in_its_counts_place,
    },
    Test {
        name: "a_mark_of_the_folded_strip_shows_its_section_in_a_box_beside_it",
        run: a_mark_of_the_folded_strip_shows_its_section_in_a_box_beside_it,
    },
    Test {
        name: "a_branch_dropped_on_another_in_the_panel_offers_what_the_two_can_do",
        run: a_branch_dropped_on_another_in_the_panel_offers_what_the_two_can_do,
    },
    Test {
        name: "a_new_tab_lists_the_recent_ones_and_a_rows_menu_does_the_rest",
        run: a_new_tab_lists_the_recent_ones_and_a_rows_menu_does_the_rest,
    },
    Test {
        name: "the_palette_hangs_from_the_toolbar_with_five_rows_of_forty",
        run: the_palette_hangs_from_the_toolbar_with_five_rows_of_forty,
    },
    Test {
        name: "the_forms_that_make_a_repository_are_the_large_dialog",
        run: the_forms_that_make_a_repository_are_the_large_dialog,
    },
    Test {
        name: "resting_on_a_branch_in_the_panel_lights_its_history_in_the_graph",
        run: resting_on_a_branch_in_the_panel_lights_its_history_in_the_graph,
    },
];

/// With the pointer resting on a branch in the references panel, the
/// graph lights that branch's commit and what its first parents lead to:
/// lit a moment after the pointer came to rest, while the tooltip is on
/// show, and put out when the pointer leaves.
fn resting_on_a_branch_in_the_panel_lights_its_history_in_the_graph() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    let tip = repos::rev(&repo, "feature/dates");
    let at = name_at(&mut stage, "refs/heads/feature/dates");
    let (x, y) = (f32::from(at.x), f32::from(at.y));
    stage.point_at((x + 1.0, y));
    stage.point_at((x, y));
    stage.pass(std::time::Duration::from_millis(300));
    let lit = stage.graph().lit;
    assert_eq!(
        lit.first(),
        Some(&tip),
        "the branch's commit is lit: {lit:?}"
    );
    // The tooltip comes; the light stays.
    stage.pass(std::time::Duration::from_secs(2));
    assert_eq!(stage.graph().lit.first(), Some(&tip), "still lit");
    // Away from the panel: out.
    stage.point_at((640.0, 400.0));
    stage.pass(std::time::Duration::from_millis(300));
    assert!(stage.graph().lit.is_empty(), "{:?}", stage.graph().lit);
}

/// The sections the panel shows, and the height of each open one's list.
fn sections(stage: &mut Stage) -> Vec<(&'static str, Option<f32>)> {
    shown(stage)
        .into_iter()
        .map(|(key, _, height)| (key, height))
        .collect()
}

/// The same, with where each heading is.
fn shown(stage: &mut Stage) -> Vec<(&'static str, Point<Pixels>, Option<f32>)> {
    stage.with_workspace(|workspace, _, cx| {
        let refs: Entity<RefsView> = workspace.read(cx).refs();
        refs.read(cx).sections_shown()
    })
}

/// Open or close a section with a click on its heading.
fn toggle(stage: &mut Stage, section: &str) {
    let (_, head, _) = shown(stage)
        .into_iter()
        .find(|(key, _, _)| *key == section)
        .expect("the section is on show");
    assert!(head.y > px(0.), "the heading of {section} was drawn");
    stage.click_at((f32::from(head.x), f32::from(head.y)));
    stage.settle();
}

/// A repository is first looked at with every section closed; a section
/// with nothing in it is not there, nor gitflow's where it is not set up;
/// an open list has five rows and a half but for the last, which takes what
/// is left; a list that closes gives its room to the one under it; and in a
/// low window every list has four rows and a half and the sections scroll.
fn every_section_starts_closed_and_the_open_ones_share_the_height() {
    repos::fresh();
    // A remote, a second worktree, a stash, tags and a submodule; no
    // gitflow.
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let shown = sections(&mut stage);
    assert_eq!(
        shown,
        [
            ("local", None),
            ("remote", None),
            ("worktrees", None),
            ("stashes", None),
            ("tags", None),
            ("submodules", None),
        ],
        "all closed, and no gitflow"
    );
    // Three sections only from here, so that the window has room for
    // three lists: the others hidden, as the headings' menu hides them.
    drop(stage);
    let state = format!(
        "[repos.{repo:?}]\nhidden_sections = [\"worktrees\", \"stashes\", \"submodules\"]\n"
    );
    let mut stage = Stage::open(Start::with(&repo).state(state));
    // What the lists have together: the sections' room less the headings.
    toggle(&mut stage, "local");
    let room = match sections(&mut stage).first() {
        Some(("local", Some(room))) => *room,
        other => panic!("the branches are open alone: {other:?}"),
    };
    assert!(
        room > 132.0 + 108.0,
        "the window has room for two lists: {room}"
    );
    let heights = |stage: &mut Stage| -> Vec<f32> {
        sections(stage)
            .iter()
            .filter_map(|(_, height)| *height)
            .collect()
    };
    // Two open: five rows and a half, and the rest.
    toggle(&mut stage, "tags");
    assert_eq!(heights(&mut stage), [132.0, room - 132.0]);
    // A third between them, for the first time: from the last, which
    // keeps the least.
    toggle(&mut stage, "remote");
    let three = heights(&mut stage);
    assert_eq!(three.len(), 3, "{three:?}");
    assert_eq!(three.first(), Some(&132.0));
    assert_eq!(three.iter().sum::<f32>(), room.max(132.0 + 108.0 + 108.0));
    assert!(three.iter().all(|height| *height >= 108.0), "{three:?}");
    // The first of three closes: the one under it gets its room; opened
    // again, it takes it back.
    toggle(&mut stage, "local");
    assert_eq!(heights(&mut stage), [three[1] + three[0], three[2]]);
    toggle(&mut stage, "local");
    assert_eq!(heights(&mut stage), three);

    // Six open in a window that has room for fewer: each has the least,
    // and the sections are taller than the panel, which scrolls them.
    drop(stage);
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    let all = heights(&mut stage);
    assert_eq!(all.len(), 6, "{:?}", sections(&mut stage));
    assert!(all.iter().all(|height| *height >= 108.0), "{all:?}");
    assert!(
        all.iter().sum::<f32>() > room,
        "the sections scroll: {all:?} in {room}"
    );
}

/// A tag is deleted and a stash dropped after a question in the bar that
/// takes the toolbar's place, not in a dialog: Escape gives up and leaves
/// it, Enter goes ahead.
fn deleting_a_tag_and_dropping_a_stash_are_asked_in_the_bar() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let tags = |repo: &std::path::Path| repos::git(repo, &["tag", "--list"]);
    assert!(tags(&repo).contains("v0.1"));

    stage.act(RunTag {
        request: TagRequest::Delete("v0.1".to_owned()),
    });
    let (said, _) = stage.shell().bar.expect("the question is in the bar");
    assert!(said.contains("v0.1"), "{said}");
    assert!(!stage.dialog_open(), "a bar, not a dialog");
    stage.keys("escape");
    assert!(stage.shell().bar.is_none());
    stage.pass(std::time::Duration::from_secs(1));
    assert!(tags(&repo).contains("v0.1"), "given up: the tag stays");

    stage.act(RunTag {
        request: TagRequest::Delete("v0.1".to_owned()),
    });
    assert!(stage.shell().bar.is_some());
    stage.keys("enter");
    stage.until("the tag is deleted", |_| !tags(&repo).contains("v0.1"));

    let stash = repos::git(&repo, &["rev-parse", "stash@{0}"]);
    stage.act(RunStash {
        request: StashRequest::Drop(stash.trim().to_owned()),
    });
    assert!(stage.shell().bar.is_some() && !stage.dialog_open());
    stage.keys("enter");
    stage.until("the stash is dropped", |_| {
        repos::git(&repo, &["stash", "list"]).trim().is_empty()
    });
}

/// The panel has no keys of its own: a click on a branch selects the commit
/// it is at in the graph, and the arrow keys go on from there, in the
/// graph.
fn a_click_on_a_branch_selects_its_commit_and_the_graph_takes_the_keys() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    let name = stage.window(|window, _| {
        window
            .try_find(ElementId::Name("ref-name:refs/heads/feature/dates".into()))
            .filter(|found| found.visible())
            .map(|found| found.bounds().center())
    });
    let name = name.expect("the branch's row is on show");
    stage.click_at((f32::from(name.x), f32::from(name.y)));
    stage.settle();
    let dates = repos::rev(&repo, "feature/dates");
    assert_eq!(stage.graph().selected.as_deref(), Some(dates.as_str()));
    assert_eq!(stage.graph_focus(), (true, false), "the graph has the keys");
    // The next key moves in the graph, not in the panel.
    stage.keys("down");
    let next = stage.graph().selected;
    assert!(
        next.is_some() && next.as_deref() != Some(dates.as_str()),
        "the arrow went on in the graph: {next:?}"
    );
}

/// What the row of a reference says at its right and in its tooltip.
fn row_said(stage: &mut Stage, full_name: &str) -> Option<(Option<String>, Option<String>)> {
    stage.with_workspace(|workspace, _, cx| {
        let refs: Entity<RefsView> = workspace.read(cx).refs();
        refs.read(cx).row_said(full_name)
    })
}

/// A local branch whose last commit is older than the preferences allow
/// says "stale" on its row, beside how far it is ahead and behind, and its
/// tooltip says so with its age.
fn an_old_branch_says_it_is_stale_on_its_row_and_in_its_tooltip() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    // The fixtures' commits are two days and more older than the clock
    // the tests are held at: stale after one day, fresh after ninety.
    let mut stage = Stage::open(
        Start::with(&repo)
            .state(state.clone())
            .preferences("stale_days = 1\n"),
    );
    stage.until("the old branch says it is stale", |stage| {
        row_said(stage, "refs/heads/feature/dates")
            .and_then(|(aside, _)| aside)
            .is_some_and(|aside| aside.ends_with("stale"))
    });
    let (_, tip) = row_said(&mut stage, "refs/heads/feature/dates").expect("its row");
    let tip = tip.unwrap_or_default();
    assert!(
        tip.lines()
            .any(|line| line.starts_with("Stale: last commit ")),
        "the tooltip says it is stale, with its age: {tip:?}"
    );

    let mut stage = Stage::open(
        Start::with(&repo)
            .state(state)
            .preferences("stale_days = 90\n"),
    );
    stage.until("the branch's row is there", |stage| {
        row_said(stage, "refs/heads/feature/dates")
            .and_then(|(_, tip)| tip)
            .is_some_and(|tip| tip.contains("Last commit "))
    });
    stage.settle();
    let (aside, tip) = row_said(&mut stage, "refs/heads/feature/dates").expect("its row");
    assert!(
        aside
            .as_deref()
            .is_none_or(|aside| !aside.contains("stale")),
        "not stale after ninety days: {aside:?}"
    );
    assert!(
        tip.as_deref().is_some_and(|tip| !tip.contains("Stale")),
        "{tip:?}"
    );
}

/// Where a branch's name is drawn, by its full name.
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

/// The left end of a row, under the pointer, is an eye that hides the
/// reference from the graph; hidden, it is a crossed eye that shows it
/// again. A folder's hides everything in it.
fn the_eye_at_the_left_of_a_row_hides_a_branch_or_a_folder_and_shows_it_again() {
    repos::fresh();
    // No remote: a branch's own commit goes from the graph with it.
    let repo = repos::project();
    // The branches open alone, so that every one of them has its row.
    let state = format!(
        "[repos.{repo:?}]\nclosed_sections = [\"remote\", \"worktrees\", \"stashes\", \"tags\", \"submodules\"]\n"
    );
    let mut stage = Stage::open(Start::with(&repo).state(state));
    let hidden = |stage: &mut Stage| {
        let repo = repo.clone();
        stage.app(move |cx| cx.global::<Prefs>().repo_state(&repo).hidden.clone())
    };
    let all = stage.graph().commits;
    // The eye is where the check mark of the checked-out branch would be:
    // 13 from the panel's edge, at the row's height.
    let eye = |at: Point<Pixels>| (13.0, f32::from(at.y));

    // A branch with a commit of its own: the commit goes with it.
    let wip = name_at(&mut stage, "refs/heads/wip");
    stage.click_at(eye(wip));
    stage.settle();
    assert_eq!(hidden(&mut stage), ["refs/heads/wip"]);
    stage.until("the branch's own commit is out of the graph", |stage| {
        stage.graph().commits < all
    });
    stage.click_at(eye(wip));
    stage.settle();
    assert!(hidden(&mut stage).is_empty(), "shown again");
    stage.until("and in the graph again", |stage| {
        stage.graph().commits == all
    });

    // A folder: the row above its first branch. Everything in it is
    // hidden with it (all of it is merged here, so the graph keeps its
    // commits), and the section says how many of its branches show.
    let dates = name_at(&mut stage, "refs/heads/feature/dates");
    let folder = (13.0, f32::from(dates.y) - 24.0);
    stage.click_at(folder);
    stage.settle();
    assert_eq!(hidden(&mut stage), ["refs/heads/feature/"]);
    let local = stage
        .with_workspace(|workspace, _, cx| workspace.read(cx).refs().read(cx).seen_of("local", cx));
    assert_eq!(
        local,
        Some((3, 6)),
        "three of six branches are in the folder"
    );
    stage.click_at(folder);
    stage.settle();
    assert!(hidden(&mut stage).is_empty());

    // At the right end of a row under the pointer, a button opens its
    // menu, the one the other button of the pointer opens.
    assert!(!stage.shows("popup-menu"));
    let wip = name_at(&mut stage, "refs/heads/wip");
    stage.click_at((190.0, f32::from(wip.y)));
    stage.settle();
    assert!(stage.shows("popup-menu"), "the row's menu is open");
    stage.keys("escape");
}

/// What the line at the top of the panel says.
fn top(stage: &mut Stage) -> (&'static str, String, Option<&'static str>) {
    stage.with_workspace(|workspace, _, cx| workspace.read(cx).refs().read(cx).top_said(cx))
}

/// The line over the filter says how many references the graph shows of
/// all, and what narrows them, with a button that undoes it: what is
/// hidden, what is shown alone, a filter.
fn the_top_line_says_what_narrows_what_is_seen_and_undoes_it() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    // Six branches and two tags.
    assert_eq!(top(&mut stage), ("Viewing", "8".to_owned(), None));
    let undo = |stage: &mut Stage| {
        stage.click("refs-undo-narrowing");
        stage.settle();
    };

    stage.act(ToggleHidden {
        full_name: "refs/heads/wip".to_owned(),
    });
    assert_eq!(
        top(&mut stage),
        ("Viewing", "7/8".to_owned(), Some("Show All"))
    );
    undo(&mut stage);
    assert_eq!(top(&mut stage), ("Viewing", "8".to_owned(), None));

    stage.act(ToggleSolo {
        full_name: "refs/heads/pages".to_owned(),
    });
    assert_eq!(
        top(&mut stage),
        ("Showing only", "1/8".to_owned(), Some("Show All"))
    );
    undo(&mut stage);
    assert_eq!(top(&mut stage), ("Viewing", "8".to_owned(), None));

    stage.act(keymap::FilterReferences);
    stage.type_text("feature");
    stage.settle();
    assert_eq!(
        top(&mut stage),
        ("Filtering", "3/8".to_owned(), Some("Clear filter"))
    );
    undo(&mut stage);
    assert_eq!(top(&mut stage), ("Viewing", "8".to_owned(), None));
}

/// Under the pointer, the heading of the submodules has a button in its
/// count's place that adds one.
fn a_heading_under_the_pointer_has_its_add_button_in_its_counts_place() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let (_, head, _) = shown(&mut stage)
        .into_iter()
        .find(|(key, _, _)| *key == "submodules")
        .expect("the submodules are on show");
    assert!(!stage.dialog_open());
    // The button stands at the heading's right end, where its count is.
    // The pointer comes onto the heading first, and the button shows.
    let at = point(px(186.0), head.y);
    stage.window(|window, cx| {
        window.dispatch_event(
            MouseMoveEvent {
                position: at,
                pressed_button: None,
                modifiers: Default::default(),
            }
            .to_platform_input(),
            cx,
        );
    });
    stage.settle();
    // The submodules' number among the sections there can be, counted
    // from nought, is eight (after the pull requests and the issues):
    // their first button.
    let button = stage.window(|window, _| {
        window
            .try_find(("refs-add", 80usize))
            .map(|found| (found.visible(), found.bounds()))
    });
    let (visible, bounds) = button.expect("the heading has a button");
    assert!(visible, "it shows under the pointer");
    let centre = bounds.center();
    stage.click_at((f32::from(centre.x), f32::from(centre.y)));
    stage.settle();
    assert_eq!(
        stage.workspace().docked,
        Some("submodule"),
        "the form that adds a submodule, docked beside the panel"
    );
    assert_eq!(
        sections(&mut stage).last(),
        Some(&("submodules", None)),
        "the click was the button's, not the heading's"
    );
    stage.keys("escape");
}

/// Folded to its strip, a click on a section's mark shows that section in a
/// box beside the strip, over the graph, and the panel stays folded; a
/// click elsewhere puts it away.
fn a_mark_of_the_folded_strip_shows_its_section_in_a_box_beside_it() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let peeking = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).refs().read(cx).peeking())
    };
    stage.click("fold-refs");
    stage.settle();
    assert!(stage.shows("unfold-refs"), "folded to the strip");
    // The tags' number among the sections there can be, counted from
    // nought: seven, after the pull requests and the issues (this
    // repository has neither).
    let mark = stage.window(|window, _| {
        window
            .try_find(("refs-rail", 7usize))
            .map(|found| found.bounds().center())
    });
    let mark = mark.expect("the strip has a mark for the tags");
    stage.click_at((f32::from(mark.x), f32::from(mark.y)));
    stage.settle();
    assert_eq!(peeking(&mut stage), Some("tags"));
    assert!(stage.shows("unfold-refs"), "and the panel stays folded");
    assert!(stage.shows("refs-peek"), "the box is on show");
    // A click on the graph puts it away.
    stage.click_at((700.0, 400.0));
    stage.settle();
    assert_eq!(peeking(&mut stage), None);
    stage.click("unfold-refs");
}

/// A branch dragged onto another branch's row in the panel offers what the
/// two can do together, the menu a drop on a row of the graph offers.
fn a_branch_dropped_on_another_in_the_panel_offers_what_the_two_can_do() {
    repos::fresh();
    let repo = repos::project();
    let state = format!(
        "[repos.{repo:?}]\nclosed_sections = [\"remote\", \"worktrees\", \"stashes\", \"tags\", \"submodules\"]\n"
    );
    let mut stage = Stage::open(Start::with(&repo).state(state));
    assert!(!stage.graph().drop_menu);
    let wip = name_at(&mut stage, "refs/heads/wip");
    let pages = name_at(&mut stage, "refs/heads/pages");
    stage.drag_at(
        (f32::from(wip.x), f32::from(wip.y)),
        (f32::from(pages.x), f32::from(pages.y)),
    );
    stage.settle();
    assert!(stage.graph().drop_menu, "the menu of what the two can do");
    stage.keys("escape");
}

/// The page of a new tab lists the recent repositories once, the
/// favourites among them, each row its name and where it is and nothing
/// more, 22 apart. The row's menu (the pointer's other button) makes one a
/// favourite, moves it and takes it out; a click on one whose folder is
/// gone asks where it is now.
fn a_new_tab_lists_the_recent_ones_and_a_rows_menu_does_the_rest() {
    repos::fresh();
    let (project, library) = (repos::project(), repos::library());
    let gone = repos::work().join("gone");
    let state =
        format!("recent = [{project:?}, {library:?}, {gone:?}]\nfavourites = [{project:?}]\n");
    let mut stage = Stage::open(Start::bare().state(state));
    let row = |path: &std::path::Path| format!("rec-{}", path.display());
    for path in [&project, &library, &gone] {
        assert!(stage.shows(row(path)), "{}", path.display());
        for what in ["star", "up", "down", "gone", "locate"] {
            assert!(!stage.shows(format!("{what}-{}", row(path))), "{what}");
        }
    }
    let first = stage.place(row(&project));
    let second = stage.place(row(&library));
    assert_eq!(first.3, 22.);
    assert_eq!(second.1 - first.1, 22., "one under the other");
    // The three buttons, one word each, 10 apart.
    let open = stage.place("browse");
    let (clone, init) = (stage.place("clone"), stage.place("init"));
    assert_eq!((open.2, clone.2, init.2), (91., 95., 95.));
    assert_eq!(clone.0 - (open.0 + open.2), 10.);
    assert_eq!(init.0 - (clone.0 + clone.2), 10.);
    assert_eq!(open.3, 45.);

    let recent = |stage: &mut Stage| stage.app(|cx| cx.global::<Prefs>().state.recent.clone());
    let favourites =
        |stage: &mut Stage| stage.app(|cx| cx.global::<Prefs>().state.favourites.clone());
    // Open, | a favourite, up, down, | take out.
    stage.right_click(row(&library));
    assert!(stage.shows("popup-menu"), "the row's menu");
    stage.keys("down down enter");
    assert_eq!(favourites(&mut stage), [project.clone(), library.clone()]);
    stage.right_click(row(&library));
    stage.keys("down down down enter");
    assert_eq!(
        recent(&mut stage),
        [library.clone(), project.clone(), gone.clone()]
    );
    stage.right_click(row(&library));
    stage.keys("down down down down down enter");
    assert_eq!(recent(&mut stage), [project.clone(), gone.clone()]);
    assert!(!stage.shows(row(&library)));
    assert_eq!(
        favourites(&mut stage),
        [project.clone(), library.clone()],
        "still a favourite"
    );

    // A folder that is gone: a click asks where it is now.
    assert!(!stage.dialog_open());
    stage.click(row(&gone));
    assert!(stage.dialog_open() && stage.shows("locate-browse"));
}

/// The palette begins at the toolbar's bottom edge, 600 wide and centred;
/// its rows are 40 high and five are on show, the rest scrolled to.
fn the_palette_hangs_from_the_toolbar_with_five_rows_of_forty() {
    repos::fresh();
    let mut stage = Stage::open(Start::with(repos::project()));
    stage.act(keymap::TogglePalette);
    assert!(stage.dialog_open(), "the palette");
    stage.pass(std::time::Duration::from_millis(600));
    let width = stage.window(|window, _| f32::from(window.viewport_size().width));
    let (x, y, w, _) = frame(&mut stage);
    // To half a point: the dialog slides into its place.
    assert!(
        (y - 82.).abs() <= 0.5 && w == 600.,
        "under the toolbar, 600 wide: {y} {w}"
    );
    assert_eq!(x, (width - 600.) / 2., "centred");
    let (_, _, _, h) = stage.place("command");
    // A row of the first section, and the next.
    let row = |stage: &mut Stage, at: usize| {
        stage.place(gpui_kit::component::IndexPath::new(at).section(0))
    };
    let (first, second) = (row(&mut stage, 0), row(&mut stage, 1));
    assert_eq!((first.3, second.1 - first.1), (40., 40.));
    // The field, the list's inset and five rows of 40: no more on show.
    assert!(h <= 4. + 33. + 8. + 5. * 40. + 1., "{h}");
    stage.keys("escape");
    assert!(!stage.dialog_open());
}

/// Cloning and a new repository are asked in the large dialog: 998 wide and
/// centred, from 83 under the top, 852 high where there is room and to 16
/// above the bottom where there is not; a cross gives up, and the one
/// button waits until the form will do.
fn the_forms_that_make_a_repository_are_the_large_dialog() {
    repos::fresh();
    let into = repos::work().join("clones");
    std::fs::create_dir_all(&into).unwrap();
    let mut stage = Stage::open(Start::bare().state(format!("clone_dir = {into:?}\n")));
    let size = stage.window(|window, _| window.viewport_size());
    let (width, height) = (f32::from(size.width), f32::from(size.height));
    for (button, ready) in [("clone", repos::library()), ("init", into.join("fresh"))] {
        stage.click(button);
        assert!(stage.dialog_open(), "{button}");
        stage.pass(std::time::Duration::from_millis(600));
        let (x, y, w, h) = frame(&mut stage);
        assert!((y - 83.).abs() <= 0.5, "{button}: {y}");
        assert_eq!(w, 998.0_f32.min(width - 32.), "{button}");
        assert_eq!(x, (width - w) / 2., "{button}: centred");
        assert_eq!(
            h,
            (height - 83. - 16.).clamp(400., 852.).min(height - 32.),
            "{button}"
        );
        assert!(!stage.shell().form_ready, "{button}: nothing typed yet");
        let typed = match button {
            "clone" => ready.display().to_string(),
            _ => "fresh".to_owned(),
        };
        stage.type_text(&typed);
        assert!(stage.shell().form_ready, "{button}: it will do");
        stage.click("form-close");
        assert!(!stage.dialog_open(), "{button}: the cross gives up");
    }
}

/// Where the dialog on show is: its left and top edge, width and height.
fn frame(stage: &mut Stage) -> (f32, f32, f32, f32) {
    stage.window(|window, _| {
        let bounds = window.within("dialog").find(ElementId::Integer(0)).bounds();
        (
            f32::from(bounds.origin.x),
            f32::from(bounds.origin.y),
            f32::from(bounds.size.width),
            f32::from(bounds.size.height),
        )
    })
}
