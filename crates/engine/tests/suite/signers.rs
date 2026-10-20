//! The allowed-signers commands, held against git's own verdict: a commit
//! signed with an SSH key is "good" for git (`%G?`, `git verify-commit`,
//! which ask `ssh-keygen -Y`) exactly when its signer is in the file these
//! commands write.
//!
//! Commands run git with the developer's own environment and configuration.
//! So that no file of the developer's is ever read or written: the scratch
//! copy's own configuration names the allowed-signers file from the first
//! moment (an empty name at first, which is "none" whatever the user's
//! configuration says), every key is made beside the copy, no agent is
//! asked, and the "user's own" configuration is a file beside the copy.
#![allow(clippy::print_stdout)]

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

const SETTING: &str = "gpg.ssh.allowedSignersFile";

/// A test that needs ssh-keygen and does not find it says so and passes.
/// Looked up, never run: without arguments it would start making a key.
fn no_ssh_keygen() -> bool {
    crate::signing::missing("ssh-keygen")
}

/// Write an object as it is given, the way a commit is forged.
fn store(git: &Git, content: &str) -> String {
    let mut child = git
        .command()
        .args([
            "hash-object",
            "-t",
            "commit",
            "-w",
            "--stdin",
            "--literally",
        ])
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
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

struct Harness {
    copy: Scratch,
    /// Keys and the files that stand for the user's own: beside the copy,
    /// and gone with it.
    keys: PathBuf,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let copy = Scratch::of("branches-merges").unwrap();
        let keys = copy.path().with_extension("signers");
        let _ = fs::remove_dir_all(&keys);
        fs::create_dir_all(&keys).unwrap();
        let mut h = Self {
            copy,
            keys,
            registry: builtin_registry(),
            ctx: CommandContext::default(),
        };
        for (key, value) in [
            // First of all: no allowed-signers file but one made here.
            (SETTING, ""),
            ("gpg.format", "ssh"),
            ("gpg.ssh.program", "ssh-keygen"),
            ("gpg.minTrustLevel", "undefined"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
            ("log.showSignature", "false"),
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
    fn git(&self) -> Git {
        self.copy.git()
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|e| e.to_string())
    }
    fn refused(&mut self, id: &str, params: Value, because: &str) {
        let err = self
            .try_run(id, params.clone())
            .expect_err(&format!("{id} {params} should be refused"));
        assert!(err.contains(because), "{id} {params}: {err}");
    }
    fn set(&self, key: &str, value: &str) {
        self.git().run(["config", "--local", key, value]).unwrap();
    }
    fn local(&self, key: &str) -> Option<String> {
        self.git()
            .run(["config", "--local", "--get", key])
            .ok()
            .map(|value| value.trim_end_matches('\n').to_owned())
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
    /// The two words of a public key file: the kind and the key.
    fn public(&self, key: &Path) -> (String, String) {
        let text = fs::read_to_string(key.with_extension("pub")).unwrap();
        let mut words = text.split_whitespace();
        (
            words.next().unwrap().to_owned(),
            words.next().unwrap().to_owned(),
        )
    }
    /// The fingerprint ssh-keygen itself gives a key.
    fn fingerprint(&self, key: &Path) -> String {
        let out = Command::new("ssh-keygen")
            .arg("-l")
            .arg("-f")
            .arg(key.with_extension("pub"))
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout)
            .split_whitespace()
            .nth(1)
            .unwrap()
            .to_owned()
    }
    /// A commit signed with this key by whoever has this address.
    fn commit_signed(&mut self, key: &Path, address: &str, message: &str) -> String {
        self.set(
            "user.signingkey",
            key.with_extension("pub").to_str().unwrap(),
        );
        self.set("user.email", address);
        self.copy.write("signed.txt", message).unwrap();
        let out = self.run(
            "commit.create",
            json!({ "message": message, "stage_all": true, "sign": true }),
        );
        out["id"].as_str().unwrap().to_owned()
    }
    /// What `sign.verify` says of a commit, held against git's own grade.
    fn verdict(&mut self, id: &str) -> Value {
        let ours = self.run("sign.verify", json!({ "ids": [id] }))[0].clone();
        let grade = self
            .git()
            .run(["log", "--no-walk", "--format=%G?", id])
            .unwrap();
        let expected = match grade.trim() {
            "G" => "good",
            "U" => "untrusted",
            "B" => "bad",
            // Without a file git sees no signature at all; we say why.
            "N" | "E" => "cannot_check",
            other => panic!("git graded {id} {other:?}"),
        };
        assert_eq!(ours["state"], expected, "{id}: {ours}");
        // "Good" is what `git verify-commit` accepts, and nothing else is.
        let accepted = self.git().run(["verify-commit", id]).is_ok();
        assert_eq!(accepted, expected == "good", "{id}: {ours}");
        ours
    }
    fn file_text(&self, file: &Path) -> String {
        fs::read_to_string(file).unwrap()
    }
    /// Choose `file` as the allowed-signers file of the repository.
    fn choose(&mut self, file: &Path) -> Value {
        self.run("sign.signers_file", json!({ "path": file }))
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.keys);
    }
}

#[test]
fn a_commit_becomes_good_once_the_key_it_was_signed_with_is_allowed() {
    if no_ssh_keygen() {
        return;
    }
    let mut h = Harness::new();
    let ada = h.ssh_key("ada");
    let signed = h.commit_signed(&ada, "ada@fixture.invalid", "signed by ada");
    let print = h.fingerprint(&ada);
    let (kind, key) = h.public(&ada);

    // Nothing is chosen yet: nobody is allowed, and nobody can be.
    let listed = h.run("sign.signers", json!({}));
    assert_eq!(
        listed,
        json!({
            "file": { "configured": null, "path": null, "exists": false, "scope": null },
            "signers": [], "skipped": 0, "more": 0,
        })
    );
    assert_ne!(h.verdict(&signed)["state"], "good");
    h.refused(
        "sign.allow",
        json!({ "commit": signed }),
        "choose one first",
    );
    h.refused(
        "sign.disallow",
        json!({ "key": print }),
        "no allowed-signers file is set",
    );

    // Chosen: the setting is what `git config` writes, and the file is
    // made, in a directory that was not there, with nobody in it.
    let file = h.keys.join("trust/allowed_signers");
    let out = h.choose(&file);
    assert_eq!(
        out,
        json!({ "configured": file, "path": file, "created": true })
    );
    assert_eq!(h.local(SETTING).as_deref(), file.to_str());
    assert_eq!(h.file_text(&file), "");
    let listed = h.run("sign.signers", json!({}));
    assert_eq!(
        listed["file"],
        json!({ "configured": file, "path": file, "exists": true, "scope": "repo" })
    );
    // One step of the journal: taken back, the setting is what it was,
    // and the file, which is the user's, stays.
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Choose the allowed-signers file"
    );
    h.run("undo.last", json!({}));
    assert_eq!(h.local(SETTING).as_deref(), Some(""));
    assert!(file.is_file());
    h.run("redo.last", json!({}));
    assert_eq!(h.local(SETTING).as_deref(), file.to_str());
    // Chosen again, nothing is made and nothing is journalled anew.
    let entry = h.run("undo.peek", json!({}))["undo_id"].clone();
    let again = h.choose(&file);
    assert_eq!(again["created"], false);
    assert_eq!(h.run("undo.peek", json!({}))["undo_id"], entry);

    // The signature is valid, by a key nothing vouches for.
    let before = h.verdict(&signed);
    assert_eq!(before["state"], "untrusted");
    assert_eq!(before["fingerprint"], print);
    assert!(before["signer"].is_null());

    // The key the commit was signed with, for whoever committed it.
    let out = h.run("sign.allow", json!({ "commit": signed }));
    assert_eq!(
        out,
        json!({
            "file": file, "principals": ["ada@fixture.invalid"], "kind": kind,
            "fingerprint": print, "added": true,
        })
    );
    // The line one would have written by hand.
    let line = format!("ada@fixture.invalid namespaces=\"git\" {kind} {key}\n");
    assert_eq!(h.file_text(&file), line);

    // Good now, for git and so for us, and the signer is named.
    let after = h.verdict(&signed);
    assert_eq!(after["state"], "good");
    assert_eq!(after["signer"], "ada@fixture.invalid");
    assert_eq!(after["fingerprint"], print);
    assert_eq!(
        h.git()
            .run(["log", "--no-walk", "--format=%GS|%GK", &signed])
            .unwrap()
            .trim(),
        format!("ada@fixture.invalid|{print}")
    );

    let listed = h.run("sign.signers", json!({}));
    assert_eq!(
        listed["signers"],
        json!([{
            "line": 1, "principals": ["ada@fixture.invalid"],
            "options": ["namespaces=\"git\""], "kind": kind, "key": key,
            "fingerprint": print, "comment": null,
        }])
    );

    // Allowed twice is allowed once.
    let out = h.run("sign.allow", json!({ "commit": signed }));
    assert_eq!(out["added"], false);
    assert_eq!(h.file_text(&file), line);

    // Taken out again, by the fingerprint the list shows: no longer good.
    let out = h.run("sign.disallow", json!({ "key": print }));
    assert_eq!(out, json!({ "file": file, "removed": 1, "key": print }));
    assert_eq!(h.file_text(&file), "");
    assert_eq!(h.verdict(&signed)["state"], "untrusted");
    h.refused(
        "sign.disallow",
        json!({ "key": print }),
        "no signer with that key",
    );

    // Under another name than the committer's: good, by that name.
    h.run(
        "sign.allow",
        json!({ "commit": signed, "principals": ["release@fixture.invalid"] }),
    );
    let named = h.verdict(&signed);
    assert_eq!(named["state"], "good");
    assert_eq!(named["signer"], "release@fixture.invalid");
}

