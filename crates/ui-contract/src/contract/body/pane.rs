//! The grip of an edge pane and its drag.

use super::BodyHooks;
use crate::contract::mount::{PaneGrip, pane_extent_id, pane_grip_id};
use crate::contract::overlay::{PlacementChange, PlacementKind};
use crate::contract::{Container, ContainerProps, Cx, Mount, PaneEdge, PartsCx};
use crate::runtime::layout_memory;
use crate::runtime::windows::ResizeEdge;
use craft_behavior::{CursorIcon, GesturePhase, HitLevel, HitPriority, PartKey, PressPhase};
use egui::{Pos2, Rect, Vec2};

/// The grip on an edge pane's inner edge.
const PANE_GRIP: PartKey = PartKey::new("pane_grip");

impl BodyHooks<'_> {
    /// The grip of an edge pane (`Mount::Pane`) on its inner edge, the edge toward the centre, inside the pane: dragging
    /// it gives the pane a new extent along its edge (`pane_drag`). A pane its caller lays out (`ShellPane::at`) and the
    /// central pane have none. Returns the edge the grip sits on and the least extent it gives.
    pub(super) fn pane_grip<C: Container>(props: &ContainerProps, rect: Rect, pcx: &mut PartsCx<'_, C>) -> Option<(ResizeEdge, f32)> {
        let Mount::Pane(pane) = &props.mount else { return None };
        let rtl = pcx.flow.is_rtl();
        let edge = match pane.edge {
            _ if pane.at.is_some() => return None,
            PaneEdge::Start if rtl => ResizeEdge::Left,
            PaneEdge::Start => ResizeEdge::Right,
            PaneEdge::End if rtl => ResizeEdge::Right,
            PaneEdge::End => ResizeEdge::Left,
            PaneEdge::Top => ResizeEdge::Bottom,
            PaneEdge::Bottom => ResizeEdge::Top,
            _ => return None,
        };
        let tokens = pcx.tokens.tokens();
        let (handle, least) = (tokens.size.handle.sm, tokens.size.pane.least);
        let (grip, cursor) = match edge {
            ResizeEdge::Left => (Rect::from_min_max(rect.min, Pos2::new(rect.min.x + handle, rect.max.y)), CursorIcon::ResizeHorizontal),
            ResizeEdge::Right => (Rect::from_min_max(Pos2::new(rect.max.x - handle, rect.min.y), rect.max), CursorIcon::ResizeHorizontal),
            ResizeEdge::Top => (Rect::from_min_max(rect.min, Pos2::new(rect.max.x, rect.min.y + handle)), CursorIcon::ResizeVertical),
            _ => (Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - handle), rect.max), CursorIcon::ResizeVertical),
        };
        pcx.part(PANE_GRIP, grip)
            .sense(craft_behavior::Sense::DRAG)
            .priority(HitPriority::new(HitLevel::Edge, 0))
            .cursor(cursor)
            .pointer_only("no command resizes a docked pane; its extent is the app's layout");
        Some((edge, least))
    }

    /// A drag of an edge pane's grip: the pane's extent follows the pointer, at least the least extent, and each step
    /// is reported as a `PlacementKind::Size` change of the pane's rect. While the gesture runs the pane host shows its
    /// extent (`PaneGrip`); at the commit a pane with no fixed extent keeps it in pane memory, and one whose caller fixes
    /// the extent shows the caller's from the next pass. Escape, which cancels the press, restores the extent.
    pub(super) fn pane_drag<C: Container>(&mut self, props: &ContainerProps, cx: &mut Cx<'_, C>) {
        let (Mount::Pane(pane), Some((edge, least))) = (&props.mount, self.pane_grip) else { return };
        let Some(phase) = cx.core.results.get(&(PANE_GRIP, None)).map(|result| result.press.phase) else { return };
        let (origin, pos, gesture) = match phase {
            PressPhase::DragStart { start, pos, .. } => (start.origin, Some(pos), GesturePhase::Begin),
            PressPhase::Drag { start, pos, .. } => (start.origin, Some(pos), GesturePhase::Update),
            PressPhase::Release { start, pos, .. } => (start.origin, Some(pos), GesturePhase::Commit),
            PressPhase::Cancel { start, .. } => (start.origin, None, GesturePhase::Cancel),
            _ => return,
        };
        let (ctx, id, rect) = (cx.core.ctx.clone(), cx.core.id, cx.core.rect);
        let horizontal = matches!(edge, ResizeEdge::Left | ResizeEdge::Right);
        let along = |size: Vec2| if horizontal { size.x } else { size.y };
        let slot = cx.core.memory.shown::<PaneGrip>(pane_grip_id(id));
        let shown = slot.extent;
        let began = match slot.began {
            Some(began) if gesture != GesturePhase::Begin => began,
            _ => along(rect.size()),
        };
        let moved = pos.map_or(0.0, |pos| match edge {
            ResizeEdge::Right => pos.x - origin.x,
            ResizeEdge::Left => origin.x - pos.x,
            ResizeEdge::Bottom => pos.y - origin.y,
            _ => origin.y - pos.y,
        });
        let extent = (began + moved).max(least);
        let resized = |extent: f32| match edge {
            ResizeEdge::Right => Rect::from_min_size(rect.min, Vec2::new(extent, rect.height())),
            ResizeEdge::Left => Rect::from_min_max(Pos2::new(rect.max.x - extent, rect.min.y), rect.max),
            ResizeEdge::Bottom => Rect::from_min_size(rect.min, Vec2::new(rect.width(), extent)),
            _ => Rect::from_min_max(Pos2::new(rect.min.x, rect.max.y - extent), rect.max),
        };
        let change = |rect: Rect, phase: GesturePhase| PlacementChange { rect, phase, pointer: pos, kind: PlacementKind::Size(edge) };
        if gesture == GesturePhase::Begin {
            self.reports.push(change(resized(began), GesturePhase::Begin));
        }
        let (phase, extent) = match gesture {
            GesturePhase::Begin => (GesturePhase::Update, extent),
            GesturePhase::Cancel => (GesturePhase::Cancel, began),
            phase => (phase, extent),
        };
        self.reports.push(change(resized(extent), phase));
        *slot = if matches!(phase, GesturePhase::Commit | GesturePhase::Cancel) {
            PaneGrip::default()
        } else {
            PaneGrip { began: Some(began), extent: Some(extent) }
        };
        if phase == GesturePhase::Commit && pane.extent.is_none() {
            let size = rect.size();
            layout_memory::record(&ctx, pane_extent_id(id), if horizontal { Vec2::new(extent, size.y) } else { Vec2::new(size.x, extent) });
        }
        // The pane showed this pass at the extent of the last step: a commit or cancel that lands elsewhere shows in
        // the frame's discard pass.
        if matches!(phase, GesturePhase::Commit | GesturePhase::Cancel) && shown.is_some_and(|shown| (shown - extent).abs() > 0.5) {
            layout_memory::ask_discard(&ctx);
        }
    }
}
