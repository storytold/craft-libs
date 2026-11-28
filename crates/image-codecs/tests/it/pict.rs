//! PICT reading, on synthetic QuickDraw pictures built here from the picture layout in
//! *Inside Macintosh: Imaging With QuickDraw*, appendix A.

use craft_image_codecs::{
    ASYMMETRIC_EXCEPTIONS, ChannelLayout, CodecError, DecodeOptions, DecodeWarning, Format, Image, Limits, caps, decode, decode_as_with, detect,
    encode, from_extension, pict_palette,
};

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

fn rect(v: &mut Vec<u8>, top: i16, left: i16, bottom: i16, right: i16) {
    for x in [top, left, bottom, right] {
        v.extend_from_slice(&x.to_be_bytes());
    }
}

/// A version 2 opcode and its data, padded to a word.
fn op(code: u16, data: &[u8]) -> Vec<u8> {
    let mut v = code.to_be_bytes().to_vec();
    v.extend_from_slice(data);
    if v.len() % 2 == 1 {
        v.push(0);
    }
    v
}

/// A version 2 picture (extended version 2 with `dpi`) of `ops`, ended with opEndPic.
fn picture(w: i16, h: i16, dpi: Option<u16>, ops: &[Vec<u8>]) -> Vec<u8> {
    let mut v = Vec::new();
    be16(&mut v, 0); // picSize: only its low 16 bits, unused by readers
    rect(&mut v, 0, 0, h, w);
    v.extend_from_slice(&[0x00, 0x11, 0x02, 0xFF]); // VersionOp, version 2
    be16(&mut v, 0x0C00); // HeaderOp
    match dpi {
        Some(d) => {
            be16(&mut v, 0xFFFE);
            be16(&mut v, 0);
            v.extend_from_slice(&(u32::from(d) << 16).to_be_bytes());
            v.extend_from_slice(&(u32::from(d) << 16).to_be_bytes());
            rect(&mut v, 0, 0, h, w);
            v.extend_from_slice(&[0; 4]);
        }
        None => {
            be16(&mut v, 0xFFFF);
            be16(&mut v, 0xFFFF);
            for x in [0, 0, w, h] {
                v.extend_from_slice(&(i32::from(x) << 16).to_be_bytes());
            }
            v.extend_from_slice(&[0; 4]);
        }
    }
    for o in ops {
        v.extend_from_slice(o);
    }
    v.extend_from_slice(&op(0x00FF, &[]));
    v
}

/// A pixel map's fields (without the base address).
struct Pm {
    row_bytes: u16,
    w: i16,
    h: i16,
    pack: u16,
    size: u16,
    cmps: u16,
}

impl Pm {
    fn bytes(&self) -> Vec<u8> {
        let mut v = Vec::new();
        be16(&mut v, self.row_bytes | 0x8000);
        rect(&mut v, 0, 0, self.h, self.w);
        be16(&mut v, 0); // pmVersion
        be16(&mut v, self.pack);
        v.extend_from_slice(&0u32.to_be_bytes()); // packSize
        v.extend_from_slice(&(72u32 << 16).to_be_bytes());
        v.extend_from_slice(&(72u32 << 16).to_be_bytes());
        be16(&mut v, if self.size >= 16 { 16 } else { 0 }); // pixelType
        be16(&mut v, self.size);
        be16(&mut v, self.cmps);
        be16(
            &mut v,
            match self.size {
                32 => 8,
                16 => 5,
                s => s,
            },
        );
        v.extend_from_slice(&[0; 12]); // planeBytes, pmTable, pmReserved
        v
    }
}

fn ctab(colors: &[[u16; 3]]) -> Vec<u8> {
    let mut v = vec![0; 4]; // ctSeed
    be16(&mut v, 0); // ctFlags
    be16(&mut v, colors.len() as u16 - 1);
    for (i, c) in colors.iter().enumerate() {
        be16(&mut v, i as u16);
        for x in c {
            be16(&mut v, *x);
        }
    }
    v
}

