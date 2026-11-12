//! Hostile-input tests for the work, memory and overflow bounds: ported from PdfCraft's
//! raster tests, which draw the same streams through a PDF. Here a stream "draws" when it decodes.

use crate::{Decoder, Image};

#[derive(Default)]
struct Count {
    black: u64,
}

impl Decoder for Count {
    fn push_pixel(&mut self, black: bool) {
        self.black += u64::from(black);
    }

    fn push_pixel_chunk(&mut self, black: bool, chunk_count: u32) {
        self.black += u64::from(black) * 8 * u64::from(chunk_count);
    }

    fn next_line(&mut self) {}
}

/// The black pixels of an embedded stream; none when it is refused.
fn black_pixels(stream: &[u8]) -> u64 {
    let mut count = Count::default();
    match Image::new_embedded(stream, None).and_then(|image| image.decode(&mut count)) {
        Ok(()) => count.black,
        Err(_) => 0,
    }
}

/// Whether a 16 x 2 stream whose page defaults to black decodes: refused, nothing is drawn.
fn jbig2_image_draws(stream: &[u8]) -> bool {
    black_pixels(stream) > 0
}

/// Bits written from the top of each byte, as JBIG2's Huffman-coded data is.
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    used: u32,
}

impl Bits {
    fn put(&mut self, value: u32, count: u32) {
        for i in (0..count).rev() {
            if self.used.is_multiple_of(8) {
                self.bytes.push(0);
            }
            if (value >> i) & 1 != 0 {
                *self.bytes.last_mut().unwrap() |= 0x80 >> (self.used % 8);
            }
            self.used += 1;
        }
    }

    fn align(&mut self) {
        self.used = self.used.div_ceil(8) * 8;
    }

    /// `value` in table B.1 (ISO 14492 Annex B).
    fn table_b1(&mut self, value: u32) {
        match value {
            0..=15 => {
                self.put(0, 1);
                self.put(value, 4);
            }
            16..=271 => {
                self.put(0b10, 2);
                self.put(value - 16, 8);
            }
            272..=65807 => {
                self.put(0b110, 3);
                self.put(value - 272, 16);
            }
            _ => {
                self.put(0b111, 3);
                self.put(value - 65808, 32);
            }
        }
    }
}

/// One embedded JBIG2 segment (ISO 14492 §7.2) of page 1. More than four references take the long
/// form, whose retention flags are left clear.
fn jbig2_segment(out: &mut Vec<u8>, number: u32, kind: u8, refs: &[u8], data: &[u8]) {
    out.extend_from_slice(&number.to_be_bytes());
    out.push(kind);
    if refs.len() <= 4 {
        out.push(u8::try_from(refs.len()).unwrap() << 5);
    } else {
        out.extend_from_slice(&(0xE000_0000u32 | u32::try_from(refs.len()).unwrap()).to_be_bytes());
        out.extend(std::iter::repeat_n(0u8, (refs.len() + 1).div_ceil(8)));
    }
    out.extend_from_slice(refs);
    out.push(1);
    out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
    out.extend_from_slice(data);
}

/// The page information of a 16 × 2 JBIG2 page whose default pixel is black, so that the image, if
/// it decodes, draws black whatever its other segments hold.
fn black_jbig2_page() -> Vec<u8> {
    let mut page = Vec::new();
    page.extend_from_slice(&16u32.to_be_bytes());
    page.extend_from_slice(&2u32.to_be_bytes());
    page.extend_from_slice(&[0; 8]); // resolution
    page.extend_from_slice(&[4, 0, 0]); // flags (default pixel black), striping
    let mut stream = Vec::new();
    jbig2_segment(&mut stream, 0, 48, &[], &page);
    stream
}

/// A region's information field, at the page's corner.
fn jbig2_region_info(width: u32, height: u32) -> Vec<u8> {
    let mut region = Vec::new();
    region.extend_from_slice(&width.to_be_bytes());
    region.extend_from_slice(&height.to_be_bytes());
    region.extend_from_slice(&[0; 9]); // x, y, combination operator
    region
}

