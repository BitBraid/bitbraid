//! Signatures as minisign makes them, with a key of a test's own: what the
//! updater's tests sign their packages with. The key is made from a seed
//! the test gives, in memory; nothing is written anywhere.
//!
//! A public key is two lines, a comment and `Ed`, the key's id and the
//! Ed25519 public key in base64. A signature is four: a comment; `ED`, the
//! key's id and the Ed25519 signature of the file's BLAKE2b-512 (the
//! hashed kind `minisign -S` makes), or `Ed` and the signature of the file
//! itself (the older kind); the trusted comment; and the signature of the
//! first signature followed by the trusted comment.
//!
//! Sources: jedisct1.github.io/minisign (the formats of the public key and
//! signature files, hashed signatures, the trusted comment and the global
//! signature); RFC 8032 (Ed25519); RFC 7693 (BLAKE2b); the documentation of
//! `ring` (`Ed25519KeyPair::from_seed_unchecked`) and of `blake2`
//! (`Blake2b512`).

use base64::Engine as _;
use blake2::{Blake2b512, Digest as _};
use ring::signature::{Ed25519KeyPair, KeyPair as _};

/// A minisign key of a test's own.
pub struct Signer {
    pair: Ed25519KeyPair,
    id: [u8; 8],
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signer").field("id", &self.id).finish()
    }
}

fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The SHA-256 of `bytes` in hexadecimal digits, as a feed of releases
/// writes it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    ring::digest::digest(&ring::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl Signer {
    /// The key made from `seed`: the same seed, the same key. Keys of
    /// different seeds have different ids.
    pub fn new(seed: u8) -> anyhow::Result<Self> {
        let pair = Ed25519KeyPair::from_seed_unchecked(&[seed; 32])
            .map_err(|_| anyhow::anyhow!("no key from that seed"))?;
        let id = [seed, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42, 0x42];
        Ok(Self { pair, id })
    }

    /// The public key, as the text of a `minisign.pub` file.
    pub fn public_key(&self) -> String {
        let mut bin = b"Ed".to_vec();
        bin.extend_from_slice(&self.id);
        bin.extend_from_slice(self.pair.public_key().as_ref());
        let id: String = self.id.iter().rev().map(|b| format!("{b:02X}")).collect();
        format!(
            "untrusted comment: minisign public key {id}\n{}\n",
            base64(&bin)
        )
    }

    /// A signature of `bytes` of the hashed kind, with this trusted
    /// comment, as the text of a `.minisig` file.
    pub fn sign(&self, bytes: &[u8], trusted_comment: &str) -> String {
        let hash = Blake2b512::digest(bytes);
        self.signature(b"ED", hash.as_slice(), trusted_comment)
    }

    /// A signature of the older kind, of `bytes` themselves.
    pub fn sign_unhashed(&self, bytes: &[u8], trusted_comment: &str) -> String {
        self.signature(b"Ed", bytes, trusted_comment)
    }

    fn signature(&self, algorithm: &[u8; 2], signed: &[u8], trusted_comment: &str) -> String {
        let first = self.pair.sign(signed);
        let mut line = algorithm.to_vec();
        line.extend_from_slice(&self.id);
        line.extend_from_slice(first.as_ref());
        let mut global = first.as_ref().to_vec();
        global.extend_from_slice(trusted_comment.as_bytes());
        let second = self.pair.sign(&global);
        format!(
            "untrusted comment: signature from a test key\n{}\ntrusted comment: {trusted_comment}\n{}\n",
            base64(&line),
            base64(second.as_ref())
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_signs_in_minisigns_four_lines() {
        let signer = Signer::new(7).unwrap();
        let public = signer.public_key();
        assert!(public.starts_with("untrusted comment: minisign public key 42424242424242"));
        assert_eq!(public.lines().count(), 2);
        let signature = signer.sign(b"hello", "version:1.0.0 file:x");
        let lines: Vec<&str> = signature.lines().collect();
        assert_eq!(lines.len(), 4);
        assert_eq!(lines[2], "trusted comment: version:1.0.0 file:x");
        // `ED` and `Ed` in base64 at the start of the line.
        assert!(lines[1].starts_with("RUQ"), "{}", lines[1]);
        assert!(
            signer
                .sign_unhashed(b"hello", "x")
                .lines()
                .nth(1)
                .unwrap()
                .starts_with("RWQ")
        );
        assert_ne!(Signer::new(8).unwrap().public_key(), public);
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
}
