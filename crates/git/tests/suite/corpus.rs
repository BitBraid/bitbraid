//! The oracle comparison on real repositories. Ignored by default: the
//! repositories are other people's and are fetched once by
//! `cargo xtask corpus`, which runs this file in release mode and names
//! them in `BITBRAID_CORPUS_REPOS` (`name<TAB>path` per line).
//!
//! What is compared with git, per repository:
//!
//! - references and the snapshot (branches, tags peeled to the end, HEAD;
//!   remote branches, the worktree and a stash on a scratch clone);
//! - the whole graph: the set of commits, their parents and dates, the
//!   order (a valid `--date-order`, position by position against git's),
//!   and the lanes (every parent edge drawn as one unbroken lane, no lane
//!   leading nowhere);
//! - every commit's author, committer, dates and subject;
//! - a sample of diffs (paths, binary files, line counts, hunk headers,
//!   renames; the hunks rebuild the new blob from the old one);
//! - a sample of blames and file histories;
//! - statuses of a scratch clone in several states.
//!
//! Where git and this crate may differ by design, the test counts and
//! prints, and holds a floor; everything else is exact.
//!
//! Sources: git-rev-list(1) ("Commit Ordering", "History Simplification"),
//! git-for-each-ref(1), git-diff-tree(1), git-blame(1) (`--porcelain`),
//! git-status(1) ("Porcelain Format Version 2"), git-log(1).
#![allow(clippy::print_stdout)]

use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::{Path, PathBuf},
    time::Instant,
};

use bitbraid_git::{
    DiffOptions, DiffSide, GraphRow, HistoryOptions, LogOptions, Oid, Repository, apply_hunks,
    blame, diff_trees, file_history, layout, log, snapshot, status, walk,
};
use bitbraid_testkit::{Git, fixtures_out_dir, scratch_root};

/// The tips the graph starts from, as git names them.
const TIPS: [&str; 4] = ["HEAD", "--branches", "--remotes", "--tags"];

fn corpora() -> Vec<(String, PathBuf)> {
    let listed = std::env::var("BITBRAID_CORPUS_REPOS").unwrap_or_default();
    let found: Vec<(String, PathBuf)> = listed
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .map(|(name, path)| (name.to_owned(), PathBuf::from(path)))
        .collect();
    if found.is_empty() {
        println!("CORPUS skipped: no repositories named (run `cargo xtask corpus`)");
    }
    found
}

fn oracle(path: &Path) -> Git {
    let git = Git::new(path, fixtures_out_dir().join(".home"));
    git.ensure_home().unwrap();
    git
}

