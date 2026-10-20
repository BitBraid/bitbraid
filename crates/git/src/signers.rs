//! The allowed-signers file: whose SSH keys count when git checks a
//! signature made with one. Where it is, who is in it, a signer added
//! (from a public key, a key file, or the key a commit was signed with)
//! and a signer taken out.
//!
//! Git hands the file to `ssh-keygen -Y find-principals` and `-Y verify`
//! and takes their word, so nothing here judges a signature: a signer is
//! written as a line ssh-keygen reads, and the verdict stays git's
//! (`crate::signature`). The key of a commit is read out of its signature,
//! which carries the public key it was made with.
//!
//! The file is the user's list of whom they trust and is usually shared
//! by all their repositories. Every line that is not understood here, and
//! every comment, is left exactly as it is.
//!
//! Sources: ssh-keygen(1) ("ALLOWED SIGNERS": a line is principals,
//! options, key type and key; the options `namespaces=`, `cert-authority`,
//! `valid-after=`, `valid-before=`; `-Y find-principals`, `-Y verify`,
//! `-l` for the form of a fingerprint); OpenSSH's PROTOCOL.sshsig (what an
//! armoured signature holds: a magic word, a version, the public key);
//! RFC 4251 section 5 (`uint32`, `string`) and RFC 4253 section 6.6 (a
//! public key begins with the name of its type); RFC 4648 (base64);
//! git-config(1) (`gpg.ssh.allowedSignersFile`: a path, taken from the top
//! of the worktree when it is relative; `--type=path`, `--show-scope`,
//! `--file`); gitformat-signature(5) (the `gpgsig` header).

use std::{
    ffi::OsString,
    fs,
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    Error, Oid, Repository, Result, exe,
    identity::{self, Scope},
};

/// The setting that names the file.
pub const SETTING: &str = "gpg.ssh.allowedSignersFile";
/// The namespace git signs in, and the one a signer added here is good for.
const NAMESPACE: &str = "git";
/// Most of the file that is read, in bytes.
const MAX_FILE: u64 = 4 * 1024 * 1024;
/// Most signers that are listed.
const MAX_LISTED: usize = 5000;
/// Longest principal: an e-mail address is no longer.
const MAX_PRINCIPAL: usize = 320;
/// Most principals of one signer.
const MAX_PRINCIPALS: usize = 32;
/// Longest path of the file, in bytes.
const MAX_PATH: usize = 4096;
/// Longest public key in base64: an RSA key of 16,384 bits is a fifth.
const MAX_KEY: usize = 16 * 1024;
/// Most of a public key file that is read, in bytes.
const MAX_KEY_FILE: u64 = 64 * 1024;
/// Longest signature taken from a commit, in bytes.
const MAX_SIGNATURE: usize = 64 * 1024;

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("signers-what-allowed-signers"),
        detail: detail.into(),
    }
}

fn io(context: &'static str, err: std::io::Error) -> Error {
    Error::Git {
        context,
        source: Box::new(err),
    }
}

// --- Base64 -----------------------------------------------------------------

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// The six bits a base64 character stands for.
fn sextet(c: u8) -> Option<u32> {
    match c {
        b'A'..=b'Z' => Some(u32::from(c - b'A')),
        b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
        b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

/// Decode base64 as ssh writes it: the standard alphabet, padded or not,
/// with nothing else in between.
fn decode(text: &str) -> Option<Vec<u8>> {
    let body = text.trim_end_matches('=');
    // Padded, it is padded to a multiple of four; not padded, a single
    // character left over stands for nothing.
    let padding = text.len().saturating_sub(body.len());
    if (padding > 0 && !text.len().is_multiple_of(4)) || padding > 2 || body.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(body.len() / 4 * 3 + 2);
    let (mut bits, mut held) = (0u32, 0u32);
    for c in body.bytes() {
        bits = (bits << 6) | sextet(c)?;
        held += 6;
        if held >= 8 {
            held -= 8;
            out.push(u8::try_from((bits >> held) & 0xff).ok()?);
        }
    }
    // What is left over has to be nothing: a stray bit is not base64.
    if bits & ((1u32 << held) - 1) != 0 {
        return None;
    }
    Some(out)
}

/// Encode as base64, padded as ssh pads a key, or not, as it writes a
/// fingerprint.
fn encode(bytes: &[u8], padded: bool) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut group = 0u32;
        for (ix, byte) in chunk.iter().enumerate() {
            group |= u32::from(*byte) << (16 - 8 * ix);
        }
        let letters = chunk.len() + 1;
        for ix in 0..4 {
            if ix < letters {
                let six = usize::try_from((group >> (18 - 6 * ix)) & 63).unwrap_or(0);
                out.push(char::from(ALPHABET.get(six).copied().unwrap_or(b'A')));
            } else if padded {
                out.push('=');
            }
        }
    }
    out
}

// --- Keys -------------------------------------------------------------------

/// Reads the `uint32` and `string` of the SSH wire format, never past the
/// end of what there is.
struct Wire<'a> {
    rest: &'a [u8],
}

impl<'a> Wire<'a> {
    fn take(&mut self, count: usize) -> Option<&'a [u8]> {
        let (taken, rest) = self.rest.split_at_checked(count)?;
        self.rest = rest;
        Some(taken)
    }

    fn uint32(&mut self) -> Option<u32> {
        let bytes: [u8; 4] = self.take(4)?.try_into().ok()?;
        Some(u32::from_be_bytes(bytes))
    }

    fn string(&mut self) -> Option<&'a [u8]> {
        let length = usize::try_from(self.uint32()?).ok()?;
        self.take(length)
    }
}

/// Whether this is the name of a kind of key that can stand in the file
/// by itself. A certificate cannot: its authority's key does, marked
/// `cert-authority`.
fn known_kind(kind: &str) -> bool {
    matches!(
        kind,
        "ssh-ed25519"
            | "ssh-rsa"
            | "ssh-dss"
            | "ecdsa-sha2-nistp256"
            | "ecdsa-sha2-nistp384"
            | "ecdsa-sha2-nistp521"
            | "sk-ssh-ed25519@openssh.com"
            | "sk-ecdsa-sha2-nistp256@openssh.com"
    )
}

