//! Uncompressed RGB(A) output from top-down RGBA8 pixels.
/// Why the supplied frame cannot be encoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rgba8EncodeError {
    /// Empty, overflowing, or mismatched dimensions.
    FrameSize,
    /// A dimension cannot fit the output format.
    TooLarge,
    /// The output buffer could not be allocated.
    OutOfMemory,
}
/// Reserves the DPX output up front; a failed reservation becomes [`Rgba8EncodeError::OutOfMemory`].
fn output_buffer(size: usize) -> std::result::Result<Vec<u8>, Rgba8EncodeError> {
    let mut buf = Vec::new();
    match buf.try_reserve_exact(size) {
        Ok(()) => Ok(buf),
        Err(_) => Err(Rgba8EncodeError::OutOfMemory),
    }
}
fn checked_len(rgba: &[u8], w: u32, h: u32) -> std::result::Result<usize, Rgba8EncodeError> {
    let n = (w as usize).checked_mul(h as usize).and_then(|n| n.checked_mul(4)).ok_or(Rgba8EncodeError::FrameSize)?;
    if w == 0 || h == 0 || rgba.len() != n {
        return Err(Rgba8EncodeError::FrameSize);
    }
    Ok(n)
}
/// Header size of the DPX files we write (file + image + orientation + film + TV headers).
const DPX_HEADER: usize = 2048;

/// A DPX file: 10-bit RGB (8-bit values widened by bit replication, so 0 → 0 and 255 → 1023).
pub fn encode_rgba8_dpx10(rgba: &[u8], w: u32, h: u32, creator: &str) -> std::result::Result<Vec<u8>, Rgba8EncodeError> {
    let n = checked_len(rgba, w, h)?;
    // one 32-bit word per pixel; `n` is already `w * h * 4`
    let size = n.checked_add(DPX_HEADER).ok_or(Rgba8EncodeError::FrameSize)?;
    let file_size = u32::try_from(size).map_err(|_| Rgba8EncodeError::TooLarge)?;
    // SMPTE ST 268: undefined numeric fields are all ones, undefined text fields are NUL
    let mut hd = vec![0xFF_u8; DPX_HEADER];
    for (a, b) in [
        (36, 660),    // file name, creation time, creator, project, copyright
        (664, 768),   // reserved
        (1356, 1408), // reserved
        (1432, 1620), // source file name, time, input device, serial number
        (1644, 1664), // reserved
        (1664, 1712), // film manufacturer, type, offset, prefix, count, format
        (1732, 1920), // frame id, slate, reserved
        (1972, 2048), // reserved
    ] {
        hd[a..b].fill(0);
    }
    for e in 0..8 {
        let d = 780 + e * 72 + 40;
        hd[d..d + 32].fill(0); // element descriptions
    }
    let put32 = |hd: &mut Vec<u8>, at: usize, v: u32| hd[at..at + 4].copy_from_slice(&v.to_be_bytes());
    let put16 = |hd: &mut Vec<u8>, at: usize, v: u16| hd[at..at + 2].copy_from_slice(&v.to_be_bytes());
    // file information header
    hd[0..4].copy_from_slice(b"SDPX");
    put32(&mut hd, 4, DPX_HEADER as u32); // offset to the image data
    hd[8..16].copy_from_slice(b"V2.0\0\0\0\0");
    put32(&mut hd, 16, file_size);
    put32(&mut hd, 20, 1); // ditto key: new frame
    put32(&mut hd, 24, 1664); // generic header length
    put32(&mut hd, 28, 384); // industry header length
    put32(&mut hd, 32, 0); // user data length
    let creator = creator.as_bytes();
    let n = creator.len().min(100);
    hd[160..160 + n].copy_from_slice(&creator[..n]);
    // image information header
    put16(&mut hd, 768, 0); // orientation: left to right, top to bottom
    put16(&mut hd, 770, 1); // one image element
    put32(&mut hd, 772, w);
    put32(&mut hd, 776, h);
    let el = 780;
    put32(&mut hd, el, 0); // unsigned
    put32(&mut hd, el + 4, 0); // reference low data code
    put32(&mut hd, el + 12, 1023); // reference high data code
    hd[el + 20] = 50; // descriptor: RGB
    hd[el + 21] = 6; // transfer: ITU-R BT.709
    hd[el + 22] = 6; // colorimetric: ITU-R BT.709
    hd[el + 23] = 10; // bit depth
    put16(&mut hd, el + 24, 1); // packing: filled to 32-bit words, method A
    put16(&mut hd, el + 26, 0); // no encoding (uncompressed)
    put32(&mut hd, el + 28, DPX_HEADER as u32); // offset to the element's data
    put32(&mut hd, el + 32, 0); // end-of-line padding
    put32(&mut hd, el + 36, 0); // end-of-image padding
    // image orientation header: no offset, square pixels
    put32(&mut hd, 1408, 0);
    put32(&mut hd, 1412, 0);
    put32(&mut hd, 1424, w);
    put32(&mut hd, 1428, h);
    put32(&mut hd, 1628, 1);
    put32(&mut hd, 1632, 1);
    // television header: progressive
    hd[1928] = 0;
    // The caller already holds the frame; reserve the output fallibly so a large export reports
    // an error instead of aborting the process.
    let mut out = output_buffer(size)?;
    out.extend_from_slice(&hd);
    let ten = |v: u8| (u32::from(v) << 2) | (u32::from(v) >> 6);
    for &[r, g, b, _] in rgba.as_chunks::<4>().0 {
        out.extend_from_slice(&((ten(r) << 22) | (ten(g) << 12) | (ten(b) << 2)).to_be_bytes());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packed_rgb_and_creator() {
        let bytes = encode_rgba8_dpx10(&[255, 0, 128, 255], 1, 1, "FilmCraft").unwrap();
        assert_eq!(&bytes[..4], b"SDPX");
        assert_eq!(&bytes[160..169], b"FilmCraft");
        assert_eq!(&bytes[2048..], &((1023u32 << 22) | (514u32 << 2)).to_be_bytes());
        assert_eq!(encode_rgba8_dpx10(&[], 0, 1, ""), Err(Rgba8EncodeError::FrameSize));
    }
    #[test]
    fn output_reservation_failure_maps_to_out_of_memory() {
        assert!(matches!(output_buffer(usize::MAX), Err(Rgba8EncodeError::OutOfMemory)));
        assert_eq!(output_buffer(DPX_HEADER).map(|b| b.capacity() >= DPX_HEADER), Ok(true));
    }
}
