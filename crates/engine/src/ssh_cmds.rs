//! SSH keys for the providers: a new key made with ssh-keygen,
//! and the fingerprint of a key that is there, so that the window can
//! say which key it is about to give a provider.
//!
//! A key is made as `ssh-keygen -q -t ed25519 -N "" [-C comment] -f path`:
//! an Ed25519 key without a passphrase, as a provider takes it, in two
//! files, the private one at `path` and the public one beside it with
//! `.pub` added. Nothing is ever written over: when either file is there
//! the command refuses, and ssh-keygen is not asked. The path has to be a
//! full one without `..` in it; the folder it is in is made when it is
//! missing (readable by the user only, as ssh wants `~/.ssh`).
//!
//! ssh-keygen gets no terminal and no input, and is given a minute: a
//! program that waits for an answer is stopped rather than waited for.
//!
//! Sources: ssh-keygen(1) (`-t`, `-N`, `-C`, `-f`, `-q`, `-l`, the
//! fingerprint line); docs.github.com "Generating a new SSH key and adding
//! it to the ssh-agent" and docs.gitlab.com "Use SSH keys to communicate
//! with GitLab" (Ed25519 is what both suggest).

use std::{
    io::Read as _,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "ssh.key_generate",
            label: say!("ssh-key-generate"),
            description: say!("ssh-key-generate-description"),
        },
        generate,
    );
    let _ = registry.register(
        CommandSpec {
            id: "ssh.key_fingerprint",
            label: say!("ssh-key-fingerprint"),
            description: say!("ssh-key-fingerprint-description"),
        },
        fingerprint,
    );
}

/// The longest path taken.
const LONGEST: usize = 4096;
/// The longest comment taken.
const LONGEST_COMMENT: usize = 1024;
/// How long ssh-keygen is waited for.
const WAIT: Duration = Duration::from_secs(60);
/// The most of what ssh-keygen writes that is read.
const MOST_OUTPUT: u64 = 64 * 1024;
/// The largest public key file read.
const MOST_KEY_FILE: u64 = 64 * 1024;

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

fn refused(what: &'static str, detail: impl Into<String>) -> CommandError {
    CommandError::Git(bitbraid_git::Error::Refused {
        what,
        detail: detail.into(),
    })
}

/// A full path without `..`, control characters or an empty name.
fn checked_path(params: &Params) -> Result<PathBuf, CommandError> {
    let text: String = param(params, "path")?;
    if text.trim().is_empty() {
        return Err(invalid("path", say!("ssh-path-empty")));
    }
    if text.len() > LONGEST || text.chars().any(char::is_control) {
        return Err(invalid("path", say!("ssh-path-not-a-path")));
    }
    let path = PathBuf::from(&text);
    if !path.is_absolute() {
        return Err(invalid("path", say!("ssh-path-not-full")));
    }
    if path
        .components()
        .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(invalid("path", say!("ssh-path-goes-up")));
    }
    if path.file_name().is_none() {
        return Err(invalid("path", say!("ssh-path-no-file")));
    }
    Ok(path)
}

/// The path with `.pub` added to its name.
fn public_of(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".pub");
    PathBuf::from(name)
}

