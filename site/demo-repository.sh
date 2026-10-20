#!/bin/sh
# Builds the repository the website's screenshots show: a small, made-up
# HTTP routing library with five authors, branches, merges, tags, a remote
# that is ahead and behind, a stash and uncommitted work. The project, its
# people and its code are invented; the addresses are example.org.
#
# usage: site/demo-repository.sh <directory>
#
# The directory must not exist. The work is done with the user's git, but
# none of the user's git configuration is read (no signing, no hooks).

set -eu

if [ $# -ne 1 ]; then
    echo "usage: $0 <directory>" >&2
    exit 2
fi
if [ -e "$1" ]; then
    echo "$1 exists; give a path that does not" >&2
    exit 1
fi
mkdir -p "$1"
root=$(cd "$1" && pwd)
repo=$root/meridian

export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
export GIT_AUTHOR_NAME GIT_AUTHOR_EMAIL GIT_COMMITTER_NAME GIT_COMMITTER_EMAIL
export GIT_AUTHOR_DATE GIT_COMMITTER_DATE

as() {
    GIT_AUTHOR_NAME=$1 GIT_AUTHOR_EMAIL=$2
    GIT_COMMITTER_NAME=$1 GIT_COMMITTER_EMAIL=$2
}
ada() { as "Ada Okafor" ada@example.org; }
mateo() { as "Mateo Ruiz" mateo@example.org; }
lin() { as "Lin Wei" lin@example.org; }
sanna() { as "Sanna Virtanen" sanna@example.org; }
priya() { as "Priya Raman" priya@example.org; }
at() { GIT_AUTHOR_DATE="$1 +0200" GIT_COMMITTER_DATE="$1 +0200"; }

commit() { git add -A && git commit -q -m "$1"; }
merge() { git merge -q --no-ff --no-edit -m "$2" "$1"; }
branch() { git switch -q -c "$1"; }
on() { git switch -q "$1"; }

git init -q -b main "$repo"
cd "$repo"
git config user.useConfigOnly false

# 0.1.0 -------------------------------------------------------------------

ada; at "2026-09-01 09:14"
cat > Cargo.toml <<'EOF'
[package]
name = "meridian"
version = "0.0.1"
edition = "2021"
description = "A small, fast HTTP router"
license = "MIT"
EOF
cat > README.md <<'EOF'
# meridian

A small, fast HTTP router for Rust.
EOF
printf '/target\n' > .gitignore
cat > src.lib.rs <<'EOF'
//! A small, fast HTTP router.

mod router;

pub use router::Router;
EOF
mkdir src && mv src.lib.rs src/lib.rs
cat > src/router.rs <<'EOF'
pub struct Router<H> {
    routes: Vec<(String, H)>,
}
EOF
commit "Initial commit"

at "2026-09-01 16:40"
cat > src/router.rs <<'EOF'
pub struct Router<H> {
    routes: Vec<(Vec<String>, H)>,
}

impl<H> Router<H> {
    pub fn new() -> Self {
        Router { routes: Vec::new() }
    }

    pub fn add(&mut self, path: &str, handler: H) {
        self.routes.push((segments(path), handler));
    }

    pub fn find(&self, path: &str) -> Option<&H> {
        let wanted = segments(path);
        self.routes
            .iter()
            .find(|(route, _)| *route == wanted)
            .map(|(_, handler)| handler)
    }
}

fn segments(path: &str) -> Vec<String> {
    path.split('/').filter(|s| !s.is_empty()).map(str::to_owned).collect()
}
EOF
commit "Match static path segments"

mateo; at "2026-09-03 11:02"
cat > src/params.rs <<'EOF'
use std::collections::HashMap;

/// The values captured by the named segments of a route.
#[derive(Debug, Default, PartialEq)]
pub struct Params(HashMap<String, String>);

impl Params {
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0.get(name).map(String::as_str)
    }

    pub(crate) fn insert(&mut self, name: &str, value: &str) {
        self.0.insert(name.to_owned(), value.to_owned());
    }
}
EOF
cat >> src/lib.rs <<'EOF'
mod params;

pub use params::Params;
EOF
commit "Capture named parameters such as /users/:id"

