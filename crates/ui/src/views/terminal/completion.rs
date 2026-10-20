//! Completion of git commands typed in a terminal.
//!
//! While a git command is typed at the shell's prompt, a list opens at the
//! cursor with what may take the place of the word being typed (git's
//! commands and options, branches, tags, remotes, stashes, files), the best
//! first and the first one marked. It is 480 wide, a row is 30 high and
//! eight rows show at once, the rest a scroll away; its left edge is 8 left
//! of the cursor and it stays inside the terminal, 14 from its right edge;
//! it opens under the cursor's row, or above it, 6 clear, where it does not
//! fit below. Each row has a sign 14 in from the left, the name at 34, and
//! a few words about it from 257 on. Up and Down mark another, Enter takes
//! the one marked, a click takes the one clicked, Escape puts the list away
//! for the word being typed; Tab goes to the shell unless the preferences
//! say it takes the one marked or marks the next. A list that says nothing
//! more than what is typed (the word is one of its entries already) is not
//! shown, so that Enter runs the command.
//!
//! The rest of the marked entry is shown faintly after the cursor where it
//! fits in the blank cells there, short of anything further on the line
//! (the prompt nushell, zsh, fish or tcsh can keep on the right; a
//! suggestion of the shell's own right after the cursor leaves no room),
//! and Ctrl+Space brings back a list that was put away. Taking an entry
//! types the rest of it into the shell, or rubs out the word typed and
//! types the entry where it does not go on from what is typed.
//!
//! When typing begins is told by the keys: the first key of a line,
//! typed while the shell itself is in front, takes the prompt
//! (`Session::prompt_here`), and what is typed after it is read off the
//! screen whenever the screen changes and whenever a frame is taken
//! (`Session::typed_line`): output that lands between the two is not
//! announced again, and the frame shows it. The words come from the
//! engine (`terminal.completion_words`, kept by the workspace and handed
//! to every terminal of the repository), and the line is completed by the
//! engine's `completion::complete` on the window's thread: nothing runs in
//! the shell to complete it.
//!
//! Sources: `bitbraid_engine::completion`.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use bitbraid_engine::completion::{self, CandidateKind, Completion, Place, Words};
use bitbraid_engine::completion_cmds::folder_in;
use bitbraid_terminal::{self as term, Session};
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::prefs::Prefs;
use crate::zoom::px;

/// How wide the list is.
pub const LIST_WIDTH: f32 = 480.0;
/// How high a row of it is.
pub const ROW_HEIGHT: f32 = 30.0;
/// How many rows show at once.
pub const ROWS_SHOWN: usize = 8;
/// How far left of the cursor the list's left edge is.
pub const LEFT_OF_CURSOR: f32 = 8.0;
/// How far from the terminal's right edge the list stays at the least.
pub const RIGHT_MARGIN: f32 = 14.0;
/// How far above the cursor's row a list that opens upwards ends.
pub const GAP_ABOVE: f32 = 6.0;
/// Where a row's sign, name and words begin, from its left edge.
const SIGN_AT: f32 = 14.0;
const SIGN_SIZE: f32 = 12.0;
const NAME_AT: f32 = 34.0;
const WORDS_AT: f32 = 257.0;
/// How faint the rest of the marked entry is after the cursor.
const HINT_OPACITY: f32 = 0.45;

/// What Tab does while the list is on show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKey {
    /// It goes to the shell.
    Shell,
    /// It takes the one marked.
    Accept,
    /// It marks the next one.
    Next,
}

/// How completion is set in the preferences.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// Whether git commands are completed.
    pub on: bool,
    /// What Tab does.
    pub tab: TabKey,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            on: true,
            tab: TabKey::Shell,
        }
    }
}

impl Settings {
    /// As the preferences say now; the defaults where there are none.
    pub fn of(cx: &App) -> Self {
        let Some(prefs) = cx.try_global::<Prefs>() else {
            return Self::default();
        };
        let terminal = &prefs.preferences.terminal_panel;
        Self {
            on: terminal.completion,
            tab: match terminal.completion_tab.as_str() {
                "accept" => TabKey::Accept,
                "next" => TabKey::Next,
                _ => TabKey::Shell,
            },
        }
    }
}

