#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Caption and subtitle files behind one neutral model.
//!
//! - **Files:** [SubRip](srt) (`.srt`), [WebVTT](vtt) (`.vtt`), [Scenarist SCC](scc) (`.scc`,
//!   CEA-608 pop-on, roll-up and paint-on on read; pop-on on write), [MacCaption MCC](mcc)
//!   (`.mcc`, SMPTE ST 334-2 packets with CEA-608 and CEA-708), [EBU STL](stl) (`.stl`, EBU
//!   Tech 3264) and [TTML](ttml) (`.ttml` IMSC1 Text profile, `.dfxp` DFXP) parse into a
//!   [`Document`] of [`Cue`]s and write back. Readers are forgiving (BOM, UTF-16, Windows-1252
//!   fallback, CRLF/CR, missing SRT indexes, `,` or `.` milliseconds, missing hours, no blank
//!   line between cues); writers are strict. WebVTT cue identifiers, cue settings, `<v Speaker>`
//!   voices and STYLE/REGION/NOTE blocks are kept.
//! - **Byte pairs:** [`cea608`] has the CEA-608 character sets and control codes;
//!   [`scc::decode_pairs`] turns a timed stream of 608 byte pairs into cues.
//! - **Time:** cue times are exact [`Tick`]s ([`time`]). SRT/VTT milliseconds and SCC 29.97 fps
//!   timecodes (drop-frame or not) convert exactly; [`Document::snap_to_frames`] snaps to a
//!   frame grid.
//! - **Limits:** input is at most [`MAX_INPUT`] bytes and a document holds at most [`MAX_CUES`]
//!   cues; TTML is read through `craft-xml` with its nesting, node and attribute limits.
//!
//! ```
//! use craft_captions::{Format, WriteOptions, detect, parse, write};
//! let srt = b"1\n00:00:01,000 --> 00:00:02,500\nHello\n";
//! assert_eq!(detect(srt, Some("srt")), Some(Format::Srt));
//! let doc = parse(srt, Format::Srt)?;
//! assert_eq!(doc.cues[0].text, "Hello");
//! let vtt = write(&doc, Format::WebVtt, WriteOptions::default());
//! assert!(vtt.starts_with(b"WEBVTT"));
//! # Ok::<(), craft_captions::Error>(())
//! ```

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

pub mod cea608;
pub mod mcc;
pub mod scc;
pub mod srt;
pub mod stl;
mod text;
/// Exact time: `craft-timeline`'s ticks, frame rates and timecode fields.
pub use craft_timeline::time;
pub mod ttml;
pub mod vtt;

pub use text::{decode_entities, entity_at, plain_text};
pub use time::{FrameRate, TICKS_PER_SECOND, Tick};

/// The creating program MCC and EBU STL files name when the caller gives none.
pub const PROGRAM: &str = "craft-captions";

/// The largest caption file [`parse`] reads, in bytes.
pub const MAX_INPUT: usize = 64 << 20;

/// The most cues a parsed [`Document`] holds; a reader stops there with a warning.
pub const MAX_CUES: usize = 1_000_000;

/// A caption file format.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Srt,
    WebVtt,
    Scc,
    /// MacCaption MCC (CEA-608 + CEA-708 in SMPTE ST 334-2 packets).
    Mcc,
    /// EBU STL (EBU Tech 3264).
    Stl,
    /// TTML, IMSC1 Text profile.
    Ttml,
    /// DFXP (TTML1).
    Dfxp,
}

impl Format {
    pub const ALL: [Format; 7] = [Format::Srt, Format::WebVtt, Format::Scc, Format::Mcc, Format::Stl, Format::Ttml, Format::Dfxp];
    pub fn name(self) -> &'static str {
        match self {
            Format::Srt => "SubRip (SRT)",
            Format::WebVtt => "WebVTT",
            Format::Scc => "Scenarist SCC (CEA-608)",
            Format::Mcc => "MacCaption MCC (CEA-608/708)",
            Format::Stl => "EBU STL",
            Format::Ttml => "TTML (IMSC1)",
            Format::Dfxp => "DFXP (TTML1)",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Format::Srt => "srt",
            Format::WebVtt => "vtt",
            Format::Scc => "scc",
            Format::Mcc => "mcc",
            Format::Stl => "stl",
            Format::Ttml => "ttml",
            Format::Dfxp => "dfxp",
        }
    }
    pub fn from_name(s: &str) -> Option<Format> {
        match s.trim_start_matches('.').to_ascii_lowercase().as_str() {
            "srt" | "subrip" => Some(Format::Srt),
            "vtt" | "webvtt" => Some(Format::WebVtt),
            "scc" | "cea608" | "cea-608" => Some(Format::Scc),
            "mcc" | "maccaption" | "cea708" | "cea-708" => Some(Format::Mcc),
            "stl" | "ebu-stl" | "ebustl" | "ebu" => Some(Format::Stl),
            "ttml" | "imsc" | "imsc1" | "xml" => Some(Format::Ttml),
            "dfxp" => Some(Format::Dfxp),
            _ => None,
        }
    }
    /// Whether the format is a binary file (EBU STL).
    pub fn is_binary(self) -> bool {
        self == Format::Stl
    }
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum Error {
    #[error("not a {0} file")]
    NotFormat(&'static str),
    #[error("line {line}: {msg}")]
    Syntax { line: usize, msg: String },
    /// The input is longer than [`MAX_INPUT`].
    #[error("the caption file is larger than {0} bytes")]
    TooLarge(usize),
}

pub type Result<T> = std::result::Result<T, Error>;

/// One caption cue.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cue {
    pub start: Tick,
    pub end: Tick,
    /// Text as written (lines separated by `\n`, inline tags kept).
    pub text: String,
    /// WebVTT voice (`<v Name>`).
    pub speaker: Option<String>,
    /// WebVTT cue identifier.
    pub id: Option<String>,
    /// WebVTT cue settings, verbatim.
    pub settings: String,
}

