//! Bypass detection (24.2): the pass-end walk that finds what a component made without the runtime.
//!
//! Every interactive egui widget of the latest pass must be one a component registered (a root or one of its
//! widgets, a focus stop's root) or one the runtime noted. Every AccessKit node that declares an action must
//! come from the runtime's node writer. A raw `ui.interact`, `ui.button` or `ui.add(Slider)` in a component
//! fails here, with the id of the offender.

use crate::runtime::egui::access::Declared;
use crate::runtime::egui::widgets;
use crate::runtime::focus;
use crate::runtime::pointer;
use crate::testing::Fixture;
use egui::Id;
use std::collections::HashSet;

/// The actions that make a node something a user operates.
const OPERATED: [Declared; 5] = [Declared::Click, Declared::Focus, Declared::Increment, Declared::SetValue, Declared::Expand];

/// What the latest frame's widgets and nodes show of code that bypassed the runtime; empty when nothing did. The
/// suite's shows case and access case fail on a non-empty list. The walk reads the root viewport.
pub fn findings(fixture: &Fixture) -> Vec<String> {
    let ctx = fixture.ctx();
    let (noted_widgets, noted_nodes) = widgets::noted_ids(ctx);
    let mut registered: HashSet<Id> = pointer::registered_widgets(ctx);
    registered.extend(noted_widgets);
    focus::with_viewport(ctx, egui::ViewportId::ROOT, |state| {
        registered.extend(state.stops.iter().chain(&state.last_stops).map(|rec| rec.stop.root));
    });
    let mut found: Vec<String> = widgets::interactive(ctx)
        .into_iter()
        .filter(|id| !registered.contains(id))
        .map(|id| format!("egui widget {id:?} is interactive and no component registered it"))
        .collect();
    let tree = fixture.tree();
    for key in tree.keys() {
        let operated = OPERATED.iter().any(|action| tree.declares_key(key, *action));
        if operated && !noted_nodes.contains(&key.raw()) {
            found.push(format!("AccessKit node {key:?} declares an action and the runtime did not write it"));
        }
    }
    found.sort();
    found
}
