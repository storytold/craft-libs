# craft-media

Media containers and bit-level I/O for the Craft apps: an MSB-first bit reader and writer, and
clean-room ISO BMFF (MP4), QuickTime (MOV) and Matroska (MKV, WebM) demuxers and muxers written
from the public specifications. No `unsafe`, no panics on malformed input, and it builds for
`wasm32-unknown-unknown`. The only dependency is `thiserror` (for `isobmff`).

## Features

| Feature | Module | Enables |
|---|---|---|
| `bitstream` | `craft_media::bitstream` | none |
| `isobmff` | `craft_media::isobmff` | `bitstream`, `thiserror` |
| `matroska` | `craft_media::matroska` | none |

`default = ["isobmff"]`, so `bitstream` and `isobmff` build. `matroska` is opt-in: it is about as
much code as `isobmff`, and most users of MP4/MOV (LightCraft, FigCraft, SoundCraft, EffectCraft)
do not read Matroska. The features are independent; `matroska` does not need the other two.

```toml
craft-media = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["matroska"] }
```

A crate that needs only bit I/O (a codec) uses:

```toml
craft-media = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", default-features = false, features = ["bitstream"] }
```

## Boundaries

- In scope: bit-level reading and writing, and container demuxing and muxing (boxes, sample
  tables, sample entries and codec configuration records).
- Out of scope for now: codecs (H.264, HEVC, ProRes, AAC and others). They stay in the apps.
  Ogg and other containers can join later as their own features.

## Origin and API

