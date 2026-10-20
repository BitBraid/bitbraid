//! The text of a terminal: what a shell has on show, drawn cell by cell,
//! and the keys, the mouse and dropped files handed to it.
//!
//! [`TerminalText`] shows one [`Session`] of `bitbraid-terminal`. When it
//! is drawn it asks the session for a [`term::Frame`] (the screen with
//! every colour worked out) and an element of its own paints it: the
//! backgrounds in spans of one colour, the text of each row shaped as
//! one line in runs of one style with every glyph set on its cell
//! (`force_width`), a wide character on its own over two cells, the
//! cursor and the text an input method is composing. Shaped lines are
//! kept by the toolkit from one frame to the next, so rows that did not
//! change, and rows that only moved up, are not shaped again.
//!
//! The session says when its screen changed at most once until the next
//! frame is asked for, so a stream of output redraws the view once per
//! frame of the window and no more.
//!
//! Keys: while the text has the focus every key goes to the shell, before
//! the window's own bindings are looked at. The toolkit runs keystroke
//! interceptors before it matches a key to a binding; the one this view
//! installs takes the key and stops it there. What it leaves to the
//! window and what it does itself instead of typing are the two short
//! lists [`WINDOW_KEYS`] and [`own_keys`], read by [`key_role`] and
//! nowhere else.
//!
//! While a git command is typed at the shell's prompt, the text offers
//! completions for it: every key typed passes [`completion`] first, which
//! may take it, and the list it shows is drawn over the window from here.
//!
//! [`completion`]: super::completion
//!
//! Sources: the toolkit's sources (gpui-pre 0.3.8: `Window::dispatch_key_event`
//! and `App::intercept_keystrokes` for the order in which a key is
//! offered; `WindowTextSystem::shape_line` and its `force_width`;
//! `EntityInputHandler`; `Interactivity::on_drop`); the documentation of
//! `alacritty_terminal` 0.26; XTerm Control Sequences
//! (invisible-island.net/xterm/ctlseqs/ctlseqs.html) for what the mouse
//! and the wheel do in a terminal.

use std::{ops::Range, path::PathBuf, rc::Rc, sync::Arc, time::Duration};

use bitbraid_terminal::{self as term, Session, SessionEvent};
use futures::{StreamExt as _, channel::mpsc};
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::completion::{self, Completer, KeyUse, ListHandlers, Settings, Shown};
use crate::views::drag_paths::DraggedPaths;
// Measures of ours are points, drawn at the window's zoom.
use crate::zoom::px;

/// Where keys of the text apply, for bindings that want it.
pub const KEY_CONTEXT: &str = "TerminalText";

/// Room left of the first column, in points at 100 %: with the panel's
/// border line, which is outside this view, the first column is 13 in.
pub const PADDING_LEFT: f32 = 12.0;
/// Room above the first row: 12 under the rule below the panel's header.
pub const PADDING_TOP: f32 = 12.0;
/// Room right of the last column, kept for the scroll position.
const PADDING_RIGHT: f32 = 10.0;

/// How long the output stays quiet after Enter before the command it ran
/// counts as finished ([`TerminalTextEvent::CommandFinished`]).
pub const QUIET: Duration = Duration::from_millis(400);

/// How strongly text and the cursor show while the text does not have the
/// keys and the look says to dim it.
const UNFOCUSED_OPACITY: f32 = 0.6;

/// The width of the bar cursor and of the hollow cursor's outline.
const CURSOR_LINE: f32 = 1.0;
/// The height of the underline cursor.
const CURSOR_UNDERLINE: f32 = 2.0;

/// Blanks between two pieces of text of a row that are shaped with them as
/// one line rather than ending it: fewer, longer lines to shape.
const BRIDGED_BLANKS: usize = 4;

/// Bindings whose keys the window keeps while the text has the focus, by
/// their id in `keymap::BINDINGS`, so that keys the user gave them count.
/// They are the panel's own: showing and hiding it, another terminal,
/// the next or the previous one, ending one.
pub const WINDOW_KEYS: &[&str] = &super::PANEL_IDS;

/// Keys the text does something with itself instead of typing them.
pub const OWN_KEYS: &[(&str, OwnKey)] = &[
    ("ctrl-c", OwnKey::CopyOrInterrupt),
    ("ctrl-shift-c", OwnKey::Copy),
    ("ctrl-shift-v", OwnKey::Paste),
];

/// The same on macOS, as every terminal there has them: Cmd+C copies and
/// Cmd+V pastes, and Ctrl+C always reaches the shell.
pub const OWN_KEYS_ON_MAC: &[(&str, OwnKey)] = &[("cmd-c", OwnKey::Copy), ("cmd-v", OwnKey::Paste)];

/// The keys the text keeps, on the system in force.
pub fn own_keys() -> &'static [(&'static str, OwnKey)] {
    match crate::keymap::System::now() {
        crate::keymap::System::Mac => OWN_KEYS_ON_MAC,
        crate::keymap::System::Other => OWN_KEYS,
    }
}

/// What one of [`own_keys`] does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OwnKey {
    /// Copy the selection.
    Copy,
    /// Copy the selection and let it go when there is one; else the key
    /// goes to the shell, which stops what runs.
    CopyOrInterrupt,
    /// Paste the clipboard.
    Paste,
}

/// The shape of the cursor when the program did not ask for one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum CursorStyle {
    /// A thin line before the cell.
    #[default]
    Bar,
    /// The whole cell.
    Block,
    /// A line under the cell.
    Underline,
}

/// How a terminal's text looks.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalLook {
    /// The font; empty for the theme's font of code.
    pub font_family: SharedString,
    /// The size of the font, in points.
    pub font_size: f32,
    /// The height of a row, times the size of the font.
    pub line_height: f32,
    /// The cursor's shape when the program did not ask for one.
    pub cursor: CursorStyle,
    /// Whether text and cursor dim while the text does not have the keys.
    pub dim_unfocused: bool,
    /// The colours.
    pub palette: term::Palette,
}

impl Default for TerminalLook {
    fn default() -> Self {
        Self {
            font_family: SharedString::default(),
            font_size: 12.0,
            line_height: 1.2,
            cursor: CursorStyle::Bar,
            dim_unfocused: true,
            palette: term::Palette::default(),
        }
    }
}

/// What a terminal's text tells its owner.
#[derive(Debug, Clone, PartialEq)]
pub enum TerminalTextEvent {
    /// The program set the title.
    Title(String),
    /// The shell ended, with its exit code when it exited by itself.
    Exited(Option<i32>),
    /// Enter was typed and the output has been quiet for [`QUIET`] since:
    /// a command has probably finished, and what it changed can be read.
    CommandFinished,
    /// The other button of the pointer was pressed here, in window
    /// coordinates, and the program did not ask for the mouse.
    ContextMenu(Point<Pixels>),
}

/// Where the cells are, as the last frame laid them out.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Geometry {
    /// The top left corner of the first cell, in the window.
    origin: Point<Pixels>,
    /// The size of one cell.
    cell: Size<Pixels>,
    /// Cells across.
    columns: u16,
    /// Rows of cells.
    rows: u16,
}

impl Geometry {
    /// The cell under a place in the window, and the half of it, held to
    /// the screen.
    fn cell_at(&self, position: Point<Pixels>) -> (term::ViewPoint, term::Side) {
        let width = f32::from(self.cell.width).max(1.0);
        let height = f32::from(self.cell.height).max(1.0);
        let x = f32::from(position.x - self.origin.x) / width;
        let y = f32::from(position.y - self.origin.y) / height;
        let last_column = self.columns.saturating_sub(1);
        let last_row = self.rows.saturating_sub(1);
        let (column, side) = if !x.is_finite() || x < 0.0 {
            (0, term::Side::Left)
        } else if x >= f32::from(self.columns) {
            (last_column, term::Side::Right)
        } else {
            let side = if x.fract() >= 0.5 {
                term::Side::Right
            } else {
                term::Side::Left
            };
            // In range: checked just above.
            ((x.floor() as u16).min(last_column), side)
        };
        let row = if !y.is_finite() || y < 0.0 {
            0
        } else {
            (y.floor() as u16).min(last_row)
        };
        (term::ViewPoint { row, column }, side)
    }

