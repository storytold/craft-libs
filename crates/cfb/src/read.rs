//! Reading compound files ([MS-CFB] §2.2–2.6).

use crate::{ENDOFCHAIN, Error, MAXREGSECT, MINI_SECTOR_SIZE, NOSTREAM, Result, compare_names, sniff};

/// Caps on what a file may ask the reader to allocate.
///
/// Everything else `open` allocates (FAT, mini FAT, mini stream) is bounded by the length of the
/// input itself: no chain can hold more sectors than the file has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Largest stream [`CompoundFile::read`] returns. Default 1 GiB.
    pub max_stream_size: u64,
    /// Most directory entries (128 bytes each in the file) `open` accepts. Default 1 048 576.
    pub max_dir_entries: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { max_stream_size: 1 << 30, max_dir_entries: 1 << 20 }
    }
}

/// Directory entry object type (§2.6.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Root,
    Storage,
    Stream,
    /// Unallocated (type 0) or an unknown type.
    Unknown,
}

/// One directory entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub kind: EntryKind,
    /// Class id of a storage.
    pub clsid: [u8; 16],
    pub state_bits: u32,
    /// FILETIME stamps (0 when not set).
    pub created: u64,
    pub modified: u64,
    /// First sector (regular or mini) of a stream; the mini stream's for the root.
    pub start: u32,
    pub size: u64,
    /// The storage this entry is a child of. `None` for the root and for entries the root's tree
    /// does not reach (unallocated entries, orphans).
    pub parent: Option<usize>,
    /// Child entries of a storage, in the in-order position of its sibling tree. For a well-formed
    /// file that is sorted by [`compare_names`]; the reader does not validate the tree's order, so
    /// a hostile file may give any order (lookups by name do not depend on it). Read-only after
    /// open: [`CompoundFile::child`] answers from an index built from this list and the entry names
    /// when the file is parsed, so changing either afterwards leaves the lookup stale.
    pub children: Vec<usize>,
    /// `children` sorted by [`compare_names`] (stable, so equal names keep their in-order
    /// position): [`CompoundFile::child`] binary-searches it instead of scanning every sibling,
    /// which made resolving every member of a wide storage quadratic.
    by_name: Vec<usize>,
    left: u32,
    right: u32,
    child: u32,
}

/// A parsed compound file over borrowed bytes.
#[derive(Clone, Debug)]
pub struct CompoundFile<'a> {
    data: &'a [u8],
    limits: Limits,
    /// Major version: 3 (512-byte sectors) or 4 (4096-byte sectors).
    pub version: u16,
    /// Header CLSID (normally zero).
    pub header_clsid: [u8; 16],
    sector_size: usize,
    mini_cutoff: u64,
    fat: Vec<u32>,
    minifat: Vec<u32>,
    mini_stream: Vec<u8>,
    /// Why the mini stream could not be read. The file still opens (streams in regular sectors
    /// stay readable); reading a stream held in the mini stream returns this error.
    mini_error: Option<Error>,
    /// The directory entries. Read-only after open (see [`Entry::children`]): the by-name index
    /// behind [`CompoundFile::child`] is built once when the file is parsed.
    pub entries: Vec<Entry>,
}

// Little-endian reads that cannot panic: a read past the end gives 0. Callers pass slices they
// have already sized (the 512-byte header, a whole sector, a 128-byte directory entry).
pub(crate) fn u16_at(b: &[u8], at: usize) -> u16 {
    b.get(at..).and_then(|s| s.first_chunk::<2>()).map_or(0, |c| u16::from_le_bytes(*c))
}
pub(crate) fn u32_at(b: &[u8], at: usize) -> u32 {
    b.get(at..).and_then(|s| s.first_chunk::<4>()).map_or(0, |c| u32::from_le_bytes(*c))
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    b.get(at..).and_then(|s| s.first_chunk::<8>()).map_or(0, |c| u64::from_le_bytes(*c))
}
fn clsid_at(b: &[u8], at: usize) -> [u8; 16] {
    b.get(at..).and_then(|s| s.first_chunk::<16>()).copied().unwrap_or([0; 16])
}

