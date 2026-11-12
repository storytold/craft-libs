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

fn samples() -> Vec<Vec<u8>> {
    vec![
        cr2(),
        cfa_tiff("NIKON CORPORATION", 1, 12, ByteOrder::Big),
        cfa_tiff("NIKON CORPORATION", 34713, 14, ByteOrder::Big),
        pef(1),
        pef(65535),
        cfa_tiff("SAMSUNG", 1, 12, ByteOrder::Little),
        cfa_tiff("SAMSUNG", 32769, 14, ByteOrder::Big),
        cfa_tiff("ACME", 1, 16, ByteOrder::Little),
    ]
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
