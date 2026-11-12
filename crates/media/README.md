# craft-media

Media containers and bit-level I/O for the Craft apps: an MSB-first bit reader and writer,
clean-room ISO BMFF (MP4), QuickTime (MOV) and Matroska (MKV, WebM) and MXF demuxers and muxers, and
Ogg, AVI and MPEG-2 Systems (transport and program stream) demuxers, written from the public specifications. No `unsafe`, no panics on malformed
input, builds for `wasm32-unknown-unknown`. Dependencies: `thiserror` (for `isobmff`), `chrono`
(for `mxf`), `serde` (optional).

Codecs are in separate crates that depend on this one: [`craft-audio`](../audio/README.md) for
audio; `craft-video` for video. What both need lives here: the containers, the
bit I/O, one `ByteSource` trait, and the [shared identifiers](#containers-codecs-and-codec-parameters)
that codec crates use to describe a file and configure a decoder.

## Features

| Feature | Module | Enables |
|---|---|---|
| `bitstream` | `craft_media::bitstream` | none |
| `isobmff` | `craft_media::isobmff` | `bitstream`, `thiserror` |
| `matroska` | `craft_media::matroska` | none |
| `ogg` | `craft_media::ogg` | none |
| `avi` | `craft_media::avi` | none |
| `mxf` | `craft_media::mxf` | `chrono` |
| `mpegts` | `craft_media::mpegts` | none |
| (none) | `Container`, `CodecId`, `CodecParams`, `PcmFormat`, `xiph_lace`, `ByteSource`, `ReadSeekSource` | none |
| `serde` | `Serialize`/`Deserialize` on `Container`, `CodecId`, `CodecParams` and `PcmFormat` | `serde` |

`default = ["isobmff"]`, so `bitstream` and `isobmff` build. `matroska` is opt-in: it is about as
much code as `isobmff`, and most users of MP4/MOV (LightCraft, FigCraft, SoundCraft, EffectCraft)
do not read Matroska. `ogg`, `avi`, `mxf` and `mpegts` are opt-in as well. The features are independent.

```toml
craft-media = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["matroska"] }
```

A crate that needs only bit I/O (a codec) uses:

```toml
craft-media = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", default-features = false, features = ["bitstream"] }
```

## Boundaries

- In scope: bit-level reading and writing, and container demuxing and muxing (boxes, sample
  tables, sample entries and codec configuration records) for audio and video tracks.
- Out of scope: codecs. Audio codecs are in `craft-audio`; video codecs (H.264, HEVC, ProRes,
  DNx, VP8, VP9, AV1) are in `craft-video`. Containers only FilmCraft reads (MXF,
  MPEG-TS/PS, AVI) are not here yet.
- Anything both the audio and the video codec crates need belongs here, so the two never depend
  on each other.

## Origin and API

The code comes from FilmCraft's `filmcraft-bitstream`, `filmcraft-isobmff` and
`filmcraft-matroska` crates at
[storytold/filmcraft@6c3948a](https://github.com/storytold/filmcraft/commit/6c3948a36c80e4996d924766ba7157eec4fca85e).
Public items keep their names and signatures, so FilmCraft can turn all three crates into
re-exports (`pub use craft_media::isobmff::*;`), except for the renames below. Our own names use
US spelling; names taken from a specification keep its spelling.

Renamed:

| FilmCraft | craft-media |
|---|---|
| `matroska::Colour` | `matroska::Color` |
| `matroska::VideoInfo::colour` | `matroska::VideoInfo::color` |
| `matroska::VideoInfo::colour_space` | `matroska::VideoInfo::color_space` |

The element IDs `element_ids::COLOUR` and `COLOUR_SPACE` and the field
`isobmff::VpcConfig::colour_primaries` keep the spellings of the Matroska and VP9 specifications.

Output that differs on valid input:

- Matroska lacing applies only to tracks with a `DefaultDuration`, and a lace ends at a timestamp
  gap. Readers space the frames of a lace by `DefaultDuration`, so FilmCraft's laces moved frames
  that followed a gap.
- The sample budget covers the whole file, so a multi-track file with more than 2²⁶ samples in
  total fails with `TooLarge` (ISO BMFF: all tracks and fragments; Matroska: all tracks, and at
  most 2¹⁶ TrackEntries). FilmCraft capped ISO BMFF per track and Matroska not at all.
- MOV and MP4 PCM chunks end at 1 MiB, as ffmpeg writes them; FilmCraft wrote
  each contiguous run of one track as one chunk. The reader splits a PCM chunk above 16 MiB into
  samples of at most 16 MiB; FilmCraft made it one sample, cut at 4 GiB.
- MOV PCM at 65,536 Hz and above, and unsigned 16-bit PCM, is written as `lpcm` (sound
  description v2), and signed 8-bit PCM as `twos`. FilmCraft wrote 0 Hz for the former and
  labeled the signedness wrongly for the latter two. `raw ` under a sound handler reads as PCM.
- Matroska `BlockDuration` is rounded to the nearest tick; FilmCraft truncated it.
- A Matroska `TimestampScale` above 2³² is ignored, like a zero scale.
- `matroska::MkvWriter` writes `MuxingApp` = `craft-media`, and `MuxOptions::default()` has
  `writing_app` = `craft-media` (FilmCraft: `filmcraft-matroska` and `filmcraft`). Apps set
  `writing_app` to their own name.

Other differences:

- `bitstream::BitReader::read_ue_plus` (from DeckCraft's copy) is new.
- `isobmff::boxes` is new: FilmCraft's private box iterator and byte reader, made public (see
  [Box reader](#box-reader)).
- `BitReader::peek`, `BitReader::read_bits` and `BitWriter::write_bits` accept widths above 32
  without a debug assertion: `peek` and `read_bits` return the low 32 bits of the field (use
  `read_bits_u64` for wide fields), and `write_bits` writes leading zero bits, at most 64 bits in
  all.
- `bitstream::length_prefixed_nals` returns the new `BitError::InvalidNalLengthSize` for a
  `length_size` outside 1 to 4 (as FilmCraft does since filmcraft#2953) and cannot overflow on a
  hostile length prefix.
- `isobmff::PcmConfig::bytes_per_frame` saturates at `u32::MAX` for a hostile channel count.
- `matroska`: a cluster timestamp above `i64::MAX` saturates at `i64::MAX` (FilmCraft wraps it to
  a negative value), and adding a block's relative timestamp saturates instead of overflowing (a
  debug-build panic in FilmCraft).
- `matroska::Error::TooLarge` is new: indexing a file past the limits above returns it.
- `matroska::MuxOptions::max_lace_frames` is clamped to 1 to 256; FilmCraft wrote a corrupt
  frame count above 256.
- `isobmff::Mp4Writer::set_sample_entry` returns an error when the new entry changes the codec,
  the media type or the PCM frame size, and `Error::NoSuchTrack` for a bad index. FilmCraft
  accepted any entry.
- `isobmff::Mp4Writer::write_sample` returns an error when a track would pass 2³² − 1 samples
  (PCM: frames), the most `stsz` can count; FilmCraft clamped the count.
- `isobmff::ByteSource`, `matroska::ByteSource` and `ogg::ByteSource` are one trait,
  `craft_media::ByteSource` (with `ReadSeekSource`, which was Matroska's). A type FilmCraft
  implements both traits for keeps one of its two impls.
- `isobmff::Track`, `matroska::Track`, `ogg::Stream`, `avi::Stream`, `mxf::EssenceTrack` and `mpegts::Stream` have `codec_params()`, all returning a
  `CodecParams` (`CodecId::Unknown` for a codec without an identifier). The MP4 one describes
  the first sample's sample description; `codec_params_for(description_index)` gives any other,
  and `has_mixed_descriptions()` says whether the samples use more than one.
  `isobmff::CodecConfig` has `codec_id()` and `extradata()`, and `OpusConfig::opus_head()`.
- `ogg::Codec` and `ogg::Error`, and the `ogg` structs, are `#[non_exhaustive]`.

## `bitstream`

- `BitReader`: MSB-first reader with Exp-Golomb (`ue(v)`, `se(v)`, `te(v)`), as used by
  H.264/HEVC/AV1 headers.
- `BitWriter`: the matching writer, for encoders and header rewriting.
- `unescape_rbsp` / `escape_rbsp`: remove or insert H.264/HEVC emulation-prevention bytes.
- `annexb_nals` / `length_prefixed_nals`: split Annex-B byte streams and AVCC/HVCC samples into
  NAL units.
- `crc32`: CRC-32 (IEEE), as used by Matroska and PNG.

## `isobmff`

### Demuxing

```rust
use craft_media::isobmff::{open, TrackKind};

let file = std::fs::File::open("clip.mov")?;
let mp4 = open(&file)?;                        // any ByteSource: &[u8], Vec<u8>, Arc<[u8]>, File, ...
let v = mp4.track_of_kind(TrackKind::Video).unwrap();
let track = &mp4.tracks[v];
let i = track.sample_at_presentation_time(1000).unwrap();   // track timescale, edit-adjusted
let key = track.sync_sample_before(i);          // decode from here
let bytes = mp4.read_sample(&file, v, key)?;    // raw sample (AVCC/HVCC length-prefixed NALs, ...)
```

- `ByteSource` (`len`, `read_at(offset, buf)`) is the input abstraction. Implementations exist for `[u8]`,
  `Vec<u8>`, `&T`, `Arc<T>`, `Box<T>`, and `std::fs::File` on non-wasm targets.
- `open` reads only top-level box headers, `ftyp`, `moov` and every `moof`. It never reads `mdat`.
  The one exception is the first sample of a `tmcd` track, which it reads (4 bytes) to get the timecode
  start frame.
- `Mp4File`: brands, `is_quicktime`, movie `timescale`/`duration`, `tracks`, `metadata`, `fragmented`,
  `fragment_count`, `chpl_chapters` (Nero `udta/chpl`). `read_chapters(src)` returns the chapter list:
  the QuickTime chapter track (`tref/chap`) when there is one, else `chpl`.
- `Track`: `id`, `kind` (Video/Audio/Timecode/Subtitle/Other), `handler`, `timescale`, `duration`,
  `language`, `tkhd` fields (size, matrix, volume, enabled), `entries: Vec<SampleEntry>`, `samples`,
  `edits`, `edit_offset`, `references` (`tref`), `sdtp`, `sample_groups` (`sbgp`+`sgpd`, raw), `pcm_chunked`.
  Helpers: `codec()`, `video()`, `audio()`, `presentation_pts(i)`, `sample_at_pts(pts)`,
  `sample_at_presentation_time(t)`, `sync_sample_before(i)`, `sync_samples()`.
- `Sample`: `offset`, `size`, `dts`, `pts` (dts + ctts), `duration`, `is_sync` (all samples are sync when
  `stss` is absent), `description_index`. All times are raw media time in the track timescale.
- **Edit lists:** `edits` holds the `elst` entries unchanged. `edit_offset` is
  `leading empty edits (rescaled to media timescale) − first non-empty edit's media_time`, so
  `presentation time = pts + edit_offset`. This matches ffprobe's timestamps for the common cases
  (B-frame delay, AAC priming, empty-edit offsets). Multi-segment edits, dwells and rate changes are exposed
  raw for the caller to interpret.
- **Uncompressed PCM** (`lpcm`, `sowt`, `twos`, `in24`, `in32`, `fl32`, `fl64`, `raw `, `ipcm`, `fpcm`) is
  merged to one `Sample` per chunk (`pcm_chunked = true`), with `duration` in frames. This avoids one
  table entry per audio frame. A chunk above 16 MiB becomes several samples of at most 16 MiB.

#### Parsed boxes
`ftyp`, `moov/mvhd`, `trak/tkhd`, `edts/elst` (v0/v1), `tref`, `mdia/mdhd` (v0/v1, ISO and Mac language
codes), `hdlr` (ISO and Pascal names), `minf/stbl`: `stsd`, `stts`, `ctts` (v0/v1, read as signed), `stss`,
`stsc`, `stsz`/`stz2` (4/8/16-bit), `stco`/`co64`, `sdtp`, `sbgp`/`sgpd`, `udta/chpl`. Also `mvex/trex`, `moof/traf/tfhd/tfdt/trun`
(every `tfhd` base-offset mode and multiple `moof`s), and `udta` metadata (QuickTime `©xxx` atoms,
`meta/ilst`, and `meta/keys` (mdta)). `uuid`, `free`, `skip`, `wide`, `sidx` and `mfra` are skipped.
Boxes with 64-bit sizes and with size 0 (runs to the end of the file) are supported.

#### Sample entries → `CodecConfig`
| Format | Config |
|---|---|
| `avc1`/`avc3` + `avcC` | `Avc(AvcConfig)`: profile, level, length size, SPS/PPS, extension bytes |
| `hvc1`/`hev1` + `hvcC` | `Hevc(HevcConfig)`: all header fields plus NAL arrays (`vps()/sps()/pps()`) |
| `av01` + `av1C` | `Av1(Av1Config)` |
| `vp09` + `vpcC` | `Vp9(VpcConfig)` |
| `apch apcn apcs apco ap4h ap4x` | `ProRes { fourcc }` |
| `jpeg mjpa mjpb` | `Jpeg { fourcc }` |
| `AVdn AVdh` | `Dnx { fourcc }` |
| `mp4a` + `esds` (in `wave` too) | `Aac(AacConfig)`: ASC bytes + decoded object type/rate/channels; `Mp3` for OTI 0x69/0x6B |
| `.mp3`/`.mp2` | `Mp3`: MPEG-1/2 audio of any layer (each frame header names it; `Track::codec_params()` reports `Mp1`, `Mp2` or `Mp3` from the first frame) |
| PCM fourccs | `Pcm(PcmConfig)`: bits, float, endianness (`enda`, lpcm flags, `pcmC`), channels, rate |
| `alac` | `Alac { cookie }` |
| `Opus` + `dOps` | `Opus(OpusConfig)` |
| `fLaC` + `dfLa` | `Flac(FlacConfig)` (STREAMINFO decoded) |
| `ac-3`/`ec-3` | `Ac3 { dac3 }` / `Eac3 { dec3 }` |
| `tmcd` | `Timecode(TimecodeConfig)`: flags (drop frame), timescale, frame duration, fps, start frame, reel name; `format_frame()` |
| anything else | `Unknown { fourcc, raw }` |

Video entries also parse `colr` (`nclx`/`nclc`/ICC), `pasp`, `clap`, `fiel`, `gama` and `btrt` into
`VideoParams`, and QuickTime's `logs` box into `VideoParams::log_transfer_function`: the CoreMedia log
transfer-function identifier (`com.apple.rec2020.apple-log` for Apple Log), kept as text whatever
curve it names. It is non-empty UTF-8 without NUL of at most 255 bytes; anything else, or a second
`logs` box, reads as unknown, and the writer refuses an identifier the reader would not accept. Audio entries parse QuickTime sound description v0/v1/v2 into `AudioParams`.

#### Robustness
The parser does not panic on malformed input; every error comes back as an `Error`. Table counts are
checked against the bytes in their box before anything is allocated. A constant-size `stsz` count is
clamped to what fits in the file. `trun` counts are checked, and the whole file (all tracks and
fragments) is capped at 2²⁶ samples.
`moov`/`moof` are capped at 1 GiB. Compressed movie headers (`cmov`) are rejected with
`Error::Unsupported`.

### Muxing

```rust
use craft_media::isobmff::*;
let mut w = Mp4Writer::new(std::io::Cursor::new(Vec::new()), WriterOptions::new(Brand::Mp4))?;
let v = w.add_track(TrackConfig::new(SampleEntry::avc(avcc, 1920, 1080), 90_000))?;
let mut acfg = TrackConfig::new(SampleEntry::aac(asc, 2, 48_000), 48_000);
acfg.media_start = Some(1024);                 // AAC encoder delay → edit list
let a = w.add_track(acfg)?;
w.write_sample(v, WriteSample { data: &au, duration: 3000, composition_offset: 6000, is_sync: true })?;
// ... interleave samples in decode order ...
let bytes = w.finish_faststart()?.into_inner(); // or finish() for moov-at-end
```

- `Mp4Writer<W: Write + Seek>` streams samples into a single `mdat`. `finish()` appends `moov`.
  `finish_faststart()` (needs `W: Read + Write + Seek`) moves the media data forward in place and writes
  `moov` before it. If `mdat` goes past 4 GiB the header becomes a large-size header, and chunk offsets
  switch to `co64` when needed.
- Brands: `Brand::Mp4` writes `isom` with `isom iso2 avc1 mp41 mp42`. `Brand::Mov` writes `qt  `, uses
  QuickTime `hdlr`/Pascal names, and sets `wide` as the placeholder box.
- Tables written: `stts`, `ctts` (v1 when an offset is negative), `stss` (omitted when every sample is
  sync), `stsc`, `stsz` (constant or table), `stco`/`co64`. Consecutive samples of one track share a chunk;
  a PCM chunk ends at 1 MiB.
- Sample entries come from `SampleEntry` (`avc`, `hevc`, `prores`, `jpeg`, `aac`, `pcm`, or any demuxed
  entry). Remuxing copies codec config, `colr`, `pasp`, `clap`, `fiel`, `gama` and `btrt`.
  PCM is written as `sowt`/`twos` (16-bit) or `raw ` (8-bit) or sound description v2 `lpcm` in MOV, and as
  `ipcm`/`fpcm` + `pcmC` in MP4. A PCM `write_sample` call can hold any whole number of frames, and the
  track timescale must equal the sample rate.
- Edit lists: set `TrackConfig::edits` explicitly, or use `media_start` to get a single edit.
- Timecode: `add_timecode_track(video, TimecodeConfig, start_frame)` adds a `tmcd` track (with `gmhd`)
  and a `tref/tmcd` link from the video track.
- `WriterOptions::metadata` writes `©xxx` text atoms to `udta`.
- `WriterOptions::chapters` (`Chapter { start, title }`, starts in the movie timescale) writes a chapter
  list in both forms players read: a QuickTime chapter track (a disabled `text` track with `gmhd`,
  one length-prefixed UTF-8 sample plus `encd` atom per chapter, referenced by `tref/chap` from the first
  video or audio track; Apple's QuickTime File Format Specification, 2016 edition, "Chapter lists"
  and "Text media") for Apple players, and a Nero `udta/chpl` list (version 1, 100 ns units; there is
  no official specification, this is the publicly documented layout) for VLC and ffprobe. Chapters are
  sorted, one per start time, cut to the movie duration, at most 255 (`MAX_CHAPTERS`) with titles of at
  most 255 bytes (`MAX_CHAPTER_TITLE_BYTES`, cut at a character boundary). A first chapter after the
  start is placed with an empty edit. The fragmented writer writes only `chpl`.
- `FragmentedWriter<W: Write>` writes fMP4. The init segment is `ftyp` + `moov` with `mvex/trex`. Each
  `flush_fragment()` then writes `moof` (`mfhd`, `traf` with default-base-is-moof `tfhd`, `tfdt` v1,
  `trun` with per-sample duration, size, flags and composition offset) followed by `mdat`.

### Box reader

`isobmff::boxes` gives direct access to the box format, for files and boxes the demuxer does not
model: CR3 `uuid` boxes, HEIF `meta` items, JPEG 2000 (JP2, JPX, JPM, MJ2) boxes.

```rust
use craft_media::isobmff::boxes::{BoxIter, Reader};

for b in BoxIter::new(&bytes) {
    let b = b?;
    if b.kind == b"uuid" && b.user_type == Some(CANON_UUID) {
        for c in BoxIter::within(&bytes, b.payload_range()) { /* CMT1..CMT4, THMB */ }
    }
}
```

- `BoxIter::new(data)` yields the sibling boxes in `data`; `BoxIter::within(data, range)` yields
  those in `data[range]` with offsets relative to `data`, so a walker keeps file offsets as it
  descends. Each `BoxRef` has `kind`, `user_type` (the 16-byte extended type of a `uuid` box),
  `offset`, `header_len`, `payload`, `payload_range()`, `range()` and `end()`.
- 64-bit `largesize` and size 0 (runs to the end of the range) are handled. A malformed header or a
  box larger than the range yields one `Err` and ends the iteration; `allow_truncated()` instead
  yields that last box cut at the end of the range, with `truncated` set. A 32-bit zero terminator
  (QuickTime) ends the iteration.
- `BoxHeader::parse(buf)` reads one header (up to `BoxHeader::MAX_LEN` = 32 bytes), for scanning a
  file through `ByteSource` without loading it.
- `Reader` is the bounds-checked big-endian cursor the demuxer uses: `u8` to `u64`, `i16` to
  `i64`, `f64`, `u24`, `fourcc`, `full_header` (version and flags), `bytes`, `skip`, `rest` and
  `count` (an entry count checked against the bytes left, before allocating).
- The iterator does not recurse. The caller bounds nesting depth and box count.

### Limitations
- `presentation time = pts + edit_offset` only applies the leading empty edits and the first media
  edit. Multi-segment edit lists, dwell edits and non-unity rates are exposed raw. ffmpeg handles an edit
  that starts mid-frame differently: it snaps to the frame.
- Other data references (external `alis`/`url ` files), encrypted entries (`encv`/`enca`, `sinf`,
  `senc`), `cmov`, and QuickTime reference movies are not supported.
- `sidx`/`mfra` are ignored, because the demuxer scans every top-level box. `stsd` entries with a
  QuickTime color table are not decoded.
- Subtitle and text tracks (`tx3g`, `c608`, `wvtt`) and `mp4v` show up as `Unknown`.
- Muxer: one `mdat`, no chunk-size or duration limits outside PCM (the caller controls interleaving), no `sdtp`,
  `sgpd`/`sbgp` or `cslg` output, ISO-style `meta/ilst` metadata is not written, and the fragmented
  writer has no PCM, `sidx` or `mfra`.
- Chapters: at most 255 per file and 255 bytes per title (the `chpl` count and length fields are one
  byte each); chapters past the end of the movie are dropped. Only the first `tref/chap` reference is
  read, and chapter-track samples are read as titles only (`styl`/`hlit` atoms and per-chapter URLs or
  images are ignored). ffprobe ends the last chapter early by the empty edit when the first chapter
  starts after zero.

## `matroska`

A clean-room Matroska (MKV) and WebM demuxer, plus a minimal muxer, written from RFC 8794 (EBML),
RFC 9559 (Matroska), the Matroska codec mappings and the WebM container guidelines. No GPL or LGPL
code (FFmpeg, libmatroska, mkvtoolnix) was consulted; FFmpeg is a test oracle only.

### Demuxing

```rust
use craft_media::matroska::{Demuxer, TrackKind, open};

// Sample tables in the same shape as isobmff (any ByteSource: &[u8], Vec<u8>, Arc<[u8]>, File, ...)
let file = std::fs::File::open("clip.mkv")?;
let mkv = open(&file)?;
let v = mkv.track_of_kind(TrackKind::Video).unwrap();
let k = mkv.keyframe_before(v, 2_000_000_000).unwrap();   // sample index
let frame = mkv.read_sample(&file, v, k)?;

// Streaming packets and seeking (from a ByteSource, any Read + Seek, or a byte slice)
let mut d = Demuxer::from_reader(std::io::BufReader::new(std::fs::File::open("clip.webm")?))?;
let sp = d.seek(v, 2_000_000_000)?;          // lands on the keyframe with pts <= 2 s
while let Some(p) = d.next_packet()? {        // file order, all tracks
    // p.track, p.pts (ticks), p.pts_ns, p.duration, p.keyframe, p.data
}
```

- **Open** (`open`, `open_with(src, &OpenOptions { index, verify_crc, index_limit })`): EBML header (DocType
  `matroska` or `webm`), the first Segment, and its level-1 elements: SeekHead (followed for
  elements after the clusters), Info, Tracks, Cues, Chapters, Tags, Attachments (listed with offset
  and size; payloads are not read). With `index: true` (the default) every Cluster is scanned,
  element and block headers only, to build per-track `Sample` tables (`offset`, `size`, `pts`,
  `duration`, `keyframe`, cluster, block offset, lace index, `BlockAdditional` location) and
  keyframe lists. With `index: false`, `open` stops at the first Cluster.
- **Progressive index** (`index_limit: Some(offset)`): the scan stops at that byte offset, so a
  source that has loaded only the front of the file opens at once. The first Cluster is always
  scanned; later ones only if they end below the limit. `MkvFile::index_end` is `Some(offset)`
  while the index is incomplete (`indexed` is `false`; the tables are exact for everything before
  it; Cues after the scanned part are not read yet), and `MkvFile::extend_index(src, &opts)`
  continues from there with a larger limit (`None`: to the end, which completes the index)
  without rescanning. A read that fails part-way (data not loaded yet) can be retried.
- **Timestamps:** Matroska ticks are `TimestampScale` ns (usually 1 ms); `Track::timebase` is that
  as a reduced fraction of a second. `pts` is in ticks, `pts_ns` in exact nanoseconds. `CodecDelay`
  is subtracted (RFC 9559), as FFmpeg does, so Opus and Vorbis priming gives negative pts. Laced
  frames get `block ts + i × (BlockDuration or DefaultDuration) / n`; when neither is known, later
  laces repeat the block timestamp and have `duration == 0` (`Packet::lace` tells which).
- **Keyframes:** the SimpleBlock keyframe flag; a BlockGroup is a keyframe when it has no
  ReferenceBlock.
- **Lacing:** Xiph, fixed-size and EBML. **Unknown sizes:** a Segment (live or streamed output) and
  Clusters end at the next top-level element. **CRC-32:** optional verification of level-1
  elements and clusters; mismatches go to `MkvFile::crc_errors` and are not fatal.
- **Damage:** an undecodable or oversized element starts a byte scan for the next plausible level-1
  element (a Cluster whose first child is Timestamp, or a sized metadata element); a truncated final
  block is dropped. Problems are listed in `MkvFile::warnings`.
- **Limits:** indexing stops with `Error::TooLarge` past 2²⁶ frames in the file, past one frame
  per file byte (each frame counts its stored size, and at least one byte), or past 2¹⁶
  TrackEntries. Level-1 elements read at most the file's length in all, so SeekHead entries that
  point into one another are skipped with a warning.
- **Seeking** (`Demuxer::seek(track, time_ns)`): uses the sample index when present, otherwise Cues
  (refined by scanning the cued cluster for the exact keyframe), otherwise builds the index by
  scanning all clusters. `Demuxer::keyframe_index(track)` returns the keyframe list.
- **Block additions:** `MkvFile::read_sample_addition` returns the `BlockAdditional` with
  `BlockAddID` 1 of an unlaced block (in WebM, the VP8/VP9 alpha layer).
- **Tracks:** number, UID, type, flags, `DefaultDuration`, name, language (and BCP 47), CodecID,
  CodecPrivate, CodecName, `CodecDelay`, `SeekPreRoll`; `VideoInfo` (pixel and display size, crop,
  interlacing, stereo, alpha mode, `pixel_aspect()`, `display_rotation()`), `Color` (matrix,
  range, transfer, primaries, subsampling and siting, MaxCLL/MaxFALL, SMPTE 2086 mastering
  metadata); `AudioInfo` (rate, output rate, channels, bit depth); `ContentEncoding`s.
- **Content encodings:** header stripping is undone in every packet and in `read_sample`. zlib,
  bzlib and lzo compression and encryption are reported (`Track::frames_readable() == false`);
  packets are then returned as stored and `read_sample` fails with `Unsupported`.
- `matroska::ebml` (public) has the EBML primitives: variable-length integers, element headers,
  typed values, CRC-32 and lacing. `matroska::element_ids` lists the element IDs.

#### Codec mapping (`Codec`)

| CodecID | `Codec` | Payload |
|---|---|---|
| `V_MPEG4/ISO/AVC` | `Avc` | avcC (CodecPrivate) |
| `V_MPEGH/ISO/HEVC` | `Hevc` | hvcC |
| `V_VP8`, `V_VP9` | `Vp8`, `Vp9` | VP9 CodecPrivate features |
| `V_AV1` | `Av1` | av1C |
| `V_APV` | `Apv` | apvC |
| `V_PRORES` | `ProRes` | FourCC; frames get the 8-byte `size` + `icpf` header re-added |
| `V_MJPEG`, `V_MS/VFW/FOURCC` (MJPG) | `Mjpeg` | |
| `V_MS/VFW/FOURCC` (other) | `VfwFourcc` | BITMAPINFOHEADER |
| `A_AAC`, legacy `A_AAC/MPEG{2,4}/...` | `Aac` | AudioSpecificConfig (synthesized for legacy IDs, SBR included) |
| `A_OPUS` | `Opus` | OpusHead |
| `A_VORBIS` | `Vorbis` | three headers split from the Xiph-laced CodecPrivate |
| `A_FLAC` | `Flac` | `fLaC` + metadata blocks |
| `A_PCM/INT/LIT`, `A_PCM/INT/BIG`, `A_PCM/FLOAT/IEEE` | `Pcm` | bit depth from Audio |
| `A_AC3`, `A_EAC3`, `A_MPEG/L3`, `A_MPEG/L2`, `A_MPEG/L1` | `Ac3`, `Eac3`, `Mp3`, `Mp2`, `Mp1` | |
| `S_TEXT/UTF8` | `SubRip` | |
| `S_TEXT/WEBVTT`, `D_WEBVTT/*` (WebM) | `WebVtt` | WebM blocks keep `id\nsettings\npayload` |
| `S_TEXT/ASS`, `S_TEXT/SSA` | `Ass` | script header |
| anything else | `Other(id)` | `Track::codec_private` |

`Codec::name()` matches FFmpeg's `codec_name` for these.

### Muxing

`MkvWriter::new(w: Write + Seek, Vec<TrackSpec>, MuxOptions)`, then `write_frame(track, pts_ns,
keyframe, data, duration_ns)` per frame, then `finish()`. It writes the EBML header, Segment,
SeekHead, Info (with Duration), Tracks, Clusters of SimpleBlocks (a new cluster at a video keyframe
after `cluster_duration_ns`), BlockGroups for frames whose duration differs from the track's
`default_duration_ns`, optional audio lacing (Xiph, EBML, fixed-size; for tracks with a
`default_duration_ns`, at most 256 frames, ending at a timestamp gap), and Cues (video keyframes,
or the first frame of each track per cluster). `MuxOptions::doc_type` = `"webm"` writes WebM.
`TrackSpec::color` writes a video track's `Colour` element (matrix, range, transfer, primaries,
chroma layout, light levels and mastering metadata: the fields that are set), which is where a WebM
file says it is Rec. 2020, P3, PQ or HLG.
CodecPrivate is fixed when the writer is created, so a codec whose configuration arrives with the
first frame (AV1 from `rav1e`) creates the writer after that frame.

### Limitations

- Only the first Segment is read (no linked or chained segments, no ordered-chapter playback).
- zlib, bzlib and lzo compressed and encrypted tracks are detected, but their frames are not
  decoded. `EncryptedBlock` is ignored.
- `TrackTimestampScale` (deprecated) is ignored.
- Lace timestamps without any duration information repeat the block timestamp (FFmpeg derives them
  from codec parsers).
- Muxer: no CRC-32, Chapters, Tags, Attachments, content encodings or block additions.

## `ogg`

A clean-room Ogg demuxer from FilmCraft's `filmcraft-ogg` (RFC 3533, RFC 7845 Ogg Opus, the Vorbis
I Ogg mapping). `open` reads every page (CRC-32 checked; damage is skipped by searching for the next
`OggS`), groups pages by logical stream and assembles packets across pages; codec headers are kept
apart from audio packets. A gap in a stream's page sequence numbers drops the packet the lost page
continued, and a beginning-of-stream page for a serial already seen starts a new stream (the next
link of a chained file), unless it carries on the page sequence of a stream with no end-of-stream
page yet. Damage recovery may read bytes again up to twice the file's length plus 1 MiB, so a file
is read at most about three times over; the packets kept and the header packets of a stream are
bounded too, with a warning when a bound is reached.
`OpusTiming` gives each Opus packet's start, the pre-skip and the end trimming of the final page.
No muxer.

## `avi`

A clean-room AVI demuxer from FilmCraft's `filmcraft-avi` (storytold/filmcraft@4bec22b), written
from Microsoft's published AVI RIFF file reference and the OpenDML AVI File Format Extensions 1.02.
No FFmpeg, libavformat or other AVI implementation was read; ffmpeg and ffprobe are test oracles
only. No muxer.

```rust
use craft_media::avi::{open, StreamKind};

let file = open(&bytes)?; // any ByteSource
for (i, s) in file.streams.iter().enumerate() {
    let params = s.codec_params(); // CodecId, FourCC, size or sample rate and channels
    if s.kind == StreamKind::Video {
        println!("{} {} fps", params.codec.name(), s.rate as f64 / s.scale as f64);
    }
    let first = file.read_chunk(&bytes, i, 0)?; // frame data, on demand
}
```

| Part | |
|---|---|
| Headers | `avih`; per stream `strh`, `strf` (`BITMAPINFOHEADER`, `WAVEFORMATEX` / `WAVEFORMATEXTENSIBLE`), `strn` |
| Index | OpenDML `indx` super index → `ix##` standard indexes (field indexes too), else `idx1` (offsets relative to `movi` or absolute), else a scan of the `movi` lists |
| Large files | `RIFF AVIX` extensions: through the OpenDML index, or scanned after `idx1`, which only covers the first RIFF |
| Key frames | from the index; a scan marks every chunk and says so (`keyframes_known`) |
| Damage | sizes clamped to the file (a cut-off recording opens with what is there) |

Limits, each with a test: a header list of at most 16 MiB (larger is refused), at most 64 streams
(more are ignored), `rec ` lists nested at most 3 deep, at most 64 MiB of OpenDML standard indexes
(past that the file opens from `idx1` or a scan), and chunk tables bounded by what the file could
hold.

`Stream::codec_params()` maps `biCompression` and the `WAVEFORMATEX` format tag to a `CodecId`
(Motion JPEG, H.264, HEVC, MPEG-4 Part 2, MPEG-2, VP8/VP9, AV1, ProRes, DNxHD; PCM, ADPCM, A-law,
µ-law, MP2, MP3, AAC, AC-3, Vorbis, FLAC). `extradata` is the bytes after the format header
as stored. For H.264 and HEVC, `Stream::nal_framing()` says what they are: nothing (parameter sets in
the chunks), Annex B parameter sets (a file written with global headers) or an `avcC` / `hvcC` record
(a stream copied from MP4, whose chunks are then length-prefixed by the size it gives;
`bitstream::length_prefixed_nals` splits them). Uncompressed video
is `CodecId::Unknown` with the FourCC in `tag`.

Differences from `filmcraft-avi`: `ByteSource` is `craft_media::ByteSource`; `Error::Io` holds the
`io::Error` (FilmCraft: its text), so `Error` is no longer `Clone`/`PartialEq`, and is
`#[non_exhaustive]`; `Stream::codec_params()` is new.

Tests: hand-built files for every index path, truncation, every single-byte mutation and random
multi-byte damage (never a panic). `tests/oracle_avi.rs` checks ffmpeg-made files (Motion JPEG,
H.264 with B-frames, BGR24, YUY2; PCM and MP3) against ffprobe; the ignored `opendml_file_past_1_gb`
checks a 1.16 GB OpenDML file with `AVIX` extensions.
## `mpegts`

A clean-room MPEG-2 Systems demuxer from FilmCraft's `filmcraft-mpegts`
(storytold/filmcraft@4bec22b): transport streams (188-byte, 192-byte BDAV / M2TS, 204-byte) and
program streams (MPEG-1/2 PS, VOB, MOD). No GPL/LGPL code (FFmpeg, libavformat) was consulted;
FFmpeg is a fixture generator and test oracle only. No muxer, no decoding.

```rust
use craft_media::mpegts::{Kind, open};

let file = open(&bytes)?; // any ByteSource
for s in &file.streams {
    println!("{:?} {} {} units", s.id, s.codec.name(), s.units.len());
}
let v = file.find(Kind::Video, |_| true).unwrap();
let au: Vec<u8> = file.read_unit(&bytes, v, 0)?; // the first access unit
```

- **Transport streams**: 188-byte packets, 192-byte BDAV packets (sync byte after a 4-byte
  arrival timestamp) and 204-byte packets; the packet size is sniffed from five consecutive sync
  bytes. Lost sync resynchronises on the next aligned run. PSI sections are reassembled across
  packets and checked with the MPEG CRC-32; the first program of the PAT is demuxed. Continuity
  counter errors, CRC errors and resyncs are reported as warnings.
- **Program streams**: MPEG-2 and MPEG-1 packs, system headers, the program stream map (stream
  types for elementary stream ids), padding, private stream 2 (DVD navigation) skipped, private
  stream 1 split by sub-stream id. Garbage resynchronises on the next pack or PES start code.
- **Streams**: MPEG-1/2 video, H.264, HEVC, MPEG audio, AAC (ADTS, LATM/LOAS), AC-3, E-AC-3,
  DTS, TrueHD, Blu-ray LPCM, DVD LPCM (format from the sub-stream header), PGS / DVB / teletext
  subtitles; ISO 639 language from the PMT.
- **Access units**, found while the file is scanned (one pass, streaming; only a few bytes of
  each stream are buffered):
  - MPEG-1/2 video: a coded frame — a frame picture, or two field pictures (paired by
    picture_structure) — with the sequence / GOP headers before it; picture type, temporal
    reference, field structure, top_field_first, repeat_first_field, progressive_frame, closed
    GOP / broken link. A PTS belongs to the unit whose picture start code is in that PES.
  - H.264 / HEVC: PES packets with a timestamp (a PES without one continues the previous unit);
    random access from IDR / IRAP NAL types; disposable when every slice is a non-reference.
  - Audio: one frame per unit from the frame headers (sync re-hunted after garbage; a header
    found while hunting is confirmed by the next one); a PTS belongs to the first frame starting
    in the PES.
  - LPCM and everything else: one PES packet per unit.
- **Timestamps**: 33-bit PTS/DTS unwrapped across the wrap (and back, for B pictures shown
  before a wrap that came earlier); PCR / SCR range.
- **Reading**: a unit records only the position of its first packet and its offset in that
  packet's payload; `read_unit` re-reads from there (the index costs a few dozen bytes per
  unit, not per packet).

`frame_bytes`, `frame_info` and `sei_has_recovery_point` are public: other containers that store
MPEG audio or AC-3 frames in chunks that do not follow them (AVI) find the frames again with them.

`Codec::codec_id()` and `Stream::codec_params()` give the crate-wide `CodecId` / `CodecParams`.
MPEG-1 video reports `Mpeg2Video`; MPEG audio reports `Mp2` whatever its layer (the layer is in
each frame header: `frame_info(..).variant`); ADTS and LATM AAC both report `Aac`; DTS, TrueHD and
subtitles have no identifier (`Unknown`). The streams carry their configuration in band, so
`extradata` is empty; DVD LPCM fills `sample_rate`, `channels` and `pcm`.

Limits, each with a test: 2²³ access units per stream (the rest are not indexed, with a warning),
128 MiB per unit read (`MAX_UNIT_BYTES`), a PES header given up after 1024 bytes, 16 stuffing bytes
in an MPEG-1 packet header, a pack header sniffed behind at most 64 zero bytes, and E-AC-3 frames of
at least 8 bytes.

### Specifications

| Document | Edition | Used for |
|---|---|---|
| ITU-T H.222.0 \| ISO/IEC 13818-1 | 2000 (second edition) | TS packets, adaptation fields, PCR, PSI (PAT / PMT, CRC-32 of Annex A), PES headers, PTS/DTS, program streams (pack, system header, program stream map), stream types |
| ISO/IEC 11172-1 | 1993 | MPEG-1 system streams (pack header, PES header stuffing / STD buffer / timestamps) |
| Blu-ray / AVCHD BDAV MPEG-2 TS layout (192-byte source packets with a 4-byte TP_extra_header; registration `HDMV`; stream types 0x80-0x92) | public descriptions | `.m2ts` / `.mts` |
| DVD-Video private stream 1 sub-stream layout (AC-3 / DTS 0x80-0x8F with a 3-byte header, LPCM 0xA0-0xAF with a 6-byte header, sub-pictures 0x20-0x3F) | public descriptions | `.vob` |
| ETSI EN 300 468 (descriptors 0x56, 0x59, 0x6A, 0x7A, 0x7B), ATSC A/52 Annex A (AC-3 in TS) | — | identifying private PES streams |
| ISO/IEC 11172-3, 13818-3 (frame headers), ISO/IEC 13818-7 (ADTS), 14496-3 (LOAS), ATSC A/52 (syncinfo, E-AC-3 frmsiz) | — | audio frame lengths for splitting |

### Gaps

- One program per TS (the first in the PAT); no program switching, no section-based
  metadata (SDT / EIT), no scrambled streams.
- H.264 / HEVC access units follow PES packets: streams that put several pictures in one PES
  packet (rare in practice; not seen in broadcast, AVCHD or FFmpeg output) would be indexed as
  one unit.
- DVD navigation (IFO / NAV packets), multiple VOB concatenation and angle interleaving are not
  handled; each file opens on its own.

Differences from `filmcraft-mpegts`: `ByteSource` is `craft_media::ByteSource`;
`Error` is written by hand, without `thiserror`, and is `#[non_exhaustive]`;
`Codec::codec_id()` and `Stream::codec_params()` are new.

Tests: a small TS muxer and PS writer (every access unit reads back byte-identical with its PTS,
key and disposable flags), 33-bit unwrap, truncation and mutation, streams of every codec filled
with random start codes and sync words, damaged PSI sections with valid CRCs, and the frame-header
helpers on arbitrary bytes. `tests/oracle_mpegts.rs` compares ten ffmpeg-made files (MPEG-2, H.264
and HEVC video; MP2, ADTS and LATM AAC, AC-3, E-AC-3, Blu-ray and DVD LPCM; TS, M2TS, VOB and
MPEG-1 system streams) with ffprobe's packets: unit counts, sizes, timestamps and key flags.

## `mxf`

A clean-room MXF demuxer and writer from FilmCraft's `filmcraft-mxf` (storytold/filmcraft@4bec22b):
OP1a and OP-Atom, index tables, AVC / VC-3 / ProRes / MPEG-2 pictures and PCM sound. No GPL/LGPL
code (FFmpeg, libMXF, bmx, MXFLib) was consulted; FFmpeg is a fixture generator and test oracle
only. One dependency: `chrono`, for the calendar dates the writer stores.

### Specifications

Implemented from the SMPTE documents (editions used):

| Document | Edition | Used for |
|---|---|---|
| SMPTE ST 336 | 2017 | KLV coding: 16-byte UL keys, BER lengths |
| SMPTE ST 377-1 | 2011 (+ Amd 1:2012) | partitions, primer pack, header metadata sets and local tags, index table segments, random index pack, run-in |
| SMPTE ST 378 | 2004 | OP1a |
| SMPTE ST 390 | 2011 | OP-Atom |
| SMPTE ST 379-1 / 379-2 | 2009 / 2010 | generic container: content packages, element keys, frame / clip wrapping |
| SMPTE ST 381-1 | 2005 | MPEG video mapping (MPEG-2 identified, MPEG video descriptor) |
| SMPTE ST 381-3 | 2013 | AVC byte-stream mapping and AVC sub-descriptor |
| SMPTE ST 382 | 2007 | AES3 and Broadcast Wave audio mapping (wave / AES3 descriptors) |
| SMPTE ST 331 | 2011 | element data of 8-channel AES3 sound (D-10) |
| SMPTE ST 386 | 2004 | D-10 (IMX) mapping: container label, sound element type |
| SMPTE ST 2019-4 | 2009 | VC-3 (DNxHD / DNxHR) mapping |
| SMPTE RDD 44 | 2017 | Apple ProRes mapping |
| SMPTE RP 224 / RP 210 | registers | labels: data definitions, picture coding, essence containers, operational patterns; property labels of the primer pack (writer) |
| SMPTE ST 330 | 2011 | basic UMIDs of the written packages |

### Reading a file

```rust
use craft_media::mxf::{TrackKind, open};

let mxf = open(&file)?; // any ByteSource
println!("{} {:?}", mxf.operational_pattern.name(), mxf.timecode.map(|t| t.format()));
let v = mxf.track_of_kind(TrackKind::Picture).unwrap();
let t = &mxf.tracks[v];
let key = t.sync_before(t.sample_at(42).unwrap()); // stored index of the random-access picture
let bytes = mxf.read_sample(&file, v, key)?; // one edit unit's essence
let a = mxf.track_of_kind(TrackKind::Sound).unwrap();
let pcm = mxf.read_pcm(&file, a, 48_000, 1920)?; // planar f32, sample-exact
```

- **Open.** The header partition pack is found in the run-in (≤ 64 KiB). Every KLV packet is then
  walked (keys and lengths only; essence values are skipped, never read): partition packs, primer
  packs, header metadata local sets, index table segments, fill, and generic-container elements
  (system items are ignored). A lost KLV sync resynchronises on the next partition pack; a
  truncated file keeps everything before the cut (a truncated frame-wrapped picture is dropped,
  clip-wrapped essence keeps its whole edit units).
- **Metadata.** The header metadata of the most complete partition is used (closed complete >
  open complete > closed > open; later partitions win ties, so the footer's final durations are
  preferred). Dynamic local tags resolve through the primer pack (e.g. `SubDescriptors`).
- **Tracks.** Material package picture/sound tracks → source clip → file package track (track
  number, edit rate, origin) → descriptor (a multiple descriptor's sub-descriptor by
  `LinkedTrackID`). Without a usable material package the file packages' tracks are used directly.
  Elements are matched by track number, or — when the number is 0 or does not match (some OP-Atom
  writers) — the only unclaimed element stream of the right kind.
- **Pictures.** One `Sample` per edit unit in stored order: frame wrapping (one element each) or
  clip wrapping (one element split by the index: constant `EditUnitByteCount` or VBR entries).
  Random access from index flag bit 7 (intra-only codings: every picture); without an index the
  essence is scanned (AVC IDR, MPEG-2 sequence header). Presentation order from the index temporal
  offsets (display position *n* is stored at *n* + `TemporalOffset[n]`). When the index marks B
  pictures but has no temporal offsets (FFmpeg-written AVC), `needs_reorder` is set: the caller
  orders by the bitstream (the AVC picture order counts).
- **Codecs.** From the picture essence coding label, then the essence container label, then the
  essence bytes: AVC / AVC-Intra, VC-3, ProRes (profile from the label), MPEG-2,
  MPEG-4 visual, JPEG 2000, DV, uncompressed.
- **Sound.** PCM chunks (frame- or clip-wrapped) with sample counts; `read_pcm` decodes
  little-endian 8/16/24/32-bit PCM (wave / AES3 descriptors) and ST 331 AES3 elements (D-10).
- **Timecode.** The material package timecode component (start frame count, rounded base,
  drop frame), falling back to the file package's; `Timecode::format` gives `HH:MM:SS:FF` /
  `HH:MM:SS;FF`.

Not supported: OP1b/OP2x+ edit lists beyond the first source clip, external essence (OP-Atom
material packages whose other tracks live in other files are opened track by track), partial or
encrypted essence (ST 429-6), and descriptive metadata (parsed sets are ignored).

`Codec::codec_id()` and `EssenceTrack::codec_params()` give the crate-wide `CodecId` /
`CodecParams`: AVC is `H264`, VC-3 is `Dnxhd`, PCM and AES3 element data are `Pcm`; JPEG 2000, DV
and uncompressed pictures have no identifier. The size is the displayed frame, sound gets its
sample rate, channels and (for plain PCM) the little-endian `PcmFormat`. MXF essence carries its
configuration in band, so `extradata` is empty.

Limits, each with a test: a run-in of at most 65 535 bytes before the header partition pack, a
metadata or index value of at most 64 MiB (larger is refused before it is read) and 256 MiB of
them in all, 16 MiB scanned
for the next partition pack after a lost KLV sync, an index position that cannot make a sparse
table, at most 256 sound channels (8 in an AES3 element), a PCM block alignment of at most 4096
bytes, and a sound request of at most 2²⁴ samples past the stored essence.

### Writing a file

```rust
use craft_media::mxf::*;

let ids = PackageIds::from_seed("unique seed (path + time)", "Clip name"); // material + file UMIDs
let mut cfg = WriterConfig::new(Pattern::Op1a, Rational::new(25, 1), ids);
cfg.picture = Some(PictureDesc::new(PictureCoding::Vc3 { cid: 1272 }, 1920, 1080));
cfg.sound = vec![SoundDesc { sample_rate: 48_000, channels: 2, bits: 24 }];
cfg.timecode = Some(StartTimecode { frames: 90_000, rate: Rational::new(25, 1), drop_frame: false });
cfg.company = "My company".into(); // `craft-media` unless set, like `product`
let mut w = MxfWriter::new(std::io::BufWriter::new(std::fs::File::create("out.mxf")?), cfg)?;
w.push_picture(&frame_bytes, FrameInfo::intra())?; // stored order; AVC: key / P / B + display position
w.push_sound(0, &interleaved_le_pcm)?; // any chunking: content packages are cut per edit unit
w.finish()?;
```

- **Streaming.** Any `Write + Seek` sink. `new` writes the header partition (open, incomplete,
  durations unknown) and a body partition; essence follows as it is pushed; `finish` writes the
  footer partition (closed, complete header metadata with the final durations and the index table
  segments), the random index pack, then rewrites the header partition pack and header metadata in
  place (the metadata's size does not depend on the durations; a reserved fill item absorbs any
  difference) and the body partition pack's footer offset. Only the index entries (11 bytes per
  edit unit, plus 4 per sound slice) are kept in memory.
- **OP1a** (ST 378): one content package per edit unit, frame wrapping (ST 379-1): the picture
  element then one element per sound track. Sound is buffered and cut into edit units exactly
  (`floor((n+1)·rate·den/num) − floor(n·rate·den/num)` samples: 1601/1602 at 29.97). Pictures
  without their sound yet wait; `finish` pads the last units' sound with silence. A multiple
  descriptor with one sub-descriptor per track (linked track ids), the "multiple wrappings"
  container label.
- **OP-Atom** (ST 390, Avid style): exactly one essence track per file, clip-wrapped (one element
  with a 9-byte BER length patched by `finish`). Several atom files of one clip can share the
  material package UMID with distinct track ids (`WriterConfig::first_track_id`). Sound files may
  use the sample rate as the edit rate (a constant-bytes index) or the frame rate (VBR entries).
- **Index tables** (ST 377-1 §11): VBR entries per edit unit — stream offset (from the first
  essence byte; clip-wrapped: from the element value), key-frame offset, flags (`0x80` random
  access, `0xC0` AVC IDR with its parameter sets, `0x22` P, `0x33` B) and temporal offsets from the
  display positions (`TO[d] = stored(d) − d`); a delta entry and a slice per sound element. Split
  into segments of at most ~60 KB (the entry array is a 2-byte-length local set item).
- **Metadata.** Preface, identification (company / product / version / modification date),
  content storage, essence container data (BodySID 1, IndexSID 2), material and file packages
  (`PackageIds`: UMIDs and names) with a timecode track (start, rounded base, drop frame) and one
  timeline track per essence (source clip → file package track; track numbers = element keys),
  CDCI picture descriptors (size, aspect ratio, depth, subsampling, black / white / range, BT.709 /
  BT.2020 PQ / HLG colour labels, coding label) and wave audio descriptors.
- **Mappings.** VC-3 (ST 2019-4: coding label `…71 cc` from the compression id, container `02 11`,
  element types 0x0C / 0x0D), ProRes (RDD 44: coding label profile byte, container `02 1C`),
  AVC byte stream (ST 381-3: container `02 10 60`, coding label from `profile_idc`; Annex B access
  units with in-band SPS / PPS — the AVC sub-descriptor is not written), Broadcast Wave PCM
  (ST 382: container `02 06 01/02`, element types 0x01 / 0x02).
- `write_opatom_pcm(&OpAtomPcm, &[i32])` builds a complete in-memory OP-Atom PCM file (AAF media
  consolidation); `encode_pcm` interleaves planar f32 as little-endian PCM.

### Differences from `filmcraft-mxf`

- `ByteSource` is `craft_media::ByteSource`.
- `WriterConfig::new` sets `company` and `product` to `craft-media` (FilmCraft: `FilmCraft`) and
  `version` to this crate's. Apps set their own; `write_opatom_pcm_as` takes them for the
  in-memory OP-Atom PCM writer (`write_opatom_pcm` writes the defaults). The identification
  set's product UID is derived from `"<product> MXF writer"`, so `company` and `product` of
  `FilmCraft` with FilmCraft's version string write the bytes `filmcraft-mxf` wrote.
- Everything read while opening (partition packs, primers, metadata sets, index segments) shares
  a budget of 256 MiB; past it the file is refused. `filmcraft-mxf` bounded each value only.
- `EssenceTrack::sync_before` returns 0 for a track without picture samples (a sound track); it
  indexed an empty table and panicked.
- `Timecode::format` saturates for a drop-frame start near `i64::MAX`; it overflowed (a panic in
  a debug build).
- `Error` is `#[non_exhaustive]`; `Codec::codec_id()` and `EssenceTrack::codec_params()` are new.

### Tests

Synthetic OP1a files (temporal offsets, key flags, PCM, timecode), writer output read back (OP1a
with two sound tracks, 29.97 sound per edit unit, long-GOP AVC index, OP-Atom picture and PCM,
multi-segment index), truncation and mutation, and `hostile_tests.rs`: every metadata item, index
item and partition pack field set to extreme values, KLV lengths that lie, and the parsers on
arbitrary bytes. `tests/oracle_mxf.rs`: eight ffmpeg-written files (OP1a H.264 with and without B
pictures, DNxHR, ProRes, MPEG-2; OP-Atom DNxHR and PCM; D-10) compared with ffprobe's packets and
ffmpeg's decoded sound sample for sample; and files written by `MxfWriter` from the same essence,
which ffprobe must describe like their source and ffmpeg must decode to the same pictures and
sound.

## Containers, codecs and codec parameters

Always built, with no dependencies, so codec crates share them:

- `Container`: the file format (`Wav`, `Aiff`, `Flac`, `Mp4`, `QuickTime`, `Matroska`, `WebM`,
  `Ogg`, `Caf`, `Adts`, `MpegAudio`, `Ac3`, `Avi`, `Mxf`, `MpegTs`, `MpegPs`), with `name()`.
- `CodecId`: audio and video codecs (`Aac`, `HeAac`, `Opus`, `Eac3`, `Speex`, …, `H264`, `Hevc`,
  `Av1`, `ProRes`, `Dnxhd`, `Apv`, `Theora`, …, `Unknown`), with `name()`, `is_audio()`,
  `is_video()`.
- `PcmFormat`: the layout of uncompressed samples, built with `PcmFormat::int(bits, signed,
  big_endian)` or `PcmFormat::float(bits, big_endian)`.
- `CodecParams`: what a decoder needs about a track, whatever its container: `codec`,
  `extradata`, `tag` (the container's four-character code, such as a ProRes profile), the audio
  fields `sample_rate`, `channels` and `pcm` (`PcmFormat`), and the video fields `width` and
  `height`. Built with `CodecParams::new(codec)` and the `with_*` methods, or by the demuxers'
  `codec_params()`, which every container returns in the same form. `extradata` follows
  Matroska's `CodecPrivate`, as ffmpeg's extradata does: the `AudioSpecificConfig` for AAC, the
  `OpusHead` for Opus (MP4's `dOps` is converted), Xiph-laced headers for Vorbis (`xiph_lace`,
  `None` for more than 256 packets), `fLaC` + metadata blocks for FLAC, the
  `avcC`/`hvcC`/`av1C`/`apvC` record for video.

All are `#[non_exhaustive]`: new formats and fields are minor changes.

## `ByteSource`

`craft_media::ByteSource` (`len`, `read_at(offset, buf)`) is what every demuxer reads from. It is
always built, so a crate that uses only `bitstream` can name it too. `isobmff::ByteSource`,
`matroska::ByteSource`, `ogg::ByteSource`, `avi::ByteSource`, `mxf::ByteSource` and `mpegts::ByteSource` are the same trait. It is implemented for `[u8]`,
`Vec<u8>`, `&T`, `Arc<T>`, `Box<T>` and `std::fs::File` (not on wasm). `ReadSeekSource` adapts any
`Read + Seek`; it is `Send + Sync` when the reader is `Send`. Implement the trait once for a
custom source (a web `Blob`, a network range reader) to use every container with it.

## Tests

- Unit tests cover the bit reader and writer, the box reader (64-bit, size-0, `uuid`, truncated and
  hostile sizes), hand-built `stbl` edge cases, codec configuration round trips and allocation
  guards. For Matroska: EBML integers, typed values, CRC-32, every lacing mode, codec mapping, a
  hand-built file (unknown-size Segment and Clusters, header stripping, BlockGroups, block
  additions, color and mastering metadata, Chapters, Tags, Attachments, CRC-32, Cue-less seek,
  hostile cluster timestamps) and mux-demux round trips. For Ogg (`src/ogg`): page parsing, CRC,
  packets across pages, damage resync and its read budget, lost pages, chained links and stray
  beginning-of-stream flags, the packet and header caps, Opus timing (end trimming, hostile granules) and the codec parameters of each codec.
- `tests/roundtrip.rs`: property tests that write random multi-track files (progressive and
  fragmented) and read them back.
- `tests/oracle_demux.rs` and `tests/oracle_mux.rs` compare the demuxer and muxer with ffprobe and
  ffmpeg on generated fixtures in `target/fixtures/isobmff/` (`$CRAFT_MEDIA_FIXTURES_DIR`
  overrides it). Fixtures are never committed. `chapters_are_listed_by_ffprobe` checks that
  `ffprobe -show_chapters` lists the chapters the muxer writes (count, starts, ends, UTF-8 titles) in
  MP4 and MOV.
- `tests/oracle_matroska.rs` compares every packet (stream, size, keyframe flag, pts, duration),
  codec names, time bases, dimensions and audio parameters with ffprobe on fixtures in
  `target/fixtures/matroska/`: H.264+AAC, VP9+Opus WebM, FLAC, Vorbis, SRT+ASS, WebVTT WebM, Cues
  at the front, a streamed file without Cues, HEVC with HDR color, ProRes+PCM, MJPEG+AC-3, AV1.
  Seeks must land on the latest keyframe at or before the target. The muxer's laced output is read
  back by ffprobe. A damaged file must resync.
- `tests/fuzz.rs` and `tests/fuzz_matroska.rs`: deterministic mutation fuzzing; nothing may panic.
  `fuzz.rs` also walks every box of a mutated file as a container and checks that each payload
  range stays inside its parent.

ffmpeg and ffprobe are test oracles only, never linked. The helpers are in `craft-media-testkit`
(an unpublished crate, shared with craft-audio and the video crate to come). They are looked up in
`$CRAFT_MEDIA_FFMPEG` / `$CRAFT_MEDIA_FFPROBE`, then `PATH`, then common install directories. When
a tool is missing, the oracle tests print `SKIPPED` and pass; `CRAFT_MEDIA_REQUIRE_ORACLES=1` turns
a skip into a failure. CI (`.github/workflows/media.yml`) sets it and installs Ubuntu's ffmpeg.
craft-audio's oracle tests use the same variables. Generate every fixture ahead of time with
`cargo test -p craft-media --all-features -- --ignored generate_fixtures`.
