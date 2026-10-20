//! `cargo xtask appbench`: the application itself, measured off screen.
//!
//! Opens a repository in the real application inside a headless compositor
//! (`cage`), lets it scroll the graph up and down for a while
//! (`BITBRAID_BENCH`), and reads what it reports when it ends: frames per
//! second and the most memory it held. By default the repository is the
//! synthetic one of a million commits, which is what the memory budget is
//! about.
//!
//! With `--resize` the graph stands still and the border of its graph
//! column is dragged to and fro instead (`BITBRAID_BENCH_RESIZE`): the
//! application sends itself the pointer's events, the way the system
//! would, and counts the frames drawn while every one of them resizes the
//! columns.
//!
//! With `--terminal` it measures the terminal instead
//! (`BITBRAID_BENCH_TERMINAL`): the repository's terminal opens and a
//! program streams coloured lines into it, and the application says how
//! many frames of it were drawn each second. The repository is this one
//! unless another is named.
//!
//! With `--tabs` it measures a tab switch instead (`BITBRAID_BENCH_TABS`,
//! `crates/ui/src/shell/tab_bench.rs`): a tab of the start page is opened
//! beside the repository's, and the main thread's time of each switch back
//! to the repository is reported.
//!
//! With `--many-refs` the repository is a generated one with many
//! references (`bitbraid_testkit::build_many_refs`). Every run also reports
//! how long the references panel took to build its rows (`BITBRAID_DEBUG`'s
//! line).
//!
//! Nothing of the developer's is touched: configuration, home, gpg home and
//! git configuration are a scratch directory that is removed afterwards,
//! and the run has no display but its own and no ssh agent.
//!
//! What the graphics driver compiles of the toolkit's shaders when the
//! window is made is kept between runs, in a directory of the bench target,
//! as it is kept in a person's home: a person waits for that compiling on
//! the first start after an install or a driver update and not again.
//! Before it measures, the application is started once briefly,
//! unmeasured, so that the binary is read from the disk and its shaders
//! compiled. `--cold` measures that first start instead: an empty shader
//! cache and no start before.
//!
//! Linux with `cage` installed, or Windows; not part of `cargo xtask ci`.
//! Windows has no headless compositor: the window opens on the display,
//! which should show it in front and be left alone while it runs. There
//! the system finds the user's folders (the application's configuration
//! and cache, the graphics driver's shaders) from `USERPROFILE`, not from
//! `APPDATA` or `LOCALAPPDATA`: the run's `USERPROFILE` is a home kept
//! between runs (`<bench target>/appbench-home`, a scratch one with
//! `--cold`), as the shader cache is kept on Linux, and the application's
//! own configuration in it is emptied and written anew before each start.
//! git's `HOME` is the run's scratch one. The main thread's time a frame
//! is not reported there.
//!
//! Sources: cage(1) (`WLR_BACKENDS=headless`), proc(5) for what the
//! application reads (`crates/ui/src/memory.rs`); NVIDIA's README for the
//! Linux driver, "Environment variables" (`__GL_SHADER_DISK_CACHE_PATH`,
//! which its Vulkan driver follows too); Mesa's "Environment variables"
//! (`MESA_SHADER_CACHE_DIR`); Microsoft Learn, "Recognized environment
//! variables" (`USERPROFILE`, `APPDATA`, `LOCALAPPDATA`) and "Known folders"
//! (`FOLDERID_RoamingAppData`, `FOLDERID_LocalAppData`).

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use bitbraid_testkit::{CorpusSpec, ManyRefsSpec, build_many_refs, build_synthetic};

