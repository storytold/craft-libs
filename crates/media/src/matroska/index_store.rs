//! Sample tables of Matroska files kept between runs.
//!
//! Building the full index scans every Cluster, and the scan reads most of a file (element and
//! block headers are read through look-ahead windows): for a recording of many gigabytes that is
//! many seconds from disk each time the file is opened. An app can register an [`IndexStore`]
//! with [`set_index_store`]; [`super::open`] and [`super::open_with`] (a full index, without CRC
//! checks or an index limit) then take a file's tables from the store when it has them, and save
//! them there after a scan.
//!
//! What is saved is what the scan produced: the clusters, each track's sample table and keyframe
//! list, and the positions of the level-1 elements the walk read (Cues, Chapters, Tags… that may
//! lie after the clusters). An open from the store reads the file's headers and those elements as
//! usual and attaches the tables, so it gives the same [`super::MkvFile`] as a scan, apart from
//! the scan's warnings.
//!
//! A file is known by its length and by hashes of its first and last 64 KiB. Saved tables are
//! checked before they are used: every count against the bytes that hold it, every offset against
//! the file's length, every cluster and sync index against its table, and a spread of the saved
//! clusters must start with a Cluster ID in the file. Anything that does not fit is ignored and
//! the file is scanned.

use std::sync::{Arc, OnceLock, RwLock};

use super::meta::ClusterInfo;
use super::track::Sample;
use crate::source::ByteSource;

/// Where an app keeps Matroska indexes between runs (a directory, a database…).
pub trait IndexStore: Send + Sync {
    /// The index saved under `key`, when there is one.
    fn load(&self, key: &str) -> Option<Vec<u8>>;
    /// Keeps `index` under `key`. A store that cannot keep it does nothing: the file is scanned
    /// again next time.
    fn save(&self, key: &str, index: &[u8]);
}

fn slot() -> &'static RwLock<Option<Arc<dyn IndexStore>>> {
    static SLOT: OnceLock<RwLock<Option<Arc<dyn IndexStore>>>> = OnceLock::new();
    SLOT.get_or_init(|| RwLock::new(None))
}

/// Registers the store every full-index open of this process uses (`None`: none, the default).
pub fn set_index_store(store: Option<Arc<dyn IndexStore>>) {
    let mut slot = slot().write().unwrap_or_else(std::sync::PoisonError::into_inner);
    *slot = store;
}

/// The registered store.
pub(crate) fn index_store() -> Option<Arc<dyn IndexStore>> {
    slot().read().unwrap_or_else(std::sync::PoisonError::into_inner).clone()
}

/// How much of the start and of the end of a file its key is made of.
const KEY_WINDOW: u64 = 64 * 1024;
/// The first bytes of a saved index (format version 1).
const MAGIC: &[u8; 8] = b"CMKVIX01";
/// The encoded size of a sample without its `addition`.
const SAMPLE_BYTES: usize = 8 + 4 + 8 + 8 + 1 + 4 + 8 + 2;
/// How many saved clusters are checked against the file (spread over the table).
const CLUSTER_CHECKS: usize = 64;

/// The key of the file `src` holds: its length and hashes of its first and last [`KEY_WINDOW`]
/// bytes. `None` when those cannot be read.
pub(crate) fn key_of<S: ByteSource + ?Sized>(src: &S) -> Option<String> {
    let len = src.len();
    let head_len = usize::try_from(len.min(KEY_WINDOW)).ok()?;
    let mut head = vec![0u8; head_len];
    src.read_at(0, &mut head).ok()?;
    let tail_start = len.saturating_sub(KEY_WINDOW);
    let mut tail = vec![0u8; usize::try_from(len - tail_start).ok()?];
    src.read_at(tail_start, &mut tail).ok()?;
    let first = fnv1a(0xcbf2_9ce4_8422_2325, &[&head, &tail, &len.to_le_bytes()]);
    let second = fnv1a(0x8422_2325_cbf2_9ce4 ^ len, &[&tail, &len.to_le_bytes(), &head]);
    Some(format!("mkv1-{len:x}-{first:016x}{second:016x}"))
}

