//! Indexes of transport and program streams kept between runs.
//!
//! [`open`](super::open) reads the whole file to build its access-unit tables: for a recording of
//! many gigabytes that takes seconds each time, even when the file is in the OS cache. An app can
//! register an [`IndexStore`] with [`set_index_store`]; `open` then takes a file's index from the
//! store when it has one for the file, and saves it there after a scan.
//!
//! What is saved is the whole [`File`] the scan gave: its format, its streams with their unit
//! tables, the program, the clock range and the warnings. An open from the store gives the same
//! `File` as a scan.
//!
//! A file is known by its length and by hashes of its first and last 64 KiB. A saved index starts
//! with the version of its form and the craft-media version that wrote it, and ends with a
//! checksum of all of it: an index of another version, or one damaged in the store, is not used.
//! Saved tables are also checked before they are used: every count against the bytes that hold
//! it, every unit's packet against the file's length, stream ids against the format, the format
//! against the one the file's first bytes show, and a spread of every stream's units must start
//! at one of that stream's packets in the file (a transport stream packet of its PID, a program
//! stream PES packet of its stream id). Anything that does not fit is ignored, and the file is
//! scanned and saved again.

use std::sync::{Arc, PoisonError, RwLock};

use super::{ByteSource, Codec, File, Format, LpcmFormat, PictureInfo, Stream, StreamId, Unit};

/// Where an app keeps transport and program stream indexes between runs (a directory, a
/// database…).
pub trait IndexStore: Send + Sync {
    /// The index saved under `key`, when there is one.
    fn load(&self, key: &str) -> Option<Vec<u8>>;
    /// Keeps `index` under `key`, in place of what was there. A store that cannot keep it does
    /// nothing: the file is scanned again next time.
    fn save(&self, key: &str, index: &[u8]);
}

static STORE: RwLock<Option<Arc<dyn IndexStore>>> = RwLock::new(None);

/// Registers the store every [`open`](super::open) of this process uses (`None`: none, the
/// default).
pub fn set_index_store(store: Option<Arc<dyn IndexStore>>) {
    // The store it replaces is dropped after the lock is released: its `Drop` may register a
    // store too.
    let previous = std::mem::replace(&mut *STORE.write().unwrap_or_else(PoisonError::into_inner), store);
    drop(previous);
}