lin; at "2026-09-04 14:25"
mkdir tests
cat > tests/routing.rs <<'EOF'
use meridian::Router;

#[test]
fn nested_static_routes() {
    let mut router = Router::new();
    router.add("/api/users", 1);
    router.add("/api/users/active", 2);
    assert_eq!(router.find("/api/users/active"), Some(&2));
}
EOF
commit "Test nested routes and parameter capture"

ada; at "2026-09-05 10:00"
sed -i 's/^version = .*/version = "0.1.0"/' Cargo.toml
cat > CHANGELOG.md <<'EOF'
# Changelog

## 0.1.0

- Static segments and named parameters.
EOF
commit "Release 0.1.0"
git tag -a v0.1.0 -m "meridian 0.1.0"

# 0.2.0 -------------------------------------------------------------------

branch feature/wildcards
mateo; at "2026-09-08 09:30"
cat > src/wildcard.rs <<'EOF'
/// A trailing `*name` segment takes the rest of the path.
pub(crate) fn is_wildcard(segment: &str) -> bool {
    segment.starts_with('*')
}
EOF
commit "Match trailing wildcards like /static/*path"

at "2026-09-09 15:12"
cat >> src/wildcard.rs <<'EOF'

/// Static segments win over parameters, and parameters over wildcards.
pub(crate) fn rank(segment: &str) -> u8 {
    match segment.chars().next() {
        Some('*') => 2,
        Some(':') => 1,
        _ => 0,
    }
}
EOF
commit "Prefer static segments over wildcards"

