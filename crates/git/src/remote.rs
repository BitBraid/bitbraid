//! Talking to remotes: fetch, pull, push, and the remotes themselves, all
//! through the git executable so that credentials, proxies and protocol
//! behave as in the user's shell. Progress is read from what git reports.
//!
//! Sources: git-fetch(1), git-pull(1), git-push(1), git-remote(1) (also
//! for what `remove` takes with it); the progress lines are described
//! under `--progress` in each. For putting a removed remote back:
//! git-for-each-ref(1) (`%(symref)`, `%00`), git-update-ref(1) (`--stdin`,
//! `create`), git-symbolic-ref(1), git-cat-file(1) (`--batch-check`). For
//! a certificate that is not accepted: git-config(1) (`http.sslCAInfo`,
//! `http.<url>.*`); what curl (with OpenSSL) and git-lfs (Go's TLS) then
//! write was found by running git 2.56 and git-lfs 3.8.0 against a server
//! with a certificate of an authority made for the test, and what GitLab
//! says when a push changes a file somebody else has locked with LFS by
//! running GitLab CE 19.4, and what github.com and git-lfs say to a token
//! that may read a repository and not write to it by running them against
//! github.com. For a proxy that turns git away: git-config(1)
//! (`http.proxy`, `http.proxyAuthMethod`, `--get-urlmatch`), git(1)
//! (`GIT_HTTP_PROXY_AUTHMETHOD`); what curl writes, and that git's own
//! way offers a proxy no Kerberos ticket, was found by running git 2.56
//! with curl 8.22 through Squid 6 with Basic, Negotiate and NTLM.

use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};

use bitbraid_strings::Message;

use crate::{Error, Oid, Repository, Result, exe, settings, settings::Setting};

/// What git says it is doing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Progress {
    /// The stage, for example `Receiving objects`.
    pub stage: String,
    /// How far along, when git gives a percentage.
    pub percent: Option<u8>,
}

/// Where an operation reports to and how it is stopped.
pub struct Reporter<'a> {
    /// Set to stop the operation.
    pub cancel: Arc<AtomicBool>,
    /// Called for each progress line.
    pub on_progress: &'a mut dyn FnMut(Progress),
}

impl std::fmt::Debug for Reporter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reporter").finish_non_exhaustive()
    }
}

