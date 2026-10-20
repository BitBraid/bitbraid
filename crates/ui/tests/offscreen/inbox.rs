//! The inbox: its pinned tab in the tab bar, its tabs, groups, choices
//! and search, its rows' pins, snoozes and merge, its empty states, the
//! status bar's summary; and their pictures. It is given the shapes
//! `inbox.items` answers with, made here, so that nothing depends on a
//! network or an account; what it asks of the commands is answered by
//! stand-ins and looked at.

use bitbraid_integrations::{
    inbox::{
        AccountState, Approval, Inbox, InboxIssue, InboxPull, Involvement, PullGroup, Snoozed,
        WorkInProgress,
    },
    model::{
        Account, CiState, Freshness, Issue, IssueState, Mergeable, Milestone, ProviderKind,
        PullRequest, ReviewState, Reviewer, SignIn,
    },
};
use bitbraid_ui::views::{
    inbox::{InboxProbe, Tab, Whose},
    pulls::calls,
};
use gpui_kit::{ElementId, test::TestWindowExt as _};
use serde_json::{Value, json};

use crate::{
    hermetic::NOW,
    pulls::{label, pull, user},
    repos,
    runner::{Shots, Test, Themes},
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "the_inbox_tab_shows_it_and_a_repository_tab_puts_it_away",
        run: the_inbox_tab_shows_it_and_a_repository_tab_puts_it_away,
    },
    Test {
        name: "without_an_account_the_inbox_invites_to_connect_one",
        run: without_an_account_the_inbox_invites_to_connect_one,
    },
    Test {
        name: "pull_requests_are_in_their_groups_and_groups_fold",
        run: pull_requests_are_in_their_groups_and_groups_fold,
    },
    Test {
        name: "the_tabs_choices_and_search_keep_their_rows",
        run: the_tabs_choices_and_search_keep_their_rows,
    },
    Test {
        name: "pins_and_snoozes_ask_their_commands_and_show_at_once",
        run: pins_and_snoozes_ask_their_commands_and_show_at_once,
    },
    Test {
        name: "merge_asks_how_and_then_the_provider",
        run: merge_asks_how_and_then_the_provider,
    },
    Test {
        name: "a_row_opens_its_repository_and_the_status_bar_opens_a_group",
        run: a_row_opens_its_repository_and_the_status_bar_opens_a_group,
    },
    Test {
        name: "the_inbox_reads_through_its_command_with_the_open_and_recent_ones",
        run: the_inbox_reads_through_its_command_with_the_open_and_recent_ones,
    },
    Test {
        name: "at_150_percent_the_inbox_keeps_its_columns_and_its_buttons_in_reach",
        run: at_150_percent_the_inbox_keeps_its_columns_and_its_buttons_in_reach,
    },
    Test {
        name: "the_inbox_offers_the_workspaces_that_are_kept",
        run: the_inbox_offers_the_workspaces_that_are_kept,
    },
    Test {
        name: "a_forks_pull_request_gets_a_worktree_at_its_fetched_head",
        run: a_forks_pull_request_gets_a_worktree_at_its_fetched_head,
    },
    Test {
        name: "the_inbox_tab_is_a_pinned_tab_as_wide_as_its_mark_and_count",
        run: the_inbox_tab_is_a_pinned_tab_as_wide_as_its_mark_and_count,
    },
];

/// The answer with each pull request `rounds` times over, each a pull
/// request of its own: more than the inbox's tab counts.
fn many(rounds: u64) -> Inbox {
    let mut given = answer("/x");
    let once = given.pulls.clone();
    given.pulls = (0..rounds)
        .flat_map(|round| {
            once.iter().cloned().map(move |mut pull| {
                pull.key = format!("{}-{round}", pull.key);
                pull.pull.number += 1000 * round;
                pull
            })
        })
        .collect();
    given
}

