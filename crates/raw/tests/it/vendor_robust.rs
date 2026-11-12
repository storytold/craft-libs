//! Malformed maker raw files must decode to an error, never panic: every truncation and random mutations of one
//! small synthetic file per maker layout.

use craft_tiff::tags as t;
use craft_tiff::{ByteOrder, IfdBuilder, ImageData, TiffWriter, Value};
use proptest::prelude::*;

const W: u32 = 32;
const H: u32 = 8;

/// A CFA TIFF the way most makers lay it out: IFD0 (make) + a SubIFD holding the raw strip.
fn cfa_tiff(make: &str, compression: u16, bits: u16, order: ByteOrder) -> Vec<u8> {
    let px = (W * H) as usize;
    let strip: Vec<u8> = (0..px * 2).map(|i| (i * 37 % 251) as u8).collect();
    let mut raw = IfdBuilder::new();
    raw.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![0]));
    raw.set(t::IMAGE_WIDTH, Value::Long(vec![W]));
    raw.set(t::IMAGE_LENGTH, Value::Long(vec![H]));
    raw.set(t::BITS_PER_SAMPLE, Value::Short(vec![bits]));
    raw.set(t::COMPRESSION, Value::Short(vec![compression]));
    raw.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
    raw.set(t::CFA_REPEAT_PATTERN_DIM, Value::Short(vec![2, 2]));
    raw.set(t::CFA_PATTERN_EP, Value::Byte(vec![0, 1, 1, 2]));
    raw.set_image(ImageData::Strips { rows_per_strip: H, strips: vec![strip] });
    let mut ifd0 = IfdBuilder::new();
    ifd0.set(t::MAKE, Value::Ascii(make.into()));
    ifd0.set(t::MODEL, Value::Ascii("TEST".into()));
    ifd0.add_sub_ifd(raw);
    TiffWriter::new(order, false).write(&[ifd0]).unwrap()
}

/// A Pentax PEF: the raw image in IFD0, wide enough for the dark-border trim.
fn pef(compression: u16) -> Vec<u8> {
    let (w, h) = (264u32, 4u32);
    let mut ifd = IfdBuilder::new();
    ifd.set(t::MAKE, Value::Ascii("PENTAX".into()));
    ifd.set(t::IMAGE_WIDTH, Value::Long(vec![w]));
    ifd.set(t::IMAGE_LENGTH, Value::Long(vec![h]));
    ifd.set(t::BITS_PER_SAMPLE, Value::Short(vec![16]));
    ifd.set(t::COMPRESSION, Value::Short(vec![compression]));
    ifd.set(t::PHOTOMETRIC, Value::Short(vec![t::photometric::CFA]));
    let strip: Vec<u8> = (0..(w * h * 2) as usize).map(|i| if i % 528 < 8 { 0 } else { (i * 7 % 200) as u8 }).collect();
    ifd.set_image(ImageData::Strips { rows_per_strip: h, strips: vec![strip] });
    TiffWriter::new(ByteOrder::Big, false).write(&[ifd]).unwrap()
}

/// A Canon CR2: three placeholder IFDs and IFD3 with a 2-component lossless JPEG frame.
fn cr2() -> Vec<u8> {
    let img: Vec<u16> = (0..(W * H) as usize).map(|i| 1024 + (i * 7919 % 12000) as u16).collect();
    let enc = craft_raw::ljpeg::encode(&img, W as usize / 2, H as usize, 2, 14, 1, 0);
    let mut ifd0 = IfdBuilder::new();
    ifd0.set(t::MAKE, Value::Ascii("Canon".into()));
    let mut ifd3 = IfdBuilder::new();
    ifd3.set(t::COMPRESSION, Value::Short(vec![6]));
    ifd3.set(0xc640, Value::Short(vec![1, 16, 16]));
    ifd3.set_image(ImageData::Strips { rows_per_strip: H, strips: vec![enc] });
    let blank = || IfdBuilder::new().with(1, Value::Short(vec![0]));
    TiffWriter::new(ByteOrder::Little, false).write(&[ifd0, blank(), blank(), ifd3]).unwrap()
}

/// A Panasonic RW2: sensor size, bit depth and raw format in IFD0, 12-bit packed blocks.
fn rw2() -> Vec<u8> {
    let mut ifd = IfdBuilder::new();
    ifd.set(0x0002, Value::Short(vec![W as u16]));
    ifd.set(0x0003, Value::Short(vec![H as u16]));
    ifd.set(0x0004, Value::Short(vec![1]));
    ifd.set(0x0005, Value::Short(vec![2]));
    ifd.set(0x0006, Value::Short(vec![H as u16 - 1]));
    ifd.set(0x0007, Value::Short(vec![W as u16 - 2]));
    ifd.set(0x000a, Value::Short(vec![12]));
    ifd.set(0x002d, Value::Short(vec![5]));
    ifd.set(0x001c, Value::Short(vec![64]));
    ifd.set(0x001d, Value::Short(vec![64]));
    ifd.set(0x001e, Value::Short(vec![64]));
    ifd.set(t::MAKE, Value::Ascii("Panasonic".into()));
    let need = (W * H).div_ceil(10) as usize * 16;
    ifd.set_image(ImageData::Strips { rows_per_strip: H, strips: vec![(0..need).map(|i| (i * 29) as u8).collect()] });
    let mut b = TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).unwrap();
    b[..4].copy_from_slice(b"IIU\0");
    b
}

