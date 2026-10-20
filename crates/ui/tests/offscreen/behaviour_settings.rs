//! What the window does, as `behaviour.rs` asks it, for the settings page
//! and profiles.

use std::time::Duration;

use bitbraid_ui::keymap;

use crate::{
    repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "the_preferences_are_a_page_of_sections",
        run: the_preferences_are_a_page_of_sections,
    },
    Test {
        name: "there_is_always_a_current_profile",
        run: there_is_always_a_current_profile,
    },
    Test {
        name: "a_renamed_profile_keeps_the_tabs_kept_under_it",
        run: a_renamed_profile_keeps_the_tabs_kept_under_it,
    },
    Test {
        name: "the_current_profiles_box_is_filled_by_its_choice",
        run: the_current_profiles_box_is_filled_by_its_choice,
    },
    Test {
        name: "the_current_profile_connects_with_a_key_file_or_the_agent",
        run: the_current_profile_connects_with_a_key_file_or_the_agent,
    },
    Test {
        name: "a_tool_is_chosen_from_a_list_or_typed",
        run: a_tool_is_chosen_from_a_list_or_typed,
    },
    Test {
        name: "a_repositorys_sections_follow_its_tab",
        run: a_repositorys_sections_follow_its_tab,
    },
    Test {
        name: "the_repositories_tab_shows_every_repository_known",
        run: the_repositories_tab_shows_every_repository_known,
    },
    Test {
        name: "what_the_panel_adds_is_asked_in_a_pane_beside_it",
        run: what_the_panel_adds_is_asked_in_a_pane_beside_it,
    },
];

/// The preferences take everything under the tabs, and every section of
/// the column can be shown, those whose rows read the view they are drawn
/// in (the tools found, the terminals found, the git that is run)
/// included.
fn the_preferences_are_a_page_of_sections() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    assert!(
        stage.shows("pull-more"),
        "the toolbar is there with the graph"
    );
    stage.act(keymap::ShowPreferences);
    assert_eq!(stage.shell().sheet, Some("preferences"));
    assert_eq!(stage.shell().preferences_section, Some("general"));
    assert!(
        !stage.shows("pull-more"),
        "no toolbar under the preferences"
    );
    for (section, shows) in [
        ("integrations", "integrations-site-github"),
        ("tools", "select-External merge tool"),
        ("interface", "check-Dates in full"),
        ("editor", "check-Wrap long lines"),
        ("experimental", "git-use"),
        ("keys", "keys-sheet"),
        ("files", "section-files"),
        ("general", "check-Prune when fetching"),
    ] {
        stage.click(format!("section-{section}"));
        assert_eq!(stage.shell().preferences_section, Some(section));
        assert!(stage.shows(shows), "{section} shows {shows}");
    }
    // The way out is the column's first row.
    stage.click("preferences-close");
    assert_eq!(stage.shell().sheet, None);
    assert!(stage.shows("pull-more"), "the toolbar is back");
}

/// There is always a current profile: the first one is made from git's
/// identity. A new profile is made in a dialog and is the current one at
/// once; a name that is taken is said before anything is saved, and what
/// was typed stays.
fn there_is_always_a_current_profile() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("a profile was made from git's identity", |stage| {
        stage.shell().profile.as_deref() == Some("Default")
    });
    // From anywhere, the preferences open included: opened in the tab
    // under the preferences, nothing would be seen of the profiles.
    stage.act(keymap::ShowPreferences);
    stage.act(keymap::ShowProfiles);
    assert_eq!(stage.shell().preferences_section, Some("profiles"));
    stage.click("profile-add");
    assert!(stage.dialog_open());
    stage.type_text("default");
    assert!(stage.shows("profile-save"));
    stage.click("profile-save");
    assert!(stage.dialog_open(), "a name that is taken is not saved");
    stage.keys("backspace backspace backspace backspace backspace backspace backspace");
    stage.type_text("Work");
    stage.click("profile-save");
    stage.until("the new profile is the current one", |stage| {
        stage.shell().profile.as_deref() == Some("Work")
    });
    assert!(!stage.dialog_open());
    assert_eq!(stage.shell().profiles, ["Default", "Work"]);
}