on main
priya; at "2026-09-08 13:45"
cat > src/decode.rs <<'EOF'
/// Decodes `%XX` escapes in a captured value.
pub(crate) fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match (bytes[i], bytes.get(i + 1..i + 3)) {
            (b'%', Some(hex)) => match u8::from_str_radix(std::str::from_utf8(hex).unwrap_or(""), 16) {
                Ok(byte) => {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                Err(_) => out.push(b'%'),
            },
            (byte, _) => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
EOF
printf 'mod decode;\n' >> src/lib.rs
commit "Decode percent-escapes in parameters"

branch feature/middleware
sanna; at "2026-09-10 10:20"
cat > src/middleware.rs <<'EOF'
use crate::{Request, Response};

/// Runs before a handler, and may answer in its place.
pub trait Middleware: Send + Sync {
    fn handle(&self, request: &mut Request) -> Option<Response>;
}
EOF
commit "Add a Middleware trait"

at "2026-09-11 17:05"
cat >> src/middleware.rs <<'EOF'

/// Middleware in the order it was added.
#[derive(Default)]
pub struct Stack(Vec<Box<dyn Middleware>>);

impl Stack {
    pub fn push(&mut self, middleware: impl Middleware + 'static) {
        self.0.push(Box::new(middleware));
    }
}
EOF
commit "Run middleware in the order it was added"

on main
ada; at "2026-09-12 09:50"
merge feature/wildcards "Merge branch 'feature/wildcards'"

lin; at "2026-09-14 11:30"
cat > src/method.rs <<'EOF'
/// The methods a path answers to, for the `Allow` header of a 405.
pub(crate) fn allow_header(methods: &[&str]) -> String {
    methods.join(", ")
}
EOF
printf 'mod method;\n' >> src/lib.rs
commit "Answer 405 with an Allow header for known paths"

branch fix/trailing-slash
at "2026-09-15 10:10"
sed -i "s/path.split('\/').filter(|s| !s.is_empty())/path.trim_end_matches('\/').split('\/').filter(|s| !s.is_empty())/" src/router.rs
cat >> tests/routing.rs <<'EOF'

#[test]
fn trailing_slash_is_optional() {
    let mut router = Router::new();
    router.add("/docs", 1);
    assert_eq!(router.find("/docs/"), Some(&1));
}
EOF
commit "Treat a trailing slash as optional"

on main
ada; at "2026-09-15 16:00"
merge fix/trailing-slash "Merge branch 'fix/trailing-slash'"
git branch -q -d fix/trailing-slash

at "2026-09-16 10:00"
sed -i 's/^version = .*/version = "0.2.0"/' Cargo.toml
cat > CHANGELOG.md <<'EOF'
# Changelog

## 0.2.0

- Trailing wildcards, ranked below static segments and parameters.
- Percent-escapes in parameters are decoded.
- A known path with the wrong method answers 405 with an Allow header.
- A trailing slash is optional.

## 0.1.0

- Static segments and named parameters.
EOF
commit "Release 0.2.0"
git tag -a v0.2.0 -m "meridian 0.2.0"

on feature/middleware
sanna; at "2026-09-17 14:20"
merge main "Merge branch 'main' into feature/middleware"

at "2026-09-18 11:00"
mkdir -p examples
cat > examples/timing.rs <<'EOF'
use std::time::Instant;

use meridian::{Middleware, Request, Response};

/// Writes how long each request took to standard error.
struct Timing;

impl Middleware for Timing {
    fn handle(&self, request: &mut Request) -> Option<Response> {
        request.extensions.insert(Instant::now());
        None
    }
}

fn main() {}
EOF
commit "Add a request timing middleware example"

# 0.3.0 -------------------------------------------------------------------

on main
branch docs/guide
priya; at "2026-09-22 10:30"
mkdir -p docs
cat > docs/guide.md <<'EOF'
# Getting started

Add meridian to `Cargo.toml`, make a `Router`, and add a route for each
path your service answers.
EOF
commit "Write a getting started guide"

at "2026-09-23 15:45"
cat >> docs/guide.md <<'EOF'

| Pattern | Matches |
|---|---|
| `/users` | `/users` only |
| `/users/:id` | `/users/42`, with `id` = `42` |
| `/static/*path` | `/static/css/site.css`, with `path` = `css/site.css` |
EOF
commit "Add a table of route patterns to the guide"

on main
ada; at "2026-09-24 09:20"
cat > src/tree.rs <<'EOF'
/// Routes stored by their common prefixes, so a lookup reads each
/// segment of the path once.
#[derive(Default)]
pub(crate) struct Node<H> {
    segment: String,
    handler: Option<H>,
    children: Vec<Node<H>>,
}
EOF
printf 'mod tree;\n' >> src/lib.rs
commit "Store routes in a radix tree"

at "2026-09-25 13:10"
mkdir -p benches
cat > benches/lookup.rs <<'EOF'
//! Lookups in a router of 500 routes: the radix tree against the linear
//! matcher it replaced.
fn main() {}
EOF
commit "Benchmark lookups against the linear matcher"

at "2026-09-29 10:00"
merge feature/middleware "Merge branch 'feature/middleware'"

mateo; at "2026-10-01 11:30"
cat >> src/router.rs <<'EOF'

/// A router is read by every request and written only while it is built,
/// so once built it is shared between threads behind an `Arc`.
pub type Shared<H> = std::sync::Arc<Router<H>>;
EOF
commit "Share one router across threads"

branch feature/host-routing
at "2026-10-05 10:15"
cat > src/host.rs <<'EOF'
/// Routes chosen by the request's Host header before its path.
pub struct HostRouter<H> {
    hosts: Vec<(String, crate::Router<H>)>,
}
EOF
commit "Route on the Host header"

at "2026-10-07 16:30"
cat >> src/host.rs <<'EOF'

/// `*.example.org` matches every subdomain of example.org, but not
/// example.org itself.
fn host_matches(pattern: &str, host: &str) -> bool {
    match pattern.strip_prefix("*.") {
        Some(domain) => host.ends_with(domain) && host.len() > domain.len() + 1,
        None => pattern.eq_ignore_ascii_case(host),
    }
}
EOF
commit "Allow wildcard subdomains"

on main
lin; at "2026-10-09 14:05"
cat >> src/router.rs <<'EOF'

/// Two routes that differ only in the names of their parameters would
/// shadow each other; the second is refused.
#[derive(Debug)]
pub struct Conflict(pub String);
EOF
commit "Refuse routes that differ only by parameter name"

ada; at "2026-10-12 10:00"
sed -i 's/^version = .*/version = "0.3.0"/' Cargo.toml
sed -i 's/^# Changelog$/# Changelog\n\n## 0.3.0\n\n- Routes live in a radix tree.\n- Middleware, run in the order it was added.\n- Routes that would shadow each other are refused./' CHANGELOG.md
commit "Release 0.3.0"
git tag -a v0.3.0 -m "meridian 0.3.0"

on feature/host-routing
mateo; at "2026-10-14 09:40"
merge main "Merge branch 'main' into feature/host-routing"

# The remote ----------------------------------------------------------------

git init -q --bare "$root/origin.git"
git remote add origin "$root/origin.git"
for b in main feature/middleware docs/guide feature/host-routing; do
    git push -q -u origin "$b"
done
git push -q origin --tags

# On the remote only: a fix nobody here has checked out.
git switch -q --detach main
sanna; at "2026-10-13 15:20"
cat >> tests/routing.rs <<'EOF'

#[test]
fn empty_segments_are_ignored() {
    let mut router = Router::new();
    router.add("/a/b", 1);
    assert_eq!(router.find("/a//b"), Some(&1));
}
EOF
commit "Ignore empty path segments"
git push -q origin HEAD:refs/heads/fix/empty-segments

# Ahead on the remote: a review fix to the guide, and the changelog.
git switch -q --detach docs/guide
lin; at "2026-10-08 12:10"
sed -i 's/Add meridian to `Cargo.toml`/Add `meridian` to the dependencies in `Cargo.toml`/' docs/guide.md
commit "Fix the dependency line in the guide"
git push -q origin HEAD:docs/guide

git switch -q --detach main
priya; at "2026-10-16 17:25"
sed -i 's/^## 0.3.0$/## Unreleased\n\n- Empty path segments are ignored.\n\n## 0.3.0/' CHANGELOG.md
commit "Start the changelog for the next release"
git push -q origin HEAD:main

git fetch -q origin

# Local work ----------------------------------------------------------------

on feature/host-routing
mateo; at "2026-10-15 11:05"
cat > docs.host.md <<'EOF'
# Routing by host

A `HostRouter` picks a router by the request's Host header, then that
router picks a handler by the path. `*.example.org` matches every
subdomain of example.org.
EOF
mkdir -p docs && mv docs.host.md docs/host.md
commit "Document routing by host"

# A stash: an experiment set aside.
at "2026-10-16 09:30"
cat >> src/params.rs <<'EOF'

/// A parameter may carry a pattern: `/users/:id(\d+)`.
pub(crate) fn constraint(segment: &str) -> Option<&str> {
    let start = segment.find('(')?;
    segment.strip_suffix(')').map(|s| &s[start + 1..])
}
EOF
git stash push -q -m "try regex constraints on parameters"

# Uncommitted: two changes staged, three not, and a new file.
cat >> README.md <<'EOF'

Routes can be chosen by host as well as by path; see `docs/host.md`.
EOF
sed -i 's/^# Changelog$/# Changelog\n\n## Unreleased\n\n- Routing by host, with wildcard subdomains./' CHANGELOG.md
git add README.md CHANGELOG.md
printf 'mod host;\npub use host::HostRouter;\n' >> src/lib.rs
cat >> src/host.rs <<'EOF'

impl<H> HostRouter<H> {
    pub fn new() -> Self {
        HostRouter { hosts: Vec::new() }
    }

    /// The router for a host, in the order hosts were added.
    pub fn router(&self, host: &str) -> Option<&crate::Router<H>> {
        let host = host.split(':').next().unwrap_or(host);
        self.hosts
            .iter()
            .find(|(pattern, _)| host_matches(pattern, host))
            .map(|(_, router)| router)
    }
}
EOF
sed -i 's/^version = .*/version = "0.4.0-dev"/' Cargo.toml
cat > tests/host.rs <<'EOF'
use meridian::{HostRouter, Router};

#[test]
fn wildcard_subdomains() {
    let mut api = Router::new();
    api.add("/status", 1);
    let mut hosts = HostRouter::new();
    hosts.add("*.example.org", api);
    assert!(hosts.router("eu.example.org").is_some());
    assert!(hosts.router("example.org").is_none());
}
EOF

echo "$repo"
