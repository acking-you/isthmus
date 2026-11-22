//! Files on both ends of a transfer: what a send collects, and how a
//! receiver turns names from the network into safe places on disk.
//!
//! An offer is a flat list of entries with `/`-separated relative paths, a
//! folder first and its contents after it. The receiver trusts none of it:
//! every path is rebuilt component by component under its own staging
//! folder, refusing `..`, absolute paths and anything empty, and replacing
//! characters Windows cannot store, so a name that is fine on a Mac still
//! lands on a PC. Files arrive in a hidden staging folder beside their
//! destination and move into place, under a free name, only once all of
//! them are complete; a staging folder left by a dropped connection is what
//! a resumed transfer continues from.

use std::fs::Metadata;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::Error;

/// The most entries one send may hold.
pub const MAX_ENTRIES: usize = 100_000;

/// The longest relative path accepted, in bytes.
const MAX_PATH_BYTES: usize = 1024;

/// The longest single name accepted, in bytes (what most file systems take).
const MAX_NAME_BYTES: usize = 255;

/// The staging folder's prefix; the transfer id follows.
pub const STAGING_PREFIX: &str = ".isthmus-";

/// Names never sent: Finder's and Explorer's per-folder bookkeeping.
const SKIPPED_NAMES: &[&str] = &[".DS_Store", "Thumbs.db", "desktop.ini"];

/// One file or folder of an offer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    /// The relative path, `/`-separated.
    pub path: String,
    /// The file's length; 0 for a folder.
    #[serde(default)]
    pub size: u64,
    /// Whether this is a folder.
    #[serde(default)]
    pub dir: bool,
}

/// What a send collected: the entries and, for each, the file to read
/// (`None` for a folder).
#[derive(Debug, Default)]
pub struct Collected {
    pub entries: Vec<Entry>,
    pub sources: Vec<Option<PathBuf>>,
}

impl Collected {
    /// The bytes all files add up to.
    pub fn total(&self) -> u64 {
        self.entries.iter().map(|entry| entry.size).sum()
    }

    /// How many files (not folders) there are.
    pub fn files(&self) -> u32 {
        u32::try_from(self.entries.iter().filter(|entry| !entry.dir).count()).unwrap_or(u32::MAX)
    }

    fn push(&mut self, entry: Entry, source: Option<PathBuf>) -> Result<(), Error> {
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Error::TooManyFiles { limit: MAX_ENTRIES });
        }
        self.entries.push(entry);
        self.sources.push(source);
        Ok(())
    }
}

/// Collects `paths` (files and folders the user dropped) into entries.
/// Folders are walked in name order; links inside them are skipped so a
/// link cannot pull in a file outside what the user chose or loop forever,
/// while a link the user dropped directly is followed.
pub fn collect(paths: &[PathBuf]) -> Result<Collected, Error> {
    let mut collected = Collected::default();
    for path in paths {
        let metadata = std::fs::metadata(path).map_err(|source| Error::Source {
            path: path.clone(),
            source,
        })?;
        let Some(name) = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
        else {
            continue;
        };
        add(&mut collected, path, &name, &metadata)?;
    }
    if collected.entries.is_empty() {
        return Err(Error::NothingToSend);
    }
    Ok(collected)
}

fn add(
    collected: &mut Collected,
    path: &Path,
    relative: &str,
    metadata: &Metadata,
) -> Result<(), Error> {
    if metadata.is_dir() {
        collected.push(
            Entry {
                path: relative.to_owned(),
                size: 0,
                dir: true,
            },
            None,
        )?;
        let read = std::fs::read_dir(path).map_err(|source| Error::Source {
            path: path.to_path_buf(),
            source,
        })?;
        let mut children: Vec<_> = read.filter_map(Result::ok).collect();
        children.sort_by_key(std::fs::DirEntry::file_name);
        for child in children {
            let name = child.file_name().to_string_lossy().into_owned();
            if SKIPPED_NAMES.contains(&name.as_str()) {
                continue;
            }
            let Ok(kind) = child.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            let child_path = child.path();
            let Ok(child_metadata) = child.metadata() else {
                continue;
            };
            add(
                collected,
                &child_path,
                &format!("{relative}/{name}"),
                &child_metadata,
            )?;
        }
        Ok(())
    } else if metadata.is_file() {
        collected.push(
            Entry {
                path: relative.to_owned(),
                size: metadata.len(),
                dir: false,
            },
            Some(path.to_path_buf()),
        )
    } else {
        // Sockets, pipes and devices are not files to send.
        Ok(())
    }
}

/// `path` from an offer as a relative path that stays inside the folder it
/// is joined to, or `None` if it cannot be made safe.
pub fn safe_relative(path: &str) -> Option<PathBuf> {
    if path.is_empty() || path.len() > MAX_PATH_BYTES {
        return None;
    }
    let mut safe = PathBuf::new();
    for component in path.split('/') {
        safe.push(safe_name(component)?);
    }
    Some(safe)
}

/// One name from an offer made safe on every system Isthmus runs on, or
/// `None` for an empty name, `.`, `..` or one too long.
pub fn safe_name(name: &str) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." || name.len() > MAX_NAME_BYTES {
        return None;
    }
    let mut safe: String = name
        .chars()
        .map(|c| match c {
            '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    // Windows drops trailing dots and spaces, which would merge names.
    while safe.ends_with('.') || safe.ends_with(' ') {
        safe.pop();
    }
    if safe.is_empty() {
        return None;
    }
    let stem = safe
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes().get(3).is_some_and(u8::is_ascii_digit));
    if reserved {
        safe.insert(0, '_');
    }
    Some(safe)
}

