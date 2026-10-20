//! The update key, and what a package is held to before anything is
//! replaced with it.
//!
//! The key is the minisign public key built into the application, from
//! `packaging/update-key.pub`. While that file is a placeholder, a build
//! with it has no key ([`UpdateKey::Missing`]): its updater says so and
//! asks nobody anything.
//!
//! A package is checked as it is written to disk ([`Checking`]), so that it
//! is never held in memory: no more bytes than the feed gives, then exactly
//! that many, its SHA-256 the feed's, and its minisign signature good for
//! the update key. Only minisign's hashed signatures are taken (Ed25519
//! over the file's BLAKE2b-512, what `minisign -S` makes), and the
//! signature's trusted comment, which the key signs too, has to name the
//! release's version and the package's file (`version:1.2.0
//! file:BitBraid-1.2.0-x86_64.AppImage`): a signed package of another
//! release, or of another kind, offered under this one's name is refused.
//!
//! Sources: jedisct1.github.io/minisign (the public key's and the
//! signature's text forms, hashed signatures, the trusted comment and the
//! global signature over it, `-G`, `-S`, `-t`); the documentation of
//! `minisign-verify` (`PublicKey::decode`, `Signature::decode`,
//! `PublicKey::verify_stream`, `StreamVerifier`) and of `sha2`; FIPS 180-4
//! (SHA-256).

use std::{fmt, io};

use minisign_verify::{PublicKey, Signature, StreamVerifier};
use sha2::{Digest as _, Sha256};

use super::feed::Package;

/// The public key built in, as `minisign -G` writes it: a comment line,
/// then the key; or the placeholder, whose second line is [`PLACEHOLDER`].
const BUILT_IN: &str = include_str!("../../../../packaging/update-key.pub");

/// What the placeholder has where a key would be.
const PLACEHOLDER: &str = "none";

/// The key packages are checked against.
#[derive(Debug, Clone)]
pub enum UpdateKey {
    /// The build has none: updates are not set up, and nothing is asked.
    Missing,
    /// The key in the build cannot be read: a mistake in making the build,
    /// which a test of this module catches. Nothing is asked either.
    Unreadable(String),
    /// This key.
    Key(Box<PublicKey>),
}

impl UpdateKey {
    /// The key built into this build.
    pub fn built_in() -> Self {
        Self::from_text(BUILT_IN)
    }

    /// A key from the text of a minisign public key file, or the
    /// placeholder.
    pub fn from_text(text: &str) -> Self {
        let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
        let _comment = lines.next();
        if lines.next() == Some(PLACEHOLDER) {
            return Self::Missing;
        }
        match PublicKey::decode(text.trim()) {
            Ok(key) => Self::Key(Box::new(key)),
            Err(err) => Self::Unreadable(err.to_string()),
        }
    }
}

/// Why a downloaded package is not installed, said of "the download".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// The signature in the feed cannot be read.
    Unreadable,
    /// It was signed with another key.
    OtherKey,
    /// The signature is of minisign's old kind, over the file itself.
    OldKind,
    /// More came than the feed gives.
    TooLarge {
        /// What the feed gives.
        expected: u64,
    },
    /// Less came than the feed gives.
    Size {
        /// What came.
        got: u64,
        /// What the feed gives.
        expected: u64,
    },
    /// Its SHA-256 is not the feed's.
    Checksum,
    /// The signature does not hold for what came.
    Signature,
    /// The signature holds, for another release or file.
    NotThisPackage {
        /// What its trusted comment says.
        comment: String,
    },
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unreadable => f.write_str(say!("update-refusal-unreadable")),
            Self::OtherKey => f.write_str(say!("update-refusal-other-key")),
            Self::OldKind => f.write_str(say!("update-refusal-old-kind")),
            Self::TooLarge { expected } => {
                f.write_str(&say!("update-refusal-too-large", expected = *expected))
            }
            Self::Size { got, expected } => f.write_str(&say!(
                "update-refusal-size",
                got = *got,
                expected = *expected
            )),
            Self::Checksum => f.write_str(say!("update-refusal-checksum")),
            Self::Signature => f.write_str(say!("update-refusal-signature")),
            Self::NotThisPackage { comment } => {
                f.write_str(&say!("update-refusal-not-this-package", comment))
            }
        }
    }
}

/// The signature of a package, read, and checked to be one the update key
/// could have made: of that key, and of the hashed kind.
pub(crate) fn signature_for(key: &PublicKey, package: &Package) -> Result<Signature, Refusal> {
    let signature = Signature::decode(&package.signature).map_err(|_| Refusal::Unreadable)?;
    match key.verify_stream(&signature) {
        Ok(_) => Ok(signature),
        Err(minisign_verify::Error::UnexpectedKeyId) => Err(Refusal::OtherKey),
        Err(minisign_verify::Error::UnsupportedLegacyMode) => Err(Refusal::OldKind),
        Err(_) => Err(Refusal::Unreadable),
    }
}

/// What a package is written to, which checks it as it comes.
pub(crate) struct Checking<'a, W: io::Write> {
    into: W,
    sha: Sha256,
    verifier: StreamVerifier<'a>,
    signature: &'a Signature,
    written: u64,
    expected: u64,
    /// More came than the feed gives: the write that would have gone over
    /// failed, and so did the download.
    pub(crate) over: bool,
    on_written: &'a mut dyn FnMut(u64),
}

