//! Profile commands: identities kept under a name of their own in
//! `profiles.toml`, listed, saved and deleted there, compared with the
//! identity in effect, and written to the repository's configuration,
//! the user's, or a configuration file that is named.
//!
//! Sources: git-config(1); the TOML specification (toml.io) for the file.

use std::{collections::HashSet, fs, io::Read as _, path::Path};

use bitbraid_git::identity::{self, Identity, NewIdentity, Scope, SigningFormat};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
};

/// The file the profiles are kept in, in the application's configuration
/// directory.
pub const FILE: &str = "profiles.toml";
/// As many profiles as are kept.
pub const MAX_PROFILES: usize = 256;
/// A file larger than this is not one that was written here, and is not
/// read.
const MAX_FILE: u64 = 1024 * 1024;
/// The longest name of a profile, in characters.
pub(crate) const MAX_NAME: usize = 64;

/// An identity under a name of its own.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// What the profile is called. No two have the same name, in whatever
    /// case it is written.
    pub name: String,
    /// `user.name`.
    pub user_name: String,
    /// `user.email`.
    pub user_email: String,
    /// The key to sign with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_key: Option<String>,
    /// How that key signs. A profile with a key always says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_format: Option<SigningFormat>,
    /// The private key file to connect with, by its full path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_key: Option<String>,
}

impl Profile {
    /// The identity, as it is written to a configuration.
    fn identity(&self) -> NewIdentity {
        NewIdentity {
            user_name: self.user_name.clone(),
            user_email: self.user_email.clone(),
            signing: self
                .signing_key
                .clone()
                .map(|key| (key, self.signing_format.unwrap_or(SigningFormat::Openpgp))),
            ssh_key: self.ssh_key.clone(),
        }
    }
}

/// The file as it is read. Anything in it that is not known is a reason
/// not to touch it: it would be gone after the next write.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    #[serde(default)]
    profiles: Vec<Profile>,
}

/// The file as it is written.
#[derive(Serialize)]
struct ToStore<'a> {
    profiles: &'a [Profile],
}

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "profile.list",
            label: say!("profile-list"),
            description: say!("profile-list-description"),
        },
        list,
    );
    let _ = registry.register(
        CommandSpec {
            id: "profile.save",
            label: say!("profile-save"),
            description: say!("profile-save-description"),
        },
        save,
    );
    let _ = registry.register(
        CommandSpec {
            id: "profile.delete",
            label: say!("profile-delete"),
            description: say!("profile-delete-description"),
        },
        delete,
    );
    let _ = registry.register(
        CommandSpec {
            id: "profile.current",
            label: say!("profile-current"),
            description: say!("profile-current-description"),
        },
        current,
    );
    let _ = registry.register(
        CommandSpec {
            id: "profile.use",
            label: say!("profile-use"),
            description: say!("profile-use-description"),
        },
        use_profile,
    );
}

fn refused(detail: impl Into<String>) -> CommandError {
    CommandError::Git(bitbraid_git::Error::Refused {
        what: say!("profile-refused"),
        detail: detail.into(),
    })
}

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

/// Where the profiles are kept, or the refusal of every profile command
/// when the application has not said.
fn directory(ctx: &CommandContext) -> Result<&Path, CommandError> {
    ctx.config_dir
        .as_deref()
        .ok_or_else(|| refused(say!("profile-no-directory")))
}

/// The name of a profile as it is kept: without blanks around it, not
/// empty, not long, and without control characters.
fn label(text: &str) -> Result<String, String> {
    let name = text.trim();
    if name.is_empty() {
        return Err(say!("profile-name-empty").to_owned());
    }
    if name.chars().count() > MAX_NAME {
        return Err(say!("profile-name-too-long", max = MAX_NAME));
    }
    if name.chars().any(char::is_control) {
        return Err(say!("profile-name-control").to_owned());
    }
    Ok(name.to_owned())
}

/// A name for comparing: two profiles are one when this is the same.
fn folded(name: &str) -> String {
    name.to_lowercase()
}

/// A profile as it is kept: the blanks around what was typed into a
/// field are gone, a blank key is no key, a key without a format is a GPG
/// key as it is to git, and a format without a key says nothing.
fn tidy(profile: Profile) -> Result<Profile, String> {
    let filled = |text: Option<String>| text.filter(|text| !text.trim().is_empty());
    let name = label(&profile.name).map_err(|why| say!("profile-its-name", why))?;
    let signing_key = filled(profile.signing_key).map(|key| key.trim().to_owned());
    let tidy = Profile {
        name,
        user_name: profile.user_name.trim().to_owned(),
        user_email: profile.user_email.trim().to_owned(),
        signing_format: signing_key
            .as_ref()
            .map(|_| profile.signing_format.unwrap_or(SigningFormat::Openpgp)),
        signing_key,
        // A path is taken as it is typed.
        ssh_key: filled(profile.ssh_key),
    };
    identity::check(&tidy.identity()).map_err(|err| err.to_string())?;
    Ok(tidy)
}

