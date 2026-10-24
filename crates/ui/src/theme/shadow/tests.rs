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
fn renderer_retains_color_radius_clip_and_painter_opacity_once() {
    let ctx = Context::default();
    let clip = rect().expand(50.0);
    let mut painter = ctx.layer_painter(LayerId::new(egui::Order::Foreground, egui::Id::new("owned"))).with_clip_rect(clip);
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
    let mut original = original(rect());
    let Shape::Vec(pair) = &mut original else { panic!() };
    for shape in pair {
        if let Shape::Rect(rect) = shape {
            rect.fill = rect.fill.gamma_multiply(0.5);
            rect.stroke.color = rect.stroke.color.gamma_multiply(0.5);
        }
    }
    let converted = replacement_shape(&original, rect(), &[rule()], None).unwrap();
    let Shape::Vec(layers) = converted else { panic!() };
    let Shape::Rect(shadow) = layers.first().unwrap() else { panic!() };
    assert_eq!(shadow.fill, rule().layers[0].color.gamma_multiply(0.5));
    assert!(replacement_shape(&Shape::Vec(layers), rect(), &[rule()], None).is_none(), "must not repeatedly append shadows");
    let removed = replacement_shape(&original, rect(), &[rule()], Some(&[])).unwrap();
    let Shape::Vec(remaining) = removed else { panic!() };
    assert_eq!(remaining.len(), 1);
    let Shape::Vec(original) = original else { panic!() };
    assert_eq!(remaining.last(), original.last());
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
                egui::Modal::new(egui::Id::new("modal")).backdrop_color(scrim).show(ui.ctx(), |ui| ui.label("content"));
            }
            "combo" => {
                let combo = egui::ComboBox::from_id_salt("combo").show_ui(ui, |ui| ui.label("content"));
                egui::Popup::open_id(ui.ctx(), combo.response.id.with("popup"));
            }
            _ => {
                egui::Popup::new(egui::Id::new("popup"), ui.ctx().clone(), egui::PopupAnchor::Position(rect().min), ui.layer_id())
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
    let rectangles = render(&ctx, |ui| {
        let frame = Frame::NONE.fill(Color32::GREEN).inner_margin(7).outer_margin(3).stroke(Stroke::new(2.0, Color32::RED)).shadow(source());
        let expected = frame.show(ui, |ui| ui.allocate_exact_size(vec2(60.0, 20.0), egui::Sense::hover())).response.rect.size();
        let response = show_frame(ui, frame, &[], |ui| ui.allocate_exact_size(vec2(60.0, 20.0), egui::Sense::hover()));
        assert_eq!(response.response.rect.size(), expected);
    });
    assert_eq!(rectangles.iter().filter(|r| r.blur_width > 0.0).count(), 1);
    assert_eq!(rectangles.iter().filter(|r| r.fill == Color32::GREEN && r.stroke_kind == StrokeKind::Inside).count(), 2);
}

#[test]
fn nested_popups_are_independent_and_layer_overrides_expire() {
    let ctx = Context::default();
    ctx.all_styles_mut(|s| {
        s.animation_time = 0.0;
        s.visuals.popup_shadow = source();
    });
    install(&ctx, vec![rule()]);
    let first = egui::Id::new("first");
    let second = egui::Id::new("second");
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
    let id = egui::Id::new("ambiguous");
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
        let area = egui::Area::new(egui::Id::new("explicit-single")).order(egui::Order::Foreground).fade_in(false).show(ui.ctx(), |ui| {
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
    let layer = LayerId::new(egui::Order::Foreground, egui::Id::new("same-area-id"));
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
                        egui::Area::new(egui::Id::new(("profile", panel)))
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
                let layer = LayerId::new(egui::Order::Middle, egui::Id::new(("profile", panel)));
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
