//! Lyrics and markers an MP3 or FLAC file carries in its metadata, read without decoding audio.
//!
//! - **ID3v2** (versions 2.2, 2.3 and 2.4; MP3 files, and an ID3 tag in front of other files):
//!   `USLT` (`ULT` in 2.2) unsynchronised lyrics, `SYLT` (`SLT`) synchronised lyrics with
//!   millisecond timestamps, and `CHAP` chapters as markers, named by their `TIT2` sub-frame.
//!   Tag- and frame-level unsynchronisation, extended headers, grouping bytes and data length
//!   indicators are handled; compressed or encrypted frames are skipped. Text is read in all four
//!   ID3 encodings (ISO-8859-1, UTF-16 with a byte order mark, UTF-16BE, UTF-8).
//! - **FLAC** metadata blocks: `VORBIS_COMMENT` fields `LYRICS` and `UNSYNCEDLYRICS`, and
//!   `CUESHEET` track index points as markers at the `STREAMINFO` sample rate.
//!
//! Markers are [`Marker`]s as in [`crate::wav_markers`], so callers map and merge them the same
//! way. Rounding, ordering, labels for unnamed markers and splitting synchronised syllables into
//! lines belong to the caller.
//!
//! The file is untrusted input: every read is bounds-checked, sizes use checked arithmetic, and
//! the number of tags, frames, blocks, lines and markers and the size of each text are capped.
//! `CHAP` sub-frames are the only nesting and are read one level deep.

use std::borrow::Cow;

pub use crate::wav_markers::Marker;

/// Most markers read from one file.
pub const MAX_MARKERS: usize = crate::wav_markers::MAX_MARKERS;
/// Most synchronised lyric lines read from one file.
pub const MAX_SYNCED_LINES: usize = 100_000;
/// Largest text kept from one frame or field (bytes, before decoding).
pub const MAX_TEXT_BYTES: usize = 1 << 20;
/// Most text kept from one file across all lyrics and lines (bytes, before decoding).
pub const MAX_TEXT_TOTAL_BYTES: usize = 8 << 20;
/// Most ID3v2 tags read one after another at the start of a file.
const MAX_ID3_TAGS: usize = 16;
/// Most frames looked at in one ID3v2 tag (sub-frames of a `CHAP` count separately).
const MAX_FRAMES: usize = 100_000;
/// Most FLAC metadata blocks looked at.
const MAX_FLAC_BLOCKS: usize = 4_096;
/// Most Vorbis comment fields looked at.
const MAX_COMMENTS: usize = 100_000;

/// Unsynchronised lyrics: the whole text at once.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lyrics {
    /// ISO 639-2 language code as written (`eng`, `deu`, `XXX`); empty when the format has none.
    pub language: String,
    /// The content descriptor ID3 lets several lyrics frames tell apart; often empty.
    pub description: String,
    /// The lyrics, line breaks as written.
    pub text: String,
}

/// One timed piece of synchronised lyrics: a line or a syllable, as the file has it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyncedLine {
    /// Start in milliseconds from the start of the audio.
    pub ms: u32,
    /// The text; writers often begin a new line with `\n` and leave syllables without one.
    pub text: String,
}

/// Synchronised lyrics: one `SYLT` frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SyncedLyrics {
    /// ISO 639-2 language code as written.
    pub language: String,
    /// The content descriptor.
    pub description: String,
    /// Lines in file order.
    pub lines: Vec<SyncedLine>,
}

/// What [`read`] found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tags {
    /// Unsynchronised lyrics in file order.
    pub lyrics: Vec<Lyrics>,
    /// Synchronised lyrics with millisecond timestamps in file order (frames timed in MPEG
    /// frames are skipped).
    pub synced: Vec<SyncedLyrics>,
    /// ID3 chapters (rate 1000, so start and duration are milliseconds) and FLAC cue sheet index
    /// points (rate: the sample rate, 0 when `STREAMINFO` is missing; named by track number).
    pub markers: Vec<Marker>,
    /// A cap above left something out.
    pub limited: bool,
}

