//! The activity log between runs: a file per repository in a directory
//! the application names, which is in its own configuration directory and
//! never inside a repository.
//!
//! One entry is one line of JSON. A thread of its own does the writing, so
//! that the git call an entry describes never waits on a disk: the entry
//! is handed over where the process ends and appended a moment later.
//! Reading is done by whoever asks, and remembered until the file changes.
//!
//! Nothing here grows without bound. A file is written again with its
//! newest entries, at most [`KEPT`] of them in at most half of
//! [`FILE_LIMIT`] bytes, once it has a quarter more entries than it keeps
//! or is larger than that limit. A directory holds at most [`MOST_FILES`]
//! files: one more, and the ones written longest ago go. What git was only
//! asked is not kept at all; there are hundreds of such entries for every
//! one that did something.
//!
//! A file is not trusted. It is read up to a bound and line by line; a
//! line that is too long, is not JSON or is not an entry is passed over,
//! and the next time the file is written again it is gone. Two processes
//! appending to one file (the application and the command line) at worst
//! lose a line that way.
//!
//! Sources: open(2) (`O_APPEND`), rename(2) (a file replaced as a whole);
//! the JSON Lines convention (one value per line).

use std::{
    collections::HashMap,
    fs,
    io::{Read as _, Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, mpsc},
    time::{Duration, SystemTime},
};

use super::{End, Entry};

/// How many entries a repository's file keeps.
pub const KEPT: usize = 1000;

/// How large a repository's file gets before it is written again, in
/// bytes.
pub const FILE_LIMIT: u64 = 1024 * 1024;

/// How many files a directory keeps: the repositories worked in last.
pub const MOST_FILES: usize = 100;

/// The longest line that is read as an entry. One entry's two outputs and
/// its command line come to 36 KiB before they are written as JSON, which
/// makes six bytes of a control character.
const LINE_LIMIT: usize = 256 * 1024;

/// What a file's name ends in.
const SUFFIX: &str = ".jsonl";

/// What is added to that for the file a rewrite is made in, which takes
/// the place of the other when it is complete.
const TEMPORARY: &str = ".tmp";

/// 64-bit FNV-1a: stable between versions of the compiler, which the
/// standard hasher is not.
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// The file in `dir` for the repository whose git directory is `git_dir`.
pub(super) fn file_for(dir: &Path, git_dir: &Path) -> PathBuf {
    let key = fnv1a64(git_dir.as_os_str().as_encoded_bytes());
    dir.join(format!("{key:016x}{SUFFIX}"))
}

enum Message {
    /// Append an entry to a file.
    Entry(PathBuf, Arc<Entry>),
    /// Forget what a file holds.
    Clear(PathBuf),
    /// Say when everything before this has been written.
    Flush(mpsc::SyncSender<()>),
}

/// The way to the writing thread, which is started when first needed.
static WRITER: Mutex<Option<mpsc::Sender<Message>>> = Mutex::new(None);

fn send(message: Message) -> bool {
    // A poisoned lock: the entry is not kept, the git call goes on.
    let Ok(mut slot) = WRITER.lock() else {
        return false;
    };
    if slot.is_none() {
        let (sender, receiver) = mpsc::channel();
        let started = std::thread::Builder::new()
            .name("bitbraid-activity-store".to_owned())
            .spawn(move || write_all(&receiver));
        if started.is_err() {
            return false;
        }
        *slot = Some(sender);
    }
    slot.as_ref()
        .is_some_and(|sender| sender.send(message).is_ok())
}

/// Have `entry` appended to `file`, later and on another thread.
pub(super) fn keep(file: PathBuf, entry: Arc<Entry>) {
    send(Message::Entry(file, entry));
}

/// Forget what `file` holds, after what was handed over before.
pub(super) fn clear(file: PathBuf) {
    send(Message::Clear(file));
}

/// Wait until everything handed over so far is written, for at most
/// `limit`. False when it took longer, or there is no thread to wait for.
pub fn flush(limit: Duration) -> bool {
    let (done, wait) = mpsc::sync_channel(1);
    send(Message::Flush(done)) && wait.recv_timeout(limit).is_ok()
}

/// How many lines the writing thread takes each file it has written to
/// to hold: what it counted when it first came to the file, and one for
/// each entry since. Another process writing to the same file is not
/// counted; the size of the file, which is asked every time, bounds what
/// the two write together.
type Tallies = HashMap<PathBuf, usize>;

