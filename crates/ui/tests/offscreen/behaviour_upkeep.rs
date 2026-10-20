//! What the window does, as `behaviour.rs` asks it, for a repository's
//! upkeep: the banner of long paths and WSL, the maintenance section, and
//! the sparse checkout and partial clone.

use bitbraid_ui::{
    keymap,
    views::{
        file_actions::FileAction,
        maintenance::{Job, MaintenanceEvent, Task},
    },
};

use crate::{
    repos,
    runner::{Shots, Test, Themes},
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "the_maintenance_section_measures_runs_and_checks",
        run: the_maintenance_section_measures_runs_and_checks,
    },
    Test {
        name: "maintenance_is_in_the_repositorys_panel",
        run: maintenance_is_in_the_repositorys_panel,
    },
    Test {
        name: "a_folder_is_checked_out_alone_and_everything_again",
        run: a_folder_is_checked_out_alone_and_everything_again,
    },
    Test {
        name: "the_clone_dialog_offers_a_partial_clone",
        run: the_clone_dialog_offers_a_partial_clone,
    },
    Test {
        name: "a_treeless_clone_is_fetched_from_only_when_asked",
        run: a_treeless_clone_is_fetched_from_only_when_asked,
    },
    Test {
        name: "packs_added_while_the_tab_is_open_are_read",
        run: packs_added_while_the_tab_is_open_are_read,
    },
    Test {
        name: "the_windows_banner_turns_long_paths_on_and_is_put_away",
        run: the_windows_banner_turns_long_paths_on_and_is_put_away,
    },
];

/// Hand the shown tab what Windows would say of a repository with long
/// paths, inside WSL: this system says nothing of either.
pub fn show_windows_notes(stage: &mut Stage) {
    use bitbraid_ui::views::workspace::PlatformNotes;
    stage.with_workspace(|workspace, _, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.show_platform_notes(
                PlatformNotes {
                    long_paths: Some((3, 284)),
                    wsl: Some("Ubuntu-24.04".to_owned()),
                    hidden: false,
                },
                cx,
            )
        })
    });
    stage.frame();
}

/// The banner's button turns git's long paths on in the repository (the
/// command runs here as there), and asked again this system has nothing
/// to say of long paths, so the line goes; the line of WSL stays until
/// the banner is put away.
fn the_windows_banner_turns_long_paths_on_and_is_put_away() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    assert!(!stage.shows("platform-banner"), "nothing to say here");
    show_windows_notes(&mut stage);
    assert!(stage.shows("platform-banner") && stage.shows("platform-long-paths"));
    stage.click("platform-long-paths");
    stage.until("long paths are on", |stage| {
        !stage.shows("platform-long-paths")
    });
    let git = bitbraid_testkit::Git::new(&repo, bitbraid_testkit::fixtures_out_dir().join(".home"));
    let set = git
        .run(["config", "--local", "--get", "core.longpaths"])
        .unwrap();
    assert_eq!(set.trim(), "true");
    assert!(stage.shows("platform-banner"));
    stage.click("platform-hide");
    assert!(!stage.shows("platform-banner"));
}

/// What the maintenance view of the shown tab says: whether it measures,
/// the count of objects measured, what a task said, what the check found.
fn upkeep(stage: &mut Stage) -> (bool, Option<u64>, Option<String>, Option<bool>) {
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace.read(cx).maintenance_view();
        let view = view.read(cx);
        (
            view.measuring(),
            view.report()
                .map(|report| report.objects.loose + report.objects.packed),
            view.task_said(Task::Gc).map(|said| match said {
                Ok(said) => said.clone(),
                Err(err) => format!("failed: {err}"),
            }),
            view.checked()
                .and_then(|checked| checked.as_ref().ok())
                .map(|checked| checked.ok),
        )
    })
}

