//! Clean-room pure-Rust Cinepak (Compact Video / CVID) video decoder.
//!
//! Implemented from public community specifications (Dr. Tim Ferguson, 2001) and observed
//! bitstream behavior using FFmpeg as a black-box verification oracle.
//!
//! Cinepak is an adaptive vector quantization (VQ) video codec operating on 4×4 pixel macroblocks.
//! Each frame is divided into one or more horizontal strips, where each strip maintains its own
//! pair of V1 (1 vector per 4×4 block) and V4 (4 vectors per 4×4 block) codebooks of up to 256
//! entries.
//!
//! Both 12 bpp color (YUV 4:2:0) and 8 bpp grayscale or paletted variants are supported, as well
//! as keyframes and inter-frames with skip bitmasks.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod codebook;
mod decode;
mod error;
mod strip;

pub use error::{Error, Result};
pub use strip::StripHeader;

use decode::{FrameBuffer, decode_vector_chunk};
use strip::StripState;

/// A decoded Cinepak picture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Packed RGB24 samples (3 bytes per pixel in raster order: R, G, B), length `width * height * 3`.
    pub rgb: Vec<u8>,
    /// Planar Y (luminance or palette index) samples, length `width * height`.
    pub y: Vec<u8>,
    /// Planar U samples (present for 12 bpp color frames), length `((width + 1) / 2) * ((height + 1) / 2)`.
    pub u: Option<Vec<u8>>,
    /// Planar V samples (present for 12 bpp color frames), length `((width + 1) / 2) * ((height + 1) / 2)`.
    pub v: Option<Vec<u8>>,
    /// Whether this was an intra-coded frame (keyframe).
    pub key: bool,
    /// Whether color codebooks (12 bpp) were decoded.
    pub is_color: bool,
}

/// Maximum bytes required for one fully-allocated Cinepak picture buffer (RGB24 + planar Y + planar U/V).
pub fn picture_bytes(width: u32, height: u32) -> Option<u64> {
    let w = width as u64;
    let h = height as u64;
    let area = w.checked_mul(h)?;
    let rgb = area.checked_mul(3)?;
    let y = area;
    let uv_w = w.div_ceil(2);
    let uv_h = h.div_ceil(2);
    let uv = uv_w.checked_mul(uv_h)?.checked_mul(2)?;
    rgb.checked_add(y)?.checked_add(uv)
}

/// Checks that all simultaneously live staging picture buffers (working, reference, and output)
/// fit within [`crate::budget::DECODE_MEMORY_BUDGET`].
pub fn check_memory_budget(width: u32, height: u32) -> Result<()> {
    let single = picture_bytes(width, height).ok_or(Error::MemoryBudget { width, height })?;
    // Accounts for working picture, reference picture, and output clone (3 pictures).
    let total = single.checked_mul(3).ok_or(Error::MemoryBudget { width, height })?;
    if total > crate::budget::DECODE_MEMORY_BUDGET {
        return Err(Error::MemoryBudget { width, height });
    }
    Ok(())
}

/// Checks that all simultaneously live picture buffers (working, output, and any retained reference
/// frame, including across dimension changes) fit within [`crate::budget::DECODE_MEMORY_BUDGET`].
pub fn check_decode_budget(width: u32, height: u32, last_frame: Option<&Frame>) -> Result<()> {
    let working = picture_bytes(width, height).ok_or(Error::MemoryBudget { width, height })?;
    let output = working;
    let old_ref = match last_frame {
        Some(f) => picture_bytes(f.width, f.height).ok_or(Error::MemoryBudget { width: f.width, height: f.height })?,
        None => 0,
    };
    let total = working.checked_add(output).and_then(|sum| sum.checked_add(old_ref)).ok_or(Error::MemoryBudget { width, height })?;
    if total > crate::budget::DECODE_MEMORY_BUDGET {
        return Err(Error::MemoryBudget { width, height });
    }
    Ok(())
}

