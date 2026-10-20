//! How the running copy was installed, and putting a checked package in
//! its place the system's way.
//!
//! - **AppImage** (Linux): the image's runtime says which file it is in
//!   `APPIMAGE` and where it mounted it in `APPDIR`; the program has to be
//!   inside that mount, since a program started from one inherits both.
//!   The new image is downloaded beside the old one (`.<name>.download`),
//!   the old one is kept as `<name>.old` (a second link to it, or a copy),
//!   and the new one is renamed over it, which replaces the file in one
//!   step: there is no moment without one. The new copy, once it starts,
//!   deletes the old ([`tidy`]).
//! - **Application bundle** (macOS): the program is at
//!   `<name>.app/Contents/MacOS/`. The package, a tarball of the bundle, is
//!   unpacked beside it with the system's `tar`; a release without one has
//!   its disk image attached read only where nobody sees it (`hdiutil`)
//!   and the bundle copied out of it (`ditto`). The old bundle is renamed
//!   to `<name>.app.old` and the new one put where it was; the new copy
//!   deletes the old. A copy macOS runs from a place of its own (App
//!   Translocation, for a bundle started where it was downloaded) is not
//!   replaced.
//! - **Windows installer**: the program is under the Program Files folders
//!   or the user's `Programs`. The downloaded `.msi` is run (`msiexec /i
//!   … /passive`) and the application closes, so that the installer can
//!   replace it. The system's PowerShell starts the installer and waits
//!   for it, and then starts the program where it was, which is the new
//!   version once the installer has put it there (the old one where the
//!   installer was refused or failed): nobody is left without the
//!   application. It is handed its script encoded (`-EncodedCommand`), so
//!   that no path is taken apart on the way, and runs without a window.
//! - **Package manager** (a program under `/usr`) and **anywhere else** (a
//!   tarball unpacked, the portable zip, a build from source): nothing is
//!   replaced; the window says a new version is out, with its page.
//!
//! Sources: docs.appimage.org "Environment variables" (`APPIMAGE`,
//! `APPDIR`); Apple's "Bundle Programming Guide" (a bundle's layout) and
//! "About App Translocation"; Microsoft's "Msiexec (command-line options)"
//! (`/i`, `/passive`), "KNOWNFOLDERID" (Program Files, the user's Programs
//! folder), "about_PowerShell_exe" (`-EncodedCommand`, `-NoProfile`),
//! "Start-Process" (`-Wait`, `-ArgumentList`) and "about_Quoting_Rules" (a
//! single quote doubled); rename(2) and link(2) (replacing a file in one step,
//! a second name for it); bsdtar(1) (`-x -z -f -C`); on macOS hdiutil(1)
//! (`attach -nobrowse -readonly -noautoopen -mountpoint`, `detach
//! [-force]`), ditto(1) and open(1) (`-n`).

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use super::feed::{Kind, System};

/// How the running copy was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Installation {
    /// An AppImage: this file, which is replaced.
    AppImage(PathBuf),
    /// A macOS application bundle: this folder, which is replaced.
    Bundle(PathBuf),
    /// Installed by the Windows installer, which is run for the next
    /// version: the program.
    Installer(PathBuf),
    /// Installed by the system's package manager: the program.
    Package(PathBuf),
    /// Anything else (a tarball unpacked, the portable zip, a build from
    /// source): the program, when it is known.
    Elsewhere(PathBuf),
}

/// What comes after a package was put in place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Then {
    /// The new version is in place: starting again starts it.
    Restart,
    /// Its installer runs and needs the application closed.
    Quit,
}

/// Whether `path` is inside `folder`, letters of any case on Windows.
fn inside(path: &Path, folder: &Path, system: System) -> bool {
    if folder.as_os_str().is_empty() {
        return false;
    }
    if system != System::Windows {
        return path.starts_with(folder);
    }
    let path = path.to_string_lossy().to_lowercase().replace('/', "\\");
    let folder = folder.to_string_lossy().to_lowercase().replace('/', "\\");
    let folder = folder.trim_end_matches('\\');
    path.strip_prefix(folder)
        .is_some_and(|rest| rest.starts_with('\\'))
}

/// The bundle a program is in: `<name>.app/Contents/MacOS/<program>`.
fn bundle_of(program: &Path) -> Option<PathBuf> {
    let macos = program.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let named = |path: &Path, name: &str| path.file_name().is_some_and(|found| found == name);
    let app = bundle
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
    (named(macos, "MacOS") && named(contents, "Contents") && app).then(|| bundle.to_path_buf())
}

