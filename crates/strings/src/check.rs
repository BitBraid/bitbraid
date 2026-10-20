//! What the crates' tests hold their catalogs and their code to.
//!
//! * [`catalog`]: the catalog's files parse, each id starts with its
//!   file's area, every [`say!`](crate::say) and
//!   [`message!`](crate::message) in the crate's code names a message that
//!   is there and gives it the arguments it takes, no more and no fewer,
//!   and every message is looked up by code that is not a test.
//! * [`literals`]: no sentence is written into the code that is not a
//!   test, and nothing with a letter in it is handed to a call that draws
//!   it, but for the names a list allows (a font, a key).
//!
//! The code is read as Rust tokens: comments and the insides of strings
//! cannot be taken for code, and a `#[cfg(test)]` item, a test, is left
//! out where that matters.
//!
//! Sources: The Rust Reference, "Tokens" (string, raw string, byte and
//! character literals, lifetimes) and "Comments"; Project Fluent's syntax
//! guide (variables, selectors, message references).

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    path::{Path, PathBuf},
};

use fluent_syntax::ast;

use crate::Catalog;

/// A token of Rust source, as far as these checks need one.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Kind {
    /// A name or a keyword.
    Ident,
    /// A string literal of any form; the text is what it holds.
    Str,
    /// One character of punctuation; brackets among them.
    Punct,
    /// A number, a character literal, a lifetime.
    Other,
    /// A comment that says the literals of its line are not shown.
    NotShown,
}

/// How a comment begins that says the literals of its line are not shown
/// to anyone.
pub const NOT_SHOWN: &str = "not shown";

#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    text: String,
    line: usize,
    /// It is inside an item that is only built for tests.
    test: bool,
}

impl Token {
    fn is(&self, kind: Kind, text: &str) -> bool {
        self.kind == kind && self.text == text
    }

    fn punct(&self, c: char) -> bool {
        self.kind == Kind::Punct && self.text.chars().eq(std::iter::once(c))
    }
}

/// What a string literal holds, its escapes resolved.
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('0') => out.push('\0'),
            Some('u') => {
                let mut hex = String::new();
                if chars.peek() == Some(&'{') {
                    chars.next();
                    for c in chars.by_ref() {
                        if c == '}' {
                            break;
                        }
                        hex.push(c);
                    }
                }
                let code = u32::from_str_radix(&hex.replace('_', ""), 16).ok();
                if let Some(c) = code.and_then(char::from_u32) {
                    out.push(c);
                }
            }
            Some('x') => {
                let hex: String = chars.by_ref().take(2).collect();
                let code = u32::from_str_radix(&hex, 16).ok();
                if let Some(c) = code.and_then(char::from_u32) {
                    out.push(c);
                }
            }
            // A line continued: the break and the blanks after it go.
            Some('\n') => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// The tokens of a file.