/// The beginning of a text, for an error message.
fn cut(text: &str) -> String {
    const SHOWN: usize = 200;
    match text.char_indices().nth(SHOWN) {
        Some((end, _)) => format!("{}…", text.get(..end).unwrap_or("")),
        None => text.to_owned(),
    }
}

/// The profiles that are kept. No file is no profiles. A file that
/// cannot be read as profiles, whatever the reason, is a refusal that
/// says so, and that is what keeps it from being written over.
fn load(dir: &Path) -> Result<Vec<Profile>, CommandError> {
    let path = dir.join(FILE);
    let unusable = |why: String| {
        refused(say!(
            "profile-file-unusable",
            path = path.display().to_string(),
            why
        ))
    };
    let meta = match fs::metadata(&path) {
        Ok(meta) => meta,
        // Windows says a path through a file is not found, where Unix
        // says that the file is not a directory.
        Err(err)
            if err.kind() == std::io::ErrorKind::NotFound
                && fs::metadata(dir).is_ok_and(|meta| !meta.is_dir()) =>
        {
            return Err(unusable(say!(
                "profile-file-parent-not-directory",
                directory = dir.display().to_string()
            )));
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => {
            return Err(unusable(say!(
                "profile-file-unreadable",
                error = err.to_string()
            )));
        }
    };
    if !meta.is_file() {
        return Err(unusable(say!("profile-file-not-file").to_owned()));
    }
    let too_large = || unusable(say!("profile-file-too-large", max = MAX_FILE / 1024));
    if meta.len() > MAX_FILE {
        return Err(too_large());
    }
    // Not a byte more than the bound, whatever the file has grown to
    // since it was asked.
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| {
            file.take(MAX_FILE.saturating_add(1))
                .read_to_end(&mut bytes)
        })
        .map_err(|err| unusable(say!("profile-file-unreadable", error = err.to_string())))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE {
        return Err(too_large());
    }
    let text =
        String::from_utf8(bytes).map_err(|_| unusable(say!("profile-file-not-text").to_owned()))?;
    let stored: Stored = toml::from_str(&text).map_err(|err| {
        let line = err
            .span()
            .and_then(|span| text.get(..span.start))
            .map(|before| before.matches('\n').count().saturating_add(1));
        let said = cut(err.message());
        unusable(match line {
            Some(line) => say!("profile-file-not-understood-at", line, said = said.as_str()),
            None => say!("profile-file-not-understood", said),
        })
    })?;
    if stored.profiles.len() > MAX_PROFILES {
        return Err(unusable(say!(
            "profile-file-too-many",
            count = stored.profiles.len(),
            max = MAX_PROFILES
        )));
    }
    let mut names = HashSet::new();
    let mut all = Vec::with_capacity(stored.profiles.len());
    for (index, profile) in stored.profiles.into_iter().enumerate() {
        let number = index.saturating_add(1);
        let profile =
            tidy(profile).map_err(|why| unusable(say!("profile-file-bad-profile", number, why)))?;
        if !names.insert(folded(&profile.name)) {
            return Err(unusable(say!("profile-file-same-name", number)));
        }
        all.push(profile);
    }
    Ok(all)
}

/// Write the profiles: beside the file first, then in its place, so that
/// the file is whole or is the one before.
fn store(dir: &Path, profiles: &[Profile]) -> Result<(), CommandError> {
    let text = toml::to_string_pretty(&ToStore { profiles })
        .map_err(|err| refused(say!("profile-not-written-down", error = err.to_string())))?;
    fs::create_dir_all(dir)
        .map_err(|err| refused(say!("profile-directory-not-made", error = err.to_string())))?;
    let path = dir.join(FILE);
    let beside = dir.join(format!("{FILE}.tmp"));
    fs::write(&beside, text)
        .map_err(|err| refused(say!("profile-not-written", error = err.to_string())))?;
    fs::rename(&beside, &path).map_err(|err| {
        let _ = fs::remove_file(&beside);
        refused(say!("profile-not-placed", error = err.to_string()))
    })
}

