//! What the window does with accounts at providers: the integrations
//! section of the preferences, its sign-in by code and by token, the SSH
//! key rows; an account's place in the dialogs that clone and make a
//! repository and in the pane that adds a remote, its picture, links to
//! a pull request; and the pictures of them.
//!
//! Nothing here reaches a provider: the answers the commands would give
//! are made up and handed to the views, which take them as they take the
//! commands' own.

use bitbraid_integrations::model::{
    Account, DeviceCode, DevicePoll, Freshness, HostedRepo, Listing, Namespace, NamespaceKind,
    ProviderKind, RepoLink, SignIn,
};
use bitbraid_integrations::{
    replay::{Recording, Replay},
    secrets::MemoryStore,
    services::Services,
};
use bitbraid_ui::{
    providers::{self, Site},
    views::{
        integrations::{IntegrationsProbe, IntegrationsView, Way},
        preferences::Section,
        provider_clone::{ProviderCloneProbe, ProviderCloneView},
        provider_create::ProviderCreateView,
        refs::RemoteRequest,
    },
};
use gpui_kit::Entity;

use crate::{
    repos,
    runner::{Shots, Test, Themes},
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "the_integrations_list_providers_and_their_accounts",
        run: the_integrations_list_providers_and_their_accounts,
    },
    Test {
        name: "a_sign_in_by_code_follows_the_providers_answers",
        run: a_sign_in_by_code_follows_the_providers_answers,
    },
    Test {
        name: "a_token_leaves_its_field_and_what_went_wrong_is_said",
        run: a_token_leaves_its_field_and_what_went_wrong_is_said,
    },
    Test {
        name: "a_clone_is_picked_from_an_accounts_list",
        run: a_clone_is_picked_from_an_accounts_list,
    },
    Test {
        name: "a_repository_is_made_at_an_account",
        run: a_repository_is_made_at_an_account,
    },
    Test {
        name: "the_remote_pane_has_a_tab_per_account_to_fork_with",
        run: the_remote_pane_has_a_tab_per_account_to_fork_with,
    },
    Test {
        name: "pictures_from_the_providers_come_only_when_asked_for",
        run: pictures_from_the_providers_come_only_when_asked_for,
    },
    Test {
        name: "a_token_connects_an_account_at_a_server_of_ones_own",
        run: a_token_connects_an_account_at_a_server_of_ones_own,
    },
    Test {
        name: "a_new_ssh_key_is_made_beside_the_others_and_used",
        run: a_new_ssh_key_is_made_beside_the_others_and_used,
    },
    Test {
        name: "a_repository_made_at_an_account_is_made_here_first",
        run: a_repository_made_at_an_account_is_made_here_first,
    },
    Test {
        name: "the_lists_of_a_gitlab_of_ones_own_are_read_by_the_commands",
        run: the_lists_of_a_gitlab_of_ones_own_are_read_by_the_commands,
    },
    Test {
        name: "the_lists_of_github_are_read_by_the_commands",
        run: the_lists_of_github_are_read_by_the_commands,
    },
    Test {
        name: "a_fork_is_made_from_the_remote_pane_by_the_command",
        run: a_fork_is_made_from_the_remote_pane_by_the_command,
    },
    Test {
        name: "the_key_in_use_is_added_to_a_gitlab_by_the_command",
        run: the_key_in_use_is_added_to_a_gitlab_by_the_command,
    },
    Test {
        name: "a_link_to_a_pull_request_without_a_linked_remote_says_to_connect_one",
        run: a_link_to_a_pull_request_without_a_linked_remote_says_to_connect_one,
    },
    Test {
        name: "a_link_to_a_pull_request_opens_its_view_or_says_it_is_not_there",
        run: a_link_to_a_pull_request_opens_its_view_or_says_it_is_not_there,
    },
];

/// The configuration directory of the window under test.
fn config_dir() -> std::path::PathBuf {
    crate::hermetic::inside()
        .expect("the tests run in scratch surroundings")
        .join("config/bitbraid")
}

/// Put stand-ins in place of the network and the keychain for the
/// provider commands of the window under test: the answers of `replay`,
/// and a keychain in memory. With none, every request is refused.
fn stand_ins(replay: Option<Replay>) -> std::sync::Arc<Replay> {
    let dir = config_dir();
    let replay = std::sync::Arc::new(replay.unwrap_or_else(|| Replay::new(Recording::default())));
    Services::install(
        &dir,
        std::sync::Arc::new(Services::new(
            replay.clone(),
            std::sync::Arc::new(MemoryStore::new()),
            &dir,
        )),
    );
    replay
}

/// Where the first account may make a repository.
fn some_namespaces() -> Vec<Namespace> {
    vec![
        Namespace {
            path: "ada-lovelace".to_owned(),
            name: "Ada Lovelace".to_owned(),
            kind: NamespaceKind::User,
            avatar_url: None,
        },
        Namespace {
            path: "analytical-society".to_owned(),
            name: "The Analytical Society".to_owned(),
            kind: NamespaceKind::Organisation,
            avatar_url: None,
        },
    ]
}

/// Open the dialog that makes a repository and choose the first
/// account's place.
fn open_create_place(stage: &mut Stage) -> Entity<ProviderCreateView> {
    stage.click("init");
    assert!(stage.dialog_open(), "the dialog for a new repository");
    stage.click("form-place-github:github.com:ada-lovelace");
    stage
        .with_shell(|shell, _, _| shell.provider_place::<ProviderCreateView>(0))
        .expect("the first account's place makes a repository there")
}

/// Fill the first account's place in: a name, a description, the
/// organisation.
fn fill_create_place(stage: &mut Stage, view: &Entity<ProviderCreateView>) {
    let view = view.clone();
    stage.window(|window, cx| {
        view.update(cx, |view, cx| {
            view.take_namespaces(Ok(some_namespaces()), cx);
            view.set_name("difference-engine", "Plans for the engine", window, cx);
            view.set_namespace(Some("analytical-society".to_owned()), cx);
        })
    });
    stage.frame();
}

/// A repository as `provider.repos` answers it.
pub fn hosted(owner: &str, name: &str, description: Option<&str>, private: bool) -> HostedRepo {
    HostedRepo {
        id: format!("{owner}/{name}"),
        owner: owner.to_owned(),
        name: name.to_owned(),
        full_name: format!("{owner}/{name}"),
        description: description.map(str::to_owned),
        private,
        parent: None,
        archived: false,
        default_branch: Some("main".to_owned()),
        clone_https: format!("https://github.com/{owner}/{name}.git"),
        clone_ssh: format!("git@github.com:{owner}/{name}.git"),
        web_url: format!("https://github.com/{owner}/{name}"),
        updated: None,
        stars: 0,
    }
}

/// A page of an account's repositories: its own and two organisations'.
pub fn some_repos() -> Listing<HostedRepo> {
    Listing {
        items: vec![
            hosted(
                "ada-lovelace",
                "notes",
                Some("Notes on the analytical engine"),
                false,
            ),
            hosted(
                "ada-lovelace",
                "bernoulli",
                Some("Numbers, the first program"),
                true,
            ),
            hosted(
                "analytical-society",
                "engine",
                Some("The engine itself"),
                false,
            ),
            hosted("analytical-society", "punched-cards", None, true),
            hosted(
                "analytical-society",
                "library",
                Some("Routines on cards"),
                false,
            ),
            hosted("royal-society", "letters", Some("Correspondence"), true),
        ],
        more: false,
        freshness: Freshness::default(),
    }
}