impl Installation {
    /// How this copy was installed, from where its program is and what its
    /// surroundings say.
    pub fn detect() -> Self {
        let program = std::env::current_exe().unwrap_or_default();
        let var = |name: &str| std::env::var_os(name).map(PathBuf::from);
        let image = var("APPIMAGE").filter(|image| image.is_file());
        let mut programs: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"]
            .into_iter()
            .filter_map(var)
            .collect();
        if let Some(local) = var("LOCALAPPDATA") {
            programs.push(local.join("Programs"));
        }
        Self::of(
            super::Target::this().map(|target| target.system),
            &program,
            image.as_deref(),
            var("APPDIR").as_deref(),
            &programs,
        )
    }

    /// How a copy whose program is at `program` was installed, on `system`,
    /// with the AppImage and the folder it mounted that its surroundings
    /// name, and the folders programs are installed in on Windows.
    pub fn of(
        system: Option<System>,
        program: &Path,
        image: Option<&Path>,
        mounted: Option<&Path>,
        programs: &[PathBuf],
    ) -> Self {
        let program_buf = program.to_path_buf();
        match system {
            // Absolute as Linux has it, from the root, whatever system
            // reads it (a test of Linux's case on Windows).
            Some(System::Linux) => match (image, mounted) {
                (Some(image), Some(mounted))
                    if image.has_root() && inside(program, mounted, System::Linux) =>
                {
                    Self::AppImage(image.to_path_buf())
                }
                _ if program.starts_with("/usr") => Self::Package(program_buf),
                _ => Self::Elsewhere(program_buf),
            },
            Some(System::Macos) => bundle_of(program)
                .map(Self::Bundle)
                .unwrap_or(Self::Elsewhere(program_buf)),
            Some(System::Windows) => {
                if programs
                    .iter()
                    .any(|folder| inside(program, folder, System::Windows))
                {
                    Self::Installer(program_buf)
                } else {
                    Self::Elsewhere(program_buf)
                }
            }
            None => Self::Elsewhere(program_buf),
        }
    }

    /// What the commands call it.
    pub fn word(&self) -> &'static str {
        match self {
            Self::AppImage(_) => "appimage",
            Self::Bundle(_) => "bundle",
            Self::Installer(_) => "installer",
            Self::Package(_) => "package",
            Self::Elsewhere(_) => "elsewhere",
        }
    }

    /// The file, folder or program it is.
    pub fn path(&self) -> &Path {
        match self {
            Self::AppImage(path)
            | Self::Bundle(path)
            | Self::Installer(path)
            | Self::Package(path)
            | Self::Elsewhere(path) => path,
        }
    }

    /// The kinds of package that update it, the one it takes first first:
    /// none when it does not update itself. A bundle takes the tarball made
    /// for the updater, else the disk image people install from.
    pub fn kinds(&self) -> &'static [Kind] {
        match self {
            Self::AppImage(_) => &[Kind::Appimage],
            Self::Bundle(path) if !translocated(path) => &[Kind::App, Kind::Dmg],
            Self::Installer(_) => &[Kind::Msi],
            Self::Bundle(_) | Self::Package(_) | Self::Elsewhere(_) => &[],
        }
    }

    /// The kind of package that updates it first, when it updates itself.
    pub fn kind(&self) -> Option<Kind> {
        self.kinds().first().copied()
    }

    /// Why it does not update itself, when it does not.
    pub fn why_not(&self) -> Option<String> {
        match self {
            Self::AppImage(_) | Self::Installer(_) => None,
            Self::Bundle(path) if !translocated(path) => None,
            Self::Bundle(_) => Some(say!("update-translocated").to_owned()),
            Self::Package(path) => Some(say!(
                "update-package-manager",
                path = path.display().to_string()
            )),
            Self::Elsewhere(path) if path.as_os_str().is_empty() => {
                Some(say!("update-nowhere").to_owned())
            }
            Self::Elsewhere(path) => {
                Some(say!("update-elsewhere", path = path.display().to_string()))
            }
        }
    }
}

/// Whether macOS runs a bundle from a place of its own (App
/// Translocation), read only and gone when it quits.
fn translocated(bundle: &Path) -> bool {
    bundle
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation")
}