/// An intermediate generic region, white, every row typical: it decodes in a few bytes however
/// large it is.
fn jbig2_white_region(width: u32, height: u32) -> Vec<u8> {
    let mut region = jbig2_region_info(width, height);
    region.push(0x08); // arithmetic coding, template 0, typical prediction
    region.extend_from_slice(&[3, 0xff, 0xfd, 0xff, 2, 0xfe, 0xfe, 0xfe]); // AT pixels
    // Every row typical, the first white: pdfcraft-filters' coding of a white 16,384 square.
    region.extend_from_slice(&[0xb6, 0xc2, 0x1f, 0xff, 0x7f, 0xff, 0xac]);
    region
}

/// A pattern dictionary of `patterns` patterns of width 1 and `height` rows, with nothing coded.
fn jbig2_pattern_dictionary(height: u8, patterns: u32) -> Vec<u8> {
    let mut dictionary = vec![0, 1, height];
    dictionary.extend_from_slice(&(patterns - 1).to_be_bytes());
    dictionary
}

/// A 1 × 1 halftone region over a grid of `width` × `height` cells, all at the region's corner,
/// coded by MMR or arithmetically.
fn jbig2_halftone(width: u32, height: u32, mmr: bool) -> Vec<u8> {
    jbig2_halftone_of(1, width, height, mmr)
}

/// [`jbig2_halftone`] for a square region of `side` pixels.
fn jbig2_halftone_of(side: u32, width: u32, height: u32, mmr: bool) -> Vec<u8> {
    let mut halftone = jbig2_region_info(side, side);
    halftone.push(u8::from(mmr)); // template 0
    halftone.extend_from_slice(&width.to_be_bytes());
    halftone.extend_from_slice(&height.to_be_bytes());
    halftone.extend_from_slice(&[0; 12]); // grid offset and vector
    halftone
}

/// A Huffman-coded symbol dictionary of `symbols` symbols of width 0 and height 1, exported or not:
/// each holds no pixels, and costs a bitmap's header all the same.
fn zero_width_jbig2_dictionary(symbols: u32, exported: bool) -> Vec<u8> {
    let mut dictionary = vec![0x00, 0x01]; // Huffman, tables B.4, B.2, B.1, B.1
    dictionary.extend_from_slice(&(if exported { symbols } else { 0 }).to_be_bytes());
    dictionary.extend_from_slice(&symbols.to_be_bytes()); // new
    let mut bits = Bits::default();
    bits.put(0, 1); // height class delta 1 (B.4)
    for _ in 0..symbols {
        bits.put(0, 1); // width delta 0 (B.2)
    }
    bits.put(0b111111, 6); // end of the class (B.2)
    bits.put(0, 5); // BMSIZE 0, uncompressed (B.1)
    bits.align();
    if exported {
        bits.table_b1(0); // the first export run isn't exported
    }
    bits.table_b1(symbols);
    dictionary.extend_from_slice(&bits.bytes);
    dictionary
}

/// A Huffman-coded symbol dictionary of one white symbol, `side` from 76 up, exported or not.
fn square_jbig2_dictionary(side: u32, exported: bool) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.put(0b11111, 5); // height class delta, the upper line of table B.4
    bits.put(side - 76, 32);
    bits.put(0b111110, 6); // width delta, the upper line of table B.2
    bits.put(side - 75, 32);
    bits.put(0b111111, 6); // end of the class (B.2)
    let bytes = side.div_ceil(8) + 1; // an MMR-coded white bitmap: one bit a row
    bits.table_b1(bytes); // BMSIZE (B.1)
    bits.align();
    for _ in 0..bytes {
        bits.put(0xFF, 8);
    }
    if exported {
        bits.table_b1(0);
    }
    bits.table_b1(1);
    let mut dictionary = vec![0x00, 0x01]; // Huffman, tables B.4, B.2, B.1, B.1
    dictionary.extend_from_slice(&u32::from(exported).to_be_bytes());
    dictionary.extend_from_slice(&1u32.to_be_bytes()); // new
    dictionary.extend_from_slice(&bits.bytes);
    dictionary
}

