//! The `Pointer` trait: presses, hover, drags and cursors, on `CxCore` and `Cx`.

use super::context::{Activation, Cx, CxCore};
use crate::contract::Component;
use crate::primitives::PartResult;
use crate::runtime::drag::{self, DragPreview, DropTarget, WindowDropTarget};
use crate::runtime::egui::areas;
use crate::runtime::{cursor, frame, pointer, wheel};
use craft_behavior::{Cursor, CursorPriority, DropOperation, PartKey, PartRef, PressCancel, PressState, Wheel};
use egui::{Id, Pos2};
use std::ops::Range;
use std::sync::Arc;

/// Pointer state and requests (chapter 8).
pub trait Pointer {
    /// The result of `part` this pass: idle for a part first registered this frame. A part `parts` did not
    /// declare is a debug panic.
    fn part(&self, part: impl Into<PartRef>) -> &PartResult;
    /// The result of `part` this pass, `None` when `parts` did not declare it: for parts that exist only under a
    /// condition.
    fn part_if_declared(&self, part: impl Into<PartRef>) -> Option<&PartResult>;
    /// Where the pointer is over `part` this pass, in the component's layer space; `None` when `part` is not hovered.
    fn hover_pos(&self, part: impl Into<PartRef>) -> Option<Pos2> {
        self.part_if_declared(part).and_then(|result| result.hover_pos)
    }
    /// The component's own press.
    fn press(&self) -> &PressState;
    /// The pointer is over the component's own shapes.
    fn hit(&self) -> bool;
    /// Set only when the component's own shapes won the press and it was released as a click, or
    /// `Intent::Activate` was routed to it.
    fn activation(&self) -> Option<Activation>;
    /// For an arena root, the member activated this pass.
    fn activated_member(&self) -> Option<Id>;
    /// This pass's wheel share, once.
    fn take_wheel(&mut self) -> Option<Wheel>;
    /// Requests `cursor` at `priority`; the highest priority of the pass wins.
    fn cursor(&mut self, cursor: impl Into<Cursor>, priority: CursorPriority);
    /// Ends every captured press of this viewport.
    fn cancel_press(&mut self, reason: PressCancel);
    /// Starts the drag session with `payload`; the context holds one session at a time, and a running one stays.
    fn start_drag<P: Clone + Send + Sync + 'static>(&mut self, payload: P, preview: DragPreview);
    /// Starts the drag session with `payload`, which also carries `also`: targets of either type see the one drag.
    fn start_drag_also<P: Clone + Send + Sync + 'static>(&mut self, payload: P, also: Arc<dyn std::any::Any + Send + Sync>, preview: DragPreview);
    /// The active drag payload of type `P` and the pointer position in this component's egui layer space (8.3), from any
    /// viewport.
    fn dragging<P: Send + Sync + 'static>(&self) -> Option<(Arc<P>, Pos2)>;
    /// Registers `target` over this component's rect for this pass, in `update`. Returns the drop released over it this
    /// pass, when it was the topmost accepting target under the pointer last pass and accepts now.
    fn drop_target<P: Send + Sync + 'static, T: DropTarget<P>>(&mut self, target: &T) -> Option<T::Drop>;
    /// Registers `target` over the whole window of the running viewport for this pass, in `update`; a target over a
    /// component beats it wherever that accepts. The payload is read without a pointer position, so files hovered
    /// before the pointer has a position reach it. OS file drops (`OsFiles`) land on it wherever no other target
    /// takes them.
    fn window_drop_target<P: Send + Sync + 'static, T: WindowDropTarget<P>>(&mut self, target: &T) -> Option<T::Drop>;
    /// The operation of a drop of this component's payload, in the pass after the drop; `None` while dragging, when the
    /// drag was cancelled and when no target took it.
    fn drag_ended<P: Send + Sync + 'static>(&self) -> Option<DropOperation>;
    /// The visible range the scroll part `key` moved to this pass (thumb or end drag, page press, or an AccessKit click
    /// on the track, which pages toward the track's centre).
    fn scroll_moved(&self, key: PartKey) -> Option<Range<f64>>;
}

