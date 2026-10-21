# AGENTS.md — for coding agents working on BitBraid

How the code is put together, the rules it keeps, how it is tested and
how to work on it safely. `CLAUDE.md` imports this file for Claude Code;
what a build needs is in `README.md` ("Building").

## 1. Where things are

```
apps/
  bitbraid/             the desktop binary
  bitbraid-cli/         the command line: every registered command, with --json;
                        `commands --json` prints the command catalog
crates/                 layer   contents
  core                  L0      paths, processes and the login shell's environment, WSL, the clock
  strings               L0      the message catalogs (`say!`, `message!`)
  testkit               L0      fixtures, twin clones, stand-in servers (smart HTTP, sshd,
                                proxies, TLS with an authority of its own); for tests only
  git                   L1      repository snapshot, graph layout, status, diff and blame; gix reader,
                                git2 adapter, the git executable (askpass, signing, LFS, tools), watcher
  terminal              L1      a shell on a pseudo-terminal, keys and the mouse to bytes,
                                paths quoted for each shell
  engine                L2      command registry, journal and undo, operations and progress,
                                conflict prediction, hooks, credential questions
  integrations          L3      GitHub and GitLab: accounts, the keychain, HTTP with recordings,
                                pull requests, issues, checks, the inbox, the updater
  ui                    L4      the application: shell, views, themes, keymap, terminal panel
xtask/                  the gate and the release tools (`cargo xtask` lists them)
fixtures/               repositories described in TOML, built by `cargo xtask fixtures`
packaging/              desktop entries, AppStream metadata, the AUR's PKGBUILD, macOS
                        bundle files, the release build's Dockerfile, the update key
site/                   the website (section 8)
brand/                  the logo, mark and icon (section 8)
```

A crate depends only on strictly lower layers: `cargo xtask layers`
checks it against `[package.metadata.bitbraid] layer` in each manifest.
Dev-dependencies are exempt, so any crate may use `bitbraid-testkit`;
`xtask` is a tool and exempt too. `engine` knows nothing about rendering
or transports.

## 2. How it works

- **Everything is a command.** User-visible behaviour is a `CommandSpec`
  registered in the engine's registry: a dotted lowercase id
  (`branch.create`), a label, a description and a parameter table, with a
  handler that takes JSON parameters and returns JSON. Registering one
  puts it in the palette, the keymap and the CLI (`bitbraid-cli run
  <id>`); the window's menus, buttons and keys run it by its id and ask
  the engine whether it can run now, and why not. The interface never
  performs git operations itself.
- **Git in three tiers.** `gix` for reads (refs, the revision walk,
  objects, status, diffs, blame, config, the index, mailmap, ignore rules,
  pathspecs); `git2` (vendored libgit2) for the few in-process mutations
  where gix is partial (line-level index edits, stash, submodule and
  worktree changes); the git executable for everything that must behave
  exactly like the user's git (fetch, push, pull, merge, rebase,
  cherry-pick, revert, commits with signing, hooks, LFS, maintenance), so
  its credential helpers, signing, hooks, rerere and merge drivers come
  for free. git 2.38 or later; an older one is warned about. Where `PATH`
  has no git and the preferences name none, a git shipped beside the
  application runs (`crates/git/src/shipped.rs`). Where gitoxide does not
  read what git reads (replaced and grafted commits), a file's history
  asks git itself.
- **git is the oracle.** Every read and mutation is held to the real git
  in differential tests (section 5).
- **Undo** is a journal of engine commands: each entry keeps the old and
  new values of the refs it touched, a tree of the index and backup
  objects for anything discarded; ref updates are compare-and-swap.
- **Interactive rebase**: our editor makes the todo; git runs it, with
  `GIT_SEQUENCE_EDITOR` pointing at a small helper that writes it. There
  is no sequencer of ours.
- **The terminal** is `crates/terminal`, on `alacritty_terminal`: the
  session (a shell on a pseudo-terminal, the parser and grid, an I/O
  thread), keys and the mouse to bytes, the frame on show, and quoting of
  paths for shells; of the workspace it depends on `strings` only.
  The engine decides what is started and how it completes
  (`terminal.shell`, `terminal.paste_paths`, `terminal.completion_words`,
  `terminal.complete`); the window keeps the sessions, one list per
  worktree path. The stream of output never passes through the registry.
- **Providers** are commands of `crates/integrations` that register into
  the engine's registry: `bitbraid_integrations::registry()` is the
  engine's commands and these, and the window and the CLI build that one.
  A command registered from a higher layer brings its access line and
  parameter table with it (`Registry::register_described`).
  `CommandContext::extensions` holds values by type that the engine never
  looks into: the integrations crate keeps its `Services` (the HTTP
  client, the keychain, the cache) there, and tests put stand-ins in it.
