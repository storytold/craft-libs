//! Writing DDS textures: every storage read back through the reader, the header fields, sizes
//! that aren't a multiple of four, the mip chain, and the block-compression error on ramps.

use craft_raster_codecs::dds::{DdsCompression, DdsOptions, encode};
use craft_raster_codecs::{ChannelLayout, CodecError, Image, Limits};

fn decode(b: &[u8]) -> Result<Image, CodecError> {
    craft_raster_codecs::dds::decode(b, &Limits::default())
}

const DDSD_PITCH: u32 = 0x8;
const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
const DDSD_LINEARSIZE: u32 = 0x8_0000;
const REQUIRED: u32 = 0x1007; // CAPS | HEIGHT | WIDTH | PIXELFORMAT
const DDSCAPS_COMPLEX: u32 = 0x8;
const DDSCAPS_TEXTURE: u32 = 0x1000;
const DDSCAPS_MIPMAP: u32 = 0x40_0000;
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;

const BLOCKS: [DdsCompression; 3] = [DdsCompression::Bc1, DdsCompression::Bc2, DdsCompression::Bc3];

fn opts(compression: DdsCompression, mipmaps: bool) -> DdsOptions {
    DdsOptions { compression, mipmaps }
}

fn le32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

/// A ramp along one line through RGB (red up, green up half as fast, blue down) across the
/// texture, and alpha falling from opaque down it: every block's colours lie on a line, which
/// is what a colour block stores.
fn ramp(w: u32, h: u32, layout: ChannelLayout) -> Image {
    let ch = layout.channels();
    let at = |v: u32, n: u32| (v * 255 / (n - 1).max(1)) as u8;
    let mut data = Vec::new();
    for y in 0..h {
        for x in 0..w {
            let r = at(x, w);
            let rgba = [r, 64 + r / 2, 255 - r, 255 - at(y, h)];
            match ch {
                1 => data.push(rgba[0]),
                2 => data.extend([rgba[0], rgba[3]]),
                _ => data.extend(&rgba[..ch]),
            }
        }
    }
    Image::from_u8(w, h, layout, data).unwrap()
}

/// The same RGBA image, fully opaque.
fn opaque(img: &Image) -> Image {
    let data = img.data().chunks(4).flat_map(|p| [p[0], p[1], p[2], 255]).collect();
    Image::from_u8(img.width(), img.height(), ChannelLayout::Rgba, data).unwrap()
}

/// Largest difference in channel `c` (of four) between RGBA images.
fn max_diff_channel(a: &Image, b: &Image, c: usize) -> u8 {
    a.data().chunks(4).zip(b.data().chunks(4)).map(|(x, y)| x[c].abs_diff(y[c])).max().unwrap_or(0)
}

/// Largest colour difference between RGBA images where `b` is visible (BC1 turns texels under
/// half opacity into transparent black).
fn max_colour_diff(a: &Image, b: &Image) -> u8 {
    a.data().chunks(4).zip(b.data().chunks(4)).filter(|(_, y)| y[3] != 0).flat_map(|(x, y)| (0..3).map(|c| x[c].abs_diff(y[c]))).max().unwrap_or(0)
}

/// A 2D gradient: red across, green down and blue along the diagonal, opaque.
fn gradient(w: u32, h: u32) -> Image {
    let data =
        (0..h).flat_map(|y| (0..w).flat_map(move |x| [(x * 4).min(255) as u8, (y * 4).min(255) as u8, ((x + y) * 2).min(255) as u8, 255])).collect();
    Image::from_u8(w, h, ChannelLayout::Rgba, data).unwrap()
}

/// PSNR of the colour (not the alpha) of two RGBA images.
fn psnr(a: &Image, b: &Image) -> f64 {
    let rgb = |i: &Image| i.data().chunks(4).flat_map(|p| [p[0], p[1], p[2]]).map(f64::from).collect::<Vec<_>>();
    let (a, b) = (rgb(a), rgb(b));
    let mse = a.iter().zip(&b).map(|(x, y)| (x - y).powi(2)).sum::<f64>() / a.len() as f64;
    if mse == 0.0 { f64::INFINITY } else { 10.0 * (255.0f64.powi(2) / mse).log10() }
}

