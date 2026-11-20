use super::*;
use egui::{Context, FullOutput, Pos2, RawInput, vec2};

fn zone(lit: Color32) -> MeterZone {
    MeterZone { lit, unlit: lit.gamma_multiply(0.2) }
}

fn style() -> LevelMeterStyle {
    LevelMeterStyle {
        scale: MeterScale::default(),
        low: zone(Color32::GREEN),
        mid: zone(Color32::YELLOW),
        high: zone(Color32::RED),
        mid_from_db: -20.0,
        high_from_db: -9.0,
        rms: Stroke::new(1.0, Color32::BLACK),
        hold: Stroke::new(2.0, Color32::WHITE),
    }
}

fn lit_area(segments: &[(Rect, Color32)], style: &LevelMeterStyle) -> f32 {
    let lit = [style.low.lit, style.mid.lit, style.high.lit];
    segments.iter().filter(|(_, c)| lit.contains(c)).map(|(r, _)| r.area()).sum()
}

#[test]
fn decibels_follow_the_field_level_definition_both_ways() {
    // ISO 80000-3 level of a field quantity: L = 20 lg(F / F0) dB; half amplitude is -6.0206 dB.
    assert!((amplitude_to_db(0.5) + 6.020_6).abs() < 1e-3);
    assert!((amplitude_to_db(0.1) + 20.0).abs() < 1e-4);
    assert!(amplitude_to_db(1.0).abs() < 1e-6);
    assert_eq!(amplitude_to_db(-0.5), amplitude_to_db(0.5));
    assert_eq!(amplitude_to_db(0.0), f32::NEG_INFINITY);
    assert_eq!(db_to_amplitude(f32::NEG_INFINITY), 0.0);
    for amplitude in [1e-6, 0.001, 0.25, 0.707, 1.0, 2.0] {
        assert!((db_to_amplitude(amplitude_to_db(amplitude)) - amplitude).abs() <= amplitude * 1e-5);
    }
}

#[test]
fn scale_rejects_empty_inverted_and_unbounded_ranges() {
    assert!(MeterScale::new(-60.0, 0.0).is_some());
    assert!(MeterScale::new(0.0, 0.0).is_none());
    assert!(MeterScale::new(0.0, -60.0).is_none());
    assert!(MeterScale::new(f32::NEG_INFINITY, 0.0).is_none());
    assert!(MeterScale::new(-60.0, f32::NAN).is_none());
    assert!(MeterScale::new(-1000.0, 0.0).is_none());
}

#[test]
fn scale_positions_rise_with_level_and_stay_on_the_bar() {
    let scale = MeterScale::new(-48.0, 6.0).unwrap();
    assert_eq!(scale.position(f32::NEG_INFINITY), 0.0);
    assert_eq!(scale.position(f32::NAN), 0.0);
    assert_eq!(scale.position(-100.0), 0.0);
    assert_eq!(scale.position(f32::INFINITY), 1.0);
    let mut last = 0.0;
    for db in [-40.0, -20.0, -6.0, 0.0, 6.0] {
        let at = scale.position(db);
        assert!(at > last, "{db} dB at {at}");
        last = at;
    }
    assert_eq!(scale.clamp(f32::NAN), -48.0);
}

#[test]
fn lit_part_grows_with_level_and_zones_cover_the_bar() {
    let style = style();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(200.0, 10.0));
    let mut last = -1.0;
    for db in [f32::NEG_INFINITY, -50.0, -15.0, -3.0, 0.0] {
        let parts = segments(&style, rect, MeterOrientation::Horizontal, db);
        let total: f32 = parts.iter().map(|(r, _)| r.area()).sum();
        assert!((total - rect.area()).abs() < 1e-2, "zones cover the bar at {db} dB");
        assert!(parts.iter().all(|(r, _)| rect.expand(1e-3).contains_rect(*r)));
        let lit = lit_area(&parts, &style);
        assert!(lit > last, "lit area grows at {db} dB");
        last = lit;
    }
    let loud = segments(&style, rect, MeterOrientation::Horizontal, -3.0);
    assert!(loud.iter().any(|(_, c)| *c == style.high.lit), "a level above high_from_db lights the high zone");
    let quiet = segments(&style, rect, MeterOrientation::Horizontal, -30.0);
    assert!(!quiet.iter().any(|(_, c)| *c == style.mid.lit || *c == style.high.lit));
}

#[test]
fn vertical_meter_fills_from_the_bottom() {
    let style = style();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(8.0, 120.0));
    let parts = segments(&style, rect, MeterOrientation::Vertical, -40.0);
    let (lit, _) = parts.iter().find(|(_, c)| *c == style.low.lit).unwrap();
    assert_eq!(lit.bottom(), rect.bottom());
    assert!(lit.top() > rect.center().y);
    assert_eq!(lit.width(), rect.width());
}

