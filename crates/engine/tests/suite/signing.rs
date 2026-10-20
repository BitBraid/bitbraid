//! Signing commands against what git itself says on the same repository:
//! `%G?` and `git verify-commit` for commits, `git verify-tag` and
//! `git for-each-ref` for tags, `git config` on a twin copy for settings.
//!
//! Commands run git with the developer's own environment and configuration,
//! so everything about signing is pinned in the scratch copy's local
//! configuration, and no key or program of the developer's is ever asked:
//! keys are made beside the copy, gpg is a script that knows one key ring
//! there, and gpgsm one that fails. Nothing is written elsewhere.
//!
//! On Windows the programs are those of Git for Windows (its gpg, gpgsm
//! and ssh-keygen, which are MSYS2's), the scripts run through its sh
//! (`bitbraid_testkit::script`, a batch file beside each), a key ring is
//! named to gpg as MSYS2 writes a path, through a junction short enough
//! for its agent's sockets, and it is not made private: Windows has no
//! such mode, and gpg only warns.
#![allow(clippy::print_stdout)]

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, scratch_root};
use serde_json::{Value, json};

const ADA: &str = "Ada Fixture <ada@fixture.invalid>";

/// Whether a program is installed. Looked up, never run: ssh-keygen without
/// arguments would start making a key.
fn on_path(program: &str) -> bool {
    let exe = format!("{program}.exe");
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .any(|dir| dir.join(program).is_file() || (cfg!(windows) && dir.join(&exe).is_file()))
    })
}

/// A test that needs a program that is not there says so and passes.
pub(crate) fn missing(program: &str) -> bool {
    if on_path(program) {
        return false;
    }
    println!("skipped: {program} is not installed");
    true
}

fn sh_quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

/// The home a ring is used by: on macOS a short link in `/tmp` to it.
/// gpg-agent's sockets are made in the home there (Linux keeps them under
/// `/run/user`), and a socket's path is at most 104 bytes; the scratch
/// folders of the tests are longer than that. The same home gives the
/// same link, so that its agent is found again to be ended. On Windows a
/// junction ([`junction`]), as gpg is told it ([`for_gpg`]).
fn agent_home(home: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        let key = bitbraid_testkit::fnv1a64(home.as_os_str().as_encoded_bytes());
        let link = PathBuf::from(format!("/tmp/gsg-{key:016x}"));
        if fs::read_link(&link).ok().as_deref() != Some(home) {
            let _ = fs::remove_file(&link);
            std::os::unix::fs::symlink(home, &link).unwrap();
        }
        link
    }
    #[cfg(windows)]
    {
        for_gpg(&junction(home, "gsg"))
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    home.to_path_buf()
}

/// Where [`junction`] puts the short name of `home`.
#[cfg(windows)]
pub(crate) fn junction_name(home: &Path, prefix: &str) -> PathBuf {
    let key = bitbraid_testkit::fnv1a64(home.as_os_str().as_encoded_bytes());
    std::env::temp_dir().join(format!("{prefix}-{key:016x}"))
}

/// A short name for a gnupg home on Windows: a junction in the temporary
/// folder, which needs no right of its own. MSYS2's gpg-agent makes its
/// sockets in the home, and their paths may be at most 108 bytes; the
/// scratch folders are longer. The same home gives the same name.
#[cfg(windows)]
pub(crate) fn junction(home: &Path, prefix: &str) -> PathBuf {
    let link = junction_name(home, prefix);
    let target = |path: &Path| fs::canonicalize(path).ok();
    if target(&link).is_none() || target(&link) != target(home) {
        let _ = fs::remove_dir(&link);
        let made = Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(made.success(), "no junction at {}", link.display());
    }
    link
}

/// A folder as gpg and gpgsm are told it. Git for Windows' are MSYS2's,
/// which take `D:\keys` for a folder inside the current one: they are
/// given `/d/keys`.
pub(crate) fn for_gpg(path: &Path) -> PathBuf {
    if !cfg!(windows) {
        return path.to_path_buf();
    }
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text);
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(drive), Some(':')) if drive.is_ascii_alphabetic() => {
            PathBuf::from(format!("/{}{}", drive.to_ascii_lowercase(), chars.as_str()))
        }
        _ => PathBuf::from(text),
    }
}

/// Write the sh script `body` at `path` and return what runs it: the
/// script on Unix, a batch file beside it on Windows.
pub(crate) fn script(path: &Path, body: &str) -> PathBuf {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        panic!("no place for a script: {}", path.display());
    };
    bitbraid_testkit::script(dir, &name.to_string_lossy(), body).unwrap()
}

/// What runs the script written at `path` (see [`script`]).
pub(crate) fn runnable(path: &Path) -> PathBuf {
    if cfg!(windows) {
        let mut runs = path.as_os_str().to_owned();
        runs.push(".cmd");
        PathBuf::from(runs)
    } else {
        path.to_path_buf()
    }
}