/// The inbox's tab is a pinned tab, as wide as what is in it: its mark,
/// and in a pill how many pull requests and issues wait, the count of the
/// inbox's tab of everything ("99+" past 99; no pill without an answer or
/// with nothing waiting). The repositories' tabs follow it directly, with
/// no room between them, its tinted ground setting it apart. It has no
/// cross, keeps its width shown or not, and names what waits in its tooltip
/// and to assistive technology.
fn the_inbox_tab_is_a_pinned_tab_as_wide_as_its_mark_and_count() {
    let tab_of = |stage: &mut Stage| stage.window(|window, _| window.find("inbox-tab"));
    // Without an answer: the mark alone, in a square of the bar's height.
    let (mut stage, _) = open(None, false, false);
    let (inbox_x, _, alone, high) = stage.place("inbox-tab");
    assert!(
        !stage.shows("inbox-count"),
        "nothing is counted without an answer"
    );
    assert!(
        (34.0..=40.0).contains(&alone) && (alone - high).abs() < 4.0,
        "the mark alone: {alone} by {high}"
    );
    let tab = tab_of(&mut stage);
    assert_eq!(tab.role(), Some(gpui_kit::Role::Tab));
    assert_eq!(tab.label(), Some("Inbox"));
    assert_eq!(tab.selected(), Some(false));
    let (first_x, ..) = stage.place(("tab", 0usize));
    assert!(
        (first_x - (inbox_x + alone)).abs() < 0.01,
        "the first tab right after the inbox's: {first_x}"
    );

    // With an answer: as many as its tab of everything counts.
    drop(stage);
    let (mut stage, _) = open(Some(answer("/x")), false, false);
    let counts = probe(&mut stage).counts;
    let count = |tab: Tab| {
        counts
            .iter()
            .find(|(counted, _)| *counted == tab)
            .map_or(0, |(_, count)| *count)
    };
    let (pulls, issues) = (count(Tab::Pulls), count(Tab::Issues));
    assert_eq!(count(Tab::All), pulls + issues);
    assert!(pulls > 1 && issues > 1, "{counts:?}");
    let said = format!("Inbox: {pulls} pull requests and {issues} issues");
    let tab = tab_of(&mut stage);
    assert_eq!(tab.label(), Some(said.as_str()));
    assert!(stage.shows("inbox-count"));
    let (x, _, wide, _) = stage.place("inbox-tab");
    let (pill_x, _, pill_wide, _) = stage.place("inbox-count");
    assert!(
        pill_x > x + 20.0 && pill_x + pill_wide < x + wide,
        "the pill is after the mark, in the tab: {pill_x} {pill_wide} in {x} {wide}"
    );
    assert!(
        wide > alone && wide < 70.0,
        "as wide as the mark and the count: {wide}"
    );
    // Shown, it is the selected tab, as wide as before; the repository's
    // tab is not selected.
    stage.click("inbox-tab");
    assert_eq!(stage.shell().sheet, Some("inbox"));
    let tab = tab_of(&mut stage);
    assert_eq!(tab.selected(), Some(true));
    assert!((stage.place("inbox-tab").2 - wide).abs() < 0.01);
    let repository = stage.window(|window, _| window.find(("tab", 0usize)));
    assert_eq!(repository.selected(), Some(false));

    // More than 99: the pill says so, and the tab stays narrow.
    stage.with_shell(|shell, _, cx| {
        let inbox = shell.inbox();
        inbox.update(cx, |view, cx| view.give(many(20), cx));
    });
    stage.settle();
    let tab = tab_of(&mut stage);
    assert!(
        tab.label()
            .is_some_and(|label| label.starts_with(&format!("Inbox: {} pull requests", 20 * pulls))),
        "{:?}",
        tab.label()
    );
    let many_wide = stage.place("inbox-tab").2;
    assert!(
        many_wide > wide && many_wide < 80.0,
        "99+ takes a little more: {many_wide}"
    );

    // Nothing waiting: no pill.
    stage.with_shell(|shell, _, cx| {
        let inbox = shell.inbox();
        let mut none = answer("/x");
        none.pulls.clear();
        none.issues.clear();
        inbox.update(cx, |view, cx| view.give(none, cx));
    });
    stage.settle();
    assert!(!stage.shows("inbox-count"));
    let tab = tab_of(&mut stage);
    assert_eq!(tab.label(), Some("Inbox: nothing waits for you"));
}

/// "Check out in a new worktree…" of a fork's pull request fetches its head
/// first (`pr.head`) and opens the pane that adds a worktree at what that
/// answers, in the repository's tab, not at `origin/<its branch>`, which
/// only the fork has.
fn a_forks_pull_request_gets_a_worktree_at_its_fetched_head() {
    use bitbraid_integrations::model::PullHead;

    repos::fresh();
    let library = repos::library();
    let here = library.to_string_lossy().into_owned();
    let mut given = answer(&here);
    let failing = key(GITHUB, "octo/tools", 38);
    for pull in &mut given.pulls {
        if pull.key == failing {
            pull.pull.head_repo = Some("hubot/tools".to_owned());
        }
    }
    let head = repos::rev(&library, "HEAD");
    let answered = PullHead {
        commit: head.clone(),
        branch: "feature/tried".to_owned(),
        remote: None,
        start: head.clone(),
    };
    let (mut stage, _) = open_on(Some(given), true, false);
    calls::stand_in(move |id, _| match id {
        "pr.head" => Some(Ok(json!(answered))),
        _ => None,
    });
    stage.click("inbox-collapse");
    stage.click("inbox-group-failing_checks");
    stage.click(ElementId::Name(
        format!("inbox-more-failing_checks-{failing}").into(),
    ));
    assert!(stage.shows("popup-menu"), "the row's menu");
    // Open at, merge, open the repository, review, check out, and then a
    // new worktree.
    stage.keys("down down down down down");
    let chosen = stage.window(|window, _| {
        gpui_kit::base::test_support::snapshots(window)
            .into_iter()
            .filter(|element| element.role() == Some(gpui_kit::Role::MenuItem))
            .filter(|element| element.selected() == Some(true))
            .filter_map(|element| element.label().map(str::to_owned))
            .collect::<Vec<_>>()
    });
    assert_eq!(chosen, ["Check out in a new worktree…"]);
    stage.keys("enter");
    stage.settle();
    let asked = asked("pr.head");
    assert_eq!(asked.len(), 1, "its head is fetched first");
    assert_eq!(asked[0]["number"], 38);
    assert_eq!(asked[0]["remote"], "origin");
    let seen = stage.workspace();
    assert_eq!(seen.docked, Some("worktree"));
    assert_eq!(
        seen.docked_fields.first().map(String::as_str),
        Some(head.as_str()),
        "it starts at the head: {:?}",
        seen.docked_fields
    );
    calls::stand_down();
}