/// Read one line of git's progress: `Receiving objects:  42% (4/9), …` and
/// `remote: Counting objects: 100% (5/5), done.` have a stage and a
/// percentage; `remote: Enumerating objects: 5, done.` a stage only.
/// Anything without a colon is not progress, and neither is a line whose
/// colon is not followed by a number: `Error while retrieving locks:
/// missing protocol` and `Warning: Permanently added …` look like a stage
/// and say something else.
pub fn parse_progress(line: &str) -> Option<Progress> {
    let line = line.trim();
    let line = line.strip_prefix("remote:").map(str::trim).unwrap_or(line);
    let (stage, rest) = line.split_once(':')?;
    let stage = stage.trim();
    // Stages are a few plain words starting with a capital ("Receiving
    // objects"); `fatal:`, `error:` and `hint:` are not, nor are URLs.
    if stage.len() > 40
        || !stage.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        || !stage.chars().all(|c| c.is_ascii_alphabetic() || c == ' ')
    {
        return None;
    }
    // What a stage is followed by is how far it is: a percentage or a
    // count. And a stage is never called a failure.
    let first_word = stage.split(' ').next().unwrap_or_default();
    if !rest.trim_start().starts_with(|c: char| c.is_ascii_digit())
        || ["Error", "Warning", "Fatal", "Hint", "Usage"].contains(&first_word)
    {
        return None;
    }
    let percent = rest
        .split('%')
        .next()
        .filter(|_| rest.contains('%'))
        .and_then(|before| before.trim().parse::<u8>().ok())
        .filter(|percent| *percent <= 100);
    Some(Progress {
        stage: stage.to_owned(),
        percent,
    })
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

/// Refuse what git could read as an option, and anything with control
/// characters.
fn check_plain(what: &'static str, text: &str) -> Result<()> {
    if text.is_empty() || text.starts_with('-') || text.chars().any(char::is_control) {
        return Err(invalid(
            what,
            say!("remote-cannot-be-used", text = format!("{text:?}")),
        ));
    }
    Ok(())
}

/// Check a remote's name: one path component that is a valid reference
/// name, and short enough to be a directory under `refs/remotes`. Git would
/// take a longer one into the configuration and fail on the first fetch.
pub fn check_remote_name(repo: &Repository, name: &str) -> Result<()> {
    check_plain(say!("remote-what-remote-name"), name)?;
    let reference = format!("refs/remotes/{name}/x");
    if name.contains('/')
        || name.len() > 100
        || exe::run(repo, ["check-ref-format", reference.as_str()], None).is_err()
    {
        return Err(invalid(
            say!("remote-what-remote-name"),
            say!("remote-not-valid-name", name = format!("{name:?}")),
        ));
    }
    Ok(())
}

fn run(repo: &Repository, args: &[&str], reporter: &mut Reporter<'_>) -> Result<String> {
    run_minding(repo, args, false, reporter)
}

/// [`run`] for a pull or a push, which run hooks of the repository
/// (`pre-push`, the hooks of the merge or rebase a pull ends in): one
/// that refuses is told apart from git and from the remote.
fn run_hooked(repo: &Repository, args: &[&str], reporter: &mut Reporter<'_>) -> Result<String> {
    run_minding(repo, args, true, reporter)
}

fn run_minding(
    repo: &Repository,
    args: &[&str],
    hooks: bool,
    reporter: &mut Reporter<'_>,
) -> Result<String> {
    let dir = repo.workdir().unwrap_or_else(|| repo.git_dir());
    let cancel = reporter.cancel.clone();
    let mut on_line = |line: &str| {
        if let Some(progress) = parse_progress(line) {
            (reporter.on_progress)(progress);
        }
    };
    let out = if hooks {
        exe::run_streaming_hooked(repo, args, &cancel, &mut on_line)
    } else {
        exe::run_streaming(repo, args, &cancel, &mut on_line)
    }
    .map_err(|err| explained_in(Some(dir), err))?;
    // What git says at the end is on stderr for these commands.
    Ok(format!("{}{}", out.stdout, out.stderr).trim().to_owned())
}

/// What a remote that would not let git in is known by, in what git and
/// ssh write, and what that means for whoever has to do something about
/// it. Their words stay; ours go before them.
///
/// ssh says "Permission denied" whatever the reason, and nothing at all
/// about a passphrase that did not open the key: it asks again, a few
/// times, and then goes on without that key.
const REFUSALS: &[(&str, Message)] = &[
    (
        "Host key verification failed",
        message!("remote-host-key-refused"),
    ),
    ("Permission denied (", message!("remote-key-refused")),
    ("Authentication failed for", CREDENTIALS_REFUSED),
    // Git-lfs, giving up after asking again for what the server refused
    // (3.8.0 does, against GitLab); it has helpers forget it as git does.
    ("too many authentication attempts", CREDENTIALS_REFUSED),
    (
        "could not read Username",
        message!("remote-user-name-unanswered"),
    ),
    (
        "could not read Password",
        message!("remote-password-unanswered"),
    ),
    // GitLab, refusing a push that changes a file somebody else holds a
    // lock on. Git-lfs only warns before such a push, unless
    // `lfs.<url>.locksverify` is set.
    ("is locked in Git LFS by", message!("remote-file-locked")),
    // An account that may not write, or a token that may do less than its
    // account: github.com's words for a push and for verifying or giving
    // up a lock (which it says also where the role may push and the token
    // may not), and git-lfs's for a 403 without words (github.com's
    // answer to a new lock), after the credentials' above.
    ("Write access to repository not granted", NOT_ALLOWED),
    ("You must have push access to", NOT_ALLOWED),
    ("Authorization error:", NOT_ALLOWED),
    // A certificate that is not accepted, as curl and git-lfs say it: the
    // particular reasons before the words that come with any of them.
    ("certificate has expired", CERTIFICATE_EXPIRED),
    ("certificate is not yet valid", CERTIFICATE_EXPIRED),
    ("x509: certificate is valid for", CERTIFICATE_NAME),
    ("x509: cannot validate certificate for", CERTIFICATE_NAME),
    (
        "no alternative certificate subject name matches",
        CERTIFICATE_NAME,
    ),
    ("does not match target host name", CERTIFICATE_NAME),
    ("SSL certificate problem", CERTIFICATE_UNTRUSTED),
    (
        "SSL certificate OpenSSL verify result",
        CERTIFICATE_UNTRUSTED,
    ),
    (
        "server certificate verification failed",
        CERTIFICATE_UNTRUSTED,
    ),
    ("tls: failed to verify certificate", CERTIFICATE_UNTRUSTED),
    (
        "x509: certificate signed by unknown authority",
        CERTIFICATE_UNTRUSTED,
    ),
    ("certificate is not trusted", CERTIFICATE_UNTRUSTED),
];

/// Credentials the server refused. A credential helper may have given them
/// without anyone being asked; git has it forget what was refused
/// (gitcredentials(7)).
const CREDENTIALS_REFUSED: Message = message!("remote-credentials-refused");

/// An account the server knows that may not do what was asked here: a
/// role that may only read, or a token that was given less than its
/// account may do (a fine-grained token at github.com that may read the
/// contents and not write them).
pub(crate) const NOT_ALLOWED: Message = message!("remote-not-allowed");

/// A server whose certificate no authority this system trusts signed.
const CERTIFICATE_UNTRUSTED: Message = message!("remote-certificate-untrusted");
/// A certificate out of its dates.
const CERTIFICATE_EXPIRED: Message = message!("remote-certificate-expired");
/// A certificate made out for another name.
const CERTIFICATE_NAME: Message = message!("remote-certificate-name");

/// What a proxy on the way that stopped git is known by, in what git (and
/// curl in it) writes, older curl's words beside newer ones. They are
/// looked for before [`REFUSALS`], and a proxy's 407 before the rest: it
/// is also a tunnel that failed. A proxy that turns away a plain
/// `http://` request answers in the server's place, and git's words for
/// that ("returned error: 403") do not say who it was, so only its 407 is
/// told apart. What curl wrote was found by running git 2.56 with curl
/// 8.22 through a proxy started by the tests.
const PROXY_REFUSALS: &[(&str, Message)] = &[
    ("returned error: 407", PROXY_LOGIN),
    ("response 407", PROXY_LOGIN),
    ("HTTP code 407 from proxy", PROXY_LOGIN),
    ("CONNECT tunnel failed", PROXY_TUNNEL),
    ("from proxy after CONNECT", PROXY_TUNNEL),
    ("Proxy CONNECT aborted", PROXY_TUNNEL),
    ("over proxy", PROXY_ABSENT),
    ("resolve proxy", PROXY_NAME),
];

/// A proxy that wants to know who is asking and was not told in a way it
/// takes, git signing in to proxies its own way (`anyauth`, Basic,
/// Digest). Git asks for a proxy's password only when the proxy's address
/// names the user, and has a credential helper forget a password the
/// proxy refused, as it does a server's (gitcredentials(7)). Its own way
/// offers no Kerberos ticket to a proxy whose address names no user, so a
/// proxy that takes Negotiate only turns git away until it is told to
/// (found against Squid with `negotiate_kerberos_auth`).
const PROXY_LOGIN: Message = message!("remote-proxy-login");
/// The same, git told to sign in with a Kerberos ticket: it had none,
/// none the proxy takes, or one for another name than the proxy's (curl
/// asks for a ticket for `HTTP/<the proxy's name as written>`, so a proxy
/// named by its address is asked for one nobody has).
const PROXY_TICKET: Message = message!("remote-proxy-ticket");
/// The same, git told to sign in with NTLM: it takes the password of the
/// user the proxy's address names, and a curl built with NTLM, which
/// builds of curl may leave out (then git sends no NTLM at all).
const PROXY_NTLM: Message = message!("remote-proxy-ntlm");

/// How long git is given to say how it signs in to a proxy.
const SETTING_PATIENCE: std::time::Duration = std::time::Duration::from_secs(5);

/// The address git names in what it said: `unable to access '<address>'`.
fn address_in(said: &str) -> Option<&str> {
    let (_, rest) = said.split_once("unable to access '")?;
    rest.split_once('\'').map(|(address, _)| address)
}

/// How git was told to sign in to a proxy on the way to the address in
/// `said`, in small letters: `GIT_HTTP_PROXY_AUTHMETHOD` as git's children
/// see it, which comes first for git, else the setting git takes for the
/// address (`http.proxyAuthMethod`, `http.<url>.proxyAuthMethod`) in the
/// repository at `dir`, or outside any.
fn proxy_sign_in(dir: Option<&Path>, said: &str) -> Option<String> {
    let set = bitbraid_core::process::var_os("GIT_HTTP_PROXY_AUTHMETHOD")
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.trim().is_empty());
    let method = match set {
        Some(method) => method,
        None => exe::url_setting(
            dir,
            "http.proxyAuthMethod",
            address_in(said)?,
            SETTING_PATIENCE,
        )?,
    };
    Some(method.trim().to_ascii_lowercase())
}

/// What a proxy's 407 means, by how git was told to sign in to it.
fn proxy_login_meaning(method: Option<&str>) -> Message {
    match method {
        Some("negotiate") => PROXY_TICKET,
        Some("ntlm") => PROXY_NTLM,
        _ => PROXY_LOGIN,
    }
}
/// A proxy that did not open the tunnel to the server.
const PROXY_TUNNEL: Message = message!("remote-proxy-tunnel");
/// A proxy that is not there.
const PROXY_ABSENT: Message = message!("remote-proxy-absent");
/// A proxy whose name stands for no address.
const PROXY_NAME: Message = message!("remote-proxy-name");

/// What a refusal by a remote, by the LFS server of one, or by a proxy on
/// the way, means, when it is one that is known.
pub(crate) fn meaning_of(said: &str) -> Option<&'static str> {
    known_meaning(said).map(|meaning| meaning.text())
}

/// [`meaning_of`], as the message it is.
fn known_meaning(said: &str) -> Option<Message> {
    PROXY_REFUSALS
        .iter()
        .chain(REFUSALS)
        .find(|(sign, _)| said.contains(sign))
        .map(|(_, meaning)| *meaning)
}