/// srcRect (the whole image), dstRect at `at`, srcCopy, and a mask region for the Rgn opcodes.
fn tail(w: i16, h: i16, at: (i16, i16), rgn: bool) -> Vec<u8> {
    let mut v = Vec::new();
    rect(&mut v, 0, 0, h, w);
    rect(&mut v, at.1, at.0, at.1 + h, at.0 + w);
    be16(&mut v, 0);
    if rgn {
        be16(&mut v, 10);
        rect(&mut v, at.1, at.0, at.1 + h, at.0 + w);
    }
    v
}

/// PackBits in units of `unit` bytes: runs of two or more equal units repeated, the rest literal.
fn packbits(data: &[u8], unit: usize) -> Vec<u8> {
    let units: Vec<&[u8]> = data.chunks(unit).collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < units.len() {
        let mut run = 1;
        while i + run < units.len() && run < 128 && units[i + run] == units[i] {
            run += 1;
        }
        if run >= 2 {
            out.push((1 - run as i16) as i8 as u8);
            out.extend_from_slice(units[i]);
            i += run;
        } else {
            let start = i;
            while i < units.len() && i - start < 128 && !(i + 1 < units.len() && units[i + 1] == units[i]) {
                i += 1;
            }
            out.push((i - start - 1) as u8);
            for u in &units[start..i] {
                out.extend_from_slice(u);
            }
        }
    }
    out
}

fn packed_rows(rows: &[Vec<u8>], row_bytes: usize, unit: usize) -> Vec<u8> {
    let mut v = Vec::new();
    for r in rows {
        let p = packbits(r, unit);
        if row_bytes > 250 {
            be16(&mut v, p.len() as u16);
        } else {
            v.push(p.len() as u8);
        }
        v.extend_from_slice(&p);
    }
    v
}

const COLORS: [[u16; 3]; 4] = [[0xFFFF, 0, 0], [0, 0xFFFF, 0], [0, 0, 0xFFFF], [0x8000, 0x8000, 0x8000]];

fn rgb8(c: [u16; 3]) -> [u8; 3] {
    c.map(|x| ((u32::from(x) * 255 + 32767) / 65535) as u8)
}

/// An indexed pixel image (`bits` per pixel, PackBitsRect when `packed`, else BitsRect) of
/// `w` x `h` pixels, index (x + 2y) mod the table size; returns the opcode and the expected RGB.
fn indexed_op(bits: u16, w: i16, h: i16, packed: bool, at: (i16, i16)) -> (Vec<u8>, Vec<u8>) {
    let n = COLORS.len().min(1 << bits);
    let row_bytes = (w as usize * bits as usize).div_ceil(8).next_multiple_of(2);
    let mut rows = Vec::new();
    let mut want = Vec::new();
    for y in 0..h as usize {
        let mut row = vec![0u8; row_bytes];
        for x in 0..w as usize {
            let i = (x + 2 * y) % n;
            let bit = x * bits as usize;
            row[bit / 8] |= (i as u8) << (8 - bits as usize - bit % 8);
            want.extend_from_slice(&rgb8(COLORS[i]));
        }
        rows.push(row);
    }
    let mut data = Pm { row_bytes: row_bytes as u16, w, h, pack: 0, size: bits, cmps: 1 }.bytes();
    data.extend_from_slice(&ctab(&COLORS[..n]));
    data.extend_from_slice(&tail(w, h, at, false));
    if packed && row_bytes >= 8 {
        data.extend_from_slice(&packed_rows(&rows, row_bytes, 1));
    } else {
        rows.iter().for_each(|r| data.extend_from_slice(r));
    }
    (op(if packed { 0x0098 } else { 0x0090 }, &data), want)
}

fn indexed(bits: u16, w: i16, h: i16, packed: bool) -> (Vec<u8>, Vec<u8>) {
    let (o, want) = indexed_op(bits, w, h, packed, (0, 0));
    (picture(w, h, None, &[o]), want)
}

