#![allow(clippy::arithmetic_side_effects)]

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

/// A big-endian field of `size` bytes.
fn put(v: &mut Vec<u8>, x: u64, size: u8) {
    match size {
        0 => assert_eq!(x, 0, "a value needs a field"),
        2 => v.extend_from_slice(&(x as u16).to_be_bytes()),
        4 => v.extend_from_slice(&(x as u32).to_be_bytes()),
        8 => v.extend_from_slice(&x.to_be_bytes()),
        _ => panic!("no {size}-byte fields"),
    }
}

const TIFF: &[u8] = b"MM\0\x2a\0\0\0\x08\0\0\0\0\0\0";

/// The 100-byte EXIF of LightCraft's issue sweep `synthetic-metadata.heic`: Make "Synthetic",
/// Model "Issue sweep camera", DateTime 2026:10:12 12:34:56, big-endian.
const SWEEP_TIFF: [u8; 100] = [
    0x4d, 0x4d, 0x00, 0x2a, 0x00, 0x00, 0x00, 0x08, 0x00, 0x03, 0x01, 0x0f, 0x00, 0x02, 0x00, 0x00, 0x00, 0x0a, 0x00, 0x00, //
    0x00, 0x32, 0x01, 0x10, 0x00, 0x02, 0x00, 0x00, 0x00, 0x13, 0x00, 0x00, 0x00, 0x3c, 0x01, 0x32, 0x00, 0x02, 0x00, 0x00, //
    0x00, 0x14, 0x00, 0x00, 0x00, 0x50, 0x00, 0x00, 0x00, 0x00, 0x53, 0x79, 0x6e, 0x74, 0x68, 0x65, 0x74, 0x69, 0x63, 0x00, //
    0x49, 0x73, 0x73, 0x75, 0x65, 0x20, 0x73, 0x77, 0x65, 0x65, 0x70, 0x20, 0x63, 0x61, 0x6d, 0x65, 0x72, 0x61, 0x00, 0x00, //
    0x32, 0x30, 0x32, 0x36, 0x3a, 0x31, 0x30, 0x3a, 0x31, 0x32, 0x20, 0x31, 0x32, 0x3a, 0x33, 0x34, 0x3a, 0x35, 0x36, 0x00,
];

/// A TIFF stream as far as this reader looks: a big-endian TIFF header, then `marker`.
fn tiff_with(marker: &str) -> Vec<u8> {
    [b"MM\0*\0\0\0\x08".as_slice(), marker.as_bytes()].concat()
}

/// An XMP packet holding `title`.
fn xmp_with(title: &str) -> String {
    format!("<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><dc:title>{title}</dc:title></x:xmpmeta>")
}

/// An EXIF item as HEIF stores it: the 4-byte offset, `skip` (which the offset jumps), the TIFF.
fn exif_item(skip: &[u8], tiff: &[u8]) -> Vec<u8> {
    [&(skip.len() as u32).to_be_bytes()[..], skip, tiff].concat()
}

/// The photo's EXIF, as [`read`] gives it.
fn exif(f: &[u8]) -> Option<Vec<u8>> {
    read(f).ok()?.0
}

/// The photo's XMP packet, as [`read`] gives it.
fn xmp(f: &[u8]) -> Option<String> {
    read(f).ok()?.1
}

/// Every metadata item [`scan`] finds, when it is sure it saw them all: `None` when the catalog
/// is incomplete or an item can't be read.
fn sure(f: &[u8]) -> Option<Vec<(u32, Kind, Vec<u8>)>> {
    let s = scan(f).ok()?;
    if !s.complete() {
        return None;
    }
    s.map(|it| it.data.ok().map(|d| (it.id, it.kind, d))).collect()
}

// -- The single-`meta` builder of the first version of this reader --------------------------------

struct Entry<'a> {
    kind: &'a [u8; 4],
    content_type: &'a str,
    data: Vec<u8>,
}

/// A HEIC-shaped file holding `items`: `ftyp`, a `meta` box with one `infe` per item, and the
/// item bytes in `idat` (`in_idat`, construction method 1) or in `mdat` (method 0). No `pitm` and
/// no `iref`: every item describes the file.
fn heic(items: &[Entry<'_>], iloc_version: u8, in_idat: bool) -> Vec<u8> {
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

/// An `Exif` item's payload: the offset from the end of this field to the TIFF header, then
/// `skip` bytes of padding, then the TIFF stream.
fn exif_padded(tiff: &[u8], skip: usize) -> Vec<u8> {
    [&(skip as u32).to_be_bytes()[..], &vec![0u8; skip], tiff].concat()
}

fn exif_only(tiff: &[u8]) -> Vec<Entry<'static>> {
    vec![Entry { kind: b"Exif", content_type: "", data: exif_padded(tiff, 0) }]
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
        Entry { kind: b"mime", content_type: "text/plain", data: b"not the packet".to_vec() },
        Entry { kind: b"mime", content_type: "application/rdf+xml", data: b"<x:xmpmeta/>\0".to_vec() },
        Entry { kind: b"Exif", content_type: "", data: exif_padded(TIFF, 0) },
    ];
    assert_eq!(read(&heic(&items, 1, false)).unwrap(), (Some(TIFF.to_vec()), Some("<x:xmpmeta/>".into())));
}

/// The `Exif` item's leading offset may point past padding, not only straight at the TIFF header.
#[test]
fn the_exif_items_tiff_header_offset_is_honoured() {
    let items = vec![Entry { kind: b"Exif", content_type: "", data: exif_padded(TIFF, 6) }];
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
    assert!(scan(&[0xFF; 64]).is_err(), "only a file that isn't HEIF is an error to scan");
}

/// A truncated or scrambled file never panics, and lies about sizes find nothing.
#[test]
fn broken_files_never_panic() {
    let good = heic(
        &[
            Entry { kind: b"Exif", content_type: "", data: exif_padded(TIFF, 0) },
            Entry { kind: b"mime", content_type: "application/rdf+xml", data: b"<x:xmpmeta/>".to_vec() },
        ],
        1,
        false,
    );
    for cut in 0..good.len() {
        let _ = read(&good[..cut]);
        let _ = scan(&good[..cut]).map(Iterator::count);
    }
    for at in 0..good.len() {
        let mut bad = good.clone();
        bad[at] ^= 0xff;
        let _ = read(&bad);
        let _ = scan(&bad).map(Iterator::count);
    }
    let mut lying = good.clone();
    if let Some(at) = lying.windows(4).position(|w| w == b"iloc") {
        lying.splice(at + 8..at + 12, u32::MAX.to_be_bytes());
    }
    assert_eq!(read(&lying).unwrap().0, None);
    assert_eq!(sure(&lying), None, "an item `iloc` can't place is unreadable, not absent");
}

/// [`read`] considers the first MAX_CANDIDATES metadata items; an item larger than MAX_ITEM is
/// left out by [`read`] and too large for [`scan`].
#[test]
fn item_limits() {
    let mut items: Vec<Entry<'_>> =
        (0..MAX_CANDIDATES).map(|_| Entry { kind: b"Exif", content_type: "", data: exif_padded(b"not TIFF", 0) }).collect();
    items.push(Entry { kind: b"Exif", content_type: "", data: exif_padded(TIFF, 0) });
    assert_eq!(read(&heic(&items, 2, false)).unwrap().0, None, "the TIFF is metadata item {}", MAX_CANDIDATES + 1);
    items.remove(0);
    assert_eq!(read(&heic(&items, 2, false)).unwrap().0.as_deref(), Some(TIFF));
    let big = heic(&[Entry { kind: b"Exif", content_type: "", data: exif_padded(&vec![0; MAX_ITEM], 0) }], 1, true);
    assert_eq!(read(&big).unwrap().0, None);
    let found: Vec<Item> = scan(&big).unwrap().collect();
    assert_eq!(found, [Item { id: 1, kind: Kind::Exif, data: Err(Unreadable::TooLarge) }]);
}