/// Byte range `[start, end)` of unit `index + skip` of `unit` bytes each, or `None` when the
/// arithmetic overflows `usize` (reachable on 32-bit targets such as wasm32).
pub(crate) fn span(index: usize, skip: usize, unit: usize) -> Option<(usize, usize)> {
    let start = index.checked_add(skip)?.checked_mul(unit)?;
    Some((start, start.checked_add(unit)?))
}

/// The first value that occurs twice in `ids`.
fn repeated(ids: &[u32]) -> Option<u32> {
    let mut sorted = ids.to_vec();
    sorted.sort_unstable();
    sorted.windows(2).find(|w| w.first() == w.get(1)).and_then(|w| w.first().copied())
}

fn invalid(s: impl Into<String>) -> Error {
    Error::Invalid(s.into())
}

impl<'a> CompoundFile<'a> {
    /// Parse the header, FAT, mini FAT, directory and mini stream of `data` with the default
    /// [`Limits`].
    pub fn open(data: &'a [u8]) -> Result<CompoundFile<'a>> {
        Self::open_with(data, Limits::default())
    }

    /// [`CompoundFile::open`] with caller-set limits.
    pub fn open_with(data: &'a [u8], limits: Limits) -> Result<CompoundFile<'a>> {
        if data.len() < 512 || !sniff(data) || u16_at(data, 28) != 0xFFFE {
            return Err(Error::NotCfb);
        }
        let version = u16_at(data, 26);
        let shift = u16_at(data, 30);
        // §2.2: 9 in version 3 and 12 in version 4, nothing else. The version field itself is not
        // checked: the shift is what decides where every sector lies.
        if shift != 9 && shift != 12 {
            return Err(invalid(format!("sector shift {shift}")));
        }
        let mini_shift = u16_at(data, 32);
        if 1usize.checked_shl(mini_shift as u32) != Some(MINI_SECTOR_SIZE) {
            return Err(invalid(format!("mini sector shift {mini_shift}")));
        }
        let sector_size = 1usize << shift;
        let n_fat = u32_at(data, 44) as usize;
        let first_dir = u32_at(data, 48);
        let first_minifat = u32_at(data, 60);
        let first_difat = u32_at(data, 68);
        let mut cf = CompoundFile {
            data,
            limits,
            version,
            header_clsid: clsid_at(data, 8),
            sector_size,
            mini_cutoff: u32_at(data, 56) as u64,
            fat: Vec::new(),
            minifat: Vec::new(),
            mini_stream: Vec::new(),
            mini_error: None,
            entries: Vec::new(),
        };
        cf.load_fat(n_fat, first_difat)?;
        // Directory.
        let dir = cf.read_chain(first_dir, None)?;
        let (raw_entries, _) = dir.as_chunks::<128>();
        if raw_entries.len() > limits.max_dir_entries {
            return Err(Error::LimitExceeded(format!("{} directory entries, the limit is {}", raw_entries.len(), limits.max_dir_entries)));
        }
        cf.entries = raw_entries.iter().map(|e| parse_entry(e, version)).collect();
        let (root_start, root_size) = match cf.entries.first() {
            None => return Err(invalid("empty directory")),
            Some(root) if root.kind != EntryKind::Root => return Err(invalid("first directory entry is not the root")),
            Some(root) => (root.start, root.size),
        };
        // Mini FAT and mini stream.
        if first_minifat <= MAXREGSECT {
            let mf = cf.read_chain(first_minifat, None)?;
            cf.minifat = mf.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
        }
        if root_start <= MAXREGSECT && root_size > 0 {
            // A mini stream that is damaged or shorter than the root entry says does not stop the
            // file from opening: only reads that need the missing part fail.
            let size = root_size.min(data.len() as u64) as usize;
            match cf.read_chain(root_start, Some(size)) {
                Ok(m) => cf.mini_stream = m,
                Err(e) => cf.mini_error = Some(e),
            }
        }
        cf.build_tree()?;
        Ok(cf)
    }

    /// An upper bound on the sectors any one chain can hold: the sectors of the file (counting the
    /// header and a cut last sector).
    fn max_sectors(&self) -> usize {
        self.data.len() / self.sector_size + 1
    }

    /// Read the FAT: its sector numbers are the first `n_fat` of the 109 header slots followed by
    /// the DIFAT chain.
    fn load_fat(&mut self, n_fat: usize, first_difat: u32) -> Result<()> {
        let (data, sector_size) = (self.data, self.sector_size);
        let max_sectors = self.max_sectors();
        if n_fat > max_sectors {
            return Err(invalid(format!("{n_fat} FAT sectors in a {}-byte file", data.len())));
        }
        let mut fat_sectors: Vec<u32> = (0..109).map(|i| u32_at(data, 76 + i * 4)).filter(|&s| s <= MAXREGSECT).collect();
        let mut d = first_difat;
        let mut seen = 0;
        while d <= MAXREGSECT && fat_sectors.len() < n_fat {
            seen += 1;
            // a DIFAT chain cannot have more sectors than the file holds; the header's DIFAT sector
            // count (up to 2^32 - 1) is not trusted, or a self-pointing sector loops that often
            if seen > max_sectors {
                return Err(invalid(format!("DIFAT chain loops: more than {max_sectors} sectors in a {}-byte file", data.len())));
            }
            let s = self.sector(d)?;
            let per = sector_size / 4 - 1;
            fat_sectors.extend((0..per).map(|i| u32_at(s, i * 4)).filter(|&v| v <= MAXREGSECT));
            d = u32_at(s, per * 4);
        }
        // Slots past the header's count are padding (free, or zero from some writers).
        fat_sectors.truncate(n_fat);
        // Every FAT sector is a sector of the file, so none is listed twice. A file that names one
        // sector over and over would otherwise build a FAT many times its own size.
        if let Some(dup) = repeated(&fat_sectors) {
            return Err(invalid(format!("FAT sector {dup} is listed twice")));
        }
        let mut fat = Vec::with_capacity(fat_sectors.len().saturating_mul(sector_size / 4));
        for s in fat_sectors {
            let b = self.sector(s)?;
            fat.extend(b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)));
        }
        self.fat = fat;
        Ok(())
    }

