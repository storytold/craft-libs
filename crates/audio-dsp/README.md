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
  dither, a signal generator, a loudness meter, a pitch shifter for a live voice with 10 ms of
  latency, a formant shifter for a live voice with 10.7 ms of latency, and noise suppression for
  a live voice with 5.3 ms of latency that follows the noise floor without first learning it.
  These latencies are at 48 kHz (the STFT sizes depend on the sample rate; `latency()` reports
  them in samples). The pitch shifter's Keep Formants switch adds the formant shifter's latency,
  and `latency()` reports it while the switch is on. A ring modulator for robotic and metallic
  voices (ring, frequency modulation, and positive or negative comb resonators, with the
  frequency moved by an LFO, the input's level or a sample-and-hold source) adds no latency, and
  neither does a bitcrusher that takes one sample in every 1 to 50 and rounds it to 1 to 16 bits.
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

`noise_suppression` estimates the noise floor from minimum spectral power over 1.5 s. The first
noise estimate after a reset starts from the median bin's power. Each bin's value is clamped
between 30 dB below its own estimate and its own estimate. Windows that overlap a run of at
least one hop of zeros (64 samples at 48 kHz) do not enter the noise estimate. The gain floor
moves from 0 dB toward the attenuation set by `reduction` during the first search window
of sound. Continuous voiced or tonal audio present from startup can enter the noise estimate
and be attenuated afterward.

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
- [storytold/soundcraft](https://github.com/storytold/soundcraft) `crates/dsp` at
  `915481be1b49d7e0502e07b059d0c0198c61911a`: the two crossfaded read taps of the voice pitch
  shifter (`src/plugins/pitch.rs`) and its YIN pitch tracker (`src/pitch_detect.rs`), and the
  ring modulator (`src/plugins/modulation.rs`).

`NOTICE` keeps the copyright notices of both projects.

## License

MIT OR Apache-2.0.

Copyright (c) 2026 ArtCraft Team and the FilmCraft contributors.
Copyright (c) 2026 ArtCraft Team and the SoundCraft contributors.
Copyright (c) 2026 The Craft contributors.
