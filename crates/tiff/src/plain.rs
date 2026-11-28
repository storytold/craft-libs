//! The uncompressed, metadata-free compatibility representation for RGBA16 stills.

use std::io::Cursor;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid dimensions or RGBA16 sample count")]
    InvalidInput,
    #[error(transparent)]
    Tiff(#[from] tiff::TiffError),
}

/// Encode straight, native-endian RGBA16 samples as an uncompressed classic TIFF with the
/// encoder's default strip layout and no extra tags such as ICC or ExtraSamples.
pub fn encode_rgba16(width: u32, height: u32, samples: &[u16]) -> Result<Vec<u8>, Error> {
    let expected = u64::from(width).checked_mul(u64::from(height)).and_then(|n| n.checked_mul(4)).and_then(|n| usize::try_from(n).ok());
    if width == 0 || height == 0 || expected != Some(samples.len()) {
        return Err(Error::InvalidInput);
    }
    let mut out = Cursor::new(Vec::new());
    tiff::encoder::TiffEncoder::new(&mut out)?.write_image::<tiff::encoder::colortype::RGBA16>(width, height, samples)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_rgba16_keeps_uncompressed_legacy_strips_and_rgba_samples() {
        // Self-generated straight RGBA data; tall enough for the encoder's second strip.
        let (width, height) = (4, 31_253);
        let samples: Vec<u16> = std::iter::repeat_n([0x12ab, 0xcdef, 0x3142, 0x8000], (width * height) as usize).flatten().collect();
        let bytes = encode_rgba16(width, height, &samples).unwrap();
        let mut reader = tiff::decoder::Decoder::new(Cursor::new(bytes)).unwrap();
        use tiff::tags::Tag;
        assert_eq!(reader.dimensions().unwrap(), (width, height));
        assert_eq!(reader.get_tag_u32(Tag::Compression).unwrap(), 1);
        assert_eq!(reader.get_tag_u32_vec(Tag::StripByteCounts).unwrap(), [1_000_000, 96]);
        assert!(reader.find_tag(Tag::ExtraSamples).unwrap().is_none());
        assert!(reader.find_tag(Tag::IccProfile).unwrap().is_none());
        let tiff::decoder::DecodingResult::U16(decoded) = reader.read_image().unwrap() else {
            panic!("expected native 16-bit samples");
        };
        assert_eq!(decoded, samples);
    }

    #[test]
    fn plain_rgba16_rejects_malformed_sample_count_and_dimensions() {
        assert!(encode_rgba16(0, 1, &[]).is_err());
        assert!(encode_rgba16(1, 1, &[0; 3]).is_err());
        assert!(encode_rgba16(1, 1, &[0; 5]).is_err());
        assert!(encode_rgba16(u32::MAX, u32::MAX, &[]).is_err());
    }

    #[test]
    fn plain_rgba16_matches_legacy_writer_bytes() -> Result<(), Box<dyn std::error::Error>> {
        use image::{ExtendedColorType, ImageEncoder, codecs::tiff::TiffEncoder};

        // Original, generated pixels with distinct channels and alpha. Tall and wide cases
        // both exceed one strip and stress the compatibility encoder's strip decisions.
        for (w, h) in [(1, 1), (5, 3), (4, 31_253), (5_123, 25)] {
            let samples: Vec<_> = (0..w * h * 4).map(|i: u32| i.wrapping_mul(7919).wrapping_add(0x1234) as u16).collect();
            let mut legacy = Cursor::new(Vec::new());
            let packed: Vec<u8> = samples.iter().flat_map(|v| v.to_ne_bytes()).collect();
            TiffEncoder::new(&mut legacy).write_image(&packed, w, h, ExtendedColorType::Rgba16)?;
            assert!(encode_rgba16(w, h, &samples)? == legacy.into_inner(), "{w}×{h}: TIFF bytes changed");
        }
        Ok(())
    }
}