/// A parsed caption file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Document {
    pub cues: Vec<Cue>,
    /// WebVTT STYLE / REGION / NOTE blocks (verbatim, without trailing blank lines).
    pub blocks: Vec<String>,
    /// Non-fatal problems found while reading (skipped cues…).
    pub warnings: Vec<String>,
    /// SCC / MCC: the time code label of time zero, read as a time ([`label_time`]: a sequence's
    /// start timecode on export). The writers count their time codes from the label it reads as,
    /// so 01:00:00:00 at 25 fps is 01:00:00;00 or 01:00:00:00 at 29.97. Readers leave it at zero.
    pub start_timecode: Tick,
    /// SCC / MCC read from a file: the frame rate of its time code labels and whether they are
    /// drop-frame. [`Document::count_from`] needs it.
    pub labels: Option<(FrameRate, bool)>,
    /// BCP 47 language tag of the text, when known: TTML's `xml:lang` on import, the caption
    /// track's language on export. TTML and DFXP write it (else `en`).
    pub language: Option<String>,
}

/// Frame `frame` of a `rate` time code (drop-frame or not) as its label read as a time: hours,
/// minutes, seconds and the frame's share of a second at the label's nominal rate (25 for 25 fps,
/// 30 for 29.97). Negative frames read as zero.
pub fn label_time(frame: i64, rate: FrameRate, drop_frame: bool) -> Tick {
    let base = rate.timecode_base();
    let (_, h, m, s, f) = crate::time::frames_to_fields(frame.max(0), rate, drop_frame && rate.supports_drop_frame());
    let seconds = h.saturating_mul(3600).saturating_add(m.saturating_mul(60)).saturating_add(s);
    Tick(TICKS_PER_SECOND.saturating_mul(seconds)) + FrameRate::new(base, 1).tick_of(f)
}

/// The frame of a `rate` time code (drop-frame or not) whose label reads as `label` (as
/// [`label_time`] gives it), to the nearest frame of the label's nominal rate.
pub fn label_frame(label: Tick, rate: FrameRate, drop_frame: bool) -> i64 {
    let nominal = FrameRate::new(rate.timecode_base(), 1);
    let n = nominal.frame_at(nominal.snap_nearest(label.max(Tick::ZERO)));
    let (_, h, m, s, f) = crate::time::frames_to_fields(n, nominal, false);
    crate::time::fields_to_frames(h, m, s, f, rate, drop_frame && rate.supports_drop_frame())
}

impl Document {
    /// SCC / MCC read from a file: count cue times from the time code labelled `start` (as
    /// [`Document::start_timecode`]) rather than from 00:00:00:00, the inverse of what the writers
    /// do. Only when every cue is at or after it, so a file that counts from zero keeps its times.
    /// Returns whether the cues moved.
    pub fn count_from(&mut self, start: Tick) -> bool {
        let Some((rate, drop_frame)) = self.labels else { return false };
        let origin = rate.tick_of(label_frame(start, rate, drop_frame));
        if origin <= Tick::ZERO || self.cues.iter().any(|c| c.start < origin) {
            return false;
        }
        for c in &mut self.cues {
            c.start -= origin;
            c.end -= origin;
        }
        true
    }

    /// Snap cue times to frame boundaries of `rate` (nearest), keeping every cue at least one frame
    /// long and removing overlaps (a cue is cut where the next one starts).
    pub fn snap_to_frames(&mut self, rate: FrameRate) {
        for c in &mut self.cues {
            c.start = rate.snap_nearest(c.start);
            c.end = rate.snap_nearest(c.end).max(c.start + rate.frame_duration());
        }
        self.normalize();
    }

    /// Sort by start, drop empty cues and cut overlaps.
    pub fn normalize(&mut self) {
        self.cues.retain(|c| c.end > c.start);
        self.cues.sort_by_key(|c| (c.start, c.end));
        let mut out: Vec<Cue> = Vec::with_capacity(self.cues.len());
        for c in self.cues.drain(..) {
            let mut c = c;
            if let Some(prev) = out.last_mut()
                && c.start < prev.end
            {
                if c.start > prev.start {
                    prev.end = c.start;
                } else {
                    // same start: keep the first, shift this one after it
                    c.start = prev.end;
                    if c.end <= c.start {
                        self.warnings.push(format!("dropped overlapping cue \"{}\"", c.text.lines().next().unwrap_or("")));
                        continue;
                    }
                }
            }
            out.push(c);
        }
        self.cues = out;
    }
}

