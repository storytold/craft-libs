//! Clean-room ISO Base Media File Format (MP4, ISO/IEC 14496-12/14/15) and QuickTime (MOV) support.
//!
//! - **Demux:** [`open`] parses box headers plus `moov` (and `moof` for fragmented files) from a
//!   [`ByteSource`] and builds per-track [`Sample`] tables; media data is read on demand with
//!   [`Mp4File::read_sample`].
//! - **Mux:** [`Mp4Writer`] writes progressive MP4/MOV (optionally faststart);
//!   [`FragmentedWriter`] writes fragmented MP4.
//! - **Boxes:** [`boxes`] iterates the boxes in a byte slice and reads their payloads, for box
//!   formats and boxes the demuxer does not model (`uuid` boxes, HEIF items, JPEG 2000, CR3).
//!
//! See the crate README for supported boxes and limitations.

pub mod boxes;
mod bytes;
mod codec;
mod demux;
mod error;
mod mux;
mod source;

pub use bytes::FourCc;
pub use codec::{
    AacConfig, ApvConfig, AudioParams, Av1Config, AvcConfig, BitRate, CleanAperture, CodecConfig, ColorInfo, FieldInfo, FlacConfig, HevcConfig,
    HevcNalArray, MasteringDisplay, OpusConfig, PcmConfig, SampleEntry, TimecodeConfig, VideoParams, VpcConfig, parse_asc,
};
pub use demux::{Edit, Metadata, Mp4File, Sample, SampleGroup, Track, TrackKind, open};
pub use error::{Error, Result};
pub use mux::{Brand, FragmentedWriter, Mp4Writer, TrackConfig, WriteSample, WriterOptions};
pub use source::ByteSource;
