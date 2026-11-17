//! Exact offscreen rendering comparisons against independent reference painters.
//!
//! These are paired same-renderer comparisons, not portable GPU golden files. The reference
//! paths deliberately do not call the shared components. Literal geometry and colors specify
//! the baseline; deliberate mutations prove color, spacing and stroke regressions are caught.
//! Set CRAFT_UI_VISUAL_DIR to retain bounded before/after/difference PNGs and comparison tables.

#[path = "visual/fixtures.rs"]
mod fixtures;

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard, PoisonError};

use egui::{Color32, Context, Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput, Rect, Response, TextureId, Ui};
use egui_kittest::{TestRenderer, wgpu::WgpuTestRenderer};
use fixtures::{CANVAS, Case, Component, Mutation, State, Theme};
use image::{Rgba, RgbaImage};

#[derive(Debug, PartialEq)]
struct Mesh {
    clip: Rect,
    texture: TextureId,
    indices: Vec<u32>,
    vertices: Vec<(Pos2, Pos2, Color32)>,
}

struct Capture {
    image: RgbaImage,
    meshes: Option<Vec<Mesh>>,
}

/// Lets one test at a time hold a wgpu renderer. Each renderer creates its own Vulkan instance and
/// device. When one test uses its device while a test on another thread creates or destroys one,
/// the Vulkan loader now and then crashes the test binary with SIGSEGV.
static GPU_LOCK: Mutex<()> = Mutex::new(());

/// Take this before creating a renderer (`let _gpu = gpu_lock();`), so it is released after the
/// renderer is dropped. A test that failed while holding it does not block the others.
fn gpu_lock() -> MutexGuard<'static, ()> {
    GPU_LOCK.lock().unwrap_or_else(PoisonError::into_inner)
}

fn context(theme: Theme, scale: f32) -> Context {
    let ctx = Context::default();
    ctx.set_theme(theme.egui());
    ctx.set_pixels_per_point(scale);
    ctx.set_fonts(egui::FontDefinitions::empty());
    ctx.all_styles_mut(|style| {
        style.animation_time = 0.0;
        style.visuals.text_cursor.blink = false;
        style.visuals.disabled_alpha = 0.4;
    });
    ctx
}

fn raw_input(frame: usize, events: Vec<Event>) -> RawInput {
    RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, CANVAS)),
        time: Some(frame as f64 / 60.0),
        events,
        max_texture_side: Some(2048),
        ..Default::default()
    }
}

fn background(ui: &mut Ui, theme: Theme) {
    let screen = Rect::from_min_size(Pos2::ZERO, CANVAS);
    ui.set_clip_rect(screen);
    ui.painter().rect_filled(screen, 0.0, theme.surface());
}

fn finish_capture(renderer: &mut WgpuTestRenderer, ctx: &Context, output: &FullOutput) -> Capture {
    let meshes = ctx
        .tessellate(output.shapes.clone(), output.pixels_per_point)
        .into_iter()
        .map(|clipped| {
            let egui::epaint::Primitive::Mesh(mesh) = clipped.primitive else { panic!("visual fixtures must not contain paint callbacks") };
            Mesh {
                clip: clipped.clip_rect,
                texture: mesh.texture_id,
                indices: mesh.indices,
                vertices: mesh.vertices.into_iter().map(|vertex| (vertex.pos, vertex.uv, vertex.color)).collect(),
            }
        })
        .collect();
    let image = renderer.render(ctx, output).expect("offscreen renderer is required for the visual suite");
    assert!(image.width() > 0 && image.height() > 0 && image.width() <= 256 && image.height() <= 144);
    Capture { image, meshes: Some(meshes) }
}

