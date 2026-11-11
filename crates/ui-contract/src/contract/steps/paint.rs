//! Step 13, paint: the root surface, state fill and focus ring around `C::paint`.

use crate::contract::{Component, Cx, Pointer as _};
use crate::primitives::{FocusKind, FocusRing, PaintShape, StateLayer, StateLayerKind, surface_look_shapes};
use crate::runtime::env::Env;
use crate::style::{ComponentStyle, Ink, StateSet};
use egui::Ui;

/// The ink the surface reads in: the surface kind of the state, else `inherited`.
pub(crate) fn surface_ink(look: Option<&crate::style::SurfaceLook>, state: &StateSet, inherited: Ink) -> Ink {
    look.and_then(|look| look.kind_in(state)).map_or(inherited, |kind| kind.ink())
}

/// Step 13, paint, the pipeline's part before `C::paint`: the root surface and the state fill.
pub(crate) fn paint_surface<C: Component>(ui: &Ui, env: &Env, cx: &Cx<'_, C>) {
    let Some(look) = cx.style.surface() else { return };
    let shapes = surface_look_shapes(&env.tokens, look, &cx.core.state, cx.core.rect, env.direction, ui.ctx().pixels_per_point());
    match cx.core.surface_slot {
        Some(slot) => ui.painter().set(slot, egui::Shape::Vec(shapes)),
        None => {
            ui.painter().extend(shapes);
        }
    }
}

/// Step 13, paint, after the root surface: the state layer of every part declared `state_layer`, with control
/// corners, under the component's own paint.
pub(crate) fn paint_part_layers<C: Component>(ui: &Ui, env: &Env, cx: &Cx<'_, C>) {
    if cx.core.state_layer_parts.is_empty() {
        return;
    }
    let look = crate::style::state_layer_look(&env.tokens);
    let radius = crate::style::UiTokens::corner(env.tokens.shape.radius.control);
    for &(key, index) in &cx.core.state_layer_parts {
        let part = craft_behavior::PartRef { key, index };
        let fill = cx.core.part_state(part).resolve(&look);
        if fill.egui().a() > 0 {
            ui.painter().extend(StateLayer::shapes(StateLayerKind::Fill, fill, cx.core.part(part).rect, radius, 0.0));
        }
    }
}

/// Step 13, paint, after `C::paint`: the focus ring of the focused component or its active part, when focus is
/// keyboard-visible. Never on `Well` surfaces.
pub(crate) fn paint_focus<C: Component>(ui: &Ui, cx: &Cx<'_, C>) {
    let Some(look) = cx.style.focus() else { return };
    let ppp = ui.ctx().pixels_per_point();
    let flags = cx.core.state.flags;
    let well = cx.style.surface().is_some_and(|surface| surface.kind == Some(crate::style::SurfaceKind::Well));
    if let Some(part) = cx.core.active.filter(|_| FocusRing::visible(flags)) {
        let rect = cx.core.part(part).rect;
        ui.painter().extend(FocusRing::shapes(look, FocusKind::Ring, &PaintShape::Rect(rect, egui::CornerRadius::ZERO), ppp));
    } else if FocusRing::visible(flags) && !well {
        let radius = cx.style.surface().map_or(egui::CornerRadius::ZERO, |surface| surface.radius);
        ui.painter().extend(FocusRing::shapes(look, FocusKind::Ring, &PaintShape::Rect(cx.core.rect, radius), ppp));
    }
}
