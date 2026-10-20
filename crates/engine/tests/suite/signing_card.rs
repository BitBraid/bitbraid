//! Signing with a key that an OpenPGP smart card holds, through the
//! engine's commands, held to git.
//!
//! The card's PIN is never the application's to ask: gpg asks it through
//! gpg-agent's pinentry, a window of the agent's own, as it asks the
//! passphrase of any key (`crates/git/src/exe/follow.rs`). What git hands
//! on when the card cannot sign says little (a card not in its reader, a
//! reader that is gone, another card, a PIN not typed: all "Operation
//! cancelled"; a wrong PIN and a blocked one: both "Bad PIN"), so the
//! refusal is made from what gpg then says of the card
//! (`crates/git/src/signature/card.rs`).
//!
//! Two tests. The first, in ci, has a script stand in for gpg: it lists
//! the signing key as one on a card, says of the card what the case wants
//! (`--card-status`), and fails to sign with what GnuPG 2.4 said in that
//! case; every case has its own words, for a commit and a tag alike.
//!
//! The second is opt-in and needs no hardware: an OpenPGP card emulated
//! in a reader of a pcscd of the test's own.
//! `BITBRAID_OPENPGP_CARD` names a directory with two programs built from
//! their own sources (nothing of them is in this repository):
//! `libifdvpcd.so`, vsmartcard's virtual reader for pcsc-lite (its
//! `virtualsmartcard` directory, `src/ifd-vpcd/.libs/libifdvpcd.so`), and
//! `opcard-vpicc`, an OpenPGP card for that reader (opcard-rs's `vpicc`
//! example, or a program like it, that connects to the reader at
//! `localhost:35963`). pcscd, unshare, ip, gpg and scdaemon have to be
//! installed. The reader's pcscd and the card run in namespaces of their
//! own, a user's, a network's and a mount's (`unshare -rnm`): the reader's
//! port is open on that namespace's loopback alone, and pcscd, which makes
//! its socket in `/run/pcscd` whatever it is told, is given a `/run` of
//! its own in the test's directory. That socket is named to gpg's
//! scdaemon by `PCSCLITE_CSOCK_NAME`; scdaemon is told not to look for
//! readers of its own (`disable-ccid`), so no card of the developer's is
//! ever asked, and the GnuPG home is one of the test's, in a short folder
//! under `/tmp` (socket paths). The card gets keys made on it (ed25519)
//! with its factory PINs; the test commits, tags, and verifies through the
//! commands, holds every verdict to `%G?`, `git verify-commit` and
//! `git verify-tag`, then has git and the commands fail on the same card:
//! a PIN not given, a wrong PIN, a blocked PIN (unblocked again with the
//! Admin PIN), the card taken out, a blank card put in, the reader's
//! service ended. What git said and what the commands said is printed.
//!
//! Sources: gpg(1) (`--card-edit`, `--card-status`, `--command-fd`,
//! `--status-fd`, `--with-colons`), GnuPG's doc/DETAILS (the status lines
//! `GET_LINE` and `GET_BOOL` and their keywords), gpg-agent(1)
//! (`pinentry-program`), the Assuan protocol of pinentry (`SETDESC`,
//! `SETPROMPT`, `GETPIN`, `CONFIRM`, `BYE`) as GnuPG's pinentry
//! documentation gives it, scdaemon(1) (`disable-ccid`, `pcsc-driver`),
//! pcscd(8) (`--foreground`, `--config`, `--disable-polkit`),
//! reader.conf(5) (`FRIENDLYNAME`, `DEVICENAME`, `LIBPATH`, `CHANNELID`),
//! vsmartcard's documentation of vpcd (the reader's port in `DEVICENAME`),
//! opcard-rs's README (its factory PINs, 123456 and 12345678), the OpenPGP
//! smart card application specification 3.4, unshare(1), ip-link(8),
//! mount(8) (`--bind`); git-commit(1) (`-S`), git-tag(1) (`-s`),
//! git-verify-commit(1), git-verify-tag(1).
#![cfg(unix)]
#![allow(clippy::print_stdout)]

