//! Standard MIDI File reading and writing (from the public SMF 1.0 specification).

use std::collections::VecDeque;

use crate::{CtrlEvent, CtrlKind, MidiError, Note, PPQ, Sequence};

/// Maximum number of events (across all tracks) [`read_smf`] will parse.
pub const MAX_EVENTS: usize = 5_000_000;
/// Maximum number of track chunks [`read_smf`] will accept.
pub const MAX_TRACKS: usize = 4096;

/// Release velocity used when a note ends with a note-on of velocity 0 (per the spec, a
/// note-on with velocity 0 is a note-off with velocity 64).
const DEFAULT_RELEASE: u8 = 64;

/// One track of a Standard MIDI File.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmfTrack {
    /// Name from the first Sequence/Track Name meta event (empty if none).
    pub name: String,
    pub sequence: Sequence,
}

/// A Standard MIDI File in model terms. All ticks are at [`PPQ`] (960) per quarter note.
#[derive(Debug, Clone, PartialEq)]
pub struct Smf {
    /// Format of the source file (0, 1 or 2). [`write_smf`] writes format 0 when this is 0
    /// (one track holding everything) and format 1 otherwise.
    pub format: u16,
    pub tracks: Vec<SmfTrack>,
    /// `(tick, beats per minute)`.
    pub tempos: Vec<(i64, f64)>,
    /// `(tick, numerator, denominator)`, e.g. `(0, 6, 8)`.
    pub meters: Vec<(i64, u8, u8)>,
    /// `(tick, text)` from Marker meta events.
    pub markers: Vec<(i64, String)>,
    /// `(tick, sharps (+) / flats (-) -7..=7, is_minor)`.
    pub key_sigs: Vec<(i64, i8, bool)>,
}

/// An empty format-1 file.
impl Default for Smf {
    fn default() -> Self {
        Smf { format: 1, tracks: Vec::new(), tempos: Vec::new(), meters: Vec::new(), markers: Vec::new(), key_sigs: Vec::new() }
    }
}

// ---------------------------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------------------------

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    /// Absolute offset of `data[0]` in the file, for error messages.
    base: usize,
}

impl<'a> Cursor<'a> {
    fn at(&self) -> usize {
        self.base.saturating_add(self.pos)
    }

    fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn u8(&mut self) -> Result<u8, MidiError> {
        let b = *self.data.get(self.pos).ok_or(MidiError::Truncated(self.at()))?;
        self.pos += 1;
        Ok(b)
    }

    fn peek(&self) -> Result<u8, MidiError> {
        self.data.get(self.pos).copied().ok_or(MidiError::Truncated(self.at()))
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], MidiError> {
        let end = self.pos.checked_add(n).ok_or(MidiError::Truncated(self.at()))?;
        let s = self.data.get(self.pos..end).ok_or(MidiError::Truncated(self.at()))?;
        self.pos = end;
        Ok(s)
    }

    fn u16(&mut self) -> Result<u16, MidiError> {
        let b = self.bytes(2)?;
        Ok(u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]))
    }

    fn u32(&mut self) -> Result<u32, MidiError> {
        let b = self.bytes(4)?;
        let mut a = [0u8; 4];
        a.copy_from_slice(b);
        Ok(u32::from_be_bytes(a))
    }

    /// Variable-length quantity: at most 4 bytes (28 bits) per the spec.
    fn vlq(&mut self) -> Result<u32, MidiError> {
        let start = self.at();
        let mut v: u32 = 0;
        for _ in 0..4 {
            let b = self.u8()?;
            v = (v << 7) | u32::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(MidiError::VlqTooLong(start))
    }
}

/// Converts file ticks to model ticks: `model = file * num / den` (rounded).
#[derive(Clone, Copy)]
struct TickConv {
    num: i128,
    den: i128,
}

impl TickConv {
    fn from_division(division: u16, offset: usize) -> Result<Self, MidiError> {
        if division & 0x8000 == 0 {
            if division == 0 {
                return Err(MidiError::Invalid { offset, reason: "division of 0 ticks per quarter note" });
            }
            Ok(Self { num: i128::from(PPQ), den: i128::from(division) })
        } else {
            // SMPTE: high byte is negative frames/sec (two's complement), low byte ticks/frame.
            // Approximated at 120 bpm: one quarter note = 0.5 s, so
            // model = file / ticks_per_sec * 2 * PPQ.
            let [hi, lo] = division.to_be_bytes();
            let fps = -i128::from(hi as i8);
            let tpf = i128::from(lo);
            if !matches!(fps, 24 | 25 | 29 | 30) || tpf == 0 {
                return Err(MidiError::Invalid { offset, reason: "invalid SMPTE division" });
            }
            let two_ppq = 2 * i128::from(PPQ);
            if fps == 29 {
                // 29.97 drop-frame: ticks/sec = 30000/1001 * tpf
                Ok(Self { num: two_ppq * 1001, den: 30000 * tpf })
            } else {
                Ok(Self { num: two_ppq, den: fps * tpf })
            }
        }
    }

