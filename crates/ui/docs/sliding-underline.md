# Sliding selection underlines

`tabs::SlidingUnderline` extracts the selection-marker motion from
[WordCraft #3781](https://github.com/storytold/wordcraft/pull/3781) for ribbons, settings tabs and
inspector strips. Keep one instance per strip and supply stable tab IDs, target geometry,
monotonic seconds, duration and the host's motion preference.

```rust
if let Some(frame) = underline.update(selected_id, target_rect, now, 0.2, animations_enabled) {
    ui.painter().rect_filled(frame.rect, 2.0, accent);
    if frame.moving { ui.ctx().request_repaint(); }
}
```

Interrupted selection changes start from the marker's current position. Initial appearance and
same-selection layout changes snap. Motion-off and invalid/backwards clocks discard movement;
invalid rectangles clear the state. Color, shape, strip identity, input and selection remain in
the caller. No page or document model is required. Consumer migration is separate.
