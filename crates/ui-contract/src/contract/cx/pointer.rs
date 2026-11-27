//! The `Pointer` trait: presses, hover, drags and cursors, on `CxCore` and `Cx`.

use super::context::{Activation, Cx, CxCore};
use crate::contract::{Component, Hover};
use crate::primitives::PartResult;
use crate::runtime::drag::{self, DragPreview, DropTarget, DropZone, Typed, WindowDropTarget};
use crate::runtime::egui::areas;
use crate::runtime::{cursor, frame, pointer, wheel};
use craft_behavior::{Cursor, CursorPriority, DropOperation, MultiTap, PartKey, PartRef, PressCancel, PressState, Wheel};
use egui::{Id, Pos2, Vec2};
use std::any::{Any, TypeId};
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
    /// The read marks the part as following the pointer (`pointer::follows_pointer`): a memoized view around the
    /// component runs on each move over the part, and the pointer-move gate runs a pass for it. A component that only
    /// asks whether the part is hovered reads `PartResult::hovered`.
    fn hover_pos(&self, part: impl Into<PartRef>) -> Option<Pos2>;
    /// The component's own press.
    fn press(&self) -> &PressState;
    /// The pointer is over the component's own shapes.
    fn hit(&self) -> bool;
    /// The pointer's stay over the component: over it, entered, left, and clicked while it has stayed over.
    fn hover(&self) -> Hover;
    /// Set only when the component's own shapes won the press and it was released as a click, or
    /// `Intent::Activate` was routed to it.
    fn activation(&self) -> Option<Activation>;
    /// For an arena root, the member activated this pass.
    fn activated_member(&self) -> Option<Id>;
    /// This pass's wheel share, once, with the pointer it came at (`Wheel::at`) in this component's egui layer space (8.3).
    fn take_wheel(&mut self) -> Option<Wheel>;
    /// This pass's tap, once, when it lies in this component's rect and this component (an arena root with its members)
    /// wins a press there.
    fn take_multi_tap(&mut self) -> Option<MultiTap>;
    /// Marks the wheel gesture this component owns, while it runs, as moved (its view changed): later shares of the
    /// gesture report `Wheel::moved`.
    fn mark_wheel_moved(&mut self);
    /// Feeds `pan`, the line pan this pass gave, to this component's wheel ease and returns the eased step for this
    /// pass. `Vec2::ZERO` on a pass with no line pan lets owed motion keep paying.
    fn ease_wheel(&mut self, pan: Vec2) -> Vec2;
    /// Drops this component's wheel ease and the motion it owes.
    fn stop_wheel_ease(&mut self);
    /// Requests `cursor` at `priority`; the highest priority of the pass wins.
    fn cursor(&mut self, cursor: impl Into<Cursor>, priority: CursorPriority);
    /// Ends every captured press of this viewport.
    fn cancel_press(&mut self, reason: PressCancel);
    /// Starts the drag session with `payload`; the context holds one session at a time, and a running one stays.
    fn start_drag<P: Clone + Send + Sync + 'static>(&mut self, payload: P, preview: DragPreview);
    /// Starts the drag session with `payload`, which also carries the payloads `also`, one per type: targets of any of
    /// these types see the one drag.
    fn start_drag_also<P: Clone + Send + Sync + 'static>(
        &mut self,
        payload: P,
        also: Vec<Arc<dyn std::any::Any + Send + Sync>>,
        preview: DragPreview,
    );
    /// The active drag payload of type `P` and the pointer position in this component's egui layer space (8.3), from any
    /// viewport.
    fn dragging<P: Send + Sync + 'static>(&self) -> Option<(Arc<P>, Pos2)>;
    /// Registers `target` over this component's rect for this pass, in `update`. Returns the drop released over it this
    /// pass, when it was the topmost accepting target under the pointer last pass and accepts now.
    fn drop_target<P: Send + Sync + 'static, T: DropTarget<P>>(&mut self, target: &T) -> Option<T::Drop>;
    /// Registers `target` over the whole window of the running viewport for this pass, in `update`; a target over a
    /// component beats it wherever that accepts. By default the payload is read without a pointer position; a target
    /// that requires one waits for it so the target at that position can take the drop.
    fn window_drop_target<P: Send + Sync + 'static, T: WindowDropTarget<P>>(&mut self, target: &T) -> Option<T::Drop>;
    /// The operation of a drop of this component's payload, in the pass after the drop; `None` while dragging, when the
    /// drag was cancelled and when no target took it.
    fn drag_ended<P: Send + Sync + 'static>(&self) -> Option<DropOperation>;
    /// The visible range the scroll part `key` moved to this pass (thumb or end drag, page press, or an AccessKit click
    /// on the track, which pages toward the track's centre).
    fn scroll_moved(&self, key: PartKey) -> Option<Range<f64>>;
}