/// A name beside `path`: its file name with `before` and `after` added.
fn beside(path: &Path, before: &str, after: &str) -> Option<PathBuf> {
    let name = path.file_name()?.to_string_lossy();
    Some(path.with_file_name(format!("{before}{name}{after}")))
}

/// What installing one package does, decided before it is downloaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Plan {
    /// The image is replaced with the one downloaded beside it, the old one
    /// kept until the new one starts.
    AppImage {
        image: PathBuf,
        staged: PathBuf,
        old: PathBuf,
    },
    /// The bundle is replaced with the one unpacked beside it from the
    /// download (a tarball, or a disk image it is copied from), the old one
    /// kept until the new one starts.
    Bundle {
        bundle: PathBuf,
        from: Kind,
        download: PathBuf,
        unpacked: PathBuf,
        old: PathBuf,
    },
    /// The installer downloaded is run, and the program it replaces
    /// started once it is done.
    Installer { download: PathBuf, program: PathBuf },
}

impl Plan {
    /// How to install the package of `version`, of `kind`, over
    /// `installation`, with downloads that are not put beside the copy
    /// going to `downloads`.
    pub(crate) fn new(
        installation: &Installation,
        kind: Kind,
        version: &semver::Version,
        downloads: Option<&Path>,
    ) -> Result<Self, String> {
        let downloaded = |name: String| -> Result<PathBuf, String> {
            downloads
                .map(|folder| folder.join(name))
                .ok_or_else(|| say!("update-no-download-folder").to_owned())
        };
        let no_name = || say!("update-no-name").to_owned();
        match installation {
            Installation::AppImage(image) => Ok(Self::AppImage {
                image: image.clone(),
                staged: beside(image, ".", ".download").ok_or_else(no_name)?,
                old: beside(image, "", ".old").ok_or_else(no_name)?,
            }),
            Installation::Bundle(bundle) if !translocated(bundle) => Ok(Self::Bundle {
                bundle: bundle.clone(),
                from: kind,
                download: downloaded(match kind {
                    Kind::Dmg => format!("BitBraid-{version}.dmg"),
                    _ => format!("BitBraid-{version}.app.tar.gz"),
                })?,
                unpacked: beside(bundle, ".", ".update").ok_or_else(no_name)?,
                old: beside(bundle, "", ".old").ok_or_else(no_name)?,
            }),
            Installation::Installer(program) => Ok(Self::Installer {
                download: downloaded(format!("BitBraid-{version}.msi"))?,
                program: program.clone(),
            }),
            other => Err(other
                .why_not()
                .unwrap_or_else(|| say!("update-not-itself").to_owned())),
        }
    }

    /// Where the package is downloaded to.
    pub(crate) fn download_path(&self) -> &Path {
        match self {
            Self::AppImage { staged, .. } => staged,
            Self::Bundle { download, .. } | Self::Installer { download, .. } => download,
        }
    }

    /// The file the package is downloaded into, made anew.
    pub(crate) fn open_download(&self) -> Result<fs::File, String> {
        let path = self.download_path();
        let folder = path.parent().unwrap_or(Path::new("."));
        if !matches!(self, Self::AppImage { .. }) {
            fs::create_dir_all(folder).map_err(|err| {
                say!(
                    "update-folder-not-made",
                    folder = folder.display().to_string(),
                    error = err.to_string()
                )
            })?;
        }
        fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)
            .map_err(|err| match self {
                Self::AppImage { .. } => say!(
                    "update-appimage-folder-unwritable",
                    folder = folder.display().to_string(),
                    error = err.to_string()
                ),
                _ => say!(
                    "update-not-written",
                    path = path.display().to_string(),
                    error = err.to_string()
                ),
            })
    }

    /// Take away what was downloaded or unpacked, after something failed.
    pub(crate) fn discard(&self) {
        let _ = fs::remove_file(self.download_path());
        if let Self::Bundle { unpacked, .. } = self {
            let _ = fs::remove_dir_all(unpacked);
        }
    }

    /// Put the checked package in place, or start its installer.
    pub(crate) fn put_in_place(&self) -> Result<Then, String> {
        match self {
            Self::AppImage { image, staged, old } => replace_image(image, staged, old),
            Self::Bundle {
                bundle,
                from,
                download,
                unpacked,
                old,
            } => replace_bundle(bundle, *from, download, unpacked, old),
            Self::Installer { download, program } => run_installer(download, program),
        }
    }
}

