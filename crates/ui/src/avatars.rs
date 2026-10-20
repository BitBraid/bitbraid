//! Pictures of authors, when the user asked for them.
//!
//! Off, which is how it starts, nothing here does anything: no thread, no
//! folder, no request, and every author has the pattern that is made on
//! this machine (`views::avatar`). On, a view that is about to draw an
//! author asks here for the picture. What is in memory is handed back at
//! once; anything else gives "none yet" at once too, and the address goes
//! into a short queue for a few threads that look on disk and, failing
//! that, ask the service (`bitbraid_integrations::avatars`, which says
//! what is sent). When an answer is there the windows are drawn again and
//! find it in memory.
//!
//! Drawing a row costs one look into a table and nothing more, whatever
//! the state: the answer "none" and the answer "not now" are kept as well,
//! so that a row drawn sixty times a second asks once. The queue holds the
//! addresses shown last and forgets the oldest when it is full, so that a
//! history scrolled through at speed leaves no backlog: what is asked for
//! is what is on screen when a thread is free. The pictures in memory and
//! the addresses remembered both have a bound.
//!
//! Switched off, the threads end, a request under way is cut short, and
//! everything held is let go.
//!
//! Pictures from the providers are a second switch: on, an author's
//! e-mail address is first looked up with the connected accounts'
//! providers (`provider.avatars`, a hundred at a time, by a task of its
//! own), and the picture at the address a provider gives is fetched by
//! the same threads (`bitbraid_integrations::avatars::addressed`); an
//! address no provider knows goes to the picture service when that is on
//! too. The pictures of accounts and of the owners of remotes are asked
//! for by their addresses alone ([`picture_at`]).
//!
//! Sources: the toolkit's documentation (`RenderImage`,
//! `Window::paint_image`, `App::drop_image`, `App::observe_global`).

use std::{
    cell::RefCell,
    collections::{HashMap, VecDeque},
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use bitbraid_integrations::{
    avatars::{self as source, Avatars, Bitmap, Outcome, Service, addressed::Addressed},
    http::Client,
};
use futures::{StreamExt as _, channel::mpsc as fmpsc};
use gpui_kit::*;

use crate::prefs::Prefs;

/// The side, in pixels, of the picture that is drawn small: in a commit's
/// node and beside an author's name in a row.
const SMALL: u32 = 36;
/// The side of the one drawn larger: in the details of a commit.
const LARGE: u32 = 64;
/// Threads that look for pictures: so many requests at most at a time.
const WORKERS: usize = 4;
/// Most addresses that wait for a thread. The oldest gives way.
const WAITING: usize = 48;
/// Most pictures held in memory. The ones not drawn for longest go.
const PICTURES: usize = 512;
/// Most addresses anything is remembered about.
const REMEMBERED: usize = 8192;
/// How long "there is none" is gone by before the disk is looked at again
/// (which knows for how long the service is not to be asked).
const NONE_FOR: Duration = Duration::from_secs(3600);
/// How long "not now" is gone by before it is tried again.
const LATER_FOR: Duration = Duration::from_secs(90);
/// How long an address nobody could have a picture for is left alone.
const NEVER_FOR: Duration = Duration::from_secs(24 * 3600);
/// What a picture at an address is known by in the table, before its
/// address; an e-mail address has no `:` before its `@`, and this has.
const AT: &str = "picture-at:";
/// Most e-mail addresses looked up with a provider at once.
const LOOKUP_BATCH: usize = 100;
/// How long "no provider knows this address" is believed.
const UNKNOWN_FOR: Duration = Duration::from_secs(3600);

/// What a provider said of an e-mail address.
enum Lookup {
    /// It is being asked.
    Asked,
    /// The picture is at this address.
    At(String),
    /// No provider knows it, until then.
    Unknown { until: Instant },
}

/// What is known about one address.
enum Slot {
    /// It waits for a thread, or a thread is at it.
    Asked,
    /// Its picture, small and large, and when it was last drawn.
    Picture {
        small: Arc<RenderImage>,
        large: Arc<RenderImage>,
        drawn: u64,
    },
    /// There is none to show until then.
    None { until: Instant },
}

/// What a thread found.
enum Found {
    Picture {
        small: Arc<RenderImage>,
        large: Arc<RenderImage>,
    },
    Absent,
    Later,
}

/// What the threads share.
struct Shared {
    /// The picture service, when it is on.
    source: Option<Avatars>,
    /// Pictures at the addresses providers gave.
    addressed: Addressed,
    /// The addresses that wait, the one shown last at the back.
    waiting: Mutex<VecDeque<String>>,
    wake: Condvar,
    /// Set when the pictures were switched off: threads end, and a request
    /// under way is cut short.
    stop: AtomicBool,
}

/// The threads, while pictures are on.
struct Pool {
    shared: Arc<Shared>,
    /// Hands what the threads found to the windows; ends with the pool.
    _delivery: Task<()>,
}

impl Drop for Pool {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        self.shared.wake.notify_all();
    }
}

