//! Blame: every line of a file with the commit that last changed it. Lines
//! from one commit form a block; its first line has the author's picture,
//! what the commit says and the day it was made. Then the line's number,
//! a bar in the commit's colour and the text: the picture 7 in, the
//! numbers ending 54 in, what the commit says from 71 (190 wide), the day
//! from 263, the bar at 395, the text from 407. Clicking a block shows the
//! commit in the graph; a block can be blamed again as of the commit
//! before it.
//!
//! The file in a file's past is this view without the commit's words and
//! day ([`BlameView::set_plain`]): the picture, the number, the bar at 77
//! and the text from 101. Both have a miniature of the text at the right,
//! as the file view of a diff has.
//!
//! With the pointer resting on a block's gutter a card opens with the
//! whole message, who wrote it and when, the id, and what can be done
//! from there. Where the repository has a file of revisions to pass over
//! (`blame.ignoreRevsFile`, or `.git-blame-ignore-revs`), a switch says
//! whether they are passed over; git's configuration decides how it
//! starts, and what is chosen is kept for the repository.
//!
//! The text of the lines is selected with the pointer and copied as in a
//! diff (`text_select.rs`): a press and a drag, two clicks for a word,
//! three for the line, Shift with a click; Ctrl+C copies it.
//!
//! Sources: git-blame(1).

use std::{collections::HashMap, rc::Rc};

use bitbraid_git::{BlameLine, CommitInfo, Oid};
use gpui_kit::component::{
    ActiveTheme, Rope, Sizable, StyledExt, VirtualListScrollHandle,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    highlighter::SyntaxHighlighter,
    hover_card::HoverCard,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::{
    keymap::{BLAME_CONTEXT, CopyText},
    prefs::Prefs,
    session::{Reply, Request, SessionHandle},
    views::{
        absolute_date, absolute_time,
        avatar::avatar,
        diff_syntax::{LineStyles, language_for},
        text_select::{self, Column, LineText, Selecting},
    },
};

/// Height of a line, and the size of its text: as a diff's.
const LINE_HEIGHT: f32 = 16.0;
const TEXT_SIZE: f32 = 12.0;
/// The column of the author's picture, the picture's size and where it
/// stands in it.
const PICTURE_COLUMN: f32 = 30.0;
const PICTURE: f32 = 16.0;
const PICTURE_INSET: f32 = 7.0;
/// Width of the line numbers, which end 54 in.
const NUMBER_WIDTH: f32 = 24.0;
/// The commit's words begin 71 in and are 190 wide; the day after them.
const GUTTER_GAP: f32 = 17.0;
const SUMMARY_WIDTH: f32 = 190.0;
const DATE_WIDTH: f32 = 132.0;
/// Width of the gutter naming the commit: its words and its day.
const GUTTER_WIDTH: f32 = SUMMARY_WIDTH + 2.0 + DATE_WIDTH;
/// Width of the bar in the commit's colour, between gutter and text, and
/// what is between the bar and the text.
const BAR_WIDTH: f32 = 2.0;
const TEXT_GAP: f32 = 10.0;
/// Without the gutter (the file in a file's past): the bar at 77, the text
/// from 101.
const PLAIN_GAP: f32 = 23.0;
const PLAIN_TEXT_GAP: f32 = 22.0;
/// The miniature of the text at the right, and the strip after it.
const MINI_WIDTH: f32 = 120.0;
const RULER_WIDTH: f32 = 14.0;
/// Files beyond this many lines stay without syntax colours.
const MAX_COLOURED_LINES: usize = 20_000;

/// Events from the blame view.
#[derive(Debug, Clone)]
pub enum BlameEvent {
    /// Select this commit in the graph.
    Reveal(Oid),
    /// Show what this commit did to the file.
    Open {
        /// The commit.
        id: Oid,
        /// The file.
        path: String,
    },
}

/// The file of revisions for blame to pass over, as `repo.blame_ignores`
/// answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoreFile {
    /// Where it is.
    pub file: String,
    /// Git's configuration names it, so git passes its revisions over by
    /// itself.
    pub configured: bool,
    /// How many revisions it lists.
    pub revisions: usize,
}

impl IgnoreFile {
    /// Read the answer; nothing when there is no such file, or it is
    /// named and not there.
    pub fn from_reply(value: &serde_json::Value) -> Option<Self> {
        if value.get("exists").and_then(|there| there.as_bool()) != Some(true) {
            return None;
        }
        Some(Self {
            file: value.get("file")?.as_str()?.to_owned(),
            configured: value.get("configured").and_then(|flag| flag.as_bool()) == Some(true),
            revisions: value
                .get("revisions")
                .and_then(|count| count.as_u64())
                .and_then(|count| usize::try_from(count).ok())
                .unwrap_or(0),
        })
    }