/// Reads the lyrics and markers in `bytes`: ID3v2 tags at the start, then FLAC metadata blocks if
/// a FLAC stream follows. Anything else yields an empty [`Tags`].
pub fn read(bytes: &[u8]) -> Tags {
    let mut r = Reader { out: Tags::default(), budget: MAX_TEXT_TOTAL_BYTES, lines: 0 };
    let mut at = 0usize;
    let mut tags = 0usize;
    while let Some(rest) = bytes.get(at..)
        && rest.starts_with(b"ID3")
    {
        if tags >= MAX_ID3_TAGS {
            r.out.limited = true;
            break;
        }
        tags += 1;
        match id3(rest, &mut r) {
            Some(len) if len > 0 => at = at.saturating_add(len),
            _ => break,
        }
    }
    if let Some(rest) = bytes.get(at..)
        && rest.starts_with(b"fLaC")
    {
        flac(rest, &mut r);
    }
    r.out
}

/// What is found so far, and what the caps still allow.
struct Reader {
    out: Tags,
    /// Text bytes still allowed in the whole file.
    budget: usize,
    /// Synchronised lines kept so far.
    lines: usize,
}

impl Reader {
    /// Takes `len` bytes of text from the budget; false (and `limited`) when it is spent or the
    /// text is too long.
    fn spend(&mut self, len: usize) -> bool {
        if len > MAX_TEXT_BYTES || len > self.budget {
            self.out.limited = true;
            return false;
        }
        self.budget -= len;
        true
    }
}

// ---------------------------------------------------------------------------------------------
// ID3v2

fn syncsafe(b: &[u8]) -> Option<usize> {
    let s = b.get(..4)?;
    if s.iter().any(|&c| c & 0x80 != 0) {
        return None;
    }
    Some(s.iter().fold(0usize, |n, &c| (n << 7) | usize::from(c)))
}

fn be(b: &[u8], n: usize) -> Option<usize> {
    Some(b.get(..n)?.iter().fold(0usize, |v, &c| (v << 8) | usize::from(c)))
}

fn be_u32(b: &[u8]) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(..4)?.try_into().ok()?))
}

/// Undoes ID3 unsynchronisation: `FF 00` becomes `FF`.
fn unsync(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut prev_ff = false;
    for &c in b {
        if prev_ff && c == 0 {
            prev_ff = false;
            continue;
        }
        out.push(c);
        prev_ff = c == 0xFF;
    }
    out
}

/// Reads one ID3v2 tag at the start of `b`; the number of bytes it takes (header, body and any
/// footer), or `None` when `b` does not start with one.
fn id3(b: &[u8], r: &mut Reader) -> Option<usize> {
    if b.get(..3)? != b"ID3" {
        return None;
    }
    let major = *b.get(3)?;
    let flags = *b.get(5)?;
    let size = syncsafe(b.get(6..)?)?;
    let footer = if major == 4 && flags & 0x10 != 0 { 10 } else { 0 };
    let total = size.checked_add(10)?.checked_add(footer)?;
    if !(2..=4).contains(&major) {
        return Some(total);
    }
    // A truncated file: read what is there.
    let end = size.saturating_add(10).min(b.len());
    let raw = b.get(10..end)?;
    let body: Cow<'_, [u8]> = if major <= 3 && flags & 0x80 != 0 { Cow::Owned(unsync(raw)) } else { Cow::Borrowed(raw) };
    let mut frames_at = 0usize;
    if flags & 0x40 != 0 && major >= 3 {
        frames_at = match major {
            3 => be(&body, 4)?.checked_add(4)?,
            _ => syncsafe(&body)?,
        };
    }
    let all_unsync = major == 4 && flags & 0x80 != 0;
    let frames = body.get(frames_at..).unwrap_or_default();
    let mut frames = Frames::new(frames, major, all_unsync);
    for (id, data) in frames.by_ref() {
        frame(&id, &data, major, r);
    }
    if frames.capped {
        r.out.limited = true;
    }
    Some(total)
}

/// The frames of an ID3v2 tag body (or of a `CHAP` frame's sub-frames): id and data, with
/// frame-level flags already applied; compressed and encrypted frames are left out.
struct Frames<'a> {
    b: &'a [u8],
    at: usize,
    major: u8,
    all_unsync: bool,
    seen: usize,
    /// More frames followed when [`MAX_FRAMES`] stopped the reading.
    capped: bool,
}

