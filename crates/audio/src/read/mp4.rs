//! Size checks on MP4 atoms before symphonia reads them: it allocates whatever an atom, a sample
//! table or a sample claims, and a failed allocation aborts the process.

use crate::pcm::{be_u32, be_u64};
use crate::{AudioError, Result};

/// The most table steps symphonia may take finding packet timestamps in an audio track, 2^34
/// (about 15 s of work). It scans the `stts` table from its first entry for every packet, so a
/// table of one-sample runs makes decoding quadratic in the packet count; [`stts_steps`] counts
/// the steps. The bound is on that work, not on entries: a long track whose packet durations vary
/// (a run change every ten AAC packets for three hours is 1.3e10 steps) stays under it, and a table
/// of many entries with few packets costs next to nothing. Ports soundcraft #1332.
const MAX_STTS_STEPS: u128 = 1 << 34;

#[derive(Clone, Copy)]
enum Level {
    Top,
    Moov,
    Trak,
    Mdia,
    Minf,
    /// `packets`: `Some` with the `stsz` sample count when the sample entry is an audio format, so
    /// symphonia will decode its packets (whatever the handler says: it builds codec parameters
    /// from `stsd` alone).
    Stbl {
        packets: Option<u64>,
    },
    Udta,
    Meta,
    Ilst,
    Tag,
}

/// Refuse the MP4 file whose top-level atoms start at `at` if an atom symphonia reads whole, a
/// sample table or a sample claims more bytes than the file holds.
pub(crate) fn check(b: &[u8], at: usize) -> Result<()> {
    atoms(b, at, b.len(), Level::Top).map(|_| ())
}

fn malformed(what: &str) -> AudioError {
    AudioError::Malformed(format!("MP4 {what} is larger than the file"))
}