/// Give `staged` the permissions of `image`, and the right to be run.
fn runnable_like(staged: &Path, image: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(image)
            .map(|meta| meta.permissions().mode())
            .unwrap_or(0o755);
        fs::set_permissions(staged, fs::Permissions::from_mode((mode | 0o500) & 0o7777))
    }
    #[cfg(not(unix))]
    {
        let _ = (staged, image);
        Ok(())
    }
}

fn replace_image(image: &Path, staged: &Path, old: &Path) -> Result<Then, String> {
    runnable_like(staged, image)
        .map_err(|err| say!("update-appimage-not-runnable", error = err.to_string()))?;
    // An old copy of an update before, which did not start: this one goes
    // in its place.
    match fs::remove_file(old) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(say!(
                "update-in-the-way",
                path = old.display().to_string(),
                error = err.to_string()
            ));
        }
    }
    fs::hard_link(image, old)
        .or_else(|_| fs::copy(image, old).map(|_| ()))
        .map_err(|err| {
            say!(
                "update-appimage-not-kept",
                path = old.display().to_string(),
                error = err.to_string()
            )
        })?;
    if let Err(err) = fs::rename(staged, image) {
        let _ = fs::remove_file(old);
        return Err(say!(
            "update-appimage-not-replaced",
            error = err.to_string()
        ));
    }
    Ok(Then::Restart)
}

/// Whether a folder is an application bundle.
fn is_bundle(folder: &Path) -> bool {
    folder.join("Contents").join("Info.plist").is_file()
        || folder.join("Contents").join("MacOS").is_dir()
}

/// The one application bundle at the top of a folder.
fn only_bundle(folder: &Path) -> Option<PathBuf> {
    let mut found = None;
    for entry in fs::read_dir(folder).ok()?.flatten().take(64) {
        let path = entry.path();
        let app = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("app"));
        if app && path.join("Contents").join("MacOS").is_dir() {
            if found.is_some() {
                return None;
            }
            found = Some(path);
        }
    }
    found
}

/// The system's `tar`.
fn tar() -> PathBuf {
    let system = Path::new("/usr/bin/tar");
    if system.is_file() {
        system.to_path_buf()
    } else {
        PathBuf::from("tar")
    }
}

/// Run a program of the system to its end; what it said on its error
/// output when it failed.
fn run_to_end(program: &Path, args: &[&std::ffi::OsStr], what: &str) -> Result<(), String> {
    let ran = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|err| {
            say!(
                "update-program-not-started",
                program = program.display().to_string(),
                what,
                error = err.to_string()
            )
        })?;
    if ran.status.success() {
        return Ok(());
    }
    let said = String::from_utf8_lossy(&ran.stderr);
    let said: String = said.trim().chars().take(300).collect();
    Err(say!("update-program-failed", what, said))
}

/// The bundle in a disk image, copied into `into`: the image attached read
/// only where nobody sees it, the bundle copied with its attributes, the
/// image detached again (macOS's `hdiutil` and `ditto`).
fn copy_from_image(image: &Path, into: &Path) -> Result<(), String> {
    let mount = into.join("image");
    fs::create_dir(&mount).map_err(|err| {
        say!(
            "update-not-made",
            path = mount.display().to_string(),
            error = err.to_string()
        )
    })?;
    let hdiutil = Path::new("/usr/bin/hdiutil");
    run_to_end(
        hdiutil,
        &[
            "attach".as_ref(),
            "-nobrowse".as_ref(),
            "-readonly".as_ref(),
            "-noautoopen".as_ref(),
            "-mountpoint".as_ref(),
            mount.as_os_str(),
            image.as_os_str(),
        ],
        say!("update-attaching-image"),
    )?;
    let copied = only_bundle(&mount)
        .ok_or_else(|| say!("update-image-no-bundle").to_owned())
        .and_then(|found| {
            let name = found.file_name().unwrap_or_default();
            run_to_end(
                Path::new("/usr/bin/ditto"),
                &[found.as_os_str(), into.join(name).as_os_str()],
                say!("update-copying-bundle"),
            )
        });
    let detach = |force: bool| {
        let mut args: Vec<&std::ffi::OsStr> = vec!["detach".as_ref(), mount.as_os_str()];
        if force {
            args.push("-force".as_ref());
        }
        run_to_end(hdiutil, &args, say!("update-detaching-image"))
    };
    if detach(false).is_err() {
        let _ = detach(true);
    }
    copied
}