/// The palette's entry shows the section, which measures the repository
/// the first time; a task runs as an operation and says what it freed,
/// after which the size is measured again; the check says nothing is
/// wrong with a sound repository.
fn the_maintenance_section_measures_runs_and_checks() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::ShowMaintenance);
    assert_eq!(stage.shell().preferences_section, Some("maintenance"));
    stage.until("the repository is measured", |stage| {
        let (measuring, objects, _, _) = upkeep(stage);
        !measuring && objects.is_some()
    });
    let (_, objects, _, _) = upkeep(&mut stage);
    let counted = repos::git(&repo, &["count-objects", "-v"]);
    let git_says: u64 = counted
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            ["count", "in-pack"]
                .contains(&key.trim())
                .then(|| value.trim().parse::<u64>().ok())?
        })
        .sum();
    assert_eq!(objects, Some(git_says));
    let largest = stage.with_workspace(|workspace, _, cx| {
        let view = workspace.read(cx).maintenance_view();
        view.read(cx)
            .report()
            .map(|report| report.largest.len())
            .unwrap_or(0)
    });
    assert!(largest > 0, "the largest files are listed");
    assert!(stage.shows(("maintenance-blob", 0usize)));

    // Collect garbage: the loose objects of the fixture go into a pack.
    stage.click("maintenance-run-gc");
    stage.until("gc said what it freed", |stage| upkeep(stage).2.is_some());
    let (_, _, said, _) = upkeep(&mut stage);
    let said = said.unwrap_or_default();
    assert!(
        said.starts_with("Freed") || said.starts_with("The objects take"),
        "{said}"
    );
    assert!(said.contains("0 loose"), "{said}");
    stage.until("measured again", |stage| !upkeep(stage).0);
    assert!(
        stage
            .notices()
            .iter()
            .any(|notice| notice.contains("loose") && notice.contains("after")),
        "{:?}",
        stage.notices()
    );

    // The check, which is further down than the window reaches: what its
    // button does.
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace.read(cx).maintenance_view();
        view.update(cx, |_, cx| cx.emit(MaintenanceEvent::Run(Job::Check)));
    });
    stage.until("the check is done", |stage| upkeep(stage).3.is_some());
    assert_eq!(upkeep(&mut stage).3, Some(true));
    let running = stage.with_workspace(|workspace, _, cx| {
        workspace
            .read(cx)
            .maintenance_view()
            .read(cx)
            .running()
            .is_some()
    });
    assert!(!running);
    // What nothing is said about is what the view says it found.
    let checked = stage.with_workspace(|workspace, _, cx| {
        workspace
            .read(cx)
            .maintenance_view()
            .read(cx)
            .checked()
            .and_then(|checked| checked.as_ref().ok())
            .map(|checked| checked.said())
    });
    assert!(
        checked
            .as_deref()
            .is_some_and(|said| said.starts_with("Nothing is wrong")),
        "{checked:?}"
    );
}

/// The repository's block in the toolbar lists the maintenance section
/// among the ways to work with the repository.
fn maintenance_is_in_the_repositorys_panel() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.click("crumb-repo");
    assert!(stage.shows(("repo-finder-also", 5usize)));
    stage.click(("repo-finder-also", 5usize));
    assert_eq!(stage.shell().preferences_section, Some("maintenance"));
}

/// The tab's word of how the checkout stands: on, by folder, the rules.
fn sparse(stage: &mut Stage) -> Option<(bool, Vec<String>)> {
    stage.with_workspace(|workspace, _, cx| {
        workspace
            .read(cx)
            .sparse_mark()
            .map(|mark| (mark.enabled, mark.rules.clone()))
    })
}

/// "Only this folder" from a folder's menu checks out that folder and the
/// files at the top; the bar over the changes says so, and its button
/// checks out everything again. A folder is added beside it, too.
fn a_folder_is_checked_out_alone_and_everything_again() {
    repos::fresh();
    let repo = repos::project();
    // Something uncommitted, so that the changes are on show.
    std::fs::write(repo.join("notes.txt"), "to do\n").unwrap();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    stage.until("the checkout is known", |stage| sparse(stage).is_some());
    assert_eq!(sparse(&mut stage), Some((false, Vec::new())));
    assert!(!stage.shows("sparse-bar"));

    // What a folder's menu sends.
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.file_action(
                FileAction::SparseOnly {
                    path: "docs".to_owned(),
                },
                window,
                cx,
            )
        })
    });
    stage.until("only docs is checked out", |stage| {
        sparse(stage) == Some((true, vec!["docs".to_owned()]))
    });
    assert!(repo.join("docs/guide.md").exists());
    assert!(repo.join("README.md").exists());
    assert!(!repo.join("src").exists(), "src is not checked out");
    // The changes panel says so once it is on show.
    stage.until("the bar is there", |stage| stage.shows("sparse-bar"));
    assert_eq!(
        repos::git(&repo, &["sparse-checkout", "list"]).trim(),
        "docs"
    );

    // A folder more.
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.file_action(
                FileAction::SparseAdd {
                    path: "src".to_owned(),
                },
                window,
                cx,
            )
        })
    });
    stage.until("src is checked out too", |stage| {
        sparse(stage) == Some((true, vec!["docs".to_owned(), "src".to_owned()]))
    });
    assert!(repo.join("src/lexer.rs").exists());

    // Everything again, from the bar.
    stage.click("sparse-everything");
    stage.until("every file is checked out", |stage| {
        sparse(stage).is_some_and(|(on, _)| !on)
    });
    assert!(repo.join("src/lexer.rs").exists());
    assert!(!stage.shows("sparse-bar"));
    assert!(
        stage
            .notices()
            .iter()
            .any(|notice| notice.contains("Every file is checked out again")),
        "{:?}",
        stage.notices()
    );
    // And from the palette, where there is nothing to do.
    stage.act(keymap::CheckOutEverything);
    assert!(
        stage
            .notices()
            .iter()
            .any(|notice| notice.contains("checked out here already")),
        "{:?}",
        stage.notices()
    );
}

