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
| `mpeg2` | [`mpeg2`](src/mpeg2/README.md) | MPEG-2 video (ITU-T H.262) and MPEG-1 video (ISO/IEC 11172-2): decoder |
| `prores` | `prores` | Apple ProRes (SMPTE RDD 36): decoder and encoder |
| `h264` | [`h264`](src/h264/decode/README.md) | ITU-T H.264 / AVC: decoder (`h264-decode` alone) and encoder (`h264-encode` alone) |
| `hevc` | [`hevc`](src/hevc/decode/README.md) | ITU-T H.265 / HEVC: decoder (`hevc-decode` alone) and encoder (`hevc-encode` alone) |
| `vp9` | [`vp9`](src/vp9/decode/README.md) | VP9: decoder (`vp9-decode` alone) and encoder (`vp9-encode` alone) |
| `av1` | [`av1`](src/av1/decode/README.md) | AV1 (AOMedia Video 1): decoder (`av1-decode` alone) and encoder (`av1-encode` alone) |
| `threads` | — | Multi-threaded slice/row parallelism via `rayon` |