fn replace_bundle(
    bundle: &Path,
    from: Kind,
    download: &Path,
    unpacked: &Path,
    old: &Path,
) -> Result<Then, String> {
    let parent = bundle.parent().unwrap_or(Path::new("/"));
    let _ = fs::remove_dir_all(unpacked);
    fs::create_dir(unpacked).map_err(|err| {
        say!(
            "update-cannot-write-in",
            folder = parent.display().to_string(),
            error = err.to_string()
        )
    })?;
    if from == Kind::Dmg {
        copy_from_image(download, unpacked)?;
    } else {
        run_to_end(
            &tar(),
            &[
                "-xzf".as_ref(),
                download.as_os_str(),
                "-C".as_ref(),
                unpacked.as_os_str(),
            ],
            say!("update-unpacking-bundle"),
        )?;
    }
    let fresh = only_bundle(unpacked).ok_or_else(|| say!("update-package-no-bundle").to_owned())?;
    if old.is_dir() && is_bundle(old) {
        fs::remove_dir_all(old).map_err(|err| {
            say!(
                "update-in-the-way",
                path = old.display().to_string(),
                error = err.to_string()
            )
        })?;
    }
    fs::rename(bundle, old).map_err(|err| {
        say!(
            "update-bundle-not-set-aside",
            path = old.display().to_string(),
            error = err.to_string()
        )
    })?;
    if let Err(err) = fs::rename(&fresh, bundle) {
        let _ = fs::rename(old, bundle);
        return Err(say!("update-bundle-not-replaced", error = err.to_string()));
    }
    let _ = fs::remove_dir_all(unpacked);
    let _ = fs::remove_file(download);
    Ok(Then::Restart)
}

/// The Windows installer's program.
fn msiexec() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("msiexec.exe"))
        .filter(|program| program.is_file())
        .unwrap_or_else(|| PathBuf::from("msiexec"))
}

/// The system's PowerShell.
fn powershell() -> PathBuf {
    std::env::var_os("SystemRoot")
        .map(|root| {
            PathBuf::from(root)
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|program| program.is_file())
        .unwrap_or_else(|| PathBuf::from("powershell"))
}

/// A path as a literal of PowerShell: in single quotes, a quote in it
/// doubled.
fn powershell_literal(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "''"))
}

/// The script that runs the installer at `msi` with `msiexec`, waits for
/// it to end, and then starts `program`, whatever came of it: the new
/// version where it was installed, the old one where it was not.
fn installer_script(msiexec: &Path, msi: &Path, program: &Path) -> String {
    // Start-Process hands its arguments on joined by spaces: the package's
    // path, which may have them, goes in double quotes, which no path on
    // Windows holds.
    let package = format!("\"{}\"", msi.display()).replace('\'', "''");
    format!(
        "Start-Process -FilePath {} -ArgumentList @('/i', '{package}', '/passive') -Wait\n\
         Start-Process -FilePath {}\n",
        powershell_literal(msiexec),
        powershell_literal(program),
    )
}

/// A script as PowerShell's `-EncodedCommand` takes it: its UTF-16 in
/// base64.
fn encoded_command(script: &str) -> String {
    use base64::Engine as _;
    let bytes: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn run_installer(download: &Path, program: &Path) -> Result<Then, String> {
    let script = installer_script(&msiexec(), download, program);
    let mut command = Command::new(powershell());
    command
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-EncodedCommand",
        ])
        .arg(encoded_command(&script))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    bitbraid_core::process::no_window(&mut command)
        .spawn()
        .map_err(|err| say!("update-installer-not-started", error = err.to_string()))?;
    Ok(Then::Quit)
}

