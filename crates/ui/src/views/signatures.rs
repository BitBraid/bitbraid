//! Signatures as the interface shows them: what `sign.verify` and
//! `tag.details` answered, said in a sentence, with a mark and a colour
//! for how far it can be trusted.
//!
//! Sources: gpgsm(1) for what is wrong with a certificate's chain.

use gpui_kit::assets::IconName;

/// How much a signature is worth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Made by a key that is trusted here.
    Good,
    /// Made, and something about it is short of that: the key is not
    /// trusted, not known, expired, or the signature cannot be checked.
    Doubtful,
    /// It does not match what it signs, or its key was revoked.
    Bad,
}

/// What git made of one signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Verification {
    /// `good`, `untrusted`, `expired`, `key_expired`, `key_revoked`,
    /// `bad`, `unknown_key`, `cannot_check`; never `none`.
    pub state: String,
    /// `openpgp`, `ssh` or `x509`.
    pub format: Option<String>,
    /// Who signed, as the key or the allowed-signers file names them.
    pub signer: Option<String>,
    /// The key it was made with.
    pub key: Option<String>,
    /// Why it could not be checked; for a certificate whose chain does not
    /// hold, what gpgsm said of the chain.
    pub reason: Option<String>,
    /// What is wrong with the chain of an X.509 certificate whose
    /// signature holds: `missing_issuer`, `untrusted_root`, `revoked`,
    /// `revocation_unknown` or `other`. Git grades all of them
    /// `untrusted`.
    pub chain: Option<String>,
}

