//! An RPM package written here, without rpmbuild: the lead, the
//! signature header with the digests (no signature: the release's files
//! are signed with minisign), the main header, and the payload, a
//! cpio archive in gzip.
//!
//! The package owns its files and the directories given as its own; the
//! directories above them belong to the system (`filesystem`,
//! `hicolor-icon-theme`) and are made by rpm where they are missing. What
//! it requires is given whole, by the libraries' names as rpm's own
//! dependency generator writes them (`libxcb.so.1()(64bit)`), so that it
//! installs on any distribution that uses rpm.
//!
//! Sources: the RPM file format as documented in rpm's
//! `docs/manual/format.md` and the LSB 5.0 "Package File Format" (the
//! lead, the header structure and its index entries, data types and their
//! alignment, the region tags `HEADERSIGNATURES` and `HEADERIMMUTABLE`
//! with their trailer, the signature header's padding to eight bytes,
//! the tags' numbers and types, `rpmlib()` dependencies, the payload as
//! cpio with `./` names); rpm's `docs/manual/tags.md` and `dependencies.md`
//! (the sense flags), `rpm -K` for which digests are checked.

use anyhow::{Context, Result, bail};
use md5::Digest as _;

use crate::archive::{self, CpioEntry, Node, Tree};

/// What the package is.
#[derive(Debug, Clone)]
pub struct Package<'a> {
    pub name: &'a str,
    pub version: &'a str,
    pub release: &'a str,
    /// rpm's name of the processor (`x86_64`, `aarch64`).
    pub arch: &'a str,
    pub summary: &'a str,
    pub description: &'a str,
    pub license: &'a str,
    pub url: &'a str,
    pub build_time: u32,
    /// Names it requires, each without a version: a package, a library
    /// (`libz.so.1()(64bit)`) or a file (`/usr/bin/git`).
    pub requires: &'a [&'a str],
    /// The directories it owns, as paths of `tree`.
    pub owned_dirs: &'a [&'a str],
    /// Files that are licences, as paths of `tree`.
    pub licences: &'a [&'a str],
}

// Tags of the main header (rpm's `rpmtag.h`).
const HEADERIMMUTABLE: u32 = 63;
const HEADERSIGNATURES: u32 = 62;
const HEADERI18NTABLE: u32 = 100;
const NAME: u32 = 1000;
const VERSION: u32 = 1001;
const RELEASE: u32 = 1002;
const SUMMARY: u32 = 1004;
const DESCRIPTION: u32 = 1005;
const BUILDTIME: u32 = 1006;
const BUILDHOST: u32 = 1007;
const SIZE: u32 = 1009;
const LICENSE: u32 = 1014;
const GROUP: u32 = 1016;
const URL: u32 = 1020;
const OS: u32 = 1021;
const ARCH: u32 = 1022;
const FILESIZES: u32 = 1028;
const FILEMODES: u32 = 1030;
const FILERDEVS: u32 = 1033;
const FILEMTIMES: u32 = 1034;
const FILEDIGESTS: u32 = 1035;
const FILELINKTOS: u32 = 1036;
const FILEFLAGS: u32 = 1037;
const FILEUSERNAME: u32 = 1039;
const FILEGROUPNAME: u32 = 1040;
const SOURCERPM: u32 = 1044;
const FILEVERIFYFLAGS: u32 = 1045;
const PROVIDENAME: u32 = 1047;
const REQUIREFLAGS: u32 = 1048;
const REQUIRENAME: u32 = 1049;
const REQUIREVERSION: u32 = 1050;
const FILEDEVICES: u32 = 1095;
const FILEINODES: u32 = 1096;
const FILELANGS: u32 = 1097;
const PROVIDEFLAGS: u32 = 1112;
const PROVIDEVERSION: u32 = 1113;
const DIRINDEXES: u32 = 1116;
const BASENAMES: u32 = 1117;
const DIRNAMES: u32 = 1118;
const PAYLOADFORMAT: u32 = 1124;
const PAYLOADCOMPRESSOR: u32 = 1125;
const PAYLOADFLAGS: u32 = 1126;
const FILEDIGESTALGO: u32 = 5011;
const ENCODING: u32 = 5062;
const PAYLOADDIGEST: u32 = 5092;
const PAYLOADDIGESTALGO: u32 = 5093;

