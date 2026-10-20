//! The mouse, reported to a program that asked for it.
//!
//! A program asks with one of the tracking modes: presses and releases
//! (1000), those and motion while a button is held (1002), or all motion
//! (1003); and says how coordinates are written: as single bytes (the
//! default, up to column and row 223), as UTF-8 (1005, up to 2015), or as
//! decimal numbers (SGR, 1006, no limit). A button is 0 for the left, 1
//! the middle, 2 the right; 3 is a release in the forms that do not say
//! which; the wheel is 64 up and 65 down; motion adds 32; Shift adds 4,
//! Alt (Meta) 8 and Control 16.
//!
//! Shift held with the mouse is the user's own: the press is not
//! reported, so text can be selected in a program that tracks the mouse,
//! as in xterm.
//!
//! Sources: XTerm Control Sequences, "Mouse Tracking"
//! (invisible-island.net/xterm/ctlseqs/ctlseqs.html).

use crate::{Modifiers, ViewPoint, frame::Modes, input::Key};

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseButton {
    /// The left (primary) button.
    Left,
    /// The middle button or the wheel pressed.
    Middle,
    /// The right (secondary) button.
    Right,
}

/// What the mouse did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MouseKind {
    /// A button went down.
    Press(MouseButton),
    /// A button came up.
    Release(MouseButton),
    /// The mouse moved, with this button held or none.
    Move {
        /// The button held while it moved.
        held: Option<MouseButton>,
    },
    /// The wheel turned by whole lines: up (towards history) when
    /// positive.
    Wheel {
        /// Lines; up when positive.
        lines: i32,
    },
}

/// A mouse event over a cell of the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Mouse {
    /// What happened.
    pub kind: MouseKind,
    /// Over which cell of what is on show.
    pub point: ViewPoint,
    /// The modifiers held.
    pub modifiers: Modifiers,
}

/// How many wheel lines at most are sent for one turn: a hostile delta
/// does not flood the program.
const MOST_WHEEL_LINES: i32 = 100;

/// What a mouse event sends to the program in these modes: a report, the
/// arrow keys the wheel stands for in the alternate screen, or none when
/// the event is the window's to handle (selection, scrolling history).
pub(crate) fn encode(event: &Mouse, modes: &Modes) -> Option<Vec<u8>> {
    let tracking = modes.mouse_reporting;
    if let MouseKind::Wheel { lines } = event.kind {
        if lines == 0 {
            return None;
        }
        let count = lines.unsigned_abs().min(MOST_WHEEL_LINES.unsigned_abs());
        if tracking && !event.modifiers.shift {
            let button = if lines > 0 { 64 } else { 65 };
            let one = report(button, event, modes, true)?;
            return Some(one.repeat(count as usize));
        }
        if modes.alternate_screen && modes.alternate_scroll && !event.modifiers.any() {
            let key = if lines > 0 { Key::Up } else { Key::Down };
            let one = crate::input::encode(
                &crate::Input::Key(key, Modifiers::NONE),
                &Modes {
                    application_cursor: modes.application_cursor,
                    ..Modes::default()
                },
            )?;
            return Some(one.repeat(count as usize));
        }
        return None;
    }
    if !tracking || event.modifiers.shift {
        return None;
    }
    match event.kind {
        MouseKind::Press(button) => report(code(button), event, modes, true),
        MouseKind::Release(button) => {
            if modes.sgr_mouse {
                report(code(button), event, modes, false)
            } else {
                report(3, event, modes, false)
            }
        }
        MouseKind::Move { held } => {
            let reported = match held {
                Some(_) => modes.mouse_drag || modes.mouse_motion,
                None => modes.mouse_motion,
            };
            if !reported {
                return None;
            }
            // Motion with no button held is "release" plus motion.
            let button = held.map_or(3, code);
            report(button + 32, event, modes, true)
        }
        MouseKind::Wheel { .. } => None,
    }
}

fn code(button: MouseButton) -> u8 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
    }
}

