//! Search: the query grammar, the commits a query holds for, content
//! search, and the commit to jump to.
//!
//! A query is words separated by white space. A word is free text or
//! `key:value`; a leading `-` turns it into "not"; double quotes keep
//! spaces together (`author:"Ada L"`, `"fix: typo"`), and inside them `\"`
//! is a quote and `\\` a backslash. Every word has to hold for a commit.
//!
//! What a commit says about itself (message, people, id, date, parents) is
//! read through gix, and only as much of it as the query asks about. Where
//! a commit sits and what it changes (`ref:`, `path:`, `change:`, `regex:`)
//! is asked of git, once per word, as a set of ids.
//!
//! `issue:` finds the commits whose message names an issue: `issue:123`
//! or `issue:#123` every key with that number (`#123`, and
//! `owner/name#123` of any repository), `issue:owner/name#123` only keys
//! of that repository, its letters in any case. A key is read from a
//! message as [`crate::issue_keys`] says: never inside a word, an address
//! or code.
//!
//! Two things are as `git log` has them. People are the ones the mailmap
//! makes of them (`author:` finds a person under the name and address
//! the project gives them, whatever a commit records), unless
//! `log.mailmap` is off. And a date is read in the user's time zone
//! ([`Zone`]): `before:2026-10-01` ends where that day begins here.
//!
//! Sources: git-log(1) (`--grep`, `--author`, `--committer`, `-i`,
//! `--fixed-strings`, `--before`, `--after`, `--full-history` and "History
//! Simplification", `-S`, `-G`, `--use-mailmap`), git-rev-list(1),
//! git-rev-parse(1), gitrevisions(7), gitglossary(7) (pathspec),
//! git-config(1) (`log.follow`, `log.showSignature`, `log.mailmap`,
//! `mailmap.file`, `mailmap.blob`), gitmailmap(5), git(1) (`TZ`); gix and
//! jiff documentation (docs.rs/gix, docs.rs/jiff: time zones, and how a
//! time of day that a zone has twice or not at all is read); the day
//! count of the Gregorian calendar after "chrono-Compatible Low-Level Date
//! Algorithms" (public domain).

use std::{
    collections::HashSet,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use gix::bstr::Finder;
use serde::{Deserialize, Serialize};

use crate::{
    Error, Oid, Repository, Result, exe,
    log::{GraphCommit, LogOptions, Tips, walk},
    oid::hex_digits,
    remote::{Progress, Reporter},
};

/// The longest query that is read, in bytes.
pub const MAX_QUERY_BYTES: usize = 4096;
/// The most words a query may have. Each of `ref:`, `path:`, `change:` and
/// `regex:` runs git once, so this bounds that too.
pub const MAX_WORDS: usize = 32;
/// How many commits a search answers with when no limit is given.
pub const DEFAULT_LIMIT: usize = 10_000;
/// The longest revision that is handed to git, in bytes.
pub const MAX_REV_BYTES: usize = 1024;
/// The most paths a content search may be limited to.
pub const MAX_PATHS: usize = 64;
/// The most starting points that are named to git on its command line,
/// where the length of everything together is limited.
pub const MAX_TIPS_FOR_GIT: usize = 512;

/// How often the scan looks at the cancel flag, in commits.
const LOOK_EVERY: usize = 1024;

/// What a word is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Key {
    /// Free text: part of the message or of the author, or the beginning
    /// of the id. Letters are compared without regard to case, here and in
    /// the three that follow.
    Text,
    /// Part of the message.
    Message,
    /// Part of the author, read as `Name <address>`.
    Author,
    /// Part of the committer, read the same way.
    Committer,
    /// The beginning of the id.
    Sha,
    /// The commit date is before an instant.
    Before,
    /// The commit date is at or after an instant.
    After,
    /// A kind of commit; `merge` is the only one.
    Is,
    /// Reachable from a branch, tag or commit.
    Ref,
    /// Changes a path: differs there from its parent, or from one of
    /// them when it is a merge.
    Path,
    /// Adds or removes a text.
    Change,
    /// Has a changed line matching a regular expression.
    Regex,
    /// Names an issue in its message.
    Issue,
}

/// The keys there are, as they are written before the colon.
pub const KEYS: [(&str, Key); 12] = [
    ("message", Key::Message),
    ("author", Key::Author),
    ("committer", Key::Committer),
    ("sha", Key::Sha),
    ("before", Key::Before),
    ("after", Key::After),
    ("is", Key::Is),
    ("ref", Key::Ref),
    ("path", Key::Path),
    ("change", Key::Change),
    ("regex", Key::Regex),
    ("issue", Key::Issue),
];

impl Key {
    fn named(name: &str) -> Option<Self> {
        KEYS.iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, key)| *key)
    }

    /// As it is written before the colon; free text has no key.
    pub fn name(self) -> &'static str {
        KEYS.iter()
            .find(|(_, key)| *key == self)
            .map(|(name, _)| *name)
            .unwrap_or("text")
    }

    /// Whether git is asked for the commits the word holds for.
    fn asks_git(self) -> bool {
        matches!(self, Key::Ref | Key::Path | Key::Change | Key::Regex)
    }
}

/// One word of a query, as understood.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Word {
    /// What the word is about; `text` for free text.
    pub key: Key,
    /// What it asks for, without quotes: lowercase for `sha:` and `is:`,
    /// the key as `#123` or `owner/name#123` for `issue:`, as written
    /// otherwise.
    pub value: String,
    /// Whether the word has to fail rather than hold.
    pub negated: bool,
    /// Where the word begins in the query, in bytes, its `-` included.
    pub start: usize,
    /// Where it ends, exclusive.
    pub end: usize,
    /// For `before:` and `after:`, the instant the commit date is compared
    /// with, in seconds since the Unix epoch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seconds: Option<i64>,
}

/// A parsed query: all its words have to hold.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Query {
    /// The words in the order they were written.
    pub words: Vec<Word>,
}

/// What is wrong with a query and where.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryError {
    /// Where it goes wrong, as a byte offset into the query.
    pub position: usize,
    /// Where the part that is wrong ends, exclusive; the same as
    /// `position` when something is missing there.
    pub end: usize,
    /// What was expected there.
    pub expected: String,
}

impl QueryError {
    fn new(position: usize, end: usize, expected: impl Into<String>) -> Self {
        Self {
            position,
            end: end.max(position),
            expected: expected.into(),
        }
    }
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&say!(
            "search-error-at",
            position = self.position,
            expected = self.expected.as_str()
        ))
    }
}

impl std::error::Error for QueryError {}

impl From<QueryError> for Error {
    fn from(err: QueryError) -> Self {
        Error::Invalid {
            what: say!("search-what-query"),
            detail: err.to_string(),
        }
    }
}

type Parsed<T> = std::result::Result<T, QueryError>;

/// The time zone the dates of a query are read in: `before:2026-10-01`
/// is the beginning of that day there. `git log --before` reads a date in
/// the user's time zone, and so does a query unless it is told another.
#[derive(Debug, Clone)]
pub struct Zone(jiff::tz::TimeZone);

/// A day of the calendar: year, month, day of the month.
type Day = (i64, i64, i64);
/// A time of day: hour, minute, second.
type Clock = (i64, i64, i64);

impl Zone {
    /// The time zone of the system: what `TZ` says, or what the system
    /// is set to. UTC where neither can be found out.
    pub fn system() -> Self {
        Self(jiff::tz::TimeZone::system())
    }

    /// How far east of UTC this zone's clocks are at an instant, in
    /// seconds; nought for an instant outside what the calendar reaches.
    pub fn offset_at(&self, seconds: i64) -> i32 {
        jiff::Timestamp::from_second(seconds)
            .map(|at| self.0.to_offset(at).seconds())
            .unwrap_or(0)
    }

    /// Coordinated universal time.
    #[cfg(test)]
    pub fn utc() -> Self {
        Self(jiff::tz::TimeZone::UTC)
    }

    /// A time zone by its name in the time zone database, such as
    /// `Europe/Vienna`, or `UTC`. None for a name the database of this
    /// system does not have.
    pub fn named(name: &str) -> Option<Self> {
        let plain = !name.is_empty()
            && name.len() <= 64
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'));
        if !plain {
            return None;
        }
        jiff::tz::TimeZone::get(name).ok().map(Self)
    }

