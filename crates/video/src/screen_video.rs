//! Screen Video (FLV codec 3 / SWF SCREENVIDEOPACKET) key-frame decoding.
//! The packet excludes the FLV video-tag codec byte. Blocks and rows are bottom-up;
//! output is straight RGBA8 in top-down row order. No inter-frame state or container I/O.

/// One decoded top-down RGBA8 picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Four bytes per pixel, alpha always 255.
    pub rgba: Vec<u8>,
}

/// Decodes a complete key frame within caller axis and RGBA allocation limits.
/// Returns `None` for malformed/truncated data, unchanged blocks or exceeded limits.
/// Format dimensions are at most 4095 per axis; each block inflation is capped to
/// exactly its expected BGR byte count. Legacy trailing packet bytes are tolerated.
pub fn decode_keyframe(p: &[u8], max_edge: u32, max_rgba_bytes: usize) -> Option<Frame> {
    let h0 = u16::from_be_bytes([*p.first()?, *p.get(1)?]);
    let h1 = u16::from_be_bytes([*p.get(2)?, *p.get(3)?]);
    let (bw, w) = ((usize::from(h0 >> 12) + 1) * 16, usize::from(h0 & 0x0fff));
    let (bh, h) = ((usize::from(h1 >> 12) + 1) * 16, usize::from(h1 & 0x0fff));
    if w == 0 || h == 0 || w > max_edge as usize || h > max_edge as usize {
        return None;
    }
    let bytes = w.checked_mul(h)?.checked_mul(4)?;
    if bytes > max_rgba_bytes {
        return None;
    }
    let mut rgba = vec![0u8; bytes];
    let mut at = 4usize;
    // Block rows from the bottom of the picture.
    for by in 0..h.div_ceil(bh) {
        for bx in 0..w.div_ceil(bw) {
            let (x0, y0) = (bx * bw, by * bh);
            let (cw, ch) = (bw.min(w - x0), bh.min(h - y0));
            let size = usize::from(u16::from_be_bytes([*p.get(at)?, *p.get(at + 1)?]));
            let packed = p.get(at + 2..at + 2 + size)?;
            at += 2 + size;
            if size == 0 {
                // An unchanged block can't be in a key frame.
                return None;
            }
            let want = cw * ch * 3;
            let bgr = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(packed, want).ok()?;
            if bgr.len() != want {
                return None;
            }
            for (row, line) in bgr.chunks_exact(cw * 3).enumerate() {
                // Rows count up from the bottom; the bitmap's rows count down from the top.
                let y = h - 1 - (y0 + row);
                for (col, px) in line.as_chunks::<3>().0.iter().enumerate() {
                    let i = (y * w + x0 + col) * 4;
                    rgba.get_mut(i..i + 4)?.copy_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
        }
    }
    Some(Frame { width: w as u32, height: h as u32, rgba })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    fn packet(w: usize, h: usize, block: usize, px: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut packet = Vec::new();
        let n = (block / 16 - 1) as u16;
        packet.extend_from_slice(&((n << 12) | w as u16).to_be_bytes());
        packet.extend_from_slice(&((n << 12) | h as u16).to_be_bytes());
        for by in 0..h.div_ceil(block) {
            for bx in 0..w.div_ceil(block) {
                let (x0, y0) = (bx * block, by * block);
                let mut bgr = Vec::new();
                for row in 0..block.min(h - y0) {
                    for col in 0..block.min(w - x0) {
                        let [r, g, b] = px(x0 + col, h - 1 - (y0 + row));
                        bgr.extend_from_slice(&[b, g, r]);
                    }
                }
                let z = miniz_oxide::deflate::compress_to_vec_zlib(&bgr, 6);
                packet.extend_from_slice(&(z.len() as u16).to_be_bytes());
                packet.extend_from_slice(&z);
            }
        }
        packet
    }
    #[test]
    fn a_screen_video_key_frame_decodes_with_its_blocks_in_place() {
        // 40 × 20 in 16-pixel blocks: partial blocks on the right and at the top.
        let px = |x: usize, y: usize| [(x * 6) as u8, (y * 12) as u8, if x < 20 { 255 } else { 0 }];
        let Frame { width, height, rgba } = decode_keyframe(&packet(40, 20, 16, px), 8192, 64 << 20).unwrap();
        assert_eq!((width, height), (40, 20));
        for (x, y) in [(0, 0), (39, 0), (0, 19), (39, 19), (17, 3), (21, 18)] {
            let i = (y * 40 + x) * 4;
            let [r, g, b] = px(x, y);
            assert_eq!(&rgba[i..i + 4], &[r, g, b, 255], "({x}, {y})");
        }
    }
    #[test]
    fn limits_truncation_and_unchanged_blocks_are_refused() {
        let p = packet(40, 20, 16, |x, y| [x as u8, y as u8, 7]);
        assert!(decode_keyframe(&p, 40, 40 * 20 * 4).is_some());
        assert!(decode_keyframe(&p, 39, usize::MAX).is_none());
        assert!(decode_keyframe(&p, 40, 40 * 20 * 4 - 1).is_none());
        for cut in 0..p.len() {
            let _ = decode_keyframe(&p[..cut], 40, 40 * 20 * 4);
        }
        let mut unchanged = p.clone();
        unchanged[4..6].fill(0);
        assert!(decode_keyframe(&unchanged, 40, 40 * 20 * 4).is_none());
        assert!(decode_keyframe(&[0, 0, 0, 0], 40, usize::MAX).is_none());
        let mut trailing = p.clone();
        trailing.extend_from_slice(&[1, 2, 3]);
        assert_eq!(decode_keyframe(&trailing, 40, 3200), decode_keyframe(&p, 40, 3200));
    }
    #[test]
    fn inflated_block_must_have_exactly_its_expected_pixel_bytes() {
        for length in [16 * 16 * 3 - 1, 16 * 16 * 3, 16 * 16 * 3 + 1] {
            let packed = miniz_oxide::deflate::compress_to_vec_zlib(&vec![7; length], 6);
            let mut p = vec![0, 16, 0, 16];
            p.extend_from_slice(&(packed.len() as u16).to_be_bytes());
            p.extend_from_slice(&packed);
            assert_eq!(decode_keyframe(&p, 16, 1024).is_some(), length == 16 * 16 * 3);
            assert!(decode_keyframe(&p[..6], 16, 1024).is_none());
        }
    }
}
