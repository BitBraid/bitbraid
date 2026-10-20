//! Paths written the way a shell reads them back: what is pasted when
//! files are dropped onto a terminal.
//!
//! A path made only of letters, digits and a few marks no shell treats
//! specially stays as it is. Anything else is quoted so that the shell
//! hands the program exactly the path again, whatever is in it: blanks,
//! quotes, `$`, backticks, globs, line breaks, other control characters
//! and, on Unix, bytes that are not UTF-8.
//!
//! - POSIX shells: single quotes, in which nothing is special; a single
//!   quote is written `\'` between two quoted parts. A C0 control
//!   character, DEL or a byte that is not UTF-8 is written `$'\ooo'`
//!   (octal), the dollar-single-quotes of bash, zsh, ksh and
//!   POSIX.1-2024, so that nothing pasted is a key the shell acts on. A
//!   C1 control character stays inside the quotes as it is: no line
//!   editor acts on one, and dash 0.5.13 misreads the bytes `\201` to
//!   `\210` in dollar-single-quotes, and ends on some of them. A byte
//!   that is not UTF-8 in that range still has to be written so, since
//!   pasted text is UTF-8.
//! - fish: single quotes, in which `\` and `'` are escaped with a
//!   backslash; control characters outside them as `\n`, `\xHH`, `\uHHHH`,
//!   and bytes that are not UTF-8 as `\XHH`.
//! - nushell: double quotes, in which `\` and `"` are escaped with a
//!   backslash and control characters are written `\n`, `\t`, `\r` or
//!   `\u{HH}` (its single quotes cannot hold a single quote). A string
//!   there is UTF-8: a byte that is not becomes the replacement
//!   character, as nushell itself shows such a name.
//! - tcsh and csh: single quotes, with `'` and `!` (history, even inside
//!   quotes) written `\'` and `\!` between quoted parts. They have no way
//!   to write a control character in a word typed at the prompt, nor a
//!   byte that is not UTF-8 in text pasted: those are left out. (tcsh
//!   6.24 keeps a character beyond U+FFFF, an emoji, only mangled,
//!   however it is quoted.)
//! - PowerShell: single quotes, in which a quote (and the typographic
//!   quotes PowerShell takes for one) is doubled; with control characters,
//!   double quotes instead, in which `` ` ``, `"` and `$` are escaped with a
//!   backtick and control characters are written `` `n ``, `` `u{HHHH} ``.
//! - cmd: double quotes. It has no way to write a control character or a
//!   double quote, which a Windows file name cannot hold; they are left
//!   out.
//!
//! Sources: POSIX Shell Command Language, 2.2 Quoting (2.2.2 Single-Quotes,
//! 2.2.4 Dollar-Single-Quotes); the bash manual, "ANSI-C Quoting"; the
//! fish documentation, "Quotes" and "Escaping Characters"; the nushell
//! book, "Working with Strings" (single, double and raw strings, the
//! escapes of double quotes); tcsh(1), "Lexical structure" and "History
//! substitution" (`!` inside single quotes); the PowerShell
//! documentation, about_Quoting_Rules and about_Special_Characters; the
//! cmd documentation (`cmd /?`, quoting of file names with spaces).

use std::path::{Path, PathBuf};

use crate::ShellKind;

/// The paths, each written for `shell`, separated by single spaces.
pub fn quote_paths(paths: &[PathBuf], shell: ShellKind) -> String {
    let mut out = String::new();
    for (at, path) in paths.iter().enumerate() {
        if at > 0 {
            out.push(' ');
        }
        out.push_str(&quote_path(path, shell));
    }
    out
}

/// One path written for `shell`.
pub fn quote_path(path: &Path, shell: ShellKind) -> String {
    let bytes = path_bytes(path);
    let pieces = pieces(&bytes);
    if !pieces.is_empty()
        && pieces
            .iter()
            .all(|piece| matches!(piece, Piece::Char(c) if is_plain(*c, shell)))
        && reads_as_text(&pieces, shell)
    {
        return pieces
            .iter()
            .filter_map(|piece| match piece {
                Piece::Char(c) => Some(*c),
                Piece::Byte(_) => None,
            })
            .collect();
    }
    match shell {
        ShellKind::Posix => posix(&pieces),
        ShellKind::Fish => fish(&pieces),
        ShellKind::Nushell => nushell(&pieces),
        ShellKind::Csh => csh(&pieces),
        ShellKind::PowerShell => powershell(&pieces),
        ShellKind::Cmd => cmd(&pieces),
    }
}

#[cfg(unix)]
fn path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;
    path.as_os_str().as_bytes().to_vec()
}

#[cfg(not(unix))]
fn path_bytes(path: &Path) -> Vec<u8> {
    path.to_string_lossy().into_owned().into_bytes()
}