/// What the preferences ask for.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Wanted {
    on: bool,
    base: Option<String>,
    /// Pictures from the providers too.
    providers: bool,
}

#[derive(Default)]
struct State {
    /// Where answers are kept between runs, when anywhere.
    cache_dir: Option<PathBuf>,
    wanted: Wanted,
    /// Whom is asked, once that is known to be an address.
    service: Option<Service>,
    /// Why nobody is asked although it is switched on.
    problem: Option<String>,
    pool: Option<Pool>,
    known: HashMap<String, Slot>,
    pictures: usize,
    clock: u64,
    /// A picture came since the windows were last drawn for one.
    came: bool,
    /// What the providers said of e-mail addresses.
    lookups: HashMap<String, Lookup>,
    /// Where addresses go to be looked up, while pictures from the
    /// providers are on; the task that looks them up.
    to_look_up: Option<fmpsc::UnboundedSender<String>>,
    look_up: Option<Task<()>>,
}

/// The pictures of authors, a GPUI global.
#[derive(Default)]
pub struct Pictures {
    state: RefCell<State>,
}

impl Global for Pictures {}

/// How many requests for a picture were handed to the threads since the
/// application started. For the tests and the measurements: with pictures
/// off it stays where it is.
pub fn asked() -> usize {
    ASKED.load(Ordering::Relaxed)
}

static ASKED: AtomicUsize = AtomicUsize::new(0);

/// The pixels of a picture as the toolkit draws them: blue first.
fn frame(bitmap: Bitmap) -> Option<Arc<RenderImage>> {
    let mut pixels = bitmap.rgba;
    for pixel in pixels.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(bitmap.side, bitmap.side, pixels)?;
    Some(Arc::new(RenderImage::new(vec![image::Frame::new(buffer)])))
}

fn found(outcome: Outcome) -> Found {
    match outcome {
        Outcome::Picture(bitmaps) => {
            let mut bitmaps = bitmaps.into_iter();
            let small = bitmaps.next().and_then(frame);
            let large = bitmaps.next().and_then(frame);
            match (small, large) {
                (Some(small), Some(large)) => Found::Picture { small, large },
                _ => Found::Absent,
            }
        }
        Outcome::Absent => Found::Absent,
        Outcome::Later => Found::Later,
    }
}

/// One thread: take the address shown last, find its picture, hand it
/// over; until the pictures are switched off.
fn work(shared: &Shared, done: &fmpsc::UnboundedSender<(String, Found)>) {
    loop {
        let address = {
            let Ok(mut waiting) = shared.waiting.lock() else {
                return;
            };
            loop {
                if shared.stop.load(Ordering::Relaxed) {
                    return;
                }
                if let Some(address) = waiting.pop_back() {
                    break address;
                }
                waiting = match shared.wake.wait(waiting) {
                    Ok(waiting) => waiting,
                    Err(_) => return,
                };
            }
        };
        let outcome = match (address.strip_prefix(AT), &shared.source) {
            (Some(at), _) => shared.addressed.picture(at, &[SMALL, LARGE], &shared.stop),
            (None, Some(source)) => source.picture(&address, &[SMALL, LARGE], &shared.stop),
            (None, None) => Outcome::Absent,
        };
        if shared.stop.load(Ordering::Relaxed)
            || done.unbounded_send((address, found(outcome))).is_err()
        {
            return;
        }
    }
}

