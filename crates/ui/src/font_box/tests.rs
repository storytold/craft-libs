use super::*;

#[test]
fn sizes_format_without_a_trailing_zero() {
    assert_eq!(format_size(11.0), "11");
    assert_eq!(format_size(10.5), "10.5");
}

#[test]
fn typed_sizes_round_to_half_points_and_clamp() {
    assert_eq!(parse_size("12"), Some(12.0));
    assert_eq!(parse_size("10,5"), Some(10.5));
    assert_eq!(parse_size(" 11.3 pt"), Some(11.5));
    assert_eq!(parse_size("0"), Some(MIN_SIZE));
    assert_eq!(parse_size("99999"), Some(MAX_SIZE));
    assert_eq!(parse_size("big"), None);
    assert_eq!(parse_size("NaN"), None);
}

#[test]
fn grow_and_shrink_walk_the_preset_list_then_single_points() {
    assert_eq!(step(Some(11.0), DEFAULT_SIZES, true), 12.0);
    assert_eq!(step(Some(11.0), DEFAULT_SIZES, false), 10.5);
    assert_eq!(step(Some(13.0), DEFAULT_SIZES, true), 14.0);
    assert_eq!(step(Some(72.0), DEFAULT_SIZES, true), 73.0);
    assert_eq!(step(Some(8.0), DEFAULT_SIZES, false), 7.0);
    assert_eq!(step(Some(1.0), DEFAULT_SIZES, false), MIN_SIZE);
    assert_eq!(step(None, DEFAULT_SIZES, true), 12.0, "a mixed size starts from 11");
}

fn run(ctx: &egui::Context, family: &mut String, size: &mut Option<f32>, events: Vec<egui::Event>) -> (FontBoxResponse, egui::FullOutput) {
    let style = FontBoxStyle::from_visuals(&egui::Visuals::light());
    let families = vec!["Inter".to_string(), "Georgia".to_string(), "Menlo".to_string()];
    let labels =
        FontBoxLabels { font: "Font", size: "Font Size", grow: "Increase Font Size", shrink: "Decrease Font Size", clear: "Clear Formatting" };
    let mut out = FontBoxResponse::default();
    let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(500.0, 400.0))), events, ..Default::default() };
    let mut full = ctx.run_ui(input, |ui| out = FontBox::new(family, size, &families, &style, labels).clear(true).show(ui));
    full.textures_delta.clear();
    (out, full)
}

/// The centre of the accessible node with this label.
fn centre(full: &egui::FullOutput, label: &str) -> egui::Pos2 {
    let update = full.platform_output.accesskit_update.as_ref().expect("accesskit is on");
    let node = update.nodes.iter().map(|(_, n)| n).find(|n| n.label() == Some(label)).expect("node with the label");
    let b = node.bounds().expect("bounds");
    egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
}

#[test]
fn grow_button_steps_the_size_and_reports_it() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let (mut family, mut size) = ("Inter".to_string(), Some(11.0));
    let (_, full) = run(&ctx, &mut family, &mut size, vec![]);
    let p = centre(&full, "Increase Font Size");
    run(&ctx, &mut family, &mut size, vec![egui::Event::PointerMoved(p), click(p, true)]);
    let (r, _) = run(&ctx, &mut family, &mut size, vec![click(p, false)]);
    assert!(r.size_changed);
    assert_eq!(size, Some(12.0));
}

#[test]
fn a_mixed_selection_shows_blank_fields_and_changes_nothing_by_itself() {
    let ctx = egui::Context::default();
    let (mut family, mut size) = (String::new(), None);
    for _ in 0..3 {
        let (r, _) = run(&ctx, &mut family, &mut size, vec![]);
        assert!(!r.family_changed && !r.size_changed && !r.clear_clicked);
    }
    assert_eq!((family.as_str(), size), ("", None));
}

fn click(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
}

#[test]
fn a_disabled_box_shows_its_hint_and_ignores_clicks() {
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let style = FontBoxStyle::from_visuals(&egui::Visuals::light());
    let labels =
        FontBoxLabels { font: "Font", size: "Font Size", grow: "Increase Font Size", shrink: "Decrease Font Size", clear: "Clear Formatting" };
    let (mut family, mut size) = (String::new(), None);
    let mut frame = |events: Vec<egui::Event>| {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(500.0, 400.0))), events, ..Default::default() };
        let mut out = FontBoxResponse::default();
        let mut full =
            ctx.run_ui(input, |ui| out = FontBox::new(&mut family, &mut size, &[], &style, labels).enabled(false).hint("Inter", 11.0).show(ui));
        full.textures_delta.clear();
        (out, full)
    };
    let (_, full) = frame(vec![]);
    let p = centre(&full, "Increase Font Size");
    frame(vec![egui::Event::PointerMoved(p), click(p, true)]);
    let (r, full) = frame(vec![click(p, false)]);
    assert!(!r.size_changed, "a disabled box does not change");
    let update = full.platform_output.accesskit_update.expect("accesskit");
    assert!(update.nodes.iter().any(|(_, n)| n.value() == Some("Inter")), "the hint is shown");
}
