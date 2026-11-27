# craft-quit

Quit a Craft app the platform's own way, hold a window close while documents are unsaved, and walk
the unsaved-changes question one document at a time. On macOS the quit is
`-[NSApplication terminate:]`; on other platforms `quit()` returns `false` and the app closes its
window as usual. The prompt itself and the saving stay in each app.

## Why macOS needs it

An eframe app on macOS must not leave by closing its window. eframe drops the window while AppKit's
run loop is still running. In the next display cycle AppKit's Touch Bar finder tries to remove a KVO
observer from the view that is gone (`-[_NSTouchBarFinderObservation invalidate]`), which raises an
uncaught Objective-C exception, and the app crashes. `terminate:`, what ⌘Q does in winit's default
menu, ends the process before another display cycle runs.

## Features and targets

| Feature or target | Adds |
| --- | --- |
| (always) | `quit()`, `QUIT_MENU_ID`, `CloseGuard`, `CloseScope`, `CloseAnswer` and `GuardStep` |
| `ui` | `intercept_close(ctx, unsaved)` and `on_menu_event(ctx, id)`, on an egui `Context` |
| `wasm32` | `web::hold_leave(unsaved)`, the browser's own leave prompt |

## `quit()`

On macOS, `quit()` schedules `terminate:` on the main queue and returns `true`. The caller keeps its
window open, and the process ends once the current event has been handled. winit's
`applicationWillTerminate:` then runs eframe's exit handling, so the app saves what it keeps in
`eframe::App::on_exit`. Code after `eframe::run_native` in `main` does not run on macOS.

Call it while the event loop is running (any eframe callback is). Return to the event loop before
blocking: a blocking native dialog opened in the same callback would run the quit while winit is
still handling the event, which panics.

On other platforms `quit()` returns `false`, and the caller closes its window.

## Holding a window close

Call `craft_quit::intercept_close(ctx, unsaved)` each frame in `eframe::App::logic`, with whether any
document has unsaved changes. When the root window was asked to close (the close button, the OS, or
the Quit menu item) and `unsaved` is `true`, it sends `ViewportCommand::CancelClose` and
`ViewportCommand::Focus` and returns `true`. The app then starts a `CloseGuard` for
`CloseScope::Quit`. In every other case it returns `false` and the close goes on. It does nothing in
a child viewport.

`intercept_close` does not call `quit()`. When it lets a close go on, the app calls `quit()` and, when
that returns `true`, sends `CancelClose` so that the window stays open until the process ends:

```rust,ignore
fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    // Called every frame, also while a guard is open, so a second close request is held too.
    if craft_quit::intercept_close(ctx, self.has_unsaved_changes()) {
        if self.guard.is_none() {
            self.guard = Some(craft_quit::CloseGuard::new(craft_quit::CloseScope::Quit, self.modified_documents()));
        }
    } else if ctx.viewport_id() == egui::ViewportId::ROOT
        && ctx.input(|i| i.viewport().close_requested())
        && craft_quit::quit()
    {
        ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
    }
}
```

An app that also holds a close for another reason (a running export, for example) makes that
decision first and skips `quit()` in a frame where it sent `CancelClose` itself.

Keep the check in `logic`, not in `ui`: eframe also calls `logic` while the window is minimized or
hidden, when `ui` does not run. If the window is hidden, show it again before showing the prompt.

## The unsaved-changes question

`CloseGuard<D>` holds the decision for closing documents, closing all, quitting and Revert. It
does not use egui. `D` is the app's document id. `CloseGuard::new(scope, modified)` takes the
documents with unsaved changes in the order they are asked about. An empty list is already finished.

The app shows the question for `current()` (craft-ui's `UnsavedChangesDialog`), passes the answer to
`answer(CloseAnswer)`, or to `answer_all` when "Apply to All" is ticked, and acts on the
`GuardStep` it gets back:

| Step | The app |
| --- | --- |
| `Save(doc)` | Saves `doc` (Save As when it has no file), closes it once the save succeeds, then calls `saved()`. A canceled step such as Save As calls `save_failed()`; a write that failed calls `retry_after_failure()`. |
| `Discard(doc)` | Closes `doc` without saving, then calls `discarded()`. |
| `Ask(doc)` | Shows the question for `doc`. |
| `Cancelled` | Drops the guard. Nothing more closes, and a quit does not happen. |
| `Finished` | Finishes the operation. For `CloseScope::Quit`, it drops the guard and sends the root window `ViewportCommand::Close`; with nothing unsaved, `intercept_close` lets that close go on and the app calls `quit()` as above. |

- For `CloseScope::Revert`, the prompt asks Revert and Cancel, and Revert answers
  `CloseAnswer::DontSave`. On `Discard(doc)` the app reloads `doc` from its file instead of closing
  it, then calls `discarded()`.
- `CloseAnswer::Cancel` ends the whole operation. Documents already closed stay closed.
- `answer_all` applies one answer to every remaining document, Save included. A canceled Save As
  stops the operation.
- `retry_after_failure()` puts the document back at the front, clears the "Apply to All" answer and
  asks about that document again.
- `forget(&doc)` drops a document that closed elsewhere while the guard ran. It returns no step. When
  it removed the last waiting document and no save is in progress, `is_finished()` is `true`: the app then treats the operation
  as `Finished` and does what the `Finished` row above says.
- `remaining()` counts the documents still waiting, the current one included. The prompt shows
  "Apply to All" when it is more than one.

## A Quit menu item that asks first

muda's `PredefinedMenuItem::quit` and winit's default app menu end the process without asking the
app. An app with its own native menu gives its Quit item the id `craft_quit::QUIT_MENU_ID` and
passes its menu events to `craft_quit::on_menu_event(ctx, id)`. For that id, `on_menu_event` sends
the root window `ViewportCommand::Close`, even when called from a child viewport, so
`intercept_close` and the app's question run as they do for the close button. It returns `false` for
other ids and leaves them to the caller. It can be called between frames; egui keeps the command for
the next frame.

craft-menu uses this id for its Quit item. An app without a menu of its own calls
`craft_menu::mac::install_app_menu` once from the eframe app creator; see
[craft-menu](../menu/README.md).

## Browser tabs

A browser tab has no close request an app can answer. On `wasm32`, call
`craft_quit::web::hold_leave(unsaved)` each frame. While the last call passed `true`, the page's
`beforeunload` listener calls `prevent_default`, and the browser may ask the user before leaving.
Browsers decide whether to show the prompt (some skip it when the page has had no user interaction),
and the prompt cannot be customized, so treat it as best effort. The first call adds the listener; a page without a window gets none, and a later call tries again.

## Notes

- No `unsafe`: the macOS calls go through objc2's safe API.
- The macOS dependencies (objc2, objc2-app-kit, dispatch2) are already in eframe apps' lock files.
