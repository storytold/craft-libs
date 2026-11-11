//! Integration tests of craft-pdf: interpretation.

#![cfg(feature = "read")]
#![allow(clippy::unwrap_used, clippy::panic)]

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

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_closed_path_through_the_corners_out_of_order_is_not_a_rectangle() {
        // A bow tie: every point is a corner of the bounding box, but two sides are diagonals.
        let shapes = draw("0 0 0 RG 10 10 m 50 40 l 10 40 l 50 10 l h S");
        assert_eq!(shapes.rects, 0);
        assert_eq!(shapes.paths.len(), 1);
    }
}
