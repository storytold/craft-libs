//! Borrowed frames the scopes read: 8-bit R'G'B'A, or 8-bit NV12 (a Y' plane and an interleaved
//! Cb Cr plane at half width and half height), each with its own row stride.

use std::fmt;

use craft_color::{Matrix, Range};

/// A frame borrowed from its owner (a readback buffer, a decoder, a capture). Strides are in bytes.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Frame<'a> {
    /// R'G'B'A, 4 bytes per pixel, `stride` bytes per row (at least `width * 4`). Alpha is ignored.
    Rgba8 { width: usize, height: usize, stride: usize, data: &'a [u8] },
    /// Y' at full size and Cb Cr pairs at half width and half height (rounded up). `matrix` and
    /// `range` say how the code values turn back into R'G'B'.
    Nv12 { width: usize, height: usize, y: &'a [u8], y_stride: usize, uv: &'a [u8], uv_stride: usize, matrix: Matrix, range: Range },
}

/// Why a [`Frame`] cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FrameError {
    /// A row stride shorter than one row of samples.
    Stride { plane: &'static str, stride: usize, needed: usize },
    /// A plane shorter than its rows.
    Short { plane: &'static str, len: usize, needed: usize },
    /// The frame's size in bytes overflows `usize`.
    TooLarge,
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrameError::Stride { plane, stride, needed } => write!(f, "{plane} plane stride {stride} is shorter than a row ({needed} bytes)"),
            FrameError::Short { plane, len, needed } => write!(f, "{plane} plane holds {len} bytes; the frame needs {needed}"),
            FrameError::TooLarge => write!(f, "the frame is too large to address"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Bytes a plane of `rows` rows needs: every row but the last at `stride`, the last one `row_bytes`.
fn plane_len(rows: usize, stride: usize, row_bytes: usize) -> Option<usize> {
    if rows == 0 { Some(0) } else { stride.checked_mul(rows - 1)?.checked_add(row_bytes) }
}

fn check_plane(plane: &'static str, len: usize, rows: usize, stride: usize, row_bytes: usize) -> Result<(), FrameError> {
    if rows == 0 || row_bytes == 0 {
        return Ok(());
    }
    if stride < row_bytes {
        return Err(FrameError::Stride { plane, stride, needed: row_bytes });
    }
    let needed = plane_len(rows, stride, row_bytes).ok_or(FrameError::TooLarge)?;
    if len < needed {
        return Err(FrameError::Short { plane, len, needed });
    }
    Ok(())
}

impl Frame<'_> {
    pub fn width(&self) -> usize {
        match *self {
            Frame::Rgba8 { width, .. } | Frame::Nv12 { width, .. } => width,
        }
    }

    pub fn height(&self) -> usize {
        match *self {
            Frame::Rgba8 { height, .. } | Frame::Nv12 { height, .. } => height,
        }
    }

    /// Checks that every row of every plane lies inside its slice. A frame with no width or no
    /// height is valid and has no samples.
    pub fn validate(&self) -> Result<(), FrameError> {
        if self.width() == 0 || self.height() == 0 {
            return Ok(());
        }
        match *self {
            Frame::Rgba8 { width, height, stride, data } => {
                let row = width.checked_mul(4).ok_or(FrameError::TooLarge)?;
                check_plane("RGBA", data.len(), height, stride, row)
            }
            Frame::Nv12 { width, height, y, y_stride, uv, uv_stride, .. } => {
                check_plane("Y", y.len(), height, y_stride, width)?;
                let uv_row = width.div_ceil(2).checked_mul(2).ok_or(FrameError::TooLarge)?;
                check_plane("UV", uv.len(), height.div_ceil(2), uv_stride, uv_row)
            }
        }
    }
}
