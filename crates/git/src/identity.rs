//! Identities: who git says the work is by, which key it signs with and
//! which key it connects with. The identity in effect is read as git
//! resolves it; one is written to a scope, which is the repository's
//! configuration, the user's, or a configuration file that is named.
//!
//! Sources: git-config(1) (`user.name`, `user.email`, `user.signingKey`,
//! `gpg.format`, `core.sshCommand`, the scopes, `--includes`, the exit
//! codes), git(1) (`GIT_SSH_COMMAND`: the command goes through the shell),
//! git-var(1) (`GIT_AUTHOR_IDENT`: what git makes of a name, observed),
//! ssh(1) and ssh_config(5) (`-i`, `IdentitiesOnly`, and what the name of
//! an identity file may stand for, observed with `ssh -v`), the POSIX
//! shell's rule for single quotes.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{Error, Repository, Result, exe};

/// The longest name, address, and key or path that are taken.
const MAX_NAME: usize = 256;
const MAX_EMAIL: usize = 320;
const LONGEST: usize = 4096;

/// The five settings an identity is made of: as git writes their names
/// when it lists them, which is in lower case, and as they are written
/// into a file.
const KEYS: &str = r"^(user\.(name|email|signingkey)|gpg\.format|core\.sshcommand)$";
const NAME: &str = "user.name";
const EMAIL: &str = "user.email";
const SIGNING_KEY: &str = "user.signingKey";
const SIGNING_FORMAT: &str = "gpg.format";
const SSH_COMMAND: &str = "core.sshCommand";

/// The command an SSH key becomes: that key and no other.
const SSH_BEFORE: &str = "ssh -i ";
const SSH_AFTER: &str = " -o IdentitiesOnly=yes";

/// How a key signs, as `gpg.format` names it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SigningFormat {
    /// A GPG key, which is what git takes a key for when nothing is said.
    Openpgp,
    /// An SSH key.
    Ssh,
    /// An X.509 certificate.
    X509,
}

impl SigningFormat {
    /// The word git has for it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Openpgp => "openpgp",
            Self::Ssh => "ssh",
            Self::X509 => "x509",
        }
    }
}

/// Where an identity is read from or written to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// The repository's own configuration.
    Repo,
    /// The user's configuration.
    User,
    /// A configuration file, named by its full path.
    File(PathBuf),
}

/// An identity as it is configured. A setting that is not there is absent;
/// one that is there and empty is an empty text.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Identity {
    /// `user.name`.
    pub user_name: Option<String>,
    /// `user.email`.
    pub user_email: Option<String>,
    /// `user.signingKey`.
    pub signing_key: Option<String>,
    /// `gpg.format` as it is written, whether or not git knows the word.
    pub signing_format: Option<String>,
    /// `core.sshCommand` as it is written.
    pub ssh_command: Option<String>,
    /// The key file in that command, when the command is one written here.
    pub ssh_key: Option<String>,
}

/// An identity to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewIdentity {
    /// `user.name`.
    pub user_name: String,
    /// `user.email`.
    pub user_email: String,
    /// The key to sign with and how it signs. Without one the scope is
    /// left with neither.
    pub signing: Option<(String, SigningFormat)>,
    /// The private key file to connect with, by its full path.
    pub ssh_key: Option<String>,
}

/// What writing an identity changed, in the scope it was written to.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Written {
    /// What the scope had.
    pub before: Identity,
    /// What it has now.
    pub after: Identity,
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("identity-what-identity"),
        detail: detail.into(),
    }
}

/// What git takes off both ends of a name before it writes it into a
/// commit: blanks, control characters and a little punctuation. Older
/// versions take a period off too, so here it counts.
fn falls_off(ch: char) -> bool {
    ch <= ' ' || matches!(ch, '.' | ',' | ':' | ';' | '<' | '>' | '"' | '\\' | '\'')
}

