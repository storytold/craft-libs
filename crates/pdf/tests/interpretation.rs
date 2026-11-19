#![cfg(feature = "read")]
#![allow(clippy::unwrap_used, clippy::panic)]

#[cfg(feature = "interpret")]
mod device {
    use craft_pdf::interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        TransformExt, font::Glyph,
    };
    use kurbo::{Affine, BezPath, Rect};

    #[derive(Default)]
    struct PathDevice {
        paths: usize,
        fail: bool,
    }

    impl Device<'_> for PathDevice {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {
            assert!(!self.fail, "synthetic device failure");
            self.paths += 1;
        }
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    fn execute(device: &mut PathDevice) -> Result<(), craft_pdf::ExecutionError> {
        let content = "1 0 0 rg 0 0 40 20 re f";
        let bytes = format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R >> endobj\n4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\ntrailer << /Root 1 0 R >>\n%%EOF",
            content.len()
        );
        let pdf = craft_pdf::open_slice(bytes.as_bytes(), None).unwrap();
        let page = &pdf.pages()[0];
        let cache = InterpreterCache::new();
        let mut context = Context::new(
            page.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, 40.0, 20.0),
            &cache,
            page.xref(),
            InterpreterSettings::default(),
        );
        craft_pdf::interpret_page(page, &mut context, device)
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn guarded_interpreter_reaches_the_device_without_raster() {
        let mut device = PathDevice::default();
        execute(&mut device).unwrap();
        assert_eq!(device.paths, 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn device_panic_is_an_execution_error_and_fresh_state_still_works() {
        assert_eq!(execute(&mut PathDevice { fail: true, ..Default::default() }), Err(craft_pdf::ExecutionError::BackendPanic));
        let mut fresh = PathDevice::default();
        execute(&mut fresh).unwrap();
        assert_eq!(fresh.paths, 1);
    }

    /// Records the clips the interpreter applies, alongside the paths painted under them.
    #[derive(Default)]
    struct ClipDevice {
        clips: Vec<BezPath>,
        paths: usize,
    }

    impl Device<'_> for ClipDevice {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {
            self.paths += 1;
        }
        fn push_clip_path(&mut self, clip_path: &ClipPath) {
            self.clips.push(clip_path.path.clone());
        }
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    fn clips_of(content: &str) -> ClipDevice {
        let bytes = format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF",
            content.len()
        );
        let pdf = craft_pdf::open_slice(bytes.as_bytes(), None).unwrap();
        let page = &pdf.pages()[0];
        let cache = InterpreterCache::new();
        let mut context = Context::new(
            page.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, 40.0, 20.0),
            &cache,
            page.xref(),
            InterpreterSettings::default(),
        );
        let mut device = ClipDevice::default();
        craft_pdf::interpret_page(page, &mut context, &mut device).unwrap();
        device
    }

    /// A glyph shown in a clipping render mode adds its outline to the clipping path, and that
    /// clip applies to everything painted after `ET`. When the glyphs shown have no outline -- a
    /// space, say -- the clip is empty and nothing paints through it. The interpreter applied a
    /// clip only once a glyph had contributed an outline, so such a text object clipped nothing
    /// and what followed painted unmasked.
    ///
    /// A text object that shows no glyph at all is a different case: there the clipping path is
    /// unchanged, as readers leave it.
    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_glyph_shown_in_a_clipping_mode_clips_even_with_no_outline() {
        // `7 Tr` is clip-only, and the glyph shown has no outline: the clip is empty.
        let empty = clips_of("q BT 7 Tr /F1 12 Tf 4 4 Td ( ) Tj ET 1 0 0 rg 0 0 40 20 re f Q");
        assert_eq!(empty.clips.len(), 1, "the empty clip is applied");
        assert!(empty.clips[0].is_empty(), "and it clips everything away: {:?}", empty.clips[0]);

        // Visible glyphs in the same mode clip to their outlines, as they always did: one clip, not
        // empty, and the fill still painted beneath it (noted in review).
        let visible = clips_of("q BT 7 Tr /F1 12 Tf 4 4 Td (H) Tj ET 1 0 0 rg 0 0 40 20 re f Q");
        assert_eq!(visible.clips.len(), 1, "the glyph outline is applied as the clip");
        assert!(!visible.clips[0].is_empty(), "and it holds the outline");
        assert_eq!(visible.paths, 1);

        // A clipping mode with no glyph shown at all leaves the clipping path alone.
        let none = clips_of("q BT 7 Tr /F1 12 Tf 4 4 Td ET 1 0 0 rg 0 0 40 20 re f Q");
        assert!(none.clips.is_empty(), "nothing was shown, so nothing clips");
        assert_eq!(none.paths, 1);

        // No text object: the fill is unclipped, as it always was.
        let plain = clips_of("q 1 0 0 rg 0 0 40 20 re f Q");
        assert!(plain.clips.is_empty(), "nothing clips the fill");
        assert_eq!(plain.paths, 1);

        // A painting mode never clips, outline or not.
        let painting = clips_of("q BT 0 Tr /F1 12 Tf 4 4 Td ( ) Tj ET 1 0 0 rg 0 0 40 20 re f Q");
        assert!(painting.clips.is_empty(), "a painting text mode never clips");
        assert_eq!(painting.paths, 1);
    }
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn invalid_pdf_is_a_typed_error() {
    assert!(matches!(craft_pdf::open_slice(b"not a pdf", None), Err(craft_pdf::OpenError::Load(_))));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn input_limit_can_be_checked_before_allocating() {
    assert!(craft_pdf::check_input_size(craft_pdf::MAX_INPUT_BYTES).is_ok());
    assert!(matches!(
        craft_pdf::check_input_size(craft_pdf::MAX_INPUT_BYTES + 1),
        Err(craft_pdf::OpenError::InputLimit { limit: craft_pdf::MAX_INPUT_BYTES })
    ));
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn parsing_without_interpretation_exposes_pages_and_objects() {
    let bytes = b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF";
    let pdf = craft_pdf::open_slice(bytes, None).unwrap();
    assert_eq!(pdf.pages().len(), 1);
    let catalog = pdf.xref().get::<craft_pdf::syntax::object::Dict<'_>>(pdf.xref().root_id()).unwrap();
    assert!(catalog.get_ref(b"Pages").is_some());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn caller_input_policy_is_checked_before_parser_and_copy() {
    let options = craft_pdf::OpenOptions { max_input_bytes: 3, ..Default::default() };
    assert!(matches!(craft_pdf::open_slice_with_options(b"bad!", None, &options), Err(craft_pdf::OpenError::InputLimit { limit: 3 })));
    assert!(matches!(
        craft_pdf::open_with_options(std::sync::Arc::new(b"bad!".to_vec()), None, &options),
        Err(craft_pdf::OpenError::InputLimit { limit: 3 })
    ));
    assert!(craft_pdf::check_input_size_with(3, &options).is_ok());
    let larger = craft_pdf::OpenOptions { max_input_bytes: usize::MAX, ..Default::default() };
    assert!(craft_pdf::check_input_size_with(craft_pdf::MAX_INPUT_BYTES + 1, &larger).is_ok());
}

#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
#[cfg_attr(not(target_arch = "wasm32"), test)]
fn caller_decode_policy_survives_open_fallback() {
    let bytes = b"%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF";
    let options =
        craft_pdf::OpenOptions { decode_limits: craft_pdf::syntax::DecodeLimits::default().with_max_decoded_stream(32), ..Default::default() };
    let low = craft_pdf::open_slice_with_options(bytes, None, &options).unwrap();
    let normal = craft_pdf::open_slice(bytes, None).unwrap();
    assert_eq!(low.decode_limits().max_decoded_stream(), 32);
    assert_eq!(normal.decode_limits(), craft_pdf::syntax::DecodeLimits::default());
}

/// What the clip does to the page, rendered: the device test above shows the clip the interpreter
/// sets, this shows the pixels it leaves (noted in review).
#[cfg(all(feature = "raster", not(target_arch = "wasm32")))]
mod rendered {
    use craft_pdf::raster::{self, Options, RenderCache};

    /// The share of a 40 x 20 pt page painted red (the fill), after `text` and then the fill.
    fn red_share(text: &str) -> f64 {
        let content = format!("q {text} 1 0 0 rg 0 0 40 20 re f Q");
        let bytes = format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 40 20] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> endobj\n4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n5 0 obj << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >> endobj\ntrailer << /Root 1 0 R >>\n%%EOF",
            content.len()
        );
        let pdf = craft_pdf::open_slice(bytes.as_bytes(), None).unwrap();
        let page = raster::render(&pdf.pages()[0], &RenderCache::new(), &Default::default(), &Options { background: [255; 4], ..Default::default() })
            .unwrap();
        let pixels = page.bytes().as_chunks::<4>().0;
        let red = pixels.iter().filter(|p| p[0] > 200 && p[1] < 80 && p[2] < 80).count();
        red as f64 / pixels.len() as f64
    }

    #[test]
    fn an_empty_text_clip_hides_what_follows_and_a_visible_one_shows_its_glyphs() {
        assert!(red_share("") > 0.99, "no text object: the fill covers the page");
        assert_eq!(red_share("BT 7 Tr /F1 12 Tf 4 4 Td ( ) Tj ET"), 0.0, "an outline-less glyph in 7 Tr: nothing paints");
        let through = red_share("BT 7 Tr /F1 12 Tf 4 4 Td (H) Tj ET");
        assert!(through > 0.02 && through < 0.5, "visible glyphs in 7 Tr: the fill shows through the H only ({through})");
        assert!(red_share("BT 7 Tr /F1 12 Tf 4 4 Td ET") > 0.99, "no glyph shown: the clip is unchanged");
    }
}

/// A path reaches the device as a rectangle only when it is one. A five-point open polyline whose
/// points reach every corner of its own bounding box (VectorCraft #2920, a zigzag in an `.ai`
/// file) used to be drawn as that box.
#[cfg(feature = "interpret")]
mod rectangles {
    use craft_pdf::interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        TransformExt, font::Glyph,
    };
    use kurbo::{Affine, BezPath, Rect};

    #[derive(Default)]
    struct Shapes {
        rects: usize,
        paths: Vec<BezPath>,
        clips: usize,
    }

    impl Device<'_> for Shapes {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, path: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {
            self.paths.push(path.clone());
        }
        fn draw_rect(&mut self, _: &Rect, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {
            self.rects += 1;
        }
        fn push_clip_path(&mut self, _: &ClipPath) {
            self.clips += 1;
        }
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, _: Image<'_, '_>, _: Affine) {}
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    fn draw(content: &str) -> Shapes {
        let bytes = format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 120 60] /Contents 4 0 R >> endobj\n4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\ntrailer << /Root 1 0 R >>\n%%EOF",
            content.len()
        );
        let pdf = craft_pdf::open_slice(bytes.as_bytes(), None).unwrap();
        let page = &pdf.pages()[0];
        let cache = InterpreterCache::new();
        let mut context = Context::new(
            page.initial_transform(true).to_kurbo(),
            Rect::new(0.0, 0.0, 120.0, 60.0),
            &cache,
            page.xref(),
            InterpreterSettings::default(),
        );
        let mut shapes = Shapes::default();
        craft_pdf::interpret_page(page, &mut context, &mut shapes).unwrap();
        shapes
    }

    /// The open zigzag of VectorCraft #2920: its ends and the points beside them land on all four
    /// corners of its bounding box, and the peak at (60.571, 29.875) is on none.
    const ZIGZAG: &str = "80 38 m 80 21.75 l 60.571 29.875 l 40 21.75 l 40 38 l";

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn an_open_five_point_polyline_is_stroked_as_drawn() {
        let shapes = draw(&format!("0 0 0 RG 2 w {ZIGZAG} S"));
        assert_eq!(shapes.rects, 0, "the zigzag was drawn as its bounding box");
        assert_eq!(shapes.paths.len(), 1);
        assert_eq!(shapes.paths[0].elements().len(), 5, "the five points arrive as written");
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn an_open_five_point_polyline_is_filled_as_drawn() {
        let shapes = draw(&format!("0 0 0 rg {ZIGZAG} f"));
        assert_eq!(shapes.rects, 0);
        assert_eq!(shapes.paths.len(), 1);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_closed_path_through_the_corners_out_of_order_is_not_a_rectangle() {
        // A bow tie: every point is a corner of the bounding box, but two sides are diagonals.
        let shapes = draw("0 0 0 RG 10 10 m 50 40 l 10 40 l 50 10 l h S");
        assert_eq!(shapes.rects, 0);
        assert_eq!(shapes.paths.len(), 1);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn rectangles_still_take_the_rectangle_route() {
        for content in [
            "0 0 0 rg 10 10 40 30 re f",
            "0 0 0 RG 10 10 40 30 re S",
            "0 0 0 RG 10 10 m 50 10 l 50 40 l 10 40 l h S",
            "0 0 0 rg 10 10 m 10 40 l 50 40 l 50 10 l 10 10 l f",
        ] {
            let shapes = draw(content);
            assert_eq!((shapes.rects, shapes.paths.len()), (1, 0), "{content}");
        }
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_polyline_clip_is_kept() {
        // A clip that is a rectangle covering the page is dropped as a no-op; this one is not a
        // rectangle, so it has to reach the device even though its bounding box covers the page.
        let shapes = draw("0 0 m 120 0 l 60 30 l 0 60 l 120 60 l W n 0 0 0 rg 0 0 120 60 re f");
        assert_eq!(shapes.clips, 1);
    }
}