impl Pointer for CxCore<'_> {
    fn part(&self, part: impl Into<PartRef>) -> &PartResult {
        let part = part.into();
        match self.results.get(&(part.key, part.index)) {
            Some(result) => result,
            None => {
                // A measuring run has no parts: its core is blank, so every part reads idle.
                debug_assert!(
                    self.face_mode || crate::runtime::measure::active(&self.ctx) || self.results.is_empty() && self.stated,
                    "`parts` did not declare the part {part:?}"
                );
                &PartResult::IDLE
            }
        }
    }

    fn part_if_declared(&self, part: impl Into<PartRef>) -> Option<&PartResult> {
        let part = part.into();
        self.results.get(&(part.key, part.index))
    }

    fn press(&self) -> &PressState {
        &self.press
    }

    fn hit(&self) -> bool {
        self.response.hovered()
    }

    fn activation(&self) -> Option<Activation> {
        self.activation
    }

    fn activated_member(&self) -> Option<Id> {
        self.activated_member
    }

    fn take_wheel(&mut self) -> Option<Wheel> {
        wheel::take(&self.ctx, self.id)
    }

    fn cursor(&mut self, cursor: impl Into<Cursor>, priority: CursorPriority) {
        if self.cursor.as_ref().is_none_or(|(_, held)| priority > *held) {
            self.cursor = Some((cursor.into(), priority));
        }
    }

    fn cancel_press(&mut self, reason: PressCancel) {
        pointer::cancel_press(&self.ctx, reason);
        frame::note_cancelled_press(&self.ctx);
        self.press_cancel = Some(reason);
    }

    fn start_drag<P: Clone + Send + Sync + 'static>(&mut self, payload: P, preview: DragPreview) {
        drag::start(&self.ctx, self.id, Arc::new(payload), None, preview);
    }

    fn start_drag_also<P: Clone + Send + Sync + 'static>(&mut self, payload: P, also: Arc<dyn std::any::Any + Send + Sync>, preview: DragPreview) {
        drag::start(&self.ctx, self.id, Arc::new(payload), Some(also), preview);
    }

    fn dragging<P: Send + Sync + 'static>(&self) -> Option<(Arc<P>, Pos2)> {
        let (payload, at) = drag::dragging::<P>(&self.ctx)?;
        Some((payload, areas::layer_map(&self.ctx, self.response.layer_id).map_or(at, |map| map.to_layer(at))))
    }

    fn drop_target<P: Send + Sync + 'static, T: DropTarget<P>>(&mut self, target: &T) -> Option<T::Drop> {
        let (payload, at) = self.dragging::<P>()?;
        // Only where the pointer is over this component's visible shapes on its own layer: not under a layer above, not
        // in a clipped-out part. The layer a dragged container moves is not above anything: the pointer is over what
        // lies under it.
        let screen = areas::layer_map(&self.ctx, self.response.layer_id).map_or(at, |map| map.to_screen(at));
        let top = match drag::moving_layer(&self.ctx) {
            Some(moving) => areas::layer_at_without(&self.ctx, screen, moving),
            None => areas::layer_at(&self.ctx, screen),
        };
        if drag::moves_viewport(&self.ctx) || !self.visible.contains(at) || top != Some(self.response.layer_id) {
            return None;
        }
        let op = drag::current_operation(&self.ctx);
        let drop = target.accept(&payload, at, op)?;
        let ordinal = drag::next_ordinal(&self.ctx, self.id);
        drag::register(&self.ctx, self.id, self.response.layer_id, target.mark(&drop), false);
        drag::take_drop::<P>(&self.ctx, self.id, ordinal, op).then_some(drop)
    }

    fn window_drop_target<P: Send + Sync + 'static, T: WindowDropTarget<P>>(&mut self, target: &T) -> Option<T::Drop> {
        let payload = drag::active::<P>(&self.ctx).filter(|_| !drag::moves_viewport(&self.ctx))?;
        let op = drag::current_operation(&self.ctx);
        let drop = target.accept(&payload, op)?;
        let ordinal = drag::next_ordinal(&self.ctx, self.id);
        drag::register(&self.ctx, self.id, self.response.layer_id, target.mark(&drop), true);
        drag::take_drop::<P>(&self.ctx, self.id, ordinal, op).then_some(drop)
    }

    fn drag_ended<P: Send + Sync + 'static>(&self) -> Option<DropOperation> {
        drag::ended::<P>(&self.ctx, self.id)
    }

    fn scroll_moved(&self, key: PartKey) -> Option<Range<f64>> {
        let moves = self.scroll_moves.iter().find(|moved| moved.key == key)?;
        moves.pressed.clone().or_else(|| moves.clicked.clone().filter(|_| self.intents.access_click(key.into())))
    }
}

