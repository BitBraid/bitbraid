//! Three commits that carry SSH signatures, the same on every machine: one
//! by a key that is among the allowed signers, one by a key that is not,
//! and one whose message was changed after it was signed. Git's verdicts
//! on them are good, untrusted and bad, which is what the graph marks and
//! the details say.
//!
//! The commits are stored as they stand here, signature and all, so their
//! ids never change and no key is needed to make them: the two keys they
//! were signed with were made for this once (`ssh-keygen -t ed25519`), and
//! their private halves were deleted as soon as the three signatures
//! existed. What is here is public: the commits, and the public half of
//! the key that is allowed. They hang off nothing (a commit without a
//! parent, of the empty tree), so nothing about a fixture can make them
//! stale.
//!
//! To make them anew: write each commit without its `gpgsig` header to a
//! file, sign that with `ssh-keygen -Y sign -n git -f <key>`, and put the
//! armour back in as the header, every line after the first led by a space.

use std::path::{Path, PathBuf};

/// The public half of the key that is allowed to sign.
pub const ALLOWED_KEY: &str =
    "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOBhLbOyud0TYQx7NrEEa3+Rc+SaFYdEKRnFTRLGUOys";
/// Who that key is allowed for.
pub const SIGNER: &str = "ada@fixture.invalid";

/// The three commits, oldest first: what each is called here, its id, and
/// the object as git stores it.
pub const COMMITS: [(&str, &str, &str); 3] = [
    (
        "good",
        "aa68f40694da8a5f6946c55fe9d1bc2b91279051",
        "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
         author Ada Fixture <ada@fixture.invalid> 1577980800 +0000\n\
         committer Ada Fixture <ada@fixture.invalid> 1577980800 +0000\n\
         gpgsig -----BEGIN SSH SIGNATURE-----\n\
         \x20U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAg4GEts7K53RNhDHs2sQRrf5Fz5J\n\
         \x20oVh0QpGcVNEsZQ7KwAAAADZ2l0AAAAAAAAAAZzaGE1MTIAAABTAAAAC3NzaC1lZDI1NTE5\n\
         \x20AAAAQB1vG3wLSbUYWUcctkW2Eky7TS4vi1GTLKUoj6bD65A2Jj39LFcboD2du3AREGsfMi\n\
         \x20Q78g77JA7WHcbi94xrbgw=\n\
         \x20-----END SSH SIGNATURE-----\n\
         \n\
         Sign the release notes\n\
         ",
    ),
    (
        "untrusted",
        "70ab893116129d3494f91ace179dcad8eff8e74e",
        "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
         parent aa68f40694da8a5f6946c55fe9d1bc2b91279051\n\
         author Bob Fixture <bob@fixture.invalid> 1577984400 +0000\n\
         committer Bob Fixture <bob@fixture.invalid> 1577984400 +0000\n\
         gpgsig -----BEGIN SSH SIGNATURE-----\n\
         \x20U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAgSqLOgRqIVRtTQ1ik1QBY68zer1\n\
         \x208dcfscRv8TMvIYIgoAAAADZ2l0AAAAAAAAAAZzaGE1MTIAAABTAAAAC3NzaC1lZDI1NTE5\n\
         \x20AAAAQG36KnW2HUwKVLNGYLutpytV0K8seOfZ/TwZREKAuXn7YPRHElTqVVlzAJI8NRwfPQ\n\
         \x20nKvWTevdvKdgU8/f7JYA8=\n\
         \x20-----END SSH SIGNATURE-----\n\
         \n\
         A fix from outside\n\
         ",
    ),
    (
        "bad",
        "e612d6a5d8dbbe08839692340af2c2c36f651315",
        "tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\n\
         parent 70ab893116129d3494f91ace179dcad8eff8e74e\n\
         author Ada Fixture <ada@fixture.invalid> 1577988000 +0000\n\
         committer Ada Fixture <ada@fixture.invalid> 1577988000 +0000\n\
         gpgsig -----BEGIN SSH SIGNATURE-----\n\
         \x20U1NIU0lHAAAAAQAAADMAAAALc3NoLWVkMjU1MTkAAAAg4GEts7K53RNhDHs2sQRrf5Fz5J\n\
         \x20oVh0QpGcVNEsZQ7KwAAAADZ2l0AAAAAAAAAAZzaGE1MTIAAABTAAAAC3NzaC1lZDI1NTE5\n\
         \x20AAAAQKFNGowQpHW+wSLvRxMaHl5MALm9OJNeeTFfecJ6TCc0Z/ikon3NPAe7clqphHLQqQ\n\
         \x20YrHCJqOoMQCl0SS+1aqg8=\n\
         \x20-----END SSH SIGNATURE-----\n\
         \n\
         Release 0.3, with one more thing\n\
         ",
    ),
];

/// Store the three commits in `repo`, put the branch `signed` on the last
/// of them, and name an allowed-signers file with the one key in the
/// repository's own configuration. Returns where that file is.
pub fn add_to(repo: &Path) -> PathBuf {
    for (name, id, object) in COMMITS {
        let stored = crate::repos::store_commit(repo, object);
        assert_eq!(
            stored, id,
            "the commit that is {name} is stored as it stands"
        );
    }
    let newest = COMMITS[2].1;
    crate::repos::git(repo, &["update-ref", "refs/heads/signed", newest]);
    let allowed = crate::repos::work().join("allowed_signers");
    std::fs::write(
        &allowed,
        format!("{SIGNER} namespaces=\"git\" {ALLOWED_KEY}\n"),
    )
    .unwrap();
    crate::repos::git(
        repo,
        &[
            "config",
            "gpg.ssh.allowedSignersFile",
            allowed.to_str().unwrap(),
        ],
    );
    allowed
}