/// The scan gathers at most its byte budget over all items; past it an item is too large.
#[test]
fn the_scan_byte_budget() {
    let items = [
        Entry { kind: b"Exif", content_type: "", data: exif_padded(TIFF, 0) },
        Entry { kind: b"mime", content_type: "application/rdf+xml", data: b"<x:xmpmeta/>".to_vec() },
    ];
    let f = heic(&items, 1, true);
    let first = TIFF.len() + 4;
    let found: Vec<Item> = scan_within(&f, first + 5).unwrap().collect();
    assert_eq!(found[0].data.as_deref().map(<[u8]>::len), Ok(first));
    assert_eq!(found[1].data, Err(Unreadable::TooLarge));
    let found: Vec<Item> = scan_within(&f, first + 12).unwrap().collect();
    assert_eq!(found[1].data.as_deref(), Ok(b"<x:xmpmeta/>".as_slice()));
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
    assert!(!scan(&file).unwrap().complete());
}

/// The container writer of this crate's tests: the reader agrees with the layout an encoder writes.
#[test]
fn a_synthetic_heif_file_carries_its_exif() {
    let mut spec = crate::testdata::Spec::new(16, 16, &|x, _| [x as u16 * 4, 0, 0]);
    spec.exif = Some(TIFF.to_vec());
    let file = crate::testdata::build(&spec);
    assert_eq!(read(&file).unwrap(), (Some(TIFF.to_vec()), None));
    let mut exif = 0u32.to_be_bytes().to_vec();
    exif.extend_from_slice(TIFF);
    assert!(matches!(sure(&file).as_deref(), Some([(_, Kind::Exif, data)]) if *data == exif));
    assert_eq!(read(&crate::testdata::build(&crate::testdata::Spec::new(16, 16, &|_, _| [0; 3]))).unwrap(), (None, None));
}

/// With the decoder built too, both readers give the same answer, also on files with a
/// thumbnail and with several items.
#[cfg(feature = "decode")]
#[test]
fn agrees_with_the_decoders_read_metadata() {
    let mut spec = crate::testdata::Spec::new(16, 16, &|x, _| [x as u16 * 4, 0, 0]);
    spec.exif = Some(TIFF.to_vec());
    let mut thumbnailed = crate::testdata::Spec::new(16, 16, &|x, _| [x as u16 * 4, 0, 0]);
    thumbnailed.exif = Some(TIFF.to_vec());
    thumbnailed.thumbnail = Some(&|_, y| [y as u16 * 4, 0, 0]);
    let files = [
        crate::testdata::build(&spec),
        crate::testdata::build(&thumbnailed),
        crate::testdata::build(&crate::testdata::Spec::new(8, 8, &|_, _| [0; 3])),
    ];
    for file in files {
        assert_eq!(read(&file), crate::read_metadata(&file));
    }
    let sequence = b"\0\0\0\x18ftypmsf1\0\0\0\0msf1hevc";
    assert_eq!(read(sequence), crate::read_metadata(sequence));
}

// -- A builder with a primary image, references and every `iloc` layout ----------------------------

#[derive(Clone, Copy, PartialEq)]
enum Store {
    File,
    Idat,
}

struct Part {
    id: u32,
    kind: &'static [u8; 4],
    /// `mime` items: content type and content encoding.
    mime: (&'static str, &'static str),
    data: Vec<u8>,
    store: Store,
    /// Extents the data is split into.
    extents: usize,
}

/// A HEIF file for the tests: `ftyp`, `meta` (`hdlr`, `pitm`, `iinf`, `iref`, `iloc`, `idat`)
/// and `mdat`.
struct Heif {
    ftyp: Vec<u8>,
    primary: u32,
    items: Vec<Part>,
    /// `iref` references: type, from, to.
    refs: Vec<(&'static [u8; 4], u32, Vec<u32>)>,
    /// `iloc` version and its offset, length, base-offset and index field sizes.
    iloc: (u8, [u8; 4]),
    /// 32-bit item ids: `infe` version 3, `pitm` and `iref` version 1 (`iloc` must be version 2).
    wide: bool,
}

impl Heif {
    /// A primary HEVC picture, item 1 (its bytes don't matter here).
    fn new() -> Heif {
        let picture = Part { id: 1, kind: b"hvc1", mime: ("", ""), data: b"coded picture".to_vec(), store: Store::File, extents: 1 };
        Heif { ftyp: b"heic\0\0\0\0mif1heic".to_vec(), primary: 1, items: vec![picture], refs: Vec::new(), iloc: (1, [4, 4, 0, 0]), wide: false }
    }

    fn item(mut self, id: u32, kind: &'static [u8; 4], data: Vec<u8>, store: Store, extents: usize) -> Heif {
        self.items.push(Part { id, kind, mime: ("", ""), data, store, extents });
        self
    }

    fn xmp(mut self, id: u32, mime: (&'static str, &'static str), data: Vec<u8>) -> Heif {
        self.items.push(Part { id, kind: b"mime", mime, data, store: Store::File, extents: 1 });
        self
    }

    fn cdsc(self, from: u32, to: &[u32]) -> Heif {
        self.iref(b"cdsc", from, to)
    }

    fn iref(mut self, kind: &'static [u8; 4], from: u32, to: &[u32]) -> Heif {
        self.refs.push((kind, from, to.to_vec()));
        self
    }

    fn meta(&self, mdat_at: usize) -> Vec<u8> {
        let id = |v: &mut Vec<u8>, id: u32| put(v, u64::from(id), if self.wide { 4 } else { 2 });
        let mut hdlr = vec![0u8; 4];
        hdlr.extend_from_slice(b"pict");
        hdlr.extend_from_slice(&[0; 13]);
        let mut pitm = Vec::new();
        id(&mut pitm, self.primary);
        let mut iinf = Vec::new();
        put(&mut iinf, self.items.len() as u64, if self.wide { 4 } else { 2 });
        for it in &self.items {
            let mut e = Vec::new();
            id(&mut e, it.id);
            e.extend_from_slice(&[0, 0]); // not protected
            e.extend_from_slice(it.kind);
            e.push(0); // no name
            if it.kind == b"mime" {
                e.extend_from_slice(it.mime.0.as_bytes());
                e.push(0);
                e.extend_from_slice(it.mime.1.as_bytes());
                e.push(0);
            }
            iinf.extend(full_box(b"infe", if self.wide { 3 } else { 2 }, &e));
        }
        let mut iref = Vec::new();
        for (kind, from, to) in &self.refs {
            let mut r = Vec::new();
            id(&mut r, *from);
            put(&mut r, to.len() as u64, 2);
            to.iter().for_each(|t| id(&mut r, *t));
            iref.extend(bx(kind, &r));
        }
        let (version, [o, l, b, i]) = self.iloc;
        let mut iloc = vec![o << 4 | l, b << 4 | i];
        put(&mut iloc, self.items.len() as u64, if version == 2 { 4 } else { 2 });
        let (mut in_file, mut in_idat) = (mdat_at as u64, 0u64);
        let mut idat = Vec::new();
        for it in &self.items {
            put(&mut iloc, u64::from(it.id), if version == 2 { 4 } else { 2 });
            let at = match it.store {
                Store::File => &mut in_file,
                Store::Idat => &mut in_idat,
            };
            if version > 0 {
                put(&mut iloc, if it.store == Store::Idat { 1 } else { 0 }, 2);
            }
            put(&mut iloc, 0, 2); // this file
            let base = if b > 0 { *at } else { 0 };
            put(&mut iloc, base, b);
            let chunk = it.data.len().div_ceil(it.extents.max(1)).max(1);
            let pieces: Vec<&[u8]> = it.data.chunks(chunk).collect();
            put(&mut iloc, pieces.len() as u64, 2);
            for piece in pieces {
                if version > 0 {
                    put(&mut iloc, 0, i);
                }
                put(&mut iloc, *at - base, o);
                put(&mut iloc, piece.len() as u64, l);
                *at += piece.len() as u64;
            }
            if it.store == Store::Idat {
                idat.extend_from_slice(&it.data);
            }
        }
        let ref_version = if self.wide { 1 } else { 0 };
        let boxes = [
            full_box(b"hdlr", 0, &hdlr),
            full_box(b"pitm", ref_version, &pitm),
            full_box(b"iinf", ref_version, &iinf),
            full_box(b"iref", ref_version, &iref),
            full_box(b"iloc", version, &iloc),
            bx(b"idat", &idat),
        ];
        full_box(b"meta", 0, &boxes.concat())
    }