impl<C: Component> Pointer for Cx<'_, C> {
    fn part(&self, part: impl Into<PartRef>) -> &PartResult {
        self.core.part(part)
    }

    fn part_if_declared(&self, part: impl Into<PartRef>) -> Option<&PartResult> {
        self.core.part_if_declared(part)
    }

    fn press(&self) -> &PressState {
        self.core.press()
    }

    fn hit(&self) -> bool {
        self.core.hit()
    }

    fn activation(&self) -> Option<Activation> {
        self.core.activation()
    }

    fn activated_member(&self) -> Option<Id> {
        self.core.activated_member()
    }

    fn take_wheel(&mut self) -> Option<Wheel> {
        self.core.take_wheel()
    }

    fn cursor(&mut self, cursor: impl Into<Cursor>, priority: CursorPriority) {
        self.core.cursor(cursor, priority);
    }

    fn cancel_press(&mut self, reason: PressCancel) {
        self.core.cancel_press(reason);
    }

    fn start_drag<P: Clone + Send + Sync + 'static>(&mut self, payload: P, preview: DragPreview) {
        self.core.start_drag(payload, preview);
    }

    fn start_drag_also<P: Clone + Send + Sync + 'static>(&mut self, payload: P, also: Arc<dyn std::any::Any + Send + Sync>, preview: DragPreview) {
        self.core.start_drag_also(payload, also, preview);
    }

    fn dragging<P: Send + Sync + 'static>(&self) -> Option<(Arc<P>, Pos2)> {
        self.core.dragging::<P>()
    }

    fn drop_target<P: Send + Sync + 'static, T: DropTarget<P>>(&mut self, target: &T) -> Option<T::Drop> {
        self.core.drop_target(target)
    }

    fn window_drop_target<P: Send + Sync + 'static, T: WindowDropTarget<P>>(&mut self, target: &T) -> Option<T::Drop> {
        self.core.window_drop_target(target)
    }

    fn drag_ended<P: Send + Sync + 'static>(&self) -> Option<DropOperation> {
        self.core.drag_ended::<P>()
    }

    fn scroll_moved(&self, key: PartKey) -> Option<Range<f64>> {
        self.core.scroll_moved(key)
    }
}

/// The cursor requests of the hovered parts' declared cursors, in registration order.
pub(crate) fn declared_cursor(core: &CxCore<'_>) -> Option<Cursor> {
    core.part_cursors.first().cloned()
}

/// Writes the cursor a component asked for into the cursor service.
pub(crate) fn write_cursor(core: &CxCore<'_>) {
    if let Some((cursor, priority)) = core.cursor.clone() {
        cursor::request(&core.ctx, cursor, priority);
    } else if let Some(cursor) = declared_cursor(core) {
        cursor::request(&core.ctx, cursor, CursorPriority::Part);
    }
}