fn tokens(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut at = 0usize;
    let mut line = 1usize;
    let get = |at: usize| chars.get(at).copied();
    let push = |out: &mut Vec<Token>, kind: Kind, text: String, line: usize| {
        out.push(Token {
            kind,
            text,
            line,
            test: false,
        });
    };
    while let Some(c) = get(at) {
        if c == '\n' {
            line += 1;
            at += 1;
            continue;
        }
        if c.is_whitespace() {
            at += 1;
            continue;
        }
        // Comments. One that begins `// not shown` says that literals near
        // it are not shown in the window (see `literals_in`): a program
        // reads them (a measuring run's output), they are written into a
        // file of a format of its own, or they are what the engine says,
        // kept to be compared with.
        if c == '/' && get(at + 1) == Some('/') {
            let start = at;
            while get(at).is_some_and(|c| c != '\n') {
                at += 1;
            }
            let comment: String = chars
                .get(start.saturating_add(2)..at)
                .map(|text| text.iter().collect())
                .unwrap_or_default();
            if comment.trim_start().starts_with(NOT_SHOWN) {
                push(&mut out, Kind::NotShown, String::new(), line);
            }
            continue;
        }
        if c == '/' && get(at + 1) == Some('*') {
            let mut depth = 0usize;
            while let Some(c) = get(at) {
                if c == '/' && get(at + 1) == Some('*') {
                    depth += 1;
                    at += 2;
                } else if c == '*' && get(at + 1) == Some('/') {
                    depth = depth.saturating_sub(1);
                    at += 2;
                    if depth == 0 {
                        break;
                    }
                } else {
                    if c == '\n' {
                        line += 1;
                    }
                    at += 1;
                }
            }
            continue;
        }
        // Raw strings: r"…", r#"…"#, br"…", cr"…".
        let raw_start = match (c, get(at + 1), get(at + 2)) {
            ('r', Some('"' | '#'), _) => Some(at + 1),
            ('b' | 'c', Some('r'), Some('"' | '#')) => Some(at + 2),
            _ => None,
        };
        if let Some(mut from) = raw_start {
            let mut hashes = 0usize;
            while get(from) == Some('#') {
                hashes += 1;
                from += 1;
            }
            if get(from) == Some('"') {
                let start_line = line;
                let mut end = from + 1;
                let mut text = String::new();
                loop {
                    match get(end) {
                        None => break,
                        Some('"') if (1..=hashes).all(|n| get(end + n) == Some('#')) => {
                            end += 1 + hashes;
                            break;
                        }
                        Some(c) => {
                            if c == '\n' {
                                line += 1;
                            }
                            text.push(c);
                            end += 1;
                        }
                    }
                }
                push(&mut out, Kind::Str, text, start_line);
                at = end;
                continue;
            }
            // `r#name`: a raw identifier, read below as a name.
        }
        // Strings: "…", b"…", c"…".
        let quote_at = match (c, get(at + 1)) {
            ('"', _) => Some(at),
            ('b' | 'c', Some('"')) => Some(at + 1),
            _ => None,
        };
        if let Some(open) = quote_at {
            let start_line = line;
            let mut end = open + 1;
            let mut raw = String::new();
            while let Some(c) = get(end) {
                match c {
                    '\\' => {
                        raw.push(c);
                        if let Some(next) = get(end + 1) {
                            if next == '\n' {
                                line += 1;
                            }
                            raw.push(next);
                        }
                        end += 2;
                    }
                    '"' => {
                        end += 1;
                        break;
                    }
                    _ => {
                        if c == '\n' {
                            line += 1;
                        }
                        raw.push(c);
                        end += 1;
                    }
                }
            }
            push(&mut out, Kind::Str, unescape(&raw), start_line);
            at = end;
            continue;
        }
        // Character literals and lifetimes.
        if c == '\'' {
            if get(at + 1) == Some('\\') {
                let mut end = at + 2;
                while get(end).is_some_and(|c| c != '\'') {
                    end += 1;
                }
                push(&mut out, Kind::Other, String::new(), line);
                at = end + 1;
                continue;
            }
            if get(at + 2) == Some('\'') {
                push(&mut out, Kind::Other, String::new(), line);
                at += 3;
                continue;
            }
            // A lifetime or a label: the quote, then a name.
            push(&mut out, Kind::Other, String::new(), line);
            at += 1;
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let mut end = at;
            if c == 'r' && get(at + 1) == Some('#') {
                end += 2;
            }
            let mut text = String::new();
            while let Some(c) = get(end).filter(|c| c.is_alphanumeric() || *c == '_') {
                text.push(c);
                end += 1;
            }
            push(&mut out, Kind::Ident, text, line);
            at = end;
            continue;
        }
        if c.is_ascii_digit() {
            let mut end = at;
            while let Some(c) = get(end) {
                let more = c.is_alphanumeric()
                    || c == '_'
                    || (c == '.' && get(end + 1).is_some_and(|c| c.is_ascii_digit()));
                if !more {
                    break;
                }
                end += 1;
            }
            push(&mut out, Kind::Other, String::new(), line);
            at = end;
            continue;
        }
        push(&mut out, Kind::Punct, c.to_string(), line);
        at += 1;
    }
    mark_tests(&mut out);
    out
}

/// The index past the bracket that closes the one at `open`.
fn past_close(tokens: &[Token], open: usize) -> usize {
    let mut depth = 0usize;
    let mut at = open;
    while let Some(token) = tokens.get(at) {
        if token.kind == Kind::Punct {
            match token.text.as_str() {
                "(" | "[" | "{" => depth += 1,
                ")" | "]" | "}" => {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return at + 1;
                    }
                }
                _ => {}
            }
        }
        at += 1;
    }
    at
}