/// The workspaces kept in their file are the inbox's choices, as the real
/// `workspace.list` answers them.
fn the_inbox_offers_the_workspaces_that_are_kept() {
    repos::fresh();
    let repo = repos::project();
    let file = format!(
        "version = 1\n\n[[workspaces]]\nid = \"team\"\nname = \"Team\"\n\n\
         [[workspaces.repositories]]\npath = {:?}\n\n\
         [[workspaces]]\nid = \"tools\"\nname = \"Tools á\"\n",
        repo.display().to_string()
    );
    let mut stage = Stage::open(Start::with(&repo).config_file("workspaces.toml", file));
    stage.click("inbox-tab");
    stage.until("the workspaces are offered", |stage| {
        stage.settle();
        !probe(stage).workspaces.is_empty()
    });
    assert_eq!(
        probe(&mut stage).workspaces,
        [
            ("team".to_owned(), "Team".to_owned()),
            ("tools".to_owned(), "Tools á".to_owned()),
        ]
    );
}

const GITHUB: &str = "github:github.com:octo";
const GITLAB: &str = "gitlab:gitlab.com:octo";

fn account(id: &str, kind: ProviderKind, web: &str) -> Account {
    Account {
        id: id.to_owned(),
        kind,
        web: web.to_owned(),
        api: format!("{web}/api"),
        login: "octo".to_owned(),
        name: None,
        avatar_url: None,
        sign_in: SignIn::Token,
        scopes: Vec::new(),
        default: true,
    }
}

fn accounts() -> Vec<Account> {
    vec![
        account(GITHUB, ProviderKind::Github, "https://github.com"),
        account(GITLAB, ProviderKind::Gitlab, "https://gitlab.com"),
    ]
}

/// A row of the inbox, from a pull request and what it adds.
fn row(
    pull: PullRequest,
    kind: ProviderKind,
    groups: &[PullGroup],
    lines: (u64, u64),
    involvement: &[Involvement],
    path: Option<&str>,
) -> InboxPull {
    let account = match kind {
        ProviderKind::Github => GITHUB,
        ProviderKind::Gitlab => GITLAB,
    };
    InboxPull {
        key: format!("{account}/pull/{}#{}", pull.repo, pull.number),
        account: account.to_owned(),
        kind,
        additions: Some(lines.0),
        deletions: Some(lines.1),
        mergeable: if groups.contains(&PullGroup::Conflicts) {
            Mergeable::Conflicts
        } else {
            Mergeable::Clean
        },
        approval: Approval::Unknown,
        milestone: None,
        involvement: involvement.to_vec(),
        groups: groups.to_vec(),
        path: path.map(str::to_owned),
        remote: path.map(|_| "origin".to_owned()),
        pinned: false,
        snoozed: None,
        pull,
    }
}

fn issue_row(number: u64, title: &str, updated: &str, path: Option<&str>) -> InboxIssue {
    InboxIssue {
        key: format!("{GITHUB}/issue/octo/tools#{number}"),
        account: GITHUB.to_owned(),
        kind: ProviderKind::Github,
        issue: Issue {
            repo: "octo/tools".to_owned(),
            number,
            title: title.to_owned(),
            state: IssueState::Open,
            author: user("ada"),
            assignees: vec![user("octo")],
            labels: vec![label("bug", "d73a4a")],
            milestone: Some(Milestone {
                id: "1".to_owned(),
                title: "v1.0".to_owned(),
                due: None,
                open: true,
            }),
            created: "2020-01-01T09:00:00Z".to_owned(),
            updated: updated.to_owned(),
            comments: 3,
            web_url: format!("https://github.com/octo/tools/issues/{number}"),
        },
        reactions: Some(2),
        involvement: vec![Involvement::Assigned],
        path: path.map(str::to_owned),
        remote: path.map(|_| "origin".to_owned()),
        pinned: false,
        snoozed: None,
    }
}

