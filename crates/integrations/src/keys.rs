//! Public SSH keys as their files hold them: one line of a kind, the key
//! itself in base64 and a comment. A key is read and checked before it is
//! sent anywhere: a private key is refused, and so is anything whose kind
//! the providers do not take or whose key does not say the kind it is
//! written as. Its fingerprint is worked out here, the way
//! `ssh-keygen -l` writes it, so that it is known whatever the provider
//! answers.
//!
//! Sources: RFC 4253 §6.6 (a public key: its kind as the first string of
//! the key), RFC 4648 §4 (base64), ssh-keygen(1) (`-l`, `SHA256:` and
//! base64 without padding), sshd(8) "AUTHORIZED_KEYS FILE FORMAT" (the
//! line: kind, key, comment); docs.github.com "Adding a new SSH key to
//! your GitHub account" and docs.gitlab.com "Use SSH keys to communicate
//! with GitLab" (the kinds they take).

use sha2::{Digest as _, Sha256};

/// The kinds of keys both providers take.
pub const KINDS: &[&str] = &[
    "ssh-ed25519",
    "ecdsa-sha2-nistp256",
    "ecdsa-sha2-nistp384",
    "ecdsa-sha2-nistp521",
    "ssh-rsa",
    "sk-ssh-ed25519@openssh.com",
    "sk-ecdsa-sha2-nistp256@openssh.com",
];

/// The longest key line taken, in bytes (an RSA key of 16384 bits is far
/// less).
pub const MAX_LINE: usize = 16 * 1024;

/// A public key, read and checked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    /// Its kind: `ssh-ed25519`.
    pub kind: String,
    /// The line to send: kind, key and comment, as the file has them.
    pub line: String,
    /// `SHA256:` and the key's hash in base64 without padding.
    pub fingerprint: String,
}

/// Read a public key's file's text. The error says what is wrong.
pub fn read(text: &str) -> Result<PublicKey, String> {
    if text.contains("PRIVATE KEY") || text.contains("-----BEGIN") {
        return Err(say!("keys-private").to_owned());
    }
    let line = text.trim();
    if line.len() > MAX_LINE {
        return Err(say!("keys-too-long", most = MAX_LINE));
    }
    if line.lines().count() != 1 || line.chars().any(|c| c.is_control() && c != '\t') {
        return Err(say!("keys-not-one-line").to_owned());
    }
    let mut parts = line.split_whitespace();
    let kind = parts.next().unwrap_or_default();
    if !KINDS.contains(&kind) {
        return Err(say!(
            "keys-kind-not-taken",
            kind = format!("{:?}", crate::api::clean_line(kind, 64)),
            kinds = KINDS.join(", ")
        ));
    }
    let blob = parts
        .next()
        .and_then(base64)
        .ok_or_else(|| say!("keys-not-base64").to_owned())?;
    if inner_kind(&blob).as_deref() != Some(kind) {
        return Err(say!("keys-kind-differs", kind));
    }
    let hash = Sha256::digest(&blob);
    Ok(PublicKey {
        kind: kind.to_owned(),
        line: line.to_owned(),
        fingerprint: format!("SHA256:{}", base64_unpadded(&hash)),
    })
}

/// The kind a key says it is: the first string of the key (a length of
/// four bytes, then that many bytes).
fn inner_kind(blob: &[u8]) -> Option<String> {
    let length = blob.get(..4)?;
    let length = u32::from_be_bytes([
        *length.first()?,
        *length.get(1)?,
        *length.get(2)?,
        *length.get(3)?,
    ]);
    let end = usize::try_from(length).ok()?.checked_add(4)?;
    let kind = blob.get(4..end)?;
    String::from_utf8(kind.to_vec()).ok()
}

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 of RFC 4648 §4 read, padding as written; `None` for anything
/// else.
fn base64(text: &str) -> Option<Vec<u8>> {
    let bytes = text.as_bytes();
    if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
        return None;
    }
    let padding = bytes.iter().rev().take_while(|b| **b == b'=').count();
    if padding > 2 {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let body = bytes.get(..bytes.len() - padding)?;
    let mut buffer = 0u32;
    let mut bits = 0u32;
    for byte in body {
        let value = ALPHABET.iter().position(|known| known == byte)?;
        buffer = (buffer << 6) | u32::try_from(value).ok()?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((buffer >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

/// Base64 without padding, as fingerprints are written.
fn base64_unpadded(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let at = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (at(0) << 16) | (at(1) << 8) | at(2);
        for i in 0..=chunk.len() {
            let index = usize::try_from((n >> (18 - 6 * i)) & 63).unwrap_or(0);
            out.push(char::from(ALPHABET.get(index).copied().unwrap_or(b'A')));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key made by `ssh-keygen -t ed25519` for these tests only (its
    /// private half was thrown away); `ssh-keygen -l` says its fingerprint.
    const ED25519: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAWUi35BfUEiiOjbuDIeFeVHDul6RR5Nf+5RPMLvtWfd tester@example.invalid";

    #[test]
    fn a_public_key_is_read_with_its_fingerprint() {
        let key = read(&format!("{ED25519}\n")).unwrap();
        assert_eq!(key.kind, "ssh-ed25519");
        assert_eq!(key.line, ED25519);
        assert_eq!(
            key.fingerprint,
            "SHA256:IeI4dtwxvncxctQMgpbLrR9zUYXixuEIOupl+ATPL/0"
        );
    }

    #[test]
    fn what_is_not_a_public_key_is_refused() {
        for text in [
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----",
            "",
            "ssh-dss AAAAB3NzaC1kc3MAAACBAP",
            "ssh-ed25519 not*base64",
            "ssh-ed25519 AAAAB3NzaC1yc2EAAAADAQAB",
            "ssh-ed25519 AAAA",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5\nssh-ed25519 AAAAC3NzaC1lZDI1NTE5",
            "--upload-pack=x AAAA",
        ] {
            assert!(read(text).is_err(), "{text}");
        }
        assert!(read(&format!("ssh-rsa {}", "A".repeat(MAX_LINE))).is_err());
    }

    #[test]
    fn base64_is_read_and_written() {
        assert_eq!(base64("aGVsbG8=").unwrap(), b"hello");
        assert_eq!(base64("aGVsbG8h").unwrap(), b"hello!");
        assert_eq!(base64("aGVsbA==").unwrap(), b"hell");
        assert!(base64("aGVsbG8").is_none());
        assert!(base64("a===").is_none());
        assert_eq!(base64_unpadded(b"hello"), "aGVsbG8");
        assert_eq!(base64_unpadded(b""), "");
    }
}
