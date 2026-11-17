//! A BMP's resolution: `biXPelsPerMeter` and `biYPelsPerMeter`, the pixels per metre of the
//! BITMAPINFOHEADER and every later header (V4, V5, OS/2 2.x), at file offsets 38 and 42. The
//! 12-byte BITMAPCOREHEADER of OS/2 1.x has no such fields. The `image` crate reads and writes
//! the pixels only, and writes 0 here, which readers take as "no resolution".

use super::png::dpi_of_ppm;

/// Bytes before the fields: the 14-byte BITMAPFILEHEADER, then the DIB header's size, width,
/// height, planes, bit count, compression and image size.
const X_PPM: usize = 38;
const Y_PPM: usize = 42;
/// The shortest DIB header that carries the resolution (BITMAPINFOHEADER).
const INFO_HEADER: u32 = 40;
const METERS_PER_INCH: f64 = 0.0254;

/// The little-endian 4 bytes at `at`, or `None` past the end of `bytes`.
fn le4(bytes: &[u8], at: usize) -> Option<[u8; 4]> {
    bytes.get(at..at.checked_add(4)?)?.try_into().ok()
}

/// Whether `bytes` start a BMP whose header has the resolution fields.
fn has_resolution_fields(bytes: &[u8]) -> bool {
    bytes.starts_with(b"BM") && le4(bytes, Y_PPM).is_some() && le4(bytes, 14).map(u32::from_le_bytes).is_some_and(|dib| dib >= INFO_HEADER)
}

/// The resolution in pixels per inch a BMP records, or `None` when its header has no fields for it
/// or either is 0 or negative (most writers leave them 0). A whole number of ppi stored rounded to
/// whole pixels per metre (300 ppi = 11811) reads back whole, as a PNG's `pHYs` does.
pub(crate) fn resolution(bytes: &[u8]) -> Option<(f32, f32)> {
    if !has_resolution_fields(bytes) {
        return None;
    }
    let field = |at: usize| le4(bytes, at).map(i32::from_le_bytes).and_then(|v| u32::try_from(v).ok());
    let (x, y) = (field(X_PPM)?, field(Y_PPM)?);
    (x > 0 && y > 0).then(|| (dpi_of_ppm(x), dpi_of_ppm(y)))
}

/// Pixels per metre for `ppi`, rounded as Photoshop and the PNG `pHYs` writer do (300 ppi = 11811),
/// or `None` for a resolution that has no positive whole number of pixels per metre.
fn ppm_of_dpi(ppi: f32) -> Option<i32> {
    let ppm = (f64::from(ppi) / METERS_PER_INCH).round();
    (ppm.is_finite() && ppm >= 1.0).then(|| ppm.min(f64::from(i32::MAX)) as i32)
}

/// Writes `dpi` into the resolution fields of the BMP in `bytes`. A header without the fields, or
/// a resolution that cannot be stored, leaves the bytes as they are.
pub(crate) fn set_resolution(bytes: &mut [u8], dpi: Option<(f32, f32)>) {
    let Some((x, y)) = dpi.and_then(|(x, y)| Some((ppm_of_dpi(x)?, ppm_of_dpi(y)?))) else { return };
    if !has_resolution_fields(bytes) {
        return;
    }
    for (at, ppm) in [(X_PPM, x), (Y_PPM, y)] {
        if let Some(field) = bytes.get_mut(at..at + 4) {
            field.copy_from_slice(&ppm.to_le_bytes());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 46-byte BMP start: file header, a DIB header of `dib` bytes, 11811 px/m each way.
    fn start(dib: u32) -> Vec<u8> {
        let mut b = vec![0u8; Y_PPM + 4];
        b[..2].copy_from_slice(b"BM");
        b[14..18].copy_from_slice(&dib.to_le_bytes());
        b[X_PPM..X_PPM + 4].copy_from_slice(&11811i32.to_le_bytes());
        b[Y_PPM..Y_PPM + 4].copy_from_slice(&11811i32.to_le_bytes());
        b
    }

    #[test]
    fn only_headers_with_the_fields_are_read_or_written() {
        assert_eq!(resolution(&start(40)), Some((300.0, 300.0)));
        assert_eq!(resolution(&start(124)), Some((300.0, 300.0)), "BITMAPV5HEADER");
        // OS/2 1.x's 12-byte core header has no resolution: those bytes are pixels or palette.
        let mut core = start(12);
        assert_eq!(resolution(&core), None);
        set_resolution(&mut core, Some((72.0, 72.0)));
        assert_eq!(core, start(12), "left as it is");
        // Too short, or not a BMP: nothing read, nothing written, no panic.
        let mut short = start(40)[..Y_PPM + 3].to_vec();
        assert_eq!(resolution(&short), None);
        set_resolution(&mut short, Some((72.0, 72.0)));
        assert_eq!(short, start(40)[..Y_PPM + 3]);
        let mut other = start(40);
        other[0] = b'X';
        assert_eq!(resolution(&other), None);
        set_resolution(&mut other, Some((72.0, 72.0)));
        assert_eq!(&other[X_PPM..X_PPM + 4], &11811i32.to_le_bytes(), "not a BMP: left as it is");
    }

    #[test]
    fn resolutions_without_a_whole_pixel_per_metre_are_not_written() {
        for bad in [0.0, -300.0, 0.01, f32::NAN, f32::INFINITY] {
            let mut b = start(40);
            set_resolution(&mut b, Some((bad, 300.0)));
            assert_eq!(b, start(40), "{bad}");
        }
        // A huge one is capped at the field's maximum rather than wrapping negative.
        let mut b = start(40);
        set_resolution(&mut b, Some((1e12, 300.0)));
        assert_eq!(&b[X_PPM..X_PPM + 4], &i32::MAX.to_le_bytes());
    }
}
