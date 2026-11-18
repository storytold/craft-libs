//! Vector quantization codebooks for Cinepak decoding.

use super::error::{Error, Result};

/// One codebook vector representing a 2×2 subblock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Vector {
    /// 4 luminance (or palette index) samples in raster order:
    /// `[top-left, top-right, bottom-left, bottom-right]`.
    pub y: [u8; 4],
    /// Shared signed chrominance U component (0 for grayscale / paletted).
    pub u: i8,
    /// Shared signed chrominance V component (0 for grayscale / paletted).
    pub v: i8,
}

/// A Cinepak codebook of up to 256 vectors.
#[derive(Clone, Debug)]
pub struct Codebook {
    /// 256 vectors.
    pub entries: [Vector; 256],
    /// Whether this codebook represents 12 bpp color vectors (true) or grayscale/paletted (false).
    pub is_color: bool,
}

impl Default for Codebook {
    fn default() -> Self {
        Self { entries: [Vector::default(); 256], is_color: false }
    }
}

impl Codebook {
    /// Create a new, blank codebook.
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Full codebook update from 12 bpp color entries (6 bytes per entry: `y0, y1, y2, y3, u, v`).
    pub fn update_full_color(&mut self, payload: &[u8]) -> Result<()> {
        let count = (payload.len() / 6).min(256);
        for (i, chunk) in payload[..count * 6].as_chunks::<6>().0.iter().enumerate() {
            if let Some(entry) = self.entries.get_mut(i) {
                entry.y = [chunk[0], chunk[1], chunk[2], chunk[3]];
                entry.u = chunk[4] as i8;
                entry.v = chunk[5] as i8;
            }
        }
        self.is_color = true;
        Ok(())
    }

    /// Partial codebook update from 12 bpp color entries with 32-bit bitmasks.
    pub fn update_partial_color(&mut self, payload: &[u8]) -> Result<()> {
        let mut pos = 0;
        let mut mask: u32 = 0;
        let mut flags: u32 = 0;

        for i in 0..256 {
            if mask == 0 {
                if pos >= payload.len() {
                    break;
                }
                let flag_bytes = payload.get(pos..pos + 4).ok_or(Error::Truncated)?;
                flags = u32::from_be_bytes([flag_bytes[0], flag_bytes[1], flag_bytes[2], flag_bytes[3]]);
                pos = pos.checked_add(4).ok_or(Error::Truncated)?;
                mask = 0x8000_0000;
            }

            if (flags & mask) != 0 {
                let entry_bytes = payload.get(pos..pos + 6).ok_or(Error::Truncated)?;
                pos = pos.checked_add(6).ok_or(Error::Truncated)?;
                if let Some(entry) = self.entries.get_mut(i) {
                    entry.y = [entry_bytes[0], entry_bytes[1], entry_bytes[2], entry_bytes[3]];
                    entry.u = entry_bytes[4] as i8;
                    entry.v = entry_bytes[5] as i8;
                }
            }
            mask >>= 1;
        }
        self.is_color = true;
        Ok(())
    }

    /// Full codebook update from 8 bpp grayscale/paletted entries (4 bytes per entry: `y0, y1, y2, y3`).
    pub fn update_full_gray(&mut self, payload: &[u8]) -> Result<()> {
        let count = (payload.len() / 4).min(256);
        for (i, chunk) in payload[..count * 4].as_chunks::<4>().0.iter().enumerate() {
            if let Some(entry) = self.entries.get_mut(i) {
                entry.y = [chunk[0], chunk[1], chunk[2], chunk[3]];
                entry.u = 0;
                entry.v = 0;
            }
        }
        self.is_color = false;
        Ok(())
    }

    /// Partial codebook update from 8 bpp grayscale/paletted entries with 32-bit bitmasks.
    pub fn update_partial_gray(&mut self, payload: &[u8]) -> Result<()> {
        let mut pos = 0;
        let mut mask: u32 = 0;
        let mut flags: u32 = 0;

        for i in 0..256 {
            if mask == 0 {
                if pos >= payload.len() {
                    break;
                }
                let flag_bytes = payload.get(pos..pos + 4).ok_or(Error::Truncated)?;
                flags = u32::from_be_bytes([flag_bytes[0], flag_bytes[1], flag_bytes[2], flag_bytes[3]]);
                pos = pos.checked_add(4).ok_or(Error::Truncated)?;
                mask = 0x8000_0000;
            }

            if (flags & mask) != 0 {
                let entry_bytes = payload.get(pos..pos + 4).ok_or(Error::Truncated)?;
                pos = pos.checked_add(4).ok_or(Error::Truncated)?;
                if let Some(entry) = self.entries.get_mut(i) {
                    entry.y = [entry_bytes[0], entry_bytes[1], entry_bytes[2], entry_bytes[3]];
                    entry.u = 0;
                    entry.v = 0;
                }
            }
            mask >>= 1;
        }
        self.is_color = false;
        Ok(())
    }
}
