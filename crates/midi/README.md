# craft-midi

MIDI file formats. Bytes in, a neutral model out, and back. No file access: callers pass bytes,
so the crate builds for wasm32.

| Format | Read | Write |
|---|---|---|
| Standard MIDI File (`.mid`, `.midi`), formats 0 and 1 | yes | format 1 |
| Standard MIDI File, format 2 | best effort (all tracks share one timeline) | no |

RIFF-wrapped MIDI (`.rmi`), System Exclusive dumps (`.syx`) and karaoke lyrics are not here; no
app reads or writes them.

## The neutral model

- `Note`: pitch, note-on velocity, release velocity, channel, start and length in ticks.
- `CtrlEvent`: a channel event at a tick: control change, pitch bend (centred, -8192..=8191),
  channel pressure, polyphonic pressure or program change.
- `Sequence`: the notes and channel events of one track, with sorting, range queries and
  duplicate removal.
- `Smf`: the file: its tracks (`SmfTrack`: a name and a `Sequence`) and the file-wide tempo,
  meter, marker and key-signature lists.

All ticks are at `PPQ` (960) per quarter note, whatever the file's division. The `serde` feature
derives `Serialize` and `Deserialize` on `Note`, `CtrlEvent`, `CtrlKind` and `Sequence`.

## Reading

`read_smf(bytes)`:

- converts the file's division to 960 ticks per quarter note; an SMPTE division (24, 25, 29.97 or
  30 frames per second) is approximated at 120 bpm;
- pairs note-ons with note-offs: running status, note-on with velocity 0 as note-off, overlapping
  notes of one pitch first in first out, an unmatched note-off ignored, a note still sounding at
  the end of its track closed there, a zero-length note given one tick;
- folds the leading conductor track of a format-1 file (no notes, no channel events) into the
  file-wide lists; its name is not kept;
- skips unknown chunks, unknown meta events and System Exclusive events;
- accepts a header track count that differs from the number of `MTrk` chunks and reads every
  chunk present.

Input is hostile. Every length is checked against the data that is left, a variable-length
quantity is at most four bytes, and a file is refused beyond `MAX_TRACKS` (4096) track chunks or
`MAX_EVENTS` (5,000,000) events. Malformed structure (bad header, truncated chunk, a data byte
without running status, a system real-time status in a track) is a `MidiError`, never a panic.

## Writing

`write_smf(&smf)` writes format 1 at 960 ticks per quarter note: a conductor track with tempos,
meters, key signatures and markers, then one track per `SmfTrack`. Negative ticks are clamped to
0 and data bytes to their MIDI ranges; a note-on velocity of 0 is written as 1. Notes of one
pitch that overlap are trimmed so that the file reads back with the same starts, and of two that
start together the longer is written. An invalid tempo, a meter denominator that is not a power
of two, more than 65535 tracks or a gap beyond the 28-bit delta limit is an error.

## Sources and clean room

Written from the public Standard MIDI Files 1.0 specification (the MIDI Association). No other
product's code, assets or strings were used. The code comes from SoundCraft's `soundcraft-midi`
crate; the tests build every file they read in code, so the crate ships no binary test data.