/// An account as `account.list` answers it.
pub fn account(
    kind: ProviderKind,
    web: &str,
    login: &str,
    name: Option<&str>,
    default: bool,
) -> Account {
    let host = web
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let kind_word = match kind {
        ProviderKind::Github => "github",
        ProviderKind::Gitlab => "gitlab",
    };
    Account {
        id: format!("{kind_word}:{host}:{login}"),
        kind,
        web: web.to_owned(),
        api: match kind {
            ProviderKind::Github => "https://api.github.com".to_owned(),
            ProviderKind::Gitlab => format!("{web}/api/v4"),
        },
        login: login.to_owned(),
        name: name.map(str::to_owned),
        avatar_url: None,
        sign_in: SignIn::Token,
        scopes: vec!["repo".to_owned()],
        default,
    }
}

/// Two accounts at GitHub, the first the default, and one at a GitLab of
/// one's own.
pub fn some_accounts() -> Vec<Account> {
    vec![
        account(
            ProviderKind::Github,
            "https://github.com",
            "ada-lovelace",
            Some("Ada Lovelace"),
            true,
        ),
        account(
            ProviderKind::Github,
            "https://github.com",
            "ada-at-work",
            None,
            false,
        ),
        account(
            ProviderKind::Gitlab,
            "https://gitlab.example.org",
            "ada",
            Some("Ada L."),
            true,
        ),
    ]
}

/// A code as `account.sign_in_start` answers it.
fn code() -> DeviceCode {
    DeviceCode {
        user_code: "WDJB-MJHT".to_owned(),
        verification_uri: "https://github.com/login/device".to_owned(),
        device_code: "3584d83530557fdd1f46af8289938c8ef79f9dc5".to_owned(),
        expires_in: 900,
        interval: 5,
    }
}

/// Take these accounts as what `account.list` answered.
pub fn set_accounts(stage: &mut Stage, accounts: Vec<Account>) {
    stage.app(|cx| providers::set_accounts(accounts, cx));
    stage.frame();
}

/// Open the preferences at the integrations, on this provider's page.
fn open_integrations(stage: &mut Stage, site: Site) -> Entity<IntegrationsView> {
    stage.with_shell(|shell, window, cx| shell.show_preferences(Section::Integrations, window, cx));
    stage.frame();
    assert_eq!(stage.shell().preferences_section, Some("integrations"));
    let view = integrations(stage);
    stage.window(|window, cx| view.update(cx, |view, cx| view.choose(site, window, cx)));
    stage.frame();
    view
}

fn integrations(stage: &mut Stage) -> Entity<IntegrationsView> {
    stage.with_shell(|shell, _, cx| shell.preferences_view().read(cx).integrations())
}

fn probe(stage: &mut Stage, view: &Entity<IntegrationsView>) -> IntegrationsProbe {
    stage.app(|cx| view.read(cx).probe(cx))
}

/// The providers are a column of their own; a provider's page has its
/// accounts as cards, the default one marked when there are several, and
/// a card to connect one more. A server of one's own asks for its address.
fn the_integrations_list_providers_and_their_accounts() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    stand_ins(None);
    set_accounts(&mut stage, Vec::new());
    let view = open_integrations(&mut stage, Site::Github);
    for site in Site::ALL {
        assert!(stage.shows(format!("integrations-site-{}", site.key())));
    }
    let seen = probe(&mut stage, &view);
    assert_eq!(seen.site, "github");
    assert!(seen.accounts.is_empty());
    assert!(stage.shows("integrations-connect"), "a card to connect one");
    assert!(
        !stage.shows("integrations-ssh-add"),
        "no key rows without an account"
    );

    set_accounts(&mut stage, some_accounts());
    let seen = probe(&mut stage, &view);
    assert_eq!(seen.accounts, ["*ada-lovelace", "ada-at-work"]);
    assert!(stage.shows(("integrations-default-mark", 0usize)));
    assert!(
        stage.shows(("integrations-default", 1usize)),
        "the other can be made default"
    );
    assert!(!stage.shows(("integrations-default", 0usize)));
    assert!(
        !stage.shows("integrations-connect"),
        "no new card until asked for"
    );
    assert!(
        stage.shows("integrations-ssh-add"),
        "the key rows of a connected provider"
    );
    stage.click("integrations-add-account");
    assert!(stage.shows("integrations-connect"));

    // Disconnecting asks first.
    stage.click(("integrations-disconnect", 1usize));
    let asked = stage.shell().bar.expect("a question in the bar").0;
    assert!(asked.contains("ada-at-work"), "{asked}");
    stage.keys("escape");
    assert_eq!(probe(&mut stage, &view).accounts.len(), 2);

    stage.click("integrations-site-gitlab_server");
    let seen = probe(&mut stage, &view);
    assert_eq!(seen.site, "gitlab_server");
    assert_eq!(seen.accounts, ["*ada"]);
    assert!(
        !stage.shows("integrations-host"),
        "the address is asked for a new one"
    );
    stage.click("integrations-add-account");
    assert!(stage.shows("integrations-host"));

    // A server's address that will not do is said before anything is
    // asked.
    // A server's page has the token's field at once.
    stage.click("integrations-site-github_server");
    stage.click("integrations-token");
    stage.type_text("ghp_not_sent");
    stage.click("integrations-token-save");
    let seen = probe(&mut stage, &view);
    assert_eq!(
        seen.token_error.as_deref(),
        Some("Type the address of the server first.")
    );
}

