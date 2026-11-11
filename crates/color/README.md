# craft-color

Colour for Craft apps. With the `ui` feature, `craft_color::ui` has the colour picker (`ColorPicker`, `ColorPopover`, `ColorArea`, `ColorStrip`, `ColorWheel`, `ColorDisc`, `ColorRamp`), the wells (`SwatchPair`, `Well`), `SwatchPalette` tables and the `HexField`. It also has the colour features built on them: the Color Picker dialog (`ColorPickerDialog`), dialog droppers (`DropperButton`, `DropperState`), the Eyedropper's options and ring, Color Sampler points, the screen colour picker (`ScreenColorPicker`), `ColorChoice`, `PaintChip` and `PaintList` (with their `PaintKindLabels` and `PaintListLabels`), the Color and Swatches panels (`ColorPanel`, `SwatchesPanel`), the profile mismatch prompt, the harmony wheel and Color Guide panel, and the `LutPicker`.

Stories: `patterns/color/{picker-dialog, panel, swatches, eyedropper-bar, samplers, paint-chip, choice, harmony, lut-picker, profile-mismatch}` show each of those features in its states. The two dialogs run on a `DialogHost` (a "Show dialog" button in an Overview tile). The screen eyedropper is off in the stories, so none starts a screen capture.

Without `ui`: `CmykModel` (CMYK through a craft-cms profile), swatch files (`palette`: `.aco`, `.act`, `.ase`, `.gpl`, `.css`), the display transform and its caches (`display`), and colour harmonies (`harmony`).

The colour model and conversions are in `craft-theme`; profiles and transforms in `craft-cms`.
