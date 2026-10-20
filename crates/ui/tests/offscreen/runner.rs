//! Runs the tests one after the other on the main thread, where the
//! toolkit wants its application, and says what came of each the way
//! cargo's own harness does.

use std::{
    collections::BTreeSet,
    fs,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
    process::ExitCode,
    time::Instant,
};

use bitbraid_ui::prefs::ThemeChoice;
use image::RgbaImage;

use crate::{compare, stage::Stage};

/// One test: a name and what it does. It fails by panicking.
#[derive(Clone, Copy)]
pub struct Test {
    pub name: &'static str,
    pub run: fn(),
}

/// One or several views, opened and brought into the state that is to be
/// looked at, each handed to [`Shots::take`].
pub struct Views {
    pub name: &'static str,
    pub run: fn(&mut Shots),
    /// Programs it shows something of: without them it is not drawn.
    pub needs: &'static [&'static str],
}

/// What is done with the pictures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Compare each with the stored one.
    Check,
    /// Store each that differs from the stored one.
    Update,
}

/// The themes a view is drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Themes {
    /// Dark and light.
    Two,
    /// Dark, light and high contrast.
    Three,
    /// Every one of ours.
    All,
}

fn theme_word(theme: &ThemeChoice) -> &'static str {
    match theme {
        ThemeChoice::Dark => "dark",
        ThemeChoice::Light => "light",
        ThemeChoice::Contrast => "contrast",
        ThemeChoice::Dusk => "dusk",
        ThemeChoice::Ember => "ember",
        ThemeChoice::Graphite => "graphite",
        ThemeChoice::Paper => "paper",
        ThemeChoice::Sage => "sage",
        ThemeChoice::User(_) => "user",
    }
}

/// Where the stored pictures are: in the repository, beside the tests.
pub fn stored_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/snapshots")
}

/// Where pictures to look at are written: `snapshots` in the target
/// directory. Never part of the repository.
pub fn review_dir() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR"))
        .parent()
        .map(|target| target.join("snapshots"))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("snapshots"))
}

/// Takes the pictures of a run and keeps what came of each.
pub struct Shots {
    mode: Mode,
    /// Every picture taken, as `<view>.<theme>`.
    taken: BTreeSet<String>,
    /// What is wrong, one line each.
    wrong: Vec<String>,
    stored: usize,
    same: usize,
    /// How far the furthest pixel of a picture that passed was from the
    /// stored one: nought on the machine that stored them.
    furthest: u8,
}

impl Shots {
    fn new(mode: Mode) -> Self {
        Self {
            mode,
            taken: BTreeSet::new(),
            wrong: Vec::new(),
            stored: 0,
            same: 0,
            furthest: 0,
        }
    }

    /// Take the picture of the window as it is now, under `view`, in
    /// each of the themes. The window is left in the dark theme.
    pub fn take(&mut self, stage: &mut Stage, view: &'static str, themes: Themes) {
        let count = match themes {
            Themes::Two => 2,
            Themes::Three => 3,
            Themes::All => ThemeChoice::BUILT_IN.len(),
        };
        for theme in ThemeChoice::BUILT_IN.iter().take(count) {
            stage.use_theme(theme);
            let frame = stage.picture();
            self.keep(&format!("{view}.{}", theme_word(theme)), &frame);
        }
        stage.use_theme(&ThemeChoice::Dark);
    }

    fn keep(&mut self, name: &str, frame: &RgbaImage) {
        assert!(
            self.taken.insert(name.to_owned()),
            "two pictures are called {name}"
        );
        let drawn = compare::reduce(frame);
        let stored_path = stored_dir().join(format!("{name}.png"));
        let review = review_dir();
        fs::create_dir_all(&review).unwrap();
        let stored = image::open(&stored_path).ok().map(|image| image.to_rgba8());
        let verdict = stored
            .as_ref()
            .map(|stored| compare::compare(stored, &drawn));
        // Leftovers of an earlier run would be taken for this one's.
        for kind in ["actual", "difference"] {
            let _ = fs::remove_file(review.join(format!("{name}.{kind}.png")));
        }
        match (self.mode, verdict) {
            (_, Some(verdict)) if verdict.same() => {
                self.same += 1;
                self.furthest = self.furthest.max(verdict.worst);
                println!("picture {name} ... ok");
            }
            (Mode::Update, verdict) => {
                fs::create_dir_all(stored_dir()).unwrap();
                save(&drawn, &stored_path);
                self.stored += 1;
                match verdict {
                    Some(verdict) => println!("picture {name} ... stored anew: {}", verdict.said()),
                    None => println!("picture {name} ... stored for the first time"),
                }
            }
            (Mode::Check, None) => {
                save(frame, &review.join(format!("{name}.actual.png")));
                println!("picture {name} ... FAILED: no stored picture");
                self.wrong.push(format!(
                    "{name}: there is no stored picture of it; look at {} and store it with \
                     `cargo xtask snapshot`",
                    review.join(format!("{name}.actual.png")).display()
                ));
            }
            (Mode::Check, Some(verdict)) => {
                let actual = review.join(format!("{name}.actual.png"));
                let difference = review.join(format!("{name}.difference.png"));
                save(frame, &actual);
                if let Some(stored) = &stored {
                    save(&compare::difference(stored, &drawn), &difference);
                }
                println!("picture {name} ... FAILED: {}", verdict.said());
                self.wrong.push(format!(
                    "{name}: {}\n    drawn now:  {}\n    stored | drawn | where they differ:  {}",
                    verdict.said(),
                    actual.display(),
                    difference.display()
                ));
            }
        }
        // In full size for whoever wants to look at a view, whatever the
        // comparison said.
        if self.mode == Mode::Update {
            save(frame, &review.join(format!("{name}.png")));
        }
    }
}