#[test]
fn signers_come_from_key_files_and_keys_and_the_rest_of_the_file_stays() {
    if no_ssh_keygen() {
        return;
    }
    let mut h = Harness::new();
    let (ada, bob, eve) = (h.ssh_key("ada"), h.ssh_key("bob"), h.ssh_key("eve"));
    let by_ada = h.commit_signed(&ada, "ada@fixture.invalid", "signed by ada");
    let by_bob = h.commit_signed(&bob, "bob@fixture.invalid", "signed by bob");
    let (ada_kind, ada_key) = h.public(&ada);
    let (bob_kind, bob_key) = h.public(&bob);
    let (eve_kind, eve_key) = h.public(&eve);

    // A file somebody keeps by hand: comments, a blank line, a line for
    // another purpose, and one that is nothing ssh would read.
    let file = h.keys.join("allowed_signers");
    let by_hand = format!(
        "# who may sign here\n\
         \n\
         eve@fixture.invalid,*@ops.fixture.invalid namespaces=\"file\",valid-before=\"20991231\" \
         {eve_kind} {eve_key} only for files\n\
         this line is not a signer\n\
         # end"
    );
    fs::write(&file, &by_hand).unwrap();
    let chosen = h.choose(&file);
    assert_eq!(chosen["created"], false, "it was there");
    assert_eq!(h.file_text(&file), by_hand, "and is as it was");
    let listed = h.run("sign.signers", json!({}));
    assert_eq!(listed["skipped"], 1);
    assert_eq!(
        listed["signers"],
        json!([{
            "line": 3,
            "principals": ["eve@fixture.invalid", "*@ops.fixture.invalid"],
            "options": ["namespaces=\"file\"", "valid-before=\"20991231\""],
            "kind": eve_kind, "key": eve_key, "fingerprint": h.fingerprint(&eve),
            "comment": "only for files",
        }])
    );
    assert_eq!(h.verdict(&by_ada)["state"], "untrusted");
    assert_eq!(h.verdict(&by_bob)["state"], "untrusted");

    // From a public key file, for two names at once.
    let out = h.run(
        "sign.allow",
        json!({
            "principals": ["ada@fixture.invalid", "ada@home.invalid"],
            "key_file": ada.with_extension("pub"),
        }),
    );
    assert_eq!(out["added"], true);
    assert_eq!(out["fingerprint"], h.fingerprint(&ada));
    // From the key itself, as it stands in such a file, comment and all.
    let text = fs::read_to_string(bob.with_extension("pub")).unwrap();
    h.run(
        "sign.allow",
        json!({ "principals": ["bob@fixture.invalid"], "key": text }),
    );
    let expected = format!(
        "{by_hand}\n\
         ada@fixture.invalid,ada@home.invalid namespaces=\"git\" {ada_kind} {ada_key}\n\
         bob@fixture.invalid namespaces=\"git\" {bob_kind} {bob_key}\n"
    );
    assert_eq!(h.file_text(&file), expected);
    let ada_verdict = h.verdict(&by_ada);
    assert_eq!(ada_verdict["state"], "good");
    assert_eq!(h.verdict(&by_bob)["state"], "good");
    assert_eq!(h.verdict(&by_bob)["signer"], "bob@fixture.invalid");

    // The same key for the same names in another order is there already;
    // for another name it is another line.
    let out = h.run(
        "sign.allow",
        json!({
            "principals": ["ada@home.invalid", "ada@fixture.invalid"],
            "key": format!("{ada_kind} {ada_key}"),
        }),
    );
    assert_eq!(out["added"], false);
    h.run(
        "sign.allow",
        json!({ "principals": ["ada@work.invalid"], "key": format!("key::{ada_kind} {ada_key}") }),
    );
    assert_eq!(
        h.run("sign.signers", json!({}))["signers"]
            .as_array()
            .unwrap()
            .len(),
        4
    );

    // One line of a key goes, named by exactly its principals; the other
    // line of that key, and everything written by hand, stays.
    let out = h.run(
        "sign.disallow",
        json!({ "key": h.fingerprint(&ada), "principals": ["ada@work.invalid"] }),
    );
    assert_eq!(out["removed"], 1);
    assert_eq!(h.file_text(&file), expected);
    assert_eq!(h.verdict(&by_ada)["state"], "good");
    // Then every line of the key, named by the key itself.
    let out = h.run("sign.disallow", json!({ "key": ada_key }));
    assert_eq!(out["removed"], 1);
    assert_eq!(
        h.file_text(&file),
        format!("{by_hand}\nbob@fixture.invalid namespaces=\"git\" {bob_kind} {bob_key}\n")
    );
    assert_eq!(h.verdict(&by_ada)["state"], "untrusted");
    assert_eq!(h.verdict(&by_bob)["state"], "good");
    // Eve's key is for another purpose: git does not take it for a commit,
    // and it was never touched.
    let by_eve = h.commit_signed(&eve, "eve@fixture.invalid", "signed by eve");
    assert_ne!(h.verdict(&by_eve)["state"], "good");

    // The file is a link to where it is really kept: written through.
    // (Making a link takes a right of its own on Windows.)
    #[cfg(unix)]
    {
        let real = h.keys.join("kept-elsewhere");
        fs::rename(&file, &real).unwrap();
        std::os::unix::fs::symlink(&real, &file).unwrap();
        h.run("sign.allow", json!({ "commit": by_eve }));
        assert!(file.is_symlink(), "the link is still a link");
        assert!(
            h.file_text(&real)
                .contains("eve@fixture.invalid namespaces=\"git\"")
        );
        assert_eq!(h.verdict(&by_eve)["state"], "good");
    }
}

