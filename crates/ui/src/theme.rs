//! Our themes on top of gpui-kit's theme system, themes of the user's own,
//! and the graph lane palette.
//!
//! The palettes in `themes/bitbraid.json` are original. The first three
//! (dark, light, high contrast) make their lane colours from evenly spaced
//! hues; the others (Dusk, Ember, Graphite, Paper, Sage) name lanes of
//! their own, picked to go with them, and bring their own colours of code.
//!
//! A theme of the user's own is a TOML file in `themes/` in the
//! configuration directory:
//!
//! ```toml
//! name = "Harbour"
//! mode = "dark"                 # which of ours it starts from: dark or light
//!
//! lanes = ["#4f8fd9", "#d9a34f"]  # the colours of the graph's lanes, in turn
//!
//! [colors]
//! background = "#10161c"
//! "primary.background" = "#2f9e8f"
//!
//! [syntax]                      # the colours of code in the diff and in blame
//! keyword = "#c586c0"
//! comment = { color = "#6a9955", font_style = "italic" }
//! ```
//!
//! It names only what it changes; everything else is our theme of the same
//! mode. The colours of the terminal inside the window are named there
//! too, as `"terminal.background"`, `"terminal.ansi.red"` and so on
//! (`theme/terminal.rs`). The folder is looked at as the keys file is, and
//! a theme that is in use follows its file.
//!
//! Sources: gpui-kit's theme documentation (`ThemeConfig`, the names of the
//! colours, `HighlightTheme` and the names of the syntax colours).

use std::{
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui_kit::component::{
    ActiveTheme, Theme, ThemeConfig, ThemeMode, ThemeRegistry, highlighter::HighlightTheme,
    try_parse_color,
};
use gpui_kit::{App, Global, Hsla, hsla};

use crate::prefs::{Prefs, ThemeChoice};

mod terminal;
pub use terminal::{
    ANSI_NAMES, Builtin as TerminalBuiltin, TerminalColors, TerminalTheme,
    builtin as terminal_builtin,
};

const THEMES_JSON: &str = include_str!("../themes/bitbraid.json");

/// One of our themes: its name in the toolkit's registry, what menus call
/// it, the colours of its terminal, and the colours of its graph's lanes
/// in turn (none: made from evenly spaced hues).
struct Ours {
    choice: ThemeChoice,
    registered: &'static str,
    label: bitbraid_strings::Message,
    terminal: terminal::Builtin,
    lanes: &'static [&'static str],
}

/// Our themes, in the order of `ThemeChoice::BUILT_IN`. The lanes of a
/// dark theme are light enough to carry its ground as the text of a
/// label, those of a light one dark enough to carry white.
static OURS: [Ours; 8] = [
    Ours {
        choice: ThemeChoice::Dark,
        registered: "BitBraid dark", // not shown: the name it is registered under
        label: message!("theme-dark"),
        terminal: terminal::Builtin::Dark,
        lanes: &[],
    },
    Ours {
        choice: ThemeChoice::Light,
        registered: "BitBraid light", // not shown: the name it is registered under
        label: message!("theme-light"),
        terminal: terminal::Builtin::Light,
        lanes: &[],
    },
    Ours {
        choice: ThemeChoice::Contrast,
        registered: "BitBraid contrast", // not shown: the name it is registered under
        label: message!("theme-contrast"),
        terminal: terminal::Builtin::Contrast,
        lanes: &[],
    },
    Ours {
        choice: ThemeChoice::Dusk,
        registered: "BitBraid dusk", // not shown: the name it is registered under
        label: message!("theme-dusk"),
        terminal: terminal::Builtin::Dusk,
        lanes: &[
            "#ae94f6", "#70d1a9", "#ed83b4", "#6dbbe8", "#f2b17f", "#6eccd1", "#d78adb", "#d6cf6e",
        ],
    },
    Ours {
        choice: ThemeChoice::Ember,
        registered: "BitBraid ember", // not shown: the name it is registered under
        label: message!("theme-ember"),
        terminal: terminal::Builtin::Ember,
        lanes: &[
            "#eaa350", "#70b4dc", "#ee8489", "#8cbf70", "#b49ce1", "#66c4be", "#d9c564", "#dd88b9",
        ],
    },
    Ours {
        choice: ThemeChoice::Graphite,
        registered: "BitBraid graphite", // not shown: the name it is registered under
        label: message!("theme-graphite"),
        terminal: terminal::Builtin::Graphite,
        lanes: &[
            "#8ba2f9", "#f2b458", "#65c98c", "#ee7d8d", "#65cbe3", "#c98be4", "#b6da70", "#f19266",
        ],
    },
    Ours {
        choice: ThemeChoice::Paper,
        registered: "BitBraid paper", // not shown: the name it is registered under
        label: message!("theme-paper"),
        terminal: terminal::Builtin::Paper,
        lanes: &[
            "#b25630", "#296898", "#597f36", "#92417a", "#227a7c", "#9f7100", "#6953a0", "#057c59",
        ],
    },
    Ours {
        choice: ThemeChoice::Sage,
        registered: "BitBraid sage", // not shown: the name it is registered under
        label: message!("theme-sage"),
        terminal: terminal::Builtin::Sage,
        lanes: &[
            "#317a45", "#1f689d", "#ad5b33", "#8b4486", "#227a7e", "#957602", "#6355a2", "#ac4259",
        ],
    },
];

/// One of our themes; none for a theme of the user's own.
fn ours(choice: &ThemeChoice) -> Option<&'static Ours> {
    OURS.iter().find(|ours| ours.choice == *choice)
}

impl Ours {
    /// The colours of its lanes; none when they are made up.
    fn lanes(&self) -> Vec<Hsla> {
        self.lanes
            .iter()
            .filter_map(|hex| try_parse_color(hex).ok())
            .collect()
    }
}

/// Name of one of our themes, as registered.
fn builtin_name(choice: &ThemeChoice) -> Option<&'static str> {
    ours(choice).map(|ours| ours.registered)
}

/// A theme as menus name it.
pub fn label(choice: &ThemeChoice) -> String {
    match (choice, ours(choice)) {
        (ThemeChoice::User(name), _) => name.clone(),
        (_, Some(ours)) => ours.label.text().to_owned(),
        (_, None) => choice.key().to_owned(),
    }
}

/// One theme of the user's own.
#[derive(Debug, Clone)]
pub struct UserTheme {
    /// Its name.
    pub name: String,
    /// The whole theme: ours of the same mode with the file's changes.
    config: Rc<ThemeConfig>,
    /// The colours of the graph's lanes, when the file names them.
    lanes: Vec<Hsla>,
    /// The colours of the terminal the file names, by their names
    /// without `terminal.` in front.
    terminal: Vec<(String, Hsla)>,
}

