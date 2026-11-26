/// IEEE half → f32 (FilmCraft `filmcraft_gpu::f16_to_f32`), for decoding `Rgba16Float` readbacks.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h & 0x8000) as u32) << 16;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x03ff) as u32;
    let bits = match exp {
        0 => {
            if mant == 0 {
                sign
            } else {
                let b = 31 - mant.leading_zeros();
                let frac = (mant ^ (1 << b)) << (23 - b);
                sign | ((103 + b) << 23) | frac
            }
        }
        0x1f => sign | 0x7f80_0000 | (mant << 13),
        _ => sign | ((exp + 112) << 23) | (mant << 13),
    };
    f32::from_bits(bits)
}

/// [`f16_to_f32`] with a fast path for normal numbers and zeros; subnormals and non-finite values use [`f16_to_f32`].
#[inline]
pub fn f16_to_f32_fast(h: u16) -> f32 {
    let u = u32::from(h);
    let sign = (u & 0x8000) << 16;
    let abs = u & 0x7fff;
    if (0x0400..0x7c00).contains(&abs) {
        f32::from_bits(sign | ((abs + (112 << 10)) << 13))
    } else if abs == 0 {
        f32::from_bits(sign)
    } else {
        f16_to_f32(h)
    }
}

/// Expand little-endian RGBA f16 texels to RGBA f32 (row-major, no padding).
pub fn f16_rgba_to_f32_bytes(src: &[u8]) -> Result<Vec<u8>, usize> {
    if !src.len().is_multiple_of(8) {
        return Err(src.len());
    }
    let mut out = Vec::with_capacity(src.len() * 2);
    for chunk in src.as_chunks::<8>().0 {
        for pair in chunk.as_chunks::<2>().0 {
            let v = f16_to_f32_fast(u16::from_le_bytes(*pair));
            out.extend_from_slice(&v.to_le_bytes());
        }
    }
    Ok(out)
}