/// The registered store.
pub(crate) fn index_store() -> Option<Arc<dyn IndexStore>> {
    STORE.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// How much of the end of a file its key is made of (the start: what `open` read to sniff it).
const TAIL_BYTES: u64 = 64 * 1024;
/// The first bytes of a saved index: its form, version 1.
const MAGIC: &[u8; 8] = b"CMTSIX01";
/// The craft-media version that wrote a saved index: another version's scan may index a file
/// differently, so its indexes are not used.
const WRITER: &str = env!("CARGO_PKG_VERSION");
/// The seed of a saved index's checksum (FNV-1a of everything before it).
const CHECK_SEED: u64 = 0xcbf2_9ce4_8422_2325;
/// The fewest bytes a saved stream takes: its id (3), stream type, codec, language length and
/// LPCM flag (1 each), packet and skipped byte counts and unit count (8 each).
const STREAM_BYTES: usize = 3 + 4 + 3 * 8;
/// The fewest bytes a saved unit takes: flags, size, position and offset.
const UNIT_BYTES: usize = 1 + 4 + 8 + 4;
/// How many of a stream's units are checked against the file (spread over the table), besides
/// the last.
const UNIT_CHECKS: usize = 16;
/// The codecs that carry no value, by their tag in a saved index. Changing this table changes the
/// saved form: change [`MAGIC`]'s version with it.
const PLAIN_CODECS: [Codec; 13] = [
    Codec::Mpeg1Video,
    Codec::Mpeg2Video,
    Codec::H264,
    Codec::Hevc,
    Codec::MpegAudio,
    Codec::AacAdts,
    Codec::AacLatm,
    Codec::Ac3,
    Codec::Eac3,
    Codec::Dts,
    Codec::TrueHd,
    Codec::LpcmBluray,
    Codec::LpcmDvd,
];
/// The tag of a subtitle codec, followed by its index in [`SUBTITLES`].
const SUBTITLE_TAG: u8 = 0xFE;
/// The tag of an unknown codec, followed by its stream type.
const UNKNOWN_TAG: u8 = 0xFF;
/// The subtitle formats the scanners name, by their index in a saved index. A file with another
/// one is not saved.
const SUBTITLES: [&str; 5] = ["PGS subtitles", "Text subtitles", "DVB subtitles", "Teletext", "DVD sub-pictures"];

/// The key of the file `src` holds, `len` bytes long, whose first bytes are `head` (as many as
/// `open` reads to sniff the format): its length and hashes of its start and of its last
/// [`TAIL_BYTES`]. `None` when the end cannot be read.
pub(crate) fn key_of<S: ByteSource + ?Sized>(src: &S, len: u64, head: &[u8]) -> Option<String> {
    let tail_start = len.saturating_sub(TAIL_BYTES);
    let mut tail = vec![0u8; usize::try_from(len - tail_start).ok()?];
    src.read_at(tail_start, &mut tail).ok()?;
    let length = len.to_le_bytes();
    let first = fnv1a(0xcbf2_9ce4_8422_2325, &[head, &tail, &length]);
    let second = fnv1a(0x8422_2325_cbf2_9ce4 ^ len, &[&tail, &length, head]);
    Some(format!("mpegts1-{len:x}-{first:016x}{second:016x}"))
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

/// The saved form of `file`, the index of a file `len` bytes long. `None` when it holds something
/// the form cannot name (a subtitle format not in [`SUBTITLES`], a language that is empty or
/// longer than 255 bytes, a text longer than 4 GiB).
pub(crate) fn encode(file: &File, len: u64) -> Option<Vec<u8>> {
    let units: usize = file.streams.iter().map(|s| s.units.len()).sum();
    let mut out = Vec::with_capacity(64 + file.streams.len() * 64 + units.saturating_mul(UNIT_BYTES + 8));
    out.extend_from_slice(MAGIC);
    put_text(&mut out, WRITER)?;
    out.extend_from_slice(&len.to_le_bytes());
    match file.format {
        Format::Ts { packet_size, sync_offset } => {
            out.push(0);
            out.extend_from_slice(&u16::try_from(packet_size).ok()?.to_le_bytes());
            out.push(u8::try_from(sync_offset).ok()?);
        }
        Format::Ps { mpeg1 } => out.extend_from_slice(&[1, u8::from(mpeg1)]),
    }
    match file.program {
        Some(program) => {
            out.push(1);
            out.extend_from_slice(&program.to_le_bytes());
        }
        None => out.push(0),
    }
    match file.pcr_range {
        Some((first, last)) => {
            out.push(1);
            out.extend_from_slice(&first.to_le_bytes());
            out.extend_from_slice(&last.to_le_bytes());
        }
        None => out.push(0),
    }
    put_count(&mut out, file.warnings.len());
    for warning in &file.warnings {
        put_text(&mut out, warning)?;
    }
    put_count(&mut out, file.streams.len());
    for stream in &file.streams {
        put_stream(&mut out, stream)?;
    }
    seal(&mut out);
    Some(out)
}

fn put_stream(out: &mut Vec<u8>, s: &Stream) -> Option<()> {
    match s.id {
        StreamId::Pid(pid) => {
            out.push(0);
            out.extend_from_slice(&pid.to_le_bytes());
        }
        StreamId::Ps { stream_id, sub_id: None } => out.extend_from_slice(&[1, stream_id, 0]),
        StreamId::Ps { stream_id, sub_id: Some(sub) } => out.extend_from_slice(&[1, stream_id, 1, sub]),
    }
    out.push(s.stream_type);
    match &s.codec {
        Codec::Subtitle(name) => {
            let index = SUBTITLES.iter().position(|known| known == name)?;
            out.extend_from_slice(&[SUBTITLE_TAG, u8::try_from(index).ok()?]);
        }
        Codec::Unknown(stream_type) => out.extend_from_slice(&[UNKNOWN_TAG, *stream_type]),
        codec => out.push(u8::try_from(PLAIN_CODECS.iter().position(|plain| plain == codec)?).ok()?),
    }
    match &s.language {
        Some(language) => {
            out.push(u8::try_from(language.len()).ok().filter(|n| *n > 0)?);
            out.extend_from_slice(language.as_bytes());
        }
        None => out.push(0),
    }
    match s.lpcm {
        Some(f) => {
            out.push(1);
            for value in [f.sample_rate, f.channels, f.bits] {
                out.extend_from_slice(&value.to_le_bytes());
            }
        }
        None => out.push(0),
    }
    out.extend_from_slice(&s.pes_packets.to_le_bytes());
    out.extend_from_slice(&s.skipped_bytes.to_le_bytes());
    put_count(out, s.units.len());
    for u in &s.units {
        let flags = u8::from(u.pts.is_some())
            | u8::from(u.dts.is_some()) << 1
            | u8::from(u.key) << 2
            | u8::from(u.disposable) << 3
            | u8::from(u.picture.is_some()) << 4;
        out.push(flags);
        out.extend_from_slice(&u.size.to_le_bytes());
        out.extend_from_slice(&u.pos.to_le_bytes());
        out.extend_from_slice(&u.offset.to_le_bytes());
        for time in [u.pts, u.dts].into_iter().flatten() {
            out.extend_from_slice(&time.to_le_bytes());
        }
        if let Some(p) = u.picture {
            out.push(p.coding_type);
            out.extend_from_slice(&p.temporal_reference.to_le_bytes());
            out.push(p.structure);
            let bits = [p.top_field_first, p.repeat_first_field, p.progressive_frame, p.sequence_header, p.gop, p.closed_gop, p.broken_link];
            out.push(bits.iter().enumerate().fold(0u8, |acc, (i, bit)| acc | u8::from(*bit) << i));
        }
    }
    Some(())
}

/// Ends a saved index with the checksum of everything before it.
pub(crate) fn seal(out: &mut Vec<u8>) {
    let check = fnv1a(CHECK_SEED, &[out]);
    out.extend_from_slice(&check.to_le_bytes());
}

fn put_count(out: &mut Vec<u8>, count: usize) {
    out.extend_from_slice(&u64::try_from(count).unwrap_or(u64::MAX).to_le_bytes());
}

fn put_text(out: &mut Vec<u8>, text: &str) -> Option<()> {
    out.extend_from_slice(&u32::try_from(text.len()).ok()?.to_le_bytes());
    out.extend_from_slice(text.as_bytes());
    Some(())
}

/// Reads a saved index of a file `len` bytes long: `None` for anything that is not one, is of
/// another version, whose checksum does not match, or whose counts, positions and ids do not fit.
pub(crate) fn decode(bytes: &[u8], len: u64) -> Option<File> {
    let (bytes, check) = bytes.split_at_checked(bytes.len().checked_sub(8)?)?;
    if fnv1a(CHECK_SEED, &[bytes]).to_le_bytes() != check {
        return None;
    }
    let mut r = Reader { bytes, at: 0 };
    if r.take(MAGIC.len())? != MAGIC || r.text()? != WRITER || r.u64()? != len {
        return None;
    }
    let format = match r.byte()? {
        0 => {
            let packet_size = usize::from(r.u16()?);
            let sync_offset = usize::from(r.byte()?);
            // what `sniff` gives
            if !matches!((packet_size, sync_offset), (188, 0) | (192, 4) | (204, 0)) {
                return None;
            }
            Format::Ts { packet_size, sync_offset }
        }
        1 => Format::Ps { mpeg1: r.flag()? },
        _ => return None,
    };
    let program = if r.flag()? { Some(r.u16()?) } else { None };
    let pcr_range = if r.flag()? {
        let (first, last) = (r.i64()?, r.i64()?);
        (first <= last).then_some((first, last))?;
        Some((first, last))
    } else {
        None
    };
    let count = r.count(4)?;
    let mut warnings = Vec::with_capacity(count);
    for _ in 0..count {
        warnings.push(r.text()?.to_owned());
    }
    let count = r.count(STREAM_BYTES)?;
    let mut streams = Vec::with_capacity(count);
    for _ in 0..count {
        streams.push(stream(&mut r, format, len)?);
    }
    (r.at == bytes.len()).then_some(File { format, streams, program, pcr_range, warnings })
}

/// One saved stream of a file of `format`, `len` bytes long.
fn stream(r: &mut Reader<'_>, format: Format, len: u64) -> Option<Stream> {
    let id = match (r.byte()?, format) {
        (0, Format::Ts { .. }) => {
            let pid = r.u16()?;
            // a PID is 13 bits
            (pid <= 0x1FFF).then_some(StreamId::Pid(pid))?
        }
        (1, Format::Ps { .. }) => {
            let stream_id = r.byte()?;
            let sub_id = if r.flag()? { Some(r.byte()?) } else { None };
            StreamId::Ps { stream_id, sub_id }
        }
        _ => return None,
    };
    let stream_type = r.byte()?;
    let codec = match r.byte()? {
        SUBTITLE_TAG => Codec::Subtitle(SUBTITLES.get(usize::from(r.byte()?))?),
        UNKNOWN_TAG => Codec::Unknown(r.byte()?),
        tag => PLAIN_CODECS.get(usize::from(tag))?.clone(),
    };
    let language = match usize::from(r.byte()?) {
        0 => None,
        n => Some(std::str::from_utf8(r.take(n)?).ok()?.to_owned()),
    };
    let lpcm = if r.flag()? { Some(LpcmFormat { sample_rate: r.u32()?, channels: r.u32()?, bits: r.u32()? }) } else { None };
    let pes_packets = r.u64()?;
    let skipped_bytes = r.u64()?;
    // what a unit's packet holds at least: a whole transport stream packet, a PES packet header
    let room = match format {
        Format::Ts { packet_size, .. } => u64::try_from(packet_size).ok()?,
        Format::Ps { .. } => 6,
    };
    let count = r.count(UNIT_BYTES)?;
    let mut units = Vec::with_capacity(count);
    for _ in 0..count {
        let u = unit(r)?;
        if u.pos.checked_add(room).is_none_or(|end| end > len) {
            return None;
        }
        units.push(u);
    }
    Some(Stream { id, stream_type, codec, language, units, lpcm, pes_packets, skipped_bytes })
}

fn unit(r: &mut Reader<'_>) -> Option<Unit> {
    let flags = r.byte()?;
    if flags & !0x1F != 0 {
        return None;
    }
    let size = r.u32()?;
    let pos = r.u64()?;
    let offset = r.u32()?;
    let pts = if flags & 1 != 0 { Some(r.i64()?) } else { None };
    let dts = if flags & 2 != 0 { Some(r.i64()?) } else { None };
    let picture = if flags & 0x10 != 0 {
        let coding_type = r.byte()?;
        let temporal_reference = r.u16()?;
        let structure = r.byte()?;
        let bits = r.byte()?;
        if bits & 0x80 != 0 {
            return None;
        }
        let bit = |i: u8| bits & (1 << i) != 0;
        Some(PictureInfo {
            coding_type,
            temporal_reference,
            structure,
            top_field_first: bit(0),
            repeat_first_field: bit(1),
            progressive_frame: bit(2),
            sequence_header: bit(3),
            gop: bit(4),
            closed_gop: bit(5),
            broken_link: bit(6),
        })
    } else {
        None
    };
    Some(Unit { pts, dts, key: flags & 4 != 0, disposable: flags & 8 != 0, size, pos, offset, picture })
}

/// Whether a saved index fits the file `src` holds, whose first bytes show `format`: the same
/// format, and a spread of every stream's units start at one of the stream's packets (a transport
/// stream packet of its PID, a PES packet of its stream id).
pub(crate) fn fits<S: ByteSource + ?Sized>(file: &File, format: Format, src: &S) -> bool {
    file.format == format
        && file
            .streams
            .iter()
            .all(|s| units_to_check(s.units.len()).all(|i| s.units.get(i).is_some_and(|u| starts_a_packet(src, format, s.id, u.pos))))
}

/// Whether one of the packets of the stream `id` starts at `pos` in a file of `format`.
fn starts_a_packet<S: ByteSource + ?Sized>(src: &S, format: Format, id: StreamId, pos: u64) -> bool {
    match (format, id) {
        (Format::Ts { sync_offset, .. }, StreamId::Pid(pid)) => {
            let mut b = [0u8; 3];
            let read = u64::try_from(sync_offset).ok().and_then(|o| pos.checked_add(o)).is_some_and(|at| src.read_at(at, &mut b).is_ok());
            read && b[0] == 0x47 && u16::from_be_bytes([b[1] & 0x1F, b[2]]) == pid
        }
        (Format::Ps { .. }, StreamId::Ps { stream_id, .. }) => {
            let mut b = [0u8; 4];
            src.read_at(pos, &mut b).is_ok() && b == [0, 0, 1, stream_id]
        }
        _ => false,
    }
}

/// Which of `count` units are checked against the file: up to [`UNIT_CHECKS`] spread over the
/// table from the first, and the last.
fn units_to_check(count: usize) -> impl Iterator<Item = usize> {
    let step = count.div_ceil(UNIT_CHECKS).max(1);
    (0..count).step_by(step).chain(count.checked_sub(1))
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let end = self.at.checked_add(n)?;
        let part = self.bytes.get(self.at..end)?;
        self.at = end;
        Some(part)
    }
    fn byte(&mut self) -> Option<u8> {
        self.take(1)?.first().copied()
    }
    /// A byte that is 0 (false) or 1 (true).
    fn flag(&mut self) -> Option<bool> {
        match self.byte()? {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
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
    /// UTF-8 text after its byte length.
    fn text(&mut self) -> Option<&'a str> {
        let n = usize::try_from(self.u32()?).ok()?;
        std::str::from_utf8(self.take(n)?).ok()
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

    const LEN: u64 = 100_000;

    fn unit(pos: u64, pts: Option<i64>, dts: Option<i64>, picture: Option<PictureInfo>) -> Unit {
        Unit { pts, dts, key: pts.is_some(), disposable: dts.is_none(), size: 1000 + pos as u32, pos, offset: (pos % 184) as u32, picture }
    }

    fn stream(id: StreamId, codec: Codec, units: Vec<Unit>) -> Stream {
        Stream { id, stream_type: 0x42, codec, language: None, units, lpcm: None, pes_packets: 3, skipped_bytes: 0 }
    }

    /// Every codec the scanners give.
    fn codecs() -> Vec<Codec> {
        let mut all = vec![
            Codec::Mpeg1Video,
            Codec::Mpeg2Video,
            Codec::H264,
            Codec::Hevc,
            Codec::MpegAudio,
            Codec::AacAdts,
            Codec::AacLatm,
            Codec::Ac3,
            Codec::Eac3,
            Codec::Dts,
            Codec::TrueHd,
            Codec::LpcmBluray,
            Codec::LpcmDvd,
            Codec::Unknown(0),
            Codec::Unknown(0xFF),
        ];
        all.extend(["PGS subtitles", "Text subtitles", "DVB subtitles", "Teletext", "DVD sub-pictures"].map(Codec::Subtitle));
        all
    }

    /// A transport stream index with every kind of value, and a stream of every codec. Its last
    /// stream has one unit without timestamps or picture.
    fn ts_file() -> File {
        let picture = PictureInfo {
            coding_type: 3,
            temporal_reference: 1023,
            structure: 1,
            top_field_first: true,
            repeat_first_field: false,
            progressive_frame: true,
            sequence_header: false,
            gop: true,
            closed_gop: false,
            broken_link: true,
        };
        let mut video = stream(
            StreamId::Pid(0x1011),
            Codec::Mpeg2Video,
            vec![unit(0, Some(-5), Some(-10), Some(picture)), unit(192 * 3, Some(1 << 40), None, None)],
        );
        video.skipped_bytes = 7;
        video.units[1].size = u32::MAX;
        let mut lpcm = stream(StreamId::Pid(0x1100), Codec::LpcmBluray, vec![unit(192, None, Some(4), None)]);
        lpcm.language = Some("jpn".into());
        lpcm.lpcm = Some(LpcmFormat { sample_rate: 48_000, channels: 6, bits: 24 });
        let mut streams = vec![video, lpcm];
        streams.extend(codecs().into_iter().enumerate().map(|(i, codec)| stream(StreamId::Pid(0x1200 + i as u16), codec, Vec::new())));
        streams.push(stream(StreamId::Pid(0x1FFF), Codec::H264, vec![unit(LEN - 192, None, None, None)]));
        File {
            format: Format::Ts { packet_size: 192, sync_offset: 4 },
            streams,
            program: Some(1),
            pcr_range: Some((-3, 1 << 45)),
            warnings: vec!["3 continuity counter error(s)".into(), "über".into(), String::new()],
        }
    }

    /// A program stream index; its last stream has no units.
    fn ps_file() -> File {
        let mut lpcm = stream(StreamId::Ps { stream_id: 0xBD, sub_id: Some(0xA0) }, Codec::LpcmDvd, vec![unit(14, Some(0), None, None)]);
        lpcm.lpcm = Some(LpcmFormat { sample_rate: 96_000, channels: 2, bits: 20 });
        File {
            format: Format::Ps { mpeg1: true },
            streams: vec![lpcm, stream(StreamId::Ps { stream_id: 0xE0, sub_id: None }, Codec::Mpeg1Video, Vec::new())],
            program: None,
            pcr_range: None,
            warnings: Vec::new(),
        }
    }

    #[test]
    fn an_index_reads_back_as_saved() {
        for file in [ts_file(), ps_file()] {
            let bytes = encode(&file, LEN).expect("encodes");
            let back = decode(&bytes, LEN).expect("decodes");
            assert_eq!(format!("{back:?}"), format!("{file:?}"));
        }
    }

    #[test]
    fn an_index_that_does_not_fit_is_refused() {
        let bytes = encode(&ts_file(), LEN).expect("encodes");
        // Another length, every truncation, and trailing bytes (after the checksum, and before it
        // with a checksum that matches).
        assert!(decode(&bytes, LEN - 1).is_none());
        for cut in 0..bytes.len() {
            assert!(decode(&bytes[..cut], LEN).is_none(), "cut at {cut}");
        }
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer, LEN).is_none());
        let resealed = |body: &[u8]| {
            let mut b = body.to_vec();
            seal(&mut b);
            b
        };
        let body = &bytes[..bytes.len() - 8];
        assert!(decode(&resealed(&[body, &[0]].concat()), LEN).is_none());
        // Another version of the form, or of the writer (with a checksum that matches).
        let mut other = body.to_vec();
        other[MAGIC.len() - 1] ^= 1;
        assert!(decode(&resealed(&other), LEN).is_none());
        let mut other = body.to_vec();
        other[MAGIC.len() + 4] ^= 1;
        assert!(decode(&resealed(&other), LEN).is_none());
        // Values out of range, each saved whole (its checksum matches).
        let refused = |change: &dyn Fn(&mut File)| {
            let mut file = ts_file();
            change(&mut file);
            decode(&encode(&file, LEN).expect("encodes"), LEN).is_none()
        };
        assert!(refused(&|f| f.streams[0].units[1].pos = LEN - 191), "a packet past the end of the file");
        assert!(refused(&|f| f.streams[0].units[1].pos = u64::MAX), "a position that overflows");
        assert!(refused(&|f| f.streams[0].id = StreamId::Pid(0x2000)), "a PID wider than 13 bits");
        assert!(refused(&|f| f.streams[0].id = StreamId::Ps { stream_id: 0xE0, sub_id: None }), "a program stream id in a transport stream");
        assert!(refused(&|f| f.format = Format::Ts { packet_size: 188, sync_offset: 4 }), "a packet layout `sniff` never gives");
        assert!(refused(&|f| f.pcr_range = Some((5, 4))), "a clock range that runs backwards");
        let mut ps = ps_file();
        ps.streams[0].units[0].pos = LEN - 5;
        assert!(decode(&encode(&ps, LEN).expect("encodes"), LEN).is_none(), "a PES header past the end of the file");
        ps.streams[0].units[0].pos = 0;
        ps.streams[1].id = StreamId::Pid(0x100);
        assert!(decode(&encode(&ps, LEN).expect("encodes"), LEN).is_none(), "a PID in a program stream");
        // Unknown unit flags (the last stream's one unit is the last bytes).
        let mut flagged = body.to_vec();
        let at = flagged.len() - UNIT_BYTES;
        flagged[at] |= 0x80;
        assert!(decode(&resealed(&flagged), LEN).is_none());
        // A warning that is not UTF-8.
        let saved = [&5u32.to_le_bytes()[..], "über".as_bytes()].concat();
        let warning = body.windows(saved.len()).position(|w| w == saved).expect("the warning");
        let mut broken = body.to_vec();
        broken[warning + 4] = 0xFF;
        assert!(decode(&resealed(&broken), LEN).is_none());
        // A codec tag past the table (the program stream's last stream ends with its codec tag,
        // language, LPCM flag and three counts).
        let ps = encode(&ps_file(), LEN).expect("encodes");
        let mut tagged = ps[..ps.len() - 8].to_vec();
        let at = tagged.len() - 2 - 3 * 8 - 1;
        assert_eq!(tagged[at], 0, "MPEG-1 video's tag");
        tagged[at] = PLAIN_CODECS.len() as u8;
        assert!(decode(&resealed(&tagged), LEN).is_none());
        // A subtitle format the form cannot name is not saved.
        let mut unnamed = ps_file();
        unnamed.streams[1].codec = Codec::Subtitle("closed captions");
        assert!(encode(&unnamed, LEN).is_none());
        // A huge count with nothing behind it (with a checksum that matches) allocates nothing.
        let mut huge = body[..body.len() - UNIT_BYTES - 8].to_vec();
        huge.extend_from_slice(&u64::MAX.to_le_bytes());
        assert!(decode(&resealed(&huge), LEN).is_none());
    }

    /// Damage that keeps every value in range (a unit moved within the file, any byte changed) is
    /// caught by the checksum.
    #[test]
    fn a_damaged_index_is_refused() {
        let bytes = encode(&ts_file(), LEN).expect("encodes");
        let mut moved = ts_file();
        moved.streams[0].units[0].pos = 192 * 5;
        let other = encode(&moved, LEN).expect("encodes");
        assert!(decode(&other, LEN).is_some());
        let mut damaged = other[..other.len() - 8].to_vec();
        damaged.extend_from_slice(&bytes[bytes.len() - 8..]);
        assert!(decode(&damaged, LEN).is_none());
        for at in 0..bytes.len() {
            let mut changed = bytes.clone();
            changed[at] ^= 0x10;
            assert!(decode(&changed, LEN).is_none(), "byte {at} changed");
        }
    }

    #[test]
    fn the_key_follows_the_start_the_end_and_the_length() {
        let key = |file: &[u8]| key_of(file, file.len() as u64, &file[..file.len().min(64 * 1024)]).expect("a key");
        for size in [300_000usize, 1000] {
            let file: Vec<u8> = (0..size as u32).map(|i| (i % 251) as u8).collect();
            let mut changed = file.clone();
            changed[10] ^= 1;
            assert_ne!(key(&changed), key(&file));
            let mut changed = file.clone();
            changed[size - 10] ^= 1;
            assert_ne!(key(&changed), key(&file));
            let mut longer = file.clone();
            longer.push(0);
            assert_ne!(key(&longer), key(&file));
        }
    }
}