/// What the inbox is given: pull requests at both providers in every
/// state, issues, work in progress, both accounts answered.
fn answer(here: &str) -> Inbox {
    use PullGroup::*;
    let mut ready = pull(40, "pages", "A landing page for the docs", "octo");
    ready.ci = CiState::Success;
    ready.reviewers = vec![Reviewer {
        user: user("bob"),
        state: ReviewState::Approved,
    }];
    ready.labels = vec![label("docs", "0075ca")];
    ready.updated = "2020-01-03T11:00:00Z".to_owned();
    let mut review = pull(41, "wip", "Quotes and the needle in one pass", "ada");
    review.ci = CiState::Running;
    review.reviewers = vec![Reviewer {
        user: user("octo"),
        state: ReviewState::Pending,
    }];
    review.labels = vec![
        label("enhancement", "a2eeef"),
        label("lexer", "5319e7"),
        label("speed", "fbca04"),
        label("tests", "c2e0c6"),
    ];
    review.updated = "2020-01-03T09:00:00Z".to_owned();
    let mut failing = pull(38, "feature/tried", "Try a cache in the lexer", "octo");
    failing.ci = CiState::Failure;
    failing.reviewers = vec![Reviewer {
        user: user("cy"),
        state: ReviewState::ChangesRequested,
    }];
    failing.updated = "2020-01-02T12:00:00Z".to_owned();
    let mut draft = pull(37, "dates-in-log", "Dates in the log, a first try", "octo");
    draft.draft = true;
    draft.updated = "2019-12-28T12:00:00Z".to_owned();
    let mut alone = pull(36, "conflicted", "Rename the options", "octo");
    alone.updated = "2019-12-20T12:00:00Z".to_owned();
    let mut gitlab = pull(12, "feature/dates", "Relative dates everywhere", "octo");
    gitlab.repo = "group/tools".to_owned();
    gitlab.ci = CiState::Success;
    gitlab.reviewers = vec![Reviewer {
        user: user("dee"),
        state: ReviewState::Commented,
    }];
    gitlab.updated = "2020-01-03T10:00:00Z".to_owned();
    gitlab.web_url = "https://gitlab.com/group/tools/-/merge_requests/12".to_owned();
    let mut first = row(
        ready,
        ProviderKind::Github,
        &[ReadyToMerge],
        (120, 8),
        &[Involvement::Created],
        Some(here),
    );
    first.milestone = Some(Milestone {
        id: "3".to_owned(),
        title: "v1.0".to_owned(),
        due: None,
        open: true,
    });
    Inbox {
        pulls: vec![
            first,
            row(
                gitlab,
                ProviderKind::Gitlab,
                &[ReadyToMerge, Commented],
                (14, 3),
                &[Involvement::Created],
                None,
            ),
            row(
                review,
                ProviderKind::Github,
                &[ReviewMine],
                (3, 1),
                &[Involvement::ReviewRequested],
                Some(here),
            ),
            row(
                failing,
                ProviderKind::Github,
                &[FailingChecks, ChangesRequested],
                (560, 210),
                &[Involvement::Created],
                Some(here),
            ),
            row(
                draft,
                ProviderKind::Github,
                &[NoReviewers, Draft],
                (40, 0),
                &[Involvement::Created, Involvement::Assigned],
                Some(here),
            ),
            row(
                alone,
                ProviderKind::Github,
                &[NoReviewers, Conflicts],
                (2, 2),
                &[Involvement::Assigned],
                None,
            ),
        ],
        issues: vec![
            issue_row(
                9,
                "The button does nothing",
                "2020-01-03T08:00:00Z",
                Some(here),
            ),
            issue_row(
                11,
                "Show the version on start",
                "2020-01-02T08:00:00Z",
                None,
            ),
        ],
        work: vec![WorkInProgress {
            path: here.to_owned(),
            name: "project".to_owned(),
            branch: Some("main".to_owned()),
            ahead: Some(1),
            behind: Some(0),
            counts: bitbraid_git::wip::Counts {
                modified: 2,
                added: 1,
                deleted: 0,
                renamed: 0,
            },
        }],
        accounts: [GITHUB, GITLAB]
            .into_iter()
            .map(|id| AccountState {
                account: id.to_owned(),
                kind: if id == GITHUB {
                    ProviderKind::Github
                } else {
                    ProviderKind::Gitlab
                },
                web: String::new(),
                login: "octo".to_owned(),
                freshness: Freshness::default(),
                more: false,
                error: None,
            })
            .collect(),
        marks_problem: None,
    }
}

/// The window on a repository, the inbox given `answer` and both
/// accounts known; the inbox shown when `show` says so.
fn open(answer: Option<Inbox>, show: bool, pictures: bool) -> (Stage, std::path::PathBuf) {
    repos::fresh();
    open_on(answer, show, pictures)
}

/// [`open`], with the repositories made already.
fn open_on(answer: Option<Inbox>, show: bool, pictures: bool) -> (Stage, std::path::PathBuf) {
    let repo = repos::project();
    let start = Start::with(&repo);
    let mut stage = Stage::open(if pictures { start.pictures() } else { start });
    if let Some(answer) = answer {
        stage.with_shell(|shell, _, cx| {
            let inbox = shell.inbox();
            inbox.update(cx, |view, cx| {
                view.give(answer, cx);
                view.give_workspaces(vec![("platform".to_owned(), "Platform".to_owned())], cx);
            });
        });
        stage.app(|cx| bitbraid_ui::providers::set_accounts(accounts(), cx));
    }
    if show {
        stage.click("inbox-tab");
    }
    stage.settle();
    (stage, repo)
}

fn probe(stage: &mut Stage) -> InboxProbe {
    stage.with_shell(|shell, _, cx| {
        let inbox = shell.inbox();
        inbox.read(cx).probe(cx)
    })
}

fn choose(stage: &mut Stage, change: impl FnOnce(&mut bitbraid_ui::views::inbox::Choices)) {
    stage.with_shell(|shell, _, cx| {
        let inbox = shell.inbox();
        inbox.update(cx, |view, cx| view.choose(change, cx));
    });
    stage.settle();
}

fn key(account: &str, repo: &str, number: u64) -> String {
    format!("{account}/pull/{repo}#{number}")
}

fn the_inbox_tab_shows_it_and_a_repository_tab_puts_it_away() {
    let (mut stage, repo) = open(Some(answer("/x")), false, false);
    assert_eq!(stage.shell().sheet, None);
    // Pinned, before the repository's tab.
    let (inbox_x, ..) = stage.place("inbox-tab");
    let (tab_x, ..) = stage.place(ElementId::from(("tab-name", 0usize)));
    assert!(inbox_x < tab_x, "{inbox_x} {tab_x}");
    stage.click("inbox-tab");
    assert_eq!(stage.shell().sheet, Some("inbox"));
    assert!(stage.shows("inbox-page"));
    assert!(stage.shows("inbox-tab-pulls"));
    // The repository's tab puts it away.
    stage.click(ElementId::from(("tab-name", 0usize)));
    assert_eq!(stage.shell().sheet, None);
    assert_eq!(stage.shell().tabs[0].path.as_deref(), Some(repo.as_path()));
    assert!(!stage.shows("inbox-page"));
}

