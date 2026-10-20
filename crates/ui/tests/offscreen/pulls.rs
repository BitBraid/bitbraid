//! The pull requests in the window: what the section, the card, the filter
//! dialog, the view of one and the graph's marks do, and their pictures.
//! They are given the shapes the provider commands answer with
//! (`bitbraid_integrations::model`), made here, so that nothing depends on
//! a network or an account.

use std::time::Duration;

use bitbraid_integrations::model::{
    Check, CiState, Comment, CommitStatus, Freshness, Label, Listing, MergeMethod, Mergeable,
    Milestone, ProviderKind, PullCommit, PullDetails, PullFile, PullRequest, PullState, RepoLink,
    Review, ReviewState, Reviewer, Thread, User,
};
use bitbraid_ui::{
    prefs::Prefs,
    views::pulls::{
        facts::Group,
        section::{Given, PullsProbe},
        view::PullViewProbe,
    },
};
use gpui_kit::{ElementId, Pixels, Point, ScrollDelta, point, px, test::TestWindowExt as _};

use crate::{
    repos,
    runner::{Shots, Test, Themes},
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "the_pull_requests_section_shows_with_a_linked_remote_and_its_groups_close",
        run: the_pull_requests_section_shows_with_a_linked_remote_and_its_groups_close,
    },
    Test {
        name: "the_search_box_keeps_the_pull_requests_that_match",
        run: the_search_box_keeps_the_pull_requests_that_match,
    },
    Test {
        name: "a_rows_card_stands_beside_it_over_the_panels_edge",
        run: a_rows_card_stands_beside_it_over_the_panels_edge,
    },
    Test {
        name: "a_pull_request_opens_in_the_middle_and_its_close_button_goes_back",
        run: a_pull_request_opens_in_the_middle_and_its_close_button_goes_back,
    },
    Test {
        name: "a_filter_is_added_from_its_dialog_and_kept_with_the_repository",
        run: a_filter_is_added_from_its_dialog_and_kept_with_the_repository,
    },
    Test {
        name: "the_graph_marks_known_checks_and_branches_with_an_open_pull_request",
        run: the_graph_marks_known_checks_and_branches_with_an_open_pull_request,
    },
    Test {
        name: "a_remote_at_github_without_an_account_invites_to_connect_one",
        run: a_remote_at_github_without_an_account_invites_to_connect_one,
    },
    Test {
        name: "hostile_answers_are_shown_without_harm",
        run: hostile_answers_are_shown_without_harm,
    },
    Test {
        name: "recorded_answers_of_a_gitlab_fill_the_section_and_a_merge_requests_view",
        run: recorded_answers_of_a_gitlab_fill_the_section_and_a_merge_requests_view,
    },
    Test {
        name: "recorded_answers_of_github_fill_the_section_and_a_pull_requests_view",
        run: recorded_answers_of_github_fill_the_section_and_a_pull_requests_view,
    },
    Test {
        name: "a_merge_request_read_again_without_its_provider_says_it_is_offline",
        run: a_merge_request_read_again_without_its_provider_says_it_is_offline,
    },
    Test {
        name: "a_thousand_pull_requests_draw_the_rows_in_view_and_the_same_ones_tell_nothing",
        run: a_thousand_pull_requests_draw_the_rows_in_view_and_the_same_ones_tell_nothing,
    },
];

/// The ids of the rows of pull requests `numbers` of the group of all
/// that the window drew in its last frame, in view or not.
fn drawn_rows(stage: &mut Stage, numbers: std::ops::RangeInclusive<u64>) -> Vec<u64> {
    stage.window(|window, _| {
        numbers
            .filter(|number| window.try_find(row("all", 0, *number)).is_some())
            .collect()
    })
}

/// A project with a thousand open pull requests: the section draws only the
/// rows in view, as it is scrolled too, and reading the lists again every
/// five minutes, answered the same, tells neither the graph nor the panel
/// anything anew. Every row drawn with every frame, or the branches' marks
/// made over with every answer, would freeze the window.
fn a_thousand_pull_requests_draw_the_rows_in_view_and_the_same_ones_tell_nothing() {
    use bitbraid_ui::views::pulls::{calls, section::PullsEvent};
    use std::{cell::Cell, rc::Rc};
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let many: Vec<PullRequest> = (1..=1000u64)
        .map(|n| pull(n, &format!("topic/{n}"), &format!("Change {n}"), "ada"))
        .collect();
    let all = serde_json::to_value(listing(many, false)).unwrap();
    let none = serde_json::to_value(listing(Vec::new(), false)).unwrap();
    calls::stand_in(move |id, params| match id {
        "pr.list" if params["filter"] == "all" => Some(Ok(all.clone())),
        "pr.list" => Some(Ok(none.clone())),
        "ci.status" => Some(Ok(serde_json::json!([]))),
        _ => None,
    });
    let pulls = stage.with_workspace(|workspace, _, cx| workspace.read(cx).pulls());
    // How many times the graph was told the marks, and the panel the
    // counts.
    let told = Rc::new(Cell::new((0usize, 0usize)));
    let counter = told.clone();
    stage.app(|cx| {
        cx.subscribe(&pulls, move |_, event: &PullsEvent, _| {
            let (marked, counted) = counter.get();
            match event {
                PullsEvent::Marked(_) => counter.set((marked + 1, counted)),
                PullsEvent::Counted => counter.set((marked, counted + 1)),
                _ => {}
            }
        })
        .detach()
    });
    let link = links().into_iter().next().unwrap();
    stage.app(|cx| pulls.update(cx, |view, cx| view.link_to(vec![link], cx)));
    toggle(&mut stage, "pulls");
    stage.until("the lists are read", |stage| {
        stage.settle();
        probe(stage)
            .groups
            .iter()
            .any(|(key, count, _)| key == "all" && *count == 1000)
    });

    let first = drawn_rows(&mut stage, 1..=1000);
    assert!(
        first.first() == Some(&1) && first.len() <= 60,
        "the rows in view: {} drawn, {first:?}",
        first.len()
    );
    let (x, y, w, h) = stage.place(row("all", 0, 1));
    crate::behaviour_fit::wheel_at(&mut stage, (x + w / 2., y + h / 2.), -24. * 600.);
    let scrolled = drawn_rows(&mut stage, 1..=1000);
    assert!(
        !scrolled.contains(&1)
            && scrolled.iter().any(|number| (500..=700).contains(number))
            && scrolled.len() <= 60,
        "the rows in view, scrolled: {} drawn, {scrolled:?}",
        scrolled.len()
    );

    let marked = told.get();
    assert!(marked.0 >= 1, "the graph was told the marks: {marked:?}");
    let reads = |_: &mut Stage| {
        calls::asked()
            .iter()
            .filter(|(id, _)| id == "pr.list")
            .count()
    };
    let before = reads(&mut stage);
    for _ in 0..3 {
        stage.pass(bitbraid_ui::views::pulls::section::READ_EVERY + Duration::from_secs(1));
        stage.settle();
    }
    assert!(reads(&mut stage) > before, "read again every five minutes");
    assert_eq!(
        told.get(),
        marked,
        "the same lists tell the graph and the panel nothing anew"
    );
    calls::stand_down();
}

