//! Reading DDS textures.

use craft_raster_codecs::{ChannelLayout, CodecError, Image, Limits, SampleType, dds};

fn decode(b: &[u8]) -> Result<Image, CodecError> {
    dds::decode(b, &Limits::default())
}

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;
const DDSD_PITCH: u32 = 0x8;

/// A DDS_HEADER for a single-level 2D texture, after the `DDS ` signature.
fn header(width: u32, height: u32, pf_flags: u32, fourcc: &[u8; 4], bits: u32, masks: [u32; 4]) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[0..4].copy_from_slice(b"DDS ");
    let mut put = |at: usize, v: u32| b[at..at + 4].copy_from_slice(&v.to_le_bytes());
    put(4, 124);
    put(8, 0x1007); // CAPS | HEIGHT | WIDTH | PIXELFORMAT
    put(12, height);
    put(16, width);
    put(76, 32);
    put(80, pf_flags);
    put(88, bits);
    for (i, m) in masks.into_iter().enumerate() {
        put(92 + 4 * i, m);
    }
    put(108, 0x1000); // DDSCAPS_TEXTURE
    b[84..88].copy_from_slice(fourcc);
    b
}

fn compressed(width: u32, height: u32, fourcc: &[u8; 4]) -> Vec<u8> {
    header(width, height, DDPF_FOURCC, fourcc, 0, [0; 4])
}

fn set_pitch(b: &mut [u8], pitch: u32) {
    b[8..12].copy_from_slice(&(0x1007 | DDSD_PITCH).to_le_bytes());
    b[20..24].copy_from_slice(&pitch.to_le_bytes());
}

/// The DX10 extension header after a `DX10` FourCC: format, dimension, misc flags, array size.
fn dx10(width: u32, height: u32, dxgi: u32, dimension: u32, misc: u32, array: u32) -> Vec<u8> {
    let mut b = compressed(width, height, b"DX10");
    for v in [dxgi, dimension, misc, array, 0] {
        b.extend(v.to_le_bytes());
    }
    b
}

/// BC1 colour endpoints as RGB565, then one 2-bit index per pixel, row by row.
fn bc1_block(c0: u16, c1: u16, rows: [u8; 4]) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(c0.to_le_bytes());
    b.extend(c1.to_le_bytes());
    b.extend(rows);
    b
}

const RED: u16 = 0xf800;
const GREEN: u16 = 0x07e0;
const BLUE: u16 = 0x001f;
const BLACK: u16 = 0x0000;
const WHITE: u16 = 0xffff;

/// Indices 0, 1 on the top row and 2, 3 below them, for the top-left 2x2 pixels.
const INDICES_0123: [u8; 4] = [0x04, 0x0e, 0, 0];

#[test]
fn dds_is_readable_and_writable() {
    let img = Image::from_u8(1, 1, ChannelLayout::Rgba, vec![1, 2, 3, 0]).unwrap();
    let b = dds::encode(&img, Default::default()).unwrap();
    assert_eq!(decode(&b).unwrap().data(), &[1, 2, 3, 0]);
}

#[test]
fn decodes_bc1_four_colour_blocks() {
    // c0 > c1: four opaque colours, the two in between at thirds.
    let mut b = compressed(2, 2, b"DXT1");
    b.extend(bc1_block(RED, BLUE, INDICES_0123));
    assert!(dds::looks_like_dds(&b));
    let im = decode(&b).unwrap();
    assert_eq!((im.dimensions(), im.layout(), im.sample_type()), ((2, 2), ChannelLayout::Rgba, SampleType::U8));
    assert_eq!(im.data(), &[255, 0, 0, 255, 0, 0, 255, 255, 170, 0, 85, 255, 85, 0, 170, 255]);
}