    /// Sector `n`, whole (FAT and DIFAT sectors: a truncated one is an error).
    fn sector(&self, n: u32) -> Result<&'a [u8]> {
        let s = self.sector_tail(n)?;
        if s.len() < self.sector_size {
            return Err(invalid(format!("sector {n} is outside the file")));
        }
        Ok(s)
    }

    /// Sector `n`, or the part of it the file holds when the file ends inside it (some writers do
    /// not pad the last sector). Never empty.
    fn sector_tail(&self, n: u32) -> Result<&'a [u8]> {
        let outside = || invalid(format!("sector {n} is outside the file"));
        if n > MAXREGSECT {
            return Err(outside());
        }
        // sector 0 follows the header sector
        let (start, end) = usize::try_from(n).ok().and_then(|i| span(i, 1, self.sector_size)).ok_or_else(|| invalid("sector number overflow"))?;
        self.data.get(start..end.min(self.data.len())).filter(|s| !s.is_empty()).ok_or_else(outside)
    }

    /// The sector numbers of the chain from `start`: all of it, or its first `want` sectors.
    fn chain_ids(&self, start: u32, want: Option<usize>) -> Result<Vec<u32>> {
        // A chain cannot visit more sectors than the file holds. Not `self.fat.len()`: the FAT can
        // be longer than the file has sectors.
        let max_steps = self.max_sectors();
        let want = want.unwrap_or(usize::MAX);
        let mut ids = Vec::new();
        let mut s = start;
        while s != ENDOFCHAIN && ids.len() < want {
            if s > MAXREGSECT {
                return Err(invalid(format!("bad sector {s:#x} in a chain")));
            }
            if ids.len() >= max_steps {
                return Err(invalid(format!("sector chain loops: more than {max_steps} sectors in a {}-byte file", self.data.len())));
            }
            ids.push(s);
            if ids.len() == want {
                break;
            }
            // a free entry (`FREESECT`) is above `MAXREGSECT` and is rejected at the top of the loop
            s = *self.fat.get(s as usize).ok_or_else(|| invalid(format!("sector {s} has no FAT entry")))?;
        }
        // A loop shorter than the stream would otherwise read back as repeated data.
        if let Some(dup) = repeated(&ids) {
            return Err(invalid(format!("sector chain loops: sector {dup} is visited twice")));
        }
        Ok(ids)
    }

    /// Concatenate a regular sector chain, up to `limit` bytes. The result is shorter than `limit`
    /// when the chain ends first or the file ends inside the chain's last sector; a sector cut
    /// short anywhere else in the chain is an error.
    fn read_chain(&self, start: u32, limit: Option<usize>) -> Result<Vec<u8>> {
        let ids = self.chain_ids(start, limit.map(|l| l.div_ceil(self.sector_size)))?;
        // `limit` is at most the file length (callers check) and the chain holds distinct sectors
        // of the file, so this capacity is bounded by the file's own size.
        let mut out = Vec::with_capacity(limit.unwrap_or(ids.len().saturating_mul(self.sector_size)).min(self.data.len()));
        for (i, &s) in ids.iter().enumerate() {
            let b = self.sector_tail(s)?;
            if b.len() < self.sector_size && i + 1 != ids.len() {
                return Err(invalid(format!("sector {s} is cut short inside a chain (the file may be truncated)")));
            }
            out.extend_from_slice(b);
        }
        if let Some(l) = limit {
            out.truncate(l);
        }
        Ok(out)
    }

    /// The `size` bytes of a stream held in the mini stream. As in [`Self::read_chain`], the mini
    /// stream may end inside the chain's last mini sector.
    fn read_mini_chain(&self, start: u32, size: usize) -> Result<Vec<u8>> {
        if let Some(e) = &self.mini_error {
            return Err(e.clone());
        }
        // `size` is at most the file length (the caller checks), which bounds the walk.
        let want = size.div_ceil(MINI_SECTOR_SIZE);
        let mut ids = Vec::new();
        let mut s = start;
        while ids.len() < want {
            if s > MAXREGSECT {
                return Err(invalid("mini stream chain ends early"));
            }
            ids.push(s);
            s = *self.minifat.get(s as usize).ok_or_else(|| invalid("mini sector has no mini FAT entry"))?;
        }
        if repeated(&ids).is_some() {
            return Err(invalid("mini sector chain loops"));
        }
        let mut out = Vec::with_capacity(size);
        for (i, &s) in ids.iter().enumerate() {
            let b = usize::try_from(s)
                .ok()
                .and_then(|i| span(i, 0, MINI_SECTOR_SIZE))
                .and_then(|(start, end)| self.mini_stream.get(start..end.min(self.mini_stream.len())))
                .filter(|b| !b.is_empty())
                .ok_or_else(|| invalid("mini sector outside the mini stream"))?;
            if b.len() < MINI_SECTOR_SIZE && i + 1 != ids.len() {
                return Err(invalid("mini sector is cut short inside a chain (the file may be truncated)"));
            }
            out.extend_from_slice(b);
        }
        if out.len() < size {
            return Err(invalid("stream is truncated"));
        }
        out.truncate(size);
        Ok(out)
    }

    /// Fill in `children`, `parent` and the by-name index of every storage the root reaches.
    /// Entries nothing reaches (orphans) are left alone, whatever they point at.
    fn build_tree(&mut self) -> Result<()> {
        let n = self.entries.len();
        let mut owner = vec![false; n];
        if let Some(root) = owner.first_mut() {
            *root = true;
        }
        // Storages whose children are still to be walked. Every entry is pushed at most once (it
        // is claimed in `owner` first), so this ends after at most `n` storages.
        let mut pending = vec![0usize];
        while let Some(parent) = pending.pop() {
            let mut kids = Vec::new();
            // in-order walk of the sibling tree, iterative, cycle-safe
            let mut stack: Vec<usize> = Vec::new();
            let mut cur = self.entries.get(parent).map_or(NOSTREAM, |e| e.child);
            loop {
                // A link to an entry that does not exist is treated as no link (the rest of the
                // tree stays readable); a link to an entry already in the tree is a cycle or a
                // shared subtree, which has no sensible reading.
                while let Some(e) = usize::try_from(cur).ok().and_then(|i| self.entries.get(i).map(|e| (i, e.left))) {
                    let (i, left) = e;
                    if owner.get(i).copied().unwrap_or(true) {
                        return Err(invalid(format!("directory entry {i} is referenced twice")));
                    }
                    if let Some(o) = owner.get_mut(i) {
                        *o = true;
                    }
                    stack.push(i);
                    cur = left;
                }
                let Some(top) = stack.pop() else { break };
                kids.push(top);
                cur = self.entries.get(top).map_or(NOSTREAM, |e| e.right);
            }
            for &k in &kids {
                if let Some(e) = self.entries.get_mut(k) {
                    e.parent = Some(parent);
                    if matches!(e.kind, EntryKind::Root | EntryKind::Storage) {
                        pending.push(k);
                    }
                }
            }
            let mut by_name = kids.clone();
            by_name.sort_by(|&a, &b| compare_names(self.name(a), self.name(b)));
            if let Some(p) = self.entries.get_mut(parent) {
                p.children = kids;
                p.by_name = by_name;
            }
        }
        Ok(())
    }

    fn name(&self, id: usize) -> &str {
        self.entries.get(id).map_or("", |e| e.name.as_str())
    }

    /// The root storage entry (id 0).
    pub fn root(&self) -> &Entry {
        // `open` refuses an empty directory, and `entries` is documented read-only after it.
        static EMPTY: Entry = Entry {
            name: String::new(),
            kind: EntryKind::Unknown,
            clsid: [0; 16],
            state_bits: 0,
            created: 0,
            modified: 0,
            start: ENDOFCHAIN,
            size: 0,
            parent: None,
            children: Vec::new(),
            by_name: Vec::new(),
            left: NOSTREAM,
            right: NOSTREAM,
            child: NOSTREAM,
        };
        self.entries.first().unwrap_or(&EMPTY)
    }

    pub fn entry(&self, id: usize) -> Option<&Entry> {
        self.entries.get(id)
    }

    /// Child of storage `parent` named `name` (case-insensitive as in §2.6.4). When a damaged
    /// file holds several children of the same name, the first in sibling-tree order wins.
    pub fn child(&self, parent: usize, name: &str) -> Option<usize> {
        let by_name = &self.entries.get(parent)?.by_name;
        let i = by_name.partition_point(|&c| compare_names(self.name(c), name).is_lt());
        by_name.get(i).copied().filter(|&c| compare_names(self.name(c), name).is_eq())
    }

    /// Entry at a `/`-separated path below the root (`""` is the root; empty components, so a
    /// leading `/` too, are ignored).
    pub fn find(&self, path: &str) -> Result<usize> {
        let mut cur = 0;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = self.child(cur, part).ok_or_else(|| Error::NotFound(path.to_string()))?;
        }
        Ok(cur)
    }

    /// Whether `path` names a stream.
    pub fn has_stream(&self, path: &str) -> bool {
        self.find(path).ok().and_then(|id| self.entries.get(id)).is_some_and(|e| e.kind == EntryKind::Stream)
    }

    /// Every storage and stream below the root, parents before their children and siblings in
    /// sibling-tree order. [`CompoundFile::path`] gives the path of each.
    pub fn walk(&self) -> Vec<usize> {
        let mut out = Vec::new();
        // ends: `build_tree` gave every entry at most one parent, so each is pushed at most once
        let mut stack: Vec<usize> = self.root().children.iter().rev().copied().collect();
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Some(e) = self.entries.get(id) {
                stack.extend(e.children.iter().rev());
            }
        }
        out
    }

    /// The `/`-separated path of entry `id` below the root (`""` for the root itself), or `None`
    /// for an id that does not exist or an entry the root does not reach. Names are joined as
    /// they are: a damaged file may hold a name with a `/` in it, which `find` will not resolve.
    pub fn path(&self, id: usize) -> Option<String> {
        let mut parts = Vec::new();
        let mut cur = id;
        // a parent chain is never longer than the directory
        for _ in 0..=self.entries.len() {
            if cur == 0 {
                parts.reverse();
                return Some(parts.join("/"));
            }
            let e = self.entries.get(cur)?;
            parts.push(e.name.as_str());
            cur = e.parent?;
        }
        None
    }

    /// Contents of stream `id`, refused above [`Limits::max_stream_size`].
    pub fn read(&self, id: usize) -> Result<Vec<u8>> {
        self.read_capped(id, u64::MAX)
    }

    /// Contents of stream `id`, refused when it is larger than `cap` bytes (or than
    /// [`Limits::max_stream_size`], whichever is smaller). Nothing is allocated for a refused
    /// stream.
    pub fn read_capped(&self, id: usize, cap: u64) -> Result<Vec<u8>> {
        let e = self.entries.get(id).ok_or_else(|| Error::NotFound(format!("entry {id}")))?;
        if e.kind != EntryKind::Stream {
            return Err(invalid(format!("{:?} is not a stream", e.name)));
        }
        let cap = cap.min(self.limits.max_stream_size);
        if e.size > cap {
            return Err(Error::LimitExceeded(format!("stream {:?} is {} bytes, the limit is {cap}", e.name, e.size)));
        }
        if e.size == 0 {
            return Ok(Vec::new());
        }
        let size = usize::try_from(e.size)
            .ok()
            .filter(|&s| s <= self.data.len())
            .ok_or_else(|| invalid(format!("stream {:?} is larger than the file", e.name)))?;
        if e.size < self.mini_cutoff {
            return self.read_mini_chain(e.start, size);
        }
        let out = self.read_chain(e.start, Some(size))?;
        if out.len() < size {
            return Err(invalid("stream is truncated"));
        }
        Ok(out)
    }

    /// Contents of the stream at `path`.
    pub fn read_path(&self, path: &str) -> Result<Vec<u8>> {
        self.read(self.find(path)?)
    }

    /// Contents of the stream at `path`, refused when larger than `cap` bytes.
    pub fn read_path_capped(&self, path: &str, cap: u64) -> Result<Vec<u8>> {
        self.read_capped(self.find(path)?, cap)
    }

    /// Sector size in bytes (512 or 4096).
    pub fn sector_size(&self) -> usize {
        self.sector_size
    }
}

