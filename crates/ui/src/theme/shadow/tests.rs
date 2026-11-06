use super::*;
use egui::{CornerRadius, Pos2, RawInput, Stroke, StrokeKind, pos2, vec2};

fn rect() -> Rect {
    Rect::from_min_size(pos2(30.0, 40.0), vec2(160.0, 80.0))
}

fn source() -> Shadow {
    Shadow { offset: [0, 4], blur: 16, spread: 0, color: Color32::from_black_alpha(100) }
}

fn rule() -> FrameShadow {
    FrameShadow::new(source(), Elevation::Medium.layers(source().color))
}

fn original(rect: Rect) -> Shape {
    Frame::NONE.fill(Color32::WHITE).stroke(Stroke::new(1.0, Color32::GRAY)).corner_radius(6).shadow(source()).paint(rect.shrink(1.0))
}

fn faded_original(source: Shadow, opacity: f32) -> Shape {
    let mut shape =
        Frame::NONE.fill(Color32::WHITE).stroke(Stroke::new(1.0, Color32::GRAY)).corner_radius(6).shadow(source).paint(rect().shrink(1.0));
    let Shape::Vec(pair) = &mut shape else { panic!("expected frame and shadow") };
    for shape in pair {
        if let Shape::Rect(rect) = shape {
            rect.fill = rect.fill.gamma_multiply(opacity);
            rect.stroke.color = rect.stroke.color.gamma_multiply(opacity);
        }
    }
    shape
}

fn rects(shape: &Shape, out: &mut Vec<egui::epaint::RectShape>) {
    match shape {
        Shape::Rect(rect) => out.push(rect.clone()),
        Shape::Vec(shapes) => {
            for shape in shapes {
                rects(shape, out);
            }
        }
        _ => {}
    }
}

#[test]
fn semantic_tokens_preserve_reference_recipes_and_support_flat_and_floating() {
    let color = Color32::from_rgba_unmultiplied(40, 100, 160, 100);
    for (semantic, compatibility) in
        [(Elevation::Subtle, Elevation::Low), (Elevation::Raised, Elevation::Medium), (Elevation::Overlay, Elevation::High)]
    {
        assert_eq!(semantic.shadow(color).as_slice(), compatibility.layers(color));
    }
    let flat = Elevation::Flat.shadow(color);
    assert_eq!(flat, ElevationShadow::NONE);
    assert_eq!(flat, ElevationShadow::default());
    assert!(shapes(rect(), CornerRadius::ZERO, flat.as_ref()).is_empty());
    let [far, near] = floating_toolbar(color);
    assert_eq!(Elevation::Floating.shadow(color), ElevationShadow::new([far, near, ShadowLayer::NONE, ShadowLayer::NONE]));
    assert_eq!(shapes(rect(), CornerRadius::ZERO, Elevation::Floating.shadow(color).as_ref()).len(), 2);
    let elevations = [Elevation::Subtle, Elevation::Raised, Elevation::Overlay, Elevation::Modal];
    assert!(elevations.windows(2).all(|pair| pair[0].layers(color)[0].blur < pair[1].layers(color)[0].blur));
}

#[test]
fn renderer_retains_color_radius_clip_and_painter_opacity_once() {
    let ctx = Context::default();
    let clip = rect().expand(50.0);
    let mut painter = ctx.layer_painter(LayerId::new(egui::Order::Foreground, egui::Id::unique("owned"))).with_clip_rect(clip);
    let clip = painter.clip_rect();
    painter.set_opacity(0.5);
    let color = Color32::from_rgba_unmultiplied(40, 100, 160, 100);
    let radius = CornerRadius { nw: 2, ne: 3, sw: 4, se: 5 };
    let layers = [Shadow { color, ..source() }];
    paint(&painter, rect(), radius, &layers);
    ctx.graphics(|graphics| {
        let entry = graphics.get(painter.layer_id()).unwrap().all_entries().next().unwrap();
        assert_eq!(entry.clip_rect, clip);
        let Shape::Rect(shadow) = &entry.shape else { panic!("expected shadow rectangle") };
        assert_eq!(shadow.fill, color.gamma_multiply(0.5));
        assert_eq!(shadow.corner_radius, radius);
    });
    assert_eq!(visual_bounds(rect(), &layers), rect().union(rect() + source().margin()));
}

