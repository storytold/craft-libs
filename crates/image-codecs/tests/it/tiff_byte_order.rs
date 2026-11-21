//! TIFF Options' Byte Order (little-endian `II`, big-endian `MM`) and Pixel
//! Order (interleaved, or Per Channel: `PlanarConfiguration` 2), TIFF 6.0 §2 and §8. Every
//! combination opens to the same image, metadata and pages as the default little-endian,
//! interleaved file.

use crate::common::*;
use craft_image_codecs::*;

const LAYOUTS: [ChannelLayout; 7] = [
    ChannelLayout::Gray,
    ChannelLayout::GrayA,
    ChannelLayout::Rgb,
    ChannelLayout::Rgba,
    ChannelLayout::Cmyk,
    ChannelLayout::CmykA,
    ChannelLayout::Lab,
];
const COMPRESSIONS: [TiffCompression; 4] = [TiffCompression::None, TiffCompression::Lzw, TiffCompression::Deflate, TiffCompression::PackBits];
const ORDERS: [(TiffByteOrder, TiffPixelOrder); 4] = [
    (TiffByteOrder::LittleEndian, TiffPixelOrder::Interleaved),
    (TiffByteOrder::LittleEndian, TiffPixelOrder::PerChannel),
    (TiffByteOrder::BigEndian, TiffPixelOrder::Interleaved),
    (TiffByteOrder::BigEndian, TiffPixelOrder::PerChannel),
];

fn opts(order: TiffByteOrder, pixels: TiffPixelOrder, compression: TiffCompression) -> EncodeOptions {
    EncodeOptions { tiff_byte_order: order, tiff_pixel_order: pixels, tiff_compression: compression, ..Default::default() }
}

/// The parsed first directory of a TIFF file.
fn ifd0(bytes: &[u8]) -> craft_tiff::Ifd {
    let t = craft_tiff::Tiff::parse(bytes).unwrap();
    t.ifds.into_iter().next().unwrap()
}

#[test]
fn the_defaults_are_little_endian_and_interleaved() {
    let o = EncodeOptions::default();
    assert_eq!((o.tiff_byte_order, o.tiff_pixel_order), (TiffByteOrder::LittleEndian, TiffPixelOrder::Interleaved));
    assert!(tiff_writes_little_endian());
    let bytes = encode(&test_image(ChannelLayout::Rgb, SampleType::U8), Format::Tiff, &o).unwrap();
    assert!(bytes.starts_with(b"II*\0"));
    let info = tiff_info(&bytes).unwrap();
    assert!(info.little_endian && !info.pages[0].planar);
}

#[test]
fn names_parse_back_and_nothing_else_does() {
    for o in [TiffByteOrder::LittleEndian, TiffByteOrder::BigEndian] {
        assert_eq!(TiffByteOrder::parse(o.name()), Some(o));
    }
    for o in [TiffPixelOrder::Interleaved, TiffPixelOrder::PerChannel] {
        assert_eq!(TiffPixelOrder::parse(o.name()), Some(o));
    }
    assert_eq!((TiffByteOrder::LittleEndian.name(), TiffByteOrder::BigEndian.name()), ("little", "big"));
    assert_eq!((TiffPixelOrder::Interleaved.name(), TiffPixelOrder::PerChannel.name()), ("interleaved", "perChannel"));
    for bad in ["", "LITTLE", "ibm", "mac", "big-endian", "planar", "per channel"] {
        assert_eq!(TiffByteOrder::parse(bad), None);
        assert_eq!(TiffPixelOrder::parse(bad), None);
    }
}

#[test]
fn every_depth_layout_compression_and_order_opens_to_the_same_image() {
    for layout in LAYOUTS {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let img = synth(37, 23, layout, sample, 7, 0.3);
            for compression in COMPRESSIONS {
                let reference =
                    decode(&encode(&img, Format::Tiff, &opts(TiffByteOrder::LittleEndian, TiffPixelOrder::Interleaved, compression)).unwrap())
                        .unwrap();
                for (order, pixels) in ORDERS {
                    let case = format!("{layout:?} {sample:?} {compression:?} {order:?} {pixels:?}");
                    let bytes = encode(&img, Format::Tiff, &opts(order, pixels, compression)).unwrap_or_else(|e| panic!("{case}: {e}"));
                    assert_eq!(&bytes[..2], if order.is_little() { b"II" } else { b"MM" }, "{case}");
                    let info = tiff_info(&bytes).unwrap();
                    assert_eq!(info.little_endian, order.is_little(), "{case}");
                    assert_eq!(info.pages.len(), 1, "{case}");
                    assert_eq!(info.pages[0].planar, pixels == TiffPixelOrder::PerChannel, "{case}");
                    let code = match compression {
                        TiffCompression::None => 1,
                        TiffCompression::Lzw => 5,
                        TiffCompression::Deflate => 8,
                        TiffCompression::PackBits => 32773,
                    };
                    assert_eq!(info.pages[0].compression, code, "{case}");
                    let back = decode(&bytes).unwrap_or_else(|e| panic!("{case}: {e}"));
                    assert_eq!(
                        (back.layout(), back.sample_type(), back.dimensions()),
                        (reference.layout(), reference.sample_type(), reference.dimensions()),
                        "{case}"
                    );
                    assert!(back.data() == reference.data(), "{case}: samples differ");
                }
            }
        }
    }
}

