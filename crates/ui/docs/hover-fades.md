# Opt-in icon button hover fades

`IconButton::animated_hover(true)` smoothly blends normal/hover fill, border and icon tint using
egui's animation clock. It carries the hover-feedback concern from
[WordCraft #3781](https://github.com/storytold/wordcraft/pull/3781) into the existing shared control.
It works for ribbons, toolbars and inspectors without importing an application theme.

```rust
craft_ui::buttons::IconButton::new(localized_label, size, &style)
    .animated_hover(true)
    .show(ui, paint_icon);
```

Existing buttons remain instantaneous by default. Duration follows global `Style::animation_time`;
the host maps its motion preference to zero to disable animations. Invalid/negative durations
snap; positive durations are capped at ten seconds. Disabled-state changes snap immediately.
Selection and an explicit pressed style take precedence over the fade. egui requests repaint
while animating. Focus, accessibility, hit geometry, clipping and command dispatch are unchanged.
Stable UI scopes keep each animation attached to its button when labels are translated.
