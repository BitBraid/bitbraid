//! The pictures of authors: that nobody is asked while they are off, who is
//! asked what when they are on, that switching them off stops everything at
//! once, and how the window looks with them.
//!
//! The service is a stand-in on this machine: a few lines that listen on a
//! loopback port the system picks, answer with a picture for the hashes
//! they were told and "not found" for the rest, and note every request.
//! The real service is never contacted.

use std::{
    collections::HashMap,
    io::{Read as _, Write as _},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

use bitbraid_integrations::avatars as source;
use bitbraid_testkit::{Git, fixtures_out_dir, oracle::clock_at};
use bitbraid_ui::{avatars, prefs::Prefs};

use crate::{
    repos,
    runner::{Shots, Themes},
    stage::{Stage, Start},
};

const ADA: &str = "ada@example.org";
const GRACE: &str = "grace@example.org";
const BOB: &str = "bob@example.org";

/// Who wrote the commits of the repository these tests open: two with a
/// picture, one without, one whose address is written in capitals, and two
/// whose addresses nobody could have a picture for.
const PEOPLE: &[(&str, &str, &str)] = &[
    ("Ada Lovelace", ADA, "Start the notes on the engine"),
    ("Grace Hopper", GRACE, "Add a compiler for the notes"),
    ("Bob Example", BOB, "Reformat with tabs"),
    (
        "dependabot[bot]",
        "49699333+dependabot[bot]@users.noreply.github.com",
        "Bump parser from 1.2 to 1.3",
    ),
    ("Ada Lovelace", " ADA@Example.org", "Number the notes"),
    (
        "Build Machine",
        "builder@forge.local",
        "Record the nightly build",
    ),
    ("Grace Hopper", GRACE, "Find the first bug"),
    ("Ada Lovelace", ADA, "Translate the last note"),
];

/// A repository whose commits are by [`PEOPLE`], an hour apart.
fn people() -> PathBuf {
    let path = repos::work().join("people");
    std::fs::create_dir_all(&path).unwrap();
    let git = Git::new(&path, fixtures_out_dir().join(".home"));
    git.ensure_home().unwrap();
    git.run(["init", "-q", "-b", "main"]).unwrap();
    let mut notes = String::new();
    for (tick, (name, address, message)) in PEOPLE.iter().enumerate() {
        notes.push_str(message);
        notes.push('\n');
        std::fs::write(path.join("notes.txt"), &notes).unwrap();
        git.run(["add", "notes.txt"]).unwrap();
        let date = clock_at(tick as u64);
        git.run_with(
            ["commit", "-q", "-m", message],
            &[
                ("GIT_AUTHOR_NAME", name),
                ("GIT_AUTHOR_EMAIL", address),
                ("GIT_COMMITTER_NAME", name),
                ("GIT_COMMITTER_EMAIL", address),
                ("GIT_AUTHOR_DATE", &date),
                ("GIT_COMMITTER_DATE", &date),
            ],
        )
        .unwrap();
    }
    path
}

/// A head and shoulders on a ground of one colour: plainly a picture and
/// not a pattern.
fn portrait(colour: [u8; 3]) -> Vec<u8> {
    let side = 64u32;
    let picture = image::RgbaImage::from_fn(side, side, |x, y| {
        let (x, y) = (x as f32 - 32.0, y as f32);
        let head = x * x + (y - 26.0) * (y - 26.0) <= 13.0 * 13.0;
        let body = (x / 23.0).powi(2) + ((y - 66.0) / 27.0).powi(2) <= 1.0;
        let [r, g, b] = colour;
        if head || body {
            image::Rgba([r / 3, g / 3, b / 3, 255])
        } else {
            image::Rgba([r, g, b, 255])
        }
    });
    let mut bytes = Vec::new();
    image::DynamicImage::ImageRgba8(picture)
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

/// The stand-in for the picture service.
struct Service {
    port: u16,
    /// The path of every request, in the order they came.
    heard: Arc<Mutex<Vec<String>>>,
    /// How long it waits before it answers, in milliseconds.
    delay: Arc<AtomicU64>,
}

impl Service {
    /// Start one that has pictures for Ada and Grace.
    fn start() -> Self {
        let pictures: HashMap<String, Vec<u8>> = [(ADA, [214, 92, 66]), (GRACE, [66, 133, 214])]
            .into_iter()
            .map(|(address, colour)| (source::hash(address), portrait(colour)))
            .collect();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let heard: Arc<Mutex<Vec<String>>> = Arc::default();
        let delay = Arc::new(AtomicU64::new(0));
        let (noted, wait) = (heard.clone(), delay.clone());
        let pictures = Arc::new(pictures);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (noted, wait, pictures) = (noted.clone(), wait.clone(), pictures.clone());
                thread::spawn(move || {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") && head.len() < 8192 {
                        match stream.read(&mut byte) {
                            Ok(1) => head.push(byte[0]),
                            _ => return,
                        }
                    }
                    let head = String::from_utf8_lossy(&head).into_owned();
                    let path = head.split(' ').nth(1).unwrap_or_default().to_owned();
                    noted.lock().unwrap().push(path.clone());
                    thread::sleep(Duration::from_millis(wait.load(Ordering::Relaxed)));
                    let hash = path
                        .split('?')
                        .next()
                        .and_then(|path| path.rsplit('/').next())
                        .unwrap_or_default();
                    let answer = match pictures.get(hash) {
                        Some(picture) => {
                            let mut answer = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\n\
                                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                                picture.len()
                            )
                            .into_bytes();
                            answer.extend_from_slice(picture);
                            answer
                        }
                        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\
                                  Connection: close\r\n\r\n"
                            .to_vec(),
                    };
                    let _ = stream.write_all(&answer);
                });
            }
        });
        Self { port, heard, delay }
    }

    fn heard(&self) -> Vec<String> {
        self.heard.lock().unwrap().clone()
    }

    /// `preferences.toml` that names this service, with pictures on or
    /// off, and the author's column in the graph.
    fn preferences(&self, on: bool) -> String {
        format!(
            "gravatar = {on}\ngravatar_base = \"http://127.0.0.1:{}/avatar\"\n\
             [columns]\norder = [\"summary\", \"author\", \"date\"]\n",
            self.port
        )
    }

    /// What is asked for the picture of an address.
    fn asked_for(address: &str) -> String {
        format!("/avatar/{}?s=64&d=404", source::hash(address))
    }
}

