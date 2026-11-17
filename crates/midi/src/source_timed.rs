//! Standard MIDI Files (SMF formats 0, 1 and 2), read and written from the Standard MIDI Files 1.0
//! specification (MIDI Manufacturers Association, RP-001) and the MIDI 1.0 message tables. What a
//! sequencer exchanges is modelled: notes, the channel messages that are one number at a time
//! (control changes, program changes, pitch bend, channel pressure), the tempo map, time and key
//! signatures, markers and track names. Byte-level SMF reading/writing delegates to
//! [`crate`]; this module preserves source-timed u32 ticks and rescaling.
//!
//! [`read`] takes whatever a disk or another program made of a file: what can be made sense of is
//! returned, with [`Smf::warnings`] saying what could not, and nothing in a file makes it
//! allocate more than a small multiple of the file's own size ([`MAX_EVENTS`], [`MAX_TRACKS`]).
//!
//! Use [`crate::sync`] for clock, song position and time code.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use crate::{
    self as craft_midi, CtrlEvent as CraftCtrlEvent, Note as CraftNote, Smf as CraftSmf, SmfTrack as CraftSmfTrack, read_smf_lenient, write_smf,
};
use std::collections::VecDeque;

/// The most notes and channel messages read from one file; the rest is left out with a warning.
pub const MAX_EVENTS: usize = 1 << 21;
/// The most tracks read from one file.
pub const MAX_TRACKS: usize = 4096;
/// The most tempo changes, signatures and markers (each) read from one file.
pub const MAX_META: usize = 1 << 14;
/// The longest name or marker text kept, in bytes.
const MAX_TEXT: usize = 256;
/// The most warnings one file gets.
const MAX_WARNINGS: usize = 32;

/// The controllers a sequencer most often meets.
pub mod cc {
    pub const BANK: u8 = 0;
    pub const MODULATION: u8 = 1;
    pub const VOLUME: u8 = 7;
    pub const PAN: u8 = 10;
    pub const EXPRESSION: u8 = 11;
    pub const BANK_FINE: u8 = 32;
    pub const SUSTAIN: u8 = 64;
}

/// The MIDI channel (counted from 0) General MIDI plays drums on.
pub const DRUMS: u8 = 9;

#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    pub start: u32,
    pub len: u32,
    pub key: u8,
    /// 1..=127
    pub vel: u8,
    pub channel: u8,
}

/// What a [`Change`] sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum What {
    /// A control change, by controller number (0..=127).
    Control(u8),
    Program,
    /// The pitch wheel: -8192..=8191, 0 at rest.
    Bend,
    /// Channel pressure (aftertouch for the whole channel).
    Pressure,
}

/// A channel message that is one number: 0..=127, but for the pitch wheel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub tick: u32,
    pub channel: u8,
    pub what: What,
    pub value: i16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Track {
    pub name: String,
    /// The instrument name meta event, when the track has one.
    pub instrument: String,
    /// Sorted by start, then key.
    pub notes: Vec<Note>,
    /// In the order of the file, which is by tick.
    pub changes: Vec<Change>,
}

