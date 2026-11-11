//! Where a container on a layer sits: its remembered placement and the overlay and opener of its mount.

use crate::contract::overlay::{OverlayFit, Place};
use crate::contract::{ContainerProps, Mount};
use crate::runtime::egui::areas;
use crate::runtime::memory::{MemoryStage, SaveKey, sub};
use crate::runtime::windows::{self};
use egui::{Context, Id, Pos2, Rect, Ui, Vec2};
use serde::{Deserialize, Serialize};

/// Where a container on a layer sits and how large the user made it.
#[derive(Clone, Default, Serialize, Deserialize, PartialEq)]
pub(crate) struct Placement {
    pub pos: Option<[f32; 2]>,
    pub size: Option<[f32; 2]>,
}

impl Placement {
    /// The placement that puts the layer at `rect`.
    pub(super) fn at(rect: Rect) -> Placement {
        Placement { pos: Some(rect.min.into()), size: Some(rect.size().into()) }
    }

    /// The placement that puts the layer's top left at `pos` and leaves its size to its content.
    fn at_pos(pos: Pos2) -> Placement {
        Placement { pos: Some(pos.into()), size: None }
    }

    /// The placement the caller of `overlay` gives, when it places the layer.
    fn given(overlay: &crate::contract::Overlay) -> Option<Placement> {
        overlay.placed.map(Placement::at).or_else(|| overlay.placed_at.map(Placement::at_pos))
    }

    /// The rect it names, taking the position and size it lacks from `rect`.
    pub(super) fn rect_over(&self, rect: Rect) -> Rect {
        Rect::from_min_size(self.pos.map_or(rect.min, Pos2::from), self.size.map_or(rect.size(), Vec2::from))
    }
}

pub(crate) fn placement_id(id: Id) -> Id {
    sub(id, "placement")
}

/// The one store of a layer's placement: the saved tier when `save` is set, else shown memory. A layer whose overlay
/// is `placed` has the caller's rect instead: the slot reads it and writes nothing, since the caller owns the
/// placement and learns of the user's changes from `PlacementChange` reports.
pub(super) struct PlacementSlot<'s> {
    stage: &'s mut MemoryStage,
    id: Id,
    save: Option<&'s SaveKey>,
    controlled: Option<Placement>,
}

impl<'s> PlacementSlot<'s> {
    pub(super) fn new(stage: &'s mut MemoryStage, id: Id, save: Option<&'s SaveKey>, overlay: &crate::contract::Overlay) -> Self {
        PlacementSlot { stage, id, save, controlled: Placement::given(overlay) }
    }

    fn stored(&mut self) -> &mut Placement {
        match self.save {
            Some(key) => self.stage.saved::<Placement>(key),
            None => self.stage.shown::<Placement>(placement_id(self.id)),
        }
    }

    pub(super) fn read(&mut self) -> Placement {
        match &self.controlled {
            Some(placement) => placement.clone(),
            None => self.stored().clone(),
        }
    }

    pub(super) fn write(&mut self, placement: Placement) {
        if self.controlled.is_none() {
            *self.stored() = placement;
        }
    }
}

/// The placement of `id` as the last pass left it, read without opening a stage: the layer host places by it before
/// the container's own pass runs.
fn placement_of(ctx: &Context, id: Id, save: Option<&SaveKey>) -> Placement {
    match save {
        Some(key) => MemoryStage::peek_saved(ctx, key),
        None => MemoryStage::peek(ctx, placement_id(id)),
    }
}

/// The placement a layer is shown at: the caller's `placed` rect or `placed_at` position, else the saved or session placement, else for a
/// floating resizable layer with a default size (`Overlay::size`) that size, within the screen less `screen_inset` on
/// each side, centred or at the overlay's float position. A saved size and position win over the default.
pub(crate) fn shown_placement(
    ctx: &Context,
    id: Id,
    save: Option<&SaveKey>,
    overlay: &crate::contract::Overlay,
    screen: Rect,
    screen_inset: f32,
) -> Placement {
    if let Some(given) = Placement::given(overlay) {
        return given;
    }
    let mut placement = placement_of(ctx, id, save);
    let (Some(default), Place::Float(float), true) = (overlay.size, &overlay.place, overlay.fit == OverlayFit::Resizable) else {
        return placement;
    };
    if placement.size.is_some() {
        return placement;
    }
    let room = (screen.size() - Vec2::splat(2.0 * screen_inset)).max(Vec2::ZERO);
    let least = overlay.size_limits.min.min(room);
    let size = default.max(least).min(overlay.size_limits.max.unwrap_or(Vec2::INFINITY)).min(room);
    let pos = placement.pos.map(Pos2::from).or(float.pos).unwrap_or_else(|| screen.center() - size / 2.0);
    let pos = Pos2::new(pos.x.min(screen.right() - size.x).max(screen.left()), pos.y.min(screen.bottom() - size.y).max(screen.top()));
    placement.pos = Some(pos.into());
    placement.size = Some(size.into());
    placement
}

/// The overlay a layer-like mount shows its container in.
pub(super) fn overlay_of(props: &ContainerProps) -> Option<&crate::contract::Overlay> {
    match &props.mount {
        Mount::Layer(overlay) | Mount::Viewport { fallback: overlay, .. } => Some(overlay),
        _ => None,
    }
}

/// The layer a measuring run lays the regions out on: the one the real run shows them on, since egui keeps a
/// region's `Ui` on one layer per pass. A viewport's own window keeps its widgets apart, so it measures on the parent's.
pub(super) fn measure_layer(props: &ContainerProps, id: Id, ui: &Ui) -> areas::Layer {
    let area = sub(id, "area");
    match &props.mount {
        Mount::Viewport { .. } if windows::WindowHost::of(ui.ctx()).viewports_available() => ui.layer_id(),
        Mount::Layer(overlay) | Mount::Viewport { fallback: overlay, .. } => areas::layer_of(overlay.rank.tier(), area),
        Mount::FullWindow => areas::layer_of(crate::contract::mount::full_window_overlay(props.focus).rank.tier(), area),
        _ => ui.layer_id(),
    }
}

/// The opener of a layer-like mount's overlay: a container with no `open` binding takes its open state from it.
pub(super) fn opener_of(props: &ContainerProps) -> Option<&crate::contract::overlay::Opener> {
    overlay_of(props)?.opener.as_ref()
}
