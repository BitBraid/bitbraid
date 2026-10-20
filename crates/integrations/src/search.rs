//! The words a user types to narrow a list of pull requests or issues:
//! `author:alice -label:wip,blocked "two words"`.
//!
//! A query is read into [`Term`]s. A term is a word or a quoted phrase,
//! or `key:value`; a `-` before a key leaves out what it names; values
//! after a key are split at commas outside quotes, and a value may be
//! quoted (`label:"needs review"`). GitLab's marks may stand for keys
//! when the reader is asked to take them: `@alice` for `assignee:alice`
//! (and `@me` for `assignee:@me`), `#bug` and `~bug` for `label:bug`,
//! `$v1` and `%v1` for `milestone:v1`. `@me` as a value is the user.
//!
//! What the values of one key mean together is the reader's to say: for
//! pull requests, values split by commas are alternatives (any of them),
//! and [`expand`] makes one search of each choice; for issues they are
//! passed to the provider as it reads them.
//!
//! A query is the user's and is bounded all the same: so many terms, so
//! many values, so long each, no control characters.
//!
//! Sources: docs.github.com "Searching issues and pull requests" and
//! "Filtering and searching issues and pull requests" (qualifiers, `-` to
//! exclude, quotes); docs.gitlab.com "Searching and filtering" (the
//! `@`, `~` and `%` marks).

/// Most terms a query has.
pub const MAX_TERMS: usize = 32;
/// Most values one key is given.
pub const MAX_VALUES: usize = 8;
/// Longest value.
pub const MAX_VALUE: usize = 256;
/// Most searches one query is made into.
pub const MAX_SEARCHES: usize = 8;

/// One term of a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Term {
    /// Whether what it names is left out (`-key:value`).
    pub negated: bool,
    /// The key in lower case, or `None` for words.
    pub key: Option<String>,
    /// Its values: one for words, one or more after a key.
    pub values: Vec<String>,
}

/// One key and one value of a search, after [`expand`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Single {
    /// Whether what it names is left out.
    pub negated: bool,
    /// The key, or `None` for words.
    pub key: Option<String>,
    /// The value.
    pub value: String,
}

impl Single {
    /// As GitHub's search writes it: `-key:value`, the value quoted when
    /// it has a space or a comma in it.
    pub fn github(&self) -> String {
        let value = quoted(&self.value);
        match &self.key {
            Some(key) => format!("{}{key}:{value}", if self.negated { "-" } else { "" }),
            None => format!("{}{value}", if self.negated { "-" } else { "" }),
        }
    }
}

/// A value as a search writes it: in quotes when it has to be, with no
/// quote of its own (a quote cannot be in a value).
fn quoted(value: &str) -> String {
    let plain: String = value.chars().filter(|c| *c != '"').collect();
    if plain.is_empty() || plain.chars().any(|c| c.is_whitespace() || c == ',') {
        format!("\"{plain}\"")
    } else {
        plain
    }
}

/// Split `text` at `separator` where it is not inside quotes, taking
/// the quotes away; an empty part is kept, for the caller to refuse.
fn split_outside_quotes(text: &str, separator: impl Fn(char) -> bool) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in text.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if !quoted && separator(c) {
            parts.push(std::mem::take(&mut current));
        } else {
            current.push(c);
        }
    }
    parts.push(current);
    parts
}

