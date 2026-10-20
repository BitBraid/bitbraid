//! Signing in by a code typed at the provider's site (the OAuth device
//! flow): what the dialog shows at each step, and how long it waits before
//! it asks again.
//!
//! `account.sign_in_start` gives a code, the page to type it on, how long
//! the code lasts and how often the provider may be asked. The dialog
//! shows the code large, with a button that copies it and one that opens
//! the page in the system's browser, and asks with `account.sign_in_poll`
//! at the interval given until the user is done there, says no, or the
//! code runs out. Asked too often, the provider says to slow down, and
//! the wait grows by five seconds, or to what it says when that is more.
//! Closing the dialog stops the asking.
//!
//! A provider that offers no sign-in by code (no application is known for
//! the server) says so at the start; the dialog then offers a token.
//!
//! [`Flow`] is the state alone, without a window: the tests drive it with
//! answers made up for them.
//!
//! Sources: RFC 8628 (OAuth 2.0 Device Authorization Grant: `interval`,
//! `slow_down`, `expired_token`, `access_denied`); docs.github.com
//! "Authorizing OAuth apps" (device flow); docs.gitlab.com "OAuth 2.0
//! identity provider API" (device authorization grant).

use std::time::{Duration, Instant};

use bitbraid_integrations::model::{Account, DeviceCode, DevicePoll};

use crate::providers::Site;

/// The least wait between two asks, whatever the provider says.
const LEAST_WAIT: u64 = 1;
/// The longest wait between two asks.
const LONGEST_WAIT: u64 = 300;
/// The wait when the provider gives none: RFC 8628 says five seconds.
const USUAL_WAIT: u64 = 5;
/// How long a code is taken to last at most.
const LONGEST_LIFE: u64 = 3600;
/// The longest code shown: longer is no code a person types.
const LONGEST_CODE: usize = 64;

/// Where a sign-in by code stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    /// The provider is being asked for a code.
    Starting,
    /// The code is on show; the provider is asked whether the user is
    /// done with it.
    Waiting,
    /// Signed in.
    Done(Box<Account>),
    /// The user said no at the provider.
    Denied,
    /// The code ran out before the user was done.
    Expired,
    /// The provider offers no sign-in by code here: a token will do.
    Unavailable(String),
    /// Something else went wrong.
    Failed(String),
}

/// A sign-in by code, from the first ask to its end.
#[derive(Debug, Clone)]
pub struct Flow {
    /// The site it is for.
    pub site: Site,
    /// The site's address.
    pub web: String,
    /// Where it stands.
    pub stage: Stage,
    /// The code and where to type it, once the provider gave them.
    code: Option<DeviceCode>,
    /// Seconds between two asks.
    interval: u64,
    /// When the code runs out.
    until: Option<Instant>,
    /// How many times the provider was asked.
    pub asked: u32,
}

/// Whether an error says that no sign-in by code is offered there.
pub fn says_unavailable(error: &str) -> bool {
    let lower = error.to_lowercase();
    lower.contains("not available")
        || lower.contains("not offered")
        || lower.contains("no client id")
        || lower.contains("no application")
        || lower.contains("use a token")
}

impl Flow {
    /// A sign-in that is about to ask for a code.
    pub fn new(site: Site, web: impl Into<String>) -> Self {
        Self {
            site,
            web: web.into(),
            stage: Stage::Starting,
            code: None,
            interval: USUAL_WAIT,
            until: None,
            asked: 0,
        }
    }

    /// The code and where to type it, while there is one.
    pub fn code(&self) -> Option<&DeviceCode> {
        self.code.as_ref()
    }

    /// Seconds between two asks.
    pub fn interval(&self) -> u64 {
        self.interval
    }

    /// Whether it is over, one way or the other.
    pub fn is_over(&self) -> bool {
        !matches!(self.stage, Stage::Starting | Stage::Waiting)
    }