/// Switch the pictures on or off, as the switch in the preferences does.
/// In the window: what follows a change of the preferences has run when
/// this returns, as it has after a click.
fn switch(stage: &mut Stage, on: bool) {
    stage.window(|_, cx| {
        let prefs = cx.global_mut::<Prefs>();
        prefs.preferences.gravatar = on;
        prefs.save_preferences();
        cx.refresh_windows();
    });
}

fn held(stage: &mut Stage, address: &str) -> bool {
    stage.app(|cx| avatars::held(address, cx))
}

/// Off, which is how it starts, not one request leaves; on, each author
/// whose row is on show is asked about once, by the hash of the address
/// and nothing else, and the ones nobody could have a picture for are not
/// asked about at all; off again, the pictures are gone at once and
/// nothing more is asked.
pub fn pictures_are_asked_for_only_while_they_are_switched_on() {
    repos::fresh();
    let repo = people();
    let service = Service::start();
    let before = avatars::asked();
    let mut stage = Stage::open(Start::with(&repo).preferences(&service.preferences(false)));
    assert_eq!(stage.graph().commits, PEOPLE.len());
    for _ in 0..3 {
        stage.frame();
    }
    assert_eq!(stage.app(|cx| avatars::asking(cx)), None);
    assert!(!held(&mut stage, ADA));
    // A row that is drawn asks for its author's picture; off, it gets
    // none and nothing is queued for anybody.
    assert!(stage.app(|cx| avatars::picture(ADA, false, cx).is_none()));
    stage.frame();
    assert_eq!(avatars::asked(), before, "something was queued while off");
    assert_eq!(service.heard(), Vec::<String>::new());

    switch(&mut stage, true);
    assert_eq!(
        stage.app(|cx| avatars::asking(cx)),
        Some(format!("127.0.0.1:{}", service.port))
    );
    stage.until("the pictures of the two who have one", |stage| {
        held(stage, ADA) && held(stage, GRACE)
    });
    stage.until("an answer about each of the three", |_| {
        service.heard().len() >= 3
    });
    // However often the rows are drawn, each was asked about once: the
    // answer "none" is kept as well.
    for _ in 0..6 {
        stage.frame();
    }
    thread::sleep(Duration::from_millis(150));
    stage.frame();
    let mut heard = service.heard();
    heard.sort();
    let mut expected = vec![
        Service::asked_for(ADA),
        Service::asked_for(GRACE),
        Service::asked_for(BOB),
    ];
    expected.sort();
    assert_eq!(
        heard, expected,
        "one request each for three authors, and none for the bot, the build machine or \
         the same address written another way"
    );
    assert_eq!(avatars::asked(), before + 3);
    assert!(!held(&mut stage, BOB), "nobody has a picture of Bob");
    // The address in capitals is the same person's.
    assert!(stage.app(|cx| avatars::picture(" ADA@Example.org", true, cx).is_some()));

    switch(&mut stage, false);
    // Gone at once, not at the next frame.
    assert!(!held(&mut stage, ADA) && !held(&mut stage, GRACE));
    assert_eq!(stage.app(|cx| avatars::asking(cx)), None);
    for _ in 0..6 {
        stage.frame();
    }
    stage.keys("down down up");
    thread::sleep(Duration::from_millis(150));
    stage.frame();
    assert_eq!(service.heard().len(), 3, "asked while off");
    assert_eq!(avatars::asked(), before + 3);

    // On again: there they are once more.
    switch(&mut stage, true);
    stage.until("the pictures again", |stage| {
        held(stage, ADA) && held(stage, GRACE)
    });
}

