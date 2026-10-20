//! Link commands: `bitbraid://open?…` links that name a repository
//! and, in it, a branch, a tag, a commit or a pull request; and what makes
//! the system hand such links to the application: a desktop entry on
//! Linux, the key of the scheme in the registry on Windows, and on macOS
//! the application bundle the program is in, made known to the system.
//!
//! A link is the one thing that reaches the application from outside the
//! machine, so reading one is strict: whatever is not exactly a link of
//! ours is refused, with what is wrong. [`parse_link`] reads no file and
//! starts nothing; the application calls it before any repository is open.
//!
//! Sources: RFC 3986 (scheme and host without regard to case, the query
//! component, percent-encoding, unreserved characters); git-clone(1)
//! "GIT URLS" and gitremote-helpers(7) (the forms the address of a remote
//! has, and `<transport>::<address>`); the freedesktop Desktop Entry
//! Specification (the `Exec` key and its quoting, `MimeType`, `NoDisplay`),
//! the XDG Base Directory Specification (`XDG_DATA_HOME`), the shared
//! MIME-info specification (`x-scheme-handler/…`), xdg-mime(1); Microsoft's
//! "Registering an Application to a URI Scheme" (the key of a scheme, its
//! default value, `URL Protocol`, `DefaultIcon`, `shell\open\command` and
//! `"%1"`), "Merged View of HKEY_CLASSES_ROOT"
//! (`HKEY_CURRENT_USER\Software\Classes`, for one user and without an
//! administrator) and "Parsing C command-line arguments" (how a program on
//! Windows reads its command line: its own name up to the next double
//! quote, then double quotes and backslashes); the documentation of
//! `windows-registry` 0.6 (`CURRENT_USER`, `Key::create`, `set_string`,
//! `remove_tree`); Apple's "Launch Services Programming Guide" (an
//! application's URL schemes come from `CFBundleURLTypes` in its
//! Info.plist, and its bundle is registered by `lsregister -f`); AppImage's
//! documentation of its runtime (`APPIMAGE`, the file's path, and `APPDIR`,
//! where it is mounted, both set for the program it runs).

use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Serialize, Serializer, ser::SerializeStruct as _};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
};

/// The scheme of a link.
pub const SCHEME: &str = "bitbraid";
/// The longest link that is read, in bytes.
pub const MAX_LINK: usize = 4096;
/// The desktop entry that takes the links, by its file name.
pub const DESKTOP_FILE: &str = "bitbraid-url.desktop";
/// What that entry says it opens: addresses of our scheme.
pub const MIME_TYPE: &str = "x-scheme-handler/bitbraid";
/// The longest path that is taken for a program or a directory.
const LONGEST: usize = 4096;
/// Where Windows looks for the key of a scheme for this user alone,
/// under `HKEY_CURRENT_USER`: no administrator is needed to write there.
const WINDOWS_CLASSES: &str = r"Software\Classes";
/// What the key of the scheme says it is.
const WINDOWS_NAME: &str = "URL:BitBraid link";
/// Launch Services' own tool, which registers an application bundle.
const LSREGISTER: &str = "/System/Library/Frameworks/CoreServices.framework/Frameworks/\
                          LaunchServices.framework/Support/lsregister";

/// The repository a link names.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LinkRepository {
    /// One on this machine, by its full path.
    Path(String),
    /// One by the address of its remote: to be found among the
    /// repositories that are known, or to be cloned.
    Url(String),
}

/// What a link names in the repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkTarget {
    /// Nothing: the repository itself.
    None,
    /// A branch.
    Branch(String),
    /// A tag.
    Tag(String),
    /// A commit, by 7 to 64 hex digits in lower case.
    Commit(String),
    /// A pull request, by its number.
    PullRequest(String),
}

impl LinkTarget {
    /// The word for what it is, which is also its parameter in a link.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Branch(_) => "branch",
            Self::Tag(_) => "tag",
            Self::Commit(_) => "commit",
            Self::PullRequest(_) => "pr",
        }
    }

    /// The name, id or number.
    pub fn value(&self) -> Option<&str> {
        match self {
            Self::None => None,
            Self::Branch(value)
            | Self::Tag(value)
            | Self::Commit(value)
            | Self::PullRequest(value) => Some(value),
        }
    }
}

/// As `{kind, value}`, the value absent for the repository itself.
impl Serialize for LinkTarget {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut out = serializer.serialize_struct("LinkTarget", 2)?;
        out.serialize_field("kind", self.kind())?;
        out.serialize_field("value", &self.value())?;
        out.end()
    }
}

/// A link, read.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Link {
    /// Which repository.
    pub repository: LinkRepository,
    /// What in it.
    pub target: LinkTarget,
}

