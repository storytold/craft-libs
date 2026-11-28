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

// These frozen RGB8 values were computed outside the renderer in f64 from the
// CIE Lab inverse, the full XYZ adaptation matrix B^-1 diag(B*D65 / B*white) B,
// and the standard sRGB matrix/transfer function. No production helper is used.
// Bradford reference: https://www.color.org/chadtag/ (ICC TN 02-2003).
// White points: D50 [0.96422, 1, 0.82521], E [1, 1, 1],
// D65 [0.95047, 1, 1.08883]. D65 adaptation is the identity.
const SAMPLES: [[f64; 3]; 4] = [[50.0, 0.0, 0.0], [40.0, 20.0, -60.0], [70.0, -50.0, 30.0], [100.0, 0.0, 0.0]];
const D50: [[u8; 3]; 4] = [[119, 119, 119], [67, 86, 194], [66, 194, 113], [255, 255, 255]];

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

fn pixel(page: &Pixels, x: u32, y: u32) -> &[u8] {
    let i = ((y * page.width + x) * 4) as usize;
    &page.rgba[i..i + 4]
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

/// Synthetic ICC profiles and colour sampling for the ICC profile cache trial.
mod default_color_spaces {
    use super::{Pixels, render};

    const WIDTH: usize = 130;
    pub(super) const RED: [u8; 3] = [255, 0, 0];
    pub(super) const BLUE: [u8; 3] = [0, 0, 255];

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

    pub(super) fn profile_stream(n: u8, profile: Vec<u8>) -> (String, Vec<u8>) {
        (format!("<< /N {n} /Length {} >>", profile.len()), profile)
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
}

/// PdfCraft ledger hayro-interpret (48) and the ICC profile cache key (#3020): two `/ICCBased`
/// profiles whose stream dictionaries are byte for byte the same (`/N 3` and one `/Length`)
/// each draw through their own profile. hayro cached the color space it read from a profile
/// stream by the dictionary's bytes alone, so the second profile drew through whichever was
/// read first. The profiles are linear sRGB and the same profile with red and blue swapped,
/// which draws red as blue; they have the same length.
mod icc_profile_cache {
    use super::default_color_spaces::{BLUE, RED, assert_near, colors, profile_stream, rgb_profile};

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
}

/// PdfCraft #5643 (storytold/pdfcraft#5532), the thin-stroke pen in `hayro`'s renderer. Adapted
/// from `raster::tests::{a_thin_stroke_is_one_pixel_wide_under_a_squeezed_matrix,
/// a_dashed_thin_stroke_keeps_the_miters_its_dashes_draw}` in pdfcraft-render at
/// a8d2d7967 (MIT OR Apache-2.0).
mod thin_strokes {
    use super::{one_page, render};

    /// The ink of each pixel of `content` drawn on a 40 × 40 pt page at 72 dpi, row by row
    /// (0 white, 1 black), and the page's width in pixels.
    fn ink_page(content: &str) -> (usize, Vec<f32>) {
        ink_on("/MediaBox [0 0 40 40]", content)
    }

    /// The ink of each pixel of `content` drawn on a page with the entries `page` at 72 dpi.
    fn ink_on(page: &str, content: &str) -> (usize, Vec<f32>) {
        let page = render(&one_page(page, content.as_bytes(), b""));
        let ink = page.rgba.as_chunks::<4>().0.iter().map(|px| f32::from(255 - px[0]) / 255.0).collect();
        (page.width as usize, ink)
    }

    /// A stroke thinner than a device pixel is drawn at least one pixel wide whichever way it
    /// runs, also under a matrix that shrinks one axis more than the other. The widening went by
    /// the larger axis scale alone, so under `1 0 0 0.25 cm` a horizontal line came out a
    /// quarter of a pixel thick and a quarter as dark; going by the smaller scale instead would
    /// have drawn a vertical line four pixels wide. Dashes keep the lengths the pattern gives
    /// them.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_thin_stroke_is_one_pixel_wide_under_a_squeezed_matrix() {
        let column = |content: &str, x: usize| {
            let (width, ink) = ink_page(content);
            ink.iter().skip(x).step_by(width).sum::<f32>()
        };
        let row = |content: &str, y: usize| {
            let (width, ink) = ink_page(content);
            ink.iter().skip(y * width).take(width).copied().collect::<Vec<f32>>()
        };
        let one_pixel = 0.85..=1.3;

        // Device y = 40 - 0.25 × 82 = 19.5, the middle of row 19.
        let across = column("q 1 0 0 0.25 0 0 cm 0.3 w 4 82 m 36 82 l S Q", 20);
        assert!(one_pixel.contains(&across), "a horizontal line is {across} px of ink thick");
        // Device x = 20.5, the middle of column 20, from row 10 to row 30.
        let along: f32 = row("q 1 0 0 0.25 0 0 cm 0.3 w 20.5 40 m 20.5 120 l S Q", 20).iter().sum();
        assert!(one_pixel.contains(&along), "a vertical line is {along} px of ink wide");
        // Shrunk on both axes, by different scales.
        let both = column("q 0.5 0 0 0.25 0 0 cm 0.1 w 8 82 m 72 82 l S Q", 20);
        assert!(one_pixel.contains(&both), "a line shrunk on both axes is {both} px thick");
        // As before: an unscaled hairline is one pixel thick.
        let plain = column("0.1 w 4 20.5 m 36 20.5 l S", 20);
        assert!(one_pixel.contains(&plain), "a plain hairline is {plain} px thick");
        // A line width of zero, the thinnest line the device can draw (ISO 32000-2 §8.4.3.2), is
        // the same: one pixel with the matrix or without it, whichever way the line runs.
        let zero = column("0 w 4 20.5 m 36 20.5 l S", 20);
        assert!(one_pixel.contains(&zero), "a zero-width line is {zero} px thick");
        let across = column("q 1 0 0 0.25 0 0 cm 0 w 4 82 m 36 82 l S Q", 20);
        assert!(one_pixel.contains(&across), "a zero-width horizontal line is {across} px thick");
        let along: f32 = row("q 1 0 0 0.25 0 0 cm 0 w 20.5 40 m 20.5 120 l S Q", 20).iter().sum();
        assert!(one_pixel.contains(&along), "a zero-width vertical line is {along} px wide");

        // Dashes of 4 pt, gaps of 4 pt from x = 4: columns 4–7 and 12–15 are inked, 8–11 are not.
        let dashes = row("q 1 0 0 0.25 0 0 cm 0.3 w [4 4] 0 d 4 82 m 36 82 l S Q", 19);
        let (on, off) = (dashes.get(5..7).unwrap_or_default(), dashes.get(9..11).unwrap_or_default());
        assert!(on.len() == 2 && on.iter().all(|i| *i > 0.85), "a dash is inked: {dashes:?}");
        assert!(off.len() == 2 && off.iter().all(|i| *i < 0.05), "a gap is not: {dashes:?}");
    }

    /// A dashed thin stroke takes its miters from the joins its dashes draw. Under
    /// `0.1 0 0 1 cm` a 2-unit line is widened across x, which sharpens a right angle pointing up
    /// (a miter in user space, under a limit of 4) and blunts a spike lying along x (a bevel in
    /// user space). No one limit keeps both, so with the spike drawn the right angle is
    /// bevelled; with the spike in a dash's gap it keeps its miter, whose tip reaches about 5
    /// pixels above the corner.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_dashed_thin_stroke_keeps_the_miters_its_dashes_draw() {
        // The right angle's corner is at device (10.8, 22); the spike's, in the gap, at (21.6, 20).
        let path = "100 10 m 108 18 l 116 10 l 216 20 l 116 30 l S Q";
        let tip = |content: &str| {
            let (width, ink) = ink_page(content);
            (17..=20).flat_map(|y| (8..=12).map(move |x| (x, y))).filter_map(|(x, y)| ink.get(y * width + x)).sum::<f32>()
        };
        let dashed = tip(&format!("q 0.1 0 0 1 0 0 cm 2 w 4 M [120 6 500 1] 0 d {path}"));
        assert!(dashed > 1.0, "the dashed right angle's miter tip has {dashed} px of ink");
        let solid = tip(&format!("q 0.1 0 0 1 0 0 cm 2 w 4 M {path}"));
        assert!(solid < 0.3, "with the spike drawn the right angle is bevelled: {solid} px of ink");
    }

    /// A stroke far wider than the canvas one way is drawn a few canvases wide, and its pen is
    /// sized for the width drawn. Under `100 0 0 0.01 cm` a 10-unit line is 1000 pixels wide
    /// across x, which is capped at 320 on a 40 × 40 canvas, and 0.1 across y. The pen was sized
    /// for the uncapped width, so the cap took a horizontal line down to 0.32 pixels thick.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_thin_stroke_capped_for_its_other_width_is_still_one_pixel_thick() {
        // Device y = 40 - 0.01 × 1950 = 20.5, the middle of row 20.
        let (width, ink) = ink_page("q 100 0 0 0.01 0 0 cm 10 w 0.04 1950 m 0.36 1950 l S Q");
        let across = ink.iter().skip(20).step_by(width).sum::<f32>();
        assert!((0.85..=1.3).contains(&across), "a capped horizontal line is {across} px of ink thick");
    }

    /// A thin stroke keeps its joins when its matrix turns a quarter. vello sets how closely
    /// kurbo strokes from the matrix's diagonal alone, which a quarter turn makes 0, and kurbo
    /// leaves out a join that turns by less than that tolerance allows for. Under
    /// `300 0 0 1 cm` a 0.1-unit line is 30 pixels wide across x and widened to 1 across y; the
    /// corner below turns by 85° in the pen's space. Turned, the join was left out, which cut
    /// about 14 pixels of ink off the corner.
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn a_thin_stroke_keeps_its_joins_under_a_quarter_turn() {
        let corner = "300 0 0 1 60 20 cm 0.1 w 0 0 m 0 20 l 0.1 20.087 l S";
        let total = |content: String| ink_on("/MediaBox [0 0 100 100]", &content).1.iter().sum::<f32>();
        let plain = total(format!("q {corner} Q"));
        // Turned a quarter about the page's centre.
        let turned = total(format!("q 0 1 -1 0 100 0 cm {corner} Q"));
        assert!(plain > 100.0, "the corner has {plain} px of ink");
        assert!((plain - turned).abs() < 0.5, "the corner has {plain} px of ink, turned {turned}");
    }
}
