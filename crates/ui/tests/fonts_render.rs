//! A CJK and an Arabic label draw with the shared UI font stack and no system fonts.
//!
//! The kittest harness panics on any character no installed face can draw, so these tests fail
//! on a replacement box ("tofu") rather than passing a snapshot nobody looks at.

use craft_ui::fonts::{CraftFace, I18nScripts, SystemFonts, UiFonts};
use egui::{FontDefinitions, FontFamily};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;

const ARABIC: CraftFace =
    CraftFace { family: "Noto Sans Arabic", style: "Regular", scripts: &["Arab"], bytes: include_bytes!("fonts/NotoSansArabic-test.ttf"), index: 0 };
const CJK: CraftFace = CraftFace {
    family: "Noto Sans CJK SC",
    style: "Regular",
    scripts: &["Hans", "Latn"],
    bytes: include_bytes!("fonts/NotoSansCJKsc-test.otf"),
    index: 0,
};

const LABELS: [&str; 3] = ["Files", "日本語のファイル", "ملف عربي"];

fn harness(ui_language: &'static str, faces: &'static [CraftFace]) -> Harness<'static> {
    Harness::builder()
        .with_size([240.0, 120.0])
        .with_context_setup(move |ctx| {
            let defaults = FontDefinitions::default();
            let mut fonts =
                UiFonts::new(&I18nScripts).craft_faces(faces).languages(&["en", "ar", "ja"]).ui_language(ui_language).system_fonts(SystemFonts::Off);
            if let Some(ubuntu) = defaults.font_data.get("Ubuntu-Light") {
                fonts = fonts.app_face("ui", (**ubuntu).clone(), &[FontFamily::Proportional]);
            }
            fonts.install(ctx);
        })
        .build_ui(|ui| {
            for label in LABELS {
                ui.label(label);
            }
        })
}

#[test]
fn fonts_draw_cjk_and_arabic_labels_without_replacement_boxes() {
    for ui_language in ["en", "ar", "ja"] {
        let mut harness = harness(ui_language, &[ARABIC, CJK]);
        harness.run();
        for label in LABELS {
            assert!(harness.query_by_label(label).is_some(), "{ui_language}: {label} not shown");
        }
        let image = harness.render().expect("render");
        let background = *image.get_pixel(image.width() - 1, image.height() - 1);
        let drawn = image.pixels().filter(|p| **p != background).count();
        assert!(drawn > 500, "{ui_language}: only {drawn} pixels drawn");
    }
}

/// Without the Arabic face, an Arabic character (U+06xx) has no glyph.
#[test]
#[should_panic(expected = "(U+06")]
fn fonts_without_an_arabic_face_fail() {
    harness("en", &[CJK]).run();
}
