craft-window
============

Decide who draws the window frame — the desktop or the app — so every Craft app shows the same
titlebar and borders on a given desktop, and keep the window list and Window menu for apps with one
window per document.

## Why

egui apps either keep the desktop's window frame (title bar, borders, window buttons) or turn it
off and draw their own. This crate makes that decision in one place, so every Craft app wears the
same title bar on a given desktop.

## The rule

The app's own bar is the title bar by default (`Frame::Integrated`). The user setting "Use system
title bar" (`TitleBarPref::System`) gives the frame to the OS, window manager or compositor
(`Frame::Native`).

| Platform, desktop, backend | Default | With "Use system title bar" |
| --- | --- | --- |
| macOS | `Inset`: full-size content view, hidden title, traffic lights over the app's bar | `Native` |
| Windows | `Integrated(Windows)`: minimize, maximize, close at the right | `Native` |
| GNOME, Wayland or XWayland | `Integrated(Adwaita)`, in the order of `button-layout` | `Native` |
| KDE, Wayland or X11 | `Integrated(Breeze)`, in the order of KWin's `ButtonsOnLeft` and `ButtonsOnRight` | `Native` |
| COSMIC, Xfce, Cinnamon, MATE, other floating desktops | `Integrated(Generic)` in the order of `gtk-decoration-layout`; Adwaita or Breeze when the GTK theme is one of those | `Native` |
| Tiling compositors and window managers (Sway, Hyprland, i3, niri, river, from `XDG_CURRENT_DESKTOP`) | `Native` | `Native` |
| Wayland without server-side decorations, not GNOME | `Integrated(Generic)` | `Native` |
| Web | `Native`; no frame code runs | `Native` |

The backend is the one the app opens its window on: an app that forces XWayland on a Wayland
session passes `Backend::X11`. The Wayland registry is probed (500 ms at most) only for
`Backend::Wayland`.

`policy::decide` is pure and takes the OS, backend, desktop, setting and override. `decide_for`
reads the platform once and calls it.

### Override

`<PREFIX>_TITLEBAR` wins over the setting. With the default prefix `CRAFT`: `CRAFT_TITLEBAR=app`
forces the app's bar (also on tiling compositors), `=system` or `=desktop` forces the native frame.
Apps pass their own prefix (`Prefix::new("PDFCRAFT")` reads `PDFCRAFT_TITLEBAR`).
When `<PREFIX>_TITLEBAR` is unset, `CRAFT_TITLEBAR` applies, so one variable covers every app. `Decision::overridden` reports `Some(true)` or `Some(false)` when the variable decided.

## Use

```rust,ignore
use craft_window::Prefix;
use craft_window::policy::{Backend, Frame, TitleBarPref, apply, decide_for};

// Once at startup, before the window is created:
let pref = if settings.system_title_bar { TitleBarPref::System } else { TitleBarPref::Integrated };
// Backend::from_env() is what winit opens; an app that forces XWayland passes Backend::X11.
let decision = decide_for(Prefix::new("PHOTOCRAFT"), Backend::from_env(), pref);
let viewport = apply(egui::ViewportBuilder::default().with_title("PhotoCraft"), &decision.frame);

if let Frame::Integrated(captions) = &decision.frame {
    // captions.family picks the button look; captions.layout.left / .right place the buttons.
}
```

This crate decides *whether* and *where*; craft-ui's title bar draws the buttons, the drag area and the resize zones for the frame it chose.

### Live layout changes

`policy::LayoutWatch` keeps the caption layout current without a restart:

```rust,ignore
use craft_window::policy::{Frame, LayoutWatch};

let mut layouts = LayoutWatch::new(&decision);
// Each frame, with the app's one craft_platform::appearance::Watcher (it also supplies the theme):
let mut changed = layouts.set_gnome(watcher.button_layout().as_deref());
if window_gained_focus {
    changed |= layouts.on_focus();
}
if changed && let Frame::Integrated(captions) = &mut decision.frame {
    captions.layout = layouts.layout().clone();
}
```

| Desktop | Live source |
| --- | --- |
| GNOME | `button-layout` from the settings portal, through `craft_platform::appearance` |
| KDE | `kwinrc`, re-read by `on_focus` |
| Other Linux and BSD desktops | GTK's `settings.ini`, re-read by `on_focus` |
| Windows, macOS, web | Fixed |

A value with no usable button gives the desktop's default. The app passes the string from
craft-platform, so this crate has no D-Bus dependency and the theme and layout share one
connection.

## Older API