/// A character of a path, or a byte of it that is not UTF-8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    Char(char),
    Byte(u8),
}

fn pieces(bytes: &[u8]) -> Vec<Piece> {
    let mut out = Vec::with_capacity(bytes.len());
    for chunk in bytes.utf8_chunks() {
        out.extend(chunk.valid().chars().map(Piece::Char));
        out.extend(chunk.invalid().iter().copied().map(Piece::Byte));
    }
    out
}

/// Whether a character needs no quoting in this shell.
fn is_plain(c: char, shell: ShellKind) -> bool {
    if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-') {
        return true;
    }
    match shell {
        ShellKind::Posix | ShellKind::Fish => matches!(c, '+' | ',' | ':' | '@' | '%'),
        ShellKind::Csh => matches!(c, '+' | ',' | ':' | '@'),
        ShellKind::Nushell => false,
        ShellKind::PowerShell | ShellKind::Cmd => matches!(c, '\\' | ':'),
    }
}

/// Whether a word of plain characters is read as the text it is. Nushell
/// reads a bare word that starts with a digit as a number, a size or a
/// duration (`1kb`, `10sec`), one that starts with `-` as a flag, and
/// `true`, `false` and `null` as what they name.
fn reads_as_text(pieces: &[Piece], shell: ShellKind) -> bool {
    if shell != ShellKind::Nushell {
        return true;
    }
    let first_is_text = matches!(
        pieces.first(),
        Some(Piece::Char(c)) if !c.is_ascii_digit() && *c != '-'
    );
    let word: String = pieces
        .iter()
        .filter_map(|piece| match piece {
            Piece::Char(c) => Some(*c),
            Piece::Byte(_) => None,
        })
        .collect();
    first_is_text && !matches!(word.as_str(), "true" | "false" | "null")
}

/// Whether a character is a C1 control character (U+0080 to U+009F).
fn is_c1(c: char) -> bool {
    ('\u{80}'..='\u{9f}').contains(&c)
}

/// Whether a character is one a terminal or a line editor may act on:
/// C0, DEL and C1.
fn is_control(c: char) -> bool {
    c.is_control()
}

fn posix(pieces: &[Piece]) -> String {
    let mut out = String::new();
    let mut open = false;
    let close = |out: &mut String, open: &mut bool| {
        if *open {
            out.push('\'');
            *open = false;
        }
    };
    for piece in pieces {
        match *piece {
            Piece::Char('\'') => {
                close(&mut out, &mut open);
                out.push_str("\\'");
            }
            Piece::Char(c) if !is_control(c) || is_c1(c) => {
                if !open {
                    out.push('\'');
                    open = true;
                }
                out.push(c);
            }
            Piece::Char(c) => {
                close(&mut out, &mut open);
                out.push_str("$'");
                match c {
                    '\n' => out.push_str("\\n"),
                    '\t' => out.push_str("\\t"),
                    '\r' => out.push_str("\\r"),
                    _ => {
                        let mut buffer = [0u8; 4];
                        for byte in c.encode_utf8(&mut buffer).bytes() {
                            out.push_str(&format!("\\{byte:03o}"));
                        }
                    }
                }
                out.push('\'');
            }
            Piece::Byte(byte) => {
                close(&mut out, &mut open);
                out.push_str(&format!("$'\\{byte:03o}'"));
            }
        }
    }
    close(&mut out, &mut open);
    if out.is_empty() {
        out.push_str("''");
    }
    out
}

fn fish(pieces: &[Piece]) -> String {
    let mut out = String::new();
    let mut open = false;
    for piece in pieces {
        match *piece {
            Piece::Char(c) if !is_control(c) => {
                if !open {
                    out.push('\'');
                    open = true;
                }
                if c == '\'' || c == '\\' {
                    out.push('\\');
                }
                out.push(c);
            }
            other => {
                if open {
                    out.push('\'');
                    open = false;
                }
                match other {
                    Piece::Char('\n') => out.push_str("\\n"),
                    Piece::Char('\t') => out.push_str("\\t"),
                    Piece::Char('\r') => out.push_str("\\r"),
                    Piece::Char(c) if (c as u32) < 0x80 => {
                        out.push_str(&format!("\\x{:02x}", c as u32));
                    }
                    Piece::Char(c) => out.push_str(&format!("\\u{:04x}", c as u32)),
                    Piece::Byte(byte) => out.push_str(&format!("\\X{byte:02x}")),
                }
            }
        }
    }
    if open {
        out.push('\'');
    }
    if out.is_empty() {
        out.push_str("''");
    }
    out
}