/// A repository whose `origin` is `remote`, opened in a window whose
/// provider commands answer from the recordings `files` (made by the
/// integrations crate's live tests, `crates/integrations/tests/recordings`)
/// with a keychain in memory, and an account connected at `web` by a
/// stand-in token: the recording says whose it is. Gives the window once
/// the section has read its lists, and the account's id.
pub fn recorded(
    kind: &str,
    web: &str,
    remote: &str,
    files: &[&str],
    no_checks: &[&str],
) -> (Stage, String) {
    recorded_through(
        kind,
        web,
        remote,
        files,
        no_checks,
        |replay| std::sync::Arc::new(replay),
        false,
    )
}

/// The same, with the recordings answering through what `through` makes
/// of them, and, when `kept`, the answers kept on disk as the application
/// keeps them.
pub fn recorded_through(
    kind: &str,
    web: &str,
    remote: &str,
    files: &[&str],
    no_checks: &[&str],
    through: impl FnOnce(
        bitbraid_integrations::replay::Replay,
    ) -> std::sync::Arc<dyn bitbraid_integrations::http::Fetch>,
    kept: bool,
) -> (Stage, String) {
    use std::sync::Arc;

    use bitbraid_integrations::{
        replay::{Recording, Replay},
        secrets::MemoryStore,
        services::Services,
    };

    repos::fresh();
    let repo = repos::project();
    repos::git(&repo, &["remote", "add", "origin", remote]);
    let recordings =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../integrations/tests/recordings");
    let mut recording = Recording::default();
    for name in files {
        let text = std::fs::read_to_string(recordings.join(name)).unwrap();
        let read: Recording = serde_json::from_str(&text).unwrap();
        recording.exchanges.extend(read.exchanges);
    }
    // Made here: the statuses of the head commits the live test did not
    // ask about, which have none.
    for sha in no_checks {
        recording.exchanges.push(
            serde_json::from_value(serde_json::json!({
                "method": "GET",
                "path_and_query": format!(
                    "/api/v4/projects/bb-p-group%2Fbb-p-project/repository/commits/{sha}/statuses?per_page=100"
                ),
                "request_body": null,
                "status": 200,
                "headers": { "content-type": "application/json" },
                "body": [],
            }))
            .unwrap(),
        );
    }
    // The window's configuration directory, where `Stage` puts it: its
    // commands take these stand-ins from now on in this process, and ask
    // no network and no keychain of the machine.
    let config = crate::hermetic::inside().unwrap().join("config/bitbraid");
    let mut services = Services::new(
        through(Replay::new(recording)),
        Arc::new(MemoryStore::new()),
        &config,
    );
    if kept {
        let cache = crate::hermetic::inside()
            .unwrap()
            .join("cache/providers-kept");
        let _ = std::fs::remove_dir_all(&cache);
        services.cache_dir = Some(cache);
    }
    Services::install(&config, Arc::new(services));
    let mut stage = Stage::open(Start::with(&repo));
    // No account yet: nothing is listed (at github.com, an invitation is).
    assert!(probe(&mut stage).rows.is_empty());
    let account = bitbraid_integrations::registry()
        .run(
            "account.add_token",
            &mut context(&config),
            &serde_json::json!({ "kind": kind, "web": web, "token": "stand-in-token" }),
        )
        .unwrap();
    let account = account["id"].as_str().unwrap().to_owned();
    // The heading's refresh asks again which remotes are linked.
    stage.with_workspace(|workspace, _, cx| {
        let pulls = workspace.read(cx).pulls();
        pulls.update(cx, |view, cx| view.refresh(cx));
    });
    stage.until("the recorded lists are in", |stage| {
        stage.settle();
        let seen = probe(stage);
        seen.listed && !seen.reading && seen.groups.len() == 4
    });
    (stage, account)
}

/// A context of commands for the window's configuration directory.
fn context(config: &std::path::Path) -> bitbraid_engine::CommandContext {
    bitbraid_engine::CommandContext {
        config_dir: Some(config.to_path_buf()),
        ..Default::default()
    }
}

/// How many pull requests `pr.list` answers for each of the section's
/// groups, asked of the same recordings as the window.
fn listed_by_the_command(account: &str, owner: &str, name: &str) -> Vec<(String, usize, bool)> {
    let config = crate::hermetic::inside().unwrap().join("config/bitbraid");
    ["mine", "assigned", "review_requested", "all"]
        .into_iter()
        .map(|filter| {
            let listing = bitbraid_integrations::registry()
                .run(
                    "pr.list",
                    &mut context(&config),
                    &serde_json::json!({
                        "account": account, "owner": owner, "name": name, "filter": filter,
                    }),
                )
                .unwrap();
            let count = listing["items"].as_array().map_or(0, Vec::len);
            (filter.to_owned(), count, true)
        })
        .collect()
}

