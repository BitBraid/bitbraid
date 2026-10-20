//! bitbraid-terminal: a shell inside the window.
//!
//! A [`Session`] runs a shell on a pseudo-terminal: `alacritty_terminal`
//! parses what it writes into a grid of cells on a thread of its own, and
//! this crate turns keys, pasted text and the mouse into the bytes the
//! shell reads ([`input`], [`mouse`]), and the grid into a [`Frame`] of
//! what is on show with every colour worked out ([`frame`]). Paths are
//! written the way a shell reads them by [`quote_paths`]. The command
//! line being typed at the shell's prompt is read off the screen for
//! completion ([`line`]).
//!
//! Nothing here knows the toolkit or git (`AGENTS.md` section 2): the
//! window draws a frame and hands keys over; the engine decides which
//! shell is started where (`terminal.shell`).
//!
//! Everything the shell writes is taken for hostile: sizes, titles,
//! clipboard requests and colours are bounded and cleaned here before
//! anything else sees them.
//!
//! Sources: the documentation and source of `alacritty_terminal` 0.26
//! (Apache-2.0); XTerm Control Sequences
//! (invisible-island.net/xterm/ctlseqs/ctlseqs.html); POSIX Shell Command
//! Language, 2.2 Quoting; the fish documentation, "Quotes" and "Escaping
//! characters"; the PowerShell documentation, about_Quoting_Rules and
//! about_Special_Characters.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

// `say!` and `message!`, in every module: the words come from the catalog.
#[macro_use]
extern crate bitbraid_strings;

pub mod contrast;
pub mod frame;
pub mod input;
pub mod line;
pub mod mouse;
pub mod quote;
pub mod session;
pub mod shell;
pub mod strings;

pub use frame::{
    Cell, CellWidth, Cursor, CursorShape, Frame, Modes, Palette, Rgb, Row, Run, Span, Style,
    Underline,
};
pub use input::{Input, Key, Modifiers};
pub use line::{Prompt, TypedLine};
pub use mouse::{Mouse, MouseButton, MouseKind};
pub use quote::{quote_path, quote_paths};
pub use session::{Error, ExitStatus, SelectOp, Session, SessionEvent, Side, ViewPoint};
pub use shell::{GridSize, ShellKind, ShellSpec};
