# craft-video

Clean-room pure-Rust video codecs for the Craft apps, one feature per codec: DNxHD/DNxHR, VP8,
ProRes, H.264, HEVC, VP9, AV1.

- Input and output are byte slices and planar sample buffers; the crate opens no files itself
  and performs no filesystem I/O.
- Implemented from published standards and RFCs (SMPTE ST 2019-1, RFC 6386, SMPTE RDD 36,
  ITU-T H.264, ITU-T H.265, VP9 bitstream spec, AV1 bitstream spec).
- Tested against ffmpeg as an external oracle with roundtrips, hostile fuzzing, and conformance vectors.
  No `unsafe`, no panics on corrupt streams. Builds for `wasm32-unknown-unknown`.
- Containers (MP4/MOV, Matroska/WebM), bit-level I/O (`craft_media::bitstream`) and shared
  identifiers (`CodecId`, `CodecParams`) come from [`craft-media`](../media/README.md).

## Cargo features

No codec feature is on by default.

| Feature | Module | Codec |
|---|---|---|
| `dnx` | [`dnx`](src/dnx/README.md) | Avid DNxHD / DNxHR (SMPTE ST 2019-1 VC-3): decoder and encoder |
| `vp8` | `vp8` | VP8 (RFC 6386): decoder |
| `threads` | — | Multi-threaded slice/row parallelism via `rayon` |
