//! Gitflow with plain git: the settings, the branches of each kind
//! (feature, bugfix, release, hotfix, support), and what starting and
//! finishing one takes.
//!
//! git-flow itself is never run. The settings live under the keys it uses
//! (`gitflow.branch.master`, `gitflow.branch.develop`, `gitflow.prefix.*`),
//! so a repository set up by either tool works with the other.
//!
//! A finish is a sequence of ordinary steps: check out, merge, tag, delete.
//! [`finish_plan`] looks at the repository and says which of them are still
//! to do, after checking everything that would make one of them refuse, so
//! that a finish either refuses with nothing changed or can be run again
//! from wherever it stopped.
//!
//! Sources: the branching model as it is publicly described ("A
//! successful Git branching model", nvie.com), and for the two kinds it
//! does not have, what git-flow's public command reference says of them
//! (a bugfix starts from develop and goes back into it, a support branch
//! starts from main and is never finished); git-config(1),
//! git-branch(1), git-switch(1), git-merge(1), git-merge-base(1),
//! git-tag(1), git-for-each-ref(1) (`HEAD`, `worktreepath`),
//! git-check-ref-format(1). Nothing of git-flow's scripts was used.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::{
    Dirty, Error, HeadState, MergeMode, Moved, Oid, Repository, Result,
    branch::{self, DeletedBranch},
    exe, rewrite, tag,
};

/// Longest branch name, prefix included, that is started or set up here.
const MAX_NAME: usize = 200;
/// Longest tag message: it is handed to git as one argument.
const MAX_MESSAGE: usize = 64 * 1024;

const KEY_MAIN: &str = "gitflow.branch.master";
const KEY_DEVELOP: &str = "gitflow.branch.develop";
const KEY_TAG: &str = "gitflow.prefix.versiontag";

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Refused {
        what,
        detail: detail.into(),
    }
}

/// The kinds of branch gitflow starts and finishes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// Work for a coming release: from develop, back into develop.
    Feature,
    /// A fix to what is being developed: from develop, back into develop.
    Bugfix,
    /// A release being prepared: from develop, into main and develop.
    Release,
    /// A fix to what is released: from main, into main and develop.
    Hotfix,
    /// An older release that is still looked after: from main, and never
    /// merged back anywhere.
    Support,
}

impl Kind {
    /// Every kind, in the order they are listed.
    pub const ALL: [Kind; 5] = [
        Kind::Feature,
        Kind::Bugfix,
        Kind::Release,
        Kind::Hotfix,
        Kind::Support,
    ];

    /// The word for it in a sentence.
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Feature => say!("flow-kind-feature"),
            Kind::Bugfix => say!("flow-kind-bugfix"),
            Kind::Release => say!("flow-kind-release"),
            Kind::Hotfix => say!("flow-kind-hotfix"),
            Kind::Support => say!("flow-kind-support"),
        }
    }

    fn key(self) -> &'static str {
        match self {
            Kind::Feature => "gitflow.prefix.feature",
            Kind::Bugfix => "gitflow.prefix.bugfix",
            Kind::Release => "gitflow.prefix.release",
            Kind::Hotfix => "gitflow.prefix.hotfix",
            Kind::Support => "gitflow.prefix.support",
        }
    }

    fn default_prefix(self) -> &'static str {
        match self {
            Kind::Feature => "feature/",
            Kind::Bugfix => "bugfix/",
            Kind::Release => "release/",
            Kind::Hotfix => "hotfix/",
            Kind::Support => "support/",
        }
    }

    /// Whether finishing one goes through main and leaves a tag there.
    fn tagged(self) -> bool {
        matches!(self, Kind::Release | Kind::Hotfix)
    }

    /// Whether one is started from main rather than from develop: what
    /// mends or carries on something that is released.
    fn starts_on_main(self) -> bool {
        matches!(self, Kind::Hotfix | Kind::Support)
    }

    /// Whether one is ever finished. A support branch is not: it stays
    /// beside main for as long as that line of releases is looked after.
    pub fn finishes(self) -> bool {
        self != Kind::Support
    }
}

/// What the branches of each kind, and the tags, begin with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prefixes {
    /// Feature branches.
    pub feature: String,
    /// Bugfix branches.
    pub bugfix: String,
    /// Release branches.
    pub release: String,
    /// Hotfix branches.
    pub hotfix: String,
    /// Support branches.
    pub support: String,
    /// The tags a release or hotfix leaves on main.
    pub tag: String,
}

impl Prefixes {
    /// The prefix of one kind.
    pub fn of(&self, kind: Kind) -> &str {
        match kind {
            Kind::Feature => &self.feature,
            Kind::Bugfix => &self.bugfix,
            Kind::Release => &self.release,
            Kind::Hotfix => &self.hotfix,
            Kind::Support => &self.support,
        }
    }
}