/// The whole way, with no network, at a GitLab: the window's provider
/// commands answer from what the integrations crate's live tests recorded
/// at a GitLab of one's own; the section's groups hold what `pr.list`
/// answers for each, and a click on a merge request shows what `pr.show`
/// read of it.
fn recorded_answers_of_a_gitlab_fill_the_section_and_a_merge_requests_view() {
    let (mut stage, account) = recorded(
        "gitlab",
        "http://127.0.0.1:8929",
        "http://127.0.0.1:8929/bb-p-group/bb-p-project.git",
        &[
            "gitlab/pr_list.json",
            "gitlab/pr_show.json",
            "gitlab/ci_status.json",
        ],
        &[
            "f780180fe81ed6da21727435683eaaa0f3ac51df",
            "5d7ea1757209327bfe5afc71748a40369ff2e76b",
        ],
    );
    let seen = probe(&mut stage);
    assert_eq!(
        seen.groups,
        [
            ("mine".to_owned(), 4, true),
            ("assigned".to_owned(), 1, true),
            ("review_requested".to_owned(), 1, true),
            ("all".to_owned(), 5, true),
        ],
        "{seen:?}"
    );
    assert_eq!(
        seen.groups,
        listed_by_the_command(&account, "bb-p-group", "bb-p-project")
    );
    assert!(!seen.offline);
    assert!(seen.rows.contains(&"!2 Add a file to review".to_owned()));
    // The checks of the five open merge requests' head commits, from
    // what ci.status read of them: three have statuses, two none.
    stage.until("the checks of the head commits are read", |stage| {
        stage.settle();
        probe(stage).checks == 5
    });
    toggle(&mut stage, "pulls");
    stage.click(row("assigned", 0, 2));
    stage.until("the merge request is read", |stage| {
        stage.settle();
        view_probe(stage).is_some_and(|seen| !seen.loading)
    });
    let seen = view_probe(&mut stage).unwrap();
    assert_eq!(seen.error, None);
    assert_eq!(seen.title.as_deref(), Some("Add a file to review"));
    assert!(seen.files > 0, "{seen:?}");
    stage.click("pull-close");
    assert_eq!(stage.workspace().sheet, None);
}

/// A recorded provider that can be cut off, as when its server stops.
#[derive(Debug)]
struct Unplugged {
    replay: bitbraid_integrations::replay::Replay,
    off: std::sync::atomic::AtomicBool,
}

impl bitbraid_integrations::http::Fetch for Unplugged {
    fn send(
        &self,
        call: &bitbraid_integrations::http::Call,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<bitbraid_integrations::http::Response, bitbraid_integrations::http::FetchError>
    {
        if self.off.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(bitbraid_integrations::http::FetchError::Connect(
                "network is unreachable".into(),
            ));
        }
        self.replay.send(call, cancel)
    }
}

/// A merge request read again while its provider cannot be reached shows
/// what was kept of it, and its heading says that it is offline and when it
/// was read.
fn a_merge_request_read_again_without_its_provider_says_it_is_offline() {
    use std::sync::{Arc, atomic::Ordering};

    let unplugged: Arc<std::sync::OnceLock<Arc<Unplugged>>> = Arc::default();
    let made = unplugged.clone();
    let (mut stage, _) = recorded_through(
        "gitlab",
        "http://127.0.0.1:8929",
        "http://127.0.0.1:8929/bb-p-group/bb-p-project.git",
        &[
            "gitlab/pr_list.json",
            "gitlab/pr_show.json",
            "gitlab/ci_status.json",
        ],
        &[
            "f780180fe81ed6da21727435683eaaa0f3ac51df",
            "5d7ea1757209327bfe5afc71748a40369ff2e76b",
        ],
        move |replay| {
            let fetch = Arc::new(Unplugged {
                replay,
                off: std::sync::atomic::AtomicBool::new(false),
            });
            let _ = made.set(fetch.clone());
            fetch
        },
        true,
    );
    toggle(&mut stage, "pulls");
    stage.click(row("assigned", 0, 2));
    stage.until("the merge request is read", |stage| {
        stage.settle();
        view_probe(stage).is_some_and(|seen| !seen.loading && seen.title.is_some())
    });
    let seen = view_probe(&mut stage).unwrap();
    assert_eq!(seen.offline, None, "read from the provider");
    assert!(!stage.shows("pull-offline"));

    unplugged
        .get()
        .expect("the network was made")
        .off
        .store(true, Ordering::SeqCst);
    stage.click("pull-refresh");
    stage.until("it is read again", |stage| {
        stage.settle();
        view_probe(stage).is_some_and(|seen| !seen.loading)
    });
    let seen = view_probe(&mut stage).unwrap();
    assert_eq!(seen.error, None, "what was kept is answered");
    assert_eq!(seen.title.as_deref(), Some("Add a file to review"));
    assert!(
        seen.offline
            .as_deref()
            .is_some_and(|said| said.starts_with("Offline: as read")),
        "{seen:?}"
    );
    assert!(stage.shows("pull-offline"), "the heading says so");
    stage.click("pull-close");
}

/// The same at GitHub, from what was recorded of a private test
/// repository there: the groups hold what `pr.list` answers, and one of
/// the pull requests whose view was recorded opens with what `pr.show`
/// read.
fn recorded_answers_of_github_fill_the_section_and_a_pull_requests_view() {
    let (mut stage, account) = recorded(
        "github",
        "https://github.com",
        "https://github.com/user-1/bitbraid-test-core.git",
        &["github/pr_list.json", "github/pr_show.json"],
        &[],
    );
    let seen = probe(&mut stage);
    assert_eq!(
        seen.groups,
        listed_by_the_command(&account, "user-1", "bitbraid-test-core"),
        "{seen:?}"
    );
    assert!(
        seen.groups.iter().any(|(_, count, _)| *count > 0),
        "{seen:?}"
    );
    // One whose view was recorded.
    let number = [1u64, 2, 4]
        .into_iter()
        .find(|number| {
            let shown = format!("#{number} ");
            seen.rows.iter().any(|row| row.starts_with(&shown))
        })
        .expect("a pull request whose view was recorded is listed");
    let opened = stage.with_workspace(|workspace, _, cx| {
        let pulls = workspace.read(cx).pulls();
        pulls.update(cx, |view, cx| view.open_number(number, cx))
    });
    assert!(opened);
    stage.until("the pull request is read", |stage| {
        stage.settle();
        view_probe(stage).is_some_and(|seen| !seen.loading)
    });
    let seen = view_probe(&mut stage).unwrap();
    assert_eq!(seen.error, None, "{seen:?}");
    assert_eq!(seen.number, number);
    assert!(seen.title.is_some_and(|title| !title.is_empty()));
    stage.click("pull-close");
    assert_eq!(stage.workspace().sheet, None);
}

