# craft-audio

Audio file I/O and codecs for the Craft apps. Native implementations: WAV and AIFF read/write,
FLAC encoder, AAC decoder/encoder, MP1/MP2 decoder, AC-3/E-AC-3 decoder, Opus decoder/encoder.
Formats without a native decoder are decoded by [symphonia](#symphonia), which is to be replaced
by native decoders.

- Input is a byte slice or any [`craft_media::ByteSource`](../media/README.md#bytesource) (a file,
  a shared buffer); output is byte vectors or any `Write + Seek`. The crate opens no files itself.
- Decoded audio is planar `f32`, full scale ±1, in `AudioBuffer` (`sample_rate`, one `Vec<f32>`
  per channel), in WAV channel order (L R C LFE, back, side) whenever the channel layout is known.
- Malformed input returns an error, never a panic (see [symphonia](#symphonia) for the one
  exception). No `unsafe`. Readers and decoders have mutation tests.
- Implemented from the published specifications; tested against ffmpeg and conformance vectors.
  Builds for `wasm32-unknown-unknown`.
- Containers (MP4/MOV, Matroska/WebM, Ogg), bit I/O and the `Container`/`CodecId`/`CodecParams`
  types come from [`craft-media`](../media/README.md). This crate never depends on video codecs.

## Cargo features

No default features. The shared types (`AudioBuffer`, `AudioInfo`, `ChannelLayout`, `AudioError`,
`EncodeOptions`, …) build with none.

| Feature | Provides | Pulls in |
|---|---|---|
| `decode` | `decode`, `probe`, `Decoder`, `tracks`, `TrackDecoder`; every format in [Format support](#format-support) | `decode-native`, `symphonia` |
| `decode-native` | the same, native formats only (others are `Unsupported`) | the four decoders below, craft-media `isobmff`/`matroska`/`ogg`, `log` |
| `symphonia` | the fallback for formats with no native decoder | `decode-native`, symphonia |
| `encode` | every encoder | `lossless-encode`, `aac-encode`, `opus-encode` |
| `lossless-encode` | WAV, AIFF and FLAC writing | `pcm-encode`, `flac-encode` |
| `pcm-encode` | `encode`, `FileEncoder` for WAV and AIFF; `wav_float_header` | none |
| `flac-encode` | `encode`, `FileEncoder` for FLAC | craft-media `bitstream` |
| `aac-decode`, `aac-encode` | `aac::Decoder`, `aac::Encoder` | craft-media `bitstream` |
| `opus-decode`, `opus-encode` | `opus::Decoder`/`StreamDecoder`, `opus::Encoder` | none |
| `mpa-decode` | `mpa::Decoder` | craft-media `bitstream` |
| `ac3-decode` | `ac3::Decoder` | none |
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

craft-media comes in as a dependency. To use its containers directly (for example to pass a
`craft_media::isobmff::Track`'s `codec_params()` to `TrackDecoder`), add craft-media with the
**same `rev`** as craft-audio and the container features you use. With two different revisions
Cargo builds two copies of craft-media, and their `CodecParams` types do not match.

## Usage

Runnable versions are in `examples/`:

```bash
cargo run -p craft-audio --example info --features decode -- take.m4a
```

```bash
cargo run -p craft-audio --example encode --features lossless-encode,aac-encode -- out
```

`stream` and `packets` run the same way with `--features decode`. Each public function also has
a rustdoc example.

### Decode a file

```rust
use craft_audio::{decode, probe};

let bytes = std::fs::read("take.m4a")?;

// Container, codec, rate, speaker layout and length. Read from the headers; when they give no
// length (an MP3 without a Xing frame, PCM in Matroska), the file is decoded to count it.
let info = probe(&bytes, Some("m4a"))?;
println!("{} in {}, {} Hz, {}", info.codec.name(), info.container.name(), info.sample_rate, info.layout);

// The first audio track: planar f32 in WAV channel order, trimmed to its presentation length.
let (info, buf) = decode(&bytes, Some("m4a"))?;
let left: &[f32] = &buf.channels[0];
```

`decode` returns `TooLarge` above 2³⁰ samples (4 GiB of `f32`). `info.damaged` counts packets
that were damaged and replaced by silence of their length.

### Choose a track

```rust
use craft_audio::{DecodeOptions, decode_with, tracks};

let list = tracks(&bytes, None)?;                 // codec, rate and channels of every audio track
let opts = DecodeOptions::default().with_track(list[1].index);
let (info, buf) = decode_with(&bytes, None, &opts)?;
```

`DecodeOptions::with_max_samples` sets another size limit. `tracks_of` lists the tracks of a
file read from a `ByteSource`; `detect_container` names a file's container from its first bytes.

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

`Decoder::open` takes any `ByteSource + Send + Sync`: a `std::fs::File`, a shared buffer, or a
`craft_media::ReadSeekSource` around any `Read + Seek`. `Decoder::from_bytes` does the same for a
buffer. When `length_known()` is false, `info().frames` is 0: add up the block lengths, or call
`Decoder::probe`, which decodes the rest of the file to count them. `Decoder::probe` is also how
to probe a file on disk without reading it into memory first.

A damaged packet is concealed and counted in `info().damaged`. A source that fails a read (a
network drop) returns `AudioError::Io` from `next_block` instead; that packet is skipped, so
`seek` back to retry it.

### Decode packets from your own demuxer

`TrackDecoder` takes a track's `CodecParams` (from any craft-media demuxer's `codec_params()`, see
[craft-media's codec parameters](../media/README.md#containers-codecs-and-codec-parameters), or
built with `CodecParams::new` for another container) and the first packets, and returns
`AudioBuffer`s in WAV order:

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

To seek: position your demuxer at the packet that starts `preroll()` frames or more before the
target, call `reset()`, decode from there, and drop the output before the target.

The codec modules are also usable on their own (`aac::Decoder`, `mpa::Decoder`, `ac3::Decoder`,
`opus::Decoder`); they return the codec's own channel order, which `aac::wav_order` and
`opus::wav_order` map to WAV order.

### Encode a file

```rust
use craft_audio::{BitDepth, ChannelLayout, EncodeFormat, EncodeOptions, FileEncoder, encode};

let opts = EncodeOptions::new(EncodeFormat::Flac).with_bit_depth(BitDepth::Int24).with_dither(true);
let flac: Vec<u8> = encode(&buf, &opts)?;

// 5.1 with side surrounds: a layout other than the default for 6 channels, recorded in the WAV.
let wav = encode(&surround, &EncodeOptions::default().with_layout(ChannelLayout::SURROUND_5_1_SIDE))?;

// Block by block into any Write + Seek; the same bytes as `encode`.
let mut enc = FileEncoder::new(std::fs::File::create("take.wav")?, &EncodeOptions::default(), 2, 48_000)?;
enc.write(&buf.channels, buf.frames())?;   // one slice (or Vec) per channel
enc.finish()?;
```

`EncodeOptions::default()` is 24-bit WAV. `with_bwf` adds a BWF `bext` chunk and
`with_force_rf64` writes RF64 below 4 GiB; both apply to WAV only. `encode_format_for("aiff")`
maps an extension to an `EncodeFormat`, and `ENCODE_EXTENSIONS` lists the extensions the enabled
features can write. After a failed write, every later `FileEncoder` call returns an error.

### Encode AAC

```rust
use craft_audio::AudioError;
use craft_audio::aac::{Encoder, EncoderConfig, aac_order};

let mut enc = Encoder::new(EncoderConfig::cbr(48_000, 6, 384_000))?;
let order = aac_order(6).ok_or_else(|| AudioError::Unsupported("no AAC order for 6 channels".into()))?;
let input: Vec<&[f32]> = order.iter().map(|&c| wav_5_1[c].as_slice()).collect();
let mut units = enc.encode(&input);            // zero or more access units
units.extend(enc.flush());
let asc = enc.audio_specific_config();         // for the MP4 `esds` box
let priming = enc.priming_samples();           // for the edit list (1024)
```

`EncoderConfig::cbr(..).with_adts(true)` makes every access unit an ADTS frame, which is a
playable `.aac` file when concatenated. To write MP4 or Matroska, pass the access units to
craft-media's muxers ([MP4](../media/README.md#muxing), [Matroska](../media/README.md#muxing-1)).

### Encode Opus

```rust
use craft_audio::opus::Encoder;

let mut enc = Encoder::new(2, 128_000)?;            // channels, bits per second
let mut packets = Vec::new();
for frame in interleaved.chunks_exact(Encoder::FRAME_SIZE * 2) {
    packets.push(enc.encode_float(frame)?);           // exactly 20 ms, interleaved
}
let head = enc.opus_head();     // the OpusHead for the container (Matroska CodecPrivate, MP4 dOps)
let pre_skip = enc.pre_skip();  // samples the player drops at the start
```

craft-media has no Ogg muxer, so `.opus` files cannot be written yet; Opus goes into Matroska/WebM
or MP4.

### Waveform overviews

`peaks`: a min/max/sum-of-squares pyramid of one channel (32-sample blocks, fan-out 4).
Range queries are exact, O(log n). `spans` and `rms_spans` return one value per display column.

### Errors

Functions return `craft_audio::Result<T>`. The codec modules have their own `Error` types, which
convert into `AudioError` with `?`.

| `AudioError` | Cause |
|---|---|
| `Unsupported` | valid input using an unsupported codec, feature or option, or a format whose feature is off |
| `Malformed` | corrupt or truncated input, or not the claimed format |
| `TooLarge` | over a size limit (`DecodeOptions::max_samples`, an allocation) |
| `Encode` | an encoder rejected the input or its settings (an empty buffer, a wrong layout, a rate AAC has no index for) |
| `Io` | the source failed a read, or the `FileEncoder` writer failed |

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
| MP4, M4A, MOV | `craft_media::isobmff` for native codecs; symphonia for the others (ALAC, MP3, FLAC) | edit lists (AAC priming, Opus pre-skip) |
| Matroska, WebM | `craft_media::matroska` for native codecs; symphonia for the others (Vorbis, FLAC, MP3) | `CodecDelay`, `DiscardPadding` |
| Ogg | `craft_media::ogg` (Opus); symphonia (Vorbis, FLAC) | pre-skip, end trimming |
| Raw ADTS AAC, MP1/MP2, AC-3, E-AC-3 | native frame sync | — |
| Raw MP3 | symphonia | — |
| FLAC | symphonia | — |
| CAF | symphonia | — |

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

## Codecs

### WAV and AIFF

- Read: WAV PCM 8/16/24/32-bit, float 32/64-bit, `WAVE_FORMAT_EXTENSIBLE` (the speaker mask
  becomes `AudioInfo::layout`; Ambisonic subtypes are unordered), RF64/BW64, BWF `bext`, truncated
  files (data chunk size past end of file). AIFF and AIFF-C `NONE`, `twos`, `sowt`, `fl32`,
  `fl64`. Other WAV/AIFF codecs go to symphonia. Headers are parsed through the source, reading
  only chunk headers and small chunk bodies, and the samples stream.
- Write: WAV/BWF 16/24/32-bit integer or 32-bit float; RF64 above 4 GiB (or always, with
  `with_force_rf64`); any layout (`with_layout`); optional `bext`. AIFF at integer depths, AIFF-C
  `fl32` for float.
- Integer output clamps to ±1, optional TPDF dither. Float output keeps values above full scale.

### FLAC encoder

Spec: RFC 9639.

- 16, 24 or 32 bits; 1–8 channels; up to 655,350 Hz.
- 4096-frame blocks. Per channel: smallest of constant, verbatim, fixed polynomial or LPC
  subframes, partitioned Rice residuals. Stereo also tries left/side, side/right, mid/side.
- STREAMINFO includes frame sizes and the audio MD5.
- Output verified bit-exact with ffmpeg and the reference `flac` decoder.

### AAC (`aac`)

Spec: ISO/IEC 14496-3.

- Decoder: AAC-LC; M/S and intensity stereo, PNS, TNS, pulse data, sine and KBD windows, all
  window sequences; 1–8 channels. HE-AAC v1/v2: AAC-LC core only. Output against ffmpeg's
  decoder: ~135 dB SNR. With PNS, output differs per sample from ffmpeg (PNS noise is
  decoder-generated) at equal band energy.
- Encoder: AAC-LC; psychoacoustic model, window switching, M/S, TNS, bit reservoir; CBR or VBR;
  1–8 channels. Outputs raw access units and the `AudioSpecificConfig`, or ADTS frames. Input is
  in AAC element order (centre first); `aac_order(channels)` maps WAV order to it.

### MP1/MP2 (`mpa`)

Spec: ISO/IEC 11172-3, 13818-3.

- Layer I and II; mono, stereo, dual channel, joint stereo; all MPEG-1, MPEG-2 and MPEG-2.5 bit
  rates and sample rates (8–48 kHz).
- Accepts Layer II bit rate/mode combinations the spec disallows (e.g. ffmpeg's default mono
  384 kbit/s).
- Not supported: Layer III (MP3), free-format bit rate.

### AC-3/E-AC-3 (`ac3`)

Spec: ATSC A/52:2012.

- AC-3: all channel modes (1/0 to 3/2, ±LFE), 32/44.1/48 kHz, all frame sizes, coupling,
  rematrixing, dynamic range control, block switching.
- E-AC-3: independent substream 0, 1–6 blocks per frame, reduced sample rates, spectral
  extension.
- Output in WAV order (fronts, LFE, surrounds), no downmix.

### Opus (`opus`)

Spec: RFC 6716, RFC 8251, RFC 7845.

- Decoder: SILK, hybrid and CELT; all frame sizes; packet-loss concealment and FEC; multistream
  (`Decoder`) and single stream (`StreamDecoder`); output at 8/12/16/24/48 kHz. Passes all 36 RFC
  8251 conformance cases with matching range-coder state after every packet.
- Encoder (`Encoder`): SILK, hybrid and CELT, selected from bit rate and `Application` (`Audio`,
  `Voip`) or set explicitly; 48 kHz, 20 ms frames, mono or stereo. Outputs packets and the
  `OpusHead`.
- `Bandwidth` is one type for both; the decoder's and encoder's `Mode` convert into each other.

## symphonia

symphonia 0.6 (MPL-2.0, used unmodified as the Craft licensing rules require) decodes the formats
marked "symphonia" in [Format support](#format-support). Each is to be replaced by a native
decoder, after which the dependency is removed. The native path is tried first, so a new native
decoder takes its formats over without an API change. Build with `decode-native` instead of
`decode` to see what still needs symphonia.

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

## Tests

Both integration test binaries need features, so a plain `cargo test -p craft-audio` runs only
the unit tests. Run everything with:

```bash
cargo test -p craft-audio --all-features
```

Tests that need ffmpeg or the Opus vectors are skipped when those are missing. To fail instead,
set `CRAFT_MEDIA_REQUIRE_ORACLES=1`.

- `tests/audio/`: round trips through every format and depth, streaming against whole-buffer
  encoding, float headroom, empty WAV data chunks, hostile input for every reader (including the
  size checks before symphonia), layouts, sources that fail, `serde` round trips.
- `src/write/flac/oracle_tests.rs`: every signal, layout, depth, level and block size decodes in
  ffmpeg to the input samples; ffmpeg's MD5 matches STREAMINFO.
- Unit tests in `src/{aac,mpa,ac3,opus,read,write,layout}`.
- `tests/codecs/`: AAC decoder and encoder against ffmpeg, mutated access units and configs,
  MP1/MP2 and AC-3 decoders against ffmpeg-encoded streams, Opus decoder against libopus and the
  RFC 8251 vectors, Opus encoder through ffmpeg's two Opus decoders, concealment.
- `tests/codecs/routing_oracle.rs`: 19 container/codec pairs written by ffmpeg. Container, codec,
  layout, rate, channels and exact presentation length must match; output must be sample-aligned
  with ffmpeg's decode within each codec's SNR bound; `Decoder` must give the same blocks and seek
  to the exact frame; `TrackDecoder` on demuxed packets must equal `decode`; track choice and the
  sample limit on two-track files; truncated and corrupted copies must not panic.

ffmpeg is a test oracle only, never linked. The oracle helpers are `craft-media-testkit`
(unpublished), shared with craft-media. They find ffmpeg through `CRAFT_MEDIA_FFMPEG`, then
`PATH`, then common install directories, and fixtures in `CRAFT_MEDIA_FIXTURES_DIR` (default
`target/fixtures`).

The Opus conformance test needs the RFC 8251 vectors (75 MB, not committed). The script fetches
them into `target/fixtures/opus/vectors`; with `CRAFT_MEDIA_FIXTURES_DIR` set, pass the
directory:

```bash
scripts/fetch-opus-vectors.sh
```

```bash
scripts/fetch-opus-vectors.sh "$CRAFT_MEDIA_FIXTURES_DIR/opus/vectors"
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

Where apps had overlapping implementations, the one that measured better was kept, with the
tests of both:

| Format | Kept | Reason |
|---|---|---|
| FLAC encoding | FilmCraft | 0.1–37% smaller output than SoundCraft's on 12 test signals; STREAMINFO MD5 |
| WAV/AIFF writing | SoundCraft | only one with BWF, dither, arbitrary speaker masks, streaming without known length |
| AAC-LC, MP1/MP2 decoding | FilmCraft (over symphonia) | equal SNR against ffmpeg; also decodes 5.1 AAC and MP2 bit rates symphonia rejects |

## Migration notes for apps

The codec modules' public items keep their names, so apps can re-export them
(`pub use craft_audio::aac::*;`). Names kept only for that are `#[doc(hidden)]`:
`opus::encoder::OpusEncoder` (now `opus::Encoder`), `ac3::Decoded` (now `ac3::Frame`) and
`aac::Upsample2x`. What else an app adjusts when it switches over:

- `AudioInfo` has `container`, `codec` and `layout` instead of SoundCraft's `format`, and
  `damaged`; `detect_container` replaces `detect_format`; `probe` reports the layout instead of
  `wav_channel_mask`.
- `EncodeOptions` is built with `EncodeOptions::new(format)` and `with_*` methods. It carries the
  speaker layout and RF64, which replace `encode_with_layout`, `encode_wav_rf64` and
  `FileEncoder::new`'s layout argument; `FileEncoder::new` takes the channel count as `u16`.
  `BwfInfo` and `aac::EncoderConfig` are built the same way (`with_adts`, …).
  `ENCODE_EXTENSIONS` is a list.
- `AudioInfo`, `AudioError`, `EncodeOptions`, `BwfInfo`, `SampleFormat`, `BitDepth`, the codec
  `Error` enums and the codec modules' header, frame and config structs are `#[non_exhaustive]`.
- `TrackInfo::sample_rate` and `channels` are `Option`s.
- Features are per codec. SoundCraft's `encode` (WAV, AIFF, FLAC) is now `lossless-encode`, made of
  `pcm-encode` (WAV, AIFF) and `flac-encode`; `encode` turns on every encoder, AAC and Opus
  included. Writing a format whose feature is off returns `AudioError::Unsupported`.
- The Opus encoder's constructors and `encode_float` return `Result` instead of panicking on an
  invalid channel count, mode/bandwidth or frame length. AAC encoder settings it cannot use are
  `aac::Error::InvalidSettings` (`AudioError::Encode`).
- `TrackDecoder::decode` returns an error for a corrupt Opus packet, as for the other codecs; call
  `conceal`.
- `FileEncoder::write` takes any per-channel slices.
- The FLAC encoder's vendor string is `craft-audio <version>`.
- `decode` uses native decoders before symphonia: AAC, MP2, AC-3 and Opus no longer go through
  symphonia; MP4/Matroska files decode to presentation length (priming and padding removed); a
  packet that fails to decode is concealed instead of dropped, and a source that fails a read is
  `AudioError::Io`.
