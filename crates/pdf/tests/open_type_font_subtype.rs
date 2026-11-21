//! A font dictionary whose own `/Subtype` is `/OpenType`, which ISO 32000-2 §9.6.2, Table 109
//! doesn't list, is read by the program it embeds (PdfCraft patch 111, storytold/pdfcraft#7387):
//! a CFF program as a Type 1 font, TrueType outlines as a TrueType font (PDFBOX-5463). A
//! `FontFile3 /OpenType` program with TrueType outlines is read in a `/TrueType` font dictionary
//! too, as Table 124 allows.
//!
//! The CFF program is the bundled fixed-pitch stand-in face, and the TrueType one is built here
//! in memory (each letter a filled box), so no font file is added. Drawn right, the same program
//! gives the same pixels however it is wrapped and labelled; the stand-in a serif font falls back
//! to is a different face.
#![cfg(feature = "raster")]
#![allow(clippy::unwrap_used)]

use craft_pdf::open_slice;
use craft_pdf::raster::{self, Options, RenderCache};

const CFF: &[u8] = include_bytes!("../../../vendor/hayro-interpret/assets/FoxitFixed.pfb");

/// `cff` in the smallest OpenType wrapper: an `OTTO` table directory with one `CFF ` table.
fn open_type(cff: &[u8]) -> Vec<u8> {
    let mut out = b"OTTO".to_vec();
    out.extend_from_slice(&[0, 1, 0, 16, 0, 0, 0, 0]);
    out.extend_from_slice(b"CFF ");
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&28u32.to_be_bytes());
    out.extend_from_slice(&u32::try_from(cff.len()).unwrap().to_be_bytes());
    out.extend_from_slice(cff);
    out
}

fn be16(out: &mut Vec<u8>, n: u16) {
    out.extend_from_slice(&n.to_be_bytes());
}

fn be32(out: &mut Vec<u8>, n: u32) {
    out.extend_from_slice(&n.to_be_bytes());
}

