//! Differential tests: the gix-based model against the git binary on every
//! fixture in `fixtures/`.

use std::collections::{BTreeMap, BTreeSet};

use bitbraid_git::{LogOptions, Repository, Tips, ascii, layout, log, snapshot};
use bitbraid_testkit::{Git, build_fixture, fixtures_out_dir, list_scripts};

fn oracle(name: &str) -> (Repository, Git) {
    let path = build_fixture(name).expect("fixture builds");
    let repo = Repository::open(&path).expect("model opens the fixture");
    let git = Git::new(&path, fixtures_out_dir().join(".home"));
    (repo, git)
}

#[test]
fn references_match_for_each_ref() {
    for name in list_scripts().unwrap() {
        let (repo, git) = oracle(&name);
        let snap = snapshot(&repo).unwrap();

        // Branches: name and target.
        let want: BTreeMap<String, String> = git
            .lines([
                "for-each-ref",
                "--format=%(refname:short) %(objectname)",
                "refs/heads",
            ])
            .unwrap()
            .into_iter()
            .map(|line| {
                let (n, id) = line.split_once(' ').unwrap();
                (n.to_owned(), id.to_owned())
            })
            .collect();
        let got: BTreeMap<String, String> = snap
            .branches
            .iter()
            .map(|b| (b.name.clone(), b.target.to_hex()))
            .collect();
        assert_eq!(got, want, "{name}: branches");

        // Tags: name and peeled target; annotated flag from the object type.
        // Peeled with `^{}`, which goes through a tag of a tag to what is
        // tagged in the end; `%(*objectname)` took one step only before
        // git 2.45.
        let want_tags: BTreeMap<String, (String, bool)> = git
            .lines([
                "for-each-ref",
                "--format=%(refname:short) %(refname) %(objecttype)",
                "refs/tags",
            ])
            .unwrap()
            .into_iter()
            .map(|line| {
                let parts: Vec<&str> = line.split(' ').collect();
                let peeled = git.rev_parse(&format!("{}^{{}}", parts[1])).unwrap();
                (parts[0].to_owned(), (peeled, parts[2] == "tag"))
            })
            .collect();
        let got_tags: BTreeMap<String, (String, bool)> = snap
            .tags
            .iter()
            .map(|t| (t.name.clone(), (t.target.to_hex(), t.annotated)))
            .collect();
        assert_eq!(got_tags, want_tags, "{name}: tags");

        // HEAD.
        let head_branch = git
            .run(["symbolic-ref", "--short", "HEAD"])
            .ok()
            .map(|s| s.trim().to_owned());
        assert_eq!(snap.head.branch, head_branch, "{name}: HEAD branch");
        assert_eq!(
            snap.head.detached,
            head_branch.is_none(),
            "{name}: detached"
        );
        assert_eq!(
            snap.head.target.map(|id| id.to_hex()),
            Some(git.rev_parse("HEAD").unwrap()),
            "{name}: HEAD target"
        );
        let head_in_branches = snap.branches.iter().filter(|b| b.is_head).count();
        assert_eq!(
            head_in_branches,
            usize::from(head_branch.is_some()),
            "{name}: exactly one HEAD branch"
        );

        if repo.is_bare() {
            assert!(snap.worktrees.is_empty(), "{name}: bare, no worktree");
        } else {
            assert_eq!(snap.worktrees.len(), 1, "{name}: only the main worktree");
            assert!(snap.worktrees[0].is_main);
        }
        assert!(snap.stashes.is_empty(), "{name}: fixtures have no stashes");
        assert!(
            snap.submodules.is_empty(),
            "{name}: fixtures have no submodules"
        );
    }
}

