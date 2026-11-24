//! Step 13, paint: the root surface, state fill and focus ring around `C::paint`, and the veil of a faded component.

use crate::contract::Pointer as _;
use crate::contract::cx::CxCore;
use crate::primitives::{FocusKind, FocusRing, PaintShape, StateLayer, StateLayerKind, surface_look_shapes};
use crate::runtime::egui::areas::Layer;
use crate::runtime::env::{Env, EnvValue};
use crate::style::{Ink, StateSet, StyleView, surface_style};
use craft_theme::Opacity;
use egui::{Color32, Shape, Ui};

/// The ink the surface reads in: the surface kind of the state, else `inherited`.
pub(crate) fn surface_ink(look: Option<&crate::style::SurfaceLook>, state: &StateSet, inherited: Ink) -> Ink {
    look.and_then(|look| look.kind_in(state)).map_or(inherited, |kind| kind.ink())
}

/// Where a faded component's shapes start in its layer's paint list, and the ground its veil takes.
#[derive(Clone, Copy)]
pub(crate) struct Fade {
    layer: Layer,
    from: usize,
    ground: Color32,
    /// The component lies under the veil of a faded owner that shows it from its paint: it paints its enabled look
    /// and adds no veil of its own.
    covered: bool,
}

/// The layer whose open veil the components shown here lie under: set by `PaintUi` for the children a faded
/// component shows while its paint runs, which its veil covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, crate::contract::MemoProps)]
pub(crate) struct UnderVeil(#[memo_props(hash = "an egui layer id is its order and id")] pub Option<Layer>);

impl EnvValue for UnderVeil {
    fn root() -> Self {
        UnderVeil(None)
    }
}

/// Step 13 for a component that is `faded` (`DisabledLook::Faded`, not enabled): notes where its shapes start in its
/// layer's paint list and the ground colour, the surface it sits on, and stores the fade in `core.fade` for
/// `PaintUi`. `None` for a component that is not faded.
pub(crate) fn fade(ui: &Ui, env: &Env, core: &mut CxCore<'_>) -> Option<Fade> {
    if !core.faded {
        core.fade = None;
        return None;
    }
    let layer = ui.layer_id();
    let covered = env.values.get::<UnderVeil>().0 == Some(layer);
    let fade = Fade { layer, from: next_shape(ui), ground: surface_style(&env.tokens, env.surface).fill, covered };
    core.fade = Some(fade);
    Some(fade)
}

/// Lays the veil over what the component painted since `fade`: one rect of the ground at `1 - Opacity::DISABLED`,
/// filling the clip the paint step set (the component's rect grown by its paint outset) with its surface's corners.
/// Over the ground the component sits on, each colour under it shows mixed toward that ground by the disabled share.
/// A range that painted nothing adds no veil; a later range of the same component moves the veil to its own end, so
/// one veil covers all the component paints. The veil is a shape of the paint list, so a settled view's record,
/// taken when the region closes, keeps it.
pub(crate) fn unfade(ui: &Ui, core: &mut CxCore<'_>, fade: Option<Fade>) {
    core.fade = None;
    let Some(fade) = fade.filter(|fade| !fade.covered && fade.layer == ui.layer_id()) else { return };
    if next_shape(ui) == fade.from {
        return;
    }
    let painter = ui.painter();
    let mut record = core.paint.borrow_mut();
    if let Some(earlier) = record.veil.take() {
        painter.set(earlier, Shape::Noop);
    }
    let veil = Shape::rect_filled(ui.clip_rect(), record.surface_radius, fade.ground.gamma_multiply(1.0 - Opacity::DISABLED));
    record.veil = Some(painter.add(veil));
}

/// The index the next shape `ui` paints gets in its layer's paint list.
pub(crate) fn next_shape(ui: &Ui) -> usize {
    ui.ctx().graphics(|layers| layers.get(ui.layer_id()).map_or(0, |list| list.next_idx().0))
}

/// Step 13, paint, the pipeline's part before `C::paint`: the root surface and the state fill. A faded component's
/// record keeps the surface's corners, which its veil takes.
pub(crate) fn paint_surface(ui: &Ui, env: &Env, core: &CxCore<'_>, style: &dyn StyleView) {
    let Some(look) = style.surface() else { return };
    if core.faded {
        core.paint.borrow_mut().surface_radius = look.radius;
    }
    let mut shapes = surface_look_shapes(&env.tokens, look, &core.state, core.rect, env.direction, ui.ctx().pixels_per_point());
    crate::primitives::fit_to_frame(ui.ctx(), ui.layer_id(), &mut shapes);
    match core.surface_slot {
        Some(slot) => ui.painter().set(slot, egui::Shape::Vec(shapes)),
        None => {
            ui.painter().extend(shapes);
        }
    }
}

/// Step 13, paint, after the root surface: the state layer of every part declared `state_layer`, with control
/// corners, under the component's own paint.
pub(crate) fn paint_part_layers(ui: &Ui, env: &Env, core: &CxCore<'_>) {
    if core.state_layer_parts.is_empty() {
        return;
    }
    let look = crate::style::state_layer_look(&env.tokens);
    let radius = crate::style::UiTokens::corner(env.tokens.shape.radius.control);
    for &(key, index) in &core.state_layer_parts {
        let part = craft_behavior::PartRef { key, index };
        let fill = core.part_state(part).resolve(&look);
        if fill.egui().a() > 0 {
            ui.painter().extend(StateLayer::shapes(StateLayerKind::Fill, fill, core.part(part).rect, radius, 0.0));
        }
    }
}

/// Step 13, paint, after `C::paint`: the focus ring of the focused component or its active part, when focus is
/// keyboard-visible. An active part's ring takes the shape the part declared (`focus_shape`), else its rect; the
/// component's own ring takes the shape it declared (`Cx::focus_shape`), else its rect with its surface's corners. Never
/// on `Well` surfaces.
pub(crate) fn paint_focus(ui: &Ui, core: &CxCore<'_>, style: &dyn StyleView) {
    let Some(look) = style.focus() else { return };
    let ppp = ui.ctx().pixels_per_point();
    let flags = core.state.flags;
    let well = style.surface().is_some_and(|surface| surface.kind == Some(crate::style::SurfaceKind::Well));
    if let Some(part) = core.active.filter(|_| FocusRing::visible(flags)) {
        let rect = core.part(part).rect;
        let declared = core.focus_shapes.iter().find(|((key, index), _)| craft_behavior::PartRef { key: *key, index: *index } == part);
        let outline = declared.map_or(PaintShape::Rect(rect, egui::CornerRadius::ZERO), |(_, shape)| FocusRing::outline(shape, rect));
        ui.painter().extend(FocusRing::shapes(look, FocusKind::Ring, &outline, ppp));
    } else if FocusRing::visible(flags) && !well {
        let outline = match &core.focus_shape {
            Some(shape) => FocusRing::outline(shape, core.rect),
            None => PaintShape::Rect(core.rect, style.surface().map_or(egui::CornerRadius::ZERO, |surface| surface.radius)),
        };
        ui.painter().extend(FocusRing::shapes(look, FocusKind::Ring, &outline, ppp));
    }
}
