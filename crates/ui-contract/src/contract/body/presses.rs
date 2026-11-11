//! What a container's hooks read of this pass's presses: on its surface, on its parts and the move band's double click.

use super::BodyHooks;
use super::window::MOVE_BAND;
use crate::contract::cx::BodyCx;
use crate::contract::{Container, Cx};
use crate::runtime::egui::areas;
use crate::runtime::memory::sub;
use crate::runtime::pointer;
use craft_behavior::{Cursor, CursorPriority, PartRef, PressState};

/// Whether the second press of a double click went down on the container's move band in this pass.
#[derive(Clone, Default)]
struct BandDouble(bool);

/// Whether the primary press of this pass went down on the container's own surface (`Sense::SURFACE`).
#[derive(Clone, Default)]
struct SurfacePress(bool);

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
    /// `Sense::SURFACE`: inside its rect, on its layer, where none of its children or parts takes the press. The
    /// container decides what it means (a dock group focuses its panel, as Photoshop does); it is no click and moves
    /// no focus by itself.
    pub fn surface_pressed(&self) -> bool {
        self.memory.recall::<SurfacePress>(sub(self.id, "surface_press")).0
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
    /// (`BodyCx::surface_pressed`): no shape of last pass under the press takes it, and the press lies in the
    /// container's rect on its layer.
    pub(super) fn surface_press<C: Container>(&self, sensed: bool, cx: &mut Cx<'_, C>) {
        let ctx = cx.core.ctx.clone();
        let pressed = sensed && {
            let pass = pointer::pass_pointer(&ctx);
            pass.frame.as_ref().and_then(|frame| frame.press).filter(|press| press.button == craft_behavior::PointerButton::Primary).is_some_and(
                |press| {
                    let layer = cx.core.response.layer_id;
                    let at = areas::layer_map(&ctx, layer).map_or(press.origin, |map| map.to_layer(press.origin));
                    areas::layer_at(&ctx, press.origin) == Some(layer)
                        && cx.core.rect.contains(at)
                        && pointer::press_winner(&ctx, press.origin, pass.press_targets).is_none()
                },
            )
        };
        cx.core.memory.shown::<SurfacePress>(sub(sub(cx.core.id, "body"), "surface_press")).0 = pressed;
    }

    /// Records for the container's header hooks whether the second press of a double click went down on the move
    /// band in this pass (`BodyCx::band_double_pressed`).
    pub(super) fn band_double<C: Container>(&self, cx: &mut Cx<'_, C>) {
        let Some(band) = cx.core.results.get(&(MOVE_BAND, None)) else { return };
        let double = band.press.double_pressed();
        cx.core.memory.shown::<BandDouble>(sub(sub(cx.core.id, "body"), "band_double")).0 = double;
    }
}
