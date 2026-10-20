//! What is started, and the size of the screen it is started on.
//!
//! Sources: the manuals of sh, bash, zsh, fish, nushell (`nu`,
//! `--login`), tcsh (`tcsh`, `csh`, `-l`), PowerShell (`pwsh`) and cmd
//! for the names their programs go by and how a login shell is asked for;
//! Microsoft Learn, "about_Pwsh" and "about_PowerShell_exe" (`-NoExit`,
//! `-Command`), "about_Prompts" (the `prompt` function) and
//! "Environment.CurrentDirectory".

use std::path::{Path, PathBuf};

/// The shell a session starts: the program, its arguments, the folder it
/// starts in and its whole environment. On Unix nothing of this process's
/// own environment reaches the shell unless it is listed here; on Windows
/// the list is added to it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellSpec {
    /// The program: a name looked up on the `PATH` of [`ShellSpec::env`],
    /// or a full path.
    pub program: String,
    pub args: Vec<String>,
    /// The folder it starts in. It has to be there.
    pub cwd: PathBuf,
    /// Its environment, complete on Unix.
    pub env: Vec<(String, String)>,
}

impl ShellSpec {
    /// The kind of shell the program is, for quoting paths.
    pub fn kind(&self) -> ShellKind {
        ShellKind::of_program(&self.program)
    }

    /// The value of a variable of its environment; the last one counts
    /// when a name is given twice. On Windows the case of a name does not
    /// count (`Path` is `PATH`).
    pub fn var(&self, name: &str) -> Option<&str> {
        self.env
            .iter()
            .rev()
            .find(|(key, _)| same_variable(key, name))
            .map(|(_, value)| value.as_str())
    }
}

/// Whether two names of environment variables name the same one: on
/// Windows the case does not count, elsewhere it does.
pub fn same_variable(a: &str, b: &str) -> bool {
    if cfg!(windows) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

/// The language a shell reads, as far as quoting a path goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ShellKind {
    /// sh, bash, zsh, dash, ksh and the others of the POSIX family; also
    /// what anything not known is taken for.
    #[default]
    Posix,
    /// fish.
    Fish,
    /// nushell (`nu`).
    Nushell,
    /// The C shells: tcsh and csh.
    Csh,
    /// PowerShell (`pwsh`, `powershell`).
    PowerShell,
    /// The Windows command prompt (`cmd`).
    Cmd,
}

impl ShellKind {
    /// The kind of the shell a program is, by its file name: the folder,
    /// the case and an `.exe` at the end do not count.
    pub fn of_program(program: &str) -> Self {
        // Both separators: a Windows path is read on any system.
        let base = program
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(program)
            .to_ascii_lowercase();
        let base = base.strip_suffix(".exe").unwrap_or(&base);
        // A login shell's name starts with a dash (`-bash`).
        let base = base.strip_prefix('-').unwrap_or(base);
        match base {
            "fish" => Self::Fish,
            "nu" => Self::Nushell,
            "tcsh" | "csh" => Self::Csh,
            "pwsh" | "powershell" | "pwsh-preview" => Self::PowerShell,
            "cmd" => Self::Cmd,
            _ => Self::Posix,
        }
    }

    /// Whether a shell of this kind is started as a login shell with
    /// `-l`: the POSIX shells (bash, zsh, sh, ksh, dash), fish, nushell and
    /// the C shells do.
    pub fn takes_login(self) -> bool {
        matches!(self, Self::Posix | Self::Fish | Self::Nushell | Self::Csh)
    }

    /// Every kind.
    pub const ALL: [Self; 6] = [
        Self::Posix,
        Self::Fish,
        Self::Nushell,
        Self::Csh,
        Self::PowerShell,
        Self::Cmd,
    ];