/// The name of a 128-byte directory entry: the UTF-16 code units its length field covers, up to
/// the first NUL.
pub(crate) fn entry_name(b: &[u8; 128]) -> String {
    let name_len = (u16_at(b, 64) as usize).min(64);
    let units: Vec<u16> = (0..name_len / 2).map(|i| u16_at(b, i * 2)).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

fn parse_entry(b: &[u8; 128], version: u16) -> Entry {
    let kind = match b.get(66) {
        Some(5) => EntryKind::Root,
        Some(1) => EntryKind::Storage,
        Some(2) => EntryKind::Stream,
        _ => EntryKind::Unknown,
    };
    let size = u64_at(b, 120);
    Entry {
        name: entry_name(b),
        kind,
        clsid: clsid_at(b, 80),
        state_bits: u32_at(b, 96),
        created: u64_at(b, 100),
        modified: u64_at(b, 108),
        start: u32_at(b, 116),
        // only version 4 has 64-bit sizes; earlier files may leave garbage in the high half (§2.6.3)
        size: if version >= 4 { size } else { size & 0xFFFF_FFFF },
        parent: None,
        children: Vec::new(),
        by_name: Vec::new(),
        left: u32_at(b, 68),
        right: u32_at(b, 72),
        child: u32_at(b, 76),
    }
}