#[test]
fn zone_boundaries_out_of_order_or_off_the_scale_still_cover_the_bar() {
    let mut style = style();
    style.mid_from_db = 10.0;
    style.high_from_db = -200.0;
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 6.0));
    let parts = segments(&style, rect, MeterOrientation::Horizontal, -10.0);
    let total: f32 = parts.iter().map(|(r, _)| r.area()).sum();
    assert!((total - rect.area()).abs() < 1e-2);
    assert!(parts.iter().all(|(r, _)| r.width() >= 0.0));
}

#[test]
fn marks_sit_on_the_bar_and_vanish_at_the_floor() {
    let scale = MeterScale::default();
    let rect = Rect::from_min_size(Pos2::ZERO, vec2(100.0, 10.0));
    assert!(mark(&scale, rect, MeterOrientation::Horizontal, f32::NEG_INFINITY).is_none());
    assert!(mark(&scale, rect, MeterOrientation::Horizontal, -80.0).is_none());
    let [a, b] = mark(&scale, rect, MeterOrientation::Horizontal, -30.0).unwrap();
    assert_eq!(a.x, b.x);
    assert!(a.x > rect.left() && a.x < rect.right());
    let louder = mark(&scale, rect, MeterOrientation::Horizontal, -6.0).unwrap();
    assert!(louder[0].x > a.x);
}

#[test]
fn peak_holds_then_falls_and_never_drops_below_the_level() {
    let mut hold = PeakHold::new(1.0, 20.0).unwrap();
    assert_eq!(hold.update(-6.0, 0.0), -6.0);
    assert_eq!(hold.update(-30.0, 0.5), -6.0, "held during the hold time");
    assert_eq!(hold.update(-30.0, 1.0), -6.0);
    let falling = hold.update(-30.0, 1.5);
    assert!(falling < -6.0 && falling > -30.0, "falls after the hold time: {falling}");
    assert!(hold.update(-30.0, 2.0) < falling);
    assert_eq!(hold.update(-30.0, 10.0), -30.0, "stops at the current level");
    assert_eq!(hold.update(-3.0, 10.1), -3.0, "a louder level is taken at once");
}

#[test]
fn peak_hold_handles_silence_bad_input_and_a_clock_that_goes_back() {
    let mut hold = PeakHold::new(0.5, 10.0).unwrap();
    assert_eq!(hold.peak_db(), f32::NEG_INFINITY);
    assert_eq!(hold.update(f32::NAN, 0.0), f32::NEG_INFINITY);
    assert_eq!(hold.update(-12.0, 5.0), -12.0);
    assert_eq!(hold.update(-40.0, f64::NAN), -12.0, "a non-finite time changes nothing");
    assert_eq!(hold.update(-40.0, 1.0), -12.0, "a clock that went back restarts the hold");
    assert_eq!(hold.update(-40.0, 1.4), -12.0);
    assert!(hold.update(-40.0, 2.0) < -12.0);
    hold.reset();
    assert_eq!(hold.peak_db(), f32::NEG_INFINITY);
    assert_eq!(hold.update(-50.0, 2.1), -50.0);
}

#[test]
fn a_clock_that_goes_back_while_the_peak_falls_does_not_raise_it() {
    let mut hold = PeakHold::new(1.0, 20.0).unwrap();
    hold.update(-6.0, 0.0);
    assert_eq!(hold.update(-60.0, 3.0), -46.0);
    assert_eq!(hold.update(-60.0, 2.0), -46.0, "the peak stays where it was");
    assert_eq!(hold.update(-60.0, 2.5), -46.0, "and the hold time restarts");
    assert!(hold.update(-60.0, 3.5) < -46.0);
}

#[test]
fn a_louder_level_after_the_clock_goes_back_is_taken_at_once() {
    let mut hold = PeakHold::new(0.5, 10.0).unwrap();
    assert_eq!(hold.update(-12.0, 5.0), -12.0);
    assert_eq!(hold.update(-3.0, 1.0), -3.0, "the peak is not left below the level");
    assert_eq!(hold.update(-40.0, 1.4), -3.0, "the hold time runs from the louder level");
    assert!(hold.update(-40.0, 2.0) < -3.0);
}