/// A 1-bit QuickDraw bitmap (BitsRect, no colour table), black where (x + y) mod 3 is 0.
fn bitmap_data(w: i16, h: i16) -> (Vec<u8>, Vec<u8>) {
    let row_bytes = (w as usize).div_ceil(8).next_multiple_of(2);
    let mut data = Vec::new();
    be16(&mut data, row_bytes as u16);
    rect(&mut data, 0, 0, h, w);
    data.extend_from_slice(&tail(w, h, (0, 0), false));
    let mut want = Vec::new();
    for y in 0..h as usize {
        let mut row = vec![0u8; row_bytes];
        for x in 0..w as usize {
            let black = (x + y) % 3 == 0;
            if black {
                row[x / 8] |= 0x80 >> (x % 8);
            }
            want.extend_from_slice(if black { &[0, 0, 0] } else { &[255, 255, 255] });
        }
        data.extend_from_slice(&row);
    }
    (data, want)
}

/// The same bitmap in a version 1 picture (byte opcodes, no alignment).
fn bitmap_v1(w: i16, h: i16) -> (Vec<u8>, Vec<u8>) {
    let (data, want) = bitmap_data(w, h);
    let mut v = Vec::new();
    be16(&mut v, 0);
    rect(&mut v, 0, 0, h, w);
    v.extend_from_slice(&[0x11, 0x01, 0xA0, 0x00, 0x82, 0x90]); // version 1, a ShortComment, BitsRect
    v.extend_from_slice(&data);
    v.push(0xFF);
    (v, want)
}

fn rgb555(x: usize, y: usize) -> u16 {
    let (r, g, b) = ((x * 7 % 32) as u16, (y * 13 % 32) as u16, ((x + y) * 5 % 32) as u16);
    (r << 10) | (g << 5) | b
}

fn expand5(v: u16) -> u8 {
    let v = (v & 31) as u8;
    (v << 3) | (v >> 2)
}

/// A 16-bit DirectBitsRect (or DirectBitsRgn) of `w` x `h` pixels with pack type `pack`.
fn direct16(w: i16, h: i16, pack: u16, rgn: bool) -> (Vec<u8>, Vec<u8>) {
    let row_bytes = w as usize * 2;
    let mut rows = Vec::new();
    let mut want = Vec::new();
    for y in 0..h as usize {
        let mut row = Vec::new();
        for x in 0..w as usize {
            let v = rgb555(x, y);
            row.extend_from_slice(&v.to_be_bytes());
            want.extend_from_slice(&[expand5(v >> 10), expand5(v >> 5), expand5(v)]);
        }
        rows.push(row);
    }
    let mut data = vec![0, 0, 0, 0xFF]; // baseAddr
    data.extend_from_slice(&Pm { row_bytes: row_bytes as u16, w, h, pack, size: 16, cmps: 3 }.bytes());
    data.extend_from_slice(&tail(w, h, (0, 0), rgn));
    if pack == 1 || row_bytes < 8 {
        rows.iter().for_each(|r| data.extend_from_slice(r));
    } else {
        data.extend_from_slice(&packed_rows(&rows, row_bytes, 2));
    }
    (picture(w, h, None, &[op(if rgn { 0x009B } else { 0x009A }, &data)]), want)
}

fn argb(x: usize, y: usize) -> [u8; 4] {
    [(x * 60 + y * 7) as u8, (x * 50) as u8, (y * 80) as u8, ((x + y) * 30) as u8]
}