/// Say what a refusal by the remote, or by a proxy on the way to it,
/// means, when it is one that is known, and leave out the lines of git's
/// progress, under which it would be lost: the counting and the writing
/// that came before. A proxy's 407 is told by how git was told to sign in
/// to proxies outside any repository.
pub(crate) fn explained(err: Error) -> Error {
    explained_in(None, err)
}

/// [`explained`], for git run in the repository at `dir`, whose own
/// settings say how git signs in to a proxy.
pub(crate) fn explained_in(dir: Option<&Path>, err: Error) -> Error {
    match err {
        Error::Command {
            command,
            status,
            stderr,
        } => {
            let said: Vec<&str> = stderr
                .lines()
                .filter(|line| parse_progress(line).is_none())
                .collect();
            let said = match said.join("\n") {
                left if left.trim().is_empty() => stderr,
                left => left,
            };
            let already = [PROXY_LOGIN, PROXY_TICKET, PROXY_NTLM]
                .iter()
                .any(|meaning| said.starts_with(meaning.text()));
            let meaning = match known_meaning(&said) {
                Some(meaning) if meaning == PROXY_LOGIN && !already => {
                    Some(proxy_login_meaning(proxy_sign_in(dir, &said).as_deref()))
                }
                other => other,
            };
            let stderr = match meaning.map(|meaning| meaning.text()) {
                Some(meaning) if !already && !said.starts_with(meaning) => {
                    format!("{meaning}\n{said}")
                }
                _ => said,
            };
            Error::Command {
                command,
                status,
                stderr,
            }
        }
        other => other,
    }
}

/// Fetch from `remote`, or from every remote when absent.
pub fn fetch(
    repo: &Repository,
    remote: Option<&str>,
    prune: bool,
    tags: bool,
    reporter: &mut Reporter<'_>,
) -> Result<String> {
    fetch_with(repo, remote, prune, tags, false, reporter)
}

/// [`fetch`], and with `quiet` nobody is asked anything: a remote that
/// wants a user name, a password, a passphrase or a yes to a host key is
/// not fetched, and the error says so ([`Error::Refused`], which
/// [`WANTS_AN_ANSWER`] is part of). That is what fetching on a timer
/// needs, where a question would come out of nowhere. Credential helpers
/// and the ssh agent still answer, as they do without anyone looking.
pub fn fetch_with(
    repo: &Repository,
    remote: Option<&str>,
    prune: bool,
    tags: bool,
    quiet: bool,
    reporter: &mut Reporter<'_>,
) -> Result<String> {
    let mut args = vec!["fetch", "--progress"];
    if prune {
        args.push("--prune");
    }
    if tags {
        args.push("--tags");
    }
    match remote {
        Some(remote) => {
            check_remote_name(repo, remote)?;
            args.extend(["--", remote]);
        }
        None => args.push("--all"),
    }
    if !quiet {
        return run(repo, &args, reporter);
    }
    let cancel = reporter.cancel.clone();
    let mut on_line = |line: &str| {
        if let Some(progress) = parse_progress(line) {
            (reporter.on_progress)(progress);
        }
    };
    match exe::run_streaming_unasked(repo, &args, &cancel, &mut on_line) {
        Ok(out) => Ok(format!("{}{}", out.stdout, out.stderr).trim().to_owned()),
        Err(Error::Command { stderr, .. }) if wants_answer(&stderr) => {
            let asked = remotes_that_failed(&stderr);
            let who = match (remote, asked.as_slice()) {
                (Some(remote), _) => remote.to_owned(),
                (None, []) => say!("remote-who-a-remote").to_owned(),
                (None, names) => names.join(", "),
            };
            Err(Error::Refused {
                what: say!("remote-what-fetch"),
                detail: say!(
                    "remote-nobody-was-asked",
                    who,
                    wants = WANTS_AN_ANSWER.text()
                ),
            })
        }
        Err(err) => Err(err),
    }
}

/// Fetch one reference of `remote` (`refs/pull/7/head`) into `FETCH_HEAD`
/// only: no branch of the remote's, nor any of this repository's, moves.
/// Answers what git said and the commit that came.
pub fn fetch_reference(
    repo: &Repository,
    remote: &str,
    reference: &str,
    reporter: &mut Reporter<'_>,
) -> Result<(String, Oid)> {
    check_remote_name(repo, remote)?;
    check_plain(say!("remote-what-reference"), reference)?;
    let plain = reference.starts_with("refs/")
        && reference.len() <= 255
        && !reference.contains("..")
        && !reference.contains("//")
        && !reference.ends_with('/')
        && !reference.ends_with(".lock")
        && reference
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_' | b'.'));
    if !plain {
        return Err(invalid(
            say!("remote-what-reference"),
            say!(
                "remote-not-a-reference-to-fetch",
                reference = format!("{reference:?}")
            ),
        ));
    }
    let said = run(
        repo,
        &["fetch", "--progress", "--no-tags", "--", remote, reference],
        reporter,
    )?;
    let out = exe::run(
        repo,
        ["rev-parse", "--verify", "--quiet", "FETCH_HEAD^{commit}"],
        None,
    )?;
    let commit = out.stdout.trim().parse().map_err(|_| Error::Refused {
        what: say!("remote-what-fetch"),
        detail: say!("remote-brought-no-commit", reference, remote),
    })?;
    Ok((said, commit))
}

/// What the error of a quiet fetch says when a remote could only have
/// been fetched by asking the user something.
pub const WANTS_AN_ANSWER: Message = message!("remote-wants-an-answer");

/// Whether what git wrote on failing says that it, or ssh, had a question
/// and nobody to put it to.
pub fn wants_answer(stderr: &str) -> bool {
    const SIGNS: [&str; 8] = [
        "terminal prompts disabled",
        "unable to read askpass response",
        "could not read Username",
        "could not read Password",
        "Authentication failed",
        "Permission denied (",
        "Host key verification failed",
        "incorrect passphrase supplied",
    ];
    SIGNS.iter().any(|sign| stderr.contains(sign))
}

/// The remotes `git fetch --all` says it could not fetch.
fn remotes_that_failed(stderr: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for line in stderr.lines() {
        let name = line
            .trim()
            .strip_prefix("error: could not fetch ")
            .map(|name| name.trim().to_owned());
        if let Some(name) = name
            && !name.is_empty()
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names.truncate(8);
    names
}

/// How a pull brings the upstream's commits in.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PullMode {
    /// Only when the branch can move forward.
    FfOnly,
    /// Move forward when possible, merge otherwise.
    #[default]
    Ff,
    /// Replay local commits on top of the upstream.
    Rebase,
    /// Always make a merge commit.
    Merge,
}

/// What a pull did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pulled {
    /// What git said.
    pub output: String,
    /// The merge or rebase of the pull stopped half-way and is still in
    /// progress: what git said about that.
    pub stopped: Option<String>,
}

