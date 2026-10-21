//! Notifications: what is said in a toast is also kept, so that it can be
//! read again, and a toast can carry something to do about it.
//!
//! Every notification goes through [`push`]. It shows the toolkit's toast
//! and adds an entry to the [`Notices`] global, which the history view
//! lists. What a notice offers to do ("Undo", "Show log") is on the toast,
//! up to [`MOST_ON_TOAST`] offers, and in the history for as long as the
//! application runs.
//!
//! The history is kept between runs in `notices.jsonl`, beside the
//! application's other files: one notice a line, as JSON, what it said and
//! when. What a notice offered to do is not kept: it is a piece of the run
//! that made it. The file is appended to and never grows without bound: no
//! more than [`KEPT`] notices are read back, a text is cut at
//! [`TEXT_LIMIT`], and the file is written anew, with the newest only, when
//! the application starts and when a run has added twice [`KEPT`]. A file
//! is not trusted: it is read up to [`FILE_LIMIT`] from its end, a line
//! that is not a notice is passed over, and nothing in it can stop the
//! application from starting.
//!
//! Sources: the JSON Lines convention (one value per line); open(2)
//! (`O_APPEND`), rename(2).

use std::{
    collections::VecDeque,
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
    rc::Rc,
};

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    notification::{Notification, NotificationSettings},
    v_flex,
};
use gpui_kit::{
    Anchor, App, Global, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    Window, div,
};
use serde::{Deserialize, Serialize};

use crate::zoom::px;

/// What kind of news a notice is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Something to know.
    Info,
    /// Something worked.
    Success,
    /// Something to look at.
    Warning,
    /// Something failed.
    Error,
}

impl Kind {
    /// The word the file has for it.
    fn word(self) -> &'static str {
        match self {
            Self::Info => "info",
            Self::Success => "success",
            Self::Warning => "warning",
            Self::Error => "error",
        }
    }

    /// What a toast of this kind is titled: a word of ours.
    pub fn title(self) -> &'static str {
        match self {
            Self::Info => say!("notices-title-info"),
            Self::Success => say!("notices-title-success"),
            Self::Warning => say!("notices-title-warning"),
            Self::Error => say!("notices-title-error"),
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        match word {
            "info" => Some(Self::Info),
            "success" => Some(Self::Success),
            "warning" => Some(Self::Warning),
            "error" => Some(Self::Error),
            _ => None,
        }
    }
}

/// What the button of a notice does when it is pressed.
pub type Run = Rc<dyn Fn(&mut Window, &mut App)>;

/// Something a notice offers to do.
#[derive(Clone)]
pub struct NoticeAction {
    /// What its button says.
    pub label: SharedString,
    /// What it does.
    pub run: Run,
}

impl std::fmt::Debug for NoticeAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NoticeAction")
            .field("label", &self.label)
            .finish()
    }
}

/// A notification to be shown and kept.
#[derive(Debug, Clone)]
pub struct Notice {
    kind: Kind,
    text: String,
    actions: Vec<NoticeAction>,
    stays: bool,
}

impl Notice {
    fn new(kind: Kind, text: impl Into<String>) -> Self {
        Self {
            kind,
            text: text.into(),
            actions: Vec::new(),
            stays: false,
        }
    }

    /// Something to know.
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(Kind::Info, text)
    }

    /// Something worked.
    pub fn success(text: impl Into<String>) -> Self {
        Self::new(Kind::Success, text)
    }

    /// Something to look at.
    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(Kind::Warning, text)
    }

    /// Something failed.
    pub fn error(text: impl Into<String>) -> Self {
        Self::new(Kind::Error, text)
    }

    /// Whether the toast goes away by itself: only a success's does, and
    /// not when told not to.
    pub fn autohide(mut self, autohide: bool) -> Self {
        self.stays = !autohide;
        self
    }

    /// Offer something to do about it. Every offer is on the toast, up to
    /// [`MOST_ON_TOAST`], and all of them are in the history.
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        run: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.actions.push(NoticeAction {
            label: label.into(),
            run: Rc::new(run),
        });
        self
    }
}

