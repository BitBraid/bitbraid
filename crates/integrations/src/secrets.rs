//! Where the tokens of accounts are kept: the system's keychain, and
//! nowhere else (`AGENTS.md` section 2).
//!
//! [`SecretStore`] is what the rest of the crate asks: a secret by the id
//! of the account it belongs to, kept with a label a person can read in the
//! keychain's own window. [`Keychain`] is the real one. On Linux and
//! FreeBSD it is the Secret Service (GNOME Keyring, KWallet's bridge,
//! KeePassXC) over D-Bus, through `oo7`: one item per account in the
//! default collection, found by two attributes, `service` (`bitbraid`) and
//! `account` (the account's id). Where the collection is locked the system
//! asks the user to unlock it, as it does for every other application. On
//! Windows it is the Credential Manager, through `keyring`: one generic
//! credential per account, named `<service>:<account>`
//! (`bitbraid:github:github.com:ada`), which is the name Credential Manager
//! lists it by. On macOS it is the login keychain, through
//! `security-framework`: one generic password per account, found by its
//! service (`bitbraid`) and its account (the account's id), labelled for
//! Keychain Access; where the keychain is locked the system asks the user
//! to unlock it. On the other systems this build has no keychain it can
//! reach, and it says so rather than keep a token anywhere else.
//! [`MemoryStore`] is a stand-in for the tests.
//!
//! [`Secret`] holds the value. Its `Debug` and `Display` texts never show
//! it, so that a token cannot end up in a log, an error or a panic text by
//! being printed with what holds it.
//!
//! Sources: the Secret Service API specification (freedesktop.org,
//! "Secret Service API Draft" 0.2: collections, items, attributes,
//! `CreateItem` with replace, `SearchItems`, prompts and the `default`
//! alias); the documentation of `oo7` 0.6 (`dbus::Service`,
//! `Collection::create_item`, `search_items`, `Item::secret`, `delete`)
//! and of `async-io` (`block_on`); Microsoft's documentation of the
//! Credential Manager's functions (`CredWriteW`, `CredReadW`,
//! `CredDeleteW`, the `CREDENTIALW` structure: generic credentials, the
//! target name, the user name, the comment and how long each may be,
//! `ERROR_NOT_FOUND`, `ERROR_NO_SUCH_LOGON_SESSION`) and the documentation
//! of `keyring` 3.6 (`Entry::new_with_credential`, `windows::WinCredential`,
//! `Error`); Apple's Keychain Services documentation (`SecItemAdd`,
//! `SecItemCopyMatching`, `SecItemUpdate`, `SecItemDelete`, generic
//! passwords found by `kSecAttrService` and `kSecAttrAccount`,
//! `kSecAttrLabel`, and the result codes `errSecItemNotFound`,
//! `errSecInteractionNotAllowed`, `errSecAuthFailed`, `errSecUserCanceled`,
//! `errSecNotAvailable` and `errSecNoSuchKeychain`) and the documentation
//! of `security-framework` 3.7 (`passwords`, `PasswordOptions`).

use std::{collections::BTreeMap, fmt, sync::Mutex};

/// A value that signs in: a token. Shown as `<hidden>` wherever it is
/// printed.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Hold `value`.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The value itself, for the one place that sends it.
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Whether there is nothing in it.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<hidden>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(say!("secrets-hidden"))
    }
}

/// Why the keychain did not do what was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SecretError {
    /// There is no keychain this build can reach.
    #[error("{}", say!("secrets-unavailable", detail = .0.as_str()))]
    Unavailable(String),
    /// The keychain is locked and was not unlocked.
    #[error("{}", say!("secrets-locked"))]
    Locked,
    /// The keychain answered with an error.
    #[error("{}", say!("secrets-refused", detail = .0.as_str()))]
    Refused(String),
    /// What is kept there is not text.
    #[error("{}", say!("secrets-unreadable"))]
    Unreadable,
}

/// A keychain: secrets by account id.
pub trait SecretStore: Send + Sync + fmt::Debug {
    /// The secret kept for this account, if there is one.
    fn get(&self, account: &str) -> Result<Option<Secret>, SecretError>;
    /// Keep `secret` for this account, in place of one kept before.
    /// `label` is what the keychain's own window shows for it (Windows'
    /// keeps it and does not show it).
    fn set(&self, account: &str, label: &str, secret: &Secret) -> Result<(), SecretError>;
    /// Remove what is kept for this account; nothing kept is no error.
    fn delete(&self, account: &str) -> Result<(), SecretError>;
}

/// The value of the `service` attribute of every item this application
/// keeps.
pub const SERVICE: &str = "bitbraid";

