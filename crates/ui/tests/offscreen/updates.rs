//! What the window does about updates, and its picture.
//!
//! The updater of the window under test is a stand-in installed for its
//! configuration directory: the feed and the package come from memory,
//! signed with a key of the tests' own, and every address asked is
//! written down, so that what is asked, and when, is held here. Nothing
//! is asked while updates are off: not by the preferences' row, not by
//! the palette's command; turned on, a newer version is looked for at
//! once and said, and installed from the preferences with a restart
//! offered.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use bitbraid_integrations::{
    http::{Call, Fetch, FetchError, Response},
    update::{Arch, Installation, System, Target, UpdateKey, Updater},
};
use bitbraid_testkit::{Signer, signer::sha256_hex};
use bitbraid_ui::keymap;
use serde_json::json;

use crate::{
    runner::{Shots, Test, Themes},
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "nothing_is_asked_about_updates_until_they_are_turned_on",
        run: nothing_is_asked_about_updates_until_they_are_turned_on,
    },
    Test {
        name: "a_newer_version_is_installed_from_the_preferences_with_a_restart_offered",
        run: a_newer_version_is_installed_from_the_preferences_with_a_restart_offered,
    },
];

const FEED: &str = "https://releases.example.test/latest.json";
const PACKAGE: &str = "https://releases.example.test/BitBraid-1.2.0-x86_64.AppImage";
const NAME: &str = "BitBraid-1.2.0-x86_64.AppImage";

/// The releases, from memory: what each address answers, and every address
/// asked, in order.
#[derive(Debug, Default)]
struct Releases {
    files: Mutex<HashMap<String, Vec<u8>>>,
    asked: Mutex<Vec<String>>,
}