/// Bytes of one level as a reader computes them from the header's pixel format.
fn level_bytes(file: &[u8], w: usize, h: usize) -> usize {
    if le32(file, 80) & DDPF_FOURCC != 0 {
        let block = if &file[84..88] == b"DXT1" { 8 } else { 16 };
        w.div_ceil(4) * h.div_ceil(4) * block
    } else {
        w * h * le32(file, 88) as usize / 8
    }
}

/// Mip level `i` of a written texture, cut out as a single-level texture and decoded.
fn level(file: &[u8], i: u32) -> Image {
    let (w, h) = (le32(file, 16) as usize, le32(file, 12) as usize);
    let size = |i: u32| ((w >> i).max(1), (h >> i).max(1));
    let start = 128 + (0..i).map(|j| level_bytes(file, size(j).0, size(j).1)).sum::<usize>();
    let (lw, lh) = size(i);
    let mut b = file[..128].to_vec();
    b[12..16].copy_from_slice(&(lh as u32).to_le_bytes());
    b[16..20].copy_from_slice(&(lw as u32).to_le_bytes());
    b[28..32].copy_from_slice(&1u32.to_le_bytes());
    // The pitch field is a row stride for uncompressed textures and the level size for BC.
    let pitch = if le32(file, 80) & DDPF_FOURCC != 0 { level_bytes(file, lw, lh) } else { lw * le32(file, 88) as usize / 8 };
    b[20..24].copy_from_slice(&(pitch as u32).to_le_bytes());
    b.extend(&file[start..start + level_bytes(file, lw, lh)]);
    decode(&b).unwrap()
}

#[test]
fn uncompressed_textures_round_trip_every_layout_exactly() {
    for (layout, pf_flags, bits, masks) in [
        (ChannelLayout::Gray, DDPF_LUMINANCE, 8, [0xff, 0, 0, 0]),
        (ChannelLayout::GrayA, DDPF_LUMINANCE | DDPF_ALPHAPIXELS, 16, [0xff, 0, 0, 0xff00]),
        (ChannelLayout::Rgb, DDPF_RGB, 32, [0xff_0000, 0xff00, 0xff, 0]),
        (ChannelLayout::Rgba, DDPF_RGB | DDPF_ALPHAPIXELS, 32, [0xff_0000, 0xff00, 0xff, 0xff00_0000]),
    ] {
        for (w, h) in [(1, 1), (5, 3), (17, 9)] {
            let img = ramp(w, h, layout);
            let b = encode(&img, DdsOptions::default()).unwrap();
            let back = decode(&b).unwrap();
            assert_eq!(back.layout(), layout, "{layout:?} {w}x{h}");
            assert_eq!(back.data(), img.data(), "{layout:?} {w}x{h}");

            assert_eq!((&b[..4], le32(&b, 4), le32(&b, 76)), (&b"DDS "[..], 124, 32));
            assert_eq!(le32(&b, 8), REQUIRED | DDSD_PITCH, "pitch, not linear size, and no mip count");
            assert_eq!((le32(&b, 12), le32(&b, 16)), (h, w));
            assert_eq!(le32(&b, 20), w * bits / 8, "pitch of {layout:?} {w}x{h}");
            assert_eq!((le32(&b, 28), le32(&b, 108)), (1, DDSCAPS_TEXTURE));
            assert_eq!((le32(&b, 80), le32(&b, 88)), (pf_flags, bits));
            assert_eq!([le32(&b, 92), le32(&b, 96), le32(&b, 100), le32(&b, 104)], masks);
            assert_eq!(b.len(), 128 + (w * h * bits / 8) as usize);
        }
    }
}

