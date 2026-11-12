//! Bitmaps as the writers store them: bottom-up, uncompressed DIBs.

use super::Image;

const BI_RGB: u32 = 0;

/// A BITMAPINFOHEADER of a bottom-up, uncompressed `width` × `height` bitmap of `bits` per pixel.
fn header(width: u32, height: u32, bits: u16, size: usize) -> Vec<u8> {
    let mut h = super::Out::default();
    h.u32(40);
    h.i32(i32::try_from(width).unwrap_or(i32::MAX));
    h.i32(i32::try_from(height).unwrap_or(i32::MAX));
    h.u16(1);
    h.u16(bits);
    h.u32(BI_RGB);
    h.u32(u32::try_from(size).unwrap_or(0));
    // 2835 pixels a metre: 72 ppi (the size on the page comes from the record).
    h.i32(2835);
    h.i32(2835);
    h.u32(0);
    h.u32(0);
    h.0
}

/// The bytes of a row of `width` pixels of `bits` each, padded to 4 bytes.
fn stride(width: u32, bits: u32) -> Option<usize> {
    let row_bits = u64::from(width).checked_mul(u64::from(bits))?;
    usize::try_from(row_bits.div_ceil(32) * 4).ok()
}

/// `img` at `opacity`, composited over white, as a 24-bit DIB → (header, bits).
pub(crate) fn rgb24(img: &Image, opacity: f32) -> (Vec<u8>, Vec<u8>) {
    let row = stride(img.width, 24).unwrap_or(0);
    let mut bits = vec![0u8; row * img.height as usize];
    let over = |c: u8, a: u32| ((u32::from(c) * a + 255 * (255 - a) + 127) / 255) as u8;
    for (y, src) in img.rgba.chunks_exact((img.width as usize * 4).max(1)).enumerate() {
        // Bottom-up: the first stored row is the image's last.
        let Some(at) = (img.height as usize).checked_sub(y + 1).map(|r| r * row) else { continue };
        let Some(dst) = bits.get_mut(at..at + img.width as usize * 3) else { continue };
        for (d, p) in dst.as_chunks_mut::<3>().0.iter_mut().zip(src.as_chunks::<4>().0) {
            let a = (f32::from(p[3]) * opacity.clamp(0.0, 1.0)).round() as u32;
            d.copy_from_slice(&[over(p[2], a), over(p[1], a), over(p[0], a)]);
        }
    }
    (header(img.width, img.height, 24, bits.len()), bits)
}

/// `img` at `opacity` as a 32-bit DIB of premultiplied BGRA → (header, bits).
pub(crate) fn bgra32(img: &Image, opacity: f32) -> (Vec<u8>, Vec<u8>) {
    let row = img.width as usize * 4;
    let mut bits = vec![0u8; row * img.height as usize];
    for (y, src) in img.rgba.chunks_exact(row.max(1)).enumerate() {
        let Some(at) = (img.height as usize).checked_sub(y + 1).map(|r| r * row) else { continue };
        let Some(dst) = bits.get_mut(at..at + row) else { continue };
        for (d, p) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<4>().0) {
            let a = (f32::from(p[3]) * opacity.clamp(0.0, 1.0)).round() as u32;
            let pre = |c: u8| ((u32::from(c) * a + 127) / 255) as u8;
            d.copy_from_slice(&[pre(p[2]), pre(p[1]), pre(p[0]), a as u8]);
        }
    }
    (header(img.width, img.height, 32, bits.len()), bits)
}
