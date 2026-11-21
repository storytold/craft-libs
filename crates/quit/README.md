# craft-quit

Quit a Craft app the platform's own way, and route an egui window close through that quit. On macOS
the quit is `-[NSApplication terminate:]`; on other platforms the crate does nothing and the window
closes as usual. Unsaved-document decisions and save prompts stay in each app.

## Why macOS needs it

An eframe app on macOS must not leave by closing its window. eframe drops the window while AppKit's
run loop is still running. In the next display cycle AppKit's Touch Bar finder tries to remove a KVO
observer from the view that is gone (`-[_NSTouchBarFinderObservation invalidate]`), which raises an
uncaught Objective-C exception, and the app crashes. `terminate:`, what ⌘Q does in winit's default
menu, ends the process before another display cycle runs.

## Features

| Feature | Adds |
| --- | --- |
| (none) | `quit()` |
| `egui` | `on_close_request`, `on_menu_event` and `QUIT_MENU_ID` |
| `menu` | `egui`, plus the deprecated `install_app_menu` (use `craft_menu::mac::install_app_menu`) |

## Use

Call `craft_quit::on_close_request(ctx)` once per frame, last in `eframe::App::logic`, after the
app's own close decision:

```rust,ignore
fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    if ctx.input(|i| i.viewport().close_requested())
        && self.has_unsaved_changes()
        && !self.discard_confirmed
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        self.show_save_prompt = true;
    }
    craft_quit::on_close_request(ctx); // last, so it sees the app's `CancelClose`
}

fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
    // ... the app's UI, including the prompt when show_save_prompt is true ...
}
```

When the root window was asked to close (the close button, the app's own `ViewportCommand::Close`,
the OS) and the app did not send `CancelClose` in the same frame, `on_close_request` cancels the
close and calls `quit()`. It returns whether it took the close over. It does nothing in a child
viewport, and it does nothing on platforms where `quit()` returns `false`.

Rules for the app:

- Keep the unsaved-changes check in `logic`, not in `ui`. `logic` runs before `ui`, so a check in
  `ui` would come after the quit. eframe also calls `logic` while the window is minimized or hidden,
  when `ui` does not run.
- Keep the prompt's state in the app and draw the prompt in `ui`. Send `ViewportCommand::Close`
  only after Save has succeeded and cleared the dirty state, or after Don't Save has set the
  confirmation flag. Cancel, a canceled file picker and a failed save keep the app open.
- Reset the prompt and confirmation state when the decision is canceled or a new document opens.
- Let an asynchronous save finish before sending Close.
- If the window is hidden, show it again before showing the prompt.
- Save what the app keeps in `eframe::App::on_exit`. winit's `applicationWillTerminate:` runs
  eframe's exit handling before the process ends. Code after `eframe::run_native` in `main` does
  not run on macOS; move it to `on_exit`.

## `quit()`

On macOS, `quit()` schedules `terminate:` on the main queue and returns `true`. The caller keeps its
window open, and the process ends once the current event has been handled. Call it while the event
loop is running (any eframe callback is). Return to the event loop before blocking: a blocking
native dialog opened in the same callback would run the quit while winit is still handling the
event, which panics.

On other platforms `quit()` returns `false`.

## A Quit menu item that asks first

muda's `PredefinedMenuItem::quit` and winit's default app menu end the process without asking the
app. An app with its own native menu gives its Quit item the id `craft_quit::QUIT_MENU_ID` and
passes its menu events to `craft_quit::on_menu_event(ctx, id)`. For that id, `on_menu_event` sends
the root window `ViewportCommand::Close`, so the app's unsaved-changes check runs as it does for the
close button, and `on_close_request` then quits. It returns `false` for other ids and leaves them
to the caller. It can be called between frames; egui keeps the command for the next frame.

An app without a menu of its own uses `craft_menu::mac::install_app_menu`, which builds a minimal
app menu with that Quit item. `craft_menu::mac` exists only on macOS, so the call needs
`#[cfg(target_os = "macos")]`; other platforms need no call. Call it once, from the eframe app
creator: winit builds its default menu before that and replaces a menu installed earlier. The call
returns a `Result`. On error winit's menu stays, and ⌘Q quits without the unsaved-changes prompt;
the app decides whether that is fatal, and `?` passes the error out of the app creator:

```rust,ignore
#[cfg(target_os = "macos")]
craft_menu::mac::install_app_menu(&cc.egui_ctx, "WordCraft")?;
```

`craft_quit::install_app_menu` is deprecated in its favor.

## Notes

- No `unsafe`: the macOS calls go through objc2's safe API.
- The macOS dependencies (objc2, objc2-app-kit, dispatch2) are already in eframe apps' lock files.
