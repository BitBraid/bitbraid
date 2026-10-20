//! Application entry: GPUI setup, globals, the main window.

use gpui_kit::component::ActiveTheme;
use gpui_kit::*;

use crate::{keymap, prefs, shell::Shell, theme};

// Icons beyond the component defaults, from the same Lucide set: the marks
// on reference labels in the graph, the toolbar, the sections of the
// references panel, and the controls of the file and changes views.
gpui_kit::assets::icon_assets!(
    ExtraIcons,
    [
        Laptop,
        Cloud,
        Tag,
        Archive,
        ArchiveRestore,
        CloudDownload,
        ArrowDownToLine,
        ArrowUpFromLine,
        ArrowUpToLine,
        ListFilter,
        CircleStop,
        GitBranch,
        GitCommitHorizontal,
        FolderTree,
        Package,
        Layers,
        Trash,
        List,
        ListTree,
        ArrowUpDown,
        ArrowDownAZ,
        ArrowDownZA,
        Pencil,
        FilePen,
        Columns2,
        FoldVertical,
        Dot,
        Save,
        Rows3,
        TextWrap,
        Pilcrow,
        FileDiff,
        Clock,
        CircleChevronLeft,
        CircleChevronRight,
        SquareCheckBig,
        Logs,
        ShieldCheck,
        ShieldAlert,
        ShieldX,
        ShieldQuestionMark,
        Workflow,
        HardDrive,
        // The maintenance section of a repository.
        Broom,
        Keyboard,
        UserRound,
        Lock,
        Bell,
        // The sections of the preferences.
        ArrowLeftFromLine,
        Wrench,
        CodeXml,
        FlaskConical,
        FolderCog,
        Users,
        KeyRound,
        Signature,
        Webhook,
        // The mark of a reference shown alone, in the left panel.
        Target,
        // The page of repositories.
        ChevronsDownUp,
        ArrowRightLeft,
        // Workspaces on it: taking one in from its file and writing it
        // out. The icons a workspace can be given are in the list
        // already (`views::repositories::ICONS`).
        FileInput,
        FileOutput,
        // Where a clone comes from, in the dialog that clones.
        Link,
        // The providers: the section of the preferences, the marks of
        // GitLab and of servers of one's own, a repository's lock, a
        // repository in the list a clone is chosen from.
        Plug,
        FolderGit2,
        Server,
        LockOpen,
        GitFork,
        Book,
        // The terminal inside the window: the toolbar's button, the
        // panel's heading and the section of the preferences.
        Terminal,
        // The signs of its completions: a command, a word of one, an
        // option, a remote, a file, a folder.
        SquareTerminal,
        CornerDownRight,
        SlidersHorizontal,
        Globe,
        File,
        Folder,
        // Pull requests: the section's mark, where each stands, the checks
        // of a commit, a filter to add, a list kept while offline, a
        // milestone.
        GitPullRequest,
        GitPullRequestDraft,
        GitPullRequestClosed,
        GitMerge,
        Circle,
        CircleDot,
        CircleDashed,
        ListFilterPlus,
        CloudOff,
        Milestone,
        // The inbox: a row's pin and snooze, the marks of its groups.
        Pin,
        PinOff,
        AlarmClock,
        AlarmClockOff,
        UserPlus,
        Hourglass,
        MessageSquare
    ]
);

/// The component icons plus ours: everything the application draws that
/// is not text comes from here, and none of it from the machine.
#[derive(Debug, Clone, Copy)]
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<std::borrow::Cow<'static, [u8]>>> {
        if let Some(bytes) = ExtraIcons.load(path)? {
            return Ok(Some(bytes));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        let mut paths = gpui_kit::assets::Assets.list(path)?;
        paths.extend(ExtraIcons.list(path)?);
        paths.sort();
        paths.dedup();
        Ok(paths)
    }
}

/// What the application was started with: the path of a repository, or a
/// link. An empty argument is no argument: taken for a path it would be
/// the directory the application happened to be started in.
fn started_with(argument: Option<String>) -> (Option<std::path::PathBuf>, Option<String>) {
    let Some(text) = argument.filter(|text| !text.trim().is_empty()) else {
        return (None, None);
    };
    if crate::instance::is_link(&text) {
        (None, Some(text))
    } else {
        (Some(std::path::PathBuf::from(text)), None)
    }
}

