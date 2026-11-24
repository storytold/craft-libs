//! TrueType fonts of a page written again as PostScript fonts: a subset of the glyphs the page
//! shows, as Type 42 fonts (Adobe Technical Note 5012) or as a CIDFontType 2 font in an
//! `Identity-H` composite font (PostScript Language Reference, third edition, §5.11.4).
//! Fonts with CFF outlines, and fonts that don't read or subset, are left out; their runs are
//! drawn from outlines.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::ops::Range;
use std::panic::{AssertUnwindSafe, catch_unwind};

use subsetter::GlyphRemapper;

use super::PsFonts;
use crate::flatten::{FontBytes, Item, Paint, TextFont};

/// The longest `sfnts` string: PostScript strings hold at most 65535 bytes, and each string
/// has an even length.
const MAX_STRING: usize = 65534;

/// Glyphs a Type 42 font's one-byte codes address.
const CODES: usize = 256;

/// A page's fonts: the program that defines them, and how each source font's glyphs are shown.
#[derive(Default)]
pub(crate) struct FontSet {
    /// Defines the fonts, in the page's `save` and the prolog's dictionary.
    pub program: String,
    /// Font programs in `program`.
    pub count: usize,
    fonts: HashMap<(u128, u32), Placed>,
}

/// One source font as written.
pub(crate) struct Placed {
    /// The PostScript font names, by [`Placed::glyph`]'s index.
    names: Vec<String>,
    /// Source glyph id to font index and character code.
    glyphs: HashMap<u32, (usize, Vec<u8>)>,
}

impl Placed {
    /// The font index and character code that show source glyph `id`.
    pub fn glyph(&self, id: u32) -> Option<(usize, &[u8])> {
        self.glyphs.get(&id).map(|(i, code)| (*i, &code[..]))
    }

    pub fn name(&self, index: usize) -> &str {
        self.names.get(index).map_or("", String::as_str)
    }
}

impl FontSet {
    /// The TrueType fonts of the solid-painted text runs in `items`, for page `page` (from 0),
    /// written as `mode` says.
    pub fn new(items: &[Item], page: usize, mode: PsFonts) -> Self {
        let mut used: BTreeMap<(u128, u32), (FontBytes, BTreeSet<u16>, bool)> = BTreeMap::new();
        collect(items, &mut used);
        let mut set = FontSet::default();
        for (n, ((key, index), (data, glyphs, ok))) in used.into_iter().enumerate() {
            if !ok {
                continue;
            }
            let base = format!("CraftF{page}_{n}");
            let Some(sub) = subset((*data).as_ref(), index, &glyphs) else { continue };
            let Some(placed) = write(&mut set.program, &base, &sub, mode) else { continue };
            set.count += 1;
            set.fonts.insert((key, index), placed);
        }
        set
    }

    pub fn get(&self, font: &TextFont) -> Option<&Placed> {
        self.fonts.get(&(font.key, font.index))
    }
}

/// The glyphs each source font shows in a solid-painted run; `false` when a glyph id doesn't fit
/// in 16 bits.
fn collect(items: &[Item], used: &mut BTreeMap<(u128, u32), (FontBytes, BTreeSet<u16>, bool)>) {
    for item in items {
        match item {
            Item::Text(t) if matches!(t.paint, Paint::Solid(_)) => {
                let Some(font) = &t.font else { continue };
                let entry = used.entry((font.key, font.index)).or_insert_with(|| (font.data.clone(), BTreeSet::new(), true));
                for g in &t.glyphs {
                    match u16::try_from(g.id) {
                        Ok(id) => {
                            entry.1.insert(id);
                        }
                        Err(_) => entry.2 = false,
                    }
                }
            }
            Item::Group(g) => collect(&g.children, used),
            _ => {}
        }
    }
}

/// A TrueType subset of `glyphs` of font `data`.
struct Subset {
    data: Vec<u8>,
    /// Source glyph id to subset glyph id.
    map: Vec<(u16, u16)>,
    glyphs: u16,
    /// `[x0 y0 x1 y1]` per em.
    bbox: [f64; 4],
    /// Where `sfnts` strings may end.
    strings: Vec<Range<usize>>,
}

