//! Portable regressions adapted from PdfCraft 7437bde378f43856eada40af25b983812e615e8a and
//! a2ec27b4775f53c40cad3456936d9406611e3a8d (MIT OR Apache-2.0).
#![cfg(feature = "raster")]
use craft_pdf::interpret::InterpreterSettings;
use craft_pdf::{
    open_slice,
    raster::{self, Options, RenderCache},
};
use std::sync::Arc;

struct Pixels {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}
fn render(bytes: &[u8]) -> Pixels {
    let pdf = open_slice(bytes, None).unwrap();
    let raster =
        raster::render(&pdf.pages()[0], &RenderCache::new(), &Default::default(), &Options { background: [255; 4], ..Default::default() }).unwrap();
    assert!(raster.diagnostics.is_empty(), "{:?}", raster.diagnostics);
    Pixels { width: raster.width, height: raster.height, rgba: raster.bytes().to_vec() }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

// These frozen RGB8 values were computed outside the renderer in f64 from the
// CIE Lab inverse, the full XYZ adaptation matrix B^-1 diag(B*D65 / B*white) B,
// and the standard sRGB matrix/transfer function. No production helper is used.
// Bradford reference: https://www.color.org/chadtag/ (ICC TN 02-2003).
// White points: D50 [0.96422, 1, 0.82521], E [1, 1, 1],
// D65 [0.95047, 1, 1.08883]. D65 adaptation is the identity.
const SAMPLES: [[f64; 3]; 4] = [[50.0, 0.0, 0.0], [40.0, 20.0, -60.0], [70.0, -50.0, 30.0], [100.0, 0.0, 0.0]];
const D50: [[u8; 3]; 4] = [[119, 119, 119], [67, 86, 194], [66, 194, 113], [255, 255, 255]];
const E: [[u8; 3]; 4] = [[119, 119, 119], [50, 88, 194], [64, 194, 114], [255, 255, 255]];
const D65: [[u8; 3]; 4] = [[119, 119, 119], [19, 90, 194], [77, 193, 114], [255, 255, 255]];

fn patch(space: &str, operands: &str) -> [u8; 4] {
    let content = format!("/C cs {operands} sc 0 0 20 20 re f");
    let bytes = format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 20 20] /Resources << /ColorSpace << /C {space} >> >> /Contents 4 0 R >> endobj\n\
         4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n\
         trailer << /Root 1 0 R >>\n%%EOF",
        content.len()
    )
    .into_bytes();
    let page = render(&bytes);
    assert_eq!((page.width, page.height), (20, 20));
    page.rgba.get((10 * 20 + 10) * 4..(10 * 20 + 10) * 4 + 4).unwrap().try_into().unwrap()
}

fn assert_rgb(actual: [u8; 4], expected: [u8; 3], context: &str) {
    assert_eq!(actual[3], 255, "{context}: {actual:?}");
    for (got, want) in actual.into_iter().zip(expected) {
        assert!(got.abs_diff(want) <= 1, "{context}: got {actual:?}, expected {expected:?}");
    }
}