    /// The instant a date and a time of day are in this zone. A time the
    /// zone has twice (the clocks were set back) is the first of the
    /// two; one it does not have (they were set forward) counts from
    /// before the jump, which puts it after.
    fn instant(&self, date: Day, time: Clock) -> Option<i64> {
        let small = |value: i64| i8::try_from(value).ok();
        let civil = jiff::civil::DateTime::new(
            i16::try_from(date.0).ok()?,
            small(date.1)?,
            small(date.2)?,
            small(time.0)?,
            small(time.1)?,
            small(time.2)?,
            0,
        )
        .ok()?;
        Some(self.0.to_zoned(civil).ok()?.timestamp().as_second())
    }

    /// The date and the time of day an instant is in this zone.
    fn civil(&self, seconds: i64) -> Option<(Day, Clock)> {
        let zoned = jiff::Timestamp::from_second(seconds)
            .ok()?
            .to_zoned(self.0.clone());
        Some((
            (
                i64::from(zoned.year()),
                i64::from(zoned.month()),
                i64::from(zoned.day()),
            ),
            (
                i64::from(zoned.hour()),
                i64::from(zoned.minute()),
                i64::from(zoned.second()),
            ),
        ))
    }
}

/// Read a query, with its dates in the system's time zone. `now` is what
/// `3d` and the other ages count back from, in seconds since the Unix
/// epoch.
///
/// A query longer than [`MAX_QUERY_BYTES`] or with more words than
/// [`MAX_WORDS`] is refused without being read to its end.
pub fn parse(text: &str, now: i64) -> Parsed<Query> {
    parse_in(text, now, &Zone::system())
}

/// [`parse`] with the dates read in `zone`.
pub fn parse_in(text: &str, now: i64, zone: &Zone) -> Parsed<Query> {
    if text.len() > MAX_QUERY_BYTES {
        let mut at = MAX_QUERY_BYTES;
        while !text.is_char_boundary(at) {
            at = at.saturating_sub(1);
        }
        return Err(QueryError::new(
            at,
            text.len(),
            say!("search-expected-end-bytes", most = MAX_QUERY_BYTES),
        ));
    }
    let mut cursor = Cursor { text, at: 0 };
    let mut words = Vec::new();
    loop {
        cursor.skip_space();
        if cursor.peek().is_none() {
            break;
        }
        if words.len() >= MAX_WORDS {
            return Err(QueryError::new(
                cursor.at,
                text.len(),
                say!("search-expected-end-words", most = MAX_WORDS),
            ));
        }
        words.push(word(&mut cursor, now, zone)?);
    }
    Ok(Query { words })
}

/// A position in the query. Only ever moved by whole characters, so it
/// stays on a character boundary.
struct Cursor<'a> {
    text: &'a str,
    at: usize,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<char> {
        self.text
            .get(self.at..)
            .and_then(|rest| rest.chars().next())
    }

    fn bump(&mut self) -> Option<char> {
        let next = self.peek()?;
        self.at = self.at.saturating_add(next.len_utf8());
        Some(next)
    }

    fn skip_space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    /// The text from `from` up to here.
    fn since(&self, from: usize) -> &str {
        self.text.get(from..self.at).unwrap_or_default()
    }

    /// Read an unquoted run up to white space, the end, or a quote that
    /// opens a value. Returns where its first colon is.
    fn bare(&mut self) -> Parsed<Option<usize>> {
        let mut colon = None;
        while let Some(next) = self.peek() {
            if next.is_whitespace() {
                break;
            }
            if next == '"' {
                // Directly after the key's colon it opens the value.
                if colon.is_some_and(|colon: usize| colon.saturating_add(1) == self.at) {
                    break;
                }
                return Err(QueryError::new(
                    self.at,
                    self.at.saturating_add(1),
                    say!("search-expected-quote-placement"),
                ));
            }
            if next.is_control() {
                return Err(self.control(next));
            }
            if next == ':' && colon.is_none() {
                colon = Some(self.at);
            }
            self.bump();
        }
        Ok(colon)
    }

    /// Read a quoted value, the cursor being on its opening quote. White
    /// space or the end has to follow the closing one.
    fn quoted(&mut self) -> Parsed<String> {
        let open = self.at;
        self.bump();
        let mut value = String::new();
        loop {
            let at = self.at;
            match self.bump() {
                None => {
                    return Err(QueryError::new(
                        open,
                        self.text.len(),
                        say!("search-expected-closing-quote"),
                    ));
                }
                Some('"') => break,
                Some('\\') => match self.peek() {
                    Some(escaped @ ('"' | '\\')) => {
                        value.push(escaped);
                        self.bump();
                    }
                    _ => value.push('\\'),
                },
                Some(next) if next.is_control() && next != '\t' => {
                    self.at = at;
                    return Err(self.control(next));
                }
                Some(next) => value.push(next),
            }
        }
        if self.peek().is_some_and(|next| !next.is_whitespace()) {
            return Err(QueryError::new(
                self.at,
                self.at,
                say!("search-expected-space-after-quote"),
            ));
        }
        Ok(value)
    }

    fn control(&self, found: char) -> QueryError {
        QueryError::new(
            self.at,
            self.at.saturating_add(found.len_utf8()),
            say!("search-expected-no-control"),
        )
    }
}

fn word(cursor: &mut Cursor<'_>, now: i64, zone: &Zone) -> Parsed<Word> {
    let start = cursor.at;
    let negated = cursor.peek() == Some('-');
    if negated {
        cursor.bump();
        match cursor.peek() {
            // One `-` is "not"; a second one is refused rather than read
            // as "not not", so a run of them costs nothing.
            Some('-') => {
                return Err(QueryError::new(
                    cursor.at,
                    cursor.at.saturating_add(1),
                    say!("search-expected-word-after-second-dash"),
                ));
            }
            Some(next) if !next.is_whitespace() => {}
            _ => {
                return Err(QueryError::new(
                    start,
                    cursor.at,
                    say!("search-expected-word-after-dash"),
                ));
            }
        }
    }
    let body = cursor.at;
    let (key, value, value_at) = if cursor.peek() == Some('"') {
        // Quoted as a whole: text, whatever colons it has.
        (Key::Text, cursor.quoted()?, body)
    } else {
        match cursor.bare()? {
            None => (Key::Text, cursor.since(body).to_owned(), body),
            Some(colon) => {
                let name = cursor.text.get(body..colon).unwrap_or_default();
                let key = Key::named(name).ok_or_else(|| {
                    QueryError::new(body, colon, say!("search-expected-key", keys = key_list()))
                })?;
                let value_at = colon.saturating_add(1);
                let value = if cursor.peek() == Some('"') {
                    cursor.quoted()?
                } else {
                    cursor.since(value_at).to_owned()
                };
                (key, value, value_at)
            }
        }
    };
    let end = cursor.at;
    let (value, seconds) = understood(key, value, now, zone)
        .map_err(|expected| QueryError::new(value_at, end, expected))?;
    Ok(Word {
        key,
        value,
        negated,
        start,
        end,
        seconds,
    })
}

/// The keys as an unknown one is answered with.
fn key_list() -> String {
    let mut list = String::new();
    for (name, _) in KEYS {
        list.push_str(name);
        list.push_str(": ");
    }
    list.push_str(say!("search-quoted-word-is-text"));
    list
}

/// Check a value against its key. Answers the value as it is used and, for
/// a date, its instant; or what was expected instead.
fn understood(
    key: Key,
    value: String,
    now: i64,
    zone: &Zone,
) -> std::result::Result<(String, Option<i64>), String> {
    if value.is_empty() {
        return Err(match key {
            Key::Text => say!("search-expected-text-in-quotes").to_owned(),
            _ => say!("search-expected-value-after-key", key = key.name()),
        });
    }
    match key {
        Key::Sha => {
            if value.len() > 64 || !value.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(say!("search-expected-hex").to_owned());
            }
            Ok((value.to_ascii_lowercase(), None))
        }
        Key::Is => {
            if !value.eq_ignore_ascii_case("merge") {
                return Err(say!("search-expected-merge").to_owned());
            }
            Ok(("merge".to_owned(), None))
        }
        Key::Before | Key::After => match instant_in(&value, now, zone) {
            Some(seconds) => Ok((value, Some(seconds))),
            None => Err(DATE_FORMS.text().to_owned()),
        },
        Key::Issue => match crate::issue_keys::IssueKey::parse(&value) {
            Some(key) => Ok((key.to_string(), None)),
            None => Err(ISSUE_FORMS.text().to_owned()),
        },
        Key::Ref => check_rev(&value).map(|()| (value, None)),
        Key::Path => check_path(&value).map(|()| (value, None)),
        Key::Text | Key::Message | Key::Author | Key::Committer | Key::Change | Key::Regex => {
            Ok((value, None))
        }
    }
}

