//! The bounded strip/tile reader (`dng_lines`) against the full DNG reader: every layout the writer
//! and the hand-built variants produce reads to the same samples, and the layouts the bounded reader
//! does not take are refused with a reason.

use super::dng::{base_ifd, chunk_samples, dng, layout_image, pack_msb, pattern, synthetic, u16_bytes};
use craft_raw::dng_lines::{self, Error};
use craft_raw::dngwrite::{DngImage, write_lossy_dng};
use craft_raw::{Cfa, DngCompression, DngWriteOptions, RawData, RawError, decode, write_dng};
use craft_tiff::classic::Tiff;
use craft_tiff::tags as t;
use craft_tiff::{ByteOrder, ImageData, Value};

fn lines(bytes: &[u8]) -> Result<dng_lines::RawImage, Error> {
    let (tiff, first) = Tiff::new(bytes)?;
    let ifds = tiff.all_ifds(first)?;
    let ifd = dng_lines::find_raw_ifd(&tiff, &ifds)?;
    dng_lines::read(&tiff, ifd)
}

fn full(bytes: &[u8]) -> Vec<u16> {
    match decode(bytes).unwrap().data {
        RawData::U16(v) => v,
        other => panic!("integer samples expected, got {other:?}"),
    }
}

#[test]
fn uncompressed_layouts_match_the_full_reader() {
    let (w, h) = (37, 23);
    for cpp in [1usize, 3] {
        for bits in [8u32, 12, 16] {
            let px = pattern(w, h, cpp, bits);
            for tile in [(16, 16), (w, 5), (w, h)] {
                for order in [ByteOrder::Little, ByteOrder::Big] {
                    let chunks = chunk_samples(&px, w, h, cpp, tile.0, tile.1, false)
                        .iter()
                        .map(|c| match bits {
                            8 => c.iter().map(|&v| v as u8).collect(),
                            16 => u16_bytes(c, order),
                            _ => pack_msb(c, bits, tile.0 * cpp),
                        })
                        .collect();
                    let mut raw = base_ifd(w, h, bits as u16, cpp, cpp == 1);
                    raw.set(t::COMPRESSION, Value::Short(vec![1]));
                    layout_image(&mut raw, chunks, w, tile);
                    let bytes = dng(raw, order);
                    let img = lines(&bytes).unwrap();
                    let what = format!("cpp {cpp} bits {bits} tile {tile:?} {order:?}");
                    assert_eq!((img.width, img.height, img.samples_per_pixel, img.bits), (w, h, cpp, bits), "{what}");
                    assert_eq!(img.data, px, "{what}");
                    assert_eq!(img.data, full(&bytes), "{what}");
                }
            }
        }
    }
}

#[test]
fn written_dngs_read_back_in_every_compression() {
    for (w, h) in [(64, 48), (33, 17)] {
        for cfa in [Some(Cfa::bayer("RGGB").unwrap()), None] {
            let cpp = if cfa.is_some() { 1 } else { 3 };
            let raw = synthetic(w, h, cfa, cpp);
            for comp in [DngCompression::Uncompressed, DngCompression::Lj92 { tile: 16 }, DngCompression::Lj92 { tile: 256 }] {
                for order in [ByteOrder::Little, ByteOrder::Big] {
                    let bytes = write_dng(&raw, &DngWriteOptions { compression: comp, order, xmp: None, ..Default::default() }).unwrap();
                    let img = lines(&bytes).unwrap();
                    assert_eq!(RawData::U16(img.data.clone()), raw.data, "{w}x{h} cpp {cpp} {comp:?} {order:?}");
                    assert_eq!(img.data, full(&bytes));
                }
            }
        }
    }
}

#[test]
fn lossy_jpeg_tiles_read_like_the_full_reader() {
    let (w, h) = (40, 21);
    let mut raw = synthetic(w, h, None, 3);
    raw.bits = 8;
    raw.data = RawData::U16(pattern(w, h, 3, 8));
    raw.white = vec![255.0];
    raw.black = craft_raw::BlackLevel::uniform(0.0);
    let linearization: Vec<u16> = (0..256).map(|v| v * 257).collect();
    let encode = |pixels: Vec<u8>, tile: u32, quality: u8| {
        let mut out = Vec::new();
        jpeg_encoder::Encoder::new(&mut out, quality)
            .encode(&pixels, tile as u16, tile as u16, jpeg_encoder::ColorType::Rgb)
            .map_err(|e| RawError::Corrupt(e.to_string()))?;
        Ok(out)
    };
    for tile in [16, 64] {
        let bytes = write_lossy_dng(&DngImage::from(&raw), &DngWriteOptions::default(), tile, 90, &linearization, &encode).unwrap();
        let img = lines(&bytes).unwrap();
        assert_eq!((img.width, img.height, img.samples_per_pixel, img.bits), (w, h, 3, 8));
        // the same decoder as the full reader, so the same 8-bit codes; the full reader also
        // applies the LinearizationTable, the bounded reader leaves that to its caller
        let codes: Vec<u16> = img.data.iter().map(|&v| linearization[usize::from(v)]).collect();
        assert_eq!(codes, full(&bytes), "tile {tile}");
    }
    // a lossy tile that is not a JPEG
    let mut bad = base_ifd(16, 16, 8, 3, false);
    bad.set(t::COMPRESSION, Value::Short(vec![34892]));
    bad.set_image(ImageData::Tiles { tile_width: 16, tile_height: 16, tiles: vec![vec![0x55; 512]] });
    assert!(matches!(lines(&dng(bad, ByteOrder::Little)), Err(Error::Malformed(_) | Error::Unsupported(_))));
}

