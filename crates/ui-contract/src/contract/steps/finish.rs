//! Step 17, finish: the response's click flags and the recorded size.

use crate::contract::{Component, Cx};
use crate::runtime::commands::{self, FireSource};
use crate::runtime::egui::response;
use crate::runtime::layout_memory;
use craft_behavior::{EditEvent, Mixed};
use egui::{Context, Id, Response, Ui, Vec2};

/// Step 17, finish, before `C::finish`: the click flags of the response are rewritten, so every output built from it
/// carries them, and the bound command of an activation fires, once: with the source `Cx::activate` injected, else
/// as a component activation.
pub(crate) fn before_finish<C: Component>(cx: &mut Cx<'_, C>, activated: bool, injected: Option<FireSource>) {
    let by_key = activated && cx.core.activation.is_some_and(|activation| activation.by_key);
    response::rewrite_clicks(&mut cx.core.response, by_key || cx.core.part_clicked, cx.core.command.is_some());
    // The response reports a change for a `Set` or a `Commit` of the pass that moved the value.
    let changed = cx.events.iter().any(|event| match event {
        EditEvent::Commit { before, after, .. } | EditEvent::Set { before, after, .. } => !matches!(before, Mixed::Same(held) if held == after),
        _ => false,
    });
    if changed {
        response::mark_changed(&mut cx.core.response);
    }
    if activated && let Some(command) = &cx.core.command {
        match injected {
            Some(source) => {
                commands::fire_resolved(&cx.core.ctx, command, source);
            }
            None => commands::fire_bound(&cx.core.ctx, command),
        }
    }
}

/// The sizes a body or collection records at step 17.
pub(crate) fn record_size(ctx: &Context, id: Id, real: Vec2) {
    layout_memory::record(ctx, id, real);
}

/// The response of a component before any pass has created its root widget: inert, for the component's own id.
pub(crate) fn inert_response(ui: &Ui, id: Id) -> Response {
    response::inert(ui, id)
}