/// Whether anything is at a path, a link that leads nowhere included.
fn taken(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// What ssh-keygen said, and whether it ended well.
struct Said {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// Run ssh-keygen without a terminal or input, for a minute at most, and
/// stop it when the command is cancelled.
fn keygen(ctx: &CommandContext, args: &[&std::ffi::OsStr]) -> Result<Said, CommandError> {
    let failed = |err: std::io::Error| {
        refused(
            "ssh-keygen",
            say!("ssh-keygen-not-started", error = err.to_string()),
        )
    };
    let mut command = Command::new("ssh-keygen");
    bitbraid_core::process::no_window(&mut command);
    bitbraid_core::process::with_user_environment(&mut command);
    let mut child = command
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(failed)?;
    let started = Instant::now();
    let status = loop {
        if ctx.cancel.load(Ordering::SeqCst) || started.elapsed() > WAIT {
            let _ = child.kill();
            let _ = child.wait();
            return Err(refused(
                "ssh-keygen",
                if ctx.cancel.load(Ordering::SeqCst) {
                    say!("ssh-keygen-cancelled")
                } else {
                    say!("ssh-keygen-too-slow")
                },
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
            Err(err) => {
                let _ = child.kill();
                return Err(failed(err));
            }
        }
    };
    // What it writes is a line or two: it is in the pipes by now.
    let read = |pipe: Option<&mut dyn std::io::Read>| {
        let mut bytes = Vec::new();
        if let Some(pipe) = pipe {
            let _ = pipe.take(MOST_OUTPUT).read_to_end(&mut bytes);
        }
        String::from_utf8_lossy(&bytes).trim().to_owned()
    };
    let stdout = read(
        child
            .stdout
            .as_mut()
            .map(|pipe| pipe as &mut dyn std::io::Read),
    );
    let stderr = read(
        child
            .stderr
            .as_mut()
            .map(|pipe| pipe as &mut dyn std::io::Read),
    );
    Ok(Said {
        ok: status.success(),
        stdout,
        stderr,
    })
}

/// A line of `ssh-keygen -l`: `256 SHA256:... comment (ED25519)`, as its
/// size, fingerprint, comment and kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fingerprint {
    /// The key's size in bits.
    pub bits: Option<u32>,
    /// `SHA256:` and the hash.
    pub fingerprint: String,
    /// What the key says about itself; may be empty.
    pub comment: String,
    /// `ED25519`, `RSA`, `ECDSA`.
    pub kind: String,
}

/// Read a line of `ssh-keygen -l`.
pub fn read_fingerprint(line: &str) -> Option<Fingerprint> {
    let line = line.trim();
    let mut words = line.splitn(3, ' ');
    let bits = words.next()?.parse::<u32>().ok();
    let fingerprint = words.next().filter(|word| word.contains(':'))?.to_owned();
    let rest = words.next().unwrap_or_default().trim();
    let (comment, kind) = match rest.rsplit_once(' ') {
        Some((comment, kind)) if kind.starts_with('(') && kind.ends_with(')') => {
            (comment.trim(), kind)
        }
        _ if rest.starts_with('(') && rest.ends_with(')') => ("", rest),
        _ => (rest, ""),
    };
    Some(Fingerprint {
        bits,
        fingerprint,
        comment: comment.to_owned(),
        kind: kind.trim_matches(['(', ')']).to_owned(),
    })
}

/// The first line of a public key file, when it reads as one.
fn public_key_text(path: &Path) -> Option<String> {
    if !std::fs::metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MOST_KEY_FILE)
        .read_to_end(&mut bytes)
        .ok()?;
    let text = String::from_utf8(bytes).ok()?;
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    let mut words = line.split_whitespace();
    let kind = words.next()?;
    let blob = words.next()?;
    let plain = kind
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || "-@._".contains(c))
        && blob
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c));
    (plain && !line.chars().any(char::is_control)).then(|| line.to_owned())
}

/// The fingerprint ssh-keygen gives the key in a file.
fn fingerprint_of(ctx: &CommandContext, path: &Path) -> Result<Fingerprint, CommandError> {
    let said = keygen(ctx, &["-l".as_ref(), "-f".as_ref(), path.as_os_str()])?;
    if !said.ok {
        return Err(refused(
            say!("ssh-fingerprint-refused"),
            if said.stderr.is_empty() {
                say!("ssh-not-a-key", path = path.display().to_string())
            } else {
                said.stderr
            },
        ));
    }
    said.stdout
        .lines()
        .find_map(read_fingerprint)
        .ok_or_else(|| {
            refused(
                say!("ssh-fingerprint-refused"),
                say!("ssh-keygen-no-fingerprint"),
            )
        })
}

/// Make the folder a key goes into, readable by the user only.
fn make_folder(folder: &Path) -> Result<(), CommandError> {
    if folder.is_dir() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(folder).map_err(|err| {
        refused(
            say!("ssh-generate-refused"),
            say!(
                "ssh-folder-not-made",
                folder = folder.display().to_string(),
                error = err.to_string()
            ),
        )
    })
}

