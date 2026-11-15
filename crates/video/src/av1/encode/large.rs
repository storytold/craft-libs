//! Tiled AV1 encoding for frames outside the clean-room encoder's single-tile limits.

use super::{EncoderConfig, Error, Frame, Packet, RateControl};
use rav1e::prelude::{
    ChromaSampling, ColorDescription, ColorPrimaries, Config, Context, EncoderConfig as Rav1eConfig, FrameParameters, FrameType, FrameTypeOverride,
    MatrixCoefficients, PixelRange, Rational, TransferCharacteristics,
};

/// Tiled AV1 encoder used when a frame is too large for [`super::Encoder`].
pub struct LargeEncoder {
    ctx: Context<u8>,
    width: usize,
    height: usize,
    seq_header: Option<Vec<u8>>,
    finished: bool,
}

/// An encoded frame and the number of the input frame it represents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BufferedPacket {
    pub input_frame_number: u64,
    pub packet: Packet,
}

impl LargeEncoder {
    /// Creates a tiled 8-bit 4:2:0 encoder for dimensions up to 8192 samples per side.
    pub fn new(cfg: &EncoderConfig) -> Result<Self, Error> {
        Self::with_speed(cfg, 6)
    }

    /// Creates the tiled encoder with a rav1e speed preset from 1 (slower) to 10 (faster).
    pub fn with_speed(cfg: &EncoderConfig, speed: u8) -> Result<Self, Error> {
        if cfg.width < 16 || cfg.height < 16 || cfg.width > 8192 || cfg.height > 8192 {
            return Err(Error::InvalidConfig("large AV1 frame dimensions must be 16..=8192".into()));
        }
        if cfg.bit_depth != 8 || cfg.fps_num == 0 || cfg.fps_den == 0 {
            return Err(Error::InvalidConfig("large AV1 encoding requires 8-bit input and a valid frame rate".into()));
        }
        let RateControl::ConstantQ(quality) = cfg.rate else {
            return Err(Error::InvalidConfig("large AV1 encoding currently supports constant quantizer rate control".into()));
        };
        let mut ec = Rav1eConfig::with_speed_preset(speed.clamp(1, 10));
        ec.width = cfg.width as usize;
        ec.height = cfg.height as usize;
        ec.time_base = Rational::new(u64::from(cfg.fps_den), u64::from(cfg.fps_num));
        ec.bit_depth = 8;
        ec.chroma_sampling = ChromaSampling::Cs420;
        ec.pixel_range = if cfg.full_range { PixelRange::Full } else { PixelRange::Limited };
        ec.color_description = Some(ColorDescription {
            color_primaries: ColorPrimaries::BT709,
            transfer_characteristics: TransferCharacteristics::BT709,
            matrix_coefficients: MatrixCoefficients::BT709,
        });
        ec.low_latency = true;
        ec.quantizer = quality.max(1) as usize;
        ec.min_quantizer = ec.quantizer.min(255) as u8;
        ec.min_key_frame_interval = 1;
        ec.max_key_frame_interval = cfg.keyint.clamp(1, 120) as u64;
        ec.tiles = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(2, 16);
        let ctx = Config::new().with_encoder_config(ec).new_context().map_err(|e| Error::InvalidConfig(format!("rav1e: {e}")))?;
        Ok(Self { ctx, width: cfg.width as usize, height: cfg.height as usize, seq_header: None, finished: false })
    }

    fn submit(&mut self, frame: &Frame<'_>, force_key: bool) -> Result<(), Error> {
        if self.finished {
            return Err(Error::InvalidFrame("AV1 encoder was already finished".into()));
        }
        if frame.y_stride < self.width
            || frame.uv_stride < self.width / 2
            || frame.y.len() < frame.y_stride.saturating_mul(self.height)
            || frame.u.len() < frame.uv_stride.saturating_mul(self.height / 2)
            || frame.v.len() < frame.uv_stride.saturating_mul(self.height / 2)
        {
            return Err(Error::InvalidFrame("plane dimensions or strides do not match the configured frame".into()));
        }
        let mut input = self.ctx.new_frame();
        copy_plane(&mut input.planes[0], frame.y, frame.y_stride, self.width, self.height)?;
        copy_plane(&mut input.planes[1], frame.u, frame.uv_stride, self.width / 2, self.height / 2)?;
        copy_plane(&mut input.planes[2], frame.v, frame.uv_stride, self.width / 2, self.height / 2)?;
        let params =
            FrameParameters { frame_type_override: if force_key { FrameTypeOverride::Key } else { FrameTypeOverride::No }, ..Default::default() };
        self.ctx.send_frame((input, params)).map_err(|e| Error::InvalidFrame(format!("rav1e: {e}")))?;
        Ok(())
    }