    /// Its name, as `terminal.shell` reports it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Posix => "posix",
            Self::Fish => "fish",
            Self::Nushell => "nushell",
            Self::Csh => "csh",
            Self::PowerShell => "powershell",
            Self::Cmd => "cmd",
        }
    }

    /// The kind a name of [`ShellKind::name`] stands for.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// The size of the screen: cells, and the size of a cell in pixels, which
/// a program may ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridSize {
    /// Cells across.
    pub columns: u16,
    /// Rows of cells.
    pub rows: u16,
    /// A cell's width in pixels.
    pub cell_width_px: u16,
    /// A cell's height in pixels.
    pub cell_height_px: u16,
}

impl Default for GridSize {
    fn default() -> Self {
        Self {
            columns: 80,
            rows: 24,
            cell_width_px: 8,
            cell_height_px: 16,
        }
    }
}

impl GridSize {
    /// The fewest columns a screen has.
    pub const MIN_COLUMNS: u16 = 2;
    /// The most columns a screen has: wider is cut.
    pub const MAX_COLUMNS: u16 = 2000;
    /// The fewest rows.
    pub const MIN_ROWS: u16 = 1;
    /// The most rows.
    pub const MAX_ROWS: u16 = 1000;

    /// A size with the cells of a cell size.
    pub fn new(columns: u16, rows: u16, cell_width_px: u16, cell_height_px: u16) -> Self {
        Self {
            columns,
            rows,
            cell_width_px,
            cell_height_px,
        }
    }

    /// The same size within the bounds a screen can have: at least two
    /// columns and a row, at most 2000 columns and 1000 rows, cells at
    /// least a pixel each way.
    pub fn bounded(self) -> Self {
        Self {
            columns: self.columns.clamp(Self::MIN_COLUMNS, Self::MAX_COLUMNS),
            rows: self.rows.clamp(Self::MIN_ROWS, Self::MAX_ROWS),
            cell_width_px: self.cell_width_px.max(1),
            cell_height_px: self.cell_height_px.max(1),
        }
    }
}

/// Where a program is: itself when it names a folder, else the first file
/// of that name among the folders of `path` (the `PATH` it will run with).
/// On Windows a name without one of the endings of `PATHEXT` (`.exe`,
/// `.cmd`, ...) is looked for with each of them, as the command prompt
/// does, and the file found is answered with its ending.
pub fn find_program(program: &str, path: Option<&str>) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    let as_path = Path::new(program);
    if program.contains('/') || program.contains('\\') || as_path.is_absolute() {
        return named(as_path);
    }
    let path = path?;
    std::env::split_paths(path)
        .filter(|dir| !dir.as_os_str().is_empty())
        .find_map(|dir| runnable(&dir.join(program)))
}

#[cfg(unix)]
fn named(path: &Path) -> Option<PathBuf> {
    path.is_file().then(|| path.to_path_buf())
}

#[cfg(unix)]
fn runnable(path: &Path) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt as _;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .then(|| path.to_path_buf())
}

#[cfg(not(unix))]
fn named(path: &Path) -> Option<PathBuf> {
    runnable(path)
}

#[cfg(not(unix))]
fn runnable(path: &Path) -> Option<PathBuf> {
    let endings = program_endings();
    let has_one = path
        .extension()
        .and_then(|ending| ending.to_str())
        .is_some_and(|ending| {
            endings
                .iter()
                .any(|known| known.trim_start_matches('.').eq_ignore_ascii_case(ending))
        });
    if has_one {
        return path.is_file().then(|| path.to_path_buf());
    }
    endings
        .iter()
        .map(|ending| {
            let mut with = path.as_os_str().to_owned();
            with.push(ending);
            PathBuf::from(with)
        })
        .find(|with| with.is_file())
}