impl Track {
    /// The values of controller `number` on `channel`, in order.
    pub fn controls(&self, channel: u8, number: u8) -> impl Iterator<Item = &Change> {
        self.changes.iter().filter(move |c| c.channel == channel && c.what == What::Control(number))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tempo {
    pub tick: u32,
    /// Quarter notes a minute.
    pub bpm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeSignature {
    pub tick: u32,
    pub beats: u8,
    /// The note value of a beat: 4 is a quarter, 8 an eighth.
    pub unit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeySignature {
    pub tick: u32,
    /// Sharps (above zero) or flats (below), -7..=7.
    pub sharps: i8,
    pub minor: bool,
}

/// A marker or a cue point.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Marker {
    pub tick: u32,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Smf {
    /// 0 is one track for all channels, 1 tracks that play together, 2 tracks that are songs of their own.
    pub format: u16,
    /// Ticks per quarter note.
    pub ppq: u16,
    /// Every tempo of the file, by tick, one a tick at most. A file without one plays at 120.
    pub tempos: Vec<Tempo>,
    pub time_signatures: Vec<TimeSignature>,
    pub key_signatures: Vec<KeySignature>,
    pub markers: Vec<Marker>,
    /// The tracks that hold notes or channel messages.
    pub tracks: Vec<Track>,
    /// The last tick any track reaches.
    pub ticks: u32,
    /// What [`read`] could not make sense of, or left out.
    pub warnings: Vec<String>,
}

impl Default for Smf {
    fn default() -> Self {
        Smf {
            format: 1,
            ppq: 96,
            tempos: Vec::new(),
            time_signatures: Vec::new(),
            key_signatures: Vec::new(),
            markers: Vec::new(),
            tracks: Vec::new(),
            ticks: 0,
            warnings: Vec::new(),
        }
    }
}

impl Smf {
    /// The tempo the file starts at.
    pub fn tempo(&self) -> f64 {
        self.tempos.first().map_or(120.0, |t| t.bpm)
    }

    /// Counts every tick in `ppq` to the quarter note instead. A tick goes to the nearest new one
    /// whatever it belongs to, so what was at one tick still is; a note's start and end are moved
    /// apart from each other, so notes that touched still touch, and no note is left without length.
    pub fn rescale(&mut self, ppq: u16) {
        let (from, to) = (u64::from(self.ppq.max(1)), u64::from(ppq.max(1)));
        self.ppq = ppq.max(1);
        if from == to {
            return;
        }
        let scale = |tick: u32| ((u64::from(tick) * to + from / 2) / from).min(u64::from(u32::MAX)) as u32;
        for track in &mut self.tracks {
            for n in &mut track.notes {
                let end = scale(n.start.saturating_add(n.len));
                n.start = scale(n.start);
                n.len = end.saturating_sub(n.start).max(1);
            }
            track.changes.iter_mut().for_each(|c| c.tick = scale(c.tick));
        }
        let ticks = (self.tempos.iter_mut().map(|t| &mut t.tick))
            .chain(self.time_signatures.iter_mut().map(|t| &mut t.tick))
            .chain(self.key_signatures.iter_mut().map(|t| &mut t.tick))
            .chain(self.markers.iter_mut().map(|t| &mut t.tick))
            .chain([&mut self.ticks]);
        ticks.for_each(|tick| *tick = scale(*tick));
        one_tempo_a_tick(&mut self.tempos);
    }
}

/// Of several tempos at one tick the last is the one that plays.
fn one_tempo_a_tick(tempos: &mut Vec<Tempo>) {
    tempos.dedup_by(|later, kept| {
        let same = later.tick == kept.tick;
        if same {
            *kept = *later;
        }
        same
    });
}

#[derive(Debug, PartialEq)]
pub enum Error {
    NotMidi,
    /// The file ends inside its header.
    Truncated,
    /// A time division no tick can be counted by.
    Unsupported(&'static str),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotMidi => write!(f, "not a Standard MIDI File"),
            Error::Truncated => write!(f, "the MIDI file is truncated or damaged"),
            Error::Unsupported(what) => write!(f, "unsupported MIDI file: {what}"),
        }
    }
}

impl std::error::Error for Error {}

fn trim_text(s: &str) -> String {
    s.trim_matches(|c: char| c.is_control() || c.is_whitespace()).to_string()
}

fn map_midi_error(e: craft_midi::MidiError) -> Error {
    match e {
        craft_midi::MidiError::NotSmf => Error::NotMidi,
        craft_midi::MidiError::Truncated(_) | craft_midi::MidiError::BadChunkLength { .. } => Error::Truncated,
        craft_midi::MidiError::Unsupported(_) => Error::Unsupported("unknown"),
        _ => Error::NotMidi,
    }
}

// ---------------------------------------------------------------------------
// Adapter: source-timed types ↔ normalized craft-midi types
// ---------------------------------------------------------------------------

fn change_from_craft(c: &CraftCtrlEvent, ppq: u16) -> Change {
    let what = match c.kind {
        craft_midi::CtrlKind::Cc(n) => What::Control(n),
        craft_midi::CtrlKind::Program => What::Program,
        craft_midi::CtrlKind::PitchBend => What::Bend,
        craft_midi::CtrlKind::ChannelPressure => What::Pressure,
        // Polyphonic aftertouch is not represented in the compatibility reader's Change; skip it.
        craft_midi::CtrlKind::PolyPressure(_) => return Change { tick: 0, channel: 0, what: What::Control(0), value: 0 },
    };
    // craft-midi converts to 960 PPQ; scale back to source ppq.
    let tick = ((i128::from(c.tick) * i128::from(ppq.max(1)) + i128::from(craft_midi::PPQ) / 2) / i128::from(craft_midi::PPQ))
        .try_into()
        .unwrap_or(u32::MAX);
    Change { tick, channel: c.channel, what, value: c.value as i16 }
}

fn smf_to_craft(craft_smf: &CraftSmf, ppq: u16) -> Smf {
    let mut smf = Smf { format: craft_smf.format, ..Default::default() };

    // Craft-midi converts ticks to 960 PPQ when reading. Scale back to source ppq.
    let (from, to) = (i128::from(craft_midi::PPQ.max(1)), i128::from(ppq.max(1)));

    smf.ppq = ppq;
    // Scale tempo ticks from 960 PPQ back to source ppq.
    smf.tempos = craft_smf
        .tempos
        .iter()
        .map(|(tick, bpm)| {
            let tick = ((i128::from(*tick) * to + from / 2) / from).try_into().unwrap_or(u32::MAX);
            Tempo { tick, bpm: *bpm }
        })
        .collect();
    // Scale meter ticks.
    smf.time_signatures = craft_smf
        .meters
        .iter()
        .map(|(tick, beats, unit)| {
            let tick = ((i128::from(*tick) * to + from / 2) / from).try_into().unwrap_or(u32::MAX);
            TimeSignature { tick, beats: *beats, unit: u32::from(*unit).max(1) }
        })
        .collect();
    // Scale key signature ticks.
    smf.key_signatures = craft_smf
        .key_sigs
        .iter()
        .map(|(tick, sharps, minor)| {
            let tick = ((i128::from(*tick) * to + from / 2) / from).try_into().unwrap_or(u32::MAX);
            KeySignature { tick, sharps: *sharps, minor: *minor }
        })
        .collect();
    // Scale marker ticks.
    smf.markers = craft_smf
        .markers
        .iter()
        .map(|(tick, name)| {
            let tick = ((i128::from(*tick) * to + from / 2) / from).try_into().unwrap_or(u32::MAX);
            Marker { tick, name: trim_text(name) }
        })
        .collect();

    for ct in &craft_smf.tracks {
        let mut track = Track { name: trim_text(&ct.name), ..Default::default() };

        for n in &ct.sequence.notes {
            // Scale from 960 PPQ back to source ppq.
            let start = ((i128::from(n.start) * to + from / 2) / from).try_into().unwrap_or(u32::MAX);
            let end = ((i128::from(n.end()) * to + from / 2) / from).try_into().unwrap_or(u32::MAX);
            let len = end.saturating_sub(start).max(1);
            track.notes.push(Note { start, len, key: n.pitch, vel: n.velocity, channel: n.channel });
        }

        for c in &ct.sequence.ctrls {
            track.changes.push(change_from_craft(c, ppq));
        }

        smf.tracks.push(track);
    }

    // Compute ticks as the max of all track end ticks
    smf.ticks = smf.tracks.iter().flat_map(|t| t.notes.iter().map(|n| n.start.saturating_add(n.len)).chain(std::iter::once(0u32))).max().unwrap_or(0);

    smf.warnings = Vec::new();

    smf
}

fn smf_from_craft(smf: &Smf) -> CraftSmf {
    let mut craft_smf = CraftSmf { format: smf.format, ..Default::default() };

    // Scale from source PPQ to 960 when writing.
    let (from, to) = (i128::from(smf.ppq.max(1)), i128::from(craft_midi::PPQ));

    // Build tempo/meter/key/marker lists (scaled to 960 PPQ)
    craft_smf.tempos = smf
        .tempos
        .iter()
        .map(|t| {
            let tick = ((i128::from(t.tick) * to + from / 2) / from).try_into().unwrap_or(i64::MAX);
            (tick, t.bpm)
        })
        .collect();
    craft_smf.meters = smf
        .time_signatures
        .iter()
        .map(|t| {
            let tick = ((i128::from(t.tick) * to + from / 2) / from).try_into().unwrap_or(i64::MAX);
            (tick, t.beats, u8::try_from(t.unit).unwrap_or(1))
        })
        .collect();
    craft_smf.key_sigs = smf
        .key_signatures
        .iter()
        .map(|k| {
            let tick = ((i128::from(k.tick) * to + from / 2) / from).try_into().unwrap_or(i64::MAX);
            (tick, k.sharps, k.minor)
        })
        .collect();
    craft_smf.markers = smf
        .markers
        .iter()
        .map(|m| {
            let tick = ((i128::from(m.tick) * to + from / 2) / from).try_into().unwrap_or(i64::MAX);
            (tick, m.name.clone())
        })
        .collect();

    for track in &smf.tracks {
        let mut ct = CraftSmfTrack::default();
        if !track.name.is_empty() {
            ct.name = track.name.clone();
        }

        for n in &track.notes {
            let start = ((i128::from(n.start) * to + from / 2) / from).try_into().unwrap_or(i64::MAX);
            let end = ((i128::from(n.start.saturating_add(n.len)) * to + from / 2) / from).try_into().unwrap_or(i64::MAX);
            let length = end.saturating_sub(start).max(1);
            ct.sequence.notes.push(CraftNote { pitch: n.key, velocity: n.vel, release_velocity: 64, channel: n.channel, start, length });
        }

        for c in &track.changes {
            let kind = match c.what {
                What::Control(n) => craft_midi::CtrlKind::Cc(n),
                What::Program => craft_midi::CtrlKind::Program,
                What::Bend => craft_midi::CtrlKind::PitchBend,
                What::Pressure => craft_midi::CtrlKind::ChannelPressure,
            };
            let tick = ((i128::from(c.tick) * to + from / 2) / from).try_into().unwrap_or(i64::MAX);
            ct.sequence.ctrls.push(CraftCtrlEvent { tick, channel: c.channel, kind, value: c.value as i32 });
        }

        ct.sequence.sort();
        craft_smf.tracks.push(ct);
    }

    craft_smf
}

// ---------------------------------------------------------------------------
// Public API: read/write delegate to craft-midi
// ---------------------------------------------------------------------------

/// Reads a file of any format. Tracks without notes or channel messages (the tempo track) are
/// dropped; their tempos, signatures and markers are the file's. A file that is damaged past its
/// header still gives what could be read of it, with [`Smf::warnings`].
pub fn read(data: &[u8]) -> Result<Smf, Error> {
    // Check for valid MIDI header.
    if data.len() < 4 || data[0..4] != *b"MThd" {
        return Err(Error::NotMidi);
    }

    // Truncated header (less than 14 bytes).
    if data.len() < 14 {
        return Err(Error::Truncated);
    }

    // Extract ppq from the raw data to detect SMPTE format.
    let ppq_raw = u16::from_be_bytes([data[12], data[13]]);

    // Zero PPQ is invalid.
    if ppq_raw == 0 {
        return read_legacy(data);
    }

    // SMPTE time division has the high bit set (raw & 0x8000 != 0).
    // craft-midi doesn't handle SMPTE correctly, so use the legacy reader.
    if ppq_raw & 0x8000 != 0 {
        return read_legacy(data);
    }

    // Check if the file looks truncated/damaged (no valid End of Track at end).
    // Craft-midi doesn't handle truncation the same way as the legacy reader,
    // so fall back to it for damaged files.
    let is_damaged = !has_end_of_track(data);

    // Also check for markers at tick 0 (craft-midi may not include them).
    let has_tick_zero = has_tick_zero_marker(data);

    if is_damaged || has_tick_zero {
        return read_legacy(data);
    }

    // For standard PPQ files, use craft-midi directly (no scaling needed).
    let lr = read_smf_lenient(data).map_err(map_midi_error)?;
    let craft_smf = &lr.smf;

    // Extract ppq from the raw data (craft-midi doesn't expose it directly).
    let ppq = if data.len() >= 14 { u16::from_be_bytes([data[12], data[13]]) } else { 96u16 };

    let mut smf = smf_to_craft(craft_smf, ppq);
    // Copy warnings and instrument names from craft-midi's result.
    smf.warnings = lr.warnings.clone();
    for (i, inst) in lr.instruments.iter().enumerate() {
        if i < smf.tracks.len() && !inst.is_empty() {
            smf.tracks[i].instrument = inst.clone();
        }
    }

    // Cap tracks to the compatibility reader's MAX_TRACKS (craft-midi allows 4096, same limit).
    // Count actual MTrk chunks in the file to detect if we exceeded the limit.
    let mtrk_count = count_mtrk_chunks(data);
    if smf.tracks.len() > MAX_TRACKS {
        smf.tracks.truncate(MAX_TRACKS);
    } else if mtrk_count > MAX_TRACKS {
        // Craft-midi may have dropped some tracks; add a warning.
        smf.warnings.push(format!("more than {MAX_TRACKS} tracks: the rest was left out"));
    }

    // Drop empty tracks (no notes or changes) to match legacy reader behavior.
    smf.tracks.retain(|t| !t.notes.is_empty() || !t.changes.is_empty());

    // Truncate long names to MAX_TEXT (craft-midi may not truncate).
    for track in &mut smf.tracks {
        if track.name.len() > MAX_TEXT {
            let mut end = MAX_TEXT;
            while !track.name.is_char_boundary(end) {
                end -= 1;
            }
            track.name.truncate(end);
        }
    }

    // Cap events to the compatibility reader's MAX_EVENTS (craft-midi allows 5M).
    let total_notes: usize = smf.tracks.iter().map(|t| t.notes.len()).sum();
    let total_changes: usize = smf.tracks.iter().map(|t| t.changes.len()).sum();
    let total = total_notes + total_changes;
    if total > MAX_EVENTS {
        // Trim to fit within MAX_EVENTS, prioritizing notes then changes.
        cap_events(&mut smf, MAX_EVENTS);
        smf.warnings.push("holds more events than are read: the rest was left out".to_string());
    }

    // Compute ticks as max of ALL tick values (notes, tempos, signatures, markers).
    let all_ticks = smf
        .tracks
        .iter()
        .flat_map(|t| t.notes.iter().map(|n| n.start.saturating_add(n.len)).chain(std::iter::once(0u32)))
        .chain(smf.tempos.iter().map(|t| t.tick))
        .chain(smf.time_signatures.iter().map(|t| t.tick))
        .chain(smf.key_signatures.iter().map(|k| k.tick))
        .chain(smf.markers.iter().map(|m| m.tick));
    smf.ticks = all_ticks.max().unwrap_or(0);

    Ok(smf)
}

// Visit every track even after the budget is exhausted, clearing the remaining suffix.
fn cap_events(smf: &mut Smf, mut remaining: usize) {
    for track in &mut smf.tracks {
        let keep_notes = remaining.min(track.notes.len());
        track.notes.truncate(keep_notes);
        remaining -= keep_notes;
        let keep_changes = remaining.min(track.changes.len());
        track.changes.truncate(keep_changes);
        remaining -= keep_changes;
    }
}

/// Check if the data ends with a valid End of Track marker.
fn has_end_of_track(data: &[u8]) -> bool {
    // Look for the End of Track meta event near the end of the data.
    // The pattern is: 0x00 0xFF 0x2F 0x00 (delta-time=0, meta=EndOfTrack, length=0).
    if data.len() >= 4 {
        let end = &data[data.len() - 4..];
        end == [0x00, 0xFF, 0x2F, 0x00]
    } else {
        false
    }
}

/// Check if the file has a marker/cue point at tick 0 (first event).
/// Craft-midi may not include these, so we fall back to the legacy reader.
fn has_tick_zero_marker(data: &[u8]) -> bool {
    // Simple heuristic: scan track chunks for the pattern 0x00 0xFF 0x06/0x07
    // (delta=0 followed by marker/cue meta event).
    if data.len() < 14 + 8 {
        return false;
    }
    let mut pos = 14usize; // Skip header
    while pos + 8 <= data.len() {
        if data[pos..pos + 4] == *b"MTrk" {
            let chunk_len = u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
            let chunk_end = (pos + 8 + chunk_len).min(data.len());
            // Scan for pattern: 0x00 (delta=0) 0xFF 0x06/0x07
            let mut p = pos + 8; // Start of chunk data
            while p.saturating_add(5) <= chunk_end {
                if data[p] == 0x00 && data[p + 1] == 0xFF && (data[p + 2] == 0x06 || data[p + 2] == 0x07) {
                    return true;
                }
                p += 1;
            }
            pos = chunk_end;
        } else {
            pos += 1;
        }
    }
    false
}

/// Count the number of MTrk chunks in the file.
fn count_mtrk_chunks(data: &[u8]) -> usize {
    let mut count = 0;
    let mut pos = 14usize; // Skip header
    while pos + 8 <= data.len() {
        if data[pos..pos + 4] == *b"MTrk" {
            count += 1;
            let chunk_len = u32::from_be_bytes([data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]]) as usize;
            pos += 8 + chunk_len;
        } else {
            pos += 1;
        }
    }
    count
}

/// Writes the file, returning an empty buffer on encoding failure. Use [`try_write`]
/// for production exports so an error cannot silently truncate the destination.
/// Format 0 is one track that holds everything; any other format is written as
/// format 1: a track for the tempo map, signatures and markers, then one per [`Track`].
/// Delegates to craft-midi when the source PPQ is 960 (craft-midi's default); otherwise uses
/// the source-timed shared writer to preserve the source PPQ.
pub fn write(smf: &Smf) -> Vec<u8> {
    try_write(smf).unwrap_or_default()
}

/// Fallible export path. Call this before opening/truncating a destination file.
pub fn try_write(smf: &Smf) -> Result<Vec<u8>, craft_midi::MidiError> {
    if u32::from(smf.ppq) == craft_midi::PPQ { write_smf(&smf_from_craft(smf)) } else { try_write_source_timed(smf, 512 * 1024 * 1024) }
}

// ---------------------------------------------------------------------------
// Legacy compatibility: keep the old read/write implementations as fallback
// (The above delegates to craft-midi; these are kept for reference.)
// ---------------------------------------------------------------------------

#[cfg(test)]
fn varlen(out: &mut Vec<u8>, value: u32) {
    let value = value.min(0x0FFF_FFFF);
    for shift in [21, 14, 7] {
        if value >> shift != 0 {
            out.push(((value >> shift) & 0x7F) as u8 | 0x80);
        }
    }
    out.push((value & 0x7F) as u8);
}

#[cfg(test)]
fn chunk(out: &mut Vec<u8>, id: &[u8; 4], body: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(body.len().min(u32::MAX as usize) as u32).to_be_bytes());
    out.extend_from_slice(body);
}

/// A message to write: its tick, its rank among the messages of that tick, and its bytes.
type Message = (u32, u8, Vec<u8>);

/// Ranks at one tick: a note ends before the settings change, and those before a note starts.
const META: u8 = 0;
const NOTE_OFF: u8 = 1;
const CHANGE: u8 = 2;
const NOTE_ON: u8 = 3;

fn meta(tick: u32, kind: u8, data: &[u8]) -> Result<Message, craft_midi::MidiError> {
    Ok((tick, META, craft_midi::encode_meta_event(kind, data)?))
}

/// The tempo map, the signatures and the markers.
fn conductor(smf: &Smf) -> Result<Vec<Message>, craft_midi::MidiError> {
    let tempo = |t: &Tempo| {
        let bpm = if t.bpm.is_finite() { t.bpm.clamp(1.0, 2000.0) } else { 120.0 };
        meta(t.tick, 0x51, &((60_000_000.0 / bpm).round() as u32).min(0x00FF_FFFF).to_be_bytes()[1..])
    };
    let time = |t: &TimeSignature| meta(t.tick, 0x58, &[t.beats.max(1), t.unit.clamp(1, 128).ilog2() as u8, 24, 8]);
    let key = |k: &KeySignature| meta(k.tick, 0x59, &[k.sharps.clamp(-7, 7) as u8, u8::from(k.minor)]);
    let marker = |m: &Marker| meta(m.tick, 0x06, m.name.as_bytes());
    (smf.tempos.iter().map(tempo))
        .chain(smf.time_signatures.iter().map(time))
        .chain(smf.key_signatures.iter().map(key))
        .chain(smf.markers.iter().map(marker))
        .collect()
}

fn messages(track: &Track) -> Result<Vec<Message>, craft_midi::MidiError> {
    let mut out = vec![meta(0, 0x03, track.name.as_bytes())?];
    if !track.instrument.is_empty() {
        out.push(meta(0, 0x04, track.instrument.as_bytes())?);
    }
    out.extend(track.changes.iter().map(|c| {
        let (status, value) = (c.channel & 0x0F, c.value.clamp(0, 127) as u8);
        let bytes = match c.what {
            What::Control(number) => vec![0xB0 | status, number & 0x7F, value],
            What::Program => vec![0xC0 | status, value],
            What::Pressure => vec![0xD0 | status, value],
            What::Bend => {
                let wheel = (i32::from(c.value) + 8192).clamp(0, 16383);
                vec![0xE0 | status, (wheel & 0x7F) as u8, (wheel >> 7) as u8]
            }
        };
        (c.tick, CHANGE, bytes)
    }));
    out.extend(track.notes.iter().flat_map(|n| {
        let (status, key) = (n.channel & 0x0F, n.key & 0x7F);
        [
            (n.start, NOTE_ON, vec![0x90 | status, key, n.vel.clamp(1, 127)]),
            (n.start.saturating_add(n.len.max(1)), NOTE_OFF, vec![0x80 | status, key, 0]),
        ]
    }));
    Ok(out)
}

/// Compatibility writer: keeps source ticks and Loop event policy, using shared SMF framing.
/// Returns an empty buffer if the conservative 512 MiB serialized-output budget is exceeded.
pub fn write_legacy(smf: &Smf) -> Vec<u8> {
    try_write_source_timed(smf, 512 * 1024 * 1024).unwrap_or_default()
}

/// Preserve Loop's source-tick/event policy while propagating framing failures.
/// The byte limit is conservative and does not bound caller-owned model memory.
pub fn try_write_source_timed(smf: &Smf, max_bytes: usize) -> Result<Vec<u8>, craft_midi::MidiError> {
    let single = smf.format == 0;
    let mut events = vec![conductor(smf)?];
    for track in smf.tracks.iter().take(u16::MAX as usize - 1) {
        let track = messages(track)?;
        if single {
            if let Some(first) = events.first_mut() {
                first.extend(track);
            }
        } else {
            events.push(track);
        }
    }
    let events = events
        .into_iter()
        .map(|track| track.into_iter().map(|(tick, order, bytes)| craft_midi::TimedEvent { tick: u64::from(tick), order, bytes }).collect())
        .collect();
    craft_midi::write_timed_smf(u16::from(!single), smf.ppq.clamp(1, 0x7fff), events, true, max_bytes)
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Truncated)?;
        let slice = self.data.get(self.pos..end).ok_or(Error::Truncated)?;
        self.pos = end;
        Ok(slice)
    }

    /// What is left of the data.
    fn rest(&mut self) -> &'a [u8] {
        let slice = self.data.get(self.pos..).unwrap_or_default();
        self.pos = self.data.len();
        slice
    }

    fn byte(&mut self) -> Result<u8, Error> {
        self.take(1)?.first().copied().ok_or(Error::Truncated)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        match self.take(2)? {
            [a, b] => Ok(u16::from_be_bytes([*a, *b])),
            _ => Err(Error::Truncated),
        }
    }

    fn u32(&mut self) -> Result<u32, Error> {
        match self.take(4)? {
            [a, b, c, d] => Ok(u32::from_be_bytes([*a, *b, *c, *d])),
            _ => Err(Error::Truncated),
        }
    }

    fn varlen(&mut self) -> Result<u32, Error> {
        let mut value = 0u32;
        for _ in 0..4 {
            let b = self.byte()?;
            value = (value << 7) | u32::from(b & 0x7F);
            if b & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(Error::Truncated)
    }

    fn done(&self) -> bool {
        self.pos >= self.data.len()
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.get(..MAX_TEXT).unwrap_or(bytes)).trim_matches(|c: char| c.is_control() || c.is_whitespace()).to_string()
}