impl State {
    /// Let go of everything: the threads, the pictures, what was known.
    /// The pictures that were drawn are handed back, to be taken out of
    /// the windows.
    fn clear(&mut self) -> Vec<Arc<RenderImage>> {
        self.pool = None;
        self.pictures = 0;
        self.lookups.clear();
        self.to_look_up = None;
        self.look_up = None;
        let mut drawn = Vec::new();
        for (_, slot) in self.known.drain() {
            if let Slot::Picture { small, large, .. } = slot {
                drawn.push(small);
                drawn.push(large);
            }
        }
        drawn
    }

    /// Start the threads. None when nobody can be asked.
    fn start(&mut self, cx: &App) -> Option<()> {
        let service = self.service.clone().filter(|_| self.wanted.on);
        if service.is_none() && !self.wanted.providers {
            return None;
        }
        // Each service's answers in a folder of the cache directory.
        let cache = self.cache_dir.as_ref().map(|dir| dir.join("avatars"));
        let agent = format!("{}/{}", crate::brand::NAME, env!("CARGO_PKG_VERSION"));
        let fetch: Arc<Client> = Arc::new(Client::new(&agent));
        let shared = Arc::new(Shared {
            source: service.map(|service| Avatars::new(fetch.clone(), service, cache.clone())),
            addressed: Addressed::new(fetch, self.cache_dir.clone()),
            waiting: Mutex::new(VecDeque::new()),
            wake: Condvar::new(),
            stop: AtomicBool::new(false),
        });
        let (done, mut arrived) = fmpsc::unbounded::<(String, Found)>();
        let mut started = 0usize;
        for number in 0..WORKERS {
            let (shared, done) = (shared.clone(), done.clone());
            let spawned = thread::Builder::new()
                // not shown: the thread's name
                .name(format!("bitbraid-avatars {number}"))
                .spawn(move || work(&shared, &done));
            if spawned.is_ok() {
                started = started.saturating_add(1);
            }
        }
        if started == 0 {
            return None;
        }
        let delivery = cx.spawn(async move |cx| {
            while let Some(first) = arrived.next().await {
                // What came together is shown together.
                let mut batch = vec![first];
                while batch.len() < WAITING {
                    match arrived.try_recv() {
                        Ok(more) => batch.push(more),
                        Err(_) => break,
                    }
                }
                cx.update(|cx| deliver(batch, cx));
            }
        });
        self.pool = Some(Pool {
            shared,
            _delivery: delivery,
        });
        Some(())
    }

    /// Put an address in the queue for the threads.
    fn ask(&mut self, address: String, cx: &App) {
        if self.pool.is_none() && self.start(cx).is_none() {
            // Nobody to ask: not asked again for a while.
            let until = Instant::now().checked_add(LATER_FOR);
            if let Some(until) = until {
                self.known.insert(address, Slot::None { until });
            }
            return;
        }
        let Some(pool) = &self.pool else { return };
        let dropped = {
            let Ok(mut waiting) = pool.shared.waiting.lock() else {
                return;
            };
            waiting.push_back(address.clone());
            // The one that waited longest is likely long off screen: it
            // is asked for again when it is drawn again.
            (waiting.len() > WAITING)
                .then(|| waiting.pop_front())
                .flatten()
        };
        pool.shared.wake.notify_one();
        ASKED.fetch_add(1, Ordering::Relaxed);
        if let Some(dropped) = dropped {
            self.known.remove(&dropped);
        }
        self.known.insert(address, Slot::Asked);
        if self.known.len() > REMEMBERED {
            // What says "none" is cheap to find out again.
            self.known
                .retain(|_, slot| !matches!(slot, Slot::None { .. }));
        }
    }

    fn picture(&mut self, address: &str, large: bool, cx: &App) -> Option<Arc<RenderImage>> {
        let gravatar = self.wanted.on && self.service.is_some();
        if !gravatar && !self.wanted.providers {
            return None;
        }
        let address = source::normalised(address);
        if self.wanted.providers && source::worth_asking(&address) {
            match self.lookups.get(address.as_ref()) {
                Some(Lookup::At(at)) => {
                    let key = format!("{AT}{at}");
                    return self.held_or_asked(key, large, cx);
                }
                // Not known to any provider: looked up again once that is
                // old, and meanwhile the picture service, if on.
                Some(Lookup::Unknown { until }) if Instant::now() >= *until => {
                    self.look_up(address.into_owned());
                    return None;
                }
                Some(Lookup::Unknown { .. }) => {}
                Some(Lookup::Asked) => return None,
                None => {
                    self.look_up(address.into_owned());
                    return None;
                }
            }
        }
        if !gravatar {
            return None;
        }
        self.held_or_asked(address.into_owned(), large, cx)
    }

