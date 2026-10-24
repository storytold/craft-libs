//! Portable regressions adapted from PdfCraft 7437bde378f43856eada40af25b983812e615e8a and
//! a2ec27b4775f53c40cad3456936d9406611e3a8d (MIT OR Apache-2.0).
#![cfg(feature = "raster")]
use craft_pdf::{
    open_slice,
    raster::{self, Options, RenderCache},
};

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