/// The forms an issue can be written in, for an error.
pub const ISSUE_FORMS: bitbraid_strings::Message = message!("search-issue-forms");

/// The forms a date can be written in, for an error.
pub const DATE_FORMS: bitbraid_strings::Message = message!("search-date-forms");

const DAY: i64 = 86_400;

/// The instant a date names, in seconds since the Unix epoch: `2026-10-01`
/// (the beginning of that day), `2026-10-01T12:00`, both in the system's
/// time zone, or an age counted back from `now`: `3d`, `2w`, `6m`, `1y`.
/// Months and years are those of the calendar, so `1m` before the 31st of
/// March is the last day of February.
///
/// `git log --before=2026-10-01` reads the date in the user's time zone
/// too. It takes the time of day from the moment it is run, though, where
/// a date alone is the beginning of the day here: a query means the same
/// in the morning and in the evening.
pub fn instant(text: &str, now: i64) -> Option<i64> {
    instant_in(text, now, &Zone::system())
}

/// [`instant`] with the date read in `zone`.
pub fn instant_in(text: &str, now: i64, zone: &Zone) -> Option<i64> {
    absolute(text, zone).or_else(|| ago(text, now, zone))
}

/// A run of ASCII digits as a number.
fn digits(text: &str, from: usize, to: usize) -> Option<i64> {
    let part = text.get(from..to)?;
    if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    part.parse().ok()
}

fn absolute(text: &str, zone: &Zone) -> Option<i64> {
    let bytes = text.as_bytes();
    if !matches!(bytes.len(), 10 | 16) || bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-')
    {
        return None;
    }
    let (year, month, day) = (
        digits(text, 0, 4)?,
        digits(text, 5, 7)?,
        digits(text, 8, 10)?,
    );
    if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    let (mut hour, mut minute) = (0, 0);
    if bytes.len() == 16 {
        if bytes.get(10) != Some(&b'T') || bytes.get(13) != Some(&b':') {
            return None;
        }
        (hour, minute) = (digits(text, 11, 13)?, digits(text, 14, 16)?);
        if hour > 23 || minute > 59 {
            return None;
        }
    }
    // In the zone. A date its calendar does not reach (the year 0) is
    // read in UTC: far from any commit either way.
    zone.instant((year, month, day), (hour, minute, 0))
        .or_else(|| {
            Some(
                days_from_civil(year, month, day)
                    .saturating_mul(DAY)
                    .saturating_add(hour.saturating_mul(3600))
                    .saturating_add(minute.saturating_mul(60)),
            )
        })
}

fn ago(text: &str, now: i64, zone: &Zone) -> Option<i64> {
    let unit = text.chars().next_back()?;
    let count = i64::from(
        digits(text, 0, text.len().checked_sub(unit.len_utf8())?)
            .and_then(|count| u32::try_from(count).ok())?,
    );
    match unit {
        'd' => Some(now.saturating_sub(count.saturating_mul(DAY))),
        'w' => Some(now.saturating_sub(count.saturating_mul(7 * DAY))),
        'm' => Some(months_before(now, count, zone)),
        'y' => Some(months_before(now, count.saturating_mul(12), zone)),
        _ => None,
    }
}

/// The year, month and day `months` calendar months before a day; the
/// last day of that month when it has no such day.
fn month_earlier(date: Day, months: i64) -> Day {
    let (year, month, day) = date;
    let count = year
        .saturating_mul(12)
        .saturating_add(month.saturating_sub(1))
        .saturating_sub(months);
    let (year, month) = (count.div_euclid(12), count.rem_euclid(12).saturating_add(1));
    (year, month, day.min(days_in_month(year, month)))
}

/// The same day and time of day `months` calendar months earlier, as the
/// zone's calendar and clock have them. Where the zone cannot say (a
/// moment or a year outside what it knows), as UTC has them.
fn months_before(now: i64, months: i64, zone: &Zone) -> i64 {
    let in_zone = zone
        .civil(now)
        .and_then(|(date, time)| zone.instant(month_earlier(date, months), time));
    in_zone.unwrap_or_else(|| {
        let (days, time_of_day) = (now.div_euclid(DAY), now.rem_euclid(DAY));
        let (year, month, day) = month_earlier(civil_from_days(days), months);
        days_from_civil(year, month, day)
            .saturating_mul(DAY)
            .saturating_add(time_of_day)
    })
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year.rem_euclid(4) == 0
            && (year.rem_euclid(100) != 0 || year.rem_euclid(400) == 0) =>
        {
            29
        }
        2 => 28,
        _ => 31,
    }
}

/// Days from 1970-01-01 to a day of the proleptic Gregorian calendar. The
/// year is counted from March so that the leap day comes last, and in
/// spans of 400 years, which all have the same number of days.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 {
        year.saturating_sub(1)
    } else {
        year
    };
    let (era, year_of_era) = (year.div_euclid(400), year.rem_euclid(400));
    let month_from_march = month.saturating_add(9).rem_euclid(12);
    let day_of_year = month_from_march
        .saturating_mul(153)
        .saturating_add(2)
        .div_euclid(5)
        .saturating_add(day)
        .saturating_sub(1);
    let day_of_era = year_of_era
        .saturating_mul(365)
        .saturating_add(year_of_era.div_euclid(4))
        .saturating_sub(year_of_era.div_euclid(100))
        .saturating_add(day_of_year);
    era.saturating_mul(146_097)
        .saturating_add(day_of_era)
        .saturating_sub(719_468)
}

/// The inverse of [`days_from_civil`]: year, month and day of a day count.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days.saturating_add(719_468);
    let (era, day_of_era) = (shifted.div_euclid(146_097), shifted.rem_euclid(146_097));
    let year_of_era = day_of_era
        .saturating_sub(day_of_era.div_euclid(1460))
        .saturating_add(day_of_era.div_euclid(36_524))
        .saturating_sub(day_of_era.div_euclid(146_096))
        .div_euclid(365);
    let day_of_year = day_of_era.saturating_sub(
        year_of_era
            .saturating_mul(365)
            .saturating_add(year_of_era.div_euclid(4))
            .saturating_sub(year_of_era.div_euclid(100)),
    );
    let month_from_march = day_of_year
        .saturating_mul(5)
        .saturating_add(2)
        .div_euclid(153);
    let day = day_of_year
        .saturating_sub(
            month_from_march
                .saturating_mul(153)
                .saturating_add(2)
                .div_euclid(5),
        )
        .saturating_add(1);
    let month = if month_from_march < 10 {
        month_from_march.saturating_add(3)
    } else {
        month_from_march.saturating_sub(9)
    };
    let year = year_of_era.saturating_add(era.saturating_mul(400));
    (
        if month <= 2 {
            year.saturating_add(1)
        } else {
            year
        },
        month,
        day,
    )
}

/// Refuse a revision git could read as an option, or that is no name at
/// all. Answers what was expected instead.
fn check_rev(rev: &str) -> std::result::Result<(), String> {
    if rev.is_empty()
        || rev.starts_with('-')
        || rev.len() > MAX_REV_BYTES
        || rev.chars().any(char::is_control)
    {
        return Err(say!("search-expected-rev", most = MAX_REV_BYTES));
    }
    Ok(())
}

/// Refuse a path git could read as an option or as pathspec magic.
fn check_path(path: &str) -> std::result::Result<(), String> {
    if path.is_empty()
        || path.starts_with('-')
        || path.starts_with(':')
        || path.len() > MAX_QUERY_BYTES
        || path.chars().any(char::is_control)
    {
        return Err(say!("search-expected-path", most = MAX_QUERY_BYTES));
    }
    Ok(())
}