/// The system's keychain, for items of one service name.
#[derive(Debug, Clone)]
pub struct Keychain {
    service: String,
}

impl Keychain {
    /// The keychain, for this application's items ([`SERVICE`]).
    pub fn new() -> Self {
        Self::for_service(SERVICE)
    }

    /// The keychain, for items of another service name: the live tests
    /// keep their accounts' tokens under `bitbraid-test`.
    pub fn for_service(service: &str) -> Self {
        Self {
            service: service.to_owned(),
        }
    }
}

impl Default for Keychain {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
mod system {
    //! The Secret Service, asked through `oo7` and waited on here.

    use oo7::dbus::{Collection, Error, Service, ServiceError};

    use super::{Secret, SecretError};

    fn failed(err: Error) -> SecretError {
        match err {
            Error::Service(ServiceError::IsLocked(_)) => SecretError::Locked,
            Error::Dismissed => SecretError::Locked,
            Error::ZBus(err) => SecretError::Unavailable(err.to_string()),
            other => SecretError::Refused(other.to_string()),
        }
    }

    /// The default collection, unlocked.
    async fn collection() -> Result<Collection, SecretError> {
        let service = Service::new().await.map_err(failed)?;
        let collection = service.default_collection().await.map_err(failed)?;
        if collection.is_locked().await.map_err(failed)? {
            collection.unlock(None).await.map_err(failed)?;
        }
        Ok(collection)
    }

    fn attributes<'a>(service: &'a str, account: &'a str) -> [(&'a str, &'a str); 2] {
        [("service", service), ("account", account)]
    }

    pub(super) fn get(service: &str, account: &str) -> Result<Option<Secret>, SecretError> {
        async_io::block_on(async {
            let collection = collection().await?;
            let items = collection
                .search_items(&attributes(service, account))
                .await
                .map_err(failed)?;
            let Some(item) = items.first() else {
                return Ok(None);
            };
            if item.is_locked().await.map_err(failed)? {
                item.unlock(None).await.map_err(failed)?;
            }
            let secret = item.secret().await.map_err(failed)?;
            let text = std::str::from_utf8(&secret).map_err(|_| SecretError::Unreadable)?;
            Ok(Some(Secret::new(text)))
        })
    }

    pub(super) fn set(
        service: &str,
        account: &str,
        label: &str,
        secret: &Secret,
    ) -> Result<(), SecretError> {
        async_io::block_on(async {
            let collection = collection().await?;
            collection
                .create_item(
                    label,
                    &attributes(service, account),
                    oo7::Secret::text(secret.expose()),
                    true,
                    None,
                )
                .await
                .map_err(failed)?;
            Ok(())
        })
    }

    pub(super) fn delete(service: &str, account: &str) -> Result<(), SecretError> {
        async_io::block_on(async {
            let collection = collection().await?;
            let items = collection
                .search_items(&attributes(service, account))
                .await
                .map_err(failed)?;
            for item in items {
                item.delete(None).await.map_err(failed)?;
            }
            Ok(())
        })
    }
}

#[cfg(windows)]
mod system {
    //! Windows' Credential Manager, through `keyring`, which makes the
    //! calls that cannot be made here without `unsafe`.
    //!
    //! One generic credential per account. It is found by its target name,
    //! `<service>:<account>`, and that is also the name Credential
    //! Manager's window (Control Panel, Windows Credentials, Generic
    //! Credentials) lists it by; its user name is the account's id. The
    //! label is kept as the credential's comment, cut to the 256 bytes a
    //! comment may take: that window does not show comments, so the target
    //! name is what a person reads there. The token is kept as UTF-16
    //! text, the way Windows keeps a password typed into that window, and
    //! may be 1280 of its units long. Keeping one again replaces it. It is
    //! kept for this user, and goes with the user's profile where the
    //! profile roams, as `keyring` keeps every credential.

    use keyring::{Entry, Error, windows::WinCredential};

    use super::{Secret, SecretError};

    /// The longest comment, in the bytes `keyring` counts.
    const LONGEST_COMMENT: usize = 256;
    /// The longest user name, in the bytes `keyring` counts.
    const LONGEST_USER: usize = 513;

    /// The name a credential is found by and listed under.
    pub(super) fn target(service: &str, account: &str) -> String {
        format!("{service}:{account}")
    }

    /// The beginning of `text` that takes at most `longest` bytes, cut
    /// between two characters.
    pub(super) fn at_most(text: &str, longest: usize) -> &str {
        let mut end = text.len().min(longest);
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        text.get(..end).unwrap_or("")
    }