/// One report of `button` (with motion and wheel bits already in it) at
/// the event's cell; none when the cell cannot be written in the
/// program's encoding.
fn report(button: u8, event: &Mouse, modes: &Modes, pressed: bool) -> Option<Vec<u8>> {
    let modifiers = event.modifiers;
    let button = button
        + 4 * u8::from(modifiers.shift)
        + 8 * u8::from(modifiers.alt)
        + 16 * u8::from(modifiers.control);
    let column = u32::from(event.point.column) + 1;
    let row = u32::from(event.point.row) + 1;
    if modes.sgr_mouse {
        let end = if pressed { 'M' } else { 'm' };
        return Some(format!("\x1b[<{button};{column};{row}{end}").into_bytes());
    }
    let mut out = vec![0x1b, b'[', b'M'];
    for value in [u32::from(button), column, row] {
        let value = value.checked_add(32)?;
        if modes.utf8_mouse {
            if value > 2047 {
                return None;
            }
            let c = char::from_u32(value)?;
            let mut buffer = [0u8; 4];
            out.extend_from_slice(c.encode_utf8(&mut buffer).as_bytes());
        } else {
            out.push(u8::try_from(value).ok()?);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(kind: MouseKind, column: u16, row: u16) -> Mouse {
        Mouse {
            kind,
            point: ViewPoint { row, column },
            modifiers: Modifiers::NONE,
        }
    }

    fn tracking() -> Modes {
        Modes {
            mouse_reporting: true,
            ..Modes::default()
        }
    }

    #[test]
    fn nothing_is_sent_unless_the_program_asked() {
        let press = at(MouseKind::Press(MouseButton::Left), 0, 0);
        assert_eq!(encode(&press, &Modes::default()), None);
        let wheel = at(MouseKind::Wheel { lines: 3 }, 0, 0);
        assert_eq!(encode(&wheel, &Modes::default()), None);
    }

    #[test]
    fn presses_and_releases_in_each_encoding() {
        let press = at(MouseKind::Press(MouseButton::Left), 4, 9);
        let release = at(MouseKind::Release(MouseButton::Left), 4, 9);
        assert_eq!(encode(&press, &tracking()).unwrap(), b"\x1b[M\x20\x25\x2a");
        assert_eq!(
            encode(&release, &tracking()).unwrap(),
            b"\x1b[M\x23\x25\x2a"
        );
        let sgr = Modes {
            sgr_mouse: true,
            ..tracking()
        };
        assert_eq!(encode(&press, &sgr).unwrap(), b"\x1b[<0;5;10M");
        assert_eq!(encode(&release, &sgr).unwrap(), b"\x1b[<0;5;10m");
        let right = at(MouseKind::Press(MouseButton::Right), 0, 0);
        let mut control = right;
        control.modifiers = Modifiers::CONTROL;
        assert_eq!(encode(&control, &sgr).unwrap(), b"\x1b[<18;1;1M");
        // Too far out for single bytes; fine in UTF-8 and SGR.
        let far = at(MouseKind::Press(MouseButton::Left), 300, 0);
        assert_eq!(encode(&far, &tracking()), None);
        let utf8 = Modes {
            utf8_mouse: true,
            ..tracking()
        };
        let mut expected = b"\x1b[M\x20".to_vec();
        expected.extend_from_slice("\u{14d}".as_bytes());
        expected.push(b'!');
        assert_eq!(encode(&far, &utf8).unwrap(), expected);
        assert_eq!(encode(&far, &sgr).unwrap(), b"\x1b[<0;301;1M");
        let farthest = at(MouseKind::Press(MouseButton::Left), u16::MAX, u16::MAX);
        assert_eq!(encode(&farthest, &utf8), None);
        assert!(encode(&farthest, &sgr).is_some());
    }

    #[test]
    fn motion_only_in_the_modes_that_ask_for_it() {
        let drag = at(
            MouseKind::Move {
                held: Some(MouseButton::Left),
            },
            1,
            1,
        );
        let hover = at(MouseKind::Move { held: None }, 1, 1);
        let sgr = Modes {
            sgr_mouse: true,
            ..tracking()
        };
        assert_eq!(encode(&drag, &sgr), None);
        let dragging = Modes {
            mouse_drag: true,
            ..sgr
        };
        assert_eq!(encode(&drag, &dragging).unwrap(), b"\x1b[<32;2;2M");
        assert_eq!(encode(&hover, &dragging), None);
        let moving = Modes {
            mouse_motion: true,
            ..sgr
        };
        assert_eq!(encode(&hover, &moving).unwrap(), b"\x1b[<35;2;2M");
    }

    #[test]
    fn the_wheel_reports_scrolls_or_is_left_to_the_window() {
        let up = at(MouseKind::Wheel { lines: 2 }, 0, 0);
        let down = at(MouseKind::Wheel { lines: -1 }, 0, 0);
        let sgr = Modes {
            sgr_mouse: true,
            ..tracking()
        };
        assert_eq!(encode(&up, &sgr).unwrap(), b"\x1b[<64;1;1M\x1b[<64;1;1M");
        assert_eq!(encode(&down, &sgr).unwrap(), b"\x1b[<65;1;1M");
        let alternate = Modes {
            alternate_screen: true,
            alternate_scroll: true,
            ..Modes::default()
        };
        assert_eq!(encode(&up, &alternate).unwrap(), b"\x1b[A\x1b[A");
        let application = Modes {
            application_cursor: true,
            ..alternate
        };
        assert_eq!(encode(&down, &application).unwrap(), b"\x1bOB");
        let huge = at(MouseKind::Wheel { lines: i32::MIN }, 0, 0);
        assert_eq!(encode(&huge, &alternate).unwrap().len(), 300);
        assert_eq!(encode(&at(MouseKind::Wheel { lines: 0 }, 0, 0), &sgr), None);
    }

    #[test]
    fn shift_keeps_the_mouse_for_the_window() {
        let mut press = at(MouseKind::Press(MouseButton::Left), 0, 0);
        press.modifiers = Modifiers::SHIFT;
        assert_eq!(encode(&press, &tracking()), None);
    }
}