#[test]
fn invalid_and_empty_inputs_do_not_paint() {
    let layer = [source()];
    for invalid in [
        Rect::NOTHING,
        Rect::from_min_size(Pos2::ZERO, vec2(0.0, 10.0)),
        Rect::from_min_size(Pos2::ZERO, vec2(f32::NAN, 10.0)),
        Rect::from_min_max(pos2(-f32::MAX, 0.0), pos2(f32::MAX, 10.0)),
    ] {
        assert!(shapes(invalid, CornerRadius::ZERO, &layer).is_empty());
        assert_eq!(visual_bounds(invalid, &layer), Rect::NOTHING);
    }
    assert!(shapes(rect(), CornerRadius::ZERO, &[]).is_empty());
    assert!(shapes(rect(), CornerRadius::ZERO, &[Shadow::NONE]).is_empty());
    assert_eq!(visual_bounds(rect(), &[Shadow::NONE]), rect());
}

#[test]
fn adapter_requires_complete_registered_signature_and_exact_caster() {
    let original = original(rect());
    let converted = replacement_shape(&original, rect(), &[rule()], None).unwrap();
    let Shape::Vec(converted) = converted else { panic!("expected frame") };
    assert_eq!(converted.len(), 5);
    assert_eq!(
        converted.last(),
        match &original {
            Shape::Vec(pair) => pair.last(),
            _ => None,
        }
    );
    assert!(replacement_shape(&Shape::rect_filled(rect(), 6, Color32::WHITE), rect(), &[rule()], None).is_none());
    assert!(replacement_shape(&original, rect().expand(1.0), &[rule()], None).is_none());
    assert!(replacement_shape(&original, rect(), &[FrameShadow::new(Shadow { blur: 17, ..source() }, vec![])], None).is_none());
    let Shape::Vec(mut pair) = original else { panic!() };
    pair.insert(0, Shape::Noop);
    assert!(replacement_shape(&Shape::Vec(pair), rect(), &[rule()], None).is_none());
}

#[test]
fn adapter_preserves_fade_and_removes_only_the_owned_shadow() {
    let original = faded_original(source(), 0.5);
    let converted = replacement_shape(&original, rect(), &[rule()], None).unwrap();
    let Shape::Vec(layers) = converted else { panic!() };
    for (shape, layer) in layers.iter().zip(rule().layers) {
        let Shape::Rect(shadow) = shape else { panic!() };
        assert_eq!(shadow.fill, layer.color.gamma_multiply(0.5));
    }
    assert!(replacement_shape(&Shape::Vec(layers), rect(), &[rule()], None).is_none(), "must not repeatedly append shadows");
    let removed = replacement_shape(&original, rect(), &[rule()], Some(&LayerOverride { source: None, layers: vec![] })).unwrap();
    let Shape::Vec(remaining) = removed else { panic!() };
    assert_eq!(remaining.len(), 1);
    let Shape::Vec(original) = original else { panic!() };
    assert_eq!(remaining.last(), original.last());
}

#[test]
fn alpha_only_signatures_require_equivalent_results_or_an_explicit_source() {
    let source = source();
    let rules = [40, 100].map(|alpha| {
        let source = Shadow { color: Color32::from_black_alpha(alpha), ..source };
        FrameShadow::new(source, Elevation::Raised.layers(source.color))
    });
    let original = faded_original(source, 0.4);
    let mut override_ = LayerOverride { source: None, layers: vec![Shadow { color: Color32::from_black_alpha(90), ..source }] };
    let equivalent = replacement_shape(&original, rect(), &rules, None).unwrap();
    let reversed = [rules[1].clone(), rules[0].clone()];
    assert_eq!(replacement_shape(&original, rect(), &reversed, None), Some(equivalent));
    assert!(replacement_shape(&original, rect(), &rules, Some(&override_)).is_none());
    assert!(replacement_shape(&original, rect(), &reversed, Some(&override_)).is_none());

    override_.source = Some(source);
    let converted = replacement_shape(&original, rect(), &rules, Some(&override_)).unwrap();
    assert_eq!(replacement_shape(&original, rect(), &reversed, Some(&override_)), Some(converted.clone()));
    let Shape::Vec(pair) = converted else { panic!() };
    let Shape::Rect(shadow) = &pair[0] else { panic!() };
    assert_eq!(shadow.fill, Color32::from_black_alpha(36));

    override_.source = Some(Shadow { blur: source.blur + 1, ..source });
    assert!(replacement_shape(&original, rect(), &rules, Some(&override_)).is_none(), "the selected source must be registered");
}