#[test]
fn the_users_own_file_is_named_in_a_configuration_that_is_not_the_repositorys() {
    if no_ssh_keygen() {
        return;
    }
    let mut h = Harness::new();
    let ada = h.ssh_key("ada");
    let signed = h.commit_signed(&ada, "ada@fixture.invalid", "signed by ada");
    // What stands for the user's configuration: a file beside the copy,
    // which the copy's own configuration takes in.
    let users = h.keys.join("users-gitconfig");
    fs::write(&users, "").unwrap();
    let file = h.keys.join("users/allowed_signers");
    let before: Vec<String> = h.git().lines(["config", "--local", "--list"]).unwrap();

    // A named configuration needs a named file: there is no usual place.
    h.refused(
        "sign.signers_file",
        json!({ "scope": { "file": users } }),
        "say where the file is to be",
    );
    let out = h.run(
        "sign.signers_file",
        json!({ "path": file, "scope": { "file": users } }),
    );
    assert_eq!(out["created"], true);
    assert_eq!(out["path"], json!(file));
    // Written there as `git config --file` writes it, and nowhere else.
    let twin = h.keys.join("twin-gitconfig");
    h.git()
        .run([
            "config",
            "--file",
            twin.to_str().unwrap(),
            SETTING,
            file.to_str().unwrap(),
        ])
        .unwrap();
    assert_eq!(
        fs::read_to_string(&users).unwrap(),
        fs::read_to_string(&twin).unwrap()
    );
    assert_eq!(
        h.git().lines(["config", "--local", "--list"]).unwrap(),
        before,
        "the repository's own configuration is as it was"
    );
    // Nothing of the repository's changed, so nothing is journalled.
    assert!(
        h.run("undo.peek", json!({}))["undo"]
            .as_str()
            .is_none_or(|label| label != "Choose the allowed-signers file")
    );

    // Taken in by the repository, in place of its own (empty) setting.
    h.set("include.path", users.to_str().unwrap());
    h.git()
        .run(["config", "--local", "--unset-all", SETTING])
        .unwrap();
    let listed = h.run("sign.signers", json!({}));
    assert_eq!(listed["file"]["path"], json!(file));
    assert_eq!(listed["file"]["exists"], true);
    assert_eq!(h.verdict(&signed)["state"], "untrusted");
    h.run("sign.allow", json!({ "commit": signed }));
    assert_eq!(h.verdict(&signed)["state"], "good");
    assert_eq!(h.file_text(&file).lines().count(), 1);

    // A file whose place cannot be made, under a file: refused, and the
    // setting written for it in the named configuration is taken back.
    let blocked = h.keys.join("a-file");
    fs::write(&blocked, "in the way\n").unwrap();
    let under = blocked.join("allowed_signers");
    let users_before = fs::read_to_string(&users).unwrap();
    h.refused(
        "sign.signers_file",
        json!({ "path": under, "scope": { "file": users } }),
        "cannot be made",
    );
    assert_eq!(
        fs::read_to_string(&users).unwrap(),
        users_before,
        "a setting that names a file that cannot be is taken back"
    );
}

