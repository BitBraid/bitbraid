//! Keys and text to the bytes a program on a terminal reads, as xterm
//! sends them.
//!
//! - Text is sent as UTF-8.
//! - Ctrl with a letter sends its control character (Ctrl+A is 0x01), and
//!   with the marks around the letters the rest of C0 (Ctrl+Space and
//!   Ctrl+2 NUL, Ctrl+[ ESC, Ctrl+\ FS, Ctrl+] GS, Ctrl+^ RS, Ctrl+_ and
//!   Ctrl+/ US, Ctrl+? DEL), as on a VT220 keyboard.
//! - Alt puts ESC in front of what the key sends otherwise.
//! - The cursor keys send `CSI A`..`CSI D`, or `SS3 A`..`SS3 D` while the
//!   program asked for application cursor keys (DECCKM); Home and End
//!   likewise with `H` and `F`. With a modifier they send `CSI 1 ; m X`,
//!   where `m` is 1 plus 1 for Shift, 2 for Alt, 4 for Ctrl and 8 for the
//!   platform key.
//! - Insert, Delete, Page Up and Page Down send `CSI n ~`, F1 to F4
//!   `SS3 P`..`SS3 S`, F5 to F20 `CSI n ~`; with a modifier
//!   `CSI n ; m ~` (`CSI 1 ; m P` for F1 to F4).
//! - Shift+Tab sends `CSI Z` (back tab); Backspace DEL, Ctrl+Backspace BS.
//! - The keypad sends its characters, or `SS3 p`..`SS3 y` and the like
//!   while the program asked for the application keypad (DECKPAM).
//!
//! Sources: XTerm Control Sequences, "PC-Style Function Keys", "VT220-Style
//! Function Keys", "VT100-Style Function Keys" and "Alt and Meta Keys"
//! (invisible-island.net/xterm/ctlseqs/ctlseqs.html); the xterm manual,
//! `modifyCursorKeys` and `modifyFunctionKeys`.

use crate::frame::Modes;

/// Modifier keys held with a key or the mouse.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    /// Alt (Option on a Mac), sent as Meta: ESC in front.
    pub alt: bool,
    /// The platform key (Super, Windows, Command).
    pub platform: bool,
}

impl Modifiers {
    /// None held.
    pub const NONE: Self = Self {
        shift: false,
        control: false,
        alt: false,
        platform: false,
    };

    /// Only Control.
    pub const CONTROL: Self = Self {
        shift: false,
        control: true,
        alt: false,
        platform: false,
    };

    /// Only Shift.
    pub const SHIFT: Self = Self {
        shift: true,
        control: false,
        alt: false,
        platform: false,
    };

    /// Only Alt.
    pub const ALT: Self = Self {
        shift: false,
        control: false,
        alt: true,
        platform: false,
    };

    /// Whether any is held.
    pub fn any(self) -> bool {
        self.shift || self.control || self.alt || self.platform
    }

    /// The parameter xterm sends for these modifiers: 1 plus 1 for Shift,
    /// 2 for Alt, 4 for Control and 8 for the platform key.
    pub fn parameter(self) -> u8 {
        1 + u8::from(self.shift)
            + 2 * u8::from(self.alt)
            + 4 * u8::from(self.control)
            + 8 * u8::from(self.platform)
    }
}

/// A key on the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// A key that types a character: the character it types with the
    /// modifiers that are held (`A` with Shift), or the one on the key
    /// when Control is held.
    Character(char),
    /// Return.
    Enter,
    Tab,
    Backspace,
    Escape,
    /// Arrow up.
    Up,
    /// Arrow down.
    Down,
    /// Arrow left.
    Left,
    /// Arrow right.
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    /// Delete (forward).
    Delete,
    /// F1 to F20.
    Function(u8),
    /// A key of the numeric keypad: `0`..`9`, `.`, `+`, `-`, `*`, `/`,
    /// `=`, or `\r` for its Enter.
    Keypad(char),
}