    fn convert(&self, file_tick: u64) -> i64 {
        let v = (i128::from(file_tick) * self.num + self.den / 2) / self.den;
        i64::try_from(v).unwrap_or(i64::MAX)
    }
}

struct Globals {
    tempos: Vec<(i64, f64)>,
    meters: Vec<(i64, u8, u8)>,
    markers: Vec<(i64, String)>,
    key_sigs: Vec<(i64, i8, bool)>,
}

/// Parses a Standard MIDI File.
///
/// - Formats 0 and 1 are fully supported; format 2 is read best-effort (all tracks share one
///   timeline). Each `MTrk` chunk becomes one [`SmfTrack`]; for format 1 a leading conductor
///   track without notes or controllers is folded into the global tempo/meter/marker lists.
/// - Ticks are converted to [`PPQ`]; an SMPTE division is approximated at 120 bpm.
/// - Running status, note-on velocity 0 as note-off, unmatched note-offs (ignored), overlapping
///   same-pitch notes (matched first-in first-out) and hanging notes (closed at the end of the
///   track) are handled. Zero-length notes get a length of 1 tick.
/// - Unknown chunks and meta events are skipped. Any malformed structure (bad header, truncated
///   chunk, VLQ longer than 4 bytes, data byte without running status, ...) returns an error.
///   [`read_smf_lenient`] reads such files as far as they make sense instead.
pub fn read_smf(bytes: &[u8]) -> Result<Smf, MidiError> {
    read(bytes, None).map(|(smf, _)| smf)
}

/// The most warnings [`read_smf_lenient`] gives for one file.
pub const MAX_WARNINGS: usize = 32;

/// Reads what can be made sense of in a file that may be damaged, as [`read_smf`] reads a sound
/// one, and says in the returned warnings (at most [`MAX_WARNINGS`]) what could not be read.
///
/// Only a file that is not a Standard MIDI File (no `MThd`, a header cut short, a time division
/// no tick can be counted by) is an error. Otherwise:
///
/// - a header length beyond the file is ignored, and an unknown format is read as format 1;
/// - a track whose events stop making sense (a cut, a data byte without a status, a system
///   message, more events than [`MAX_EVENTS`] in all) keeps the events before that point, and its
///   open notes end there;
/// - a track length beyond the file reads the rest of the file; a length that is too long or too
///   short is not believed when the track's End of Track says otherwise: the next chunk is looked
///   for where the track's events ended;
/// - bytes that are no chunk are skipped up to the next `MTrk`, and trailing bytes are ignored;
/// - past [`MAX_TRACKS`] tracks the rest is left out;
/// - lyrics are counted in a warning (they are not kept); a format-2 file gets a warning that its
///   tracks are read side by side.
pub fn read_smf_lenient(bytes: &[u8]) -> Result<LenientRead, MidiError> {
    let mut warnings = Vec::new();
    let (smf, extra) = read(bytes, Some(&mut warnings))?;
    Ok(LenientRead { smf, warnings, instruments: extra.instruments, end: extra.end })
}

/// What [`read_smf_lenient`] makes of a file.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct LenientRead {
    pub smf: Smf,
    /// What could not be read, in words; empty for a sound file.
    pub warnings: Vec<String>,
    /// The Instrument Name meta event of each of `smf.tracks` (empty where there is none).
    pub instruments: Vec<String>,
    /// The last tick any track reaches: its End of Track, or where its events stop.
    pub end: i64,
}

/// What the reader finds beyond [`Smf`]: instrument names per kept track, and the end tick.
struct Extra {
    instruments: Vec<String>,
    end: i64,
}

fn warn(warnings: &mut Option<&mut Vec<String>>, what: String) {
    if let Some(w) = warnings.as_deref_mut()
        && w.len() < MAX_WARNINGS
    {
        w.push(what);
    }
}