#[test]
fn decodes_bc1_one_bit_alpha_blocks() {
    // c0 <= c1: three colours (the third halfway) and index 3 is transparent black.
    let mut b = compressed(2, 2, b"DXT1");
    b.extend(bc1_block(BLUE, RED, INDICES_0123));
    assert_eq!(decode(&b).unwrap().data(), &[0, 0, 255, 255, 255, 0, 0, 255, 128, 0, 128, 255, 0, 0, 0, 0]);
}

#[test]
fn decodes_bc2_explicit_alpha_with_four_colour_blocks() {
    let mut b = compressed(2, 2, b"DXT3");
    // Four bits of alpha per pixel: 15, 0 on the top row, 8, 1 below.
    b.extend([0x0f, 0x00, 0x18, 0x00, 0, 0, 0, 0]);
    // c0 < c1 would mean transparency in BC1; BC2 colour blocks always have four colours.
    b.extend(bc1_block(BLACK, WHITE, [0x04, 0x0b, 0, 0]));
    let im = decode(&b).unwrap();
    assert_eq!(im.layout(), ChannelLayout::Rgba);
    assert_eq!(im.data(), &[0, 0, 0, 255, 255, 255, 255, 0, 170, 170, 170, 136, 85, 85, 85, 17]);
}

#[test]
fn decodes_bc3_interpolated_alpha() {
    // a0 > a1: eight alpha values, six interpolated at sevenths. Indices 0, 1 / 2, 7.
    let mut b = compressed(2, 2, b"DXT5");
    b.extend([255, 0, 0x08, 0xa0, 0x03, 0, 0, 0]);
    b.extend(bc1_block(BLACK, WHITE, [0x04, 0x0b, 0, 0]));
    assert_eq!(decode(&b).unwrap().data(), &[0, 0, 0, 255, 255, 255, 255, 0, 170, 170, 170, 219, 85, 85, 85, 36]);

    // a0 <= a1: six values, four interpolated at fifths, then 0 and 255. Indices 2, 5 / 6, 7.
    let mut b = compressed(2, 2, b"DXT5");
    let bits: u64 = 2 | 5 << 3 | 6 << 12 | 7 << 15;
    b.extend([0, 255]);
    b.extend(&bits.to_le_bytes()[..6]);
    b.extend(bc1_block(GREEN, GREEN, [0; 4]));
    assert_eq!(decode(&b).unwrap().data(), &[0, 255, 0, 51, 0, 255, 0, 204, 0, 255, 0, 0, 0, 255, 0, 255]);
}

#[test]
fn bc_textures_need_not_be_a_multiple_of_four() {
    // 5x3 is two blocks wide and one high; the right block's first column is x = 4.
    let mut b = compressed(5, 3, b"DXT1");
    b.extend(bc1_block(RED, RED, [0; 4]));
    b.extend(bc1_block(BLUE, BLUE, [0; 4]));
    let im = decode(&b).unwrap();
    assert_eq!(im.dimensions(), (5, 3));
    let px = |x: usize, y: usize| &im.data()[(y * 5 + x) * 4..(y * 5 + x + 1) * 4];
    assert_eq!((px(3, 2), px(4, 0), px(4, 2)), (&[255, 0, 0, 255][..], &[0, 0, 255, 255][..], &[0, 0, 255, 255][..]));
}

#[test]
fn decodes_uncompressed_rgba_and_bgra_masks() {
    let rgba = [0xff, 0xff00, 0xff_0000, 0xff00_0000];
    let mut b = header(2, 1, DDPF_RGB | DDPF_ALPHAPIXELS, &[0; 4], 32, rgba);
    b.extend([1, 2, 3, 4, 250, 251, 252, 253]);
    let im = decode(&b).unwrap();
    assert_eq!(im.layout(), ChannelLayout::Rgba);
    assert_eq!(im.data(), &[1, 2, 3, 4, 250, 251, 252, 253]);

    let bgra = [0xff_0000, 0xff00, 0xff, 0xff00_0000];
    let mut b = header(2, 1, DDPF_RGB | DDPF_ALPHAPIXELS, &[0; 4], 32, bgra);
    b.extend([3, 2, 1, 4, 252, 251, 250, 253]);
    assert_eq!(decode(&b).unwrap().data(), &[1, 2, 3, 4, 250, 251, 252, 253]);

    // An alpha mask without DDPF_ALPHAPIXELS is not alpha: X8R8G8B8 opens as RGB.
    let mut b = header(1, 1, DDPF_RGB, &[0; 4], 32, bgra);
    b.extend([3, 2, 1, 0]);
    let im = decode(&b).unwrap();
    assert_eq!((im.layout(), im.data()), (ChannelLayout::Rgb, &[1u8, 2, 3][..]));
}