#[test]
fn per_channel_has_one_set_of_strips_per_channel_and_predicts_within_a_channel() {
    // Tall enough for several strips per channel.
    let img = synth(700, 900, ChannelLayout::Rgba, SampleType::U16, 3, 0.1);
    for order in [TiffByteOrder::LittleEndian, TiffByteOrder::BigEndian] {
        let bytes = encode(&img, Format::Tiff, &opts(order, TiffPixelOrder::PerChannel, TiffCompression::Lzw)).unwrap();
        let d = ifd0(&bytes);
        assert_eq!(d.u16(284), Some(2));
        assert_eq!(d.u16(317), Some(2), "16-bit LZW is predicted, as in the interleaved file");
        let rows = u64::from(d.u32(278).unwrap());
        let strips = 900u64.div_ceil(rows);
        assert!(strips > 1, "{rows} rows per strip");
        assert_eq!(d.u64s(273).unwrap().len() as u64, strips * 4);
        assert_eq!(d.u64s(279).unwrap().len() as u64, strips * 4);
        assert_eq!(decode(&bytes).unwrap().data(), img.data(), "{order:?}");
    }
    // Floats are not predicted, in either order (the `tiff` encoder's rule).
    let f = synth(16, 8, ChannelLayout::Rgb, SampleType::F32, 3, 0.1);
    let bytes = encode(&f, Format::Tiff, &opts(TiffByteOrder::BigEndian, TiffPixelOrder::PerChannel, TiffCompression::Deflate)).unwrap();
    assert_eq!(ifd0(&bytes).u16(317), Some(1));
}

#[test]
fn big_endian_samples_are_big_endian() {
    // One 16-bit gray pixel row, uncompressed: the strip holds the samples most significant byte
    // first.
    let img =
        Image::from_raw(2, 1, ChannelLayout::Gray, SampleType::U16, [0x1234u16, 0xABCD].iter().flat_map(|v| v.to_ne_bytes()).collect()).unwrap();
    let bytes = encode(&img, Format::Tiff, &opts(TiffByteOrder::BigEndian, TiffPixelOrder::Interleaved, TiffCompression::None)).unwrap();
    let d = ifd0(&bytes);
    let at = d.u64(273).unwrap() as usize;
    assert_eq!(&bytes[at..at + 4], &[0x12, 0x34, 0xAB, 0xCD]);
    assert_eq!(d.u16(258), Some(16));
}

#[test]
fn extra_samples_keep_their_planes() {
    for layout in [ChannelLayout::Rgb, ChannelLayout::Rgba, ChannelLayout::Cmyk, ChannelLayout::Gray] {
        for sample in [SampleType::U8, SampleType::U16, SampleType::F32] {
            let mut img = synth(5, 4, layout, sample, 2, 0.4);
            let plane: Vec<u8> = (0..20 * sample.bytes()).map(|i| (i * 31 % 251) as u8).collect();
            let plane =
                if sample == SampleType::F32 { plane.chunks(4).flat_map(|c| (f32::from(c[0]) / 255.0).to_ne_bytes()).collect() } else { plane };
            img.meta.extra_samples =
                vec![ExtraSample { kind: 0, data: plane.clone() }, ExtraSample { kind: 0, data: plane.iter().rev().copied().collect() }];
            let reference = decode(&encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap()).unwrap();
            for (order, pixels) in ORDERS {
                for compression in COMPRESSIONS {
                    let case = format!("{layout:?} {sample:?} {order:?} {pixels:?} {compression:?}");
                    let back = decode(&encode(&img, Format::Tiff, &opts(order, pixels, compression)).unwrap()).unwrap();
                    assert_eq!(back.data(), reference.data(), "{case}");
                    assert_eq!(back.meta.extra_samples, reference.meta.extra_samples, "{case}");
                }
            }
        }
    }
}

/// A little-endian EXIF block: IFD0 with Make and a pointer to an Exif directory holding
/// ExposureTime (RATIONAL) and ISOSpeedRatings (SHORT).
fn exif_block() -> Vec<u8> {
    let mut b = b"II*\0\x08\0\0\0".to_vec();
    let entry = |b: &mut Vec<u8>, tag: u16, ty: u16, count: u32, value: [u8; 4]| {
        b.extend(tag.to_le_bytes());
        b.extend(ty.to_le_bytes());
        b.extend(count.to_le_bytes());
        b.extend(value);
    };
    b.extend(2u16.to_le_bytes());
    entry(&mut b, 271, 2, 6, 38u32.to_le_bytes());
    entry(&mut b, 34665, 4, 1, 44u32.to_le_bytes());
    b.extend(0u32.to_le_bytes());
    b.extend(b"Craft\0");
    b.extend(2u16.to_le_bytes());
    entry(&mut b, 33434, 5, 1, 74u32.to_le_bytes());
    entry(&mut b, 34855, 3, 1, [100, 0, 0, 0]);
    b.extend(0u32.to_le_bytes());
    b.extend(1u32.to_le_bytes());
    b.extend(250u32.to_le_bytes());
    b
}

