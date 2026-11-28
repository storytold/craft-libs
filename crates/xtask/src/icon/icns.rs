//! macOS `.icns` files of PNG images, written on any OS.
//!
//! The file starts with the magic `icns` and the file's length, followed by elements: a four-letter
//! type, the element's length including its 8-byte header, and a PNG. Both lengths are big-endian
//! ([ICNS format](https://hexbase.dev/formats/icns/)). Each PNG fills every slot of its pixel size in
//! [`SLOTS`]: a 32 px image is both the 32 pt icon (`icp5`) and the 16 pt @2x one (`ic11`).

use std::path::Path;

use super::{Error, Limits, read_bounded, reserve, sorted_sizes};

/// The PNG element types, by pixel size, in the order they are written.
pub const SLOTS: [(u32, [u8; 4]); 11] = [
    (16, *b"icp4"),
    (32, *b"icp5"),
    (32, *b"ic11"), // 16 @2x
    (64, *b"icp6"),
    (64, *b"ic12"), // 32 @2x
    (128, *b"ic07"),
    (256, *b"ic08"),
    (256, *b"ic13"), // 128 @2x
    (512, *b"ic09"),
    (512, *b"ic14"),  // 256 @2x
    (1024, *b"ic10"), // 512 @2x
];

/// The number of different sizes in [`SLOTS`], and so the most images an `.icns` file holds.
pub const MAX_IMAGES: usize = 7;

/// The most elements [`read`] returns for each image [`Limits::max_images`] allows. A file from
/// [`pack`] holds at most 2 per image, and other tools also write a table of contents and other
/// elements that are not images.
pub const ELEMENTS_PER_IMAGE: usize = 4;

/// The magic and length at the start of the file, and the type and length of each element, in bytes.
const HEADER: u64 = 8;

/// The element types of [`SLOTS`] that an image of `size` px fills.
fn types(size: u32) -> impl Iterator<Item = &'static [u8; 4]> {
    let slots: &'static [(u32, [u8; 4]); 11] = &SLOTS;
    slots.iter().filter(move |(s, _)| *s == size).map(|(_, kind)| kind)
}

/// An `.icns` file holding `pngs`: each square, one of the sizes in [`SLOTS`] and a different size
/// from the others. The elements are written in increasing size, whatever the order of `pngs`, and
/// in [`SLOTS`] order within a size.
pub fn pack<P: AsRef<[u8]>>(pngs: &[P], limits: &Limits) -> Result<Vec<u8>, Error> {
    let sizes = sorted_sizes(pngs, limits, MAX_IMAGES, |info| match types(info.width).next() {
        Some(_) => Ok(()),
        None => Err(Error::UnsupportedSize { format: "ICNS", size: info.width }),
    })?;
    let total = sizes.iter().fold(HEADER, |t, &(size, i)| {
        let element = HEADER.saturating_add(pngs[i].as_ref().len() as u64);
        t.saturating_add(element.saturating_mul(types(size).count() as u64))
    });
    let mut out = reserve(total, limits)?;
    // `reserve` checked that the whole file, and so every element, fits in a u32.
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&(total as u32).to_be_bytes());
    for &(size, i) in &sizes {
        let png = pngs[i].as_ref();
        let len = (HEADER + png.len() as u64) as u32;
        for kind in types(size) {
            out.extend_from_slice(kind);
            out.extend_from_slice(&len.to_be_bytes());
            out.extend_from_slice(png);
        }
    }
    Ok(out)
}

/// One element of an `.icns` file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    /// The four-letter type, for example `ic10`.
    pub kind: [u8; 4],
    /// The data after the 8-byte element header.
    pub data: Vec<u8>,
}

