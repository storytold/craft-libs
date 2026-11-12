//! A small picture of a file from its merged image, without decoding it whole:
//! [`PsdFile::merged_preview`].
//!
//! Every `k`-th row of each plane is decoded (RLE and raw rows are read on their own, see
//! [`crate::RowDecoder`]) and every `k`-th sample of it kept. For a file parsed with
//! [`PsdFile::from_source`] that reads a few megabytes of a multi-gigabyte file, so the picture
//! can show while the layers are still being decoded.

use crate::compression::Compression;
use crate::error::Result;
use crate::file::PsdFile;
use crate::header::ColorMode;

/// A preview picture: RGBA, 8 bits per channel, row-major, opaque. A transparent document's
/// merged image is stored matted with white, so the picture is the document over white.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width × height × 4` bytes.
    pub rgba: Vec<u8>,
}

impl PsdFile {
    /// The merged image reduced (by a whole factor, nearest sample) to fit `max_side`. `None` when
    /// there is no real merged image to show: an RGB or grayscale one (8/16/32-bit) stored raw
    /// or RLE (a ZIP stream would have to be inflated whole), and not flagged as a placeholder.
    pub fn merged_preview(&self, max_side: u32) -> Result<Option<Preview>> {
        let h = &self.header;
        let color = match h.color_mode {
            ColorMode::Rgb => 3usize,
            ColorMode::Grayscale => 1,
            _ => return Ok(None),
        };
        if !matches!(h.depth, 8 | 16 | 32)
            || !matches!(self.image_data.compression, Compression::Raw | Compression::Rle)
            || self.has_real_merged_data() == Some(false)
            || usize::from(h.channels) < color
            || h.width == 0
            || h.height == 0
        {
            return Ok(None);
        }
        let k = h.width.max(h.height).div_ceil(max_side.max(1)).max(1);
        let (w, ht) = (h.width.div_ceil(k), h.height.div_ceil(k));
        let bps = usize::from(h.depth / 8);
        // At most `max_side`² pixels, but `max_side` is the caller's: checked and fallible.
        let n = (w as usize)
            .checked_mul(ht as usize)
            .and_then(|p| p.checked_mul(4))
            .filter(|&n| n as u64 <= crate::compression::MAX_DECODED_BYTES)
            .ok_or(crate::error::PsdError::LimitExceeded("preview exceeds MAX_DECODED_BYTES"))?;
        let dec = self.image_data.row_decoder(h)?;
        let mut rgba = Vec::new();
        rgba.try_reserve_exact(n).map_err(|_| crate::error::PsdError::OutOfMemory("not enough memory for the preview"))?;
        rgba.resize(n, 255u8);
        for y in 0..ht {
            let src_y = (y * k) as usize;
            for p in 0..color {
                let r = p * h.height as usize + src_y;
                let row = dec.rows(r, r + 1)?;
                for x in 0..w as usize {
                    let at = x * k as usize * bps;
                    let Some(s) = row.get(at..at + bps) else { continue };
                    let v = match *s {
                        [a] => a,
                        [a, _] => a,
                        // 32-bit samples are linear light: encoded for display (sRGB) first.
                        [a, b, c, d] => (srgb_encode(f32::from_be_bytes([a, b, c, d]).clamp(0.0, 1.0)) * 255.0).round() as u8,
                        _ => 0,
                    };
                    let px = (y as usize * w as usize + x) * 4;
                    if color == 1 {
                        rgba[px..px + 3].fill(v);
                    } else {
                        rgba[px + p] = v;
                    }
                }
            }
        }
        Ok(Some(Preview { width: w, height: ht, rgba }))
    }
}

/// The sRGB transfer of a linear value in `0..=1`.
fn srgb_encode(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
mod tests {
    use crate::source::Source;
    use crate::{Compression, PsdFile, Version, testgen};

    /// The preview of a file read from a source is every k-th pixel of the merged image's
    /// colour planes, opaque.
    #[test]
    fn preview_samples_the_merged_image() {
        for c in [Compression::Raw, Compression::Rle] {
            let file = testgen::small(Version::Psd, c);
            let bytes = file.to_bytes().unwrap();
            let lazy = PsdFile::from_source(Source::new(bytes)).unwrap();
            let planes = file.decode_merged().unwrap();
            let (w, h) = (file.header.width, file.header.height);
            for max in [2, 3, 1000] {
                let p = lazy.merged_preview(max).unwrap().unwrap();
                let k = w.max(h).div_ceil(max);
                assert_eq!((p.width, p.height), (w.div_ceil(k), h.div_ceil(k)));
                for y in 0..p.height {
                    for x in 0..p.width {
                        let b = ((y * p.width + x) * 4) as usize;
                        for ch in 0..3u32 {
                            let a = (ch * w * h + y * k * w + x * k) as usize;
                            assert_eq!(p.rgba[b + ch as usize], planes[a], "{c:?} {max} at {x},{y}");
                        }
                        assert_eq!(p.rgba[b + 3], 255);
                    }
                }
            }
        }
        assert!(testgen::small(Version::Psd, Compression::Zip).merged_preview(4).unwrap().is_none());
    }

    /// The preview size is the caller's: one over the decode limit is refused before allocating.
    #[test]
    fn a_preview_over_the_decode_limit_is_refused() {
        let mut file = testgen::small(Version::Psb, Compression::Raw);
        (file.header.width, file.header.height) = (300_000, 300_000);
        assert!(matches!(file.merged_preview(300_000), Err(crate::error::PsdError::LimitExceeded(_))));
    }

    /// Grayscale is replicated to RGB; 16-bit keeps each sample's high byte; 32-bit linear
    /// samples are sRGB-encoded (0.5 is 188).
    #[test]
    fn preview_converts_grayscale_and_deep_samples() {
        use crate::header::{ColorMode, Header};
        use crate::image_data::ImageData;
        let make = |mode: ColorMode, depth: u16, channels: u16, plane: Vec<u8>| {
            let mut file = testgen::small(Version::Psd, Compression::Raw);
            file.layer_info = None;
            file.global_blocks.clear();
            file.header = Header { channels, width: 2, height: 1, depth, color_mode: mode, ..file.header.clone() };
            let planes: Vec<u8> = (0..channels).flat_map(|_| plane.clone()).collect();
            file.image_data = ImageData::encode(Compression::Raw, &planes, &file.header).unwrap();
            let bytes = file.to_bytes().unwrap();
            PsdFile::from_source(Source::new(bytes)).unwrap().merged_preview(16).unwrap().unwrap()
        };
        let gray = make(ColorMode::Grayscale, 8, 1, vec![10, 200]);
        assert_eq!(gray.rgba, vec![10, 10, 10, 255, 200, 200, 200, 255]);
        let deep = make(ColorMode::Rgb, 16, 3, vec![0x12, 0x34, 0xAB, 0xCD]);
        assert_eq!(deep.rgba, vec![0x12, 0x12, 0x12, 255, 0xAB, 0xAB, 0xAB, 255]);
        let float: Vec<u8> = [0.5f32, 1.0].iter().flat_map(|v| v.to_be_bytes()).collect();
        let f = make(ColorMode::Rgb, 32, 3, float);
        assert_eq!(f.rgba, vec![188, 188, 188, 255, 255, 255, 255, 255]);
    }
}
