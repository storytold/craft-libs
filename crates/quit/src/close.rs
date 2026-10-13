//! Route an egui window close through the platform quit.

use egui::{Context, ViewportCommand, ViewportId};

/// Call once per frame at the end of `eframe::App::logic`, after the app's own close decision.
/// Move that decision out of `ui` as well: `logic` runs first, and `ui` does not run while the
/// window is minimized or hidden. See the crate example for keeping a save prompt in app state.
/// When the root window was asked
/// to close (close button, the app's own `ViewportCommand::Close`, the OS) and this platform has
/// its own quit (see [`crate::quit`]), cancels the close and quits that way instead. The window
/// stays for the quit to close.
///
/// Returns whether it took the close over. `false` means there was no close request, the app had
/// already cancelled it this frame, or the platform has no quit of its own, and eframe goes on as
/// it would without this call.
pub fn on_close_request(ctx: &Context) -> bool {
    take_over_close(ctx, crate::quit)
}

/// The id of a Quit menu item that asks the app first.
///
/// An app that builds its own native app menu (muda, any version) gives its Quit item this id,
/// instead of using muda's `PredefinedMenuItem::quit`, which ends the process without asking, and
/// passes its menu events to [`on_menu_event`]. Apps without their own menu use
/// [`crate::install_app_menu`], which does both.
pub const QUIT_MENU_ID: &str = "craft-quit.quit";

/// Handle a native menu event by the id of the item chosen. For [`QUIT_MENU_ID`], asks the root
/// window to close (`ViewportCommand::Close`), so the app's unsaved-changes check runs as it does
/// for the close button and [`on_close_request`] then quits. Returns whether the item was Quit;
/// other items are left to the caller. Safe to call from a menu-event handler between frames:
/// egui keeps the command for the next frame.
pub fn on_menu_event(ctx: &Context, id: &str) -> bool {
    if id != QUIT_MENU_ID {
        return false;
    }
    ctx.send_viewport_cmd_to(ViewportId::ROOT, ViewportCommand::Close);
    true
}

