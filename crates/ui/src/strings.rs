//! The window's words: every label, heading, sentence and notice the
//! interface shows is a message in `strings/en/<area>.ftl`, one file for
//! each area of the window, looked up with `say!` (and named in a table
//! with `message!`). See `bitbraid_strings` for how.
//!
//! The tests below hold the crate to it: every lookup is in the catalog
//! with the arguments its message takes, every message is looked up, and
//! no code but a test writes a sentence or hands a word to what draws.
//!
//! Sources: Project Fluent's syntax guide
//! (<https://projectfluent.org/fluent/guide/>).

bitbraid_strings::catalog![
    "about",
    "access",
    "activity",
    "app",
    "ask",
    "blame",
    "changes",
    "commit",
    "details",
    "dialogs",
    "diff",
    "edit",
    "file-actions",
    "find-file",
    "flow",
    "graph",
    "history",
    "inbox",
    "index-flags",
    "integrations",
    "issues",
    "journal",
    "keymap",
    "lfs",
    "maintenance",
    "menus",
    "merge-tool",
    "notices",
    "preferences",
    "prefs",
    "profiles",
    "provider-clone",
    "provider-create",
    "providers",
    "pulls",
    "rebase",
    "refs",
    "repositories",
    "rerere",
    "rewrite",
    "session",
    "shell",
    "shortcuts",
    "signatures",
    "signing",
    "sparse",
    "start",
    "stashes",
    "stopped",
    "submodules",
    "tags",
    "terminal",
    "theme",
    "updates",
    "views",
    "workspace",
    "worktrees",
];

#[cfg(test)]
mod tests {
    use bitbraid_strings::check;

    /// Calls whose first argument is drawn as text.
    const DRAWN: &[&str] = &[
        "child",
        "label",
        "tooltip",
        "title",
        "description",
        "placeholder",
        "accessibility_label",
        "menu",
        "prompt",
        "ok_text",
        "cancel_text",
    ];

    /// Calls whose literal arguments are never shown to anyone: notes
    /// for whoever measures or debugs, on standard output and error, and
    /// what git's or the engine's text is compared with.
    const UNSEEN: &[&str] = &[
        "milestone",
        "debug",
        "println",
        "eprintln",
        "contains",
        "starts_with",
        "ends_with",
        "strip_prefix",
        "strip_suffix",
        "split",
        "split_once",
        "find",
    ];

    /// Literals that are not words of a language: the names of keys on
    /// the keyboard, as the key sheet and the hints print them, of
    /// systems, a sign, a mark and an address.
    const ALLOWED: &[&str] = &[
        "Alt",
        "Backspace",
        "Ctrl",
        "Del",
        "End",
        "Enter",
        "Esc",
        "Fn",
        "Home",
        "Shift",
        "Space",
        "Super",
        "Tab",
        "Linux",
        "Windows",
        // The sign of a carriage return at the end of a line of a diff.
        "CR",
        // GitLab's initials, drawn as its mark where the icons have none.
        "GL",
        // An address, as an example of one.
        "https://gitlab.example.org",
    ];

    #[test]
    fn every_lookup_is_in_the_catalog_and_every_message_is_looked_up() {
        let problems = check::catalog(&super::CATALOG, env!("CARGO_MANIFEST_DIR"));
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }

    #[test]
    fn no_code_writes_words_of_its_own() {
        let rules = check::Rules {
            drawn: DRAWN,
            unseen: UNSEEN,
            allowed: ALLOWED,
        };
        let found = check::literals(env!("CARGO_MANIFEST_DIR"), &rules);
        assert!(found.is_empty(), "{}", found.join("\n"));
    }
}