/// A branch of one of the kinds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FlowBranch {
    /// Its name without the prefix: what start and finish are given.
    pub name: String,
    /// The branch.
    pub branch: String,
    /// The commit it points at.
    pub tip: Oid,
    /// It is the one checked out here.
    pub current: bool,
}

/// How gitflow stands in a repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Status {
    /// Both branches are configured and exist.
    pub initialized: bool,
    /// When not: what is missing.
    pub reason: Option<String>,
    /// The branch releases are made on, as configured.
    pub main: Option<String>,
    /// The branch work is gathered on, as configured.
    pub develop: Option<String>,
    /// The prefixes in effect: configured ones, the defaults for the rest.
    pub prefixes: Prefixes,
    /// Feature branches there are.
    pub features: Vec<FlowBranch>,
    /// Bugfix branches there are.
    pub bugfixes: Vec<FlowBranch>,
    /// Release branches there are.
    pub releases: Vec<FlowBranch>,
    /// Hotfix branches there are.
    pub hotfixes: Vec<FlowBranch>,
    /// Support branches there are.
    pub supports: Vec<FlowBranch>,
}

/// The settings of a repository where gitflow is set up.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Settings {
    main: String,
    develop: String,
    prefixes: Prefixes,
}

/// Refuse what git could read as an option or as more than one line.
fn plain(what: &'static str, text: &str) -> Result<()> {
    if text.is_empty() {
        return Err(invalid(what, say!("flow-empty")));
    }
    if text.starts_with('-') {
        return Err(invalid(
            what,
            say!("flow-starts-with-dash", text = format!("{text:?}")),
        ));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid(
            what,
            say!("flow-control-characters", text = format!("{text:?}")),
        ));
    }
    if text.len() > MAX_NAME {
        return Err(invalid(what, say!("flow-too-long")));
    }
    Ok(())
}

/// Entries of the configuration whose key matches `pattern`, as git
/// resolves them: every scope, in the order git reads them, so the last
/// one of a key is the one in effect.
pub(crate) fn config_matching(repo: &Repository, pattern: &str) -> Result<Vec<(String, String)>> {
    let (code, out) = exe::run_with_code(repo, ["config", "-z", "--get-regexp", pattern], None)?;
    match code {
        // `key\nvalue\0`; a key without a value has no line break either.
        Some(0) => Ok(out
            .stdout
            .split('\0')
            .filter(|entry| !entry.is_empty())
            .map(|entry| match entry.split_once('\n') {
                Some((key, value)) => (key.to_owned(), value.to_owned()),
                None => (entry.to_owned(), String::new()),
            })
            .collect()),
        // Nothing matches.
        Some(1) => Ok(Vec::new()),
        _ => Err(invalid(
            say!("flow-what-configuration"),
            out.stderr.trim().to_owned(),
        )),
    }
}

fn config_set(repo: &Repository, key: &str, value: &str) -> Result<()> {
    exe::run(
        repo,
        ["config", "--local", "--replace-all", "--", key, value],
        None,
    )?;
    Ok(())
}

/// Everything under `gitflow.` as it is configured.
fn configured(repo: &Repository) -> Result<HashMap<String, String>> {
    Ok(config_matching(repo, r"^gitflow\.")?.into_iter().collect())
}

/// A branch name from the settings: one that is set to something.
fn named(raw: &HashMap<String, String>, key: &str) -> Option<String> {
    raw.get(key).filter(|name| !name.is_empty()).cloned()
}

/// The main and the develop branch as they are configured, whether or
/// not there are such branches.
pub(crate) fn configured_branches(repo: &Repository) -> Result<(Option<String>, Option<String>)> {
    let raw = configured(repo)?;
    Ok((named(&raw, KEY_MAIN), named(&raw, KEY_DEVELOP)))
}

fn prefixes_of(raw: &HashMap<String, String>) -> Prefixes {
    let of = |kind: Kind| {
        raw.get(kind.key())
            .cloned()
            .unwrap_or_else(|| kind.default_prefix().to_owned())
    };
    Prefixes {
        feature: of(Kind::Feature),
        bugfix: of(Kind::Bugfix),
        release: of(Kind::Release),
        hotfix: of(Kind::Hotfix),
        support: of(Kind::Support),
        tag: raw.get(KEY_TAG).cloned().unwrap_or_default(),
    }
}

/// A local branch as [`heads`] lists it.
struct Head {
    name: String,
    tip: Option<Oid>,
    current: bool,
    /// Another worktree that has it checked out.
    worktree: Option<String>,
}