#[test]
fn decodes_packed_masks_and_luminance() {
    // R5G6B5: each field scaled to 0..255; 3 of 31 is 25.
    let mut b = header(4, 1, DDPF_RGB, &[0; 4], 16, [0xf800, 0x07e0, 0x001f, 0]);
    for v in [RED, GREEN, BLUE, 3 << 11] {
        b.extend(v.to_le_bytes());
    }
    let im = decode(&b).unwrap();
    assert_eq!(im.layout(), ChannelLayout::Rgb);
    assert_eq!(im.data(), &[255, 0, 0, 0, 255, 0, 0, 0, 255, 25, 0, 0]);

    // 24-bit rows are packed three bytes a pixel, without padding.
    let mut b = header(2, 2, DDPF_RGB, &[0; 4], 24, [0xff_0000, 0xff00, 0xff, 0]);
    b.extend([3, 2, 1, 6, 5, 4, 9, 8, 7, 12, 11, 10]);
    assert_eq!(decode(&b).unwrap().data(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]);

    let mut b = header(2, 1, DDPF_LUMINANCE | DDPF_ALPHAPIXELS, &[0; 4], 16, [0xff, 0, 0, 0xff00]);
    b.extend([10, 20, 30, 40]);
    let im = decode(&b).unwrap();
    assert_eq!((im.layout(), im.data()), (ChannelLayout::GrayA, &[10u8, 20, 30, 40][..]));
}

#[test]
fn decodes_padded_rgb_rows() {
    // DWORD-aligned 24-bit rows: the fourth byte is padding, not the next pixel.
    let mut b = header(1, 2, DDPF_RGB, &[0; 4], 24, [0xff, 0xff00, 0xff_0000, 0]);
    set_pitch(&mut b, 4);
    b.extend([255, 0, 0, 99, 0, 255, 0, 88]);
    let im = decode(&b).unwrap();
    assert_eq!((im.dimensions(), im.layout()), ((1, 2), ChannelLayout::Rgb));
    assert_eq!(im.data(), &[255, 0, 0, 0, 255, 0]);

    // Padding can contain whole pixels; only the declared width is decoded.
    let mut b = header(2, 2, DDPF_RGB, &[0; 4], 16, [0xf800, 0x07e0, 0x001f, 0]);
    set_pitch(&mut b, 8);
    for v in [RED, GREEN, WHITE, WHITE, BLUE, BLACK, WHITE, WHITE] {
        b.extend(v.to_le_bytes());
    }
    assert_eq!(decode(&b).unwrap().data(), &[255, 0, 0, 0, 255, 0, 0, 0, 255, 0, 0, 0]);
}

#[test]
fn decodes_padded_luminance_rows() {
    let mut b = header(2, 2, DDPF_LUMINANCE, &[0; 4], 8, [0xff, 0, 0, 0]);
    set_pitch(&mut b, 4);
    b.extend([10, 20, 99, 98, 30, 40, 97, 96]);
    let im = decode(&b).unwrap();
    assert_eq!((im.layout(), im.data()), (ChannelLayout::Gray, &[10u8, 20, 30, 40][..]));

    let mut b = header(1, 2, DDPF_LUMINANCE | DDPF_ALPHAPIXELS, &[0; 4], 16, [0xff, 0, 0, 0xff00]);
    set_pitch(&mut b, 4);
    b.extend([10, 20, 99, 98, 30, 40, 97, 96]);
    let im = decode(&b).unwrap();
    assert_eq!((im.layout(), im.data()), (ChannelLayout::GrayA, &[10u8, 20, 30, 40][..]));
}