fn write_all(messages: &mpsc::Receiver<Message>) {
    let mut tallies = Tallies::new();
    for message in messages {
        match message {
            Message::Entry(file, entry) => append(&file, &entry, &mut tallies),
            Message::Clear(file) => {
                let _ = fs::remove_file(&file);
                tallies.remove(&file);
                forget_read(&file);
            }
            Message::Flush(done) => {
                let _ = done.send(());
            }
        }
    }
}

/// One entry as a line. None for one that would not be read back.
fn line_of(entry: &Entry) -> Option<Vec<u8>> {
    let mut line = serde_json::to_vec(entry).ok()?;
    if line.len() > LINE_LIMIT {
        return None;
    }
    line.push(b'\n');
    Some(line)
}

fn append(file: &Path, entry: &Entry, tallies: &mut Tallies) {
    let Some(mut line) = line_of(entry) else {
        return;
    };
    // The file is looked at every time: it may have been written by
    // another process since, or removed, or be something else by now.
    let size = fs::metadata(file).map(|meta| meta.len()).ok();
    let lines = match (size, tallies.get(file)) {
        (Some(_), Some(lines)) => *lines,
        (Some(_), None) => read_lines(file).len(),
        (None, _) => {
            // A file that is begun: its directory is made, and room in it.
            if let Some(dir) = file.parent() {
                let _ = fs::create_dir_all(dir);
                make_room(dir);
            }
            0
        }
    };
    // A file that does not end where a line ends was written by
    // something else, or a line of ours was cut off: start a new one.
    if size.is_some_and(|size| size > 0) && !ends_a_line(file) {
        line.insert(0, b'\n');
    }
    let written = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)
        .and_then(|mut open| open.write_all(&line));
    if written.is_err() {
        // A directory that cannot be written to: nothing is kept, and
        // nothing is asked of it again for this entry.
        return;
    }
    let mut lines = lines.saturating_add(1);
    let bytes = size
        .unwrap_or(0)
        .saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
    if lines > KEPT.saturating_add(KEPT / 4) || bytes > FILE_LIMIT {
        // When that fails it is tried again with the next entry.
        lines = compact(file).unwrap_or(lines);
    }
    tallies.insert(file.to_path_buf(), lines);
}

/// Whether the last byte of a file is the end of a line.
fn ends_a_line(file: &Path) -> bool {
    let last = || -> Option<u8> {
        let mut open = fs::File::open(file).ok()?;
        open.seek(SeekFrom::End(-1)).ok()?;
        let mut byte = [0u8; 1];
        open.read_exact(&mut byte).ok()?;
        Some(u8::from_le_bytes(byte))
    };
    last() == Some(b'\n')
}

/// Remove the files of `dir` that were written longest ago, until one
/// more fits, and what a rewrite that never finished left behind. Only
/// files with our suffixes are counted or touched.
fn make_room(dir: &Path) {
    let Ok(listing) = fs::read_dir(dir) else {
        return;
    };
    let leftover = format!("{SUFFIX}{TEMPORARY}");
    let mut ours: Vec<(SystemTime, PathBuf)> = listing
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(&leftover) {
                let _ = fs::remove_file(entry.path());
            }
            name.ends_with(SUFFIX)
        })
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            meta.is_file().then(|| {
                (
                    meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    entry.path(),
                )
            })
        })
        .collect();
    if ours.len() < MOST_FILES {
        return;
    }
    ours.sort();
    let excess = ours.len().saturating_sub(MOST_FILES.saturating_sub(1));
    for (_, path) in ours.into_iter().take(excess) {
        let _ = fs::remove_file(path);
    }
}

/// The end of a file, as much of it as is ever read: twice what a file is
/// allowed to grow to. A file longer than that was not written by us.
fn read_end(file: &Path) -> Option<Vec<u8>> {
    let most = FILE_LIMIT.saturating_mul(2);
    let mut open = fs::File::open(file).ok()?;
    let length = open.metadata().ok()?.len();
    let cut = length > most;
    if cut {
        open.seek(SeekFrom::Start(length.saturating_sub(most)))
            .ok()?;
    }
    let mut bytes = Vec::new();
    open.take(most).read_to_end(&mut bytes).ok()?;
    if cut {
        // What is left of the line the cut went through is no line.
        let first = bytes.iter().position(|byte| *byte == b'\n')?;
        bytes.drain(..=first);
    }
    Some(bytes)
}

