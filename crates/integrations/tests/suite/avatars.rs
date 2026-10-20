//! The source of pictures: what it asks, what it keeps, when it asks
//! again, and what it makes of a service that misbehaves.
//!
//! The first half hands it a stand-in for the network that answers from a
//! script and notes every request, so that "nothing was asked" can be
//! said exactly. The second half is the real client against a server on
//! this machine. The real service is never contacted.

use std::{
    io::Write as _,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

use crate::common::{Server, answer, path_of, png, serve, write_forever};
use bitbraid_integrations::{
    avatars::{self, Avatars, Bounds, Outcome, Service},
    http::{Call, Client, Fetch, FetchError, Method, Response},
};

const DAY: Duration = Duration::from_secs(24 * 3600);
const SIDES: &[u32] = &[36, 64];

type Script = Box<dyn Fn(&str) -> Result<Response, FetchError> + Send + Sync>;

/// The network, from a script: what each address is answered with.
struct Scripted {
    script: Script,
    asked: Mutex<Vec<String>>,
    accepts: Mutex<Vec<String>>,
    bodies: Mutex<Vec<usize>>,
}

impl std::fmt::Debug for Scripted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Scripted")
    }
}

impl Scripted {
    fn new(
        script: impl Fn(&str) -> Result<Response, FetchError> + Send + Sync + 'static,
    ) -> Arc<Self> {
        Arc::new(Self {
            script: Box::new(script),
            asked: Mutex::default(),
            accepts: Mutex::default(),
            bodies: Mutex::default(),
        })
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Fetch for Scripted {
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError> {
        if cancel.load(Ordering::Relaxed) {
            return Err(FetchError::Cancelled);
        }
        assert_eq!(call.method, Method::Get);
        assert!(call.body.is_none());
        let address = call.url.to_string();
        self.asked.lock().unwrap().push(address.clone());
        self.accepts
            .lock()
            .unwrap()
            .push(call.header("accept").unwrap_or_default().to_owned());
        self.bodies.lock().unwrap().push(call.limits.body);
        (self.script)(&address)
    }
}

fn ok(body: &[u8]) -> Result<Response, FetchError> {
    Ok(Response {
        status: 200,
        location: None,
        content_type: Some("image/png".to_owned()),
        body: body.to_vec(),
        ..Default::default()
    })
}

fn status(status: u16) -> Result<Response, FetchError> {
    Ok(Response {
        status,
        location: None,
        content_type: None,
        body: Vec::new(),
        ..Default::default()
    })
}

fn redirect(to: &str) -> Result<Response, FetchError> {
    Ok(Response {
        status: 302,
        location: Some(to.to_owned()),
        content_type: None,
        body: Vec::new(),
        ..Default::default()
    })
}

fn scratch(name: &str) -> PathBuf {
    let dir = bitbraid_testkit::scratch_root()
        .join("avatars")
        .join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn source(fetch: Arc<Scripted>, cache: Option<PathBuf>) -> Avatars {
    Avatars::new(fetch, Service::new(None).unwrap(), cache)
}

fn go() -> AtomicBool {
    AtomicBool::new(false)
}

fn hash_of(address: &str) -> String {
    avatars::hash(&avatars::normalised(address))
}

#[test]
fn a_picture_is_asked_for_once_and_then_kept() {
    let picture = png(64, 200, 80, 40);
    let fetch = Scripted::new(move |_| ok(&picture));
    let dir = scratch("once");
    let avatars = source(fetch.clone(), Some(dir.clone()));

    let Outcome::Picture(bitmaps) = avatars.picture(" Ada@Example.ORG ", SIDES, &go()) else {
        panic!("no picture");
    };
    assert_eq!(bitmaps.iter().map(|b| b.side).collect::<Vec<_>>(), [36, 64]);
    assert_eq!(bitmaps[1].rgba.len(), 64 * 64 * 4);

    // Exactly what was asked: the service's address, the hash of the
    // address as the service documents it, the size and "404 if none".
    let hash = hash_of("ada@example.org");
    assert_eq!(
        fetch.asked(),
        [format!("https://gravatar.com/avatar/{hash}?s=64&d=404")]
    );
    assert_eq!(
        *fetch.accepts.lock().unwrap(),
        ["image/png, image/jpeg, image/gif, image/webp"]
    );
    assert_eq!(*fetch.bodies.lock().unwrap(), [512 * 1024]);
    // Nothing of the address itself is in it.
    assert!(!fetch.asked()[0].contains("ada"));
    assert!(!fetch.asked()[0].contains("example"));

    // Again, written another way, and by a source that starts anew: the
    // answer is on disk, and nobody is asked.
    assert!(matches!(
        avatars.picture("ada@example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    let again = source(fetch.clone(), Some(dir));
    assert!(matches!(
        again.picture("ADA@example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    assert_eq!(fetch.asked().len(), 1);
}

#[test]
fn no_picture_is_remembered_for_a_good_while() {
    let fetch = Scripted::new(|_| status(404));
    let dir = scratch("absent");
    let avatars = source(fetch.clone(), Some(dir.clone()));
    let (now, at) = (SystemTime::now(), Instant::now());
    assert_eq!(
        avatars.picture_at("bob@example.org", SIDES, &go(), now, at),
        Outcome::Absent
    );
    for hours in [1, 24, 71] {
        let later = now + Duration::from_secs(3600 * hours);
        assert_eq!(
            avatars.picture_at("bob@example.org", SIDES, &go(), later, at),
            Outcome::Absent
        );
    }
    assert_eq!(fetch.asked().len(), 1, "asked again within three days");
    // A source that starts anew remembers too.
    let again = source(fetch.clone(), Some(dir));
    assert_eq!(
        again.picture_at("bob@example.org", SIDES, &go(), now + DAY, at),
        Outcome::Absent
    );
    assert_eq!(fetch.asked().len(), 1);
    // After three days it is asked once more, and remembered from then.
    assert_eq!(
        again.picture_at("bob@example.org", SIDES, &go(), now + DAY * 4, at),
        Outcome::Absent
    );
    assert_eq!(fetch.asked().len(), 2);
}

#[test]
fn a_picture_that_is_due_is_asked_for_again_and_shown_meanwhile() {
    let serving = Arc::new(Mutex::new(Some(png(64, 10, 200, 10))));
    let script = serving.clone();
    let fetch = Scripted::new(move |_| match script.lock().unwrap().as_ref() {
        Some(picture) => ok(picture),
        None => Err(FetchError::Timeout),
    });
    let avatars = source(fetch.clone(), Some(scratch("due")));
    let (now, at) = (SystemTime::now(), Instant::now());
    let Outcome::Picture(first) = avatars.picture_at("eve@example.org", SIDES, &go(), now, at)
    else {
        panic!("no picture");
    };
    // Within the week: not asked.
    assert!(matches!(
        avatars.picture_at("eve@example.org", SIDES, &go(), now + DAY * 6, at),
        Outcome::Picture(_)
    ));
    assert_eq!(fetch.asked().len(), 1);
    // After it, with the service gone: asked, and the old one still shown.
    *serving.lock().unwrap() = None;
    let Outcome::Picture(old) =
        avatars.picture_at("eve@example.org", SIDES, &go(), now + DAY * 8, at)
    else {
        panic!("the kept picture was dropped");
    };
    assert_eq!(old, first);
    assert_eq!(fetch.asked().len(), 2);
    // With the service back and another picture: that one, from then on.
    *serving.lock().unwrap() = Some(png(64, 10, 10, 220));
    let Outcome::Picture(new) =
        avatars.picture_at("eve@example.org", SIDES, &go(), now + DAY * 8, at)
    else {
        panic!("no picture");
    };
    assert_ne!(new, first);
    assert_eq!(fetch.asked().len(), 3);
    assert!(matches!(
        avatars.picture_at("eve@example.org", SIDES, &go(), now + DAY * 8, at),
        Outcome::Picture(_)
    ));
    assert_eq!(fetch.asked().len(), 3);
}

#[test]
fn a_picture_that_was_taken_down_is_gone_after_its_week() {
    let there = Arc::new(AtomicBool::new(true));
    let script = there.clone();
    let picture = png(64, 90, 90, 200);
    let fetch = Scripted::new(move |_| {
        if script.load(Ordering::Relaxed) {
            ok(&picture)
        } else {
            status(404)
        }
    });
    let avatars = source(fetch.clone(), Some(scratch("down")));
    let (now, at) = (SystemTime::now(), Instant::now());
    assert!(matches!(
        avatars.picture_at("dan@example.org", SIDES, &go(), now, at),
        Outcome::Picture(_)
    ));
    there.store(false, Ordering::Relaxed);
    assert_eq!(
        avatars.picture_at("dan@example.org", SIDES, &go(), now + DAY * 8, at),
        Outcome::Absent
    );
    assert_eq!(
        avatars.picture_at("dan@example.org", SIDES, &go(), now + DAY * 9, at),
        Outcome::Absent
    );
    assert_eq!(fetch.asked().len(), 2);
}

#[test]
fn addresses_nobody_registered_are_never_sent() {
    let fetch = Scripted::new(|_| status(404));
    let dir = scratch("never");
    let avatars = source(fetch.clone(), Some(dir.clone()));
    for address in [
        "",
        "nobody",
        "ada@localhost",
        "ada@build-host.local",
        "ada@fixture.invalid",
        "49699333+dependabot[bot]@users.noreply.github.com",
        "ada@machine.(none)",
    ] {
        assert_eq!(
            avatars.picture(address, SIDES, &go()),
            Outcome::Absent,
            "{address}"
        );
    }
    assert!(fetch.asked().is_empty());
    // Not even a folder was made for them.
    assert!(!dir.exists());
}

#[test]
fn an_answer_that_is_no_picture_counts_as_none() {
    let huge = png(2000, 1, 2, 3);
    assert!(huge.len() < 512 * 1024, "{}", huge.len());
    for (name, body) in [
        (
            "html",
            b"<html><body>Sign in to continue</body></html>".to_vec(),
        ),
        ("zeros", vec![0u8; 3000]),
        ("cut", png(64, 5, 5, 5)[..60].to_vec()),
        // A picture, and far larger than one that was asked for at 64.
        ("huge", huge),
        ("empty", Vec::new()),
    ] {
        let fetch = Scripted::new(move |_| ok(&body));
        let dir = scratch(&format!("garbage-{name}"));
        let avatars = source(fetch.clone(), Some(dir.clone()));
        assert_eq!(
            avatars.picture("ada@example.org", SIDES, &go()),
            Outcome::Absent,
            "{name}"
        );
        // Remembered as "none": the same nonsense is not fetched again.
        assert_eq!(
            avatars.picture("ada@example.org", SIDES, &go()),
            Outcome::Absent,
            "{name}"
        );
        assert_eq!(fetch.asked().len(), 1, "{name}");
        // And none of it is kept as a picture.
        let kept: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            kept.iter().all(|file| !file.ends_with(".img")),
            "{name}: {kept:?}"
        );
    }
}

#[test]
fn a_service_that_does_not_answer_is_left_alone_for_a_while() {
    let fetch = Scripted::new(|_| Err(FetchError::Timeout));
    let avatars = source(fetch.clone(), Some(scratch("down-service")));
    let (now, at) = (SystemTime::now(), Instant::now());
    for number in 0..3 {
        assert_eq!(
            avatars.picture_at(&format!("p{number}@example.org"), SIDES, &go(), now, at),
            Outcome::Later
        );
    }
    assert_eq!(fetch.asked().len(), 3);
    // Three in a row: for the next minute nobody is asked, whoever it is
    // about. A hundred authors scrolled past cost nothing.
    for number in 3..100 {
        assert_eq!(
            avatars.picture_at(&format!("p{number}@example.org"), SIDES, &go(), now, at),
            Outcome::Later
        );
    }
    assert_eq!(fetch.asked().len(), 3);
    // After the minute one more is tried, and with that failing too the
    // wait is twice as long.
    let after = at + Duration::from_secs(61);
    assert_eq!(
        avatars.picture_at("q@example.org", SIDES, &go(), now, after),
        Outcome::Later
    );
    assert_eq!(fetch.asked().len(), 4);
    assert_eq!(
        avatars.picture_at(
            "r@example.org",
            SIDES,
            &go(),
            now,
            after + Duration::from_secs(100)
        ),
        Outcome::Later
    );
    assert_eq!(fetch.asked().len(), 4);
    assert_eq!(
        avatars.picture_at(
            "r@example.org",
            SIDES,
            &go(),
            now,
            after + Duration::from_secs(121)
        ),
        Outcome::Later
    );
    assert_eq!(fetch.asked().len(), 5);
    // Nothing of a failure is written down: it is not "no picture".
    assert_eq!(avatars.cache().unwrap().size(), (0, 0));
}

#[test]
fn every_kind_of_refusal_is_a_failure_and_none_is_kept() {
    for (name, result) in [
        ("500", status(500)),
        ("503", status(503)),
        ("429", status(429)),
        ("403", status(403)),
        ("204", status(204)),
        ("too large", Err(FetchError::TooLarge)),
        ("malformed", Err(FetchError::Malformed("the head"))),
        ("tls", Err(FetchError::Tls("unknown issuer".to_owned()))),
        ("connect", Err(FetchError::Connect("refused".to_owned()))),
        (
            "resolve",
            Err(FetchError::Resolve("gravatar.com".to_owned())),
        ),
        ("broken", Err(FetchError::Broken("reset".to_owned()))),
    ] {
        let script = result.clone();
        let fetch = Scripted::new(move |_| script.clone());
        let avatars = source(
            fetch.clone(),
            Some(scratch(&format!("refusal-{}", name.replace(' ', "-")))),
        );
        assert_eq!(
            avatars.picture("ada@example.org", SIDES, &go()),
            Outcome::Later,
            "{name}"
        );
        // Not remembered as an answer: asked again the next time.
        assert_eq!(
            avatars.picture("ada@example.org", SIDES, &go()),
            Outcome::Later,
            "{name}"
        );
        assert_eq!(fetch.asked().len(), 2, "{name}");
        assert_eq!(avatars.cache().unwrap().size(), (0, 0), "{name}");
    }
}

#[test]
fn a_redirect_is_followed_within_the_service_and_no_further() {
    let picture = png(64, 150, 150, 20);
    // To the same service under another name: followed, to the picture.
    let fetch = Scripted::new(move |address| {
        if address.starts_with("https://gravatar.com/") {
            redirect(&address.replace("https://gravatar.com/", "https://secure.gravatar.com/"))
        } else {
            ok(&picture)
        }
    });
    let avatars = source(fetch.clone(), None);
    assert!(matches!(
        avatars.picture("ada@example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    let asked = fetch.asked();
    assert_eq!(asked.len(), 2);
    assert!(asked[1].starts_with("https://secure.gravatar.com/avatar/"));

    // Away from it, or down to plain HTTP: not followed, and nothing is
    // asked of where it pointed.
    for target in [
        "https://example.org/collect",
        "http://gravatar.com/avatar/x",
        "https://gravatar.com.example.org/avatar/x",
        "ftp://gravatar.com/x",
        "https://exa mple.org/",
    ] {
        let target = target.to_owned();
        let fetch = Scripted::new(move |_| redirect(&target));
        let avatars = source(fetch.clone(), None);
        assert_eq!(
            avatars.picture("ada@example.org", SIDES, &go()),
            Outcome::Later
        );
        assert_eq!(fetch.asked().len(), 1);
        assert!(fetch.asked()[0].starts_with("https://gravatar.com/avatar/"));
    }
    // A redirect that names no place.
    let fetch = Scripted::new(|_| status(302));
    let avatars = source(fetch.clone(), None);
    assert_eq!(
        avatars.picture("ada@example.org", SIDES, &go()),
        Outcome::Later
    );
    assert_eq!(fetch.asked().len(), 1);
}

#[test]
fn a_circle_of_redirects_ends_after_a_few() {
    let fetch = Scripted::new(|address| {
        if address.contains("/a/") {
            redirect("/avatar/b/back")
        } else {
            redirect("/a/forth")
        }
    });
    let avatars = source(fetch.clone(), Some(scratch("circle")));
    assert_eq!(
        avatars.picture("ada@example.org", SIDES, &go()),
        Outcome::Later
    );
    // The first and three more, and that is all.
    assert_eq!(fetch.asked().len(), 4);
    // It was one failure, not four: two more are needed for the pause.
    let (now, at) = (SystemTime::now(), Instant::now());
    assert_eq!(
        avatars.picture_at("b@example.org", SIDES, &go(), now, at),
        Outcome::Later
    );
    assert_eq!(fetch.asked().len(), 8);
    assert_eq!(
        avatars.picture_at("c@example.org", SIDES, &go(), now, at),
        Outcome::Later
    );
    assert_eq!(fetch.asked().len(), 12);
    assert_eq!(
        avatars.picture_at("d@example.org", SIDES, &go(), now, at),
        Outcome::Later
    );
    assert_eq!(fetch.asked().len(), 12);
}

#[test]
fn a_request_that_was_stopped_is_neither_an_answer_nor_a_failure() {
    let picture = png(64, 1, 99, 99);
    let fetch = Scripted::new(move |_| ok(&picture));
    let avatars = source(fetch.clone(), Some(scratch("stopped")));
    let stop = AtomicBool::new(true);
    for number in 0..10 {
        assert_eq!(
            avatars.picture(&format!("p{number}@example.org"), SIDES, &stop),
            Outcome::Later
        );
    }
    assert!(fetch.asked().is_empty());
    assert_eq!(avatars.cache().unwrap().size(), (0, 0));
    // Ten stops are no reason to leave the service alone.
    assert!(matches!(
        avatars.picture("p0@example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    assert_eq!(fetch.asked().len(), 1);
}

#[test]
fn what_is_kept_stays_within_its_bounds() {
    let picture = png(64, 30, 60, 90);
    let size = picture.len() as u64;
    let fetch = Scripted::new(move |address| {
        // Every third address has a picture.
        let last = address.split('?').next().unwrap().bytes().last().unwrap();
        if last % 3 == 0 {
            ok(&picture)
        } else {
            status(404)
        }
    });
    let dir = scratch("bounded");
    let bounds = Bounds {
        entries: 40,
        bytes: size * 6,
        ..Bounds::default()
    };
    let avatars = Avatars::with_bounds(
        fetch.clone(),
        Service::new(None).unwrap(),
        Some(dir.clone()),
        bounds,
    );
    for number in 0..600 {
        avatars.picture(&format!("person{number}@example.org"), &[16], &go());
        let (entries, bytes) = avatars.cache().unwrap().size();
        assert!(
            entries <= 40 && bytes <= size * 6,
            "{entries} entries, {bytes} bytes"
        );
    }
    assert_eq!(fetch.asked().len(), 600);
    let files = std::fs::read_dir(&dir).unwrap().count();
    assert!(files <= 41, "{files} files");
    let on_disk: u64 = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    assert!(on_disk <= size * 6 + 64, "{on_disk} bytes");
}

#[test]
fn without_a_folder_it_still_answers() {
    let picture = png(64, 77, 77, 77);
    let fetch = Scripted::new(move |_| ok(&picture));
    let avatars = source(fetch.clone(), None);
    assert!(avatars.cache().is_none());
    assert!(matches!(
        avatars.picture("ada@example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    assert!(matches!(
        avatars.picture("ada@example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    // Nothing is kept, so it is asked each time; whoever holds the
    // pictures in memory asks once.
    assert_eq!(fetch.asked().len(), 2);
}

#[test]
fn a_kept_picture_that_was_damaged_is_asked_for_anew() {
    let picture = png(64, 9, 99, 199);
    let fetch = Scripted::new(move |_| ok(&picture));
    let dir = scratch("damaged");
    let avatars = source(fetch.clone(), Some(dir.clone()));
    assert!(matches!(
        avatars.picture("ada@example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    let file = dir.join(format!("{}.img", hash_of("ada@example.org")));
    std::fs::write(&file, b"\x89PNG\r\n\x1a\nand then nothing of use").unwrap();
    assert!(matches!(
        avatars.picture("ada@example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    assert_eq!(fetch.asked().len(), 2);
    // What is on disk is the picture again.
    assert!(std::fs::read(&file).unwrap().len() > 100);
}

// ---- the real client, against a server on this machine ----

fn local(server: &Server, cache: Option<PathBuf>) -> Avatars {
    let service = Service::new(Some(&format!("{}/avatar", server.base()))).unwrap();
    Avatars::new(Arc::new(Client::new("BitBraid-Test/1.0")), service, cache)
        .patient(Duration::from_secs(2), Duration::from_millis(700))
}

#[test]
fn over_the_wire_a_picture_arrives_and_only_its_hash_is_sent() {
    let ada = hash_of("ada@example.org");
    let known = ada.clone();
    let server = serve(move |stream, head| {
        let reply = if path_of(head).contains(&known) {
            answer("200 OK", "image/png", &png(64, 220, 120, 20))
        } else {
            answer("404 Not Found", "text/plain", b"no such picture")
        };
        let _ = stream.write_all(&reply);
    });
    let dir = scratch("wire");
    let avatars = local(&server, Some(dir.clone()));
    assert!(matches!(
        avatars.picture("Ada@Example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    assert_eq!(
        avatars.picture("bob@example.org", SIDES, &go()),
        Outcome::Absent
    );
    assert!(matches!(
        avatars.picture("ada@example.org", SIDES, &go()),
        Outcome::Picture(_)
    ));
    assert_eq!(
        avatars.picture("bob@example.org", SIDES, &go()),
        Outcome::Absent
    );

    let requests = server.requests();
    assert_eq!(requests.len(), 2, "{requests:?}");
    assert_eq!(
        requests[0],
        format!(
            "GET /avatar/{ada}?s=64&d=404 HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: BitBraid-Test/1.0\r\n\
             Accept: image/png, image/jpeg, image/gif, image/webp\r\n\
             Accept-Encoding: identity\r\n\
             Connection: close\r\n\r\n",
            server.address
        )
    );
    // In all that was sent there is no address, no name, no path.
    for request in &requests {
        for word in ["ada", "bob", "example.org", "Ada", "@"] {
            assert!(!request.contains(word), "{word:?} in {request:?}");
        }
    }
    // On disk: the picture as it came, a mark for "none", and the label.
    let mut kept: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    kept.sort();
    let mut expected = vec![
        format!("{ada}.img"),
        format!("{}.none", hash_of("bob@example.org")),
        "service".to_owned(),
    ];
    expected.sort();
    assert_eq!(kept, expected);
}

#[test]
fn over_the_wire_hostile_servers_cost_a_moment_and_keep_nothing() {
    let piece = vec![b'x'; 16 * 1024];
    let huge = png(2000, 9, 9, 9);
    type Answer = Box<dyn Fn(&mut std::net::TcpStream) + Send + Sync>;
    let servers: Vec<(&str, Outcome, Answer)> = vec![
        // It never answers.
        (
            "silent",
            Outcome::Later,
            Box::new(|_| thread::sleep(Duration::from_secs(10))),
        ),
        // It answers a byte at a time, for ever.
        (
            "slow",
            Outcome::Later,
            Box::new(|stream| {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5000\r\n\r\n");
                write_forever(stream, b"\x89", Duration::from_millis(30), 2000);
            }),
        ),
        // A body that never ends.
        (
            "endless",
            Outcome::Later,
            Box::new(move |stream| {
                let _ = stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: image/png\r\n\r\n");
                write_forever(stream, &piece, Duration::ZERO, 1_000_000);
            }),
        ),
        // Nonsense in place of an answer.
        (
            "garbage",
            Outcome::Later,
            Box::new(|stream| {
                let _ =
                    stream.write_all(b"\x16\x03\x01\x02\x00\x01\x00\x01\xfc\x03\x03\n\n\r\n\r\n");
            }),
        ),
        // A proper answer whose body is nonsense: that is "none".
        (
            "not-a-picture",
            Outcome::Absent,
            Box::new(|stream| {
                let _ = stream.write_all(&answer("200 OK", "image/png", &[0x42; 5000]));
            }),
        ),
        // A real picture of four million pixels where 64 by 64 was asked.
        (
            "huge",
            Outcome::Absent,
            Box::new(move |stream| {
                let _ = stream.write_all(&answer("200 OK", "image/png", &huge));
            }),
        ),
        // Round and round.
        (
            "circle",
            Outcome::Later,
            Box::new(|stream| {
                let _ = stream.write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: /avatar/again\r\nContent-Length: 0\r\n\r\n",
                );
            }),
        ),
        // Off to somebody else.
        (
            "away",
            Outcome::Later,
            Box::new(|stream| {
                let _ = stream.write_all(
                    b"HTTP/1.1 301 Moved\r\nLocation: http://192.0.2.7/collect\r\nContent-Length: 0\r\n\r\n",
                );
            }),
        ),
    ];
    for (name, expected, respond) in servers {
        let server = serve(move |stream, _| respond(stream));
        let dir = scratch(&format!("hostile-{name}"));
        let avatars = local(&server, Some(dir.clone()));
        let started = Instant::now();
        let outcome = avatars.picture("ada@example.org", SIDES, &go());
        let took = started.elapsed();
        assert_eq!(outcome, expected, "{name}");
        // The patience given was 0.7 s for the whole of it.
        assert!(took < Duration::from_secs(4), "{name} took {took:?}");
        let pictures = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter(|entry| {
                        entry.as_ref().is_ok_and(|entry| {
                            entry.file_name().to_string_lossy().ends_with(".img")
                        })
                    })
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(pictures, 0, "{name}");
        match name {
            "circle" => assert_eq!(server.requests().len(), 4),
            _ => assert_eq!(server.requests().len(), 1, "{name}"),
        }
    }
}

#[test]
fn over_the_wire_a_stop_ends_the_wait() {
    let server = serve(|_, _| thread::sleep(Duration::from_secs(10)));
    let service = Service::new(Some(&format!("{}/avatar", server.base()))).unwrap();
    let avatars = Avatars::new(Arc::new(Client::new("BitBraid-Test/1.0")), service, None)
        .patient(Duration::from_secs(5), Duration::from_secs(8));
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(250));
        stop.store(true, Ordering::Relaxed);
    });
    let started = Instant::now();
    assert_eq!(
        avatars.picture("ada@example.org", SIDES, &cancel),
        Outcome::Later
    );
    assert!(
        started.elapsed() < Duration::from_millis(1500),
        "{:?}",
        started.elapsed()
    );
}