#[test]
fn decodes_padded_dx10_rows() {
    let mut b = dx10(1, 2, 28, 3, 0, 1); // R8G8B8A8_UNORM
    set_pitch(&mut b, 8);
    b.extend([1, 2, 3, 4, 99, 98, 97, 96, 5, 6, 7, 8, 95, 94, 93, 92]);
    assert_eq!(decode(&b).unwrap().data(), &[1, 2, 3, 4, 5, 6, 7, 8]);
}

#[test]
fn ignores_pitch_without_the_pitch_flag() {
    let mut b = header(1, 2, DDPF_RGB, &[0; 4], 24, [0xff, 0xff00, 0xff_0000, 0]);
    b[20..24].copy_from_slice(&u32::MAX.to_le_bytes());
    b.extend([255, 0, 0, 0, 255, 0]);
    assert_eq!(decode(&b).unwrap().data(), &[255, 0, 0, 0, 255, 0]);

    set_pitch(&mut b, 3); // An explicitly tightly packed row is also valid.
    assert_eq!(decode(&b).unwrap().data(), &[255, 0, 0, 0, 255, 0]);
}

#[test]
fn compressed_linear_size_is_not_a_row_pitch() {
    let mut b = compressed(4, 8, b"DXT1");
    b[8..12].copy_from_slice(&(0x1007u32 | 0x8_0000).to_le_bytes()); // DDSD_LINEARSIZE
    b[20..24].copy_from_slice(&16u32.to_le_bytes()); // Two 8-byte block rows.
    b.extend(bc1_block(RED, RED, [0; 4]));
    b.extend(bc1_block(BLUE, BLUE, [0; 4]));
    let im = decode(&b).unwrap();
    assert!(im.data()[..64].as_chunks::<4>().0.iter().all(|p| p == &[255, 0, 0, 255]));
    assert!(im.data()[64..].as_chunks::<4>().0.iter().all(|p| p == &[0, 0, 255, 255]));
}

#[test]
fn rejects_pitch_smaller_than_a_pixel_row() {
    for pitch in [0, 1, 2] {
        let mut b = header(1, 2, DDPF_RGB, &[0; 4], 24, [0xff, 0xff00, 0xff_0000, 0]);
        set_pitch(&mut b, pitch);
        b.extend([255, 0, 0, 0, 255, 0]);
        assert!(matches!(decode(&b), Err(CodecError::Malformed { .. })), "pitch {pitch}");
    }
}

#[test]
fn rejects_truncated_padded_rows() {
    let mut b = header(1, 2, DDPF_RGB, &[0; 4], 24, [0xff, 0xff00, 0xff_0000, 0]);
    set_pitch(&mut b, 4);
    b.extend([255, 0, 0, 99, 0, 255, 0, 88]);
    assert!(decode(&b).is_ok());
    for n in 128..b.len() {
        assert!(matches!(decode(&b[..n]), Err(CodecError::Malformed { .. })), "decoded {n} of {} bytes", b.len());
    }
}

#[test]
fn rejects_huge_pitch_without_allocating() {
    let mut b = header(1, 2, DDPF_LUMINANCE, &[0; 4], 8, [0xff, 0, 0, 0]);
    set_pitch(&mut b, u32::MAX);
    b.extend([10, 20]);
    // On 32-bit targets pitch * height overflows; on 64-bit the payload is too short.
    assert!(matches!(decode(&b), Err(CodecError::Malformed { .. })));
}