/// Whether the attribute that starts at `at` (`#`) says the item after it
/// is a test: `#[test]`, or a `cfg` that names `test` and not `not`.
fn test_attribute(tokens: &[Token], at: usize) -> Option<usize> {
    let hash = tokens.get(at)?;
    let open = tokens.get(at + 1)?;
    if !hash.punct('#') || !open.punct('[') {
        return None;
    }
    let end = past_close(tokens, at + 1);
    let inside = tokens.get(at + 2..end.saturating_sub(1))?;
    let first = inside.first()?;
    let test = if first.is(Kind::Ident, "test") && inside.len() == 1 {
        true
    } else {
        first.is(Kind::Ident, "cfg")
            && inside.iter().any(|token| token.is(Kind::Ident, "test"))
            && !inside.iter().any(|token| token.is(Kind::Ident, "not"))
    };
    test.then_some(end)
}

/// Marks the tokens of every item that is built only for tests.
fn mark_tests(tokens: &mut [Token]) {
    let mut at = 0usize;
    while at < tokens.len() {
        let Some(mut end) = test_attribute(tokens, at) else {
            at += 1;
            continue;
        };
        // Further attributes of the same item.
        while tokens.get(end).is_some_and(|token| token.punct('#'))
            && tokens.get(end + 1).is_some_and(|token| token.punct('['))
        {
            end = past_close(tokens, end + 1);
        }
        // The item: up to its block's end, or its `;`.
        let mut item_end = end;
        while let Some(token) = tokens.get(item_end) {
            if token.punct(';') {
                item_end += 1;
                break;
            }
            if token.punct('{') {
                item_end = past_close(tokens, item_end);
                break;
            }
            if token.punct('(') || token.punct('[') {
                item_end = past_close(tokens, item_end);
                continue;
            }
            item_end += 1;
        }
        for token in tokens.get_mut(at..item_end).into_iter().flatten() {
            token.test = true;
        }
        at = item_end.max(at + 1);
    }
}

/// The `.rs` files under a folder, in order.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// A lookup of a message in the code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    /// The file, relative to the crate.
    pub file: String,
    /// Its line, from one.
    pub line: usize,
    /// The message's id.
    pub id: String,
    /// The names of the arguments given; `None` for [`message!`](crate::message).
    pub args: Option<BTreeSet<String>>,
    /// It is in a test.
    pub test: bool,
}

/// Every [`say!`](crate::say) and [`message!`](crate::message) in the
/// files under `src` of the crate at `crate_dir`.
pub fn uses(crate_dir: &Path) -> Vec<Use> {
    let mut out = Vec::new();
    for path in rust_files(&crate_dir.join("src")) {
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let file = path
            .strip_prefix(crate_dir)
            .unwrap_or(&path)
            .display()
            .to_string();
        out.extend(uses_in(&file, &source));
    }
    out
}

fn uses_in(file: &str, source: &str) -> Vec<Use> {
    let tokens = tokens(source);
    let mut out = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        let kind = match token.text.as_str() {
            "say" if token.kind == Kind::Ident => true,
            "message" if token.kind == Kind::Ident => false,
            _ => continue,
        };
        let is_macro = tokens.get(at + 1).is_some_and(|t| t.punct('!'))
            && tokens.get(at + 2).is_some_and(|t| t.punct('('));
        if !is_macro {
            continue;
        }
        // `macro_rules! say`: the definition, not a use.
        if at > 0 && tokens.get(at - 1).is_some_and(|t| t.punct('!')) {
            continue;
        }
        let Some(id) = tokens.get(at + 3).filter(|t| t.kind == Kind::Str) else {
            continue;
        };
        let end = past_close(&tokens, at + 2).saturating_sub(1);
        let args = kind.then(|| arguments(&tokens, at + 4, end));
        out.push(Use {
            file: file.to_owned(),
            line: token.line,
            id: id.text.clone(),
            args,
            test: token.test,
        });
    }
    out
}

/// The names of the arguments between `from` and `end`: `, name = value`
/// or `, name`, split at the commas outside brackets.
fn arguments(tokens: &[Token], from: usize, end: usize) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    let mut at = from;
    while at < end {
        let Some(token) = tokens.get(at) else {
            break;
        };
        if token.punct(',') {
            if let Some(name) = tokens.get(at + 1).filter(|t| t.kind == Kind::Ident) {
                let next = tokens.get(at + 2);
                let named = next.is_some_and(|t| t.punct('='))
                    && !tokens.get(at + 3).is_some_and(|t| t.punct('='));
                let alone = next.is_some_and(|t| t.punct(',')) || at + 2 == end;
                if named || alone {
                    names.insert(name.text.clone());
                }
            }
            at += 1;
            continue;
        }
        if token.punct('(') || token.punct('[') || token.punct('{') {
            at = past_close(tokens, at);
            continue;
        }
        at += 1;
    }
    names
}

