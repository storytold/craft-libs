#![cfg(feature = "read")]
#![allow(clippy::unwrap_used, clippy::panic)]

//! `RasterImage::default_color_space` answers each draw of an image by the `/DefaultCMYK` in
//! scope at that draw (ISO 32000-2 §8.6.5.6).

#[cfg(feature = "interpret")]
mod draws {
    use craft_pdf::interpret::{
        BlendMode, ClipPath, Context, Device, GlyphDrawMode, Image, InterpreterCache, InterpreterSettings, Paint, PathDrawMode, SoftMask,
        TransformExt, font::Glyph,
    };
    use kurbo::{Affine, BezPath, Rect};

    const CGATS: &[u8] = include_bytes!("../../../vendor/hayro-interpret/assets/CGATS001Compat-v2-micro.icc");

    /// Whether each raster image drawn was drawn in a default color space.
    #[derive(Default)]
    struct Defaults(Vec<bool>);

    impl Device<'_> for Defaults {
        fn set_soft_mask(&mut self, _: Option<SoftMask<'_>>) {}
        fn set_blend_mode(&mut self, _: BlendMode) {}
        fn draw_path(&mut self, _: &BezPath, _: Affine, _: &Paint<'_>, _: &PathDrawMode) {}
        fn push_clip_path(&mut self, _: &ClipPath) {}
        fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'_>>, _: BlendMode) {}
        fn draw_glyph(&mut self, _: &Glyph<'_>, _: Affine, _: Affine, _: &Paint<'_>, _: &GlyphDrawMode) {}
        fn draw_image(&mut self, image: Image<'_, '_>, _: Affine) {
            if let Image::Raster(r) = image {
                self.0.push(r.default_color_space().is_some());
            }
        }
        fn pop_clip_path(&mut self) {}
        fn pop_transparency_group(&mut self) {}
    }

    /// A page that draws `page`, whose `/Fm0` form draws `form` with a `/DefaultCMYK` in its
    /// resources. Both have `/Im0`, a 1 × 1 DeviceCMYK image XObject, and `/CS0`, a `/ColorSpace`
    /// resource that is DeviceCMYK.
    fn draw(page: &str, form: &str) -> Vec<bool> {
        let mut objects: Vec<Vec<u8>> = vec![
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 4 2] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R /Fm0 7 0 R >> /ColorSpace << /CS0 /DeviceCMYK >> >> >>"
                .to_vec(),
        ];
        let stream = |dict: &str, data: &[u8]| {
            let mut s = format!("<< {dict} /Length {} >> stream\n", data.len()).into_bytes();
            s.extend_from_slice(data);
            s.extend_from_slice(b"\nendstream");
            s
        };
        objects.push(stream("", page.as_bytes()));
        objects.push(stream("/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceCMYK /BitsPerComponent 8", &[0, 0, 0, 255]));
        objects.push(stream("/N 4", CGATS));
        objects.push(stream(
            "/Type /XObject /Subtype /Form /BBox [0 0 4 2] /Resources << /XObject << /Im0 5 0 R >> /ColorSpace << /DefaultCMYK [/ICCBased 6 0 R] /CS0 /DeviceCMYK >> >>",
            form.as_bytes(),
        ));
        let mut bytes = b"%PDF-1.7\n".to_vec();
        let mut offsets = vec![];
        for (i, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj ", i + 1).as_bytes());
            bytes.extend_from_slice(object);
            bytes.extend_from_slice(b" endobj\n");
        }
        let xref = bytes.len();
        bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes());
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(format!("trailer << /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF", objects.len() + 1).as_bytes());

        let pdf = craft_pdf::open_slice(&bytes, None).unwrap();
        let page = &pdf.pages()[0];
        let cache = InterpreterCache::new();
        let mut context =
            Context::new(page.initial_transform(true).to_kurbo(), Rect::new(0.0, 0.0, 4.0, 2.0), &cache, page.xref(), InterpreterSettings::default());
        let mut defaults = Defaults::default();
        craft_pdf::interpret_page(page, &mut context, &mut defaults).unwrap();
        defaults.0
    }

    const INLINE: &str = "BI /W 1 /H 1 /CS /CMYK /BPC 8 /F /AHx ID 000000FF> EI";
    const INLINE_CS0: &str = "BI /W 1 /H 1 /CS /CS0 /BPC 8 /F /AHx ID 000000FF> EI";

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn an_image_xobject_reports_the_default_of_each_draw() {
        assert_eq!(draw("/Im0 Do /Fm0 Do /Im0 Do", "/Im0 Do"), [false, true, false]);
    }

    #[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
    #[cfg_attr(not(target_arch = "wasm32"), test)]
    fn inline_images_report_the_default_of_each_draw() {
        // A device space named directly, and one named by a `/ColorSpace` resource.
        for inline in [INLINE, INLINE_CS0] {
            assert_eq!(draw(&format!("{inline} /Fm0 Do {inline}"), inline), [false, true, false], "{inline}");
        }
    }
}