    /// Send an e-mail address to be looked up with the providers.
    fn look_up(&mut self, address: String) {
        let Some(sender) = &self.to_look_up else {
            return;
        };
        if sender.unbounded_send(address.clone()).is_ok() {
            self.lookups.insert(address, Lookup::Asked);
        }
        if self.lookups.len() > REMEMBERED {
            self.lookups
                .retain(|_, lookup| !matches!(lookup, Lookup::Unknown { .. }));
        }
    }

    /// The picture at a provider's address.
    fn picture_at(&mut self, at: &str, large: bool, cx: &App) -> Option<Arc<RenderImage>> {
        if !self.wanted.providers || !source::addressed::may_ask(at.trim()) {
            return None;
        }
        // A provider may answer with an address at a picture service; that
        // is asked only when the user turned that service on.
        let gravatar = self.wanted.on && self.service.is_some();
        if !gravatar && source::addressed::at_picture_service(at, self.wanted.base.as_deref()) {
            return None;
        }
        self.held_or_asked(format!("{AT}{}", at.trim()), large, cx)
    }

    /// What is held under a key (an e-mail address, or a picture's
    /// address after [`AT`]); otherwise it is asked for, once.
    fn held_or_asked(&mut self, key: String, large: bool, cx: &App) -> Option<Arc<RenderImage>> {
        let address = key;
        self.clock = self.clock.wrapping_add(1);
        match self.known.get_mut(address.as_str()) {
            Some(Slot::Picture {
                small,
                large: larger,
                drawn,
            }) => {
                *drawn = self.clock;
                return Some(if large { larger.clone() } else { small.clone() });
            }
            Some(Slot::Asked) => return None,
            Some(Slot::None { until }) if Instant::now() < *until => return None,
            Some(Slot::None { .. }) | None => {}
        }
        if address.starts_with(AT) || source::worth_asking(&address) {
            self.ask(address, cx);
        } else if let Some(until) = Instant::now().checked_add(NEVER_FOR) {
            // Nothing is sent about it, and it is not looked at again
            // with every frame.
            self.known.insert(address, Slot::None { until });
        }
        None
    }

    /// Take in what the threads found. The pictures that had to make room
    /// are handed back.
    fn take(&mut self, batch: Vec<(String, Found)>) -> Vec<Arc<RenderImage>> {
        let now = Instant::now();
        for (address, found) in batch {
            // Only what is still waited for: the rest was let go since.
            if !matches!(self.known.get(&address), Some(Slot::Asked)) {
                continue;
            }
            let slot = match found {
                Found::Picture { small, large } => {
                    self.pictures = self.pictures.saturating_add(1);
                    self.came = true;
                    Slot::Picture {
                        small,
                        large,
                        drawn: self.clock,
                    }
                }
                Found::Absent => match now.checked_add(NONE_FOR) {
                    Some(until) => Slot::None { until },
                    None => continue,
                },
                Found::Later => match now.checked_add(LATER_FOR) {
                    Some(until) => Slot::None { until },
                    None => continue,
                },
            };
            self.known.insert(address, slot);
        }
        if self.pictures <= PICTURES {
            return Vec::new();
        }
        // Too many: the quarter that was drawn longest ago goes.
        let mut by_age: Vec<(u64, String)> = self
            .known
            .iter()
            .filter_map(|(address, slot)| match slot {
                Slot::Picture { drawn, .. } => Some((*drawn, address.clone())),
                _ => None,
            })
            .collect();
        by_age.sort_unstable();
        let mut gone = Vec::new();
        for (_, address) in by_age.into_iter().take(PICTURES / 4) {
            if let Some(Slot::Picture { small, large, .. }) = self.known.remove(&address) {
                self.pictures = self.pictures.saturating_sub(1);
                gone.push(small);
                gone.push(large);
            }
        }
        gone
    }
}

fn deliver(batch: Vec<(String, Found)>, cx: &mut App) {
    let (came, gone) = cx
        .try_global::<Pictures>()
        .and_then(|pictures| {
            let mut state = pictures.state.try_borrow_mut().ok()?;
            let gone = state.take(batch);
            Some((std::mem::take(&mut state.came), gone))
        })
        .unwrap_or_default();
    for picture in gone {
        cx.drop_image(picture, None);
    }
    // "None" changes nothing that is drawn: the pattern stays.
    if came {
        cx.refresh_windows();
    }
}