/// The list on show.
#[derive(Debug, Clone, PartialEq)]
pub struct Shown {
    /// The word being typed and its entries.
    pub completion: Completion,
    /// The entry marked.
    pub marked: usize,
    /// The first entry on show.
    pub first: usize,
    /// Where the cursor is on the screen.
    pub cursor: term::ViewPoint,
    /// How many blank cells follow the cursor on the line before
    /// something stands there (the shell's prompt on the right, a
    /// suggestion of its own); none when nothing does.
    pub room_after: Option<usize>,
}

impl Shown {
    fn count(&self) -> usize {
        self.completion.candidates.len()
    }

    /// Mark an entry, held to the list, and scroll it into view.
    pub fn mark(&mut self, index: usize) {
        let last = self.count().saturating_sub(1);
        self.marked = index.min(last);
        if self.marked < self.first {
            self.first = self.marked;
        } else if self.marked >= self.first.saturating_add(ROWS_SHOWN) {
            self.first = self.marked.saturating_add(1).saturating_sub(ROWS_SHOWN);
        }
    }

    /// Mark the entry `by` away; past either end it goes round.
    pub fn step(&mut self, by: isize) {
        let count = self.count();
        if count == 0 {
            return;
        }
        let count_signed = isize::try_from(count).unwrap_or(isize::MAX);
        let at = isize::try_from(self.marked).unwrap_or(0);
        let moved = at.saturating_add(by);
        let next = if moved < 0 || moved >= count_signed {
            if by.unsigned_abs() > 1 {
                // A page past the end stops at it.
                if moved < 0 { 0 } else { count_signed - 1 }
            } else {
                moved.rem_euclid(count_signed)
            }
        } else {
            moved
        };
        self.mark(usize::try_from(next).unwrap_or(0));
    }

    /// Scroll the rows on show by `rows`.
    pub fn scroll(&mut self, rows: isize) {
        let most = self.count().saturating_sub(ROWS_SHOWN);
        let first = isize::try_from(self.first)
            .unwrap_or(0)
            .saturating_add(rows)
            .max(0);
        self.first = usize::try_from(first).unwrap_or(0).min(most);
    }

    /// The rest of the marked entry after what is typed, when it goes on
    /// from it and fits in the blank cells after the cursor with one to
    /// spare before whatever stands further on (a prompt the shell shows
    /// on the right), so that it never covers anything of the shell's.
    pub fn hint(&self) -> Option<String> {
        if self.completion.word.is_empty() {
            return None;
        }
        let candidate = self.completion.candidates.get(self.marked)?;
        let rest = candidate
            .insert
            .strip_prefix(self.completion.word.as_str())?;
        let fits = self
            .room_after
            .is_none_or(|room| rest.chars().count() < room);
        (fits && !rest.is_empty() && !rest.chars().any(char::is_control)).then(|| rest.to_owned())
    }
}

/// What a key does to the completions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyUse {
    /// Not theirs: it goes to the shell.
    Pass,
    /// Theirs, and done: nothing goes to the shell.
    Taken,
    /// Theirs: the list is to be read again from the screen.
    Reopen,
    /// Take this entry.
    Accept(usize),
}

/// The completions of one terminal: where typing began, whether a line is
/// being typed, and the list on show.
#[derive(Debug, Default)]
pub struct Completer {
    words: Option<Arc<Words>>,
    prompt: Option<term::Prompt>,
    /// A key that changes the line was typed since the prompt: the list
    /// may open.
    typing: bool,
    /// The list was put away for the word that begins here.
    dismissed: Option<usize>,
    shown: Option<Shown>,
    /// The line last worked out, up to the cursor, and where the cursor
    /// was.
    last: Option<(String, term::ViewPoint)>,
}