/// The code is shown with what stands, and every answer of the provider
/// moves it on; closing the dialog ends it.
fn a_sign_in_by_code_follows_the_providers_answers() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    stand_ins(None);
    set_accounts(&mut stage, Vec::new());
    let view = open_integrations(&mut stage, Site::Github);

    let answer = |stage: &mut Stage,
                  start: Result<DeviceCode, String>,
                  polls: Vec<Result<DevicePoll, String>>| {
        let view = view.clone();
        stage.window(|window, cx| {
            view.update(cx, |view, cx| view.show_sign_in(start, polls, window, cx))
        });
        stage.frame();
    };

    answer(&mut stage, Ok(code()), Vec::new());
    assert!(stage.dialog_open());
    let seen = probe(&mut stage, &view);
    assert_eq!(
        seen.sign_in,
        Some(("waiting", Some("WDJB-MJHT".to_owned())))
    );
    assert_eq!(seen.connecting, Some(Way::Code));
    assert!(
        stage.shows("sign-in-code") && stage.shows("sign-in-copy") && stage.shows("sign-in-open")
    );

    // Waiting and slowing down change nothing that is seen but the wait.
    let view_for = view.clone();
    stage.window(|window, cx| {
        view_for.update(cx, |view, cx| {
            view.take_poll(Ok(DevicePoll::Waiting), window, cx);
            view.take_poll(Ok(DevicePoll::SlowDown { interval: 10 }), window, cx);
        })
    });
    stage.frame();
    assert_eq!(
        probe(&mut stage, &view).sign_in.map(|(stage, _)| stage),
        Some("waiting")
    );

    // Turned down at the provider: it can be tried again.
    stage.window(|window, cx| {
        view_for.update(cx, |view, cx| {
            view.take_poll(Ok(DevicePoll::Denied), window, cx)
        })
    });
    stage.frame();
    assert_eq!(
        probe(&mut stage, &view).sign_in.map(|(stage, _)| stage),
        Some("denied")
    );
    assert!(stage.shows("sign-in-again"));
    assert!(!stage.shows("sign-in-code"));

    // Closing it ends it, and the card is as it was.
    stage.click("sign-in-close");
    assert!(!stage.dialog_open());
    let seen = probe(&mut stage, &view);
    assert_eq!(seen.sign_in, None);
    assert_eq!(seen.connecting, None);

    // The code runs out.
    answer(&mut stage, Ok(code()), vec![Ok(DevicePoll::Expired)]);
    assert_eq!(
        probe(&mut stage, &view).sign_in.map(|(stage, _)| stage),
        Some("expired")
    );
    stage.click("sign-in-close");

    // Signed in: the account is read again and the card says so.
    let done = account(
        ProviderKind::Github,
        "https://github.com",
        "ada-lovelace",
        None,
        true,
    );
    answer(
        &mut stage,
        Ok(code()),
        vec![
            Ok(DevicePoll::Waiting),
            Ok(DevicePoll::Done { account: done }),
        ],
    );
    let seen = probe(&mut stage, &view);
    assert_eq!(seen.sign_in.map(|(stage, _)| stage), Some("done"));
    assert_eq!(seen.connecting, None);
    stage.click("sign-in-close");
    assert!(stage.shows("integrations-said"));

    // A provider that offers no sign-in by code: a token it is.
    answer(
        &mut stage,
        Err("Signing in with a code is not available for github.com: use a token.".to_owned()),
        Vec::new(),
    );
    let seen = probe(&mut stage, &view);
    assert_eq!(seen.sign_in.map(|(stage, _)| stage), Some("unavailable"));
    assert_eq!(seen.connecting, Some(Way::Token));
    assert!(stage.shows("sign-in-token"));
    stage.click("sign-in-token");
    assert!(!stage.dialog_open());
    assert_eq!(probe(&mut stage, &view).connecting, Some(Way::Token));
    assert!(stage.shows("integrations-token"));
    // From then on only a token is offered there.
    stage.click("integrations-cancel");
    assert!(stage.shows("integrations-use-token"));
    assert!(!stage.shows("integrations-connect"));
}

/// A token is typed masked, leaves its field when it is saved whatever
/// comes of it, and what the provider said against it stands under the
/// field.
fn a_token_leaves_its_field_and_what_went_wrong_is_said() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    stand_ins(None);
    set_accounts(&mut stage, Vec::new());
    // A server on this machine that is not there: the token is tried
    // and refused without anything leaving the machine.
    let view = open_integrations(&mut stage, Site::GithubServer);
    // What the token needs and the button to the server's page for one
    // wait for an address.
    assert!(!stage.shows("integrations-token-page"));
    assert!(!stage.shows("integrations-token-scopes"));
    stage.click("integrations-host");
    stage.type_text("http://127.0.0.1:9");
    assert!(
        stage.shows("integrations-token-page"),
        "a button to the page that makes one"
    );
    assert!(stage.shows("integrations-token-scopes"));
    stage.click("integrations-token");
    stage.type_text("ghp_0123456789abcdefNOTREAL");
    assert!(probe(&mut stage, &view).token_typed);
    stage.keys("enter");
    // Gone from the field at once, whatever the provider says.
    assert!(!probe(&mut stage, &view).token_typed);
    stage.until("the command answered", |stage| {
        let view = integrations(stage);
        stage
            .app(|cx| view.read(cx).probe(cx))
            .token_error
            .is_some()
    });

    // What the provider says against a token is said under the field.
    let view_for = view.clone();
    stage.window(|_, cx| {
        view_for.update(cx, |view, cx| {
            view.take_token_answer(
                Site::GithubServer,
                Err("The token was refused: the server answered 401 (Bad credentials).".to_owned()),
                cx,
            )
        })
    });
    stage.frame();
    assert!(stage.shows("integrations-token-error"));
    assert_eq!(probe(&mut stage, &view).connecting, Some(Way::Token));
    // Typing again takes it back.
    stage.click("integrations-token");
    stage.type_text("x");
    assert_eq!(probe(&mut stage, &view).token_error, None);
}

/// Open the dialog that clones and choose the first account's place.
fn open_clone_place(stage: &mut Stage) -> Entity<ProviderCloneView> {
    stage.click("clone");
    assert!(stage.dialog_open(), "the clone dialog");
    stage.click("form-place-github:github.com:ada-lovelace");
    stage
        .with_shell(|shell, _, _| shell.provider_place::<ProviderCloneView>(0))
        .expect("the first account's place is a list to clone from")
}

fn clone_probe(stage: &mut Stage, view: &Entity<ProviderCloneView>) -> ProviderCloneProbe {
    stage.app(|cx| view.read(cx).probe(cx))
}

/// Wait until the provider answered what the place asked.
fn answered(stage: &mut Stage, view: &Entity<ProviderCloneView>) {
    stage.until("the provider answered", |stage| {
        let view = view.clone();
        !stage.app(|cx| view.read(cx).probe(cx)).loading
    });
}

/// An account's place lists its repositories under their owners; what
/// is typed narrows them, a filter asks for others, and the one picked
/// gives the address, over SSH or HTTPS, and the folder.
fn a_clone_is_picked_from_an_accounts_list() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    stand_ins(None);
    set_accounts(&mut stage, some_accounts());
    let view = open_clone_place(&mut stage);
    assert_eq!(clone_probe(&mut stage, &view).filter, "own");
    answered(&mut stage, &view);
    let view_for = view.clone();
    stage.app(|cx| view_for.update(cx, |view, cx| view.show_listing(some_repos(), cx)));
    stage.frame();
    let seen = clone_probe(&mut stage, &view);
    let owners: Vec<&str> = seen
        .groups
        .iter()
        .map(|(owner, _)| owner.as_str())
        .collect();
    assert_eq!(
        owners,
        ["ada-lovelace", "analytical-society", "royal-society"]
    );
    assert_eq!(seen.groups[0].1, ["bernoulli", "notes"]);
    assert!(seen.url.is_none() && seen.picked.is_none());

    // Typing narrows the list, by name or description.
    stage.click("clone-search");
    stage.type_text("cards");
    let seen = clone_probe(&mut stage, &view);
    assert_eq!(
        seen.groups,
        [(
            "analytical-society".to_owned(),
            vec!["library".to_owned(), "punched-cards".to_owned()]
        )]
    );
    stage.click("clone-repo-analytical-society/library");
    let seen = clone_probe(&mut stage, &view);
    assert_eq!(seen.picked.as_deref(), Some("analytical-society/library"));
    assert_eq!(
        seen.url.as_deref(),
        Some("git@github.com:analytical-society/library.git")
    );
    assert_eq!(
        seen.path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str()),
        Some("library")
    );
    stage.click("clone-over-https");
    assert_eq!(
        clone_probe(&mut stage, &view).url.as_deref(),
        Some("https://github.com/analytical-society/library.git")
    );

    // Another filter asks the provider again.
    stage.click("clone-filter-starred");
    let seen = clone_probe(&mut stage, &view);
    assert_eq!(seen.filter, "starred");
    answered(&mut stage, &view);
    let view_for = view.clone();
    stage.app(|cx| {
        view_for.update(cx, |view, cx| {
            view.show_problem("GitHub could not be reached.".to_owned(), cx)
        })
    });
    stage.frame();
    assert!(stage.shows("clone-problem"));

    // The form's own place is the address, as it was.
    stage.click("form-place");
    assert!(stage.shows(("form-row", 1usize)));
}

