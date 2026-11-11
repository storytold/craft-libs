//! Reading a package: every part up front ([`Package`]) or on demand ([`LazyPackage`]).

use std::collections::hash_map::Entry;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::{Cursor, Read};
use std::sync::Arc;

use craft_xml::{El, Options};

use crate::guard::check_entry_count;
use crate::name::{normalize, rels_path};
use crate::rels::{ContentTypes, Rels, targets_of};
use crate::{Error, Limits, Oversized};

/// Most bytes reserved for a part before any of it has been read, whatever size it declares.
const MAX_PREALLOC: u64 = 16 * 1024 * 1024;

type Zip<'a> = zip::ZipArchive<Cursor<&'a [u8]>>;

fn open_zip<'a>(bytes: &'a [u8], limits: &Limits) -> Result<Zip<'a>, Error> {
    // Before `zip` parses every entry the end record declares.
    check_entry_count(bytes, limits.max_entries as u64)?;
    let zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Zip(e.to_string()))?;
    if zip.len() > limits.max_entries {
        return Err(Error::TooManyEntries { declared: zip.len() as u64, limit: limits.max_entries as u64 });
    }
    Ok(zip)
}

/// A part name as stored: `\` as `/` (code that splits paths on `/` must not meet a `\`), no
/// leading `/`.
fn stored_name(name: &str) -> String {
    name.replace('\\', "/").trim_start_matches('/').to_string()
}

/// The inflated parts of a package, within [`Limits`]. Shared, so a part the caller keeps as it
/// is (a picture, an embedded object) is handed over rather than copied.
#[derive(Clone, Debug, Default)]
pub struct Package {
    files: BTreeMap<String, Arc<Vec<u8>>>,
    /// Lowercased part name → its stored spelling: part names are case-insensitive (ECMA-376
    /// Part 2 §6.2.2.3), so every spelling is one part.
    lower: HashMap<String, String>,
}

impl Package {
    /// Inflates every part.
    ///
    /// An entry the archive cannot produce (a failed checksum, an unknown method) is left out
    /// with a logged warning, and what it inflated is charged to [`Limits::max_work`]: an entry
    /// inflates in full before its checksum is compared, so failed entries would otherwise
    /// inflate without limit. A part over a limit refuses the package or is left out
    /// ([`Limits::oversized`]).
    pub fn open(bytes: &[u8], limits: &Limits) -> Result<Package, Error> {
        let mut zip = open_zip(bytes, limits)?;
        let skip = limits.oversized == Oversized::Skip;
        let mut files = BTreeMap::new();
        let (mut total, mut work) = (0u64, 0u64);
        for i in 0..zip.len() {
            if skip && work >= limits.max_work {
                log::warn!("opc: decompression budget spent; skipping the remaining {} entries", zip.len().saturating_sub(i));
                break;
            }
            let mut f = match zip.by_index(i) {
                Ok(f) => f,
                Err(e) => {
                    log::warn!("opc: skipping unreadable zip entry {i}: {e}");
                    continue;
                }
            };
            if f.is_dir() {
                continue;
            }
            let name = match f.name() {
                Ok(name) => stored_name(&name),
                Err(e) => {
                    log::warn!("opc: skipping zip entry {i} with an unreadable name: {e}");
                    continue;
                }
            };
            if f.size() > limits.max_part {
                if skip {
                    log::warn!("opc: skipping oversized part {name} ({} bytes)", f.size());
                    continue;
                }
                return Err(Error::PartTooLarge { part: name, limit: limits.max_part });
            }
            let left = limits.max_total.saturating_sub(total).min(limits.max_work.saturating_sub(work));
            let room = limits.max_part.min(left);
            let mut buf = Vec::with_capacity(f.size().min(MAX_PREALLOC) as usize);
            let read = (&mut f).take(room.saturating_add(1)).read_to_end(&mut buf);
            // Charged whether or not the part is kept.
            work = work.saturating_add(buf.len() as u64);
            if buf.len() as u64 > room {
                if skip {
                    log::warn!("opc: part {name} exceeds size limits; skipped");
                    continue;
                }
                return Err(if left < limits.max_part {
                    Error::PackageTooLarge { limit: limits.max_total.min(limits.max_work) }
                } else {
                    Error::PartTooLarge { part: name, limit: limits.max_part }
                });
            }
            if let Err(e) = read {
                log::warn!("opc: zip entry {name} unreadable: {e}");
                continue;
            }
            total = total.saturating_add(buf.len() as u64);
            files.insert(name, Arc::new(buf));
        }
        Ok(Package::new(files))
    }

    /// A package from parts already in memory (a Flat OPC file, a converted document), by part
    /// name.
    pub fn from_parts(files: BTreeMap<String, Vec<u8>>) -> Package {
        Package::new(files.into_iter().map(|(name, bytes)| (stored_name(&name), Arc::new(bytes))).collect())
    }