use std::fs;

use serde_json::json;

use crate::signing::{Harness, script};

/// The fingerprint and serial number of the key the stand-in lists, which
/// are those GnuPG 2.4 gave for a key made on the emulated card.
const SIGNING: &str = "7A6929A8BA898A56619E701F05CD90DC7EC9BFBD";
const SERIAL: &str = "D2760001240103040000000000000000";

/// A gpg for a key on a card in the state of the file `card` beside it,
/// failing to sign with the reason in the file `reason` beside it.
const STAND_IN: &str = r#"#!/bin/sh
here=$(dirname "$0")
state=$(cat "$here/card")
case " $* " in
  *" --list-secret-keys "*)
    if [ "$state" = on-disk ]; then serial=; else serial=@SERIAL@; fi
    echo "sec:u:255:22:05CD90DC7EC9BFBD:1792233082:::u:::scESCA:::$serial::ed25519:::0:"
    echo "fpr:::::::::@FPR@:"
    echo "uid:u::::1792233082::786A3D11567B28C1B1974D7B74A677F3814A7E4D::Ada Fixture <ada@fixture.invalid>::::::::::0:"
    exit 0 ;;
  *" --card-status "*)
    case "$state" in
      absent|daemon|service)
        case "$state" in
          absent) why="No such device" ;;
          daemon) why="No SmartCard daemon" ;;
          service) why="Service is not running" ;;
        esac
        echo "gpg: selecting card failed: $why" >&2
        echo "gpg: OpenPGP card not available: $why" >&2
        echo "AID:::"
        exit 2 ;;
      garbage) echo "%%%"; echo "::::"; exit 0 ;;
    esac
    aid=@SERIAL@; prints="@FPR@::"; tries=3
    case "$state" in
      other) aid=D2760001240103040006123456780000 ;;
      reset) prints="::" ;;
      tries-*) tries=${state#tries-} ;;
    esac
    echo "Reader:Test reader 00 00:AID:$aid:openpgp-card:"
    echo "version:0304:"
    echo "pinretry:$tries:0:3:"
    echo "fpr:$prints"
    exit 0 ;;
esac
echo "[GNUPG:] KEY_CONSIDERED @FPR@ 2" >&2
echo "[GNUPG:] BEGIN_SIGNING H10" >&2
echo "gpg: signing failed: $(cat "$here/reason")" >&2
echo "[GNUPG:] FAILURE sign 83886179" >&2
echo "gpg: signing failed: $(cat "$here/reason")" >&2
exit 2
"#;

#[test]
fn a_key_on_a_smart_card_that_cannot_sign_says_what_is_the_matter() {
    let mut h = Harness::new();
    let dir = h.keys.join("card-stand-in");
    fs::create_dir_all(&dir).unwrap();
    let gpg = dir.join("gpg.sh");
    script(
        &gpg,
        &STAND_IN
            .replace("@FPR@", SIGNING)
            .replace("@SERIAL@", SERIAL),
    );
    let gpg_path = gpg.to_str().unwrap();
    h.set("gpg.program", gpg_path);
    h.set("gpg.openpgp.program", gpg_path);
    h.set("user.signingkey", SIGNING);
    let (head, refs) = (h.head(), h.git().lines(["for-each-ref"]).unwrap());

    // Each case: what the card is up to, what gpg says when signing, and
    // what the refusal says.
    let cases: [(&str, &str, &[&str]); 13] = [
        (
            "absent",
            "Operation cancelled",
            &["smart card 0000 00000000, which is not in a reader here"],
        ),
        (
            "service",
            "Operation cancelled",
            &["no card reader can be reached", "smart card service"],
        ),
        (
            "daemon",
            "Operation cancelled",
            &["could not start scdaemon"],
        ),
        (
            "other",
            "Operation cancelled",
            &["(0006 12345678) is not the one that holds the signing key (0000 00000000)"],
        ),
        (
            "reset",
            "Operation cancelled",
            &["no longer holds the signing key"],
        ),
        (
            "tries-3",
            "Operation cancelled",
            &[
                "PIN of the smart card 0000 00000000",
                "was not given (Operation cancelled)",
            ],
        ),
        (
            "tries-3",
            "No pinentry",
            &["was not given (No pinentry)", "pinentry-program"],
        ),
        (
            "tries-2",
            "Bad PIN",
            &["did not take the PIN (Bad PIN); 2 more wrong PINs block it"],
        ),
        ("tries-1", "Bad PIN", &["one more wrong PIN blocks it"]),
        ("tries-0", "Bad PIN", &["is blocked", "Admin PIN"]),
        (
            "garbage",
            "Operation cancelled",
            &["not in a reader, or its PIN was not given"],
        ),
        // The first signing after the card was taken out.
        (
            "absent",
            "Card error",
            &["smart card 0000 00000000, which is not in a reader here"],
        ),
        (
            "tries-3",
            "Card error",
            &["could not sign (Card error)", "put it in again"],
        ),
    ];
    for (state, reason, words) in cases {
        fs::write(dir.join("card"), state).unwrap();
        fs::write(dir.join("reason"), reason).unwrap();
        let commit = h
            .try_run(
                "commit.create",
                json!({ "message": "signed", "allow_empty": true, "sign": true }),
            )
            .unwrap_err();
        let tag = h
            .try_run(
                "tag.create",
                json!({ "name": "card", "message": "signed", "sign": true }),
            )
            .unwrap_err();
        for said in [&commit, &tag] {
            for part in words {
                assert!(said.contains(part), "{state}, {reason}: {said}");
            }
        }
        assert_eq!(
            commit.trim_start_matches("commit: "),
            tag.trim_start_matches("tag: ")
        );
    }
    // A key on disk is not a card's: the refusal is about its passphrase.
    fs::write(dir.join("card"), "on-disk").unwrap();
    fs::write(dir.join("reason"), "Operation cancelled").unwrap();
    let said = h
        .try_run(
            "commit.create",
            json!({ "message": "signed", "allow_empty": true, "sign": true }),
        )
        .unwrap_err();
    assert!(said.contains("could not get the passphrase"), "{said}");
    assert!(!said.contains("smart card"), "{said}");
    // Another format is not asked about cards.
    h.set("gpg.format", "x509");
    h.set("gpg.x509.program", gpg_path);
    fs::write(dir.join("card"), "absent").unwrap();
    let said = h
        .try_run(
            "commit.create",
            json!({ "message": "signed", "allow_empty": true, "sign": true }),
        )
        .unwrap_err();
    assert!(!said.contains("not in a reader here"), "{said}");

    assert_eq!(h.head(), head);
    assert_eq!(h.git().lines(["for-each-ref"]).unwrap(), refs);
}

/// The opt-in test with the emulated card: Linux, for its namespaces.
#[cfg(target_os = "linux")]
mod emulated {
    use std::{
        fs,
        io::{BufRead as _, BufReader, Read as _, Write as _},
        os::unix::fs::PermissionsExt as _,
        path::{Path, PathBuf},
        process::{Child, Command, Output, Stdio},
        sync::mpsc,
        time::{Duration, Instant},
    };

    use serde_json::json;

    use crate::signing::{Harness, missing, script};

    /// The factory PINs of the emulated card (opcard-rs's README).
    const PIN: &str = "123456";
    const ADMIN_PIN: &str = "12345678";

    /// A pinentry that answers from files beside it and writes down what it
    /// was sent: `admin-pin` for a request that names the Admin PIN, `pin` for
    /// any other, a cancel where the file is missing; a confirmation ("put the
    /// card in") is cancelled, as by somebody without the card.
    const PINENTRY: &str = r#"#!/bin/sh
here=$(dirname "$0")
echo "OK Pleased to meet you"
which=pin
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$here/asked"
  case "$line" in
    SETDESC*|SETPROMPT*)
      case "$line" in *Admin*) which=admin-pin ;; esac
      echo "OK" ;;
    GETPIN*)
      if [ -f "$here/$which" ]; then
        printf 'D %s\nOK\n' "$(head -n 1 "$here/$which")"
      else
        echo "ERR 83886179 Operation cancelled <Pinentry>"
      fi
      which=pin ;;
    CONFIRM*) echo "ERR 83886179 Operation cancelled <Pinentry>" ;;
    BYE*) echo "OK closing connection"; exit 0 ;;
    *) echo "OK" ;;
  esac
