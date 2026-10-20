//! The system's keychain, for real: kept, read, replaced and forgotten.
//!
//! Through the Secret Service on Linux, never against the keychain of
//! whoever runs the tests. It runs only inside a Secret Service of its
//! own, with a scratch home, and refuses otherwise:
//!
//! ```sh
//! cargo test -p bitbraid-integrations --test suite --no-run   # note the binary
//! mkdir -p /tmp/bb-keychain && HOME=/tmp/bb-keychain XDG_DATA_HOME=/tmp/bb-keychain/data \
//!   dbus-run-session -- sh -c 'printf pw | gnome-keyring-daemon --unlock --components=secrets >/dev/null;
//!     BITBRAID_TEST_KEYCHAIN=scratch <binary> --ignored keychain::'
//! ```
//!
//! Through Credential Manager on Windows and the login keychain on macOS,
//! which have no second one to run in: in the user's own, under a service
//! name of the test's own (`bitbraid-test-<process>-<nanoseconds>`) that
//! nothing else uses, and everything it keeps there is removed again when
//! it ends, whether it passed or not.

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos",
    windows
))]
use bitbraid_integrations::secrets::{Keychain, Secret, SecretStore};

/// Whether this runs in a Secret Service of its own: asked for, and on a
/// session bus that is not the user's.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn scratch_service() -> bool {
    let asked = std::env::var("BITBRAID_TEST_KEYCHAIN").as_deref() == Ok("scratch");
    let bus = std::env::var("DBUS_SESSION_BUS_ADDRESS").unwrap_or_default();
    asked && !bus.is_empty() && !bus.contains("/run/user/")
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[test]
#[ignore = "needs a Secret Service of its own: see the head of this file"]
fn the_system_keychain_keeps_replaces_and_forgets() {
    assert!(
        scratch_service(),
        "refusing: this runs only in a Secret Service of its own (see the head of this file)"
    );
    let keychain = Keychain::for_service("bitbraid-keychain-test");
    let account = "github:github.com:keychain-test";
    keychain.delete(account).unwrap();
    assert_eq!(keychain.get(account).unwrap(), None);
    keychain
        .set(account, "BitBraid test", &Secret::new("first-value"))
        .unwrap();
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "first-value"
    );
    // Kept again: replaced, not added beside.
    keychain
        .set(account, "BitBraid test", &Secret::new("second-value"))
        .unwrap();
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "second-value"
    );
    // Another account and another service are other items.
    let other = Keychain::for_service("bitbraid-keychain-test-2");
    assert_eq!(other.get(account).unwrap(), None);
    keychain
        .set(
            "gitlab:gitlab.com:else",
            "BitBraid test",
            &Secret::new("else"),
        )
        .unwrap();
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "second-value"
    );
    keychain.delete(account).unwrap();
    keychain.delete("gitlab:gitlab.com:else").unwrap();
    assert_eq!(keychain.get(account).unwrap(), None);
    // Forgetting what is not there is no error.
    keychain.delete(account).unwrap();
}

/// The accounts the Windows test keeps tokens for.
#[cfg(windows)]
const ACCOUNTS: [&str; 3] = [
    "github:github.com:keychain-test",
    "gitlab:gitlab.com:else",
    "github:github.com:long",
];

/// What the Windows test keeps in Credential Manager, removed when the
/// test ends, whether it passed or not.
#[cfg(windows)]
struct Kept {
    service: String,
}

#[cfg(windows)]
impl Kept {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self {
            service: format!("bitbraid-test-{}-{nanos}", std::process::id()),
        }
    }

    fn keychain(&self) -> Keychain {
        Keychain::for_service(&self.service)
    }

    fn target(&self, account: &str) -> String {
        format!("{}:{account}", self.service)
    }

    /// The credential as Windows keeps it, asked for by its target name
    /// without this crate's code.
    fn credential(&self, account: &str) -> Option<keyring::windows::WinCredential> {
        let wanted = keyring::windows::WinCredential {
            username: String::new(),
            target_name: self.target(account),
            target_alias: String::new(),
            comment: String::new(),
        };
        match wanted.get_credential() {
            Ok(found) => Some(found),
            Err(keyring::Error::NoEntry) => None,
            Err(err) => panic!("{}: {err}", self.target(account)),
        }
    }

    /// Whether Windows' own `cmdkey` lists a credential by this name, as
    /// Credential Manager's window does. Its heading names what was asked
    /// for and ends with a colon; a line for a credential found ends with
    /// its name.
    fn listed(&self, account: &str) -> bool {
        let target = self.target(account);
        let out = std::process::Command::new("cmdkey")
            .arg(format!("/list:{target}"))
            .output()
            .unwrap();
        assert!(out.status.success(), "cmdkey /list:{target}");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|line| line.trim_end().ends_with(&format!(" {target}")))
    }
}