    /// Take what `account.sign_in_start` answered, at `now`.
    pub fn started(&mut self, answer: Result<DeviceCode, String>, now: Instant) {
        if self.stage != Stage::Starting {
            return;
        }
        match answer {
            Ok(mut code) => {
                let code_fits = !code.user_code.trim().is_empty()
                    && code.user_code.len() <= LONGEST_CODE
                    && !code.user_code.chars().any(char::is_control)
                    && !code.device_code.is_empty();
                if !code_fits {
                    self.stage = Stage::Failed(say!("integrations-no-code").to_owned());
                    return;
                }
                code.user_code = code.user_code.trim().to_owned();
                self.interval = code.interval.clamp(LEAST_WAIT, LONGEST_WAIT);
                if code.interval == 0 {
                    self.interval = USUAL_WAIT;
                }
                let life = code.expires_in.clamp(1, LONGEST_LIFE);
                self.until = now.checked_add(Duration::from_secs(life));
                self.code = Some(code);
                self.stage = Stage::Waiting;
            }
            Err(error) if says_unavailable(&error) => self.stage = Stage::Unavailable(error),
            Err(error) => self.stage = Stage::Failed(error),
        }
    }

    /// How long to wait before the provider is asked again, at `now`; none
    /// when it is not to be asked again. A code that ran out meanwhile
    /// ends the sign-in here, without asking.
    pub fn next_wait(&mut self, now: Instant) -> Option<Duration> {
        if self.stage != Stage::Waiting {
            return None;
        }
        if self.until.is_some_and(|until| now >= until) {
            self.stage = Stage::Expired;
            return None;
        }
        Some(Duration::from_secs(self.interval))
    }

    /// Take what `account.sign_in_poll` answered.
    pub fn polled(&mut self, answer: Result<DevicePoll, String>) {
        if self.stage != Stage::Waiting {
            return;
        }
        self.asked = self.asked.saturating_add(1);
        match answer {
            Ok(DevicePoll::Waiting) => {}
            Ok(DevicePoll::SlowDown { interval }) => {
                // At least five seconds more, as RFC 8628 has it, or what
                // the provider says when that is more.
                let more = self.interval.saturating_add(5);
                self.interval = interval.max(more).clamp(LEAST_WAIT, LONGEST_WAIT);
            }
            Ok(DevicePoll::Denied) => self.stage = Stage::Denied,
            Ok(DevicePoll::Expired) => self.stage = Stage::Expired,
            Ok(DevicePoll::Done { account }) => self.stage = Stage::Done(Box::new(account)),
            Err(error) => self.stage = Stage::Failed(error),
        }
    }

    /// What the dialog says under its title.
    pub fn words(&self) -> String {
        let site = self.site.name();
        match &self.stage {
            Stage::Starting => say!("integrations-asking-code", site),
            Stage::Waiting => say!("integrations-type-code", site),
            Stage::Done(account) => say!(
                "integrations-signed-in",
                site,
                login = account.login.as_str()
            ),
            Stage::Denied => say!("integrations-denied", site),
            Stage::Expired => say!("integrations-expired").to_owned(),
            Stage::Unavailable(_) => say!("integrations-unavailable", site),
            Stage::Failed(why) => why.clone(),
        }
    }