/// Set up what the shell reads before there is a window: the toolkit, the
/// preferences as they were loaded, the notices, the git that is run, the
/// themes and the keys. The application loads the preferences from the
/// platform's configuration directory; the off-screen tests load them from
/// a scratch one, and nothing else here looks at the real one.
pub fn init_with(prefs: prefs::Prefs, cx: &mut App) {
    gpui_kit::init(cx);
    prefs::install(prefs, cx);
    crate::avatars::init(cx);
    crate::notices::init(cx);
    // The accounts at providers, read once.
    crate::providers::init(cx);
    crate::gitexe::init(cx);
    crate::motion::init(cx);
    // What the window knows of updates: nothing until asked.
    crate::updates::init(cx);
    theme::init(cx);
    keymap::init(cx);
}

/// Have the toolkit's text system match the fonts the window draws with
/// against the system's fonts, on a thread of its own while the window is
/// being made. Matching reads every font file of the system, once for each
/// family, weight and style: on the main thread it is most of the first
/// frame. The fonts are the theme's, as the shell draws with them (the
/// family, and joining switched off: `literal::features`), at the weights
/// and the style the views use; a font is matched for each of these as a
/// whole, and one the first frame asks for and this did not is matched
/// then.
fn prewarm_fonts(cx: &App) {
    let theme = cx.theme();
    let (ui, mono) = (theme.font_family.clone(), theme.mono_font_family.clone());
    let drawn = |family: SharedString| Font {
        features: crate::literal::features(),
        ..font(family)
    };
    let mut fonts: Vec<Font> = [
        FontWeight::NORMAL,
        FontWeight::MEDIUM,
        FontWeight::SEMIBOLD,
        FontWeight::BOLD,
    ]
    .into_iter()
    .map(|weight| Font {
        weight,
        ..drawn(ui.clone())
    })
    .collect();
    fonts.push(drawn(ui).italic());
    fonts.push(drawn(mono));
    let text = cx.text_system().clone();
    // A thread of its own: should no font resolve, the toolkit gives up on
    // this thread and not the application's (the first frame would then
    // fail as it does without this).
    let _ = std::thread::Builder::new()
        .name("bitbraid-fonts".to_owned())
        .spawn(move || text.prewarm_fonts(&fonts));
}