    fn new(mut files: BTreeMap<String, Arc<Vec<u8>>>) -> Package {
        let mut lower: HashMap<String, String> = HashMap::new();
        let mut collides = false;
        for name in files.keys() {
            match lower.entry(name.to_ascii_lowercase()) {
                Entry::Occupied(_) => collides = true,
                Entry::Vacant(v) => {
                    v.insert(name.clone());
                }
            }
        }
        if !collides {
            return Package { files, lower };
        }
        // Entries whose names differ only in case are one part, so every spelling of the name
        // reaches the same stored entry. The spelling kept is the one a relationship names, so
        // the part the document points at is the one read; the first in byte order only when no
        // relationship names any of them.
        let mut named = HashSet::new();
        for (name, bytes) in &files {
            targets_of(name, bytes, |t| {
                named.insert(t);
            });
        }
        for name in files.keys().rev().filter(|n| named.contains(n.as_str())) {
            lower.insert(name.to_ascii_lowercase(), name.clone());
        }
        let before = files.len();
        files.retain(|name, _| lower.get(&name.to_ascii_lowercase()) == Some(name));
        let dropped = before.saturating_sub(files.len());
        log::warn!("opc: {dropped} zip entries are other spellings of parts already read; they are left out");
        Package { files, lower }
    }

    /// The bytes of all its parts.
    pub fn size(&self) -> u64 {
        self.files.values().map(|f| f.len() as u64).sum()
    }

    /// Every part name as stored, in byte order.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }

    /// The stored spelling of part `name` (leading `/` ignored; case-insensitive).
    pub fn canonical(&self, name: &str) -> Option<&str> {
        self.lower.get(&normalize(name)).map(String::as_str)
    }

    /// Part bytes by name (leading `/` ignored; case-insensitive).
    pub fn get(&self, name: &str) -> Option<&[u8]> {
        self.get_shared(name).map(|v| v.as_slice())
    }

    /// [`Package::get`], as the shared bytes themselves: a part the caller keeps whole then
    /// costs no copy.
    pub fn get_shared(&self, name: &str) -> Option<&Arc<Vec<u8>>> {
        self.canonical(name).and_then(|n| self.files.get(n))
    }

    /// Part `name` as XML, read with `options` (which carry the caller's limits and its choice
    /// of names and strictness). `Ok(None)` when there is no such part.
    pub fn xml(&self, name: &str, options: &Options) -> Result<Option<El>, Error> {
        match self.get(name) {
            Some(b) => options.parse(b).map(Some).map_err(|error| Error::Xml { part: name.to_string(), error }),
            None => Ok(None),
        }
    }

    /// The relationships of `part` (from `dir/_rels/name.rels`); those of the package itself
    /// for `""`. Internal targets name the part as stored, so a part reached under several
    /// spellings is one part. A missing or unreadable relationships part gives none.
    pub fn rels(&self, part: &str) -> Rels {
        match self.get(&rels_path(part)) {
            Some(bytes) => Rels::parse_in(bytes, part, |n| self.canonical(n).map(str::to_string)),
            None => Rels::default(),
        }
    }

    /// `[Content_Types].xml`.
    pub fn content_types(&self) -> ContentTypes {
        self.get("[Content_Types].xml").map(ContentTypes::parse).unwrap_or_default()
    }
}

/// A package whose parts are inflated when they are asked for, against a running total: for a
/// reader that needs a few parts of a large file, or one part at a time.
pub struct LazyPackage<'a> {
    zip: Zip<'a>,
    /// Normalized part name → zip index (the first entry of a name).
    names: HashMap<String, usize>,
    total: u64,
    limits: Limits,
}