    fn entry(service: &str, account: &str, label: &str) -> Result<Entry, SecretError> {
        // With a colon in it, `a:b` and `c` would name the same credential
        // as `a` and `b:c`.
        if service.contains(':') {
            return Err(SecretError::Refused(
                say!("secrets-colon-in-service").to_owned(),
            ));
        }
        Ok(Entry::new_with_credential(Box::new(WinCredential {
            username: at_most(account, LONGEST_USER).to_owned(),
            target_name: target(service, account),
            target_alias: String::new(),
            comment: at_most(label, LONGEST_COMMENT).to_owned(),
        })))
    }

    /// What went wrong, in words that never hold the token: `keyring`
    /// keeps the bytes it could not read in its error, and they are
    /// dropped here.
    fn failed(err: Error) -> SecretError {
        match err {
            Error::NoStorageAccess(err) => SecretError::Unavailable(say!(
                "secrets-windows-unreachable",
                error = err.to_string()
            )),
            Error::BadEncoding(_) => SecretError::Unreadable,
            Error::TooLong(what, longest) => {
                SecretError::Refused(say!("secrets-windows-too-long", what, longest))
            }
            Error::PlatformFailure(err) => {
                SecretError::Refused(say!("secrets-windows-failed", error = err.to_string()))
            }
            other => SecretError::Refused(other.to_string()),
        }
    }

    pub(super) fn get(service: &str, account: &str) -> Result<Option<Secret>, SecretError> {
        match entry(service, account, "")?.get_password() {
            Ok(text) => Ok(Some(Secret::new(text))),
            Err(Error::NoEntry) => Ok(None),
            Err(err) => Err(failed(err)),
        }
    }

    pub(super) fn set(
        service: &str,
        account: &str,
        label: &str,
        secret: &Secret,
    ) -> Result<(), SecretError> {
        entry(service, account, label)?
            .set_password(secret.expose())
            .map_err(failed)
    }

    pub(super) fn delete(service: &str, account: &str) -> Result<(), SecretError> {
        match entry(service, account, "")?.delete_credential() {
            Ok(()) | Err(Error::NoEntry) => Ok(()),
            Err(err) => Err(failed(err)),
        }
    }
}

#[cfg(target_os = "macos")]
mod system {
    //! macOS's login keychain, through `security-framework`.

    use security_framework::{
        base::Error,
        passwords::{
            PasswordOptions, delete_generic_password, generic_password, set_generic_password,
            set_generic_password_options,
        },
    };

    use super::{Secret, SecretError};

    /// No item for the service and account (`errSecItemNotFound`).
    const NOT_FOUND: i32 = -25300;

    /// What went wrong, in words that never hold the token.
    fn failed(err: Error) -> SecretError {
        match err.code() {
            // errSecInteractionNotAllowed: locked, and it may not ask now;
            // errSecAuthFailed: not unlocked; errSecUserCanceled: the user
            // said no.
            -25308 | -25293 | -128 => SecretError::Locked,
            // errSecNotAvailable, errSecNoSuchKeychain: no keychain.
            -25291 | -25294 => {
                SecretError::Unavailable(say!("secrets-keychain-says", error = err.to_string()))
            }
            _ => SecretError::Refused(say!("secrets-keychain-says", error = err.to_string())),
        }
    }

    pub(super) fn get(service: &str, account: &str) -> Result<Option<Secret>, SecretError> {
        match generic_password(PasswordOptions::new_generic_password(service, account)) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(|text| Some(Secret::new(text)))
                .map_err(|_| SecretError::Unreadable),
            Err(err) if err.code() == NOT_FOUND => Ok(None),
            Err(err) => Err(failed(err)),
        }
    }

    pub(super) fn set(
        service: &str,
        account: &str,
        label: &str,
        secret: &Secret,
    ) -> Result<(), SecretError> {
        let mut options = PasswordOptions::new_generic_password(service, account);
        options.set_label(label);
        match set_generic_password_options(secret.expose().as_bytes(), options) {
            Ok(()) => Ok(()),
            // An item kept before under another label: the label is a part
            // of what the update looks for, so it finds none. The value is
            // replaced, and the old label stays.
            Err(err) if err.code() == NOT_FOUND => {
                set_generic_password(service, account, secret.expose().as_bytes()).map_err(failed)
            }
            Err(err) => Err(failed(err)),
        }
    }

    pub(super) fn delete(service: &str, account: &str) -> Result<(), SecretError> {
        match delete_generic_password(service, account) {
            Ok(()) => Ok(()),
            Err(err) if err.code() == NOT_FOUND => Ok(()),
            Err(err) => Err(failed(err)),
        }
    }
}

#[cfg(not(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "macos",
    windows
)))]
mod system {
    //! No keychain this build can reach.

    use super::{Secret, SecretError};

