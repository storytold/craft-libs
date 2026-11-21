//! The `craft_process` descriptor from `ABI.md` section 7. Little endian, no padding, versioned by `size`.

use std::fmt;

/// Byte size of the version 1 descriptor.
pub const DESCRIPTOR_V1_SIZE: usize = 96;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Kind {
    Image = 1,
    Audio = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Sample {
    U8 = 1,
    U16 = 2,
    F16 = 3,
    F32 = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Alpha {
    None = 0,
    Straight = 1,
    Premultiplied = 2,
}

/// Why a descriptor failed to decode.
#[derive(Clone, Debug, PartialEq)]
pub enum DescriptorError {
    /// Fewer bytes than the `size` field (or the 4 bytes of the `size` field itself) promise.
    Truncated { needed: usize, got: usize },
    /// `size` is smaller than the version 1 layout.
    SizeTooSmall(u32),
    /// An enum field holds a value `ABI.md` does not define.
    BadEnum { field: &'static str, value: u32 },
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DescriptorError::Truncated { needed, got } => write!(f, "descriptor truncated: need {needed} bytes, got {got}"),
            DescriptorError::SizeTooSmall(s) => write!(f, "descriptor size {s} is below the v1 size {DESCRIPTOR_V1_SIZE}"),
            DescriptorError::BadEnum { field, value } => write!(f, "descriptor field {field} has undefined value {value}"),
        }
    }
}

impl std::error::Error for DescriptorError {}

/// A decoded descriptor. `size` is not stored. `encode` always writes `DESCRIPTOR_V1_SIZE`.
#[derive(Clone, Debug, PartialEq)]
pub struct ProcessDescriptor {
    pub kind: Kind,
    pub entry: u32,
    pub w: u32,
    pub h: u32,
    pub stride_bytes: u32,
    pub channels: u32,
    pub sample: Sample,
    pub alpha: Alpha,
    pub color_mode: u32,
    pub depth: u32,
    pub origin_x: i32,
    pub origin_y: i32,
    pub overlap: u32,
    pub scale: f64,
    pub time: f64,
    pub sample_rate: u32,
    pub frames: u32,
    pub in_ptr: u32,
    pub out_ptr: u32,
    pub params_ptr: u32,
    pub params_len: u32,
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn bytes<const N: usize>(&mut self) -> [u8; N] {
        let mut out = [0u8; N];
        out.copy_from_slice(&self.buf[self.pos..self.pos + N]);
        self.pos += N;
        out
    }
    fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.bytes())
    }
    fn i32(&mut self) -> i32 {
        i32::from_le_bytes(self.bytes())
    }
    fn f64(&mut self) -> f64 {
        f64::from_le_bytes(self.bytes())
    }
}

