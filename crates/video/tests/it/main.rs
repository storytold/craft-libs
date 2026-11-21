//! Integration tests grouped by codec, retaining each former binary's feature gate.

#[cfg(feature = "vmd-decode")]
#[path = "vmd/main.rs"]
mod vmd;

#[cfg(feature = "hap")]
#[path = "hap.rs"]
mod hap;

#[cfg(feature = "mjpeg")]
#[path = "mjpeg.rs"]
mod mjpeg;

#[cfg(feature = "dnx")]
#[path = "dnx/main.rs"]
mod dnx;

#[cfg(feature = "apv")]
#[path = "apv/main.rs"]
mod apv;

#[cfg(feature = "vp8-decode")]
#[path = "vp8/main.rs"]
mod vp8;

#[cfg(feature = "av1-decode")]
#[path = "av1_decode/main.rs"]
mod av1_decode;

#[cfg(feature = "h264-decode")]
#[path = "h264_decode/main.rs"]
mod h264_decode;

#[cfg(feature = "h264-encode")]
#[path = "h264_encode/main.rs"]
mod h264_encode;

#[cfg(feature = "hevc-decode")]
#[path = "hevc_decode/main.rs"]
mod hevc_decode;

#[cfg(feature = "vp8")]
#[path = "vp8_encode/main.rs"]
mod vp8_encode;

#[cfg(feature = "vp9-decode")]
#[path = "vp9_decode/main.rs"]
mod vp9_decode;

#[cfg(feature = "mpeg4-encode")]
#[path = "mpeg4_encode/main.rs"]
mod mpeg4_encode;

#[cfg(feature = "mpeg2-decode")]
#[path = "mpeg2/main.rs"]
mod mpeg2;

#[cfg(feature = "ffv1")]
#[path = "ffv1.rs"]
mod ffv1;

#[cfg(feature = "prores")]
#[path = "prores/main.rs"]
mod prores;

#[cfg(feature = "hevc-encode")]
#[path = "hevc/main.rs"]
mod hevc;

#[cfg(feature = "av1")]
#[path = "av1/main.rs"]
mod av1;

#[path = "track/main.rs"]
mod track;