/// The lane colours of the theme in use, a GPUI global: none when the
/// theme names none, and they are made from evenly spaced hues.
#[derive(Debug, Default)]
pub struct Lanes(pub Vec<Hsla>);

impl Global for Lanes {}

/// As many lane colours as a theme may name.
pub const MOST_LANES: usize = 16;

/// The themes of the user's own, a GPUI global.
#[derive(Debug, Default)]
pub struct UserThemes {
    /// The folder they are read from, when there is a place.
    pub dir: Option<PathBuf>,
    /// The themes that could be read, by name.
    pub themes: Vec<UserTheme>,
    /// What is wrong with the files that could not, or not wholly.
    pub problems: Vec<String>,
}

impl Global for UserThemes {}

/// What the folder of themes looked like: the files with their length and
/// the time they were written. Enough to tell that something changed.
pub type FolderStamp = Vec<(String, u64, Option<std::time::SystemTime>)>;

/// Look at the folder of themes.
pub fn folder_stamp(dir: &Path) -> FolderStamp {
    let mut stamp: FolderStamp = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "toml"))
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            Some((
                entry.file_name().to_string_lossy().into_owned(),
                meta.len(),
                meta.modified().ok(),
            ))
        })
        .collect();
    stamp.sort();
    stamp
}

/// Our theme of a mode, as the JSON it is kept as.
fn builtin_json(dark: bool) -> Option<serde_json::Value> {
    let all: serde_json::Value = serde_json::from_str(THEMES_JSON).ok()?;
    // not shown: the names our themes are registered under
    let wanted = if dark {
        "BitBraid dark"
    } else {
        "BitBraid light"
    };
    all.get("themes")?
        .as_array()?
        .iter()
        .find(|theme| theme.get("name").and_then(|name| name.as_str()) == Some(wanted))
        .cloned()
}