#[test]
fn hostile_signers_keys_commits_and_files_are_refused_and_change_nothing() {
    if no_ssh_keygen() {
        return;
    }
    let mut h = Harness::new();
    let (ada, mallory) = (h.ssh_key("ada"), h.ssh_key("mallory"));
    let signed = h.commit_signed(&ada, "ada@fixture.invalid", "signed by ada");
    let unsigned = h.git().rev_parse("HEAD~1").unwrap();
    let (kind, key) = h.public(&ada);
    let (_, mallory_key) = h.public(&mallory);
    let good_key = format!("{kind} {key}");
    let file = h.keys.join("allowed_signers");
    h.choose(&file);
    let long = "n".repeat(5000);

    // Who: nothing that could become a second word, a second line, a
    // comment or an option.
    let many: Vec<String> = (0..33).map(|n| format!("p{n}@fixture.invalid")).collect();
    for principals in [
        json!([""]),
        json!(["a b"]),
        json!(["a\nmallory@fixture.invalid ssh-ed25519 AAAA"]),
        json!(["a\tb"]),
        json!(["\"quoted\""]),
        json!(["a,b"]),
        json!(["#comment"]),
        json!(["-option"]),
        json!(["a\u{0}b"]),
        json!([long]),
        json!(many),
        json!("not a list"),
        json!([7]),
        json!({ "who": "ada" }),
    ] {
        assert!(
            h.try_run(
                "sign.allow",
                json!({ "principals": principals, "key": good_key })
            )
            .is_err(),
            "sign.allow accepted the principals {principals}"
        );
        assert_eq!(h.file_text(&file), "", "after {principals}");
    }
    // No name at all, where no commit says whose key it is.
    h.refused("sign.allow", json!({ "key": good_key }), "say who it is");

    // The key: text that is none, kinds that are not the key's, keys cut
    // short, a private key, and several ways at once.
    let private = fs::read_to_string(&ada).unwrap();
    for bad in [
        json!(""),
        json!("ssh-ed25519"),
        json!("ssh-ed25519 AAAA"),
        json!(format!("ssh-rsa {key}")),
        json!(format!("ssh-ed25519 {}", &key[..key.len() - 4])),
        json!(format!("ssh-ed25519-cert-v01@openssh.com {key}")),
        json!(private),
        json!(long),
        json!("A".repeat(200_000)),
        json!(7),
        json!(["ssh-ed25519", key]),
    ] {
        assert!(
            h.try_run(
                "sign.allow",
                json!({ "principals": ["ada@fixture.invalid"], "key": bad })
            )
            .is_err(),
            "sign.allow accepted the key {bad}"
        );
        assert_eq!(h.file_text(&file), "", "after {bad}");
    }
    h.refused(
        "sign.allow",
        json!({ "principals": ["ada@fixture.invalid"], "key": private }),
        "private key",
    );
    h.refused(
        "sign.allow",
        json!({ "principals": ["ada@fixture.invalid"] }),
        "key, key_file or commit",
    );
    h.refused(
        "sign.allow",
        json!({ "key": good_key, "commit": signed }),
        "not several",
    );

    // A key file: a full path to a public key, and nothing else.
    let big = h.keys.join("big.pub");
    fs::write(&big, "A".repeat(1024 * 1024)).unwrap();
    let passwd = Path::new("/etc/passwd");
    for path in [
        json!(""),
        json!("ada.pub"),
        json!("../ada.pub"),
        json!("-f"),
        json!(h.keys),
        json!(h.keys.join("no-such.pub")),
        json!(ada),
        json!(big),
        json!(passwd),
        json!(format!("{}\n", ada.with_extension("pub").display())),
        json!(long),
        json!(7),
    ] {
        assert!(
            h.try_run(
                "sign.allow",
                json!({ "principals": ["ada@fixture.invalid"], "key_file": path })
            )
            .is_err(),
            "sign.allow accepted the key file {path}"
        );
        assert_eq!(h.file_text(&file), "", "after {path}");
    }

    // A commit: one that is not signed, names that are none, an id that
    // is no commit, and signatures that are no SSH signature or a broken
    // one.
    let git = h.git();
    let raw = git.run(["cat-file", "commit", &signed]).unwrap();
    let tree = git.rev_parse("HEAD^{tree}").unwrap();
    let forged = |from: &str, to: &str| {
        assert!(raw.contains(from), "{from:?} is not in {raw}");
        store(&h.git(), &raw.replacen(from, to, 1))
    };
    let pgp = forged(
        "-----BEGIN SSH SIGNATURE-----",
        "-----BEGIN PGP SIGNATURE-----",
    );
    let first_line_end = raw.find("-----BEGIN SSH SIGNATURE-----\n").unwrap() + 30;
    let body_start = &raw[first_line_end..first_line_end + 12];
    // Not the magic word any more; and cut after the armour's first line.
    let no_magic = forged(body_start, " QUJDREVGR0h");
    let cut = store(
        &h.git(),
        &raw.replacen(
            &raw[first_line_end..raw.find(" -----END SSH SIGNATURE-----").unwrap()],
            " U1NIU0lHAAAAAf////8=\n",
            1,
        ),
    );
    for (rev, says) in [
        (unsigned.as_str(), "is not signed"),
        ("no-such-branch", "names no commit"),
        ("-x", "revision"),
        ("", "revision"),
        (tree.as_str(), "names no commit"),
        (pgp.as_str(), "not an SSH signature"),
        (no_magic.as_str(), "cannot be read"),
        (cut.as_str(), "cannot be read"),
    ] {
        let err = h
            .try_run("sign.allow", json!({ "commit": rev }))
            .expect_err(&format!("sign.allow took the key of {rev:?}"));
        assert!(err.contains(says), "{rev:?}: {err}");
        assert_eq!(h.file_text(&file), "", "after {rev:?}");
    }

    // A text with two keys in it is one signer, the first, and whatever
    // came with it is not written.
    let two = format!("{good_key} ada\nmallory@fixture.invalid ssh-ed25519 {mallory_key}\n");
    h.run(
        "sign.allow",
        json!({ "principals": ["ada@fixture.invalid"], "key": two }),
    );
    let one_line = format!("ada@fixture.invalid namespaces=\"git\" {kind} {key}\n");
    assert_eq!(h.file_text(&file), one_line);
    assert_eq!(h.verdict(&signed)["state"], "good");

    // Taking out: a key that is no key, and parameters that are none.
    for params in [
        json!({}),
        json!({ "key": "" }),
        json!({ "key": "two words" }),
        json!({ "key": long }),
        json!({ "key": "A".repeat(200_000) }),
        json!({ "key": 7 }),
        json!({ "key": "SHA256:nobody" }),
        json!({ "key": h.fingerprint(&ada), "principals": ["somebody@else.invalid"] }),
        json!({ "key": h.fingerprint(&ada), "principals": "ada@fixture.invalid" }),
        json!({ "key": h.fingerprint(&mallory) }),
    ] {
        assert!(
            h.try_run("sign.disallow", params.clone()).is_err(),
            "sign.disallow accepted {params}"
        );
        assert_eq!(h.file_text(&file), one_line, "after {params}");
    }

    // Where the file is: paths that are none, scopes that are none.
    let setting = h.local(SETTING);
    for params in [
        json!({ "path": "-x" }),
        json!({ "path": "a\nb" }),
        json!({ "path": long }),
        json!({ "path": 7 }),
        json!({ "path": h.keys.join("x"), "scope": "nonsense" }),
        json!({ "path": h.keys.join("x"), "scope": { "file": "relative/config" } }),
        json!({ "path": h.keys.join("x"), "scope": { "file": h.keys } }),
        json!({ "path": h.keys.join("x"), "scope": 7 }),
        json!({ "path": h.keys, "scope": "repo" }),
    ] {
        assert!(
            h.try_run("sign.signers_file", params.clone()).is_err(),
            "sign.signers_file accepted {params}"
        );
        assert_eq!(h.local(SETTING), setting, "after {params}");
    }
    assert!(!h.keys.join("x").exists());

    // The file itself in a state nobody means well with: nothing is
    // written over what is not understood.
    let odd = h.keys.join("odd");
    for (what, make) in [
        (
            "a directory",
            Box::new(|path: &Path| fs::create_dir_all(path).unwrap()) as Box<dyn Fn(&Path)>,
        ),
        (
            "not text",
            Box::new(|path: &Path| fs::write(path, [0xff, 0xfe, 0x00, 0x80]).unwrap()),
        ),
        (
            "too large",
            Box::new(|path: &Path| fs::write(path, vec![b'#'; 5 * 1024 * 1024]).unwrap()),
        ),
    ] {
        let _ = fs::remove_dir_all(&odd);
        let _ = fs::remove_file(&odd);
        make(&odd);
        h.set(SETTING, odd.to_str().unwrap());
        assert!(
            h.try_run("sign.signers", json!({})).is_err(),
            "{what} was listed"
        );
        assert!(
            h.try_run("sign.allow", json!({ "commit": signed }))
                .is_err(),
            "a signer was added to {what}"
        );
        assert!(
            h.try_run("sign.disallow", json!({ "key": h.fingerprint(&ada) }))
                .is_err(),
            "a signer was taken out of {what}"
        );
        // And verifying still answers, with whatever git makes of it.
        let verdicts = h.run("sign.verify", json!({ "ids": [signed] }));
        assert_ne!(verdicts[0]["state"], "good", "{what}");
    }
    // A file that is not there lists nobody; where its directory is not
    // there either, the first signer is refused and nothing is made.
    let gone = h.keys.join("gone/away");
    h.set(SETTING, gone.to_str().unwrap());
    let listed = h.run("sign.signers", json!({}));
    assert_eq!(listed["file"]["exists"], false);
    assert_eq!(listed["signers"], json!([]));
    assert!(
        h.try_run("sign.allow", json!({ "commit": signed }))
            .is_err()
    );
    assert!(
        !gone.exists(),
        "its directory is not there, and is not made"
    );
    // Lines of every kind of nonsense are skipped, not read wrongly.
    let noise = h.keys.join("noise");
    fs::write(
        &noise,
        format!(
            "ada@fixture.invalid\n\"unclosed quote {kind} {key}\n{kind} {key}\n\
             a b c d e f g\n{}\n\u{1b}[31mred\n",
            "x".repeat(100_000)
        ),
    )
    .unwrap();
    h.set(SETTING, noise.to_str().unwrap());
    let listed = h.run("sign.signers", json!({}));
    assert_eq!(listed["signers"], json!([]));
    assert_eq!(listed["skipped"], 6);
}

