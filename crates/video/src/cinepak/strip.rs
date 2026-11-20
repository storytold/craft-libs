//! Cinepak strip header parsing and state tracking.

use super::codebook::Codebook;
use super::error::{Error, Result};

/// State of a Cinepak strip, retaining codebooks across inter-coded frames.
#[derive(Clone, Debug, Default)]
pub struct StripState {
    /// 1-vector per 4×4 block codebook.
    pub v1_codebook: Codebook,
    /// 4-vector per 4×4 block codebook.
    pub v4_codebook: Codebook,
    /// Whether this strip uses 12 bpp color codebooks (true) or grayscale/paletted (false).
    pub is_color: bool,
}

/// Parsed header of a Cinepak strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StripHeader {
    /// Strip type ID (typically `0x1000` intra or `0x1100` inter).
    pub id: u16,
    /// Declared byte length of this strip, including the 12-byte header.
    pub size: usize,
    /// Top Y position.
    pub y0: u32,
    /// Left X position.
    pub x0: u32,
    /// Bottom Y position (or height if relative).
    pub y1: u32,
    /// Right X position.
    pub x1: u32,
}

impl StripHeader {
    /// Parse a 12-byte strip header.
    pub fn parse(data: &[u8]) -> Result<Self> {
        let b = data.get(..12).ok_or(Error::Truncated)?;
        let id = u16::from_be_bytes([b[0], b[1]]);
        let size = u16::from_be_bytes([b[2], b[3]]) as usize;
        let y0 = u16::from_be_bytes([b[4], b[5]]) as u32;
        let x0 = u16::from_be_bytes([b[6], b[7]]) as u32;
        let y1 = u16::from_be_bytes([b[8], b[9]]) as u32;
        let x1 = u16::from_be_bytes([b[10], b[11]]) as u32;

        if size < 12 {
            return Err(Error::InvalidStrip("strip size less than header length"));
        }

        Ok(Self { id, size, y0, x0, y1, x1 })
    }

    /// Resolve effective strip boundaries `(x0, y0, x1, y1)` given the preceding strip's bottom Y
    /// and overall frame dimensions.
    pub fn resolve_bounds(&self, is_first: bool, prev_bottom_y: u32, frame_width: u32, frame_height: u32) -> Result<(u32, u32, u32, u32)> {
        let (actual_y0, actual_y1) = if !is_first && self.y0 == 0 {
            let actual_y0 = prev_bottom_y;
            let actual_y1 = actual_y0.checked_add(self.y1).ok_or(Error::InvalidStrip("vertical coordinate overflow"))?;
            (actual_y0, actual_y1)
        } else {
            (self.y0, self.y1)
        };

        let x0 = self.x0;
        let x1 = if self.x1 == 0 { frame_width } else { self.x1 };

        if actual_y0 > actual_y1 || actual_y1 > frame_height || x0 > x1 || x1 > frame_width {
            return Err(Error::InvalidStrip("strip coordinates exceed frame dimensions"));
        }

        Ok((x0, actual_y0, x1, actual_y1))
    }
}