impl<W: io::Write> fmt::Debug for Checking<'_, W> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Checking")
            .field("written", &self.written)
            .field("expected", &self.expected)
            .finish_non_exhaustive()
    }
}

impl<'a, W: io::Write> Checking<'a, W> {
    /// Write a package of `expected` bytes into `into`, checked against
    /// `signature` (from [`signature_for`]) and `key` as it comes; told how
    /// much was written after each piece.
    pub(crate) fn new(
        into: W,
        key: &'a PublicKey,
        signature: &'a Signature,
        expected: u64,
        on_written: &'a mut dyn FnMut(u64),
    ) -> Result<Self, Refusal> {
        let verifier = key
            .verify_stream(signature)
            .map_err(|_| Refusal::OtherKey)?;
        Ok(Self {
            into,
            sha: Sha256::new(),
            verifier,
            signature,
            written: 0,
            expected,
            over: false,
            on_written,
        })
    }

    /// How many bytes came and were written so far.
    pub(crate) fn written(&self) -> u64 {
        self.written
    }

    /// Whether what came is the package the feed gives for `version`,
    /// signed with the key: its size, its SHA-256, its signature and what
    /// its trusted comment names. What it was written to, flushed.
    pub(crate) fn finish(mut self, package: &Package, version: &str) -> Result<W, Refusal> {
        if self.over {
            return Err(Refusal::TooLarge {
                expected: self.expected,
            });
        }
        if self.written != self.expected {
            return Err(Refusal::Size {
                got: self.written,
                expected: self.expected,
            });
        }
        let digest = std::mem::take(&mut self.sha).finalize();
        if digest.as_slice() != package.sha256.as_slice() {
            return Err(Refusal::Checksum);
        }
        self.verifier.finalize().map_err(|_| Refusal::Signature)?;
        let comment = self.signature.trusted_comment();
        if !names_package(comment, version, &package.name) {
            return Err(Refusal::NotThisPackage {
                comment: comment.chars().take(120).collect(),
            });
        }
        let _ = self.into.flush();
        Ok(self.into)
    }
}

impl<W: io::Write> io::Write for Checking<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let after = self
            .written
            .checked_add(u64::try_from(buf.len()).unwrap_or(u64::MAX));
        if after.is_none_or(|after| after > self.expected) {
            self.over = true;
            return Err(io::Error::other(say!("update-more-than-given")));
        }
        self.into.write_all(buf)?;
        self.sha.update(buf);
        self.verifier.update(buf);
        self.written = after.unwrap_or(self.written);
        (self.on_written)(self.written);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.into.flush()
    }
}

/// Whether a trusted comment names this version and file: words
/// `version:<version>` and `file:<name>` among others, in any order.
pub(crate) fn names_package(comment: &str, version: &str, name: &str) -> bool {
    let has = |key: &str, value: &str| {
        comment
            .split_whitespace()
            .any(|word| word.strip_prefix(key) == Some(value))
    };
    has("version:", version) && has("file:", name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_built_in_key_is_the_placeholder_or_a_key() {
        // A key that cannot be read would turn updates off unseen.
        assert!(
            !matches!(UpdateKey::built_in(), UpdateKey::Unreadable(_)),
            "packaging/update-key.pub is neither the placeholder nor a minisign public key"
        );
    }

    #[test]
    fn a_key_file_is_read_and_the_placeholder_is_no_key() {
        assert!(matches!(
            UpdateKey::from_text("untrusted comment: no key yet\nnone\n"),
            UpdateKey::Missing
        ));
        assert!(matches!(
            UpdateKey::from_text(
                "untrusted comment: minisign public key E7620F1842B4E81F\n\
                 RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3\n"
            ),
            UpdateKey::Key(_)
        ));
        for odd in [
            "",
            "one line",
            "untrusted comment: x\nRWQ!!\n",
            "\u{0}\n\u{0}",
        ] {
            assert!(
                matches!(UpdateKey::from_text(odd), UpdateKey::Unreadable(_)),
                "{odd:?}"
            );
        }
    }

    #[test]
    fn a_trusted_comment_has_to_name_the_version_and_the_file() {
        let name = "BitBraid-1.2.0-x86_64.AppImage";
        assert!(names_package(
            "version:1.2.0 file:BitBraid-1.2.0-x86_64.AppImage",
            "1.2.0",
            name
        ));
        assert!(names_package(
            "timestamp:1700000000\tfile:BitBraid-1.2.0-x86_64.AppImage\tversion:1.2.0",
            "1.2.0",
            name
        ));
        assert!(!names_package(
            "version:1.1.0 file:BitBraid-1.2.0-x86_64.AppImage",
            "1.2.0",
            name
        ));
        assert!(!names_package(
            "version:1.2.0 file:BitBraid-1.2.0-x86_64.deb",
            "1.2.0",
            name
        ));
        assert!(!names_package("version:1.2.0", "1.2.0", name));
        assert!(!names_package("", "1.2.0", name));
        assert!(!names_package(
            "version:1.2.00 file:BitBraid-1.2.0-x86_64.AppImage",
            "1.2.0",
            name
        ));
    }
}
