# craft-audio-dsp

Audio DSP for the Craft apps, in Rust with no dependencies. It builds for `wasm32-unknown-unknown`
and contains no `unsafe` code.

## Contents

- `AudioEffect`: the trait every effect implements, and `effects()`, the registry that lists each
  effect's id, name, category and parameters (range, unit, default). `create_effect(id, rate,
  channels)` builds one by id.
- Effects: equalizers (parametric, graphic, notch, scientific and FFT filters), dynamics
  (compressor, expander and gate, limiter, maximizer, multiband and tube-modeled compressors),
  delays and reverbs (including convolution and surround), modulation, restoration (denoise,
  dehum, click remover, de-esser, de-reverb, speech enhancement), channel tools, distortion,
  dither, a signal generator and a loudness meter.
- `loudness`: an ITU-R BS.1770-4 / EBU R128 meter with momentary, short-term and integrated
  loudness, loudness range (EBU Tech 3342), sample peak and true peak.
- `pan`: stereo pan laws, a surround panner over any layout of up to 16 speakers, and first-order
  ambisonic encoding and decoding.
- `channels`: channel layouts, ITU-R BS.775 upmix and downmix, and the 5.1 panner.
- `generator`: band-limited sine, square, saw and triangle oscillators, and white and pink noise.
- `resample`: streaming rate conversion for playback. `offline::resample` converts whole buffers.
- `stretch`: time stretch that keeps the pitch.
- `sync`: the offset between two recordings of one event.
- `ducking`: speech-activity regions in a trigger track and the volume keyframes that lower a
  music track under them.
- `biquad` and `design`: biquad filters with their magnitude response, and IIR filter design from
  Butterworth, Bessel, Chebyshev and elliptic prototypes.

## Conventions

Audio is planar `f32`: one slice per channel, all the same length. `AudioEffect::process`,
`process_keyed`, `set_param` and `reset` do not allocate; every buffer is sized when the effect is
created. When a parameter changes, gains, mix amounts and equalizer bands move to the new value
per sample. Other parameters, such as a compressor's threshold, ratio, knee and times or a
limiter's ceiling and release, take effect from the next sample. With the parameters held still,
the output does not depend on the block size. The convolution reverb is the one effect whose
`process` call can take much longer than a block: after a change to its impulse, room size or
damping, the next call rebuilds the impulse's partitions in buffers sized when the effect was
created. Non-finite input samples do not leave an effect's state at NaN.

Constructors clamp the sample rate to `MIN_SAMPLE_RATE ..= MAX_SAMPLE_RATE` (1 kHz to 1.536 MHz,
with 48 kHz for a non-finite rate) and the channel count to `1 ..= MAX_CHANNELS` (64).

Compressor, gate and limiter-type effects report their gain reduction in dB as a non-negative
number through `AudioEffect::gain_reduction_db`. The compressor and the gate take a key
(sidechain) input of up to two channels through `AudioEffect::process_keyed`.

```rust
use craft_audio_dsp::{AudioEffect, effects::Compressor, loudness::LoudnessMeter};

let mut comp = Compressor::new(48_000.0, 2);
comp.set_param("threshold", -24.0);
let (mut left, mut right) = (vec![0.0f32; 1024], vec![0.0f32; 1024]);
comp.process(&mut [&mut left, &mut right]);

let mut meter = LoudnessMeter::new(48_000.0, 2);
meter.process(&[&left, &right]);
let _integrated = meter.integrated();
```

## API stability

The items the crate exports are its API. Removing or changing one is a breaking change and bumps
the major version (the minor version while it is below 1.0). Effect ids and parameter ids are part
of the API, because hosts save them in their files.

## Origin

The crate combines two crates from the ArtCraft apps, with changes:

- [storytold/filmcraft](https://github.com/storytold/filmcraft) `crates/audio-dsp` at
  `3f137a7ed420cac79b32145f5243b7f98b31f71c`: the `AudioEffect` trait, the effects and their
  registry, the loudness meter, the streaming resampler, time stretch, offset search and ducking.
- [storytold/soundcraft](https://github.com/storytold/soundcraft) `crates/dsp` at
  `648ed9db9a0e98cc55643babffac381521a556c8`: the tail length, offline mode, events, saved state
  and editor parts of the trait, the maximizer, dither, panning, the signal generator and the
  offline resampler.

`NOTICE` keeps the copyright notices of both projects.

## License

MIT OR Apache-2.0.

Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors.
Copyright (c) 2026 ArtCraft Team and the SoundCraft contributors.
Copyright (c) 2026 The Craft contributors.