#[test]
fn block_compressed_textures_round_trip_through_the_reader() {
    for (compression, fourcc, block) in [(DdsCompression::Bc1, b"DXT1", 8), (DdsCompression::Bc2, b"DXT3", 16), (DdsCompression::Bc3, b"DXT5", 16)] {
        // Not multiples of four, and the 1×1 and 2×2 textures smaller than one block.
        for (w, h) in [(1, 1), (2, 2), (4, 4), (5, 3), (7, 13), (33, 17)] {
            let img = ramp(w, h, ChannelLayout::Rgba);
            let b = encode(&img, opts(compression, false)).unwrap();
            let linear = w.div_ceil(4) * h.div_ceil(4) * block;
            assert_eq!(&b[84..88], fourcc);
            assert_eq!((le32(&b, 80), le32(&b, 88)), (DDPF_FOURCC, 0));
            assert_eq!(le32(&b, 8), REQUIRED | DDSD_LINEARSIZE, "linear size, not pitch");
            assert_eq!(le32(&b, 20), linear, "{compression:?} {w}x{h}: the top level's size");
            assert_eq!(b.len(), 128 + linear as usize);

            let back = decode(&b).unwrap();
            assert_eq!((back.dimensions(), back.layout()), ((w, h), ChannelLayout::Rgba));
            // Steep ramps across tiny textures put three colours where a block interpolates at
            // thirds, so this bound is loose; it still catches swapped channels or misplaced
            // blocks. `block_compression_error_on_gradients_is_bounded` is the tight one. A BC1
            // block with transparent texels has only three colours, so BC1's colours are
            // checked on the opaque ramp.
            let (src, back_colour) = if compression == DdsCompression::Bc1 {
                let o = opaque(&img);
                let back = decode(&encode(&o, opts(compression, false)).unwrap()).unwrap();
                (o, back)
            } else {
                (img.clone(), back.clone())
            };
            let colour = max_colour_diff(&src, &back_colour);
            assert!(colour <= 24, "{compression:?} {w}x{h}: colour off by {colour}");
            let alpha = max_diff_channel(&img, &back, 3);
            match compression {
                DdsCompression::Bc1 => {
                    assert!(back.data().chunks(4).zip(img.data().chunks(4)).all(|(b, i)| b[3] == if i[3] < 128 { 0 } else { 255 }))
                }
                _ => assert!(alpha <= 8, "{compression:?} {w}x{h}: alpha off by {alpha}"),
            }
        }
    }
}

#[test]
fn block_compressions_take_rgba_and_every_store_takes_8_bits() {
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb] {
        for compression in BLOCKS {
            assert!(matches!(encode(&ramp(8, 8, layout), opts(compression, false)), Err(CodecError::Encode { .. })), "{layout:?} {compression:?}");
        }
    }
    let wide = Image::from_u16(1, 1, ChannelLayout::Rgba, &[0; 4]).unwrap();
    assert!(matches!(encode(&wide, DdsOptions::default()), Err(CodecError::Encode { .. })));
}

#[test]
fn block_compression_error_on_gradients_is_bounded() {
    let (line, plane) = (ramp(64, 64, ChannelLayout::Rgba), gradient(64, 64));
    for compression in BLOCKS {
        let roundtrip = |img: &Image| decode(&encode(img, opts(compression, false)).unwrap()).unwrap();
        let (line_back, plane_back) = (roundtrip(&line), roundtrip(&plane));
        let (line_err, plane_err) = (max_colour_diff(&line, &line_back), max_colour_diff(&plane, &plane_back));
        let p = psnr(&plane, &plane_back);
        let alpha = max_diff_channel(&line, &line_back, 3);
        // Colours on a line only lose the RGB565 rounding of the endpoints (up to 4 levels in
        // red and blue). A gradient in two directions is not on a line: the spread off a
        // block's best line alone limits this one to about 40 dB, and RGB565 costs a little
        // more (the encoder measures 38.6 dB and 9 levels at most).
        assert!(line_err <= 5, "{compression:?}: colour ramp off by {line_err}");
        assert!(plane_err <= 10, "{compression:?}: 2D gradient off by {plane_err}");
        assert!(p >= 38.0, "{compression:?}: 2D gradient PSNR {p:.1} dB");
        match compression {
            DdsCompression::Bc1 => {
                assert!(line_back.data().chunks(4).zip(line.data().chunks(4)).all(|(b, i)| b[3] == if i[3] < 128 { 0 } else { 255 }))
            }
            // Four bits: within half a step of 17.
            DdsCompression::Bc2 => assert!(alpha <= 8, "BC2 alpha off by {alpha}"),
            // Eight values across a block's few levels of alpha.
            _ => assert!(alpha <= 2, "BC3 alpha off by {alpha}"),
        }
    }
}