done
"#;

    /// In the namespaces: loopback up, `/run` the test's, pcscd with the
    /// virtual reader, and a card in it; a card again whenever `insert`
    /// appears, everything ended when `stop` does. Pids in `*.pid`.
    const RIG: &str = r#"set -eu
dir=$1
setup=$2
ip link set lo up
mkdir -p "$dir/run"
mount --bind "$dir/run" /run
export PCSCLITE_CSOCK_NAME="$dir/run/pcscd/pcscd.comm"
pcscd --foreground --disable-polkit --config "$dir/reader.conf.d" >"$dir/pcscd.log" 2>&1 &
echo $! >"$dir/pcscd.pid"
i=0
while [ ! -S "$PCSCLITE_CSOCK_NAME" ] && [ $i -lt 100 ]; do sleep 0.1; i=$((i+1)); done
card() {
  n=0
  while [ $n -lt 40 ]; do
    "$setup/opcard-vpicc" >>"$dir/card.log" 2>&1 &
    pid=$!
    sleep 0.5
    if kill -0 $pid 2>/dev/null; then echo $pid >"$dir/card.pid"; return 0; fi
    n=$((n+1))
  done
}
card
while [ ! -e "$dir/stop" ]; do
  if [ -e "$dir/insert" ]; then rm -f "$dir/insert"; card; fi
  sleep 0.1
