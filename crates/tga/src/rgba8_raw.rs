//! Uncompressed RGB(A) output from top-down RGBA8 pixels.
/// Why the supplied frame cannot be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rgba8EncodeError {
    /// Empty, overflowing, or mismatched dimensions.
    FrameSize,
    /// A dimension cannot fit the 16-bit Targa header.
    Dimensions,
    /// The output buffer could not be allocated.
    OutOfMemory,
}
fn checked_len(rgba: &[u8], w: u32, h: u32) -> std::result::Result<usize, Rgba8EncodeError> {
    let n = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4)).ok_or(Rgba8EncodeError::FrameSize)?;
    if w == 0 || h == 0 || rgba.len() != n {
        return Err(Rgba8EncodeError::FrameSize);
    }
    Ok(n)
}
/// An uncompressed Targa file: 24-bit BGR, or 32-bit BGRA with `alpha` (straight alpha, as the
/// frame carries it).
pub fn encode_rgba8_raw(rgba: &[u8], w: u32, h: u32, alpha: bool) -> std::result::Result<Vec<u8>, Rgba8EncodeError> {
    checked_len(rgba, w, h)?;
    let (tw, th) = match (u16::try_from(w), u16::try_from(h)) {
        (Ok(tw), Ok(th)) => (tw, th),
        _ => return Err(Rgba8EncodeError::Dimensions),
    };
    let bpp: usize = if alpha { 4 } else { 3 };
    let row = w as usize * 4;
    // The caller already holds the frame; reserve the second, output-sized buffer fallibly so a
    // large export reports an error instead of aborting the process.
    let len = (w as usize * h as usize).checked_mul(bpp).and_then(|n| n.checked_add(18 + 26)).ok_or(Rgba8EncodeError::FrameSize)?;
    let mut out = Vec::new();
    out.try_reserve_exact(len).map_err(|_| Rgba8EncodeError::OutOfMemory)?;
    out.extend_from_slice(&[0, 0, 2]); // no ID, no colour map, uncompressed true-colour
    out.extend_from_slice(&[0; 5]); // colour map specification (unused)
    out.extend_from_slice(&[0, 0, 0, 0]); // x / y origin
    out.extend_from_slice(&tw.to_le_bytes());
    out.extend_from_slice(&th.to_le_bytes());
    out.push(if alpha { 32 } else { 24 });
    out.push(if alpha { 8 } else { 0 }); // attribute bits; bottom-left origin
    // bottom-left origin: the last frame row comes first
    for line in rgba.chunks_exact(row).rev() {
        for &[r, g, b, a] in line.as_chunks::<4>().0 {
            out.extend_from_slice(&[b, g, r]);
            if alpha {
                out.push(a);
            }
        }
    }
    // TGA 2.0 footer without extension or developer areas
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(b"TRUEVISION-XFILE.\0");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn raw_round_trip() {
        let bytes = encode_rgba8_raw(&[1, 2, 3, 4], 1, 1, true).unwrap();
        assert_eq!(&bytes[18..22], &[3, 2, 1, 4]);
        assert_eq!(&bytes[bytes.len() - 18..], b"TRUEVISION-XFILE.\0");
        assert_eq!(encode_rgba8_raw(&[], 0, 1, false), Err(Rgba8EncodeError::FrameSize));
    }
}