/// A message of a catalog, as the checks see it.
#[derive(Debug, Clone, Default)]
struct Entry {
    area: String,
    line: usize,
    has_value: bool,
    attributes: usize,
    variables: BTreeSet<String>,
    references: BTreeSet<String>,
}

fn walk_pattern(pattern: &ast::Pattern<&str>, entry: &mut Entry) {
    for element in &pattern.elements {
        if let ast::PatternElement::Placeable { expression } = element {
            walk_expression(expression, entry);
        }
    }
}

fn walk_expression(expression: &ast::Expression<&str>, entry: &mut Entry) {
    match expression {
        ast::Expression::Select { selector, variants } => {
            walk_inline(selector, entry);
            for variant in variants {
                walk_pattern(&variant.value, entry);
            }
        }
        ast::Expression::Inline(inline) => walk_inline(inline, entry),
    }
}

fn walk_inline(inline: &ast::InlineExpression<&str>, entry: &mut Entry) {
    match inline {
        ast::InlineExpression::VariableReference { id } => {
            entry.variables.insert(id.name.to_owned());
        }
        ast::InlineExpression::MessageReference { id, .. } => {
            entry.references.insert(id.name.to_owned());
        }
        ast::InlineExpression::FunctionReference { arguments, .. } => {
            walk_arguments(arguments, entry);
        }
        ast::InlineExpression::TermReference { arguments, .. } => {
            if let Some(arguments) = arguments {
                walk_arguments(arguments, entry);
            }
        }
        ast::InlineExpression::Placeable { expression } => walk_expression(expression, entry),
        ast::InlineExpression::StringLiteral { .. }
        | ast::InlineExpression::NumberLiteral { .. } => {}
    }
}

fn walk_arguments(arguments: &ast::CallArguments<&str>, entry: &mut Entry) {
    for positional in &arguments.positional {
        walk_inline(positional, entry);
    }
    for named in &arguments.named {
        walk_inline(&named.value, entry);
    }
}

/// The messages of a catalog's files, and what is wrong with the files.
fn entries(catalog: &Catalog) -> (BTreeMap<String, Entry>, Vec<String>) {
    let mut problems = Vec::new();
    let mut out: BTreeMap<String, Entry> = BTreeMap::new();
    for (area, source) in catalog.files() {
        let resource = match fluent_syntax::parser::parse(*source) {
            Ok(resource) => resource,
            Err((resource, errors)) => {
                for error in errors {
                    problems.push(format!(
                        "strings/en/{area}.ftl:{}: does not parse: {:?}",
                        crate::line_of(source, error.pos.start),
                        error.kind
                    ));
                }
                resource
            }
        };
        let prefix = format!("{area}-");
        for item in &resource.body {
            match item {
                ast::Entry::Message(message) => {
                    let id = message.id.name.to_owned();
                    let line = source
                        .find(&format!("\n{id} ="))
                        .map(|at| crate::line_of(source, at + 1))
                        .or_else(|| source.starts_with(&format!("{id} =")).then_some(1))
                        .unwrap_or(0);
                    if !id.starts_with(&prefix) {
                        problems.push(format!(
                            "strings/en/{area}.ftl:{line}: {id} does not start with {prefix}"
                        ));
                    }
                    let mut entry = Entry {
                        area: (*area).to_owned(),
                        line,
                        has_value: message.value.is_some(),
                        attributes: message.attributes.len(),
                        ..Entry::default()
                    };
                    if let Some(value) = &message.value {
                        walk_pattern(value, &mut entry);
                    }
                    if entry.attributes > 0 {
                        problems.push(format!(
                            "strings/en/{area}.ftl:{line}: {id} has attributes, which are never looked up; make each a message of its own"
                        ));
                    }
                    if !entry.has_value {
                        problems.push(format!("strings/en/{area}.ftl:{line}: {id} has no value"));
                    }
                    if out.insert(id.clone(), entry).is_some() {
                        problems.push(format!(
                            "strings/en/{area}.ftl:{line}: {id} is in the catalog twice"
                        ));
                    }
                }
                ast::Entry::Junk { content } => problems.push(format!(
                    "strings/en/{area}.ftl: cannot be read: {}",
                    content.lines().next().unwrap_or_default()
                )),
                _ => {}
            }
        }
    }
    // A message that puts in another takes that one's arguments too.
    let direct: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = out
        .iter()
        .map(|(id, entry)| {
            (
                id.clone(),
                (entry.variables.clone(), entry.references.clone()),
            )
        })
        .collect();
    for (id, entry) in &mut out {
        let mut seen = BTreeSet::from([id.clone()]);
        let mut pending: Vec<String> = entry.references.iter().cloned().collect();
        while let Some(next) = pending.pop() {
            if !seen.insert(next.clone()) {
                continue;
            }
            match direct.get(&next) {
                Some((variables, references)) => {
                    entry.variables.extend(variables.iter().cloned());
                    pending.extend(references.iter().cloned());
                }
                None => problems.push(format!(
                    "strings/en/{}.ftl:{}: {id} puts in {next}, which is not in the catalog",
                    entry.area, entry.line
                )),
            }
        }
    }
    (out, problems)
}