/// An account's place in the dialog that makes a repository: where, its
/// name, whether it is private, and the folder here; a name that will
/// not do is said.
fn a_repository_is_made_at_an_account() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    stand_ins(None);
    set_accounts(&mut stage, some_accounts());
    let view = open_create_place(&mut stage);
    let seen = stage.app(|cx| view.read(cx).probe(cx));
    assert_eq!(seen.creation, Err("Name the repository.".to_owned()));
    assert!(seen.private, "private unless told otherwise");
    fill_create_place(&mut stage, &view);
    stage.click("create-public");
    let seen = stage.app(|cx| view.read(cx).probe(cx));
    assert_eq!(seen.namespaces, ["ada-lovelace", "analytical-society"]);
    let creation = seen.creation.expect("it would do");
    assert_eq!(creation.name, "difference-engine");
    assert_eq!(creation.namespace.as_deref(), Some("analytical-society"));
    assert_eq!(
        creation.description.as_deref(),
        Some("Plans for the engine")
    );
    assert!(!creation.private);
    assert!(creation.path.ends_with("difference-engine"));

    let view_for = view.clone();
    stage.window(|window, cx| {
        view_for.update(cx, |view, cx| view.set_name("no good", "", window, cx))
    });
    stage.frame();
    assert!(stage.app(|cx| view.read(cx).probe(cx)).creation.is_err());
    assert!(stage.shows("create-said"));
}

/// The pane that adds a remote has a tab per account: one the
/// repository's remotes are not linked to has nothing to fork.
fn the_remote_pane_has_a_tab_per_account_to_fork_with() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    stand_ins(None);
    assert!(stage.workspace().loaded);
    set_accounts(&mut stage, some_accounts());
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.remote(RemoteRequest::Add, window, cx)
        })
    });
    stage.frame();
    assert_eq!(stage.workspace().docked, Some("remote"));
    assert!(stage.shows("docked-tab-url"));
    stage.click("docked-tab-github:github.com:ada-lovelace");
    stage.until("the remotes' links are read", |stage| {
        stage.shows("docked-fork-none")
    });
    // Back to the address: the fields are as they were.
    stage.click("docked-tab-url");
    assert!(stage.shows(("docked-field", 1usize)));
}

/// A stand-in for a provider's picture host on this machine: a picture
/// for any path that ends in `.png`, "not found" for the rest, every
/// request noted.
struct PictureHost {
    port: u16,
    heard: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

impl PictureHost {
    fn start() -> Self {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let heard: std::sync::Arc<std::sync::Mutex<Vec<String>>> = Default::default();
        let noted = heard.clone();
        let picture = {
            let image = image::RgbaImage::from_fn(64, 64, |x, y| {
                if (x as i32 - 32).pow(2) + (y as i32 - 26).pow(2) < 169 {
                    image::Rgba([40, 60, 90, 255])
                } else {
                    image::Rgba([120, 180, 240, 255])
                }
            });
            let mut bytes = Vec::new();
            image::DynamicImage::ImageRgba8(image)
                .write_to(
                    &mut std::io::Cursor::new(&mut bytes),
                    image::ImageFormat::Png,
                )
                .unwrap();
            std::sync::Arc::new(bytes)
        };
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (noted, picture) = (noted.clone(), picture.clone());
                std::thread::spawn(move || {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                        match stream.read(&mut byte) {
                            Ok(1) => head.push(byte[0]),
                            _ => return,
                        }
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let path = head.split(' ').nth(1).unwrap_or_default().to_owned();
                    noted.lock().unwrap().push(path.clone());
                    let answer = if path.ends_with(".png") {
                        let mut answer = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n",
                            picture.len()
                        )
                        .into_bytes();
                        answer.extend_from_slice(&picture);
                        answer
                    } else {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    };
                    let _ = stream.write_all(&answer);
                });
            }
        });
        Self { port, heard }
    }

    fn heard(&self) -> Vec<String> {
        self.heard.lock().unwrap().clone()
    }
}

/// Off, as it starts, nothing is asked of a provider's picture host; on,
/// an account's picture is fetched from the address it has, and it is the
/// picture of the owner of a remote linked to the account.
fn pictures_from_the_providers_come_only_when_asked_for() {
    use bitbraid_testkit::{Git, fixtures_out_dir};
    repos::fresh();
    let repo = repos::project();
    let git = Git::new(&repo, fixtures_out_dir().join(".home"));
    git.run([
        "remote",
        "add",
        "origin",
        "https://github.com/ada-lovelace/engine.git",
    ])
    .unwrap();
    let host = PictureHost::start();
    let mut stage = Stage::open(Start::with(&repo));
    stand_ins(None);
    assert!(stage.workspace().loaded);
    let address = format!("http://127.0.0.1:{}/u/ada.png", host.port);
    let mut ada = account(
        ProviderKind::Github,
        "https://github.com",
        "ada-lovelace",
        None,
        true,
    );
    ada.avatar_url = Some(address.clone());
    set_accounts(&mut stage, vec![ada.clone()]);
    let dir = stage
        .app(|cx| providers::drawing_dir(cx))
        .expect("the window draws the repository");
    let remotes = vec![(
        "origin".to_owned(),
        Some("https://github.com/ada-lovelace/engine.git".to_owned()),
    )];
    let link = RepoLink {
        remote: "origin".to_owned(),
        account: ada.id.clone(),
        kind: ProviderKind::Github,
        owner: "ada-lovelace".to_owned(),
        name: "engine".to_owned(),
    };
    stage.app(|cx| providers::set_links(&dir, remotes, vec![link], cx));
    for _ in 0..3 {
        stage.frame();
    }
    assert_eq!(
        stage.app(|cx| providers::owner_picture_address("origin", cx)),
        Some(address.clone()),
        "the remote's owner is the account"
    );
    // Off: nothing is asked, and the remote keeps its mark.
    assert!(
        stage
            .app(|cx| providers::remote_picture("origin", 14., cx))
            .is_none()
    );
    std::thread::sleep(std::time::Duration::from_millis(100));
    stage.frame();
    assert!(
        host.heard().is_empty(),
        "asked while off: {:?}",
        host.heard()
    );

    // On: the picture is fetched once, from its address.
    stage.window(|_, cx| {
        let prefs = cx.global_mut::<bitbraid_ui::prefs::Prefs>();
        prefs.preferences.provider_pictures = true;
        prefs.save_preferences();
        cx.refresh_windows();
    });
    stage.until("the owner's picture is in", |stage| {
        stage.app(|cx| providers::remote_picture("origin/main", 14., cx).is_some())
    });
    assert_eq!(host.heard(), ["/u/ada.png"]);
}

