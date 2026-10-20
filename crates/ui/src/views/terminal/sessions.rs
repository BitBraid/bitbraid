//! The window's terminals: one panel per repository or worktree folder,
//! kept by the window rather than by a tab (`AGENTS.md` section 2), so that
//! a worktree opened in a tab's place finds its own terminals when it is
//! opened there again, and a command keeps running while another tab or
//! another worktree is looked at. They end with the window, when the user
//! ends them, or when the last tab that showed their folder is closed.
//!
//! Sources: none (original design).

use std::path::{Path, PathBuf};

use gpui_kit::*;

use super::panel::{PanelEvent, TerminalPanel};

/// A shell's command line as the preferences hold it, in words, the way
/// a POSIX shell splits it: blanks part words, single quotes keep
/// everything, double quotes keep all but a backslash before `"`, `\`,
/// `$` or a backquote, a backslash outside quotes keeps the next
/// character. None when a quote is not closed or a word holds a nul.
pub fn command_words(line: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\0' => return None,
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '\'' => break,
                        '\0' => return None,
                        inner => word.push(inner),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\0' => return None,
                        '\\' => match chars.next()? {
                            kept @ ('"' | '\\' | '$' | '`') => word.push(kept),
                            '\0' => return None,
                            other => {
                                word.push('\\');
                                word.push(other);
                            }
                        },
                        inner => word.push(inner),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some('\0') => return None,
                    Some(next) => word.push(next),
                    None => word.push('\\'),
                }
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Some(words)
}

/// The shell `terminal.shell` answered: program, arguments, folder and
/// the whole environment. None when the answer is not one.
pub fn spec_of(value: &serde_json::Value) -> Option<bitbraid_terminal::ShellSpec> {
    let program = value.get("program")?.as_str()?.to_owned();
    let args = value
        .get("args")?
        .as_array()?
        .iter()
        .map(|arg| arg.as_str().map(str::to_owned))
        .collect::<Option<Vec<String>>>()?;
    let cwd = PathBuf::from(value.get("cwd")?.as_str()?);
    let mut env: Vec<(String, String)> = value
        .get("env")?
        .as_object()?
        .iter()
        .filter_map(|(name, value)| Some((name.clone(), value.as_str()?.to_owned())))
        .collect();
    env.sort();
    Some(bitbraid_terminal::ShellSpec {
        program,
        args,
        cwd,
        env,
    })
}

/// Start the shell `spec` names and the text that shows it: the one place
/// a session is made. What the session says is carried to the text on
/// the window's thread for as long as the returned task is kept.
pub fn open_text(
    spec: &bitbraid_terminal::ShellSpec,
    window: &mut Window,
    cx: &mut App,
) -> Result<super::panel::Running, String> {
    use futures::StreamExt as _;
    let (sender, mut receiver) = futures::channel::mpsc::unbounded();
    let session = bitbraid_terminal::Session::spawn(
        spec,
        bitbraid_terminal::GridSize::default(),
        move |event| {
            // Nobody listens once the text is gone: nothing to tell.
            let _ = sender.unbounded_send(event);
        },
    )
    .map_err(|err| {
        say!(
            "terminal-cannot-start",
            program = &spec.program,
            error = err.to_string()
        )
    })?;
    let text = cx.new(|cx| super::text::TerminalText::new(session, window, cx));
    let weak = text.downgrade();
    let forward = cx.spawn(async move |cx| {
        while let Some(event) = receiver.next().await {
            if weak
                .update(cx, |text, cx| text.on_session_event(event, cx))
                .is_err()
            {
                break;
            }
        }
    });
    Ok(super::panel::Running { text, forward })
}

/// Something one of the panels said, with the folder it is of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalsEvent {
    /// The folder of the panel.
    pub path: PathBuf,
    /// What it said.
    pub what: PanelEvent,
}