/// The elements of an `.icns` file, in file order.
///
/// The checks are the magic `icns`, a stated length equal to the file's length and at most
/// [`Limits::max_total_bytes`], 1 to [`ELEMENTS_PER_IMAGE`] times [`Limits::max_images`]
/// elements, and elements of at least 8 bytes, with at most [`Limits::max_png_bytes`] of data, that
/// end exactly at the end of the file. The data is returned as stored: files written by other tools also hold elements that are not
/// PNGs, such as a table of contents. Pass an element's data to [`png_info`](super::png_info) to check
/// it.
pub fn read(icns: &[u8], limits: &Limits) -> Result<Vec<Element>, Error> {
    let bad = Error::InvalidIcon;
    if icns.len() > limits.max_total_bytes {
        return Err(Error::TooLarge(format!("the icon file holds {} bytes, more than {}", icns.len(), limits.max_total_bytes)));
    }
    let [b'i', b'c', b'n', b's', l0, l1, l2, l3, ..] = icns else {
        return Err(bad("no icns header".into()));
    };
    let stated = u32::from_be_bytes([*l0, *l1, *l2, *l3]) as usize;
    if stated != icns.len() {
        return Err(bad(format!("the header says {stated} bytes but the file holds {}", icns.len())));
    }
    let max_elements = limits.max_images.saturating_mul(ELEMENTS_PER_IMAGE);
    let mut out = Vec::new();
    let mut at = 8;
    while at < icns.len() {
        if out.len() == max_elements {
            return Err(Error::TooLarge(format!("the icon file holds more than {max_elements} elements")));
        }
        let header = icns.get(at..at + 8).ok_or_else(|| bad(format!("truncated element at byte {at}")))?;
        let kind = [header[0], header[1], header[2], header[3]];
        let len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let data = at
            .checked_add(len)
            .filter(|_| len >= 8)
            .and_then(|end| icns.get(at + 8..end))
            .ok_or_else(|| bad(format!("the element at byte {at} has a bad length {len}")))?;
        if data.len() > limits.max_png_bytes {
            return Err(Error::TooLarge(format!("the element at byte {at} holds {} bytes, more than {}", data.len(), limits.max_png_bytes)));
        }
        out.push(Element { kind, data: data.to_vec() });
        at += len;
    }
    if out.is_empty() {
        return Err(bad("the icon file holds no elements".into()));
    }
    Ok(out)
}

/// [`read`] on the file at `path`, which is read only up to [`Limits::max_total_bytes`]. An app's
/// test of its committed icon passes the icon's path.
pub fn read_file(path: &Path, limits: &Limits) -> Result<Vec<Element>, Error> {
    read(&read_bounded(path, limits.max_total_bytes)?, limits)
}

#[cfg(test)]
mod tests {
    use super::super::png_size;
    use super::super::tests::{png_of, rgba};
    use super::*;

    fn kinds(icns: &[u8]) -> Vec<String> {
        read(icns, &Limits::default()).unwrap().iter().map(|e| String::from_utf8_lossy(&e.kind).into_owned()).collect()
    }

    #[test]
    fn fills_every_slot_of_each_size() {
        let (a, b) = (rgba(32), rgba(1024));
        let icns = pack(&[a.clone(), b.clone()], &Limits::default()).unwrap();
        assert_eq!(&icns[..4], b"icns");
        assert_eq!(u32::from_be_bytes(icns[4..8].try_into().unwrap()) as usize, icns.len());
        let e = read(&icns, &Limits::default()).unwrap();
        assert_eq!(kinds(&icns), ["icp5", "ic11", "ic10"]);
        assert_eq!(e[0].data, a);
        assert_eq!(e[1].data, a);
        assert_eq!(e[2].data, b);
    }

    #[test]
    fn packs_icns_elements() {
        let (a, b) = (rgba(16), rgba(32));
        let icns = pack(&[a.clone(), b.clone()], &Limits::default()).unwrap();
        // The header and the first element's header, byte for byte from the documented layout.
        let total = (8 + (8 + a.len()) + 2 * (8 + b.len())) as u32;
        let mut want = b"icns".to_vec();
        want.extend_from_slice(&total.to_be_bytes());
        want.extend_from_slice(b"icp4");
        want.extend_from_slice(&((a.len() + 8) as u32).to_be_bytes());
        assert_eq!(&icns[..16], &want[..]);
        assert_eq!(icns.len(), total as usize);
        let at = 16 + a.len();
        assert_eq!(&icns[at..at + 4], b"icp5");
        assert_eq!(u32::from_be_bytes(icns[at + 4..at + 8].try_into().unwrap()) as usize, b.len() + 8);
        assert_eq!(kinds(&icns), ["icp4", "icp5", "ic11"]);
    }