impl<'a> Frames<'a> {
    fn new(b: &'a [u8], major: u8, all_unsync: bool) -> Self {
        Self { b, at: 0, major, all_unsync, seen: 0, capped: false }
    }
}

impl<'a> Iterator for Frames<'a> {
    type Item = ([u8; 4], Cow<'a, [u8]>);

    fn next(&mut self) -> Option<Self::Item> {
        let (id_len, header) = if self.major == 2 { (3, 6) } else { (4, 10) };
        loop {
            if self.seen >= MAX_FRAMES {
                // Padding or the end is not a cut; another frame id is.
                self.capped = self.b.get(self.at).is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
                return None;
            }
            self.seen += 1;
            let h = self.b.get(self.at..self.at.checked_add(header)?)?;
            let id_bytes = h.get(..id_len)?;
            // Padding, or garbage: no more frames.
            if !id_bytes.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()) {
                return None;
            }
            let size = match self.major {
                2 => be(h.get(3..)?, 3)?,
                3 => be(h.get(4..)?, 4)?,
                _ => syncsafe(h.get(4..)?)?,
            };
            let start = self.at.checked_add(header)?;
            let end = start.checked_add(size)?;
            let data = self.b.get(start..end)?;
            self.at = end;
            let mut id = [0u8; 4];
            for (d, s) in id.iter_mut().zip(id_bytes) {
                *d = *s;
            }
            let format = if self.major == 2 { 0 } else { *h.get(9)? };
            let mut data: Cow<'a, [u8]> = Cow::Borrowed(data);
            match self.major {
                3 => {
                    if format & 0xC0 != 0 {
                        continue;
                    }
                    if format & 0x20 != 0 {
                        data = skip(data, 1);
                    }
                }
                4 => {
                    if format & 0x0C != 0 {
                        continue;
                    }
                    if format & 0x40 != 0 {
                        data = skip(data, 1);
                    }
                    if format & 0x01 != 0 {
                        data = skip(data, 4);
                    }
                    if format & 0x02 != 0 || self.all_unsync {
                        data = Cow::Owned(unsync(&data));
                    }
                }
                _ => {}
            }
            return Some((id, data));
        }
    }
}

fn skip(b: Cow<'_, [u8]>, n: usize) -> Cow<'_, [u8]> {
    match b {
        Cow::Borrowed(s) => Cow::Borrowed(s.get(n..).unwrap_or_default()),
        Cow::Owned(v) => Cow::Owned(v.get(n..).unwrap_or_default().to_vec()),
    }
}

fn frame(id: &[u8; 4], d: &[u8], major: u8, r: &mut Reader) {
    match (major, id) {
        (2, b"ULT\0") | (3 | 4, b"USLT") => uslt(d, r),
        (2, b"SLT\0") | (3 | 4, b"SYLT") => sylt(d, r),
        (3 | 4, b"CHAP") => chap(d, major, &mut r.out),
        _ => {}
    }
}

fn uslt(d: &[u8], r: &mut Reader) {
    let (Some(&enc), Some(lang), Some(rest)) = (d.first(), d.get(1..4), d.get(4..)) else { return };
    let (desc, text) = split(enc, rest);
    // An empty frame carries nothing worth keeping.
    if text.is_empty() || !r.spend(text.len().saturating_add(desc.len())) {
        return;
    }
    r.out.lyrics.push(Lyrics { language: latin1(lang), description: decode(enc, desc), text: decode(enc, text) });
}

fn sylt(d: &[u8], r: &mut Reader) {
    let (Some(&enc), Some(lang), Some(&stamp), Some(rest)) = (d.first(), d.get(1..4), d.get(4), d.get(6..)) else { return };
    // 2: milliseconds; 1 (MPEG frames) would need the stream's frame length.
    if stamp != 2 {
        return;
    }
    let (desc, mut rest) = split(enc, rest);
    if !r.spend(desc.len()) {
        return;
    }
    let mut lyrics = SyncedLyrics { language: latin1(lang), description: decode(enc, desc), lines: Vec::new() };
    while !rest.is_empty() {
        if r.lines >= MAX_SYNCED_LINES {
            r.out.limited = true;
            break;
        }
        let (text, after) = split(enc, rest);
        let Some(ms) = be_u32(after) else { break };
        rest = after.get(4..).unwrap_or_default();
        if !r.spend(text.len()) {
            break;
        }
        lyrics.lines.push(SyncedLine { ms, text: decode(enc, text) });
        r.lines += 1;
    }
    // A frame without lines carries nothing worth keeping, and its descriptor is not kept either.
    if lyrics.lines.is_empty() {
        r.budget += desc.len();
    } else {
        r.out.synced.push(lyrics);
    }
}

/// At most [`MAX_TEXT_BYTES`] of `b` (and `limited` when that cuts it).
fn capped<'a>(b: &'a [u8], out: &mut Tags) -> &'a [u8] {
    if b.len() > MAX_TEXT_BYTES {
        out.limited = true;
    }
    b.get(..b.len().min(MAX_TEXT_BYTES)).unwrap_or_default()
}

