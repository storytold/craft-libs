//! Windows `.ico` files of PNG images.
//!
//! The file starts with a 6-byte header (reserved 0, type 1 for an icon, the image count) and one
//! 16-byte directory entry per image (width, height, palette size 0, reserved 0, 1 plane, 32 bits per
//! pixel, the PNG's byte length and its offset from the start of the file), all little-endian. The
//! PNGs follow in the order of the entries. A side of 256 px is written as 0 in the one-byte width
//! and height. Each PNG must be 8-bit RGBA, the 32 bits per pixel that Windows reads from a PNG entry
//! ([ICO format, part 4](https://devblogs.microsoft.com/oldnewthing/20101022-00/?p=12473/)).

use std::path::Path;

use super::{Error, Limits, png_info, read_bounded, reserve, sorted_sizes};

/// The most images an ICO file holds: its image count is 16 bits.
pub const MAX_IMAGES: usize = u16::MAX as usize;

/// The largest image side, in pixels.
pub const MAX_SIZE: u32 = 256;

/// The header and one directory entry, in bytes.
const HEADER: u64 = 6;
const ENTRY: u64 = 16;

/// An ICO file holding `pngs`: each square, 1 to 256 px, 8-bit RGBA and a different size from the
/// others. The entries are written in increasing size, whatever the order of `pngs`.
pub fn pack<P: AsRef<[u8]>>(pngs: &[P], limits: &Limits) -> Result<Vec<u8>, Error> {
    let sizes = sorted_sizes(pngs, limits, MAX_IMAGES, |info| {
        if info.width > MAX_SIZE {
            return Err(Error::UnsupportedSize { format: "ICO", size: info.width });
        }
        if !info.is_rgba8() {
            return Err(Error::InvalidPng(format!("ICO images must be 8-bit RGBA, got color type {} at {} bits", info.color_type, info.bit_depth)));
        }
        Ok(())
    })?;
    let dir_end = HEADER + ENTRY * sizes.len() as u64;
    let total = pngs.iter().fold(dir_end, |t, png| t.saturating_add(png.as_ref().len() as u64));
    let mut out = reserve(total, limits)?;
    // `reserve` checked that the whole file, and so every length and offset, fits in a u32, and
    // `sorted_sizes` that the count fits in a u16.
    out.extend_from_slice(&0u16.to_le_bytes()); // reserved
    out.extend_from_slice(&1u16.to_le_bytes()); // type: icon
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = dir_end;
    for &(size, i) in &sizes {
        let len = pngs[i].as_ref().len() as u64;
        let dim = if size == MAX_SIZE { 0 } else { size as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]); // width, height, palette size, reserved
        out.extend_from_slice(&1u16.to_le_bytes()); // color planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bits per pixel
        out.extend_from_slice(&(len as u32).to_le_bytes());
        out.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += len;
    }
    for &(_, i) in &sizes {
        out.extend_from_slice(pngs[i].as_ref());
    }
    Ok(out)
}

/// One image of an ICO file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The side in pixels, with a stored 0 read as 256.
    pub size: u32,
    /// The PNG.
    pub png: Vec<u8>,
}