/// Guess a caption format from content (and the file extension as a hint).
pub fn detect(bytes: &[u8], ext: Option<&str>) -> Option<Format> {
    if stl::sniff(bytes) {
        return Some(Format::Stl);
    }
    let text = decode_text(&bytes[..bytes.len().min(4096)]);
    let head = text.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with("WEBVTT") {
        return Some(Format::WebVtt);
    }
    if head.starts_with("Scenarist_SCC") {
        return Some(Format::Scc);
    }
    if head.starts_with("File Format=MacCaption_MCC") {
        return Some(Format::Mcc);
    }
    let ext = ext.map(|e| e.trim_start_matches('.').to_ascii_lowercase());
    if (head.starts_with('<') && (head.contains("<tt ") || head.contains("<tt>") || head.contains(":tt ")))
        && (head.contains("ns/ttml") || head.contains("ttaf1"))
    {
        let legacy = head.contains("ttaf1") || head.contains("dfxp");
        return Some(if ext.as_deref() == Some("dfxp") || (legacy && ext.as_deref() != Some("ttml")) { Format::Dfxp } else { Format::Ttml });
    }
    let has_arrow_timing = head.lines().take(40).any(|l| srt::parse_timing_line(l).is_some());
    match ext.as_deref() {
        Some("srt") if has_arrow_timing || head.is_empty() => Some(Format::Srt),
        Some("vtt") if has_arrow_timing => Some(Format::WebVtt),
        Some("srt" | "vtt" | "scc" | "mcc" | "stl" | "ttml" | "dfxp" | "xml") => None,
        _ if has_arrow_timing => Some(Format::Srt),
        _ => None,
    }
}

/// Parse caption bytes.
pub fn parse(bytes: &[u8], format: Format) -> Result<Document> {
    if bytes.len() > MAX_INPUT {
        return Err(Error::TooLarge(MAX_INPUT));
    }
    let mut doc = parse_unbounded(bytes, format)?;
    cap_cues(&mut doc, MAX_CUES);
    Ok(doc)
}

fn cap_cues(doc: &mut Document, max: usize) {
    if doc.cues.len() > max {
        doc.cues.truncate(max);
        doc.warnings.push(format!("more than {max} cues: the rest were skipped"));
    }
}

fn parse_unbounded(bytes: &[u8], format: Format) -> Result<Document> {
    if format == Format::Stl {
        return stl::parse(bytes);
    }
    let text = decode_text(bytes);
    match format {
        Format::Srt => srt::parse(&text),
        Format::WebVtt => vtt::parse(&text),
        Format::Scc => scc::parse(&text),
        Format::Mcc => mcc::parse(&text),
        Format::Ttml | Format::Dfxp => ttml::parse(&text),
        Format::Stl => stl::parse(bytes),
    }
}

/// Options for [`write`].
#[derive(Clone, Copy, Debug)]
pub struct WriteOptions {
    /// SCC / MCC: drop-frame timecode (the usual choice for 29.97 fps).
    pub drop_frame: bool,
    /// The sequence frame rate: TTML (IMSC1) writes exact frame times at it; EBU STL picks
    /// `STL25.01` or `STL30.01` from it (None = 25 fps). Other formats ignore it.
    pub rate: Option<FrameRate>,
    /// MCC / EBU STL: the name written as the creating program ([`PROGRAM`] by default).
    pub program: &'static str,
    /// MCC: a packet on every frame from the start to this time (the end of the programme), with
    /// no caption data where there is none, as SDI and frame-wrapped MXF carry one per frame.
    /// The frames are MCC's 29.97 fps ones, so this is one packet per video frame only for
    /// 29.97 fps video. None (the default) writes only the frames that carry caption data.
    pub every_frame_until: Option<Tick>,
}

impl Default for WriteOptions {
    fn default() -> Self {
        Self { drop_frame: true, rate: None, program: PROGRAM, every_frame_until: None }
    }
}

/// Write a document.
pub fn write(doc: &Document, format: Format, opts: WriteOptions) -> Vec<u8> {
    match format {
        Format::Srt => srt::write(doc).into_bytes(),
        Format::WebVtt => vtt::write(doc).into_bytes(),
        Format::Scc => scc::write(doc, opts.drop_frame).into_bytes(),
        Format::Mcc => mcc::write_padded(doc, opts.drop_frame, true, true, mcc_pad_frames(opts), opts.program).into_bytes(),
        Format::Stl => stl::write(doc, opts.rate, opts.program),
        Format::Ttml => ttml::write(doc, ttml::Flavor::Imsc1, opts.rate, doc.language.as_deref().unwrap_or("")).into_bytes(),
        Format::Dfxp => ttml::write(doc, ttml::Flavor::Dfxp, opts.rate, doc.language.as_deref().unwrap_or("")).into_bytes(),
    }
}

/// How many characters [`charset_warnings`] names one by one per caption stream; the rest share one warning.
const LISTED_CHARS: usize = 20;
/// How many cut captions [`charset_warnings`] names one by one per caption stream; the rest share one warning.
const LISTED_CUTS: usize = 20;
/// The rows of 32 columns CEA-608 shows ([`scc::caption_rows`] keeps at most this many).
const ROWS_608: usize = 15;

