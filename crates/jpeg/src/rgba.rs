//! Conversion of decoded 8-bit JPEG samples to opaque RGBA.

/// The JPEG component count is not supported by the RGBA conversion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UnsupportedComponents(pub u8);

/// Convert grayscale, RGB or CMYK samples to opaque RGBA, ignoring incomplete trailing pixels.
pub fn to_rgba8(samples: &[u8], components: u8) -> Result<Vec<u8>, UnsupportedComponents> {
    let mut rgba = Vec::with_capacity(samples.len().saturating_mul(4));
    match components {
        1 => {
            for &gray in samples {
                rgba.extend_from_slice(&[gray, gray, gray, 255]);
            }
        }
        3 => {
            for rgb in samples.as_chunks::<3>().0 {
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        4 => {
            for cmyk in samples.as_chunks::<4>().0 {
                let (c, m, y, k) = (u16::from(cmyk[0]), u16::from(cmyk[1]), u16::from(cmyk[2]), u16::from(cmyk[3]));
                let rgb = [((255 - c) * (255 - k) / 255) as u8, ((255 - m) * (255 - k) / 255) as u8, ((255 - y) * (255 - k) / 255) as u8];
                rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        n => return Err(UnsupportedComponents(n)),
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gray_rgb_and_cmyk_conversion_matches_integer_rules() -> Result<(), UnsupportedComponents> {
        assert_eq!(to_rgba8(&[12], 1)?, [12, 12, 12, 255]);
        assert_eq!(to_rgba8(&[1, 2, 3, 4], 3)?, [1, 2, 3, 255]);
        assert_eq!(to_rgba8(&[0, 255, 128, 0], 4)?, [255, 0, 127, 255]);
        assert_eq!(to_rgba8(&[], 2), Err(UnsupportedComponents(2)));
        Ok(())
    }
}
