use super::*;
use egui::{Context, Event, Modifiers, PointerButton, RawInput, Rect, pos2, vec2};

#[test]
fn unit_conversions_match_exact_ratios() {
    assert_eq!(MeasureUnit::Points.points(), 1.0);
    assert_eq!(MeasureUnit::Picas.points(), 12.0);
    assert_eq!(MeasureUnit::Inches.points(), 72.0);
    assert!((MeasureUnit::Millimeters.points() - 72.0 / 25.4).abs() < 1e-9);
    assert!((MeasureUnit::Centimeters.points() - 72.0 / 2.54).abs() < 1e-9);
    assert!((MeasureUnit::Q.points() - 72.0 / 101.6).abs() < 1e-9);
    assert!((MeasureUnit::Ha.points() - 72.0 / 101.6).abs() < 1e-9);

    assert_eq!(MeasureUnit::Points.to_pt(10.0), 10.0);
    assert_eq!(MeasureUnit::Picas.to_pt(2.0), 24.0);
    assert_eq!(MeasureUnit::Inches.to_pt(1.5), 108.0);
}

#[test]
fn unit_parsing_and_arithmetic_expressions() {
    assert_eq!(MeasureUnit::Points.parse("10"), Some(10.0));
    assert_eq!(MeasureUnit::Points.parse("10pt"), Some(10.0));
    assert_eq!(MeasureUnit::Points.parse("1in"), Some(72.0));
    assert_eq!(MeasureUnit::Inches.parse("2"), Some(144.0));
    assert_eq!(MeasureUnit::Points.parse("10 + 5"), Some(15.0));
    assert_eq!(MeasureUnit::Points.parse("10 * 3"), Some(30.0));
    assert_eq!(MeasureUnit::Points.parse("10 x 3"), Some(30.0));
    assert_eq!(MeasureUnit::Points.parse("10 × 3"), Some(30.0));
    assert_eq!(MeasureUnit::Points.parse("100 / 4"), Some(25.0));

    // Picas and points: 2p6 = 2 * 12 + 6 = 30 points
    assert_eq!(MeasureUnit::Points.parse("2p6"), Some(30.0));
    assert_eq!(MeasureUnit::Points.parse("-2p6"), Some(-30.0));

    // Percentages after operators
    assert_eq!(MeasureUnit::Points.parse("50pt + 25%"), Some(62.5));
    assert_eq!(MeasureUnit::Points.parse("100pt - 20%"), Some(80.0));
    assert_eq!(MeasureUnit::Points.parse("10pt * 50%"), Some(5.0));

    // Decimal commas
    assert_eq!(MeasureUnit::Millimeters.parse("0,5 mm"), MeasureUnit::Millimeters.parse("0.5 mm"));
    assert_eq!(MeasureUnit::Points.parse("1,5"), Some(1.5));

    // Thousands commas refused so not misinterpreted as 1.0
    assert!(MeasureUnit::Points.parse("1,000").is_none());
    assert!(MeasureUnit::Points.parse("12,500 mm").is_none());
}

#[test]
fn units_named_lookup() {
    assert_eq!(MeasureUnit::named("pt"), Some(MeasureUnit::Points));
    assert_eq!(MeasureUnit::named("points"), Some(MeasureUnit::Points));
    assert_eq!(MeasureUnit::named("mm"), Some(MeasureUnit::Millimeters));
    assert_eq!(MeasureUnit::named("millimeters"), Some(MeasureUnit::Millimeters));
    assert_eq!(MeasureUnit::named("in"), Some(MeasureUnit::Inches));
    assert_eq!(MeasureUnit::named("inches"), Some(MeasureUnit::Inches));
    assert_eq!(MeasureUnit::named("picas"), Some(MeasureUnit::Picas));
    assert_eq!(MeasureUnit::named("pc"), Some(MeasureUnit::Picas));

    // East Asian
    assert_eq!(MeasureUnit::named("q"), None);
    assert_eq!(MeasureUnit::named_east_asian("q"), Some(MeasureUnit::Q));
    assert_eq!(MeasureUnit::named_east_asian("ha"), Some(MeasureUnit::Ha));
}