`Choice::probe` and the root `Frame::{Desktop, App}` follow the earlier two-way rule (the desktop
draws the frame, except on Wayland without server-side decorations). They still work; `policy`
supersedes them.

The `ui` feature adds `policy::apply` (the frame decision on an `egui::ViewportBuilder`) and
`desktop::fits`. Without it, the crate is pure decision and bookkeeping logic with no egui
dependency.

### macOS traffic lights

`mac::measure` measures the zoom button of a titled full-size-content window and returns the
`MacInset` the app's bar reserves: `leading` is the width to the end of the lights plus the gap
they keep from the top, `centre_from_top` their centre line. `mac::Cache` keeps the result and
measures again only after leaving full screen or on a scale change. Until a window can be measured
`mac::default_inset(major)` gives the inset for the running macOS version (67 / 14 before macOS 26,
78 / 16 from 26, the latter measured on 27). In full screen there is none.

To check on a Mac (macOS 26 and 27): the inset ends one gap after the green button and centres on
the lights; a second window gets the same inset; full screen drops it and leaving full screen
restores it; moving the window between displays of different scale re-measures.

## One window per document

For apps that open each document in its own top-level window, the crate also keeps the shared
window bookkeeping. It has no egui dependency.

| Type | What it does |
| --- | --- |
| `WindowList<T>` | Open windows with the app's per-window data `T`. Opening order (stable, for menus) and most-recent order; the first in most-recent order is the active window, which menu commands go to. `close` returns the data and, when the active window closed, the window to focus next. `find` returns the most recent window whose data matches, e.g. the window already showing a file. |
| `WindowId` | A window's id: `Copy`, `Hash`, `Ord`, from one process-wide counter, so never reused while the app runs. Use it as the key for per-window state (UI state, pending dialogs). |
| `WindowMenu` | The Window menu (macOS) or View › Switch Windows list: (id, title, dirty, active) per window, in opening order. Windows on the same document are titled `Name:1`, `Name:2`, numbered by opening order; a document with one window keeps its plain name. The app supplies each window's document key, name and dirty flag. |

## Desktop and fit

`craft_window::desktop` is a public module: `Backend` and the desktop detection read the environment
and, on Wayland, the compositor's registry. With the `ui` feature it adds `fits(outer, monitor)`:
whether a window of outer size `outer` fits `monitor`, keeping `TASKBAR` clear at the monitor's
bottom. egui does not expose the OS work area, so this is an estimate, not an OS query.

## The application menu bar

`native_menu` makes the platform's own menu bar (macOS, feature `appkit`, built on muda) from a plain
description: `Bar` of `Menu`s of `Node`s (`Item { id, label, enabled, checked, shortcut }`, submenus,
separators, and the platform's own items such as Services and Zoom). `NativeBar::install` makes it the
application's menu bar, `NativeBar::apply` keeps it in step (a changed `structure` key rebuilds, a changed
`windows` key rebuilds only the Windows menu, anything else updates in place), and the `Handler` set with
`NativeBar::set_handler` receives each chosen item's `id` and, for a key equivalent, the modifiers held.
`install_app_menu` installs a minimal app menu whose Quit reports to a handler. Off macOS (and without the
feature) every call does nothing, so callers need no platform check. craft-menu lowers its menu model to
this description.

## Window restore

`craft_window::restore` (no egui).

- `place(saved, monitors, home, &Defaults) -> Placement` is pure. First launch: the default size fitted
  to the home monitor, maximized when it does not fit. Saved sizes are points at interface zoom 1.
  A window with less than 64×64 pt of its top strip on any one monitor moves to its saved monitor,
  else the nearest, else the primary; sizes clamp to the monitor's usable area and `min`. Positions
  are physical pixels (negative left of or above the primary). `Defaults::no_position` (Wayland)
  neither reads nor returns a position. Full screen returns only with `Defaults::restore_fullscreen`.
- `Store::open(app_id)` keeps `window.json` beside `config.toml` (same folder rules as
  `craft-settings`), `Store::in_memory()` keeps nothing. `get`, `set`, `save`.
  Format: `{ "schema_version": 1, "windows": { "main": Placement, "<key>": Placement } }`.
  Writes are atomic and private; a damaged file is moved aside as `window.broken-<ts>.json`; a newer
  `schema_version` is read-only; unknown keys are written back unchanged.
- `restore::legacy` reads eframe's `"window"` entry of `app.ron` (without a RON crate) and has small
  JSON helpers for an app's own old format.

Multi-window apps save the first window under `restore::MAIN`. Share one store between all windows as
`Arc<Mutex<Store>>`.

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