impl Pointer for CxCore<'_> {
    /// Step 12 leaves the report; a run that stopped before it (a sizing pass) reports no stay, so its output never
    /// repeats an `entered` or `left` the real pass reported.
    fn hover(&self) -> Hover {
        if !self.stated || self.face_mode {
            return Hover::default();
        }
        self.memory.recall::<Hover>(crate::contract::steps::ShownState::hover_slot(self.id))
    }

    fn part(&self, part: impl Into<PartRef>) -> &PartResult {
        let part = part.into();
        match self.results.get(&(part.key, part.index)) {
            Some(result) => result,
            None => {
                // A run that stopped before step 7 has no parts: its core is blank, so every part reads idle.
                debug_assert!(
                    self.face_mode || !self.parted || crate::runtime::measure::active(&self.ctx) || self.results.is_empty() && self.stated,
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

    fn hover_pos(&self, part: impl Into<PartRef>) -> Option<Pos2> {
        let result = self.part_if_declared(part)?;
        let at = result.hover_pos?;
        pointer::follows_part(&self.ctx, self.response.layer_id, result.rect);
        Some(at)
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
        let mut wheel = wheel::take(&self.ctx, self.id)?;
        if let (Some(at), Some(map)) = (wheel.at, areas::layer_map(&self.ctx, self.response.layer_id)) {
            wheel.at = Some(map.to_layer(at));
        }
        Some(wheel)
    }

    fn take_multi_tap(&mut self) -> Option<MultiTap> {
        let map = areas::layer_map(&self.ctx, self.response.layer_id);
        let to_layer = |at: Pos2| map.as_ref().map_or(at, |map| map.to_layer(at));
        let rect = self.response.rect;
        let tap = pointer::take_multi_tap(&self.ctx, self.id, |at| rect.contains(to_layer(at)))?;
        Some(MultiTap::new(tap.fingers, to_layer(tap.at)))
    }

    fn mark_wheel_moved(&mut self) {
        wheel::mark_moved(&self.ctx, self.id);
    }

    fn ease_wheel(&mut self, pan: Vec2) -> Vec2 {
        wheel::ease(&self.ctx, self.id, pan)
    }

    fn stop_wheel_ease(&mut self) {
        wheel::stop_ease(&self.ctx, self.id);
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
        drag::start(&self.ctx, self.id, &self.env, Arc::new(payload), Vec::new(), preview);
    }

    fn start_drag_also<P: Clone + Send + Sync + 'static>(
        &mut self,
        payload: P,
        also: Vec<Arc<dyn std::any::Any + Send + Sync>>,
        preview: DragPreview,
    ) {
        drag::start(&self.ctx, self.id, &self.env, Arc::new(payload), also, preview);
    }

    fn dragging<P: Send + Sync + 'static>(&self) -> Option<(Arc<P>, Pos2)> {
        let (payload, at) = self.dragging_of(TypeId::of::<P>())?;
        Some((payload.downcast::<P>().ok()?, at))
    }

    fn drop_target<P: Send + Sync + 'static, T: DropTarget<P>>(&mut self, target: &T) -> Option<T::Drop> {
        self.drop_target_of(TypeId::of::<P>(), &Typed::new(target))
    }

    fn window_drop_target<P: Send + Sync + 'static, T: WindowDropTarget<P>>(&mut self, target: &T) -> Option<T::Drop> {
        if target.requires_pointer() && self.dragging::<P>().is_none() {
            return None;
        }
        // A target under the pointer in another window takes the drop of a drag that left this one.
        let payload = drag::active::<P>(&self.ctx).filter(|_| !drag::moves_viewport(&self.ctx) && !drag::held_elsewhere(&self.ctx))?;
        let op = drag::current_operation(&self.ctx, craft_behavior::CopyKeys::Alt);
        let drop = target.accept(&payload, op)?;
        let ordinal = drag::next_ordinal(&self.ctx, self.id);
        let reach = drag::TargetReach { window: true, under_modal: target.under_modal() };
        drag::register(&self.ctx, self.id, self.response.layer_id, target.mark(&drop), op, self.env.shared_tokens(), reach);
        drag::take_drop(&self.ctx, self.id, ordinal, op, TypeId::of::<P>()).then_some(drop)
    }

    fn drag_ended<P: Send + Sync + 'static>(&self) -> Option<DropOperation> {
        drag::ended::<P>(&self.ctx, self.id)
    }

    fn scroll_moved(&self, key: PartKey) -> Option<Range<f64>> {
        let moves = self.scroll_moves.iter().find(|moved| moved.key == key)?;
        moves.pressed.clone().or_else(|| moves.clicked.clone().filter(|_| self.intents.access_click(key.into())))
    }
}

