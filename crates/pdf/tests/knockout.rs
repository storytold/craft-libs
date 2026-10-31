//! Knockout transparency groups through the raster path. Adapted from PdfCraft #2974 by mtlcl410
//! (`raster::tests::knockout_groups_replace_what_their_elements_cover` in pdfcraft-render at
//! bae69819594a6385a027fec1f67e6b3c36c259b4, MIT OR Apache-2.0).
#![cfg(feature = "raster")]

use craft_pdf::interpret::InterpreterSettings;
use craft_pdf::{
    open_slice,
    raster::{self, Options, RenderCache},
};

/// A 300 × 200 pt page that draws form `Fm1` (a transparency group with `group` entries and
/// content `form`), then a green square at 50 % opacity.
fn knockout_pdf(group: &str, form: &str) -> Vec<u8> {
    format!(
        "%PDF-1.7
1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj
2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj
3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R /Resources << /XObject << /Fm1 5 0 R >> /ExtGState << /G << /ca 0.5 >> >> >> >> endobj
4 0 obj << /Length 39 >> stream
/Fm1 Do /G gs 0 1 0 rg 10 10 30 30 re f
endstream endobj
5 0 obj << /Type /XObject /Subtype /Form /BBox [0 0 300 200] /Group << /S /Transparency {group} >> /Resources << /ExtGState << /A << /ca 0.5 >> >> /XObject << /In 6 0 R /Msk 7 0 R /Neu 8 0 R /Img 9 0 R /Sm 10 0 R /Nimg 12 0 R /Deep 13 0 R /Shg 16 0 R /Big 17 0 R >> >> /Length {} >> stream
{form}
endstream endobj
6 0 obj << /Type /XObject /Subtype /Form /BBox [0 0 300 200] /Group << /S /Transparency >> /Resources << /ExtGState << /A << /ca 0.5 >> >> >> /Length 34 >> stream
/A gs 0 0 1 rg 100 50 150 120 re f
endstream endobj
7 0 obj << /Type /XObject /Subtype /Image /Width 2 /Height 2 /ImageMask true /Filter /ASCIIHexDecode /Length 5 >> stream
0000>
endstream endobj
8 0 obj << /Type /XObject /Subtype /Form /BBox [0 0 300 200] /Group << /S /Transparency >> /Resources << /ExtGState << /H << /ca 0.5 >> >> >> /Length 62 >> stream
/H gs 0 1 0 rg 80 40 100 100 re f 0 0 1 rg 130 80 100 100 re f
endstream endobj
9 0 obj << /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length 7 >> stream
0000FF>
endstream endobj
10 0 obj << /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /SMask 11 0 R /Filter /ASCIIHexDecode /Length 13 >> stream
0000FF0000FF>
endstream endobj
11 0 obj << /Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length 5 >> stream
00FF>
endstream endobj
12 0 obj << /Type /XObject /Subtype /Form /BBox [0 0 300 200] /Group << /S /Transparency >> /Resources << /ExtGState << /H << /ca 0.5 >> >> /XObject << /Img 9 0 R >> >> /Length 38 >> stream
/H gs q 160 0 0 100 60 50 cm /Img Do Q
endstream endobj
13 0 obj << /Type /XObject /Subtype /Form /BBox [0 0 300 200] /Group << /S /Transparency >> /Resources << /ExtGState << /H << /ca 0.5 >> >> /XObject << /In3 14 0 R >> >> /Length 13 >> stream
/H gs /In3 Do
endstream endobj
14 0 obj << /Type /XObject /Subtype /Form /BBox [0 0 300 200] /Group << /S /Transparency >> /Length 28 >> stream
0 0 1 rg 100 50 150 120 re f
endstream endobj
15 0 obj << /PatternType 2 /Shading << /ShadingType 2 /ColorSpace /DeviceRGB /Coords [0 0 1 0] /Function << /FunctionType 2 /Domain [0 1] /C0 [0 0 1] /C1 [0 0 1] /N 1 >> /Extend [true true] /BBox [100 50 140 170] >> >> endobj
16 0 obj << /Type /XObject /Subtype /Form /BBox [0 0 300 200] /Group << /S /Transparency >> /Resources << /Pattern << /P 15 0 R >> >> /Length 38 >> stream
/Pattern cs /P scn 100 50 150 120 re f
endstream endobj
17 0 obj << /Type /XObject /Subtype /Image /Width 20000 /Height 20000 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /ASCIIHexDecode /Length 7 >> stream
0000FF>
endstream endobj
trailer << /Root 1 0 R >>
%%EOF",
        form.len()
    )
    .into_bytes()
}

/// The page rendered over white, as RGBA8, 300 × 200 pixels.
fn render(group: &str, form: &str) -> Vec<u8> {
    let pdf = knockout_pdf(group, form);
    let doc = open_slice(&pdf, None).unwrap();
    let options = Options { background: [255; 4], ..Default::default() };
    let raster = raster::render(&doc.pages()[0], &RenderCache::new(), &InterpreterSettings::default(), &options).unwrap();
    assert_eq!((raster.width, raster.height), (300, 200));
    raster.bytes().to_vec()
}

/// The RGB at PDF point (x, y); pixel rows run top down.
fn px(page: &[u8], x: usize, y: usize) -> [u8; 3] {
    let p = &page[((200 - y) * 300 + x) * 4..][..3];
    [p[0], p[1], p[2]]
}