fn check_white(white: &str, expected: [[u8; 3]; 4]) {
    let space = format!("[/Lab << /WhitePoint {white} /Range [-100 100 -100 100] >>]");
    for (sample, expected) in SAMPLES.into_iter().zip(expected) {
        let operands = format!("{} {} {}", sample[0], sample[1], sample[2]);
        assert_rgb(patch(&space, &operands), expected, &format!("{white} Lab {sample:?}"));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn lab_bradford_d50_neutral_and_chromatic_oracles() {
    check_white("[0.96422 1 0.82521]", D50);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn lab_bradford_equal_energy_neutral_and_chromatic_oracles() {
    check_white("[1 1 1]", E);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn lab_bradford_d65_identity_oracles() {
    check_white("[0.95047 1 1.08883]", D65);
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn lab_bradford_indexed_matches_direct_and_oracle() {
    for (white, blue) in [("[0.96422 1 0.82521]", D50[1]), ("[1 1 1]", E[1]), ("[0.95047 1 1.08883]", D65[1])] {
        // <66 99 33> maps exactly to L*=40, a*=20, b*=-60.
        let lab = format!("[/Lab << /WhitePoint {white} /Range [-100 100 -100 100] >>]");
        let indexed = patch(&format!("[/Indexed {lab} 0 <669933>]"), "0");
        assert_rgb(indexed, blue, &format!("{white} Indexed blue"));
        assert_eq!(indexed, patch(&lab, "40 20 -60"), "{white} direct/Indexed blue");

        // A non-default Range maps <80 80 80> to L*=128/255*100, a*=b*=0.
        let lab = format!("[/Lab << /WhitePoint {white} /Range [-128 127 -128 127] >>]");
        let indexed = patch(&format!("[/Indexed {lab} 0 <808080>]"), "0");
        assert_rgb(indexed, [119, 119, 119], &format!("{white} Indexed neutral"));
        assert_eq!(indexed, patch(&lab, "50.19607843137255 0 0"), "{white} direct/Indexed neutral");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn lab_bradford_degenerate_cones_fall_back_to_unadapted_xyz() {
    // In f32, B*[1, 1, 7.1964064] has a zero L cone response.
    // B*[3e38, 3e38, 3e38] overflows its L/M responses. The guard must
    // return the sample XYZ in both cases. Independent unadapted XYZ ->
    // sRGB references for Lab (50, 0, 0) are frozen below.
    for (white, expected) in [
        ("[1 1 7.1964064]", [0, 130, 255]),
        (
            "[300000000000000000000000000000000000000 300000000000000000000000000000000000000 300000000000000000000000000000000000000]",
            [255, 255, 255],
        ),
    ] {
        let space = format!("[/Lab << /WhitePoint {white} >>]");
        assert_rgb(patch(&space, "50 0 0"), expected, &format!("{white} degenerate cones"));
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn lab_bradford_asymmetric_chromatic_range_matches_direct_and_oracle() {
    // With a* [-40, 70] and b* [-90, 20], <80 99 33> maps to
    // L*=128/255*100, a*=26, b*=-68. Frozen independent D50 oracle:
    // RGB (98, 108, 237). The unequal, asymmetric ranges expose swaps
    // and default-Range scaling that a neutral lookup cannot detect.
    let lab = "[/Lab << /WhitePoint [0.96422 1 0.82521] /Range [-40 70 -90 20] >>]";
    let indexed = patch(&format!("[/Indexed {lab} 0 <809933>]"), "0");
    assert_rgb(indexed, [98, 108, 237], "D50 asymmetric chromatic Range");
    assert_eq!(indexed, patch(lab, "50.19607843137255 26 -68"));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn shrunk_soft_mask_larger_than_its_image_keeps_thin_strokes() {
    let mask: [u8; 8] = [0, 0, 255, 0, 0, 0, 0, 0];
    let content = "q 4 0 0 1 0 0 cm /Im0 Do Q";
    let pdf = format!(
        "%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 4 1] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R >> >> >> endobj\n\
         4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n\
         5 0 obj << /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /SMask 6 0 R /Filter /ASCIIHexDecode /Length 3 >> stream\n00>\nendstream endobj\n\
         6 0 obj << /Type /XObject /Subtype /Image /Width 8 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length {} >> stream\n{}>\nendstream endobj\n\
         trailer << /Root 1 0 R >>\n%%EOF\n",
        content.len(),
        hex(&mask).len() + 1,
        hex(&mask)
    );
    let page = render(pdf.as_bytes());
    assert_eq!((page.width, page.height), (4, 1));
    let grays: Vec<u8> = page.rgba.as_chunks::<4>().0.iter().map(|px| px[0]).collect();
    // The stroke covers half of device pixel 1, so it is a mid-tone; the far side stays white.
    assert!((64..=191).contains(&grays[1]), "the stroke is lost or not a mid-tone: {grays:?}");
    assert_eq!(grays[3], 255, "{grays:?}");
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn text_rendering_modes_other_than_invisible_draw_type3_glyphs_as_filled() {
    let render = |glyph: &str, mode: u8| {
        let content = format!("BT 1 0 0 RG 6 w 0 g /F1 20 Tf {mode} Tr 10.3 10.3 Td (A) Tj ET");
        let pdf = format!(
            "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length {} >> stream
{content}
endstream endobj
5 0 obj << /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /FirstChar 65 /LastChar 65 /Widths [1]
  /Encoding << /Differences [65 /a] >> /CharProcs << /a 6 0 R >> >> endobj
6 0 obj << /Length {} >> stream
{glyph}
endstream endobj
trailer << /Root 1 0 R >>
%%EOF",
            content.len(),
            glyph.len()
        );
        let p = render(pdf.as_bytes());
        p.rgba
    };
    for glyph in ["1 0 0 0 1 1 d1 0.12 0.12 0.71 0.71 re f", "1 0 d0 0.12 0.12 0.71 0.71 re f"] {
        let filled = render(glyph, 0);
        assert!(filled.as_chunks::<4>().0.iter().any(|p| p[0] < 128), "{glyph}: the glyph paints");
        for mode in [1, 2, 4, 5, 6, 7] {
            assert!(render(glyph, mode) == filled, "{glyph}: {mode} Tr draws as 0 Tr");
        }
        assert!(render(glyph, 3).as_chunks::<4>().0.iter().all(|p| p[..3] == [255, 255, 255]), "{glyph}: 3 Tr hides the glyph");
    }
}

// Fixed zlib streams encode the two known rows in each case. These values are
// independent of the decoder and preserve packed row padding in 1/2/4-bit RGB.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn tiff_predictor_packed_rgb_rows_keep_their_byte_boundaries() {
    use craft_pdf::syntax::object::{FromBytes, Object};
    struct RowCase {
        bpc: u8,
        columns: u32,
        compressed: &'static [u8],
        expected: &'static [u8],
        masks: &'static [u8],
    }
    let cases = [
        RowCase { bpc: 1, columns: 1, compressed: &[120, 156, 107, 112, 0, 0, 1, 66, 0, 193], expected: &[128, 64], masks: &[224] },
        RowCase { bpc: 2, columns: 1, compressed: &[120, 156, 59, 96, 0, 0, 1, 178, 0, 241], expected: &[192, 48], masks: &[252] },
        RowCase {
            bpc: 4,
            columns: 1,
            compressed: &[120, 156, 251, 192, 192, 207, 0, 0, 3, 226, 1, 0],
            expected: &[240, 0, 15, 0],
            masks: &[255, 240],
        },
        RowCase {
            bpc: 1,
            columns: 3,
            compressed: &[120, 156, 155, 217, 16, 213, 0, 0, 5, 28, 1, 244],
            expected: &[136, 128, 80, 128],
            masks: &[255, 128],
        },
        RowCase {
            bpc: 8,
            columns: 1,
            compressed: &[120, 156, 251, 207, 192, 192, 240, 159, 1, 0, 7, 254, 1, 255],
            expected: &[255, 0, 0, 0, 255, 0],
            masks: &[255, 255, 255],
        },
    ];
    for RowCase { bpc, columns, compressed, expected, masks } in cases {
        let mut bytes = format!(
            "<< /Filter /FlateDecode /DecodeParms << /Predictor 2 /Colors 3 /BitsPerComponent {bpc} /Columns {columns} >> /Length {} >> stream\n",
            compressed.len()
        )
        .into_bytes();
        bytes.extend_from_slice(compressed);
        bytes.extend_from_slice(b"\nendstream");
        let Object::Stream(stream) = Object::from_bytes(&bytes).unwrap() else { panic!("stream expected") };
        let decoded = stream.decoded().unwrap();
        assert_eq!(decoded.len(), expected.len());
        for ((actual, expected), mask) in decoded.iter().zip(expected).zip(masks.iter().cycle()) {
            assert_eq!(actual & mask, *expected, "{bpc}-bit RGB, {columns} columns");
        }
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn tiff_predictor_packed_rgb_image_renders_every_row() {
    // Deflated [0x80, 0x40]: one red then one green 1-bit RGB sample, each byte-aligned.
    let compressed = [120, 156, 107, 112, 0, 0, 1, 66, 0, 193];
    let content = "q 40 0 0 40 0 0 cm /Im1 Do Q";
    let mut pdf = format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R /Resources << /XObject << /Im1 5 0 R >> >> >> endobj\n\
         4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n\
         5 0 obj << /Type /XObject /Subtype /Image /Width 1 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 1 /Filter /FlateDecode /DecodeParms << /Predictor 2 /Colors 3 /BitsPerComponent 1 /Columns 1 >> /Length {} >> stream\n",
        content.len(), compressed.len()
    ).into_bytes();
    pdf.extend_from_slice(&compressed);
    pdf.extend_from_slice(b"\nendstream endobj\ntrailer << /Root 1 0 R >>\n%%EOF\n");
    let page = render(&pdf);
    assert_eq!((page.width, page.height), (40, 40));
    let pixel = |y: usize| &page.rgba[((y * 40 + 20) * 4)..][..4];
    assert_eq!(pixel(10), &[255, 0, 0, 255]);
    assert_eq!(pixel(30), &[0, 255, 0, 255]);
}

// An 8 x 4 all-black Group 4 stream, hand-encoded from ITU-T T.6: one horizontal-mode row
// (white 0, black 8), three V0 rows, then EOFB.
const ALL_BLACK_8X4: [u8; 6] = [0x26, 0xa2, 0xfe, 0x00, 0x20, 0x02];

fn fax_page(space: &str, params: &str) -> Pixels {
    let content = "q 20 0 0 20 5 5 cm /Im1 Do Q";
    let mut pdf = format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R /Resources << /XObject << /Im1 5 0 R >> >> >> endobj
4 0 obj << /Length {} >> stream
{content}
endstream endobj
5 0 obj << /Type /XObject /Subtype /Image /Width 8 /Height 4 /ColorSpace {space} /BitsPerComponent 1 /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8 {params} >> /Length {} >> stream\n",
        content.len(),
        ALL_BLACK_8X4.len()
    )
    .into_bytes();
    pdf.extend_from_slice(&ALL_BLACK_8X4);
    pdf.extend_from_slice(b"\nendstream endobj\ntrailer << /Root 1 0 R >>\n%%EOF");
    render(&pdf)
}

fn pixel(page: &Pixels, x: u32, y: u32) -> &[u8] {
    let i = ((y * page.width + x) * 4) as usize;
    &page.rgba[i..i + 4]
}

/// Scanners and fax services write Group 4 pages with `/Rows 0`, which ISO 32000-2 Table 11
/// defines as "height not predetermined" (the default), not as zero rows.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn group4_fax_with_rows_zero_decodes_to_the_image_height() {
    for rows in ["", "/Rows 4", "/Rows 0"] {
        let page = fax_page("/DeviceGray", rows);
        assert_eq!(pixel(&page, 15, 24), &[0, 0, 0, 255], "{rows:?}: the fax image paints black");
        assert_eq!(pixel(&page, 1, 38), &[255, 255, 255, 255], "{rows:?}: outside the image stays white");
    }
}

/// PdfCraft #3308: a fax image whose `/Rows` is below its `/Height` is decoded to its height.
/// `/BlackIs1` turns the fixture white, so rows left undecoded show as black. Also through an
/// `/Indexed` color space, the filter's other output path.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn group4_fax_with_rows_below_its_height_decodes_to_the_image_height() {
    for (space, rows) in [("/DeviceGray", "/Rows 2"), ("[/Indexed /DeviceRGB 1 <000000FFFFFF>]", "/Rows 2"), ("/DeviceGray", "/Rows 4")] {
        let page = fax_page(space, &format!("{rows} /BlackIs1 true"));
        for y in [17, 22, 27, 32] {
            assert_eq!(pixel(&page, 15, y), &[255, 255, 255, 255], "{space} {rows}: image row at y {y} is decoded");
        }
    }
}

/// PdfCraft ledger hayro-interpret P2 (#3178): a shaded Type 3 glyph clips to its `/FontBBox` in
/// device space at every position, not to the glyph-space box.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn shaded_type3_glyph_draws_at_multiple_positions() {
    let content = "BT /F 30 Tf 1 0 0 1 50 70 Tm (A) Tj 1 0 0 1 220 70 Tm (A) Tj ET";
    let glyph = "100 0 d0 0 0 100 100 re W n /S sh";
    let pdf = format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 150] /Resources << /Font << /F 5 0 R >> >> /Contents 4 0 R >> endobj
4 0 obj << /Length {} >> stream
{content}
endstream endobj
5 0 obj << /Type /Font /Subtype /Type3 /FontBBox [0 0 100 100] /FontMatrix [0.01 0 0 0.01 0 0] /CharProcs << /A 6 0 R >> /Encoding << /Differences [65 /A] >> /FirstChar 65 /LastChar 65 /Widths [100] /Resources << /Shading << /S 7 0 R >> >> >> endobj
6 0 obj << /Length {} >> stream
{glyph}
endstream endobj
7 0 obj << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [0 0 1] /N 1 >> /Extend [true true] >> endobj
trailer << /Root 1 0 R >>
%%EOF",
        content.len(),
        glyph.len()
    );
    let page = render(pdf.as_bytes());
    for x in [65, 235] {
        let px = pixel(&page, x, 65);
        assert!(px[0] < 240 && px[2] < 240 && px[1] < 10, "glyph at {x}: {px:?}");
    }
}

/// PdfCraft ledger hayro-interpret (26) extension: comma-below letters in non-embedded standard
/// fonts are the letter with the font's comma under it, apart from the letter and inside the
/// descender, whichever name the font gives them (`/scommaaccent`, `/Kcedilla`, `/uni0219`).
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn comma_below_letters_in_standard_fonts_draw_a_comma_under_the_letter() {
    let pairs = [
        ("s", "uni0219"),
        ("t", "uni021B"),
        ("T", "uni021A"),
        ("K", "uni0136"),
        ("k", "kcedilla"),
        ("G", "uni0122"),
        ("G", "Gcedilla"),
        ("R", "uni0156"),
        ("s", "scommaaccent"),
        ("S", "Scommaaccent"),
        ("t", "tcommaaccent"),
        ("T", "Tcommaaccent"),
        ("k", "kcommaaccent"),
        ("K", "Kcommaaccent"),
        ("l", "lcommaaccent"),
        ("L", "Lcommaaccent"),
        ("n", "ncommaaccent"),
        ("N", "Ncommaaccent"),
        ("r", "rcommaaccent"),
        ("R", "Rcommaaccent"),
        ("G", "Gcommaaccent"),
    ];
    let letters: Vec<&str> = pairs.iter().flat_map(|(base, accented)| [*base, *accented]).collect();
    let (size, baseline) = (40u32, 60u32);
    for font in ["Helvetica", "Times-Roman", "Courier", "Helvetica-BoldOblique"] {
        let mut content = String::new();
        for i in 0..letters.len() {
            content.push_str(&format!("BT /F1 {size} Tf {} {} Td <{:02X}> Tj ET\n", 10 + 40 * i, 80 - baseline, 1 + i));
        }
        let differences = letters.iter().map(|n| format!("/{n}")).collect::<Vec<_>>().join(" ");
        let pdf = format!(
            "%PDF-1.4
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 {} 80] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj
4 0 obj << /Length {} >> stream
{content}endstream endobj
5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /{font}
  /Encoding << /Type /Encoding /BaseEncoding /WinAnsiEncoding /Differences [1 {differences}] >> >> endobj
trailer << /Root 1 0 R >>
%%EOF",
            20 + 40 * letters.len(),
            content.len()
        );
        let p = render(pdf.as_bytes());
        // The rows (y down) of the i-th letter that have dark pixels.
        let ink_rows = |i: u32| -> Vec<u32> {
            (0..p.height).filter(|&y| ((8 + 40 * i)..(48 + 40 * i).min(p.width)).any(|x| pixel(&p, x, y)[0] < 128)).collect()
        };
        for (pair, (_, name)) in pairs.iter().enumerate() {
            let (base, accented) = (ink_rows(2 * pair as u32), ink_rows(2 * pair as u32 + 1));
            let (Some(&base_bottom), Some(&bottom)) = (base.last(), accented.last()) else {
                panic!("{font} {name}: draws nothing ({} rows of its base letter)", base.len());
            };
            assert!(bottom > base_bottom + 2, "{font} {name}: a mark hangs below the letter ({bottom} vs {base_bottom})");
            assert!(
                (base_bottom + 1..bottom).any(|y| !accented.contains(&y)),
                "{font} {name}: the mark is apart from the letter, a comma and not a cedilla"
            );
            assert!(bottom < baseline + size * 3 / 10, "{font} {name}: the comma stays within the descender ({bottom})");
        }
    }
}

/// A one-page PDF: `page` holds the page dictionary's entries besides its tree links and
/// contents (the media box, resources, annotations), and `objects` the objects from 5 on.
fn one_page(page: &str, content: &[u8], objects: &[u8]) -> Vec<u8> {
    let mut pdf = format!(
        "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
         2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
         3 0 obj << /Type /Page /Parent 2 0 R {page} /Contents 4 0 R >> endobj\n\
         4 0 obj << /Length {} >> stream\n",
        content.len()
    )
    .into_bytes();
    pdf.extend_from_slice(content);
    pdf.extend_from_slice(b"\nendstream endobj\n");
    pdf.extend_from_slice(objects);
    pdf.extend_from_slice(b"\ntrailer << /Root 1 0 R >>\n%%EOF\n");
    pdf
}

/// PdfCraft ledger hayro-interpret (46) and the hayro stencil paint (#2975): a stencil mask is painted in the
/// fill color at the fill opacity, once: at `ca 0.5` it is as opaque as a rectangle filled at
/// `ca 0.5` beside it. The same holds when the fill is a pattern, an uncolored tiling pattern
/// (drawn in its fill color, not its stroke color) or a shading pattern.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn stencil_masks_are_drawn_at_the_fill_opacity_once() {
    let fills = [("color", "0 0 1 rg"), ("uncolored tiling pattern", "/CsP cs 0 0 1 /Tp scn"), ("shading pattern", "/Pattern cs /Sp scn")];
    for (fill_name, fill) in fills {
        for alpha in ["1", "0.5", "0.25"] {
            let content = format!("/A gs {fill} q 100 0 0 100 20 50 cm /Msk Do Q 160 50 100 100 re f");
            let page = format!(
                "/MediaBox [0 0 300 200] /Resources << /ExtGState << /A << /ca {alpha} >> >> /XObject << /Msk 5 0 R >> \
                 /ColorSpace << /CsP [/Pattern /DeviceRGB] >> /Pattern << /Tp 6 0 R /Sp 7 0 R >> >>"
            );
            let objects = b"5 0 obj << /Type /XObject /Subtype /Image /Width 2 /Height 2 /ImageMask true /Length 2 >> stream\n\0\0\nendstream endobj\n\
6 0 obj << /Type /Pattern /PatternType 1 /PaintType 2 /TilingType 1 /BBox [0 0 10 10] /XStep 10 /YStep 10 /Resources << >> /Length 14 >> stream\n0 0 10 10 re f\nendstream endobj\n\
7 0 obj << /Type /Pattern /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 300 0] /Extend [true true] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 1] /C1 [0 0 1] /N 1 >> >> >> endobj";
            let p = render(&one_page(&page, content.as_bytes(), objects));
            // Pixel rows run top down: PDF (70, 100) is in the mask, (210, 100) in the rectangle.
            let (mask, rect) = (pixel(&p, 70, 100), pixel(&p, 210, 100));
            assert!(rect[..3] != [255, 255, 255], "{fill_name}, ca {alpha}: the rectangle is painted");
            assert!(mask.iter().zip(rect).all(|(m, r)| m.abs_diff(*r) <= 1), "{fill_name}, ca {alpha}: mask {mask:?}, rectangle {rect:?}");
        }
    }
}

/// PdfCraft #3246 (#3176, with ledger hayro-interpret (46)): `CA` and `ca` belong to the graphics
/// state at paint time, so a `gs` after `SCN` or `scn` still fades a shading pattern, and a
/// stroke takes the stroke alpha.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn shading_pattern_uses_the_alpha_current_at_paint_time() {
    let content = b"/Pattern CS /P0 SCN /S1 gs 10 w 0 75 m 100 75 l S /Pattern cs /P0 scn /F1 gs 0 0 100 50 re f";
    let page = "/MediaBox [0 0 100 100] /Resources << /ExtGState << /S1 << /CA 0.25 >> /F1 << /ca 0.25 >> >> /Pattern << /P0 5 0 R >> >>";
    let objects = b"5 0 obj << /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 100 0] /Extend [true true] \
/Function << /FunctionType 2 /Domain [0 1] /C0 [1 0 0] /C1 [1 0 0] /N 1 >> >> >> endobj";
    let p = render(&one_page(page, content, objects));
    // A quarter red over white: (255, 191, 191).
    for (what, y) in [("stroke", 25), ("fill", 75)] {
        let px = pixel(&p, 50, y);
        assert!(px[0] == 255 && (185..=197).contains(&px[1]) && (185..=197).contains(&px[2]), "{what}: {px:?}");
    }
}

fn type4_mesh_page(data: &[u8]) -> Pixels {
    let mut objects = format!(
        "5 0 obj << /ShadingType 4 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 \
         /Decode [0 255 0 255 0 1 0 1 0 1] /Length {} >> stream\n",
        data.len()
    )
    .into_bytes();
    objects.extend_from_slice(data);
    objects.extend_from_slice(b"\nendstream endobj");
    let page = render(&one_page("/MediaBox [0 0 50 50] /Resources << /Shading << /Sh0 5 0 R >> >>", b"/Sh0 sh", &objects));
    assert_eq!((page.width, page.height), (50, 50));
    page
}

/// PdfCraft ledger hayro-interpret (36) (#3035): two triangles of a mesh that share an edge cover it
/// together. Testing only each pixel's corner, strictly inside a triangle, left the shared
/// diagonal white, and a pixel row or column along the mesh's own sides.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn mesh_triangles_sharing_an_edge_leave_no_seam() {
    // A red square from 10 to 40 on a 50 × 50 page, cut along its diagonal; in device space
    // the diagonal runs through pixel centers and corners alike.
    let vertex = |flag: u8, x: u8, y: u8| [flag, x, y, 255, 0, 0];
    let (lo, hi) = (10, 40);
    let mut triangles = Vec::new();
    for [a, b, c] in [[(lo, lo), (hi, lo), (hi, hi)], [(lo, lo), (hi, hi), (lo, hi)]] {
        for (x, y) in [a, b, c] {
            triangles.extend(vertex(0, x, y));
        }
    }
    // Each triangle as its own three vertices, and the second one as a strip (flag 1).
    let mut strip = Vec::new();
    for (flag, (x, y)) in [(0, (lo, lo)), (0, (hi, lo)), (0, (hi, hi)), (2, (lo, hi))] {
        strip.extend(vertex(flag, x, y));
    }
    for (label, data) in [("separate triangles", triangles), ("a strip", strip)] {
        let page = type4_mesh_page(&data);
        let (mut white_inside, mut red_outside) = (Vec::new(), Vec::new());
        for y in 0..50 {
            for x in 0..50 {
                let p = pixel(&page, x, y);
                let inside = (10..40).contains(&x) && (10..40).contains(&y);
                if inside && p != [255, 0, 0, 255] {
                    white_inside.push((x, y));
                } else if !inside && p != [255, 255, 255, 255] {
                    red_outside.push((x, y));
                }
            }
        }
        assert!(white_inside.is_empty(), "{label}: unshaded pixels inside the square: {white_inside:?}");
        assert!(red_outside.is_empty(), "{label}: shaded pixels outside the square: {red_outside:?}");
    }

    // One triangle alone, the square's upper left half: a pixel whose corner lies on its
    // diagonal is taken by its center, so one wholly across the diagonal (x + y ≥ 50 in
    // device space) is not shaded, and one wholly inside is.
    let mut data = Vec::new();
    for (x, y) in [(lo, lo), (hi, hi), (lo, hi)] {
        data.extend(vertex(0, x, y));
    }
    let page = type4_mesh_page(&data);
    let (mut spilled, mut missing) = (Vec::new(), Vec::new());
    for y in 10..40 {
        for x in 10..40 {
            let p = pixel(&page, x, y);
            if x + y >= 50 && p == [255, 0, 0, 255] {
                spilled.push((x, y));
            } else if x + y + 2 <= 50 && p != [255, 0, 0, 255] {
                missing.push((x, y));
            }
        }
    }
    assert!(spilled.is_empty(), "shaded pixels wholly across the diagonal: {spilled:?}");
    assert!(missing.is_empty(), "unshaded pixels wholly inside the triangle: {missing:?}");
}

/// PdfCraft ledger hayro-interpret (40) (#2987): an optional content membership dictionary
/// with a visibility expression (`/VE`, ISO 32000-2 §8.11.2.2) shows its content only when the
/// expression is true, and the expression takes precedence over `/OCGs` and `/P`. A malformed
/// expression, one naming an object that isn't a listed group, or one nested too deep falls
/// back to `/OCGs`.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn visibility_expressions_decide_what_a_membership_dictionary_shows() {
    let render = |ocmd: &str, off: &[i32]| {
        let catalog = "<< /Type /Catalog /Pages 2 0 R /OCProperties << /OCGs [5 0 R 6 0 R] /D << /Order [5 0 R 6 0 R] >> >> >>";
        let objects = format!("5 0 obj << /Type /OCG /Name (A) >> endobj\n6 0 obj << /Type /OCG /Name (B) >> endobj\n7 0 obj {ocmd} endobj");
        let pdf = String::from_utf8(one_page(
            "/MediaBox [0 0 100 100] /Resources << /Properties << /M1 7 0 R >> >>",
            b"/OC /M1 BDC 1 0 0 rg 0 0 100 100 re f EMC",
            objects.as_bytes(),
        ))
        .unwrap()
        .replacen("<< /Type /Catalog /Pages 2 0 R >>", catalog, 1);
        let doc = open_slice(pdf.as_bytes(), None).unwrap();
        let settings = InterpreterSettings { ocg_overrides: Arc::new(off.iter().map(|n| (*n, 0, false)).collect()), ..Default::default() };
        let options = Options { background: [255; 4], scale: [0.5, 0.5], ..Default::default() };
        let raster = raster::render(&doc.pages()[0], &RenderCache::new(), &settings, &options).unwrap();
        raster.bytes()[..4] == [255, 0, 0, 255]
    };
    let and = "<< /Type /OCMD /VE [/And 5 0 R 6 0 R] >>";
    assert!(render(and, &[]), "And: both on");
    assert!(!render(and, &[5]), "And: one off");
    let or = "<< /Type /OCMD /VE [/Or 5 0 R 6 0 R] >>";
    assert!(render(or, &[5]), "Or: one on");
    assert!(!render(or, &[5, 6]), "Or: both off");
    let not = "<< /Type /OCMD /VE [/Not 5 0 R] >>";
    assert!(!render(not, &[]), "Not: the group is on");
    assert!(render(not, &[5]), "Not: the group is off");
    let nested = "<< /Type /OCMD /VE [/And [/Not 5 0 R] 6 0 R] >>";
    assert!(render(nested, &[5]), "nested: A off, B on");
    assert!(!render(nested, &[]), "nested: A on");
    let both = "<< /Type /OCMD /OCGs [5 0 R] /P /AllOn /VE [/Not 5 0 R] >>";
    assert!(render(both, &[5]), "the expression, not /P, decides");
    // Malformed: an unknown operator, a wrong operand count, a bare group, a string operand,
    // a missing object, and the page instead of a group.
    for bad in ["[/Xor 5 0 R]", "[/Not 5 0 R 6 0 R]", "[/And]", "[5 0 R]", "[/And (5)]", "[/Or 8 0 R]", "[/Or 3 0 R]"] {
        let ocmd = format!("<< /Type /OCMD /OCGs [5 0 R] /VE {bad} >>");
        assert!(render(&ocmd, &[]), "{bad}: /OCGs on");
        assert!(!render(&ocmd, &[5]), "{bad}: /OCGs off");
    }
    let deep = format!("<< /Type /OCMD /OCGs [5 0 R] /VE {}5 0 R{} >>", "[/Not ".repeat(41), "]".repeat(41));
    assert!(!render(&deep, &[5]), "too deep: /OCGs decides");
}