/// The picture of the person with this e-mail address, when it is on and
/// there is one in memory. Otherwise none, at once: where it is on and
/// nothing is known yet, the address is queued, and the windows are drawn
/// again when its picture is there.
pub fn picture(address: &str, large: bool, cx: &App) -> Option<Arc<RenderImage>> {
    let pictures = cx.try_global::<Pictures>()?;
    let mut state = pictures.state.try_borrow_mut().ok()?;
    state.picture(address, large, cx)
}

/// The picture at a provider's address (an account's, a repository
/// owner's), when pictures from the providers are on and it is in memory;
/// otherwise none, at once, as [`picture`].
pub fn picture_at(address: &str, large: bool, cx: &App) -> Option<Arc<RenderImage>> {
    let pictures = cx.try_global::<Pictures>()?;
    let mut state = pictures.state.try_borrow_mut().ok()?;
    state.picture_at(address, large, cx)
}

/// Whether pictures from the providers are on.
pub fn from_providers(cx: &App) -> bool {
    cx.try_global::<Pictures>()
        .and_then(|pictures| {
            pictures
                .state
                .try_borrow()
                .ok()
                .map(|state| state.wanted.providers)
        })
        .unwrap_or(false)
}

/// What the providers said of a batch of e-mail addresses: the address of
/// each one's picture, or none. Asked of each account of `targets` (with
/// the repository it is asked about, when there is one), one after the
/// other, each for the addresses the ones before did not know.
async fn ask_providers(
    addresses: Vec<String>,
    targets: Vec<(String, Option<(String, String)>)>,
) -> Vec<(String, Option<String>)> {
    let mut left = addresses;
    let mut found: Vec<(String, Option<String>)> = Vec::new();
    for (account, target) in targets {
        if left.is_empty() {
            break;
        }
        let mut params = serde_json::json!({ "account": account, "emails": left });
        if let (Some((owner, name)), Some(object)) = (target, params.as_object_mut()) {
            object.insert("owner".to_owned(), owner.into());
            object.insert("name".to_owned(), name.into());
        }
        let answer = crate::providers::ask::<serde_json::Value>("provider.avatars", params).await;
        let Ok(serde_json::Value::Object(map)) = answer else {
            continue;
        };
        left.retain(|address| {
            let at = map
                .get(address)
                .and_then(|at| at.as_str())
                .map(str::trim)
                .filter(|at| source::addressed::may_ask(at));
            match at {
                Some(at) => {
                    found.push((address.clone(), Some(at.to_owned())));
                    false
                }
                None => true,
            }
        });
    }
    found.extend(left.into_iter().map(|address| (address, None)));
    found
}

/// Take in what the providers said.
fn take_lookups(found: Vec<(String, Option<String>)>, cx: &mut App) {
    let came = cx
        .try_global::<Pictures>()
        .and_then(|pictures| {
            let mut state = pictures.state.try_borrow_mut().ok()?;
            let until = Instant::now().checked_add(UNKNOWN_FOR)?;
            let mut came = false;
            for (address, at) in found {
                // Only what is still waited for.
                if !matches!(state.lookups.get(&address), Some(Lookup::Asked)) {
                    continue;
                }
                let lookup = match at {
                    Some(at) => {
                        came = true;
                        Lookup::At(at)
                    }
                    None => Lookup::Unknown { until },
                };
                state.lookups.insert(address, lookup);
            }
            Some(came)
        })
        .unwrap_or(false);
    if came {
        cx.refresh_windows();
    }
}

/// Start the task that looks e-mail addresses up with the providers.
fn start_lookups(cx: &mut App) {
    let (sender, mut addresses) = fmpsc::unbounded::<String>();
    let task = cx.spawn(async move |cx| {
        while let Some(first) = addresses.next().await {
            let mut batch = vec![first];
            while batch.len() < LOOKUP_BATCH {
                match addresses.try_recv() {
                    Ok(more) => batch.push(more),
                    Err(_) => break,
                }
            }
            batch.sort();
            batch.dedup();
            let targets = cx.update(|cx| crate::providers::lookup_targets(cx));
            let found = ask_providers(batch, targets).await;
            cx.update(|cx| take_lookups(found, cx));
        }
    });
    if let Some(pictures) = cx.try_global::<Pictures>()
        && let Ok(mut state) = pictures.state.try_borrow_mut()
    {
        state.to_look_up = Some(sender);
        state.look_up = Some(task);
    }
}

