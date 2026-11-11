# craft-imaging

Raster image editing for Craft apps.

Without egui, on every target: `craft_imaging::scopes` (the clipping rule and clipping masks, the vectorscope grid, hue and saturation, and histogram statistics; the app does the colour management and the histogram count), `selection` (selection preview and Quick Mask rules), `trace` (mask outline tracing) and `layer_style` (the Layer Style value model).

With the `ui` feature, `craft_imaging::ui` has:

- the `WarpGrid` (a warp's mesh on a canvas, with `WarpMesh`, `WarpEdit` and the press and release helpers), the `WarpFields` of its options bar, `PinWarp` with `PinWarpBar`, and `PerspectiveWarp` with `PerspectiveBar`; `TransformPreview` shows a transform or warp in progress;
- the `LevelsEditor` (a channel's `Levels` over its histogram) and the editors of the other adjustments: curves (`CurvesPanel`), brightness and contrast, color balance, hue and saturation, black and white, channel mixer, selective color, photo filter and threshold, over an `AdjustSession`; `LivePreview` renders them on a proxy of the image; `BlendKeys` picks a blend mode from the keyboard;
- the `LayerStyleDialog` with the `BlendIfBar` and the `ContourField`;
- the selection tools (`MarqueeTool`, `LassoTool`, `PolygonLassoTool`, `MagneticLassoTool`), the `SelectionOverlay` (ants, matte, Quick Mask) and the `MaskViewControls`;
- the full-window editors, each a workspace over a session, panel and options bar: `SelectMaskWorkspace`, `ContentAwareFillWorkspace`, `FilterGalleryWorkspace`, `LiquifyWorkspace`, `WideAngleWorkspace` and `VanishingPointWorkspace` (settings in `VanishingPointSettings` and `StampSettings`); the limits an app passes to Select and Mask and Content-Aware Fill are `SelectMaskLimits` and `FillLimits`, and a refused edit is a `LimitRefused` or a `FillLimit`; `BlurPinsOverlay` and `BlurPinsPanel` edit blur pins on the canvas;
- the `ChannelsPanel` (thumbnails, eyes, selection, rename in place with `start_rename`, drag, the footer (`ChannelsEvent::Footer` and `FooterDrop`) and the menus over plain `ChannelRow`s; `ChannelsPanel::style(&UiTokens)` gives the look);
- the `ScopePanel` (histogram, statistics, tone zones and vectorscope) and the `ClippingOverlay` for a preview;
- their stories, registered in `craft_ui::testing::stories()`.

The main types of every module are re-exported at `craft_imaging::ui`. The panels and editors report what the user did as events and hold no document. Thumbnails load as jobs on `craft_host::queue::JobQueue`.

The curves editor and the histogram stay in `craft-ui`.
