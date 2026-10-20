//! The archives the packages are made of, written here rather than by
//! the tools of the machine the release is built on: tar (ustar), ar,
//! cpio (the "new ASCII" format), zip, and gzip around them. Every entry
//! is owned by root, carries the one time it is given
//! (`SOURCE_DATE_EPOCH`) and comes in the order of its path, so that the
//! same files make the same archive to the byte, wherever it is made.
//!
//! Sources: POSIX.1-2017, pax, "ustar Interchange Format" (the header's
//! fields, the checksum, the name split into prefix and name); the ar
//! format as deb(5) and dpkg-deb(1) describe it (the global header, the
//! 60-byte member header, even alignment); cpio(5), "New ASCII Format"
//! (`070701`, the 13 hexadecimal fields, alignment to four bytes,
//! `TRAILER!!!`); PKWARE's APPNOTE.TXT 6.3.10 (local file header, central
//! directory header, end of central directory record, MS-DOS date and
//! time, the UTF-8 flag); RFC 1952 (gzip's MTIME, zero for none); the
//! documentation of flate2 1.1 (`GzBuilder`, `DeflateEncoder`, `Crc`).

use std::{collections::BTreeMap, io::Write as _};

use anyhow::{Context, Result, bail};
use flate2::{Compression, Crc, GzBuilder, write::DeflateEncoder};

/// What a path in a [`Tree`] is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A file, its contents and permissions (`0o755`, `0o644`).
    File { data: Vec<u8>, mode: u32 },
    /// A directory and its permissions.
    Dir { mode: u32 },
    /// A symbolic link and what it points at.
    Link { target: String },
}

/// Files, directories and links by their path, relative, with `/` between
/// its parts and none at either end. Kept in the order of their paths,
/// which puts a directory before what it holds.
#[derive(Debug, Clone, Default)]
pub struct Tree {
    nodes: BTreeMap<String, Node>,
}

impl Tree {
    pub fn new() -> Self {
        Self::default()
    }

    /// A file at `path`.
    pub fn file(&mut self, path: &str, data: Vec<u8>, mode: u32) -> Result<()> {
        self.put(path, Node::File { data, mode })
    }

    /// A directory at `path`.
    pub fn dir(&mut self, path: &str, mode: u32) -> Result<()> {
        self.put(path, Node::Dir { mode })
    }

    /// A symbolic link at `path` to `target`.
    pub fn link(&mut self, path: &str, target: &str) -> Result<()> {
        if target.is_empty() || target.contains('\0') {
            bail!("{path}: a link must point somewhere");
        }
        self.put(
            path,
            Node::Link {
                target: target.to_owned(),
            },
        )
    }

    fn put(&mut self, path: &str, node: Node) -> Result<()> {
        check_path(path)?;
        if self.nodes.insert(path.to_owned(), node).is_some() {
            bail!("{path} is in the archive twice");
        }
        Ok(())
    }

    /// The same, with a directory (`0o755`) for every parent that has
    /// none.
    pub fn with_parents(&self) -> Tree {
        let mut out = self.clone();
        for path in self.nodes.keys() {
            let mut parent = path.as_str();
            while let Some((up, _)) = parent.rsplit_once('/') {
                out.nodes
                    .entry(up.to_owned())
                    .or_insert(Node::Dir { mode: 0o755 });
                parent = up;
            }
        }
        out
    }

    /// The same under `prefix`, a directory that holds it all.
    pub fn under(&self, prefix: &str) -> Result<Tree> {
        check_path(prefix)?;
        let mut out = Tree::new();
        out.nodes
            .insert(prefix.to_owned(), Node::Dir { mode: 0o755 });
        for (path, node) in &self.nodes {
            out.put(&format!("{prefix}/{path}"), node.clone())?;
        }
        Ok(out)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Node)> {
        self.nodes.iter().map(|(path, node)| (path.as_str(), node))
    }

    pub fn get(&self, path: &str) -> Option<&Node> {
        self.nodes.get(path)
    }
}

/// A path an archive can hold: relative, its parts neither empty nor `.`
/// or `..`, no backslash (a separator on Windows) and nothing unprintable.
fn check_path(path: &str) -> Result<()> {
    let fine = !path.is_empty()
        && path
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && !path.contains('\\')
        && !path.chars().any(char::is_control);
    if !fine {
        bail!("{path:?} is not a path an archive can hold");
    }
    Ok(())
}

