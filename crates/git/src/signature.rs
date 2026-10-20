//! Signatures: how this repository signs, which keys could sign, what git
//! makes of the signature on a commit or a tag, and what went wrong when
//! signing failed.
//!
//! The verdict is always git's own (`%G?` and its companions for commits,
//! `git verify-tag --raw` for tags), so trust, the allowed-signers file and
//! the configured programs count as they do on the command line. Whether a
//! commit carries a signature at all is read from the object, with no
//! process, because the graph asks for every row. Signing that failed with
//! a key on a smart card is explained with what gpg says of the card
//! (`card`).
//!
//! Sources: git-config(1) (`gpg.format`, `gpg.program`,
//! `gpg.<format>.program`, `gpg.ssh.allowedSignersFile`,
//! `gpg.ssh.defaultKeyCommand`, `user.signingKey`, `commit.gpgSign`,
//! `tag.gpgSign`), git-log(1) (pretty formats `%G?`, `%GS`, `%GK`, `%GF`,
//! `%GP`, `%GT`, `%GG`), git-verify-tag(1), git-var(1),
//! gitformat-signature(5), gpg(1) (`--with-colons`, `--status-fd`) with
//! GnuPG's doc/DETAILS for the colon listing and the status lines,
//! gpgsm(1) and dirmngr(8) with the GnuPG manual's chapters on them
//! (certificate chains, `trustlist.txt`, revocation lists), ssh-keygen(1)
//! (`-Y`, `-l`), ssh-add(1); and what git 2.56, GnuPG 2.4 and OpenSSH 10
//! print when run.

use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    io::Read as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use gix::bstr::ByteSlice as _;
use serde::{Deserialize, Serialize};

use crate::{Error, Oid, Repository, Result, exe};

mod card;

/// Most commits one [`verify`] call takes: every signed one costs git a
/// process or two.
pub const VERIFY_LIMIT: usize = 256;
/// Most commits one [`signed`] call takes.
pub const SIGNED_LIMIT: usize = 10_000;
/// Longest value taken for `user.signingkey`, in bytes. A public key given
/// in full fits several times over.
const MAX_KEY: usize = 8192;
/// Most public key files listed from one directory.
const MAX_KEY_FILES: usize = 64;
/// Most entries of that directory looked at to find them.
const MAX_ENTRIES: usize = 4096;
/// Most of a public key file that is read, in bytes.
const MAX_KEY_FILE: u64 = 16 * 1024;
/// How deep tags of tags are followed.
const MAX_TAG_DEPTH: usize = 16;
/// How long gpg, ssh-keygen or ssh-add get to answer a question.
const ANSWER_WITHIN: Duration = Duration::from_secs(10);
/// How long a program's streams are read after it ended, when something
/// it started keeps them open.
const AFTER_END: Duration = Duration::from_millis(250);
/// Most that is read of such an answer, in bytes.
const MAX_ANSWER: u64 = 4 * 1024 * 1024;
/// Longest reason taken over from what a program wrote, in characters.
const MAX_REASON: usize = 300;

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

/// How a signature is made, by the name `gpg.format` has for it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    /// OpenPGP, made by gpg.
    Openpgp,
    /// An SSH key, with ssh-keygen.
    Ssh,
    /// An X.509 certificate, with gpgsm.
    X509,
}

impl Format {
    /// The value of `gpg.format`.
    pub fn name(self) -> &'static str {
        match self {
            Format::Openpgp => "openpgp",
            Format::Ssh => "ssh",
            Format::X509 => "x509",
        }
    }

    /// From a value of `gpg.format`, which git reads as written.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "openpgp" => Some(Format::Openpgp),
            "ssh" => Some(Format::Ssh),
            "x509" => Some(Format::X509),
            _ => None,
        }
    }

    /// From the first line of a signature: each format has its own.
    fn from_armor(line: &[u8]) -> Option<Self> {
        if line.starts_with(b"-----BEGIN PGP SIGNATURE-----")
            || line.starts_with(b"-----BEGIN PGP MESSAGE-----")
        {
            Some(Format::Openpgp)
        } else if line.starts_with(b"-----BEGIN SSH SIGNATURE-----") {
            Some(Format::Ssh)
        } else if line.starts_with(b"-----BEGIN SIGNED MESSAGE-----") {
            Some(Format::X509)
        } else {
            None
        }
    }
}

/// What a signature is worth.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum State {
    /// There is no signature.
    None,
    /// Good, by a key that is trusted.
    Good,
    /// Good, by a key nothing vouches for.
    Untrusted,
    /// Good, but the signature has expired.
    Expired,
    /// Good, made by a key that has expired since.
    KeyExpired,
    /// Good, made by a key that was revoked.
    KeyRevoked,
    /// It does not fit what it signs.
    Bad,
    /// The key it was made with is not known here.
    UnknownKey,
    /// There is a signature and nothing could check it; `reason` says why.
    CannotCheck,
}

impl State {
    /// Whether a key was found and the signature held against it, so that
    /// trust in that key means something.
    fn used_a_key(self) -> bool {
        !matches!(
            self,
            State::None | State::Bad | State::UnknownKey | State::CannotCheck
        )
    }
}

/// What gpgsm found wrong with the chain of an X.509 certificate whose
/// signature holds. Git grades every one of these as a key nothing vouches
/// for ("U"), and `git verify-commit` refuses them all; this says which it
/// was, since a revoked certificate is not the same as an unknown root.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChainFault {
    /// A certificate between the signer's and a root is not there: not in
    /// the store, and not carried in the signature.
    MissingIssuer,
    /// The chain ends at a root that is not among the trusted ones.
    UntrustedRoot,
    /// A certificate of the chain was revoked by its authority.
    Revoked,
    /// Whether a certificate of the chain was revoked could not be found
    /// out: no revocation list was to be had, or none that is current.
    RevocationUnknown,
    /// Something else, which `reason` gives in gpgsm's words.
    Other,
}

/// A signature as git judged it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Verdict {
    /// What it is worth.
    pub state: State,
    /// How it was made; absent without a signature, and for one in a form
    /// git does not know.
    pub format: Option<Format>,
    /// Who signed: a user id for OpenPGP, the principal for SSH.
    pub signer: Option<String>,
    /// The key: an id for OpenPGP, the fingerprint for SSH.
    pub key: Option<String>,
    /// Fingerprint of the key that signed.
    pub fingerprint: Option<String>,
    /// Fingerprint of the primary key, when a subkey signed.
    pub primary_fingerprint: Option<String>,
    /// How far the key is trusted: undefined, never, marginal, fully,
    /// ultimate.
    pub trust: Option<String>,
    /// Why it could not be checked; for an X.509 certificate whose chain
    /// does not hold, what gpgsm said of the chain.
    pub reason: Option<String>,
    /// For an X.509 signature that holds by a certificate whose chain does
    /// not: what is wrong with the chain. Always with the state
    /// [`State::Untrusted`], which is git's grade for it.
    pub chain: Option<ChainFault>,
}

impl Verdict {
    fn of(state: State) -> Self {
        Self {
            state,
            format: None,
            signer: None,
            key: None,
            fingerprint: None,
            primary_fingerprint: None,
            trust: None,
            reason: None,
            chain: None,
        }
    }

    fn unchecked(reason: impl Into<String>) -> Self {
        Self {
            reason: Some(reason.into()),
            ..Self::of(State::CannotCheck)
        }
    }
}

/// The verdict on one commit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitVerdict {
    /// The commit.
    pub id: Oid,
    /// What its signature is worth.
    #[serde(flatten)]
    pub verdict: Verdict,
}

/// Whether a commit carries a signature.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signed {
    /// The commit.
    pub id: Oid,
    /// Whether it has a signature.
    pub signed: bool,
    /// In which format; absent for a form git does not know.
    pub format: Option<Format>,
}

/// How this repository signs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signing {
    /// `gpg.format`; absent when it holds something git does not know.
    pub format: Option<Format>,
    /// `user.signingkey`.
    pub key: Option<String>,
    /// `commit.gpgsign`: commits are signed unless told otherwise.
    pub commits: bool,
    /// `tag.gpgsign`: annotated tags are signed unless told otherwise.
    pub tags: bool,
    /// `gpg.ssh.allowedSignersFile`, which SSH signatures are checked
    /// against.
    pub allowed_signers: Option<PathBuf>,
    /// The program that signs in this format.
    pub program: Option<String>,
    /// Whether signing can work at all.
    pub can_sign: bool,
    /// Why not.
    pub reason: Option<String>,
}

/// Where a key that could sign was found.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum KeySource {
    /// A secret key gpg knows.
    Gpg,
    /// A certificate gpgsm has the secret key of.
    Gpgsm,
    /// A public key file.
    File,
    /// A key only the SSH agent has.
    Agent,
}

/// A key that could sign.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Key {
    /// The format it signs in.
    pub format: Format,
    /// What `user.signingkey` has to hold to sign with it.
    pub key: String,
    /// Whose it is: the user id of a gpg key, the comment of an SSH key.
    pub name: String,
    pub fingerprint: Option<String>,
    /// The kind of key (`ed25519`, `rsa4096`, `ssh-ed25519`).
    pub kind: Option<String>,
    /// Where it was found.
    pub source: KeySource,
    /// The public key file, for one found in a file.
    pub path: Option<PathBuf>,
    /// Whether the SSH agent holds it.
    pub in_agent: bool,
    /// When it expires, in seconds since the Unix epoch.
    pub expires: Option<i64>,
    /// Whether it can sign now: not expired, revoked or disabled.
    pub usable: bool,
}

/// Where a setting is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The repository's own configuration.
    Repo,
    /// The user's, for every repository.
    User,
}

impl Scope {
    /// From its name: `repo` or `user`.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "repo" => Some(Scope::Repo),
            "user" => Some(Scope::User),
            _ => None,
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Scope::Repo => "--local",
            Scope::User => "--global",
        }
    }
}

/// What to change about signing. What is absent stays as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Settings {
    /// `gpg.format`.
    pub format: Option<Format>,
    /// `user.signingkey`; empty takes the setting away.
    pub key: Option<String>,
    /// `commit.gpgsign`.
    pub commits: Option<bool>,
    /// `tag.gpgsign`.
    pub tags: Option<bool>,
}

/// An annotated tag in full, or the little there is to a lightweight one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TagDetails {
    pub name: String,
    /// Whether it is a tag object rather than a bare reference.
    pub annotated: bool,
    /// What `refs/tags/<name>` holds: the tag object, or the commit.
    pub value: Oid,
    /// What it comes to once tags of tags are followed.
    pub target: Oid,
    /// What kind of object that is: commit, tree, blob.
    pub target_kind: String,
    /// Who made it and when; a lightweight tag has nobody.
    pub tagger: Option<crate::Signature>,
    /// Its message, without the signature.
    pub message: String,
    /// The tag's own signature.
    pub signature: Verdict,
}

// --- Programs other than git ------------------------------------------------

/// What a program other than git answered.
struct Answer {
    ok: bool,
    stdout: String,
    stderr: String,
}

/// Where git runs, and so what a relative program path is relative to.
fn base_dir(repo: &Repository) -> &Path {
    repo.workdir().unwrap_or_else(|| repo.git_dir())
}

/// What a child wrote to one stream, kept as it comes.
type Kept = Arc<Mutex<Vec<u8>>>;

