use super::*;

/// A 40-byte-header BMP. `palette` holds BGRX entries; `pixel_at` overrides `bfOffBits` when given.
#[allow(clippy::too_many_arguments)]
fn make(width: i32, height: i32, bits: u16, compression: u32, colours: u32, palette: &[[u8; 4]], pixel_at: Option<u32>, body: &[u8]) -> Vec<u8> {
    let at = 54 + palette.len() * 4;
    let mut out = b"BM".to_vec();
    out.extend_from_slice(&u32::try_from(at + body.len()).unwrap_or(0).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&pixel_at.unwrap_or(u32::try_from(at).unwrap_or(0)).to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(&compression.to_le_bytes());
    out.extend_from_slice(&[0; 12]);
    out.extend_from_slice(&colours.to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    for entry in palette {
        out.extend_from_slice(entry);
    }
    out.extend_from_slice(body);
    out
}

const BW: [[u8; 4]; 2] = [[0, 0, 0, 0], [255, 255, 255, 0]];

#[test]
fn the_header_gives_size_depth_and_row_order() {
    let up = parse(&make(13, 4, 24, 0, 0, &[], None, &[0; 160])).expect("info header");
    assert_eq!((up.width, up.height, up.top_down, up.bits, up.compression), (13, 4, false, 24, Compression::Rgb));
    let down = parse(&make(13, -4, 24, 0, 0, &[], None, &[0; 160])).expect("top-down");
    assert_eq!((down.height, down.top_down), (4, true));
    assert_eq!(size(&make(816, 1144, 1, 0, 0, &BW, None, &[])), Some((816, 1144)));
    assert_eq!(size(&make(850, -1169, 8, 0, 0, &[[0; 4]; 256], None, &[])), Some((850, 1169)));
}

#[test]
fn an_os2_core_header_is_read() {
    let mut out = b"BM".to_vec();
    out.extend_from_slice(&[0; 12]);
    out[10..14].copy_from_slice(&(14u32 + 12 + 6).to_le_bytes());
    out.extend_from_slice(&12u32.to_le_bytes());
    out.extend_from_slice(&5u16.to_le_bytes());
    out.extend_from_slice(&3u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 255, 255, 255]); // three-byte entries: black, white
    out.extend_from_slice(&[0b1010_0000, 0, 0, 0, 0b0101_1000, 0, 0, 0, 0b1111_1000, 0, 0, 0]);
    let info = parse(&out).expect("core");
    assert_eq!((info.width, info.height, info.bits, info.palette_entry, info.declared_colours), (5, 3, 1, 3, 2));
    assert_eq!(info.colour(&out, 1), [255, 255, 255]);
    let (w, h, rgba) = to_rgba(&out).expect("core pixels");
    assert_eq!((w, h), (5, 3));
    // Bottom-up: the first stored row is the bottom one.
    let row = |y: usize| (0..5).map(|x| rgba[(y * 5 + x) * 4]).collect::<Vec<_>>();
    assert_eq!(row(2), vec![255, 0, 255, 0, 0]);
    assert_eq!(row(0), vec![255, 255, 255, 255, 255]);
}

#[test]
fn bad_headers_are_refused() {
    let good = make(13, 4, 24, 0, 0, &[], None, &[0; 160]);
    assert!(parse(&good).is_some());
    for (label, at, value, wide) in [
        ("zero width", 18, 0u32, 4),
        ("negative width", 18, (-13i32) as u32, 4),
        ("zero height", 22, 0, 4),
        ("height i32::MIN", 22, 0x8000_0000, 4),
        ("header size 20", 14, 20, 4),
        ("header size 0", 14, 0, 4),
        ("header size huge", 14, 0xFFFF_FFF0, 4),
        ("planes 0", 26, 0, 2),
        ("planes 2", 26, 2, 2),
        ("bits 0", 28, 0, 2),
        ("bits 3", 28, 3, 2),
        ("bits 64", 28, 64, 2),
        ("rle8 on 24 bits", 30, 1, 4),
        ("rle4 on 24 bits", 30, 2, 4),
        ("bit fields on 24 bits", 30, 3, 4),
    ] {
        let mut bytes = good.clone();
        bytes[at..at + wide].copy_from_slice(&value.to_le_bytes()[..wide]);
        assert!(parse(&bytes).is_none(), "{label}");
    }
    assert!(parse(b"").is_none() && parse(b"BM").is_none() && parse(&good[..20]).is_none() && parse(&good[..53]).is_none());
    assert!(parse(b"PNG not a bitmap at all, but long enough to have a header....").is_none());
    // Compressions the pixel readers do not handle still describe their size.
    assert_eq!(parse(&make(8, 8, 24, 4, 0, &[], None, &[])).map(|i| i.compression), Some(Compression::Other(4)));
}

#[test]
fn the_palette_size_comes_from_the_depth_the_count_and_the_pixel_offset() {
    let table = [[0, 0, 0, 0], [10, 20, 30, 0], [40, 50, 60, 0], [255, 255, 255, 0]];
    // biClrUsed = 0: the full table the depth allows, limited by what is stored before the pixels.
    let info = parse(&make(2, 1, 8, 0, 0, &table, None, &[0, 1, 0, 0])).expect("8-bit");
    assert_eq!((info.declared_colours, info.stored_colours()), (256, 4));
    assert_eq!(info.colour(&make(2, 1, 8, 0, 0, &table, None, &[0, 1, 0, 0]), 1), [30, 20, 10]);
    // An explicit short count, and a count the depth cannot hold.
    assert_eq!(parse(&make(2, 1, 8, 0, 3, &table, None, &[0; 4])).map(|i| i.stored_colours()), Some(3));
    assert_eq!(parse(&make(2, 1, 4, 0, 0xFFFF_FFFF, &table, None, &[0; 4])).map(|i| i.declared_colours), Some(16));
    assert_eq!(parse(&make(2, 1, 24, 0, 77, &[], None, &[0; 8])).map(|i| i.declared_colours), Some(0));
    // An index past the table is black, never a failure.
    let bytes = make(2, 1, 8, 0, 0, &table, None, &[3, 9, 0, 0]);
    let (_, _, rgba) = to_rgba(&bytes).expect("index past the table");
    assert_eq!(rgba, [255, 255, 255, 255, 0, 0, 0, 255]);
}

#[test]
fn bf_off_bits_is_honoured_zero_means_after_the_palette_and_the_headers_are_off_limits() {
    let body = [0b0100_0000, 0, 0, 0];
    // Extra bytes between the palette and the pixels are skipped.
    let mut gap = make(2, 1, 1, 0, 0, &BW, Some(62 + 5), &body);
    gap.splice(62..62, [0xAA; 5]);
    let (_, _, rgba) = to_rgba(&gap).expect("gap");
    assert_eq!(rgba, [0, 0, 0, 255, 255, 255, 255, 255]);
    // 0 means the pixels follow the palette.
    let (_, _, rgba) = to_rgba(&make(2, 1, 1, 0, 0, &BW, Some(0), &body)).expect("offset 0");
    assert_eq!(rgba, [0, 0, 0, 255, 255, 255, 255, 255]);
    // 0 with an info header and bit fields: the three masks come first.
    let info = parse(&make(1, 1, 16, 3, 0, &[], Some(0), &[])).expect("bit fields");
    assert_eq!(info.data_at(), Some(54 + 12));
    // An offset inside the headers, or past the end, never yields pixels.
    for at in [1u32, 13, 14, 40, 53, 400, 0xFFFF_FFFF] {
        assert!(to_rgba(&make(2, 1, 24, 0, 0, &[], Some(at), &[0; 8])).is_none(), "offset {at}");
    }
    assert_eq!(size(&make(2, 1, 24, 0, 0, &[], Some(400), &[0; 8])), Some((2, 1)), "the size does not depend on the offset");
}

#[test]
fn rows_are_returned_top_first_for_both_row_orders() {
    // Two rows of 8-bit index data, 3 px wide (stride 4): stored order 0,1,2 then 2,1,0.
    let body = [0, 1, 2, 0, 2, 1, 0, 0];
    let pal = [[0, 0, 0, 0], [0, 0, 255, 0], [0, 255, 0, 0]];
    let up = to_rgba(&make(3, 2, 8, 0, 3, &pal, None, &body)).expect("bottom-up").2;
    let down = to_rgba(&make(3, -2, 8, 0, 3, &pal, None, &body)).expect("top-down").2;
    let red = |rgba: &[u8], i: usize| rgba[i * 4];
    // Bottom-up: the visual top row is the LAST stored row (2,1,0).
    assert_eq!((red(&up, 0), red(&up, 1), red(&up, 2)), (0, 255, 0));
    assert_eq!((red(&up, 3), red(&up, 4), red(&up, 5)), (0, 255, 0));
    assert_eq!(&up[0..4], [0, 255, 0, 255]);
    assert_eq!(&up[4..8], [255, 0, 0, 255]);
    assert_eq!(&up[8..12], [0, 0, 0, 255]);
    assert_eq!(&up[12..16], [0, 0, 0, 255]);
    assert_eq!(&up[16..20], [255, 0, 0, 255]);
    assert_eq!(&down[4..8], [255, 0, 0, 255]);
    assert_eq!(&down[8..12], [0, 255, 0, 255]);
    assert_ne!(up, down);
}

#[test]
fn giant_claims_fail_fast_without_allocating() {
    // 100000 x 100000 in a file of a few hundred bytes, every depth and both row orders.
    for bits in [1u16, 4, 8, 24, 32] {
        for height in [100_000, -100_000] {
            let bytes = make(100_000, height, bits, 0, 0, &[[0; 4]; 2], None, &[0; 64]);
            assert!(to_rgba(&bytes).is_none(), "{bits} bits {height}");
        }
    }
    for compression in [1u32, 2] {
        let bits = if compression == 1 { 8 } else { 4 };
        assert!(to_rgba(&make(100_000, 100_000, bits, compression, 0, &[[0; 4]; 16], None, &[0, 1])).is_none());
        assert!(to_rgba(&make(i32::MAX, i32::MAX, bits, compression, 0, &[[0; 4]; 16], None, &[0, 1])).is_none());
    }
    assert!(to_rgba(&make(i32::MAX, 1, 24, 0, 0, &[], None, &[0; 64])).is_none());
    assert!(to_rgba(&make(1, i32::MAX, 24, 0, 0, &[], None, &[0; 64])).is_none());
}

#[test]
fn a_short_rle_body_cannot_claim_a_huge_picture() {
    // 8192 x 8192 is under the preview cap and a 4 KB body passes a length-only check, but a stream can skip whole
    // rows, so the declaration itself is capped: refused up front, before the 64 MiB index buffer.
    let started = std::time::Instant::now();
    let mut body = vec![0u8, 1];
    body.resize(4096, 0);
    let palette = [[0u8; 4]; 256];
    for height in [8192, -8192] {
        assert!(to_rgba(&make(8192, height, 8, 1, 0, &palette, None, &body)).is_none(), "rle8 {height}");
        assert!(to_rgba(&make(8192, height, 4, 2, 0, &[[0; 4]; 16], None, &body)).is_none(), "rle4 {height}");
    }
    assert!(started.elapsed() < std::time::Duration::from_secs(5), "refusal must be quick");
    // A modest RLE picture still decodes.
    assert!(to_rgba(&make(4, 2, 8, 1, 0, &palette, None, &[4, 1, 0, 0, 4, 2, 0, 1])).is_some());
}

#[test]
fn truncated_files_give_none_at_every_cut() {
    let full = make(5, 3, 24, 0, 0, &[], None, &[7; 48]);
    assert!(to_rgba(&full).is_some());
    for cut in 0..full.len() {
        assert!(to_rgba(&full[..cut]).is_none(), "cut at {cut}");
    }
    let rle = make(4, 2, 8, 1, 0, &[[0; 4]; 256], None, &[4, 1, 0, 0, 4, 2, 0, 1]);
    assert!(to_rgba(&rle).is_some());
    // Cut inside the headers or the palette, or in the middle of a pair: nothing. Between pairs the rest stays black.
    for cut in (0..rle.len()).filter(|cut| *cut < 54 + 1024 || (cut - 54 - 1024) % 2 == 1) {
        assert!(to_rgba(&rle[..cut]).is_none(), "rle cut at {cut}");
    }
}

#[test]
fn rle_forms_decode_and_unreadable_ones_are_refused() {
    let pal = [[0, 0, 0, 0], [0, 0, 255, 0], [0, 255, 0, 0], [255, 0, 0, 0]];
    // RLE8 4x2 bottom-up: bottom row a run of four index-1 pixels; top row an absolute run of three, then one.
    let body = [4, 1, 0, 0, 0, 3, 2, 3, 1, 0, 1, 2, 0, 0, 0, 1];
    let (_, _, rgba) = to_rgba(&make(4, 2, 8, 1, 4, &pal, None, &body)).expect("rle8");
    assert_eq!(&rgba[..16], [0, 255, 0, 255, 0, 0, 255, 255, 255, 0, 0, 255, 0, 255, 0, 255]);
    assert_eq!(&rgba[16..20], [255, 0, 0, 255]);
    // A run past the edge is clipped; a stream that ends mid-pair is refused.
    assert!(to_rgba(&make(2, 1, 8, 1, 4, &pal, None, &[5, 1, 0, 1])).is_some());
    assert!(to_rgba(&make(4, 1, 8, 1, 4, &pal, None, &[4])).is_none());
    assert!(to_rgba(&make(4, 1, 8, 1, 4, &pal, None, &[0, 4, 1, 2])).is_none());
    // Bit fields, embedded JPEG/PNG and 16 bits are another decoder's job.
    for (bits, compression) in [(32u16, 3u32), (16, 3), (24, 4), (24, 5), (16, 0)] {
        assert!(to_rgba(&make(2, 2, bits, compression, 0, &[], None, &[0; 16])).is_none(), "{bits}/{compression}");
    }
}