fn check_name(name: &str) -> Result<()> {
    let what = say!("identity-what-user-name");
    if name.len() > MAX_NAME {
        return Err(invalid(what, say!("identity-too-long")));
    }
    if name.chars().any(char::is_control) {
        return Err(invalid(what, say!("identity-control")));
    }
    // Git leaves these out of what it writes, so the name in a commit
    // would not be the one that was asked for.
    if name.contains(['<', '>']) {
        return Err(invalid(what, say!("identity-angle-brackets")));
    }
    if name.trim_matches(falls_off).is_empty() {
        return Err(invalid(what, say!("identity-empty")));
    }
    Ok(())
}

fn check_email(email: &str) -> Result<()> {
    let what = say!("identity-what-email");
    if email.is_empty() {
        return Err(invalid(what, say!("identity-empty")));
    }
    if email.len() > MAX_EMAIL {
        return Err(invalid(what, say!("identity-too-long")));
    }
    if email.chars().any(char::is_control) {
        return Err(invalid(what, say!("identity-control")));
    }
    if email.contains(['<', '>']) {
        return Err(invalid(what, say!("identity-angle-brackets")));
    }
    if email.chars().any(char::is_whitespace) {
        return Err(invalid(what, say!("identity-spaces")));
    }
    Ok(())
}

fn check_signing_key(key: &str) -> Result<()> {
    let what = say!("identity-what-signing-key");
    if key.trim().is_empty() {
        return Err(invalid(what, say!("identity-empty")));
    }
    if key.len() > LONGEST {
        return Err(invalid(what, say!("identity-too-long")));
    }
    if key.chars().any(char::is_control) {
        return Err(invalid(what, say!("identity-control")));
    }
    // It is handed to the signing program as the value of an option.
    if key.starts_with('-') {
        return Err(invalid(what, say!("identity-starts-with-dash")));
    }
    Ok(())
}

/// A key file's path as it can be put into the SSH command: whether the
/// file is there is not asked.
fn check_ssh_key(key: &str) -> Result<()> {
    let what = say!("identity-what-ssh-key");
    if key.is_empty() {
        return Err(invalid(what, say!("identity-empty")));
    }
    if key.len() > LONGEST {
        return Err(invalid(what, say!("identity-too-long")));
    }
    if key.chars().any(char::is_control) {
        return Err(invalid(what, say!("identity-control")));
    }
    // Ssh is started wherever git happens to be working.
    if !Path::new(key).is_absolute() {
        return Err(invalid(what, say!("identity-key-not-absolute")));
    }
    // Ssh fills these in itself, after the shell: `%d` in the name of a
    // key file is the home directory to it, `${HOME}` a variable. No
    // quoting reaches that, so such a file cannot be named.
    if key.contains('%') || key.contains("${") {
        return Err(invalid(what, say!("identity-key-fill-in")));
    }
    Ok(())
}

/// Check an identity before it is kept or written: what git would refuse,
/// drop or change, and what cannot be said in the SSH command.
pub fn check(identity: &NewIdentity) -> Result<()> {
    check_name(&identity.user_name)?;
    check_email(&identity.user_email)?;
    if let Some((key, _)) = &identity.signing {
        check_signing_key(key)?;
    }
    if let Some(key) = &identity.ssh_key {
        check_ssh_key(key)?;
    }
    Ok(())
}

/// One word for the shell, whatever is in it: inside single quotes
/// nothing has a meaning, and a single quote is written by leaving them
/// for the length of an escaped one.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// The value of `core.sshCommand` that connects with this key file and
/// with no other key. Git hands the value to the shell, so the path is
/// quoted for it.
pub fn ssh_command_for(key: &str) -> Result<String> {
    check_ssh_key(key)?;
    Ok(format!("{SSH_BEFORE}{}{SSH_AFTER}", quoted(key)))
}