/// The local branches, with where each is checked out.
fn heads(repo: &Repository) -> Result<Vec<Head>> {
    let out = exe::run(
        repo,
        [
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(HEAD)%00%(worktreepath)%00",
            "refs/heads",
        ],
        None,
    )?;
    // Every field ends in a NUL, so a path with a line break in it stays
    // one field; the break between records leads the next one's name.
    let fields: Vec<&str> = out.stdout.split('\0').collect();
    Ok(fields
        .as_chunks::<4>()
        .0
        .iter()
        .filter_map(|[name, tip, head, path]| {
            let name = name.trim_start_matches('\n').strip_prefix("refs/heads/")?;
            let current = *head == "*";
            Some(Head {
                name: name.to_owned(),
                tip: tip.parse().ok(),
                current,
                worktree: (!current && !path.is_empty()).then(|| (*path).to_owned()),
            })
        })
        .collect())
}

/// The commit of a local branch, when the name is a branch name and there
/// is such a branch. A name out of the configuration is not trusted to be
/// one: `main~1` would resolve.
fn local_branch(repo: &Repository, name: &str) -> Option<Oid> {
    plain(say!("flow-what-branch-name"), name).ok()?;
    branch::check_branch_name(repo, name).ok()?;
    branch::branch_tip(repo, name)
}

/// Whether everything `ancestor` has is in `of`.
fn is_ancestor(repo: &Repository, ancestor: Oid, of: Oid) -> bool {
    let (ancestor, of) = (ancestor.to_hex(), of.to_hex());
    exe::run_with_code(
        repo,
        [
            "merge-base",
            "--is-ancestor",
            ancestor.as_str(),
            of.as_str(),
        ],
        None,
    )
    .is_ok_and(|(code, _)| code == Some(0))
}

/// Whether two commits have any history in common. Git refuses to merge
/// two that have none.
fn related(repo: &Repository, one: Oid, other: Oid) -> bool {
    let (one, other) = (one.to_hex(), other.to_hex());
    exe::run_with_code(repo, ["merge-base", one.as_str(), other.as_str()], None)
        .is_ok_and(|(code, _)| code == Some(0))
}

/// Why gitflow cannot be used with these two names, if it cannot.
fn problem(repo: &Repository, main: Option<&str>, develop: Option<&str>) -> Option<String> {
    let (Some(main), Some(develop)) = (main, develop) else {
        return Some(say!("flow-not-set-up").to_owned());
    };
    if main == develop {
        return Some(say!("flow-same-branches", main));
    }
    if local_branch(repo, main).is_none() {
        return Some(say!("flow-no-main", main));
    }
    if local_branch(repo, develop).is_none() {
        return Some(say!("flow-no-develop", develop));
    }
    None
}

/// How gitflow stands here: the settings, and the branches of each kind.
pub fn status(repo: &Repository) -> Result<Status> {
    let raw = configured(repo)?;
    let (main, develop) = (named(&raw, KEY_MAIN), named(&raw, KEY_DEVELOP));
    let prefixes = prefixes_of(&raw);
    let reason = problem(repo, main.as_deref(), develop.as_deref());
    let heads = heads(repo)?;
    let of = |kind: Kind| -> Vec<FlowBranch> {
        let prefix = prefixes.of(kind);
        heads
            .iter()
            .filter(|head| {
                Some(head.name.as_str()) != main.as_deref()
                    && Some(head.name.as_str()) != develop.as_deref()
            })
            .filter_map(|head| {
                let name = head.name.strip_prefix(prefix)?;
                (!name.is_empty()).then_some(FlowBranch {
                    name: name.to_owned(),
                    branch: head.name.clone(),
                    tip: head.tip?,
                    current: head.current,
                })
            })
            .collect()
    };
    Ok(Status {
        initialized: reason.is_none(),
        features: of(Kind::Feature),
        bugfixes: of(Kind::Bugfix),
        releases: of(Kind::Release),
        hotfixes: of(Kind::Hotfix),
        supports: of(Kind::Support),
        reason,
        main,
        develop,
        prefixes,
    })
}

/// The settings, or the refusal every command but set-up gives without.
fn require(repo: &Repository, what: &'static str) -> Result<Settings> {
    let raw = configured(repo)?;
    let (main, develop) = (named(&raw, KEY_MAIN), named(&raw, KEY_DEVELOP));
    if let Some(reason) = problem(repo, main.as_deref(), develop.as_deref()) {
        return Err(refused(what, say!("flow-set-up-first", reason)));
    }
    match (main, develop) {
        (Some(main), Some(develop)) => Ok(Settings {
            main,
            develop,
            prefixes: prefixes_of(&raw),
        }),
        _ => Err(refused(what, say!("flow-not-set-up"))),
    }
}

