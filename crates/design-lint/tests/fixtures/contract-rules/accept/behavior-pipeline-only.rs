use craft_behavior::press::{PointerButton, PointerSample, PressCancel, PressPhase, PressStart, PressState};
use craft_behavior::{CancelRule, ScopeKind, Sense, ValueDrag};
use craft_ui::contract::PressPhase as Phase;
use crate::contract::Cx;

fn pressed(cx: &Cx<'_, Chip>) -> bool {
    let state: &PressState = cx.press();
    matches!(state.phase(), Some(PressPhase::Held))
}

fn stack(scopes: &mut Stack) {
    scopes.push_scope();
    scopes.pop_scope();
    push_scope_label(1);
}

fn push_scope_label(n: u32) {}
