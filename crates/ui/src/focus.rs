//! Window focus as pointer gestures read it.
//!
//! A drag or resize ends when its window loses focus, not while the window merely reports no
//! focus. egui-winit starts `RawInput::focused` false and only a winit `Focused` event sets it
//! (on macOS to `has_focus()` at the time the event is read), so a window can take pointer
//! input for a long time while egui reports it unfocused, at first launch until the user
//! switches to another application and back. Gestures in such a window still run.

use egui::{Context, Id};

#[derive(Clone, Copy)]
struct Seen {
    pass: u64,
    before: bool,
    now: bool,
}

/// Whether `now` is false and was true on the previous pass, as recorded under `key`. With no
/// record from the previous pass there is no transition, so a gesture asks on every pass it
/// runs. Asking again in the same pass gives the same answer.
pub(crate) fn turned_false(ctx: &Context, key: Id, now: bool) -> bool {
    let pass = ctx.cumulative_pass_nr();
    ctx.data_mut(|d| {
        let seen = match d.get_temp::<Seen>(key) {
            Some(seen) if seen.pass == pass => seen,
            Some(seen) if seen.pass + 1 == pass => Seen { pass, before: seen.now, now },
            _ => Seen { pass, before: now, now },
        };
        d.insert_temp(key, seen);
        seen.before && !seen.now
    })
}

/// Whether the window being drawn lost focus this pass: focused on the previous pass, not now.
pub(crate) fn lost(ctx: &Context) -> bool {
    let focused = ctx.input(|i| i.focused);
    turned_false(ctx, Id::unique("craft-ui-window-focus").with(ctx.viewport_id()), focused)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::RawInput;

    fn pass(ctx: &Context, focused: bool) -> bool {
        let mut lost_now = false;
        let mut out = ctx.run_ui(RawInput { focused, ..Default::default() }, |ui| {
            lost_now = lost(ui.ctx());
            assert_eq!(lost(ui.ctx()), lost_now, "asking twice in a pass");
        });
        out.textures_delta.clear();
        lost_now
    }

    #[test]
    fn only_a_focused_to_unfocused_pass_is_a_loss() {
        let ctx = Context::default();
        assert!(!pass(&ctx, false), "a window that never had focus has not lost it");
        assert!(!pass(&ctx, false));
        assert!(!pass(&ctx, true));
        assert!(pass(&ctx, false));
        assert!(!pass(&ctx, false), "the loss is one pass");
    }
}