    /// The rows a place is above the first row (positive) or below the
    /// last (negative); 0 on the screen.
    fn rows_outside(&self, position: Point<Pixels>) -> i32 {
        let top = self.origin.y;
        let bottom = top + self.cell.height * f32::from(self.rows);
        if position.y < top {
            1
        } else if position.y >= bottom {
            -1
        } else {
            0
        }
    }

    /// The bounds of `cells` cells from a cell on.
    fn bounds(&self, row: u16, column: u16, cells: u16) -> Bounds<Pixels> {
        Bounds {
            origin: point(
                self.origin.x + self.cell.width * f32::from(column),
                self.origin.y + self.cell.height * f32::from(row),
            ),
            size: size(self.cell.width * f32::from(cells.max(1)), self.cell.height),
        }
    }
}

/// A terminal's text: one session on show.
pub struct TerminalText {
    session: Session,
    look: TerminalLook,
    focus_handle: FocusHandle,
    /// The size the session was given last.
    size: Option<term::GridSize>,
    /// Where the cells were in the last frame.
    geometry: Option<Geometry>,
    /// Where the text was in the last frame, in the window.
    area: Option<Bounds<Pixels>>,
    /// The completions of the git command being typed.
    completer: Completer,
    /// Where the cursor was in the last frame.
    cursor_at: Option<(u16, u16)>,
    /// The left button went down on the text and selects while it is held.
    selecting: bool,
    /// The program was told of a press and hears the moves and the release.
    reporting: Option<term::MouseButton>,
    /// The part of a line the wheel has turned that is not a whole line yet.
    wheel_rest: f32,
    /// Text an input method is composing, drawn at the cursor.
    marked: Option<String>,
    /// Enter was typed: the output is waited on to go quiet.
    command_running: bool,
    /// Runs out [`QUIET`] after the last output following Enter.
    quiet: Option<Task<()>>,
    /// The shell ended.
    exited: bool,
    events: Option<Task<()>>,
    /// Frames drawn so far, for the measuring run.
    frames: u64,
    bench: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<TerminalTextEvent> for TerminalText {}

impl Focusable for TerminalText {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TerminalText {
    /// Start the shell `spec` names and show it. What the session says is
    /// carried to this view; this is the way to make one.
    pub fn spawn(
        spec: &term::ShellSpec,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<Self, term::Error> {
        let (sender, receiver) = mpsc::unbounded::<SessionEvent>();
        let session = Session::spawn(spec, term::GridSize::default(), move |event| {
            // The view is gone when nobody listens: nothing to tell.
            let _ = sender.unbounded_send(event);
        })?;
        let mut text = Self::new(session, window, cx);
        text.listen(receiver, cx);
        Ok(text)
    }

    /// Show a session started elsewhere. Whoever started it hands what it
    /// says on to [`Self::on_session_event`], on the window's thread.
    pub fn new(session: Session, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let view = cx.weak_entity();
        let focus = focus_handle.clone();
        // Before the window matches the key to a binding: see the module's
        // documentation.
        let keys = cx.intercept_keystrokes(move |event, window, cx| {
            if !focus.is_focused(window) {
                return;
            }
            let role = key_role(&event.keystroke, cx);
            if role == KeyRole::Window {
                return;
            }
            let taken = view.update(cx, |text, cx| text.take_key(role, window, cx));
            if taken.is_ok() {
                cx.stop_propagation();
            }
        });
        let focus_in = cx.on_focus_in(&focus_handle, window, |text, _, cx| {
            text.session.focus(true);
            cx.notify();
        });
        let focus_out = cx.on_focus_out(&focus_handle, window, |text, _, _, cx| {
            text.session.focus(false);
            cx.notify();
        });
        let mut text = Self {
            session,
            look: TerminalLook::default(),
            focus_handle,
            size: None,
            geometry: None,
            area: None,
            completer: Completer::default(),
            cursor_at: None,
            selecting: false,
            reporting: None,
            wheel_rest: 0.0,
            marked: None,
            command_running: false,
            quiet: None,
            exited: false,
            events: None,
            frames: 0,
            bench: None,
            _subscriptions: vec![keys, focus_in, focus_out],
        };
        if let Some(seconds) = bench_seconds()
            && !BENCH_STARTED.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            text.start_bench(seconds, cx);
        }
        text
    }

    /// Hear what the session says from a channel its callback feeds.
    fn listen(
        &mut self,
        mut receiver: mpsc::UnboundedReceiver<SessionEvent>,
        cx: &mut Context<Self>,
    ) {
        self.events = Some(cx.spawn(async move |text, cx| {
            while let Some(event) = receiver.next().await {
                if text
                    .update(cx, |text, cx| text.on_session_event(event, cx))
                    .is_err()
                {
                    break;
                }
            }
        }));
    }

    /// The session on show.
    pub fn session(&self) -> &Session {
        &self.session
    }

    /// How the text looks.
    pub fn look(&self) -> &TerminalLook {
        &self.look
    }

    /// Change how the text looks; the screen is fitted again.
    pub fn set_look(&mut self, look: TerminalLook, cx: &mut Context<Self>) {
        if self.look != look {
            self.look = look;
            cx.notify();
        }
    }

    /// Something the session said, on the window's thread.
    pub fn on_session_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Output => {
                if self.command_running {
                    self.wait_for_quiet(cx);
                }
                self.completer.refresh(&self.session, Settings::of(cx));
                cx.notify();
            }
            SessionEvent::Title(title) => cx.emit(TerminalTextEvent::Title(title)),
            SessionEvent::Bell => {}
            SessionEvent::Clipboard(text) => {
                // A program may put text on the clipboard; it is never read
                // back to one.
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
            SessionEvent::Exited(code) => {
                self.exited = true;
                self.command_running = false;
                self.quiet = None;
                cx.emit(TerminalTextEvent::Exited(code));
                cx.notify();
            }
        }
    }

    /// Start waiting anew for the output to stay quiet for [`QUIET`].
    fn wait_for_quiet(&mut self, cx: &mut Context<Self>) {
        self.quiet = Some(cx.spawn(async move |text, cx| {
            cx.background_executor().timer(QUIET).await;
            let _ = text.update(cx, |text, cx| {
                if text.command_running {
                    text.command_running = false;
                    cx.emit(TerminalTextEvent::CommandFinished);
                }
            });
        }));
    }

    /// Copy the selection to the clipboard.
    pub fn copy(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.session.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Paste the clipboard's text.
    pub fn paste(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.paste_text(&text, cx);
        }
    }

    /// Paste text, as the program asked pasted text to come.
    fn paste_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        self.completer.pasted(&self.session, Settings::of(cx));
        self.session.paste(text);
        cx.notify();
    }