/// Every entry of every directory of `block`, as typed values, sorted.
fn exif_values(block: &[u8]) -> Vec<(u16, craft_tiff::Value)> {
    let t = craft_tiff::Tiff::parse(block).unwrap();
    let mut v: Vec<_> =
        t.all_ifds().into_iter().flat_map(|d| d.entries.iter().filter(|e| e.tag != 34665).map(|e| (e.tag, e.value.clone()))).collect();
    v.sort_by_key(|(tag, _)| *tag);
    v
}

#[test]
fn metadata_and_photoshop_tags_survive_either_order() {
    let mut img = synth(9, 7, ChannelLayout::Rgb, SampleType::U16, 5, 0.3);
    img.icc = Some(sample_icc(600));
    img.meta.xmp = Some(SAMPLE_XMP.to_string());
    img.meta.dpi = Some((300.0, 150.0));
    img.meta.text = vec![("Software".into(), "Synthetic".into()), ("Artist".into(), "Nobody".into())];
    img.meta.exif = Some(exif_block());
    img.meta.photoshop_resources =
        Some(b"8BIM\x03\xed\x00\x00\x00\x00\x00\x10\x00\x48\x00\x00\x00\x01\x00\x01\x00\x48\x00\x00\x00\x01\x00\x01".to_vec());
    img.meta.photoshop_layers = Some(b"Adobe Photoshop Document Data Block\0opaque".to_vec());
    let reference = decode(&encode(&img, Format::Tiff, &EncodeOptions::default()).unwrap()).unwrap();
    assert!(reference.meta.exif.is_some());
    for (order, pixels) in ORDERS {
        for compression in COMPRESSIONS {
            let case = format!("{order:?} {pixels:?} {compression:?}");
            let bytes = encode(&img, Format::Tiff, &opts(order, pixels, compression)).unwrap();
            let back = decode(&bytes).unwrap();
            assert_eq!(back.icc, reference.icc, "{case}");
            assert_eq!(back.meta.xmp, reference.meta.xmp, "{case}");
            assert_eq!(back.meta.dpi, reference.meta.dpi, "{case}");
            assert_eq!(back.meta.text, reference.meta.text, "{case}");
            // The block is rebuilt in the file's byte order: compare what it says.
            assert_eq!(exif_values(back.meta.exif.as_deref().unwrap()), exif_values(reference.meta.exif.as_deref().unwrap()), "{case}");
            // Photoshop's tags are copied as they are.
            assert_eq!(tiff_photoshop_tags(&bytes), (img.meta.photoshop_resources.as_deref(), img.meta.photoshop_layers.as_deref()), "{case}");
            assert_eq!(back.meta.photoshop_resources, reference.meta.photoshop_resources, "{case}");
            assert_eq!(back.meta.photoshop_layers, reference.meta.photoshop_layers, "{case}");
            assert_eq!(back.data(), reference.data(), "{case}");
        }
    }
}

#[test]
fn pyramid_levels_and_bigtiff_follow_the_order() {
    let img = synth(300, 200, ChannelLayout::Rgba, SampleType::U8, 9, 0.2);
    let pyramid = EncodeOptions { tiff_pyramid: true, ..Default::default() };
    let reference = encode(&img, Format::Tiff, &pyramid).unwrap();
    let kinds = |b: &[u8]| tiff_info(b).unwrap().pages.iter().map(|p| (p.kind, p.width, p.height, p.planar)).collect::<Vec<_>>();
    for (order, pixels) in ORDERS {
        for bigtiff in [false, true] {
            let case = format!("{order:?} {pixels:?} bigtiff {bigtiff}");
            let o = EncodeOptions {
                tiff_byte_order: order,
                tiff_pixel_order: pixels,
                tiff_bigtiff: bigtiff,
                tiff_compression: TiffCompression::Deflate,
                ..pyramid.clone()
            };
            let bytes = encode(&img, Format::Tiff, &o).unwrap();
            let info = tiff_info(&bytes).unwrap();
            assert_eq!((info.big_tiff, info.little_endian), (bigtiff, order.is_little()), "{case}");
            let planar = pixels == TiffPixelOrder::PerChannel;
            let want: Vec<_> = kinds(&reference).into_iter().map(|(k, w, h, _)| (k, w, h, planar)).collect();
            assert_eq!(kinds(&bytes), want, "{case}");
            for page in 0..want.len() {
                let a = decode_tiff_page(&bytes, page, &DecodeOptions::default()).unwrap();
                let b = decode_tiff_page(&reference, page, &DecodeOptions::default()).unwrap();
                assert_eq!(a.data(), b.data(), "{case} page {page}");
            }
        }
    }
}