    /// What the switch says.
    pub fn label(&self) -> String {
        say!("blame-pass-over", count = self.revisions)
    }
}

/// Whether revisions are passed over: what was chosen for the repository
/// when something was, else what git itself does (it passes them over
/// when its configuration names the file, and not for a file that is only
/// there).
pub fn passes_over(chosen: Option<bool>, file: Option<&IgnoreFile>) -> bool {
    match file {
        None => false,
        Some(file) => chosen.unwrap_or(file.configured),
    }
}

/// What the card of a commit says under its message: who, and when.
pub fn card_byline(info: &CommitInfo) -> String {
    format!(
        "{} <{}>  ·  {}",
        info.author.name,
        info.author.email,
        absolute_time(info.author.seconds, info.author.offset)
    )
}

pub struct BlameView {
    session: SessionHandle,
    path: Option<String>,
    /// The commit the file is blamed at; HEAD when absent.
    rev: Option<Oid>,
    /// The commit it was asked to be blamed at: where "back" goes after a
    /// look at the file before a block's commit.
    origin: Option<Oid>,
    /// The file of revisions to pass over, when the repository has one,
    /// and whether they are passed over.
    ignore: Option<IgnoreFile>,
    ignoring: bool,
    lines: Rc<Vec<BlameLine>>,
    /// Per line: its block's number, and whether it opens the block.
    blocks: Rc<Vec<(usize, bool)>>,
    /// Per line: the colour of its commit's bar.
    bars: Rc<Vec<u32>>,
    /// Syntax colours per line.
    colours: Rc<Vec<LineStyles>>,
    colours_dark: bool,
    /// The colours of code they were worked out with: a theme of the
    /// user's own can change them without changing the mode.
    colours_theme: std::sync::Arc<gpui_kit::component::highlighter::HighlightTheme>,
    commits: HashMap<Oid, CommitInfo>,
    sizes: crate::zoom::Sizes,
    loading: bool,
    error: Option<String>,
    scroll: VirtualListScrollHandle,
    scroll_x: f32,
    /// Without the commit's words and day: the file in a file's past.
    plain: bool,
    /// For the miniature: the runs of characters that are not blank, per
    /// line, where it was last painted, and whether the pointer is on it.
    mini: Rc<Vec<Vec<(u16, u16)>>>,
    mini_bounds: Rc<std::cell::Cell<Bounds<Pixels>>>,
    mini_hovered: bool,
    /// Text selected with the pointer, and where the list was painted
    /// last, for a selection that is dragged past its top or bottom.
    text: Selecting,
    list_bounds: Rc<std::cell::Cell<Bounds<Pixels>>>,
    focus_handle: FocusHandle,
}

impl EventEmitter<BlameEvent> for BlameView {}

/// The text of a line of the file: what a selection reads.
fn line_of(lines: &[BlameLine], row: usize) -> Option<&str> {
    lines.get(row).map(|line| line.text.as_str())
}

/// For each line, which colour the bar between its commit and its text
/// has: a commit keeps one colour through the file, and they are given out
/// in the order the commits first appear.
fn bars_of(lines: &[BlameLine]) -> Vec<u32> {
    let mut seen: HashMap<Oid, u32> = HashMap::new();
    lines
        .iter()
        .map(|line| {
            let next = u32::try_from(seen.len()).unwrap_or(u32::MAX);
            *seen.entry(line.commit).or_insert(next)
        })
        .collect()
}

/// Number the runs of consecutive lines from one commit.
fn blocks_of(lines: &[BlameLine]) -> Vec<(usize, bool)> {
    let mut out = Vec::with_capacity(lines.len());
    let mut block = 0;
    let mut previous: Option<Oid> = None;
    for line in lines {
        let opens = previous != Some(line.commit);
        if opens && previous.is_some() {
            block += 1;
        }
        previous = Some(line.commit);
        out.push((block, opens));
    }
    out
}