#[cfg(windows)]
impl Drop for Kept {
    fn drop(&mut self) {
        let keychain = self.keychain();
        for account in ACCOUNTS {
            let _ = keychain.delete(account);
        }
    }
}

#[cfg(windows)]
#[test]
fn credential_manager_keeps_replaces_and_forgets() {
    let kept = Kept::new();
    let keychain = kept.keychain();
    let [account, else_account, long_account] = ACCOUNTS;
    let label = "BitBraid: keychain-test at github.com";

    // Nothing kept: nothing read, and forgetting it is no error.
    assert_eq!(keychain.get(account).unwrap(), None);
    keychain.delete(account).unwrap();

    keychain
        .set(account, label, &Secret::new("first-value"))
        .unwrap();
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "first-value"
    );
    // Listed under `<service>:<account>`, with the account as its user
    // name and the label as its comment.
    let found = kept.credential(account).unwrap();
    assert_eq!(found.target_name, kept.target(account));
    assert_eq!(found.username, account);
    assert_eq!(found.comment, label);
    assert!(kept.listed(account), "{}", kept.target(account));

    // Kept again: replaced, label and all, not added beside.
    keychain
        .set(account, "BitBraid: renamed", &Secret::new("second-value"))
        .unwrap();
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "second-value"
    );
    assert_eq!(
        kept.credential(account).unwrap().comment,
        "BitBraid: renamed"
    );

    // Another service and another account are other credentials.
    let other = Keychain::for_service(&format!("{}-2", kept.service));
    assert_eq!(other.get(account).unwrap(), None);
    keychain
        .set(else_account, label, &Secret::new("else-ünï-鍵-🦑"))
        .unwrap();
    assert_eq!(
        keychain.get(else_account).unwrap().unwrap().expose(),
        "else-ünï-鍵-🦑"
    );
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "second-value"
    );

    // The longest token Credential Manager keeps, and a label longer than
    // a comment may be, cut between two characters.
    let longest = "tok3n".repeat(256);
    let long_label = format!("BitBraid: {}", "é".repeat(300));
    keychain
        .set(long_account, &long_label, &Secret::new(longest.clone()))
        .unwrap();
    assert_eq!(
        keychain.get(long_account).unwrap().unwrap().expose(),
        longest
    );
    let comment = kept.credential(long_account).unwrap().comment;
    assert!(
        comment.len() > 250 && comment.len() <= 256 && long_label.starts_with(&comment),
        "{comment}"
    );
    // One that is longer is refused, and the error does not hold it; the
    // one kept before stays.
    let err = keychain
        .set(long_account, label, &Secret::new("tok3n".repeat(257)))
        .unwrap_err();
    assert!(!err.to_string().contains("tok3n"), "{err}");
    assert!(!format!("{err:?}").contains("tok3n"), "{err:?}");
    assert_eq!(
        keychain.get(long_account).unwrap().unwrap().expose(),
        longest
    );

    // Forgotten, and forgetting again is no error.
    for account in ACCOUNTS {
        keychain.delete(account).unwrap();
        assert_eq!(keychain.get(account).unwrap(), None);
        assert!(kept.credential(account).is_none(), "{account}");
        keychain.delete(account).unwrap();
    }
    assert!(!kept.listed(account));

    // What was kept is gone when the test ends.
    let service = kept.service.clone();
    drop(kept);
    let after = Keychain::for_service(&service);
    for account in ACCOUNTS {
        assert_eq!(after.get(account).unwrap(), None, "{account}");
    }
}

/// A test that fails half-way leaves nothing behind either.
#[cfg(windows)]
#[test]
fn credential_manager_is_cleaned_after_a_test_that_fails() {
    let (service, outcome) = {
        let kept = Kept::new();
        let service = kept.service.clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let keychain = kept.keychain();
            for account in ACCOUNTS {
                keychain
                    .set(account, "BitBraid test", &Secret::new("left-behind?"))
                    .unwrap();
            }
            assert!(keychain.get(ACCOUNTS[0]).unwrap().is_some());
            panic!("an assertion that fails, on purpose");
        }));
        (service, outcome)
    };
    assert!(outcome.is_err());
    let after = Keychain::for_service(&service);
    for account in ACCOUNTS {
        assert_eq!(after.get(account).unwrap(), None, "{account}");
    }
}

/// What the macOS test keeps in the login keychain, removed when the test
/// ends, whether it passed or not.
#[cfg(target_os = "macos")]
struct KeptOnMac {
    service: String,
}

#[cfg(target_os = "macos")]
const MAC_ACCOUNTS: [&str; 3] = [
    "github:github.com:keychain-test",
    "gitlab:gitlab.com:else",
    "github:github.com:long",
];