/// The key file of an SSH command of exactly the form
/// [`ssh_command_for`] writes. Anything else, however close, is somebody
/// else's command.
pub fn ssh_key_of(command: &str) -> Option<String> {
    let word = command
        .strip_prefix(SSH_BEFORE)?
        .strip_suffix(SSH_AFTER)?
        .strip_prefix('\'')?
        .strip_suffix('\'')?;
    let key = word.split(r"'\''").collect::<Vec<_>>().join("'");
    // Written again it has to come out the same, to the letter.
    (ssh_command_for(&key).ok().as_deref() == Some(command)).then_some(key)
}

/// The options that name a scope to `git config`.
pub(crate) fn scope_args(scope: &Scope) -> Vec<OsString> {
    match scope {
        Scope::Repo => vec!["--local".into()],
        Scope::User => vec!["--global".into()],
        Scope::File(path) => vec!["--file".into(), path.clone().into_os_string()],
    }
}

/// Whether a scope can be asked or written at all.
pub(crate) fn check_scope(repo: Option<&Repository>, scope: &Scope) -> Result<()> {
    match scope {
        Scope::Repo if repo.is_none() => Err(refused(say!("identity-no-repository"))),
        Scope::Repo | Scope::User => Ok(()),
        Scope::File(path) => {
            let what = say!("identity-what-config-file");
            let text = path
                .to_str()
                .ok_or_else(|| invalid(what, say!("identity-path-not-text")))?;
            if text.is_empty() {
                return Err(invalid(what, say!("identity-empty")));
            }
            if text.len() > LONGEST {
                return Err(invalid(what, say!("identity-path-too-long")));
            }
            if text.chars().any(char::is_control) {
                return Err(invalid(what, say!("identity-path-control")));
            }
            if !path.is_absolute() {
                return Err(invalid(what, say!("identity-full-path")));
            }
            if path.exists() && !path.is_file() {
                return Err(refused(say!(
                    "identity-not-a-file",
                    path = path.display().to_string()
                )));
            }
            Ok(())
        }
    }
}

/// Run `git config`, in the repository when one is open. What is asked
/// of a scope that is not the repository's is the same anywhere, so
/// without a repository any directory that is there will do.
fn config(repo: Option<&Repository>, args: Vec<OsString>) -> Result<(Option<i32>, exe::Output)> {
    let mut all: Vec<OsString> = vec!["config".into()];
    all.extend(args);
    match repo {
        Some(repo) => exe::run_with_code(repo, all, None),
        None => match exe::run_in(&std::env::temp_dir(), all) {
            Ok(out) => Ok((Some(0), out)),
            // A question that finds nothing ends with 1 and says nothing;
            // whatever else goes wrong, git says what.
            Err(Error::Command { stderr, .. }) if stderr.is_empty() => Ok((
                Some(1),
                exe::Output {
                    stdout: String::new(),
                    stderr,
                },
            )),
            Err(err) => Err(err),
        },
    }
}

/// Cut `git config -z --get-regexp` into an identity. An entry is a key,
/// a line break and the value; a key without a value has no line break.
/// Entries come in the order git reads them, so the last one of a key is
/// the one in effect.
fn parse(text: &str) -> Identity {
    let mut identity = Identity::default();
    for entry in text.split('\0').filter(|entry| !entry.is_empty()) {
        let (key, value) = entry.split_once('\n').unwrap_or((entry, ""));
        let value = Some(value.to_owned());
        match key {
            "user.name" => identity.user_name = value,
            "user.email" => identity.user_email = value,
            "user.signingkey" => identity.signing_key = value,
            "gpg.format" => identity.signing_format = value,
            "core.sshcommand" => identity.ssh_command = value,
            _ => {}
        }
    }
    identity.ssh_key = identity.ssh_command.as_deref().and_then(ssh_key_of);
    identity
}