    /// Select everything, history included.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.session.select(term::SelectOp::All);
        self.offer_primary(cx);
        cx.notify();
    }

    /// Take back what was typed on the line: Ctrl+U to the shell.
    pub fn delete_line(&mut self, cx: &mut Context<Self>) {
        self.session.input(term::Input::Key(
            term::Key::Character('u'),
            term::Modifiers::CONTROL,
        ));
        cx.notify();
    }

    /// Clear the screen and the history; the line typed in stays.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.session.clear();
        cx.notify();
    }

    /// Whether something is selected.
    pub fn has_selection(&self) -> bool {
        self.session.has_selection()
    }

    /// Whether the shell has ended.
    pub fn exited(&self) -> bool {
        self.exited
    }

    /// Type paths into the shell, written as it reads them, and a space.
    pub fn drop_paths(&mut self, paths: &[PathBuf], window: &mut Window, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let mut text = term::quote_paths(paths, self.session.kind());
        if text.is_empty() {
            return;
        }
        text.push(' ');
        self.paste_text(&text, cx);
        self.focus_handle.focus(window, cx);
    }

    /// What the text has on show, for tests and the window's probes.
    pub fn probe(&self, window: &Window) -> TerminalTextProbe {
        let frame = self.session.frame(&self.look.palette);
        let size = self.session.size();
        TerminalTextProbe {
            text: frame.text(),
            columns: size.columns,
            rows: size.rows,
            display_offset: frame.display_offset,
            history: frame.history,
            selected: self.session.selected_text(),
            focused: self.focus_handle.is_focused(window),
            cursor: frame.cursor.map(|cursor| (cursor.row, cursor.column)),
            origin: self
                .geometry
                .map(|geometry| (f32::from(geometry.origin.x), f32::from(geometry.origin.y))),
            cell: self.geometry.map(|geometry| {
                (
                    f32::from(geometry.cell.width),
                    f32::from(geometry.cell.height),
                )
            }),
            completions: self
                .completer
                .shown()
                .map(|shown| {
                    shown
                        .completion
                        .candidates
                        .iter()
                        .map(|candidate| candidate.label.clone())
                        .collect()
                })
                .unwrap_or_default(),
            marked: self.completer.shown().map(|shown| shown.marked),
            hint: self.completer.shown().and_then(Shown::hint),
            list_at: self.completion_place(),
        }
    }

    /// Where the list of completions stands, in the window.
    fn completion_place(&self) -> Option<(f32, f32)> {
        let shown = self.completer.shown()?;
        let cell = self
            .geometry?
            .bounds(shown.cursor.row, shown.cursor.column, 1);
        let at = completion::place(cell, self.area?, shown.completion.candidates.len());
        Some((f32::from(at.x), f32::from(at.y)))
    }

    /// A key the interceptor took while the text has the focus.
    fn take_key(&mut self, role: KeyRole, window: &mut Window, cx: &mut Context<Self>) {
        match role {
            KeyRole::Window | KeyRole::Nothing => {}
            KeyRole::Own(OwnKey::Copy) => self.copy(cx),
            KeyRole::Own(OwnKey::CopyOrInterrupt) => {
                if self.session.has_selection() {
                    self.copy(cx);
                    self.session.select(term::SelectOp::Clear);
                    cx.notify();
                } else {
                    let interrupt =
                        term::Input::Key(term::Key::Character('c'), term::Modifiers::CONTROL);
                    self.type_input(interrupt, cx);
                }
            }
            KeyRole::Own(OwnKey::Paste) => self.paste(window, cx),
            KeyRole::Shell(input) => self.shell_key(input, window, cx),
        }
    }

    /// A key for the shell, unless the completions take it.
    fn shell_key(&mut self, input: term::Input, window: &mut Window, cx: &mut Context<Self>) {
        let settings = Settings::of(cx);
        match self.completer.key(&input, settings) {
            KeyUse::Pass => self.type_input(input, cx),
            KeyUse::Taken => cx.notify(),
            KeyUse::Reopen => {
                self.completer.refresh(&self.session, settings);
                cx.notify();
            }
            KeyUse::Accept(index) => self.take_completion(index, window, cx),
        }
    }

    /// Take an entry of the completions: what it adds is typed into the
    /// shell.
    pub fn take_completion(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(inputs) = self.completer.accept(index) {
            for input in inputs {
                self.session.input(input);
            }
        }
        self.focus_handle.focus(window, cx);
        cx.notify();
    }

    /// The repository's names, to complete git commands with.
    pub fn set_completion_words(&mut self, words: Arc<bitbraid_engine::completion::Words>) {
        self.completer.set_words(words);
    }

    /// The completions on show.
    pub fn completion(&self) -> Option<&Shown> {
        self.completer.shown()
    }

    /// The list of completions, drawn over the window at the cursor.
    fn completion_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let shown = self.completer.shown()?;
        let geometry = self.geometry?;
        let area = self.area?;
        let cell = geometry.bounds(shown.cursor.row, shown.cursor.column, 1);
        let at = completion::place(cell, area, shown.completion.candidates.len());
        let view = cx.entity().downgrade();
        let pick = view.clone();
        let hover = view.clone();
        let handlers = ListHandlers {
            pick: Rc::new(move |index, window, cx| {
                let _ = pick.update(cx, |text, cx| text.take_completion(index, window, cx));
            }),
            hover: Rc::new(move |index, _, cx| {
                let _ = hover.update(cx, |text, cx| {
                    if text.completer.mark(index) {
                        cx.notify();
                    }
                });
            }),
            wheel: Rc::new(move |rows, _, cx| {
                let _ = view.update(cx, |text, cx| {
                    if text.completer.scroll(rows) {
                        cx.notify();
                    }
                });
            }),
        };
        Some(completion::list(shown, at, handlers, cx))
    }

    /// Send typed input to the shell.
    fn type_input(&mut self, input: term::Input, cx: &mut Context<Self>) {
        let enter = matches!(input, term::Input::Key(term::Key::Enter, _));
        self.completer
            .typed(&input, &self.session, Settings::of(cx));
        self.session.input(input);
        if self.session.has_selection() {
            self.session.select(term::SelectOp::Clear);
        }
        if enter {
            self.command_running = true;
            self.wait_for_quiet(cx);
        }
        cx.notify();
    }

    /// Fit the session to the room the element has: called while it is
    /// laid out. Whether the size changed.
    fn fit(&mut self, geometry: Geometry, size: term::GridSize) -> bool {
        self.geometry = Some(geometry);
        if self.size == Some(size) {
            return false;
        }
        self.size = Some(size);
        self.session.resize(size);
        true
    }

    /// Put the selection on the primary selection, where the desktop has
    /// one (a middle click elsewhere pastes it).
    fn offer_primary(&self, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if let Some(text) = self.session.selected_text() {
            cx.write_to_primary(ClipboardItem::new_string(text));
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let _ = cx;
    }

    /// The text of the primary selection, where there is one, else of the
    /// clipboard: what a middle click pastes.
    fn primary_text(cx: &mut Context<Self>) -> Option<String> {
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if let Some(text) = cx.read_from_primary().and_then(|item| item.text()) {
            return Some(text);
        }
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    fn mouse_event(
        &self,
        kind: term::MouseKind,
        position: Point<Pixels>,
        held: &Modifiers,
    ) -> Option<term::Mouse> {
        let geometry = self.geometry?;
        let (point, _) = geometry.cell_at(position);
        Some(term::Mouse {
            kind,
            point,
            modifiers: modifiers_of(held),
        })
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_handle.focus(window, cx);
        let Some(geometry) = self.geometry else {
            return;
        };
        let Some(button) = button_of(event.button) else {
            return;
        };
        if let Some(mouse) = self.mouse_event(
            term::MouseKind::Press(button),
            event.position,
            &event.modifiers,
        ) && self.session.mouse(mouse)
        {
            self.reporting = Some(button);
            return;
        }
        match button {
            term::MouseButton::Left => {
                let (point, side) = geometry.cell_at(event.position);
                let op = match event.click_count {
                    0 | 1 if event.modifiers.shift => term::SelectOp::Extend(point, side),
                    0 | 1 => term::SelectOp::Simple(point, side),
                    2 => term::SelectOp::Word(point, side),
                    _ => term::SelectOp::Line(point, side),
                };
                self.session.select(op);
                self.selecting = true;
                cx.notify();
            }
            term::MouseButton::Middle => {
                if let Some(text) = Self::primary_text(cx) {
                    self.paste_text(&text, cx);
                }
            }
            term::MouseButton::Right => cx.emit(TerminalTextEvent::ContextMenu(event.position)),
        }
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, hovered: bool, cx: &mut Context<Self>) {
        let Some(geometry) = self.geometry else {
            return;
        };
        if self.selecting {
            if event.pressed_button != Some(MouseButton::Left) {
                // The release happened where nobody heard it.
                self.selecting = false;
                self.offer_primary(cx);
                return;
            }
            let outside = geometry.rows_outside(event.position);
            if outside != 0 {
                self.session.scroll(outside);
            }
            let (point, side) = geometry.cell_at(event.position);
            self.session.select(term::SelectOp::Extend(point, side));
            cx.notify();
            return;
        }
        if hovered || self.reporting.is_some() {
            let held = event.pressed_button.and_then(button_of);
            if let Some(mouse) = self.mouse_event(
                term::MouseKind::Move { held },
                event.position,
                &event.modifiers,
            ) {
                self.session.mouse(mouse);
            }
        }
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, hovered: bool, cx: &mut Context<Self>) {
        let Some(button) = button_of(event.button) else {
            return;
        };
        if button == term::MouseButton::Left && self.selecting {
            self.selecting = false;
            self.offer_primary(cx);
            cx.notify();
            return;
        }
        if (hovered || self.reporting == Some(button))
            && let Some(mouse) = self.mouse_event(
                term::MouseKind::Release(button),
                event.position,
                &event.modifiers,
            )
        {
            self.session.mouse(mouse);
        }
        if self.reporting == Some(button) {
            self.reporting = None;
        }
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let Some(geometry) = self.geometry else {
            return;
        };
        let lines = match event.delta {
            ScrollDelta::Lines(delta) => delta.y,
            ScrollDelta::Pixels(delta) => {
                f32::from(delta.y) / f32::from(geometry.cell.height).max(1.0)
            }
        };
        if !lines.is_finite() {
            return;
        }
        let total = self.wheel_rest + lines;
        let whole = total.trunc();
        self.wheel_rest = total - whole;
        // A hostile delta is held to what a screen can scroll.
        let whole = whole.clamp(-10_000.0, 10_000.0) as i32;
        if whole == 0 {
            return;
        }
        if let Some(mouse) = self.mouse_event(
            term::MouseKind::Wheel { lines: whole },
            event.position,
            &event.modifiers,
        ) && self.session.mouse(mouse)
        {
            return;
        }
        self.session.scroll(whole);
        cx.notify();
    }
}

/// The measuring run of a terminal: `BITBRAID_BENCH_TERMINAL=<seconds>`,
/// which `cargo xtask appbench --terminal` sets. The first terminal of the
/// window runs a program that writes coloured lines, each different,
/// faster than frames are drawn; the text is asked to draw again every
/// few milliseconds (a headless compositor paces its frames slower than a
/// screen), says each second how many frames it drew, and at the end
/// writes an `APP_BENCH terminal ...` line and quits the application.
pub const BENCH_VARIABLE: &str = "BITBRAID_BENCH_TERMINAL";

/// How long the measuring run lasts, when one is asked for.
pub fn bench_seconds() -> Option<u64> {
    std::env::var(BENCH_VARIABLE)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|seconds| (1..=3600).contains(seconds))
}