/// A 32-bit DirectBitsRect of `w` x `h` pixels, pack type `pack`, `cmps` components, alpha from
/// `alpha` (when there are four components). Returns the picture and the expected samples
/// (RGBA when the alpha holds values, else RGB).
fn direct32(w: i16, h: i16, pack: u16, cmps: u16, alpha: fn(usize, usize) -> u8) -> (Vec<u8>, Vec<u8>) {
    let (wu, hu) = (w as usize, h as usize);
    let row_bytes = wu * 4;
    let mut rows = Vec::new();
    let mut samples = Vec::new();
    for y in 0..hu {
        let px: Vec<[u8; 4]> = (0..wu).map(|x| [alpha(x, y), argb(x, y)[1], argb(x, y)[2], argb(x, y)[3]]).collect();
        let row: Vec<u8> = match pack {
            1 => px.iter().flat_map(|p| [if cmps == 4 { p[0] } else { 0 }, p[1], p[2], p[3]]).collect(),
            2 => px.iter().flat_map(|p| [p[1], p[2], p[3]]).collect(),
            _ => {
                let planes: &[usize] = if cmps == 4 { &[0, 1, 2, 3] } else { &[1, 2, 3] };
                planes.iter().flat_map(|&c| px.iter().map(move |p| p[c])).collect()
            }
        };
        rows.push(row);
        samples.extend(px.iter().map(|p| [p[1], p[2], p[3], if cmps == 4 { p[0] } else { 255 }]));
    }
    let mut data = vec![0, 0, 0, 0xFF];
    data.extend_from_slice(&Pm { row_bytes: row_bytes as u16, w, h, pack, size: 32, cmps }.bytes());
    data.extend_from_slice(&tail(w, h, (0, 0), false));
    match pack {
        1 | 2 => rows.iter().for_each(|r| data.extend_from_slice(r)),
        _ => data.extend_from_slice(&packed_rows(&rows, row_bytes, 1)),
    }
    let real_alpha = samples.iter().any(|s| s[3] != samples[0][3]) || !matches!(samples[0][3], 0 | 255);
    let want = if real_alpha { samples.concat() } else { samples.iter().flat_map(|s| [s[0], s[1], s[2]]).collect() };
    (picture(w, h, None, &[op(0x009A, &data)]), want)
}

fn no_alpha(_: usize, _: usize) -> u8 {
    0
}

fn graded_alpha(x: usize, y: usize) -> u8 {
    (x * 40 + y * 20) as u8
}

fn check(bytes: &[u8], w: u32, h: u32, layout: ChannelLayout, want: &[u8]) -> Image {
    assert_eq!(detect(bytes), Some(Format::Pict));
    let img = decode(bytes).unwrap();
    assert_eq!((img.width(), img.height(), img.layout()), (w, h, layout));
    assert_eq!(img.data(), want);
    img
}

#[test]
fn pict_is_read_only_and_registered() {
    assert_eq!(from_extension("scan.PCT"), Some(Format::Pict));
    assert_eq!(from_extension("art.pict"), Some(Format::Pict));
    assert!(caps(Format::Pict).read && !caps(Format::Pict).write);
    assert!(ASYMMETRIC_EXCEPTIONS.iter().any(|(f, _)| *f == Format::Pict));
    let img = Image::from_u8(1, 1, ChannelLayout::Rgb, vec![1, 2, 3]).unwrap();
    assert!(matches!(encode(&img, Format::Pict, &Default::default()), Err(CodecError::Unsupported { .. })));
}

#[test]
fn indexed_pixel_maps_open_with_their_colour_table() {
    for bits in [1, 2, 4, 8] {
        for packed in [false, true] {
            for w in [3, 10, 37] {
                let (bytes, want) = indexed(bits, w, 3, packed);
                check(&bytes, w as u32, 3, ChannelLayout::Rgb, &want);
                let n = COLORS.len().min(1 << bits);
                let colors: Vec<[u8; 3]> = COLORS[..n].iter().map(|c| rgb8(*c)).collect();
                assert_eq!(pict_palette(&bytes).unwrap(), Some(colors), "{bits}-bit");
            }
        }
    }
}

