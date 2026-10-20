//! What the window does, as `behaviour.rs` asks it, when files are dragged
//! out of its lists.

use std::path::{Path, PathBuf};

use gpui_kit::test::TestWindowExt as _;

use crate::{
    repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[Test {
    name: "rows_of_files_drag_their_full_paths",
    run: rows_of_files_drag_their_full_paths,
}];

/// What follows the pointer while paths are dragged: what it says, and
/// the paths it names. None while nothing is dragged.
fn dragged(stage: &mut Stage) -> Option<(String, Vec<PathBuf>)> {
    stage.window(|window, _| {
        let chip = window.try_find("dragged-paths")?;
        Some((
            chip.label().unwrap_or_default().to_owned(),
            chip.value()
                .unwrap_or_default()
                .lines()
                .map(PathBuf::from)
                .collect(),
        ))
    })
}

/// A drag from one place to another, looked at before it is let go, and
/// let go where it went: on nothing that takes paths.
fn drag(stage: &mut Stage, from: (f32, f32)) -> (String, Vec<PathBuf>) {
    let to = (from.0 + 150., from.1 + 6.);
    stage.press_and_move(from, to);
    let seen = dragged(stage).expect("a drag of paths is under way");
    stage.release_at(to);
    assert_eq!(dragged(stage), None, "the drag ended where it was let go");
    seen
}

fn full(top: &Path, relative: &str) -> PathBuf {
    top.join(relative)
}

/// The rows of files drag their files by their full paths: a row of the
/// changes lists drags its file, one of several selected rows drags all
/// of them, a folder of the tree drags its folder, a file of a commit
/// drags its path in the worktree. Clicking still opens a file, and a
/// drag clicks nothing on its way.
fn rows_of_files_drag_their_full_paths() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let top = repo.canonicalize().unwrap();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    let shown = |stage: &mut Stage| stage.workspace().file.map(|(path, _)| path);
    let (x, y, _, _) = stage.place("unstaged-rows");
    let (first, second) = ((x + 80., y + 16.), (x + 80., y + 48.));

    // Which files the first rows are: a click opens each.
    stage.click_at(second);
    let b = shown(&mut stage).expect("a click opens the second file");
    stage.click_at(first);
    let a = shown(&mut stage).expect("a click opens the first file");
    assert_ne!(a, b);

    // One row: its file, by its full path; the file stays open.
    let (label, paths) = drag(&mut stage, first);
    assert_eq!(paths, [full(&top, &a)]);
    assert_eq!(
        label,
        Path::new(&a).file_name().unwrap().to_string_lossy(),
        "the chip says the file's name"
    );
    assert_eq!(shown(&mut stage).as_deref(), Some(a.as_str()), "no click");

    // Two selected: either drags both.
    stage.click_at_with(second, gpui_kit::Modifiers::secondary_key());
    assert_eq!(shown(&mut stage), None, "two are selected");
    let (label, paths) = drag(&mut stage, second);
    assert_eq!(paths, [full(&top, &a), full(&top, &b)]);
    assert_eq!(label, "2 files");
    let (_, again) = drag(&mut stage, first);
    assert_eq!(again, paths);
    // A row that is not selected drags itself alone: the staged file.
    let (x, y, _, _) = stage.place("staged-rows");
    let (_, paths) = drag(&mut stage, (x + 80., y + 16.));
    assert_eq!(paths, [full(&top, "docs/guide.md")]);
    // Nothing was clicked on the way: the two are still selected.
    assert_eq!(shown(&mut stage), None);

    // A file of a commit: its path in the worktree.
    let commit = stage.graph_spot(1, false);
    stage.click_at(commit);
    stage.until("a commit is selected", |stage| !stage.graph().wip_selected);
    stage.until("its files are listed", |stage| stage.shows("commit-files"));
    let (fx, fy, _, _) = stage.place("commit-files");
    let file = (fx + 80., fy + 16.);
    stage.click_at(file);
    let in_commit = shown(&mut stage).expect("a click opens the commit's file");
    let (_, paths) = drag(&mut stage, file);
    assert_eq!(paths, [full(&top, &in_commit)]);

    // A folder of the tree: its folder.
    stage.click("view-changes");
    stage.until("the uncommitted work is selected again", |stage| {
        stage.graph().wip_selected
    });
    stage.focus_changes();
    stage.keys("t");
    let (x, y, _, _) = stage.place("unstaged-rows");
    // Under the row that opens every folder, the first folder.
    let (label, paths) = drag(&mut stage, (x + 80., y + 30. + 10.));
    assert_eq!(paths.len(), 1, "{paths:?}");
    assert!(paths[0].is_dir(), "{paths:?}");
    assert!(paths[0].starts_with(&top));
    assert_eq!(paths[0].file_name().unwrap().to_string_lossy(), label);
}
