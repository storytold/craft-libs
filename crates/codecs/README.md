# craft-codecs

Umbrella crate centralizing pure-Rust video codecs for the Storytold Craft Apps ecosystem.

## Canonical Ownership & Architecture

`craft-libs` is the single canonical source of truth for the shared video codecs:
- `craft-h264enc`: Clean-room H.264/AVC baseline, main, and high profile encoder with CABAC/CAVLC rate control.
- `craft-prores`: Clean-room Apple ProRes 422 (Proxy, LT, Standard, HQ) and 4444 encoder and decoder (SMPTE RDD 36).
- `craft-dnx`: Avid DNxHD / DNxHR encoder and decoder (SMPTE ST 2019-1:2016).
- `craft-vp8`: RFC 6386 VP8 frame decoder.
- `craft-hevcenc`: Pure-Rust HEVC (H.265) video encoder with CABAC and rate control.
- `craft-vp9enc`: Pure-Rust VP9 Profile 0 video encoder.
- `craft-av1enc`: Pure-Rust AV1 video encoder with normative spec tables and OBU bitstream generation.

## Application Migration & De-duplication Plan

Previously, several applications (specifically `storytold/filmcraft`) maintained in-tree copies of codec implementations. With the centralization into `craft-libs`:
1. `craft-libs` owns and maintains all shared video codecs going forward.
2. In `storytold/filmcraft`, a planned follow-up will migrate workspace dependencies to `craft-libs` crates (`craft-h264enc`, `craft-prores`, etc.) and deprecate or remove the in-tree duplicate crates (`crates/h264enc`, `crates/prores`, `crates/dnx`, `crates/vp8`, `crates/hevc`, `crates/vp9`, `crates/av1`).
3. Audio codecs (AAC, Opus, WAV/AIFF/FLAC I/O) are owned by `craft-audio` (#441–#448), keeping video and audio concerns cleanly separated.