    fn build(&self) -> Vec<u8> {
        let ftyp = bx(b"ftyp", &self.ftyp);
        let mdat_at = ftyp.len() + self.meta(0).len() + 8;
        let mdat: Vec<u8> = self.items.iter().filter(|i| i.store == Store::File).flat_map(|i| i.data.iter().copied()).collect();
        [ftyp, self.meta(mdat_at), bx(b"mdat", &mdat)].concat()
    }
}

/// One item's bytes, as [`read`] gathers them: its place from `iloc`, then its extents.
fn item_bytes(file: &[u8], iloc: &[u8], idat: Option<&[u8]>, id: u32) -> Option<Vec<u8>> {
    let iloc = Iloc::parse(iloc)?;
    let mut place = None;
    iloc.walk(|i, p| {
        if i == id {
            place = Some(p);
        }
        place.is_none()
    })?;
    match iloc.gather(file, idat, &place?, MAX_ITEM) {
        Gathered::Bytes(b) => Some(b),
        _ => None,
    }
}

/// LightCraft's issue sweep case, laid out as its HEIF test builder writes it: `iloc` version 1,
/// the EXIF item (offset 0, then the TIFF) tied to the primary image by `cdsc`.
#[test]
fn the_issue_sweep_heic_gives_its_exif() {
    let f = Heif::new().item(8, b"Exif", exif_item(&[], &SWEEP_TIFF), Store::File, 1).cdsc(8, &[1]).build();
    assert_eq!(exif(&f).as_deref(), Some(SWEEP_TIFF.as_slice()));
}

/// The 4-byte offset in front of the TIFF stream: iPhones write 6 and `Exif\0\0`, libheif and
/// avif-serialize 0. Writers that left the field out, or got it wrong, are read when the stream
/// is plain to see; anything else is no EXIF.
#[test]
fn exif_items_with_and_without_their_offset() {
    let t = &SWEEP_TIFF[..];
    let tiff = Some(t);
    assert_eq!(exif_tiff(&exif_item(b"Exif\0\0", t)), tiff, "iPhone");
    assert_eq!(exif_tiff(&exif_item(&[], t)), tiff, "libheif, avif-serialize");
    assert_eq!(exif_tiff(&exif_item(b"\0\0padding", t)), tiff, "any bytes the offset skips");
    assert_eq!(exif_tiff(&[b"\0\0\0\0Exif\0\0".as_slice(), t].concat()), tiff, "offset 0 though a prefix follows");
    assert_eq!(exif_tiff(&[&1000u32.to_be_bytes()[..], t].concat()), tiff, "offset past the end");
    assert_eq!(exif_tiff(&[b"Exif\0\0".as_slice(), t].concat()), tiff, "no offset field, JPEG style");
    assert_eq!(exif_tiff(t), tiff, "no offset field at all");
    assert_eq!(exif_tiff(&exif_item(&[], b"not a TIFF stream")), None);
    assert_eq!(exif_tiff(&[0, 0]), None);
    assert_eq!(exif_tiff(&[]), None);
    assert_eq!(exif_tiff(b"Exif\0"), None, "a prefix cut short");
    // and through a whole file, the iPhone way
    let f = Heif::new().item(2, b"Exif", exif_item(b"Exif\0\0", t), Store::File, 1).cdsc(2, &[1]).build();
    assert_eq!(exif(&f).as_deref(), tiff);
}

/// Items in several extents, in the file or the `meta` box's `idat`, with every `iloc` version
/// and field size.
#[test]
fn item_extents_and_construction_methods() {
    let packet = xmp_with("Harbour at dusk");
    let rdf = ("application/rdf+xml", "");
    // EXIF in `idat` (method 1), XMP in the file in three extents, a 32-bit base offset
    let mut h = Heif::new().item(2, b"Exif", exif_item(&[], &SWEEP_TIFF), Store::Idat, 1).cdsc(2, &[1]).xmp(3, rdf, packet.clone().into_bytes());
    h.items[2].extents = 3;
    h.iloc = (1, [8, 4, 4, 0]);
    let f = h.cdsc(3, &[1]).build();
    assert_eq!(read(&f).unwrap(), (Some(SWEEP_TIFF.to_vec()), Some(packet.clone())));
    // version 2: 32-bit item ids, an extent index, 64-bit lengths; EXIF in two extents
    let mut h = Heif::new().item(70_000, b"Exif", exif_item(b"Exif\0\0", &SWEEP_TIFF), Store::File, 2).cdsc(70_000, &[70_001]);
    (h.primary, h.items[0].id, h.wide, h.iloc) = (70_001, 70_001, true, (2, [4, 8, 0, 4]));
    assert_eq!(exif(&h.build()).as_deref(), Some(SWEEP_TIFF.as_slice()));
    // version 0: no construction method, file offsets only
    let mut h = Heif::new().item(2, b"Exif", exif_item(&[], &SWEEP_TIFF), Store::File, 1).cdsc(2, &[1]);
    h.iloc = (0, [4, 4, 0, 0]);
    assert_eq!(exif(&h.build()).as_deref(), Some(SWEEP_TIFF.as_slice()));
    // an XMP item in `idat`
    let mut h = Heif::new().xmp(5, rdf, packet.clone().into_bytes());
    h.items[1].store = Store::Idat;
    assert_eq!(xmp(&h.build()), Some(packet));
}

/// `iloc` entries the builder doesn't write: a length of 0 (to the end of the data), an item
/// in another file, an item built from other items, unknown versions and field sizes.
#[test]
fn iloc_entries_that_are_read_or_refused() {
    let file = b"0123456789".as_slice();
    let idat = b"abcdefgh".as_slice();
    // version 1, offset and length 4 bytes, no base or index; one item with one extent
    let iloc = |version: u8, sizes: [u8; 2], method: u16, data_ref: u16, offset: u32, length: u32| {
        let mut l = vec![version, 0, 0, 0];
        l.extend_from_slice(&sizes);
        l.extend_from_slice(&1u16.to_be_bytes());
        l.extend_from_slice(&7u16.to_be_bytes());
        if version > 0 {
            l.extend_from_slice(&method.to_be_bytes());
        }
        l.extend_from_slice(&data_ref.to_be_bytes());
        l.extend_from_slice(&1u16.to_be_bytes());
        l.extend_from_slice(&offset.to_be_bytes());
        l.extend_from_slice(&length.to_be_bytes());
        l
    };
    let get = |l: &[u8]| item_bytes(file, l, Some(idat), 7);
    assert_eq!(get(&iloc(1, [0x44, 0], 0, 0, 2, 3)).as_deref(), Some(b"234".as_slice()));
    assert_eq!(get(&iloc(1, [0x44, 0], 1, 0, 2, 3)).as_deref(), Some(b"cde".as_slice()), "from idat");
    assert_eq!(get(&iloc(1, [0x44, 0], 0, 0, 6, 0)).as_deref(), Some(b"6789".as_slice()), "length 0: to the end of the file");
    assert_eq!(get(&iloc(1, [0x44, 0], 1, 0, 5, 0)).as_deref(), Some(b"fgh".as_slice()), "length 0: to the end of idat");
    assert_eq!(item_bytes(file, &iloc(1, [0x44, 0], 1, 0, 0, 1), None, 7), None, "method 1 without idat");
    assert_eq!(get(&iloc(1, [0x44, 0], 2, 0, 0, 1)), None, "method 2 (from other items)");
    assert_eq!(get(&iloc(1, [0x44, 0], 0, 1, 0, 1)), None, "in another file");
    assert_eq!(get(&iloc(1, [0x44, 0], 0, 0, 8, 3)), None, "past the end");
    assert_eq!(get(&iloc(1, [0x44, 0], 0, 0, 11, 0)), None, "starts past the end");
    assert_eq!(get(&iloc(1, [0x22, 0], 0, 0, 0, 1)), None, "2-byte fields are not in the standard");
    assert_eq!(get(&iloc(3, [0x44, 0], 0, 0, 0, 1)), None, "unknown version");
    assert_eq!(item_bytes(file, &iloc(1, [0x44, 0], 0, 0, 0, 1), None, 8), None, "no such item");
    // a base offset near u64::MAX plus an extent offset overflows: refused, not wrapped
    let mut l = vec![1, 0, 0, 0, 0x44, 0x80];
    l.extend_from_slice(&1u16.to_be_bytes());
    l.extend_from_slice(&7u16.to_be_bytes());
    l.extend_from_slice(&[0, 0, 0, 0]); // method 0, data reference 0
    l.extend_from_slice(&u64::MAX.to_be_bytes());
    l.extend_from_slice(&1u16.to_be_bytes());
    l.extend_from_slice(&1u32.to_be_bytes());
    l.extend_from_slice(&1u32.to_be_bytes());
    assert_eq!(get(&l), None);
}

/// The primary image's metadata is read; metadata tied only to another image is not.
#[test]
fn the_primary_images_metadata_is_read() {
    let thumb = |h: Heif| h.item(2, b"hvc1", b"thumbnail".to_vec(), Store::File, 1);
    // a thumbnail's EXIF comes first in the file; the primary image's is the one read
    let f = thumb(Heif::new())
        .item(10, b"Exif", exif_item(&[], &tiff_with("Thumbnail")), Store::File, 1)
        .item(11, b"Exif", exif_item(&[], &tiff_with("Primary")), Store::File, 1)
        .cdsc(10, &[2])
        .cdsc(11, &[1])
        .build();
    assert_eq!(exif(&f), Some(tiff_with("Primary")));
    // EXIF tied only to another image is not this photo's
    let f = thumb(Heif::new()).item(10, b"Exif", exif_item(&[], &tiff_with("Thumbnail")), Store::File, 1).cdsc(10, &[2]).build();
    assert_eq!(exif(&f), None);
    // tied to no image: it describes the file; tied to both: read
    let f = Heif::new().item(10, b"Exif", exif_item(&[], &tiff_with("Whole file")), Store::File, 1).build();
    assert_eq!(exif(&f), Some(tiff_with("Whole file")));
    let f = thumb(Heif::new()).item(10, b"Exif", exif_item(&[], &tiff_with("Both")), Store::File, 1).cdsc(10, &[2, 1]).build();
    assert_eq!(exif(&f), Some(tiff_with("Both")));
    // a primary image's EXIF that can't be read gives way to one tied to no image
    let f = Heif::new()
        .item(10, b"Exif", exif_item(&[], b"damaged"), Store::File, 1)
        .item(11, b"Exif", exif_item(&[], &tiff_with("Fallback")), Store::File, 1)
        .cdsc(10, &[1])
        .build();
    assert_eq!(exif(&f), Some(tiff_with("Fallback")));
    // XMP: only uncompressed `application/rdf+xml` items, the primary image's first
    let f = thumb(Heif::new())
        .xmp(20, ("image/jpeg", ""), b"\xff\xd8 not XMP".to_vec())
        .xmp(21, ("application/rdf+xml", "deflate"), b"compressed".to_vec())
        .xmp(22, ("application/rdf+xml", ""), xmp_with("Thumbnail").into_bytes())
        .xmp(23, (" Application/RDF+XML ", ""), xmp_with("Primary").into_bytes())
        .cdsc(22, &[2])
        .cdsc(23, &[1])
        .build();
    assert_eq!(xmp(&f), Some(xmp_with("Primary")));
    // 32-bit ids in `pitm`, `iinf` and `iref`
    let mut h = Heif::new().item(70_000, b"Exif", exif_item(&[], &tiff_with("Wide")), Store::File, 1).cdsc(70_000, &[70_001]);
    (h.primary, h.items[0].id, h.wide, h.iloc.0) = (70_001, 70_001, true, 2);
    assert_eq!(exif(&h.build()), Some(tiff_with("Wide")));
}

/// EXIF tied to an image the primary is derived from (`dimg`: an `iden` turn, an `iovl` overlay,
/// a `tmap` tone map, a grid) is the primary image's, through any depth and despite loops. So is
/// EXIF in a file whose `pitm` names no item. A thumbnail's EXIF still isn't.
#[test]
fn metadata_of_the_images_a_primary_is_made_from_is_read() {
    let base = |h: Heif| {
        let mut h = h
            .item(2, b"hvc1", b"thumbnail".to_vec(), Store::File, 1)
            .item(3, b"hvc1", b"coded picture".to_vec(), Store::File, 1)
            .item(10, b"Exif", exif_item(&[], &tiff_with("Thumbnail")), Store::File, 1)
            .item(11, b"Exif", exif_item(&[], &tiff_with("Input")), Store::File, 1)
            .cdsc(10, &[2])
            .cdsc(11, &[3]);
        h.items[0].kind = b"iden";
        h
    };
    // the primary (1) turns item 3
    let f = base(Heif::new()).iref(b"dimg", 1, &[3]).build();
    assert_eq!(exif(&f), Some(tiff_with("Input")));
    // two levels (a tone map of a grid of 3), and a loop back to the primary
    let f =
        base(Heif::new()).item(4, b"grid", Vec::new(), Store::File, 1).iref(b"dimg", 1, &[4]).iref(b"dimg", 4, &[3]).iref(b"dimg", 3, &[1]).build();
    assert_eq!(exif(&f), Some(tiff_with("Input")));
    // `pitm` names an item that doesn't exist: there is no primary image, so tied items count
    let mut h = Heif::new().item(11, b"Exif", exif_item(&[], &tiff_with("Orphan")), Store::File, 1).cdsc(11, &[1]);
    h.primary = 99;
    assert_eq!(exif(&h.build()), Some(tiff_with("Orphan")));
    // the thumbnail is not an input of the primary: without the `dimg`, nothing is the photo's
    assert_eq!(exif(&base(Heif::new()).build()), None);
}

/// HEIF, HEIC and AVIF brands are read; other files built of the same boxes are not HEIF.
#[test]
fn heif_and_avif_brands() {
    let with = |ftyp: &[u8]| {
        let mut h = Heif::new().item(2, b"Exif", exif_item(&[], &SWEEP_TIFF), Store::File, 1).cdsc(2, &[1]);
        h.ftyp = ftyp.to_vec();
        h.build()
    };
    for ftyp in [&b"heic\0\0\0\0mif1heic"[..], b"mif1\0\0\0\0mif1", b"heix\0\0\0\0mif1heix", b"avif\0\0\0\0avifmif1miaf", b"msf1\0\0\0\0msf1hevc"] {
        assert!(is_heif(&with(ftyp)), "{}", String::from_utf8_lossy(ftyp));
        assert_eq!(exif(&with(ftyp)).as_deref(), Some(SWEEP_TIFF.as_slice()), "{}", String::from_utf8_lossy(ftyp));
    }
    // MP4 and QuickTime video, JPEG XL, Canon CR3
    for ftyp in [&b"isom\0\0\x02\0isomiso2mp41"[..], b"qt  \0\0\0\0qt  ", b"jxl \0\0\0\0jxl ", b"crx \0\0\0\x01crx isom", b"mif1\0\0\0\0crx "] {
        assert!(!is_heif(&with(ftyp)), "{}", String::from_utf8_lossy(ftyp));
        assert!(read(&with(ftyp)).is_err() && scan(&with(ftyp)).is_err());
    }
    assert!(!is_heif(b"\0\0\0\x0cftyp"), "no brand");
    assert!(!is_heif(&[0xff, 0xd8, 0xff, 0xe0, 0, 0x10, b'J', b'F', b'I', b'F', 0, 1]), "JPEG");
    // a HEIF file without a `meta` box has no metadata
    let bare = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
    assert!(matches!(read(&bare), Err(Error::Unsupported(_))));
    assert_eq!(sure(&bare), Some(Vec::new()));
}

/// Damaged structure: an out-of-range box, an `iinf` or `iref` that claims more than it holds,
/// extents that overlap to gather more than [`MAX_ITEM`], an item count far past the box.
#[test]
fn malformed_bounds_are_refused() {
    let good = Heif::new().item(2, b"Exif", exif_item(&[], &SWEEP_TIFF), Store::File, 1).cdsc(2, &[1]).build();
    // the `meta` box says it is longer than the file: nothing is read
    let mut f = good.clone();
    let at = u32::from_be_bytes(f[0..4].try_into().unwrap()) as usize; // `meta` follows `ftyp`
    assert_eq!(&f[at + 4..at + 8], b"meta");
    f[at..at + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(exif(&f), None);
    assert_eq!(sure(&f), None);
    // an extent past the end of the file
    let mut f = good.clone();
    f.truncate(f.len() - 1);
    assert_eq!(exif(&f), None);
    // 65,535 extents over the same 64 KiB: refused at MAX_ITEM, quickly
    let file = vec![0u8; 64 << 10];
    let mut l = vec![1, 0, 0, 0, 0x44, 0];
    l.extend_from_slice(&1u16.to_be_bytes());
    l.extend_from_slice(&7u16.to_be_bytes());
    l.extend_from_slice(&[0, 0, 0, 0]);
    l.extend_from_slice(&u16::MAX.to_be_bytes());
    for _ in 0..u16::MAX {
        l.extend_from_slice(&0u32.to_be_bytes());
        l.extend_from_slice(&(file.len() as u32).to_be_bytes());
    }
    assert_eq!(item_bytes(&file, &l, None, 7), None);
    // an item count of 2^32 - 1 in a box holding one entry
    let mut l = vec![2, 0, 0, 0, 0x44, 0];
    l.extend_from_slice(&u32::MAX.to_be_bytes());
    l.extend_from_slice(&1u32.to_be_bytes());
    l.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    assert_eq!(item_bytes(&file, &l, None, 7), None);
    // a `cdsc` claiming 65,535 targets with none there, an `infe` cut short
    let mut iref = vec![0, 0, 0, 0];
    iref.extend(bx(b"cdsc", &[0, 2, 0xff, 0xff]));
    let iref = bx(b"iref", &iref);
    let mut items = vec![Candidate { id: 2, kind: Kind::Exif, describes: false, describes_primary: false, protected: false }];
    let mut budget = MAX_BOXES;
    let b = boxes(&iref, 0, iref.len(), &mut budget);
    link(&iref, &b[0], Some(1), &mut items, &mut budget);
    assert!(!items[0].describes);
    assert_eq!(item_info(&[2, 0, 0, 0, 0, 2, 0, 0, b'E', b'x']), None, "cut short: the item type is unknown");
    assert_eq!(item_info(&[4, 0, 0, 0, 0, 2, 0, 0, b'E', b'x', b'i', b'f', 0]), None, "unknown infe version");
    assert_eq!(item_info(&[2, 0, 0, 0, 0, 2, 0, 1, b'E', b'x', b'i', b'f', 0]), Some((2, Some(Kind::Exif), true)), "protected");
    assert_eq!(item_info(&[2, 0, 0, 0, 0, 2, 0, 1, b'E', b'x']), None, "protected and cut short");
    // a full box's version is its own first byte, never the next box's
    let two = [bx(b"iref", &[]), bx(b"free", &[1, 2, 3])].concat();
    let mut budget = MAX_BOXES;
    let b = boxes(&two, 0, two.len(), &mut budget);
    assert_eq!((version(&two, &b[0]), version(&two, &b[1])), (None, Some(1)));
}

/// Box headers that lie: a 64-bit size, a `uuid` box too short for its extended type, a size
/// smaller than the header, a size of 0 (to the end).
#[test]
fn box_headers() {
    let mut large = 1u32.to_be_bytes().to_vec();
    large.extend_from_slice(b"free");
    large.extend_from_slice(&20u64.to_be_bytes());
    large.extend_from_slice(b"abcd");
    let mut budget = MAX_BOXES;
    let b = boxes(&large, 0, large.len(), &mut budget);
    assert_eq!((b.len(), b[0].body, b[0].end), (1, 16, 20));
    large[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
    assert!(boxes(&large, 0, large.len(), &mut budget).is_empty(), "a 64-bit size past the end");
    let short_uuid = bx(b"uuid", &[0; 8]);
    assert!(boxes(&short_uuid, 0, short_uuid.len(), &mut budget).is_empty());
    let uuid = bx(b"uuid", &[0; 20]);
    assert_eq!(boxes(&uuid, 0, uuid.len(), &mut budget)[0].body, 24);
    assert!(boxes(&[0, 0, 0, 4, b'f', b'r', b'e', b'e'], 0, 8, &mut budget).is_empty(), "smaller than its header");
    let to_end = [&[0u8, 0, 0, 0][..], b"mdat", b"payload"].concat();
    assert_eq!(boxes(&to_end, 0, to_end.len(), &mut budget)[0].end, to_end.len());
    assert!(boxes(&to_end, 9, 4, &mut budget).is_empty(), "an empty range");
    let mut none = 0;
    assert!(boxes(&to_end, 0, to_end.len(), &mut none).is_empty(), "no budget left");
}

/// Every cut-short or damaged file is read without a panic.
#[test]
fn truncated_and_damaged_files_never_panic() {
    let mut h = Heif::new()
        .item(2, b"Exif", exif_item(b"Exif\0\0", &SWEEP_TIFF), Store::Idat, 2)
        .xmp(3, ("application/rdf+xml", ""), xmp_with("t").into_bytes())
        .cdsc(2, &[1])
        .cdsc(3, &[1]);
    h.items[2].extents = 3;
    h.iloc = (1, [4, 8, 4, 4]);
    let f = h.build();
    assert!(matches!(read(&f), Ok((Some(_), Some(_)))));
    for n in 0..f.len() {
        let _ = read(&f[..n]);
        let _ = scan(&f[..n]).map(Iterator::count);
    }
    for i in 0..f.len() {
        for v in [0x00, 0xff, 0x7f, 0x01] {
            let mut g = f.clone();
            g[i] = v;
            let _ = read(&g);
            let _ = scan(&g).map(Iterator::count);
        }
    }
}

/// [`scan`] returns every EXIF and XMP item, not only the photo's own first EXIF: a thumbnail's
/// EXIF and XMP, the primary image's second EXIF item, a compressed XMP item.
#[test]
fn the_scan_returns_every_metadata_item() {
    let plain = tiff_with("Primary");
    let thumb = |h: Heif| h.item(2, b"hvc1", b"thumbnail".to_vec(), Store::File, 1);
    let ids = |f: &[u8]| sure(f).map(|items| items.into_iter().map(|(id, kind, _)| (id, kind)).collect::<Vec<_>>());
    // the thumbnail's EXIF
    let f = thumb(Heif::new())
        .item(10, b"Exif", exif_item(&[], &plain), Store::File, 1)
        .item(11, b"Exif", exif_item(&[], &tiff_with("Thumbnail")), Store::File, 1)
        .cdsc(10, &[1])
        .cdsc(11, &[2])
        .build();
    assert_eq!(exif(&f), Some(plain.clone()), "the photo's own EXIF");
    let all = sure(&f).unwrap();
    assert_eq!(all[1], (11, Kind::Exif, exif_item(&[], &tiff_with("Thumbnail"))));
    // the primary image's second EXIF item
    let f = Heif::new()
        .item(10, b"Exif", exif_item(&[], &plain), Store::File, 1)
        .item(11, b"Exif", exif_item(&[], &tiff_with("Second")), Store::File, 1)
        .cdsc(10, &[1])
        .cdsc(11, &[1])
        .build();
    assert_eq!(exif(&f), Some(plain.clone()));
    assert_eq!(ids(&f), Some(vec![(10, Kind::Exif), (11, Kind::Exif)]));
    // a thumbnail's XMP (with place names, say) is not the photo's, but the scan has it
    let f = thumb(Heif::new())
        .item(10, b"Exif", exif_item(&[], &plain), Store::File, 1)
        .cdsc(10, &[1])
        .xmp(20, ("application/rdf+xml", ""), xmp_with("Thumbnail").into_bytes())
        .cdsc(20, &[2])
        .build();
    assert_eq!(xmp(&f), None, "the thumbnail's packet is not the primary image's");
    assert_eq!(sure(&f).unwrap()[1], (20, Kind::Xmp, xmp_with("Thumbnail").into_bytes()));
    // a compressed XMP item comes back as stored, for the caller to judge
    let f = Heif::new().xmp(20, ("application/rdf+xml", "deflate"), b"compressed".to_vec()).build();
    assert_eq!(sure(&f), Some(vec![(20, Kind::EncodedXmp, b"compressed".to_vec())]));
    // an EXIF item cut off by the end of the file, one larger than the reader takes, more items
    // than it looks at: not sure
    let mut f = Heif::new().item(10, b"Exif", exif_item(&[], &plain), Store::File, 1).build();
    f.truncate(f.len() - 1);
    assert_eq!(sure(&f), None, "cut off");
    let f = Heif::new().item(10, b"Exif", exif_item(&[], &vec![0; MAX_ITEM]), Store::File, 1).build();
    assert_eq!(scan(&f).unwrap().next().map(|it| it.data), Some(Err(Unreadable::TooLarge)), "too large");
    let mut h = Heif::new();
    for id in 0..(MAX_SCANNED as u32 + 1) {
        h = h.item(10 + id, b"Exif", exif_item(&[], &plain), Store::File, 1);
    }
    let f = h.build();
    assert!(!scan(&f).unwrap().complete(), "too many items");
    assert_eq!(scan(&f).unwrap().count(), MAX_SCANNED);
    assert_eq!(exif(&f), Some(plain), "read still finds the photo's EXIF");
}

/// Replace only the catalog; the real metadata remains in `idat` with unchanged `iloc`.
fn with_iinf(h: &Heif, iinf: Option<&[u8]>) -> Vec<u8> {
    assert!(h.items.iter().filter(|it| it.kind == b"Exif" || it.kind == b"mime").all(|it| it.store == Store::Idat));
    let meta = h.meta(0);
    let mut budget = MAX_BOXES;
    let children: Vec<u8> = boxes(&meta, 12, meta.len(), &mut budget)
        .iter()
        .flat_map(|b| if &b.kind == b"iinf" { iinf.map(|data| bx(b"iinf", data)).unwrap_or_default() } else { bx(&b.kind, &meta[b.body..b.end]) })
        .collect();
    [bx(b"ftyp", &h.ftyp), full_box(b"meta", 0, &children)].concat()
}

fn catalog(version: u8, count: u32, entries: &[u8]) -> Vec<u8> {
    let mut data = vec![version, 0, 0, 0];
    put(&mut data, u64::from(count), if version == 0 { 2 } else { 4 });
    data.extend_from_slice(entries);
    data
}

fn catalog_entry(version: u8, id: u32, kind: &[u8; 4], fields: &[u8]) -> Vec<u8> {
    let mut data = vec![version, 0, 0, 0];
    put(&mut data, u64::from(id), if version == 3 { 4 } else { 2 });
    data.extend_from_slice(&[0, 0]);
    if version >= 2 {
        data.extend_from_slice(kind);
    }
    data.extend_from_slice(fields);
    data
}

/// An `infe` cut anywhere, even one that keeps the id and part of the type, makes the scan unsure.
#[test]
fn item_catalog_truncated_type_is_uncertain() {
    let tiff = tiff_with("Located");
    let h = Heif::new().item(2, b"Exif", exif_item(&[], &tiff), Store::Idat, 1);
    for version in [2, 3] {
        let entry = catalog_entry(version, 2, b"Exif", b"\0");
        let intact = with_iinf(&h, Some(&catalog(0, 1, &bx(b"infe", &entry))));
        assert_eq!(exif(&intact), Some(tiff.clone()), "infe v{version}");
        assert_eq!(sure(&intact), Some(vec![(2, Kind::Exif, exif_item(&[], &tiff))]), "infe v{version}");
        for end in 0..entry.len() {
            let damaged = with_iinf(&h, Some(&catalog(0, 1, &bx(b"infe", &entry[..end]))));
            assert_eq!(sure(&damaged), None, "infe v{version}, cut at {end}");
        }
    }
}

/// An XMP entry cut in its fields or strings makes the scan unsure, in every `infe` and `iinf` version.
#[test]
fn item_catalog_truncated_fields_and_strings_are_uncertain() {
    let packet = xmp_with("Located").into_bytes();
    let mut h = Heif::new().xmp(2, ("application/rdf+xml", ""), packet.clone());
    h.items[1].store = Store::Idat;
    for version in 0..=3 {
        // The optional content encoding is absent; name and content type are terminated.
        let entry = catalog_entry(version, 2, b"mime", b"name\0application/rdf+xml\0");
        for table_version in [0, 1] {
            let intact = with_iinf(&h, Some(&catalog(table_version, 1, &bx(b"infe", &entry))));
            assert_eq!(sure(&intact), Some(vec![(2, Kind::Xmp, packet.clone())]), "infe v{version}, iinf v{table_version}");
            for end in 0..entry.len() {
                let damaged = with_iinf(&h, Some(&catalog(table_version, 1, &bx(b"infe", &entry[..end]))));
                assert_eq!(sure(&damaged), None, "infe v{version}, iinf v{table_version}, cut at {end}");
            }
        }
    }
}

/// A missing or cut catalog, unknown versions, unknown entry boxes, a count that doesn't match
/// the entries and unterminated strings all make the scan unsure.
#[test]
fn item_catalog_incomplete_walks_counts_and_versions_are_uncertain() {
    let h = Heif::new().item(2, b"Exif", exif_item(&[], &tiff_with("Located")), Store::Idat, 1);
    let entry = catalog_entry(2, 2, b"Exif", b"\0");
    let intact = catalog(0, 1, &bx(b"infe", &entry));
    assert!(sure(&with_iinf(&h, Some(&intact))).is_some());
    let mut cases = vec![("missing catalog".to_string(), None)];
    for end in 0..intact.len() {
        cases.push((format!("catalog cut at {end}"), Some(intact[..end].to_vec())));
    }
    cases.push(("unsupported table version".into(), Some(catalog(2, 1, &bx(b"infe", &entry)))));
    let mut unsupported = entry.clone();
    unsupported[0] = 4;
    cases.push(("unsupported entry version".into(), Some(catalog(0, 1, &bx(b"infe", &unsupported)))));
    cases.push(("unknown entry box".into(), Some(catalog(0, 1, &bx(b"free", &entry)))));
    // A valid image entry cannot explain a declared but absent second entry.
    let image = bx(b"infe", &catalog_entry(2, 1, b"hvc1", b"\0"));
    for version in [0, 1] {
        for count in [0, 2] {
            cases.push((format!("iinf v{version}, count {count}"), Some(catalog(version, count, &image))));
        }
    }
    for (kind, fields) in [(b"hvc1", b"unterminated".as_slice()), (b"uri ", b"name\0unterminated"), (b"mime", b"name\0image/jpeg\0gzip")] {
        cases.push((format!("unterminated {kind:?} field"), Some(catalog(0, 1, &bx(b"infe", &catalog_entry(2, 2, kind, fields))))));
    }
    for (label, data) in cases {
        assert_eq!(sure(&with_iinf(&h, data.as_deref())), None, "{label}");
    }
}

/// An unreadable catalog entry doesn't stop [`read`] from returning the EXIF it could read.
#[test]
fn item_catalog_uncertainty_keeps_readable_exif() {
    let mut h = Heif::new().item(2, b"Exif", exif_item(&[], &tiff_with("Readable")), Store::Idat, 1).xmp(
        3,
        ("application/rdf+xml", ""),
        xmp_with("Located").into_bytes(),
    );
    h.items[2].store = Store::Idat;
    assert!(sure(&h.build()).is_some());
    let exif_entry = bx(b"infe", &catalog_entry(2, 2, b"Exif", b"\0"));
    let broken = bx(b"infe", &catalog_entry(2, 3, b"mime", b"name\0application/rdf+"));
    for entries in [[exif_entry.clone(), broken.clone()], [broken, exif_entry]] {
        let f = with_iinf(&h, Some(&catalog(0, 2, &entries.concat())));
        assert_eq!(exif(&f), Some(tiff_with("Readable")));
        assert_eq!(sure(&f), None, "unreadable XMP after or before readable EXIF");
    }
}

/// Catalogs without metadata items leave a complete, empty scan.
#[test]
fn item_catalog_benign_entries_and_empty_tables_are_complete() {
    let h = Heif::new();
    assert_eq!(sure(&h.build()), Some(Vec::new()), "image only");
    for table_version in [0, 1] {
        assert_eq!(sure(&with_iinf(&h, Some(&catalog(table_version, 0, &[])))), Some(Vec::new()), "empty catalog");
        for version in 0..=3 {
            for fields in [b"\0image/jpeg\0".as_slice(), b"name\0image/jpeg\0\0", b"name\0image/jpeg\0gzip\0"] {
                let entry = catalog_entry(version, 1, b"mime", fields);
                assert_eq!(sure(&with_iinf(&h, Some(&catalog(table_version, 1, &bx(b"infe", &entry))))), Some(Vec::new()));
            }
        }
        for kind in [b"hvc1", b"av01", b"zzzz", b"uri "] {
            let fields = if kind == b"uri " { b"name\0https://example.invalid/item\0".as_slice() } else { b"name\0" };
            let entry = catalog_entry(2, 1, kind, fields);
            assert_eq!(sure(&with_iinf(&h, Some(&catalog(table_version, 1, &bx(b"infe", &entry))))), Some(Vec::new()));
        }
    }
    // A protected (encrypted) image item is no metadata item.
    let mut entry = catalog_entry(2, 1, b"hvc1", b"\0");
    entry[7] = 1;
    assert_eq!(sure(&with_iinf(&h, Some(&catalog(0, 1, &bx(b"infe", &entry))))), Some(Vec::new()));
    // Complete top-level files without an item meta box have no item catalog to inspect.
    assert_eq!(sure(&bx(b"ftyp", b"msf1\0\0\0\0msf1")), Some(Vec::new()));
}

/// A protected (encrypted) `Exif` or XMP item is in the scan, as unreadable: the file holds
/// metadata that can't be checked. [`read`] leaves it out.
#[test]
fn protected_metadata_is_unreadable_not_absent() {
    let h = Heif::new().item(2, b"Exif", exif_item(&[], &tiff_with("Located")), Store::Idat, 1);
    for (kind, fields, expected) in [(b"Exif", b"\0".as_slice(), Kind::Exif), (b"mime", b"name\0application/rdf+xml\0", Kind::Xmp)] {
        for version in [2, 3] {
            let mut entry = catalog_entry(version, 2, kind, fields);
            let protection_at = if version == 3 { 9 } else { 7 };
            entry[protection_at] = 1;
            let f = with_iinf(&h, Some(&catalog(0, 1, &bx(b"infe", &entry))));
            let s = scan(&f).unwrap();
            assert!(s.complete(), "{kind:?} v{version}");
            let found: Vec<Item> = s.collect();
            assert_eq!(found, [Item { id: 2, kind: expected, data: Err(Unreadable::Protected) }], "{kind:?} v{version}");
            assert_eq!(sure(&f), None, "{kind:?} v{version}");
            assert_eq!(read(&f).unwrap(), (None, None), "{kind:?} v{version}");
        }
    }
}

/// Running out of box budget, or a walk that stops before the end of its box, never makes a
/// scan complete.
#[test]
fn item_catalog_box_budget_and_enclosing_walks_cannot_clear_uncertainty() {
    let h = Heif::new().item(2, b"Exif", exif_item(&[], &tiff_with("Located")), Store::Idat, 1);
    let f = h.build();
    let ftyp = bx(b"ftyp", &h.ftyp);
    let mut top_cut = f.clone();
    top_cut[ftyp.len()..ftyp.len() + 4].copy_from_slice(&u32::MAX.to_be_bytes());
    assert_eq!(sure(&top_cut), None, "meta box cut short");
    let hidden = [ftyp.clone(), bx(b"free", &[]).repeat(MAX_BOXES), h.meta(0)].concat();
    assert_eq!(sure(&hidden), None, "top-level budget hides meta");
    let hidden = [ftyp.clone(), full_box(b"meta", 0, &[bx(b"free", &[]).repeat(MAX_BOXES), h.meta(0)[12..].to_vec()].concat())].concat();
    assert_eq!(sure(&hidden), None, "meta budget hides catalog");
    let image = bx(b"infe", &catalog_entry(2, 1, b"hvc1", b"\0"));
    let data = catalog(1, MAX_BOXES as u32, &image.repeat(MAX_BOXES));
    assert_eq!(sure(&with_iinf(&h, Some(&data))), None, "catalog budget");
    let hidden = [ftyp, full_box(b"meta", 0, &[vec![0; 3], h.meta(0)[12..].to_vec()].concat())].concat();
    assert_eq!(sure(&hidden), None, "incomplete meta child walk");
}

/// Replace `iloc` in a builder fixture. Metadata stays in `idat`, so changing the box sizes
/// cannot change its offsets. The coded picture is not used by these metadata tests.
fn with_iloc(h: &Heif, iloc: Option<&[u8]>) -> Vec<u8> {
    assert!(h.items.iter().filter(|it| it.kind == b"Exif" || it.kind == b"mime").all(|it| it.store == Store::Idat));
    let meta = h.meta(0);
    let mut budget = MAX_BOXES;
    let children: Vec<u8> = boxes(&meta, 12, meta.len(), &mut budget)
        .iter()
        .flat_map(|b| if &b.kind == b"iloc" { iloc.map(|data| bx(b"iloc", data)).unwrap_or_default() } else { bx(&b.kind, &meta[b.body..b.end]) })
        .collect();
    [bx(b"ftyp", &h.ftyp), full_box(b"meta", 0, &children)].concat()
}

/// The `iloc` bytes of a builder fixture.
fn iloc_of(f: &[u8]) -> Vec<u8> {
    Container::read(f, MAX_SCANNED).unwrap().iloc.unwrap().data.to_vec()
}

/// An `iloc` cut after a readable EXIF entry leaves the later XMP unresolved: [`read`] still
/// returns the EXIF, and the scan reports the XMP as unreadable.
#[test]
fn iloc_truncation_after_readable_exif_leaves_xmp_unreadable() {
    let h = Heif::new().item(2, b"Exif", exif_item(&[], &tiff_with("First")), Store::Idat, 1);
    assert!(sure(&h.build()).is_some(), "readable EXIF alone");
    let mut h = h.xmp(3, ("application/rdf+xml", ""), xmp_with("Located").into_bytes());
    h.items[2].store = Store::Idat;
    let f = h.build();
    assert_eq!(sure(&f).map(|v| v.len()), Some(2), "the intact XMP");
    let iloc = iloc_of(&f);
    let cut = with_iloc(&h, Some(&iloc[..iloc.len() - 1]));
    assert_eq!(exif(&cut), Some(tiff_with("First")), "the first metadata item still reads");
    let found: Vec<Item> = scan(&cut).unwrap().collect();
    assert_eq!(found[1], Item { id: 3, kind: Kind::Xmp, data: Err(Unreadable::Placement) }, "the unresolved XMP isn't skipped");
}

/// A missing, cut or unsupported `iloc` leaves every metadata item unreadable.
#[test]
fn iloc_unresolved_metadata_is_unreadable() {
    for kind in [b"Exif", b"mime"] {
        let mut h = Heif::new().item(2, kind, b"unread metadata".to_vec(), Store::Idat, 1);
        h.items[1].mime = ("application/rdf+xml", "");
        let iloc = iloc_of(&h.build());
        assert_eq!(sure(&with_iloc(&h, None)), None, "missing iloc: {kind:?}");
        // Every cut leaves the metadata entry unresolved, including cuts in its last extent.
        for end in 0..iloc.len() {
            assert_eq!(sure(&with_iloc(&h, Some(&iloc[..end]))), None, "{kind:?}: iloc cut at {end}");
        }
        // Unsupported version or any one of the four field sizes.
        for (at, value) in [(0, 3), (4, 0x24), (4, 0x42), (5, 0x20), (5, 0x02)] {
            let mut bad = iloc.clone();
            bad[at] = value;
            assert_eq!(sure(&with_iloc(&h, Some(&bad))), None, "{kind:?}: field {at} = {value}");
        }
    }
}

/// A partial `iloc` walk that placed one item doesn't make another, unplaced item absent.
#[test]
fn iloc_partial_walk_does_not_clear_unresolved_metadata() {
    let h = Heif::new().item(2, b"Exif", Vec::new(), Store::Idat, 1).item(3, b"Exif", b"unread metadata".to_vec(), Store::Idat, 1);
    let iloc = iloc_of(&h.build());
    // Picture: 16 bytes; empty EXIF: 8 bytes; final EXIF: 16 bytes. Keep the first EXIF
    // intact while cutting the second entry at every byte, including its extent fields.
    for end in 32..iloc.len() {
        let cut = with_iloc(&h, Some(&iloc[..end]));
        let partial = Container::read(&cut, MAX_SCANNED).unwrap();
        assert!(matches!(partial.gather(0, MAX_ITEM), Gathered::Bytes(b) if b.is_empty()));
        assert_eq!(sure(&cut), None, "iloc cut at {end}");
    }
}

/// A complete `iloc` that places no entry for an item, or one with no extents, is sure: the
/// item has no bytes.
#[test]
fn iloc_complete_walk_allows_unplaced_and_empty_items() {
    let h = Heif::new().item(2, b"Exif", Vec::new(), Store::Idat, 1);
    assert_eq!(sure(&h.build()), Some(vec![(2, Kind::Exif, Vec::new())]), "placed EXIF with no extents");
    // A well-formed empty table, or a complete table placing only a different item.
    for version in 0..=2 {
        let mut iloc = vec![version, 0, 0, 0, 0, 0];
        put(&mut iloc, 0, if version == 2 { 4 } else { 2 });
        assert_eq!(sure(&with_iloc(&h, Some(&iloc))), Some(Vec::new()), "empty iloc v{version}");
        let count_at = iloc.len() - 1;
        iloc[count_at] = 1;
        put(&mut iloc, 9, if version == 2 { 4 } else { 2 });
        if version > 0 {
            put(&mut iloc, 0, 2); // construction method
        }
        iloc.extend_from_slice(&[0, 0, 0, 0]); // this file, no extents
        assert_eq!(sure(&with_iloc(&h, Some(&iloc))), Some(Vec::new()), "absent item in complete iloc v{version}");
    }
    // No metadata candidates: there is nothing to resolve, even without a usable iloc.
    assert_eq!(sure(&Heif::new().build()), Some(Vec::new()));
    for iloc in [None, Some([3, 0, 0, 0, 0, 0].as_slice())] {
        ILOC_ENTRIES.with(|n| n.set(0));
        assert_eq!(sure(&with_iloc(&Heif::new(), iloc)), Some(Vec::new()));
        assert_eq!(ILOC_ENTRIES.with(std::cell::Cell::get), 0);
    }
}

/// The `iloc` walk ends once every metadata item is placed, before a damaged later entry.
#[test]
fn iloc_stops_after_all_metadata_is_placed() {
    let h = Heif::new().item(2, b"Exif", Vec::new(), Store::Idat, 1).item(3, b"hvc1", vec![1], Store::File, 1);
    let iloc = iloc_of(&h.build());
    // The last picture entry is truncated. The only metadata entry is already complete.
    let cut = with_iloc(&h, Some(&iloc[..32]));
    ILOC_ENTRIES.with(|n| n.set(0));
    assert_eq!(sure(&cut), Some(vec![(2, Kind::Exif, Vec::new())]));
    assert_eq!(ILOC_ENTRIES.with(std::cell::Cell::get), 2, "stop after the picture and the EXIF entry");
}

/// A placed item whose extents leave its data, or pass the limit, can't be read.
#[test]
fn iloc_resolved_extents_still_obey_bounds_and_budget() {
    let h = Heif::new().item(2, b"Exif", b"abcd".to_vec(), Store::Idat, 1);
    let f = h.build();
    let c = Container::read(&f, MAX_SCANNED).unwrap();
    assert!(matches!(c.gather(0, 4), Gathered::Bytes(b) if b == b"abcd"));
    assert!(matches!(c.gather(0, 3), Gathered::TooLarge));
    let iloc = iloc_of(&f);
    for (offset, length) in [(0u32, 5u32), (5, 0), (u32::MAX, 1)] {
        let mut bad = iloc.clone();
        let end = bad.len();
        bad[end - 8..end - 4].copy_from_slice(&offset.to_be_bytes());
        bad[end - 4..].copy_from_slice(&length.to_be_bytes());
        let f = with_iloc(&h, Some(&bad));
        let c = Container::read(&f, MAX_SCANNED).unwrap();
        assert!(matches!(c.gather(0, MAX_ITEM), Gathered::Unreadable));
        assert_eq!(sure(&f), None);
    }
}

/// A file whose MAX_SCANNED `Exif` items each name one 64 KiB region 257 times: every item is
/// too large, and finding that out copies nothing, for [`scan`] (all 1,024 gathers) and for
/// [`read`] (its MAX_CANDIDATES). Before, each gather copied MAX_ITEM bytes before giving up,
/// about 16 GiB for the scan of a 2 MiB file.
#[test]
fn too_large_items_are_refused_before_copying() {
    const REGION: u32 = 64 << 10;
    const REPEATS: u16 = 257;
    let count = MAX_SCANNED as u16;
    let mut iinf = count.to_be_bytes().to_vec();
    for id in 1..=count {
        let mut e = vec![2u8, 0, 0, 0];
        e.extend_from_slice(&id.to_be_bytes());
        e.extend_from_slice(&[0, 0]);
        e.extend_from_slice(b"Exif\0");
        iinf.extend(bx(b"infe", &e));
    }
    let iloc = |region_at: u32| {
        let mut l = vec![0x44, 0x00];
        l.extend_from_slice(&count.to_be_bytes());
        for id in 1..=count {
            l.extend_from_slice(&id.to_be_bytes());
            l.extend_from_slice(&[0, 0, 0, 0]); // method 0, this file
            l.extend_from_slice(&REPEATS.to_be_bytes());
            for _ in 0..REPEATS {
                l.extend_from_slice(&region_at.to_be_bytes());
                l.extend_from_slice(&REGION.to_be_bytes());
            }
        }
        full_box(b"iloc", 1, &l)
    };
    let ftyp = bx(b"ftyp", b"heic\0\0\0\0mif1heic");
    let meta = |region_at| full_box(b"meta", 0, &[full_box(b"iinf", 0, &iinf), iloc(region_at)].concat());
    let region_at = (ftyp.len() + meta(0).len() + 8) as u32;
    let f = [ftyp, meta(region_at), bx(b"mdat", &vec![0; REGION as usize])].concat();

    COPIED.with(|c| c.set(0));
    let s = scan(&f).unwrap();
    assert!(s.complete());
    let found: Vec<Item> = s.collect();
    assert_eq!(found.len(), MAX_SCANNED);
    assert!(found.iter().all(|it| it.data == Err(Unreadable::TooLarge)));
    assert_eq!(COPIED.with(std::cell::Cell::get), 0, "scan copied bytes of items it refused");

    COPIED.with(|c| c.set(0));
    assert_eq!(read(&f).unwrap(), (None, None));
    assert_eq!(COPIED.with(std::cell::Cell::get), 0, "read copied bytes of items it refused");

    // an item that fits is copied once, extent by extent
    let mut h = Heif::new().item(2, b"Exif", exif_item(&[], &SWEEP_TIFF), Store::Idat, 3);
    h.iloc = (1, [4, 4, 4, 0]);
    let f = h.build();
    COPIED.with(|c| c.set(0));
    assert_eq!(sure(&f), Some(vec![(2, Kind::Exif, exif_item(&[], &SWEEP_TIFF))]));
    assert_eq!(COPIED.with(std::cell::Cell::get), SWEEP_TIFF.len() + 4);
}

/// `iloc` is walked once, however many metadata items there are to place (one walk per item
/// took over a second on a 48 MB `iloc` with 64 items).
#[test]
fn iloc_is_walked_once() {
    // 64 metadata items, none placed by `iloc`, and 20,000 entries for other items
    let mut iinf = vec![0u8, 0, 0, 0];
    iinf.extend_from_slice(&64u16.to_be_bytes());
    for id in 0..64u16 {
        let mut e = vec![2u8, 0, 0, 0];
        e.extend_from_slice(&(1000 + id).to_be_bytes());
        e.extend_from_slice(&[0, 0]);
        e.extend_from_slice(if id % 2 == 0 { b"Exif\0" } else { b"mime\0application/rdf+xml\0" });
        iinf.extend(bx(b"infe", &e));
    }
    let entries = 20_000usize;
    let mut iloc = vec![2u8, 0, 0, 0, 0, 0];
    iloc.extend_from_slice(&(entries as u32).to_be_bytes());
    for _ in 0..entries {
        iloc.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0]); // item 1, method 0, this file, no extents
    }
    let meta = full_box(b"meta", 0, &[bx(b"iinf", &iinf), bx(b"iloc", &iloc)].concat());
    let f = [bx(b"ftyp", b"heic\0\0\0\0mif1heic"), meta].concat();
    let walked = |run: &dyn Fn(&[u8])| {
        ILOC_ENTRIES.with(|n| n.set(0));
        run(&f);
        ILOC_ENTRIES.with(std::cell::Cell::get)
    };
    let n = walked(&|f| assert_eq!(read(f).unwrap(), (None, None)));
    assert!(n <= entries, "read walked {n} entries of {entries}");
    let n = walked(&|f| assert_eq!(sure(f), Some(Vec::new())));
    assert!(n <= entries, "scan walked {n} entries of {entries}");
}

proptest::proptest! {
    /// Arbitrary bytes behind a HEIF `ftyp`, or inside its `meta` box, never panic.
    #[test]
    fn arbitrary_boxes_never_panic(body in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..512)) {
        for f in [[bx(b"ftyp", b"heic\0\0\0\0mif1heic"), body.clone()].concat(), [bx(b"ftyp", b"heic\0\0\0\0mif1heic"), full_box(b"meta", 0, &body)].concat()] {
            let _ = read(&f);
            let _ = scan(&f).map(Iterator::count);
        }
    }
}