- **Tokens only in the keychain**: one item per account, service
  `bitbraid` (the Secret Service through `oo7` on Linux, Credential
  Manager through `keyring` on Windows, the login keychain through
  `security-framework` on macOS); the account's other facts are in
  `integrations.toml`. No token is written to a file, a log, an error, a
  command's output or a `Debug` text, and a build that cannot reach a
  keychain refuses to store one rather than keep it elsewhere.
- **Updates** are off until the user turns them on; then the updater
  reads a small feed published with each release and checks a minisign
  signature against `packaging/update-key.pub`, built into the
  application (a placeholder there means no update is ever installed).
- **The application's own connections** sign in to a proxy with Basic
  only; git goes through any proxy curl can, and a proxy that asks for
  more is said so.
- **Signatures**: BitBraid shows git's and gpgsm's verdict and never
  checks certificates itself.
- **Nothing that needs a vendor's proprietary cloud** is in scope.
- **What a released version wrote stays readable.** 0.1.0 has been
  released, so a later build reads the files and settings a released one
  left: a renamed file or key is migrated or still read. What changed
  only between two releases needs no such care.

## 3. Working on the interface

- Every view has a picture in both themes. The offscreen harness
  (`crates/ui/tests/offscreen.rs`) opens the real window on the toolkit's
  test platform, without a display, with two fonts of its own
  (`crates/ui/tests/fonts`). A view is brought on show in
  `tests/offscreen/views.rs` (or the file of its area: `inbox.rs`,
  `pulls.rs`, `workspaces.rs` and the like) and asked what it shows (its
  `probe`) before its picture is taken. The frame is drawn at two pixels
  to the point, stored averaged down four to one, and compared with a
  tolerance of 3 of 255 per channel.
- After a change to a view, run `cargo xtask snapshot`, look at the
  pictures it stored anew (in full size under `snapshots/` in the target
  directory) and commit them with the change; a failing comparison writes
  the drawn picture and a difference picture there too. After a merge
  that conflicts in a picture, take either side and run it again.
- What a view does when keys are pressed is tested in
  `tests/offscreen/behaviour*.rs`, which needs no renderer. Both run on
  Linux only; elsewhere the program says it skipped them.
- The harness starts itself again with only scratch directories in its
  environment, no display, no SSH agent, no session bus, `TZ=UTC` and the
  clock held at 2020-01-03 12:00 UTC (`BITBRAID_NOW`); its scratch
  directories are at `/tmp/bitbraid-offscreen`, because paths are drawn,
  and a lock lets one run at a time use it. The git it runs is the real
  one behind a script that answers `--version` with 2.99.0, and that a
  test can hold up (`Stage::hold_git`) to look at the window while
  something takes a while.
- It runs the scheduler until no request to a session thread is
  unanswered (`session::unanswered`) and nothing is left to run: no
  sleeps. Timers run on the test platform's clock, which stands still
  until the test moves it (`Stage::pass`); what the toolkit does a moment
  later (focus handed back, a toast going away) happens only then.
- If a comparison fails everywhere by small amounts on a new graphics
  adapter, `crates/ui/tests/snapshots/drawn-by.txt` says which adapter
  stored the pictures; the tolerance is `compare::CHANNEL_TOLERANCE`, and
  beyond 5 a faint border could go missing unseen.
- Switches for running the application by hand: `BITBRAID_DEBUG=1` (a log
  on stderr with start-up timings), `BITBRAID_NEW_INSTANCE=1` (do not hand
  over to a running instance), `BITBRAID_GIT=<path>` (the git executable
  when the preferences name none), `BITBRAID_NOW` (hold the clock).
  `BITBRAID_GITHUB_CLIENT_ID`, `BITBRAID_GITLAB_CLIENT_ID` and
  `BITBRAID_UPDATE_FEED` point a build at other OAuth applications and
  another update feed.

## 4. Rules

### Never crash (outranks everything)

- No panics in non-test code. No `unwrap`, `expect`, `panic!`, `todo!`,
  `unimplemented!`, `unreachable!`, `dbg!`, and no slice indexing: use `?`,
  `get()`, `ok_or`, `saturating_*`, `checked_*` and explicit fallbacks. The
  workspace lints enforce this; tests may relax them with
  `#![cfg_attr(test, allow(...))]`. `unsafe` is forbidden workspace-wide.
- Input-derived numbers are hostile: validate bounds before use. So is
  everything a provider, a remote, a shell or a file in a repository says.
