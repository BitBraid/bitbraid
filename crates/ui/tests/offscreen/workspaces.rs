//! Workspaces on the page of repositories and on the start page: what
//! making one does, ticking rows, the bar over a section, a command on many
//! repositories with one that fails, putting sections in order, and the
//! pictures of each.
//!
//! The repositories are made here with git: a few beside each other in
//! a folder, each with a bare remote of its own: one as its remote has it,
//! one ahead of it, one with uncommitted work. The workspaces' file is
//! written into the scratch configuration directory, as the engine keeps
//! it.

use std::path::{Path, PathBuf};

use bitbraid_engine::workspace::{RepositoryOutcome, RepositoryState};
use bitbraid_ui::views::repositories::{Making, RepositoriesView, bulk::Kind, candidates};
use gpui_kit::Entity;

use crate::{
    repos,
    runner::{Shots, Test, Themes},
    stage::{Stage, Start},
};

/// The tests.
pub const TESTS: &[Test] = &[
    Test {
        name: "a_workspace_is_made_from_a_folder_looked_through",
        run: a_workspace_is_made_from_a_folder_looked_through,
    },
    Test {
        name: "ticking_rows_puts_a_bar_over_the_section_that_names_the_count",
        run: ticking_rows_puts_a_bar_over_the_section_that_names_the_count,
    },
    Test {
        name: "a_fetch_of_many_says_on_each_row_how_it_went_and_a_failure_stays",
        run: a_fetch_of_many_says_on_each_row_how_it_went_and_a_failure_stays,
    },
    Test {
        name: "sections_are_put_in_order_by_their_headings_and_hidden",
        run: sections_are_put_in_order_by_their_headings_and_hidden,
    },
    Test {
        name: "a_column_is_dragged_wider_and_keeps_its_width",
        run: a_column_is_dragged_wider_and_keeps_its_width,
    },
    Test {
        name: "the_start_page_lists_the_workspaces_and_leads_to_one",
        run: the_start_page_lists_the_workspaces_and_leads_to_one,
    },
    Test {
        name: "a_workspace_s_repositories_open_with_its_layout",
        run: a_workspace_s_repositories_open_with_its_layout,
    },
    Test {
        name: "a_workspace_is_written_to_a_file_and_taken_in_again",
        run: a_workspace_is_written_to_a_file_and_taken_in_again,
    },
    Test {
        name: "a_workspace_is_deleted_after_a_question_and_its_folders_stay",
        run: a_workspace_is_deleted_after_a_question_and_its_folders_stay,
    },
    Test {
        name: "a_repository_that_moved_is_located",
        run: a_repository_that_moved_is_located,
    },
    Test {
        name: "a_workspace_made_from_the_palette_is_on_the_pages_at_once",
        run: a_workspace_made_from_the_palette_is_on_the_pages_at_once,
    },
    Test {
        name: "a_workspace_s_row_says_when_its_repository_was_last_fetched",
        run: a_workspace_s_row_says_when_its_repository_was_last_fetched,
    },
];

/// Where the pointer rests while a picture is taken: on the status bar.
const AWAY: (f32, f32) = (640., 712.);

/// The folder the team's repositories are in.
fn team_folder() -> PathBuf {
    repos::work().join("team")
}

