//! Packed and planar uncompressed AVI and QuickTime sample decoding.

/// A malformed uncompressed frame or an unsupported image size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    UnsupportedDimensions,
    Truncated,
}
/// Decoded top-down RGBA pixels or planar YUV samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RawPixels {
    Rgba(Vec<u8>),
    Yuv420([Vec<u8>; 3]),
    Yuv422([Vec<u8>; 3]),
}

/// Uncompressed video layouts (AVI `BI_RGB` and YUV FourCCs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RawFormat {
    /// 24-bit B, G, R; rows padded to 4 bytes.
    Bgr24,
    /// 32-bit B, G, R, X.
    Bgrx32,
    /// Packed 4:2:2: Y0 U Y1 V (`YUY2`, `YUYV`).
    Yuy2,
    /// Packed 4:2:2: U Y0 V Y1 (`UYVY`, `2vuy`).
    Uyvy,
    /// Planar 4:2:0: Y, U, V (`I420`, `IYUV`).
    I420,
    /// Planar 4:2:0: Y, V, U (`YV12`).
    Yv12,
    /// Y plane, interleaved U V at half resolution (`NV12`).
    Nv12,
    /// Luma only (`Y800`, `GREY`, `Y8  `).
    Gray8,
    /// 24-bit R, G, B; rows padded to an even byte count (QuickTime `raw ` at depth 24). The
    /// last row's pad byte may be missing.
    Rgb24,
    /// 10-bit 4:2:2 Y'CbCr, six pixels in four little-endian words, rows padded to 128 bytes
    /// (QuickTime `v210`); shown at 8 bits.
    V210,
}

impl RawFormat {
    /// The layout of an AVI video stream: `BI_RGB` with its bit count, or a YUV FourCC.
    /// `BI_BITFIELDS` needs its colour masks, so it goes through [`Self::from_avi_bitfields`].
    pub fn from_avi(compression: [u8; 4], bit_count: u16) -> Option<RawFormat> {
        if u32::from_le_bytes(compression) == 0 {
            return match bit_count {
                24 => Some(RawFormat::Bgr24),
                32 => Some(RawFormat::Bgrx32),
                _ => None,
            };
        }
        let mut upper = compression;
        upper.make_ascii_uppercase();
        Some(match &upper {
            b"YUY2" | b"YUYV" | b"YUNV" | b"V422" => RawFormat::Yuy2,
            b"UYVY" | b"2VUY" | b"UYNV" | b"HDYC" => RawFormat::Uyvy,
            b"I420" | b"IYUV" => RawFormat::I420,
            b"YV12" => RawFormat::Yv12,
            b"NV12" => RawFormat::Nv12,
            b"Y800" | b"GREY" | b"GRAY" | b"Y8  " => RawFormat::Gray8,
            _ => return None,
        })
    }

    /// The layout of an AVI `BI_BITFIELDS` stream: only 32-bit with the masks that make it B, G, R, X
    /// (R `0x00ff_0000`, G `0x0000_ff00`, B `0x0000_00ff`). Other masks are a different byte order.
    pub fn from_avi_bitfields(bit_count: u16, [r, g, b]: [u32; 3]) -> Option<RawFormat> {
        (bit_count == 32 && (r, g, b) == (0x00ff_0000, 0x0000_ff00, 0x0000_00ff)).then_some(RawFormat::Bgrx32)
    }

    /// The layout of a QuickTime / MP4 video sample entry (`raw ` RGB at depth 24, `2vuy`, `v210`).
    pub fn from_mov(fourcc: [u8; 4], depth: u16) -> Option<RawFormat> {
        match (&fourcc, depth) {
            (b"raw ", 24) => Some(RawFormat::Rgb24),
            (b"2vuy", _) => Some(RawFormat::Uyvy),
            (b"v210", _) => Some(RawFormat::V210),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RawFormat::Bgr24 | RawFormat::Rgb24 => "Uncompressed RGB 24-bit",
            RawFormat::Bgrx32 => "Uncompressed RGB 32-bit",
            RawFormat::Yuy2 | RawFormat::Uyvy => "Uncompressed YUV 4:2:2",
            RawFormat::I420 | RawFormat::Yv12 | RawFormat::Nv12 => "Uncompressed YUV 4:2:0",
            RawFormat::Gray8 => "Uncompressed greyscale",
            RawFormat::V210 => "Uncompressed YUV 4:2:2 10-bit",
        }
    }
}

