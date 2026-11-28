//! Integration tests of craft-pdf: interpretation.

#![cfg(feature = "read")]
#![allow(clippy::unwrap_used, clippy::panic)]

// A CID that is its character's vertical form draws a substitute's vertical glyph.
#[cfg(feature = "interpret")]
#[path = "interpretation/vertical_forms.rs"]
mod vertical_forms;

#[cfg(feature = "interpret")]
mod device {
    use craft_pdf::interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        TransformExt, font::Glyph,
    };
    use kurbo::{Affine, BezPath, Rect};

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

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_closed_path_through_the_corners_out_of_order_is_not_a_rectangle() {
        // A bow tie: every point is a corner of the bounding box, but two sides are diagonals.
        let shapes = draw("0 0 0 RG 10 10 m 50 40 l 10 40 l 50 10 l h S");
        assert_eq!(shapes.rects, 0);
        assert_eq!(shapes.paths.len(), 1);
    }
}

#[cfg(feature = "interpret")]
mod image_limits {
    use craft_pdf::interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, ImageData, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        TransformExt, font::Glyph,
    };
    use craft_pdf::syntax::DecodeLimits;
    use kurbo::{Affine, BezPath, Rect};

    /// The RGB samples and alpha of a raster image.
    type Pixels = (Vec<u8>, Option<Vec<u8>>);

    /// The pixels of each raster image drawn (`None`: it did not decode).
    #[derive(Default)]
    struct Images(Vec<Option<Pixels>>);

    impl Images {
        /// The RGB samples of each image, without alpha.
        fn rgb(&self) -> Vec<Option<Vec<u8>>> {
            self.0.iter().map(|i| i.as_ref().map(|(rgb, _)| rgb.clone())).collect()
        }
    }

    impl Device<'_> for Images {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, image: Image<'_, '_>, _: Affine) {
            let mut rgb = None;
            if let Image::Raster(r) = image {
                r.with_rgba(
                    |img, alpha| {
                        if let ImageData::Rgb(d) = img {
                            rgb = Some((d.data, alpha.map(|a| a.data)));
                        }
                    },
                    None,
                );
            }
            self.0.push(rgb);
        }
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    /// A page that draws a 4 × 2 `DeviceRGB` image of 8-bit samples 0, 1, … 23, read with `limits`.
    /// The samples are one run-length literal run, written in hex.
    fn draw(limits: DecodeLimits) -> Images {
        draw_with(limits, RGB)
    }

    /// The color space and sample size of [`draw`]'s image.
    const RGB: &str = "/ColorSpace /DeviceRGB /BitsPerComponent 8";

    /// [`draw`], with `extra` in the image's dictionary instead of [`RGB`].
    fn draw_with(limits: DecodeLimits, extra: &str) -> Images {
        let run: Vec<u8> = [23].into_iter().chain(0..24).chain([128]).collect();
        let hex: String = run.iter().map(|b| format!("{b:02X}")).collect();
        let content = "q 4 0 0 2 0 0 cm /Im0 Do Q";
        let bytes = format!(
            "%PDF-1.7\n1 0 obj << /Type /Catalog /Pages 2 0 R >> endobj\n2 0 obj << /Type /Pages /Kids [3 0 R] /Count 1 >> endobj\n3 0 obj << /Type /Page /Parent 2 0 R /MediaBox [0 0 4 2] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R >> >> >> endobj\n4 0 obj << /Length {} >> stream\n{content}\nendstream endobj\n5 0 obj << /Type /XObject /Subtype /Image /Width 4 /Height 2 /Filter [/ASCIIHexDecode /RunLengthDecode] {extra} /Length {} >> stream\n{hex}>\nendstream endobj\ntrailer << /Root 1 0 R >>\n%%EOF",
            content.len(),
            hex.len() + 1
        );
        let options = craft_pdf::OpenOptions { decode_limits: limits, ..Default::default() };
        let pdf = craft_pdf::open_slice_with_options(bytes.as_bytes(), None, &options).unwrap();
        let page = &pdf.pages()[0];
        let cache = InterpreterCache::new();
        let mut context =
            Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, 4.0, 2.0), &cache, page.xref(), InterpreterSettings::default());
        let mut images = Images::default();
        craft_pdf::interpret_page(page, &mut context, &mut images).unwrap();
        images
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn device_rgb_samples_arrive_as_written() {
        assert_eq!(draw(DecodeLimits::default()).0, [Some(((0..24).collect::<Vec<u8>>(), None))]);
        // An inverted `/Decode` inverts them.
        let inverted: Vec<u8> = (0..24).map(|s| 255 - s).collect();
        assert_eq!(draw_with(DecodeLimits::default(), &format!("{RGB} /Decode [1 0 1 0 1 0]")).rgb(), [Some(inverted)]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_color_key_mask_reads_device_rgb_samples() {
        // Pixels 0 (0, 1, 2) and 1 (3, 4, 5) have every sample in 0 to 5, so they are masked out.
        let images = draw_with(DecodeLimits::default(), &format!("{RGB} /Mask [0 5 0 5 0 5]"));
        assert_eq!(images.0, [Some(((0..24).collect::<Vec<u8>>(), Some(vec![0, 0, 255, 255, 255, 255, 255, 255])))]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn a_color_key_mask_of_an_emptied_gray_image_reads_its_padding() {
        // A 1-bit inverted gray image whose samples the byte limit leaves out: the missing samples
        // read as 0, white once inverted, and inside the key range, so every pixel is masked out.
        let images =
            draw_with(DecodeLimits::default().with_max_decoded_image(0), "/ColorSpace /DeviceGray /BitsPerComponent 1 /Decode [1 0] /Mask [0 0]");
        assert_eq!(images.0, [Some((vec![255; 24], Some(vec![0; 8])))]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn the_pixel_limit_comes_from_the_document() {
        assert_eq!(draw(DecodeLimits::default().with_max_image_pixels(7)).0, [None], "8 pixels are past a limit of 7");
        assert_eq!(draw(DecodeLimits::default().with_max_image_pixels(8)).rgb(), [Some((0..24).collect::<Vec<u8>>())]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn the_image_byte_limit_comes_from_the_document() {
        // The run-length filter stops at 10 bytes: one row is left, its last two samples zero.
        let cut: Vec<u8> = (0..10).chain([0, 0]).collect();
        assert_eq!(draw(DecodeLimits::default().with_max_decoded_image(10)).rgb(), [Some(cut)]);
        assert_eq!(draw(DecodeLimits::default().with_max_decoded_image(1 << 40)).rgb(), [Some((0..24).collect::<Vec<u8>>())]);
    }
}
