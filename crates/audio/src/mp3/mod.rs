//! MPEG-1 / MPEG-2 Audio Layer III (MP3): a clean-room decoder ([`Decoder`], feature
//! `mp3-decode`) and encoder ([`Encoder`], feature `mp3-encode`), written from ISO/IEC 11172-3
//! and ISO/IEC 13818-3. They share the Huffman code tables and scalefactor bands.
//!
//! # Decoder (feature `mp3-decode`)
//!
//! [`Decoder`] decodes Layer III frames one at a time: MPEG-1, the MPEG-2 lower sampling
//! frequencies and MPEG-2.5 (8-48 kHz), every bit rate (CBR or VBR), mono, stereo, dual channel
//! and joint stereo (mid/side and intensity), long, short and mixed blocks, across the bit
//! reservoir. [`InfoTag`] reads the Xing / Info or VBRI frame at the start of a file and the
//! LAME tag's encoder delay and padding. `craft_audio::decode` (feature `decode-native`) reads
//! `.mp3` files and MP3 in MP4 and Matroska through them, gapless.
//!
//! # Encoder (feature `mp3-encode`)
//!
//! A clean-room encoder, written from ISO/IEC 11172-3:1993
//! (MPEG-1 Audio: §2.4.1 bit stream syntax, §2.4.2 semantics, Annex B tables, Annex C encoding
//! process) and ISO/IEC 13818-3:1998 (the lower sampling frequencies).
//!
//! - Constant bit rate: 32-320 kbit/s at 32 / 44.1 / 48 kHz (MPEG-1, two granules per frame) and
//!   8-160 kbit/s at 16 / 22.05 / 24 kHz (MPEG-2 LSF, one granule per frame).
//! - Mono, stereo, or joint stereo with frame-by-frame mid/side coding.
//! - The polyphase analysis filterbank of Annex C.1.3, the normal-block MDCT and alias reduction;
//!   long blocks only (no block switching yet).
//! - A simple psychoacoustic model (spread band energies against the threshold in quiet), the
//!   inner / outer quantization loops with scalefactors, Huffman table and region selection, and
//!   the bit reservoir (`main_data_begin`).
//! - Output: a plain stream of frames, no ID3 or Xing / Info tag, no CRC. The frames written one
//!   after another are a playable `.mp3` file.
//!
//! From FilmCraft's `filmcraft-mp3enc` (storytold/filmcraft@4bec22b). Public items keep their
//! names, so FilmCraft can re-export this module.

// Index loops follow the specification's pseudo-code (FilmCraft allows this lint workspace-wide).
#![allow(clippy::needless_range_loop)]

#[cfg(feature = "mp3-decode")]
mod decoder;
#[cfg(feature = "mp3-encode")]
mod encoder;
#[cfg(feature = "mp3-encode")]
mod filterbank;
#[cfg(feature = "mp3-encode")]
mod huffman;
mod huffman_tables;
#[cfg(feature = "mp3-encode")]
mod quantize;
mod tables;

#[cfg(feature = "mp3-decode")]
pub use decoder::{Decoder, InfoTag};
#[cfg(feature = "mp3-encode")]
pub use encoder::{Config, DELAY, Encoder, Error, Result, SAMPLE_RATES, StereoMode, bitrates, nearest_bitrate, nearest_sample_rate};
