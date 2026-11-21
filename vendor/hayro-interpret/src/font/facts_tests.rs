//! PdfCraft patch (107): tests of the font facts ([`crate::font::Glyph::with_facts`]), the
//! per-object font keys ([`crate::InterpreterSettings::font_per_object`]) and the probe of a
//! CID font read without building it ([`crate::font::CidFontProbe`]), on synthetic documents
//! whose font programs are built here.

use crate::font::{
    CidFontProbe, CidProbeCache, FontFlags, FontKind, Glyph, built_cid_font_numbers,
    font_object_key,
};
use crate::{BlendMode, interpret_page};
use crate::{
    ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint,
    PathDrawMode, SoftMask,
};
use hayro_cmap::BfString;
use hayro_syntax::Pdf;
use hayro_syntax::object::Dict;
use kurbo::{Affine, BezPath, Rect};
use skrifa::GlyphId;
use skrifa::raw::TableProvider;

fn be16(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(&n.to_be_bytes());
}

fn be32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}

/// A TrueType program with `num_glyphs` glyphs, each a small square, and a symbolic (3, 0)
/// `cmap` mapping `0xF000 + code` to the glyph `pairs` gives it (none when `pairs` is empty).
fn truetype(num_glyphs: u16, pairs: &[(u8, u16)]) -> Vec<u8> {
    let mut square = Vec::new();
    for n in [1i16, 0, 0, 500, 500] {
        square.extend_from_slice(&n.to_be_bytes());
    }
    be16(&mut square, 3);
    be16(&mut square, 0);
    square.extend([1u8; 4]);
    for d in [0i16, 500, 0, -500, 0, 0, 500, 0] {
        square.extend_from_slice(&d.to_be_bytes());
    }
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for _ in 0..num_glyphs {
        be32(&mut loca, u32::try_from(glyf.len()).unwrap());
        glyf.extend_from_slice(&square);
    }
    be32(&mut loca, u32::try_from(glyf.len()).unwrap());
    let mut head = vec![0; 54];
    head[0..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&0x5f0f_3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&1i16.to_be_bytes());
    let mut hhea = vec![0; 36];
    hhea[0..4].copy_from_slice(&[0, 1, 0, 0]);
    hhea[34..36].copy_from_slice(&1u16.to_be_bytes());
    let mut hmtx = Vec::new();
    be16(&mut hmtx, 600);
    be16(&mut hmtx, 0);
    for _ in 1..num_glyphs {
        be16(&mut hmtx, 0);
    }
    let mut maxp = vec![0, 1, 0, 0];
    be16(&mut maxp, num_glyphs);
    maxp.resize(32, 0);
    let mut tables: Vec<([u8; 4], Vec<u8>)> = Vec::new();
    if !pairs.is_empty() {
        // One format 4 segment per pair, then the closing 0xFFFF segment.
        let mut sub = Vec::new();
        let segments = pairs.len() + 1;
        be16(&mut sub, 4);
        be16(&mut sub, u16::try_from(16 + segments * 8).unwrap());
        be16(&mut sub, 0);
        be16(&mut sub, u16::try_from(segments * 2).unwrap());
        for n in [2, 0, 0] {
            be16(&mut sub, n);
        }
        for &(c, _) in pairs {
            be16(&mut sub, 0xF000 | u16::from(c));
        }
        be16(&mut sub, 0xFFFF);
        be16(&mut sub, 0);
        for &(c, _) in pairs {
            be16(&mut sub, 0xF000 | u16::from(c));
        }
        be16(&mut sub, 0xFFFF);
        for &(c, g) in pairs {
            be16(&mut sub, g.wrapping_sub(0xF000 | u16::from(c)));
        }
        be16(&mut sub, 1);
        for _ in 0..segments {
            be16(&mut sub, 0);
        }
        let mut cmap = Vec::new();
        for n in [0, 1, 3, 0] {
            be16(&mut cmap, n);
        }
        be32(&mut cmap, 12);
        cmap.extend(sub);
        tables.push((*b"cmap", cmap));
    }
    tables.push((*b"glyf", glyf));
    tables.push((*b"head", head));
    tables.push((*b"hhea", hhea));
    tables.push((*b"hmtx", hmtx));
    tables.push((*b"loca", loca));
    tables.push((*b"maxp", maxp));
    let count = u16::try_from(tables.len()).unwrap();
    let mut out = vec![0, 1, 0, 0];
    be16(&mut out, count);
    for n in [128u16, 3, (count * 16).saturating_sub(128)] {
        be16(&mut out, n);
    }
    let mut payload = Vec::new();
    let mut offset = 12 + 16 * tables.len();
    for (tag, bytes) in tables {
        out.extend(tag);
        let mut padded = bytes.clone();
        padded.resize(bytes.len().div_ceil(4) * 4, 0);
        be32(&mut out, 0);
        be32(&mut out, u32::try_from(offset).unwrap());
        be32(&mut out, u32::try_from(bytes.len()).unwrap());
        offset += padded.len();
        payload.extend(padded);
    }
    out.extend(payload);
    out
}