/// A remote at github.com with no account connected: the window asks the
/// real commands (`provider.links`, `account.list`), lists the section with
/// nothing in it but a line that invites to connect an account, and the
/// line opens the settings where that is done.
fn a_remote_at_github_without_an_account_invites_to_connect_one() {
    repos::fresh();
    let repo = repos::project();
    repos::git(
        &repo,
        &["remote", "add", "upstream", "git@github.com:octo/tools.git"],
    );
    let mut stage = Stage::open(Start::with(&repo));
    stage.until("the section invites", |stage| {
        probe(stage).invite == ["github.com"]
    });
    let seen = probe(&mut stage);
    assert!(seen.listed && seen.rows.is_empty(), "{seen:?}");
    assert!(
        sections(&mut stage)
            .iter()
            .any(|(key, _, _)| *key == "pulls")
    );
    toggle(&mut stage, "pulls");
    assert!(
        stage.shows(("pulls-connect", 0usize)),
        "alone: with no linked remote there are no groups"
    );
    stage.click(("pulls-connect", 0usize));
    stage.settle();
    assert_eq!(stage.shell().preferences_section, Some("integrations"));
}

/// What a provider answers is another machine's: titles with control
/// characters and no end, numbers at their largest, colours that are none,
/// a description that nests without end, a patch whose ranges are no
/// numbers. All of it is drawn, opened and closed without harm.
fn hostile_answers_are_shown_without_harm() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let mut odd = pull(
        u64::MAX,
        "\u{0}\u{1b}[31m",
        &"\u{202e}long\n".repeat(5000),
        "\t",
    );
    odd.labels = vec![
        label("x", "zzzzzz"),
        label(&"y".repeat(10_000), "#"),
        Label {
            name: String::new(),
            color: None,
            description: None,
        },
    ];
    odd.created = "9999-99-99T99:99:99Z".to_owned();
    odd.updated = String::new();
    odd.head_repo = Some("/".to_owned());
    odd.repo = String::new();
    odd.web_url = "javascript:alert(1)".to_owned();
    let mut given = given();
    given.links.truncate(1);
    given.lists = vec![(Group::All, 0, listing(vec![odd.clone()], true))];
    given.checks = vec![CommitStatus {
        sha: odd.head_sha.clone(),
        state: CiState::Failure,
        checks: (0..500)
            .map(|n| Check {
                name: format!("{n}\u{7}"),
                state: CiState::Neutral,
                url: Some("file:///etc/passwd".to_owned()),
                description: Some("\u{0}".repeat(100)),
                started: Some("yesterday".to_owned()),
                finished: None,
            })
            .collect(),
    }];
    give(&mut stage, given);
    toggle(&mut stage, "pulls");
    let seen = probe(&mut stage);
    assert_eq!(seen.rows.len(), 1, "{seen:?}");
    for group in ["mine", "assigned", "review_requested"] {
        stage.click(ElementId::Name(format!("pulls-group:{group}").into()));
    }
    // Its card, while the pointer rests on its row.
    let (x, y, w, h) = stage.place(row("all", 0, u64::MAX));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    stage.click(row("all", 0, u64::MAX));
    stage.settle();
    let mut details = details();
    details.pull = odd;
    details.body = format!(
        "{}{}{}",
        ">".repeat(100_000),
        "*_".repeat(50_000),
        "[".repeat(50_000)
    );
    details.files = vec![PullFile {
        path: String::new(),
        previous_path: None,
        status: "\u{0}".to_owned(),
        additions: u64::MAX,
        deletions: u64::MAX,
        patch: Some(format!(
            "@@ -x,y +{},z @@\n+a\n-b\n\\\n?\n{}",
            u64::MAX,
            "+".repeat(1_000_000)
        )),
    }];
    details.threads.push(Thread {
        id: String::new(),
        path: Some("\u{0}".to_owned()),
        line: Some(u64::MAX),
        diff_hunk: Some("@@ @@\n".repeat(10_000)),
        resolved: true,
        comments: vec![comment("", "", &"`".repeat(100_000), "")],
    });
    details.milestone = Some(Milestone {
        id: String::new(),
        title: "\n".repeat(100),
        due: Some("-1".to_owned()),
        open: false,
    });
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace
            .read(cx)
            .pull_view()
            .expect("a pull request is shown");
        view.update(cx, |view, cx| view.give(details, cx));
    });
    stage.settle();
    stage.frame();
    stage.click(("pull-file-row", 0usize));
    stage.frame();
    assert_eq!(
        view_probe(&mut stage).unwrap().file_open.as_deref(),
        Some("")
    );
    stage.click("pull-close");
    assert_eq!(stage.workspace().sheet, None);
}

/// A person at a provider.
pub fn user(login: &str) -> User {
    User {
        login: login.to_owned(),
        name: None,
        avatar_url: None,
        web_url: None,
    }
}

pub fn label(name: &str, colour: &str) -> Label {
    Label {
        name: name.to_owned(),
        color: Some(colour.to_owned()),
        description: None,
    }
}

/// The clock of the tests stands at 2020-01-03 12:00 UTC: these are an
/// hour, a day and two days before.
const HOUR_AGO: &str = "2020-01-03T11:00:00Z";
const DAY_AGO: &str = "2020-01-02T12:00:00Z";
const TWO_DAYS_AGO: &str = "2020-01-01T12:00:00Z";

pub fn pull(number: u64, head: &str, title: &str, author: &str) -> PullRequest {
    PullRequest {
        repo: "octo/tools".to_owned(),
        number,
        title: title.to_owned(),
        state: PullState::Open,
        draft: false,
        author: user(author),
        head_ref: head.to_owned(),
        head_repo: None,
        head_sha: format!("{number:040}"),
        base_ref: "main".to_owned(),
        created: TWO_DAYS_AGO.to_owned(),
        updated: HOUR_AGO.to_owned(),
        labels: Vec::new(),
        reviewers: Vec::new(),
        assignees: Vec::new(),
        ci: CiState::None,
        comments: None,
        web_url: format!("https://github.com/octo/tools/pull/{number}"),
    }
}

