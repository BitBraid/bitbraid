//! What the picture service answered, kept on disk.
//!
//! One folder, in the cache directory of the system (never where the
//! configuration is: everything in it can be thrown away and asked for
//! again). A picture is the file `<hash>.img`, exactly as it came; "there
//! is no picture" is the empty file `<hash>.none`. How old an answer is,
//! is the file's own date. The file `service` names whom the answers are
//! from; with another service the folder starts over.
//!
//! It does not grow without end: the number of answers and the bytes of
//! the pictures both have a bound, and when one is passed the oldest
//! answers go until there is a quarter of room again. A file that is not
//! what it should be (empty, too large, cut off, of an odd name) costs
//! that one answer and nothing else. A folder that cannot be written is a
//! cache that keeps nothing, not an error.
//!
//! Sources: the standard library (`fs`, `SystemTime`); the XDG Base
//! Directory Specification (what a cache directory is for).

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use super::decode::MAX_BYTES;

const DAY: Duration = Duration::from_secs(24 * 3600);

/// How much is kept and for how long.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bounds {
    /// Most answers kept: pictures and "no picture" together.
    pub entries: usize,
    /// Most bytes of pictures kept.
    pub bytes: u64,
    /// How long a picture is shown before it is asked for again.
    pub picture: Duration,
    /// How long "no picture" is believed before it is asked again.
    pub absent: Duration,
}

impl Default for Bounds {
    fn default() -> Self {
        Self {
            entries: 4096,
            bytes: 32 * 1024 * 1024,
            picture: DAY * 7,
            absent: DAY * 3,
        }
    }
}

/// What is kept for a hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kept {
    /// A picture, and whether it is recent enough to go by.
    Picture {
        /// The file as it came.
        bytes: Vec<u8>,
        /// Not yet due to be asked for again.
        fresh: bool,
    },
    /// The service had none, and whether that is recent enough to go by.
    Absent {
        /// Not yet due to be asked again.
        fresh: bool,
    },
    /// Nothing is known.
    Nothing,
}

#[derive(Debug, Default, Clone, Copy)]
struct Tally {
    entries: usize,
    bytes: u64,
}

/// The folder of kept answers.
#[derive(Debug)]
pub struct Cache {
    dir: PathBuf,
    service: String,
    bounds: Bounds,
    /// Whether the folder is there and ours, found out at the first use.
    ready: OnceLock<bool>,
    tally: Mutex<Tally>,
    /// Numbers the files that are being written.
    writes: AtomicU64,
}

/// Whether `text` is a hash as files are named by: 64 hexadecimal digits
/// in lower case. Nothing else ever becomes part of a path.
pub fn is_hash(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// What a file in the folder is, by its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Named {
    Picture,
    Absent,
    /// A file that was being written when something stopped.
    Unfinished,
    Other,
}

fn named(name: &str) -> Named {
    match name.rsplit_once('.') {
        Some((hash, "img")) if is_hash(hash) => Named::Picture,
        Some((hash, "none")) if is_hash(hash) => Named::Absent,
        Some((_, "part")) => Named::Unfinished,
        _ => Named::Other,
    }
}

/// How old a file is. One dated after now is new, unless it is dated more
/// than a day ahead: then the clock was wrong when it was written, and it
/// is as old as can be.
fn age(modified: SystemTime, now: SystemTime) -> Duration {
    match now.duration_since(modified) {
        Ok(age) => age,
        Err(ahead) if ahead.duration() > DAY => Duration::MAX,
        Err(_) => Duration::ZERO,
    }
}

impl Cache {
    /// The cache in `dir` for answers of `service`. Nothing is read or
    /// written until the first answer is looked for or kept.
    pub fn new(dir: PathBuf, service: &str, bounds: Bounds) -> Self {
        Self {
            dir,
            service: service.to_owned(),
            bounds,
            ready: OnceLock::new(),
            tally: Mutex::new(Tally::default()),
            writes: AtomicU64::new(0),
        }
    }

