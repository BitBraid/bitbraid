//! Which repository at a provider a remote is: its address read into a
//! host and a path, and the path matched to a connected account.
//!
//! The forms read are those git itself takes for a remote on another
//! machine: the scp-like `[user@]host:owner/name.git`, and the URLs
//! `ssh://[user@]host[:port]/path`, `git+ssh://…`, `https://…`,
//! `http://…` and `git://…`. The path's last part is the repository's
//! name, without `.git`; what comes before it is the owner, which at
//! GitLab may be a group with subgroups (`group/sub/name`). A slash at the
//! end is left out. Anything else is not a provider's repository and gives
//! no link: a local path, a `file://` address, a query or a fragment, a
//! part that is `.` or `..` or holds a character no provider allows in a
//! name, a space or a control character anywhere.
//!
//! Sources: git-clone(1) and git-fetch(1), "GIT URLS" (the forms of a
//! remote's address); docs.github.com "About remote repositories"
//! (`https://github.com/OWNER/REPO.git`, `git@github.com:OWNER/REPO.git`)
//! and "Using SSH over the HTTPS port" (`ssh.github.com`); docs.gitlab.com
//! "Namespaces" (nested groups) and "Reserved project and group names".

use crate::model::{Account, ProviderKind, RepoLink};

/// A remote's address, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteAddress {
    /// The host, in lower case.
    pub host: String,
    /// The port, when one was written.
    pub port: Option<u16>,
    /// Whether it is reached over SSH.
    pub ssh: bool,
    /// Everything of the path but its last part: `owner`, `group/sub`.
    pub owner: String,
    /// The last part, without `.git`.
    pub name: String,
}

impl RemoteAddress {
    /// `owner/name`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// How many parts the owner has.
    pub fn depth(&self) -> usize {
        self.owner.split('/').count()
    }
}

/// Longest address read.
const MAX_ADDRESS: usize = 2048;
/// Most parts of a path (GitLab allows twenty levels of groups).
const MAX_PARTS: usize = 21;
/// Longest part of a path.
const MAX_PART: usize = 255;

/// Whether `part` may be a part of a repository's path.
pub(crate) fn path_part(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= MAX_PART
        && part != "."
        && part != ".."
        && !part.starts_with('-')
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Whether `host` is a host's name or an IPv4 address.
fn host_name(host: &str) -> bool {
    !host.is_empty()
        && host.len() <= 253
        && !host.starts_with(['.', '-'])
        && !host.ends_with(['.', '-'])
        && !host.contains("..")
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
}

/// Read the path of an address into owner and name.
fn owner_and_name(path: &str) -> Option<(String, String)> {
    let path = path.strip_prefix('/').unwrap_or(path);
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() < 2 || parts.len() > MAX_PARTS || !parts.iter().all(|part| path_part(part)) {
        return None;
    }
    let (name, owner) = parts.split_last()?;
    Some((owner.join("/"), (*name).to_owned()))
}

/// Read a remote's address. None when it is not one of a repository on
/// another machine, or holds anything odd.
pub fn parse(text: &str) -> Option<RemoteAddress> {
    let text = text.trim();
    if text.is_empty()
        || text.len() > MAX_ADDRESS
        || text
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || !c.is_ascii())
        || text.contains(['?', '#', '\\'])
    {
        return None;
    }
    if let Some((scheme, rest)) = text.split_once("://") {
        let scheme = scheme.to_ascii_lowercase();
        let ssh = match scheme.as_str() {
            "ssh" | "git+ssh" | "ssh+git" => true,
            "https" | "http" | "git" => false,
            _ => return None,
        };
        let (authority, path) = rest.split_once('/')?;
        // A user name (and, in an old https address, a password) is left
        // out: it says who signs in, not where the repository is.
        let host_port = authority
            .rsplit_once('@')
            .map_or(authority, |(_, host)| host);
        let (host, port) = match host_port.split_once(':') {
            Some((host, port)) => {
                let port = port.parse::<u16>().ok().filter(|port| *port != 0)?;
                (host, Some(port))
            }
            None => (host_port, None),
        };
        if !host_name(host) {
            return None;
        }
        // `ssh://host/~user/repo` is a home folder, not a provider's path.
        if path.starts_with('~') {
            return None;
        }
        let (owner, name) = owner_and_name(path)?;
        return Some(RemoteAddress {
            host: host.to_ascii_lowercase(),
            port,
            ssh,
            owner,
            name,
        });
    }
    // The scp-like form: a colon before any slash.
    let (before, path) = text.split_once(':')?;
    if before.contains('/') || path.starts_with('/') || path.starts_with('~') {
        return None;
    }
    let host = before.rsplit_once('@').map_or(before, |(_, host)| host);
    if !host_name(host) {
        return None;
    }
    let (owner, name) = owner_and_name(path)?;
    Some(RemoteAddress {
        host: host.to_ascii_lowercase(),
        port: None,
        ssh: true,
        owner,
        name,
    })
}