impl Key {
    /// The key a name stands for: a single character, or a name of the
    /// kind toolkits use (`enter`, `tab`, `backspace`, `escape`, `up`,
    /// `down`, `left`, `right`, `home`, `end`, `pageup`, `pagedown`,
    /// `insert`, `delete`, `space`, `f1`..`f20`). None for any other.
    pub fn named(name: &str) -> Option<Self> {
        let mut chars = name.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            return Some(Self::Character(c));
        }
        Some(match name.to_ascii_lowercase().as_str() {
            "enter" | "return" => Self::Enter,
            "tab" => Self::Tab,
            "backspace" => Self::Backspace,
            "escape" | "esc" => Self::Escape,
            "up" => Self::Up,
            "down" => Self::Down,
            "left" => Self::Left,
            "right" => Self::Right,
            "home" => Self::Home,
            "end" => Self::End,
            "pageup" | "page_up" => Self::PageUp,
            "pagedown" | "page_down" => Self::PageDown,
            "insert" => Self::Insert,
            "delete" | "del" => Self::Delete,
            "space" => Self::Character(' '),
            other => {
                let number: u8 = other.strip_prefix('f')?.parse().ok()?;
                if !(1..=20).contains(&number) {
                    return None;
                }
                Self::Function(number)
            }
        })
    }
}

/// What is typed into a terminal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Input {
    /// A key with the modifiers held.
    Key(Key, Modifiers),
    /// Text, as an input method commits it: sent as it is.
    Text(String),
}

const ESC: u8 = 0x1b;

/// The bytes an input sends, in the modes the program set. None for a
/// key that sends nothing (F21, Ctrl with a key that has no control
/// character, the platform key with a character).
pub fn encode(input: &Input, modes: &Modes) -> Option<Vec<u8>> {
    match input {
        Input::Text(text) => (!text.is_empty()).then(|| text.as_bytes().to_vec()),
        Input::Key(key, modifiers) => encode_key(*key, *modifiers, modes),
    }
}

fn csi(body: &str) -> Vec<u8> {
    let mut out = vec![ESC, b'['];
    out.extend_from_slice(body.as_bytes());
    out
}

fn ss3(final_byte: u8) -> Vec<u8> {
    vec![ESC, b'O', final_byte]
}

fn with_alt(mut bytes: Vec<u8>, alt: bool) -> Vec<u8> {
    if alt {
        bytes.insert(0, ESC);
    }
    bytes
}