fn chap(d: &[u8], major: u8, out: &mut Tags) {
    if out.markers.len() >= MAX_MARKERS {
        out.limited = true;
        return;
    }
    let (element, rest) = split(0, d);
    let (Some(start), Some(end), Some(subframes)) = (be_u32(rest), rest.get(4..).and_then(be_u32), rest.get(16..)) else { return };
    let mut name = None;
    let mut frames = Frames::new(subframes, major, false);
    for (id, data) in frames.by_ref() {
        if &id == b"TIT2"
            && let Some((&enc, text)) = data.split_first()
        {
            let text = capped(text, out);
            name = Some(decode(enc, split(enc, text).0));
            break;
        }
    }
    if frames.capped {
        out.limited = true;
    }
    let name = match name.filter(|n| !n.trim().is_empty()) {
        Some(n) => n,
        None => latin1(capped(element, out)),
    };
    out.markers.push(Marker { start: f64::from(start), duration: f64::from(end.saturating_sub(start)), rate: 1000.0, name });
}

/// Splits a terminated string off the front of `b` in encoding `enc`: (string, the rest).
fn split(enc: u8, b: &[u8]) -> (&[u8], &[u8]) {
    let wide = enc == 1 || enc == 2;
    let found = if wide { b.as_chunks::<2>().0.iter().position(|c| *c == [0, 0]).map(|i| i * 2) } else { b.iter().position(|&c| c == 0) };
    match found {
        Some(i) => (b.get(..i).unwrap_or_default(), b.get(i + if wide { 2 } else { 1 }..).unwrap_or_default()),
        None => (b, &[]),
    }
}

fn latin1(b: &[u8]) -> String {
    b.iter().take_while(|&&c| c != 0).map(|&c| char::from(c)).collect()
}

/// Text in ID3 encoding `enc`, trailing NULs dropped.
fn decode(enc: u8, b: &[u8]) -> String {
    let text = match enc {
        0 => latin1(b),
        1 | 2 => {
            let (big, body) = match b {
                [0xFF, 0xFE, rest @ ..] => (false, rest),
                [0xFE, 0xFF, rest @ ..] => (true, rest),
                // UTF-16 with no byte order mark: big endian for 2, and in practice little for 1.
                _ => (enc == 2, b),
            };
            let units = body.as_chunks::<2>().0.iter().map(|&c| if big { u16::from_be_bytes(c) } else { u16::from_le_bytes(c) });
            char::decode_utf16(units).map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
        }
        _ => String::from_utf8_lossy(b).into_owned(),
    };
    text.trim_end_matches('\0').to_owned()
}

// ---------------------------------------------------------------------------------------------
// FLAC

fn le_u32(b: &[u8], at: usize) -> Option<usize> {
    let s = b.get(at..at.checked_add(4)?)?;
    usize::try_from(u32::from_le_bytes(s.try_into().ok()?)).ok()
}