/// The images of an ICO file of PNG entries, in directory order.
///
/// The checks are the header (reserved 0, type 1, 1 to [`Limits::max_images`] images), the file
/// size against [`Limits::max_total_bytes`], and for each entry: a PNG of at most
/// [`Limits::max_png_bytes`], square dimensions, palette size 0, reserved 0, 1 plane and 32 bits per
/// pixel, as [`pack`] writes them, a size no other entry has, and an 8-bit RGBA PNG that
/// [`png_info`](super::png_info) accepts, of the size the entry states, lying wholly after the
/// directory and inside the file. The PNGs together hold at most [`Limits::max_total_bytes`].
pub fn read(ico: &[u8], limits: &Limits) -> Result<Vec<Entry>, Error> {
    let bad = Error::InvalidIcon;
    if ico.len() > limits.max_total_bytes {
        return Err(Error::TooLarge(format!("the icon file holds {} bytes, more than {}", ico.len(), limits.max_total_bytes)));
    }
    let [0, 0, 1, 0, c0, c1, ..] = ico else {
        return Err(bad("no ICO header (reserved 0, type 1)".into()));
    };
    let count = usize::from(u16::from_le_bytes([*c0, *c1]));
    if count == 0 {
        return Err(bad("no images".into()));
    }
    if count > limits.max_images {
        return Err(Error::TooManyImages { count, max: limits.max_images });
    }
    let dir_end = 6 + 16 * count;
    let dir = ico.get(6..dir_end).ok_or_else(|| bad(format!("the directory of {count} entries is truncated")))?;
    let mut out = Vec::with_capacity(count);
    let mut copied = 0usize;
    for (n, e) in dir.as_chunks::<16>().0.iter().enumerate() {
        let (w, h) = (e[0], e[1]);
        if w != h {
            return Err(bad(format!("entry {} is {w}x{h}, not square", n + 1)));
        }
        if e[2..8] != [0, 0, 1, 0, 32, 0] {
            return Err(bad(format!("entry {} is not palette size 0, reserved 0, 1 plane and 32 bits per pixel", n + 1)));
        }
        let size = if w == 0 { MAX_SIZE } else { u32::from(w) };
        let len = u32::from_le_bytes([e[8], e[9], e[10], e[11]]) as usize;
        let offset = u32::from_le_bytes([e[12], e[13], e[14], e[15]]) as usize;
        if len > limits.max_png_bytes {
            return Err(Error::TooLarge(format!("entry {} holds {len} bytes, more than {}", n + 1, limits.max_png_bytes)));
        }
        let png = offset
            .checked_add(len)
            .filter(|_| offset >= dir_end)
            .and_then(|end| ico.get(offset..end))
            .ok_or_else(|| bad(format!("entry {} points outside the file", n + 1)))?;
        let info = png_info(png)?;
        if (info.width, info.height) != (size, size) {
            return Err(bad(format!("entry {} says {size} px but its PNG is {}x{}", n + 1, info.width, info.height)));
        }
        if !info.is_rgba8() {
            return Err(bad(format!("the PNG of entry {} is not 8-bit RGBA", n + 1)));
        }
        if out.iter().any(|e: &Entry| e.size == size) {
            return Err(Error::DuplicateSize(size));
        }
        // Entries can point at overlapping bytes, so the copies are counted against the limit.
        copied = copied.saturating_add(len);
        if copied > limits.max_total_bytes {
            return Err(Error::TooLarge(format!("the entries hold more than {} bytes together", limits.max_total_bytes)));
        }
        out.push(Entry { size, png: png.to_vec() });
    }
    Ok(out)
}

/// [`read`] on the file at `path`, which is read only up to [`Limits::max_total_bytes`]. An app's
/// test of its committed icon passes the icon's path.
pub fn read_file(path: &Path, limits: &Limits) -> Result<Vec<Entry>, Error> {
    read(&read_bounded(path, limits.max_total_bytes)?, limits)
}

#[cfg(test)]
mod tests {
    use super::super::tests::{png_of, push_chunk, rgba};
    use super::*;

    fn u32_le(b: &[u8]) -> u32 {
        u32::from_le_bytes(b.try_into().unwrap())
    }

    #[test]
    fn packs_directory_and_payloads() {
        let (a, b) = (rgba(16), rgba(256));
        let ico = pack(&[a.clone(), b.clone()], &Limits::default()).unwrap();
        // Header and entries, byte for byte from the documented layout.
        let (la, lb) = ((a.len() as u32).to_le_bytes(), (b.len() as u32).to_le_bytes());
        let off_b = (38 + a.len() as u32).to_le_bytes();
        let mut want = vec![0, 0, 1, 0, 2, 0];
        want.extend_from_slice(&[16, 16, 0, 0, 1, 0, 32, 0]);
        want.extend_from_slice(&la);
        want.extend_from_slice(&[38, 0, 0, 0]);
        want.extend_from_slice(&[0, 0, 0, 0, 1, 0, 32, 0]); // 256 px is stored as 0
        want.extend_from_slice(&lb);
        want.extend_from_slice(&off_b);
        assert_eq!(&ico[..38], &want[..]);
        assert_eq!(u32_le(&ico[34..38]), 38 + a.len() as u32);
        assert_eq!(&ico[38..38 + a.len()], &a[..]);
        assert_eq!(&ico[38 + a.len()..], &b[..]);
    }

