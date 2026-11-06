# Shared group hover glow

`theme::edge_glow` shares the bottom-edge accent glow from
[WordCraft #3781](https://github.com/storytold/wordcraft/pull/3781). It can highlight ribbon groups
and inspector sections without adding a second interactive widget over their child controls.

Reserve a shape behind the group before rendering its contents, then replace it once its
rectangle is known:

```rust
let slot = ui.painter().add(egui::Shape::Noop);
// Render controls and obtain group_rect here.
let over = ui.is_enabled() && ui.rect_contains_pointer(group_rect);
let strength = ui.ctx().animate_bool_responsive(group_id.with("glow"), over);
ui.painter().set(slot, craft_ui::theme::edge_glow(group_rect, accent, strength, 9.0));
```

Set global egui animation time to zero for the host's motion-off preference. The painter obeys
its normal clip rectangle. The glow stays inside its group and respects the supplied accent's
alpha; use a translucent accent for a subtle effect. Invalid geometry/strength produces no shape.
Theme, hover policy, stable group identity, enabled state and layout remain caller-owned.
