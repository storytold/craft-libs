//! FLAC framing shared by the MP4 and Matroska packet readers.

use crate::flac_decode::{Decoder, StreamInfo};
use crate::{AudioError, ChannelLayout, Result};

pub(super) struct FlacTrack {
    pub(super) info: StreamInfo,
    decoder: Decoder,
}

impl FlacTrack {
    pub(super) fn new(private: &[u8]) -> Result<(Self, ChannelLayout)> {
        let [b'f', b'L', b'a', b'C', kind, 0, 0, 34, body @ ..] = private else {
            return Err(AudioError::Malformed("FLAC track without a STREAMINFO block".into()));
        };
        if kind & 0x7f != 0 {
            return Err(AudioError::Malformed("FLAC STREAMINFO must be the first metadata block".into()));
        }
        let info = StreamInfo::parse(body).map_err(|error| AudioError::Malformed(format!("FLAC STREAMINFO: {error}")))?;
        if info.sample_rate == 0 {
            return Err(AudioError::Malformed("FLAC track sample rate is zero".into()));
        }
        let layout = crate::flac_layout::layout_of(private, 0, u16::from(info.channels));
        Ok((Self { info, decoder: Decoder::new(info) }, layout))
    }

    pub(super) fn decode(&self, packet: &[u8]) -> Result<Vec<Vec<f32>>> {
        let frame = self.decoder.decode_frame(packet).map_err(|error| AudioError::Malformed(format!("FLAC packet: {error}")))?;
        if frame.size != packet.len() {
            return Err(AudioError::Malformed("FLAC packet must hold exactly one frame".into()));
        }
        if frame.sample_rate != self.info.sample_rate
            || frame.bits_per_sample != self.info.bits_per_sample
            || frame.channels.len() != usize::from(self.info.channels)
            || frame.block_size > usize::from(u16::MAX)
        {
            return Err(AudioError::Malformed("FLAC packet format differs from STREAMINFO".into()));
        }
        let scale = 2f64.powi(1 - i32::from(self.info.bits_per_sample));
        Ok(frame.channels.into_iter().map(|plane| plane.into_iter().map(|sample| (f64::from(sample) * scale) as f32).collect()).collect())
    }

    pub(super) fn packet_frames(&self, data: &[u8]) -> Option<usize> {
        self.decoder.packet_block_size(data)
    }
}