    fn packet(&mut self, packet: rav1e::prelude::Packet<u8>) -> Result<BufferedPacket, Error> {
        let (data, seq) = sample_data(&packet.data)?;
        if self.seq_header.is_none() {
            self.seq_header = seq;
        }
        Ok(BufferedPacket { input_frame_number: packet.input_frameno, packet: Packet { data, keyframe: packet.frame_type == FrameType::KEY } })
    }

    fn drain(&mut self) -> Result<Vec<BufferedPacket>, Error> {
        let mut packets = Vec::new();
        loop {
            match self.ctx.receive_packet() {
                Ok(packet) => packets.push(self.packet(packet)?),
                Err(rav1e::prelude::EncoderStatus::Encoded) => {}
                Err(rav1e::prelude::EncoderStatus::NeedMoreData) if !self.finished => return Ok(packets),
                Err(rav1e::prelude::EncoderStatus::LimitReached) if self.finished => return Ok(packets),
                Err(e) => return Err(Error::InvalidFrame(format!("rav1e: {e}"))),
            }
        }
    }

    /// Submit a frame and return all packets available now, which may be empty while the encoder
    /// buffers input. Call [`Self::finish`] after the last frame to retrieve delayed packets.
    pub fn push(&mut self, frame: &Frame<'_>, force_key: bool) -> Result<Vec<BufferedPacket>, Error> {
        self.submit(frame, force_key)?;
        self.drain()
    }

    /// Finish the stream and return its remaining packets. Repeated calls return no packets.
    pub fn finish(&mut self) -> Result<Vec<BufferedPacket>, Error> {
        if !self.finished {
            self.ctx.send_frame(None).map_err(|e| Error::InvalidFrame(format!("rav1e: {e}")))?;
            self.finished = true;
        }
        self.drain()
    }

    /// Encodes one frame if a packet is immediately available. The input is accepted even if
    /// rav1e needs more frames and this returns an error; new callers should use [`Self::push`].
    pub fn encode_with_key(&mut self, frame: &Frame<'_>, force_key: bool) -> Result<Packet, Error> {
        self.submit(frame, force_key)?;
        loop {
            match self.ctx.receive_packet() {
                Ok(packet) => {
                    return Ok(self.packet(packet)?.packet);
                }
                Err(rav1e::prelude::EncoderStatus::Encoded) => continue,
                Err(rav1e::prelude::EncoderStatus::NeedMoreData | rav1e::prelude::EncoderStatus::LimitReached) => {
                    return Err(Error::InvalidFrame("rav1e did not emit a low-latency frame".into()));
                }
                Err(e) => return Err(Error::InvalidFrame(format!("rav1e: {e}"))),
            }
        }
    }

    /// Returns the AV1 configuration record after the first frame has been encoded.
    pub fn av1c(&self) -> Option<Vec<u8>> {
        self.seq_header.as_ref().map(|obu| {
            let mut config = vec![0x81, 31, 0x0c, 0];
            config.extend_from_slice(obu);
            config
        })
    }
}

fn copy_plane(plane: &mut rav1e::prelude::Plane<u8>, source: &[u16], stride: usize, width: usize, height: usize) -> Result<(), Error> {
    let mut packed = Vec::with_capacity(width * height);
    for row in 0..height {
        let start = row * stride;
        let pixels = &source[start..start + width];
        if pixels.iter().any(|&value| value > u8::MAX as u16) {
            return Err(Error::InvalidFrame("8-bit AV1 samples must be in 0..=255".into()));
        }
        packed.extend(pixels.iter().map(|&value| value as u8));
    }
    plane.copy_from_raw_u8(&packed, width, 1);
    Ok(())
}