/// What set-up is asked for. What is absent stays as it is configured,
/// and is the default where nothing is.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct InitOptions {
    /// The branch releases are made on.
    pub main: Option<String>,
    /// The branch work is gathered on.
    pub develop: Option<String>,
    /// Prefix of feature branches.
    pub feature: Option<String>,
    /// Prefix of bugfix branches.
    pub bugfix: Option<String>,
    /// Prefix of release branches.
    pub release: Option<String>,
    /// Prefix of hotfix branches.
    pub hotfix: Option<String>,
    /// Prefix of support branches.
    pub support: Option<String>,
    /// Prefix of the tags; may be empty.
    pub tag: Option<String>,
}

impl InitOptions {
    /// The prefix asked for one kind.
    fn of(&self, kind: Kind) -> &Option<String> {
        match kind {
            Kind::Feature => &self.feature,
            Kind::Bugfix => &self.bugfix,
            Kind::Release => &self.release,
            Kind::Hotfix => &self.hotfix,
            Kind::Support => &self.support,
        }
    }
}

/// What set-up did.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Initialized {
    /// How gitflow stands now.
    pub status: Status,
    /// The commit the develop branch was created at, when it was missing.
    pub created: Option<Oid>,
}

/// Write the settings, and create the develop branch from the main one
/// when there is none. Nothing is checked out.
pub fn init(repo: &Repository, options: &InitOptions) -> Result<Initialized> {
    let what = say!("flow-what-gitflow");
    let raw = configured(repo)?;
    let heads = heads(repo)?;
    if heads.is_empty() {
        return Err(refused(what, say!("flow-no-commit-yet")));
    }
    let exists = |name: &str| heads.iter().any(|head| head.name == name);

    let main = options
        .main
        .clone()
        .or_else(|| named(&raw, KEY_MAIN).filter(|name| exists(name)))
        .or_else(|| {
            ["main", "master"]
                .into_iter()
                .find(|name| exists(name))
                .map(str::to_owned)
        })
        .ok_or_else(|| refused(what, say!("flow-no-main-or-master")))?;
    plain(say!("flow-what-main-branch"), &main)?;
    branch::check_branch_name(repo, &main)?;
    let main_tip = heads
        .iter()
        .find(|head| head.name == main)
        .and_then(|head| head.tip)
        .ok_or_else(|| refused(what, say!("flow-no-branch", name = main.as_str())))?;

    let develop = options
        .develop
        .clone()
        .or_else(|| named(&raw, KEY_DEVELOP))
        .unwrap_or_else(|| "develop".to_owned());
    plain(say!("flow-what-develop-branch"), &develop)?;
    branch::check_branch_name(repo, &develop)?;
    if develop == main {
        return Err(refused(what, say!("flow-cannot-both-be", main)));
    }

    let chosen = |given: &Option<String>, key: &str, default: &str| {
        given
            .clone()
            .or_else(|| raw.get(key).cloned())
            .unwrap_or_else(|| default.to_owned())
    };
    let of = |kind: Kind| chosen(options.of(kind), kind.key(), kind.default_prefix());
    let prefixes = Prefixes {
        feature: of(Kind::Feature),
        bugfix: of(Kind::Bugfix),
        release: of(Kind::Release),
        hotfix: of(Kind::Hotfix),
        support: of(Kind::Support),
        tag: chosen(&options.tag, KEY_TAG, ""),
    };
    for kind in Kind::ALL {
        let prefix = prefixes.of(kind);
        // A branch of the kind is the prefix and a name: try one.
        let usable = plain(say!("flow-what-prefix"), prefix).is_ok()
            && branch::check_branch_name(repo, &format!("{prefix}x")).is_ok();
        if !usable {
            return Err(invalid(
                say!("flow-what-prefix"),
                say!(
                    "flow-prefix-unusable",
                    prefix = format!("{prefix:?}"),
                    kind = kind.noun()
                ),
            ));
        }
        if let Some(other) = Kind::ALL
            .into_iter()
            .find(|other| *other != kind && prefixes.of(*other) == prefix)
        {
            return Err(invalid(
                say!("flow-what-prefix"),
                say!(
                    "flow-prefix-shared",
                    kind = kind.noun(),
                    other = other.noun(),
                    prefix = format!("{prefix:?}")
                ),
            ));
        }
    }
    // The tag prefix may be empty; with something after it, it is a tag.
    if prefixes.tag.len() > MAX_NAME
        || tag::check_tag_name(repo, &format!("{}x", prefixes.tag)).is_err()
    {
        return Err(invalid(
            say!("flow-what-prefix"),
            say!(
                "flow-tag-prefix-unusable",
                prefix = format!("{:?}", prefixes.tag)
            ),
        ));
    }

    let mut created = None;
    if !exists(&develop) {
        // A develop branch that is only on a remote is the one to work
        // on; a new one from main would part ways with it.
        let remote = exe::run(
            repo,
            ["for-each-ref", "--format=%(refname)", "refs/remotes"],
            None,
        )?;
        if let Some(found) = remote
            .stdout
            .lines()
            .filter_map(|name| name.strip_prefix("refs/remotes/"))
            .find(|short| {
                short
                    .split_once('/')
                    .is_some_and(|(_, rest)| rest == develop)
            })
        {
            return Err(refused(
                what,
                say!("flow-develop-on-remote", develop = develop.as_str(), found),
            ));
        }
        created = Some(branch::branch_create(
            repo,
            &develop,
            Some(&main_tip.to_hex()),
        )?);
    }

    let mut writes = vec![(KEY_MAIN, main.as_str()), (KEY_DEVELOP, develop.as_str())];
    writes.extend(Kind::ALL.map(|kind| (kind.key(), prefixes.of(kind))));
    writes.push((KEY_TAG, prefixes.tag.as_str()));
    for (key, value) in writes {
        if let Err(err) = config_set(repo, key, value) {
            // Half set up is worse than not: take the branch back.
            if let Some(target) = created {
                let _ = branch::branch_move(repo, &develop, None, Some(target));
            }
            return Err(err);
        }
    }
    Ok(Initialized {
        status: status(repo)?,
        created,
    })
}