/// What writing `doc` as `format` loses of its text, the characters the caption data cannot carry
/// and the text cut to fit. EBU STL writes `?` for the characters ISO/IEC 6937 has no code for.
/// SCC and MCC drop the characters CEA-608 has no code for (…) or send a close one (’ as '), and
/// MCC's CEA-708 service sends `?` for the characters it has no code for. One warning per such
/// character, in order of first use, with how often and the first caption that has it. CEA-608
/// shows at most 15 rows of 32 characters and MCC's CEA-708 window 12 rows: one warning per
/// caption cut to fit, with how many characters it lost (spaces not counted) and where the lost
/// text starts. Other formats: none.
pub fn charset_warnings(doc: &Document, format: Format, opts: WriteOptions) -> Vec<String> {
    let mut out = Vec::new();
    // the time code the file has for a caption: counted from the start time code, as the writers do
    let origin = label_frame(doc.start_timecode, scc::RATE, opts.drop_frame);
    let at_608 = |t: Tick| scc::format_timecode(scc::frame_of(t).saturating_add(origin), opts.drop_frame);
    // the rows each writer loads, so the warnings name exactly what the files leave out
    if matches!(format, Format::Scc | Format::Mcc) {
        lossy_chars(doc, "CEA-608", |text| changed_in_rows(&scc::caption_rows(text), cea608::shown_as), at_608, &mut out);
        cut_rows(doc, &format!("CEA-608 shows {ROWS_608} rows of 32 characters"), scc::caption_rows, at_608, &mut out);
    }
    if format == Format::Mcc {
        let shown_as = |c| Some(mcc::shown_as_708(c));
        lossy_chars(doc, "CEA-708", |text| changed_in_rows(&mcc::window_rows(text), shown_as), at_608, &mut out);
        let limit = format!("The CEA-708 window holds {} rows of 32 characters", mcc::WINDOW_ROWS);
        cut_rows(doc, &limit, mcc::window_rows, at_608, &mut out);
    }
    if format == Format::Stl {
        let unencodable = |text: &str| -> Vec<(char, Option<char>)> { stl::unencodable(text).into_iter().map(|c| (c, Some('?'))).collect() };
        lossy_chars(doc, "EBU STL (ISO/IEC 6937)", unencodable, |t| stl::timecode(t, opts.rate), &mut out);
    }
    out
}

/// The characters of the `rows` a writer loads that `shown_as` changes, with what a decoder shows
/// instead.
fn changed_in_rows(rows: &[String], shown_as: impl Fn(char) -> Option<char>) -> Vec<(char, Option<char>)> {
    rows.iter().flat_map(|r| r.chars()).map(|c| (c, shown_as(c))).filter(|(c, shown)| *shown != Some(*c)).collect()
}

/// `n` of `what`, singular for one.
fn count(n: usize, what: &str) -> String {
    if n == 1 { format!("1 {what}") } else { format!("{n} {what}s") }
}

/// Warnings for the captions of `doc` whose text the writer's `rows_of` (the rows it loads for a
/// caption's text) does not hold in full, as `limit` says: how many characters each loses, not
/// counting spaces, and the start of the lost text.
fn cut_rows(doc: &Document, limit: &str, rows_of: impl Fn(&str) -> Vec<String>, at: impl Fn(Tick) -> String, out: &mut Vec<String>) {
    let visible = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count();
    let mut cues: Vec<&Cue> = doc.cues.iter().filter(|c| c.end > c.start).collect();
    cues.sort_by_key(|c| c.start);
    // (start of the caption, the text it loses)
    let mut cut: Vec<(Tick, String)> = Vec::new();
    for cue in cues {
        let text = plain_text(&cue.text);
        // the rows keep a prefix of the text: the lost part starts at the first character past them
        let kept: usize = rows_of(&cue.text).iter().map(|r| visible(r)).sum();
        if let Some((i, _)) = text.char_indices().filter(|(_, c)| !c.is_whitespace()).nth(kept) {
            cut.push((cue.start, text[i..].to_string()));
        }
    }
    for (start, lost) in cut.iter().take(LISTED_CUTS) {
        // the first characters of the lost text, on one line
        let lost_text = lost.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut from: String = lost_text.chars().take(24).collect();
        if from.len() < lost_text.len() {
            from.push('…');
        }
        let n = visible(lost);
        let what = if n == 1 { "its last character".to_string() } else { format!("its last {n} characters") };
        out.push(format!("{limit}: the caption at {} lost {what}, from \"{from}\"", at(*start)));
    }
    if let Some(rest) = cut.get(LISTED_CUTS..).filter(|rest| !rest.is_empty()) {
        let n = rest.iter().fold(0usize, |n, (_, lost)| n.saturating_add(visible(lost)));
        let captions = if rest.len() == 1 { "caption was" } else { "captions were" };
        out.push(format!("{limit}: {} more {captions} cut to fit too, losing {}", rest.len(), count(n, "character")));
    }
}