/// Ask for the five settings: of one scope, or of all of them as git
/// resolves them. With `includes`, what the scope's file includes counts
/// too, as it does when git works.
fn ask(repo: Option<&Repository>, scope: Option<&Scope>, includes: bool) -> Result<Identity> {
    let mut args = scope.map(scope_args).unwrap_or_default();
    if scope.is_some() && includes {
        args.push("--includes".into());
    }
    args.extend(["-z".into(), "--get-regexp".into(), KEYS.into()]);
    let (code, out) = config(repo, args)?;
    match code {
        Some(0) => Ok(parse(&out.stdout)),
        // None of them is set.
        Some(1) => Ok(Identity::default()),
        _ => Err(refused(say!(
            "identity-cannot-read",
            said = out.stderr.trim()
        ))),
    }
}

/// The identity in effect in a repository: every scope, as git resolves
/// them.
pub fn effective(repo: &Repository) -> Result<Identity> {
    ask(Some(repo), None, true)
}

/// The identity one scope has, with what its file includes. A file that
/// is not there has none.
pub fn read(repo: Option<&Repository>, scope: &Scope) -> Result<Identity> {
    check_scope(repo, scope)?;
    ask(repo, Some(scope), true)
}

fn set(repo: Option<&Repository>, scope: &Scope, key: &str, value: &str) -> Result<()> {
    let mut args = scope_args(scope);
    args.extend([
        "--replace-all".into(),
        "--".into(),
        key.into(),
        value.into(),
    ]);
    written(config(repo, args)?)
}

fn unset(repo: Option<&Repository>, scope: &Scope, key: &str) -> Result<()> {
    let mut args = scope_args(scope);
    args.extend(["--unset-all".into(), "--".into(), key.into()]);
    written(config(repo, args)?)
}

fn written((code, out): (Option<i32>, exe::Output)) -> Result<()> {
    if code == Some(0) {
        return Ok(());
    }
    Err(refused(say!(
        "identity-cannot-write",
        said = out.stderr.trim()
    )))
}

