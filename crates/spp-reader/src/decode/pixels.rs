//! [`SourceFormat`]: the reference app's stored pixel formats (texture-cache GL formats and image-resource formats) and how to
//! read one pixel of them.
//!
//! Texture-cache pixels are premultiplied by coverage (the last component); colour (sRGB) entries are premultiplied in
//! linear light before sRGB encoding (Verified on partially covered corpus pixels). Image resources are straight.
//! Undoing the premultiplication is colour maths and is left to the importer ([`Alpha`] says which kind applies).

use super::image::PixelFormat as ImageFormat;

/// GL internal formats found in the texture cache.
const GL_SRGB8_ALPHA8: u32 = 0x8C43;
const GL_RG8: u32 = 0x822B;
const GL_RG16: u32 = 0x822C;
const GL_RG16F: u32 = 0x822F;
const GL_RGBA16F: u32 = 0x881A;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// Cache: sRGB colour + coverage, premultiplied in linear light.
    CacheSrgb8Alpha8,
    /// Cache: value + coverage, premultiplied.
    CacheRg8,
    /// Cache of a 16-bit (L16) channel: value + coverage, premultiplied.
    CacheRg16,
    CacheRg16F,
    /// Cache: encoded vector + coverage, premultiplied (normals).
    CacheRgba16F,
    Bgra8,
    Rgba8,
    Rgba16,
    Rgba16F,
    Rgba32F,
    Lum8,
    Lum16,
}

/// How the colour components relate to the last (coverage) component.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alpha {
    Straight,
    Premultiplied,
    /// Premultiplied in linear light, then sRGB-encoded.
    PremultipliedLinearSrgb,
}

/// Order of the rows in a stored buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowOrder {
    TopFirst,
    /// OpenGL texture order (the texture cache): the first row is the bottom of the image.
    BottomFirst,
}

impl RowOrder {
    /// The stored row that holds image row `y` (counted from the top) of an image `height` rows high.
    pub fn stored_row(self, y: usize, height: usize) -> usize {
        match self {
            RowOrder::TopFirst => y,
            RowOrder::BottomFirst => height.saturating_sub(1).saturating_sub(y),
        }
    }
}

impl SourceFormat {
    pub fn from_gl(internal_format: u32) -> Option<SourceFormat> {
        Some(match internal_format {
            GL_SRGB8_ALPHA8 => SourceFormat::CacheSrgb8Alpha8,
            GL_RG8 => SourceFormat::CacheRg8,
            GL_RG16 => SourceFormat::CacheRg16,
            GL_RG16F => SourceFormat::CacheRg16F,
            GL_RGBA16F => SourceFormat::CacheRgba16F,
            _ => return None,
        })
    }

    pub fn from_image(format: &ImageFormat) -> Option<SourceFormat> {
        Some(match format {
            ImageFormat::Bgra8 => SourceFormat::Bgra8,
            ImageFormat::Rgba8 => SourceFormat::Rgba8,
            ImageFormat::Rgba16 => SourceFormat::Rgba16,
            ImageFormat::Rgba16F => SourceFormat::Rgba16F,
            ImageFormat::Rgba32F => SourceFormat::Rgba32F,
            ImageFormat::Lum8 => SourceFormat::Lum8,
            ImageFormat::Lum16 => SourceFormat::Lum16,
            ImageFormat::Other(_) => return None,
        })
    }

    pub fn bytes_per_pixel(self) -> usize {
        match self {
            SourceFormat::Lum8 => 1,
            SourceFormat::CacheRg8 | SourceFormat::Lum16 => 2,
            SourceFormat::CacheSrgb8Alpha8 | SourceFormat::CacheRg16 | SourceFormat::CacheRg16F | SourceFormat::Bgra8 | SourceFormat::Rgba8 => 4,
            SourceFormat::CacheRgba16F | SourceFormat::Rgba16 | SourceFormat::Rgba16F => 8,
            SourceFormat::Rgba32F => 16,
        }
    }

    pub fn alpha(self) -> Alpha {
        match self {
            SourceFormat::CacheSrgb8Alpha8 => Alpha::PremultipliedLinearSrgb,
            SourceFormat::CacheRg8 | SourceFormat::CacheRg16 | SourceFormat::CacheRg16F | SourceFormat::CacheRgba16F => Alpha::Premultiplied,
            _ => Alpha::Straight,
        }
    }

