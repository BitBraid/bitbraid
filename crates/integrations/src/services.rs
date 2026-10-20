//! What a provider command needs besides its parameters: the network,
//! the keychain, where the accounts are written down, the application ids
//! of the device flow, where answers are kept and the clock. One
//! [`Services`] value holds them.
//!
//! A command takes it from the context's slot
//! ([`CommandContext::extensions`], `AGENTS.md` section 2) when the
//! application or a test put one there, and otherwise uses the one of the
//! context's configuration directory, made once per process: the real
//! client, the system's keychain and the application's cache directory
//! (the one of [`cache_dir`]). A test of the window, whose sessions make
//! their own contexts, installs stand-ins for its scratch configuration
//! directory instead ([`Services::install`]).
//!
//! Sources: none (original design).

use std::{
    collections::BTreeMap,
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use bitbraid_engine::CommandContext;

use crate::{
    api::ProviderError,
    http::{Client, Fetch},
    oauth,
    secrets::{Keychain, SecretStore},
};

/// The application ids of the device flow at github.com and gitlab.com.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientIds {
    /// At github.com.
    pub github: String,
    /// At gitlab.com.
    pub gitlab: String,
}

impl ClientIds {
    /// The ids built in, or those of `BITBRAID_GITHUB_CLIENT_ID` and
    /// `BITBRAID_GITLAB_CLIENT_ID` where they are set.
    pub fn from_environment() -> Self {
        let read = |name: &str, built_in: &str| {
            std::env::var(name)
                .ok()
                .map(|id| id.trim().to_owned())
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| built_in.to_owned())
        };
        Self {
            github: read("BITBRAID_GITHUB_CLIENT_ID", oauth::GITHUB_CLIENT_ID),
            gitlab: read("BITBRAID_GITLAB_CLIENT_ID", oauth::GITLAB_CLIENT_ID),
        }
    }
}

/// Everything a provider command uses besides its parameters.
pub struct Services {
    /// The network.
    pub fetch: Arc<dyn Fetch>,
    /// The keychain.
    pub secrets: Arc<dyn SecretStore>,
    /// Where `integrations.toml` is.
    pub config_dir: PathBuf,
    /// Where the answers of reads are kept, `providers/` in it; none
    /// are kept without one.
    pub cache_dir: Option<PathBuf>,
    /// The device flow's application ids at github.com and gitlab.com.
    pub client_ids: ClientIds,
    /// Seconds since the Unix epoch.
    pub now: fn() -> i64,
}

impl fmt::Debug for Services {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Services")
            .field("fetch", &self.fetch)
            .field("secrets", &self.secrets)
            .field("config_dir", &self.config_dir)
            .field("cache_dir", &self.cache_dir)
            .field("client_ids", &self.client_ids)
            .finish()
    }
}

/// The services of each configuration directory, made or installed.
fn made() -> &'static Mutex<BTreeMap<PathBuf, Arc<Services>>> {
    static MADE: OnceLock<Mutex<BTreeMap<PathBuf, Arc<Services>>>> = OnceLock::new();
    MADE.get_or_init(Mutex::default)
}

/// What the application calls itself when it asks a provider.
pub fn agent() -> String {
    format!("BitBraid/{}", env!("CARGO_PKG_VERSION"))
}

/// The application's cache directory, as the window's is.
pub fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "bitbraid").map(|dirs| dirs.cache_dir().to_path_buf())
}

impl Services {
    /// These parts, with the ids of the environment and the real clock,
    /// and no cache directory.
    pub fn new(
        fetch: Arc<dyn Fetch>,
        secrets: Arc<dyn SecretStore>,
        config_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            fetch,
            secrets,
            config_dir: config_dir.into(),
            cache_dir: None,
            client_ids: ClientIds::from_environment(),
            now: bitbraid_core::clock::unix_seconds,
        }
    }

    /// The real ones for a configuration directory, or those installed for
    /// it: the same value for the same directory for as long as the
    /// process lasts.
    pub fn for_config_dir(dir: &Path) -> Arc<Self> {
        let fresh = || {
            let mut services = Self::new(
                Arc::new(Client::new(&agent())),
                Arc::new(Keychain::new()),
                dir,
            );
            services.cache_dir = cache_dir();
            Arc::new(services)
        };
        match made().lock() {
            Ok(mut made) => made.entry(dir.to_path_buf()).or_insert_with(fresh).clone(),
            // Another thread failed while it held the list: make another.
            Err(_) => fresh(),
        }
    }

    /// Use `services` for every command whose configuration directory is
    /// `dir`, from now on in this process: for a test of the window, which
    /// cannot reach into the contexts its sessions make, or a run that
    /// must not reach the network or the keychain.
    pub fn install(dir: &Path, services: Arc<Self>) {
        if let Ok(mut made) = made().lock() {
            made.insert(dir.to_path_buf(), services);
        }
    }

    /// The ones a command uses: those in the context's slot, or the real
    /// ones of its configuration directory.
    pub fn of(ctx: &CommandContext) -> Result<Arc<Self>, ProviderError> {
        if let Some(services) = ctx.extensions.get::<Self>() {
            return Ok(services);
        }
        let dir = ctx
            .config_dir
            .as_deref()
            .ok_or_else(|| ProviderError::Refused(say!("services-no-config-dir").to_owned()))?;
        Ok(Self::for_config_dir(dir))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::MemoryStore;

    #[test]
    fn a_context_finds_its_services_in_its_slot_or_by_its_directory() {
        let dir = std::env::temp_dir().join(format!("bitbraid-services-{}", std::process::id()));
        let none = CommandContext::default();
        assert!(Services::of(&none).is_err());
        let by_dir = CommandContext {
            config_dir: Some(dir.clone()),
            ..Default::default()
        };
        let first = Services::of(&by_dir).unwrap();
        assert!(Arc::ptr_eq(&first, &Services::of(&by_dir).unwrap()));
        // Installed for the directory: from now on, those.
        let stand_in = Arc::new(Services::new(
            Arc::new(Client::new("Test/1")),
            Arc::new(MemoryStore::new()),
            &dir,
        ));
        Services::install(&dir, stand_in.clone());
        assert!(Arc::ptr_eq(&stand_in, &Services::of(&by_dir).unwrap()));
        // In the slot: those, whatever the directory.
        let mut slotted = CommandContext {
            config_dir: Some(dir),
            ..Default::default()
        };
        slotted.extensions.insert(first.clone());
        assert!(Arc::ptr_eq(&first, &Services::of(&slotted).unwrap()));
    }
}
