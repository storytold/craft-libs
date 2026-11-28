//! Vorbis packet durations and stream timing, from the Vorbis I specification: the block sizes of
//! the identification header (§4.2.2), the modes at the end of the setup header (§4.2.4), the
//! samples a packet returns (§4.3.8) and the Ogg granule positions (§A.2).

use super::{MAX_OPUS_GRANULE, Packet, Stream, VorbisInfo};

/// The most modes a setup header can declare (a 6-bit count plus one).
const MAX_MODES: usize = 64;

/// What a Vorbis stream's packets need for their durations: the two block sizes and the block flag
/// of each mode.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VorbisModes {
    /// The short and the long block size, in samples.
    pub blocksizes: [u32; 2],
    /// The block flag of each mode: `true` for the long block.
    pub long: Vec<bool>,
}

impl VorbisModes {
    /// Reads the block sizes from the identification header and the modes from the setup header;
    /// `None` when either is not a valid Vorbis I header.
    ///
    /// The setup header is walked from its start (codebooks, time-domain transforms, floors,
    /// residues and mappings) to reach the modes, without building any decoder tables, so the cost
    /// is one pass over the header's bits.
    pub fn parse(identification: &[u8], setup: &[u8]) -> Option<VorbisModes> {
        let info = VorbisInfo::parse(identification)?;
        let [b0, b1] = [info.blocksize_0, info.blocksize_1];
        // Vorbis I is version 0, with channels and a sample rate (§4.2.2)
        let version = u32::from_le_bytes([identification[7], identification[8], identification[9], identification[10]]);
        if version != 0
            || info.channels == 0
            || info.sample_rate == 0
            || !(64..=8192).contains(&b0)
            || !(64..=8192).contains(&b1)
            || b0 > b1
            || identification[29] & 1 == 0
        {
            return None;
        }
        let long = setup_modes(setup, info.channels)?;
        Some(VorbisModes { blocksizes: [b0, b1], long })
    }

    /// The block size of an audio packet from its first byte (§4.3.1: a packet-type bit of 0, then
    /// the mode number in `ilog(modes - 1)` bits); `None` for a packet that is not audio or names a
    /// mode the setup header does not have.
    pub fn blocksize(&self, first: u8) -> Option<u32> {
        self.mode(first).map(|(_, long)| self.blocksizes[usize::from(long)])
    }

    /// The block size of an audio packet of `size` bytes starting with `prefix`; `None` also when
    /// the packet ends inside its header, whose long blocks carry the previous and next window
    /// flags after the mode number (§4.3.1), since a decoder drops such a packet.
    fn packet_blocksize(&self, prefix: [u8; 2], size: u32) -> Option<u32> {
        let (bits, long) = self.mode(prefix[0])?;
        let header = 1 + bits + if long { 2 } else { 0 };
        (u64::from(size) * 8 >= u64::from(header)).then(|| self.blocksizes[usize::from(long)])
    }

    /// The width of the mode number and the block flag of the mode an audio packet's first byte
    /// names.
    fn mode(&self, first: u8) -> Option<(u32, bool)> {
        if first & 1 != 0 {
            return None;
        }
        let bits = ilog(self.long.len().saturating_sub(1) as u32);
        let mode = usize::from(first >> 1) & ((1usize << bits) - 1);
        self.long.get(mode).map(|&l| (bits, l))
    }
}

/// Vorbis I §A.2 timing of a Vorbis stream, in samples at the stream's sample rate.
#[non_exhaustive]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VorbisTiming {
    /// Granule position of the first sample each packet returns. A packet's samples run from its
    /// start for its duration; positions before 0 are samples to discard.
    pub starts: Vec<i64>,
    /// Samples each packet returns (§4.3.8): 0 for the first audio packet, which only primes the
    /// decoder, and for packets that are not audio; otherwise a quarter of the previous audio
    /// packet's block size plus a quarter of its own.
    pub durations: Vec<u32>,
    /// Samples to discard at the start: the first page's granule position states fewer samples
    /// than its packets return (a negative starting position, at most the first two audio
    /// packets' overlap).
    pub skip: u32,
    /// Granule position of the first sample to play: 0, or the positive start offset of a stream
    /// that does not begin at time zero.
    pub first: i64,
    /// Playable samples from `first`, with the final page's end trimming applied.
    pub total: i64,
    /// The first page's granule position is damaged: it lies above where the next page places
    /// that page's end, or above the final page's granule position when only two pages place
    /// the stream. The first page's packets are then placed to end where the next page starts,
    /// or from 0 when the next page is the last.
    pub first_damaged: bool,
}