/// A public key: the name of its kind and the key as ssh sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PublicKey {
    kind: String,
    blob: Vec<u8>,
}

impl PublicKey {
    /// From the key as ssh sends it, which begins with its kind.
    fn from_blob(blob: &[u8]) -> Option<Self> {
        let mut wire = Wire { rest: blob };
        let kind = std::str::from_utf8(wire.string()?).ok()?;
        if !known_kind(kind) {
            return None;
        }
        // After its kind a key is a run of strings (numbers, a curve's
        // name, a point) that ends where the key ends: one that was cut
        // short, or has something hanging on, is not a key.
        let mut parts = 0usize;
        let mut first = None;
        while !wire.rest.is_empty() {
            let part = wire.string()?;
            first.get_or_insert(part.len());
            parts = parts.saturating_add(1);
        }
        let whole = match kind {
            "ssh-ed25519" => parts == 1 && first == Some(32),
            _ => parts >= 2,
        };
        whole.then(|| Self {
            kind: kind.to_owned(),
            blob: blob.to_vec(),
        })
    }

    /// From the two words of a public key file or of a line of the
    /// allowed signers: the kind, and the key in base64, which has to be
    /// of that kind.
    fn from_words(kind: &str, key: &str) -> Option<Self> {
        if key.len() > MAX_KEY {
            return None;
        }
        let found = Self::from_blob(&decode(key)?)?;
        (found.kind == kind).then_some(found)
    }

    fn base64(&self) -> String {
        encode(&self.blob, true)
    }

    /// The fingerprint as `ssh-keygen -l` and git print it.
    fn fingerprint(&self) -> String {
        let mut hasher = gix::hash::hasher(gix::hash::Kind::Sha256);
        hasher.update(&self.blob);
        match hasher.try_finalize() {
            Ok(digest) => format!("SHA256:{}", encode(digest.as_bytes(), false)),
            Err(_) => String::new(),
        }
    }
}

/// The public key in a text: the first line that is one, as a `.pub` file
/// or `user.signingKey` has it (`kind key comment`, or `key::kind key`).
fn key_in_text(text: &str) -> Result<PublicKey> {
    let what = say!("signers-what-public-key");
    if text.contains("PRIVATE KEY-----") {
        return Err(invalid(what, say!("signers-private-key")));
    }
    text.lines()
        .map(|line| line.trim().strip_prefix("key::").unwrap_or(line.trim()))
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .find_map(|line| {
            let mut words = line.split_whitespace();
            PublicKey::from_words(words.next()?, words.next()?)
        })
        .ok_or_else(|| invalid(what, say!("signers-no-public-key")))
}

/// The public key an armoured SSH signature was made with: the signature
/// carries it.
fn key_of_signature(armor: &str) -> Result<PublicKey> {
    let what = say!("signers-what-signature");
    let mut lines = armor.lines().map(str::trim);
    if lines.next() != Some("-----BEGIN SSH SIGNATURE-----") {
        return Err(invalid(what, say!("signers-not-ssh-signature")));
    }
    let body: String = lines
        .take_while(|line| *line != "-----END SSH SIGNATURE-----")
        .collect();
    let broken = || invalid(what, say!("signers-signature-unreadable"));
    let bytes = decode(&body).ok_or_else(broken)?;
    let mut wire = Wire { rest: &bytes };
    if wire.take(6) != Some(b"SSHSIG".as_slice()) || wire.uint32() != Some(1) {
        return Err(broken());
    }
    let blob = wire.string().ok_or_else(broken)?;
    PublicKey::from_blob(blob).ok_or_else(|| invalid(what, say!("signers-signature-key-unknown")))
}

/// The signature a commit carries, whole, and the address of whoever
/// committed it.
fn signature_of(repo: &Repository, id: Oid) -> Result<(String, String)> {
    let r = repo.gix();
    let commit = r.find_commit(*id.as_gix()).map_err(|_| {
        invalid(
            say!("signers-what-commit"),
            say!("signers-not-a-commit", id = id.to_hex()),
        )
    })?;
    let header: &[u8] = match r.object_hash() {
        gix::hash::Kind::Sha1 => b"gpgsig",
        _ => b"gpgsig-sha256",
    };
    let mut found: Option<Vec<u8>> = None;
    for line in commit.data.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            break;
        }
        match &mut found {
            // The lines of a signature after its first begin with a space.
            Some(armor) => match line.strip_prefix(b" ") {
                Some(more) => {
                    armor.push(b'\n');
                    armor.extend_from_slice(more);
                }
                None => break,
            },
            None => {
                if let Some(first) = line
                    .strip_prefix(header)
                    .and_then(|rest| rest.strip_prefix(b" "))
                {
                    found = Some(first.to_vec());
                }
            }
        }
        if found
            .as_ref()
            .is_some_and(|armor| armor.len() > MAX_SIGNATURE)
        {
            return Err(invalid(
                say!("signers-what-signature"),
                say!("signers-too-long"),
            ));
        }
    }
    let armor = found.ok_or_else(|| refused(say!("signers-not-signed", id = id.short(7))))?;
    let address = commit
        .committer()
        .map(|who| who.email.to_string())
        .unwrap_or_default();
    Ok((String::from_utf8_lossy(&armor).into_owned(), address))
}

// --- The file ---------------------------------------------------------------

/// Where the allowed signers are kept.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Place {
    /// The setting as it is written; absent when there is none, or an
    /// empty one.
    pub configured: Option<String>,
    /// The file it names.
    #[serde(serialize_with = "crate::opt_path_text")]
    pub path: Option<PathBuf>,
    /// Whether that file is there.
    pub exists: bool,
    /// Which configuration says so: `repo`, `user`, or git's own word for
    /// another (`system`, `worktree`, `command`).
    pub scope: Option<String>,
}