#[cfg(target_os = "macos")]
impl KeptOnMac {
    fn new() -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self {
            service: format!("bitbraid-test-{}-{nanos}", std::process::id()),
        }
    }

    fn keychain(&self) -> Keychain {
        Keychain::for_service(&self.service)
    }

    /// The item as the system's `security` lists it, without this crate's
    /// code and without its secret: its attributes, or none.
    fn listed(&self, account: &str) -> Option<String> {
        let out = std::process::Command::new("security")
            .args(["find-generic-password", "-s", &self.service, "-a", account])
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

#[cfg(target_os = "macos")]
impl Drop for KeptOnMac {
    fn drop(&mut self) {
        let keychain = self.keychain();
        for account in MAC_ACCOUNTS {
            let _ = keychain.delete(account);
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn the_login_keychain_keeps_replaces_and_forgets() {
    let kept = KeptOnMac::new();
    let keychain = kept.keychain();
    let [account, else_account, long_account] = MAC_ACCOUNTS;
    let label = "BitBraid: keychain-test at github.com";

    // Nothing kept: nothing read, and forgetting it is no error.
    assert_eq!(keychain.get(account).unwrap(), None);
    keychain.delete(account).unwrap();
    assert!(kept.listed(account).is_none());

    keychain
        .set(account, label, &Secret::new("first-value"))
        .unwrap();
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "first-value"
    );
    // Listed by its service and account, under the label, without the
    // secret.
    let listed = kept.listed(account).unwrap();
    assert!(
        listed.contains(&format!("\"svce\"<blob>=\"{}\"", kept.service)),
        "{listed}"
    );
    assert!(
        listed.contains(&format!("\"acct\"<blob>=\"{account}\"")),
        "{listed}"
    );
    // The label is attribute 7 (kSecLabelItemAttr), which `security`
    // prints by its number.
    assert!(
        listed.contains(&format!("0x00000007 <blob>=\"{label}\"")),
        "{listed}"
    );
    assert!(!listed.contains("first-value"), "{listed}");

    // Kept again: replaced, not added beside, under another label too.
    keychain
        .set(account, "BitBraid: renamed", &Secret::new("second-value"))
        .unwrap();
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "second-value"
    );
    keychain
        .set(account, label, &Secret::new("third-value"))
        .unwrap();
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "third-value"
    );

    // Another service and another account are other items.
    let other = Keychain::for_service(&format!("{}-2", kept.service));
    assert_eq!(other.get(account).unwrap(), None);
    keychain
        .set(else_account, label, &Secret::new("else-ünï-鍵-🦑"))
        .unwrap();
    assert_eq!(
        keychain.get(else_account).unwrap().unwrap().expose(),
        "else-ünï-鍵-🦑"
    );
    assert_eq!(
        keychain.get(account).unwrap().unwrap().expose(),
        "third-value"
    );

    // A long token, and a long label.
    let longest = "tok3n".repeat(4096);
    let long_label = format!("BitBraid: {}", "é".repeat(300));
    keychain
        .set(long_account, &long_label, &Secret::new(longest.clone()))
        .unwrap();
    assert_eq!(
        keychain.get(long_account).unwrap().unwrap().expose(),
        longest
    );

    // Forgotten, and forgetting again is no error.
    for account in MAC_ACCOUNTS {
        keychain.delete(account).unwrap();
        assert_eq!(keychain.get(account).unwrap(), None);
        assert!(kept.listed(account).is_none(), "{account}");
        keychain.delete(account).unwrap();
    }

    // What was kept is gone when the test ends.
    let service = kept.service.clone();
    drop(kept);
    let after = Keychain::for_service(&service);
    for account in MAC_ACCOUNTS {
        assert_eq!(after.get(account).unwrap(), None, "{account}");
    }
}

/// A test that fails half-way leaves nothing behind either.
#[cfg(target_os = "macos")]
#[test]
fn the_login_keychain_is_cleaned_after_a_test_that_fails() {
    let (service, outcome) = {
        let kept = KeptOnMac::new();
        let service = kept.service.clone();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
            let keychain = kept.keychain();
            for account in MAC_ACCOUNTS {
                keychain
                    .set(account, "BitBraid test", &Secret::new("left-behind?"))
                    .unwrap();
            }
            assert!(keychain.get(MAC_ACCOUNTS[0]).unwrap().is_some());
            panic!("an assertion that fails, on purpose");
        }));
        (service, outcome)
    };
    assert!(outcome.is_err());
    let after = Keychain::for_service(&service);
    for account in MAC_ACCOUNTS {
        assert_eq!(after.get(account).unwrap(), None, "{account}");
    }
}
