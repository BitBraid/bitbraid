//! bitbraid-strings: the string catalog.
//!
//! Every word a person reads, a label, a sentence, a heading, an error,
//! is a message in a Fluent file and is looked up by its id. Each crate
//! that says anything has its own catalog: `strings/en/<area>.ftl` beside
//! its `Cargo.toml`, one file per area of the crate so that two changes
//! to different areas do not meet in one file. English is the only
//! language.
//!
//! A crate declares its files once, in a module named `strings`:
//!
//! ```ignore
//! bitbraid_strings::catalog!["graph", "refs"];
//! ```
//!
//! and looks a message up with [`say!`]:
//!
//! ```ignore
//! .child(say!("graph-loading"))                    // &'static str
//! .child(say!("graph-commits", count = rows.len())) // String
//! ```
//!
//! A message without arguments is formatted once, when the catalog is
//! first read, and handed out as a `&'static str`, as a literal would
//! be; one with arguments is formatted when it is asked for.
//! Plurals and the words around a number are the message's business
//! (Fluent's selectors), never the code's. Where a message has to be
//! named before it is shown, in a table of `static`s, [`message!`] makes a
//! [`Message`] that is looked up later.
//!
//! Each message's id starts with its file's area (`graph-` in
//! `graph.ftl`). The terms every catalog may use (`{ -product }`) are in
//! this crate's `strings/en/common.ftl`.
//!
//! Nothing here can crash: an id that is not in the catalog is shown as
//! itself, and a file that does not parse gives what it could. The
//! crates' tests (feature `check`) see to it that neither happens: every
//! id looked up is in the catalog with the arguments its message takes,
//! every message is looked up somewhere, and, for the interface, no
//! sentence is written into the code.
//!
//! Sources: Project Fluent's syntax guide and specification
//! (<https://projectfluent.org/fluent/guide/>, the EBNF in
//! `projectfluent/fluent`, `spec/fluent.ebnf`); the documentation of the
//! `fluent-bundle` and `fluent-syntax` crates; Unicode CLDR's plural rules
//! for English, which `intl_pluralrules` carries.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::{borrow::Cow, collections::HashMap, fmt, sync::OnceLock};

use fluent_bundle::{FluentArgs, FluentResource, FluentValue, concurrent::FluentBundle};

#[cfg(feature = "check")]
pub mod check;

/// The language of the catalogs.
pub const LANGUAGE: &str = "en";

/// The terms every catalog may use, ahead of its own files.
pub const COMMON: &str = include_str!("../strings/en/common.ftl");

/// Declares the calling crate's catalog, `CATALOG`, from its files
/// `strings/en/<area>.ftl`, in the module this is written in (which
/// [`say!`] and [`message!`] expect to be the crate's `strings`).
#[macro_export]
macro_rules! catalog {
    ($($area:literal),* $(,)?) => {
        /// The crate's messages, read from its Fluent files when one is
        /// first asked for.
        pub static CATALOG: $crate::Catalog = $crate::Catalog::new(
            env!("CARGO_PKG_NAME"),
            &[$((
                $area,
                include_str!(concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/strings/en/",
                    $area,
                    ".ftl"
                )),
            )),*],
        );
    };
}

/// A message of the calling crate's catalog, by its id: a `&'static str`
/// for a message without arguments, a `String` for one with them.
///
/// Arguments are named as the message names them: `count = rows.len()`,
/// or only `count` for a variable of that name. A value is text (`&str`,
/// `String`) or a number, which is what plurals select on.
///
/// The catalog is the calling crate's own: `crate::strings::CATALOG`.
#[macro_export]
#[allow(clippy::crate_in_macro_def)]
macro_rules! say {
    ($id:literal $(,)?) => {
        crate::strings::CATALOG.text($id)
    };
    ($id:literal, $($name:ident $(= $value:expr)?),+ $(,)?) => {
        crate::strings::CATALOG.format(
            $id,
            &[$((stringify!($name), $crate::argument!($name $(= $value)?))),+],
        )
    };
}