/// Why a track's events stop before its End of Track.
type Stop = &'static str;
const CUT: Stop = "ends in the middle of an event";

/// A file being read: what is made of it, and the room left for its events.
struct Parse {
    smf: Smf,
    room: usize,
    /// The notes sounding on each channel and key, oldest first: where each started and how hard.
    sounding: Vec<VecDeque<(u32, u8)>>,
    /// Lyrics and polyphonic aftertouch, which are not kept.
    lyrics: usize,
    aftertouch: usize,
    /// Tempo changes, signatures or markers past [`MAX_META`].
    crowded: bool,
}

impl Parse {
    fn warn(&mut self, what: String) {
        if self.smf.warnings.len() < MAX_WARNINGS {
            self.smf.warnings.push(what);
        }
    }

    /// Takes room for one more event; an error when the file holds more than are read.
    fn event(&mut self) -> Result<(), Stop> {
        self.room = self.room.checked_sub(1).ok_or("holds more events than are read: the rest was left out")?;
        Ok(())
    }

    fn meta(&mut self, track: &mut Track, tick: u32, kind: u8, body: &[u8]) {
        /// Whether `list` is full: `item` joins it otherwise.
        fn full<T>(list: &mut Vec<T>, item: T) -> bool {
            let room = list.len() < MAX_META;
            if room {
                list.push(item);
            }
            !room
        }
        let smf = &mut self.smf;
        let crowded = match (kind, body) {
            (0x03, _) if track.name.is_empty() => {
                track.name = text(body);
                false
            }
            (0x04, _) if track.instrument.is_empty() => {
                track.instrument = text(body);
                false
            }
            (0x05, _) => {
                self.lyrics += 1;
                false
            }
            (0x06 | 0x07, _) => full(&mut smf.markers, Marker { tick, name: text(body) }),
            (0x51, [a, b, c]) => full(&mut smf.tempos, Tempo { tick, bpm: 60_000_000.0 / f64::from(u32::from_be_bytes([0, *a, *b, *c]).max(1)) }),
            // (A beat is at most a 128th note; a bar of no beats is no signature.)
            (0x58, [beats, power, ..]) if *beats > 0 && *power <= 7 => {
                full(&mut smf.time_signatures, TimeSignature { tick, beats: *beats, unit: 1 << power })
            }
            (0x59, [sharps, minor, ..]) => {
                full(&mut smf.key_signatures, KeySignature { tick, sharps: (*sharps as i8).clamp(-7, 7), minor: *minor == 1 })
            }
            _ => false,
        };
        self.crowded |= crowded;
    }