impl Frame {
    /// Allocate a new, blank frame with the given dimensions, checking the memory budget.
    pub fn new(width: u32, height: u32, is_color: bool) -> Result<Self> {
        check_memory_budget(width, height)?;
        let w = width as u64;
        let h = height as u64;
        let area = w.checked_mul(h).ok_or(Error::MemoryBudget { width, height })?;
        let rgb_bytes = area.checked_mul(3).ok_or(Error::MemoryBudget { width, height })?;
        let rgb_len = usize::try_from(rgb_bytes).map_err(|_| Error::MemoryBudget { width, height })?;
        let y_len = usize::try_from(area).map_err(|_| Error::MemoryBudget { width, height })?;

        let (u, v) = if is_color {
            let cw = w.div_ceil(2);
            let ch = h.div_ceil(2);
            let chroma_len = usize::try_from(cw.checked_mul(ch).ok_or(Error::MemoryBudget { width, height })?)
                .map_err(|_| Error::MemoryBudget { width, height })?;
            (Some(vec![128; chroma_len]), Some(vec![128; chroma_len]))
        } else {
            (None, None)
        };

        Ok(Self { width, height, rgb: vec![0; rgb_len], y: vec![0; y_len], u, v, key: true, is_color })
    }

    /// Read an RGB pixel at `(x, y)`. Returns `None` if out of bounds.
    pub fn pixel_rgb(&self, x: u32, y: u32) -> Option<[u8; 3]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let row_offset = (y as usize).checked_mul(self.width as usize)?;
        let pixel_offset = row_offset.checked_add(x as usize)?;
        let idx = pixel_offset.checked_mul(3)?;
        let slice = self.rgb.get(idx..idx.checked_add(3)?)?;
        Some([slice[0], slice[1], slice[2]])
    }
}

/// State of a Cinepak decoder across sequential frames.
#[derive(Clone, Debug, Default)]
pub struct Decoder {
    last_frame: Option<Frame>,
    strips: Vec<StripState>,
    palette: Option<[u8; 768]>,
}

impl Decoder {
    /// Create a new Cinepak decoder.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a new Cinepak decoder with an 8-bit palette (256 RGB triplets = 768 bytes).
    pub fn with_palette(palette: [u8; 768]) -> Self {
        Self { last_frame: None, strips: Vec::new(), palette: Some(palette) }
    }

    /// Create a new Cinepak decoder with an 8-bit palette from a byte slice (up to 768 bytes).
    pub fn with_palette_slice(palette: &[u8]) -> Self {
        let mut pal = [0u8; 768];
        let n = palette.len().min(768);
        pal[..n].copy_from_slice(&palette[..n]);
        Self::with_palette(pal)
    }

    /// Set or clear the 8-bit color palette.
    ///
    /// An 8 bpp picture is stored as palette indices (`Frame::y`) next to the RGB it produced, and
    /// an inter frame keeps the pixels of its skipped blocks. So that those pixels and the blocks
    /// the next frame decodes share one palette, changing the palette recolors the retained
    /// reference picture from its indices. 12 bpp color pictures do not use the palette and are
    /// left as they are.
    pub fn set_palette(&mut self, palette: Option<[u8; 768]>) {
        self.palette = palette;
        self.recolor_reference();
    }

    /// Set or clear the 8-bit color palette from a slice (up to 768 bytes); see [`Self::set_palette`].
    pub fn set_palette_slice(&mut self, palette: Option<&[u8]>) {
        self.palette = palette.map(|p| {
            let mut pal = [0u8; 768];
            let n = p.len().min(768);
            pal[..n].copy_from_slice(&p[..n]);
            pal
        });
        self.recolor_reference();
    }

    /// Rebuild the RGB of an indexed reference picture with the current palette.
    fn recolor_reference(&mut self) {
        let Some(frame) = self.last_frame.as_mut().filter(|f| !f.is_color) else { return };
        for (index, rgb) in frame.y.iter().zip(frame.rgb.as_chunks_mut::<3>().0) {
            let i = usize::from(*index) * 3;
            match self.palette.as_ref().and_then(|p| p.get(i..i + 3)) {
                Some(entry) => rgb.copy_from_slice(entry),
                None => *rgb = [*index; 3],
            }
        }
    }