/// A token typed for a GitLab of one's own goes to `account.add_token`,
/// which asks the server whose it is (answered here from what a GitLab CE
/// answered, `crates/integrations/tests/recordings`) and keeps it; the
/// account is read again and has its card.
fn a_token_connects_an_account_at_a_server_of_ones_own() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    let recording = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../integrations/tests/recordings/gitlab/add_token_and_pr_list.json");
    let replay = stand_ins(Some(Replay::load(&recording).unwrap()));
    providers_reload(&mut stage);
    let view = open_integrations(&mut stage, Site::GitlabServer);
    assert!(probe(&mut stage, &view).accounts.is_empty());
    stage.click("integrations-host");
    stage.type_text("http://127.0.0.1:8929");
    stage.click("integrations-token");
    stage.type_text("glpat-not-a-real-token");
    stage.click("integrations-token-save");
    assert!(!probe(&mut stage, &view).token_typed);
    stage.until("the account is connected", |stage| {
        let view = integrations(stage);
        !stage.app(|cx| view.read(cx).probe(cx)).accounts.is_empty()
    });
    let seen = probe(&mut stage, &view);
    assert_eq!(seen.accounts, ["*user-1"]);
    assert_eq!(seen.connecting, None);
    assert_eq!(seen.token_error, None);
    assert!(stage.shows("integrations-said"));
    let asked = replay.asked();
    assert!(
        asked.iter().any(|asked| asked.ends_with("/api/v4/user")),
        "{asked:?}"
    );
    // Nothing of the token is in the accounts' file.
    let written = std::fs::read_to_string(config_dir().join("integrations.toml")).unwrap();
    assert!(!written.contains("glpat-not-a-real-token"));
    assert!(written.contains("user-1"));
}

/// "Generate and add" makes a new key in the scratch home's `.ssh` with
/// ssh-keygen (`ssh.key_generate`), never over one that is there, offers
/// it to the provider (refused here: nothing leaves the machine), and the
/// profile connects with it from then on.
fn a_new_ssh_key_is_made_beside_the_others_and_used() {
    if !repos::on_path("ssh-keygen") {
        println!("skipped: ssh-keygen is not installed");
        return;
    }
    repos::fresh();
    let home = crate::hermetic::inside().unwrap().join("home");
    let ssh = home.join(".ssh");
    let _ = std::fs::remove_dir_all(&ssh);
    let mut stage = Stage::open(Start::bare());
    stand_ins(None);
    set_accounts(&mut stage, some_accounts());
    let view = open_integrations(&mut stage, Site::Github);
    stage.click("integrations-ssh-generate");
    stage.until("the key is made and offered", |stage| {
        let view = integrations(stage);
        stage.app(|cx| view.read(cx).key_said(cx)).is_some()
    });
    let said = stage.app(|cx| view.read(cx).key_said(cx)).unwrap();
    assert!(said.starts_with("A new key is at"), "{said}");
    let made = ssh.join("id_ed25519_bitbraid");
    assert!(made.is_file() && ssh.join("id_ed25519_bitbraid.pub").is_file());
    // A second one goes beside it.
    stage.click("integrations-ssh-generate");
    stage.until("the second key is made", |_| {
        ssh.join("id_ed25519_bitbraid_2.pub").is_file()
    });
    stage.until("the profile connects with it", |stage| {
        let view = integrations(stage);
        stage
            .app(|cx| view.read(cx).probe(cx))
            .ssh_key
            .is_some_and(|(public, _)| public.ends_with("id_ed25519_bitbraid_2.pub"))
    });
    let seen = probe(&mut stage, &view);
    let (_, print) = seen.ssh_key.unwrap();
    assert!(print.is_some_and(|print| print.starts_with("SHA256:")));
    let _ = std::fs::remove_dir_all(&ssh);
}

/// Going ahead in an account's place makes the repository here with a
/// first commit and opens it; its tab then asks the provider to make it
/// there (refused by the stand-ins, which is said).
fn a_repository_made_at_an_account_is_made_here_first() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    stand_ins(None);
    set_accounts(&mut stage, some_accounts());
    let view = open_create_place(&mut stage);
    fill_create_place(&mut stage, &view);
    let path = stage
        .app(|cx| view.read(cx).probe(cx))
        .creation
        .expect("it would do")
        .path;
    let _ = std::fs::remove_dir_all(&path);
    stage.click("create-go");
    assert!(!stage.dialog_open(), "the dialog goes");
    stage.until("the repository is open in a tab", |stage| {
        stage
            .shell()
            .tabs
            .iter()
            .any(|tab| tab.path.as_deref() == Some(path.as_path()))
    });
    assert!(path.join(".git").is_dir());
    stage.until(
        "the provider was asked, and what it said is said",
        |stage| stage.notices().len() >= 2,
    );
    let notices = stage.notices();
    assert!(
        notices
            .iter()
            .any(|notice| notice.contains("Created the repository difference-engine")),
        "{notices:?}"
    );
    assert!(
        notices
            .iter()
            .any(|notice| !notice.contains("Created the repository")),
        "{notices:?}"
    );
    let _ = std::fs::remove_dir_all(&path);
}

/// Connect the account of a recording at a GitLab of one's own through
/// the integrations section, as the user would.
fn connect_by_token(stage: &mut Stage) -> Account {
    open_integrations(stage, Site::GitlabServer);
    stage.click("integrations-host");
    stage.type_text("http://127.0.0.1:8929");
    stage.click("integrations-token");
    stage.type_text("glpat-not-a-real-token");
    stage.click("integrations-token-save");
    stage.until("the account is connected", |stage| {
        stage.app(|cx| !providers::accounts(cx).is_empty())
    });
    stage.app(|cx| providers::accounts(cx)).remove(0)
}

/// The lists of the clone and new-repository dialogs come from the real
/// provider commands, answered from what a GitLab CE answered
/// (`crates/integrations/tests/recordings/gitlab/repos.json`).
fn the_lists_of_a_gitlab_of_ones_own_are_read_by_the_commands() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    let recording = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../integrations/tests/recordings/gitlab/repos.json");
    let replay = stand_ins(Some(Replay::load(&recording).unwrap()));
    providers_reload(&mut stage);
    let account = connect_by_token(&mut stage);
    assert_eq!(account.login, "user-1");
    stage.act(bitbraid_ui::keymap::ShowPreferences);
    assert_eq!(stage.shell().sheet, None);

    // The account's place in the dialog that clones: its own projects.
    stage.click("clone");
    assert!(stage.dialog_open(), "the clone dialog");
    stage.click(format!("form-place-{}", account.id));
    let view = stage
        .with_shell(|shell, _, _| shell.provider_place::<ProviderCloneView>(0))
        .unwrap();
    answered(&mut stage, &view);
    let seen = clone_probe(&mut stage, &view);
    assert_eq!(seen.problem, None, "{:?}", replay.asked());
    assert!(!seen.groups.is_empty(), "{seen:?}");
    let (owner, names) = seen.groups[0].clone();
    let first = format!("{owner}/{}", names[0]);
    stage.click(format!("clone-repo-{first}"));
    let seen = clone_probe(&mut stage, &view);
    assert_eq!(seen.picked.as_deref(), Some(first.as_str()));
    assert!(seen.url.is_some(), "{seen:?}");
    // The starred ones are another list.
    stage.click("clone-filter-starred");
    answered(&mut stage, &view);
    assert_eq!(clone_probe(&mut stage, &view).problem, None);
    stage.click("form-close");

    // The places a repository may be made in.
    stage.click("init");
    stage.click(format!("form-place-{}", account.id));
    let create = stage
        .with_shell(|shell, _, _| shell.provider_place::<ProviderCreateView>(0))
        .unwrap();
    stage.until("the places are read", |stage| {
        let create = create.clone();
        !stage
            .app(|cx| create.read(cx).probe(cx))
            .namespaces
            .is_empty()
    });
    let asked = replay.asked();
    assert!(
        asked
            .iter()
            .any(|asked| asked.contains("/api/v4/namespaces")),
        "{asked:?}"
    );
}