/// The terminals of a window, by folder.
#[derive(Default)]
pub struct Terminals {
    panels: Vec<(PathBuf, Entity<TerminalPanel>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TerminalsEvent> for Terminals {}

impl Terminals {
    /// No terminals yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The panel of a folder, when it has one.
    pub fn panel(&self, path: &Path) -> Option<Entity<TerminalPanel>> {
        self.panels
            .iter()
            .find(|(at, _)| at == path)
            .map(|(_, panel)| panel.clone())
    }

    /// The panel of a folder, made when it has none.
    pub fn panel_or_new(&mut self, path: &Path, cx: &mut Context<Self>) -> Entity<TerminalPanel> {
        if let Some(panel) = self.panel(path) {
            return panel;
        }
        let panel = cx.new(|_| TerminalPanel::new(path.to_path_buf()));
        let of = path.to_path_buf();
        self._subscriptions
            .push(cx.subscribe(&panel, move |_, _, event: &PanelEvent, cx| {
                cx.emit(TerminalsEvent {
                    path: of.clone(),
                    what: event.clone(),
                });
                cx.notify();
            }));
        self.panels.push((path.to_path_buf(), panel.clone()));
        panel
    }

    /// The folders that have a terminal.
    pub fn folders(&self, cx: &App) -> Vec<PathBuf> {
        self.panels
            .iter()
            .filter(|(_, panel)| !panel.read(cx).is_empty())
            .map(|(path, _)| path.clone())
            .collect()
    }

    /// End the terminals of every folder that is not among `kept`: no tab
    /// shows it, nor shows it again when a worktree is opened in its place.
    pub fn end_unless(&mut self, kept: &[PathBuf], cx: &mut Context<Self>) {
        for (path, panel) in &self.panels {
            if !kept.contains(path) {
                panel.update(cx, |panel, cx| panel.end_all(cx));
            }
        }
        self.panels.retain(|(path, _)| kept.contains(path));
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::{command_words, spec_of};

    fn words(line: &str) -> Option<Vec<String>> {
        command_words(line)
    }

    #[test]
    fn a_shells_command_line_is_split_as_a_shell_splits_it() {
        assert_eq!(words("").unwrap(), Vec::<String>::new());
        assert_eq!(words("   ").unwrap(), Vec::<String>::new());
        assert_eq!(words("zsh").unwrap(), ["zsh"]);
        assert_eq!(words("  fish   --login ").unwrap(), ["fish", "--login"]);
        assert_eq!(
            words("env 'PS1=$ ' /bin/sh").unwrap(),
            ["env", "PS1=$ ", "/bin/sh"]
        );
        assert_eq!(
            words(r#"/opt/my\ shells/bash -c "echo \"hi\" \$HOME \n""#).unwrap(),
            ["/opt/my shells/bash", "-c", r#"echo "hi" $HOME \n"#]
        );
        assert_eq!(words("a''b \"\"").unwrap(), ["ab", ""]);
        assert_eq!(words("trailing\\").unwrap(), ["trailing\\"]);
        for broken in ["'open", "\"open", "\"a\\", "nul\0", "'\0'"] {
            assert_eq!(words(broken), None, "{broken:?}");
        }
    }

    #[test]
    fn the_engines_answer_is_read_as_a_shell() {
        let value = serde_json::json!({
            "program": "/bin/sh",
            "args": ["-i"],
            "cwd": "/work/project",
            "worktree": "/work/project",
            "env": { "TERM": "xterm-256color", "HOME": "/home/me" },
            "kind": "posix",
        });
        let spec = spec_of(&value).unwrap();
        assert_eq!(spec.program, "/bin/sh");
        assert_eq!(spec.args, ["-i"]);
        assert_eq!(spec.cwd, std::path::Path::new("/work/project"));
        assert_eq!(spec.var("TERM"), Some("xterm-256color"));
        for broken in [
            serde_json::json!({}),
            serde_json::json!({ "program": 3, "args": [], "cwd": "/", "env": {} }),
            serde_json::json!({ "program": "sh", "args": [1], "cwd": "/", "env": {} }),
            serde_json::json!({ "program": "sh", "args": [], "cwd": "/" }),
        ] {
            assert!(spec_of(&broken).is_none(), "{broken}");
        }
    }
}