#[test]
fn conflicting_recipes_do_not_depend_on_registration_order() {
    let original = original(rect());
    let rules = [rule(), FrameShadow::new(source(), Elevation::Modal.layers(source().color))];
    assert!(replacement_shape(&original, rect(), &rules, None).is_none());
    assert!(replacement_shape(&original, rect(), &[rules[1].clone(), rules[0].clone()], None).is_none());
}

#[test]
fn quantization_uncertainty_is_not_amplified_by_dense_replacements() {
    for alpha in [0, 1, 3, 4, 5, 255] {
        let source = Shadow { color: Color32::from_black_alpha(alpha), ..source() };
        let normal = FrameShadow::new(source, Elevation::Raised.layers(source.color));
        let dense = FrameShadow::new(source, [Shadow { color: Color32::BLACK, ..source }]);
        for opacity in [0.0, 0.005, 0.1, 0.5, 0.9, 1.0] {
            let original = faded_original(source, opacity);
            assert_eq!(
                replacement_shape(&original, rect(), std::slice::from_ref(&normal), None).is_some(),
                alpha != 0,
                "alpha={alpha}, opacity={opacity}"
            );
            let converted = replacement_shape(&original, rect(), std::slice::from_ref(&dense), None);
            assert_eq!(converted.is_some(), alpha == 255, "dense alpha={alpha}, opacity={opacity}");
            if let Some(Shape::Vec(shapes)) = converted {
                let alpha = match shapes.first() {
                    Some(Shape::Rect(shadow)) if shadow.blur_width > 0.0 => shadow.fill.a(),
                    _ => 0,
                };
                assert!(alpha.abs_diff(Color32::BLACK.gamma_multiply(opacity).a()) <= 1);
            }
        }
    }
}

#[test]
fn registered_colored_shadows_match_through_quantized_fades() {
    let source = Shadow { color: Color32::from_rgba_premultiplied(10, 40, 99, 100), ..source() };
    let rule = FrameShadow::new(source, Elevation::Raised.layers(source.color));
    for tick in 0..=1000 {
        let fade = tick as f32 / 1000.0;
        let opacity = Opacity::infer(source.color, source.color.gamma_multiply(fade)).unwrap();
        assert!(opacity.precise_for(&rule.layers));
        for layer in &rule.layers {
            assert!(
                layer
                    .color
                    .gamma_multiply(opacity.value)
                    .to_array()
                    .into_iter()
                    .zip(layer.color.gamma_multiply(fade).to_array())
                    .all(|(actual, expected)| actual.abs_diff(expected) <= 1)
            );
        }
        let original = faded_original(source, fade);
        let converted = replacement_shape(&original, rect(), std::slice::from_ref(&rule), None).unwrap();
        let (Shape::Vec(original), Shape::Vec(converted)) = (original, converted) else { panic!() };
        assert_eq!(converted.last(), original.last(), "the surface and border must remain untouched");
    }
    let mut tinted = faded_original(source, 0.5);
    let Shape::Vec(pair) = &mut tinted else { panic!() };
    let Shape::Rect(shadow) = &mut pair[0] else { panic!() };
    shadow.fill = Color32::from_rgba_premultiplied(20, shadow.fill.g(), shadow.fill.b(), shadow.fill.a());
    assert!(replacement_shape(&tinted, rect(), &[rule], None).is_none());
}

