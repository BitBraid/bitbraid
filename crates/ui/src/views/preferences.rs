//! The preferences: everything `preferences.toml` holds, with a control
//! for each, in pages. A change is written to the file as it is made.
//!
//! The view belongs to the window, not to a repository: it is there with
//! no repository open. Every setting of every page is a row of the same
//! form (`row`): its words, its control beside them, and what explains it
//! under the control.

pub(crate) mod page;
pub(crate) mod row;
mod terminal;

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::{
    gitexe::GitExecutable,
    prefs::{Column, Preferences, Prefs, ThemeChoice},
    session::Reply,
    views::flow,
    views::integrations::{IntegrationsEvent, IntegrationsView},
    views::profiles,
    views::workspace::PULL_MODES,
};
use page::Group;
use profiles::{Act, Profiles};
use row::{Note, Number, Parts, Typed};
use std::rc::Rc;

/// A section of the preferences, in the order the column lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// Fetching, pulling, branches.
    General,
    /// Who is committed as.
    Profiles,
    /// The key the current profile connects with.
    Ssh,
    /// The accounts at GitHub and GitLab.
    Integrations,
    /// The programs started from here.
    Tools,
    /// How the window looks.
    Interface,
    /// How commits and tags are signed.
    Signing,
    /// How a file and its changes are shown.
    Editor,
    /// The terminal inside the window.
    Terminal,
    /// The git that is run.
    Experimental,
    /// Keys of one's own.
    Keys,
    /// Where all of this is kept.
    Files,
    /// The open repository's gitflow branches and prefixes.
    Gitflow,
    /// The hooks git runs in the open repository.
    Hooks,
    /// The open repository's large files.
    Lfs,
    /// The open repository's size, git's tasks that keep it in shape, and
    /// a check of its objects.
    Maintenance,
}

impl Section {
    /// Every section every repository shares, in their order.
    pub const SHARED: [Section; 12] = [
        Section::General,
        Section::Profiles,
        Section::Ssh,
        Section::Integrations,
        Section::Tools,
        Section::Interface,
        Section::Signing,
        Section::Editor,
        Section::Terminal,
        Section::Experimental,
        Section::Keys,
        Section::Files,
    ];

    /// The sections of the repository whose tab is open, in their order.
    pub const REPO: [Section; 4] = [
        Section::Gitflow,
        Section::Hooks,
        Section::Lfs,
        Section::Maintenance,
    ];

    /// Whether it is a section of the open repository.
    pub fn of_repo(self) -> bool {
        Section::REPO.contains(&self)
    }

    /// What the column and the heading call it.
    pub fn name(self) -> &'static str {
        match self {
            Section::General => say!("preferences-section-general"),
            Section::Profiles => say!("preferences-section-profiles"),
            Section::Ssh => say!("preferences-section-ssh"),
            Section::Integrations => say!("preferences-section-integrations"),
            Section::Tools => say!("preferences-section-tools"),
            Section::Interface => say!("preferences-section-interface"),
            Section::Signing => say!("preferences-section-signing"),
            Section::Editor => say!("preferences-section-editor"),
            Section::Terminal => say!("preferences-section-terminal"),
            Section::Experimental => say!("preferences-section-experimental"),
            Section::Keys => say!("preferences-section-keys"),
            Section::Files => say!("preferences-section-files"),
            Section::Gitflow => say!("preferences-section-gitflow"),
            Section::Hooks => say!("preferences-section-hooks"),
            Section::Lfs => say!("preferences-section-lfs"),
            Section::Maintenance => say!("preferences-section-maintenance"),
        }
    }

    /// What it is called in a test.
    pub fn key(self) -> &'static str {
        match self {
            Section::General => "general",
            Section::Profiles => "profiles",
            Section::Ssh => "ssh",
            Section::Integrations => "integrations",
            Section::Tools => "tools",
            Section::Interface => "interface",
            Section::Signing => "signing",
            Section::Editor => "editor",
            Section::Terminal => "terminal",
            Section::Experimental => "experimental",
            Section::Keys => "keys",
            Section::Files => "files",
            Section::Gitflow => "gitflow",
            Section::Hooks => "hooks",
            Section::Lfs => "lfs",
            Section::Maintenance => "maintenance",
        }
    }

    fn icon(self) -> Icon {
        use gpui_kit::assets::IconName as Name;
        Icon::new(match self {
            Section::General => Name::Settings,
            Section::Profiles => Name::Users,
            Section::Ssh => Name::KeyRound,
            Section::Integrations => Name::Plug,
            Section::Tools => Name::Wrench,
            Section::Interface => Name::Palette,
            Section::Signing => Name::Signature,
            Section::Editor => Name::CodeXml,
            Section::Terminal => Name::Terminal,
            Section::Experimental => Name::FlaskConical,
            Section::Keys => Name::Keyboard,
            Section::Files => Name::FolderCog,
            Section::Gitflow => Name::Workflow,
            Section::Hooks => Name::Webhook,
            Section::Lfs => Name::HardDrive,
            Section::Maintenance => Name::Broom,
        })
    }
}

/// What the preferences ask of the window.
#[derive(Debug, Clone)]
pub enum PreferencesEvent {
    /// Put the preferences away.
    Close,
    /// Show every key there is.
    Shortcuts,
    /// Read `keybindings.toml` again.
    ReloadKeys,
    /// Ask whether every key is to be the default again.
    ResetKeys,
    /// Write a theme file to start from into the folder of themes.
    ExampleTheme,
    /// Another git executable is run from now on: what is shown of the
    /// open repositories is to be read again.
    GitChanged,
    /// Make this profile the current one.
    SwitchProfile(String),
    /// Commit as this profile in the open repository only.
    UseProfileHere(String),
    /// A profile was made, changed or deleted: read them again.
    ProfilesChanged,
    /// Something asked of the updater.
    Updates(crate::updates::Ask),
}

/// The repository whose tab is open, as far as the preferences show it.
#[derive(Clone)]
pub struct RepoContext {
    /// What its tab is called.
    pub name: SharedString,
    /// Its folder.
    pub path: std::path::PathBuf,
    /// Its tab, which keeps what its sections show.
    pub workspace: WeakEntity<crate::views::workspace::Workspace>,
}

/// The fields of gitflow's section: for which repository, and the main and
/// develop branch, the prefixes in the order of `flow::KINDS`, the tag's.
struct FlowFields {
    path: std::path::PathBuf,
    inputs: Vec<Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
}

/// What `hook.list` answered: where the hooks are, whether git's settings
/// name that place, and each hook with its file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HookList {
    pub directory: String,
    pub configured: bool,
    pub hooks: Vec<(String, String)>,
}

impl HookList {
    pub(crate) fn from_json(value: &serde_json::Value) -> Self {
        let text = |value: &serde_json::Value, key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        Self {
            directory: text(value, "directory"),
            configured: value.get("configured").and_then(|flag| flag.as_bool()) == Some(true),
            hooks: value
                .get("hooks")
                .and_then(|hooks| hooks.as_array())
                .into_iter()
                .flatten()
                .filter_map(|hook| {
                    let name = text(hook, "name");
                    (!name.is_empty()).then(|| (name, text(hook, "path")))
                })
                .take(64)
                .collect(),
        }
    }
}

impl EventEmitter<PreferencesEvent> for PreferencesView {}

