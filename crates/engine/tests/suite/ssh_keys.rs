//! SSH keys for the providers: a key made with ssh-keygen,
//! never over a file that is there, and the fingerprint of one that is.
//!
//! Every key is made in a scratch folder; nothing here looks at the
//! user's own `~/.ssh` or their agent.
#![allow(clippy::print_stdout)]

use std::path::{Path, PathBuf};
use std::process::Command;

use bitbraid_engine::{CommandContext, builtin_registry};
use bitbraid_testkit::scratch_root;
use serde_json::{Value, json};

/// Whether ssh-keygen is installed: looked up on PATH, never run.
fn missing_keygen() -> bool {
    let found = std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| dir.join("ssh-keygen").is_file())
    });
    if !found {
        println!("skipped: ssh-keygen is not installed");
    }
    !found
}

/// A folder of its own for one test.
fn folder(name: &str) -> PathBuf {
    let dir = scratch_root().join("ssh-keys").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(id: &str, params: Value) -> Result<Value, String> {
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run(id, &mut ctx, &params)
        .map_err(|err| err.to_string())
}

/// The fingerprint ssh-keygen itself gives a key.
fn keygen_fingerprint(path: &Path) -> String {
    let out = Command::new("ssh-keygen")
        .args(["-l", "-f"])
        .arg(path)
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned()
}

#[test]
fn a_new_key_is_made_beside_its_public_half_and_read_back() {
    if missing_keygen() {
        return;
    }
    let dir = folder("new");
    // The folder of the key is made when it is not there.
    let path = dir.join("keys").join("id_ed25519_provider");
    let made = run(
        "ssh.key_generate",
        json!({ "path": path, "comment": "someone@bitbraid" }),
    )
    .unwrap();
    let public = dir.join("keys").join("id_ed25519_provider.pub");
    assert_eq!(made["private"], json!(path));
    assert_eq!(made["public"], json!(public));
    assert!(path.is_file() && public.is_file());
    assert_eq!(made["kind"], "ED25519");
    assert_eq!(made["comment"], "someone@bitbraid");
    assert_eq!(made["fingerprint"], keygen_fingerprint(&public));
    let key = made["public_key"].as_str().unwrap();
    assert!(key.starts_with("ssh-ed25519 "), "{key}");
    assert!(key.ends_with(" someone@bitbraid"), "{key}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(dir.join("keys"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0, "the folder is the user's alone: {mode:o}");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "the private key is the user's alone");
    }

    // The fingerprint of the private key is that of its public half.
    let print = run("ssh.key_fingerprint", json!({ "path": path })).unwrap();
    assert_eq!(print["fingerprint"], made["fingerprint"]);
    assert_eq!(print["public"], json!(public));
    assert_eq!(print["public_key"], made["public_key"]);
    assert_eq!(print["bits"], 256);
    let print = run("ssh.key_fingerprint", json!({ "path": public })).unwrap();
    assert_eq!(print["fingerprint"], made["fingerprint"]);
}

#[test]
fn nothing_is_ever_written_over() {
    if missing_keygen() {
        return;
    }
    let dir = folder("over");
    let path = dir.join("id_ed25519");
    std::fs::write(&path, "mine\n").unwrap();
    let err = run("ssh.key_generate", json!({ "path": path })).unwrap_err();
    assert!(err.contains("not written over"), "{err}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine\n");

    // A public key alone is enough to refuse: it would be replaced.
    let other = dir.join("id_other");
    std::fs::write(dir.join("id_other.pub"), "ssh-ed25519 AAAA mine\n").unwrap();
    let err = run("ssh.key_generate", json!({ "path": other })).unwrap_err();
    assert!(err.contains("not written over"), "{err}");
    assert!(!other.exists());

    // And a link that leads nowhere.
    #[cfg(unix)]
    {
        let link = dir.join("id_link");
        std::os::unix::fs::symlink(dir.join("nowhere"), &link).unwrap();
        let err = run("ssh.key_generate", json!({ "path": link })).unwrap_err();
        assert!(err.contains("not written over"), "{err}");
        assert!(!dir.join("nowhere").exists());
    }
}

#[test]
fn hostile_parameters_are_refused_without_harm() {
    let dir = folder("hostile");
    let long = "a".repeat(100_000);
    for params in [
        json!({}),
        json!({ "path": null }),
        json!({ "path": 7 }),
        json!({ "path": ["a"] }),
        json!({ "path": "" }),
        json!({ "path": "relative/id" }),
        json!({ "path": dir.join("..").join("escape") }),
        json!({ "path": format!("{}/{long}", dir.display()) }),
        json!({ "path": format!("{}/a\u{0}b", dir.display()) }),
        json!({ "path": format!("{}/a\nb", dir.display()) }),
        json!({ "path": dir.join("c1"), "comment": "two\nlines" }),
        json!({ "path": dir.join("c2"), "comment": long }),
        json!({ "path": dir.join("c3"), "comment": 12 }),
    ] {
        assert!(
            run("ssh.key_generate", params.clone()).is_err(),
            "{params} was taken"
        );
        assert!(
            run("ssh.key_fingerprint", params.clone()).is_err(),
            "{params} was taken"
        );
    }
    // Nothing was made by any of them.
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
    assert!(!scratch_root().join("ssh-keys").join("escape").exists());

    // A file that is no key has no fingerprint.
    if !missing_keygen() {
        let junk = dir.join("junk");
        std::fs::write(&junk, "not a key at all\n").unwrap();
        assert!(run("ssh.key_fingerprint", json!({ "path": junk })).is_err());
        assert!(run("ssh.key_fingerprint", json!({ "path": dir.join("absent") })).is_err());
    }
}