#[test]
fn log_matches_rev_list_and_is_date_ordered() {
    for name in list_scripts().unwrap() {
        let (repo, git) = oracle(&name);
        let commits = log(&repo, &LogOptions::default()).unwrap();

        // Same set of commits and the same parents as git.
        let want_parents: BTreeMap<String, Vec<String>> = git
            .lines(["rev-list", "--all", "--parents"])
            .unwrap()
            .into_iter()
            .map(|line| {
                let mut it = line.split(' ').map(str::to_owned);
                (it.next().unwrap(), it.collect())
            })
            .collect();
        let got_parents: BTreeMap<String, Vec<String>> = commits
            .iter()
            .map(|c| {
                (
                    c.id.to_hex(),
                    c.parents.iter().map(|p| p.to_hex()).collect(),
                )
            })
            .collect();
        assert_eq!(got_parents, want_parents, "{name}: commit set and parents");

        // Same order as git's --date-order (fixture timestamps are distinct).
        let want_order = git.lines(["rev-list", "--all", "--date-order"]).unwrap();
        let got_order: Vec<String> = commits.iter().map(|c| c.id.to_hex()).collect();
        assert_eq!(got_order, want_order, "{name}: date order");

        // Child before parent, always.
        let mut seen = BTreeSet::new();
        for c in &commits {
            for p in &c.parents {
                assert!(
                    !seen.contains(p),
                    "{name}: parent {p} listed before child {}",
                    c.id
                );
            }
            seen.insert(c.id);
        }

        // Summaries and authors.
        let want_meta: BTreeMap<String, (String, String, String)> = git
            .lines(["log", "--all", "--format=%H%x00%s%x00%an%x00%ae"])
            .unwrap()
            .into_iter()
            .map(|line| {
                let parts: Vec<&str> = line.split('\0').collect();
                (
                    parts[0].to_owned(),
                    (
                        parts[1].to_owned(),
                        parts[2].to_owned(),
                        parts[3].to_owned(),
                    ),
                )
            })
            .collect();
        for c in &commits {
            let (summary, author, email) = &want_meta[&c.id.to_hex()];
            assert_eq!(&c.summary, summary, "{name}: summary of {}", c.id);
            assert_eq!(&c.author.name, author, "{name}: author of {}", c.id);
            assert_eq!(&c.author.email, email, "{name}: author email of {}", c.id);
        }

        // Author times are the fixture clock (1 hour apart, +0000).
        let want_times: BTreeMap<String, i64> = git
            .lines(["log", "--all", "--format=%H %at"])
            .unwrap()
            .into_iter()
            .map(|line| {
                let (id, t) = line.split_once(' ').unwrap();
                (id.to_owned(), t.parse().unwrap())
            })
            .collect();
        for c in &commits {
            assert_eq!(
                c.author.seconds,
                want_times[&c.id.to_hex()],
                "{name}: author time of {}",
                c.id
            );
            assert_eq!(c.author.offset, 0, "{name}: author offset of {}", c.id);
        }
    }
}

#[test]
fn topological_order_matches_rev_list_topo_order() {
    use bitbraid_git::{Order, walk};
    let mut differ_from_date_order = 0;
    for name in list_scripts().unwrap() {
        let (repo, git) = oracle(&name);
        // By the author's date, as git has it.
        let written = LogOptions {
            tips: Tips::All,
            order: Order::AuthorDate,
            ..Default::default()
        };
        let got: Vec<String> = walk(&repo, &written)
            .unwrap()
            .iter()
            .map(|commit| commit.id.to_hex())
            .collect();
        let want = git
            .lines(["rev-list", "--all", "--author-date-order"])
            .unwrap();
        assert_eq!(got, want, "{name}: author-date order from all");
        let topo = |tips: Tips| LogOptions {
            tips,
            order: Order::Topo,
            ..Default::default()
        };
        let ids = |commits: &[bitbraid_git::GraphCommit]| -> Vec<String> {
            commits.iter().map(|c| c.id.to_hex()).collect()
        };
        // From every reference, and from HEAD alone.
        let got = walk(&repo, &topo(Tips::All)).unwrap();
        let want = git.lines(["rev-list", "--all", "--topo-order"]).unwrap();
        assert_eq!(ids(&got), want, "{name}: topological order from all");
        let got_head = walk(&repo, &topo(Tips::Head)).unwrap();
        let want_head = git.lines(["rev-list", "HEAD", "--topo-order"]).unwrap();
        assert_eq!(ids(&got_head), want_head, "{name}: from HEAD");

        // A commit before its parents here too, and the same commits as in
        // date order.
        let mut seen = BTreeSet::new();
        for commit in &got {
            for parent in &commit.parents {
                assert!(!seen.contains(parent), "{name}: a parent before its child");
            }
            seen.insert(commit.id);
        }
        let by_date = walk(&repo, &LogOptions::default()).unwrap();
        let mut a = ids(&by_date);
        let mut b = ids(&got);
        if a != b {
            differ_from_date_order += 1;
        }
        a.sort();
        b.sort();
        assert_eq!(a, b, "{name}: the two orders hold the same commits");

        // The layout takes either order: every lane comes from the row above.
        check_lanes(&format!("{name} topo"), &layout(&got));
    }
    assert!(
        differ_from_date_order >= 2,
        "the fixtures would not tell the two orders apart"
    );
}