/// A text region of `width` × `height` placing symbol 0 of one, Huffman-coded, `instances` of 0 or 1.
fn jbig2_text_region(width: u32, height: u32, instances: u32) -> Vec<u8> {
    let mut text = jbig2_region_info(width, height);
    text.extend_from_slice(&[0x00, 0x11, 0, 0]); // Huffman, top left; tables B.6, B.8, B.11
    text.extend_from_slice(&instances.to_be_bytes());
    let mut bits = Bits::default();
    for code in 0..35 {
        bits.put(u32::from(code == 1), 4); // only run code 1, one bit long
    }
    bits.put(0, 1); // symbol 0 is one bit long
    bits.align();
    bits.put(0, 1); // the first strip's T (B.11)
    if instances > 0 {
        bits.put(0, 1); // delta T 1 (B.11)
        bits.put(0, 9); // first S 0 (B.6)
        bits.put(0, 1); // symbol 0
        bits.put(0b01, 2); // end of the strip (B.8)
    }
    text.extend_from_slice(&bits.bytes);
    text
}

/// A 1 × 1 text region placing one instance of symbol 0 of one, refined, Huffman-coded.
fn jbig2_refined_text_region() -> Vec<u8> {
    let mut text = jbig2_region_info(1, 1);
    text.extend_from_slice(&[0x80, 0x13, 0, 0]); // Huffman, refined by template 1, top left
    text.extend_from_slice(&1u32.to_be_bytes());
    let mut bits = Bits::default();
    for code in 0..35 {
        bits.put(u32::from(code == 1), 4); // only run code 1, one bit long
    }
    bits.put(0, 1); // symbol 0 is one bit long
    bits.align();
    bits.put(0, 1); // the first strip's T (B.11)
    bits.put(0, 1); // delta T 1 (B.11)
    bits.put(0, 9); // first S 0 (B.6)
    bits.put(0, 1); // symbol 0
    bits.put(1, 1); // refined
    bits.put(0, 4); // no change of width, height or offset (B.14)
    bits.table_b1(0); // no refinement data (B.1)
    bits.align();
    bits.put(0b01, 2); // end of the strip (B.8)
    text.extend_from_slice(&bits.bytes);
    text
}

/// A generic refinement region of 1 × 1, with whatever its data decodes to.
fn jbig2_refinement_region() -> Vec<u8> {
    let mut region = jbig2_region_info(1, 1);
    region.push(1); // template 1, no typical prediction
    region.extend_from_slice(&[0; 4]);
    region
}