/// Only the first terminal of a measuring run measures.
static BENCH_STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What the measuring run types: lines of 160 characters and more, in
/// eight colours on seven backgrounds, each with a number of its own.
// not shown: a command line the shell runs
const BENCH_PROGRAM: &str = r#"awk 'BEGIN { for (i = 0; ; i++) printf "\033[3%dm%08d \033[1;4%dm%0150d\033[0m %d\n", i % 8, i, i % 7, i, i }'"#;

/// How often the measuring run asks for a frame.
const BENCH_TICK: Duration = Duration::from_millis(4);

impl TerminalText {
    /// Start the measuring run (see [`BENCH_VARIABLE`]).
    #[allow(clippy::print_stdout)] // the measuring run reports on stdout by design
    fn start_bench(&mut self, seconds: u64, cx: &mut Context<Self>) {
        self.session
            .input(term::Input::Text(BENCH_PROGRAM.to_owned()));
        self.session
            .input(term::Input::Key(term::Key::Enter, term::Modifiers::NONE));
        let duration = Duration::from_secs(seconds);
        self.bench = Some(cx.spawn(async move |text, cx| {
            // The shell starts and the stream gets going first.
            cx.background_executor()
                .timer(Duration::from_secs(1))
                .await;
            let Ok(first) = text.update(cx, |text, _| text.frames) else {
                return;
            };
            let started = std::time::Instant::now();
            let mut second_start = started;
            let mut second_frames = first;
            let mut samples: Vec<f64> = Vec::new();
            loop {
                cx.background_executor().timer(BENCH_TICK).await;
                let Ok(frames) = text.update(cx, |text, cx| {
                    cx.notify();
                    text.frames
                }) else {
                    return;
                };
                let now = std::time::Instant::now();
                let span = now.duration_since(second_start).as_secs_f64();
                if span >= 1.0 {
                    let fps = frames.saturating_sub(second_frames) as f64 / span;
                    println!("fps {fps:.1}");
                    samples.push(fps);
                    second_start = now;
                    second_frames = frames;
                }
                if now.duration_since(started) >= duration {
                    let min = samples.iter().copied().fold(f64::INFINITY, f64::min);
                    let avg = if samples.is_empty() {
                        0.0
                    } else {
                        samples.iter().sum::<f64>() / samples.len() as f64
                    };
                    let memory = crate::memory::resident()
                        .map(|held| {
                            format!(
                                " peak_rss_mib={} rss_mib={}", // not shown: xtask appbench reads it
                                held.peak_mib(),
                                held.now_mib()
                            )
                        })
                        .unwrap_or_default();
                    let _ = text.update(cx, |text, cx| {
                        let size = text.session.size();
                        println!(
                            "APP_BENCH terminal columns={} rows={} seconds={seconds} frames={} min_fps={min:.1} avg_fps={avg:.1}{memory}",
                            size.columns,
                            size.rows,
                            frames.saturating_sub(first)
                        );
                        text.session.kill();
                        cx.quit();
                    });
                    return;
                }
            }
        }));
    }
}

/// What a terminal's text has on show.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalTextProbe {
    /// The text on show, a line per row, blanks at the ends left out.
    pub text: String,
    /// Columns of the screen.
    pub columns: u16,
    /// Rows of the screen.
    pub rows: u16,
    /// How far the view is scrolled back into history.
    pub display_offset: usize,
    /// Lines of history.
    pub history: usize,
    /// The selected text.
    pub selected: Option<String>,
    /// Whether the text has the keys.
    pub focused: bool,
    /// The cursor's row and column, when it shows.
    pub cursor: Option<(u16, u16)>,
    /// Where the first cell was drawn, in the window.
    pub origin: Option<(f32, f32)>,
    /// The size of a cell.
    pub cell: Option<(f32, f32)>,
    /// The completions on show, by what the list says.
    pub completions: Vec<String>,
    /// Which of them is marked.
    pub marked: Option<usize>,
    /// The rest of the marked one, shown faintly after the cursor.
    pub hint: Option<String>,
    /// Where the list stands, in the window.
    pub list_at: Option<(f32, f32)>,
}

/// What a key does while the text has the focus.
#[derive(Debug, Clone, PartialEq, Eq)]
enum KeyRole {
    /// The window's: one of [`WINDOW_KEYS`].
    Window,
    /// The text's own: one of [`own_keys`].
    Own(OwnKey),
    /// Typed into the shell.
    Shell(term::Input),
    /// Taken, and nothing done: a modifier alone, a key a terminal has no
    /// bytes for.
    Nothing,
}

/// What a key does while the text has the focus: the one place the
/// exceptions are decided.
fn key_role(keystroke: &Keystroke, cx: &App) -> KeyRole {
    let typed = crate::keymap::normal_key(&keystroke.unparse()).unwrap_or_default();
    if window_keys(cx).contains(&typed) {
        return KeyRole::Window;
    }
    if let Some((_, own)) = own_keys()
        .iter()
        .find(|(key, _)| crate::keymap::normal_key(key).is_some_and(|key| key == typed))
    {
        return KeyRole::Own(*own);
    }
    match input_of(keystroke) {
        Some(input) => KeyRole::Shell(input),
        None => KeyRole::Nothing,
    }
}