fn generate(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path = checked_path(params)?;
    let comment: Option<String> = param_or(params, "comment", None)?;
    let comment = comment
        .map(|comment| comment.trim().to_owned())
        .filter(|comment| !comment.is_empty());
    if let Some(comment) = &comment
        && (comment.len() > LONGEST_COMMENT || comment.chars().any(char::is_control))
    {
        return Err(invalid("comment", say!("ssh-comment-not-a-line")));
    }
    let public = public_of(&path);
    for file in [&path, &public] {
        if taken(file) {
            return Err(refused(
                say!("ssh-generate-refused"),
                say!("ssh-file-there", path = file.display().to_string()),
            ));
        }
    }
    if let Some(folder) = path.parent() {
        make_folder(folder)?;
    }
    let mut args: Vec<&std::ffi::OsStr> = vec![
        "-q".as_ref(),
        "-t".as_ref(),
        "ed25519".as_ref(),
        "-N".as_ref(),
        "".as_ref(),
    ];
    if let Some(comment) = &comment {
        args.push("-C".as_ref());
        args.push(comment.as_ref());
    }
    args.push("-f".as_ref());
    args.push(path.as_os_str());
    let said = keygen(ctx, &args)?;
    if !said.ok || !path.is_file() || !public.is_file() {
        return Err(refused(
            say!("ssh-generate-refused"),
            if said.stderr.is_empty() {
                say!("ssh-keygen-no-key").to_owned()
            } else {
                said.stderr
            },
        ));
    }
    let print = fingerprint_of(ctx, &public)?;
    Ok(json!({
        "private": path,
        "public": public,
        "public_key": public_key_text(&public),
        "fingerprint": print.fingerprint,
        "kind": print.kind,
        "comment": print.comment,
    }))
}

fn fingerprint(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path = checked_path(params)?;
    if !path.is_file() {
        return Err(refused(
            say!("ssh-fingerprint-refused"),
            say!("ssh-no-file", path = path.display().to_string()),
        ));
    }
    // The public key that goes with a private one is beside it; a public
    // key is its own.
    let is_public = path.extension().is_some_and(|extension| extension == "pub");
    let public = if is_public {
        Some(path.clone())
    } else {
        Some(public_of(&path)).filter(|public| public.is_file())
    };
    let print = fingerprint_of(ctx, public.as_deref().unwrap_or(&path))?;
    Ok(json!({
        "path": path,
        "public": public,
        "public_key": public.as_deref().and_then(public_key_text),
        "fingerprint": print.fingerprint,
        "bits": print.bits,
        "kind": print.kind,
        "comment": print.comment,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_line_is_read_in_its_parts() {
        let print = read_fingerprint("256 SHA256:abcDEF+/0 someone@machine (ED25519)\n").unwrap();
        assert_eq!(print.bits, Some(256));
        assert_eq!(print.fingerprint, "SHA256:abcDEF+/0");
        assert_eq!(print.comment, "someone@machine");
        assert_eq!(print.kind, "ED25519");
        let bare = read_fingerprint("3072 SHA256:xyz (RSA)").unwrap();
        assert_eq!(bare.comment, "");
        assert_eq!(bare.kind, "RSA");
        let spaced = read_fingerprint("256 SHA256:xyz a comment with words (ECDSA)").unwrap();
        assert_eq!(spaced.comment, "a comment with words");
        assert!(read_fingerprint("").is_none());
        assert!(read_fingerprint("not a fingerprint").is_none());
        assert!(read_fingerprint("256 nocolon c (ED25519)").is_none());
    }

    #[test]
    fn paths_that_will_not_do_are_refused() {
        // A full path begins with a drive on Windows.
        let root = if cfg!(windows) { "C:" } else { "" };
        for (path, why) in [
            (json!("relative/key"), "full path"),
            (json!(format!("{root}/tmp/../etc/key")), ".."),
            (json!(""), "empty"),
            (json!(format!("{root}/tmp/key\nmore")), "not a path"),
            (json!(42), "invalid type"),
            (json!(format!("{root}/")), "names no file"),
        ] {
            let err = checked_path(&json!({ "path": path }))
                .unwrap_err()
                .to_string();
            assert!(err.contains(why), "{path}: {err}");
        }
        assert!(checked_path(&json!({})).is_err());
        assert_eq!(
            public_of(Path::new("/a/id_ed25519")),
            PathBuf::from("/a/id_ed25519.pub")
        );
    }
}
