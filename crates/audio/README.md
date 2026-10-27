# craft-audio

Audio file I/O and codecs for the Craft apps. Native implementations: WAV and AIFF read/write, FLAC encoder, AAC
decoder/encoder, MP1/MP2 decoder, AC-3/E-AC-3 decoder, Opus decoder/encoder. Formats without a
native decoder (MP3, FLAC, ALAC, Vorbis, CAF, ADPCM, A-law, µ-law) are decoded by
[symphonia](https://github.com/pdeljanov/Symphonia), which is to be replaced by native decoders.

- Input and output are byte slices; no filesystem access.
- Decoded audio is planar `f32`, full scale ±1, in `AudioBuffer` (`sample_rate`, one `Vec<f32>`
  per channel), in WAV/SMPTE channel order (L R C LFE, back, side).
- Malformed input returns an error, never a panic. No `unsafe`. Readers and decoders are
  mutation-fuzzed.
- Implemented from ISO/IEC, ATSC and IETF specifications; tested against ffmpeg and conformance
  vectors. Builds for `wasm32-unknown-unknown`.
- Containers (MP4/MOV, Matroska/WebM, Ogg) and bit I/O come from
  [`craft-media`](../media/README.md). This crate never depends on video code.

## Format support

| Format | Decode | Encode |
|---|---|---|
| WAV, BWF, RF64/BW64 | native | native |
| AIFF, AIFF-C | native | native |
| FLAC | symphonia | native |
| AAC-LC | native | native |
| HE-AAC v1/v2 | native, AAC-LC core only (see [Limits](#limits)) | — |
| MP1, MP2 (MPEG audio Layer I/II) | native | — |
| MP3 (Layer III) | symphonia | — |
| AC-3, E-AC-3 | native | — |
| Opus | native | native |
| ALAC, Vorbis, ADPCM, A-law, µ-law | symphonia | — |

| Container | Demuxer | Timing |
|---|---|---|
| MP4, M4A, MOV | `craft_media::isobmff` | edit lists (AAC priming, Opus pre-skip) |
| Matroska, WebM | `craft_media::matroska` | `CodecDelay`, `DiscardPadding` |
| Ogg | `craft_media::ogg` (Opus); symphonia (Vorbis) | pre-skip, end trimming |
| Raw ADTS AAC, MP1/MP2, AC-3, E-AC-3 | native frame sync | — |
| CAF | symphonia | — |

`encode` writes WAV, AIFF and FLAC. The AAC and Opus encoders output packets; muxing them into
MP4 or Matroska is done with the `isobmff` and `matroska` muxers.

## Cargo features

No default features. The shared types (`AudioBuffer`, `AudioInfo`, `AudioError`, `EncodeOptions`,
…) build with none; their only dependency is `thiserror`.

| Feature | Provides | Dependencies |
|---|---|---|
| `decode` | `decode`, `probe`, `detect_format`; decoders in `aac`, `mpa`, `ac3`, `opus` | craft-media (`bitstream`, `isobmff`, `matroska`, `ogg`), `symphonia`, `log` |
| `encode` | `encode`, `FileEncoder`, `aac::Encoder`, `opus::encoder::OpusEncoder` | craft-media (`bitstream`) |
| `peaks` | `peaks::Peaks` | none |
| `rayon` | parallel AAC encoding | `rayon` |
| `serde` | `Serialize`/`Deserialize` on the shared types | `serde` |

`encode` does not depend on symphonia or the demuxers. Encode only:

```toml
craft-audio = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["encode"] }
```

Decode, encode and waveform overviews:

```toml
craft-audio = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["decode", "encode", "peaks"] }
```

craft-media comes in as a dependency of craft-audio; list it separately only to use the containers
directly.

## Codecs

### WAV and AIFF

Source: SoundCraft `soundcraft-audio-io`.

- Read: WAV PCM 8/16/24/32-bit, float 32/64-bit, `WAVE_FORMAT_EXTENSIBLE` (including Ambisonic
  subtypes), RF64/BW64, BWF `bext`, truncated files (data chunk size past end of file). AIFF and
  AIFF-C `NONE`, `twos`, `sowt`, `fl32`, `fl64`. Other WAV/AIFF codecs go to symphonia.
- Write: WAV/BWF 16/24/32-bit integer or 32-bit float; RF64 above 4 GiB (`encode_wav_rf64` forces
  it); any speaker mask (`encode_with_channel_mask`); optional `bext`. AIFF at integer depths,
  AIFF-C `fl32` for float.
- Integer output clamps to ±1, optional TPDF dither. Float output keeps values above full scale.

### FLAC encoder

Source: FilmCraft `filmcraft-flac`. Spec: RFC 9639.

- 16, 24 or 32 bits; 1–8 channels; up to 655,350 Hz.
- 4096-frame blocks. Per channel: smallest of constant, verbatim, fixed polynomial or LPC
  subframes, partitioned Rice residuals. Stereo also tries left/side, side/right, mid/side.
- STREAMINFO includes frame sizes and the audio MD5.
- Output verified bit-exact with ffmpeg and the reference `flac` decoder.

### AAC (`aac`)

Source: FilmCraft `filmcraft-aac`. Spec: ISO/IEC 14496-3.

- Decoder: AAC-LC; M/S and intensity stereo, PNS, TNS, pulse data, sine and KBD windows, all
  window sequences; 1–8 channels. HE-AAC v1/v2: AAC-LC core only.
- Encoder: AAC-LC; psychoacoustic model, window switching, M/S, TNS, bit reservoir; CBR or VBR;
  1–8 channels. Outputs raw access units and the `AudioSpecificConfig`, or ADTS frames.
- SNR against ffmpeg's decoder: ~135 dB. With PNS, output differs per sample from ffmpeg (PNS
  noise is decoder-generated) at equal band energy.

### MP1/MP2 (`mpa`)

Source: FilmCraft `filmcraft-mpa`. Spec: ISO/IEC 11172-3, 13818-3.

- Layer I and II; mono, stereo, dual channel, joint stereo; all MPEG-1, MPEG-2 and MPEG-2.5 bit
  rates and sample rates (8–48 kHz).
- Accepts Layer II bit rate/mode combinations the spec disallows (e.g. ffmpeg's default mono
  384 kbit/s).
- Not supported: Layer III (MP3), free-format bit rate.

### AC-3/E-AC-3 (`ac3`)

Source: FilmCraft `filmcraft-ac3`. Spec: ATSC A/52:2012.

- AC-3: all channel modes (1/0 to 3/2, ±LFE), 32/44.1/48 kHz, all frame sizes, coupling,
  rematrixing, dynamic range control, block switching.
- E-AC-3: independent substream 0, 1–6 blocks per frame, reduced sample rates, spectral
  extension.
- Output in WAV order (L R C LFE Ls Rs), no downmix.

### Opus (`opus`)

Source: decoder FilmCraft `filmcraft-opus`, encoder EffectCraft `effectcraft-opusenc`. Spec:
RFC 6716, RFC 8251, RFC 7845.

- Decoder: SILK, hybrid and CELT; all frame sizes; packet-loss concealment and FEC; multistream
  (`Decoder`) and single stream (`StreamDecoder`); output at 8/12/16/24/48 kHz. Passes all 36 RFC
  8251 conformance cases with matching range-coder state after every packet.
- Encoder: SILK, hybrid and CELT, selected from bit rate and `Application` (`Audio`, `Voip`) or set
  explicitly; 48 kHz, 20 ms frames, mono or stereo. Outputs packets and the `OpusHead`.

## symphonia

symphonia 0.6 (MPL-2.0, used unmodified as the Craft licensing rules require) decodes the formats
without a native decoder:

| Format | Decoder |
|---|---|
| MP3 | symphonia |
| FLAC decoding | symphonia |
| ALAC | symphonia |
| Vorbis | symphonia |
| CAF | symphonia |
| ADPCM, A-law, µ-law (WAV, AIFF, CAF) | symphonia |

All of these are to be replaced by native decoders, after which the symphonia dependency is
removed. AAC, MP2, AC-3 and Opus were previously decoded by symphonia in the apps and are now
native.

`decode` and `probe` try the native path (`read/native.rs`) first and fall back to symphonia
(`read/symph.rs`). Adding a native decoder therefore needs no API change. Removing symphonia means
deleting `read/symph.rs`, the dependency and the profile override below.

Notes while symphonia is in use:

- symphonia allocates sizes taken from the file. Container size fields of WAV, FLAC, MP4 and CAF
  files are checked against the file length before symphonia reads them. A panic inside symphonia
  is caught and returned as `AudioError::Malformed`.
- `symphonia-bundle-flac` overflows an `i32` add on 32-bit LPC FLAC; the wrapped value is correct.
  Release builds decode correctly; debug builds panic (returned as `Malformed`). Apps that decode
  32-bit FLAC need this in the workspace `Cargo.toml` (craft-libs has it):

  ```toml
  [profile.dev.package.symphonia-bundle-flac]
  overflow-checks = false
  ```

## Usage

### Decode a file

```rust
use craft_audio::{decode, detect_format, probe};

let bytes = std::fs::read("take.m4a")?;

// What is it? Magic bytes first, then the extension as a fallback.
let format = detect_format(&bytes, Some("m4a"));

// Format, sample rate, channels and length, without decoding the whole file.
let info = probe(&bytes, Some("m4a"))?;
println!("{:?} {} Hz, {} ch, {:.1} s", info.format, info.sample_rate, info.channels, info.duration_secs());

// The first audio track, as planar f32 in WAV channel order, trimmed to its presentation length.
let (info, buf) = decode(&bytes, Some("m4a"))?;
let left: &[f32] = &buf.channels[0];
```

The extension hint is optional (`None`). `decode` handles native and symphonia formats.

### Encode a file

```rust
use craft_audio::{BitDepth, EncodeOptions, FileFormat, encode};

let opts = EncodeOptions { format: FileFormat::Flac, bit_depth: BitDepth::Int24, dither: true, bwf: None };
let flac: Vec<u8> = encode(&buf, &opts)?;
std::fs::write("take.flac", flac)?;
```

`EncodeOptions::default()` is 24-bit WAV. `encode_format_for("aiff")` maps an extension to a
`FileFormat`; `ENCODE_EXTENSIONS` lists the supported ones.

`FileEncoder` writes the same output incrementally to any `Write + Seek`:

```rust
use craft_audio::{EncodeOptions, FileEncoder};

let file = std::fs::File::create("recording.wav")?;
let mut enc = FileEncoder::new(file, &EncodeOptions::default(), 2, 48_000, None)?;
for block in blocks {          // each block: one Vec<f32> per channel
    enc.write(&block, block[0].len())?;
}
enc.finish()?;                 // fills in the header sizes
```

After a failed write, all later calls return an error.

### Codec modules

For packets from a demuxer, a stream or an encoder. Decoders return planar `f32`.

AAC from ADTS, or from an MP4 `AudioSpecificConfig`:

```rust
use craft_audio::aac::{Decoder, split_adts};

let frames = split_adts(&adts_bytes)?;
let mut dec = Decoder::from_adts(&frames[0].0)?;
for (_header, au) in &frames {
    let pcm = dec.decode(au)?;   // 1024 samples per channel
}

let mut dec = Decoder::new(&asc)?;  // asc: the bytes from the MP4 `esds` box
```

AAC encoding for MP4:

```rust
use craft_audio::aac::{Encoder, EncoderConfig};

let mut enc = Encoder::new(EncoderConfig::cbr(48_000, 2, 192_000))?;
let mut units = enc.encode(&[&left, &right]);  // zero or more access units
units.extend(enc.flush());
let asc = enc.audio_specific_config();         // for the `esds` box
let priming = enc.priming_samples();           // for the edit list (1024)
```

MP1/MP2 and AC-3/E-AC-3 decode one frame per call; `parse_header` returns the frame length:

```rust
use craft_audio::{ac3, mpa};

let mut dec = mpa::Decoder::new();
let mut rest: &[u8] = &mp2_bytes;
while let Ok(header) = mpa::parse_header(rest) {
    let Some(frame) = rest.get(..header.frame_bytes) else { break };
    let decoded = dec.decode(frame)?;   // decoded.channels: 384 or 1152 samples each
    rest = &rest[header.frame_bytes..];
}

let mut dec = ac3::Decoder::new();
let decoded = dec.decode(ac3_frame)?;  // decoded.channels, already in WAV order
```

Opus decoding (Ogg, MP4 or WebM track) and encoding:

```rust
use craft_audio::opus::{Decoder, encoder::OpusEncoder};

let mut dec = Decoder::new(&opus_head)?;  // the track's OpusHead bytes
dec.set_trim_pre_skip(true);
let pcm = dec.decode(Some(&packet))?;     // decode(None) conceals a lost packet

let mut enc = OpusEncoder::new(2, 128_000)?;
let frame = vec![0.0f32; OpusEncoder::FRAME_SIZE * 2];  // 20 ms, interleaved
let packet = enc.encode_float(&frame)?;
let head = enc.opus_head();               // for the container
```

### Channel order

`decode` returns WAV order. The AAC and Opus decoders return the codec's channel order for
surround; `aac::wav_order(channel_config)` and `opus::wav_order(channels)` give the decoded channel
index for each WAV-order output channel:

```rust
let order = craft_audio::opus::wav_order(pcm.len());
let wav: Vec<Vec<f32>> = match order {
    Some(order) => order.iter().map(|&i| pcm[i].clone()).collect(),
    None => pcm,   // mono, stereo and unusual layouts keep their order
};
```

AC-3 output is already in WAV order.

### Waveform overviews

`peaks`: a min/max/sum-of-squares pyramid of one channel (32-sample blocks, fan-out 4).
Range queries are exact, O(log n).

```rust
use craft_audio::peaks::Peaks;

let peaks = Peaks::build(&buf.channels[0]);
let (min, max) = peaks.minmax(&buf.channels[0], 0, 48_000);   // the first second
```

`spans` and `rms_spans` return one value per display column.

### Errors

Functions return `craft_audio::Result<T>`; the codec modules have their own `Error` types.

| `AudioError` | Cause |
|---|---|
| `Unsupported` | valid input using an unsupported codec or option |
| `Malformed` | corrupt or truncated input, or not the claimed format |
| `TooLarge` | size limit exceeded (decode limit: 2³⁰ samples) |
| `Encode` | encoder rejected the input (e.g. empty buffer) |
| `Io` | the `FileEncoder` writer failed |

## Limits

- HE-AAC: the AAC-LC core is decoded and upsampled 2×. SBR is not reconstructed, so there is no
  content above the core's Nyquist frequency.
- E-AC-3: 7.1 programs decode as their 5.1 core. Adaptive hybrid transform and enhanced coupling
  return `Unsupported`.
- AC-3: matches ffmpeg at 25–45 dB SNR on noisy input, not bit-exact; zero-bit mantissa dither is
  decoder-specific (A/52 §7.3.4).
- WebM Opus: `probe` can over-report by up to one block; `decode` is exact.
- `FileFormat`: AC-3, E-AC-3, Opus in MP4/WebM and PCM in MOV/MKV report `Other`; MP1/MP2 report
  `Mp3`.

## Tests

- `tests/audio/`: SoundCraft's tests. Round trips through every format and depth, streaming
  against whole-buffer encoding, float headroom, empty WAV data chunks, hostile input for every
  reader (including the size checks before symphonia), `serde` round trips.
- `src/write/flac/oracle_tests.rs`: FilmCraft's FLAC tests. Every signal, layout, depth, level and
  block size decodes in ffmpeg to the input samples; ffmpeg's MD5 matches STREAMINFO.
- Unit tests in `src/{aac,mpa,ac3,opus}`.
- `tests/codecs/` (one test binary; `testkit.rs` replaces `filmcraft-testkit`): AAC decoder and
  encoder against ffmpeg, mutated access units and configs, MP1/MP2 and AC-3 decoders against
  ffmpeg-encoded streams, Opus decoder against libopus and the RFC 8251 vectors, Opus encoder
  through ffmpeg's two Opus decoders.
- `tests/codecs/routing_oracle.rs`: 19 container/codec pairs written by ffmpeg go through
  `decode`; format, rate, channels and exact presentation length must match, output must be
  sample-aligned with ffmpeg's decode within each codec's SNR bound, and truncated and corrupted
  copies must not panic.

ffmpeg is a test oracle only, never linked. Oracle tests use the same environment variables as
craft-media's: `CRAFT_MEDIA_FFMPEG` (then `PATH`, then common install directories),
`CRAFT_MEDIA_FIXTURES_DIR` (default `target/fixtures`), and `CRAFT_MEDIA_REQUIRE_ORACLES=1` to fail
instead of skip when ffmpeg is missing. CI: `.github/workflows/audio.yml`.

The Opus conformance test needs the RFC 8251 vectors (not committed, 75 MB) in
`target/fixtures/opus/vectors/`; it is skipped without them:

```bash
mkdir -p target/fixtures/opus/vectors && tar xzf opus_testvectors-rfc8251.tar.gz -C target/fixtures/opus/vectors --strip-components=1
```

Archive: <https://opus-codec.org/static/testvectors/opus_testvectors-rfc8251.tar.gz>.

## Origin

| Part | Source |
|---|---|
| WAV, AIFF, `decode`/`probe`, peaks | SoundCraft `soundcraft-audio-io` [7dc56a4](https://github.com/storytold/soundcraft/commit/7dc56a4dbeebf57eacece09536726c57844fc59d) |
| FLAC encoder, AAC, MP1/MP2, AC-3, Opus decoder, Ogg | FilmCraft [8fda2f2](https://github.com/storytold/filmcraft/commit/8fda2f232198c4c79ff9cfe5b6a347bb1375fa9e) |
| Opus encoder | EffectCraft `effectcraft-opusenc` [c618429](https://github.com/storytold/effectcraft/commit/c618429252587525a2a6d4b5bccbe4ec5261e1b9) |

Public item names are unchanged, so apps can re-export (`pub use craft_audio::aac::*;`).
Where apps had overlapping implementations, the one that measured better was kept, with the tests
of both:

| Format | Kept | Reason |
|---|---|---|
| FLAC encoding | FilmCraft | 0.1–37% smaller output than SoundCraft's on 12 test signals; STREAMINFO MD5 |
| WAV/AIFF writing | SoundCraft | only one with BWF, dither, arbitrary speaker masks, streaming without known length |
| AAC-LC, MP1/MP2 decoding | FilmCraft (over symphonia) | equal SNR against ffmpeg; also decodes 5.1 AAC and MP2 bit rates symphonia rejects |

Changes from the app versions:

- `serde` derives are behind the `serde` feature; decode, encode and peaks are behind their
  features.
- The FLAC encoder is used through SoundCraft's `encode`/`FileEncoder` API; vendor string
  `craft-audio <version>`.
- `OpusEncoder::new`, `with_application`, `with_mode` and `encode_float` return `Result` instead
  of panicking on invalid channel count, mode/bandwidth or frame length.
- `decode` uses native decoders before symphonia: AAC, MP2, AC-3 and Opus no longer go through
  symphonia, and MP4/Matroska files decode to presentation length (priming and padding removed).