/// In a clone that left the trees of history on its remote, nothing the
/// window does by itself asks the remote for anything: not the trial merges
/// that mark the local branches that would conflict, with every section
/// open; not the files of the commit that is selected when the repository
/// opens, which say that what they need was left on the remote and fetch it
/// when the user asks; not the size measured when the maintenance section
/// is shown. What the user asks for is fetched, as git would: those files,
/// and the files of another commit that is clicked. Asking by itself, the
/// window would ask the remote some hundred times for a repository of
/// fifty branches, each answer a pack.
fn a_treeless_clone_is_fetched_from_only_when_asked() {
    use std::time::Duration;
    repos::fresh();
    let (repo, log) = repos::treeless_clone();
    let packs = repos::packs(&repo);
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    stage.until("the trial merges are done", |stage| {
        // The pause between two of them goes by the test's clock.
        stage.pass(Duration::from_millis(200));
        let (under_way, answered) = stage.workspace().conflict_marks;
        !under_way && answered > 0
    });
    stage.until("the selected commit's files are answered", |stage| {
        let probe = stage.workspace();
        probe.files_left_on_remote || probe.commit_files > 0
    });
    let probe = stage.workspace();
    assert!(probe.files_left_on_remote, "the details say so");
    assert_eq!(probe.commit_files, 0);
    assert_eq!(repos::remote_asked(&log), 0, "nobody asked for anything");
    assert_eq!(repos::packs(&repo), packs, "no pack came");

    // Asked for: the selected commit's files.
    stage.click("details-fetch-files");
    stage.until("the files are fetched and listed", |stage| {
        stage.workspace().commit_files > 0
    });
    let asked = repos::remote_asked(&log);
    assert!(asked > 0, "the remote was asked");
    // A click on another commit lists its files, fetching what they need.
    let (x, y) = stage.graph_spot(3, false);
    stage.click_at((x, y));
    stage.until("its files are listed", |stage| {
        let probe = stage.workspace();
        assert!(!probe.files_left_on_remote, "a click is asking");
        probe.commit_files > 0
    });
    let asked_again = repos::remote_asked(&log);
    assert!(asked_again > asked, "the remote was asked again");

    // The size, measured when the maintenance section is shown: from what
    // is here.
    stage.act(keymap::ShowMaintenance);
    stage.until("the repository is measured", |stage| {
        let (measuring, objects, _, _) = upkeep(stage);
        !measuring && objects.is_some()
    });
    assert_eq!(
        repos::remote_asked(&log),
        asked_again,
        "measuring asked the remote"
    );
}

/// Git adds packs to a repository while its tab is open: here a commit
/// made from outside, in a pack of its own, beside more packs than the
/// tab's handle has room for. The commit comes into the graph, and its
/// files are listed when it is clicked, not "too few index slots" for as
/// long as the tab stays open.
fn packs_added_while_the_tab_is_open_are_read() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the repository is loaded", |stage| stage.workspace().loaded);
    let commits = stage.graph().commits;
    std::fs::write(repo.join("added.txt"), "made while the tab was open\n").unwrap();
    repos::git(&repo, &["add", "added.txt"]);
    repos::git_at(&repo, 1000, &["commit", "-q", "-m", "made from outside"]);
    repos::git(&repo, &["repack", "-d", "-q"]);
    repos::more_packs(&repo, 1100);
    stage.until("the commit is in the graph", |stage| {
        stage.graph().commits == commits + 1
    });
    let made = repos::rev(&repo, "HEAD");
    if stage.graph().selected.as_deref() != Some(made.as_str()) {
        let (x, y) = stage.graph_spot(0, false);
        stage.click_at((x, y));
    }
    assert_eq!(stage.graph().selected.as_deref(), Some(made.as_str()));
    stage.until("its files are listed", |stage| {
        let probe = stage.workspace();
        assert_eq!(probe.files_error, None);
        probe.commit_files == 1
    });
}