    /// Its stage as a word, for the tests.
    pub fn stage_word(&self) -> &'static str {
        match self.stage {
            Stage::Starting => "starting",
            Stage::Waiting => "waiting",
            Stage::Done(_) => "done",
            Stage::Denied => "denied",
            Stage::Expired => "expired",
            Stage::Unavailable(_) => "unavailable",
            Stage::Failed(_) => "failed",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bitbraid_integrations::model::{Account, DeviceCode, DevicePoll, ProviderKind, SignIn};

    use super::{Flow, Stage, says_unavailable};
    use crate::providers::Site;

    fn code(interval: u64, expires_in: u64) -> DeviceCode {
        DeviceCode {
            user_code: "WDJB-MJHT".to_owned(),
            verification_uri: "https://github.com/login/device".to_owned(),
            device_code: "3584d83530557fdd1f46af8289938c8ef79f9dc5".to_owned(),
            expires_in,
            interval,
        }
    }

    fn account() -> Account {
        Account {
            id: "github:github.com:octo".to_owned(),
            kind: ProviderKind::Github,
            web: "https://github.com".to_owned(),
            api: "https://api.github.com".to_owned(),
            login: "octo".to_owned(),
            name: None,
            avatar_url: None,
            sign_in: SignIn::Device,
            scopes: vec!["repo".to_owned()],
            default: true,
        }
    }

    #[test]
    fn a_code_is_asked_about_at_its_interval_until_it_is_used() {
        let now = Instant::now();
        let mut flow = Flow::new(Site::Github, "https://github.com");
        assert_eq!(flow.next_wait(now), None, "nothing to ask about yet");
        flow.started(Ok(code(5, 900)), now);
        assert_eq!(flow.stage, Stage::Waiting);
        assert_eq!(flow.code().unwrap().user_code, "WDJB-MJHT");
        assert_eq!(flow.next_wait(now), Some(Duration::from_secs(5)));
        flow.polled(Ok(DevicePoll::Waiting));
        assert_eq!(flow.next_wait(now), Some(Duration::from_secs(5)));
        // Asked too often: five seconds more, or what the provider says.
        flow.polled(Ok(DevicePoll::SlowDown { interval: 7 }));
        assert_eq!(flow.interval(), 10);
        flow.polled(Ok(DevicePoll::SlowDown { interval: 30 }));
        assert_eq!(flow.interval(), 30);
        flow.polled(Ok(DevicePoll::Done { account: account() }));
        assert_eq!(flow.stage_word(), "done");
        assert!(flow.is_over());
        assert_eq!(flow.next_wait(now), None);
        assert!(flow.words().contains("as octo"));
        assert_eq!(flow.asked, 4);
        // Nothing moves it once it is over.
        flow.polled(Ok(DevicePoll::Denied));
        assert_eq!(flow.stage_word(), "done");
    }

    #[test]
    fn a_code_that_runs_out_ends_the_asking() {
        let now = Instant::now();
        let mut flow = Flow::new(Site::Gitlab, "https://gitlab.com");
        flow.started(Ok(code(5, 60)), now);
        let later = now + Duration::from_secs(61);
        assert_eq!(flow.next_wait(later), None);
        assert_eq!(flow.stage, Stage::Expired);

        let mut flow = Flow::new(Site::Gitlab, "https://gitlab.com");
        flow.started(Ok(code(5, 60)), now);
        flow.polled(Ok(DevicePoll::Expired));
        assert_eq!(flow.stage, Stage::Expired);

        let mut flow = Flow::new(Site::Gitlab, "https://gitlab.com");
        flow.started(Ok(code(5, 60)), now);
        flow.polled(Ok(DevicePoll::Denied));
        assert_eq!(flow.stage, Stage::Denied);
        assert!(flow.words().contains("turned down"));
    }

    #[test]
    fn hostile_answers_are_held_in_bounds() {
        let now = Instant::now();
        let mut flow = Flow::new(Site::Github, "https://github.com");
        flow.started(Ok(code(u64::MAX, u64::MAX)), now);
        assert_eq!(flow.interval(), 300);
        flow.polled(Ok(DevicePoll::SlowDown { interval: u64::MAX }));
        assert_eq!(flow.interval(), 300);

        let mut flow = Flow::new(Site::Github, "https://github.com");
        flow.started(Ok(code(0, 0)), now);
        assert_eq!(flow.interval(), 5);
        assert_eq!(flow.next_wait(now + Duration::from_secs(2)), None);
        assert_eq!(flow.stage, Stage::Expired);

        for user_code in ["", "  ", "a\u{7}b", &"X".repeat(65)] {
            let mut flow = Flow::new(Site::Github, "https://github.com");
            let mut bad = code(5, 900);
            bad.user_code = user_code.to_owned();
            flow.started(Ok(bad), now);
            assert_eq!(flow.stage_word(), "failed", "{user_code:?}");
        }
    }

    #[test]
    fn no_sign_in_by_code_offers_a_token() {
        let now = Instant::now();
        let mut flow = Flow::new(Site::GitlabServer, "https://gitlab.example.org");
        flow.started(
            Err(
                "Signing in with a code is not available for gitlab.example.org: use a token."
                    .to_owned(),
            ),
            now,
        );
        assert_eq!(flow.stage_word(), "unavailable");
        assert!(flow.words().contains("Use a token"));
        let mut flow = Flow::new(Site::Github, "https://github.com");
        flow.started(Err("The network is down.".to_owned()), now);
        assert_eq!(flow.stage, Stage::Failed("The network is down.".to_owned()));
        assert!(says_unavailable("no client id is known for this host"));
        assert!(!says_unavailable("connection refused"));
    }
}