#[test]
fn peak_hold_without_fall_keeps_the_peak_until_reset() {
    let mut hold = PeakHold::new(0.0, 0.0).unwrap();
    hold.update(-1.0, 0.0);
    assert_eq!(hold.update(f32::NEG_INFINITY, 1e6), -1.0);
    hold.reset();
    assert_eq!(hold.update(f32::NEG_INFINITY, 1e6 + 1.0), f32::NEG_INFINITY);
}

#[test]
fn peak_hold_rejects_negative_and_non_finite_settings() {
    assert!(PeakHold::new(0.0, 0.0).is_some());
    for (hold_secs, fall) in [(-1.0, 10.0), (f32::NAN, 10.0), (f32::INFINITY, 10.0), (1.0, -5.0), (1.0, f32::NAN), (1.0, f32::INFINITY)] {
        assert!(PeakHold::new(hold_secs, fall).is_none(), "{hold_secs} s, {fall} dB/s");
    }
}

#[test]
fn an_infinite_level_does_not_pin_the_held_peak() {
    let mut hold = PeakHold::new(1.0, 20.0).unwrap();
    assert_eq!(hold.update(amplitude_to_db(f32::INFINITY), 0.0), PeakHold::MAX_LEVEL_DB);
    assert_eq!(hold.update(1e30, 0.5), PeakHold::MAX_LEVEL_DB);
    let falling = hold.update(-6.0, 2.5);
    assert!(falling.is_finite() && falling < PeakHold::MAX_LEVEL_DB, "{falling}");
    assert_eq!(hold.update(-6.0, 1000.0), -6.0, "the peak falls back to a finite level");
}

fn run(mut meter: impl FnMut(&mut Ui) -> Response) -> (Response, FullOutput) {
    let ctx = Context::default();
    ctx.enable_accesskit();
    let mut response = None;
    let mut output = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(400.0, 200.0))), ..Default::default() }, |ui| {
        response = Some(meter(ui));
    });
    output.textures_delta.clear();
    (response.unwrap(), output)
}

#[test]
fn screen_readers_get_a_meter_with_its_range_and_level() {
    let style = style();
    let (_, output) =
        run(|ui| LevelMeter::new("Microphone left", vec2(120.0, 6.0), &style).level_db(-12.345).hold_db(-3.0).value_text("-12.3 dB").show(ui));
    let tree = output.platform_output.accesskit_update.unwrap();
    let (_, node) = tree.nodes.iter().find(|(_, n)| n.label() == Some("Microphone left")).unwrap();
    assert_eq!(node.role(), accesskit::Role::Meter);
    assert_eq!(node.numeric_value(), Some(-12.3));
    assert_eq!(node.min_numeric_value(), Some(-60.0));
    assert_eq!(node.max_numeric_value(), Some(0.0));
    assert_eq!(node.value(), Some("-12.3 dB"));
    assert_eq!(node.orientation(), Some(accesskit::Orientation::Horizontal));
}

#[test]
fn the_rounded_level_stays_within_a_scale_that_is_not_whole_tenths() {
    let style = LevelMeterStyle { scale: MeterScale::new(-59.96, -0.04).unwrap(), ..style() };
    for (label, level) in [("Loud", 0.0), ("Silent", f32::NEG_INFINITY)] {
        let (_, output) = run(|ui| LevelMeter::new(label, vec2(120.0, 6.0), &style).level_db(level).show(ui));
        let tree = output.platform_output.accesskit_update.unwrap();
        let (_, node) = tree.nodes.iter().find(|(_, n)| n.label() == Some(label)).unwrap();
        let (value, min, max) = (node.numeric_value().unwrap(), node.min_numeric_value().unwrap(), node.max_numeric_value().unwrap());
        assert!(value >= min && value <= max, "{label}: {value} outside {min}..={max}");
    }
}

#[test]
fn a_silent_meter_reports_the_floor_and_a_bad_size_takes_no_space() {
    let style = style();
    let (_, output) = run(|ui| LevelMeter::new("Desktop audio", vec2(8.0, 80.0), &style).orientation(MeterOrientation::Vertical).show(ui));
    let tree = output.platform_output.accesskit_update.unwrap();
    let (_, node) = tree.nodes.iter().find(|(_, n)| n.label() == Some("Desktop audio")).unwrap();
    assert_eq!(node.numeric_value(), Some(-60.0));
    assert_eq!(node.orientation(), Some(accesskit::Orientation::Vertical));
    for size in [vec2(f32::NAN, 6.0), vec2(0.0, 6.0), vec2(120.0, -1.0), vec2(5000.0, 6.0)] {
        let (response, _) = run(|ui| LevelMeter::new("Bad", size, &style).level_db(-6.0).show(ui));
        assert_eq!(response.rect.size(), Vec2::ZERO, "{size:?}");
    }
}
