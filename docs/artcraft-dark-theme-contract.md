# ArtCraft dark theme contract (14 October 2026)

This is the proposed standard-dark interface scheme exposed as
`craft_ui::theme::SuitePalette::ARTCRAFT_DARK`. Adoption and default selection remain decisions
for each app's maintainers and project owner. This document defines colors and adapter boundaries;
it does not prescribe a complete theme, layout, or application preference.

## Interface roles

All roles below are opaque sRGB colors. Names describe interface use, not document pixels.

| Role | Color |
| --- | --- |
| `window` | `#121316` |
| `chrome` | `#0a0a0c` |
| `panel` | `#101014` |
| `raised` | `#1e1f22` |
| `input` | `#060607` |
| `hover` | `#323337` |
| `pressed` | `#414246` |
| `border` | `#36373a` |
| `border_strong` | `#77787c` |
| `text` | `#f2f1ee` |
| `text_strong` | `#ffffff` |
| `text_secondary` | `#a6a6a8` |
| `text_disabled` | `#717176` |
| `accent` | `#2d81ff` |
| `accent_hover` | `#59a7ff` |
| `focus` | `#74aaff` |
| `on_accent` | `#060607` |
| `selection` | `#203b65` |
| `selection_text` | `#f2f1ee` |
| `primary` | `#ffffff` |
| `on_primary` | `#0b0b0c` |

Use `on_accent` on blue filled controls and `selection_text` on selected interface rows.
`primary` and `on_primary` support inverted white primary buttons. Disabled text and subtle
borders have different contrast requirements from normal body text.

## Relationship to the ArtCraft reference

The scheme reuses ArtCraft's near-black surfaces, warm foreground, blue accent and focus, and
inverted primary-button colors from
[the native reference](https://github.com/storytold/artcraft/blob/d4e4ac69bbd93d237eaf5d0260bc6a5ff3a5e373/artcraft-native/src/theme.rs).
It is an intentional standardization, not an exact copy of every reference token.

That reference uses translucent `LINE` (white at 38/255 alpha), `LINE_STRONG`
(warm foreground at 102/255), `WASH` (white at 13/255), selected wash (white at 20/255),
and hover wash (white at 26/255). Translucent colors depend on the surface below them.
The shared border, stronger border, hover, pressed and selection roles are explicit opaque
choices for predictable appearance and contrast across different app surfaces; they are not
claimed to be flattened equivalents of those overlays. Secondary and disabled text similarly
use explicit opaque values. App-specific quiet washes, dashed targets, shadows and faded icons
can remain local translucent tokens.

## Adapter boundary

Applications map roles into their local tokens after selecting their standard-dark preset.
Keep saved preference IDs, stored choices, light and named alternative themes, High Contrast,
custom highlight/contrast settings, typography, spacing, radii and layout local. Inherited dark
constructors must not silently recolor alternate presets. An app may override roles or define
another scheme using the same `SuitePalette` type.

Preserve document, page, worksheet, slide, artboard and media colors; canvas preferences;
semantic warning/error/success/record/meter colors; swatches, guides and meaningful editing
selection overlays. A common UI accent does not imply a common document-selection accent.
The ArtCraft web frontend needs its own CSS adapter and parity checks because it cannot
directly consume the Rust constant.

Shared tests check role values and intended contrast pairs. Each app still needs adapter tests,
alternate-theme preservation checks and source UI verification. Publishing a shared-library
change alone does not update pinned consumers or installed applications.