#[test]
fn scrubbing_drag_lifecycle_and_modifiers() {
    let ctx = Context::default();
    assert_eq!(Scrubber::phase(&ctx), ScrubPhase::Idle);

    let id = Id::unique("test-field");
    let bounds = 0.0..=100.0;
    let label_rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 20.0));
    let field_rect = Rect::from_min_size(pos2(55.0, 10.0), vec2(60.0, 20.0));

    let mut current_val = 10.0;
    let mut focused = false;

    // Frame 1: draw label and field
    let mut out = ctx.run_ui(RawInput::default(), |ui| {
        Scrubber::note_label(ui, label_rect);
        let params = FieldScrubParams::new(id, field_rect, "10", Some(current_val), 1, &bounds);
        let stepped = Scrubber::field(ui, params, |_, _, _| {
            focused = true;
        });
        assert_eq!(stepped, None);
    });
    out.textures_delta.clear();

    // Frame 2: mouse down on label
    let from = label_rect.center();
    let mut out = ctx.run_ui(
        RawInput {
            events: vec![
                Event::PointerMoved(from),
                Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::default() },
            ],
            ..Default::default()
        },
        |ui| {
            Scrubber::note_label(ui, label_rect);
            let params = FieldScrubParams::new(id, field_rect, "10", Some(current_val), 1, &bounds);
            let stepped = Scrubber::field(ui, params, |_, _, _| {
                focused = true;
            });
            assert_eq!(stepped, None);
        },
    );
    out.textures_delta.clear();

    // Frame 3: drag start
    let to = from + vec2(10.0, 0.0);
    let mut out = ctx.run_ui(RawInput { events: vec![Event::PointerMoved(to)], ..Default::default() }, |ui| {
        Scrubber::note_label(ui, label_rect);
        let params = FieldScrubParams::new(id, field_rect, "10", Some(current_val), 1, &bounds);
        let stepped = Scrubber::field(ui, params, |_, _, _| {
            focused = true;
        });
        if let Some(v) = stepped {
            current_val = v;
        }
    });
    out.textures_delta.clear();
    assert_eq!(Scrubber::phase(&ctx), ScrubPhase::Dragging);

    // Frame 4: drag continues (produces stepped value)
    let to2 = to + vec2(20.0, 0.0);
    let mut out = ctx.run_ui(RawInput { events: vec![Event::PointerMoved(to2)], ..Default::default() }, |ui| {
        Scrubber::note_label(ui, label_rect);
        let params = FieldScrubParams::new(id, field_rect, "10", Some(current_val), 1, &bounds);
        let stepped = Scrubber::field(ui, params, |_, _, _| {
            focused = true;
        });
        if let Some(v) = stepped {
            current_val = v;
        }
    });
    out.textures_delta.clear();
    assert!(current_val > 10.0);
    assert_eq!(Scrubber::phase(&ctx), ScrubPhase::Dragging);

    // Frame 5: release
    let mut out = ctx.run_ui(
        RawInput {
            events: vec![Event::PointerButton { pos: to2, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::default() }],
            ..Default::default()
        },
        |ui| {
            Scrubber::note_label(ui, label_rect);
            let params = FieldScrubParams::new(id, field_rect, "10", Some(current_val), 1, &bounds);
            let _ = Scrubber::field(ui, params, |_, _, _| {});
        },
    );
    out.textures_delta.clear();
    assert_eq!(Scrubber::phase(&ctx), ScrubPhase::Released);
}

#[test]
fn thousands_separator_validation_in_expressions() {
    let pts = MeasureUnit::Points;
    let mm = MeasureUnit::Millimeters;

    // Ambiguous grouping in operand is rejected
    assert!(pts.parse("12,500 mm + 1mm").is_none());
    assert!(pts.parse("1mm + 12,500 mm").is_none());
    assert!(pts.parse("12,500 mm").is_none());

    // Valid decimal commas in expressions succeed
    let val = mm.parse("0,5 mm + 1mm");
    assert!(val.is_some());
    let expected = mm.to_pt(1.5);
    assert!((val.unwrap() - expected).abs() < 1e-4);

    let val2 = mm.parse("1,25 mm + 0,75 mm");
    assert!(val2.is_some());
    let expected2 = mm.to_pt(2.0);
    assert!((val2.unwrap() - expected2).abs() < 1e-4);
}

#[test]
fn operator_limit_enforced_in_parse_measure() {
    let mut expr = "1".to_string();
    for _ in 0..129 {
        expr.push_str(" + 1");
    }
    assert!(parse_measure(&expr, MeasureUnit::Points).is_none());

    let mut ok_expr = "1".to_string();
    for _ in 0..128 {
        ok_expr.push_str(" + 1");
    }
    assert_eq!(parse_measure(&ok_expr, MeasureUnit::Points), Some(129.0));
}