/// PdfCraft ledger hayro (18) (#2972): a page's objects composite with a transparent backdrop
/// and the page group is then put on the paper, so a blend mode only blends where something
/// was drawn. A green fill with `/BM /Screen` (or Lighten, Difference…) half over a red square
/// takes the mode's color over the red and stays plain green over the empty page.
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn blend_modes_over_an_empty_part_of_the_page_keep_their_color() {
    for (mode, over_red) in [
        ("Normal", Some([0, 255, 0])),
        ("Multiply", Some([0, 0, 0])),
        ("Screen", Some([255, 255, 0])),
        ("Lighten", Some([255, 255, 0])),
        ("Difference", Some([255, 255, 0])),
        ("Exclusion", Some([255, 255, 0])),
        ("ColorDodge", Some([255, 0, 0])),
        ("Overlay", Some([255, 0, 0])),
        // The non-separable modes mix hue, saturation and luminosity; over an empty part of
        // the page they too keep the source color.
        ("Hue", None),
        ("Saturation", None),
        ("Color", None),
        ("Luminosity", None),
    ] {
        let page = format!("/MediaBox [0 0 300 200] /Resources << /ExtGState << /G1 << /Type /ExtGState /BM /{mode} >> >> >>");
        let p = render(&one_page(&page, b"1 0 0 rg 20 20 150 150 re f /G1 gs 0 1 0 rg 100 50 150 120 re f", b""));
        // Pixel rows run top down: PDF (210, 100) and (130, 100) are rows 100 from the top.
        assert_eq!(pixel(&p, 210, 100), [0, 255, 0, 255], "{mode} over the empty page");
        if let Some(over_red) = over_red {
            assert_eq!(pixel(&p, 130, 100)[..3], over_red, "{mode} over the red square");
        }
        assert_eq!(pixel(&p, 280, 10), [255, 255, 255, 255], "{mode}: the paper stays white");
    }
}