fn subset(data: &[u8], index: u32, glyphs: &BTreeSet<u16>) -> Option<Subset> {
    let face = ttf_parser::Face::parse(data, index).ok()?;
    face.tables().glyf?;
    let mut remapper = GlyphRemapper::new();
    for g in glyphs {
        remapper.remap(*g);
    }
    // Font bytes come from the PDF: a font the subsetter trips on is drawn from outlines.
    let sub = catch_unwind(AssertUnwindSafe(|| subsetter::subset(data, index, &remapper))).ok()?.ok()?;
    let face = ttf_parser::Face::parse(&sub, 0).ok()?;
    let upem = f64::from(face.units_per_em());
    let r = face.global_bounding_box();
    let bbox = [f64::from(r.x_min) / upem, f64::from(r.y_min) / upem, f64::from(r.x_max) / upem, f64::from(r.y_max) / upem];
    let strings = sfnts(&sub)?;
    let map = glyphs.iter().filter_map(|g| Some((*g, remapper.get(*g)?))).collect();
    Some(Subset { glyphs: remapper.num_gids(), data: sub, map, bbox, strings })
}

/// The ranges of `font` the `sfnts` strings hold: each at most [`MAX_STRING`] bytes, of even
/// length, ending at a table boundary or, inside `glyf`, at a glyph boundary. `None` when it
/// can't be split so.
fn sfnts(font: &[u8]) -> Option<Vec<Range<usize>>> {
    let be16 = |at: usize| font.get(at..at + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
    let be32 = |at: usize| font.get(at..at + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]));
    let tables = usize::from(be16(4)?);
    let mut ends = BTreeSet::new();
    let mut find = HashMap::new();
    for i in 0..tables {
        let at = 12 + 16 * i;
        let tag = font.get(at..at + 4)?;
        let (offset, len) = (usize::try_from(be32(at + 8)?).ok()?, usize::try_from(be32(at + 12)?).ok()?);
        if offset.checked_add(len)? > font.len() {
            return None;
        }
        ends.insert(offset);
        find.insert(tag.to_vec(), (offset, len));
    }
    if let (Some(&(glyf, _)), Some(&(loca, loca_len)), Some(&(head, _))) = (find.get(&b"glyf"[..]), find.get(&b"loca"[..]), find.get(&b"head"[..])) {
        let long = be16(head + 50)? == 1;
        let (step, count) = if long { (4, loca_len / 4) } else { (2, loca_len / 2) };
        for i in 0..count {
            let at = loca + i * step;
            let v = if long { usize::try_from(be32(at)?).ok()? } else { usize::from(be16(at)?) * 2 };
            ends.insert(glyf + v);
        }
    }
    ends.insert(font.len());
    let mut strings = Vec::new();
    let (mut start, mut last) = (0, 0);
    for &end in ends.iter().filter(|e| **e > 0 && **e <= font.len()) {
        if end - start > MAX_STRING {
            if last == start {
                return None;
            }
            strings.push(start..last);
            start = last;
            if end - start > MAX_STRING {
                return None;
            }
        }
        last = end;
    }
    if start < font.len() {
        strings.push(start..font.len());
    }
    // Every string but the last ends at a boundary; an odd one can't be a string.
    let inner = strings.len().saturating_sub(1);
    strings.iter().take(inner).all(|r| r.len() % 2 == 0).then_some(strings)
}