/// Why a text is not a link of ours.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LinkError {
    /// Longer than [`MAX_LINK`].
    #[error("{}", say!("link-too-long", max = MAX_LINK))]
    TooLong,
    /// A blank, a control character or a character that changes the
    /// direction of writing, as it stands in the link.
    #[error("{}", say!("link-raw-character"))]
    RawCharacter,
    /// Another scheme, or none.
    #[error("{}", say!("link-scheme", scheme = SCHEME))]
    Scheme,
    /// Another action than `open`.
    #[error("{}", say!("link-action"))]
    Action,
    /// A `#` and what follows it.
    #[error("{}", say!("link-fragment"))]
    Fragment,
    /// An `&` with nothing before or after it.
    #[error("{}", say!("link-empty-parameter"))]
    EmptyParameter,
    /// A parameter that is none of ours.
    #[error("{}", say!("link-unknown-parameter", parameter = format!("{:?}", .0)))]
    UnknownParameter(String),
    /// One parameter, two values.
    #[error("{}", say!("link-twice", parameter = *.0))]
    Twice(&'static str),
    /// A parameter without a value.
    #[error("{}", say!("link-no-value", parameter = *.0))]
    NoValue(&'static str),
    /// A `%` that is not followed by two hex digits.
    #[error("{}", say!("link-escape", parameter = *.0))]
    Escape(&'static str),
    /// Bytes that are no text.
    #[error("{}", say!("link-not-utf8", parameter = *.0))]
    NotUtf8(&'static str),
    /// A control character, or one that changes the direction of writing.
    #[error("{}", say!("link-control", parameter = *.0))]
    Control(&'static str),
    /// Neither `path` nor `url`.
    #[error("{}", say!("link-no-repository"))]
    NoRepository,
    /// Both `path` and `url`.
    #[error("{}", say!("link-two-repositories"))]
    TwoRepositories,
    /// More than one of `branch`, `tag`, `commit` and `pr`.
    #[error("{}", say!("link-targets"))]
    Targets,
    /// A path that does not begin at the root or at a drive.
    #[error("{}", say!("link-path-not-absolute"))]
    PathNotAbsolute,
    /// A path on another machine (`\\server\share`).
    #[error("{}", say!("link-path-elsewhere"))]
    PathElsewhere,
    /// An address that is not one of the forms a link may carry.
    #[error("{}", say!("link-url"))]
    Url,
    /// An address with a password in it.
    #[error("{}", say!("link-url-password"))]
    UrlPassword,
    /// A name that git would take for an option.
    #[error("{}", say!("link-dash", parameter = *.0))]
    Dash(&'static str),
    /// A commit that is not 7 to 64 hex digits.
    #[error("{}", say!("link-commit"))]
    Commit,
    /// A pull request that is not a number.
    #[error("{}", say!("link-pull-request"))]
    PullRequest,
}

/// The beginning of a text, for an error message.
fn cut(text: &str) -> String {
    const SHOWN: usize = 60;
    match text.char_indices().nth(SHOWN) {
        Some((end, _)) => format!("{}…", text.get(..end).unwrap_or("")),
        None => text.to_owned(),
    }
}

/// A character that shows as nothing, or as something else than it is:
/// control characters, line and paragraph separators, and the characters
/// that turn the direction of writing around.
fn hidden(ch: char) -> bool {
    ch.is_control()
        || matches!(ch, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// Read the percent escapes of one value. Everything else stands for
/// itself, a `+` included: this is not a web form.
fn decode(raw: &str, name: &'static str) -> Result<String, LinkError> {
    let digit = |byte: Option<u8>| {
        byte.and_then(|byte| char::from(byte).to_digit(16))
            .and_then(|digit| u8::try_from(digit).ok())
    };
    let mut bytes = Vec::with_capacity(raw.len());
    let mut rest = raw.bytes();
    while let Some(byte) = rest.next() {
        if byte != b'%' {
            bytes.push(byte);
            continue;
        }
        let (Some(high), Some(low)) = (digit(rest.next()), digit(rest.next())) else {
            return Err(LinkError::Escape(name));
        };
        bytes.push(high << 4 | low);
    }
    String::from_utf8(bytes).map_err(|_| LinkError::NotUtf8(name))
}

/// A full path, in the way of either kind of system: a link is read the
/// same wherever it is read. Whether anything is there is not asked.
fn checked_path(path: String) -> Result<String, LinkError> {
    let mut chars = path.chars();
    let (first, second, third) = (chars.next(), chars.next(), chars.next());
    let slash = |ch: Option<char>| matches!(ch, Some('/' | '\\'));
    // Two of them begin the name of another machine, and opening a
    // repository there would have this one log in to it.
    if slash(first) && slash(second) {
        return Err(LinkError::PathElsewhere);
    }
    let drive =
        first.is_some_and(|ch| ch.is_ascii_alphabetic()) && second == Some(':') && slash(third);
    if first == Some('/') || drive {
        Ok(path)
    } else {
        Err(LinkError::PathNotAbsolute)
    }
}

/// The address of a remote, of a kind that reaches a server and does
/// nothing else. Git takes `<transport>::<address>` for a program to
/// run, a leading dash for an option and a path for a repository here;
/// none of those is for a link to say.
fn checked_url(url: String) -> Result<String, LinkError> {
    let host_like = |text: &str| {
        !text.starts_with('-')
            && !text.is_empty()
            && text
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    };
    let fine = match url.split_once("://") {
        Some((scheme, rest)) => {
            let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
            let (user, host) = match authority.rsplit_once('@') {
                Some((user, host)) => (Some(user), host),
                None => (None, authority),
            };
            if user.is_some_and(|user| user.contains(':')) {
                return Err(LinkError::UrlPassword);
            }
            matches!(scheme, "https" | "http" | "ssh" | "git")
                && !authority.starts_with('-')
                && !host.is_empty()
                && !host.starts_with('-')
        }
        // `host:path` and `user@host:path`. One letter before the colon
        // is a drive, which is a path.
        None => url.split_once(':').is_some_and(|(before, path)| {
            let (user, host) = match before.rsplit_once('@') {
                Some((user, host)) => (Some(user), host),
                None => (None, before),
            };
            user.is_none_or(host_like)
                && host_like(host)
                && host.len() > 1
                && !path.is_empty()
                && !path.starts_with(':')
        }),
    };
    if fine { Ok(url) } else { Err(LinkError::Url) }
}

fn checked_name(name: String, what: &'static str) -> Result<String, LinkError> {
    if name.starts_with('-') {
        return Err(LinkError::Dash(what));
    }
    Ok(name)
}

fn checked_commit(id: String) -> Result<String, LinkError> {
    if (7..=64).contains(&id.len()) && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(id.to_ascii_lowercase())
    } else {
        Err(LinkError::Commit)
    }
}

fn checked_pull_request(number: String) -> Result<String, LinkError> {
    if (1..=18).contains(&number.len())
        && !number.starts_with('0')
        && number.bytes().all(|byte| byte.is_ascii_digit())
    {
        Ok(number)
    } else {
        Err(LinkError::PullRequest)
    }
}

/// Read a link: `bitbraid://open?` and then `path` or `url`, and at
/// most one of `branch`, `tag`, `commit` and `pr`, each `name=value` with
/// the value percent-encoded, joined by `&`.
///
/// Nothing is looked up: a path is a full path and may lead nowhere, a
/// branch is a name and may not exist. That is for whoever opens it.
pub fn parse_link(text: &str) -> Result<Link, LinkError> {
    if text.len() > MAX_LINK {
        return Err(LinkError::TooLong);
    }
    if text.chars().any(|ch| ch.is_whitespace() || hidden(ch)) {
        return Err(LinkError::RawCharacter);
    }
    let rest = text
        .split_once("://")
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case(SCHEME))
        .map(|(_, rest)| rest)
        .ok_or(LinkError::Scheme)?;
    if rest.contains('#') {
        return Err(LinkError::Fragment);
    }
    let (action, query) = rest.split_once('?').unwrap_or((rest, ""));
    // No path and the path `/` are the same address, and a launcher
    // that takes a link apart and puts it together again (GIO does)
    // hands it over with the slash.
    let action = action.strip_suffix('/').unwrap_or(action);
    if !action.eq_ignore_ascii_case("open") {
        return Err(LinkError::Action);
    }
    if query.is_empty() {
        return Err(LinkError::NoRepository);
    }

    let (mut path, mut url) = (None, None);
    let (mut branch, mut tag, mut commit, mut pull_request) = (None, None, None, None);
    for pair in query.split('&') {
        if pair.is_empty() {
            return Err(LinkError::EmptyParameter);
        }
        let (key, raw) = match pair.split_once('=') {
            Some((key, raw)) => (key, raw),
            None => (pair, ""),
        };
        let (name, slot) = match key {
            "path" => ("path", &mut path),
            "url" => ("url", &mut url),
            "branch" => ("branch", &mut branch),
            "tag" => ("tag", &mut tag),
            "commit" => ("commit", &mut commit),
            "pr" => ("pr", &mut pull_request),
            _ => return Err(LinkError::UnknownParameter(cut(key))),
        };
        if slot.is_some() {
            return Err(LinkError::Twice(name));
        }
        if raw.is_empty() {
            return Err(LinkError::NoValue(name));
        }
        let value = decode(raw, name)?;
        if value.chars().any(hidden) {
            return Err(LinkError::Control(name));
        }
        *slot = Some(value);
    }

    let repository = match (path, url) {
        (Some(path), None) => LinkRepository::Path(checked_path(path)?),
        (None, Some(url)) => LinkRepository::Url(checked_url(url)?),
        (None, None) => return Err(LinkError::NoRepository),
        (Some(_), Some(_)) => return Err(LinkError::TwoRepositories),
    };
    let target = match (branch, tag, commit, pull_request) {
        (None, None, None, None) => LinkTarget::None,
        (Some(name), None, None, None) => LinkTarget::Branch(checked_name(name, "branch")?),
        (None, Some(name), None, None) => LinkTarget::Tag(checked_name(name, "tag")?),
        (None, None, Some(id), None) => LinkTarget::Commit(checked_commit(id)?),
        (None, None, None, Some(number)) => LinkTarget::PullRequest(checked_pull_request(number)?),
        _ => return Err(LinkError::Targets),
    };
    Ok(Link { repository, target })
}

/// Add `name=value` with the value percent-encoded: everything but
/// letters, digits, `-`, `.`, `_`, `~` and, for the eye, `/`, which a
/// query may hold as it is.
fn push_parameter(out: &mut String, name: &str, value: &str) {
    out.push_str(name);
    out.push('=');
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(byte));
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
}

/// Write a link. It is read back before it is handed out, so what is
/// made here is never something [`parse_link`] refuses, and reading it
/// gives what went in (a commit in lower case).
pub fn make_link(link: &Link) -> Result<String, LinkError> {
    let mut out = format!("{SCHEME}://open?");
    match &link.repository {
        LinkRepository::Path(path) => push_parameter(&mut out, "path", path),
        LinkRepository::Url(url) => push_parameter(&mut out, "url", url),
    }
    if let Some(value) = link.target.value() {
        out.push('&');
        push_parameter(&mut out, link.target.kind(), value);
    }
    parse_link(&out)?;
    Ok(out)
}

/// One argument of an `Exec` line, for a text without a percent sign. An
/// argument with anything in it that the specification reserves goes
/// between double quotes, where `"`, `` ` ``, `$` and `\` take a
/// backslash; and a backslash in any value of an entry is written twice,
/// on top of that.
fn exec_argument(text: &str) -> String {
    let plain = !text.is_empty()
        && text.chars().all(|ch| {
            ch.is_ascii_alphanumeric()
                || matches!(ch, '/' | '.' | '_' | '-' | '+' | ',' | ':' | '@')
        });
    if plain {
        return text.to_owned();
    }
    let mut out = String::from('"');
    for ch in text.chars() {
        match ch {
            '"' | '`' | '$' => {
                out.push_str(r"\\");
                out.push(ch);
            }
            '\\' => out.push_str(r"\\\\"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

/// The desktop entry that starts `exe` with a link: what
/// `link.register` writes. The program is named by its full path.
pub fn desktop_entry(exe: &str) -> Result<String, String> {
    if exe.is_empty() {
        return Err(say!("link-empty").to_owned());
    }
    if exe.len() > LONGEST {
        return Err(say!("link-path-too-long").to_owned());
    }
    if exe.chars().any(hidden) {
        return Err(say!("link-control-characters").to_owned());
    }
    if !Path::new(exe).is_absolute() {
        return Err(say!("link-program-not-full").to_owned());
    }
    // The specification has no way to write one in the name of a program.
    if exe.contains('=') {
        return Err(say!("link-desktop-equals").to_owned());
    }
    // It has one for this, `%%`, but a launcher built on GLib looks for
    // the program before it reads that, finds none and drops the entry.
    if exe.contains('%') {
        return Err(say!("link-desktop-percent").to_owned());
    }
    Ok(format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=BitBraid\n\
         Exec={} %u\n\
         MimeType={MIME_TYPE};\n\
         NoDisplay=true\n\
         Terminal=false\n",
        exec_argument(exe)
    ))
}

/// A full path in the way of Windows: on a drive (`C:\`) or on a share of
/// another machine (`\\server\share`). Read the same on every system, so
/// that it is tested on every one.
fn windows_full_path(text: &str) -> bool {
    let mut chars = text.chars();
    let (first, second, third) = (chars.next(), chars.next(), chars.next());
    let slash = |ch: Option<char>| matches!(ch, Some('/' | '\\'));
    let drive =
        first.is_some_and(|ch| ch.is_ascii_alphabetic()) && second == Some(':') && slash(third);
    let share = slash(first) && slash(second) && third.is_some() && !slash(third);
    drive || share
}

/// The command Windows runs for a link: the program between double
/// quotes, then `%1` between its own, which Windows replaces with the
/// link. A program on Windows reads the first as its own name, up to the
/// next double quote, and the second as one argument, blanks and all.
pub fn windows_command(exe: &str) -> Result<String, String> {
    if exe.is_empty() {
        return Err(say!("link-empty").to_owned());
    }
    if exe.len() > LONGEST {
        return Err(say!("link-path-too-long").to_owned());
    }
    if exe.chars().any(hidden) {
        return Err(say!("link-control-characters").to_owned());
    }
    if !windows_full_path(exe) {
        return Err(say!("link-program-not-full").to_owned());
    }
    // There is none in the name of a file on Windows, and one would end
    // the quotes.
    if exe.contains('"') {
        return Err(say!("link-windows-quote").to_owned());
    }
    // Windows reads `%1` and the like in the command as the places of
    // arguments.
    if exe.contains('%') {
        return Err(say!("link-windows-percent").to_owned());
    }
    Ok(format!("\"{exe}\" \"%1\""))
}

/// One value that `link.register` writes on Windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegistryValue {
    /// The key it is in, under the key of the scheme; empty for that one.
    pub key: &'static str,
    /// Its name; empty for the default value of the key.
    pub name: &'static str,
    /// What it holds, as text.
    pub data: String,
}

/// What `link.register` writes on Windows under the key of the scheme,
/// in the order it is written: what the key is, the program's icon, the
/// command that opens a link, and last `URL Protocol`, which makes it the
/// key of a scheme, so that a key written half-way is never taken for
/// one.
pub fn windows_values(exe: &str) -> Result<Vec<RegistryValue>, String> {
    let command = windows_command(exe)?;
    let value = |key, name, data| RegistryValue { key, name, data };
    Ok(vec![
        value("", "", WINDOWS_NAME.to_owned()),
        value("DefaultIcon", "", format!("{exe},0")),
        value(r"shell\open\command", "", command),
        value("", "URL Protocol", String::new()),
    ])
}

/// Write the key of a scheme under `HKEY_CURRENT_USER\Software\Classes`,
/// in place of whatever was there: a verb or a `DelegateExecute` that
/// another program left would be mixed with ours. `scheme` is the name of
/// the key; the tests give one of their own.
#[cfg(windows)]
fn write_scheme_key(scheme: &str, values: &[RegistryValue]) -> Result<(), String> {
    use windows_registry::CURRENT_USER;
    /// What Windows answers for a key that is not there.
    const NOT_FOUND: i32 = 0x8007_0002_u32 as i32;
    let path = format!(r"{WINDOWS_CLASSES}\{scheme}");
    if let Err(err) = CURRENT_USER.remove_tree(&path)
        && err.code().0 != NOT_FOUND
    {
        return Err(say!("link-key-not-removed", error = err.to_string()));
    }
    for value in values {
        let key = if value.key.is_empty() {
            path.clone()
        } else {
            format!(r"{path}\{}", value.key)
        };
        CURRENT_USER
            .create(&key)
            .and_then(|key| key.set_string(value.name, &value.data))
            .map_err(|err| say!("link-key-not-written", error = err.to_string()))?;
    }
    Ok(())
}

/// Where there is no registry.
#[cfg(not(windows))]
fn write_scheme_key(_: &str, _: &[RegistryValue]) -> Result<(), String> {
    Err(say!("link-no-registry").to_owned())
}

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "link.parse",
            label: say!("link-parse"),
            description: say!("link-parse-description"),
        },
        parse,
    );
    let _ = registry.register(
        CommandSpec {
            id: "link.make",
            label: say!("link-make"),
            description: say!("link-make-description"),
        },
        make,
    );
    let _ = registry.register(
        CommandSpec {
            id: "link.register",
            label: say!("link-register"),
            description: say!("link-register-description"),
        },
        register_handler,
    );
}

fn refused(detail: impl Into<String>) -> CommandError {
    CommandError::Git(bitbraid_git::Error::Refused {
        what: say!("link-refused"),
        detail: detail.into(),
    })
}

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

fn parse(_: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let text: String = param(params, "url")?;
    let link = parse_link(&text).map_err(|err| invalid("url", err.to_string()))?;
    Ok(serde_json::to_value(link)?)
}

fn make(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: Option<String> = param_or(params, "path", None)?;
    let url: Option<String> = param_or(params, "url", None)?;
    let branch: Option<String> = param_or(params, "branch", None)?;
    let tag: Option<String> = param_or(params, "tag", None)?;
    let commit: Option<String> = param_or(params, "commit", None)?;
    let repository = match (path, url) {
        (Some(path), None) => LinkRepository::Path(path),
        (None, Some(url)) => LinkRepository::Url(url),
        (Some(_), Some(_)) => {
            return Err(invalid("path", say!("link-path-and-url")));
        }
        (None, None) => {
            let Some(repo) = &ctx.repo else {
                return Err(invalid("path", say!("link-which-repository")));
            };
            // Opened as `.`, as the command line does, the repository
            // knows itself by a relative path; a link names the full one.
            let here = repo.workdir().unwrap_or_else(|| repo.git_dir());
            let here =
                std::path::absolute(here).map_err(|_| refused(say!("link-repository-not-full")))?;
            let here = here
                .to_str()
                .ok_or_else(|| refused(say!("link-repository-not-text")))?;
            LinkRepository::Path(here.to_owned())
        }
    };
    let target = match (branch, tag, commit) {
        (None, None, None) => LinkTarget::None,
        (Some(name), None, None) => LinkTarget::Branch(name),
        (None, Some(name), None) => LinkTarget::Tag(name),
        (None, None, Some(id)) => LinkTarget::Commit(id.to_ascii_lowercase()),
        _ => {
            return Err(invalid("branch", say!("link-one-target")));
        }
    };
    let link = make_link(&Link { repository, target })
        .map_err(|err| refused(say!("link-not-a-link", error = err.to_string())))?;
    Ok(json!({ "link": link }))
}

/// A directory as it was typed: its full path.
fn typed_directory(text: &str) -> Result<PathBuf, CommandError> {
    const NAME: &str = "applications_dir";
    if text.is_empty() {
        return Err(invalid(NAME, say!("link-empty")));
    }
    if text.len() > LONGEST {
        return Err(invalid(NAME, say!("link-path-too-long")));
    }
    if text.chars().any(hidden) {
        return Err(invalid(NAME, say!("link-control-characters")));
    }
    let path = Path::new(text);
    if !path.is_absolute() {
        return Err(invalid(NAME, say!("link-directory-not-full")));
    }
    Ok(path.to_path_buf())
}

/// Where a user's own desktop entries go: `applications` under
/// `$XDG_DATA_HOME`, which is `~/.local/share` when it is not set. A
/// path there that is not a full path counts as not set.
fn applications_directory() -> Option<PathBuf> {
    let full = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    full("XDG_DATA_HOME")
        .or_else(|| full("HOME").map(|home| home.join(".local").join("share")))
        .map(|data| data.join("applications"))
}

/// Make the entry the system's choice for the scheme.
fn make_default() -> Result<(), String> {
    let out = Command::new("xdg-mime")
        .args(["default", DESKTOP_FILE, MIME_TYPE])
        .stdin(Stdio::null())
        .output()
        .map_err(|err| {
            say!(
                "link-program-not-started",
                program = "xdg-mime",
                error = err.to_string()
            )
        })?;
    if out.status.success() {
        return Ok(());
    }
    Err(say!(
        "link-program-failed",
        program = "xdg-mime",
        status = out.status.to_string(),
        said = cut(String::from_utf8_lossy(&out.stderr).trim())
    ))
}

/// The application bundle a program is the executable of: the
/// `<name>.app` of `<name>.app/Contents/MacOS/<program>`.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let named = |path: &Path, name: &str| path.file_name().is_some_and(|it| it == name);
    (named(macos, "MacOS")
        && named(contents, "Contents")
        && bundle
            .extension()
            .is_some_and(|it| it.eq_ignore_ascii_case("app")))
    .then(|| bundle.to_path_buf())
}

/// `link.register` on macOS: the system takes the scheme from the
/// application bundle's Info.plist, and is told about the bundle the
/// program is in. A program in no bundle cannot have the system's links.
fn register_on_mac(
    exe: Option<String>,
    directory: Option<String>,
    dry_run: bool,
    no_mime: bool,
) -> Result<Output, CommandError> {
    if directory.is_some() || no_mime {
        return Err(invalid(
            if directory.is_some() {
                "applications_dir"
            } else {
                "no_mime"
            },
            say!("link-mac-only-bundle"),
        ));
    }
    let exe = match exe {
        Some(exe) => exe,
        None => running_program()?,
    };
    if exe.len() > LONGEST || exe.chars().any(hidden) {
        return Err(invalid("exe", say!("link-mac-no-program")));
    }
    let bundle = bundle_of(Path::new(&exe))
        .ok_or_else(|| refused(say!("link-no-bundle", program = cut(&exe))))?;
    if !Path::new(&exe).is_file() || !bundle.join("Contents/Info.plist").is_file() {
        return Err(refused(say!(
            "link-no-application",
            path = cut(&bundle.display().to_string())
        )));
    }
    let shown = bundle.display().to_string();
    let answer = |written: bool| {
        json!({
            "bundle": shown,
            "command": [LSREGISTER, "-f", shown],
            "dry_run": dry_run,
            "written": written,
        })
    };
    if dry_run {
        return Ok(answer(false));
    }
    let out = Command::new(LSREGISTER)
        .arg("-f")
        .arg(&bundle)
        .stdin(Stdio::null())
        .output()
        .map_err(|err| {
            refused(say!(
                "link-program-not-started",
                program = "lsregister",
                error = err.to_string()
            ))
        })?;
    if !out.status.success() {
        return Err(refused(say!(
            "link-program-failed",
            program = "lsregister",
            status = out.status.to_string(),
            said = cut(String::from_utf8_lossy(&out.stderr).trim())
        )));
    }
    Ok(answer(true))
}

/// The program links are to start: the running one, or, run from an
/// AppImage, the AppImage itself, as the running program lies in a mount
/// of it that is gone when it quits.
fn running_program() -> Result<String, CommandError> {
    let exe = std::env::current_exe().map_err(|_| refused(say!("link-running-program")))?;
    if let Some(image) = appimage_of(
        &exe,
        bitbraid_core::process::var_os("APPIMAGE"),
        bitbraid_core::process::var_os("APPDIR"),
    ) {
        return Ok(image);
    }
    exe.to_str()
        .map(str::to_owned)
        .ok_or_else(|| refused(say!("link-running-program")))
}

/// The AppImage `exe` runs from: the file its runtime names in
/// `APPIMAGE`, when `exe` lies in the directory the runtime mounted it at
/// (`APPDIR`). A program started from another AppImage's program (a
/// terminal, say) inherits that one's variables, and is not in its
/// directory.
fn appimage_of(
    exe: &Path,
    image: Option<std::ffi::OsString>,
    appdir: Option<std::ffi::OsString>,
) -> Option<String> {
    let appdir = PathBuf::from(appdir?);
    let image = image?.into_string().ok()?;
    let ours = cfg!(target_os = "linux")
        && appdir.is_absolute()
        && appdir.components().count() > 1
        && exe.starts_with(&appdir);
    (ours && Path::new(&image).is_absolute() && Path::new(&image).is_file()).then_some(image)
}

/// `link.register` on Windows: the key of the scheme under
/// `HKEY_CURRENT_USER\Software\Classes`, for this user alone. Windows
/// hands links to the program named there as soon as it is written,
/// unless another program was chosen for them in Windows' settings.
fn register_on_windows(
    exe: Option<String>,
    directory: Option<String>,
    dry_run: bool,
    no_mime: bool,
) -> Result<Output, CommandError> {
    if directory.is_some() {
        return Err(invalid("applications_dir", say!("link-windows-no-desktop")));
    }
    if no_mime {
        return Err(invalid("no_mime", say!("link-windows-no-mime")));
    }
    let exe = match exe {
        Some(exe) => exe,
        None => running_program()?,
    };
    let command = windows_command(&exe).map_err(|why| invalid("exe", why))?;
    let values = windows_values(&exe).map_err(|why| invalid("exe", why))?;
    if !Path::new(&exe).is_file() {
        return Err(refused(say!("link-no-program", path = cut(&exe))));
    }
    let answer = |written: bool| {
        json!({
            "key": format!(r"HKEY_CURRENT_USER\{WINDOWS_CLASSES}\{SCHEME}"),
            "command": command,
            "values": values,
            "dry_run": dry_run,
            "written": written,
        })
    };
    if dry_run {
        return Ok(answer(false));
    }
    write_scheme_key(SCHEME, &values).map_err(refused)?;
    Ok(answer(true))
}

fn register_handler(_: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    // Everything is read and checked before anything is written.
    let exe: Option<String> = param_or(params, "exe", None)?;
    let directory: Option<String> = param_or(params, "applications_dir", None)?;
    let dry_run: bool = param_or(params, "dry_run", false)?;
    let no_mime: bool = param_or(params, "no_mime", false)?;
    if cfg!(windows) {
        return register_on_windows(exe, directory, dry_run, no_mime);
    }
    if cfg!(target_os = "macos") {
        return register_on_mac(exe, directory, dry_run, no_mime);
    }
    if !cfg!(target_os = "linux") {
        return Err(refused(say!("link-other-system")));
    }
    let exe = match exe {
        Some(exe) => exe,
        None => running_program()?,
    };
    let entry = desktop_entry(&exe).map_err(|why| invalid("exe", why))?;
    if !Path::new(&exe).is_file() {
        return Err(refused(say!("link-no-program", path = cut(&exe))));
    }
    let directory = match directory {
        Some(text) => typed_directory(&text)?,
        None => applications_directory()
            .ok_or_else(|| refused(say!("link-no-applications-directory")))?,
    };
    let file = directory.join(DESKTOP_FILE);
    let answer = |written: bool, default_set: bool, problem: Option<String>| {
        json!({
            "file": file.display().to_string(),
            "entry": entry,
            "dry_run": dry_run,
            "written": written,
            "default_set": default_set,
            "problem": problem,
        })
    };
    if dry_run {
        return Ok(answer(false, false, None));
    }

    fs::create_dir_all(&directory)
        .map_err(|err| refused(say!("link-directory-not-made", error = err.to_string())))?;
    // Beside it first, then in its place: there is never half an entry.
    let beside = directory.join(format!("{DESKTOP_FILE}.tmp"));
    fs::write(&beside, &entry)
        .map_err(|err| refused(say!("link-entry-not-written", error = err.to_string())))?;
    if let Err(err) = fs::rename(&beside, &file) {
        let _ = fs::remove_file(&beside);
        return Err(refused(say!(
            "link-entry-not-placed",
            error = err.to_string()
        )));
    }
    if no_mime {
        return Ok(answer(true, false, None));
    }
    // The entry is there either way; without this the system may still
    // offer the application, but has not been told to prefer it.
    Ok(match make_default() {
        Ok(()) => answer(true, true, None),
        Err(problem) => answer(true, false, Some(problem)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn links_name_the_appimage_its_own_program_runs_from() {
        let scratch =
            std::env::temp_dir().join(format!("bitbraid-appimage-{}", std::process::id()));
        let _ = fs::remove_dir_all(&scratch);
        fs::create_dir_all(&scratch).unwrap();
        let image = scratch.join("BitBraid.AppImage");
        fs::write(&image, b"").unwrap();
        let image = image.to_str().unwrap().to_owned();
        let os = |text: &str| Some(std::ffi::OsString::from(text));
        let mount = "/tmp/.mount_BitBraXYZ";
        let exe = Path::new("/tmp/.mount_BitBraXYZ/usr/bin/bitbraid");

        let found = appimage_of(exe, os(&image), os(mount));
        if cfg!(target_os = "linux") {
            assert_eq!(found.as_deref(), Some(image.as_str()));
        } else {
            assert_eq!(found, None);
        }
        // Started from another AppImage's terminal: its variables, not its
        // directory.
        let installed = Path::new("/usr/bin/bitbraid");
        assert_eq!(appimage_of(installed, os(&image), os(mount)), None);
        // A relative name, a file that is not there, the root as the
        // mount, or a variable missing: the running program.
        assert_eq!(appimage_of(exe, os("BitBraid.AppImage"), os(mount)), None);
        assert_eq!(
            appimage_of(exe, os("/nowhere/BitBraid.AppImage"), os(mount)),
            None
        );
        assert_eq!(appimage_of(exe, os(&image), os("/")), None);
        assert_eq!(appimage_of(exe, os(&image), os("")), None);
        assert_eq!(appimage_of(exe, os(&image), None), None);
        assert_eq!(appimage_of(exe, None, os(mount)), None);
        let _ = fs::remove_dir_all(&scratch);
    }

    #[test]
    fn a_program_is_in_the_bundle_three_folders_up() {
        let bundle = |exe: &str| bundle_of(Path::new(exe));
        assert_eq!(
            bundle("/Applications/BitBraid.app/Contents/MacOS/bitbraid"),
            Some(PathBuf::from("/Applications/BitBraid.app"))
        );
        assert_eq!(
            bundle("/x/Y.APP/Contents/MacOS/z"),
            Some(PathBuf::from("/x/Y.APP"))
        );
        for loose in [
            "/usr/local/bin/bitbraid",
            "/Applications/BitBraid.app/Contents/bitbraid",
            "/Applications/BitBraid/Contents/MacOS/bitbraid",
            "/Applications/BitBraid.app/Other/MacOS/bitbraid",
            "Contents/MacOS/bitbraid",
            "bitbraid",
            "",
        ] {
            assert_eq!(bundle(loose), None, "{loose}");
        }
    }

    fn path_link(path: &str, target: LinkTarget) -> Link {
        Link {
            repository: LinkRepository::Path(path.to_owned()),
            target,
        }
    }

    #[test]
    fn a_link_is_read_into_what_it_names() {
        assert_eq!(
            parse_link("bitbraid://open?path=/home/ada/work").unwrap(),
            path_link("/home/ada/work", LinkTarget::None)
        );
        assert_eq!(
            parse_link("bitbraid://open?path=%2Fhome%2Fada%2Fmy%20work&branch=feature%2Fa%26b")
                .unwrap(),
            path_link(
                "/home/ada/my work",
                LinkTarget::Branch("feature/a&b".to_owned())
            )
        );
        // The parameters in any order; scheme and action in any case.
        assert_eq!(
            parse_link("BitBraid://OPEN?tag=v1.0&path=C%3A%5CUsers%5Cada%5Crepo").unwrap(),
            path_link(r"C:\Users\ada\repo", LinkTarget::Tag("v1.0".to_owned()))
        );
        assert_eq!(
            parse_link("bitbraid://open?url=https://example.invalid/a/b.git&commit=ABCDEF0")
                .unwrap(),
            Link {
                repository: LinkRepository::Url("https://example.invalid/a/b.git".to_owned()),
                target: LinkTarget::Commit("abcdef0".to_owned()),
            }
        );
        assert_eq!(
            parse_link("bitbraid://open?url=git%40example.invalid%3Aa%2Fb.git&pr=128").unwrap(),
            Link {
                repository: LinkRepository::Url("git@example.invalid:a/b.git".to_owned()),
                target: LinkTarget::PullRequest("128".to_owned()),
            }
        );
        // With the slash a launcher may put after the action.
        assert_eq!(
            parse_link("bitbraid://open/?path=/a&branch=b").unwrap(),
            path_link("/a", LinkTarget::Branch("b".to_owned()))
        );
        // A plus sign is a plus sign; = and ? may stand in a value.
        assert_eq!(
            parse_link("bitbraid://open?path=/c++/a=b?c&branch=c++").unwrap(),
            path_link("/c++/a=b?c", LinkTarget::Branch("c++".to_owned()))
        );
        // Text that is not ASCII, escaped or as it is.
        for link in [
            "bitbraid://open?path=/home/%C3%A4d%C3%A4/%E9%8D%B5",
            "bitbraid://open?path=/home/ädä/鍵",
        ] {
            assert_eq!(
                parse_link(link).unwrap(),
                path_link("/home/ädä/鍵", LinkTarget::None)
            );
        }
    }

    #[test]
    fn what_is_not_a_link_of_ours_is_refused_with_the_reason() {
        use LinkError as E;
        let long = format!("bitbraid://open?path=/{}", "a".repeat(MAX_LINK));
        let just = format!("bitbraid://open?path=/{}", "a".repeat(MAX_LINK - 22));
        assert_eq!(just.len(), MAX_LINK);
        assert!(parse_link(&just).is_ok(), "exactly 4 KiB is read");
        let cases: Vec<(&str, E)> = vec![
            (long.as_str(), E::TooLong),
            ("", E::Scheme),
            ("bitbraid", E::Scheme),
            ("bitbraid:open?path=/a", E::Scheme),
            ("bitbraid:/open?path=/a", E::Scheme),
            ("https://open?path=/a", E::Scheme),
            ("file:///etc/passwd", E::Scheme),
            ("bitbraidx://open?path=/a", E::Scheme),
            ("xbitbraid://open?path=/a", E::Scheme),
            ("x://bitbraid://open?path=/a", E::Scheme),
            ("://open?path=/a", E::Scheme),
            ("bitbraid://?path=/a", E::Action),
            ("bitbraid://clone?url=https://x/y", E::Action),
            ("bitbraid://open//?path=/a", E::Action),
            ("bitbraid://open/x?path=/a", E::Action),
            ("bitbraid://open/x/?path=/a", E::Action),
            ("bitbraid:///?path=/a", E::Action),
            ("bitbraid://open/", E::NoRepository),
            ("bitbraid://opens?path=/a", E::Action),
            ("bitbraid://user@open?path=/a", E::Action),
            ("bitbraid://open:80?path=/a", E::Action),
            ("bitbraid://path=/a", E::Action),
            ("bitbraid://open", E::NoRepository),
            ("bitbraid://open?", E::NoRepository),
            ("bitbraid://open?branch=main", E::NoRepository),
            (
                "bitbraid://open?path=/a&url=https://x/y",
                E::TwoRepositories,
            ),
            ("bitbraid://open?path=/a&branch=b&tag=t", E::Targets),
            ("bitbraid://open?path=/a&commit=abcdef0&pr=1", E::Targets),
            (
                "bitbraid://open?path=/a&branch=b&tag=t&commit=abcdef0&pr=1",
                E::Targets,
            ),
            ("bitbraid://open?path=/a&path=/b", E::Twice("path")),
            (
                "bitbraid://open?path=/a&branch=b&branch=b",
                E::Twice("branch"),
            ),
            (
                "bitbraid://open?url=https://x/y&url=https://x/y",
                E::Twice("url"),
            ),
            ("bitbraid://open?path=/a&", E::EmptyParameter),
            ("bitbraid://open?&path=/a", E::EmptyParameter),
            ("bitbraid://open?path=/a&&branch=b", E::EmptyParameter),
            ("bitbraid://open?path", E::NoValue("path")),
            ("bitbraid://open?path=", E::NoValue("path")),
            ("bitbraid://open?path=/a&branch=", E::NoValue("branch")),
            (
                "bitbraid://open?path=/a&file=x",
                E::UnknownParameter("file".to_owned()),
            ),
            (
                "bitbraid://open?Path=/a",
                E::UnknownParameter("Path".to_owned()),
            ),
            (
                "bitbraid://open?p%61th=/a",
                E::UnknownParameter("p%61th".to_owned()),
            ),
            ("bitbraid://open?=/a", E::UnknownParameter(String::new())),
            ("bitbraid://open?path=/a#branch=b", E::Fragment),
            ("bitbraid://open#?path=/a", E::Fragment),
            ("bitbraid://open?path=/a&branch=issue-#12", E::Fragment),
            ("bitbraid://open?path=/a b", E::RawCharacter),
            ("bitbraid://open?path=/a\n", E::RawCharacter),
            ("bitbraid://open?path=/a\tb", E::RawCharacter),
            (" bitbraid://open?path=/a", E::RawCharacter),
            ("bitbraid://open?path=/a\u{0}", E::RawCharacter),
            ("bitbraid://open?path=/a\u{a0}b", E::RawCharacter),
            ("bitbraid://open?path=/a\u{202e}b", E::RawCharacter),
            ("bitbraid://open?path=/a%", E::Escape("path")),
            ("bitbraid://open?path=/a%2", E::Escape("path")),
            ("bitbraid://open?path=/a%zz", E::Escape("path")),
            ("bitbraid://open?path=/a%2g", E::Escape("path")),
            ("bitbraid://open?path=/a%%20", E::Escape("path")),
            ("bitbraid://open?path=/a&branch=%+1", E::Escape("branch")),
            ("bitbraid://open?path=/a%FF", E::NotUtf8("path")),
            ("bitbraid://open?path=/a%C3", E::NotUtf8("path")),
            ("bitbraid://open?path=/a%C3%28", E::NotUtf8("path")),
            ("bitbraid://open?path=/a%ED%A0%80", E::NotUtf8("path")),
            ("bitbraid://open?path=/a%C0%AF", E::NotUtf8("path")),
            ("bitbraid://open?path=/a&tag=%E9%8D", E::NotUtf8("tag")),
            ("bitbraid://open?path=/a%00", E::Control("path")),
            ("bitbraid://open?path=/a%0Ab", E::Control("path")),
            ("bitbraid://open?path=/a%1B%5B31m", E::Control("path")),
            ("bitbraid://open?path=/a%7F", E::Control("path")),
            ("bitbraid://open?path=/a%C2%85", E::Control("path")),
            ("bitbraid://open?path=/a%E2%80%AE", E::Control("path")),
            (
                "bitbraid://open?path=/a&branch=b%0D%0Ac",
                E::Control("branch"),
            ),
            ("bitbraid://open?url=https://x/y%0A", E::Control("url")),
            ("bitbraid://open?path=a", E::PathNotAbsolute),
            ("bitbraid://open?path=./a", E::PathNotAbsolute),
            ("bitbraid://open?path=../../etc", E::PathNotAbsolute),
            ("bitbraid://open?path=~/work", E::PathNotAbsolute),
            ("bitbraid://open?path=%20/a", E::PathNotAbsolute),
            ("bitbraid://open?path=C:", E::PathNotAbsolute),
            ("bitbraid://open?path=C:repo", E::PathNotAbsolute),
            ("bitbraid://open?path=%5Crepo", E::PathNotAbsolute),
            ("bitbraid://open?path=-x", E::PathNotAbsolute),
            ("bitbraid://open?path=//server/share", E::PathElsewhere),
            (
                "bitbraid://open?path=%5C%5Cserver%5Cshare",
                E::PathElsewhere,
            ),
            (
                "bitbraid://open?path=%5C%5C%3F%5CC%3A%5Crepo",
                E::PathElsewhere,
            ),
            ("bitbraid://open?path=/%5Cserver/share", E::PathElsewhere),
            ("bitbraid://open?path=/a&commit=abcdef", E::Commit),
            ("bitbraid://open?path=/a&commit=abcdefg", E::Commit),
            ("bitbraid://open?path=/a&commit=HEAD", E::Commit),
            ("bitbraid://open?path=/a&commit=abcdef0%5E", E::Commit),
            ("bitbraid://open?path=/a&commit=-abcdef0", E::Commit),
            ("bitbraid://open?path=/a&commit=abcdef0..1234567", E::Commit),
            ("bitbraid://open?path=/a&pr=0", E::PullRequest),
            ("bitbraid://open?path=/a&pr=007", E::PullRequest),
            ("bitbraid://open?path=/a&pr=-1", E::PullRequest),
            ("bitbraid://open?path=/a&pr=1e3", E::PullRequest),
            (
                "bitbraid://open?path=/a&pr=12345678901234567890",
                E::PullRequest,
            ),
            ("bitbraid://open?path=/a&branch=-D", E::Dash("branch")),
            (
                "bitbraid://open?path=/a&branch=--upload-pack%3Dx",
                E::Dash("branch"),
            ),
            ("bitbraid://open?path=/a&tag=-d", E::Dash("tag")),
        ];
        for (link, why) in cases {
            assert_eq!(parse_link(link), Err(why), "{link:?}");
        }
        let longer_commit = format!("bitbraid://open?path=/a&commit={}", "a".repeat(65));
        assert_eq!(parse_link(&longer_commit), Err(E::Commit));
        let full = format!("bitbraid://open?path=/a&commit={}", "A".repeat(64));
        assert_eq!(
            parse_link(&full).unwrap().target,
            LinkTarget::Commit("a".repeat(64))
        );
    }

    #[test]
    fn an_address_is_one_that_reaches_a_server_and_does_nothing_else() {
        let read = |url: &str| {
            let mut link = String::from("bitbraid://open?");
            push_parameter(&mut link, "url", url);
            parse_link(&link).map(|link| link.repository)
        };
        for url in [
            "https://example.invalid/a/b.git",
            "http://example.invalid:8080/a/b",
            "https://org@example.invalid/org/project/_git/repo",
            "ssh://git@example.invalid/a/b.git",
            "ssh://git@example.invalid:2222/a/b.git",
            "ssh://[2001:db8::1]/a/b.git",
            "git://example.invalid/a/b.git",
            "git@example.invalid:a/b.git",
            "example.invalid:a/b.git",
            "build_user@host-1.example.invalid:/srv/git/a.git",
        ] {
            assert_eq!(read(url), Ok(LinkRepository::Url(url.to_owned())), "{url}");
        }
        for url in [
            // Programs to run.
            "ext::sh -c 'touch /tmp/x'",
            "ext::ssh://example.invalid/a",
            "fd::17",
            "foo::https://example.invalid/a",
            "https::example.invalid/a",
            // Options.
            "-u",
            "--upload-pack=touch /tmp/x",
            "--upload-pack=x:y",
            "-oProxyCommand=x@example.invalid:a",
            "git@-oProxyCommand=x:a",
            "ssh://-oProxyCommand=x/a",
            "ssh://git@-oProxyCommand=x/a",
            "ssh://-oProxyCommand=x@example.invalid/a",
            // Paths, here or on a drive.
            "/srv/git/a.git",
            "./a.git",
            "../a.git",
            "a.git",
            "C:/repos/a.git",
            "C:\\repos\\a.git",
            "file:///srv/git/a.git",
            "dir/with:colon",
            // Other schemes, and none.
            "ftp://example.invalid/a",
            "HTTPS://example.invalid/a",
            "javascript://example.invalid/%0Aalert(1)",
            "https://",
            "https:///a/b",
            "ssh://git@/a",
            "://example.invalid/a",
            "example.invalid:",
            "example.invalid",
            ":a/b",
            "@example.invalid:a",
            "git@:a",
            "a b@example.invalid:a",
        ] {
            assert_eq!(read(url), Err(LinkError::Url), "{url}");
        }
        for url in [
            "https://ada:hunter2@example.invalid/a.git",
            "https://:hunter2@example.invalid/a.git",
            "ssh://git:secret@example.invalid/a.git",
        ] {
            assert_eq!(read(url), Err(LinkError::UrlPassword), "{url}");
        }
    }

    #[test]
    fn a_link_that_is_made_reads_back_as_what_went_in() {
        let repositories = [
            LinkRepository::Path("/home/ada/work".to_owned()),
            LinkRepository::Path("/with space/and & and = and # and % and ? and +".to_owned()),
            LinkRepository::Path("/quotes \"double\" 'single' `back`".to_owned()),
            LinkRepository::Path("/ünï/ключ/鍵/🦑".to_owned()),
            LinkRepository::Path(r"C:\Users\Ada Lovelace\my repo".to_owned()),
            LinkRepository::Path("D:/work/100%/a&b".to_owned()),
            LinkRepository::Path("/%2F%20%zz/already%escaped".to_owned()),
            LinkRepository::Path("/~user/a.b-c_d".to_owned()),
            LinkRepository::Url("https://example.invalid/a/b.git".to_owned()),
            LinkRepository::Url("https://example.invalid/a/b.git?x=1&y=2".to_owned()),
            LinkRepository::Url("ssh://git@example.invalid:2222/~ada/b.git".to_owned()),
            LinkRepository::Url("git@example.invalid:a/b c.git".to_owned()),
        ];
        let targets = [
            LinkTarget::None,
            LinkTarget::Branch("main".to_owned()),
            LinkTarget::Branch("feature/a&b=c#d%e+f".to_owned()),
            LinkTarget::Branch("ünï/ветка".to_owned()),
            LinkTarget::Tag("v1.0 final".to_owned()),
            LinkTarget::Tag("release/2026-10-10@{0}".to_owned()),
            LinkTarget::Commit("abcdef0".to_owned()),
            LinkTarget::Commit("0123456789abcdef0123456789abcdef01234567".to_owned()),
            LinkTarget::PullRequest("128".to_owned()),
        ];
        for repository in &repositories {
            for target in &targets {
                let link = Link {
                    repository: repository.clone(),
                    target: target.clone(),
                };
                let made = make_link(&link).unwrap();
                assert!(
                    made.bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || b"-._~/%:?&=".contains(&byte)),
                    "{made}"
                );
                assert_eq!(parse_link(&made).as_ref(), Ok(&link), "{made}");
            }
        }
        assert_eq!(
            make_link(&path_link(
                "/home/ada/my work",
                LinkTarget::Branch("a&b".to_owned())
            ))
            .unwrap(),
            "bitbraid://open?path=/home/ada/my%20work&branch=a%26b"
        );
    }

    #[test]
    fn a_link_that_could_not_be_read_is_not_made() {
        for (link, why) in [
            (
                path_link("relative", LinkTarget::None),
                LinkError::PathNotAbsolute,
            ),
            (path_link("", LinkTarget::None), LinkError::NoValue("path")),
            (
                path_link("/a\nb", LinkTarget::None),
                LinkError::Control("path"),
            ),
            (
                path_link("//server/share", LinkTarget::None),
                LinkError::PathElsewhere,
            ),
            (
                path_link("/a", LinkTarget::Branch(String::new())),
                LinkError::NoValue("branch"),
            ),
            (
                path_link("/a", LinkTarget::Branch("-x".to_owned())),
                LinkError::Dash("branch"),
            ),
            (
                path_link("/a", LinkTarget::Commit("HEAD".to_owned())),
                LinkError::Commit,
            ),
            (
                path_link("/a", LinkTarget::PullRequest("one".to_owned())),
                LinkError::PullRequest,
            ),
            (
                path_link(&format!("/{}", "é".repeat(700)), LinkTarget::None),
                LinkError::TooLong,
            ),
            (
                Link {
                    repository: LinkRepository::Url("ext::sh -c id".to_owned()),
                    target: LinkTarget::None,
                },
                LinkError::Url,
            ),
        ] {
            assert_eq!(make_link(&link), Err(why), "{link:?}");
        }
    }

    #[test]
    fn a_target_is_shown_as_kind_and_value() {
        let shown = |link: &str| serde_json::to_value(parse_link(link).unwrap()).unwrap();
        assert_eq!(
            shown("bitbraid://open?path=/a"),
            json!({ "repository": { "path": "/a" }, "target": { "kind": "none", "value": null } })
        );
        assert_eq!(
            shown("bitbraid://open?url=https://x/y&pr=7"),
            json!({ "repository": { "url": "https://x/y" }, "target": { "kind": "pr", "value": "7" } })
        );
        for kind in ["branch", "tag"] {
            assert_eq!(
                shown(&format!("bitbraid://open?path=/a&{kind}=x"))["target"],
                json!({ "kind": kind, "value": "x" })
            );
        }
        assert_eq!(
            shown("bitbraid://open?path=/a&commit=ABCDEF0")["target"],
            json!({ "kind": "commit", "value": "abcdef0" })
        );
    }

    #[test]
    fn hostile_text_never_gets_past_the_reader() {
        // Every single byte in every position of a link that reads, and
        // every byte as an escape: an answer or a refusal, and what is
        // answered has nothing hidden in it.
        let base = "bitbraid://open?path=/a&branch=b";
        for position in 0..=base.len() {
            for byte in 0u8..=255 {
                let mut bytes = base.as_bytes().to_vec();
                bytes.insert(position, byte);
                let Ok(text) = String::from_utf8(bytes) else {
                    continue;
                };
                if let Ok(link) = parse_link(&text) {
                    let LinkRepository::Path(path) = &link.repository else {
                        panic!("{text:?} named an address");
                    };
                    assert!(
                        path.starts_with('/') && !path.chars().any(hidden),
                        "{text:?}"
                    );
                }
            }
        }
        for byte in 0u8..=255 {
            let text = format!("bitbraid://open?path=/a%{byte:02x}b");
            match parse_link(&text) {
                Ok(link) => {
                    let ch = char::from(byte);
                    assert!(byte.is_ascii() && !ch.is_control(), "{text}");
                    assert_eq!(
                        link.repository,
                        LinkRepository::Path(format!("/a{ch}b")),
                        "{text}"
                    );
                }
                Err(why) => assert!(
                    matches!(why, LinkError::Control("path") | LinkError::NotUtf8("path")),
                    "{text}: {why}"
                ),
            }
        }
        // Lengths around every bound, and text that is cut anywhere.
        let long = format!(
            "bitbraid://open?path=/{}&branch={}&commit=abcdef0",
            "%C3%A9".repeat(300),
            "x".repeat(900)
        );
        for end in 0..=long.len() {
            if let Some(part) = long.get(..end) {
                let _ = parse_link(part);
            }
        }
        for count in [0, 1, 1000, 100_000] {
            let _ = parse_link(&"&".repeat(count));
            let _ = parse_link(&format!("bitbraid://open?{}", "&".repeat(count)));
            let _ = parse_link(&format!("bitbraid://open?{}", "path=/a&".repeat(count)));
            let _ = parse_link(&format!("bitbraid://open?path={}", "%".repeat(count)));
            let _ = parse_link(&format!("bitbraid://{}", "open?".repeat(count)));
            let _ = parse_link(&"://".repeat(count));
        }
        // An error says what is wrong in a line, whatever it was given.
        let junk = format!("bitbraid://open?{}=1", "k".repeat(3000));
        let said = parse_link(&junk).unwrap_err().to_string();
        assert!(said.len() < 300 && !said.contains('\n'), "{said}");
    }

    /// An `Exec` value read the way the specification says a launcher
    /// reads it: the escapes of a string first, then the arguments with
    /// their quoting. The last argument is the field code.
    #[cfg(unix)]
    fn exec_arguments(value: &str) -> Vec<String> {
        let mut unescaped = String::new();
        let mut chars = value.chars();
        while let Some(ch) = chars.next() {
            if ch != '\\' {
                unescaped.push(ch);
                continue;
            }
            match chars.next().unwrap() {
                's' => unescaped.push(' '),
                'n' => unescaped.push('\n'),
                't' => unescaped.push('\t'),
                'r' => unescaped.push('\r'),
                '\\' => unescaped.push('\\'),
                other => panic!("\\{other} is no escape of a string"),
            }
        }
        let mut arguments = Vec::new();
        let mut chars = unescaped.chars().peekable();
        while chars.peek().is_some() {
            let mut argument = String::new();
            if chars.peek() == Some(&'"') {
                chars.next();
                loop {
                    match chars.next().expect("a quote that is closed") {
                        '"' => break,
                        '\\' => {
                            let escaped = chars.next().unwrap();
                            assert!("\"`$\\".contains(escaped), "\\{escaped} in quotes");
                            argument.push(escaped);
                        }
                        ch => {
                            assert!(!"`$".contains(ch), "{ch} without its backslash");
                            argument.push(ch);
                        }
                    }
                }
                assert!(matches!(chars.next(), None | Some(' ')));
            } else {
                for ch in chars.by_ref() {
                    if ch == ' ' {
                        break;
                    }
                    assert!(
                        !"\t\n\"'\\><~|&;$*?#()`".contains(ch),
                        "{ch} is reserved and not quoted"
                    );
                    argument.push(ch);
                }
            }
            arguments.push(argument);
        }
        // A percent sign anywhere but in the field code would be one too.
        let last = arguments.len() - 1;
        for argument in &arguments[..last] {
            assert!(!argument.contains('%'), "{argument}");
        }
        arguments
    }

    #[cfg(unix)]
    #[test]
    fn the_desktop_entry_names_the_program_as_one_argument() {
        assert_eq!(
            desktop_entry("/usr/bin/bitbraid").unwrap(),
            "[Desktop Entry]\nType=Application\nName=BitBraid\nExec=/usr/bin/bitbraid %u\n\
             MimeType=x-scheme-handler/bitbraid;\nNoDisplay=true\nTerminal=false\n"
        );
        for exe in [
            "/usr/bin/bitbraid",
            "/opt/Bit Braid/bin/bitbraid",
            "/home/ada/it's here/bitbraid",
            "/quotes \"double\"/bitbraid",
            "/dollar $HOME $(id)/bitbraid",
            "/back\\slash\\/bitbraid",
            "/back`tick`/bitbraid",
            "/semi;colon & pipe | star * hash #/bitbraid",
            "/ünï/ключ/bitbraid",
            "/trailing backslash\\",
            "/~tilde/(parens)/<angles>/?",
        ] {
            let entry = desktop_entry(exe).unwrap();
            let lines: Vec<&str> = entry.lines().collect();
            assert_eq!(lines.len(), 7, "{entry}");
            let exec = lines
                .iter()
                .find_map(|line| line.strip_prefix("Exec="))
                .unwrap();
            assert_eq!(exec_arguments(exec), [exe, "%u"], "{exec}");
        }
        let long = format!("/{}", "x".repeat(5000));
        for exe in [
            "",
            "bitbraid",
            "./bitbraid",
            "bin/bitbraid",
            "/line\nbreak/bitbraid",
            "/tab\there",
            "/nul\0",
            "/with=equals/bitbraid",
            "/100%/bitbraid",
            "/field %u code/bitbraid",
            "/usr/bin/bitbraid\nExec=/bin/sh",
            long.as_str(),
        ] {
            assert!(desktop_entry(exe).is_err(), "{exe:?} was taken");
        }
    }

    /// A command line read the way a program on Windows reads its own
    /// (Microsoft's C runtime and Rust's standard library alike): its
    /// name up to the next double quote when it begins with one, else up
    /// to a blank; then the arguments, where a double quote begins or ends
    /// a quoted part, two of them in a quoted part stand for one, and
    /// backslashes count only before a double quote.
    fn windows_arguments(line: &str) -> Vec<String> {
        let mut chars = line.chars().peekable();
        let mut program = String::new();
        if chars.peek() == Some(&'"') {
            chars.next();
            for ch in chars.by_ref() {
                if ch == '"' {
                    break;
                }
                program.push(ch);
            }
        } else {
            while let Some(&ch) = chars.peek() {
                if ch == ' ' || ch == '\t' {
                    break;
                }
                program.push(ch);
                chars.next();
            }
        }
        let mut arguments = vec![program];
        loop {
            while matches!(chars.peek(), Some(' ' | '\t')) {
                chars.next();
            }
            if chars.peek().is_none() {
                return arguments;
            }
            let mut argument = String::new();
            let mut quoted = false;
            while let Some(&ch) = chars.peek() {
                match ch {
                    '\\' => {
                        let mut count = 0;
                        while chars.peek() == Some(&'\\') {
                            chars.next();
                            count += 1;
                        }
                        if chars.peek() == Some(&'"') {
                            argument.push_str(&"\\".repeat(count / 2));
                            if count % 2 == 1 {
                                argument.push('"');
                                chars.next();
                            }
                        } else {
                            argument.push_str(&"\\".repeat(count));
                        }
                    }
                    '"' => {
                        chars.next();
                        if quoted && chars.peek() == Some(&'"') {
                            argument.push('"');
                            chars.next();
                        } else {
                            quoted = !quoted;
                        }
                    }
                    ' ' | '\t' if !quoted => break,
                    _ => {
                        argument.push(ch);
                        chars.next();
                    }
                }
            }
            arguments.push(argument);
        }
    }

    #[test]
    fn the_windows_command_names_the_program_and_the_link_as_one_argument_each() {
        assert_eq!(
            windows_command(r"C:\Program Files\BitBraid\bitbraid.exe").unwrap(),
            r#""C:\Program Files\BitBraid\bitbraid.exe" "%1""#
        );
        // The reader reads what it should.
        assert_eq!(
            windows_arguments(r#"a.exe "b c" d\\"e f" "g""h" i\"j \\k"#),
            ["a.exe", "b c", r"d\e f", r#"g"h"#, r#"i"j"#, r"\\k"]
        );
        let links = [
            "bitbraid://open?path=/a",
            "bitbraid://open?path=C:%5CUsers%5CAda%20Lovelace%5Crepo&branch=a%26b",
            "bitbraid://open?url=https%3A//example.invalid/a.git&tag=v1.0",
            // As a browser may hand one over that a person typed.
            r"bitbraid://open?path=C:\Users\Ada Lovelace\repo&branch=a&b",
        ];
        for exe in [
            r"C:\Program Files\BitBraid\bitbraid.exe",
            r"C:\Program Files (x86)\Bit Braid\bitbraid.exe",
            r"D:\it's here\a & b\bitbraid.exe",
            r"C:\semi;colon,comma=equals^caret!bang\bitbraid.exe",
            r"C:\Users\Ádá Lövelace\鍵\🦑\bitbraid.exe",
            r"C:\two\\backslashes\bitbraid.exe",
            r"C:\trailing space \bitbraid.exe",
            r"\\server\share\Bit Braid\bitbraid.exe",
            "C:/forward/slashes/bitbraid.exe",
            r"c:\lower\case\bitbraid.exe",
        ] {
            let command = windows_command(exe).unwrap();
            assert!(command.ends_with(r#"" "%1""#), "{command}");
            for link in links {
                let line = command.replace("%1", link);
                assert_eq!(windows_arguments(&line), [exe, link], "{line}");
            }
        }
        let long = format!(r"C:\{}", "x".repeat(5000));
        for (exe, because) in [
            ("", "must not be empty"),
            ("bitbraid.exe", "full path"),
            (r".\bitbraid.exe", "full path"),
            (r"bin\bitbraid.exe", "full path"),
            ("C:bitbraid.exe", "full path"),
            ("C:", "full path"),
            (r"\bitbraid.exe", "full path"),
            (r"\\", "full path"),
            (r"\\\server\bitbraid.exe", "full path"),
            ("/usr/bin/bitbraid", "full path"),
            ("1:\\bitbraid.exe", "full path"),
            (r#"C:\with"quote\bitbraid.exe"#, "double quote"),
            (r#"C:\x.exe" "--evil"#, "double quote"),
            (r"C:\100%\bitbraid.exe", "with % in its path"),
            (r"C:\%1\bitbraid.exe", "with % in its path"),
            ("C:\\line\nbreak\\bitbraid.exe", "control characters"),
            ("C:\\nul\0.exe", "control characters"),
            ("C:\\turned\u{202e}exe.rcs", "control characters"),
            (long.as_str(), "too long"),
        ] {
            let err = windows_command(exe).unwrap_err();
            assert!(err.contains(because), "{exe:?}: {err}");
            assert!(windows_values(exe).is_err(), "{exe:?}");
        }
    }

    #[test]
    fn the_windows_key_is_that_of_a_scheme_with_its_icon_and_command() {
        let exe = r"C:\Program Files\BitBraid\bitbraid.exe";
        let values = windows_values(exe).unwrap();
        let shown: Vec<(&str, &str, &str)> = values
            .iter()
            .map(|value| (value.key, value.name, value.data.as_str()))
            .collect();
        assert_eq!(
            shown,
            [
                ("", "", "URL:BitBraid link"),
                (
                    "DefaultIcon",
                    "",
                    r"C:\Program Files\BitBraid\bitbraid.exe,0"
                ),
                (
                    r"shell\open\command",
                    "",
                    r#""C:\Program Files\BitBraid\bitbraid.exe" "%1""#
                ),
                // Last: a key written half-way is no scheme's.
                ("", "URL Protocol", ""),
            ]
        );
        assert_eq!(
            serde_json::to_value(&values[1]).unwrap(),
            json!({ "key": "DefaultIcon", "name": "", "data": format!("{exe},0") })
        );
    }

    /// A key of the test's own under `HKEY_CURRENT_USER\Software\Classes`,
    /// in place of the user's own `bitbraid`, removed when the test ends,
    /// whether it passed or not.
    #[cfg(windows)]
    struct Throwaway {
        scheme: String,
    }

    #[cfg(windows)]
    impl Throwaway {
        fn new() -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self {
                scheme: format!("bitbraid-test-{}-{nanos}", std::process::id()),
            }
        }

        fn path(&self, under: &str) -> String {
            match under {
                "" => format!(r"{WINDOWS_CLASSES}\{}", self.scheme),
                _ => format!(r"{WINDOWS_CLASSES}\{}\{under}", self.scheme),
            }
        }
    }

    #[cfg(windows)]
    impl Drop for Throwaway {
        fn drop(&mut self) {
            let _ = windows_registry::CURRENT_USER.remove_tree(self.path(""));
        }
    }

    #[cfg(windows)]
    #[test]
    fn registering_on_windows_writes_the_key_of_the_scheme_in_place_of_what_was_there() {
        use windows_registry::{CLASSES_ROOT, CURRENT_USER};
        let key = Throwaway::new();
        // What another program could have left: a command, a
        // DelegateExecute that Windows would take over it, another verb.
        let older = CURRENT_USER
            .create(key.path(r"shell\open\command"))
            .unwrap();
        older
            .set_string("", r#""C:\older\program.exe" "%1""#)
            .unwrap();
        older
            .set_string("DelegateExecute", "{00000000-0000-0000-0000-000000000000}")
            .unwrap();
        drop(older);
        CURRENT_USER
            .create(key.path(r"shell\print\command"))
            .unwrap()
            .set_string("", "x")
            .unwrap();
        CURRENT_USER
            .create(key.path(""))
            .unwrap()
            .set_string("EditFlags", "2")
            .unwrap();

        let exe = std::env::current_exe().unwrap();
        let exe = exe.to_str().unwrap();
        let values = windows_values(exe).unwrap();
        let command = format!("\"{exe}\" \"%1\"");
        let text = |under: &str, name: &str| {
            CURRENT_USER
                .open(key.path(under))
                .unwrap()
                .get_string(name)
                .unwrap()
        };
        let keys = |under: &str| {
            let mut keys: Vec<String> = CURRENT_USER
                .open(key.path(under))
                .unwrap()
                .keys()
                .unwrap()
                .collect();
            keys.sort();
            keys
        };
        let names = |under: &str| {
            let mut names: Vec<String> = CURRENT_USER
                .open(key.path(under))
                .unwrap()
                .values()
                .unwrap()
                .map(|(name, _)| name)
                .collect();
            names.sort();
            names
        };
        // Written, and written again over itself.
        for _ in 0..2 {
            write_scheme_key(&key.scheme, &values).unwrap();
            assert_eq!(text("", ""), "URL:BitBraid link");
            assert_eq!(text("", "URL Protocol"), "");
            assert_eq!(text("DefaultIcon", ""), format!("{exe},0"));
            assert_eq!(text(r"shell\open\command", ""), command);
            // Nothing else, and nothing of the older one.
            assert_eq!(keys(""), ["DefaultIcon", "shell"]);
            assert_eq!(keys("shell"), ["open"]);
            assert_eq!(keys(r"shell\open"), ["command"]);
            assert!(keys(r"shell\open\command").is_empty());
            assert_eq!(names(""), ["", "URL Protocol"]);
            assert_eq!(names(r"shell\open\command"), [""]);
            // Where Windows looks for the key of a scheme it is seen too.
            let seen = CLASSES_ROOT
                .open(format!(r"{}\shell\open\command", key.scheme))
                .unwrap()
                .get_string("")
                .unwrap();
            assert_eq!(seen, command);
        }
        let path = key.path("");
        drop(key);
        assert!(CURRENT_USER.open(&path).is_err(), "{path} was left");
    }

    /// A test that fails half-way leaves no key behind either.
    #[cfg(windows)]
    #[test]
    fn registering_on_windows_leaves_no_key_after_a_test_that_fails() {
        let key = Throwaway::new();
        let path = key.path("");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let values = windows_values(r"C:\Program Files\BitBraid\bitbraid.exe").unwrap();
            write_scheme_key(&key.scheme, &values).unwrap();
            assert!(windows_registry::CURRENT_USER.open(key.path("")).is_ok());
            panic!("an assertion that fails, on purpose");
        }));
        assert!(outcome.is_err());
        assert!(
            windows_registry::CURRENT_USER.open(&path).is_err(),
            "{path} was left"
        );
    }
}
