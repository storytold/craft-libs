use crate::SampleType;

/// Shape of a PXR24 block with full-resolution channels in file order.
#[derive(Clone, Copy, Debug)]
pub struct Layout<'a> {
    width: usize,
    rows: usize,
    channels: &'a [SampleType],
    inflated_len: usize,
    output_len: usize,
}

impl<'a> Layout<'a> {
    /// Check the dimensions and byte counts against the caller's plain output limit.
    pub fn new(width: usize, rows: usize, channels: &'a [SampleType], max_output: usize) -> Option<Self> {
        if width == 0 || rows == 0 || channels.is_empty() {
            return None;
        }
        let area = width.checked_mul(rows)?;
        let (inflated_pixel, output_pixel) = channels.iter().try_fold((0usize, 0usize), |(inflated, output), &channel| {
            let (planes, plain) = sample_bytes(channel);
            Some((inflated.checked_add(planes)?, output.checked_add(plain)?))
        })?;
        let inflated_len = inflated_pixel.checked_mul(area)?;
        let output_len = output_pixel.checked_mul(area)?;
        (output_len <= max_output).then_some(Self { width, rows, channels, inflated_len, output_len })
    }

    /// Exact number of bytes required from the zlib reader.
    pub fn inflated_len(self) -> usize {
        self.inflated_len
    }

    /// Reconstruct little-endian samples in row, channel and pixel order.
    ///
    /// Each scanline and channel starts a new horizontal predictor. The stored
    /// 24 bits of a FLOAT sample become its high bits, with eight low zero bits.
    /// Returns `None` when the plane length is wrong or allocation fails.
    pub fn unpack(self, mut input: &[u8]) -> Option<Vec<u8>> {
        if input.len() != self.inflated_len {
            return None;
        }
        let mut output = Vec::new();
        output.try_reserve_exact(self.output_len).ok()?;
        for _ in 0..self.rows {
            for &channel in self.channels {
                let (planes, plain) = sample_bytes(channel);
                let (line, rest) = input.split_at_checked(self.width * planes)?;
                input = rest;
                let mut sample = 0u32;
                for x in 0..self.width {
                    let delta = line[x..].iter().step_by(self.width).fold(0u32, |value, &byte| (value << 8) | u32::from(byte));
                    sample = sample.wrapping_add(delta);
                    let bytes = match channel {
                        SampleType::F32 => (sample << 8).to_le_bytes(),
                        SampleType::F16 | SampleType::U32 => sample.to_le_bytes(),
                    };
                    output.extend_from_slice(&bytes[..plain]);
                }
            }
        }
        Some(output)
    }
}

fn sample_bytes(channel: SampleType) -> (usize, usize) {
    match channel {
        SampleType::F16 => (2, 2),
        SampleType::F32 => (3, 4),
        SampleType::U32 => (4, 4),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIXED: [SampleType; 3] = [SampleType::F16, SampleType::F32, SampleType::U32];

    #[test]
    fn mixed_types_wrap_and_reset_across_rows() {
        #[rustfmt::skip]
        let planes = [
            0xff, 0x00, 0xfe, 0x03,
            0x7f, 0xff, 0x01, 0xfe, 0x23, 0xdd,
            0xff, 0x00, 0xff, 0x00, 0xff, 0x00, 0xf0, 0x30,
            0x01, 0xff, 0x02, 0xfe,
            0x00, 0xff, 0x00, 0xff, 0x01, 0xfe,
            0x12, 0xff, 0x34, 0xff, 0x56, 0xff, 0x78, 0xff,
        ];
        #[rustfmt::skip]
        let expected = [
            0xfe, 0xff, 0x01, 0x00,
            0x00, 0x23, 0x01, 0x7f, 0x00, 0x00, 0x00, 0x7f,
            0xf0, 0xff, 0xff, 0xff, 0x20, 0x00, 0x00, 0x00,
            0x02, 0x01, 0x00, 0x01,
            0x00, 0x01, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff,
            0x78, 0x56, 0x34, 0x12, 0x77, 0x56, 0x34, 0x12,
        ];
        let layout = Layout::new(2, 2, &MIXED, expected.len());
        assert_eq!(layout.map(Layout::inflated_len), Some(planes.len()));
        assert_eq!(layout.and_then(|layout| layout.unpack(&planes)).as_deref(), Some(expected.as_slice()));
    }

    #[test]
    fn rejects_invalid_shapes_and_plane_lengths() {
        assert!(Layout::new(0, 1, &MIXED, 40).is_none());
        assert!(Layout::new(1, 0, &MIXED, 40).is_none());
        assert!(Layout::new(1, 1, &[], 40).is_none());
        assert!(Layout::new(usize::MAX, 2, &MIXED, usize::MAX).is_none());
        assert!(Layout::new(usize::MAX, 1, &MIXED, usize::MAX).is_none());
        assert!(Layout::new(2, 2, &MIXED, 39).is_none());
        let short = Layout::new(2, 2, &MIXED, 40).and_then(|layout| layout.unpack(&[0; 35]));
        let long = Layout::new(2, 2, &MIXED, 40).and_then(|layout| layout.unpack(&[0; 37]));
        assert!(short.is_none());
        assert!(long.is_none());
    }
}