impl Verification {
    /// Read one answer; `None` for a commit or tag without a signature.
    pub fn from_json(value: &serde_json::Value) -> Option<Self> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let state = text("state").filter(|state| state != "none")?;
        Some(Self {
            state,
            format: text("format"),
            signer: text("signer"),
            key: text("key"),
            reason: text("reason"),
            chain: text("chain"),
        })
    }

    /// How much it is worth. A certificate that was revoked is as bad as a
    /// key that was, whatever letter git gives it.
    pub fn level(&self) -> Level {
        match self.state.as_str() {
            "good" => Level::Good,
            "bad" | "key_revoked" => Level::Bad,
            _ if self.chain.as_deref() == Some("revoked") => Level::Bad,
            _ => Level::Doubtful,
        }
    }

    /// Whether it was made with a certificate rather than a key.
    fn certificate(&self) -> bool {
        self.format.as_deref() == Some("x509")
    }

    /// What is wrong with a certificate's chain, as the end of a sentence
    /// that says who signed; `None` when nothing is said to be wrong.
    fn chain_said(&self) -> Option<String> {
        let reason = self.reason.as_deref().filter(|reason| !reason.is_empty());
        let said = match self.chain.as_deref()? {
            "missing_issuer" => say!("signatures-chain-missing-issuer").to_owned(),
            "untrusted_root" => say!("signatures-chain-untrusted-root").to_owned(),
            "revoked" => say!("signatures-chain-revoked").to_owned(),
            "revocation_unknown" => match reason {
                Some(reason) => say!("signatures-chain-revocation-unknown-why", reason),
                None => say!("signatures-chain-revocation-unknown").to_owned(),
            },
            _ => match reason {
                Some(reason) => say!("signatures-chain-other-why", reason),
                None => say!("signatures-chain-other").to_owned(),
            },
        };
        Some(said)
    }

    /// Whether allowing the key it was made with could make it good: an
    /// SSH signature that holds, by a key that is in no allowed-signers
    /// file, or one that could not be checked for want of such a file.
    /// One that does not match what it signs stays what it is.
    pub fn key_can_be_allowed(&self) -> bool {
        self.format.as_deref() == Some("ssh")
            && matches!(
                self.state.as_str(),
                "untrusted" | "unknown_key" | "cannot_check"
            )
    }

    /// The mark that goes with it.
    pub fn icon(&self) -> IconName {
        match self.level() {
            Level::Good => IconName::ShieldCheck,
            Level::Bad => IconName::ShieldX,
            Level::Doubtful if self.state == "cannot_check" => IconName::ShieldQuestionMark,
            Level::Doubtful => IconName::ShieldAlert,
        }
    }

    /// What there is to say about it, in a sentence.
    pub fn sentence(&self) -> String {
        let with_key = |what: &str| match &self.key {
            Some(key) => format!("{what} ({key})"),
            None => what.to_owned(),
        };
        // A certificate whose chain does not hold: who signed, and what
        // is wrong with the chain.
        if self.state == "untrusted"
            && let Some(said) = self.chain_said()
        {
            return match &self.signer {
                Some(signer) => say!("signatures-signed-by-but", signer, said),
                None => say!("signatures-signed-but", said),
            };
        }
        let certificate = self.certificate();
        let signer = self.signer.as_deref();
        // What is said, and what is said when a signer is named. A
        // signature that names nobody (an SSH key no file of allowed
        // signers knows, a signature that does not match) ends where it
        // has said what it is, with the key when that is known: "It
        // claims to be" with nobody behind it would say nothing.
        let (what, by) = match self.state.as_str() {
            "good" => (
                say!("signatures-good"),
                signer.map(|signer| say!("signatures-good-by", signer)),
            ),
            "untrusted" if certificate => (
                say!("signatures-untrusted-certificate"),
                signer.map(|signer| say!("signatures-untrusted-certificate-by", signer)),
            ),
            "untrusted" => (
                say!("signatures-untrusted"),
                signer.map(|signer| say!("signatures-untrusted-by", signer)),
            ),
            "expired" => (
                say!("signatures-expired"),
                signer.map(|signer| say!("signatures-expired-by", signer)),
            ),
            "key_expired" if certificate => (
                say!("signatures-key-expired-certificate"),
                signer.map(|signer| say!("signatures-key-expired-certificate-by", signer)),
            ),
            "key_expired" => (
                say!("signatures-key-expired"),
                signer.map(|signer| say!("signatures-key-expired-by", signer)),
            ),
            "key_revoked" if certificate => (
                say!("signatures-key-revoked-certificate"),
                signer.map(|signer| say!("signatures-key-revoked-certificate-by", signer)),
            ),
            "key_revoked" => (
                say!("signatures-key-revoked"),
                signer.map(|signer| say!("signatures-key-revoked-by", signer)),
            ),
            "bad" => (
                say!("signatures-bad"),
                signer.map(|signer| say!("signatures-bad-by", signer)),
            ),
            "unknown_key" if certificate => {
                return with_key(say!("signatures-unknown-key-certificate"));
            }
            "unknown_key" => return with_key(say!("signatures-unknown-key")),
            _ => {
                return match &self.reason {
                    Some(reason) => say!("signatures-cannot-check-why", reason),
                    None => say!("signatures-cannot-check").to_owned(),
                };
            }
        };
        match by {
            Some(by) => by,
            // A signature that does not match says nothing true about
            // its key either.
            None if self.state == "bad" => what.to_owned(),
            None => with_key(what),
        }
    }
}

