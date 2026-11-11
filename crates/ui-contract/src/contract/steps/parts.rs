//! Step 7, parts: what the step gathers and its checks.

use crate::contract::cx::Decls;
use crate::runtime::inspect;
use craft_behavior::{PartKey, PartRef, Roving, Sense};
use egui::Context;

/// What step 7 gathers.
pub(crate) struct PartsStep {
    pub decls: Decls,
    pub rovings: Vec<Roving>,
    pub scrolls: Vec<crate::contract::cx::ScrollDecl>,
    pub text: crate::contract::text::TextStep,
}

/// Whether step 7 declared no parts, no roving group, no scroll part and no text or editor part.
pub(crate) fn declares_none(parts: &PartsStep) -> bool {
    parts.decls.is_empty() && parts.rovings.is_empty() && parts.scrolls.is_empty() && parts.text.decl.is_none() && parts.text.editor.is_none()
}

/// Step 7's finding (22.4): a part that senses `CLICK` or `DRAG` is focusable, declares its keyboard path, or names
/// the focusable part of the component that carries it.
pub(crate) fn check_parts(ctx: &Context, component: &'static str, parts: &PartsStep) {
    let focusable = |key: PartKey| parts.decls.iter().any(|(decl, extras)| decl.key == key && extras.focusable);
    for (decl, extras) in &parts.decls {
        let has_path = extras.focusable || extras.pointer_only.is_some() || extras.keyboard_via.is_some_and(focusable);
        if decl.sense.intersects(Sense::CLICK | Sense::DRAG) && !has_path {
            inspect::finding(
                ctx,
                inspect::Finding {
                    rule: "pointer-part-without-keyboard-path",
                    component,
                    subject: format!("{:?}", PartRef { key: decl.key, index: decl.index }),
                },
            );
        }
    }
}