/// One argument of [`say!`]: the value given, or the variable of the
/// argument's name.
#[doc(hidden)]
#[macro_export]
macro_rules! argument {
    ($name:ident) => {
        $crate::Arg::from($name)
    };
    ($name:ident = $value:expr) => {
        $crate::Arg::from($value)
    };
}

/// A message of the calling crate's catalog, named now and looked up
/// later ([`Message::text`]): for tables of `static`s, where a lookup
/// cannot run.
#[macro_export]
#[allow(clippy::crate_in_macro_def)]
macro_rules! message {
    ($id:literal $(,)?) => {
        $crate::Message::new(&crate::strings::CATALOG, $id)
    };
}

/// A value given to a message: text, or a number.
#[derive(Debug, Clone)]
pub struct Arg<'a>(FluentValue<'a>);

impl<'a> From<&'a str> for Arg<'a> {
    fn from(value: &'a str) -> Self {
        Self(FluentValue::from(value))
    }
}

impl<'a> From<&'a String> for Arg<'a> {
    fn from(value: &'a String) -> Self {
        Self(FluentValue::from(value.as_str()))
    }
}

impl From<String> for Arg<'_> {
    fn from(value: String) -> Self {
        Self(FluentValue::from(value))
    }
}

impl<'a> From<Cow<'a, str>> for Arg<'a> {
    fn from(value: Cow<'a, str>) -> Self {
        Self(FluentValue::from(value))
    }
}

/// The largest whole number a Fluent number (a 64-bit float) holds
/// exactly: 2^53.
const EXACT: u64 = 1 << 53;

macro_rules! whole_numbers {
    ($($kind:ty),*) => {
        $(
            impl From<$kind> for Arg<'_> {
                /// A number, which plurals select on; past 2^53, where a
                /// float would round it, its digits as text (which
                /// selects the default variant, as such a number does).
                fn from(value: $kind) -> Self {
                    let exact = u64::try_from(value)
                        .ok()
                        .or_else(|| i64::try_from(value).ok().map(i64::unsigned_abs))
                        .is_some_and(|size| size <= EXACT);
                    if exact {
                        Self(FluentValue::from(value))
                    } else {
                        Self(FluentValue::from(value.to_string()))
                    }
                }
            }
        )*
    };
}

whole_numbers!(u8, u16, u32, u64, usize, i8, i16, i32, i64, isize);

impl From<f32> for Arg<'_> {
    fn from(value: f32) -> Self {
        Self(FluentValue::from(value))
    }
}

impl From<f64> for Arg<'_> {
    fn from(value: f64) -> Self {
        Self(FluentValue::from(value))
    }
}

/// A message named before it is looked up.
#[derive(Clone, Copy)]
pub struct Message {
    catalog: &'static Catalog,
    id: &'static str,
}

impl Message {
    /// The message `id` of `catalog`. [`message!`] is the way to make one.
    pub const fn new(catalog: &'static Catalog, id: &'static str) -> Self {
        Self { catalog, id }
    }

    pub fn id(&self) -> &'static str {
        self.id
    }

    /// What it says.
    pub fn text(&self) -> &'static str {
        self.catalog.text(self.id)
    }
}

impl fmt::Debug for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Message").field(&self.id).finish()
    }
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text())
    }
}

impl PartialEq for Message {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && std::ptr::eq(self.catalog, other.catalog)
    }
}

impl Eq for Message {}

/// A message is written as what it says, where a table that names it is
/// written out (`commands --json`).
#[cfg(feature = "serde")]
impl serde::Serialize for Message {
    fn serialize<S: serde::Serializer>(&self, to: S) -> Result<S::Ok, S::Error> {
        to.serialize_str(self.text())
    }
}

/// A crate's messages: its Fluent files, read when the first one is asked
/// for. [`catalog!`] declares one.
pub struct Catalog {
    owner: &'static str,
    files: &'static [(&'static str, &'static str)],
    loaded: OnceLock<Loaded>,
}