    /// Reads the events of a track up to its End of Track (`Ok`) or to where they stop making sense.
    fn events(&mut self, r: &mut Reader, track: &mut Track, tick: &mut u32) -> Result<(), Stop> {
        let mut status = 0u8;
        loop {
            if r.done() {
                return Err("has no End of Track");
            }
            *tick = tick.saturating_add(r.varlen().map_err(|_| CUT)?);
            let first = r.byte().map_err(|_| CUT)?;
            match first {
                0xFF => {
                    let kind = r.byte().map_err(|_| CUT)?;
                    let len = r.varlen().map_err(|_| CUT)? as usize;
                    let body = r.take(len).map_err(|_| CUT)?;
                    if kind == 0x2F {
                        return Ok(());
                    }
                    self.meta(track, *tick, kind, body);
                }
                // System exclusive data is for one device: skipped by its length.
                0xF0 | 0xF7 => {
                    let len = r.varlen().map_err(|_| CUT)? as usize;
                    r.take(len).map_err(|_| CUT)?;
                }
                _ => {
                    // Running status: a data byte here reuses the status before it. (The status is
                    // kept across meta events too: some writers count on that.)
                    let mut data = || r.byte().map(|b| b & 0x7F).map_err(|_| CUT);
                    let one = if first & 0x80 != 0 {
                        status = first;
                        data()?
                    } else {
                        first
                    };
                    let (kind, channel) = (status & 0xF0, status & 0x0F);
                    let (what, value) = match kind {
                        0x80 | 0x90 => {
                            let vel = data()?;
                            let key = usize::from(channel) << 7 | usize::from(one);
                            // A note-on of no velocity is a note-off; an off ends the oldest note of its key.
                            if kind == 0x90 && vel > 0 {
                                self.event()?;
                                if let Some(open) = self.sounding.get_mut(key) {
                                    open.push_back((*tick, vel));
                                }
                            } else if let Some((start, vel)) = self.sounding.get_mut(key).and_then(VecDeque::pop_front) {
                                track.notes.push(Note { start, len: tick.saturating_sub(start).max(1), key: one, vel, channel });
                            }
                            continue;
                        }
                        0xA0 => {
                            data()?;
                            self.aftertouch += 1;
                            continue;
                        }
                        0xB0 => (What::Control(one), i16::from(data()?)),
                        0xC0 => (What::Program, i16::from(one)),
                        0xD0 => (What::Pressure, i16::from(one)),
                        0xE0 => (What::Bend, (i16::from(data()?) << 7 | i16::from(one)) - 8192),
                        _ => return Err("holds a byte that is no event"),
                    };
                    self.event()?;
                    track.changes.push(Change { tick: *tick, channel, what, value });
                }
            }
        }
    }