/// Whether the picture of the person with this address is in memory.
/// Asks nobody and queues nothing.
pub fn held(address: &str, cx: &App) -> bool {
    let Some(pictures) = cx.try_global::<Pictures>() else {
        return false;
    };
    let Ok(state) = pictures.state.try_borrow() else {
        return false;
    };
    matches!(
        state.known.get(source::normalised(address).as_ref()),
        Some(Slot::Picture { .. })
    )
}

/// Whether pictures are asked for, and of whom: the host's name. None
/// while it is off.
pub fn asking(cx: &App) -> Option<String> {
    let pictures = cx.try_global::<Pictures>()?;
    let state = pictures.state.try_borrow().ok()?;
    state
        .wanted
        .on
        .then(|| state.service.as_ref().map(Service::name))
        .flatten()
}

/// Whom pictures are asked of when that is switched on.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Whom {
    /// The host of the service. None when the address of a service of
    /// one's own is none that can be asked: then nobody is.
    pub host: Option<String>,
    /// Whether it is asked over TLS (a service that is not, is on this
    /// machine).
    pub secure: bool,
    /// What is wrong with the address, when something is.
    pub problem: Option<String>,
    /// The folder answers are kept in, when in one.
    pub kept_in: Option<PathBuf>,
}

/// Whom pictures are asked of when that is switched on, for the
/// preferences to say.
pub fn whom(cx: &App) -> Whom {
    let Some(pictures) = cx.try_global::<Pictures>() else {
        return Whom::default();
    };
    let Ok(state) = pictures.state.try_borrow() else {
        return Whom::default();
    };
    Whom {
        host: state.service.as_ref().map(Service::name),
        secure: state.service.as_ref().is_some_and(Service::secure),
        problem: state.problem.clone(),
        kept_in: state.cache_dir.as_ref().map(|dir| dir.join("avatars")),
    }
}

/// Bring what runs in line with what the preferences say. Switched off,
/// or pointed at another service, everything stops and is let go, now.
pub fn sync(cx: &mut App) {
    let Some(prefs) = cx.try_global::<Prefs>() else {
        return;
    };
    let wanted = Wanted {
        on: prefs.preferences.gravatar,
        base: prefs.preferences.gravatar_base.clone(),
        providers: prefs.preferences.provider_pictures,
    };
    let gone = {
        let Some(pictures) = cx.try_global::<Pictures>() else {
            return;
        };
        let Ok(mut state) = pictures.state.try_borrow_mut() else {
            return;
        };
        if state.wanted == wanted && (state.service.is_some() || state.problem.is_some()) {
            return;
        }
        let gone = state.clear();
        match Service::new(wanted.base.as_deref()) {
            Ok(service) => {
                state.service = Some(service);
                state.problem = None;
            }
            // An address that is none: nobody is asked, rather than
            // somebody the user did not name.
            Err(err) => {
                state.service = None;
                state.problem = Some(err.to_string());
            }
        }
        let providers = wanted.providers;
        state.wanted = wanted;
        (gone, providers)
    };
    let (gone, providers) = gone;
    if providers {
        start_lookups(cx);
    }
    if !gone.is_empty() {
        for picture in gone {
            cx.drop_image(picture, None);
        }
        cx.refresh_windows();
    }
}

/// Keep answers under this directory between runs (the system's cache
/// directory for the application). Without one they are kept in memory.
pub fn keep_in(dir: Option<PathBuf>, cx: &mut App) {
    if let Some(pictures) = cx.try_global::<Pictures>()
        && let Ok(mut state) = pictures.state.try_borrow_mut()
    {
        state.cache_dir = dir;
        // Threads that run have the old place.
        state.pool = None;
    }
}

/// Install the global and follow the preferences. Nothing starts here:
/// with pictures off nothing ever does.
pub fn init(cx: &mut App) {
    cx.set_global(Pictures::default());
    sync(cx);
    cx.observe_global::<Prefs>(sync).detach();
}