fn save(image: &RgbaImage, path: &Path) {
    use image::ImageEncoder as _;
    // Opaque pictures: three channels, and the smallest file the encoder
    // makes, since the stored ones are kept in the repository.
    let rgb = image::DynamicImage::ImageRgba8(image.clone()).to_rgb8();
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new_with_quality(
        &mut bytes,
        image::codecs::png::CompressionType::Best,
        image::codecs::png::FilterType::Adaptive,
    )
    .write_image(
        rgb.as_raw(),
        rgb.width(),
        rgb.height(),
        image::ExtendedColorType::Rgb8,
    )
    .unwrap_or_else(|err| panic!("encoding {}: {err}", path.display()));
    fs::write(path, bytes).unwrap_or_else(|err| panic!("writing {}: {err}", path.display()));
}

struct Arguments {
    snapshots: Option<Mode>,
    filters: Vec<String>,
    list: bool,
    nothing: bool,
}

fn arguments(given: &[String]) -> Result<Arguments, String> {
    let mut out = Arguments {
        snapshots: None,
        filters: Vec::new(),
        list: false,
        nothing: false,
    };
    let mut skip_value = false;
    for argument in given {
        if skip_value {
            skip_value = false;
            continue;
        }
        match argument.as_str() {
            "--snapshots=check" => out.snapshots = Some(Mode::Check),
            "--snapshots=update" => out.snapshots = Some(Mode::Update),
            "--list" => out.list = true,
            // Nothing here is ignored or a benchmark.
            "--ignored" | "--bench" => out.nothing = true,
            // What cargo's own harness takes and this one has no use for.
            "--test-threads" | "--color" | "--format" | "--skip" | "-Z" => skip_value = true,
            other if other.starts_with("--snapshots") => {
                return Err(format!(
                    "{other}: it is --snapshots=check or --snapshots=update"
                ));
            }
            other if other.starts_with('-') => {}
            filter => out.filters.push(filter.to_owned()),
        }
    }
    Ok(out)
}

fn wanted(name: &str, filters: &[String]) -> bool {
    filters.is_empty() || filters.iter().any(|filter| name.contains(filter.as_str()))
}

fn said(panic: Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|text| (*text).to_owned()))
        .unwrap_or_else(|| "it panicked".to_owned())
}

fn on_path(program: &str) -> bool {
    crate::repos::on_path(program)
}

/// Run what the arguments ask for, inside the scratch surroundings.
pub fn run(given: &[String], tests: &[Test], views: &[Views]) -> ExitCode {
    let arguments = match arguments(given) {
        Ok(arguments) => arguments,
        Err(wrong) => {
            println!("offscreen: {wrong}");
            return ExitCode::FAILURE;
        }
    };
    if arguments.list {
        for test in tests {
            println!("{}: test", test.name);
        }
        for view in views {
            println!("{}: views", view.name);
        }
        return ExitCode::SUCCESS;
    }
    if arguments.nothing {
        println!("\nrunning 0 tests\n\ntest result: ok. 0 passed; 0 failed");
        return ExitCode::SUCCESS;
    }
    match arguments.snapshots {
        None => run_tests(tests, &arguments.filters),
        Some(mode) => run_views(views, mode, &arguments.filters),
    }
}