/// The keys of [`WINDOW_KEYS`] as they are bound now, the user's included;
/// a key of several strokes is never one keystroke and passes nothing.
fn window_keys(cx: &App) -> Vec<String> {
    let Some(bound) = cx.try_global::<crate::keymap::Keys>() else {
        return Vec::new();
    };
    WINDOW_KEYS
        .iter()
        .flat_map(|id| bound.resolved.keys_of(id))
        .filter_map(|key| crate::keymap::normal_key(key))
        .collect()
}

/// The modifiers as the terminal crate has them.
fn modifiers_of(held: &Modifiers) -> term::Modifiers {
    term::Modifiers {
        shift: held.shift,
        control: held.control,
        alt: held.alt,
        platform: held.platform,
    }
}

/// The terminal's name of a button of the pointer.
fn button_of(button: MouseButton) -> Option<term::MouseButton> {
    match button {
        MouseButton::Left => Some(term::MouseButton::Left),
        MouseButton::Middle => Some(term::MouseButton::Middle),
        MouseButton::Right => Some(term::MouseButton::Right),
        _ => None,
    }
}

/// The only character of a string, when it has exactly one.
fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}

/// What a key types into a terminal, from the toolkit's keystroke: a named
/// key with its modifiers, a character with Control or Alt, or the text a
/// key types. None for a modifier alone or a key a terminal has no bytes
/// for.
fn input_of(keystroke: &Keystroke) -> Option<term::Input> {
    let modifiers = modifiers_of(&keystroke.modifiers);
    let key = keystroke.key.as_str();
    let typed = keystroke
        .key_char
        .as_deref()
        .filter(|text| !text.is_empty() && !text.chars().any(char::is_control));
    if matches!(
        key,
        "shift" | "control" | "alt" | "platform" | "function" | "capslock"
    ) {
        return None;
    }
    // On macOS Option types what the key has under it (ø; @, [ and | on
    // many layouts), as the terminals there do unless told to send Escape
    // first.
    if crate::keymap::System::now() == crate::keymap::System::Mac
        && modifiers.alt
        && !modifiers.control
        && !modifiers.platform
        && let Some(text) = typed
    {
        return Some(term::Input::Text(text.to_owned()));
    }
    if key == "space" {
        return Some(if modifiers.control || modifiers.alt {
            term::Input::Key(term::Key::Character(' '), modifiers)
        } else {
            term::Input::Text(" ".to_owned())
        });
    }
    if let Some(on_key) = single_char(key) {
        if modifiers.control || modifiers.alt || modifiers.platform {
            // With Control the character on the key; with Alt alone the one
            // the key types (Alt+Shift+a is ESC A).
            let character = if modifiers.control {
                on_key
            } else {
                typed.and_then(single_char).unwrap_or(on_key)
            };
            return Some(term::Input::Key(term::Key::Character(character), modifiers));
        }
        return Some(match typed {
            Some(text) => term::Input::Text(text.to_owned()),
            None => term::Input::Key(term::Key::Character(on_key), modifiers),
        });
    }
    if let Some(named) = term::Key::named(key) {
        return Some(term::Input::Key(named, modifiers));
    }
    // A key named after what it types (a letter of another alphabet, a
    // composed character, a key of the keypad).
    let text = typed?;
    if modifiers.control || modifiers.platform {
        return None;
    }
    if modifiers.alt
        && let Some(character) = single_char(text)
    {
        return Some(term::Input::Key(term::Key::Character(character), modifiers));
    }
    Some(term::Input::Text(text.to_owned()))
}

/// A colour of the terminal as the toolkit's.
fn color_of(rgb: term::Rgb) -> Hsla {
    Hsla::from(gpui_kit::rgb(rgb.to_hex()))
}

/// The fonts of a look: regular, bold, italic, bold italic.
struct Fonts {
    regular: Font,
    bold: Font,
    italic: Font,
    bold_italic: Font,
}

impl Fonts {
    fn of(family: SharedString) -> Self {
        let regular = font(family);
        Self {
            bold: regular.clone().bold(),
            italic: regular.clone().italic(),
            bold_italic: regular.clone().bold().italic(),
            regular,
        }
    }

    fn pick(&self, style: &term::Style) -> &Font {
        match (style.bold, style.italic) {
            (false, false) => &self.regular,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (true, true) => &self.bold_italic,
        }
    }
}

/// The size of a cell for a font: the advance of `m` across; the size of
/// the font times the line height down, cut to whole device pixels (a font
/// of 12 at 1.2 is a row of 14).
fn cell_size(window: &Window, font: &Font, font_size: Pixels, line_height: f32) -> Size<Pixels> {
    let text_system = window.text_system();
    let font_id = text_system.resolve_font(font);
    let width = text_system
        .advance(font_id, font_size, 'm')
        .map(|advance| advance.width)
        .unwrap_or(font_size * 0.6);
    let scale = window.scale_factor().max(1.0);
    let height = f32::from(font_size) * line_height.clamp(0.5, 4.0);
    let height = ((height * scale).floor() / scale).max(f32::from(font_size).min(1.0));
    let width = f32::from(width);
    let width = if width.is_finite() && width > 0.0 {
        width
    } else {
        f32::from(font_size) * 0.6
    };
    // Already the toolkit's pixels: the font's size was zoomed.
    size(gpui_kit::px(width.max(1.0)), gpui_kit::px(height.max(1.0)))
}

/// A piece of a row shaped as one line.
struct Piece {
    column: u16,
    text: String,
    runs: Vec<TextRun>,
    /// One wide character, centred over two cells.
    wide: bool,
}

impl Piece {
    fn new(column: u16) -> Self {
        Self {
            column,
            text: String::new(),
            runs: Vec::new(),
            wide: false,
        }
    }

    /// Add text in a style, joining the last run when it has that style.
    fn push(
        &mut self,
        text: &str,
        font: &Font,
        color: Hsla,
        underline: Option<UnderlineStyle>,
        strikethrough: Option<StrikethroughStyle>,
    ) {
        if text.is_empty() {
            return;
        }
        self.text.push_str(text);
        if let Some(last) = self.runs.last_mut()
            && last.color == color
            && last.underline == underline
            && last.strikethrough == strikethrough
            && last.font == *font
        {
            last.len = last.len.saturating_add(text.len());
            return;
        }
        self.runs.push(TextRun {
            len: text.len(),
            font: font.clone(),
            color,
            background_color: None,
            underline,
            strikethrough,
        });
    }

    /// Blanks after the text, in the last run's font and colour.
    fn push_blanks(&mut self, count: usize, fallback: &Font, color: Hsla) {
        if count == 0 {
            return;
        }
        let (font, color) = match self.runs.last() {
            Some(last) => (last.font.clone(), last.color),
            None => (fallback.clone(), color),
        };
        self.push(&" ".repeat(count), &font, color, None, None);
    }
}

/// The underline of a style, in the toolkit's terms.
fn underline_of(style: &term::Style, color: Hsla, opacity: f32) -> Option<UnderlineStyle> {
    let line_color = style
        .underline_color
        .map(|rgb| color_of(rgb).opacity(opacity))
        .unwrap_or(color);
    match style.underline {
        term::Underline::None => None,
        term::Underline::Single => Some(UnderlineStyle {
            thickness: px(1.0),
            color: Some(line_color),
            wavy: false,
        }),
        term::Underline::Double => Some(UnderlineStyle {
            thickness: px(2.0),
            color: Some(line_color),
            wavy: false,
        }),
        term::Underline::Curly => Some(UnderlineStyle {
            thickness: px(1.0),
            color: Some(line_color),
            wavy: true,
        }),
    }
}