/// Strict (`warnings` is `None`): any damage is an error. Lenient: damage is a warning.
fn read(bytes: &[u8], mut warnings: Option<&mut Vec<String>>) -> Result<(Smf, Extra), MidiError> {
    let lenient = warnings.is_some();
    let mut cur = Cursor { data: bytes, pos: 0, base: 0 };
    if cur.bytes(4).map_err(|_| MidiError::NotSmf)? != b"MThd" {
        return Err(MidiError::NotSmf);
    }
    let header_len = cur.u32()?;
    if header_len < 6 {
        return Err(MidiError::Invalid { offset: 4, reason: "header chunk shorter than 6 bytes" });
    }
    let header_len = usize::try_from(header_len).map_err(|_| MidiError::BadChunkLength { offset: 4, len: u64::from(header_len) })?;
    let header = match cur.bytes(header_len) {
        Ok(header) => header,
        Err(_) if lenient => {
            warn(&mut warnings, "the header gives a length the file does not have".into());
            cur.bytes(6)?
        }
        Err(_) => return Err(MidiError::BadChunkLength { offset: 4, len: u64::try_from(header_len).unwrap_or(u64::MAX) }),
    };
    let mut hc = Cursor { data: header, pos: 0, base: 8 };
    let mut format = hc.u16()?;
    let _ntrks = hc.u16()?;
    let division = hc.u16()?;
    if format > 2 {
        if !lenient {
            return Err(MidiError::Unsupported("SMF format greater than 2"));
        }
        warn(&mut warnings, format!("format {format} is unknown: read as tracks that play together"));
        format = 1;
    }
    let conv = TickConv::from_division(division, 12)?;

    let mut globals = Globals { tempos: Vec::new(), meters: Vec::new(), markers: Vec::new(), key_sigs: Vec::new() };
    let (mut tracks, mut instruments) = (Vec::new(), Vec::new());
    let mut event_budget = MAX_EVENTS;
    let (mut end, mut seen, mut lyrics) = (0i64, 0usize, 0usize);

    while !cur.is_empty() {
        let chunk_at = cur.at();
        let head = cur.bytes(4).and_then(|id| Ok((id, cur.u32()?)));
        let (id, len) = match head {
            Ok(head) => head,
            Err(_) if lenient => {
                warn(&mut warnings, format!("{} bytes after the last chunk were ignored", bytes.len() - chunk_at));
                break;
            }
            Err(e) => return Err(e),
        };
        let remaining = bytes.len().saturating_sub(cur.pos);
        let len_usize = usize::try_from(len).unwrap_or(usize::MAX);
        let fits = len_usize <= remaining;
        if !fits && !lenient {
            return Err(MidiError::BadChunkLength { offset: chunk_at, len: u64::from(len) });
        }
        if id != b"MTrk" {
            // Unknown chunk types must be ignored. Leniently, only what looks like a chunk is one.
            if fits && (!lenient || id.iter().all(|b| b.is_ascii_graphic() || *b == b' ')) {
                cur.pos += len_usize;
                continue;
            }
            // No chunk at all: the next track, if there is one, is looked for.
            let after = bytes.get(chunk_at + 1..).unwrap_or_default();
            match after.windows(4).position(|w| w == b"MTrk") {
                Some(skip) => {
                    warn(&mut warnings, format!("{} bytes that are no chunk were skipped", skip + 1));
                    cur.pos = chunk_at + 1 + skip;
                    continue;
                }
                None => {
                    warn(&mut warnings, format!("{} bytes after the last chunk were ignored", bytes.len() - chunk_at));
                    break;
                }
            }
        }
        seen += 1;
        if tracks.len() >= MAX_TRACKS {
            if !lenient {
                return Err(MidiError::TooManyTracks(MAX_TRACKS));
            }
            warn(&mut warnings, format!("more than {MAX_TRACKS} tracks: the rest was left out"));
            break;
        }
        let (body_at, body_start) = (cur.at(), cur.pos);
        let body = if fits { cur.bytes(len_usize)? } else { bytes.get(cur.pos..).unwrap_or_default() };
        if !fits {
            warn(&mut warnings, format!("track {seen} is shorter than its length says"));
            cur.pos = bytes.len();
        }
        let read = read_track(body, body_at, conv, &mut globals, &mut event_budget);
        lyrics += read.lyrics;
        end = end.max(read.end);
        match read.stop {
            Some(e) if !lenient => return Err(e),
            Some(e) => warn(&mut warnings, format!("track {seen} {}: what came before is kept", stop_reason(&e))),
            None if !read.ended && lenient => warn(&mut warnings, format!("track {seen} has no End of Track: what came before is kept")),
            None => {}
        }
        // A length that reaches past the track's end into what follows is not believed: the next
        // chunk is looked for where the track's events ended.
        if lenient && read.ended && read.used < body.len() {
            let next = bytes.get(cur.pos..).unwrap_or_default();
            let believed = fits && (next.is_empty() || next.starts_with(b"MTrk"));
            if !believed {
                cur.pos = body_start + read.used;
                if fits {
                    warn(&mut warnings, format!("track {seen} ends before its length says"));
                }
            }
        }
        tracks.push(read.track);
        instruments.push(read.instrument);
    }
    if lyrics > 0 {
        warn(&mut warnings, format!("{lyrics} lyrics were left out"));
    }
    if format == 2 {
        warn(&mut warnings, "format 2: each track is a song of its own; they are read side by side".into());
    }

    // A lone track is only a conductor track if it carried global data and has no name; otherwise it is kept.
    let globals_present = !(globals.tempos.is_empty() && globals.meters.is_empty() && globals.markers.is_empty() && globals.key_sigs.is_empty());
    if format == 1
        && (tracks.len() > 1 || globals_present)
        && tracks.first().is_some_and(|t| t.sequence.notes.is_empty() && t.sequence.ctrls.is_empty() && (tracks.len() > 1 || t.name.is_empty()))
    {
        tracks.remove(0);
        instruments.remove(0);
    }

    // Keep global lists ordered by tick (stable: file order within a tick).
    globals.tempos.sort_by_key(|t| t.0);
    globals.meters.sort_by_key(|t| t.0);
    globals.markers.sort_by_key(|t| t.0);
    globals.key_sigs.sort_by_key(|t| t.0);

    Ok((
        Smf { format, tracks, tempos: globals.tempos, meters: globals.meters, markers: globals.markers, key_sigs: globals.key_sigs },
        Extra { instruments, end },
    ))
}

