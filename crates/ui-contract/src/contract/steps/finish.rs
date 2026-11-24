//! Step 17, finish: the response's click flags and the recorded size.

use crate::contract::Mount;
use crate::contract::cx::CxCore;
use crate::runtime::commands::{self, FireSource};
use crate::runtime::egui::response;
use crate::runtime::pointer;
use crate::runtime::{layout_memory, windows};
use craft_behavior::{EditEvent, Mixed};
use egui::{Context, Id, Response, Ui, Vec2};

/// Step 17, finish, before `C::finish`: the click flags of the response are rewritten, so every output built from it
/// carries them, and the bound command of an activation fires, once: with the source `Cx::activate` injected, else
/// as a component activation.
/// `changed` is `value_changed` of the pass's events.
pub(crate) fn before_finish(core: &mut CxCore<'_>, activated: bool, injected: Option<FireSource>, changed: bool) {
    let by_key = activated && core.activation.is_some_and(|activation| activation.by_key);
    response::rewrite_clicks(&mut core.response, by_key || core.part_clicked, core.command.is_some() && activated);
    if changed {
        response::mark_changed(&mut core.response);
    }
    if activated && let Some(command) = &core.command {
        match injected {
            Some(source) => {
                commands::fire_resolved(&core.ctx, command, source, pointer::frame(&core.ctx).mods);
            }
            None => {
                let mods = core.activation.map_or_else(|| pointer::frame(&core.ctx).mods, |activation| activation.mods);
                commands::fire_bound(&core.ctx, command, mods);
            }
        }
    }
}

/// Whether a `Set` or a `Commit` of the pass moved the value: the response then reports a change.
pub(crate) fn value_changed<V: PartialEq>(events: &[EditEvent<V>]) -> bool {
    events.iter().any(|event| match event {
        EditEvent::Commit { before, after, .. } | EditEvent::Set { before, after, .. } => !matches!(before, Mixed::Same(held) if held == after),
        _ => false,
    })
}

/// The sizes a body or collection records at step 17.
pub(crate) fn record_size(ctx: &Context, id: Id, real: Vec2) {
    layout_memory::record(ctx, id, real);
}

/// The size the body of container `id`, mounted by `mount`, records at step 17 under the container's id: what it took.
/// A body in an OS window that has not reported its place records nothing, since it laid out in room the OS has not
/// given; a content-sized window opens by this record once its content measured in it (`ViewportHost`).
pub(crate) fn record_body_size(ctx: &Context, mount: &Mount, id: Id, took: Vec2) {
    if matches!(mount, Mount::Viewport { .. }) && windows::unplaced(ctx, id) {
        return;
    }
    record_size(ctx, id, took);
}

/// The response of a component before any pass has created its root widget: inert, for the component's own id.
pub(crate) fn inert_response(ui: &Ui, id: Id) -> Response {
    response::inert(ui, id)
}
