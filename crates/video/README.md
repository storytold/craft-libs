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
| `mpeg2` | [`mpeg2`](src/mpeg2/README.md) | MPEG-2 video (ITU-T H.262) and MPEG-1 video (ISO/IEC 11172-2): decoder (`mpeg2-decode` alone); MPEG-2 encoder with I, P and B pictures and constant or variable bitrate within the VBV buffer (`mpeg2-encode`, which also turns on the decoder) |
| `mpeg4` | [`mpeg4`](src/mpeg4/README.md) | MPEG-4 Part 2 video (ISO/IEC 14496-2, the Xvid/DivX format): Simple profile encoder (`mpeg4-encode`) |
| `hap` | `hap` | Vidvox Hap: Hap, Hap Alpha, Hap Q, Hap Q Alpha and Hap Alpha-Only, decoder and encoder, with the Snappy and chunked second stages. DXT1, DXT5, scaled YCoCg-DXT5 and BC4 pictures decode in software; BC7 (Hap R) and BC6 (Hap HDR) frames are read and their blocks returned but not decoded to pictures. |
| `mjpeg` | `mjpeg` | Motion JPEG: a decoder (baseline, extended and progressive Huffman JPEG; the Annex K tables for AVI `MJPG` packets that have none; `Decoder::decode_into` writes RGBA or BGRA pixels into a caller's buffer at any row stride) and a baseline encoder, both over `craft-jpeg`. No two-field (MJPEG-A/B) packets and no lossless JPEG. |
| `prores` | `prores` | Apple ProRes (SMPTE RDD 36): decoder and encoder |
| `h264` | [`h264`](src/h264/decode/README.md) | ITU-T H.264 / AVC: decoder (`h264-decode` alone) and encoder (`h264-encode` alone) |
| `hevc` | [`hevc`](src/hevc/decode/README.md) | ITU-T H.265 / HEVC: decoder (`hevc-decode` alone) and encoder (`hevc-encode` alone) |
| `vp9` | [`vp9`](src/vp9/decode/README.md) | VP9: decoder (`vp9-decode` alone) and encoder (`vp9-encode` alone) |
| `av1` | [`av1`](src/av1/decode/README.md) | AV1 (AOMedia Video 1): decoder (`av1-decode` alone) and clean-room encoder (`av1-encode`); tiled rav1e fallback for large frames (`av1-large-encode`) |
| `vmd` | `vmd` | Sierra VMD video (the 8-bit paletted pictures of Sierra and Coktel Vision game videos): decoder (`vmd-decode`) |
| `threads` | — | Multi-threaded slice/row parallelism via `rayon` |

## Decoded frames

Each decoder returns its own picture type. `frame::VideoFrame` is a codec-agnostic form that
every decoder's output converts into with `VideoFrame::try_from` (always built; each conversion
comes with its codec's feature):

- Planes are `[Y, Cb, Cr]`, `[R, G, B]` or `[Y]` (`ColorFamily`), plus an optional alpha plane.
  DNx, VP9 and AV1 (identity matrix) store RGB as G, B, R; the conversion reorders them.
- Samples keep the decoder's storage, `u8` or `u16`, with a bit depth per plane (HEVC chroma may
  differ from luma). Converting moves the buffers and copies no samples, except for Motion JPEG,
  whose decoder returns interleaved samples (they are split into planes), and paletted video: VMD
  holds its picture as palette indices in the decoder, so `VideoFrame::try_from(&decoder)` looks
  them up and writes new 8-bit R, G, B planes.
- Chroma subsampling is a log2 shift per axis, 0 to 3 (FFV1 codes up to 3).
- Colour is given as ITU-T H.273 code points (`ColorTags`), mapped from each codec's own
  signalling. Codecs that do not signal a value leave it unspecified (2).
- `pts`, sample aspect ratio, field order and the decoder's draft flag are carried over where the
  decoder reports them. ProRes signals the picture's shape (4:3 or 16:9); the sample aspect
  ratio is worked out from it and the frame size.

The constructors check that every plane's buffer, stride and size fit the frame, so a picture
with inconsistent fields is an error, not an out-of-bounds read.

## Decoding a track

`TrackDecoder` is the counterpart of `craft_audio::TrackDecoder` for an app with its own demuxer.
It picks the decoder from a track's `CodecParams`, takes packets in decode order with their
timestamps, and returns `VideoFrame`s in display order. `flush` returns the frames still held at
the end of the stream, and `reset` prepares for decoding from another key frame after a seek. It
is built with any of the decoder features below and decodes the codecs whose feature is on.

| Codec | Feature | `extradata` |
|---|---|---|
| H.264 | `h264-decode` | an `avcC` record (length-prefixed packets), Annex B parameter sets, or empty (Annex B packets) |
| HEVC | `hevc-decode` | an `hvcC` record (length-prefixed packets), Annex B parameter sets, or empty (Annex B packets) |
| VP9 | `vp9-decode` | ignored |
| AV1 | `av1-decode` | an `av1C` record, or empty |
| MPEG-2 and MPEG-1 video | `mpeg2-decode` | a sequence header and its extensions, or empty |
| ProRes | `prores` | ignored (frame headers carry the format) |
| DNxHD / DNxHR | `dnx` | ignored (frame headers carry the format) |
| APV | `apv-decode` | ignored (access units carry the format) |
| FFV1 | `ffv1` | required version 3 configuration record; container width and height are required too |
| Motion JPEG | `mjpeg` | ignored (each packet carries its own picture) |
| VP8 | `vp8-decode` | ignored (key frames carry the format) |

Configuration that holds picture data (a slice, a frame OBU, an MPEG-2 picture) is refused.
Errors are `VideoError`: `Unsupported`, `Malformed`, or a picture that did not convert.

When a picture does not convert, the frames finished with it are kept and the next call returns
them first. When the codec itself fails, the pictures it had finished are up to the codec:

- H.264 keeps them for later calls, also when the error comes at `flush`.
- HEVC does too, but only its newest 32 while calls keep failing, which bounds its memory.
- VP9 keeps those of earlier packets.
- MPEG-2 returns them with the call, unless the packet made too many pictures.
- AV1 drops those the failing `decode` or `flush` had finished.

ProRes, DNx, APV and FFV1 return one frame per successful packet, with its packet timestamp;
VP8 returns no picture for a hidden reference frame. These codecs have no delayed output.
APV uses the codec's default selection of the first decodable primary frame and its alpha.
FFV1 has the underlying decoder's version 3, range-coded, key-frame-only support. Reset keeps
its configuration and container dimensions; VP8 reset discards its reference pictures.