    /// Reset internal state, clearing references and codebooks.
    pub fn reset(&mut self) {
        self.last_frame = None;
        self.strips.clear();
    }

    /// Decode one Cinepak frame.
    pub fn decode(&mut self, data: &[u8]) -> Result<Frame> {
        if data.len() < 10 {
            return Err(Error::Truncated);
        }

        let flags = data[0];
        let can_inherit = (flags & 0x01) != 0;
        let total_len = ((data[1] as usize) << 16) | ((data[2] as usize) << 8) | (data[3] as usize);
        let width = u16::from_be_bytes([data[4], data[5]]) as u32;
        let height = u16::from_be_bytes([data[6], data[7]]) as u32;
        let num_strips = u16::from_be_bytes([data[8], data[9]]) as usize;

        if width == 0 || height == 0 {
            return Err(Error::InvalidHeader("width and height must be non-zero"));
        }
        check_decode_budget(width, height, self.last_frame.as_ref())?;
        if num_strips == 0 {
            return Err(Error::InvalidHeader("strip count must be greater than zero"));
        }
        if num_strips > 64 {
            return Err(Error::InvalidHeader("excessive strip count"));
        }
        if data.len() < total_len {
            return Err(Error::Truncated);
        }

        // Validate strip headers and check if any strip is inter-coded.
        let mut scan_offset = 10;
        let mut has_inter = false;
        for _ in 0..num_strips {
            let strip_hdr_data = data.get(scan_offset..).ok_or(Error::Truncated)?;
            let hdr = StripHeader::parse(strip_hdr_data)?;
            let strip_end = scan_offset.checked_add(hdr.size).ok_or(Error::Truncated)?;
            if strip_end > total_len {
                return Err(Error::Truncated);
            }
            if (hdr.id & 0xFF00) == 0x1100 {
                has_inter = true;
            }
            scan_offset = strip_end;
        }

        let is_key = !has_inter;
        let mut frame = if is_key {
            Frame::new(width, height, false)?
        } else {
            let last = self.last_frame.as_ref().ok_or(Error::MissingReference)?;
            if last.width != width || last.height != height {
                return Err(Error::InvalidHeader("inter frame dimensions mismatch"));
            }
            last.clone()
        };
        frame.key = is_key;

        // Stage strip updates in a local copy so decode errors leave decoder state unchanged.
        let mut strips = if !is_key || can_inherit {
            let mut s = self.strips.clone();
            if s.len() < num_strips {
                s.resize_with(num_strips, StripState::default);
            }
            s
        } else {
            vec![StripState::default(); num_strips]
        };

        let mut offset = 10;
        let mut prev_bottom_y = 0;

        for s_idx in 0..num_strips {
            let strip_hdr_data = data.get(offset..).ok_or(Error::Truncated)?;
            let hdr = StripHeader::parse(strip_hdr_data)?;

            let strip_end = offset + hdr.size;
            let bounds = hdr.resolve_bounds(s_idx == 0, prev_bottom_y, width, height)?;
            prev_bottom_y = bounds.3;

            // When codebook inheritance is enabled, a strip starts from the codebooks and mode of the strip
            // before it: the previous strip of this frame, or for the first strip the last strip
            // of the previous frame.
            if can_inherit {
                let source = if s_idx > 0 { strips.get(s_idx - 1) } else { self.strips.last() };
                if let Some(prev_strip) = source.cloned() {
                    strips[s_idx] = prev_strip;
                }
            }

            let strip_slice = &data[offset + 12..strip_end];
            let strip = match strips.get_mut(s_idx) {
                Some(s) => s,
                None => return Err(Error::InvalidStrip("strip index out of bounds")),
            };

            let mut c_offset = 0;
            while c_offset < strip_slice.len() {
                let chunk_head = strip_slice.get(c_offset..).ok_or(Error::Truncated)?;
                if chunk_head.len() < 4 {
                    break;
                }
                let chunk_type = chunk_head[0];
                let chunk_size = ((chunk_head[1] as usize) << 16) | ((chunk_head[2] as usize) << 8) | (chunk_head[3] as usize);
                if chunk_size < 4 {
                    return Err(Error::InvalidChunk("chunk size less than 4"));
                }
                let chunk_end = c_offset.checked_add(chunk_size).ok_or(Error::Truncated)?;
                if chunk_end > strip_slice.len() {
                    return Err(Error::Truncated);
                }

                let payload = &strip_slice[c_offset + 4..chunk_end];
                match chunk_type {
                    0x20 => {
                        strip.v4_codebook.update_full_color(payload)?;
                        strip.is_color = strip.v1_codebook.is_color || strip.v4_codebook.is_color;
                    }
                    0x21 => {
                        strip.v4_codebook.update_partial_color(payload)?;
                        strip.is_color = strip.v1_codebook.is_color || strip.v4_codebook.is_color;
                    }
                    0x22 => {
                        strip.v1_codebook.update_full_color(payload)?;
                        strip.is_color = strip.v1_codebook.is_color || strip.v4_codebook.is_color;
                    }
                    0x23 => {
                        strip.v1_codebook.update_partial_color(payload)?;
                        strip.is_color = strip.v1_codebook.is_color || strip.v4_codebook.is_color;
                    }
                    0x24 => {
                        strip.v4_codebook.update_full_gray(payload)?;
                        strip.is_color = strip.v1_codebook.is_color || strip.v4_codebook.is_color;
                    }
                    0x25 => {
                        strip.v4_codebook.update_partial_gray(payload)?;
                        strip.is_color = strip.v1_codebook.is_color || strip.v4_codebook.is_color;
                    }
                    0x26 => {
                        strip.v1_codebook.update_full_gray(payload)?;
                        strip.is_color = strip.v1_codebook.is_color || strip.v4_codebook.is_color;
                    }
                    0x27 => {
                        strip.v1_codebook.update_partial_gray(payload)?;
                        strip.is_color = strip.v1_codebook.is_color || strip.v4_codebook.is_color;
                    }
                    0x30..=0x32 => {
                        if strip.is_color && frame.u.is_none() {
                            let cw = ((width as usize).div_ceil(2)).max(1);
                            let ch = ((height as usize).div_ceil(2)).max(1);
                            frame.u = Some(vec![128; cw * ch]);
                            frame.v = Some(vec![128; cw * ch]);
                        }
                        let mut fb = FrameBuffer {
                            width,
                            height,
                            rgb: &mut frame.rgb,
                            y: &mut frame.y,
                            u: frame.u.as_deref_mut(),
                            v: frame.v.as_deref_mut(),
                            palette: self.palette.as_ref(),
                        };
                        decode_vector_chunk(&mut fb, strip, chunk_type, payload, bounds)?;
                    }
                    _ => {
                        // Skip unrecognized non-critical subchunks
                    }
                }
                c_offset = chunk_end;
            }

            offset = strip_end;
        }

        // Only this frame's strips count: the next frame's first strip inherits from the last.
        strips.truncate(num_strips);
        frame.is_color = (!is_key && frame.is_color) || strips.iter().any(|s| s.is_color);
        self.strips = strips;
        let out_frame = frame.clone();
        self.last_frame = Some(frame);
        Ok(out_frame)
    }
}

/// Convenience function to decode a single self-contained Cinepak keyframe.
pub fn decode_frame(data: &[u8]) -> Result<Frame> {
    let mut decoder = Decoder::new();
    decoder.decode(data)
}

/// Convenience function to decode a single Cinepak frame with an 8-bit palette.
pub fn decode_frame_with_palette(data: &[u8], palette: &[u8; 768]) -> Result<Frame> {
    let mut decoder = Decoder::with_palette(*palette);
    decoder.decode(data)
}
