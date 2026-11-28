use craft_raster_codecs::{ChannelLayout, CodecError, Image, Limits, SampleType, pcx};

fn decode(b: &[u8]) -> Result<Image, CodecError> {
    pcx::decode(b, &Limits::default())
}

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
fn decodes_three_plane_rgb_and_keeps_dpi() {
    let mut b = header(8, 3, 2, 1, 2);
    for row in [&[255, 0][..], &[0, 255][..], &[0, 0][..]] {
        rle_row(&mut b, row);
    }
    assert!(pcx::looks_like_pcx(&b));
    let im = decode(&b).unwrap();
    assert_eq!(im.layout(), ChannelLayout::Rgb);
    assert_eq!(im.sample_type(), SampleType::U8);
    assert_eq!(im.dimensions(), (2, 1));
    assert_eq!(im.data(), &[255, 0, 0, 0, 255, 0]);
    assert_eq!(im.dpi, Some((300.0, 150.0)));
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
    assert!(decode(&b).is_err(), "header alone is truncated");
    b.extend([0xc7, 0xff]); // a seven-byte run cannot cross a six-byte scanline
    assert!(decode(&b).is_err());
    let mut b = header(8, 3, 2, 1, 2);
    b.extend([0xc1]); // missing run value
    assert!(decode(&b).is_err());
    let mut b = header(8, 3, 2, 1, 2);
    b[66..68].copy_from_slice(&0u16.to_le_bytes());
    assert!(!pcx::looks_like_pcx(&b), "invalid stride is not a PCX signature");

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
    let limits = Limits { max_alloc: 1024, ..Default::default() };
    assert!(matches!(pcx::decode(&b, &limits), Err(CodecError::LimitExceeded(_))));
    b[2] = 0;
    assert!(!pcx::looks_like_pcx(&b), "unsupported uncompressed encoding");
}

#[test]
fn decodes_rgb_runs_across_planes() {
    let mut b = header(8, 3, 2, 1, 2);
    b.extend([0xc4, 0xff, 0xc2, 0]);
    let im = decode(&b).unwrap();
    assert_eq!(im.data(), &[255, 255, 0, 255, 255, 0]);
}

#[test]
fn decodes_four_plane_runs_across_planes() {
    let mut b = header(1, 4, 2, 1, 2);
    b[16 + 15 * 3..16 + 16 * 3].copy_from_slice(&[255, 255, 0]);
    b.extend([0xc8, 0xff]);
    assert_eq!(decode(&b).unwrap().data(), &[255, 255, 0, 255, 255, 0]);
}

#[test]
fn pcx_header_declaring_gigabytes_needs_a_plausible_file_size() {
    // 24-bit (three 8-bit planes) 65535 x 16384, with one row of data.
    let mut b = vec![0u8; 128];
    b[0] = 0x0a;
    b[1] = 5;
    b[2] = 1;
    b[3] = 8;
    b[8..10].copy_from_slice(&65534u16.to_le_bytes());
    b[10..12].copy_from_slice(&16383u16.to_le_bytes());
    b[65] = 3;
    b[66..68].copy_from_slice(&65535u16.to_le_bytes());
    b[68..70].copy_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&[0xff, 0]);
    // PCX's own minimum-packet check refuses it before the output is allocated.
    match decode(&b) {
        Err(CodecError::Malformed { message, .. }) => assert!(message.contains("too short"), "{message}"),
        other => panic!("expected a too-short refusal, got {:?}", other.map(|i| (i.width(), i.height()))),
    }
}

/// Four 8-bit planes are red, green, blue and alpha.
#[test]
fn decodes_four_8_bit_planes_as_rgba() {
    let mut b = header(8, 4, 2, 1, 2);
    rle_row(&mut b, &[10, 40, 20, 50, 30, 60, 255, 0]);
    assert_eq!(pcx::dimensions(&b), Some((2, 1)));
    let im = decode(&b).unwrap();
    assert_eq!((im.layout(), im.data()), (ChannelLayout::Rgba, &[10, 20, 30, 255, 40, 50, 60, 0][..]));
    // Two 8-bit planes are still not a PCX layout.
    b[65] = 2;
    assert!(!pcx::looks_like_pcx(&b) && pcx::dimensions(&b).is_none() && decode(&b).is_err());
}

/// Version 4 (PC Paintbrush for Windows) files read like version 5.
#[test]
fn version_4_headers_are_pcx() {
    let mut b = header(8, 3, 1, 1, 2);
    rle_row(&mut b, &[1, 0, 2, 0, 3, 0]);
    b[1] = 4;
    assert_eq!(decode(&b).unwrap().data(), &[1, 2, 3]);
    b[1] = 6;
    assert!(decode(&b).is_err());
}