/// What is wrong with a crate's catalog and its lookups: a line for each
/// fault, empty when there is none. `crate_dir` is the crate's folder
/// (`env!("CARGO_MANIFEST_DIR")`).
pub fn catalog(catalog: &'static Catalog, crate_dir: &str) -> Vec<String> {
    let crate_dir = Path::new(crate_dir);
    let mut problems: Vec<String> = catalog.problems().to_vec();
    let (entries, found) = entries(catalog);
    problems.extend(found);
    let uses = uses(crate_dir);
    let mut shown: BTreeSet<&str> = BTreeSet::new();
    for found in &uses {
        let at = format!("{}:{}", found.file, found.line);
        if !found.test {
            shown.insert(&found.id);
        }
        let Some(entry) = entries.get(&found.id) else {
            problems.push(format!("{at}: {} is not in the catalog", found.id));
            continue;
        };
        match &found.args {
            None if !entry.variables.is_empty() => problems.push(format!(
                "{at}: message!({}) cannot take arguments, and the message takes {}",
                found.id,
                listed(&entry.variables)
            )),
            Some(args) if args != &entry.variables => {
                let mut said = format!("{at}: {} ", found.id);
                let missing: BTreeSet<String> = entry.variables.difference(args).cloned().collect();
                let extra: BTreeSet<String> = args.difference(&entry.variables).cloned().collect();
                if !missing.is_empty() {
                    let _ = write!(said, "takes {} and is not given it", listed(&missing));
                }
                if !extra.is_empty() {
                    if !missing.is_empty() {
                        said.push_str("; ");
                    }
                    let _ = write!(said, "is given {}, which it does not take", listed(&extra));
                }
                problems.push(said);
            }
            _ => {}
        }
    }
    for (id, entry) in &entries {
        if !shown.contains(id.as_str()) {
            problems.push(format!(
                "strings/en/{}.ftl:{}: {id} is looked up nowhere but in tests",
                entry.area, entry.line
            ));
        }
    }
    problems
}

fn listed(names: &BTreeSet<String>) -> String {
    names
        .iter()
        .map(|name| format!("${name}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a crate's code is held to by [`literals`].
#[derive(Debug, Clone, Copy)]
pub struct Rules<'a> {
    /// Calls whose first argument is drawn as text: a literal with a
    /// letter in it may not be handed to them (`child`, `label`).
    pub drawn: &'a [&'a str],
    /// Calls whose literal arguments are never shown (git's arguments, a
    /// key's context, a field of an answer): a sentence there is not said
    /// to anyone.
    pub unseen: &'a [&'a str],
    /// Literals that may stand anywhere: names that are not words of a
    /// language (a font, a key on the keyboard).
    pub allowed: &'a [&'a str],
}