pub fn decode(format: RawFormat, width: u32, height: u32, bottom_up: bool, s: &[u8]) -> Result<RawPixels, Error> {
    if width == 0 || height == 0 || width > 16_384 || height > 16_384 {
        return Err(Error::UnsupportedDimensions);
    }
    let (w, h) = (width as usize, height as usize);
    let short = || Error::Truncated;
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    Ok(match format {
        RawFormat::Bgr24 | RawFormat::Bgrx32 => {
            let bpp = if format == RawFormat::Bgr24 { 3 } else { 4 };
            let stride = (w * bpp).div_ceil(4) * 4;
            if s.len() < stride * h {
                return Err(short());
            }
            let mut rgba = Vec::with_capacity(w * h * 4);
            for row in 0..h {
                let src = if bottom_up { h - 1 - row } else { row };
                for px in s.get(src * stride..src * stride + w * bpp).unwrap_or_default().chunks_exact(bpp) {
                    rgba.extend_from_slice(&[px[2], px[1], px[0], 255]);
                }
            }
            RawPixels::Rgba(rgba)
        }
        RawFormat::Rgb24 => {
            // QuickTime starts each scanline on an even byte, so odd widths carry one pad byte
            let (row_len, stride) = (w * 3, (w * 3).next_multiple_of(2));
            if h > 0 && s.len() < stride * (h - 1) + row_len {
                return Err(short());
            }
            let mut rgba = Vec::with_capacity(w * h * 4);
            for r in 0..h {
                let row = s.get(r * stride..r * stride + row_len).ok_or_else(short)?;
                for px in row.as_chunks::<3>().0 {
                    rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
                }
            }
            RawPixels::Rgba(rgba)
        }
        RawFormat::V210 => {
            // 6 pixels per 16 bytes; each row padded to a multiple of 128 bytes (48 pixels)
            let stride = w.div_ceil(48) * 128;
            if s.len() < stride * h {
                return Err(short());
            }
            let (mut y, mut u, mut v) = (Vec::with_capacity(w * h), Vec::with_capacity(cw * h), Vec::with_capacity(cw * h));
            for row in s.chunks_exact(stride).take(h) {
                // the row's 10-bit samples in order: Cb Y Cr Y Cb Y Cr Y …, kept at 8 bits
                let mut comps = row.as_chunks::<4>().0.iter().flat_map(|b| {
                    let word = u32::from_le_bytes(*b);
                    [word & 0x3ff, (word >> 10) & 0x3ff, (word >> 20) & 0x3ff].map(|c| (c >> 2) as u8)
                });
                for x in 0..cw {
                    let (Some(cb), Some(y0), Some(cr), Some(y1)) = (comps.next(), comps.next(), comps.next(), comps.next()) else { break };
                    u.push(cb);
                    v.push(cr);
                    y.push(y0);
                    if 2 * x + 1 < w {
                        y.push(y1);
                    }
                }
            }
            RawPixels::Yuv422([y, u, v])
        }
        RawFormat::Yuy2 | RawFormat::Uyvy => {
            let stride = cw * 4;
            if s.len() < stride * h {
                return Err(short());
            }
            let (yo, uo, vo, y2) = if format == RawFormat::Yuy2 { (0, 1, 3, 2) } else { (1, 0, 2, 3) };
            let (mut y, mut u, mut v) = (Vec::with_capacity(w * h), Vec::with_capacity(cw * h), Vec::with_capacity(cw * h));
            for row in s.chunks_exact(stride).take(h) {
                for (k, q) in row.as_chunks::<4>().0.iter().enumerate() {
                    y.push(q[yo]);
                    if 2 * k + 1 < w {
                        y.push(q[y2]);
                    }
                    u.push(q[uo]);
                    v.push(q[vo]);
                }
            }
            RawPixels::Yuv422([y, u, v])
        }
        RawFormat::I420 | RawFormat::Yv12 | RawFormat::Nv12 | RawFormat::Gray8 => {
            let luma = w * h;
            let need = if format == RawFormat::Gray8 { luma } else { luma + 2 * cw * ch };
            if s.len() < need {
                return Err(short());
            }
            let y = s.get(..luma).unwrap_or_default().to_vec();
            let (u, v) = match format {
                RawFormat::Gray8 => (vec![128; cw * ch], vec![128; cw * ch]),
                RawFormat::Nv12 => s.get(luma..luma + 2 * cw * ch).unwrap_or_default().as_chunks::<2>().0.iter().map(|p| (p[0], p[1])).unzip(),
                _ => {
                    let a = s.get(luma..luma + cw * ch).unwrap_or_default().to_vec();
                    let b = s.get(luma + cw * ch..luma + 2 * cw * ch).unwrap_or_default().to_vec();
                    if format == RawFormat::Yv12 { (b, a) } else { (a, b) }
                }
            };
            RawPixels::Yuv420([y, u, v])
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn avi_and_mov_layouts_and_bottom_up_rows() -> Result<(), Error> {
        assert_eq!(RawFormat::from_avi(*b"yuyv", 16), Some(RawFormat::Yuy2));
        assert_eq!(RawFormat::from_avi([0; 4], 24), Some(RawFormat::Bgr24));
        assert_eq!(RawFormat::from_mov(*b"v210", 0), Some(RawFormat::V210));
        let pixels = decode(RawFormat::Bgr24, 1, 2, true, &[1, 2, 3, 0, 4, 5, 6, 0])?;
        assert_eq!(pixels, RawPixels::Rgba(vec![6, 5, 4, 255, 3, 2, 1, 255]));
        Ok(())
    }

    #[test]
    fn odd_planar_and_packed_frames_preserve_chroma() -> Result<(), Error> {
        let yuy = decode(RawFormat::Yuy2, 3, 1, false, &[10, 20, 11, 30, 12, 21, 99, 31])?;
        assert_eq!(yuy, RawPixels::Yuv422([vec![10, 11, 12], vec![20, 21], vec![30, 31]]));
        let nv = decode(RawFormat::Nv12, 3, 1, false, &[1, 2, 3, 10, 20, 11, 21])?;
        assert_eq!(nv, RawPixels::Yuv420([vec![1, 2, 3], vec![10, 11], vec![20, 21]]));
        Ok(())
    }

    #[test]
    fn truncated_frames_and_hostile_dimensions_are_rejected() {
        for format in [
            RawFormat::Bgr24,
            RawFormat::Bgrx32,
            RawFormat::Rgb24,
            RawFormat::V210,
            RawFormat::Yuy2,
            RawFormat::Uyvy,
            RawFormat::I420,
            RawFormat::Yv12,
            RawFormat::Nv12,
            RawFormat::Gray8,
        ] {
            assert_eq!(decode(format, 3, 2, false, &[]), Err(Error::Truncated));
        }
        assert_eq!(decode(RawFormat::Gray8, u32::MAX, 1, false, &[]), Err(Error::UnsupportedDimensions));
    }

    #[test]
    fn every_fourcc_and_label_is_mapped() {
        for (cc, want) in [
            (*b"UYVY", RawFormat::Uyvy),
            (*b"hdyc", RawFormat::Uyvy),
            (*b"I420", RawFormat::I420),
            (*b"iyuv", RawFormat::I420),
            (*b"YV12", RawFormat::Yv12),
            (*b"nv12", RawFormat::Nv12),
            (*b"Y800", RawFormat::Gray8),
            (*b"grey", RawFormat::Gray8),
            (*b"Y8  ", RawFormat::Gray8),
        ] {
            assert_eq!(RawFormat::from_avi(cc, 0), Some(want));
        }
        assert_eq!(RawFormat::from_avi(3u32.to_le_bytes(), 32), None);
        assert_eq!(RawFormat::from_avi_bitfields(32, [0x00ff_0000, 0x0000_ff00, 0x0000_00ff]), Some(RawFormat::Bgrx32));
        assert_eq!(RawFormat::from_avi_bitfields(32, [0x0000_00ff, 0x0000_ff00, 0x00ff_0000]), None);
        assert_eq!(RawFormat::from_avi_bitfields(24, [0x00ff_0000, 0x0000_ff00, 0x0000_00ff]), None);
        assert_eq!(RawFormat::from_avi([0; 4], 16), None);
        assert_eq!(RawFormat::from_avi(*b"H264", 0), None);
        assert_eq!(RawFormat::from_mov(*b"raw ", 24), Some(RawFormat::Rgb24));
        assert_eq!(RawFormat::from_mov(*b"raw ", 32), None);
        assert_eq!(RawFormat::from_mov(*b"2vuy", 0), Some(RawFormat::Uyvy));
        let labels = [
            (RawFormat::Bgr24, "Uncompressed RGB 24-bit"),
            (RawFormat::Rgb24, "Uncompressed RGB 24-bit"),
            (RawFormat::Bgrx32, "Uncompressed RGB 32-bit"),
            (RawFormat::Yuy2, "Uncompressed YUV 4:2:2"),
            (RawFormat::Nv12, "Uncompressed YUV 4:2:0"),
            (RawFormat::Gray8, "Uncompressed greyscale"),
            (RawFormat::V210, "Uncompressed YUV 4:2:2 10-bit"),
        ];
        for (format, label) in labels {
            assert_eq!(format.label(), label);
        }
    }

    #[test]
    fn rgb_v210_and_planar_layouts_decode() -> Result<(), Error> {
        assert_eq!(decode(RawFormat::Rgb24, 1, 1, false, &[1, 2, 3])?, RawPixels::Rgba(vec![1, 2, 3, 255]));
        // odd width: each row carries one pad byte
        assert_eq!(decode(RawFormat::Rgb24, 1, 2, false, &[1, 2, 3, 0, 4, 5, 6, 0])?, RawPixels::Rgba(vec![1, 2, 3, 255, 4, 5, 6, 255]));
        // the last row's pad byte may be missing, but earlier rows are still padded
        assert_eq!(decode(RawFormat::Rgb24, 1, 2, false, &[1, 2, 3, 0, 4, 5, 6])?, RawPixels::Rgba(vec![1, 2, 3, 255, 4, 5, 6, 255]));
        assert!(decode(RawFormat::Rgb24, 1, 2, false, &[1, 2, 3, 4, 5, 6]).is_err(), "unpadded rows are too short");
        assert_eq!(decode(RawFormat::Rgb24, 2, 1, false, &[1, 2, 3, 4, 5, 6])?, RawPixels::Rgba(vec![1, 2, 3, 255, 4, 5, 6, 255]));
        assert_eq!(decode(RawFormat::Bgrx32, 1, 1, false, &[1, 2, 3, 9])?, RawPixels::Rgba(vec![3, 2, 1, 255]));
        // v210: Cb, Y0, Cr in the first word, Y1 in the low bits of the second; rows are 128 bytes.
        let mut row = vec![0u8; 128];
        row[..4].copy_from_slice(&(0x100u32 | (0x200 << 10) | (0x300 << 20)).to_le_bytes());
        row[4..8].copy_from_slice(&0x0FCu32.to_le_bytes());
        assert_eq!(decode(RawFormat::V210, 2, 1, false, &row)?, RawPixels::Yuv422([vec![128, 63], vec![64], vec![192]]));
        assert_eq!(decode(RawFormat::V210, 1, 1, false, &row)?, RawPixels::Yuv422([vec![128], vec![64], vec![192]]));
        let planar = [1, 2, 3, 4, 10, 20];
        assert_eq!(decode(RawFormat::I420, 2, 2, false, &planar)?, RawPixels::Yuv420([vec![1, 2, 3, 4], vec![10], vec![20]]));
        assert_eq!(decode(RawFormat::Yv12, 2, 2, false, &planar)?, RawPixels::Yuv420([vec![1, 2, 3, 4], vec![20], vec![10]]));
        assert_eq!(decode(RawFormat::Gray8, 2, 2, false, &planar[..4])?, RawPixels::Yuv420([vec![1, 2, 3, 4], vec![128], vec![128]]));
        assert_eq!(decode(RawFormat::Uyvy, 2, 1, false, &[20, 10, 30, 11])?, RawPixels::Yuv422([vec![10, 11], vec![20], vec![30]]));
        Ok(())
    }
}