/// PdfCraft #3018 (ISO 32000-2 §8.6.5.6): the `/DefaultGray`, `/DefaultRGB` and `/DefaultCMYK`
/// color spaces of a resource dictionary stand in for the device spaces its content selects,
/// with color operators, images and shadings. The defaults are synthetic ICC profiles that send
/// device color somewhere it cannot go uncalibrated: red to green (or blue), black to mid gray,
/// black ink to blue.
mod default_color_spaces {
    use super::{Pixels, render};

    const WIDTH: usize = 130;
    pub(super) const RED: [u8; 3] = [255, 0, 0];
    const GREEN: [u8; 3] = [0, 255, 0];
    pub(super) const BLUE: [u8; 3] = [0, 0, 255];
    const BLACK: [u8; 3] = [0, 0, 0];
    /// Device black through `gray_profile`: half the luminance of white, sRGB-encoded.
    const MID_GRAY: [u8; 3] = [188, 188, 188];
    /// Full black ink through `cmyk_profile`: CIELAB (40, 20, -60) relative to D50.
    const INK_BLUE: [u8; 3] = [67, 86, 194];
    const ONE_PIXEL_RGB: &str = "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length 3 >>";

    /// An s15Fixed16Number (ICC.1 §4.6).
    fn s15f16(value: f64) -> [u8; 4] {
        ((value * 65536.0).round() as i32).to_be_bytes()
    }