// ---------------------------------------------------------------------
// tar

/// The tree as a tar archive (ustar). With `dot`, every name starts with
/// `./` and the archive starts with `./` itself, as a Debian package's
/// do.
pub fn tar(tree: &Tree, mtime: u64, dot: bool) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    if dot {
        out.extend_from_slice(&tar_header("./", 0o755, 0, mtime, b'5', "")?);
    }
    let lead = if dot { "./" } else { "" };
    for (path, node) in tree.iter() {
        match node {
            Node::File { data, mode } => {
                let name = format!("{lead}{path}");
                out.extend_from_slice(&tar_header(
                    &name,
                    *mode,
                    data.len() as u64,
                    mtime,
                    b'0',
                    "",
                )?);
                out.extend_from_slice(data);
                out.resize(out.len().next_multiple_of(512), 0);
            }
            Node::Dir { mode } => {
                let name = format!("{lead}{path}/");
                out.extend_from_slice(&tar_header(&name, *mode, 0, mtime, b'5', "")?);
            }
            Node::Link { target } => {
                let name = format!("{lead}{path}");
                out.extend_from_slice(&tar_header(&name, 0o777, 0, mtime, b'2', target)?);
            }
        }
    }
    // Two empty blocks end it; GNU tar fills the last record of 10 KiB.
    out.resize(out.len() + 1024, 0);
    out.resize(out.len().next_multiple_of(10240), 0);
    Ok(out)
}

fn tar_header(
    name: &str,
    mode: u32,
    size: u64,
    mtime: u64,
    kind: u8,
    link: &str,
) -> Result<[u8; 512]> {
    let mut header = [0u8; 512];
    let (prefix, name) = split_tar_name(name)?;
    if link.len() > 100 {
        bail!("{link}: too long for a link in a tar archive");
    }
    put(&mut header[0..100], name.as_bytes());
    octal(&mut header[100..108], u64::from(mode & 0o7777))?;
    octal(&mut header[108..116], 0)?;
    octal(&mut header[116..124], 0)?;
    octal(&mut header[124..136], size)?;
    octal(&mut header[136..148], mtime)?;
    header[148..156].copy_from_slice(b"        ");
    header[156] = kind;
    put(&mut header[157..257], link.as_bytes());
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    put(&mut header[265..297], b"root");
    put(&mut header[297..329], b"root");
    octal(&mut header[329..337], 0)?;
    octal(&mut header[337..345], 0)?;
    put(&mut header[345..500], prefix.as_bytes());
    let sum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    put(&mut header[148..156], format!("{sum:06o}\0 ").as_bytes());
    Ok(header)
}

/// A name in ustar's two fields: up to 155 bytes of directories, a `/`,
/// and up to 100 bytes of the rest.
fn split_tar_name(name: &str) -> Result<(&str, &str)> {
    if name.len() <= 100 {
        return Ok(("", name));
    }
    // A directory's name ends in `/`, which stays with the last part.
    let body = name.strip_suffix('/').unwrap_or(name);
    for (at, _) in body.match_indices('/') {
        let (prefix, rest) = name.split_at(at);
        let rest = rest.get(1..).unwrap_or_default();
        if prefix.len() <= 155 && !rest.is_empty() && rest.len() <= 100 {
            return Ok((prefix, rest));
        }
    }
    bail!("{name}: too long for a tar archive")
}

/// `value` in octal, as many digits as the field holds less one, and a
/// NUL.
fn octal(field: &mut [u8], value: u64) -> Result<()> {
    let digits = field.len().saturating_sub(1);
    let text = format!("{value:0digits$o}");
    if text.len() > digits {
        bail!("{value} does not fit a field of {digits} octal digits");
    }
    put(field, text.as_bytes());
    Ok(())
}

/// `bytes` at the start of `field`, cut to its length.
fn put(field: &mut [u8], bytes: &[u8]) {
    for (to, from) in field.iter_mut().zip(bytes) {
        *to = *from;
    }
}

// ---------------------------------------------------------------------
// ar