#[test]
fn adapter_tolerates_single_unit_alpha_quantization_jitter() {
    let mut candidate = original(rect());
    let original_alpha = {
        let Shape::Vec(pair) = &mut candidate else { panic!() };
        let Shape::Rect(shadow) = &mut pair[0] else { panic!() };
        let alpha = shadow.fill.a();
        shadow.fill = Color32::from_rgba_premultiplied(shadow.fill.r(), shadow.fill.g(), shadow.fill.b(), alpha.saturating_add(1));
        alpha
    };
    assert!(replacement_shape(&candidate, rect(), &[rule()], None).is_some(), "+1 alpha jitter must be tolerated");

    {
        let Shape::Vec(pair) = &mut candidate else { panic!() };
        let Shape::Rect(shadow) = &mut pair[0] else { panic!() };
        shadow.fill = Color32::from_rgba_premultiplied(shadow.fill.r(), shadow.fill.g(), shadow.fill.b(), original_alpha.saturating_sub(1));
    }
    assert!(replacement_shape(&candidate, rect(), &[rule()], None).is_some(), "-1 alpha jitter must be tolerated");

    {
        let Shape::Vec(pair) = &mut candidate else { panic!() };
        let Shape::Rect(shadow) = &mut pair[0] else { panic!() };
        shadow.fill = Color32::from_rgba_premultiplied(shadow.fill.r(), shadow.fill.g(), shadow.fill.b(), original_alpha.saturating_add(2));
    }
    assert!(replacement_shape(&candidate, rect(), &[rule()], None).is_none(), "two-unit alpha overshoot must be rejected as distinct shadow");
}

fn render(ctx: &Context, mut content: impl FnMut(&mut Ui)) -> Vec<egui::epaint::RectShape> {
    let mut output = egui::FullOutput::default();
    for frame in 0..4 {
        output = ctx.run_ui(
            RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(640.0, 480.0))), time: Some(frame as f64), ..Default::default() },
            |ui| content(ui),
        );
        output.textures_delta.clear();
    }
    let mut rectangles = Vec::new();
    for entry in &output.shapes {
        rects(&entry.shape, &mut rectangles);
    }
    rectangles
}

#[test]
#[expect(clippy::disallowed_types, reason = "test egui's stock ComboBox popup adapter")]
fn real_window_popup_and_modal_use_the_adapter_without_touching_the_scrim() {
    for kind in ["window", "popup", "modal", "combo"] {
        let ctx = Context::default();
        ctx.all_styles_mut(|s| {
            s.animation_time = 0.0;
            s.visuals.window_shadow = source();
            s.visuals.popup_shadow = source();
        });
        install(&ctx, vec![rule()]);
        let scrim = Color32::from_black_alpha(63);
        let rectangles = render(&ctx, |ui| match kind {
            "window" => {
                egui::Window::new("test").fixed_pos(rect().min).show(ui.ctx(), |ui| ui.label("content"));
            }
            "modal" => {
                egui::Modal::new(egui::Id::unique("modal")).backdrop_color(scrim).show(ui.ctx(), |ui| ui.label("content"));
            }
            "combo" => {
                let combo = egui::ComboBox::from_id_salt("combo").show_ui(ui, |ui| ui.label("content"));
                egui::Popup::open_id(ui.ctx(), combo.response.id.with("popup"));
            }
            _ => {
                egui::Popup::new(egui::Id::unique("popup"), ui.ctx().clone(), egui::PopupAnchor::Position(rect().min), ui.layer_id())
                    .open(true)
                    .show(|ui| ui.label("content"));
            }
        });
        let shadows: Vec<_> = rectangles.iter().filter(|r| r.blur_width > 0.0).collect();
        assert_eq!(shadows.len(), 4, "{kind}: {shadows:?}");
        if kind == "modal" {
            assert!(rectangles.iter().any(|r| r.fill == scrim && r.blur_width == 0.0 && r.corner_radius == CornerRadius::ZERO));
        }
    }
}