/// Warnings for the characters of `doc` that the writer changes (`lost_in` gives them for a
/// caption's text, with what a decoder shows instead), timed by `at`.
fn lossy_chars(
    doc: &Document,
    stream: &str,
    lost_in: impl Fn(&str) -> Vec<(char, Option<char>)>,
    at: impl Fn(Tick) -> String,
    out: &mut Vec<String>,
) {
    // (character, what a decoder shows instead, count, start of the first caption with it)
    let mut lost: Vec<(char, Option<char>, usize, Tick)> = Vec::new();
    let mut index: std::collections::HashMap<char, usize> = std::collections::HashMap::new();
    let mut cues: Vec<&Cue> = doc.cues.iter().filter(|c| c.end > c.start).collect();
    cues.sort_by_key(|c| c.start);
    for cue in cues {
        for (c, shown) in lost_in(&cue.text) {
            match index.get(&c).and_then(|&i| lost.get_mut(i)) {
                Some(entry) => entry.2 = entry.2.saturating_add(1),
                None => {
                    index.insert(c, lost.len());
                    lost.push((c, shown, 1, cue.start));
                }
            }
        }
    }
    let times = |n: usize| if n == 1 { "once".to_string() } else { format!("{n} times") };
    for &(c, shown, n, start) in lost.iter().take(LISTED_CHARS) {
        let what = shown.map_or_else(|| "dropped".to_string(), |s| format!("written as \"{s}\""));
        out.push(format!("{stream} has no \"{c}\" (U+{:04X}): {what} {}, first in the caption at {}", c as u32, times(n), at(start)));
    }
    if let Some(rest) = lost.get(LISTED_CHARS..).filter(|rest| !rest.is_empty()) {
        let n = rest.iter().fold(0usize, |n, e| n.saturating_add(e.2));
        out.push(format!("{stream} has no code for {} more characters either: changed or dropped {}", rest.len(), times(n)));
    }
}

/// [`write`] into `out`. MCC goes out line by line, so a long `every_frame_until` export is never
/// held in memory; the other formats are small and written whole.
pub fn write_to(out: &mut dyn std::io::Write, doc: &Document, format: Format, opts: WriteOptions) -> std::io::Result<()> {
    match format {
        Format::Mcc => mcc::write_padded_to(out, doc, opts.drop_frame, true, true, mcc_pad_frames(opts), opts.program),
        _ => out.write_all(&write(doc, format, opts)),
    }
}

/// The MCC frames `opts.every_frame_until` asks for: those that start before it, so a last
/// partial frame gets its packet too.
fn mcc_pad_frames(opts: WriteOptions) -> i64 {
    opts.every_frame_until.map_or(0, |end| {
        let f = mcc::RATE.frame_at(end);
        if mcc::RATE.tick_of(f) < end { f.saturating_add(1) } else { f }
    })
}

/// Decode text bytes: UTF-8 (BOM stripped), UTF-16 with BOM, else Windows-1252. Line endings are
/// normalised to `\n`.
pub fn decode_text(bytes: &[u8]) -> String {
    let s = if let Some(rest) = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
        let le = bytes[0] == 0xff;
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| if le { u16::from_le_bytes([c[0], c[1]]) } else { u16::from_be_bytes([c[0], c[1]]) })
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        match std::str::from_utf8(bytes) {
            Ok(s) => s.to_string(),
            Err(_) => bytes.iter().map(|&b| cp1252(b)).collect(),
        }
    };
    let s = s.strip_prefix('\u{feff}').map(str::to_string).unwrap_or(s);
    if s.contains('\r') { s.replace("\r\n", "\n").replace('\r', "\n") } else { s }
}

fn cp1252(b: u8) -> char {
    const HI: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—',
        '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
    ];
    if (0x80..0xa0).contains(&b) { HI[(b - 0x80) as usize] } else { b as char }
}

pub(crate) const TICKS_PER_MS: i64 = TICKS_PER_SECOND / 1000;

/// Parse `[H…:]MM:SS[.,]fff` (the fraction may have 1–9 digits) into ticks.
pub fn parse_clock(s: &str) -> Option<Tick> {
    let s = s.trim();
    let (hms, frac) = match s.find([',', '.']) {
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };
    let parts: Vec<&str> = hms.split(':').collect();
    let num = |p: &str| -> Option<i64> { if !p.is_empty() && p.len() <= 9 && p.bytes().all(|b| b.is_ascii_digit()) { p.parse().ok() } else { None } };
    let (h, m, sec) = match parts.as_slice() {
        [h, m, s] => (num(h)?, num(m)?, num(s)?),
        [m, s] => (0, num(m)?, num(s)?),
        _ => return None,
    };
    if m >= 60 || sec >= 60 || h > 9999 {
        return None;
    }
    let mut ticks = ((h * 60 + m) * 60 + sec) * TICKS_PER_SECOND;
    if !frac.is_empty() {
        if frac.len() > 9 || !frac.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let d: i64 = frac.parse().ok()?;
        ticks += (d as i128 * TICKS_PER_SECOND as i128 / 10i128.pow(frac.len() as u32)) as i64;
    }
    Some(Tick(ticks))
}

