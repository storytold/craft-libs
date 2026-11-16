//! Observable resource, object-depth and text-paint compatibility contracts.
#![cfg(feature = "read")]
#![allow(clippy::unwrap_used, clippy::panic)]

use craft_pdf::syntax::object::{FromBytes, ObjRef, Object};

fn stream(dictionary: &str, content: &[u8]) -> Vec<u8> {
    let mut bytes = format!("<< {dictionary} /Length {} >>\nstream\n", content.len()).into_bytes();
    bytes.extend_from_slice(content);
    bytes.extend_from_slice(b"\nendstream");
    bytes
}

fn object(bytes: &mut Vec<u8>, number: usize, body: &[u8]) -> usize {
    let offset = bytes.len();
    bytes.extend_from_slice(format!("{number} 0 obj\n").as_bytes());
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(b"\nendobj\n");
    offset
}

fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let offsets: Vec<_> = objects.iter().enumerate().map(|(i, body)| object(&mut bytes, i + 1, body)).collect();
    let start = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(format!("trailer << /Size {} /Root 1 0 R >>\nstartxref\n{start}\n%%EOF\n", objects.len() + 1).as_bytes());
    bytes
}

fn nested_object(depth: usize, kind: usize) -> Vec<u8> {
    let mut value = "42".to_string();
    for level in 0..depth {
        value = if kind == 0 || kind == 2 && level % 2 == 0 { format!("[{value}]") } else { format!("<< /V {value} >>") };
    }
    value.into_bytes()
}