done
kill "$(cat "$dir/card.pid")" "$(cat "$dir/pcscd.pid")" 2>/dev/null || true
"#;

    /// The emulated card, its reader and its pcscd, and the GnuPG home that
    /// uses them: all in a short folder under `/tmp`, and all ended with it.
    struct Rig {
        dir: PathBuf,
        home: PathBuf,
        gpg: PathBuf,
        child: Option<Child>,
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            // The agent and scdaemon of this home only: gpgconf with any other
            // would be the developer's.
            if self.home.starts_with(&self.dir) {
                for action in [&["--kill", "all"][..], &["--remove-socketdir"]] {
                    let _ = Command::new("gpgconf")
                        .args(action)
                        .env("GNUPGHOME", &self.home)
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
            let _ = fs::write(self.dir.join("stop"), "");
            for name in ["card", "pcscd"] {
                self.kill(name);
            }
            if let Some(mut child) = self.child.take() {
                let started = Instant::now();
                while matches!(child.try_wait(), Ok(None))
                    && started.elapsed() < Duration::from_secs(5)
                {
                    std::thread::sleep(Duration::from_millis(50));
                }
                let _ = child.kill();
                let _ = child.wait();
            }
            if self.dir.starts_with("/tmp") {
                let _ = fs::remove_dir_all(&self.dir);
            }
        }
    }

    impl Rig {
        /// The rig, with a card in the reader that gpg sees. Panics with
        /// what pcscd and the card said where none comes.
        fn start(setup: &Path) -> Self {
            let dir = PathBuf::from(format!("/tmp/bbc-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            let home = dir.join("g");
            fs::create_dir_all(dir.join("reader.conf.d")).unwrap();
            fs::create_dir_all(&home).unwrap();
            fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
            // The reader listens on the namespace's loopback, port 35963.
            let reader = format!(
                "FRIENDLYNAME \"BitBraid test reader\"\nDEVICENAME /dev/null:0x8C7B\nLIBPATH {}\nCHANNELID 0x8C7B\n",
                setup.join("libifdvpcd.so").display()
            );
            fs::write(dir.join("reader.conf.d").join("vpcd"), reader).unwrap();
            let socket = dir.join("run").join("pcscd").join("pcscd.comm");
            // No reader of scdaemon's own: the card of the test, and no other.
            fs::write(home.join("scdaemon.conf"), "disable-ccid\n").unwrap();
            script(&home.join("pinentry.sh"), PINENTRY);
            fs::write(
                home.join("gpg-agent.conf"),
                format!("pinentry-program {}\n", home.join("pinentry.sh").display()),
            )
            .unwrap();
            let gpg = dir.join("gpg.sh");
            script(
                &gpg,
                &format!(
                    "#!/bin/sh\n# gpg with the test's home and card, and no window to ask in.\nunset DISPLAY WAYLAND_DISPLAY GPG_TTY\nGNUPGHOME={}\nPCSCLITE_CSOCK_NAME={}\nexport GNUPGHOME PCSCLITE_CSOCK_NAME\nexec gpg \"$@\"\n",
                    home.display(),
                    socket.display()
                ),
            );
            let child = Command::new("unshare")
                .args(["--user", "--map-root-user", "--net", "--mount", "--"])
                .args(["sh", "-c", RIG, "sh"])
                .arg(&dir)
                .arg(setup)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let rig = Self {
                dir,
                home,
                gpg,
                child: Some(child),
            };
            let seen = rig.wait(|| rig.card_status().is_some(), 30);
            assert!(
                seen,
                "no card could be seen (pcscd: {}; card: {})",
                fs::read_to_string(rig.dir.join("pcscd.log"))
                    .unwrap_or_default()
                    .trim(),
                fs::read_to_string(rig.dir.join("card.log"))
                    .unwrap_or_default()
                    .trim()
            );
            rig
        }

        fn gpg(&self, args: &[&str]) -> Output {
            Command::new(&self.gpg)
                .args(args)
                .stdin(Stdio::null())
                .output()
                .unwrap()
        }

        /// `--card-status --with-colons` when a card is in, else None.
        fn card_status(&self) -> Option<String> {
            let out = self.gpg(&["--batch", "--no-tty", "--with-colons", "--card-status"]);
            let said = String::from_utf8_lossy(&out.stdout).into_owned();
            (out.status.success() && said.starts_with("Reader:")).then_some(said)
        }

        /// What gpg wrote besides when asked about the card.
        fn card_said(&self) -> String {
            let out = self.gpg(&["--batch", "--no-tty", "--with-colons", "--card-status"]);
            String::from_utf8_lossy(&out.stderr).into_owned()
        }

        /// Whether `done` holds within `seconds`.
        fn wait(&self, done: impl Fn() -> bool, seconds: u64) -> bool {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(seconds) {
                if done() {
                    return true;
                }
                std::thread::sleep(Duration::from_millis(200));
            }
            false
        }

        fn kill(&self, name: &str) {
            if let Ok(pid) = fs::read_to_string(self.dir.join(format!("{name}.pid"))) {
                let _ = Command::new("kill")
                    .arg(pid.trim())
                    .stderr(Stdio::null())
                    .status();
            }
        }

        /// What the pinentry answers: the PIN, or a cancel.
        fn answer(&self, pin: Option<&str>) {
            let file = self.home.join("pin");
            match pin {
                Some(pin) => fs::write(&file, format!("{pin}\n")).unwrap(),
                None => {
                    let _ = fs::remove_file(&file);
                }
            }
            fs::write(self.home.join("admin-pin"), format!("{ADMIN_PIN}\n")).unwrap();
            fs::write(self.home.join("asked"), "").unwrap();
        }

        /// The lines the pinentry was sent since [`Rig::answer`] that ask
        /// something: what it said and what it asked for.
        fn asked(&self) -> Vec<String> {
            fs::read_to_string(self.home.join("asked"))
                .unwrap_or_default()
                .lines()
                .filter(|line| {
                    ["SETDESC", "SETPROMPT", "GETPIN", "CONFIRM"]
                        .iter()
                        .any(|word| line.starts_with(word))
                })
                .map(str::to_owned)
                .collect()
        }

        /// `--card-edit` driven through its prompts: the commands at its own
        /// prompt in turn, every other question answered by its keyword (the
        /// answers for one keyword in turn, the last one again after them).
        /// Panics on a question it has no answer for, or when gpg fails.
        fn card_edit(&self, commands: &[&str], answers: &[(&str, &str)]) {
            let mut child = Command::new(&self.gpg)
                .args([
                    "--command-fd",
                    "0",
                    "--status-fd",
                    "1",
                    "--no-tty",
                    "--card-edit",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let mut stdin = child.stdin.take().unwrap();
            let (stdout, mut stderr) = (child.stdout.take().unwrap(), child.stderr.take().unwrap());
            let errors = std::thread::spawn(move || {
                let mut text = String::new();
                let _ = stderr.read_to_string(&mut text);
                text
            });
            let (send, receive) = mpsc::channel();
            std::thread::spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if send.send(line).is_err() {
                        break;
                    }
                }
            });
            let mut commands = commands.iter();
            let mut used = vec![false; answers.len()];
            let deadline = Instant::now() + Duration::from_secs(120);
            let mut unknown = None;
            while let Ok(line) =
                receive.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                let Some(keyword) = line
                    .strip_prefix("[GNUPG:] GET_LINE ")
                    .or_else(|| line.strip_prefix("[GNUPG:] GET_BOOL "))
                else {
                    continue;
                };
                let answer = if keyword == "cardedit.prompt" {
                    commands.next().copied().unwrap_or("quit")
                } else if let Some(at) = (0..answers.len())
                    .find(|at| answers[*at].0 == keyword && !used[*at])
                    .or_else(|| (0..answers.len()).rfind(|at| answers[*at].0 == keyword))
                {
                    used[at] = true;
                    answers[at].1
                } else {
                    unknown = Some(keyword.to_owned());
                    break;
                };
                if writeln!(stdin, "{answer}").is_err() {
                    break;
                }
            }
            drop(stdin);
            let ended = Instant::now();
            while matches!(child.try_wait(), Ok(None)) && ended.elapsed() < Duration::from_secs(10)
            {
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let status = child.wait().unwrap();
            let errors = errors.join().unwrap_or_default();
            assert!(unknown.is_none(), "gpg asked {unknown:?}: {errors}");
            assert!(status.success(), "{status}: {errors}");
        }
    }

    /// When git is asked to sign the same way, if at all.
    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Git {
        Before,
        After,
        Not,
    }

    /// Git and the commands failing to sign with the card as it is: what
    /// each said is printed, and what the commands said returned; HEAD is
    /// still `head`.
    fn refused(h: &mut Harness, head: &str, case: &str, git: Git) -> String {
        let by_git = |h: &Harness| {
            let theirs = h
                .git()
                .run(["commit", "--allow-empty", "-S", "-m", "by git"])
                .unwrap_err()
                .to_string();
            let line = theirs
                .lines()
                .find(|line| line.contains("signing failed"))
                .unwrap_or("(nothing)")
                .trim()
                .to_owned();
            println!("{case}: git says {line:?}");
        };
        if git == Git::Before {
            by_git(h);
        }
        let ours = h
            .try_run(
                "commit.create",
                json!({ "message": "by us", "allow_empty": true, "sign": true }),
            )
            .unwrap_err();
        println!("{case}: we say {ours:?}");
        if git == Git::After {
            by_git(h);
        }
        assert_eq!(h.head(), head, "{case}");
        ours
    }

    #[test]
    fn a_key_on_an_emulated_openpgp_card_signs_and_says_what_is_the_matter() {
        let Some(setup) = std::env::var_os("BITBRAID_OPENPGP_CARD").map(PathBuf::from) else {
            println!(
                "skipped: set BITBRAID_OPENPGP_CARD to a directory with vsmartcard's libifdvpcd.so and an OpenPGP card for it, opcard-vpicc"
            );
            return;
        };
        for program in ["libifdvpcd.so", "opcard-vpicc"] {
            if !setup.join(program).is_file() {
                println!("skipped: {program} is not in {}", setup.display());
                return;
            }
        }
        for program in ["pcscd", "unshare", "ip", "gpg", "gpgconf"] {
            if missing(program) {
                return;
            }
        }
        let components = Command::new("gpgconf")
            .arg("--list-components")
            .output()
            .unwrap();
        if !String::from_utf8_lossy(&components.stdout).contains("scdaemon:") {
            println!("skipped: GnuPG has no scdaemon here");
            return;
        }
        let setup = fs::canonicalize(&setup).unwrap();
        let mut h = Harness::new();
        let rig = Rig::start(&setup);

        // Keys made on the card: ed25519 for all three, with its factory PINs.
        rig.answer(Some(PIN));
        rig.card_edit(
            &["admin", "key-attr", "generate", "quit"],
            &[
                ("cardedit.genkeys.algo", "2"),
                ("keygen.curve", "1"),
                ("cardedit.genkeys.backup_enc", "n"),
                ("cardedit.genkeys.replace_keys", "y"),
                ("keygen.valid", "0"),
                ("keygen.valid.okay", "y"),
                ("keygen.name", "Ada Fixture"),
                ("keygen.email", "ada@fixture.invalid"),
                ("keygen.comment", ""),
                ("keygen.userid.cmd", "O"),
            ],
        );
        let listing = rig.gpg(&["--batch", "--with-colons", "--list-secret-keys"]);
        let listing = String::from_utf8_lossy(&listing.stdout).into_owned();
        let mut lines = listing.lines();
        let sec = lines
            .find(|line| line.starts_with("sec:"))
            .unwrap_or_else(|| panic!("no key was made on the card: {listing}"));
        let serial = sec.split(':').nth(14).unwrap_or("").to_owned();
        assert!(serial.starts_with("D276000124"), "not on a card: {sec}");
        let fingerprint = lines
            .find_map(|line| line.strip_prefix("fpr:::::::::"))
            .map(|rest| rest.trim_end_matches(':').to_owned())
            .unwrap();
        let gpg = rig.gpg.to_str().unwrap();
        h.set("gpg.program", gpg);
        h.set("gpg.openpgp.program", gpg);
        h.set("user.signingkey", &fingerprint);

        // The commands see a key that can sign.
        let status = h.run("sign.status", json!({}));
        assert_eq!(status["can_sign"], true, "{status}");
        let no_files = h.keys.join("no-ssh-keys");
        fs::create_dir_all(&no_files).unwrap();
        let keys = h.run("sign.keys", json!({ "ssh_dir": no_files, "agent": false }));
        let listed = keys
            .as_array()
            .unwrap()
            .iter()
            .find(|key| key["fingerprint"] == fingerprint.as_str())
            .unwrap_or_else(|| panic!("the card's key is not listed: {keys}"));
        assert_eq!(listed["usable"], true, "{listed}");

        // A signed commit: the PIN asked by gpg-agent's pinentry, once.
        rig.answer(Some(PIN));
        h.copy.write("card.txt", "signed on a card\n").unwrap();
        let made = h.run(
            "commit.create",
            json!({ "message": "signed on a card", "stage_all": true, "sign": true }),
        );
        let id = made["id"].as_str().unwrap().to_owned();
        let asked = rig.asked();
        assert_eq!(
            asked.iter().filter(|line| *line == "GETPIN").count(),
            1,
            "{asked:?}"
        );
        assert!(
            asked
                .iter()
                .any(|line| line.starts_with("SETDESC Please unlock the card")
                    && line.contains("Number: 0000 00000000")),
            "{asked:?}"
        );
        assert!(
            asked.iter().any(|line| line == "SETPROMPT PIN"),
            "{asked:?}"
        );
        let verdict = &h.verified(&[&id])[0];
        assert_eq!(verdict["state"], "good", "{verdict}");
        assert_eq!(verdict["format"], "openpgp", "{verdict}");
        assert_eq!(verdict["signer"], "Ada Fixture <ada@fixture.invalid>");
        assert_eq!(verdict["fingerprint"], fingerprint.as_str());
        println!(
            "signed commit: git G, verify-commit accepts; ours {}",
            verdict["state"]
        );

        // A signed tag.
        rig.answer(Some(PIN));
        h.run(
            "tag.create",
            json!({ "name": "on-a-card", "message": "tagged on a card", "sign": true }),
        );
        let details = h.tag_details("on-a-card");
        assert_eq!(details["signature"]["state"], "good", "{details}");
        println!(
            "signed tag: verify-tag accepts; ours {}",
            details["signature"]["state"]
        );

        let head = h.head();
        // The PIN not given.
        rig.answer(None);
        let said = refused(&mut h, &head, "PIN not given", Git::Before);
        assert!(
            said.contains("PIN of the smart card 0000 00000000")
                && said.contains("was not given (Operation cancelled)"),
            "{said}"
        );

        // A wrong PIN: git takes a try, the commands the next two.
        rig.answer(Some("000000"));
        let said = refused(&mut h, &head, "wrong PIN", Git::Before);
        assert!(
            said.contains("did not take the PIN (Bad PIN); one more wrong PIN blocks it"),
            "{said}"
        );
        let said = refused(&mut h, &head, "PIN blocked", Git::Not);
        assert!(said.contains("is blocked"), "{said}");
        rig.answer(Some(PIN));
        let said = refused(&mut h, &head, "PIN blocked, the right one typed", Git::Not);
        assert!(said.contains("is blocked"), "{said}");
        // Unblocked as the refusal says, with the Admin PIN; then it signs.
        rig.answer(Some(PIN));
        rig.card_edit(
            &["admin", "passwd", "quit"],
            &[
                ("cardutil.change_pin.menu", "2"),
                ("cardutil.change_pin.menu", "q"),
            ],
        );
        let status = rig.card_status().unwrap();
        assert!(status.contains("\npinretry:3:"), "{status}");
        rig.answer(Some(PIN));
        let again = h.run(
            "commit.create",
            json!({ "message": "after the unblocking", "allow_empty": true, "sign": true }),
        );
        let head = again["id"].as_str().unwrap().to_owned();
        assert_eq!(h.verified(&[&head])[0]["state"], "good");

        // The card taken out: gpg-agent asks for it, and nobody puts it in.
        // The first signing after it may end in "Card error" instead,
        // which the commands are given.
        rig.kill("card");
        assert!(
            rig.wait(|| rig.card_status().is_none(), 20),
            "the card stayed"
        );
        rig.answer(Some(PIN));
        let said = refused(&mut h, &head, "card taken out", Git::After);
        assert!(
            said.contains("smart card 0000 00000000, which is not in a reader here"),
            "{said}"
        );
        assert!(
            rig.asked()
                .iter()
                .any(|line| line.starts_with("SETDESC Please insert the card")),
            "{:?}",
            rig.asked()
        );

        // A blank card put in: the same number, and not the key.
        fs::write(rig.dir.join("insert"), "").unwrap();
        assert!(rig.wait(|| rig.card_status().is_some(), 30), "no card came");
        let said = refused(&mut h, &head, "a blank card put in", Git::Before);
        assert!(said.contains("no longer holds the signing key"), "{said}");

        // The reader's service ended. Asked at once, scdaemon says "No such
        // device", as for a card taken out, while its connection to pcscd
        // breaks; after that, that the service is not running.
        rig.kill("pcscd");
        assert!(
            rig.wait(|| rig.card_said().contains("Service is not running"), 20),
            "{}",
            rig.card_said()
        );
        let said = refused(&mut h, &head, "reader gone", Git::Before);
        assert!(
            said.contains("no card reader can be reached")
                && said.contains("(pcscd) is not running"),
            "{said}"
        );
    }
}
