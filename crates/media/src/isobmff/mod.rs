//! Clean-room ISO Base Media File Format (MP4, ISO/IEC 14496-12/14/15) and QuickTime (MOV) support.
//!
//! - **Demux:** [`open`] parses box headers plus `moov` (and `moof` for fragmented files) from a
//!   [`ByteSource`] and builds per-track [`Sample`] tables; media data is read on demand with
//!   [`Mp4File::read_sample`].
//! - **Mux:** [`Mp4Writer`] writes progressive MP4/MOV (optionally faststart);
//!   [`FragmentedWriter`] writes fragmented MP4. Both can write a chapter list ([`chapters`]).
//! - **Boxes:** [`boxes`] iterates the boxes in a byte slice and reads their payloads, for box
//!   formats and boxes the demuxer does not model (`uuid` boxes, HEIF items, JPEG 2000, CR3).
//!
//! See the crate README for supported boxes and limitations.

pub mod boxes;
mod bytes;
pub mod chapters;
mod codec;
mod config_fields;
mod demux;
mod error;
mod mux;
mod parameter_sets;
mod rtmd;

pub use crate::source::{ByteSource, ReadSeekSource};
pub use bytes::FourCc;
pub use chapters::{Chapter, MAX_CHAPTER_TITLE_BYTES, MAX_CHAPTERS};
pub use codec::{
    AacConfig, ApvConfig, AudioParams, Av1Config, AvcConfig, BitRate, CleanAperture, CodecConfig, ColorInfo, FieldInfo, FlacConfig, HevcConfig,
    HevcNalArray, MasteringDisplay, OpusConfig, PcmConfig, SampleEntry, TimecodeConfig, VideoParams, VpcConfig, parse_asc,
};
pub use config_fields::{avcc_length_size, dac3_layout, dec3_layout, esds_object_type, hvcc_length_size_and_tid};
pub use demux::{Edit, Metadata, Mp4File, Sample, SampleGroup, Track, TrackKind, open};
pub use error::{Error, Result};
pub use mux::{Brand, FragmentedWriter, Mp4Writer, TrackConfig, WriteSample, WriterOptions};
pub use parameter_sets::{AvcSps, HevcSps};
pub use rtmd::{SONY_RTMD_TIMECODE_PREFIX_LEN, SonyRtmdTimecode, parse_sony_rtmd_timecode};
