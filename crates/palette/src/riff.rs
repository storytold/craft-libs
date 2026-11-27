//! RIFF palette (.pal) reading.
//!
//! A RIFF palette is a `RIFF` file of form `PAL `. Its `data` chunk holds a version word, an
//! entry count and four bytes per entry (red, green, blue, flags), all little-endian. Other
//! chunks may come before `data`; each chunk is padded to an even length.

/// The `PALETTEENTRY` flag that makes an entry an index into the display's hardware palette
/// instead of a colour.
const PC_EXPLICIT: u8 = 0x02;

/// Reads a RIFF palette's colours in file order, flags dropped.
///
/// Chunks other than `data` are skipped, and only chunks inside the form's declared size count,
/// so bytes after the form are ignored. A file that isn't a RIFF palette, a form or chunk that
/// runs past the end (before or after `data`), a `data` chunk with fewer entries than its count, a
/// form without a `data` chunk and a palette without colours are errors. An entry flagged
/// `PC_EXPLICIT` holds a hardware palette index, which an RGB colour can't stand for, so it is an
/// error too. Never panics. The count is a 16-bit word, so at most 65 535 colours are returned.
pub fn parse_riff_pal(bytes: &[u8]) -> Result<Vec<[u8; 3]>, String> {
    if bytes.get(..4) != Some(b"RIFF".as_slice()) || bytes.get(8..12) != Some(b"PAL ".as_slice()) {
        return Err("this isn't a RIFF palette (.pal) file".into());
    }
    let le32 = |b: &[u8]| b.first_chunk::<4>().and_then(|w| usize::try_from(u32::from_le_bytes(*w)).ok());
    // The form's size counts its `PAL ` type and its chunks.
    let size = bytes.get(4..8).and_then(le32).ok_or_else(cut_short)?;
    let mut rest = 8usize.checked_add(size).and_then(|end| bytes.get(12..end)).ok_or_else(cut_short)?;
    let mut colors = None;
    while let (Some(id), Some(len)) = (rest.get(..4), rest.get(4..8).and_then(le32)) {
        let end = 8usize.checked_add(len).ok_or_else(cut_short)?;
        let body = rest.get(8..end).ok_or_else(cut_short)?;
        // The first `data` chunk gives the colours; the chunks after it are still checked.
        if id == b"data" && colors.is_none() {
            colors = Some(data_colors(body)?);
        }
        // Chunks are padded to an even length; a pad byte the form leaves out ends it.
        rest = end.checked_add(len & 1).and_then(|next| rest.get(next..)).unwrap_or_default();
    }
    colors.ok_or_else(|| "the palette has no colour data".into())
}

fn cut_short() -> String {
    "the palette is cut short".into()
}