    #[test]
    fn writes_every_size_in_slot_order() {
        let sizes = [1024, 512, 256, 128, 64, 32, 16];
        let icns = pack(&sizes.map(rgba), &Limits::default()).unwrap();
        let want: Vec<String> = SLOTS.iter().map(|(_, k)| String::from_utf8_lossy(k).into_owned()).collect();
        assert_eq!(kinds(&icns), want);
        for e in read(&icns, &Limits::default()).unwrap() {
            let size = SLOTS.iter().find(|(_, k)| *k == e.kind).map(|(s, _)| *s).unwrap();
            assert_eq!(png_size(&e.data), Ok((size, size)));
        }
    }

    #[test]
    fn rejects_bad_input() {
        let limits = Limits::default();
        assert_eq!(pack::<Vec<u8>>(&[], &limits), Err(Error::Empty));
        assert_eq!(pack(&[rgba(48)], &limits), Err(Error::UnsupportedSize { format: "ICNS", size: 48 }));
        assert!(matches!(pack(&[b"GIF89a".to_vec()], &limits), Err(Error::InvalidPng(_))));
        assert_eq!(pack(&[png_of(32, 16, 8, 6)], &limits), Err(Error::NotSquare { width: 32, height: 16 }));
        assert_eq!(pack(&[rgba(32), rgba(32)], &limits), Err(Error::DuplicateSize(32)));
        // Each image fills two slots here, so the file holds twice its PNG bytes.
        let one = rgba(32).len() as u64;
        let tight = Limits { max_total_bytes: (8 + 2 * (8 + one) - 1) as usize, ..limits };
        assert!(matches!(pack(&[rgba(32)], &tight), Err(Error::TooLarge(_))));
        assert!(pack(&[rgba(32)], &Limits { max_total_bytes: tight.max_total_bytes + 1, ..limits }).is_ok());
    }

    #[test]
    fn read_rejects_bad_files() {
        let limits = Limits::default();
        let icns = pack(&[rgba(16)], &limits).unwrap();
        assert!(matches!(read(b"GIF89a", &limits), Err(Error::InvalidIcon(_))));
        assert!(matches!(read(b"icns\0\0\0\x08", &limits), Err(Error::InvalidIcon(m)) if m.contains("no elements")));
        assert!(matches!(read(&icns[..icns.len() - 1], &limits), Err(Error::InvalidIcon(_))));
        let mut short = icns.clone();
        short[12..16].copy_from_slice(&4u32.to_be_bytes());
        assert!(matches!(read(&short, &limits), Err(Error::InvalidIcon(_))));
        let mut long = icns.clone();
        long[12..16].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(matches!(read(&long, &limits), Err(Error::InvalidIcon(_))));
        assert!(matches!(read(&icns, &Limits { max_total_bytes: 8, ..limits }), Err(Error::TooLarge(_))));
        let small = Limits { max_png_bytes: rgba(16).len() - 1, ..limits };
        assert!(matches!(read(&icns, &small), Err(Error::TooLarge(m)) if m.contains("byte 8")));
    }

    #[test]
    fn read_stops_at_the_element_limit() {
        let empty_elements = |n: u32| {
            let mut f = b"icns".to_vec();
            f.extend_from_slice(&(8 + 8 * n).to_be_bytes());
            for _ in 0..n {
                f.extend_from_slice(b"TOC ");
                f.extend_from_slice(&8u32.to_be_bytes());
            }
            f
        };
        let one = Limits { max_images: 1, ..Limits::default() };
        assert_eq!(read(&empty_elements(4), &one).map(|e| e.len()), Ok(4));
        assert!(matches!(read(&empty_elements(5), &one), Err(Error::TooLarge(m)) if m.contains("4 elements")));
    }

    #[test]
    fn committed_icon_is_valid() {
        // An app passes its own committed icon, for example `<root>/assets/app-icon/<app>.icns`.
        let dir = tempfile::tempdir().expect("temporary directory");
        let path = dir.path().join("app.icns");
        let limits = Limits::default();
        std::fs::write(&path, pack(&[rgba(16), rgba(1024)], &limits).unwrap()).expect("write the icon");
        let elements = read_file(&path, &limits).unwrap();
        assert!(elements.iter().any(|e| &e.kind == b"ic10"), "expected a 1024 px image in {}", path.display());
    }
}