impl Completer {
    /// The repository's names to complete with.
    pub fn set_words(&mut self, words: Arc<Words>) {
        self.words = Some(words);
        self.last = None;
    }

    /// The list on show.
    pub fn shown(&self) -> Option<&Shown> {
        self.shown.as_ref()
    }

    /// The prompt typing began after, while a line is typed.
    pub fn prompt(&self) -> Option<&term::Prompt> {
        self.prompt.as_ref()
    }

    /// A key, before it goes to the shell: whether the list takes it.
    pub fn key(&mut self, input: &term::Input, settings: Settings) -> KeyUse {
        use term::{Input, Key, Modifiers};
        if !settings.on {
            return KeyUse::Pass;
        }
        if let Input::Key(Key::Character(' '), held) = input
            && *held == Modifiers::CONTROL
        {
            if self.prompt.is_none() {
                return KeyUse::Pass;
            }
            self.dismissed = None;
            self.typing = true;
            self.last = None;
            return KeyUse::Reopen;
        }
        let Some(shown) = self.shown.as_mut() else {
            return KeyUse::Pass;
        };
        let Input::Key(key, held) = input else {
            return KeyUse::Pass;
        };
        let plain = *held == Modifiers::NONE;
        match key {
            Key::Up if plain => shown.step(-1),
            Key::Down if plain => shown.step(1),
            Key::PageUp if plain => shown.step(-isize::try_from(ROWS_SHOWN).unwrap_or(8)),
            Key::PageDown if plain => shown.step(isize::try_from(ROWS_SHOWN).unwrap_or(8)),
            Key::Enter if plain => return KeyUse::Accept(shown.marked),
            Key::Tab if plain => match settings.tab {
                TabKey::Shell => return KeyUse::Pass,
                TabKey::Accept => return KeyUse::Accept(shown.marked),
                TabKey::Next => shown.step(1),
            },
            Key::Tab if *held == Modifiers::SHIFT && settings.tab == TabKey::Next => shown.step(-1),
            Key::Escape if plain => {
                self.dismissed = Some(shown.completion.start);
                self.shown = None;
            }
            _ => return KeyUse::Pass,
        }
        KeyUse::Taken
    }

    /// A key or text on its way to the shell, after the list passed it.
    /// The first of a line takes the prompt, while the shell itself is in
    /// front (not a command it runs).
    pub fn typed(&mut self, input: &term::Input, session: &Session, settings: Settings) {
        use term::{Input, Key};
        if !settings.on {
            self.forget_line();
            return;
        }
        if self.prompt.is_none() {
            self.prompt = session.prompt_here();
        }
        match input {
            Input::Key(Key::Enter, _) => self.forget_line(),
            Input::Key(Key::Character('c' | 'd'), held) if held.control => self.forget_line(),
            // The shell's history: what comes is not typed.
            Input::Key(Key::Up | Key::Down, _) => {
                self.typing = false;
                self.shown = None;
            }
            Input::Text(_) | Input::Key(Key::Backspace | Key::Delete, _) => self.typing = true,
            _ => {}
        }
    }

    /// Text pasted into the shell: typed, as far as the list goes.
    pub fn pasted(&mut self, session: &Session, settings: Settings) {
        self.typed(&term::Input::Text(String::new()), session, settings);
    }

    /// The line is done with: the next key begins another.
    pub fn forget_line(&mut self) {
        self.prompt = None;
        self.typing = false;
        self.dismissed = None;
        self.shown = None;
        self.last = None;
    }

    fn close(&mut self) -> bool {
        self.shown.take().is_some()
    }