/// Whether a literal reads as words of a sentence or a label: a word of
/// two letters or more, and a blank between words or a capital to begin
/// with. A name in one piece (`GraphView`, `rerere.records`, `HEAD`,
/// `--force`) does not.
pub fn reads_as_words(text: &str) -> bool {
    // What `format!` puts in is not a word of the literal.
    let mut plain = String::with_capacity(text.len());
    let mut depth = 0usize;
    for c in text.chars() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth == 0 => plain.push(c),
            _ => {}
        }
    }
    let trimmed = plain.trim();
    let words: Vec<&str> = trimmed
        .split(|c: char| !c.is_alphabetic() && c != '\'')
        .filter(|word| word.chars().filter(|c| c.is_alphabetic()).count() >= 2)
        .collect();
    let word = words.iter().any(|word| {
        let mut chars = word.chars().filter(|c| c.is_alphabetic());
        let first = chars.next();
        let rest_lower = chars.all(|c| c.is_lowercase());
        rest_lower && first.is_some()
    });
    if !word {
        return false;
    }
    // A blank between words, or between a word and what is put in.
    let blank = text.trim().chars().any(char::is_whitespace);
    if blank {
        return true;
    }
    // One piece: a capitalised word is a label (`Fetch`, `Fetch…`), a
    // name made of several (`GraphView`) or with marks inside
    // (`Cargo.toml`, `some_name`) is not.
    let mut chars = trimmed.chars();
    let capital = chars.next().is_some_and(char::is_uppercase)
        && chars.next().is_some_and(char::is_lowercase);
    let inner: &str = trimmed.trim_end_matches(|c: char| !c.is_alphanumeric());
    let one_word = inner.chars().skip(1).all(char::is_lowercase);
    capital && one_word
}

/// Every literal in the code of the crate at `crate_dir` that is not a
/// test and that says something to a person: a sentence or a label
/// anywhere, or anything with a letter handed to a call that draws it.
/// A line for each, empty when there is none.
pub fn literals(crate_dir: &str, rules: &Rules<'_>) -> Vec<String> {
    let crate_dir = Path::new(crate_dir);
    let mut found = Vec::new();
    for path in rust_files(&crate_dir.join("src")) {
        let Ok(source) = std::fs::read_to_string(&path) else {
            continue;
        };
        let file = path
            .strip_prefix(crate_dir)
            .unwrap_or(&path)
            .display()
            .to_string();
        found.extend(literals_in(&file, &source, rules));
    }
    found
}