/// The control character Ctrl with this character sends.
fn control_character(c: char) -> Option<u8> {
    let c = c.to_ascii_lowercase();
    Some(match c {
        'a'..='z' => (c as u8) - b'a' + 1,
        ' ' | '@' | '2' | '`' => 0,
        '[' | '3' | '{' => ESC,
        '\\' | '4' | '|' => 0x1c,
        ']' | '5' | '}' => 0x1d,
        '^' | '6' | '~' => 0x1e,
        '_' | '7' | '/' | '-' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

fn encode_key(key: Key, modifiers: Modifiers, modes: &Modes) -> Option<Vec<u8>> {
    let parameter = modifiers.parameter();
    // Cursor keys and Home and End: `CSI X`, `SS3 X`, or `CSI 1 ; m X`.
    let cursor = |final_byte: u8| -> Vec<u8> {
        if parameter > 1 {
            csi(&format!("1;{parameter}{}", final_byte as char))
        } else if modes.application_cursor {
            ss3(final_byte)
        } else {
            vec![ESC, b'[', final_byte]
        }
    };
    // Keys of the editing pad and F5 up: `CSI n ~` or `CSI n ; m ~`.
    let tilde = |number: u8| -> Vec<u8> {
        if parameter > 1 {
            csi(&format!("{number};{parameter}~"))
        } else {
            csi(&format!("{number}~"))
        }
    };
    Some(match key {
        Key::Character(c) => {
            if modifiers.platform {
                return None;
            }
            if modifiers.control {
                let control = control_character(c)?;
                with_alt(vec![control], modifiers.alt)
            } else {
                let mut buffer = [0u8; 4];
                with_alt(
                    c.encode_utf8(&mut buffer).as_bytes().to_vec(),
                    modifiers.alt,
                )
            }
        }
        Key::Enter => {
            let bytes = if modes.line_feed_new_line {
                vec![b'\r', b'\n']
            } else {
                vec![b'\r']
            };
            with_alt(bytes, modifiers.alt)
        }
        Key::Tab if modifiers.shift => csi("Z"),
        Key::Tab => with_alt(vec![b'\t'], modifiers.alt),
        Key::Backspace => {
            let byte = if modifiers.control { 0x08 } else { 0x7f };
            with_alt(vec![byte], modifiers.alt)
        }
        Key::Escape => with_alt(vec![ESC], modifiers.alt),
        Key::Up => cursor(b'A'),
        Key::Down => cursor(b'B'),
        Key::Right => cursor(b'C'),
        Key::Left => cursor(b'D'),
        Key::Home => cursor(b'H'),
        Key::End => cursor(b'F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::Function(number @ 1..=4) => {
            let final_byte = b'P' + (number - 1);
            if parameter > 1 {
                csi(&format!("1;{parameter}{}", final_byte as char))
            } else {
                ss3(final_byte)
            }
        }
        Key::Function(number) => {
            let code = match number {
                5 => 15,
                6 => 17,
                7 => 18,
                8 => 19,
                9 => 20,
                10 => 21,
                11 => 23,
                12 => 24,
                13 => 25,
                14 => 26,
                15 => 28,
                16 => 29,
                17 => 31,
                18 => 32,
                19 => 33,
                20 => 34,
                _ => return None,
            };
            tilde(code)
        }
        Key::Keypad(c) => {
            if modes.application_keypad {
                let final_byte = match c {
                    '0'..='9' => b'p' + (c as u8 - b'0'),
                    '.' => b'n',
                    '+' => b'k',
                    '-' => b'm',
                    '*' => b'j',
                    '/' => b'o',
                    '=' => b'X',
                    '\r' | '\n' => b'M',
                    _ => return None,
                };
                ss3(final_byte)
            } else {
                match c {
                    '\r' | '\n' => return encode_key(Key::Enter, modifiers, modes),
                    '0'..='9' | '.' | '+' | '-' | '*' | '/' | '=' => {
                        with_alt(vec![c as u8], modifiers.alt)
                    }
                    _ => return None,
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes(key: Key, modifiers: Modifiers) -> Vec<u8> {
        encode(&Input::Key(key, modifiers), &Modes::default()).unwrap()
    }

    #[test]
    fn characters_control_and_alt() {
        assert_eq!(bytes(Key::Character('a'), Modifiers::NONE), b"a");
        assert_eq!(bytes(Key::Character('é'), Modifiers::NONE), "é".as_bytes());
        assert_eq!(bytes(Key::Character('c'), Modifiers::CONTROL), [3]);
        assert_eq!(bytes(Key::Character('C'), Modifiers::CONTROL), [3]);
        assert_eq!(bytes(Key::Character('u'), Modifiers::CONTROL), [0x15]);
        assert_eq!(bytes(Key::Character(' '), Modifiers::CONTROL), [0]);
        assert_eq!(bytes(Key::Character('['), Modifiers::CONTROL), [0x1b]);
        assert_eq!(bytes(Key::Character('/'), Modifiers::CONTROL), [0x1f]);
        assert_eq!(bytes(Key::Character('b'), Modifiers::ALT), b"\x1bb");
        let both = Modifiers {
            control: true,
            alt: true,
            ..Modifiers::NONE
        };
        assert_eq!(bytes(Key::Character('x'), both), b"\x1b\x18");
        assert_eq!(
            encode(
                &Input::Key(Key::Character('1'), Modifiers::CONTROL),
                &Modes::default()
            ),
            None
        );
        let platform = Modifiers {
            platform: true,
            ..Modifiers::NONE
        };
        assert_eq!(
            encode(
                &Input::Key(Key::Character('c'), platform),
                &Modes::default()
            ),
            None
        );
    }

    #[test]
    fn cursor_keys_follow_the_mode_and_the_modifiers() {
        assert_eq!(bytes(Key::Up, Modifiers::NONE), b"\x1b[A");
        assert_eq!(bytes(Key::Home, Modifiers::NONE), b"\x1b[H");
        let application = Modes {
            application_cursor: true,
            ..Modes::default()
        };
        let up = Input::Key(Key::Up, Modifiers::NONE);
        assert_eq!(encode(&up, &application).unwrap(), b"\x1bOA");
        assert_eq!(bytes(Key::Right, Modifiers::CONTROL), b"\x1b[1;5C");
        assert_eq!(bytes(Key::Left, Modifiers::SHIFT), b"\x1b[1;2D");
        assert_eq!(bytes(Key::End, Modifiers::ALT), b"\x1b[1;3F");
        let ctrl_up = Input::Key(Key::Up, Modifiers::CONTROL);
        assert_eq!(encode(&ctrl_up, &application).unwrap(), b"\x1b[1;5A");
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(bytes(Key::Delete, Modifiers::NONE), b"\x1b[3~");
        assert_eq!(bytes(Key::PageUp, Modifiers::SHIFT), b"\x1b[5;2~");
        assert_eq!(bytes(Key::Function(1), Modifiers::NONE), b"\x1bOP");
        assert_eq!(bytes(Key::Function(4), Modifiers::CONTROL), b"\x1b[1;5S");
        assert_eq!(bytes(Key::Function(5), Modifiers::NONE), b"\x1b[15~");
        assert_eq!(bytes(Key::Function(12), Modifiers::SHIFT), b"\x1b[24;2~");
        assert_eq!(bytes(Key::Function(20), Modifiers::NONE), b"\x1b[34~");
        for odd in [0, 21, 255] {
            assert_eq!(
                encode(
                    &Input::Key(Key::Function(odd), Modifiers::NONE),
                    &Modes::default()
                ),
                None
            );
        }
        assert_eq!(bytes(Key::Tab, Modifiers::SHIFT), b"\x1b[Z");
        assert_eq!(bytes(Key::Backspace, Modifiers::NONE), [0x7f]);
        assert_eq!(bytes(Key::Backspace, Modifiers::CONTROL), [0x08]);
        assert_eq!(bytes(Key::Backspace, Modifiers::ALT), b"\x1b\x7f");
        assert_eq!(bytes(Key::Enter, Modifiers::NONE), b"\r");
        assert_eq!(bytes(Key::Escape, Modifiers::NONE), b"\x1b");
    }

    #[test]
    fn the_keypad_follows_its_mode() {
        assert_eq!(bytes(Key::Keypad('5'), Modifiers::NONE), b"5");
        let application = Modes {
            application_keypad: true,
            ..Modes::default()
        };
        let five = Input::Key(Key::Keypad('5'), Modifiers::NONE);
        assert_eq!(encode(&five, &application).unwrap(), b"\x1bOu");
        let enter = Input::Key(Key::Keypad('\r'), Modifiers::NONE);
        assert_eq!(encode(&enter, &application).unwrap(), b"\x1bOM");
        assert_eq!(encode(&enter, &Modes::default()).unwrap(), b"\r");
    }

    #[test]
    fn names_of_keys() {
        assert_eq!(Key::named("a"), Some(Key::Character('a')));
        assert_eq!(Key::named("space"), Some(Key::Character(' ')));
        assert_eq!(Key::named("enter"), Some(Key::Enter));
        assert_eq!(Key::named("f12"), Some(Key::Function(12)));
        assert_eq!(Key::named("f0"), None);
        assert_eq!(Key::named("f99999"), None);
        assert_eq!(Key::named("shift"), None);
        assert_eq!(Key::named(""), None);
    }

    #[test]
    fn text_is_sent_as_it_is() {
        let text = Input::Text("grüße 👋".into());
        assert_eq!(
            encode(&text, &Modes::default()).unwrap(),
            "grüße 👋".as_bytes()
        );
        assert_eq!(encode(&Input::Text(String::new()), &Modes::default()), None);
    }
}
