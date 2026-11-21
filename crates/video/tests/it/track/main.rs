//! `TrackDecoder` on real streams: ffmpeg encodes a short clip into MP4, MPEG-TS or AVI,
//! craft-media demuxes it, and the decoder's frames must be ffmpeg's decode of the file, in display
//! order, with the timestamps of the packets that coded them. Each codec also checks that a reset
//! decodes the track again and decodes from a later key packet.
#![cfg(any(feature = "h264-decode", feature = "hevc-decode", feature = "vp9-decode", feature = "av1-decode", feature = "mpeg2-decode"))]

mod common;

#[cfg(feature = "av1-decode")]
mod av1;
#[cfg(feature = "h264-decode")]
mod h264;
#[cfg(feature = "hevc-decode")]
mod hevc;
#[cfg(feature = "mpeg2-decode")]
mod mpeg2;
#[cfg(feature = "vp9-decode")]
mod vp9;