#[test]
fn label_registration_scoped_to_container_id() {
    let run_drag_in_layers = |field_in_different_layer: bool| -> f64 {
        let ctx = Context::default();
        let label_rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 20.0));
        let field_rect = Rect::from_min_size(pos2(55.0, 10.0), vec2(60.0, 20.0));
        let bounds = 0.0..=100.0;
        let id = Id::unique("field-layer");
        let mut val = 10.0;
        let center = label_rect.center();

        let layer1 = egui::LayerId::new(egui::Order::Middle, Id::unique("container_1"));
        let layer2 = if field_in_different_layer { egui::LayerId::new(egui::Order::Middle, Id::unique("container_2")) } else { layer1 };

        let run_frame = |ctx: &Context, events: Vec<Event>, val: &mut f64| {
            let mut out = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
                ui.scope_builder(egui::UiBuilder::new().layer_id(layer1), |ui1| {
                    Scrubber::note_label(ui1, label_rect);
                });
                ui.scope_builder(egui::UiBuilder::new().layer_id(layer2), |ui2| {
                    let params = FieldScrubParams::new(id, field_rect, "10", Some(*val), 1, &bounds);
                    if let Some(v) = Scrubber::field(ui2, params, |_, _, _| {}) {
                        *val = v;
                    }
                });
            });
            out.textures_delta.clear();
        };

        // Frame 1: initial layout
        run_frame(&ctx, vec![], &mut val);

        // Frame 2: mouse down on label
        run_frame(
            &ctx,
            vec![
                Event::PointerMoved(center),
                Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::default() },
            ],
            &mut val,
        );

        // Frame 3: drag start
        let moved = center + vec2(10.0, 0.0);
        run_frame(&ctx, vec![Event::PointerMoved(moved)], &mut val);

        // Frame 4: drag continues
        let drag_pos = moved + vec2(40.0, 0.0);
        run_frame(&ctx, vec![Event::PointerMoved(drag_pos)], &mut val);

        val
    };

    let diff_layer_val = run_drag_in_layers(true);
    assert_eq!(diff_layer_val, 10.0, "field in container_2 must not change when label is in container_1");

    let same_layer_val = run_drag_in_layers(false);
    assert!(same_layer_val > 10.0, "same container control must change value when dragged");

    // Interleaved draw: container_1 notes label, container_2 draws unrelated field,
    // then container_1 draws its intended field in the same pass.
    let (interleaved_unrelated_val, interleaved_owning_val) = {
        let ctx = Context::default();
        let label_rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 20.0));
        let field_rect = Rect::from_min_size(pos2(55.0, 10.0), vec2(60.0, 20.0));
        let bounds = 0.0..=100.0;
        let id_unrelated = Id::unique("field-unrelated");
        let id_owning = Id::unique("field-owning");
        let mut val_unrelated = 10.0;
        let mut val_owning = 10.0;
        let center = label_rect.center();

        let layer1 = egui::LayerId::new(egui::Order::Middle, Id::unique("container_1"));
        let layer2 = egui::LayerId::new(egui::Order::Middle, Id::unique("container_2"));

        let run_interleaved_frame = |ctx: &Context, events: Vec<Event>, val_unrelated: &mut f64, val_owning: &mut f64| {
            let mut out = ctx.run_ui(RawInput { events, ..Default::default() }, |ui| {
                // 1. Container 1 notes label
                ui.scope_builder(egui::UiBuilder::new().layer_id(layer1), |ui1| {
                    Scrubber::note_label(ui1, label_rect);
                });
                // 2. Container 2 draws an unrelated numeric field at adjacent coordinates
                ui.scope_builder(egui::UiBuilder::new().layer_id(layer2), |ui2| {
                    let params = FieldScrubParams::new(id_unrelated, field_rect, "10", Some(*val_unrelated), 1, &bounds);
                    if let Some(v) = Scrubber::field(ui2, params, |_, _, _| {}) {
                        *val_unrelated = v;
                    }
                });
                // 3. Container 1 draws its intended numeric field
                ui.scope_builder(egui::UiBuilder::new().layer_id(layer1), |ui1| {
                    let params = FieldScrubParams::new(id_owning, field_rect, "10", Some(*val_owning), 1, &bounds);
                    if let Some(v) = Scrubber::field(ui1, params, |_, _, _| {}) {
                        *val_owning = v;
                    }
                });
            });
            out.textures_delta.clear();
        };

        run_interleaved_frame(&ctx, vec![], &mut val_unrelated, &mut val_owning);
        run_interleaved_frame(
            &ctx,
            vec![
                Event::PointerMoved(center),
                Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::default() },
            ],
            &mut val_unrelated,
            &mut val_owning,
        );
        let moved = center + vec2(10.0, 0.0);
        run_interleaved_frame(&ctx, vec![Event::PointerMoved(moved)], &mut val_unrelated, &mut val_owning);
        let drag_pos = moved + vec2(40.0, 0.0);
        run_interleaved_frame(&ctx, vec![Event::PointerMoved(drag_pos)], &mut val_unrelated, &mut val_owning);

        (val_unrelated, val_owning)
    };

    assert_eq!(interleaved_unrelated_val, 10.0, "interleaved field in container_2 must not consume container_1 label");
    assert!(interleaved_owning_val > 10.0, "owning field in container_1 must retain label binding and change value when dragged");
}