    #[test]
    fn writes_entries_in_increasing_size() {
        let (a, b, c) = (rgba(48), rgba(16), rgba(32));
        let ico = pack(&[&a[..], &b[..], &c[..]], &Limits::default()).unwrap();
        let sizes: Vec<u32> = read(&ico, &Limits::default()).unwrap().iter().map(|e| e.size).collect();
        assert_eq!(sizes, [16, 32, 48]);
        assert_eq!(ico, pack(&[b, c, a], &Limits::default()).unwrap());
    }

    #[test]
    fn rejects_bad_input() {
        let limits = Limits::default();
        assert_eq!(pack::<Vec<u8>>(&[], &limits), Err(Error::Empty));
        assert_eq!(pack(&[rgba(512)], &limits), Err(Error::UnsupportedSize { format: "ICO", size: 512 }));
        assert!(matches!(pack(&[b"GIF89a".to_vec()], &limits), Err(Error::InvalidPng(_))));
        assert_eq!(pack(&[png_of(16, 32, 8, 6)], &limits), Err(Error::NotSquare { width: 16, height: 32 }));
        assert_eq!(pack(&[rgba(16), rgba(32), rgba(16)], &limits), Err(Error::DuplicateSize(16)));
        assert_eq!(pack(&[rgba(16), rgba(32)], &Limits { max_images: 1, ..limits }), Err(Error::TooManyImages { count: 2, max: 1 }));
        assert!(matches!(pack(&[rgba(16)], &Limits { max_png_bytes: 10, ..limits }), Err(Error::TooLarge(_))));
        assert!(matches!(pack(&[rgba(16)], &Limits { max_total_bytes: 40, ..limits }), Err(Error::TooLarge(_))));
    }

    #[test]
    fn rejects_a_png_that_is_not_rgba8() {
        let limits = Limits::default();
        for (depth, color) in [(8, 2), (16, 6), (8, 3), (8, 0), (8, 4)] {
            assert!(matches!(pack(&[png_of(16, 16, depth, color)], &limits), Err(Error::InvalidPng(m)) if m.contains("RGBA")));
        }
    }

    #[test]
    fn reads_what_it_packs() {
        let pngs = [rgba(1), rgba(16), rgba(256)];
        let ico = pack(&pngs, &Limits::default()).unwrap();
        let entries = read(&ico, &Limits::default()).unwrap();
        let got: Vec<(u32, &[u8])> = entries.iter().map(|e| (e.size, &e.png[..])).collect();
        assert_eq!(got, [(1, &pngs[0][..]), (16, &pngs[1][..]), (256, &pngs[2][..])]);
    }