/// One line of the file: somebody who may sign, and their key.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Signer {
    /// The line it is on, counted from one.
    pub line: usize,
    /// Who: usually one e-mail address; patterns as ssh has them.
    pub principals: Vec<String>,
    /// What the line says besides: `namespaces="git"`, `cert-authority`,
    /// `valid-before=…`.
    pub options: Vec<String>,
    /// The kind of key (`ssh-ed25519`).
    pub kind: String,
    /// The key itself, in base64.
    pub key: String,
    /// Its fingerprint, as git names the key of a signature.
    pub fingerprint: String,
    /// What follows the key on the line.
    pub comment: Option<String>,
}

/// The allowed signers.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Signers {
    /// Where they are kept.
    pub file: Place,
    /// Who is in it, in the order of the file.
    pub signers: Vec<Signer>,
    /// Lines that are neither a signer, a comment nor empty.
    pub skipped: usize,
    /// Signers beyond the ones listed.
    pub more: usize,
}

/// The file as the configuration names it.
pub fn place(repo: &Repository) -> Place {
    let shown = exe::run(repo, ["config", "--show-scope", "--get", SETTING], None)
        .ok()
        .map(|out| out.stdout.trim_end_matches(['\n', '\r']).to_owned())
        .unwrap_or_default();
    let (scope, configured) = match shown.split_once('\t') {
        Some((scope, value)) if !value.is_empty() => (
            Some(match scope {
                "local" => "repo".to_owned(),
                "global" => "user".to_owned(),
                other => other.to_owned(),
            }),
            Some(value.to_owned()),
        ),
        _ => (None, None),
    };
    let path = configured
        .as_ref()
        .and_then(|_| exe::config_path(repo, SETTING));
    Place {
        exists: path.as_deref().is_some_and(Path::is_file),
        configured,
        path,
        scope,
    }
}

/// The words of a line: split at blanks, except inside double quotes.
fn words(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut quoted) = (None, false);
    for (at, c) in line.char_indices() {
        if c == '"' {
            quoted = !quoted;
        }
        if c.is_whitespace() && !quoted {
            if let Some(from) = start.take() {
                out.extend(line.get(from..at));
            }
        } else if start.is_none() {
            start = Some(at);
        }
    }
    if let Some(from) = start {
        out.extend(line.get(from..));
    }
    out
}

/// Split at commas, except inside double quotes.
fn listed(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut from, mut quoted) = (0, false);
    for (at, c) in text.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                out.extend(text.get(from..at).map(str::to_owned));
                from = at.saturating_add(1);
            }
            _ => {}
        }
    }
    out.extend(text.get(from..).map(str::to_owned));
    out.retain(|part| !part.is_empty());
    out
}

/// A line of the file that names a signer, taken apart.
struct Line {
    principals: Vec<String>,
    options: Vec<String>,
    key: PublicKey,
    comment: Option<String>,
}

/// A line of the file read as a signer; `None` for a comment, an empty
/// line and anything that is not one.
fn signer_of(line: &str) -> Option<Line> {
    let words = words(line);
    let first = *words.first()?;
    if first.starts_with('#') {
        return None;
    }
    // The kind of key is the second word, or the third after options.
    let (at, key) = [1usize, 2].into_iter().find_map(|at| {
        let key = PublicKey::from_words(words.get(at)?, words.get(at.checked_add(1)?)?)?;
        Some((at, key))
    })?;
    let options = match at {
        2 => listed(words.get(1)?),
        _ => Vec::new(),
    };
    let comment = words
        .get(at.saturating_add(2)..)
        .filter(|rest| !rest.is_empty())
        .map(|rest| rest.join(" "));
    let principals = listed(first.trim_matches('"'));
    (!principals.is_empty()).then_some(Line {
        principals,
        options,
        key,
        comment,
    })
}

/// Read the file whole, when it is one and no larger than is read.
fn read(path: &Path) -> Result<Option<String>> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(io(say!("signers-looking-at-file"), err)),
    };
    // A link is followed: the file is often kept elsewhere and linked.
    let meta = if meta.is_symlink() {
        match fs::metadata(path) {
            Ok(meta) => meta,
            Err(_) => return Ok(None),
        }
    } else {
        meta
    };
    if !meta.is_file() {
        return Err(refused(say!(
            "signers-not-a-file",
            path = path.display().to_string()
        )));
    }
    if meta.len() > MAX_FILE {
        return Err(refused(say!(
            "signers-file-too-large",
            path = path.display().to_string(),
            mib = MAX_FILE / (1024 * 1024)
        )));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|file| file.take(MAX_FILE).read_to_end(&mut bytes))
        .map_err(|err| io(say!("signers-reading-file"), err))?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| refused(say!("signers-not-text", path = path.display().to_string())))
}

/// Who is among the allowed signers, and where the file is. No file, or
/// none configured, is nobody.
pub fn list(repo: &Repository) -> Result<Signers> {
    let file = place(repo);
    let text = match &file.path {
        Some(path) => read(path)?.unwrap_or_default(),
        None => String::new(),
    };
    let (mut signers, mut skipped, mut more) = (Vec::new(), 0usize, 0usize);
    for (ix, line) in text.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some(Line {
            principals,
            options,
            key,
            comment,
        }) = signer_of(trimmed)
        else {
            skipped = skipped.saturating_add(1);
            continue;
        };
        if signers.len() >= MAX_LISTED {
            more = more.saturating_add(1);
            continue;
        }
        signers.push(Signer {
            line: ix.saturating_add(1),
            principals,
            options,
            fingerprint: key.fingerprint(),
            key: key.base64(),
            kind: key.kind,
            comment,
        });
    }
    Ok(Signers {
        file,
        signers,
        skipped,
        more,
    })
}