/// The pieces of a row to shape: text in runs of one style, broken where a
/// wide character is or where many blanks are.
fn pieces(row: &term::Row, fonts: &Fonts, opacity: f32) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut current: Option<Piece> = None;
    let mut blanks = 0usize;
    // Colours repeat along a row: converted once per change.
    let mut last_rgb: Option<(term::Rgb, Hsla)> = None;
    let mut color = |rgb: term::Rgb| -> Hsla {
        match last_rgb {
            Some((seen, hsla)) if seen == rgb => hsla,
            _ => {
                let hsla = color_of(rgb).opacity(opacity);
                last_rgb = Some((rgb, hsla));
                hsla
            }
        }
    };
    let mut buffer = [0u8; 4];
    // The colour and style of the last cell added to a run.
    let mut last_style: Option<(term::Rgb, term::Style)> = None;
    for (column, cell) in row.cells.iter().enumerate() {
        let Ok(column) = u16::try_from(column) else {
            break;
        };
        match cell.width {
            term::CellWidth::Spacer => {
                out.extend(current.take());
                blanks = 0;
                continue;
            }
            term::CellWidth::Wide => {
                out.extend(current.take());
                blanks = 0;
                let mut piece = Piece::new(column);
                piece.wide = true;
                let hsla = color(cell.foreground);
                let mut text = cell.character.to_string();
                if let Some(extra) = &cell.zero_width {
                    text.push_str(extra);
                }
                piece.push(
                    &text,
                    fonts.pick(&cell.style),
                    hsla,
                    underline_of(&cell.style, hsla, opacity),
                    cell.style.strikeout.then_some(StrikethroughStyle {
                        thickness: px(1.0),
                        color: Some(hsla),
                    }),
                );
                out.push(piece);
                continue;
            }
            term::CellWidth::Normal => {}
        }
        let blank = cell.character == ' '
            && cell.zero_width.is_none()
            && cell.style.underline == term::Underline::None
            && !cell.style.strikeout;
        if blank {
            if current.is_some() {
                blanks = blanks.saturating_add(1);
            }
            continue;
        }
        // The same style as the cell before: the run goes on.
        if blanks == 0
            && last_style == Some((cell.foreground, cell.style))
            && let Some(piece) = current.as_mut()
            && let Some(run) = piece.runs.last_mut()
        {
            let before = piece.text.len();
            piece.text.push(cell.character);
            if let Some(extra) = &cell.zero_width {
                piece.text.push_str(extra);
            }
            run.len = run
                .len
                .saturating_add(piece.text.len().saturating_sub(before));
            continue;
        }
        last_style = Some((cell.foreground, cell.style));
        let hsla = color(cell.foreground);
        let font = fonts.pick(&cell.style);
        match current.as_mut() {
            Some(piece) if blanks <= BRIDGED_BLANKS => piece.push_blanks(blanks, font, hsla),
            Some(_) => out.extend(current.take()),
            None => {}
        }
        blanks = 0;
        let piece = current.get_or_insert_with(|| Piece::new(column));
        let text: &str = cell.character.encode_utf8(&mut buffer);
        let underline = underline_of(&cell.style, hsla, opacity);
        let strikethrough = cell.style.strikeout.then_some(StrikethroughStyle {
            thickness: px(1.0),
            color: Some(hsla),
        });
        piece.push(text, font, hsla, underline, strikethrough);
        if let Some(extra) = &cell.zero_width {
            piece.push(extra, font, hsla, underline, strikethrough);
        }
    }
    out.extend(current);
    out
}

impl Render for TerminalText {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let frame = self.session.frame(&self.look.palette);
        // Output that came after the list was last worked out and before
        // this frame was taken is not announced again: the session says
        // so once until a frame is taken. The list is worked out from the
        // screen as it stands now, and what comes after is announced.
        self.completer.refresh(&self.session, Settings::of(cx));
        self.frames = self.frames.saturating_add(1);
        self.cursor_at = frame.cursor.map(|cursor| (cursor.row, cursor.column));
        let focused = self.focus_handle.is_focused(window);
        let family = if self.look.font_family.is_empty() {
            cx.theme().mono_font_family.clone()
        } else {
            self.look.font_family.clone()
        };
        let highlight = cx.theme().primary;
        let background = color_of(frame.background);
        // What the screen says, for assistive technology while it runs;
        // worked out only then.
        let spoken = window.is_a11y_active().then(|| frame.text());
        div()
            .id("terminal-text")
            .test_support()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .role(Role::Terminal)
            .aria_label(crate::access::TERMINAL_NAME.text())
            .when_some(spoken, |terminal, text| terminal.aria_value(text))
            .relative()
            .size_full()
            .overflow_hidden()
            .bg(background)
            // Files from the window's lists, and from the desktop. The
            // window's own drop (a folder opens as a repository) is not
            // reached.
            .on_drop(cx.listener(|text, dragged: &DraggedPaths, window, cx| {
                text.drop_paths(&dragged.paths, window, cx);
                cx.stop_propagation();
            }))
            .on_drop(cx.listener(|text, paths: &ExternalPaths, window, cx| {
                text.drop_paths(paths.paths(), window, cx);
                cx.stop_propagation();
            }))
            .child(TextElement {
                view: cx.entity(),
                frame,
                fonts: Fonts::of(family),
                look: self.look.clone(),
                focused,
                marked: self.marked.clone(),
                hint: self
                    .completer
                    .shown()
                    .and_then(|shown| Some((shown.cursor, shown.hint()?))),
                focus_handle: self.focus_handle.clone(),
            })
            .children(self.completion_list(cx))
            .child(
                // Lights up while files are dragged over the text.
                div()
                    .id("terminal-text-drop")
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .drag_over::<DraggedPaths>(move |style, _, _, _| drop_target(style, highlight))
                    .drag_over::<ExternalPaths>(move |style, _, _, _| {
                        drop_target(style, highlight)
                    }),
            )
    }
}

/// How the text looks while files are dragged over it.
fn drop_target(style: StyleRefinement, highlight: Hsla) -> StyleRefinement {
    style
        .border_2()
        .border_color(highlight)
        .bg(highlight.opacity(DROP_TINT))
}

/// How strongly the text is tinted while files are dragged over it.
const DROP_TINT: f32 = 0.08;

/// The element that paints a frame and hears the mouse.
struct TextElement {
    view: Entity<TerminalText>,
    frame: term::Frame,
    fonts: Fonts,
    look: TerminalLook,
    focused: bool,
    marked: Option<String>,
    /// The rest of the completion marked, and where the cursor is.
    hint: Option<(term::ViewPoint, String)>,
    focus_handle: FocusHandle,
}

/// What laying the element out found.
struct Laid {
    hitbox: Hitbox,
    geometry: Geometry,
    font_size: Pixels,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = Laid;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.0).into();
        style.size.height = relative(1.0).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let font_size = px(self.look.font_size.clamp(4.0, 96.0));
        let cell = cell_size(
            window,
            &self.fonts.regular,
            font_size,
            self.look.line_height,
        );
        let room_across = f32::from(bounds.size.width - px(PADDING_LEFT) - px(PADDING_RIGHT));
        let room_down = f32::from(bounds.size.height - px(PADDING_TOP));
        // Saturating casts: a negative or huge room is held to the screen's bounds.
        let columns = (room_across / f32::from(cell.width)).floor().max(0.0) as u16;
        let rows = (room_down / f32::from(cell.height)).floor().max(0.0) as u16;
        let size = term::GridSize::new(
            columns,
            rows,
            f32::from(cell.width).round() as u16,
            f32::from(cell.height).round() as u16,
        )
        .bounded();
        let geometry = Geometry {
            origin: point(
                bounds.origin.x + px(PADDING_LEFT),
                bounds.origin.y + px(PADDING_TOP),
            ),
            cell,
            columns: size.columns,
            rows: size.rows,
        };
        let resized = self.view.update(cx, |text, _| {
            text.area = Some(bounds);
            text.fit(geometry, size)
        });
        if resized {
            // The screen the session keeps now has the new size; draw it.
            window.request_animation_frame();
        }
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        Laid {
            hitbox,
            geometry,
            font_size,
        }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        laid: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let geometry = laid.geometry;
        if !self.frame.mode.mouse_reporting {
            window.set_cursor_style(gpui_kit::CursorStyle::IBeam, &laid.hitbox);
        }
        let opacity = if self.focused || !self.look.dim_unfocused {
            1.0
        } else {
            UNFOCUSED_OPACITY
        };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            window.paint_quad(fill(bounds, color_of(self.frame.background)));
            self.paint_backgrounds(bounds, &geometry, window);
            self.paint_text(&geometry, laid.font_size, opacity, window, cx);
            self.paint_cursor(&geometry, laid.font_size, opacity, window, cx);
            self.paint_marked(&geometry, laid.font_size, window, cx);
            if let Some((at, hint)) = &self.hint
                && self.marked.is_none()
            {
                completion::paint_hint(
                    hint,
                    geometry.bounds(at.row, at.column, 1),
                    &self.fonts.regular,
                    laid.font_size,
                    color_of(self.frame.foreground),
                    window,
                    cx,
                );
            }
        });
        window.handle_input(
            &self.focus_handle,
            ElementInputHandler::new(bounds, self.view.clone()),
            cx,
        );
        self.listen_to_the_mouse(&laid.hitbox, window);
    }
}

