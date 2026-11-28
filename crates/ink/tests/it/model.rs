//! The neutral model: brush palette, colours, creation keys and unit conversion.

use craft_ink::recognize::{Accelerator, FixedRecognizer, LineImage, Recognizer, TROCR_SMALL_HANDWRITTEN};
use craft_ink::stroke::{Brush, Color, DEFAULT_PENS, Effect, MAX_PRESSURE, Tool, cm_to_px, guid, ink_size_cm, mint, mint_free, widths_mm};

#[test]
fn the_six_default_pens_are_the_gallery_office_shows() {
    assert_eq!(DEFAULT_PENS.len(), 6);
    assert_eq!(DEFAULT_PENS.iter().map(|p| p.id).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(DEFAULT_PENS[0].effect, Some(Effect::Galaxy));
    assert_eq!(DEFAULT_PENS[1].color, Color::rgb(0xD1, 0x3B, 0x2F));
    assert_eq!(DEFAULT_PENS[2].tool, Tool::Pencil);
    assert_eq!(DEFAULT_PENS[3].tool, Tool::Highlighter);
    assert_eq!(DEFAULT_PENS[4].tool, Tool::FountainPen);
    assert_eq!(DEFAULT_PENS[5].tool, Tool::BrushPen);
    for pen in DEFAULT_PENS {
        let base = pen.effect.map_or(Color::BLACK, Effect::base_color);
        assert_eq!(Color::from_hex(pen.effect.map_or("#000000", Effect::base_hex)), Some(base));
        assert!(pen.width_mm > 0.0);
    }
}

#[test]
fn the_eight_textures_have_names_colours_and_inverse_parsing() {
    assert_eq!(Effect::ALL.len(), 8);
    for effect in Effect::ALL {
        assert_eq!(Effect::from_name(effect.name()), Some(effect));
        assert_eq!(Effect::from_name(effect.inkml_name()), Some(effect));
        assert_eq!(Color::from_hex(effect.base_hex()), Some(effect.base_color()));
    }
    assert_eq!(Effect::from_name("rose_gold"), Some(Effect::RoseGold));
    assert_eq!(Effect::from_name("none"), None);
}

#[test]
fn brush_names_parse_and_thickness_scales_by_kind() {
    for tool in Tool::ALL {
        assert_eq!(Tool::from_name(tool.name()), Some(tool));
        assert_eq!(Tool::from_name(&tool.name().to_lowercase()), Some(tool));
        assert!(!widths_mm(tool).is_empty());
    }
    assert_eq!(Tool::from_name("fountain"), Some(Tool::FountainPen));
    assert_eq!(Tool::from_name("brush"), Some(Tool::BrushPen));
    assert_eq!(Tool::from_name("eraser"), None);
    let (w, h) = ink_size_cm(Tool::Pencil, 1.0);
    assert!((w - 0.1).abs() < 1e-6 && (h - 0.6).abs() < 1e-6, "a pencil is six times as tall");
    let (w, h) = ink_size_cm(Tool::Highlighter, 4.0);
    assert!((w - 0.4).abs() < 1e-6 && (h - 0.8).abs() < 1e-6, "a highlighter twice");
    let (w, h) = ink_size_cm(Tool::Pen, 3.5);
    assert!((w - 0.35).abs() < 1e-6 && w == h);
    // Bounded: a huge width is clamped to 5 cm and a hostile one takes the default.
    assert_eq!(ink_size_cm(Tool::Pen, 1e9).0, 5.0);
    assert_eq!(ink_size_cm(Tool::Pen, f32::INFINITY).0, 0.05);
}

#[test]
fn sheet_points_and_centimetres_convert_both_ways() {
    assert!((cm_to_px(2.54) - 96.0).abs() < 1e-9);
    assert!((craft_ink::stroke::px_to_cm(96.0) - 2.54).abs() < 1e-9);
    for px in [0.0, 0.5, 37.0, 10_000.0] {
        assert!((cm_to_px(craft_ink::stroke::px_to_cm(px)) - px).abs() < 1e-9);
    }
}

#[test]
fn creation_keys_are_canonical_guid_shaped_and_unique() {
    let key = mint(None);
    assert_eq!(guid(&key).as_deref(), Some(key.as_str()));
    assert_ne!(key, mint(None));
    let held = "(kept)".to_string();
    let second = mint_free(|candidate| candidate == key || candidate == held);
    assert_ne!(second, key);
    assert_eq!(mint(Some("{ABCDEF01-2345-6789-ABCD-EF0123456789}")), "{abcdef01-2345-6789-abcd-ef0123456789}");
    assert_ne!(mint(Some("not-a-guid")), "not-a-guid");
    for bad in ["{}", "{x}", "{11111111-2222-4333-8444-55555555555}"] {
        assert_eq!(guid(bad), None, "{bad}");
    }
}

#[test]
fn a_stroke_holds_at_most_the_point_limit_and_valid_pressure() {
    use craft_ink::stroke::{MAX_POINTS, Stroke, StrokePoint};
    let point = StrokePoint::new(0.0, 0.0);
    let mut stroke = Stroke::new(vec![point], Brush::default(), Color::BLACK);
    assert!(stroke.is_valid());
    stroke.points = vec![point; MAX_POINTS + 1];
    assert!(!stroke.is_valid());
    stroke.points = vec![point; MAX_POINTS];
    assert!(stroke.is_valid());
    stroke.points[0].pressure = Some(MAX_PRESSURE);
    assert!(!stroke.is_valid(), "pressure on one point only");
    for p in &mut stroke.points {
        p.pressure = Some(MAX_PRESSURE);
    }
    assert!(stroke.is_valid());
    stroke.points[0].pressure = Some(MAX_PRESSURE + 1);
    assert!(!stroke.is_valid());
    stroke.points = vec![StrokePoint::new(f64::INFINITY, 0.0)];
    assert!(!stroke.is_valid());
}

#[test]
fn the_fake_recognizer_answers_without_a_model() {
    let image = LineImage::new(384, 384, vec![255; 384 * 384]).expect("the model's input size");
    let recognizer = FixedRecognizer::new("Hello");
    assert_eq!(recognizer.recognize(&image), Ok("Hello".into()));
    assert_eq!(recognizer.model_id(), TROCR_SMALL_HANDWRITTEN.id);
    assert_eq!(recognizer.accelerator(), Accelerator::Cpu);
    assert!(LineImage::new(0, 1, vec![]).is_none());
    assert!(LineImage::new(1, 1, vec![0, 0]).is_none());
    assert_eq!(TROCR_SMALL_HANDWRITTEN.validate(), Ok(()));
}