/// Write the file anew beside itself and put it in its place, so that it
/// is whole at every moment. A link is written through, not replaced.
fn write(path: &Path, text: &str) -> Result<()> {
    let target = bitbraid_core::paths::canonical(path).unwrap_or_else(|_| path.to_path_buf());
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(refused(say!(
            "signers-cannot-write",
            path = path.display().to_string()
        )));
    };
    let mut beside = OsString::from(".");
    beside.push(name);
    beside.push(format!(".{}.tmp", std::process::id()));
    let beside = dir.join(beside);
    let written = (|| {
        let mut file = fs::File::create(&beside)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        // Who may read it stays who could.
        if let Ok(meta) = fs::metadata(&target) {
            let _ = fs::set_permissions(&beside, meta.permissions());
        }
        fs::rename(&beside, &target)
    })();
    written.map_err(|err| {
        let _ = fs::remove_file(&beside);
        io(say!("signers-writing-file"), err)
    })
}

/// Check one principal: what ssh matches a signer's name against.
fn check_principal(principal: &str) -> Result<()> {
    let what = say!("signers-what-signer");
    if principal.is_empty() {
        return Err(invalid(what, say!("signers-say-who")));
    }
    if principal.len() > MAX_PRINCIPAL {
        return Err(invalid(what, say!("signers-name-too-long")));
    }
    if principal.starts_with(['#', '-'])
        || principal
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == ',')
    {
        return Err(invalid(
            what,
            say!(
                "signers-principal-unwritable",
                principal = format!("{principal:?}")
            ),
        ));
    }
    Ok(())
}

/// Where the key of a new signer comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyFrom {
    /// The public key itself, as a `.pub` file has it.
    Text(String),
    /// A public key file, by its full path.
    File(PathBuf),
    /// The key this commit was signed with.
    Commit(Oid),
}

/// A change to the lines of an allowed-signers file, kept so that it can be
/// taken back and made again: the lines that came and the lines that
/// went, each as it stood. Nothing else of the file is kept, which is
/// somebody's own list of whom they trust.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Edit {
    pub file: String,
    /// The lines put in at the end, each with its ending.
    #[serde(default)]
    pub added: Vec<String>,
    /// The lines taken out: where each stood among the lines there were
    /// (from 0), and the line with its ending.
    #[serde(default)]
    pub removed: Vec<(usize, String)>,
    /// The last line there was had no ending and was given one.
    #[serde(default)]
    pub mended: bool,
    /// The file was not there and was made.
    #[serde(default)]
    pub created: bool,
}

/// The content with a change taken back, or `None` when it is not as the
/// change left it: a line that was put in is gone, or one that was taken
/// out is there again.
fn undone(text: &str, edit: &Edit) -> Option<String> {
    let mut lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut gave_ending = false;
    for added in edit.added.iter().rev() {
        let at = lines.iter().rposition(|line| *line == added.as_str())?;
        // Only what was the last line can have given the one before it
        // its ending.
        gave_ending = edit.mended && at.saturating_add(1) == lines.len();
        lines.remove(at);
    }
    if edit
        .removed
        .iter()
        .any(|(_, gone)| lines.contains(&gone.as_str()))
    {
        return None;
    }
    let mut out: Vec<String> = lines.into_iter().map(str::to_owned).collect();
    if gave_ending
        && let Some(last) = out.last_mut()
        && let Some(bare) = last.strip_suffix('\n')
    {
        *last = bare.to_owned();
    }
    for (at, line) in &edit.removed {
        let at = (*at).min(out.len());
        // A line put back after one that has no ending would run into it.
        if let Some(before) = at.checked_sub(1).and_then(|before| out.get_mut(before))
            && !before.ends_with('\n')
        {
            before.push('\n');
        }
        // And one that had no ending, being the last, needs one where
        // more follows it now.
        let mut line = line.clone();
        if at < out.len() && !line.ends_with('\n') {
            line.push('\n');
        }
        out.insert(at, line);
    }
    Some(out.concat())
}

/// The content with a change made again, or `None` when it cannot be: a
/// line to take out is not there, or one to put in is there already.
fn redone(text: &str, edit: &Edit) -> Option<String> {
    let mut lines: Vec<&str> = text.split_inclusive('\n').collect();
    // From the last, so that the places of the others stay what they were.
    for (at, gone) in edit.removed.iter().rev() {
        let at = match lines.get(*at) {
            Some(line) if *line == gone.as_str() => *at,
            _ => lines.iter().position(|line| *line == gone.as_str())?,
        };
        lines.remove(at);
    }
    if edit
        .added
        .iter()
        .any(|added| lines.contains(&added.as_str()))
    {
        return None;
    }
    let mut out = lines.concat();
    if !edit.added.is_empty() && !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    for added in &edit.added {
        out.push_str(added);
    }
    Some(out)
}

fn not_as_left(edit: &Edit) -> Error {
    refused(say!("signers-not-as-left", file = edit.file.as_str()))
}

/// Take a change to an allowed-signers file back: the lines it put in go,
/// the lines it took out come back where they stood. Refused, and nothing
/// written, when the file no longer holds what the change left there. A
/// file the change made, and that is empty again, is removed.
pub fn take_back(edit: &Edit) -> Result<()> {
    let path = PathBuf::from(&edit.file);
    let text = read(&path)?.ok_or_else(|| not_as_left(edit))?;
    let before = undone(&text, edit).ok_or_else(|| not_as_left(edit))?;
    if edit.created && before.is_empty() {
        return fs::remove_file(&path).map_err(|err| io(say!("signers-removing-file"), err));
    }
    write(&path, &before)
}

/// Make a change to an allowed-signers file again after it was taken
/// back. Refused, and nothing written, when a line it takes out is not
/// there or one it puts in is.
pub fn make_again(edit: &Edit) -> Result<()> {
    let path = PathBuf::from(&edit.file);
    let text = match read(&path)? {
        Some(text) => text,
        None if edit.created => String::new(),
        None => return Err(not_as_left(edit)),
    };
    let after = redone(&text, edit).ok_or_else(|| not_as_left(edit))?;
    write(&path, &after)
}

