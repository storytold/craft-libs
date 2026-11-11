//! Step 13, paint: the root surface, state fill and focus ring around `C::paint`.

use crate::contract::Pointer as _;
use crate::contract::cx::CxCore;
use crate::primitives::{FocusKind, FocusRing, PaintShape, StateLayer, StateLayerKind, surface_look_shapes};
use crate::runtime::env::Env;
use crate::style::{Ink, StateSet, StyleView};
use egui::Ui;

/// The ink the surface reads in: the surface kind of the state, else `inherited`.
pub(crate) fn surface_ink(look: Option<&crate::style::SurfaceLook>, state: &StateSet, inherited: Ink) -> Ink {
    look.and_then(|look| look.kind_in(state)).map_or(inherited, |kind| kind.ink())
}

/// Step 13, paint, the pipeline's part before `C::paint`: the root surface and the state fill.
pub(crate) fn paint_surface(ui: &Ui, env: &Env, core: &CxCore<'_>, style: &dyn StyleView) {
    let Some(look) = style.surface() else { return };
    let shapes = surface_look_shapes(&env.tokens, look, &core.state, core.rect, env.direction, ui.ctx().pixels_per_point());
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
/// keyboard-visible. Never on `Well` surfaces.
pub(crate) fn paint_focus(ui: &Ui, core: &CxCore<'_>, style: &dyn StyleView) {
    let Some(look) = style.focus() else { return };
    let ppp = ui.ctx().pixels_per_point();
    let flags = core.state.flags;
    let well = style.surface().is_some_and(|surface| surface.kind == Some(crate::style::SurfaceKind::Well));
    if let Some(part) = core.active.filter(|_| FocusRing::visible(flags)) {
        let rect = core.part(part).rect;
        ui.painter().extend(FocusRing::shapes(look, FocusKind::Ring, &PaintShape::Rect(rect, egui::CornerRadius::ZERO), ppp));
    } else if FocusRing::visible(flags) && !well {
        let radius = style.surface().map_or(egui::CornerRadius::ZERO, |surface| surface.radius);
        ui.painter().extend(FocusRing::shapes(look, FocusKind::Ring, &PaintShape::Rect(core.rect, radius), ppp));
    }
}