impl TextElement {
    /// The backgrounds of the cells, in one layer: they do not overlap, so
    /// the toolkit need not work out for each which others it lies over
    /// (a full screen of colours is thousands of them).
    fn paint_backgrounds(&self, bounds: Bounds<Pixels>, geometry: &Geometry, window: &mut Window) {
        window.paint_layer(bounds, |window| {
            for (row, cells) in self.frame.rows.iter().enumerate() {
                let Ok(row) = u16::try_from(row) else {
                    break;
                };
                for span in cells.backgrounds(self.frame.background) {
                    window.paint_quad(fill(
                        geometry.bounds(row, span.column, span.columns),
                        color_of(span.color),
                    ));
                }
            }
        });
    }

    fn paint_text(
        &self,
        geometry: &Geometry,
        font_size: Pixels,
        opacity: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        for (row, cells) in self.frame.rows.iter().enumerate() {
            let Ok(row) = u16::try_from(row) else {
                break;
            };
            for piece in pieces(cells, &self.fonts, opacity) {
                let force = (!piece.wide).then_some(geometry.cell.width);
                let shaped = window.text_system().shape_line(
                    SharedString::from(piece.text),
                    font_size,
                    &piece.runs,
                    force,
                );
                let place = geometry.bounds(row, piece.column, if piece.wide { 2 } else { 1 });
                let mut origin = place.origin;
                if piece.wide && shaped.width() < place.size.width {
                    origin.x += (place.size.width - shaped.width()) / 2.0;
                }
                // A line that cannot be painted leaves its cells blank.
                let _ = shaped.paint(
                    origin,
                    geometry.cell.height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
        }
    }

    fn paint_cursor(
        &self,
        geometry: &Geometry,
        font_size: Pixels,
        opacity: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(cursor) = self.frame.cursor else {
            return;
        };
        if self.marked.is_some() {
            return;
        }
        let cells = if cursor.wide { 2 } else { 1 };
        let place = geometry.bounds(cursor.row, cursor.column, cells);
        let color = color_of(cursor.color).opacity(opacity);
        if !self.focused {
            window.paint_quad(outline(place, color, BorderStyle::Solid));
            return;
        }
        let shape = match cursor.shape {
            Some(term::CursorShape::Block) => CursorStyle::Block,
            Some(term::CursorShape::Underline) => CursorStyle::Underline,
            Some(term::CursorShape::Bar) => CursorStyle::Bar,
            Some(term::CursorShape::HollowBlock) => {
                window.paint_quad(outline(place, color, BorderStyle::Solid));
                return;
            }
            None => self.look.cursor,
        };
        match shape {
            CursorStyle::Bar => window.paint_quad(fill(
                Bounds {
                    origin: place.origin,
                    size: size(px(CURSOR_LINE), place.size.height),
                },
                color,
            )),
            CursorStyle::Underline => window.paint_quad(fill(
                Bounds {
                    origin: point(
                        place.origin.x,
                        place.origin.y + place.size.height - px(CURSOR_UNDERLINE),
                    ),
                    size: size(place.size.width, px(CURSOR_UNDERLINE)),
                },
                color,
            )),
            CursorStyle::Block => {
                window.paint_quad(fill(place, color));
                if cursor.character != ' ' {
                    let text = cursor.character.to_string();
                    let run = TextRun {
                        len: text.len(),
                        font: self.fonts.regular.clone(),
                        color: color_of(cursor.text_color),
                        background_color: None,
                        underline: None,
                        strikethrough: None,
                    };
                    let shaped = window.text_system().shape_line(
                        SharedString::from(text),
                        font_size,
                        &[run],
                        None,
                    );
                    let _ = shaped.paint(
                        place.origin,
                        geometry.cell.height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    );
                }
            }
        }
    }

    /// What an input method is composing, at the cursor, underlined.
    fn paint_marked(
        &self,
        geometry: &Geometry,
        font_size: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(marked) = self.marked.as_deref().filter(|text| !text.is_empty()) else {
            return;
        };
        let Some(cursor) = self.frame.cursor else {
            return;
        };
        let color = color_of(self.frame.foreground);
        let run = TextRun {
            len: marked.len(),
            font: self.fonts.regular.clone(),
            color,
            background_color: None,
            underline: Some(UnderlineStyle {
                thickness: px(1.0),
                color: Some(color),
                wavy: false,
            }),
            strikethrough: None,
        };
        let shaped = window.text_system().shape_line(
            SharedString::from(marked.to_owned()),
            font_size,
            &[run],
            None,
        );
        let place = geometry.bounds(cursor.row, cursor.column, 1);
        window.paint_quad(fill(
            Bounds {
                origin: place.origin,
                size: size(shaped.width().max(place.size.width), place.size.height),
            },
            color_of(self.frame.background),
        ));
        let _ = shaped.paint(
            place.origin,
            geometry.cell.height,
            TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    fn listen_to_the_mouse(&self, hitbox: &Hitbox, window: &mut Window) {
        let view = self.view.clone();
        let over = hitbox.clone();
        window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !over.is_hovered(window) {
                return;
            }
            view.update(cx, |text, cx| text.mouse_down(event, window, cx));
            cx.stop_propagation();
        });
        let view = self.view.clone();
        let over = hitbox.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let hovered = over.is_hovered(window);
            view.update(cx, |text, cx| text.mouse_move(event, hovered, cx));
        });
        let view = self.view.clone();
        let over = hitbox.clone();
        window.on_mouse_event(move |event: &MouseUpEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let hovered = over.is_hovered(window);
            view.update(cx, |text, cx| text.mouse_up(event, hovered, cx));
        });
        let view = self.view.clone();
        let over = hitbox.clone();
        window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !over.should_handle_scroll(window) {
                return;
            }
            view.update(cx, |text, cx| text.scroll_wheel(event, cx));
            cx.stop_propagation();
        });
    }
}

/// The byte offset of a UTF-16 offset in `text`, held to its length.
fn byte_of_utf16(text: &str, offset: usize) -> usize {
    let mut units = 0usize;
    for (index, c) in text.char_indices() {
        if units >= offset {
            return index;
        }
        units = units.saturating_add(c.len_utf16());
    }
    text.len()
}