    /// An ICC v2 profile of device class `class` for the color space `space`, with `tags`
    /// (signature, data) in that order. The device model stays blank: hayro skips the transform
    /// of a profile whose model reads `sRGB`.
    fn icc(class: &[u8; 4], space: &[u8; 4], pcs: &[u8; 4], tags: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
        let start = 128 + 4 + 12 * tags.len();
        let (mut table, mut data) = (Vec::new(), Vec::new());
        for (signature, tag) in tags {
            table.extend_from_slice(signature);
            table.extend_from_slice(&u32::try_from(start + data.len()).unwrap().to_be_bytes());
            table.extend_from_slice(&u32::try_from(tag.len()).unwrap().to_be_bytes());
            data.extend_from_slice(tag);
            data.resize(data.len().next_multiple_of(4), 0);
        }
        let mut profile = vec![0; 128];
        profile[0..4].copy_from_slice(&u32::try_from(start + data.len()).unwrap().to_be_bytes());
        profile[8..12].copy_from_slice(&[2, 0x10, 0, 0]);
        profile[12..16].copy_from_slice(class);
        profile[16..20].copy_from_slice(space);
        profile[20..24].copy_from_slice(pcs);
        profile[36..40].copy_from_slice(b"acsp");
        for (i, value) in [0.9642, 1.0, 0.8249].into_iter().enumerate() {
            profile[68 + 4 * i..72 + 4 * i].copy_from_slice(&s15f16(value));
        }
        profile.extend_from_slice(&u32::try_from(tags.len()).unwrap().to_be_bytes());
        profile.extend(table);
        profile.extend(data);
        profile
    }

    fn xyz([x, y, z]: [f64; 3]) -> Vec<u8> {
        [b"XYZ ".as_slice(), &[0; 4], &s15f16(x), &s15f16(y), &s15f16(z)].concat()
    }

    /// A `curveType` tag: one entry is a gamma (u8Fixed8), more are points spread evenly over 0..1.
    fn curve(points: &[u16]) -> Vec<u8> {
        let mut tag = [b"curv".as_slice(), &[0; 4], &u32::try_from(points.len()).unwrap().to_be_bytes()].concat();
        for point in points {
            tag.extend_from_slice(&point.to_be_bytes());
        }
        tag
    }

    /// sRGB's red, green and blue colorants, adapted to D50.
    const SRGB_COLORANTS: [[f64; 3]; 3] = [[0.4361, 0.2225, 0.0139], [0.3851, 0.7169, 0.0971], [0.1431, 0.0606, 0.7141]];

    /// A linear RGB profile whose red, green and blue take the sRGB colorants `primaries` names:
    /// with `[1, 2, 0]` red is drawn as sRGB green.
    pub(super) fn rgb_profile(primaries: [usize; 3]) -> Vec<u8> {
        let mut tags = Vec::new();
        for (signature, primary) in [*b"rXYZ", *b"gXYZ", *b"bXYZ"].into_iter().zip(primaries) {
            tags.push((signature, xyz(SRGB_COLORANTS[primary])));
        }
        for signature in [*b"rTRC", *b"gTRC", *b"bTRC"] {
            tags.push((signature, curve(&[0x0100])));
        }
        icc(b"mntr", b"RGB ", b"XYZ ", &tags)
    }