/// The tabs that were open under a profile are kept by its name; a profile
/// that is given another name in its dialog keeps them, and switching to it
/// brings them back.
fn a_renamed_profile_keeps_the_tabs_kept_under_it() {
    repos::fresh();
    let repo = repos::project();
    let other = repos::fixture("linear", "other");
    let profiles = "[[profiles]]\nname = \"Default\"\nuser_name = \"Ui Scratch\"\n\
                    user_email = \"ui@scratch.invalid\"\n\n[[profiles]]\nname = \"Alternate\"\n\
                    user_name = \"Alternate Tester\"\nuser_email = \"alternate@example.invalid\"\n";
    let state = format!(
        "open = [{repo:?}]\nactive = 0\nprofile = \"Default\"\n\n\
         [profile_tabs.Alternate]\nopen = [{repo:?}, {other:?}]\nactive = 1\n"
    );
    let mut stage = Stage::open(
        Start::bare()
            .state(state)
            .config_file("profiles.toml", profiles),
    );
    stage.until("the profiles are read", |stage| {
        stage.shell().profiles.len() > 1
    });
    assert_eq!(stage.shell().profiles, ["Alternate", "Default"]);
    assert_eq!(stage.shell().profile.as_deref(), Some("Default"));
    let kept = |stage: &mut Stage, name: &str| {
        stage.app(|cx| {
            cx.global::<bitbraid_ui::prefs::Prefs>()
                .state
                .profile_tabs
                .get(name)
                .map(|tabs| tabs.open.clone())
        })
    };
    assert_eq!(
        kept(&mut stage, "Alternate"),
        Some(vec![repo.clone(), other.clone()])
    );

    // Alternate, which is not current, is given another name.
    stage.act(keymap::ShowPreferences);
    stage.act(keymap::ShowProfiles);
    stage.click(("profile-menu", 0usize));
    assert!(stage.shows("popup-menu"));
    stage.keys("down down down enter");
    assert!(stage.dialog_open() && stage.shows("profile-save"));
    stage.click("profile-name");
    stage.keys("ctrl-a");
    stage.type_text("QA alternate");
    stage.click("profile-save");
    stage.until("the profile has its new name", |stage| {
        stage.shell().profiles == ["Default", "QA alternate"]
    });
    assert_eq!(kept(&mut stage, "Alternate"), None);
    assert_eq!(kept(&mut stage, "QA alternate"), Some(vec![repo, other]));
}

/// With several profiles the current one is a choice that fills its box:
/// the picture as far from the top as from the bottom, and the line before
/// the arrow from the box's top to its bottom, not the toolkit's height
/// for a button at the top of the box.
fn the_current_profiles_box_is_filled_by_its_choice() {
    repos::fresh();
    let profiles = "[[profiles]]\nname = \"Default\"\nuser_name = \"Ui Scratch\"\n\
                    user_email = \"ui@scratch.invalid\"\n\n[[profiles]]\nname = \"Alternate\"\n\
                    user_name = \"Alternate Tester\"\nuser_email = \"alternate@example.invalid\"\n";
    let mut stage = Stage::open(Start::bare().config_file("profiles.toml", profiles));
    stage.until("the profiles are read", |stage| {
        stage.shell().profiles.len() > 1
    });
    stage.act(keymap::ShowPreferences);
    stage.until("the current profile is a choice", |stage| {
        stage.shows("current-profile")
    });
    // The box is 281 by 40 with a border of one point on each side; the
    // choice sits in a popover of the toolkit's between the two, so a
    // height relative to the box would give the toolkit's 32 for a button.
    let (_, _, w, h) = stage.place("current-profile");
    assert_eq!((w, h), (279., 38.), "the choice fills its box");
}

/// The SSH section: letting go of the agent and naming a key file writes
/// the command ssh is run with; a file that is not there is refused at
/// the field and nothing is kept; the agent again takes the command out.
fn the_current_profile_connects_with_a_key_file_or_the_agent() {
    repos::fresh();
    let repo = repos::project();
    let key = repos::work().join("connect_key");
    std::fs::write(&key, "a key file\n").unwrap();
    // The user's own settings, which the profiles write: the file the
    // application was given (`repos::git` runs with settings of its own).
    let global = std::env::var("GIT_CONFIG_GLOBAL").unwrap();
    let command = || {
        std::fs::read_to_string(&global)
            .unwrap()
            .lines()
            .find(|line| line.trim_start().starts_with("sshCommand"))
            .unwrap_or_default()
            .to_owned()
    };
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the current profile is read", |stage| {
        stage.shell().profile.is_some()
    });
    stage.act(keymap::ShowPreferences);
    stage.click("section-ssh");
    assert_eq!(stage.shell().preferences_section, Some("ssh"));
    stage.click("check-ssh-agent");
    stage.click("field-ssh-key");
    stage.type_text("/nowhere/at/all");
    stage.keys("enter");
    stage.pass(Duration::from_millis(500));
    assert_eq!(command(), "", "a file that is not there is not used");
    stage.keys(&["backspace"; 15].join(" "));
    stage.type_text(&key.display().to_string());
    stage.keys("enter");
    stage.until("ssh is run with the key file", |_| {
        command().contains("connect_key")
    });
    stage.click("check-ssh-agent");
    stage.until("and with the agent again", |_| command().is_empty());
}