The code comes from FilmCraft's `filmcraft-bitstream`, `filmcraft-isobmff` and
`filmcraft-matroska` crates at
[storytold/filmcraft@6c3948a](https://github.com/storytold/filmcraft/commit/6c3948a36c80e4996d924766ba7157eec4fca85e).
Every public item keeps its name, signature and behavior for valid input, so FilmCraft can turn
all three crates into re-exports (`pub use craft_media::isobmff::*;`). The differences:

- `bitstream::BitReader::read_ue_plus` (from DeckCraft's copy) is new.
- `isobmff::boxes` is new: FilmCraft's private box iterator and byte reader, made public (see
  [Box reader](#box-reader)).
- `BitReader::peek`, `BitReader::read_bits` and `BitWriter::write_bits` accept widths above 32
  without a debug assertion: `peek` and `read_bits` return the low 32 bits of the field (use
  `read_bits_u64` for wide fields), and `write_bits` writes leading zero bits, at most 64 bits in
  all.
- `bitstream::length_prefixed_nals` returns `BitError::Eof` for a zero `length_size` on non-empty
  data and cannot overflow on a hostile length prefix.
- `isobmff::PcmConfig::bytes_per_frame` saturates at `u32::MAX` for a hostile channel count.
- `matroska`: a cluster timestamp above `i64::MAX` saturates at `i64::MAX` (FilmCraft wraps it to
  a negative value), and adding a block's relative timestamp saturates instead of overflowing (a
  debug-build panic in FilmCraft).
- `matroska::MkvWriter` writes `MuxingApp` = `craft-media`, and `MuxOptions::default()` has
  `writing_app` = `craft-media` (FilmCraft: `filmcraft-matroska` and `filmcraft`). Apps set
  `writing_app` to their own name.
- `matroska::ByteSource` stays a separate trait from `isobmff::ByteSource`, with the same shape.
  FilmCraft implements both for one type, which a single shared trait would reject as conflicting
  impls.

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
  `fragment_count`.
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
  table entry per audio frame.

#### Parsed boxes
`ftyp`, `moov/mvhd`, `trak/tkhd`, `edts/elst` (v0/v1), `tref`, `mdia/mdhd` (v0/v1, ISO and Mac language
codes), `hdlr` (ISO and Pascal names), `minf/stbl`: `stsd`, `stts`, `ctts` (v0/v1, read as signed), `stss`,
`stsc`, `stsz`/`stz2` (4/8/16-bit), `stco`/`co64`, `sdtp`, `sbgp`/`sgpd`. Also `mvex/trex`, `moof/traf/tfhd/tfdt/trun`
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
| `.mp3`/`.mp2` | `Mp3`: MPEG-1/2 audio of any layer (each frame header names it) |
| PCM fourccs | `Pcm(PcmConfig)`: bits, float, endianness (`enda`, lpcm flags, `pcmC`), channels, rate |
| `alac` | `Alac { cookie }` |
| `Opus` + `dOps` | `Opus(OpusConfig)` |
| `fLaC` + `dfLa` | `Flac(FlacConfig)` (STREAMINFO decoded) |
| `ac-3`/`ec-3` | `Ac3 { dac3 }` / `Eac3 { dec3 }` |
| `tmcd` | `Timecode(TimecodeConfig)`: flags (drop frame), timescale, frame duration, fps, start frame, reel name; `format_frame()` |
| anything else | `Unknown { fourcc, raw }` |

Video entries also parse `colr` (`nclx`/`nclc`/ICC), `pasp`, `clap`, `fiel`, `gama` and `btrt` into
`VideoParams`. Audio entries parse QuickTime sound description v0/v1/v2 into `AudioParams`.

#### Robustness
The parser does not panic on malformed input; every error comes back as an `Error`. Table counts are
checked against the bytes in their box before anything is allocated. A constant-size `stsz` count is
clamped to what fits in the file. `trun` counts are checked, and each track is capped at 2²⁶ samples.
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
  sync), `stsc`, `stsz` (constant or table), `stco`/`co64`. Consecutive samples of one track share a chunk.
- Sample entries come from `SampleEntry` (`avc`, `hevc`, `prores`, `jpeg`, `aac`, `pcm`, or any demuxed
  entry). Remuxing copies codec config, `colr`, `pasp`, `clap`, `fiel`, `gama` and `btrt`.
  PCM is written as `sowt`/`twos` (16-bit) or `raw ` (8-bit) or sound description v2 `lpcm` in MOV, and as
  `ipcm`/`fpcm` + `pcmC` in MP4. A PCM `write_sample` call can hold any whole number of frames, and the
  track timescale must equal the sample rate.
- Edit lists: set `TrackConfig::edits` explicitly, or use `media_start` to get a single edit.
- Timecode: `add_timecode_track(video, TimecodeConfig, start_frame)` adds a `tmcd` track (with `gmhd`)
  and a `tref/tmcd` link from the video track.
- `WriterOptions::metadata` writes `©xxx` text atoms to `udta`.
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
- Muxer: one `mdat`, no chunk-size or duration limits (the caller controls interleaving), no `sdtp`,
  `sgpd`/`sbgp` or `cslg` output, ISO-style `meta/ilst` metadata is not written, and the fragmented
  writer has no PCM, `sidx` or `mfra`.

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

- **Open** (`open`, `open_with(src, &OpenOptions { index, verify_crc })`): EBML header (DocType
  `matroska` or `webm`), the first Segment, and its level-1 elements: SeekHead (followed for
  elements after the clusters), Info, Tracks, Cues, Chapters, Tags, Attachments (listed with offset
  and size; payloads are not read). With `index: true` (the default) every Cluster is scanned,
  element and block headers only, to build per-track `Sample` tables (`offset`, `size`, `pts`,
  `duration`, `keyframe`, cluster, block offset, lace index, `BlockAdditional` location) and
  keyframe lists. With `index: false`, `open` stops at the first Cluster.
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
`default_duration_ns`, optional audio lacing (Xiph, EBML, fixed-size), and Cues (video keyframes,
or the first frame of each track per cluster). `MuxOptions::doc_type` = `"webm"` writes WebM.
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

## Tests

- Unit tests cover the bit reader and writer, the box reader (64-bit, size-0, `uuid`, truncated and
  hostile sizes), hand-built `stbl` edge cases, codec configuration round trips and allocation
  guards. For Matroska: EBML integers, typed values, CRC-32, every lacing mode, codec mapping, a
  hand-built file (unknown-size Segment and Clusters, header stripping, BlockGroups, block
  additions, color and mastering metadata, Chapters, Tags, Attachments, CRC-32, Cue-less seek,
  hostile cluster timestamps) and mux-demux round trips.
- `tests/roundtrip.rs`: property tests that write random multi-track files (progressive and
  fragmented) and read them back.
- `tests/oracle_demux.rs` and `tests/oracle_mux.rs` compare the demuxer and muxer with ffprobe and
  ffmpeg on generated fixtures in `target/fixtures/isobmff/` (`$CRAFT_MEDIA_FIXTURES_DIR`
  overrides it). Fixtures are never committed.
- `tests/oracle_matroska.rs` compares every packet (stream, size, keyframe flag, pts, duration),
  codec names, time bases, dimensions and audio parameters with ffprobe on fixtures in
  `target/fixtures/matroska/`: H.264+AAC, VP9+Opus WebM, FLAC, Vorbis, SRT+ASS, WebVTT WebM, Cues
  at the front, a streamed file without Cues, HEVC with HDR color, ProRes+PCM, MJPEG+AC-3, AV1.
  Seeks must land on the latest keyframe at or before the target. The muxer's laced output is read
  back by ffprobe. A damaged file must resync.
- `tests/fuzz.rs` and `tests/fuzz_matroska.rs`: deterministic mutation fuzzing; nothing may panic.
  `fuzz.rs` also walks every box of a mutated file as a container and checks that each payload
  range stays inside its parent.

ffmpeg and ffprobe are test oracles only, never linked. They are looked up in
`$CRAFT_MEDIA_FFMPEG` / `$CRAFT_MEDIA_FFPROBE`, then `PATH`, then common install directories. When
a tool is missing, the oracle tests print `SKIPPED` and pass; `CRAFT_MEDIA_REQUIRE_ORACLES=1` turns
a skip into a failure. CI (`.github/workflows/media.yml`) sets it and installs Ubuntu's ffmpeg.
Generate every fixture ahead of time with
`cargo test -p craft-media --all-features -- --ignored generate_fixtures`.