fn run_tests(tests: &[Test], filters: &[String]) -> ExitCode {
    let chosen: Vec<&Test> = tests
        .iter()
        .filter(|test| wanted(test.name, filters))
        .collect();
    println!("\nrunning {} tests", chosen.len());
    let started = Instant::now();
    let mut failed = Vec::new();
    for test in &chosen {
        let began = Instant::now();
        match catch_unwind(AssertUnwindSafe(test.run)) {
            Ok(()) => println!("test {} ... ok ({:.1?})", test.name, began.elapsed()),
            Err(panic) => {
                println!("test {} ... FAILED", test.name);
                failed.push((test.name, said(panic)));
            }
        }
    }
    if !failed.is_empty() {
        println!("\nfailures:");
        for (name, why) in &failed {
            println!("    {name}: {why}");
        }
    }
    println!(
        "\ntest result: {}. {} passed; {} failed; {} filtered out; finished in {:.2?}",
        if failed.is_empty() { "ok" } else { "FAILED" },
        chosen.len() - failed.len(),
        failed.len(),
        tests.len() - chosen.len(),
        started.elapsed()
    );
    println!(
        "The views are not compared with their stored pictures here: \
         `cargo xtask snapshot --check` does that."
    );
    if failed.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn run_views(views: &[Views], mode: Mode, filters: &[String]) -> ExitCode {
    crate::stage::listen_for_adapter();
    if let Some(why) = crate::stage::no_renderer() {
        println!(
            "snapshot: skipped: {why}. Nothing was compared. The views were not drawn; the \
             tests of what they do (`cargo test -p bitbraid-ui --test offscreen`) need no \
             renderer and still run."
        );
        return ExitCode::SUCCESS;
    }
    // Which adapter draws, and which one drew what is stored: a picture
    // that is a little off everywhere on another adapter is its rounding.
    let adapter = crate::stage::adapter();
    let stored_by_file = stored_dir().join("drawn-by.txt");
    let stored_by = fs::read_to_string(&stored_by_file)
        .map(|text| text.trim().to_owned())
        .unwrap_or_default();
    println!("snapshot: drawn by {adapter}");
    if !stored_by.is_empty() && stored_by != adapter {
        println!("snapshot: the stored pictures were last drawn by {stored_by}");
    }
    let started = Instant::now();
    let mut shots = Shots::new(mode);
    let mut skipped = Vec::new();
    let mut ran_all = true;
    for view in views {
        if !wanted(view.name, filters) {
            ran_all = false;
            continue;
        }
        if let Some(missing) = view.needs.iter().find(|program| !on_path(program)) {
            println!(
                "views {} ... skipped: {missing} is not installed",
                view.name
            );
            skipped.push(view.name);
            ran_all = false;
            continue;
        }
        let before = shots.taken.len();
        let began = Instant::now();
        match catch_unwind(AssertUnwindSafe(|| (view.run)(&mut shots))) {
            Ok(()) => println!(
                "views {} ... {} pictures ({:.1?})",
                view.name,
                shots.taken.len() - before,
                began.elapsed()
            ),
            Err(panic) => {
                println!("views {} ... FAILED", view.name);
                shots.wrong.push(format!(
                    "{}: it did not get as far as its pictures: {}",
                    view.name,
                    said(panic)
                ));
                ran_all = false;
            }
        }
    }
    // A stored picture no view takes any more is not looked at by anyone.
    if ran_all && let Ok(entries) = fs::read_dir(stored_dir()) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .unwrap_or_default();
            if path.extension().is_some_and(|ext| ext == "png") && !shots.taken.contains(name) {
                match mode {
                    Mode::Update => {
                        fs::remove_file(&path).unwrap();
                        println!("picture {name} ... removed: no view takes it any more");
                    }
                    Mode::Check => shots.wrong.push(format!(
                        "{name}: a stored picture no view takes any more; `cargo xtask snapshot` \
                         removes it"
                    )),
                }
            }
        }
    }
    let bytes: u64 = fs::read_dir(stored_dir())
        .map(|entries| {
            entries
                .flatten()
                .filter_map(|entry| entry.metadata().ok())
                .map(|meta| meta.len())
                .sum()
        })
        .unwrap_or(0);
    println!(
        "\nsnapshot: {} pictures, {} as stored (the furthest pixel {} of 255 off, {} allowed), \
         {} stored anew, {} wrong; {} sets of views skipped; \
         {:.1} MB stored in {}; finished in {:.1?}",
        shots.taken.len(),
        shots.same,
        shots.furthest,
        compare::CHANNEL_TOLERANCE,
        shots.stored,
        shots.wrong.len(),
        skipped.len(),
        bytes as f64 / (1024.0 * 1024.0),
        stored_dir().display(),
        started.elapsed()
    );
    if mode == Mode::Update {
        println!("Every view in full size is in {}.", review_dir().display());
        // Said only when a picture was stored: the others are still the
        // adapter's that is named there.
        if shots.stored > 0 && stored_by != adapter {
            fs::write(&stored_by_file, format!("{adapter}\n")).unwrap();
        }
    }
    if shots.wrong.is_empty() {
        return ExitCode::SUCCESS;
    }
    println!("\nsnapshot: what is wrong (view.theme):");
    for wrong in &shots.wrong {
        println!("  {wrong}");
    }
    if !stored_by.is_empty() && stored_by != adapter {
        println!(
            "\nThe stored pictures were drawn by {stored_by}, these by {adapter}. Pictures that \
             differ a little in many places, with nothing to see in them, are the two rounding \
             differently: see `AGENTS.md`, section 3."
        );
    }
    println!(
        "\nA view that changed on purpose: look at the pictures named above, then store the new \
         ones with `cargo xtask snapshot` and commit them."
    );
    ExitCode::FAILURE
}