/// Write an identity to a scope: the name and the address; the signing
/// key and its format, or neither when the identity has no key; and the
/// SSH command for its key file. Without a key file an SSH command is
/// taken away only when it is one that was written here: any other is
/// the user's own and stays.
///
/// Everything is checked before the first setting is written, and a
/// write that fails half-way is taken back.
pub fn write(repo: Option<&Repository>, scope: &Scope, identity: &NewIdentity) -> Result<Written> {
    check(identity)?;
    check_scope(repo, scope)?;
    if let Scope::File(path) = scope
        && !path.parent().is_some_and(Path::is_dir)
    {
        return Err(refused(say!(
            "identity-no-directory",
            path = path.display().to_string()
        )));
    }
    let ssh_command = match &identity.ssh_key {
        Some(key) => {
            // Ssh passes over a key file it cannot find and goes on with
            // the keys it finds by itself: the wrong ones, without a word.
            if !Path::new(key).is_file() {
                return Err(refused(say!("identity-no-key-file", key = key.as_str())));
            }
            Some(ssh_command_for(key)?)
        }
        None => None,
    };
    // What the scope itself says, without what it includes: only that
    // can be changed here.
    let before = ask(repo, Some(scope), false)?;

    // Each setting with what it is to be (nothing: gone) and what it was.
    let (key, format) = match &identity.signing {
        Some((key, format)) => (Some(key.as_str()), Some(format.as_str())),
        None => (None, None),
    };
    let mut plan = vec![
        (NAME, Some(identity.user_name.as_str()), &before.user_name),
        (
            EMAIL,
            Some(identity.user_email.as_str()),
            &before.user_email,
        ),
        (SIGNING_KEY, key, &before.signing_key),
        (SIGNING_FORMAT, format, &before.signing_format),
    ];
    if ssh_command.is_some() || before.ssh_key.is_some() {
        plan.push((SSH_COMMAND, ssh_command.as_deref(), &before.ssh_command));
    }

    let mut done: Vec<(&str, &Option<String>)> = Vec::new();
    for (key, wanted, was) in plan {
        let result = match wanted {
            Some(value) => set(repo, scope, key, value),
            None if was.is_some() => unset(repo, scope, key),
            None => Ok(()),
        };
        if let Err(err) = result {
            for (key, was) in done.into_iter().rev() {
                let _ = match was {
                    Some(value) => set(repo, scope, key, value),
                    None => unset(repo, scope, key),
                };
            }
            return Err(err);
        }
        done.push((key, was));
    }
    Ok(Written {
        before,
        after: ask(repo, Some(scope), false)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(name: &str, email: &str) -> NewIdentity {
        NewIdentity {
            user_name: name.to_owned(),
            user_email: email.to_owned(),
            signing: None,
            ssh_key: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_listing_is_cut_into_an_identity_and_the_last_value_counts() {
        let command = ssh_command_for("/keys/id work").unwrap();
        let text = format!(
            "user.name\nFirst\0user.email\nada@example.invalid\0user.name\nAda L\0\
             gpg.format\nssh\0user.signingkey\n/keys/id.pub\0core.sshcommand\n{command}\0\
             core.editor\nvi\0"
        );
        let read = parse(&text);
        assert_eq!(read.user_name.as_deref(), Some("Ada L"));
        assert_eq!(read.user_email.as_deref(), Some("ada@example.invalid"));
        assert_eq!(read.signing_key.as_deref(), Some("/keys/id.pub"));
        assert_eq!(read.signing_format.as_deref(), Some("ssh"));
        assert_eq!(read.ssh_command.as_deref(), Some(command.as_str()));
        assert_eq!(read.ssh_key.as_deref(), Some("/keys/id work"));

        assert_eq!(parse(""), Identity::default());
        assert_eq!(parse("\0\0\0"), Identity::default());
        // A key without a value is there, and empty; a value may have
        // line breaks of its own.
        let odd = parse("user.name\0user.email\na\nb\0core.sshcommand\nssh -v\0");
        assert_eq!(odd.user_name.as_deref(), Some(""));
        assert_eq!(odd.user_email.as_deref(), Some("a\nb"));
        assert_eq!(odd.ssh_command.as_deref(), Some("ssh -v"));
        assert_eq!(odd.ssh_key, None, "not a command written here");
    }

    #[cfg(unix)]
    #[test]
    fn the_ssh_command_quotes_the_key_and_is_known_again() {
        assert_eq!(
            ssh_command_for("/home/ada/.ssh/id_work").unwrap(),
            "ssh -i '/home/ada/.ssh/id_work' -o IdentitiesOnly=yes"
        );
        assert_eq!(
            ssh_command_for("/k/it's").unwrap(),
            r"ssh -i '/k/it'\''s' -o IdentitiesOnly=yes"
        );
        for key in [
            "/plain",
            "/with space/key",
            "/it's/a 'quoted' one",
            "/double \"quotes\"",
            "/dollar $HOME $(id) `id`",
            "/back\\slash\\",
            "/semi;colon&and|pipe>out",
            "/''",
            "/'",
            "/trailing '",
            "/ünï/ключ/鍵",
            "/-i/-oProxyCommand=x",
        ] {
            let command = ssh_command_for(key).unwrap();
            assert_eq!(ssh_key_of(&command).as_deref(), Some(key), "{command}");
        }
    }

    #[test]
    fn a_key_path_that_cannot_be_said_is_refused() {
        let long = format!("/{}", "k".repeat(5000));
        for key in [
            "",
            "relative/key",
            "~/.ssh/id_ed25519",
            "-oProxyCommand=x",
            "/line\nbreak",
            "/nul\0",
            "/bell\u{7}",
            "/percent/%d/key",
            "/percent%",
            "/variable/${HOME}/key",
            long.as_str(),
        ] {
            assert!(ssh_command_for(key).is_err(), "{key:?} was taken");
        }
    }

    #[test]
    fn only_the_exact_form_is_taken_for_one_written_here() {
        for command in [
            "",
            "ssh",
            "ssh -i ",
            "ssh -i  -o IdentitiesOnly=yes",
            "ssh -i '' -o IdentitiesOnly=yes",
            "ssh -i ' -o IdentitiesOnly=yes",
            "ssh -i /plain -o IdentitiesOnly=yes",
            "ssh -i \"/k\" -o IdentitiesOnly=yes",
            "ssh -i '/k' -o IdentitiesOnly=yes -v",
            "ssh -v -i '/k' -o IdentitiesOnly=yes",
            "ssh  -i '/k' -o IdentitiesOnly=yes",
            "ssh -i '/k' -o IdentitiesOnly=no",
            "ssh -i '/k' '/l' -o IdentitiesOnly=yes",
            "ssh -i '/k'; rm -rf '/' -o IdentitiesOnly=yes",
            "ssh -i '/k'\\''' -o IdentitiesOnly=yes x",
            "ssh -i 'relative' -o IdentitiesOnly=yes",
            "ssh -i '/k%d' -o IdentitiesOnly=yes",
            "/usr/bin/ssh -i '/k' -o IdentitiesOnly=yes",
            "ssh -i '/k' -o IdentitiesOnly=yes\n",
        ] {
            assert_eq!(ssh_key_of(command), None, "{command:?}");
        }
    }

    #[test]
    fn names_and_addresses_git_would_change_are_refused() {
        assert!(check(&identity("Ada Lovelace", "ada@example.invalid")).is_ok());
        assert!(check(&identity("Ådä Ł. O'Neil-Smith, Jr.", "a+b@x")).is_ok());
        assert!(check(&identity("人", "local")).is_ok());
        let long = "n".repeat(300);
        for name in [
            "",
            " ",
            "...",
            "<>",
            "Ada <ada@x>",
            "Ada > Bob",
            "two\nlines",
            "nul\0",
            "tab\there",
            "\u{1b}[31mred",
            long.as_str(),
        ] {
            assert!(
                check(&identity(name, "ada@example.invalid")).is_err(),
                "the name {name:?} was taken"
            );
        }
        let long = format!("{}@example.invalid", "a".repeat(320));
        for email in [
            "",
            "ada@x\n",
            "ada@x\nCommitter: eve <eve@x>",
            "<ada@x>",
            "ada@x>",
            "ada @x",
            "ada\u{a0}@x",
            "nul\0@x",
            long.as_str(),
        ] {
            assert!(
                check(&identity("Ada", email)).is_err(),
                "the address {email:?} was taken"
            );
        }
        let long = "k".repeat(5000);
        for key in ["", "  ", "-u", "--default-key=x", "a\nb", long.as_str()] {
            let mut with_key = identity("Ada", "ada@x");
            with_key.signing = Some((key.to_owned(), SigningFormat::Openpgp));
            assert!(check(&with_key).is_err(), "the key {key:?} was taken");
        }
    }

    #[test]
    fn a_scope_names_itself_to_git() {
        assert_eq!(scope_args(&Scope::Repo), ["--local"]);
        assert_eq!(scope_args(&Scope::User), ["--global"]);
        assert_eq!(scope_args(&Scope::File("/x/y".into())), ["--file", "/x/y"]);
        // The repository's needs a repository; a file needs its full path.
        assert!(check_scope(None, &Scope::Repo).is_err());
        assert!(check_scope(None, &Scope::User).is_ok());
        for path in ["", "relative", "./x", "/line\nbreak", "/nul\0"] {
            assert!(
                check_scope(None, &Scope::File(path.into())).is_err(),
                "{path:?}"
            );
        }
        // A directory is not a file to write settings to.
        assert!(check_scope(None, &Scope::File(std::env::temp_dir())).is_err());
    }
}