/// The pull requests of the GitHub remote: one waiting, one ready, one
/// failing, one a draft.
pub fn github_pulls() -> Vec<PullRequest> {
    let mut waiting = pull(41, "wip", "Quotes and the needle in one pass", "ada");
    waiting.ci = CiState::Running;
    waiting.reviewers = vec![Reviewer {
        user: user("bob"),
        state: ReviewState::Pending,
    }];
    waiting.assignees = vec![user("octo")];
    waiting.labels = vec![label("enhancement", "a2eeef"), label("lexer", "5319e7")];
    let mut ready = pull(40, "pages", "A landing page for the docs", "octo");
    ready.ci = CiState::Success;
    ready.reviewers = vec![Reviewer {
        user: user("bob"),
        state: ReviewState::Approved,
    }];
    ready.labels = vec![label("docs", "0075ca")];
    let mut failing = pull(38, "feature/tried", "Try a cache in the lexer", "cy");
    failing.ci = CiState::Failure;
    failing.reviewers = vec![Reviewer {
        user: user("octo"),
        state: ReviewState::ChangesRequested,
    }];
    let mut draft = pull(37, "dates-in-log", "Dates in the log, a first try", "ada");
    draft.draft = true;
    draft.head_repo = Some("ada/tools".to_owned());
    vec![waiting, ready, failing, draft]
}

/// The merge request of the GitLab remote.
pub fn gitlab_pulls() -> Vec<PullRequest> {
    let mut dates = pull(12, "feature/dates", "Relative dates everywhere", "dee");
    dates.repo = "group/tools".to_owned();
    dates.ci = CiState::Success;
    dates.reviewers = vec![Reviewer {
        user: user("octo"),
        state: ReviewState::Approved,
    }];
    dates.web_url = "http://127.0.0.1:8929/group/tools/-/merge_requests/12".to_owned();
    vec![dates]
}

pub fn listing(items: Vec<PullRequest>, offline: bool) -> Listing<PullRequest> {
    Listing {
        items,
        more: false,
        freshness: Freshness {
            fetched: Some(if offline {
                "2020-01-03T11:40:00Z".to_owned()
            } else {
                "2020-01-03T12:00:00Z".to_owned()
            }),
            offline,
            remaining: None,
            reset: None,
        },
    }
}

pub fn links() -> Vec<RepoLink> {
    vec![
        RepoLink {
            remote: "origin".to_owned(),
            account: "github:github.com:octo".to_owned(),
            kind: ProviderKind::Github,
            owner: "octo".to_owned(),
            name: "tools".to_owned(),
        },
        RepoLink {
            remote: "lab".to_owned(),
            account: "gitlab:127.0.0.1:octo".to_owned(),
            kind: ProviderKind::Gitlab,
            owner: "group".to_owned(),
            name: "tools".to_owned(),
        },
    ]
}

/// What the section is given: two remotes, GitHub's and GitLab's, the
/// latter's lists kept from before while it could not be reached.
pub fn given() -> Given {
    let hub = github_pulls();
    let lab = gitlab_pulls();
    let pick = |numbers: &[u64]| -> Vec<PullRequest> {
        hub.iter()
            .filter(|pull| numbers.contains(&pull.number))
            .cloned()
            .collect()
    };
    Given {
        links: links(),
        lists: vec![
            (Group::Mine, 0, listing(pick(&[40]), false)),
            (Group::Assigned, 0, listing(pick(&[41]), false)),
            (Group::ReviewRequested, 0, listing(pick(&[38]), false)),
            (Group::All, 0, listing(hub.clone(), false)),
            (Group::Mine, 1, listing(Vec::new(), true)),
            (Group::Assigned, 1, listing(Vec::new(), true)),
            (Group::ReviewRequested, 1, listing(lab.clone(), true)),
            (Group::All, 1, listing(lab, true)),
        ],
        checks: vec![CommitStatus {
            sha: format!("{:040}", 41),
            state: CiState::Running,
            checks: checks(),
        }],
        invite: Vec::new(),
    }
}

pub fn checks() -> Vec<Check> {
    let check = |name: &str, state: CiState, started: &str, finished: Option<&str>| Check {
        name: name.to_owned(),
        state,
        url: Some(format!("https://github.com/octo/tools/actions/runs/{name}")),
        description: None,
        started: Some(started.to_owned()),
        finished: finished.map(str::to_owned),
    };
    vec![
        check(
            "build (ubuntu)",
            CiState::Success,
            "2020-01-03T11:20:00Z",
            Some("2020-01-03T11:31:00Z"),
        ),
        check(
            "test (ubuntu)",
            CiState::Running,
            "2020-01-03T11:32:00Z",
            None,
        ),
        check("lint", CiState::Pending, "2020-01-03T11:50:00Z", None),
    ]
}

pub fn comment(id: &str, login: &str, body: &str, created: &str) -> Comment {
    Comment {
        id: id.to_owned(),
        author: user(login),
        body: body.to_owned(),
        created: created.to_owned(),
        updated: None,
    }
}

