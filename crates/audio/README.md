# craft-audio

Audio file I/O and codecs for the Craft apps. Native implementations: WAV and AIFF read/write,
FLAC encoder.
Formats without a native decoder are decoded by [symphonia](#symphonia), which is to be replaced
by native decoders.

- Input is a byte slice or any [`ByteSource`](../media/README.md#bytesource) (a file, a shared
  buffer); output is byte vectors or any `Write + Seek`. The crate opens no files itself.
- Decoded audio is planar `f32`, full scale ±1, in `AudioBuffer` (`sample_rate`, one `Vec<f32>`
  per channel), in WAV channel order (L R C LFE, back, side) whenever the channel layout is known.
- Malformed input returns an error, never a panic (see [symphonia](#symphonia) for the one
  exception). No `unsafe`. Readers and decoders are tested with corrupted and truncated input.
- Implemented from the published specifications; tested against ffmpeg.
  Builds for `wasm32-unknown-unknown`.
- Containers (MP4/MOV, Matroska/WebM, Ogg), bit I/O and the `Container`/`CodecId`/`CodecParams`
  types come from [`craft-media`](../media/README.md). This crate never depends on video codecs.

## Cargo features

No default features. The shared types (`AudioBuffer`, `AudioInfo`, `ChannelLayout`, `AudioError`,
`EncodeOptions`, `ByteSource`, …) build with none.

| Feature | Provides | Pulls in |
|---|---|---|
| `decode` | `decode`, `probe`, `Decoder`, `tracks`, `TrackDecoder`; every format in [Format support](#format-support) | `decode-native`, `symphonia` |
| `decode-native` | the same, native formats only (others are `Unsupported`) | craft-media `isobmff`/`matroska`/`ogg`, `log` |
| `symphonia` | the fallback for formats with no native decoder; turning it on is the same as `decode` | `decode-native`, symphonia |
| `encode` | every encoder: the WAV, AIFF and FLAC file writers | `lossless-encode` |
| `lossless-encode` | WAV, AIFF and FLAC writing | `pcm-encode`, `flac-encode` |
| `pcm-encode` | `encode`, `FileEncoder` for WAV and AIFF; `wav_float_header` | none |
| `flac-encode` | `encode`, `FileEncoder` for FLAC | craft-media `bitstream` |
| `peaks` | `peaks::Peaks` | none |
| `serde` | `Serialize`/`Deserialize` on the shared types | `serde` |

An app that decodes 32-bit FLAC with `decode` needs a profile override in its workspace; see
[symphonia](#symphonia).

WAV, AIFF and FLAC export only (no decoders, no demuxers, no symphonia):

```toml
craft-audio = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["lossless-encode"] }
```

Decode, every encoder and waveform overviews:

```toml
craft-audio = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["decode", "encode", "peaks"] }
```

craft-audio re-exports the craft-media types its API uses (`ByteSource`, `ReadSeekSource`,
`CodecParams`, …), and craft-media itself as `craft_audio::craft_media`. Use that to reach the
containers directly (for example to pass an `isobmff::Track`'s `codec_params()` to
`TrackDecoder`); `decode-native` turns on all three. A separate craft-media dependency must use
the **same `rev`** as craft-audio: with two revisions Cargo builds two copies, and their types do
not match.

## Usage

Runnable versions are in `examples/`:

```bash
cargo run -p craft-audio --example info --features decode -- take.m4a
```

```bash
cargo run -p craft-audio --example encode --features lossless-encode -- out
```

```bash
cargo run -p craft-audio --example stream --features decode -- long-recording.m4a 60
```

```bash
cargo run -p craft-audio --example packets --features decode -- take.mov
```

`stream` takes a file and a start time in seconds; `packets` takes an MP4 file. The entry points
(`decode`, `probe`, `tracks`, `Decoder`, `TrackDecoder`, `encode`, `FileEncoder`) and the
functions around them have rustdoc examples.

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
if let Some(second) = list.get(1) {
    let opts = DecodeOptions::default().with_track(second.index);
    let (info, buf) = decode_with(&bytes, None, &opts)?;
}
```

A track index the file does not have is `Unsupported`. `DecodeOptions::with_max_samples` sets
another size limit. `tracks_of` lists the tracks of a file read from a `ByteSource`;
`detect_container` names a file's container from its first bytes.

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

`Decoder::open` takes any `ByteSource` that is `Send + Sync` (`SharedSource`): a `Vec<u8>` or
`Arc<[u8]>`, a `ReadSeekSource` around any `Read + Seek`, or a `std::fs::File`. Prefer
`ReadSeekSource` for files: it takes the length once and reports a failure when it is made. On
single-threaded WebAssembly the source need not be `Send`, so a reader over a JS `Blob` works.
`Decoder::from_bytes` does the same for a borrowed buffer.

`seek` returns the frame decoding resumes at. A seek to or past the end of a file whose length is
known stops at the end, and the next `next_block` returns `None`. When `length_known()` is false,
`info().frames` is 0: add up the block lengths, or call `Decoder::probe`, which decodes the rest
of the file to count them. `Decoder::probe` is also how to probe a file on disk.

Native formats read their samples from the source packet by packet. The container's packet index
stays in memory (tens of bytes per packet), and opening a Matroska file reads through the whole
file to build it. Formats symphonia decodes are read whole first.

A damaged packet is concealed and counted in `info().damaged`. A source that fails a read (a
network drop) returns `AudioError::Io` from `next_block` instead, and that packet is skipped. To
retry it, keep the position: start from the frame the last `seek` returned (0 without one), add
each block's frames, and `seek` to that position.

### Decode packets from your own demuxer

`TrackDecoder` takes a track's `CodecParams` and the first packets. The parameters come from any
craft-media demuxer's `codec_params()` (see
[craft-media's codec parameters](../media/README.md#containers-codecs-and-codec-parameters)), or
are built with `CodecParams::new` for another container:

```rust
use craft_audio::{CodecId, CodecParams, PcmFormat, TrackDecoder};

let params = CodecParams::new(CodecId::Pcm).with_pcm(PcmFormat::int(24, true, true)).with_sample_rate(48_000).with_channels(2);
let lookahead: Vec<&[u8]> = packets.iter().take(craft_audio::LOOKAHEAD_PACKETS).map(Vec::as_slice).collect();
let mut dec = TrackDecoder::new(&params, &lookahead)?;
for p in &packets {
    let block = dec.decode(p).unwrap_or_else(|_| dec.conceal(Some(p)));
}
```

It decodes PCM so far, and returns `AudioBuffer`s in WAV order when `layout()` is ordered.
`codec()`, `sample_rate()` and `layout()` describe the output. Container timing is the caller's.

To seek: position your demuxer at the packet that starts `preroll()` frames or more before the
target, call `reset()`, decode from there, and drop the output before the target.

### Encode a file

```rust
use craft_audio::{AudioBuffer, BitDepth, ChannelLayout, EncodeFormat, EncodeOptions, FileEncoder, encode};

// One Vec of samples per channel, full scale ±1. `AudioBuffer::new(rate, channels, frames)`
// makes a silent one to fill.
let buf = AudioBuffer { sample_rate: 48_000, channels: vec![left, right] };

let opts = EncodeOptions::new(EncodeFormat::Flac).with_bit_depth(BitDepth::Int24).with_dither(true);
let flac: Vec<u8> = encode(&buf, &opts)?;

// 5.1 with side surrounds: a layout other than the default for 6 channels, recorded in the WAV.
let wav = encode(&surround, &EncodeOptions::default().with_layout(ChannelLayout::SURROUND_5_1_SIDE))?;

// Block by block into any Write + Seek; the same bytes as `encode`.
let mut enc = FileEncoder::new(std::fs::File::create("take.wav")?, &EncodeOptions::default(), 48_000, 2)?;
enc.write(&buf.channels, buf.frames())?;   // one slice (or Vec) per channel
enc.finish()?;
```

`EncodeOptions::default()` is 24-bit WAV. `with_bwf` adds a BWF `bext` chunk and
`with_force_rf64` writes RF64 below 4 GiB; both apply to WAV only. `encode_format_for("aiff")`
maps an extension to an `EncodeFormat`, and `ENCODE_EXTENSIONS` lists the extensions the enabled
features can write. `write` refuses a block without one slice per channel, or with a slice
shorter than `frames`, and writes nothing. After a write fails for another reason, every later
`write` and `finish` returns an error.

### Waveform overviews

```rust
use craft_audio::peaks::Peaks;

let peaks = Peaks::build(&left);                         // once per channel
// Start sample, samples per column, overlap, columns; pass the same samples each time. One
// (min, max) per column, `None` past the end of the data.
let columns = peaks.spans(&left, 0.0, 512.0, 0.0, 800);
```

`Peaks` is a min/max/sum-of-squares pyramid (32-sample blocks, fan-out 4). Range queries are
exact, O(log n). `rms_spans` returns one RMS value per column.

### Errors

Functions return `craft_audio::Result<T>`.

| `AudioError` | Cause |
|---|---|
| `Unsupported` | valid input using an unsupported codec, feature or option, or a format whose feature is off |
| `Malformed` | corrupt or truncated input, or not the claimed format |
| `TooLarge` | over a size limit (`DecodeOptions::max_samples`, an allocation) |
| `Encode` | an encoder rejected the input or its settings (a buffer with no channels or a 0 Hz rate, a block shorter than its frame count, a wrong layout) |
| `Io` | the source failed a read, or the `FileEncoder` writer failed |

`AudioError` is `#[non_exhaustive]`, so a `match` needs a `_` arm:

```rust
match craft_audio::decode(&bytes, None) {
    Ok((info, buf)) => { /* … */ }
    Err(craft_audio::AudioError::Unsupported(what)) => eprintln!("cannot decode: {what}"),
    Err(e) => eprintln!("{e}"),
}
```

## Format support

| Format | Decode | Encode |
|---|---|---|
| WAV, BWF, RF64/BW64 | native | native |
| AIFF, AIFF-C | native | native |
| FLAC | symphonia | native |
| AAC-LC, HE-AAC | symphonia | — |
| MP1, MP2, MP3 (MPEG audio Layer I/II/III) | symphonia | — |
| ALAC, Vorbis, ADPCM, A-law, µ-law | symphonia | — |
| Opus | — | — |

| Container | Demuxer | Timing |
|---|---|---|
| MP4, M4A, MOV | `craft_media::isobmff` for PCM; symphonia for the others (AAC, ALAC, MP3, FLAC) | edit lists |
| Matroska, WebM | `craft_media::matroska` for PCM; symphonia for the others (AAC, Vorbis, FLAC, MP3) | `CodecDelay`, `DiscardPadding` |
| Ogg | symphonia (Vorbis, FLAC) | — |
| Raw ADTS AAC, MP1/MP2/MP3 | symphonia | — |
| FLAC | symphonia | — |
| CAF | symphonia | — |

## Limits

- AIFF with more than four channels decodes with an unordered layout, in the file's order.
- Not decoded at all: AC-3, E-AC-3, Opus and Speex. `TrackDecoder` returns `Unsupported` for every
  codec but PCM; `decode` and `Decoder` read those through symphonia.

## Codecs

| Codec | Spec | Scope | Verified against |
|---|---|---|---|
| WAV, AIFF | Microsoft RIFF, Apple AIFF/AIFF-C | read: integer 8–32-bit, float 32/64-bit, `WAVE_FORMAT_EXTENSIBLE` masks, RF64/BW64, BWF `bext`, truncated files; write: 16/24/32-bit integer and 32-bit float, RF64 above 4 GiB, any layout, TPDF dither | round trips; hostile-input tests |
| FLAC encoder | RFC 9639 | 16, 24, 32 bits; 1–8 channels; up to 655,350 Hz; LPC, stereo decorrelation, STREAMINFO MD5 | bit-exact through ffmpeg and the reference `flac` decoder |

## symphonia

symphonia 0.6 (MPL-2.0, used unmodified as the Craft licensing rules require) decodes the formats
marked "symphonia" in [Format support](#format-support). Each is to be replaced by a native
decoder, after which the dependency is removed. The native path is tried first, so a new native
decoder takes its formats over without an API change. Build with `decode-native` instead of
`decode` to see what still needs symphonia.

Notes while symphonia is in use:

- symphonia reads from memory: a file it decodes is read whole first. Its decoded audio still
  streams through `Decoder`.
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

Tests that need ffmpeg are skipped when it is missing. To fail instead, set
`CRAFT_MEDIA_REQUIRE_ORACLES=1`. The helpers and their variables are shared with craft-media (see
[its tests](../media/README.md#tests)). The test files describe what they check;
`tests/codecs/routing_oracle.rs` is the end-to-end check of containers, codecs, timing, seeking
and track choice against ffmpeg.

CI (`.github/workflows/audio.yml`) runs clippy for each feature on its own, rustdoc with warnings
as errors, the WASM build, encode-only and native-only test runs, and an MSRV check. It also runs
the full tests with ffmpeg required.

## Origin

| Part | Source |
|---|---|
| WAV, AIFF, `decode`/`probe`, peaks | SoundCraft `soundcraft-audio-io` [7dc56a4](https://github.com/storytold/soundcraft/commit/7dc56a4dbeebf57eacece09536726c57844fc59d) |
| FLAC encoder | FilmCraft [8fda2f2](https://github.com/storytold/filmcraft/commit/8fda2f232198c4c79ff9cfe5b6a347bb1375fa9e) |

Where apps had overlapping implementations, the one that measured better was kept, with the
tests of both:

| Format | Kept | Reason |
|---|---|---|
| FLAC encoding | FilmCraft | 0.1–37% smaller output than SoundCraft's on 12 test signals; STREAMINFO MD5 |
| WAV/AIFF writing | SoundCraft | only one with BWF, dither, arbitrary speaker masks, streaming without known length |

## Migration notes for apps

For the apps switching from their own code; to be removed once all three have switched.

What an app adjusts:

- `AudioInfo` has `container`, `codec`, `layout` and `damaged` instead of SoundCraft's `format`.
  `detect_container` replaces `detect_format`. `probe` reports the layout instead of
  `wav_channel_mask`.
- `EncodeOptions` is built with `EncodeOptions::new(format)` and `with_*` methods. It carries the
  speaker layout and RF64, which replace `encode_with_layout`, `encode_wav_rf64` and
  `FileEncoder::new`'s layout argument. `BwfInfo` is built the same way. `ENCODE_EXTENSIONS` is a
  list.
- Arguments go sample rate, then channels, and channel counts are `u16`:
  `FileEncoder::new(out, &opts, sample_rate, channels)`, `wav_float_header(sample_rate,
  channels, frames)`, `AudioBuffer::new(sample_rate, channels, frames)`,
  `AudioBuffer::from_interleaved` and `num_channels`, and the `ChannelLayout` constructors and
  `channels()`.
- `AudioInfo`, `AudioError`, `EncodeOptions`, `BwfInfo`, `SampleFormat` and `BitDepth` are
  `#[non_exhaustive]`.
  `Speaker::ALL` is a slice.
- `TrackInfo::sample_rate` and `channels` are `Option`s.
- Features are per format. SoundCraft's `encode` (WAV, AIFF, FLAC) is now `lossless-encode`, made
  of `pcm-encode` (WAV, AIFF) and `flac-encode`. `encode` turns on every encoder. Writing a format
  whose feature is off returns `AudioError::Unsupported`.
- `FileEncoder::write` takes any per-channel slices, but refuses a block with missing channels or
  short slices instead of filling them with silence.
- `DecodeOptions` is not `Copy`.
- The FLAC encoder's vendor string is `craft-audio <version>`.
- `decode` tries the native path first (WAV, AIFF, and PCM in MP4 and Matroska), which decodes
  MP4 and Matroska files to presentation length. A failed source read is `AudioError::Io`.