    /// Where the answers are kept.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn picture_path(&self, hash: &str) -> PathBuf {
        self.dir.join(format!("{hash}.img"))
    }

    fn absent_path(&self, hash: &str) -> PathBuf {
        self.dir.join(format!("{hash}.none"))
    }

    /// Every answer in the folder: its path, its date and its bytes.
    fn answers(&self) -> Vec<(PathBuf, SystemTime, u64)> {
        let Ok(entries) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut answers = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            match named(name) {
                Named::Picture => answers.push((entry.path(), modified, metadata.len())),
                Named::Absent => answers.push((entry.path(), modified, 0)),
                // What a run that was stopped left half written.
                Named::Unfinished => {
                    if age(modified, SystemTime::now()) > Duration::from_secs(3600) {
                        let _ = fs::remove_file(entry.path());
                    }
                }
                Named::Other => {}
            }
        }
        answers
    }

    fn count(&self) {
        let answers = self.answers();
        if let Ok(mut tally) = self.tally.lock() {
            *tally = Tally {
                entries: answers.len(),
                bytes: answers.iter().map(|(_, _, bytes)| *bytes).sum(),
            };
        }
    }

    /// Make the folder ours: there, and holding answers of this service
    /// only. False when it cannot be.
    fn ready(&self) -> bool {
        *self.ready.get_or_init(|| {
            if fs::create_dir_all(&self.dir).is_err() {
                return false;
            }
            let label = self.dir.join("service");
            let same = fs::read_to_string(&label).is_ok_and(|kept| kept.trim() == self.service);
            if !same {
                // Answers of another service are not this one's.
                for (path, _, _) in self.answers() {
                    let _ = fs::remove_file(path);
                }
                if self
                    .write("service", self.service.as_bytes(), SystemTime::now())
                    .is_none()
                {
                    return false;
                }
            }
            self.count();
            self.trim();
            true
        })
    }

    /// Write a file whole or not at all: under another name first. It is
    /// dated `now`, which is the moment the answer came.
    fn write(&self, name: &str, bytes: &[u8], now: SystemTime) -> Option<()> {
        let number = self.writes.fetch_add(1, Ordering::Relaxed);
        let part = self
            .dir
            .join(format!("{name}.{}.{number}.part", std::process::id()));
        let written = fs::File::create(&part)
            .and_then(|mut file| {
                file.write_all(bytes)?;
                file.set_modified(now)
            })
            .and_then(|()| fs::rename(&part, self.dir.join(name)));
        if written.is_err() {
            let _ = fs::remove_file(&part);
        }
        written.ok()
    }

    /// When a bound is passed: the oldest answers go, until a quarter of
    /// each bound is free.
    fn trim(&self) {
        let over =
            |tally: &Tally| tally.entries > self.bounds.entries || tally.bytes > self.bounds.bytes;
        if !self.tally.lock().is_ok_and(|tally| over(&tally)) {
            return;
        }
        let mut answers = self.answers();
        answers.sort_by_key(|(_, modified, _)| *modified);
        let mut tally = Tally {
            entries: answers.len(),
            bytes: answers.iter().map(|(_, _, bytes)| *bytes).sum(),
        };
        let (entries, bytes) = (self.bounds.entries / 4 * 3, self.bounds.bytes / 4 * 3);
        for (path, _, size) in answers {
            if tally.entries <= entries && tally.bytes <= bytes {
                break;
            }
            let _ = fs::remove_file(path);
            tally.entries = tally.entries.saturating_sub(1);
            tally.bytes = tally.bytes.saturating_sub(size);
        }
        if let Ok(mut kept) = self.tally.lock() {
            *kept = tally;
        }
    }

    /// What is kept for `hash`, as of `now`.
    pub fn look(&self, hash: &str, now: SystemTime) -> Kept {
        if !is_hash(hash) || !self.ready() {
            return Kept::Nothing;
        }
        let path = self.picture_path(hash);
        if let Ok(metadata) = fs::metadata(&path) {
            let size = usize::try_from(metadata.len()).unwrap_or(usize::MAX);
            // Looked at before it is read: a file of any size can be there.
            let bytes = (metadata.is_file() && size > 0 && size <= MAX_BYTES)
                .then(|| fs::read(&path).ok())
                .flatten()
                .filter(|bytes| !bytes.is_empty() && bytes.len() <= MAX_BYTES);
            match bytes {
                Some(bytes) => {
                    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                    return Kept::Picture {
                        bytes,
                        fresh: age(modified, now) < self.bounds.picture,
                    };
                }
                // Not a file a picture was written to: it goes.
                None => self.forget(hash),
            }
        }
        match fs::metadata(self.absent_path(hash)).and_then(|metadata| metadata.modified()) {
            Ok(modified) => Kept::Absent {
                fresh: age(modified, now) < self.bounds.absent,
            },
            Err(_) => Kept::Nothing,
        }
    }

    /// Take away what is kept for `hash`.
    fn forget(&self, hash: &str) {
        let mut gone = Tally::default();
        if let Ok(metadata) = fs::metadata(self.picture_path(hash))
            && fs::remove_file(self.picture_path(hash)).is_ok()
        {
            gone.entries = gone.entries.saturating_add(1);
            gone.bytes = gone.bytes.saturating_add(metadata.len());
        }
        if fs::remove_file(self.absent_path(hash)).is_ok() {
            gone.entries = gone.entries.saturating_add(1);
        }
        if let Ok(mut tally) = self.tally.lock() {
            tally.entries = tally.entries.saturating_sub(gone.entries);
            tally.bytes = tally.bytes.saturating_sub(gone.bytes);
        }
    }

    /// Keep the picture the service sent for `hash` at `now`.
    pub fn keep_picture(&self, hash: &str, bytes: &[u8], now: SystemTime) {
        if !is_hash(hash) || bytes.is_empty() || bytes.len() > MAX_BYTES || !self.ready() {
            return;
        }
        self.forget(hash);
        if self.write(&format!("{hash}.img"), bytes, now).is_some() {
            if let Ok(mut tally) = self.tally.lock() {
                tally.entries = tally.entries.saturating_add(1);
                tally.bytes = tally
                    .bytes
                    .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            }
            self.trim();
        }
    }

    /// Keep that the service had no picture for `hash` at `now`.
    pub fn keep_absent(&self, hash: &str, now: SystemTime) {
        if !is_hash(hash) || !self.ready() {
            return;
        }
        self.forget(hash);
        if self.write(&format!("{hash}.none"), &[], now).is_some() {
            if let Ok(mut tally) = self.tally.lock() {
                tally.entries = tally.entries.saturating_add(1);
            }
            self.trim();
        }
    }

    /// How many answers are kept and how many bytes of pictures, as last
    /// counted.
    pub fn size(&self) -> (usize, u64) {
        self.tally
            .lock()
            .map(|tally| (tally.entries, tally.bytes))
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(number: u32) -> String {
        format!("{number:064x}")
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = bitbraid_testkit::scratch_root()
            .join("avatar-cache")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn small() -> Bounds {
        Bounds {
            entries: 8,
            bytes: 1000,
            ..Bounds::default()
        }
    }

    fn files(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn nothing_is_made_until_something_is_asked() {
        let dir = scratch("lazy");
        let cache = Cache::new(dir.clone(), "example.org", Bounds::default());
        assert!(!dir.exists());
        assert_eq!(cache.look(&hash(1), SystemTime::now()), Kept::Nothing);
        assert_eq!(files(&dir), ["service"]);
        assert_eq!(
            fs::read_to_string(dir.join("service")).unwrap(),
            "example.org"
        );
    }

    #[test]
    fn a_picture_and_no_picture_are_kept_and_grow_old() {
        let dir = scratch("kept");
        let cache = Cache::new(dir.clone(), "example.org", Bounds::default());
        let now = SystemTime::now();
        cache.keep_picture(&hash(1), b"picture bytes", SystemTime::now());
        cache.keep_absent(&hash(2), SystemTime::now());
        assert_eq!(
            cache.look(&hash(1), now),
            Kept::Picture {
                bytes: b"picture bytes".to_vec(),
                fresh: true
            }
        );
        assert_eq!(cache.look(&hash(2), now), Kept::Absent { fresh: true });
        assert_eq!(cache.look(&hash(3), now), Kept::Nothing);
        assert_eq!(cache.size(), (2, 13));

        // Six days on, the picture is still gone by, eight days on it is
        // due; "no picture" is due after three.
        let later = |days: u32| now + DAY * days;
        assert!(matches!(
            cache.look(&hash(1), later(6)),
            Kept::Picture { fresh: true, .. }
        ));
        assert!(matches!(
            cache.look(&hash(1), later(8)),
            Kept::Picture { fresh: false, .. }
        ));
        assert_eq!(cache.look(&hash(2), later(2)), Kept::Absent { fresh: true });
        assert_eq!(
            cache.look(&hash(2), later(4)),
            Kept::Absent { fresh: false }
        );

        // An answer takes the place of the one before it.
        cache.keep_absent(&hash(1), SystemTime::now());
        assert_eq!(cache.look(&hash(1), now), Kept::Absent { fresh: true });
        cache.keep_picture(&hash(2), b"now there is one", SystemTime::now());
        assert!(matches!(cache.look(&hash(2), now), Kept::Picture { .. }));
        assert_eq!(cache.size(), (2, 16));
        assert_eq!(
            files(&dir),
            [
                format!("{}.none", hash(1)),
                format!("{}.img", hash(2)),
                "service".to_owned()
            ]
        );

        // Another process finds what this one kept.
        let again = Cache::new(dir, "example.org", Bounds::default());
        assert_eq!(again.look(&hash(1), now), Kept::Absent { fresh: true });
        assert_eq!(again.size(), (2, 16));
    }

    #[test]
    fn a_file_dated_far_ahead_is_old() {
        let dir = scratch("ahead");
        let cache = Cache::new(dir.clone(), "example.org", Bounds::default());
        cache.keep_picture(&hash(1), b"x", SystemTime::now());
        let now = SystemTime::now();
        // Read as of a year ago: the file is dated a year ahead of that.
        let Some(then) = now.checked_sub(DAY * 365) else {
            return;
        };
        assert!(matches!(
            cache.look(&hash(1), then),
            Kept::Picture { fresh: false, .. }
        ));
        // A few minutes ahead is a clock that was set, not a wrong date.
        let Some(just) = now.checked_sub(Duration::from_secs(300)) else {
            return;
        };
        assert!(matches!(
            cache.look(&hash(1), just),
            Kept::Picture { fresh: true, .. }
        ));
    }

    #[test]
    fn only_a_hash_names_a_file() {
        let dir = scratch("names");
        let cache = Cache::new(dir.clone(), "example.org", Bounds::default());
        for name in [
            "",
            "abc",
            "../../../../etc/passwd",
            &"A".repeat(64),
            &"g".repeat(64),
            &format!("{}/", "a".repeat(63)),
            &"a".repeat(65),
        ] {
            cache.keep_picture(name, b"x", SystemTime::now());
            cache.keep_absent(name, SystemTime::now());
            assert_eq!(
                cache.look(name, SystemTime::now()),
                Kept::Nothing,
                "{name:?}"
            );
        }
        assert!(!dir.exists() || files(&dir).iter().all(|name| name == "service"));
        assert!(is_hash(&hash(77)));
    }

    #[test]
    fn the_oldest_go_when_there_are_too_many() {
        let dir = scratch("count");
        let cache = Cache::new(dir.clone(), "example.org", small());
        let start = SystemTime::now() - DAY;
        for number in 0..9u32 {
            // Each a minute newer than the one before.
            cache.keep_absent(
                &hash(number),
                start + Duration::from_secs(60 * u64::from(number)),
            );
        }
        // The ninth passed the bound of eight: six are left, the newest.
        let (entries, _) = cache.size();
        assert_eq!(entries, 6);
        let now = SystemTime::now();
        for number in 0..3u32 {
            assert_eq!(cache.look(&hash(number), now), Kept::Nothing, "{number}");
        }
        for number in 3..9u32 {
            assert_eq!(cache.look(&hash(number), now), Kept::Absent { fresh: true });
        }
        // However many are kept, it never holds more than the bound.
        for number in 100..400u32 {
            cache.keep_absent(&hash(number), SystemTime::now());
            assert!(cache.size().0 <= 8);
        }
        assert!(files(&dir).len() <= 9, "{:?}", files(&dir));
    }

    #[test]
    fn the_oldest_go_when_the_pictures_are_too_many_bytes() {
        let dir = scratch("bytes");
        let cache = Cache::new(dir.clone(), "example.org", small());
        let start = SystemTime::now() - DAY;
        for number in 0..5u32 {
            let came = start + Duration::from_secs(60 * u64::from(number));
            cache.keep_picture(&hash(number), &[7u8; 300], came);
        }
        // The two oldest went.
        let now = SystemTime::now();
        assert_eq!(cache.look(&hash(0), now), Kept::Nothing);
        assert_eq!(cache.look(&hash(1), now), Kept::Nothing);
        assert!(matches!(cache.look(&hash(2), now), Kept::Picture { .. }));
        // The fourth made 1200 bytes of 1000: down to 750 or less, which
        // is two; the fifth is the third again.
        assert_eq!(cache.size(), (3, 900));
        let on_disk: u64 = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().metadata().unwrap().len())
            .sum();
        assert!(on_disk <= 1000 + "example.org".len() as u64);
        // A file larger than a picture may be is not kept at all.
        cache.keep_picture(&hash(50), &vec![0u8; MAX_BYTES + 1], now);
        assert_eq!(cache.look(&hash(50), SystemTime::now()), Kept::Nothing);
    }

    #[test]
    fn files_that_are_not_what_they_should_be_cost_one_answer() {
        let dir = scratch("corrupt");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("service"), "example.org").unwrap();
        // An empty picture, one far too large, a folder where a picture
        // should be, something of no name of ours, and a file that was
        // being written a day ago.
        fs::write(dir.join(format!("{}.img", hash(1))), b"").unwrap();
        fs::write(
            dir.join(format!("{}.img", hash(2))),
            vec![1u8; MAX_BYTES + 10],
        )
        .unwrap();
        fs::create_dir(dir.join(format!("{}.img", hash(3)))).unwrap();
        fs::write(dir.join("notes.txt"), b"not ours").unwrap();
        let part = dir.join(format!("{}.img.1.2.part", hash(4)));
        fs::write(&part, b"half").unwrap();
        fs::File::options()
            .write(true)
            .open(&part)
            .unwrap()
            .set_modified(SystemTime::now() - DAY)
            .unwrap();
        fs::write(dir.join(format!("{}.img", hash(5))), b"good").unwrap();

        let cache = Cache::new(dir.clone(), "example.org", Bounds::default());
        let now = SystemTime::now();
        assert_eq!(cache.look(&hash(1), now), Kept::Nothing);
        assert_eq!(cache.look(&hash(2), now), Kept::Nothing);
        assert_eq!(cache.look(&hash(3), now), Kept::Nothing);
        assert!(matches!(cache.look(&hash(5), now), Kept::Picture { .. }));
        // The bad ones are gone, what is not ours is left alone.
        let left = files(&dir);
        assert!(!left.contains(&format!("{}.img", hash(1))));
        assert!(!left.contains(&format!("{}.img", hash(2))));
        assert!(!part.exists());
        assert!(left.contains(&"notes.txt".to_owned()));
        // And each can be kept anew.
        cache.keep_picture(&hash(1), b"fine now", SystemTime::now());
        assert!(matches!(cache.look(&hash(1), now), Kept::Picture { .. }));
    }

    #[test]
    fn another_service_starts_over() {
        let dir = scratch("service");
        let first = Cache::new(dir.clone(), "one.example.org", Bounds::default());
        first.keep_picture(&hash(1), b"from the first", SystemTime::now());
        first.keep_absent(&hash(2), SystemTime::now());
        let second = Cache::new(dir.clone(), "two.example.org", Bounds::default());
        let now = SystemTime::now();
        assert_eq!(second.look(&hash(1), now), Kept::Nothing);
        assert_eq!(second.look(&hash(2), now), Kept::Nothing);
        assert_eq!(files(&dir), ["service"]);
        assert_eq!(second.size(), (0, 0));
        // A label that is nonsense is another service too.
        fs::write(dir.join("service"), [0xff, 0xfe, 0x00]).unwrap();
        second.keep_picture(&hash(3), b"x", SystemTime::now());
        let third = Cache::new(dir.clone(), "two.example.org", Bounds::default());
        assert_eq!(third.look(&hash(3), now), Kept::Nothing);
    }

    #[test]
    fn a_folder_that_cannot_be_made_keeps_nothing() {
        let dir = scratch("blocked");
        fs::create_dir_all(dir.parent().unwrap()).unwrap();
        // A file where the folder should be.
        fs::write(&dir, b"in the way").unwrap();
        let cache = Cache::new(dir.join("avatars"), "example.org", Bounds::default());
        cache.keep_picture(&hash(1), b"x", SystemTime::now());
        cache.keep_absent(&hash(2), SystemTime::now());
        assert_eq!(cache.look(&hash(1), SystemTime::now()), Kept::Nothing);
        assert_eq!(cache.size(), (0, 0));
        let _ = fs::remove_file(&dir);
    }
}
