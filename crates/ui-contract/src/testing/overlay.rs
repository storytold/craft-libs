//! The layer stack's test hooks: what a window's stack judges presses by, for a trial's failure report.

use crate::runtime::overlay::OpenEntry;
use egui::{Context, Id, Pos2, Rect};

/// What the layer stack of the running viewport judges the presses of this pass by.
#[derive(Clone, Debug, Default)]
pub struct PressJudgement {
    /// The screen rects a press may land in without closing the top menu family, as the last pass left them: the
    /// family, its anchors and its peers, without the layer that hosts them. `None` when the top layer does not absorb
    /// outside presses.
    pub zone: Option<Vec<Rect>>,
    /// The outside press the input hook recorded for this pass, which closes the layers that absorb outside presses.
    pub absorbed: Option<Pos2>,
    /// The layers the last pass showed in this viewport, parent first: a layer shown in a popup window of its own has
    /// the entry that stands for it in its opener's stack here.
    pub layers: Vec<OpenEntry>,
}

/// What the running viewport's layer stack judges this pass's presses by. Read it in the pass (a panel's body): it
/// notes no tracked input and changes nothing.
pub fn press_judgement(ctx: &Context) -> PressJudgement {
    let (zone, absorbed, layers) = crate::runtime::overlay::press_judgement(ctx);
    PressJudgement { zone, absorbed, layers }
}

/// Where the pointer was when the layer `id` first appeared, as the layer remembers it while shown (what
/// `Anchor::Pointer` hangs it from); `None` when it remembers none. Opens no memory stage.
pub fn layer_pointer_at(ctx: &Context, id: Id) -> Option<Pos2> {
    crate::contract::overlay::remembered_pointer(ctx, id)
}