/// Each item a JBIG2 stream decodes costs its work, and the stream may ask for 2^30: four of the
/// largest pages, some 6 s to 17 s. Each bitmap is bounded on its own, but nothing bounded how many
/// there were, so 1,140 bytes of generic regions took over 40 s, and a 795-byte PDF's one halftone
/// 265 s. Vendored hayro-jbig2 patch: `MAX_WORK`. A row costs 4 pixels, even in a bitmap of no
/// width, and a copy goes by the word and the row, so a tall thin one costs its rows. Here regions
/// of typical rows, cheap to decode though charged like any, spend all but exactly what the item
/// costs, so it draws with that left and is refused with one less, for each item at its own cost.
#[test]
fn jbig2_decoding_work_is_counted_item_by_item() {
    struct Item {
        what: &'static str,
        /// What comes first, and what it costs.
        before: Vec<(u8, Vec<u8>)>,
        spent: u64,
        kind: u8,
        refs: &'static [u8],
        data: Vec<u8>,
        cost: u64,
    }
    let stream = |item: &Item, left: u64| {
        let mut stream = black_jbig2_page();
        let mut number = 0;
        for (kind, data) in &item.before {
            number += 1;
            jbig2_segment(&mut stream, number, *kind, &[], data);
        }
        // A square costs 16,384 rows of 16,388, and a single row what it has and 4 more.
        let work = (1u64 << 30) - item.spent - left;
        let (row, square) = (16388u64, 16384 * 16388u64);
        let mut squares = work / square;
        let mut rest = work % square;
        if rest > 0 && rest < 5 {
            squares -= 1;
            rest += square;
        }
        let mut rows = rest / row;
        let mut last = rest % row;
        if last > 0 && last < 5 {
            rows -= 1;
            last += row;
        }
        let regions = std::iter::repeat_n((16384, 16384), usize::try_from(squares).unwrap())
            .chain((rows > 0).then_some((16384, rows)))
            .chain((last > 0).then_some((last - 4, 1)));
        for (width, height) in regions {
            number += 1;
            let region = jbig2_white_region(u32::try_from(width).unwrap(), u32::try_from(height).unwrap());
            jbig2_segment(&mut stream, number, 38, &[], &region); // immediate generic region
        }
        jbig2_segment(&mut stream, number + 1, item.kind, item.refs, &item.data);
        stream
    };
    let symbol = || vec![(0, zero_width_jbig2_dictionary(1, true))];
    let patterns = || vec![(16, jbig2_pattern_dictionary(1, 2))];
    let exporting_none = {
        let mut dictionary = vec![0x00, 0x01];
        dictionary.extend_from_slice(&[0; 8]); // exported, new
        let mut bits = Bits::default();
        bits.table_b1(1); // the first export run, not exported, is the one it refers to
        dictionary.extend_from_slice(&bits.bytes);
        dictionary
    };
    let zero = |height: u32| {
        let mut bits = Bits::default();
        bits.put(0b11111, 5); // height class delta, the upper line of table B.4
        bits.put(height - 76, 32);
        bits.put(0, 1); // width delta 0 (B.2)
        bits.put(0b111111, 6); // end of the class (B.2)
        bits.table_b1(0); // BMSIZE 0, stored (B.1)
        bits.align();
        bits.table_b1(1); // the first export run, not exported, is the symbol
        let mut dictionary = vec![0x00, 0x01];
        dictionary.extend_from_slice(&0u32.to_be_bytes()); // exported
        dictionary.extend_from_slice(&1u32.to_be_bytes()); // new
        dictionary.extend_from_slice(&bits.bytes);
        dictionary
    };
    // The cost of a pattern dictionary of two patterns: rows of them side by side, then each taken out.
    let two_patterns = |width: u64, height: u64| height * (2 * width + 4) + 2 * height * (width.div_ceil(32) + 1);
    let thin_region = |side: u32| {
        let mut region = jbig2_white_region(1, 1);
        region.splice(0..8, [0u32.to_be_bytes(), side.to_be_bytes()].concat());
        region
    };
    let items = vec![
        Item { what: "immediate generic", before: vec![], spent: 0, kind: 38, refs: &[], data: jbig2_white_region(1, 1), cost: 5 },
        Item { what: "intermediate generic", before: vec![], spent: 0, kind: 36, refs: &[], data: jbig2_white_region(1, 1), cost: 5 },
        Item { what: "a region of no width", before: vec![], spent: 0, kind: 38, refs: &[], data: thin_region(1000), cost: 4000 },
        Item { what: "immediate refinement", before: vec![], spent: 0, kind: 42, refs: &[], data: jbig2_refinement_region(), cost: 5 },
        Item { what: "intermediate refinement", before: vec![], spent: 0, kind: 40, refs: &[], data: jbig2_refinement_region(), cost: 5 },
        Item {
            what: "pattern dictionary",
            before: vec![],
            spent: 0,
            kind: 16,
            refs: &[],
            data: jbig2_pattern_dictionary(1, 2),
            cost: two_patterns(1, 1),
        },
        Item {
            what: "tall patterns",
            before: vec![],
            spent: 0,
            kind: 16,
            refs: &[],
            data: jbig2_pattern_dictionary(32, 2),
            cost: two_patterns(1, 32),
        },
        // Rows cost in a symbol of no pixels too.
        Item { what: "symbols without pixels", before: vec![], spent: 0, kind: 0, refs: &[], data: zero_width_jbig2_dictionary(3, false), cost: 12 },
        Item { what: "a tall symbol of no width", before: vec![], spent: 0, kind: 0, refs: &[], data: zero(1000), cost: 4000 },
        Item { what: "new symbols", before: vec![], spent: 0, kind: 0, refs: &[], data: square_jbig2_dictionary(80, false), cost: 6720 },
        Item { what: "referred symbols", before: symbol(), spent: 4, kind: 0, refs: &[1], data: exporting_none, cost: 1 },
        // Its symbol, and its one row and word; Huffman-coded, a table line for the symbol.
        Item { what: "immediate text", before: symbol(), spent: 4, kind: 6, refs: &[1], data: jbig2_text_region(1, 1, 0), cost: 13 },
        Item { what: "intermediate text", before: symbol(), spent: 4, kind: 4, refs: &[1], data: jbig2_text_region(1, 1, 0), cost: 13 },
        Item { what: "a text region's pixels", before: symbol(), spent: 4, kind: 6, refs: &[1], data: jbig2_text_region(32, 32, 0), cost: 75 },
        Item {
            what: "an intermediate text region's pixels",
            before: symbol(),
            spent: 4,
            kind: 4,
            refs: &[1],
            data: jbig2_text_region(32, 32, 0),
            cost: 75,
        },
        // The region's 13, a copy of the symbol's row and the round of the loop.
        Item { what: "an instance", before: symbol(), spent: 4, kind: 6, refs: &[1], data: jbig2_text_region(1, 1, 1), cost: 15 },
        // The region's 13, 80 rows of 4 words copied, the round of the loop.
        Item {
            what: "an instance of a large symbol",
            before: vec![(0, square_jbig2_dictionary(80, true))],
            spent: 6720,
            kind: 6,
            refs: &[1],
            data: jbig2_text_region(1, 1, 1),
            cost: 334,
        },
        // And decoded as well as copied.
        Item {
            what: "a refined instance",
            before: vec![(0, square_jbig2_dictionary(80, true))],
            spent: 6720,
            kind: 6,
            refs: &[1],
            data: jbig2_refined_text_region(),
            cost: 7054,
        },
        // A bit plane, a cell's copy and the region's.
        Item {
            what: "immediate halftone",
            before: patterns(),
            spent: two_patterns(1, 1),
            kind: 22,
            refs: &[1],
            data: jbig2_halftone(1, 1, false),
            cost: 9,
        },
        Item {
            what: "intermediate halftone",
            before: patterns(),
            spent: two_patterns(1, 1),
            kind: 20,
            refs: &[1],
            data: jbig2_halftone(1, 1, false),
            cost: 9,
        },
        // A plane of 2 cells, each a copy of 32 rows of 2 words' cost, and the region's.
        Item {
            what: "a halftone's cells",
            before: vec![(16, jbig2_pattern_dictionary(32, 2))],
            spent: two_patterns(1, 32),
            kind: 22,
            refs: &[1],
            data: jbig2_halftone(2, 1, false),
            cost: 136,
        },
        Item {
            what: "a grid of no width",
            before: patterns(),
            spent: two_patterns(1, 1),
            kind: 22,
            refs: &[1],
            data: jbig2_halftone(0, 1000, false),
            cost: 4002,
        },
        Item {
            what: "a halftone's pixels",
            before: patterns(),
            spent: two_patterns(1, 1),
            kind: 22,
            refs: &[1],
            data: jbig2_halftone_of(32, 1, 1, false),
            cost: 71,
        },
    ];
    for item in &items {
        assert!(jbig2_image_draws(&stream(item, item.cost)), "{}: with its cost left it draws", item.what);
        assert!(!jbig2_image_draws(&stream(item, item.cost - 1)), "{}: with one less it's refused", item.what);
    }
}