impl BlameView {
    pub fn new(session: SessionHandle, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            session,
            path: None,
            rev: None,
            ignore: None,
            ignoring: false,
            lines: Rc::new(Vec::new()),
            blocks: Rc::new(Vec::new()),
            bars: Rc::new(Vec::new()),
            colours: Rc::new(Vec::new()),
            colours_dark: cx.theme().is_dark(),
            colours_theme: cx.theme().highlight_theme.clone(),
            commits: HashMap::new(),
            sizes: crate::zoom::Sizes::default(),
            loading: false,
            error: None,
            scroll: VirtualListScrollHandle::new(),
            scroll_x: 0.0,
            plain: false,
            origin: None,
            mini: Rc::new(Vec::new()),
            mini_bounds: Rc::new(std::cell::Cell::new(Bounds::default())),
            mini_hovered: false,
            text: Selecting::default(),
            list_bounds: Rc::new(std::cell::Cell::new(Bounds::default())),
            focus_handle: cx.focus_handle(),
        }
    }

    /// What is shown or being loaded: the path and the commit it is blamed at.
    pub fn target(&self) -> Option<(&str, Option<Oid>)> {
        self.path.as_deref().map(|path| (path, self.rev))
    }

    /// Blame `path` as of `rev`, or of HEAD.
    pub fn show(&mut self, path: String, rev: Option<Oid>, cx: &mut Context<Self>) {
        self.origin = rev;
        self.look_at(path, rev, cx);
    }

    /// Blame `path` as of `rev`, keeping where "back" goes: after a
    /// block's "before this commit" the way back is offered.
    fn look_at(&mut self, path: String, rev: Option<Oid>, cx: &mut Context<Self>) {
        self.path = Some(path.clone());
        self.rev = rev;
        self.lines = Rc::new(Vec::new());
        self.blocks = Rc::new(Vec::new());
        self.bars = Rc::new(Vec::new());
        self.colours = Rc::new(Vec::new());
        self.sizes = crate::zoom::Sizes::default();
        self.error = None;
        self.loading = true;
        self.scroll_x = 0.0;
        cx.notify();
        // First whether there are revisions to pass over (the file may
        // have come or gone since the last time), then the blame.
        let asked = Request::Command {
            id: "repo.blame_ignores".to_owned(),
            params: json!({}),
        };
        self.session.call(asked, cx, move |this, reply, cx| {
            if this.path.as_deref() != Some(path.as_str()) || this.rev != rev {
                return;
            }
            this.ignore = match reply {
                Reply::Command(value) => IgnoreFile::from_reply(&value),
                _ => None,
            };
            let chosen = cx
                .global::<Prefs>()
                .repo_state(&this.session.path)
                .blame_ignore_revs;
            this.ignoring = passes_over(chosen, this.ignore.as_ref());
            this.load(path, rev, cx);
        });
    }

    /// Blame the file as it was before `commit`, where `path` is its name
    /// in that commit: the engine says which commit that is and the name
    /// the file had there, another where the commit renamed it. A commit
    /// that added the file has nothing before it, and that is said.
    fn blame_before(&mut self, commit: Oid, path: String, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "repo.blame_previous".to_owned(),
            params: json!({ "path": path, "rev": commit.to_hex() }),
        };
        let asked = (self.path.clone(), self.rev);
        self.session
            .call_asked(request, cx, move |this, reply, cx| {
                if (this.path.clone(), this.rev) != asked {
                    return; // something else was asked for meanwhile
                }
                match reply {
                    Reply::Command(value) => {
                        let rev = value
                            .get("rev")
                            .and_then(|rev| rev.as_str())
                            .and_then(|rev| rev.parse::<Oid>().ok());
                        let before = value.get("path").and_then(|path| path.as_str());
                        match (rev, before) {
                            (Some(rev), Some(before)) => {
                                this.look_at(before.to_owned(), Some(rev), cx)
                            }
                            _ => {
                                this.error = Some(say!(
                                    "blame-added-here",
                                    commit = commit.short(7),
                                    path = path.as_str()
                                ));
                                cx.notify();
                            }
                        }
                    }
                    Reply::Error(err) => {
                        this.error = Some(err);
                        cx.notify();
                    }
                    _ => {}
                }
            });
    }

    /// Pass the listed revisions over, or do not, and remember it for
    /// this repository.
    fn set_ignoring(&mut self, ignoring: bool, cx: &mut Context<Self>) {
        let path = self.session.path.clone();
        cx.global_mut::<Prefs>()
            .update_repo_state(&path, |state| state.blame_ignore_revs = Some(ignoring));
        if let Some(path) = self.path.clone() {
            self.look_at(path, self.rev, cx);
        }
    }

    /// Ask for the blame itself.
    fn load(&mut self, path: String, rev: Option<Oid>, cx: &mut Context<Self>) {
        // Said only where there is a file: without one there is nothing
        // to pass over, and asking for it would be refused.
        let ignore_revs = self.ignore.as_ref().map(|_| self.ignoring);
        let request = Request::Command {
            id: "repo.blame".to_owned(),
            params: json!({
                "path": path,
                "rev": rev.map(|id| id.to_hex()),
                "ignore_revs": ignore_revs,
            }),
        };
        self.session
            .call_asked(request, cx, move |this, reply, cx| {
                if this.path.as_deref() != Some(path.as_str()) || this.rev != rev {
                    return; // something else was asked for meanwhile
                }
                this.loading = false;
                match reply {
                    Reply::Command(value) => {
                        match serde_json::from_value::<Vec<BlameLine>>(value) {
                            Ok(lines) => this.set_lines(lines, cx),
                            Err(err) => this.error = Some(err.to_string()),
                        }
                    }
                    Reply::Error(err) => this.error = Some(err),
                    _ => {}
                }
                cx.notify();
            });
        cx.notify();
    }

    fn set_lines(&mut self, lines: Vec<BlameLine>, cx: &mut Context<Self>) {
        self.text.clear();
        self.blocks = Rc::new(blocks_of(&lines));
        self.bars = Rc::new(bars_of(&lines));
        self.sizes = crate::zoom::Sizes::new(vec![size(px(0.), px(LINE_HEIGHT)); lines.len()]);
        self.mini = Rc::new(
            lines
                .iter()
                .map(|line| crate::views::diff::mini_runs(&line.text))
                .collect(),
        );
        let mut wanted: Vec<Oid> = Vec::new();
        for line in &lines {
            if !self.commits.contains_key(&line.commit) && !wanted.contains(&line.commit) {
                wanted.push(line.commit);
            }
        }
        self.lines = Rc::new(lines);
        self.colour(cx);
        if !wanted.is_empty() {
            self.session
                .call(Request::Details(wanted), cx, |this, reply, cx| {
                    if let Reply::Details(details) = reply {
                        for commit in details {
                            this.commits.insert(commit.id, commit);
                        }
                        cx.notify();
                    }
                });
        }
    }

    /// Colour the whole file once; unlike a diff, blame has all of it.
    fn colour(&mut self, cx: &mut Context<Self>) {
        self.colours_dark = cx.theme().is_dark();
        self.colours_theme = cx.theme().highlight_theme.clone();
        let mut colours = vec![Vec::new(); self.lines.len()];
        let language = self.path.as_deref().and_then(language_for);
        if let Some(language) = language
            && self.lines.len() <= MAX_COLOURED_LINES
        {
            let mut text = String::new();
            let mut starts = Vec::with_capacity(self.lines.len());
            for line in self.lines.iter() {
                starts.push(text.len());
                text.push_str(&line.text);
                text.push('\n');
            }
            let mut highlighter = SyntaxHighlighter::new(language);
            highlighter.update(None, &Rope::from_str(&text), None);
            let theme = cx.theme().highlight_theme.clone();
            let mut at = 0;
            for (range, style) in highlighter.styles(&(0..text.len()), &*theme) {
                if style == HighlightStyle::default() || range.start >= range.end {
                    continue;
                }
                while self.lines.get(at).is_some_and(|line| {
                    starts.get(at).copied().unwrap_or(0) + line.text.len() <= range.start
                }) {
                    at += 1;
                }
                let mut ix = at;
                while let (Some(line), Some(start)) = (self.lines.get(ix), starts.get(ix).copied())
                {
                    if start >= range.end {
                        break;
                    }
                    let from = range.start.max(start) - start;
                    let to = range.end.min(start + line.text.len()) - start;
                    if from < to
                        && line.text.is_char_boundary(from)
                        && line.text.is_char_boundary(to)
                        && let Some(slot) = colours.get_mut(ix)
                    {
                        slot.push((from..to, style));
                    }
                    ix += 1;
                }
            }
        }
        self.colours = Rc::new(colours);
    }

    /// Horizontal wheel (or shift+wheel) moves the text sideways.
    fn scroll_sideways(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = event.delta.pixel_delta(px(20.));
        let dx = if event.modifiers.shift && delta.x == px(0.) {
            delta.y
        } else {
            delta.x
        };
        if dx == px(0.) {
            return;
        }
        let longest = self
            .lines
            .iter()
            .map(|line| line.text.chars().count())
            .max()
            .unwrap_or(0);
        let _ = cx;
        let char_width = TEXT_SIZE * 0.6;
        let max = (longest as f32 * char_width - 240.0).max(0.0);
        self.scroll_x = (self.scroll_x - crate::zoom::unpx(dx)).clamp(0.0, max);
        cx.notify();
    }

    /// The gutter of a block's first line, with the card that opens when
    /// the pointer rests on it: the whole message, who wrote it and when,
    /// the id, and what can be done from there.
    /// `path` is the file's name in the commit.
    fn card(
        &self,
        ix: usize,
        commit: Oid,
        info: Option<CommitInfo>,
        path: String,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let trigger = h_flex()
            .w(px(GUTTER_WIDTH))
            .gap(px(2.))
            .items_center()
            .child(
                div()
                    .w(px(SUMMARY_WIDTH))
                    .flex_none()
                    .truncate()
                    .child(match &info {
                        Some(info) => info.summary.clone(),
                        None => commit.short(7),
                    }),
            )
            .child(
                div()
                    .w(px(DATE_WIDTH))
                    .flex_none()
                    .whitespace_nowrap()
                    .text_color(theme.muted_foreground)
                    .child(
                        info.as_ref()
                            .map(|i| absolute_date(i.author.seconds, i.author.offset))
                            .unwrap_or_default(),
                    ),
            );
        let view = cx.entity().downgrade();
        let ui_font = theme.font_family.clone();
        HoverCard::new(("blame-card", ix))
            .anchor(Anchor::TopLeft)
            .trigger(trigger)
            .content(move |_, _, cx| {
                let theme = cx.theme().clone();
                let parent = info.as_ref().and_then(|info| info.parents.first().copied());
                let hex = commit.to_hex();
                let act = |what: Act| {
                    let (view, path) = (view.clone(), path.clone());
                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        let _ = view.update(cx, |this, cx| match what {
                            Act::Reveal => cx.emit(BlameEvent::Reveal(commit)),
                            Act::Open => cx.emit(BlameEvent::Open {
                                id: commit,
                                path: path.clone(),
                            }),
                            Act::Before => this.blame_before(commit, path.clone(), cx),
                        });
                    }
                };
                v_flex()
                    .id(("blame-card-content", ix))
                    .w(px(460.))
                    .max_h(px(360.))
                    .p_1()
                    .gap_2()
                    .font_family(ui_font.clone())
                    .text_sm()
                    .child(
                        div()
                            .font_semibold()
                            .whitespace_normal()
                            .child(match &info {
                                Some(info) => info.summary.clone(),
                                None => say!("blame-loading").to_owned(),
                            }),
                    )
                    .when_some(
                        info.as_ref()
                            .map(|info| info.body.clone())
                            .filter(|body| !body.is_empty()),
                        |card, body| {
                            card.child(
                                div()
                                    .id(("blame-card-body", ix))
                                    .max_h(px(180.))
                                    .overflow_y_scroll()
                                    .whitespace_normal()
                                    .text_color(theme.muted_foreground)
                                    .child(body),
                            )
                        },
                    )
                    .when_some(info.as_ref(), |card, info| {
                        card.child(div().text_xs().whitespace_normal().child(card_byline(info)))
                    })
                    .child(
                        div()
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(hex.clone()),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .flex_wrap()
                            .child(
                                Button::new(("card-reveal", ix))
                                    .outline()
                                    .xsmall()
                                    .label(say!("blame-show-in-graph"))
                                    .on_click(act(Act::Reveal)),
                            )
                            .child(
                                Button::new(("card-open", ix))
                                    .outline()
                                    .xsmall()
                                    .label(say!("blame-what-it-changed"))
                                    .on_click(act(Act::Open)),
                            )
                            .when(parent.is_some(), |row| {
                                row.child(
                                    Button::new(("card-before", ix))
                                        .outline()
                                        .xsmall()
                                        .label(say!("blame-before-it"))
                                        .on_click(act(Act::Before)),
                                )
                            })
                            .child(
                                Button::new(("card-copy", ix))
                                    .outline()
                                    .xsmall()
                                    .label(say!("blame-copy-id"))
                                    .on_click(move |_, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            hex.clone(),
                                        ));
                                    }),
                            ),
                    )
            })
    }

    fn render_row(&self, ix: usize, line: &BlameLine, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let (_, opens) = self.blocks.get(ix).copied().unwrap_or((0, false));
        let commit = line.commit;
        let info = self.commits.get(&commit);
        let parent = info.and_then(|info| info.parents.first().copied());
        // The file's name in the line's commit: another where it was
        // renamed since.
        let at = line
            .source_path
            .clone()
            .unwrap_or_else(|| self.path.clone().unwrap_or_default());
        // The author's picture on the first line of a block.
        let picture = div()
            .w(px(PICTURE_COLUMN))
            .h_full()
            .flex_none()
            .pl(px(PICTURE_INSET))
            .flex()
            .items_center()
            .when_some(info.filter(|_| opens), |column, info| {
                column.child(avatar(&info.author.email, px(PICTURE), theme.is_dark(), cx))
            });
        let number = div()
            .w(px(NUMBER_WIDTH))
            .flex_none()
            .text_right()
            .text_color(theme.muted_foreground)
            .child(line.line.to_string());
        let card =
            (opens && !self.plain).then(|| self.card(ix, commit, info.cloned(), at.clone(), cx));
        let gutter = (!self.plain).then(|| {
            h_flex()
                .id(("blame-gutter", ix))
                .ml(px(GUTTER_GAP))
                .w(px(GUTTER_WIDTH))
                .h_full()
                .flex_none()
                .items_center()
                .font_family(theme.font_family.clone())
                .text_xs()
                .cursor_pointer()
                .when(opens && ix > 0, |d| {
                    d.border_t_1().border_color(theme.border)
                })
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(BlameEvent::Reveal(commit))))
                .when_some(card, |d, card| {
                    // What the commit says and when, with the card of the
                    // commit behind it; the button to look behind it shows
                    // while the pointer is on the row.
                    d.group("blame-block")
                        .relative()
                        .child(card)
                        .when(parent.is_some(), |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .right_1()
                                    .top_0()
                                    .bottom_0()
                                    .flex()
                                    .items_center()
                                    .bg(theme.background)
                                    .invisible()
                                    .group_hover("blame-block", |s| s.visible())
                                    .child(
                                        Button::new(("blame-before", ix))
                                            .ghost()
                                            .xsmall()
                                            .label(say!("blame-before-this-commit"))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                cx.stop_propagation();
                                                this.blame_before(commit, at.clone(), cx);
                                            })),
                                    ),
                            )
                        })
                })
        });
        let colours = self.colours.get(ix).cloned().unwrap_or_default();
        let bar = crate::theme::lane_color(self.bars.get(ix).copied().unwrap_or(0), cx);
        let (bar_gap, text_gap) = if self.plain {
            (PLAIN_GAP, PLAIN_TEXT_GAP)
        } else {
            (0.0, TEXT_GAP)
        };
        h_flex()
            .w_full()
            .h(px(LINE_HEIGHT))
            .items_center()
            .child(picture)
            .child(number)
            .children(gutter)
            .child(
                div()
                    .ml(px(bar_gap))
                    .w(px(BAR_WIDTH))
                    .h_full()
                    .flex_none()
                    .bg(bar),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .pl(px(text_gap))
                    .overflow_hidden()
                    .cursor_text()
                    .child(
                        div()
                            .ml(px(-self.scroll_x))
                            .whitespace_nowrap()
                            .child(LineText::new(
                                StyledText::new(line.text.clone()).with_highlights(colours),
                                (ix, Column::Only, 0),
                                self.text.in_row(ix, Column::Only, line.text.len()),
                                theme.selection,
                                self.text.drawn(),
                            )),
                    ),
            )
            .into_any_element()
    }

    /// The pointer went down on the list: a selection of text begins on
    /// a line's text, and goes anywhere else.
    fn press_text(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let lines = self.lines.as_slice();
        if self.text.press(event, |row, _| line_of(lines, row)) {
            cx.notify();
        }
    }

    /// The pointer moved with its button held, wherever it is: a
    /// selection that grows follows it, and the list scrolls toward it
    /// while it is above or below the list.
    fn drag_text(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.text.is_growing() {
            return;
        }
        let list = self.list_bounds.get();
        let beyond = if position.y < list.top() {
            crate::zoom::unpx(position.y - list.top())
        } else if position.y > list.bottom() {
            crate::zoom::unpx(position.y - list.bottom())
        } else {
            0.0
        };
        if beyond != 0.0 {
            let offset = self.scroll.offset();
            let total = LINE_HEIGHT * self.lines.len() as f32;
            let room = crate::zoom::unpx(list.size.height);
            let scrolled = -crate::zoom::unpx(offset.y);
            let step = beyond.clamp(-LINE_HEIGHT * 3.0, LINE_HEIGHT * 3.0);
            let wanted = (scrolled + step).clamp(0.0, (total - room).max(0.0));
            if (wanted - scrolled).abs() >= 0.5 {
                self.scroll.set_offset(point(offset.x, px(-wanted)));
                cx.notify();
            }
        }
        let lines = self.lines.as_slice();
        if self.text.drag(position, |row, _| line_of(lines, row)) {
            cx.notify();
        }
    }

    /// The pointer's button went up: the selection stops growing, and is
    /// what a middle click pastes elsewhere on the desktop.
    fn release_text(&mut self, cx: &mut Context<Self>) {
        if self.text.release() {
            text_select::offer_primary(self.selected_text(), cx);
        }
    }

    /// The selected text, as it is copied.
    pub fn selected_text(&self) -> Option<String> {
        let lines = self.lines.as_slice();
        self.text.text(|row, _| line_of(lines, row))
    }

    /// Copy the selected text (Ctrl+C).
    fn copy_text(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// Show the file without the commit's words and day (the file in a
    /// file's past), or with them (blame).
    pub fn set_plain(&mut self, plain: bool, cx: &mut Context<Self>) {
        if self.plain != plain {
            self.plain = plain;
            cx.notify();
        }
    }

    /// Whether the commit's words and day are left out.
    pub fn is_plain(&self) -> bool {
        self.plain
    }

    /// How many lines are shown, for the off-screen tests.
    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// The miniature was clicked or dragged at window height `y`: bring
    /// the line under the pointer to the middle.
    fn mini_jump(&mut self, y: Pixels, cx: &mut Context<Self>) {
        let strip = self.mini_bounds.get();
        let height = crate::zoom::unpx(strip.size.height);
        let total = self.lines.len() as f32 * LINE_HEIGHT;
        if height <= 0.0 || total <= height {
            return;
        }
        let scrolled = -crate::zoom::unpx(self.scroll.offset().y);
        let geometry = crate::views::diff::MiniGeometry::new(self.lines.len(), total, height);
        let row = geometry.row_at(crate::zoom::unpx(y - strip.top()), scrolled);
        let top = (row as f32 * LINE_HEIGHT - height / 2.0).clamp(0.0, total - height);
        let offset = self.scroll.offset();
        self.scroll.set_offset(point(offset.x, px(-top)));
        cx.notify();
    }

    /// The miniature of the text at the right, and the strip after it.
    fn render_mini(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let (ink, view, rule) = (
            theme.foreground.opacity(0.4),
            theme.foreground.opacity(0.14),
            theme.border,
        );
        let mini = self.mini.clone();
        let rows = self.lines.len();
        let painted = self.mini_bounds.clone();
        let scrolled = -crate::zoom::unpx(self.scroll.offset().y);
        let hovered = self.mini_hovered;
        div()
            .id("blame-minimap")
            .test_support()
            .flex_none()
            .w(px(MINI_WIDTH + RULER_WIDTH))
            .h_full()
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.mini_hovered = *hovered;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.mini_jump(event.position.y, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.mini_jump(event.position.y, cx);
                }
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        painted.set(bounds);
                        let height = crate::zoom::unpx(bounds.size.height);
                        let total = rows as f32 * LINE_HEIGHT;
                        if height <= 0.0 || total <= 0.0 {
                            return;
                        }
                        crate::views::diff::paint_miniature(
                            bounds,
                            &mini,
                            (total, scrolled),
                            |row| row as f32 * LINE_HEIGHT,
                            (ink, view, rule),
                            hovered,
                            window,
                        );
                    },
                )
                .size_full(),
            )
    }
}