impl CxCore<'_> {
    /// The active drag payload whose type is `payload` and the pointer in this component's egui layer space.
    pub(crate) fn dragging_of(&self, payload: TypeId) -> Option<(Arc<dyn Any + Send + Sync>, Pos2)> {
        let (payload, at) = drag::dragging_of(&self.ctx, payload)?;
        Some((payload, areas::layer_map(&self.ctx, self.response.layer_id).map_or(at, |map| map.to_layer(at))))
    }

    /// `Pointer::drop_target` for the payload whose type is `payload_type`, chosen at run time: a collection takes the
    /// payload types its drop kinds name. The target lies over this component's visible shapes on its own layer.
    pub(crate) fn drop_target_of<T: DropTarget<dyn Any + Send + Sync>>(&mut self, payload_type: TypeId, target: &T) -> Option<T::Drop> {
        drag::target_of(&self.ctx, self.id, DropZone::new(self.response.layer_id, self.visible), self.env.shared_tokens(), payload_type, target)
    }
}

impl<C: Component> Pointer for Cx<'_, C> {
    fn hover(&self) -> Hover {
        self.core.hover()
    }

    fn part(&self, part: impl Into<PartRef>) -> &PartResult {
        self.core.part(part)
    }

    fn part_if_declared(&self, part: impl Into<PartRef>) -> Option<&PartResult> {
        self.core.part_if_declared(part)
    }

    fn hover_pos(&self, part: impl Into<PartRef>) -> Option<Pos2> {
        self.core.hover_pos(part)
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

    fn take_multi_tap(&mut self) -> Option<MultiTap> {
        self.core.take_multi_tap()
    }

    fn mark_wheel_moved(&mut self) {
        self.core.mark_wheel_moved();
    }

    fn ease_wheel(&mut self, pan: Vec2) -> Vec2 {
        self.core.ease_wheel(pan)
    }

    fn stop_wheel_ease(&mut self) {
        self.core.stop_wheel_ease();
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

    fn start_drag_also<P: Clone + Send + Sync + 'static>(
        &mut self,
        payload: P,
        also: Vec<Arc<dyn std::any::Any + Send + Sync>>,
        preview: DragPreview,
    ) {
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

impl<C: Component> Cx<'_, C> {
    /// The value a primary-button sweep sets on this toggle. Only siblings in the same layout
    /// participate. A control that uses this must sense both clicks and drags.
    pub fn swept_toggle(&self, on: Option<bool>) -> Option<bool> {
        let group = self.pointer_group?;
        let target = crate::runtime::toggle_sweep::ToggleTarget {
            id: self.id,
            layer: self.response.layer_id,
            visible: self.visible,
            enabled: self.resolved.enabled && !self.resolved.read_only,
            press: self.press().phase,
        };
        crate::runtime::toggle_sweep::value(&self.ctx, target, group, on)
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
