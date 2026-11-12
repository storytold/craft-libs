//! HEIF / HEIC / AVIF metadata from the box structure alone (feature `metadata`): the `Exif` and
//! XMP items of the ISO base media file container (box structure from ISO/IEC 14496-12, item model
//! from ISO/IEC 23008-12). It needs no HEVC decoder and no heic-rs, so a build that only catalogues
//! photos can read their capture time, camera and lens.
//!
//! The `meta` box holds an item information box (`iinf`, one `infe` per item) naming each item's
//! type, and an item location box (`iloc`) saying where its bytes are: in the file, or in the
//! `idat` box next to it. The `Exif` item is a four-byte offset followed by a TIFF stream; the XMP
//! packet is a `mime` item whose content type is `application/rdf+xml`. The first such item counts
//! (the decoder's [`crate::read_metadata`] takes the primary image's instead). Moved from LightCraft's
//! `lightcraft-meta`.

use crate::{Error, SEQUENCE};

/// The Exif and XMP blocks found.
#[derive(Default)]
struct Embedded {
    exif: Option<Vec<u8>>,
    xmp: Option<String>,
}

/// The first `Exif` item's TIFF stream (without HEIF's offset header) and the first XMP packet of a
/// HEIF file, as [`crate::read_metadata`] returns them. An error when the file is not an ISO base
/// media still image ([`is_heif`]), or has no `meta` box (an image sequence); an item that can't be
/// located is left out rather than reported.
pub fn read(bytes: &[u8]) -> Result<(Option<Vec<u8>>, Option<String>), Error> {
    if !is_heif(bytes) {
        return Err(Error::Malformed("not a HEIF file".into()));
    }
    let mut budget = MAX_BOXES;
    if !boxes(bytes, 0, bytes.len(), &mut budget).iter().any(|b| &b.kind == b"meta") {
        return Err(Error::Unsupported(SEQUENCE));
    }
    let e = metadata(bytes);
    Ok((e.exif, e.xmp))
}

/// At most this many boxes are walked in one file.
pub const MAX_BOXES: usize = 4096;

/// Brands that declare a still image in the ISO base media container.
const IMAGE_BRANDS: [&[u8]; 12] = [b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"hevm", b"hevs", b"mif1", b"msf1", b"avif", b"avis"];

/// Longest `ftyp` box this reads brands from.
pub const MAX_FTYP: usize = 4096;
/// Most items one file's `iinf` / `iloc` describes (an iPhone HEIC has a handful).
pub const MAX_ITEMS: usize = 1024;
/// Most extents one item's bytes are split into.
pub const MAX_EXTENTS: u16 = 1024;
/// Largest metadata item this returns; a real photo's Exif and XMP are a few hundred kilobytes.
pub const MAX_ITEM: usize = 32 << 20;

/// Does this file declare an ISO base media still image (HEIF / HEIC / AVIF)?
pub fn is_heif(bytes: &[u8]) -> bool {
    if bytes.get(4..8) != Some(b"ftyp".as_slice()) {
        return false;
    }
    let Some(size) = be32(bytes, 0).map(|v| v as usize) else {
        return false;
    };
    let end = size.min(bytes.len()).min(MAX_FTYP);
    let brand = |at: usize| bytes.get(at..at.saturating_add(4)).is_some_and(|b| IMAGE_BRANDS.contains(&b));
    // the major brand, then (after the minor version) the compatible brands
    brand(8) || (16..end).step_by(4).any(brand)
}