/// Read one stream of a child on a thread, keeping what comes as it comes,
/// and say on `send` when its end is reached. None when that could not be
/// set up.
fn drain<R>(pipe: Option<R>, send: &mpsc::Sender<()>) -> Option<Kept>
where
    R: std::io::Read + Send + 'static,
{
    let pipe = pipe?;
    let kept: Kept = Arc::new(Mutex::new(Vec::new()));
    let (send, into) = (send.clone(), kept.clone());
    std::thread::Builder::new()
        .name("bitbraid-answer".to_owned())
        .spawn(move || {
            let mut pipe = pipe.take(MAX_ANSWER);
            let mut chunk = [0u8; 8192];
            loop {
                match pipe.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(count) => {
                        let read = chunk.get(..count).unwrap_or_default();
                        match into.lock() {
                            Ok(mut bytes) => bytes.extend_from_slice(read),
                            Err(poisoned) => poisoned.into_inner().extend_from_slice(read),
                        }
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
            let _ = send.send(());
        })
        .ok()?;
    Some(kept)
}

/// What was kept of a stream so far.
fn taken(kept: &Kept) -> String {
    let bytes = match kept.lock() {
        Ok(bytes) => bytes.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    };
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Ask gpg, ssh-keygen or ssh-add something. They talk to agents, and an
/// agent can hang: there is nothing to read from, no terminal to ask on,
/// and a limit on how long and how much the answer may be. None when the
/// program could not be started or did not answer in time.
fn ask<I, S>(repo: &Repository, program: &str, args: I) -> Option<Answer>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    ask_in(base_dir(repo), program, args)
}

/// [`ask`] in the folder `dir`.
fn ask_in<I, S>(dir: &Path, program: &str, args: I) -> Option<Answer>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut command = Command::new(program);
    bitbraid_core::process::no_window(&mut command);
    bitbraid_core::process::with_user_environment(&mut command);
    let mut child = command
        .args(args)
        .current_dir(dir)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let give_up = |mut child: std::process::Child| {
        let _ = child.kill();
        let _ = child.wait();
    };
    let (send, receive) = mpsc::channel();
    let (Some(stdout), Some(stderr)) = (
        drain(child.stdout.take(), &send),
        drain(child.stderr.take(), &send),
    ) else {
        give_up(child);
        return None;
    };
    let started = Instant::now();
    // Both streams to their end, and the program's: a program that has
    // said everything closes them. A daemon it started may hold them open
    // all the same (on Windows gpg's agent and keybox daemon keep every
    // handle gpg had), so once the program has ended what came within a
    // moment is all there is.
    let (mut open, mut ended) = (2_u8, None);
    let status = loop {
        match receive.recv_timeout(Duration::from_millis(20)) {
            Ok(()) => open = open.saturating_sub(1),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => open = 0,
        }
        if ended.is_none() {
            match child.try_wait() {
                Ok(Some(status)) => ended = Some((status, Instant::now())),
                Ok(None) => {}
                Err(_) => {
                    give_up(child);
                    return None;
                }
            }
        }
        if let Some((status, at)) = ended
            && (open == 0 || at.elapsed() >= AFTER_END)
        {
            break status;
        }
        if started.elapsed() >= ANSWER_WITHIN {
            give_up(child);
            return None;
        }
    };
    Some(Answer {
        ok: status.success(),
        stdout: taken(&stdout),
        stderr: taken(&stderr),
    })
}

/// Whether a program can be started: a path that names a file, or a name
/// found in a directory of `PATH`.
fn installed(repo: &Repository, program: &str) -> bool {
    if program.is_empty() {
        return false;
    }
    let path = Path::new(program);
    if path.components().count() > 1 {
        return base_dir(repo).join(path).is_file();
    }
    bitbraid_core::process::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            let candidate = dir.join(program);
            candidate.is_file() || (cfg!(windows) && candidate.with_extension("exe").is_file())
        })
    })
}

// --- Configuration ----------------------------------------------------------

/// A configuration value read as a type (`--type=bool`, `--type=path`),
/// from all scopes.
fn config_typed(repo: &Repository, kind: &str, key: &str) -> Option<String> {
    exe::run(repo, ["config", kind, "--get", key], None)
        .ok()
        .map(|out| out.stdout.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn config_bool(repo: &Repository, key: &str) -> bool {
    config_typed(repo, "--type=bool", key).as_deref() == Some("true")
}

fn config_path(repo: &Repository, key: &str) -> Option<String> {
    config_typed(repo, "--type=path", key)
}

/// The program that signs and verifies in a format.
fn program(repo: &Repository, format: Format) -> String {
    match format {
        // Two names for one setting; as with any setting, the one read
        // last counts.
        Format::Openpgp => exe::run(
            repo,
            [
                "config",
                "--type=path",
                "--get-regexp",
                r"^gpg\.(openpgp\.)?program$",
            ],
            None,
        )
        .ok()
        .and_then(|out| {
            out.stdout.lines().rev().find_map(|line| {
                line.split_once(' ')
                    .map(|(_, value)| value.trim().to_owned())
                    .filter(|value| !value.is_empty())
            })
        })
        .unwrap_or_else(|| "gpg".to_owned()),
        Format::Ssh => {
            config_path(repo, "gpg.ssh.program").unwrap_or_else(|| "ssh-keygen".to_owned())
        }
        Format::X509 => config_path(repo, "gpg.x509.program").unwrap_or_else(|| "gpgsm".to_owned()),
    }
}

/// Whom git signs as when no key is set: the committer, without the date.
fn committer(repo: &Repository) -> Option<String> {
    let out = exe::run(repo, ["var", "GIT_COMMITTER_IDENT"], None).ok()?;
    let line = out.stdout.trim();
    let end = line.rfind('>')?;
    line.get(..=end).map(str::to_owned)
}

/// How this repository signs, and whether that can work.
pub fn status(repo: &Repository) -> Signing {
    let written = exe::config_get(repo, "gpg.format");
    let format = match written.as_deref() {
        None => Some(Format::Openpgp),
        Some(text) => Format::parse(text),
    };
    let key = exe::config_get(repo, "user.signingkey");
    let (program, reason) = match format {
        Some(format) => {
            let program = program(repo, format);
            let reason = obstacle(repo, format, &program, key.as_deref());
            (Some(program), reason)
        }
        None => (
            None,
            Some(say!(
                "signature-format-unknown",
                value = format!("{:?}", written.unwrap_or_default())
            )),
        ),
    };
    Signing {
        format,
        key,
        commits: config_bool(repo, "commit.gpgsign"),
        tags: config_bool(repo, "tag.gpgsign"),
        allowed_signers: config_path(repo, "gpg.ssh.allowedSignersFile").map(PathBuf::from),
        program,
        can_sign: reason.is_none(),
        reason,
    }
}

/// What stands in the way of signing, if anything does. Only what is
/// certain counts: a key that needs a passphrase nobody can type shows
/// when signing is tried, and a signing program that is neither gpg nor
/// ssh-keygen keeps its keys where nothing here can look.
fn obstacle(repo: &Repository, format: Format, program: &str, key: Option<&str>) -> Option<String> {
    if !installed(repo, program) {
        return Some(say!(
            "signature-signer-not-found",
            program,
            format = format.name()
        ));
    }
    match format {
        Format::Ssh => ssh_obstacle(repo, program, key),
        Format::Openpgp | Format::X509 => gpg_obstacle(repo, format, program, key),
    }
}

/// Whether the program is GnuPG's gpgsm by its own word, under whatever
/// name or wrapper it is started.
fn is_gpgsm(repo: &Repository, program: &str) -> bool {
    ask(repo, program, ["--version"])
        .is_some_and(|answer| answer.ok && answer.stdout.starts_with("gpgsm (GnuPG)"))
}

fn gpg_obstacle(
    repo: &Repository,
    format: Format,
    program: &str,
    key: Option<&str>,
) -> Option<String> {
    let Some(who) = key.map(str::to_owned).or_else(|| committer(repo)) else {
        return Some(say!("signature-no-key-no-name").to_owned());
    };
    let listing = ask(
        repo,
        program,
        [
            "--batch",
            "--no-tty",
            "--with-colons",
            "--list-secret-keys",
            "--",
            who.as_str(),
        ],
    );
    let Some(listing) = listing else {
        return Some(say!("signature-program-silent", program));
    };
    let keys = secret_keys(&listing.stdout);
    if keys.iter().any(|key| key.usable) {
        return None;
    }
    if let Some(found) = keys.first() {
        return Some(say!("signature-key-unusable", key = found.id.as_str()));
    }
    // No key listed. That is only gpg's word when gpg says so: a program
    // that lists nothing because it is not gpg may sign all the same.
    // Gpgsm says it by listing nothing and ending well, which is taken
    // from gpgsm alone.
    let said_so = if listing.ok {
        format == Format::X509 && is_gpgsm(repo, program)
    } else {
        listing.stderr.contains("No secret key")
    };
    if !said_so {
        return None;
    }
    Some(match key {
        Some(_) => say!(
            "signature-no-secret-key-for-set",
            program,
            who = format!("{who:?}")
        ),
        None => say!(
            "signature-no-key-no-secret-key",
            program,
            who = format!("{who:?}")
        ),
    })
}

fn ssh_obstacle(repo: &Repository, program: &str, key: Option<&str>) -> Option<String> {
    let Some(key) = key else {
        // Without a key git asks this command for one.
        return exe::config_get(repo, "gpg.ssh.defaultKeyCommand")
            .is_none()
            .then(|| say!("signature-no-key").to_owned());
    };
    // Where the private half has to be is ssh-keygen's rule; another
    // program has its own.
    if Path::new(program).file_stem() != Some(OsStr::new("ssh-keygen")) {
        return None;
    }
    if is_literal(key) {
        let Some(public) = literal_key(key) else {
            return Some(say!("signature-key-not-public").to_owned());
        };
        // Given in full, the private half is the agent's to have.
        return (!agent_holds(&agent_keys(repo), &public))
            .then(|| say!("signature-agent-lacks-literal-key").to_owned());
    }
    let path = config_path(repo, "user.signingkey").unwrap_or_else(|| key.to_owned());
    let path = base_dir(repo).join(path);
    if !path.is_file() {
        return Some(say!(
            "signature-key-file-missing",
            path = path.display().to_string()
        ));
    }
    // A public key file: the private half lies beside it or is the agent's.
    if path.extension().is_some_and(|extension| extension == "pub")
        && !path.with_extension("").is_file()
    {
        let held =
            read_key_file(&path).is_some_and(|public| agent_holds(&agent_keys(repo), &public));
        if !held {
            return Some(say!(
                "signature-no-private-key",
                path = path.display().to_string()
            ));
        }
    }
    None
}

// --- Keys -------------------------------------------------------------------

/// One secret key of a `--with-colons` listing.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SecretKey {
    id: String,
    fingerprint: Option<String>,
    user: Option<String>,
    kind: Option<String>,
    expires: Option<i64>,
    usable: bool,
}

/// Read a `--list-secret-keys --with-colons` listing: a `sec` record (`crs`
/// from gpgsm) opens a key, the `fpr` and `uid` records after it are its
/// own, and those after an `ssb` belong to a subkey.
fn secret_keys(listing: &str) -> Vec<SecretKey> {
    let mut keys: Vec<SecretKey> = Vec::new();
    let mut primary = false;
    for line in listing.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        let field = |n: usize| fields.get(n).copied().unwrap_or("");
        match field(0) {
            "sec" | "crs" => {
                let (validity, capabilities) = (field(1), field(11));
                // Capital letters say what the key as a whole can do; gpgsm
                // may leave the field empty.
                let signs = capabilities.is_empty() || capabilities.contains('S');
                keys.push(SecretKey {
                    id: field(4).to_owned(),
                    fingerprint: None,
                    user: None,
                    kind: key_kind(field(3), field(2), field(16)),
                    expires: expiry(field(6)),
                    usable: signs
                        && !capabilities.contains('D')
                        && !matches!(validity, "e" | "r" | "d" | "i"),
                });
                primary = true;
            }
            "ssb" | "sub" => primary = false,
            "fpr" if primary => {
                if let Some(key) = keys.last_mut()
                    && key.fingerprint.is_none()
                    && !field(9).is_empty()
                {
                    key.fingerprint = Some(field(9).to_owned());
                }
            }
            "uid" => {
                if let Some(key) = keys.last_mut()
                    && key.user.is_none()
                {
                    key.user = Some(unescaped(field(9)));
                }
            }
            _ => {}
        }
    }
    keys.retain(|key| !key.id.is_empty());
    keys
}

/// When a key expires, in seconds since the Unix epoch: gpg writes the
/// seconds, gpgsm the date and time as `20301231T235959` (in UTC).
fn expiry(field: &str) -> Option<i64> {
    if let Ok(seconds) = field.parse() {
        return Some(seconds);
    }
    let at = jiff::civil::DateTime::strptime("%Y%m%dT%H%M%S", field).ok()?;
    Some(
        at.to_zoned(jiff::tz::TimeZone::UTC)
            .ok()?
            .timestamp()
            .as_second(),
    )
}