/// The same at GitHub: the clone dialog's lists (the user's own, of the
/// organisations, starred) and the places a repository may be made in
/// come from the real provider commands, answered from what GitHub's
/// published examples say they answer
/// (`crates/integrations/tests/recordings/github/hand_made_repos.json`).
fn the_lists_of_github_are_read_by_the_commands() {
    repos::fresh();
    let mut stage = Stage::open(Start::bare());
    let recording = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../integrations/tests/recordings/github/hand_made_repos.json");
    let replay = stand_ins(Some(Replay::load(&recording).unwrap()));
    let added = bitbraid_integrations::registry()
        .run(
            "account.add_token",
            &mut bitbraid_engine::CommandContext {
                config_dir: Some(config_dir()),
                ..Default::default()
            },
            &serde_json::json!({ "kind": "github", "token": "stand-in-token" }),
        )
        .unwrap();
    let id = added["id"].as_str().unwrap().to_owned();
    providers_reload(&mut stage);
    stage.until("the account is known", |stage| {
        stage.app(|cx| {
            providers::accounts(cx)
                .iter()
                .any(|account| account.id == id)
        })
    });

    stage.click("clone");
    assert!(stage.dialog_open(), "the clone dialog");
    stage.click(format!("form-place-{id}"));
    let view = stage
        .with_shell(|shell, _, _| shell.provider_place::<ProviderCloneView>(0))
        .unwrap();
    answered(&mut stage, &view);
    let seen = clone_probe(&mut stage, &view);
    assert_eq!(seen.problem, None, "{:?}", replay.asked());
    assert!(!seen.groups.is_empty(), "{seen:?}");
    let (owner, names) = seen.groups[0].clone();
    let first = format!("{owner}/{}", names[0]);
    stage.click(format!("clone-repo-{first}"));
    let seen = clone_probe(&mut stage, &view);
    assert_eq!(seen.picked.as_deref(), Some(first.as_str()));
    assert!(seen.url.is_some(), "{seen:?}");
    for filter in ["organisation", "starred"] {
        stage.click(format!("clone-filter-{filter}"));
        answered(&mut stage, &view);
        assert_eq!(clone_probe(&mut stage, &view).problem, None, "{filter}");
    }
    stage.click("form-close");

    stage.click("init");
    stage.click(format!("form-place-{id}"));
    let create = stage
        .with_shell(|shell, _, _| shell.provider_place::<ProviderCreateView>(0))
        .unwrap();
    stage.until("the places are read", |stage| {
        let create = create.clone();
        !stage
            .app(|cx| create.read(cx).probe(cx))
            .namespaces
            .is_empty()
    });
    let asked = replay.asked();
    assert!(
        asked.iter().any(|asked| asked.contains("/user/orgs")),
        "{asked:?}"
    );
}

/// The remote pane's tab of an account at a GitLab of one's own forks
/// the repository of `origin` with the real `provider.fork`, answered from
/// what the local GitLab answered
/// (`crates/integrations/tests/recordings/gitlab/repos_forks_keys.json`),
/// and the fork is added as a remote named after the account's user.
fn a_fork_is_made_from_the_remote_pane_by_the_command() {
    repos::fresh();
    let repo = repos::project();
    repos::git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            "http://127.0.0.1:8929/bb-p-group/bb-p3-writes.git",
        ],
    );
    let mut stage = Stage::open(Start::with(&repo));
    let recording = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../integrations/tests/recordings/gitlab/repos_forks_keys.json");
    let replay = stand_ins(Some(Replay::load(&recording).unwrap()));
    let added = bitbraid_integrations::registry()
        .run(
            "account.add_token",
            &mut bitbraid_engine::CommandContext {
                config_dir: Some(config_dir()),
                ..Default::default()
            },
            &serde_json::json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929",
                                 "token": "stand-in-token" }),
        )
        .unwrap();
    let id = added["id"].as_str().unwrap().to_owned();
    providers_reload(&mut stage);
    stage.until("the account is known", |stage| {
        stage.app(|cx| {
            providers::accounts(cx)
                .iter()
                .any(|account| account.id == id)
        })
    });
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.remote(RemoteRequest::Add, window, cx)
        })
    });
    stage.frame();
    stage.click(format!("docked-tab-{id}"));
    stage.until("the fork can be made", |stage| {
        stage.settle();
        stage.shows("docked-fork-go")
    });
    stage.click("docked-fork-go");
    stage.until("the fork is a remote", |stage| {
        stage.settle();
        repos::git(&repo, &["remote"])
            .lines()
            .any(|name| name == "user-2")
    });
    let address = repos::git(&repo, &["remote", "get-url", "user-2"]);
    assert_eq!(
        address.trim(),
        "http://127.0.0.1:8929/user-2/bb-p3-writes.git"
    );
    assert!(
        replay
            .asked()
            .iter()
            .any(|asked| asked.contains("/projects/bb-p-group%2Fbb-p3-writes/fork")),
        "{:?}",
        replay.asked()
    );
}