#[test]
fn owned_frame_preserves_layout_and_allows_no_shadow() {
    let ctx = Context::default();
    let mut recorded_response_rect = Rect::NOTHING;
    let mut faded_response_rect = Rect::NOTHING;
    let layers = Elevation::Raised.layers(source().color);
    let rectangles = render(&ctx, |ui| {
        let frame = Frame::NONE.fill(Color32::GREEN).inner_margin(7).outer_margin(3).stroke(Stroke::new(2.0, Color32::RED)).shadow(source());
        let expected_size = frame.show(ui, |ui| ui.allocate_exact_size(vec2(60.0, 20.0), egui::Sense::hover())).response.rect.size();
        let response = show_frame(ui, frame, &[], |ui| ui.allocate_exact_size(vec2(60.0, 20.0), egui::Sense::hover()));
        assert_eq!(response.response.rect.size(), expected_size);
        recorded_response_rect = response.response.rect;

        ui.multiply_opacity(0.5);
        let response = show_frame(ui, frame, &layers, |ui| ui.allocate_exact_size(vec2(60.0, 20.0), egui::Sense::hover()));
        assert_eq!(response.response.rect.size(), expected_size);
        faded_response_rect = response.response.rect;
    });
    let shadows: Vec<_> = rectangles.iter().filter(|r| r.blur_width > 0.0).collect();
    assert_eq!(shadows.len(), 5);
    assert_eq!(shadows[0].fill, source().color);
    for (shadow, layer) in shadows[1..].iter().zip(layers) {
        assert_eq!(shadow.fill, layer.color.gamma_multiply(0.5));
    }
    let frames: Vec<_> = rectangles.iter().filter(|r| r.fill.g() > 0 && r.stroke_kind == StrokeKind::Inside).collect();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[1].rect, recorded_response_rect.shrink(3.0));
    assert_eq!(frames[2].rect, faded_response_rect.shrink(3.0));
    assert_eq!(frames[2].fill, Color32::GREEN.gamma_multiply(0.5));
    assert_eq!(frames[2].stroke.color, Color32::RED.gamma_multiply(0.5));
    assert_eq!(frames[2].stroke.width, 2.0);
}

#[test]
fn source_bound_layer_override_preserves_fade_and_expires() {
    let ctx = Context::default();
    let id = egui::Id::unique("source-bound-override");
    let layer = LayerId::new(egui::Order::Foreground, id);
    let rules = [40, 100].map(|alpha| {
        let source = Shadow { color: Color32::from_black_alpha(alpha), ..source() };
        FrameShadow::new(source, Elevation::Raised.layers(source.color))
    });
    let replacement = [Shadow { color: Color32::from_black_alpha(90), ..source() }];
    assert!(!set_layer_shadows_for_source(&ctx, layer, source(), replacement));
    install(&ctx, rules.into());
    for bound in [false, true] {
        let rectangles = render(&ctx, |ui| {
            egui::Area::new(id).order(layer.order).fade_in(false).show(ui.ctx(), |ui| {
                ui.multiply_opacity(0.4);
                Frame::NONE.fill(Color32::WHITE).shadow(source()).show(ui, |ui| ui.set_min_size(vec2(120.0, 80.0)));
            });
            if bound {
                assert!(set_layer_shadows_for_source(ui.ctx(), layer, source(), replacement));
            } else {
                assert!(set_layer_shadows(ui.ctx(), layer, replacement));
            }
        });
        let shadows: Vec<_> = rectangles.iter().filter(|r| r.blur_width > 0.0).collect();
        assert_eq!(shadows.len(), 1);
        assert_eq!(shadows[0].fill.a(), if bound { 36 } else { 40 });
        assert_eq!(frame_status(&ctx, layer), Some(if bound { FrameStatus::Applied } else { FrameStatus::Unchanged }));
    }
    let rectangles = render(&ctx, |ui| {
        egui::Area::new(id).order(layer.order).fade_in(false).show(ui.ctx(), |ui| {
            ui.multiply_opacity(0.4);
            Frame::NONE.fill(Color32::WHITE).shadow(source()).show(ui, |ui| ui.set_min_size(vec2(120.0, 80.0)));
        });
    });
    assert_eq!(rectangles.iter().filter(|r| r.blur_width > 0.0).count(), 4);
    assert_eq!(frame_status(&ctx, layer), Some(FrameStatus::Applied));
}

#[test]
fn empty_rules_preserve_original_frames_even_with_an_override() {
    let ctx = Context::default();
    ctx.all_styles_mut(|style| {
        style.animation_time = 0.0;
        style.visuals.window_shadow = source();
    });
    install(&ctx, vec![]);
    let mut layer = LayerId::background();
    let rectangles = render(&ctx, |ui| {
        let response = egui::Window::new("empty rules").show(ui.ctx(), |ui| ui.label("content")).unwrap();
        layer = response.response.layer_id;
        assert!(set_layer_shadows_for_source(ui.ctx(), layer, source(), vec![]));
    });
    let shadows: Vec<_> = rectangles.iter().filter(|r| r.blur_width > 0.0).collect();
    assert_eq!(shadows.len(), 1);
    assert_eq!(shadows[0].fill, source().color);
    assert_eq!(frame_status(&ctx, layer), Some(FrameStatus::Unchanged));
}