/// A name for the kind of key: its curve, or the algorithm and size.
fn key_kind(algorithm: &str, bits: &str, curve: &str) -> Option<String> {
    if !curve.is_empty() {
        return Some(curve.to_owned());
    }
    match algorithm {
        "1" | "2" | "3" => Some(format!("rsa{bits}")),
        "17" => Some(format!("dsa{bits}")),
        _ => None,
    }
}

/// A user id out of a colon listing, where a colon and anything unprintable
/// is written `\xNN`.
fn unescaped(text: &str) -> String {
    let raw = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(raw.len());
    let mut at = 0usize;
    while let Some(byte) = raw.get(at).copied() {
        let digits = text.get(at.saturating_add(2)..at.saturating_add(4));
        if byte == b'\\'
            && raw.get(at.saturating_add(1)) == Some(&b'x')
            && let Some(value) = digits.and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(value);
            at = at.saturating_add(4);
        } else {
            out.push(byte);
            at = at.saturating_add(1);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A public key as ssh writes it: its type, the key, and a comment.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicKey {
    kind: String,
    blob: String,
    comment: String,
}

fn public_key(line: &str) -> Option<PublicKey> {
    let mut words = line.split_whitespace();
    let (kind, blob) = (words.next()?, words.next()?);
    let known = ["ssh-", "ecdsa-", "sk-"]
        .iter()
        .any(|prefix| kind.starts_with(prefix));
    let plain = |text: &str, also: &[u8]| {
        text.bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || also.contains(&byte))
    };
    if !known || !plain(kind, b"-@._") || !plain(blob, b"+/=") {
        return None;
    }
    Some(PublicKey {
        kind: kind.to_owned(),
        blob: blob.to_owned(),
        comment: words.collect::<Vec<_>>().join(" "),
    })
}

/// Whether `user.signingkey` holds a public key in full rather than the
/// path of a file: `key::<key>`, or the older bare `ssh-...`.
fn is_literal(value: &str) -> bool {
    value.starts_with("key::") || value.starts_with("ssh-")
}

fn literal_key(value: &str) -> Option<PublicKey> {
    public_key(value.strip_prefix("key::").unwrap_or(value))
}

/// The fingerprint in a line of `ssh-keygen -l` or `ssh-add -l`:
/// `256 SHA256:... comment (ED25519)`.
fn fingerprint_of(line: &str) -> Option<String> {
    line.split_whitespace()
        .nth(1)
        .filter(|word| word.contains(':'))
        .map(str::to_owned)
}

/// The `ssh-add` that talks to the agent git's `ssh-keygen` signs with:
/// the one beside `program` when that is a path and there is one, else
/// the one on `PATH`. On Windows two agents are common, Windows' own
/// behind a named pipe and the one of Git for Windows behind a socket,
/// and each one's `ssh-add` reaches its own alone.
fn ssh_add_for(program: &str) -> String {
    let path = Path::new(program);
    let name = format!("ssh-add{}", std::env::consts::EXE_SUFFIX);
    path.parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .map(|dir| dir.join(&name))
        .filter(|beside| beside.is_file())
        .and_then(|beside| beside.to_str().map(str::to_owned))
        .unwrap_or_else(|| "ssh-add".to_owned())
}

/// The keys the SSH agent holds, each with its fingerprint when the agent
/// lists as many of those. Empty without an agent. On Unix an agent says
/// where it is in `SSH_AUTH_SOCK`; the agent of Windows is found by
/// `ssh-add` itself, at a pipe of a fixed name, and says so when it is not
/// running. The agent asked is the one of git's `ssh-keygen`
/// ([`ssh_add_for`]).
fn agent_keys(repo: &Repository) -> Vec<(PublicKey, Option<String>)> {
    if cfg!(unix) && std::env::var_os("SSH_AUTH_SOCK").is_none_or(|socket| socket.is_empty()) {
        return Vec::new();
    }
    let ssh_add = ssh_add_for(&program(repo, Format::Ssh));
    let Some(listed) = ask(repo, &ssh_add, ["-L"]).filter(|answer| answer.ok) else {
        return Vec::new();
    };
    let keys: Vec<PublicKey> = listed.stdout.lines().filter_map(public_key).collect();
    let prints: Vec<String> = ask(repo, &ssh_add, ["-l"])
        .filter(|answer| answer.ok)
        .map(|answer| answer.stdout.lines().filter_map(fingerprint_of).collect())
        .unwrap_or_default();
    let aligned = prints.len() == keys.len();
    keys.into_iter()
        .enumerate()
        .map(|(index, key)| {
            let print = prints.get(index).filter(|_| aligned).cloned();
            (key, print)
        })
        .collect()
}

fn agent_holds(agent: &[(PublicKey, Option<String>)], key: &PublicKey) -> bool {
    agent.iter().any(|(held, _)| held.blob == key.blob)
}

/// The public key in a file, if it is one. Only regular files are opened,
/// and only their beginning is read.
fn read_key_file(path: &Path) -> Option<PublicKey> {
    if !path.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_KEY_FILE)
        .read_to_end(&mut bytes)
        .ok()?;
    String::from_utf8_lossy(&bytes).lines().find_map(public_key)
}

/// The public keys in a directory: `*.pub`, certificates left out, by name.
fn key_files(
    repo: &Repository,
    dir: &Path,
    agent: &[(PublicKey, Option<String>)],
) -> Vec<(PublicKey, Key)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths: Vec<PathBuf> = entries
        .take(MAX_ENTRIES)
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.file_name().is_some_and(|name| {
                let name = name.to_string_lossy();
                name.ends_with(".pub") && !name.ends_with("-cert.pub")
            })
        })
        .collect();
    paths.sort();
    paths.truncate(MAX_KEY_FILES);
    let mut out = Vec::new();
    for path in paths {
        let Some(public) = read_key_file(&path) else {
            continue;
        };
        // ssh-keygen knows how a fingerprint is made; without it there is
        // none to show.
        let fingerprint = ask(
            repo,
            "ssh-keygen",
            [OsStr::new("-l"), OsStr::new("-f"), path.as_os_str()],
        )
        .filter(|answer| answer.ok)
        .and_then(|answer| answer.stdout.lines().find_map(fingerprint_of));
        let key = Key {
            format: Format::Ssh,
            key: path.to_string_lossy().into_owned(),
            name: public.comment.clone(),
            fingerprint,
            kind: Some(public.kind.clone()),
            source: KeySource::File,
            in_agent: agent_holds(agent, &public),
            path: Some(path),
            expires: None,
            usable: true,
        };
        out.push((public, key));
    }
    out
}

/// Keys that could sign: the secret keys gpg knows, the certificates gpgsm
/// has the secret keys of, the public keys in `ssh_dir`, and with `agent`
/// those the SSH agent holds. A key that is in a file and in the agent is
/// listed once, as the file.
pub fn keys(repo: &Repository, ssh_dir: Option<&Path>, agent: bool) -> Vec<Key> {
    let mut out: Vec<Key> = Vec::new();
    for (format, source) in [
        (Format::Openpgp, KeySource::Gpg),
        (Format::X509, KeySource::Gpgsm),
    ] {
        let listing = ask(
            repo,
            &program(repo, format),
            ["--batch", "--no-tty", "--with-colons", "--list-secret-keys"],
        );
        out.extend(
            listing
                .map(|answer| secret_keys(&answer.stdout))
                .unwrap_or_default()
                .into_iter()
                .map(|key| Key {
                    format,
                    key: key.fingerprint.clone().unwrap_or_else(|| key.id.clone()),
                    name: key.user.unwrap_or_default(),
                    fingerprint: key.fingerprint,
                    kind: key.kind,
                    source,
                    path: None,
                    in_agent: false,
                    expires: key.expires,
                    usable: key.usable,
                }),
        );
    }
    let held = if agent { agent_keys(repo) } else { Vec::new() };
    let files = ssh_dir
        .map(|dir| key_files(repo, dir, &held))
        .unwrap_or_default();
    out.extend(files.iter().map(|(_, key)| key.clone()));
    for (public, fingerprint) in &held {
        if files.iter().any(|(file, _)| file.blob == public.blob) {
            continue;
        }
        out.push(Key {
            format: Format::Ssh,
            key: format!("key::{} {}", public.kind, public.blob),
            name: public.comment.clone(),
            fingerprint: fingerprint.clone(),
            kind: Some(public.kind.clone()),
            source: KeySource::Agent,
            path: None,
            in_agent: true,
            expires: None,
            usable: true,
        });
    }
    out
}

// --- Settings ---------------------------------------------------------------

/// Write signing settings to the repository's configuration or the user's.
/// Everything is checked before anything is written, and when a write fails
/// those before it are taken back.
pub fn configure(repo: &Repository, settings: &Settings, scope: Scope) -> Result<()> {
    let mut changes: Vec<(&'static str, Option<String>)> = Vec::new();
    if let Some(format) = settings.format {
        changes.push(("gpg.format", Some(format.name().to_owned())));
    }
    if let Some(given) = settings.key.as_deref() {
        // As it will be written: nothing git could read as an option.
        let key = given.trim();
        if given.len() > MAX_KEY || key.starts_with('-') || given.chars().any(char::is_control) {
            return Err(invalid(
                say!("signature-what-signing-key"),
                say!("signature-not-a-key"),
            ));
        }
        changes.push(("user.signingkey", (!key.is_empty()).then(|| key.to_owned())));
    }
    if let Some(on) = settings.commits {
        changes.push(("commit.gpgsign", Some(on.to_string())));
    }
    if let Some(on) = settings.tags {
        changes.push(("tag.gpgsign", Some(on.to_string())));
    }
    if changes.is_empty() {
        return Err(invalid(
            say!("signature-what-signing-settings"),
            say!("signature-nothing-to-change"),
        ));
    }
    let flag = scope.flag();
    let before: Vec<Option<String>> = changes
        .iter()
        .map(|(key, _)| {
            exe::run(repo, ["config", flag, "--get", *key], None)
                .ok()
                .map(|out| out.stdout.trim_end_matches('\n').to_owned())
        })
        .collect();
    for (done, (key, value)) in changes.iter().enumerate() {
        if let Err(err) = write_setting(repo, flag, key, value.as_deref()) {
            for ((key, _), old) in changes.iter().zip(&before).take(done) {
                let _ = write_setting(repo, flag, key, old.as_deref());
            }
            return Err(err);
        }
    }
    Ok(())
}

fn write_setting(repo: &Repository, flag: &str, key: &str, value: Option<&str>) -> Result<()> {
    match value {
        Some(value) => {
            exe::run(repo, ["config", flag, "--replace-all", key, value], None).map(|_| ())
        }
        None => {
            let (code, out) = exe::run_with_code(repo, ["config", flag, "--unset-all", key], None)?;
            // 5: there was nothing to take away.
            if matches!(code, Some(0 | 5)) {
                Ok(())
            } else {
                Err(Error::Refused {
                    what: say!("signature-what-signing-settings"),
                    detail: say!(
                        "signature-setting-not-removed",
                        key,
                        said = out.stderr.trim()
                    ),
                })
            }
        }
    }
}

// --- What a commit carries --------------------------------------------------

/// The first line of the signature a commit carries: the value of the
/// header `name`. Headers end at the first empty line, and the lines of a
/// signature after its first begin with a space, as do those of a tag
/// carried in a `mergetag` header, so only a header itself matches.
fn signature_header<'a>(commit: &'a [u8], name: &[u8]) -> Option<&'a [u8]> {
    for line in commit.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            return None;
        }
        if let Some(value) = line
            .strip_prefix(name)
            .and_then(|rest| rest.strip_prefix(b" "))
        {
            return Some(value);
        }
    }
    None
}