/// Format ticks as `HH:MM:SS<sep>mmm` (rounded to the nearest millisecond).
pub fn format_clock(t: Tick, sep: char) -> String {
    // The rounding addition must not overflow for an input-derived Tick near i64::MAX.
    let ms = (i128::from(t.0.max(0)) + i128::from(TICKS_PER_MS) / 2) / i128::from(TICKS_PER_MS);
    let (h, m, s, f) = (ms / 3_600_000, (ms / 60_000) % 60, (ms / 1000) % 60, ms % 1000);
    format!("{h:02}:{m:02}:{s:02}{sep}{f:03}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A start time code keeps its label from a sequence's rate to the SCC / MCC 29.97 grid.
    #[test]
    fn start_timecode_labels_carry_across_rates() {
        let secs = |s: i64| Tick(TICKS_PER_SECOND * s);
        let scc = FrameRate::FPS_29_97;
        // 01:00:00:00 at 25, 24 and 29.97 drop-frame all read as one hour
        assert_eq!(label_time(90_000, FrameRate::FPS_25, false), secs(3600));
        assert_eq!(label_time(86_400, FrameRate::FPS_23_976, false), secs(3600));
        assert_eq!(label_time(107_892, scc, true), secs(3600));
        assert_eq!(label_time(108_000, scc, false), secs(3600));
        // 01:00:02:00 at 25 fps is 01:00:02;00 drop-frame and 01:00:02:00 non-drop at 29.97
        let two = label_time(90_050, FrameRate::FPS_25, false);
        assert_eq!(crate::time::format_timecode_frames(label_frame(two, scc, true), scc, true), "01;00;02;00");
        assert_eq!(crate::time::format_timecode_frames(label_frame(two, scc, false), scc, false), "01:00:02:00");
        // a frame's share of a second goes to the nearest 29.97 label frame
        assert_eq!(label_frame(label_time(90_001, FrameRate::FPS_25, false), scc, false), 108_001);
        // drop-frame on a rate that has none counts non-drop
        assert_eq!(label_time(90_000, FrameRate::FPS_25, true), secs(3600));
        // hostile values neither panic nor go negative
        assert_eq!(label_time(-5, FrameRate::FPS_25, false), Tick::ZERO);
        assert!(label_time(i64::MAX, FrameRate::FPS_25, false) > Tick::ZERO);
        assert_eq!(label_frame(Tick(i64::MIN), scc, true), 0);
        assert!(label_frame(Tick(i64::MAX), scc, true) > 0);
    }

    #[test]
    fn clock() {
        assert_eq!(parse_clock("00:00:01,500"), Some(Tick(TICKS_PER_SECOND * 3 / 2)));
        assert_eq!(parse_clock("01:02.25"), Some(Tick(TICKS_PER_SECOND * 62 + TICKS_PER_SECOND / 4)));
        assert_eq!(parse_clock("100:00:00.000"), Some(Tick(TICKS_PER_SECOND * 360_000)));
        assert_eq!(parse_clock("00:61:00,000"), None);
        assert_eq!(parse_clock("aa:00:00,000"), None);
        assert_eq!(format_clock(Tick(TICKS_PER_SECOND * 3723 + TICKS_PER_MS * 45), ','), "01:02:03,045");
        assert!(format_clock(Tick(i64::MAX), ',').ends_with(",077"));
        assert_eq!(format_clock(Tick(i64::MIN), '.'), "00:00:00.000");
    }

    #[test]
    fn decode() {
        assert_eq!(decode_text(b"\xef\xbb\xbfa\r\nb\rc"), "a\nb\nc");
        assert_eq!(decode_text(&[0xff, 0xfe, b'h', 0, b'i', 0]), "hi");
        assert_eq!(decode_text(b"caf\xe9 \x93x\x94"), "café “x”");
    }

    #[test]
    fn detects() {
        assert_eq!(detect(b"WEBVTT\n\n00:01.000 --> 00:02.000\nhi\n", Some("vtt")), Some(Format::WebVtt));
        assert_eq!(detect(b"1\n00:00:01,000 --> 00:00:02,000\nhi\n", Some("srt")), Some(Format::Srt));
        assert_eq!(detect(b"Scenarist_SCC V1.0\n\n00:00:00;00\t9420\n", Some("scc")), Some(Format::Scc));
        assert_eq!(detect(b"hello", Some("srt")), None);
        assert_eq!(detect(b"<xml/>", None), None);
        assert_eq!(detect(b"File Format=MacCaption_MCC V1.0\n", Some("mcc")), Some(Format::Mcc));
        let ttml = b"<?xml version=\"1.0\"?><tt xmlns=\"http://www.w3.org/ns/ttml\"><body/></tt>";
        assert_eq!(detect(ttml, Some("ttml")), Some(Format::Ttml));
        assert_eq!(detect(ttml, Some("dfxp")), Some(Format::Dfxp));
        assert_eq!(detect(b"<tt xmlns=\"http://www.w3.org/2006/10/ttaf1\"></tt>", Some("xml")), Some(Format::Dfxp));
        let stl = stl::write(&Document::default(), None, PROGRAM);
        assert_eq!(detect(&stl, Some("stl")), Some(Format::Stl));
        for f in Format::ALL {
            assert_eq!(Format::from_name(f.extension()), Some(f));
        }
    }

    /// SCC and MCC exports name each character CEA-608 or CEA-708 drops or replaces.
    #[test]
    fn charset_warnings_name_lost_characters() {
        let rate = FrameRate::FPS_29_97;
        let cue = |s: i64, text: &str| Cue { start: rate.tick_of(s), end: rate.tick_of(s + 60), text: text.into(), ..Default::default() };
        let doc =
            Document { cues: vec![cue(0, "Espere… isso é “demais” — não?"), cue(90, "<i>Don’t</i> stop… ♪")], ..Default::default() };
        let opts = WriteOptions::default();
        let dropped = "CEA-608 has no \"…\" (U+2026): dropped 2 times, first in the caption at 00:00:00;00";
        let replaced = "CEA-608 has no \"’\" (U+2019): written as \"'\" once, first in the caption at 00:00:03;00";
        assert_eq!(charset_warnings(&doc, Format::Scc, opts), [dropped, replaced]);
        // CEA-708 carries … (G2) and ’ but not the em dash, which 608 does carry
        let em_dash = "CEA-708 has no \"—\" (U+2014): written as \"?\" once, first in the caption at 00:00:00;00";
        assert_eq!(charset_warnings(&doc, Format::Mcc, opts), [dropped, replaced, em_dash]);
        // a document that starts at 01:00:00:00 names the time codes its file has
        let master = Document { start_timecode: Tick(3600 * TICKS_PER_SECOND), ..doc.clone() };
        assert!(charset_warnings(&master, Format::Scc, opts)[0].ends_with("first in the caption at 01:00:00;00"));
        let ndf = charset_warnings(&doc, Format::Scc, WriteOptions { drop_frame: false, ..opts });
        assert!(ndf[0].ends_with("00:00:00:00"), "{ndf:?}");
        for f in [Format::Srt, Format::WebVtt, Format::Ttml, Format::Dfxp] {
            assert!(charset_warnings(&doc, f, opts).is_empty(), "{f:?}");
        }
        // what the warnings name is what the files lose
        let scc = scc::parse(&String::from_utf8(write(&doc, Format::Scc, opts)).unwrap()).unwrap();
        assert_eq!(scc.cues[0].text, "Espere isso é “demais” — não?");
        let clean = Document { cues: vec![cue(0, "Café “q” ♪ — ação")], ..Default::default() };
        assert!(charset_warnings(&clean, Format::Scc, opts).is_empty());
    }

    /// A track in a script 608 cannot show gives a bounded list, not one warning per character.
    #[test]
    fn charset_warnings_are_bounded() {
        let text: String = (0x4e00..0x4e00 + 400).filter_map(char::from_u32).collect();
        let doc = Document {
            cues: (0..3)
                .map(|i| Cue { start: Tick(i * TICKS_PER_SECOND), end: Tick((i + 1) * TICKS_PER_SECOND), text: text.clone(), ..Default::default() })
                .collect(),
            ..Default::default()
        };
        let w = charset_warnings(&doc, Format::Scc, WriteOptions::default());
        assert_eq!(w.len(), LISTED_CHARS + 1);
        assert_eq!(w[LISTED_CHARS], "CEA-608 has no code for 380 more characters either: changed or dropped 1140 times");
    }

    #[test]
    fn a_document_keeps_at_most_the_cue_limit() {
        let mut doc = parse(
            b"1\n00:00:01,000 --> 00:00:02,000\na\n\n2\n00:00:03,000 --> 00:00:04,000\nb\n\n3\n00:00:05,000 --> 00:00:06,000\nc\n",
            Format::Srt,
        )
        .unwrap();
        cap_cues(&mut doc, 3);
        assert_eq!((doc.cues.len(), doc.warnings.len()), (3, 0));
        cap_cues(&mut doc, 2);
        assert_eq!(doc.cues.len(), 2);
        assert_eq!(doc.warnings, ["more than 2 cues: the rest were skipped"]);
    }

    /// EBU STL writes `?` for what ISO/IEC 6937 has no code for, and names it with the time code
    /// the STL file gives the caption (25 fps, or 29.97 for a 29.97 or 30 fps sequence).
    #[test]
    fn charset_warnings_name_what_stl_cannot_carry() {
        let at = |s: i64| Tick(TICKS_PER_SECOND * s / 5);
        let cue = |s: i64, text: &str| Cue { start: at(s), end: at(s + 10), text: text.into(), ..Default::default() };
        let doc = Document { cues: vec![cue(6, "Espere… custa 5 €, não?"), cue(20, "<i>Mais €</i> — Ação ♪")], ..Default::default() };
        // (sequence rate, time code of the first caption, of the second)
        for (rate, first, second) in [(None, "00:00:01:05", "00:00:04:00"), (Some(FrameRate::FPS_29_97), "00:00:01:06", "00:00:04:00")] {
            let opts = WriteOptions { rate, ..Default::default() };
            let warnings = charset_warnings(&doc, Format::Stl, opts);
            assert_eq!(
                warnings,
                [
                    format!("EBU STL (ISO/IEC 6937) has no \"…\" (U+2026): written as \"?\" once, first in the caption at {first}"),
                    format!("EBU STL (ISO/IEC 6937) has no \"€\" (U+20AC): written as \"?\" 2 times, first in the caption at {first}"),
                    format!("EBU STL (ISO/IEC 6937) has no \"—\" (U+2014): written as \"?\" once, first in the caption at {second}"),
                ],
                "{rate:?}"
            );
            // what the warnings name is what the file loses
            let back = stl::parse(&write(&doc, Format::Stl, opts)).unwrap();
            assert_eq!(back.cues[0].text, "Espere? custa 5 ?, não?");
        }
        let clean = Document { cues: vec![cue(0, "Ação “já” ♪ ½ ― Øre")], ..Default::default() };
        assert!(charset_warnings(&clean, Format::Stl, WriteOptions::default()).is_empty());
    }

    /// CEA-608 shows 15 rows of 32 characters and the MCC writer's CEA-708 window 12: a longer
    /// caption loses its last rows, and the warning names the caption and exactly what the file
    /// leaves out, short rows included.
    #[test]
    fn charset_warnings_name_captions_cut_to_fit() {
        let visible = |s: &str| s.chars().filter(|c| !c.is_whitespace()).count();
        // rows of exactly 32 characters, numbered so the lost part is visible
        let full = |k: usize| format!("row{k:02}{}", "x".repeat(27));
        let rows = |n: usize, last: &[&str]| (1..=n).map(full).chain(last.iter().map(|r| r.to_string())).collect::<Vec<_>>();
        let doc = |rows: &[String], cues: i64| Document {
            cues: (0..cues)
                .map(|i| Cue {
                    start: Tick((2 * i + 1) * TICKS_PER_SECOND),
                    end: Tick((2 * i + 2) * TICKS_PER_SECOND),
                    text: rows.join("\n"),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let row13 = format!("row13{}…", "x".repeat(19));
        let row16 = format!("row16{}…", "x".repeat(19));
        // (case, rows of text, what SCC loses, what MCC's CEA-708 loses: characters and the start)
        let cases = [
            ("12 full rows", rows(12, &[]), (0, ""), (0, "")),
            ("12 rows, the last short", rows(11, &["end"]), (0, ""), (0, "")),
            ("a short row 12, then one character", rows(11, &["end", "z"]), (0, ""), (1, "z")),
            ("13 full rows", rows(13, &[]), (0, ""), (32, row13.as_str())),
            ("17 full rows", rows(17, &[]), (64, row16.as_str()), (160, row13.as_str())),
        ];
        let cut = |limit: &str, (n, from): (usize, &str)| {
            let what = if n == 1 { "its last character".to_string() } else { format!("its last {n} characters") };
            (n > 0).then(|| format!("{limit}: the caption at 00:00:01;00 lost {what}, from \"{from}\""))
        };
        let opts = WriteOptions::default();
        for (case, rows, scc_lost, dtv_lost) in cases {
            let doc = doc(&rows, 1);
            let c608 = cut("CEA-608 shows 15 rows of 32 characters", scc_lost);
            let c708 = cut("The CEA-708 window holds 12 rows of 32 characters", dtv_lost);
            assert_eq!(charset_warnings(&doc, Format::Scc, opts), Vec::from_iter(c608.clone()), "{case}, SCC");
            assert_eq!(charset_warnings(&doc, Format::Mcc, opts), Vec::from_iter(c608.into_iter().chain(c708)), "{case}, MCC");
            for f in [Format::Srt, Format::WebVtt, Format::Stl, Format::Ttml, Format::Dfxp] {
                assert!(charset_warnings(&doc, f, opts).is_empty(), "{case}, {f:?}");
            }
            // what the warnings count is what the files leave out
            let all = visible(&rows.concat());
            let scc = scc::parse(&String::from_utf8(write(&doc, Format::Scc, opts)).unwrap()).unwrap();
            assert_eq!(all - visible(&scc.cues[0].text), scc_lost.0, "{case}, SCC file");
            let dtv = mcc::parse(&mcc::write_with(&doc, true, false, true, "test")).unwrap();
            assert_eq!(all - visible(&dtv.cues[0].text), dtv_lost.0, "{case}, CEA-708 in the MCC file");
        }
        // many cut captions: a bounded list
        let w = charset_warnings(&doc(&rows(16, &[]), 23), Format::Scc, opts);
        assert_eq!(w.len(), LISTED_CUTS + 1);
        assert_eq!(w[LISTED_CUTS], "CEA-608 shows 15 rows of 32 characters: 3 more captions were cut to fit too, losing 96 characters");
    }

    #[test]
    fn normalize_cuts_overlaps() {
        let c = |s: i64, e: i64| Cue { start: Tick(s), end: Tick(e), text: "x".into(), ..Default::default() };
        let mut d = Document { cues: vec![c(10, 20), c(0, 15), c(10, 12)], ..Default::default() };
        d.normalize();
        let r: Vec<(i64, i64)> = d.cues.iter().map(|c| (c.start.0, c.end.0)).collect();
        assert_eq!(r, vec![(0, 10), (10, 12), (12, 20)]);
    }

    /// `write_to` streams the same bytes `write` returns, for every format and a padded MCC.
    #[test]
    fn write_to_matches_write() {
        let cue = Cue { start: Tick(TICKS_PER_SECOND), end: Tick(2 * TICKS_PER_SECOND), text: "Hello".into(), ..Default::default() };
        let doc = Document { cues: vec![cue], ..Default::default() };
        let padded = WriteOptions { every_frame_until: Some(Tick(3 * TICKS_PER_SECOND)), ..Default::default() };
        for (format, opts) in Format::ALL.map(|f| (f, WriteOptions::default())).into_iter().chain([(Format::Mcc, padded)]) {
            let mut out = Vec::new();
            write_to(&mut out, &doc, format, opts).unwrap();
            assert_eq!(out, write(&doc, format, opts), "{format:?}");
        }
    }
}