/// FNV-1a (64-bit) of `parts` one after another, from `seed`.
fn fnv1a(seed: u64, parts: &[&[u8]]) -> u64 {
    let mut hash = seed;
    for part in parts {
        for byte in *part {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// Tables read back from a saved index (checked against the file's length, not yet against the
/// file's tracks and clusters).
pub(crate) struct Saved {
    pub(crate) clusters: Vec<ClusterInfo>,
    /// Each track's number, sample table and keyframe list, in the file's track order.
    pub(crate) tracks: Vec<(u64, Vec<Sample>, Vec<u32>)>,
    /// Positions of the level-1 elements the scan read.
    pub(crate) elements: Vec<u64>,
}

/// The saved form of a scanned file's tables (`elements`: the level-1 elements the walk read) of
/// a file `len` bytes long.
pub(crate) fn encode(clusters: &[ClusterInfo], tracks: &[(u64, &[Sample], &[u32])], elements: &[u64], len: u64) -> Vec<u8> {
    let samples: usize = tracks.iter().map(|(_, samples, _)| samples.len()).sum();
    let mut out = Vec::with_capacity(32 + clusters.len() * 32 + samples * (SAMPLE_BYTES + 12));
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&len.to_le_bytes());
    put_count(&mut out, clusters.len());
    for c in clusters {
        for value in [c.offset, c.data_start, c.end, c.timestamp] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    put_count(&mut out, elements.len());
    for position in elements {
        out.extend_from_slice(&position.to_le_bytes());
    }
    put_count(&mut out, tracks.len());
    for (number, samples, sync) in tracks {
        out.extend_from_slice(&number.to_le_bytes());
        put_count(&mut out, samples.len());
        for s in *samples {
            out.extend_from_slice(&s.offset.to_le_bytes());
            out.extend_from_slice(&s.size.to_le_bytes());
            out.extend_from_slice(&s.pts.to_le_bytes());
            out.extend_from_slice(&s.duration.to_le_bytes());
            out.push(u8::from(s.keyframe) | (u8::from(s.addition.is_some()) << 1));
            out.extend_from_slice(&s.cluster.to_le_bytes());
            out.extend_from_slice(&s.block_offset.to_le_bytes());
            out.extend_from_slice(&s.lace.to_le_bytes());
            if let Some((offset, size)) = s.addition {
                out.extend_from_slice(&offset.to_le_bytes());
                out.extend_from_slice(&size.to_le_bytes());
            }
        }
        put_count(&mut out, sync.len());
        for index in *sync {
            out.extend_from_slice(&index.to_le_bytes());
        }
    }
    out
}

fn put_count(out: &mut Vec<u8>, count: usize) {
    out.extend_from_slice(&u64::try_from(count).unwrap_or(u64::MAX).to_le_bytes());
}

/// Reads a saved index of a file `len` bytes long: `None` for anything that is not one, or whose
/// counts, offsets and indices do not fit.
pub(crate) fn decode(bytes: &[u8], len: u64) -> Option<Saved> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(MAGIC.len())? != MAGIC || r.u64()? != len {
        return None;
    }
    let count = r.count(32)?;
    let mut clusters = Vec::with_capacity(count);
    for _ in 0..count {
        let c = ClusterInfo { offset: r.u64()?, data_start: r.u64()?, end: r.u64()?, timestamp: r.u64()? };
        if c.offset >= len || c.data_start > c.end || c.end > len || clusters.last().is_some_and(|p: &ClusterInfo| p.offset >= c.offset) {
            return None;
        }
        clusters.push(c);
    }
    let count = r.count(8)?;
    let mut elements = Vec::with_capacity(count);
    for _ in 0..count {
        let position = r.u64()?;
        if position >= len {
            return None;
        }
        elements.push(position);
    }
    let count = r.count(16)?;
    let mut tracks = Vec::with_capacity(count);
    for _ in 0..count {
        let number = r.u64()?;
        let samples = r.count(SAMPLE_BYTES)?;
        let mut table = Vec::with_capacity(samples);
        for _ in 0..samples {
            let s = sample(&mut r, len, clusters.len())?;
            table.push(s);
        }
        let syncs = r.count(4)?;
        let mut sync = Vec::with_capacity(syncs);
        for _ in 0..syncs {
            let index = r.u32()?;
            if usize::try_from(index).ok()? >= table.len() || sync.last().is_some_and(|p| *p >= index) {
                return None;
            }
            sync.push(index);
        }
        tracks.push((number, table, sync));
    }
    (r.at == bytes.len()).then_some(Saved { clusters, tracks, elements })
}

/// One saved sample, checked against a file `len` bytes long with `clusters` clusters.
fn sample(r: &mut Reader<'_>, len: u64, clusters: usize) -> Option<Sample> {
    let offset = r.u64()?;
    let size = r.u32()?;
    let pts = r.i64()?;
    let duration = r.u64()?;
    let flags = r.byte()?;
    let cluster = r.u32()?;
    let block_offset = r.u64()?;
    let lace = r.u16()?;
    let addition = if flags & 2 != 0 {
        let (at, n) = (r.u64()?, r.u32()?);
        (at.checked_add(u64::from(n))? <= len).then_some(Some((at, n)))?
    } else {
        None
    };
    let fits = offset.checked_add(u64::from(size)).is_some_and(|end| end <= len)
        && block_offset < len
        && usize::try_from(cluster).is_ok_and(|c| c < clusters)
        && flags & !3 == 0;
    fits.then_some(Sample { offset, size, pts, duration, keyframe: flags & 1 != 0, cluster, block_offset, lace, addition })
}

/// Which saved clusters an open from the store checks against the file: the first, the last and
/// up to [`CLUSTER_CHECKS`] spread between them.
pub(crate) fn clusters_to_check(count: usize) -> impl Iterator<Item = usize> {
    let step = (count / CLUSTER_CHECKS).max(1);
    (0..count).step_by(step).chain(count.checked_sub(1))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        let end = self.at.checked_add(n)?;
        let part = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(part)
    }
    fn byte(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }
    fn u16(&mut self) -> Option<u16> {
        Some(u16::from_le_bytes(self.take(2)?.try_into().ok()?))
    }
    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }
    fn u64(&mut self) -> Option<u64> {
        Some(u64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }
    /// A count of records of at least `each` bytes: no more than the bytes left can hold.
    fn count(&mut self, each: usize) -> Option<usize> {
        let count = usize::try_from(self.u64()?).ok()?;
        let left = self.bytes.len().saturating_sub(self.at);
        (count.checked_mul(each.max(1))? <= left).then_some(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tables() -> (Vec<ClusterInfo>, Vec<Sample>, Vec<u32>) {
        let clusters = vec![
            ClusterInfo { offset: 100, data_start: 112, end: 5000, timestamp: 0 },
            ClusterInfo { offset: 5000, data_start: 5012, end: 9000, timestamp: 1000 },
        ];
        let samples = vec![
            Sample { offset: 120, size: 400, pts: 0, duration: 33, keyframe: true, cluster: 0, block_offset: 116, lace: 0, addition: None },
            Sample {
                offset: 5020,
                size: 300,
                pts: 33,
                duration: 33,
                keyframe: false,
                cluster: 1,
                block_offset: 5016,
                lace: 1,
                addition: Some((5400, 20)),
            },
        ];
        (clusters, samples, vec![0])
    }

    #[test]
    fn an_index_reads_back_as_saved() {
        let (clusters, samples, sync) = tables();
        let bytes = encode(&clusters, &[(1, &samples, &sync)], &[40, 9000], 10_000);
        let saved = decode(&bytes, 10_000).expect("decodes");
        assert_eq!(saved.clusters, clusters);
        assert_eq!(saved.elements, vec![40, 9000]);
        assert_eq!(saved.tracks.len(), 1);
        assert_eq!(saved.tracks[0].0, 1);
        assert_eq!(saved.tracks[0].1, samples);
        assert_eq!(saved.tracks[0].2, sync);
    }

    #[test]
    fn an_index_that_does_not_fit_is_refused() {
        let (clusters, samples, sync) = tables();
        let bytes = encode(&clusters, &[(1, &samples, &sync)], &[40], 10_000);
        // Another length, every truncation, and trailing bytes.
        assert!(decode(&bytes, 9_999).is_none());
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut], 10_000).is_none(), "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer, 10_000).is_none());
        // A sample past the end of the file, a cluster index past the table, a sync past the samples.
        let mut far = samples.clone();
        far[1].offset = 9_900;
        assert!(decode(&encode(&clusters, &[(1, &far, &sync)], &[], 10_000), 10_000).is_none());
        let mut lost = samples.clone();
        lost[1].cluster = 2;
        assert!(decode(&encode(&clusters, &[(1, &lost, &sync)], &[], 10_000), 10_000).is_none());
        assert!(decode(&encode(&clusters, &[(1, &samples, &[5])], &[], 10_000), 10_000).is_none());
        // A huge count with nothing behind it allocates nothing.
        let mut huge = MAGIC.to_vec();
        huge.extend_from_slice(&10_000u64.to_le_bytes());
        huge.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&huge, 10_000).is_none());
    }

    #[test]
    fn the_key_follows_the_start_the_end_and_the_length() {
        let file: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let key = key_of(&file).expect("a key");
        let mut changed = file.clone();
        changed[10] ^= 1;
        assert_ne!(key_of(&changed).expect("a key"), key);
        let mut changed = file.clone();
        changed[299_990] ^= 1;
        assert_ne!(key_of(&changed).expect("a key"), key);
        let mut longer = file;
        longer.push(0);
        assert_ne!(key_of(&longer).expect("a key"), key);
    }
}
