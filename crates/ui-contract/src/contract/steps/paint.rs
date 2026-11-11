//! Step 13, paint: the root surface, state fill and focus ring around `C::paint`.

use crate::contract::{Component, Cx, Pointer as _, Styled as _};
use crate::primitives::{FocusKind, FocusRing, PaintShape, StateLayer, StateLayerKind, Surface};
use crate::runtime::env::Env;
use crate::style::{ComponentStyle, Ink, StateSet, SurfaceEdge, surface_style};
use egui::Ui;

/// The ink the surface reads in: the surface kind of the state, else `inherited`.
pub(crate) fn surface_ink(look: Option<&crate::style::SurfaceLook>, state: &StateSet, inherited: Ink) -> Ink {
    look.and_then(|look| look.kind_in(state)).map_or(inherited, |kind| kind.ink())
}

/// Step 13, paint, the pipeline's part before `C::paint`: the root surface and the state fill.
pub(crate) fn paint_surface<C: Component>(ui: &Ui, env: &Env, cx: &Cx<'_, C>) {
    let Some(look) = cx.style.surface() else { return };
    let fill = cx.core.resolve(&look.fill);
    let ppp = ui.ctx().pixels_per_point();
    let shapes = match look.kind_in(&cx.core.state) {
        Some(kind) => {
            let mut style = surface_style(&env.tokens, kind);
            // A look that names its own radius (a capsule button) shapes the kind's fill and edge too, so no plate with the kind's corners shows outside it.
            if look.radius != egui::CornerRadius::ZERO {
                style.radius = look.radius;
            }
            if look.border_replaces_edge && matches!(style.edge, SurfaceEdge::Border(_)) {
                style.edge = SurfaceEdge::None;
            }
            let mut shapes = Surface::shapes(&style, Some(fill.egui()), cx.core.rect, look.corners, env.direction, ppp);
            if let Some(border) = &look.border {
                let radius = look.corners.apply(look.radius, env.direction);
                let color = cx.core.resolve(border);
                shapes.extend(StateLayer::shapes(StateLayerKind::Outline, color, cx.core.rect, radius, look.border_width));
            }
            shapes
        }
        None => {
            let radius = look.corners.apply(look.radius, env.direction);
            let mut shapes = StateLayer::shapes(StateLayerKind::Fill, fill, cx.core.rect, radius, 0.0);
            if let Some(border) = &look.border {
                let color = cx.core.resolve(border);
                shapes.extend(StateLayer::shapes(StateLayerKind::Outline, color, cx.core.rect, radius, look.border_width));
            }
            shapes
        }
    };
    match cx.core.surface_slot {
        Some(slot) => ui.painter().set(slot, egui::Shape::Vec(shapes)),
        None => {
            ui.painter().extend(shapes);
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