fn without_an_account_the_inbox_invites_to_connect_one() {
    let (mut stage, _) = open(None, true, false);
    assert!(stage.shows("inbox-no-account"), "the card that invites");
    assert!(
        stage.shows("inbox-summary-connect"),
        "the status bar says so"
    );
    stage.click("inbox-connect");
    assert_eq!(stage.shell().sheet, Some("preferences"));
    assert_eq!(stage.shell().preferences_section, Some("integrations"));
}

fn pull_requests_are_in_their_groups_and_groups_fold() {
    let (mut stage, _) = open(Some(answer("/x")), true, false);
    let seen = probe(&mut stage);
    assert_eq!(seen.tab, Tab::Pulls);
    let groups: Vec<(PullGroup, usize)> = seen
        .groups
        .iter()
        .map(|(group, _, rows)| (*group, rows.len()))
        .collect();
    assert_eq!(
        groups,
        [
            (PullGroup::ReadyToMerge, 2),
            (PullGroup::ReviewMine, 1),
            (PullGroup::NoReviewers, 2),
            (PullGroup::FailingChecks, 1),
            (PullGroup::Conflicts, 1),
            (PullGroup::ChangesRequested, 1),
            (PullGroup::Commented, 1),
            (PullGroup::Draft, 1),
        ]
    );
    assert!(seen.groups.iter().all(|(_, open, _)| *open));
    assert!(
        stage.shows(ElementId::Name(
            format!(
                "inbox-pull-ready_to_merge-{}",
                key(GITHUB, "octo/tools", 40)
            )
            .into()
        ))
    );
    // A heading folds its group, and opens it again.
    stage.click("inbox-group-ready_to_merge");
    assert!(!probe(&mut stage).groups[0].1);
    assert!(
        !stage.shows(ElementId::Name(
            format!(
                "inbox-pull-ready_to_merge-{}",
                key(GITHUB, "octo/tools", 40)
            )
            .into()
        ))
    );
    stage.click("inbox-group-ready_to_merge");
    assert!(probe(&mut stage).groups[0].1);
    stage.click("inbox-collapse");
    assert!(probe(&mut stage).groups.iter().all(|(_, open, _)| !*open));
    stage.click("inbox-expand");
    assert!(probe(&mut stage).groups.iter().all(|(_, open, _)| *open));
    // Every answer fresh: no mark; one kept from before: the mark.
    assert!(!stage.shows("inbox-offline"));
    let mut kept = answer("/x");
    if let Some(state) = kept.accounts.get_mut(0) {
        state.freshness.offline = true;
    }
    stage.with_shell(|shell, _, cx| {
        let inbox = shell.inbox();
        inbox.update(cx, |view, cx| view.give(kept, cx));
    });
    stage.settle();
    assert!(stage.shows("inbox-offline"));
    assert_eq!(
        seen.counts,
        [
            (Tab::Pulls, 6),
            (Tab::Issues, 2),
            (Tab::Work, 1),
            (Tab::All, 8),
            (Tab::Snoozed, 0)
        ]
    );
}

fn the_tabs_choices_and_search_keep_their_rows() {
    let (mut stage, _) = open(Some(answer("/x")), true, false);
    stage.click("inbox-tab-issues");
    let seen = probe(&mut stage);
    assert_eq!(seen.tab, Tab::Issues);
    assert_eq!(seen.rows.len(), 2);
    assert!(stage.shows("inbox-table-list"));
    stage.click("inbox-tab-all");
    assert_eq!(probe(&mut stage).rows.len(), 8);
    stage.click("inbox-tab-work");
    assert_eq!(probe(&mut stage).work, ["/x"]);
    assert!(stage.shows("inbox-table-work"));
    stage.click("inbox-tab-snoozed");
    assert!(stage.shows("inbox-nothing"));
    stage.click("inbox-tab-pulls");
    // The account: only GitLab's.
    choose(&mut stage, |choices| {
        choices.account = Some(GITLAB.to_owned())
    });
    let seen = probe(&mut stage);
    assert_eq!(
        seen.groups
            .iter()
            .flat_map(|(_, _, rows)| rows.iter().cloned())
            .collect::<Vec<_>>(),
        [
            key(GITLAB, "group/tools", 12),
            key(GITLAB, "group/tools", 12)
        ]
    );
    choose(&mut stage, |choices| choices.account = None);
    // Whose: waiting for my review.
    choose(&mut stage, |choices| choices.whose = Whose::ReviewRequested);
    let seen = probe(&mut stage);
    assert_eq!(seen.groups.len(), 1);
    assert_eq!(seen.groups[0].0, PullGroup::ReviewMine);
    choose(&mut stage, |choices| choices.whose = Whose::Mine);
    // The labels: those with "docs".
    choose(&mut stage, |choices| {
        choices.label = Some("docs".to_owned())
    });
    assert_eq!(probe(&mut stage).groups.len(), 1);
    choose(&mut stage, |choices| choices.label = None);
    // The search takes a number.
    stage.click("inbox-search");
    stage.type_text("#38");
    stage.settle();
    let seen = probe(&mut stage);
    let keys: Vec<String> = seen
        .groups
        .iter()
        .flat_map(|(_, _, rows)| rows.iter().cloned())
        .collect();
    assert_eq!(
        keys,
        [key(GITHUB, "octo/tools", 38), key(GITHUB, "octo/tools", 38)]
    );
    stage.keys("ctrl-a backspace");
    stage.type_text("needle");
    stage.settle();
    assert_eq!(probe(&mut stage).groups.len(), 1);
    // Whose an issue is means another thing: switching to the issues
    // leaves "waiting for my review" for "mine".
    choose(&mut stage, |choices| choices.whose = Whose::ReviewRequested);
    stage.click("inbox-tab-issues");
    assert_eq!(probe(&mut stage).choices.whose, Whose::Mine);
}