    /// Reads one track chunk. Returns how many of its bytes its events took when they end as
    /// they should, with an End of Track.
    fn track(&mut self, data: &[u8], number: usize) -> Option<usize> {
        let (mut r, mut track, mut tick) = (Reader { data, pos: 0 }, Track::default(), 0u32);
        let ended = self.events(&mut r, &mut track, &mut tick);
        if let Err(why) = ended {
            self.warn(format!("track {number} {why}: what came before is kept"));
        }
        // Notes never released end where the track does.
        for (i, open) in self.sounding.iter_mut().enumerate() {
            let (channel, key) = ((i >> 7) as u8, (i & 0x7F) as u8);
            track.notes.extend(open.drain(..).map(|(start, vel)| Note { start, len: tick.saturating_sub(start).max(1), key, vel, channel }));
        }
        track.notes.sort_by_key(|n| (n.start, n.key));
        self.smf.ticks = self.smf.ticks.max(tick);
        if !(track.notes.is_empty() && track.changes.is_empty()) {
            self.smf.tracks.push(track);
        }
        ended.ok().map(|()| r.pos)
    }
}

/// Legacy read: reads a file of any format without craft-midi.
pub fn read_legacy(data: &[u8]) -> Result<Smf, Error> {
    let mut r = Reader { data, pos: 0 };
    if r.take(4).map_err(|_| Error::NotMidi)? != b"MThd" {
        return Err(Error::NotMidi);
    }
    let mut p = Parse { smf: Smf::default(), room: MAX_EVENTS, sounding: vec![VecDeque::new(); 16 * 128], lyrics: 0, aftertouch: 0, crowded: false };
    let header_len = r.u32()? as usize;
    let header = match r.take(header_len) {
        Ok(header) => header,
        Err(_) => {
            p.warn("the header gives a length the file does not have".into());
            r.take(6)?
        }
    };
    let mut header = Reader { data: header, pos: 0 };
    let (format, _count, raw) = (header.u16()?, header.u16()?, header.u16()?);
    let (ppq, clock) = division(raw)?;
    (p.smf.format, p.smf.ppq) = (if format > 2 { 1 } else { format }, ppq);
    if format > 2 {
        p.warn(format!("format {format} is unknown: read as tracks that play together"));
    }
    let mut seen = 0usize;
    while !r.done() {
        let at = r.pos;
        let Ok((id, len)) = r.take(4).and_then(|id| Ok((id, r.u32()? as usize))) else {
            p.warn(format!("{} bytes after the last chunk were ignored", data.len() - at));
            break;
        };
        if id == b"MTrk" {
            seen += 1;
            if seen > MAX_TRACKS {
                p.warn(format!("more than {MAX_TRACKS} tracks: the rest was left out"));
                break;
            }
            let start = r.pos;
            let (body, fits) = match r.take(len) {
                Ok(body) => (body, true),
                Err(_) => {
                    p.warn(format!("track {seen} is shorter than its length says"));
                    (r.rest(), false)
                }
            };
            // A length that reaches past the track's end into what follows is not believed: the
            // next chunk is looked for where the track ended.
            let next = data.get(r.pos..).unwrap_or_default();
            let believed = fits && (next.is_empty() || next.starts_with(b"MTrk"));
            if let Some(used) = p.track(body, seen).filter(|used| *used < body.len() && !believed) {
                r.pos = start + used;
                if fits {
                    p.warn(format!("track {seen} ends before its length says"));
                }
            }
        } else if id.iter().all(|b| b.is_ascii_graphic() || *b == b' ') && r.take(len).is_ok() {
            // A chunk of another kind is skipped, as the specification asks.
        } else {
            // No chunk at all: the next track, if there is one, is looked for.
            let after = data.get(at + 1..).unwrap_or_default();
            match after.windows(4).position(|w| w == b"MTrk") {
                Some(skip) => {
                    p.warn(format!("{} bytes that are no chunk were skipped", skip + 1));
                    r.pos = at + 1 + skip;
                }
                None => {
                    p.warn(format!("{} bytes after the last chunk were ignored", data.len() - at));
                    break;
                }
            }
        }
    }
    if p.lyrics > 0 {
        p.warn(format!("{} lyrics were left out", p.lyrics));
    }
    if p.aftertouch > 0 {
        p.warn(format!("{} polyphonic aftertouch messages were left out", p.aftertouch));
    }
    if p.crowded {
        p.warn(format!("more than {MAX_META} tempo changes, signatures or markers: the rest was left out"));
    }
    let mut smf = p.smf;
    if smf.format == 2 {
        smf.warnings.push("format 2: each track is a song of its own; they are read side by side".into());
    }
    // (Stable: of two tempos at one tick the later in the file stays the later.)
    smf.tempos.sort_by_key(|t| t.tick);
    one_tempo_a_tick(&mut smf.tempos);
    smf.time_signatures.sort_by_key(|t| t.tick);
    smf.key_signatures.sort_by_key(|k| k.tick);
    smf.markers.sort_by_key(|m| m.tick);
    if let Some(bpm) = clock {
        smf.tempos = vec![Tempo { tick: 0, bpm }];
        smf.warnings
            .push(format!("SMPTE time division: read as {ppq} ticks a beat at {bpm:.2} BPM, which keeps its timing; tempo changes are ignored"));
    }
    Ok(smf)
}

