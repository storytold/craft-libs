//! Integration tests, built into the library's own test executable.

#[cfg(all(feature = "decode", feature = "lossless-encode", feature = "peaks"))]
#[path = "audio/main.rs"]
mod audio;
#[cfg(all(feature = "decode", feature = "encode"))]
#[path = "codecs/main.rs"]
mod codecs;
#[cfg(all(feature = "flac-decode", feature = "flac-encode"))]
#[path = "flac_decode.rs"]
mod flac_decode;
#[cfg(feature = "gsm")]
#[path = "gsm_oracle.rs"]
mod gsm_oracle;
#[cfg(feature = "qoa")]
#[path = "qoa_oracle.rs"]
mod qoa_oracle;
#[cfg(feature = "sbc")]
#[path = "sbc_oracle.rs"]
mod sbc_oracle;