/// The first component of an entry's path: what lands in the receive
/// folder.
pub fn top_level(path: &str) -> &str {
    path.split('/').next().unwrap_or(path)
}

/// A name in `dir` not yet taken: `name`, then `name (1)`, `name (2)`…
/// with the number before the extension of a file.
pub fn free_destination(dir: &Path, name: &str, is_dir: bool) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, extension) = match name.rfind('.') {
        Some(dot) if dot > 0 && !is_dir => name.split_at(dot),
        _ => (name, ""),
    };
    let mut number = 1u32;
    loop {
        let candidate = dir.join(format!("{stem} ({number}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
        number = number.saturating_add(1);
    }
}

/// The staging folder for transfer `id` in `receive_dir`.
pub fn staging_dir(receive_dir: &Path, id: &str) -> PathBuf {
    receive_dir.join(format!("{STAGING_PREFIX}{id}"))
}

/// A transfer id: 32 lowercase hex digits from the system's random source.
pub fn new_transfer_id() -> Result<String, getrandom::Error> {
    let mut bytes = [0; 16];
    getrandom::fill(&mut bytes)?;
    Ok(crate::hex::encode(&bytes))
}

/// Whether `id` looks like a transfer id, before it names a folder.
pub fn valid_transfer_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// What to call a transfer in a list: its one item's name, or the first
/// item's.
pub fn summary_name(entries: &[Entry]) -> String {
    entries
        .first()
        .map(|entry| top_level(&entry.path).to_owned())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::ScratchDir;

    /// Nothing from the network can climb out of the folder it is joined
    /// to: `..`, empty components and absolute paths are refused, and
    /// separators inside a name are replaced, not followed.
    #[test]
    fn offered_paths_cannot_escape() {
        assert!(safe_relative("../etc/passwd").is_none());
        assert!(safe_relative("a/../../b").is_none());
        assert!(safe_relative("/etc/passwd").is_none());
        assert!(safe_relative("a//b").is_none());
        assert!(safe_relative("").is_none());
        assert_eq!(
            safe_relative("a\\..\\b").unwrap(),
            PathBuf::from("a_.._b"),
            "a backslash is a character, never a separator"
        );
        assert_eq!(
            safe_relative("photos/2026/a.jpg").unwrap(),
            Path::new("photos/2026/a.jpg")
        );
    }

    /// Names a Windows receiver cannot store are made storable there.
    #[test]
    fn names_are_made_portable() {
        assert_eq!(safe_name("CON").unwrap(), "_CON");
        assert_eq!(safe_name("com1.txt").unwrap(), "_com1.txt");
        assert_eq!(safe_name("a:b?.txt").unwrap(), "a_b_.txt");
        assert_eq!(safe_name("notes. ").unwrap(), "notes");
        assert!(safe_name("...").is_none());
        assert_eq!(safe_name("console.log").unwrap(), "console.log");
    }

    /// A taken name gets a number before its extension; a folder's dots are
    /// part of its name.
    #[test]
    fn a_taken_name_gets_the_next_free_number() {
        let scratch = ScratchDir::new("free-name");
        let dir = scratch.path();
        assert_eq!(free_destination(dir, "a.txt", false), dir.join("a.txt"));
        std::fs::write(dir.join("a.txt"), b"").unwrap();
        std::fs::write(dir.join("a (1).txt"), b"").unwrap();
        assert_eq!(free_destination(dir, "a.txt", false), dir.join("a (2).txt"));
        std::fs::create_dir(dir.join("v1.2")).unwrap();
        assert_eq!(free_destination(dir, "v1.2", true), dir.join("v1.2 (1)"));
    }

    /// A dropped folder becomes the folder then its contents in name order,
    /// without Finder's bookkeeping files or links inside it.
    #[test]
    fn a_folder_is_collected_in_order_without_bookkeeping() {
        let scratch = ScratchDir::new("collect");
        let root = scratch.path().join("Trip");
        std::fs::create_dir_all(root.join("day 1")).unwrap();
        std::fs::write(root.join("b.jpg"), b"bb").unwrap();
        std::fs::write(root.join("day 1").join("a.jpg"), b"a").unwrap();
        std::fs::write(root.join(".DS_Store"), b"x").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("/etc/hosts", root.join("hosts")).unwrap();
        let collected = collect(&[root]).unwrap();
        let paths: Vec<&str> = collected.entries.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["Trip", "Trip/b.jpg", "Trip/day 1", "Trip/day 1/a.jpg"]
        );
        assert_eq!(collected.total(), 3);
        assert_eq!(collected.files(), 2);
        assert!(collected.sources[0].is_none());
    }

    /// A missing path is an error naming it; an empty selection has
    /// nothing to send.
    #[test]
    fn missing_or_empty_selections_are_errors() {
        let scratch = ScratchDir::new("collect-missing");
        let missing = scratch.path().join("gone.txt");
        assert!(matches!(collect(&[missing]), Err(Error::Source { .. })));
        assert!(matches!(collect(&[]), Err(Error::NothingToSend)));
    }
}
