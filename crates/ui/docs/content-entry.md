# Tab content entry

`tabs::ContentEntry` shares the directional content transition from
[WordCraft #3781](https://github.com/storytold/wordcraft/pull/3781). It tracks one region's selected
tab, stable identity, order and entry clock without retaining or rendering application content.

```rust
let frame = entry.update(selected_id, tab_index, now, 0.16, animations_enabled);
let offset = egui::vec2(f32::from(frame.direction) * (1.0 - frame.progress) * 14.0, 0.0);
let opacity = 0.2 + 0.8 * frame.progress;
// Apply offset and opacity to the caller's clipped child UI containing the new page.
if frame.moving { ui.ctx().request_repaint(); }
```

Initial appearance, same-index page replacement, motion-off, invalid parameters and backwards
clocks settle immediately. Rapid selection changes restart the new page's entry. Reordering the
same page does not start another animation. Host-owned layout decides direction for RTL strips.
The host keeps focus and command policy, disabled state, clipping and page contents, and decides
whether input should be suppressed during entry. A fading page is not automatically disabled.
