# craft-midi

MIDI file formats. Bytes in, a neutral model out, and back. No file access: callers pass bytes,
so the crate builds for wasm32.

| Format | Read | Write |
|---|---|---|
| Standard MIDI File (`.mid`, `.midi`), formats 0 and 1 | yes | format 0 or 1 |
| Standard MIDI File, format 2 | best effort (all tracks share one timeline) | no |

RIFF-wrapped MIDI (`.rmi`), System Exclusive dumps (`.syx`) and karaoke lyrics are not here; no
app reads or writes them.

## The neutral model

- `Note`: pitch, note-on velocity, release velocity, channel, start and length in ticks.
- `CtrlEvent`: a channel event at a tick: control change, pitch bend (centred, -8192..=8191),
  channel pressure, polyphonic pressure or program change.
- `Sequence`: the notes and channel events of one track, with sorting, range queries and
  duplicate removal.
- `Smf`: the file: its format, its tracks (`SmfTrack`: a name and a `Sequence`) and the
  file-wide tempo, meter, marker and key-signature lists. `Smf::default()` is an empty format-1
  file.

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

## Reading a damaged file

`read_smf_lenient(bytes)` returns a `LenientRead`: the `Smf`, a list of warnings (at most
`MAX_WARNINGS`, 32), the Instrument Name of each track and `end`, the last tick any track
reaches (its End of Track). It does not fail on damage after the header. Only a file that is not a Standard MIDI File
(no `MThd`, a header cut short, an unusable division) is an error. Otherwise:

- a header length beyond the file is ignored; an unknown format is read as format 1;
- a track whose events stop making sense (cut short, a data byte without a status, a system
  message, the event limit reached) keeps what came before, and its open notes end there; a
  track without End of Track is kept with a warning;
- a track length beyond the file reads the rest of it; a length that disagrees with the track's
  End of Track is not believed, and the next chunk is looked for where the events ended;
- bytes that are no chunk are skipped to the next `MTrk`; trailing bytes are ignored;
- past `MAX_TRACKS` the rest is left out; lyrics are counted in a warning; a format-2 file is
  read side by side, with a warning saying so.

On a sound file both readers return the same `Smf` and the lenient one no warnings. The lenient
rules come from LoopCraft's reader.

## Writing

`write_smf(&smf)` writes format 1 at 960 ticks per quarter note: a conductor track with tempos,
meters, key signatures and markers, then one track per `SmfTrack`. When `smf.format` is 0 it writes format 0: all of that in one track. Negative ticks are clamped to
0 and data bytes to their MIDI ranges; a note-on velocity of 0 is written as 1. Notes of one
pitch that overlap are trimmed so that the file reads back with the same starts, and of two that
start together the longer is written. An invalid tempo, a meter denominator that is not a power
of two, more than 65535 tracks or a gap beyond the 28-bit delta limit is an error.

## Sources and clean room

Written from the public Standard MIDI Files 1.0 specification (the MIDI Association). No other
product's code, assets or strings were used. The code comes from SoundCraft's `soundcraft-midi`
crate; the tests build every file they read in code, so the crate ships no binary test data.