/// A piece of input for an error message, cut to a length that can be read.
fn shown(text: &str) -> String {
    const LONGEST: usize = 80;
    if text.len() <= LONGEST {
        return text.to_owned();
    }
    let mut cut = LONGEST;
    while !text.is_char_boundary(cut) {
        cut = cut.saturating_sub(1);
    }
    format!("{}…", text.get(..cut).unwrap_or_default())
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("search-what-search"),
        detail: detail.into(),
    }
}

/// The error of a search that was stopped on request, as [`exe`] reports
/// a git process that was.
fn cancelled() -> Error {
    invalid(say!("search-what-operation"), say!("search-cancelled"))
}

/// What a search starts from and how much it may answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchOptions {
    /// The commits the graph is walked from.
    pub tips: Tips,
    /// The most commits to answer with.
    pub limit: usize,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            tips: Tips::All,
            limit: DEFAULT_LIMIT,
        }
    }
}

/// The commits a search found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Found {
    /// The matching commits in the order the graph has them.
    pub ids: Vec<Oid>,
    /// Whether more matched than the limit let through.
    pub truncated: bool,
}

/// Text to look for without regard to case.
struct Needle(Finder<'static>);

impl Needle {
    fn new(text: &str) -> Self {
        Self(Finder::new(text.to_lowercase().as_bytes()).into_owned())
    }

    /// Whether it is part of text that [`fold`] has prepared.
    fn is_in(&self, folded: &[u8]) -> bool {
        self.0.find(folded).is_some()
    }
}

/// Lowercase `parts`, one after the other, into `out`. Text that is not
/// UTF-8 is read as far as it can be.
fn fold(parts: &[&[u8]], out: &mut Vec<u8>) {
    out.clear();
    if parts.iter().all(|part| part.is_ascii()) {
        for part in parts {
            out.extend(part.iter().map(u8::to_ascii_lowercase));
        }
        return;
    }
    let whole = parts.concat();
    out.extend_from_slice(String::from_utf8_lossy(&whole).to_lowercase().as_bytes());
}

/// Whether the id, written in hexadecimal, begins with `digits`.
fn id_begins(id: &Oid, digits: &[u8]) -> bool {
    id.begins_with(digits)
}

/// One word made ready to be decided for commit after commit.
enum Test {
    Merge,
    Id(Vec<u8>),
    Before(i64),
    After(i64),
    Among(HashSet<Oid>),
    Author(Needle),
    Committer(Needle),
    Message(Needle),
    /// An issue's key, and `#` with its number, which a message naming it
    /// has to hold before it is read for keys.
    Issue(crate::issue_keys::IssueKey, Needle),
    Text {
        needle: Needle,
        id: Option<Vec<u8>>,
    },
}

impl Test {
    /// What deciding it costs; the cheap ones are asked first.
    fn cost(&self) -> u8 {
        match self {
            Test::Merge | Test::Id(_) | Test::Before(_) | Test::After(_) | Test::Among(_) => 0,
            Test::Author(_) | Test::Committer(_) => 1,
            Test::Message(_) | Test::Issue(..) => 2,
            Test::Text { .. } => 3,
        }
    }
}

struct Check {
    negated: bool,
    test: Test,
}

/// The commit a scan is at, read as far as the words have asked: the
/// object is loaded at the first question that needs it, and each of
/// message, author and committer is prepared when first asked for.
struct Reader<'r> {
    repo: &'r gix::Repository,
    commit: Option<gix::Commit<'r>>,
    loaded: bool,
    message: Vec<u8>,
    author: Vec<u8>,
    committer: Vec<u8>,
    has_message: bool,
    has_author: bool,
    has_committer: bool,
    /// The mailmap, read when a person is first asked for: none when
    /// there is none, or when `log.mailmap` says not to use it.
    mailmap: Option<Option<gix::mailmap::Snapshot>>,
}

/// The mailmap people are looked up in, as `git log` uses it: the
/// `.mailmap` of the worktree (of HEAD in a bare repository), and what
/// `mailmap.file` and `mailmap.blob` name. None when there is nothing in
/// any of them, or when `log.mailmap` is switched off.
///
/// The settings are read anew for it: the handle a session works with
/// has the configuration of the moment it was opened, and `log.mailmap`
/// or `mailmap.file` may have been set since, as git would see at once.
/// That is one more opening of the repository for a search that asks
/// about people, and none for one that does not.
fn mailmap_of(repo: &gix::Repository) -> Option<gix::mailmap::Snapshot> {
    let fresh = gix::open_opts(
        repo.workdir().unwrap_or_else(|| repo.git_dir()),
        crate::system_config::open_options(),
    )
    .ok();
    let repo = fresh.as_ref().unwrap_or(repo);
    if repo.config_snapshot().boolean("log.mailmap") == Some(false) {
        return None;
    }
    let snapshot = repo.open_mailmap();
    let empty = snapshot.iter().next().is_none();
    (!empty).then_some(snapshot)
}

impl<'r> Reader<'r> {
    fn new(repo: &'r gix::Repository) -> Self {
        Self {
            repo,
            commit: None,
            loaded: false,
            message: Vec::new(),
            author: Vec::new(),
            committer: Vec::new(),
            has_message: false,
            has_author: false,
            has_committer: false,
            mailmap: None,
        }
    }

    /// `Name <address>` of a person as the mailmap has them, folded for
    /// comparing, into `into`. `git log --author` and `--committer` match
    /// against the mapped person (git-log(1), `log.mailmap`), so that one
    /// who committed under two addresses is found under the one the
    /// project calls theirs.
    fn person(
        repo: &gix::Repository,
        mailmap: &mut Option<Option<gix::mailmap::Snapshot>>,
        person: gix::actor::SignatureRef<'_>,
        into: &mut Vec<u8>,
    ) {
        let mapped = mailmap
            .get_or_insert_with(|| mailmap_of(repo))
            .as_ref()
            .and_then(|mailmap| mailmap.try_resolve(person));
        let (name, email): (&[u8], &[u8]) = match &mapped {
            Some(mapped) => (mapped.name.as_ref(), mapped.email.as_ref()),
            None => (person.name, person.email),
        };
        fold(&[name, b" <", email, b">"], into);
    }

    /// Forget the commit that was read; the buffers are kept for the next.
    fn turn(&mut self) {
        self.commit = None;
        self.loaded = false;
        self.has_message = false;
        self.has_author = false;
        self.has_committer = false;
    }

    fn load(&mut self, id: &Oid) {
        if !self.loaded {
            self.loaded = true;
            self.commit = self.repo.find_commit(*id.as_gix()).ok();
        }
    }

    /// The commit date, for a commit the walk had none for.
    fn seconds(&mut self, id: &Oid) -> Option<i64> {
        self.load(id);
        self.commit
            .as_ref()
            .and_then(|commit| commit.time().ok())
            .map(|time| time.seconds)
    }

    fn message(&mut self, id: &Oid) -> Option<&[u8]> {
        if !self.has_message {
            self.load(id);
            let message: &[u8] = self.commit.as_ref()?.message_raw_sloppy();
            fold(&[message], &mut self.message);
            self.has_message = true;
        }
        Some(&self.message)
    }

    /// `Name <address>`, the way git matches `--author` against it.
    fn author(&mut self, id: &Oid) -> Option<&[u8]> {
        if !self.has_author {
            self.load(id);
            let person = self.commit.as_ref()?.author().ok()?;
            Self::person(self.repo, &mut self.mailmap, person, &mut self.author);
            self.has_author = true;
        }
        Some(&self.author)
    }

    fn committer(&mut self, id: &Oid) -> Option<&[u8]> {
        if !self.has_committer {
            self.load(id);
            let person = self.commit.as_ref()?.committer().ok()?;
            Self::person(self.repo, &mut self.mailmap, person, &mut self.committer);
            self.has_committer = true;
        }
        Some(&self.committer)
    }
}