/// Stand-ins for the commands that keep the marks: each answers the
/// marks as they would be.
fn marks_stand_in() {
    use std::{cell::RefCell, rc::Rc};
    /// The pins, and the snoozes with their ends.
    type Kept = (Vec<String>, Vec<(String, Option<i64>)>);
    let kept: Rc<RefCell<Kept>> = Rc::default();
    calls::stand_in(move |id, params| {
        let key = params["key"].as_str().unwrap_or_default().to_owned();
        let mut kept = kept.borrow_mut();
        match id {
            "inbox.pin" => kept.0.push(key),
            "inbox.unpin" => kept.0.retain(|known| *known != key),
            "inbox.snooze" => {
                let until = match params["for"].as_str() {
                    Some("hour") => Some(NOW + 3600),
                    _ => None,
                };
                kept.1.retain(|(known, _)| *known != key);
                kept.1.push((key, until));
            }
            "inbox.unsnooze" => kept.1.retain(|(known, _)| *known != key),
            _ => return None,
        }
        Some(Ok(json!({
            "version": 1,
            "pin": kept.0.iter().map(|key| json!({ "key": key, "since": NOW })).collect::<Vec<Value>>(),
            "snooze": kept.1.iter().map(|(key, until)| json!({ "key": key, "since": NOW, "until": until })).collect::<Vec<Value>>(),
        })))
    });
}

fn asked(id: &str) -> Vec<Value> {
    calls::asked()
        .into_iter()
        .filter(|(asked, _)| asked == id)
        .map(|(_, params)| params)
        .collect()
}

fn pins_and_snoozes_ask_their_commands_and_show_at_once() {
    let (mut stage, _) = open(Some(answer("/x")), true, false);
    marks_stand_in();
    let draft = key(GITHUB, "octo/tools", 37);
    // Only the group it is looked at in is open; its row under the
    // pointer shows the pin and the snooze.
    stage.click("inbox-collapse");
    stage.click("inbox-group-no_reviewers");
    let hover = |stage: &mut Stage| {
        let (x, y, w, h) = stage.place(ElementId::Name(
            format!("inbox-pull-no_reviewers-{}", key(GITHUB, "octo/tools", 37)).into(),
        ));
        stage.point_at((x + w / 2., y + h / 2.));
        stage.frame();
    };
    hover(&mut stage);
    // Pinned: first in its groups.
    stage.click(ElementId::Name(
        format!("inbox-pin-no_reviewers-{draft}").into(),
    ));
    stage.settle();
    assert_eq!(asked("inbox.pin"), [json!({ "key": draft })]);
    let seen = probe(&mut stage);
    assert_eq!(seen.pinned, std::slice::from_ref(&draft));
    let no_reviewers = seen
        .groups
        .iter()
        .find(|(group, _, _)| *group == PullGroup::NoReviewers)
        .unwrap();
    assert_eq!(no_reviewers.2[0], draft);
    // Unpinned again.
    hover(&mut stage);
    stage.click(ElementId::Name(
        format!("inbox-pin-no_reviewers-{draft}").into(),
    ));
    stage.settle();
    assert_eq!(asked("inbox.unpin").len(), 1);
    assert!(probe(&mut stage).pinned.is_empty());
    // Snoozed for an hour from its menu: out of its groups, in the
    // snoozed tab.
    hover(&mut stage);
    stage.click(ElementId::Name(
        format!("inbox-snooze-no_reviewers-{draft}").into(),
    ));
    assert!(stage.shows("popup-menu"), "the snooze's choices");
    stage.keys("down down enter");
    stage.settle();
    assert_eq!(
        asked("inbox.snooze"),
        [json!({ "key": draft, "for": "hour" })]
    );
    let seen = probe(&mut stage);
    assert_eq!(seen.snoozed, std::slice::from_ref(&draft));
    assert!(
        seen.groups
            .iter()
            .all(|(_, _, rows)| !rows.contains(&draft))
    );
    assert_eq!(
        seen.counts.iter().find(|(tab, _)| *tab == Tab::Snoozed),
        Some(&(Tab::Snoozed, 1))
    );
    stage.click("inbox-tab-snoozed");
    assert_eq!(probe(&mut stage).rows, std::slice::from_ref(&draft));
    // Brought back from there.
    stage.click(ElementId::Name(
        format!("inbox-unsnooze-list-{draft}").into(),
    ));
    stage.settle();
    assert_eq!(asked("inbox.unsnooze"), [json!({ "key": draft })]);
    assert!(probe(&mut stage).snoozed.is_empty());
    calls::stand_down();
}