    /// A gray profile whose tone curve rises from half the luminance of white, so `0 g` is mid gray.
    fn gray_profile() -> Vec<u8> {
        icc(b"mntr", b"GRAY", b"XYZ ", &[(*b"kTRC", curve(&[0x8000, 0xffff]))])
    }

    /// A CMYK profile (`lut8Type` into CIELAB) that draws full black ink as L*a*b* (40, 20, -60)
    /// and no black ink as white, whatever the other inks.
    fn cmyk_profile() -> Vec<u8> {
        let mut lut = [b"mft1".as_slice(), &[0; 4], &[4, 3, 2, 0]].concat();
        for value in [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0] {
            lut.extend_from_slice(&s15f16(value));
        }
        for _ in 0..4 {
            lut.extend(0..=255_u8);
        }
        // Two grid points per ink, black varying fastest; Lab encoded as L* · 2.55, a* + 128, b* + 128.
        for point in 0..16 {
            lut.extend_from_slice(if point % 2 == 1 { &[102, 148, 68] } else { &[255, 128, 128] });
        }
        for _ in 0..3 {
            lut.extend(0..=255_u8);
        }
        icc(b"prtr", b"CMYK", b"Lab ", &[(*b"A2B0", lut)])
    }

    pub(super) fn profile_stream(n: u8, profile: Vec<u8>) -> (String, Vec<u8>) {
        (format!("<< /N {n} /Length {} >>", profile.len()), profile)
    }

    /// A form XObject covering the page, with the dictionary entries `extra`.
    fn form(extra: &str, content: &str) -> (String, Vec<u8>) {
        (format!("<< /Type /XObject /Subtype /Form /BBox [0 0 130 70] {extra} /Length {} >>", content.len()), content.as_bytes().to_vec())
    }

    /// A 130 × 70 pt page that draws `content` with `resources`, whose objects 5, 6, … are the
    /// streams `objects`.
    pub(super) fn page_pdf(content: &str, resources: &str, objects: &[(String, Vec<u8>)]) -> Vec<u8> {
        let mut pdf = format!(
            "%PDF-1.4\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n\
             2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n\
             3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 130 70] /Contents 4 0 R /Resources {resources} >> endobj\n\
             4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n",
            content.len()
        )
        .into_bytes();
        for (i, (dict, data)) in objects.iter().enumerate() {
            pdf.extend(format!("{} 0 obj {dict} stream\n", i + 5).bytes());
            pdf.extend_from_slice(data);
            pdf.extend_from_slice(b"\nendstream endobj\n");
        }
        pdf.extend_from_slice(b"trailer << /Root 1 0 R >>\n%%EOF");
        pdf
    }

    /// Draw `content` with `resources` on a 130 × 70 pt page whose objects 5, 6, … are the
    /// streams `objects`, and give the color at each of `points` (from the top left).
    pub(super) fn colors(content: &str, resources: &str, objects: &[(String, Vec<u8>)], points: &[(usize, usize)]) -> Vec<[u8; 3]> {
        let Pixels { width, height, rgba } = render(&page_pdf(content, resources, objects));
        assert_eq!((width as usize, height), (WIDTH, 70));
        points.iter().map(|&(x, y)| rgba[(y * WIDTH + x) * 4..][..3].try_into().unwrap()).collect()
    }

    #[track_caller]
    pub(super) fn assert_near(got: [u8; 3], want: [u8; 3], what: &str) {
        assert!(got.iter().zip(want).all(|(got, want)| got.abs_diff(want) <= 6), "{what}: got {got:?}, expected about {want:?}");
    }

    /// Device CMYK black ink through hayro's own CMYK profile: near black.
    #[track_caller]
    fn assert_dark(got: [u8; 3], what: &str) {
        assert!(got.iter().all(|c| *c < 90), "{what}: got {got:?}, expected near black");
    }

