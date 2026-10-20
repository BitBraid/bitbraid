//! What the window does, as `behaviour.rs` asks it, for access: every view
//! is reached and left by keys, what assistive technology is told names
//! every control and says what the window's own elements are, and motion
//! can be turned off.

use bitbraid_ui::views::workspace::Workspace;
use gpui_kit::{App, FocusHandle, Focusable as _};

use crate::{
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "every_control_on_show_has_a_name",
        run: every_control_on_show_has_a_name,
    },
    Test {
        name: "the_window_s_own_elements_say_what_they_are",
        run: the_window_s_own_elements_say_what_they_are,
    },
    Test {
        name: "the_window_speaks_from_its_catalog",
        run: the_window_speaks_from_its_catalog,
    },
    Test {
        name: "every_view_is_reached_used_and_left_by_keys",
        run: every_view_is_reached_used_and_left_by_keys,
    },
    Test {
        name: "reduced_motion_is_a_setting_that_stops_what_moves",
        run: reduced_motion_is_a_setting_that_stops_what_moves,
    },
];

/// "Reduce motion" in the Interface section sets the toolkit's flag that
/// every animation asks, whatever the system says, and is kept; off again,
/// the flag is the system's (here, where no system is asked, off).
fn reduced_motion_is_a_setting_that_stops_what_moves() {
    let mut stage = Stage::open(Start::bare());
    // Every window of the tests is opened with the flag set; this one
    // starts from motion.
    stage.app(|cx| cx.set_reduce_motion(false));
    stage.keys("ctrl-,");
    stage.click("section-interface");
    assert!(
        stage.shows("check-Reduce motion"),
        "the setting is on the page"
    );
    stage.click("check-Reduce motion");
    assert!(stage.app(|cx| cx.reduce_motion()), "nothing moves");
    let kept = stage.app(|cx| {
        cx.global::<bitbraid_ui::prefs::Prefs>()
            .preferences
            .reduce_motion
    });
    assert!(kept, "the setting is kept");
    let root = crate::hermetic::inside().expect("the tests run in scratch surroundings");
    let file = std::fs::read_to_string(root.join("config/bitbraid/preferences.toml")).unwrap();
    assert!(file.contains("reduce_motion = true"), "{file}");
    stage.click("check-Reduce motion");
    assert!(!stage.app(|cx| cx.reduce_motion()), "motion again");

    // Set in the file, it holds from the start, and a change of the
    // file is followed.
    let mut stage = Stage::open(Start::bare().preferences("reduce_motion = true\n"));
    stage.app(|cx| cx.set_reduce_motion(false));
    stage.app(bitbraid_ui::motion::sync);
    assert!(
        !stage.app(|cx| cx.reduce_motion()),
        "applied once, at the start"
    );
    for on in [false, true] {
        stage.app(|cx| {
            cx.global_mut::<bitbraid_ui::prefs::Prefs>()
                .preferences
                .reduce_motion = on;
        });
        // What the change sets off runs with the next frame.
        stage.frame();
        assert_eq!(stage.app(|cx| cx.reduce_motion()), on);
    }
}

