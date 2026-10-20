//! bitbraid-core: what every other crate stands on (layer L0). Paths as
//! the system and people write them ([`paths`]), the programs the
//! application starts and the login shell's environment they get
//! ([`process`]), repositories inside a WSL distribution ([`wsl`]), and
//! the clock that can be held still ([`clock`]).
//!
//! Sources: none (original design); each module lists its own.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod paths;
pub mod process;
pub mod wsl;

pub mod clock {
    //! The time of day, for what is written down with a time and shown as
    //! an age ("3 d ago"): the journal, the activity log, the interface.
    //!
    //! Dev switch: `BITBRAID_NOW=<seconds since the Unix epoch>` holds
    //! this clock still at that moment for the whole process. Pictures of
    //! the interface that are compared with stored ones are taken that
    //! way, so that an age reads the same next year. It is read once.

    use std::sync::OnceLock;

    /// The moment the clock is held at, when it is.
    pub fn held() -> Option<i64> {
        static HELD: OnceLock<Option<i64>> = OnceLock::new();
        *HELD.get_or_init(|| parse(std::env::var("BITBRAID_NOW").ok().as_deref()))
    }

    fn parse(set: Option<&str>) -> Option<i64> {
        set.and_then(|seconds| seconds.trim().parse().ok())
    }

    /// Seconds since the Unix epoch: now, or the moment the clock is held
    /// at.
    pub fn unix_seconds() -> i64 {
        held().unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
                .unwrap_or(0)
        })
    }

    #[cfg(test)]
    mod tests {
        use super::parse;

        #[test]
        fn the_switch_is_a_number_of_seconds_or_nothing() {
            assert_eq!(parse(None), None);
            assert_eq!(parse(Some("")), None);
            assert_eq!(parse(Some("soon")), None);
            assert_eq!(parse(Some("1577836800")), Some(1_577_836_800));
            assert_eq!(parse(Some(" 12 \n")), Some(12));
            assert_eq!(parse(Some("-5")), Some(-5));
        }
    }
}