- Lock poisoning and thread joins are `Result`s, not panics.
- Every crash fix ships with a regression test.

### Original work

- Write the code from public documentation, public specifications and the
  observed behaviour of git and the services BitBraid talks to. Never copy
  code, text, icons, images or styles from another product.
- Git's source is GPL: read it to understand, never copy it. The
  application takes no GPL dependency (libgit2's GPLv2 carries the linking
  exception); Zed's terminal crates are GPL as well and are not a source.
- Every module that implements a feature carries a `Sources:` line in its
  doc comment listing the public documents consulted: git's manual pages,
  RFCs, the providers' API documentation.

### Names and words

- Product: BitBraid. Binary: `bitbraid`. Crates: `bitbraid-<name>`.
  Environment variables: `BITBRAID_*`. Desktop application id: `bitbraid`.
- Command ids are `area.verb[_qualifier]`. Files and modules are
  `snake_case`; types `PascalCase`; no abbreviations in public APIs.
- Words a person reads are messages of the crate's catalog,
  `strings/en/<area>.ftl`, looked up with `say!` (`message!` in a
  `static` table); an id begins with its file's area. Never a literal in
  the code: the crates' `strings::tests` fail on a lookup the catalog
  lacks and on a message nobody looks up, the interface's on a word
  written into its code (`crates/strings` says how).

### Adding a feature

1. Write the command in `crates/engine/src/<area>_cmds.rs`, or in
   `crates/integrations/src/<area>_cmds.rs` for a provider feature (a new
   module per area avoids merge conflicts), with its `Sources:` line.
2. Add differential tests against git on fixtures from `fixtures/*.toml`
   (add a fixture if none fits), and a panic-hunt case: hostile
   parameters, a hostile repository. A provider feature is tested against
   recorded answers (`crates/integrations/tests/recordings`).
3. The CLI has it through the registry. If it has UI, add it in
   `crates/ui`, with its pictures (section 3).
4. If the feature makes a network connection, keep the README's "Privacy"
   section true in the same commit.

## 5. Testing

- `cargo xtask ci` must pass: fmt, clippy `-D warnings`, tests, snapshot
  `--check`, layers, fixtures, cargo-deny. Install cargo-deny (`cargo
  install cargo-deny`): without it the gate skips that step with a note,
  which is not a pass. The hosted `ci` workflow, started by hand, runs
  the same gate on Linux, Windows and macOS.
- Reads: compare with `git` output on fixtures. Mutations: perform the same
  operation with `git` on a twin clone and compare refs, index and tree.
- Hostile input in a panic-hunt case stays small, a few megabytes at
  most: what is tested is a bound, not volume.