/// A signer put in or taken out is one step of the journal: taken back
/// the file is what it was to the letter, and git grades the commit as it
/// did before; made again, as after.
#[test]
fn allowing_and_removing_a_signer_are_undone_and_redone() {
    if no_ssh_keygen() {
        return;
    }
    let mut h = Harness::new();
    let (ada, bob) = (h.ssh_key("ada"), h.ssh_key("bob"));
    let by_ada = h.commit_signed(&ada, "ada@fixture.invalid", "signed by ada");
    let by_bob = h.commit_signed(&bob, "bob@fixture.invalid", "signed by bob");
    let peek = |h: &mut Harness| h.run("undo.peek", json!({}));

    // A file kept by hand, whose last line has no ending.
    let file = h.keys.join("allowed_signers");
    let by_hand = "# who may sign here\n\nthis line is not a signer\n# end";
    fs::write(&file, by_hand).unwrap();
    h.choose(&file);
    assert_eq!(peek(&mut h)["undo"], "Choose the allowed-signers file");
    assert_eq!(h.verdict(&by_ada)["state"], "untrusted");

    // Allowed from the commit: one more step, named after whom.
    let out = h.run("sign.allow", json!({ "commit": by_ada }));
    assert_eq!(out["added"], true);
    assert!(
        out.get("edit").is_none(),
        "what undo keeps is not an answer"
    );
    assert_eq!(peek(&mut h)["undo"], "Allow signer ada@fixture.invalid");
    let with_ada = h.file_text(&file);
    assert!(with_ada.starts_with(&format!("{by_hand}\nada@fixture.invalid ")));
    assert_eq!(h.verdict(&by_ada)["state"], "good");
    // The same again adds nothing and is no step.
    assert_eq!(
        h.run("sign.allow", json!({ "commit": by_ada }))["added"],
        false
    );
    assert_eq!(peek(&mut h)["undo"], "Allow signer ada@fixture.invalid");

    h.run("undo.last", json!({}));
    assert_eq!(h.file_text(&file), by_hand, "to the letter, ending and all");
    assert_eq!(h.verdict(&by_ada)["state"], "untrusted");
    assert_eq!(peek(&mut h)["redo"], "Allow signer ada@fixture.invalid");
    h.run("redo.last", json!({}));
    assert_eq!(h.file_text(&file), with_ada);
    assert_eq!(h.verdict(&by_ada)["state"], "good");

    // A second signer, for two names; then the first one taken out.
    h.run(
        "sign.allow",
        json!({
            "principals": ["bob@fixture.invalid", "bob@home.invalid"],
            "key_file": bob.with_extension("pub"),
        }),
    );
    assert_eq!(
        peek(&mut h)["undo"],
        "Allow signer bob@fixture.invalid, bob@home.invalid"
    );
    let with_both = h.file_text(&file);
    let out = h.run("sign.disallow", json!({ "key": h.fingerprint(&ada) }));
    assert_eq!(out["removed"], 1);
    assert_eq!(peek(&mut h)["undo"], "Remove signer ada@fixture.invalid");
    let without_ada = h.file_text(&file);
    assert!(!without_ada.contains("ada@fixture.invalid"));
    assert_eq!(h.verdict(&by_ada)["state"], "untrusted");
    assert_eq!(h.verdict(&by_bob)["state"], "good");

    h.run("undo.last", json!({}));
    assert_eq!(h.file_text(&file), with_both, "back where the line stood");
    assert_eq!(h.verdict(&by_ada)["state"], "good");
    h.run("redo.last", json!({}));
    assert_eq!(h.file_text(&file), without_ada);
    assert_eq!(h.verdict(&by_ada)["state"], "untrusted");

    // The file was changed since: nothing is guessed and nothing written.
    let edited = without_ada.replace(
        "bob@fixture.invalid,bob@home.invalid",
        "bob@fixture.invalid",
    );
    assert_ne!(edited, without_ada);
    fs::write(&file, &edited).unwrap();
    h.run("undo.last", json!({}));
    // Ada's line came back: it was not what changed.
    assert!(h.file_text(&file).contains("ada@fixture.invalid "));
    // Bob's line is not the one that was put in any more.
    let now = h.file_text(&file);
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("is not as this action left it"), "{err}");
    assert_eq!(h.file_text(&file), now);
    assert_eq!(
        peek(&mut h)["undo"],
        "Allow signer bob@fixture.invalid, bob@home.invalid",
        "still the step to take back"
    );
}

