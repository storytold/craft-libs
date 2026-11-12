#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! The blocks of a GIF, read without decompressing any pixels (GIF89a specification: Logical
//! Screen Descriptor, Global and Local Color Tables, Graphic Control Extension, Application
//! Extension, Image Descriptor).
//!
//! [`scan`] walks the file once and reports what an importer needs before it decodes anything:
//! the logical screen size, every frame's position, size, delay and transparent colour, whether
//! the file reached its trailer, and the loop count. That answers "is this an animation, how
//! long is it, how many frames" and "can a file this small hold a picture this large" from the
//! headers alone. [`palette`] gives the colour table of the first frame, for opening a GIF as an
//! indexed image.
//!
//! With feature `frames`, [`frames::read_frames`] decodes every frame through the `image` crate's
//! GIF decoder, after the checks [`scan`] makes possible, with bounded memory.
//!
//! Input is treated as hostile: every read is bounds-checked, and the number of frames recorded
//! is capped by the caller.

#[cfg(feature = "frames")]
pub mod frames;
#[cfg(test)]
mod tests;

/// One frame (image descriptor) of a GIF.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Frame {
    /// File offset of the image descriptor (its `0x2C` separator).
    pub offset: usize,
    /// Position of the frame on the logical screen.
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
    /// Delay before the next frame in hundredths of a second, as stored (0 when the frame has no
    /// Graphic Control Extension). Players show very small delays longer; that is the caller's rule.
    pub delay_cs: u16,
    /// The transparent colour index, when the frame's Graphic Control Extension names one.
    pub transparent: Option<u8>,
    /// Disposal method (0–7) from the Graphic Control Extension.
    pub disposal: u8,
    /// Whether the frame has its own colour table.
    pub local_table: bool,
}

/// What the blocks of a GIF say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    /// Logical screen size.
    pub width: u16,
    pub height: u16,
    /// Whether the file has a Global Color Table.
    pub global_table: bool,
    /// The frames found, in file order, up to the caller's limit.
    pub frames: Vec<Frame>,
    /// Whether the walk reached the trailer. When it did not (the file is cut short or damaged, or
    /// the frame limit was reached), more frames may exist than [`Info::frames`] holds.
    pub complete: bool,
    /// The NETSCAPE2.0 loop count: `Some(0)` loops forever, `None` when the file has none (play once).
    pub loop_count: Option<u16>,
}

impl Info {
    /// Whether the file is known to hold more than one frame, or `None` when the walk stopped
    /// before the trailer after at most one frame, so frames past the break could not be counted.
    pub fn has_more_frames(&self) -> Option<bool> {
        if self.frames.len() > 1 { Some(true) } else { self.complete.then_some(false) }
    }

    /// The sum of the frame delays in hundredths of a second, as stored.
    pub fn total_delay_cs(&self) -> u64 {
        self.frames.iter().map(|f| u64::from(f.delay_cs)).sum()
    }
}

/// Whether `bytes` starts with a GIF signature.
pub fn sniff(bytes: &[u8]) -> bool {
    bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")
}