fn refused(edit: impl FnOnce(&mut craft_tiff::IfdBuilder), cpp: usize, cfa: bool) -> Error {
    let (w, h) = (8, 4);
    let mut raw = base_ifd(w, h, 16, cpp, cfa);
    raw.set(t::COMPRESSION, Value::Short(vec![1]));
    raw.set_image(ImageData::Strips { rows_per_strip: h as u32, strips: vec![vec![0; w * h * cpp * 2]] });
    edit(&mut raw);
    lines(&dng(raw, ByteOrder::Little)).unwrap_err()
}

#[test]
fn unsupported_and_damaged_layouts_are_refused() {
    let unsupported = |e: Error| matches!(e, Error::Unsupported(_));
    assert!(unsupported(refused(
        |r| {
            r.set(t::COMPRESSION, Value::Short(vec![8]));
        },
        1,
        true
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::COMPRESSION, Value::Short(vec![52546]));
        },
        1,
        true
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::COMPRESSION, Value::Short(vec![5]));
        },
        1,
        true
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::COMPRESSION, Value::Short(vec![34892]));
        },
        1,
        true
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::COMPRESSION, Value::Short(vec![34892]));
        },
        3,
        false
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::PREDICTOR, Value::Short(vec![2]));
        },
        1,
        true
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::PLANAR_CONFIGURATION, Value::Short(vec![2]));
        },
        3,
        false
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::SAMPLE_FORMAT, Value::Short(vec![3; 3]));
        },
        3,
        false
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::BITS_PER_SAMPLE, Value::Short(vec![16, 12, 16]));
        },
        3,
        false
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::BITS_PER_SAMPLE, Value::Short(vec![32]));
        },
        1,
        true
    )));
    assert!(unsupported(refused(
        |r| {
            r.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![5]));
        },
        1,
        false
    )));
    assert!(matches!(
        refused(
            |r| {
                r.set(t::SAMPLES_PER_PIXEL, Value::Short(vec![3]));
            },
            1,
            true
        ),
        Error::Malformed(_)
    ));
    assert!(matches!(
        refused(
            |r| {
                r.set(t::IMAGE_WIDTH, Value::Long(vec![0]));
            },
            1,
            true
        ),
        Error::Malformed(_)
    ));
    // a strip shorter than its rows, and fewer strips than the image needs
    let short = |r: &mut craft_tiff::IfdBuilder| {
        r.set_image(ImageData::Strips { rows_per_strip: 4, strips: vec![vec![0; 10]] });
    };
    assert_eq!(refused(short, 1, true), Error::Truncated);
    let few = |r: &mut craft_tiff::IfdBuilder| {
        r.set_image(ImageData::Strips { rows_per_strip: 1, strips: vec![vec![0; 16]] });
    };
    assert!(matches!(refused(few, 1, true), Error::Malformed(_)));
    // no full-resolution CFA or LinearRaw IFD at all
    let none = |r: &mut craft_tiff::IfdBuilder| {
        r.set(t::NEW_SUBFILE_TYPE, Value::Long(vec![1]));
    };
    assert!(unsupported(refused(none, 1, true)));
    assert_eq!(lines(b"not a tiff at all").unwrap_err(), Error::NotTiff);
}