/// Why a track's events stopped, in words for a warning.
fn stop_reason(e: &MidiError) -> &'static str {
    match e {
        MidiError::TooManyEvents(_) => "holds more events than are read: the rest was left out",
        MidiError::Invalid { reason, .. } if reason.starts_with("data byte") || reason.starts_with("system") || reason.starts_with("status") => {
            "holds a byte that is no event"
        }
        _ => "ends in the middle of an event",
    }
}

/// Open note-ons for one (channel, pitch), oldest first: `(model tick, velocity)`.
type OpenNotes = Vec<VecDeque<(i64, u8)>>;

fn close_note(open: &mut OpenNotes, notes: &mut Vec<Note>, channel: u8, pitch: u8, tick: i64, release: u8) {
    let key = usize::from(channel & 0x0F) * 128 + usize::from(pitch & 0x7F);
    let Some(stack) = open.get_mut(key) else { return };
    if stack.is_empty() {
        return; // unmatched note-off: ignored
    }
    let Some((start, velocity)) = stack.pop_front() else { return };
    let length = tick.saturating_sub(start).max(1);
    notes.push(Note { pitch, velocity, release_velocity: release, channel, start, length });
}

/// What [`read_track`] made of a track chunk: the track (its notes closed), and how its events ended.
struct TrackRead {
    track: SmfTrack,
    instrument: String,
    /// The tick the track's events reach (its End of Track, or where they stopped).
    end: i64,
    /// Whether an End of Track ended it, and the bytes up to and including it.
    ended: bool,
    used: usize,
    /// Why its events stopped making sense, if they did.
    stop: Option<MidiError>,
    lyrics: usize,
}

/// The state of a track being read.
struct TrackState {
    name: Option<String>,
    instrument: Option<String>,
    seq: Sequence,
    open: OpenNotes,
    tick: i64,
    lyrics: usize,
}

fn read_track(body: &[u8], base: usize, conv: TickConv, g: &mut Globals, budget: &mut usize) -> TrackRead {
    let mut cur = Cursor { data: body, pos: 0, base };
    let mut st = TrackState { name: None, instrument: None, seq: Sequence::default(), open: vec![VecDeque::new(); 16 * 128], tick: 0, lyrics: 0 };
    let result = track_events(&mut cur, conv, g, budget, &mut st);
    let TrackState { name, instrument, mut seq, mut open, tick, lyrics } = st;
    // Close hanging notes at the end of the track (or where its events stopped).
    for (key, stack) in open.iter_mut().enumerate() {
        let channel = u8::try_from(key / 128).unwrap_or(0);
        let pitch = u8::try_from(key % 128).unwrap_or(0);
        for (start, velocity) in stack.drain(..) {
            let length = tick.saturating_sub(start).max(1);
            seq.notes.push(Note { pitch, velocity, release_velocity: DEFAULT_RELEASE, channel, start, length });
        }
    }
    seq.sort();
    let track = SmfTrack { name: name.unwrap_or_default(), sequence: seq };
    let (ended, stop) = match result {
        Ok(ended) => (ended, None),
        Err(e) => (false, Some(e)),
    };
    TrackRead { track, instrument: instrument.unwrap_or_default(), end: tick, ended, used: cur.pos, stop, lyrics }
}