/// Whether each commit carries a signature, and in which format. Nothing is
/// verified and no process is started. Ids that are not commits are left
/// out, as [`crate::details`] leaves them out.
pub fn signed(repo: &Repository, ids: &[Oid]) -> Result<Vec<Signed>> {
    if ids.len() > SIGNED_LIMIT {
        return Err(invalid(
            say!("signature-what-ids"),
            say!("signature-too-many-commits", most = SIGNED_LIMIT),
        ));
    }
    let r = repo.gix();
    // The header is named after the hash the signature was made over.
    let header: &[u8] = match r.object_hash() {
        gix::hash::Kind::Sha1 => b"gpgsig",
        _ => b"gpgsig-sha256",
    };
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Ok(commit) = r.find_commit(*id.as_gix()) else {
            continue;
        };
        let armor = signature_header(&commit.data, header);
        out.push(Signed {
            id: *id,
            signed: armor.is_some(),
            format: armor.and_then(Format::from_armor),
        });
    }
    Ok(out)
}

// --- Verification -----------------------------------------------------------

/// One record per commit, fields and records ended by NUL: nothing a key
/// or a program can say contains one.
const GRADES: &str = "--format=%H%x00%G?%x00%GS%x00%GK%x00%GF%x00%GP%x00%GT%x00%GG%x00";
const GRADE_FIELDS: usize = 8;

/// What git makes of each commit's signature. A commit without one costs
/// nothing; the others are verified by one `git log`. A signature git could
/// not look at comes back as [`State::CannotCheck`] with the reason, never
/// as "none". Ids that are not commits are left out; one that is named
/// twice is answered once.
pub fn verify(repo: &Repository, ids: &[Oid]) -> Result<Vec<CommitVerdict>> {
    if ids.len() > VERIFY_LIMIT {
        return Err(invalid(
            say!("signature-what-ids"),
            say!("signature-too-many-to-verify", most = VERIFY_LIMIT),
        ));
    }
    let mut seen = HashSet::new();
    let wanted: Vec<Oid> = ids.iter().copied().filter(|id| seen.insert(*id)).collect();
    let carried = signed(repo, &wanted)?;
    // Git gives up on a signature whose form it does not know, and with it
    // on every commit asked about in the same breath: those stay here.
    let asked: Vec<Oid> = carried
        .iter()
        .filter(|commit| commit.signed && commit.format.is_some())
        .map(|commit| commit.id)
        .collect();
    let mut graded = HashMap::new();
    let mut refused: HashMap<Oid, String> = HashMap::new();
    if !asked.is_empty() {
        match grade(repo, &asked) {
            Ok(found) => graded = found,
            // One commit git cannot make sense of must not cost the others
            // their answer: once more, one at a time.
            Err(Error::Command { stderr, .. }) => {
                for id in &asked {
                    if asked.len() == 1 {
                        refused.insert(*id, complaint(&stderr));
                        break;
                    }
                    match grade(repo, &[*id]) {
                        Ok(found) => graded.extend(found),
                        Err(Error::Command { stderr, .. }) => {
                            refused.insert(*id, complaint(&stderr));
                        }
                        Err(err) => return Err(err),
                    }
                }
            }
            Err(err) => return Err(err),
        }
    }
    let mut reasons: HashMap<Option<Format>, String> = HashMap::new();
    let mut out = Vec::with_capacity(carried.len());
    for commit in carried {
        let mut verdict = if commit.signed {
            // "No signature" about a commit that has one: git could not
            // look at it.
            graded
                .remove(&commit.id)
                .filter(|verdict| verdict.state != State::None)
                .unwrap_or_else(|| Verdict::of(State::CannotCheck))
        } else {
            Verdict::of(State::None)
        };
        verdict.format = commit.format;
        if verdict.state == State::CannotCheck && verdict.reason.is_none() {
            verdict.reason = Some(match refused.remove(&commit.id) {
                Some(said) => say!("signature-git-could-not-check", said),
                None => reasons
                    .entry(commit.format)
                    .or_insert_with(|| why_unchecked(repo, commit.format, ""))
                    .clone(),
            });
        }
        out.push(CommitVerdict {
            id: commit.id,
            verdict,
        });
    }
    Ok(out)
}

/// Ask git for its grade of these commits, in one process.
fn grade(repo: &Repository, ids: &[Oid]) -> Result<HashMap<Oid, Verdict>> {
    let mut asked = String::new();
    for id in ids {
        asked.push_str(&id.to_hex());
        asked.push('\n');
    }
    // The ids go in on standard input, so their number is no concern of
    // the command line; `log.showSignature` would add text of its own.
    let out = exe::run(
        repo,
        [
            "log",
            "--no-walk=unsorted",
            "--no-show-signature",
            "--no-patch",
            GRADES,
            "--stdin",
        ],
        Some(asked.as_bytes()),
    )?;
    Ok(grades(&out.stdout))
}

/// What git gave as its reason for giving up, in a line.
fn complaint(stderr: &str) -> String {
    let lines = || {
        stderr
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    };
    let line = lines()
        .find_map(|line| line.strip_prefix("fatal: "))
        .or_else(|| lines().next_back())
        .unwrap_or(say!("signature-no-reason"));
    line.chars().take(MAX_REASON).collect()
}

/// Read the output of `git log` with [`GRADES`].
fn grades(listing: &str) -> HashMap<Oid, Verdict> {
    let fields: Vec<&str> = listing.split('\0').collect();
    let mut out = HashMap::new();
    for [id, grade, signer, key, fingerprint, primary, trust, said] in
        fields.as_chunks::<GRADE_FIELDS>().0
    {
        // Every record but the first begins with the line break that ended
        // the one before.
        let Ok(id) = id.trim().parse::<Oid>() else {
            continue;
        };
        let some = |text: &str| Some(text.trim().to_owned()).filter(|text| !text.is_empty());
        let mut verdict = Verdict {
            signer: some(signer),
            key: some(key),
            fingerprint: some(fingerprint),
            primary_fingerprint: some(primary),
            trust: some(trust),
            ..Verdict::of(State::CannotCheck)
        };
        verdict.state = match grade.trim() {
            "G" => State::Good,
            "U" => State::Untrusted,
            "X" => State::Expired,
            "Y" => State::KeyExpired,
            "R" => State::KeyRevoked,
            "B" => State::Bad,
            // Git names the key it looked for and did not find. Without
            // one, what the program said is all there is.
            "E" if verdict.key.is_some() => State::UnknownKey,
            "E" => {
                verdict.reason = last_line(said);
                State::CannotCheck
            }
            "N" => State::None,
            other => {
                verdict.reason = Some(say!(
                    "signature-unknown-grade",
                    grade = format!("{other:?}")
                ));
                State::CannotCheck
            }
        };
        // Trust in a key says nothing where no key was put to use.
        if !verdict.state.used_a_key() {
            verdict.trust = None;
        }
        // A certificate nothing vouches for: gpgsm says why.
        if verdict.state == State::Untrusted
            && let Some((fault, reason)) = chain_fault(said)
        {
            verdict.chain = Some(fault);
            verdict.reason = Some(reason);
        }
        out.insert(id, verdict);
    }
    out
}

/// What gpgsm said is wrong with a certificate's chain, in what it writes
/// for people (git's `%GG`, `git verify-tag` without `--raw`): the line
/// that the chain is invalid, and before it the revocation list that could
/// not be checked, when that was it. None where it said no such thing.
/// The words are gpgsm's own under `LC_ALL=C`, which every child of git's
/// here is started with.
fn chain_fault(said: &str) -> Option<(ChainFault, String)> {
    let after = |marker: &str| {
        said.lines().find_map(|line| {
            line.find(marker)
                .and_then(|at| line.get(at.saturating_add(marker.len())..))
                .map(str::trim)
                .filter(|rest| !rest.is_empty())
        })
    };
    let invalid = after("invalid certification chain: ")?;
    let crl = after("checking the CRL failed: ");
    let fault = match invalid {
        "Missing issuer certificate" => ChainFault::MissingIssuer,
        "Not trusted" => ChainFault::UntrustedRoot,
        "Certificate revoked" => ChainFault::Revoked,
        "No CRL known" | "CRL too old" => ChainFault::RevocationUnknown,
        _ if crl.is_some() => ChainFault::RevocationUnknown,
        _ => ChainFault::Other,
    };
    let reason = match (fault, crl) {
        (ChainFault::RevocationUnknown, Some(crl)) => crl.to_owned(),
        _ => invalid.to_owned(),
    };
    Some((fault, reason.chars().take(MAX_REASON).collect()))
}

/// The last thing a program said, as a reason, cut to a readable length.
fn last_line(said: &str) -> Option<String> {
    said.lines()
        .map(str::trim)
        .rfind(|line| !line.is_empty() && !line.starts_with("[GNUPG:]"))
        .map(|line| line.chars().take(MAX_REASON).collect())
}

/// Why git could not look at a signature of this format: what the
/// configuration tells, and failing that what git itself `said`.
fn why_unchecked(repo: &Repository, format: Option<Format>, said: &str) -> String {
    let Some(format) = format else {
        return say!("signature-form-unknown").to_owned();
    };
    if format == Format::Ssh && config_path(repo, "gpg.ssh.allowedSignersFile").is_none() {
        return say!("signature-no-allowed-signers").to_owned();
    }
    let program = program(repo, format);
    if !installed(repo, &program) {
        return say!(
            "signature-checker-not-found",
            program,
            format = format.name()
        );
    }
    match said
        .lines()
        .find_map(|line| line.trim().strip_prefix("fatal: "))
    {
        Some(line) => say!(
            "signature-git-could-not-check",
            said = line.chars().take(MAX_REASON).collect::<String>()
        ),
        None => say!("signature-git-could-not-check-it").to_owned(),
    }
}

/// The verdict in what `git verify-tag --raw` wrote. `accepted` is how git
/// ended: what it did not accept is not good, whatever the lines say.
fn read_verdict(format: Format, accepted: bool, said: &str) -> Verdict {
    let verdict = match format {
        Format::Ssh => ssh_verdict(said),
        Format::Openpgp | Format::X509 => gnupg_verdict(said),
    };
    if verdict.state == State::Good && !accepted {
        return Verdict::unchecked(say!("signature-not-accepted"));
    }
    verdict
}

/// Read GnuPG's status lines, grading as git grades a commit: a good
/// signature by a key of undefined trust, or of none, is "untrusted".
fn gnupg_verdict(said: &str) -> Verdict {
    let mut verdict = Verdict::of(State::CannotCheck);
    let mut results = 0usize;
    for line in said.lines() {
        let Some(status) = line.trim().strip_prefix("[GNUPG:] ") else {
            continue;
        };
        let (word, rest) = status.split_once(' ').unwrap_or((status, ""));
        let state = match word {
            "GOODSIG" => Some(State::Good),
            "EXPSIG" => Some(State::Expired),
            "EXPKEYSIG" => Some(State::KeyExpired),
            "REVKEYSIG" => Some(State::KeyRevoked),
            "BADSIG" => Some(State::Bad),
            _ => None,
        };
        if let Some(state) = state {
            // The key id, then the user id to the end of the line.
            let (key, signer) = rest.split_once(' ').unwrap_or((rest, ""));
            results = results.saturating_add(1);
            verdict.state = state;
            verdict.key = Some(key.to_owned()).filter(|key| !key.is_empty());
            verdict.signer = Some(signer.trim().to_owned()).filter(|signer| !signer.is_empty());
        } else if word == "ERRSIG" {
            // Key id, two algorithms, class, time, and the reason as a
            // number: 9 is a public key that is missing.
            let mut fields = rest.split(' ');
            results = results.saturating_add(1);
            verdict.key = fields
                .next()
                .map(str::to_owned)
                .filter(|key| !key.is_empty());
            verdict.state = if fields.nth(4) == Some("9") {
                State::UnknownKey
            } else {
                State::CannotCheck
            };
        } else if word == "VALIDSIG" {
            // The fingerprint first; the primary key's, when given, last.
            let fields: Vec<&str> = rest.split(' ').collect();
            let print = |n: usize| {
                fields
                    .get(n)
                    .filter(|print| !print.is_empty())
                    .map(|print| (*print).to_owned())
            };
            verdict.fingerprint = print(0);
            verdict.primary_fingerprint = print(9);
        } else if let Some(level) = word.strip_prefix("TRUST_") {
            verdict.trust = Some(level.to_lowercase());
        }
    }
    if results > 1 {
        return Verdict::unchecked(say!("signature-more-than-one"));
    }
    let trusted = matches!(
        verdict.trust.as_deref(),
        Some("marginal" | "fully" | "ultimate")
    );
    if verdict.state == State::Good && !trusted {
        verdict.state = State::Untrusted;
    }
    if !verdict.state.used_a_key() {
        verdict.trust = None;
    }
    verdict
}