/// Pull the current branch from its upstream. Like the operations in
/// [`crate::rewrite`], a pull whose merge or rebase meets a conflict stops
/// there, in progress.
pub fn pull(repo: &Repository, mode: PullMode, reporter: &mut Reporter<'_>) -> Result<Pulled> {
    use crate::rewrite::{OnStop, ensure_idle, stopped_or_failed};
    ensure_idle(repo, say!("remote-what-pull"))?;
    let mut args = vec!["pull", "--progress"];
    args.extend(match mode {
        PullMode::FfOnly => ["--ff-only", "--no-rebase"],
        PullMode::Ff => ["--ff", "--no-rebase"],
        PullMode::Rebase => ["--rebase", "--no-ff"],
        PullMode::Merge => ["--no-ff", "--no-rebase"],
    });
    // A merge message is written without asking an editor that is not there.
    args.push("--no-edit");
    match run_hooked(repo, &args, reporter) {
        Ok(output) => Ok(Pulled {
            output,
            stopped: None,
        }),
        Err(err) => {
            let stopped = stopped_or_failed(repo, OnStop::Leave, say!("remote-what-pull"), err)?;
            Ok(Pulled {
                output: String::new(),
                stopped,
            })
        }
    }
}

/// How hard a push insists.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Force {
    /// Only fast-forwards.
    #[default]
    No,
    /// Overwrite, unless the remote moved since it was last fetched.
    Lease,
    /// Overwrite.
    Force,
}

/// What to push and how.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct PushOptions {
    /// Remote; git's default for the branch when absent.
    #[serde(default)]
    pub remote: Option<String>,
    /// Local branch; the current one when absent.
    #[serde(default)]
    pub branch: Option<String>,
    /// Make the pushed branch the upstream (first push of a branch).
    #[serde(default)]
    pub set_upstream: bool,
    /// How hard to insist.
    #[serde(default)]
    pub force: Force,
    /// Push tags as well.
    #[serde(default)]
    pub tags: bool,
    /// With [`Force::Lease`]: the commit the remote's branch has to be at
    /// for the push to overwrite it, which is what the user was shown
    /// would go. Absent, the lease is on where the remote-tracking branch
    /// is at the moment of the push, and something that fetched in
    /// between (a timer) would have moved that.
    #[serde(default)]
    pub expect: Option<Oid>,
    /// The branch on the remote to push to, when it is called something
    /// else than the local one.
    #[serde(default)]
    pub to: Option<String>,
    /// Without the `pre-push` hook (`git push --no-verify`).
    #[serde(default)]
    pub no_verify: bool,
}

/// The remote a push of `branch` goes to when none is named, as git
/// picks it: `branch.<name>.pushRemote`, `remote.pushDefault`, the remote
/// the branch follows, and then `origin` or the only remote there is.
pub fn push_remote(repo: &Repository, branch: &str) -> Result<String> {
    let own = [
        format!("branch.{branch}.pushRemote"),
        "remote.pushDefault".to_owned(),
        format!("branch.{branch}.remote"),
    ];
    for key in &own {
        // `.` is "this repository": a branch that follows a local one.
        if let Some(remote) = exe::config_get(repo, key).filter(|remote| remote != ".") {
            return Ok(remote);
        }
    }
    default_remote(repo)
}

/// What a push that overwrites would take away, as far as is known here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PushPreview {
    /// The local branch.
    pub branch: String,
    /// The remote it would go to.
    pub remote: String,
    /// Where the branch is on the remote, as last fetched; none when no
    /// branch of that name is known there.
    pub there: Option<Oid>,
    /// Commits the local branch has and the remote's does not.
    pub ahead: u32,
    /// Commits the remote's branch has and the local one does not: what a
    /// forced push takes away.
    pub behind: u32,
    /// The first of those, newest first: id and summary.
    pub lost: Vec<(Oid, String)>,
}

/// Most commits a [`PushPreview`] lists.
pub const PREVIEW_LISTED: usize = 20;

/// Say what pushing `branch` (the current one when absent) to `remote`
/// (its own when absent) by force would overwrite, from what was last
/// fetched. Nothing is asked of the remote.
pub fn push_preview(
    repo: &Repository,
    remote: Option<&str>,
    branch: Option<&str>,
) -> Result<PushPreview> {
    let branch = match branch {
        Some(branch) => branch.to_owned(),
        None => crate::branch::head_state(repo)?
            .branch
            .ok_or_else(|| invalid(say!("remote-what-push"), say!("remote-head-not-on-branch")))?,
    };
    crate::branch::check_branch_name(repo, &branch)?;
    let local = format!("refs/heads/{branch}");
    if crate::branch::resolve(repo, &local).is_none() {
        return Err(invalid(
            say!("remote-what-push"),
            say!("remote-no-branch", branch = branch.as_str()),
        ));
    }
    let remote = match remote {
        Some(remote) => remote.to_owned(),
        None => push_remote(repo, &branch)?,
    };
    check_remote_name(repo, &remote)?;
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let Some(there) = crate::branch::resolve(repo, &tracking) else {
        return Ok(PushPreview {
            branch,
            remote,
            there: None,
            ahead: 0,
            behind: 0,
            lost: Vec::new(),
        });
    };
    let range = format!("{local}...{tracking}");
    let counted = exe::run(
        repo,
        ["rev-list", "--count", "--left-right", range.as_str(), "--"],
        None,
    )?;
    let mut counts = counted
        .stdout
        .split_whitespace()
        .map(|count| count.parse::<u32>().unwrap_or(0));
    let (ahead, behind) = (counts.next().unwrap_or(0), counts.next().unwrap_or(0));
    let gone = format!("{local}..{tracking}");
    let limit = format!("--max-count={PREVIEW_LISTED}");
    let listed = exe::run(
        repo,
        [
            "log",
            "--format=%H%x00%s",
            limit.as_str(),
            gone.as_str(),
            "--",
        ],
        None,
    )?;
    let lost = listed
        .stdout
        .lines()
        .filter_map(|line| {
            let (id, summary) = line.split_once('\0')?;
            Some((id.parse().ok()?, summary.to_owned()))
        })
        .collect();
    Ok(PushPreview {
        branch,
        remote,
        there: Some(there),
        ahead,
        behind,
        lost,
    })
}

/// Push a branch.
pub fn push(
    repo: &Repository,
    options: &PushOptions,
    reporter: &mut Reporter<'_>,
) -> Result<String> {
    let mut args = vec!["push", "--progress"];
    let current;
    let branch = match &options.branch {
        Some(branch) => {
            crate::branch::check_branch_name(repo, branch)?;
            Some(branch.as_str())
        }
        None if options.remote.is_some()
            || options.set_upstream
            || options.expect.is_some()
            || options.to.is_some() =>
        {
            current = crate::branch::head_state(repo)?.branch;
            Some(current.as_deref().ok_or_else(|| {
                invalid(say!("remote-what-push"), say!("remote-head-not-on-branch"))
            })?)
        }
        None => None,
    };
    if let Some(to) = &options.to {
        crate::branch::check_branch_name(repo, to)?;
    }
    // What the branch is called on the remote.
    let there = options.to.as_deref().or(branch);
    let lease;
    match (options.force, options.expect, there) {
        (Force::No, _, _) => {}
        // A lease on the commit that was shown: the remote's branch goes
        // only if it is still that one.
        (Force::Lease, Some(expect), Some(there)) => {
            lease = format!("--force-with-lease=refs/heads/{there}:{}", expect.to_hex());
            args.push(lease.as_str());
        }
        (Force::Lease, _, _) => args.push("--force-with-lease"),
        (Force::Force, _, _) => args.push("--force"),
    }
    if options.tags {
        args.push("--tags");
    }
    if options.set_upstream {
        args.push("--set-upstream");
    }
    if options.no_verify {
        args.push("--no-verify");
    }
    let (remote, refspec);
    if let Some(branch) = branch {
        remote = match &options.remote {
            Some(remote) => remote.clone(),
            None => push_remote(repo, branch)?,
        };
        check_remote_name(repo, &remote)?;
        refspec = match &options.to {
            Some(to) => format!("refs/heads/{branch}:refs/heads/{to}"),
            None => branch.to_owned(),
        };
        args.extend(["--", remote.as_str(), refspec.as_str()]);
    }
    run_hooked(repo, &args, reporter)
}