// Tags of the signature header.
const SIG_SHA256: u32 = 273;
const SIG_SIZE: u32 = 1000;
const SIG_MD5: u32 = 1004;
const SIG_PAYLOADSIZE: u32 = 1007;

/// SHA-256, as rpm numbers digest algorithms (PGP's numbers).
const SHA256_ALGO: u32 = 8;
/// A dependency's sense: equal, less, and one on rpm itself.
const SENSE_LESS: u32 = 1 << 1;
const SENSE_EQUAL: u32 = 1 << 3;
const SENSE_RPMLIB: u32 = 1 << 24;
/// A file that is a licence.
const FILE_LICENSE: u32 = 1 << 7;

/// The values of a header's entries, by rpm's types.
#[derive(Debug, Clone)]
enum Value {
    Int16(Vec<u16>),
    Int32(Vec<u32>),
    Str(String),
    Bin(Vec<u8>),
    StrArray(Vec<String>),
    I18n(String),
}

impl Value {
    fn kind(&self) -> u32 {
        match self {
            Value::Int16(_) => 3,
            Value::Int32(_) => 4,
            Value::Str(_) => 6,
            Value::Bin(_) => 7,
            Value::StrArray(_) => 8,
            Value::I18n(_) => 9,
        }
    }

    fn alignment(&self) -> usize {
        match self {
            Value::Int16(_) => 2,
            Value::Int32(_) => 4,
            _ => 1,
        }
    }

    fn count(&self) -> Result<u32> {
        let count = match self {
            Value::Int16(values) => values.len(),
            Value::Int32(values) => values.len(),
            Value::Str(_) | Value::I18n(_) => 1,
            Value::Bin(bytes) => bytes.len(),
            Value::StrArray(values) => values.len(),
        };
        u32::try_from(count).context("too many values for a header entry")
    }

    fn write(&self, out: &mut Vec<u8>) -> Result<()> {
        let string = |out: &mut Vec<u8>, text: &str| -> Result<()> {
            if text.contains('\0') {
                bail!("{text:?}: a header's string holds no NUL");
            }
            out.extend_from_slice(text.as_bytes());
            out.push(0);
            Ok(())
        };
        match self {
            Value::Int16(values) => values
                .iter()
                .for_each(|value| out.extend_from_slice(&value.to_be_bytes())),
            Value::Int32(values) => values
                .iter()
                .for_each(|value| out.extend_from_slice(&value.to_be_bytes())),
            Value::Str(text) | Value::I18n(text) => string(out, text)?,
            Value::Bin(bytes) => out.extend_from_slice(bytes),
            Value::StrArray(values) => {
                for text in values {
                    string(out, text)?;
                }
            }
        }
        Ok(())
    }
}

