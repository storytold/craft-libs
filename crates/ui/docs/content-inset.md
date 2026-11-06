# Dialog field indentation

`craft_ui::layout::ContentInset` places subordinate controls under a section heading without
adding a tree guide or changing the application's global indentation settings. It supports
fractional theme spacing and horizontal parents, and bounds padding in narrow containers.

```rust
ui.label("Display"); // localize in the application
craft_ui::layout::ContentInset::new(12.5).show(ui, |ui| {
    ui.checkbox(show_grid, "Show grid");
});
```

Add `.right(points)` for trailing space. Left/right mean physical sides; applications choose
the appropriate padding for their language and layout. Both insets are included in the consumed
scope, so following horizontal siblings retain the gap. The scope inherits layout, clipping and
disabled state. It adds no vertical space or frame. Use it independently or inside a caller-styled
section. Invalid or negative padding becomes zero; finite padding is capped at one million logical
points, then at available width, with the left side taking priority.

Extracted from the indentation concern in [WordCraft #4038](https://github.com/storytold/wordcraft/pull/4038).
This also suits property inspectors and settings panels in other Craft apps. Consumer adoption is separate.