    #[test]
    fn read_rejects_bad_files() {
        let limits = Limits::default();
        let ico = pack(&[rgba(16), rgba(32)], &limits).unwrap();
        assert!(matches!(read(b"GIF89a", &limits), Err(Error::InvalidIcon(_))));
        assert!(matches!(read(&ico[..30], &limits), Err(Error::InvalidIcon(_))));
        assert!(matches!(read(&ico[..ico.len() - 1], &limits), Err(Error::InvalidIcon(_))));
        let mut wrong_size = ico.clone();
        wrong_size[6] = 17;
        wrong_size[7] = 17;
        assert!(matches!(read(&wrong_size, &limits), Err(Error::InvalidIcon(m)) if m.contains("17 px")));
        let mut into_directory = ico.clone();
        into_directory[18..22].copy_from_slice(&6u32.to_le_bytes());
        assert!(matches!(read(&into_directory, &limits), Err(Error::InvalidIcon(_))));
        for (at, value) in [(8, 1), (9, 1), (10, 0), (12, 24)] {
            let mut field = ico.clone();
            field[at] = value;
            assert!(matches!(read(&field, &limits), Err(Error::InvalidIcon(m)) if m.contains("32 bits")), "accepted byte {at} = {value}");
        }
        let mut gray = ico.clone();
        let png = png_of(16, 16, 8, 0);
        gray.splice(38..38 + rgba(16).len(), png.iter().copied());
        let delta = png.len() as i64 - rgba(16).len() as i64;
        gray[14..18].copy_from_slice(&(png.len() as u32).to_le_bytes());
        let off = (u32_le(&gray[34..38]) as i64 + delta) as u32;
        gray[34..38].copy_from_slice(&off.to_le_bytes());
        assert!(matches!(read(&gray, &limits), Err(Error::InvalidIcon(m)) if m.contains("RGBA")));
        let mut huge = ico.clone();
        huge[14..18].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(matches!(read(&huge, &limits), Err(Error::TooLarge(_))));
        assert!(matches!(read(&huge, &Limits { max_png_bytes: usize::MAX, ..limits }), Err(Error::InvalidIcon(_))));
        assert_eq!(read(&ico, &Limits { max_images: 1, ..limits }), Err(Error::TooManyImages { count: 2, max: 1 }));
        let small = Limits { max_png_bytes: rgba(32).len() - 1, ..limits };
        assert!(matches!(read(&ico, &small), Err(Error::TooLarge(m)) if m.contains("entry 2")));
        // Entry 2 made a second 16 px entry that points at entry 1's PNG.
        let mut shared = ico.clone();
        shared[22..24].copy_from_slice(&[16, 16]);
        shared.copy_within(14..22, 30);
        assert_eq!(read(&shared, &limits), Err(Error::DuplicateSize(16)));
    }

    #[test]
    fn read_counts_overlapping_entries_against_the_limit() {
        // A 32 px PNG that carries a whole 16 px PNG in an ancillary chunk, and a 16 px entry that
        // points inside it.
        let (inner, outer) = (rgba(16), rgba(32));
        let mut png = outer[..33].to_vec();
        push_chunk(&mut png, b"zzZz", &inner);
        png.extend_from_slice(&outer[33..]);
        let mut ico = vec![0, 0, 1, 0, 2, 0];
        for (size, len, offset) in [(16u8, inner.len(), 38 + 33 + 8), (32, png.len(), 38)] {
            ico.extend_from_slice(&[size, size, 0, 0, 1, 0, 32, 0]);
            ico.extend_from_slice(&(len as u32).to_le_bytes());
            ico.extend_from_slice(&(offset as u32).to_le_bytes());
        }
        ico.extend_from_slice(&png);
        let fits = Limits { max_total_bytes: ico.len(), ..Limits::default() };
        assert!(matches!(read(&ico, &fits), Err(Error::TooLarge(m)) if m.contains("together")));
        let sizes: Vec<u32> = read(&ico, &Limits::default()).unwrap().iter().map(|e| e.size).collect();
        assert_eq!(sizes, [16, 32]);
    }

    #[test]
    fn committed_icon_is_valid() {
        // An app passes its own committed icon, for example `<root>/assets/app-icon/<app>.ico`.
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("app.ico");
        let limits = Limits::default();
        let pngs = [16, 24, 32, 48, 64, 128, 256].map(rgba);
        std::fs::write(&path, pack(&pngs, &limits).unwrap()).expect("write the icon");
        let entries = read_file(&path, &limits).unwrap();
        assert!(entries.len() >= 4, "expected several sizes in {}", path.display());
        assert!(matches!(read_file(&path, &Limits { max_total_bytes: 64, ..limits }), Err(Error::TooLarge(_))));
    }
}
