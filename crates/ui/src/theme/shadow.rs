//! Layered UI shadows in logical egui points. Applications own surface roles and colors.
//!
//! Prefer [`paint`] or [`show_frame`] for owned surfaces. [`install`] opts into a constrained
//! egui 0.37 frame adapter for built-in containers, including ComboBoxes and nested menus.

use egui::{Color32, Context, CornerRadius, Frame, InnerResponse, LayerId, Painter, Rect, Shadow, Shape, Ui, layers::ShapeIdx};
use std::collections::HashMap;

/// One layer in an elevation recipe, using egui's logical-point geometry and color.
pub type ShadowLayer = Shadow;

/// A fixed-size, allocation-free elevation recipe, ordered back to front.
/// Unused layers are [`Shadow::NONE`]; the painting helpers omit them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ElevationShadow {
    layers: [ShadowLayer; 4],
}

impl ElevationShadow {
    pub const NONE: Self = Self::new([Shadow::NONE; 4]);

    pub const fn new(layers: [ShadowLayer; 4]) -> Self {
        Self { layers }
    }

    pub const fn as_slice(&self) -> &[ShadowLayer] {
        &self.layers
    }
}

impl AsRef<[ShadowLayer]> for ElevationShadow {
    fn as_ref(&self) -> &[ShadowLayer] {
        self.as_slice()
    }
}

/// Semantic shadow tokens. Applications still own surface roles, colors and stacking policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Elevation {
    Flat,
    Subtle,
    Raised,
    Overlay,
    Floating,
    Modal,
    /// Compatibility name for [`Self::Subtle`].
    Low,
    /// Compatibility name for [`Self::Raised`].
    Medium,
    /// Compatibility name for [`Self::Overlay`].
    High,
}

impl Elevation {
    /// Evaluate a token without allocating. Preserve hue and scale premultiplied opacity.
    pub fn shadow(self, color: Color32) -> ElevationShadow {
        let parameters = match self {
            Self::Flat => return ElevationShadow::NONE,
            Self::Floating => {
                let [far, near] = floating_toolbar(color);
                return ElevationShadow::new([far, near, Shadow::NONE, Shadow::NONE]);
            }
            Self::Subtle | Self::Low => [(2, 10), (1, 6), (1, 3), (0, 2)],
            Self::Raised | Self::Medium => [(5, 18), (3, 10), (2, 5), (1, 3)],
            Self::Overlay | Self::High => [(8, 28), (4, 14), (2, 7), (1, 4)],
            Self::Modal => [(12, 44), (6, 20), (3, 10), (1, 5)],
        };
        let weights = [0.30, 0.35, 0.25, 0.10];
        ElevationShadow::new(std::array::from_fn(|i| {
            let (y, blur) = parameters[i];
            Shadow { offset: [0, y], blur, spread: 0, color: color.gamma_multiply(weights[i]) }
        }))
    }

    /// Back-to-front layers, including transparent padding for Flat and Floating.
    pub fn layers(self, color: Color32) -> [ShadowLayer; 4] {
        self.shadow(color).layers
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

/// Configure the egui 0.37 adapter once per theme change. Reconfiguration replaces its rules.
///
/// At the end of each pass it recognizes only an exact two-rectangle egui Frame shape whose
/// caster matches its Area bounds and whose original shadow matches a registered signature.
/// It preserves the painted frame, clipping, layer transform and inherited fade opacity.
/// Other structures (including nested/arbitrary shape vectors) are left untouched.
/// Matching rules must agree on the replacement. Quantized RGBA channels constrain the fade;
/// if their uncertainty can change a replacement channel by more than one unit, it is retained
/// unchanged. This avoids amplifying a faint source into an incorrectly opaque replacement.
/// This also covers popups created inside craft-ui without changing their interaction code.
pub fn install(ctx: &Context, rules: Vec<FrameShadow>) {
    ctx.plugin_or_default::<EguiFrames>().lock().rules = rules;
}

/// Override one known container's recipe for this pass, after its `show` call.
/// Returns false if the adapter is not installed. Unsupported frame structures still fall back
/// unchanged. An empty recipe removes a recognized shadow; it never removes the surface/scrim.
/// If multiple source signatures imply different results, the original shadow is retained.
/// Use [`set_layer_shadows_for_source`] to disambiguate a known container's original shadow.
pub fn set_layer_shadows(ctx: &Context, layer: LayerId, layers: impl Into<Vec<Shadow>>) -> bool {
    set_layer_override(ctx, layer, None, layers)
}

/// Override a container with an explicitly known, registered original shadow signature.
/// This disambiguates signatures differing only in alpha during fades. Precision checks and
/// unsupported-structure fallback still apply. The override expires after the current pass.
/// The return value reports whether the adapter is installed, like [`set_layer_shadows`].
pub fn set_layer_shadows_for_source(ctx: &Context, layer: LayerId, source: Shadow, layers: impl Into<Vec<Shadow>>) -> bool {
    set_layer_override(ctx, layer, Some(source), layers)
}

fn set_layer_override(ctx: &Context, layer: LayerId, source: Option<Shadow>, layers: impl Into<Vec<Shadow>>) -> bool {
    let viewport = ctx.viewport_id();
    ctx.with_plugin(|plugin: &mut EguiFrames| {
        let pass = plugin.passes.entry(viewport).or_default();
        let replacement = LayerOverride { source, layers: layers.into() };
        if let Some((_, previous)) = pass.overrides.iter_mut().find(|(id, _)| *id == layer) {
            *previous = replacement;
        } else {
            pass.overrides.push((layer, replacement));
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
    overrides: Vec<(LayerId, LayerOverride)>,
    results: Vec<(LayerId, FrameStatus)>,
    owned: Vec<(LayerId, ShapeIdx)>,
}

struct LayerOverride {
    source: Option<Shadow>,
    layers: Vec<Shadow>,
}

impl egui::plugin::Plugin for EguiFrames {
    fn debug_name(&self) -> &'static str {
        "craft-ui shadows (egui 0.37)"
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
                if self.rules.is_empty() {
                    pass.results.push((layer, FrameStatus::Unchanged));
                    continue;
                }
                let replacement = pass.overrides.iter().find(|(id, _)| *id == layer).map(|(_, replacement)| replacement);
                let candidate = {
                    let mut candidates =
                        list.all_entries().enumerate().filter(|(i, _)| !pass.owned.contains(&(layer, ShapeIdx(*i)))).filter_map(|(i, entry)| {
                            replacement_shape(&entry.shape, rect, &self.rules, replacement).map(|shape| (ShapeIdx(i), shape))
                        });
                    let first = candidates.next();
                    if candidates.next().is_none() { first } else { None }
                };
                // Ambiguous structure is unsupported: do not guess which entry owns the frame.
                if let Some((index, shape)) = candidate {
                    list.mutate_shape(index, |entry| entry.shape = shape);
                    pass.results.push((layer, FrameStatus::Applied));
                } else {
                    pass.results.push((layer, FrameStatus::Unchanged));
                }
            }
        });
    }
}

/// Quantized channels constrain opacity to an interval, rather than one exact alpha ratio.
struct Opacity {
    value: f32,
    min: f32,
    max: f32,
}

impl Opacity {
    fn infer(source: Color32, painted: Color32) -> Option<Self> {
        if source.a() == 0 || painted.a() > source.a().saturating_add(1) {
            return None;
        }
        // Prefer an exact quantized match. A one-unit alpha tolerance also handles jitter at
        // full opacity; RGB must always be consistent with the same inherited opacity.
        Self::from_channels(source, painted, 0).or_else(|| Self::from_channels(source, painted, 1))
    }