impl Focusable for BlameView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for BlameView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.colours_dark != cx.theme().is_dark()
            || !std::sync::Arc::ptr_eq(&self.colours_theme, &cx.theme().highlight_theme)
        {
            self.colour(cx);
        }
        let theme = cx.theme().clone();
        // Which file it is, the view around this one says. A line over the
        // lines only where there is something to choose: passing over
        // listed revisions, going back to the commit it was asked for.
        let at_rev = self.rev != self.origin && !self.plain;
        let back = match self.origin {
            Some(origin) => say!("blame-back-to", commit = origin.short(7)),
            None => say!("blame-back-to-head").to_owned(),
        };
        let lines = self.lines.clone();
        let ignore = self.ignore.clone();
        let ignoring = self.ignoring;
        let drawn = self.text.drawn();
        let painted = self.list_bounds.clone();
        let view = cx.entity().downgrade();
        let list =
            v_flex()
                .id("blame-list")
                .test_support()
                .relative()
                .size_full()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(TEXT_SIZE))
                .line_height(px(LINE_HEIGHT))
                // Laid out before the rows: what they drew in the last
                // frame is forgotten as they draw again; the window's moves
                // and the release reach the view wherever the pointer is.
                .child(
                    canvas(
                        move |_, _, _| text_select::forget_drawn(&drawn),
                        move |bounds, _, window, _| {
                            painted.set(bounds);
                            text_select::follow_pointer(
                                view.clone(),
                                |this: &mut Self, position, cx| this.drag_text(position, cx),
                                |this: &mut Self, cx| this.release_text(cx),
                                window,
                            );
                        },
                    )
                    .absolute()
                    .size_full(),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, window, cx| {
                        this.focus_handle.focus(window, cx);
                        this.press_text(event, cx)
                    }),
                )
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                    this.scroll_sideways(event, cx)
                }))
                .child(
                    v_virtual_list(
                        cx.entity().clone(),
                        "blame-rows",
                        self.sizes.get(),
                        move |this, range, _window, cx| {
                            range
                                .filter_map(|ix| Some(this.render_row(ix, lines.get(ix)?, cx)))
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.scroll),
                );
        let ignore = ignore.filter(|_| !self.plain);
        let header = ignore.is_some() || at_rev;

        v_flex()
            .id("blame")
            .test_support()
            .key_context(BLAME_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, _: &CopyText, _, cx| this.copy_text(cx)))
            .size_full()
            .bg(theme.background)
            .when(self.loading, |d| {
                d.child(
                    div()
                        .p_2()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(say!("blame-loading")),
                )
            })
            .when(header, |d| {
                d.child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .py_1()
                        .gap_2()
                        .items_center()
                        .justify_end()
                        .border_b_1()
                        .border_color(theme.border)
                        // Only where the repository lists revisions to pass
                        // over.
                        .when_some(ignore, |d, ignore| {
                            d.child(
                                Checkbox::new("blame-ignore-revs")
                                    .label(ignore.label())
                                    .checked(ignoring)
                                    .tooltip(say!(
                                        "blame-ignore-revs-tooltip",
                                        file = ignore.file.as_str()
                                    ))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.set_ignoring(*checked, cx)
                                    })),
                            )
                        })
                        .when(at_rev, |d| {
                            d.child(
                                Button::new("blame-head")
                                    .ghost()
                                    .xsmall()
                                    .label(back.clone())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(path) = this.path.clone() {
                                            let origin = this.origin;
                                            this.look_at(path, origin, cx);
                                        }
                                    })),
                            )
                        }),
                )
            })
            .when_some(self.error.clone(), |d, err| {
                d.child(div().p_2().text_sm().text_color(theme.danger).child(err))
            })
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().h_full().child(list))
                    .when(!self.lines.is_empty(), |row| {
                        row.child(self.render_mini(cx))
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_git::{BlameLine, CommitInfo, Oid, Signature};
    use serde_json::json;

    use super::{IgnoreFile, bars_of, blocks_of, card_byline, passes_over};

    #[test]
    fn the_file_of_revisions_is_read_and_git_decides_how_the_switch_starts() {
        let configured = IgnoreFile::from_reply(&json!({
            "file": "/r/.git-blame-ignore-revs", "configured": true, "exists": true,
            "revisions": 3,
        }))
        .unwrap();
        assert_eq!(configured.label(), "Pass over 3 listed revisions");
        let conventional = IgnoreFile::from_reply(&json!({
            "file": "/r/.git-blame-ignore-revs", "configured": false, "exists": true,
            "revisions": 1,
        }))
        .unwrap();
        assert_eq!(conventional.label(), "Pass over 1 listed revision");
        // No file, a file that is named and gone, an answer that is none.
        for none in [
            json!(null),
            json!({ "file": "/r/x", "configured": true, "exists": false, "revisions": 0 }),
            json!({ "exists": true }),
            json!("nonsense"),
        ] {
            assert_eq!(IgnoreFile::from_reply(&none), None, "{none}");
        }
        // As git does it, until something was chosen here.
        assert!(passes_over(None, Some(&configured)));
        assert!(!passes_over(None, Some(&conventional)));
        assert!(passes_over(Some(true), Some(&conventional)));
        assert!(!passes_over(Some(false), Some(&configured)));
        // Nothing to pass over, whatever was chosen once.
        assert!(!passes_over(Some(true), None));
    }

    #[test]
    fn the_card_says_who_wrote_the_commit_and_when() {
        let who = Signature {
            name: "Ada Fixture".to_owned(),
            email: "ada@fixture.invalid".to_owned(),
            seconds: 1_577_836_800,
            offset: 3600,
        };
        let info = CommitInfo {
            id: "ab".repeat(20).parse().unwrap(),
            parents: Vec::new(),
            author: who.clone(),
            committer: who,
            summary: "a summary".to_owned(),
            body: String::new(),
        };
        assert_eq!(
            card_byline(&info),
            "Ada Fixture <ada@fixture.invalid>  ·  2020-01-01 01:00"
        );
    }

    #[test]
    fn consecutive_lines_of_one_commit_form_a_block() {
        let oid = |b: u8| -> Oid { format!("{b:02x}").repeat(20).parse().unwrap() };
        let lines: Vec<BlameLine> = [1u8, 1, 2, 1, 1, 3]
            .into_iter()
            .enumerate()
            .map(|(ix, commit)| BlameLine {
                line: ix as u32 + 1,
                commit: oid(commit),
                source_line: 1,
                text: String::new(),
                source_path: None,
            })
            .collect();
        assert_eq!(
            blocks_of(&lines),
            [
                (0, true),
                (0, false),
                (1, true),
                (2, true),
                (2, false),
                (3, true)
            ]
        );
        assert!(blocks_of(&[]).is_empty());
        // A commit that comes back further down has the colour it had: the
        // bars tell commits apart, not blocks.
        assert_eq!(bars_of(&lines), [0, 0, 1, 0, 0, 2]);
        assert!(bars_of(&[]).is_empty());
    }
}

/// What a button of a commit's card does.
#[derive(Debug, Clone, Copy)]
enum Act {
    /// Show the commit in the graph.
    Reveal,
    /// Show what the commit did to the file.
    Open,
    /// Blame the file as it was before this commit.
    Before,
}