#[test]
fn nested_popups_are_independent_and_layer_overrides_expire() {
    let ctx = Context::default();
    ctx.all_styles_mut(|s| {
        s.animation_time = 0.0;
        s.visuals.popup_shadow = source();
    });
    install(&ctx, vec![rule()]);
    let first = egui::Id::unique("first");
    let second = egui::Id::unique("second");
    let nested = |ui: &mut Ui, hide_first| {
        let first = egui::Popup::new(first, ui.ctx().clone(), egui::PopupAnchor::Position(pos2(30.0, 30.0)), ui.layer_id())
            .open(true)
            .show(|ui| {
                ui.label("parent");
                egui::Popup::new(second, ui.ctx().clone(), egui::PopupAnchor::Position(pos2(250.0, 30.0)), ui.layer_id())
                    .open(true)
                    .show(|ui| ui.label("child"));
            })
            .unwrap();
        if hide_first {
            assert!(set_layer_shadows(ui.ctx(), first.response.layer_id, vec![]));
        }
    };
    let rectangles = render(&ctx, |ui| nested(ui, true));
    assert_eq!(rectangles.iter().filter(|r| r.blur_width > 0.0).count(), 4, "remove only the parent's shadow");
    let rectangles = render(&ctx, |ui| nested(ui, false));
    assert_eq!(rectangles.iter().filter(|r| r.blur_width > 0.0).count(), 8, "the previous pass's override must expire");
    install(&ctx, vec![FrameShadow::new(source(), Vec::new())]);
    let rectangles = render(&ctx, |ui| nested(ui, false));
    assert_eq!(rectangles.iter().filter(|r| r.blur_width > 0.0).count(), 0, "reinstalling replaces the theme recipes");
}

#[test]
fn unsupported_or_ambiguous_area_frames_are_unchanged() {
    let ctx = Context::default();
    install(&ctx, vec![rule()]);
    let id = egui::Id::unique("ambiguous");
    let layer = LayerId::new(egui::Order::Foreground, id);
    let area = rect();
    let backdrop = Shape::rect_filled(area.expand(100.0), 0, Color32::from_black_alpha(50));
    let originals = [backdrop, Shape::Noop, original(area), original(area)];
    let expected = originals.clone();
    // Reproduce two competing frame candidates in an actual Area paint list.
    ctx.on_end_pass(
        "ambiguity acceptance",
        std::sync::Arc::new(move |ctx| {
            ctx.graphics(|g| {
                let entries: Vec<_> = g.get(layer).unwrap().all_entries().map(|s| s.shape.clone()).collect();
                assert_eq!(entries, expected);
            });
        }),
    );
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let area = egui::Area::new(id).fixed_pos(area.min).order(layer.order).fade_in(false).show(ui.ctx(), |ui| ui.set_min_size(area.size()));
        ctx.graphics_mut(|g| {
            let list = g.entry(layer);
            for shape in &originals {
                list.add(area.response.rect.expand(100.0), shape.clone());
            }
        });
    });
    output.textures_delta.clear();
    assert_eq!(frame_status(&ctx, layer), Some(FrameStatus::Unchanged));
}

#[test]
fn explicit_owned_frame_is_not_reprocessed_by_the_container_adapter() {
    let ctx = Context::default();
    install(&ctx, vec![rule()]);
    let rectangles = render(&ctx, |ui| {
        let area = egui::Area::new(egui::Id::unique("explicit-single")).order(egui::Order::Foreground).fade_in(false).show(ui.ctx(), |ui| {
            show_frame(ui, Frame::NONE.fill(Color32::WHITE), &[source()], |ui| {
                ui.allocate_exact_size(vec2(120.0, 80.0), egui::Sense::hover());
            });
        });
        set_layer_shadows(ui.ctx(), area.response.layer_id, vec![]);
    });
    assert_eq!(rectangles.iter().filter(|r| r.blur_width > 0.0).count(), 1, "explicit painting takes precedence over automatic rules and overrides");
}

