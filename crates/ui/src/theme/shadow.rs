//! Layered UI shadows in logical egui points. Applications own surface roles and colors.
//!
//! Prefer [`paint`] or [`show_frame`] for owned surfaces. [`install`] opts into a constrained
//! egui 0.36 frame adapter for built-in containers, including ComboBoxes and nested menus.

use egui::{Color32, Context, CornerRadius, Frame, InnerResponse, LayerId, Painter, Rect, Shadow, Shape, Ui, layers::ShapeIdx};
use std::collections::HashMap;

/// The PhotoCraft reference recipes. These are optional defaults, not stacking/input policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elevation {
    Low,
    Medium,
    High,
    Modal,
}

impl Elevation {
    /// Back-to-front layers. Preserve the supplied hue and scale its premultiplied opacity.
    pub fn layers(self, color: Color32) -> [Shadow; 4] {
        let parameters = match self {
            Self::Low => [(2, 10), (1, 6), (1, 3), (0, 2)],
            Self::Medium => [(5, 18), (3, 10), (2, 5), (1, 3)],
            Self::High => [(8, 28), (4, 14), (2, 7), (1, 4)],
            Self::Modal => [(12, 44), (6, 20), (3, 10), (1, 5)],
        };
        let weights = [0.30, 0.35, 0.25, 0.10];
        std::array::from_fn(|i| {
            let (y, blur) = parameters[i];
            Shadow { offset: [0, y], blur, spread: 0, color: color.gamma_multiply(weights[i]) }
        })
    }
}

/// A restrained, two-layer starting point for persistent floating toolbars.
pub fn floating_toolbar(color: Color32) -> [Shadow; 2] {
    [
        Shadow { offset: [0, 3], blur: 16, spread: 0, color: color.gamma_multiply(0.45) },
        Shadow { offset: [0, 1], blur: 4, spread: 0, color: color.gamma_multiply(0.30) },
    ]
}

fn valid(rect: Rect) -> bool {
    rect.is_finite() && rect.size().is_finite() && rect.is_positive()
}

/// Generate only shadow shapes, in back-to-front order. An empty recipe means no shadow.
/// `Shadow::blur` is egui's feathered edge width, not Gaussian sigma.
pub fn shapes(rect: Rect, radius: CornerRadius, layers: &[Shadow]) -> Vec<Shape> {
    if !valid(rect) {
        return Vec::new();
    }
    layers.iter().filter(|s| s.color != Color32::TRANSPARENT).map(|s| Shape::Rect(s.as_shape(rect, radius))).collect()
}

/// Paint behind a surface that the caller draws afterward. Retains painter clipping/opacity.
pub fn paint(painter: &Painter, rect: Rect, radius: CornerRadius, layers: &[Shadow]) {
    painter.extend(shapes(rect, radius, layers));
}

/// Bounds of the surface and its visible shadows; does not enlarge the painter's clip rect.
pub fn visual_bounds(rect: Rect, layers: &[Shadow]) -> Rect {
    if !valid(rect) {
        return Rect::NOTHING;
    }
    layers.iter().filter(|s| s.color != Color32::TRANSPARENT).fold(rect, |bounds, s| bounds.union(rect + s.margin()))
}

/// A frame with an owned shadow slot; preserves egui's layout, margins and content response.
/// The supplied layers replace `frame.shadow`, including when the layer slice is empty.
pub fn show_frame<R>(ui: &mut Ui, frame: Frame, layers: &[Shadow], content: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let slot = ui.painter().add(Shape::Noop);
    let viewport = ui.ctx().viewport_id();
    ui.ctx().with_plugin(|plugin: &mut EguiFrames| {
        plugin.passes.entry(viewport).or_default().owned.push((ui.layer_id(), slot));
    });
    let frame = frame.shadow(Shadow::NONE);
    let mut prepared = frame.begin(ui);
    let inner = content(&mut prepared.content_ui);
    let rect = frame.widget_rect(prepared.content_ui.min_rect());
    let mut background = shapes(rect, frame.corner_radius, layers);
    background.push(frame.paint(prepared.content_ui.min_rect()));
    ui.painter().set(slot, Shape::Vec(background));
    let response = prepared.allocate_space(ui);
    InnerResponse { inner, response }
}

/// An explicitly registered single-shadow signature and its replacement layers.
/// Unrecognized frames retain their original shadow. Do not register document effects here.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameShadow {
    pub source: Shadow,
    pub layers: Vec<Shadow>,
}

impl FrameShadow {
    pub fn new(source: Shadow, layers: impl Into<Vec<Shadow>>) -> Self {
        Self { source, layers: layers.into() }
    }
}

/// Configure the egui 0.36 adapter once per theme change. Reconfiguration replaces its rules.
///
/// At the end of each pass it recognizes only an exact two-rectangle egui Frame shape whose
/// caster matches its Area bounds and whose original shadow matches a registered signature.
/// It preserves the painted frame, clipping, layer transform and inherited fade opacity.
/// Other structures (including nested/arbitrary shape vectors) are left untouched.
/// This also covers popups created inside craft-ui without changing their interaction code.
pub fn install(ctx: &Context, rules: Vec<FrameShadow>) {
    ctx.plugin_or_default::<EguiFrames>().lock().rules = rules;
}