fn nushell(pieces: &[Piece]) -> String {
    let mut out = String::from("\"");
    for piece in pieces {
        match *piece {
            Piece::Char('\\') => out.push_str("\\\\"),
            Piece::Char('"') => out.push_str("\\\""),
            Piece::Char('\n') => out.push_str("\\n"),
            Piece::Char('\t') => out.push_str("\\t"),
            Piece::Char('\r') => out.push_str("\\r"),
            Piece::Char(c) if is_control(c) => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            Piece::Char(c) => out.push(c),
            Piece::Byte(_) => out.push(char::REPLACEMENT_CHARACTER),
        }
    }
    out.push('"');
    out
}

fn csh(pieces: &[Piece]) -> String {
    let mut out = String::new();
    let mut open = false;
    for piece in pieces {
        match *piece {
            Piece::Char(c @ ('\'' | '!')) => {
                if open {
                    out.push('\'');
                    open = false;
                }
                out.push('\\');
                out.push(c);
            }
            Piece::Char(c) if !is_control(c) => {
                if !open {
                    out.push('\'');
                    open = true;
                }
                out.push(c);
            }
            // Nothing a C shell reads at its prompt writes these.
            Piece::Char(_) | Piece::Byte(_) => {}
        }
    }
    if open {
        out.push('\'');
    }
    if out.is_empty() {
        out.push_str("''");
    }
    out
}

/// The characters PowerShell takes for a single quote.
fn powershell_single_quote(c: char) -> bool {
    matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201a}' | '\u{201b}')
}

/// The characters PowerShell takes for a double quote.
fn powershell_double_quote(c: char) -> bool {
    matches!(c, '"' | '\u{201c}' | '\u{201d}' | '\u{201e}')
}

