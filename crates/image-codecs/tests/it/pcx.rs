use craft_image_codecs::{ChannelLayout, CodecError, Format, SampleType, caps, decode, decode_as, detect, encode, from_extension};

fn header(bits: u8, planes: u8, width: u16, height: u16, stride: u16) -> Vec<u8> {
    let mut b = vec![0u8; 128];
    b[0] = 0x0a; // ZSoft
    b[1] = 5; // PC Paintbrush 3.0+
    b[2] = 1; // RLE
    b[3] = bits;
    b[8..10].copy_from_slice(&(width - 1).to_le_bytes());
    b[10..12].copy_from_slice(&(height - 1).to_le_bytes());
    b[12..14].copy_from_slice(&300u16.to_le_bytes());
    b[14..16].copy_from_slice(&150u16.to_le_bytes());
    b[65] = planes;
    b[66..68].copy_from_slice(&stride.to_le_bytes());
    b[68..70].copy_from_slice(&1u16.to_le_bytes());
    b
}

fn rle_row(out: &mut Vec<u8>, row: &[u8]) {
    for &value in row {
        if value >= 0xc0 {
            out.extend_from_slice(&[0xc1, value]);
        } else {
            out.push(value);
        }
    }
}

#[test]
fn pcx_is_readable_but_not_writable() {
    assert_eq!(from_extension("archive.PCX"), Some(Format::Pcx));
    assert!(caps(Format::Pcx).read && !caps(Format::Pcx).write);
    let img = craft_image_codecs::Image::from_u8(1, 1, ChannelLayout::Rgb, vec![0, 0, 0]).unwrap();
    assert!(matches!(encode(&img, Format::Pcx, &Default::default()), Err(CodecError::Unsupported { .. })));
}

#[test]
fn decodes_three_plane_rgb_and_keeps_dpi() {
    let mut b = header(8, 3, 2, 1, 2);
    for row in [&[255, 0][..], &[0, 255][..], &[0, 0][..]] {
        rle_row(&mut b, row);
    }
    assert_eq!(detect(&b), Some(Format::Pcx));
    let im = decode(&b).unwrap();
    assert_eq!(im.layout(), ChannelLayout::Rgb);
    assert_eq!(im.sample_type(), SampleType::U8);
    assert_eq!(im.dimensions(), (2, 1));
    assert_eq!(im.data(), &[255, 0, 0, 0, 255, 0]);
    assert_eq!(im.meta.dpi, Some((300.0, 150.0)));
}

#[test]
fn decodes_256_colour_vga_palette() {
    let mut b = header(8, 1, 2, 1, 2);
    rle_row(&mut b, &[1, 2]);
    b.push(0x0c);
    let mut palette = vec![0; 256 * 3];
    palette[3..6].copy_from_slice(&[255, 0, 0]);
    palette[6..9].copy_from_slice(&[0, 255, 0]);
    b.extend(palette);
    assert_eq!(decode(&b).unwrap().data(), &[255, 0, 0, 0, 255, 0]);
}

#[test]
fn decodes_4bit_and_monochrome_palettes() {
    let mut four = header(4, 1, 2, 1, 2);
    four[16 + 3..16 + 6].copy_from_slice(&[255, 0, 0]);
    four[16 + 6..16 + 9].copy_from_slice(&[0, 0, 255]);
    rle_row(&mut four, &[0x12, 0]);
    assert_eq!(decode(&four).unwrap().data(), &[255, 0, 0, 0, 0, 255]);

    let mut one = header(1, 1, 2, 1, 2);
    one[16 + 3..16 + 6].copy_from_slice(&[255, 0, 0]);
    rle_row(&mut one, &[0x80, 0]);
    assert_eq!(decode(&one).unwrap().data(), &[255, 0, 0, 0, 0, 0]);
}

#[test]
fn rejects_truncated_and_overshooting_rle_without_panicking() {
    let mut b = header(8, 3, 2, 1, 2);
    assert!(decode_as(Format::Pcx, &b).is_err(), "header alone is truncated");
    b.extend([0xc7, 0xff]); // a seven-byte run cannot cross a six-byte scanline
    assert!(decode_as(Format::Pcx, &b).is_err());
    let mut b = header(8, 3, 2, 1, 2);
    b.extend([0xc1]); // missing run value
    assert!(decode_as(Format::Pcx, &b).is_err());
    let mut b = header(8, 3, 2, 1, 2);
    b[66..68].copy_from_slice(&0u16.to_le_bytes());
    assert_eq!(detect(&b), None, "invalid stride is not a PCX signature");

    // A solid image with stride of the form 63k + 1 (64, 127) encodes as k full 63-byte runs
    // plus a 1-byte literal (2k + 1 bytes per plane row), which is valid and must decode.
    for width in [64u16, 127] {
        let mut solid = header(8, 3, width, 1, width);
        for _plane in 0..3 {
            let mut rem = usize::from(width);
            while rem >= 63 {
                solid.extend_from_slice(&[0xff, 0x00]); // 63-byte run of 0x00
                rem -= 63;
            }
            if rem == 1 {
                solid.push(0x00); // 1-byte literal 0x00
            }
        }
        let im = decode(&solid).unwrap();
        assert_eq!(im.dimensions(), (u32::from(width), 1));
        assert!(im.data().iter().all(|&v| v == 0));
    }
}

#[test]
fn refuses_headers_that_exceed_allocation_budget() {
    let mut b = header(8, 3, 3000, 3000, 3000);
    // The default decoder rejects the allocation before reading any scanline.
    let opts =
        craft_image_codecs::DecodeOptions { limits: craft_image_codecs::Limits { max_alloc: 1024, ..Default::default() }, ..Default::default() };
    assert!(matches!(craft_image_codecs::decode_as_with(Format::Pcx, &b, &opts), Err(CodecError::LimitExceeded(_))));
    b[2] = 0;
    assert_eq!(detect(&b), None, "unsupported uncompressed encoding");
}

#[test]
fn decodes_rgb_runs_across_planes() {
    let mut b = header(8, 3, 2, 1, 2);
    b.extend([0xc4, 0xff, 0xc2, 0]);
    let im = decode(&b).unwrap();
    assert_eq!(im.data(), &[255, 255, 0, 255, 255, 0]);
    let png = encode(&im, Format::Png, &Default::default()).unwrap();
    assert_eq!(decode(&png).unwrap().data(), im.data());
}

#[test]
fn decodes_four_plane_runs_across_planes() {
    let mut b = header(1, 4, 2, 1, 2);
    b[16 + 15 * 3..16 + 16 * 3].copy_from_slice(&[255, 255, 0]);
    b.extend([0xc8, 0xff]);
    assert_eq!(decode(&b).unwrap().data(), &[255, 255, 0, 255, 255, 0]);
}