#[cfg(test)]
mod tests {
    use super::{Found, LARGE, PICTURES, SMALL, Slot, State, frame};
    use bitbraid_integrations::avatars::Bitmap;

    fn bitmap(side: u32) -> Bitmap {
        let mut rgba = Vec::new();
        for _ in 0..side * side {
            rgba.extend_from_slice(&[10, 20, 30, 255]);
        }
        Bitmap { side, rgba }
    }

    fn picture() -> Found {
        Found::Picture {
            small: frame(bitmap(SMALL)).unwrap(),
            large: frame(bitmap(LARGE)).unwrap(),
        }
    }

    #[test]
    fn pixels_are_handed_to_the_toolkit_blue_first() {
        let image = frame(bitmap(2)).unwrap();
        assert_eq!(image.as_bytes(0).unwrap(), [30, 20, 10, 255].repeat(4));
        // Pixels that do not make the square they claim are no picture.
        let mut short = bitmap(4);
        short.rgba.truncate(10);
        assert!(frame(short).is_none());
    }

    #[test]
    fn what_was_found_is_taken_only_while_it_is_waited_for() {
        let mut state = State::default();
        state.known.insert("a@example.org".to_owned(), Slot::Asked);
        state.known.insert("b@example.org".to_owned(), Slot::Asked);
        state.known.insert("c@example.org".to_owned(), Slot::Asked);
        let gone = state.take(vec![
            ("a@example.org".to_owned(), picture()),
            ("b@example.org".to_owned(), Found::Absent),
            ("c@example.org".to_owned(), Found::Later),
            // Nobody waits for this one any more.
            ("d@example.org".to_owned(), picture()),
        ]);
        assert!(gone.is_empty());
        assert!(matches!(
            state.known.get("a@example.org"),
            Some(Slot::Picture { .. })
        ));
        assert!(matches!(
            state.known.get("b@example.org"),
            Some(Slot::None { .. })
        ));
        assert!(matches!(
            state.known.get("c@example.org"),
            Some(Slot::None { .. })
        ));
        assert!(!state.known.contains_key("d@example.org"));
        assert_eq!(state.pictures, 1);
        // "Not now" is tried again sooner than "none" is looked up again.
        let (Some(Slot::None { until: none }), Some(Slot::None { until: later })) = (
            state.known.get("b@example.org"),
            state.known.get("c@example.org"),
        ) else {
            panic!("no slots");
        };
        assert!(later < none);
        // A picture is something to draw the windows for; an answer
        // without one is not.
        assert!(std::mem::take(&mut state.came));
        state.known.insert("e@example.org".to_owned(), Slot::Asked);
        state.take(vec![("e@example.org".to_owned(), Found::Absent)]);
        assert!(!state.came);
    }

    #[test]
    fn the_pictures_in_memory_have_a_bound() {
        let mut state = State::default();
        let mut most = 0;
        for number in 0..PICTURES * 3 {
            let address = format!("p{number}@example.org");
            state.known.insert(address.clone(), Slot::Asked);
            // Each drawn later than the one before.
            state.clock = number as u64;
            let gone = state.take(vec![(address, picture())]);
            // Two sizes of each that went.
            assert_eq!(gone.len() % 2, 0);
            most = most.max(state.pictures);
            let held = state
                .known
                .values()
                .filter(|slot| matches!(slot, Slot::Picture { .. }))
                .count();
            assert_eq!(held, state.pictures);
        }
        // Never more than the bound once what was found is taken in.
        assert_eq!(most, PICTURES);
        // The ones that went are the ones drawn longest ago.
        assert!(!state.known.contains_key("p0@example.org"));
        assert!(
            state
                .known
                .contains_key(&format!("p{}@example.org", PICTURES * 3 - 1))
        );
    }

    #[test]
    fn letting_go_hands_back_every_picture() {
        let mut state = State::default();
        for number in 0..5 {
            let address = format!("p{number}@example.org");
            state.known.insert(address.clone(), Slot::Asked);
            state.take(vec![(address, picture())]);
        }
        state
            .known
            .insert("waiting@example.org".to_owned(), Slot::Asked);
        let gone = state.clear();
        assert_eq!(gone.len(), 10);
        assert!(state.known.is_empty());
        assert_eq!(state.pictures, 0);
    }
}