/// What allowing a signer did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Allowed {
    /// The file it was written to.
    #[serde(serialize_with = "crate::path_text")]
    pub file: PathBuf,
    /// Who.
    pub principals: Vec<String>,
    /// The kind of key.
    pub kind: String,
    pub fingerprint: String,
    /// False when exactly this signer with this key was there already.
    pub added: bool,
    /// What was written, for undo. None when nothing was, or when the
    /// file's path is not text.
    #[serde(skip)]
    pub edit: Option<Edit>,
}

/// The content with a signer added at the end, or `None` when a line says
/// the same already: the same key for the same principals.
fn with_signer(text: &str, principals: &[String], key: &PublicKey) -> Option<String> {
    let mut wanted: Vec<&str> = principals.iter().map(String::as_str).collect();
    wanted.sort_unstable();
    let there = text.lines().any(|line| {
        signer_of(line.trim()).is_some_and(|found| {
            let mut names = found.principals;
            names.sort_unstable();
            found.key == *key && names == wanted
        })
    });
    if there {
        return None;
    }
    let mut out = text.to_owned();
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!(
        "{} namespaces=\"{NAMESPACE}\" {} {}\n",
        principals.join(","),
        key.kind,
        key.base64()
    ));
    Some(out)
}

/// Add a signer to the allowed-signers file the configuration names,
/// making the file when it is not there yet. Without principals the key
/// of a commit is allowed for the address of whoever committed it.
pub fn allow(repo: &Repository, principals: &[String], from: &KeyFrom) -> Result<Allowed> {
    let (key, committer) = match from {
        KeyFrom::Text(text) => {
            if text.len() > MAX_KEY.saturating_mul(2) {
                return Err(invalid(
                    say!("signers-what-public-key"),
                    say!("signers-too-long"),
                ));
            }
            (key_in_text(text)?, None)
        }
        KeyFrom::File(path) => {
            let named = path.to_str().unwrap_or_default();
            if named.is_empty()
                || named.len() > MAX_PATH
                || named.chars().any(char::is_control)
                || !path.is_absolute()
            {
                return Err(invalid(
                    say!("signers-what-key-file"),
                    say!("signers-key-file-full-path"),
                ));
            }
            if !path.is_file() {
                return Err(refused(say!(
                    "signers-not-a-file",
                    path = path.display().to_string()
                )));
            }
            let mut bytes = Vec::new();
            fs::File::open(path)
                .and_then(|file| file.take(MAX_KEY_FILE).read_to_end(&mut bytes))
                .map_err(|err| io(say!("signers-reading-key-file"), err))?;
            (key_in_text(&String::from_utf8_lossy(&bytes))?, None)
        }
        KeyFrom::Commit(id) => {
            let (armor, address) = signature_of(repo, *id)?;
            (key_of_signature(&armor)?, Some(address))
        }
    };
    let principals: Vec<String> = match (principals.is_empty(), committer) {
        (true, Some(address)) => vec![address],
        _ => principals.to_vec(),
    };
    if principals.is_empty() {
        return Err(invalid(
            say!("signers-what-signer"),
            say!("signers-say-who"),
        ));
    }
    if principals.len() > MAX_PRINCIPALS {
        return Err(invalid(
            say!("signers-what-signer"),
            say!("signers-too-many-names"),
        ));
    }
    for principal in &principals {
        check_principal(principal)?;
    }
    let Some(path) = place(repo).path else {
        return Err(refused(say!("signers-no-file-choose", setting = SETTING)));
    };
    let there = read(&path)?;
    let created = there.is_none();
    let text = there.unwrap_or_default();
    let (added, edit) = match with_signer(&text, &principals, &key) {
        Some(new) => {
            write(&path, &new)?;
            // What was put after what there was: an ending for a last
            // line without one, and the signer's line.
            let mended = !text.is_empty() && !text.ends_with('\n');
            let from = text.len().saturating_add(usize::from(mended));
            let edit = path.to_str().zip(new.get(from..)).map(|(file, line)| Edit {
                file: file.to_owned(),
                added: vec![line.to_owned()],
                removed: Vec::new(),
                mended,
                created,
            });
            (true, edit)
        }
        None => (false, None),
    };
    Ok(Allowed {
        file: path,
        principals,
        fingerprint: key.fingerprint(),
        kind: key.kind,
        added,
        edit,
    })
}

/// What taking a signer out did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Disallowed {
    /// The file it was taken out of.
    #[serde(serialize_with = "crate::path_text")]
    pub file: PathBuf,
    /// How many lines went.
    pub removed: usize,
    /// What was taken out, for undo. None when the file's path is not
    /// text.
    #[serde(skip)]
    pub edit: Option<Edit>,
}

/// The content without the lines of a key, named by its fingerprint or in
/// base64, and how many those were. With `principals`, only the lines for
/// exactly those. Every other line stays as it is, to the letter.
#[cfg(test)]
fn without_signer(text: &str, key: &str, principals: Option<&[String]>) -> (String, usize) {
    let (out, gone) = taken_out(text, key, principals);
    (out, gone.len())
}

/// [`without_signer`], with the lines that went: where each stood among
/// the lines of the file, and the line as it was.
fn taken_out(
    text: &str,
    key: &str,
    principals: Option<&[String]>,
) -> (String, Vec<(usize, String)>) {
    let wanted = principals.map(|names| {
        let mut names: Vec<&str> = names.iter().map(String::as_str).collect();
        names.sort_unstable();
        names
    });
    let mut removed = Vec::new();
    let mut out = String::with_capacity(text.len());
    for (at, line) in text.split_inclusive('\n').enumerate() {
        let goes = signer_of(line.trim()).is_some_and(|found| {
            let mut names = found.principals;
            names.sort_unstable();
            (found.key.fingerprint() == key || found.key.base64() == key)
                && wanted.as_ref().is_none_or(|wanted| names == *wanted)
        });
        if goes {
            removed.push((at, line.to_owned()));
        } else {
            out.push_str(line);
        }
    }
    (out, removed)
}

