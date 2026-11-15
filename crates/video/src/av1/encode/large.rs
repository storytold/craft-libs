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
        Ok(Self { ctx, width: cfg.width as usize, height: cfg.height as usize, seq_header: None })
    }

    /// Encodes one frame. The frame planes must match the configured 4:2:0 dimensions.
    pub fn encode_with_key(&mut self, frame: &Frame<'_>, force_key: bool) -> Result<Packet, Error> {
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
        loop {
            match self.ctx.receive_packet() {
                Ok(packet) => {
                    let (data, seq) = sample_data(&packet.data)?;
                    if self.seq_header.is_none() {
                        self.seq_header = seq;
                    }
                    return Ok(Packet { data, keyframe: packet.frame_type == FrameType::KEY });
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
