//! What a container's hooks read of this pass's presses: on its surface, on its parts and the move band's double click.

use super::BodyHooks;
use super::window::MOVE_BAND;
use crate::contract::cx::BodyCx;
use crate::contract::cx::CxCore;
use crate::runtime::egui::areas;
use crate::runtime::memory::sub;
use crate::runtime::pointer;
use craft_behavior::{Cursor, CursorPriority, PartRef, PointerButton, PressState};
use egui::{Pos2, Ui};

/// Whether the second press of a double click went down on the container's move band in this pass.
#[derive(Clone, Default)]
struct BandDouble(bool);

/// Whether the primary press of this pass went down on the container's own surface (`Sense::SURFACE`), whether it
/// is the second of a double click, and whether this pass's release completes a click that began there.
#[derive(Clone, Default)]
struct SurfacePress {
    pressed: bool,
    double: bool,
    clicked: bool,
}

impl BodyCx<'_> {
    /// Whether the second press of a double click went down on the container's move band in this pass. The body step
    /// only reports it; the container decides what it means (a title bar maximizes the window, a floating frame
    /// minimizes it). A band double click that the container never reads does nothing. A measuring run sees none: the
    /// record it would read is last pass's, and a container that latches it would report the click twice.
    pub fn band_double_pressed(&self) -> bool {
        !self.memory.is_peek() && self.memory.recall::<BandDouble>(sub(self.id, "band_double")).0
    }

    /// The press state of the container's own part `part` in this pass, the same in a measuring run and in the real
    /// run: the container's press machine steps before the measuring run, from the parts it declared last pass (a
    /// part declared first this pass reads idle). Idle for a container that has not stepped.
    pub fn part_press(&self, part: impl Into<PartRef>) -> PressState {
        crate::primitives::sensing::part_press(self.memory.ctx(), self.resolved.id, part.into()).press
    }

    /// Whether the primary press of this pass went down on the container's own surface, for a root that senses
    /// `Sense::SURFACE`: inside its rect, on its layer, where none of its children or parts and no app's own egui widget
    /// in its body takes the press. The container decides what it means (a dock group focuses its panel, as Photoshop
    /// does); it is no click and moves no focus by itself.
    pub fn surface_pressed(&self) -> bool {
        self.memory.recall::<SurfacePress>(sub(self.id, "surface_press")).pressed
    }

    /// Whether the primary press of this pass went down in the body's rect, on the layer `ui` (a `Ui` of this body)
    /// paints on, whatever takes it: a child, a part, an app's own egui widget or the surface itself. A press on a
    /// popup or a window above the body is not in it. The container decides what it means (a dock group whose body is
    /// the app's own egui code names its panel as the one pressed into); it moves no focus by itself. A measuring run
    /// sees none, for the reason [`band_double_pressed`](Self::band_double_pressed) gives.
    pub fn pressed_within(&self, ui: &Ui) -> bool {
        let ctx = self.memory.ctx();
        !self.memory.is_peek() && pointer::primary_press_in(ctx, ui.layer_id(), self.rect).is_some()
    }

    /// Whether the press of [`surface_pressed`](Self::surface_pressed) is the second press of a double click: no child
    /// or part takes it and its click count is two or more. One own-surface signal for every container (an empty
    /// workspace opens a file on it). A measuring run sees none, for the reason
    /// [`band_double_pressed`](Self::band_double_pressed) gives.
    pub fn surface_double_pressed(&self) -> bool {
        !self.memory.is_peek() && self.memory.recall::<SurfacePress>(sub(self.id, "surface_press")).double
    }

    /// Whether this pass's primary release ends a press that went down on the container's own surface, moved less
    /// than `PointerFrame::slop` and was released inside. A drag, a cancel, a release elsewhere and a measuring run
    /// give false. It takes no focus and is no Tab stop.
    pub fn surface_clicked(&self) -> bool {
        !self.memory.is_peek() && self.memory.recall::<SurfacePress>(sub(self.id, "surface_press")).clicked
    }

    /// Requests `cursor` at `priority` for this pass, in the cursor service the parts' and components' requests go
    /// to: the highest priority of the pass wins, the later request on a tie. A body that learns something only after
    /// its children ran (a drag its panes report as refused) sets the cursor in the same pass. A measuring run
    /// requests nothing.
    pub fn cursor(&mut self, cursor: impl Into<Cursor>, priority: CursorPriority) {
        if !self.memory.is_peek() {
            crate::runtime::cursor::request(self.memory.ctx(), cursor.into(), priority);
        }
    }
}

impl BodyHooks<'_> {
    /// Records for the container's hooks whether this pass's primary press went down on its own surface
    /// (`BodyCx::surface_pressed`): no shape of last pass under the press takes it, nor a component's text widget
    /// egui gives it to (a focused field's), nor an app's own egui widget (a raw text field or canvas in the body), and
    /// the press lies in the container's rect on its layer; and whether that press is the second of a double click
    /// (`BodyCx::surface_double_pressed`), read from the same press's click count; and whether this pass's primary
    /// release ends a press that went down there (`BodyCx::surface_clicked`). The press origin waits in the press slot
    /// until the release; Escape and a blur clear it, and so does any pass with no button held.
    pub(super) fn surface_press(&self, sensed: bool, core: &mut CxCore<'_>) {
        let ctx = core.ctx.clone();
        let pass = pointer::pass_pointer(&ctx);
        let (layer, rect) = (core.response.layer_id, core.rect);
        let inside = |pos: Pos2| areas::lies_in(&ctx, layer, rect, pos);
        let surface = if sensed {
            pointer::primary_press_in(&ctx, layer, rect).filter(|press| {
                pointer::press_winner(&ctx, press.origin, pass.press_targets).is_none()
                    && !pointer::text_takes_press(&ctx, pass.press_targets)
                    && ![pass.press_targets.click, pass.press_targets.drag].into_iter().flatten().any(|widget| pointer::foreign_target(&ctx, widget))
            })
        } else {
            None
        };
        let state_id = sub(sub(core.id, "body"), "surface_press");
        let mut clicked = false;
        if sensed {
            let origin = core.memory.press::<Pos2>(state_id);
            if let Some(press) = surface {
                *origin = Some(press.origin);
            }
            match pass.frame.as_ref() {
                Some(frame) if frame.release == Some(PointerButton::Primary) => {
                    clicked = origin.take().is_some_and(|from| frame.pos.is_some_and(|pos| inside(pos) && (pos - from).length() < frame.slop));
                }
                _ if surface.is_none() && (!pass.primary_down || pointer::cancel_of_pass(&ctx).is_some()) => *origin = None,
                _ => {}
            }
            if origin.is_some() && !core.memory.is_peek() {
                pointer::hold_surface_press(&ctx);
            }
        }
        let shown = core.memory.shown::<SurfacePress>(state_id);
        shown.pressed = surface.is_some();
        shown.double = surface.is_some_and(|press| press.clicks >= 2);
        shown.clicked = clicked;
    }

    /// Records for the container's header hooks whether the second press of a double click went down on the move
    /// band in this pass (`BodyCx::band_double_pressed`).
    pub(super) fn band_double(&self, core: &mut CxCore<'_>) {
        let Some(band) = core.results.get(&(MOVE_BAND, None)) else { return };
        let double = band.press.double_pressed();
        core.memory.shown::<BandDouble>(sub(sub(core.id, "body"), "band_double")).0 = double;
    }
}