/// The branch of a kind with this name, checked as a whole.
fn branch_of(repo: &Repository, prefixes: &Prefixes, kind: Kind, name: &str) -> Result<String> {
    plain(say!("flow-what-name"), name)?;
    let branch = format!("{}{name}", prefixes.of(kind));
    plain(say!("flow-what-branch-name"), &branch)?;
    branch::check_branch_name(repo, &branch)?;
    Ok(branch)
}

/// The tag a release or hotfix of this name leaves, checked.
fn tag_of(repo: &Repository, prefixes: &Prefixes, name: &str) -> Result<String> {
    let tag = format!("{}{name}", prefixes.tag);
    plain(say!("flow-what-tag-name"), &tag)?;
    tag::check_tag_name(repo, &tag)?;
    Ok(tag)
}

/// A branch that was started.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Started {
    pub kind: Kind,
    /// Its name without the prefix.
    pub name: String,
    /// The branch that was created and checked out.
    pub branch: String,
    /// What it was started from.
    pub base: String,
    /// The commit it points at.
    pub target: Oid,
    /// HEAD before.
    pub before: HeadState,
    /// HEAD after.
    pub after: HeadState,
}

/// Start a branch of a kind and check it out: a feature, a bugfix or a
/// release from develop, a hotfix or a support branch from main, or from
/// `base` when one is given. Local
/// changes come along when git can carry them and make it refuse when it
/// cannot; the branch is then not left behind.
pub fn start(repo: &Repository, kind: Kind, name: &str, base: Option<&str>) -> Result<Started> {
    let what = say!("flow-what-start");
    let settings = require(repo, what)?;
    if repo.workdir().is_none() {
        return Err(refused(what, say!("flow-no-worktree-checkout")));
    }
    let branch = branch_of(repo, &settings.prefixes, kind, name)?;
    if branch::branch_tip(repo, &branch).is_some() {
        return Err(refused(
            what,
            say!("flow-branch-exists", branch = branch.as_str()),
        ));
    }
    if kind.tagged() {
        // Finishing would have nowhere to put its tag.
        let tag = tag_of(repo, &settings.prefixes, name)?;
        if tag::tag_value(repo, &tag).is_some() {
            return Err(refused(what, say!("flow-tag-exists", tag = tag.as_str())));
        }
    }
    rewrite::ensure_idle(repo, what)?;
    let (base, start) = match base {
        Some(given) => (given.to_owned(), given.to_owned()),
        None => {
            let from = if kind.starts_on_main() {
                &settings.main
            } else {
                &settings.develop
            };
            // By its commit: a tag of the same name must not be taken.
            let tip = local_branch(repo, from)
                .ok_or_else(|| refused(what, say!("flow-no-branch", name = from.as_str())))?;
            (from.clone(), tip.to_hex())
        }
    };
    let target = branch::branch_create(repo, &branch, Some(&start))?;
    match branch::checkout(repo, &branch, Dirty::Refuse) {
        Ok(outcome) => Ok(Started {
            kind,
            name: name.to_owned(),
            branch,
            base,
            target,
            before: outcome.before,
            after: outcome.after,
        }),
        Err(err) => {
            // Half done is worse than not done: take the branch back.
            let _ = branch::branch_move(repo, &branch, None, Some(target));
            Err(err)
        }
    }
}

/// One step of a finish.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Check out the main branch.
    CheckoutMain,
    /// Merge the branch into main, always with a merge commit.
    MergeMain,
    /// Tag main.
    Tag,
    /// Check out the develop branch.
    CheckoutDevelop,
    /// Merge the branch into develop, always with a merge commit.
    MergeDevelop,
    /// Delete the branch.
    Delete,
}