fn powershell(pieces: &[Piece]) -> String {
    // Bytes that are not UTF-8 cannot be written in PowerShell: they
    // become the replacement character, as everywhere on Windows.
    let chars: Vec<char> = pieces
        .iter()
        .map(|piece| match piece {
            Piece::Char(c) => *c,
            Piece::Byte(_) => char::REPLACEMENT_CHARACTER,
        })
        .collect();
    let mut out = String::new();
    if !chars.iter().any(|c| is_control(*c)) {
        out.push('\'');
        for c in chars {
            if powershell_single_quote(c) {
                out.push(c);
            }
            out.push(c);
        }
        out.push('\'');
        return out;
    }
    out.push('"');
    for c in chars {
        match c {
            '\0' => out.push_str("`0"),
            '\u{7}' => out.push_str("`a"),
            '\u{8}' => out.push_str("`b"),
            '\u{1b}' => out.push_str("`e"),
            '\u{c}' => out.push_str("`f"),
            '\n' => out.push_str("`n"),
            '\r' => out.push_str("`r"),
            '\t' => out.push_str("`t"),
            '\u{b}' => out.push_str("`v"),
            c if is_control(c) => out.push_str(&format!("`u{{{:x}}}", c as u32)),
            c if c == '`' || c == '$' || powershell_double_quote(c) => {
                out.push('`');
                out.push(c);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn cmd(pieces: &[Piece]) -> String {
    let mut out = String::from("\"");
    for piece in pieces {
        match *piece {
            Piece::Char(c) if is_control(c) || c == '"' => {}
            Piece::Char(c) => out.push(c),
            Piece::Byte(_) => out.push(char::REPLACEMENT_CHARACTER),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quoted(path: &str, shell: ShellKind) -> String {
        quote_path(Path::new(path), shell)
    }

    #[test]
    fn plain_paths_stay_as_they_are() {
        for shell in ShellKind::ALL {
            assert_eq!(
                quoted("/work/repo/src/main.rs", shell),
                "/work/repo/src/main.rs"
            );
        }
        assert_eq!(quoted("a+b,c:d@e%f", ShellKind::Posix), "a+b,c:d@e%f");
        assert_eq!(quoted("a+b", ShellKind::PowerShell), "'a+b'");
        assert_eq!(
            quoted("C:\\Users\\me\\x.txt", ShellKind::PowerShell),
            "C:\\Users\\me\\x.txt"
        );
    }

    #[test]
    fn posix_quoting() {
        let shell = ShellKind::Posix;
        assert_eq!(quoted("a b", shell), "'a b'");
        assert_eq!(quoted("it's", shell), "'it'\\''s'");
        assert_eq!(quoted("'", shell), "\\'");
        assert_eq!(quoted("$HOME `id` *", shell), "'$HOME `id` *'");
        assert_eq!(quoted("a\nb", shell), "'a'$'\\n''b'");
        assert_eq!(quoted("\u{1b}[2J", shell), "$'\\033''[2J'");
        // C1 stays inside the quotes; C0 and DEL do not.
        assert_eq!(quoted("\u{85}a\u{9b}", shell), "'\u{85}a\u{9b}'");
        assert_eq!(quoted("\u{7f}", shell), "$'\\177'");
        assert_eq!(quoted("", shell), "''");
        assert_eq!(quoted("=cmd", shell), "'=cmd'");
        assert_eq!(quoted("~user", shell), "'~user'");
    }

    #[cfg(unix)]
    #[test]
    fn bytes_that_are_not_utf8_are_written_as_bytes() {
        use std::os::unix::ffi::OsStrExt as _;
        let path = Path::new(std::ffi::OsStr::from_bytes(b"a\xffb c"));
        assert_eq!(quote_path(path, ShellKind::Posix), "'a'$'\\377''b c'");
        assert_eq!(quote_path(path, ShellKind::Fish), "'a'\\Xff'b c'");
        assert_eq!(quote_path(path, ShellKind::PowerShell), "'a\u{fffd}b c'");
        assert_eq!(quote_path(path, ShellKind::Nushell), "\"a\u{fffd}b c\"");
        assert_eq!(quote_path(path, ShellKind::Csh), "'ab c'");
    }

    #[test]
    fn fish_quoting() {
        let shell = ShellKind::Fish;
        assert_eq!(quoted("a b", shell), "'a b'");
        assert_eq!(quoted("it's a\\b", shell), "'it\\'s a\\\\b'");
        assert_eq!(quoted("a\nb\u{1}", shell), "'a'\\n'b'\\x01");
        assert_eq!(quoted("\u{9b}", shell), "\\u009b");
        assert_eq!(quoted("$x (y) {z}", shell), "'$x (y) {z}'");
    }

    #[test]
    fn nushell_quoting() {
        let shell = ShellKind::Nushell;
        assert_eq!(quoted("a b", shell), "\"a b\"");
        assert_eq!(quoted("it's", shell), "\"it's\"");
        assert_eq!(quoted("say \"hi\" \\", shell), "\"say \\\"hi\\\" \\\\\"");
        assert_eq!(quoted("$x (y) `z` #", shell), "\"$x (y) `z` #\"");
        assert_eq!(quoted("a\nb\u{1}\u{9b}", shell), "\"a\\nb\\u{1}\\u{9b}\"");
        assert_eq!(quoted("a+b", shell), "\"a+b\"");
        assert_eq!(quoted("", shell), "\"\"");
        // Bare words that nushell would read as something else.
        assert_eq!(quoted("main", shell), "main");
        assert_eq!(quoted("1kb", shell), "\"1kb\"");
        assert_eq!(quoted("-n", shell), "\"-n\"");
        assert_eq!(quoted("null", shell), "\"null\"");
        assert_eq!(quoted("/tmp/1kb", shell), "/tmp/1kb");
    }

    #[test]
    fn csh_quoting() {
        let shell = ShellKind::Csh;
        assert_eq!(quoted("a b", shell), "'a b'");
        assert_eq!(quoted("it's", shell), "'it'\\''s'");
        assert_eq!(quoted("!x !!", shell), "\\!'x '\\!\\!");
        assert_eq!(quoted("$HOME `id` * ~", shell), "'$HOME `id` * ~'");
        assert_eq!(quoted("a\nb", shell), "'ab'");
        assert_eq!(quoted("%1", shell), "'%1'");
        assert_eq!(quoted("a+b,c:d@e", shell), "a+b,c:d@e");
        assert_eq!(quoted("", shell), "''");
        assert_eq!(quoted("\n", shell), "''");
    }

    #[test]
    fn powershell_quoting() {
        let shell = ShellKind::PowerShell;
        assert_eq!(quoted("a b", shell), "'a b'");
        assert_eq!(quoted("it's", shell), "'it''s'");
        assert_eq!(quoted("it\u{2019}s", shell), "'it\u{2019}\u{2019}s'");
        assert_eq!(quoted("$x `y`", shell), "'$x `y`'");
        assert_eq!(quoted("a\n$b\"`", shell), "\"a`n`$b`\"``\"");
        assert_eq!(quoted("\u{1}", shell), "\"`u{1}\"");
    }

    #[test]
    fn cmd_quoting() {
        let shell = ShellKind::Cmd;
        assert_eq!(quoted("C:\\My Files\\a&b", shell), "\"C:\\My Files\\a&b\"");
        assert_eq!(quoted("a\"b\nc", shell), "\"abc\"");
    }

    #[test]
    fn several_paths_are_separated_by_a_space() {
        let paths = [
            PathBuf::from("/a b"),
            PathBuf::from("/c"),
            PathBuf::from(""),
        ];
        assert_eq!(quote_paths(&paths, ShellKind::Posix), "'/a b' /c ''");
        assert_eq!(quote_paths(&[], ShellKind::Posix), "");
    }
}