impl Releases {
    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl Fetch for Releases {
    fn send(&self, call: &Call, _cancel: &AtomicBool) -> Result<Response, FetchError> {
        let url = call.url.to_string();
        self.asked.lock().unwrap().push(url.clone());
        Ok(match self.files.lock().unwrap().get(&url) {
            Some(body) => Response {
                status: 200,
                body: body.clone(),
                ..Response::default()
            },
            None => Response {
                status: 404,
                ..Response::default()
            },
        })
    }
}

/// The configuration directory of the window under test.
fn config_dir() -> PathBuf {
    crate::hermetic::inside()
        .expect("the tests run in scratch surroundings")
        .join("config/bitbraid")
}

/// Put a stand-in updater in place for the window under test: a copy at
/// 1.0.0 that is the AppImage `image`, and a feed of 1.2.0 with `package`
/// as its AppImage, signed.
fn stand_in(image: &Path, package: &[u8]) -> Arc<Releases> {
    let signer = Signer::new(3).unwrap();
    let feed = json!({
        "schema": 1,
        "version": "1.2.0",
        "page": "https://releases.example.test/v1.2.0",
        "packages": [{
            "system": "linux",
            "arch": "x86_64",
            "kind": "appimage",
            "name": NAME,
            "url": PACKAGE,
            "size": package.len(),
            "sha256": sha256_hex(package),
            "signature": signer.sign(package, &format!("version:1.2.0 file:{NAME}")),
        }],
    });
    let releases = Arc::new(Releases::default());
    {
        let mut files = releases.files.lock().unwrap();
        files.insert(FEED.to_owned(), feed.to_string().into_bytes());
        files.insert(PACKAGE.to_owned(), package.to_vec());
    }
    let updater = Updater {
        fetch: releases.clone(),
        key: UpdateKey::from_text(&signer.public_key()),
        feed: FEED.to_owned(),
        current: bitbraid_integrations::update::feed::version("1.0.0").unwrap(),
        installation: Installation::AppImage(image.to_path_buf()),
        target: Some(Target {
            system: System::Linux,
            arch: Arch::X86_64,
        }),
        downloads: Some(crate::repos::work().join("update-downloads")),
    };
    Updater::install_for(&config_dir(), Arc::new(updater));
    releases
}

/// The real updater again, for the tests that come after.
fn put_back() {
    Updater::install_for(&config_dir(), Updater::this_copy());
}

/// Show the preferences' General section scrolled down to its updates.
fn show_updates(stage: &mut Stage) {
    if stage.shell().sheet != Some("preferences") {
        stage.act(keymap::ShowPreferences);
    }
    assert_eq!(stage.shell().preferences_section, Some("general"));
    for _ in 0..40 {
        if stage.shows("updates-said") {
            break;
        }
        crate::behaviour_fit::wheel_at(stage, (800., 400.), -200.);
    }
    assert!(
        stage.shows("check-Look for updates"),
        "the switch is on show"
    );
    assert!(stage.shows("updates-said"), "and what this copy is");
    stage.pointer_away();
}

fn updates_on(stage: &mut Stage) -> bool {
    stage.app(|cx| cx.global::<bitbraid_ui::prefs::Prefs>().preferences.updates)
}

/// Off, as it comes: the preferences say what this copy is without asking
/// anybody, their button to look is greyed, and the palette's command says
/// that updates are off. Turned on, the feed is asked at once and a newer
/// version is said, with a button that installs it.
fn nothing_is_asked_about_updates_until_they_are_turned_on() {
    crate::repos::fresh();
    let image = crate::repos::work().join("BitBraid.AppImage");
    std::fs::write(&image, b"the copy that runs").unwrap();
    let releases = stand_in(&image, b"the new version");
    let mut stage = Stage::open(Start::bare());
    show_updates(&mut stage);
    assert!(!updates_on(&mut stage));
    stage.until("what this copy is was read", |stage| {
        stage.app(|cx| {
            cx.global::<bitbraid_ui::updates::Updates>()
                .status
                .is_some()
        })
    });
    stage.click("updates-look");
    stage.act(keymap::TogglePalette);
    stage.type_text("Check for updates");
    stage.keys("enter");
    stage.until("the palette's command answers", |stage| {
        stage
            .notices()
            .iter()
            .any(|notice| notice.starts_with("Updates are off"))
    });
    assert!(releases.asked().is_empty(), "{:?}", releases.asked());

    show_updates(&mut stage);
    stage.click("check-Look for updates");
    assert!(updates_on(&mut stage));
    let written = std::fs::read_to_string(config_dir().join("preferences.toml")).unwrap();
    assert!(written.contains("updates = true"), "{written}");
    stage.until("a newer version is said", |stage| {
        stage
            .notices()
            .iter()
            .any(|notice| notice == "BitBraid 1.2.0 is out; this is 1.0.0.")
    });
    assert_eq!(releases.asked(), [FEED]);
    stage.until("the row offers to install it", |stage| {
        stage.shows("updates-install")
    });

    // Turned off again: nothing more is asked.
    stage.click("check-Look for updates");
    assert!(!updates_on(&mut stage));
    stage.click("updates-look");
    assert_eq!(releases.asked(), [FEED]);
    put_back();
}

/// On from the start: the feed is asked as the window opens and the newer
/// version said; installed from the preferences, the AppImage is the new
/// one, the old one kept beside it, and a restart is offered.
fn a_newer_version_is_installed_from_the_preferences_with_a_restart_offered() {
    crate::repos::fresh();
    let image = crate::repos::work().join("BitBraid.AppImage");
    std::fs::write(&image, b"the copy that runs").unwrap();
    let releases = stand_in(&image, b"the new version");
    let mut stage = Stage::open(Start::bare().preferences("updates = true\n"));
    stage.until("a newer version is said at the start", |stage| {
        stage
            .notices()
            .iter()
            .any(|notice| notice == "BitBraid 1.2.0 is out; this is 1.0.0.")
    });
    show_updates(&mut stage);
    stage.until("the row offers to install it", |stage| {
        stage.shows("updates-install")
    });
    stage.click("updates-install");
    stage.until("it is installed", |stage| {
        stage
            .notices()
            .iter()
            .any(|notice| notice == "BitBraid 1.2.0 is installed. Restart BitBraid to use it.")
    });
    assert_eq!(std::fs::read(&image).unwrap(), b"the new version");
    assert_eq!(
        std::fs::read(image.with_file_name("BitBraid.AppImage.old")).unwrap(),
        b"the copy that runs"
    );
    assert_eq!(releases.asked(), [FEED, FEED, PACKAGE]);
    stage.until("the row offers the restart", |stage| {
        stage.shows("updates-restart")
    });
    put_back();
}

/// The preferences' updates, on, with a newer version found that this copy
/// installs itself.
pub fn pictures(shots: &mut Shots) {
    crate::repos::fresh();
    // A path that is the same on every machine: nothing is installed here.
    let releases = stand_in(
        Path::new("/home/ada/Applications/BitBraid.AppImage"),
        b"the new version",
    );
    let mut stage = Stage::open(Start::bare().preferences("updates = true\n").pictures());
    stage.until("a newer version was found", |stage| {
        stage.app(|cx| {
            let updates = cx.global::<bitbraid_ui::updates::Updates>();
            updates.found.is_some() && updates.status.is_some()
        })
    });
    assert_eq!(releases.asked(), [FEED]);
    show_updates(&mut stage);
    assert!(stage.shows("updates-install"));
    shots.take(&mut stage, "preferences_updates", Themes::Two);
    put_back();
}