/// One repository of the team with a bare remote of its own, its branch
/// following the remote's.
fn member(name: &str) -> PathBuf {
    let folder = team_folder();
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join(name);
    std::fs::create_dir_all(&path).unwrap();
    repos::git(&path, &["init", "-q", "-b", "main"]);
    std::fs::write(path.join("README.md"), format!("# {name}\n")).unwrap();
    repos::git(&path, &["add", "-A"]);
    repos::git(&path, &["commit", "-q", "-m", "start"]);
    let remote = repos::work().join("remotes").join(format!("{name}.git"));
    std::fs::create_dir_all(remote.parent().unwrap()).unwrap();
    repos::git(
        &path,
        &["clone", "-q", "--bare", ".", remote.to_str().unwrap()],
    );
    repos::git(
        &path,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    repos::git(&path, &["fetch", "-q", "origin"]);
    repos::git(&path, &["branch", "-q", "-u", "origin/main", "main"]);
    // Fetched two hours before the clock the window is held at, so that
    // its row says the same on every run.
    repos::fetched_at(&path, crate::hermetic::NOW - 2 * 3600);
    path
}

/// The team: alpha as its remote has it, beta a commit ahead, gamma with
/// uncommitted work. Their paths, in that order.
fn team() -> Vec<PathBuf> {
    let alpha = member("alpha");
    let beta = member("beta");
    std::fs::write(beta.join("notes.txt"), "two\n").unwrap();
    repos::git(&beta, &["add", "-A"]);
    repos::git(&beta, &["commit", "-q", "-m", "notes"]);
    let gamma = member("gamma");
    std::fs::write(gamma.join("README.md"), "# gamma, changed\n").unwrap();
    std::fs::write(gamma.join("new.txt"), "new\n").unwrap();
    vec![alpha, beta, gamma]
}

/// The scratch configuration directory of the application.
fn config() -> PathBuf {
    crate::hermetic::inside().unwrap().join("config/bitbraid")
}

/// A workspace as the file the engine reads has it: its id, name, colour
/// and repositories, each with the remote it is cloned from.
fn workspace_text(
    id: &str,
    name: &str,
    colour: &str,
    members: &[(PathBuf, Option<&str>)],
) -> String {
    let mut text = format!(
        "\n[[workspaces]]\nid = {id:?}\nname = {name:?}\ncolour = {colour:?}\nicon = \"code-xml\"\n\
         description = \"The tools of the team\"\n"
    );
    for (path, remote) in members {
        text.push_str(&format!(
            "\n[[workspaces.repositories]]\npath = {:?}\n",
            path.display().to_string()
        ));
        if let Some(remote) = remote {
            text.push_str(&format!("remote_url = {remote:?}\n"));
        }
    }
    text
}

/// The file the engine keeps the workspaces in, with these.
fn workspaces_file(workspaces: &[String]) -> String {
    format!("version = 1\n{}", workspaces.concat())
}

/// The team as a workspace, with a fourth repository that is not there
/// and can be cloned.
fn team_text(paths: &[PathBuf]) -> String {
    let mut members: Vec<(PathBuf, Option<&str>)> =
        paths.iter().map(|path| (path.clone(), None)).collect();
    members.push((
        team_folder().join("delta"),
        Some("https://example.invalid/team/delta.git"),
    ));
    workspace_text("team", "Team", "#3aa6a0", &members)
}

/// Started on the first of the team, with the team as a workspace.
fn with_team(paths: &[PathBuf]) -> Start {
    Start::with(&paths[0]).config_file("workspaces.toml", workspaces_file(&[team_text(paths)]))
}

/// Click the box of a row of the team, where it is drawn: 41 in from the
/// row's left edge (its middle 8 further), halfway down the row.
fn tick(stage: &mut Stage, name: &str) {
    let (x, y, _, h) = stage.place(format!("repo-ws-team-{name}"));
    stage.click_at((x + 41. + 8., y + h / 2.));
}

/// The page of repositories, once it was shown.
fn page(stage: &mut Stage) -> Entity<RepositoriesView> {
    stage.with_shell(|shell, _, _| shell.repositories_view().expect("the page was shown"))
}

/// Show the page with only the workspace's section unfolded, so that its
/// rows are in sight in a window this low, and wait until git was asked
/// about the team.
fn show_page(stage: &mut Stage) {
    stage.click("repositories-tab");
    assert_eq!(stage.shell().sheet, Some("repositories"));
    stage.until("the workspace is on the page", |stage| {
        stage.shows("part-workspace:team")
    });
    stage.click("repositories-collapse");
    stage.click("part-workspace:team");
    stage.until("the team's repositories are looked at", |stage| {
        stage.shows("branch-ws-team-alpha") && stage.shows("changes-ws-team-gamma")
    });
}

fn a_workspace_is_made_from_a_folder_looked_through() {
    repos::fresh();
    let paths = team();
    let mut stage = Stage::open(Start::with(&paths[0]));
    stage.click("repositories-tab");
    stage.click("repositories-new-workspace");
    assert!(stage.dialog_open(), "the dialog that makes a workspace");
    let page = page(&mut stage);
    let form = stage.app(|cx| page.read(cx).form().expect("the dialog is the page's"));
    assert_eq!(stage.app(|cx| form.read(cx).making().clone()), Making::New);
    // It waits for a name and for repositories.
    assert_eq!(stage.app(|cx| form.read(cx).waits_for(cx)), Some("a name"));
    stage.type_text("Team");
    assert_eq!(
        stage.app(|cx| form.read(cx).waits_for(cx)),
        Some("a repository")
    );
    // A folder looked through: the three repositories, and the remotes'
    // folder is no repository of the team's.
    let folder = team_folder();
    stage.window(|window, cx| form.update(cx, |form, cx| form.scan(&folder, window, cx)));
    stage.until("the folder was looked through", |stage| {
        stage.app(|cx| form.read(cx).found().len()) == 3
    });
    assert!(stage.shows("workspace-count"));
    stage.app(|cx| form.update(cx, |form, cx| form.set_follow(true, cx)));
    stage.app(|cx| form.update(cx, |form, cx| form.set_colour(Some("#9563d6"), cx)));
    assert_eq!(stage.app(|cx| form.read(cx).waits_for(cx)), None);
    stage.click("workspace-form-go");
    stage.until("the workspace is on the page", |stage| {
        stage.shows("part-workspace:team")
    });
    assert!(!stage.dialog_open(), "the dialog went away");
    let kept = std::fs::read_to_string(config().join("workspaces.toml")).unwrap();
    assert!(kept.contains("name = \"Team\""), "{kept}");
    assert!(kept.contains("#9563d6"), "{kept}");
    assert!(kept.contains("follows_folder"), "{kept}");
    stage.click("repositories-collapse");
    stage.click("part-workspace:team");
    stage.until("its rows are looked at", |stage| {
        stage.shows("branch-ws-team-alpha") && stage.shows("ahead-ws-team-beta")
    });
    // A name that is taken does not go ahead.
    stage.click("repositories-new-workspace");
    let form = stage.app(|cx| page.read(cx).form().unwrap());
    stage.type_text("team");
    stage.app(|cx| form.update(cx, |form, cx| form.take_found(candidates(&paths[..1]), cx)));
    assert_eq!(
        stage.app(|cx| form.read(cx).waits_for(cx)),
        Some("a name no other workspace has")
    );
}

fn ticking_rows_puts_a_bar_over_the_section_that_names_the_count() {
    repos::fresh();
    let paths = team();
    let mut stage = Stage::open(with_team(&paths));
    show_page(&mut stage);
    // With nothing ticked, the heading's buttons are icons for all of them.
    assert!(stage.shows("add-team") && stage.shows("fetch-team"));
    assert!(!stage.shows("remove-team"));
    tick(&mut stage, "alpha");
    tick(&mut stage, "beta");
    let page = page(&mut stage);
    assert_eq!(stage.app(|cx| page.read(cx).ticked_in("team").len()), 2);
    assert!(stage.shows("remove-team") && stage.shows("untick-team"));
    assert!(!stage.shows("add-team"), "the bar takes the icons' place");
    assert!(!stage.shows("clone-team"), "nothing missing is ticked");
    // The missing one ticked: it can be located and cloned.
    tick(&mut stage, "delta");
    assert!(stage.shows("locate-team") && stage.shows("clone-team"));
    // Untick them all: the icons are back.
    stage.click("untick-team");
    assert!(stage.shows("add-team"));
    assert!(stage.app(|cx| page.read(cx).ticked_in("team").is_empty()));
    // Removing two from the workspace leaves their folders.
    tick(&mut stage, "alpha");
    tick(&mut stage, "gamma");
    stage.click("remove-team");
    stage.until("they are out of the workspace", |stage| {
        !stage.shows("repo-ws-team-alpha") && !stage.shows("repo-ws-team-gamma")
    });
    assert!(paths[0].join(".git").is_dir() && paths[2].join(".git").is_dir());
    assert!(stage.shows("repo-ws-team-beta"));
}

fn a_fetch_of_many_says_on_each_row_how_it_went_and_a_failure_stays() {
    repos::fresh();
    let paths = team();
    // Beta's remote is gone: its fetch fails, the others' do not.
    std::fs::remove_dir_all(repos::work().join("remotes/beta.git")).unwrap();
    let mut stage = Stage::open(with_team(&paths));
    show_page(&mut stage);
    let page = page(&mut stage);
    stage.click("fetch-team");
    stage.until("every repository was fetched or failed", |stage| {
        stage.app(|cx| !page.read(cx).is_running("team"))
            && stage.app(|cx| {
                page.read(cx)
                    .outcome("team", &paths[1].display().to_string())
                    .is_some()
            })
    });
    let state = |stage: &mut Stage, path: &Path| {
        stage.app(|cx| {
            page.read(cx)
                .outcome("team", &path.display().to_string())
                .map(|outcome| outcome.state)
        })
    };
    assert_eq!(state(&mut stage, &paths[0]), Some(RepositoryState::Done));
    assert_eq!(state(&mut stage, &paths[1]), Some(RepositoryState::Failed));
    assert_eq!(state(&mut stage, &paths[2]), Some(RepositoryState::Done));
    assert!(stage.shows("state-ws-team-beta") && stage.shows("dismiss-ws-team-beta"));
    let error = stage.app(|cx| {
        page.read(cx)
            .outcome("team", &paths[1].display().to_string())
            .and_then(|outcome| outcome.error.clone())
    });
    assert!(
        error.is_some_and(|error| !error.is_empty()),
        "the failure says why"
    );
    // Shown again, the page forgets what went well and keeps the failure.
    stage.click(("tab-name", 0usize));
    show_page(&mut stage);
    assert_eq!(state(&mut stage, &paths[0]), None);
    assert_eq!(state(&mut stage, &paths[1]), Some(RepositoryState::Failed));
    stage.click("dismiss-ws-team-beta");
    assert_eq!(state(&mut stage, &paths[1]), None);
    assert!(!stage.shows("dismiss-ws-team-beta"));
}

fn sections_are_put_in_order_by_their_headings_and_hidden() {
    repos::fresh();
    let paths = team();
    let mut stage = Stage::open(with_team(&paths));
    show_page(&mut stage);
    let page = page(&mut stage);
    let order = |stage: &mut Stage| -> Vec<String> {
        stage.app(|cx| {
            page.read(cx)
                .shown_sections(cx)
                .iter()
                .map(|section| section.key())
                .collect()
        })
    };
    assert_eq!(
        order(&mut stage),
        ["open", "favourites", "recent", "all", "workspace:team"]
    );
    // The workspace's heading dragged onto the first: it goes before it.
    let (x, y, w, h) = stage.place("part-workspace:team");
    let from = (x + w / 2., y + h / 2.);
    let (x, y, w, h) = stage.place("part-open");
    stage.drag_at(from, (x + w / 2., y + h / 2.));
    assert_eq!(
        order(&mut stage),
        ["workspace:team", "open", "favourites", "recent", "all"]
    );
    let kept = std::fs::read_to_string(config().join("state.toml")).unwrap();
    assert!(kept.contains("workspace:team"), "the order is kept: {kept}");
    // Hidden from its menu, and shown again from the button that counts
    // the hidden ones.
    assert!(!stage.shows("repositories-hidden"));
    stage.app(|cx| {
        page.update(cx, |page, cx| {
            page.hide_section(
                &bitbraid_ui::views::repositories::Section::Part(
                    bitbraid_ui::views::repositories::Part::All,
                ),
                true,
                cx,
            )
        })
    });
    stage.frame();
    assert!(!stage.shows("part-all"));
    assert!(stage.shows("repositories-hidden"));
    assert_eq!(
        order(&mut stage),
        ["workspace:team", "open", "favourites", "recent"]
    );
    stage.app(|cx| {
        page.update(cx, |page, cx| {
            page.hide_section(
                &bitbraid_ui::views::repositories::Section::Part(
                    bitbraid_ui::views::repositories::Part::All,
                ),
                false,
                cx,
            )
        })
    });
    stage.frame();
    assert!(stage.shows("part-all") && !stage.shows("repositories-hidden"));
    // Folding a workspace's section hides its rows.
    stage.click("part-workspace:team");
    assert!(!stage.shows("repo-ws-team-alpha"));
    stage.click("part-workspace:team");
    assert!(stage.shows("repo-ws-team-alpha"));
}

fn a_column_is_dragged_wider_and_keeps_its_width() {
    repos::fresh();
    let paths = team();
    let mut stage = Stage::open(with_team(&paths));
    show_page(&mut stage);
    let (x, y, w, h) = stage.place("grip-name-ws-team-alpha");
    let (name_x, _, name_w, _) = stage.place("name-ws-team-alpha");
    stage.drag_at((x + w / 2., y + h / 2.), (x + w / 2. + 60., y + h / 2.));
    let (_, _, wider, _) = stage.place("name-ws-team-alpha");
    assert!(
        (wider - name_w - 60.).abs() < 2.,
        "the names are 60 wider: {name_w} then {wider}"
    );
    let (moved_x, _, _, _) = stage.place("name-ws-team-alpha");
    assert!(
        (moved_x - name_x).abs() < 0.5,
        "the names begin where they did"
    );
    let kept = std::fs::read_to_string(config().join("state.toml")).unwrap();
    assert!(kept.contains("name_width"), "the width is kept: {kept}");
}

fn the_start_page_lists_the_workspaces_and_leads_to_one() {
    repos::fresh();
    let paths = team();
    let file = workspaces_file(&[team_text(&paths)]);
    let mut stage = Stage::open(Start::bare().config_file("workspaces.toml", file));
    stage.until("the start page lists the workspace", |stage| {
        stage.shows("start-workspace-team")
    });
    stage.click("start-workspace-team");
    assert_eq!(stage.shell().sheet, Some("repositories"));
    stage.until("its section is on the page", |stage| {
        stage.shows("part-workspace:team") && stage.shows("repo-ws-team-beta")
    });
}

fn a_workspace_s_repositories_open_with_its_layout() {
    repos::fresh();
    let paths = team();
    let text = workspaces_file(&[team_text(&paths)]).replace(
            "description = \"The tools of the team\"\n",
            "description = \"The tools of the team\"\n\n[workspaces.layout]\nleft_width = 333.0\nhidden_sections = [\"tags\"]\n",
        );
    let mut stage = Stage::open(Start::with(&paths[0]).config_file("workspaces.toml", text));
    show_page(&mut stage);
    // Beta was never opened: it takes the workspace's layout.
    stage.click("open-ws-team-beta");
    stage.until("beta is open in a tab", |stage| {
        stage.shell().tabs.len() == 2
    });
    let kept = stage.app(|cx| {
        cx.global::<bitbraid_ui::prefs::Prefs>()
            .state
            .repos
            .get(&paths[1].display().to_string())
            .cloned()
    });
    let kept = kept.expect("beta has a layout");
    assert_eq!(kept.left, Some(333.));
    assert_eq!(kept.hidden_sections, ["tags"]);
}

fn a_workspace_is_written_to_a_file_and_taken_in_again() {
    repos::fresh();
    let paths = team();
    let mut stage = Stage::open(with_team(&paths));
    show_page(&mut stage);
    let page = page(&mut stage);
    // Written beside the team's repositories, as it would be kept in a
    // repository of its own.
    let file = team_folder().join("team.workspace.toml");
    stage
        .window(|window, cx| page.update(cx, |page, cx| page.export_to("team", &file, window, cx)));
    stage.until("the file is written", |_| file.is_file());
    let written = std::fs::read_to_string(&file).unwrap();
    assert!(
        written.contains("Team") && written.contains("alpha"),
        "{written}"
    );
    assert!(
        !written.contains(&team_folder().display().to_string()),
        "the paths are relative to the file: {written}"
    );
    // Deleted here, then taken in from the file.
    stage
        .window(|window, cx| page.update(cx, |page, cx| page.delete_workspace("team", window, cx)));
    stage.until("the workspace is gone", |stage| {
        !stage.shows("part-workspace:team")
    });
    stage.window(|window, cx| page.update(cx, |page, cx| page.import_from(&file, window, cx)));
    stage.until("it is back, from the file", |stage| {
        stage.app(|cx| page.read(cx).workspaces().len()) == 1
    });
    let back = stage.app(|cx| page.read(cx).workspaces()[0].workspace.clone());
    assert_eq!(back.name, "Team");
    let kept: Vec<String> = back
        .repositories
        .iter()
        .map(|one| one.path.clone())
        .collect();
    for path in &paths {
        assert!(kept.contains(&path.display().to_string()), "{kept:?}");
    }
}

fn a_workspace_is_deleted_after_a_question_and_its_folders_stay() {
    repos::fresh();
    let paths = team();
    let mut stage = Stage::open(with_team(&paths));
    show_page(&mut stage);
    // What the last entry of the heading's menu does: asked in the bar,
    // which says the folders stay.
    let page = page(&mut stage);
    stage.window(|window, cx| {
        page.update(cx, |page, cx| page.ask_delete_workspace("team", window, cx))
    });
    let asked = stage.app(|cx| bitbraid_ui::ask::on_show(cx));
    let (said, _) = asked.expect("the bar asks");
    assert!(said.contains("Team") && said.contains("stay"), "{said}");
    stage.click(("ask-choice", 0usize));
    stage.until("the workspace is gone", |stage| {
        !stage.shows("part-workspace:team")
    });
    for path in &paths {
        assert!(path.join(".git").is_dir(), "{} stays", path.display());
    }
    let kept = std::fs::read_to_string(config().join("workspaces.toml")).unwrap();
    assert!(!kept.contains("name = \"Team\""), "{kept}");
}

fn a_repository_that_moved_is_located() {
    repos::fresh();
    let paths = team();
    let mut stage = Stage::open(with_team(&paths));
    show_page(&mut stage);
    let page = page(&mut stage);
    let moved = team_folder().join("gamma-moved");
    std::fs::rename(&paths[2], &moved).unwrap();
    // Shown again, the page sees that it is not there.
    stage.click(("tab-name", 0usize));
    stage.click("repositories-tab");
    stage.until("it is not found", |stage| {
        stage.shows("problem-ws-team-gamma")
    });
    let from = paths[2].display().to_string();
    stage.window(|window, cx| {
        page.update(cx, |page, cx| {
            page.locate_at("team", &from, &moved, window, cx)
        })
    });
    stage.until("it is found where it is now", |stage| {
        stage.shows("branch-ws-team-gamma-moved")
    });
    let kept = stage.app(|cx| page.read(cx).workspaces()[0].workspace.clone());
    assert!(
        kept.repositories
            .iter()
            .any(|one| one.path == moved.display().to_string())
    );
}

/// A workspace's row says when its repository was last fetched, and says
/// nothing of a repository that never was.
fn a_workspace_s_row_says_when_its_repository_was_last_fetched() {
    repos::fresh();
    let paths = team();
    std::fs::remove_file(paths[2].join(".git/FETCH_HEAD")).unwrap();
    let mut stage = Stage::open(with_team(&paths));
    show_page(&mut stage);
    assert!(
        stage.shows("fetched-ws-team-alpha"),
        "alpha says when it was fetched"
    );
    assert!(stage.shows("fetched-ws-team-beta"));
    assert!(!stage.shows("fetched-ws-team-gamma"), "gamma never was");
}

/// Run `workspace.create` from the palette, naming the workspace `name`
/// and nothing else.
fn create_from_the_palette(stage: &mut Stage, name: &str) {
    stage.act(bitbraid_ui::keymap::TogglePalette);
    stage.type_text("workspace.create");
    stage.keys("enter");
    assert_eq!(stage.shell().asking_for, Some("workspace.create"));
    stage.type_text(name);
    stage.click("param-ok");
    stage.settle();
}

/// A workspace made by its command from the palette is on the page of
/// repositories at once, with the page on show over a repository's tab,
/// and on the start page of a tab without a repository, without the page
/// being left and shown again.
fn a_workspace_made_from_the_palette_is_on_the_pages_at_once() {
    repos::fresh();
    let paths = team();
    let mut stage = Stage::open(with_team(&paths));
    show_page(&mut stage);
    let page = page(&mut stage);
    create_from_the_palette(&mut stage, "made");
    stage.until("it is on the page", |stage| {
        stage.settle();
        stage.app(|cx| {
            page.read(cx)
                .workspaces()
                .iter()
                .any(|listed| listed.workspace.name == "made")
        })
    });
    stage.until("its section is on show", |stage| {
        stage.shows("part-workspace:made")
    });

    // On the start page, where no repository is open.
    let file = workspaces_file(&[team_text(&paths)]);
    let mut stage = Stage::open(Start::bare().config_file("workspaces.toml", file));
    stage.until("the start page lists the workspace", |stage| {
        stage.shows("start-workspace-team")
    });
    create_from_the_palette(&mut stage, "fresh");
    stage.until("the start page lists the new one", |stage| {
        stage.settle();
        stage.shows("start-workspace-fresh")
    });
}

/// The pictures of the page with a workspace: its heading in its colour,
/// rows with their remotes, one that is missing; rows ticked, a fetch
/// under way and one that failed; the dialog that makes one and edits one.
pub fn pictures(shots: &mut Shots) {
    repos::fresh();
    let paths = team();
    let remotes = paths
        .iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_str().unwrap();
            (
                path.clone(),
                Some(format!("https://example.invalid/team/{name}.git")),
            )
        })
        .collect::<Vec<_>>();
    let mut members: Vec<(PathBuf, Option<&str>)> = remotes
        .iter()
        .map(|(path, remote)| (path.clone(), remote.as_deref()))
        .collect();
    members.push((
        team_folder().join("delta"),
        Some("https://example.invalid/team/delta.git"),
    ));
    let file = workspaces_file(&[workspace_text("team", "Team", "#3aa6a0", &members)]);
    let mut stage = Stage::open(
        Start::with(&paths[0])
            .config_file("workspaces.toml", file)
            .pictures(),
    );
    show_page(&mut stage);
    let page = page(&mut stage);
    // Only the workspace unfolded, as `show_page` leaves it.
    shots.take(&mut stage, "workspaces", Themes::Two);

    // Two ticked: the bar names the count.
    tick(&mut stage, "alpha");
    tick(&mut stage, "delta");
    // The pointer away from the rows and the heading, which it tints.
    stage.point_at(AWAY);
    shots.take(&mut stage, "workspaces_ticked", Themes::Two);
    stage.click("untick-team");
    stage.point_at(AWAY);

    // A fetch under way: one done, one fetching, one waiting; then one
    // that failed and says why.
    let at = |path: &Path, state: RepositoryState, said: Option<&str>| {
        let mut outcome = RepositoryOutcome::queued(&path.display().to_string());
        outcome.state = state;
        match state {
            RepositoryState::Failed => outcome.error = said.map(str::to_owned),
            _ => outcome.progress = said.map(str::to_owned),
        }
        if state == RepositoryState::Running {
            outcome.percent = Some(42);
        }
        outcome
    };
    let running = vec![
        at(&paths[0], RepositoryState::Done, None),
        at(
            &paths[1],
            RepositoryState::Running,
            Some("Receiving objects"),
        ),
        at(&paths[2], RepositoryState::Queued, None),
    ];
    stage.app(|cx| {
        page.update(cx, |page, cx| {
            page.show_outcomes("team", Kind::Fetch, running, true, cx)
        })
    });
    shots.take(&mut stage, "workspaces_running", Themes::Two);
    let failed = vec![at(
        &paths[1],
        RepositoryState::Failed,
        Some("could not read from the remote: the address does not answer"),
    )];
    stage.app(|cx| {
        page.update(cx, |page, cx| {
            page.show_outcomes("team", Kind::Fetch, failed, false, cx)
        })
    });
    shots.take(&mut stage, "workspaces_failed", Themes::Two);

    // The dialog that makes one, with what a folder gave.
    stage.click("repositories-new-workspace");
    let form = stage.app(|cx| page.read(cx).form().unwrap());
    stage.type_text("Tools");
    let folder = team_folder();
    stage.window(|window, cx| form.update(cx, |form, cx| form.scan(&folder, window, cx)));
    stage.until("the folder was looked through", |stage| {
        stage.app(|cx| form.read(cx).found().len()) == 3
    });
    stage.app(|cx| form.update(cx, |form, cx| form.set_colour(Some("#d9a34f"), cx)));
    shots.take(&mut stage, "workspace_new", Themes::Two);
    stage.keys("escape");

    // Editing one.
    stage.window(|window, cx| {
        page.update(cx, |page, cx| {
            page.open_form(Making::Edit("team".to_owned()), window, cx)
        })
    });
    stage.frame();
    shots.take(&mut stage, "workspace_edit", Themes::Two);
}

/// The start page with the workspaces under the recent repositories.
pub fn start_pictures(shots: &mut Shots) {
    repos::fresh();
    let paths = team();
    let file = workspaces_file(&[
        team_text(&paths),
        workspace_text(
            "docs",
            "Documentation",
            "#d9a34f",
            &[(paths[0].clone(), None)],
        ),
    ]);
    let state = format!("recent = [{:?}, {:?}]\n", paths[0], paths[1]);
    let mut stage = Stage::open(
        Start::bare()
            .state(state)
            .config_file("workspaces.toml", file)
            .pictures(),
    );
    stage.until("the start page lists the workspaces", |stage| {
        stage.shows("start-workspace-team") && stage.shows("start-workspace-docs")
    });
    shots.take(&mut stage, "start_workspaces", Themes::Two);
}