struct Loaded {
    bundle: FluentBundle<FluentResource>,
    /// The messages without arguments, formatted.
    plain: HashMap<String, Box<str>>,
    /// What went wrong in reading the files.
    problems: Vec<String>,
}

impl fmt::Debug for Catalog {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Catalog")
            .field("owner", &self.owner)
            .field("areas", &self.areas().collect::<Vec<_>>())
            .finish()
    }
}

impl Catalog {
    /// The catalog of the crate `owner`, from its files: each an area's
    /// name and what the file holds.
    pub const fn new(owner: &'static str, files: &'static [(&'static str, &'static str)]) -> Self {
        Self {
            owner,
            files,
            loaded: OnceLock::new(),
        }
    }

    /// The crate whose catalog this is.
    pub fn owner(&self) -> &'static str {
        self.owner
    }

    /// The names of its areas, in order.
    pub fn areas(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.files.iter().map(|(area, _)| *area)
    }

    /// Its files: each an area's name and what the file holds.
    pub fn files(&self) -> &'static [(&'static str, &'static str)] {
        self.files
    }

    /// What the message `id` says, without arguments; the id itself when
    /// there is no such message or it needs arguments.
    pub fn text(&'static self, id: &'static str) -> &'static str {
        self.loaded()
            .plain
            .get(id)
            .map(|text| &**text)
            .unwrap_or(id)
    }

    /// What the message `id` says with these arguments; the id itself
    /// when there is no such message.
    pub fn format(&'static self, id: &str, args: &[(&str, Arg<'_>)]) -> String {
        let loaded = self.loaded();
        let Some(pattern) = loaded
            .bundle
            .get_message(id)
            .and_then(|message| message.value())
        else {
            return id.to_owned();
        };
        let mut given = FluentArgs::with_capacity(args.len());
        for (name, value) in args {
            given.set(*name, value.0.clone());
        }
        let mut errors = Vec::new();
        loaded
            .bundle
            .format_pattern(pattern, Some(&given), &mut errors)
            .into_owned()
    }

    /// Whether there is a message `id`.
    pub fn has(&'static self, id: &str) -> bool {
        self.loaded().bundle.has_message(id)
    }

    /// What went wrong in reading the files: a line for each fault, empty
    /// when there was none.
    pub fn problems(&'static self) -> &'static [String] {
        &self.loaded().problems
    }

    fn loaded(&'static self) -> &'static Loaded {
        self.loaded.get_or_init(|| load(self.files))
    }
}

/// The line a byte of a file is on, from one.
fn line_of(source: &str, at: usize) -> usize {
    source
        .get(..at)
        .map(|before| before.matches('\n').count())
        .unwrap_or(0)
        .saturating_add(1)
}

fn load(files: &[(&str, &str)]) -> Loaded {
    let language = LANGUAGE.parse().unwrap_or_default();
    let mut bundle = FluentBundle::new_concurrent(vec![language]);
    // The bidirectional isolation marks Fluent puts around an argument
    // are for text that mixes directions; English has none, and the
    // marks would be characters in what is copied and compared.
    bundle.set_use_isolating(false);
    let mut problems = Vec::new();
    let mut ids = Vec::new();
    let sources = std::iter::once(("common", COMMON)).chain(files.iter().copied());
    for (area, source) in sources {
        let resource = match FluentResource::try_new(source.to_owned()) {
            Ok(resource) => resource,
            Err((resource, errors)) => {
                for error in errors {
                    problems.push(format!(
                        "{area}.ftl:{}: {:?}",
                        line_of(source, error.pos.start),
                        error.kind
                    ));
                }
                resource
            }
        };
        for entry in resource.entries() {
            if let fluent_syntax::ast::Entry::Message(message) = entry {
                ids.push(message.id.name.to_owned());
            }
        }
        if let Err(errors) = bundle.add_resource(resource) {
            for error in errors {
                problems.push(format!("{area}.ftl: {error}"));
            }
        }
    }
    let mut plain = HashMap::with_capacity(ids.len());
    for id in ids {
        let Some(pattern) = bundle.get_message(&id).and_then(|message| message.value()) else {
            continue;
        };
        let mut errors = Vec::new();
        let text = bundle.format_pattern(pattern, None, &mut errors);
        if errors.is_empty() {
            let text: Box<str> = text.into();
            plain.insert(id, text);
        }
    }
    Loaded {
        bundle,
        plain,
        problems,
    }
}

/// The catalog of this crate's own tests: [`say!`] looks in the calling
/// crate's `strings` module.
#[cfg(test)]
mod strings {
    static FILES: &[(&str, &str)] = &[(
        "test",
        "test-plain = Fetch\n\
         test-product = About { -product }\n\
         test-count = { $count ->\n    [0] nothing\n    [one] one commit\n   *[other] { $count } commits\n}\n\
         test-named = { $name } is { $state }\n\
         test-spaces = {\" · \"}\n\
         test-lines =\n    first line\n    second line\n\
         test-blank =\n    one\n\n    two\n      indented\n\
         test-braces = {\"{\"}path{\"}\"} and {\"[\"}x]\n",
    )];

    pub static CATALOG: crate::Catalog = crate::Catalog::new("test", FILES);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_messages_are_handed_out_as_they_are() {
        assert_eq!(say!("test-plain"), "Fetch");
        assert_eq!(say!("test-product"), "About BitBraid");
        assert_eq!(say!("test-spaces"), " · ");
        assert_eq!(say!("test-lines"), "first line\nsecond line");
        assert_eq!(say!("test-blank"), "one\n\ntwo\n  indented");
        assert_eq!(say!("test-braces"), "{path} and [x]");
        assert!(strings::CATALOG.problems().is_empty());
    }

    #[test]
    fn plurals_select_on_the_number() {
        assert_eq!(say!("test-count", count = 0usize), "nothing");
        assert_eq!(say!("test-count", count = 1usize), "one commit");
        assert_eq!(say!("test-count", count = 1234567u64), "1234567 commits");
        let count = 2i32;
        assert_eq!(say!("test-count", count), "2 commits");
        // Past what a float holds exactly, the digits are kept.
        assert_eq!(
            say!("test-count", count = usize::MAX),
            format!("{} commits", usize::MAX)
        );
        assert_eq!(
            say!("test-count", count = i64::MIN),
            format!("{} commits", i64::MIN)
        );
        assert_eq!(
            say!("test-count", count = 1u64 << 53),
            format!("{} commits", 1u64 << 53)
        );
    }

    #[test]
    fn arguments_are_put_in_without_marks() {
        let state = String::from("open");
        assert_eq!(
            say!("test-named", name = "main", state = &state),
            "main is open"
        );
    }

    #[test]
    fn what_is_not_in_the_catalog_is_shown_as_its_id() {
        assert_eq!(say!("test-missing"), "test-missing");
        assert_eq!(say!("test-missing", count = 1usize), "test-missing");
        assert_eq!(say!("test-count"), "test-count");
    }

    #[test]
    fn a_message_named_in_a_table_is_looked_up_later() {
        static NAMED: Message = message!("test-plain");
        assert_eq!(NAMED.text(), "Fetch");
        assert_eq!(NAMED.to_string(), "Fetch");
        assert_eq!(NAMED.id(), "test-plain");
    }

    #[test]
    fn a_file_that_does_not_parse_is_said_and_the_rest_still_read() {
        static BROKEN: Catalog = Catalog::new(
            "broken",
            &[(
                "broken",
                "broken-good = Good\nbroken bad\nbroken-good = Twice\n",
            )],
        );
        assert_eq!(BROKEN.text("broken-good"), "Good");
        let problems = BROKEN.problems();
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(problems[0].starts_with("broken.ftl:2:"), "{problems:?}");
    }
}
