//! Integration tests, built into the library's own test executable.

#[cfg(all(feature = "decode", feature = "lossless-encode", feature = "peaks"))]
#[path = "audio/main.rs"]
mod audio;
#[cfg(all(feature = "decode", feature = "encode"))]
#[path = "codecs/main.rs"]
mod codecs;
