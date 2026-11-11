//! Chapter lists, in the two forms players read.
//!
//! - **QuickTime chapter track** (QuickTime File Format, "Chapter lists" and "Text media"): a
//!   disabled text track whose samples are the chapter titles, referenced from a video or audio track
//!   by a `tref/chap` track reference. QuickTime Player, Apple TV, iOS and Final Cut read this form.
//!   Each sample is a 16-bit big-endian length, the title, and an `encd` atom with value `0x100`
//!   marking the text as UTF-8 (the convention Apple's own chapter tracks in `.m4a`/`.m4v` files use;
//!   plain QuickTime text is Mac Roman).
//! - **Nero chapter list** `moov/udta/chpl`, version 1: `u32` reserved, `u8` chapter count, then per
//!   chapter a `u64` start in 100 ns units, a `u8` title length and the UTF-8 title. It has no
//!   official specification; this is the layout publicly documented for Nero Digital files and
//!   written by many tools. VLC and ffprobe (`-show_chapters`) read it.
//!
//! Both forms carry at most [`MAX_CHAPTERS`] chapters with titles of at most
//! [`MAX_CHAPTER_TITLE_BYTES`] bytes (the `chpl` count and length fields are one byte each).

use crate::isobmff::bytes::{BoxBuf, Cur};

/// One chapter mark: `start` in the movie timescale ([`crate::isobmff::WriterOptions::movie_timescale`]
/// when writing, [`crate::isobmff::Mp4File::timescale`] when reading), `title` UTF-8.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chapter {
    /// Start of the chapter, in the movie timescale.
    pub start: u64,
    /// The chapter's title.
    pub title: String,
}

/// Most chapters written (the `chpl` count is one byte); later ones are dropped.
pub const MAX_CHAPTERS: usize = 255;
/// Longest chapter title in bytes (the `chpl` length is one byte); longer titles are cut at a
/// character boundary.
pub const MAX_CHAPTER_TITLE_BYTES: usize = 255;

/// `chpl` times are in 100 ns units.
const CHPL_UNITS_PER_SECOND: u64 = 10_000_000;

/// Cut `s` to at most `max` bytes without splitting a character.
pub(crate) fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.get(..end).unwrap_or("")
}

/// The chapters a file can carry: sorted by start, one per start time (the first title given for
/// a time wins), starts before `duration` only (when it is known), titles cut to
/// [`MAX_CHAPTER_TITLE_BYTES`] with control characters replaced by spaces, at most
/// [`MAX_CHAPTERS`].
pub fn normalize_chapters(chapters: &[Chapter], duration: Option<u64>) -> Vec<Chapter> {
    let mut v: Vec<&Chapter> = chapters.iter().filter(|c| duration.is_none_or(|d| c.start < d)).collect();
    v.sort_by_key(|c| c.start);
    v.dedup_by_key(|c| c.start);
    v.into_iter()
        .take(MAX_CHAPTERS)
        .map(|c| {
            let clean: String = c.title.chars().map(|ch| if ch.is_control() { ' ' } else { ch }).collect();
            Chapter { start: c.start, title: truncate_utf8(clean.trim(), MAX_CHAPTER_TITLE_BYTES).to_string() }
        })
        .collect()
}

fn rescale(v: u64, from: u64, to: u64) -> u64 {
    if from == 0 {
        return 0;
    }
    (u128::from(v) * u128::from(to) / u128::from(from)).min(u128::from(u64::MAX)) as u64
}

/// Write `udta/chpl` content (the box itself) for already normalized chapters.
pub(crate) fn write_chpl(b: &mut BoxBuf, chapters: &[Chapter], movie_timescale: u32) {
    let m = b.start_full(b"chpl", 1, 0);
    b.u32(0);
    let n = chapters.len().min(MAX_CHAPTERS);
    b.u8(n as u8);
    for c in chapters.iter().take(n) {
        b.u64(rescale(c.start, u64::from(movie_timescale), CHPL_UNITS_PER_SECOND));
        let t = truncate_utf8(&c.title, MAX_CHAPTER_TITLE_BYTES);
        b.u8(t.len() as u8);
        b.bytes(t.as_bytes());
    }
    b.end(m);
}