/// An ar archive of these members, in this order, as dpkg-deb writes one:
/// no symbol table, names of up to 16 bytes, owned by root.
pub fn ar(members: &[(&str, &[u8])], mtime: u64) -> Result<Vec<u8>> {
    let mut out = b"!<arch>\n".to_vec();
    for (name, data) in members {
        if name.is_empty() || name.len() > 16 || name.contains([' ', '/']) {
            bail!("{name:?}: not a name an ar member can have");
        }
        let header = format!(
            "{name:<16}{mtime:<12}{:<6}{:<6}{:<8}{:<10}`\n",
            0,
            0,
            "100644",
            data.len()
        );
        if header.len() != 60 {
            bail!("{name}: the member does not fit an ar header");
        }
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(b'\n');
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// cpio

/// One entry of a cpio archive.
#[derive(Debug, Clone)]
pub struct CpioEntry<'a> {
    pub name: String,
    /// The whole mode, the type of file included (`0o100644`).
    pub mode: u32,
    pub ino: u32,
    pub mtime: u32,
    pub data: &'a [u8],
}

/// A cpio archive in the "new ASCII" format, owned by root, ended by its
/// trailer.
pub fn cpio(entries: &[CpioEntry<'_>]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for entry in entries {
        let size = u32::try_from(entry.data.len())
            .with_context(|| format!("{}: too large for cpio", entry.name))?;
        let nlink = if entry.mode & 0o170000 == 0o040000 {
            2
        } else {
            1
        };
        cpio_entry(
            &mut out,
            &entry.name,
            entry.ino,
            entry.mode,
            nlink,
            entry.mtime,
            size,
        )?;
        out.extend_from_slice(entry.data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    cpio_entry(&mut out, "TRAILER!!!", 0, 0, 1, 0, 0)?;
    Ok(out)
}

fn cpio_entry(
    out: &mut Vec<u8>,
    name: &str,
    ino: u32,
    mode: u32,
    nlink: u32,
    mtime: u32,
    size: u32,
) -> Result<()> {
    let namesize = u32::try_from(name.len() + 1).context("a name too long for cpio")?;
    let header = format!(
        "070701{ino:08x}{mode:08x}{:08x}{:08x}{nlink:08x}{mtime:08x}{size:08x}\
         {:08x}{:08x}{:08x}{:08x}{namesize:08x}{:08x}",
        0, 0, 0, 0, 0, 0, 0
    );
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(name.as_bytes());
    out.push(0);
    out.resize(out.len().next_multiple_of(4), 0);
    Ok(())
}

// ---------------------------------------------------------------------
// zip

/// The tree as a zip archive: files deflated where that makes them
/// smaller, directories as entries of their own, names in UTF-8, every
/// entry dated `mtime` (in MS-DOS's terms, which start in 1980 and keep
/// even seconds). Links are refused: the archive is for Windows.
pub fn zip(tree: &Tree, mtime: u64) -> Result<Vec<u8>> {
    let (time, date) = dos_time(mtime);
    let mut out = Vec::new();
    let mut central = Vec::new();
    let mut count: u16 = 0;
    for (path, node) in tree.iter() {
        let (name, mode, method, crc, stored, size) = match node {
            Node::File { data, mode } => {
                let mut crc = Crc::new();
                crc.update(data);
                let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
                encoder.write_all(data)?;
                let deflated = encoder.finish()?;
                let (method, stored) = if deflated.len() < data.len() {
                    (8u16, deflated)
                } else {
                    (0u16, data.clone())
                };
                (
                    path.to_owned(),
                    0o100000 | (mode & 0o7777),
                    method,
                    crc.sum(),
                    stored,
                    data.len(),
                )
            }
            Node::Dir { mode } => (
                format!("{path}/"),
                0o040000 | (mode & 0o7777),
                0,
                0,
                Vec::new(),
                0,
            ),
            Node::Link { .. } => bail!("{path}: a zip archive here holds no links"),
        };
        let too_large = || format!("{path}: too large for a zip archive without zip64");
        let offset = u32::try_from(out.len()).with_context(too_large)?;
        let stored_len = u32::try_from(stored.len()).with_context(too_large)?;
        let size = u32::try_from(size).with_context(too_large)?;
        let name_len = u16::try_from(name.len()).with_context(too_large)?;
        count = count
            .checked_add(1)
            .context("too many entries for a zip archive without zip64")?;
        // Bit 11: the name is UTF-8.
        let flags: u16 = 0x0800;

        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&time.to_le_bytes());
        out.extend_from_slice(&date.to_le_bytes());
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&stored_len.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&name_len.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(&stored);

        // Made on Unix (3), so that the permissions travel in the high
        // half of the external attributes; MS-DOS's directory bit beside.
        let dos = if matches!(node, Node::Dir { .. }) {
            0x10
        } else {
            0
        };
        central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        central.extend_from_slice(&((3u16 << 8) | 20).to_le_bytes());
        central.extend_from_slice(&20u16.to_le_bytes());
        central.extend_from_slice(&flags.to_le_bytes());
        central.extend_from_slice(&method.to_le_bytes());
        central.extend_from_slice(&time.to_le_bytes());
        central.extend_from_slice(&date.to_le_bytes());
        central.extend_from_slice(&crc.to_le_bytes());
        central.extend_from_slice(&stored_len.to_le_bytes());
        central.extend_from_slice(&size.to_le_bytes());
        central.extend_from_slice(&name_len.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
        central.extend_from_slice(&((mode << 16) | dos).to_le_bytes());
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(name.as_bytes());
    }
    let too_large = || "too large for a zip archive without zip64";
    let central_at = u32::try_from(out.len()).context(too_large())?;
    let central_len = u32::try_from(central.len()).context(too_large())?;
    out.extend_from_slice(&central);
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    out.extend_from_slice(&central_len.to_le_bytes());
    out.extend_from_slice(&central_at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    if u32::try_from(out.len()).is_err() {
        bail!(too_large());
    }
    Ok(out)
}

/// Seconds since 1970 (UTC) as MS-DOS's time and date: before 1980 is
/// the first moment of 1980, after 2107 the last of 2107.
fn dos_time(seconds: u64) -> (u16, u16) {
    let days = seconds / 86_400;
    let in_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    if year < 1980 {
        return (0, (1 << 5) | 1);
    }
    if year > 2107 {
        return ((23 << 11) | (59 << 5) | 29, (127 << 9) | (12 << 5) | 31);
    }
    let time = ((in_day / 3600) << 11) | (((in_day % 3600) / 60) << 5) | ((in_day % 60) / 2);
    let date = ((year - 1980) << 9) | (month << 5) | day;
    (
        u16::try_from(time).unwrap_or(0),
        u16::try_from(date).unwrap_or(0),
    )
}

/// The year, month and day of a count of days since 1970-01-01, in the
/// proleptic Gregorian calendar (`civil_from_days` of
/// "chrono-Compatible Low-Level Date Algorithms").
pub fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

// ---------------------------------------------------------------------
// gzip

/// `data` in gzip at the best compression, with no name and no time in
/// the header.
pub fn gzip(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), Compression::best());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use super::*;

    fn sample() -> Tree {
        let mut tree = Tree::new();
        tree.file("usr/bin/tool", b"#!/bin/sh\necho hi\n".to_vec(), 0o755)
            .unwrap();
        tree.file("usr/share/doc/tool/README", b"read me".to_vec(), 0o644)
            .unwrap();
        tree.link("usr/bin/alias", "tool").unwrap();
        tree.with_parents()
    }

    /// The entries of a tar archive: name, type, mode, contents and link
    /// target, each header's checksum checked.
    fn read_tar(archive: &[u8]) -> Vec<(String, u8, u32, Vec<u8>, String)> {
        assert_eq!(archive.len() % 10240, 0);
        let mut at = 0;
        let mut out = Vec::new();
        while at + 512 <= archive.len() {
            let header = &archive[at..at + 512];
            if header.iter().all(|byte| *byte == 0) {
                break;
            }
            let text = |range: std::ops::Range<usize>| {
                let field = &header[range];
                let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
                String::from_utf8(field[..end].to_vec()).unwrap()
            };
            let number = |range| u64::from_str_radix(text(range).trim(), 8).unwrap();
            let mut copy = header.to_vec();
            copy[148..156].copy_from_slice(b"        ");
            let sum: u64 = copy.iter().map(|b| u64::from(*b)).sum();
            assert_eq!(number(148..154), sum, "checksum");
            assert_eq!(&header[257..263], b"ustar\0");
            assert_eq!(text(265..297), "root");
            let prefix = text(345..500);
            let name = if prefix.is_empty() {
                text(0..100)
            } else {
                format!("{prefix}/{}", text(0..100))
            };
            let size = number(124..136) as usize;
            assert_eq!(number(136..148), 1_700_000_000);
            let data = archive[at + 512..at + 512 + size].to_vec();
            out.push((
                name,
                header[156],
                number(100..108) as u32,
                data,
                text(157..257),
            ));
            at += 512 + size.next_multiple_of(512);
        }
        out
    }

    #[test]
    fn a_tar_archive_holds_the_tree_in_order() {
        let archive = tar(&sample(), 1_700_000_000, false).unwrap();
        let entries = read_tar(&archive);
        let names: Vec<_> = entries.iter().map(|e| e.0.as_str()).collect();
        assert_eq!(
            names,
            [
                "usr/",
                "usr/bin/",
                "usr/bin/alias",
                "usr/bin/tool",
                "usr/share/",
                "usr/share/doc/",
                "usr/share/doc/tool/",
                "usr/share/doc/tool/README"
            ]
        );
        let tool = &entries[3];
        assert_eq!((tool.1, tool.2), (b'0', 0o755));
        assert_eq!(tool.3, b"#!/bin/sh\necho hi\n");
        let alias = &entries[2];
        assert_eq!((alias.1, alias.4.as_str()), (b'2', "tool"));
        assert_eq!(entries[0].1, b'5');
        // The same tree, the same bytes.
        assert_eq!(archive, tar(&sample(), 1_700_000_000, false).unwrap());
    }

    #[test]
    fn a_debian_tar_starts_with_dot() {
        let entries = read_tar(&tar(&sample(), 1_700_000_000, true).unwrap());
        assert_eq!(entries[0].0, "./");
        assert!(entries.iter().all(|e| e.0.starts_with("./")));
        assert!(entries.iter().any(|e| e.0 == "./usr/bin/tool"));
    }

    #[test]
    fn long_names_are_split_and_longer_ones_refused() {
        let deep = format!("{}/{}", "d".repeat(150), "f".repeat(90));
        let mut tree = Tree::new();
        tree.file(&deep, vec![1], 0o644).unwrap();
        let entries = read_tar(&tar(&tree, 1_700_000_000, false).unwrap());
        assert_eq!(entries[0].0, deep);
        let mut tree = Tree::new();
        tree.file(&"x".repeat(101), vec![], 0o644).unwrap();
        assert!(tar(&tree, 0, false).is_err());
    }

    #[test]
    fn odd_paths_are_refused() {
        let mut tree = Tree::new();
        for path in ["", "/abs", "a/../b", "a//b", "./a", "a/", "a\\b", "a\nb"] {
            assert!(tree.file(path, vec![], 0o644).is_err(), "{path:?}");
        }
        tree.file("a", vec![], 0o644).unwrap();
        assert!(tree.file("a", vec![], 0o644).is_err(), "twice");
        assert!(tree.link("b", "").is_err());
    }

    #[test]
    fn under_puts_everything_in_one_directory() {
        let tree = sample().under("tool-1.0").unwrap();
        assert!(matches!(tree.get("tool-1.0"), Some(Node::Dir { .. })));
        assert!(matches!(
            tree.get("tool-1.0/usr/bin/tool"),
            Some(Node::File { .. })
        ));
        assert_eq!(tree.iter().count(), sample().iter().count() + 1);
    }

    #[test]
    fn an_ar_archive_is_its_members_with_their_headers() {
        let out = ar(
            &[("debian-binary", b"2.0\n"), ("odd", b"abc")],
            1_700_000_000,
        )
        .unwrap();
        assert_eq!(&out[..8], b"!<arch>\n");
        let header = std::str::from_utf8(&out[8..68]).unwrap();
        assert_eq!(&header[..16], "debian-binary   ");
        assert_eq!(header[16..28].trim(), "1700000000");
        assert_eq!(header[40..48].trim(), "100644");
        assert_eq!(header[48..58].trim(), "4");
        assert_eq!(&header[58..], "`\n");
        assert_eq!(&out[68..72], b"2.0\n");
        let second = std::str::from_utf8(&out[72..132]).unwrap();
        assert_eq!(second[48..58].trim(), "3");
        // Odd members are padded to an even length.
        assert_eq!(&out[132..], b"abc\n");
        assert!(ar(&[("a name far too long", b"")], 0).is_err());
    }

    #[test]
    fn a_cpio_archive_aligns_and_ends_with_its_trailer() {
        let data = b"hello";
        let out = cpio(&[CpioEntry {
            name: "./usr/bin/hello".to_owned(),
            mode: 0o100755,
            ino: 1,
            mtime: 1_700_000_000,
            data,
        }])
        .unwrap();
        let header = std::str::from_utf8(&out[..110]).unwrap();
        assert_eq!(&header[..6], "070701");
        let field = |n: usize| u32::from_str_radix(&header[6 + 8 * n..14 + 8 * n], 16).unwrap();
        assert_eq!(field(0), 1);
        assert_eq!(field(1), 0o100755);
        assert_eq!(field(4), 1);
        assert_eq!(field(5), 1_700_000_000);
        assert_eq!(field(6), 5);
        assert_eq!(field(11), 16);
        // 110 + 16 = 126, padded to 128; then the data, padded to 136.
        assert_eq!(&out[110..125], b"./usr/bin/hello");
        assert_eq!(&out[128..133], b"hello");
        assert_eq!(&out[136..142], b"070701");
        assert_eq!(out.len() % 4, 0);
        assert!(out.windows(10).any(|w| w == b"TRAILER!!!"));
    }

    #[test]
    fn a_zip_archive_reads_back() {
        let mut tree = Tree::new();
        let big = b"BitBraid ".repeat(1000);
        tree.file("App/app.exe", big.clone(), 0o755).unwrap();
        tree.file("App/tiny.txt", b"x".to_vec(), 0o644).unwrap();
        let tree = tree.with_parents();
        let out = zip(&tree, 1_792_251_863).unwrap();
        // The end record names the central directory.
        let end = out.len() - 22;
        assert_eq!(&out[end..end + 4], &0x0605_4b50u32.to_le_bytes());
        let count = u16::from_le_bytes([out[end + 10], out[end + 11]]);
        assert_eq!(count, 3);
        let central_at = u32::from_le_bytes(out[end + 16..end + 20].try_into().unwrap()) as usize;
        let mut at = central_at;
        let mut names = Vec::new();
        for _ in 0..count {
            assert_eq!(&out[at..at + 4], &0x0201_4b50u32.to_le_bytes());
            let method = u16::from_le_bytes([out[at + 10], out[at + 11]]);
            let crc = u32::from_le_bytes(out[at + 16..at + 20].try_into().unwrap());
            let stored = u32::from_le_bytes(out[at + 20..at + 24].try_into().unwrap()) as usize;
            let size = u32::from_le_bytes(out[at + 24..at + 28].try_into().unwrap()) as usize;
            let name_len = u16::from_le_bytes([out[at + 28], out[at + 29]]) as usize;
            let local = u32::from_le_bytes(out[at + 42..at + 46].try_into().unwrap()) as usize;
            let name = std::str::from_utf8(&out[at + 46..at + 46 + name_len]).unwrap();
            names.push(name.to_owned());
            // The local header says the same, and the data is there.
            assert_eq!(&out[local..local + 4], &0x0403_4b50u32.to_le_bytes());
            let data_at = local + 30 + name_len;
            let data = &out[data_at..data_at + stored];
            let plain = if method == 8 {
                let mut plain = Vec::new();
                flate2::read::DeflateDecoder::new(data)
                    .read_to_end(&mut plain)
                    .unwrap();
                plain
            } else {
                data.to_vec()
            };
            assert_eq!(plain.len(), size);
            let mut check = Crc::new();
            check.update(&plain);
            assert_eq!(check.sum(), crc);
            if name == "App/app.exe" {
                assert_eq!(method, 8);
                assert_eq!(plain, big);
            }
            at += 46 + name_len;
        }
        assert_eq!(names, ["App/", "App/app.exe", "App/tiny.txt"]);
        assert_eq!(out, zip(&tree, 1_792_251_863).unwrap());
    }

    #[test]
    fn dos_time_keeps_to_its_range() {
        // 2026-10-17 15:44:23 UTC.
        let (time, date) = dos_time(1_792_251_863);
        assert_eq!(date >> 9, 2026 - 1980);
        assert_eq!((date >> 5) & 0xf, 10);
        assert_eq!(date & 0x1f, 17);
        assert_eq!(time >> 11, 15);
        assert_eq!((time >> 5) & 0x3f, 44);
        assert_eq!(time & 0x1f, 11);
        assert_eq!(dos_time(0), (0, 33));
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
    }

    #[test]
    fn gzip_has_no_time_and_reads_back() {
        let out = gzip(b"the same every time").unwrap();
        assert_eq!(&out[4..8], &[0, 0, 0, 0]);
        let mut back = String::new();
        flate2::read::GzDecoder::new(&out[..])
            .read_to_string(&mut back)
            .unwrap();
        assert_eq!(back, "the same every time");
        assert_eq!(out, gzip(b"the same every time").unwrap());
    }
}
