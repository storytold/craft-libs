# craft-sf2

SoundFont 2 (`.sf2`) files, read. Bytes in, a neutral model out; no file access, no
dependencies beyond `thiserror`, builds for wasm32.

```rust
let bytes = craft_sf2::fixture::build(&[], &[], 1); // any .sf2 file's bytes
match craft_sf2::read(&bytes) {
    Ok(font) => println!("{}: {} presets, {} frames", font.name, font.presets.len(), font.pool.len()),
    Err(e) => println!("{e}"),
}
```

(The example needs the `test-support` feature for `fixture`.)

## The model

- `SoundFont`: the file's name, the sample pool (every sample end to end, mono `f32` at full
  scale ±1) and the presets, ordered by bank and program.
- `Preset`: name, bank, program and its `Region`s.
- `Region`: what one preset zone and one instrument zone sound together: key and velocity range,
  the pool frames it plays and loops over, the recorded sample rate, the fractional root key
  (with every tuning applied), pan, linear gain and a volume `Envelope`.

Resolving zones is done here, as the specification describes: a first zone without an
instrument (or sample) is global, instrument generators are absolute, preset generators are
added to them, and key and velocity ranges intersect.

Honoured: keyRange, velRange, instrument, sampleID, overridingRootKey, coarseTune, fineTune (with
the sample's pitch correction), sampleModes (loop or not), the start, end and loop address
offsets (fine and coarse), pan, initialAttenuation and the volume envelope's attack, decay,
sustain and release. Ignored: modulators, the filter, the modulation envelope, both LFOs, the
chorus and reverb sends, the volume envelope's delay, hold and key scaling, scaleTuning,
exclusiveClass, the fixed key and velocity, the 24-bit extension (`sm24`) and ROM samples.

## Limits

Input is hostile. Every offset is checked against the file; a file whose chunks run past its
end is an error, not played in part. The pool is read up to `MAX_POOL_FRAMES` (2^28) frames and
regions are clamped to it. Reading stops after 2^24 generators and a file may make at most 65,536
regions (`Error::TooManyZones`), so a file that points every preset zone at the same long lists
cannot multiply itself out. A font with nothing that can sound is `Error::Empty`.

## Features

- `test-support`: `fixture::build`, a writer of small valid SoundFont 2 files, for the tests of
  crates that read SoundFonts. Not a general SoundFont writer.

## Sources and clean room

Written from the SoundFont 2.04 Technical Specification (E-mu Systems / Creative Technology,
2006). No other product's code or data was used. The code comes from LoopCraft's
`loopcraft-audiofile` crate; the tests build every file they read in code, so the crate ships no
binary test data.
