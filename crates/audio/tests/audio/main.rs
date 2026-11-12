//! Integration tests, built as one test binary: each file is a module.

mod decode_extensions;
mod encode_format;
mod flac_damage;
mod float_headroom;
mod malformed;
mod mp4_length;
mod raw_pcm;
mod roundtrip;
mod stream;
mod wav_empty_data;
