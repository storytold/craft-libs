# craft-theme

The Craft design system's theme model. A theme is a few seeds and optional pins; `resolve` turns
it, with an appearance (mode, contrast, size, text size), into tokens that apps map onto their
own styles.

Pure data and functions: no egui, no I/O, no threads, and it builds for wasm. Theme files on
disk, the suite folder and change watching belong to `craft-theme-store`; `craft_ui::theme`
converts tokens for egui. [docs/suite-themes.md](../../docs/suite-themes.md) is the reference for
theme authors and app developers.

```rust
use craft_theme::{Appearance, Contrast, Mode, Size, TextSize, builtins, resolve};

let theme = builtins().iter().find(|t| t.id.as_str() == "craft.graphite").unwrap();
let appearance = Appearance { mode: Mode::Dark, contrast: Contrast::Standard, size: Size::Compact, text_size: TextSize::Small, reduced_motion: false };
let resolved = resolve(theme, appearance, None);
// resolved.tokens: colours, shape, size, space and type tokens; resolved.warnings: contrast misses.
```

## Tokens

Three layers, each derived from the one before:

1. **Primitives**: sRGB and OKLCH colour, WCAG contrast, 12-step colour ramps, fixed scales
   (space, radius, stroke, opacity, duration, easing, elevation).
2. **Semantic tokens**: surface, text, line, state, accent, status, overlay, data and caption
   colours; shape, size, space, motion and type tokens. Seeds (neutral, accent, four status
   colours) generate ramps, and ramps fill the tokens.
3. **Component tokens**: one struct per component family (button, badge, check box, radio button and switch, text and number field, guide, handle, icon button, kbd, list box, marquee, progress bar, readout, ruler, segmented control, slider, swatch, toggle button, tooltip), with curated overrides.

A theme sets **seeds** and may **pin** any colour token (`"surface.panel": "#F0F0F0"`), or a
component field, per mode. Pins win over derivation. Apps can pin per app (`apps.<id>`).

## Contrast

Text, state indicators, canvas overlays and data graphics have rules: text on surfaces, state fills
and solid fills (4.5, high contrast 7); focus rings, accent marks and a list's drop marker on its
halo (3, high contrast 4.5); overlay strokes on their halo and the marquee on its dashes (3, high
contrast 4.5); and the channel colours a curve editor draws (`data.red`, `data.green`, `data.blue`,
`data.cyan`, `data.magenta`, `data.yellow`) on `surface.sunken` (3, high contrast 4.5). Hover, pressed, dividers, edges, knob faces and row selection fills have none, and
neither have guide and smart-guide colours, which are the user's choice, nor `overlay.line`, the
bare accent line of boxes, paths and meshes on the canvas. In standard contrast an unpinned colour is moved to the minimum and
a failing pin is kept and reported, with every component pair it makes fail; in high contrast every colour is fixed, pins included.

## Files, built-ins, selection

| Module | What it is |
|---|---|
| `format` | the `craft-design-theme` v1 file: `parse`, `write`, `to_file`; every error with its path |
| `builtins()` | twelve `craft.*` themes, embedded from `themes/*.json` |
| `selection` | `Selection`, `SuiteChoice`, `AppChoice` (an app's own theme, mode and size over the suite's), `AppDefaults` (with the app's `AppSizes`), `resolve_selection`: what the user picked, filled in and resolved against what the OS reports |
| `ThemeId`, `AppId`, `APP_DEFAULT`, `SUITE_DEFAULT` | ids `[a-z0-9][a-z0-9._-]{1,63}`; `craft.` is reserved for built-ins |

The `craft-theme check <file>... [--json]` binary parses and validates theme files and prints
contrast warnings for every appearance. It exits 1 if any file has an error.

The selection types and the enums they hold implement `Serialize`/`Deserialize`:
colours as `"#rrggbb[aa]"`, ids as strings, enums in snake_case.