#[test]
fn solid_and_cutout_textures_keep_their_values() {
    // RGB565-exact colours and BC2's 4-bit alpha steps come back exactly.
    let solid = Image::from_u8(6, 5, ChannelLayout::Rgba, [255, 0, 255, 170].repeat(30)).unwrap();
    for compression in BLOCKS {
        let back = decode(&encode(&solid, opts(compression, false)).unwrap()).unwrap();
        let want = if compression == DdsCompression::Bc1 { [255, 0, 255, 255] } else { [255, 0, 255, 170] };
        assert!(back.data().chunks(4).all(|p| p == want), "{compression:?}: {:?}", &back.data()[..4]);
    }
    // A cutout: BC1's transparent texels decode as transparent black, the rest stay opaque.
    let cutout: Vec<u8> = (0..36).flat_map(|i| if (i / 6 + i % 6) % 2 == 0 { [200, 40, 40, 255] } else { [10, 10, 10, 0] }).collect();
    let cutout = Image::from_u8(6, 6, ChannelLayout::Rgba, cutout).unwrap();
    for compression in BLOCKS {
        let back = decode(&encode(&cutout, opts(compression, false)).unwrap()).unwrap();
        assert_eq!(max_diff_channel(&cutout, &back, 3), 0, "{compression:?}");
        if compression == DdsCompression::Bc1 {
            assert!(back.data().chunks(4).filter(|p| p[3] == 0).all(|p| p == [0, 0, 0, 0]));
        }
    }
}

#[test]
fn mipmaps_write_the_full_chain_box_filtered() {
    // 16×8 halves to 8×4, 4×2, 2×1 and 1×1: five levels.
    let img = ramp(16, 8, ChannelLayout::Rgba);
    let b = encode(&img, opts(DdsCompression::None, true)).unwrap();
    assert_eq!(le32(&b, 8), REQUIRED | DDSD_PITCH | DDSD_MIPMAPCOUNT);
    assert_eq!(le32(&b, 28), 5);
    assert_eq!(le32(&b, 108), DDSCAPS_TEXTURE | DDSCAPS_COMPLEX | DDSCAPS_MIPMAP);
    assert_eq!(b.len(), 128 + 4 * (16 * 8 + 8 * 4 + 4 * 2 + 2 + 1));
    assert_eq!(decode(&b).unwrap().data(), img.data(), "the reader still reads the top level");
    let mut above = img.clone();
    for i in 1..5 {
        let got = level(&b, i);
        assert_eq!(got.dimensions(), ((16 >> i).max(1), (8 >> i).max(1)));
        let want = box_filter(&above);
        assert_eq!(got.data(), want.data(), "level {i}");
        above = got;
    }
    // 1×1 has a single level.
    let one = encode(&ramp(1, 1, ChannelLayout::Rgba), opts(DdsCompression::None, true)).unwrap();
    assert_eq!((le32(&one, 28), le32(&one, 8) & DDSD_MIPMAPCOUNT), (1, 0));
}

/// The documented filter, written independently: each pixel averages the 2×2 pixels it covers,
/// and the last column and row of an odd-sized level take in the leftover pixel too.
fn box_filter(src: &Image) -> Image {
    let (w, h) = (src.width() as usize, src.height() as usize);
    let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
    let ch = src.layout().channels();
    let span = |i: usize, n: usize, size: usize| (2 * i, if i + 1 == n { size } else { 2 * i + 2 });
    let mut out = Vec::new();
    for y in 0..nh {
        for x in 0..nw {
            let ((x0, x1), (y0, y1)) = (span(x, nw, w), span(y, nh, h));
            for c in 0..ch {
                let values: Vec<u32> =
                    (y0..y1).flat_map(|sy| (x0..x1).map(move |sx| (sy, sx))).map(|(sy, sx)| u32::from(src.data()[(sy * w + sx) * ch + c])).collect();
                let n = values.len() as u32;
                out.push(((values.iter().sum::<u32>() + n / 2) / n) as u8);
            }
        }
    }
    Image::from_u8(nw as u32, nh as u32, src.layout(), out).unwrap()
}

