//! Object identifiers, serialised as hex strings.

use std::{collections::HashMap, fmt, hash::BuildHasherDefault, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Error;

/// A git object id. Serialises as lowercase hex.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Oid(gix::ObjectId);

impl Oid {
    /// The all-zero id, a placeholder that never names an object.
    pub fn zero() -> Self {
        Self(gix::ObjectId::null(gix::hash::Kind::Sha1))
    }

    /// The underlying gitoxide id.
    pub fn as_gix(&self) -> &gix::ObjectId {
        &self.0
    }

    /// Full hex form.
    pub fn to_hex(&self) -> String {
        self.0.to_string()
    }

    /// First `n` hex characters (clamped to the full length).
    pub fn short(&self, n: usize) -> String {
        let hex = self.to_hex();
        let n = n.min(hex.len());
        hex.get(..n).unwrap_or(&hex).to_owned()
    }

    /// Whether the id, written in hexadecimal, begins with these digits
    /// (see [`hex_digits`]). Nothing is written out for it: an id is asked
    /// this once for every commit of a history at every key that is typed
    /// into a search.
    pub fn begins_with(&self, digits: &[u8]) -> bool {
        let bytes = self.0.as_bytes();
        digits.iter().enumerate().all(|(ix, digit)| {
            bytes.get(ix / 2).is_some_and(|byte| {
                let half = if ix % 2 == 0 { byte >> 4 } else { byte & 0x0f };
                half == *digit
            })
        })
    }
}

/// The hexadecimal digits of `text` as numbers, upper or lower case, when
/// that is all it is and it is no longer than an id can be: what
/// [`Oid::begins_with`] compares an id with.
pub fn hex_digits(text: &str) -> Option<Vec<u8>> {
    if text.len() > 64 {
        return None;
    }
    text.chars()
        .map(|c| c.to_digit(16).and_then(|digit| u8::try_from(digit).ok()))
        .collect()
}

impl From<gix::ObjectId> for Oid {
    fn from(id: gix::ObjectId) -> Self {
        Self(id)
    }
}

impl From<Oid> for gix::ObjectId {
    fn from(id: Oid) -> Self {
        id.0
    }
}

impl FromStr for Oid {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        gix::ObjectId::from_hex(s.trim().as_bytes())
            .map(Self)
            .map_err(|err| Error::Invalid {
                what: say!("oid-what"),
                detail: format!("{s:?}: {err}"),
            })
    }
}

impl fmt::Display for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl fmt::Debug for Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Oid({})", self.short(10))
    }
}

impl Serialize for Oid {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Oid {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// A hasher for keys that are hashes already, as object ids are: the
/// first eight bytes of each write are taken as they are, and the writes
/// are folded together. The standard one (SipHash) works through all
/// twenty bytes of every id against an attacker that ids written by git
/// cannot have, which makes an index of them a large part of a walk's time.
#[derive(Debug, Default, Clone, Copy)]
pub struct IdHasher(u64);

impl std::hash::Hasher for IdHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut first = [0u8; 8];
        for (into, byte) in first.iter_mut().zip(bytes) {
            *into = *byte;
        }
        self.0 = self.0.rotate_left(17) ^ u64::from_ne_bytes(first);
    }
}

/// A map keyed by object ids, hashed by [`IdHasher`].
pub type OidMap<V> = HashMap<Oid, V, BuildHasherDefault<IdHasher>>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_hex_and_json() {
        let hex = "0123456789abcdef0123456789abcdef01234567";
        let oid: Oid = hex.parse().unwrap();
        assert_eq!(oid.to_hex(), hex);
        assert_eq!(oid.short(7), "0123456");
        let json = serde_json::to_string(&oid).unwrap();
        assert_eq!(json, format!("\"{hex}\""));
        let back: Oid = serde_json::from_str(&json).unwrap();
        assert_eq!(back, oid);
    }

    #[test]
    fn ids_are_hashed_by_their_first_bytes_and_other_keys_still_work() {
        use std::hash::{BuildHasher, Hasher};
        let build = BuildHasherDefault::<IdHasher>::default();
        let a: Oid = "0123456789abcdef0123456789abcdef01234567".parse().unwrap();
        let b: Oid = "fedcba9876543210fedcba9876543210fedcba98".parse().unwrap();
        assert_ne!(build.hash_one(a), build.hash_one(b));
        assert_eq!(build.hash_one(a), build.hash_one(a));
        let mut map: OidMap<usize> = OidMap::default();
        map.insert(a, 1);
        map.insert(b, 2);
        assert_eq!((map.get(&a), map.get(&b)), (Some(&1), Some(&2)));
        // Short writes and nothing at all are no trouble.
        let mut hasher = IdHasher::default();
        hasher.write(&[]);
        hasher.write_u8(7);
        hasher.write_u64(u64::MAX);
        let _ = hasher.finish();
    }

    #[test]
    fn rejects_garbage() {
        assert!("not-hex".parse::<Oid>().is_err());
        assert!(serde_json::from_str::<Oid>("\"zz\"").is_err());
    }
}