/// Appends the definitions of `sub` named after `base` and returns how its glyphs are shown.
fn write(program: &mut String, base: &str, sub: &Subset, mode: PsFonts) -> Option<Placed> {
    let mut sfnts = String::from("[\n");
    for r in &sub.strings {
        let bytes = sub.data.get(r.clone())?;
        sfnts.push('<');
        for (i, b) in bytes.iter().enumerate() {
            if i % 40 == 0 {
                sfnts.push('\n');
            }
            let _ = write!(sfnts, "{b:02x}");
        }
        if bytes.len() % 2 == 1 {
            sfnts.push_str("00");
        }
        sfnts.push_str("\n>\n");
    }
    sfnts.push(']');
    let [x0, y0, x1, y1] = sub.bbox.map(num);
    let bbox = format!("[{x0} {y0} {x1} {y1}]");
    let notdefs = "256 array 0 1 255 {1 index exch /.notdef put} for";
    match mode {
        PsFonts::TrueType42 => {
            let fonts = usize::from(sub.glyphs).div_ceil(CODES);
            let _ = writeln!(program, "%%BeginResource: font {base}_0\n/{base}_sfnts {sfnts} def");
            let _ = write!(program, "/{base}_cs {} dict dup begin /.notdef 0 def", usize::from(sub.glyphs) + 1);
            for gid in 1..sub.glyphs {
                let _ = write!(program, "{}/g{gid} {gid} def", if gid % 16 == 0 { "\n" } else { " " });
            }
            program.push_str(" end def\n");
            for k in 0..fonts {
                let _ = writeln!(
                    program,
                    "12 dict begin /FontType 42 def /FontName /{base}_{k} def /PaintType 0 def /FontMatrix [1 0 0 1 0 0] def\n/FontBBox {bbox} def /Encoding {notdefs} def"
                );
                for code in 0..CODES {
                    let gid = k * CODES + code;
                    if gid > 0 && gid < usize::from(sub.glyphs) {
                        let _ = writeln!(program, "Encoding {code} /g{gid} put");
                    }
                }
                let _ = writeln!(program, "/CharStrings {base}_cs def /sfnts {base}_sfnts def\nFontName currentdict end definefont pop");
            }
            program.push_str("%%EndResource\n");
            let names = (0..fonts).map(|k| format!("{base}_{k}")).collect();
            let glyphs = sub
                .map
                .iter()
                .filter_map(|&(old, new)| {
                    let new = usize::from(new);
                    Some((u32::from(old), (new / CODES, vec![u8::try_from(new % CODES).ok()?])))
                })
                .collect();
            Some(Placed { names, glyphs })
        }
        PsFonts::Cid => {
            let _ = writeln!(
                program,
                "%%BeginResource: CIDFont {base}\n20 dict begin /CIDFontName /{base} def /CIDFontType 2 def /FontType 42 def /PaintType 0 def
/CIDSystemInfo 3 dict dup begin /Registry (Adobe) def /Ordering (Identity) def /Supplement 0 def end def
/FontMatrix [1 0 0 1 0 0] def /FontBBox {bbox} def /CIDCount {} def /GDBytes 2 def /CIDMap 0 def
/CharStrings 1 dict dup begin /.notdef 0 def end def /Encoding {notdefs} def
/sfnts {sfnts} def
CIDFontName currentdict end /CIDFont defineresource pop
%%EndResource
/{base}_0 /Identity-H [/{base} /CIDFont findresource] composefont pop",
                sub.glyphs
            );
            let glyphs = sub.map.iter().map(|&(old, new)| (u32::from(old), (0, new.to_be_bytes().to_vec()))).collect();
            Some(Placed { names: vec![format!("{base}_0")], glyphs })
        }
    }
}

fn num(v: f64) -> String {
    craft_ps::write::num(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An sfnt directory of `tables` (tag, length), laid out one after another.
    fn sfnt(tables: &[(&[u8; 4], usize)]) -> Vec<u8> {
        let mut out = vec![0, 1, 0, 0];
        out.extend((tables.len() as u16).to_be_bytes());
        out.extend([0; 6]);
        let mut offset = 12 + 16 * tables.len();
        for (tag, len) in tables {
            out.extend(*tag);
            out.extend([0; 4]);
            out.extend((offset as u32).to_be_bytes());
            out.extend((*len as u32).to_be_bytes());
            offset += len.next_multiple_of(4);
        }
        for (_, len) in tables {
            out.resize(out.len() + len.next_multiple_of(4), 0);
        }
        out
    }

    #[test]
    fn sfnts_strings_end_at_table_boundaries_and_stay_short() {
        let font = sfnt(&[(b"cvt ", 40_000), (b"hmtx", 40_000), (b"name", 100)]);
        let strings = sfnts(&font).unwrap();
        assert!(strings.len() >= 2);
        assert!(strings.iter().all(|r| r.len() <= MAX_STRING && r.len() % 2 == 0));
        assert_eq!(strings.first().map(|r| r.start), Some(0));
        assert_eq!(strings.last().map(|r| r.end), Some(font.len()));
        assert!(strings.windows(2).all(|w| w[0].end == w[1].start));
        // A table past the limit with no glyph boundaries inside can't be split.
        assert!(sfnts(&sfnt(&[(b"cvt ", 70_000)])).is_none());
    }
}