#[test]
fn odd_sized_mip_chains_keep_every_pixel() {
    // 5×3 → 2×1 → 1×1. The 2×1 level's right pixel averages columns 2–4 of all three rows.
    let img = ramp(5, 3, ChannelLayout::Gray);
    let b = encode(&img, opts(DdsCompression::None, true)).unwrap();
    assert_eq!(le32(&b, 28), 3);
    assert_eq!(b.len(), 128 + 5 * 3 + 2 + 1);
    let second = level(&b, 1);
    assert_eq!(second.data(), box_filter(&img).data());
    let px = |x: usize, y: usize| u32::from(img.data()[y * 5 + x]);
    let right: u32 = (0..3).flat_map(|y| (2..5).map(move |x| (x, y))).map(|(x, y)| px(x, y)).sum();
    assert_eq!(u32::from(second.data()[1]), (right + 4) / 9);
}

#[test]
fn block_compressed_mip_chains_have_a_block_per_small_level() {
    for compression in BLOCKS {
        let block = if compression == DdsCompression::Bc1 { 8 } else { 16 };
        let img = ramp(12, 6, ChannelLayout::Rgba);
        let b = encode(&img, opts(compression, true)).unwrap();
        // 12×6, 6×3, 3×1, 1×1: 3×2, 2×1, 1 and 1 blocks.
        assert_eq!(le32(&b, 28), 4);
        assert_eq!(le32(&b, 8), REQUIRED | DDSD_LINEARSIZE | DDSD_MIPMAPCOUNT);
        assert_eq!(le32(&b, 20), 6 * block);
        assert_eq!(b.len(), 128 + (6 + 2 + 1 + 1) * block as usize);
        for i in 0..4 {
            let got = level(&b, i);
            assert_eq!(got.dimensions(), ((12 >> i).max(1), (6 >> i).max(1)), "{compression:?} level {i}");
        }
    }
}

#[test]
fn strips_and_tiny_textures_encode_with_every_option() {
    for (w, h) in [(1, 1), (1, 9), (9, 1), (2, 3), (3, 2)] {
        for compression in DdsCompression::ALL {
            for mipmaps in [false, true] {
                let img = ramp(w, h, ChannelLayout::Rgba);
                let b = encode(&img, opts(compression, mipmaps)).unwrap_or_else(|e| panic!("{w}x{h} {compression:?} {mipmaps}: {e}"));
                assert_eq!(decode(&b).unwrap().dimensions(), (w, h));
            }
        }
    }
}

#[test]
fn compression_names_round_trip() {
    for c in DdsCompression::ALL {
        assert_eq!(DdsCompression::from_name(c.name()), Some(c));
    }
    assert_eq!(DdsCompression::from_name("DXT5"), Some(DdsCompression::Bc3));
    assert_eq!(DdsCompression::from_name("dxt1"), Some(DdsCompression::Bc1));
    for bad in ["", "bc7", "dxt2", "uncompressed "] {
        assert_eq!(DdsCompression::from_name(bad), None, "{bad:?}");
    }
}

/// Encode timings on a synthetic ~30 MP texture (6336×4752). Ignored by default; run in release:
///
/// ```sh
/// cargo test --release -p craft-raster-codecs --test dds_write -- --ignored --nocapture
/// ```
#[test]
#[ignore]
fn dds_encode_timings_on_30_mp() {
    let (w, h) = (6336u32, 4752u32);
    let mut s = 0x1234_5678u32;
    let mut data = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h {
        for x in 0..w {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            data.extend([(x / 25 + y / 19) as u8, (x / 31) as u8 ^ (s as u8 & 3), (y / 23) as u8, (255 - x / 25) as u8]);
        }
    }
    let img = Image::from_u8(w, h, ChannelLayout::Rgba, data).unwrap();
    for compression in DdsCompression::ALL {
        for mipmaps in [false, true] {
            let t = std::time::Instant::now();
            let b = encode(&img, opts(compression, mipmaps)).unwrap();
            eprintln!("{compression:?} mipmaps={mipmaps}: {:.0} ms, {} MB", t.elapsed().as_secs_f64() * 1e3, b.len() >> 20);
        }
    }
}