/// Reads a track's events; `Ok(true)` when an End of Track ends them, `Ok(false)` when the data does.
fn track_events(cur: &mut Cursor, conv: TickConv, g: &mut Globals, budget: &mut usize, st: &mut TrackState) -> Result<bool, MidiError> {
    let mut file_tick: u64 = 0;
    let mut running: Option<u8> = None;

    while !cur.is_empty() {
        if *budget == 0 {
            return Err(MidiError::TooManyEvents(MAX_EVENTS));
        }
        *budget -= 1;

        let delta = cur.vlq()?;
        file_tick = file_tick.saturating_add(u64::from(delta));
        st.tick = conv.convert(file_tick);
        let tick = st.tick;

        let event_at = cur.at();
        let first = cur.peek()?;
        let status = if first & 0x80 != 0 {
            cur.pos += 1;
            first
        } else {
            running.ok_or(MidiError::Invalid { offset: event_at, reason: "data byte without running status" })?
        };

        match status {
            0xFF => {
                running = None;
                let kind = cur.u8()?;
                let len = cur.vlq()?;
                let data = cur.bytes(usize::try_from(len).unwrap_or(usize::MAX))?;
                match kind {
                    0x2F => return Ok(true), // End of Track
                    0x03 if st.name.is_none() => {
                        st.name = Some(String::from_utf8_lossy(data).into_owned());
                    }
                    0x04 if st.instrument.is_none() => {
                        st.instrument = Some(String::from_utf8_lossy(data).into_owned());
                    }
                    0x05 => st.lyrics += 1,
                    0x06 => g.markers.push((tick, String::from_utf8_lossy(data).into_owned())),
                    0x51 => {
                        if let [a, b, c] = data {
                            let us = (u32::from(*a) << 16) | (u32::from(*b) << 8) | u32::from(*c);
                            if us > 0 {
                                g.tempos.push((tick, 60_000_000.0 / f64::from(us)));
                            }
                        }
                    }
                    0x58 => {
                        if let [nn, dd, ..] = data
                            && *nn > 0
                            && *dd <= 7
                        {
                            g.meters.push((tick, *nn, 1u8 << *dd));
                        }
                    }
                    0x59 => {
                        if let [sf, mi, ..] = data {
                            let sf = *sf as i8;
                            if (-7..=7).contains(&sf) {
                                g.key_sigs.push((tick, sf, *mi == 1));
                            }
                        }
                    }
                    _ => {}
                }
            }
            0xF0 | 0xF7 => {
                running = None;
                let len = cur.vlq()?;
                cur.bytes(usize::try_from(len).unwrap_or(usize::MAX))?;
            }
            0x80..=0xEF => {
                running = Some(status);
                let channel = status & 0x0F;
                let d1 = cur.u8()?;
                if d1 & 0x80 != 0 {
                    return Err(MidiError::Invalid { offset: event_at, reason: "status byte where a data byte was expected" });
                }
                let needs_two = !matches!(status & 0xF0, 0xC0 | 0xD0);
                let d2 = if needs_two {
                    let d = cur.u8()?;
                    if d & 0x80 != 0 {
                        return Err(MidiError::Invalid { offset: event_at, reason: "status byte where a data byte was expected" });
                    }
                    d
                } else {
                    0
                };
                let seq = &mut st.seq;
                match status & 0xF0 {
                    0x80 => close_note(&mut st.open, &mut seq.notes, channel, d1, tick, d2),
                    0x90 if d2 == 0 => close_note(&mut st.open, &mut seq.notes, channel, d1, tick, DEFAULT_RELEASE),
                    0x90 => {
                        if let Some(stack) = st.open.get_mut(usize::from(channel) * 128 + usize::from(d1)) {
                            stack.push_back((tick, d2));
                        }
                    }
                    0xA0 => seq.ctrls.push(CtrlEvent { tick, channel, kind: CtrlKind::PolyPressure(d1), value: i32::from(d2) }),
                    0xB0 => seq.ctrls.push(CtrlEvent { tick, channel, kind: CtrlKind::Cc(d1), value: i32::from(d2) }),
                    0xC0 => seq.ctrls.push(CtrlEvent { tick, channel, kind: CtrlKind::Program, value: i32::from(d1) }),
                    0xD0 => seq.ctrls.push(CtrlEvent { tick, channel, kind: CtrlKind::ChannelPressure, value: i32::from(d1) }),
                    _ => {
                        // 0xE0 pitch bend: LSB then MSB, centred on 8192.
                        let v = (i32::from(d2) << 7 | i32::from(d1)) - 8192;
                        seq.ctrls.push(CtrlEvent { tick, channel, kind: CtrlKind::PitchBend, value: v });
                    }
                }
            }
            _ => return Err(MidiError::Invalid { offset: event_at, reason: "system real-time/common status in a track" }),
        }
    }
    Ok(false)
}

