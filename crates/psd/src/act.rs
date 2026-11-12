//! Photoshop Color Table files (`.act`): raw 8-bit RGB palettes.
//!
//! Layout from Adobe's public "Photoshop File Formats Specification" (Color Table): 256 RGB
//! triplets (768 bytes), optionally followed by a 4-byte trailer: a big-endian `u16` color
//! count ("the number of colors to use") and a big-endian `u16` transparent color index.
//! There are no names, no groups and no other color spaces. The trailer is kept verbatim so a
//! read → write round trip never loses it; interpreting it is the caller's job.

use crate::error::{PsdError, Result};
use crate::io::{Reader, WriteExt};

/// Colors in a `.act` palette.
pub const COLOR_COUNT: usize = 256;
/// Palette bytes: 256 RGB triplets.
pub const PALETTE_LEN: usize = COLOR_COUNT * 3;
/// Trailer bytes (color count + transparent index).
pub const TRAILER_LEN: usize = 4;

/// A parsed `.act` file: the full 256-color palette plus the optional trailer, kept verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActFile {
    /// All 256 palette colors as RGB triplets.
    pub colors: [[u8; 3]; COLOR_COUNT],
    /// Trailer color count ("the number of colors to use"), if the file has one.
    pub count: Option<u16>,
    /// Trailer transparent color index, if the file has one.
    pub transparent: Option<u16>,
}

impl ActFile {
    /// Colors to use: the trailer count clamped to the palette, else the whole palette.
    pub fn used(&self) -> &[[u8; 3]] {
        let n = self.count.map_or(COLOR_COUNT, |c| (c as usize).min(COLOR_COUNT));
        &self.colors[..n]
    }

    /// The transparent color index, if the trailer names one inside the used colors.
    pub fn transparent_index(&self) -> Option<usize> {
        let t = usize::from(self.transparent?);
        (t < self.used().len()).then_some(t)
    }
}

/// Parse an `.act` file. Anything but 768 bytes (palette only) or 772 bytes (plus the
/// trailer) is an error; never panics.
pub fn parse(data: &[u8]) -> Result<ActFile> {
    if data.len() != PALETTE_LEN && data.len() != PALETTE_LEN + TRAILER_LEN {
        return Err(PsdError::invalid("an .act file is 768 bytes, or 772 with the count trailer"));
    }
    let mut r = Reader::new(data);
    let raw = r.bytes(PALETTE_LEN)?;
    let mut colors = [[0u8; 3]; COLOR_COUNT];
    for (i, c) in colors.iter_mut().enumerate() {
        let o = i * 3;
        c.copy_from_slice(&raw[o..o + 3]);
    }
    let (count, transparent) = if r.remaining() >= TRAILER_LEN { (Some(r.u16()?), Some(r.u16()?)) } else { (None, None) };
    Ok(ActFile { colors, count, transparent })
}

/// Write an `.act` file: the 256-color palette plus the trailer when `count` or `transparent` is given.
/// Short palettes are padded with black; fails on a palette longer than 256 colors.
pub fn write(colors: &[[u8; 3]], count: Option<u16>, transparent: Option<u16>) -> Result<Vec<u8>> {
    if colors.len() > COLOR_COUNT {
        return Err(PsdError::LimitExceeded("an .act file holds at most 256 colors"));
    }
    let mut out = Vec::with_capacity(PALETTE_LEN + TRAILER_LEN);
    for c in colors {
        out.extend_from_slice(c);
    }
    out.resize(PALETTE_LEN, 0);
    if count.is_some() || transparent.is_some() {
        out.put_u16(count.unwrap_or(COLOR_COUNT as u16));
        out.put_u16(transparent.unwrap_or(0xFFFF));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> [[u8; 3]; COLOR_COUNT] {
        let mut colors = [[0u8; 3]; COLOR_COUNT];
        for (i, c) in colors.iter_mut().enumerate() {
            let v = i as u8;
            *c = [v, v.wrapping_mul(7), v.wrapping_add(13)];
        }
        colors[0] = [255, 255, 255];
        colors
    }

    #[test]
    fn round_trips_with_and_without_the_trailer() {
        let colors = sample();
        let bare = write(&colors, None, None).unwrap();
        assert_eq!(bare.len(), PALETTE_LEN);
        let f = parse(&bare).unwrap();
        assert_eq!((f.count, f.transparent), (None, None));
        assert_eq!(f.used(), &colors);
        assert_eq!(f.transparent_index(), None);
        assert_eq!(write(&f.colors, f.count, f.transparent).unwrap(), bare, "byte-stable");

        let trailer = write(&colors, Some(40), Some(7)).unwrap();
        assert_eq!(trailer.len(), PALETTE_LEN + TRAILER_LEN);
        let f = parse(&trailer).unwrap();
        assert_eq!((f.count, f.transparent), (Some(40), Some(7)));
        assert_eq!(f.used().len(), 40);
        assert_eq!(f.transparent_index(), Some(7));
        assert_eq!(write(&f.colors, f.count, f.transparent).unwrap(), trailer, "byte-stable");
    }

    #[test]
    fn short_palettes_pad_and_long_ones_fail() {
        let few = vec![[1u8, 2, 3]; 3];
        let bytes = write(&few, None, None).unwrap();
        assert_eq!(bytes.len(), PALETTE_LEN);
        assert_eq!(&bytes[..9], &[1u8, 2, 3, 1, 2, 3, 1, 2, 3]);
        assert_eq!(&bytes[9..12], &[0u8, 0, 0]);
        let many = vec![[0u8; 3]; COLOR_COUNT + 1];
        assert!(write(&many, None, None).is_err());
    }

    #[test]
    fn hostile_lengths_are_errors_not_panics() {
        let good = write(&sample(), Some(256), Some(255)).unwrap();
        assert_eq!(good.len(), PALETTE_LEN + TRAILER_LEN);
        for n in 0..good.len() {
            // Only the two exact lengths parse (768 reads as a trailer-less palette).
            if n == PALETTE_LEN {
                assert!(parse(&good[..n]).is_ok(), "{n}");
            } else {
                assert!(parse(&good[..n]).is_err(), "{n}");
            }
        }
        assert!(parse(&[]).is_err());
        assert!(parse(&vec![0u8; PALETTE_LEN + TRAILER_LEN + 1]).is_err(), "trailing byte");
        // A count past the palette clamps; a transparent index past it is none.
        let f = parse(&write(&sample(), Some(1000), Some(999)).unwrap()).unwrap();
        assert_eq!(f.used().len(), COLOR_COUNT);
        assert_eq!(f.transparent_index(), None);
    }
}