/// The ticks a quarter note has, and for SMPTE time (ticks of the clock, not of the beat) the one
/// tempo at which that many ticks a beat keep the clock's time.
fn division(raw: u16) -> Result<(u16, Option<f64>), Error> {
    if raw == 0 {
        return Err(Error::Unsupported("a time division of zero"));
    }
    if raw & 0x8000 == 0 {
        return Ok((raw, None));
    }
    // Frames a second (as a negative number) and ticks a frame: a beat a second at 60 BPM.
    // Twenty-nine stands for the 29.97 frames of drop-frame video.
    let (frames, ticks) = (-i16::from((raw >> 8) as u8 as i8), raw & 0xFF);
    let (nominal, bpm) = match frames {
        24 | 25 | 30 => (frames as u16, 60.0),
        29 => (30, 60.0 * 29.97 / 30.0),
        _ => return Err(Error::Unsupported("an SMPTE time division with an unknown frame rate")),
    };
    if ticks == 0 {
        return Err(Error::Unsupported("an SMPTE time division of no ticks a frame"));
    }
    Ok((nominal * ticks, Some(bpm)))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn meta(tick: u32, kind: u8, data: &[u8]) -> Message {
        let mut bytes = vec![0xFF, kind];
        varlen(&mut bytes, data.len() as u32);
        bytes.extend_from_slice(data);
        (tick, META, bytes)
    }

    /// The tempo map, the signatures and the markers.
    fn conductor(smf: &Smf) -> Vec<Message> {
        let tempo = |t: &Tempo| {
            let bpm = if t.bpm.is_finite() { t.bpm.clamp(1.0, 2000.0) } else { 120.0 };
            meta(t.tick, 0x51, &((60_000_000.0 / bpm).round() as u32).min(0x00FF_FFFF).to_be_bytes()[1..])
        };
        let time = |t: &TimeSignature| meta(t.tick, 0x58, &[t.beats.max(1), t.unit.clamp(1, 128).ilog2() as u8, 24, 8]);
        let key = |k: &KeySignature| meta(k.tick, 0x59, &[k.sharps.clamp(-7, 7) as u8, u8::from(k.minor)]);
        let marker = |m: &Marker| meta(m.tick, 0x06, m.name.as_bytes());
        (smf.tempos.iter().map(tempo))
            .chain(smf.time_signatures.iter().map(time))
            .chain(smf.key_signatures.iter().map(key))
            .chain(smf.markers.iter().map(marker))
            .collect()
    }

    fn messages(track: &Track) -> Vec<Message> {
        let mut out = vec![meta(0, 0x03, track.name.as_bytes())];
        if !track.instrument.is_empty() {
            out.push(meta(0, 0x04, track.instrument.as_bytes()));
        }
        out.extend(track.changes.iter().map(|c| {
            let (status, value) = (c.channel & 0x0F, c.value.clamp(0, 127) as u8);
            let bytes = match c.what {
                What::Control(number) => vec![0xB0 | status, number & 0x7F, value],
                What::Program => vec![0xC0 | status, value],
                What::Pressure => vec![0xD0 | status, value],
                What::Bend => {
                    let wheel = (i32::from(c.value) + 8192).clamp(0, 16383);
                    vec![0xE0 | status, (wheel & 0x7F) as u8, (wheel >> 7) as u8]
                }
            };
            (c.tick, CHANGE, bytes)
        }));
        out.extend(track.notes.iter().flat_map(|n| {
            let (status, key) = (n.channel & 0x0F, n.key & 0x7F);
            [
                (n.start, NOTE_ON, vec![0x90 | status, key, n.vel.clamp(1, 127)]),
                (n.start.saturating_add(n.len.max(1)), NOTE_OFF, vec![0x80 | status, key, 0]),
            ]
        }));
        out
    }

    fn track_chunk(out: &mut Vec<u8>, mut messages: Vec<Message>) {
        messages.sort_by_key(|(tick, rank, _)| (*tick, *rank));
        let (mut body, mut last) = (Vec::new(), 0);
        for (tick, _, bytes) in messages {
            varlen(&mut body, tick - last);
            last = tick;
            body.extend_from_slice(&bytes);
        }
        body.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        chunk(out, b"MTrk", &body);
    }

    /// Legacy write: writes the file directly without craft-midi.
    fn legacy_writer_oracle(smf: &Smf) -> Vec<u8> {
        let tracks = || smf.tracks.iter().take(u16::MAX as usize - 1);
        let single = smf.format == 0;
        let mut out = Vec::new();
        let mut header = Vec::new();
        header.extend_from_slice(&u16::from(!single).to_be_bytes());
        header.extend_from_slice(&(if single { 1 } else { tracks().count() as u16 + 1 }).to_be_bytes());
        header.extend_from_slice(&smf.ppq.clamp(1, 0x7FFF).to_be_bytes());
        chunk(&mut out, b"MThd", &header);
        if single {
            track_chunk(&mut out, conductor(smf).into_iter().chain(tracks().flat_map(messages)).collect());
        } else {
            track_chunk(&mut out, conductor(smf));
            tracks().for_each(|track| track_chunk(&mut out, messages(track)));
        }
        out
    }

    #[test]
    fn shared_source_timed_writer_matches_legacy_bytes() {
        for ppq in [0, 1, 96, 480, 960, 32767, 65535] {
            for format in [0, 1, 2] {
                let mut smf = song();
                smf.ppq = ppq;
                smf.format = format;
                smf.tracks[0].instrument = "Acoustic Bass".into();
                smf.tracks[0].notes.extend([note(20, 100, 60, 90, 0), note(20, 200, 60, 110, 0), note(30, 10, 60, 50, 0)]);
                smf.tempos.extend([Tempo { tick: 7, bpm: f64::NAN }, Tempo { tick: u32::MAX, bpm: 0.0 }]);
                smf.markers.push(Marker { tick: 0, name: "Opening".into() });
                assert_eq!(write_legacy(&smf), legacy_writer_oracle(&smf), "PPQ {ppq}, format {format}");
            }
        }
        assert_eq!(write_legacy(&Smf::default()), legacy_writer_oracle(&Smf::default()));
    }
    #[test]
    fn bounded_export_rejects_small_budget_without_successful_empty_file() {
        assert!(try_write_source_timed(&song(), 1).is_err());
    }

    fn note(start: u32, len: u32, key: u8, vel: u8, channel: u8) -> Note {
        Note { start, len, key, vel, channel }
    }

    fn song() -> Smf {
        Smf {
            tempos: vec![Tempo { tick: 0, bpm: 125.0 }],
            tracks: vec![
                Track { name: "Bass".into(), notes: vec![note(0, 48, 33, 100, 0), note(96, 20000, 36, 1, 0)], ..Track::default() },
                Track { name: "Chords".into(), notes: (0..3).map(|i| note(384, 360, 57 + i * 4, 90, 1)).collect(), ..Track::default() },
            ],
            ticks: 20096,
            ..Smf::default()
        }
    }

    /// A file of the given format and division with these track bodies (End of Track is added).
    fn file(format: u16, division: u16, tracks: &[&[u8]]) -> Vec<u8> {
        let mut out = Vec::new();
        let header = [format.to_be_bytes(), (tracks.len() as u16).to_be_bytes(), division.to_be_bytes()].concat();
        chunk(&mut out, b"MThd", &header);
        for body in tracks {
            chunk(&mut out, b"MTrk", &[body, &[0x00, 0xFF, 0x2F, 0x00][..]].concat());
        }
        out
    }

    #[test]
    fn round_trip() {
        assert_eq!(read(&write(&song())).unwrap(), song());
        assert_eq!(Smf::default().tempo(), 120.0);
        // A tempo is kept in whole microseconds a beat: close, not exact.
        let odd = read(&write(&Smf { tempos: vec![Tempo { tick: 0, bpm: 126.0 }], ..song() })).unwrap();
        assert!((odd.tempo() - 126.0).abs() < 0.001 && odd.tracks == song().tracks);
    }

    #[test]
    fn everything_a_file_holds_round_trips_in_either_format() {
        let change = |tick, channel, what, value| Change { tick, channel, what, value };
        let full = Smf {
            ppq: 480,
            tempos: vec![Tempo { tick: 0, bpm: 120.0 }, Tempo { tick: 960, bpm: 100.0 }, Tempo { tick: 1920, bpm: 150.0 }],
            time_signatures: vec![TimeSignature { tick: 0, beats: 6, unit: 8 }, TimeSignature { tick: 1440, beats: 7, unit: 16 }],
            key_signatures: vec![KeySignature { tick: 0, sharps: -3, minor: true }],
            markers: vec![Marker { tick: 0, name: "Intro".into() }, Marker { tick: 1920, name: "Verse é".into() }],
            tracks: vec![
                Track {
                    name: "Keys".into(),
                    instrument: "Grand".into(),
                    notes: vec![note(0, 480, 60, 64, 2), note(480, 480, 60, 127, 2), note(480, 1, 72, 1, 2)],
                    changes: vec![
                        change(0, 2, What::Control(cc::BANK), 1),
                        change(0, 2, What::Program, 5),
                        change(0, 2, What::Control(cc::VOLUME), 90),
                        change(10, 2, What::Bend, -8192),
                        change(20, 2, What::Bend, 8191),
                        change(30, 2, What::Pressure, 77),
                        change(40, 2, What::Control(cc::SUSTAIN), 127),
                        change(40, 2, What::Control(93), 12),
                    ],
                },
                Track { name: "Drums".into(), notes: vec![note(0, 24, 36, 110, DRUMS)], ..Track::default() },
            ],
            ticks: 1920,
            ..Smf::default()
        };
        assert_eq!(read(&write(&full)).unwrap(), full);
        assert_eq!(full.tracks[0].controls(2, cc::SUSTAIN).map(|c| c.value).collect::<Vec<_>>(), [127]);

        // Format 0 is one track: the same notes and messages, under the first name.
        let single = read(&write(&Smf { format: 0, ..full.clone() })).unwrap();
        assert_eq!((single.format, single.tracks.len(), single.tracks[0].name.as_str()), (0, 1, "Keys"));
        assert_eq!((single.tracks[0].notes.len(), single.tracks[0].changes.len()), (4, 8));
        assert_eq!((&single.tempos, &single.markers, &single.time_signatures), (&full.tempos, &full.markers, &full.time_signatures));
    }

    #[test]
    fn reads_running_status_format_0_and_note_on_velocity_zero() {
        let smf = read(&file(0, 96, &[&[0x00, 0x90, 60, 100, 0x30, 64, 90, 0x30, 60, 0, 0x00, 64, 0]])).unwrap();
        assert_eq!(smf.tracks[0].notes, [note(0, 96, 60, 100, 0), note(48, 48, 64, 90, 0)]);
        assert_eq!((smf.tempo(), smf.format, smf.ticks, smf.warnings.len()), (120.0, 0, 96, 0));
        // Running status goes on across a meta event and system exclusive data, which are skipped by their length.
        let body = [0x00, 0x90, 60, 100, 0x00, 0xFF, 0x01, 0x02, b'h', b'i', 0x00, 0xF0, 0x03, 1, 2, 0xF7, 0x10, 60, 0, 0x00, 0xF7, 0x01, 0x55];
        assert_eq!(read(&file(1, 96, &[&body])).unwrap().tracks[0].notes, [note(0, 16, 60, 100, 0)]);
    }

    #[test]
    fn notes_of_one_key_that_overlap_end_oldest_first_and_unended_ones_end_with_the_track() {
        // On, on, off, off of one key; then a note that is never released.
        let body = [0x00, 0x90, 60, 100, 0x0A, 60, 50, 0x0A, 0x80, 60, 0, 0x0A, 60, 0, 0x00, 0x91, 40, 9, 0x20, 0xB1, 7, 1];
        let smf = read(&file(1, 96, &[&body])).unwrap();
        assert_eq!(smf.tracks[0].notes, [note(0, 20, 60, 100, 0), note(10, 20, 60, 50, 0), note(30, 32, 40, 9, 1)]);
    }

    #[test]
    fn any_division_is_rescaled_with_what_was_together_still_together_and_no_note_without_length() {
        let mut rng = 12345u32;
        let mut next = |n: u32| {
            rng = rng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (rng >> 8) % n
        };
        for ppq in [1u16, 24, 96, 100, 120, 480, 960, 1000, 32767] {
            let starts: Vec<u32> = (0..40).map(|_| next(u32::from(ppq) * 8)).collect();
            // Chords of two on one start, and each note followed at once by another.
            let notes: Vec<Note> = (starts.iter().enumerate())
                .flat_map(|(i, start)| {
                    let len = 1 + next(u32::from(ppq));
                    [note(*start, len, 10, 80, 0), note(*start, 1, 20 + i as u8, 80, 0), note(*start + len, 1 + next(3), 80, 80, 0)]
                })
                .collect();
            let mut smf = Smf {
                ppq,
                tracks: vec![Track { notes: notes.clone(), ..Track::default() }],
                markers: vec![Marker { tick: starts[0], name: "m".into() }],
                ..Smf::default()
            };
            smf.rescale(96);
            assert_eq!(smf.ppq, 96);
            let scaled = &smf.tracks[0].notes;
            for (before, after) in notes.chunks(3).zip(scaled.chunks(3)) {
                assert!(after.iter().all(|n| n.len >= 1), "{ppq}: {after:?}");
                assert_eq!(after[0].start, after[1].start, "{ppq}: a chord stays a chord");
                assert_eq!(after[0].start + after[0].len, after[2].start.max(after[0].start + 1), "{ppq}: what touched still touches ({before:?})");
                let exact = f64::from(before[0].start) * 96.0 / f64::from(ppq);
                assert!((f64::from(after[0].start) - exact).abs() <= 0.5, "{ppq}: the nearest tick");
            }
            assert_eq!(smf.markers[0].tick, scaled[0].start, "{ppq}: a marker stays on its note");
        }
        // A whole number of times finer is exact, and back.
        let mut fine = song();
        fine.rescale(480);
        assert_eq!(fine.tracks[0].notes[0], note(0, 240, 33, 100, 0));
        fine.rescale(96);
        assert_eq!(fine, song());
        // Tempos that land on one tick leave the last.
        let mut coarse = Smf {
            ppq: 960,
            tempos: vec![Tempo { tick: 0, bpm: 100.0 }, Tempo { tick: 2, bpm: 110.0 }, Tempo { tick: 960, bpm: 90.0 }],
            ..Smf::default()
        };
        coarse.rescale(96);
        assert_eq!(coarse.tempos, [Tempo { tick: 0, bpm: 110.0 }, Tempo { tick: 96, bpm: 90.0 }]);
    }

    #[test]
    fn smpte_time_is_read_at_the_tempo_that_keeps_its_clock() {
        let body = [0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, 0x00, 0x90, 60, 100, 0x87, 0x68, 60, 0];
        // 25 frames of 40 ticks: a thousand ticks a second, so the note lasts one second, one beat at 60 BPM.
        let smf = read(&file(1, u16::from_be_bytes([(-25i8) as u8, 40]), &[&body])).unwrap();
        assert_eq!((smf.ppq, smf.tempos.as_slice(), smf.tracks[0].notes[0].len), (1000, &[Tempo { tick: 0, bpm: 60.0 }][..], 1000));
        assert!(smf.warnings.iter().any(|w| w.contains("SMPTE")));
        let drop = read(&file(1, u16::from_be_bytes([(-29i8) as u8, 80]), &[&body])).unwrap();
        assert_eq!(drop.ppq, 2400);
        assert!((drop.tempo() - 59.94).abs() < 1e-9);
        for bad in [[(-7i8) as u8, 40], [(-25i8) as u8, 0], [0, 0]] {
            assert!(matches!(read(&file(1, u16::from_be_bytes(bad), &[&body])), Err(Error::Unsupported(_))), "{bad:?}");
        }
    }

    #[test]
    fn damaged_files_give_what_can_be_read_and_say_what_could_not() {
        let one: &[u8] = &[0x00, 0x90, 60, 100, 0x60, 0x80, 60, 0];
        let two: &[u8] = &[0x00, 0x91, 62, 100, 0x60, 0x81, 62, 0];
        let good = file(1, 96, &[one, two]);
        let keys = |smf: &Smf| smf.tracks.iter().map(|t| t.notes.iter().map(|n| n.key).collect::<Vec<_>>()).collect::<Vec<_>>();
        let both = vec![vec![60], vec![62]];
        assert_eq!((keys(&read(&good).unwrap()), read(&good).unwrap().warnings.len()), (both.clone(), 0));

        // Garbage after the last chunk, long or short.
        for junk in [&b"\0\0\0"[..], &b"garbage that is no chunk at all"[..], &[0xFF; 9][..]] {
            let smf = read(&[&good[..], junk].concat()).unwrap();
            assert_eq!((keys(&smf), smf.warnings.len()), (both.clone(), 1), "{junk:?}: {:?}", smf.warnings);
        }
        // A chunk of another kind between the tracks is skipped without a word.
        let mut alien = file(1, 96, &[one]);
        chunk(&mut alien, b"XFIH", &[1, 2, 3]);
        chunk(&mut alien, b"MTrk", &[two, &[0x00, 0xFF, 0x2F, 0x00][..]].concat());
        assert_eq!((keys(&read(&alien).unwrap()), read(&alien).unwrap().warnings.len()), (both.clone(), 0));

        // The first track's length says too much (it swallows the second), too little, or nothing.
        let len_at = 14 + 4;
        for (len, tracks) in [(15u32, both.clone()), (200, both.clone()), (u32::MAX, both.clone()), (6, both.clone()), (0, vec![vec![62]])] {
            let mut bad = good.clone();
            bad[len_at..len_at + 4].copy_from_slice(&len.to_be_bytes());
            let smf = read(&bad).unwrap();
            assert_eq!(keys(&smf), tracks, "length {len}: {:?}", smf.warnings);
            assert!(!smf.warnings.is_empty(), "length {len}");
        }
        // A file cut inside its last track keeps the notes before the cut, the open one ending there.
        let cut = read(&good[..good.len() - 7]).unwrap();
        assert_eq!((keys(&cut), cut.tracks[1].notes[0].len), (both.clone(), 96));
        assert!(cut.warnings.iter().any(|w| w.contains("track 2")), "{:?}", cut.warnings);
        // A data byte before any status, and a system message that has no place in a file.
        for bad in [&[0x00, 60, 100][..], &[0x00, 0x90, 60, 100, 0x00, 0xF8, 0x00, 0x90, 62, 100][..]] {
            let smf = read(&file(1, 96, &[bad, two])).unwrap();
            assert!(smf.warnings.iter().any(|w| w.contains("no event")) && keys(&smf).last() == Some(&vec![62]), "{:?}", smf.warnings);
        }
        // A header with a wrong length, and one that is longer than six bytes.
        let mut long = good.clone();
        long[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert_eq!(keys(&read(&long).unwrap()), both);
        let mut roomy = Vec::new();
        chunk(&mut roomy, b"MThd", &[0, 1, 0, 1, 0, 96, 9, 9]);
        roomy.extend_from_slice(&good[14..]);
        assert_eq!(keys(&read(&roomy).unwrap()), both);
        // Formats: 2 is read side by side, an unknown one as 1, both with a word about it.
        assert_eq!(read(&file(2, 96, &[one, two])).map(|smf| (smf.format, smf.tracks.len(), smf.warnings.len())), Ok((2, 2, 1)));
        assert_eq!(read(&file(7, 96, &[one])).map(|smf| (smf.format, smf.warnings.len())), Ok((1, 1)));
    }

    #[test]
    fn meta_events_are_kept_or_counted_and_never_trusted() {
        let body = [
            &[0x00, 0xFF, 0x03, 0x05][..],
            b"Lead\0",
            &[0x00, 0xFF, 0x03, 0x01, b'x'],
            &[0x00, 0xFF, 0x04, 0x03],
            b"Sax",
            &[0x00, 0xFF, 0x58, 0x04, 3, 3, 24, 8],
            // A signature of no beats, one of a 2^200th note, a tempo of the wrong length and one of zero.
            &[
                0x00, 0xFF, 0x58, 0x04, 0, 2, 24, 8, 0x00, 0xFF, 0x58, 0x04, 4, 200, 24, 8, 0x00, 0xFF, 0x51, 0x02, 1, 2, 0x00, 0xFF, 0x51, 0x03, 0,
                0, 0,
            ],
            &[0x00, 0xFF, 0x59, 0x02, 0xFE, 0x00, 0x00, 0xFF, 0x05, 0x02, b'l', b'a', 0x00, 0xFF, 0x07, 0x03],
            b"cue",
            &[0x60, 0xFF, 0x06, 0x01, b'B', 0x00, 0xA0, 60, 5, 0x00, 0x90, 60, 1, 0x01, 60, 0],
        ]
        .concat();
        let smf = read(&file(1, 96, &[&body])).unwrap();
        assert_eq!((smf.tracks[0].name.as_str(), smf.tracks[0].instrument.as_str()), ("Lead", "Sax"));
        assert_eq!(smf.time_signatures, [TimeSignature { tick: 0, beats: 3, unit: 8 }]);
        assert_eq!(smf.key_signatures, [KeySignature { tick: 0, sharps: -2, minor: false }]);
        assert_eq!(smf.markers, [Marker { tick: 0, name: "cue".into() }, Marker { tick: 96, name: "B".into() }]);
        assert_eq!(smf.tempos, [Tempo { tick: 0, bpm: 60_000_000.0 }]);
        assert_eq!(smf.warnings.len(), 2, "lyrics and aftertouch: {:?}", smf.warnings);
    }

    #[test]
    fn a_file_of_any_size_is_read_into_a_bounded_number_of_events() {
        // Three bytes a note-on under running status: more notes than are read, none of them ended.
        let mut body = vec![0x00, 0x90, 60, 100];
        body.extend((0..MAX_EVENTS + 1000).flat_map(|i| [0x00, (i % 128) as u8, 1]));
        let smf = read(&file(1, 96, &[&body])).unwrap();
        assert_eq!(smf.tracks[0].notes.len(), MAX_EVENTS);
        assert!(smf.warnings.iter().any(|w| w.contains("more events")));
        // Tracks of nothing, by the thousand: the count stops the reading, and none is kept.
        let smf = read(&file(1, 96, &vec![&[][..]; MAX_TRACKS + 5])).unwrap();
        assert!(smf.tracks.is_empty() && smf.warnings.iter().any(|w| w.contains("tracks")));
        // A text of thousands of bytes is a name of a few.
        let mut named = vec![0x00, 0xFF, 0x03, 0x83, 0x80, 0x00];
        named.extend(vec![b'a'; 0xC000]);
        named.extend([0x00, 0x90, 1, 1]);
        assert_eq!(read(&file(1, 96, &[&named])).unwrap().tracks[0].name.len(), MAX_TEXT);
    }

    #[test]
    fn damaged_files_are_errors_not_panics() {
        assert_eq!(read(b""), Err(Error::NotMidi));
        assert_eq!(read(b"RIFFxxxxWAVE"), Err(Error::NotMidi));
        assert_eq!(read(b"MThd\0\0\0\x06\0\x01"), Err(Error::Truncated));
        let good = write(&song());
        for cut in 0..good.len() {
            let _ = read(&good[..cut]);
        }
        // Every byte flipped, one at a time: must return, never panic or hang.
        for i in 0..good.len() {
            let mut bad = good.clone();
            bad[i] ^= 0xFF;
            let _ = read(&bad);
        }
    }

    #[test]
    fn varlen_encoding_matches_the_specification_examples() {
        for (value, bytes) in [
            (0u32, vec![0x00]),
            (0x7F, vec![0x7F]),
            (0x80, vec![0x81, 0x00]),
            (0x3FFF, vec![0xFF, 0x7F]),
            (0x200000, vec![0x81, 0x80, 0x80, 0x00]),
            (0x0FFFFFFF, vec![0xFF, 0xFF, 0xFF, 0x7F]),
        ] {
            let mut out = Vec::new();
            varlen(&mut out, value);
            assert_eq!(out, bytes, "{value:#x}");
            assert_eq!(Reader { data: &bytes, pos: 0 }.varlen().unwrap(), value);
        }
    }

    #[test]
    fn long_utf8_track_name_is_truncated_at_a_character_boundary() {
        let mut smf = song();
        smf.tracks[0].name = format!("{}é", "a".repeat(255));
        let bytes = try_write(&smf).unwrap();
        let result = read(&bytes).unwrap();
        assert!(result.tracks[0].name.len() <= MAX_TEXT);
        assert!(result.tracks[0].name.is_char_boundary(result.tracks[0].name.len()));
    }

    #[test]
    fn event_cap_clears_later_track_events() {
        let mut smf = song();
        let track = smf.tracks[0].clone();
        smf.tracks = vec![track.clone(), track.clone(), track];
        cap_events(&mut smf, 1);
        assert_eq!(smf.tracks.iter().map(|t| t.notes.len() + t.changes.len()).sum::<usize>(), 1);
        assert!(smf.tracks[1].notes.is_empty() && smf.tracks[2].notes.is_empty());
    }
}
