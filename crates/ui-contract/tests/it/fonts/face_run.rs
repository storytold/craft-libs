//! `Draw::face_run`: text in a document face at its size on its baseline, drawn smaller where the font atlas could
//! not hold its glyphs, with the interface fonts drawing what the face lacks.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use craft_ui::contract::{Common, Component, Cx, PaintUi, Role, Show as _};
use craft_ui::fonts::{EmbeddedFace, FaceBytes, FontRef, I18nScripts, SystemFonts, UiFonts};
use craft_ui::primitives::FaceRun;
use craft_ui::style::{DataColor, StyleCx};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::PlainStyle;
use egui::epaint::TextShape;
use egui::epaint::text::{Fonts, TextOptions};
use egui::{Color32, FontData, FontDefinitions, FontFamily, FontId, Galley, Pos2, Rect, Response, Shape};

/// The document face: Latin, no kana.
const NOTO: &[u8] = include_bytes!("../../fonts/NotoSans-test.ttf");
/// The interface's CJK face.
const CJK: &[u8] = include_bytes!("../../fonts/NotoSansCJKsc-test.otf");
const KEY: &str = "NotoSans-test.ttf#0";
/// Where the run's baseline starts.
const START: Pos2 = Pos2::new(40.0, 300.0);
const ROOM: Rect = Rect { min: Pos2::ZERO, max: Pos2::new(800.0, 600.0) };
/// The frames a face may take to be drawn in.
const FRAMES_TO_FACE: usize = 10;

/// Paints one face run from `START`, counting how often its face is loaded.
#[derive(craft_ui::contract::Component)]
struct Composition {
    #[common]
    common: Common,
    text: &'static str,
    size: f32,
    loads: Arc<AtomicUsize>,
}

impl Component for Composition {
    const ROLE: Role = Role::LABEL;
    type Style = PlainStyle;
    type Output = Response;

    fn style(_: &StyleCx<'_>, _: CompositionStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn paint(&mut self, ui: &mut PaintUi<'_>, _: &Cx<'_, Self>) {
        let loads = &self.loads;
        let run = FaceRun::new(self.text, KEY, self.size, DataColor::new(Color32::BLACK));
        ui.draw().face_run(START, &run, || {
            loads.fetch_add(1, Ordering::Relaxed);
            Some(FaceBytes::new(Arc::from(NOTO), 0))
        });
    }

    fn finish(self, cx: Cx<'_, Self>) -> Response {
        cx.response.clone()
    }
}

/// The text shape of `text` that one frame paints.
fn frame(frames: &mut Frames, text: &'static str, size: f32, loads: &Arc<AtomicUsize>) -> TextShape {
    frames.record_paint();
    frames.frame(vec![], |ui| {
        Composition { common: Common::new(), text, size, loads: Arc::clone(loads) }.show_at(ui, ROOM);
    });
    let mut paint = frames.take_paint().expect("the frames record their paint");
    // No renderer takes the frame's atlas changes.
    paint.textures_delta.clear();
    let mut shapes: Vec<Shape> = paint.shapes.into_iter().map(|clipped| clipped.shape).collect();
    while let Some(shape) = shapes.pop() {
        match shape {
            Shape::Text(shape) if shape.galley.job.text == text => return shape,
            Shape::Vec(nested) => shapes.extend(nested),
            _ => {}
        }
    }
    panic!("{text:?} was painted");
}

fn font_of(shape: &TextShape) -> FontId {
    shape.galley.job.sections.first().map(|section| section.format.font_id.clone()).expect("a section")
}

/// Frames until the run is drawn in a family of its own, not the interface's.
fn until_in_face(frames: &mut Frames, text: &'static str, size: f32, loads: &Arc<AtomicUsize>) -> TextShape {
    for _ in 0..FRAMES_TO_FACE {
        let shape = frame(frames, text, size, loads);
        if font_of(&shape).family != FontFamily::Proportional {
            return shape;
        }
    }
    panic!("{text:?} was not drawn in its face in {FRAMES_TO_FACE} frames");
}

/// The first row's baseline, from the galley's top.
fn first_baseline(galley: &Galley) -> f32 {
    let row = galley.rows.first().expect("a row");
    row.pos.y + row.row.glyphs.first().expect("a glyph").pos.y
}