/// Read what ssh-keygen says of a signature. With a principal from the
/// allowed signers it is good; valid without one it is by a key nothing
/// vouches for.
fn ssh_verdict(said: &str) -> Verdict {
    for line in said.lines().map(str::trim) {
        // `<TYPE> key <fingerprint>` ends both forms.
        let print = |text: &str| {
            text.rsplit(' ')
                .next()
                .filter(|print| !print.is_empty())
                .map(str::to_owned)
        };
        if let Some(rest) = line.strip_prefix("Good \"git\" signature for ") {
            let (signer, key) = rest.rsplit_once(" with ").unwrap_or((rest, ""));
            return Verdict {
                signer: Some(signer.to_owned()),
                key: print(key),
                fingerprint: print(key),
                trust: Some("fully".to_owned()),
                ..Verdict::of(State::Good)
            };
        }
        if let Some(key) = line.strip_prefix("Good \"git\" signature with ") {
            return Verdict {
                key: print(key),
                fingerprint: print(key),
                trust: Some("undefined".to_owned()),
                ..Verdict::of(State::Untrusted)
            };
        }
        if line.starts_with("Could not verify signature")
            || line.starts_with("Signature verification failed")
        {
            return Verdict::of(State::Bad);
        }
    }
    Verdict::of(State::CannotCheck)
}

// --- Tags -------------------------------------------------------------------

fn person(signature: gix::actor::SignatureRef<'_>) -> crate::Signature {
    let time = signature.time().ok();
    crate::Signature {
        name: signature.name.to_string(),
        email: signature.email.to_string(),
        seconds: time.map(|time| time.seconds).unwrap_or_default(),
        offset: time.map(|time| time.offset).unwrap_or_default(),
    }
}

fn decoded<'a>(data: &'a [u8], hash: gix::hash::Kind) -> Result<gix::objs::TagRef<'a>> {
    gix::objs::TagRef::from_bytes(data, hash).map_err(|err| {
        invalid(
            say!("signature-what-tag"),
            say!("signature-tag-unreadable", error = err.to_string()),
        )
    })
}

/// A tag in full: who made it and when, its message without the signature,
/// what it points at in the end, and what its own signature is worth. A
/// lightweight tag is only a name for a commit, and is answered as that.
pub fn tag_details(repo: &Repository, name: &str) -> Result<TagDetails> {
    crate::tag::check_tag_name(repo, name)?;
    let value = crate::tag::tag_value(repo, name).ok_or_else(|| Error::Refused {
        what: say!("signature-what-tag"),
        detail: say!("signature-no-tag", name),
    })?;
    let r = repo.gix();
    let object = r
        .find_object(*value.as_gix())
        .map_err(crate::git_err(say!("signature-loading-tag")))?;
    if object.kind != gix::object::Kind::Tag {
        return Ok(TagDetails {
            name: name.to_owned(),
            annotated: false,
            value,
            target: value,
            target_kind: object.kind.to_string(),
            tagger: None,
            message: String::new(),
            signature: Verdict::of(State::None),
        });
    }
    let tag = decoded(&object.data, r.object_hash())?;
    let signature = match tag.signature {
        Some(armor) => tag_verdict(repo, value, Format::from_armor(armor)),
        None => Verdict::of(State::None),
    };
    // Tags of tags: follow them to what is tagged in the end. What kind of
    // object that is, the object says and not the tag; one that is not
    // there ends it.
    let mut target: Oid = tag.target.to_str_lossy().parse()?;
    let mut depth = 0usize;
    let kind = loop {
        let kind = r
            .find_header(*target.as_gix())
            .map_err(crate::git_err(say!("signature-loading-tag-target")))?
            .kind();
        if kind != gix::object::Kind::Tag {
            break kind;
        }
        depth = depth.saturating_add(1);
        if depth > MAX_TAG_DEPTH {
            return Err(invalid(
                say!("signature-what-tag"),
                say!("signature-tags-too-deep", name, most = MAX_TAG_DEPTH),
            ));
        }
        let inner = r
            .find_object(*target.as_gix())
            .map_err(crate::git_err(say!("signature-loading-tag")))?;
        target = decoded(&inner.data, r.object_hash())?
            .target
            .to_str_lossy()
            .parse()?;
    };
    Ok(TagDetails {
        name: name.to_owned(),
        annotated: true,
        value,
        target,
        target_kind: kind.to_string(),
        tagger: tag.tagger().ok().flatten().map(person),
        message: tag.message.to_str_lossy().trim_end_matches('\n').to_owned(),
        signature,
    })
}

/// What git makes of a tag object's signature. One in a form git does not
/// know is not shown to it.
fn tag_verdict(repo: &Repository, tag: Oid, format: Option<Format>) -> Verdict {
    let hex = tag.to_hex();
    let said = format.and_then(|format| {
        let (code, out) =
            exe::run_with_code(repo, ["verify-tag", "--raw", hex.as_str()], None).ok()?;
        Some((
            read_verdict(format, code == Some(0), &out.stderr),
            out.stderr,
        ))
    });
    let (mut verdict, said) = said.unwrap_or((Verdict::of(State::CannotCheck), String::new()));
    verdict.format = format;
    // What is wrong with a certificate's chain gpgsm says in words, which
    // `--raw` leaves out: asked once more for those, in this case alone.
    if format == Some(Format::X509)
        && verdict.state == State::Untrusted
        && let Ok((_, out)) = exe::run_with_code(repo, ["verify-tag", hex.as_str()], None)
        && let Some((fault, reason)) = chain_fault(&out.stderr)
    {
        verdict.chain = Some(fault);
        verdict.reason = Some(reason);
    }
    if verdict.state == State::CannotCheck && verdict.reason.is_none() {
        verdict.reason = Some(why_unchecked(repo, format, &said));
    }
    verdict
}

// --- Signing that failed ----------------------------------------------------

/// Turn git's failure to sign into a refusal that says what is wrong and
/// what to do about it. Any other error is handed back as it came.
pub fn explain_failure(repo: &Repository, err: Error, what: &'static str) -> Error {
    let Error::Command { stderr, .. } = &err else {
        return err;
    };
    // Most failures have nothing to do with signing: look before asking
    // git for the key.
    if signing_failure(stderr, None).is_none() {
        return err;
    }
    let key = exe::config_get(repo, "user.signingkey");
    // A key on a smart card: what gpg said is the same for a card that is
    // not there and a PIN that was not typed, so the card is asked.
    let detail = card::failure(repo, stderr, key.as_deref())
        .or_else(|| signing_failure(stderr, key.as_deref()));
    match detail {
        Some(detail) => Error::Refused { what, detail },
        None => err,
    }
}

/// The rest of the line after the first `open` in `said`.
fn after<'a>(said: &'a str, open: &str) -> Option<&'a str> {
    let (_, rest) = said.split_once(open)?;
    Some(rest.lines().next().unwrap_or("").trim())
}

/// The text between `open` and the next `close` on the same line.
fn between<'a>(said: &'a str, open: &str, close: &str) -> Option<&'a str> {
    after(said, open)?.split_once(close).map(|(inner, _)| inner)
}

/// What gpg or gpgsm said when it would not sign, explained: the key it
/// did not have or could not use, the passphrase it could not ask for.
fn gpg_failure(said: &str, key: Option<&str>) -> Option<String> {
    let whom = between(said, "skipped \"", "\": ")
        .or_else(|| between(said, "can't sign using '", "': "))
        .map(|name| say!("signature-for-whom", name = format!("{name:?}")))
        .unwrap_or_default();
    if said.contains("Unusable secret key") {
        return Some(say!("signature-secret-key-unusable", whom));
    }
    if said.contains("No secret key") || said.contains("can't sign using") {
        return Some(match key {
            Some(_) => say!("signature-no-secret-key-set", whom),
            None => say!("signature-no-secret-key-unset", whom),
        });
    }
    if let Some(why) = after(said, "signing failed: ") {
        // A key on a smart card; what the card itself is up to is asked
        // where the repository is at hand (`card`).
        if why.contains("Bad PIN") {
            return Some(say!("signature-card-pin-refused", why));
        }
        if why.contains("PIN blocked") {
            return Some(say!("signature-card-pin-blocked", why));
        }
        if why.contains("No SmartCard daemon") {
            return Some(say!("signature-card-no-daemon").to_owned());
        }
        if why.contains("Card removed") || why.contains("Card not present") {
            return Some(say!("signature-card-not-in-reader", why));
        }
        const UNANSWERED: [&str; 8] = [
            "No pinentry",
            "Inappropriate ioctl for device",
            "cancelled",
            "canceled",
            "Timeout",
            "Bad passphrase",
            "No agent",
            "IPC ",
        ];
        if UNANSWERED.iter().any(|sign| why.contains(sign)) {
            // Homebrew's gpg asks in a terminal unless told otherwise.
            let which = if cfg!(target_os = "macos") {
                say!("signature-pinentry-for-macos")
            } else {
                ""
            };
            return Some(say!("signature-no-passphrase", why, which));
        }
    }
    said.contains("can't connect to the agent")
        .then(|| say!("signature-no-gpg-agent").to_owned())
}