fn literals_in(file: &str, source: &str, rules: &Rules<'_>) -> Vec<String> {
    let tokens = tokens(source);
    let mut found = Vec::new();
    // A `// not shown` comment after code speaks for the literals of its
    // line; one on a line of its own speaks for the statement, field or
    // arm that follows it, up to its `;` or `,`.
    let trailing: BTreeSet<usize> = tokens
        .iter()
        .enumerate()
        .filter(|(at, token)| {
            token.kind == Kind::NotShown
                && at
                    .checked_sub(1)
                    .and_then(|at| tokens.get(at))
                    .is_some_and(|before| before.line == token.line)
        })
        .map(|(_, token)| token.line)
        .collect();
    // The depth of brackets a standalone comment was met at, while the
    // statement after it lasts.
    let mut covering: Option<usize> = None;
    // The call each open bracket belongs to: the name before a `(`, after
    // a `!` for a macro.
    let mut calls: Vec<(char, Option<String>)> = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        if token.kind == Kind::NotShown && !trailing.contains(&token.line) {
            covering = Some(calls.len());
            continue;
        }
        if token.kind == Kind::Punct {
            if let Some(depth) = covering {
                let ends = match token.text.as_str() {
                    ";" | "," => calls.len() <= depth,
                    ")" | "]" | "}" => calls.len() <= depth,
                    _ => false,
                };
                if ends {
                    covering = None;
                }
            }
            match token.text.as_str() {
                "(" | "[" | "{" => {
                    let before = at.checked_sub(1).and_then(|at| tokens.get(at));
                    let callee = match before {
                        Some(name) if name.kind == Kind::Ident => Some(name.text.clone()),
                        Some(bang) if bang.punct('!') => at
                            .checked_sub(2)
                            .and_then(|at| tokens.get(at))
                            .filter(|name| name.kind == Kind::Ident)
                            .map(|name| name.text.clone()),
                        _ => None,
                    };
                    let open = token.text.chars().next().unwrap_or('(');
                    calls.push((open, callee));
                }
                ")" | "]" | "}" => {
                    calls.pop();
                }
                _ => {}
            }
            continue;
        }
        if token.kind != Kind::Str || token.test {
            continue;
        }
        if rules.allowed.contains(&token.text.as_str())
            || trailing.contains(&token.line)
            || covering.is_some()
        {
            continue;
        }
        // Inside a call nobody sees the words of, at any depth.
        let unseen = calls.iter().any(|(open, callee)| {
            *open == '('
                && callee
                    .as_deref()
                    .is_some_and(|callee| rules.unseen.contains(&callee))
        });
        if unseen {
            continue;
        }
        let callee = calls
            .iter()
            .rev()
            .find(|(open, _)| *open == '(')
            .and_then(|(_, callee)| callee.as_deref());
        let first_argument = at
            .checked_sub(1)
            .and_then(|at| tokens.get(at))
            .is_some_and(|before| before.punct('('));
        let drawn = first_argument
            && callee.is_some_and(|callee| rules.drawn.contains(&callee))
            && token.text.chars().any(char::is_alphabetic);
        if drawn || reads_as_words(&token.text) {
            found.push(format!(
                "{file}:{}: {:?} is written into the code; put it in the catalog",
                token.line, token.text
            ));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strings_comments_and_characters_are_told_apart() {
        let source = r####"
            // say!("in-a-comment")
            /* say!("in /* a nested */ block") */
            let a = 'x'; let b = '\''; fn f<'a>(x: &'a str) {}
            let c = r#"a "raw" string"#;
            let d = "an \"escaped\" one \u{b7}";
        "####;
        let found: Vec<String> = tokens(source)
            .into_iter()
            .filter(|token| token.kind == Kind::Str)
            .map(|token| token.text)
            .collect();
        assert_eq!(found, vec!["a \"raw\" string", "an \"escaped\" one \u{b7}"]);
    }

    #[test]
    fn uses_are_found_with_their_arguments() {
        let source = r#"
            fn view() {
                let a = say!("area-plain");
                let b = say!("area-counted", count = rows.len(), name);
                let c = say!("area-nested", path = join(a, b), equal = x == y,);
                static M: Message = message!("area-named");
            }
            #[cfg(test)]
            mod tests {
                fn t() { say!("area-tested"); }
            }
        "#;
        let found = uses_in("x.rs", source);
        let ids: Vec<(&str, bool)> = found.iter().map(|u| (u.id.as_str(), u.test)).collect();
        assert_eq!(
            ids,
            vec![
                ("area-plain", false),
                ("area-counted", false),
                ("area-nested", false),
                ("area-named", false),
                ("area-tested", true),
            ]
        );
        let names = |at: usize| {
            found[at]
                .args
                .clone()
                .map(|args| args.into_iter().collect::<Vec<_>>())
        };
        assert_eq!(names(0), Some(vec![]));
        assert_eq!(names(1), Some(vec!["count".to_owned(), "name".to_owned()]));
        assert_eq!(names(2), Some(vec!["equal".to_owned(), "path".to_owned()]));
        assert_eq!(names(3), None);
    }

    #[test]
    fn words_are_told_from_names() {
        for words in [
            "Fetch",
            "Fetch…",
            "Open a repository",
            "{count} files",
            "nothing to undo",
            "Cannot open {}: {err}",
        ] {
            assert!(reads_as_words(words), "{words}");
        }
        for name in [
            "fetch",
            "GraphView",
            "rerere.records",
            "HEAD",
            "--force",
            "Cargo.toml",
            " \u{b7} ",
            "{}",
            "ctrl-shift-p",
            "%H %P",
            "!Input",
        ] {
            assert!(!reads_as_words(name), "{name}");
        }
    }

    #[test]
    fn literals_outside_tests_and_unseen_calls_are_found() {
        let source = r#"
            fn view() {
                div().child("Fetch all").child(say!("area-ok"));
                Button::new("fetch").label("fetch");
                git.args(["log", "--format=%H %s", "Some words"]);
                let x = "GraphView";
                let note = format!(" rss used={}", x); // not shown: a script reads it
                // not shown: written into a file
                out.push_str("A file of your own, line one\n");
                // not shown: names it is kept under
                let names = [
                    "Some name",
                    "Other name",
                ];
                let after = "Seen again";
                debug(|| format!("built {} rows", n));
            }
            #[cfg(test)]
            mod tests { fn t() { assert_eq!(x, "Some words"); } }
        "#;
        let rules = Rules {
            drawn: &["child", "label"],
            unseen: &["args", "debug"],
            allowed: &[],
        };
        let found = literals_in("v.rs", source, &rules);
        assert_eq!(found.len(), 3, "{found:?}");
        assert!(found[0].contains("\"Fetch all\""));
        assert!(found[1].contains("\"fetch\""));
        // The comment spoke for the statement after it, and no further.
        assert!(found[2].contains("\"Seen again\""));
    }
}