const USAGE: &str = "\
cargo xtask appbench [--terminal | --resize | --tabs] [--seconds N] [--repo <path> | --many-refs] [--bin <path>] [--budget-mib N] [--cold]

  Runs the application off screen (on Windows, on the display) on a
  repository, scrolling its graph for N seconds (default 12), and prints
  its report: frames per second and peak resident memory. Fails when the
  peak is over the budget (default 1536 MiB). It
  is started once briefly before, unmeasured, and the graphics driver's
  shader cache is kept between runs (<bench target>/appbench-shaders; on
  Windows the user's home, <bench target>/appbench-home, with the
  application's configuration in it written anew), as a person's home
  keeps it.

  --resize        drag the graph column's border to and fro instead of
                  scrolling: frames per second while the columns resize
  --terminal      measure the terminal instead of the graph: its frames per
                  second while a program streams coloured lines into it
  --tabs          measure switching to the repository's tab from a tab of
                  the start page, every second: the main thread's time each
  --repo <path>   the repository to open (default: the synthetic one of a
                  million commits under <bench target>/corpus, generated if
                  it is not there; with --terminal, this repository)
  --many-refs     open the generated repository with many references
                  (33,000 commits, 4,300 references, 19 worktrees, 14,000
                  files) under <bench target>/corpus, made the first time
  --bin <path>    the application binary (default: a release build into
                  the bench target)
  --cold          the first start after an install or a driver update: an
                  empty shader cache, and no start before the measured one
";

/// The budget for a million commits open: 1.5 GB.
const BUDGET_MIB: u64 = 1536;

pub fn run(args: &[String]) -> Result<()> {
    let mut seconds = 12u64;
    let mut repo: Option<PathBuf> = None;
    let mut bin: Option<PathBuf> = None;
    let mut budget = BUDGET_MIB;
    let mut terminal = false;
    let mut resize = false;
    let mut tabs = false;
    let mut many_refs = false;
    let mut cold = false;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |what: &str| rest.next().with_context(|| format!("{arg} needs {what}"));
        match arg.as_str() {
            "--seconds" => seconds = value("a number")?.parse().context("--seconds")?,
            "--budget-mib" => budget = value("a number")?.parse().context("--budget-mib")?,
            "--repo" => repo = Some(PathBuf::from(value("a path")?)),
            "--bin" => bin = Some(PathBuf::from(value("a path")?)),
            "--terminal" => terminal = true,
            "--resize" => resize = true,
            "--tabs" => tabs = true,
            "--many-refs" => many_refs = true,
            "--cold" => cold = true,
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => bail!("unknown argument {other:?}\n{USAGE}"),
        }
    }
    if [terminal, resize, tabs]
        .iter()
        .filter(|asked| **asked)
        .count()
        > 1
    {
        bail!("--terminal, --resize and --tabs measure different things; ask for one");
    }
    if many_refs && repo.is_some() {
        bail!("--many-refs is a repository of its own; leave out --repo");
    }
    if !(1..=3600).contains(&seconds) {
        bail!("--seconds must be between 1 and 3600");
    }
    if !cfg!(windows) && Command::new("cage").arg("-v").output().is_err() {
        bail!("`cage` is not installed; the application cannot be run off screen without it");
    }

    let target = crate::corpus::bench_target();
    let generated_here =
        std::env::var_os("CARGO_TARGET_DIR").is_some_and(|dir| Path::new(&dir) == target);
    let repo = match repo {
        Some(path) => {
            canonical(&path).with_context(|| format!("no repository at {}", path.display()))?
        }
        None if many_refs => {
            // The testkit generates under the target directory it is told
            // about, which has to be the bench target, absolutely.
            if !generated_here {
                bail!(
                    "set CARGO_TARGET_DIR={} to have the repository with many references generated there",
                    target.display()
                );
            }
            println!("==> the repository with many references (generated the first time)");
            build_many_refs(&ManyRefsSpec::default())?
        }
        None if terminal => crate::root(),
        None => {
            let spec = CorpusSpec::default();
            let there = target.join("corpus").join(format!(
                "synthetic-{}-l{}-m{}-f{}",
                spec.commits, spec.lanes, spec.merge_every, spec.files
            ));
            if there.join(".corpus-ok").is_file() {
                there
            } else if generated_here {
                // The testkit generates under the target directory it is
                // told about, which has to be the bench target, absolutely.
                build_synthetic(&spec)?
            } else {
                bail!(
                    "the synthetic repository is not at {}; set CARGO_TARGET_DIR={} to have it generated there, or pass --repo",
                    there.display(),
                    target.display()
                );
            }
        }
    };
    // As the application names it, which is what its state is kept under:
    // a generated one is under the target directory as written, which may
    // have the other slashes on Windows.
    let repo = canonical(&repo).with_context(|| format!("no repository at {}", repo.display()))?;
    let bin = match bin {
        Some(path) => {
            canonical(&path).with_context(|| format!("no binary at {}", path.display()))?
        }
        None => {
            println!("==> cargo build --release --package bitbraid");
            let status = Command::new("cargo")
                .args(["build", "--release", "--package", "bitbraid"])
                .env("CARGO_TARGET_DIR", &target)
                .current_dir(crate::root())
                .status()
                .context("spawning cargo build")?;
            if !status.success() {
                bail!("building the application failed with {status}");
            }
            target
                .join("release")
                .join(format!("bitbraid{}", std::env::consts::EXE_SUFFIX))
        }
    };

    let scratch = std::env::temp_dir().join(format!("bitbraid-appbench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    let variables: &[&str] = if terminal {
        &["BITBRAID_BENCH_TERMINAL"]
    } else if tabs {
        &["BITBRAID_BENCH_TABS"]
    } else if resize {
        &["BITBRAID_BENCH", "BITBRAID_BENCH_RESIZE"]
    } else {
        &["BITBRAID_BENCH"]
    };
    // What the driver compiled of the shaders, kept as a person's home
    // keeps it; an empty one of the scratch directory for a first start.
    // On Windows the driver keeps them in the user's home, which is kept
    // the same way.
    let kept = if cfg!(windows) {
        "appbench-home"
    } else {
        "appbench-shaders"
    };
    let shaders = if cold {
        scratch.join("shaders")
    } else {
        target.join(kept)
    };
    std::fs::create_dir_all(&shaders).with_context(|| format!("creating {}", shaders.display()))?;
    if !cold {
        // Started once, unmeasured: the binary read from the disk and the
        // shaders compiled, as before any start but the first.
        println!("==> a start before the measured one");
        let warm = scratch.join("warm");
        let _ = measure(&warm, &bin, &repo, &shaders, 1, &["BITBRAID_BENCH"]);
        tidy(&warm);
    }
    let report = measure(&scratch, &bin, &repo, &shaders, seconds, variables);
    tidy(&scratch);
    let report = report?;
    println!(
        "==> {}",
        if cold {
            "the first start: an empty shader cache, no start before"
        } else {
            "started once before, the shader cache kept"
        }
    );

    println!("{report}");
    let field = |name: &str| -> Option<&str> {
        report
            .split_whitespace()
            .find_map(|part| part.strip_prefix(name)?.strip_prefix('='))
    };
    let Some(peak) = field("peak_rss_mib").and_then(|text| text.parse::<u64>().ok()) else {
        bail!("the application did not report its memory (it does on Linux, macOS and Windows)");
    };
    if tabs {
        println!(
            "appbench: {} switches to the repository's tab, main thread {} ms each at the median, {} on \
             average, {} at most, {} at least; peak resident memory {peak} MiB of {budget} MiB; window \
             open after {}, graph drawn after {}, references panel {}",
            field("tab_switches").unwrap_or("?"),
            field("tab_switch_ms_median").unwrap_or("?"),
            field("tab_switch_ms_mean").unwrap_or("?"),
            field("tab_switch_ms_max").unwrap_or("?"),
            field("tab_switch_ms_min").unwrap_or("?"),
            field("window_open").unwrap_or("?"),
            field("first_graph_render").unwrap_or("?"),
            field("refs_panel").unwrap_or("?"),
        );
    } else if terminal {
        println!(
            "appbench: terminal of {} by {} cells, {} fps on average ({} at least) while a program streams \
             into it, peak resident memory {peak} MiB of {budget} MiB",
            field("columns").unwrap_or("?"),
            field("rows").unwrap_or("?"),
            field("avg_fps").unwrap_or("?"),
            field("min_fps").unwrap_or("?"),
        );
    } else if resize {
        println!(
            "appbench: {} rows, {} fps on average ({} at least) while the graph column's border is dragged \
             ({} of {} pointer events resized the columns), {} ms of the main thread per frame, \
             peak resident memory {peak} MiB of {budget} MiB",
            field("rows").unwrap_or("?"),
            field("avg_fps").unwrap_or("?"),
            field("min_fps").unwrap_or("?"),
            field("resizes").unwrap_or("?"),
            field("sent").unwrap_or("?"),
            field("main_thread_ms_per_frame").unwrap_or("?"),
        );
        // A drag that never took measured a graph standing still.
        if field("resizes").and_then(|text| text.parse::<u64>().ok()) == Some(0) {
            bail!("the border was never taken: no pointer event resized the columns");
        }
    } else {
        println!(
            "appbench: {} rows, {} fps on average ({} at least), {} ms of the main thread per frame, \
             peak resident memory {peak} MiB of {budget} MiB; the toolkit's window {}, window open after {}, \
             graph drawn after {}, first commit's details on screen after {}; references panel {} on open, \
             {} ms at most over {} builds ({} rows then)",
            field("rows").unwrap_or("?"),
            field("avg_fps").unwrap_or("?"),
            field("min_fps").unwrap_or("?"),
            field("main_thread_ms_per_frame").unwrap_or("?"),
            field("toolkit_window").unwrap_or("?"),
            field("window_open").unwrap_or("?"),
            field("first_graph_render").unwrap_or("?"),
            field("first_commit_details").unwrap_or("?"),
            field("refs_panel").unwrap_or("?"),
            field("refs_panel_max_ms").unwrap_or("?"),
            field("refs_panel_builds").unwrap_or("?"),
            field("refs_panel_slowest_rows").unwrap_or("?"),
        );
    }
    if peak > budget {
        bail!("peak resident memory {peak} MiB is over the budget of {budget} MiB");
    }
    Ok(())
}

/// Remove a run's scratch directory, and what the application started for
/// its home: it asks gpg for keys.
fn tidy(scratch: &Path) {
    let _ = Command::new("gpgconf")
        .arg("--homedir")
        .arg(scratch.join("gnupg"))
        .args(["--kill", "all"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = std::fs::remove_dir_all(scratch);
}

/// Run the application on `repo` for `seconds` of measuring (`variables`
/// say what is measured), with the graphics driver's shader cache in
/// `shaders`, and return its `APP_BENCH` line, with when the window was
/// open and the graph drawn.
fn measure(
    scratch: &Path,
    bin: &Path,
    repo: &Path,
    shaders: &Path,
    seconds: u64,
    variables: &[&str],
) -> Result<String> {
    let config = scratch.join("config");
    let home = scratch.join("home");
    let gnupg = scratch.join("gnupg");
    // Where the application keeps its state: under `XDG_CONFIG_HOME`, or on
    // Windows in the roaming folder of the home kept with the shaders, as
    // `directories` lays it out; emptied first, as a scratch one is.
    let roaming = shaders.join("AppData").join("Roaming");
    let state_dir = if cfg!(windows) {
        let own = roaming.join("bitbraid");
        if own.exists() {
            std::fs::remove_dir_all(&own).with_context(|| format!("emptying {}", own.display()))?;
        }
        own.join("config")
    } else {
        config.join("bitbraid")
    };
    for dir in [&state_dir, &home, &gnupg] {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&gnupg, std::fs::Permissions::from_mode(0o700))?;
    }
    let git_config = scratch.join("gitconfig");
    std::fs::write(
        &git_config,
        "[user]\n\tname = App Bench\n\temail = bench@scratch.invalid\n",
    )?;
    // The references panel's sections open, so that its rows are built
    // (a configuration of nothing starts with them closed).
    std::fs::write(
        state_dir.join("state.toml"),
        format!("[repos.{repo:?}]\nclosed_sections = []\n"),
    )?;

    let mut command = if cfg!(windows) {
        // No compositor of our own but the display's; the user's folders,
        // which the system finds from `USERPROFILE`, are the kept home's.
        let mut command = Command::new(bin);
        command
            .arg(repo)
            .env("USERPROFILE", shaders)
            .env("APPDATA", &roaming)
            .env("LOCALAPPDATA", shaders.join("AppData").join("Local"));
        command
    } else {
        let mut command = Command::new("cage");
        command
            .arg("--")
            .arg(bin)
            .arg(repo)
            .env("WLR_BACKENDS", "headless")
            .env("WLR_LIBINPUT_NO_DEVICES", "1");
        command
    };
    let mut child = command
        .envs(
            variables
                .iter()
                .map(|variable| (variable, seconds.to_string())),
        )
        // The terminal's shell, whatever the developer's is.
        .env("SHELL", "/bin/sh")
        // The session says when the window was up and the graph drawn.
        .env("BITBRAID_DEBUG", "1")
        .env("XDG_CONFIG_HOME", &config)
        .env("HOME", &home)
        .env("GNUPGHOME", &gnupg)
        .env("GIT_CONFIG_GLOBAL", &git_config)
        // The driver's compiled shaders, which would otherwise go under
        // the scratch home and be compiled again by every run.
        .env("__GL_SHADER_DISK_CACHE_PATH", shaders)
        .env("MESA_SHADER_CACHE_DIR", shaders)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .env_remove("SSH_AUTH_SOCK")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(std::fs::File::create(scratch.join("stderr"))?)
        .spawn()
        .context("starting the application")?;
    let mut stdout = child.stdout.take().context("the application's output")?;
    let reader = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut text);
        text
    });
    // Opening a million commits takes a few seconds before the scrolling
    // starts; a run that is not done long after is stuck.
    let limit = Duration::from_secs(seconds.saturating_add(180));
    let started = Instant::now();
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            bail!("the application did not finish within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let text = reader.join().unwrap_or_default();
    let report = text
        .lines()
        .find(|line| line.starts_with("APP_BENCH "))
        .map(str::trim_end)
        .with_context(|| format!("the application ended without a report; it said:\n{text}"))?;
    // When each step of starting up was reached, from the process's start.
    let said = std::fs::read_to_string(scratch.join("stderr")).unwrap_or_default();
    let reached = |what: &str| -> String {
        said.lines()
            .find_map(|line| line.split_once(&format!("milestone {what} at ")))
            .map_or_else(|| "?".to_owned(), |(_, at)| at.trim().to_owned())
    };
    // How long the references panel took to build its rows, the first time
    // (as the repository opens) and at most, and how many rows of how many
    // references the slowest build made.
    let panel: Vec<(&str, &str)> = said
        .lines()
        .filter_map(|line| line.split_once("refs panel: ").map(|(_, rest)| rest))
        .filter_map(|rest| rest.rsplit_once(" built in "))
        .map(|(what, took)| (what.trim(), took.trim()))
        .collect();
    let slowest = panel
        .iter()
        .filter_map(|(what, took)| Some((*what, millis(took)?)))
        .max_by(|a, b| a.1.total_cmp(&b.1));
    let (most_what, most) = slowest.unwrap_or(("?", 0.0));
    // The details are on screen with the first frame at the earliest, which
    // the window is open after: read ahead of the window, they come before.
    let window = reached("window open");
    let details = reached("first commit details");
    let shown = match (millis(&window), millis(&details)) {
        (Some(open), Some(come)) if open > come => window.clone(),
        _ => details,
    };
    // The toolkit making its window: from the sessions being started to
    // the shell being built inside the new window.
    let toolkit = match (
        millis(&reached("sessions preloading")),
        millis(&reached("shell building")),
    ) {
        (Some(from), Some(to)) => format!("{:.0}ms", to - from),
        _ => "?".to_owned(),
    };
    Ok(format!(
        "{report} toolkit_window={toolkit} window_open={} first_graph_render={} first_commit_details={} refs_panel={} refs_panel_max_ms={most:.1} refs_panel_builds={} refs_panel_slowest_rows={}",
        window,
        reached("first graph render"),
        shown,
        panel.first().map_or("?", |(_, took)| took),
        panel.len(),
        most_what.split_whitespace().next().unwrap_or("?")
    ))
}