/// A profile as a command answers with it: every field, the absent ones
/// as null.
fn shown(profile: &Profile) -> Value {
    json!({
        "name": profile.name,
        "user_name": profile.user_name,
        "user_email": profile.user_email,
        "signing_key": profile.signing_key,
        "signing_format": profile.signing_format,
        "ssh_key": profile.ssh_key,
    })
}

/// Where the profile of a name is among the others.
fn position(profiles: &[Profile], name: &str) -> Option<usize> {
    let name = folded(name);
    profiles
        .iter()
        .position(|profile| folded(&profile.name) == name)
}

/// The name of a profile as a parameter.
fn named(params: &Params, parameter: &str) -> Result<String, CommandError> {
    let text: String = param(params, parameter)?;
    label(&text).map_err(|why| invalid(parameter, why))
}

fn list(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let all = load(directory(ctx)?)?;
    Ok(json!({ "profiles": all.iter().map(shown).collect::<Vec<_>>() }))
}

fn save(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let profile: Profile = param(params, "profile")?;
    let rename_from = match params.get("rename_from") {
        None | Some(Value::Null) => None,
        Some(_) => Some(named(params, "rename_from")?),
    };
    let profile = tidy(profile).map_err(|why| invalid("profile", why))?;
    let dir = directory(ctx)?;
    let mut all = load(dir)?;

    let same_name = position(&all, &profile.name);
    let (place, renamed_from) = match &rename_from {
        Some(old) => {
            let at = position(&all, old).ok_or_else(|| {
                refused(say!("profile-no-such-to-rename", name = format!("{old:?}")))
            })?;
            // Its own name in another case is still its own.
            if same_name.is_some_and(|other| other != at) {
                return Err(refused(say!(
                    "profile-exists",
                    name = format!("{:?}", profile.name)
                )));
            }
            (Some(at), all.get(at).map(|old| old.name.clone()))
        }
        None => (same_name, None),
    };
    match place.and_then(|at| all.get_mut(at)) {
        Some(kept) => *kept = profile.clone(),
        None => {
            if all.len() >= MAX_PROFILES {
                return Err(refused(say!("profile-too-many", max = MAX_PROFILES)));
            }
            all.push(profile.clone());
        }
    }
    store(dir, &all)?;
    let mut out = json!({
        "profile": shown(&profile),
        "created": place.is_none(),
        "renamed_from": renamed_from,
    });
    // The workspaces that name it by its old name name it by the new one.
    // The profile is renamed whatever comes of that; what kept a workspace
    // from following is said.
    if let Some(old) = renamed_from.as_deref().filter(|old| *old != profile.name)
        && let Some(fields) = out.as_object_mut()
    {
        match crate::workspace::rename_profile(dir, old, &profile.name) {
            Ok(0) => {}
            Ok(renamed) => {
                fields.insert("workspaces".to_owned(), json!(renamed));
            }
            Err(err) => {
                fields.insert("workspaces_error".to_owned(), json!(err.to_string()));
            }
        }
    }
    Ok(out)
}

fn delete(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name = named(params, "name")?;
    let dir = directory(ctx)?;
    // No two have one name, so this takes one out or none.
    let key = folded(&name);
    let (gone, kept): (Vec<Profile>, Vec<Profile>) = load(dir)?
        .into_iter()
        .partition(|profile| folded(&profile.name) == key);
    let gone = gone
        .first()
        .ok_or_else(|| refused(say!("profile-no-such", name = format!("{name:?}"))))?;
    store(dir, &kept)?;
    let mut out = json!({ "deleted": shown(gone) });
    // The workspaces that named it as their default profile name none: a
    // workspace whose profile is gone would only say so at every opening.
    // The profile is deleted whatever comes of that.
    if let Some(fields) = out.as_object_mut() {
        match crate::workspace::forget_profile(dir, &gone.name) {
            Ok(0) => {}
            Ok(cleared) => {
                fields.insert("workspaces".to_owned(), json!(cleared));
            }
            Err(err) => {
                fields.insert("workspaces_error".to_owned(), json!(err.to_string()));
            }
        }
    }
    Ok(out)
}

/// Where an identity is read or written, as a parameter: `"repo"`,
/// `"user"`, or `{"file": "<path>"}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Place {
    Repo,
    User,
    File(String),
}

impl Place {
    fn scope(&self) -> Scope {
        match self {
            Self::Repo => Scope::Repo,
            Self::User => Scope::User,
            Self::File(path) => Scope::File(path.into()),
        }
    }

    /// As it was given.
    fn shown(&self) -> Value {
        match self {
            Self::Repo => json!("repo"),
            Self::User => json!("user"),
            Self::File(path) => json!({ "file": path }),
        }
    }
}