/// The remote to use when none is named: `origin` when there is one, else
/// the only remote there is.
pub fn default_remote(repo: &Repository) -> Result<String> {
    let out = exe::run(repo, ["remote"], None)?;
    let names: Vec<&str> = out.stdout.lines().filter(|name| !name.is_empty()).collect();
    match names.as_slice() {
        [] => Err(invalid(
            say!("remote-what-remote"),
            say!("remote-no-remote"),
        )),
        [only] => Ok((*only).to_owned()),
        many if many.contains(&"origin") => Ok("origin".to_owned()),
        _ => Err(invalid(
            say!("remote-what-remote"),
            say!("remote-several-remotes"),
        )),
    }
}

/// Push one tag to `remote`, or every tag when none is named.
pub fn push_tag(
    repo: &Repository,
    remote: &str,
    tag: Option<&str>,
    reporter: &mut Reporter<'_>,
) -> Result<String> {
    check_remote_name(repo, remote)?;
    match tag {
        Some(name) => {
            crate::tag::check_tag_name(repo, name)?;
            let reference = format!("refs/tags/{name}");
            run(
                repo,
                &["push", "--progress", "--", remote, reference.as_str()],
                reporter,
            )
        }
        None => run(
            repo,
            &["push", "--progress", "--tags", "--", remote],
            reporter,
        ),
    }
}

/// Delete a tag on `remote`. The local tag stays.
pub fn delete_remote_tag(
    repo: &Repository,
    remote: &str,
    tag: &str,
    reporter: &mut Reporter<'_>,
) -> Result<String> {
    check_remote_name(repo, remote)?;
    crate::tag::check_tag_name(repo, tag)?;
    let reference = format!("refs/tags/{tag}");
    run(
        repo,
        &[
            "push",
            "--progress",
            "--delete",
            "--",
            remote,
            reference.as_str(),
        ],
        reporter,
    )
}

/// A branch that was deleted on a remote: what it takes to put it back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeletedRemoteBranch {
    /// The remote.
    pub remote: String,
    /// The branch there, without the remote's name.
    pub branch: String,
    /// The commit it was at.
    pub target: Oid,
}

/// What git says when the remote's branch is not where the lease expects.
fn lease_failed(said: &Error) -> bool {
    matches!(said, Error::Command { stderr, .. } if stderr.contains("stale info"))
}

/// Delete a branch on `remote`. The local branch of that name, if there
/// is one, stays.
///
/// It is deleted only where it was last fetched: the push carries a lease
/// on the commit the remote-tracking branch is at, so a branch somebody
/// pushed to in the meantime is refused rather than lost, and what was
/// deleted is exactly the commit that is known here. A branch that was
/// never fetched is refused for the same reason.
pub fn delete_remote_branch(
    repo: &Repository,
    remote: &str,
    branch: &str,
    reporter: &mut Reporter<'_>,
) -> Result<DeletedRemoteBranch> {
    check_remote_name(repo, remote)?;
    crate::branch::check_branch_name(repo, branch)?;
    let refuse = |detail: String| Error::Refused {
        what: say!("remote-what-delete-on-the-remote"),
        detail,
    };
    let tracking = format!("refs/remotes/{remote}/{branch}");
    let target = crate::branch::resolve(repo, &tracking)
        .ok_or_else(|| refuse(say!("remote-branch-not-known", remote, branch)))?;
    let reference = format!("refs/heads/{branch}");
    let lease = format!("--force-with-lease={reference}:{}", target.to_hex());
    let pushed = run_hooked(
        repo,
        &[
            "push",
            "--progress",
            "--delete",
            lease.as_str(),
            "--",
            remote,
            reference.as_str(),
        ],
        reporter,
    );
    match pushed {
        Ok(_) => Ok(DeletedRemoteBranch {
            remote: remote.to_owned(),
            branch: branch.to_owned(),
            target,
        }),
        Err(said) if lease_failed(&said) => {
            Err(refuse(say!("remote-branch-moved", remote, branch)))
        }
        Err(said) => Err(said),
    }
}

/// Put a branch back on `remote` at `target`, if the remote has no branch
/// of that name: what undoing [`delete_remote_branch`] does. A branch
/// that is there again, whoever made it, is left alone.
pub fn restore_remote_branch(
    repo: &Repository,
    deleted: &DeletedRemoteBranch,
    reporter: &mut Reporter<'_>,
) -> Result<String> {
    let DeletedRemoteBranch {
        remote,
        branch,
        target,
    } = deleted;
    check_remote_name(repo, remote)?;
    crate::branch::check_branch_name(repo, branch)?;
    let reference = format!("refs/heads/{branch}");
    // A lease on nothing: the branch must not exist there.
    let lease = format!("--force-with-lease={reference}:");
    let refspec = format!("{}:{reference}", target.to_hex());
    let pushed = run_hooked(
        repo,
        &[
            "push",
            "--progress",
            lease.as_str(),
            "--",
            remote.as_str(),
            refspec.as_str(),
        ],
        reporter,
    );
    match pushed {
        Err(said) if lease_failed(&said) => Err(Error::Refused {
            what: say!("remote-what-put-the-branch-back"),
            detail: say!("remote-branch-there-again", remote, branch),
        }),
        other => other,
    }
}

/// Add a remote.
pub fn remote_add(repo: &Repository, name: &str, url: &str) -> Result<()> {
    check_remote_name(repo, name)?;
    check_plain(say!("remote-what-remote-url"), url)?;
    exe::run(repo, ["remote", "add", "--", name, url], None)?;
    Ok(())
}

/// A reference that removing a remote took away.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DroppedRef {
    /// Its full name.
    pub name: String,
    /// The object it held; for a symbolic one, what it came to.
    pub object: Oid,
    /// What it pointed at, when it was symbolic (`refs/remotes/origin/HEAD`).
    pub symbolic: Option<String>,
}

/// What removing a remote took away: enough to put it back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RemovedRemote {
    /// The remote's name.
    pub name: String,
    /// The settings that went or changed: the remote's own (its URLs and
    /// fetch specifications, whatever else was set for it) and what
    /// branches that followed it lost.
    pub settings: Vec<Setting>,
    /// Its remote-tracking references.
    pub references: Vec<DroppedRef>,
}

/// Every reference there is, with what it holds.
fn all_references(repo: &Repository) -> Result<Vec<DroppedRef>> {
    let out = exe::run(
        repo,
        [
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(symref)",
        ],
        None,
    )?;
    Ok(out
        .stdout
        .lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let (name, object, symbolic) = (parts.next()?, parts.next()?, parts.next()?);
            Some(DroppedRef {
                name: name.to_owned(),
                object: object.parse().ok()?,
                symbolic: (!symbolic.is_empty()).then(|| symbolic.to_owned()),
            })
        })
        .collect())
}