#[test]
fn decimal_point_linear_scan_performance_and_leading_signs() {
    let pts = MeasureUnit::Points;

    // Linear scan handles repeated commas without pathological slowdown
    let repeated = "0,".repeat(5_000);
    let normalized = decimal_point(&repeated);
    assert!(normalized.starts_with("0.0."));

    // Leading signs before decimal numbers
    assert_eq!(pts.parse("-0,5 mm + +0,5 mm"), Some(0.0));
    assert_eq!(pts.parse("-1,5 pt"), Some(-1.5));

    // Unit-ending 'x' (e.g. px) does not treat following minus as a leading sign
    assert!(pts.parse("1px - 12,500mm").is_none());
    let sub = pts.parse("1px - 0,5mm");
    assert!(sub.is_some());
    assert!((sub.unwrap() - (1.0 - MeasureUnit::Millimeters.to_pt(0.5))).abs() < 1e-4);
}

#[test]
fn modifier_drags_scale_step_sizes_differently() {
    let drag_with_modifier = |mods: Modifiers| -> f64 {
        let ctx = Context::default();
        let label_rect = Rect::from_min_size(pos2(10.0, 10.0), vec2(40.0, 20.0));
        let field_rect = Rect::from_min_size(pos2(55.0, 10.0), vec2(60.0, 20.0));
        let bounds = 0.0..=1000.0;
        let id = Id::unique("field-mod");
        let mut val = 100.0;

        let center = label_rect.center();
        // Frame 1: initial layout
        let mut out0 = ctx.run_ui(RawInput::default(), |ui| {
            Scrubber::note_label(ui, label_rect);
            let params = FieldScrubParams::new(id, field_rect, "100.0", Some(val), 1, &bounds);
            let _ = Scrubber::field(ui, params, |_, _, _| {});
        });
        out0.textures_delta.clear();

        // Frame 2: press on label
        let mut out1 = ctx.run_ui(
            RawInput {
                events: vec![
                    Event::ModifiersChanged(mods),
                    Event::PointerMoved(center),
                    Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: true, modifiers: mods },
                ],
                ..Default::default()
            },
            |ui| {
                Scrubber::note_label(ui, label_rect);
                let params = FieldScrubParams::new(id, field_rect, "100.0", Some(val), 1, &bounds);
                let _ = Scrubber::field(ui, params, |_, _, _| {});
            },
        );
        out1.textures_delta.clear();

        // Frame 3: drag started
        let moved_pos = center + vec2(10.0, 0.0);
        let mut out2 =
            ctx.run_ui(RawInput { events: vec![Event::ModifiersChanged(mods), Event::PointerMoved(moved_pos)], ..Default::default() }, |ui| {
                Scrubber::note_label(ui, label_rect);
                let params = FieldScrubParams::new(id, field_rect, "100.0", Some(val), 1, &bounds);
                if let Some(v) = Scrubber::field(ui, params, |_, _, _| {}) {
                    val = v;
                }
            });
        out2.textures_delta.clear();

        // Frame 4: drag continues
        let drag_pos = moved_pos + vec2(40.0, 0.0);
        let mut out3 =
            ctx.run_ui(RawInput { events: vec![Event::ModifiersChanged(mods), Event::PointerMoved(drag_pos)], ..Default::default() }, |ui| {
                Scrubber::note_label(ui, label_rect);
                let params = FieldScrubParams::new(id, field_rect, "100.0", Some(val), 1, &bounds);
                if let Some(v) = Scrubber::field(ui, params, |_, _, _| {}) {
                    val = v;
                }
            });
        out3.textures_delta.clear();
        val
    };

    let base = drag_with_modifier(Modifiers::default());
    let with_shift = drag_with_modifier(Modifiers::SHIFT);
    let with_cmd = drag_with_modifier(Modifiers::COMMAND);

    let delta_base = (base - 100.0).abs();
    let delta_shift = (with_shift - 100.0).abs();
    let delta_cmd = (with_cmd - 100.0).abs();

    assert!(delta_shift > delta_base, "Shift should increase delta: shift={delta_shift}, base={delta_base}");
    assert!(delta_cmd < delta_base, "Command should decrease delta: cmd={delta_cmd}, base={delta_base}");
}