#[trial]
fn a_face_run_draws_in_its_face_at_its_size_on_its_baseline() {
    let (mut frames, loads) = (Frames::new(), Arc::default());
    // The app holds the face's bytes, so the run is drawn in its face in the pass that registers it.
    let shape = frame(&mut frames, "Typed", 30.0, &loads);
    let font = font_of(&shape);
    assert!(matches!(font.family, FontFamily::Name(_)), "the face's own family: {:?}", font.family);
    assert_eq!(font.size, 30.0, "the run's own size");
    let baseline = shape.pos.y + first_baseline(&shape.galley);
    assert!((baseline - START.y).abs() < 1e-3, "the first baseline lies on the given one: {baseline} vs {}", START.y);
    assert_eq!(shape.pos.x, START.x, "the run starts where the baseline does");
    for _ in 0..5 {
        assert_eq!(font_of(&frame(&mut frames, "Typed", 30.0, &loads)).family, font.family, "the face stays");
    }
    assert_eq!(loads.load(Ordering::Relaxed), 1, "the face is loaded and installed once, not every frame");
}

#[trial]
fn an_oversized_face_run_is_drawn_smaller_than_the_atlas_cap() {
    let (mut frames, loads) = (Frames::new(), Arc::default());
    frames.pixels_per_point(2.0);
    // 1,638 points at 500 % is about 8,000 points: wider than egui's atlas, which fails on such a glyph.
    let shape = until_in_face(&mut frames, "Big", 8000.0, &loads);
    let side = frames.ctx.input(|input| input.max_texture_side) as f32;
    let cap = (side / 4.0).min(1024.0) / 2.0;
    assert_eq!(font_of(&shape).size, cap, "the run is drawn at a quarter of the atlas side, at most 1,024 pixels");
    assert!(shape.galley.rows.iter().all(|row| row.row.size.y * 2.0 < side), "each row fits the atlas");
    let (nan, infinite) =
        (FaceRun::new("x", KEY, f32::NAN, DataColor::new(Color32::BLACK)), FaceRun::new("x", KEY, f32::INFINITY, DataColor::new(Color32::BLACK)));
    assert_eq!(nan.drawn_size(&frames.ctx), 1.0, "a NaN size draws at 1 point");
    assert_eq!(infinite.drawn_size(&frames.ctx), FaceRun::largest_size(&frames.ctx), "an infinite size at the cap");
}

/// An interface stack whose CJK face is the CJK test face, with no system fonts.
fn interface_with_cjk(ctx: &egui::Context) {
    let latin = (**craft_ui::fonts::deflated_default_definitions().font_data.get("Ubuntu-Light").expect("egui's face")).clone();
    let cjk = EmbeddedFace { family: "Noto Sans CJK SC", style: "Regular", scripts: &["Hans", "Latn"], bytes: CJK, index: 0, role: None };
    UiFonts::new(&I18nScripts)
        .app_face("ui", latin, &[FontRef::proportional()])
        .craft_faces(&[cjk])
        .languages(&["en", "zh-CN"])
        .ui_language("en")
        .system_fonts(SystemFonts::Off)
        .install(ctx);
}

#[trial]
fn a_kana_run_in_a_face_without_kana_draws_through_the_interface_cjk_face() {
    let (mut frames, loads) = (Frames::new(), Arc::default());
    // A character no font of the run can draw fails the trial instead of drawing a box.
    frames.ctx.set_missing_glyph_policy(egui::MissingGlyphPolicy::Panic);
    interface_with_cjk(&frames.ctx);
    // Katakana and hiragana the interface's CJK test face has.
    let text = "ファイルのA";
    let mut alone = FontDefinitions::empty();
    alone.font_data.insert("noto".into(), Arc::new(FontData::from_static(NOTO)));
    alone.families.insert(FontFamily::Proportional, vec!["noto".into()]);
    let mut alone = Fonts::new(TextOptions::default(), alone);
    assert!(text.chars().filter(|c| !c.is_ascii()).all(|c| !alone.has_glyph(&FontId::proportional(20.0), c)), "the face itself has no kana");
    let shape = until_in_face(&mut frames, text, 20.0, &loads);
    let font = font_of(&shape);
    for c in text.chars() {
        assert!(frames.ctx.fonts_mut(|fonts| fonts.has_glyph(&font, c)), "{c} is drawn by a face of the run's family");
    }
    assert_eq!(shape.galley.rows.first().map(|row| row.row.glyphs.len()), Some(text.chars().count()), "every character is drawn");
}