/// Whether every check holds for the commit. A commit that cannot be read
/// holds for nothing that needs it read, negated or not.
fn holds(checks: &[Check], commit: &GraphCommit, reader: &mut Reader<'_>) -> bool {
    reader.turn();
    let id = &commit.id;
    for check in checks {
        let held = match &check.test {
            Test::Merge => commit.parents.len() > 1,
            Test::Id(digits) => id_begins(id, digits),
            Test::Among(set) => set.contains(id),
            Test::Before(instant) | Test::After(instant) => {
                let Some(seconds) = commit.seconds.or_else(|| reader.seconds(id)) else {
                    return false;
                };
                // Before is strict and after is not, so that exactly one
                // of the two holds for any commit.
                (seconds < *instant) == matches!(check.test, Test::Before(_))
            }
            Test::Author(needle) => match reader.author(id) {
                Some(author) => needle.is_in(author),
                None => return false,
            },
            Test::Committer(needle) => match reader.committer(id) {
                Some(committer) => needle.is_in(committer),
                None => return false,
            },
            Test::Message(needle) => match reader.message(id) {
                Some(message) => needle.is_in(message),
                None => return false,
            },
            Test::Issue(key, needle) => match reader.message(id) {
                Some(message) => needle.is_in(message) && names_issue(message, key),
                None => return false,
            },
            Test::Text { needle, id: digits } => {
                if digits.as_ref().is_some_and(|digits| id_begins(id, digits)) {
                    true
                } else {
                    let Some(in_author) = reader.author(id).map(|author| needle.is_in(author))
                    else {
                        return false;
                    };
                    in_author
                        || match reader.message(id) {
                            Some(message) => needle.is_in(message),
                            None => return false,
                        }
                }
            }
        };
        if held == check.negated {
            return false;
        }
    }
    true
}

/// Whether a message, as [`fold`] prepared it, names an issue. The fold
/// put the letters in lowercase, which keys compare without regard to.
fn names_issue(message: &[u8], key: &crate::issue_keys::IssueKey) -> bool {
    let text = String::from_utf8_lossy(message);
    crate::issue_keys::keys_in(&text)
        .iter()
        .any(|found| key.finds(&found.key))
}

/// The commits of `walked` that are `wanted`, in that order, up to `limit`.
/// Looks at the cancel flag as it goes and reports how far it is.
fn pick(
    walked: &[GraphCommit],
    limit: usize,
    reporter: &mut Reporter<'_>,
    mut wanted: impl FnMut(&GraphCommit) -> bool,
) -> Result<Found> {
    let mut found = Found::default();
    let mut reported = None;
    for (ix, commit) in walked.iter().enumerate() {
        if ix % LOOK_EVERY == 0 {
            if reporter.cancel.load(Ordering::SeqCst) {
                return Err(cancelled());
            }
            let percent = u8::try_from(ix.saturating_mul(100) / walked.len().max(1)).unwrap_or(100);
            if reported != Some(percent) {
                reported = Some(percent);
                (reporter.on_progress)(Progress {
                    stage: say!("search-stage-matching").to_owned(),
                    percent: Some(percent),
                });
            }
        }
        if !wanted(commit) {
            continue;
        }
        if found.ids.len() >= limit {
            found.truncated = true;
            break;
        }
        found.ids.push(commit.id);
    }
    Ok(found)
}

/// Run git for a list of commit ids, one to a line. Stops on the flag.
fn ask(repo: &Repository, args: &[&str], cancel: &Arc<AtomicBool>) -> Result<HashSet<Oid>> {
    let out = exe::run_streaming(repo, args, cancel, &mut |_| {}).map_err(|err| {
        // A regular expression git cannot read is the user's to put
        // right: say what git says of it, without the command line.
        if let Error::Command { stderr, .. } = &err
            && let Some(reason) = stderr
                .lines()
                .find_map(|line| line.strip_prefix("fatal: invalid regex: "))
        {
            return invalid(say!("search-what-regex"), shown(reason));
        }
        err
    })?;
    Ok(out
        .stdout
        .lines()
        .filter_map(|line| line.parse().ok())
        .collect())
}

/// `git log` printing ids only. Two settings a user may have made for
/// reading a log are kept out of it: `log.follow` would make one path mean
/// something else than two, and `log.showSignature` would check and print
/// the signature of every commit listed.
const LOG_IDS: [&str; 6] = [
    "-c",
    "log.follow=false",
    "-c",
    "log.showSignature=false",
    "log",
    "--format=%H",
];

/// The starting points of a walk as git is told them. What is asked about
/// is a commit's own, given `--full-history` wherever paths are named, so
/// git walking further than the graph does (never less) changes nothing;
/// naming the starting points only saves it work.
fn tip_args(repo: &Repository, tips: &Tips, walked: &[GraphCommit]) -> Result<Vec<String>> {
    let head = || repo.gix().head_id().ok().map(|_| "HEAD".to_owned());
    let all = || {
        ["--branches", "--remotes", "--tags"]
            .into_iter()
            .map(str::to_owned)
            .chain(head())
            .collect()
    };
    Ok(match tips {
        Tips::All => all(),
        Tips::Head => head().into_iter().collect(),
        Tips::Ids(_) => {
            // However many were named, and whatever among them is not a
            // commit: the commits of the walk that are no other's parent
            // reach all of it, and they are usually few.
            let parents: HashSet<Oid> = walked
                .iter()
                .flat_map(|commit| commit.parents.iter().copied())
                .collect();
            let heads: Vec<String> = walked
                .iter()
                .filter(|commit| !parents.contains(&commit.id))
                .take(MAX_TIPS_FOR_GIT.saturating_add(1))
                .map(|commit| commit.id.to_hex())
                .collect();
            if heads.len() > MAX_TIPS_FOR_GIT {
                return Err(refused(say!(
                    "search-too-many-tips",
                    most = MAX_TIPS_FOR_GIT
                )));
            }
            if heads.is_empty() {
                // Only a history that goes in a circle has none.
                all()
            } else {
                heads
            }
        }
    })
}

/// The commit a revision names.
fn commit_named(repo: &Repository, rev: &str) -> Result<Oid> {
    let bad = || {
        invalid(
            say!("search-what-revision"),
            say!("search-not-a-commit", rev = format!("{:?}", shown(rev))),
        )
    };
    check_rev(rev).map_err(|_| bad())?;
    let spec = format!("{rev}^{{commit}}");
    // The answer has to be an id and nothing else: `^main` is one
    // revision to git too, "not main", and comes back with its caret.
    exe::run(
        repo,
        ["rev-parse", "--verify", "--quiet", spec.as_str()],
        None,
    )
    .ok()
    .and_then(|out| out.stdout.trim().parse().ok())
    .ok_or_else(bad)
}

/// The set of commits a word that asks git holds for.
fn asked(
    repo: &Repository,
    word: &Word,
    tips: &[String],
    reporter: &mut Reporter<'_>,
) -> Result<HashSet<Oid>> {
    let mut tell = |stage: &str| {
        (reporter.on_progress)(Progress {
            stage: stage.to_owned(),
            percent: None,
        });
    };
    let mut args: Vec<&str> = Vec::new();
    let flag;
    let commit;
    match word.key {
        Key::Ref => {
            tell(say!("search-stage-listing"));
            commit = commit_named(repo, &word.value)?.to_hex();
            args.extend(["rev-list", commit.as_str(), "--"]);
        }
        Key::Path => {
            tell(say!("search-stage-paths"));
            check_path(&word.value)
                .map_err(|expected| invalid(say!("search-what-path"), expected))?;
            // What `git log --full-history -- <path>` lists, from the
            // command that no setting for reading a log reaches.
            args.extend(["rev-list", "--full-history"]);
            args.extend(tips.iter().map(String::as_str));
            args.extend(["--", word.value.as_str()]);
        }
        Key::Change | Key::Regex => {
            tell(say!("search-stage-content"));
            check_needle(&word.value)?;
            let option = if word.key == Key::Change { "-S" } else { "-G" };
            flag = format!("{option}{}", word.value);
            args.extend(LOG_IDS);
            args.push(flag.as_str());
            args.extend(tips.iter().map(String::as_str));
            args.push("--");
        }
        _ => return Ok(HashSet::new()),
    }
    ask(repo, &args, &reporter.cancel)
}

/// What is searched for in content: something, on one line, that fits on a
/// command line.
fn check_needle(text: &str) -> Result<()> {
    if text.is_empty()
        || text.len() > MAX_QUERY_BYTES
        || text.chars().any(|c| c.is_control() && c != '\t')
    {
        return Err(invalid(
            say!("search-what-text"),
            say!("search-text-bounds", most = MAX_QUERY_BYTES),
        ));
    }
    Ok(())
}

