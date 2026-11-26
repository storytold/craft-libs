craft-tablet
============

Pen pressure from a drawing tablet on X11 and Xwayland, where winit reports the pen as a plain
mouse. `spawn` reads XInput2 raw events on a second, pure-Rust X connection (no libxcb) in a thread
of its own and reports each pen sample's pressure (0..=1), or `None` when another pointer moves.
`xi` is the pure mapping from XInput2 valuators to pressure.

Linux only: elsewhere `spawn` returns `Error::Unsupported`. A native Wayland window gets no tablet
input through X, so an app starts the reader only for an X11 or Xwayland window.

```no_run
craft_tablet::spawn("my-app-tablet", |pressure| println!("pen: {pressure:?}"), || println!("X closed"))
    .unwrap_or_else(|e| eprintln!("no tablet pressure: {e}"));
```