/// A folder for keys that only its owner may enter, where the system has
/// such a thing (gpg warns of one others may).
pub(crate) fn private(dir: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Write an object as it is given, the way a commit or a tag is forged.
fn store(git: &Git, kind: &str, content: &str, literally: bool) -> Option<String> {
    let mut command = git.command();
    command.args(["hash-object", "-t", kind, "-w", "--stdin"]);
    if literally {
        command.arg("--literally");
    }
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(content.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

pub(crate) struct Harness {
    pub(crate) copy: Scratch,
    /// Keys, key rings and the scripts that stand in for gpg: beside the
    /// copy, and gone with it.
    pub(crate) keys: PathBuf,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    pub(crate) fn new() -> Self {
        let copy = Scratch::of("branches-merges").unwrap();
        let keys = copy.path().with_extension("keys");
        let _ = fs::remove_dir_all(&keys);
        fs::create_dir_all(&keys).unwrap();
        let mut h = Self {
            copy,
            keys,
            registry: builtin_registry(),
            ctx: CommandContext::default(),
        };
        // Before anything can sign or verify: the programs that would.
        let nobody = h.ring("nobody");
        h.use_gpg(&nobody);
        script(&h.keys.join("no-gpgsm.sh"), "#!/bin/sh\nexit 2\n");
        for (key, value) in [
            (
                "gpg.x509.program",
                runnable(&h.keys.join("no-gpgsm.sh")).to_str().unwrap(),
            ),
            ("gpg.ssh.program", "ssh-keygen"),
            ("gpg.format", "openpgp"),
            ("gpg.minTrustLevel", "undefined"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
        ] {
            h.set(key, value);
        }
        h.registry
            .run("repo.open", &mut h.ctx, &json!({ "path": h.copy.path() }))
            .expect("open scratch");
        h
    }
    pub(crate) fn git(&self) -> Git {
        self.copy.git()
    }
    pub(crate) fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    pub(crate) fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|e| e.to_string())
    }
    pub(crate) fn set(&self, key: &str, value: &str) {
        self.git().run(["config", "--local", key, value]).unwrap();
    }
    fn unset(&self, key: &str) {
        let _ = self.git().run(["config", "--local", "--unset-all", key]);
    }
    /// A setting as the commands see it: with the developer's configuration
    /// behind the copy's. Some cases need it to be absent everywhere.
    fn seen(&self, key: &str) -> Option<String> {
        let out = Command::new("git")
            .current_dir(self.copy.path())
            .args(["config", "--get", key])
            .stdin(Stdio::null())
            .output()
            .ok()?;
        let value = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        (out.status.success() && !value.is_empty()).then_some(value)
    }
    pub(crate) fn head(&self) -> String {
        self.git().rev_parse("HEAD").unwrap()
    }
    fn refs(&self) -> Vec<String> {
        self.git().lines(["for-each-ref"]).unwrap()
    }
    fn local_config(&self) -> Vec<String> {
        self.git().lines(["config", "--local", "--list"]).unwrap()
    }

    /// A key ring of its own, and the script that is gpg with that ring.
    fn ring(&self, name: &str) -> PathBuf {
        let home = self.keys.join(format!("gnupg-{name}"));
        fs::create_dir_all(&home).unwrap();
        private(&home);
        script(
            &self.keys.join(format!("gpg-{name}.sh")),
            &format!(
                "#!/bin/sh\n# gpg with a key ring of its own, and no window to ask in.\nunset DISPLAY WAYLAND_DISPLAY\nGNUPGHOME={}\nexport GNUPGHOME\nexec gpg \"$@\"\n",
                sh_quoted(&agent_home(&home))
            ),
        )
    }
    fn use_gpg(&self, wrapper: &Path) {
        self.set("gpg.program", wrapper.to_str().unwrap());
        self.set("gpg.openpgp.program", wrapper.to_str().unwrap());
    }
    pub(crate) fn gpg(&self, wrapper: &Path, args: &[&str]) -> std::process::Output {
        // A script that was just written can be "busy" for a moment: a
        // child another test's thread forked meanwhile still holds it open
        // for writing until it becomes its own program.
        let mut tries = 0;
        loop {
            match Command::new(wrapper)
                .args(args)
                .stdin(Stdio::null())
                .output()
            {
                Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 200 => {
                    tries += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => return other.unwrap(),
            }
        }
    }
    /// Make a signing key without a passphrase in a ring. False, with a
    /// line saying why, where gpg cannot make one.
    fn generate(&self, wrapper: &Path, user: &str) -> bool {
        let out = self.gpg(
            wrapper,
            &[
                "--batch",
                "--pinentry-mode",
                "loopback",
                "--passphrase",
                "",
                "--quick-generate-key",
                user,
                "ed25519",
                "sign",
                "never",
            ],
        );
        if !out.status.success() {
            println!(
                "skipped: gpg could not make a key here: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        out.status.success()
    }
    fn fingerprint(&self, wrapper: &Path) -> String {
        let out = self.gpg(wrapper, &["--batch", "--with-colons", "--list-secret-keys"]);
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .find_map(|line| line.strip_prefix("fpr:::::::::"))
            .map(|rest| rest.trim_end_matches(':').to_owned())
            .unwrap()
    }
    /// Copy the public keys of one ring into another, without any trust.
    fn publish(&self, from: &Path, to: &Path) {
        let exported = self.gpg(from, &["--batch", "--armor", "--export"]);
        let file = self.keys.join("exported.asc");
        fs::write(&file, &exported.stdout).unwrap();
        let out = self.gpg(to, &["--batch", "--import", file.to_str().unwrap()]);
        assert!(out.status.success());
    }

    /// A certificate store of its own (a gnupg home, like a key ring) and
    /// the script that is gpgsm with that store. It asks nobody: no window,
    /// no pinentry program at all, and no directory manager, which would
    /// go to the network for revocation lists.
    fn store_x509(&self, name: &str) -> (PathBuf, PathBuf) {
        // Named like the rings, so that its agent is stopped with theirs.
        let home = self.keys.join(format!("gnupg-x509-{name}"));
        fs::create_dir_all(&home).unwrap();
        private(&home);
        fs::write(
            home.join("gpgsm.conf"),
            "disable-crl-checks\ndisable-dirmngr\n",
        )
        .unwrap();
        fs::write(
            home.join("gpg-agent.conf"),
            "pinentry-program /bin/false\nno-allow-mark-trusted\n",
        )
        .unwrap();
        let wrapper = script(
            &self.keys.join(format!("gpgsm-{name}.sh")),
            &format!(
                "#!/bin/sh\n# gpgsm with a store of its own, and no window to ask in.\nunset DISPLAY WAYLAND_DISPLAY\nGNUPGHOME={}\nexport GNUPGHOME\nexec gpgsm \"$@\"\n",
                sh_quoted(&agent_home(&home))
            ),
        );
        (wrapper, home)
    }
    /// Make a key without a passphrase and a certificate for it that it
    /// signs itself, in a store. Returns the certificate's fingerprint;
    /// nothing, with a line saying why, where gpgsm cannot make one.
    fn certificate(&self, wrapper: &Path, name: &str) -> Option<String> {
        let params = self.keys.join("certificate-params");
        fs::write(
            &params,
            format!(
                "Key-Type: RSA\nKey-Length: 2048\nKey-Usage: sign\nSerial: random\n\
                 Name-DN: CN={name},O=BitBraid Test\nName-Email: ada@fixture.invalid\n"
            ),
        )
        .unwrap();
        // The passphrase is read from standard input, which is empty.
        let made = self.gpg(
            wrapper,
            &[
                "--batch",
                "--armor",
                "--pinentry-mode",
                "loopback",
                "--passphrase-fd",
                "0",
                "--generate-key",
                params.to_str().unwrap(),
            ],
        );
        if !made.status.success() {
            println!(
                "skipped: gpgsm could not make a certificate here: {}",
                String::from_utf8_lossy(&made.stderr).trim()
            );
            return None;
        }
        let file = self.keys.join("certificate.pem");
        fs::write(&file, &made.stdout).unwrap();
        let out = self.gpg(wrapper, &["--batch", "--import", file.to_str().unwrap()]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let listed = self.gpg(wrapper, &["--batch", "--with-colons", "--list-secret-keys"]);
        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .find_map(|line| line.strip_prefix("fpr:::::::::"))
            .and_then(|rest| rest.split(':').next())
            .map(str::to_owned)
    }
    /// Copy the certificates of one store into another.
    fn publish_x509(&self, from: &Path, to: &Path) {
        let exported = self.gpg(from, &["--batch", "--armor", "--export"]);
        let file = self.keys.join("exported.pem");
        fs::write(&file, &exported.stdout).unwrap();
        let out = self.gpg(to, &["--batch", "--import", file.to_str().unwrap()]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    /// Have a store take a certificate that signs itself at its word: the
    /// agent's list of trusted roots, which is a file, and never a
    /// question to anybody.
    fn trust_x509(&self, home: &Path, fingerprint: &str) {
        assert!(home.starts_with(scratch_root()), "{home:?}");
        fs::write(
            home.join("trustlist.txt"),
            format!("{fingerprint} S relax\n"),
        )
        .unwrap();
        // An agent that is already running for this store reads it anew.
        let _ = Command::new("gpgconf")
            .args(["--reload", "gpg-agent"])
            .env("GNUPGHOME", agent_home(home))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    /// An ed25519 key without a passphrase; the public half is `<path>.pub`.
    fn ssh_key(&self, name: &str) -> PathBuf {
        let path = self.keys.join(name);
        let status = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f"])
            .arg(&path)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        path
    }
    fn use_ssh(&self, key: &Path) {
        self.set("gpg.format", "ssh");
        self.set(
            "user.signingkey",
            key.with_extension("pub").to_str().unwrap(),
        );
    }
    /// An allowed-signers file naming these keys, and the setting for it.
    fn allow(&self, signers: &[(&str, &Path)]) {
        let mut text = String::new();
        for (principal, key) in signers {
            let public = fs::read_to_string(key.with_extension("pub")).unwrap();
            let words: Vec<&str> = public.split_whitespace().collect();
            text.push_str(&format!("{principal} {} {}\n", words[0], words[1]));
        }
        let file = self.keys.join("allowed-signers");
        fs::write(&file, text).unwrap();
        self.set("gpg.ssh.allowedSignersFile", file.to_str().unwrap());
    }

    /// A signed commit made by the command, and the same commit with its
    /// message changed under the signature.
    fn signed_and_tampered(&mut self, message: &str) -> (String, String) {
        self.copy.write("signed.txt", message).unwrap();
        let out = self.run(
            "commit.create",
            json!({ "message": message, "stage_all": true, "sign": true }),
        );
        let signed = out["id"].as_str().unwrap().to_owned();
        let git = self.git();
        let raw = git.run(["cat-file", "commit", &signed]).unwrap();
        assert!(raw.contains("\ngpgsig "), "not signed: {raw}");
        let altered = raw.replace(message, &format!("{message}, then altered"));
        let tampered = store(&git, "commit", &altered, false).unwrap();
        assert_ne!(signed, tampered);
        (signed, tampered)
    }

    /// `sign.verify` and `sign.signed` on these commits, each held against
    /// git's own grade of it. Returns the verdicts.
    pub(crate) fn verified(&mut self, ids: &[&str]) -> Vec<Value> {
        let verdicts = self.run("sign.verify", json!({ "ids": ids }));
        let carried = self.run("sign.signed", json!({ "ids": ids }));
        let (verdicts, carried) = (verdicts.as_array().unwrap(), carried.as_array().unwrap());
        assert_eq!(verdicts.len(), ids.len());
        assert_eq!(carried.len(), ids.len());
        let git = self.git();
        for ((id, ours), mark) in ids.iter().zip(verdicts).zip(carried) {
            let said = git
                .run([
                    "log",
                    "--no-walk",
                    "--format=%G?%x1f%GS%x1f%GK%x1f%GF%x1f%GP%x1f%GT",
                    id,
                ])
                .unwrap();
            let fields: Vec<&str> = said.trim_end_matches('\n').split('\u{1f}').collect();
            let raw = git.run(["cat-file", "commit", id]).unwrap();
            let headers = raw.split("\n\n").next().unwrap();
            let has = headers.lines().any(|line| line.starts_with("gpgsig "));
            assert_eq!(ours["id"], *id);
            assert_eq!(mark["id"], *id);
            assert_eq!(mark["signed"], has, "{id}: signed");
            let state = match fields[0] {
                "G" => "good",
                "U" => "untrusted",
                "X" => "expired",
                "Y" => "key_expired",
                "R" => "key_revoked",
                "B" => "bad",
                "E" if !fields[2].is_empty() => "unknown_key",
                "E" => "cannot_check",
                // The one place the command knows better than the grade.
                "N" if has => "cannot_check",
                "N" => "none",
                other => panic!("git graded {id} {other:?}"),
            };
            assert_eq!(ours["state"], state, "{id}: git says {fields:?}, we {ours}");
            assert_eq!(ours["format"], mark["format"], "{id}");
            for (name, theirs) in [
                ("signer", fields[1]),
                ("key", fields[2]),
                ("fingerprint", fields[3]),
                ("primary_fingerprint", fields[4]),
            ] {
                let theirs = if theirs.is_empty() {
                    Value::Null
                } else {
                    json!(theirs)
                };
                assert_eq!(ours[name], theirs, "{id}: {name}");
            }
            // What is wrong with an X.509 chain comes with gpgsm's words
            // for it, and only where git graded "U".
            let chain = !ours["chain"].is_null();
            if chain {
                assert_eq!(state, "untrusted", "{id}: {ours}");
                assert_eq!(ours["format"], "x509", "{id}: {ours}");
            }
            if state == "cannot_check" || chain {
                assert!(ours["reason"].is_string(), "{id}: no reason given: {ours}");
            } else {
                assert!(ours["reason"].is_null(), "{id}: {ours}");
            }
            // And `git verify-commit` accepts what is good, nothing else.
            // (A key nobody vouches for it accepts from gpg and not from
            // ssh, so that one is left to the grade; a certificate whose
            // chain does not hold it refuses.)
            let accepted = git.run(["verify-commit", id]).is_ok();
            match state {
                "good" => assert!(accepted, "{id}"),
                "untrusted" if chain => assert!(!accepted, "{id}: {ours}"),
                "untrusted" => {}
                _ => assert!(!accepted, "{id}: {state}"),
            }
        }
        verdicts.clone()
    }

    /// `tag.details` held against `git for-each-ref`, `rev-parse` and
    /// `cat-file` on the same tag. Returns the details.
    pub(crate) fn tag_details(&mut self, name: &str) -> Value {
        let details = self.run("tag.details", json!({ "name": name }));
        let git = self.git();
        let atom = |atom: &str| {
            let out = git
                .run([
                    "for-each-ref",
                    &format!("--format=%({atom})"),
                    &format!("refs/tags/{name}"),
                ])
                .unwrap();
            out.strip_suffix('\n').unwrap_or(&out).to_owned()
        };
        let reference = format!("refs/tags/{name}");
        let peeled = format!("{reference}^{{}}");
        assert_eq!(details["name"], name);
        assert_eq!(details["value"], git.rev_parse(&reference).unwrap());
        assert_eq!(details["target"], git.rev_parse(&peeled).unwrap());
        assert_eq!(
            details["target_kind"],
            git.run(["cat-file", "-t", &peeled]).unwrap().trim()
        );
        let annotated = atom("objecttype") == "tag";
        assert_eq!(details["annotated"], annotated, "{name}");
        if !annotated {
            assert!(details["tagger"].is_null(), "{details}");
            assert_eq!(details["message"], "");
            assert_eq!(details["signature"]["state"], "none");
            return details;
        }
        let tagger = &details["tagger"];
        assert_eq!(tagger["name"], atom("taggername"));
        assert_eq!(
            format!("<{}>", tagger["email"].as_str().unwrap()),
            atom("taggeremail")
        );
        let date = atom("taggerdate:raw");
        let (seconds, zone) = date.split_once(' ').unwrap();
        assert_eq!(tagger["seconds"], seconds.parse::<i64>().unwrap());
        let minutes: i64 =
            zone[1..3].parse::<i64>().unwrap() * 60 + zone[3..5].parse::<i64>().unwrap();
        let east = if zone.starts_with('-') { -60 } else { 60 };
        assert_eq!(tagger["offset"], minutes * east);
        // Git's own split of message and signature.
        let (subject, body) = (atom("contents:subject"), atom("contents:body"));
        let message = match body.trim_end() {
            "" => subject,
            body => format!("{subject}\n\n{body}"),
        };
        assert_eq!(details["message"], message, "{name}");
        let signature = atom("contents:signature");
        assert_eq!(
            details["signature"]["state"] != "none",
            !signature.trim().is_empty(),
            "{name}: {details}"
        );
        assert!(
            !details["message"].as_str().unwrap().contains("-----BEGIN"),
            "{details}"
        );
        // `git verify-tag` accepts what is good and nothing that is less
        // than a valid signature.
        let accepted = git.run(["verify-tag", &reference]).is_ok();
        match details["signature"]["state"].as_str().unwrap() {
            "good" => assert!(accepted, "{name}"),
            "untrusted" => {}
            _ => assert!(!accepted, "{name}: {details}"),
        }
        details
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        // Every key ring that was used has an agent of its own by now.
        // Stop it and take away its sockets, which gpg keeps outside the
        // ring. Only ever for a ring made here: gpgconf with any other home
        // would be talking about the developer's own agent.
        let scratch = scratch_root();
        let rings = fs::read_dir(&self.keys)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.starts_with(&scratch)
                    && path.is_dir()
                    && path
                        .file_name()
                        .is_some_and(|name| name.to_string_lossy().starts_with("gnupg-"))
            });
        for ring in rings {
            let home = agent_home(&ring);
            for action in [&["--kill", "all"][..], &["--remove-socketdir"]] {
                let _ = Command::new("gpgconf")
                    .args(action)
                    .env("GNUPGHOME", &home)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            #[cfg(target_os = "macos")]
            {
                if home != ring {
                    let _ = fs::remove_file(&home);
                }
            }
            #[cfg(windows)]
            let _ = fs::remove_dir(junction_name(&ring, "gsg"));
        }
        let _ = fs::remove_dir_all(&self.keys);
    }
}

#[test]
fn ssh_signatures_are_judged_as_git_judges_them() {
    if missing("ssh-keygen") {
        return;
    }
    let mut h = Harness::new();
    let (key, stranger) = (h.ssh_key("ada"), h.ssh_key("stranger"));
    h.use_ssh(&key);
    let unsigned = h.head();
    let (signed, tampered) = h.signed_and_tampered("signed with ssh");
    let ids = [signed.as_str(), unsigned.as_str(), tampered.as_str()];
    let marks = h.run("sign.signed", json!({ "ids": ids }));
    assert_eq!(
        marks,
        json!([
            { "id": signed, "signed": true, "format": "ssh" },
            { "id": unsigned, "signed": false, "format": null },
            { "id": tampered, "signed": true, "format": "ssh" },
        ])
    );

    // No allowed-signers file: git grades the commit "N", as if it had no
    // signature. It has one, and that is what comes back, with the reason.
    if h.seen("gpg.ssh.allowedSignersFile").is_none() {
        let verdicts = h.verified(&ids);
        assert_eq!(verdicts[0]["state"], "cannot_check");
        assert_eq!(verdicts[0]["format"], "ssh");
        let reason = verdicts[0]["reason"].as_str().unwrap();
        assert!(reason.contains("gpg.ssh.allowedSignersFile"), "{reason}");
        assert_eq!(verdicts[1]["state"], "none");
        assert_eq!(verdicts[2]["state"], "cannot_check");
    } else {
        println!("skipped one case: the developer's configuration has an allowed-signers file");
    }

    // The key is among the allowed signers.
    h.allow(&[("ada@fixture.invalid", &key)]);
    let verdicts = h.verified(&ids);
    assert_eq!(verdicts[0]["state"], "good");
    assert_eq!(verdicts[0]["signer"], "ada@fixture.invalid");
    assert_eq!(verdicts[0]["trust"], "fully");
    let print = verdicts[0]["fingerprint"].as_str().unwrap();
    assert!(print.starts_with("SHA256:"), "{print}");
    assert_eq!(verdicts[1]["state"], "none");
    assert!(verdicts[1]["format"].is_null());
    assert_eq!(verdicts[2]["state"], "bad");

    // Asked twice in one call, answered once; `log.showSignature` adds
    // nothing to what is read.
    h.set("log.showSignature", "true");
    let twice = h.run(
        "sign.verify",
        json!({ "ids": [signed, unsigned, signed, signed] }),
    );
    assert_eq!(twice.as_array().unwrap().len(), 2);
    assert_eq!(twice[0], verdicts[0]);
    h.unset("log.showSignature");

    // Only somebody else's key is allowed: a valid signature by a key
    // nothing vouches for.
    h.allow(&[("stranger@fixture.invalid", &stranger)]);
    let verdicts = h.verified(&ids);
    assert_eq!(verdicts[0]["state"], "untrusted");
    assert!(verdicts[0]["signer"].is_null());
    assert_eq!(verdicts[0]["fingerprint"], print);
    assert_eq!(verdicts[2]["state"], "bad");
}

#[test]
fn gpg_signatures_are_judged_as_git_judges_them() {
    if missing("gpg") {
        return;
    }
    let mut h = Harness::new();
    let own = h.ring("own");
    if !h.generate(&own, ADA) {
        return;
    }
    let public = h.ring("public");
    h.publish(&own, &public);
    let nobody = h.ring("nobody");
    h.use_gpg(&own);
    let unsigned = h.head();
    let (signed, tampered) = h.signed_and_tampered("signed with gpg");
    let ids = [signed.as_str(), unsigned.as_str(), tampered.as_str()];
    assert_eq!(
        h.run("sign.signed", json!({ "ids": ids })),
        json!([
            { "id": signed, "signed": true, "format": "openpgp" },
            { "id": unsigned, "signed": false, "format": null },
            { "id": tampered, "signed": true, "format": "openpgp" },
        ])
    );

    // The ring the key was made in trusts it.
    let verdicts = h.verified(&ids);
    assert_eq!(verdicts[0]["state"], "good");
    assert_eq!(verdicts[0]["signer"], ADA);
    assert_eq!(verdicts[0]["fingerprint"], h.fingerprint(&own));
    assert_eq!(verdicts[0]["trust"], "ultimate");
    assert_eq!(verdicts[1]["state"], "none");
    assert_eq!(verdicts[2]["state"], "bad");

    // A ring that has the public key and no word on whose it is.
    h.use_gpg(&public);
    let verdicts = h.verified(&ids);
    assert_eq!(verdicts[0]["state"], "untrusted");
    assert_eq!(verdicts[0]["signer"], ADA);
    assert_eq!(verdicts[2]["state"], "bad");

    // A ring without the key.
    h.use_gpg(&nobody);
    let verdicts = h.verified(&ids);
    assert_eq!(verdicts[0]["state"], "unknown_key");
    assert!(verdicts[0]["key"].is_string(), "{}", verdicts[0]);
    assert!(verdicts[0]["signer"].is_null());

    // No gpg at all: git grades "N"; the commit is signed all the same.
    let nowhere = h.keys.join("no-such-gpg");
    h.use_gpg(&nowhere);
    let verdicts = h.verified(&ids);
    assert_eq!(verdicts[0]["state"], "cannot_check");
    assert_eq!(verdicts[0]["format"], "openpgp");
    let reason = verdicts[0]["reason"].as_str().unwrap();
    assert!(reason.contains("was not found"), "{reason}");
    assert_eq!(verdicts[1]["state"], "none");
}

/// X.509 with a real gpgsm: a certificate that signs itself, made
/// in a store of the test's own, signs a commit through the engine, and
/// the signature is judged as git judges it in a store that trusts the
/// certificate, one that has it and does not, and one that has never
/// seen it.
#[test]
fn x509_signatures_are_made_and_judged_as_git_judges_them() {
    if missing("gpgsm") || missing("gpgconf") {
        return;
    }
    let mut h = Harness::new();
    let (own, own_home) = h.store_x509("own");
    let Some(print) = h.certificate(&own, "Ada Fixture") else {
        return;
    };
    assert_eq!(print.len(), 40, "{print}");
    h.trust_x509(&own_home, &print);
    let (known, _) = h.store_x509("known");
    h.publish_x509(&own, &known);
    let (nobody, _) = h.store_x509("nobody");

    // The harness has gpgsm set to a program that fails; from here on it
    // is the real one, with the store the certificate was made in.
    let use_store = |h: &Harness, wrapper: &Path| {
        h.set("gpg.x509.program", wrapper.to_str().unwrap());
    };
    h.set("gpg.format", "x509");
    h.set("user.signingkey", &print);
    use_store(&h, &own);
    let status = h.run("sign.status", json!({}));
    assert_eq!(status["format"], "x509");
    assert_eq!(status["program"], own.to_str().unwrap());
    assert_eq!(status["can_sign"], true, "{status}");
    // The certificate is among the keys there are to choose from.
    // (Key files are looked for in a directory of the test's own, and
    // the agent is not asked: neither is the developer's to be read.)
    let no_files = h.keys.join("no-key-files");
    fs::create_dir_all(&no_files).unwrap();
    let keys = h.run("sign.keys", json!({ "ssh_dir": no_files, "agent": false }));
    let listed = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|key| key["fingerprint"] == print.as_str())
        .unwrap_or_else(|| panic!("the certificate is not listed: {keys}"));
    assert_eq!(listed["format"], "x509", "{listed}");
    assert_eq!(listed["source"], "gpgsm", "{listed}");
    assert_eq!(listed["key"], print.as_str());
    assert_eq!(listed["name"], "CN=Ada Fixture,O=BitBraid Test");
    assert_eq!(listed["usable"], true);
    assert!(
        listed["expires"]
            .as_i64()
            .is_some_and(|at| at > 1_800_000_000)
    );

    let unsigned = h.head();
    let (signed, tampered) = h.signed_and_tampered("signed with a certificate");
    let raw = h.git().run(["cat-file", "commit", &signed]).unwrap();
    assert!(
        raw.contains("gpgsig -----BEGIN SIGNED MESSAGE-----"),
        "{raw}"
    );
    let ids = [signed.as_str(), unsigned.as_str(), tampered.as_str()];
    assert_eq!(
        h.run("sign.signed", json!({ "ids": ids })),
        json!([
            { "id": signed, "signed": true, "format": "x509" },
            { "id": unsigned, "signed": false, "format": null },
            { "id": tampered, "signed": true, "format": "x509" },
        ])
    );

    // The store the certificate was made in, which trusts it.
    let verdicts = h.verified(&ids);
    assert_eq!(verdicts[0]["state"], "good", "{}", verdicts[0]);
    assert_eq!(verdicts[0]["format"], "x509");
    assert_eq!(verdicts[0]["fingerprint"], print.as_str());
    let signer = verdicts[0]["signer"].as_str().unwrap();
    assert!(signer.contains("CN=Ada Fixture"), "{signer}");
    assert_eq!(verdicts[0]["trust"], "fully");
    assert_eq!(verdicts[1]["state"], "none");
    assert_eq!(verdicts[2]["state"], "bad");

    // A store that has the certificate and no word on its root.
    use_store(&h, &known);
    let verdicts = h.verified(&ids);
    assert_eq!(verdicts[0]["state"], "untrusted", "{}", verdicts[0]);
    assert!(
        verdicts[0]["signer"]
            .as_str()
            .unwrap()
            .contains("CN=Ada Fixture")
    );
    assert_eq!(verdicts[2]["state"], "bad");
    // It cannot sign with it: the key is not there.
    let status = h.run("sign.status", json!({}));
    assert_eq!(status["can_sign"], false, "{status}");
    assert!(status["reason"].is_string(), "{status}");

    // A store that has never seen it: whatever git makes of that, and
    // never a good signature.
    use_store(&h, &nobody);
    let verdicts = h.verified(&ids);
    assert!(
        ["unknown_key", "cannot_check", "bad"].contains(&verdicts[0]["state"].as_str().unwrap()),
        "{}",
        verdicts[0]
    );

    // Signing where the key is not ends with what is wrong, and with no
    // commit made.
    let before = h.head();
    h.copy
        .write("unsigned.txt", "not to be committed\n")
        .unwrap();
    let err = h
        .try_run(
            "commit.create",
            json!({ "message": "cannot be signed", "stage_all": true, "sign": true }),
        )
        .expect_err("no key to sign with");
    assert!(!err.is_empty());
    assert_eq!(h.head(), before);
}

#[test]
fn tag_details_match_git_and_carry_the_signature_state() {
    if missing("ssh-keygen") {
        return;
    }
    let mut h = Harness::new();
    let (key, stranger) = (h.ssh_key("ada"), h.ssh_key("stranger"));
    h.use_ssh(&key);
    h.allow(&[("ada@fixture.invalid", &key)]);
    let message = "signed tag\n\nwith a body\nof two lines";
    h.run(
        "tag.create",
        json!({ "name": "signed", "message": message, "sign": true }),
    );
    h.run(
        "tag.create",
        json!({ "name": "noted", "message": "plain annotated", "target": "HEAD~1" }),
    );
    h.run("tag.create", json!({ "name": "light" }));
    let git = h.git();
    git.run(["tag", "-a", "-m", "a tag of a tag", "nested", "signed"])
        .unwrap();
    git.run(["tag", "of-a-tree", "HEAD^{tree}"]).unwrap();
    git.run(["tag", "-a", "-m", "the tree", "noted-tree", "HEAD^{tree}"])
        .unwrap();

    let signed = h.tag_details("signed");
    assert_eq!(signed["message"], message);
    assert_eq!(signed["target"], h.head());
    assert_eq!(signed["target_kind"], "commit");
    assert_eq!(signed["tagger"]["name"], "Ada Fixture");
    assert_eq!(signed["tagger"]["email"], "ada@fixture.invalid");
    assert_eq!(signed["signature"]["state"], "good");
    assert_eq!(signed["signature"]["format"], "ssh");
    assert_eq!(signed["signature"]["signer"], "ada@fixture.invalid");
    let print = signed["signature"]["fingerprint"].as_str().unwrap();
    assert!(print.starts_with("SHA256:"), "{print}");
    assert!(signed["signature"]["reason"].is_null());

    let noted = h.tag_details("noted");
    assert_eq!(noted["message"], "plain annotated");
    assert_eq!(noted["signature"]["state"], "none");
    assert!(noted["signature"]["format"].is_null());

    // The fixture's own annotated tag, made with the fixture's clock.
    let fixture = h.tag_details("v1.0");
    assert_eq!(fixture["message"], "release 1.0");
    assert_eq!(fixture["tagger"]["offset"], 0);

    // Only a name for a commit: nobody made it and it says nothing.
    let light = h.tag_details("light");
    assert_eq!(light["annotated"], false);
    assert_eq!(light["value"], h.head());
    assert_eq!(light["target"], h.head());

    // A tag of a tag comes to the commit; its own signature is what counts,
    // and it has none.
    let nested = h.tag_details("nested");
    assert_eq!(nested["value"], git.rev_parse("refs/tags/nested").unwrap());
    assert_eq!(nested["target"], h.head());
    assert_eq!(nested["signature"]["state"], "none");
    assert_eq!(h.tag_details("of-a-tree")["target_kind"], "tree");
    assert_eq!(h.tag_details("noted-tree")["target_kind"], "tree");

    // The message changed under the signature.
    let raw = git.run(["cat-file", "tag", "signed"]).unwrap();
    let forged = store(
        &git,
        "tag",
        &raw.replace("signed tag\n", "signed tag!\n"),
        false,
    )
    .unwrap();
    git.run(["update-ref", "refs/tags/forged", &forged])
        .unwrap();
    let forged = h.tag_details("forged");
    assert_eq!(forged["signature"]["state"], "bad");
    assert!(
        forged["message"]
            .as_str()
            .unwrap()
            .starts_with("signed tag!")
    );

    // Somebody else's key is the only one allowed.
    h.allow(&[("stranger@fixture.invalid", &stranger)]);
    let details = h.tag_details("signed");
    assert_eq!(details["signature"]["state"], "untrusted");
    assert!(details["signature"]["signer"].is_null());
    assert_eq!(details["signature"]["fingerprint"], print);

    // No allowed signers at all: cannot be checked, and why.
    h.unset("gpg.ssh.allowedSignersFile");
    if h.seen("gpg.ssh.allowedSignersFile").is_none() {
        let details = h.tag_details("signed");
        assert_eq!(details["signature"]["state"], "cannot_check");
        assert_eq!(details["signature"]["format"], "ssh");
        let reason = details["signature"]["reason"].as_str().unwrap();
        assert!(reason.contains("gpg.ssh.allowedSignersFile"), "{reason}");
    } else {
        println!("skipped one case: the developer's configuration has an allowed-signers file");
    }

    // The same with gpg, from "good" down to "no such program".
    if missing("gpg") {
        return;
    }
    let own = h.ring("own");
    if !h.generate(&own, ADA) {
        return;
    }
    let public = h.ring("public");
    h.publish(&own, &public);
    let nobody = h.ring("nobody");
    h.set("gpg.format", "openpgp");
    h.unset("user.signingkey");
    if h.seen("user.signingkey").is_some() {
        println!("skipped the gpg cases: the developer's configuration names a signing key");
        return;
    }
    h.use_gpg(&own);
    h.run(
        "tag.create",
        json!({ "name": "sealed", "message": "sealed with gpg", "sign": true }),
    );
    let sealed = h.tag_details("sealed");
    assert_eq!(sealed["message"], "sealed with gpg");
    assert_eq!(sealed["signature"]["state"], "good");
    assert_eq!(sealed["signature"]["format"], "openpgp");
    assert_eq!(sealed["signature"]["signer"], ADA);
    assert_eq!(sealed["signature"]["fingerprint"], h.fingerprint(&own));
    assert_eq!(sealed["signature"]["trust"], "ultimate");
    let raw = git.run(["cat-file", "tag", "sealed"]).unwrap();
    let broken = store(
        &git,
        "tag",
        &raw.replace("sealed with", "broken with"),
        false,
    )
    .unwrap();
    git.run(["update-ref", "refs/tags/broken", &broken])
        .unwrap();
    assert_eq!(h.tag_details("broken")["signature"]["state"], "bad");

    h.use_gpg(&public);
    let details = h.tag_details("sealed");
    assert_eq!(details["signature"]["state"], "untrusted");
    assert_eq!(details["signature"]["signer"], ADA);
    h.use_gpg(&nobody);
    let details = h.tag_details("sealed");
    assert_eq!(details["signature"]["state"], "unknown_key");
    assert!(details["signature"]["key"].is_string());
    let nowhere = h.keys.join("no-such-gpg");
    h.use_gpg(&nowhere);
    let details = h.tag_details("sealed");
    assert_eq!(details["signature"]["state"], "cannot_check");
    let reason = details["signature"]["reason"].as_str().unwrap();
    assert!(reason.contains("was not found"), "{reason}");
    // The earlier SSH tag is still what it was: the format is the tag's.
    assert_eq!(h.tag_details("signed")["signature"]["format"], "ssh");
}

/// The four settings `sign.configure` writes, as the copy's own
/// configuration has them.
fn signing_settings(git: &Git) -> Vec<Option<String>> {
    [
        "gpg.format",
        "user.signingkey",
        "commit.gpgsign",
        "tag.gpgsign",
    ]
    .iter()
    .map(|key| {
        git.run(["config", "--local", "--get", key])
            .ok()
            .map(|value| value.trim().to_owned())
    })
    .collect()
}

#[test]
fn configure_writes_what_git_config_writes_and_status_reads_it_back() {
    if missing("ssh-keygen") {
        return;
    }
    let mut h = Harness::new();
    let twin = Harness::new();
    let key = h.ssh_key("ada");
    let public = key.with_extension("pub");
    let public = public.to_str().unwrap();
    let rest = |h: &Harness| -> Vec<String> {
        h.local_config()
            .into_iter()
            .filter(|line| {
                ![
                    "gpg.format=",
                    "user.signingkey=",
                    "commit.gpgsign=",
                    "tag.gpgsign=",
                ]
                .iter()
                .any(|key| line.starts_with(key))
            })
            .collect()
    };
    let other_settings = rest(&h);

    // Everything at once, in the repository's own configuration.
    let status = h.run(
        "sign.configure",
        json!({ "format": "ssh", "key": public, "commits": true, "tags": true, "scope": "repo" }),
    );
    let git = twin.git();
    git.run(["config", "--local", "gpg.format", "ssh"]).unwrap();
    git.run(["config", "--local", "user.signingkey", public])
        .unwrap();
    git.run(["config", "--local", "commit.gpgsign", "true"])
        .unwrap();
    git.run(["config", "--local", "tag.gpgsign", "true"])
        .unwrap();
    assert_eq!(signing_settings(&h.git()), signing_settings(&git));
    assert_eq!(rest(&h), other_settings, "nothing else was touched");
    assert_eq!(status["format"], "ssh");
    assert_eq!(status["key"], public);
    assert_eq!(status["commits"], true);
    assert_eq!(status["tags"], true);
    assert_eq!(status["program"], "ssh-keygen");
    assert_eq!(status["can_sign"], true, "{status}");
    assert!(status["reason"].is_null());
    assert_eq!(h.run("sign.status", json!({})), status);

    // With that, a commit is signed without being asked to be.
    h.copy.write("signed.txt", "by default\n").unwrap();
    let out = h.run(
        "commit.create",
        json!({ "message": "signed by default", "stage_all": true }),
    );
    let marks = h.run("sign.signed", json!({ "ids": [out["id"]] }));
    assert_eq!(marks[0]["format"], "ssh");

    // One setting, and the scope left out: the others stay.
    let status = h.run("sign.configure", json!({ "commits": false }));
    git.run(["config", "--local", "commit.gpgsign", "false"])
        .unwrap();
    assert_eq!(signing_settings(&h.git()), signing_settings(&git));
    assert_eq!(status["commits"], false);
    assert_eq!(status["tags"], true);
    assert_eq!(status["key"], public);

    // An empty key takes the setting away, as `--unset` does; twice is
    // as good as once. (Back to gpg with it: under SSH a key of the
    // developer's own would be looked at next.)
    for _ in 0..2 {
        h.run(
            "sign.configure",
            json!({ "format": "openpgp", "key": "", "tags": false }),
        );
    }
    git.run(["config", "--local", "gpg.format", "openpgp"])
        .unwrap();
    git.run(["config", "--local", "--unset", "user.signingkey"])
        .unwrap();
    git.run(["config", "--local", "tag.gpgsign", "false"])
        .unwrap();
    assert_eq!(signing_settings(&h.git()), signing_settings(&git));
    assert_eq!(rest(&h), other_settings);
    // No key is set, and gpg (a ring without keys) has none for the
    // committer.
    if h.seen("user.signingkey").is_none() && on_path("gpg") {
        let status = h.run("sign.status", json!({}));
        assert!(status["key"].is_null());
        assert_eq!(status["can_sign"], false);
        let reason = status["reason"].as_str().unwrap();
        assert!(reason.contains("no signing key is set"), "{reason}");
    }

    // A key file that is not there; the allowed-signers file as it is set.
    let gone = h.keys.join("gone.pub");
    h.run("sign.configure", json!({ "format": "ssh", "key": gone }));
    h.allow(&[("ada@fixture.invalid", &key)]);
    let status = h.run("sign.status", json!({}));
    assert_eq!(status["can_sign"], false);
    assert!(
        status["reason"]
            .as_str()
            .unwrap()
            .contains("does not exist"),
        "{status}"
    );
    assert_eq!(
        status["allowed_signers"],
        h.keys.join("allowed-signers").to_str().unwrap()
    );
    // The public half alone, with no private key beside it. With an agent
    // running the answer would be the agent's, and the developer's agent is
    // not asked: only without one.
    if std::env::var_os("SSH_AUTH_SOCK").is_none() {
        let lone = h.keys.join("lone.pub");
        fs::copy(public, &lone).unwrap();
        let status = h.run("sign.configure", json!({ "key": lone }));
        assert_eq!(status["can_sign"], false);
        assert!(
            status["reason"]
                .as_str()
                .unwrap()
                .contains("no private key"),
            "{status}"
        );
    }

    // A format git does not know, written behind the command's back.
    h.set("gpg.format", "nonsense");
    let status = h.run("sign.status", json!({}));
    assert!(status["format"].is_null());
    assert!(status["program"].is_null());
    assert_eq!(status["can_sign"], false);
    assert!(status["reason"].as_str().unwrap().contains("nonsense"));
    // A signing program that is not gpg and lists no keys: what it can
    // sign with is its own business, so nothing is said to be in the way.
    // One that is not there is.
    h.set("gpg.format", "x509");
    let status = h.run("sign.status", json!({}));
    assert_eq!(status["format"], "x509");
    assert_eq!(
        status["program"],
        runnable(&h.keys.join("no-gpgsm.sh")).to_str().unwrap()
    );
    assert_eq!(status["can_sign"], true, "{status}");
    h.set("gpg.x509.program", h.keys.join("gone").to_str().unwrap());
    let status = h.run("sign.status", json!({}));
    assert_eq!(status["can_sign"], false);
    assert!(status["reason"].as_str().unwrap().contains("was not found"));
    // The same holds for SSH: another program than ssh-keygen keeps its
    // keys where it likes.
    h.set("gpg.format", "ssh");
    h.set(
        "gpg.ssh.program",
        runnable(&h.keys.join("no-gpgsm.sh")).to_str().unwrap(),
    );
    h.set("user.signingkey", "key::ssh-ed25519 AAAA");
    assert_eq!(h.run("sign.status", json!({}))["can_sign"], true);
}

#[test]
fn status_and_keys_know_the_keys_gpg_and_ssh_have() {
    if missing("ssh-keygen") || missing("gpg") {
        return;
    }
    let mut h = Harness::new();
    let own = h.ring("own");
    if !h.generate(&own, ADA) {
        return;
    }
    let print = h.fingerprint(&own);
    let nobody = h.ring("nobody");
    h.unset("user.signingkey");
    let named = h.seen("user.signingkey").is_some();

    // No key is set: gpg is asked for the committer's.
    h.use_gpg(&own);
    let status = h.run("sign.status", json!({}));
    assert_eq!(status["format"], "openpgp");
    assert_eq!(status["program"], own.to_str().unwrap());
    assert_eq!(status["commits"], false);
    if !named {
        assert!(status["key"].is_null());
        assert_eq!(status["can_sign"], true, "{status}");
        h.use_gpg(&nobody);
        let status = h.run("sign.status", json!({}));
        assert_eq!(status["can_sign"], false);
        let reason = status["reason"].as_str().unwrap();
        assert!(reason.contains("no signing key is set"), "{reason}");
        assert!(reason.contains(ADA), "{reason}");
        h.use_gpg(&own);
    }
    // A key by its fingerprint; one gpg does not have; no gpg.
    h.run("sign.configure", json!({ "key": print }));
    assert_eq!(h.run("sign.status", json!({}))["can_sign"], true);
    let status = h.run("sign.configure", json!({ "key": "DEADBEEFDEADBEEF" }));
    assert_eq!(status["can_sign"], false);
    let reason = status["reason"].as_str().unwrap();
    assert!(
        reason.contains("no secret key for \"DEADBEEFDEADBEEF\""),
        "{reason}"
    );
    let nowhere = h.keys.join("no-such-gpg");
    h.use_gpg(&nowhere);
    let status = h.run("sign.status", json!({}));
    assert_eq!(status["can_sign"], false);
    assert!(status["reason"].as_str().unwrap().contains("was not found"));
    h.use_gpg(&own);

    // The keys: gpg's secret key, and the public key files of a directory.
    let (ada, bob) = (h.ssh_key("ada"), h.ssh_key("bob"));
    let dir = h.keys.join("dot-ssh");
    fs::create_dir_all(dir.join("a-directory.pub")).unwrap();
    for key in [&ada, &bob] {
        let public = key.with_extension("pub");
        fs::copy(&public, dir.join(public.file_name().unwrap())).unwrap();
    }
    fs::write(dir.join("garbage.pub"), "not a key\n").unwrap();
    fs::write(dir.join("empty.pub"), "").unwrap();
    fs::write(
        dir.join("known_hosts"),
        "example.invalid ssh-ed25519 AAAA\n",
    )
    .unwrap();
    fs::copy(ada.with_extension("pub"), dir.join("ada-cert.pub")).unwrap();
    let keys = h.run("sign.keys", json!({ "ssh_dir": dir, "agent": false }));
    let keys = keys.as_array().unwrap();
    assert_eq!(keys.len(), 3, "{keys:?}");
    assert_eq!(keys[0]["format"], "openpgp");
    assert_eq!(keys[0]["source"], "gpg");
    assert_eq!(keys[0]["key"], print);
    assert_eq!(keys[0]["fingerprint"], print);
    assert_eq!(keys[0]["name"], ADA);
    assert_eq!(keys[0]["kind"], "ed25519");
    assert_eq!(keys[0]["usable"], true);
    assert!(keys[0]["expires"].is_null());
    for (listed, name) in keys[1..].iter().zip(["ada", "bob"]) {
        let path = dir.join(format!("{name}.pub"));
        assert_eq!(listed["format"], "ssh");
        assert_eq!(listed["source"], "file");
        assert_eq!(listed["key"], path.to_str().unwrap());
        assert_eq!(listed["path"], path.to_str().unwrap());
        assert_eq!(listed["name"], name);
        assert_eq!(listed["kind"], "ssh-ed25519");
        assert_eq!(listed["in_agent"], false);
        // The fingerprint is ssh-keygen's own.
        let out = Command::new("ssh-keygen")
            .args(["-l", "-f"])
            .arg(&path)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        let line = String::from_utf8_lossy(&out.stdout).into_owned();
        assert_eq!(
            listed["fingerprint"],
            line.split_whitespace().nth(1).unwrap(),
            "{line}"
        );
    }
    // What a key says of itself is what `sign.configure` takes: signing
    // with the listed SSH key works.
    h.run(
        "sign.configure",
        json!({ "format": "ssh", "key": ada.with_extension("pub") }),
    );
    assert_eq!(h.run("sign.status", json!({}))["can_sign"], true);

    // A ring without keys, a directory that is not there, a file.
    h.use_gpg(&nobody);
    for dir in [h.keys.join("nowhere"), ada.with_extension("pub")] {
        let keys = h.run("sign.keys", json!({ "ssh_dir": dir, "agent": false }));
        assert_eq!(keys, json!([]));
    }
}

#[test]
fn a_signing_failure_says_what_is_wrong_and_changes_nothing() {
    let mut h = Harness::new();
    let (head, refs) = (h.head(), h.refs());
    let attempts = |h: &mut Harness| -> Vec<String> {
        let commit = h
            .try_run(
                "commit.create",
                json!({ "message": "signed", "allow_empty": true, "sign": true }),
            )
            .unwrap_err();
        let tag = h
            .try_run(
                "tag.create",
                json!({ "name": "signed", "message": "signed", "sign": true }),
            )
            .unwrap_err();
        assert!(commit.starts_with("commit: "), "{commit}");
        assert!(tag.starts_with("tag: "), "{tag}");
        assert_eq!(
            commit.trim_start_matches("commit: "),
            tag.trim_start_matches("tag: ")
        );
        vec![commit, tag]
    };
    h.unset("user.signingkey");
    let named = h.seen("user.signingkey").is_some();

    if on_path("gpg") {
        // A ring without keys: nothing for the committer.
        if !named {
            for said in attempts(&mut h) {
                assert!(said.contains("no signing key is set"), "{said}");
                assert!(said.contains(ADA), "{said}");
                assert!(said.contains("user.signingkey"), "{said}");
            }
        }
        // A key is named and gpg does not have it.
        h.set("user.signingkey", "DEADBEEFDEADBEEF");
        for said in attempts(&mut h) {
            assert!(
                said.contains("no secret key for \"DEADBEEFDEADBEEF\""),
                "{said}"
            );
            assert!(said.contains("the signing key that is set"), "{said}");
        }
    } else {
        println!("skipped two cases: gpg is not installed");
    }
    // No such program.
    let nowhere = h.keys.join("no-such-gpg");
    h.use_gpg(&nowhere);
    for said in attempts(&mut h) {
        assert!(said.contains("no-such-gpg could not be started"), "{said}");
    }

    if on_path("ssh-keygen") {
        // The key file is not there.
        let gone = h.keys.join("gone.pub");
        h.set("gpg.format", "ssh");
        h.set("user.signingkey", gone.to_str().unwrap());
        for said in attempts(&mut h) {
            assert!(said.contains("gone.pub could not be read"), "{said}");
        }
        // The public half without the private one. (ssh-keygen may ask a
        // running agent first; whatever it then says is explained too.)
        let key = h.ssh_key("ada");
        let lone = h.keys.join("lone.pub");
        fs::copy(key.with_extension("pub"), &lone).unwrap();
        h.set("user.signingkey", lone.to_str().unwrap());
        for said in attempts(&mut h) {
            assert!(said.contains("no private key for"), "{said}");
        }
        // No key at all.
        h.unset("user.signingkey");
        if !named && h.seen("gpg.ssh.defaultKeyCommand").is_none() {
            for said in attempts(&mut h) {
                assert!(said.contains("no SSH signing key is set"), "{said}");
            }
        }
    } else {
        println!("skipped three cases: ssh-keygen is not installed");
    }
    // A format git does not know.
    h.set("gpg.format", "nonsense");
    for said in attempts(&mut h) {
        assert!(said.contains("gpg.format is set to \"nonsense\""), "{said}");
    }

    assert_eq!(h.head(), head);
    assert_eq!(h.refs(), refs);
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    // Without signing the same commit goes through.
    h.set("gpg.format", "openpgp");
    h.run(
        "commit.create",
        json!({ "message": "unsigned", "allow_empty": true, "sign": false }),
    );
    assert_ne!(h.head(), head);
}

#[test]
fn hostile_parameters_and_repositories_are_refused_or_reported_never_a_crash() {
    let mut h = Harness::new();
    let git = h.git();
    let head = h.head();
    let (refs, config) = (h.refs(), h.local_config());
    let long = "k".repeat(9000);
    let tree = git.rev_parse("HEAD^{tree}").unwrap();
    let blob = git.rev_parse("HEAD:README.md").unwrap();
    let nothing = "0".repeat(40);

    // Ids: wrong shapes are refused; ids that are no commits drop out.
    for command in ["sign.verify", "sign.signed"] {
        for params in [
            json!({}),
            json!({ "ids": null }),
            json!({ "ids": "HEAD" }),
            json!({ "ids": head }),
            json!({ "ids": [7] }),
            json!({ "ids": ["HEAD"] }),
            json!({ "ids": [""] }),
            json!({ "ids": ["--all"] }),
            json!({ "ids": [format!("{head}^")] }),
            json!({ "ids": [&head[..12]] }),
            json!({ "ids": ["z".repeat(40)] }),
            json!({ "ids": [head, "-x"] }),
            json!({ "ids": [[head]] }),
            json!({ "ids": { "0": head } }),
        ] {
            assert!(
                h.try_run(command, params.clone()).is_err(),
                "{command} took {params}"
            );
        }
        let out = h.run(command, json!({ "ids": [tree, blob, nothing] }));
        assert_eq!(out, json!([]), "{command}");
        assert_eq!(h.run(command, json!({ "ids": [] })), json!([]));
    }
    // More than a call takes.
    let many = |n: usize| -> Vec<String> { (0..n).map(|i| format!("{i:040x}")).collect() };
    assert!(
        h.try_run("sign.verify", json!({ "ids": many(257) }))
            .is_err()
    );
    assert_eq!(h.run("sign.verify", json!({ "ids": many(256) })), json!([]));
    assert!(
        h.try_run("sign.signed", json!({ "ids": many(10_001) }))
            .is_err()
    );

    // Tag names.
    for name in [
        "",
        "-d",
        "--raw",
        "a..b",
        "a b",
        "a\nb",
        "HEAD",
        "@",
        "@{-1}",
        "x.lock",
        "/lead",
        "trail/",
        "v1.0~1",
        "v1.0^{}",
        "v1.0:README.md",
        "refs/tags/v1.0",
        "main",
        "no-such-tag",
        long.as_str(),
    ] {
        assert!(
            h.try_run("tag.details", json!({ "name": name })).is_err(),
            "tag.details took the name {name:?}"
        );
    }
    for params in [json!({}), json!({ "name": 3 }), json!({ "name": ["v1.0"] })] {
        assert!(
            h.try_run("tag.details", params.clone()).is_err(),
            "{params}"
        );
    }

    // Settings: nothing is written unless everything is in order. The
    // user's own configuration is never the target here.
    for params in [
        json!({}),
        json!({ "scope": "repo" }),
        json!({ "format": "" }),
        json!({ "format": "SSH" }),
        json!({ "format": "gpg" }),
        json!({ "format": "-x" }),
        json!({ "format": 3 }),
        json!({ "format": "ssh", "scope": "" }),
        json!({ "format": "ssh", "scope": "global" }),
        json!({ "format": "ssh", "scope": "system" }),
        json!({ "format": "ssh", "scope": "--global" }),
        json!({ "format": "ssh", "scope": "User" }),
        json!({ "format": "ssh", "scope": "repo " }),
        json!({ "format": "ssh", "scope": 3 }),
        json!({ "key": "-x" }),
        json!({ "key": "  -x" }),
        json!({ "key": " --local" }),
        json!({ "key": "--file=/etc/passwd" }),
        json!({ "key": "a\nb" }),
        json!({ "key": "nul\u{0}" }),
        json!({ "key": long }),
        json!({ "key": 3 }),
        json!({ "commits": "yes" }),
        json!({ "tags": 1 }),
        json!({ "format": "ssh", "commits": true, "key": "-x" }),
        json!({ "format": "ssh", "commits": true, "tags": "no" }),
    ] {
        assert!(
            h.try_run("sign.configure", params.clone()).is_err(),
            "sign.configure took {params}"
        );
        assert_eq!(h.local_config(), config, "after {params}");
    }

    // Keys: the directory has to be named in full; whatever is in it is
    // only read.
    for dir in [
        json!(""),
        json!("-x"),
        json!("relative/dir"),
        json!("~/.ssh"),
        json!("/a\nb"),
        json!(format!("/{long}")),
        json!(3),
        json!(["/tmp"]),
    ] {
        assert!(
            h.try_run("sign.keys", json!({ "ssh_dir": dir, "agent": false }))
                .is_err(),
            "sign.keys took the directory {dir}"
        );
    }
    let scratch_dir = h.keys.join("dot-ssh");
    fs::create_dir_all(&scratch_dir).unwrap();
    assert!(
        h.try_run(
            "sign.keys",
            json!({ "ssh_dir": scratch_dir, "agent": "yes" })
        )
        .is_err()
    );
    fs::write(
        scratch_dir.join("binary.pub"),
        [0u8, 159, 146, 150, 255, 10],
    )
    .unwrap();
    fs::write(scratch_dir.join("huge.pub"), "ssh-ed25519 ".repeat(100_000)).unwrap();
    fs::write(scratch_dir.join("half.pub"), "ssh-ed25519").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        scratch_dir.join("nowhere"),
        scratch_dir.join("dangling.pub"),
    )
    .unwrap();
    let keys = h.run(
        "sign.keys",
        json!({ "ssh_dir": scratch_dir, "agent": false }),
    );
    assert_eq!(keys, json!([]));
    assert_eq!(h.refs(), refs);
    assert_eq!(h.local_config(), config);

    // Commits whose signature is not one. Whatever git makes of them is
    // what comes back; a signature git will not look at is "cannot check",
    // and it never costs the commits asked about with it their answer.
    let raw = git.run(["cat-file", "commit", "HEAD"]).unwrap();
    let (headers, message) = raw.split_once("\n\n").unwrap();
    let mut forgeries = vec![head.clone()];
    for (header, format) in [
        ("gpgsig garbage", Value::Null),
        ("gpgsig ", Value::Null),
        (
            "gpgsig -----BEGIN PGP SIGNATURE-----\n \n nonsense\n -----END PGP SIGNATURE-----",
            json!("openpgp"),
        ),
        ("gpgsig -----BEGIN PGP SIGNATURE-----", json!("openpgp")),
        (
            "gpgsig -----BEGIN SSH SIGNATURE-----\n AAAA\n -----END SSH SIGNATURE-----",
            json!("ssh"),
        ),
        (
            "gpgsig -----BEGIN SIGNED MESSAGE-----\n AAAA\n -----END SIGNED MESSAGE-----",
            json!("x509"),
        ),
    ] {
        let forged = store(
            &git,
            "commit",
            &format!("{headers}\n{header}\n\n{message}"),
            false,
        )
        .unwrap_or_else(|| panic!("git would not store a commit with {header:?}"));
        let marks = h.run("sign.signed", json!({ "ids": [forged] }));
        assert_eq!(marks[0]["signed"], true, "{header:?}");
        assert_eq!(marks[0]["format"], format, "{header:?}");
        if format.is_null() {
            // Git gives up on a signature whose form it does not know, and
            // would on the whole request: it is not shown one.
            assert!(
                git.run(["log", "--no-walk", "--format=%G?", &forged])
                    .is_err(),
                "git reads {header:?} after all"
            );
        } else {
            h.verified(&[forged.as_str()]);
        }
        let verdicts = h.run("sign.verify", json!({ "ids": [forged, head] }));
        assert_eq!(verdicts[0]["state"], "cannot_check", "{header:?}");
        assert_eq!(verdicts[0]["format"], format, "{header:?}");
        assert!(verdicts[0]["reason"].is_string(), "{header:?}");
        assert_eq!(verdicts[1]["state"], "none", "{header:?}");
        forgeries.push(forged);
    }
    // A signature quoted in the message is no signature.
    let quoted = store(
        &git,
        "commit",
        &format!("{headers}\n\ngpgsig -----BEGIN PGP SIGNATURE-----\n{message}"),
        false,
    )
    .unwrap();
    let verdicts = h.verified(&[quoted.as_str()]);
    assert_eq!(verdicts[0]["state"], "none");

    // Tags that are not what a tag should be.
    let tagged = |name: &str, body: &str| -> Option<String> {
        let content = format!(
            "object {head}\ntype commit\ntag {name}\ntagger Ada Fixture <ada@fixture.invalid> 1577836800 +0000\n\n{body}"
        );
        let id = store(&git, "tag", &content, false)?;
        git.run(["update-ref", &format!("refs/tags/{name}"), &id])
            .ok()?;
        Some(id)
    };
    for (name, body, format) in [
        (
            "sig-garbage",
            "words\n-----BEGIN PGP SIGNATURE-----\n\nnonsense\n-----END PGP SIGNATURE-----\n",
            json!("openpgp"),
        ),
        (
            "sig-cut",
            "words\n-----BEGIN PGP SIGNATURE-----",
            json!("openpgp"),
        ),
        (
            "sig-ssh",
            "words\n-----BEGIN SSH SIGNATURE-----\nAAAA\n-----END SSH SIGNATURE-----\n",
            json!("ssh"),
        ),
        (
            "sig-x509",
            "words\n-----BEGIN SIGNED MESSAGE-----\nAAAA\n-----END SIGNED MESSAGE-----\n",
            json!("x509"),
        ),
        (
            "sig-only",
            "-----BEGIN PGP SIGNATURE-----\n",
            json!("openpgp"),
        ),
    ] {
        tagged(name, body).unwrap_or_else(|| panic!("git would not store the tag {name}"));
        let details = h.run("tag.details", json!({ "name": name }));
        let state = details["signature"]["state"].as_str().unwrap().to_owned();
        assert!(
            !["good", "untrusted", "none"].contains(&state.as_str()),
            "{name}: {details}"
        );
        assert_eq!(details["signature"]["format"], format, "{name}");
        assert!(!details["message"].as_str().unwrap().contains("-----BEGIN"));
        assert!(git.run(["verify-tag", name]).is_err(), "{name}");
    }
    assert!(tagged("no-body", "").is_some());
    assert_eq!(
        h.run("tag.details", json!({ "name": "no-body" }))["message"],
        ""
    );
    // Tag objects that are no tags. Git will not point a reference at most
    // of them, so the reference is written behind its back.
    let tags = h.copy.path().join(".git/refs/tags");
    let mut broken = 0;
    for (name, content) in [
        ("broken-garbage", "garbage\n".to_owned()),
        ("broken-empty", String::new()),
        ("broken-headless", format!("object {head}\n")),
        (
            "broken-type",
            format!("object {head}\ntype nonsense\ntag x\n\nm\n"),
        ),
        (
            "broken-object",
            "object zzzz\ntype commit\ntag x\n\nm\n".to_owned(),
        ),
        (
            "broken-tagger",
            format!("object {head}\ntype commit\ntag x\ntagger nobody\n\nm\n"),
        ),
        (
            "tag-of-nothing",
            format!("object {nothing}\ntype tag\ntag x\n\nm\n"),
        ),
        (
            "says-tag-is-commit",
            format!("object {head}\ntype tag\ntag x\n\nm\n"),
        ),
    ] {
        let Some(id) = store(&git, "tag", &content, true) else {
            continue;
        };
        broken += 1;
        fs::write(tags.join(name), format!("{id}\n")).unwrap();
        // An answer or an error; never a crash, and never a signature.
        if let Ok(details) = h.try_run("tag.details", json!({ "name": name })) {
            assert_eq!(details["signature"]["state"], "none", "{name}: {details}");
        }
        fs::remove_file(tags.join(name)).unwrap();
    }
    if broken == 0 {
        println!("skipped one case: this git stores no malformed tag objects");
    }
    // What a tag points at has to be there, and is what it is, whatever
    // the tag says of it.
    for (name, content, fine) in [
        (
            "points-nowhere",
            format!("object {nothing}\ntype commit\ntag x\n\nm\n"),
            false,
        ),
        (
            "tree-called-commit",
            format!("object {tree}\ntype commit\ntag x\n\nm\n"),
            true,
        ),
    ] {
        let Some(id) = store(&git, "tag", &content, true) else {
            continue;
        };
        fs::write(tags.join(name), format!("{id}\n")).unwrap();
        let details = h.try_run("tag.details", json!({ "name": name }));
        assert_eq!(details.is_ok(), fine, "{name}: {details:?}");
        if let Ok(details) = details {
            assert_eq!(details["target"], tree);
            assert_eq!(details["target_kind"], "tree");
        }
        fs::remove_file(tags.join(name)).unwrap();
    }
    // A reference to an object that is not there, and one that is not an id.
    fs::write(tags.join("dangling"), format!("{}\n", "1".repeat(40))).unwrap();
    fs::write(tags.join("scribble"), "not an id\n").unwrap();
    fs::write(tags.join("void"), "").unwrap();
    for name in ["dangling", "scribble", "void"] {
        assert!(
            h.try_run("tag.details", json!({ "name": name })).is_err(),
            "{name}"
        );
    }
    for name in ["dangling", "scribble", "void"] {
        fs::remove_file(tags.join(name)).unwrap();
    }
    // Tags of tags, deeper than anyone makes them.
    let mut inner = "v1.0".to_owned();
    for depth in 0..20 {
        let name = format!("deep-{depth}");
        git.run(["tag", "-a", "-m", "deeper", &name, &inner])
            .unwrap();
        inner = name;
    }
    assert_eq!(
        h.tag_details("deep-5")["target"],
        git.rev_parse("v1.0^{}").unwrap()
    );
    let err = h
        .try_run("tag.details", json!({ "name": "deep-19" }))
        .unwrap_err();
    assert!(err.contains("deep"), "{err}");

    // Settings that are not what they should be.
    let mut gave_up = 0;
    for (key, value) in [
        ("commit.gpgsign", "maybe"),
        ("tag.gpgsign", ""),
        ("gpg.ssh.allowedSignersFile", "/dev/null/nothing"),
        ("gpg.minTrustLevel", "nonsense"),
        // The key before the format: with SSH and no key of the copy's own,
        // the developer's would be looked at.
        ("user.signingkey", "key::garbage"),
        ("gpg.format", "ssh"),
        ("user.signingkey", "key::"),
        ("user.signingkey", "/dev/null"),
        ("user.signingkey", "~nobody-at-all/key"),
        ("gpg.format", "x509"),
        ("gpg.format", "nonsense"),
        ("gpg.format", ""),
    ] {
        h.set(key, value);
        let status = h.run("sign.status", json!({}));
        assert!(status["can_sign"].is_boolean(), "{key}={value}: {status}");
        assert_eq!(
            status["can_sign"] == false,
            status["reason"].is_string(),
            "{key}={value}: {status}"
        );
        // Every commit still gets its answer, and no forgery a good one.
        // Where git gives up over the setting, its reason is handed on.
        let verdicts = h.run("sign.verify", json!({ "ids": forgeries }));
        let states: Vec<&str> = verdicts
            .as_array()
            .unwrap()
            .iter()
            .map(|verdict| verdict["state"].as_str().unwrap())
            .collect();
        assert_eq!(states.len(), forgeries.len());
        assert_eq!(states[0], "none", "{key}={value}");
        for state in &states[1..] {
            assert!(
                !["none", "good", "untrusted"].contains(state),
                "{key}={value}: {verdicts}"
            );
        }
        if git
            .run(["log", "--no-walk", "--format=%G?", &forgeries[3]])
            .is_err()
        {
            let reason = verdicts[3]["reason"].as_str().unwrap();
            assert!(
                reason.starts_with("git could not check this signature: "),
                "{key}={value}: {reason}"
            );
            gave_up += 1;
        }
        let marks = h.run("sign.signed", json!({ "ids": forgeries }));
        assert_eq!(marks.as_array().unwrap().len(), forgeries.len());
        // Tags: an answer or an error, as git takes the setting.
        for name in ["sig-garbage", "sig-ssh", "v1.0"] {
            if let Ok(details) = h.try_run("tag.details", json!({ "name": name })) {
                let state = details["signature"]["state"].as_str().unwrap().to_owned();
                assert!(
                    !["good", "untrusted"].contains(&state.as_str()),
                    "{details}"
                );
            }
        }
    }
    assert!(gave_up > 0, "no setting made git give up");
    // The configuration itself is garbage, and git refuses everything. With
    // it went the settings that keep the developer's own gpg out of this,
    // so only what starts no signing program is asked from here on.
    fs::write(h.copy.path().join(".git/config"), "[core\n\0garbage").unwrap();
    assert!(
        h.try_run("sign.configure", json!({ "commits": true }))
            .is_err()
    );
    let marks = h.run("sign.signed", json!({ "ids": forgeries }));
    assert_eq!(marks.as_array().unwrap().len(), forgeries.len());
    let verdicts = h.run("sign.verify", json!({ "ids": [head, quoted] }));
    assert_eq!(verdicts[0]["state"], "none");
    assert_eq!(verdicts[1]["state"], "none");
    let _ = h.try_run("tag.details", json!({ "name": "v1.0" }));
}