/// Whether `key` may be a key: letters, digits, `-` and `_`.
fn key_like(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 32
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// Read a query into its terms; `marks` takes GitLab's marks for keys.
pub fn parse(query: &str, marks: bool) -> Result<Vec<Term>, String> {
    if query.chars().any(char::is_control) {
        return Err(say!("search-control-characters").to_owned());
    }
    let mut terms = Vec::new();
    for token in split_raw(query) {
        let (negated, rest) = match token.strip_prefix('-') {
            Some(rest) if !rest.is_empty() => (true, rest),
            _ => (false, token.as_str()),
        };
        let marked = if marks {
            match rest.chars().next() {
                Some('@') => Some("assignee"),
                Some('#' | '~') => Some("label"),
                Some('$' | '%') => Some("milestone"),
                _ => None,
            }
        } else {
            None
        };
        let term = if let Some(key) = marked {
            let value = rest.get(1..).unwrap_or_default();
            Term {
                negated,
                key: Some(key.to_owned()),
                // `@me` is the user, as `assignee:@me` is.
                values: if rest == "@me" {
                    vec!["@me".to_owned()]
                } else {
                    split_outside_quotes(value, |c| c == ',')
                },
            }
        } else {
            match rest.split_once(':') {
                Some((key, value)) if key_like(key) && !key.contains('"') => Term {
                    negated,
                    key: Some(key.to_ascii_lowercase()),
                    values: split_outside_quotes(value, |c| c == ','),
                },
                _ => Term {
                    negated,
                    key: None,
                    values: split_outside_quotes(rest, |_| false),
                },
            }
        };
        if term.values.is_empty() || term.values.iter().any(|value| value.trim().is_empty()) {
            return Err(say!("search-no-value", term = format!("{token:?}")));
        }
        if term.values.len() > MAX_VALUES {
            return Err(say!(
                "search-too-many-values",
                term = format!("{token:?}"),
                most = MAX_VALUES
            ));
        }
        if term.values.iter().any(|value| value.len() > MAX_VALUE) {
            return Err(say!("search-value-too-long", most = MAX_VALUE));
        }
        terms.push(term);
        if terms.len() > MAX_TERMS {
            return Err(say!("search-too-many-terms", most = MAX_TERMS));
        }
    }
    Ok(terms)
}

/// The tokens of a query: split at spaces outside quotes, quotes kept.
fn split_raw(query: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    for c in query.chars() {
        if c == '"' {
            quoted = !quoted;
            current.push(c);
        } else if !quoted && c.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// The searches a query of pull requests stands for. A key with several
/// values is any of them: one search for each, and for several such
/// keys one for each choice of values, at most [`MAX_SEARCHES`]. A key
/// left out with several values leaves out each of them in every search.
pub fn expand(terms: &[Term]) -> Result<Vec<Vec<Single>>, String> {
    let mut searches: Vec<Vec<Single>> = vec![Vec::new()];
    for term in terms {
        if term.negated || term.values.len() == 1 || term.key.is_none() {
            for search in &mut searches {
                for value in &term.values {
                    search.push(Single {
                        negated: term.negated,
                        key: term.key.clone(),
                        value: value.clone(),
                    });
                }
            }
            continue;
        }
        let mut next = Vec::new();
        for search in &searches {
            for value in &term.values {
                let mut one = search.clone();
                one.push(Single {
                    negated: false,
                    key: term.key.clone(),
                    value: value.clone(),
                });
                next.push(one);
                if next.len() > MAX_SEARCHES {
                    return Err(say!("search-too-many-searches", most = MAX_SEARCHES));
                }
            }
        }
        searches = next;
    }
    Ok(searches)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(negated: bool, key: Option<&str>, values: &[&str]) -> Term {
        Term {
            negated,
            key: key.map(str::to_owned),
            values: values.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn a_query_is_read_into_terms() {
        assert_eq!(
            parse(
                r#"author:alice -label:wip,"needs review" fix  "two words" Draft:true"#,
                false
            )
            .unwrap(),
            [
                term(false, Some("author"), &["alice"]),
                term(true, Some("label"), &["wip", "needs review"]),
                term(false, None, &["fix"]),
                term(false, None, &["two words"]),
                term(false, Some("draft"), &["true"]),
            ]
        );
        // A colon in a word that is no key leaves it a word.
        assert_eq!(
            parse("v1.2:rc", false).unwrap(),
            [term(false, None, &["v1.2:rc"])]
        );
        assert_eq!(parse("", false).unwrap(), []);
        assert_eq!(parse("-", false).unwrap(), [term(false, None, &["-"])]);
    }

    #[test]
    fn gitlab_marks_stand_for_keys() {
        assert_eq!(
            parse(r#"@bob #bug ~"to do" $v1 %"Release 2" -@carol words"#, true).unwrap(),
            [
                term(false, Some("assignee"), &["bob"]),
                term(false, Some("label"), &["bug"]),
                term(false, Some("label"), &["to do"]),
                term(false, Some("milestone"), &["v1"]),
                term(false, Some("milestone"), &["Release 2"]),
                term(true, Some("assignee"), &["carol"]),
                term(false, None, &["words"]),
            ]
        );
        // Without marks, they are words.
        assert_eq!(
            parse("@bob", false).unwrap(),
            [term(false, None, &["@bob"])]
        );
    }

    #[test]
    fn a_query_beyond_its_bounds_is_refused() {
        assert!(parse("label:", false).is_err());
        assert!(parse("label:a,,b", false).is_err());
        assert!(parse("a\u{7}b", false).is_err());
        assert!(parse(&"w ".repeat(MAX_TERMS + 1), false).is_err());
        assert!(parse("label:a,b,c,d,e,f,g,h,i", false).is_err());
        assert!(parse(&format!("label:{}", "x".repeat(MAX_VALUE + 1)), false).is_err());
    }

    #[test]
    fn alternatives_make_one_search_each() {
        let searches = expand(&parse("author:a,b -label:x,y label:c,d w", false).unwrap()).unwrap();
        let written: Vec<String> = searches
            .iter()
            .map(|search| {
                search
                    .iter()
                    .map(Single::github)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .collect();
        assert_eq!(
            written,
            [
                "author:a -label:x -label:y label:c w",
                "author:a -label:x -label:y label:d w",
                "author:b -label:x -label:y label:c w",
                "author:b -label:x -label:y label:d w",
            ]
        );
        assert!(expand(&parse("a:1,2,3 b:1,2,3", false).unwrap()).is_err());
        assert_eq!(expand(&[]).unwrap(), [Vec::<Single>::new()]);
        assert_eq!(
            Single {
                negated: false,
                key: Some("label".into()),
                value: "needs \"review\"".into()
            }
            .github(),
            "label:\"needs review\""
        );
    }
}