/// The preferences view.
pub struct PreferencesView {
    focus_handle: FocusHandle,
    /// The field the git executable is typed into.
    git_input: Entity<InputState>,
    /// What came of the last attempt to use another executable: whether it
    /// worked, and what there is to say.
    git_note: Option<(bool, String)>,
    /// An executable is being asked what it is.
    git_busy: bool,
    /// The commands of the terminals found on this system, the one that
    /// is started first.
    terminals: Vec<String>,
    /// The diff and merge tools found on this system, and whether the
    /// look for them is over.
    tools: Vec<FoundTool>,
    tools_asked: bool,
    /// The section that is shown.
    chosen: Section,
    /// The profiles, and which is the current one: the window reads them.
    profiles: Profiles,
    /// The repository whose tab is open.
    repo: Option<RepoContext>,
    /// What redraws a profile's dialog while it is typed in.
    dialog_subscriptions: Vec<Subscription>,
    /// The field the current profile's SSH key file is typed into, and
    /// whether the agent was let go of before a file was given.
    ssh_input: Entity<InputState>,
    ssh_without_agent: bool,
    /// Why the key file that was given is not used.
    ssh_refused: Option<String>,
    /// The tools for which a command of one's own was chosen, while it is
    /// still being typed.
    custom_tools: Vec<Tool>,
    /// Gitflow's fields, once its section was shown.
    flow_fields: Option<FlowFields>,
    /// The hooks of a repository, once its section was shown: for which,
    /// and what was found or why not.
    hooks: Option<(std::path::PathBuf, Result<HookList, String>)>,
    /// The accounts at providers: a column of its own and a page.
    integrations: Entity<IntegrationsView>,
    /// Where the column and the section are scrolled to.
    column_scroll: ScrollHandle,
    section_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

/// Change the preferences, bring them into range, write them, and show the
/// change in every window.
fn change(cx: &mut App, change: impl FnOnce(&mut Preferences)) {
    let prefs = cx.global_mut::<Prefs>();
    change(&mut prefs.preferences);
    prefs.preferences.sanitize();
    prefs.save_preferences();
    cx.refresh_windows();
}

/// What the switch for the pictures of authors says under its name:
/// exactly what is sent when it is on, and to whom.
pub(crate) fn pictures_text(whom: &crate::avatars::Whom) -> String {
    let Some(host) = &whom.host else {
        return say!(
            "preferences-pictures-bad-address",
            problem = whom
                .problem
                .as_deref()
                .unwrap_or(say!("preferences-pictures-unreadable"))
        );
    };
    let how = if whom.secure {
        say!("preferences-pictures-over-https")
    } else {
        say!("preferences-pictures-without-tls")
    };
    let kept = match &whom.kept_in {
        Some(folder) => format!(
            " {}",
            say!(
                "preferences-pictures-kept",
                folder = folder.display().to_string()
            )
        ),
        None => String::new(),
    };
    say!("preferences-pictures-note", host, how, kept)
}

/// Whether pictures are asked of the providers of the connected
/// accounts, with what that sends and to whom.
fn provider_pictures_item() -> row::Item {
    row::check(
        say!("preferences-provider-pictures"),
        Note::plain(say!("preferences-provider-pictures-note")),
        |preferences| preferences.provider_pictures,
        |preferences, on| preferences.provider_pictures = on,
    )
}

/// The row about this copy and the updater: what it does with this copy and
/// what the last look found, and the buttons for what can be done now:
/// look, install what was found or open its page, stop the download, start
/// the version that was installed.
fn updates_item(view: WeakEntity<PreferencesView>) -> row::Item {
    use crate::updates::{Ask, Installed, Updates};
    row::item_of(say!("preferences-this-copy"), move |_, cx| {
        let updates = cx.global::<Updates>().clone();
        let on = cx.global::<Prefs>().preferences.updates;
        let said = crate::updates::copy_text(&updates, on, crate::views::now());
        let key = updates.status.as_ref().is_some_and(|status| status.key);
        let busy = updates.looking || updates.installing.is_some();
        let button = |id: &'static str, label: SharedString, ask: Ask, primary: bool| {
            let view = view.clone();
            let button = Button::new(id)
                .small()
                .h(px(row::BUTTON_HEIGHT))
                .label(label)
                .on_click(move |_, _, cx| {
                    let _ = view.update(cx, |_, cx| cx.emit(PreferencesEvent::Updates(ask)));
                });
            if primary {
                button.primary()
            } else {
                button.outline()
            }
        };
        let mut buttons = h_flex().gap(px(5.));
        if updates.installing.is_some() {
            buttons = buttons.child(button(
                "updates-stop",
                say!("preferences-updates-stop").into(),
                Ask::Stop,
                false,
            ));
        } else if let Some(Installed::Restart(version)) = &updates.installed {
            buttons = buttons.child(button(
                "updates-restart",
                say!("preferences-restart-with", version).into(),
                Ask::Restart,
                true,
            ));
        } else {
            buttons = buttons.child(
                button(
                    "updates-look",
                    say!("preferences-look-now").into(),
                    Ask::Look,
                    false,
                )
                .disabled(!on || !key || busy),
            );
            match updates.found.as_ref().filter(|look| look.newer) {
                Some(look) if look.installs => {
                    buttons = buttons.child(
                        button(
                            "updates-install",
                            say!("preferences-install-version", version = &look.latest).into(),
                            Ask::Install,
                            true,
                        )
                        .disabled(busy),
                    );
                }
                Some(_) => {
                    buttons = buttons.child(button(
                        "updates-page",
                        say!("preferences-open-page").into(),
                        Ask::Page,
                        false,
                    ));
                }
                None => {}
            }
        }
        row::Parts {
            control: Some(buttons.into_any_element()),
            under: Some(
                div()
                    .id("updates-said")
                    .child(said)
                    .test_support()
                    .into_any_element(),
            ),
        }
    })
}

/// What a field holds, as the preferences keep a command: nothing when
/// it is empty.
fn command_value(typed: &str) -> Option<String> {
    let trimmed = typed.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// The commands of the terminals `repo.terminals` found.
pub(crate) fn terminal_commands(value: &serde_json::Value) -> Vec<String> {
    value
        .get("terminals")
        .and_then(|terminals| terminals.as_array())
        .map(|terminals| {
            terminals
                .iter()
                .filter_map(|terminal| terminal.get("command")?.as_str().map(str::to_owned))
                .take(32)
                .collect()
        })
        .unwrap_or_default()
}

/// Gitflow's settings as its fields hold them: the main and the develop
/// branch, the prefixes in the order of `flow::KINDS`, the tag's. A field
/// that is not there is empty, which the review waits on.
pub(crate) fn flow_settings_from(typed: &[String]) -> flow::FlowSettings {
    let get = |ix: usize| typed.get(ix).cloned().unwrap_or_default();
    flow::FlowSettings {
        main: get(0),
        develop: get(1),
        prefixes: [2, 3, 4, 5, 6].map(get),
        tag: get(7),
    }
}

/// A program started from here, as the External Tools section lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tool {
    Merge,
    Diff,
    Editor,
    Terminal,
    FileManager,
}

/// The word of the list's first choice: what the system or git says.
const DEFAULT_TOOL: &str = "default";
/// The word of the list's last choice: a command of one's own.
const CUSTOM_TOOL: &str = "custom";

impl Tool {
    /// In the order the section has them.
    const ALL: [Tool; 5] = [
        Tool::Merge,
        Tool::Diff,
        Tool::Editor,
        Tool::Terminal,
        Tool::FileManager,
    ];

    fn get(self, preferences: &Preferences) -> &Option<String> {
        match self {
            Tool::Merge => &preferences.external_merge,
            Tool::Diff => &preferences.external_diff,
            Tool::Editor => &preferences.editor,
            Tool::Terminal => &preferences.terminal,
            Tool::FileManager => &preferences.file_manager,
        }
    }

    fn set(self, preferences: &mut Preferences, command: Option<String>) {
        match self {
            Tool::Merge => preferences.external_merge = command,
            Tool::Diff => preferences.external_diff = command,
            Tool::Editor => preferences.editor = command,
            Tool::Terminal => preferences.terminal = command,
            Tool::FileManager => preferences.file_manager = command,
        }
    }

    fn heading(self) -> &'static str {
        match self {
            Tool::Merge => say!("preferences-tool-merge"),
            Tool::Diff => say!("preferences-tool-diff"),
            Tool::Editor => say!("preferences-tool-editor"),
            Tool::Terminal => say!("preferences-tool-terminal"),
            Tool::FileManager => say!("preferences-tool-file-manager"),
        }
    }

    fn words(self) -> &'static str {
        match self {
            Tool::Merge => say!("preferences-tool-merge-words"),
            Tool::Diff => say!("preferences-tool-diff-words"),
            Tool::Editor => say!("preferences-tool-editor-words"),
            Tool::Terminal => say!("preferences-tool-terminal-words"),
            Tool::FileManager => say!("preferences-tool-file-manager"),
        }
    }

    fn command_words(self) -> &'static str {
        match self {
            Tool::Merge => say!("preferences-tool-merge-command"),
            Tool::Diff => say!("preferences-tool-diff-command"),
            Tool::Editor => say!("preferences-tool-editor-command"),
            Tool::Terminal => say!("preferences-tool-terminal-command"),
            Tool::FileManager => say!("preferences-tool-file-manager-command"),
        }
    }

    fn default_label(self) -> &'static str {
        match self {
            Tool::Merge | Tool::Diff => say!("preferences-tool-gits-setting"),
            Tool::Editor | Tool::FileManager => say!("preferences-tool-systems-choice"),
            Tool::Terminal => say!("preferences-tool-first-found"),
        }
    }

    fn note(self) -> &'static str {
        match self {
            Tool::Merge => say!("preferences-tool-merge-note"),
            Tool::Diff => say!("preferences-tool-diff-note"),
            Tool::Editor => say!("preferences-tool-editor-note"),
            Tool::Terminal => say!("preferences-tool-terminal-note"),
            Tool::FileManager => say!("preferences-tool-file-manager-note"),
        }
    }

    fn example(self) -> &'static str {
        match self {
            Tool::Merge => say!("preferences-tool-merge-example"),
            Tool::Diff => say!("preferences-tool-diff-example"),
            Tool::Editor => say!("preferences-tool-editor-example"),
            Tool::Terminal => say!("preferences-tool-terminal-example"),
            Tool::FileManager => say!("preferences-tool-file-manager-example"),
        }
    }

    fn typed(self) -> Typed {
        match self {
            Tool::Merge => Typed {
                show: |preferences| preferences.external_merge.clone().unwrap_or_default(),
                take: |preferences, typed| preferences.external_merge = command_value(typed),
            },
            Tool::Diff => Typed {
                show: |preferences| preferences.external_diff.clone().unwrap_or_default(),
                take: |preferences, typed| preferences.external_diff = command_value(typed),
            },
            Tool::Editor => Typed {
                show: |preferences| preferences.editor.clone().unwrap_or_default(),
                take: |preferences, typed| preferences.editor = command_value(typed),
            },
            Tool::Terminal => Typed {
                show: |preferences| preferences.terminal.clone().unwrap_or_default(),
                take: |preferences, typed| preferences.terminal = command_value(typed),
            },
            Tool::FileManager => Typed {
                show: |preferences| preferences.file_manager.clone().unwrap_or_default(),
                take: |preferences, typed| preferences.file_manager = command_value(typed),
            },
        }
    }
}

/// Which choice of a tool's list stands for what the preferences hold:
/// nothing is the first; a command that was found is that one; any other,
/// or one's own chosen and not yet typed, is the last.
pub(crate) fn chosen_tool(
    found: &[(String, String)],
    stored: Option<&str>,
    custom: bool,
) -> String {
    match stored.map(str::trim).filter(|command| !command.is_empty()) {
        None if custom => CUSTOM_TOOL.to_owned(),
        None => DEFAULT_TOOL.to_owned(),
        Some(command) => found
            .iter()
            .position(|(_, found)| found == command)
            .map(|ix| format!("found-{ix}"))
            .unwrap_or_else(|| CUSTOM_TOOL.to_owned()),
    }
}

/// A diff or merge tool that was found, as `tool.known` answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FoundTool {
    pub name: String,
    /// The command that shows two versions of a file, when it has one.
    pub diff: Option<String>,
    /// The command that resolves a file in conflict, when it has one.
    pub merge: Option<String>,
}

