//! Vorbis I: the pure-Rust [`decoder`] (feature `vorbis-decode`) and [`encoder`] (feature
//! `vorbis-encode`). `craft_audio::decode` still reads Vorbis files through symphonia.

#[cfg(feature = "vorbis-decode")]
pub mod decoder;
#[cfg(feature = "vorbis-encode")]
pub mod encoder;
