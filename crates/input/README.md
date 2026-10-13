# craft-input

One display policy shared by the Craft suite and ArtCraft Launcher. The default feature set
is pure Rust, with no windowing or OS dependencies, and supports WebAssembly.

`DisplayMode::Auto` follows `XDG_SESSION_TYPE` when its connection is configured. Otherwise it
uses a Wayland connection (`WAYLAND_DISPLAY` or `WAYLAND_SOCKET`), then X11 (`DISPLAY`), then
lets winit report the missing display. Explicit unavailable choices return a startup error.
`CRAFT_DISPLAY_BACKEND=auto|wayland|x11` overrides an app's saved preference for one launch.
Without an override, an existing explicit app preference is retained.

Enable `serde` for persisted choices, `native` for winit helpers, and `wgpu` or `glow` for the
renderer the consuming eframe app already uses. These features do not choose a new renderer.
`configure` composes with an existing event-loop builder callback. It does not mutate global
environment variables, and leaves other operating systems' startup unchanged.

The Wayland adapter requires the suite's patched winit and egui-winit dependencies. Native
file drops use the existing egui dropped-file API. Portal transfers retrieve paths off the UI
thread and hold the source offer until retrieval completes. Tablet-v2 samples belong to the
window that received them: pressure, tilt, barrel rotation and eraser state reach `tablet_feed`
without a second Wayland connection or a permanent polling timer. An app with its own richer
pen model may filter tagged tablet Touch contacts with `is_native_tablet_touch`; finger input
and the pointer events remain available.

Native input does not establish physical tablet or compositor acceptance. The suite's runtime
checks and hardware acceptance matrix must be completed before declaring a full migration.