/// The profile an identity is: the one with its name and its address
/// (which is the same in either case). When several have them, the one
/// whose keys are the identity's too.
fn matching<'a>(profiles: &'a [Profile], identity: &Identity) -> Option<&'a Profile> {
    let same_person = |profile: &&Profile| {
        identity.user_name.as_deref() == Some(profile.user_name.as_str())
            && identity
                .user_email
                .as_deref()
                .is_some_and(|email| email.eq_ignore_ascii_case(&profile.user_email))
    };
    let same_keys = |profile: &&Profile| {
        // A key without a format is a GPG key to git.
        let format = identity
            .signing_key
            .as_ref()
            .map(|_| identity.signing_format.as_deref().unwrap_or("openpgp"));
        identity.signing_key == profile.signing_key
            && format == profile.signing_format.map(SigningFormat::as_str)
            && identity.ssh_key == profile.ssh_key
    };
    profiles
        .iter()
        .filter(same_person)
        .find(same_keys)
        .or_else(|| profiles.iter().find(same_person))
}

fn current(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let place: Option<Place> = param_or(params, "scope", None)?;
    let dir = directory(ctx)?;
    let (identity, read_from) = match &place {
        Some(place) => {
            let scope = place.scope();
            let repo = match scope {
                Scope::Repo => Some(ctx.repo()?),
                Scope::User | Scope::File(_) => ctx.repo.as_ref(),
            };
            (identity::read(repo, &scope)?, place.shown())
        }
        None => match &ctx.repo {
            Some(repo) => (identity::effective(repo)?, json!("effective")),
            None => (identity::read(None, &Scope::User)?, json!("user")),
        },
    };
    // The identity is what it is, whatever has become of the profiles.
    let (profile, problem) = match load(dir) {
        Ok(all) => (
            matching(&all, &identity).map(|profile| profile.name.clone()),
            None,
        ),
        Err(err) => (None, Some(err.to_string())),
    };
    Ok(json!({
        "identity": identity,
        "scope": read_from,
        "profile": profile,
        "problem": problem,
    }))
}