fn assert_nested_leaf(mut value: Object<'_>, depth: usize) {
    for _ in 0..depth {
        value = match value {
            Object::Array(array) => array.iter::<Object<'_>>().next().unwrap(),
            Object::Dict(dict) => dict.get::<Object<'_>>(b"V").unwrap(),
            _ => panic!("expected another direct container"),
        };
    }
    assert_eq!(value.into_i32(), Some(42));
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn direct_objects_accept_64_containers_and_reject_65() {
    for kind in 0..3 {
        let accepted = nested_object(64, kind);
        assert_nested_leaf(Object::from_bytes(&accepted).unwrap(), 64);
        assert!(Object::from_bytes(&nested_object(65, kind)).is_none(), "container kind {kind}");
    }
}

// Object 6 exists only inside an ASCIIHex-decoded object stream. A real xref stream
// selects the compressed object, exercising its parser rather than a direct-byte helper.
fn compressed_object(value: &[u8]) -> Vec<u8> {
    let mut payload = b"6 0 ".to_vec();
    payload.extend_from_slice(value);
    let encoded = payload.iter().map(|b| format!("{b:02X}")).collect::<String>() + ">";
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R >>".to_vec(),
        stream("", b""),
        stream("/Type /ObjStm /N 1 /First 4 /Filter /ASCIIHexDecode", encoded.as_bytes()),
    ];
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (i, body) in objects.iter().enumerate() {
        offsets.push(object(&mut bytes, i + 1, body));
    }
    let xref = bytes.len();
    let mut rows = Vec::new();
    for number in 0..8 {
        let (kind, first, second): (u8, u32, u16) = match number {
            0 => (0, 0, 65535),
            6 => (2, 5, 0),
            7 => (1, u32::try_from(xref).unwrap(), 0),
            n => (1, u32::try_from(offsets[n]).unwrap(), 0),
        };
        rows.push(kind);
        rows.extend_from_slice(&first.to_be_bytes());
        rows.extend_from_slice(&second.to_be_bytes());
    }
    object(&mut bytes, 7, &stream("/Type /XRef /Size 8 /Root 1 0 R /W [1 4 2]", &rows));
    bytes.extend_from_slice(format!("startxref\n{xref}\n%%EOF\n").as_bytes());
    bytes
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn decoded_object_streams_keep_the_same_64_container_boundary() {
    for kind in 0..3 {
        let accepted = craft_pdf::open_slice(&compressed_object(&nested_object(64, kind)), None).unwrap();
        assert_nested_leaf(accepted.xref().get::<Object<'_>>(ObjRef::new(6, 0).into()).unwrap(), 64);
        let rejected = craft_pdf::open_slice(&compressed_object(&nested_object(65, kind)), None).unwrap();
        assert!(rejected.xref().get::<Object<'_>>(ObjRef::new(6, 0).into()).is_none(), "container kind {kind}");
    }
}

mod page_trees {
    use super::assemble;
    use craft_pdf::syntax::object::{Array, FromBytes, MaybeRef, Name, Number};
    use craft_pdf::syntax::page::{Page, Rotation};

    fn assert_page(page: &Page<'_>, id: i32, media: [f64; 4], crop: [f64; 4], rotation: i32, user_unit: f32) {
        assert_eq!(page.raw().get::<i32>(b"ContractPage"), Some(id));
        let actual = page.media_box();
        assert_eq!([actual.x0, actual.y0, actual.x1, actual.y1], media);
        let actual = page.crop_box();
        assert_eq!([actual.x0, actual.y0, actual.x1, actual.y1], crop);
        let actual = match page.rotation() {
            Rotation::None => 0,
            Rotation::Horizontal => 90,
            Rotation::Flipped => 180,
            Rotation::FlippedHorizontal => 270,
        };
        assert_eq!(actual, rotation);
        assert_eq!(page.user_unit(), user_unit);
    }

    fn deep_page_tree_contract() {
        const DEPTH: usize = 1024;
        let mut objects = vec![b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()];
        for level in 0..DEPTH {
            let number = level + 2;
            let parent = if level == 0 { String::new() } else { format!("/Parent {} 0 R", number - 1) };
            let attributes = match level {
                0 => "/MediaBox [0 0 200 300] /CropBox [1 2 190 290] /Rotate 90 /Resources << /Font << /Root << /BaseFont /Helvetica >> >> >>",
                512 => "/MediaBox [0 0 300 400] /Rotate 180 /Resources << /Font << /Middle << /BaseFont /Courier >> >> >>",
                _ => "",
            };
            objects.push(format!("<< /Type /Pages /Count 1 /Kids [{} 0 R] {parent} {attributes} >>", number + 1).into_bytes());
        }
        // An empty page is valid. Omitting Contents also prevents a brute-force scan
        // from satisfying this contract after losing the actual page-tree ancestry.
        objects.push(format!("<< /Type /Page /Parent {} 0 R /ContractPage 1024 /UserUnit 2 >>", DEPTH + 1).into_bytes());
        let pdf = craft_pdf::open_slice(&assemble(&objects), None).unwrap();
        assert_eq!(pdf.pages().len(), 1, "the valid leaf must survive all 1024 ancestors");
        let page = &pdf.pages()[0];
        assert_page(page, 1024, [0.0, 0.0, 300.0, 400.0], [1.0, 2.0, 190.0, 290.0], 180, 2.0);
        let middle = Name::from_bytes(b"/Middle").unwrap();
        let root = Name::from_bytes(b"/Root").unwrap();
        let font = page.resources().get_font(&middle).unwrap();
        assert_eq!(font.get::<Name<'_>>(b"BaseFont").unwrap().as_str(), "Courier");
        assert!(page.resources().get_font(&root).is_none(), "the middle dictionary replaces the root scope");
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn deep_page_trees_preserve_leaf_attributes_and_resources() {
        // Assert the observable contract before the small-stack check, so an old
        // recursive backend fails on missing pages before it can overflow that stack.
        deep_page_tree_contract();
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new().stack_size(128 * 1024).spawn(deep_page_tree_contract).unwrap().join().unwrap();
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn cyclic_page_trees_keep_valid_sibling_order_and_deduplicate_leaves() {
        let pdf = craft_pdf::open_slice(
            &assemble(&[
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Count 3 /Kids [3 0 R 6 0 R 5 0 R 4 0 R 6 0 R] /MediaBox [0 0 100 200] /CropBox [1 2 90 180] /Rotate 90 /Resources << /Font << /Root << /BaseFont /Helvetica >> >> >> >>".to_vec(),
                b"<< /Type /Pages /Parent 2 0 R /Count 2 /Kids [2 0 R 5 0 R 4 0 R 5 0 R] /MediaBox [0 0 300 400] /CropBox [10 20 280 370] /Rotate 180 /Resources << /Font << /Local << /BaseFont /Courier >> >> >> >>".to_vec(),
                b"<< /Type /Page /Parent 3 0 R /ContractPage 4 /Rotate 270 /UserUnit 2 /Resources << >> >>".to_vec(),
                b"<< /Type /Page /Parent 3 0 R /ContractPage 5 >>".to_vec(),
                b"<< /Type /Page /Parent 2 0 R /ContractPage 6 >>".to_vec(),
            ]),
            None,
        )
        .unwrap();
        assert_eq!(pdf.pages().len(), 3, "cycles and repeated references must not hide or duplicate valid pages");
        assert_page(&pdf.pages()[0], 5, [0.0, 0.0, 300.0, 400.0], [10.0, 20.0, 280.0, 370.0], 180, 1.0);
        assert_page(&pdf.pages()[1], 4, [0.0, 0.0, 300.0, 400.0], [10.0, 20.0, 280.0, 370.0], 270, 2.0);
        assert_page(&pdf.pages()[2], 6, [0.0, 0.0, 100.0, 200.0], [1.0, 2.0, 90.0, 180.0], 90, 1.0);
        let root = Name::from_bytes(b"/Root").unwrap();
        let local = Name::from_bytes(b"/Local").unwrap();
        assert!(pdf.pages()[0].resources().get_font(&local).is_some());
        assert!(pdf.pages()[0].resources().get_font(&root).is_none());
        assert!(pdf.pages()[1].resources().get_font(&local).is_none());
        assert!(pdf.pages()[1].resources().get_font(&root).is_none());
        assert!(pdf.pages()[2].resources().get_font(&root).is_some());
        assert!(pdf.pages()[2].resources().get_font(&local).is_none());
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn sign_only_numeric_children_preserve_later_valid_pages() {
        // Exercise the parser invariant directly: the old raw iterator panicked
        // after array validation accepted '+' but parsing could not read it.
        let array = Array::from_bytes(b"[+ 3]").unwrap();
        let values: Vec<_> = array
            .raw_iter()
            .map(|value| match value {
                MaybeRef::NotRef(value) => value.into_i32(),
                MaybeRef::Ref(_) => None,
            })
            .collect();
        assert_eq!(values, [Some(0), Some(3)]);
        for token in ["+", "+.", "-"] {
            let pdf = craft_pdf::open_slice(
                &assemble(&[
                    b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                    format!("<< /Type /Pages /Count 1 /Kids [{token} 3 0 R] /MediaBox [0 0 100 200] /CropBox [1 2 90 180] /Rotate 90 >>")
                        .into_bytes(),
                    b"<< /Type /Page /Parent 2 0 R /ContractPage 3 >>".to_vec(),
                ]),
                None,
            )
            .unwrap();
            assert_eq!(pdf.pages().len(), 1, "{token}: an ignored numeric child must not hide the valid sibling");
            assert_page(&pdf.pages()[0], 3, [0.0, 0.0, 100.0, 200.0], [1.0, 2.0, 90.0, 180.0], 90, 1.0);
            let bytes = format!("{token} ");
            assert_eq!(Number::from_bytes(bytes.as_bytes()).unwrap().as_f64(), 0.0, "{token}: keep the reader's zero tolerance");
        }
    }
}

#[cfg(feature = "interpret")]
mod painting {
    use super::{assemble, stream};
    use craft_pdf::interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        TransformExt, font::Glyph,
    };
    use kurbo::{Affine, BezPath, Point, Rect};

    #[cfg(craft_pdf_unicode_fixtures)]
    const INTER: &[u8] = include_bytes!(concat!(env!("CRAFT_PDF_TEST_FONT_DIR"), "/Inter-Regular.ttf"));

    #[derive(Debug)]
    struct GlyphEvent {
        text: String,
        mode: char,
        rgba: [f32; 4],
        origin: Point,
    }

    #[derive(Default)]
    struct Events {
        paths: Vec<[f32; 4]>,
        glyphs: Vec<GlyphEvent>,
        clips: Vec<String>,
    }

    fn rgba(paint: &Paint<'_>) -> [f32; 4] {
        let Paint::Color(color) = paint else { panic!("solid paint expected") };
        color.to_rgba().components()
    }

    impl Device<'_> for Events {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &BezPath, _: Affine, paint: &Paint<'_>, _: &PathDrawMode) {
            self.paths.push(rgba(paint));
        }
        fn push_clip_path(&mut self, clip: &ClipPath) {
            self.clips.push(clip.path.to_svg());
        }
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, glyph: &Glyph<'_>, _: Affine, glyph_transform: Affine, paint: &Paint<'_>, mode: &GlyphDrawMode) {
            let Glyph::Outline(outline) = glyph else { panic!("embedded outline glyph expected") };
            assert!(outline.font_data().unwrap().data.as_ref().as_ref() == fixture_font(), "fallback font cannot satisfy the contract");
            let text = match glyph.as_unicode().unwrap() {
                hayro_cmap::BfString::Char(c) => c.to_string(),
                hayro_cmap::BfString::String(s) => s,
            };
            let mode = match mode {
                GlyphDrawMode::Fill => 'F',
                GlyphDrawMode::Stroke(_) => 'S',
                GlyphDrawMode::Invisible => 'I',
                GlyphDrawMode::Clip => 'C',
            };
            self.glyphs.push(GlyphEvent { text, mode, rgba: rgba(paint), origin: glyph_transform * Point::ORIGIN });
        }
        fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    fn events(bytes: &[u8]) -> Events {
        let pdf = craft_pdf::open_slice(bytes, None).unwrap();
        let page = &pdf.pages()[0];
        let cache = InterpreterCache::new();
        let mut context = Context::new(
            page.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, 100.0, 60.0),
            &cache,
            page.xref(),
            InterpreterSettings::default(),
        );
        let mut device = Events::default();
        craft_pdf::interpret_page(page, &mut context, &mut device).unwrap();
        device
    }

    fn page(resources: &str, content: &[u8], extra: &[Vec<u8>]) -> Vec<u8> {
        let mut objects = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 40 40] /Contents 4 0 R /Resources {resources} >>").into_bytes(),
            stream("", content),
        ];
        objects.extend_from_slice(extra);
        assemble(&objects)
    }

    fn painted_form(color: &str, side: u32) -> Vec<u8> {
        stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 {side} {side}]"), format!("{color} rg 0 0 {side} {side} re f").as_bytes())
    }

    const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const GREEN: [f32; 4] = [0.0, 1.0, 0.0, 1.0];
    const GREEN_SCOPE: &str = "/Resources << /XObject << /Target 7 0 R >> >>";

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn nearest_page_resource_dictionary_replaces_the_whole_ancestor_dictionary() {
        for (middle, leaf, expected) in [
            ("", "", vec![RED]),
            ("/Resources << >>", "", vec![]),
            ("/Resources << /ProcSet [/PDF] >>", "", vec![]),
            (GREEN_SCOPE, "", vec![GREEN]),
            (GREEN_SCOPE, "/Resources << >>", vec![]),
            ("", "/Resources << /XObject << /Unused 7 0 R >> >>", vec![]),
        ] {
            let bytes = assemble(&[
                b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
                b"<< /Type /Pages /Count 1 /Kids [3 0 R] /Resources << /XObject << /Target 6 0 R >> >> >>".to_vec(),
                format!("<< /Type /Pages /Parent 2 0 R /Count 1 /Kids [4 0 R] {middle} >>").into_bytes(),
                format!("<< /Type /Page /Parent 3 0 R /MediaBox [0 0 40 40] /Contents 5 0 R {leaf} >>").into_bytes(),
                stream("", b"/Target Do"),
                painted_form("1 0 0", 40),
                painted_form("0 1 0", 40),
            ]);
            assert_eq!(events(&bytes).paths, expected, "middle={middle}, page={leaf}");
        }
    }

    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn forms_inherit_only_when_resources_are_absent_and_restore_the_caller_scope() {
        for (scope, first) in [("", Some(RED)), ("/Resources << >>", None), ("/Resources << /ProcSet [/PDF] >>", None), (GREEN_SCOPE, Some(GREEN))] {
            let bytes = page(
                "<< /XObject << /Outer 5 0 R /Target 6 0 R >> >>",
                b"/Outer Do /Target Do",
                &[
                    stream(&format!("/Type /XObject /Subtype /Form /BBox [0 0 40 40] {scope}"), b"/Target Do"),
                    painted_form("1 0 0", 40),
                    painted_form("0 1 0", 40),
                ],
            );
            let expected: Vec<_> = first.into_iter().chain([RED]).collect();
            assert_eq!(events(&bytes).paths, expected, "{scope}");
        }
    }

    #[cfg(feature = "raster")]
    fn pixels(bytes: &[u8]) -> Vec<u8> {
        use craft_pdf::raster::{self, Options, RenderCache};
        let pdf = craft_pdf::open_slice(bytes, None).unwrap();
        let image = raster::render(&pdf.pages()[0], &RenderCache::new(), &InterpreterSettings::default(), &Options::default()).unwrap();
        assert!(image.diagnostics.is_empty(), "{:?}", image.diagnostics);
        image.bytes().to_vec()
    }

    #[cfg(feature = "raster")]
    fn center(bytes: &[u8]) -> [u8; 4] {
        pixels(bytes)[(20 * 40 + 20) * 4..(20 * 40 + 20) * 4 + 4].try_into().unwrap()
    }

    #[cfg(feature = "raster")]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn type3_procedure_and_font_resources_replace_the_enclosing_scope() {
        for (font_scope, procedure_scope, expected) in [
            ("", "", [255, 0, 0, 255]),
            ("/Resources << >>", "", [0; 4]),
            (GREEN_SCOPE, "", [0, 255, 0, 255]),
            (GREEN_SCOPE, "/Resources << >>", [0; 4]),
            (GREEN_SCOPE, "/Resources << /ProcSet [/PDF] >>", [0; 4]),
            ("/Resources << >>", GREEN_SCOPE, [0, 255, 0, 255]),
        ] {
            let bytes = page(
                "<< /Font << /F1 5 0 R >> /XObject << /Target 6 0 R >> >>",
                b"BT /F1 20 Tf 10 10 Td (A) Tj ET",
                &[
                    format!("<< /Type /Font /Subtype /Type3 /FontBBox [0 0 1 1] /FontMatrix [1 0 0 1 0 0] /FirstChar 65 /LastChar 65 /Widths [1] /Encoding << /Differences [65 /a] >> /CharProcs << /a 8 0 R >> {font_scope} >>").into_bytes(),
                    painted_form("1 0 0", 1),
                    painted_form("0 1 0", 1),
                    stream(procedure_scope, b"1 0 d0 /Target Do"),
                ],
            );
            assert_eq!(center(&bytes), expected, "font={font_scope}, procedure={procedure_scope}");
        }
    }

    #[cfg(feature = "raster")]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn tiling_patterns_do_not_import_missing_resources_from_the_page() {
        for (scope, expected) in
            [("", [0; 4]), ("/Resources << >>", [0; 4]), ("/Resources << /ProcSet [/PDF] >>", [0; 4]), (GREEN_SCOPE, [0, 255, 0, 255])]
        {
            let bytes = page(
                "<< /Pattern << /P 5 0 R >> /XObject << /Target 6 0 R >> >>",
                b"/Pattern cs /P scn 0 0 40 40 re f",
                &[
                    stream(
                        &format!("/Type /Pattern /PatternType 1 /PaintType 1 /TilingType 1 /BBox [0 0 40 40] /XStep 40 /YStep 40 {scope}"),
                        b"/Target Do",
                    ),
                    painted_form("1 0 0", 40),
                    painted_form("0 1 0", 40),
                ],
            );
            assert_eq!(center(&bytes), expected, "{scope}");
        }
    }

    fn fixture_font() -> &'static [u8] {
        #[cfg(craft_pdf_unicode_fixtures)]
        {
            INTER
        }
        #[cfg(not(craft_pdf_unicode_fixtures))]
        {
            panic!("enable the hash-verified fonts.json fixtures")
        }
    }

    fn text_pdf(content: &str) -> Vec<u8> {
        assemble(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 60] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> /ExtGState << /G << /ca 0.25 /CA 0.75 >> >> >> >>".to_vec(),
            stream("", content.as_bytes()),
            b"<< /Type /Font /Subtype /TrueType /BaseFont /Inter-Regular /Encoding /WinAnsiEncoding /FirstChar 65 /LastChar 67 /Widths [600 600 600] /FontDescriptor 6 0 R >>".to_vec(),
            b"<< /Type /FontDescriptor /FontName /Inter-Regular /Flags 32 /FontFile2 7 0 R >>".to_vec(),
            stream("", fixture_font()),
        ])
    }

    #[cfg_attr(not(craft_pdf_unicode_fixtures), ignore = "requires hash-verified external fonts and craft_pdf_unicode_fixtures cfg")]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn filled_stroked_tj_runs_preserve_paint_order_alpha_and_text_advances() {
        let content = |mode| format!("/G gs 1 0 0 rg 0 0 1 RG 2 w BT /F1 20 Tf {mode} Tr 1 0 0 1 10 20 Tm [(A) 400 (B)] TJ (C) Tj ET");
        for mode in [2, 6] {
            let result = events(&text_pdf(&content(mode)));
            let expected = [('F', "A", 10.0), ('F', "B", 14.0), ('S', "A", 10.0), ('S', "B", 14.0), ('F', "C", 26.0), ('S', "C", 26.0)];
            assert_eq!(result.glyphs.len(), expected.len());
            for (glyph, (paint, text, x)) in result.glyphs.iter().zip(expected) {
                assert_eq!((glyph.mode, glyph.text.as_str()), (paint, text), "{mode} Tr: {:?}", result.glyphs);
                assert!((glyph.origin.x - x).abs() < 1e-6 && (glyph.origin.y - 20.0).abs() < 1e-6, "{glyph:?}");
                assert_eq!(glyph.rgba, if paint == 'F' { [1.0, 0.0, 0.0, 64.0 / 255.0] } else { [0.0, 0.0, 1.0, 191.0 / 255.0] });
            }
            if mode == 6 {
                assert_eq!(result.clips, events(&text_pdf(&content(4))).clips, "stroking must not duplicate text clipping outlines");
            }
        }
    }

    #[cfg(feature = "raster")]
    #[cfg_attr(not(craft_pdf_unicode_fixtures), ignore = "requires hash-verified external fonts and craft_pdf_unicode_fixtures cfg")]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn overlapping_translucent_glyphs_fill_the_whole_run_before_stroking() {
        let reference = pixels(&text_pdf(
            "/G gs 1 0 0 rg 0 0 1 RG 2 w BT /F1 20 Tf 0 Tr 1 0 0 1 10 20 Tm [(A) 400 (B)] TJ 1 Tr 1 0 0 1 10 20 Tm [(A) 400 (B)] TJ ET",
        ));
        assert!(reference.as_chunks::<4>().0.iter().any(|p| p[0] > 0 && p[2] > 0 && p[3] < 255));
        for mode in [2, 6] {
            let actual = pixels(&text_pdf(&format!("/G gs 1 0 0 rg 0 0 1 RG 2 w BT /F1 20 Tf {mode} Tr 1 0 0 1 10 20 Tm [(A) 400 (B)] TJ ET")));
            assert!(actual == reference, "{mode} Tr must match explicit whole-run fill followed by whole-run stroke");
        }
    }

    #[cfg(feature = "raster")]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    fn soft_mask_transfer_endpoints_use_rounded_output_bytes() {
        for (low, high, left, right) in [("0", "0", 0, 0), ("0.5", "0.5", 128, 128), ("1", "1", 255, 255), ("0.5001", "0.5", 128, 128)] {
            let bytes = page(
                "<< /ExtGState << /G 5 0 R >> >>",
                b"/G gs 0 0 1 rg 0 0 40 40 re f",
                &[
                    b"<< /SMask << /S /Luminosity /G 6 0 R /TR 7 0 R >> >>".to_vec(),
                    stream(
                        "/Type /XObject /Subtype /Form /BBox [0 0 40 40] /Group << /S /Transparency /CS /DeviceGray >>",
                        b"0 g 0 0 20 40 re f 1 g 20 0 20 40 re f",
                    ),
                    format!("<< /FunctionType 2 /Domain [0 1] /C0 [{low}] /C1 [{high}] /N 1 >>").into_bytes(),
                ],
            );
            let rendered = pixels(&bytes);
            for (x, alpha) in [(10, left), (30, right)] {
                assert_eq!(&rendered[(20 * 40 + x) * 4..(20 * 40 + x) * 4 + 4], &[0, 0, alpha, alpha], "C0={low}, C1={high}, x={x}");
            }
        }
    }
}