#[test]
fn bitmaps_open_in_version_1_and_version_2_pictures() {
    let (v1, want) = bitmap_v1(13, 4);
    check(&v1, 13, 4, ChannelLayout::Rgb, &want);
    assert_eq!(pict_palette(&v1).unwrap(), Some(vec![[255, 255, 255], [0, 0, 0]]));
    let (data, want) = bitmap_data(13, 4);
    let v2 = picture(13, 4, None, &[op(0x0090, &data)]);
    check(&v2, 13, 4, ChannelLayout::Rgb, &want);
}

#[test]
fn direct_16_bit_pixels_open_stored_and_packed() {
    for pack in [0, 1, 3] {
        for rgn in [false, true] {
            let (bytes, want) = direct16(9, 3, pack, rgn);
            check(&bytes, 9, 3, ChannelLayout::Rgb, &want);
            assert_eq!(pict_palette(&bytes).unwrap(), None);
        }
    }
    // Rows under 8 bytes are stored whatever the pack type says.
    let (bytes, want) = direct16(3, 2, 3, false);
    check(&bytes, 3, 2, ChannelLayout::Rgb, &want);
}

#[test]
fn direct_32_bit_pixels_open_for_every_pack_type() {
    for (pack, cmps) in [(0, 3), (1, 3), (2, 3), (4, 3), (4, 4), (1, 4), (0, 4)] {
        let (bytes, want) = direct32(6, 3, pack, cmps, no_alpha);
        check(&bytes, 6, 3, ChannelLayout::Rgb, &want);
    }
    // Rows over 250 bytes take a 16-bit byte count.
    let (bytes, want) = direct32(70, 2, 4, 3, no_alpha);
    check(&bytes, 70, 2, ChannelLayout::Rgb, &want);
}

#[test]
fn an_alpha_plane_with_values_opens_as_transparency() {
    for pack in [1, 4] {
        let (bytes, want) = direct32(5, 4, pack, 4, graded_alpha);
        check(&bytes, 5, 4, ChannelLayout::Rgba, &want);
    }
    // An all-zero fourth component is the unused pad byte many writers leave: opaque RGB.
    let (bytes, want) = direct32(5, 4, 4, 4, no_alpha);
    check(&bytes, 5, 4, ChannelLayout::Rgb, &want);
}

#[test]
fn a_512_byte_file_header_is_skipped() {
    let (bytes, want) = indexed(8, 10, 2, true);
    let mut with_header = vec![0u8; 512];
    with_header.extend_from_slice(&bytes);
    check(&with_header, 10, 2, ChannelLayout::Rgb, &want);
    let (bytes, want) = bitmap_v1(9, 2);
    let mut with_header = vec![0u8; 512];
    with_header.extend_from_slice(&bytes);
    check(&with_header, 9, 2, ChannelLayout::Rgb, &want);
}

#[test]
fn resolution_comes_from_the_extended_header_or_the_pixel_map() {
    let (o, _) = indexed_op(8, 10, 2, true, (0, 0));
    let img = decode(&picture(10, 2, Some(144), std::slice::from_ref(&o))).unwrap();
    assert_eq!(img.meta.dpi, Some((144.0, 144.0)));
    let img = decode(&picture(10, 2, None, &[o])).unwrap();
    assert_eq!(img.meta.dpi, Some((72.0, 72.0)));
}

#[test]
fn bands_are_drawn_where_the_picture_places_them() {
    let (top, want_top) = indexed_op(8, 10, 2, true, (0, 0));
    let (bottom, want_bottom) = indexed_op(4, 10, 3, true, (0, 2));
    let bytes = picture(10, 5, None, &[top, bottom]);
    let want = [want_top, want_bottom].concat();
    check(&bytes, 10, 5, ChannelLayout::Rgb, &want);
    assert_eq!(pict_palette(&bytes).unwrap(), None, "two tables: no single palette");
}