impl VorbisTiming {
    /// Timing of the Vorbis stream `stream`, from its identification and setup headers and the
    /// granule positions of its pages; `None` when the stream has no valid Vorbis headers.
    ///
    /// A granule position outside `0..=MAX_OPUS_GRANULE` is treated like a missing one, as
    /// [`OpusTiming::of`](super::OpusTiming::of) does.
    pub fn of(stream: &Stream) -> Option<VorbisTiming> {
        let modes = VorbisModes::parse(stream.headers.first()?, stream.headers.get(2)?)?;
        Some(VorbisTiming::with_modes(stream, &modes))
    }

    /// Timing of `stream` with modes already read, for example from a stream whose headers came
    /// from elsewhere.
    pub fn with_modes(stream: &Stream, modes: &VorbisModes) -> VorbisTiming {
        // Granule positions come straight from the file, so the arithmetic saturates.
        let mut previous: Option<u32> = None;
        // samples the first two audio packets return, the most a negative start can discard
        let mut overlap = 0i64;
        let mut audio = 0u8;
        let durations: Vec<u32> = stream
            .packets
            .iter()
            .map(|p| {
                let Some(size) = modes.packet_blocksize(p.prefix, p.size) else { return 0 };
                let n = previous.map_or(0, |prev| prev / 4 + size / 4);
                previous = Some(size);
                if audio < 2 {
                    audio += 1;
                    overlap = overlap.saturating_add(i64::from(n));
                }
                n
            })
            .collect();
        VorbisTiming::place(stream, durations, overlap)
    }

    /// Timing of `stream` with the samples each packet returns given by `duration`, called once
    /// per packet in stream order; a decoder passes the durations it decodes to, including those it
    /// gives a packet it conceals, as [`OpusTiming::of_with_durations`](super::OpusTiming::of_with_durations)
    /// takes them. A negative start may discard at most the samples of the first two packets.
    pub fn of_with_durations(stream: &Stream, duration: impl FnMut(&Packet) -> u32) -> VorbisTiming {
        let durations: Vec<u32> = stream.packets.iter().map(duration).collect();
        let overlap = durations.iter().take(2).fold(0i64, |a, &d| a.saturating_add(i64::from(d)));
        VorbisTiming::place(stream, durations, overlap)
    }

