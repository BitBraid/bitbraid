//! A repository that is open survives the packs git adds while it is
//! open. gitoxide fixes the room its object database has for packs when
//! it opens a repository, and fails a read once more are on disk than
//! that ("too few index slots"); a treeless clone that programs work in
//! can gain hundreds of packs in minutes. The room is twice what is on
//! disk and at least 1024, and a handle that runs out all the same is
//! opened again and has room for what is there.

use std::path::Path;

use bitbraid_git::{DiffOptions, Oid, Repository, commit_files, repo::is_out_of_room};
use bitbraid_testkit::Scratch;

/// A commit made from outside the open handle, after it was opened, and
/// put into a pack of its own with everything else that was loose: the
/// handle has to look among the packs on disk to find it.
fn commit_in_a_new_pack(copy: &Scratch) -> Oid {
    let git = copy.git();
    copy.write("added/later.txt", "made while the repository was open\n")
        .unwrap();
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "made from outside"])
        .unwrap();
    git.run(["repack", "-d", "-q"]).unwrap();
    git.rev_parse("HEAD").unwrap().parse().unwrap()
}

/// `count` packs more, as git leaves them after as many small fetches:
/// copies of a pack that is there, each under a name of its own.
fn more_packs(repo: &Path, count: usize) {
    let dir = repo.join(".git/objects/pack");
    let pack = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "pack"))
        .expect("a pack to copy");
    for n in 0..count {
        let name = format!("pack-{:040x}", 0x00fe_ed00_0000_usize.saturating_add(n));
        std::fs::copy(&pack, dir.join(format!("{name}.pack"))).unwrap();
        std::fs::copy(pack.with_extension("idx"), dir.join(format!("{name}.idx"))).unwrap();
    }
}

fn files_of(repo: &Repository, id: Oid) -> Result<Vec<String>, String> {
    commit_files(repo, id, &DiffOptions::default())
        .map(|files| files.into_iter().map(|file| file.path).collect())
        .map_err(|err| err.to_string())
}

/// A hundred packs added while the repository is open: the handle that was
/// open finds a commit in one of them. gitoxide's own room would be 32.
#[test]
fn a_hundred_packs_added_while_open_are_read() {
    let copy = Scratch::of("linear").unwrap();
    let repo = Repository::open(copy.path()).unwrap();
    let made = commit_in_a_new_pack(&copy);
    more_packs(copy.path(), 100);
    assert_eq!(
        files_of(&repo, made),
        Ok(vec!["added/later.txt".to_owned()])
    );
}

/// More packs added than there is room for: the read says so in words
/// that are known for it, and the handle opened again reads it.
#[test]
fn a_handle_that_runs_out_of_room_is_opened_again() {
    let copy = Scratch::of("linear").unwrap();
    let mut repo = Repository::open(copy.path()).unwrap();
    let made = commit_in_a_new_pack(&copy);
    more_packs(copy.path(), 1100);
    let said = files_of(&repo, made).unwrap_err();
    assert!(is_out_of_room(&said), "{said}");
    repo.reopen().unwrap();
    assert_eq!(
        files_of(&repo, made),
        Ok(vec!["added/later.txt".to_owned()])
    );
    // Nothing else is taken for it.
    for other in [
        "",
        "An object with id 1234 could not be found",
        "the partial clone left it on the remote",
    ] {
        assert!(!is_out_of_room(other), "{other}");
    }
}
