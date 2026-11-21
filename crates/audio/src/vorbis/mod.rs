//! Vorbis I: the pure-Rust [`decoder`] (feature `vorbis-decode`) and [`encoder`] (feature
//! `vorbis-encode`). `craft_audio::decode` and `craft_audio::Decoder` read Ogg Vorbis files with
//! the decoder; Vorbis in Matroska still goes through symphonia.

#[cfg(feature = "vorbis-decode")]
pub mod decoder;
#[cfg(feature = "vorbis-encode")]
pub mod encoder;