#[test]
fn skipped_drawing_is_reported_beside_the_pixel_image() {
    let mut clip = vec![0, 10];
    rect(&mut clip, 0, 0, 2, 10);
    let mut text = vec![0, 1, 0, 1, 3];
    text.extend_from_slice(b"abc");
    let mut long_comment = vec![0, 100, 0, 3];
    long_comment.extend_from_slice(b"xyz");
    let mut pix_pat = vec![0, 2];
    pix_pat.extend_from_slice(&[0xAA; 8]);
    pix_pat.extend_from_slice(&[0xFF; 6]);
    let (image, want) = indexed_op(8, 10, 2, true, (0, 0));
    let ops = [
        op(0x0001, &clip),
        op(0x001E, &[]),
        op(0x0004, &[1]),
        op(0x001A, &[0xFF; 6]),
        op(0x0012, &pix_pat),
        op(0x0020, &[0, 0, 0, 0, 0, 5, 0, 5]),
        op(0x0031, &[0, 0, 0, 0, 0, 2, 0, 2]),
        op(0x0028, &text),
        op(0x00A0, &[0, 1]),
        op(0x00A1, &long_comment),
        op(0x00D0, &[0, 0, 0, 2, 9, 9]),
        op(0x0C01, &[0; 24]),
        op(0x8000, &[]),
        op(0x8100, &[0, 0, 0, 0]),
        image,
    ];
    let bytes = picture(10, 2, None, &ops);
    let img = check(&bytes, 10, 2, ChannelLayout::Rgb, &want);
    assert_eq!(img.warnings, [DecodeWarning::DrawingSkipped { format: Format::Pict, count: 3 }]);
    assert!(img.warnings[0].to_string().contains("3 PICT drawing operations"));
}

