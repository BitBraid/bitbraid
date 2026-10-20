//! A signing key that an OpenPGP smart card holds, when gpg could not sign
//! with it: what is the matter with the card, in words of our own.
//!
//! What git hands on from gpg says little here. With the card out of its
//! reader, the reader or the system's smart card service gone, scdaemon
//! missing, or another card in the reader, gpg-agent has its pinentry ask
//! for the card to be put in; with the card there, it asks for the PIN.
//! Either question cancelled, gpg says only "Operation cancelled" ("No
//! pinentry" where there is nobody to ask); the first signing after the
//! card was taken out may end in "Card error" instead. A wrong PIN and a
//! PIN that is blocked are both "Bad PIN". So once signing has failed so, gpg is
//! asked whether the key is on a card (its listing of secret keys gives
//! such a key the serial number of the card) and, when it is, about the
//! card in the reader (`--card-status`): which card it is, the keys it
//! holds and the tries left for its PIN. Both are questions to the program
//! git signed with, in the same environment and bounded like every other
//! question to gpg here. `--card-status` has gpg-agent note the keys of the
//! card that is in, as it does whenever it is run.
//!
//! The card's number is said as gpg-agent's pinentry says it ("0000
//! 00000000": the maker and the serial number out of the card's
//! application identifier), so that it can be matched with what the
//! pinentry asked for.
//!
//! Sources: gpg(1) (`--card-status`, `--card-edit`, `--with-colons`),
//! GnuPG's doc/DETAILS (field 15 of a key record, the serial number of the
//! card that holds the key, `#` for a key that is not there; the records
//! of `--card-status --with-colons`: `Reader` with the application
//! identifier, `pinretry`, `fpr`), gpg-agent(1) (pinentry), scdaemon(1),
//! pcscd(8), the OpenPGP smart card application specification 3.4 (the
//! application identifier: maker and serial number; the PIN's retry
//! counter); and what GnuPG 2.4 says with an emulated card (an OpenPGP
//! card on a virtual reader of pcsc-lite).

use super::{Answer, Format, MAX_REASON, after, ask, committer, program};
use crate::{Repository, exe};

/// Reasons gpg gives for not signing that a card can be behind.
const CUES: [&str; 13] = [
    "Bad PIN",
    "PIN blocked",
    "No pinentry",
    "Inappropriate ioctl for device",
    "cancelled",
    "canceled",
    "Timeout",
    "No SmartCard daemon",
    "Card removed",
    "Card not present",
    "No such device",
    "Service is not running",
    "Card error",
];
/// Reasons among them that are a question nobody answered.
const UNANSWERED: [&str; 5] = [
    "No pinentry",
    "Inappropriate ioctl for device",
    "cancelled",
    "canceled",
    "Timeout",
];
/// Longest serial number or fingerprint taken from gpg, in characters.
const MAX_FIELD: usize = 64;
/// Most fingerprints taken from one listing.
const MAX_PRINTS: usize = 16;

/// The card a signing key is on, by gpg's listing of secret keys.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CardKey {
    /// The serial number gpg keeps for the card: its application
    /// identifier, for an OpenPGP card.
    serial: String,
    /// The fingerprints of the keys of that card that can sign.
    fingerprints: Vec<String>,
}

/// What `--card-status` told of the card in the reader.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Card {
    /// gpg could not say: it did not answer, or not as gpg does.
    Unknown,
    /// gpg-agent could not start scdaemon.
    NoDaemon,
    /// The system's smart card service is not running.
    NoService,
    /// There is no card in a reader.
    Absent,
    /// A card is in a reader.
    Present {
        serial: String,
        fingerprints: Vec<String>,
        /// Tries left for the PIN that signs, where the card says.
        tries: Option<u32>,
    },
}

/// A field of gpg's, cut to a sane length.
fn field(text: &str) -> String {
    text.trim().chars().take(MAX_FIELD).collect()
}