/// The host of a site's address (`https://gitlab.example.org:8443` →
/// `gitlab.example.org`).
pub fn site_host(web: &str) -> Option<String> {
    crate::http::Url::parse(web)
        .ok()
        .map(|url| url.host().to_owned())
}

/// Whether a remote at `host` is at the site of `account`: the same host,
/// GitHub's SSH host on the HTTPS port for github.com, or one of the SSH
/// host names the user listed for that site (`aliases`).
pub fn same_site(account: &Account, host: &str, aliases: &[String]) -> bool {
    let Some(site) = site_host(&account.web) else {
        return false;
    };
    host == site
        || (account.kind == ProviderKind::Github
            && site == "github.com"
            && host == "ssh.github.com")
        || aliases.iter().any(|alias| alias.eq_ignore_ascii_case(host))
}

/// The links of an open repository's remotes to the accounts: one per
/// remote whose address is a repository at a connected account's site,
/// `origin` first and then by name. Where several accounts are at that
/// site, the default one is asked. A GitHub address has exactly an owner
/// and a name.
pub fn links(
    remotes: &[(String, String)],
    accounts: &[Account],
    aliases: impl Fn(&Account) -> Vec<String>,
) -> Vec<RepoLink> {
    let mut ordered: Vec<&(String, String)> = remotes.iter().collect();
    ordered.sort_by(|a, b| (a.0 != "origin", &a.0).cmp(&(b.0 != "origin", &b.0)));
    let mut found = Vec::new();
    for (remote, address) in ordered {
        let Some(address) = parse(address) else {
            continue;
        };
        let mut matching: Vec<&Account> = accounts
            .iter()
            .filter(|account| same_site(account, &address.host, &aliases(account)))
            .filter(|account| account.kind != ProviderKind::Github || address.depth() == 1)
            .collect();
        matching.sort_by_key(|account| !account.default);
        let Some(account) = matching.first() else {
            continue;
        };
        found.push(RepoLink {
            remote: remote.clone(),
            account: account.id.clone(),
            kind: account.kind,
            owner: address.owner.clone(),
            name: address.name.clone(),
        });
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::SignIn;

    fn read(text: &str) -> Option<(String, Option<u16>, bool, String, String)> {
        parse(text).map(|address| {
            (
                address.host,
                address.port,
                address.ssh,
                address.owner,
                address.name,
            )
        })
    }

    fn at(
        host: &str,
        port: Option<u16>,
        ssh: bool,
        owner: &str,
        name: &str,
    ) -> Option<(String, Option<u16>, bool, String, String)> {
        Some((host.into(), port, ssh, owner.into(), name.into()))
    }

    #[test]
    fn the_forms_git_takes_are_read() {
        for (text, want) in [
            (
                "git@github.com:octo/hello.git",
                at("github.com", None, true, "octo", "hello"),
            ),
            (
                "github.com:octo/hello",
                at("github.com", None, true, "octo", "hello"),
            ),
            (
                "git@GitHub.com:octo/hello.world.git/",
                at("github.com", None, true, "octo", "hello.world"),
            ),
            (
                "https://github.com/octo/hello",
                at("github.com", None, false, "octo", "hello"),
            ),
            (
                "https://github.com/octo/hello.git",
                at("github.com", None, false, "octo", "hello"),
            ),
            (
                "https://github.com/octo/hello/",
                at("github.com", None, false, "octo", "hello"),
            ),
            (
                "  https://user:pass@github.com/octo/.github  ",
                at("github.com", None, false, "octo", ".github"),
            ),
            (
                "HTTPS://gitlab.example.org:8443/group/sub/deeper/project.git",
                at(
                    "gitlab.example.org",
                    Some(8443),
                    false,
                    "group/sub/deeper",
                    "project",
                ),
            ),
            (
                "ssh://git@gitlab.example.org:2224/group/sub/name.git",
                at("gitlab.example.org", Some(2224), true, "group/sub", "name"),
            ),
            (
                "ssh://git@127.0.0.1:2224/test-group/project.git",
                at("127.0.0.1", Some(2224), true, "test-group", "project"),
            ),
            (
                "git+ssh://git@ssh.github.com:443/octo/hello.git",
                at("ssh.github.com", Some(443), true, "octo", "hello"),
            ),
            (
                "git://example.org/a/b",
                at("example.org", None, false, "a", "b"),
            ),
            (
                "http://localhost:8929/root/x_y-z.git",
                at("localhost", Some(8929), false, "root", "x_y-z"),
            ),
        ] {
            assert_eq!(read(text), want, "{text}");
        }
    }

    #[test]
    fn what_is_odd_is_not_read() {
        let long = format!("https://example.org/{}/x", "a".repeat(3000));
        let deep = format!("https://example.org/{}x", "a/".repeat(30));
        for text in [
            "",
            "   ",
            "/srv/git/project.git",
            "../relative/path",
            "file:///srv/git/project.git",
            "C:\\repos\\x",
            "ftp://example.org/a/b",
            "https://example.org/onlyone",
            "https://example.org/",
            "https://example.org",
            "git@github.com:onlyone.git",
            "git@github.com:/abs/path.git",
            "git@github.com:~user/x.git",
            "ssh://host/~user/x.git",
            "https://example.org/a/../b",
            "https://example.org/a/./b",
            "https://example.org/a//b",
            "https://example.org/a/b?x=1",
            "https://example.org/a/b#top",
            "https://example.org/a b/c",
            "https://example.org/a\n/b",
            "https://exa\u{0}mple.org/a/b",
            "https://example.org/a/caf\u{e9}",
            "https://example.org/-a/b",
            "https://example.org:0/a/b",
            "https://example.org:99999/a/b",
            "https://example.org:port/a/b",
            "https://-bad.org/a/b",
            "https://bad..org/a/b",
            "https://[::1]/a/b",
            "https://example.org/a/b%2Fc",
            long.as_str(),
            deep.as_str(),
        ] {
            assert_eq!(read(text), None, "{text:?}");
        }
    }

    fn account(kind: ProviderKind, web: &str, login: &str, default: bool) -> Account {
        let host = site_host(web).unwrap();
        Account {
            id: format!(
                "{}:{host}:{login}",
                if kind == ProviderKind::Github {
                    "github"
                } else {
                    "gitlab"
                }
            ),
            kind,
            web: web.into(),
            api: String::new(),
            login: login.into(),
            name: None,
            avatar_url: None,
            sign_in: SignIn::Token,
            scopes: Vec::new(),
            default,
        }
    }

    #[test]
    fn remotes_are_linked_to_the_accounts_at_their_site() {
        let accounts = [
            account(ProviderKind::Github, "https://github.com", "first", false),
            account(ProviderKind::Github, "https://github.com", "second", true),
            account(ProviderKind::Gitlab, "http://127.0.0.1:8929", "alice", true),
            account(
                ProviderKind::Gitlab,
                "https://gitlab.example.org",
                "bob",
                true,
            ),
        ];
        let remotes = [
            (
                "upstream".to_owned(),
                "https://github.com/up/proj.git".to_owned(),
            ),
            ("origin".to_owned(), "git@github.com:me/proj.git".to_owned()),
            ("local".to_owned(), "/srv/git/proj.git".to_owned()),
            (
                "lab".to_owned(),
                "ssh://git@127.0.0.1:2224/grp/sub/proj.git".to_owned(),
            ),
            ("aliased".to_owned(), "work-lab:team/proj.git".to_owned()),
            ("nested".to_owned(), "https://github.com/a/b/c".to_owned()),
            ("elsewhere".to_owned(), "https://example.org/x/y".to_owned()),
            (
                "tunnel".to_owned(),
                "ssh://git@ssh.github.com:443/me/other.git".to_owned(),
            ),
        ];
        let aliases = |account: &Account| {
            if account.login == "bob" {
                vec!["work-lab".to_owned()]
            } else {
                Vec::new()
            }
        };
        let found = links(&remotes, &accounts, aliases);
        let shown: Vec<(&str, &str, &str, &str)> = found
            .iter()
            .map(|link| {
                (
                    link.remote.as_str(),
                    link.account.as_str(),
                    link.owner.as_str(),
                    link.name.as_str(),
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                ("origin", "github:github.com:second", "me", "proj"),
                ("aliased", "gitlab:gitlab.example.org:bob", "team", "proj"),
                ("lab", "gitlab:127.0.0.1:alice", "grp/sub", "proj"),
                ("tunnel", "github:github.com:second", "me", "other"),
                ("upstream", "github:github.com:second", "up", "proj"),
            ]
        );
        assert!(links(&remotes, &[], |_| Vec::new()).is_empty());
    }
}