/// The Exif and XMP blocks of a HEIF file. Anything malformed is left out rather than reported:
/// a file whose items can't be located simply has no metadata, as before this reader existed.
fn metadata(bytes: &[u8]) -> Embedded {
    let mut out = Embedded::default();
    let mut budget = MAX_BOXES;
    let Some(meta) = boxes(bytes, 0, bytes.len(), &mut budget).into_iter().find(|b| &b.kind == b"meta") else {
        return out;
    };
    // `meta` is a FullBox: a version and flags before its children
    let (mut items, mut locs, mut idat) = (Vec::new(), Vec::new(), None);
    for b in boxes(bytes, meta.body.saturating_add(4), meta.end, &mut budget) {
        match &b.kind {
            b"iinf" => items = item_infos(bytes, &b, &mut budget),
            b"iloc" => locs = item_locations(bytes.get(b.body..b.end).unwrap_or_default()),
            b"idat" => idat = bytes.get(b.body..b.end),
            _ => {}
        }
    }
    for (id, kind, content_type) in items {
        let want_exif = &kind == b"Exif" && out.exif.is_none();
        let want_xmp = &kind == b"mime" && content_type == "application/rdf+xml" && out.xmp.is_none();
        if !want_exif && !want_xmp {
            continue;
        }
        let Some(loc) = locs.iter().find(|l| l.id == id) else { continue };
        let Some(block) = item_bytes(bytes, idat, loc) else { continue };
        if want_exif {
            // ISO/IEC 23008-12 A.2.1: a four-byte offset from the end of itself to the TIFF header
            let skip = be32(&block, 0).and_then(|v| usize::try_from(v).ok()).and_then(|v| v.checked_add(4));
            out.exif = skip.and_then(|at| block.get(at..)).filter(|b| !b.is_empty()).map(<[u8]>::to_vec);
        } else {
            out.xmp = Some(String::from_utf8_lossy(&block).trim_end_matches('\0').to_string());
        }
    }
    out
}

/// `iinf` → one `(item_ID, item_type, content_type)` per `infe` entry. Entries of version 0 and 1
/// name no item type and are skipped; HEIF files use version 2 or 3.
fn item_infos(bytes: &[u8], iinf: &BoxRef, budget: &mut usize) -> Vec<(u32, [u8; 4], String)> {
    // FullBox version and flags, then a 16- or 32-bit entry count before the `infe` boxes
    let version = *bytes.get(iinf.body).unwrap_or(&0);
    let entries_at = iinf.body.saturating_add(if version == 0 { 6 } else { 8 });
    let mut out = Vec::new();
    for b in boxes(bytes, entries_at, iinf.end, budget) {
        if &b.kind != b"infe" || out.len() >= MAX_ITEMS {
            continue;
        }
        let Some(e) = bytes.get(b.body..b.end) else { continue };
        let (id, kind_at) = match *e.first().unwrap_or(&0) {
            2 => (be16(e, 4).map(u32::from), 8),
            3 => (be32(e, 4), 10),
            _ => continue,
        };
        let (Some(id), Some(kind)) = (id, e.get(kind_at..kind_at + 4)) else { continue };
        let mut kind4 = [0u8; 4];
        kind4.copy_from_slice(kind);
        // the item name, then (for a `mime` item) the content type, both NUL-terminated
        let mut strings = e.get(kind_at + 4..).unwrap_or_default().split(|&c| c == 0).skip(1);
        let content_type = String::from_utf8_lossy(strings.next().unwrap_or_default()).into_owned();
        out.push((id, kind4, content_type));
    }
    out
}

/// Where one item's bytes are.
struct Loc {
    id: u32,
    /// 0 = offsets into the file, 1 = into the `idat` box, 2 = another item (not read).
    construction: u8,
    base: u64,
    /// `(offset, length)` per extent, in order.
    extents: Vec<(u64, u64)>,
}

