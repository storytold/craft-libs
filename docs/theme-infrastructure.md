# Shared theme infrastructure

The first shared mechanism is `craft-ui::theme::WidgetPalette`: it applies colors to egui's five
widget states, with a supplied foreground stroke and corner radius. VectorCraft, DesignCraft and
LightCraft use the same operation. Theme choices, fonts, preferences and application-specific palettes remain in their frontends.
The additive `SuitePalette::ARTCRAFT_DARK` now supplies canonical standard-dark interface roles;
it does not alter the existing widget operation. This revises the original boundary: basic
interface role colors may be shared, while complete app themes and their extra tokens remain local.

This follows the reusable-UI workstream of [craft-libs issue 2](https://github.com/storytold/craft-libs/issues/2).
The [cross-app architecture discussion](https://github.com/storytold/vectorcraft/issues/647) and
[base-crate request](https://github.com/storytold/photocraft/issues/1217) provide context; this change
keeps the existing repository, document and release boundaries.

## Source comparison

Reviewed 2026-10-09 at the revisions below. All links point to
`crates/ui-egui/src/theme.rs`. The 3,454-line total includes font setup, specialized helpers and
tests; it is not a count of duplicate theme code. Token-field counts include non-color fields.

| App | Source revision | Lines | Token fields | Theme model |
| --- | --- | ---: | ---: | --- |
| PhotoCraft | [`e7a83ca342cd`](https://github.com/storytold/photocraft/blob/e7a83ca342cdff568254f85f442b9cc8afb08066/crates/ui-egui/src/theme.rs) | 832 | 37 | Five presets; per-preset metrics and Classic bevels |
| VectorCraft | [`825d4fdb8b52`](https://github.com/storytold/vectorcraft/blob/825d4fdb8b527c597855d273fb4bcc1971855b6a/crates/ui-egui/src/theme.rs) | 345 | 38 | Four brightness presets |
| DesignCraft | [`66c7ce7e96ec`](https://github.com/storytold/designcraft/blob/66c7ce7e96ec286a091878cc2541ee8369aa79d6/crates/ui-egui/src/theme.rs) | 426 | 33 | Four brightness presets plus High Contrast |
| WordCraft | [`d7d9702bda1f`](https://github.com/storytold/wordcraft/blob/d7d9702bda1fe626952024dc37fd2624323dad0f/crates/ui-egui/src/theme.rs) | 231 | 32 | Light/dark, ribbon and page colors |
| PdfCraft | [`f626db25cca2`](https://github.com/storytold/pdfcraft/blob/f626db25cca2c89f8fa3f7e30d22bf59faea65ec/crates/ui-egui/src/theme.rs) | 314 | 22 | System/light/dark preference |
| LightCraft | [`88ea1a804b35`](https://github.com/storytold/lightcraft/blob/88ea1a804b35709e1fe436e3792adf0428b97daa/crates/ui-egui/src/theme.rs) | 345 | 35 | Dark photo workflow palette |
| FilmCraft | [`7b6c13447228`](https://github.com/storytold/filmcraft/blob/7b6c134472287db4367fca80bcb498240ba36567/crates/ui-egui/src/theme.rs) | 447 | 43 | Three presets, custom accent and accessible contrast |
| EffectCraft | [`2a460986ee7a`](https://github.com/storytold/effectcraft/blob/2a460986ee7a4b32656a3d434849693904b486bc/crates/ui-egui/src/theme.rs) | 514 | 44 | Three presets, brightness and label colors |

The matching widget-state operation is visible in [VectorCraft lines 299–312][vector-loop],
[DesignCraft lines 336–348][design-loop] and [LightCraft lines 235–247][light-loop]. It assigns
`bg_fill`, `weak_bg_fill`, `corner_radius` and `fg_stroke` across noninteractive, inactive, hovered,
active and open states. The state colors differ, but the assignments and their meaning agree.

The surrounding `Visuals` operations differ materially:

| App | Differences that remain local |
| --- | --- |
| VectorCraft | Selection uses accent at 0.55 gamma with an accent stroke. Popup/window radius is 6; shadows vary with light/dark. Inactive border is absent, hover uses input border and active uses accent. Body/button text is 13 pt. |
| DesignCraft | Selection uses strong accent and strong-text stroke. Radius is 4; window shadow stays egui's default. Interactive borders use field border. Inactive buttons get transparent weak fill and input background after the shared operation. Body/button text is 11 pt. High Contrast keeps black/white/yellow colors. |
| LightCraft | The matching loop uses `text_label`, intentionally dimmer than the app's main text, and a fixed dark palette. Catalog label helpers depend on its own model. |
| FilmCraft | Custom highlight and contrast preferences feed its palette. Widget states use ordinary text for inactive/noninteractive and brighter text for hovered/active; open foreground remains inherited. |
| EffectCraft | Its widget assignment block matches FilmCraft's, but brightness adjustment, label/gradient helpers, popup radius, shadows, spacing and solid scrollbars differ. |
| PhotoCraft | Noninteractive text is dimmer; interactive states have separate border and expansion policies. Classic styling includes bevels, and font size is preference-driven. |
| PdfCraft | System theme resolution and named context storage are local. Its foreground loop preserves each inherited stroke width; a blanket full-stroke assignment would change that contract. Light text and font subpixel settings differ. |
| WordCraft | Tokens derive from egui's current dark mode. Only three interactive states get explicit radii; other states inherit egui defaults. Ribbon/document colors have separate meaning. |

FilmCraft and EffectCraft's separate matching block is a useful second extraction candidate, but
it has a different foreground-state contract. Extending the first helper to cover it immediately
would obscure the small invariant being shared. There is no single cleanest complete theme:
the first pair offers the clearest common operation, and the other apps preserve different
requirements rather than providing interchangeable replacements.

## Boundary and API

```rust
pub struct WidgetPalette {
    pub noninteractive: egui::Color32,
    pub inactive: egui::Color32,
    pub hovered: egui::Color32,
    pub active: egui::Color32,
    pub open: egui::Color32,
}

impl WidgetPalette {
    pub fn apply_to(
        self,
        widgets: &mut egui::style::Widgets,
        radius: egui::CornerRadius,
        foreground: egui::Stroke,
    );
}
```

The implementation lives in [`crates/ui/src/theme`](../crates/ui/src/theme/mod.rs). It changes only
those four properties. Borders, expansion, selection, shadows and the rest of egui's visuals
stay untouched. Apps apply their state-specific exceptions afterward. `noninteractive` names
egui's state, not a new disabled-control policy.

A separate `craft-theme` crate adds no useful boundary: this mechanism already depends on egui
and belongs beside the controls it styles. Shared role colors live in `SuitePalette`, because one
palette keeps the apps consistent; a theme enum, application identifier or global context key is
unnecessary. Constructing another large visual-options
object would mostly rename `egui::Visuals` fields without simplifying their use. Context lookup
and storage are already small and retain different fallback themes.

The identical packed-RGB helper in VectorCraft and DesignCraft is only a few casts; it remains
local in this extraction. A contrast API also needs a second runtime consumer before extraction.

## Canonical ArtCraft standard dark roles

`craft_ui::theme::SuitePalette::ARTCRAFT_DARK` is the shared source of truth for the 21 role
colors in the [ArtCraft dark theme contract (14 October 2026)](artcraft-dark-theme-contract.md). Its public `Color32`
fields use the contract's names: `window`, `chrome`, `panel`, `raised`, `input`, `hover`,
`pressed`, `border`, `border_strong`, `text`, `text_strong`, `text_secondary`, `text_disabled`,
`accent`, `accent_hover`, `focus`, `on_accent`, `selection`, `selection_text`, `primary` and
`on_primary`. Each scheme is a constant of the same type, so light or app-group schemes can be added without
changing the role API. An app can define its own scheme or override individual roles with
`SuitePalette { accent, ..SuitePalette::ARTCRAFT_DARK }`. The palette is immutable data and has
no application or global-visuals method.

Applications adapt these roles to their existing local tokens in their standard dark path.
They retain preference IDs and defaults, system-theme resolution, custom highlight/contrast
settings, light and alternate brightness presets, High Contrast, fonts, metrics and radii.
Apply the canonical roles after choosing the standard dark preset so alternatives inheriting
its old base are not silently recolored. This API adds no app dependency and requires no
preference migration. Consumer dependency upgrades and releases remain separate decisions.

Use `on_accent` on blue filled controls and `selection_text` on `selection` interface rows;
`primary` and `on_primary` describe inverted white primary buttons. Preserve semantic warning,
error, success, record and meter colors, and editing/document pixels, swatches, guides, canvas
preferences and meaningful selection overlays. Do not replace literal colors mechanically.
`WidgetPalette` continues to accept caller-provided colors and changes exactly its existing
four properties; apps decide how role colors feed widget states and apply local exceptions.

Tests lock all 21 exact opaque values and check normal text on interface surfaces (secondary
labels through hover; pressed controls use normal/strong text), selected
rows, filled controls, focus edges and strong borders using sRGB relative luminance. Disabled
text and subtle borders intentionally have no normal-body-text contrast guarantee. Shared
palette tests cannot certify whole-app accessibility: each adapter still needs role/style tests,
light/alternate/High Contrast preservation checks and source-based UI renders. No installed or
released application outcome follows from this library-only addition.

## Fonts and styling outside theme modules

Font setup occupies hundreds of the audited lines. It depends on app-specific font catalogs,
locale handling and fallback order: PhotoCraft uses its text and preference crates; VectorCraft
uses `vectorcraft_text`; DesignCraft and WordCraft use their font crates; PdfCraft adds script
and system fallback; LightCraft uses its engine catalog; FilmCraft and EffectCraft use their text
crates and local font discovery. DesignCraft also configures named Arabic shaping families.
These dependencies and font assets stay outside `craft-ui`.

Theme references occur throughout panels, menus and tools, with local visual/spacing overrides
in every app. A source search alone cannot turn this into a percentage of correctly themed UI:
many literal colors are document content, swatches, previews or intentional contrast choices.
A search of UI source files outside `theme.rs` found the following. The columns count files,
including tests, containing token/theme references, local visual/spacing mutation, or literal
`Color32` construction. They are navigation aids, not counts of defective controls.

| App | Theme references | Local overrides | Literal colors |
| --- | ---: | ---: | ---: |
| PhotoCraft | 115 | 37 | 59 |
| VectorCraft | 149 | 40 | 41 |
| DesignCraft | 26 | 8 | 19 |
| WordCraft | 10 | 4 | 9 |
| PdfCraft | 40 | 11 | 30 |
| LightCraft | 40 | 20 | 27 |
| FilmCraft | 53 | 4 | 46 |
| EffectCraft | 61 | 1 | 55 |

Examples worth preserving or reviewing separately:

- PhotoCraft's `widgets.rs` draws fixed light/dark Classic bevels and removes field chrome;
  `menus.rs` supplies its own menu styling.
- VectorCraft's `widgets.rs` uses white primary-button labels and contrasting fill/stroke wells.
- DesignCraft's `widgets.rs:147` sets focused numeric-field colors independently of High Contrast;
  fixed dialog warning/error colors also merit a later accessibility review.
- WordCraft's ribbon Share control uses white; PdfCraft's notification surfaces and dialog
  widgets have local light/dark styling.
- LightCraft's dialog download errors use a fixed hue, while its color helpers also draw actual
  photo-workflow values.
- FilmCraft's header customizes menu chrome; EffectCraft's effect widgets and ScriptUI controls
  customize backgrounds and slider widths.

This extraction preserves those choices. It does not certify whole-app accessibility or theme
consistency, and it does not move font installation into the shared crate.

## Migration and validation

1. Capture VectorCraft's four themes and DesignCraft's five, including High Contrast, before
   replacing either loop. Use synthetic documents, panels/inspectors, menus and preferences at
   1× and 2× scales. Save the complete egui Style and app Tokens for comparison.
2. Add the shared helper with a test that preserves custom borders and expansion while replacing
   all state fills, including reapplying a theme after a local override.
3. Replace only each app's five-state loop. Keep its existing colors, strokes, post-loop
   exceptions, fonts, spacing, context state, commands and saved preference schema.
4. Repeat the renders and compare complete styles/tokens and images. Exercise theme commands,
   menu check states and an in-memory saved-preference round trip. A process restart or native
   display acceptance remains a separate check.
5. Run shared fmt, strict Clippy, tests and wasm, then each app's own gates. Each app pins the
   shared revision independently and can revert its adapter/pin without migrating preferences.

The main risks are accidentally resetting inherited egui fields, losing DesignCraft's inactive
outline exception or High Contrast colors, and confusing font/locale behavior with theme
application. Exact style comparison plus visual checks covers the first two; keeping font code
unchanged limits the third. Shared code depends only on egui, and layer checks keep it above
engines and document models.

The suite trial also adopts LightCraft's identical loop. FilmCraft/EffectCraft's widget-state matrix
and consistent focused numeric fields remain candidates for separate work. Theme adapters for the
shared tabs and buttons now also serve PdfCraft, WordCraft, GridCraft, DeckCraft, CADCraft and
SoundCraft; their full palettes stay local. Further extractions need two real consumers and their
own visual and interaction checks.

[vector-loop]: https://github.com/storytold/vectorcraft/blob/825d4fdb8b527c597855d273fb4bcc1971855b6a/crates/ui-egui/src/theme.rs#L299-L312

[design-loop]: https://github.com/storytold/designcraft/blob/66c7ce7e96ec286a091878cc2541ee8369aa79d6/crates/ui-egui/src/theme.rs#L336-L348

[light-loop]: https://github.com/storytold/lightcraft/blob/88ea1a804b35709e1fe436e3792adf0428b97daa/crates/ui-egui/src/theme.rs#L235-L247