/// The tools `tool.known` found.
pub(crate) fn found_tools(value: &serde_json::Value) -> Vec<FoundTool> {
    let command = |tool: &serde_json::Value, key: &str| {
        tool.get(key)
            .and_then(|command| command.as_str())
            .map(str::trim)
            .filter(|command| !command.is_empty())
            .map(str::to_owned)
    };
    value
        .get("found")
        .and_then(|found| found.as_array())
        .map(|found| {
            found
                .iter()
                .filter_map(|tool| {
                    let found = FoundTool {
                        name: tool.get("name")?.as_str()?.to_owned(),
                        diff: command(tool, "diff"),
                        merge: command(tool, "merge"),
                    };
                    (found.diff.is_some() || found.merge.is_some()).then_some(found)
                })
                .take(32)
                .collect()
        })
        .unwrap_or_default()
}

/// Names separated by commas or spaces, each once, in the order given.
pub(crate) fn names_from(typed: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for name in typed.split([',', ' ', '\t', '\n']) {
        let name = name.trim();
        if !name.is_empty() && !names.iter().any(|known| known == name) {
            names.push(name.to_owned());
        }
    }
    names
}

impl PreferencesView {
    /// Create the view.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let chosen = cx
            .global::<Prefs>()
            .preferences
            .git_executable
            .clone()
            .unwrap_or_default();
        let git_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(say!("preferences-git-placeholder"))
                .default_value(chosen)
        });
        let ssh_input = cx
            .new(|cx| InputState::new(window, cx).placeholder(say!("preferences-ssh-placeholder")));
        let subscriptions = vec![
            cx.subscribe_in(
                &ssh_input,
                window,
                |this, input, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                        let typed = input.read(cx).value().trim().to_owned();
                        let stored = this
                            .profiles
                            .current_profile()
                            .and_then(|profile| profile.ssh_key.clone())
                            .unwrap_or_default();
                        if !typed.is_empty() && typed != stored {
                            this.connect_with(Some(typed), window, cx);
                        }
                    }
                },
            ),
            cx.subscribe_in(
                &git_input,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.use_git(window, cx);
                    }
                },
            ),
        ];
        let integrations = cx.new(|cx| IntegrationsView::new(window, cx));
        let subscriptions = subscriptions
            .into_iter()
            .chain([cx.subscribe_in(
                &integrations,
                window,
                |this, _, event: &IntegrationsEvent, window, cx| match event {
                    IntegrationsEvent::UseKey(key) => {
                        this.connect_with(Some(key.clone()), window, cx)
                    }
                },
            )])
            .collect::<Vec<_>>();
        // Which terminals there are is a look through PATH: asked once.
        let found = crate::session::run_standalone("repo.terminals", json!({}));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Reply::Command(value)) = found.reply.await {
                let _ = this.update(cx, |this, cx| {
                    this.terminals = terminal_commands(&value);
                    cx.notify();
                });
            }
        })
        .detach();
        // And which diff and merge tools: the same kind of look.
        let found = crate::session::run_standalone("tool.known", json!({}));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Reply::Command(value)) = found.reply.await {
                let _ = this.update(cx, |this, cx| {
                    this.tools = found_tools(&value);
                    this.tools_asked = true;
                    cx.notify();
                });
            }
        })
        .detach();
        Self {
            focus_handle: cx.focus_handle(),
            git_input,
            git_note: None,
            git_busy: false,
            terminals: Vec::new(),
            tools: Vec::new(),
            tools_asked: false,
            chosen: Section::General,
            profiles: Profiles::default(),
            repo: None,
            dialog_subscriptions: Vec::new(),
            ssh_input,
            ssh_without_agent: false,
            ssh_refused: None,
            custom_tools: Vec::new(),
            flow_fields: None,
            hooks: None,
            integrations,
            column_scroll: ScrollHandle::new(),
            section_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// The section that is shown.
    pub fn section(&self) -> Section {
        self.chosen
    }

    /// Show a section, from its top.
    pub fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        self.chosen = section;
        self.section_scroll.set_offset(point(px(0.), px(0.)));
        match section {
            // Read again each time they are shown.
            Section::Hooks => {
                self.hooks = None;
                self.load_hooks(cx);
            }
            Section::Gitflow => self.flow_fields = None,
            _ => {}
        }
        cx.notify();
    }

    /// What gitflow's fields hold, in their order; for the off-screen
    /// tests.
    pub fn flow_values(&self, cx: &App) -> Vec<String> {
        self.flow_fields
            .as_ref()
            .map(|fields| {
                fields
                    .inputs
                    .iter()
                    .map(|input| input.read(cx).value().to_string())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Ask the open repository which hooks git runs there.
    fn load_hooks(&mut self, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self
            .hooks
            .as_ref()
            .is_some_and(|(path, _)| *path == repo.path)
        {
            return;
        }
        let Some(workspace) = repo.workspace.upgrade() else {
            return;
        };
        let session = workspace.read(cx).session();
        // Asked once for this repository: what was found stands meanwhile.
        self.hooks = Some((repo.path.clone(), Ok(HookList::default())));
        let path = repo.path.clone();
        session.call(
            crate::session::Request::Command {
                id: "hook.list".to_owned(),
                params: json!({}),
            },
            cx,
            move |this, reply, cx| {
                let found = match reply {
                    Reply::Command(value) => Ok(HookList::from_json(&value)),
                    Reply::Error(err) => Err(err),
                    _ => return,
                };
                this.hooks = Some((path, found));
                cx.notify();
            },
        );
    }

    /// The profiles as the window has read them.
    pub fn set_profiles(&mut self, profiles: Profiles, cx: &mut Context<Self>) {
        if self.profiles != profiles {
            self.profiles = profiles;
            let key = self
                .profiles
                .current_profile()
                .and_then(|profile| profile.ssh_key.clone());
            self.integrations
                .update(cx, |view, cx| view.set_profile_key(key, cx));
            cx.notify();
        }
    }

    /// The section of the accounts at providers.
    pub fn integrations(&self) -> Entity<IntegrationsView> {
        self.integrations.clone()
    }

    /// The repository whose tab is open, or none.
    pub fn set_repo(&mut self, repo: Option<RepoContext>) {
        // Without a repository its sections are not there to be shown.
        if repo.is_none() && self.chosen.of_repo() {
            self.chosen = Section::General;
        }
        self.repo = repo;
    }

    /// Do what was chosen for a profile.
    fn profile_act(&mut self, act: Act, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        match act {
            Act::Add | Act::Edit(_) => {
                let editing = match &act {
                    Act::Edit(name) => self
                        .profiles
                        .list
                        .iter()
                        .find(|profile| &profile.name == name)
                        .cloned(),
                    _ => None,
                };
                let on_saved: profiles::OnSaved = Rc::new(move |name, new, _, cx| {
                    let _ = view.update(cx, |_, cx| {
                        cx.emit(PreferencesEvent::ProfilesChanged);
                        // A new profile is the current one at once.
                        if new {
                            cx.emit(PreferencesEvent::SwitchProfile(name));
                        }
                    });
                });
                self.dialog_subscriptions =
                    profiles::open_dialog(&self.profiles, editing.as_ref(), on_saved, window, cx);
            }
            Act::Delete(name) => {
                let gone = name.clone();
                crate::ask::show(
                    crate::ask::Ask::new(say!("preferences-delete-profile", name)).danger(
                        say!("preferences-delete"),
                        move |_, window, cx| {
                            let run = crate::session::run_standalone(
                                "profile.delete",
                                json!({ "name": gone.clone() }),
                            );
                            let view = view.clone();
                            window
                                .spawn(cx, async move |cx| {
                                    let reply = run.reply.await;
                                    let _ = cx.update(|window, cx| {
                                        match reply {
                                            Ok(Reply::Error(err)) => {
                                                crate::notices::push(
                                                    window,
                                                    cx,
                                                    crate::notices::Notice::error(
                                                        crate::shell::sentence(&err),
                                                    ),
                                                );
                                            }
                                            // The engine took the profile out of
                                            // the workspaces that named it; what
                                            // kept it from that is said.
                                            Ok(Reply::Command(value)) => {
                                                if let Some(why) = value
                                                    .get("workspaces_error")
                                                    .and_then(serde_json::Value::as_str)
                                                {
                                                    crate::notices::push(
                                                        window,
                                                        cx,
                                                        crate::notices::Notice::warning(say!(
                                                            "preferences-workspaces-still-name",
                                                            error = crate::shell::sentence(why)
                                                        )),
                                                    );
                                                }
                                            }
                                            _ => {}
                                        }
                                        let _ = view.update(cx, |_, cx| {
                                            cx.emit(PreferencesEvent::ProfilesChanged)
                                        });
                                    });
                                })
                                .detach();
                        },
                    ),
                    window,
                    cx,
                );
            }
            Act::Switch(name) => cx.emit(PreferencesEvent::SwitchProfile(name)),
            Act::UseHere(name) => cx.emit(PreferencesEvent::UseProfileHere(name)),
        }
    }

    /// What the section's controls hand what is chosen to.
    fn on_act(&self, cx: &mut Context<Self>) -> profiles::OnAct {
        let view = cx.entity().downgrade();
        Rc::new(move |act, window, cx| {
            let _ = view.update(cx, |this, cx| this.profile_act(act, window, cx));
        })
    }

    /// Who is committed as, at the top of the column: the current profile
    /// in a box, a list to switch from when there are several.
    fn profile_box(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let current = self.profiles.current_profile().cloned();
        let several = self.profiles.list.len() > 1;
        let (key, name) = current
            .as_ref()
            .map(|profile| (profile.picture_key().to_owned(), profile.name.clone()))
            .unwrap_or_default();
        let inside = h_flex()
            .w_full()
            .h_full()
            .items_center()
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .pl(px(11.))
                    .gap(px(11.))
                    .child(profiles::picture(&key, 28., cx))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(name),
                    ),
            )
            .when(several, |row| {
                row.child(
                    div()
                        .w(px(30.))
                        .h_full()
                        .flex_none()
                        .flex()
                        .items_center()
                        .justify_center()
                        .border_l_1()
                        .border_color(theme.border)
                        .child(Icon::new(IconName::ChevronDown).size(px(16.))),
                )
            });
        let frame = div()
            .mx(px(10.))
            .w(px(281.))
            .h(px(40.))
            .flex_none()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.background);
        if !several {
            return frame.child(inside).into_any_element();
        }
        let listed: Vec<(String, String, bool)> = self
            .profiles
            .current_first()
            .into_iter()
            .map(|profile| {
                (
                    profile.name.clone(),
                    profile.picture_key().to_owned(),
                    current.as_ref().is_some_and(|now| now.name == profile.name),
                )
            })
            .collect();
        let view = cx.entity().downgrade();
        frame
            .child(
                Button::new("current-profile")
                    .accessibility_label(say!("preferences-profile"))
                    .ghost()
                    // The size of what is inside the box's border, given
                    // whole: the menu's popover stands between the box and
                    // the button, so a size relative to the box would be
                    // lost and the button would keep the toolkit's 32 high,
                    // at the box's top.
                    .w(px(281. - 2.))
                    .h(px(40. - 2.))
                    .p_0()
                    .child(inside)
                    .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                        listed.iter().fold(menu, |menu, (name, key, now)| {
                            let (shown, key, view) = (name.clone(), key.clone(), view.clone());
                            let switch_to = name.clone();
                            menu.item(
                                PopupMenuItem::element(move |_, cx| {
                                    h_flex()
                                        .h(px(37.))
                                        .gap(px(10.))
                                        .items_center()
                                        .child(profiles::picture(&key, 28., cx))
                                        .child(shown.clone())
                                })
                                .checked(*now)
                                .on_click(move |_, _, cx| {
                                    let name = switch_to.clone();
                                    let _ = view.update(cx, |_, cx| {
                                        cx.emit(PreferencesEvent::SwitchProfile(name))
                                    });
                                }),
                            )
                        })
                    }),
            )
            .into_any_element()
    }

    /// Connect with a key file from now on, or with the agent's keys: the
    /// current profile keeps it, and git's settings are written again.
    fn connect_with(&mut self, key: Option<String>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut profile) = self.profiles.current_profile().cloned() else {
            return;
        };
        let rename_from = profile.stored.then(|| profile.name.clone());
        profile.ssh_key = key
            .as_deref()
            .and_then(|key| crate::shell::expand_home(key).to_str().map(str::to_owned));
        // A file that is not there is not kept: ssh would pass over it
        // and go on with keys of its own choosing.
        if let Some(key) = &profile.ssh_key
            && !std::path::Path::new(key).is_file()
        {
            self.ssh_refused = Some(say!("preferences-ssh-no-key-file", key));
            cx.notify();
            return;
        }
        let said = match &profile.ssh_key {
            Some(key) => say!(
                "preferences-ssh-connects-with-key",
                name = &profile.name,
                key
            ),
            None => say!("preferences-ssh-connects-with-agent", name = &profile.name),
        };
        let (name, saved) = (profile.name.clone(), profile.as_json());
        let shown = profile.ssh_key.clone().unwrap_or_default();
        cx.spawn_in(window, async move |this, cx| {
            let run = crate::session::run_standalone(
                "profile.save",
                json!({ "profile": saved, "rename_from": rename_from }),
            );
            let mut reply = run.reply.await;
            if let Ok(Reply::Command(_)) = reply {
                let used = crate::session::run_standalone(
                    "profile.use",
                    json!({ "name": name, "scope": "user" }),
                );
                reply = used.reply.await;
            }
            let _ = this.update_in(cx, |this, window, cx| {
                match reply {
                    Ok(Reply::Command(_)) => {
                        this.ssh_without_agent = false;
                        this.ssh_refused = None;
                        this.ssh_input
                            .update(cx, |input, cx| input.set_value(shown, window, cx));
                        crate::notices::push(window, cx, crate::notices::Notice::success(said));
                    }
                    // Said at the field, which keeps what was typed.
                    Ok(Reply::Error(err)) => {
                        this.ssh_refused = Some(crate::shell::sentence(&err));
                        cx.notify();
                    }
                    _ => {}
                }
                cx.emit(PreferencesEvent::ProfilesChanged);
            });
        })
        .detach();
    }

    /// Choose the key file in the desktop's chooser.
    fn browse_for_key(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(say!("preferences-ssh-use-this-key").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
                && let Some(path) = path.to_str().map(str::to_owned)
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.connect_with(Some(path), window, cx)
                });
            }
        })
        .detach();
    }

    /// The SSH key the current profile connects with: the agent's keys, or
    /// a key file of its own.
    fn ssh_section(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let current = self.profiles.current_profile().cloned().unwrap_or_default();
        let own = current.ssh_key.clone();
        let agent = own.is_none() && !self.ssh_without_agent;
        let input = self.ssh_input.clone();
        let refused = self.ssh_refused.clone();
        let danger = cx.theme().danger;
        let view = cx.entity().downgrade();
        let (for_check, for_browse) = (view.clone(), view.clone());
        let name = current.name.clone();
        vec![
            Group::new()
                .item(row::item_of(
                    say!("preferences-ssh-use-agent"),
                    move |_, _| {
                        let for_check = for_check.clone();
                        Parts {
                            control: Some(
                                gpui_kit::component::checkbox::Checkbox::new("check-ssh-agent")
                                    .accessibility_label(say!("preferences-ssh-use-agent"))
                                    .checked(agent)
                                    .on_click(move |on: &bool, window, cx| {
                                        let on = *on;
                                        let _ = for_check.update(cx, |this, cx| {
                                            if on {
                                                this.connect_with(None, window, cx);
                                            } else {
                                                this.ssh_without_agent = true;
                                                cx.notify();
                                            }
                                        });
                                    })
                                    .into_any_element(),
                            ),
                            under: Some(
                                div()
                                    .child(say!("preferences-ssh-agent-note", name = &name))
                                    .into_any_element(),
                            ),
                        }
                    },
                ))
                .item(row::item_of(
                    say!("preferences-ssh-key"),
                    move |window, cx| {
                        // The field shows the key the profile has, until something
                        // else is typed or what was typed is refused.
                        let shown = own.clone().unwrap_or_default();
                        if refused.is_none()
                            && !input.read(cx).focus_handle(cx).is_focused(window)
                            && *input.read(cx).value() != *shown
                        {
                            input
                                .update(cx, |input, cx| input.set_value(shown.clone(), window, cx));
                        }
                        let for_browse = for_browse.clone();
                        Parts {
                            control: Some(
                                h_flex()
                                    .w(px(row::CONTROL_WIDTH))
                                    .gap(px(5.))
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .when(refused.is_some(), |field| {
                                                field
                                                    .rounded(px(6.))
                                                    .border_1()
                                                    .border_color(danger)
                                            })
                                            .child(
                                                Input::new(&input)
                                                    .id("field-ssh-key")
                                                    .disabled(agent)
                                                    .min_h(px(row::CONTROL_HEIGHT))
                                                    .w_full(),
                                            ),
                                    )
                                    .child(
                                        Button::new("ssh-browse")
                                            .outline()
                                            .small()
                                            .h(px(row::BUTTON_HEIGHT))
                                            .w(px(68.))
                                            .label(say!("preferences-ssh-browse"))
                                            .disabled(agent)
                                            .on_click(move |_, window, cx| {
                                                let _ = for_browse.update(cx, |this, cx| {
                                                    this.browse_for_key(window, cx)
                                                });
                                            }),
                                    )
                                    .into_any_element(),
                            ),
                            under: Some(
                                v_flex()
                                    .children(
                                        refused
                                            .clone()
                                            .map(|why| div().text_color(danger).child(why)),
                                    )
                                    .child(say!("preferences-ssh-key-note"))
                                    .into_any_element(),
                            ),
                        }
                    },
                )),
        ]
    }

    /// The section of the profiles.
    fn profiles_section(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let on = self.on_act(cx);
        let profiles = self.profiles.clone();
        let repo_open = self.repo.is_some();
        let for_add = on.clone();
        vec![
            Group::new()
                .wide(|_, cx| page::paragraph(say!("preferences-profiles-intro"), cx))
                .wide(move |_, _| profiles::add_button(for_add.clone()))
                .wide(move |_, cx| profiles::table(&profiles, repo_open, on.clone(), cx)),
        ]
    }

    /// What the column holds: the way out, who is committed as, then the
    /// sections.
    fn column_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut rows = vec![
            page::way_out(
                cx.listener(|_, _, _, cx| cx.emit(PreferencesEvent::Close)),
                cx,
            ),
            page::space(6.),
            page::caption(say!("preferences-current-profile"), cx),
            page::space(7.),
            self.profile_box(cx),
            page::space(16.),
            page::caption(say!("preferences-caption"), cx),
            page::space(1.),
        ];
        let section_row = |rows: &mut Vec<AnyElement>, section: Section| {
            rows.push(page::section_row(
                SharedString::from(format!("section-{}", section.key())),
                section.icon(),
                section.name().into(),
                self.chosen == section,
                cx.listener(move |this, _, _, cx| this.show(section, cx)),
                cx,
            ));
        };
        for section in Section::SHARED {
            section_row(&mut rows, section);
        }
        if let Some(repo) = self.repo.clone() {
            rows.push(page::space(40.));
            rows.push(page::caption(say!("preferences-this-repository"), cx));
            rows.push(page::repo_line(repo.name.clone(), cx));
            for section in Section::REPO {
                section_row(&mut rows, section);
            }
        }
        rows
    }

    /// The section of how commits are signed: the open repository's.
    fn signing_section(&self, cx: &mut Context<Self>) -> Vec<Group> {
        match self.repo.as_ref().and_then(|repo| repo.workspace.upgrade()) {
            Some(workspace) => {
                let view = workspace.read(cx).signing_view();
                vec![Group::new().own(move |_, _| view.clone().into_any_element())]
            }
            None => vec![
                Group::new()
                    .wide(|_, cx| page::paragraph(say!("preferences-signing-no-repository"), cx)),
            ],
        }
    }

    /// The section of the open repository's large files.
    fn lfs_section(&self, cx: &mut Context<Self>) -> Vec<Group> {
        match self.repo.as_ref().and_then(|repo| repo.workspace.upgrade()) {
            Some(workspace) => {
                let view = workspace.read(cx).lfs_view();
                vec![Group::new().own(move |_, _| view.clone().into_any_element())]
            }
            None => Vec::new(),
        }
    }

    /// The section of the open repository's upkeep.
    fn maintenance_section(&self, cx: &mut Context<Self>) -> Vec<Group> {
        match self.repo.as_ref().and_then(|repo| repo.workspace.upgrade()) {
            Some(workspace) => {
                let view = workspace.read(cx).maintenance_view();
                vec![Group::new().own(move |_, _| view.clone().into_any_element())]
            }
            None => Vec::new(),
        }
    }

    /// Gitflow's fields for the open repository, filled from what its
    /// settings are, made the first time the section is shown there.
    fn ensure_flow_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(repo) = self.repo.clone() else {
            return;
        };
        if self
            .flow_fields
            .as_ref()
            .is_some_and(|fields| fields.path == repo.path)
        {
            return;
        }
        let Some(workspace) = repo.workspace.upgrade() else {
            return;
        };
        let settings = {
            let workspace = workspace.read(cx);
            flow::FlowSettings::of(workspace.flow_state(), &workspace.local_branch_names())
        };
        let mut values = vec![settings.main.clone(), settings.develop.clone()];
        values.extend(settings.prefixes.iter().cloned());
        values.push(settings.tag.clone());
        let inputs: Vec<Entity<InputState>> = values
            .into_iter()
            .map(|value| cx.new(|cx| InputState::new(window, cx).default_value(value)))
            .collect();
        let subscriptions = inputs
            .iter()
            .map(|input| {
                cx.subscribe(input, |_, _, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        cx.notify();
                    }
                })
            })
            .collect();
        self.flow_fields = Some(FlowFields {
            path: repo.path,
            inputs,
            _subscriptions: subscriptions,
        });
    }

    /// The section of gitflow's two branches and its prefixes.
    fn gitflow_section(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let (Some(repo), Some(fields)) = (self.repo.clone(), self.flow_fields.as_ref()) else {
            return Vec::new();
        };
        let Some(workspace) = repo.workspace.upgrade() else {
            return Vec::new();
        };
        let typed: Vec<String> = fields
            .inputs
            .iter()
            .map(|input| input.read(cx).value().trim().to_owned())
            .collect();
        let settings = flow_settings_from(&typed);
        let (state, local) = {
            let workspace = workspace.read(cx);
            (
                workspace.flow_state().clone(),
                workspace.local_branch_names(),
            )
        };
        let review = settings.reviewed(&state, &local);
        let set_up = state.initialized;
        let unchanged = set_up && flow::FlowSettings::of(&state, &local) == settings;
        let unfilled = typed.iter().take(7).any(String::is_empty);
        let field_row = |words: &'static str, ix: usize, note: Option<&'static str>| {
            let input = fields.inputs.get(ix).cloned();
            row::item_of(words, move |_, _| Parts {
                control: input.as_ref().map(|input| {
                    row::field(format!("flow-{ix}").into(), words, input).into_any_element()
                }),
                under: note.map(|note| div().child(note).into_any_element()),
            })
        };
        let danger = cx.theme().danger;
        let muted = cx.theme().muted_foreground;
        let save = {
            let workspace = repo.workspace.clone();
            let settings = settings.clone();
            let refused = review.is_err() || unfilled || unchanged;
            let said = match &review {
                Err(why) => Some((why.clone(), danger)),
                Ok(Some(remark)) => Some((remark.clone(), muted)),
                Ok(None) => None,
            };
            row::item_of("", move |_, _| {
                let (workspace, settings) = (workspace.clone(), settings.clone());
                Parts {
                    control: Some(
                        Button::new("flow-save")
                            .primary()
                            .small()
                            .h(px(row::BUTTON_HEIGHT))
                            .label(if set_up {
                                say!("preferences-flow-save")
                            } else {
                                say!("preferences-flow-set-up")
                            })
                            .disabled(refused)
                            .on_click(move |_, window, cx| {
                                let settings = settings.clone();
                                let _ = workspace.update(cx, |workspace, cx| {
                                    workspace.save_flow_settings(settings, window, cx)
                                });
                            })
                            .into_any_element(),
                    ),
                    under: said.clone().map(|(text, colour)| {
                        div().text_color(colour).child(text).into_any_element()
                    }),
                }
            })
        };
        vec![
            Group::titled(say!("preferences-flow-branches"))
                .item(field_row(say!("preferences-flow-main"), 0, None))
                .item(field_row(
                    say!("preferences-flow-develop"),
                    1,
                    Some(say!("preferences-flow-develop-note")),
                )),
            Group::titled(say!("preferences-flow-prefixes"))
                .item(field_row(say!("preferences-flow-feature"), 2, None))
                .item(field_row(say!("preferences-flow-bugfix"), 3, None))
                .item(field_row(say!("preferences-flow-release"), 4, None))
                .item(field_row(say!("preferences-flow-hotfix"), 5, None))
                .item(field_row(say!("preferences-flow-support"), 6, None))
                .item(field_row(
                    say!("preferences-flow-version-tag"),
                    7,
                    Some(say!("preferences-flow-version-tag-note")),
                ))
                .item(save),
        ]
    }

    /// The section of the hooks git runs in the open repository.
    fn hooks_section(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let found = self.hooks.as_ref().map(|(_, found)| found.clone());
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let mono = cx.theme().mono_font_family.clone();
        let (directory, configured, hooks, problem) = match found {
            Some(Ok(list)) => (list.directory, list.configured, list.hooks, None),
            Some(Err(why)) => (String::new(), false, Vec::new(), Some(why)),
            None => (String::new(), false, Vec::new(), None),
        };
        vec![
            Group::new()
                .wide(|_, cx| page::paragraph(say!("preferences-hooks-intro"), cx))
                .item(row::item_of(
                    say!("preferences-hooks-directory"),
                    move |_, _| Parts {
                        control: Some(
                            div()
                                .w(px(row::CONTROL_WIDTH))
                                .truncate()
                                .font_family(mono.clone())
                                .text_sm()
                                .child(directory.clone())
                                .into_any_element(),
                        ),
                        under: Some(match &problem {
                            Some(why) => div()
                                .text_color(danger)
                                .child(why.clone())
                                .into_any_element(),
                            None => div()
                                .child(if configured {
                                    say!("preferences-hooks-configured")
                                } else {
                                    say!("preferences-hooks-own")
                                })
                                .into_any_element(),
                        }),
                    },
                ))
                .item(row::item_of(
                    say!("preferences-hooks-that-run"),
                    move |_, _| Parts {
                        control: None,
                        under: Some(if hooks.is_empty() {
                            div()
                                .child(say!("preferences-hooks-none"))
                                .into_any_element()
                        } else {
                            v_flex()
                                .children(hooks.iter().map(|(name, path)| {
                                    h_flex()
                                        .gap_2()
                                        .child(
                                            div()
                                                .w(px(150.))
                                                .flex_none()
                                                .text_color(muted)
                                                .child(name.clone()),
                                        )
                                        .child(div().min_w_0().truncate().child(path.clone()))
                                }))
                                .into_any_element()
                        }),
                    },
                )),
        ]
    }

    /// The groups of the section that is shown.
    fn groups(&self, cx: &mut Context<Self>) -> Vec<Group> {
        match self.chosen {
            Section::General => self.general(cx),
            Section::Profiles => self.profiles_section(cx),
            Section::Ssh => self.ssh_section(cx),
            Section::Tools => self.tools(cx),
            Section::Interface => self.interface(cx),
            Section::Signing => self.signing_section(cx),
            Section::Gitflow => self.gitflow_section(cx),
            Section::Hooks => self.hooks_section(cx),
            Section::Lfs => self.lfs_section(cx),
            Section::Maintenance => self.maintenance_section(cx),
            Section::Editor => self.editor(),
            Section::Terminal => self.terminal_section(cx),
            // A page of its own (see `render`).
            Section::Integrations => Vec::new(),

            Section::Experimental => self.experimental(cx),
            Section::Keys => self.keys(cx),
            Section::Files => self.files(cx),
        }
    }

    /// Ask the executable in the field what it is and, when it is git, run
    /// it from now on and remember it. An empty field is the default.
    fn use_git(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.git_busy {
            return;
        }
        let typed = self.git_input.read(cx).value().trim().to_owned();
        let program = (!typed.is_empty()).then_some(typed);
        self.git_busy = true;
        self.git_note = None;
        cx.notify();
        let run = crate::session::run_standalone("git.use", json!({ "program": program }));
        cx.spawn_in(window, async move |this, cx| {
            let reply = run.reply.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.git_busy = false;
                match reply {
                    Ok(Reply::Command(value)) => {
                        let info = value.get("info").filter(|info| info.is_object());
                        let git = match info {
                            Some(info) => GitExecutable::from_json(info),
                            None => GitExecutable::failed(
                                say!("preferences-git-none-at-all").to_owned(),
                            ),
                        };
                        this.git_note = Some(match (&git.version, &program) {
                            (Some(version), Some(program)) => {
                                (true, say!("preferences-git-in-use", version, program))
                            }
                            (Some(version), None) if git.shipped => {
                                (true, say!("preferences-git-shipped", version))
                            }
                            (Some(version), None) => {
                                (true, say!("preferences-git-found-again", version))
                            }
                            (None, _) => (false, say!("preferences-git-not-found").to_owned()),
                        });
                        cx.set_global(git);
                        change(cx, |preferences| preferences.git_executable = program);
                        cx.emit(PreferencesEvent::GitChanged);
                    }
                    Ok(Reply::Error(err)) => {
                        this.git_note = Some((false, crate::shell::sentence(&err)));
                    }
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn general(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let modes: Vec<(SharedString, SharedString)> = PULL_MODES
            .iter()
            .map(|(id, label)| ((*id).into(), label.text().into()))
            .collect();
        vec![
            Group::new()
                .item(row::number(
                    say!("preferences-auto-fetch"),
                    Note::plain(say!("preferences-auto-fetch-note")),
                    Number {
                        min: 0.0,
                        max: 60.0,
                        get: |preferences| f64::from(preferences.auto_fetch_minutes),
                        set: |preferences, minutes| {
                            preferences.auto_fetch_minutes = minutes.clamp(0.0, 60.0) as u32
                        },
                    },
                ))
                .item(row::check(
                    say!("preferences-prune"),
                    Note::plain(say!("preferences-prune-note")),
                    |preferences| preferences.fetch_prune,
                    |preferences, on| preferences.fetch_prune = on,
                ))
                .item(row::check(
                    say!("preferences-fetch-tags"),
                    Note::plain(say!("preferences-fetch-tags-note")),
                    |preferences| preferences.fetch_tags,
                    |preferences, on| preferences.fetch_tags = on,
                ))
                .item(row::select(
                    say!("preferences-pull-mode"),
                    Note::plain(say!("preferences-pull-mode-note")),
                    modes,
                    |cx| cx.global::<Prefs>().preferences.pull_mode.clone().into(),
                    |value, cx| change(cx, |preferences| preferences.pull_mode = value.to_string()),
                ))
                .item(row::check(
                    say!("preferences-checkout-stash"),
                    Note::plain(say!("preferences-checkout-stash-note")),
                    |preferences| preferences.checkout_stash,
                    |preferences, on| preferences.checkout_stash = on,
                ))
                .item(row::text(
                    say!("preferences-protected"),
                    Note::plain(say!("preferences-protected-note")),
                    Typed {
                        show: |preferences| preferences.protected_branches.join(", "),
                        take: |preferences, typed| {
                            preferences.protected_branches = names_from(typed)
                        },
                    },
                    false,
                ))
                .item(row::number(
                    say!("preferences-stale-days"),
                    Note::plain(say!("preferences-stale-days-note")),
                    Number {
                        min: 1.0,
                        max: 3650.0,
                        get: |preferences| f64::from(preferences.stale_days),
                        set: |preferences, days| {
                            preferences.stale_days = days.clamp(1.0, 3650.0) as u32
                        },
                    },
                )),
            // Issues of GitHub and GitLab.
            Group::titled(say!("preferences-issues"))
                .item(row::check(
                    say!("preferences-issue-trailer"),
                    Note::plain(say!("preferences-issue-trailer-note")),
                    |preferences| preferences.issue_trailer,
                    |preferences, on| preferences.issue_trailer = on,
                ))
                .item(row::text(
                    say!("preferences-issue-branch"),
                    Note::plain(say!("preferences-issue-branch-note")),
                    Typed {
                        show: |preferences| preferences.issue_branch.clone(),
                        take: |preferences, typed| {
                            let typed = typed.trim();
                            preferences.issue_branch = if typed.is_empty() || typed.len() > 255 {
                                crate::prefs::default_issue_branch()
                            } else {
                                typed.to_owned()
                            };
                        },
                    },
                    false,
                )),
            // The updater: off until it is turned on.
            Group::titled(say!("preferences-updates"))
                .item(row::check(
                    say!("preferences-look-for-updates"),
                    Note::plain(crate::updates::switch_note()),
                    |preferences| preferences.updates,
                    |preferences, on| preferences.updates = on,
                ))
                .item(updates_item(cx.entity().downgrade())),
        ]
    }

    fn interface(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let themes: Vec<(SharedString, SharedString)> = crate::theme::choices(cx)
            .iter()
            .map(|choice| {
                (
                    choice.key().to_owned().into(),
                    crate::theme::label(choice).into(),
                )
            })
            .collect();
        let words =
            |choices: &[(&str, bitbraid_strings::Message)]| -> Vec<(SharedString, SharedString)> {
                choices
                    .iter()
                    .map(|(word, label)| ((*word).to_owned().into(), label.text().into()))
                    .collect()
            };
        let column = |words: &'static str, column: Column, note: &'static str| {
            row::check(
                words,
                Note::plain(note),
                move |preferences| preferences.columns.order.contains(&column),
                move |preferences, on| {
                    let order = &mut preferences.columns.order;
                    order.retain(|shown| *shown != column);
                    if on {
                        order.push(column);
                    }
                },
            )
        };
        vec![
            Group::new()
                .item(row::select(
                    say!("preferences-theme"),
                    Note::None,
                    themes,
                    |cx| {
                        cx.global::<Prefs>()
                            .preferences
                            .theme
                            .key()
                            .to_owned()
                            .into()
                    },
                    |value, cx| {
                        crate::theme::choose(ThemeChoice::from_key(&value), cx);
                    },
                ))
                .item(self.themes_item(cx))
                .item(row::number(
                    say!("preferences-zoom"),
                    Note::plain(say!(
                        "preferences-zoom-note",
                        larger = crate::keymap::display_key(&crate::keymap::here("ctrl-=")),
                        smaller = crate::keymap::display_key(&crate::keymap::here("ctrl--")),
                    )),
                    Number {
                        min: 50.0,
                        max: 300.0,
                        get: |preferences| f64::from(preferences.zoom * 100.0).round(),
                        set: |preferences, per_cent| preferences.zoom = (per_cent / 100.0) as f32,
                    },
                ))
                .item(row::check(
                    say!("preferences-reduce-motion"),
                    Note::plain(say!("preferences-reduce-motion-note")),
                    |preferences| preferences.reduce_motion,
                    |preferences, on| preferences.reduce_motion = on,
                )),
            Group::titled(say!("preferences-dates"))
                .ruled()
                .item(row::check(
                    say!("preferences-dates-in-full"),
                    Note::plain(say!("preferences-dates-in-full-note")),
                    |preferences| preferences.columns.absolute_dates,
                    |preferences, on| preferences.columns.absolute_dates = on,
                )),
            Group::new()
                .item(row::check(
                    say!("preferences-toolbar-labels"),
                    Note::plain(say!("preferences-toolbar-labels-note")),
                    |preferences| preferences.toolbar_labels,
                    |preferences, on| preferences.toolbar_labels = on,
                ))
                .item(self.pictures_item(cx))
                .item(provider_pictures_item())
                .item(column(
                    say!("preferences-column-author"),
                    Column::Author,
                    say!("preferences-column-author-note"),
                ))
                .item(column(
                    say!("preferences-column-date"),
                    Column::Date,
                    say!("preferences-column-date-note"),
                ))
                .item(column(
                    say!("preferences-column-id"),
                    Column::Id,
                    say!("preferences-column-id-note"),
                ))
                .item(row::check(
                    say!("preferences-date-dividers"),
                    Note::plain(say!("preferences-date-dividers-note")),
                    |preferences| preferences.columns.date_dividers,
                    |preferences, on| preferences.columns.date_dividers = on,
                ))
                .item(row::select(
                    say!("preferences-graph-order"),
                    Note::Marked(say!("preferences-graph-order-note")),
                    words(crate::prefs::GRAPH_ORDERS),
                    |cx| cx.global::<Prefs>().preferences.graph_order.clone().into(),
                    |value, cx| {
                        change(cx, |preferences| {
                            preferences.graph_order = value.to_string()
                        })
                    },
                ))
                .item(row::select(
                    say!("preferences-graph-date"),
                    Note::plain(say!("preferences-graph-date-note")),
                    words(crate::prefs::GRAPH_DATES),
                    |cx| cx.global::<Prefs>().preferences.graph_date.clone().into(),
                    |value, cx| {
                        change(cx, |preferences| preferences.graph_date = value.to_string())
                    },
                ))
                .item(row::check(
                    say!("preferences-left-panel"),
                    Note::plain(say!("preferences-left-panel-note")),
                    |preferences| preferences.left_panel,
                    |preferences, on| preferences.left_panel = on,
                ))
                .item(row::check(
                    say!("preferences-right-panel"),
                    Note::plain(say!("preferences-right-panel-note")),
                    |preferences| preferences.right_panel,
                    |preferences, on| preferences.right_panel = on,
                ))
                .item(row::check(
                    say!("preferences-changes-tree"),
                    Note::plain(say!("preferences-changes-tree-note")),
                    |preferences| preferences.changes_tree,
                    |preferences, on| preferences.changes_tree = on,
                )),
        ]
    }

    /// What the diff viewer's own buttons choose; they write here too.
    fn editor(&self) -> Vec<Group> {
        vec![
            Group::new()
                .item(row::select(
                    say!("preferences-layout"),
                    Note::None,
                    vec![
                        ("hunks".into(), say!("preferences-layout-hunks").into()),
                        (
                            "one_column".into(),
                            say!("preferences-layout-one-column").into(),
                        ),
                        (
                            "two_columns".into(),
                            say!("preferences-layout-two-columns").into(),
                        ),
                    ],
                    |cx| cx.global::<Prefs>().preferences.diff.layout.clone().into(),
                    |value, cx| {
                        change(cx, |preferences| {
                            preferences.diff.layout = value.to_string()
                        })
                    },
                ))
                .item(row::select(
                    say!("preferences-whitespace"),
                    Note::None,
                    vec![
                        ("exact".into(), say!("preferences-whitespace-counts").into()),
                        (
                            "ignore_trailing".into(),
                            say!("preferences-whitespace-ignored-at-ends").into(),
                        ),
                        (
                            "ignore_all".into(),
                            say!("preferences-whitespace-ignored").into(),
                        ),
                        (
                            "ignore_cr_at_eol".into(),
                            say!("preferences-whitespace-cr-ignored").into(),
                        ),
                    ],
                    |cx| {
                        cx.global::<Prefs>()
                            .preferences
                            .diff
                            .whitespace
                            .clone()
                            .into()
                    },
                    |value, cx| {
                        change(cx, |preferences| {
                            preferences.diff.whitespace = value.to_string()
                        })
                    },
                ))
                .item(row::check(
                    say!("preferences-word-diff"),
                    Note::plain(say!("preferences-word-diff-note")),
                    |preferences| preferences.diff.words,
                    |preferences, on| preferences.diff.words = on,
                ))
                .item(row::check(
                    say!("preferences-wrap"),
                    Note::plain(say!("preferences-wrap-note")),
                    |preferences| preferences.diff.wrap,
                    |preferences, on| preferences.diff.wrap = on,
                )),
        ]
    }

    /// Whether the pictures of authors are asked of a service, with what
    /// that sends and to whom.
    fn pictures_item(&self, cx: &App) -> row::Item {
        let whom = crate::avatars::whom(cx);
        let title = match &whom.host {
            Some(host) => say!("preferences-pictures-from", host),
            None => say!("preferences-pictures").to_owned(),
        };
        row::check(
            title,
            Note::plain(pictures_text(&whom)),
            |preferences| preferences.gravatar,
            |preferences, on| preferences.gravatar = on,
        )
    }

    /// The folder of the user's own themes: where it is, what is wrong in
    /// it, and a file to start from.
    fn themes_item(&self, cx: &mut Context<Self>) -> row::Item {
        let view = cx.entity().downgrade();
        row::item_of(say!("preferences-own-themes"), move |_, cx| {
            let user = cx.global::<crate::theme::UserThemes>();
            let dir = user.dir.as_ref().map(|dir| dir.display().to_string());
            let (count, problems) = (user.themes.len(), user.problems.clone());
            let chosen = cx.global::<Prefs>().preferences.theme.clone();
            let missing = !crate::theme::exists(&chosen, cx);
            let danger = cx.theme().danger;
            let for_example = view.clone();
            let control = dir.as_ref().map(|_| {
                Button::new("theme-example")
                    .outline()
                    .small()
                    .label(say!("preferences-own-themes-example"))
                    .on_click(move |_, _, cx| {
                        let _ =
                            for_example.update(cx, |_, cx| cx.emit(PreferencesEvent::ExampleTheme));
                    })
                    .into_any_element()
            });
            let under = v_flex()
                .gap_1()
                .child(div().child(match &dir {
                    None => say!("preferences-own-themes-no-place").to_owned(),
                    Some(dir) => say!("preferences-own-themes-count", count, dir),
                }))
                .when(missing, |column| {
                    column.child(div().text_color(danger).child(say!(
                        "preferences-own-theme-missing",
                        theme = format!("{:?}", chosen.key())
                    )))
                })
                .children(
                    problems
                        .into_iter()
                        .take(8)
                        .map(|said| div().text_color(danger).child(said)),
                )
                .child(div().child(say!("preferences-own-themes-note")))
                .into_any_element();
            Parts {
                control,
                under: Some(under),
            }
        })
    }

    fn tools(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let mut groups = Vec::new();
        for tool in Tool::ALL {
            let preferences = &cx.global::<Prefs>().preferences;
            let found = self.found_for(tool);
            let stored = tool.get(preferences).clone();
            let custom = self.custom_tools.contains(&tool);
            let chosen = chosen_tool(&found, stored.as_deref(), custom);
            let mut choices: Vec<(SharedString, SharedString)> =
                vec![(DEFAULT_TOOL.into(), tool.default_label().into())];
            choices.extend(
                found
                    .iter()
                    .enumerate()
                    .map(|(ix, (name, _))| (format!("found-{ix}").into(), name.clone().into())),
            );
            choices.push((CUSTOM_TOOL.into(), say!("preferences-tool-custom").into()));
            let view = cx.entity().downgrade();
            let pick: row::Pick = Rc::new(move |word, _, cx| {
                let found = found.clone();
                let _ = view.update(cx, |this, cx| {
                    this.custom_tools.retain(|other| *other != tool);
                    match word.as_ref() {
                        DEFAULT_TOOL => change(cx, |preferences| tool.set(preferences, None)),
                        CUSTOM_TOOL => this.custom_tools.push(tool),
                        found_word => {
                            let command = found_word
                                .strip_prefix("found-")
                                .and_then(|ix| ix.parse::<usize>().ok())
                                .and_then(|ix| found.get(ix))
                                .map(|(_, command)| command.clone());
                            if let Some(command) = command {
                                change(cx, |preferences| tool.set(preferences, Some(command)));
                            }
                        }
                    }
                    cx.notify();
                });
            });
            let mut group = Group::titled(tool.heading()).item(row::choose(
                tool.words(),
                Note::plain(tool.note()),
                choices,
                chosen.clone().into(),
                pick,
            ));
            if chosen == CUSTOM_TOOL {
                group = group.item(row::text(
                    tool.command_words(),
                    Note::Marked(tool.example()),
                    tool.typed(),
                    true,
                ));
            }
            groups.push(group);
        }
        groups
    }

    /// What was found on this system for a tool: what the list calls it,
    /// and its command.
    fn found_for(&self, tool: Tool) -> Vec<(String, String)> {
        match tool {
            Tool::Merge => self
                .tools
                .iter()
                .filter_map(|found| Some((found.name.clone(), found.merge.clone()?)))
                .collect(),
            Tool::Diff => self
                .tools
                .iter()
                .filter_map(|found| Some((found.name.clone(), found.diff.clone()?)))
                .collect(),
            Tool::Terminal => self
                .terminals
                .iter()
                .map(|command| {
                    let program = command.split_whitespace().next().unwrap_or(command);
                    (program.to_owned(), command.clone())
                })
                .collect(),
            Tool::Editor | Tool::FileManager => Vec::new(),
        }
    }

    fn experimental(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let view = cx.entity().downgrade();
        let (git_input, git_note, git_busy) =
            (self.git_input.clone(), self.git_note.clone(), self.git_busy);
        vec![
            Group::titled(say!("preferences-git-executable")).item(row::item_of(
                say!("preferences-git-program"),
                move |_, cx| {
                    let (input, note, busy) = (git_input.clone(), git_note.clone(), git_busy);
                    let git = cx.global::<GitExecutable>().clone();
                    let theme = cx.theme();
                    let (foreground, danger, success) =
                        (theme.foreground, theme.danger, theme.success);
                    let found = git.in_use();
                    let (for_use, for_default) = (view.clone(), view.clone());
                    // What was given and would not do is outlined.
                    let refused = note.as_ref().is_some_and(|(ok, _)| !ok);
                    let control = div()
                        .w(px(row::CONTROL_WIDTH))
                        .flex_none()
                        .when(refused, |field| {
                            field.rounded(px(6.)).border_1().border_color(danger)
                        })
                        .child(
                            Input::new(&input)
                                .id("field-git")
                                .min_h(px(row::CONTROL_HEIGHT))
                                .w_full(),
                        )
                        .into_any_element();
                    // The buttons that act stand under the field, at its edge.
                    let buttons = h_flex()
                        .gap(px(5.))
                        .pb(px(6.))
                        .child(
                            Button::new("git-use")
                                .primary()
                                .small()
                                .h(px(row::BUTTON_HEIGHT))
                                .label(say!("preferences-git-use"))
                                .disabled(busy)
                                .on_click(move |_, window, cx| {
                                    let _ = for_use.update(cx, |this, cx| this.use_git(window, cx));
                                }),
                        )
                        .child(
                            Button::new("git-default")
                                .outline()
                                .small()
                                .h(px(row::BUTTON_HEIGHT))
                                .label(say!("preferences-git-default"))
                                .disabled(busy)
                                .on_click(move |_, window, cx| {
                                    let _ = for_default.update(cx, |this, cx| {
                                        this.git_input.update(cx, |state, cx| {
                                            state.set_value("", window, cx)
                                        });
                                        this.use_git(window, cx);
                                    });
                                }),
                        );
                    let under = v_flex()
                        .gap_1()
                        .child(buttons)
                        .child(
                            div()
                                .text_sm()
                                .text_color(if git.error.is_some() {
                                    danger
                                } else {
                                    foreground
                                })
                                .child(found),
                        )
                        .children(
                            git.warning()
                                .filter(|_| git.error.is_none())
                                .map(|warning| div().text_sm().text_color(danger).child(warning)),
                        )
                        .children(note.map(|(ok, text)| {
                            div()
                                .text_sm()
                                .text_color(if ok { success } else { danger })
                                .child(text)
                        }))
                        .child(div().child(say!(
                            "preferences-git-note",
                            minimum = if git.minimum.is_empty() {
                                "2.38.0"
                            } else {
                                git.minimum.as_str()
                            }
                        )))
                        .into_any_element();
                    Parts {
                        control: Some(control),
                        under: Some(under),
                    }
                },
            )),
        ]
    }

    fn keys(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let view = cx.entity().downgrade();
        vec![
            Group::new().item(row::item_of(say!("preferences-keys"), move |_, cx| {
                let keys = cx.global::<crate::keymap::Keys>();
                let own = keys.resolved.bound.iter().filter(|bound| bound.own).count();
                let complaints = keys.complaints();
                let file = keys.file.as_ref().map(|file| file.display().to_string());
                let theme = cx.theme();
                let (foreground, danger) = (theme.foreground, theme.danger);
                let (for_sheet, for_reload, for_reset) = (view.clone(), view.clone(), view.clone());
                let control = h_flex()
                    .gap(px(5.))
                    .child(
                        Button::new("keys-sheet")
                            .primary()
                            .small()
                            .label(say!("preferences-keys-change"))
                            .on_click(move |_, _, cx| {
                                let _ = for_sheet
                                    .update(cx, |_, cx| cx.emit(PreferencesEvent::Shortcuts));
                            }),
                    )
                    .child(
                        Button::new("keys-reload")
                            .outline()
                            .small()
                            .label(say!("preferences-keys-reload"))
                            .on_click(move |_, _, cx| {
                                let _ = for_reload
                                    .update(cx, |_, cx| cx.emit(PreferencesEvent::ReloadKeys));
                            }),
                    )
                    .when(own > 0, |row| {
                        row.child(
                            Button::new("keys-reset")
                                .outline()
                                .small()
                                .label(say!("preferences-keys-reset"))
                                .on_click(move |_, _, cx| {
                                    let _ = for_reset
                                        .update(cx, |_, cx| cx.emit(PreferencesEvent::ResetKeys));
                                }),
                        )
                    })
                    .into_any_element();
                let under = v_flex()
                    .gap_1()
                    .child(div().text_sm().text_color(foreground).child(match &file {
                        None => say!("preferences-keys-no-place").to_owned(),
                        Some(file) => say!("preferences-keys-own", own, file),
                    }))
                    .children(
                        complaints
                            .into_iter()
                            .map(|said| div().text_sm().text_color(danger).child(said)),
                    )
                    .child(div().child(say!(
                        "preferences-keys-note",
                        key = crate::keymap::here("ctrl-alt-p")
                    )))
                    .into_any_element();
                Parts {
                    control: Some(control),
                    under: Some(under),
                }
            })),
        ]
    }

    fn files(&self, cx: &mut Context<Self>) -> Vec<Group> {
        let dir = cx
            .global::<Prefs>()
            .dir()
            .map(|dir| dir.display().to_string());
        vec![Group::new().item(row::item_of(
            say!("preferences-files-folder"),
            move |_, cx| {
                let foreground = cx.theme().foreground;
                let mono = cx.theme().mono_font_family.clone();
                let line = |name: &'static str, what: &'static str| {
                    h_flex()
                        .w_full()
                        .gap_3()
                        .items_start()
                        .child(
                            div()
                                .w(px(130.))
                                .flex_none()
                                .text_color(foreground)
                                .font_family(mono.clone())
                                .child(name),
                        )
                        .child(div().flex_1().min_w_0().child(what))
                };
                let control = div()
                    .text_sm()
                    .whitespace_normal()
                    .child(match &dir {
                        Some(dir) => say!("preferences-files-everything-in", dir),
                        None => say!("preferences-files-no-place").to_owned(),
                    })
                    .into_any_element();
                let under = v_flex()
                    .gap_1()
                    .child(line(
                        "preferences.toml",
                        say!("preferences-files-preferences"),
                    ))
                    .child(line("state.toml", say!("preferences-files-state")))
                    .child(line(
                        "keybindings.toml",
                        say!("preferences-files-keybindings"),
                    ))
                    .child(line("profiles.toml", say!("preferences-files-profiles")))
                    .into_any_element();
                Parts {
                    control: Some(control),
                    under: Some(under),
                }
            },
        ))]
    }
}

impl Focusable for PreferencesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PreferencesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        match self.chosen {
            Section::Gitflow => self.ensure_flow_fields(window, cx),
            Section::Hooks => self.load_hooks(cx),
            _ => {}
        }
        let rows = self.column_rows(cx);
        let column = page::column(rows, &self.column_scroll, cx);
        // The integrations have a column of providers of their own between
        // this one and the page.
        if self.chosen == Section::Integrations {
            return h_flex()
                .id("preferences")
                .track_focus(&self.focus_handle)
                .size_full()
                .bg(theme.background)
                .child(column)
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .h_full()
                        .child(self.integrations.clone()),
                );
        }
        let groups = self.groups(cx);
        let heading = SharedString::from(self.chosen.name());
        h_flex()
            .id("preferences")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(theme.background)
            .child(column)
            .child(page::section(
                heading,
                groups,
                &self.section_scroll,
                window,
                cx,
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::{command_value, names_from, terminal_commands};

    #[test]
    fn the_terminals_found_are_read() {
        let found = serde_json::json!({
            "default": { "name": "kitty", "command": "kitty --directory {path}" },
            "terminals": [
                { "name": "kitty", "command": "kitty --directory {path}" },
                { "name": "xterm", "command": "xterm" },
                { "name": "nameless" },
            ],
        });
        assert_eq!(
            terminal_commands(&found),
            ["kitty --directory {path}", "xterm"]
        );
        assert!(terminal_commands(&serde_json::json!({ "terminals": [] })).is_empty());
        assert!(terminal_commands(&serde_json::json!("nonsense")).is_empty());
    }

    #[test]
    fn the_tools_found_are_read_with_the_commands_they_have() {
        let value = serde_json::json!({
            "found": [
                { "name": "Meld", "program": "meld", "path": "/usr/bin/meld",
                  "diff": "meld \"$LOCAL\" \"$REMOTE\"",
                  "merge": "meld \"$LOCAL\" \"$BASE\" \"$REMOTE\" --output \"$MERGED\"" },
                { "name": "Kompare", "program": "kompare", "diff": "kompare \"$LOCAL\" \"$REMOTE\"",
                  "merge": null },
                { "name": "No commands", "diff": "  ", "merge": null },
                { "program": "nameless", "diff": "x" },
                7,
            ],
        });
        let found = super::found_tools(&value);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Meld");
        assert_eq!(
            found[0].diff.as_deref(),
            Some("meld \"$LOCAL\" \"$REMOTE\"")
        );
        assert!(
            found[0]
                .merge
                .as_deref()
                .is_some_and(|merge| merge.contains("$MERGED"))
        );
        assert_eq!(
            (found[1].name.as_str(), &found[1].merge),
            ("Kompare", &None)
        );
        assert!(super::found_tools(&serde_json::json!({ "found": 3 })).is_empty());
        assert!(super::found_tools(&serde_json::json!(null)).is_empty());
    }

    #[test]
    fn a_tools_list_shows_what_the_preferences_hold() {
        use super::chosen_tool;
        let found = [
            ("Meld".to_owned(), "meld $LOCAL $REMOTE".to_owned()),
            ("Kompare".to_owned(), "kompare $LOCAL $REMOTE".to_owned()),
        ];
        assert_eq!(chosen_tool(&found, None, false), "default");
        assert_eq!(chosen_tool(&found, Some("  "), false), "default");
        assert_eq!(chosen_tool(&found, None, true), "custom", "being typed");
        assert_eq!(
            chosen_tool(&found, Some("kompare $LOCAL $REMOTE"), false),
            "found-1"
        );
        assert_eq!(chosen_tool(&found, Some("vimdiff"), false), "custom");
        assert_eq!(chosen_tool(&[], Some("vimdiff"), true), "custom");
    }

    #[test]
    fn gitflows_fields_are_its_settings_in_order() {
        let typed = ["trunk", "next", "f/", "b/", "r/", "h/", "s/", "v"].map(str::to_owned);
        let settings = super::flow_settings_from(&typed);
        assert_eq!(settings.main, "trunk");
        assert_eq!(settings.develop, "next");
        assert_eq!(
            settings.prefixes,
            ["f/", "b/", "r/", "h/", "s/"].map(str::to_owned)
        );
        assert_eq!(settings.tag, "v");
        let none = super::flow_settings_from(&[]);
        assert!(none.main.is_empty() && none.prefixes.iter().all(String::is_empty));
    }

    #[test]
    fn names_are_split_and_kept_once() {
        assert_eq!(names_from("main, master"), ["main", "master"]);
        assert_eq!(
            names_from("  trunk,release/1.x ,, trunk\tdevelop\n"),
            ["trunk", "release/1.x", "develop"]
        );
        assert!(names_from(" , ,").is_empty());
    }

    #[test]
    fn the_switch_for_pictures_says_what_is_sent_and_to_whom() {
        use crate::avatars::Whom;
        let gravatar = super::pictures_text(&Whom {
            host: Some("gravatar.com".to_owned()),
            secure: true,
            problem: None,
            kept_in: Some("/home/ada/.cache/bitbraid/avatars".into()),
        });
        for said in [
            "nothing is sent anywhere",
            "a SHA-256 hash of the e-mail address of each author",
            "is sent to gravatar.com over HTTPS",
            "network address",
            "nothing about the repository",
            "/home/ada/.cache/bitbraid/avatars",
        ] {
            assert!(gravatar.contains(said), "{said:?} is not in {gravatar:?}");
        }
        // A service of one's own is named in its place.
        let own = super::pictures_text(&Whom {
            host: Some("127.0.0.1:8080".to_owned()),
            secure: false,
            problem: None,
            kept_in: None,
        });
        assert!(own.contains("is sent to 127.0.0.1:8080 on this machine, without TLS"));
        assert!(!own.contains("gravatar"));
        assert!(!own.contains("kept in"));
        // An address that is none: it says that nobody is asked.
        let broken = super::pictures_text(&Whom {
            host: None,
            secure: false,
            problem: Some("neither https nor http".to_owned()),
            kept_in: None,
        });
        assert!(broken.contains("Nobody is asked"));
        assert!(broken.contains("neither https nor http"));
    }

    #[test]
    fn an_empty_command_is_no_command() {
        assert_eq!(command_value(""), None);
        assert_eq!(command_value("   "), None);
        assert_eq!(
            command_value(" code --goto {path}:{line} ").as_deref(),
            Some("code --goto {path}:{line}")
        );
    }

    /// A field filled again from the tidied preference after every key
    /// would lose a space as soon as it was typed: no command with an
    /// argument could be typed at all.
    #[test]
    fn what_is_being_typed_is_not_put_back_tidied() {
        use super::row::{Typed, overtaken};
        const EDITOR: Typed = Typed {
            show: |preferences| preferences.editor.clone().unwrap_or_default(),
            take: |preferences, typed| preferences.editor = command_value(typed),
        };
        let mut preferences = crate::prefs::Preferences::default();
        // On the way to `code --wait`: the preference is the tidied text,
        // and the field keeps what was typed.
        let mut held = (EDITOR.show)(&preferences);
        assert_eq!(held, "");
        for typed in ["code", "code ", "code -", "code --wait", " code", "  "] {
            held = EDITOR.typed(&mut preferences, typed);
            assert_eq!(held, typed.trim());
            assert!(!overtaken(&held, &(EDITOR.show)(&preferences)), "{typed:?}");
        }
        assert_eq!(preferences.editor, None);
        // Changed from elsewhere (a button that writes a command, the
        // file read again): the field follows.
        preferences.editor = Some("meld".to_owned());
        assert!(overtaken(&held, &(EDITOR.show)(&preferences)));
        let held = EDITOR.typed(&mut preferences, "code ");
        preferences.editor = None;
        assert!(overtaken(&held, &(EDITOR.show)(&preferences)));
    }
}