/// Whether a step is still to do.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Need {
    /// It is to do.
    Do,
    /// The repository is already as the step would leave it.
    Already,
    /// It was asked not to be done: the branch is kept.
    Skip,
}

/// A step and whether it is still to do.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct Planned {
    pub step: Step,
    /// Whether it is to do.
    pub need: Need,
}

/// A finish, as far as it can be told before anything is changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FinishPlan {
    pub kind: Kind,
    /// The name without the prefix.
    pub name: String,
    /// The branch that is finished.
    pub branch: String,
    /// The main branch.
    pub main: String,
    /// The develop branch.
    pub develop: String,
    /// The tag a release or hotfix leaves on main.
    pub tag: Option<String>,
    /// The message of that tag.
    pub message: Option<String>,
    /// The steps in order.
    pub steps: Vec<Planned>,
}

impl FinishPlan {
    /// The branch a step works on.
    pub fn onto(&self, step: Step) -> &str {
        match step {
            Step::CheckoutMain | Step::MergeMain | Step::Tag => &self.main,
            Step::CheckoutDevelop | Step::MergeDevelop | Step::Delete => &self.develop,
        }
    }
}

/// The first few of `paths`, and how many more there are.
fn listed(paths: &[String]) -> String {
    const SHOWN: usize = 5;
    let mut text = paths
        .iter()
        .take(SHOWN)
        .map(String::as_str)
        .collect::<Vec<_>>()
        .join(", ");
    let more = paths.len().saturating_sub(SHOWN);
    if more > 0 {
        text = say!("flow-listed-more", shown = text.as_str(), more);
    }
    text
}

/// Refuse a finish that local work is in the way of: any change to a
/// tracked file, since a finish checks out and merges, and an untracked
/// file where one of `tips`, the commits it brings to the worktree, has
/// a file.
fn ensure_clear(repo: &Repository, head: &HeadState, tips: &[Oid]) -> Result<()> {
    let what = say!("flow-what-finish");
    let status = crate::status(repo)?;
    let changed: Vec<String> = status
        .iter()
        .filter(|entry| rewrite::is_tracked_change(entry))
        .map(|entry| entry.path.clone())
        .collect();
    if !changed.is_empty() {
        return Err(refused(
            what,
            say!("flow-changes-in-way", paths = listed(&changed)),
        ));
    }
    let mut blocked = Vec::new();
    for tip in tips {
        if Some(*tip) == head.commit {
            continue;
        }
        let found = rewrite::untracked_in_the_way(repo, &status, head.commit, *tip).map_err(
            |err| match err {
                Error::Refused { detail, .. } => refused(what, detail),
                other => other,
            },
        )?;
        blocked.extend(found);
    }
    blocked.sort();
    blocked.dedup();
    if !blocked.is_empty() {
        return Err(refused(
            what,
            say!("flow-untracked-in-way", paths = listed(&blocked)),
        ));
    }
    Ok(())
}