#[test]
fn unpack_reads_every_depth_in_both_byte_orders() {
    assert_eq!(dng_lines::unpack(&[1, 2, 3, 4], 2, 1, 1, 16, false).unwrap(), [0x0201, 0x0403]);
    assert_eq!(dng_lines::unpack(&[1, 2, 3, 4], 2, 1, 1, 16, true).unwrap(), [0x0102, 0x0304]);
    assert_eq!(dng_lines::unpack(&[7, 9], 1, 2, 1, 8, false).unwrap(), [7, 9]);
    // rows start on a byte boundary: 3 x 4 bits per row is two bytes
    assert_eq!(dng_lines::unpack(&[0x12, 0x30, 0x45, 0x60], 3, 2, 1, 4, true).unwrap(), [1, 2, 3, 4, 5, 6]);
    assert_eq!(dng_lines::unpack(&[0x12], 3, 1, 1, 4, true).unwrap_err(), Error::Truncated);
}

#[test]
fn a_lossless_jpeg_larger_than_its_tile_is_refused_before_decoding_it_all() {
    // 16 x 16 tiles each holding a 512 x 512 lossless JPEG: the decoder may allocate only the
    // tile's samples (and a little slack), not the frame the JPEG claims
    let big = craft_raw::ljpeg::encode(&vec![7u16; 512 * 512], 512, 512, 1, 16, 1, 0);
    let mut raw = base_ifd(32, 16, 16, 1, true);
    raw.set(t::COMPRESSION, Value::Short(vec![7]));
    raw.set_image(ImageData::Tiles { tile_width: 16, tile_height: 16, tiles: vec![big.clone(), big] });
    assert!(matches!(lines(&dng(raw, ByteOrder::Little)), Err(Error::TooLarge | Error::Malformed(_))));
    // the same tiles coded at their own size decode
    let fit = craft_raw::ljpeg::encode(&vec![7u16; 16 * 16], 16, 16, 1, 16, 1, 0);
    let mut raw = base_ifd(32, 16, 16, 1, true);
    raw.set(t::COMPRESSION, Value::Short(vec![7]));
    raw.set_image(ImageData::Tiles { tile_width: 16, tile_height: 16, tiles: vec![fit.clone(), fit] });
    assert_eq!(lines(&dng(raw, ByteOrder::Little)).unwrap().data, vec![7; 32 * 16]);
}

#[test]
fn short_lossy_jpeg_strips_are_charged_their_own_rows() {
    // 8192 x 4096 RGB (below MAX_SAMPLES) in 16-row lossy JPEG strips: each strip's JPEG is exactly
    // 16 rows, so the decode budget charges 16 rows, not the 32 of the MCU padding allowance (which
    // would put the image over the aggregate budget and refuse it)
    let (w, h, rows) = (8192usize, 4096usize, 16usize);
    let strip: Vec<u8> = (0..w * rows).flat_map(|i| [(i % 251) as u8, 90, 160]).collect();
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 90).encode(&strip, w as u16, rows as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    let mut raw = base_ifd(w, h, 8, 3, false);
    raw.set(t::COMPRESSION, Value::Short(vec![34892]));
    raw.set_image(ImageData::Strips { rows_per_strip: rows as u32, strips: vec![jpeg; h / rows] });
    let img = lines(&dng(raw, ByteOrder::Little)).unwrap();
    assert_eq!((img.width, img.height, img.samples_per_pixel), (w, h, 3));
    assert_eq!(img.data.len(), w * h * 3);
    // every strip holds the same picture
    let band = w * rows * 3;
    assert_eq!(&img.data[..band], &img.data[img.data.len() - band..]);
}

#[test]
fn a_small_lossy_jpeg_in_a_huge_tile_is_charged_the_whole_tile() {
    // a 256 x 256 RGB image in one 16384 x 16384 tile whose JPEG is only 256 x 256 (padded with
    // comments to pass the bytes-per-sample guard): the JPEG's frame is small, but decoding fills
    // the whole tile (about 1.5 GiB of samples), so it is refused before anything is allocated
    let (w, tile) = (256usize, 16384usize);
    let px: Vec<u8> = (0..w * w).flat_map(|i| [(i % 251) as u8, 90, 160]).collect();
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 90).encode(&px, w as u16, w as u16, jpeg_encoder::ColorType::Rgb).unwrap();
    let mut padded = jpeg[..2].to_vec();
    let need = tile * tile * 3 / 768 + 1;
    while padded.len() < need {
        padded.extend_from_slice(&[0xFF, 0xFE, 0xFF, 0xFF]);
        padded.extend(std::iter::repeat_n(0u8, 0xFFFF - 2));
    }
    padded.extend_from_slice(&jpeg[2..]);
    let mut raw = base_ifd(w, w, 8, 3, false);
    raw.set(t::COMPRESSION, Value::Short(vec![34892]));
    raw.set_image(ImageData::Tiles { tile_width: tile as u32, tile_height: tile as u32, tiles: vec![padded] });
    assert_eq!(lines(&dng(raw, ByteOrder::Little)).unwrap_err(), Error::TooLarge);
}