#[test]
fn decodes_dx10_headers() {
    let mut b = dx10(1, 1, 28, 3, 0, 1); // R8G8B8A8_UNORM
    b.extend([1, 2, 3, 4]);
    assert_eq!(decode(&b).unwrap().data(), &[1, 2, 3, 4]);

    let mut b = dx10(1, 1, 91, 3, 0, 1); // B8G8R8A8_UNORM_SRGB
    b.extend([3, 2, 1, 4]);
    assert_eq!(decode(&b).unwrap().data(), &[1, 2, 3, 4]);

    let mut b = dx10(2, 2, 71, 3, 0, 1); // BC1_UNORM
    b.extend(bc1_block(BLUE, RED, INDICES_0123));
    assert_eq!(decode(&b).unwrap().data(), &[0, 0, 255, 255, 255, 0, 0, 255, 128, 0, 128, 255, 0, 0, 0, 0]);
}

#[test]
fn only_the_top_mip_level_is_read() {
    let mut b = compressed(4, 4, b"DXT1");
    b[28..32].copy_from_slice(&3u32.to_le_bytes());
    b.extend(bc1_block(RED, RED, [0; 4]));
    b.extend(bc1_block(BLUE, BLUE, [0; 4]));
    b.extend(bc1_block(GREEN, GREEN, [0; 4]));
    let im = decode(&b).unwrap();
    assert_eq!(im.dimensions(), (4, 4));
    assert!(im.data().chunks(4).all(|p| p == [255, 0, 0, 255]));
}

#[test]
fn refuses_cubemaps_arrays_and_volumes() {
    let unsupported = |b: &[u8], what: &str| match decode(b) {
        Err(CodecError::Unsupported { reason, .. }) => assert!(reason.contains(what), "{reason}"),
        other => panic!("{what}: {other:?}"),
    };
    let mut cube = compressed(4, 4, b"DXT1");
    cube[112..116].copy_from_slice(&0xfe00u32.to_le_bytes()); // DDSCAPS2_CUBEMAP and all faces
    cube.extend([0; 8 * 6]);
    unsupported(&cube, "cubemap");
    let mut volume = compressed(4, 4, b"DXT1");
    volume[8..12].copy_from_slice(&(0x1007u32 | 0x80_0000).to_le_bytes()); // DDSD_DEPTH
    volume[24..28].copy_from_slice(&4u32.to_le_bytes());
    volume[112..116].copy_from_slice(&0x20_0000u32.to_le_bytes()); // DDSCAPS2_VOLUME
    volume.extend([0; 8 * 4]);
    unsupported(&volume, "volume");
    unsupported(&dx10(4, 4, 71, 3, 0x4, 1), "cubemap");
    unsupported(&dx10(4, 4, 71, 3, 0, 6), "array");
    unsupported(&dx10(4, 4, 71, 4, 0, 1), "2D");
    unsupported(&dx10(4, 4, 98, 3, 0, 1), "98"); // BC7_UNORM
    unsupported(&compressed(4, 4, b"ATI2"), "ATI2");
    unsupported(&compressed(4, 4, b"DXT2"), "premultiplied");
}

/// Face declarations matter even when the general cube flag is absent; padding does not.
#[test]
fn source_probe_distinguishes_extra_faces_from_single_images_and_padding() {
    for caps in [0u32, 0x400, 0xc00, 0xfc00] {
        let mut b = compressed(4, 4, b"DXT1");
        b[112..116].copy_from_slice(&caps.to_le_bytes());
        b.extend(bc1_block(RED, RED, [0; 4]));
        for _ in 1..caps.count_ones() {
            b.extend(bc1_block(BLUE, BLUE, [0; 4]));
        }
        assert!(decode(&b).unwrap().data().chunks(4).all(|p| p == [255, 0, 0, 255]));
        assert_eq!(dds::has_additional_images(&b).unwrap(), caps.count_ones() > 1);
    }
    // A FourCC-looking unused union and trailing bytes are not a DX10 declaration.
    let mut single = header(1, 1, DDPF_RGB, b"DX10", 24, [0xff, 0xff00, 0xff_0000, 0]);
    single.extend([1, 2, 3]);
    single.extend([0xff; 64]);
    assert_eq!(decode(&single).unwrap().data(), &[1, 2, 3]);
    for mips in [0u32, 1] {
        single[28..32].copy_from_slice(&mips.to_le_bytes());
        assert!(!dds::has_additional_images(&single).unwrap());
    }
}