/// Each external tool is a list: git's or the system's choice, what was
/// found here, or a command of one's own, which brings a field of its
/// own under the list.
fn a_tool_is_chosen_from_a_list_or_typed() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let merge = |stage: &mut Stage| {
        stage.app(|cx| {
            cx.global::<bitbraid_ui::prefs::Prefs>()
                .preferences
                .external_merge
                .clone()
        })
    };
    stage.act(keymap::ShowPreferences);
    stage.click("section-tools");
    assert!(!stage.shows("field-Merge tool command"));
    // The last choice of the list is one's own.
    stage.click("select-External merge tool");
    stage.keys("up enter");
    assert!(stage.shows("field-Merge tool command"), "a field for it");
    assert_eq!(merge(&mut stage), None, "nothing typed yet");
    stage.click("field-Merge tool command");
    stage.type_text("vimdiff $LOCAL $REMOTE");
    assert_eq!(merge(&mut stage).as_deref(), Some("vimdiff $LOCAL $REMOTE"));
    // The first is git's own again, and the field goes.
    stage.click("select-External merge tool");
    stage.keys("down enter");
    assert_eq!(merge(&mut stage), None);
    assert!(!stage.shows("field-Merge tool command"));
}

/// The sections of the open repository are in the column while its tab is
/// shown, and gone with a tab that has none. Gitflow is set up from its
/// section, which the palette's entry opens; the hooks are read.
fn a_repositorys_sections_follow_its_tab() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stage.act(keymap::FlowSettings);
    assert_eq!(stage.shell().preferences_section, Some("gitflow"));
    assert!(stage.shows("flow-0"), "the main branch's field");
    stage.click("flow-save");
    stage.until("gitflow is set up", |_| {
        repos::git(
            &repo,
            &["config", "--default", "", "--get", "gitflow.branch.develop"],
        )
        .trim()
            == "develop"
    });
    // Further down the column: the hooks, and the large files.
    stage.with_shell(|shell, window, cx| {
        shell.show_preferences(bitbraid_ui::views::preferences::Section::Hooks, window, cx)
    });
    assert_eq!(stage.shell().preferences_section, Some("hooks"));
    stage.act(keymap::ShowLfs);
    assert_eq!(stage.shell().preferences_section, Some("lfs"));
    // A tab without a repository has none of its sections.
    stage.click("preferences-close");
    stage.act(keymap::NewTab);
    stage.act(keymap::ShowPreferences);
    assert_eq!(stage.shell().preferences_section, Some("general"));
    assert!(!stage.shows("section-gitflow"));
    assert!(stage.shows("section-signing"));
}