fn capture(renderer: &mut WgpuTestRenderer, case: Case, reference: bool, mutation: Mutation) -> Capture {
    let ctx = context(case.theme, case.scale);
    let mut output = FullOutput::default();
    let mut responses: Vec<Response> = Vec::new();
    for frame in 0..4 {
        let mut events = Vec::new();
        if !reference && frame == 2 {
            if case.state == State::Focus {
                ctx.memory_mut(|memory| memory.request_focus(responses[0].id));
            }
            if case.state.pointer() {
                let center = case.control_rect().center();
                events.push(Event::PointerMoved(center));
                if case.state.pressed() {
                    events.push(Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                }
            }
        }
        output = ctx.run_ui(raw_input(frame, events), |ui| {
            background(ui, case.theme);
            if reference {
                fixtures::reference(ui, case);
            } else {
                responses = fixtures::actual(ui, case, mutation);
            }
        });
        renderer.handle_delta(&mut output.textures_delta);
    }
    assert_eq!(output.pixels_per_point, case.scale);
    if !reference {
        let response = &responses[0];
        assert_eq!(response.enabled(), case.state.enabled(), "{case:?}");
        if case.state == State::Hover {
            assert!(response.hovered(), "hover was not exercised: {case:?}");
        }
        if case.state.pressed() {
            assert!(response.is_pointer_button_down_on(), "press was not exercised: {case:?}");
            if case.component.is_split() {
                assert!(response.dragged(), "divider drag was not exercised: {case:?}");
            }
        }
        if case.state == State::Focus {
            assert!(response.has_focus(), "focus was not exercised: {case:?}");
        }
        if case.state == State::Disabled {
            assert!(!response.hovered() && !response.has_focus(), "disabled pointer input must remain inert: {case:?}");
        }
    }
    finish_capture(renderer, &ctx, &output)
}

fn static_capture(renderer: &mut WgpuTestRenderer, theme: Theme, scale: f32, mut paint: impl FnMut(&mut Ui)) -> Capture {
    let ctx = context(theme, scale);
    let mut output = FullOutput::default();
    for frame in 0..2 {
        output = ctx.run_ui(raw_input(frame, vec![]), |ui| {
            background(ui, theme);
            paint(ui);
        });
        renderer.handle_delta(&mut output.textures_delta);
    }
    assert_eq!(output.pixels_per_point, scale);
    finish_capture(renderer, &ctx, &output)
}

struct Artifacts {
    directory: Option<PathBuf>,
    rows: String,
}

impl Artifacts {
    fn new(suite: &str) -> Self {
        let directory = std::env::var_os("CRAFT_UI_VISUAL_DIR").map(|root| PathBuf::from(root).join(suite));
        if let Some(directory) = &directory {
            std::fs::create_dir_all(directory).unwrap();
        }
        Self { directory, rows: "case\twidth\theight\tchanged_pixels\tmesh_equal\texpected_change\n".into() }
    }

    fn record(&mut self, name: &str, before: &Capture, after: &Capture, expected_change: bool) -> usize {
        assert_eq!(before.image.dimensions(), after.image.dimensions(), "{name}");
        let mut changed = 0;
        let mut difference = RgbaImage::new(before.image.width(), before.image.height());
        for ((before, after), pixel) in before.image.pixels().zip(after.image.pixels()).zip(difference.pixels_mut()) {
            if before != after {
                changed += 1;
                *pixel = Rgba([255, 0, 128, 255]);
            }
        }
        let mesh_equal = match (&before.meshes, &after.meshes) {
            (Some(before), Some(after)) => (before == after).to_string(),
            (None, None) => "not_applicable".into(),
            _ => panic!("both captures must have the same comparison scope"),
        };
        self.rows.push_str(&format!("{name}\t{}\t{}\t{changed}\t{mesh_equal}\t{expected_change}\n", before.image.width(), before.image.height()));
        if let Some(directory) = &self.directory {
            before.image.save(directory.join(format!("{name}-before.png"))).unwrap();
            after.image.save(directory.join(format!("{name}-after.png"))).unwrap();
            difference.save(directory.join(format!("{name}-difference.png"))).unwrap();
            // Preserve completed rows even if a later assertion fails.
            std::fs::write(directory.join("comparison.tsv"), &self.rows).unwrap();
        }
        changed
    }
}

#[test]
fn rendered_controls_match_independent_reference_pixels_and_meshes() {
    let _gpu = gpu_lock();
    let mut renderer = WgpuTestRenderer::new();
    let mut artifacts = Artifacts::new("controls");
    let mut count = 0;
    for component in Component::ALL {
        for state in State::ALL {
            if component.is_split() && state.selected() {
                continue; // A resize handle has no selected state in its public contract.
            }
            for theme in [Theme::Light, Theme::Dark] {
                for scale in [1.0, 1.5, 2.0] {
                    for compact in [false, true] {
                        let case = Case { component, state, theme, scale, compact };
                        let before = capture(&mut renderer, case, true, Mutation::None);
                        let after = capture(&mut renderer, case, false, Mutation::None);
                        let changed = artifacts.record(&case.name(), &before, &after, false);
                        assert_eq!(before.meshes, after.meshes, "reference mesh mismatch: {case:?}");
                        assert_eq!(changed, 0, "reference pixels changed: {case:?}");
                        count += 1;
                    }
                }
            }
        }
    }
    assert_eq!(count, 372);
    eprintln!("{count} control before/after pairs matched exact RGBA pixels and tessellated meshes");
}

#[test]
fn visual_comparisons_detect_color_spacing_and_stroke_mutations() {
    let _gpu = gpu_lock();
    let mut renderer = WgpuTestRenderer::new();
    let mut artifacts = Artifacts::new("negative-controls");
    let mut count = 0;
    for theme in [Theme::Light, Theme::Dark] {
        for scale in [1.0, 1.5, 2.0] {
            for compact in [false, true] {
                let case = Case { component: Component::Button, state: State::Normal, theme, scale, compact };
                let before = capture(&mut renderer, case, true, Mutation::None);
                for mutation in [Mutation::Color, Mutation::Spacing, Mutation::Stroke] {
                    let after = capture(&mut renderer, case, false, mutation);
                    let changed = artifacts.record(&format!("{}-{mutation:?}", case.name()), &before, &after, true);
                    assert_ne!(before.meshes, after.meshes, "mesh comparison missed {mutation:?}: {case:?}");
                    assert!(changed > 0, "pixel comparison missed {mutation:?}: {case:?}");
                    count += 1;
                }
            }
        }
    }
    assert_eq!(count, 36);
    eprintln!("{count} deliberate color/spacing/stroke changes were detected in both pixels and meshes");
}

#[test]
fn toolbar_fitting_matches_literal_group_and_overflow_positions() {
    use craft_ui::buttons::IconButton;
    use craft_ui::layout::fit_toolbar;
    use egui::{Align, Layout, Stroke, StrokeKind, UiBuilder, pos2, vec2};

    let _gpu = gpu_lock();
    let mut renderer = WgpuTestRenderer::new();
    let mut artifacts = Artifacts::new("toolbar");
    let mut count = 0;
    // Independently specified rectangles: three natural groups, one group + More, More only.
    for (available, expected) in
        [(112.0, vec![(12.0, 24.0), (40.0, 28.0), (72.0, 36.0)]), (64.0, vec![(12.0, 24.0), (40.0, 20.0)]), (20.0, vec![(12.0, 20.0)])]
    {
        for theme in [Theme::Light, Theme::Dark] {
            for scale in [1.0, 1.5, 2.0] {
                let p = theme.palette();
                let before = static_capture(&mut renderer, theme, scale, |ui| {
                    for &(x, width) in &expected {
                        let rect = Rect::from_min_size(pos2(x, 12.0), vec2(width, 24.0));
                        ui.painter().rect_filled(rect, 3.0, p.normal);
                        ui.painter().rect_stroke(rect, 3.0, Stroke::new(1.0, p.border), StrokeKind::Inside);
                        fixtures::icon(ui.painter(), rect, p.icon, true);
                    }
                });
                let after = static_capture(&mut renderer, theme, scale, |ui| {
                    let fit = fit_toolbar(&[24.0, 28.0, 36.0], available, 4.0, 20.0).unwrap();
                    let style = fixtures::button_style(theme);
                    ui.scope_builder(
                        UiBuilder::new()
                            .max_rect(Rect::from_min_size(pos2(12.0, 12.0), vec2(available, 24.0)))
                            .layout(Layout::left_to_right(Align::Min)),
                        |ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for &(_, width) in &fit.shown {
                                IconButton::new("Group", vec2(width, 24.0), &style)
                                    .show(ui, |ui, rect, tint| fixtures::icon(ui.painter(), rect, tint, true));
                            }
                            if !fit.overflow.is_empty() {
                                IconButton::new("More", vec2(fit.overflow_width, 24.0), &style)
                                    .show(ui, |ui, rect, tint| fixtures::icon(ui.painter(), rect, tint, true));
                            }
                        },
                    );
                });
                let name = format!("toolbar-{available}-{}-{scale}x", theme.name());
                let changed = artifacts.record(&name, &before, &after, false);
                assert_eq!(before.meshes, after.meshes, "{name}");
                assert_eq!(changed, 0, "{name}");
                count += 1;
            }
        }
    }
    assert_eq!(count, 18);
}