fn be_u64(b: &[u8], at: usize) -> Option<u64> {
    let s = b.get(at..at.checked_add(8)?)?;
    Some(u64::from_be_bytes(s.try_into().ok()?))
}

/// Reads the metadata blocks of a FLAC stream that starts with `fLaC`.
fn flac(b: &[u8], r: &mut Reader) {
    let mut at = 4usize;
    let mut rate = 0u32;
    let mut cues = Vec::new();
    let mut blocks = 0usize;
    while let Some(header) = b.get(at..at.saturating_add(4)) {
        if blocks >= MAX_FLAC_BLOCKS {
            r.out.limited = true;
            break;
        }
        blocks += 1;
        let Some(len) = be(header.get(1..).unwrap_or_default(), 3) else { break };
        let kind = header.first().map_or(127, |h| h & 0x7F);
        let last = header.first().is_some_and(|h| h & 0x80 != 0);
        let Some(start) = at.checked_add(4) else { break };
        let Some(end) = start.checked_add(len) else { break };
        let Some(data) = b.get(start..end) else { break };
        match kind {
            0 => rate = streaminfo_rate(data).unwrap_or(0),
            4 => vorbis_comments(data, r),
            5 => cues.extend(cuesheet(data)),
            127 => break,
            _ => {}
        }
        at = end;
        if last {
            break;
        }
    }
    for (samples, track) in cues {
        if r.out.markers.len() >= MAX_MARKERS {
            r.out.limited = true;
            break;
        }
        // Sample offsets are exact up to 2^53 samples (thousands of years at any rate).
        r.out.markers.push(Marker { start: samples as f64, duration: 0.0, rate: f64::from(rate), name: track.to_string() });
    }
}

fn streaminfo_rate(d: &[u8]) -> Option<u32> {
    let s = d.get(10..13)?;
    Some((u32::from(s[0]) << 12) | (u32::from(s[1]) << 4) | (u32::from(s[2]) >> 4))
}

fn vorbis_comments(d: &[u8], r: &mut Reader) {
    let Some(vendor) = le_u32(d, 0) else { return };
    let Some(mut at) = vendor.checked_add(4) else { return };
    let Some(count) = le_u32(d, at) else { return };
    at += 4;
    if count > MAX_COMMENTS {
        r.out.limited = true;
    }
    for _ in 0..count.min(MAX_COMMENTS) {
        let Some(len) = le_u32(d, at) else { return };
        let Some(start) = at.checked_add(4) else { return };
        let Some(end) = start.checked_add(len) else { return };
        let Some(field) = d.get(start..end) else { return };
        at = end;
        let Some(eq) = field.iter().position(|&c| c == b'=') else { continue };
        let (key, value) = (field.get(..eq).unwrap_or_default(), field.get(eq + 1..).unwrap_or_default());
        if key.eq_ignore_ascii_case(b"LYRICS") || key.eq_ignore_ascii_case(b"UNSYNCEDLYRICS") {
            if value.is_empty() || !r.spend(value.len()) {
                continue;
            }
            r.out.lyrics.push(Lyrics { language: String::new(), description: String::new(), text: String::from_utf8_lossy(value).into_owned() });
        }
    }
}

