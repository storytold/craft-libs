//! Integration tests, built into the library's own test executable.

#[cfg(feature = "adpcm")]
#[path = "adpcm_oracle.rs"]
mod adpcm_oracle;
#[cfg(all(feature = "decode", feature = "lossless-encode", feature = "peaks"))]
#[path = "audio/main.rs"]
mod audio;
#[cfg(all(feature = "decode", feature = "encode"))]
#[path = "codecs/main.rs"]
mod codecs;
#[cfg(feature = "dsd")]
#[path = "dsd_bounds.rs"]
mod dsd_bounds;
#[cfg(feature = "dsd")]
#[path = "dsd_metadata.rs"]
mod dsd_metadata;
#[cfg(feature = "dsd")]
#[path = "dsd_oracle.rs"]
mod dsd_oracle;
#[cfg(all(feature = "flac-decode", feature = "flac-encode"))]
#[path = "flac_decode.rs"]
mod flac_decode;
#[cfg(feature = "g722")]
#[path = "g722_oracle.rs"]
mod g722_oracle;
#[cfg(feature = "g726")]
#[path = "g726_oracle.rs"]
mod g726_oracle;
#[cfg(feature = "gsm")]
#[path = "gsm_oracle.rs"]
mod gsm_oracle;
#[cfg(feature = "mpa-encode")]
#[path = "mp2_encoder_oracle.rs"]
mod mp2_encoder_oracle;
#[cfg(feature = "qoa")]
#[path = "qoa_oracle.rs"]
mod qoa_oracle;
#[cfg(feature = "sbc")]
#[path = "sbc_oracle.rs"]
mod sbc_oracle;
#[cfg(feature = "vmd-decode")]
#[path = "vmd.rs"]
mod vmd;
#[cfg(all(feature = "vorbis-decode", feature = "vorbis-encode"))]
#[path = "vorbis_decode_hostile.rs"]
mod vorbis_decode_hostile;
#[cfg(all(feature = "vorbis-decode", feature = "vorbis-encode", feature = "decode"))]
#[path = "vorbis_decode_oracle.rs"]
mod vorbis_decode_oracle;
#[cfg(feature = "vorbis-decode")]
#[path = "vorbis_decode_synthetic.rs"]
mod vorbis_decode_synthetic;