/// What to tell the user when git could not sign: what is wrong and what
/// to do, read from what git and the signing program wrote. `key` is the
/// signing key that is set, which tells "no key" from "key not found".
/// None when the failure is not one of signing.
pub fn signing_failure(said: &str, key: Option<&str>) -> Option<String> {
    // Said before anything is tried.
    if said.contains("user.signingkey or gpg.ssh.defaultKeyCommand needs to be configured") {
        return Some(say!("signature-no-ssh-key").to_owned());
    }
    if let Some(value) = between(said, "invalid value for 'gpg.format': '", "'") {
        return Some(say!(
            "signature-format-unknown",
            value = format!("{value:?}")
        ));
    }
    let signing =
        said.contains("failed to sign the data") || said.contains("unable to sign the tag");
    if !signing && !said.contains("failed to write commit object") {
        return None;
    }

    // The program itself. Git for Windows says it could not spawn it, and
    // its path has a colon after the drive: the reason follows the last
    // ": " of the line.
    let missing = between(said, "cannot exec '", "'").or_else(|| {
        ["cannot spawn ", "cannot run "]
            .into_iter()
            .find_map(|open| after(said, open)?.rsplit_once(": "))
            .map(|(program, _)| program)
    });
    if let Some(program) = missing {
        return Some(say!("signature-signer-not-started", program));
    }

    if signing && let Some(told) = gpg_failure(said, key) {
        return Some(told);
    }

    // ssh-keygen: the key file, the agent, the passphrase.
    if let Some(rest) = after(said, "Couldn't load public key ") {
        let path = rest.rsplit_once(": ").map_or(rest, |(path, _)| path);
        return Some(say!("signature-ssh-key-unreadable", path));
    }
    if let Some(path) = between(said, "No private key found for public key \"", "\"") {
        return Some(say!("signature-no-private-key-anywhere", path));
    }
    if said.contains("Couldn't get agent socket") {
        return Some(say!("signature-no-ssh-agent").to_owned());
    }
    if said.contains("Couldn't find key in agent") {
        return Some(say!("signature-agent-lacks-key").to_owned());
    }
    if said.contains("incorrect passphrase supplied") || said.contains("Enter passphrase for") {
        return Some(say!("signature-ssh-passphrase").to_owned());
    }

    // Signing, but nothing known: hand on the program's own last word.
    if !signing {
        return None;
    }
    let word = said
        .lines()
        .map(str::trim)
        .rfind(|line| {
            !line.is_empty()
                && !line.starts_with("[GNUPG:]")
                && !line.starts_with("error:")
                && !line.starts_with("fatal:")
                && !line.starts_with("hint:")
                && !line.starts_with("The tag message has been left")
                && *line != "(no gpg output)"
        })
        .map(|line| line.chars().take(MAX_REASON).collect::<String>());
    Some(match word {
        Some(word) => say!("signature-signing-failed", word),
        None => say!("signature-signing-failed-silently").to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program that ends while something it started keeps its streams
    /// open, as gpg does on Windows when it starts its agent: what it said
    /// comes at once, not when the other ends or the time is up.
    #[test]
    fn an_answer_is_read_when_the_program_ends_though_a_daemon_holds_its_pipes() {
        let dir = std::env::temp_dir();
        let started = Instant::now();
        let answer = if cfg!(windows) {
            ask_in(
                &dir,
                "cmd",
                ["/C", "echo said& start /b ping -n 6 127.0.0.1 >nul"],
            )
        } else {
            ask_in(&dir, "sh", ["-c", "echo said; sleep 5 & exit 0"])
        }
        .unwrap();
        assert!(
            started.elapsed() < Duration::from_secs(3),
            "{:?}",
            started.elapsed()
        );
        assert!(answer.ok);
        assert_eq!(answer.stdout.trim(), "said");
    }

    #[test]
    fn the_agent_is_asked_by_the_ssh_add_beside_gits_ssh_keygen() {
        let dir = std::env::temp_dir().join(format!("bitbraid-ssh-add-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let exe = std::env::consts::EXE_SUFFIX;
        let keygen = dir.join(format!("ssh-keygen{exe}"));
        std::fs::write(&keygen, "").unwrap();
        let keygen = keygen.to_str().unwrap().to_owned();
        // Nothing beside it: the one on PATH.
        assert_eq!(ssh_add_for(&keygen), "ssh-add");
        let beside = dir.join(format!("ssh-add{exe}"));
        std::fs::write(&beside, "").unwrap();
        assert_eq!(ssh_add_for(&keygen), beside.to_str().unwrap());
        // A name alone, or nothing: the one on PATH.
        assert_eq!(ssh_add_for("ssh-keygen"), "ssh-add");
        assert_eq!(ssh_add_for(""), "ssh-add");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_format_is_told_from_the_first_line_of_a_signature() {
        for (line, format) in [
            ("-----BEGIN PGP SIGNATURE-----", Some(Format::Openpgp)),
            ("-----BEGIN PGP MESSAGE-----", Some(Format::Openpgp)),
            ("-----BEGIN SSH SIGNATURE-----", Some(Format::Ssh)),
            ("-----BEGIN SIGNED MESSAGE-----", Some(Format::X509)),
            ("-----BEGIN CERTIFICATE-----", None),
            (" -----BEGIN PGP SIGNATURE-----", None),
            ("", None),
        ] {
            assert_eq!(Format::from_armor(line.as_bytes()), format, "{line:?}");
        }
        for format in [Format::Openpgp, Format::Ssh, Format::X509] {
            assert_eq!(Format::parse(format.name()), Some(format));
            assert_eq!(
                serde_json::to_value(format).unwrap(),
                serde_json::json!(format.name())
            );
        }
        for text in ["", "SSH", "gpg", "pgp", " ssh", "nonsense"] {
            assert_eq!(Format::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn only_a_header_of_the_commit_counts_as_its_signature() {
        let signed = b"tree 8f26\nparent eb39\nauthor A <a@b> 1 +0000\ncommitter A <a@b> 1 +0000\ngpgsig -----BEGIN SSH SIGNATURE-----\n U1NI\n -----END SSH SIGNATURE-----\n\nmessage\n";
        assert_eq!(
            signature_header(signed, b"gpgsig"),
            Some(b"-----BEGIN SSH SIGNATURE-----".as_slice())
        );
        // The other hash's header is another header.
        assert_eq!(signature_header(signed, b"gpgsig-sha256"), None);
        let other = b"tree 8f26\ngpgsig-sha256 -----BEGIN PGP SIGNATURE-----\n x\n\nm\n";
        assert_eq!(signature_header(other, b"gpgsig"), None);
        assert!(signature_header(other, b"gpgsig-sha256").is_some());
        // A message that quotes a signature, and a signed tag that was
        // merged, are not signatures of the commit.
        let quoted = b"tree 8f26\nauthor A <a@b> 1 +0000\n\ngpgsig -----BEGIN PGP SIGNATURE-----\n";
        assert_eq!(signature_header(quoted, b"gpgsig"), None);
        let merged = b"tree 8f26\nmergetag object 1234\n type commit\n tag v1\n \n gpgsig -----BEGIN PGP SIGNATURE-----\n -----BEGIN PGP SIGNATURE-----\n\nm\n";
        assert_eq!(signature_header(merged, b"gpgsig"), None);
        for garbage in [
            &b""[..],
            b"\n",
            b"gpgsig",
            b"gpgsig\n",
            b"\0\0\0",
            b"gpgsig ",
        ] {
            let _ = signature_header(garbage, b"gpgsig");
        }
        assert_eq!(signature_header(b"gpgsig ", b"gpgsig"), Some(&b""[..]));
    }

    /// `git log` with [`GRADES`], as git 2.56 writes it.
    fn record(id: &str, fields: [&str; 7]) -> String {
        let mut out = format!("{id}\0");
        for field in fields {
            out.push_str(field);
            out.push('\0');
        }
        out.push('\n');
        out
    }

    #[test]
    fn every_grade_of_git_has_a_state() {
        let id = |n: u8| format!("{:040x}", n);
        let fpr = "75983EFC91DF51D61A0D31A37B50146F95142D4D";
        let ada = "Ada Fixture <ada@fixture.invalid>";
        let key = "7B50146F95142D4D";
        let mut listing = String::new();
        for (n, grade) in (0u8..).zip(["G", "U", "X", "Y", "R"]) {
            listing.push_str(&record(
                &id(n),
                [
                    grade,
                    ada,
                    key,
                    fpr,
                    fpr,
                    "ultimate",
                    "gpg: Good signature\n",
                ],
            ));
        }
        listing.push_str(&record(
            &id(5),
            ["B", ada, key, "", "", "undefined", "gpg: BAD signature\n"],
        ));
        listing.push_str(&record(
            &id(6),
            [
                "E",
                "",
                key,
                "",
                "",
                "undefined",
                "gpg: Can't check signature: No public key\n",
            ],
        ));
        listing.push_str(&record(
            &id(7),
            [
                "E",
                "",
                "",
                "",
                "",
                "undefined",
                "gpg: some\ngpg: other trouble\n\n",
            ],
        ));
        listing.push_str(&record(&id(8), ["N", "", "", "", "", "undefined", ""]));
        listing.push_str(&record(&id(9), ["?", "", "", "", "", "", ""]));
        let graded = grades(&listing);
        let state = |n: u8| graded[&id(n).parse::<Oid>().unwrap()].state;
        assert_eq!(
            (0..10).map(state).collect::<Vec<_>>(),
            [
                State::Good,
                State::Untrusted,
                State::Expired,
                State::KeyExpired,
                State::KeyRevoked,
                State::Bad,
                State::UnknownKey,
                State::CannotCheck,
                State::None,
                State::CannotCheck,
            ]
        );
        let good = &graded[&id(0).parse::<Oid>().unwrap()];
        assert_eq!(good.signer.as_deref(), Some(ada));
        assert_eq!(good.key.as_deref(), Some(key));
        assert_eq!(good.fingerprint.as_deref(), Some(fpr));
        assert_eq!(good.primary_fingerprint.as_deref(), Some(fpr));
        assert_eq!(good.trust.as_deref(), Some("ultimate"));
        assert_eq!(good.reason, None);
        let unknown = &graded[&id(6).parse::<Oid>().unwrap()];
        assert_eq!(unknown.key.as_deref(), Some(key));
        assert_eq!(unknown.trust, None, "no key was used, so none is trusted");
        let unchecked = &graded[&id(7).parse::<Oid>().unwrap()];
        assert_eq!(unchecked.reason.as_deref(), Some("gpg: other trouble"));
        assert_eq!(unchecked.trust, None);
        assert!(graded[&id(9).parse::<Oid>().unwrap()].reason.is_some());

        // Cut short, empty or not a listing at all: what is whole is read.
        let whole = record(&id(1), ["G", "", "", "", "", "", ""]);
        for cut in 0..whole.len() {
            let _ = grades(whole.get(..cut).unwrap_or(""));
        }
        assert_eq!(grades("").len(), 0);
        assert_eq!(grades("\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0").len(), 0);
        assert_eq!(
            grades(&format!("not-an-id\0G\0\0\0\0\0\0\0\n{whole}")).len(),
            1
        );
    }

    /// What gpgsm 2.4 wrote for people about a certificate from a local
    /// authority, in each case the tests with a real gpgsm and dirmngr
    /// bring about (`tests/suite/x509_chains.rs` in the engine).
    #[test]
    fn what_gpgsm_says_of_a_chain_is_read() {
        let said = |lines: &[&str]| {
            let mut text = String::from(
                "gpgsm: Signature made 2026-10-17 06:11:47 UTC\n\
                 gpgsm:                using rsa2048 key 6D9FDA24ACCD0838389C68B859BE0B146AD74A50\n",
            );
            for line in lines {
                text.push_str("gpgsm: ");
                text.push_str(line);
                text.push('\n');
            }
            text
        };
        // What gpgsm wrote, and what is read from it.
        type Case<'a> = (&'a [&'a str], Option<(ChainFault, &'a str)>);
        let cases: &[Case<'_>] = &[
            (
                &[
                    "issuer certificate not found",
                    "issuer certificate: #/O=BitBraid Test,CN=Test Intermediate",
                    "invalid certification chain: Missing issuer certificate",
                ],
                Some((ChainFault::MissingIssuer, "Missing issuer certificate")),
            ),
            (
                &[
                    "root certificate is not marked trusted",
                    "interactive marking as trusted not enabled in gpg-agent",
                    "invalid certification chain: Not trusted",
                ],
                Some((ChainFault::UntrustedRoot, "Not trusted")),
            ),
            (
                &[
                    "certificate #03/O=BitBraid Test,CN=Test Intermediate",
                    "certificate has been revoked",
                    "invalid certification chain: Certificate revoked",
                ],
                Some((ChainFault::Revoked, "Certificate revoked")),
            ),
            (
                &[
                    "certificate #03/O=BitBraid Test,CN=Test Intermediate",
                    "checking the CRL failed: Not supported",
                    "invalid certification chain: Not supported",
                ],
                Some((ChainFault::RevocationUnknown, "Not supported")),
            ),
            (
                &[
                    "checking the CRL failed: Connection refused",
                    "invalid certification chain: Connection refused",
                ],
                Some((ChainFault::RevocationUnknown, "Connection refused")),
            ),
            (
                &["invalid certification chain: No CRL known"],
                Some((ChainFault::RevocationUnknown, "No CRL known")),
            ),
            (
                &["invalid certification chain: Bad certificate"],
                Some((ChainFault::Other, "Bad certificate")),
            ),
            // A chain that holds, and nothing at all.
            (
                &[
                    "CRLs not checked due to --disable-crl-checks option",
                    "Good signature from \"/CN=Ada Fixture/O=BitBraid Test\"",
                ],
                None,
            ),
            (&[], None),
            (&["invalid certification chain: "], None),
        ];
        for (lines, expected) in cases {
            let found = chain_fault(&said(lines));
            assert_eq!(
                found
                    .as_ref()
                    .map(|(fault, reason)| (*fault, reason.as_str())),
                *expected,
                "{lines:?}"
            );
        }

        // Read with git's grade: only a key nothing vouches for carries it.
        let id = |n: u8| format!("{:040x}", n);
        let leaf = "6D9FDA24ACCD0838389C68B859BE0B146AD74A50";
        let signer = "/CN=Ada Fixture/O=BitBraid Test";
        let revoked = said(&[
            "certificate has been revoked",
            "invalid certification chain: Certificate revoked",
        ]);
        let mut listing = record(
            &id(1),
            ["U", signer, leaf, leaf, "", "never", revoked.as_str()],
        );
        listing.push_str(&record(
            &id(2),
            ["G", signer, leaf, leaf, "", "fully", revoked.as_str()],
        ));
        let graded = grades(&listing);
        let first = &graded[&id(1).parse::<Oid>().unwrap()];
        assert_eq!(first.state, State::Untrusted);
        assert_eq!(first.chain, Some(ChainFault::Revoked));
        assert_eq!(first.reason.as_deref(), Some("Certificate revoked"));
        assert_eq!(first.trust.as_deref(), Some("never"));
        let second = &graded[&id(2).parse::<Oid>().unwrap()];
        assert_eq!(second.chain, None);
        assert_eq!(second.reason, None);

        // Anything at all, cut anywhere.
        let long = said(&[&format!(
            "invalid certification chain: {}",
            "x".repeat(5000)
        )]);
        let (_, reason) = chain_fault(&long).unwrap();
        assert_eq!(reason.chars().count(), MAX_REASON);
        for cut in 0..revoked.len() {
            let _ = chain_fault(revoked.get(..cut).unwrap_or(""));
        }
        let _ = chain_fault("invalid certification chain: \u{0}\u{7f}é");
    }

    #[test]
    fn gnupg_status_lines_are_graded_as_git_grades_a_commit() {
        let good = "[GNUPG:] NEWSIG ada@fixture.invalid\n\
            [GNUPG:] KEY_CONSIDERED 75983EFC91DF51D61A0D31A37B50146F95142D4D 0\n\
            [GNUPG:] SIG_ID 47Vzd7/2S+ZKYgLwfbIT/V6Tng8 2026-10-10 1791611687\n\
            [GNUPG:] GOODSIG 7B50146F95142D4D Ada Fixture <ada@fixture.invalid>\n\
            [GNUPG:] VALIDSIG 75983EFC91DF51D61A0D31A37B50146F95142D4D 2026-10-10 1791611687 0 4 0 22 10 00 AAAA3EFC91DF51D61A0D31A37B50146F95142D4D\n\
            [GNUPG:] TRUST_ULTIMATE 0 pgp ada@fixture.invalid\n";
        let verdict = read_verdict(Format::Openpgp, true, good);
        assert_eq!(verdict.state, State::Good);
        assert_eq!(
            verdict.signer.as_deref(),
            Some("Ada Fixture <ada@fixture.invalid>")
        );
        assert_eq!(verdict.key.as_deref(), Some("7B50146F95142D4D"));
        assert_eq!(
            verdict.fingerprint.as_deref(),
            Some("75983EFC91DF51D61A0D31A37B50146F95142D4D")
        );
        assert_eq!(
            verdict.primary_fingerprint.as_deref(),
            Some("AAAA3EFC91DF51D61A0D31A37B50146F95142D4D")
        );
        assert_eq!(verdict.trust.as_deref(), Some("ultimate"));
        // Git did not accept it: then it is not good.
        let refused = read_verdict(Format::Openpgp, false, good);
        assert_eq!(refused.state, State::CannotCheck);
        assert!(refused.reason.is_some());

        for (trust, state) in [
            ("TRUST_UNDEFINED 0 pgp", State::Untrusted),
            ("TRUST_NEVER", State::Untrusted),
            ("TRUST_MARGINAL 0 pgp", State::Good),
            ("TRUST_FULLY 0 pgp", State::Good),
        ] {
            let said = good.replace("TRUST_ULTIMATE 0 pgp ada@fixture.invalid", trust);
            assert_eq!(gnupg_verdict(&said).state, state, "{trust}");
        }
        // No word on trust is no trust.
        let silent: String = good.lines().take(5).collect::<Vec<_>>().join("\n");
        assert_eq!(gnupg_verdict(&silent).state, State::Untrusted);

        for (word, state) in [
            ("EXPSIG", State::Expired),
            ("EXPKEYSIG", State::KeyExpired),
            ("REVKEYSIG", State::KeyRevoked),
            ("BADSIG", State::Bad),
        ] {
            let said = good.replace("GOODSIG", word);
            assert_eq!(gnupg_verdict(&said).state, state, "{word}");
        }
        let missing = "[GNUPG:] NEWSIG ada@fixture.invalid\n\
            [GNUPG:] ERRSIG 7B50146F95142D4D 22 10 00 1791611687 9 75983EFC91DF51D61A0D31A37B50146F95142D4D\n\
            [GNUPG:] NO_PUBKEY 7B50146F95142D4D\n\
            [GNUPG:] FAILURE gpg-exit 33554433\n";
        let verdict = gnupg_verdict(missing);
        assert_eq!(verdict.state, State::UnknownKey);
        assert_eq!(verdict.key.as_deref(), Some("7B50146F95142D4D"));
        let unsupported = missing.replace(" 9 7598", " 4 7598");
        assert_eq!(gnupg_verdict(&unsupported).state, State::CannotCheck);

        // Two signatures in one: neither is taken.
        let twice = format!("{good}{good}");
        let verdict = gnupg_verdict(&twice);
        assert_eq!(verdict.state, State::CannotCheck);
        assert!(verdict.reason.is_some());

        for garbage in [
            "",
            "[GNUPG:] ",
            "[GNUPG:] GOODSIG",
            "[GNUPG:] ERRSIG",
            "[GNUPG:] ERRSIG a b",
            "[GNUPG:] VALIDSIG",
            "[GNUPG:] TRUST_",
            "fatal: cannot exec 'gpg': No such file or directory",
            "\0\u{1f}\n\n[GNUPG:]",
        ] {
            let verdict = gnupg_verdict(garbage);
            assert!(
                matches!(verdict.state, State::CannotCheck | State::Untrusted),
                "{garbage:?}: {verdict:?}"
            );
        }
    }

    #[test]
    fn what_ssh_keygen_says_is_read() {
        let print = "SHA256:tqO0Qg5pMcH31It8HIHHoNV3YMDsxC2jeanMRfG7on4";
        let good =
            format!("Good \"git\" signature for ada@fixture.invalid with ED25519 key {print}\n");
        let verdict = read_verdict(Format::Ssh, true, &good);
        assert_eq!(verdict.state, State::Good);
        assert_eq!(verdict.signer.as_deref(), Some("ada@fixture.invalid"));
        assert_eq!(verdict.key.as_deref(), Some(print));
        assert_eq!(verdict.fingerprint.as_deref(), Some(print));
        assert_eq!(
            read_verdict(Format::Ssh, false, &good).state,
            State::CannotCheck
        );
        // A principal may have spaces, and "with", in it.
        let odd = format!("Good \"git\" signature for ada with a hat with ED25519 key {print}\n");
        assert_eq!(ssh_verdict(&odd).signer.as_deref(), Some("ada with a hat"));

        let stranger =
            format!("Good \"git\" signature with ED25519 key {print}\nNo principal matched.\n");
        let verdict = read_verdict(Format::Ssh, false, &stranger);
        assert_eq!(verdict.state, State::Untrusted);
        assert_eq!(verdict.signer, None);
        assert_eq!(verdict.fingerprint.as_deref(), Some(print));

        let bad =
            "Could not verify signature.\nSignature verification failed: incorrect signature\n";
        assert_eq!(read_verdict(Format::Ssh, false, bad).state, State::Bad);
        for unread in [
            "",
            "error: gpg.ssh.allowedSignersFile needs to be configured and exist for ssh signature verification\n",
            "Good \"git\" signature",
            "\0",
        ] {
            assert_eq!(ssh_verdict(unread).state, State::CannotCheck, "{unread:?}");
        }
        for short in [
            "Good \"git\" signature for ",
            "Good \"git\" signature with ",
        ] {
            let _ = ssh_verdict(short);
        }
    }

    /// What gpgsm 2.4 wrote for a certificate made in the tests: the dates
    /// are written out, where gpg counts seconds.
    #[test]
    fn a_listing_by_gpgsm_gives_the_certificate() {
        let listing = "/tmp/store/pubring.kbx\n-------------------------------------\n\
            crs:n:2048:1:CEAA6749E3335EFC:20261010T125710:20630405T170000:0C98FC194D00F0C0::CN=Ada Fixture,O=BitBraid Test::sS::::::23:\n\
            fpr:::::::::2B384BD571947E20DA1FB9F5CEAA6749E3335EFC:::2B384BD571947E20DA1FB9F5CEAA6749E3335EFC:\n\
            fp2:::::::::4EC0FB30D557FAD78F5AEDAE4D6FF226F9F3CC86BFC104582A5B5DD544359CCF::::\n\
            grp:::::::::4729BA3B1E43CAF26AE4144168538DDB972531B7:\n\
            uid:n::::::::CN=Ada Fixture,O=BitBraid Test:::\n\
            uid:n::::::::<ada@fixture.invalid>::\n";
        let keys = secret_keys(listing);
        assert_eq!(keys.len(), 1, "{keys:?}");
        assert_eq!(keys[0].id, "CEAA6749E3335EFC");
        assert_eq!(
            keys[0].fingerprint.as_deref(),
            Some("2B384BD571947E20DA1FB9F5CEAA6749E3335EFC")
        );
        assert_eq!(
            keys[0].user.as_deref(),
            Some("CN=Ada Fixture,O=BitBraid Test")
        );
        assert_eq!(keys[0].kind.as_deref(), Some("rsa2048"));
        assert!(keys[0].usable);
        // 2063-04-05 17:00:00 UTC.
        assert_eq!(keys[0].expires, Some(2_943_018_000));
        // Dates that are none are no date.
        assert_eq!(expiry(""), None);
        assert_eq!(expiry("20631345T990000"), None);
        assert_eq!(expiry("not a date"), None);
        assert_eq!(expiry("1893495600"), Some(1_893_495_600));
        assert_eq!(expiry("19700101T000000"), Some(0));
    }

    #[test]
    fn a_colon_listing_gives_the_keys_that_can_sign() {
        let listing = "sec:u:2048:1:32ED9C4AD7E62387:1791611986:1893495600::u:::scSC:::+:::23::0:\n\
            fpr:::::::::F60CB46B359C935A57BBCCFD32ED9C4AD7E62387:\n\
            grp:::::::::44F333382DC88D3363E3C717ED99CF8F01373D8A:\n\
            uid:u::::1791611986::BBC0::Ada \"Colon\\x3a Fixture\" <ada@fixture.invalid>::::::::::0:\n\
            uid:u::::1791611986::BBC1::Second Name <second@fixture.invalid>::::::::::0:\n\
            ssb:u:2048:1:1111111111111111:1791611986::::::e:::+:::23:\n\
            fpr:::::::::0000000000000000000000001111111111111111:\n\
            sec:u:255:22:7B50146F95142D4D:1791611686:::u:::scSC:::+::ed25519:::0:\n\
            fpr:::::::::75983EFC91DF51D61A0D31A37B50146F95142D4D:\n\
            uid:u::::1791611686::786A::Ada Fixture <ada@fixture.invalid>::::::::::0:\n\
            sec:u:2048:1:55B51AB07B99D4C8:1791611986:::u:::ecEC:::+:::23::0:\n\
            fpr:::::::::F5D588D8620D3FB4D007A9F855B51AB07B99D4C8:\n\
            uid:u::::1791611986::2FB9::Enc Only <enc@fixture.invalid>::::::::::0:\n\
            sec:e:255:22:97B13A414ECCF41C:1791611741:1893495600::u:::sc:::+::ed25519:::0:\n\
            fpr:::::::::BCE548C75B902C3DFED579DB97B13A414ECCF41C:\n\
            uid:e::::1791611741::AAAA::Old Key <old@fixture.invalid>::::::::::0:\n";
        let keys = secret_keys(listing);
        assert_eq!(keys.len(), 4);
        assert_eq!(keys[0].id, "32ED9C4AD7E62387");
        assert_eq!(
            keys[0].fingerprint.as_deref(),
            Some("F60CB46B359C935A57BBCCFD32ED9C4AD7E62387"),
            "the key's own, not its subkey's"
        );
        assert_eq!(
            keys[0].user.as_deref(),
            Some("Ada \"Colon: Fixture\" <ada@fixture.invalid>")
        );
        assert_eq!(keys[0].kind.as_deref(), Some("rsa2048"));
        assert_eq!(keys[0].expires, Some(1_893_495_600));
        assert!(keys[0].usable);
        assert_eq!(keys[1].kind.as_deref(), Some("ed25519"));
        assert_eq!(keys[1].expires, None);
        assert!(keys[1].usable);
        assert!(!keys[2].usable, "a key that only encrypts");
        assert!(!keys[3].usable, "an expired key");

        for garbage in [
            "",
            "sec",
            "sec:",
            "sec::::\nfpr\nuid",
            "fpr:::::::::ABCD:\nuid:::::::::x:",
            ":::::::::::::::::::::",
            "uid:u::::1::2::\\x\\x4\\xzz\\x3",
        ] {
            assert!(secret_keys(garbage).is_empty(), "{garbage:?}");
        }
        assert_eq!(unescaped("a\\x3ab"), "a:b");
        assert_eq!(unescaped("\\x"), "\\x");
        assert_eq!(unescaped("\\xzz\\x4"), "\\xzz\\x4");
        assert_eq!(unescaped("é\\x3a"), "é:");
    }

    #[test]
    fn public_keys_are_told_from_everything_else() {
        let line = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIEK8P/cbnj3b4Iwcs9A/Nkpw08w+sI+/myMSRMnJHhPm ada@fixture.invalid with a note";
        let key = public_key(line).unwrap();
        assert_eq!(key.kind, "ssh-ed25519");
        assert_eq!(key.comment, "ada@fixture.invalid with a note");
        assert!(public_key("ecdsa-sha2-nistp256 AAAA").is_some());
        assert!(public_key("sk-ssh-ed25519@openssh.com AAAA= key").is_some());
        for not in [
            "",
            "ssh-ed25519",
            "garbage",
            "not a key",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
            "ssh-ed25519 not!base64",
            "ssh-ed\u{0}25519 AAAA",
            "# ssh-ed25519 AAAA",
        ] {
            assert_eq!(public_key(not), None, "{not:?}");
        }
        assert!(is_literal("key::ssh-ed25519 AAAA"));
        assert!(is_literal("ssh-rsa AAAA me"));
        assert!(!is_literal("/home/ada/.ssh/id_ed25519.pub"));
        assert!(!is_literal("~/.ssh/id_ed25519"));
        assert_eq!(
            literal_key("key::ssh-ed25519 AAAA").map(|key| key.blob),
            Some("AAAA".to_owned())
        );
        assert_eq!(literal_key("key::nothing"), None);
        assert_eq!(
            fingerprint_of("256 SHA256:3DS04pVr1AalkEzs5caJv0rjroEGgg4NgjIBq5bFReg ada (ED25519)")
                .as_deref(),
            Some("SHA256:3DS04pVr1AalkEzs5caJv0rjroEGgg4NgjIBq5bFReg")
        );
        assert_eq!(fingerprint_of("The agent has no identities."), None);
        assert_eq!(fingerprint_of(""), None);
    }

    /// What git 2.56 with GnuPG 2.4.9 and OpenSSH 10.5 wrote in each case.
    #[test]
    fn signing_failures_are_explained() {
        let no_key = "error: gpg failed to sign the data:\n\
            gpg: skipped \"Ada Fixture <ada@fixture.invalid>\": No secret key\n\
            [GNUPG:] INV_SGNR 9 Ada Fixture <ada@fixture.invalid>\n\
            [GNUPG:] FAILURE sign 17\n\
            gpg: signing failed: No secret key\n\
            \n\
            fatal: failed to write commit object";
        let said = signing_failure(no_key, None).unwrap();
        assert!(said.starts_with("no signing key is set"), "{said}");
        assert!(said.contains("Ada Fixture <ada@fixture.invalid>"), "{said}");

        let not_found = "error: gpg failed to sign the data:\n\
            gpg: skipped \"DEADBEEFDEADBEEF\": No secret key\n\
            [GNUPG:] INV_SGNR 9 DEADBEEFDEADBEEF\n\
            [GNUPG:] FAILURE sign 17\n\
            gpg: signing failed: No secret key\n\
            \n\
            error: unable to sign the tag\n\
            The tag message has been left in .git/TAG_EDITMSG";
        let said = signing_failure(not_found, Some("DEADBEEFDEADBEEF")).unwrap();
        assert!(
            said.contains("no secret key for \"DEADBEEFDEADBEEF\""),
            "{said}"
        );
        assert!(said.contains("the signing key that is set"), "{said}");

        let unusable = "error: gpg failed to sign the data:\n\
            [GNUPG:] KEY_CONSIDERED BCE548C75B902C3DFED579DB97B13A414ECCF41C 3\n\
            gpg: skipped \"Ada Fixture <ada@fixture.invalid>\": Unusable secret key\n\
            [GNUPG:] INV_SGNR 9 Ada Fixture <ada@fixture.invalid>\n\
            [GNUPG:] FAILURE sign 54\n\
            gpg: signing failed: Unusable secret key\n\
            \n\
            fatal: failed to write commit object";
        let said = signing_failure(unusable, None).unwrap();
        assert!(said.contains("expired"), "{said}");

        let no_program = "fatal: cannot exec '/opt/no-such-gpg': No such file or directory\n\
            error: gpg failed to sign the data:\n\
            (no gpg output)\n\
            fatal: failed to write commit object";
        let said = signing_failure(no_program, None).unwrap();
        assert!(
            said.contains("/opt/no-such-gpg could not be started"),
            "{said}"
        );
        let no_keygen = "fatal: cannot exec '/opt/no-such-keygen': No such file or directory\n\
            error: \n\
            error: unable to sign the tag\n\
            The tag message has been left in .git/TAG_EDITMSG";
        let said = signing_failure(no_keygen, None).unwrap();
        assert!(
            said.contains("/opt/no-such-keygen could not be started"),
            "{said}"
        );
        // As Git for Windows says it, with the drive's colon in the path.
        let no_program_on_windows = "error: cannot spawn D:/keys/no-such-gpg: No such file or directory\n\
            error: gpg failed to sign the data:\n\
            (no gpg output)\n\
            fatal: failed to write commit object";
        let said = signing_failure(no_program_on_windows, None).unwrap();
        assert!(
            said.contains("D:/keys/no-such-gpg could not be started"),
            "{said}"
        );

        let no_pinentry = "error: gpg failed to sign the data:\n\
            [GNUPG:] KEY_CONSIDERED 95AA381B8F7E43B6E10D5C250754F4F322C1DB15 2\n\
            [GNUPG:] BEGIN_SIGNING H10\n\
            gpg: signing failed: No pinentry\n\
            [GNUPG:] FAILURE sign 67108949\n\
            gpg: signing failed: No pinentry\n\
            \n\
            fatal: failed to write commit object";
        let said = signing_failure(no_pinentry, None).unwrap();
        assert!(
            said.contains("passphrase") && said.contains("No pinentry"),
            "{said}"
        );
        let no_terminal = no_pinentry.replace("No pinentry", "Inappropriate ioctl for device");
        assert!(
            signing_failure(&no_terminal, None)
                .unwrap()
                .contains("passphrase")
        );

        let certificate = "error: gpg failed to sign the data:\n\
            gpgsm: can't sign using 'Ada Fixture <ada@fixture.invalid>': No public key\n\
            [GNUPG:] INV_SGNR 1 Ada Fixture <ada@fixture.invalid>\n\
            [GNUPG:] INV_RECP 1 Ada Fixture <ada@fixture.invalid>\n\
            \n\
            fatal: failed to write commit object";
        let said = signing_failure(certificate, None).unwrap();
        assert!(said.starts_with("no signing key is set"), "{said}");

        let ssh_no_key =
            "fatal: either user.signingkey or gpg.ssh.defaultKeyCommand needs to be configured";
        assert!(
            signing_failure(ssh_no_key, None)
                .unwrap()
                .starts_with("no SSH signing key is set")
        );

        let ssh_missing = "error: Couldn't load public key /keys/missing.pub: No such file or directory?\n\
            \n\
            fatal: failed to write commit object";
        let said = signing_failure(ssh_missing, Some("/keys/missing.pub")).unwrap();
        assert!(
            said.contains("/keys/missing.pub could not be read"),
            "{said}"
        );

        let ssh_no_agent =
            "error: Couldn't get agent socket?\n\nfatal: failed to write commit object";
        assert!(
            signing_failure(ssh_no_agent, None)
                .unwrap()
                .contains("no agent is running")
        );
        let ssh_not_held = "error: Couldn't find key in agent?\n\nerror: unable to sign the tag";
        assert!(
            signing_failure(ssh_not_held, None)
                .unwrap()
                .contains("does not have the signing key")
        );
        let ssh_half = "error: No private key found for public key \"/keys/key.pub\"?\n\
            \n\
            fatal: failed to write commit object";
        let said = signing_failure(ssh_half, None).unwrap();
        assert!(said.contains("no private key for /keys/key.pub"), "{said}");
        let ssh_locked = "error: Enter passphrase for \"/keys/locked\": Load key \"/keys/locked\": incorrect passphrase supplied to decrypt private key?\n\
            \n\
            fatal: failed to write commit object";
        assert!(
            signing_failure(ssh_locked, None)
                .unwrap()
                .contains("passphrase")
        );

        let format = "error: invalid value for 'gpg.format': 'nonsense'\n\
            fatal: bad config variable 'gpg.format' in file '.git/config' at line 7";
        assert!(
            signing_failure(format, None)
                .unwrap()
                .contains("\"nonsense\"")
        );

        // A key on a smart card, as git 2.56 hands on what GnuPG 2.4 said
        // with an emulated card; the card itself is asked elsewhere.
        let bad_pin = no_pinentry.replace("No pinentry", "Bad PIN");
        let said =
            signing_failure(&bad_pin, Some("7A6929A8BA898A56619E701F05CD90DC7EC9BFBD")).unwrap();
        assert!(
            said.contains("did not take the PIN (Bad PIN)") && said.contains("Admin PIN"),
            "{said}"
        );
        for (why, words) in [
            ("PIN blocked", "is blocked"),
            ("No SmartCard daemon", "could not start scdaemon"),
            ("Card removed", "not in a reader"),
            ("Card not present", "not in a reader"),
        ] {
            let said = signing_failure(&no_pinentry.replace("No pinentry", why), None).unwrap();
            assert!(said.contains(words), "{why}: {said}");
        }

        // Signing that failed for a reason not known here: its own word.
        let other = "error: gpg failed to sign the data:\n\
            gpg: signing failed: Card error\n\
            [GNUPG:] FAILURE sign 100\n\
            \n\
            fatal: failed to write commit object";
        let said = signing_failure(other, None).unwrap();
        assert!(said.contains("gpg: signing failed: Card error"), "{said}");
        let mute = "error: gpg failed to sign the data:\n(no gpg output)\nfatal: failed to write commit object";
        assert!(
            signing_failure(mute, None)
                .unwrap()
                .contains("did not say why")
        );

        // Failures that are not about signing stay what they are.
        for said in [
            "",
            "nothing to commit, working tree clean",
            "fatal: failed to write commit object",
            "error: unable to write file .git/objects/ab/cd: No space left on device\nfatal: failed to write commit object",
            "error: cannot run .git/hooks/pre-commit: No such file or directory",
            "fatal: tag 'v1' already exists",
            "husky: gpg: signing failed: No secret key",
            // A hook that talks about keys, and a commit that then could
            // not be written for another reason.
            "pre-commit: No secret key in the vault\nfatal: failed to write commit object",
            "\0\n\n\0",
        ] {
            assert_eq!(signing_failure(said, None), None, "{said:?}");
        }
        // Cut anywhere, it is still only read.
        for cut in 0..no_key.len() {
            let _ = signing_failure(no_key.get(..cut).unwrap_or(""), None);
        }
    }

    #[test]
    fn what_git_gave_up_over_is_one_line() {
        assert_eq!(
            complaint("warning: something\nfatal: bad/incompatible signature 'garbage\n'\n"),
            "bad/incompatible signature 'garbage"
        );
        assert_eq!(
            complaint("error: this\nerror: and that\n\n"),
            "error: and that"
        );
        assert_eq!(complaint(""), "it gave no reason");
        assert_eq!(complaint("\n \n"), "it gave no reason");
        let long = format!("fatal: {}", "é".repeat(5000));
        assert_eq!(complaint(&long).chars().count(), MAX_REASON);
    }

    #[test]
    fn scopes_have_two_names_and_no_others() {
        assert_eq!(Scope::parse("repo"), Some(Scope::Repo));
        assert_eq!(Scope::parse("user"), Some(Scope::User));
        for other in [
            "", "Repo", "USER", "global", "local", "system", "--global", "user ",
        ] {
            assert_eq!(Scope::parse(other), None, "{other:?}");
        }
    }
}
