# craft-video

Clean-room pure-Rust video codecs for the Craft apps, one feature per codec: DNxHD/DNxHR, VP8,
ProRes and H.264.

- Input and output are byte slices and planar sample buffers; the crate opens no files itself
  and performs no filesystem I/O.
- Implemented from published standards and RFCs (SMPTE ST 2019-1, RFC 6386, SMPTE RDD 36 and
  ITU-T H.264).
- Tested against ffmpeg as an external oracle with roundtrips, hostile fuzzing, and conformance vectors.
  No `unsafe`, no panics on corrupt streams. Builds for `wasm32-unknown-unknown`.
- Containers (MP4/MOV, Matroska/WebM), bit-level I/O (`craft_media::bitstream`) and shared
  identifiers (`CodecId`, `CodecParams`) come from [`craft-media`](../media/README.md).

## Cargo features

No codec feature is on by default.

| Feature | Module | Codec |
|---|---|---|
| `dnx` | [`dnx`](src/dnx/README.md) | Avid DNxHD / DNxHR (SMPTE ST 2019-1 VC-3): decoder and encoder |
| `apv` | [`apv`](src/apv/README.md) | APV (Advanced Professional Video, RFC 9924): decoder (`apv-decode` alone) and encoder (`apv-encode` alone) |
| `vp8` | `vp8` | VP8 (RFC 6386): decoder |
| `mpeg2` | [`mpeg2`](src/mpeg2/README.md) | MPEG-2 video (ITU-T H.262) and MPEG-1 video (ISO/IEC 11172-2): decoder (`mpeg2-decode` alone); MPEG-2 encoder with I, P and B pictures (`mpeg2-encode`, which also turns on the decoder) |
| `mpeg4` | [`mpeg4`](src/mpeg4/README.md) | MPEG-4 Part 2 video (ISO/IEC 14496-2, the Xvid/DivX format): Simple profile encoder (`mpeg4-encode`) |
| `prores` | `prores` | Apple ProRes (SMPTE RDD 36): decoder and encoder |
| `h264` | [`h264`](src/h264/decode/README.md) | ITU-T H.264 / AVC: decoder (`h264-decode` alone) and encoder (`h264-encode` alone) |
| `hevc` | [`hevc`](src/hevc/decode/README.md) | ITU-T H.265 / HEVC: decoder (`hevc-decode` alone) and encoder (`hevc-encode` alone) |
| `vp9` | [`vp9`](src/vp9/decode/README.md) | VP9: decoder (`vp9-decode` alone) and encoder (`vp9-encode` alone) |
| `av1` | [`av1`](src/av1/decode/README.md) | AV1 (AOMedia Video 1): decoder (`av1-decode` alone) and clean-room encoder (`av1-encode`); tiled rav1e fallback for large frames (`av1-large-encode`) |
| `smacker` | `smacker` | Smacker video (8-bit paletted, `SMK2` and `SMK4`): decoder |
| `threads` | — | Multi-threaded slice/row parallelism via `rayon` |

## Smacker

`smacker::Decoder` decodes Smacker video, the 8-bit paletted format of 1990s game cutscenes.
Smacker has no published specification: the decoder is written from the public community
description of the format (the MultimediaWiki "Smacker" page), with ffmpeg as an external
oracle. No FFmpeg or other Smacker implementation was read.

```rust
use craft_media::smacker::{open, split_frame};
use craft_video::smacker::Decoder;

let bytes = std::fs::read("intro.smk")?;
let file = open(&bytes)?;
let mut dec = Decoder::new(&file.video_params())?;
for frame in &file.frames {
    let data = file.read(&bytes, frame)?;
    let parts = split_frame(&data, frame)?;
    dec.decode(parts.palette, parts.video)?;
    let (indexes, palette) = (dec.pixels(), dec.palette()); // or dec.to_rgba()
}
```

- Output: one palette index a pixel and a 256-entry RGB palette, or RGBA through `to_rgba`.
  The decoder keeps the picture and the palette between frames, so frames are decoded in order
  from the first.
- Block types: mono, full, void and solid, with runs; the `SMK4` double and half full-block
  modes; palette chunks (set, keep and copy records). A chunk that ends before the last entry
  keeps the other entries, as the format description gives it; ffmpeg reads on into the
  frame's other data instead. No real file had such a chunk.
- A file's ring frame (`SmackerFile::ring_frame`) decodes like any other frame, after the last
  one. In 216 of the 243 real files with a ring frame, the picture after it equals the first
  picture; in the other 27 up to 7% of the pixels differ.
- The header's row doubling and interlacing flags (`Header::Y_DOUBLED`, `Header::Y_INTERLACED`
  in `craft_media::smacker`) describe how to show the picture and are left to the caller.
- Bounds: a picture has at most `MAX_PIXELS` pixels (4096x4096), a tree at most its declared
  size and the 131071 nodes a tree over 16-bit values can have, and a frame that ends early is an error that keeps the
  blocks decoded before it.
- Verified: `tests/smacker` compares pictures with ffmpeg on a directory of real files
  (`CRAFT_SMACKER_SAMPLES`, the first 150 frames of each file, or all with `CRAFT_SMACKER_FRAMES=0`; skipped when it is not set, since real files come from commercial
  games and are not in the repository). 1,796 files from four games match on every frame: 357 `SMK2` and 1,439 `SMK4`, whose two
  extra full-block modes many of them use. ffmpeg refuses six more files (4x4 pictures that
  only carry sound); the decoder reads them without error, with nothing to compare against.