/// The metadata probe follows the same DX10 discriminator as preview decoding.
#[test]
fn source_probe_reads_dx10_array_and_cube_declarations() {
    for (array, misc) in [(0, 0), (1, 0), (2, 0), (1, 4)] {
        let mut b = dx10(1, 1, 28, 3, misc, array);
        b.extend([1, 2, 3, 4]);
        let multiple = array > 1 || misc == 4;
        assert_eq!(dds::has_additional_images(&b).unwrap(), multiple);
        if multiple {
            assert!(matches!(decode(&b), Err(CodecError::Unsupported { .. })));
        } else {
            assert_eq!(decode(&b).unwrap().data(), &[1, 2, 3, 4]);
        }
    }
}

#[test]
fn rejects_truncated_files_at_every_length_without_panicking() {
    let mut b = dx10(5, 3, 77, 3, 0, 1); // BC3_UNORM: two 16-byte blocks
    b.extend([0x5a; 32]);
    assert!(decode(&b).is_ok());
    for n in 0..b.len() {
        assert!(decode(&b[..n]).is_err(), "decoded {n} of {} bytes", b.len());
    }
    let mut b = header(3, 2, DDPF_RGB, &[0; 4], 24, [0xff_0000, 0xff00, 0xff, 0]);
    b.extend([7; 18]);
    assert!(decode(&b).is_ok());
    assert!(decode(&b[..b.len() - 1]).is_err(), "one byte short");
}

#[test]
fn rejects_bogus_sizes_and_pixel_formats() {
    // Huge dimensions are refused before anything is allocated.
    let mut huge = compressed(u32::MAX, u32::MAX, b"DXT1");
    huge.extend([0; 8]);
    assert!(matches!(decode(&huge), Err(CodecError::LimitExceeded(_))));
    let tight_limits = Limits { max_alloc: 1024, ..Default::default() };
    let mut tight = header(64, 64, DDPF_RGB, &[0; 4], 32, [0xff, 0xff00, 0xff_0000, 0]);
    tight.extend(vec![0; 64 * 64 * 4]);
    assert!(matches!(dds::decode(&tight, &tight_limits), Err(CodecError::LimitExceeded(_))));
    assert!(decode(&compressed(0, 4, b"DXT1")).is_err(), "zero width");

    let mut bad_size = compressed(4, 4, b"DXT1");
    bad_size[4..8].copy_from_slice(&100u32.to_le_bytes());
    bad_size.extend([0; 8]);
    assert!(matches!(decode(&bad_size), Err(CodecError::Malformed { .. })), "header size must be 124");

    for (bits, masks) in [
        (12, [0xf00, 0xf0, 0xf, 0]),        // not a whole number of bytes
        (16, [0xff_0000, 0xff00, 0xff, 0]), // a mask wider than the pixel
        (32, [0xf0f, 0xf0f0, 0xf_0000, 0]), // a mask with a gap
        (32, [0, 0, 0, 0]),                 // no colour at all
        (0, [0xff, 0xff00, 0xff_0000, 0]),  // zero bits per pixel
    ] {
        let mut b = header(1, 1, DDPF_RGB, &[0; 4], bits, masks);
        b.extend([0; 4]);
        assert!(decode(&b).is_err(), "{bits} bits, masks {masks:x?}");
    }
    let mut no_format = header(1, 1, 0, &[0; 4], 32, [0xff, 0xff00, 0xff_0000, 0]);
    no_format.extend([0; 4]);
    assert!(matches!(decode(&no_format), Err(CodecError::Unsupported { .. })));
}