/// Remove a remote with its remote-tracking branches and configuration.
/// Returns what went with it, with which [`remote_restore`] puts it back:
/// git says what it will take (the references its fetch specifications
/// name, the upstream of every branch that follows it), and what it took
/// is read off the repository before and after rather than worked out.
pub fn remote_remove(repo: &Repository, name: &str) -> Result<RemovedRemote> {
    check_remote_name(repo, name)?;
    let settings_before = settings::local(repo)?;
    let references_before = all_references(repo)?;
    exe::run(repo, ["remote", "remove", "--", name], None)?;
    let settings = settings::changed(&settings_before, &settings::local(repo)?);
    let left: std::collections::HashSet<String> = all_references(repo)?
        .into_iter()
        .map(|reference| reference.name)
        .collect();
    Ok(RemovedRemote {
        name: name.to_owned(),
        settings,
        references: references_before
            .into_iter()
            .filter(|reference| !left.contains(&reference.name))
            .collect(),
    })
}

/// A reference name out of a journal, before it is handed to git: under
/// `refs/`, and nothing git could read as anything else.
fn check_reference(name: &str) -> Result<()> {
    if !name.starts_with("refs/")
        || name.len() > 1024
        || name.chars().any(|ch| ch.is_control() || ch == ' ')
    {
        return Err(invalid(
            say!("remote-what-reference"),
            say!(
                "remote-not-a-reference-name",
                name = format!("{:?}", name.chars().take(80).collect::<String>())
            ),
        ));
    }
    Ok(())
}

/// Put a removed remote back: its settings, the upstream of the branches
/// that followed it, and its remote-tracking references where they were.
/// Refused, with nothing changed, when a setting or a reference of that
/// name is there again, or when what a reference held is gone.
pub fn remote_restore(repo: &Repository, removed: &RemovedRemote) -> Result<()> {
    let refuse = |detail: String| Error::Refused {
        what: say!("remote-what-remote"),
        detail,
    };
    check_remote_name(repo, &removed.name)?;
    for reference in &removed.references {
        check_reference(&reference.name)?;
        if let Some(target) = &reference.symbolic {
            check_reference(target)?;
        }
    }
    let there: std::collections::HashSet<String> = all_references(repo)?
        .into_iter()
        .map(|reference| reference.name)
        .collect();
    if let Some(taken) = removed
        .references
        .iter()
        .find(|reference| there.contains(&reference.name))
    {
        return Err(refuse(say!(
            "remote-reference-there-again",
            name = taken.name.as_str()
        )));
    }
    // Every object has to be here still: a reference to nothing is worse
    // than no reference.
    let asked: String = removed
        .references
        .iter()
        .map(|reference| format!("{}\n", reference.object.to_hex()))
        .collect();
    let answered = exe::run(repo, ["cat-file", "--batch-check"], Some(asked.as_bytes()))?;
    if let Some(gone) = answered
        .stdout
        .lines()
        .find(|line| line.ends_with(" missing"))
    {
        let object = gone.split(' ').next().unwrap_or(gone);
        let name = removed
            .references
            .iter()
            .find(|reference| reference.object.to_hex() == object)
            .map_or(object, |reference| reference.name.as_str());
        return Err(refuse(say!("remote-object-gone", name)));
    }
    settings::put(repo, &removed.settings, true)?;
    let created: String = removed
        .references
        .iter()
        .filter(|reference| reference.symbolic.is_none())
        .map(|reference| format!("create {} {}\n", reference.name, reference.object.to_hex()))
        .collect();
    let references =
        exe::run(repo, ["update-ref", "--stdin"], Some(created.as_bytes())).and_then(|_| {
            for reference in &removed.references {
                if let Some(target) = &reference.symbolic {
                    exe::run(
                        repo,
                        ["symbolic-ref", reference.name.as_str(), target.as_str()],
                        None,
                    )?;
                }
            }
            Ok(())
        });
    if let Err(err) = references {
        // Half done is worse than not done: the settings go again.
        let _ = settings::put(repo, &removed.settings, false);
        return Err(err);
    }
    Ok(())
}