/// A TrueType font (`0x00010000`) of two glyphs, `.notdef` empty and glyph 1 a box from (50, 0)
/// to (550, 700), with a (1, 0) `cmap` subtable mapping `A`, `g` and `W` to the box.
fn true_type() -> Vec<u8> {
    let mut glyf = Vec::new();
    for n in [1i16, 50, 0, 550, 700] {
        glyf.extend_from_slice(&n.to_be_bytes());
    }
    be16(&mut glyf, 3); // the contour's last point
    be16(&mut glyf, 0); // no instructions
    glyf.extend([1u8; 4]); // four on-curve points, two-byte coordinates
    for dx in [50i16, 500, 0, -500] {
        glyf.extend_from_slice(&dx.to_be_bytes());
    }
    for dy in [0i16, 0, 700, 0] {
        glyf.extend_from_slice(&dy.to_be_bytes());
    }
    let mut loca = Vec::new();
    for offset in [0u32, 0, glyf.len() as u32] {
        be32(&mut loca, offset);
    }

    let mut head = vec![0; 54];
    head[0..4].copy_from_slice(&[0, 1, 0, 0]);
    head[12..16].copy_from_slice(&0x5f0f_3cf5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[40..42].copy_from_slice(&600i16.to_be_bytes());
    head[42..44].copy_from_slice(&700i16.to_be_bytes());
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    let mut hhea = vec![0; 36];
    hhea[0..4].copy_from_slice(&[0, 1, 0, 0]);
    hhea[4..6].copy_from_slice(&800i16.to_be_bytes());
    hhea[34..36].copy_from_slice(&2u16.to_be_bytes());
    let mut hmtx = Vec::new();
    for _ in 0..2 {
        be16(&mut hmtx, 600);
        be16(&mut hmtx, 0);
    }
    let mut maxp = vec![0, 1, 0, 0];
    be16(&mut maxp, 2);
    maxp.resize(32, 0);

    // (1, 0), format 0.
    let mut mac = Vec::new();
    be16(&mut mac, 0);
    be16(&mut mac, 262);
    be16(&mut mac, 0);
    let mut ids = [0u8; 256];
    for c in *b"AgW" {
        ids[usize::from(c)] = 1;
    }
    mac.extend(ids);
    let mut cmap = Vec::new();
    be16(&mut cmap, 0);
    be16(&mut cmap, 1);
    be16(&mut cmap, 1);
    be16(&mut cmap, 0);
    be32(&mut cmap, 12);
    cmap.extend(mac);

    let tables = [(*b"cmap", cmap), (*b"glyf", glyf), (*b"head", head), (*b"hhea", hhea), (*b"hmtx", hmtx), (*b"loca", loca), (*b"maxp", maxp)];
    let mut out = vec![0, 1, 0, 0];
    be16(&mut out, tables.len() as u16);
    for n in [64u16, 2, 48] {
        be16(&mut out, n);
    }
    let mut payload = Vec::new();
    let mut offset = 12 + 16 * tables.len();
    for (tag, bytes) in tables {
        out.extend(tag);
        let mut padded = bytes.clone();
        padded.resize(bytes.len().div_ceil(4) * 4, 0);
        let sum = padded.chunks(4).fold(0u32, |s, b| s.wrapping_add(u32::from_be_bytes(b.try_into().unwrap())));
        be32(&mut out, sum);
        be32(&mut out, offset as u32);
        be32(&mut out, bytes.len() as u32);
        offset += padded.len();
        payload.extend(padded);
    }
    out.extend(payload);
    out
}

/// `data` as a zlib stream of stored (uncompressed) deflate blocks (RFC 1950, RFC 1951 §3.2.4).
fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    let chunks: Vec<&[u8]> = data.chunks(0xFFFF).collect();
    for (i, chunk) in chunks.iter().enumerate() {
        out.push(u8::from(i + 1 == chunks.len()));
        let len = chunk.len() as u16;
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    out.extend_from_slice(&((b << 16) | a).to_be_bytes());
    out
}

/// A page showing "AgW" at 60 pt in a serif font of `font_subtype` whose descriptor's `key`
/// holds `program` (of stream entries `stream`, if not empty), or which embeds no program when
/// `None`. `flags` are the descriptor's `/Flags`.
fn font_page(font_subtype: &str, flags: u32, program: Option<(&[u8], &str, &str)>) -> Vec<u8> {
    let file = program.map(|(_, key, _)| format!(" /{key} 7 0 R")).unwrap_or_default();
    let body = "BT /F1 60 Tf 10 30 Td (AgW) Tj ET";
    let widths = vec!["600"; 39].join(" ");
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 120] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
        format!("<< /Length {} >>\nstream\n{body}\nendstream", body.len()).into_bytes(),
        format!("<< /Type /Font /Subtype /{font_subtype} /BaseFont /Synthetic-Semibold /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 103 /Widths [{widths}] /FontDescriptor 6 0 R >>").into_bytes(),
        format!("<< /Type /FontDescriptor /FontName /Synthetic-Semibold /Flags {flags} /FontBBox [0 -200 1000 900] /ItalicAngle 0 /Ascent 800 /Descent -200 /CapHeight 700 /StemV 120{file} >>").into_bytes(),
    ];
    if let Some((program, _, entries)) = program {
        let mut stream = format!("<< {entries} /Length {} >>\nstream\n", program.len()).into_bytes();
        stream.extend_from_slice(program);
        stream.extend_from_slice(b"\nendstream");
        objects.push(stream);
    }

    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        bytes.extend_from_slice(object);
        bytes.extend_from_slice(b"\nendobj\n");
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objects.len() + 1).as_bytes());
    bytes
}

/// A serif, nonsymbolic font.
const SERIF: u32 = 34;
/// A symbolic font, whose codes reach the TrueType program's (1, 0) `cmap` subtable directly.
const SYMBOLIC: u32 = 4;

fn pixels(bytes: Vec<u8>) -> Vec<u8> {
    let pdf = open_slice(&bytes, None).unwrap();
    let raster =
        raster::render(&pdf.pages()[0], &RenderCache::new(), &Default::default(), &Options { background: [255; 4], ..Default::default() }).unwrap();
    raster.bytes().to_vec()
}

fn inked(rgba: &[u8]) -> usize {
    rgba.chunks(4).filter(|p| p.first().is_some_and(|r| *r < 128)).count()
}

/// PDFBOX-5463: a `/Subtype /OpenType` font dictionary with an `OTTO` (CFF) `FontFile3 /OpenType`
/// draws its program, exactly as the `Type1` font with the same program does. It was read as
/// TrueType, which reads only a `FontFile2`, and drew the stand-in.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_open_type_font_dictionary_with_a_cff_program_draws_it_as_a_type1_font() {
    let wrapped = open_type(CFF);
    let type1 = pixels(font_page("Type1", SERIF, Some((&wrapped, "FontFile3", "/Subtype /OpenType"))));
    assert!(inked(&type1) > 200, "the Type1 font drew no text");
    let open_type_dict = pixels(font_page("OpenType", SERIF, Some((&wrapped, "FontFile3", "/Subtype /OpenType"))));
    assert!(open_type_dict == type1, "the /OpenType font drew another face than its CFF program");
    let bare = pixels(font_page("OpenType", SERIF, Some((CFF, "FontFile3", "/Subtype /Type1C"))));
    assert!(bare == type1, "the /OpenType font drew another face than its bare CFF program");
    assert!(type1 != pixels(font_page("OpenType", SERIF, None)), "the program and the stand-in look alike");
}