/// The SSH rows of a GitLab of one's own add the key in use (here the
/// usual `~/.ssh/id_ed25519.pub` of the scratch home) under the title
/// typed, with the real `provider.ssh_key_add`, answered from what the
/// local GitLab answered; what came of it is said under the rows.
fn the_key_in_use_is_added_to_a_gitlab_by_the_command() {
    // The key of the recording, made by `ssh-keygen -t ed25519` (its
    // private half was thrown away).
    const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAWUi35BfUEiiOjbuDIeFeVHDul6RR5Nf+5RPMLvtWfd bb-p3@example.invalid\n";
    repos::fresh();
    let ssh = crate::hermetic::inside().unwrap().join("home/.ssh");
    let _ = std::fs::remove_dir_all(&ssh);
    std::fs::create_dir_all(&ssh).unwrap();
    std::fs::write(ssh.join("id_ed25519.pub"), KEY).unwrap();
    let mut stage = Stage::open(Start::bare());
    let recording = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../integrations/tests/recordings/gitlab/repos_forks_keys.json");
    let replay = stand_ins(Some(Replay::load(&recording).unwrap()));
    bitbraid_integrations::registry()
        .run(
            "account.add_token",
            &mut bitbraid_engine::CommandContext {
                config_dir: Some(config_dir()),
                ..Default::default()
            },
            &serde_json::json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929",
                                 "token": "stand-in-token" }),
        )
        .unwrap();
    providers_reload(&mut stage);
    stage.until("the account is known", |stage| {
        stage.app(|cx| !providers::accounts(cx).is_empty())
    });
    let view = open_integrations(&mut stage, Site::GitlabServer);
    // The profile connects with that key: a test before may have chosen
    // another for it.
    let private = ssh.join("id_ed25519").to_string_lossy().into_owned();
    stage.app(|cx| view.update(cx, |view, cx| view.set_profile_key(Some(private), cx)));
    stage.until("the key in use is shown", |stage| {
        stage.settle();
        stage.shows("integrations-ssh-add")
    });
    stage.click("integrations-ssh-title");
    stage.keys("ctrl-a backspace");
    stage.type_text("bb-p3 test key");
    stage.click("integrations-ssh-add");
    stage.until("the provider answered", |stage| {
        stage.settle();
        let view = view.clone();
        stage.app(|cx| view.read(cx).key_said(cx)).is_some()
    });
    let said = stage.app(|cx| view.read(cx).key_said(cx)).unwrap();
    assert_eq!(
        said,
        "The key was added to GitLab Self-Managed as \"bb-p3 test key\".",
        "{:?}",
        replay.asked()
    );
    assert!(
        replay
            .asked()
            .iter()
            .any(|asked| asked.contains("/api/v4/user/keys")),
        "{:?}",
        replay.asked()
    );
    let _ = std::fs::remove_dir_all(&ssh);
}

/// Read the accounts again with `account.list`, and wait for them.
fn providers_reload(stage: &mut Stage) {
    stage.app(providers::reload);
    for _ in 0..3 {
        stage.frame();
    }
}

/// The pictures of the integrations section.
pub fn pictures(shots: &mut Shots) {
    repos::fresh();
    let mut stage = Stage::open(Start::bare().pictures());
    stand_ins(None);
    set_accounts(&mut stage, Vec::new());
    let view = open_integrations(&mut stage, Site::Github);
    shots.take(&mut stage, "integrations_none", Themes::Two);

    // Connecting by code.
    let view_for = view.clone();
    stage.window(|window, cx| {
        view_for.update(cx, |view, cx| {
            view.show_sign_in(Ok(code()), Vec::new(), window, cx)
        })
    });
    stage.frame();
    assert!(stage.dialog_open());
    shots.take(&mut stage, "integrations_code", Themes::Two);
    stage.click("sign-in-close");

    // The token refused.
    stage.click("integrations-use-token");
    stage.window(|_, cx| {
        view_for.update(cx, |view, cx| {
            view.take_token_answer(
                Site::Github,
                Err("The token was refused: GitHub answered 401 (Bad credentials).".to_owned()),
                cx,
            )
        })
    });
    stage.frame();
    assert!(stage.shows("integrations-token-error"));
    shots.take(&mut stage, "integrations_token_error", Themes::Two);
    stage.click("integrations-cancel");

    // Two accounts, and the key that goes to them.
    set_accounts(&mut stage, some_accounts());
    stage.window(|_, cx| {
        view_for.update(cx, |view, cx| {
            view.show_key(
                "/home/ada/.ssh/id_ed25519".to_owned(),
                Ok("SHA256:Q8f3n0mWj2cUu2hB1Q9sKzV8qJ4o2xXcF3kWm1pT6Yc".to_owned()),
                cx,
            )
        })
    });
    stage.frame();
    assert_eq!(probe(&mut stage, &view).accounts.len(), 2);
    assert!(stage.shows("integrations-ssh-generate"));
    shots.take(&mut stage, "integrations_accounts", Themes::Two);

    // The key rows, at the end of the page.
    stage.window(|_, cx| {
        view_for.update(cx, |view, cx| {
            view.show_key_said(
                true,
                "The key was added to GitHub as \"BitBraid\".".to_owned(),
                cx,
            )
        })
    });
    stage.frame();
    stage.frame();
    assert!(stage.shows("integrations-ssh-said"));
    shots.take(&mut stage, "integrations_ssh", Themes::Two);

    // A server of one's own: its address, then a token.
    stage.click("integrations-site-gitlab_server");
    stage.click("integrations-add-account");
    stage.click("integrations-host");
    stage.type_text("gitlab.example.org");
    assert!(stage.shows("integrations-token-page"));
    shots.take(&mut stage, "integrations_server", Themes::Two);
}

/// The pictures of an account's place in the dialogs that clone and make
/// a repository.
pub fn clone_pictures(shots: &mut Shots) {
    repos::fresh();
    let mut stage = Stage::open(Start::bare().pictures());
    stand_ins(None);
    set_accounts(&mut stage, some_accounts());
    let view = open_clone_place(&mut stage);
    answered(&mut stage, &view);
    let view_for = view.clone();
    stage.app(|cx| {
        view_for.update(cx, |view, cx| {
            view.show_listing(some_repos(), cx);
            view.pick("ada-lovelace/notes", cx);
        })
    });
    stage.frame();
    assert_eq!(
        clone_probe(&mut stage, &view).picked.as_deref(),
        Some("ada-lovelace/notes")
    );
    shots.take(&mut stage, "clone_provider", Themes::Two);
    stage.click("form-close");

    // Making one at a provider.
    let view = open_create_place(&mut stage);
    fill_create_place(&mut stage, &view);
    assert!(stage.app(|cx| view.read(cx).probe(cx)).creation.is_ok());
    shots.take(&mut stage, "create_provider", Themes::Two);
}

/// The picture of the owner of a remote linked to an account, in REMOTES
/// and on the remote's branch labels in the graph, from a stand-in for
/// the provider's picture host on this machine.
pub fn owner_pictures(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project_with_everything();
    let host = PictureHost::start();
    let state = format!("[repos.{repo:?}]\nclosed_sections = []\n");
    let mut stage = Stage::open(
        Start::with(&repo)
            .state(state)
            .preferences("provider_pictures = true\n")
            .pictures(),
    );
    stand_ins(None);
    assert!(stage.workspace().loaded);
    let mut ada = account(
        ProviderKind::Github,
        "https://github.com",
        "ada-lovelace",
        None,
        true,
    );
    ada.avatar_url = Some(format!("http://127.0.0.1:{}/u/ada.png", host.port));
    set_accounts(&mut stage, vec![ada.clone()]);
    let dir = stage
        .app(|cx| providers::drawing_dir(cx))
        .expect("the window draws the repository");
    let url = repos::git(&repo, &["remote", "get-url", "origin"]);
    let link = RepoLink {
        remote: "origin".to_owned(),
        account: ada.id.clone(),
        kind: ProviderKind::Github,
        owner: "ada-lovelace".to_owned(),
        name: "project".to_owned(),
    };
    stage.app(|cx| {
        providers::set_links(
            &dir,
            vec![("origin".to_owned(), Some(url.trim().to_owned()))],
            vec![link],
            cx,
        )
    });
    stage.until("the owner's picture is in", |stage| {
        stage.app(|cx| providers::remote_picture("origin", 14., cx).is_some())
    });
    for _ in 0..3 {
        stage.frame();
    }
    shots.take(&mut stage, "remote_owner_pictures", Themes::Two);
}