/// Switched off while the service is still thinking: the answers that
/// were under way are not waited for, not shown, and nothing follows them.
pub fn switching_pictures_off_drops_what_is_under_way() {
    repos::fresh();
    let repo = people();
    let service = Service::start();
    service.delay.store(600, Ordering::Relaxed);
    let mut stage = Stage::open(Start::with(&repo).preferences(&service.preferences(true)));
    stage.until("the first request", |_| !service.heard().is_empty());
    let asked = avatars::asked();
    switch(&mut stage, false);
    let heard = service.heard().len();
    // Longer than the service thinks: its answers have come by now, to
    // nobody.
    thread::sleep(Duration::from_millis(900));
    for _ in 0..4 {
        stage.frame();
    }
    assert!(!held(&mut stage, ADA) && !held(&mut stage, GRACE));
    assert_eq!(avatars::asked(), asked);
    assert_eq!(
        service.heard().len(),
        heard,
        "a request was made after pictures were switched off"
    );
    // The window is none the worse for it.
    assert_eq!(stage.graph().commits, PEOPLE.len());
    assert_eq!(stage.notices(), Vec::<String>::new());
}

/// The graph, the author's column and the details with pictures where
/// there are some and patterns where there are none.
pub fn pictures(shots: &mut Shots) {
    repos::fresh();
    let repo = people();
    let service = Service::start();
    let mut stage = Stage::open(
        Start::with(&repo)
            .preferences(&service.preferences(true))
            .pictures(),
    );
    assert_eq!(stage.graph().commits, PEOPLE.len());
    stage.until("the pictures of the two who have one", |stage| {
        held(stage, ADA) && held(stage, GRACE)
    });
    stage.until("an answer about each of the three", |_| {
        service.heard().len() >= 3
    });
    assert!(!held(&mut stage, BOB));
    shots.take(&mut stage, "graph_pictures", Themes::Two);
}