/// Everything the view of #41 shows.
pub fn details() -> PullDetails {
    let pull = github_pulls().remove(0);
    PullDetails {
        pull,
        body: "Lexing quotes and looking for the needle went over the line twice. \
               This does it in **one pass**.\n\n\
               ## What changes\n\n\
               - `lex()` keeps track of quotes\n\
               - the needle is found on the way\n\n\
               See [the guide](https://example.org/guide) and ![the timing](https://example.org/t.png).\n\n\
               ```rust\nfn lex() { quotes(); needle(); }\n```\n"
            .to_owned(),
        milestone: Some(Milestone {
            id: "3".to_owned(),
            title: "0.3".to_owned(),
            due: Some("2020-02-01T00:00:00Z".to_owned()),
            open: true,
        }),
        mergeable: Mergeable::Clean,
        merge_methods: vec![MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase],
        checks: checks(),
        commits: vec![
            PullCommit {
                sha: "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678".to_owned(),
                summary: "lexer: count quotes as it goes".to_owned(),
                author: Some(user("ada")),
                author_name: "Ada".to_owned(),
                date: DAY_AGO.to_owned(),
            },
            PullCommit {
                sha: "b2c3d4e5f60718293a4b5c6d7e8f901234567890".to_owned(),
                summary: "wip: Ada's unfinished idea".to_owned(),
                author: Some(user("ada")),
                author_name: "Ada".to_owned(),
                date: HOUR_AGO.to_owned(),
            },
        ],
        files: vec![
            PullFile {
                path: "src/lexer.rs".to_owned(),
                previous_path: None,
                status: "modified".to_owned(),
                additions: 3,
                deletions: 1,
                patch: Some(
                    "@@ -1,3 +1,5 @@\n-fn lex() { quotes(); }\n+fn lex() {\n+    quotes();\n+    needle();\n+}\n \n // the end\n"
                        .to_owned(),
                ),
            },
            PullFile {
                path: "docs/guide.md".to_owned(),
                previous_path: None,
                status: "added".to_owned(),
                additions: 2,
                deletions: 0,
                patch: Some("@@ -0,0 +1,2 @@\n+Search for a needle.\n+Use quotes.\n".to_owned()),
            },
            PullFile {
                path: "assets/logo.png".to_owned(),
                previous_path: None,
                status: "added".to_owned(),
                additions: 0,
                deletions: 0,
                patch: None,
            },
        ],
        reviews: vec![Review {
            id: "r1".to_owned(),
            author: user("bob"),
            state: ReviewState::Commented,
            body: "Close: one question on the lexer.".to_owned(),
            submitted: Some("2020-01-03T09:00:00Z".to_owned()),
        }],
        threads: vec![
            Thread {
                id: "t1".to_owned(),
                path: None,
                line: None,
                diff_hunk: None,
                resolved: false,
                comments: vec![comment(
                    "c1",
                    "octo",
                    "Thanks! Does it still find a needle *inside* quotes?",
                    DAY_AGO,
                )],
            },
            Thread {
                id: "t2".to_owned(),
                path: Some("src/lexer.rs".to_owned()),
                line: Some(3),
                diff_hunk: Some(
                    "@@ -1,3 +1,5 @@\n-fn lex() { quotes(); }\n+fn lex() {\n+    quotes();\n+    needle();"
                        .to_owned(),
                ),
                resolved: false,
                comments: vec![
                    comment("c2", "bob", "Should `needle()` come before `quotes()`?", HOUR_AGO),
                    comment("c3", "ada", "No: a quote can hide a needle.", HOUR_AGO),
                ],
            },
        ],
        freshness: Default::default(),
    }
}

/// Give the section of the shown tab what it would have read.
pub fn give(stage: &mut Stage, given: Given) {
    stage.with_workspace(|workspace, _, cx| {
        let pulls = workspace.read(cx).pulls();
        pulls.update(cx, |view, cx| view.give(given, cx));
    });
    stage.settle();
}

pub fn probe(stage: &mut Stage) -> PullsProbe {
    stage.with_workspace(|workspace, _, cx| workspace.read(cx).pulls().read(cx).probe())
}

pub fn view_probe(stage: &mut Stage) -> Option<PullViewProbe> {
    stage.with_workspace(|workspace, _, cx| {
        workspace
            .read(cx)
            .pull_view()
            .map(|view| view.read(cx).probe())
    })
}

/// The sections of the references panel on show, with where each heading
/// is and how high its list is.
pub fn sections(stage: &mut Stage) -> Vec<(&'static str, Point<Pixels>, Option<f32>)> {
    stage.with_workspace(|workspace, _, cx| workspace.read(cx).refs().read(cx).sections_shown())
}

/// Open or close a section with a click on its heading.
pub fn toggle(stage: &mut Stage, section: &str) {
    let (_, head, _) = sections(stage)
        .into_iter()
        .find(|(key, _, _)| *key == section)
        .expect("the section is on show");
    assert!(head.y > px(0.), "the heading of {section} was drawn");
    stage.click_at((f32::from(head.x), f32::from(head.y)));
    stage.settle();
}

/// A repository with a remote, its pull requests given, and their
/// section open.
pub fn open_section(pictures: bool) -> Stage {
    repos::fresh();
    let repo = repos::project_with_everything();
    let start = Start::with(&repo);
    let mut stage = Stage::open(if pictures { start.pictures() } else { start });
    give(&mut stage, given());
    toggle(&mut stage, "pulls");
    stage
}

pub fn row(group: &str, link: usize, number: u64) -> ElementId {
    ElementId::Name(format!("pull:{group}:{link}:{number}").into())
}

/// The section is there while a remote is linked to an account (or could
/// be), says how many pull requests there are, reads them when it comes
/// into view, and its groups close and open from their headings and stay
/// as they were left.
fn the_pull_requests_section_shows_with_a_linked_remote_and_its_groups_close() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    // Its remote is a folder on this machine: nothing to link.
    assert!(
        !sections(&mut stage)
            .iter()
            .any(|(key, _, _)| *key == "pulls"),
        "{:?}",
        sections(&mut stage)
    );
    give(&mut stage, given());
    let keys: Vec<&str> = sections(&mut stage)
        .iter()
        .map(|(key, _, _)| *key)
        .collect();
    assert_eq!(
        keys,
        [
            "local",
            "remote",
            "worktrees",
            "stashes",
            "pulls",
            "tags",
            "submodules"
        ]
    );
    assert!(!probe(&mut stage).shown, "closed, as every section starts");
    toggle(&mut stage, "pulls");
    let seen = probe(&mut stage);
    assert!(seen.shown, "{seen:?}");
    assert_eq!(
        seen.groups,
        [
            ("mine".to_owned(), 1, true),
            ("assigned".to_owned(), 1, true),
            ("review_requested".to_owned(), 2, true),
            ("all".to_owned(), 5, true),
        ]
    );
    assert!(seen.offline, "GitLab's lists were kept from before");
    assert_eq!(
        seen.rows.first().map(String::as_str),
        Some("#40 A landing page for the docs")
    );
    assert!(
        seen.rows
            .contains(&"!12 Relative dates everywhere".to_owned())
    );
    // A group closes from its heading; a remote in a group does not
    // close, and its row has no arrow.
    for group in ["mine", "assigned", "review_requested"] {
        stage.click(ElementId::Name(format!("pulls-group:{group}").into()));
    }
    let seen = probe(&mut stage);
    assert_eq!(seen.groups[0], ("mine".to_owned(), 1, false));
    assert_eq!(seen.groups[3], ("all".to_owned(), 5, true));
    assert!(!stage.shows(row("mine", 0, 40)));
    assert!(stage.shows(row("all", 0, 41)));
    assert!(
        seen.rows
            .contains(&"!12 Relative dates everywhere".to_owned()),
        "GitLab's still open in the group of all: {:?}",
        seen.rows
    );
    let closed = stage.app(|cx| {
        cx.global::<Prefs>()
            .repo_state(&repo)
            .closed_pull_groups
            .clone()
    });
    assert_eq!(closed, ["assigned", "mine", "review_requested"]);
    stage.click(ElementId::Name("pulls-group:mine".into()));
    assert_eq!(probe(&mut stage).groups[0], ("mine".to_owned(), 1, true));
    // Closing the section tells its rows they are out of view.
    toggle(&mut stage, "pulls");
    stage.settle();
    assert!(!probe(&mut stage).shown);
}