impl ProcessDescriptor {
    /// Write the version 1 layout.
    pub fn encode(&self) -> [u8; DESCRIPTOR_V1_SIZE] {
        let mut out = Vec::with_capacity(DESCRIPTOR_V1_SIZE);
        let u = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());
        u(&mut out, DESCRIPTOR_V1_SIZE as u32);
        u(&mut out, self.kind as u32);
        u(&mut out, self.entry);
        u(&mut out, self.w);
        u(&mut out, self.h);
        u(&mut out, self.stride_bytes);
        u(&mut out, self.channels);
        u(&mut out, self.sample as u32);
        u(&mut out, self.alpha as u32);
        u(&mut out, self.color_mode);
        u(&mut out, self.depth);
        out.extend_from_slice(&self.origin_x.to_le_bytes());
        out.extend_from_slice(&self.origin_y.to_le_bytes());
        u(&mut out, self.overlap);
        out.extend_from_slice(&self.scale.to_le_bytes());
        out.extend_from_slice(&self.time.to_le_bytes());
        u(&mut out, self.sample_rate);
        u(&mut out, self.frames);
        u(&mut out, self.in_ptr);
        u(&mut out, self.out_ptr);
        u(&mut out, self.params_ptr);
        u(&mut out, self.params_len);
        let mut arr = [0u8; DESCRIPTOR_V1_SIZE];
        arr.copy_from_slice(&out);
        arr
    }

    /// Read a descriptor. A `size` larger than v1 is accepted: later versions append fields, and
    /// this reader ignores the tail. Bytes past `size` in `buf` are ignored too.
    pub fn decode(buf: &[u8]) -> Result<ProcessDescriptor, DescriptorError> {
        if buf.len() < 4 {
            return Err(DescriptorError::Truncated { needed: 4, got: buf.len() });
        }
        let size = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
        if (size as usize) < DESCRIPTOR_V1_SIZE {
            return Err(DescriptorError::SizeTooSmall(size));
        }
        if buf.len() < size as usize {
            return Err(DescriptorError::Truncated { needed: size as usize, got: buf.len() });
        }
        let mut r = Reader { buf, pos: 4 };
        let kind = r.u32();
        let entry = r.u32();
        let (w, h, stride_bytes, channels) = (r.u32(), r.u32(), r.u32(), r.u32());
        let sample = r.u32();
        let alpha = r.u32();
        let color_mode = r.u32();
        let depth = r.u32();
        let (origin_x, origin_y) = (r.i32(), r.i32());
        let overlap = r.u32();
        let (scale, time) = (r.f64(), r.f64());
        let (sample_rate, frames) = (r.u32(), r.u32());
        let (in_ptr, out_ptr, params_ptr, params_len) = (r.u32(), r.u32(), r.u32(), r.u32());
        Ok(ProcessDescriptor {
            kind: match kind {
                1 => Kind::Image,
                2 => Kind::Audio,
                value => return Err(DescriptorError::BadEnum { field: "kind", value }),
            },
            entry,
            w,
            h,
            stride_bytes,
            channels,
            sample: match sample {
                1 => Sample::U8,
                2 => Sample::U16,
                3 => Sample::F16,
                4 => Sample::F32,
                value => return Err(DescriptorError::BadEnum { field: "sample", value }),
            },
            alpha: match alpha {
                0 => Alpha::None,
                1 => Alpha::Straight,
                2 => Alpha::Premultiplied,
                value => return Err(DescriptorError::BadEnum { field: "alpha", value }),
            },
            color_mode,
            depth,
            origin_x,
            origin_y,
            overlap,
            scale,
            time,
            sample_rate,
            frames,
            in_ptr,
            out_ptr,
            params_ptr,
            params_len,
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    fn sample_desc() -> ProcessDescriptor {
        ProcessDescriptor {
            kind: Kind::Image,
            entry: 3,
            w: 1920,
            h: 1080,
            stride_bytes: 7680,
            channels: 4,
            sample: Sample::F16,
            alpha: Alpha::Premultiplied,
            color_mode: 3,
            depth: 16,
            origin_x: -12,
            origin_y: 34,
            overlap: 8,
            scale: 0.5,
            time: 12.25,
            sample_rate: 48_000,
            frames: 1024,
            in_ptr: 0x1000,
            out_ptr: 0x2000,
            params_ptr: 0x3000,
            params_len: 17,
        }
    }

    #[test]
    fn layout_is_96_bytes_with_pinned_offsets() {
        let bytes = sample_desc().encode();
        assert_eq!(bytes.len(), 96);
        let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
        assert_eq!(u32_at(0), 96, "size");
        assert_eq!(u32_at(4), 1, "kind");
        assert_eq!(u32_at(8), 3, "entry");
        assert_eq!(u32_at(12), 1920, "w");
        assert_eq!(u32_at(28), 3, "sample f16");
        assert_eq!(u32_at(32), 2, "alpha premultiplied");
        assert_eq!(i32::from_le_bytes(bytes[44..48].try_into().unwrap()), -12, "origin_x");
        assert_eq!(f64::from_le_bytes(bytes[56..64].try_into().unwrap()), 0.5, "scale");
        assert_eq!(f64::from_le_bytes(bytes[64..72].try_into().unwrap()), 12.25, "time");
        assert_eq!(u32_at(72), 48_000, "sample_rate");
        assert_eq!(u32_at(84), 0x2000, "out_ptr");
        assert_eq!(u32_at(92), 17, "params_len");
    }

    #[test]
    fn round_trips() {
        let d = sample_desc();
        assert_eq!(ProcessDescriptor::decode(&d.encode()).unwrap(), d);
        let audio = ProcessDescriptor { kind: Kind::Audio, sample: Sample::F32, alpha: Alpha::None, ..sample_desc() };
        assert_eq!(ProcessDescriptor::decode(&audio.encode()).unwrap(), audio);
    }

    #[test]
    fn every_truncation_is_rejected() {
        let bytes = sample_desc().encode();
        for n in 0..bytes.len() {
            let err = ProcessDescriptor::decode(&bytes[..n]).unwrap_err();
            assert!(matches!(err, DescriptorError::Truncated { .. }), "len {n}: {err:?}");
        }
    }

    #[test]
    fn larger_size_field_is_a_later_version_and_its_tail_is_ignored() {
        let mut v = sample_desc().encode().to_vec();
        v[0..4].copy_from_slice(&112u32.to_le_bytes());
        v.extend_from_slice(&[0xAB; 16]);
        assert_eq!(ProcessDescriptor::decode(&v).unwrap(), sample_desc());
        // The larger size still has to be backed by bytes.
        v.truncate(100);
        assert_eq!(ProcessDescriptor::decode(&v).unwrap_err(), DescriptorError::Truncated { needed: 112, got: 100 });
    }

    #[test]
    fn size_below_v1_is_rejected() {
        let mut v = sample_desc().encode();
        v[0..4].copy_from_slice(&95u32.to_le_bytes());
        assert_eq!(ProcessDescriptor::decode(&v).unwrap_err(), DescriptorError::SizeTooSmall(95));
    }

    #[test]
    fn undefined_enum_values_are_rejected() {
        for (offset, field, value) in [(4usize, "kind", 0u32), (4, "kind", 3), (28, "sample", 0), (28, "sample", 5), (32, "alpha", 3)] {
            let mut v = sample_desc().encode();
            v[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
            assert_eq!(ProcessDescriptor::decode(&v).unwrap_err(), DescriptorError::BadEnum { field, value });
        }
    }
}