/// Set the fetch URL of a remote, or its push URL.
pub fn remote_set_url(repo: &Repository, name: &str, url: &str, push: bool) -> Result<()> {
    check_remote_name(repo, name)?;
    check_plain(say!("remote-what-remote-url"), url)?;
    let mut args = vec!["remote", "set-url"];
    if push {
        args.push("--push");
    }
    args.extend(["--", name, url]);
    exe::run(repo, args, None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        CERTIFICATE_EXPIRED, CERTIFICATE_NAME, CERTIFICATE_UNTRUSTED, Progress, explained,
        parse_progress,
    };
    use crate::Error;

    fn progress(stage: &str, percent: Option<u8>) -> Option<Progress> {
        Some(Progress {
            stage: stage.to_owned(),
            percent,
        })
    }

    #[test]
    fn progress_lines_give_a_stage_and_a_percentage() {
        assert_eq!(
            parse_progress("Receiving objects:  42% (4/9), 1.20 MiB | 1.00 MiB/s"),
            progress("Receiving objects", Some(42))
        );
        assert_eq!(
            parse_progress("remote: Counting objects: 100% (5/5), done."),
            progress("Counting objects", Some(100))
        );
        assert_eq!(
            parse_progress("remote: Enumerating objects: 5, done."),
            progress("Enumerating objects", None)
        );
        assert_eq!(
            parse_progress("Resolving deltas:   0% (0/3)"),
            progress("Resolving deltas", Some(0))
        );
        // The other stages git and git-lfs have: a count, or a percentage.
        assert_eq!(
            parse_progress("Checking connectivity: 5, done."),
            progress("Checking connectivity", None)
        );
        assert_eq!(
            parse_progress("Updating files: 100% (3/3), done."),
            progress("Updating files", Some(100))
        );
        assert_eq!(
            parse_progress("Downloading LFS objects:  50% (1/2), 12 B | 0 B/s"),
            progress("Downloading LFS objects", Some(50))
        );
        assert_eq!(
            parse_progress("Expanding reachable commits in commit graph: 12, done."),
            None,
            "longer than a stage is"
        );
    }

    #[test]
    fn a_question_nobody_could_be_asked_is_told_from_other_failures() {
        use super::{remotes_that_failed, wants_answer};
        for said in [
            "fatal: could not read Username for 'http://127.0.0.1:1/x.git': terminal prompts disabled",
            "error: unable to read askpass response from 'false'\nfatal: could not read Password for 'https://ada@example.org': terminal prompts disabled",
            "fatal: Authentication failed for 'https://example.org/x.git/'",
            "git@example.org: Permission denied (publickey).\nfatal: Could not read from remote repository.",
            "Host key verification failed.\nfatal: Could not read from remote repository.",
        ] {
            assert!(wants_answer(said), "{said}");
        }
        for said in [
            "",
            "fatal: unable to access 'https://example.org/x.git/': Could not resolve host: example.org",
            "fatal: '/nowhere' does not appear to be a git repository",
            "error: cannot lock ref 'refs/remotes/origin/main': is at 1 but expected 2",
            "ssh: connect to host example.org port 22: Connection timed out",
        ] {
            assert!(!wants_answer(said), "{said}");
        }
        assert_eq!(
            remotes_that_failed(
                "Fetching origin\nFetching locked\nfatal: could not read Username\n\
                 error: could not fetch locked\nerror: could not fetch locked\n\
                 error: could not fetch other\n"
            ),
            ["locked", "other"]
        );
        assert!(remotes_that_failed("error: could not fetch \n").is_empty());
    }

    #[test]
    fn other_output_is_not_progress() {
        for line in [
            "",
            "From /tmp/remote",
            "   42f22b3..9a1c0de  main       -> origin/main",
            "To github.com:owner/repo.git",
            "fatal: unable to access 'https://example.org/x.git/': Could not resolve host",
            "error: failed to push some refs to 'origin'",
            "hint: Updates were rejected because the tip of your current branch is behind",
            "Percent: 250% of nothing",
            ":",
            "https://example.org/a:b",
            // A stage in looks, and a failure or a remark in fact: this is
            // what git-lfs says when the server cannot be asked.
            "Error while retrieving locks: missing protocol: \"\"",
            "Error downloading object: big.bin (3b18e51): Smudge error",
            "Warning: Permanently added 'example.org' (ED25519) to the list of known hosts.",
            "Error: 5 things went wrong",
            "Warning: 100% sure",
            "Fatal: 3",
            "Hint: 2 of them",
            "Usage: 1",
            "Server says: hello",
            "Receiving objects: soon",
            "Receiving objects:",
        ] {
            if line.starts_with("Percent") {
                continue;
            }
            assert_eq!(parse_progress(line), None, "{line:?}");
        }
        // A number that is no percentage is dropped, the stage kept.
        assert_eq!(
            parse_progress("Percent: 250% of nothing"),
            progress("Percent", None)
        );
    }

    fn failed(stderr: &str) -> Error {
        Error::Command {
            command: "git fetch --progress -- origin".to_owned(),
            status: "exit status: 128".to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    fn said(err: Error) -> String {
        match explained(err) {
            Error::Command { stderr, .. } => stderr,
            other => panic!("another kind of error came out: {other}"),
        }
    }

    #[test]
    fn a_refusal_by_the_remote_is_said_in_words_before_gits_own() {
        let theirs = "ada@example.invalid: Permission denied (publickey).\n\
                      fatal: Could not read from remote repository.";
        let ours = said(failed(theirs));
        assert!(
            ours.starts_with("The server did not accept the key."),
            "{ours}"
        );
        assert!(ours.contains("passphrase"), "{ours}");
        assert!(ours.ends_with(theirs), "{ours}");
        // Said once, however often it is explained.
        assert_eq!(said(failed(&ours)), ours);

        for (theirs, ours) in [
            (
                "Host key verification failed.\nfatal: Could not read",
                "The key of the host",
            ),
            (
                "git@host: Permission denied (publickey,password).",
                "The server did not accept",
            ),
            (
                "fatal: Authentication failed for 'https://example.invalid/r.git/'",
                "The server refused the user name",
            ),
            (
                "fatal: could not read Username for 'https://example.invalid': terminal prompts disabled",
                "The question for the user name",
            ),
            (
                "fatal: could not read Password for 'https://ada@example.invalid': terminal prompts disabled",
                "The question for the password",
            ),
            // Git-lfs 3.8.0, having asked again for what GitLab refused.
            (
                "Fetching reference refs/heads/main\n\
                 batch response: too many authentication attempts\n\
                 error: failed to fetch some objects from 'http://ada@127.0.0.1:8929/g/p.git/info/lfs'",
                "The server refused the user name",
            ),
        ] {
            let explained = said(failed(theirs));
            assert!(explained.starts_with(ours), "{explained}");
            assert!(explained.ends_with(theirs), "{explained}");
        }
    }

    /// What git 2.56 (curl with OpenSSL), curl with OpenSSL before 8.x and
    /// with GnuTLS, and git-lfs 3.8.0 wrote for a certificate that was not
    /// accepted, and what GitLab CE 19.4 wrote for a push of a file that
    /// somebody else holds a lock on.
    #[test]
    fn a_certificate_that_is_not_accepted_and_a_locked_file_are_said_in_words() {
        let untrusted = [
            "fatal: unable to access 'https://127.0.0.1:38109/g/p.git/': SSL certificate OpenSSL verify result: self-signed certificate in certificate chain (19)",
            "fatal: unable to access 'https://example.invalid/r.git/': SSL certificate problem: unable to get local issuer certificate",
            "fatal: unable to access 'https://example.invalid/r.git/': server certificate verification failed. CAfile: none CRLfile: none",
            "batch response: Post \"https://ada@127.0.0.1:38109/g/p.git/info/lfs/objects/batch\": tls: failed to verify certificate: x509: certificate signed by unknown authority",
        ];
        for theirs in untrusted {
            let explained = said(failed(theirs));
            assert!(
                explained.starts_with(CERTIFICATE_UNTRUSTED.text()),
                "{explained}"
            );
            assert!(explained.contains("http.sslCAInfo"), "{explained}");
            assert!(explained.ends_with(theirs), "{explained}");
            assert_eq!(said(failed(&explained)), explained);
        }
        for (theirs, ours) in [
            (
                "fatal: unable to access 'https://example.invalid/': SSL certificate problem: certificate has expired",
                CERTIFICATE_EXPIRED,
            ),
            (
                "Post \"https://example.invalid/info/lfs/locks\": tls: failed to verify certificate: x509: certificate has expired or is not yet valid: current time 2026-10-17T06:00:00Z is after 2026-01-01T00:00:00Z",
                CERTIFICATE_EXPIRED,
            ),
            (
                "Post \"https://10.0.0.1/info/lfs/locks\": tls: failed to verify certificate: x509: certificate is valid for example.invalid, not other.invalid",
                CERTIFICATE_NAME,
            ),
            (
                "Get \"https://127.0.0.1:1/x\": tls: failed to verify certificate: x509: cannot validate certificate for 127.0.0.1 because it doesn't contain any IP SANs",
                CERTIFICATE_NAME,
            ),
            (
                "fatal: unable to access 'https://other.invalid/': SSL: no alternative certificate subject name matches target host name 'other.invalid'",
                CERTIFICATE_NAME,
            ),
        ] {
            let explained = said(failed(theirs));
            assert!(explained.starts_with(ours.text()), "{explained}");
            assert!(explained.ends_with(theirs), "{explained}");
        }

        // GitLab's refusal of a push, without the counting and writing
        // that came before it.
        let pushed = "Locking support detected on remote \"origin\". Consider enabling it with:\n  \
             $ git config lfs.http://127.0.0.1:8929/g/p.git/info/lfs.locksverify true\n\
             Enumerating objects: 5, done.\n\
             Counting objects:  20% (1/5)\n\
             Counting objects: 100% (5/5), done.\n\
             Delta compression using up to 8 threads\n\
             Compressing objects: 100% (3/3), done.\n\
             Writing objects: 100% (3/3), 364 bytes | 364.00 KiB/s, done.\n\
             Total 3 (delta 1), reused 0 (delta 0), pack-reused 0 (from 0)\n\
             remote: GitLab: 'a.bin' is locked in Git LFS by @lfs-alice\n\
             To http://127.0.0.1:8929/g/p.git\n \
             ! [remote rejected] main -> topic (pre-receive hook declined)\n\
             error: failed to push some refs to 'http://127.0.0.1:8929/g/p.git'";
        let explained = said(failed(pushed));
        assert!(
            explained.starts_with(
                "The server refused the push: it changes a file that somebody else has locked"
            ),
            "{explained}"
        );
        assert!(
            explained.contains("remote: GitLab: 'a.bin' is locked in Git LFS by @lfs-alice"),
            "{explained}"
        );
        assert!(explained.contains("[remote rejected]"), "{explained}");
        for progress in [
            "Counting objects",
            "Writing objects",
            "Enumerating",
            "Compressing",
        ] {
            assert!(!explained.contains(progress), "{explained}");
        }
        // Nothing but progress: left as it is rather than emptied.
        let only = "Counting objects: 100% (5/5), done.";
        assert_eq!(said(failed(only)), only);
    }

    /// github.com refuses a push, a lock and an unlock to a token that may
    /// read and not write, whose account's role may push: git's and
    /// git-lfs's words alone ("You must have push access", to one whose
    /// role has it) would mislead.
    #[test]
    fn an_account_or_a_token_that_may_not_write_is_said_in_words_before_theirs() {
        use super::{CREDENTIALS_REFUSED, NOT_ALLOWED, meaning_of};
        // What git 2.56 and git-lfs 3.8.0 wrote, at github.com.
        for theirs in [
            "remote: Write access to repository not granted.\n\
             fatal: unable to access 'https://github.com/o/r.git/': The requested URL returned error: 403",
            "Authorization error: https://u@github.com/o/r.git/info/lfs/locks\n\
             Check that you have proper access to the repository",
            "You must have push access to delete a lock",
            "Error while retrieving locks: You must have push access to verify locks",
        ] {
            let explained = said(failed(theirs));
            assert!(explained.starts_with(NOT_ALLOWED.text()), "{explained}");
            assert!(explained.ends_with(theirs), "{explained}");
        }
        // Refused credentials keep their own words where both are there.
        let both = "Authorization error: https://u@h/o/r.git/info/lfs/locks\n\
                    batch response: too many authentication attempts";
        assert!(said(failed(both)).starts_with(CREDENTIALS_REFUSED.text()));
        // A role that may not force is the server's to say.
        let role = "You must have maintainer access to force delete a lock";
        assert_eq!(meaning_of(role), None);
    }

    #[test]
    fn a_proxy_that_stopped_git_is_said_in_words_before_gits_own() {
        use super::{PROXY_ABSENT, PROXY_LOGIN, PROXY_NAME, PROXY_TUNNEL};
        // What git 2.56 with curl 8.22 writes, and older curl's words.
        for (theirs, ours) in [
            (
                "fatal: unable to access 'http://h.example.invalid/r.git/': The requested URL returned error: 407",
                PROXY_LOGIN,
            ),
            (
                "fatal: unable to access 'https://h.example.invalid/r.git/': CONNECT tunnel failed, response 407",
                PROXY_LOGIN,
            ),
            (
                "fatal: unable to access 'https://h.example.invalid/r.git/': Received HTTP code 407 from proxy after CONNECT",
                PROXY_LOGIN,
            ),
            (
                "fatal: unable to access 'https://h.example.invalid/r.git/': CONNECT tunnel failed, response 403",
                PROXY_TUNNEL,
            ),
            (
                "fatal: unable to access 'https://h.example.invalid/r.git/': Received HTTP code 403 from proxy after CONNECT",
                PROXY_TUNNEL,
            ),
            (
                "fatal: unable to access 'https://127.0.0.1:9/r.git/': Proxy CONNECT aborted",
                PROXY_TUNNEL,
            ),
            (
                "fatal: unable to access 'https://h.example.invalid/r.git/': Failed to connect to h.example.invalid:443 over proxy 127.0.0.1 after 0 ms: Could not connect to server",
                PROXY_ABSENT,
            ),
            (
                "fatal: unable to access 'http://127.0.0.1:9/r.git/': Could not resolve proxy: proxy.invalid",
                PROXY_NAME,
            ),
            (
                "fatal: unable to access 'http://127.0.0.1:9/r.git/': Couldn't resolve proxy 'proxy.invalid'",
                PROXY_NAME,
            ),
        ] {
            let explained = said(failed(theirs));
            assert!(explained.starts_with(ours.text()), "{explained}");
            assert!(explained.ends_with(theirs), "{explained}");
            assert_eq!(said(failed(&explained)), explained, "said once");
        }
        // A server's own 403 is the server's: nothing is said of a proxy.
        let theirs = "fatal: unable to access 'http://h.example.invalid/r.git/': The requested URL returned error: 403";
        assert_eq!(said(failed(theirs)), theirs);
    }

    #[test]
    fn a_proxys_407_is_told_by_how_git_was_to_sign_in_to_it() {
        use super::{PROXY_LOGIN, PROXY_NTLM, PROXY_TICKET, address_in, proxy_login_meaning};
        assert_eq!(proxy_login_meaning(Some("negotiate")), PROXY_TICKET);
        assert_eq!(proxy_login_meaning(Some("ntlm")), PROXY_NTLM);
        for other in [
            None,
            Some("anyauth"),
            Some("basic"),
            Some("digest"),
            Some(""),
        ] {
            assert_eq!(proxy_login_meaning(other), PROXY_LOGIN, "{other:?}");
        }
        // Git's own way says that Kerberos has to be asked for.
        assert!(
            PROXY_LOGIN
                .text()
                .contains("http.proxyAuthMethod set to negotiate")
        );
        assert!(PROXY_TICKET.text().contains("kinit"));
        assert_eq!(
            address_in(
                "fatal: unable to access 'https://h.example.invalid/r.git/': CONNECT tunnel failed, response 407"
            ),
            Some("https://h.example.invalid/r.git/")
        );
        for nothing in ["", "unable to access '", "fatal: something else"] {
            assert_eq!(address_in(nothing), None, "{nothing:?}");
        }
        // Explained once, whichever of the three it was.
        for meaning in [PROXY_LOGIN, PROXY_TICKET, PROXY_NTLM] {
            let theirs = format!(
                "{meaning}\nfatal: unable to access 'http://h.example.invalid/r.git/': The requested URL returned error: 407"
            );
            assert_eq!(said(failed(&theirs)), theirs);
        }
    }

    #[test]
    fn other_failures_are_left_as_git_said_them() {
        for theirs in [
            "",
            "fatal: 'nowhere' does not appear to be a git repository",
            "error: cannot open '.git/FETCH_HEAD': Permission denied",
            " ! [rejected]        main -> main (non-fast-forward)",
            "fatal: unable to access 'https://example.invalid/': Could not resolve host",
        ] {
            assert_eq!(said(failed(theirs)), theirs);
        }
        // What is no failure of git's passes through untouched.
        let refused = Error::Refused {
            what: "push",
            detail: "Permission denied (publickey)".to_owned(),
        };
        assert!(matches!(explained(refused), Error::Refused { .. }));
    }
}