    /// Places the packets of `stream`, which return `durations` samples, from the pages' granule
    /// positions (§A.2); a negative start discards at most `overlap` samples.
    fn place(stream: &Stream, durations: Vec<u32>, overlap: i64) -> VorbisTiming {
        // Granule positions come straight from the file, so the arithmetic saturates.
        let n = durations.len().min(stream.packets.len());
        let dur = |k: usize| i64::from(durations[k]);
        let granule_of = |k: usize| stream.packets.get(k).and_then(|p| p.granule).filter(|g| (0..=MAX_OPUS_GRANULE).contains(g));
        let mut ends: Vec<i64> = vec![0; n];
        let mut cursor: Option<i64> = None;
        let mut trimmed_end: Option<i64> = None;
        let mut start: Option<i64> = None;
        // pages with a granule position so far, the granule position of a page holding only the
        // priming packet, and the last packet of the first page that placed the stream
        let (mut pages, mut primed, mut placed_at) = (0usize, None::<i64>, None::<usize>);
        let mut first_damaged = false;
        let mut i = 0;
        while i < n {
            // packets completing on the same page: i..=j, the last carries the page's granule
            let mut j = i;
            while j < n && granule_of(j).is_none() {
                j += 1;
            }
            let sum = (i..=j.min(n - 1)).fold(0i64, |a, k| a.saturating_add(dur(k)));
            // A final page after the first whose granule position is below the first page's: the
            // first page's granule position is damaged, so that page is placed from 0.
            if let Some(g) = granule_of(j)
                && stream.packets[j].eos
                && pages == 1
                && cursor.or(primed).is_some_and(|c| g < c)
            {
                let mut e = 0i64;
                for (k, end) in ends.iter_mut().enumerate().take(i) {
                    e = e.saturating_add(dur(k));
                    *end = e;
                }
                cursor = Some(e);
                start = Some(0);
                first_damaged = true;
            }
            if granule_of(j).is_some() {
                pages += 1;
            }
            let end = match (granule_of(j), cursor) {
                // trailing packets without a granule (a truncated file): continue from the cursor
                (None, c) => c.unwrap_or(0).saturating_add(sum),
                // end trimming on the final page: keep the positions the packets would have
                (Some(g), Some(c)) if stream.packets[j].eos && g < c.saturating_add(sum) => {
                    trimmed_end = Some(g);
                    c.saturating_add(sum)
                }
                // a page that returns no samples (only the priming packet) does not place the stream
                (Some(g), None) if sum == 0 && !stream.packets[j].eos => {
                    ends[i..=j].fill(g);
                    primed = Some(g);
                    i = j + 1;
                    continue;
                }
                // the first page with audio: its granule places the stream, which may start before
                // or after 0 (§A.2)
                (Some(g), None) => {
                    placed_at = Some(j);
                    let s = g.saturating_sub(sum);
                    start = Some(s);
                    if stream.packets[j].eos && s < 0 {
                        // one page holds the whole stream: a granule below the samples it returns
                        // trims the end, not the start
                        trimmed_end = Some(g);
                        start = Some(0);
                        sum
                    } else {
                        g
                    }
                }
                // a discontinuity: trust the granule
                (Some(g), Some(_)) => g,
            };
            let mut e = end;
            for k in (i..=j.min(n - 1)).rev() {
                ends[k] = e;
                e = e.saturating_sub(dur(k));
            }
            cursor = Some(end);
            i = j + 1;
        }
        // A first page placed after where the next page starts (the granule positions come back
        // down after it) has a damaged granule position: it ends where the next page starts.
        // A page holding only the priming packet places nothing, so it is not checked.
        if let Some(j) = placed_at.filter(|&j| j + 1 < n) {
            let back = ends[j].saturating_sub(ends[j + 1].saturating_sub(dur(j + 1)));
            if back > 0 {
                for end in &mut ends[..=j] {
                    *end = end.saturating_sub(back);
                }
                start = start.map(|s| s.saturating_sub(back));
                first_damaged = true;
            }
        }
        let starts: Vec<i64> = ends.iter().zip(&durations).map(|(e, &d)| e.saturating_sub(i64::from(d))).collect();
        let origin = start.unwrap_or(0);
        // a negative start discards samples, never more than the first two audio packets overlap
        let skip = origin.saturating_neg().clamp(0, overlap);
        let first = origin.saturating_add(skip).max(0);
        let end = trimmed_end.or(cursor).unwrap_or(0);
        let total = end.saturating_sub(first).max(0);
        VorbisTiming { starts, durations, skip: u32::try_from(skip).unwrap_or(u32::MAX), first, total, first_damaged }
    }
}

/// Number of bits needed to hold `x` (§9.2.1).
fn ilog(x: u32) -> u32 {
    32 - x.leading_zeros()
}

/// Bits read least significant first (§2.1.4).
struct Bits<'a> {
    data: &'a [u8],
    pos: u64,
}