/// Walk the atoms in `pos..end` that symphonia descends into, as it reads them. False where
/// symphonia stops reading: at a header it refuses, or after an atom that runs past `end`.
fn atoms(b: &[u8], mut pos: usize, end: usize, level: Level) -> Result<bool> {
    // A header that starts inside the container is read whole, even past its end.
    while pos < end {
        let (Some(size), Some(kind)) = (be_u32(b, pos), b.get(pos + 4..pos + 8)) else { return Ok(false) };
        let head = if size == 1 { 16 } else { 8 };
        // Size 0: no data, and the next atom starts at the container's end.
        let len = match size {
            0 => head,
            1 => be_u64(b, pos + 8).unwrap_or(0),
            n => u64::from(n),
        };
        if len < head {
            return Ok(false);
        }
        let body = pos + head as usize;
        let data_end = usize::try_from(len).ok().and_then(|n| pos.checked_add(n));
        let next = if size == 0 { Some(end) } else { data_end };
        let inner = data_end.map_or(b.len(), |n| n.min(b.len()));
        let more = match (level, kind) {
            (Level::Top, b"moov") => atoms(b, body, inner, Level::Moov)?,
            (Level::Top | Level::Udta, b"meta") => atoms(b, body + 4, inner, Level::Meta)?,
            (Level::Moov, b"trak") => atoms(b, body, inner, Level::Trak)?,
            (Level::Moov, b"udta") => atoms(b, body, inner, Level::Udta)?,
            (Level::Trak, b"mdia") => atoms(b, body, inner, Level::Mdia)?,
            (Level::Mdia, b"minf") => atoms(b, body, inner, Level::Minf)?,
            (Level::Minf, b"stbl") => atoms(b, body, inner, Level::Stbl { packets: audio_packets(b, body, inner) })?,
            (Level::Meta, b"ilst") => atoms(b, body, inner, Level::Ilst)?,
            (Level::Ilst, _) => atoms(b, body, inner, Level::Tag)?,
            (Level::Stbl { packets }, b"stts") => {
                let fits = table(b, body + 4, 8, inner)?;
                if packets.is_some_and(|packets| stts_steps(b, body + 4, inner, packets) > MAX_STTS_STEPS) {
                    return Err(AudioError::Malformed("MP4 audio timing table makes finding packet times too slow".into()));
                }
                fits
            }
            (Level::Stbl { .. }, b"co64") => table(b, body + 4, 8, inner)?,
            (Level::Stbl { .. }, b"stsc") => table(b, body + 4, 12, inner)?,
            (Level::Stbl { .. }, b"stco") => table(b, body + 4, 4, inner)?,
            (Level::Stbl { .. }, b"stsz") => sample_sizes(b, body, inner)?,
            (Level::Mdia, b"hdlr") | (Level::Tag, b"data" | b"name" | b"mean") if len > (b.len() - pos) as u64 => {
                return Err(malformed(&format!("`{}` atom", String::from_utf8_lossy(kind))));
            }
            _ => true,
        };
        match next {
            Some(n) if more && n <= end => pos = n,
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// The sample entry formats symphonia reads as audio (the list `stsd` is matched against in
/// symphonia-format-isomp4 0.6); every other format, video and subtitles included, has no audio packets.
const AUDIO_FORMATS: [&[u8; 4]; 18] = [
    b"mp4a", b"alac", b"ac-3", b"ec-3", b"fLaC", b"Opus", b".mp3", b"lpcm", b"wave", b"alaw", b"ulaw", b"raw ", b"sowt", b"twos", b"in24", b"in32",
    b"fl32", b"fl64",
];

/// The packet count (`stsz` sample count) of the `stbl` atom whose children lie in `pos..end` when
/// it describes audio, else `None`: its `stsd` and `stsz` may come before or after the timing
/// tables, so the children are walked once more. Symphonia keeps the last `stsd` and `stsz` it
/// reads, so the last of each decides.
fn audio_packets(b: &[u8], mut pos: usize, end: usize) -> Option<u64> {
    let (mut audio, mut packets) = (false, 0);
    while pos < end {
        let (Some(size), Some(kind)) = (be_u32(b, pos), b.get(pos + 4..pos + 8)) else { break };
        let head = if size == 1 { 16 } else { 8 };
        if kind == b"stsd" {
            // Version and flags, the entry count, then the first entry's size and format.
            audio = b.get(pos + head + 12..pos + head + 16).is_some_and(|f| AUDIO_FORMATS.iter().any(|a| a.as_slice() == f));
        } else if kind == b"stsz" {
            // Version and flags, the common sample size, then the count.
            packets = be_u32(b, pos + head + 8).map_or(0, u64::from);
        }
        let len = match size {
            0 => break,
            1 => be_u64(b, pos + 8).unwrap_or(0),
            n => u64::from(n),
        };
        match usize::try_from(len).ok().filter(|&n| n >= head).and_then(|n| pos.checked_add(n)) {
            Some(next) => pos = next,
            None => break,
        }
    }
    audio.then_some(packets)
}

/// The steps symphonia takes finding the timestamps of `packets` packets in the `stts` table whose
/// entry count is at `count`: packet `i` costs the number of entries up to and including its run,
/// and only the packets that exist (`packets`) are looked up. Entries past the atom's `end` are
/// never read.
fn stts_steps(b: &[u8], count: usize, end: usize, packets: u64) -> u128 {
    let entries = be_u32(b, count).map_or(0, |n| n as usize);
    let mut left = packets;
    let mut steps = 0u128;
    for j in 0..entries {
        let at = count + 4 + 8 * j;
        let Some(run) = (at + 8 <= end).then(|| be_u32(b, at)).flatten() else { break };
        let run = u64::from(run).min(left);
        steps += u128::from(run) * (j as u128 + 1);
        left -= run;
        if left == 0 {
            break;
        }
    }
    steps
}

/// A sample table's entries, counted at `count`, must fit in the file: symphonia reserves room for
/// all of them first. False if they run past the table's atom (`end`), where symphonia stops.
fn table(b: &[u8], count: usize, entry: u64, end: usize) -> Result<bool> {
    let Some(n) = be_u32(b, count) else { return Ok(false) };
    let (first, bytes) = (count + 4, u64::from(n) * entry);
    if bytes > b.len().saturating_sub(first) as u64 {
        return Err(malformed("sample table"));
    }
    Ok(first as u64 + bytes <= end as u64)
}

/// Every sample's size must fit in the file: symphonia reads each one into a buffer of that size.
/// Sizes past the atom are never used, as symphonia stops before reading a packet.
fn sample_sizes(b: &[u8], body: usize, end: usize) -> Result<bool> {
    let (Some(size), Some(count)) = (be_u32(b, body + 4), be_u32(b, body + 8)) else { return Ok(false) };
    let file = b.len() as u64;
    if u64::from(size) > file {
        return Err(malformed("sample"));
    }
    if size != 0 {
        return Ok(true);
    }
    let fits = table(b, body + 8, 4, end)?;
    let (sizes, _) = b.get(body + 12..end).unwrap_or_default().as_chunks::<4>();
    if sizes.iter().take(count as usize).any(|s| u64::from(u32::from_be_bytes(*s)) > file) {
        return Err(malformed("sample"));
    }
    Ok(fits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        [&(8 + body.len() as u32).to_be_bytes()[..], kind, body].concat()
    }

    /// A `moov` with one `.mp3` track of `packets` packets whose `stts` has `runs` runs of
    /// `per_run` packets each. Only the atoms the check walks are present.
    fn track(runs: u32, per_run: u32, packets: u32) -> Vec<u8> {
        let stsd = atom(b"stsd", &[&[0u8; 4][..], &1u32.to_be_bytes(), &atom(b".mp3", &[0; 28])].concat());
        let stts_body: Vec<u8> = [0, runs]
            .iter()
            .flat_map(|v: &u32| v.to_be_bytes())
            .chain((0..runs).flat_map(|_| [per_run, 1024].into_iter().flat_map(u32::to_be_bytes)))
            .collect();
        let stsz = atom(b"stsz", &[0u32, 1, packets].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>());
        let stbl = atom(b"stbl", &[stsd, atom(b"stts", &stts_body), stsz].concat());
        atom(b"moov", &atom(b"trak", &atom(b"mdia", &atom(b"minf", &stbl))))
    }

    #[test]
    fn the_bound_is_on_work_not_on_entries() {
        // many entries, few packets: only the packets that exist are looked up
        assert!(check(&track(100_000, 1, 100), 0).is_ok());
        // a three-hour track (500 000 AAC packets) whose duration changes every ten packets
        assert!(check(&track(50_000, 10, 500_000), 0).is_ok());
        // a run per packet over the same packets: quadratic work, refused
        assert!(matches!(check(&track(500_000, 1, 500_000), 0), Err(AudioError::Malformed(m)) if m.contains("timing table")));
        // the same table in a video track is not symphonia's to scan
        let video = track(500_000, 1, 500_000);
        let at = video.windows(4).position(|w| w == b".mp3").unwrap();
        let mut video = video;
        video[at..at + 4].copy_from_slice(b"avc1");
        assert!(check(&video, 0).is_ok());
    }
}