/// The lines of a file that could be entries, oldest first.
fn read_lines(file: &Path) -> Vec<Vec<u8>> {
    read_end(file)
        .unwrap_or_default()
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty() && line.len() <= LINE_LIMIT)
        .map(<[u8]>::to_vec)
        .collect()
}

/// An entry from a line, when it is one. A process that an earlier run
/// left running is not running now.
fn entry_of(line: &[u8]) -> Option<Entry> {
    let mut entry: Entry = serde_json::from_slice(line).ok()?;
    if entry.end == End::Running {
        entry.end = End::Lost;
    }
    Some(entry)
}

/// The entries a file holds, oldest first: the newest [`KEPT`] of the
/// lines that are entries.
fn read(file: &Path) -> Vec<Arc<Entry>> {
    let mut entries: Vec<Arc<Entry>> = read_lines(file)
        .iter()
        .filter_map(|line| entry_of(line))
        .map(Arc::new)
        .collect();
    let excess = entries.len().saturating_sub(KEPT);
    entries.drain(..excess);
    entries
}

/// Write a file again with the newest of its entries: at most [`KEPT`],
/// in at most half of [`FILE_LIMIT`]. What was no entry is gone with it.
fn compact(file: &Path) -> Option<usize> {
    let lines: Vec<Vec<u8>> = read(file)
        .iter()
        .filter_map(|entry| line_of(entry))
        .collect();
    let mut bytes = 0u64;
    let mut kept = 0usize;
    for line in lines.iter().rev() {
        let more = bytes.saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
        if more > FILE_LIMIT / 2 {
            break;
        }
        bytes = more;
        kept = kept.saturating_add(1);
    }
    let from = lines.len().saturating_sub(kept);
    let content: Vec<u8> = lines.get(from..).unwrap_or(&[]).concat();
    let mut name = file.file_name()?.to_os_string();
    name.push(TEMPORARY);
    let tmp = file.with_file_name(name);
    fs::write(&tmp, &content).ok()?;
    if fs::rename(&tmp, file).is_err() {
        let _ = fs::remove_file(&tmp);
        return None;
    }
    Some(kept)
}

/// A file as it was last read.
struct Read {
    length: u64,
    modified: Option<SystemTime>,
    entries: Arc<Vec<Arc<Entry>>>,
}

/// What was read, by file, until the file changes. Asked while the log is
/// looked at, which is no time to read a megabyte again and again.
static READ: Mutex<Option<HashMap<PathBuf, Read>>> = Mutex::new(None);

fn forget_read(file: &Path) {
    if let Ok(mut read) = READ.lock()
        && let Some(read) = read.as_mut()
    {
        read.remove(file);
    }
}