/// `iloc` (payload after the box header) → one [`Loc`] per entry, stopping at the first
/// unreadable one.
fn item_locations(p: &[u8]) -> Vec<Loc> {
    let version = *p.first().unwrap_or(&0);
    let (Some(sizes), Some(base_index)) = (p.get(4), p.get(5)) else { return Vec::new() };
    let (offset_size, length_size) = ((sizes >> 4) as usize, (sizes & 0xf) as usize);
    let (base_size, index_size) = ((base_index >> 4) as usize, if version > 0 { (base_index & 0xf) as usize } else { 0 });
    let (count, mut at) = match version {
        0 | 1 => (be16(p, 6).map(usize::from), 8),
        2 => (be32(p, 6).and_then(|v| usize::try_from(v).ok()), 10),
        _ => return Vec::new(),
    };
    let mut out = Vec::new();
    for _ in 0..count.unwrap_or(0).min(MAX_ITEMS) {
        let (id, after_id) = match version {
            2 => (be32(p, at), at + 4),
            _ => (be16(p, at).map(u32::from), at + 2),
        };
        // version 1 and 2 put the construction method in the low nibble of a reserved 16-bit field
        let construction = if version == 0 { Some(0) } else { be16(p, after_id).map(|v| (v & 0xf) as u8) };
        let after_method = after_id + if version == 0 { 0 } else { 2 };
        // a data reference index of 0 means this file; an external reference has no bytes here
        let external = be16(p, after_method).is_some_and(|v| v != 0);
        let base = uint(p, after_method + 2, base_size);
        let extent_at = after_method + 2 + base_size;
        let (Some(id), Some(construction), Some(base), Some(extent_count)) = (id, construction, base, be16(p, extent_at)) else {
            break;
        };
        let stride = index_size + offset_size + length_size;
        let mut extents = Vec::new();
        let mut e = extent_at + 2;
        for _ in 0..extent_count.min(MAX_EXTENTS) {
            let (Some(offset), Some(length)) = (uint(p, e + index_size, offset_size), uint(p, e + index_size + offset_size, length_size)) else {
                return out;
            };
            extents.push((offset, length));
            e += stride;
        }
        at = e;
        if !external {
            out.push(Loc { id, construction, base, extents });
        }
    }
    out
}

/// A big-endian unsigned integer of `size` bytes; `size` 0 is the value 0, and the item boxes
/// allow no other width than 4 or 8.
fn uint(b: &[u8], at: usize, size: usize) -> Option<u64> {
    match size {
        0 => Some(0),
        4 => be32(b, at).map(u64::from),
        8 => be64(b, at),
        _ => None,
    }
}

/// An item's bytes, its extents concatenated. `None` when they don't fit the file (or the `idat`
/// box), when the item is built by referencing another one, or when it is implausibly large.
fn item_bytes(bytes: &[u8], idat: Option<&[u8]>, loc: &Loc) -> Option<Vec<u8>> {
    let src = match loc.construction {
        0 => bytes,
        1 => idat?,
        _ => return None,
    };
    let mut out = Vec::new();
    for &(offset, length) in &loc.extents {
        let at = loc.base.checked_add(offset)?;
        let (start, end) = (usize::try_from(at).ok()?, usize::try_from(at.checked_add(length)?).ok()?);
        let extent = src.get(start..end)?;
        if out.len().checked_add(extent.len())? > MAX_ITEM {
            return None;
        }
        out.extend_from_slice(extent);
    }
    (!out.is_empty()).then_some(out)
}

/// A box: its type and where its payload ends.
struct BoxRef {
    kind: [u8; 4],
    /// Start of the payload (after the header and, for `uuid`, the 16-byte uuid).
    body: usize,
    end: usize,
}

/// The boxes directly inside `[from, to)`; stops at the first malformed header.
fn boxes(bytes: &[u8], from: usize, to: usize, budget: &mut usize) -> Vec<BoxRef> {
    let mut out = Vec::new();
    let mut at = from;
    let to = to.min(bytes.len());
    while at.saturating_add(8) <= to && *budget > 0 {
        *budget -= 1;
        let (Some(size), Some(kind)) = (be32(bytes, at), bytes.get(at + 4..at + 8)) else { break };
        let mut kind4 = [0u8; 4];
        kind4.copy_from_slice(kind);
        let (len, mut hdr) = match size {
            0 => (to - at, 8),
            1 => match be64(bytes, at + 8).and_then(|v| usize::try_from(v).ok()) {
                Some(l) => (l, 16),
                None => break,
            },
            n => (n as usize, 8),
        };
        let Some(end) = at.checked_add(len).filter(|&e| e <= to && len >= hdr) else { break };
        if &kind4 == b"uuid" {
            // the 16-byte extended type is part of the header
            if at + hdr + 16 > end {
                break;
            }
            hdr += 16;
        }
        out.push(BoxRef { kind: kind4, body: at + hdr, end });
        at = end;
    }
    out
}