/// The pinned tab at the left end of the tab bar shows the page of
/// repositories, and a tab chosen puts it away. Each repository known is
/// listed in the sections it is known in, with its branch, how far that
/// is from its upstream and what is uncommitted, as git says once it has
/// been asked; one whose folder is gone says so. The search, the folding
/// and the box for the changes change what is shown; a repository is made
/// a favourite, taken out of a list, opened and closed from the page.
fn the_repositories_tab_shows_every_repository_known() {
    repos::fresh();
    // With a remote it is ahead of; and the library it takes in, which
    // follows nothing.
    let repo = repos::project_with_everything();
    let library = repos::work().join("library");
    assert!(library.join(".git").is_dir());
    std::fs::write(repo.join("a-new-note.txt"), "new\n").unwrap();
    let gone = repos::work().join("gone");
    let state = format!("recent = [{repo:?}, {library:?}, {gone:?}]\nfavourites = []\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    let name = |path: &std::path::Path| path.file_name().unwrap().to_str().unwrap().to_owned();
    let (project, lib) = (name(&repo), name(&library));
    assert!(
        stage.shows("pull-more"),
        "the toolbar is there with the graph"
    );

    stage.click("repositories-tab");
    assert_eq!(stage.shell().sheet, Some("repositories"));
    assert!(!stage.shows("pull-more"), "no toolbar over the page");
    stage.until("git has been asked about each repository", |stage| {
        stage.shows(format!("branch-recent-{lib}"))
            && stage.shows(format!("changes-open-{project}"))
            && stage.shows("problem-recent-gone")
    });
    assert!(
        stage.shows(format!("ahead-open-{project}")),
        "ahead of its remote"
    );
    assert!(
        !stage.shows(format!("changes-recent-{lib}"))
            && !stage.shows(format!("ahead-recent-{lib}")),
        "nothing uncommitted in the library, and nothing it follows"
    );
    for part in ["open", "recent", "all"] {
        assert!(stage.shows(format!("repo-{part}-{project}")), "{part}");
    }
    assert!(!stage.shows(format!("repo-open-{lib}")), "not open");

    // The box for the changes.
    stage.click("repositories-changes");
    assert!(!stage.shows(format!("changes-open-{project}")));
    assert!(stage.shows(format!("branch-open-{project}")));
    stage.click("repositories-changes");
    assert!(stage.shows(format!("changes-open-{project}")));

    // The search has the focus as the page is shown.
    stage.type_text(&lib);
    assert!(stage.shows(format!("repo-recent-{lib}")));
    assert!(!stage.shows(format!("repo-recent-{project}")));
    assert!(!stage.shows(format!("repo-open-{project}")));
    stage.keys("ctrl-a backspace");
    assert!(stage.shows(format!("repo-recent-{project}")));

    // Folding: all of them, and one.
    stage.click("repositories-collapse");
    for part in ["open", "recent", "all"] {
        assert!(!stage.shows(format!("repo-{part}-{project}")), "{part}");
        assert!(stage.shows(format!("part-{part}")), "{part}");
    }
    stage.click("repositories-expand");
    stage.click("part-recent");
    assert!(!stage.shows(format!("repo-recent-{lib}")));
    assert!(stage.shows(format!("repo-open-{project}")));
    stage.click("part-recent");
    assert!(stage.shows(format!("repo-recent-{lib}")));

    // A favourite, then taken out of the recent ones: it stays a
    // favourite.
    stage.click(format!("star-recent-{lib}"));
    assert!(stage.shows(format!("repo-favourites-{lib}")));
    stage.click(format!("take-out-recent-{lib}"));
    assert!(!stage.shows(format!("repo-recent-{lib}")));
    assert!(stage.shows(format!("repo-favourites-{lib}")));

    // Opened from the page: a tab of its own, and the page goes away.
    stage.click(format!("open-favourites-{lib}"));
    stage.until("the library is open in a tab", |stage| {
        stage.shell().tabs.len() == 2
    });
    assert_eq!(stage.shell().sheet, None);

    // Closed from the page, which stays.
    stage.click("repositories-tab");
    assert!(stage.shows(format!("repo-open-{lib}")));
    stage.click(format!("take-out-open-{lib}"));
    assert_eq!(stage.shell().tabs.len(), 1);
    assert_eq!(stage.shell().sheet, Some("repositories"));
    assert!(!stage.shows(format!("repo-open-{lib}")));

    // Escape puts it away, and so does a tab chosen; its own tab, clicked
    // again, leaves it shown, as a shown tab is.
    stage.keys("escape");
    assert_eq!(stage.shell().sheet, None);
    stage.click("repositories-tab");
    assert_eq!(stage.shell().sheet, Some("repositories"));
    stage.click("repositories-tab");
    assert_eq!(stage.shell().sheet, Some("repositories"));
    stage.click(("tab-name", 0usize));
    assert_eq!(stage.shell().sheet, None);
    assert!(stage.shows("pull-more"), "the toolbar is back");
    // Its key shows it.
    stage.keys("ctrl-alt-o");
    assert_eq!(stage.shell().sheet, Some("repositories"));
}

/// A remote, a worktree and a submodule are asked for in a pane that docks
/// at the right edge of the references panel, over the graph, each at a
/// width of its own: the fields one under the other, Tab from one to the
/// next and Enter to go ahead. What will not do is said in the pane, which
/// stays; what will do is done and the pane goes. Escape and its cross put
/// it away.
fn what_the_panel_adds_is_asked_in_a_pane_beside_it() {
    use bitbraid_ui::views::{
        refs::RemoteRequest,
        submodules::{RunSubmodule, SubmoduleRequest},
        worktrees::{RunWorktree, WorktreeRequest},
    };
    repos::fresh();
    let repo = repos::project();
    let elsewhere = repos::work().join("elsewhere.git");
    repos::git(
        &repo,
        &["clone", "-q", "--bare", ".", elsewhere.to_str().unwrap()],
    );
    let mut stage = Stage::open(Start::with(&repo));
    let add_remote = |stage: &mut Stage| {
        stage.with_workspace(|workspace, window, cx| {
            workspace.update(cx, |workspace, cx| {
                workspace.remote(RemoteRequest::Add, window, cx)
            })
        });
        stage.frame();
    };

    // A remote, with somewhere else to push to.
    add_remote(&mut stage);
    assert_eq!(stage.workspace().docked, Some("remote"));
    assert!(!stage.dialog_open(), "no dialog");
    // At the panel's right edge, not in the middle of the window.
    let (pane_x, _, pane_width, _) = stage.place("docked-remote");
    assert_eq!(pane_width, 500.);
    assert!((150.0..=320.0).contains(&pane_x), "{pane_x}");
    let push = "/srv/nowhere/push-here.git";
    stage.type_text("upstream");
    stage.keys("tab");
    stage.type_text(elsewhere.to_str().unwrap());
    stage.keys("tab");
    stage.type_text(push);
    assert_eq!(
        stage.workspace().docked_fields,
        ["upstream", elsewhere.to_str().unwrap(), push]
    );
    stage.keys("enter");
    assert_eq!(stage.workspace().docked, None, "the pane goes");
    stage.until("the remote is added", |_| {
        repos::git(&repo, &["remote"]).contains("upstream")
    });
    stage.until("its pushes go elsewhere", |_| {
        repos::git(&repo, &["config", "--get-all", "remote.upstream.pushurl"]).trim() == push
    });
    stage.pass(Duration::from_secs(1));

    // A name that is taken: said, and the pane stays.
    add_remote(&mut stage);
    stage.type_text("upstream");
    stage.until("the name is said to be taken", |stage| {
        stage.shows("docked-said")
    });
    stage.keys("tab");
    stage.type_text(elsewhere.to_str().unwrap());
    stage.keys("enter");
    assert_eq!(stage.workspace().docked, Some("remote"));
    stage.keys("escape");
    assert_eq!(stage.workspace().docked, None);

    // A worktree on a new branch: its folder follows the branch.
    stage.act(keymap::FocusReferences);
    stage.act(RunWorktree {
        request: WorktreeRequest::New(None),
    });
    assert_eq!(stage.workspace().docked, Some("worktree"));
    let (_, _, pane_width, _) = stage.place("docked-worktree");
    assert_eq!(pane_width, 600.);
    let fields = stage.workspace().docked_fields;
    assert_eq!(fields[..2], ["", ""]);
    assert!(fields[2].ends_with("project-worktree"), "{fields:?}");
    // Nothing to check out and no branch: said, and it stays.
    stage.keys("enter");
    assert_eq!(stage.workspace().docked, Some("worktree"));
    assert!(stage.shows("docked-said"));
    stage.keys("tab");
    stage.type_text("feature/side");
    let folder = stage.workspace().docked_fields[2].clone();
    assert!(
        folder.ends_with("project-side"),
        "{:?}",
        stage.workspace().docked_fields
    );
    assert!(stage.shows("docked-browse") && stage.shows("docked-refs"));
    stage.keys("enter");
    assert_eq!(stage.workspace().docked, None);
    stage.until("the worktree is there on its branch", |_| {
        std::path::Path::new(&folder).join(".git").is_file()
    });
    assert_eq!(
        repos::git(std::path::Path::new(&folder), &["branch", "--show-current"]).trim(),
        "feature/side"
    );
    stage.pass(Duration::from_secs(1));

    // From a tag: checked out as it is, which is said.
    stage.act(keymap::FocusReferences);
    stage.act(RunWorktree {
        request: WorktreeRequest::New(Some("v0.1".to_owned())),
    });
    assert_eq!(stage.workspace().docked_fields[0], "v0.1");
    assert!(stage.shows("docked-said"), "detached");
    stage.click("docked-close");
    assert_eq!(stage.workspace().docked, None);

    // A submodule: 400 wide, its three fields.
    stage.act(keymap::FocusReferences);
    stage.act(RunSubmodule {
        request: SubmoduleRequest::Add,
    });
    assert_eq!(stage.workspace().docked, Some("submodule"));
    let (_, _, pane_width, _) = stage.place("docked-submodule");
    assert_eq!(pane_width, 400.);
    assert_eq!(stage.workspace().docked_fields, ["", "", ""]);
    // One pane at a time: the remote's takes its place.
    add_remote(&mut stage);
    assert_eq!(stage.workspace().docked, Some("remote"));
    stage.keys("escape");
    assert_eq!(stage.workspace().docked, None);
}