impl<'a> LazyPackage<'a> {
    /// Indexes the archive. Nothing is inflated; [`Limits::max_work`] and
    /// [`Limits::oversized`] are not used.
    pub fn open(bytes: &'a [u8], limits: &Limits) -> Result<LazyPackage<'a>, Error> {
        let zip = open_zip(bytes, limits)?;
        let mut names = HashMap::new();
        for i in 0..zip.len() {
            if let Some(Ok(n)) = zip.name_for_index(i) {
                names.entry(normalize(&n)).or_insert(i);
            }
        }
        Ok(LazyPackage { zip, names, total: 0, limits: *limits })
    }

    /// Every part's normalized name ([`normalize`]), sorted.
    pub fn names(&self) -> Vec<&str> {
        let mut v: Vec<&str> = self.names.keys().map(String::as_str).collect();
        v.sort_unstable();
        v
    }

    pub fn has(&self, part: &str) -> bool {
        self.names.contains_key(&normalize(part))
    }

    /// Bytes charged to [`Limits::max_total`] so far. It is the whole of it once a read has
    /// failed: see [`LazyPackage::read_limited`].
    pub fn spent(&self) -> u64 {
        self.total
    }

    /// A part's bytes, up to [`Limits::max_part`]; `Ok(None)` when it doesn't exist.
    pub fn read(&mut self, part: &str) -> Result<Option<Vec<u8>>, Error> {
        self.read_limited(part, self.limits.max_part)
    }

    /// A part the reader can do without (the caller warns and goes on when it fails). A read
    /// that fails part-way is charged this part's whole allowance (`maximum`, and the byte that
    /// detects an overrun) instead of ending the package's: the decompression work stays
    /// bounded, and the parts read after it still open.
    pub fn read_optional_limited(&mut self, part: &str, maximum: u64) -> Result<Option<Vec<u8>>, Error> {
        let before = self.total;
        let read = self.read_limited(part, maximum);
        if let Err(Error::Zip(_)) = &read {
            self.total = before.saturating_add(maximum.min(self.limits.max_part).saturating_add(1)).min(self.limits.max_total);
        }
        read
    }

    /// A part's bytes, up to `maximum` (and [`Limits::max_part`]): for a small metadata part
    /// that must not expand to the general limit.
    ///
    /// A read that fails or overruns spends the whole of [`Limits::max_total`]: it has already
    /// cost decompression work the total never saw, and a loop over many broken parts must not
    /// pay it again and again. Every later read is then a cheap error.
    pub fn read_limited(&mut self, part: &str, maximum: u64) -> Result<Option<Vec<u8>>, Error> {
        let maximum = maximum.min(self.limits.max_part);
        let Some(&idx) = self.names.get(&normalize(part)) else {
            return Ok(None);
        };
        let mut f = self.zip.by_index(idx).map_err(|e| Error::Zip(format!("{part}: {e}")))?;
        if f.size() > maximum {
            return Err(Error::PartTooLarge { part: part.to_string(), limit: maximum });
        }
        let remaining = self.limits.max_total.saturating_sub(self.total);
        let limit = maximum.min(remaining);
        let mut out = Vec::with_capacity(f.size().min(MAX_PREALLOC) as usize);
        let read = (&mut f).take(limit.saturating_add(1)).read_to_end(&mut out);
        if out.len() as u64 > limit {
            self.total = self.limits.max_total;
            return Err(if remaining < maximum {
                Error::PackageTooLarge { limit: self.limits.max_total }
            } else {
                Error::PartTooLarge { part: part.to_string(), limit: maximum }
            });
        }
        if let Err(e) = read {
            self.total = self.limits.max_total;
            return Err(Error::Zip(format!("{part}: {e}")));
        }
        self.total = self.total.saturating_add(out.len() as u64);
        Ok(Some(out))
    }

    /// Part `name` as XML, read with `options`. `Ok(None)` when there is no such part.
    pub fn read_xml(&mut self, part: &str, options: &Options) -> Result<Option<El>, Error> {
        match self.read(part)? {
            Some(b) => options.parse(&b).map(Some).map_err(|error| Error::Xml { part: part.to_string(), error }),
            None => Ok(None),
        }
    }

    /// The relationships of `part`. A missing or unparseable relationships part gives none; one
    /// that cannot be inflated is the error.
    pub fn rels(&mut self, part: &str) -> Result<Rels, Error> {
        let Some(bytes) = self.read(&rels_path(part))? else { return Ok(Rels::default()) };
        Ok(Rels::parse_in(&bytes, part, |n| self.has(n).then(|| n.to_string())))
    }
}

/// The first part whose declared size is over [`Limits::max_part`], with that size: for telling
/// the user which part made a package too large.
pub fn declared_oversized(bytes: &[u8], limits: &Limits) -> Option<(String, u64)> {
    let mut zip = open_zip(bytes, limits).ok()?;
    (0..zip.len()).find_map(|i| {
        let f = zip.by_index_raw(i).ok()?;
        (f.size() > limits.max_part).then(|| (f.name().map(|n| n.into_owned()).unwrap_or_default(), f.size()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip64_entry_count_sample;

    #[test]
    fn a_package_declaring_too_many_entries_is_refused_before_zip_reads_them() {
        let limits = Limits::new(20_000, 1 << 20, 1 << 20);
        let bomb = zip64_entry_count_sample(20_001);
        let refused = Some(Error::TooManyEntries { declared: 20_001, limit: 20_000 });
        assert_eq!(Package::open(&bomb, &limits).err(), refused);
        assert_eq!(LazyPackage::open(&bomb, &limits).err(), refused);
        assert!(declared_oversized(&bomb, &limits).is_none());
        // Within the limit, the same archive is the `zip` crate's to judge.
        assert!(!matches!(Package::open(&zip64_entry_count_sample(5), &limits), Err(Error::TooManyEntries { .. })));
    }
}