fn check(page: &[u8], x: usize, y: usize, want: [u8; 3], what: &str) {
    let got = px(page, x, y);
    assert!(got.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 2), "{what}: {got:?}, expected {want:?}");
}

/// In a knockout transparency group (`/K true`) each element composites with the group's initial
/// backdrop, not with the elements drawn before it in the group, over the element's shape. A blue
/// square at 50 % opacity over a red one at 50 % shows only the blue where they overlap; without
/// `/K` the two blend. The same holds for an image (its shape is its rectangle; a soft-masked
/// image's shape is its mask, so its transparent part leaves what is under it) and for a nested
/// group (its shape is the union of its elements' shapes, while its own elements still blend with
/// each other).
#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn knockout_groups_replace_what_their_elements_cover() {
    let red = "/A gs 1 0 0 rg 20 20 150 150 re f ";
    let ko = "/I true /K true";

    let squares = render(ko, &format!("{red}0 0 1 rg 100 50 150 120 re f"));
    check(&squares, 130, 100, [128, 128, 255], "knocked out");
    check(&squares, 50, 40, [255, 128, 128], "red alone");
    check(&squares, 210, 100, [128, 128, 255], "blue alone");
    check(&squares, 280, 190, [255, 255, 255], "paper");
    // Page content after the group composites normally again: green at 50 % over the red.
    check(&squares, 30, 30, [128, 191, 64], "after the group");
    let plain = render("/I true", &format!("{red}0 0 1 rg 100 50 150 120 re f"));
    check(&plain, 130, 100, [128, 64, 191], "blended without /K");

    // An image at 50 % replaces the red under its rectangle.
    let image = render(ko, &format!("{red}q 160 0 0 100 60 50 cm /Img Do Q"));
    check(&image, 100, 100, [128, 128, 255], "image knocks out");
    check(&image, 40, 40, [255, 128, 128], "red beside the image");
    // A soft-masked image: its transparent left half leaves the red, its opaque half replaces it.
    let masked = render(ko, "1 0 0 rg 20 20 150 150 re f q 160 0 0 100 60 50 cm /Sm Do Q");
    check(&masked, 100, 100, [255, 0, 0], "under the transparent half");
    check(&masked, 155, 100, [0, 0, 255], "under the opaque half");

    // A nested group entered at 50 % (blue at 25 % in all) replaces the red under its shape.
    let nested = render(ko, &format!("{red}/In Do"));
    check(&nested, 130, 100, [191, 191, 255], "nested group knocks out");
    check(&nested, 50, 40, [255, 128, 128], "red beside the nested group");
    // A nested group at full opacity: it knocks out the red as a whole, and its own two squares
    // at 50 % still blend with each other where they overlap.
    let neutral = render(ko, "1 0 0 rg 20 20 150 150 re f /Neu Do");
    check(&neutral, 100, 60, [128, 255, 128], "green over the knocked-out red");
    check(&neutral, 150, 110, [64, 128, 191], "the nested squares blend");
    check(&neutral, 40, 40, [255, 0, 0], "red beside the nested group");

    // An image inside a nested group is drawn once (blue at 50 %), over the knocked-out red.
    let nested_image = render(ko, "1 0 0 rg 20 20 150 150 re f /Nimg Do");
    check(&nested_image, 100, 100, [128, 128, 255], "image in a nested group");
    // A group inside a nested group knocks out the red as well.
    let deep = render(ko, "1 0 0 rg 20 20 150 150 re f /Deep Do");
    check(&deep, 130, 100, [128, 128, 255], "group in a nested group");
    check(&deep, 40, 40, [255, 0, 0], "red beside it");

    // A stencil mask must not erase the red beside it. Where the stencil paints (every sample 0)
    // its blue is drawn; a stencil composites rather than knocking the red out, and its exact
    // color also depends on its fill opacity (pdfcraft #2975), so only "blue was drawn here" is
    // pinned.
    let stencil = render(ko, &format!("{red}0 0 1 rg q 150 0 0 120 100 50 cm /Msk Do Q"));
    check(&stencil, 50, 40, [255, 128, 128], "red beside a stencil");
    let under = px(&stencil, 130, 100);
    assert!(under != [255, 128, 128] && under[2] >= 150 && under[0] < 230, "stencil drew nothing: {under:?}");

    // A shading's /BBox clips it (ISO 32000-2 Table 77), so a shading fill's shape is its path
    // inside the box: the erase pass clears only that, and the red outside the box but inside
    // the path stays.
    let shaded = render(ko, &format!("{red}/Shg Do"));
    check(&shaded, 120, 100, [128, 128, 255], "the shading inside its box");
    check(&shaded, 155, 100, [255, 128, 128], "red outside the shading's box");

    // An image that isn't decoded (here over the renderer's pixel limit) draws nothing, so it
    // doesn't clear what is under its rectangle either.
    let undecoded = render(ko, &format!("{red}q 160 0 0 100 60 50 cm /Big Do Q"));
    check(&undecoded, 100, 100, [255, 128, 128], "red under an image that isn't drawn");
    // A tiny image still knocks out in a final render.
    let tiny = render(ko, &format!("{red}q 3 0 0 3 100 100 cm /Img Do Q"));
    check(&tiny, 101, 101, [128, 128, 255], "a tiny image");
}
