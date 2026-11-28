//! OpenEXR read/write for single-part scanline images with HALF, FLOAT or UINT channels at full resolution.
//! Writes NONE, ZIPS or ZIP compression and also reads RLE.
//!
//! ```
//! use craft_material_graph::exr::{self, ExrChannel, ExrImage, SampleKind};
//! let img = ExrImage { width: 2, height: 1, channels: vec![ExrChannel::new("R", SampleKind::Half, vec![0.5, 4.0])] };
//! let back = exr::decode(&exr::encode(&img).unwrap()).unwrap();
//! assert_eq!(back, img);
//! ```

mod compression;
mod decode;
mod encode;
mod header;
// Main's dependency-free Tex writers (uncompressed EXR and Radiance HDR) used by export; merged in from main
// unchanged and re-exported so `exr::encode_exr` etc. keep their paths.
mod tex_writers;

pub use compression::Compression;
pub use decode::decode;
pub use encode::{encode, encode_with};
pub use tex_writers::{encode_exr, encode_hdr, f32_to_half, half_to_f32};

/// Stored sample type of a channel. Samples are handled as f32: HALF values convert exactly both ways after the
/// first rounding, UINT values are exact up to 2^24.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleKind {
    Uint,
    Half,
    Float,
}

impl SampleKind {
    fn code(self) -> i32 {
        match self {
            SampleKind::Uint => 0,
            SampleKind::Half => 1,
            SampleKind::Float => 2,
        }
    }

    fn from_code(c: i32) -> Option<Self> {
        match c {
            0 => Some(SampleKind::Uint),
            1 => Some(SampleKind::Half),
            2 => Some(SampleKind::Float),
            _ => None,
        }
    }

    /// Bytes per stored sample.
    fn size(self) -> usize {
        match self {
            SampleKind::Half => 2,
            SampleKind::Uint | SampleKind::Float => 4,
        }
    }
}

/// One channel: `data` holds `width * height` samples, row-major, top row first.
#[derive(Clone, Debug, PartialEq)]
pub struct ExrChannel {
    pub name: String,
    pub sample: SampleKind,
    pub data: Vec<f32>,
}

impl ExrChannel {
    pub fn new(name: &str, sample: SampleKind, data: Vec<f32>) -> Self {
        ExrChannel { name: name.to_string(), sample, data }
    }
}

/// A scanline image. Channel order is free; files store channels sorted by name and `decode` returns them that way.
#[derive(Clone, Debug, PartialEq)]
pub struct ExrImage {
    pub width: usize,
    pub height: usize,
    pub channels: Vec<ExrChannel>,
}

impl ExrImage {
    pub fn channel(&self, name: &str) -> Option<&ExrChannel> {
        self.channels.iter().find(|c| c.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_image(w: usize, h: usize) -> ExrImage {
        let ramp = |k: f32| (0..w * h).map(|i| ((i % w) as f32 / w as f32 + (i / w) as f32 * k) * 3.0 - 0.5).collect::<Vec<f32>>();
        let half = |v: Vec<f32>| v.into_iter().map(|x| crate::half::f16_to_f32(crate::half::f32_to_f16(x))).collect::<Vec<f32>>();
        ExrImage {
            width: w,
            height: h,
            channels: vec![
                ExrChannel::new("A", SampleKind::Half, half(ramp(0.01))),
                ExrChannel::new("B", SampleKind::Float, ramp(0.3)),
                ExrChannel::new("G", SampleKind::Half, half(ramp(-0.2))),
                ExrChannel::new("R", SampleKind::Float, ramp(1.0)),
                ExrChannel::new("id", SampleKind::Uint, (0..w * h).map(|i| (i * 7 % 1000) as f32).collect()),
            ],
        }
    }

    #[test]
    fn round_trips_every_compression() {
        for (w, h) in [(1, 1), (5, 3), (64, 40), (300, 17)] {
            let img = sample_image(w, h);
            for c in [Compression::None, Compression::Zips, Compression::Zip] {
                let bytes = encode_with(&img, c).unwrap();
                assert_eq!(decode(&bytes).unwrap(), img, "{w}x{h} {c:?}");
            }
        }
    }

    #[test]
    fn channels_are_sorted_on_write() {
        let mut img = sample_image(4, 4);
        img.channels.reverse();
        let back = decode(&encode(&img).unwrap()).unwrap();
        let names: Vec<&str> = back.channels.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["A", "B", "G", "R", "id"]);
        assert_eq!(back.channel("R"), img.channel("R"));
    }

    #[test]
    fn special_values_survive() {
        let v = vec![0.0, -0.0, f32::INFINITY, f32::NEG_INFINITY, 65504.0, 6.0e-8, 1e30, -1e-30];
        let img = ExrImage {
            width: 8,
            height: 1,
            channels: vec![ExrChannel::new("F", SampleKind::Float, v.clone()), ExrChannel::new("H", SampleKind::Half, v)],
        };
        let back = decode(&encode(&img).unwrap()).unwrap();
        assert_eq!(
            back.channels[0].data.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
            img.channels[0].data.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
        );
        let h = &back.channels[1].data;
        assert_eq!((h[2], h[3], h[4]), (f32::INFINITY, f32::NEG_INFINITY, 65504.0));
        assert_eq!((h[5], h[6]), (crate::half::f16_to_f32(1), f32::INFINITY));
        assert!(h[7] == 0.0 && h[7].is_sign_negative());
    }

    #[test]
    fn invalid_images_are_rejected() {
        let bad = |img: ExrImage| assert!(encode(&img).is_err());
        bad(ExrImage { width: 0, height: 1, channels: vec![ExrChannel::new("R", SampleKind::Half, vec![])] });
        bad(ExrImage { width: 1, height: 1, channels: vec![] });
        bad(ExrImage { width: 2, height: 1, channels: vec![ExrChannel::new("R", SampleKind::Half, vec![1.0])] });
        bad(ExrImage {
            width: 1,
            height: 1,
            channels: vec![ExrChannel::new("R", SampleKind::Half, vec![1.0]), ExrChannel::new("R", SampleKind::Float, vec![1.0])],
        });
        bad(ExrImage { width: 1, height: 1, channels: vec![ExrChannel::new("", SampleKind::Half, vec![1.0])] });
    }

    #[test]
    fn truncations_and_corruption_error() {
        let img = sample_image(9, 20);
        for c in [Compression::None, Compression::Zip] {
            let bytes = encode_with(&img, c).unwrap();
            for n in 0..bytes.len() {
                assert!(decode(&bytes[..n]).is_err(), "{c:?} prefix {n}");
            }
            // Flipping bytes must never panic (it may or may not be detected).
            for i in (0..bytes.len()).step_by(3) {
                let mut b = bytes.clone();
                b[i] ^= 0x5a;
                let _ = decode(&b);
            }
        }
    }
}