/// A CFF INDEX of `items`, offsets two bytes wide.
fn cff_index(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = u16::try_from(items.len()).unwrap().to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    out.push(2);
    let mut at = 1usize;
    out.extend_from_slice(&u16::try_from(at).unwrap().to_be_bytes());
    for item in items {
        at += item.len();
        out.extend_from_slice(&u16::try_from(at).unwrap().to_be_bytes());
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

/// An integer operand in its five-byte form, so offsets don't change a DICT's size.
fn cff_int(v: usize) -> Vec<u8> {
    let mut b = vec![29];
    b.extend_from_slice(&i32::try_from(v).unwrap().to_be_bytes());
    b
}

/// A CID-keyed CFF program (Adobe Technical Note #5176 §18) with glyph 0 and one empty glyph
/// for each of `cids`, in order, as its charset gives them.
fn cid_keyed_cff(cids: &[u16]) -> Vec<u8> {
    let n = cids.len() + 1;
    let cid_count = cids.iter().max().map_or(1, |&m| usize::from(m) + 1);
    let header = vec![1, 0, 4, 1];
    let name = cff_index(&[b"Synthetic".to_vec()]);
    let strings = cff_index(&[b"Adobe".to_vec(), b"Identity".to_vec()]);
    let global_subrs = cff_index(&[]);
    let charset: Vec<u8> = std::iter::once(0u8)
        .chain(cids.iter().flat_map(|c| c.to_be_bytes()))
        .collect();
    let fd_select: Vec<u8> = std::iter::once(0u8)
        .chain(std::iter::repeat_n(0u8, n))
        .collect();
    let charstrings = cff_index(&vec![vec![14u8]; n]);
    let private = vec![139, 20];
    let font_dict = |at: usize| [cff_int(private.len()), cff_int(at), vec![18]].concat();
    let fd_array_len = cff_index(&[font_dict(0)]).len();
    let top = |charset_at: usize, fd_select_at: usize, charstrings_at: usize, fd_at: usize| {
        [
            cff_int(391),
            cff_int(392),
            cff_int(0),
            vec![12, 30],
            cff_int(cid_count),
            vec![12, 34],
            cff_int(charset_at),
            vec![15],
            cff_int(charstrings_at),
            vec![17],
            cff_int(fd_at),
            vec![12, 36],
            cff_int(fd_select_at),
            vec![12, 37],
        ]
        .concat()
    };
    let top_len = cff_index(&[top(0, 0, 0, 0)]).len();
    let charset_at = header.len() + name.len() + top_len + strings.len() + global_subrs.len();
    let fd_select_at = charset_at + charset.len();
    let charstrings_at = fd_select_at + fd_select.len();
    let fd_array_at = charstrings_at + charstrings.len();
    let private_at = fd_array_at + fd_array_len;
    let top_index = cff_index(&[top(charset_at, fd_select_at, charstrings_at, fd_array_at)]);
    let fd_array = cff_index(&[font_dict(private_at)]);
    [
        header,
        name,
        top_index,
        strings,
        global_subrs,
        charset,
        fd_select,
        charstrings,
        fd_array,
        private,
    ]
    .concat()
}

/// A stream object's body holding `data` as it is, with `extra` in its dictionary.
fn stream(data: &[u8], extra: &str) -> Vec<u8> {
    let mut out = format!("<< /Length {} {extra} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A one-page document whose page draws `content` with `fonts` (its `/Font` resources, as
/// written) and holds `objects` from object 5 on.
fn document(fonts: &str, content: &str, objects: &[Vec<u8>]) -> Pdf {
    let mut bodies: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Contents 4 0 R \
             /Resources << /Font << {fonts} >> >> >>"
        )
        .into_bytes(),
        stream(content.as_bytes(), ""),
    ];
    bodies.extend(objects.iter().cloned());
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in bodies.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", bodies.len() + 1).as_bytes(),
    );
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            bodies.len() + 1
        )
        .as_bytes(),
    );
    Pdf::new(out).unwrap()
}