/// How a view is reached from the graph with keys alone.
#[derive(Debug, Clone, Copy)]
enum Reach {
    /// Its own keys.
    Keys(&'static str),
    /// The palette: what is typed there to find it, then Enter.
    Palette(&'static str),
    /// Tab, as often as it takes (a dozen and a half at most) to reach the
    /// part of the window where the keys are, then these keys.
    Tab(fn(&mut Stage) -> bool, &'static str),
}

/// A view of the window, how it is reached, and how it is known to be on
/// show.
struct View {
    name: &'static str,
    reach: Reach,
    shown: fn(&mut Stage) -> bool,
    /// Whether Tab goes from control to control in it. A view of one
    /// field (the palette, a name typed in the graph) keeps the keys in
    /// the field; Up and Down, Enter and Escape are what it takes. In a
    /// panel of the window Tab goes on to the next panel.
    tabs: bool,
    /// The keys that leave it: Escape, unless it says otherwise.
    leave: &'static str,
}

/// What has the keyboard's focus, as something to compare.
fn focused(stage: &mut Stage) -> Option<String> {
    stage.window(|window, cx| window.focused(cx).map(|handle| format!("{handle:?}")))
}

/// Press Tab until the focus comes back to a control it had or has gone
/// round a dozen; answers the controls in the order they were reached,
/// and the one it came back to, if it did.
fn tab_round(stage: &mut Stage) -> (Vec<Option<String>>, Option<Option<String>>) {
    let mut seen = vec![focused(stage)];
    for _ in 0..12 {
        stage.keys("tab");
        let now = focused(stage);
        if seen.contains(&now) {
            return (seen, Some(now));
        }
        seen.push(now);
    }
    (seen, None)
}

/// Walk back with Shift+Tab from where [`tab_round`] stopped: the focus
/// goes through the same controls the other way. Answers how many steps
/// went where they should.
fn walk_back(stage: &mut Stage, seen: &[Option<String>], came_back: bool) -> usize {
    let mut steps = 0usize;
    // Came back round: the step before the first one again is the last.
    let mut expected: Vec<&Option<String>> = seen.iter().skip(1).rev().collect();
    if !came_back {
        // Stopped on the last one: the step back is to the one before.
        expected.remove(0);
    }
    for want in expected {
        stage.keys("shift-tab");
        if focused(stage) != *want {
            break;
        }
        steps = steps.saturating_add(1);
    }
    steps
}

fn reach(stage: &mut Stage, how: Reach) {
    match how {
        Reach::Keys(keys) => stage.keys(keys),
        Reach::Palette(typed) => {
            stage.keys("ctrl-p");
            assert!(stage.shell().palette_open, "the palette opens");
            stage.type_text(typed);
            stage.keys("enter");
        }
        Reach::Tab(there, then) => {
            for _ in 0..18 {
                if there(stage) {
                    break;
                }
                stage.keys("tab");
            }
            stage.keys(then);
        }
    }
    stage.settle();
}

/// Whether the keys are in this part of the shown tab.
fn keys_in(stage: &mut Stage, part: fn(&Workspace, &App) -> FocusHandle) -> bool {
    stage.with_workspace(|workspace, window, cx| {
        part(workspace.read(cx), cx).contains_focused(window, cx)
    })
}

fn in_changes(stage: &mut Stage) -> bool {
    keys_in(stage, |w, cx| w.changes().read(cx).focus_handle(cx))
}

fn in_commit_box(stage: &mut Stage) -> bool {
    keys_in(stage, |w, cx| w.commit_box().read(cx).focus_handle(cx))
}

const VIEWS: &[View] = &[
    View {
        name: "changes",
        reach: Reach::Tab(in_changes, ""),
        shown: in_changes,
        tabs: false,
        leave: "escape",
    },
    View {
        name: "commit_message",
        reach: Reach::Tab(in_commit_box, ""),
        shown: in_commit_box,
        tabs: false,
        leave: "escape",
    },
    View {
        name: "file",
        reach: Reach::Tab(in_changes, "home enter"),
        shown: |stage| stage.workspace().file.is_some(),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "search",
        reach: Reach::Keys("ctrl-f"),
        shown: |stage| stage.graph_focus().1,
        tabs: false,
        leave: "escape",
    },
    View {
        name: "references_filter",
        reach: Reach::Keys("ctrl-alt-f"),
        shown: |stage| {
            let (rows, search) = stage.graph_focus();
            !rows && !search && focused(stage).is_some()
        },
        tabs: false,
        leave: "escape",
    },
    View {
        name: "terminal",
        reach: Reach::Keys("ctrl-`"),
        shown: |stage| stage.terminal().shown,
        tabs: false,
        leave: "ctrl-`",
    },
    View {
        name: "start_page",
        reach: Reach::Keys("ctrl-t"),
        shown: |stage| {
            let shell = stage.shell();
            shell
                .tabs
                .get(shell.active)
                .is_some_and(|tab| tab.path.is_none())
        },
        tabs: true,
        leave: "ctrl-w",
    },
    View {
        name: "palette",
        reach: Reach::Keys("ctrl-p"),
        shown: |stage| stage.shell().palette_open,
        tabs: false,
        leave: "escape",
    },
    View {
        name: "preferences",
        reach: Reach::Keys("ctrl-,"),
        shown: |stage| stage.shell().sheet == Some("preferences"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "keys",
        reach: Reach::Keys("ctrl-/"),
        shown: |stage| stage.shell().sheet == Some("keys"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "shortcuts",
        reach: Reach::Palette("Change keys"),
        shown: |stage| stage.shell().sheet == Some("shortcuts"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "notifications",
        reach: Reach::Keys("ctrl-shift-n"),
        shown: |stage| stage.shell().sheet == Some("notifications"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "inbox",
        reach: Reach::Palette("Inbox: pull requests"),
        shown: |stage| stage.shell().sheet == Some("inbox"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "signing",
        reach: Reach::Palette("Signing: keys"),
        shown: |stage| stage.shell().preferences_section == Some("signing"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "profiles",
        reach: Reach::Palette("Profiles: who"),
        shown: |stage| stage.shell().preferences_section == Some("profiles"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "integrations",
        reach: Reach::Palette("Integrations: accounts"),
        shown: |stage| stage.shell().preferences_section == Some("integrations"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "lfs",
        reach: Reach::Palette("Large files (LFS)"),
        shown: |stage| stage.shell().preferences_section == Some("lfs"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "activity",
        reach: Reach::Keys("ctrl-shift-l"),
        shown: |stage| stage.workspace().sheet == Some("activity"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "undo_history",
        reach: Reach::Keys("alt-shift-h"),
        shown: |stage| stage.workspace().sheet == Some("undo_history"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "index_flags",
        reach: Reach::Palette("Files hidden from the status"),
        shown: |stage| stage.workspace().sheet == Some("index_flags"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "rerere",
        reach: Reach::Palette("Recorded conflict resolutions"),
        shown: |stage| stage.workspace().sheet == Some("rerere"),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "clone",
        reach: Reach::Keys("ctrl-n"),
        shown: |stage| stage.dialog_open(),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "new_repository",
        reach: Reach::Keys("ctrl-i"),
        shown: |stage| stage.dialog_open(),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "tab_list",
        reach: Reach::Keys("ctrl-shift-a"),
        shown: |stage| stage.shows(("tab-list-row", 0usize)),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "new_branch",
        reach: Reach::Keys("ctrl-b"),
        shown: |stage| stage.graph().naming.is_some(),
        tabs: false,
        leave: "escape",
    },
    View {
        name: "jump",
        reach: Reach::Keys("ctrl-g"),
        shown: |stage| stage.shell().bar.is_some(),
        tabs: true,
        leave: "escape",
    },
    View {
        name: "find_file",
        reach: Reach::Keys("ctrl-shift-h"),
        shown: |stage| stage.dialog_open() || stage.shell().bar.is_some(),
        tabs: true,
        leave: "escape",
    },
];

/// Tab goes from control to control of a view that stays on show, and
/// Shift+Tab back the same way; what is wrong is added to `wrong`.
fn tabs_through(stage: &mut Stage, view: &View, wrong: &mut Vec<String>) {
    let (round, again) = tab_round(stage);
    let stops = round.len().saturating_sub(1);
    if stops == 0 {
        wrong.push(format!("{}: Tab reaches no control", view.name));
        return;
    }
    if !(view.shown)(stage) {
        wrong.push(format!("{}: Tab put it away", view.name));
        return;
    }
    let wanted = if again.is_some() {
        stops
    } else {
        stops.saturating_sub(1)
    };
    let back = walk_back(stage, &round, again.is_some());
    if back != wanted {
        wrong.push(format!(
            "{}: Shift+Tab went back {back} of {wanted} steps",
            view.name
        ));
    }
}

/// Every view can be reached from the graph with keys alone, its
/// controls take the focus one after another with Tab and back again in
/// the same order with Shift+Tab while it stays on show, and Escape
/// leaves it with the keys back in the graph.
fn every_view_is_reached_used_and_left_by_keys() {
    crate::repos::fresh();
    let repo = crate::repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    let mut wrong = Vec::new();
    for view in VIEWS {
        if !stage.graph_focus().0 {
            wrong.push(format!(
                "before {}: the keys are not in the graph",
                view.name
            ));
            stage.keys("escape");
        }
        reach(&mut stage, view.reach);
        if !(view.shown)(&mut stage) {
            wrong.push(format!("{}: not reached by {:?}", view.name, view.reach));
            stage.keys("escape");
            continue;
        }
        if view.tabs {
            tabs_through(&mut stage, view, &mut wrong);
        }
        stage.keys(view.leave);
        stage.settle();
        if (view.shown)(&mut stage) {
            wrong.push(format!("{}: {} does not leave it", view.name, view.leave));
            continue;
        }
        if !stage.graph_focus().0 {
            wrong.push(format!("{}: the keys are not back in the graph", view.name));
            stage.keys("escape");
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// The roles of what can be pointed at, pressed, typed into or chosen:
/// each needs a name.
fn is_control(role: gpui_kit::Role) -> bool {
    use gpui_kit::Role;
    matches!(
        role,
        Role::Button
            | Role::CheckBox
            | Role::Switch
            | Role::RadioButton
            | Role::ComboBox
            | Role::Link
            | Role::MenuItem
            | Role::Tab
            | Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::ListBoxOption
            | Role::ListItem
            | Role::TreeItem
            | Role::Terminal
            | Role::ListBox
            | Role::List
            | Role::TabList
            | Role::Dialog
    )
}

/// What is drawn (on show, or only while the pointer is on its row) and
/// has a role but no name, by its role and the last parts of its place in
/// the window. The palette's own list and its dialog are the toolkit's and
/// have none: the toolkit gives their entries no name and takes none for
/// them.
fn unnamed(stage: &mut Stage) -> Vec<String> {
    stage.window(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|element| element.role().is_some_and(is_control))
            .filter(|element| element.label().is_none_or(|label| label.trim().is_empty()))
            .filter(|element| {
                let path = format!("{:?}", element.path());
                !path.contains("command-list") && !path.contains("Name(\"dialog-0\")")
            })
            .map(|element| {
                let place: Vec<String> = element
                    .path()
                    .iter()
                    .rev()
                    .take(2)
                    .map(|part| format!("{part:?}"))
                    .collect();
                format!("{:?} at {}", element.role(), place.join(" in "))
            })
            .collect()
    })
}

/// In the graph, with every section of the references open, and in every
/// view of `VIEWS`, every control has a name for assistive technology: a
/// button of only an icon is named by what its tooltip says, a check box
/// or a field by the words of its row.
fn every_control_on_show_has_a_name() {
    crate::repos::fresh();
    let repo = crate::repos::project_with_changes();
    // Every section of the references open.
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    let mut wrong: Vec<String> = unnamed(&mut stage)
        .into_iter()
        .map(|what| format!("graph: {what}"))
        .collect();
    for view in VIEWS {
        reach(&mut stage, view.reach);
        wrong.extend(
            unnamed(&mut stage)
                .into_iter()
                .map(|what| format!("{}: {what}", view.name)),
        );
        stage.keys(view.leave);
        stage.settle();
        if !stage.graph_focus().0 {
            stage.keys("escape");
        }
    }
    wrong.sort();
    wrong.dedup();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// What has this role and a name that holds these words, on show.
fn named(
    stage: &mut Stage,
    role: gpui_kit::Role,
    words: &str,
) -> Vec<gpui_kit::test::ElementSnapshot> {
    let words = words.to_owned();
    stage.window(move |window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|element| element.visible() && element.role() == Some(role))
            .filter(|element| element.label().is_some_and(|label| label.contains(&words)))
            .collect()
    })
}

/// The window speaks from its catalog: the names it gives the graph, the
/// changes, the tabs and the toolbar's buttons are the catalog's messages,
/// as `bitbraid_ui::strings::CATALOG` reads them, and the catalog's files
/// were read without a fault. (That every word of the interface's code is
/// a lookup, and every lookup is in the catalog, is held by the crate's
/// own `strings::tests`.)
fn the_window_speaks_from_its_catalog() {
    use bitbraid_ui::strings::CATALOG;
    use gpui_kit::Role;
    crate::repos::fresh();
    let repo = crate::repos::project_with_changes();
    let mut stage = Stage::open(Start::with(&repo));
    assert!(CATALOG.problems().is_empty(), "{:?}", CATALOG.problems());
    // The row of uncommitted work selected, so that the changes are on show.
    stage.keys("ctrl-home");
    stage.settle();
    for (role, id) in [
        (Role::ListBox, "graph-access-list"),
        (Role::List, "access-changes-name"),
        (Role::TabList, "access-tabs-name"),
        (Role::Button, "shell-pull"),
        (Role::Button, "shell-push"),
        (Role::Button, "shell-branch"),
        (Role::Button, "shell-stash"),
    ] {
        let words = CATALOG.text(id);
        assert_ne!(words, id, "{id} is a message of the catalog");
        let found = named(&mut stage, role, words);
        assert!(
            found.iter().any(|element| element.label() == Some(words)),
            "something on show is named {words:?} ({id}): {found:?}"
        );
    }
}

/// The graph is a list box of commits whose rows say what each commit
/// says, who wrote it, when, and what is on it, the selected one marked;
/// the row of uncommitted work counts its files; the references are tree
/// items under headings that say whether they are open; the changes are a
/// list of files that say what happened to them; the tabs are a tab list;
/// the terminal says it is one.
fn the_window_s_own_elements_say_what_they_are() {
    use gpui_kit::Role;
    crate::repos::fresh();
    let repo = crate::repos::project_with_changes();
    // Every section of the references open.
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));

    assert_eq!(named(&mut stage, Role::ListBox, "Commits").len(), 1);
    let head = crate::repos::git(&repo, &["log", "-1", "--format=%s%x09%an"]);
    let (summary, author) = head.trim().split_once('\t').unwrap();
    let rows = named(&mut stage, Role::ListBoxOption, summary);
    let row = rows
        .first()
        .unwrap_or_else(|| panic!("a row says {summary:?}"));
    let label = row.label().unwrap_or_default().to_owned();
    assert!(label.contains(&format!("by {author}")), "{label}");
    assert!(label.contains("branch main, checked out"), "{label}");
    let wip = named(&mut stage, Role::ListBoxOption, "Uncommitted changes");
    assert_eq!(wip.len(), 1, "the row of uncommitted work");
    // The selected row says so; moving the selection moves the mark.
    stage.keys("ctrl-home");
    assert_eq!(wip_selected(&mut stage), Some(true));
    stage.keys("down");
    assert_eq!(wip_selected(&mut stage), Some(false));
    let selected = stage.window(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|element| element.role() == Some(Role::ListBoxOption))
            .filter(|element| element.selected() == Some(true))
            .count()
    });
    assert_eq!(selected, 1, "one row is selected");

    // The references.
    let local = named(&mut stage, Role::Button, "Branches, ");
    let heading = local.first().expect("the heading of the local branches");
    assert_eq!(heading.expanded(), Some(true), "{heading:?}");
    let main = named(&mut stage, Role::TreeItem, "main, checked out");
    assert!(!main.is_empty(), "the checked-out branch's row");

    // The changes: their rows show once the row of uncommitted work is
    // selected.
    stage.keys("ctrl-home");
    stage.settle();
    assert_eq!(named(&mut stage, Role::List, "Changed files").len(), 1);
    let files = stage.window(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|element| element.visible() && element.role() == Some(Role::ListItem))
            .filter_map(|element| element.label().map(str::to_owned))
            .collect::<Vec<_>>()
    });
    assert!(
        files.iter().any(|file| file.ends_with(", not staged")),
        "{files:?}"
    );

    // The tabs.
    assert_eq!(named(&mut stage, Role::TabList, "Tabs").len(), 1);
    let tabs = stage.window(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|element| element.visible() && element.role() == Some(Role::Tab))
            .map(|element| element.selected())
            .collect::<Vec<_>>()
    });
    assert!(tabs.contains(&Some(true)), "{tabs:?}");

    // The terminal.
    stage.keys("ctrl-`");
    stage.settle();
    assert_eq!(named(&mut stage, Role::Terminal, "Terminal").len(), 1);
}

/// Whether the row of uncommitted work is marked selected.
fn wip_selected(stage: &mut Stage) -> Option<bool> {
    named(stage, gpui_kit::Role::ListBoxOption, "Uncommitted changes")
        .first()
        .and_then(|row| row.selected())
}