    /// Work out what the list shows from the screen; whether that
    /// changed.
    pub fn refresh(&mut self, session: &Session, settings: Settings) -> bool {
        if !settings.on || !self.typing {
            return self.close();
        }
        let Some(prompt) = self.prompt.as_ref() else {
            return self.close();
        };
        let Some(line) = session.typed_line(prompt) else {
            return self.close();
        };
        if self
            .last
            .as_ref()
            .is_some_and(|(before, at)| *before == line.before && *at == line.cursor)
        {
            return false;
        }
        self.last = Some((line.before.clone(), line.cursor));
        let empty = Words::default();
        let words = self.words.as_deref();
        let place = Place {
            folder: words
                .and_then(|words| folder_in(Path::new(&words.top), &session.current_folder())),
            shell: session.kind(),
        };
        let Some(found) = completion::complete(&line.before, words.unwrap_or(&empty), &place)
        else {
            self.dismissed = None;
            return self.close();
        };
        if self.dismissed == Some(found.start) {
            return self.close();
        }
        self.dismissed = None;
        if found.candidates.is_empty()
            || found
                .candidates
                .iter()
                .any(|candidate| candidate.insert == found.word)
        {
            return self.close();
        }
        // The entry marked stays marked while it is still there.
        let marked = self
            .shown
            .as_ref()
            .filter(|shown| shown.completion.start == found.start)
            .and_then(|shown| shown.completion.candidates.get(shown.marked))
            .and_then(|marked| {
                found
                    .candidates
                    .iter()
                    .position(|candidate| candidate.label == marked.label)
            })
            .unwrap_or(0);
        let mut shown = Shown {
            completion: found,
            marked: 0,
            first: 0,
            cursor: line.cursor,
            room_after: (!line.after.is_empty())
                .then(|| line.after.chars().take_while(|c| *c == ' ').count()),
        };
        shown.mark(marked);
        self.shown = Some(shown);
        true
    }

    /// Mark an entry (the pointer is on it).
    pub fn mark(&mut self, index: usize) -> bool {
        match self.shown.as_mut() {
            Some(shown) if shown.marked != index => {
                shown.mark(index);
                true
            }
            _ => false,
        }
    }

    /// Scroll the list.
    pub fn scroll(&mut self, rows: isize) -> bool {
        match self.shown.as_mut() {
            Some(shown) => {
                let before = shown.first;
                shown.scroll(rows);
                shown.first != before
            }
            None => false,
        }
    }

    /// Take an entry: what is typed into the shell for it. Typing goes on,
    /// so that a folder taken lists what is in it.
    pub fn accept(&mut self, index: usize) -> Option<Vec<term::Input>> {
        use term::{Input, Key, Modifiers};
        let shown = self.shown.take()?;
        let candidate = shown.completion.candidates.get(index)?;
        let word = shown.completion.word.as_str();
        let mut out = Vec::new();
        match candidate.insert.strip_prefix(word) {
            Some(rest) => {
                if !rest.is_empty() {
                    out.push(Input::Text(rest.to_owned()));
                }
            }
            None => {
                for _ in word.chars() {
                    out.push(Input::Key(Key::Backspace, Modifiers::NONE));
                }
                out.push(Input::Text(candidate.insert.clone()));
            }
        }
        self.typing = true;
        self.last = None;
        Some(out)
    }
}

/// The sign of an entry.
fn sign(kind: CandidateKind) -> gpui_kit::assets::IconName {
    use gpui_kit::assets::IconName;
    match kind {
        CandidateKind::Command => IconName::SquareTerminal,
        CandidateKind::Word => IconName::CornerDownRight,
        CandidateKind::Option => IconName::SlidersHorizontal,
        CandidateKind::Branch => IconName::GitBranch,
        CandidateKind::RemoteBranch => IconName::Cloud,
        CandidateKind::Tag => IconName::Tag,
        CandidateKind::Remote => IconName::Globe,
        CandidateKind::Stash => IconName::Archive,
        CandidateKind::File => IconName::File,
        CandidateKind::Folder => IconName::Folder,
    }
}