/// The pictures of an account's tab in the pane that adds a remote: with
/// a fork there, and with none.
pub fn fork_pictures(shots: &mut Shots) {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo).pictures());
    stand_ins(None);
    assert!(stage.workspace().loaded);
    set_accounts(&mut stage, some_accounts());
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.remote(RemoteRequest::Add, window, cx)
        })
    });
    stage.frame();
    let account = "github:github.com:ada-lovelace".to_owned();
    let mut fork = hosted("ada-lovelace", "engine", Some("A fork"), false);
    fork.parent = Some("analytical-society/engine".to_owned());
    let links = vec![RepoLink {
        remote: "origin".to_owned(),
        account: account.clone(),
        kind: ProviderKind::Github,
        owner: "analytical-society".to_owned(),
        name: "engine".to_owned(),
    }];
    let (for_none, links_none) = (account.clone(), links.clone());
    stage.with_workspace(|workspace, _, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.show_fork_tab(account, links, Ok(vec![fork]), cx)
        })
    });
    stage.frame();
    assert!(stage.shows(("docked-fork", 0usize)));
    assert!(
        !stage.shows("docked-fork-go"),
        "a fork is there: it is added"
    );
    shots.take(&mut stage, "docked_fork", Themes::Two);

    // None yet: one is made.
    stage.with_workspace(|workspace, _, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.show_fork_tab(for_none, links_none, Ok(Vec::new()), cx)
        })
    });
    stage.frame();
    assert!(stage.shows("docked-fork-go"));
    shots.take(&mut stage, "docked_fork_none", Themes::Two);
}

/// A `bitbraid://` link to the repository at `path`, or at the remote
/// address `url`, naming pull request `number`.
fn pull_link(path: Option<&std::path::Path>, url: Option<&str>, number: u64) -> String {
    use bitbraid_engine::link_cmds::{Link, LinkRepository, LinkTarget, make_link};
    let repository = match (path, url) {
        (Some(path), _) => LinkRepository::Path(path.to_str().unwrap().to_owned()),
        (None, Some(url)) => LinkRepository::Url(url.to_owned()),
        (None, None) => panic!("a link names a repository"),
    };
    make_link(&Link {
        repository,
        target: LinkTarget::PullRequest(number.to_string()),
    })
    .unwrap()
}

/// Follow a link in the window, and let the timers it waits with run
/// until `met` holds.
fn follow_link(stage: &mut Stage, link: &str, what: &str, mut met: impl FnMut(&mut Stage) -> bool) {
    stage.with_shell(|shell, window, cx| shell.open_link(link, window, cx));
    stage.until(what, |stage| {
        stage.settle();
        stage.pass(std::time::Duration::from_millis(100));
        met(stage)
    });
}

/// A link that names a pull request of a repository none of whose
/// remotes is linked to an account opens the repository and says that
/// one has to be connected; no pull request's view is opened.
fn a_link_to_a_pull_request_without_a_linked_remote_says_to_connect_one() {
    repos::fresh();
    let repo = repos::project();
    let mut stage = Stage::open(Start::with(&repo));
    let link = pull_link(Some(&repo), None, 41);
    follow_link(
        &mut stage,
        &link,
        "the notice that no remote is linked",
        |stage| {
            stage.notices().iter().any(|said| {
                said.contains("pull request 41") && said.contains("linked to an account")
            })
        },
    );
    assert_eq!(stage.workspace().sheet, None);
    assert!(crate::pulls::view_probe(&mut stage).is_none());
}

/// A provider that has no pull request 999 of the test repository: it
/// answers that it is not found, as GitHub does, and everything else as
/// it was recorded.
#[derive(Debug)]
struct NoSuchPull {
    replay: Replay,
}

impl bitbraid_integrations::http::Fetch for NoSuchPull {
    fn send(
        &self,
        call: &bitbraid_integrations::http::Call,
        cancel: &std::sync::atomic::AtomicBool,
    ) -> Result<bitbraid_integrations::http::Response, bitbraid_integrations::http::FetchError>
    {
        if call
            .url
            .path()
            .starts_with("/repos/user-1/bitbraid-test-core/pulls/999")
        {
            return Ok(bitbraid_integrations::http::Response {
                status: 404,
                location: None,
                content_type: Some("application/json".to_owned()),
                headers: Vec::new(),
                body: br#"{"message":"Not Found"}"#.to_vec(),
            });
        }
        self.replay.send(call, cancel)
    }
}

/// A link that names a pull request of a repository whose remote is
/// linked to an account at GitHub opens its view in the middle, read from
/// what was recorded of a test repository there, whether the link names
/// the repository by its remote's address or by its path; one the
/// provider does not have opens the view, which says so, and a notice
/// says it too.
fn a_link_to_a_pull_request_opens_its_view_or_says_it_is_not_there() {
    let remote = "https://github.com/user-1/bitbraid-test-core.git";
    let (mut stage, _) = crate::pulls::recorded_through(
        "github",
        "https://github.com",
        remote,
        &["github/pr_list.json", "github/pr_show.json"],
        &[],
        |replay| std::sync::Arc::new(NoSuchPull { replay }),
        false,
    );
    assert_eq!(stage.workspace().sheet, None);

    // By the remote's address.
    let link = pull_link(None, Some(remote), 2);
    follow_link(&mut stage, &link, "pull request 2 is read", |stage| {
        crate::pulls::view_probe(stage).is_some_and(|seen| seen.number == 2 && !seen.loading)
    });
    assert_eq!(stage.workspace().sheet, Some("pull"));
    let seen = crate::pulls::view_probe(&mut stage).unwrap();
    assert_eq!(seen.error, None, "{seen:?}");
    assert!(seen.title.is_some_and(|title| !title.is_empty()));
    stage.click("pull-close");
    assert_eq!(stage.workspace().sheet, None);

    // By the repository's path.
    let repo = repos::work().join("project");
    let link = pull_link(Some(&repo), None, 4);
    follow_link(&mut stage, &link, "pull request 4 is read", |stage| {
        crate::pulls::view_probe(stage).is_some_and(|seen| seen.number == 4 && !seen.loading)
    });
    assert_eq!(crate::pulls::view_probe(&mut stage).unwrap().error, None);

    // One that is not there.
    let link = pull_link(Some(&repo), None, 999);
    follow_link(
        &mut stage,
        &link,
        "the notice that 999 is not there",
        |stage| {
            stage
                .notices()
                .iter()
                .any(|said| said.starts_with("Pull request #999 at GitHub could not be opened"))
        },
    );
    let seen = crate::pulls::view_probe(&mut stage).unwrap();
    assert_eq!(seen.number, 999);
    assert!(
        seen.error
            .as_deref()
            .is_some_and(|error| error.contains("not found")),
        "{seen:?}"
    );
    let said = stage.notices();
    assert!(
        said.iter()
            .any(|said| said.contains("not found at the provider")),
        "{said:?}"
    );
    // Nothing says that pull requests are not built.
    assert!(
        !said.iter().any(|said| said.contains("not built")),
        "{said:?}"
    );
}