// ---------------------------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------------------------

const MAX_VLQ: u64 = 0x0FFF_FFFF;

fn push_vlq(out: &mut Vec<u8>, v: u64) -> Result<(), MidiError> {
    if v > MAX_VLQ {
        return Err(MidiError::Write(format!("value {v} does not fit in a MIDI variable-length quantity")));
    }
    let mut buf = [0u8; 4];
    let mut n = 0;
    let mut x = v;
    loop {
        if let Some(slot) = buf.get_mut(n) {
            *slot = (x & 0x7F) as u8;
        }
        n += 1;
        x >>= 7;
        if x == 0 || n == 4 {
            break;
        }
    }
    for i in (0..n).rev() {
        let b = buf.get(i).copied().unwrap_or(0);
        out.push(if i > 0 { b | 0x80 } else { b });
    }
    Ok(())
}

/// An event ready to be serialised: `(tick, order, bytes)`. `order` breaks ties at one tick.
struct WEvent {
    tick: u64,
    order: u8,
    bytes: Vec<u8>,
}

fn clamp_tick(t: i64) -> u64 {
    u64::try_from(t.max(0)).unwrap_or(0)
}

fn meta(kind: u8, data: &[u8]) -> Result<Vec<u8>, MidiError> {
    let mut b = vec![0xFF, kind];
    push_vlq(&mut b, u64::try_from(data.len()).unwrap_or(u64::MAX))?;
    b.extend_from_slice(data);
    Ok(b)
}

fn write_track(out: &mut Vec<u8>, mut events: Vec<WEvent>) -> Result<(), MidiError> {
    events.sort_by_key(|e| (e.tick, e.order));
    let mut body = Vec::new();
    let mut last_tick = 0u64;
    let mut running: Option<u8> = None;
    for e in &events {
        push_vlq(&mut body, e.tick - last_tick)?;
        last_tick = e.tick;
        let status = e.bytes.first().copied().unwrap_or(0);
        if (0x80..=0xEF).contains(&status) {
            if running == Some(status) {
                body.extend_from_slice(e.bytes.get(1..).unwrap_or(&[]));
            } else {
                body.extend_from_slice(&e.bytes);
            }
            running = Some(status);
        } else {
            running = None;
            body.extend_from_slice(&e.bytes);
        }
    }
    // End of Track.
    body.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
    let len = u32::try_from(body.len()).map_err(|_| MidiError::Write("track larger than 4 GiB".into()))?;
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&body);
    Ok(())
}

/// The conductor track: tempos, meters, key signatures and markers.
fn conductor_events(smf: &Smf) -> Result<Vec<WEvent>, MidiError> {
    let mut ev = Vec::new();
    for &(t, bpm) in &smf.tempos {
        if !bpm.is_finite() || bpm <= 0.0 {
            return Err(MidiError::Write(format!("invalid tempo {bpm} bpm")));
        }
        let us = (60_000_000.0 / bpm).round().clamp(1.0, f64::from(0x00FF_FFFFu32)) as u32;
        let [_, a, b, c] = us.to_be_bytes();
        ev.push(WEvent { tick: clamp_tick(t), order: 0, bytes: meta(0x51, &[a, b, c])? });
    }
    for &(t, num, den) in &smf.meters {
        if den == 0 || !den.is_power_of_two() {
            return Err(MidiError::Write(format!("meter denominator {den} is not a power of two")));
        }
        let dd = den.trailing_zeros() as u8;
        ev.push(WEvent { tick: clamp_tick(t), order: 1, bytes: meta(0x58, &[num, dd, 24, 8])? });
    }
    for &(t, sf, minor) in &smf.key_sigs {
        let sf = sf.clamp(-7, 7);
        ev.push(WEvent { tick: clamp_tick(t), order: 2, bytes: meta(0x59, &[sf as u8, u8::from(minor)])? });
    }
    for (t, text) in &smf.markers {
        ev.push(WEvent { tick: clamp_tick(*t), order: 3, bytes: meta(0x06, text.as_bytes())? });
    }
    Ok(ev)
}