/// The commits a query holds for, among those reachable from the tips, in
/// the order [`walk`] gives them.
///
/// Stops on the reporter's flag: between commits while it reads them, and
/// by ending git while that searches.
pub fn commits(
    repo: &Repository,
    query: &Query,
    options: &SearchOptions,
    reporter: &mut Reporter<'_>,
) -> Result<Found> {
    if query.words.len() > MAX_WORDS {
        return Err(invalid(
            say!("search-what-query"),
            say!("search-too-many-words", most = MAX_WORDS),
        ));
    }
    // Stopped in the middle of the walk as well: on a long history that
    // is most of the time a search of the commits themselves takes.
    let walked = crate::pieces::walk_unless(
        repo,
        &LogOptions {
            tips: options.tips.clone(),
            ..Default::default()
        },
        &reporter.cancel,
    )?
    .ok_or_else(cancelled)?;
    if walked.is_empty() {
        return Ok(Found::default());
    }
    let mut checks = Vec::with_capacity(query.words.len());
    for word in query.words.iter().filter(|word| !word.key.asks_git()) {
        let test = match word.key {
            Key::Text => Test::Text {
                needle: Needle::new(&word.value),
                id: hex_digits(&word.value),
            },
            Key::Message => Test::Message(Needle::new(&word.value)),
            Key::Issue => {
                let key = crate::issue_keys::IssueKey::parse(&word.value)
                    .ok_or_else(|| invalid(say!("search-what-query"), ISSUE_FORMS.text()))?;
                let needle = Needle::new(&format!("#{}", key.number));
                Test::Issue(key, needle)
            }
            Key::Author => Test::Author(Needle::new(&word.value)),
            Key::Committer => Test::Committer(Needle::new(&word.value)),
            Key::Sha => {
                let digits = hex_digits(&word.value).ok_or_else(|| {
                    invalid(say!("search-what-query"), say!("search-sha-takes-hex"))
                })?;
                Test::Id(digits)
            }
            Key::Before | Key::After => {
                let seconds = word.seconds.ok_or_else(|| {
                    invalid(
                        say!("search-what-query"),
                        say!("search-date-without-instant"),
                    )
                })?;
                if word.key == Key::Before {
                    Test::Before(seconds)
                } else {
                    Test::After(seconds)
                }
            }
            _ => Test::Merge,
        };
        checks.push(Check {
            negated: word.negated,
            test,
        });
    }
    // Git's answers last, and the quick question before the slow ones: a
    // word that holds for nothing ends the search before they are asked.
    let mut asking: Vec<&Word> = query
        .words
        .iter()
        .filter(|word| word.key.asks_git())
        .collect();
    asking.sort_by_key(|word| match word.key {
        Key::Ref => 0,
        Key::Path => 1,
        _ => 2,
    });
    if !asking.is_empty() {
        let tips = tip_args(repo, &options.tips, &walked)?;
        for word in asking {
            let set = asked(repo, word, &tips, reporter)?;
            if set.is_empty() && !word.negated {
                return Ok(Found::default());
            }
            checks.push(Check {
                negated: word.negated,
                test: Test::Among(set),
            });
        }
    }
    checks.sort_by_key(|check| check.test.cost());
    let mut reader = Reader::new(repo.gix());
    pick(&walked, options.limit, reporter, |commit| {
        holds(&checks, commit, &mut reader)
    })
}

/// A search of content alone.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentSearch {
    /// What to look for.
    pub text: String,
    /// Read `text` as a regular expression and look for changed lines
    /// that match it (`git log -G`). Otherwise it is a string, and a
    /// commit matches when it adds or removes it (`git log -S`).
    #[serde(default)]
    pub regex: bool,
    /// Look only at these paths; everywhere when empty.
    #[serde(default)]
    pub paths: Vec<String>,
}

/// The commits whose changes hold what a content search looks for, among
/// those reachable from the tips, in the order [`walk`] gives them. Takes
/// as long as git takes to look at every diff, and stops on the flag.
pub fn content(
    repo: &Repository,
    search: &ContentSearch,
    options: &SearchOptions,
    reporter: &mut Reporter<'_>,
) -> Result<Found> {
    check_needle(&search.text)?;
    if search.paths.len() > MAX_PATHS {
        return Err(invalid(
            say!("search-what-paths"),
            say!("search-too-many-paths", most = MAX_PATHS),
        ));
    }
    for path in &search.paths {
        check_path(path).map_err(|expected| invalid(say!("search-what-path"), expected))?;
    }
    let walked = crate::pieces::walk_unless(
        repo,
        &LogOptions {
            tips: options.tips.clone(),
            ..Default::default()
        },
        &reporter.cancel,
    )?
    .ok_or_else(cancelled)?;
    if walked.is_empty() {
        return Ok(Found::default());
    }
    let tips = tip_args(repo, &options.tips, &walked)?;
    let flag = format!("{}{}", if search.regex { "-G" } else { "-S" }, search.text);
    let mut args: Vec<&str> = LOG_IDS.to_vec();
    args.push(flag.as_str());
    if !search.paths.is_empty() {
        // Every commit that touches the paths, as `path:` has it, not
        // the simplified history git shows for a path by default.
        args.push("--full-history");
    }
    args.extend(tips.iter().map(String::as_str));
    args.push("--");
    args.extend(search.paths.iter().map(String::as_str));
    (reporter.on_progress)(Progress {
        stage: say!("search-stage-content").to_owned(),
        percent: None,
    });
    let set = ask(repo, &args, &reporter.cancel)?;
    if set.is_empty() {
        return Ok(Found::default());
    }
    pick(&walked, options.limit, reporter, |commit| {
        set.contains(&commit.id)
    })
}

/// The first commit in the order of the graph whose commit date is at or
/// before `seconds`: where to jump to for a date. None when every commit
/// is newer.
pub fn locate_date(repo: &Repository, seconds: i64, tips: &Tips) -> Result<Option<Oid>> {
    let walked = walk(
        repo,
        &LogOptions {
            tips: tips.clone(),
            ..Default::default()
        },
    )?;
    let mut reader = Reader::new(repo.gix());
    Ok(walked
        .iter()
        .find(|commit| {
            reader.turn();
            commit
                .seconds
                .or_else(|| reader.seconds(&commit.id))
                .is_some_and(|at| at <= seconds)
        })
        .map(|commit| commit.id))
}