/// NUL-separated output of git, lossless for odd paths as far as UTF-8 goes.
fn fields(git: &Git, args: &[&str]) -> Vec<String> {
    let out = git.run(args).unwrap();
    out.split('\0')
        .filter(|field| !field.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
#[ignore = "needs the corpus; run via cargo xtask corpus"]
fn references_match_git() {
    for (name, path) in corpora() {
        let repo = Repository::open(&path).unwrap();
        let git = oracle(&path);
        let snap = snapshot(&repo).unwrap();

        let want: BTreeMap<String, String> = git
            .lines([
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/heads",
            ])
            .unwrap()
            .into_iter()
            .map(|line| {
                let (full, id) = line.split_once(' ').unwrap();
                (full.to_owned(), id.to_owned())
            })
            .collect();
        let got: BTreeMap<String, String> = snap
            .branches
            .iter()
            .map(|b| (b.full_name.clone(), b.target.to_hex()))
            .collect();
        assert_eq!(got, want, "{name}: branches");
        for branch in &snap.branches {
            assert_eq!(
                branch.full_name.strip_prefix("refs/heads/"),
                Some(branch.name.as_str()),
                "{name}: short name"
            );
        }

        // Tags: what each comes to once every tag on the way is followed,
        // and whether the reference holds a tag object.
        let listed = git
            .lines([
                "for-each-ref",
                "--format=%(refname) %(objecttype) %(objectname)",
                "refs/tags",
            ])
            .unwrap();
        let mut want_tags: BTreeMap<String, (String, bool, Option<String>)> = BTreeMap::new();
        for chunk in listed.chunks(400) {
            let mut args = vec!["rev-parse".to_owned()];
            args.extend(chunk.iter().map(|line| {
                let full = line.split(' ').next().unwrap();
                format!("{full}^{{}}")
            }));
            let peeled = git.lines(&args).unwrap();
            assert_eq!(peeled.len(), chunk.len(), "{name}: every tag peels");
            for (line, peeled) in chunk.iter().zip(peeled) {
                let parts: Vec<&str> = line.split(' ').collect();
                let annotated = parts[1] == "tag";
                want_tags.insert(
                    parts[0].strip_prefix("refs/tags/").unwrap().to_owned(),
                    (peeled, annotated, annotated.then(|| parts[2].to_owned())),
                );
            }
        }
        let got_tags: BTreeMap<String, (String, bool, Option<String>)> = snap
            .tags
            .iter()
            .map(|t| {
                (
                    t.name.clone(),
                    (
                        t.target.to_hex(),
                        t.annotated,
                        t.tag_object.map(|id| id.to_hex()),
                    ),
                )
            })
            .collect();
        assert_eq!(got_tags, want_tags, "{name}: tags");

        let head_branch = git
            .run(["symbolic-ref", "--short", "HEAD"])
            .ok()
            .map(|s| s.trim().to_owned());
        assert_eq!(snap.head.branch, head_branch, "{name}: HEAD branch");
        assert_eq!(
            snap.head.target.map(|id| id.to_hex()),
            git.rev_parse("HEAD").ok(),
            "{name}: HEAD target"
        );
        if repo.is_bare() {
            assert!(snap.worktrees.is_empty(), "{name}: bare, no worktree");
        }
        println!(
            "CORPUS {name} references: branches={} tags={} (annotated={}) head={:?} exact",
            got.len(),
            got_tags.len(),
            got_tags.values().filter(|t| t.1).count(),
            snap.head.branch
        );
    }
}

/// What is wrong with `order` as a `--date-order`: commits shown before one
/// of their children, and commits shown while a newer one was ready (all
/// its children shown). Ties in time may fall either way. Returns both
/// counts and the first offender.
fn date_order_faults(
    order: &[usize],
    parents: &[Vec<usize>],
    time: &[i64],
) -> (usize, usize, Option<usize>) {
    let n = parents.len();
    let mut pending = vec![0usize; n];
    for of_one in parents {
        for parent in of_one.iter().collect::<BTreeSet<_>>() {
            pending[*parent] += 1;
        }
    }
    let mut ready: BTreeMap<i64, usize> = BTreeMap::new();
    for (commit, waiting) in pending.iter().enumerate() {
        if *waiting == 0 {
            *ready.entry(time[commit]).or_default() += 1;
        }
    }
    let mut shown = vec![false; n];
    let (mut early, mut stale, mut first) = (0usize, 0usize, None);
    for &commit in order {
        shown[commit] = true;
        if pending[commit] != 0 {
            early += 1;
            first.get_or_insert(commit);
        } else {
            let newest = ready.keys().next_back().copied().unwrap_or(i64::MIN);
            if time[commit] != newest {
                stale += 1;
                first.get_or_insert(commit);
            }
            if let Some(count) = ready.get_mut(&time[commit]) {
                *count -= 1;
                if *count == 0 {
                    ready.remove(&time[commit]);
                }
            }
        }
        for parent in parents[commit].iter().collect::<BTreeSet<_>>() {
            pending[*parent] -= 1;
            if pending[*parent] == 0 && !shown[*parent] {
                *ready.entry(time[*parent]).or_default() += 1;
            }
        }
    }
    (early, stale, first)
}

/// Follow the lanes the rows draw and say where they do not agree with the
/// parents git reports: a lane is a column heading for one commit, from the
/// row of a child down to the row of that parent.
fn lane_faults(rows: &[GraphRow], order: &[usize], parents: &[Vec<usize>]) -> Vec<String> {
    let mut faults = Vec::new();
    let mut heading: BTreeMap<u16, usize> = BTreeMap::new();
    for (at, (row, &commit)) in rows.iter().zip(order).enumerate() {
        let mut fault = |what: &str| {
            if faults.len() < 20 {
                faults.push(format!("row {at}: {what}"));
            }
        };
        // Lanes heading for this commit end here: its own, and the ones
        // drawn curving in.
        let arriving: Vec<u16> = heading
            .iter()
            .filter(|(_, target)| **target == commit)
            .map(|(slot, _)| *slot)
            .collect();
        let mut drawn: Vec<u16> = row.merges_in.iter().map(|lane| lane.slot).collect();
        if arriving.contains(&row.lane) {
            drawn.push(row.lane);
        } else if heading.contains_key(&row.lane) {
            fault("the node sits on a lane that leads to another commit");
        }
        drawn.sort_unstable();
        if drawn != arriving {
            fault(&format!(
                "lanes {arriving:?} arrive, {drawn:?} are drawn arriving"
            ));
        }
        for slot in &arriving {
            heading.remove(slot);
        }
        // Everything else passes straight through.
        let straight: Vec<u16> = row.straight.iter().map(|lane| lane.slot).collect();
        let passing: Vec<u16> = heading.keys().copied().collect();
        if straight != passing {
            fault(&format!(
                "lanes {passing:?} pass, {straight:?} are drawn passing"
            ));
        }
        // Lines to the parents leave the node.
        let of_commit = &parents[commit];
        if row.continues != !of_commit.is_empty() {
            fault("the node's lane continues without a parent, or stops with one");
        }
        if let Some(first) = of_commit.first() {
            heading.insert(row.lane, *first);
        }
        if row.forks.len() != of_commit.len().saturating_sub(1) {
            fault(&format!(
                "{} further parents, {} lines to them",
                of_commit.len().saturating_sub(1),
                row.forks.len()
            ));
        }
        for (fork, parent) in row.forks.iter().zip(of_commit.iter().skip(1)) {
            match heading.get(&fork.slot) {
                Some(target) if target == parent => {}
                Some(_) => fault("a line to a parent joins a lane that leads elsewhere"),
                None => {
                    heading.insert(fork.slot, *parent);
                }
            }
        }
        let widest = heading
            .keys()
            .next_back()
            .copied()
            .unwrap_or(0)
            .max(row.lane);
        if row.width <= widest {
            fault("the row is narrower than its lanes");
        }
        if row.color != u32::from(row.lane) {
            fault("the node's colour is not its lane's");
        }
        for slot in heading.keys() {
            if !row.leaves(*slot) {
                fault(&format!("lane {slot} is not reported as leaving the row"));
            }
        }
    }
    if !heading.is_empty() {
        faults.push(format!("{} lanes never reach their commit", heading.len()));
    }
    faults
}

#[test]
#[ignore = "needs the corpus; run via cargo xtask corpus"]
fn the_whole_graph_matches_git_log() {
    for (name, path) in corpora() {
        let repo = Repository::open(&path).unwrap();
        let git = oracle(&path);

        let t = Instant::now();
        let walked = walk(&repo, &LogOptions::default()).unwrap();
        let walk_time = t.elapsed();
        let t = Instant::now();
        let rows = layout(&walked);
        let layout_time = t.elapsed();

        // git: "<time> <commit> <parents>" per line, in its date order.
        let mut args = vec!["rev-list", "--date-order", "--parents", "--timestamp"];
        args.extend(TIPS);
        let lines = git.lines(&args).unwrap();
        let mut index: HashMap<String, usize> = HashMap::with_capacity(lines.len());
        let mut time = Vec::with_capacity(lines.len());
        let mut parent_names: Vec<Vec<String>> = Vec::with_capacity(lines.len());
        let mut names: Vec<String> = Vec::with_capacity(lines.len());
        for (at, line) in lines.iter().enumerate() {
            let mut parts = line.split(' ');
            time.push(parts.next().unwrap().parse::<i64>().unwrap());
            let id = parts.next().unwrap().to_owned();
            index.insert(id.clone(), at);
            names.push(id);
            parent_names.push(parts.map(str::to_owned).collect());
        }
        let parents: Vec<Vec<usize>> = parent_names
            .iter()
            .map(|of_one| of_one.iter().map(|p| index[p]).collect())
            .collect();

        // The same commits, with the same parents and the same dates.
        assert_eq!(walked.len(), lines.len(), "{name}: number of commits");
        let mut ours_order = Vec::with_capacity(walked.len());
        for commit in &walked {
            let hex = commit.id.to_hex();
            let at = *index
                .get(&hex)
                .unwrap_or_else(|| panic!("{name}: {hex} is not in git's list"));
            let ours: Vec<String> = commit.parents.iter().map(Oid::to_hex).collect();
            assert_eq!(ours, parent_names[at], "{name}: parents of {hex}");
            assert_eq!(commit.seconds, Some(time[at]), "{name}: date of {hex}");
            ours_order.push(at);
        }
        assert_eq!(
            ours_order.iter().collect::<BTreeSet<_>>().len(),
            walked.len(),
            "{name}: no commit twice"
        );

        // The order. Git's own must pass the check, or the check is wrong.
        let gits_order: Vec<usize> = (0..lines.len()).collect();
        assert_eq!(
            date_order_faults(&gits_order, &parents, &time),
            (0, 0, None),
            "{name}: git's order is a date order as this test understands it"
        );
        let (early, stale, first) = date_order_faults(&ours_order, &parents, &time);
        let differing = ours_order
            .iter()
            .enumerate()
            .filter(|(at, commit)| at != *commit)
            .count();
        let ties = {
            let mut seen: HashMap<i64, usize> = HashMap::new();
            for t in &time {
                *seen.entry(*t).or_default() += 1;
            }
            seen.values().filter(|n| **n > 1).copied().sum::<usize>()
        };
        let merges = parents.iter().filter(|p| p.len() > 1).count();
        let octopus = parents.iter().filter(|p| p.len() > 2).count();
        let roots = parents.iter().filter(|p| p.is_empty()).count();
        let width = rows.iter().map(|r| r.width).max().unwrap_or(0);
        println!(
            "CORPUS {name} graph: commits={} merges={merges} octopus={octopus} roots={roots} \
             walk={walk_time:?} layout={layout_time:?} lanes_wide={width}",
            walked.len()
        );
        println!(
            "CORPUS {name} order: parent_before_child={early} newer_commit_was_ready={stale} \
             positions_differing_from_git={differing} ({:.2}%) commits_sharing_a_date={ties}",
            differing as f64 * 100.0 / walked.len().max(1) as f64
        );
        assert_eq!(
            (early, stale),
            (0, 0),
            "{name}: not a date order; first at {:?}",
            first.map(|at| &names[at])
        );
        // Commits of one date may come either way and it is still a date
        // order; they are held to git's way all the same.
        assert_eq!(differing, 0, "{name}: not git's order among equal dates");

        // The lanes.
        assert_eq!(rows.len(), walked.len());
        let faults = lane_faults(&rows, &ours_order, &parents);
        println!("CORPUS {name} lanes: faults={}", faults.len());
        assert!(faults.is_empty(), "{name}: lanes\n{}", faults.join("\n"));
    }
}

/// `+0130` and the like, as seconds east of UTC.
fn offset_seconds(raw: &str) -> Option<i32> {
    let (sign, digits) = match raw.as_bytes().first()? {
        b'-' => (-1, raw.get(1..)?),
        b'+' => (1, raw.get(1..)?),
        _ => (1, raw),
    };
    let number: i32 = digits.parse().ok()?;
    Some(sign * ((number / 100) * 3600 + (number % 100) * 60))
}

#[test]
#[ignore = "needs the corpus; run via cargo xtask corpus"]
fn every_commit_reads_as_git_shows_it() {
    for (name, path) in corpora() {
        let repo = Repository::open(&path).unwrap();
        let git = oracle(&path);
        let t = Instant::now();
        let commits = log(&repo, &LogOptions::default()).unwrap();
        let took = t.elapsed();
        let mut args = vec![
            "log",
            "--date=raw",
            "--format=%H%x00%an%x00%ae%x00%ad%x00%cn%x00%ce%x00%cd%x00%s",
        ];
        args.extend(TIPS);
        let out = git.run(&args).unwrap();
        let mut want: HashMap<&str, Vec<&str>> = HashMap::new();
        for line in out.split('\n').filter(|line| !line.is_empty()) {
            let parts: Vec<&str> = line.split('\0').collect();
            assert_eq!(parts.len(), 8, "{name}: {line:?}");
            want.insert(parts[0], parts);
        }
        assert_eq!(commits.len(), want.len(), "{name}: number of commits");
        let mut wrong: BTreeMap<&str, Vec<String>> = BTreeMap::new();
        for commit in &commits {
            let hex = commit.id.to_hex();
            let parts = &want[hex.as_str()];
            let date = |raw: &str| -> (i64, Option<i32>) {
                let (seconds, zone) = raw.split_once(' ').unwrap_or((raw, "+0000"));
                (seconds.parse().unwrap_or(0), offset_seconds(zone))
            };
            let checks: [(&str, bool); 7] = [
                ("author name", commit.author.name == parts[1]),
                ("author email", commit.author.email == parts[2]),
                (
                    "author date",
                    (commit.author.seconds, Some(commit.author.offset)) == date(parts[3]),
                ),
                ("committer name", commit.committer.name == parts[4]),
                ("committer email", commit.committer.email == parts[5]),
                (
                    "committer date",
                    (commit.committer.seconds, Some(commit.committer.offset)) == date(parts[6]),
                ),
                ("subject", commit.summary.trim() == parts[7].trim()),
            ];
            for (what, same) in checks {
                if !same {
                    wrong.entry(what).or_default().push(hex.clone());
                }
            }
        }
        println!(
            "CORPUS {name} commits: read={} in {took:?} differing={}",
            commits.len(),
            wrong
                .iter()
                .map(|(what, ids)| format!("{what}:{}", ids.len()))
                .collect::<Vec<_>>()
                .join(",")
        );
        for (what, ids) in &wrong {
            for id in ids.iter().take(5) {
                let ours = commits.iter().find(|c| c.id.to_hex() == *id).unwrap();
                println!(
                    "CORPUS {name} commits:   {what} of {id}: ours {:?} / {:?} / {:?}, git {:?}",
                    ours.author,
                    ours.committer,
                    ours.summary,
                    want[id.as_str()]
                );
            }
        }
        assert!(wrong.is_empty(), "{name}: commits read differently");
    }
}

/// The empty tree, which a root commit is compared with.
fn empty_tree(git: &Git) -> String {
    git.run(["hash-object", "-t", "tree", "/dev/null"])
        .unwrap()
        .trim()
        .to_owned()
}

/// Hunk headers per path out of `git diff-tree -p`, cut after the ranges.
fn git_hunk_headers(patch: &str) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut path: Option<String> = None;
    let mut in_header = false;
    let mut old_path: Option<String> = None;
    for line in patch.split('\n') {
        if line.starts_with("diff --git ") {
            in_header = true;
            path = None;
            old_path = None;
            continue;
        }
        if in_header {
            if let Some(rest) = line.strip_prefix("--- ") {
                old_path = rest.strip_prefix("a/").map(str::to_owned);
                continue;
            }
            if let Some(rest) = line.strip_prefix("+++ ") {
                path = rest
                    .strip_prefix("b/")
                    .map(str::to_owned)
                    .or_else(|| old_path.clone());
                continue;
            }
        }
        if let Some(rest) = line.strip_prefix("@@")
            && let Some((ranges, _)) = rest.split_once("@@")
        {
            in_header = false;
            if let Some(path) = &path {
                out.entry(path.clone())
                    .or_default()
                    .push(format!("@@{ranges}@@"));
            }
        }
    }
    out
}

#[test]
#[ignore = "needs the corpus; run via cargo xtask corpus"]
fn a_sample_of_diffs_matches_git() {
    for (name, path) in corpora() {
        let repo = Repository::open(&path).unwrap();
        let git = oracle(&path);
        let walked = walk(&repo, &LogOptions::default()).unwrap();
        let empty = empty_tree(&git);

        // Every so-manieth commit, and every octopus merge and root.
        let stride = (walked.len() / 150).max(1);
        let sample: Vec<&bitbraid_git::GraphCommit> = walked
            .iter()
            .enumerate()
            .filter(|(at, c)| at % stride == 0 || c.parents.len() > 2 || c.parents.is_empty())
            .map(|(_, c)| c)
            .take(260)
            .collect();

        let (mut files, mut binaries, mut count_differs, mut header_differs) = (0, 0, 0, 0);
        let (mut rebuilt, mut not_text) = (0, 0);
        let (mut rename_commits, mut rename_differs) = (0, 0);
        let mut shown = 0;
        let t = Instant::now();
        for commit in &sample {
            let new_hex = commit.id.to_hex();
            let (old_side, old_hex) = match commit.parents.first() {
                Some(parent) => (DiffSide::Commit(*parent), parent.to_hex()),
                None => (DiffSide::Empty, empty.clone()),
            };
            let plain = DiffOptions {
                detect_renames: false,
                ..Default::default()
            };
            let ours = diff_trees(&repo, &old_side, &DiffSide::Commit(commit.id), &plain).unwrap();

            // Paths, binary files and line counts.
            let numstat = fields(
                &git,
                &[
                    "diff-tree",
                    "-r",
                    "--no-renames",
                    "--numstat",
                    "-z",
                    &old_hex,
                    &new_hex,
                ],
            );
            let mut want: BTreeMap<String, Option<(u32, u32)>> = BTreeMap::new();
            for record in &numstat {
                let parts: Vec<&str> = record.splitn(3, '\t').collect();
                let counts = match (parts[0].parse(), parts[1].parse()) {
                    (Ok(added), Ok(removed)) => Some((added, removed)),
                    _ => None, // "-": binary
                };
                want.insert(parts[2].to_owned(), counts);
            }
            let got_paths: Vec<&str> = ours.iter().map(|f| f.path.as_str()).collect();
            let want_paths: Vec<&str> = want.keys().map(String::as_str).collect();
            assert_eq!(got_paths, want_paths, "{name}: paths changed by {new_hex}");

            let patch = git
                .run([
                    "-c",
                    "core.quotePath=false",
                    "diff-tree",
                    "-r",
                    "-p",
                    "-U3",
                    "--no-renames",
                    "--no-color",
                    "--no-ext-diff",
                    &old_hex,
                    &new_hex,
                ])
                .unwrap();
            let want_headers = git_hunk_headers(&patch);

            for file in &ours {
                files += 1;
                let counts = want[&file.path];
                if file.is_submodule() {
                    continue;
                }
                assert_eq!(
                    file.binary,
                    counts.is_none(),
                    "{name}: {} in {new_hex} is binary to one and text to the other",
                    file.path
                );
                if file.binary {
                    binaries += 1;
                    continue;
                }
                if counts != Some((file.additions, file.deletions)) {
                    count_differs += 1;
                    if shown < 8 {
                        shown += 1;
                        println!(
                            "CORPUS {name} diffs:   {} in {new_hex}: ours +{} -{}, git {counts:?}",
                            file.path, file.additions, file.deletions
                        );
                    }
                }
                let got_headers: Vec<String> = file.hunks.iter().map(|h| h.header()).collect();
                let empty_list = Vec::new();
                if &got_headers != want_headers.get(&file.path).unwrap_or(&empty_list) {
                    header_differs += 1;
                }
                // The hunks turn the old text into the new one.
                let text = |id: Option<Oid>| -> Option<String> {
                    match id {
                        Some(id) => String::from_utf8(
                            repo.gix().find_blob(*id.as_gix()).unwrap().data.clone(),
                        )
                        .ok(),
                        None => Some(String::new()),
                    }
                };
                match (text(file.old_id), text(file.new_id)) {
                    (Some(old), Some(new)) => {
                        assert_eq!(
                            apply_hunks(&old, &file.hunks).as_deref(),
                            Some(new.as_str()),
                            "{name}: hunks of {} in {new_hex} do not rebuild it",
                            file.path
                        );
                        rebuilt += 1;
                    }
                    _ => not_text += 1,
                }
            }

            // Renames, where there is something to pair.
            let status = fields(
                &git,
                &[
                    "diff-tree",
                    "-r",
                    "-M",
                    "--name-status",
                    "-z",
                    &old_hex,
                    &new_hex,
                ],
            );
            let mut want_moves: BTreeSet<(String, String)> = BTreeSet::new();
            let mut it = status.iter();
            while let Some(letter) = it.next() {
                if letter.starts_with('R') {
                    let from = it.next().unwrap().clone();
                    let to = it.next().unwrap().clone();
                    want_moves.insert((from, to));
                } else {
                    it.next();
                }
            }
            let with_renames = diff_trees(
                &repo,
                &old_side,
                &DiffSide::Commit(commit.id),
                &DiffOptions::default(),
            )
            .unwrap();
            let got_moves: BTreeSet<(String, String)> = with_renames
                .iter()
                .filter(|f| f.kind == bitbraid_git::diff::FileChangeKind::Renamed)
                .map(|f| (f.old_path.clone().unwrap_or_default(), f.path.clone()))
                .collect();
            if !want_moves.is_empty() || !got_moves.is_empty() {
                rename_commits += 1;
                if want_moves != got_moves {
                    rename_differs += 1;
                    if shown < 16 {
                        shown += 1;
                        let only_git: Vec<_> = want_moves.difference(&got_moves).take(3).collect();
                        let only_ours: Vec<_> = got_moves.difference(&want_moves).take(3).collect();
                        println!(
                            "CORPUS {name} diffs:   renames in {new_hex}: git {} ours {}; only git {only_git:?}; only ours {only_ours:?}",
                            want_moves.len(),
                            got_moves.len()
                        );
                    }
                }
            }
        }
        println!(
            "CORPUS {name} diffs: commits={} files={files} binary={binaries} rebuilt={rebuilt} \
             not_utf8={not_text} line_counts_differing={count_differs} hunk_headers_differing={header_differs} \
             commits_with_renames={rename_commits} of_which_paired_differently={rename_differs} took={:?}",
            sample.len(),
            t.elapsed()
        );
        // Which lines are called changed is the diff algorithm's choice
        // where a file allows several shortest answers; the floor says how
        // often that may show.
        assert!(
            count_differs * 100 <= files,
            "{name}: line counts differ from git's in more than 1% of the files"
        );
        assert!(
            header_differs * 100 <= files * 3,
            "{name}: hunk headers differ from git's in more than 3% of the files"
        );
        assert!(
            rename_differs * 10 <= rename_commits.max(10),
            "{name}: renames paired differently in more than a tenth of the commits with any"
        );
    }
}

/// Text files of HEAD, spread over the tree: `(path, size)`.
fn sample_files(git: &Git, how_many: usize) -> Vec<String> {
    let listed = fields(git, &["ls-tree", "-r", "-l", "-z", "HEAD"]);
    let text = [
        "c", "h", "rs", "sh", "txt", "md", "toml", "py", "pl", "adoc", "json", "yml", "po",
    ];
    let candidates: Vec<String> = listed
        .iter()
        .filter_map(|record| {
            let (meta, path) = record.split_once('\t')?;
            let mut meta = meta.split_whitespace();
            let mode = meta.next()?;
            let size: u64 = meta.nth(2)?.parse().ok()?;
            let extension = Path::new(path).extension()?.to_str()?;
            (mode.starts_with("100") && text.contains(&extension) && (200..80_000).contains(&size))
                .then(|| path.to_owned())
        })
        .collect();
    let stride = (candidates.len() / how_many.max(1)).max(1);
    candidates
        .into_iter()
        .step_by(stride)
        .take(how_many)
        .collect()
}

#[test]
#[ignore = "needs the corpus; run via cargo xtask corpus"]
fn a_sample_of_blames_matches_git() {
    for (name, path) in corpora() {
        let repo = Repository::open(&path).unwrap();
        let git = oracle(&path);
        let files = sample_files(&git, 24);
        let (mut lines, mut differing, mut files_differing) = (0usize, 0usize, 0usize);
        let (mut ours_time, mut gits_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        for file in &files {
            let t = Instant::now();
            let ours = blame(&repo, file, None).unwrap();
            ours_time += t.elapsed();
            let t = Instant::now();
            let porcelain = git
                .run(["blame", "--porcelain", "HEAD", "--", file.as_str()])
                .unwrap();
            gits_time += t.elapsed();
            let mut want: Vec<(u32, String, u32)> = Vec::new();
            for line in porcelain.split('\n') {
                let parts: Vec<&str> = line.split(' ').collect();
                if parts.len() >= 3
                    && parts[0].len() == 40
                    && parts[0].bytes().all(|b| b.is_ascii_hexdigit())
                {
                    want.push((
                        parts[2].parse().unwrap(),
                        parts[0].to_owned(),
                        parts[1].parse().unwrap(),
                    ));
                }
            }
            want.sort();
            let got: Vec<(u32, String, u32)> = ours
                .iter()
                .map(|l| (l.line, l.commit.to_hex(), l.source_line))
                .collect();
            assert_eq!(got.len(), want.len(), "{name}: lines of {file}");
            let wrong = got
                .iter()
                .zip(&want)
                .filter(|(ours, gits)| ours.1 != gits.1)
                .count();
            lines += got.len();
            differing += wrong;
            if wrong > 0 {
                files_differing += 1;
                println!(
                    "CORPUS {name} blames:   {file}: {wrong} of {} lines name another commit",
                    got.len()
                );
            }
        }
        println!(
            "CORPUS {name} blames: files={} lines={lines} files_differing={files_differing} \
             lines_naming_another_commit={differing} ({:.2}%) ours={ours_time:?} git={gits_time:?}",
            files.len(),
            differing as f64 * 100.0 / lines.max(1) as f64
        );
        assert!(
            differing * 100 <= lines * 2,
            "{name}: more than 2% of the blamed lines name another commit than git does"
        );
    }
}

#[test]
#[ignore = "needs the corpus; run via cargo xtask corpus"]
fn a_sample_of_file_histories_matches_git() {
    for (name, path) in corpora() {
        let repo = Repository::open(&path).unwrap();
        let git = oracle(&path);
        let files = sample_files(&git, 24);
        let (mut exact, mut same_set, mut differing) = (0usize, 0usize, 0usize);
        let (mut follow_exact, mut follow_differing) = (0usize, 0usize);
        let (mut ours_time, mut gits_time) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
        let (mut ours_followed, mut gits_followed) = (ours_time, gits_time);
        for file in &files {
            let t = Instant::now();
            let plain = file_history(
                &repo,
                file,
                &HistoryOptions {
                    follow: false,
                    ..Default::default()
                },
            )
            .unwrap();
            ours_time += t.elapsed();
            let got: Vec<String> = plain.iter().map(|e| e.commit.id.to_hex()).collect();
            let t = Instant::now();
            let want = git
                .lines(["log", "--format=%H", "HEAD", "--", file.as_str()])
                .unwrap();
            gits_time += t.elapsed();
            if got == want {
                exact += 1;
            } else if got.iter().collect::<BTreeSet<_>>() == want.iter().collect::<BTreeSet<_>>() {
                same_set += 1;
            } else {
                differing += 1;
                let ours: BTreeSet<&String> = got.iter().collect();
                let gits: BTreeSet<&String> = want.iter().collect();
                println!(
                    "CORPUS {name} histories:   {file}: git {} ours {}; only git {:?}; only ours {:?}",
                    want.len(),
                    got.len(),
                    gits.difference(&ours).take(3).collect::<Vec<_>>(),
                    ours.difference(&gits).take(3).collect::<Vec<_>>()
                );
            }

            let t = Instant::now();
            let followed = file_history(&repo, file, &HistoryOptions::default()).unwrap();
            ours_followed += t.elapsed();
            let got: Vec<String> = followed.iter().map(|e| e.commit.id.to_hex()).collect();
            let t = Instant::now();
            let want = git
                .lines([
                    "log",
                    "--follow",
                    "--format=%H",
                    "HEAD",
                    "--",
                    file.as_str(),
                ])
                .unwrap();
            gits_followed += t.elapsed();
            if got == want {
                follow_exact += 1;
            } else {
                follow_differing += 1;
                let ours: BTreeSet<&String> = got.iter().collect();
                let gits: BTreeSet<&String> = want.iter().collect();
                println!(
                    "CORPUS {name} histories:   {file} followed: git {} ours {}; only git {:?}; only ours {:?}",
                    want.len(),
                    got.len(),
                    gits.difference(&ours).take(3).collect::<Vec<_>>(),
                    ours.difference(&gits).take(3).collect::<Vec<_>>()
                );
            }
        }
        println!(
            "CORPUS {name} histories: files={} exact={exact} same_commits_other_order={same_set} \
             differing={differing} followed_exact={follow_exact} followed_differing={follow_differing} \
             ours={ours_time:?} git={gits_time:?} followed_ours={ours_followed:?} followed_git={gits_followed:?}",
            files.len()
        );
        assert_eq!(
            differing, 0,
            "{name}: file histories list other commits than git log"
        );
        assert_eq!(same_set, 0, "{name}: file histories in another order");
        assert!(
            follow_differing * 10 <= files.len(),
            "{name}: more than a tenth of the followed histories differ from git log --follow"
        );
    }
}

/// `git status --porcelain=v2 -z`: path -> (staged, unstaged, moved from).
fn porcelain(git: &Git) -> BTreeMap<String, (char, char, Option<String>)> {
    let out = git
        .run(["status", "--porcelain=v2", "-z", "--untracked-files=all"])
        .unwrap();
    let mut entries = BTreeMap::new();
    let mut records = out.split('\0');
    while let Some(record) = records.next() {
        let (tag, rest) = record.split_once(' ').unwrap_or((record, ""));
        match tag {
            "1" => {
                let parts: Vec<&str> = rest.splitn(8, ' ').collect();
                let xy: Vec<char> = parts[0].chars().collect();
                entries.insert(parts[7].to_owned(), (xy[0], xy[1], None));
            }
            "2" => {
                let parts: Vec<&str> = rest.splitn(9, ' ').collect();
                let xy: Vec<char> = parts[0].chars().collect();
                let from = records.next().unwrap().to_owned();
                entries.insert(parts[8].to_owned(), (xy[0], xy[1], Some(from)));
            }
            "u" => {
                let parts: Vec<&str> = rest.splitn(10, ' ').collect();
                entries.insert(parts[9].to_owned(), ('U', 'U', None));
            }
            "?" => {
                entries.insert(rest.to_owned(), ('.', '?', None));
            }
            _ => {}
        }
    }
    entries
}

fn ours(repo: &Repository, what: &str) -> BTreeMap<String, (char, char, Option<String>)> {
    status(repo)
        .unwrap_or_else(|err| panic!("status failed, {what}: {err}"))
        .into_iter()
        .map(|e| {
            if e.conflict {
                return (e.path, ('U', 'U', None));
            }
            (
                e.path,
                (
                    e.staged.map(|k| k.letter()).unwrap_or('.'),
                    e.unstaged.map(|k| k.letter()).unwrap_or('.'),
                    e.original_path,
                ),
            )
        })
        .collect()
}

/// A scratch clone with a working tree, outside every repository, sharing
/// the corpus repository's objects. Removed when dropped.
struct WorkClone {
    path: PathBuf,
}

impl WorkClone {
    fn of(name: &str, source: &Path) -> Self {
        let root = scratch_root();
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join(format!("corpus-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        oracle(&root)
            .run([
                "clone".as_ref(),
                "--quiet".as_ref(),
                "--shared".as_ref(),
                source.as_os_str(),
                path.as_os_str(),
            ])
            .unwrap();
        Self { path }
    }

    fn write(&self, rel: &str, content: &[u8]) {
        let path = self.path.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn append(&self, rel: &str, content: &str) {
        let path = self.path.join(rel);
        let mut bytes = std::fs::read(&path).unwrap();
        bytes.extend_from_slice(content.as_bytes());
        std::fs::write(path, bytes).unwrap();
    }
}

impl Drop for WorkClone {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[test]
#[ignore = "needs the corpus; run via cargo xtask corpus"]
fn statuses_of_a_scratch_clone_match_git() {
    for (name, path) in corpora() {
        let clone = WorkClone::of(&name, &path);
        let git = oracle(&clone.path);
        let repo = Repository::open(&clone.path).unwrap();
        let mut states = 0;
        let mut compare = |what: &str| {
            let (got, want) = (ours(&repo, what), porcelain(&git));
            let only_ours: Vec<_> = got
                .iter()
                .filter(|(k, v)| want.get(*k) != Some(v))
                .collect();
            let only_git: Vec<_> = want
                .iter()
                .filter(|(k, v)| got.get(*k) != Some(v))
                .collect();
            assert!(
                only_ours.is_empty() && only_git.is_empty(),
                "{name}: status, {what}\n ours: {only_ours:?}\n git:  {only_git:?}"
            );
            states += 1;
            got.len()
        };

        // As cloned: clean, with remote branches.
        assert_eq!(compare("fresh clone"), 0, "{name}: a fresh clone is clean");
        let snap = snapshot(&repo).unwrap();
        let want_remote: BTreeMap<String, String> = git
            .lines([
                "for-each-ref",
                "--format=%(refname) %(objectname) %(symref)",
                "refs/remotes/origin",
            ])
            .unwrap()
            .into_iter()
            .filter_map(|line| {
                let parts: Vec<&str> = line.split(' ').collect();
                // origin/HEAD names a branch; it is not one.
                (parts.len() == 2 || parts[2].is_empty())
                    .then(|| (parts[0].to_owned(), parts[1].to_owned()))
            })
            .collect();
        let origin = snap
            .remotes
            .iter()
            .find(|remote| remote.name == "origin")
            .expect("origin is listed");
        let got_remote: BTreeMap<String, String> = origin
            .branches
            .iter()
            .filter(|b| !b.full_name.ends_with("/HEAD"))
            .map(|b| (b.full_name.clone(), b.target.to_hex()))
            .collect();
        assert_eq!(got_remote, want_remote, "{name}: remote branches");
        assert_eq!(snap.worktrees.len(), 1, "{name}: one worktree");
        assert!(snap.stashes.is_empty());

        // A working day: every kind of change at once.
        let tracked: Vec<String> = fields(&git, &["ls-files", "-z"])
            .into_iter()
            .filter(|path| {
                // Regular files in directories, away from what git itself reads.
                !path.starts_with(".git")
                    && clone.path.join(path).is_file()
                    && !clone.path.join(path).is_symlink()
            })
            .collect();
        assert!(tracked.len() > 60, "{name}: enough files to work with");
        let stride = tracked.len() / 40;
        let pick = |n: usize| tracked[(n * stride + 7) % tracked.len()].as_str();
        for n in 0..5 {
            clone.append(pick(n), "\nunstaged change\n");
        }
        for n in 5..10 {
            clone.append(pick(n), "\nstaged change\n");
            git.run(["add", "--", pick(n)]).unwrap();
        }
        for n in 10..13 {
            std::fs::remove_file(clone.path.join(pick(n))).unwrap();
        }
        for n in 13..16 {
            git.run(["rm", "-q", "--", pick(n)]).unwrap();
        }
        for n in 16..19 {
            let to = format!("{}.moved", pick(n));
            git.run(["mv", "--", pick(n), to.as_str()]).unwrap();
        }
        // Staged, then changed again; staged, then deleted.
        clone.append(pick(19), "\nstaged\n");
        git.run(["add", "--", pick(19)]).unwrap();
        clone.append(pick(19), "and changed again\n");
        clone.append(pick(20), "\nstaged\n");
        git.run(["add", "--", pick(20)]).unwrap();
        std::fs::remove_file(clone.path.join(pick(20))).unwrap();
        // New files: untracked, staged, staged and changed, announced only.
        clone.write("zz-corpus/untracked.txt", b"new\n");
        clone.write("zz-corpus/deeper/and deeper/untracked two.txt", b"new\n");
        clone.write("zz-corpus-top.txt", b"new\n");
        clone.write("zz-corpus/staged.txt", b"new\n");
        git.run(["add", "--", "zz-corpus/staged.txt"]).unwrap();
        clone.write("zz-corpus/staged-then-changed.txt", b"new\n");
        git.run(["add", "--", "zz-corpus/staged-then-changed.txt"])
            .unwrap();
        clone.append("zz-corpus/staged-then-changed.txt", "more\n");
        clone.write("zz-corpus/intended.txt", b"new\n");
        git.run(["add", "-N", "--", "zz-corpus/intended.txt"])
            .unwrap();
        // Ignored through the repository's own exclude file: not listed.
        clone.write("zz-ignored/kept-out.txt", b"ignored\n");
        clone.append(".git/info/exclude", "zz-ignored/\n");
        // Another kind of thing in a file's place, and a mode alone.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let link = clone.path.join(pick(21));
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink("somewhere-else", &link).unwrap();
            let mode = clone.path.join(pick(22));
            let mut permissions = std::fs::metadata(&mode).unwrap().permissions();
            permissions.set_mode(permissions.mode() ^ 0o111);
            std::fs::set_permissions(&mode, permissions).unwrap();
        }
        let mixed = compare("a working day");
        assert!(mixed >= 25, "{name}: the changes show ({mixed})");

        // Put away, untracked files included: one stash, a clean tree. (A
        // file that is only announced cannot be stashed by git; it goes
        // back to being untracked first.)
        git.run(["reset", "--quiet", "--", "zz-corpus/intended.txt"])
            .unwrap();
        compare("the announced file untracked again");
        git.run(["stash", "push", "--quiet", "--include-untracked"])
            .unwrap();
        compare("after stashing");
        let snap = snapshot(&repo).unwrap();
        assert_eq!(snap.stashes.len(), 1, "{name}: the stash is listed");
        assert_eq!(
            snap.stashes[0].target.to_hex(),
            git.rev_parse("refs/stash").unwrap()
        );
        git.run(["stash", "pop", "--quiet", "--index"]).unwrap();
        compare("after taking the stash back");

        // A merge that stops: both sides changed the same line, and one
        // side deleted what the other changed.
        git.run(["reset", "--quiet", "--hard"]).unwrap();
        git.run(["clean", "-qfdx"]).unwrap();
        compare("after resetting");
        let (both, gone) = (pick(23), pick(24));
        git.run(["checkout", "--quiet", "-b", "corpus-side"])
            .unwrap();
        clone.write(both, b"their side\n");
        git.run(["rm", "-q", "--", gone]).unwrap();
        git.run(["commit", "--quiet", "-am", "side"]).unwrap();
        git.run(["checkout", "--quiet", "-"]).unwrap();
        clone.write(both, b"our side\n");
        clone.append(gone, "\nchanged here\n");
        git.run(["commit", "--quiet", "-am", "ours"]).unwrap();
        assert!(git.run(["merge", "corpus-side"]).is_err(), "it conflicts");
        let conflicted = compare("a stopped merge");
        assert!(conflicted >= 2, "{name}: both conflicts show");
        let entries = status(&repo).unwrap();
        assert!(entries.iter().any(|e| e.path == both && e.conflict));
        assert!(entries.iter().any(|e| e.path == gone && e.conflict));

        println!(
            "CORPUS {name} statuses: states={states} entries_in_the_mixed_state={mixed} \
             remote_branches={} exact",
            got_remote.len()
        );
    }
}