/// The colours of a `data` chunk's body: a version word, an entry count and the entries.
fn data_colors(body: &[u8]) -> Result<Vec<[u8; 3]>, String> {
    let [_, _, lo, hi, entries @ ..] = body else { return Err(cut_short()) };
    let count = usize::from(u16::from_le_bytes([*lo, *hi]));
    let colors = entries.as_chunks::<4>().0.iter().take(count).enumerate().map(|(i, &[r, g, b, flags])| {
        if flags & PC_EXPLICIT == 0 {
            Ok([r, g, b])
        } else {
            Err(format!("colour {} of the palette is a hardware palette index, not a colour", i + 1))
        }
    });
    let colors = colors.collect::<Result<Vec<[u8; 3]>, String>>()?;
    if colors.len() < count {
        return Err(cut_short());
    }
    if colors.is_empty() {
        return Err("the palette has no colours in it".into());
    }
    Ok(colors)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    /// `n` distinct colours.
    fn colors(n: usize) -> Vec<[u8; 3]> {
        (0..n).map(|i| [(i % 256) as u8, (i / 256) as u8 * 100 + 7, 255 - (i % 256) as u8]).collect()
    }

    /// A RIFF palette declaring `count` entries and holding `colors`, with an unrelated chunk of
    /// odd length (so padded) before `data`.
    fn pal(colors: &[[u8; 3]], count: u16) -> Vec<u8> {
        let mut data = vec![0x00, 0x03];
        data.extend_from_slice(&count.to_le_bytes());
        for c in colors {
            data.extend_from_slice(&[c[0], c[1], c[2], 0x55]);
        }
        let mut body = b"PAL ".to_vec();
        body.extend_from_slice(b"note");
        body.extend_from_slice(&3u32.to_le_bytes());
        body.extend_from_slice(b"abc\0");
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(data.len() as u32).to_le_bytes());
        body.extend_from_slice(&data);
        let mut b = b"RIFF".to_vec();
        b.extend_from_slice(&(body.len() as u32).to_le_bytes());
        b.extend_from_slice(&body);
        b
    }

    #[test]
    fn colours_come_in_file_order_past_other_chunks() {
        let want = colors(300);
        assert_eq!(parse_riff_pal(&pal(&want, 300)).unwrap(), want);
        // Entries past the count and bytes after the form don't count.
        let mut extra = pal(&want, 3);
        extra.extend_from_slice(b"junk after the form");
        assert_eq!(parse_riff_pal(&extra).unwrap(), want[..3]);
    }

    #[test]
    fn an_entry_that_indexes_the_hardware_palette_is_refused() {
        let want = colors(3);
        let mut bytes = pal(&want, 3);
        assert_eq!(parse_riff_pal(&bytes).unwrap(), want);
        // Past the chunk header, the version and count words and the first entry: the second
        // entry's flags byte. The other flags `pal` sets (reserved, no-collapse) still read.
        let at = bytes.windows(4).position(|w| w == b"data").unwrap() + 8 + 4 + 4 + 3;
        bytes[at] |= PC_EXPLICIT;
        assert!(parse_riff_pal(&bytes).is_err());
    }

    #[test]
    fn damaged_or_empty_palettes_are_refused() {
        let three = colors(3);
        let mut no_data = pal(&three, 3);
        let at = no_data.windows(4).position(|w| w == b"data").unwrap();
        no_data[at..at + 4].copy_from_slice(b"junk");
        // A form declared as only its `PAL ` type, with the colour data after it, outside the form.
        let mut empty_form = pal(&three, 3);
        empty_form[4..8].copy_from_slice(&4u32.to_le_bytes());
        // A form declared one byte longer than the file.
        let mut past_end = pal(&three, 3);
        let size = past_end.len() as u32 - 7;
        past_end[4..8].copy_from_slice(&size.to_le_bytes());
        // A chunk whose length runs to the top of the address range.
        let mut huge_chunk = pal(&three, 3);
        huge_chunk[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        let mut not_pal = pal(&three, 3);
        not_pal[8..12].copy_from_slice(b"WAVE");
        // A chunk after `data`, inside the form, whose body runs past the end.
        let mut after_data = pal(&three, 3);
        after_data.extend_from_slice(b"JUNK");
        after_data.extend_from_slice(&100u32.to_le_bytes());
        let size = after_data.len() as u32 - 8;
        after_data[4..8].copy_from_slice(&size.to_le_bytes());
        let cases = [
            ("empty", Vec::new()),
            ("text", b"JASC-PAL\r\n0100\r\n1\r\n255 0 0\r\n".to_vec()),
            ("not PAL", not_pal),
            ("no colours", pal(&[], 0)),
            ("short", pal(&three[..2], 3)),
            ("no data", no_data),
            ("empty form", empty_form),
            ("past end", past_end),
            ("huge chunk", huge_chunk),
            ("chunk past end after data", after_data),
        ];
        for (name, bytes) in cases {
            assert!(parse_riff_pal(&bytes).is_err(), "{name} should be refused");
        }
        // Every truncation of a valid palette is refused.
        let whole = pal(&three, 3);
        for n in 0..whole.len() {
            assert!(parse_riff_pal(&whole[..n]).is_err(), "cut to {n} bytes");
        }
    }
}