    /// `rg`, `RG`, `/DeviceRGB cs` and a `/ColorSpace` resource naming DeviceRGB all draw in the
    /// page's DefaultRGB; without one they draw device red.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn device_rgb_operators_draw_in_default_rgb() {
        let content = "1 0 0 rg 0 0 30 70 re f /DeviceRGB cs 1 0 0 sc 32 0 30 70 re f /CS0 cs 1 0 0 sc 64 0 30 70 re f \
                       1 0 0 RG 20 w 111 0 m 111 70 l S";
        let points = [(15, 35), (47, 35), (79, 35), (111, 35)];
        let what = ["rg", "/DeviceRGB cs", "/CS0 cs (/DeviceRGB)", "RG"];
        let objects = [profile_stream(3, rgb_profile([1, 2, 0]))];
        let calibrated = colors(content, "<< /ColorSpace << /DefaultRGB [/ICCBased 5 0 R] /CS0 /DeviceRGB >> >>", &objects, &points);
        for (got, what) in calibrated.into_iter().zip(what) {
            assert_near(got, GREEN, &format!("{what} with DefaultRGB"));
        }
        let device = colors(content, "<< /ColorSpace << /CS0 /DeviceRGB >> >>", &objects, &points);
        for (got, what) in device.into_iter().zip(what) {
            assert_eq!(got, RED, "{what} without DefaultRGB");
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn device_cmyk_operators_draw_in_default_cmyk() {
        let content = "0 0 0 1 k 0 0 40 70 re f /DeviceCMYK cs 0 0 0 1 sc 45 0 40 70 re f 0 0 0 1 K 20 w 110 0 m 110 70 l S";
        let points = [(20, 35), (65, 35), (110, 35)];
        let what = ["k", "/DeviceCMYK cs", "K"];
        let objects = [profile_stream(4, cmyk_profile())];
        let calibrated = colors(content, "<< /ColorSpace << /DefaultCMYK [/ICCBased 5 0 R] >> >>", &objects, &points);
        for (got, what) in calibrated.into_iter().zip(what) {
            assert_near(got, INK_BLUE, &format!("{what} with DefaultCMYK"));
        }
        for (got, what) in colors(content, "<< >>", &objects, &points).into_iter().zip(what) {
            assert_dark(got, &format!("{what} without DefaultCMYK"));
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn device_gray_operators_draw_in_default_gray() {
        let content = "0 g 0 0 40 70 re f /DeviceGray cs 0 sc 45 0 40 70 re f 0 G 20 w 110 0 m 110 70 l S";
        let points = [(20, 35), (65, 35), (110, 35)];
        let what = ["g", "/DeviceGray cs", "G"];
        let objects = [profile_stream(1, gray_profile())];
        let calibrated = colors(content, "<< /ColorSpace << /DefaultGray [/ICCBased 5 0 R] >> >>", &objects, &points);
        for (got, what) in calibrated.into_iter().zip(what) {
            assert_near(got, MID_GRAY, &format!("{what} with DefaultGray"));
        }
        for (got, what) in colors(content, "<< >>", &objects, &points).into_iter().zip(what) {
            assert_eq!(got, BLACK, "{what} without DefaultGray");
        }
    }

    /// Image XObjects and inline images (whose `/RGB` and `/G` abbreviate the device spaces, or
    /// that name a `/ColorSpace` resource that is DeviceRGB) in a device space draw in its default.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn device_images_draw_in_the_default_color_spaces() {
        let content = "q 20 0 0 70 0 0 cm /R Do Q q 20 0 0 70 22 0 cm /G Do Q q 20 0 0 70 44 0 cm /K Do Q \
                       q 20 0 0 70 66 0 cm BI /W 1 /H 1 /CS /RGB /BPC 8 /F /AHx ID ff0000> EI Q \
                       q 20 0 0 70 88 0 cm BI /W 1 /H 1 /CS /CS0 /BPC 8 /F /AHx ID ff0000> EI Q \
                       q 20 0 0 70 110 0 cm BI /W 1 /H 1 /CS /G /BPC 8 /F /AHx ID 00> EI Q";
        let points = [(10, 35), (32, 35), (54, 35), (76, 35), (98, 35), (120, 35)];
        let image = |space: &str, data: &[u8]| {
            let dict =
                format!("<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /{space} /BitsPerComponent 8 /Length {} >>", data.len());
            (dict, data.to_vec())
        };
        let objects = [
            profile_stream(3, rgb_profile([1, 2, 0])),
            profile_stream(1, gray_profile()),
            profile_stream(4, cmyk_profile()),
            image("DeviceRGB", &[255, 0, 0]),
            image("DeviceGray", &[0]),
            image("DeviceCMYK", &[0, 0, 0, 255]),
        ];
        let x_objects = "/XObject << /R 8 0 R /G 9 0 R /K 10 0 R >>";
        let defaults = "/DefaultRGB [/ICCBased 5 0 R] /DefaultGray [/ICCBased 6 0 R] /DefaultCMYK [/ICCBased 7 0 R]";
        let calibrated = colors(content, &format!("<< /ColorSpace << {defaults} /CS0 /DeviceRGB >> {x_objects} >>"), &objects, &points);
        assert_near(calibrated[0], GREEN, "DeviceRGB image");
        assert_near(calibrated[1], MID_GRAY, "DeviceGray image");
        assert_near(calibrated[2], INK_BLUE, "DeviceCMYK image");
        assert_near(calibrated[3], GREEN, "inline /RGB image");
        assert_near(calibrated[4], GREEN, "inline image in /CS0 (/DeviceRGB)");
        assert_near(calibrated[5], MID_GRAY, "inline /G image");

        let device = colors(content, &format!("<< /ColorSpace << /CS0 /DeviceRGB >> {x_objects} >>"), &objects, &points);
        assert_eq!(device[0], RED, "DeviceRGB image without DefaultRGB");
        assert_eq!(device[1], BLACK, "DeviceGray image without DefaultGray");
        assert_dark(device[2], "DeviceCMYK image without DefaultCMYK");
        assert_eq!(device[3], RED, "inline /RGB image without DefaultRGB");
        assert_eq!(device[4], RED, "inline /CS0 image without DefaultRGB");
        assert_eq!(device[5], BLACK, "inline /G image without DefaultGray");
    }

    /// Shadings in a device space, painted by `sh` or as a shading pattern, with colors from a
    /// function (axial) or from their vertices (a Type 4 mesh), draw in its default.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn device_shadings_draw_in_the_default_color_spaces() {
        let axial = |space: &str, color: &str| {
            format!(
                "<< /ShadingType 2 /ColorSpace /{space} /Coords [0 0 130 0] /Extend [true true] \
                 /Function << /FunctionType 2 /Domain [0 1] /C0 [{color}] /C1 [{color}] /N 1 >> >>"
            )
        };
        // Two red triangles over the page: a flag, x, y and red, green and blue, a byte each.
        let mut mesh: Vec<u8> = Vec::new();
        for (x, y) in [(0, 0), (255, 0), (0, 255), (255, 0), (255, 255), (0, 255)] {
            mesh.extend_from_slice(&[0, x, y, 255, 0, 0]);
        }
        let objects = [
            profile_stream(3, rgb_profile([1, 2, 0])),
            profile_stream(1, gray_profile()),
            (
                format!(
                    "<< /ShadingType 4 /ColorSpace /DeviceRGB /BitsPerCoordinate 8 /BitsPerComponent 8 /BitsPerFlag 8 \
                     /Decode [0 255 0 255 0 1 0 1 0 1] /Length {} >>",
                    mesh.len()
                ),
                mesh,
            ),
        ];
        let content = "q 0 0 30 70 re W n /A sh Q /Pattern cs /P scn 32 0 30 70 re f \
                       q 64 0 30 70 re W n /M sh Q q 96 0 30 70 re W n /G sh Q";
        let points = [(15, 35), (47, 35), (79, 35), (111, 35)];
        let shadings = format!(
            "/Shading << /A {} /M 7 0 R /G {} >> /Pattern << /P << /PatternType 2 /Shading {} >> >>",
            axial("DeviceRGB", "1 0 0"),
            axial("DeviceGray", "0"),
            axial("DeviceRGB", "1 0 0")
        );
        let calibrated = colors(
            content,
            &format!("<< /ColorSpace << /DefaultRGB [/ICCBased 5 0 R] /DefaultGray [/ICCBased 6 0 R] >> {shadings} >>"),
            &objects,
            &points,
        );
        assert_near(calibrated[0], GREEN, "sh, an axial shading in DeviceRGB");
        assert_near(calibrated[1], GREEN, "a shading pattern in DeviceRGB");
        assert_near(calibrated[2], GREEN, "sh, a Type 4 mesh in DeviceRGB");
        assert_near(calibrated[3], MID_GRAY, "sh, an axial shading in DeviceGray");

        let device = colors(content, &format!("<< {shadings} >>"), &objects, &points);
        assert_near(device[0], RED, "sh, an axial shading without DefaultRGB");
        assert_near(device[1], RED, "a shading pattern without DefaultRGB");
        assert_near(device[2], RED, "sh, a Type 4 mesh without DefaultRGB");
        assert_near(device[3], BLACK, "sh, an axial shading without DefaultGray");
    }

    /// One image drawn by a page with DefaultRGB and by a form that switches it off is decoded
    /// for each: the decoded-image cache keys the default.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn an_image_drawn_with_and_without_a_default_keeps_both_colors() {
        let objects = [
            profile_stream(3, rgb_profile([1, 2, 0])),
            (ONE_PIXEL_RGB.to_owned(), vec![255, 0, 0]),
            form("/Resources << /ColorSpace << /DefaultRGB /DeviceRGB >> /XObject << /R 6 0 R >> >>", "q 40 0 0 70 70 0 cm /R Do Q"),
        ];
        let got = colors(
            "q 40 0 0 70 0 0 cm /R Do Q /F Do",
            "<< /ColorSpace << /DefaultRGB [/ICCBased 5 0 R] >> /XObject << /R 6 0 R /F 7 0 R >> >>",
            &objects,
            &[(20, 35), (90, 35)],
        );
        assert_near(got[0], GREEN, "the image on the page, in DefaultRGB");
        assert_eq!(got[1], RED, "the same image in the form without DefaultRGB");
    }