/// [`on_close_request`] with the platform quit passed in. `quit` returns whether it quits.
fn take_over_close(ctx: &Context, quit: impl FnOnce() -> bool) -> bool {
    if ctx.viewport_id() != ViewportId::ROOT || !ctx.input(|i| i.viewport().close_requested()) {
        return false;
    }
    // The app's unsaved-changes prompt (or anything else) is holding the window open.
    if ctx.viewport(|v| v.commands.contains(&ViewportCommand::CancelClose)) {
        return false;
    }
    if !quit() {
        return false;
    }
    ctx.send_viewport_cmd(ViewportCommand::CancelClose);
    true
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use egui::{RawInput, ViewportEvent};

    use super::*;

    /// One frame of the root window, with its close button pressed or not. `app` runs before the
    /// take-over, like the app's own unsaved-changes check. Returns whether the close was taken
    /// over, how often the quit ran and the frame's commands for the root window.
    fn frame(close: bool, quits: bool, app: impl Fn(&Context)) -> (bool, u32, Vec<ViewportCommand>) {
        let ctx = Context::default();
        let mut input = RawInput::default();
        if close {
            input.viewports.entry(ViewportId::ROOT).or_default().events.push(ViewportEvent::Close);
        }
        let (taken, quit_calls) = (Cell::new(false), Cell::new(0));
        let mut out = ctx.run_ui(input, |ui| {
            app(ui.ctx());
            taken.set(take_over_close(ui.ctx(), || {
                quit_calls.set(quit_calls.get() + 1);
                quits
            }));
        });
        out.textures_delta.clear(); // not painted
        let commands = out.viewport_output.remove(&ViewportId::ROOT).map(|o| o.commands).unwrap_or_default();
        (taken.get(), quit_calls.get(), commands)
    }

    fn cancels(commands: &[ViewportCommand]) -> bool {
        commands.contains(&ViewportCommand::CancelClose)
    }

    #[test]
    fn a_close_request_quits_and_keeps_the_window() {
        let (taken, quit_calls, commands) = frame(true, true, |_| {});
        assert!(taken);
        assert_eq!(quit_calls, 1);
        assert!(cancels(&commands), "the window stays for the quit to close");
    }

    #[test]
    fn nothing_happens_without_a_close_request() {
        let (taken, quit_calls, commands) = frame(false, true, |_| {});
        assert!(!taken);
        assert_eq!(quit_calls, 0);
        assert!(!cancels(&commands));
    }

    #[test]
    fn a_close_the_app_cancelled_does_not_quit() {
        let (taken, quit_calls, _) = frame(true, true, |ctx| ctx.send_viewport_cmd(ViewportCommand::CancelClose));
        assert!(!taken);
        assert_eq!(quit_calls, 0, "unsaved work asks first");
    }

    /// The root window's commands from one frame run after `before`, which runs between frames
    /// like a native menu-event handler.
    fn commands_after(before: impl FnOnce(&Context)) -> Vec<ViewportCommand> {
        let ctx = Context::default();
        before(&ctx);
        let mut out = ctx.run_ui(RawInput::default(), |_| {});
        out.textures_delta.clear(); // not painted
        out.viewport_output.remove(&ViewportId::ROOT).map(|o| o.commands).unwrap_or_default()
    }

    #[test]
    fn the_quit_menu_item_asks_the_window_to_close() {
        let commands = commands_after(|ctx| assert!(on_menu_event(ctx, QUIT_MENU_ID)));
        assert!(commands.contains(&ViewportCommand::Close), "the app's close handling decides");
    }

    #[test]
    fn quit_from_a_child_viewport_still_asks_the_root_to_close() {
        let ctx = Context::default();
        let child = ViewportId::from_hash_of("document-child");
        let mut input = RawInput { viewport_id: child, ..RawInput::default() };
        input.viewports.entry(child).or_default().parent = Some(ViewportId::ROOT);
        let mut out = ctx.run_ui(input, |ui| {
            assert_eq!(ui.ctx().viewport_id(), child);
            assert!(on_menu_event(ui.ctx(), QUIT_MENU_ID));
        });
        out.textures_delta.clear(); // not painted
        let root_commands = out.viewport_output.get(&ViewportId::ROOT).map(|o| o.commands.as_slice()).unwrap_or_default();
        let child_commands = out.viewport_output.get(&child).map(|o| o.commands.as_slice()).unwrap_or_default();
        assert!(root_commands.contains(&ViewportCommand::Close), "root unsaved handling decides whether the app exits");
        assert!(!child_commands.contains(&ViewportCommand::Close), "Quit must not just close the current child");
    }

    #[test]
    fn other_menu_items_are_left_to_the_app() {
        let commands = commands_after(|ctx| assert!(!on_menu_event(ctx, "edit.copy")));
        assert!(!commands.contains(&ViewportCommand::Close));
    }

    #[test]
    fn without_a_platform_quit_the_window_closes_as_usual() {
        let (taken, quit_calls, commands) = frame(true, false, |_| {});
        assert!(!taken);
        assert_eq!(quit_calls, 1);
        assert!(!cancels(&commands), "eframe closes the window");
    }

    /// eframe uses only the logic callback while minimized; no UI callback runs here.
    fn minimized_logic_frame(unsaved: bool) -> (bool, u32, bool, Vec<ViewportCommand>) {
        let ctx = Context::default();
        let mut input = RawInput::default();
        let root = input.viewports.entry(ViewportId::ROOT).or_default();
        root.minimized = Some(true);
        root.events.push(ViewportEvent::Close);
        let (taken, quit_calls, show_save_prompt) = (Cell::new(false), Cell::new(0), Cell::new(false));
        let mut out = ctx.run_logic(&input, |ctx| {
            if ctx.input(|i| i.viewport().close_requested()) && unsaved {
                ctx.send_viewport_cmd(ViewportCommand::CancelClose);
                show_save_prompt.set(true);
            }
            taken.set(take_over_close(ctx, || {
                quit_calls.set(quit_calls.get() + 1);
                true
            }));
        });
        let commands = out.viewport_commands.remove(&ViewportId::ROOT).unwrap_or_default();
        (taken.get(), quit_calls.get(), show_save_prompt.get(), commands)
    }

    #[test]
    fn minimized_unsaved_close_waits_for_the_prompt_without_running_ui() {
        let (taken, quit_calls, show_save_prompt, commands) = minimized_logic_frame(true);
        assert!(!taken);
        assert_eq!(quit_calls, 0);
        assert!(show_save_prompt, "UI can render the decision stored by logic");
        assert!(cancels(&commands));
    }

    #[test]
    fn minimized_clean_close_takes_over_without_running_ui() {
        let (taken, quit_calls, show_save_prompt, commands) = minimized_logic_frame(false);
        assert!(taken);
        assert_eq!(quit_calls, 1);
        assert!(!show_save_prompt);
        assert!(cancels(&commands), "eframe must not tear down the window before platform quit");
    }
}