fn sample_data(data: &[u8]) -> Result<(Vec<u8>, Option<Vec<u8>>), Error> {
    let mut pos = 0usize;
    let mut sample = Vec::with_capacity(data.len());
    let mut sequence = None;
    while pos < data.len() {
        let start = pos;
        let header = data[pos];
        if header & 0x80 != 0 {
            return Err(Error::InvalidFrame("rav1e emitted an invalid OBU".into()));
        }
        let kind = (header >> 3) & 0x0f;
        pos += 1 + usize::from(header & 0x04 != 0);
        let end = if header & 0x02 != 0 {
            let mut size = 0usize;
            let mut shift = 0usize;
            loop {
                let byte = *data.get(pos).ok_or_else(|| Error::InvalidFrame("truncated OBU size".into()))?;
                pos += 1;
                size |= usize::from(byte & 0x7f).checked_shl(shift as u32).ok_or_else(|| Error::InvalidFrame("OBU size overflow".into()))?;
                if byte & 0x80 == 0 {
                    break;
                }
                shift += 7;
                if shift >= usize::BITS as usize {
                    return Err(Error::InvalidFrame("OBU size overflow".into()));
                }
            }
            pos.checked_add(size).filter(|end| *end <= data.len()).ok_or_else(|| Error::InvalidFrame("OBU exceeds packet bounds".into()))?
        } else {
            data.len()
        };
        if kind == 1 && sequence.is_none() {
            sequence = Some(data[start..end].to_vec());
        }
        if kind != 2 && kind != 15 {
            sample.extend_from_slice(&data[start..end]);
        }
        pos = end;
    }
    Ok((sample, sequence))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buffered_frames_are_returned_with_input_numbers_and_finish_drains() {
        let cfg = EncoderConfig::new(64, 48, 24, 1);
        let mut encoder = LargeEncoder::with_speed(&cfg, 10).unwrap();
        assert!(encoder.av1c().is_none());
        let y = vec![96; 64 * 48];
        let uv = vec![128; 32 * 24];
        let frame = Frame { y: &y, u: &uv, v: &uv, y_stride: 64, uv_stride: 32 };
        let mut packets = Vec::new();
        for i in 0..4 {
            packets.extend(encoder.push(&frame, i == 2).unwrap());
        }
        packets.extend(encoder.finish().unwrap());
        assert_eq!(packets.iter().map(|p| p.input_frame_number).collect::<Vec<_>>(), [0, 1, 2, 3]);
        assert!(packets[0].packet.keyframe);
        assert!(packets[2].packet.keyframe, "requested key frame");
        assert!(encoder.av1c().is_some());
        #[cfg(feature = "av1-decode")]
        {
            let mut decoder = crate::av1::decode::Decoder::with_threads(1);
            let mut frames = Vec::new();
            for packet in &packets {
                frames.extend(decoder.decode(&packet.packet.data).unwrap());
            }
            frames.extend(decoder.flush_result().unwrap());
            assert_eq!(frames.len(), 4);
            assert!(frames.iter().all(|frame| (frame.width, frame.height) == (64, 48)));
        }
        assert!(encoder.finish().unwrap().is_empty());
        assert!(encoder.push(&frame, false).is_err());
    }

    #[test]
    fn a_large_frame_can_be_flushed_even_when_the_first_push_is_buffered() {
        let cfg = EncoderConfig::new(4112, 16, 30, 1);
        let mut encoder = LargeEncoder::with_speed(&cfg, 10).unwrap();
        let y = vec![64; 4112 * 16];
        let uv = vec![128; 2056 * 8];
        let frame = Frame { y: &y, u: &uv, v: &uv, y_stride: 4112, uv_stride: 2056 };
        let mut packets = encoder.push(&frame, false).unwrap();
        packets.extend(encoder.finish().unwrap());
        assert_eq!(packets.len(), 1);
        assert_eq!(packets[0].input_frame_number, 0);
        assert!(packets[0].packet.keyframe);
        assert!(!packets[0].packet.data.is_empty());
        assert!(encoder.av1c().is_some());
    }

    #[test]
    fn invalid_frame_is_not_queued_and_empty_stream_finishes() {
        let cfg = EncoderConfig::new(64, 48, 24, 1);
        let mut encoder = LargeEncoder::with_speed(&cfg, 10).unwrap();
        let short = [0; 1];
        let frame = Frame { y: &short, u: &short, v: &short, y_stride: 64, uv_stride: 32 };
        assert!(encoder.push(&frame, false).is_err());
        assert!(encoder.finish().unwrap().is_empty());
        assert!(encoder.av1c().is_none());
    }
}
