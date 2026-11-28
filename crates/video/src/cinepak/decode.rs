//! Cinepak vector chunk decoding and rasterization.

use super::codebook::Vector;
use super::error::{Error, Result};
use super::strip::StripState;

/// In-progress decoded frame buffers.
pub struct FrameBuffer<'a> {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Packed RGB24 pixel buffer (`width * height * 3`).
    pub rgb: &'a mut [u8],
    /// Planar Y luminance buffer (`width * height`).
    pub y: &'a mut [u8],
    /// Optional planar U chrominance buffer (`(width/2) * (height/2)`).
    pub u: Option<&'a mut [u8]>,
    /// Optional planar V chrominance buffer (`(width/2) * (height/2)`).
    pub v: Option<&'a mut [u8]>,
    /// Optional 256-color palette (768 bytes, 3 bytes per RGB entry).
    pub palette: Option<&'a [u8; 768]>,
}

impl<'a> FrameBuffer<'a> {
    #[inline]
    fn put_pixel(&mut self, px: u32, py: u32, y_val: u8, u_val: i8, v_val: i8, is_color: bool) {
        if px >= self.width || py >= self.height {
            return;
        }

        let pixel_idx = match (py as usize).checked_mul(self.width as usize).and_then(|row| row.checked_add(px as usize)) {
            Some(idx) => idx,
            None => return,
        };

        if let Some(y_out) = self.y.get_mut(pixel_idx) {
            *y_out = y_val;
        }

        let (r, g, b) = if is_color {
            let u_s = u_val as i32;
            let v_s = v_val as i32;
            let y_s = y_val as i32;
            let r = (y_s + 2 * v_s).clamp(0, 255) as u8;
            let g = (y_s - (u_s / 2) - v_s).clamp(0, 255) as u8;
            let b = (y_s + 2 * u_s).clamp(0, 255) as u8;
            (r, g, b)
        } else if let Some(pal) = self.palette {
            let pal_idx = (y_val as usize) * 3;
            if let Some(rgb_chunk) = pal.get(pal_idx..pal_idx + 3) { (rgb_chunk[0], rgb_chunk[1], rgb_chunk[2]) } else { (y_val, y_val, y_val) }
        } else {
            (y_val, y_val, y_val)
        };

        let rgb_offset = match pixel_idx.checked_mul(3) {
            Some(o) => o,
            None => return,
        };

        if let Some(dst) = self.rgb.get_mut(rgb_offset..rgb_offset + 3) {
            dst[0] = r;
            dst[1] = g;
            dst[2] = b;
        }
    }

    #[inline]
    fn put_chroma(&mut self, cx: u32, cy: u32, u_val: i8, v_val: i8) {
        let chroma_w = self.width.div_ceil(2);
        let chroma_h = self.height.div_ceil(2);
        if cx >= chroma_w || cy >= chroma_h {
            return;
        }

        let chroma_idx = match (cy as usize).checked_mul(chroma_w as usize).and_then(|row| row.checked_add(cx as usize)) {
            Some(idx) => idx,
            None => return,
        };

        if let Some(ref mut u_buf) = self.u
            && let Some(u_out) = u_buf.get_mut(chroma_idx)
        {
            *u_out = (u_val as i16 + 128).clamp(0, 255) as u8;
        }
        if let Some(ref mut v_buf) = self.v
            && let Some(v_out) = v_buf.get_mut(chroma_idx)
        {
            *v_out = (v_val as i16 + 128).clamp(0, 255) as u8;
        }
    }

    /// Render a 2×2 quadrant using vector luminance and chrominance.
    fn render_quadrant(&mut self, qx: u32, qy: u32, y: [u8; 4], u: i8, v: i8, is_color: bool) {
        self.put_pixel(qx, qy, y[0], u, v, is_color);
        self.put_pixel(qx + 1, qy, y[1], u, v, is_color);
        self.put_pixel(qx, qy + 1, y[2], u, v, is_color);
        self.put_pixel(qx + 1, qy + 1, y[3], u, v, is_color);

        if is_color {
            self.put_chroma(qx / 2, qy / 2, u, v);
        } else if self.u.is_some() || self.v.is_some() {
            self.put_chroma(qx / 2, qy / 2, 0, 0);
        }
    }

    /// Render a V4 macroblock (4 distinct 2×2 vectors).
    fn render_v4(&mut self, bx: u32, by: u32, vectors: [&Vector; 4], is_color: bool) {
        self.render_quadrant(bx, by, vectors[0].y, vectors[0].u, vectors[0].v, is_color);
        self.render_quadrant(bx + 2, by, vectors[1].y, vectors[1].u, vectors[1].v, is_color);
        self.render_quadrant(bx, by + 2, vectors[2].y, vectors[2].u, vectors[2].v, is_color);
        self.render_quadrant(bx + 2, by + 2, vectors[3].y, vectors[3].u, vectors[3].v, is_color);
    }