/// Take a signer out of the allowed-signers file: every line with this
/// key (its fingerprint, or the key in base64), or only the one for
/// exactly these principals.
pub fn disallow(repo: &Repository, key: &str, principals: Option<&[String]>) -> Result<Disallowed> {
    let key = key.trim();
    if key.is_empty() || key.len() > MAX_KEY || key.chars().any(char::is_whitespace) {
        return Err(invalid(
            say!("signers-what-key"),
            say!("signers-give-fingerprint"),
        ));
    }
    let Some(path) = place(repo).path else {
        return Err(refused(say!("signers-no-file", setting = SETTING)));
    };
    let Some(text) = read(&path)? else {
        return Err(refused(say!(
            "signers-not-there",
            path = path.display().to_string()
        )));
    };
    let (left, gone) = taken_out(&text, key, principals);
    if gone.is_empty() {
        return Err(refused(say!(
            "signers-no-such-signer",
            path = path.display().to_string()
        )));
    }
    write(&path, &left)?;
    let removed = gone.len();
    let edit = path.to_str().map(|file| Edit {
        file: file.to_owned(),
        added: Vec::new(),
        removed: gone,
        mended: false,
        created: false,
    });
    Ok(Disallowed {
        file: path,
        removed,
        edit,
    })
}

/// What choosing the file did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Chosen {
    /// The setting as it was written.
    pub configured: String,
    /// The file it names.
    #[serde(serialize_with = "crate::path_text")]
    pub path: PathBuf,
    /// Whether the file was made: it was not there.
    pub created: bool,
}

/// The file kept for one repository alone when none is named: beside its
/// configuration, where every worktree of it finds it.
pub fn file_of_repository(repo: &Repository) -> PathBuf {
    repo.gix().common_dir().join("allowed_signers")
}