    /// One pixel as RGBA, as stored (gray formats in R, G and B; coverage or alpha in A). `b` holds one pixel; bytes it
    /// lacks read as zero.
    pub fn decode(self, b: &[u8]) -> [f32; 4] {
        let byte = |i: usize| b.get(i).copied().unwrap_or(0);
        let u8n = |i: usize| f32::from(byte(i)) / 255.0;
        let u16_at = |i: usize| u16::from_le_bytes([byte(2 * i), byte(2 * i + 1)]);
        let u16n = |i: usize| f32::from(u16_at(i)) / 65535.0;
        let f16 = |i: usize| f16_to_f32(u16_at(i));
        let f32n = |i: usize| f32::from_le_bytes([byte(4 * i), byte(4 * i + 1), byte(4 * i + 2), byte(4 * i + 3)]);
        match self {
            SourceFormat::CacheSrgb8Alpha8 | SourceFormat::Rgba8 => [u8n(0), u8n(1), u8n(2), u8n(3)],
            SourceFormat::Bgra8 => [u8n(2), u8n(1), u8n(0), u8n(3)],
            SourceFormat::CacheRg8 => [u8n(0), u8n(0), u8n(0), u8n(1)],
            SourceFormat::CacheRg16 => [u16n(0), u16n(0), u16n(0), u16n(1)],
            SourceFormat::CacheRg16F => [f16(0), f16(0), f16(0), f16(1)],
            SourceFormat::CacheRgba16F | SourceFormat::Rgba16F => [f16(0), f16(1), f16(2), f16(3)],
            SourceFormat::Rgba16 => [u16n(0), u16n(1), u16n(2), u16n(3)],
            SourceFormat::Rgba32F => [f32n(0), f32n(1), f32n(2), f32n(3)],
            SourceFormat::Lum8 => [u8n(0), u8n(0), u8n(0), 1.0],
            SourceFormat::Lum16 => [u16n(0), u16n(0), u16n(0), 1.0],
        }
    }

    /// Pixel `index` of a buffer in this format, as stored. Pixels past the end of `raw`, or the missing tail of a pixel
    /// it cuts short (a short LZ4 block), read as zero.
    pub fn decode_at(self, raw: &[u8], index: usize) -> [f32; 4] {
        let bpp = self.bytes_per_pixel();
        let at = index.saturating_mul(bpp);
        let end = at.saturating_add(bpp).min(raw.len());
        self.decode(raw.get(at..end).unwrap_or(&[]))
    }
}

/// IEEE half to single precision; exact, as every half value is a single one.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = u32::from(h >> 15) << 31;
    let exp = u32::from((h >> 10) & 0x1f);
    let man = u32::from(h & 0x3ff);
    let bits = match (exp, man) {
        (0, 0) => sign,
        (0, _) => {
            // subnormal: normalise the mantissa
            let shift = man.leading_zeros() - 21;
            let man = (man << shift) & 0x3ff;
            sign | ((113 - shift) << 23) | (man << 13)
        }
        (0x1f, 0) => sign | 0x7f80_0000,
        (0x1f, _) => sign | 0x7fc0_0000 | (man << 13),
        _ => sign | ((exp + 112) << 23) | (man << 13),
    };
    f32::from_bits(bits)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_swizzles() {
        assert_eq!(SourceFormat::Bgra8.decode(&[0, 0, 255, 255]), [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(SourceFormat::CacheRg8.decode(&[255, 0]), [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(SourceFormat::Lum16.decode(&[0xff, 0xff])[0], 1.0);
        assert_eq!(SourceFormat::from_gl(0x822F), Some(SourceFormat::CacheRg16F));
        assert_eq!(SourceFormat::from_gl(0x822C), Some(SourceFormat::CacheRg16));
        assert_eq!(SourceFormat::CacheRg16.decode(&[0xff, 0xff, 0, 0]), [1.0, 1.0, 1.0, 0.0]);
        assert_eq!(SourceFormat::CacheRg16.alpha(), Alpha::Premultiplied);
        assert_eq!(SourceFormat::from_gl(1), None);
        assert_eq!(SourceFormat::from_image(&ImageFormat::Lum8), Some(SourceFormat::Lum8));
        for f in [SourceFormat::Rgba32F, SourceFormat::CacheRgba16F, SourceFormat::Lum8, SourceFormat::Rgba16] {
            let zero = vec![0u8; f.bytes_per_pixel()];
            assert_eq!(f.decode(&zero)[0], 0.0);
        }
    }

    #[test]
    fn short_and_missing_bytes_read_as_zero() {
        // a pixel cut short never panics: what is there counts, the rest is zero
        assert_eq!(SourceFormat::Rgba32F.decode(&[]), [0.0; 4]);
        assert_eq!(SourceFormat::Rgba8.decode(&[255]), [1.0, 0.0, 0.0, 0.0]);
        let raw = [0xff, 0xff, 0xff];
        assert_eq!(SourceFormat::Lum16.decode_at(&raw, 0)[0], 1.0);
        assert_eq!(SourceFormat::Lum16.decode_at(&raw, 1)[0], 255.0 / 65535.0);
        assert_eq!(SourceFormat::Lum16.decode_at(&raw, 2)[0], 0.0);
        assert_eq!(SourceFormat::Lum16.decode_at(&raw, usize::MAX)[0], 0.0);
    }

    #[test]
    fn rows_and_halves() {
        assert_eq!(RowOrder::BottomFirst.stored_row(0, 4), 3);
        assert_eq!(RowOrder::TopFirst.stored_row(1, 4), 1);
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24));
        assert!(f16_to_f32(0x7e00).is_nan());
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
    }
}