/// A notice that was shown.
#[derive(Debug, Clone)]
pub struct Entry {
    /// What kind of news it was.
    pub kind: Kind,
    /// What it said.
    pub text: String,
    /// When, in unix seconds.
    pub at: i64,
    /// What it offered to do; nothing for one of an earlier run.
    pub actions: Vec<NoticeAction>,
    /// Whether it was said in an earlier run of the application.
    pub earlier: bool,
}

/// How many notices are kept.
pub const KEPT: usize = 200;

/// How many letters of a notice are kept between runs.
pub const TEXT_LIMIT: usize = 2000;

/// How much of the file is read, from its end, in bytes.
pub const FILE_LIMIT: u64 = 512 * 1024;

/// How many of a notice's offers get a button on the toast.
pub const MOST_ON_TOAST: usize = 3;

/// Width of a toast.
pub const TOAST_WIDTH: f32 = 352.0;
/// Width of the band in the colour of its kind at a toast's left.
const TOAST_BAND: f32 = 36.0;
/// From the window's left edge to a toast.
const TOAST_LEFT: f32 = 16.0;
/// From the top of the status bar to a toast's bottom.
const TOAST_ABOVE_STATUS: f32 = 7.0;

/// Where toasts stand and how wide they are: at the bottom left, over the
/// left panel, just above the status bar.
pub fn place(settings: &mut NotificationSettings, status_bar: f32) {
    settings.placement = Anchor::BottomLeft;
    settings.width = px(TOAST_WIDTH);
    settings.margins.left = px(TOAST_LEFT);
    settings.margins.bottom = px(status_bar + TOAST_ABOVE_STATUS);
}

/// A notice as a line of the file.
#[derive(Debug, Serialize, Deserialize)]
struct Line {
    kind: String,
    text: String,
    at: i64,
}

impl Line {
    fn of(entry: &Entry) -> Self {
        Self {
            kind: entry.kind.word().to_owned(),
            text: entry.text.chars().take(TEXT_LIMIT).collect(),
            at: entry.at,
        }
    }
}

/// What was read of the file: the notices, and whether the file is worth
/// writing anew (it held more than is kept, or something that is no
/// notice).
#[derive(Debug, Default)]
pub struct Kept {
    /// The newest notices, oldest first.
    pub entries: Vec<Entry>,
    /// The file holds more than these, or something that is no notice.
    pub untidy: bool,
}

/// Read the notices of earlier runs. A file that is not there, cannot be
/// read or holds nothing that is a notice is no notices.
pub fn read_kept(path: &Path) -> Kept {
    let Ok(mut file) = std::fs::File::open(path) else {
        return Kept::default();
    };
    let length = file.metadata().map(|meta| meta.len()).unwrap_or(0);
    let from = length.saturating_sub(FILE_LIMIT);
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(from)).is_err()
        || file.take(FILE_LIMIT).read_to_end(&mut bytes).is_err()
    {
        return Kept {
            entries: Vec::new(),
            untidy: true,
        };
    }
    let mut untidy = from > 0;
    let mut lines = bytes.split(|byte| *byte == b'\n');
    if from > 0 {
        // What the cut left of a line is not one.
        lines.next();
    }
    let mut entries = Vec::new();
    for line in lines {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let read = serde_json::from_slice::<Line>(line)
            .ok()
            .and_then(|line| Some((Kind::from_word(&line.kind)?, line)));
        match read {
            Some((kind, line)) => {
                untidy |= line.text.chars().nth(TEXT_LIMIT).is_some();
                entries.push(Entry {
                    kind,
                    text: line.text.chars().take(TEXT_LIMIT).collect(),
                    at: line.at,
                    actions: Vec::new(),
                    earlier: true,
                });
            }
            None => untidy = true,
        }
    }
    if entries.len() > KEPT {
        untidy = true;
        entries.drain(..entries.len().saturating_sub(KEPT));
    }
    Kept { entries, untidy }
}