/// Override one known container's recipe for this pass, after its `show` call.
/// Returns false if the adapter is not installed. Unsupported frame structures still fall back
/// unchanged. An empty recipe removes a recognized shadow; it never removes the surface/scrim.
pub fn set_layer_shadows(ctx: &Context, layer: LayerId, layers: impl Into<Vec<Shadow>>) -> bool {
    let viewport = ctx.viewport_id();
    ctx.with_plugin(|plugin: &mut EguiFrames| {
        let pass = plugin.passes.entry(viewport).or_default();
        let layers = layers.into();
        if let Some((_, previous)) = pass.overrides.iter_mut().find(|(id, _)| *id == layer) {
            *previous = layers;
        } else {
            pass.overrides.push((layer, layers));
        }
    })
    .is_some()
}

/// Result for an inspected Area in the most recently completed pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameStatus {
    Applied,
    /// The original painting was retained, including unsupported or ambiguous structures.
    Unchanged,
}

/// Read after rendering a pass. `None` means the adapter did not inspect this Area (or is not
/// installed). This is a development diagnostic; no fallback warnings are shown in the UI.
pub fn frame_status(ctx: &Context, layer: LayerId) -> Option<FrameStatus> {
    let viewport = ctx.viewport_id();
    ctx.with_plugin(|plugin: &mut EguiFrames| {
        plugin.passes.get(&viewport).and_then(|pass| pass.results.iter().find(|(id, _)| *id == layer).map(|(_, status)| *status))
    })
    .flatten()
}

#[derive(Default)]
struct EguiFrames {
    rules: Vec<FrameShadow>,
    passes: HashMap<egui::ViewportId, FramePass>,
}

#[derive(Default)]
struct FramePass {
    overrides: Vec<(LayerId, Vec<Shadow>)>,
    results: Vec<(LayerId, FrameStatus)>,
    owned: Vec<(LayerId, ShapeIdx)>,
}

impl egui::plugin::Plugin for EguiFrames {
    fn debug_name(&self) -> &'static str {
        "craft-ui shadows (egui 0.36)"
    }

    fn on_begin_pass(&mut self, ui: &mut Ui) {
        // Immediate child viewports can render inside their parent's pass.
        // Clear only this viewport's state and release state for closed viewports.
        ui.ctx().input(|input| self.passes.retain(|id, _| input.raw.viewports.contains_key(id)));
        let pass = self.passes.entry(ui.ctx().viewport_id()).or_default();
        pass.overrides.clear();
        pass.results.clear();
        pass.owned.clear();
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx();
        let pass = self.passes.entry(ctx.viewport_id()).or_default();
        let areas: Vec<_> = ctx.memory(|m| m.layer_ids().filter_map(|layer| m.area_rect(layer.id).map(|rect| (layer, rect))).collect());
        ctx.graphics_mut(|graphics| {
            for (layer, rect) in areas {
                if !valid(rect) || layer.order == egui::Order::Background {
                    continue;
                }
                let Some(list) = graphics.get_mut(layer) else { continue };
                let replacement = pass.overrides.iter().find(|(id, _)| *id == layer).map(|(_, layers)| layers.as_slice());
                let candidates: Vec<_> = list
                    .all_entries()
                    .enumerate()
                    .filter(|(i, _)| !pass.owned.contains(&(layer, ShapeIdx(*i))))
                    .filter_map(|(i, entry)| replacement_shape(&entry.shape, rect, &self.rules, replacement).map(|shape| (ShapeIdx(i), shape)))
                    .collect();
                // Ambiguous structure is unsupported: do not guess which entry owns the frame.
                if let [(index, shape)] = candidates.as_slice() {
                    list.mutate_shape(*index, |entry| entry.shape = shape.clone());
                    pass.results.push((layer, FrameStatus::Applied));
                } else {
                    pass.results.push((layer, FrameStatus::Unchanged));
                }
            }
        });
    }
}

fn replacement_shape(shape: &Shape, area: Rect, rules: &[FrameShadow], override_layers: Option<&[Shadow]>) -> Option<Shape> {
    let Shape::Vec(pair) = shape else { return None };
    let [Shape::Rect(shadow), Shape::Rect(frame)] = pair.as_slice() else { return None };
    if !valid(frame.rect) || frame.rect != area || frame.angle != 0.0 || frame.blur_width != 0.0 {
        return None;
    }
    for rule in rules {
        let source_alpha = f32::from(rule.source.color.a());
        if source_alpha == 0.0 {
            continue;
        }
        let opacity = f32::from(shadow.fill.a()) / source_alpha;
        if opacity > 1.0 {
            continue;
        }
        let mut expected = rule.source.as_shape(frame.rect, frame.corner_radius);
        expected.fill = rule.source.color.gamma_multiply(opacity);
        if *shadow != expected {
            continue;
        }
        let layers: Vec<_> =
            override_layers.unwrap_or(&rule.layers).iter().map(|s| Shadow { color: s.color.gamma_multiply(opacity), ..*s }).collect();
        let mut result = shapes(frame.rect, frame.corner_radius, &layers);
        result.push(Shape::Rect(frame.clone()));
        return Some(Shape::Vec(result));
    }
    None
}

#[cfg(test)]
mod tests;
