# craft-audio

Audio file I/O and codecs for the Craft apps. Native implementations: WAV and AIFF read/write,
FLAC encoder, AAC and ALAC decoders/encoders, AC-3/E-AC-3 decoder/encoder, MP3 encoder, Opus and MP1/MP2 decoders.
Formats without a native decoder are decoded by [symphonia](#symphonia), which is to be replaced
by native decoders.

- Input is a byte slice or any [`ByteSource`](../media/README.md#bytesource) (a file, a shared
  buffer); output is byte vectors or any `Write + Seek`. The crate opens no files itself.
- Decoded audio is planar `f32`, full scale ±1, in `AudioBuffer` (`sample_rate`, one `Vec<f32>`
  per channel), in WAV channel order (L R C LFE, back, side) whenever the channel layout is known.
- Malformed input returns an error, never a panic (see [symphonia](#symphonia) for the one
  exception). No `unsafe`. Readers and decoders are tested with corrupted and truncated input.
- Implemented from the published specifications, except ALAC, which is ported from Apple's
  Apache-2.0 reference implementation (see [Origin](#origin)); tested against ffmpeg and
  conformance vectors.
  Builds for `wasm32-unknown-unknown`.
- Containers (MP4/MOV, Matroska/WebM, Ogg), bit I/O and the `Container`/`CodecId`/`CodecParams`
  types come from [`craft-media`](../media/README.md). This crate never depends on video codecs.

## Cargo features

No default features. The shared types (`AudioBuffer`, `AudioInfo`, `ChannelLayout`, `AudioError`,
`EncodeOptions`, `ByteSource`, …) build with none.

| Feature | Provides | Pulls in |
|---|---|---|
| `decode` | `decode`, `probe`, `Decoder`, `tracks`, `TrackDecoder`; every format in [Format support](#format-support) | `decode-native`, `symphonia` |
| `decode-native` | the same, native formats only (others are `Unsupported`); `wav_pcm`, `aiff_pcm` (a file's integer PCM bit for bit, as `RawPcm`) | the five decoders below, craft-media `isobmff`/`matroska`/`ogg`, `log` |
| `symphonia` | the fallback for formats with no native decoder; turning it on is the same as `decode` | `decode-native`, symphonia |
| `encode` | every encoder: the WAV, AIFF and FLAC file writers, and the AAC, ALAC, AC-3/E-AC-3 and MP3 encoders | `lossless-encode`, `aac-encode`, `alac-encode`, `ac3-encode`, `eac3-encode`, `mp3-encode` |
| `lossless-encode` | WAV, AIFF and FLAC writing | `pcm-encode`, `flac-encode` |
| `pcm-encode` | `encode`, `FileEncoder` for WAV and AIFF; `wav_float_header`; `pcm_wav_file`, `pcm_wav_header`, `pcm_aiff_file`, `pcm_aiff_header` (minimal files around integer PCM bytes, for essence embedded in AAF / OMF) | none |
| `flac-encode` | `encode`, `FileEncoder` for FLAC; `flac::Encoder`, the packet encoder for callers that mux FLAC themselves | craft-media `bitstream` |
| `aac-decode`, `aac-encode` | `aac::Decoder`, `aac::Encoder`; with `aac-decode`, `aac::latm_config` / `latm_split` for LOAS / LATM frames (AAC in MPEG transport streams) | craft-media `bitstream` |
| `alac-decode` | `alac::Decoder` | none |
| `alac-encode` | `alac::Encoder` | none |
| `opus-decode` | `opus::Decoder`/`StreamDecoder` | none |
| `ac3-decode` | `ac3::Decoder`, `ac3::ProgrammeDecoder` | craft-media `bitstream` |
| `ac3-encode`, `eac3-encode` | `ac3::encoder::Encoder` | craft-media `bitstream` |
| `mpa-decode` | `mpa::Decoder` | craft-media `bitstream` |
| `mp3-encode` | `mp3::Encoder` | craft-media `bitstream` |
| `peaks` | `peaks::Peaks` | none |
| `rayon` | parallel AAC encoding (with `aac-encode`) | `rayon` |
| `serde` | `Serialize`/`Deserialize` on the shared types | `serde` |

Any one decoder feature (`aac-decode`, `alac-decode`, `opus-decode`, `ac3-decode`, `mpa-decode`)
also gives `TrackDecoder`, for that codec and PCM.

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

Packets from an app's own demuxer, AAC both ways:

```toml
craft-audio = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["aac-decode", "aac-encode"] }
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
cargo run -p craft-audio --example encode --features lossless-encode,aac-encode -- out
```

```bash
cargo run -p craft-audio --example stream --features decode -- long-recording.m4a 60
```

```bash
cargo run -p craft-audio --example packets --features decode -- take.m4a
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
that were damaged and replaced by silence of their length. A FLAC frame whose checksum fails is
dropped by the demuxer; the jump in the timestamps shows it, and the lost frames are silence too (one
count per run of them), so the audio after them stays in place.

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
another size limit. The default is `DEFAULT_MAX_SAMPLES`, 2^30 samples (4 GiB of `f32`), which
`decode` and `decode_with` can fill from a small compressed file because they keep the whole
result. An app that opens files it did not write passes a limit it can hold (for example
`DEFAULT_MAX_SAMPLES / 8`, or its longest supported clip), or streams with `Decoder`, which has no
limit because it keeps nothing. `tracks_of` lists the tracks of a file read from a `ByteSource`;
`detect_container` names a file's container from its first bytes. `DECODE_EXTENSIONS` lists the
file extensions of the formats `decode` reads, for a file picker's filter.

### Stream and seek

```rust
use craft_audio::{DecodeOptions, Decoder};

let file = std::fs::File::open("long-recording.m4a")?;
let mut dec = Decoder::open(file, Some("m4a"), &DecodeOptions::default())?;
dec.seek(48_000 * 60)?;                   // sample-exact (see Limits): the next block starts here
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
stays in memory (tens of bytes per packet), and opening an Ogg or Matroska file or a raw ADTS
stream reads through the whole file to build it. Formats symphonia decodes are read whole first.

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
use craft_audio::{CodecId, CodecParams, TrackDecoder};

let params = CodecParams::new(CodecId::Aac).with_extradata(asc).with_sample_rate(48_000);
let lookahead: Vec<&[u8]> = packets.iter().take(craft_audio::LOOKAHEAD_PACKETS).map(Vec::as_slice).collect();
let mut dec = TrackDecoder::new(&params, &lookahead)?;
for p in &packets {
    let block = dec.decode(p).or_else(|_| dec.conceal(Some(p)))?;
}
```

It decodes AAC (finding HE-AAC in the lookahead), Opus, AC-3/E-AC-3, MP1/MP2 and PCM, and returns
`AudioBuffer`s in WAV order when `layout()` is ordered. `codec()`, `sample_rate()` and `layout()`
describe the output. Container timing is the caller's.

Streams without codec configuration work where the packets carry it. AAC with empty `extradata`
takes its configuration from the ADTS headers (MPEG-TS, raw `.aac`), which are stripped from each
packet. Opus with empty `extradata` and 1 or 2 channels (RTP, MPEG-TS) needs no `OpusHead`.

To seek: position your demuxer at the packet that starts `preroll()` frames or more before the
target, call `reset()`, decode from there, and drop the output before the target.

The codec modules are also usable on their own. `aac::Decoder` and `opus::Decoder` return the
codec's channel order; `aac::wav_order(channel_config)` and `opus::wav_order(channels)` map it to
WAV order. `ac3::Decoder` and `mpa::Decoder` output WAV order.

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

// Block by block into any Write + Seek with nothing after its position; the same bytes as `encode`.
let mut enc = FileEncoder::new(std::fs::File::create("take.wav")?, &EncodeOptions::default(), 48_000, 2)?;
enc.write(&buf.channels, buf.frames())?;   // one slice (or Vec) per channel
enc.finish()?;
```

`EncodeOptions::default()` is 24-bit WAV. `with_bwf` adds a BWF `bext` chunk and
`with_force_rf64` writes RF64 below 4 GiB; both apply to WAV only. `encode_format_for("aiff")`
maps an extension to an `EncodeFormat`, and `ENCODE_EXTENSIONS` lists the extensions the enabled
features can write. `EncodeFormat::ALL` lists the formats; `extension()` is the extension a new
file gets and `mime_type()` its media type. `write` refuses a block without one slice per channel, or with a slice
shorter than `frames`, and writes nothing. After a write fails for another reason, every later
`write` and `finish` returns an error.

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

The sample rate must be one of the 13 AAC rates (7350–96000 Hz). `aac_order` returns `None` for
mono and stereo, which are already in order, and for 8 channels. Eight channels are written as AAC
channel configuration 7 and taken in its element order: C, Lc Rc, L R, Ls Rs, LFE (front centre,
front left and right of centre, front left and right, surrounds). A WAV 7.1 buffer (L R C LFE Lb Rb
Ls Rs) has no equivalent in that layout; passed unchanged, its channels go to the wrong speakers.

`EncoderConfig::cbr(..).with_adts(true)` makes every access unit an ADTS frame; concatenated, they
are a playable `.aac` file. To write MP4 or Matroska, pass the access units to craft-media's
muxers ([MP4](../media/README.md#muxing), [Matroska](../media/README.md#muxing-1)).

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

Functions return `craft_audio::Result<T>`. The codec modules have their own `Error` types, which
convert into `AudioError` with `?`.

| `AudioError` | Cause |
|---|---|
| `Unsupported` | valid input using an unsupported codec, feature or option, or a format whose feature is off |
| `Malformed` | corrupt or truncated input, or not the claimed format |
| `TooLarge` | over a size limit (`DecodeOptions::max_samples`, an allocation) |
| `Encode` | an encoder rejected the input or its settings (a buffer with no channels or a 0 Hz rate, a block shorter than its frame count, a wrong layout, a rate AAC has no index for) |
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
| AAC-LC | native | native |
| HE-AAC v1/v2 | native, core only (see [Limits](#limits)) | — |
| ALAC | native | native, packets (`alac::Encoder`; no file writer) |
| MP1, MP2 (MPEG audio Layer I/II) | native | — |
| MP3 (Layer III) | symphonia | native, frames (`mp3::Encoder`; the frames in a row are an `.mp3` file) |
| Opus | native | — |
| AC-3, E-AC-3 | native | native packet encoder |
| Vorbis, ADPCM, A-law, µ-law | symphonia | — |

| Container | Demuxer | Timing |
|---|---|---|
| MP4, M4A, MOV | `craft_media::isobmff` for native codecs; symphonia for the others (MP3, FLAC) | edit lists (AAC priming, Opus pre-skip) |
| Matroska, WebM | `craft_media::matroska` for native codecs; symphonia for the others (Vorbis, FLAC, MP3) | `CodecDelay`, `DiscardPadding` |
| Ogg | `craft_media::ogg` (Opus); symphonia (Vorbis, FLAC) | pre-skip, end trimming |
| Raw ADTS AAC, AC-3, E-AC-3, MP1/MP2 | native frame sync | — |
| Raw MP3 | symphonia | — |
| FLAC | symphonia | — |
| CAF | symphonia | — |

## Limits

- HE-AAC: the AAC-LC core is decoded. `decode`, `Decoder` and `TrackDecoder` upsample it 2×;
  `aac::Decoder` alone outputs the core rate. SBR is not reconstructed, so there is no content
  above the core's Nyquist frequency.
- HE-AAC v2: parametric stereo is not reconstructed either, so the mono core decodes as mono.
- AAC: Main, SSR and LTP profiles, coupling channels and 960-sample frames are unsupported.
- Raw ADTS: frames with several raw data blocks are unsupported: a stream of them is refused, and
  one among other frames decodes as silence of its length and counts as damaged, as does a frame
  that changes the stream's sample rate, profile or channel configuration.
- A seek into AAC with PNS gives the same samples each time, but its noise differs from a continuous
  decode's, at the same band energy.
- MP4 gapless information outside an edit list is not read: a file from Apple's encoders with
  `iTunSMPB` and no `elst` decodes with its priming and padding.
- A Matroska track with no block durations (AAC from ffmpeg 6) ends at its `DURATION` tag, when
  that ends inside the last packet. Otherwise all its audio is presented, including any padding.
- ffmpeg before 9 ends some codecs' tracks in MP4 and Matroska up to 11 ms early. The audio past
  the end a file declares is not presented.
- A Matroska track's end comes from its timestamps, so it is known to one timestamp tick (usually
  1 ms) unless the last block has `DiscardPadding`.
- E-AC-3: `ac3::Decoder` and `TrackDecoder` retain independent-substream decoding, so a
  7.1 programme is presented as its 5.1 compatibility downmix. Use `ac3::ProgrammeDecoder`
  with an explicit expected layout for complete six-block 7.1 access units; see below.
  Adaptive hybrid transform and enhanced coupling return `Unsupported`.
- E-AC-3 with syncframes of fewer than six blocks: a seek decodes from the start of the stream,
  because such syncframes may reuse exponents from any syncframe before them. A seek in Matroska
  E-AC-3 reads the syncframe headers of the packets before its target to find the packets' places.
- AC-3: not bit-exact. Zero-bit mantissas are filled with decoder-specific dither (A/52 §7.3.4),
  so white noise matches ffmpeg at only 27 dB (AC-3) and 48 dB (E-AC-3) SNR.
- WebM Opus: `probe` can over-report by up to one block (the last block's `DiscardPadding` is
  known only when it is read); `decode` and `Decoder` are exact.
- AAC channel configuration 7 and program-config layouts, Opus mapping families other than 0 and
  1, and AIFF with more than four channels decode with an unordered layout, in the codec's order.
- A FLAC layout that decoders would not assume for the channel count depends on its
  `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` comment. This includes the usual layouts for 5 and 6 channels:
  without the comment, back surrounds are read as side surrounds. It includes the usual 7-channel
  layout too: without the comment, its back left, back right and back centre are read as back
  centre, side left and side right.
- FLAC at 32 bits codes stereo channels independently, because a side channel would need 33 bits.
- Outside RFC 9639's streamable subset (§7), which some decoders require: FLAC at 32 bits, at a
  sample rate the frame header cannot name, or with a layout that needs the mask comment.
- `ChannelLayout::unordered` is written to FLAC as mask `0x0000`, as RFC 9639 §8.6.2 allows, but
  ffmpeg and `flac -d` ignore that mask and use their default layout for the channel count.
- Not decoded at all: Speex.
- `TrackDecoder` returns `Unsupported` for MP3, FLAC and Vorbis; `decode` and `Decoder` read
  those through symphonia. ALAC in CAF still goes through symphonia (craft-media has no CAF
  demuxer).
- ALAC: `EncodeFormat` has no ALAC entry; `alac::Encoder` writes packets and the configuration,
  and the caller muxes them (MP4 `alac` box, Matroska `A_ALAC`). craft-media's MP4 muxer writes no
  `chan` box, so an MP4 of more than two channels carries no explicit layout; decoders then use the
  standard ALAC layout for the count.
- ALAC 32-bit streams: ffmpeg does not decode their uncompressed (escaped) stereo frames, which
  Apple's encoder and this one write for audio that does not compress; Apple's decoder and this
  one do.
- MP4 edit lists: a single media edit (encoder priming and the end) is applied, alone or after one
  empty edit; the empty edit's delay is not added as silence. Other edit lists are not applied, and
  the media plays as it is stored.
- AAC in MP4 stored as ADTS frames, when the sample durations count fewer frames than the samples
  hold: HE-AAC durations that count only the core frames, or several frames in a sample timed as
  one. Such a file is broken: it is outside the MP4 specification in two ways.
  - An MP4 AAC sample is one raw access unit, with the stream's configuration in the sample
    description (ISO/IEC 14496-14 and 14496-3). ADTS is a transport framing for streams such as
    MPEG-TS and raw `.aac`, and its headers do not belong in an MP4 sample.
  - A sample's duration in the time-to-sample table is the time it decodes to (ISO/IEC 14496-12).
    Durations that count fewer frames than the samples hold do not describe the track.

  ffmpeg, Apple's encoders, phones and cameras write raw AAC with matching durations, and a
  correct remux from MPEG-TS or raw `.aac` converts the framing, so files within the specification
  are not affected. In a file outside it:
  - Read from the start, the track decodes in full: durations that fall short of the frames in
    the samples read at open do not end it.
  - `Decoder::seek` places samples by those durations, so it lands later than asked, and an edit
    list measured in them can end the track early.
  - These are not corrected because where each sample's audio starts is only known by counting
    the ADTS frames in every sample before it: one read per sample, hundreds of thousands on a
    two-hour track. Done at open, that would slow every ADTS-framed MP4 to work around files
    that are outside the specification.

## Codecs

Each codec module's rustdoc lists the coding tools it implements.

| Codec | Spec | Scope | Verified against |
|---|---|---|---|
| WAV, AIFF | Microsoft RIFF, Apple AIFF/AIFF-C | read: integer 8–32-bit, float 32/64-bit, `WAVE_FORMAT_EXTENSIBLE` masks, RF64/BW64, BWF `bext`, truncated files; write: 16/24/32-bit integer and 32-bit float, RF64 above 4 GiB, any layout, TPDF dither | round trips; hostile-input tests |
| FLAC encoder | RFC 9639 | 16, 24, 32 bits; 1–8 channels, layouts decoders would not assume for the count as a `WAVEFORMATEXTENSIBLE_CHANNEL_MASK` comment; up to 655,350 Hz; LPC, stereo decorrelation, wasted bits, STREAMINFO MD5 | bit-exact through ffmpeg, STREAMINFO MD5 checked against ffmpeg |
| AAC (`aac`) | ISO/IEC 14496-3 | AAC-LC decode and encode (CBR, VBR, ADTS), 1–8 channels; HE-AAC: see [Limits](#limits) | ffmpeg: ~135 dB SNR; PNS noise differs per sample at equal band energy |
| Opus (`opus`) | RFC 6716, 8251, 7845 | decoder: SILK, hybrid, CELT, all frame sizes, PLC and FEC, multistream (`Decoder`) and single stream (`StreamDecoder`), 8–48 kHz output | all 36 RFC 8251 conformance cases |
| AC-3/E-AC-3 (`ac3`) | decoder: ATSC A/52:2012; encoder/programme API: ETSI TS 102 366 V1.4.1 | single-syncframe and complete-programme decoding; packet encoding through discrete 7.1 (see below) | decoder comparisons and encoder output through ffmpeg, CRC/channel/timing/bit-budget regressions; synthetic quality limitations below |
| MP1/MP2 (`mpa`) | ISO/IEC 11172-3, 13818-3 | Layer I and II, MPEG-1/2/2.5, 8–48 kHz; also the Layer II rate/mode pairs the spec disallows (ffmpeg's default mono 384 kbit/s); no free format | ffmpeg-encoded streams, 73–84 dB SNR |
| MP3 encoder (`mp3`) | ISO/IEC 11172-3, 13818-3 | Layer III, constant bit rate: 32–320 kbit/s at 32/44.1/48 kHz, 8–160 kbit/s at 16/22.05/24 kHz; mono, stereo, joint stereo (mid/side); bit reservoir; long blocks only; no Xing/Info tag, VBR, CRC or free format | ffmpeg decodes every Huffman code as intended; full encodes at every sampling frequency checked with ffprobe and by SNR of ffmpeg's decode |
| ALAC (`alac`) | Apple's reference implementation | decode and encode, 1–8 channels, 16/20/24/32 bits (decode: any depth 1–32), fast mode, magic cookie in MP4/Matroska and CAF form | Apple's reference: same packets from the encoder, same samples from both decoders on 766 configurations; ffmpeg: bit-exact both ways |

### MP3 encoding

```rust
use craft_audio::mp3::{Config, DELAY, Encoder};

let mut enc = Encoder::new(Config::cbr(48_000, 2, 192))?;
let mut mp3 = enc.encode(&[&left, &right]); // whole frames as they complete
mp3.extend(enc.flush()); // pads with silence to the last input sample
// a decoder's output lags the input by DELAY (1057) samples
```

`mp3::bitrates(rate)`, `nearest_bitrate(rate, kbps)` and `nearest_sample_rate(rate)` map arbitrary
settings onto what a stream can carry; `Encoder::new` refuses anything else. Samples that are not
finite count as silence and the rest are clamped to ±4, so no input makes an invalid frame.

- Hybrid filterbank: the Annex C polyphase analysis filter, the normal-window MDCT and alias
  reduction; reconstruction is checked against a reference of the normative decoder filterbank.
- Psychoacoustic model: band energies spread across scalefactor bands, floored by the threshold in
  quiet (full scale = 96 dB SPL); a low-pass by bit rate.
- Quantization: the inner and outer loops with scalefactors, an exhaustive choice of the region
  split and the Huffman table per region, and the bit reservoir (`main_data_begin`) bounded by the
  decoder's 7680-bit input buffer.
- Not yet: block switching (short blocks; transients pre-echo), `scfsi` and `preflag`, intensity
  stereo, a tonality-aware model, VBR/ABR, the Xing/Info and LAME gapless tags (the delay is not
  signalled), CRC, free format, and an `EncodeFormat` entry for `encode`/`FileEncoder`.

### AC-3 and E-AC-3 encoding

`ac3::encoder` is implemented from **ETSI TS 102 366 V1.4.1 (September 2017)**,
with the existing first-party decoder's normative allocation tables/math shared internally.
No third-party codec implementation source is used. Decoder executables are interoperability
oracles, not linked dependencies. This is a basic encoder implementation, not a Dolby-certified
product or a claim of perceptual parity with mature commercial encoders.

The public `Config` chooses `Codec`, sample rate, programme bitrate, `CodingMode` and LFE.
`Encoder::new` validates these before taking PCM. Both codecs accept 32, 44.1 and 48 kHz,
all eight AC-3 coding modes (including distinct dual mono), and optional LFE. AC-3 uses
its standardized 32–640 kb/s frame-size table; this implementation requires at least
32 kb/s per full-bandwidth channel. E-AC-3 accepts 32–1024 kb/s programme rates, subject
to that same minimum and the per-substream 2048-word limit. E-AC-3 discrete 7.1 requires
LFE and at least 384 kb/s. Illegal rate/layout combinations return errors rather than
changing the requested codec or silently reducing channel count.

PCM planes have equal lengths, finite values in ±1, and the explicit speaker order returned
by `channel_layout()`. Values outside this input contract are rejected. Ordinary 5.1 uses
**L, R, C, LFE, Ls, Rs**; discrete 7.1 uses WAVE order **L, R, C, LFE, Lrs, Rrs, Ls, Rs**.
Dual mono is an unordered layout of programme 1, programme 2 and optional LFE. Mono surround
and other intermediate modes retain their distinct speaker masks rather than using only a
channel count.

The encoder performs windowed long/short transforms with transient switching, adaptive
D15/D25/D45 exponent grouping and reuse, standardized masking/bit allocation, grouped and
asymmetric mantissa quantization, and CRC generation. Rate control enforces both AC-3
regional bit budgets as well as the total frame size; sparse frames use bounded skip fields
so auxiliary padding cannot violate the final-three-eighths constraint. LFE receives a
stateful six-pole 120 Hz low-pass before analysis. The initial implementation does not use
channel coupling, stereo rematrixing, AHT, spectral extension, enhanced coupling or VBR;
these are optional coding tools whose absence limits efficiency, particularly at low rates.
The emitted dialogue normalization is 31 and no dynamic-range gain processing is requested.

A 7.1 E-AC-3 packet contains an independent 5.1 compatibility downmix followed by a dependent
substream carrying the original side and rear pairs (`chanmap = 0x1a00`). The compatibility
surrounds average side and rear inputs, **−6 dB per input**, so correlated full-scale signals
retain headroom. This differs from the optional Annex H `dmixtyp=1` −3 dB mix; that metadata
is not signalled. Full 7.1 decoding restores the original surrounds. The `dec3` descriptor
reports one dependent substream and the added rear pair, not the replacement side pair.

`push` accepts arbitrary chunk sizes and returns complete 1536-sample access units.
`finish` drains overlap and returns final packets plus **256 priming samples** and exact
trailing padding; an empty stream returns no packets and zero trimming. A muxer should use
these counts once in its presentation timing. Raw elementary streams do not carry the
original arbitrary sample length, so raw-file consumers cannot recover this trimming by
inspecting the bitstream alone. `codec_configuration()` returns the `dac3` or `dec3` payload,
without its box header, for `ac-3`/`ec-3` sample entries. Dependent syncframes are part of the
same timed packet, not separately timestamped samples. Input validation errors consume no
PCM; an internal encoding error terminates that encoder, and its partial stream must be
discarded rather than retrying input with ambiguous consumption.

`ac3::ProgrammeDecoder::new(expected_layout)` provides a separate, strict complete-access-unit
path for the encoder's six-block E-AC-3 topology. It checks CRCs, bounds, substream order,
layout, block counts and dependent channel maps before publishing PCM. Its explicit
full-range policy does not apply metadata DRC independently to component substreams.
Unsupported topology is rejected rather than reported as a successful reduced-channel decode.
The older single-syncframe decoder's contract is preserved.

Validation includes all sample rates/coding modes, channel isolation, CRC and actual decoder
bit-position checks, short/long transition reconstruction, LFE response, chunk invariance,
invalid input and exact trim accounting. Independent executable checks cover original
impulse/tone/noise boundary stimuli and 7.1 channel identities, plus externally
produced 7.1 programme metadata. Synthetic SNR is a diagnostic, not a perceptual quality
certificate: broad dialogue/music listening, platform player behaviour and peak-memory
qualification remain separate checks. Lossy ringing can exceed the source PCM peak.

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
- AAC in CAF: the priming is not trimmed and `Decoder::seek` is not sample-exact, as CAF goes
  through symphonia.
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
set `CRAFT_MEDIA_REQUIRE_ORACLES=1`. The helpers and their variables are shared with craft-media
(see [its tests](../media/README.md#tests)). The test files describe what they check;
`tests/codecs/routing_oracle.rs` is the end-to-end check of containers, codecs, timing, seeking
and track choice against ffmpeg.

The Opus conformance test needs the RFC 8251 vectors (75 MB, not committed). Run the script from
the repository root; it fetches them into `target/fixtures/opus/vectors`. With
`CRAFT_MEDIA_FIXTURES_DIR` set, pass the directory:

```bash
scripts/fetch-opus-vectors.sh
```

```bash
scripts/fetch-opus-vectors.sh "$CRAFT_MEDIA_FIXTURES_DIR/opus/vectors"
```

CI (`.github/workflows/audio.yml`) runs clippy for each feature on its own, rustdoc with warnings
as errors, the WASM build, encode-only and native-only test runs, and an MSRV check. It also runs
the full tests with ffmpeg and the vectors required.

## Origin

| Part | Source |
|---|---|
| WAV, AIFF, `decode`/`probe`, peaks | SoundCraft `soundcraft-audio-io` [7dc56a4](https://github.com/storytold/soundcraft/commit/7dc56a4dbeebf57eacece09536726c57844fc59d) |
| FLAC encoder, AAC, Opus decoder, AC-3, MP1/MP2, Ogg | FilmCraft [8fda2f2](https://github.com/storytold/filmcraft/commit/8fda2f232198c4c79ff9cfe5b6a347bb1375fa9e) |
| MP3 encoder | FilmCraft `filmcraft-mp3enc` [4bec22b](https://github.com/storytold/filmcraft/commit/4bec22bd03bd1d995c9cf72e02a284f96d3d72cd) |
| ALAC | Apple's reference implementation [macosforge/alac@c38887c](https://github.com/macosforge/alac/tree/c38887c5c5e64a4b31108733bd79ca9b2496d987/codec), Copyright © 2011 Apple Inc., Apache-2.0; ported to Rust and modified (see the `alac` module docs and [NOTICE](../../NOTICE)) |

Where apps had overlapping implementations, the one that measured better was kept, with the
tests of both:

| Format | Kept | Reason |
|---|---|---|
| FLAC encoding | FilmCraft | 0.1–37% smaller output than SoundCraft's on 12 test signals; STREAMINFO MD5 |
| WAV/AIFF writing | SoundCraft | only one with BWF, dither, arbitrary speaker masks, streaming without known length |
| AAC-LC, MP1/MP2 decoding | FilmCraft (over symphonia) | equal SNR against ffmpeg; also decodes 5.1 AAC and MP2 bit rates symphonia rejects |

## Migration notes for apps

For the apps switching from their own code; to be removed once all three have switched.

The codec modules' public items keep their names, so apps can re-export them
(`pub use craft_audio::aac::*;`). Names kept only for that are `#[doc(hidden)]`:
`ac3::Decoded` (now `ac3::Frame`), `aac::Upsample2x`, `aac::ProgramConfig::parse`,
`aac::config_layout` and `aac::sample_rate_index`. What else an app adjusts:

- `AudioInfo` has `container`, `codec`, `layout` and `damaged` instead of SoundCraft's `format`.
  `detect_container` replaces `detect_format`. `probe` reports the layout instead of
  `wav_channel_mask`.
- SoundCraft's `FileFormat` tables are `DECODE_EXTENSIONS` (was `readable_extensions`),
  `ENCODE_EXTENSIONS` and `encode_format_for` (was `from_write_extension`), and
  `EncodeFormat::ALL`, `extension()` and `mime_type()` (were `WRITE`, `default_extension` and
  `mime_type`). `aifc` is a writable name here.
- `EncodeOptions` is built with `EncodeOptions::new(format)` and `with_*` methods. It carries the
  speaker layout and RF64, which replace `encode_with_layout`, `encode_wav_rf64` and
  `FileEncoder::new`'s layout argument. `BwfInfo` and `aac::EncoderConfig` are built the same way
  (`with_adts`, …). `ENCODE_EXTENSIONS` is a list.
- Arguments go sample rate, then channels, and channel counts are `u16`:
  `FileEncoder::new(out, &opts, sample_rate, channels)`, `wav_float_header(sample_rate,
  channels, frames)`, `AudioBuffer::new(sample_rate, channels, frames)`,
  `AudioBuffer::from_interleaved` and `num_channels`, and the `ChannelLayout` constructors and
  `channels()`.
- `AudioInfo`, `AudioError`, `EncodeOptions`, `BwfInfo`, `SampleFormat`, `BitDepth`, the codec
  `Error` enums and the codec modules' header, frame and config structs (but for
  `aac::ProgramConfig`) are `#[non_exhaustive]`.
  `Speaker::ALL` is a slice.
- `TrackInfo::sample_rate` and `channels` are `Option`s.
- Features are per codec. SoundCraft's `encode` (WAV, AIFF, FLAC) is now `lossless-encode`, made
  of `pcm-encode` (WAV, AIFF) and `flac-encode`. `encode` turns on every encoder, including AAC,
  ALAC and AC-3/E-AC-3. Writing a format whose feature is off returns `AudioError::Unsupported`.
- AAC encoder settings it cannot use are `aac::Error::InvalidSettings` (`AudioError::Encode`).
- `TrackDecoder::decode` returns an error for a corrupt Opus packet, as for the other codecs;
  call `conceal`.
- `FileEncoder::write` takes any per-channel slices, but refuses a block with missing channels or
  short slices instead of filling them with silence.
- `DecodeOptions` is not `Copy`.
- The FLAC encoder's vendor string is `craft-audio <version>`.
- `decode` tries native decoders first. AAC, ALAC, Opus, AC-3 and MP2 no longer go through symphonia, except AAC in
  CAF.
  MP4 and Matroska files decode to presentation length. A packet that fails is concealed, not
  dropped. A failed source read is `AudioError::Io`.