/// The endings of programs on Windows: `PATHEXT`, or the usual four.
#[cfg(not(unix))]
fn program_endings() -> Vec<String> {
    let set = std::env::var("PATHEXT").unwrap_or_default();
    let endings: Vec<String> = set
        .split(';')
        .map(str::trim)
        .filter(|ending| ending.starts_with('.') && ending.len() > 1)
        .map(str::to_owned)
        .collect();
    if endings.is_empty() {
        [".COM", ".EXE", ".BAT", ".CMD"].map(str::to_owned).to_vec()
    } else {
        endings
    }
}

/// The arguments PowerShell is started with in the terminal when it is
/// given none: once the user's profile has run, its `prompt` is wrapped so
/// that, each time the prompt is shown, the process's working folder is
/// set to where the shell is. PowerShell's `cd` moves only its own idea of
/// where it is, and the working folder is what the system tells others
/// (the terminal's completion and its folder, on Windows). What the
/// prompt shows is the user's own.
pub fn powershell_following() -> Vec<String> {
    vec![
        "-NoExit".to_owned(),
        "-Command".to_owned(),
        POWERSHELL_FOLLOWING.to_owned(),
    ]
}

/// The script of [`powershell_following`].
const POWERSHELL_FOLLOWING: &str = concat!(
    "$global:__bitbraidPrompt = $function:prompt; ",
    "function global:prompt { ",
    "if ($PWD.Provider.Name -eq 'FileSystem') ",
    "{ [System.Environment]::CurrentDirectory = $PWD.ProviderPath }; ",
    "& $global:__bitbraidPrompt }"
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_kind_is_read_from_the_programs_name() {
        for (program, kind) in [
            ("/bin/sh", ShellKind::Posix),
            ("bash", ShellKind::Posix),
            ("/usr/bin/zsh", ShellKind::Posix),
            ("-bash", ShellKind::Posix),
            ("/usr/local/bin/fish", ShellKind::Fish),
            ("FISH", ShellKind::Fish),
            ("pwsh", ShellKind::PowerShell),
            (
                "C:\\Program Files\\PowerShell\\7\\pwsh.exe",
                ShellKind::PowerShell,
            ),
            ("powershell.EXE", ShellKind::PowerShell),
            ("C:\\Windows\\System32\\cmd.exe", ShellKind::Cmd),
            ("", ShellKind::Posix),
            ("nu", ShellKind::Nushell),
            ("/home/me/.cargo/bin/nu", ShellKind::Nushell),
            ("C:\\Program Files\\nu\\bin\\nu.exe", ShellKind::Nushell),
            ("nushell", ShellKind::Posix),
            ("/bin/tcsh", ShellKind::Csh),
            ("csh", ShellKind::Csh),
            ("-tcsh", ShellKind::Csh),
            ("/usr/bin/dash", ShellKind::Posix),
            ("/opt/fish/bin/", ShellKind::Posix),
        ] {
            assert_eq!(ShellKind::of_program(program), kind, "{program:?}");
        }
        for kind in ShellKind::ALL {
            assert_eq!(ShellKind::from_name(kind.name()), Some(kind));
        }
        assert_eq!(ShellKind::from_name("bash"), None);
        assert!(ShellKind::Nushell.takes_login() && ShellKind::Csh.takes_login());
        assert!(!ShellKind::PowerShell.takes_login());
    }

    #[test]
    fn sizes_are_kept_within_bounds() {
        let size = GridSize::new(0, 0, 0, 0).bounded();
        assert_eq!(size, GridSize::new(2, 1, 1, 1));
        let size = GridSize::new(u16::MAX, u16::MAX, u16::MAX, 9).bounded();
        assert_eq!(size, GridSize::new(2000, 1000, u16::MAX, 9));
    }

    #[test]
    fn the_last_value_of_a_variable_counts() {
        let spec = ShellSpec {
            env: vec![
                ("A".into(), "1".into()),
                ("B".into(), "2".into()),
                ("A".into(), "3".into()),
            ],
            ..ShellSpec::default()
        };
        assert_eq!(spec.var("A"), Some("3"));
        assert_eq!(spec.var("C"), None);
    }
}