/// A big-endian `u16` at `at`, or `None` when the buffer is shorter than that.
fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at.checked_add(2)?).map(|s| u16::from_be_bytes([s[0], s[1]]))
}
/// A big-endian `u32` at `at`, or `None` when the buffer is shorter than that.
fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at.checked_add(4)?).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}
/// A big-endian `u64` at `at`, or `None` when the buffer is shorter than that.
fn be64(b: &[u8], at: usize) -> Option<u64> {
    let s = b.get(at..at.checked_add(8)?)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(u64::from_be_bytes(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One box: its own size, its four-letter kind, then its payload.
    fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend_from_slice(kind);
        v.extend_from_slice(payload);
        v
    }

    /// A FullBox: a box whose payload begins with a version byte and three flag bytes.
    fn full_box(kind: &[u8; 4], version: u8, payload: &[u8]) -> Vec<u8> {
        bx(kind, &[&[version, 0, 0, 0][..], payload].concat())
    }

    struct Item<'a> {
        kind: &'a [u8; 4],
        content_type: &'a str,
        data: Vec<u8>,
    }

    /// A HEIC-shaped file holding `items`: `ftyp`, a `meta` box with one `infe` per item, and the
    /// item bytes in `idat` (`in_idat`, construction method 1) or in `mdat` (method 0).
    fn heic(items: &[Item<'_>], iloc_version: u8, in_idat: bool) -> Vec<u8> {
        let mut iinf = (items.len() as u16).to_be_bytes().to_vec();
        for (i, it) in items.iter().enumerate() {
            let mut e = vec![2u8, 0, 0, 0]; // version 2, flags
            e.extend_from_slice(&(i as u16 + 1).to_be_bytes());
            e.extend_from_slice(&[0, 0]); // protection index
            e.extend_from_slice(it.kind);
            e.extend_from_slice(b"an item\0");
            if it.kind == b"mime" {
                e.extend_from_slice(it.content_type.as_bytes());
                e.push(0);
            }
            iinf.extend_from_slice(&bx(b"infe", &e));
        }
        let iinf = full_box(b"iinf", 0, &iinf);
        let payload: Vec<u8> = items.iter().flat_map(|i| i.data.iter().copied()).collect();
        let idat = if in_idat { bx(b"idat", &payload) } else { Vec::new() };
        let iloc = |base: u32| {
            // four-byte offsets and lengths, no base offset and no extent index
            let mut l = vec![0x44, 0x00];
            match iloc_version {
                2 => l.extend_from_slice(&(items.len() as u32).to_be_bytes()),
                _ => l.extend_from_slice(&(items.len() as u16).to_be_bytes()),
            }
            let mut at = base;
            for (i, it) in items.iter().enumerate() {
                match iloc_version {
                    2 => l.extend_from_slice(&(i as u32 + 1).to_be_bytes()),
                    _ => l.extend_from_slice(&(i as u16 + 1).to_be_bytes()),
                }
                if iloc_version > 0 {
                    l.extend_from_slice(&[0, u8::from(in_idat)]); // construction method
                }
                l.extend_from_slice(&[0, 0]); // data reference index: this file
                l.extend_from_slice(&[0, 1]); // one extent
                l.extend_from_slice(&at.to_be_bytes());
                l.extend_from_slice(&(it.data.len() as u32).to_be_bytes());
                at += it.data.len() as u32;
            }
            full_box(b"iloc", iloc_version, &l)
        };
        let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        let meta = |base| full_box(b"meta", 0, &[iloc(base), iinf.clone(), idat.clone()].concat());
        if in_idat {
            return [ftyp, meta(0)].concat();
        }
        let base = (ftyp.len() + meta(0).len() + 8) as u32;
        [ftyp, meta(base), bx(b"mdat", &payload)].concat()
    }

    const TIFF: &[u8] = b"MM\0\x2a\0\0\0\x08\0\0\0\0\0\0";

    /// An `Exif` item's payload: the offset from the end of this field to the TIFF header, then
    /// `skip` bytes of padding, then the TIFF stream.
    fn exif_item(tiff: &[u8], skip: usize) -> Vec<u8> {
        [&(skip as u32).to_be_bytes()[..], &vec![0u8; skip], tiff].concat()
    }

    fn exif_only(tiff: &[u8]) -> Vec<Item<'static>> {
        vec![Item { kind: b"Exif", content_type: "", data: exif_item(tiff, 0) }]
    }

    /// Every item location box the format allows: counts and ids are 16-bit before version 2 and
    /// 32-bit in it, and the bytes live in `mdat` or in `idat` next to the location box itself.
    #[test]
    fn items_are_located_in_every_iloc_version() {
        for (version, in_idat) in [(0, false), (1, false), (1, true), (2, false), (2, true)] {
            let file = heic(&exif_only(TIFF), version, in_idat);
            assert_eq!(read(&file).unwrap(), (Some(TIFF.to_vec()), None), "iloc v{version}, idat {in_idat}");
        }
    }

    /// The XMP packet is the `mime` item that declares `application/rdf+xml`, not any other one.
    #[test]
    fn the_xmp_packet_is_the_rdf_mime_item() {
        let items = vec![
            Item { kind: b"mime", content_type: "text/plain", data: b"not the packet".to_vec() },
            Item { kind: b"mime", content_type: "application/rdf+xml", data: b"<x:xmpmeta/>\0".to_vec() },
            Item { kind: b"Exif", content_type: "", data: exif_item(TIFF, 0) },
        ];
        assert_eq!(read(&heic(&items, 1, false)).unwrap(), (Some(TIFF.to_vec()), Some("<x:xmpmeta/>".into())));
    }

    /// The `Exif` item's leading offset may point past padding, not only straight at the TIFF header.
    #[test]
    fn the_exif_items_tiff_header_offset_is_honoured() {
        let items = vec![Item { kind: b"Exif", content_type: "", data: exif_item(TIFF, 6) }];
        assert_eq!(read(&heic(&items, 1, false)).unwrap().0.as_deref(), Some(TIFF));
    }

    /// Which `ftyp` brands count as a still image in this container, major or compatible.
    #[test]
    fn brands() {
        for brand in ["heic", "heix", "mif1", "avif", "hevc", "msf1"] {
            let file = bx(b"ftyp", format!("{brand}\0\0\0\0mif1").as_bytes());
            assert!(is_heif(&file), "{brand}");
        }
        assert!(is_heif(&bx(b"ftyp", b"mp41\0\0\0\0isomheic")));
        assert!(!is_heif(&bx(b"ftyp", b"crx \0\0\0\0crx isom")));
        assert!(!is_heif(&bx(b"ftyp", b"jxl \0\0\0\0jxl ")));
        assert!(!is_heif(b"\xff\xd8\xff\xe1 a jpeg"));
        assert!(!is_heif(b""));
        // a compatible brand past MAX_FTYP bytes is not read
        let mut long = b"mp41\0\0\0\0".to_vec();
        long.resize(MAX_FTYP, b'x');
        long.extend_from_slice(b"heic");
        assert!(!is_heif(&bx(b"ftyp", &long)));
    }

    /// Not a HEIF file, an image sequence (no `meta`), garbage: errors as the decoder reports them.
    #[test]
    fn what_is_not_a_still_image_is_an_error() {
        assert!(matches!(read(b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc"), Err(Error::Unsupported(m)) if m.contains("sequence")));
        for bytes in [&b""[..], b"\0\0\0\x18ftypheic\0\0\0\0mif1heic", &[0xFF; 64]] {
            assert!(read(bytes).is_err());
        }
    }

    /// A truncated or scrambled file never panics, and lies about sizes find nothing.
    #[test]
    fn broken_files_never_panic() {
        let good = heic(
            &[
                Item { kind: b"Exif", content_type: "", data: exif_item(TIFF, 0) },
                Item { kind: b"mime", content_type: "application/rdf+xml", data: b"<x:xmpmeta/>".to_vec() },
            ],
            1,
            false,
        );
        for cut in 0..good.len() {
            let _ = read(&good[..cut]);
        }
        for at in 0..good.len() {
            let mut bad = good.clone();
            bad[at] ^= 0xff;
            let _ = read(&bad);
        }
        let mut lying = good.clone();
        if let Some(at) = lying.windows(4).position(|w| w == b"iloc") {
            lying.splice(at + 8..at + 12, u32::MAX.to_be_bytes());
        }
        assert_eq!(read(&lying).unwrap().0, None);
    }

    /// Past MAX_ITEMS `infe` entries the rest are not read; an item larger than MAX_ITEM is left out.
    #[test]
    fn item_limits() {
        let mut items: Vec<Item<'_>> = (0..MAX_ITEMS).map(|_| Item { kind: b"mime", content_type: "text/plain", data: vec![b'x'] }).collect();
        items.push(Item { kind: b"Exif", content_type: "", data: exif_item(TIFF, 0) });
        assert_eq!(read(&heic(&items, 2, false)).unwrap().0, None, "the Exif item is entry {}", MAX_ITEMS + 1);
        items.remove(0);
        assert_eq!(read(&heic(&items, 2, false)).unwrap().0.as_deref(), Some(TIFF));
        let big = vec![Item { kind: b"Exif", content_type: "", data: exif_item(&vec![0; MAX_ITEM], 0) }];
        assert_eq!(read(&heic(&big, 1, true)).unwrap().0, None);
    }

    /// The box walk stops after MAX_BOXES boxes: a `meta` box after that many is not found.
    #[test]
    fn box_limit() {
        let mut file = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
        for _ in 0..MAX_BOXES {
            file.extend(bx(b"free", &[]));
        }
        file.extend(heic(&exif_only(TIFF), 1, true).split_off(bx(b"ftyp", b"heic\0\0\0\0mif1heic").len()));
        assert!(matches!(read(&file), Err(Error::Unsupported(_))));
    }

    /// The container writer of this crate's tests: the reader agrees with the layout an encoder writes.
    #[test]
    fn a_synthetic_heif_file_carries_its_exif() {
        let mut spec = crate::testdata::Spec::new(16, 16, &|x, _| [x as u16 * 4, 0, 0]);
        spec.exif = Some(TIFF.to_vec());
        assert_eq!(read(&crate::testdata::build(&spec)).unwrap(), (Some(TIFF.to_vec()), None));
        assert_eq!(read(&crate::testdata::build(&crate::testdata::Spec::new(16, 16, &|_, _| [0; 3]))).unwrap(), (None, None));
    }

    /// With the decoder built too, both readers give the same answer on files with one Exif item.
    #[cfg(feature = "decode")]
    #[test]
    fn agrees_with_the_decoders_read_metadata() {
        let mut spec = crate::testdata::Spec::new(16, 16, &|x, _| [x as u16 * 4, 0, 0]);
        spec.exif = Some(TIFF.to_vec());
        for file in [crate::testdata::build(&spec), crate::testdata::build(&crate::testdata::Spec::new(8, 8, &|_, _| [0; 3]))] {
            assert_eq!(read(&file), crate::read_metadata(&file));
        }
        let sequence = b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc";
        assert_eq!(read(sequence), crate::read_metadata(sequence));
    }
}