/// The entries `file` holds, oldest first. None are an error: a file that
/// is not there or cannot be read holds nothing.
pub(super) fn kept(file: &Path) -> Arc<Vec<Arc<Entry>>> {
    let Ok(meta) = fs::metadata(file) else {
        forget_read(file);
        return Arc::default();
    };
    let (length, modified) = (meta.len(), meta.modified().ok());
    if let Ok(read) = READ.lock()
        && let Some(known) = read.as_ref().and_then(|read| read.get(file))
        && known.length == length
        && known.modified == modified
    {
        return known.entries.clone();
    }
    // Read without the lock: another thread asking meanwhile reads too,
    // and both come to the same.
    let entries = Arc::new(read(file));
    if let Ok(mut read) = READ.lock() {
        let read = read.get_or_insert_with(HashMap::new);
        // As many files as a directory keeps, and no more.
        if read.len() >= MOST_FILES {
            read.clear();
        }
        read.insert(
            file.to_path_buf(),
            Read {
                length,
                modified,
                entries: entries.clone(),
            },
        );
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "bitbraid-activity-store-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn entry(id: u64, stderr: &str) -> Entry {
        Entry {
            id,
            directory: PathBuf::from("/somewhere"),
            repository: Some(PathBuf::from("/somewhere/.git")),
            command: format!("git fetch {id}"),
            started: 1_700_000_000_000 + id,
            duration_ms: 12,
            end: End::Exited,
            code: Some(0),
            stdout: String::new(),
            stdout_cut: false,
            stderr: stderr.to_owned(),
            stderr_cut: false,
            question: false,
            origin: Some("remote.fetch".into()),
        }
    }

    fn ids(file: &Path) -> Vec<u64> {
        read(file).iter().map(|entry| entry.id).collect()
    }

    #[test]
    fn an_entry_comes_back_as_it_was_written() {
        let dir = dir("round");
        let file = file_for(&dir, Path::new("/somewhere/.git"));
        assert_eq!(file.parent(), Some(dir.as_path()));
        assert!(file.to_string_lossy().ends_with(".jsonl"));
        let mut tallies = HashMap::new();
        let odd = entry(2, "line one\nline \"two\"\t\u{1b}[31m ünï 人\u{0}");
        append(&file, &entry(1, ""), &mut tallies);
        append(&file, &odd, &mut tallies);
        let back = read(&file);
        assert_eq!(back.len(), 2);
        assert_eq!(*back[0], entry(1, ""));
        assert_eq!(*back[1], odd);
        assert_eq!(tallies[&file], 2);
        // Another repository, another file.
        assert_ne!(file, file_for(&dir, Path::new("/elsewhere/.git")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_is_written_again_before_it_holds_too_many_entries() {
        let dir = dir("count");
        let file = dir.join("many.jsonl");
        let mut tallies = HashMap::new();
        let most = KEPT + KEPT / 4;
        for id in 1..=most as u64 {
            append(&file, &entry(id, ""), &mut tallies);
        }
        assert_eq!(
            fs::read(&file).unwrap().split(|b| *b == b'\n').count() - 1,
            most
        );
        // One more: the oldest go, the newest stay.
        append(&file, &entry(most as u64 + 1, ""), &mut tallies);
        let kept = ids(&file);
        assert_eq!(kept.len(), KEPT);
        assert_eq!(kept.first().copied(), Some(most as u64 + 2 - KEPT as u64));
        assert_eq!(kept.last().copied(), Some(most as u64 + 1));
        assert_eq!(tallies[&file], KEPT);
        assert!(!dir.join("many.jsonl.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_is_written_again_before_it_is_too_large() {
        let dir = dir("size");
        let file = dir.join("large.jsonl");
        let mut tallies = HashMap::new();
        let output = "x".repeat(16 * 1024);
        let mut id = 0u64;
        let mut largest = 0u64;
        // Far more than fits: the file never passes its bound by more
        // than the entry that took it there.
        for _ in 0..300 {
            id += 1;
            append(&file, &entry(id, &output), &mut tallies);
            largest = largest.max(fs::metadata(&file).unwrap().len());
        }
        assert!(largest <= FILE_LIMIT + 17 * 1024, "{largest}");
        let kept = ids(&file);
        assert!(kept.len() >= 30 && kept.len() < 70, "{}", kept.len());
        assert_eq!(kept.last().copied(), Some(id));
        assert!(kept.windows(2).all(|pair| pair[0] + 1 == pair[1]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_that_is_not_ours_is_read_for_what_it_has() {
        let dir = dir("corrupt");
        let file = dir.join("odd.jsonl");
        let good = |id| String::from_utf8(line_of(&entry(id, "said")).unwrap()).unwrap();
        // A line cut off, binary, other JSON, an entry with a field of
        // the wrong kind, one far too long, a blank line; and entries.
        let mut content: Vec<u8> = Vec::new();
        content.extend(good(1).as_bytes());
        content.extend(&good(2).as_bytes()[..40]);
        content.push(b'\n');
        content.extend([0u8, 159, 146, 150, 255, b'\n', b'\n']);
        content.extend(b"{\"id\": 3}\n[1, 2, 3]\n\"text\"\nnull\n");
        content.extend(
            good(4)
                .replace("\"code\":0", "\"code\":\"zero\"")
                .as_bytes(),
        );
        content.extend(
            format!(
                "{{\"id\": 5, \"command\": \"{}\"}}\n",
                "y".repeat(LINE_LIMIT)
            )
            .as_bytes(),
        );
        content.extend(good(6).replace("\"exited\"", "\"running\"").as_bytes());
        content.extend(good(7).replace("\"exited\"", "\"exploded\"").as_bytes());
        content.extend(good(8).as_bytes());
        // And no newline at the very end.
        content.extend(good(9).trim_end().as_bytes());
        fs::write(&file, &content).unwrap();
        let back = read(&file);
        assert_eq!(back.iter().map(|e| e.id).collect::<Vec<_>>(), [1, 6, 8, 9]);
        assert_eq!(
            back[1].end,
            End::Lost,
            "nothing of an earlier run still runs"
        );
        // Written again, only the entries are left.
        assert_eq!(compact(&file), Some(4));
        assert_eq!(ids(&file), [1, 6, 8, 9]);
        assert_eq!(
            fs::read(&file)
                .unwrap()
                .iter()
                .filter(|b| **b == b'\n')
                .count(),
            4
        );

        // Nothing but garbage, an empty file, a directory, nothing at all.
        fs::write(&file, vec![0xffu8; 100_000]).unwrap();
        assert!(read(&file).is_empty());
        fs::write(&file, "").unwrap();
        assert!(read(&file).is_empty());
        assert!(read(&dir).is_empty());
        assert!(read(&dir.join("absent.jsonl")).is_empty());
        assert!(kept(&dir.join("absent.jsonl")).is_empty());
        // Appending to garbage keeps the entry, and the garbage goes with
        // the next rewrite.
        fs::write(&file, vec![b'z'; 3000]).unwrap();
        let mut tallies = HashMap::new();
        append(&file, &entry(10, ""), &mut tallies);
        assert_eq!(ids(&file), [10]);
        // Also when the file became garbage after it was first written
        // to, or went away, or is a directory for a while.
        fs::write(&file, b"cut off in the middle of a li").unwrap();
        append(&file, &entry(11, ""), &mut tallies);
        assert_eq!(ids(&file), [11]);
        fs::remove_file(&file).unwrap();
        fs::create_dir(&file).unwrap();
        append(&file, &entry(12, ""), &mut tallies);
        assert!(read(&file).is_empty());
        fs::remove_dir(&file).unwrap();
        append(&file, &entry(13, ""), &mut tallies);
        assert_eq!(ids(&file), [13]);
        assert_eq!(tallies[&file], 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_file_far_too_long_is_read_from_its_end_only() {
        let dir = dir("long");
        let file = dir.join("long.jsonl");
        let line = line_of(&entry(1, &"o".repeat(8 * 1024))).unwrap();
        let mut content = Vec::new();
        let mut id = 0u64;
        while (content.len() as u64) < FILE_LIMIT * 3 {
            id += 1;
            content.extend(line_of(&entry(id, &"o".repeat(8 * 1024))).unwrap());
        }
        fs::write(&file, &content).unwrap();
        let back = ids(&file);
        assert!(back.len() as u64 <= FILE_LIMIT * 2 / line.len() as u64 + 1);
        assert_eq!(back.last().copied(), Some(id));
        assert!(back.windows(2).all(|pair| pair[0] + 1 == pair[1]));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_keeps_the_files_written_last() {
        let dir = dir("files");
        for n in 0..MOST_FILES + 5 {
            fs::write(dir.join(format!("{n:04}.jsonl")), "").unwrap();
        }
        fs::write(dir.join("profiles.toml"), "kept").unwrap();
        fs::write(dir.join("0001.jsonl.tmp"), "left by a rewrite").unwrap();
        let new = dir.join("new.jsonl");
        let mut tallies = HashMap::new();
        append(&new, &entry(1, ""), &mut tallies);
        let ours = fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .filter(|e| e.file_name().to_string_lossy().ends_with(".jsonl"))
            .count();
        assert_eq!(ours, MOST_FILES);
        assert!(new.is_file());
        assert!(dir.join("profiles.toml").is_file(), "not ours, not touched");
        assert!(!dir.join("0001.jsonl.tmp").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_was_read_is_read_again_only_when_the_file_changed() {
        let dir = dir("cache");
        let file = dir.join("cached.jsonl");
        let mut tallies = HashMap::new();
        append(&file, &entry(1, ""), &mut tallies);
        let first = kept(&file);
        assert_eq!(first.len(), 1);
        assert!(Arc::ptr_eq(&first, &kept(&file)), "the same reading");
        append(&file, &entry(2, ""), &mut tallies);
        let second = kept(&file);
        assert_eq!(second.len(), 2);
        fs::remove_file(&file).unwrap();
        assert!(kept(&file).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_thread_writes_in_the_order_it_was_asked_and_says_when_it_is_done() {
        let dir = dir("thread");
        let file = dir.join("deep/er/threaded.jsonl");
        for id in 1..=50 {
            keep(file.clone(), Arc::new(entry(id, "")));
        }
        assert!(flush(Duration::from_secs(20)));
        assert_eq!(ids(&file), (1..=50).collect::<Vec<u64>>());
        keep(file.clone(), Arc::new(entry(51, "")));
        clear(file.clone());
        keep(file.clone(), Arc::new(entry(52, "")));
        assert!(flush(Duration::from_secs(20)));
        assert_eq!(ids(&file), [52]);
        let _ = fs::remove_dir_all(&dir);
    }
}