/// Where the list stands, in the window: left of the cursor's cell by a
/// little and inside the terminal's text, under the cursor's row or, where
/// it does not fit there, above it.
pub fn place(cell: Bounds<Pixels>, area: Bounds<Pixels>, rows: usize) -> Point<Pixels> {
    let width = px(LIST_WIDTH);
    let shown = rows.clamp(1, ROWS_SHOWN);
    let height = px(ROW_HEIGHT) * shown as f32;
    let right_most = area.origin.x + area.size.width - px(RIGHT_MARGIN) - width;
    let x = (cell.origin.x - px(LEFT_OF_CURSOR))
        .min(right_most)
        .max(area.origin.x);
    let below = cell.origin.y + cell.size.height;
    let bottom = area.origin.y + area.size.height;
    let y = if below + height <= bottom {
        below
    } else {
        cell.origin.y - px(GAP_ABOVE) - height
    };
    point(x, y)
}

/// Something done with an entry of the list, by its place.
pub type OnEntry = Rc<dyn Fn(usize, &mut Window, &mut App)>;
/// The wheel turned over the list, by rows.
pub type OnWheel = Rc<dyn Fn(isize, &mut Window, &mut App)>;

/// What the list does when it is used.
pub struct ListHandlers {
    /// An entry was clicked.
    pub pick: OnEntry,
    /// The pointer is on an entry.
    pub hover: OnEntry,
    /// The wheel turned over it, by rows.
    pub wheel: OnWheel,
}

/// The list, drawn over everything at `at`.
pub fn list(shown: &Shown, at: Point<Pixels>, handlers: ListHandlers, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let count = shown.completion.candidates.len();
    let visible = count.clamp(1, ROWS_SHOWN);
    let rows: Vec<AnyElement> = shown
        .completion
        .candidates
        .iter()
        .enumerate()
        .skip(shown.first)
        .take(ROWS_SHOWN)
        .map(|(index, candidate)| {
            let marked = index == shown.marked;
            let pick = handlers.pick.clone();
            let hover = handlers.hover.clone();
            let sign_color = match candidate.kind {
                CandidateKind::Command | CandidateKind::Word => theme.primary,
                CandidateKind::Branch | CandidateKind::RemoteBranch => theme.success,
                CandidateKind::Tag | CandidateKind::Stash => theme.warning,
                CandidateKind::Folder => theme.info,
                CandidateKind::Option | CandidateKind::Remote | CandidateKind::File => {
                    theme.muted_foreground
                }
            };
            div()
                .id(("terminal-completion-row", index))
                .relative()
                .h(px(ROW_HEIGHT))
                .w_full()
                .flex_none()
                .cursor_pointer()
                .when(marked, |row| row.bg(theme.list_active))
                .when(!marked, |row| row.hover(|style| style.bg(theme.list_hover)))
                .on_mouse_move(move |_, window, cx| hover(index, window, cx))
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    pick(index, window, cx)
                })
                .child(
                    div()
                        .absolute()
                        .left(px(SIGN_AT))
                        .top_0()
                        .h_full()
                        .flex()
                        .items_center()
                        .child(
                            Icon::new(sign(candidate.kind))
                                .size(px(SIGN_SIZE))
                                .text_color(sign_color),
                        ),
                )
                .child(
                    h_flex()
                        .absolute()
                        .left(px(NAME_AT))
                        .w(px(WORDS_AT - NAME_AT - 8.0))
                        .top_0()
                        .h_full()
                        .items_center()
                        .font_family(theme.mono_font_family.clone())
                        .text_size(px(12.0))
                        .text_color(theme.popover_foreground)
                        .child(div().min_w_0().truncate().child(candidate.label.clone())),
                )
                .child(
                    h_flex()
                        .absolute()
                        .left(px(WORDS_AT))
                        .right(px(12.0))
                        .top_0()
                        .h_full()
                        .items_center()
                        .text_size(px(12.0))
                        .text_color(theme.muted_foreground)
                        .child(div().min_w_0().truncate().child(candidate.detail.clone())),
                )
                .into_any_element()
        })
        .collect();
    // A thumb that says how much of the list shows, where there is more.
    let thumb = (count > ROWS_SHOWN).then(|| {
        let whole = px(ROW_HEIGHT) * visible as f32;
        let share = ROWS_SHOWN as f32 / count as f32;
        let top = shown.first as f32 / count as f32;
        div()
            .absolute()
            .right(px(2.0))
            .top(whole * top)
            .w(px(4.0))
            .h((whole * share).max(px(12.0)))
            .rounded(px(2.0))
            .bg(theme.muted_foreground.opacity(0.5))
    });
    let wheel = handlers.wheel.clone();
    deferred(
        anchored().position(at).child(
            div()
                .id("terminal-completion")
                .occlude()
                .relative()
                .w(px(LIST_WIDTH))
                .h(px(ROW_HEIGHT) * visible as f32)
                .overflow_hidden()
                .bg(theme.popover)
                .border_1()
                .border_color(theme.border)
                .rounded(theme.radius)
                .shadow_md()
                .on_scroll_wheel(move |event, window, cx| {
                    let rows = match event.delta {
                        ScrollDelta::Lines(delta) => -delta.y,
                        ScrollDelta::Pixels(delta) => -f32::from(delta.y) / ROW_HEIGHT,
                    };
                    if rows.is_finite() && rows.abs() >= 0.5 {
                        cx.stop_propagation();
                        // Held to what a list can scroll.
                        wheel(rows.round().clamp(-1000.0, 1000.0) as isize, window, cx);
                    }
                })
                .children(rows)
                .children(thumb),
        ),
    )
    .with_priority(2)
    .into_any_element()
}