/// Cue sheet tracks as (sample offset of index point 1, or the first one, track number); the
/// lead-out track is left out.
fn cuesheet(d: &[u8]) -> Vec<(u64, u8)> {
    // Catalogue number (128), lead-in (8), CD flag and reserved (259), then the track count.
    const TRACKS_AT: usize = 128 + 8 + 259;
    const TRACK_LEN: usize = 8 + 1 + 12 + 1 + 13 + 1;
    const INDEX_LEN: usize = 8 + 1 + 3;
    let mut found = Vec::new();
    let Some(&count) = d.get(TRACKS_AT) else { return found };
    let mut at = TRACKS_AT + 1;
    for _ in 0..count {
        let (Some(offset), Some(&number), Some(&points)) = (be_u64(d, at), d.get(at + 8), d.get(at + TRACK_LEN - 1)) else { break };
        at += TRACK_LEN;
        let mut index = None;
        for _ in 0..points {
            let (Some(o), Some(&n)) = (be_u64(d, at), d.get(at + 8)) else { return found };
            at += INDEX_LEN;
            if index.is_none() || n == 1 {
                index = Some(o);
            }
        }
        if number == 170 || number == 255 {
            continue;
        }
        if let Some(sample) = offset.checked_add(index.unwrap_or(0)) {
            found.push((sample, number));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn syncsafe_bytes(n: usize) -> [u8; 4] {
        [(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F]
    }

    fn frame23(id: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut f = id.to_vec();
        f.extend((data.len() as u32).to_be_bytes());
        f.extend([0, 0]);
        f.extend(data);
        f
    }

    fn frame24(id: &[u8; 4], flags: u8, data: &[u8]) -> Vec<u8> {
        let mut f = id.to_vec();
        f.extend(syncsafe_bytes(data.len()));
        f.extend([0, flags]);
        f.extend(data);
        f
    }

    fn tag(major: u8, flags: u8, body: &[u8]) -> Vec<u8> {
        let mut t = b"ID3".to_vec();
        t.extend([major, 0, flags]);
        t.extend(syncsafe_bytes(body.len()));
        t.extend(body);
        t
    }

    fn utf16(s: &str) -> Vec<u8> {
        let mut v = vec![0xFF, 0xFE];
        v.extend(s.encode_utf16().flat_map(u16::to_le_bytes));
        v
    }

    fn v23() -> Vec<u8> {
        let mut uslt = vec![0];
        uslt.extend(b"eng\0Hello\nWorld");
        let mut sylt = vec![1];
        sylt.extend(b"deu");
        sylt.extend([2, 1]);
        sylt.extend(utf16("Words"));
        sylt.extend([0, 0]);
        for (text, ms) in [("Gr\u{fc}\u{df}", 1_500u32), ("\nzwei", 4_000)] {
            sylt.extend(utf16(text));
            sylt.extend([0, 0]);
            sylt.extend(ms.to_be_bytes());
        }
        let mut chap = b"ch0\0".to_vec();
        for n in [10_000u32, 25_000, u32::MAX, u32::MAX] {
            chap.extend(n.to_be_bytes());
        }
        chap.extend(frame23(b"TIT2", b"\x03Verse \xc3\xa9"));
        let mut chap2 = b"ch1\0".to_vec();
        for n in [25_000u32, 20_000, 0, 0] {
            chap2.extend(n.to_be_bytes());
        }
        let mut body =
            [frame23(b"TIT2", b"\0Song"), frame23(b"USLT", &uslt), frame23(b"SYLT", &sylt), frame23(b"CHAP", &chap), frame23(b"CHAP", &chap2)]
                .concat();
        body.extend([0; 32]);
        tag(3, 0, &body)
    }

    #[test]
    fn id3v23_lyrics_synced_lyrics_and_chapters() {
        let mut file = v23();
        file.extend([0xFF, 0xFB, 0x90, 0x00]);
        let t = read(&file);
        assert_eq!(t.lyrics, vec![Lyrics { language: "eng".into(), description: String::new(), text: "Hello\nWorld".into() }]);
        assert_eq!(t.synced.len(), 1);
        assert_eq!(t.synced[0].description, "Words");
        assert_eq!(t.synced[0].language, "deu");
        assert_eq!(t.synced[0].lines, vec![SyncedLine { ms: 1_500, text: "Gr\u{fc}\u{df}".into() }, SyncedLine { ms: 4_000, text: "\nzwei".into() }]);
        assert_eq!(
            t.markers,
            vec![
                Marker { start: 10_000.0, duration: 15_000.0, rate: 1000.0, name: "Verse \u{e9}".into() },
                Marker { start: 25_000.0, duration: 0.0, rate: 1000.0, name: "ch1".into() },
            ]
        );
        assert!(!t.limited);
    }

    #[test]
    fn id3v24_frame_flags_and_v22_frames() {
        // Data length indicator plus frame-level unsynchronisation (FF 00 in the text).
        let mut uslt = vec![3];
        uslt.extend(b"eng\0a\xc3\xbf");
        let synced = [&[0, 0, 0, 7][..], &unsync_encode(&uslt)].concat();
        let lyrics = frame24(b"USLT", 0x03, &synced);
        // Compressed: skipped.
        let skipped = frame24(b"USLT", 0x08, b"\0engx\0y");
        // Grouping byte in front.
        let grouped = frame24(b"USLT", 0x40, b"\x07\x00eng\x00grouped");
        let t = read(&tag(4, 0, &[lyrics, skipped, grouped].concat()));
        let texts: Vec<&str> = t.lyrics.iter().map(|l| l.text.as_str()).collect();
        assert_eq!(texts, vec!["a\u{ff}", "grouped"]);

        let mut ult = b"ULT".to_vec();
        let data = b"\0engdesc\0old lyrics";
        ult.extend([0, 0, data.len() as u8]);
        ult.extend(data);
        let t = read(&tag(2, 0, &ult));
        assert_eq!(t.lyrics.first().map(|l| (l.description.as_str(), l.text.as_str())), Some(("desc", "old lyrics")));
    }

    #[test]
    fn tag_level_unsynchronisation_and_extended_header() {
        let mut uslt = vec![0];
        uslt.extend(b"eng\0\xff\xe0");
        let body = frame23(b"USLT", &uslt);
        let mut ext = vec![0, 0, 0, 6, 0, 0, 0, 0, 0, 0];
        ext.extend(&body);
        let t = read(&tag(3, 0xC0, &unsync_encode(&ext)));
        assert_eq!(t.lyrics.first().map(|l| l.text.clone()), Some("\u{ff}\u{e0}".into()));
    }

    fn unsync_encode(b: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, &c) in b.iter().enumerate() {
            out.push(c);
            if c == 0xFF && b.get(i + 1).is_none_or(|&n| n == 0 || n & 0xE0 == 0xE0) {
                out.push(0);
            }
        }
        out
    }

    fn flac_file() -> Vec<u8> {
        let mut f = b"fLaC".to_vec();
        let mut info = vec![0u8; 34];
        // 48000 Hz = 0x0BB80 in 20 bits.
        info[10] = 0x0B;
        info[11] = 0xB8;
        info[12] = 0x00;
        f.push(0);
        f.extend(&(info.len() as u32).to_be_bytes()[1..]);
        f.extend(info);
        let mut vc = Vec::new();
        vc.extend(3u32.to_le_bytes());
        vc.extend(b"enc");
        vc.extend(3u32.to_le_bytes());
        for field in [&b"TITLE=x"[..], b"lyrics=Line one\nLine two", b"NOEQUALS"] {
            vc.extend((field.len() as u32).to_le_bytes());
            vc.extend(field);
        }
        f.push(4);
        f.extend(&(vc.len() as u32).to_be_bytes()[1..]);
        f.extend(vc);
        let mut cue = vec![0u8; 395];
        cue.push(3);
        for (offset, number, points) in [(0u64, 1u8, vec![(0u64, 0u8), (24_000, 1)]), (96_000, 2, vec![(0, 1)]), (200_000, 170, vec![])] {
            cue.extend(offset.to_be_bytes());
            cue.push(number);
            cue.extend([0u8; 12 + 1 + 13]);
            cue.push(points.len() as u8);
            for (o, n) in points {
                cue.extend(o.to_be_bytes());
                cue.push(n);
                cue.extend([0, 0, 0]);
            }
        }
        f.push(0x80 | 5);
        f.extend(&(cue.len() as u32).to_be_bytes()[1..]);
        f.extend(cue);
        f.extend([0xFF, 0xF8]);
        f
    }

    #[test]
    fn flac_vorbis_comment_lyrics_and_cue_sheet() {
        let t = read(&flac_file());
        assert_eq!(t.lyrics.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(), vec!["Line one\nLine two"]);
        assert_eq!(
            t.markers,
            vec![
                Marker { start: 24_000.0, duration: 0.0, rate: 48_000.0, name: "1".into() },
                Marker { start: 96_000.0, duration: 0.0, rate: 48_000.0, name: "2".into() },
            ]
        );
        // An ID3 tag in front of the FLAC stream is read too.
        let both = read(&[v23(), flac_file()].concat());
        assert_eq!(both.lyrics.len(), 2);
        assert_eq!(both.markers.len(), 4);
    }

    #[test]
    fn hostile_input_never_panics() {
        assert_eq!(read(b""), Tags::default());
        assert_eq!(read(b"RIFF\0\0\0\0WAVE"), Tags::default());
        for file in [v23(), flac_file(), [v23(), flac_file()].concat()] {
            for n in 0..file.len() {
                let _ = read(&file[..n]);
            }
            for i in 0..file.len() {
                for v in [0x00, 0x7F, 0x80, 0xFF] {
                    let mut f = file.clone();
                    f[i] = v;
                    let _ = read(&f);
                }
            }
        }
        // A huge declared size with a tiny body.
        let _ = read(b"ID3\x04\x00\x00\x7f\x7f\x7f\x7fUSLT\x7f\x7f\x7f\x7f\x00\x00");
        let _ = read(b"fLaC\x04\xff\xff\xff\xff\xff\xff\xff");
    }

    #[test]
    fn frame_cap_descriptors_and_names_set_limited() {
        // Lyrics after more frames than are read: reported as left out.
        let filler = frame23(b"TXXX", b"\0");
        let mut uslt = vec![0];
        uslt.extend(b"eng\0late");
        let mut body = Vec::with_capacity((MAX_FRAMES + 1) * filler.len());
        for _ in 0..MAX_FRAMES {
            body.extend(&filler);
        }
        body.extend(frame23(b"USLT", &uslt));
        let t = read(&tag(3, 0, &body));
        assert!(t.lyrics.is_empty() && t.limited);
        // Padding after the frames is not a cut.
        let t = read(&tag(3, 0, &[frame23(b"USLT", &uslt), vec![0; 64]].concat()));
        assert!(!t.limited);
        // A synchronised-lyrics descriptor over the text cap: the frame is dropped.
        let mut sylt = vec![0];
        sylt.extend(b"eng");
        sylt.extend([2, 1]);
        sylt.extend(vec![b'd'; MAX_TEXT_BYTES + 1]);
        sylt.extend([0, b'x', 0, 0, 0, 0, 1]);
        let t = read(&tag(3, 0, &frame23(b"SYLT", &sylt)));
        assert!(t.synced.is_empty() && t.limited);
        // Empty synchronised-lyrics frames keep nothing.
        let mut empty = vec![0];
        empty.extend(b"eng");
        empty.extend([2, 1, 0]);
        let t = read(&tag(3, 0, &frame23(b"SYLT", &empty).repeat(1_000)));
        assert!(t.synced.is_empty() && !t.limited);
        // Dropped frames with long descriptors do not use up the budget for lyrics after them.
        let mut long = vec![0];
        long.extend(b"eng");
        long.extend([2, 1]);
        long.extend(vec![b'd'; MAX_TEXT_BYTES]);
        long.push(0);
        let mut body = frame23(b"SYLT", &long).repeat(MAX_TEXT_TOTAL_BYTES / MAX_TEXT_BYTES + 1);
        body.extend(frame23(b"USLT", &uslt));
        let t = read(&tag(3, 0, &body));
        assert_eq!(t.lyrics.first().map(|l| l.text.as_str()), Some("late"));
        assert!(!t.limited);
        // An untitled chapter's element id over the text cap is cut.
        let mut chap = vec![b'c'; MAX_TEXT_BYTES + 10];
        chap.push(0);
        chap.extend([0u8; 16]);
        let t = read(&tag(3, 0, &frame23(b"CHAP", &chap)));
        assert!(t.markers.first().is_some_and(|m| m.name.len() <= MAX_TEXT_BYTES) && t.limited);
    }

    #[test]
    fn caps_set_limited() {
        let mut body = Vec::new();
        let mut chap = b"c\0".to_vec();
        chap.extend([0u8; 16]);
        let one = frame23(b"CHAP", &chap);
        for _ in 0..(MAX_MARKERS + 1) {
            body.extend(&one);
        }
        let t = read(&tag(3, 0, &body));
        assert_eq!(t.markers.len(), MAX_MARKERS);
        assert!(t.limited);
    }
}
