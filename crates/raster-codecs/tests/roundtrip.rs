//! Every format with a writer reads back what it wrote, and no reader panics on damaged files.

use craft_raster_codecs::dds::{DdsCompression, DdsOptions};
use craft_raster_codecs::{ChannelLayout as L, CodecError, Image, Limits, SampleType as S, dds, dicom, fits, iff, pcx, pnm, xpm};

/// SplitMix64.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
}

/// Noise in every sample; floats in 0..=1.
fn synth(w: u32, h: u32, layout: L, sample: S) -> Image {
    let mut rng = Rng(u64::from(w) * 31 + layout.channels() as u64);
    let n = (w * h) as usize * layout.channels();
    match sample {
        S::U8 => Image::from_u8(w, h, layout, (0..n).map(|_| rng.next() as u8).collect()),
        S::U16 => Image::from_u16(w, h, layout, &(0..n).map(|_| rng.next() as u16).collect::<Vec<_>>()),
        S::F32 => Image::from_f32(w, h, layout, &(0..n).map(|_| (rng.next() >> 40) as f32 / (1u64 << 24) as f32).collect::<Vec<_>>()),
    }
    .unwrap()
}

type Decode = fn(&[u8], &Limits) -> Result<Image, CodecError>;

/// Each writer's output for each layout and depth it stores, with its reader.
fn written() -> Vec<(&'static str, Decode, Image, Vec<u8>)> {
    let mut v = Vec::new();
    for layout in [L::Gray, L::GrayA, L::Rgb, L::Rgba, L::Cmyk, L::CmykA] {
        for sample in [S::U8, S::U16] {
            let img = synth(19, 13, layout, sample);
            v.push(("PNM", pnm::decode as Decode, img.clone(), pnm::encode(&img).unwrap()));
        }
    }
    for layout in [L::Gray, L::Rgb] {
        let img = synth(19, 13, layout, S::F32);
        v.push(("PFM", pnm::decode, img.clone(), pnm::encode(&img).unwrap()));
        for sample in [S::U8, S::U16, S::F32] {
            let img = synth(19, 13, layout, sample);
            v.push(("FITS", fits::decode, img.clone(), fits::encode(&img).unwrap()));
        }
    }
    for layout in [L::Gray, L::GrayA, L::Rgb, L::Rgba] {
        let img = synth(19, 13, layout, S::U8);
        v.push(("DDS", dds::decode, img.clone(), dds::encode(&img, DdsOptions::default()).unwrap()));
    }
    for layout in [L::Rgb, L::Rgba] {
        let mut img = synth(19, 13, layout, S::U8);
        if layout == L::Rgba {
            // An ILBM mask plane is one bit per pixel.
            let data = img.data().chunks(4).flat_map(|p| [p[0], p[1], p[2], if p[3] < 128 { 0 } else { 255 }]).collect();
            img = Image::from_u8(19, 13, layout, data).unwrap();
        }
        v.push(("IFF", iff::decode, img.clone(), iff::encode(&img).unwrap()));
    }
    v
}

#[test]
fn writers_round_trip_exactly() {
    for (name, decode, img, bytes) in written() {
        let back = decode(&bytes, &Limits::default()).unwrap_or_else(|e| panic!("{name} {:?} {:?}: {e}", img.layout(), img.sample_type()));
        assert_eq!((back.dimensions(), back.layout(), back.sample_type()), (img.dimensions(), img.layout(), img.sample_type()), "{name}");
        assert_eq!(back.data(), img.data(), "{name} {:?} {:?}", img.layout(), img.sample_type());
    }
}

#[test]
fn writers_refuse_what_the_format_cannot_store() {
    let img = |l, s| synth(2, 2, l, s);
    assert!(matches!(pnm::encode(&img(L::Rgba, S::F32)), Err(CodecError::Encode { .. })));
    assert!(matches!(fits::encode(&img(L::Rgba, S::U8)), Err(CodecError::Encode { .. })));
    assert!(matches!(iff::encode(&img(L::Gray, S::U8)), Err(CodecError::Encode { .. })));
    assert!(matches!(iff::encode(&img(L::Rgb, S::U16)), Err(CodecError::Encode { .. })));
    assert!(matches!(dds::encode(&img(L::Cmyk, S::U8), DdsOptions::default()), Err(CodecError::Encode { .. })));
}