/// The card that holds a key gpg would sign with, out of a
/// `--list-secret-keys --with-colons` listing: the first key or subkey that
/// can sign (`s` among its own capabilities) and whose secret half is on a
/// card (a serial number in field 15). None where no such key is listed.
fn card_key(listing: &str) -> Option<CardKey> {
    let mut found: Option<CardKey> = None;
    // Whether the record just read is a signing key on the card found.
    let mut pending = false;
    for line in listing.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        let at = |n: usize| fields.get(n).copied().unwrap_or("");
        match at(0) {
            "sec" | "ssb" => {
                let serial = at(14).trim();
                let signs = at(11).contains('s');
                let on_card = !serial.is_empty() && serial != "#" && serial != "+";
                pending = false;
                if signs && on_card {
                    match &found {
                        None => {
                            found = Some(CardKey {
                                serial: field(serial),
                                fingerprints: Vec::new(),
                            });
                            pending = true;
                        }
                        Some(card) => pending = card.serial.eq_ignore_ascii_case(serial),
                    }
                }
            }
            "fpr" if pending => {
                pending = false;
                if let Some(card) = found.as_mut()
                    && !at(9).trim().is_empty()
                    && card.fingerprints.len() < MAX_PRINTS
                {
                    card.fingerprints.push(field(at(9)));
                }
            }
            "pub" | "sub" | "crs" | "crt" => pending = false,
            _ => {}
        }
    }
    found
}

/// What `--card-status --with-colons` said, and what gpg wrote besides.
fn read_card(answer: Option<&Answer>) -> Card {
    let Some(answer) = answer else {
        return Card::Unknown;
    };
    let mut serial = None;
    let mut fingerprints = Vec::new();
    let mut tries = None;
    for line in answer.stdout.lines() {
        let fields: Vec<&str> = line.split(':').collect();
        let at = |n: usize| fields.get(n).copied().unwrap_or("");
        match at(0) {
            // `Reader:<name>:AID:<identifier>:<kind>:`
            "Reader" => {
                let identifier = fields
                    .iter()
                    .position(|word| *word == "AID")
                    .and_then(|n| fields.get(n.saturating_add(1)))
                    .map(|identifier| identifier.trim())
                    .filter(|identifier| !identifier.is_empty());
                serial = identifier.map(field);
            }
            "pinretry" => tries = at(1).trim().parse().ok(),
            "fpr" => {
                for print in fields.iter().skip(1).map(|print| print.trim()) {
                    if !print.is_empty() && fingerprints.len() < MAX_PRINTS {
                        fingerprints.push(field(print));
                    }
                }
            }
            _ => {}
        }
    }
    if answer.ok
        && let Some(serial) = serial
    {
        return Card::Present {
            serial,
            fingerprints,
            tries,
        };
    }
    let said = &answer.stderr;
    if said.contains("No SmartCard daemon") {
        Card::NoDaemon
    } else if said.contains("Service is not running") {
        Card::NoService
    } else if ["No such device", "Card removed", "Card not present"]
        .iter()
        .any(|sign| said.contains(sign))
    {
        Card::Absent
    } else {
        Card::Unknown
    }
}

/// A card's number as gpg-agent's pinentry shows it: the maker and the
/// serial number of an OpenPGP card's application identifier
/// (`D276000124 01 0304 <maker> <serial> 0000`), else the serial as gpg
/// keeps it.
fn number(serial: &str) -> String {
    let openpgp = serial.len() == 32
        && serial.is_ascii()
        && serial
            .get(..10)
            .is_some_and(|rid| rid.eq_ignore_ascii_case("D276000124"));
    match (openpgp, serial.get(16..20), serial.get(20..28)) {
        (true, Some(maker), Some(number)) => format!("{maker} {number}"),
        _ => serial.to_owned(),
    }
}

/// Where the system's smart card service is named, its name.
fn service() -> &'static str {
    if cfg!(windows) {
        say!("signature-card-service-windows")
    } else if cfg!(target_os = "macos") {
        ""
    } else {
        " (pcscd)"
    }
}