/// `path` made absolute with its links resolved, without Windows' verbatim
/// prefix (`\\?\`) where the path reads the same without it: the
/// application keeps a repository's state under the path as it shows it.
fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    let path = std::fs::canonicalize(path)?;
    let plain = path
        .to_str()
        .and_then(|text| text.strip_prefix(r"\\?\"))
        .filter(|rest| rest.get(1..3) == Some(r":\") && rest.len() < 260)
        .map(PathBuf::from);
    Ok(plain.unwrap_or(path))
}

/// A duration as Rust's `Debug` writes it (`3.2ms`, `850µs`, `1.1s`), in
/// milliseconds.
fn millis(text: &str) -> Option<f64> {
    let (number, scale) = [("ms", 1.0), ("µs", 0.001), ("ns", 0.000_001), ("s", 1000.0)]
        .into_iter()
        .find_map(|(unit, scale)| text.strip_suffix(unit).map(|n| (n, scale)))?;
    number.parse::<f64>().ok().map(|n| n * scale)
}

#[cfg(test)]
mod tests {
    use super::millis;

    #[test]
    fn a_debug_duration_is_read_in_milliseconds() {
        assert_eq!(millis("3.5ms"), Some(3.5));
        assert_eq!(millis("850µs"), Some(0.85));
        assert_eq!(millis("1.25s"), Some(1250.0));
        assert_eq!(millis("2000000ns"), Some(2.0));
        assert_eq!(millis("soon"), None);
    }
}