/// Valid files of every format: the writers' output, block-compressed DDS, and hand-made files
/// for the read-only formats.
fn samples() -> Vec<(Decode, Vec<u8>)> {
    let mut v: Vec<(Decode, Vec<u8>)> = written().into_iter().map(|(_, d, _, b)| (d, b)).collect();
    for compression in [DdsCompression::Bc1, DdsCompression::Bc2, DdsCompression::Bc3] {
        v.push((dds::decode, dds::encode(&synth(19, 13, L::Rgba, S::U8), DdsOptions { compression, mipmaps: true }).unwrap()));
    }
    v.push((pnm::decode, b"P3\n2 2\n255\n0 1 2 3 4 5 6 7 8 9 10 11\n".to_vec()));
    v.push((xpm::decode, b"/* XPM */\nstatic char *x[] = {\n\"2 2 2 1\",\n\". c #102030\",\n\"# c None\",\n\".#\",\n\"#.\",\n};\n".to_vec()));
    v.push((xpm::decode, b"! XPM2\n2 1 2 1\na c red\nb c #00ff00\nab\n".to_vec()));
    // PCX: version 5, RLE, 8 bits, one plane, 4x2, a VGA palette after the rows.
    let mut p = vec![0u8; 128];
    p[..4].copy_from_slice(&[0x0a, 5, 1, 8]);
    p[8..12].copy_from_slice(&[3, 0, 1, 0]);
    p[65] = 1;
    p[66] = 4;
    p.extend([1, 2, 3, 4, 4, 3, 2, 1, 0x0c]);
    p.extend((0..768).map(|i| i as u8));
    v.push((pcx::decode, p));
    // DICOM: explicit VR little endian, 2x2 8-bit MONOCHROME2.
    let mut d = vec![0u8; 128];
    d.extend(b"DICM");
    let el = |d: &mut Vec<u8>, group: u16, element: u16, vr: &[u8; 2], value: &[u8]| {
        d.extend(group.to_le_bytes());
        d.extend(element.to_le_bytes());
        d.extend(vr);
        if matches!(vr, b"OB" | b"OW" | b"UN" | b"SQ") {
            d.extend([0, 0]);
            d.extend((value.len() as u32).to_le_bytes());
        } else {
            d.extend((value.len() as u16).to_le_bytes());
        }
        d.extend(value);
    };
    el(&mut d, 0x0002, 0x0010, b"UI", b"1.2.840.10008.1.2.1\0");
    el(&mut d, 0x0028, 0x0002, b"US", &1u16.to_le_bytes());
    el(&mut d, 0x0028, 0x0004, b"CS", b"MONOCHROME2 ");
    el(&mut d, 0x0028, 0x0010, b"US", &2u16.to_le_bytes());
    el(&mut d, 0x0028, 0x0011, b"US", &2u16.to_le_bytes());
    el(&mut d, 0x0028, 0x0100, b"US", &8u16.to_le_bytes());
    el(&mut d, 0x0028, 0x0101, b"US", &8u16.to_le_bytes());
    el(&mut d, 0x7fe0, 0x0010, b"OB", &[1, 2, 3, 4]);
    v.push((dicom::decode, d));
    v
}

fn tight() -> Limits {
    Limits { max_width: 4096, max_height: 4096, max_pixels: 1 << 22, max_alloc: 64 << 20 }
}

#[test]
fn every_sample_decodes() {
    for (i, (decode, bytes)) in samples().into_iter().enumerate() {
        decode(&bytes, &tight()).unwrap_or_else(|e| panic!("sample {i}: {e}"));
    }
}

#[test]
fn truncated_at_many_offsets() {
    for (decode, bytes) in samples() {
        let n = bytes.len();
        let mut cuts: Vec<usize> = (0..n.min(80)).collect();
        cuts.extend((0..n).step_by((n / 97).max(1)));
        cuts.extend(n.saturating_sub(40)..n);
        for cut in cuts {
            let _ = decode(&bytes[..cut], &tight());
        }
    }
}

#[test]
fn bit_flips_never_panic() {
    let mut rng = Rng(42);
    for (decode, bytes) in samples() {
        for _ in 0..150 {
            let mut b = bytes.clone();
            for _ in 0..1 + rng.next() % 4 {
                let i = rng.next() as usize % b.len();
                b[i] ^= 1 << (rng.next() % 8);
            }
            let _ = decode(&b, &tight());
        }
    }
}

#[test]
fn byte_overwrites_in_header_never_panic() {
    for (decode, bytes) in samples() {
        for i in 0..bytes.len().min(160) {
            for v in [0x00, 0x7F, 0xFF] {
                let mut b = bytes.clone();
                b[i] = v;
                let _ = decode(&b, &tight());
            }
        }
    }
}

#[test]
fn empty_and_foreign_input_is_an_error() {
    for decode in [dds::decode as Decode, dicom::decode, fits::decode, iff::decode, pcx::decode, pnm::decode, xpm::decode] {
        assert!(decode(&[], &tight()).is_err());
        assert!(decode(b"\x89PNG\r\n\x1a\n", &tight()).is_err());
    }
}