#[test]
fn widget_palette_renders_all_five_states_with_existing_borders_and_expansion() {
    use craft_ui::theme::WidgetPalette;
    use egui::{CornerRadius, Stroke, StrokeKind, pos2, vec2};

    let _gpu = gpu_lock();
    let mut renderer = WgpuTestRenderer::new();
    let mut artifacts = Artifacts::new("widget-palette");
    for theme in [Theme::Light, Theme::Dark] {
        for scale in [1.0, 1.5, 2.0] {
            for compact in [false, true] {
                let p = theme.palette();
                let fills = [p.normal, p.hovered, p.pressed, p.selected, theme.surface()];
                let rect = |index| {
                    Rect::from_min_size(
                        pos2(12.0 + (index % 3) as f32 * 34.0, 12.0 + (index / 3) as f32 * 26.0),
                        if compact { vec2(20.0, 12.0) } else { vec2(26.0, 18.0) },
                    )
                };
                let borders = [Color32::RED, Color32::GREEN, Color32::BLUE, p.border, p.focus];
                let before = static_capture(&mut renderer, theme, scale, |ui| {
                    for index in 0..5 {
                        let rect = rect(index).expand(if index % 2 == 0 { 0.0 } else { 0.5 });
                        ui.painter().rect_filled(rect, 2.0, fills[index]);
                        // A separate inset uses the weak fill, so omission of either fill is visible.
                        ui.painter().rect_filled(rect.shrink(3.0), 0.0, fills[index]);
                        ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, borders[index]), StrokeKind::Inside);
                        ui.painter()
                            .line_segment([rect.left_center() + vec2(4.0, 0.0), rect.right_center() - vec2(4.0, 0.0)], Stroke::new(1.5, p.icon));
                    }
                });
                let after = static_capture(&mut renderer, theme, scale, |ui| {
                    let mut widgets = ui.visuals().widgets.clone();
                    for (index, widget) in
                        [&mut widgets.noninteractive, &mut widgets.inactive, &mut widgets.hovered, &mut widgets.active, &mut widgets.open]
                            .into_iter()
                            .enumerate()
                    {
                        widget.bg_stroke = Stroke::new(1.0, borders[index]);
                        widget.expansion = if index % 2 == 0 { 0.0 } else { 0.5 };
                    }
                    WidgetPalette { noninteractive: p.normal, inactive: p.hovered, hovered: p.pressed, active: p.selected, open: theme.surface() }
                        .apply_to(&mut widgets, CornerRadius::same(2), Stroke::new(1.5, p.icon));
                    for (index, widget) in
                        [widgets.noninteractive, widgets.inactive, widgets.hovered, widgets.active, widgets.open].iter().enumerate()
                    {
                        let rect = rect(index).expand(widget.expansion);
                        ui.painter().rect_filled(rect, widget.corner_radius, widget.bg_fill);
                        ui.painter().rect_filled(rect.shrink(3.0), 0.0, widget.weak_bg_fill);
                        ui.painter().rect_stroke(rect, widget.corner_radius, widget.bg_stroke, StrokeKind::Inside);
                        ui.painter().line_segment([rect.left_center() + vec2(4.0, 0.0), rect.right_center() - vec2(4.0, 0.0)], widget.fg_stroke);
                    }
                });
                let name = format!("palette-{}-{scale}x-{}", theme.name(), if compact { "compact" } else { "normal" });
                let changed = artifacts.record(&name, &before, &after, false);
                assert_eq!(before.meshes, after.meshes, "{name}");
                assert_eq!(changed, 0, "{name}");
            }
        }
    }
}