#[test]
fn viewport_pass_state_is_isolated_and_closed_viewports_are_released() {
    let ctx = Context::default();
    install(&ctx, vec![rule()]);
    let root = egui::ViewportId::ROOT;
    let child = egui::ViewportId::from_hash_of("shadow-child");
    let layer = LayerId::new(egui::Order::Foreground, egui::Id::unique("same-area-id"));
    for viewport_id in [root, child] {
        let mut input = RawInput { viewport_id, ..Default::default() };
        input.viewports.insert(child, egui::ViewportInfo { parent: Some(root), ..Default::default() });
        let mut output = ctx.run_ui(input, |ui| {
            assert!(set_layer_shadows(ui.ctx(), layer, Elevation::Modal.layers(source().color)));
        });
        output.textures_delta.clear();
    }
    ctx.with_plugin(|plugin: &mut EguiFrames| {
        assert_eq!(plugin.passes.get(&root).unwrap().overrides.len(), 1);
        assert_eq!(plugin.passes.get(&child).unwrap().overrides.len(), 1);
    });
    let mut output = ctx.run_ui(Default::default(), |_| {});
    output.textures_delta.clear();
    ctx.with_plugin(|plugin: &mut EguiFrames| {
        assert!(plugin.passes.get(&root).unwrap().overrides.is_empty());
        assert!(!plugin.passes.contains_key(&child));
    });
}

/// A CPU-only comparison, deliberately outside the ordinary unit-test gate. No PNGs or GPU
/// rendering. Run with: cargo test -p craft-ui --lib profile_shadow_adapter -- --ignored --nocapture
#[test]
#[ignore = "explicit CPU profiling; timings depend on the machine and build profile"]
fn profile_shadow_adapter() {
    fn sample(adapt: bool) -> (std::time::Duration, usize) {
        let ctx = Context::default();
        ctx.all_styles_mut(|s| {
            s.animation_time = 0.0;
            s.visuals.popup_shadow = source();
        });
        if adapt {
            install(&ctx, vec![rule()]);
        }
        let mut times = Vec::new();
        let mut vertices = 0;
        for frame in 0..108 {
            let start = std::time::Instant::now();
            let mut output = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1280.0, 1080.0))),
                    time: Some(frame as f64 / 60.0),
                    ..Default::default()
                },
                |ui| {
                    for panel in 0..16 {
                        egui::Area::new(egui::Id::unique(("profile", panel)))
                            .order(egui::Order::Middle)
                            .fixed_pos(pos2(20.0 + (panel % 4) as f32 * 260.0, 20.0 + (panel / 4) as f32 * 240.0))
                            .fade_in(false)
                            .show(ui.ctx(), |ui| {
                                Frame::popup(ui.style()).show(ui, |ui| {
                                    ui.set_min_width(200.0);
                                    for _ in 0..8 {
                                        ui.label("A representative menu or panel row");
                                    }
                                });
                            });
                    }
                },
            );
            output.textures_delta.clear();
            let meshes = ctx.tessellate(output.shapes, output.pixels_per_point);
            let elapsed = start.elapsed();
            vertices = meshes
                .iter()
                .map(|mesh| match &mesh.primitive {
                    egui::epaint::Primitive::Mesh(mesh) => mesh.vertices.len(),
                    _ => 0,
                })
                .sum();
            if frame >= 8 {
                times.push(elapsed);
            }
        }
        if adapt {
            for panel in 0..16 {
                let layer = LayerId::new(egui::Order::Middle, egui::Id::unique(("profile", panel)));
                assert_eq!(frame_status(&ctx, layer), Some(FrameStatus::Applied));
            }
        }
        times.sort_unstable();
        (times[times.len() / 2], vertices)
    }
    // Alternate order to make one-time cache/order effects visible in the output.
    for round in 0..2 {
        let (baseline, adapted) = if round == 0 {
            (sample(false), sample(true))
        } else {
            let adapted = sample(true);
            (sample(false), adapted)
        };
        assert!(adapted.1 > baseline.1);
        println!(
            "16 Areas x 8 text rows, 100 warmed samples, round {}: single {:?} / {} vertices; layered {:?} / {} vertices; delta {:+.1}%",
            round + 1,
            baseline.0,
            baseline.1,
            adapted.0,
            adapted.1,
            (adapted.0.as_secs_f64() / baseline.0.as_secs_f64() - 1.0) * 100.0
        );
    }
}