/// Start the desktop application. Returns when the last window closes.
/// A path given on the command line is opened in addition to restored tabs.
#[allow(clippy::print_stderr)] // startup failures have nowhere else to go
pub fn run() {
    // Started by git or ssh as the askpass program of a running BitBraid:
    // pass the question on, print the answer, and that is all.
    if let Some(code) = bitbraid_git::askpass::client_main() {
        std::process::exit(code);
    }
    crate::session::milestone("process");
    // What it was started with: a link, which names a repository and
    // something in it, or the path of a repository.
    let (initial, link) = started_with(std::env::args().nth(1));
    // Started with something to open while an instance with the same
    // configuration is running: that one opens it, in a tab of its window.
    // A measuring run, and one that asks for it, is always its own.
    let own = std::env::var_os("BITBRAID_NEW_INSTANCE").is_some()
        || std::env::var_os("BITBRAID_BENCH").is_some()
        || std::env::var_os(crate::views::terminal::text::BENCH_VARIABLE).is_some()
        || std::env::var_os(crate::shell::tab_bench::VARIABLE).is_some();
    if !own
        && let Some(socket) = crate::instance::own_socket()
        && let Some(message) = crate::instance::message_for(link.as_deref(), initial.as_deref())
        && crate::instance::hand_over(&socket, &message)
    {
        return;
    }
    // Started by the system (the Finder, the Dock, a link) rather than
    // from a shell, the application on macOS has the system's few
    // variables: the user's login shell is asked for its own once, and
    // every program started from here on gets it (`bitbraid_core::process`).
    if cfg!(target_os = "macos") && std::env::var_os("TERM").is_none() {
        let shell = std::env::var_os("SHELL")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::PathBuf::from("/bin/zsh"));
        if let Some(vars) =
            bitbraid_core::process::login_environment(&shell, std::time::Duration::from_secs(3))
        {
            bitbraid_core::process::adopt_environment(vars);
        }
        crate::session::milestone("login shell asked");
    }
    // The application is its one window: closing it quits, on macOS too,
    // where the toolkit otherwise keeps running without a window and
    // nothing would bring one back.
    let app = gpui_kit::application()
        .with_assets(Assets)
        .with_quit_mode(QuitMode::LastWindowClosed);
    // On macOS a link clicked anywhere, and a folder dropped on the icon or
    // opened with the application, arrive from the system, at the start
    // and while it runs.
    app.on_open_urls(crate::instance::opened_by_system);
    app.run(move |cx| {
        crate::session::milestone("app running");
        init_with(prefs::Prefs::load(), cx);
        // macOS's menu bar, with the keys just bound.
        crate::menus::init(cx);
        // What the picture service answered is kept in the cache directory.
        crate::avatars::keep_in(prefs::cache_dir(), cx);
        crate::askpass::init(cx);
        crate::instance::init(cx);
        // Quitting stops what is running and writes the activity log out.
        cx.on_app_quit(|_| {
            crate::session::quit();
            async {}
        })
        .detach();
        // What an update left: the old copy, now that the new
        // one runs, and what was downloaded and not used. On a thread of
        // its own, since an old bundle is many files; nothing is asked of
        // anybody.
        let _ = std::thread::Builder::new()
            .name("bitbraid-update-tidy".to_owned())
            .spawn(|| {
                let updater = bitbraid_integrations::update::Updater::this_copy();
                bitbraid_integrations::update::tidy(
                    &updater.installation,
                    updater.downloads.as_deref(),
                );
            });
        crate::session::milestone("globals ready");

        // Open the repositories the shell is about to show before the window
        // exists, so that they are read while the window is being created.
        let mut preloaded = crate::session::Preloaded::default();
        let remembered = cx.global::<prefs::Prefs>().state.open.clone();
        let wanted =
            remembered
                .into_iter()
                .filter(|path| path.is_dir())
                .chain(initial.as_ref().map(|path| {
                    bitbraid_core::paths::canonical(path).unwrap_or_else(|_| path.clone())
                }));
        for path in wanted {
            let repo_state = cx.global::<prefs::Prefs>().repo_state(&path);
            let order = cx.global::<prefs::Prefs>().preferences.order();
            let graph = crate::views::workspace::graph_request(&repo_state, None, order);
            // Where its list was left, for the details of its first rows.
            let shown = crate::session::Shown::remembered(
                repo_state.top.as_deref(),
                repo_state.selected.as_deref(),
            );
            preloaded.open(&path, graph, shown);
        }
        cx.set_global(preloaded);
        prewarm_fonts(cx);
        crate::session::milestone("sessions preloading");

        let bounds = Bounds::centered(None, size(px(1440.), px(920.)), cx);
        let options = WindowOptions {
            titlebar: Some(crate::chrome::titlebar(
                crate::brand::NAME.into(),
                crate::zoom::px(crate::shell::TAB_BAR_HEIGHT),
            )),
            // Desktops find the icon and the name through the desktop entry
            // of this id (`packaging/linux/bitbraid.desktop`).
            app_id: Some(crate::brand::APP_ID.to_owned()),
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            ..Default::default()
        };
        let (initial, link) = (initial.clone(), link.clone());
        match gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| Shell::new(initial, link, window, cx))
        }) {
            Ok(_) => {
                crate::session::milestone("window open");
                // Whatever the shell did not take is not going to be shown.
                cx.set_global(crate::session::Preloaded::default());
                cx.activate(true)
            }
            Err(err) => {
                eprintln!("bitbraid: cannot open a window: {err:#}");
                cx.quit();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::started_with;

    #[test]
    fn an_argument_is_a_path_a_link_or_nothing() {
        assert_eq!(started_with(None), (None, None));
        // An empty word taken for a path would show the directory the
        // application was started in, under no name.
        assert_eq!(started_with(Some(String::new())), (None, None));
        assert_eq!(started_with(Some("  ".to_owned())), (None, None));
        assert_eq!(
            started_with(Some("/work/project".to_owned())),
            (Some(PathBuf::from("/work/project")), None)
        );
        assert_eq!(
            started_with(Some(".".to_owned())),
            (Some(PathBuf::from(".")), None)
        );
        let link = "BitBraid://open?path=/work/project";
        assert_eq!(
            started_with(Some(link.to_owned())),
            (None, Some(link.to_owned()))
        );
        // Too short to be a link, and not valid to cut where the scheme
        // would end: paths.
        assert_eq!(started_with(Some("gits".to_owned())).1, None);
        assert_eq!(started_with(Some("aéééééé".to_owned())).1, None);
    }
}