/// Add a notice to the end of the file.
fn append(path: &Path, entry: &Entry) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_vec(&Line::of(entry))?;
    line.push(b'\n');
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(&line)
}

/// Write the file anew with these notices: beside it first, then over it.
/// With none, the file goes.
fn rewrite<'a>(path: &Path, entries: impl Iterator<Item = &'a Entry>) -> std::io::Result<()> {
    let mut text = Vec::new();
    for entry in entries {
        text.extend(serde_json::to_vec(&Line::of(entry))?);
        text.push(b'\n');
    }
    if text.is_empty() {
        return match std::fs::remove_file(path) {
            Err(err) if err.kind() != std::io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        };
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut beside = path.as_os_str().to_owned();
    beside.push(".tmp");
    let beside = PathBuf::from(beside);
    std::fs::write(&beside, text)?;
    std::fs::rename(&beside, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&beside);
    })
}

/// The notices, newest last: those of this run, and before them what is
/// kept of earlier runs.
#[derive(Debug, Default)]
pub struct Notices {
    /// What was said.
    pub entries: VecDeque<Entry>,
    /// How many of them were said since the history was last looked at.
    pub unseen: usize,
    /// Where they are kept between runs, when there is a place.
    file: Option<PathBuf>,
    /// How many lines this run added to the file since it was last
    /// written anew.
    added: usize,
}

impl Global for Notices {}

impl Notices {
    /// The notices that are kept in a file, with what it holds of earlier
    /// runs; the file is tidied when it holds more than that.
    pub fn kept_in(file: PathBuf) -> Self {
        let kept = read_kept(&file);
        if kept.untidy {
            let _ = rewrite(&file, kept.entries.iter());
        }
        Self {
            entries: kept.entries.into(),
            unseen: 0,
            file: Some(file),
            added: 0,
        }
    }

    /// Keep a notice; the oldest goes when there are too many.
    pub fn keep(&mut self, kind: Kind, text: String, at: i64, actions: Vec<NoticeAction>) {
        let entry = Entry {
            kind,
            text,
            at,
            actions,
            earlier: false,
        };
        if let Some(file) = &self.file {
            // What cannot be written is not kept between runs; it is said
            // and in the history of this run all the same.
            if append(file, &entry).is_ok() {
                self.added = self.added.saturating_add(1);
            }
        }
        self.entries.push_back(entry);
        while self.entries.len() > KEPT {
            self.entries.pop_front();
        }
        if self.added >= KEPT.saturating_mul(2)
            && let Some(file) = &self.file
        {
            let _ = rewrite(file, self.entries.iter());
            self.added = 0;
        }
        let of_this_run = self.entries.iter().filter(|entry| !entry.earlier).count();
        self.unseen = self.unseen.saturating_add(1).min(of_this_run);
    }

    /// The history was looked at.
    pub fn seen(&mut self) {
        self.unseen = 0;
    }

    /// Forget everything, the file too.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.unseen = 0;
        self.added = 0;
        if let Some(file) = &self.file {
            let _ = rewrite(file, std::iter::empty());
        }
    }
}

/// Install the global, with what is kept of earlier runs.
pub fn init(cx: &mut App) {
    let file = cx
        .try_global::<crate::prefs::Prefs>()
        .and_then(|prefs| prefs.dir().map(|dir| dir.join("notices.jsonl")));
    cx.set_global(match file {
        Some(file) => Notices::kept_in(file),
        None => Notices::default(),
    });
}