/// The commit a revision names: a branch, a tag (peeled), an id or its
/// beginning, or any other expression git reads as one commit.
pub fn locate_rev(repo: &Repository, rev: &str) -> Result<Oid> {
    commit_named(repo, rev)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2020-01-01T00:00:00Z.
    const NOW: i64 = 1_577_836_800;

    /// Most of the tests below are about what a query says, not about
    /// where it is read: they read their dates in UTC, whatever the time
    /// zone of the machine they run on.
    fn parse(text: &str, now: i64) -> Parsed<Query> {
        parse_in(text, now, &Zone::utc())
    }

    fn instant(text: &str, now: i64) -> Option<i64> {
        instant_in(text, now, &Zone::utc())
    }

    /// A zone of the time zone database. None where the system has no
    /// such database to ask; what is about that zone is then left out.
    fn zone(name: &str) -> Option<Zone> {
        Zone::named(name)
    }

    #[test]
    fn a_date_is_read_in_its_time_zone() {
        let utc = |text: &str| instant_in(text, 0, &Zone::utc()).unwrap();
        // East of Greenwich a day begins earlier, west of it later; an
        // hour more in summer where the clocks are moved.
        for (name, date, same_as_utc) in [
            ("UTC", "2020-01-01", "2020-01-01T00:00"),
            ("Europe/Vienna", "2020-01-01", "2019-12-31T23:00"),
            ("Europe/Vienna", "2020-07-01", "2020-06-30T22:00"),
            ("Europe/Vienna", "2020-07-01T12:00", "2020-07-01T10:00"),
            ("America/New_York", "2020-01-01", "2020-01-01T05:00"),
            ("America/New_York", "2020-07-01T23:59", "2020-07-02T03:59"),
            ("Asia/Kolkata", "2020-01-01", "2019-12-31T18:30"),
            ("Pacific/Auckland", "2020-01-01T00:00", "2019-12-31T11:00"),
            // The clocks go forward at 02:00 on 2020-03-29 in Vienna:
            // there is no 02:30, and it is read as half an hour after
            // the jump. They go back at 03:00 on 2020-10-25: 02:30 is
            // there twice, and it is the first of the two.
            ("Europe/Vienna", "2020-03-29T02:30", "2020-03-29T01:30"),
            ("Europe/Vienna", "2020-10-25T02:30", "2020-10-25T00:30"),
        ] {
            let Some(zone) = zone(name) else { continue };
            assert_eq!(
                instant_in(date, 0, &zone),
                Some(utc(same_as_utc)),
                "{date} in {name}"
            );
        }
        // An age of months is counted by the zone's calendar and clock:
        // noon a month before noon, also across the day the clocks moved.
        if let Some(vienna) = zone("Europe/Vienna") {
            let at = |text: &str| instant_in(text, 0, &vienna).unwrap();
            let noon = at("2020-04-15T12:00");
            assert_eq!(
                instant_in("1m", noon, &vienna),
                Some(at("2020-03-15T12:00"))
            );
            assert_eq!(
                instant_in("1m", noon, &Zone::utc()),
                Some(at("2020-03-15T11:00")),
                "the same age by UTC's clock is an hour off there"
            );
            // The day of the month is the zone's: late in the evening
            // of the 31st in Vienna it is the 31st, though not yet in UTC.
            let late = at("2020-03-31T00:30");
            assert_eq!(
                instant_in("1m", late, &vienna),
                Some(at("2020-02-29T00:30"))
            );
            // Days and weeks are the same everywhere.
            assert_eq!(instant_in("3d", noon, &vienna), Some(noon - 3 * 86_400));
        }
        // A word of the query carries the instant of its zone.
        if let Some(zone) = zone("Asia/Kolkata") {
            let query = parse_in("before:2020-01-01 after:2019-12-31T12:00", NOW, &zone).unwrap();
            assert_eq!(query.words[0].seconds, Some(utc("2019-12-31T18:30")));
            assert_eq!(query.words[1].seconds, Some(utc("2019-12-31T06:30")));
        }
        // The system's zone is some zone: a date is within a day of UTC's.
        let here = super::instant("2020-01-01", 0).unwrap();
        assert!((here - NOW).abs() <= 14 * 3600, "{here}");
    }

    #[test]
    fn a_time_zone_is_a_name_from_the_database_and_nothing_else() {
        assert!(Zone::named("UTC").is_some());
        let long = "A".repeat(65);
        for name in [
            "",
            " ",
            "Europe/Nowhere",
            "../../../etc/passwd",
            "/etc/localtime",
            "Europe/Vienna\n",
            "Europe Vienna",
            "a\u{0}b",
            long.as_str(),
            "🦑",
        ] {
            assert!(Zone::named(name).is_none(), "{name:?}");
        }
        // A date outside what a zone's calendar reaches is still a date.
        if let Some(zone) = zone("Europe/Vienna") {
            assert!(instant_in("0000-01-01", 0, &zone).is_some());
            assert!(instant_in("9999-12-31T23:59", 0, &zone).is_some());
            for now in [i64::MIN, i64::MAX] {
                assert!(instant_in("1m", now, &zone).is_some());
                assert!(instant_in("4294967295y", now, &zone).is_some());
            }
        }
    }

    fn words(text: &str) -> Vec<(Key, String, bool)> {
        parse(text, NOW)
            .unwrap_or_else(|err| panic!("{text:?}: {err}"))
            .words
            .into_iter()
            .map(|word| (word.key, word.value, word.negated))
            .collect()
    }

    fn wrong(text: &str) -> (usize, usize, String) {
        let err = parse(text, NOW).expect_err(text);
        (err.position, err.end, err.expected)
    }

    #[test]
    fn an_issue_is_a_number_with_a_repository_or_without() {
        assert_eq!(
            words("issue:12 issue:#12 -Issue:Octo/Tools#5 issue:\"g/s/p#7\""),
            [
                (Key::Issue, "#12".to_owned(), false),
                (Key::Issue, "#12".to_owned(), false),
                (Key::Issue, "Octo/Tools#5".to_owned(), true),
                (Key::Issue, "g/s/p#7".to_owned(), false),
            ]
        );
        for bad in [
            "issue:x",
            "issue:#0",
            "issue:tools#5",
            "issue:!12",
            "issue:a/b#",
        ] {
            assert_eq!(wrong(bad), (6, bad.len(), ISSUE_FORMS.to_string()), "{bad}");
        }
    }

    #[test]
    fn words_keys_quotes_and_negation() {
        assert!(words("").is_empty());
        assert!(words("  \t\n ").is_empty());
        assert_eq!(
            words("fix  -wip author:ada -committer:Bob"),
            [
                (Key::Text, "fix".to_owned(), false),
                (Key::Text, "wip".to_owned(), true),
                (Key::Author, "ada".to_owned(), false),
                (Key::Committer, "Bob".to_owned(), true),
            ]
        );
        // Quotes keep spaces; a colon inside them is text.
        assert_eq!(
            words(r#""fix: typo" author:"Ada L" -"not this" -message:"a b""#),
            [
                (Key::Text, "fix: typo".to_owned(), false),
                (Key::Author, "Ada L".to_owned(), false),
                (Key::Text, "not this".to_owned(), true),
                (Key::Message, "a b".to_owned(), true),
            ]
        );
        // Only the first colon divides; a dash inside a word is a dash.
        assert_eq!(
            words("after:2020-01-01T10:30 ref:a-b path:a:b half-done"),
            [
                (Key::After, "2020-01-01T10:30".to_owned(), false),
                (Key::Ref, "a-b".to_owned(), false),
                (Key::Path, "a:b".to_owned(), false),
                (Key::Text, "half-done".to_owned(), false),
            ]
        );
        // Keys and `merge` in any case, ids in lowercase.
        assert_eq!(
            words("Author:x IS:Merge sha:ABCdef"),
            [
                (Key::Author, "x".to_owned(), false),
                (Key::Is, "merge".to_owned(), false),
                (Key::Sha, "abcdef".to_owned(), false),
            ]
        );
        // Inside quotes: `\"` a quote, `\\` a backslash, any other
        // backslash itself; a tab stays.
        assert_eq!(
            words(r#"change:"say \"hi\"" regex:"\d+\\n" "-dash" "a	b""#),
            [
                (Key::Change, r#"say "hi""#.to_owned(), false),
                (Key::Regex, r"\d+\n".to_owned(), false),
                (Key::Text, "-dash".to_owned(), false),
                (Key::Text, "a\tb".to_owned(), false),
            ]
        );
        assert_eq!(
            words("ünï -message:çé"),
            [
                (Key::Text, "ünï".to_owned(), false),
                (Key::Message, "çé".to_owned(), true),
            ]
        );
    }

    #[test]
    fn spans_are_bytes_of_the_query() {
        let text = r#"é -author:"A b"  before:1d"#;
        let query = parse(text, NOW).unwrap();
        let spans: Vec<&str> = query
            .words
            .iter()
            .map(|word| &text[word.start..word.end])
            .collect();
        assert_eq!(spans, ["é", r#"-author:"A b""#, "before:1d"]);
        assert_eq!(query.words[2].seconds, Some(NOW - 86_400));
        assert_eq!(query.words[0].seconds, None);
    }

    #[test]
    fn what_is_wrong_and_where() {
        // A lone dash, at the end and in the middle.
        assert_eq!(wrong("-").0, 0);
        assert_eq!(wrong("a - b"), (2, 3, "a word after `-`".to_owned()));
        // A second dash is not read as "not not".
        let (position, end, expected) = wrong("a --b");
        assert_eq!((position, end), (3, 4));
        assert!(expected.contains("in quotes"), "{expected}");
        // An unclosed quote: from where it opens.
        assert_eq!(wrong(r#"a "b c"#), (2, 6, "a closing quote".to_owned()));
        assert_eq!(wrong(r#"author:"b"#).0, 7);
        assert_eq!(wrong(r#""a\""#).2, "a closing quote");
        // Something glued to a closing quote, and a quote in mid-word.
        assert_eq!(wrong(r#""a"b"#).0, 3);
        assert_eq!(wrong(r#"a"b"#).0, 1);
        assert_eq!(wrong(r#"author:x"y"#).0, 8);
        // An empty value.
        assert_eq!(
            wrong("fix author:"),
            (11, 11, "a value after `author:`".to_owned())
        );
        assert_eq!(wrong(r#"author:"" x"#).0, 7);
        assert_eq!(
            wrong(r#"x """#),
            (2, 4, "text between the quotes".to_owned())
        );
        assert_eq!(wrong(r#"-"""#).0, 1);
        // An unknown key names the ones there are.
        let (position, end, expected) = wrong("fix foo:bar");
        assert_eq!((position, end), (4, 7));
        for (name, _) in KEYS {
            assert!(expected.contains(&format!("{name}:")), "{expected}");
        }
        assert_eq!(wrong(":x").0, 0);
        assert_eq!(wrong("http://example.invalid").1, 4);
        // Values that do not fit their key.
        assert_eq!(wrong("is:fast").0, 3);
        assert_eq!(wrong("sha:xyz").0, 4);
        assert_eq!(wrong(&format!("sha:{}", "a".repeat(65))).0, 4);
        assert_eq!(wrong("x before:tuesday"), (9, 16, DATE_FORMS.to_string()));
        assert_eq!(wrong("ref:-x").0, 4);
        assert_eq!(wrong("ref:--all").0, 4);
        assert_eq!(wrong("path:-x").0, 5);
        assert_eq!(wrong("path::(top)x").0, 5);
        assert_eq!(wrong(r#"ref:"a	b""#).0, 4);
        // Control characters, bare and quoted.
        assert_eq!(
            wrong("a\u{1}b"),
            (1, 2, "text without control characters".to_owned())
        );
        assert_eq!(wrong("\"a\nb\"").0, 2);
        assert_eq!(wrong("x\0").0, 1);
    }

    #[test]
    fn a_query_that_is_too_much_is_refused_unread() {
        let long = "a".repeat(MAX_QUERY_BYTES + 1);
        assert_eq!(wrong(&long).0, MAX_QUERY_BYTES);
        assert!(parse(&"a".repeat(MAX_QUERY_BYTES), NOW).is_ok());
        // Cut on a character, not inside one.
        let wide = format!("a{}", "é".repeat(MAX_QUERY_BYTES));
        let (position, end, _) = wrong(&wide);
        assert_eq!((position, end), (MAX_QUERY_BYTES - 1, wide.len()));
        // Megabytes cost nothing: the length is all that is looked at.
        let huge = "-".repeat(8 * 1024 * 1024);
        assert!(parse(&huge, NOW).is_err());

        let many = "a ".repeat(MAX_WORDS + 1);
        assert_eq!(wrong(&many).0, MAX_WORDS * 2);
        assert_eq!(words(&"a ".repeat(MAX_WORDS)).len(), MAX_WORDS);
        // Dashes without end are refused at the second.
        assert_eq!(wrong(&"-".repeat(4000)).0, 1);
        assert_eq!(wrong(&"- ".repeat(2000)).0, 0);
        assert_eq!(wrong(&"\"".repeat(4001)).0, 2);
    }

    #[test]
    fn dates_are_instants_in_utc() {
        assert_eq!(instant("1970-01-01", 0), Some(0));
        assert_eq!(instant("2020-01-01", 0), Some(NOW));
        assert_eq!(
            instant("2020-01-01T10:30", 0),
            Some(NOW + 10 * 3600 + 30 * 60)
        );
        assert_eq!(instant("2024-02-29", 0), Some(1_709_164_800));
        assert_eq!(instant("2000-02-29T23:59", 0), Some(951_868_740));
        assert_eq!(instant("1969-12-31", 0), Some(-86_400));
        assert_eq!(instant("0001-01-01", 0), Some(-62_135_596_800));
        for bad in [
            "",
            "2020",
            "2020-1-1",
            "2020-13-01",
            "2020-00-10",
            "2020-01-00",
            "2020-01-32",
            "2023-02-29",
            "1900-02-29",
            "2020-01-01T24:00",
            "2020-01-01T10:60",
            "2020-01-01 10:30",
            "2020-01-01T10:30:00",
            "2020/01/01",
            "+020-01-01",
            "２０２０-01-01",
            "yesterday",
            "d",
            "3",
            "3h",
            "-3d",
            "3 d",
            "3D",
            "1.5d",
            "99999999999d",
            "3é",
        ] {
            assert_eq!(instant(bad, NOW), None, "{bad:?}");
        }
    }

    #[test]
    fn ages_count_back_by_the_calendar() {
        let at = |text: &str| instant(text, 0).unwrap();
        assert_eq!(instant("0d", NOW), Some(NOW));
        assert_eq!(instant("3d", NOW), Some(NOW - 3 * 86_400));
        assert_eq!(instant("2w", NOW), Some(NOW - 14 * 86_400));
        // The same day, and time of day, of an earlier month.
        let noon = at("2026-10-10T12:00");
        assert_eq!(instant("6m", noon), Some(at("2026-04-10T12:00")));
        assert_eq!(instant("10m", noon), Some(at("2025-12-10T12:00")));
        assert_eq!(instant("1y", noon), Some(at("2025-10-10T12:00")));
        assert_eq!(instant("26m", noon), Some(at("2024-08-10T12:00")));
        // A day the earlier month does not have: its last one.
        assert_eq!(instant("1m", at("2026-03-31")), Some(at("2026-02-28")));
        assert_eq!(instant("1m", at("2024-03-31")), Some(at("2024-02-29")));
        assert_eq!(instant("1y", at("2024-02-29")), Some(at("2023-02-28")));
        assert_eq!(instant("1m", at("2026-01-15")), Some(at("2025-12-15")));
        // Before the epoch, and numbers nobody means: an answer, no panic.
        assert_eq!(instant("1m", at("1970-01-15")), Some(at("1969-12-15")));
        for now in [i64::MIN, -1, 0, i64::MAX] {
            for text in [
                "4294967295d",
                "4294967295w",
                "4294967295m",
                "4294967295y",
                "1m",
            ] {
                assert!(instant(text, now).is_some(), "{text} from {now}");
            }
        }
    }

    #[test]
    fn day_counts_go_there_and_back() {
        for days in (-800_000..800_000).step_by(37) {
            let (year, month, day) = civil_from_days(days);
            assert!((1..=12).contains(&month) && (1..=days_in_month(year, month)).contains(&day));
            assert_eq!(days_from_civil(year, month, day), days);
        }
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(18_262), (2020, 1, 1));
    }

    #[test]
    fn text_is_found_without_regard_to_case() {
        let mut folded = Vec::new();
        let mut found = |needle: &str, parts: &[&[u8]]| {
            fold(parts, &mut folded);
            Needle::new(needle).is_in(&folded)
        };
        assert!(found("FIX", &[b"a Fix for it"]));
        assert!(found("fix", &[b"A FIX"]));
        assert!(!found("fixx", &[b"a fix"]));
        // Across the parts of a person, as git reads the header.
        assert!(found(
            "Fixture <ADA@",
            &[b"Ada Fixture", b" <", b"ada@fixture.invalid", b">"]
        ));
        assert!(found("ÉCOLE", &["à l'école".as_bytes()]));
        assert!(found("école", &["À L'ÉCOLE".as_bytes()]));
        assert!(!found("é", &[b"ecole"]));
        // Not UTF-8: read as far as it can be.
        assert!(found("caf", &[b"Caf\xe9 au lait"]));
        assert!(found("lait", &[b"Caf\xe9 au LAIT"]));
    }

    #[test]
    fn ids_are_matched_by_their_beginning() {
        let id: Oid = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();
        let begins = |text: &str| hex_digits(text).is_some_and(|digits| id_begins(&id, &digits));
        assert!(begins("0"));
        assert!(begins("012"));
        assert!(begins("0123456789ABCDEF"));
        assert!(begins("0123456789abcdef0123456789abcdef01234567"));
        assert!(!begins("1"));
        assert!(!begins("0124"));
        // Longer than an id, and not hexadecimal at all.
        assert!(!begins("0123456789abcdef0123456789abcdef012345670"));
        assert!(!begins("01g"));
        assert!(hex_digits(&"a".repeat(65)).is_none());
    }
}