    fn from_channels(source: Color32, painted: Color32, alpha_tolerance: u8) -> Option<Self> {
        let mut min = 0.0_f32;
        let mut max = 1.0_f32;
        for ((source, painted), tolerance) in source.to_array().into_iter().zip(painted.to_array()).zip([0, 0, 0, alpha_tolerance]) {
            if source == 0 {
                if painted > tolerance {
                    return None;
                }
                continue;
            }
            let source = f32::from(source);
            let painted = f32::from(painted);
            let tolerance = f32::from(tolerance);
            min = min.max((painted - tolerance - 0.5) / source);
            max = max.min((painted + tolerance + 0.5) / source);
            if min > max {
                return None;
            }
        }
        let matches = |opacity| {
            let expected = source.gamma_multiply(opacity);
            painted.r() == expected.r()
                && painted.g() == expected.g()
                && painted.b() == expected.b()
                && painted.a().abs_diff(expected.a()) <= alpha_tolerance
        };
        let mut value = (f32::from(painted.a()) / f32::from(source.a())).clamp(min, max);
        if !matches(value) {
            // An interval's upper endpoint can lie exactly on the next rounding boundary.
            value = (min + max) * 0.5;
        }
        matches(value).then_some(Self { value, min, max })
    }

    fn precise_for(&self, layers: &[Shadow]) -> bool {
        layers.iter().all(|layer| {
            let color = layer.color.gamma_multiply(self.value);
            [self.min, self.max].into_iter().all(|opacity| {
                color.to_array().into_iter().zip(layer.color.gamma_multiply(opacity).to_array()).all(|(value, limit)| value.abs_diff(limit) <= 1)
            })
        })
    }
}

fn faded_layers(layers: &[Shadow], opacity: f32) -> impl Iterator<Item = Shadow> + '_ {
    layers.iter().map(move |s| Shadow { color: s.color.gamma_multiply(opacity), ..*s }).filter(|s| s.color != Color32::TRANSPARENT)
}

fn replacement_shape(shape: &Shape, area: Rect, rules: &[FrameShadow], override_: Option<&LayerOverride>) -> Option<Shape> {
    let Shape::Vec(pair) = shape else { return None };
    let [Shape::Rect(shadow), Shape::Rect(frame)] = pair.as_slice() else { return None };
    if !valid(frame.rect) || frame.rect != area || frame.angle != 0.0 || frame.blur_width != 0.0 {
        return None;
    }
    let mut selected: Option<(&[Shadow], f32)> = None;
    for rule in rules {
        if override_.and_then(|replacement| replacement.source).is_some_and(|source| source != rule.source) {
            continue;
        }
        let mut expected = rule.source.as_shape(frame.rect, frame.corner_radius);
        expected.fill = shadow.fill;
        if *shadow != expected {
            continue;
        }
        let Some(opacity) = Opacity::infer(rule.source.color, shadow.fill) else { continue };
        let layers = override_.map_or(rule.layers.as_slice(), |replacement| replacement.layers.as_slice());
        // Do not amplify the source's quantization uncertainty into a visible opacity jump.
        if !opacity.precise_for(layers) {
            return None;
        }
        if let Some((previous, previous_opacity)) = selected
            && !faded_layers(previous, previous_opacity).eq(faded_layers(layers, opacity.value))
        {
            return None;
        }
        selected = Some((layers, opacity.value));
    }
    let (layers, opacity) = selected?;
    let mut result: Vec<_> = faded_layers(layers, opacity).map(|s| Shape::Rect(s.as_shape(frame.rect, frame.corner_radius))).collect();
    result.push(Shape::Rect(frame.clone()));
    Some(Shape::Vec(result))
}

#[cfg(test)]
mod tests;