fn use_profile(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name = named(params, "name")?;
    let place: Place = param(params, "scope")?;
    let all = load(directory(ctx)?)?;
    let profile = position(&all, &name)
        .and_then(|at| all.get(at))
        .ok_or_else(|| refused(say!("profile-no-such", name = format!("{name:?}"))))?;
    let scope = place.scope();
    let label = say!("profile-entry-use", name = profile.name.as_str());
    let identity = profile.identity();
    let written = if ctx.repo.is_some() {
        // With a repository open, what this writes to its own settings is
        // journalled: all of it for the scope of the repository, and
        // nothing for the user's (or only what a file that is named
        // happens to share with it).
        crate::settings::journalled(
            ctx,
            |repo| Ok(identity::write(Some(repo), &scope, &identity)?),
            |_| label,
        )?
    } else {
        if scope == Scope::Repo {
            return Err(CommandError::NoRepository);
        }
        identity::write(None, &scope, &identity)?
    };
    Ok(json!({
        "profile": profile.name,
        "scope": place.shown(),
        "before": written.before,
        "after": written.after,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(name: &str) -> Profile {
        Profile {
            name: name.to_owned(),
            user_name: "Ada Lovelace".to_owned(),
            user_email: "ada@example.invalid".to_owned(),
            signing_key: None,
            signing_format: None,
            ssh_key: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_profile_is_tidied_before_it_is_kept() {
        let typed = Profile {
            name: "  Work \t".to_owned(),
            user_name: " Ada Lovelace ".to_owned(),
            user_email: " ada@example.invalid\n".to_owned(),
            signing_key: Some(" ABCD1234 ".to_owned()),
            signing_format: None,
            ssh_key: Some("/keys/id work ".to_owned()),
        };
        let kept = tidy(typed).unwrap();
        assert_eq!(kept.name, "Work");
        assert_eq!(kept.user_name, "Ada Lovelace");
        assert_eq!(kept.user_email, "ada@example.invalid");
        assert_eq!(kept.signing_key.as_deref(), Some("ABCD1234"));
        assert_eq!(kept.signing_format, Some(SigningFormat::Openpgp));
        assert_eq!(kept.ssh_key.as_deref(), Some("/keys/id work "));
        // Tidy once is tidy.
        assert_eq!(tidy(kept.clone()).unwrap(), kept);

        // Blank fields are fields left empty; a format alone says nothing.
        let blank = Profile {
            signing_key: Some("  ".to_owned()),
            signing_format: Some(SigningFormat::Ssh),
            ssh_key: Some(String::new()),
            ..profile("Home")
        };
        assert_eq!(tidy(blank).unwrap(), profile("Home"));
        let ssh = Profile {
            signing_key: Some("/keys/id.pub".to_owned()),
            signing_format: Some(SigningFormat::Ssh),
            ..profile("Home")
        };
        assert_eq!(tidy(ssh.clone()).unwrap(), ssh);
    }

    #[test]
    fn a_name_is_something_short_without_control_characters() {
        assert_eq!(label(" Work ").unwrap(), "Work");
        assert_eq!(label("Ünï cödé 人").unwrap(), "Ünï cödé 人");
        assert_eq!(
            label(&"人".repeat(MAX_NAME)).unwrap().chars().count(),
            MAX_NAME
        );
        let long = "n".repeat(MAX_NAME + 1);
        for name in ["", "   ", "\n", "a\nb", "a\0", "\u{1b}[2J", long.as_str()] {
            assert!(label(name).is_err(), "{name:?} was taken");
        }
        assert_eq!(folded("WORK Ünï"), folded("work ünÏ"));
    }

    #[test]
    fn the_profile_of_an_identity_is_found_by_name_and_address() {
        let work = Profile {
            signing_key: Some("ABCD".to_owned()),
            signing_format: Some(SigningFormat::Openpgp),
            ..profile("Work")
        };
        let plain = profile("Plain");
        let other = Profile {
            user_email: "ada@home.invalid".to_owned(),
            ..profile("Home")
        };
        let all = [plain.clone(), work.clone(), other];
        let identity = |email: &str, key: Option<&str>, format: Option<&str>| Identity {
            user_name: Some("Ada Lovelace".to_owned()),
            user_email: Some(email.to_owned()),
            signing_key: key.map(str::to_owned),
            signing_format: format.map(str::to_owned),
            ..Identity::default()
        };
        let found = |identity: &Identity| matching(&all, identity).map(|p| p.name.as_str());
        // Two have the name and the address; the keys tell them apart.
        assert_eq!(
            found(&identity("ada@example.invalid", None, None)),
            Some("Plain")
        );
        assert_eq!(
            found(&identity("ada@example.invalid", Some("ABCD"), None)),
            Some("Work")
        );
        assert_eq!(
            found(&identity(
                "ada@example.invalid",
                Some("ABCD"),
                Some("openpgp")
            )),
            Some("Work")
        );
        // Keys that are nobody's: the first of the two.
        assert_eq!(
            found(&identity("ada@example.invalid", Some("ABCD"), Some("ssh"))),
            Some("Plain")
        );
        assert_eq!(
            found(&identity("ADA@Example.Invalid", None, None)),
            Some("Plain")
        );
        assert_eq!(
            found(&identity("ada@home.invalid", Some("X"), None)),
            Some("Home")
        );
        assert_eq!(found(&identity("ada@nowhere.invalid", None, None)), None);
        assert_eq!(found(&Identity::default()), None);
        let nameless = Identity {
            user_name: None,
            ..identity("ada@example.invalid", None, None)
        };
        assert_eq!(found(&nameless), None);
        assert_eq!(
            matching(&[], &identity("ada@example.invalid", None, None)),
            None
        );
    }

    #[test]
    fn the_file_holds_only_what_a_profile_has() {
        let all = [
            profile("Plain"),
            Profile {
                signing_key: Some("key \"quoted\" \\ back".to_owned()),
                signing_format: Some(SigningFormat::X509),
                ssh_key: Some("/keys/it's $HOME".to_owned()),
                ..profile("Ünï 人")
            },
        ];
        let text = toml::to_string_pretty(&ToStore { profiles: &all }).unwrap();
        assert!(!text.contains("signing_key = \"\""), "{text}");
        let back: Stored = toml::from_str(&text).unwrap();
        assert_eq!(back.profiles, all);
        assert!(toml::from_str::<Stored>("").unwrap().profiles.is_empty());
        // What is not known is not passed over.
        for text in [
            "version = 2\n",
            "[[profiles]]\nname = \"a\"\nuser_name = \"a\"\nuser_email = \"a\"\ncolour = \"red\"\n",
            "[[profiles]]\nname = \"a\"\n",
            "[[profiles]]\nname = \"a\"\nuser_name = \"a\"\nuser_email = \"a\"\nsigning_format = \"pgp\"\n",
            "profiles = \"none\"\n",
            "[profiles]\nname = \"a\"\n",
        ] {
            assert!(toml::from_str::<Stored>(text).is_err(), "{text}");
        }
    }
}