/// Start the copy that was put in place, to take over from this one.
pub fn start_again(installation: &Installation) -> Result<(), String> {
    let mut command = match installation {
        Installation::AppImage(image) => Command::new(image),
        Installation::Bundle(bundle) => {
            let mut open = Command::new("/usr/bin/open");
            open.arg("-n").arg(bundle);
            open
        }
        _ => return Err(say!("update-not-started-again").to_owned()),
    };
    bitbraid_core::process::with_user_environment(&mut command);
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| say!("update-new-not-started", error = err.to_string()))?;
    // Collected when it ends, should this process still be there.
    let _ = std::thread::Builder::new()
        .name("bitbraid-started-again".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

/// Delete what an update left: the old copy, once the new one runs (this
/// is called as it starts), and what was downloaded or unpacked and not
/// used. What cannot be deleted is left.
pub fn tidy(installation: &Installation, downloads: Option<&Path>) {
    match installation {
        Installation::AppImage(image) => {
            for leftover in [beside(image, "", ".old"), beside(image, ".", ".download")]
                .into_iter()
                .flatten()
            {
                if leftover.is_file() {
                    let _ = fs::remove_file(leftover);
                }
            }
        }
        Installation::Bundle(bundle) => {
            if let Some(old) = beside(bundle, "", ".old")
                && old.is_dir()
                && is_bundle(&old)
            {
                let _ = fs::remove_dir_all(old);
            }
            if let Some(unpacked) = beside(bundle, ".", ".update")
                && unpacked.is_dir()
            {
                let _ = fs::remove_dir_all(unpacked);
            }
        }
        Installation::Installer(_) | Installation::Package(_) | Installation::Elsewhere(_) => {}
    }
    if let Some(folder) = downloads
        && folder.is_dir()
    {
        let _ = fs::remove_dir_all(folder);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_installer_is_waited_for_and_the_program_started_after_it() {
        let script = installer_script(
            Path::new(r"C:\Windows\System32\msiexec.exe"),
            Path::new(r"C:\Users\O'Brien\AppData\Local\bitbraid\cache\updates\BitBraid-1.2.0.msi"),
            Path::new(r"C:\Program Files\BitBraid\bitbraid.exe"),
        );
        assert_eq!(
            script,
            "Start-Process -FilePath 'C:\\Windows\\System32\\msiexec.exe' \
             -ArgumentList @('/i', '\"C:\\Users\\O''Brien\\AppData\\Local\\bitbraid\\cache\\updates\\BitBraid-1.2.0.msi\"', '/passive') \
             -Wait\n\
             Start-Process -FilePath 'C:\\Program Files\\BitBraid\\bitbraid.exe'\n"
        );
        // UTF-16, little end first, in base64: "$i" is 24 00 69 00.
        assert_eq!(encoded_command("$i"), "JABpAA==");
        assert_eq!(encoded_command(""), "");
    }

    #[test]
    fn an_appimage_is_its_file_when_the_program_is_inside_its_mount() {
        let image = Path::new("/home/u/Apps/BitBraid.AppImage");
        let mounted = Path::new("/tmp/.mount_BitBraXYZ");
        let inside_it = Path::new("/tmp/.mount_BitBraXYZ/usr/bin/bitbraid");
        assert_eq!(
            Installation::of(
                Some(System::Linux),
                inside_it,
                Some(image),
                Some(mounted),
                &[]
            ),
            Installation::AppImage(image.to_path_buf())
        );
        // A program started from inside an AppImage inherits its variables:
        // it is not the image.
        let other = Path::new("/home/u/src/bitbraid/target/debug/bitbraid");
        assert_eq!(
            Installation::of(Some(System::Linux), other, Some(image), Some(mounted), &[]),
            Installation::Elsewhere(other.to_path_buf())
        );
        let packaged = Path::new("/usr/bin/bitbraid");
        assert_eq!(
            Installation::of(Some(System::Linux), packaged, None, None, &[]),
            Installation::Package(packaged.to_path_buf())
        );
        assert_eq!(
            Installation::of(
                Some(System::Linux),
                Path::new("/usrx/bitbraid"),
                None,
                None,
                &[]
            ),
            Installation::Elsewhere(PathBuf::from("/usrx/bitbraid"))
        );
    }

    #[test]
    fn a_bundle_is_found_from_its_program_and_a_translocated_one_is_not_replaced() {
        let program = Path::new("/Applications/BitBraid.app/Contents/MacOS/bitbraid");
        let found = Installation::of(Some(System::Macos), program, None, None, &[]);
        assert_eq!(
            found,
            Installation::Bundle(PathBuf::from("/Applications/BitBraid.app"))
        );
        assert_eq!(found.kind(), Some(Kind::App));
        assert_eq!(found.why_not(), None);
        let moved = Path::new(
            "/private/var/folders/x/AppTranslocation/ABC/d/BitBraid.app/Contents/MacOS/bitbraid",
        );
        let translocated = Installation::of(Some(System::Macos), moved, None, None, &[]);
        assert_eq!(translocated.kind(), None);
        assert!(translocated.why_not().unwrap().contains("Applications"));
        let loose = Path::new("/Users/u/bin/bitbraid");
        assert_eq!(
            Installation::of(Some(System::Macos), loose, None, None, &[]),
            Installation::Elsewhere(loose.to_path_buf())
        );
    }

    #[test]
    fn a_program_under_program_files_was_installed_by_the_installer() {
        let programs = [
            PathBuf::from(r"C:\Program Files"),
            PathBuf::from(r"C:\Users\u\AppData\Local\Programs"),
        ];
        let installed = Path::new(r"c:\program files\BitBraid\bitbraid.exe");
        assert_eq!(
            Installation::of(Some(System::Windows), installed, None, None, &programs).kind(),
            Some(Kind::Msi)
        );
        let mine = Path::new(r"C:\Users\u\AppData\Local\Programs\BitBraid\bitbraid.exe");
        assert_eq!(
            Installation::of(Some(System::Windows), mine, None, None, &programs).word(),
            "installer"
        );
        let portable = Path::new(r"D:\tools\BitBraid\bitbraid.exe");
        let found = Installation::of(Some(System::Windows), portable, None, None, &programs);
        assert_eq!(found.kind(), None);
        assert!(found.why_not().unwrap().contains("download"));
        let near = Path::new(r"C:\Program Files Extra\bitbraid.exe");
        assert_eq!(
            Installation::of(Some(System::Windows), near, None, None, &programs).word(),
            "elsewhere"
        );
    }

    #[test]
    fn an_unknown_system_updates_nothing() {
        let found = Installation::of(None, Path::new("/opt/bitbraid"), None, None, &[]);
        assert_eq!(found.kind(), None);
        assert!(found.why_not().is_some());
        assert!(
            Installation::Elsewhere(PathBuf::new())
                .why_not()
                .unwrap()
                .contains("cannot tell")
        );
    }

    #[test]
    fn a_plan_puts_its_files_beside_the_copy_or_in_the_downloads() {
        let version = semver::Version::new(1, 2, 0);
        let image = Installation::AppImage(PathBuf::from("/a/BitBraid.AppImage"));
        assert_eq!(
            Plan::new(&image, Kind::Appimage, &version, None).unwrap(),
            Plan::AppImage {
                image: PathBuf::from("/a/BitBraid.AppImage"),
                staged: PathBuf::from("/a/.BitBraid.AppImage.download"),
                old: PathBuf::from("/a/BitBraid.AppImage.old"),
            }
        );
        let bundle = Installation::Bundle(PathBuf::from("/Applications/BitBraid.app"));
        assert_eq!(bundle.kinds(), [Kind::App, Kind::Dmg]);
        assert!(Plan::new(&bundle, Kind::App, &version, None).is_err());
        let downloads = Some(Path::new("/c/updates"));
        assert_eq!(
            Plan::new(&bundle, Kind::App, &version, downloads)
                .unwrap()
                .download_path(),
            Path::new("/c/updates/BitBraid-1.2.0.app.tar.gz")
        );
        // The disk image, where the release has no tarball of the bundle.
        assert_eq!(
            Plan::new(&bundle, Kind::Dmg, &version, downloads).unwrap(),
            Plan::Bundle {
                bundle: PathBuf::from("/Applications/BitBraid.app"),
                from: Kind::Dmg,
                download: PathBuf::from("/c/updates/BitBraid-1.2.0.dmg"),
                unpacked: PathBuf::from("/Applications/.BitBraid.app.update"),
                old: PathBuf::from("/Applications/BitBraid.app.old"),
            }
        );
        let installer = Installation::Installer(PathBuf::from("/p/bitbraid.exe"));
        assert_eq!(
            Plan::new(&installer, Kind::Msi, &version, downloads).unwrap(),
            Plan::Installer {
                download: PathBuf::from("/c/updates/BitBraid-1.2.0.msi"),
                program: PathBuf::from("/p/bitbraid.exe"),
            }
        );
        let package = Installation::Package(PathBuf::from("/usr/bin/bitbraid"));
        assert!(package.kinds().is_empty());
        assert!(
            Plan::new(&package, Kind::Deb, &version, None)
                .unwrap_err()
                .contains("package manager")
        );
    }
}