- A crate's integration tests are one program, `tests/suite/main.rs`: a new
  test file is a module there (`tests/suite/<name>.rs`), never a file of its
  own in `tests/`, which would be compiled and linked as a program of its
  own. The one exception is a test that changes or counts what is one for
  the whole process (the git executable chosen, the process's threads):
  its own file in `tests/`, saying why at its head, since the suite's tests
  run side by side in one process. The UI crate's one program is
  `tests/offscreen.rs` (section 3).
- A test that needs a program the machine may lack (gpg, gpgsm, git-lfs,
  a shell) says that it is skipped when it is missing.
- A test that needs a service from outside (github.com, gitlab.com, a
  GitLab of one's own, Docker, a real tool or proxy) is opt-in and never
  part of the gate (section 6). Live tests at a provider run only against
  accounts made for testing, check whose token they hold and refuse any
  other. Recordings are scrubbed of tokens, names, addresses and ids
  before they are committed.

```
cargo xtask ci                                     # the full gate
cargo xtask snapshot [--check] [name]              # draw the views off screen: store the pictures, or compare
cargo xtask fixtures [--clean] [name...]           # build fixture repos into <target>/fixtures
cargo test -p <crate> --test suite <name>::        # one module of a crate's tests
cargo test -p bitbraid-ui --test offscreen [name]  # what the window does, without a renderer
cargo xtask                                        # the rest: benchmarks, corpora, packages, sbom, update feed
```

## 6. Opt-in tests

None of these runs by default. Each is asked for with environment
variables (and `--ignored` where the test is ignored), and says it was
skipped otherwise. Nothing about any account is in the repository.

**Test accounts in the keychain** (Linux, Secret Service, `secret-tool`).
The live provider tests read each token from an item of the service
`bitbraid-test`, with the account's login as the `login` attribute; the
secret is read from standard input:

```
secret-tool store --label='BitBraid test' service bitbraid-test account github-main login <login>
secret-tool store --label='BitBraid test' service bitbraid-test account github-reviewer login <login>
secret-tool store --label='BitBraid test' service bitbraid-test account gitlab-main login <login>
```

A test's first request must be `GET /user` answering with that login, or
nothing else is let through for that token. Use accounts made for
testing only.

| Tests | Variables | Needs | Command |
|---|---|---|---|
| Provider commands at github.com | `BITBRAID_LIVE_GITHUB_ORG=<org>`; optional `BITBRAID_LIVE_REPO` (default `bitbraid-live`), `BITBRAID_LIVE_GITHUB_DEVICE=<file>` (the full device flow; someone approves the code) | a test organisation; its machine account (admin; token scopes `repo workflow read:org read:user user:email delete_repo admin:public_key`) as `github-main`; a second account as `github-reviewer` | `cargo test -p bitbraid-integrations --test suite live_github:: -- --ignored` |
| Provider commands at gitlab.com | `BITBRAID_LIVE_GITLAB_COM=1`; optional `BITBRAID_LIVE_REPO`, `BITBRAID_LIVE_GITLAB_DEVICE=<file>` | one test account (token scopes `api read_user`) as `gitlab-main` | `cargo test -p bitbraid-integrations --test suite live_gitlab_com:: -- --ignored` |
| Provider commands and issue branches at a GitLab of one's own | `BITBRAID_GITLAB_CE=http://127.0.0.1:8929`, `BITBRAID_GITLAB_CE_TOKEN_FILE=<file>` | GitLab CE on the same machine (the recordings and their replays expect 127.0.0.1:8929); an admin's personal access token (scope `api`) in the file | `cargo test -p bitbraid-integrations --test suite live_gitlab_ce:: -- --ignored`, and `live_issue_branch::` the same way |
| The window's merge requests at a GitLab of one's own | the same two | the same; Linux | `cargo test -p bitbraid-ui --test offscreen live_gitlab` (not ignored; the variables ask for it) |
| Git LFS at a GitLab of one's own | the same two; optional `BITBRAID_GITLAB_CE_SSH_PORT` (2224), `BITBRAID_LIVE_SCRATCH`, `BITBRAID_LIVE_KEEP` | git-lfs, ssh, ssh-keygen; the GitLab's SSH on 2224 with `lfs_pure_ssh_protocol` on; a scratch `HOME` | `cargo test -p bitbraid-cli --test suite live_lfs:: -- --ignored` |
| Git LFS at github.com | `BITBRAID_LIVE_GITHUB_ORG`; optional `BITBRAID_LIVE_GITHUB_SECOND=1` | git-lfs; `github-main` (and `github-reviewer` for the second test) | `cargo test -p bitbraid-cli --test suite live_lfs_github:: -- --ignored` |
| Fetch and push over HTTPS and SSH at github.com and gitlab.com | `BITBRAID_LIVE_TRANSPORT=github,gitlab`, `BITBRAID_LIVE_GITHUB_ORG`; optional `BITBRAID_LIVE_REPO` | `github-main`, `gitlab-main`; `<repo>-writes`, made by the provider tests; ssh; a scratch `HOME` and `GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM=1`, `SSH_AUTH_SOCK` and `GITLAB_TOKEN` unset | `cargo test -p bitbraid-cli --test suite live_hosts:: -- --ignored` |
| Real proxies (Squid: Basic, Negotiate, NTLM) | `BITBRAID_REAL_PROXY=1` | Docker without sudo (the image is built at the first run) | `cargo test -p bitbraid-integrations --test suite real_proxy:: -- --ignored`, and the same in `bitbraid-cli` |
| The system's keychain (Linux) | `BITBRAID_TEST_KEYCHAIN=scratch` | a Secret Service of its own: `dbus-run-session` with `gnome-keyring-daemon` and a scratch `HOME` (the head of `crates/integrations/tests/suite/keychain.rs` says how) | the suite's program with `--ignored keychain::`, inside that session |
| SSH signing with a PKCS#11 token | `BITBRAID_SOFTHSM2=<libsofthsm2.so>`; on Windows also `BITBRAID_SOFTHSM2_CONF` | SoftHSM2 | `cargo test -p bitbraid-cli --test suite token_signing::` |
| An emulated OpenPGP card | `BITBRAID_OPENPGP_CARD=<folder with libifdvpcd.so and opcard-vpicc>` | pcscd, unshare, ip, gpg, scdaemon | `cargo test -p bitbraid-engine --test suite signing_card::` |
| The known diff and merge tools | `BITBRAID_TEST_TOOLS=<folder of the tools' programs>` | the tools; on Linux a headless compositor (`cage`, `WLR_BACKENDS=headless`) | `cargo test -p bitbraid-engine --test suite known_tools_live::` |
| Other shells on a pseudo-terminal | `BITBRAID_TEST_SHELLS=<folder with zsh, fish, nu, dash, tcsh>` | the shells | `cargo test -p bitbraid-terminal --test suite shells::` (`quoting::` always runs; the variable adds shells) |
| MinGit beside the application (Windows) | `BITBRAID_MINGIT=<unpacked MinGit>` | Windows | `cargo test -p bitbraid-cli --test suite shipped_mingit::` |
| A repository inside WSL (Windows) | `BITBRAID_WSL_DISTRIBUTION=<name>` | a distribution with git and sh | `cargo test -p bitbraid-cli --test suite wsl::` |

**Recording.** With `BITBRAID_RECORD=<dir>` the live provider tests write
recordings into `<dir>`, scrubbed: people, tokens, ids, node ids and
commit ids are replaced, and a `BITBRAID_LIVE_REPO` name is recorded as
`bitbraid-live`. `BITBRAID_RECORD_FORBID=<word,word,...>` lists words
that may not appear (logins, the organisation, parts of names): a
recording holding one is refused, and with the variable set
`cargo test -p bitbraid-integrations --test suite recorded::` checks every
committed recording for them. Without it that check still looks for
tokens, addresses, pictures, ids and node ids.

## 7. Working safely

- Use an isolated target directory per agent, as an absolute path:
  `CARGO_TARGET_DIR=<checkout>/target-<name>`. Delete it once the branch is
  merged; a build directory is about 11 GB after a full gate. Several cold
  builds at once exhaust a machine's memory: build one, then copy it to
  the others.
- On Linux run every build, test and application run inside a memory cap
  of its own, so that a runaway test ends there and not the session:
  `systemd-run --user --scope -q -p MemoryMax=8G -p MemorySwapMax=2G --
  <command>`.
- Anything run by hand outside the test harnesses (the application, a
  script, git, gpg, ssh, an external tool) runs in scratch surroundings,
  never the user's: `HOME`, `XDG_CONFIG_HOME`, `XDG_CACHE_HOME` and
  `GIT_CONFIG_GLOBAL` pointed at a scratch directory; `GNUPGHOME` set as
  well, since gpg keeps its sockets under the runtime directory and a
  scratch `HOME` alone reaches the user's agent; `SSH_AUTH_SOCK` unset;
  Java programs given `-Duser.home=<scratch>`; a credential helper,
  keychain or Secret Service of the run's own, never the user's. Never
  touch the user's `~/.gnupg`, `~/.ssh` or keychain beyond reading.
- Opt-in tests (section 6) run only when you are asked to, with the
  variables you are given, against the accounts you are given.
- Others may have uncommitted work in the same tree. Run `git status`
  before staging and stage explicit paths only (never `git add -A` or
  `git commit -a`). Look at a diff in one step and commit in the next,
  never both in one command.
- Commit subjects are `area: imperative summary` (for example
  `engine: add branch.create command`). Never commit secrets, tokens or
  personal data.
- An agent on a branch of its own rebases it onto `main` and passes the
  gate there before reporting; merging and pushing are for whoever started
  it.
- New commands go in new modules; keep edits to shared files small and
  re-read a file before editing it.
- Keep every `Cargo.toml` valid at all times; write manifests atomically.
- Do not reformat files you did not otherwise change.

## 8. The website and the logo

- `brand/` holds the logo, the mark and the icon as SVG, the only copies:
  the Windows binary's icon is drawn from `icon.svg` when it is built
  (`apps/bitbraid/build.rs`), `cargo xtask bundle` and `cargo xtask
  package` draw the others from it, the application shows the logo and
  the mark as they are (`crates/ui/src/brand.rs`) and the website copies
  them. No raster of them is stored. The terms for them are at the end of
  `LICENSE`.
- `site/` is one static page: no build tools, no scripts or fonts from
  other sites, no cookies. `.github/workflows/pages.yml` publishes it to
  GitHub Pages on a push to `main` that changes `site/`, `brand/` or the
  workflow, and when a release is published. To look at it:
  `site/build.sh /tmp/bitbraid-site` and
  `python3 -m http.server -d /tmp/bitbraid-site 8000`.
- Its screenshots, which the README shows too, are taken again after a
  change that shows in them:
  `cargo build --profile dist -p bitbraid`, then
  `site/screenshots.sh target/dist/bitbraid` (what it needs is at its
  head). Look at them before committing.
- The page says what BitBraid does in its own words, never names another
  product, and claims only what the application does; change it when that
  changes.