/// Show a notice as a toast and keep it in the history.
pub fn push(window: &mut Window, cx: &mut App, notice: Notice) {
    let Notice {
        kind,
        text,
        actions,
        stays,
    } = notice;
    if cx.has_global::<Notices>() {
        cx.global_mut::<Notices>()
            .keep(kind, text.clone(), crate::views::now(), actions.clone());
    }
    // What the toast holds is drawn here: a band in the colour of its kind
    // with the kind's mark, a title, what is said (much of it git's, to be
    // read letter by letter: `crate::literal`) and what can be done about
    // it. The cross that puts it away is the toolkit's, on show while the
    // pointer is over the toast.
    let offers: Vec<NoticeAction> = actions.into_iter().take(MOST_ON_TOAST).collect();
    let mut toast = Notification::new().p_0().content(move |_, _, cx| {
        let theme = cx.theme();
        let (band, mark, icon) = match kind {
            Kind::Info => (theme.info, theme.info_foreground, IconName::Info),
            Kind::Success => (
                theme.success,
                theme.success_foreground,
                IconName::CircleCheck,
            ),
            Kind::Warning => (
                theme.warning,
                theme.warning_foreground,
                IconName::TriangleAlert,
            ),
            Kind::Error => (theme.danger, theme.danger_foreground, IconName::CircleX),
        };
        let (radius, muted) = (theme.radius_lg, theme.muted_foreground);
        let toast = cx.entity().downgrade();
        let buttons = (!offers.is_empty()).then(|| {
            h_flex()
                .mt_3()
                .gap_2()
                .flex_wrap()
                .children(offers.iter().enumerate().map(|(n, action)| {
                    let (run, toast) = (action.run.clone(), toast.clone());
                    let button = Button::new(("notice-action", n))
                        .small()
                        .label(action.label.clone())
                        .on_click(move |_, window, cx| {
                            run(window, cx);
                            let _ = toast.update(cx, |toast, cx| toast.dismiss(window, cx));
                        });
                    // The first is what is most likely wanted.
                    if n == 0 {
                        button.primary()
                    } else {
                        button.outline()
                    }
                }))
        });
        h_flex()
            .id("notice")
            .w_full()
            .items_stretch()
            .child(
                div()
                    .w(px(TOAST_BAND))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_l(radius)
                    .bg(band)
                    .child(Icon::new(icon).size_5().text_color(mark)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .pl_4()
                    .pr_8()
                    .py_4()
                    .gap_2()
                    .child(div().text_base().child(kind.title()))
                    .child(
                        div()
                            .text_sm()
                            .text_color(muted)
                            .child(crate::literal::said(text.clone(), cx)),
                    )
                    .children(buttons),
            )
            .into_any_element()
    });
    // What succeeded goes by itself; what failed, what warns and what is
    // to be known stay until they are put away, as anything does that
    // was told to.
    toast = toast.autohide(kind == Kind::Success && !stays);
    window.push_notification(toast, cx);
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::{FILE_LIMIT, KEPT, Kind, Notice, Notices, TEXT_LIMIT, read_kept};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bitbraid-notices-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn notices_are_kept_counted_and_bounded() {
        let mut notices = Notices::default();
        notices.keep(Kind::Success, "Fetched".to_owned(), 10, Vec::new());
        notices.keep(Kind::Error, "Push failed".to_owned(), 11, Vec::new());
        assert_eq!(notices.entries.len(), 2);
        assert_eq!(notices.unseen, 2);
        notices.seen();
        assert_eq!(notices.unseen, 0);
        notices.keep(Kind::Info, "One more".to_owned(), 12, Vec::new());
        assert_eq!(notices.unseen, 1);
        assert_eq!(notices.entries.back().unwrap().text, "One more");
        // Only so many are kept; the oldest go first.
        for n in 0..(KEPT + 50) {
            notices.keep(Kind::Info, format!("notice {n}"), 20, Vec::new());
        }
        assert_eq!(notices.entries.len(), KEPT);
        assert_eq!(notices.entries.front().unwrap().text, "notice 50");
        assert!(notices.unseen <= KEPT);
        notices.clear();
        assert!(notices.entries.is_empty() && notices.unseen == 0);
    }

    #[test]
    fn a_notice_carries_what_it_offers() {
        let notice = Notice::error("Push was rejected")
            .action("Pull", |_, _| {})
            .action("Show log", |_, _| {})
            .autohide(false);
        assert_eq!(notice.kind, Kind::Error);
        assert_eq!(notice.actions.len(), 2);
        assert_eq!(notice.actions[0].label.as_ref(), "Pull");
        assert!(notice.stays);
        assert!(!Notice::success("Done").stays);
    }

    #[test]
    fn the_history_is_there_again_in_the_next_run() {
        let dir = scratch("runs");
        // The folder is made when there is something to keep.
        let file = dir.join("deeper").join("notices.jsonl");
        let mut first = Notices::kept_in(file.clone());
        assert!(first.entries.is_empty() && !file.exists());
        first.keep(Kind::Success, "Fetched origin".to_owned(), 100, Vec::new());
        let offer = Notice::error("Push was rejected: \"main\"\nfetch first")
            .action("Pull", |_, _| {})
            .action("Show log", |_, _| {});
        first.keep(Kind::Error, offer.text.clone(), 101, offer.actions.clone());
        first.keep(Kind::Warning, "größer 😀".to_owned(), 102, Vec::new());
        assert_eq!(first.unseen, 3);

        // The next run: the same notices, in order, marked as earlier, with
        // nothing to do any more and none of them unseen.
        let mut second = Notices::kept_in(file.clone());
        let read: Vec<(Kind, &str, i64)> = second
            .entries
            .iter()
            .map(|entry| (entry.kind, entry.text.as_str(), entry.at))
            .collect();
        assert_eq!(
            read,
            [
                (Kind::Success, "Fetched origin", 100),
                (Kind::Error, "Push was rejected: \"main\"\nfetch first", 101),
                (Kind::Warning, "größer 😀", 102),
            ]
        );
        assert!(
            second
                .entries
                .iter()
                .all(|entry| entry.earlier && entry.actions.is_empty())
        );
        assert_eq!(second.unseen, 0);
        // What this run says comes behind them, and only that is unseen.
        second.keep(Kind::Info, "Now".to_owned(), 200, Vec::new());
        assert_eq!(second.unseen, 1);
        assert!(!second.entries.back().unwrap().earlier);
        assert_eq!(Notices::kept_in(file.clone()).entries.len(), 4);

        // Cleared is cleared for the next run too.
        second.clear();
        assert!(!file.exists());
        assert!(Notices::kept_in(file).entries.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_file_does_not_grow_without_bound() {
        let dir = scratch("bound");
        let file = dir.join("notices.jsonl");
        let mut notices = Notices::kept_in(file.clone());
        // A run that says a great deal: the file is written anew now and
        // then, and never holds more than three times what is kept.
        let long = "x".repeat(TEXT_LIMIT * 3);
        for n in 0..(KEPT * 5 + 7) {
            notices.keep(Kind::Info, format!("{n} {long}"), n as i64, Vec::new());
            if n % 97 == 0 {
                let lines = std::fs::read_to_string(&file).unwrap().lines().count();
                assert!(lines <= KEPT * 3, "{lines} lines after {n}");
            }
        }
        // A text is cut where it is kept, not where it is shown.
        assert!(notices.entries.back().unwrap().text.len() > TEXT_LIMIT * 2);
        let size = std::fs::metadata(&file).unwrap().len();
        assert!(size < (KEPT * 3 * (TEXT_LIMIT + 100)) as u64, "{size}");

        // The next run reads the newest that are kept and tidies the file.
        let next = Notices::kept_in(file.clone());
        assert!(next.entries.len() <= KEPT && next.entries.len() > KEPT / 2);
        assert!(
            next.entries
                .back()
                .unwrap()
                .text
                .starts_with(&format!("{} ", KEPT * 5 + 6))
        );
        assert!(
            next.entries
                .iter()
                .all(|entry| entry.text.chars().count() <= TEXT_LIMIT)
        );
        let lines = std::fs::read_to_string(&file).unwrap().lines().count();
        assert_eq!(lines, next.entries.len());
        assert!(!dir.join("notices.jsonl.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_that_is_anything_at_all_is_read_for_what_it_holds() {
        let dir = scratch("hostile");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("notices.jsonl");
        let good = "{\"kind\":\"error\",\"text\":\"kept\",\"at\":5}\n";
        for (broken, held) in [
            (&b""[..], 0),
            (b"\n\n\n", 0),
            (b"not json at all\n", 0),
            (b"\xff\xfe\x00\x01\n", 0),
            (b"{\"kind\":\"error\"}\n", 0),
            (b"{\"kind\":\"shouting\",\"text\":\"x\",\"at\":1}\n", 0),
            (b"{\"kind\":\"info\",\"text\":7,\"at\":1}\n", 0),
            (b"{\"kind\":\"info\",\"text\":\"x\",\"at\":\"soon\"}\n", 0),
            (b"[1,2,3]\nnull\n{}\n", 0),
            (
                b"{\"kind\":\"info\",\"text\":\"x\",\"at\":-9223372036854775808}\n",
                1,
            ),
            (
                b"{\"kind\":\"info\",\"text\":\"\\u0000\\u202e\",\"at\":1,\"more\":[1]}\n",
                1,
            ),
            (b"{\"kind\":\"info\",\"text\":\"cut", 0),
        ] {
            let mut bytes = broken.to_vec();
            bytes.extend(good.as_bytes());
            bytes.extend(broken);
            std::fs::write(&file, &bytes).unwrap();
            let kept = read_kept(&file);
            // The good line between the two halves is always found, unless
            // the half in front of it ran into it without a line's end.
            let joined = !broken.is_empty() && !broken.ends_with(b"\n");
            let expected = if joined { 0 } else { 1 + 2 * held };
            assert_eq!(
                kept.entries.len(),
                expected,
                "{:?}",
                String::from_utf8_lossy(broken)
            );
            // And the application starts with it, and tidies it.
            let notices = Notices::kept_in(file.clone());
            assert_eq!(notices.entries.len(), expected);
            let after = read_kept(&file);
            assert!(!after.untidy);
            assert_eq!(after.entries.len(), expected);
        }

        // A line longer than any notice, and a file larger than is read.
        let huge = format!(
            "{{\"kind\":\"info\",\"text\":\"{}\",\"at\":1}}\n",
            "y".repeat(FILE_LIMIT as usize)
        );
        std::fs::write(&file, format!("{good}{huge}{good}{good}")).unwrap();
        let kept = read_kept(&file);
        assert!(kept.untidy);
        assert_eq!(kept.entries.len(), 2);
        assert!(kept.entries.iter().all(|entry| entry.text == "kept"));

        // Where a file should be there is a folder, or nothing can be
        // written: nothing is kept, and nothing fails.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir_all(&file).unwrap();
        let mut notices = Notices::kept_in(file.clone());
        notices.keep(Kind::Info, "said".to_owned(), 1, Vec::new());
        assert_eq!(notices.entries.len(), 1);
        notices.clear();
        // Below a file no folder can be made, on any system.
        let in_the_way = dir.join("a-file");
        std::fs::write(&in_the_way, "in the way\n").unwrap();
        let mut nowhere = Notices::kept_in(in_the_way.join("no-such").join("notices.jsonl"));
        nowhere.keep(Kind::Info, "said".to_owned(), 1, Vec::new());
        assert_eq!(nowhere.entries.len(), 1);
        assert_eq!(
            std::fs::read_to_string(&in_the_way).unwrap(),
            "in the way\n"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