/// What is typed in the search box keeps the pull requests whose title,
/// number, branch or author has it, in every group, and the counts say
/// so.
fn the_search_box_keeps_the_pull_requests_that_match() {
    let mut stage = open_section(false);
    stage.click("pulls-search");
    stage.type_text("lexer");
    let seen = probe(&mut stage);
    assert_eq!(seen.search, "lexer");
    assert_eq!(
        seen.rows,
        [
            "#38 Try a cache in the lexer",
            "#38 Try a cache in the lexer"
        ],
        "in the group waiting for review and in all"
    );
    assert_eq!(seen.groups[0], ("mine".to_owned(), 0, true));
    stage.keys("ctrl-a backspace");
    stage.type_text("#12");
    let seen = probe(&mut stage);
    assert_eq!(seen.rows.len(), 2, "{seen:?}");
    assert!(seen.rows.iter().all(|row| row.starts_with("!12")));
    stage.keys("ctrl-a backspace");
    stage.type_text("ada");
    assert_eq!(
        probe(&mut stage).rows.len(),
        3,
        "two of hers in all, one assigned"
    );
}

/// While the pointer rests on a row, its card stands beside it: 500 wide,
/// its left edge 8 over the right edge of the row, its middle at the row's
/// middle; it goes when the pointer leaves, and when the row is clicked.
fn a_rows_card_stands_beside_it_over_the_panels_edge() {
    let mut stage = open_section(false);
    assert!(!stage.shows("hover-card"));
    let (x, y, w, h) = stage.place(row("mine", 0, 40));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    assert!(stage.shows("hover-card"), "the card shows after a rest");
    let (card_x, card_y, card_w, card_h) = stage.place("hover-card");
    assert!((card_w - 500.).abs() < 0.5, "500 wide: {card_w}");
    assert!(
        (card_x - (x + w - 8.)).abs() < 0.5,
        "8 over the row's right edge: {card_x} against {}",
        x + w
    );
    assert!(
        ((card_y + card_h / 2.) - (y + h / 2.)).abs() < 1.,
        "its middle at the row's: {card_y}+{card_h}/2 against {y}+{h}/2"
    );
    stage.point_at((x + w + 700., y));
    stage.pass(Duration::from_secs(1));
    assert!(!stage.shows("hover-card"), "gone with the pointer");
    // Back on the row, a click puts it away.
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    assert!(stage.shows("hover-card"));
    stage.click(row("mine", 0, 40));
    stage.pass(Duration::from_secs(2));
    assert!(
        !stage.shows("hover-card"),
        "a click puts it away while the pointer stays"
    );
    stage.point_at((x + w + 700., y));
    stage.pass(Duration::from_millis(300));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    assert!(stage.shows("hover-card"), "back after the pointer left");
    // A click before the card came: it does not come.
    stage.point_at((x + w + 700., y));
    stage.pass(Duration::from_millis(300));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_millis(200));
    stage.click(row("mine", 0, 40));
    stage.pass(Duration::from_secs(2));
    assert!(
        !stage.shows("hover-card"),
        "a click before it came keeps it away"
    );
}

/// A click on a row shows the pull request in the middle; a file of it
/// shows its changes in the left column and goes back; the close button
/// brings the graph back.
fn a_pull_request_opens_in_the_middle_and_its_close_button_goes_back() {
    let mut stage = open_section(false);
    assert_eq!(stage.workspace().sheet, None);
    stage.click(row("assigned", 0, 41));
    stage.settle();
    assert_eq!(stage.workspace().sheet, Some("pull"));
    let seen = view_probe(&mut stage).expect("the view is there");
    assert_eq!(seen.number, 41);
    give_details(&mut stage);
    let seen = view_probe(&mut stage).unwrap();
    assert_eq!(
        seen.title.as_deref(),
        Some("Quotes and the needle in one pass")
    );
    assert_eq!((seen.files, seen.threads, seen.checks), (3, 2, 3));
    assert!(stage.shows("pull-title") && stage.shows("pull-description"));
    stage.click(("pull-file-row", 0usize));
    assert_eq!(
        view_probe(&mut stage).unwrap().file_open.as_deref(),
        Some("src/lexer.rs")
    );
    stage.click("pull-file-back");
    assert_eq!(view_probe(&mut stage).unwrap().file_open, None);
    stage.click("pull-close");
    assert_eq!(stage.workspace().sheet, None);
    assert!(view_probe(&mut stage).is_none());
    // Opened again, it is the same view, as it was left.
    stage.click(row("mine", 0, 40));
    stage.settle();
    assert_eq!(view_probe(&mut stage).map(|seen| seen.number), Some(40));
}

pub fn give_details(stage: &mut Stage) {
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace
            .read(cx)
            .pull_view()
            .expect("a pull request is shown");
        view.update(cx, |view, cx| view.give(details(), cx));
    });
    stage.settle();
}

/// The button beside the search box opens a dialog that takes a name and
/// a query, says what GitLab leaves out of it, and adds a group of its
/// own that is kept with the repository.
fn a_filter_is_added_from_its_dialog_and_kept_with_the_repository() {
    let mut stage = open_section(false);
    stage.click("pulls-add-filter");
    stage.pass(Duration::from_millis(300));
    assert!(stage.dialog_open());
    stage.type_text("Reviewed");
    stage.click("pull-filter-query");
    stage.type_text("review:approved -label:wip");
    assert!(
        stage.shows("pull-filter-left-out"),
        "GitLab is asked by fewer"
    );
    stage.click("pull-filter-save");
    stage.settle();
    assert!(!stage.dialog_open());
    let seen = probe(&mut stage);
    assert_eq!(seen.filters, ["Reviewed"]);
    assert_eq!(
        seen.groups.last().map(|group| group.0.as_str()),
        Some("custom:0")
    );
    let kept = stage.with_workspace(|workspace, _, cx| {
        let path = workspace.read(cx).session().path;
        cx.global::<Prefs>().repo_state(&path).pull_filters
    });
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].query, "review:approved -label:wip");
}