#[test]
fn svg_preparation_matches_literal_raster_references_at_all_scales() {
    use craft_ui::icons::{SvgIconSet, SvgStyle};
    use egui::{
        ImageSource,
        load::{Bytes, ImagePoll, SizeHint},
    };

    const SOURCE: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="4" y="4" width="16" height="16"/><path d="M4 4L20 20"/></svg>"#;
    const LIGHT: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#0f172a" stroke-width="1.75"><rect x="4" y="4" width="16" height="16"/><path d="M4 4L20 20"/></svg>"##;
    const DARK: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none" stroke="#e2e8f0" stroke-width="1.75"><rect x="4" y="4" width="16" height="16"/><path d="M4 4L20 20"/></svg>"##;
    fn raster(ctx: &Context, uri: &str, size: u32) -> Capture {
        let ImagePoll::Ready { image } = ctx.try_load_image(uri, SizeHint::Size { width: size, height: size, maintain_aspect_ratio: true }).unwrap()
        else {
            panic!("compiled-in SVG must rasterize synchronously")
        };
        assert_eq!(image.size, [size as usize; 2]);
        let image = RgbaImage::from_raw(size, size, image.pixels.iter().flat_map(|pixel| pixel.to_srgba_unmultiplied()).collect()).unwrap();
        assert!(image.pixels().any(|pixel| pixel[3] != 0), "blank SVG is not a valid matching baseline");
        Capture { image, meshes: None }
    }
    let mut artifacts = Artifacts::new("svg");
    for (theme, color, literal) in [(Theme::Light, "#0f172a", LIGHT), (Theme::Dark, "#e2e8f0", DARK)] {
        for (scale, numerator) in [(1.0, 2), (1.5, 3), (2.0, 4)] {
            for logical_size in [12, 24] {
                let ctx = Context::default();
                egui_extras::install_image_loaders(&ctx);
                ctx.include_bytes("bytes://literal-reference.svg", Bytes::Static(literal));
                let icons =
                    SvgIconSet::new(&[("box", SOURCE)], SvgStyle { current_color: color, stroke_width: Some("1.75") }, "bytes://actual/", "box");
                let ImageSource::Bytes { uri, bytes } = icons.source("box") else { panic!("catalog must return embedded bytes") };
                ctx.include_bytes(uri.clone(), bytes);
                let size = logical_size * numerator / 2;
                let before = raster(&ctx, "bytes://literal-reference.svg", size);
                let after = raster(&ctx, &uri, size);
                let name = format!("svg-{}-{scale}x-{logical_size}pt", theme.name());
                assert_eq!(artifacts.record(&name, &before, &after, false), 0, "{name}");
            }
        }
    }
}