#[test]
fn pictures_without_a_pixel_image_are_unsupported() {
    let vector = picture(10, 10, None, &[op(0x0020, &[0, 0, 0, 0, 0, 5, 0, 5]), op(0x0031, &[0, 0, 0, 0, 0, 2, 0, 2])]);
    match decode(&vector) {
        Err(CodecError::Unsupported { format: Format::Pict, reason }) => assert!(reason.contains("QuickDraw drawing"), "{reason}"),
        other => panic!("{other:?}"),
    }
    let quicktime = picture(10, 10, None, &[op(0x8200, &[0, 0, 0, 4, 1, 2, 3, 4])]);
    match decode(&quicktime) {
        Err(CodecError::Unsupported { format: Format::Pict, reason }) => assert!(reason.contains("QuickTime"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(decode(&picture(10, 10, None, &[])), Err(CodecError::Malformed { .. })));
}

#[test]
fn a_picture_without_its_end_opcode_opens_with_a_warning() {
    let (mut bytes, want) = indexed(8, 10, 2, true);
    bytes.truncate(bytes.len() - 2);
    let img = check(&bytes, 10, 2, ChannelLayout::Rgb, &want);
    assert_eq!(img.warnings, [DecodeWarning::Truncated { format: Format::Pict }]);
}

#[test]
fn hostile_headers_are_errors() {
    // A row that unpacks past its row bytes.
    let mut data = Pm { row_bytes: 8, w: 8, h: 1, pack: 0, size: 8, cmps: 1 }.bytes();
    data.extend_from_slice(&ctab(&COLORS));
    data.extend_from_slice(&tail(8, 1, (0, 0), false));
    data.extend_from_slice(&[2, 0x80 | 0x75, 1]); // repeat 1 twelve times
    assert!(matches!(decode(&picture(8, 1, None, &[op(0x0098, &data)])), Err(CodecError::Malformed { .. })));
    // A colour table of more than 256 entries.
    let mut data = Pm { row_bytes: 8, w: 8, h: 1, pack: 0, size: 8, cmps: 1 }.bytes();
    data.extend_from_slice(&[0, 0, 0, 0, 0, 0, 1, 0x2B]);
    assert!(matches!(decode(&picture(8, 1, None, &[op(0x0098, &data)])), Err(CodecError::Malformed { .. })));
    // A region shorter than its own size word.
    assert!(matches!(decode(&picture(8, 1, None, &[op(0x0001, &[0, 1])])), Err(CodecError::Malformed { .. })));
    // 24-bit direct pixels are not a QuickDraw pixel size.
    let mut data = vec![0, 0, 0, 0xFF];
    data.extend_from_slice(&Pm { row_bytes: 24, w: 8, h: 1, pack: 1, size: 24, cmps: 3 }.bytes());
    data.extend_from_slice(&tail(8, 1, (0, 0), false));
    data.extend_from_slice(&[0; 24]);
    assert!(matches!(decode(&picture(8, 1, None, &[op(0x009A, &data)])), Err(CodecError::Unsupported { .. })));
    // An empty pixel image.
    let mut data = Pm { row_bytes: 8, w: 0, h: 1, pack: 0, size: 8, cmps: 1 }.bytes();
    data.extend_from_slice(&ctab(&COLORS));
    data.extend_from_slice(&tail(0, 1, (0, 0), false));
    assert!(matches!(decode(&picture(8, 1, None, &[op(0x0098, &data)])), Err(CodecError::Malformed { .. })));
}

#[test]
fn a_huge_declared_image_needs_its_rows() {
    // 32,000 x 32,000 RGB declared, one row present: an error long before a 4 GB buffer.
    let mut data = vec![0, 0, 0, 0xFF];
    data.extend_from_slice(&Pm { row_bytes: 0x3FFC, w: 32_000, h: 32_000, pack: 4, size: 32, cmps: 3 }.bytes());
    data.extend_from_slice(&tail(32_000, 32_000, (0, 0), false));
    data.extend_from_slice(&packed_rows(&[vec![7; 96_000]], 0x3FFC, 1));
    let bytes = picture(32_000, 32_000, None, &[op(0x009A, &data)]);
    assert!(decode(&bytes).is_err());
    // Limits apply before the pixels are read.
    let (bytes, _) = indexed(8, 10, 3, true);
    let opts = DecodeOptions { limits: Limits::new(64, 64, 20, 1 << 20), ..Default::default() };
    assert!(matches!(decode_as_with(Format::Pict, &bytes, &opts), Err(CodecError::LimitExceeded(_))));
}

#[test]
fn bands_far_apart_cannot_ask_for_a_huge_canvas() {
    let (a, _) = indexed_op(8, 10, 2, true, (0, 0));
    let (b, _) = indexed_op(8, 10, 2, true, (30_000, 30_000));
    assert!(matches!(decode(&picture(10, 2, None, &[a, b])), Err(CodecError::LimitExceeded(_))));
}

fn fixtures() -> Vec<Vec<u8>> {
    let mut all = vec![
        indexed(8, 10, 3, true).0,
        indexed(1, 13, 2, false).0,
        indexed(4, 9, 2, true).0,
        bitmap_v1(13, 3).0,
        direct16(9, 2, 0, true).0,
        direct16(9, 2, 1, false).0,
        direct32(5, 2, 4, 4, graded_alpha).0,
        direct32(4, 2, 1, 3, no_alpha).0,
        direct32(4, 2, 2, 3, no_alpha).0,
    ];
    let (top, _) = indexed_op(8, 10, 1, true, (0, 0));
    let (bottom, _) = indexed_op(8, 10, 1, true, (0, 1));
    all.push(picture(10, 2, Some(144), &[op(0x0020, &[0; 8]), top, bottom]));
    all
}

/// Every truncation and every single-bit flip of the fixtures decodes or fails cleanly.
#[test]
fn truncated_and_bit_flipped_pictures_never_panic() {
    for bytes in fixtures() {
        for len in 0..bytes.len() {
            let _ = decode(&bytes[..len]);
            let _ = pict_palette(&bytes[..len]);
        }
        let mut flipped = bytes.clone();
        for i in 0..bytes.len() {
            for bit in 0..8 {
                flipped[i] ^= 1 << bit;
                if let Ok(img) = decode(&flipped) {
                    assert!(img.width() > 0 && img.height() > 0);
                }
                let _ = pict_palette(&flipped);
                flipped[i] ^= 1 << bit;
            }
        }
    }
}