/// What to tell the user: gpg gave up for `why` on a key that `key` says
/// is on a card, and `card` is what is in the reader.
fn words(key: &CardKey, card: &Card, why: &str) -> String {
    let why: String = why.chars().take(MAX_REASON).collect();
    let ours = number(&key.serial);
    let wrong_pin = why.contains("Bad PIN");
    let unanswered = UNANSWERED.iter().any(|sign| why.contains(sign));
    match card {
        Card::NoDaemon => say!("signature-card-no-daemon").to_owned(),
        Card::NoService => say!(
            "signature-card-no-service",
            card = ours.as_str(),
            service = service()
        ),
        Card::Absent => say!("signature-card-absent", card = ours.as_str()),
        Card::Present { serial, .. } if !serial.eq_ignore_ascii_case(&key.serial) => say!(
            "signature-card-another",
            inserted = number(serial),
            card = ours.as_str()
        ),
        Card::Present { fingerprints, .. }
            if !key.fingerprints.is_empty()
                && !key
                    .fingerprints
                    .iter()
                    .any(|ours| fingerprints.iter().any(|on| on.eq_ignore_ascii_case(ours))) =>
        {
            say!("signature-card-key-gone", card = ours.as_str())
        }
        Card::Present { tries: Some(0), .. } => {
            say!("signature-card-blocked", card = ours.as_str())
        }
        Card::Present { tries, .. } if wrong_pin => {
            let left = match tries {
                Some(n) => say!("signature-card-tries-left", tries = *n),
                None => String::new(),
            };
            say!(
                "signature-card-wrong-pin",
                card = ours.as_str(),
                why = why.as_str(),
                left
            )
        }
        Card::Present { .. } if unanswered => say!(
            "signature-card-pin-not-given",
            card = ours.as_str(),
            why = why.as_str()
        ),
        Card::Present { .. } => say!(
            "signature-card-could-not-sign",
            card = ours.as_str(),
            why = why.as_str()
        ),
        Card::Unknown if wrong_pin => say!(
            "signature-named-card-pin-refused",
            card = ours.as_str(),
            why = why.as_str()
        ),
        Card::Unknown if unanswered => say!(
            "signature-card-unusable-unanswered",
            card = ours.as_str(),
            why = why.as_str()
        ),
        Card::Unknown => say!(
            "signature-card-unusable",
            card = ours.as_str(),
            why = why.as_str()
        ),
    }
}