/// Bytes of the colour table a packed-fields byte announces (0 without one).
fn table_len(packed: u8) -> usize {
    if packed & 0x80 != 0 { 3usize << ((packed & 7) + 1) } else { 0 }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

/// Skips the data sub-blocks starting at `at` (each length-prefixed, ended by a zero length).
fn skip_sub_blocks(b: &[u8], mut at: usize) -> Option<usize> {
    loop {
        let n = usize::from(*b.get(at)?);
        at = at.checked_add(1 + n)?;
        if n == 0 {
            return Some(at);
        }
    }
}

/// Walks the blocks of a GIF without decompressing any, recording at most `max_frames` frames.
/// `None` when `bytes` is not a GIF or ends inside its Logical Screen Descriptor. A file that
/// breaks off later gives what was read, with [`Info::complete`] false.
pub fn scan(bytes: &[u8], max_frames: usize) -> Option<Info> {
    let b = bytes;
    if !sniff(b) {
        return None;
    }
    // Logical Screen Descriptor: width, height, packed fields, background, aspect.
    let (width, height, packed) = (u16_at(b, 6)?, u16_at(b, 8)?, *b.get(10)?);
    b.get(12)?;
    let mut info = Info { width, height, global_table: packed & 0x80 != 0, frames: Vec::new(), complete: false, loop_count: None };
    let mut at = 13 + table_len(packed);
    // What the last Graphic Control Extension said, for the next image.
    let (mut delay_cs, mut transparent, mut disposal) = (0u16, None, 0u8);
    // Each step either finishes the walk or moves `at` forward, so this ends with the file.
    let mut step = || -> Option<bool> {
        match *b.get(at)? {
            // Extension: label, then data sub-blocks up to a zero-length one.
            0x21 => {
                let label = *b.get(at + 1)?;
                let body = at + 2;
                // Graphic Control Extension: block size 4, packed (bit 0: transparent colour
                // given; bits 2–4: disposal), delay (2 bytes), transparent colour index.
                if label == 0xF9 && *b.get(body)? == 4 {
                    let flags = *b.get(body + 1)?;
                    delay_cs = u16_at(b, body + 2)?;
                    transparent = (flags & 0x01 != 0).then_some(*b.get(body + 4)?);
                    disposal = (flags >> 2) & 7;
                }
                // Application Extension NETSCAPE2.0 (or ANIMEXTS1.0): sub-block 3, 1, loop count.
                if label == 0xFF
                    && matches!(b.get(body..body + 12), Some([11, id @ ..]) if id == b"NETSCAPE2.0" || id == b"ANIMEXTS1.0")
                    && b.get(body + 12..body + 14) == Some(&[3, 1])
                {
                    info.loop_count = u16_at(b, body + 14);
                }
                at = skip_sub_blocks(b, body)?;
                Some(true)
            }
            // Image Descriptor: separator, left, top, width, height, packed fields, then the
            // Local Color Table if bit 7 is set, the LZW code size and the data sub-blocks.
            0x2C => {
                if info.frames.len() >= max_frames {
                    return None;
                }
                let packed = *b.get(at + 9)?;
                info.frames.push(Frame {
                    offset: at,
                    left: u16_at(b, at + 1)?,
                    top: u16_at(b, at + 3)?,
                    width: u16_at(b, at + 5)?,
                    height: u16_at(b, at + 7)?,
                    delay_cs,
                    transparent,
                    disposal,
                    local_table: packed & 0x80 != 0,
                });
                (delay_cs, transparent, disposal) = (0, None, 0);
                at = skip_sub_blocks(b, (at + 10).checked_add(table_len(packed))?.checked_add(1)?)?;
                Some(true)
            }
            // Trailer.
            0x3B => {
                info.complete = true;
                Some(false)
            }
            // Anything else: the blocks break off here.
            _ => None,
        }
    };
    while step() == Some(true) {}
    Some(info)
}

/// The colours of the table the first frame uses (its Local Color Table when it has one, else
/// the Global one) and its transparent entry. `None` for anything that is not a well-formed GIF
/// up to its first image, or a first frame without a table. A transparent index past the table
/// is no transparent entry.
pub fn palette(bytes: &[u8]) -> Option<(Vec<[u8; 3]>, Option<u8>)> {
    let info = scan(bytes, 1)?;
    let first = info.frames.first()?;
    let (at, packed) = if first.local_table { (first.offset + 10, *bytes.get(first.offset + 9)?) } else { (13, *bytes.get(10)?) };
    let colors = bytes.get(at..at.checked_add(table_len(packed))?)?.as_chunks::<3>().0.to_vec();
    if colors.is_empty() {
        return None;
    }
    let transparent = first.transparent.filter(|i| usize::from(*i) < colors.len());
    Some((colors, transparent))
}

/// Whether a file of `file_len` bytes can plausibly hold `declared_bytes` of decoded picture: at
/// most `max_ratio` decoded bytes for each file byte, plus 1 MiB. A decoder that sets up the whole
/// logical screen before it reads a frame should be given only files that pass this.
pub fn plausible(declared_bytes: u64, file_len: usize, max_ratio: u64) -> bool {
    declared_bytes <= (file_len as u64).saturating_mul(max_ratio).saturating_add(1 << 20)
}

impl Info {
    /// The decoded bytes the file must plausibly back at `bytes_per_pixel`: the first frame, and
    /// whatever of the logical screen exceeds `unbacked_canvas` bytes (the screen around the first
    /// frame needs no data, so that much of it is free). Use with [`plausible`].
    pub fn bytes_to_back(&self, bytes_per_pixel: u64, unbacked_canvas: u64) -> u64 {
        let screen = u64::from(self.width).saturating_mul(u64::from(self.height)).saturating_mul(bytes_per_pixel);
        match self.frames.first() {
            Some(f) => {
                u64::from(f.width).saturating_mul(u64::from(f.height)).saturating_mul(bytes_per_pixel).max(screen.saturating_sub(unbacked_canvas))
            }
            None => screen,
        }
    }
}