    /// Render a V1 macroblock (1 vector upscaled to 4×4).
    fn render_v1(&mut self, bx: u32, by: u32, v: &Vector, is_color: bool) {
        self.render_quadrant(bx, by, [v.y[0], v.y[0], v.y[0], v.y[0]], v.u, v.v, is_color);
        self.render_quadrant(bx + 2, by, [v.y[1], v.y[1], v.y[1], v.y[1]], v.u, v.v, is_color);
        self.render_quadrant(bx, by + 2, [v.y[2], v.y[2], v.y[2], v.y[2]], v.u, v.v, is_color);
        self.render_quadrant(bx + 2, by + 2, [v.y[3], v.y[3], v.y[3], v.y[3]], v.u, v.v, is_color);
    }
}

/// Bitstream reader for VLC flags in 0x30 and 0x31 vector chunks.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    mask: u32,
    flags: u32,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, mask: 0, flags: 0 }
    }

    fn read_bit(&mut self) -> Result<bool> {
        if self.mask == 0 {
            let b = self.data.get(self.pos..self.pos + 4).ok_or(Error::Truncated)?;
            self.flags = u32::from_be_bytes([b[0], b[1], b[2], b[3]]);
            self.pos = self.pos.checked_add(4).ok_or(Error::Truncated)?;
            self.mask = 0x8000_0000;
        }
        let bit = (self.flags & self.mask) != 0;
        self.mask >>= 1;
        Ok(bit)
    }

    fn read_byte(&mut self) -> Result<u8> {
        let b = self.data.get(self.pos).copied().ok_or(Error::Truncated)?;
        self.pos = self.pos.checked_add(1).ok_or(Error::Truncated)?;
        Ok(b)
    }

    fn read_bytes_4(&mut self) -> Result<[u8; 4]> {
        let b = self.data.get(self.pos..self.pos + 4).ok_or(Error::Truncated)?;
        self.pos = self.pos.checked_add(4).ok_or(Error::Truncated)?;
        Ok([b[0], b[1], b[2], b[3]])
    }
}

/// Decode a vector chunk (`0x30`, `0x31`, or `0x32`) for a strip.
pub fn decode_vector_chunk(fb: &mut FrameBuffer<'_>, strip: &StripState, chunk_type: u8, payload: &[u8], bounds: (u32, u32, u32, u32)) -> Result<()> {
    let (x0, y0, x1, y1) = bounds;
    let mut reader = BitReader::new(payload);

    match chunk_type {
        0x30 => {
            // Intra vectors: 1 bit per macroblock (1 = V4, 0 = V1).
            for y in (y0..y1).step_by(4) {
                for x in (x0..x1).step_by(4) {
                    let is_v4 = reader.read_bit()?;
                    if is_v4 {
                        let indices = reader.read_bytes_4()?;
                        let v0 = &strip.v4_codebook.entries[indices[0] as usize];
                        let v1 = &strip.v4_codebook.entries[indices[1] as usize];
                        let v2 = &strip.v4_codebook.entries[indices[2] as usize];
                        let v3 = &strip.v4_codebook.entries[indices[3] as usize];
                        fb.render_v4(x, y, [v0, v1, v2, v3], strip.v4_codebook.is_color);
                    } else {
                        let idx = reader.read_byte()?;
                        let v = &strip.v1_codebook.entries[idx as usize];
                        fb.render_v1(x, y, v, strip.v1_codebook.is_color);
                    }
                }
            }
        }
        0x31 => {
            // Inter vectors: VLC (0 = skip, 10 = V1, 11 = V4).
            for y in (y0..y1).step_by(4) {
                for x in (x0..x1).step_by(4) {
                    let updated = reader.read_bit()?;
                    if updated {
                        let is_v4 = reader.read_bit()?;
                        if is_v4 {
                            let indices = reader.read_bytes_4()?;
                            let v0 = &strip.v4_codebook.entries[indices[0] as usize];
                            let v1 = &strip.v4_codebook.entries[indices[1] as usize];
                            let v2 = &strip.v4_codebook.entries[indices[2] as usize];
                            let v3 = &strip.v4_codebook.entries[indices[3] as usize];
                            fb.render_v4(x, y, [v0, v1, v2, v3], strip.v4_codebook.is_color);
                        } else {
                            let idx = reader.read_byte()?;
                            let v = &strip.v1_codebook.entries[idx as usize];
                            fb.render_v1(x, y, v, strip.v1_codebook.is_color);
                        }
                    }
                }
            }
        }
        0x32 => {
            // V1 only: no flags, each macroblock is 1 byte index.
            for y in (y0..y1).step_by(4) {
                for x in (x0..x1).step_by(4) {
                    let idx = reader.read_byte()?;
                    let v = &strip.v1_codebook.entries[idx as usize];
                    fb.render_v1(x, y, v, strip.v1_codebook.is_color);
                }
            }
        }
        _ => return Err(Error::InvalidChunk("unrecognized Cinepak vector chunk type")),
    }

    Ok(())
}