    fn none() -> SecretError {
        SecretError::Unavailable(say!("secrets-no-keychain").to_owned())
    }

    pub(super) fn get(_: &str, _: &str) -> Result<Option<Secret>, SecretError> {
        Err(none())
    }

    pub(super) fn set(_: &str, _: &str, _: &str, _: &Secret) -> Result<(), SecretError> {
        Err(none())
    }

    pub(super) fn delete(_: &str, _: &str) -> Result<(), SecretError> {
        Err(none())
    }
}

impl SecretStore for Keychain {
    fn get(&self, account: &str) -> Result<Option<Secret>, SecretError> {
        system::get(&self.service, account)
    }

    fn set(&self, account: &str, label: &str, secret: &Secret) -> Result<(), SecretError> {
        system::set(&self.service, account, label, secret)
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        system::delete(&self.service, account)
    }
}

/// A keychain in memory, for the tests. It forgets everything when it
/// goes.
#[derive(Default)]
pub struct MemoryStore {
    items: Mutex<BTreeMap<String, (String, Secret)>>,
}

impl MemoryStore {
    /// An empty one.
    pub fn new() -> Self {
        Self::default()
    }

    /// The label kept with an account's secret.
    pub fn label(&self, account: &str) -> Option<String> {
        let items = self.items.lock().ok()?;
        items.get(account).map(|(label, _)| label.clone())
    }

    /// The accounts something is kept for.
    pub fn accounts(&self) -> Vec<String> {
        self.items
            .lock()
            .map(|items| items.keys().cloned().collect())
            .unwrap_or_default()
    }
}

impl fmt::Debug for MemoryStore {
    /// The accounts, and nothing that is kept for them.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MemoryStore")
            .field("accounts", &self.accounts())
            .finish()
    }
}

fn poisoned() -> SecretError {
    SecretError::Refused(say!("secrets-poisoned").to_owned())
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<Secret>, SecretError> {
        let items = self.items.lock().map_err(|_| poisoned())?;
        Ok(items.get(account).map(|(_, secret)| secret.clone()))
    }

    fn set(&self, account: &str, label: &str, secret: &Secret) -> Result<(), SecretError> {
        let mut items = self.items.lock().map_err(|_| poisoned())?;
        items.insert(account.to_owned(), (label.to_owned(), secret.clone()));
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretError> {
        let mut items = self.items.lock().map_err(|_| poisoned())?;
        items.remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_never_printed() {
        let secret = Secret::new("ghp_0123456789abcdef");
        assert_eq!(format!("{secret}"), "<hidden>");
        assert_eq!(format!("{secret:?}"), "Secret(<hidden>)");
        assert_eq!(
            format!("{:?}", Some(vec![secret.clone()])),
            "Some([Secret(<hidden>)])"
        );
        assert_eq!(secret.expose(), "ghp_0123456789abcdef");
    }

    #[test]
    fn the_memory_store_keeps_replaces_and_forgets() {
        let store = MemoryStore::new();
        assert_eq!(store.get("a").unwrap(), None);
        store.set("a", "first", &Secret::new("one")).unwrap();
        store.set("a", "second", &Secret::new("two")).unwrap();
        store.set("b", "other", &Secret::new("three")).unwrap();
        assert_eq!(store.get("a").unwrap(), Some(Secret::new("two")));
        assert_eq!(store.label("a").as_deref(), Some("second"));
        assert!(!format!("{store:?}").contains("two"));
        store.delete("a").unwrap();
        store.delete("a").unwrap();
        assert_eq!(store.get("a").unwrap(), None);
        assert_eq!(store.accounts(), ["b"]);
    }

    #[cfg(windows)]
    #[test]
    fn a_credential_is_named_by_service_and_account_and_cut_between_characters() {
        assert_eq!(
            system::target("bitbraid", "github:github.com:ada"),
            "bitbraid:github:github.com:ada"
        );
        assert_eq!(system::at_most("short", 256), "short");
        assert_eq!(system::at_most("abcdef", 3), "abc");
        assert_eq!(system::at_most("", 3), "");
        // é is two bytes: never half of one.
        assert_eq!(system::at_most("aéé", 4), "aé");
        assert_eq!(system::at_most("aéé", 3), "aé");
        assert_eq!(system::at_most("aéé", 2), "a");
        assert_eq!(system::at_most("🦑", 3), "");
        let long = "é".repeat(300);
        let cut = system::at_most(&long, 256);
        assert_eq!(cut.len(), 256);
        assert!(long.starts_with(cut));
        // A service with a colon would share its names with another.
        let err = Keychain::for_service("a:b").get("c").unwrap_err();
        assert!(matches!(err, SecretError::Refused(_)), "{err:?}");
    }
}