/// Name the allowed-signers file in the repository's configuration, the
/// user's or a configuration file, and make the file when it is not there
/// (empty: nobody is allowed yet). The path is written as it is given, so
/// `~/…` stays that; a relative one counts from the top of the worktree.
pub fn use_file(repo: &Repository, path: &str, scope: &Scope) -> Result<Chosen> {
    let what = say!("signers-what-file");
    let path = path.trim();
    if path.is_empty() {
        return Err(invalid(what, say!("signers-say-where")));
    }
    if path.len() > MAX_PATH || path.starts_with('-') || path.chars().any(char::is_control) {
        return Err(invalid(
            what,
            say!("signers-bad-path", path = format!("{path:?}")),
        ));
    }
    identity::check_scope(Some(repo), scope)?;
    let config = |more: &[&str]| {
        let mut args: Vec<OsString> = vec!["config".into()];
        args.extend(identity::scope_args(scope));
        args.extend(more.iter().map(OsString::from));
        exe::run_with_code(repo, args, None)
    };
    let before = match config(&["--get", SETTING])? {
        (Some(0), out) => Some(out.stdout.trim_end_matches(['\n', '\r']).to_owned()),
        _ => None,
    };
    let put_back = || match &before {
        Some(old) => config(&["--replace-all", SETTING, old]).map(|_| ()),
        None => config(&["--unset-all", SETTING]).map(|_| ()),
    };
    match config(&["--replace-all", SETTING, path])? {
        (Some(0), _) => {}
        (_, out) => {
            return Err(refused(say!(
                "signers-setting-not-written",
                stderr = out.stderr.trim()
            )));
        }
    }
    // As git reads it back from there: `~` filled in.
    let resolved = match config(&["--type=path", "--get", SETTING]) {
        Ok((Some(0), out)) => PathBuf::from(out.stdout.trim_end_matches(['\n', '\r'])),
        _ => PathBuf::from(path),
    };
    let resolved = if resolved.is_absolute() {
        resolved
    } else {
        match repo.workdir() {
            Some(top) => top.join(resolved),
            None => {
                let _ = put_back();
                return Err(refused(say!("signers-no-worktree")));
            }
        }
    };
    let made = (|| -> std::io::Result<bool> {
        if resolved.is_file() {
            return Ok(false);
        }
        if let Some(dir) = resolved.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&resolved)?;
        Ok(true)
    })();
    match made {
        Ok(created) => Ok(Chosen {
            configured: path.to_owned(),
            path: resolved,
            created,
        }),
        Err(err) => {
            // A setting that names a file that cannot be is worse than
            // the one before.
            let _ = put_back();
            Err(refused(say!(
                "signers-cannot-make",
                path = resolved.display().to_string(),
                error = err.to_string()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Line, PublicKey, Wire, decode, encode, key_in_text, key_of_signature, known_kind, listed,
        signer_of, with_signer, without_signer, words,
    };

    /// The public halves of two ed25519 keys made for these tests; the
    /// private halves were deleted as they were made.
    const KEY: &str = "AAAAC3NzaC1lZDI1NTE5AAAAINBvEivhFTA8QiNTh5Dc8w5ak5Ep6BH0UpKZPXwB9XXH";
    const OTHER: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIGNEZG62DLSWCIVWvHcvrpayQZpY5B46Zyc2+ir+Bzg0";

    #[test]
    fn base64_goes_there_and_back() {
        for (plain, coded) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
            (&[0xff, 0xfe, 0xfd], "//79"),
        ] {
            assert_eq!(encode(plain, true), coded);
            assert_eq!(decode(coded).as_deref(), Some(plain));
            // Unpadded, as a fingerprint is written, reads the same.
            let bare = coded.trim_end_matches('=');
            assert_eq!(encode(plain, false), bare);
            assert_eq!(decode(bare).as_deref(), Some(plain));
        }
        for broken in [
            "Z", "Zg=", "Zm9v=", "Zg===", "Z g=", "Zm9v\n", "Zm9*", "Zh==", "Zm9=",
        ] {
            assert_eq!(decode(broken), None, "{broken:?}");
        }
    }

    #[test]
    fn the_wire_is_never_read_past_its_end() {
        let mut wire = Wire {
            rest: &[0, 0, 0, 2, b'h', b'i', 0, 0, 0, 9, b'x'],
        };
        assert_eq!(wire.string(), Some(&b"hi"[..]));
        // A length that is more than what is left.
        assert_eq!(wire.string(), None);
        let mut wire = Wire {
            rest: &[0xff, 0xff, 0xff, 0xff, 1],
        };
        assert_eq!(wire.string(), None);
        assert_eq!(Wire { rest: &[1, 2] }.uint32(), None);
    }

    #[test]
    fn a_key_is_read_from_its_words_and_named_by_its_fingerprint() {
        let key = PublicKey::from_words("ssh-ed25519", KEY).unwrap();
        assert_eq!(key.kind, "ssh-ed25519");
        assert_eq!(key.base64(), KEY);
        let print = key.fingerprint();
        assert!(print.starts_with("SHA256:") && print.len() == 50, "{print}");
        assert!(!print.ends_with('='));
        // The kind it says it is has to be the kind it is.
        assert!(PublicKey::from_words("ssh-rsa", KEY).is_none());
        assert!(PublicKey::from_words("ssh-ed25519", "AAAA").is_none());
        assert!(PublicKey::from_words("ssh-ed25519", "not base64").is_none());
        assert!(PublicKey::from_words("ssh-ed25519", &"A".repeat(40_000)).is_none());
        // Only the name of a kind, and nothing of a key after it; a key
        // that was cut short; one with something hanging on.
        assert!(PublicKey::from_words("ssh-ed25519", "AAAAC3NzaC1lZDI1NTE5").is_none());
        assert!(PublicKey::from_words("ssh-ed25519", &KEY[..KEY.len() - 4]).is_none());
        let mut longer = decode(KEY).unwrap();
        longer.extend([0, 0, 0, 1, 7]);
        assert!(PublicKey::from_blob(&longer).is_none());
        longer.truncate(longer.len() - 2);
        assert!(PublicKey::from_blob(&longer).is_none());
        // Other kinds are their kind and two parts or more.
        let mut rsa = Vec::new();
        for part in [&b"ssh-rsa"[..], &[1, 0, 1], &[0x00, 0xc1, 0x55, 0x7f]] {
            rsa.extend(u32::try_from(part.len()).unwrap().to_be_bytes());
            rsa.extend(part);
        }
        assert_eq!(PublicKey::from_blob(&rsa).unwrap().kind, "ssh-rsa");
        assert!(PublicKey::from_blob(&rsa[..rsa.len() - 8]).is_none());
        assert!(known_kind("ecdsa-sha2-nistp256") && known_kind("sk-ssh-ed25519@openssh.com"));
        assert!(!known_kind("ssh-ed25519-cert-v01@openssh.com") && !known_kind(""));

        assert_eq!(
            key_in_text(&format!("# mine\n\nssh-ed25519 {KEY} ada@laptop\n")).unwrap(),
            key
        );
        assert_eq!(
            key_in_text(&format!("key::ssh-ed25519 {KEY}")).unwrap(),
            key
        );
        let private = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3Blbn…\n";
        assert!(
            key_in_text(private)
                .unwrap_err()
                .to_string()
                .contains("private key")
        );
        for nothing in ["", "hello", "ssh-ed25519", "ssh-ed25519 AAAA", "root:x:0:0"] {
            assert!(key_in_text(nothing).is_err(), "{nothing:?}");
        }
    }

    #[test]
    fn the_key_is_read_out_of_a_signature() {
        // SSHSIG, version 1, then the public key as a string.
        let blob = decode(KEY).unwrap();
        let mut signature = b"SSHSIG".to_vec();
        signature.extend(1u32.to_be_bytes());
        signature.extend(u32::try_from(blob.len()).unwrap().to_be_bytes());
        signature.extend(&blob);
        signature.extend(b"\0\0\0\x03git and the rest is not looked at");
        let armoured = |bytes: &[u8]| {
            let coded = encode(bytes, true);
            let lines: Vec<&str> = coded
                .as_bytes()
                .chunks(70)
                .map(|chunk| std::str::from_utf8(chunk).unwrap())
                .collect();
            format!(
                "-----BEGIN SSH SIGNATURE-----\n{}\n-----END SSH SIGNATURE-----\n",
                lines.join("\n")
            )
        };
        let key = key_of_signature(&armoured(&signature)).unwrap();
        assert_eq!(key.base64(), KEY);

        // Cut anywhere, or with another magic or version, it is refused.
        for cut in [0, 5, 6, 9, 10, 13, 14, 20, 40] {
            assert!(
                key_of_signature(&armoured(&signature[..cut])).is_err(),
                "cut at {cut}"
            );
        }
        let mut other_version = signature.clone();
        other_version[9] = 2;
        assert!(key_of_signature(&armoured(&other_version)).is_err());
        let mut huge_length = signature.clone();
        huge_length[10..14].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(key_of_signature(&armoured(&huge_length)).is_err());
        for not_ssh in [
            "",
            "-----BEGIN PGP SIGNATURE-----\n\niQEz\n-----END PGP SIGNATURE-----",
            "-----BEGIN SSH SIGNATURE-----",
            "-----BEGIN SSH SIGNATURE-----\n!!!\n-----END SSH SIGNATURE-----",
        ] {
            assert!(key_of_signature(not_ssh).is_err(), "{not_ssh:?}");
        }
    }

    #[test]
    fn a_line_is_read_as_principals_options_key_and_comment() {
        assert_eq!(
            words("  a  \"b c\" d=\"e f\",g\th "),
            ["a", "\"b c\"", "d=\"e f\",g", "h"]
        );
        assert_eq!(listed("a,b,,c"), ["a", "b", "c"]);
        assert_eq!(
            listed("namespaces=\"git,file\",cert-authority"),
            ["namespaces=\"git,file\"", "cert-authority"]
        );

        let plain = signer_of(&format!("ada@fixture.invalid ssh-ed25519 {KEY}")).unwrap();
        assert_eq!(plain.principals, ["ada@fixture.invalid"]);
        assert!(plain.options.is_empty() && plain.comment.is_none());
        assert_eq!(plain.key.base64(), KEY);

        let Line {
            principals: names,
            options,
            comment,
            ..
        } = signer_of(&format!(
            "ada@fixture.invalid,*@example.invalid namespaces=\"git,file\",valid-before=\"20300101\" \
             ssh-ed25519 {KEY} the laptop, since March"
        ))
        .unwrap();
        assert_eq!(names, ["ada@fixture.invalid", "*@example.invalid"]);
        assert_eq!(
            options,
            ["namespaces=\"git,file\"", "valid-before=\"20300101\""]
        );
        assert_eq!(comment.as_deref(), Some("the laptop, since March"));

        for not_one in [
            "",
            "# ada@fixture.invalid ssh-ed25519 AAAA",
            "ada@fixture.invalid",
            "ada@fixture.invalid ssh-ed25519",
            "ada@fixture.invalid ssh-ed25519 notakey",
            "ada@fixture.invalid one two ssh-ed25519 AAAA",
        ] {
            assert!(signer_of(not_one).is_none(), "{not_one:?}");
        }
    }

    #[test]
    fn a_signer_is_added_once_and_taken_out_without_touching_the_rest() {
        let key = PublicKey::from_words("ssh-ed25519", KEY).unwrap();
        let other = PublicKey::from_words("ssh-ed25519", OTHER).unwrap();
        let ada = ["ada@fixture.invalid".to_owned()];
        let line = format!("ada@fixture.invalid namespaces=\"git\" ssh-ed25519 {KEY}\n");
        // Into nothing, and after a last line that does not end.
        assert_eq!(with_signer("", &ada, &key).as_deref(), Some(line.as_str()));
        let kept = "# the team\n\nbroken line here";
        let grown = with_signer(kept, &ada, &key).unwrap();
        assert_eq!(grown, format!("{kept}\n{line}"));
        // The same signer with the same key is there already, however
        // the line is written.
        assert_eq!(with_signer(&grown, &ada, &key), None);
        assert_eq!(
            with_signer(
                &format!("ada@fixture.invalid ssh-ed25519 {KEY} old\n"),
                &ada,
                &key
            ),
            None
        );
        // Another name for the same key is another line.
        let bob = ["bob@fixture.invalid".to_owned()];
        let both = with_signer(&grown, &bob, &key).unwrap();
        assert_eq!(both.lines().count(), 5);

        // Taken out by fingerprint or by the key, for one name or all.
        let print = key.fingerprint();
        let (left, removed) = without_signer(&both, &print, Some(&bob));
        assert_eq!((left.as_str(), removed), (grown.as_str(), 1));
        let (left, removed) = without_signer(&both, KEY, None);
        assert_eq!(
            (left.as_str(), removed),
            ("# the team\n\nbroken line here\n", 2)
        );
        assert_eq!(
            without_signer(&both, "SHA256:nobody", None),
            (both.clone(), 0)
        );
        // A key that is not in it at all.
        assert_ne!(other.fingerprint(), print);
        assert_eq!(without_signer(&both, &other.fingerprint(), None).1, 0);
        // Windows line ends stay what they are on the lines that stay.
        let crlf = format!("# a\r\n{}\r\n# b\r\n", line.trim_end());
        assert_eq!(
            without_signer(&crlf, &print, None),
            ("# a\r\n# b\r\n".to_owned(), 1)
        );
    }
    #[test]
    fn a_change_to_the_lines_is_taken_back_and_made_again_to_the_letter() {
        use super::{Edit, redone, taken_out, undone};
        let line = |who: &str| format!("{who} namespaces=\"git\" ssh-ed25519 {KEY}\n");
        let edit = |added: &[String], removed: &[(usize, String)], mended: bool| Edit {
            file: "/x/allowed_signers".to_owned(),
            added: added.to_vec(),
            removed: removed.to_vec(),
            mended,
            created: false,
        };
        // A line put in after a last line that had no ending.
        let before = "# by hand\nlast line";
        let added = edit(&[line("ada@x")], &[], true);
        let after = redone(before, &added).unwrap();
        assert_eq!(after, format!("# by hand\nlast line\n{}", line("ada@x")));
        assert_eq!(undone(&after, &added).unwrap(), before);
        // Into nothing, and after a file that ends as files do.
        let plain = edit(&[line("ada@x")], &[], false);
        assert_eq!(undone(&redone("", &plain).unwrap(), &plain).unwrap(), "");
        assert_eq!(
            undone(&redone("a\n", &plain).unwrap(), &plain).unwrap(),
            "a\n"
        );
        // The line is gone, or there twice over: nothing is guessed.
        assert_eq!(undone(before, &added), None);
        assert_eq!(redone(&after, &added), None);
        // Something written after it since: the line goes, the ending it
        // gave stays with the line that now needs it.
        let later = format!("{after}# later\n");
        assert_eq!(
            undone(&later, &added).unwrap(),
            "# by hand\nlast line\n# later\n"
        );

        // Lines taken out, from the middle and the end, one without its
        // ending.
        let file = format!(
            "# top\n{}middle\n{}",
            line("ada@x"),
            line("ada@y").trim_end()
        );
        let (left, gone) = taken_out(&file, KEY, None);
        assert_eq!(left, "# top\nmiddle\n");
        assert_eq!(gone.len(), 2);
        let removed = edit(&[], &gone, false);
        assert_eq!(undone(&left, &removed).unwrap(), file);
        assert_eq!(redone(&file, &removed).unwrap(), left);
        // One of them is back already, or was never there.
        assert_eq!(undone(&file, &removed), None);
        assert_eq!(redone(&left, &removed), None);
        // The file grew meanwhile: they go back where they stood, or at
        // the end when it is shorter.
        assert_eq!(
            undone("# top\nmiddle\nmore\n", &removed).unwrap(),
            format!(
                "# top\n{}middle\n{}\nmore\n",
                line("ada@x"),
                line("ada@y").trim_end()
            )
        );
        assert_eq!(
            undone("", &removed).unwrap(),
            format!("{}{}", line("ada@x"), line("ada@y").trim_end())
        );
    }
}
