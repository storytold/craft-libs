//! `behavior/pipeline-only`: layers 5 to 7 do not name the behaviours the pipeline runs (spec 20.4).
//!
//! Components read the press (`cx.press()` gives a `PressState`, a `PressPhase` can be matched) but never drive the
//! machine, open focus scopes or run gestures and typing runs themselves. Use items, expressions and types are all checked.

use crate::report::Finding;

use super::Cx;
use super::sites::Kind;

pub const ID: &str = "behavior/pipeline-only";

/// Names of the behaviours the pipeline runs: the press machine and its input, `Gesture`, `TypingRun`.
const BEHAVIOURS: [&str; 6] = ["PressMachine", "PressStep", "PassInput", "count_clicks", "Gesture", "TypingRun"];

/// Functions that open and close a focus scope, written `focus::<name>`.
const SCOPE_FNS: [&str; 2] = ["push_scope", "pop_scope"];

/// The module that holds the press machine.
const PRESS_MODULE: [&str; 2] = ["craft_behavior", "press"];

/// The banned item a resolved path names.
fn banned(path: &[String], in_use: bool) -> Option<String> {
    if let Some(name) = path.iter().find(|s| BEHAVIOURS.contains(&s.as_str())) {
        return Some(name.clone());
    }
    if let Some(w) = path.windows(2).find(|w| w[0] == "focus" && SCOPE_FNS.contains(&w[1].as_str())) {
        return Some(w[1].clone());
    }
    (in_use && path == PRESS_MODULE).then(|| "press".to_owned())
}

pub(super) fn check(cx: &Cx<'_>, out: &mut Vec<Finding>) {
    if !matches!(cx.scope.layer(cx.path), Some(5..=7)) {
        return;
    }
    for site in &cx.sites {
        let Kind::Path { path, in_use, .. } = &site.kind else { continue };
        if let Some(name) = banned(path, *in_use) {
            cx.flag(
                out,
                ID,
                ID,
                site,
                format!("`{}` names `{name}`, which the pipeline runs; components reach it through the contract", path.join("::")),
            );
        }
    }
}