fn merge_asks_how_and_then_the_provider() {
    let (mut stage, _) = open(Some(answer("/x")), true, false);
    calls::stand_in(|id, _| match id {
        "pr.merge" => Some(Ok(json!({ "merged": true, "sha": "abc", "message": null }))),
        _ => None,
    });
    let ready = key(GITHUB, "octo/tools", 40);
    stage.click(ElementId::Name(
        format!("inbox-action-ready_to_merge-{ready}").into(),
    ));
    assert_eq!(probe(&mut stage).merging, Some(ready.clone()));
    assert!(asked("pr.merge").is_empty(), "nothing merged before asking");
    stage.click("inbox-merge-method-Squash");
    stage.click("inbox-merge-go");
    stage.settle();
    assert_eq!(
        asked("pr.merge"),
        [json!({
            "account": GITHUB, "owner": "octo", "name": "tools", "number": 40,
            "method": "squash", "delete_branch": false,
        })]
    );
    assert_eq!(probe(&mut stage).merging, None);
    // Refused: the dialog stays and says why.
    calls::stand_in(|id, _| match id {
        "pr.merge" => Some(Err("provider: the base branch is protected".to_owned())),
        _ => None,
    });
    stage.click(ElementId::Name(
        format!("inbox-action-ready_to_merge-{ready}").into(),
    ));
    stage.click("inbox-merge-go");
    stage.settle();
    assert!(stage.shows("inbox-merge-error"));
    stage.click("inbox-merge-cancel");
    assert_eq!(probe(&mut stage).merging, None);
    calls::stand_down();
}

fn a_row_opens_its_repository_and_the_status_bar_opens_a_group() {
    repos::fresh();
    let library = repos::library();
    let here = library.to_string_lossy().into_owned();
    let (mut stage, repo) = open_on(Some(answer(&here)), true, false);
    assert_eq!(stage.shell().tabs.len(), 1);
    // A pull request whose checks failed, at a repository here: its
    // button opens the repository.
    stage.click("inbox-collapse");
    stage.click("inbox-group-failing_checks");
    let failing = key(GITHUB, "octo/tools", 38);
    stage.click(ElementId::Name(
        format!("inbox-action-failing_checks-{failing}").into(),
    ));
    stage.settle();
    let shell = stage.shell();
    assert_eq!(shell.sheet, None, "the repository's tab is shown");
    assert_eq!(shell.tabs.len(), 2);
    assert_eq!(
        shell.tabs[shell.active].path.as_deref(),
        Some(library.as_path())
    );
    assert_ne!(library, repo);
    // The status bar: the first urgent group's first.
    let status = probe(&mut stage).status;
    assert_eq!(
        status.as_deref(),
        Some("octo/tools#40 can be merged, and 1 more")
    );
    stage.click("inbox-summary-button");
    stage.settle();
    stage.click("inbox-summary-conflicts");
    stage.settle();
    assert_eq!(stage.shell().sheet, Some("inbox"));
    let seen = probe(&mut stage);
    let open: Vec<PullGroup> = seen
        .groups
        .iter()
        .filter(|(_, open, _)| *open)
        .map(|(group, _, _)| *group)
        .collect();
    assert_eq!(open, [PullGroup::Conflicts]);
}

fn the_inbox_reads_through_its_command_with_the_open_and_recent_ones() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    calls::stand_in(|id, _| match id {
        "inbox.items" => Some(Ok(serde_json::to_value(answer("/x")).unwrap())),
        // As the command answers.
        "workspace.list" => Some(Ok(json!({
            "workspaces": [{ "id": "platform", "name": "Platform", "repositories": [] }],
            "read_only": false
        }))),
        _ => None,
    });
    stage.app(|cx| bitbraid_ui::providers::set_accounts(accounts(), cx));
    stage.settle();
    stage.until("the inbox is read", |stage| {
        stage.settle();
        !probe(stage).groups.is_empty()
    });
    let params = asked("inbox.items");
    assert!(!params.is_empty());
    let paths = params[0]["paths"].as_array().unwrap();
    assert!(
        paths
            .iter()
            .any(|path| path.as_str() == Some(repo.to_string_lossy().as_ref())),
        "{params:?}"
    );
    assert!(params[0]["workspace"].is_null());
    // Shown, it asks for the workspaces; a workspace chosen reads again
    // with it.
    stage.click("inbox-tab");
    stage.settle();
    assert!(!asked("workspace.list").is_empty());
    assert_eq!(
        probe(&mut stage).workspaces,
        [("platform".to_owned(), "Platform".to_owned())]
    );
    stage.click("inbox-refresh");
    stage.settle();
    let params = asked("inbox.items");
    assert_eq!(params.last().unwrap()["fresh"], true);
    calls::stand_down();
}