/// Known checks put a mark on their commit's row, and a branch with an
/// open pull request a mark at the end of its label and that pull
/// request's page and link in its menu; with nothing known there is
/// neither.
fn the_graph_marks_known_checks_and_branches_with_an_open_pull_request() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let mut stage = Stage::open(Start::with(&repo));
    let probe_graph = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).graph().read(cx).checks_probe())
    };
    assert_eq!(probe_graph(&mut stage), (false, 0, 0));
    give(&mut stage, given());
    // Given lists ask nothing: the graph neither.
    let (asking, marked, branches) = probe_graph(&mut stage);
    assert!(!asking && marked == 0);
    assert_eq!(
        branches, 4,
        "wip, pages and feature/tried at GitHub, feature/dates at GitLab; dates-in-log is a fork's"
    );
    give_checks(&mut stage, &repo);
    assert_eq!(probe_graph(&mut stage).1, 3);
    assert!(stage.shows(("checks", 1usize)), "the row of main");
    // A branch's menu, here or on its remote, offers its open pull
    // request's page and link.
    let mark_of = |stage: &mut Stage, name: &str| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).pulls().read(cx).mark_of(name))
    };
    let wip = mark_of(&mut stage, "wip").expect("wip has an open pull request");
    assert!(
        wip.web_url.ends_with(&format!("/pull/{}", wip.number)),
        "{wip:?}"
    );
    assert_eq!(
        mark_of(&mut stage, "origin/wip").map(|mark| mark.number),
        Some(wip.number)
    );
    assert!(mark_of(&mut stage, "main").is_none());
}

/// Marks on three commits of the graph: the tips of wip, pages and main.
fn give_checks(stage: &mut Stage, repo: &std::path::Path) {
    let statuses = [
        ("wip", CiState::Running),
        ("pages", CiState::Success),
        ("main", CiState::Failure),
    ]
    .into_iter()
    .map(|(branch, state)| CommitStatus {
        sha: repos::rev(repo, branch),
        state,
        checks: checks(),
    })
    .collect::<Vec<_>>();
    stage.with_workspace(|workspace, _, cx| {
        let graph = workspace.read(cx).graph();
        graph.update(cx, |view, cx| view.give_checks(statuses, cx));
    });
    stage.settle();
}

/// The pictures: the section with its groups for GitHub and GitLab, a
/// row's card, the dialog that adds a filter, a pull request's view and a
/// file of it, and the graph with the marks of checks and of branches
/// with a pull request.
pub fn pulls(shots: &mut Shots) {
    let mut stage = open_section(true);
    shots.take(&mut stage, "pulls_section", Themes::Two);

    // The card of a row, while the pointer rests on it.
    let (x, y, w, h) = stage.place(row("assigned", 0, 41));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    shots.take(&mut stage, "pulls_card", Themes::Two);
    stage.point_at((x + w + 400., y));

    stage.click("pulls-add-filter");
    stage.pass(Duration::from_millis(600));
    stage.type_text("Reviewed");
    stage.click("pull-filter-query");
    stage.type_text("review:approved -label:wip");
    shots.take(&mut stage, "pulls_filter", Themes::Two);
    stage.click("pull-filter-cancel");
    stage.settle();

    stage.click(row("assigned", 0, 41));
    stage.settle();
    give_details(&mut stage);
    shots.take(&mut stage, "pull_view", Themes::Two);
    // Kept from an hour ago, the provider away: its heading says so.
    stage.with_workspace(|workspace, _, cx| {
        let view = workspace
            .read(cx)
            .pull_view()
            .expect("a pull request is shown");
        let mut kept = details();
        kept.freshness = Freshness {
            fetched: Some("2020-01-03T11:00:00Z".to_owned()),
            offline: true,
            remaining: None,
            reset: None,
        };
        view.update(cx, |view, cx| view.give(kept, cx));
    });
    stage.settle();
    shots.take(&mut stage, "pull_view_offline", Themes::Two);
    give_details(&mut stage);
    // Further down: the thread on a line with its place in the diff, and
    // the merge box.
    stage.window(|window, cx| {
        window.scroll(
            "pull-page",
            ScrollDelta::Pixels(point(px(0.), px(-2000.))),
            cx,
        )
    });
    stage.frame();
    shots.take(&mut stage, "pull_view_threads", Themes::Two);
    // Back up the page, where the files are.
    stage.window(|window, cx| {
        window.scroll(
            "pull-page",
            ScrollDelta::Pixels(point(px(0.), px(4000.))),
            cx,
        )
    });
    stage.frame();
    stage.click(("pull-file-row", 0usize));
    shots.take(&mut stage, "pull_view_file", Themes::Two);
    stage.click("pull-close");
    stage.settle();

    // The graph, the section closed: marks of checks on three rows and of
    // pull requests on three labels, and the checks of a row in its hover.
    toggle(&mut stage, "pulls");
    let repo = stage.with_workspace(|workspace, _, cx| workspace.read(cx).session().path);
    give_checks(&mut stage, &repo);
    shots.take(&mut stage, "graph_checks", Themes::Two);
    let (x, y, w, h) = stage.place(("checks", 1usize));
    stage.point_at((x + w / 2., y + h / 2.));
    stage.pass(Duration::from_secs(2));
    shots.take(&mut stage, "graph_checks_hover", Themes::Two);
    drop(stage);

    // A remote at github.com and no account: the line that invites to
    // connect one, from what the real commands answer.
    repos::fresh();
    let repo = repos::project();
    repos::git(
        &repo,
        &["remote", "add", "upstream", "git@github.com:octo/tools.git"],
    );
    let mut stage = Stage::open(Start::with(&repo).pictures());
    stage.until("the section invites", |stage| {
        probe(stage).invite == ["github.com"]
    });
    toggle(&mut stage, "pulls");
    shots.take(&mut stage, "pulls_invite", Themes::Two);
}