/// Look at what finishing a branch still takes, and refuse now whatever
/// would make a step refuse later: not set up, no such branch, a tag of
/// that name on something else, an operation in progress, local changes
/// in the way, a branch held by another worktree, histories that have
/// nothing in common. Nothing is changed.
pub fn finish_plan(
    repo: &Repository,
    kind: Kind,
    name: &str,
    message: Option<&str>,
    keep: bool,
) -> Result<FinishPlan> {
    let what = say!("flow-what-finish");
    let settings = require(repo, what)?;
    if !kind.finishes() {
        return Err(refused(
            what,
            say!("flow-support-not-finished", main = settings.main.as_str()),
        ));
    }
    if repo.workdir().is_none() {
        return Err(refused(what, say!("flow-no-worktree-merge")));
    }
    let branch = branch_of(repo, &settings.prefixes, kind, name)?;
    let (main, develop) = (settings.main.clone(), settings.develop.clone());
    if branch == main || branch == develop {
        return Err(refused(
            what,
            say!("flow-branch-is-base", branch = branch.as_str()),
        ));
    }
    let tag = if kind.tagged() {
        Some(tag_of(repo, &settings.prefixes, name)?)
    } else {
        None
    };
    let message = message.map(str::trim).filter(|text| !text.is_empty());
    if message.is_some_and(|text| text.contains('\0') || text.len() > MAX_MESSAGE) {
        return Err(invalid(
            say!("flow-what-tag-message"),
            say!("flow-tag-message-unusable"),
        ));
    }
    let Some(tip) = branch::branch_tip(repo, &branch) else {
        let detail = match &tag {
            Some(tag) if tag::tag_value(repo, tag).is_some() => say!(
                "flow-no-branch-tagged",
                branch = branch.as_str(),
                tag = tag.as_str()
            ),
            _ => say!("flow-no-branch", name = branch.as_str()),
        };
        return Err(refused(what, detail));
    };
    // Merges are given the short name, for git's own wording of the
    // message. It has to mean the branch.
    if rewrite::commit_of(repo, "branch", &branch).ok() != Some(tip) {
        return Err(refused(
            what,
            say!("flow-ambiguous", branch = branch.as_str()),
        ));
    }
    let missing = |name: &str| refused(what, say!("flow-no-branch", name));
    let main_tip = local_branch(repo, &main).ok_or_else(|| missing(&main))?;
    let develop_tip = local_branch(repo, &develop).ok_or_else(|| missing(&develop))?;
    rewrite::ensure_idle(repo, what)?;

    let mut tag_there = false;
    if let Some(tag) = &tag
        && tag::tag_value(repo, tag).is_some()
    {
        // There already: fine when it is this release that it marks.
        let marks = rewrite::commit_of(repo, "tag", &format!("refs/tags/{tag}"))
            .is_ok_and(|commit| is_ancestor(repo, tip, commit));
        if !marks {
            return Err(refused(
                what,
                say!(
                    "flow-tag-elsewhere",
                    tag = tag.as_str(),
                    branch = branch.as_str()
                ),
            ));
        }
        tag_there = true;
    }

    let head = branch::head_state(repo)?;
    let need = |todo: bool| if todo { Need::Do } else { Need::Already };
    let mut steps = Vec::new();
    // Where HEAD will be as the steps go.
    let mut on = head.branch.clone();
    let merge_main = kind.tagged() && !is_ancestor(repo, tip, main_tip);
    if kind.tagged() {
        steps.push(Planned {
            step: Step::CheckoutMain,
            need: need(merge_main && on.as_deref() != Some(main.as_str())),
        });
        if merge_main {
            on = Some(main.clone());
        }
        steps.push(Planned {
            step: Step::MergeMain,
            need: need(merge_main),
        });
        steps.push(Planned {
            step: Step::Tag,
            need: need(!tag_there),
        });
    }
    let merge_develop = !is_ancestor(repo, tip, develop_tip);
    // The branch cannot be deleted from under HEAD.
    let to_develop = merge_develop || (!keep && on.as_deref() == Some(branch.as_str()));
    let checkout_develop = to_develop && on.as_deref() != Some(develop.as_str());
    steps.push(Planned {
        step: Step::CheckoutDevelop,
        need: need(checkout_develop),
    });
    steps.push(Planned {
        step: Step::MergeDevelop,
        need: need(merge_develop),
    });
    steps.push(Planned {
        step: Step::Delete,
        need: if keep { Need::Skip } else { Need::Do },
    });

    let does = |step: Step| {
        steps
            .iter()
            .any(|planned| planned.step == step && planned.need == Need::Do)
    };
    for (merges, onto, onto_tip) in [
        (merge_main, &main, main_tip),
        (merge_develop, &develop, develop_tip),
    ] {
        if merges && !related(repo, tip, onto_tip) {
            return Err(refused(
                what,
                say!(
                    "flow-unrelated",
                    branch = branch.as_str(),
                    onto = onto.as_str()
                ),
            ));
        }
    }
    let held = heads(repo)?;
    let held_at = |name: &str| {
        held.iter()
            .find(|head| head.name == name)
            .and_then(|head| head.worktree.clone())
    };
    for (step, name) in [
        (Step::CheckoutMain, &main),
        (Step::CheckoutDevelop, &develop),
        (Step::Delete, &branch),
    ] {
        if does(step)
            && let Some(path) = held_at(name)
        {
            return Err(refused(what, say!("flow-held", name = name.as_str(), path)));
        }
    }
    // A merge is where a finish could no longer refuse with nothing done,
    // so local work has to be out of its way before the first step. With
    // only a checkout left, git decides as for any checkout.
    let mut written = Vec::new();
    if merge_main {
        written.extend([main_tip, tip]);
    }
    if merge_develop {
        written.extend([develop_tip, tip]);
    }
    if !written.is_empty() {
        ensure_clear(repo, &head, &written)?;
    }

    let message = tag.as_ref().map(|_| match message {
        Some(text) => text.to_owned(),
        None => {
            let mut noun = kind.noun().to_owned();
            if let Some(first) = noun.get_mut(..1) {
                first.make_ascii_uppercase();
            }
            format!("{noun} {name}")
        }
    });
    Ok(FinishPlan {
        kind,
        name: name.to_owned(),
        branch,
        main,
        develop,
        tag,
        message,
        steps,
    })
}