/// The rest of the marked entry, faint, from the cursor's cell on.
pub fn paint_hint(
    hint: &str,
    cell: Bounds<Pixels>,
    font: &Font,
    font_size: Pixels,
    color: Hsla,
    window: &mut Window,
    cx: &mut App,
) {
    let run = TextRun {
        len: hint.len(),
        font: font.clone(),
        color: color.opacity(HINT_OPACITY),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let shaped = window.text_system().shape_line(
        SharedString::from(hint.to_owned()),
        font_size,
        &[run],
        Some(cell.size.width),
    );
    // A line that cannot be painted leaves no hint.
    let _ = shaped.paint(
        cell.origin,
        cell.size.height,
        TextAlign::Left,
        None,
        window,
        cx,
    );
}

#[cfg(test)]
mod tests {
    use bitbraid_engine::completion::{Candidate, CandidateKind, Completion};
    use gpui_kit::{Bounds, point, px, size};

    use super::{KeyUse, ROW_HEIGHT, Settings, Shown, TabKey, place};

    fn shown(count: usize, word: &str) -> Shown {
        Shown {
            completion: Completion {
                start: 4,
                word: word.to_owned(),
                candidates: (0..count)
                    .map(|n| Candidate {
                        label: format!("ch{n}"),
                        insert: format!("ch{n}"),
                        kind: CandidateKind::Command,
                        detail: String::new(),
                    })
                    .collect(),
            },
            marked: 0,
            first: 0,
            cursor: Default::default(),
            room_after: None,
        }
    }

    #[test]
    fn marking_goes_round_and_keeps_the_mark_in_view() {
        let mut list = shown(20, "ch");
        list.step(-1);
        assert_eq!((list.marked, list.first), (19, 12));
        list.step(1);
        assert_eq!((list.marked, list.first), (0, 0));
        list.step(8);
        assert_eq!((list.marked, list.first), (8, 1));
        list.step(100);
        assert_eq!(list.marked, 19);
        list.mark(500);
        assert_eq!(list.marked, 19);
        list.scroll(-100);
        assert_eq!(list.first, 0);
        list.scroll(100);
        assert_eq!(list.first, 12);
        let mut empty = shown(0, "");
        empty.step(1);
        empty.scroll(3);
        assert_eq!((empty.marked, empty.first), (0, 0));
    }

    #[test]
    fn the_hint_is_the_rest_of_the_marked_entry() {
        let mut list = shown(3, "ch");
        list.mark(1);
        assert_eq!(list.hint().as_deref(), Some("1"));
        // Something right after the cursor (a suggestion of the shell's
        // own): no room.
        list.room_after = Some(0);
        assert_eq!(list.hint(), None);
        // A prompt on the right, further on: room enough, with a cell to
        // spare before it.
        list.room_after = Some(2);
        assert_eq!(list.hint().as_deref(), Some("1"));
        list.room_after = Some(1);
        assert_eq!(list.hint(), None);
        let other = shown(1, "zz");
        assert_eq!(other.hint(), None);
    }

    #[test]
    fn the_list_stands_under_the_cursor_or_above_it_inside_the_terminal() {
        let area = Bounds {
            origin: point(px(200.0), px(400.0)),
            size: size(px(700.0), px(300.0)),
        };
        let cell = |x: f32, y: f32| Bounds {
            origin: point(px(x), px(y)),
            size: size(px(6.0), px(14.0)),
        };
        // Room below: under the row, 8 left of the cursor.
        assert_eq!(
            place(cell(300.0, 420.0), area, 3),
            point(px(292.0), px(434.0))
        );
        // No room below: above it, 6 clear.
        let above = place(cell(300.0, 600.0), area, 8);
        assert_eq!(above, point(px(292.0), px(600.0 - 6.0 - ROW_HEIGHT * 8.0)));
        // Far right: kept 14 from the edge; far left: at the edge.
        assert_eq!(
            place(cell(880.0, 420.0), area, 1).x,
            px(900.0 - 14.0 - 480.0)
        );
        assert_eq!(place(cell(201.0, 420.0), area, 1).x, px(200.0));
    }

    #[test]
    fn keys_the_list_takes() {
        use bitbraid_terminal::{Input, Key, Modifiers};
        let mut completer = super::Completer::default();
        let on = Settings::default();
        // Without a list every key goes to the shell.
        let down = Input::Key(Key::Down, Modifiers::NONE);
        assert_eq!(completer.key(&down, on), KeyUse::Pass);
        completer.shown = Some(shown(3, "ch"));
        assert_eq!(completer.key(&down, on), KeyUse::Taken);
        assert_eq!(
            completer.key(&Input::Key(Key::Enter, Modifiers::NONE), on),
            KeyUse::Accept(1)
        );
        let tab = Input::Key(Key::Tab, Modifiers::NONE);
        assert_eq!(completer.key(&tab, on), KeyUse::Pass);
        let accept = Settings {
            tab: TabKey::Accept,
            ..on
        };
        assert_eq!(completer.key(&tab, accept), KeyUse::Accept(1));
        let next = Settings {
            tab: TabKey::Next,
            ..on
        };
        assert_eq!(completer.key(&tab, next), KeyUse::Taken);
        assert_eq!(completer.shown().map(|shown| shown.marked), Some(2));
        // Letters go to the shell; Escape puts the list away.
        assert_eq!(completer.key(&Input::Text("x".into()), on), KeyUse::Pass);
        assert_eq!(
            completer.key(&Input::Key(Key::Escape, Modifiers::NONE), on),
            KeyUse::Taken
        );
        assert!(completer.shown().is_none());
        // Off: nothing is the list's.
        let off = Settings { on: false, ..on };
        completer.shown = Some(shown(3, "ch"));
        assert_eq!(completer.key(&down, off), KeyUse::Pass);
    }

    #[test]
    fn taking_an_entry_types_the_rest_or_the_whole() {
        use bitbraid_terminal::{Input, Key, Modifiers};
        let mut completer = super::Completer {
            shown: Some(shown(3, "ch")),
            ..super::Completer::default()
        };
        assert_eq!(completer.accept(2), Some(vec![Input::Text("2".into())]));
        let mut fuzzy = shown(1, "c1");
        fuzzy.completion.candidates[0].insert = "cherry".into();
        completer.shown = Some(fuzzy);
        assert_eq!(
            completer.accept(0),
            Some(vec![
                Input::Key(Key::Backspace, Modifiers::NONE),
                Input::Key(Key::Backspace, Modifiers::NONE),
                Input::Text("cherry".into()),
            ])
        );
        assert_eq!(completer.accept(0), None);
    }
}