/// What is the matter, when git could not sign because gpg gave up on a
/// key that a smart card holds. None where it was not that: another
/// format, another reason, a key that is not on a card, or a program that
/// does not answer as gpg does.
pub(super) fn failure(repo: &Repository, said: &str, key: Option<&str>) -> Option<String> {
    let why = after(said, "signing failed: ")?;
    if !CUES.iter().any(|cue| why.contains(cue)) {
        return None;
    }
    let written = exe::config_get(repo, "gpg.format");
    if written.as_deref().is_some_and(|format| format != "openpgp") {
        return None;
    }
    let program = program(repo, Format::Openpgp);
    let who = key.map(str::to_owned).or_else(|| committer(repo))?;
    let listing = ask(
        repo,
        &program,
        [
            "--batch",
            "--no-tty",
            "--with-colons",
            "--list-secret-keys",
            "--",
            who.as_str(),
        ],
    )?;
    let card_key = card_key(&listing.stdout)?;
    let status = ask(
        repo,
        &program,
        ["--batch", "--no-tty", "--with-colons", "--card-status"],
    );
    Some(words(&card_key, &read_card(status.as_ref()), why))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The listing GnuPG 2.4 gave for keys made on an emulated card.
    const LISTING: &str = "sec:u:255:22:05CD90DC7EC9BFBD:1792233082:::u:::scESCA:::D2760001240103040000000000000000::ed25519:::0:\n\
        fpr:::::::::7A6929A8BA898A56619E701F05CD90DC7EC9BFBD:\n\
        grp:::::::::2B585AA3B4F8C857A328513108F58907505C6386:\n\
        uid:u::::1792233082::786A3D11567B28C1B1974D7B74A677F3814A7E4D::Ada Fixture <ada@fixture.invalid>::::::::::0:\n\
        ssb:u:255:22:34AD65C7C77E32E5:1792233082::::::a:::D2760001240103040000000000000000::ed25519::\n\
        fpr:::::::::7F1118E4C2CE62188646BDFA34AD65C7C77E32E5:\n\
        ssb:u:255:18:12F72DB8603B81F9:1792233082::::::e:::D2760001240103040000000000000000::cv25519::\n\
        fpr:::::::::328469A18E7CCCC00570136912F72DB8603B81F9:\n";
    const SERIAL: &str = "D2760001240103040000000000000000";
    const SIGNING: &str = "7A6929A8BA898A56619E701F05CD90DC7EC9BFBD";

    /// `--card-status --with-colons` with that card in the reader.
    fn status(tries: u32, prints: &str) -> Answer {
        Answer {
            ok: true,
            stdout: format!(
                "Reader:BitBraid test reader 00 00:AID:{SERIAL}:openpgp-card:\n\
                 version:0304:\nvendor:0000:test card:\nserial:00000000:\n\
                 keyattr:1:22:Ed25519:\npinretry:{tries}:0:3:\nsigcount:4:::\n\
                 fpr:{prints}\n"
            ),
            stderr: String::new(),
        }
    }

    fn failed(stderr: &str) -> Answer {
        Answer {
            ok: false,
            stdout: "AID:::\n".to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    #[test]
    fn the_card_of_a_signing_key_is_read_from_the_listing() {
        let card = card_key(LISTING).unwrap();
        assert_eq!(card.serial, SERIAL);
        assert_eq!(card.fingerprints, vec![SIGNING.to_owned()]);
        // A signing subkey on the card, the primary key elsewhere.
        let subkey = "sec:u:255:22:AAAA:1::::::scSC:::#::ed25519:\n\
            fpr:::::::::AAAA:\n\
            ssb:u:255:22:BBBB:1::::::s:::D2760001240103040006123456780000::ed25519:\n\
            fpr:::::::::BBBB:\n";
        let card = card_key(subkey).unwrap();
        assert_eq!(card.serial, "D2760001240103040006123456780000");
        assert_eq!(card.fingerprints, vec!["BBBB".to_owned()]);
        assert_eq!(number(&card.serial), "0006 12345678");
        // Keys on disk, a key that is nowhere, a card key that cannot sign.
        for listing in [
            "sec:u:255:22:AAAA:1:::u:::scSC:::+::ed25519:::0:\nfpr:::::::::AAAA:\n",
            "sec:u:255:22:AAAA:1:::u:::scSC:::::ed25519:::0:\nfpr:::::::::AAAA:\n",
            "sec:u:255:22:AAAA:1:::u:::scSC:::#::ed25519:::0:\nfpr:::::::::AAAA:\n",
            "ssb:u:255:18:AAAA:1::::::e:::D2760001240103040000000000000000::cv25519::\n",
            "",
            "garbage\n::::\n\0",
        ] {
            assert_eq!(card_key(listing), None, "{listing:?}");
        }
        // Cut anywhere, it is still only read.
        for cut in 0..LISTING.len() {
            let _ = card_key(LISTING.get(..cut).unwrap_or(""));
        }
    }

    #[test]
    fn what_is_in_the_reader_is_read_from_the_card_status() {
        let present = read_card(Some(&status(2, &format!("{SIGNING}:AB:CD:"))));
        assert_eq!(
            present,
            Card::Present {
                serial: SERIAL.to_owned(),
                fingerprints: vec![SIGNING.to_owned(), "AB".to_owned(), "CD".to_owned()],
                tries: Some(2),
            }
        );
        for (said, card) in [
            (
                "gpg: selecting card failed: No such device\ngpg: OpenPGP card not available: No such device\n",
                Card::Absent,
            ),
            (
                "gpg: selecting card failed: Service is not running\ngpg: OpenPGP card not available: Service is not running\n",
                Card::NoService,
            ),
            (
                "gpg: error getting version from 'scdaemon': No SmartCard daemon\ngpg: OpenPGP card not available: No SmartCard daemon\n",
                Card::NoDaemon,
            ),
            ("gpg: something else entirely\n", Card::Unknown),
            ("", Card::Unknown),
        ] {
            assert_eq!(read_card(Some(&failed(said))), card, "{said:?}");
        }
        assert_eq!(read_card(None), Card::Unknown);
        // A program that ends well and says nothing gpg would.
        let quiet = Answer {
            ok: true,
            stdout: "hello\n".to_owned(),
            stderr: String::new(),
        };
        assert_eq!(read_card(Some(&quiet)), Card::Unknown);
        let whole = status(3, "A:B:C:").stdout;
        for cut in 0..whole.len() {
            let answer = Answer {
                ok: true,
                stdout: whole.get(..cut).unwrap_or("").to_owned(),
                stderr: String::new(),
            };
            let _ = read_card(Some(&answer));
        }
    }

    #[test]
    fn each_trouble_with_the_card_is_said_in_words_of_its_own() {
        let key = card_key(LISTING).unwrap();
        let ours = |prints: &str, tries: u32| read_card(Some(&status(tries, prints)));
        let cancelled = "Operation cancelled";
        let cases: [(Card, &str, &[&str]); 9] = [
            (Card::NoDaemon, cancelled, &["scdaemon", "install"]),
            (
                Card::NoService,
                cancelled,
                &["0000 00000000", "smart card service", "not running"],
            ),
            (
                Card::Absent,
                cancelled,
                &["0000 00000000", "not in a reader", "put it in"],
            ),
            (
                Card::Present {
                    serial: "D2760001240103040006123456780000".to_owned(),
                    fingerprints: vec![SIGNING.to_owned()],
                    tries: Some(3),
                },
                cancelled,
                &["(0006 12345678) is not the one", "(0000 00000000)"],
            ),
            (
                ours("::", 3),
                cancelled,
                &["no longer holds the signing key"],
            ),
            (
                ours(&format!("{SIGNING}::"), 0),
                "Bad PIN",
                &["blocked", "Admin PIN"],
            ),
            (
                ours(&format!("{SIGNING}::"), 2),
                "Bad PIN",
                &[
                    "did not take the PIN (Bad PIN)",
                    "2 more wrong PINs block it",
                ],
            ),
            (
                ours(&format!("{SIGNING}::"), 3),
                cancelled,
                &["was not given (Operation cancelled)", "pinentry"],
            ),
            (
                Card::Unknown,
                cancelled,
                &["0000 00000000", "not in a reader, or its PIN was not given"],
            ),
        ];
        for (card, why, expected) in cases {
            let said = words(&key, &card, why);
            for part in expected {
                assert!(said.contains(part), "{card:?}: {said}");
            }
        }
        let flaky = words(&key, &ours(&format!("{SIGNING}::"), 3), "Card error");
        assert!(flaky.contains("could not sign (Card error)"), "{flaky}");
        let unknown = words(&key, &Card::Unknown, "Card error");
        assert!(
            unknown.contains("see that the card is in its reader"),
            "{unknown}"
        );
        let one = words(&key, &ours(&format!("{SIGNING}::"), 1), "Bad PIN");
        assert!(one.contains("one more wrong PIN blocks it"), "{one}");
        let unknown = words(&key, &Card::Unknown, "Bad PIN");
        assert!(unknown.contains("Admin PIN unblocks it"), "{unknown}");
        // A reason of any length is cut.
        let long = words(&key, &Card::Absent, &"x".repeat(10_000));
        assert!(long.len() < 1_000, "{}", long.len());
    }

    #[test]
    fn a_card_number_is_the_pinentry_s_and_otherwise_the_serial() {
        assert_eq!(number(SERIAL), "0000 00000000");
        assert_eq!(number("d2760001240103040006123456780000"), "0006 12345678");
        for other in ["", "1234", "D27600012401030400061234567800", "Zé"] {
            assert_eq!(number(other), other);
        }
        let multibyte = format!("D276000124{}", "é".repeat(11));
        assert_eq!(number(&multibyte), multibyte);
    }
}