/// Trim notes still sounding when the next same-pitch note starts, and drop duplicates.
fn trim_overlaps(notes: &[Note], ends: &mut [u64], dropped: &mut [bool]) {
    // The reader pairs offs with ons first-in first-out, so a note still sounding when the next
    // same-pitch note starts would swap ends with it: trim it to that start. When two same-pitch
    // notes start on the same tick they cannot be told apart, so only the longer one is written.
    let mut order: Vec<usize> = (0..notes.len()).collect();
    order.sort_by_key(|&i| {
        notes.get(i).map_or((0, 0, 0, std::cmp::Reverse(0)), |n| {
            (n.channel & 0x0F, n.pitch.min(127), clamp_tick(n.start), std::cmp::Reverse(ends.get(i).copied().unwrap_or(0)))
        })
    });
    let mut prev: Option<usize> = None;
    for &b in &order {
        let Some(nb) = notes.get(b) else { continue };
        if let Some(a) = prev
            && let Some(na) = notes.get(a)
            && na.channel & 0x0F == nb.channel & 0x0F
            && na.pitch.min(127) == nb.pitch.min(127)
        {
            let (sa, sb) = (clamp_tick(na.start), clamp_tick(nb.start));
            if sa == sb {
                if let Some(d) = dropped.get_mut(b) {
                    *d = true;
                }
                continue;
            }
            if let Some(ea) = ends.get_mut(a)
                && sb < *ea
            {
                *ea = sb;
            }
        }
        prev = Some(b);
    }
}