/// What a test reads of one drawn glyph.
#[derive(Clone, Debug, PartialEq)]
struct Seen {
    strict: Option<BfString>,
    key: u128,
    cache_key: u128,
    kind: FontKind,
    /// What `with_facts` gives, flattened.
    facts: Facts,
    /// The fill colour's components, as the content set them.
    fill: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
struct Facts {
    kind: FontKind,
    base_font: Option<String>,
    flags: Option<FontFlags>,
    font_weight: Option<f32>,
    italic_angle: Option<f32>,
    has_to_unicode: bool,
    has_encoding: bool,
    /// The names of codes 65 to 67.
    names: Option<Vec<Option<String>>>,
    program_glyphs_in_program: Option<u16>,
    /// The glyph and strict text of codes 65 to 67.
    code_glyph: Option<Vec<(GlyphId, Option<BfString>)>>,
    to_unicode_checkable: bool,
    ids: Option<Vec<u32>>,
    /// The widths of ids 0 to 3 and 65.
    id_width: Option<Vec<Option<f32>>>,
    /// The text of ids 0 to 3.
    id_text: Option<Vec<Option<BfString>>>,
    identity_encoding: bool,
    program_glyphs: Option<u32>,
    substitute: bool,
    /// The `/ToUnicode` text, the encoding's text, whether an outline is drawn, and the
    /// `/Widths` width of codes 65 to 67.
    to_unicode_text: Option<Vec<Option<BfString>>>,
    encoding_text: Option<Vec<Option<BfString>>>,
    code_outline: Option<Vec<bool>>,
    code_width: Option<Vec<Option<f32>>>,
    gid: GlyphId,
    name: Option<String>,
    id: Option<u32>,
    code: Option<u8>,
}

#[derive(Default)]
struct Recorder(Vec<Seen>);

impl<'a> Device<'a> for Recorder {
    fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
    fn set_blend_mode(&mut self, _: BlendMode) {}
    fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph(
        &mut self,
        glyph: &Glyph<'a>,
        _: Affine,
        _: Affine,
        paint: &Paint<'a>,
        _: &GlyphDrawMode,
    ) {
        let facts = glyph.with_facts(|font, g| Facts {
            kind: font.kind,
            base_font: font.base_font.map(str::to_owned),
            flags: font.flags.copied(),
            font_weight: font.font_weight,
            italic_angle: font.italic_angle,
            has_to_unicode: font.has_to_unicode,
            has_encoding: font.has_encoding,
            names: font
                .names
                .map(|names| (65..=67).map(|c| names(c).map(str::to_owned)).collect()),
            program_glyphs_in_program: font
                .program
                .map(|p| p.maxp().map(|m| m.num_glyphs()).unwrap_or(0)),
            code_glyph: font.code_glyph.map(|f| (65..=67).map(f).collect()),
            to_unicode_checkable: font.to_unicode_checkable,
            ids: font.ids.map(|ids| ids()),
            id_width: font
                .id_width
                .map(|w| [0, 1, 2, 3, 65].into_iter().map(w).collect()),
            id_text: font.id_text.map(|t| (0..=3).map(t).collect()),
            identity_encoding: font.identity_encoding,
            program_glyphs: font.program_glyphs,
            substitute: font.substitute,
            to_unicode_text: font.to_unicode_text.map(|t| (65..=67).map(t).collect()),
            encoding_text: font.encoding_text.map(|t| (65..=67).map(t).collect()),
            code_outline: font
                .code_outline
                .map(|o| (65..=67).map(|c| o(c).is_some_and(|p| !p.elements().is_empty())).collect()),
            code_width: font.code_width.map(|w| (65..=67).map(w).collect()),
            gid: g.gid,
            name: g.name.map(str::to_owned),
            id: g.id,
            code: g.code,
        });
        let fill = match paint {
            Paint::Color(c) => c.components().to_vec(),
            Paint::Pattern(_) => Vec::new(),
        };
        self.0.push(Seen {
            strict: glyph.as_unicode(),
            key: glyph.font_key(),
            cache_key: glyph.font_cache_key(),
            kind: glyph.font_kind(),
            facts,
            fill,
        });
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
    fn pop_clip_path(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The glyphs page 1 of `pdf` draws, read with `settings`.
fn glyphs_with(pdf: &Pdf, settings: InterpreterSettings) -> Vec<Seen> {
    let pages = pdf.pages();
    let page = pages.first().unwrap();
    let cache = InterpreterCache::new();
    let bbox = Rect::new(0.0, 0.0, 200.0, 200.0);
    let mut ctx = Context::new(Affine::IDENTITY, bbox, &cache, page.xref(), settings);
    let mut device = Recorder::default();
    interpret_page(page, &mut ctx, &mut device);
    device.0
}

fn glyphs(pdf: &Pdf) -> Vec<Seen> {
    glyphs_with(pdf, InterpreterSettings::default())
}

/// A Type 3 font whose codes 65 and 66 have glyph procedures (`/a`, `/b`) and 67 names a glyph
/// without one, with `extra` in its dictionary.
fn type3_font(extra: &str) -> Vec<u8> {
    format!(
        "<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1000 1000] \
         /FontMatrix [0.001 0 0 0.001 0 0] /CharProcs << /a 7 0 R /b 7 0 R >> \
         /Encoding << /Type /Encoding /Differences [65 /a /b /c] >> /FirstChar 65 \
         /LastChar 67 /Widths [500 600 700] {extra} >>"
    )
    .into_bytes()
}

fn glyph_procedure() -> Vec<u8> {
    stream(b"500 0 d0 0 0 500 500 re f", "")
}

/// Two font objects with one dictionary are one font to draw: they share a key unless the
/// interpreter keeps a font per object, when each has its own (a reader deciding per object by
/// the glyphs each shows needs that). The cache key is the dictionary's either way.
#[test]
fn fonts_with_one_dictionary_are_one_font_unless_kept_per_object() {
    let pdf = document(
        "/F1 5 0 R /F2 6 0 R",
        "BT /F1 10 Tf (A) Tj /F2 10 Tf (A) Tj ET",
        &[type3_font(""), type3_font(""), glyph_procedure()],
    );
    let shared = glyphs(&pdf);
    assert_eq!(shared.len(), 2);
    assert_eq!(shared[0].key, shared[1].key, "one font to draw");
    let apart = glyphs_with(
        &pdf,
        InterpreterSettings {
            font_per_object: true,
            ..InterpreterSettings::default()
        },
    );
    assert_eq!(apart.len(), 2);
    assert_ne!(apart[0].key, apart[1].key, "a font per object");
    assert_eq!(apart[0].cache_key, apart[1].cache_key);
    assert_eq!(apart[0].cache_key, shared[0].cache_key);
    assert_eq!(apart[0].strict, shared[0].strict, "the text is the same");
    // A reader that finds the fonts in the resources keys them as their glyphs are keyed.
    let mut keys: Vec<u128> = page_fonts(&pdf).iter().map(font_object_key).collect();
    let mut drawn: Vec<u128> = apart.iter().map(|g| g.key).collect();
    keys.sort_unstable();
    drawn.sort_unstable();
    assert_eq!(keys, drawn);
}

/// A Type 3 font's facts: the codes with a glyph procedure, their `/Widths`, the encoding's
/// names, the code as the glyph's id, and its descriptor.
#[test]
fn a_type3_fonts_facts_give_its_procedures_widths_and_names() {
    let pdf = document(
        "/F1 5 0 R",
        "BT /F1 10 Tf 0.25 0.5 0.75 rg (AB) Tj ET",
        &[
            type3_font("/Name /Marks /FontDescriptor 6 0 R"),
            b"<< /Type /FontDescriptor /Flags 262144.0 /FontWeight 700 /ItalicAngle -12 >>"
                .to_vec(),
            glyph_procedure(),
        ],
    );
    let seen = glyphs(&pdf);
    assert_eq!(seen.len(), 2);
    let a = &seen[0].facts;
    assert_eq!(seen[0].kind, FontKind::Type3);
    assert_eq!(a.kind, FontKind::Type3);
    assert_eq!(a.base_font, None, "a Type 3 font's /Name is no /BaseFont");
    assert_eq!(
        a.flags,
        Some(FontFlags::FORCE_BOLD),
        "a real /Flags is truncated"
    );
    assert_eq!((a.font_weight, a.italic_angle), (Some(700.0), Some(-12.0)));
    assert!(a.has_encoding && !a.has_to_unicode);
    let names = |n: &[&str]| Some(n.iter().map(|s| Some((*s).to_owned())).collect());
    assert_eq!(a.names, names(&["a", "b", "c"]));
    assert_eq!(a.ids, Some(vec![65, 66]), "code 67 has no procedure");
    assert_eq!(a.id_width, Some(vec![None, None, None, None, Some(500.0)]));
    assert_eq!(
        (a.gid, a.name.as_deref(), a.id),
        (GlyphId::NOTDEF, Some("a"), Some(65))
    );
    assert_eq!(seen[1].facts.id, Some(66));
    assert_eq!(
        seen[0].fill,
        vec![0.25, 0.5, 0.75],
        "the operands as the content set them"
    );
}

/// A simple Type 1 font's facts: the encoding's names with its `/Differences`, `/BaseFont` as
/// written and the descriptor.
#[test]
fn a_type1_fonts_facts_give_its_encoding_names() {
    let pdf = document(
        "/F1 5 0 R",
        "BT /F1 10 Tf (AB) Tj ET",
        &[
            b"<< /Type /Font /Subtype /Type1 /BaseFont /ABCDEF+Helvetica \
               /Encoding << /Differences [66 /C83] >> /FontDescriptor 6 0 R >>"
                .to_vec(),
            b"<< /Type /FontDescriptor /Flags 4 /ItalicAngle 0 >>".to_vec(),
        ],
    );
    let seen = glyphs(&pdf);
    assert_eq!(seen.len(), 2);
    let a = &seen[0].facts;
    assert_eq!(a.kind, FontKind::Type1);
    assert_eq!(a.base_font.as_deref(), Some("ABCDEF+Helvetica"));
    assert_eq!(a.flags, Some(FontFlags::SYMBOLIC));
    assert!(a.has_encoding && !a.has_to_unicode);
    assert_eq!(
        a.names,
        Some(vec![
            Some("A".to_owned()),
            Some("C83".to_owned()),
            Some("C".to_owned())
        ])
    );
    assert_eq!(a.name.as_deref(), Some("A"));
    assert_eq!(seen[1].facts.name.as_deref(), Some("C83"));
    assert_eq!(
        (a.program_glyphs_in_program, a.ids.as_ref(), a.id),
        (None, None, None)
    );
}

/// A simple TrueType font's facts: its program, and the glyph and strict text of each code.
#[test]
fn a_truetype_fonts_facts_give_its_program_and_code_glyphs() {
    let program = truetype(5, &[(65, 3), (66, 4)]);
    let pdf = document(
        "/F1 5 0 R",
        "BT /F1 10 Tf (AB) Tj ET",
        &[
            b"<< /Type /Font /Subtype /TrueType /BaseFont /Shapes /FirstChar 65 /LastChar 66 \
               /Widths [600 600] /FontDescriptor 6 0 R >>"
                .to_vec(),
            b"<< /Type /FontDescriptor /Flags 4 /FontFile2 7 0 R >>".to_vec(),
            stream(&program, ""),
        ],
    );
    let seen = glyphs(&pdf);
    assert_eq!(seen.len(), 2);
    let a = &seen[0].facts;
    assert_eq!(a.kind, FontKind::TrueType);
    assert_eq!(a.program_glyphs_in_program, Some(5));
    assert!(!a.has_encoding);
    let code_glyph = a.code_glyph.as_ref().unwrap();
    assert_eq!(code_glyph[0].0, GlyphId::new(3));
    assert_eq!(code_glyph[1].0, GlyphId::new(4));
    assert_eq!(
        (a.gid, seen[1].facts.gid),
        (GlyphId::new(3), GlyphId::new(4))
    );
}

/// A simple font's facts give each code's `/ToUnicode` text and its text by the encoding apart
/// (the strict text is the first, else the second), the outline of the glyph it draws from an
/// embedded program, the width `/Widths` gives it, and the glyph's code. A standard font has no
/// program to draw from and no explicit widths; a Type 3 or Type 0 glyph has no code here.
#[test]
fn a_simple_fonts_facts_give_its_codes_text_outlines_and_widths() {
    let program = truetype(5, &[(65, 3), (66, 4)]);
    let to_unicode = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap /CMapName /T def \
        1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <41> <0058> endbfchar \
        endcmap CMapName currentdict /CMap defineresource pop end end";
    let pdf = document(
        "/F1 5 0 R /F2 8 0 R /F3 9 0 R",
        "BT /F1 10 Tf (AB) Tj /F2 10 Tf (A) Tj /F3 10 Tf (A) Tj ET",
        &[
            b"<< /Type /Font /Subtype /TrueType /BaseFont /Shapes /FirstChar 65 /LastChar 66 \
               /Widths [600 700] /FontDescriptor 6 0 R /ToUnicode 10 0 R >>"
                .to_vec(),
            b"<< /Type /FontDescriptor /Flags 4 /FontFile2 7 0 R >>".to_vec(),
            stream(&program, ""),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            type3_font(""),
            stream(to_unicode, ""),
            glyph_procedure(),
        ],
    );
    let seen = glyphs(&pdf);
    assert_eq!(seen.len(), 4);
    let (a, b) = (&seen[0], &seen[1]);
    assert_eq!((&a.strict, &b.strict), (&Some(BfString::Char('X')), &Some(BfString::Char('B'))));
    let x = |c| Some(BfString::Char(c));
    assert_eq!(a.facts.to_unicode_text, Some(vec![x('X'), None, None]));
    assert_eq!(a.facts.encoding_text, Some(vec![x('A'), x('B'), x('C')]));
    assert_eq!(a.facts.code_outline, Some(vec![true, true, false]), "code 67 draws .notdef");
    assert_eq!(a.facts.code_width, Some(vec![Some(600.0), Some(700.0), None]));
    assert_eq!((a.facts.code, b.facts.code), (Some(65), Some(66)));
    let helvetica = &seen[2].facts;
    assert_eq!(helvetica.kind, FontKind::Type1);
    assert_eq!(helvetica.to_unicode_text, Some(vec![None, None, None]));
    assert_eq!(helvetica.encoding_text, Some(vec![x('A'), x('B'), x('C')]));
    assert_eq!(helvetica.code_outline, None, "a standard font has no program");
    assert_eq!(helvetica.code_width, Some(vec![None, None, None]));
    assert_eq!(helvetica.code, Some(65));
    let type3 = &seen[3].facts;
    assert_eq!(type3.kind, FontKind::Type3);
    assert_eq!((type3.code, type3.encoding_text.is_some()), (None, false));
}

/// A Type 0 font over an identity CMap and `/CIDToGIDMap` with a TrueType program: its codes
/// are CIDs and CIDs glyph ids, so the facts give the text of each glyph id and the widths by
/// CID, and a `/ToUnicode` stream can be checked against them.
#[test]
fn an_identity_cid_truetype_fonts_facts_give_text_and_widths_by_glyph_id() {
    let program = truetype(4, &[]);
    let to_unicode = b"/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
        1 begincodespacerange <0000> <FFFF> endcodespacerange \
        2 beginbfchar <0001> <0041> <0002> <0042> endbfchar endcmap end end";
    let pdf = document(
        "/F1 5 0 R",
        "BT /F1 10 Tf <00010002> Tj ET",
        &[
            b"<< /Type /Font /Subtype /Type0 /BaseFont /ABCDEF+Shapes /Encoding /Identity-H \
               /DescendantFonts [6 0 R] /ToUnicode 9 0 R >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /ABCDEF+Shapes \
               /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
               /CIDToGIDMap /Identity /W [1 [700 800]] /FontDescriptor 7 0 R >>"
                .to_vec(),
            b"<< /Type /FontDescriptor /Flags 4 /FontFile2 8 0 R >>".to_vec(),
            stream(&program, ""),
            stream(to_unicode, ""),
        ],
    );
    let seen = glyphs(&pdf);
    assert_eq!(seen.len(), 2);
    let a = &seen[0].facts;
    assert_eq!(
        (seen[0].kind, a.kind),
        (FontKind::CidTrueType, FontKind::CidTrueType)
    );
    assert!(a.has_to_unicode && a.to_unicode_checkable && a.identity_encoding);
    assert!(!a.has_encoding && !a.substitute);
    assert_eq!(
        (a.program_glyphs, a.program_glyphs_in_program),
        (Some(4), Some(4))
    );
    assert_eq!(
        a.id_text,
        Some(vec![
            None,
            Some(BfString::Char('A')),
            Some(BfString::Char('B')),
            None
        ])
    );
    assert_eq!(
        a.id_width,
        Some(vec![None, Some(700.0), Some(800.0), None, None])
    );
    assert_eq!((a.gid, a.id), (GlyphId::new(1), Some(1)));
    assert_eq!(
        (seen[1].facts.gid, seen[1].facts.id),
        (GlyphId::new(2), Some(2))
    );
}

/// A Type 0 font over a CID-keyed CFF program: the facts give its charset's CIDs, in glyph
/// order, and each glyph's CID.
#[test]
fn a_cid_cff_fonts_facts_give_its_charset() {
    let program = cid_keyed_cff(&[34, 66, 7]);
    let pdf = document(
        "/F1 5 0 R",
        "BT /F1 10 Tf <00220042> Tj ET",
        &[
            b"<< /Type /Font /Subtype /Type0 /BaseFont /Synthetic /Encoding /Identity-H \
               /DescendantFonts [6 0 R] >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Synthetic \
               /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
               /W [34 [667] 66 [556]] /FontDescriptor 7 0 R >>"
                .to_vec(),
            b"<< /Type /FontDescriptor /Flags 4 /FontFile3 8 0 R >>".to_vec(),
            stream(&program, "/Subtype /CIDFontType0C"),
        ],
    );
    let seen = glyphs(&pdf);
    assert_eq!(seen.len(), 2);
    let a = &seen[0].facts;
    assert_eq!(a.kind, FontKind::CidCff);
    assert_eq!(a.ids, Some(vec![0, 34, 66, 7]), "glyph 0 is CID 0");
    assert_eq!(a.program_glyphs, Some(4));
    assert!(a.program_glyphs_in_program.is_none() && !a.has_to_unicode && !a.to_unicode_checkable);
    assert_eq!(a.id_width.as_ref().unwrap()[4], None, "CID 65 has no width");
    assert_eq!((a.id, seen[1].facts.id), (Some(34), Some(66)));
}

/// A Type 0 font without a program draws with a substitute, whose glyph ids are its own: no
/// program is given, and the font is of no kind a reader reads by number.
#[test]
fn a_substitute_cid_fonts_facts_give_no_program() {
    let pdf = document(
        "/F1 5 0 R",
        "BT /F1 10 Tf <0022> Tj ET",
        &[
            b"<< /Type /Font /Subtype /Type0 /BaseFont /Missing /Encoding /Identity-H \
               /DescendantFonts [6 0 R] >>"
                .to_vec(),
            b"<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Missing \
               /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
               /FontDescriptor 7 0 R >>"
                .to_vec(),
            b"<< /Type /FontDescriptor /Flags 4 >>".to_vec(),
        ],
    );
    let seen = glyphs(&pdf);
    assert_eq!(seen.len(), 1);
    let a = &seen[0].facts;
    assert_eq!(a.kind, FontKind::CidOther);
    assert!(a.substitute);
    assert_eq!(
        (
            a.program_glyphs_in_program,
            a.program_glyphs,
            a.ids.as_ref()
        ),
        (None, None, None)
    );
}

/// The font dictionaries of page 1, in object order.
fn page_fonts(pdf: &Pdf) -> Vec<Dict<'_>> {
    let pages = pdf.pages();
    let page = pages.first().unwrap();
    let fonts = &page.resources().fonts;
    let mut out = Vec::new();
    for name in fonts.keys() {
        out.push(fonts.get::<Dict<'_>>(name.as_ref()).unwrap());
    }
    out.sort_by_key(|d| d.obj_id().map(|id| id.obj_number));
    out
}

/// A probe reads each CID font as the font built from it draws: the kind, the glyph count and
/// the number of the glyph each code draws, through an identity `/CIDToGIDMap`, a map stream
/// (a CID it leaves out has no number) and a CFF charset. Fonts it doesn't read by number (an
/// Adobe collection, another CMap, a substitute) are `None` both ways, and a program shared by
/// two fonts is read once.
#[test]
fn a_probe_reads_cid_fonts_as_the_built_font_does() {
    let tt = truetype(6, &[]);
    let cff = cid_keyed_cff(&[34, 66, 7]);
    // CID 0 → 0, 1 → 5, 2 → 3; CIDs from 3 are past the map.
    let map: Vec<u8> = [0u16, 5, 3].iter().flat_map(|g| g.to_be_bytes()).collect();
    let descendant = |subtype: &str, ordering: &str, extra: &str, descriptor: u32| {
        format!(
            "<< /Type /Font /Subtype /{subtype} /BaseFont /F \
             /CIDSystemInfo << /Registry (Adobe) /Ordering ({ordering}) /Supplement 0 >> \
             {extra} /FontDescriptor {descriptor} 0 R >>"
        )
        .into_bytes()
    };
    let type0 = |encoding: &str, descendant: u32| {
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /F /Encoding /{encoding} \
             /DescendantFonts [{descendant} 0 R] >>"
        )
        .into_bytes()
    };
    let objects = vec![
        // 5–9: the fonts.
        type0("Identity-H", 10),
        type0("Identity-V", 11),
        type0("Identity-H", 12),
        type0("Identity-H", 13),
        type0("Identity-H", 14),
        // 10: TrueType, identity map. 11: the same program through a map stream.
        descendant("CIDFontType2", "Identity", "/CIDToGIDMap /Identity", 15),
        descendant("CIDFontType2", "Identity", "/CIDToGIDMap 18 0 R", 15),
        // 12: CFF by CID. 13: an Adobe collection. 14: a substitute.
        descendant("CIDFontType0", "Identity", "", 16),
        descendant("CIDFontType2", "Japan1", "", 15),
        descendant("CIDFontType2", "Identity", "", 17),
        b"<< /Type /FontDescriptor /Flags 4 /FontFile2 19 0 R >>".to_vec(),
        b"<< /Type /FontDescriptor /Flags 4 /FontFile3 20 0 R >>".to_vec(),
        b"<< /Type /FontDescriptor /Flags 4 >>".to_vec(),
        stream(&map, ""),
        stream(&tt, ""),
        stream(&cff, "/Subtype /CIDFontType0C"),
    ];
    let pdf = document("/A 5 0 R /B 6 0 R /C 7 0 R /D 8 0 R /E 9 0 R", "", &objects);
    let settings = InterpreterSettings::default();
    let codes = || 0..=80u16;
    let mut seen = CidProbeCache::default();
    let mut read = Vec::new();
    for dict in page_fonts(&pdf) {
        let probe = CidFontProbe::read(&dict, &mut seen).map(|p| {
            (
                p.kind(),
                p.glyphs(),
                codes().map(|c| p.number(c)).collect::<Vec<_>>(),
            )
        });
        let built = built_cid_font_numbers(&dict, &settings, codes());
        assert_eq!(probe, built, "font {:?}", dict.obj_id());
        read.push(probe.map(|(kind, glyphs, numbers)| (kind, glyphs, numbers[..4].to_vec())));
    }
    assert_eq!(
        read,
        vec![
            Some((
                FontKind::CidTrueType,
                Some(6),
                vec![Some(0), Some(1), Some(2), Some(3)]
            )),
            Some((
                FontKind::CidTrueType,
                Some(6),
                vec![Some(0), Some(5), Some(3), None]
            )),
            Some((
                FontKind::CidCff,
                Some(4),
                vec![Some(0), Some(1), Some(2), Some(3)]
            )),
            None,
            None,
        ]
    );
    assert_eq!(
        seen.programs_read(),
        2,
        "one TrueType program for two fonts, one CFF"
    );
}

/// The glyph-name readings a reader of the facts shares with the strict text.
#[test]
fn glyph_names_read_as_the_strict_text_reads_them() {
    use crate::font::{
        glyph_name_numbers, glyph_name_to_unicode, is_numbered_glyph_name, symbol_encoding_name,
    };
    assert!(is_numbered_glyph_name("C83") && is_numbered_glyph_name("MT252"));
    assert!(!is_numbered_glyph_name("A") && !is_numbered_glyph_name("uni0041"));
    assert!(!is_numbered_glyph_name("Euro"), "a glyph list name");
    assert!(
        glyph_name_numbers("G41").any(|n| n == 41) && glyph_name_numbers("G41").any(|n| n == 0x41)
    );
    assert_eq!(glyph_name_to_unicode("Adieresis"), Some('Ä'));
    assert_eq!(symbol_encoding_name(0x61), Some("alpha"));
    assert_eq!(symbol_encoding_name(0x80), None);
}

/// The glyph list read both ways, as the strict text reads names: an expert font's small
/// capital names a private use character and that character names it back; the `uni`/`u` forms
/// are read by the Adobe Glyph List Specification's rule alone, upper-case hex of the right
/// length.
#[test]
fn the_glyph_list_reads_names_and_characters() {
    use crate::font::{agl_unicode_from_name, glyph_list_name, glyph_list_text};
    assert_eq!(glyph_list_text("Asmall"), Some("\u{F761}"));
    assert_eq!(glyph_list_text("zerooldstyle"), Some("\u{F730}"));
    assert_eq!(glyph_list_name('\u{F761}'), Some("Asmall"));
    assert_eq!(glyph_list_name('A'), Some("A"));
    assert_eq!(glyph_list_text("uni0041"), None, "not a list name");
    assert_eq!(glyph_list_text("a.sc"), None, "no suffixes");
    assert_eq!(glyph_list_name('\u{E000}'), None);
    assert_eq!(agl_unicode_from_name("uni0041"), Some('A'));
    assert_eq!(agl_unicode_from_name("u1F600"), Some('\u{1F600}'));
    assert_eq!(agl_unicode_from_name("uni65"), None);
    assert_eq!(agl_unicode_from_name("uni20ac"), None);
    assert_eq!(agl_unicode_from_name("uD800"), None);
}