/// A `/Subtype /OpenType` font whose `FontFile3 /OpenType` has TrueType outlines, and a
/// `/Subtype /TrueType` font with the same program (ISO 32000-2 Table 124 allows it there), draw
/// it as the TrueType font with the program in a `FontFile2` does.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_font_file3_open_type_program_with_true_type_outlines_draws_as_true_type() {
    let program = true_type();
    let reference = pixels(font_page("TrueType", SYMBOLIC, Some((&program, "FontFile2", ""))));
    assert!(inked(&reference) > 200, "the TrueType font drew no text");
    assert!(reference != pixels(font_page("TrueType", SYMBOLIC, None)), "the program and the stand-in look alike");
    for font_subtype in ["OpenType", "TrueType"] {
        let rgba = pixels(font_page(font_subtype, SYMBOLIC, Some((&program, "FontFile3", "/Subtype /OpenType"))));
        assert!(rgba == reference, "/{font_subtype} with FontFile3 /OpenType drew another face than its program");
    }
    let file2 = pixels(font_page("OpenType", SYMBOLIC, Some((&program, "FontFile2", ""))));
    assert!(file2 == reference, "/OpenType with a FontFile2 drew another face than its program");
}

/// A `FontFile3 /OpenType` program with TrueType outlines behind Flate and a PNG predictor
/// (ISO 32000-2 §7.4.4.4), or behind two filters (§7.3.8.2), draws as TrueType: its sfnt tag is
/// read from the decoded program, where the predictor gives nothing from the first four inflated
/// bytes because a row is 16 bytes long.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_predicted_true_type_program_is_still_read_as_true_type() {
    let program = true_type();
    let reference = pixels(font_page("TrueType", SYMBOLIC, Some((&program, "FontFile2", ""))));
    let mut padded = program.clone();
    padded.resize(program.len().div_ceil(16) * 16, 0);
    let rows: Vec<u8> = padded.chunks(16).flat_map(|row| std::iter::once(0).chain(row.iter().copied())).collect();
    let encoded = zlib_stored(&rows);
    let entries = "/Subtype /OpenType /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 16 >>";
    for font_subtype in ["OpenType", "TrueType"] {
        let rgba = pixels(font_page(font_subtype, SYMBOLIC, Some((&encoded, "FontFile3", entries))));
        assert!(rgba == reference, "/{font_subtype} with a predicted FontFile3 /OpenType drew another face than its program");
    }
    // A filter after another: four hex digits give two bytes of the zlib stream, too few to inflate.
    let mut hex: Vec<u8> = encoded.iter().flat_map(|b| format!("{b:02X}").into_bytes()).collect();
    hex.push(b'>');
    let chain = "/Subtype /OpenType /Filter [/ASCIIHexDecode /FlateDecode] /DecodeParms [null << /Predictor 12 /Columns 16 >>]";
    let rgba = pixels(font_page("OpenType", SYMBOLIC, Some((&hex, "FontFile3", chain))));
    assert!(rgba == reference, "a FontFile3 /OpenType behind two filters drew another face than its program");
}

/// The same for a CFF program in an `OTTO` wrapper: behind Flate and a PNG predictor, its tag is
/// read from the whole decoded program, so the `/OpenType` font is still read as Type 1.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn a_predicted_cff_program_is_still_read_as_type1() {
    let wrapped = open_type(CFF);
    let reference = pixels(font_page("Type1", SERIF, Some((&wrapped, "FontFile3", "/Subtype /OpenType"))));
    let mut padded = wrapped.clone();
    padded.resize(wrapped.len().div_ceil(16) * 16, 0);
    let rows: Vec<u8> = padded.chunks(16).flat_map(|row| std::iter::once(0).chain(row.iter().copied())).collect();
    let entries = "/Subtype /OpenType /Filter /FlateDecode /DecodeParms << /Predictor 12 /Columns 16 >>";
    let rgba = pixels(font_page("OpenType", SERIF, Some((&zlib_stored(&rows), "FontFile3", entries))));
    assert!(rgba == reference, "the /OpenType font with a predicted CFF program drew another face than its program");
}

/// A `/Subtype /OpenType` font with an unreadable CFF program, or with no program, still draws
/// a stand-in rather than failing the page.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn an_open_type_font_dictionary_without_a_usable_program_falls_back() {
    let mut broken = open_type(CFF);
    broken.get_mut(12..16).unwrap().copy_from_slice(b"glyf");
    for program in [Some((&broken[..], "FontFile3", "/Subtype /OpenType")), Some((&[][..], "FontFile3", "/Subtype /OpenType")), None] {
        assert!(inked(&pixels(font_page("OpenType", SERIF, program))) > 200, "the stand-in drew no text");
    }
}
