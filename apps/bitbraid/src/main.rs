//! BitBraid desktop binary.
//!
//! On Windows it is a program of the window system, not of the console:
//! started from Explorer or a link, no console window opens beside it.
//! Started as git's or ssh's askpass program it still answers on the pipe
//! it was given.
//!
//! `bitbraid --version` prints the version and opens nothing: what the
//! packages' tests start, and what a script asks. On Windows it reaches
//! only a pipe or a file (`bitbraid --version | more`), the program having
//! no console of its own.
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::io::Write as _;

fn main() {
    if version_asked(std::env::args_os().skip(1)) {
        // A closed output is no reason to fail: there is nobody to tell.
        let _ = writeln!(
            std::io::stdout().lock(),
            "bitbraid {}",
            env!("CARGO_PKG_VERSION")
        );
        return;
    }
    bitbraid_ui::run();
}

/// Whether the arguments, the program's name left out, are `--version`
/// and nothing else. Anything more is a path or a link to open, which may
/// be called that.
fn version_asked(mut args: impl Iterator<Item = std::ffi::OsString>) -> bool {
    args.next().is_some_and(|first| first == "--version") && args.next().is_none()
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::version_asked;

    fn asked(args: &[&str]) -> bool {
        version_asked(args.iter().map(OsString::from))
    }

    #[test]
    fn the_version_is_asked_by_the_one_flag_alone() {
        assert!(asked(&["--version"]));
        assert!(!asked(&[]));
        assert!(!asked(&["/work/project"]));
        assert!(!asked(&["--version", "/work/project"]));
        assert!(!asked(&["/work/--version"]));
        assert!(!asked(&["-V"]));
    }
}