impl Bits<'_> {
    fn left(&self) -> u64 {
        (self.data.len() as u64 * 8).saturating_sub(self.pos)
    }
    fn read(&mut self, n: u32) -> Option<u32> {
        if u64::from(n) > self.left() {
            return None;
        }
        let mut v = 0u32;
        for k in 0..n {
            let at = self.pos + u64::from(k);
            let bit = (self.data[(at / 8) as usize] >> (at % 8)) & 1;
            v |= u32::from(bit) << k;
        }
        self.pos += u64::from(n);
        Some(v)
    }
    fn skip(&mut self, n: u64) -> Option<()> {
        (n <= self.left()).then(|| self.pos += n)
    }
    fn zero(&mut self, n: u32) -> Option<()> {
        (self.read(n)? == 0).then_some(())
    }
}

/// The largest `r` with `r^dims <= entries` (§9.2.3).
pub(super) fn lookup1_values(entries: u32, dims: u32) -> u64 {
    let fits = |r: u64| {
        let mut p = 1u64;
        for _ in 0..dims {
            p = p.saturating_mul(r);
            if p > u64::from(entries) {
                return false;
            }
        }
        true
    };
    let (mut lo, mut hi) = (0u64, u64::from(entries));
    while lo < hi {
        let mid = lo + (hi - lo).div_ceil(2);
        if fits(mid) { lo = mid } else { hi = mid - 1 }
    }
    lo
}

/// The block flag of each mode of a setup header (§4.2.4), passing over everything before them.
fn setup_modes(h: &[u8], channels: u8) -> Option<Vec<bool>> {
    if !h.starts_with(b"\x05vorbis") {
        return None;
    }
    let mut b = Bits { data: &h[7..], pos: 0 };
    let codebooks = b.read(8)? + 1;
    for _ in 0..codebooks {
        codebook(&mut b)?;
    }
    for _ in 0..b.read(6)? + 1 {
        b.zero(16)?;
    }
    let floors = b.read(6)? + 1;
    for _ in 0..floors {
        floor(&mut b, codebooks)?;
    }
    let residues = b.read(6)? + 1;
    for _ in 0..residues {
        residue(&mut b, codebooks)?;
    }
    let mappings = b.read(6)? + 1;
    for _ in 0..mappings {
        mapping(&mut b, channels, floors, residues)?;
    }
    let count = b.read(6)? as usize + 1;
    let mut long = Vec::with_capacity(count.min(MAX_MODES));
    for _ in 0..count {
        let flag = b.read(1)? == 1;
        b.zero(16)?;
        b.zero(16)?;
        if b.read(8)? >= mappings {
            return None;
        }
        long.push(flag);
    }
    (b.read(1)? == 1).then_some(long)
}

/// Passes over one codebook (§3.2.1).
fn codebook(b: &mut Bits<'_>) -> Option<()> {
    if b.read(24)? != 0x56_4342 {
        return None;
    }
    // a codebook of no dimensions is allowed and has no lookup values, as in the reference decoder
    let dims = b.read(16)?;
    let entries = b.read(24)?;
    if b.read(1)? == 1 {
        // ordered: runs of entries per length, each count in ilog(entries left) bits
        let mut length = b.read(5)? + 1;
        let mut entry = 0u32;
        while entry < entries {
            if length > 32 {
                return None;
            }
            entry = entry.checked_add(b.read(ilog(entries - entry))?).filter(|&e| e <= entries)?;
            length += 1;
        }
    } else {
        let sparse = b.read(1)? == 1;
        for _ in 0..entries {
            if !sparse || b.read(1)? == 1 {
                b.skip(5)?;
            }
        }
    }
    match b.read(4)? {
        0 => {}
        kind @ (1 | 2) => {
            // minimum value and delta, then the value width and the sequence flag
            b.skip(32 + 32)?;
            let width = u64::from(b.read(4)? + 1);
            b.skip(1)?;
            let values = match (kind, dims) {
                (_, 0) => 0,
                (1, _) => lookup1_values(entries, dims),
                _ => u64::from(entries) * u64::from(dims),
            };
            b.skip(values.checked_mul(width)?)?;
        }
        _ => return None,
    }
    Some(())
}

