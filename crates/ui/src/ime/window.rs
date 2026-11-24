//! The IME descriptor of a native floating window, published by docking once per pass.
//!
//! Docking draws each native window's panel bodies in that window's own egui pass. A composer
//! drawn there records its descriptor for the pass instead of writing it into the output, and
//! docking writes what was recorded after the window's last body. So a body drawn after the
//! editor's cannot clear the descriptor: egui-winit turns the IME off in a pass without one,
//! which ends the composition.

use egui::{Context, Id, ViewportId, output::IMEOutput};

/// The descriptor the composers of one native-window pass recorded.
#[derive(Clone)]
struct Recorded {
    pass: u64,
    ime: Option<IMEOutput>,
}

fn key(viewport: ViewportId) -> Id {
    Id::unique(("craft_ui::ime::window", viewport))
}

/// The recording for the current pass of the current viewport, if docking began one.
fn current(ctx: &Context) -> Option<Recorded> {
    let (pass, key) = (ctx.cumulative_pass_nr(), key(ctx.viewport_id()));
    ctx.data(|d| d.get_temp::<Recorded>(key)).filter(|r| r.pass == pass)
}

fn store(ctx: &Context, recorded: Recorded) {
    let key = key(ctx.viewport_id());
    ctx.data_mut(|d| d.insert_temp(key, recorded));
}

/// Docking starts drawing a native window's bodies in this pass of the current viewport.
#[cfg_attr(not(feature = "native-windows"), allow(dead_code))]
pub(crate) fn begin_window(ctx: &Context) {
    let (recorded, key) = (Recorded { pass: ctx.cumulative_pass_nr(), ime: None }, key(ctx.viewport_id()));
    ctx.data_mut(|d| d.insert_temp(key, recorded));
}

/// Docking has drawn the window's last body: the recorded descriptor goes out, unless something
/// wrote one directly in this pass (a focused egui `TextEdit`, an app's own editor).
#[cfg_attr(not(feature = "native-windows"), allow(dead_code))]
pub(crate) fn finish_window(ctx: &Context) {
    let (recorded, key) = (current(ctx), key(ctx.viewport_id()));
    ctx.data_mut(|d| d.remove::<Recorded>(key));
    if let Some(ime) = recorded.and_then(|r| r.ime) {
        ctx.output_mut(|o| {
            o.ime.get_or_insert(ime);
        });
    }
}

/// Whether composers record their descriptor in this pass rather than write it.
pub(super) fn recording(ctx: &Context) -> bool {
    current(ctx).is_some()
}

/// Record `ime` for the pass. A later record in the same pass keeps an earlier interrupt.
pub(super) fn record(ctx: &Context, mut ime: IMEOutput) {
    let Some(mut recorded) = current(ctx) else { return };
    ime.should_interrupt_composition |= recorded.ime.as_ref().is_some_and(|ime| ime.should_interrupt_composition);
    recorded.ime = Some(ime);
    store(ctx, recorded);
}

/// Drop the pass's descriptor if `owner` recorded it.
pub(super) fn withdraw(ctx: &Context, owner: Id) {
    let Some(mut recorded) = current(ctx) else { return };
    if recorded.ime.as_ref().is_some_and(|ime| ime.owner == owner) {
        recorded.ime = None;
        store(ctx, recorded);
    }
}