/// A file that the first signer made goes again with that signer, and the
/// whole is taken back step by step down to the setting.
#[test]
fn a_file_made_for_the_first_signer_goes_with_them() {
    if no_ssh_keygen() {
        return;
    }
    let mut h = Harness::new();
    let ada = h.ssh_key("ada");
    // Named in the configuration by hand, and not there.
    let file = h.keys.join("not-yet/allowed_signers");
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    h.set(SETTING, file.to_str().unwrap());
    assert!(!file.exists());
    h.run(
        "sign.allow",
        json!({ "principals": ["ada@fixture.invalid"], "key_file": ada.with_extension("pub") }),
    );
    let made = h.file_text(&file);
    assert!(made.starts_with("ada@fixture.invalid "));
    h.run("undo.last", json!({}));
    assert!(!file.exists(), "it was not there before");
    h.run("redo.last", json!({}));
    assert_eq!(h.file_text(&file), made);
    // Somebody else's line in it since: the signer goes, the file stays.
    fs::write(&file, format!("{made}# kept\n")).unwrap();
    h.run("undo.last", json!({}));
    assert_eq!(h.file_text(&file), "# kept\n");
    // Gone altogether: said, not made up.
    h.run("redo.last", json!({}));
    fs::remove_file(&file).unwrap();
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("is not as this action left it"), "{err}");
    assert!(!file.exists());
}