/// Writes a format-1 Standard MIDI File at [`PPQ`] (960) ticks per quarter note, or format 0
/// when [`Smf::format`] is 0.
///
/// Track 1 is a conductor track holding tempos, meters, key signatures and markers; every
/// [`SmfTrack`] follows as its own track, with its name when it is not empty. Format 0 is one track holding all of that. Negative ticks are clamped to 0, data values are
/// clamped to their MIDI ranges, and note-on velocity 0 is written as 1 (0 would mean
/// note-off). At each tick note-offs are written before controllers, then note-ons.
///
/// Errors: invalid tempo (non-finite or <= 0 bpm), meter denominator that is not a power of two
/// (1..=128), more than 65535 tracks, or a gap between events beyond the 28-bit delta limit.
pub fn write_smf(smf: &Smf) -> Result<Vec<u8>, MidiError> {
    // The conductor track is omitted when there is nothing to put in it and no content tracks, so an
    // empty `Smf` is written as a file with zero tracks and reads back as zero tracks.
    let has_conductor =
        !(smf.tempos.is_empty() && smf.meters.is_empty() && smf.key_sigs.is_empty() && smf.markers.is_empty() && smf.tracks.is_empty());
    let ntrks =
        u16::try_from(smf.tracks.len().saturating_add(usize::from(has_conductor))).map_err(|_| MidiError::Write("more than 65535 tracks".into()))?;
    let mut out = Vec::new();
    out.extend_from_slice(b"MThd");
    out.extend_from_slice(&6u32.to_be_bytes());
    let single = smf.format == 0;
    out.extend_from_slice(&u16::from(!single).to_be_bytes());
    out.extend_from_slice(&(if single { 1 } else { ntrks }).to_be_bytes());
    out.extend_from_slice(&u16::try_from(PPQ).unwrap_or(960).to_be_bytes());

    // Conductor track; in format 0 the start of the one track.
    let mut merged = conductor_events(smf)?;
    if has_conductor && !single {
        write_track(&mut out, std::mem::take(&mut merged))?;
    }

    for track in &smf.tracks {
        let mut ev = Vec::new();
        if !track.name.is_empty() {
            ev.push(WEvent { tick: 0, order: 0, bytes: meta(0x03, track.name.as_bytes())? });
        }
        let notes = &track.sequence.notes;
        let mut ends: Vec<u64> = notes.iter().map(|n| clamp_tick(n.end()).max(clamp_tick(n.start).saturating_add(1))).collect();
        let mut dropped = vec![false; notes.len()];
        trim_overlaps(notes, &mut ends, &mut dropped);
        for (i, n) in notes.iter().enumerate() {
            if dropped.get(i).copied().unwrap_or(false) {
                continue;
            }
            let ch = n.channel & 0x0F;
            let pitch = n.pitch.min(127);
            let start = clamp_tick(n.start);
            let end = ends.get(i).copied().unwrap_or(start.saturating_add(1));
            ev.push(WEvent { tick: start, order: 3, bytes: vec![0x90 | ch, pitch, n.velocity.clamp(1, 127)] });
            ev.push(WEvent { tick: end, order: 1, bytes: vec![0x80 | ch, pitch, n.release_velocity.min(127)] });
        }
        for c in &track.sequence.ctrls {
            let ch = c.channel & 0x0F;
            let v7 = c.value.clamp(0, 127) as u8;
            let bytes = match c.kind {
                CtrlKind::Cc(num) => vec![0xB0 | ch, num.min(127), v7],
                CtrlKind::Program => vec![0xC0 | ch, v7],
                CtrlKind::ChannelPressure => vec![0xD0 | ch, v7],
                CtrlKind::PolyPressure(p) => vec![0xA0 | ch, p.min(127), v7],
                CtrlKind::PitchBend => {
                    let v = (c.value.saturating_add(8192)).clamp(0, 16383);
                    vec![0xE0 | ch, (v & 0x7F) as u8, ((v >> 7) & 0x7F) as u8]
                }
            };
            ev.push(WEvent { tick: clamp_tick(c.tick), order: 2, bytes });
        }
        if single {
            merged.extend(ev);
        } else {
            write_track(&mut out, ev)?;
        }
    }
    if single {
        write_track(&mut out, merged)?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vlq_round_trip() {
        for v in [0u64, 0x40, 0x7F, 0x80, 0x2000, 0x3FFF, 0x4000, 0x1F_FFFF, 0x20_0000, MAX_VLQ] {
            let mut b = Vec::new();
            push_vlq(&mut b, v).unwrap();
            let mut c = Cursor { data: &b, pos: 0, base: 0 };
            assert_eq!(u64::from(c.vlq().unwrap()), v);
            assert!(c.is_empty());
        }
        assert!(push_vlq(&mut Vec::new(), MAX_VLQ + 1).is_err());
    }

    #[test]
    fn empty_smf_round_trips_with_no_tracks() {
        let r = read_smf(&write_smf(&Smf::default()).unwrap()).unwrap();
        assert_eq!(r.tracks.len(), 0);
        let one = Smf { tracks: vec![SmfTrack::default()], ..Smf::default() };
        let r = read_smf(&write_smf(&one).unwrap()).unwrap();
        assert_eq!(r.tracks.len(), 1);
        let cond = Smf { tempos: vec![(0, 90.0)], ..Smf::default() };
        let r = read_smf(&write_smf(&cond).unwrap()).unwrap();
        assert_eq!(r.tracks.len(), 0);
        assert_eq!(r.tempos.len(), 1);
    }

    #[test]
    fn spec_vlq_examples() {
        let mut b = Vec::new();
        push_vlq(&mut b, 0x0800_0000).unwrap();
        assert_eq!(b, [0xC0, 0x80, 0x80, 0x00]);
        let mut b = Vec::new();
        push_vlq(&mut b, 0x2000).unwrap();
        assert_eq!(b, [0xC0, 0x00]);
    }

    #[test]
    fn same_pitch_overlaps_do_not_swap_ends() {
        let mk = |start, length| Note { pitch: 60, velocity: 100, release_velocity: 64, channel: 0, start, length };
        let smf = Smf {
            format: 1,
            tracks: vec![SmfTrack {
                name: String::new(),
                sequence: Sequence { notes: vec![mk(0, 1000), mk(100, 200), mk(1000, 100), mk(2000, 500), mk(2000, 900)], ..Sequence::default() },
            }],
            tempos: vec![],
            meters: vec![],
            markers: vec![],
            key_sigs: vec![],
        };
        let back = read_smf(&write_smf(&smf).unwrap()).unwrap();
        let mut got: Vec<(i64, i64)> = back.tracks.iter().flat_map(|t| t.sequence.notes.iter().map(|n| (n.start, n.length))).collect();
        got.sort_unstable();
        // The earlier note is trimmed to the start of the next same-pitch note; later notes keep their length;
        // of two notes starting together only the longer survives.
        assert_eq!(got, vec![(0, 100), (100, 200), (1000, 100), (2000, 900)]);
    }
}