/// A header: its entries in the order of their tags, behind a region
/// entry (`region`) that says they are all of it, as rpm writes a package's
/// headers and checks them.
fn header(region: u32, mut entries: Vec<(u32, Value)>) -> Result<Vec<u8>> {
    entries.sort_by_key(|(tag, _)| *tag);
    if entries.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        bail!("a header holds a tag twice");
    }
    let count = entries.len() + 1;
    let mut index = Vec::with_capacity(count * 16);
    let mut data = Vec::new();
    let mut rest = Vec::with_capacity(entries.len() * 16);
    for (tag, value) in &entries {
        data.resize(data.len().next_multiple_of(value.alignment()), 0);
        let offset = i32::try_from(data.len()).context("a header too large")?;
        put_index(&mut rest, *tag, value.kind(), offset, value.count()?);
        value.write(&mut data)?;
    }
    // The region's trailer, last in the data: an entry of the region's
    // tag whose offset, negative, spans every entry of the index.
    let trailer_at = i32::try_from(data.len()).context("a header too large")?;
    let span = i32::try_from(count * 16).context("a header too large")?;
    put_index(&mut index, region, 7, trailer_at, 16);
    index.extend_from_slice(&rest);
    let mut trailer = Vec::with_capacity(16);
    put_index(&mut trailer, region, 7, -span, 16);
    data.extend_from_slice(&trailer);

    let mut out = vec![0x8e, 0xad, 0xe8, 0x01, 0, 0, 0, 0];
    out.extend_from_slice(&u32::try_from(count)?.to_be_bytes());
    out.extend_from_slice(&u32::try_from(data.len())?.to_be_bytes());
    out.extend_from_slice(&index);
    out.extend_from_slice(&data);
    Ok(out)
}

/// One entry of a header's index.
fn put_index(index: &mut Vec<u8>, tag: u32, kind: u32, offset: i32, count: u32) {
    index.extend_from_slice(&tag.to_be_bytes());
    index.extend_from_slice(&kind.to_be_bytes());
    index.extend_from_slice(&offset.to_be_bytes());
    index.extend_from_slice(&count.to_be_bytes());
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    hex(&sha2::Sha256::digest(bytes))
}