/// What to say of a commit that carries a signature nobody has looked at
/// yet.
pub fn unchecked_sentence(format: Option<&str>) -> String {
    match format {
        Some("ssh") => say!("signatures-unchecked-ssh").to_owned(),
        Some("x509") => say!("signatures-unchecked-certificate").to_owned(),
        Some(_) => say!("signatures-unchecked-gpg").to_owned(),
        None => say!("signatures-unchecked").to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use gpui_kit::assets::IconName;

    use super::{Level, Verification, unchecked_sentence};

    fn read(value: serde_json::Value) -> Verification {
        Verification::from_json(&value).unwrap()
    }

    #[test]
    fn every_state_has_a_level_and_a_sentence() {
        let good =
            read(json!({ "state": "good", "format": "ssh", "signer": "ada@fixture.invalid" }));
        assert_eq!(good.level(), Level::Good);
        assert_eq!(good.sentence(), "Good signature by ada@fixture.invalid");
        assert_eq!(
            read(json!({ "state": "good" })).sentence(),
            "Good signature"
        );

        let untrusted = read(json!({ "state": "untrusted", "signer": "Bob" }));
        assert_eq!(untrusted.level(), Level::Doubtful);
        assert_eq!(
            untrusted.sentence(),
            "Signed, with a key that is not trusted here, by Bob"
        );
        assert_eq!(
            read(json!({ "state": "untrusted" })).sentence(),
            "Signed, with a key that is not trusted here"
        );
        let bad = read(json!({ "state": "bad", "signer": "Ada" }));
        assert_eq!(bad.level(), Level::Bad);
        assert_eq!(
            bad.sentence(),
            "The signature does not match what it signs. It claims to be by Ada"
        );
        // One that names nobody (an SSH signature that does not match)
        // ends there, not in "It claims to be" with nothing after it.
        assert_eq!(
            read(json!({ "state": "bad", "format": "ssh", "key": "SHA256:abc" })).sentence(),
            "The signature does not match what it signs"
        );
        assert_eq!(
            read(json!({ "state": "expired", "signer": "Ada" })).sentence(),
            "Signed; the signature has expired. It was made by Ada"
        );
        assert_eq!(
            read(json!({ "state": "expired" })).sentence(),
            "Signed; the signature has expired"
        );
        assert_eq!(
            read(json!({ "state": "key_expired", "signer": "Ada" })).sentence(),
            "Signed, with a key that has expired since, by Ada"
        );
        assert_eq!(
            read(json!({ "state": "key_revoked", "signer": "Ada" })).sentence(),
            "Signed with a key that was revoked, by Ada"
        );
        // Nobody is named, the key is: an SSH key the allowed signers do
        // not know is told by its fingerprint.
        assert_eq!(
            read(json!({ "state": "untrusted", "format": "ssh", "key": "SHA256:abc" })).sentence(),
            "Signed, with a key that is not trusted here (SHA256:abc)"
        );
        // No state ends in a word that waits for another.
        for state in [
            "good",
            "untrusted",
            "expired",
            "key_expired",
            "key_revoked",
            "bad",
            "unknown_key",
            "cannot_check",
        ] {
            for value in [
                json!({ "state": state }),
                json!({ "state": state, "key": "K" }),
                json!({ "state": state, "signer": "S" }),
            ] {
                let said = read(value).sentence();
                for dangling in [" be", " made", " by", ",", " to", ":"] {
                    assert!(!said.ends_with(dangling), "{state}: {said:?}");
                }
            }
        }
        assert_eq!(read(json!({ "state": "key_revoked" })).level(), Level::Bad);
        let unknown = read(json!({ "state": "unknown_key", "key": "ABCD1234" }));
        assert_eq!(
            unknown.sentence(),
            "Signed with a key that is not known here (ABCD1234)"
        );
        let cannot = read(json!({ "state": "cannot_check", "reason": "no allowed-signers file" }));
        assert_eq!(cannot.level(), Level::Doubtful);
        assert_eq!(
            cannot.sentence(),
            "Signed; it cannot be checked here: no allowed-signers file"
        );
        // A state from a later version is doubtful, not good.
        let later = read(json!({ "state": "something new" }));
        assert_eq!(later.level(), Level::Doubtful);
        assert_eq!(later.sentence(), "Signed; it cannot be checked here");
    }

    /// What `sign.verify` answers in each case of a certificate from an
    /// authority (the engine's `tests/suite/x509_chains.rs`, with gpgsm
    /// 2.4 and git 2.56), and what the mark and the sentence make of it.
    #[test]
    fn certificates_from_an_authority_say_what_git_said() {
        let signer = "/CN=Ada Fixture/O=BitBraid Test";
        let x509 = |state: &str, trust: &str, chain: Option<&str>, reason: Option<&str>| {
            read(json!({
                "state": state, "format": "x509", "signer": signer,
                "key": "6D9FDA24ACCD0838389C68B859BE0B146AD74A50",
                "trust": trust, "chain": chain, "reason": reason,
            }))
        };
        let cases = [
            // The chain trusted at its root (lists checked or not).
            (
                x509("good", "fully", None, None),
                Level::Good,
                IconName::ShieldCheck,
                format!("Good signature by {signer}"),
            ),
            // The intermediate nowhere.
            (
                x509(
                    "untrusted",
                    "undefined",
                    Some("missing_issuer"),
                    Some("Missing issuer certificate"),
                ),
                Level::Doubtful,
                IconName::ShieldAlert,
                format!(
                    "Signed by {signer}, but the certificate's chain is not complete here: a \
                     certificate that issued it is missing"
                ),
            ),
            // The root not trusted.
            (
                x509(
                    "untrusted",
                    "undefined",
                    Some("untrusted_root"),
                    Some("Not trusted"),
                ),
                Level::Doubtful,
                IconName::ShieldAlert,
                format!(
                    "Signed by {signer}, but the certificate leads to a root certificate that is \
                     not trusted here"
                ),
            ),
            // The certificate expired since.
            (
                x509("key_expired", "fully", None, None),
                Level::Doubtful,
                IconName::ShieldAlert,
                format!("Signed, with a certificate that has expired since, by {signer}"),
            ),
            // Revoked by the intermediate's list: git grades it "U", with
            // trust "never"; it is shown as bad.
            (
                x509(
                    "untrusted",
                    "never",
                    Some("revoked"),
                    Some("Certificate revoked"),
                ),
                Level::Bad,
                IconName::ShieldX,
                format!("Signed by {signer}, but a certificate of its chain was revoked"),
            ),
            // No list to be had.
            (
                x509(
                    "untrusted",
                    "undefined",
                    Some("revocation_unknown"),
                    Some("Not supported"),
                ),
                Level::Doubtful,
                IconName::ShieldAlert,
                format!(
                    "Signed by {signer}, but whether the certificate was revoked could not be \
                     checked (Not supported)"
                ),
            ),
        ];
        for (verification, level, icon, sentence) in cases {
            assert_eq!(verification.level(), level, "{verification:?}");
            assert_eq!(verification.icon(), icon, "{verification:?}");
            assert_eq!(verification.sentence(), sentence);
        }
        // Without a signer, or with a fault from a later version.
        let nobody = read(json!({ "state": "untrusted", "format": "x509", "chain": "revoked" }));
        assert_eq!(
            nobody.sentence(),
            "Signed, but a certificate of its chain was revoked"
        );
        let later = read(json!({
            "state": "untrusted", "format": "x509", "chain": "something new", "reason": "odd"
        }));
        assert_eq!(later.level(), Level::Doubtful);
        assert_eq!(
            later.sentence(),
            "Signed, but the certificate's chain does not hold here (odd)"
        );
        // A chain fault only counts with git's "untrusted".
        let bad = read(json!({ "state": "bad", "format": "x509", "chain": "untrusted_root" }));
        assert_eq!(bad.sentence(), "The signature does not match what it signs");
        // A certificate nothing is said about, or one not known here.
        assert_eq!(
            read(json!({ "state": "untrusted", "format": "x509" })).sentence(),
            "Signed, with a certificate that is not trusted here"
        );
        assert_eq!(
            read(json!({ "state": "unknown_key", "format": "x509", "key": "ABCD" })).sentence(),
            "Signed with a certificate that is not known here (ABCD)"
        );
        assert_eq!(
            read(json!({ "state": "key_revoked", "format": "x509" })).sentence(),
            "Signed with a certificate that was revoked"
        );
    }

    #[test]
    fn only_an_ssh_key_that_holds_and_is_not_vouched_for_can_be_allowed() {
        for (state, format, can) in [
            ("untrusted", "ssh", true),
            ("cannot_check", "ssh", true),
            ("unknown_key", "ssh", true),
            // Good already; or it does not match, which no key mends.
            ("good", "ssh", false),
            ("bad", "ssh", false),
            ("key_revoked", "ssh", false),
            // The allowed signers are about SSH keys only.
            ("untrusted", "openpgp", false),
            ("unknown_key", "x509", false),
        ] {
            let verification = read(json!({ "state": state, "format": format }));
            assert_eq!(verification.key_can_be_allowed(), can, "{state} {format}");
        }
        assert!(!read(json!({ "state": "untrusted" })).key_can_be_allowed());
    }

    #[test]
    fn no_signature_is_nothing_to_show() {
        for nothing in [
            json!({ "state": "none" }),
            json!({ "state": "" }),
            json!({}),
            json!("nonsense"),
        ] {
            assert!(Verification::from_json(&nothing).is_none(), "{nothing}");
        }
        assert_eq!(
            unchecked_sentence(Some("ssh")),
            "Signed with an SSH key; not checked yet"
        );
        assert_eq!(unchecked_sentence(None), "Signed; not checked yet");
    }
}