/// Passes over one floor configuration (§6.2.1, §7.2.2), whose books are among `codebooks`.
fn floor(b: &mut Bits<'_>, codebooks: u32) -> Option<()> {
    match b.read(16)? {
        0 => {
            // order, rate, bark map size, amplitude bits, amplitude offset, then the books
            b.skip(8 + 16 + 16 + 6 + 8)?;
            let books = b.read(4)? + 1;
            for _ in 0..books {
                if b.read(8)? >= codebooks {
                    return None;
                }
            }
            Some(())
        }
        1 => floor1(b, codebooks),
        _ => None,
    }
}

/// Passes over one residue configuration (§8.6.1), whose books are among `codebooks`.
fn residue(b: &mut Bits<'_>, codebooks: u32) -> Option<()> {
    if b.read(16)? > 2 {
        return None;
    }
    // begin, end, partition size, then the classifications and the classbook
    b.skip(24 + 24 + 24)?;
    let classifications = b.read(6)? + 1;
    if b.read(8)? >= codebooks {
        return None;
    }
    let mut books = 0u64;
    for _ in 0..classifications {
        let low = b.read(3)?;
        let high = if b.read(1)? == 1 { b.read(5)? } else { 0 };
        books += u64::from((high << 3 | low).count_ones());
    }
    for _ in 0..books {
        if b.read(8)? >= codebooks {
            return None;
        }
    }
    Some(())
}

/// Passes over one mapping configuration (§4.2.4, mappings), whose submaps name one of `floors`
/// floors and one of `residues` residues.
fn mapping(b: &mut Bits<'_>, channels: u8, floors: u32, residues: u32) -> Option<()> {
    b.zero(16)?;
    let submaps = if b.read(1)? == 1 { b.read(4)? + 1 } else { 1 };
    if b.read(1)? == 1 {
        let channel_bits = ilog(u32::from(channels) - 1);
        for _ in 0..b.read(8)? + 1 {
            let (magnitude, angle) = (b.read(channel_bits)?, b.read(channel_bits)?);
            if magnitude == angle || magnitude >= u32::from(channels) || angle >= u32::from(channels) {
                return None;
            }
        }
    }
    b.zero(2)?;
    if submaps > 1 {
        for _ in 0..channels {
            if b.read(4)? >= submaps {
                return None;
            }
        }
    }
    // per submap: an unused time configuration, the floor and the residue
    for _ in 0..submaps {
        b.skip(8)?;
        if b.read(8)? >= floors || b.read(8)? >= residues {
            return None;
        }
    }
    Some(())
}

/// Passes over a floor 1 configuration (§7.2.2), whose books are among `codebooks`. The X list,
/// with its two implicit end points, holds at most 65 distinct values.
fn floor1(b: &mut Bits<'_>, codebooks: u32) -> Option<()> {
    let partitions = b.read(5)?;
    let mut classes = Vec::with_capacity(partitions as usize);
    for _ in 0..partitions {
        classes.push(b.read(4)? as usize);
    }
    let count = classes.iter().max().map_or(0, |&m| m + 1);
    let mut dims = [0u32; 16];
    for d in dims.iter_mut().take(count) {
        *d = b.read(3)? + 1;
        let subclasses = b.read(2)?;
        if subclasses != 0 && b.read(8)? >= codebooks {
            return None;
        }
        // a subclass book is its number plus one, and 0 means none
        for _ in 0..1u32 << subclasses {
            if b.read(8)? > codebooks {
                return None;
            }
        }
    }
    b.skip(2)?;
    let range = b.read(4)?;
    let points: u64 = classes.iter().map(|&c| u64::from(dims[c])).sum();
    if points + 2 > 65 {
        return None;
    }
    let mut xs = vec![0u32, 1u32 << range];
    for _ in 0..points {
        xs.push(b.read(range)?);
    }
    xs.sort_unstable();
    xs.windows(2).all(|w| w[0] != w[1]).then_some(())
}