/// A Panasonic RW2 of raw format `format` (or, with `format` 0, the oldest 16-bit-word kind): sensor size, borders,
/// bit depth, black levels, an in-camera crop and `n` bytes of payload at the end, at `RawDataOffset`. Format 8 also
/// gets two strips covering the payload and a code table.
fn rw2_of(format: u16, bits: u16, n: usize) -> Vec<u8> {
    let u16s = |v: &[u16]| Value::Undefined(v.iter().flat_map(|x| x.to_le_bytes()).collect());
    let build = |off: u32| {
        let mut ifd = IfdBuilder::new();
        for (tag, v) in [(2, W as u16), (3, H as u16), (4, 2), (5, 4), (6, H as u16), (7, W as u16), (9, 2), (0x0a, bits), (0x1c, 128), (0x1d, 128)] {
            ifd.set(tag, Value::Short(vec![v]));
        }
        for (tag, v) in [(0x2f, 2u16), (0x30, 6), (0x31, H as u16 - 2), (0x32, W as u16 - 2)] {
            ifd.set(tag, Value::Short(vec![v]));
        }
        if format == 0 {
            ifd.set(0x000b, Value::Short(vec![34828]));
            ifd.set(t::STRIP_OFFSETS, Value::Long(vec![off]));
        } else {
            ifd.set(0x002d, Value::Short(vec![format]));
            ifd.set(0x0118, Value::Long(vec![off]));
        }
        if format == 8 {
            let table = [
                (6, 62),
                (7, 126),
                (6, 61),
                (5, 28),
                (4, 12),
                (3, 4),
                (3, 2),
                (2, 0),
                (3, 3),
                (3, 5),
                (4, 13),
                (5, 29),
                (6, 60),
                (8, 254),
                (8, 255),
                (12, 4094),
                (12, 4095),
            ];
            ifd.set(0x0040, u16s(&std::iter::once(17).chain(table.iter().flat_map(|&(l, c)| [l, c])).collect::<Vec<u16>>()));
            let half = (n / 2) as u32;
            let (a, b) = (off, off + half);
            ifd.set(0x0044, u16s(&[2, a as u16, (a >> 16) as u16, b as u16, (b >> 16) as u16]));
            ifd.set(0x0045, u16s(&[2, 0, 0, W as u16 / 2, 0]));
            ifd.set(0x0046, u16s(&[2, (half * 8) as u16, ((half * 8) >> 16) as u16, (half * 8) as u16, ((half * 8) >> 16) as u16]));
            ifd.set(0x0047, u16s(&[2, W as u16 / 2, W as u16 / 2]));
            ifd.set(0x0048, u16s(&[2, H as u16, H as u16]));
        }
        ifd.set(t::MAKE, Value::Ascii("Panasonic".into()));
        let mut b = TiffWriter::new(ByteOrder::Little, false).write(&[ifd]).unwrap();
        b[..4].copy_from_slice(b"IIU\0");
        b
    };
    let off = build(0).len();
    let mut b = build(off as u32);
    b.extend((0..n).map(|i| (i * 89 % 253) as u8));
    b
}

/// One RW2 per encoding with a sample decoder of its own (formats 4, 6 at 14 and 12 bits, 8, and 16-bit words).
fn rw2_formats() -> Vec<Vec<u8>> {
    vec![
        rw2_of(4, 12, (W * H).div_ceil(14) as usize * 16),
        rw2_of(6, 14, (W * H).div_ceil(11) as usize * 16),
        rw2_of(6, 12, (W * H).div_ceil(14) as usize * 16),
        rw2_of(8, 14, (W * H) as usize * 2),
        rw2_of(0, 12, (W * H) as usize * 2),
    ]
}

/// The RW2 samples decode as they are, so the mutations reach every format's sample decoder.
#[cfg(feature = "rw2")]
#[test]
fn rw2_samples_decode_unmutated() {
    for s in &rw2_formats() {
        let r = craft_raw::decode(s).unwrap();
        assert_eq!((r.width, r.height), (W as usize, H as usize));
    }
}

fn samples() -> Vec<Vec<u8>> {
    vec![
        rw2(),
        cr2(),
        cfa_tiff("NIKON CORPORATION", 1, 12, ByteOrder::Big),
        cfa_tiff("NIKON CORPORATION", 34713, 14, ByteOrder::Big),
        pef(1),
        pef(65535),
        cfa_tiff("SAMSUNG", 1, 12, ByteOrder::Little),
        cfa_tiff("SAMSUNG", 32769, 14, ByteOrder::Big),
        cfa_tiff("ACME", 1, 16, ByteOrder::Little),
    ]
    .into_iter()
    .chain(rw2_formats())
    .collect()
}

fn exercise(bytes: &[u8]) {
    let _ = craft_raw::probe(bytes);
    let _ = craft_raw::probe_info(bytes);
    let _ = craft_raw::embedded_preview(bytes);
    if let Ok(img) = craft_raw::decode(bytes) {
        assert!(img.validate().is_ok());
    }
}

#[test]
fn vendor_samples_truncated_at_every_length() {
    for s in samples() {
        exercise(&s);
        for n in (0..s.len()).step_by(3) {
            exercise(&s[..n]);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 2000, .. ProptestConfig::default() })]

    #[test]
    fn mutated_vendor_files_never_panic(kind in any::<prop::sample::Index>(), flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..16), cut in any::<usize>()) {
        let all = samples();
        let mut data = all[kind.index(all.len())].clone();
        let n = data.len();
        for (i, v) in flips {
            data[i % n] = v;
        }
        exercise(&data);
        exercise(&data[..cut % (n + 1)]);
    }
}