impl EntityInputHandler for TerminalText {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let marked = self.marked.as_deref().unwrap_or("");
        let start = byte_of_utf16(marked, range.start);
        let end = byte_of_utf16(marked, range.end).max(start);
        let text = marked.get(start..end)?.to_owned();
        let units = marked
            .get(..start)
            .map_or(0, |head| head.encode_utf16().count());
        *adjusted_range = Some(units..units.saturating_add(text.encode_utf16().count()));
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let end = self
            .marked
            .as_deref()
            .map_or(0, |marked| marked.encode_utf16().count());
        Some(UTF16Selection {
            range: end..end,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked
            .as_deref()
            .map(|marked| 0..marked.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.marked.take().is_some() {
            cx.notify();
        }
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = None;
        if !text.is_empty() {
            self.type_input(term::Input::Text(text.to_owned()), cx);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!new_text.is_empty()).then(|| new_text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let geometry = self.geometry?;
        let (row, column) = self.cursor_at?;
        Some(geometry.bounds(row, column, 1))
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::{Keystroke, point, px, size};

    use super::{Fonts, Geometry, byte_of_utf16, input_of, pieces, term};

    fn keystroke(text: &str) -> Keystroke {
        Keystroke::parse(text).unwrap()
    }

    fn typed(key: &str, key_char: &str) -> Keystroke {
        let mut stroke = keystroke(key);
        stroke.key_char = Some(key_char.to_owned());
        stroke
    }

    #[test]
    fn keys_become_what_a_terminal_reads() {
        use term::{Input, Key, Modifiers};
        assert_eq!(input_of(&typed("a", "a")), Some(Input::Text("a".into())));
        assert_eq!(
            input_of(&typed("shift-a", "A")),
            Some(Input::Text("A".into()))
        );
        assert_eq!(input_of(&typed("!", "!")), Some(Input::Text("!".into())));
        assert_eq!(
            input_of(&keystroke("ctrl-c")),
            Some(Input::Key(Key::Character('c'), Modifiers::CONTROL))
        );
        assert_eq!(
            input_of(&keystroke("ctrl-shift-p")),
            Some(Input::Key(
                Key::Character('p'),
                Modifiers {
                    control: true,
                    shift: true,
                    ..Modifiers::NONE
                }
            ))
        );
        assert_eq!(
            input_of(&typed("alt-b", "b")),
            Some(Input::Key(Key::Character('b'), Modifiers::ALT))
        );
        assert_eq!(
            input_of(&keystroke("enter")),
            Some(Input::Key(Key::Enter, Modifiers::NONE))
        );
        // The toolkit's tests give Enter a character of its own.
        assert_eq!(
            input_of(&typed("enter", "\n")),
            Some(Input::Key(Key::Enter, Modifiers::NONE))
        );
        assert_eq!(
            input_of(&keystroke("shift-tab")),
            Some(Input::Key(Key::Tab, Modifiers::SHIFT))
        );
        assert_eq!(
            input_of(&keystroke("up")),
            Some(Input::Key(Key::Up, Modifiers::NONE))
        );
        assert_eq!(
            input_of(&keystroke("f5")),
            Some(Input::Key(Key::Function(5), Modifiers::NONE))
        );
        assert_eq!(
            input_of(&typed("space", " ")),
            Some(Input::Text(" ".into()))
        );
        assert_eq!(
            input_of(&keystroke("ctrl-space")),
            Some(Input::Key(Key::Character(' '), Modifiers::CONTROL))
        );
        // A key named after the character it types.
        assert_eq!(
            input_of(&typed("eacute", "é")),
            Some(Input::Text("é".into()))
        );
        assert_eq!(input_of(&keystroke("shift")), None);
        assert_eq!(input_of(&keystroke("ctrl-eacute")), None);
    }

    #[test]
    fn on_macos_option_types_and_cmd_copies() {
        use term::{Input, Key, Modifiers};
        crate::keymap::tests::as_on(crate::keymap::System::Mac);
        // Option with a key types its character; on a Hungarian layout
        // Option+W is |, which Escape in front would turn into a command.
        assert_eq!(
            input_of(&typed("alt-o", "ø")),
            Some(Input::Text("ø".into()))
        );
        assert_eq!(
            input_of(&typed("alt-w", "|")),
            Some(Input::Text("|".into()))
        );
        // With a key that types nothing it is a key still.
        assert_eq!(
            input_of(&keystroke("alt-left")),
            Some(Input::Key(Key::Left, Modifiers::ALT))
        );
        // Ctrl is the shell's.
        assert_eq!(
            input_of(&keystroke("ctrl-c")),
            Some(Input::Key(Key::Character('c'), Modifiers::CONTROL))
        );
        let own = |key: &str| {
            super::own_keys()
                .iter()
                .find(|(own, _)| *own == key)
                .map(|(_, what)| *what)
        };
        assert_eq!(own("cmd-c"), Some(super::OwnKey::Copy));
        assert_eq!(own("cmd-v"), Some(super::OwnKey::Paste));
        assert_eq!(own("ctrl-c"), None);
    }

    #[test]
    fn a_place_is_a_cell_and_a_half_of_it() {
        let geometry = Geometry {
            origin: point(px(12.0), px(12.0)),
            cell: size(px(6.0), px(14.0)),
            columns: 10,
            rows: 3,
        };
        let at = |x: f32, y: f32| geometry.cell_at(point(px(x), px(y)));
        assert_eq!(
            at(12.0, 12.0),
            (term::ViewPoint { row: 0, column: 0 }, term::Side::Left)
        );
        assert_eq!(
            at(16.0, 27.0),
            (term::ViewPoint { row: 1, column: 0 }, term::Side::Right)
        );
        // Outside the screen: held to its edges.
        assert_eq!(
            at(-50.0, -50.0),
            (term::ViewPoint { row: 0, column: 0 }, term::Side::Left)
        );
        assert_eq!(
            at(5000.0, 5000.0),
            (term::ViewPoint { row: 2, column: 9 }, term::Side::Right)
        );
        assert_eq!(
            at(f32::NAN, f32::INFINITY),
            (term::ViewPoint { row: 0, column: 0 }, term::Side::Left)
        );
        assert_eq!(geometry.rows_outside(point(px(20.0), px(0.0))), 1);
        assert_eq!(geometry.rows_outside(point(px(20.0), px(20.0))), 0);
        assert_eq!(geometry.rows_outside(point(px(20.0), px(60.0))), -1);
    }

    #[test]
    fn utf16_offsets_are_held_to_the_text() {
        assert_eq!(byte_of_utf16("aé😀b", 0), 0);
        assert_eq!(byte_of_utf16("aé😀b", 2), 3);
        assert_eq!(byte_of_utf16("aé😀b", 4), 7);
        assert_eq!(byte_of_utf16("aé😀b", 99), 8);
    }

    fn cell(character: char, foreground: term::Rgb) -> term::Cell {
        term::Cell {
            character,
            foreground,
            ..term::Cell::default()
        }
    }

    #[test]
    fn a_row_is_shaped_in_few_pieces() {
        let red = term::Rgb::new(200, 0, 0);
        let green = term::Rgb::new(0, 200, 0);
        let fonts = Fonts::of("Mono".into());
        let mut cells = vec![
            cell('a', red),
            cell('b', green),
            cell(' ', red),
            cell('c', red),
        ];
        // Many blanks end a piece.
        cells.extend(std::iter::repeat_n(cell(' ', red), 10));
        cells.push(cell('d', red));
        // A wide character and its spacer.
        cells.push(term::Cell {
            width: term::CellWidth::Wide,
            ..cell('漢', red)
        });
        cells.push(term::Cell {
            width: term::CellWidth::Spacer,
            ..cell(' ', red)
        });
        cells.push(cell('e', red));
        let row = term::Row { cells };
        let pieces = pieces(&row, &fonts, 1.0);
        let shown: Vec<(u16, &str, bool)> = pieces
            .iter()
            .map(|piece| (piece.column, piece.text.as_str(), piece.wide))
            .collect();
        assert_eq!(
            shown,
            [
                (0, "ab c", false),
                (14, "d", false),
                (15, "漢", true),
                (17, "e", false)
            ]
        );
        // Red, green with the blank after it, then red again.
        assert_eq!(pieces[0].runs.len(), 3);
        assert_eq!(pieces[0].runs.iter().map(|run| run.len).sum::<usize>(), 4);
    }
}