/// The dialog that clones has the choice of a partial clone, beside the
/// depth, and a switch for a sparse checkout.
fn the_clone_dialog_offers_a_partial_clone() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    stage.act(keymap::CloneRepository);
    assert!(stage.dialog_open());
    assert!(stage.shows("form-choose"), "the partial clone's list");
    assert!(
        stage.shows(("form-check", 2usize)),
        "the sparse checkout's switch"
    );
}

/// What the pictures show of a repository's size: numbers of their own,
/// so that the picture does not hang on the file system's block size.
fn measured() -> serde_json::Value {
    serde_json::json!({
        "objects": {
            "loose": 38, "loose_bytes": 155_648, "packed": 12_406, "packs": 2,
            "pack_bytes": 9_646_899, "prune_packable": 0, "garbage": 0, "garbage_bytes": 0,
        },
        "by_type": {
            "commits": { "count": 2_210, "bytes": 1_402_118, "complete": true },
            "trees": { "count": 4_967, "bytes": 2_811_407, "complete": true },
            "blobs": { "count": 5_265, "bytes": 5_587_022, "complete": true },
            "tags": { "count": 2, "bytes": 310, "complete": true },
        },
        "lfs": null,
        "worktrees": [
            { "path": "/home/ada/work/project", "branch": "main", "is_main": true,
              "files": { "count": 412, "bytes": 3_240_118, "complete": true } },
            { "path": "/home/ada/work/project-release", "branch": "release/2.0",
              "is_main": false,
              "files": { "count": 398, "bytes": 3_101_772, "complete": true } },
        ],
        "largest": [
            { "id": "a1", "size": 1_948_160, "disk_size": 1_201_337,
              "path": "assets/demo/screencast.webm",
              "commit": "4c1d2e9f0a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d",
              "summary": "Add the demo screencast", "time": 1_700_000_000 },
            { "id": "b2", "size": 655_360, "disk_size": 402_113,
              "path": "docs/manual.pdf",
              "commit": "9e8f7a6b5c4d4c1d2e9f0a8b7c6d5e4f3a2b1c0d",
              "summary": "Ship the manual", "time": 1_690_000_000 },
            { "id": "c3", "size": 210_944, "disk_size": 48_002,
              "path": "src/generated/tables.rs",
              "commit": "0a8b7c6d5e4f3a2b1c0d9e8f7a6b5c4d4c1d2e9f",
              "summary": "Generate the lookup tables", "time": 1_680_000_000 },
            { "id": "d4", "size": 98_304, "disk_size": 31_877,
              "path": "fixtures/large.json",
              "commit": "7a6b5c4d4c1d2e9f0a8b7c6d5e4f3a2b1c0d9e8f",
              "summary": "Add a large fixture", "time": 1_670_000_000 },
        ],
        "walked": 2_210,
        "walk_complete": true,
    })
}

/// The maintenance section, measured, with what collecting garbage said;
/// and the bar over the changes while only a folder is checked out.
pub fn pictures(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    std::fs::write(repo.join("notes.txt"), "to do\n").unwrap();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    stage.act(keymap::ShowMaintenance);
    assert_eq!(stage.shell().preferences_section, Some("maintenance"));
    stage.until("the repository is measured", |stage| {
        let (measuring, objects, _, _) = upkeep(stage);
        !measuring && objects.is_some()
    });
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace.read(cx).maintenance_view();
        view.update(cx, |view, cx| {
            view.finished(
                Job::Task(Task::Gc),
                Ok(serde_json::json!({
                    "before": { "loose": 38, "packs": 2 },
                    "after": { "loose": 0, "packs": 1 },
                    "freed": 1_284_096,
                })),
                cx,
            )
        });
    });
    stage.until("measured again", |stage| !upkeep(stage).0);
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace.read(cx).maintenance_view();
        view.update(cx, |view, cx| view.take_report(&measured(), cx));
    });
    stage.frame();
    assert!(upkeep(&mut stage).2.is_some());
    shots.take(&mut stage, "maintenance", Themes::Two);
    stage.click("preferences-close");

    stage.until("the row of uncommitted work is selected", |stage| {
        stage.graph().wip_selected
    });
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.file_action(
                FileAction::SparseOnly {
                    path: "docs".to_owned(),
                },
                window,
                cx,
            )
        })
    });
    stage.until("the bar is there", |stage| stage.shows("sparse-bar"));
    shots.take(&mut stage, "sparse_checkout", Themes::Two);
}