    /// A form XObject draws device color in the defaults of the resources in its scope: the
    /// page's when it has no `/Resources` (directly or nested in another form), else only its
    /// own, as for every other resource here. PdfCraft, Poppler and MuPDF also fall back to the
    /// page's default from a form with `/Resources` of its own. A form whose own default is a
    /// device space or not a usable default draws device color.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn forms_draw_in_the_default_color_spaces_of_their_resource_scope() {
        let blue = rgb_profile([2, 0, 1]);
        let objects = [
            profile_stream(3, rgb_profile([1, 2, 0])),
            // hayro caches a profile by its stream's dictionary, which would otherwise match the first.
            (format!("<< /N 3 /Alternate /DeviceRGB /Length {} >>", blue.len()), blue),
            form("", "1 0 0 rg 22 37 20 33 re f"),
            form("/Resources << /ProcSet [/PDF] >>", "1 0 0 rg 44 37 20 33 re f"),
            form("/Resources << /ColorSpace << /DefaultRGB [/ICCBased 6 0 R] >> /XObject << /N 12 0 R >> >>", "1 0 0 rg 66 37 20 33 re f /N Do"),
            form("/Resources << /ColorSpace << /DefaultRGB /DeviceRGB >> >>", "1 0 0 rg 88 37 20 33 re f"),
            form("/Resources << /ColorSpace << /DefaultRGB [/Lab << /WhitePoint [0.9642 1 0.8249] >>] >> >>", "1 0 0 rg 110 37 20 33 re f"),
            form("", "1 0 0 rg 0 0 20 33 re f"),
            form("/Resources << /ColorSpace << /CS1 /DeviceGray >> >>", "1 0 0 rg 22 0 20 33 re f"),
        ];
        let got = colors(
            "1 0 0 rg 0 37 20 33 re f /A Do /B Do /C Do /D Do /E Do /F Do",
            "<< /ColorSpace << /DefaultRGB [/ICCBased 5 0 R] >> /XObject << /A 7 0 R /B 8 0 R /C 9 0 R /D 10 0 R /E 11 0 R /F 13 0 R >> >>",
            &objects,
            &[(10, 17), (32, 17), (54, 17), (76, 17), (98, 17), (120, 17), (10, 53), (32, 53)],
        );
        assert_near(got[0], GREEN, "the page");
        assert_near(got[1], GREEN, "a form without /Resources");
        assert_eq!(got[2], RED, "a form whose /Resources has no /ColorSpace");
        assert_near(got[3], BLUE, "a form with a DefaultRGB of its own");
        assert_eq!(got[4], RED, "a form whose DefaultRGB is /DeviceRGB");
        assert_eq!(got[5], RED, "a form whose DefaultRGB is a Lab space");
        assert_near(got[6], BLUE, "a form nested in the one with its own DefaultRGB");
        assert_eq!(got[7], RED, "a form whose /ColorSpace has no DefaultRGB");
    }

    /// A default that cannot stand in for its device space (Lab, Indexed, Pattern, another device
    /// space, the wrong number of components, or no color space) leaves device color as it is,
    /// and the dictionary's other defaults still apply.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn unusable_defaults_leave_device_color_alone() {
        let objects = [
            profile_stream(1, gray_profile()),
            profile_stream(4, cmyk_profile()),
            (String::from("<< /N 3 /Length 13 >>"), b"not a profile".to_vec()),
        ];
        for default in [
            "[/Lab << /WhitePoint [0.9642 1 0.8249] >>]",
            "[/CalGray << /WhitePoint [0.9642 1 0.8249] >>]",
            "[/Indexed /DeviceRGB 0 <00FF00>]",
            "/Pattern",
            "[/Pattern /DeviceRGB]",
            "/DeviceCMYK",
            "/DeviceGray",
            "[/ICCBased 6 0 R]",
            "[/ICCBased 7 0 R]",
            "42",
            "[/NoSuchSpace]",
            "/DefaultRGB",
        ] {
            let got = colors(
                "1 0 0 rg 0 0 60 70 re f 0 g 70 0 60 70 re f",
                &format!("<< /ColorSpace << /DefaultRGB {default} /DefaultGray [/ICCBased 5 0 R] >> >>"),
                &objects,
                &[(30, 35), (100, 35)],
            );
            assert_eq!(got[0], RED, "DefaultRGB {default}: device RGB is drawn as it is");
            assert_near(got[1], MID_GRAY, &format!("DefaultRGB {default}: DefaultGray still applies"));
        }
    }
}

/// PdfCraft ledger hayro-interpret (48) and the ICC profile cache key (#3020): two `/ICCBased`
/// profiles whose stream dictionaries are byte for byte the same (`/N 3` and one `/Length`)
/// each draw through their own profile. hayro cached the color space it read from a profile
/// stream by the dictionary's bytes alone, so the second profile drew through whichever was
/// read first. The profiles are linear sRGB and the same profile with red and blue swapped,
/// which draws red as blue; they have the same length.
mod icc_profile_cache {
    use super::default_color_spaces::{BLUE, RED, assert_near, colors, page_pdf, profile_stream, rgb_profile};
    use craft_pdf::interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        TransformExt, font::Glyph,
    };
    use kurbo::{Affine, BezPath, Rect};

    /// An `/ICCBased` space whose profile stream is written directly in the space's array, not
    /// as an object of its own: the profile as ASCIIHex digits, each a token of its own, in
    /// upper case (a lone `f` reads as a broken `false`). Its dictionary reports the object
    /// holding the array, which is the page for every one of them.
    fn direct_space(profile: &[u8]) -> String {
        let hex: String = profile.iter().flat_map(|b| [b >> 4, b & 0xf]).map(|d| format!("{d:X} ")).collect();
        format!("[/ICCBased << /N 3 /Filter /ASCIIHexDecode /Length {} >> stream\n{hex}\nendstream]", hex.len())
    }

    /// Six 20 pt squares, side by side, filled with `1 0 0 sc` (red) in `/A` (object 5, linear
    /// sRGB), `/B` (object 6, red and blue swapped), `/A2` (object 5 again, through an array of
    /// other bytes), `/A3` (object 7, a copy of object 5), `/C` and `/D` (the two profiles
    /// again, each written directly in its array). Objects 5, 6 and 7 have the same dictionary,
    /// and so do the two direct streams.
    fn page() -> (String, String, Vec<(String, Vec<u8>)>) {
        let (srgb, swapped) = (rgb_profile([0, 1, 2]), rgb_profile([2, 1, 0]));
        assert_eq!(srgb.len(), swapped.len(), "the profiles have the same length, so the same dictionary");
        assert_ne!(srgb, swapped);
        let resources = format!(
            "<< /ColorSpace << /A [/ICCBased 5 0 R] /B [/ICCBased 6 0 R] /A2 [ /ICCBased 5 0 R ] /A3 [/ICCBased 7 0 R] /C {} /D {} >> >>",
            direct_space(&srgb),
            direct_space(&swapped)
        );
        let content = ["A", "B", "A2", "A3", "C", "D"]
            .iter()
            .enumerate()
            .map(|(i, name)| format!("/{name} cs 1 0 0 sc {} 20 20 30 re f", 2 + 21 * i))
            .collect::<Vec<_>>()
            .join("\n");
        (content, resources, vec![profile_stream(3, srgb.clone()), profile_stream(3, swapped), profile_stream(3, srgb)])
    }

    /// Each square is drawn through its own profile, whether the profile is an object or
    /// written in its array, and object 5 looked up again and its copy still draw red.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn icc_profiles_with_identical_dictionaries_draw_their_own_colors() {
        let (content, resources, objects) = page();
        let points: Vec<_> = (0..6).map(|i| (12 + 21 * i, 35)).collect();
        let got = colors(&content, &resources, &objects, &points);
        let expected = [
            ("/A, object 5, linear sRGB", RED),
            ("/B, object 6, red and blue swapped", BLUE),
            ("/A2, object 5 again", RED),
            ("/A3, object 7, a copy of object 5", RED),
            ("/C, linear sRGB written in its array", RED),
            ("/D, red and blue swapped written in its array", BLUE),
        ];
        for (got, (what, want)) in got.into_iter().zip(expected) {
            assert_near(got, want, what);
        }
    }

    /// Draws nothing: only the color spaces the page selects matter here.
    struct Nothing;
    impl<'a> Device<'a> for Nothing {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'a>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'a>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'a>, _: Affine, _: Affine, _: &Paint<'a>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, _: Image<'a, '_>, _: Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    /// The profiles are still cached: the four different ones (objects 5 and 6, and the two
    /// written in their arrays) are read once each, and neither object 5 looked up again nor its
    /// copy, object 7, is read again. A second pass through the same cache reads none.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn icc_profiles_used_again_are_read_once() {
        let (content, resources, objects) = page();
        let bytes = page_pdf(&content, &resources, &objects);
        let pdf = craft_pdf::open_slice(&bytes, None).unwrap();
        let page = &pdf.pages()[0];
        let cache = InterpreterCache::new();
        for pass in ["first", "second"] {
            let initial = page.initial_transform(true).to_kurbo();
            let mut ctx = Context::new(initial, Rect::new(0.0, 0.0, 130.0, 70.0), &cache, page.xref(), InterpreterSettings::default());
            craft_pdf::interpret_page(page, &mut ctx, &mut Nothing).unwrap();
            assert_eq!(cache.icc_profile_reads(), 4, "after the {pass} pass: each different profile is read once");
        }
    }
}