/// The inbox's pictures: every tab, a folded group, the empty states,
/// an account that could not be asked, the merge dialog and the status
/// bar's panel.
pub fn pictures(shots: &mut Shots) {
    // The inbox's tab with what waits counted, while a repository is
    // shown; and with more than it counts.
    let (mut stage, _) = open(Some(answer("/x")), false, true);
    shots.take(&mut stage, "inbox_tab", Themes::All);
    stage.with_shell(|shell, _, cx| {
        let inbox = shell.inbox();
        inbox.update(cx, |view, cx| view.give(many(20), cx));
    });
    stage.settle();
    shots.take(&mut stage, "inbox_tab_many", Themes::Two);
    drop(stage);
    let (mut stage, _) = open(Some(answer("/x")), true, true);
    shots.take(&mut stage, "inbox_pulls", Themes::Two);
    stage.window(|window, cx| {
        window.scroll(
            "inbox-list",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(0.), gpui_kit::px(-700.))),
            cx,
        )
    });
    stage.frame();
    shots.take(&mut stage, "inbox_pulls_lower", Themes::Two);
    stage.click("inbox-collapse");
    shots.take(&mut stage, "inbox_folded", Themes::Two);
    stage.click("inbox-expand");
    stage.click("inbox-tab-issues");
    shots.take(&mut stage, "inbox_issues", Themes::Two);
    stage.click("inbox-tab-work");
    shots.take(&mut stage, "inbox_work", Themes::Two);
    stage.click("inbox-tab-all");
    shots.take(&mut stage, "inbox_all", Themes::Two);
    // Snoozed: one until it is brought back, one until a time.
    let mut snoozed = answer("/x");
    if let Some(pull) = snoozed.pulls.get_mut(4) {
        pull.snoozed = Some(Snoozed {
            since: NOW,
            until: Some(NOW + 3 * 3600),
        });
    }
    if let Some(issue) = snoozed.issues.get_mut(1) {
        issue.snoozed = Some(Snoozed {
            since: NOW,
            until: None,
        });
    }
    if let Some(pull) = snoozed.pulls.get_mut(0) {
        pull.pinned = true;
    }
    stage.with_shell(|shell, _, cx| {
        let inbox = shell.inbox();
        inbox.update(cx, |view, cx| view.give(snoozed, cx));
    });
    stage.click("inbox-tab-snoozed");
    shots.take(&mut stage, "inbox_snoozed", Themes::Two);
    stage.click("inbox-tab-pulls");
    // The merge dialog.
    stage.click(ElementId::Name(
        format!(
            "inbox-action-ready_to_merge-{}",
            key(GITHUB, "octo/tools", 40)
        )
        .into(),
    ));
    shots.take(&mut stage, "inbox_merge", Themes::Two);
    stage.click("inbox-merge-cancel");
    // The status bar's panel.
    stage.click("inbox-summary-button");
    stage.settle();
    shots.take(&mut stage, "inbox_summary", Themes::Two);
    stage.click("inbox-summary-ready_to_merge");
    stage.settle();
    // Nothing found, and an account that could not be asked.
    let mut empty = answer("/x");
    empty.pulls.clear();
    empty.issues.clear();
    if let Some(state) = empty.accounts.get_mut(1) {
        state.error = Some("provider: GitLab could not be reached".to_owned());
    }
    if let Some(state) = empty.accounts.get_mut(0) {
        state.freshness.offline = true;
    }
    stage.with_shell(|shell, _, cx| {
        let inbox = shell.inbox();
        inbox.update(cx, |view, cx| view.give(empty, cx));
    });
    stage.settle();
    shots.take(&mut stage, "inbox_nothing", Themes::Two);
    drop(stage);
    // No account at all.
    let (mut stage, _) = open(None, true, true);
    shots.take(&mut stage, "inbox_no_account", Themes::Two);
}

/// At 150 % a window 1280 wide is 853 points, less than the inbox's columns
/// take. The choices go on to another line where they have no room, and the
/// table keeps its columns, the title's included, and scrolls sideways to
/// its buttons: no column goes under another, and no choice or button out
/// of the window.
fn at_150_percent_the_inbox_keeps_its_columns_and_its_buttons_in_reach() {
    use crate::stage::WIDTH;
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).preferences("zoom = 1.5\n"));
    stage.with_shell(|shell, _, cx| {
        let inbox = shell.inbox();
        inbox.update(cx, |view, cx| view.give(answer("/x"), cx));
    });
    stage.app(|cx| bitbraid_ui::providers::set_accounts(accounts(), cx));
    stage.click("inbox-tab");
    stage.settle();
    let right = |stage: &mut Stage, id: ElementId| {
        let (left, _, width, _) = stage.place(id);
        (left, left + width)
    };
    for id in [
        "inbox-workspace",
        "inbox-account",
        "inbox-labels",
        "inbox-milestones",
        "inbox-whose",
    ] {
        let (left, end) = right(&mut stage, id.into());
        assert!(left >= 0.0 && end <= WIDTH + 0.5, "{id} at {left}..{end}");
    }
    let row = ElementId::Name(
        format!(
            "inbox-pull-ready_to_merge-{}",
            key(GITHUB, "octo/tools", 40)
        )
        .into(),
    );
    let action = ElementId::Name(
        format!(
            "inbox-action-ready_to_merge-{}",
            key(GITHUB, "octo/tools", 40)
        )
        .into(),
    );
    // The title keeps a column of its own: the row is as wide as all the
    // columns at their widths and the title's at its least.
    let (_, _, row_width, _) = stage.place(row);
    assert!(row_width / 1.5 >= 1100.0, "the row is {row_width} wide");
    // The button at the row's end is reached sideways.
    stage.window(|window, cx| {
        window.scroll(
            "inbox-list",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(gpui_kit::px(-2000.), gpui_kit::px(0.))),
            cx,
        )
    });
    stage.frame();
    let (left, end) = right(&mut stage, action);
    assert!(
        left >= 0.0 && end <= WIDTH + 0.5,
        "the button at {left}..{end}"
    );
}