/// The package of `tree` (paths relative to `/`), as the bytes of an
/// `.rpm` file.
pub fn rpm(package: &Package<'_>, tree: &Tree) -> Result<Vec<u8>> {
    let mtime = package.build_time;
    // The files it holds: every file and link, and the directories it
    // owns, in the order of their paths.
    let mut paths: Vec<(&str, &Node)> = tree
        .iter()
        .filter(|(path, node)| {
            !matches!(node, Node::Dir { .. }) || package.owned_dirs.contains(path)
        })
        .collect();
    paths.sort_by(|a, b| a.0.cmp(b.0));
    for dir in package.owned_dirs {
        if !matches!(tree.get(dir), Some(Node::Dir { .. })) {
            bail!("{dir}: owned, but not a directory of the package");
        }
    }

    let mut dirnames: Vec<String> = Vec::new();
    let mut dirindexes = Vec::new();
    let mut basenames = Vec::new();
    let mut sizes = Vec::new();
    let mut modes = Vec::new();
    let mut digests = Vec::new();
    let mut links = Vec::new();
    let mut flags = Vec::new();
    let mut entries = Vec::new();
    let mut total: u64 = 0;
    for (ino, (path, node)) in paths.iter().enumerate() {
        let (dir, base) = match path.rsplit_once('/') {
            Some((dir, base)) => (format!("/{dir}/"), base),
            None => ("/".to_owned(), *path),
        };
        let at = match dirnames.iter().position(|known| *known == dir) {
            Some(at) => at,
            None => {
                dirnames.push(dir);
                dirnames.len() - 1
            }
        };
        dirindexes.push(u32::try_from(at)?);
        basenames.push(base.to_owned());
        let (mode, size, digest, link, data): (u32, u32, String, String, &[u8]) = match node {
            Node::File { data, mode } => (
                0o100000 | (mode & 0o7777),
                u32::try_from(data.len()).context("a file too large for this rpm")?,
                sha256_hex(data),
                String::new(),
                data.as_slice(),
            ),
            Node::Dir { mode } => (
                0o040000 | (mode & 0o7777),
                0,
                String::new(),
                String::new(),
                &[],
            ),
            Node::Link { target } => (
                0o120777,
                u32::try_from(target.len())?,
                String::new(),
                target.clone(),
                target.as_bytes(),
            ),
        };
        total += u64::from(size);
        modes.push(u16::try_from(mode).context("a mode rpm cannot hold")?);
        sizes.push(size);
        digests.push(digest);
        links.push(link);
        flags.push(if package.licences.contains(path) {
            FILE_LICENSE
        } else {
            0
        });
        entries.push(CpioEntry {
            name: format!("./{path}"),
            mode,
            ino: u32::try_from(ino + 1)?,
            mtime,
            data,
        });
    }
    let files = paths.len();
    let payload = archive::cpio(&entries)?;
    let compressed = archive::gzip(&payload)?;

    let evr = format!("{}-{}", package.version, package.release);
    let mut requires: Vec<(String, u32, String)> = package
        .requires
        .iter()
        .map(|name| ((*name).to_owned(), 0, String::new()))
        .collect();
    // What rpm itself has to understand of this package.
    for (feature, version) in [
        ("CompressedFileNames", "3.0.4-1"),
        ("FileDigests", "4.6.0-1"),
        ("PayloadFilesHavePrefix", "4.0-1"),
    ] {
        requires.push((
            format!("rpmlib({feature})"),
            SENSE_RPMLIB | SENSE_LESS | SENSE_EQUAL,
            version.to_owned(),
        ));
    }
    let arch_provide = match package.arch {
        "x86_64" => "x86-64",
        "aarch64" => "aarch-64",
        other => other,
    };
    let strings = |values: Vec<String>| Value::StrArray(values);
    let main = header(
        HEADERIMMUTABLE,
        vec![
            (HEADERI18NTABLE, strings(vec!["C".to_owned()])),
            (NAME, Value::Str(package.name.to_owned())),
            (VERSION, Value::Str(package.version.to_owned())),
            (RELEASE, Value::Str(package.release.to_owned())),
            (SUMMARY, Value::I18n(package.summary.to_owned())),
            (DESCRIPTION, Value::I18n(package.description.to_owned())),
            (BUILDTIME, Value::Int32(vec![package.build_time])),
            (BUILDHOST, Value::Str("reproducible".to_owned())),
            (
                SIZE,
                Value::Int32(vec![u32::try_from(total).context("a package too large")?]),
            ),
            (LICENSE, Value::Str(package.license.to_owned())),
            (GROUP, Value::I18n("Unspecified".to_owned())),
            (URL, Value::Str(package.url.to_owned())),
            (OS, Value::Str("linux".to_owned())),
            (ARCH, Value::Str(package.arch.to_owned())),
            (FILESIZES, Value::Int32(sizes)),
            (FILEMODES, Value::Int16(modes)),
            (FILERDEVS, Value::Int16(vec![0; files])),
            (FILEMTIMES, Value::Int32(vec![mtime; files])),
            (FILEDIGESTS, strings(digests)),
            (FILELINKTOS, strings(links)),
            (FILEFLAGS, Value::Int32(flags)),
            (FILEUSERNAME, strings(vec!["root".to_owned(); files])),
            (FILEGROUPNAME, strings(vec!["root".to_owned(); files])),
            (
                SOURCERPM,
                Value::Str(format!("{}-{evr}.src.rpm", package.name)),
            ),
            (FILEVERIFYFLAGS, Value::Int32(vec![u32::MAX; files])),
            (
                PROVIDENAME,
                strings(vec![
                    package.name.to_owned(),
                    format!("{}({arch_provide})", package.name),
                ]),
            ),
            (
                REQUIREFLAGS,
                Value::Int32(requires.iter().map(|r| r.1).collect()),
            ),
            (
                REQUIRENAME,
                strings(requires.iter().map(|r| r.0.clone()).collect()),
            ),
            (
                REQUIREVERSION,
                strings(requires.iter().map(|r| r.2.clone()).collect()),
            ),
            (FILEDEVICES, Value::Int32(vec![1; files])),
            (
                FILEINODES,
                Value::Int32((1..=u32::try_from(files)?).collect()),
            ),
            (FILELANGS, strings(vec![String::new(); files])),
            (PROVIDEFLAGS, Value::Int32(vec![SENSE_EQUAL; 2])),
            (PROVIDEVERSION, strings(vec![evr.clone(), evr.clone()])),
            (DIRINDEXES, Value::Int32(dirindexes)),
            (BASENAMES, strings(basenames)),
            (DIRNAMES, strings(dirnames)),
            (PAYLOADFORMAT, Value::Str("cpio".to_owned())),
            (PAYLOADCOMPRESSOR, Value::Str("gzip".to_owned())),
            (PAYLOADFLAGS, Value::Str("9".to_owned())),
            (FILEDIGESTALGO, Value::Int32(vec![SHA256_ALGO])),
            (ENCODING, Value::Str("utf-8".to_owned())),
            (PAYLOADDIGEST, strings(vec![sha256_hex(&compressed)])),
            (PAYLOADDIGESTALGO, Value::Int32(vec![SHA256_ALGO])),
        ],
    )?;

    let mut md5 = md5::Md5::new();
    md5.update(&main);
    md5.update(&compressed);
    let signature = header(
        HEADERSIGNATURES,
        vec![
            (SIG_SHA256, Value::Str(sha256_hex(&main))),
            (
                SIG_SIZE,
                Value::Int32(vec![
                    u32::try_from(main.len() + compressed.len()).context("a package too large")?,
                ]),
            ),
            (SIG_MD5, Value::Bin(md5.finalize().to_vec())),
            (
                SIG_PAYLOADSIZE,
                Value::Int32(vec![
                    u32::try_from(payload.len()).context("a payload too large")?,
                ]),
            ),
        ],
    )?;

    // The lead: a relic rpm still reads the magic of, and the name.
    let mut out = vec![0xed, 0xab, 0xee, 0xdb, 3, 0];
    out.extend_from_slice(&0u16.to_be_bytes()); // binary
    let archnum: u16 = match package.arch {
        "x86_64" => 1,
        "aarch64" => 19,
        _ => 0,
    };
    out.extend_from_slice(&archnum.to_be_bytes());
    let mut name = [0u8; 66];
    let full = format!("{}-{evr}", package.name);
    if full.len() >= name.len() {
        bail!("{full}: too long for the lead of an rpm");
    }
    for (to, from) in name.iter_mut().zip(full.as_bytes()) {
        *to = *from;
    }
    out.extend_from_slice(&name);
    out.extend_from_slice(&1u16.to_be_bytes()); // Linux
    out.extend_from_slice(&5u16.to_be_bytes()); // signature in a header
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(&signature);
    out.resize(out.len().next_multiple_of(8), 0);
    out.extend_from_slice(&main);
    out.extend_from_slice(&compressed);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    /// An index entry: tag, type, offset, count.
    type Entry = (u32, u32, i32, u32);

    /// A header's entries, its data, and its whole length.
    fn entries(bytes: &[u8]) -> (Vec<Entry>, &[u8], usize) {
        assert_eq!(&bytes[..8], &[0x8e, 0xad, 0xe8, 0x01, 0, 0, 0, 0]);
        let n = u32::from_be_bytes(bytes[8..12].try_into().unwrap()) as usize;
        let size = u32::from_be_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let index = (0..n)
            .map(|i| {
                let at = 16 + 16 * i;
                let word = |k: usize| bytes[at + 4 * k..at + 4 * k + 4].try_into().unwrap();
                (
                    u32::from_be_bytes(word(0)),
                    u32::from_be_bytes(word(1)),
                    i32::from_be_bytes(word(2)),
                    u32::from_be_bytes(word(3)),
                )
            })
            .collect();
        let data = &bytes[16 + 16 * n..16 + 16 * n + size];
        (index, data, 16 + 16 * n + size)
    }

    fn string_at(data: &[u8], offset: i32) -> String {
        let start = offset as usize;
        let end = start + data[start..].iter().position(|b| *b == 0).unwrap();
        String::from_utf8(data[start..end].to_vec()).unwrap()
    }

    fn sample() -> Vec<u8> {
        let mut tree = Tree::new();
        tree.file("usr/bin/tool", b"\x7fELF tool".to_vec(), 0o755)
            .unwrap();
        tree.file("usr/share/licenses/tool/LICENSE", b"MIT".to_vec(), 0o644)
            .unwrap();
        let tree = tree.with_parents();
        rpm(
            &Package {
                name: "tool",
                version: "1.2.3",
                release: "1",
                arch: "x86_64",
                summary: "A tool",
                description: "It does things.",
                license: "MIT",
                url: "https://example.invalid",
                build_time: 1_792_251_863,
                requires: &["/usr/bin/git", "libz.so.1()(64bit)"],
                owned_dirs: &["usr/share/licenses/tool"],
                licences: &["usr/share/licenses/tool/LICENSE"],
            },
            &tree,
        )
        .unwrap()
    }

    #[test]
    fn a_package_is_lead_signature_header_and_payload() {
        let out = sample();
        assert_eq!(&out[..4], &[0xed, 0xab, 0xee, 0xdb]);
        assert_eq!(&out[10..24], b"tool-1.2.3-1\0\0");
        let (signature, sig_data, sig_len) = entries(&out[96..]);
        // The region first, its trailer last in the data, spanning all.
        assert_eq!(signature[0], (62, 7, (sig_data.len() - 16) as i32, 16));
        let trailer = &sig_data[sig_data.len() - 16..];
        assert_eq!(
            i32::from_be_bytes(trailer[8..12].try_into().unwrap()),
            -((signature.len() * 16) as i32)
        );
        let main_at = (96 + sig_len).next_multiple_of(8);
        let (main, data, main_len) = entries(&out[main_at..]);
        assert_eq!(main[0].0, 63);
        // Tags in order, each offset aligned for its type and after the last.
        let mut last_end = 0;
        for pair in main[1..].windows(2) {
            assert!(pair[0].0 < pair[1].0);
        }
        for (_, kind, offset, _) in &main[1..] {
            let align = match kind {
                3 => 2,
                4 => 4,
                _ => 1,
            };
            assert_eq!(*offset as usize % align, 0);
            assert!(*offset as usize >= last_end);
            last_end = *offset as usize;
        }
        let find = |tag: u32| *main.iter().find(|e| e.0 == tag).unwrap();
        assert_eq!(string_at(data, find(NAME).2), "tool");
        assert_eq!(string_at(data, find(SOURCERPM).2), "tool-1.2.3-1.src.rpm");
        assert_eq!(find(BASENAMES).3, 3);
        assert_eq!(find(DIRNAMES).3, 3);
        assert_eq!(string_at(data, find(DIRNAMES).2), "/usr/bin/");
        // Requires: the two named and rpm's own three.
        assert_eq!(find(REQUIRENAME).3, 5);

        // The digests hold for the header and the payload.
        let header_bytes = &out[main_at..main_at + main_len];
        let payload = &out[main_at + main_len..];
        let sig_find = |tag: u32| *signature.iter().find(|e| e.0 == tag).unwrap();
        assert_eq!(
            string_at(sig_data, sig_find(SIG_SHA256).2),
            sha256_hex(header_bytes)
        );
        assert_eq!(string_at(data, find(PAYLOADDIGEST).2), sha256_hex(payload));
        let mut cpio = Vec::new();
        flate2::read::GzDecoder::new(payload)
            .read_to_end(&mut cpio)
            .unwrap();
        let size_at = sig_find(SIG_PAYLOADSIZE).2 as usize;
        assert_eq!(
            u32::from_be_bytes(sig_data[size_at..size_at + 4].try_into().unwrap()) as usize,
            cpio.len()
        );
        assert!(cpio.windows(14).any(|w| w == b"./usr/bin/tool"));
        assert!(!cpio.windows(11).any(|w| w == b"./usr/share\0"));
        // The same package, the same bytes.
        assert_eq!(out, sample());
    }
}
