//! Interface fonts.

mod document;
mod on_demand;
mod served;
mod stack;

/// Builds a minimal SFNT font mapping `glyphs` (`(codepoint, has_outline)`) under `family`.
/// When `colr` is `true`, every mapped glyph also carries a COLRv0 record (and a `CPAL` table)
/// so colour-font handling can be tested even with empty `glyf` outlines.
pub(super) fn synthetic_ttf(family: &str, glyphs: &[(char, bool)], colr: bool) -> Vec<u8> {
    let mut sorted: Vec<(u32, bool)> = glyphs.iter().map(|&(c, outline)| (u32::from(c), outline)).collect();
    sorted.sort_by_key(|&(code, _)| code);

    let be16 = |v: &mut Vec<u8>, x: i32| v.extend((x as u16).to_be_bytes());
    let be32 = |v: &mut Vec<u8>, x: u32| v.extend(x.to_be_bytes());

    let mut boxed = Vec::new();
    for x in [1, 50, 0, 550, 700, 3, 0] {
        be16(&mut boxed, x);
    }
    boxed.extend([1u8; 4]);
    for x in [50, 0, 500, 0, 0, 700, 0, -700] {
        be16(&mut boxed, x);
    }
    boxed.extend([0, 0]);

    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    // Glyph 0 (.notdef): empty outline.
    be32(&mut loca, 0);
    be32(&mut loca, 0);
    for &(_, has_outline) in &sorted {
        if has_outline {
            glyf.extend_from_slice(&boxed);
        }
        be32(&mut loca, glyf.len() as u32);
    }

    let num_glyphs = (1 + sorted.len()) as i32;
    let mut head = Vec::new();
    for x in [0x0001_0000, 0x0001_0000, 0, 0x5F0F_3CF5] {
        be32(&mut head, x);
    }
    for x in [0x000B, 1000, 0, 0, 0, 0, 0, 0, 0, 0, 0, -200, 600, 800, 0, 8, 2, 1, 0] {
        be16(&mut head, x);
    }

    let mut hhea = Vec::new();
    be32(&mut hhea, 0x0001_0000);
    for x in [800, -200, 0, 600, 0, 0, 550, 1, 0, 0, 0, 0, 0, 0, 0, num_glyphs] {
        be16(&mut hhea, x);
    }

    let mut maxp = Vec::new();
    be32(&mut maxp, 0x0001_0000);
    for x in [num_glyphs, 4, 1, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0] {
        be16(&mut maxp, x);
    }

    let mut hmtx = Vec::new();
    for _ in 0..num_glyphs {
        be16(&mut hmtx, 600);
        be16(&mut hmtx, 50);
    }

    let mut cmap = Vec::new();
    for x in [0, 1, 3, 10] {
        be16(&mut cmap, x);
    }
    be32(&mut cmap, 12);
    for x in [12, 0] {
        be16(&mut cmap, x);
    }
    be32(&mut cmap, 16 + 12 * sorted.len() as u32);
    be32(&mut cmap, 0);
    be32(&mut cmap, sorted.len() as u32);
    for (i, &(code, _)) in sorted.iter().enumerate() {
        be32(&mut cmap, code);
        be32(&mut cmap, code);
        be32(&mut cmap, (i + 1) as u32);
    }

    let utf16: Vec<u8> = family.encode_utf16().flat_map(u16::to_be_bytes).collect();
    let mut name = Vec::new();
    for x in [0, 1, 18, 3, 1, 0x0409, 1, utf16.len() as i32, 0] {
        be16(&mut name, x);
    }
    name.extend(utf16);

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = Vec::new();
    if colr {
        let count = sorted.len() as u32;
        let mut colr_table = Vec::new();
        be16(&mut colr_table, 0);
        be16(&mut colr_table, count as i32);
        be32(&mut colr_table, 14);
        be16(&mut colr_table, count as i32);
        be32(&mut colr_table, 14 + 6 * count);
        for i in 0..count {
            let gid = (i + 1) as i32;
            be16(&mut colr_table, gid);
            be16(&mut colr_table, i as i32);
            be16(&mut colr_table, 1);
        }
        for i in 0..count {
            be16(&mut colr_table, (i + 1) as i32);
            be16(&mut colr_table, 0);
        }
        let mut cpal = Vec::new();
        for x in [0, 1, 1, 1] {
            be16(&mut cpal, x);
        }
        be32(&mut cpal, 14);
        be16(&mut cpal, 0);
        cpal.extend([0xFF, 0x00, 0x00, 0xFF]);
        tables.push((b"COLR", colr_table));
        tables.push((b"CPAL", cpal));
    }
    tables.extend([
        (b"cmap", cmap),
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"name", name),
    ]);

    let n = tables.len() as u16;
    let entry_selector = n.ilog2() as u16;
    let search_range = (1u16 << entry_selector) * 16;
    let range_shift = n * 16 - search_range;

    let mut font = Vec::new();
    be32(&mut font, 0x0001_0000);
    for x in [n, search_range, entry_selector, range_shift] {
        be16(&mut font, i32::from(x));
    }
    let mut offset = 12 + 16 * u32::from(n);
    for (tag, data) in &tables {
        font.extend(*tag);
        be32(&mut font, 0);
        be32(&mut font, offset);
        be32(&mut font, data.len() as u32);
        offset += (data.len() as u32).next_multiple_of(4);
    }
    for (_, data) in &tables {
        font.extend(data);
        font.resize(font.len().next_multiple_of(4), 0);
    }
    font
}