/// The names a colour can have: the ones the toolkit's theme knows.
fn known_colors() -> Vec<String> {
    serde_json::to_value(ThemeConfig::default())
        .ok()
        .and_then(|config| {
            config
                .get("colors")
                .and_then(|colors| colors.as_object())
                .map(|colors| colors.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// The colours of a `[colors]` table, with tables inside it written out
/// as dotted names: `[colors.primary] background` is `primary.background`.
fn flatten(prefix: &str, table: &toml::Table, out: &mut Vec<(String, toml::Value)>) {
    for (key, value) in table {
        let name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match value {
            toml::Value::Table(inner) => flatten(&name, inner, out),
            other => out.push((name, other.clone())),
        }
    }
}

/// The syntax colours of the toolkit's own theme of a mode, as the JSON a
/// theme holds them as: what a theme of the user's starts from.
fn builtin_highlight(dark: bool) -> Option<serde_json::Value> {
    let theme = if dark {
        HighlightTheme::default_dark()
    } else {
        HighlightTheme::default_light()
    };
    serde_json::to_value(&theme.style).ok()
}

/// The names a syntax colour can have.
fn known_syntax() -> Vec<String> {
    builtin_highlight(true)
        .and_then(|style| {
            style
                .get("syntax")
                .and_then(|syntax| syntax.as_object())
                .map(|syntax| syntax.keys().cloned().collect())
        })
        .unwrap_or_default()
}

/// A colour as a theme holds it, from what a file says; none for what is
/// no colour.
fn color_json(text: &str) -> Option<serde_json::Value> {
    if text.len() > 200 {
        return None;
    }
    serde_json::to_value(try_parse_color(text).ok()?).ok()
}

/// What a `[syntax]` table says, name by name: a colour, or a table with
/// `color`, `font_style` and `font_weight`. A table may also hold the
/// names under it: `[syntax.string] escape` is `string.escape`. What
/// cannot be right is said and left out.
fn read_syntax(
    prefix: &str,
    table: &toml::Table,
    known: &[String],
    out: &mut serde_json::Map<String, serde_json::Value>,
    skipped: &mut Vec<String>,
    depth: usize,
) {
    /// What a style is made of.
    const PARTS: [&str; 3] = ["color", "font_style", "font_weight"];
    for (key, value) in table {
        let name = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        let said = shortened(&name);
        let mut style = serde_json::Map::new();
        match value {
            toml::Value::String(text) => match color_json(text) {
                Some(color) => {
                    style.insert("color".to_owned(), color);
                }
                None => {
                    skipped.push(say!(
                        "theme-not-a-colour",
                        key = format!("syntax.{said}"),
                        value = format!("{:?}", shortened(text))
                    ));
                    continue;
                }
            },
            toml::Value::Table(inner) => {
                for (part, value) in inner {
                    match (part.as_str(), value) {
                        ("color", toml::Value::String(text)) => match color_json(text) {
                            Some(color) => {
                                style.insert("color".to_owned(), color);
                            }
                            None => skipped.push(say!(
                                "theme-not-a-colour",
                                key = format!("syntax.{said}"),
                                value = format!("{:?}", shortened(text))
                            )),
                        },
                        ("font_style", toml::Value::String(text))
                            if ["normal", "italic", "underline"].contains(&text.as_str()) =>
                        {
                            style.insert(part.clone(), serde_json::Value::String(text.clone()));
                        }
                        ("font_style", _) => {
                            skipped.push(say!("theme-font-style", key = format!("syntax.{said}")))
                        }
                        ("font_weight", toml::Value::Integer(weight))
                            if (100..=900).contains(weight) && weight % 100 == 0 =>
                        {
                            style.insert(part.clone(), serde_json::Value::from(*weight));
                        }
                        ("font_weight", _) => {
                            skipped.push(say!("theme-font-weight", key = format!("syntax.{said}")))
                        }
                        ("color", _) => skipped
                            .push(say!("theme-colour-is-text", key = format!("syntax.{said}"))),
                        _ => {}
                    }
                }
                // The names under this one, when the table holds any.
                let under: toml::Table = inner
                    .iter()
                    .filter(|(part, _)| !PARTS.contains(&part.as_str()))
                    .map(|(part, value)| (part.clone(), value.clone()))
                    .collect();
                if !under.is_empty() {
                    if depth < 4 {
                        read_syntax(&name, &under, known, out, skipped, depth.saturating_add(1));
                    } else {
                        skipped.push(say!(
                            "theme-no-syntax-colour",
                            key = format!("syntax.{said}")
                        ));
                    }
                }
                if style.is_empty() {
                    continue;
                }
            }
            _ => {
                skipped.push(say!(
                    "theme-colour-or-table",
                    key = format!("syntax.{said}")
                ));
                continue;
            }
        }
        if !known.contains(&name) {
            skipped.push(say!(
                "theme-no-syntax-colour",
                key = format!("syntax.{said}")
            ));
            continue;
        }
        out.insert(name, serde_json::Value::Object(style));
    }
}

/// A theme file read: its name, the whole theme as JSON (ours of the same
/// mode with the file's changes), and what in the file was left out
/// because it cannot be right.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadTheme {
    /// The theme's name.
    pub name: String,
    /// The theme, as the toolkit reads one.
    pub json: serde_json::Value,
    /// The colours of the graph's lanes, in turn; none when the file names
    /// none.
    pub lanes: Vec<Hsla>,
    /// The colours of the terminal the file names, by their names without
    /// `terminal.` in front.
    pub terminal: Vec<(String, Hsla)>,
    /// What was skipped, in words.
    pub skipped: Vec<String>,
}

/// Read a theme file. `file_name` names the theme when the file does not.
/// An error says why nothing of the file can be used.
pub fn read_user_theme(text: &str, file_name: &str) -> Result<ReadTheme, String> {
    let table: toml::Table = toml::from_str(text).map_err(|err| {
        err.to_string()
            .lines()
            .next()
            .unwrap_or(say!("theme-not-toml"))
            .trim()
            .to_owned()
    })?;
    let stem = file_name.strip_suffix(".toml").unwrap_or(file_name);
    let name = match table.get("name") {
        None => stem.to_owned(),
        Some(toml::Value::String(name)) if !name.trim().is_empty() => name.trim().to_owned(),
        Some(_) => return Err(say!("theme-name-not-text").to_owned()),
    };
    if name.chars().count() > 60 || name.chars().any(char::is_control) {
        return Err(say!("theme-name-unfit").to_owned());
    }
    let lower = name.to_lowercase();
    let taken = OURS
        .iter()
        .any(|ours| ours.choice.key() == lower || ours.label.text().to_lowercase() == lower);
    if taken || name.starts_with("BitBraid") {
        return Err(say!("theme-name-taken", name = format!("{name:?}")));
    }
    let dark = match table.get("mode") {
        None => true,
        Some(toml::Value::String(mode)) if mode == "dark" => true,
        Some(toml::Value::String(mode)) if mode == "light" => false,
        Some(_) => return Err(say!("theme-mode").to_owned()),
    };
    let mut json = builtin_json(dark).ok_or(say!("theme-built-in-unreadable"))?;
    let mut skipped = Vec::new();
    let mut lanes = Vec::new();
    let mut terminal_colors = Vec::new();
    let known = known_colors();
    let Some(object) = json.as_object_mut() else {
        return Err(say!("theme-built-in-unreadable").to_owned());
    };
    object.insert("name".to_owned(), serde_json::Value::String(name.clone()));
    object.insert("is_default".to_owned(), serde_json::Value::Bool(false));

    for (key, value) in &table {
        match (key.as_str(), value) {
            ("name" | "mode", _) => {}
            ("colors", toml::Value::Table(colors)) => {
                let mut flat = Vec::new();
                flatten("", colors, &mut flat);
                let target = object
                    .entry("colors")
                    .or_insert_with(|| serde_json::Value::Object(Default::default()));
                for (color, value) in flat {
                    let toml::Value::String(text) = &value else {
                        skipped.push(say!("theme-colour-is-text", key = &color));
                        continue;
                    };
                    // The terminal's colours are ours, not the toolkit's.
                    if let Some(name) = color.strip_prefix("terminal.") {
                        let parsed = (text.len() <= 200)
                            .then(|| try_parse_color(text).ok())
                            .flatten();
                        let mut probe = terminal::builtin(terminal::Builtin::Dark);
                        match parsed {
                            None => skipped.push(say!(
                                "theme-not-a-colour",
                                key = &color,
                                value = format!("{:?}", shortened(text))
                            )),
                            Some(parsed) if terminal::set(&mut probe, name, parsed) => {
                                terminal_colors.push((name.to_owned(), parsed))
                            }
                            Some(_) => {
                                skipped.push(say!("theme-no-colour", key = shortened(&color)))
                            }
                        }
                        continue;
                    }
                    if !known.contains(&color) {
                        skipped.push(say!("theme-no-colour", key = &color));
                        continue;
                    }
                    if text.len() > 200 || try_parse_color(text).is_err() {
                        skipped.push(say!(
                            "theme-not-a-colour",
                            key = &color,
                            value = format!("{:?}", shortened(text))
                        ));
                        continue;
                    }
                    if let Some(target) = target.as_object_mut() {
                        target.insert(color, serde_json::Value::String(text.clone()));
                    }
                }
            }
            ("colors", _) => skipped.push(say!("theme-colors-not-a-table").to_owned()),
            ("lanes", toml::Value::Array(list)) => {
                if list.is_empty() || list.len() > MOST_LANES {
                    skipped.push(say!("theme-lanes-count", most = MOST_LANES));
                    continue;
                }
                for (at, value) in list.iter().enumerate() {
                    let color = value
                        .as_str()
                        .filter(|text| text.len() <= 200)
                        .and_then(|text| try_parse_color(text).ok());
                    match color {
                        Some(color) => lanes.push(color),
                        None => skipped.push(say!(
                            "theme-lane-not-a-colour",
                            number = at.saturating_add(1)
                        )),
                    }
                }
            }
            ("lanes", _) => skipped.push(say!("theme-lanes-not-a-list").to_owned()),
            ("syntax", toml::Value::Table(syntax)) => {
                let mut named = serde_json::Map::new();
                read_syntax("", syntax, &known_syntax(), &mut named, &mut skipped, 0);
                if named.is_empty() {
                    continue;
                }
                // The toolkit's syntax colours of the same mode, with what
                // the file names over them.
                let Some(mut highlight) = builtin_highlight(dark) else {
                    continue;
                };
                if let Some(target) = highlight
                    .get_mut("syntax")
                    .and_then(|syntax| syntax.as_object_mut())
                {
                    for (name, style) in named {
                        let slot = target.entry(name).or_insert(serde_json::Value::Null);
                        match (slot.as_object_mut(), style) {
                            (Some(slot), serde_json::Value::Object(style)) => slot.extend(style),
                            (_, style) => *slot = style,
                        }
                    }
                }
                object.insert("highlight".to_owned(), highlight);
            }
            ("syntax", _) => skipped.push(say!("theme-syntax-not-a-table").to_owned()),
            ("radius" | "radius.lg", toml::Value::Integer(radius)) if (0..=40).contains(radius) => {
                object.insert(key.clone(), serde_json::Value::from(*radius));
            }
            ("shadow", toml::Value::Boolean(shadow)) => {
                object.insert(key.clone(), serde_json::Value::Bool(*shadow));
            }
            ("font.family" | "mono_font.family", toml::Value::String(family))
                if !family.trim().is_empty() && family.len() <= 100 =>
            {
                object.insert(key.clone(), serde_json::Value::String(family.clone()));
            }
            (other, _) => skipped.push(say!("theme-not-a-setting", key = other)),
        }
    }
    Ok(ReadTheme {
        name,
        json,
        lanes,
        terminal: terminal_colors,
        skipped,
    })
}

/// The start of a text, for a message.
fn shortened(text: &str) -> String {
    let mut cut: String = text.chars().take(30).collect();
    if cut.len() < text.len() {
        cut.push('…');
    }
    cut
}

/// Read every theme file of a folder: the themes, by name, and what there
/// is to say about files or parts of files that could not be used.
pub fn read_folder(dir: &Path) -> (Vec<UserTheme>, Vec<String>) {
    let mut themes: Vec<UserTheme> = Vec::new();
    let mut problems = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "toml") && path.is_file())
        .collect();
    files.sort();
    for file in files.into_iter().take(200) {
        let file_name = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = match std::fs::read(&file) {
            Ok(bytes) if bytes.len() > 1024 * 1024 => {
                problems.push(say!("theme-file-too-large", file_name = &file_name));
                continue;
            }
            Ok(bytes) => match String::from_utf8(bytes) {
                Ok(text) => text,
                Err(_) => {
                    problems.push(say!("theme-file-not-text", file_name = &file_name));
                    continue;
                }
            },
            Err(err) => {
                problems.push(format!("{file_name}: {err}"));
                continue;
            }
        };
        let read = match read_user_theme(&text, &file_name) {
            Ok(read) => read,
            Err(why) => {
                problems.push(format!("{file_name}: {why}"));
                continue;
            }
        };
        if themes.iter().any(|theme| theme.name == read.name) {
            problems.push(say!(
                "theme-file-name-taken",
                file_name = &file_name,
                name = format!("{:?}", read.name)
            ));
            continue;
        }
        let lanes = read.lanes;
        let terminal = read.terminal;
        let config: ThemeConfig = match serde_json::from_value(read.json) {
            Ok(config) => config,
            Err(err) => {
                problems.push(format!("{file_name}: {err}"));
                continue;
            }
        };
        problems.extend(
            read.skipped
                .into_iter()
                .take(6)
                .map(|what| format!("{file_name}: {what}")),
        );
        themes.push(UserTheme {
            name: read.name,
            config: Rc::new(config),
            lanes,
            terminal,
        });
    }
    themes.sort_by_key(|theme| theme.name.to_lowercase());
    (themes, problems)
}

/// A theme file to start from: our dark theme with every colour it sets,
/// and what a file may hold.
pub fn example_file() -> String {
    let mut out = format!("{}\n", say!("theme-example-head"));
    let lanes: Vec<String> = (0..LANE_COLORS)
        .filter_map(|lane| serde_json::to_value(generated_lane(lane, true)).ok())
        .filter_map(|color| color.as_str().map(|color| format!("\"{color}\"")))
        .collect();
    // not shown: the file's keys
    out.push_str(&format!("lanes = [{}]\n\n[colors]\n", lanes.join(", ")));
    let colors = builtin_json(true)
        .and_then(|theme| theme.get("colors").cloned())
        .and_then(|colors| colors.as_object().cloned())
        .unwrap_or_default();
    for (name, value) in colors {
        if let Some(value) = value.as_str() {
            let key = if name.contains('.') {
                format!("\"{name}\"")
            } else {
                name
            };
            out.push_str(&format!("{key} = \"{value}\"\n"));
        }
    }
    // The terminal's: its text and ground, the cursor, the selection,
    // faint text, and the sixteen colours programs ask for by number.
    out.push_str(say!("theme-example-terminal"));
    out.push('\n');
    for (name, color) in terminal::named(&terminal::builtin(terminal::Builtin::Dark)) {
        if let Some(color) = serde_json::to_value(color)
            .ok()
            .and_then(|color| color.as_str().map(str::to_owned))
        {
            // not shown: the file's keys
            out.push_str(&format!("\"terminal.{name}\" = \"{color}\"\n"));
        }
    }
    // The colours of code. One that is a table can also say
    // font_style = "italic" and font_weight = 700.
    out.push('\n');
    out.push_str(say!("theme-example-syntax"));
    out.push_str("\n[syntax]\n");
    let syntax = builtin_highlight(true)
        .and_then(|style| style.get("syntax").cloned())
        .and_then(|syntax| syntax.as_object().cloned())
        .unwrap_or_default();
    for (name, style) in syntax {
        let key = if name.contains('.') {
            format!("\"{name}\"")
        } else {
            name
        };
        match style.get("color").and_then(|color| color.as_str()) {
            Some(color) => out.push_str(&format!("{key} = \"{color}\"\n")),
            // One the built-in theme leaves to the colour of the text.
            // not shown: a key of the file, as a comment
            None => out.push_str(&format!("# {key} = \"#rrggbb\"\n")),
        }
    }
    out
}

/// Every theme there is to choose from: ours, then the user's by name.
pub fn choices(cx: &App) -> Vec<ThemeChoice> {
    let mut all = ThemeChoice::BUILT_IN.to_vec();
    if let Some(user) = cx.try_global::<UserThemes>() {
        all.extend(
            user.themes
                .iter()
                .map(|theme| ThemeChoice::User(theme.name.clone())),
        );
    }
    all
}

/// The theme after `current` among `all`, going round; the first when
/// `current` is not among them.
pub fn next_choice(current: &ThemeChoice, all: &[ThemeChoice]) -> ThemeChoice {
    let position = all.iter().position(|choice| choice == current);
    let next = match position {
        Some(ix) => ix.saturating_add(1).checked_rem(all.len()).unwrap_or(0),
        None => 0,
    };
    all.get(next).cloned().unwrap_or_default()
}

/// Register our themes, read the user's and apply the preferred one.
#[allow(clippy::print_stderr)] // a broken bundled theme is a build defect worth shouting about
pub fn init(cx: &mut App) {
    if let Err(err) = ThemeRegistry::global_mut(cx).load_themes_from_str(THEMES_JSON) {
        eprintln!("bitbraid: cannot load bundled themes: {err:#}");
    }
    let dir = cx.global::<Prefs>().dir().map(|dir| dir.join("themes"));
    cx.set_global(UserThemes {
        dir,
        ..UserThemes::default()
    });
    let _ = reload_user(cx);
}

/// Read the folder of themes again and apply the theme in use anew, so
/// that it follows its file. Returns what there is to say.
pub fn reload_user(cx: &mut App) -> Vec<String> {
    let dir = cx.global::<UserThemes>().dir.clone();
    let (themes, problems) = match &dir {
        Some(dir) => read_folder(dir),
        None => (Vec::new(), Vec::new()),
    };
    let user = cx.global_mut::<UserThemes>();
    user.themes = themes;
    user.problems = problems.clone();
    let choice = cx.global::<Prefs>().preferences.theme.clone();
    apply(&choice, cx);
    problems
}

/// Whether the chosen theme is there to be applied: one of ours always is.
pub fn exists(choice: &ThemeChoice, cx: &App) -> bool {
    match choice {
        ThemeChoice::User(name) => cx
            .try_global::<UserThemes>()
            .is_some_and(|user| user.themes.iter().any(|theme| &theme.name == name)),
        _ => true,
    }
}

/// Apply a theme choice. A theme of the user's own that is not there (its
/// file is gone, or broken) shows as our dark theme until it is back.
pub fn apply(choice: &ThemeChoice, cx: &mut App) {
    let own = match choice {
        ThemeChoice::User(name) => cx.try_global::<UserThemes>().and_then(|user| {
            user.themes
                .iter()
                .find(|theme| &theme.name == name)
                .map(|theme| {
                    (
                        theme.config.clone(),
                        theme.lanes.clone(),
                        theme.terminal.clone(),
                    )
                })
        }),
        _ => None,
    };
    let (config, lanes, terminal_colors) = match own {
        Some((config, lanes, terminal_colors)) => (Some(config), lanes, terminal_colors),
        None => (
            None,
            ours(choice).map(Ours::lanes).unwrap_or_default(),
            Vec::new(),
        ),
    };
    cx.set_global(Lanes(lanes));
    let config = config.or_else(|| {
        // not shown: the name the dark theme is registered under
        let name = builtin_name(choice).unwrap_or("BitBraid dark");
        ThemeRegistry::global(cx).themes().get(name).cloned()
    });
    let mode = match config.as_ref().map(|config| config.mode) {
        Some(ThemeMode::Light) => ThemeMode::Light,
        _ => ThemeMode::Dark,
    };
    // The terminal's colours: ours of the theme, or of its mode, with
    // what a file of the user's names over them.
    let mut terminal = terminal::builtin(match (ours(choice), mode) {
        (Some(ours), _) => ours.terminal,
        (None, ThemeMode::Light) => terminal::Builtin::Light,
        (None, ThemeMode::Dark) => terminal::Builtin::Dark,
    });
    for (name, color) in &terminal_colors {
        terminal::set(&mut terminal, name, *color);
    }
    cx.set_global(TerminalTheme(terminal));
    Theme::change(mode, None, cx);
    crate::chrome::follow_theme(mode == ThemeMode::Dark, cx);
    let own_colours = config
        .as_ref()
        .is_some_and(|config| config.highlight.is_some());
    if let Some(config) = config {
        Theme::update(cx, |theme| theme.apply_config(&config));
    }
    // Toasts stand in the same place whatever the theme.
    Theme::update(cx, |theme| {
        crate::notices::place(&mut theme.notification, crate::shell::STATUS_BAR_HEIGHT)
    });
    // The colours of code (an editor's ground, the syntax in a diff) come
    // with a theme only when it brings its own (a user's theme with a
    // `[syntax]` table does, and so do ours but the first three), and the
    // toolkit keeps whichever it had last: after dark, light and dark again
    // they would be the light ones. A theme without them gets those of its
    // mode.
    if !own_colours {
        Theme::update(cx, |theme| {
            theme.highlight_theme = match mode {
                ThemeMode::Light => HighlightTheme::default_light(),
                ThemeMode::Dark => HighlightTheme::default_dark(),
            };
        });
    }
    cx.refresh_windows();
}

/// The colours of the terminal in the theme in use.
pub fn terminal_colors(cx: &App) -> TerminalColors {
    cx.try_global::<TerminalTheme>()
        .map(|theme| theme.0)
        .unwrap_or_else(|| terminal::builtin(terminal::Builtin::Dark))
}

/// Choose a theme: remember it and apply it.
pub fn choose(choice: ThemeChoice, cx: &mut App) {
    cx.global_mut::<Prefs>().preferences.theme = choice.clone();
    cx.global::<Prefs>().save_preferences();
    apply(&choice, cx);
}

/// Move to the next theme and remember it.
pub fn cycle(cx: &mut App) {
    let current = cx.global::<Prefs>().preferences.theme.clone();
    let next = next_choice(&current, &choices(cx));
    choose(next, cx);
}

/// Number of distinct lane colours before they repeat, in a theme that
/// names none.
pub const LANE_COLORS: u32 = 8;

/// The lane colour a theme that names none has for a lane colour id.
fn generated_lane(color: u32, dark: bool) -> Hsla {
    let step = (color % LANE_COLORS) as f32 / LANE_COLORS as f32;
    // Start at a blue, walk the wheel; skip a little so neighbours differ.
    let hue = (0.58 + step * 0.91) % 1.0;
    if dark {
        hsla(hue, 0.42, 0.56, 1.0)
    } else {
        hsla(hue, 0.40, 0.42, 1.0)
    }
}

/// The lane colour for an id among the colours a theme names; none when it
/// names none.
fn named_lane(lanes: &[Hsla], color: u32) -> Option<Hsla> {
    let count = u32::try_from(lanes.len()).ok().filter(|count| *count > 0)?;
    lanes.get(usize::try_from(color % count).ok()?).copied()
}

/// How many lane colours the theme in use has before they repeat.
pub fn lane_count(cx: &App) -> u32 {
    cx.try_global::<Lanes>()
        .and_then(|lanes| u32::try_from(lanes.0.len()).ok())
        .filter(|count| *count > 0)
        .unwrap_or(LANE_COLORS)
}

/// Colour for a lane colour id in the active theme: the theme's own when
/// it names lane colours, else one made from evenly spaced hues.
pub fn lane_color(color: u32, cx: &App) -> Hsla {
    cx.try_global::<Lanes>()
        .and_then(|lanes| named_lane(&lanes.0, color))
        .unwrap_or_else(|| generated_lane(color, cx.theme().is_dark()))
}

#[cfg(test)]
mod tests {
    use super::{
        MOST_LANES, ThemeChoice, ThemeConfig, example_file, folder_stamp, named_lane, next_choice,
        read_folder, read_user_theme,
    };

    #[test]
    fn bundled_theme_json_parses() {
        let value: serde_json::Value = serde_json::from_str(super::THEMES_JSON).unwrap();
        let themes = value["themes"].as_array().unwrap();
        assert_eq!(themes.len(), super::OURS.len());
        // Every colour is one the toolkit knows by that name: one it does
        // not know is dropped without a word.
        let known = super::known_colors();
        assert!(known.len() > 50, "{known:?}");
        let known_syntax = super::known_syntax();
        for theme in themes {
            assert!(theme["colors"]["background"].is_string());
            // All of ours name the same colours.
            assert_eq!(
                theme["colors"].as_object().unwrap().len(),
                themes[0]["colors"].as_object().unwrap().len(),
                "{}",
                theme["name"]
            );
            for (name, value) in theme["colors"].as_object().unwrap() {
                assert!(known.contains(name), "{}: {name}", theme["name"]);
                assert!(
                    super::try_parse_color(value.as_str().unwrap()).is_ok(),
                    "{}: {name} = {value}",
                    theme["name"]
                );
            }
            // So is every syntax colour, for the themes that bring them.
            if let Some(syntax) = theme["highlight"]["syntax"].as_object() {
                for name in syntax.keys() {
                    assert!(known_syntax.contains(name), "{}: {name}", theme["name"]);
                }
            }
            let config: ThemeConfig = serde_json::from_value(theme.clone()).unwrap();
            assert_eq!(
                config.highlight.is_some(),
                theme.get("highlight").is_some(),
                "{}",
                theme["name"]
            );
        }
        // Each of ours is there under its name, in the order of the menus.
        for (ours, choice) in super::OURS.iter().zip(ThemeChoice::BUILT_IN) {
            assert_eq!(ours.choice, choice);
            assert!(
                themes.iter().any(|theme| theme["name"] == ours.registered),
                "{}",
                ours.registered
            );
            assert_eq!(super::label(&choice), ours.label.text());
            assert_eq!(ours.lanes().len(), ours.lanes.len(), "{}", ours.label);
        }
    }

    /// Each of ours reads: its text on its grounds, the text of its
    /// buttons and links, the text of a label on each of its lanes, and
    /// the colours of code on the ground of the code.
    #[test]
    fn every_one_of_ours_reads() {
        use super::terminal::contrast;
        let value: serde_json::Value = serde_json::from_str(super::THEMES_JSON).unwrap();
        for ours in &super::OURS {
            let theme = value["themes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|theme| theme["name"] == ours.registered)
                .unwrap();
            let color = |name: &str| {
                super::try_parse_color(theme["colors"][name].as_str().unwrap()).unwrap()
            };
            for (text, ground, least) in [
                ("foreground", "background", 7.0),
                ("foreground", "list.active.background", 7.0),
                ("popover.foreground", "popover.background", 7.0),
                ("muted.foreground", "background", 4.5),
                ("tab.foreground", "tab_bar.background", 4.5),
                ("link", "background", 4.5),
                ("primary.foreground", "primary.background", 3.5),
            ] {
                let measured = contrast(color(text), color(ground));
                assert!(
                    measured >= least,
                    "{}: {text} on {ground} ({measured:.2})",
                    ours.label
                );
            }
            // A label's text is the ground on a dark theme, white on a
            // light one (`graph.rs`).
            let dark = theme["mode"] == "dark";
            let on_label = if dark {
                color("background")
            } else {
                gpui_kit::hsla(0., 0., 1., 1.)
            };
            for (at, lane) in ours.lanes().into_iter().enumerate() {
                let measured = contrast(on_label, lane);
                assert!(
                    measured >= 4.0,
                    "{}: a label on lane {at} ({measured:.2})",
                    ours.label
                );
            }
            let highlight = &theme["highlight"];
            if let Some(syntax) = highlight["syntax"].as_object() {
                let ground =
                    super::try_parse_color(highlight["editor.background"].as_str().unwrap())
                        .unwrap();
                for (name, style) in syntax {
                    let Some(text) = style["color"].as_str() else {
                        continue;
                    };
                    let measured = contrast(super::try_parse_color(text).unwrap(), ground);
                    assert!(
                        measured >= 4.0,
                        "{}: syntax {name} ({measured:.2})",
                        ours.label
                    );
                }
            }
        }
    }

    #[test]
    fn a_user_theme_cannot_take_the_name_of_one_of_ours() {
        for name in ["Ember", "sage", "High contrast", "BitBraid dusk"] {
            let err = read_user_theme(&format!("name = \"{name}\""), "x.toml").unwrap_err();
            assert!(err.contains("built-in"), "{name}: {err}");
        }
        // A file called after one of ours, without a name in it.
        assert!(read_user_theme("", "paper.toml").is_err());
        assert!(read_user_theme("", "papers.toml").is_ok());
    }

    #[test]
    fn a_theme_file_changes_what_it_names_and_keeps_the_rest() {
        let read = read_user_theme(
            "name = \"Harbour\"\nmode = \"light\"\nradius = 2\n\n[colors]\nbackground = \"#f4f1ea\"\n\
             \"primary.background\" = \"#2f9e8f\"\n\n[colors.muted]\nforeground = \"#667788\"\n",
            "harbour.toml",
        )
        .unwrap();
        assert_eq!(read.name, "Harbour");
        assert_eq!(read.skipped, Vec::<String>::new());
        assert_eq!(read.json["mode"], "light");
        assert_eq!(read.json["radius"], 2);
        assert_eq!(read.json["colors"]["background"], "#f4f1ea");
        assert_eq!(read.json["colors"]["primary.background"], "#2f9e8f");
        // A table inside the colours is the dotted name.
        assert_eq!(read.json["colors"]["muted.foreground"], "#667788");
        // What it does not name is our light theme's.
        let light = super::builtin_json(false).unwrap();
        assert_eq!(read.json["colors"]["border"], light["colors"]["border"]);
        // And the toolkit takes the whole of it.
        let config: ThemeConfig = serde_json::from_value(read.json).unwrap();
        assert_eq!(config.name.as_ref(), "Harbour");
        assert!(!config.mode.is_dark());

        // No name: the file's; no mode: dark; nothing at all is our dark theme.
        let plain = read_user_theme("", "plain.toml").unwrap();
        assert_eq!(plain.name, "plain");
        assert_eq!(plain.json["mode"], "dark");
    }

    #[test]
    fn what_cannot_be_right_in_a_theme_is_left_out_and_said() {
        let read = read_user_theme(
            "name = \"Odd\"\nradius = 4000\nwidgets = 3\n\n[colors]\nbackground = \"not a colour\"\n\
             border = 12\nnonsense = \"#ffffff\"\nforeground = \"#ffffff\"\n",
            "odd.toml",
        )
        .unwrap();
        assert_eq!(read.skipped.len(), 5, "{:?}", read.skipped);
        assert!(
            read.skipped
                .iter()
                .any(|said| said.starts_with("background:"))
        );
        assert!(
            read.skipped
                .iter()
                .any(|said| said.starts_with("nonsense:"))
        );
        assert!(read.skipped.iter().any(|said| said.starts_with("radius:")));
        // What is right in it counts all the same.
        assert_eq!(read.json["colors"]["foreground"], "#ffffff");
        let dark = super::builtin_json(true).unwrap();
        assert_eq!(
            read.json["colors"]["background"],
            dark["colors"]["background"]
        );
        assert!(serde_json::from_value::<ThemeConfig>(read.json).is_ok());

        // Files of which nothing can be used.
        for (text, why) in [
            ("name = ", "TOML"),
            ("name = 7", "name has to be a text"),
            ("mode = \"sepia\"", "mode is"),
            ("name = \"Dark\"", "built-in"),
            ("name = \"BitBraid dark\"", "built-in"),
            ("name = \"a\\u0007b\"", "control character"),
        ] {
            let err = read_user_theme(text, "x.toml").unwrap_err();
            assert!(err.contains(why), "{text:?}: {err}");
        }
        assert!(read_user_theme(&format!("name = \"{}\"", "n".repeat(100)), "x.toml").is_err());
    }

    #[test]
    fn a_folder_of_themes_is_read_and_broken_files_are_said() {
        let dir = std::env::temp_dir().join(format!("bitbraid-themes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // A folder that is not there is no themes and nothing to say.
        assert!(read_folder(&dir).0.is_empty() && read_folder(&dir).1.is_empty());
        assert!(folder_stamp(&dir).is_empty());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("b.toml"),
            "name = \"Zeta\"\n[colors]\nborder = \"#123456\"\n",
        )
        .unwrap();
        std::fs::write(dir.join("a.toml"), "mode = \"light\"\n").unwrap();
        std::fs::write(dir.join("broken.toml"), "[colors\n").unwrap();
        std::fs::write(dir.join("twice.toml"), "name = \"Zeta\"\n").unwrap();
        std::fs::write(dir.join("binary.toml"), [0xff, 0xfe, 0x00]).unwrap();
        std::fs::write(dir.join("notes.txt"), "not a theme").unwrap();
        std::fs::write(dir.join("example.toml"), example_file()).unwrap();
        let (themes, problems) = read_folder(&dir);
        let names: Vec<&str> = themes.iter().map(|theme| theme.name.as_str()).collect();
        assert_eq!(names, ["a", "Example", "Zeta"]);
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(problems.iter().any(|said| said.starts_with("broken.toml:")));
        assert!(problems.iter().any(|said| said.starts_with("twice.toml:")));
        assert!(problems.iter().any(|said| said.starts_with("binary.toml:")));
        // The example is our dark theme, colour for colour.
        let example = themes.iter().find(|theme| theme.name == "Example").unwrap();
        assert!(example.config.mode.is_dark());

        // A change to a file is seen in the stamp.
        let before = folder_stamp(&dir);
        assert_eq!(before.len(), 6);
        std::fs::write(dir.join("a.toml"), "mode = \"light\"\nradius = 3\n").unwrap();
        assert_ne!(folder_stamp(&dir), before);
        std::fs::remove_file(dir.join("a.toml")).unwrap();
        assert_eq!(folder_stamp(&dir).len(), 5);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_theme_names_lane_and_syntax_colours() {
        let read = read_user_theme(
            concat!(
                "name = \"Harbour\"\n",
                "lanes = [\"#4f8fd9\", \"#D9A34F\", \"#ffffff80\"]\n\n",
                "[syntax]\n",
                "keyword = \"#c586c0\"\n",
                "\"string.escape\" = \"#d7ba7d\"\n",
                "comment = { color = \"#6a9955\", font_style = \"italic\", font_weight = 700 }\n",
                "function = { font_style = \"italic\" }\n\n",
                "[syntax.string]\n",
                "color = \"#ce9178\"\n",
                "regex = \"#d16969\"\n",
            ),
            "harbour.toml",
        )
        .unwrap();
        assert_eq!(read.skipped, Vec::<String>::new());
        assert_eq!(read.lanes.len(), 3);
        assert_eq!(named_lane(&read.lanes, 0), read.lanes.first().copied());
        // The colours go round.
        assert_eq!(named_lane(&read.lanes, 4), read.lanes.get(1).copied());
        assert_eq!(
            named_lane(&read.lanes, u32::MAX),
            read.lanes.first().copied()
        );
        assert_eq!(named_lane(&[], 3), None);

        let syntax = &read.json["highlight"]["syntax"];
        assert_eq!(syntax["keyword"]["color"], "#c586c0ff");
        assert_eq!(syntax["string.escape"]["color"], "#d7ba7dff");
        assert_eq!(syntax["comment"]["color"], "#6a9955ff");
        assert_eq!(syntax["comment"]["font_style"], "italic");
        assert_eq!(syntax["comment"]["font_weight"], 700);
        // A table under [syntax] is the name and the names under it.
        assert_eq!(syntax["string"]["color"], "#ce9178ff");
        assert_eq!(syntax["string.regex"]["color"], "#d16969ff");
        // What it does not name is the built-in theme's, and a name it
        // only says the slant of keeps its colour.
        let ours = super::builtin_highlight(true).unwrap();
        assert_eq!(syntax["number"], ours["syntax"]["number"]);
        assert!(ours["syntax"]["number"]["color"].is_string());
        assert_eq!(
            syntax["function"]["color"],
            ours["syntax"]["function"]["color"]
        );
        assert_eq!(syntax["function"]["font_style"], "italic");
        // And the toolkit takes the whole of it.
        let config: ThemeConfig = serde_json::from_value(read.json).unwrap();
        assert!(config.highlight.is_some());

        // A light theme starts from the light colours of code.
        let light = read_user_theme(
            "mode = \"light\"\n[syntax]\nkeyword = \"#000080\"\n",
            "l.toml",
        )
        .unwrap();
        let ours = super::builtin_highlight(false).unwrap();
        assert_eq!(
            light.json["highlight"]["syntax"]["number"],
            ours["syntax"]["number"]
        );
        assert_ne!(ours, super::builtin_highlight(true).unwrap());
        // A theme that names none has none of its own: the built-in ones
        // of its mode are used, and the lanes are made up.
        let plain = read_user_theme("name = \"Plain\"\n", "p.toml").unwrap();
        assert!(plain.json.get("highlight").is_none() && plain.lanes.is_empty());
        // The names there are.
        let known = super::known_syntax();
        assert!(known.len() > 30, "{known:?}");
        for name in ["keyword", "comment", "string.escape", "type", "function"] {
            assert!(known.iter().any(|known| known == name), "{name}");
        }
    }

    #[test]
    fn broken_lanes_and_syntax_colours_are_said_and_left_out() {
        let read = read_user_theme(
            concat!(
                "lanes = [\"#fff\", 7, \"nope\", \"#000000\"]\n\n",
                "[syntax]\n",
                "nonsense = \"#ffffff\"\n",
                "keyword = \"nope\"\n",
                "comment = 7\n",
                "function = { color = 3, font_style = \"bold\", font_weight = 450 }\n",
                "number = \"#b5cea8\"\n\n",
                "[syntax.string]\n",
                "colour = \"#ffffff\"\n",
            ),
            "odd.toml",
        )
        .unwrap();
        // The colours that are colours count.
        assert_eq!(read.lanes.len(), 2);
        assert_eq!(read.skipped.len(), 9, "{:?}", read.skipped);
        for said in [
            "lanes: number 2 is not a colour",
            "lanes: number 3 is not a colour",
            "syntax.nonsense: there is no syntax colour of that name",
            "syntax.keyword: \"nope\" is not a colour",
            "syntax.string.colour: there is no syntax colour of that name",
        ] {
            assert!(read.skipped.iter().any(|skipped| skipped == said), "{said}");
        }
        assert!(read.skipped.iter().any(|said| said.contains("font_style")));
        assert!(read.skipped.iter().any(|said| said.contains("font_weight")));
        assert!(read.skipped.iter().all(|said| said.len() < 200));
        let ours = super::builtin_highlight(true).unwrap();
        let syntax = &read.json["highlight"]["syntax"];
        assert_eq!(syntax["number"]["color"], "#b5cea8ff");
        assert_eq!(syntax["keyword"], ours["syntax"]["keyword"]);
        assert_eq!(syntax["function"], ours["syntax"]["function"]);
        assert!(serde_json::from_value::<ThemeConfig>(read.json).is_ok());

        // Nothing right in it: the built-in colours of code, and lanes
        // made up as in a theme that names none.
        for (text, why) in [
            ("lanes = \"blue\"", "lanes has to be a list"),
            ("lanes = []", "lanes: a list of 1 to 16"),
            (
                "lanes = [\"red\", \"green\"]\nsyntax = \"dark\"",
                "syntax has to be a table",
            ),
            ("[syntax]\nkeyword = \"\"", "is not a colour"),
            ("[syntax.a.b.c.d.e.f.g]\nh = \"#fff\"", "no syntax colour"),
        ] {
            let read = read_user_theme(text, "x.toml").unwrap();
            assert!(
                read.skipped.iter().any(|said| said.contains(why)),
                "{text:?}: {:?}",
                read.skipped
            );
            assert!(read.json.get("highlight").is_none(), "{text:?}");
            assert!(serde_json::from_value::<ThemeConfig>(read.json).is_ok());
        }
        let many = format!(
            "lanes = [{}]",
            vec!["\"#123456\""; MOST_LANES + 1].join(", ")
        );
        let read = read_user_theme(&many, "x.toml").unwrap();
        assert!(read.lanes.is_empty() && read.skipped.len() == 1);
        let most = format!("lanes = [{}]", vec!["\"#123456\""; MOST_LANES].join(", "));
        assert_eq!(
            read_user_theme(&most, "x.toml").unwrap().lanes.len(),
            MOST_LANES
        );
        let long = format!(
            "[syntax]\n\"{}\" = \"{}\"",
            "k".repeat(5000),
            "v".repeat(5000)
        );
        let read = read_user_theme(&long, "x.toml").unwrap();
        assert!(
            read.skipped.iter().all(|said| said.len() < 200),
            "{:?}",
            read.skipped
        );
    }

    #[test]
    fn the_example_names_everything_a_theme_can() {
        let read = read_user_theme(&example_file(), "example.toml").unwrap();
        assert_eq!(read.skipped, Vec::<String>::new());
        assert_eq!(read.name, "Example");
        // The lanes of a theme that names none, and the built-in colours
        // of code: the example changes nothing.
        assert_eq!(read.lanes.len(), super::LANE_COLORS as usize);
        let ours = super::builtin_highlight(true).unwrap();
        assert_eq!(read.json["highlight"], ours);
        assert!(serde_json::from_value::<ThemeConfig>(read.json).is_ok());
    }

    #[test]
    fn a_theme_file_names_the_terminals_colours() {
        let read = read_user_theme(
            concat!(
                "mode = \"light\"\n[colors]\n\"terminal.background\" = \"#fdf6e3\"\n",
                "\"terminal.ansi.bright_red\" = \"#aa0000\"\n",
                "\"terminal.ansi.orange\" = \"#ff8800\"\n",
                "\"terminal.cursor\" = \"not a colour\"\n",
                "[colors.terminal.selection]\nbackground = \"#eee8d5\"\n",
            ),
            "solar.toml",
        )
        .unwrap();
        let mut names: Vec<&str> = read
            .terminal
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        names.sort_unstable();
        assert_eq!(
            names,
            ["ansi.bright_red", "background", "selection.background"]
        );
        assert_eq!(read.skipped.len(), 2, "{:?}", read.skipped);
        assert!(
            read.skipped
                .iter()
                .any(|said| said.starts_with("terminal.ansi.orange:"))
        );
        assert!(
            read.skipped
                .iter()
                .any(|said| said.starts_with("terminal.cursor:"))
        );
        // They are not the toolkit's, and the toolkit takes the rest.
        assert!(read.json["colors"].get("terminal.background").is_none());
        assert!(serde_json::from_value::<ThemeConfig>(read.json).is_ok());
    }

    #[test]
    fn the_cycle_goes_through_ours_and_the_users() {
        let user = |name: &str| ThemeChoice::User(name.to_owned());
        let all = [
            ThemeChoice::Dark,
            ThemeChoice::Light,
            ThemeChoice::Contrast,
            user("Harbour"),
        ];
        assert_eq!(next_choice(&ThemeChoice::Dark, &all), ThemeChoice::Light);
        assert_eq!(next_choice(&ThemeChoice::Contrast, &all), user("Harbour"));
        assert_eq!(next_choice(&user("Harbour"), &all), ThemeChoice::Dark);
        // One that is gone: back to the first.
        assert_eq!(next_choice(&user("Gone"), &all), ThemeChoice::Dark);
        assert_eq!(next_choice(&ThemeChoice::Dark, &[]), ThemeChoice::Dark);
    }
}