#[test]
fn head_only_and_first_parent_and_limit() {
    let (repo, git) = oracle("branches-merges");
    let head = log(
        &repo,
        &LogOptions {
            tips: Tips::Head,
            ..Default::default()
        },
    )
    .unwrap();
    let want = git.lines(["rev-list", "HEAD", "--date-order"]).unwrap();
    assert_eq!(head.iter().map(|c| c.id.to_hex()).collect::<Vec<_>>(), want);

    let first = log(
        &repo,
        &LogOptions {
            tips: Tips::Head,
            first_parent: true,
            ..Default::default()
        },
    )
    .unwrap();
    let want = git.lines(["rev-list", "HEAD", "--first-parent"]).unwrap();
    assert_eq!(
        first.iter().map(|c| c.id.to_hex()).collect::<Vec<_>>(),
        want
    );

    let limited = log(
        &repo,
        &LogOptions {
            tips: Tips::All,
            limit: Some(3),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(limited.len(), 3);
}

#[test]
fn graph_layouts_match_golden_files() {
    let mut mismatches = Vec::new();
    for name in list_scripts().unwrap() {
        let (repo, _git) = oracle(&name);
        let commits = log(&repo, &LogOptions::default()).unwrap();
        let rows = layout(&commits);
        assert_eq!(rows.len(), commits.len());
        let art = ascii(&commits, &rows, |c| {
            format!("{} {}", c.id.short(7), c.summary)
        });
        let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(format!("{name}.graph.txt"));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&golden, &art).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!(
                "missing golden {}; run with UPDATE_GOLDEN=1",
                golden.display()
            )
        });
        if want != art {
            mismatches.push(format!(
                "--- {name} expected:\n{want}--- {name} got:\n{art}"
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "golden mismatches:\n{}",
        mismatches.join("\n")
    );
}

/// What a layout has to be whatever is pinned: every lane that ends in a
/// row or passes through it left the row above, and the lanes passing
/// through are told once each, in order, and are not the node's own.
fn check_lanes(name: &str, rows: &[bitbraid_git::GraphRow]) {
    for (ix, row) in rows.iter().enumerate() {
        assert!(
            row.straight
                .windows(2)
                .all(|pair| pair[0].slot < pair[1].slot),
            "{name} row {ix}: lanes out of order"
        );
        assert!(
            row.straight.iter().all(|lane| lane.slot != row.lane),
            "{name} row {ix}: a lane passes through the node"
        );
        let Some(above) = ix.checked_sub(1).and_then(|above| rows.get(above)) else {
            assert!(row.straight.is_empty() && row.merges_in.is_empty());
            continue;
        };
        for lane in row.straight.iter().chain(&row.merges_in) {
            assert!(
                above.leaves(lane.slot),
                "{name} row {ix}: lane {} comes from nowhere",
                lane.slot
            );
        }
        for lane in &row.straight {
            assert!(row.leaves(lane.slot), "{name} row {ix}: a lane is lost");
        }
        assert!(row.width > row.lane, "{name} row {ix}: the node is outside");
    }
}

/// The commits of a branch as the layout pins them: its tip and the first
/// parents below it, by git.
fn first_parents(git: &bitbraid_testkit::Git, branch: &str) -> Vec<String> {
    git.lines(["rev-list", "--first-parent", branch]).unwrap()
}

#[test]
fn pinned_layouts_match_golden_files() {
    use bitbraid_git::layout_pinned;
    let mut mismatches = Vec::new();
    for name in list_scripts().unwrap() {
        let (repo, git) = oracle(&name);
        let commits = log(&repo, &LogOptions::default()).unwrap();
        let snap = snapshot(&repo).unwrap();
        check_lanes(&name, &layout(&commits));
        // Every local branch pinned by itself: its line has the first lane
        // from its tip to its root, and nothing else sits in that lane.
        for branch in &snap.branches {
            let rows = layout_pinned(&commits, &[branch.target]);
            assert_eq!(rows.len(), commits.len());
            check_lanes(&format!("{name} pinned {}", branch.name), &rows);
            let line = first_parents(&git, &branch.name);
            let on_line = |ix: usize| line.contains(&commits[ix].id.to_hex());
            let tip = commits.iter().position(|c| c.id == branch.target).unwrap();
            let root = commits
                .iter()
                .position(|c| c.id.to_hex() == *line.last().unwrap())
                .unwrap();
            for (ix, row) in rows.iter().enumerate() {
                if on_line(ix) {
                    assert_eq!(row.lane, 0, "{name}: {} row {ix}", branch.name);
                } else if ix <= root {
                    assert_ne!(row.lane, 0, "{name}: {} row {ix}", branch.name);
                }
                // Above the tip the lane holds nothing but the line of a
                // merge that has the tip for a parent, on its way there.
                if ix < tip && row.leaves(0) {
                    let merged_above = commits
                        .iter()
                        .take(ix + 1)
                        .any(|commit| commit.parents.iter().skip(1).any(|p| *p == branch.target));
                    assert!(merged_above, "{name}: {} row {ix}", branch.name);
                }
            }
        }
        // Two pins, the last two local branches by name with the later
        // name first: what the golden file shows.
        let pins: Vec<&bitbraid_git::snapshot::Branch> =
            snap.branches.iter().rev().take(2).collect();
        let ids: Vec<bitbraid_git::Oid> = pins.iter().map(|branch| branch.target).collect();
        let rows = layout_pinned(&commits, &ids);
        check_lanes(&format!("{name} two pins"), &rows);
        let names: Vec<&str> = pins.iter().map(|branch| branch.name.as_str()).collect();
        let art = format!(
            "pinned: {}\n{}",
            names.join(", "),
            ascii(&commits, &rows, |c| {
                format!("{} {}", c.id.short(7), c.summary)
            })
        );
        let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(format!("{name}.pinned.txt"));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&golden, &art).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!(
                "missing golden {}; run with UPDATE_GOLDEN=1",
                golden.display()
            )
        });
        if want != art {
            mismatches.push(format!(
                "--- {name} expected:\n{want}--- {name} got:\n{art}"
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "golden mismatches:\n{}",
        mismatches.join("\n")
    );
}

/// Rows for the uncommitted work of other worktrees: one worktree on every
/// local branch and a detached one on the first commit of the history,
/// laid out and held to a golden file.
#[test]
fn worktree_rows_match_golden_files() {
    use bitbraid_git::{details, layout_with_wip, walk, wip};
    let mut mismatches = Vec::new();
    for name in list_scripts().unwrap() {
        let (repo, _git) = oracle(&name);
        let walked = walk(&repo, &LogOptions::default()).unwrap();
        let snap = snapshot(&repo).unwrap();
        let ids: Vec<bitbraid_git::Oid> = walked.iter().map(|c| c.id).collect();
        let summaries: BTreeMap<bitbraid_git::Oid, String> = details(&repo, &ids)
            .unwrap()
            .into_iter()
            .map(|c| (c.id, c.summary))
            .collect();
        let mut places: Vec<(String, bitbraid_git::Oid)> = snap
            .branches
            .iter()
            .map(|branch| (format!("on {}", branch.name), branch.target))
            .collect();
        places.push((
            "detached, at the first commit".to_owned(),
            *ids.last().unwrap(),
        ));
        let rows: Vec<(bitbraid_git::Oid, bitbraid_git::Oid)> = places
            .iter()
            .map(|(what, head)| {
                (
                    wip::row_id(std::path::Path::new(&format!("/work/{what}"))),
                    *head,
                )
            })
            .collect();
        let with = wip::with_rows(walked.clone(), &rows);
        assert_eq!(with.len(), walked.len() + rows.len(), "{name}");
        // Every row stands right above a run of rows of the same commit
        // and that commit, which is its only parent; the commits are in
        // the order they were in.
        for (row, head) in &rows {
            let at = with.iter().position(|c| c.id == *row).unwrap();
            assert_eq!(with[at].parents, [*head], "{name}");
            let below = with[at + 1..]
                .iter()
                .find(|c| !rows.iter().any(|(id, _)| *id == c.id))
                .unwrap();
            assert_eq!(below.id, *head, "{name}");
        }
        let without: Vec<bitbraid_git::Oid> = with
            .iter()
            .map(|c| c.id)
            .filter(|id| !rows.iter().any(|(row, _)| row == id))
            .collect();
        assert_eq!(without, ids, "{name}");

        let laid = layout(&with);
        check_lanes(&format!("{name} worktree rows"), &laid);
        // With the row of the open worktree on top as well.
        let (top, under) = layout_with_wip(&with, snap.head.target);
        assert!(top.continues, "{name}: HEAD is in the graph");
        check_lanes(
            &format!("{name} worktree rows under uncommitted work"),
            &std::iter::once(top).chain(under).collect::<Vec<_>>(),
        );
        let art = ascii(&with, &laid, |c| {
            match rows.iter().position(|(row, _)| *row == c.id) {
                Some(at) => format!("~ uncommitted work in a worktree {}", places[at].0),
                None => format!("{} {}", c.id.short(7), summaries[&c.id]),
            }
        });
        let golden = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/golden")
            .join(format!("{name}.worktrees.txt"));
        if std::env::var_os("UPDATE_GOLDEN").is_some() {
            std::fs::write(&golden, &art).unwrap();
            continue;
        }
        let want = std::fs::read_to_string(&golden).unwrap_or_else(|_| {
            panic!(
                "missing golden {}; run with UPDATE_GOLDEN=1",
                golden.display()
            )
        });
        if want != art {
            mismatches.push(format!(
                "--- {name} expected:\n{want}--- {name} got:\n{art}"
            ));
        }
    }
    assert!(
        mismatches.is_empty(),
        "golden mismatches:\n{}",
        mismatches.join("\n")
    );
}

/// A history handed out in pieces: every piece is the beginning of the
/// whole, row for row and lane for lane, in both orders.
#[test]
fn pieces_of_a_walk_are_the_beginning_of_the_whole() {
    use bitbraid_git::Order;
    for name in list_scripts().unwrap() {
        let (repo, _git) = oracle(&name);
        for order in [Order::Date, Order::Topo] {
            check_pieces(&name, &repo, order, 1);
        }
    }
    // The fixtures' commits are an hour apart and the margin holds three
    // days of them back, so their pieces are empty. A history of several
    // weeks with lines that merge again and again has pieces with rows.
    let spec = bitbraid_testkit::CorpusSpec {
        commits: 900,
        lanes: 3,
        merge_every: 7,
        files: 12,
    };
    let dir = bitbraid_testkit::build_synthetic(&spec).unwrap();
    let repo = Repository::open(&dir).unwrap();
    let (longest, handed) = check_pieces("synthetic", &repo, Order::Date, 37);
    assert!(longest > 700, "by date nearly all is certain: {longest}");
    assert!(handed >= 1, "a walk to the end hands pieces out");
    // In topological order a piece ends where the newest line leaves what
    // was walked: short here, where lines are seven commits long.
    let (longest, _) = check_pieces("synthetic", &repo, Order::Topo, 37);
    assert!(longest >= 1, "{longest}");
}

/// Hold the pieces of a history to the whole of it: first pieces of every
/// `step`-th size, and a walk to the end. Returns the most rows a first
/// piece had and how many pieces the walk to the end handed out.
fn check_pieces(
    name: &str,
    repo: &Repository,
    order: bitbraid_git::Order,
    step: usize,
) -> (usize, usize) {
    use bitbraid_git::{pieces, walk};
    let options = LogOptions {
        order,
        ..Default::default()
    };
    let whole = walk(repo, &options).unwrap();
    let whole_rows = layout(&whole);
    // A first piece as large as the history, or larger, is the history.
    assert_eq!(
        pieces::first(repo, &options, whole.len()).unwrap(),
        pieces::First::Whole(whole.clone()),
        "{name} {order:?}"
    );
    assert_eq!(
        pieces::first(repo, &options, usize::MAX).unwrap(),
        pieces::First::Whole(whole.clone())
    );
    // Smaller ones are its beginning, row for row and lane for lane.
    let mut longest = 0;
    for size in (0..whole.len()).step_by(step.max(1)) {
        match pieces::first(repo, &options, size).unwrap() {
            pieces::First::Whole(_) => panic!("{name}: {size} is not all"),
            pieces::First::Piece { rows, walked } => {
                assert_eq!(walked, size);
                assert!(rows.len() <= size);
                assert_eq!(rows, whole[..rows.len()], "{name} {order:?} {size}");
                assert_eq!(
                    layout(&rows),
                    whole_rows[..rows.len()],
                    "{name} {order:?} {size}: lanes"
                );
                longest = longest.max(rows.len());
            }
        }
    }
    // Walked to the end in pieces: the same commits, and every piece on
    // the way is a beginning too.
    let mut handed: Vec<(usize, usize)> = Vec::new();
    let all = pieces::walk_in_pieces(repo, &options, 1, &|| true, &mut |rows, walked| {
        assert!(rows.len() >= pieces::SMALLEST_PIECE);
        assert_eq!(rows, whole[..rows.len()], "{name} {order:?} at {walked}");
        handed.push((rows.len(), walked));
    })
    .unwrap();
    assert_eq!(all.as_ref(), Some(&whole), "{name} {order:?}");
    assert!(handed.windows(2).all(|pair| pair[0].1 < pair[1].1));
    // Stopped: no result.
    let stopped = pieces::walk_in_pieces(repo, &options, 1, &|| false, &mut |_, _| {}).unwrap();
    assert_eq!(stopped, None);
    (longest, handed.len())
}

#[test]
fn opening_a_non_repository_fails_cleanly() {
    let dir = std::env::temp_dir().join(format!("bitbraid-not-a-repo-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let err = Repository::open(&dir).expect_err("must fail");
    // The same words wherever the search stopped: at the root, at a
    // ceiling or at the edge of a file system (`/tmp` is one of its own on
    // some machines and not on others).
    assert_eq!(
        err.to_string(),
        format!(
            "cannot open a repository at {}: there is no git repository there or in any folder above it",
            dir.display()
        )
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn linked_worktrees_appear_in_the_snapshot() {
    use bitbraid_testkit::Scratch;
    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    let wt_path = scratch.path().parent().unwrap().join(format!(
        "{}-wt",
        scratch.path().file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&wt_path);
    git.run([
        "worktree",
        "add",
        "-q",
        wt_path.to_str().unwrap(),
        "feature/a",
    ])
    .unwrap();

    let repo = Repository::open(scratch.path()).unwrap();
    let snap = snapshot(&repo).unwrap();
    assert_eq!(snap.worktrees.len(), 2, "{:?}", snap.worktrees);
    let linked = snap
        .worktrees
        .iter()
        .find(|w| !w.is_main)
        .expect("linked worktree listed");
    assert_eq!(linked.branch.as_deref(), Some("feature/a"));
    assert_eq!(
        linked.head.map(|id| id.to_hex()),
        Some(git.rev_parse("feature/a").unwrap())
    );
    assert_eq!(
        linked.path.as_ref().map(|p| p.canonicalize().unwrap()),
        Some(wt_path.canonicalize().unwrap())
    );
    assert!(!linked.locked);

    git.run([
        "worktree",
        "lock",
        "--reason",
        "testing",
        wt_path.to_str().unwrap(),
    ])
    .unwrap();
    let snap = snapshot(&repo).unwrap();
    let linked = snap.worktrees.iter().find(|w| !w.is_main).unwrap();
    assert!(linked.locked);
    assert_eq!(linked.lock_reason.as_deref(), Some("testing"));

    // Status of the main worktree is unaffected by the linked one.
    assert!(bitbraid_git::status(&repo).unwrap().is_empty());
    let current: Vec<bool> = snap.worktrees.iter().map(|w| w.current).collect();
    assert_eq!(current, [true, false]);

    // Opened in the linked one, the main one is still the main one, and
    // each is listed once.
    let inside = Repository::open(&wt_path).unwrap();
    let seen = snapshot(&inside).unwrap();
    assert_eq!(seen.worktrees.len(), 2, "{:?}", seen.worktrees);
    let (main, linked) = (&seen.worktrees[0], &seen.worktrees[1]);
    assert!(main.is_main && !main.current);
    assert!(!linked.is_main && linked.current);
    assert_eq!(
        main.path.as_ref().map(|p| p.canonicalize().unwrap()),
        Some(scratch.path().canonicalize().unwrap())
    );
    assert_eq!(main.branch.as_deref(), Some("main"));
    assert_eq!(
        main.head.map(|id| id.to_hex()),
        Some(git.rev_parse("main").unwrap())
    );
    assert_eq!(linked.branch.as_deref(), Some("feature/a"));
    assert_eq!(seen.head.branch.as_deref(), Some("feature/a"));

    git.run(["worktree", "unlock", wt_path.to_str().unwrap()])
        .unwrap();
    git.run(["worktree", "remove", "--force", wt_path.to_str().unwrap()])
        .unwrap();
    let _ = std::fs::remove_dir_all(&wt_path);
}

#[test]
fn submodules_appear_in_the_snapshot_and_status() {
    use bitbraid_testkit::Scratch;
    let sub_source = build_fixture("linear").unwrap();
    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    git.run([
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        sub_source.to_str().unwrap(),
        "vendor/linear",
    ])
    .unwrap();
    git.run(["commit", "-qm", "add submodule"]).unwrap();

    let repo = Repository::open(scratch.path()).unwrap();
    let snap = snapshot(&repo).unwrap();
    assert_eq!(snap.submodules.len(), 1, "{:?}", snap.submodules);
    let sm = &snap.submodules[0];
    assert_eq!(sm.name, "vendor/linear");
    assert_eq!(sm.path, "vendor/linear");
    let sub_head = Git::new(&sub_source, fixtures_out_dir().join(".home"))
        .rev_parse("HEAD")
        .unwrap();
    assert_eq!(sm.head.map(|id| id.to_hex()), Some(sub_head.clone()));
    assert_eq!(sm.index.map(|id| id.to_hex()), Some(sub_head));
    assert!(sm.active);
    assert!(
        bitbraid_git::status(&repo).unwrap().is_empty(),
        "clean after committing the submodule"
    );

    // The superproject's tree diff shows the submodule as a commit entry without hunks.
    let head = git.rev_parse("HEAD").unwrap().parse().unwrap();
    let parent = git.rev_parse("HEAD~1").unwrap().parse().unwrap();
    let files = bitbraid_git::diff_trees(
        &repo,
        &bitbraid_git::DiffSide::Commit(parent),
        &bitbraid_git::DiffSide::Commit(head),
        &bitbraid_git::DiffOptions::default(),
    )
    .unwrap();
    let sm_diff = files
        .iter()
        .find(|f| f.path == "vendor/linear")
        .expect("submodule entry in diff");
    assert_eq!(sm_diff.new_mode.as_deref(), Some("160000"));
    assert!(sm_diff.hunks.is_empty());
    assert!(sm_diff.is_submodule());
    assert!(files.iter().any(|f| f.path == ".gitmodules"));

    // One path between any two sides gives the same, the index and the
    // worktree included: the commits recorded and checked out, no hunks.
    use bitbraid_git::{DiffSide, diff::FileChangeKind, diff_file};
    let sub_head: bitbraid_git::Oid = git
        .rev_parse("HEAD:vendor/linear")
        .unwrap()
        .parse()
        .unwrap();
    let one = |old: &DiffSide, new: &DiffSide| {
        diff_file(&repo, old, new, "vendor/linear", 3)
            .unwrap()
            .expect("the submodule is on a side")
    };
    let added = one(&DiffSide::Commit(parent), &DiffSide::Commit(head));
    assert_eq!(&added, sm_diff);
    assert_eq!(added.kind, FileChangeKind::Added);
    for (old, new) in [
        (DiffSide::Commit(head), DiffSide::Index),
        (DiffSide::Index, DiffSide::Worktree),
        (DiffSide::Commit(head), DiffSide::Worktree),
    ] {
        let same = one(&old, &new);
        assert!(same.is_submodule() && same.hunks.is_empty() && !same.binary);
        assert_eq!(same.kind, FileChangeKind::Modified);
        assert_eq!((same.old_id, same.new_id), (Some(sub_head), Some(sub_head)));
    }
    // Checked out at another commit: the worktree side is where it is.
    let inside = Git::new(
        scratch.path().join("vendor/linear"),
        fixtures_out_dir().join(".home"),
    );
    inside.run(["checkout", "-q", "HEAD~2"]).unwrap();
    let moved: bitbraid_git::Oid = inside.rev_parse("HEAD").unwrap().parse().unwrap();
    let unstaged = one(&DiffSide::Index, &DiffSide::Worktree);
    assert_eq!(
        (unstaged.old_id, unstaged.new_id),
        (Some(sub_head), Some(moved))
    );
    assert_eq!(
        git.run(["diff", "--raw", "--abbrev=40", "--", "vendor/linear"])
            .unwrap()
            .split_whitespace()
            .nth(2),
        Some(sub_head.to_hex().as_str()),
        "git names the recorded commit as the old side too"
    );
    git.run(["add", "vendor/linear"]).unwrap();
    let staged = one(&DiffSide::Commit(head), &DiffSide::Index);
    assert_eq!(
        (staged.old_id, staged.new_id),
        (Some(sub_head), Some(moved))
    );
    // Not checked out any more: it stands where the index has it.
    git.run([
        "submodule",
        "--quiet",
        "deinit",
        "--force",
        "--",
        "vendor/linear",
    ])
    .unwrap();
    let absent = one(&DiffSide::Index, &DiffSide::Worktree);
    assert_eq!(absent.old_id, absent.new_id);
    // A plain directory is not a file and not a submodule.
    assert!(
        diff_file(&repo, &DiffSide::Index, &DiffSide::Worktree, "vendor", 3)
            .unwrap()
            .is_none()
    );
}

/// A branch is taken at its word when the walk starts; one that points at a
/// tree must cost that branch only, not the whole graph.
#[test]
fn a_branch_pointing_at_a_tree_does_not_stop_the_walk() {
    let scratch = bitbraid_testkit::Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    let before = {
        let repo = Repository::open(scratch.path()).unwrap();
        bitbraid_git::walk(&repo, &LogOptions::default()).unwrap()
    };
    assert_eq!(before.len(), 7);

    let tree = git.rev_parse("HEAD^{tree}").unwrap();
    // git refuses to point a local branch at a tree; a hostile repository
    // has no such manners, so write the reference file directly.
    std::fs::write(
        scratch.path().join(".git/refs/heads/not-a-commit"),
        format!("{tree}\n"),
    )
    .unwrap();
    git.run(["update-ref", "refs/remotes/origin/also-not", tree.as_str()])
        .unwrap();

    let repo = Repository::open(scratch.path()).unwrap();
    let after = bitbraid_git::walk(&repo, &LogOptions::default()).unwrap();
    assert_eq!(after, before);
    // The references panel still lists the branch; it has no commit row.
    let snap = snapshot(&repo).unwrap();
    assert!(snap.branches.iter().any(|b| b.name == "not-a-commit"));
}

/// The HEAD of a linked worktree is read from its `HEAD` file: detached,
/// unborn and hostile contents all have to come out as data, never a panic.
#[test]
fn linked_worktree_heads_survive_odd_head_files() {
    use bitbraid_testkit::Scratch;
    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    let wt_path = scratch.path().parent().unwrap().join(format!(
        "{}-wt-detached",
        scratch.path().file_name().unwrap().to_string_lossy()
    ));
    let _ = std::fs::remove_dir_all(&wt_path);
    git.run([
        "worktree",
        "add",
        "-q",
        "--detach",
        wt_path.to_str().unwrap(),
        "HEAD~1",
    ])
    .unwrap();
    let repo = Repository::open(scratch.path()).unwrap();
    let linked = |repo: &Repository| {
        snapshot(repo)
            .unwrap()
            .worktrees
            .into_iter()
            .find(|w| !w.is_main)
            .expect("linked worktree listed")
    };

    let detached = linked(&repo);
    assert_eq!(detached.branch, None);
    assert_eq!(
        detached.head.map(|id| id.to_hex()),
        Some(git.rev_parse("HEAD~1").unwrap())
    );

    let head_file = scratch
        .path()
        .join(".git/worktrees")
        .join(&detached.id)
        .join("HEAD");
    assert!(head_file.is_file(), "{head_file:?}");

    std::fs::write(&head_file, "ref: refs/heads/not-born-yet\n").unwrap();
    let unborn = linked(&repo);
    assert_eq!(unborn.branch.as_deref(), Some("not-born-yet"));
    assert_eq!(unborn.head, None);

    std::fs::write(&head_file, "ref: refs/heads/feature/b\n").unwrap();
    let on_branch = linked(&repo);
    assert_eq!(on_branch.branch.as_deref(), Some("feature/b"));
    assert_eq!(
        on_branch.head.map(|id| id.to_hex()),
        Some(git.rev_parse("feature/b").unwrap())
    );

    for hostile in [
        String::new(),
        "not an object id\n".to_owned(),
        "ref:\n".to_owned(),
        "ref: ../../../../etc/passwd\n".to_owned(),
        "f".repeat(1 << 20),
    ] {
        std::fs::write(&head_file, &hostile).unwrap();
        let odd = linked(&repo);
        assert_eq!(odd.head, None, "{:?}", hostile.get(..20));
    }

    let _ = std::fs::remove_dir_all(&wt_path);
}

/// What a tag comes to in the end, as git peels it.
fn peeled(git: &Git, tag: &str) -> String {
    git.rev_parse(&format!("refs/tags/{tag}^{{}}")).unwrap()
}

#[test]
fn a_tag_of_a_tag_is_read_as_the_commit_it_comes_to() {
    let (repo, git) = oracle("nested-tags");
    let snap = snapshot(&repo).unwrap();
    let names: Vec<&str> = snap.tags.iter().map(|tag| tag.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "alias",
            "inner",
            "kept-twice",
            "outer",
            "outermost",
            "plain"
        ]
    );
    for tag in &snap.tags {
        let name = tag.name.as_str();
        // Where it points once every tag on the way is followed: a commit.
        assert_eq!(tag.target.to_hex(), peeled(&git, name), "{name}: target");
        assert_eq!(
            git.run(["cat-file", "-t", &tag.target.to_hex()]).unwrap(),
            "commit\n",
            "{name}: the target is a commit"
        );
        // What the reference itself holds: the outermost tag object.
        let held = git.rev_parse(&format!("refs/tags/{name}")).unwrap();
        let is_object = git.run(["cat-file", "-t", &held]).unwrap() == "tag\n";
        assert_eq!(tag.annotated, is_object, "{name}: annotated");
        assert_eq!(
            tag.tag_object.map(|id| id.to_hex()),
            is_object.then_some(held),
            "{name}: tag object"
        );
        // The message is that tag object's own, not one from further in.
        let subject = git
            .run([
                "for-each-ref",
                "--format=%(contents:subject)",
                &format!("refs/tags/{name}"),
            ])
            .unwrap();
        if is_object {
            assert_eq!(
                tag.message.as_deref().and_then(|text| text.lines().next()),
                Some(subject.trim()),
                "{name}: message"
            );
        } else {
            assert_eq!(tag.message, None, "{name}: message");
        }
    }
    // The fixture is what it says: tags three deep, and a reference that
    // holds another tag's object.
    let by_name = |name: &str| snap.tags.iter().find(|tag| tag.name == name).unwrap();
    let one_step = |name: &str| {
        git.rev_parse(&format!("refs/tags/{name}^{{tag}}"))
            .and_then(|tag| git.run(["cat-file", "-p", &tag]))
            .unwrap()
    };
    assert!(one_step("outermost").contains("type tag"), "a tag of a tag");
    assert!(one_step("outer").contains("type tag"), "a tag of a tag");
    assert!(one_step("inner").contains("type commit"));
    assert_eq!(by_name("alias").tag_object, by_name("inner").tag_object);
    for name in ["alias", "outer", "outermost"] {
        assert_eq!(by_name(name).target, by_name("inner").target, "{name}");
    }
    assert_eq!(
        by_name("inner").target.to_hex(),
        git.rev_parse("main~1").unwrap()
    );
}

#[test]
fn nested_tags_label_the_commits_git_decorates_with_them() {
    let (repo, git) = oracle("nested-tags");
    let snap = snapshot(&repo).unwrap();
    // What the graph writes beside a commit: the tags whose target it is.
    let mut ours: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for tag in &snap.tags {
        ours.entry(tag.target.to_hex())
            .or_default()
            .insert(tag.name.clone());
    }
    // Git's decorations, which follow tags of tags too.
    let mut theirs: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for line in git
        .lines(["log", "--all", "--decorate=full", "--format=%H %D"])
        .unwrap()
    {
        let (commit, decorations) = line.split_once(' ').unwrap_or((line.as_str(), ""));
        for decoration in decorations.split(", ") {
            if let Some(tag) = decoration.strip_prefix("tag: refs/tags/") {
                theirs
                    .entry(commit.to_owned())
                    .or_default()
                    .insert(tag.to_owned());
            }
        }
    }
    assert_eq!(ours, theirs);
    assert_eq!(ours.len(), 3, "three commits carry tags: {ours:?}");

    // Every labelled commit has a row to be labelled on, also the one
    // that only a tag of a tag still holds.
    let commits = log(&repo, &LogOptions::default()).unwrap();
    let rows: BTreeSet<String> = commits.iter().map(|c| c.id.to_hex()).collect();
    for commit in ours.keys() {
        assert!(rows.contains(commit), "{commit} is in no row");
    }
    let kept = peeled(&git, "kept-twice");
    assert!(
        git.lines(["branch", "--contains", &kept])
            .unwrap()
            .is_empty(),
        "no branch has the commit"
    );
    let row = commits.iter().find(|c| c.id.to_hex() == kept).unwrap();
    assert_eq!(row.summary, "g1");

    // The walk from the tags alone starts at the commits, not at the tags.
    let tips: Vec<bitbraid_git::Oid> = snap.tags.iter().map(|tag| tag.target).collect();
    let from_tags = log(
        &repo,
        &LogOptions {
            tips: Tips::Ids(tips),
            ..Default::default()
        },
    )
    .unwrap();
    let want: BTreeSet<String> = git
        .lines(["rev-list", "--tags"])
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(
        from_tags
            .iter()
            .map(|c| c.id.to_hex())
            .collect::<BTreeSet<_>>(),
        want
    );
}