/// Merge the finished branch into the branch `step` works on, which HEAD
/// has to be on by now. Always a merge commit, with git's own message. A
/// conflict stops it like any merge.
pub fn finish_merge(repo: &Repository, plan: &FinishPlan, step: Step) -> Result<Moved> {
    let onto = plan.onto(step);
    if branch::head_state(repo)?.branch.as_deref() != Some(onto) {
        return Err(refused(
            say!("flow-what-finish"),
            say!("flow-head-not-on", onto),
        ));
    }
    rewrite::merge(repo, &plan.branch, MergeMode::NoFf)
}

/// Tag the tip of main for a finished release or hotfix. Returns the tag
/// and what its reference holds.
pub fn finish_tag(repo: &Repository, plan: &FinishPlan) -> Result<(String, Oid)> {
    let (Some(tag), Some(message)) = (&plan.tag, &plan.message) else {
        return Err(refused(
            say!("flow-what-finish"),
            say!("flow-feature-no-tag"),
        ));
    };
    let main = local_branch(repo, &plan.main).ok_or_else(|| {
        refused(
            say!("flow-what-finish"),
            say!("flow-no-branch", name = plan.main.as_str()),
        )
    })?;
    let value = tag::tag_create(repo, tag, Some(&main.to_hex()), Some(message), false)?;
    Ok((tag.clone(), value))
}

/// Delete the finished branch, once everything it has is where the finish
/// merges it. Git's own check is against the upstream or HEAD, which is
/// another question, so the branch is deleted without it.
pub fn finish_delete(repo: &Repository, plan: &FinishPlan) -> Result<DeletedBranch> {
    let what = say!("flow-what-finish");
    let tip = branch::branch_tip(repo, &plan.branch)
        .ok_or_else(|| refused(what, say!("flow-no-branch", name = plan.branch.as_str())))?;
    let mut into = vec![&plan.develop];
    if plan.tag.is_some() {
        into.push(&plan.main);
    }
    for name in into {
        if !local_branch(repo, name).is_some_and(|onto| is_ancestor(repo, tip, onto)) {
            return Err(refused(
                what,
                say!(
                    "flow-not-merged",
                    branch = plan.branch.as_str(),
                    name = name.as_str()
                ),
            ));
        }
    }
    branch::branch_delete(repo, &plan.branch, true)
}

#[cfg(test)]
mod tests {
    use super::{Kind, Prefixes, listed, plain};

    #[test]
    fn names_that_git_could_misread_are_refused() {
        for bad in ["", "-x", "--force", "a\nb", "a\u{0}b", "\u{1b}[0m"] {
            assert!(plain("name", bad).is_err(), "{bad:?}");
        }
        assert!(plain("name", &"n".repeat(201)).is_err());
        assert!(plain("name", &"n".repeat(200)).is_ok());
        assert!(plain("name", "1.2").is_ok());
    }

    #[test]
    fn a_long_list_is_cut_and_counted() {
        let paths: Vec<String> = (0..8).map(|n| format!("f{n}")).collect();
        assert_eq!(listed(&paths), "f0, f1, f2, f3, f4 and 3 more");
        assert_eq!(listed(paths.get(..2).unwrap()), "f0, f1");
        assert_eq!(listed(&[]), "");
    }

    #[test]
    fn each_kind_has_its_prefix() {
        let prefixes = Prefixes {
            feature: "f/".to_owned(),
            bugfix: "b/".to_owned(),
            release: "r/".to_owned(),
            hotfix: "h/".to_owned(),
            support: "s/".to_owned(),
            tag: "v".to_owned(),
        };
        assert_eq!(prefixes.of(Kind::Feature), "f/");
        assert_eq!(prefixes.of(Kind::Bugfix), "b/");
        assert_eq!(prefixes.of(Kind::Release), "r/");
        assert_eq!(prefixes.of(Kind::Hotfix), "h/");
        assert_eq!(prefixes.of(Kind::Support), "s/");
        assert!(!Kind::Feature.tagged() && !Kind::Bugfix.tagged() && !Kind::Support.tagged());
        assert!(Kind::Release.tagged() && Kind::Hotfix.tagged());
        // What mends or carries on a release starts from main.
        let from_main: Vec<Kind> = Kind::ALL
            .into_iter()
            .filter(|kind| kind.starts_on_main())
            .collect();
        assert_eq!(from_main, [Kind::Hotfix, Kind::Support]);
        let never: Vec<Kind> = Kind::ALL
            .into_iter()
            .filter(|kind| !kind.finishes())
            .collect();
        assert_eq!(never, [Kind::Support]);
        // No two kinds share a key or a default.
        for (ix, kind) in Kind::ALL.into_iter().enumerate() {
            for other in Kind::ALL.into_iter().skip(ix + 1) {
                assert_ne!(kind.key(), other.key());
                assert_ne!(kind.default_prefix(), other.default_prefix());
                assert_ne!(kind.noun(), other.noun());
            }
        }
    }
}
