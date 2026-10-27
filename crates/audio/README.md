# craft-audio

Audio file I/O and codecs for the Craft apps. Native implementations: WAV and AIFF read/write,
FLAC encoder, AAC decoder/encoder, MP1/MP2 decoder, AC-3/E-AC-3 decoder, Opus decoder/encoder.
Formats without a native decoder (MP3, FLAC, ALAC, Vorbis, CAF, ADPCM, A-law, µ-law) are decoded
by [symphonia](https://github.com/pdeljanov/Symphonia), which is to be replaced by native decoders.

- Input is a byte slice or any [`craft_media::ByteSource`](../media/README.md) (a file, a shared
  buffer); output is byte vectors or any `Write + Seek`. The crate opens no files itself.
- Decoded audio is planar `f32`, full scale ±1, in `AudioBuffer` (`sample_rate`, one `Vec<f32>`
  per channel), in WAV channel order (L R C LFE, back, side) whenever the channel layout is known.
- Malformed input returns an error, never a panic (see [symphonia](#symphonia) for the one
  exception). No `unsafe`. Readers and decoders are mutation-fuzzed.
- Implemented from ISO/IEC, ATSC and IETF specifications; tested against ffmpeg and conformance
  vectors. Builds for `wasm32-unknown-unknown`.
- Containers (MP4/MOV, Matroska/WebM, Ogg), bit I/O and the `Container`/`CodecId`/`CodecParams`
  types come from [`craft-media`](../media/README.md). This crate never depends on video code.

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
| Ogg | `craft_media::ogg` (Opus); symphonia (Vorbis, FLAC) | pre-skip, end trimming |
| Raw ADTS AAC, MP1/MP2, AC-3, E-AC-3 | native frame sync | — |
| CAF | symphonia | — |

`lossless-encode` writes WAV, AIFF and FLAC. The AAC and Opus encoders output packets; muxing them into
MP4 or Matroska is done with craft-media's muxers.

## Cargo features

No default features. The shared types (`AudioBuffer`, `AudioInfo`, `ChannelLayout`, `AudioError`,
`EncodeOptions`, …) build with none.

| Feature | Provides | Pulls in |
|---|---|---|
| `decode` | `decode`, `probe`, `Decoder`, `tracks`, `TrackDecoder`; every format above | `decode-native`, `symphonia` |
| `decode-native` | the same, native formats only (others are `Unsupported`) | the four decoders below, craft-media `isobmff`/`matroska`/`ogg`, `log` |
| `symphonia` | the fallback for formats with no native decoder | `decode-native`, symphonia |
| `encode` | every encoder | `lossless-encode`, `aac-encode`, `opus-encode` |
| `lossless-encode` | `encode`, `FileEncoder`, `wav_float_header` (WAV, AIFF, FLAC) | craft-media `bitstream` |
| `aac-decode`, `aac-encode` | `aac::Decoder`, `aac::Encoder` | craft-media `bitstream` |
| `opus-decode`, `opus-encode` | `opus::Decoder`/`StreamDecoder`, `opus::encoder::OpusEncoder` | none |
| `mpa-decode`, `ac3-decode` | `mpa::Decoder`, `ac3::Decoder` | none / craft-media `bitstream` |
| `peaks` | `peaks::Peaks` | none |
| `rayon` | parallel AAC encoding (with `aac-encode`) | `rayon` |
| `serde` | `Serialize`/`Deserialize` on the shared types | `serde` |

WAV, AIFF and FLAC export only (no decoders, no demuxers, no symphonia):

```toml
craft-audio = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["lossless-encode"] }
```

Decode, every encoder and waveform overviews:

```toml
craft-audio = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["decode", "encode", "peaks"] }
```

Packets from an app's own demuxer, AAC both ways:

```toml
craft-audio = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["aac-decode", "aac-encode"] }
```

craft-media comes in as a dependency; list it separately only to use the containers directly.

## Codecs

### WAV and AIFF

Source: SoundCraft `soundcraft-audio-io`.

- Read: WAV PCM 8/16/24/32-bit, float 32/64-bit, `WAVE_FORMAT_EXTENSIBLE` (the speaker mask
  becomes `AudioInfo::layout`; Ambisonic subtypes are unordered), RF64/BW64, BWF `bext`, truncated
  files (data chunk size past end of file). AIFF and AIFF-C `NONE`, `twos`, `sowt`, `fl32`,
  `fl64`. Other WAV/AIFF codecs go to symphonia. Headers are parsed through the source, reading
  only chunk headers and small chunk bodies, and the samples stream.
- Write: WAV/BWF 16/24/32-bit integer or 32-bit float; RF64 above 4 GiB (`encode_wav_rf64` forces
  it); any layout (`encode_with_layout`, or `FileEncoder`'s `layout`); optional `bext`. AIFF at
  integer depths, AIFF-C `fl32` for float.
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
  1–8 channels. Outputs raw access units and the `AudioSpecificConfig`, or ADTS frames. Input is
  in AAC element order (centre first); `aac_order(channels)` maps WAV order to it.
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
- Output in WAV order (fronts, LFE, surrounds), no downmix.

### Opus (`opus`)

Source: decoder FilmCraft `filmcraft-opus`, encoder EffectCraft `effectcraft-opusenc`. Spec:
RFC 6716, RFC 8251, RFC 7845.

- Decoder: SILK, hybrid and CELT; all frame sizes; packet-loss concealment and FEC; multistream
  (`Decoder`) and single stream (`StreamDecoder`); output at 8/12/16/24/48 kHz. Passes all 36 RFC
  8251 conformance cases with matching range-coder state after every packet.
- Encoder: SILK, hybrid and CELT, selected from bit rate and `Application` (`Audio`, `Voip`) or set
  explicitly; 48 kHz, 20 ms frames, mono or stereo. Outputs packets and the `OpusHead`.
- `Bandwidth` is one type for both; the decoder's and encoder's `Mode` convert into each other.

## Reading files

The read side has three layers. Each uses the one below it.

| Layer | For | Memory |
|---|---|---|
| `decode`, `probe`, `decode_with`, `probe_with`, `tracks` | a file in memory, whole | the decoded audio |
| `Decoder` | streaming and seeking, from bytes or a `ByteSource` | one block at a time |
| `TrackDecoder` | packets from any demuxer, including an app's own | one packet at a time |

`Decoder` and the functions above it find the container from the bytes (the extension hint is
used only when the bytes do not say), read the track's packets with craft-media's demuxers,
decode them with `TrackDecoder`, and apply the container's timing so a file decodes to exactly its
presentation length. A packet that fails to decode is replaced by concealment of its length, so the
audio after it keeps its timing; a stream where nothing decodes is an error.

`AudioInfo` says what a track is: `container` and `codec` (`craft_media::Container`,
`craft_media::CodecId`; HE-AAC, MP1/MP2 and E-AC-3 are told apart), `sample_format`,
`sample_rate`, `channels`, `layout` (`ChannelLayout`: a WAV speaker mask, or unordered when the
channels have no speaker assignment), `frames` and BWF metadata. `AudioInfo`, `AudioError` and the
public enums are `#[non_exhaustive]`: new codecs, containers and fields are minor changes, and a
`match` on them needs a `_` arm.

## symphonia

symphonia 0.6 (MPL-2.0, used unmodified as the Craft licensing rules require) decodes the formats
without a native decoder:

| Format | Decoder |
|---|---|
| MP3 | symphonia |
| FLAC decoding | symphonia |
| ALAC | symphonia |
| Vorbis | symphonia |
| CAF (and AAC inside CAF) | symphonia |
| ADPCM, A-law, µ-law (WAV, AIFF, CAF) | symphonia |

All of these are to be replaced by native decoders, after which the symphonia dependency is
removed; FLAC and MP3 first, as the most common and because FLAC is also this crate's lossless
output format. AAC, MP2, AC-3 and Opus were previously decoded by symphonia in the apps and are now
native.

The native path (`read/native.rs`) is tried first and symphonia (`read/symph.rs`) is the fallback,
so a new native decoder takes its formats over without an API change. Removing symphonia means
deleting `read/symph.rs` and its size checks, the `symphonia` feature and the profile override
below. Build with `decode-native` instead of `decode` to see what still needs it.

Notes while symphonia is in use:

- symphonia reads from memory: a file it decodes is read whole first (its decoded audio still
  streams through `Decoder`).
- symphonia allocates sizes taken from the file. Container size fields of WAV, FLAC, MP4 and CAF
  files are checked against the file length before symphonia reads them.
- A panic inside symphonia is caught and returned as `AudioError::Malformed` only where panics
  unwind: native builds with the default `panic = "unwind"`. On `wasm32-unknown-unknown`, and in
  an app built with `panic = "abort"`, such a panic aborts. The native decoders do not panic.
- `symphonia-bundle-flac` overflows an `i32` add on 32-bit LPC FLAC; the wrapped value is correct.
  Release builds decode correctly; debug builds panic (returned as `Malformed`). Apps that decode
  32-bit FLAC need this in the workspace `Cargo.toml` (craft-libs has it):

  ```toml
  [profile.dev.package.symphonia-bundle-flac]
  overflow-checks = false
  ```

## Usage

Runnable versions of these are in `examples/` (`info`, `stream`, `packets`, `encode`), and each
public function has a rustdoc example.

### Decode a file

```rust
use craft_audio::{decode, probe};

let bytes = std::fs::read("take.m4a")?;

// Container, codec, rate, speaker layout and length, from the headers where they say.
let info = probe(&bytes, Some("m4a"))?;
println!("{} in {}, {} Hz, {}", info.codec.name(), info.container.name(), info.sample_rate, info.layout);

// The first audio track: planar f32 in WAV channel order, trimmed to its presentation length.
let (info, buf) = decode(&bytes, Some("m4a"))?;
let left: &[f32] = &buf.channels[0];
```

`decode` refuses a decode above 2³⁰ samples (4 GiB of `f32`). `decode_with` and `probe_with` take
`DecodeOptions`: another track (`with_track`, counted as `tracks` lists them) or another limit
(`with_max_samples`).

### Stream and seek

```rust
use craft_audio::{DecodeOptions, Decoder};

let file = std::fs::File::open("long-recording.m4a")?;
let mut dec = Decoder::open(file, Some("m4a"), &DecodeOptions::default())?;
dec.seek(48_000 * 60)?;                   // sample-exact: the next block starts here
while let Some(block) = dec.next_block()? {
    // block.channels: planar f32, WAV channel order
}
```

`Decoder::from_bytes` does the same for a buffer. `Decoder::info().frames` is 0 when the container
does not give the length (`length_known()` says); it is known after decoding.

### Decode packets from your own demuxer

`TrackDecoder` takes a track's `CodecParams` (from `isobmff::Track::codec_params`,
`matroska::Track::codec_params`, `ogg::Stream::codec_params`, or built with `CodecParams::new`
for another container) and the first packets, and returns `AudioBuffer`s in WAV order:

```rust
use craft_audio::{CodecId, CodecParams, TrackDecoder};

let params = CodecParams::new(CodecId::Aac).with_extradata(asc).with_sample_rate(48_000);
let lookahead: Vec<&[u8]> = packets.iter().take(craft_audio::LOOKAHEAD_PACKETS).map(Vec::as_slice).collect();
let mut dec = TrackDecoder::new(&params, &lookahead)?;
for p in &packets {
    let block = dec.decode(p).unwrap_or_else(|_| dec.conceal(Some(p)));
}
```

It decodes AAC (finding HE-AAC in the lookahead), MP1/MP2, AC-3/E-AC-3, Opus and PCM;
`codec()`, `sample_rate()` and `layout()` describe the output. Container timing is the caller's.
After a seek, `reset()` and decode `preroll()` frames before the target.

The codec modules are also usable on their own (`aac::Decoder`, `mpa::Decoder`, `ac3::Decoder`,
`opus::Decoder`); they return the codec's own channel order, which `aac::wav_order` and
`opus::wav_order` map to WAV order.

### Encode a file

```rust
use craft_audio::{BitDepth, ChannelLayout, EncodeFormat, EncodeOptions, FileEncoder, encode, encode_with_layout};

let opts = EncodeOptions { format: EncodeFormat::Flac, bit_depth: BitDepth::Int24, dither: true, bwf: None };
let flac: Vec<u8> = encode(&buf, &opts)?;

// A layout a channel count cannot name (5.1 with side surrounds), recorded in the WAV header.
let wav = encode_with_layout(&surround, &EncodeOptions::default(), ChannelLayout::SURROUND_5_1_SIDE)?;

// Block by block into any Write + Seek; the same bytes as `encode`.
let mut enc = FileEncoder::new(std::fs::File::create("take.wav")?, &EncodeOptions::default(), 2, 48_000, None)?;
enc.write(&[left, right], left.len())?;   // one slice (or Vec) per channel
enc.finish()?;
```

`EncodeOptions::default()` is 24-bit WAV. `encode_format_for("aiff")` maps an extension to an
`EncodeFormat`. After a failed write, every later `FileEncoder` call returns an error.

### Encode AAC or Opus packets

```rust
use craft_audio::aac::{Encoder, EncoderConfig, aac_order};

let mut enc = Encoder::new(EncoderConfig::cbr(48_000, 6, 384_000))?;
let input: Vec<&[f32]> = aac_order(6).unwrap().iter().map(|&c| wav_5_1[c].as_slice()).collect();
let mut units = enc.encode(&input);            // zero or more access units
units.extend(enc.flush());
let asc = enc.audio_specific_config();         // for the MP4 `esds` box
let priming = enc.priming_samples();           // for the edit list (1024)
```

`opus::encoder::OpusEncoder::encode_float` takes exactly 20 ms of interleaved samples and returns
one packet; `opus_head()` is the configuration for the container.

### Waveform overviews

`peaks`: a min/max/sum-of-squares pyramid of one channel (32-sample blocks, fan-out 4).
Range queries are exact, O(log n). `spans` and `rms_spans` return one value per display column.

### Errors

Functions return `craft_audio::Result<T>`. The codec modules have their own `Error` types, which
convert into `AudioError` with `?`.

| `AudioError` | Cause |
|---|---|
| `Unsupported` | valid input using an unsupported codec, feature or option |
| `Malformed` | corrupt or truncated input, or not the claimed format |
| `TooLarge` | over a size limit (`DecodeOptions::max_samples`, an allocation) |
| `Encode` | the encoder rejected the input (e.g. empty buffer, wrong layout) |
| `Io` | the source or the `FileEncoder` writer failed |

## Limits

- HE-AAC: the AAC-LC core is decoded and upsampled 2×. SBR is not reconstructed, so there is no
  content above the core's Nyquist frequency.
- E-AC-3: 7.1 programs decode as their 5.1 core. Adaptive hybrid transform and enhanced coupling
  return `Unsupported`.
- AC-3: matches ffmpeg at 25–45 dB SNR on noisy input, not bit-exact; zero-bit mantissa dither is
  decoder-specific (A/52 §7.3.4).
- WebM Opus: `probe` can over-report by up to one block (the last block's `DiscardPadding` is
  known only when it is read); `decode` and `Decoder` are exact.
- AAC channel configuration 7 and program-config layouts, Opus mapping families other than 0 and
  1, and AIFF with more than four channels decode with an unordered layout, in the codec's order.

## Tests

- `tests/audio/`: SoundCraft's tests. Round trips through every format and depth, streaming
  against whole-buffer encoding, float headroom, empty WAV data chunks, hostile input for every
  reader (including the size checks before symphonia), layouts, `serde` round trips.
- `src/write/flac/oracle_tests.rs`: FilmCraft's FLAC tests. Every signal, layout, depth, level and
  block size decodes in ffmpeg to the input samples; ffmpeg's MD5 matches STREAMINFO.
- Unit tests in `src/{aac,mpa,ac3,opus,read,layout}`.
- `tests/codecs/` (one test binary; `testkit.rs` replaces `filmcraft-testkit`): AAC decoder and
  encoder against ffmpeg, mutated access units and configs, MP1/MP2 and AC-3 decoders against
  ffmpeg-encoded streams, Opus decoder against libopus and the RFC 8251 vectors, Opus encoder
  through ffmpeg's two Opus decoders, concealment.
- `tests/codecs/routing_oracle.rs`: 19 container/codec pairs written by ffmpeg. Container, codec,
  layout, rate, channels and exact presentation length must match; output must be sample-aligned
  with ffmpeg's decode within each codec's SNR bound; `Decoder` must give the same blocks and seek
  to the exact frame; `TrackDecoder` on demuxed packets must equal `decode`; track choice and the
  sample limit on two-track files; truncated and corrupted copies must not panic.

ffmpeg is a test oracle only, never linked. The oracle helpers are `craft-media-testkit`
(unpublished), shared with craft-media: `CRAFT_MEDIA_FFMPEG` (then `PATH`, then common install
directories), `CRAFT_MEDIA_FIXTURES_DIR` (default `target/fixtures`), and
`CRAFT_MEDIA_REQUIRE_ORACLES=1` to fail instead of skip when ffmpeg or the vectors are missing.

The Opus conformance test needs the RFC 8251 vectors (75 MB, not committed):

```bash
scripts/fetch-opus-vectors.sh
```

CI (`.github/workflows/audio.yml`) runs clippy for each feature on its own, rustdoc with warnings
as errors, the WASM build, encode-only and native-only test runs, an MSRV check, and the full tests
with ffmpeg and the vectors required.

## Origin

| Part | Source |
|---|---|
| WAV, AIFF, `decode`/`probe`, peaks | SoundCraft `soundcraft-audio-io` [7dc56a4](https://github.com/storytold/soundcraft/commit/7dc56a4dbeebf57eacece09536726c57844fc59d) |
| FLAC encoder, AAC, MP1/MP2, AC-3, Opus decoder, Ogg | FilmCraft [8fda2f2](https://github.com/storytold/filmcraft/commit/8fda2f232198c4c79ff9cfe5b6a347bb1375fa9e) |
| Opus encoder | EffectCraft `effectcraft-opusenc` [c618429](https://github.com/storytold/effectcraft/commit/c618429252587525a2a6d4b5bccbe4ec5261e1b9) |

The codec modules' public items keep their names, so apps can re-export them
(`pub use craft_audio::aac::*;`); a few kept only for that are `#[doc(hidden)]`. Where apps had
overlapping implementations, the one that measured better was kept, with the tests of both:

| Format | Kept | Reason |
|---|---|---|
| FLAC encoding | FilmCraft | 0.1–37% smaller output than SoundCraft's on 12 test signals; STREAMINFO MD5 |
| WAV/AIFF writing | SoundCraft | only one with BWF, dither, arbitrary speaker masks, streaming without known length |
| AAC-LC, MP1/MP2 decoding | FilmCraft (over symphonia) | equal SNR against ffmpeg; also decodes 5.1 AAC and MP2 bit rates symphonia rejects |

Changes from the app versions (what an app switching over adjusts):

- `AudioInfo` has `container`, `codec` and `layout` instead of SoundCraft's `format`;
  `EncodeOptions::format` is an `EncodeFormat`; `detect_container` replaces `detect_format`;
  `encode_with_layout` and `FileEncoder::new(.., Option<ChannelLayout>)` replace the raw speaker
  mask, and `probe` reports the layout instead of `wav_channel_mask`.
- `AudioInfo`, `AudioError`, `SampleFormat`, `BitDepth` and the codec `Error` enums are
  `#[non_exhaustive]`.
- Features are per codec. SoundCraft's `encode` (WAV, AIFF, FLAC) is now `lossless-encode`;
  `encode` turns on every encoder, AAC and Opus included.
- `OpusEncoder::new`, `with_application`, `with_mode` and `encode_float` return `Result` instead
  of panicking on invalid channel count, mode/bandwidth or frame length.
- `FileEncoder::write` takes any per-channel slices.
- The FLAC encoder's vendor string is `craft-audio <version>`.
- `decode` uses native decoders before symphonia: AAC, MP2, AC-3 and Opus no longer go through
  symphonia; MP4/Matroska files decode to presentation length (priming and padding removed); a
  packet that fails to decode is concealed instead of dropped.
