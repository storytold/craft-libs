craft-window
============

Decide who draws the window frame — the desktop or the app — so every Craft app shows the same
titlebar and borders on a given desktop, make the app's own top strip act as the title bar
when it draws one, and keep the window list, cascade placement and Window menu for apps with one
window per document.

## Why

egui apps either keep the desktop's window frame (titlebar, borders, window buttons) or turn it off
and draw their own. Across the suite this had drifted:

- some apps always drew their own frame on Linux (so KDE's titlebar was replaced by the app's),
- some always kept the desktop's,
- some probed the compositor and only drew their own on GNOME.

So two Craft apps open side by side on KDE could wear different titlebars. This crate makes the
decision in one place, matching each desktop.

## The rule

| Platform | Frame |
| --- | --- |
| macOS, Windows | desktop (the OS) |
| Linux/BSD on X11 | desktop (the window manager) |
| Linux/BSD on Wayland **with** `zxdg_decoration_manager_v1` (KDE, Sway, Hyprland, COSMIC) | desktop (the compositor) |
| Linux/BSD on Wayland **without** it (GNOME's Mutter) | the app |

When the app draws the frame, the decision carries the window-button `Layout` the desktop names
(GNOME's `button-layout`), so the app's header bar puts minimize/maximize/close where that desktop
puts them. This crate decides *whether* and *where*; drawing the buttons stays in each app, whose
title bars differ.

### Override

`<PREFIX>_TITLEBAR` forces the choice. With the default prefix `CRAFT`: `CRAFT_TITLEBAR=app` forces
the app frame, `=system` (or `=desktop`) forces the desktop frame. Apps pass their own prefix
(`Prefix::new("PDFCRAFT")` → `PDFCRAFT_TITLEBAR`) so each keeps its own variable name.

## Use

```rust,ignore
use craft_window::{Choice, Prefix, decorate};

// Once at startup, before the window is created:
let choice = Choice::probe(Prefix::new("PHOTOCRAFT"));
let viewport = decorate(egui::ViewportBuilder::default().with_title("PhotoCraft"), choice.frame());

// If the app draws the frame, lay the header-bar buttons out by the desktop's order:
if let Some(layout) = choice.frame().layout() {
    // layout.left / layout.right -> draw Minimize/Maximize/Close there
}
```

An app that offers a "System title bar" preference checks `choice.overridden()` and its own setting,
and can override the probe by setting the frame to `Frame::Desktop`.

The `egui` feature adds [`decorate`], the [`title_bar`](#the-apps-own-title-bar) module and
`Viewports`. Without it, the crate is pure decision and bookkeeping logic with no egui dependency.

## The app's own title bar

An app that draws over the title bar — a hidden macOS title bar with a full-size content view, or
`Frame::App` — makes its top strip act as one with `craft_window::title_bar` (`egui` feature):

| Where the drag or double-click starts | What happens |
| --- | --- |
| The strip's empty part | A drag moves the window (`ViewportCommand::StartDrag`); a double-click follows the system setting (macOS `AppleActionOnDoubleClick`: zoom, minimize or nothing; zoom elsewhere) |
| A control on the strip | The control gets it; the window stays put |
| An in-window panel over the strip (an `Order::Middle` layer: a floating dock, an `egui::Window`) | The panel gets it; the window stays put |
| A modal's backdrop or a tooltip (`Foreground`/`Tooltip`) | The window moves, as under AppKit's own title bar |
| Anything, in full screen | Nothing happens to the window |

```rust,ignore
egui::Panel::top("app_bar").show(ui, |ui| {
    // Before the bar's controls, so they sit on top and take their own clicks.
    craft_window::title_bar::empty_area(ui, ui.max_rect());
    // ... the bar's controls ...
});
// macOS, once per frame after the UI: `applied` is app state, starting at `None`.
#[cfg(target_os = "macos")]
craft_window::title_bar::sync_movable(ui.ctx(), &mut self.applied);
```

On macOS, AppKit moves a full-size-content window from a drag anywhere in the title-bar band, the
app's controls and panels included, unless `NSWindow.movable` is false. `sync_movable` sets that
flag from `title_bar::window_movable`, which is false while the pointer is on a control or a panel
over the strip. It leaves windows with hidden window buttons alone, such as floating panels in
their own windows.

## One window per document

For apps that open each document in its own top-level window, the crate also keeps the shared
window bookkeeping. The core has no egui dependency.

| Type | What it does |
| --- | --- |
| `WindowList<T>` | Open windows with the app's per-window data `T`. Opening order (stable, for menus) and most-recent order; the first in most-recent order is the active window, which menu commands go to. `close` returns the data and, when the active window closed, the window to focus next. `find` returns the most recent window whose data matches, e.g. the window already showing a file. |
| `WindowId` | A window's id: `Copy`, `Hash`, `Ord`, from one process-wide counter, so never reused while the app runs. Use it as the key for per-window state (UI state, pending dialogs). |
| `cascade` | Places a new window one step (about a title bar, `CASCADE_STEP`) right of and below the most recent window. Each axis that would run off the bounds (monitor or work area) starts over at the bounds' edge. |
| `WindowMenu` | The Window menu (macOS) or View › Switch Windows list: (id, title, dirty, active) per window, in opening order. Windows on the same document are titled `Name:1`, `Name:2`, numbered by opening order; a document with one window keeps its plain name. The app supplies each window's document key, name and dirty flag. |

With the `egui` feature, `Viewports` shows the windows as egui immediate viewports:

- Call `show` once per frame from the root viewport's UI. It runs the app's callback for each
  window inside that window's viewport, so `ui.ctx()` input (close requests, IME) and commands
  belong to that window. A window's viewport id is `child_viewport_id(id)`.
- New windows get `decorate` (the frame decision above) and a cascade position from the most
  recently focused window. On Wayland, where windows have no known position, the compositor places
  them.
- The root viewport stays the app's. `set_root(Some(id))` draws window `id` in it. When the root's
  window closes while others stay open, `adopt` moves another window into the root viewport (its
  position, size and focus); that window's id and data stay the same.
- `show` returns `WindowEvent::Focused` and `WindowEvent::CloseRequested`. Focus also updates the
  list's active window. Close and quit policy stay with the app: a child window closes when the app
  removes it from the list, and for the root's window the app sends `ViewportCommand::CancelClose`
  to the root in the same frame unless the app should quit.

```rust,ignore
use craft_window::{Document, Viewports, WindowEvent, WindowList, WindowMenu};

let menu = WindowMenu::new(&windows, |_, w| Document { key: w.doc, name: name_of(w.doc), dirty: is_dirty(w.doc) });
let base = egui::ViewportBuilder::default().with_inner_size([1000.0, 700.0]);
for event in viewports.show(
    ui,
    &mut windows,
    |id, _| base.clone().with_title(menu.title(id).unwrap_or_default()),
    |ui, id, window| draw_window(ui, id, window),
) {
    if let WindowEvent::CloseRequested(id) = event {
        // The app's policy: prompt for the document's last dirty window, then `windows.close(id)`,
        // and `adopt` or `focus` the window `Closed::focus` names.
    }
}
```

Immediate viewports repaint every window when one repaints, so cache expensive per-window drawing.

## Notes

- The Wayland probe opens a short-lived connection and reads the global registry once, on its own
  thread with a 500 ms timeout; on failure or timeout it keeps the desktop's frame. Call it once at
  startup.
- It never touches the connection winit inherits: when `WAYLAND_SOCKET` is set (a compositor handed
  the app an open descriptor), the probe skips connecting and keeps the desktop frame, so it can't
  consume the socket winit needs.
- The GNOME `button-layout` lookup (`gsettings`) runs behind the same bounded worker, so a stalled
  `gsettings` can't block startup either.
- `#![forbid(unsafe_code)]`, and the never-crash lints the workspace requires.