/// Parse a `chpl` payload (after the box header) into chapters in `movie_timescale` units.
/// Malformed or truncated lists yield the chapters read so far.
pub(crate) fn parse_chpl(p: &[u8], movie_timescale: u32) -> Vec<Chapter> {
    let mut out = Vec::new();
    let mut c = Cur::labeled(p, "chpl");
    let Ok((version, _)) = c.full_header() else { return out };
    if version >= 1 && c.skip(4).is_err() {
        return out;
    }
    let Ok(n) = c.u8() else { return out };
    for _ in 0..n {
        let Ok(start) = c.u64() else { break };
        let Ok(len) = c.u8() else { break };
        let Ok(title) = c.bytes(len as usize) else { break };
        out.push(Chapter {
            start: rescale(start, CHPL_UNITS_PER_SECOND, u64::from(movie_timescale)),
            title: String::from_utf8_lossy(title).into_owned(),
        });
    }
    out
}

/// Payload of the QuickTime `text` sample description (after its 8-byte box header), QTFF "Text
/// sample description": reserved + data reference index, display flags, justification, background
/// colour, default text box, reserved, font number and face, reserved, foreground colour, an empty
/// Pascal font name, then an `ftab` font table naming one font.
pub(crate) fn text_sample_entry_payload() -> Vec<u8> {
    let mut b = BoxBuf::new();
    b.zeros(6);
    b.u16(1); // data reference index
    b.u32(0); // display flags
    b.i32(0); // justification: left
    b.zeros(6); // background colour (RGB)
    b.zeros(8); // default text box (top, left, bottom, right)
    b.zeros(8); // reserved
    b.u16(1); // font number (matches the ftab entry)
    b.u16(0); // font face: plain
    b.u8(0); // reserved
    b.u16(0); // reserved
    b.zeros(6); // foreground colour (RGB)
    b.u8(0); // text name: empty Pascal string
    let f = b.start(b"ftab");
    b.u16(1);
    b.u16(1);
    let name = b"Sans-Serif";
    b.u8(name.len() as u8);
    b.bytes(name);
    b.end(f);
    b.buf
}

/// One chapter-track sample: length-prefixed UTF-8 title plus an `encd` atom (UTF-8).
pub(crate) fn text_sample(title: &str) -> Vec<u8> {
    let t = truncate_utf8(title, MAX_CHAPTER_TITLE_BYTES);
    let mut b = BoxBuf::new();
    b.u16(t.len() as u16);
    b.bytes(t.as_bytes());
    let e = b.start(b"encd");
    b.u32(0x100);
    b.end(e);
    b.buf
}

/// The title in a QuickTime text sample (UTF-8, or UTF-16 when it starts with a byte order mark).
pub fn parse_text_sample(data: &[u8]) -> Option<String> {
    let mut c = Cur::labeled(data, "text sample");
    let n = c.u16().ok()? as usize;
    decode_title(c.bytes(n).ok()?)
}

/// [`parse_text_sample`] of the start of a longer sample: as much of the title as `data` holds.
pub(crate) fn parse_text_sample_prefix(data: &[u8]) -> Option<String> {
    let (declared, rest) = data.split_first_chunk::<2>()?;
    decode_title(rest.get(..usize::from(u16::from_be_bytes(*declared))).unwrap_or(rest))
}

fn decode_title(t: &[u8]) -> Option<String> {
    if let Some(rest) = t.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|u| u16::from_be_bytes(*u)).collect();
        return Some(String::from_utf16_lossy(&units));
    }
    if let Some(rest) = t.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest.as_chunks::<2>().0.iter().map(|u| u16::from_le_bytes(*u)).collect();
        return Some(String::from_utf16_lossy(&units));
    }
    // trailing atoms (`encd`, `styl` …) are not part of the text
    Some(String::from_utf8_lossy(t).into_owned())
}