/// A JBIG2 stream can put a region, a reference or a symbol at the very end of what an `i32` holds,
/// where adding to it overflowed: a panic wherever overflow is checked, a build's own tests included.
/// A region or a symbol out there is off the page, so nothing draws. Vendored hayro-jbig2 patch: the arithmetic wraps, as
/// a release build's did, or in `Bitmap::combine` saturates, which comes to the same. Three of the text
/// streams are arithmetic-coded: one symbol on a strip of -2^31 + 1, with its bottom left as the corner
/// placed, one on a strip of -2^31, and one refined by an offset of `i32::MIN`, which reads nothing of
/// the old symbol and still draws. Two are Huffman-coded, each with its first S at an offset of
/// 0x7FFFFFFF on a table's 32-bit range line, past what an `i32` holds, and are refused further on.
#[test]
fn jbig2_placements_at_the_end_of_an_i32_draw_nothing() {
    const FAR_CORNER: &str = "0000000030000100000013000000100000000c0000000000000000420000000000010000010000001e000003fffdff02fefefe00000001000000014fdc3ce0205df55c36bfffac00000002062001010000001f000000100000000c000000000000000004000000000001969747cbe88bffac";
    const STRIP: &str = "0000000030000100000013000000100000000c0000000000000000420000000000010000010000001e000003fffdff02fefefe00000001000000014fdda25e46956d3e0d3fffac00000002062001010000001f000000100000000c000000000000000004001000000001969747cbe5dbffac";
    const UPPER_LINE: &str = "0000000030000100000013000000100000000c0000000000000000420000000000010000010000001d000003fffdff02fefefe00000001000000014fdaea38c99641e0c8ffac000000020620010100000038000000100000000c00000000000000000400110000000000010100000000000000000000000000000000003f7fffffff0000000000000000";
    const LOWER_LINE: &str = "0000000030000100000013000000100000000c0000000000000000420000000000010000010000001d000003fffdff02fefefe00000001000000014fdaea38c99641e0c8ffac000000020620010100000038000000100000000c00000000000000000400110000000000010100000000000000000000000000000000003e7fffffff0000000000000000";
    const OFFSET: &str = "0000000030000100000013000000100000000c0000000000000000420000000000010000010000001e000003fffdff02fefefe00000001000000014fdda9172f614a037ddfffac0000000206200101000000260000000800000006000000000000000004801200000001a9695690b3e5f44a2f105e30d3ffac";
    let segment = |number: u32, kind: u8, data: &[u8]| {
        let mut out = number.to_be_bytes().to_vec();
        out.extend_from_slice(&[kind, 0, 1]);
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        out.extend_from_slice(data);
        out
    };
    let side = 8u32.to_be_bytes();
    let page = |width: u32| {
        let mut page = [width.to_be_bytes(), side].concat();
        page.extend_from_slice(&[0; 8]); // resolution
        page.extend_from_slice(&[0, 0, 0]); // flags, striping
        segment(0, 48, &page)
    };
    let region = |width: u32, kind: u8, x: u32, y: u32, header: &[u8]| {
        let mut region = [width.to_be_bytes(), side, x.to_be_bytes(), y.to_be_bytes()].concat();
        region.push(0); // combination operator
        region.extend_from_slice(header);
        region.extend_from_slice(&[0x5a; 64]);
        [page(width), segment(1, kind, &region)].concat()
    };
    let generic = |x, y| region(8, 38, x, y, &[0, 3, 0xff, 0xfd, 0xff, 2, 0xfe, 0xfe, 0xfe]); // template 0, AT pixels
    // Immediate refinements of the page: template 0 with its usual AT pixels, with others, with typical
    // prediction, and template 1.
    let refined = |width, x, y, form: usize| {
        region(width, 42, x, y, [&[0, 0xff, 0xff, 0xff, 0xff][..], &[0, 0xff, 0xff, 127, 127], &[2, 0xff, 0xff, 0xff, 0xff], &[1]][form])
    };
    let hex = |text: &str| (0..text.len() / 2).map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap()).collect::<Vec<u8>>();
    let ink = |jbig2: Vec<u8>, _width: u32, _height: u32| black_pixels(&jbig2);
    assert!(ink(generic(2, 0), 8, 8) > 0, "a region on the page draws");
    for form in 0..4 {
        assert!(ink(refined(8, 0, 0, form), 8, 8) > 0, "a refinement on the page draws: {form}");
    }
    let end = 0x7FFF_FFFFu32;
    for (x, y) in [(end, 0), (0, end), (end - 7, 0), (0, end - 7), (end - 129, 0), (0, end - 129)] {
        assert_eq!(ink(generic(x, y), 8, 8), 0, "generic at {x:#x}, {y:#x}");
        for form in 0..4 {
            assert_eq!(ink(refined(8, x, y, form), 8, 8), 0, "refinement {form} at {x:#x}, {y:#x}");
        }
    }
    // Past a 32-bit word of the row, where the reference is read again.
    assert_eq!(ink(refined(64, 0, end, 0), 64, 8), 0, "a wide refinement");
    assert_eq!(ink(hex(FAR_CORNER), 16, 12), 0, "a symbol's far side past the end");
    assert_eq!(ink(hex(STRIP), 16, 12), 0, "a strip of -2^31");
    assert!(ink(hex(OFFSET), 16, 12) > 0, "a symbol refined by an offset of i32::MIN");
    assert_eq!(ink(hex(UPPER_LINE), 16, 12), 0, "an upper range line past an i32");
    assert_eq!(ink(hex(LOWER_LINE), 16, 12), 0, "a lower range line past an i32");
}
