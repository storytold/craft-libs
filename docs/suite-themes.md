# Suite themes

A suite theme is a named set of colours that every Craft app can paint its interface with, so that
one choice in one app can restyle the whole suite. This page is the contract the apps share and
how an app adopts it. It follows skylartaylor's brief on
[pdfcraft#1090](https://github.com/storytold/pdfcraft/pull/1090). PdfCraft, PhotoCraft and the
ArtCraft Launcher are its first consumers.

The contract is deliberately small. Apps keep their own `Tokens`, `egui::Visuals`, layout, fonts,
document and canvas colours and saved preferences. They map the 13 core roles below into their own
tokens through an adapter.

| Crate | Contents |
|---|---|
| [`craft-theme`](../crates/theme/README.md) | Theme ids, roles, themes, derived states, contrast, built-ins, resolution. No egui, no I/O; builds for wasm. |
| `craft-theme-store` | Theme files, the suite folder, the suite selection and change watching, outside rendering. |
| [`craft_ui::theme`](../crates/ui/README.md#theme-mechanisms) | `color32` and `install`: the bridge to egui. |

## Themes and selection

A theme has a stable `ThemeId` (`[a-z0-9][a-z0-9._-]{1,63}`), a display name, a version and a light
variant, a dark variant or both. Renaming a theme keeps its id. Ids starting with `craft.` belong to
the built-ins. User theme ids become file names, so `ThemeId::parse_user` and
`SuiteTheme::from_maps` refuse `craft.` ids and ids whose first dot-separated part is a Windows
device name (`con`, `nul`, `aux`, `prn`, `com1`-`com9`, `lpt1`-`lpt9`).

| Id | Name | Light | Dark |
|---|---|---|---|
| `craft.app-default` | App default | the app's own palette | the app's own palette |
| `craft.artcraft` | ArtCraft | PhotoCraft Pro Light | PhotoCraft Pro Medium |
| `craft.graphite` | Graphite | PdfCraft Light gray | PdfCraft Dark gray |
| `craft.high-contrast` | High contrast | black on white, dark blue accent | white on black, yellow accent |

`builtins()` returns the last three. `craft.app-default` is not a theme with colours: it stands for
each app's current palette, and the settings UI lists it beside the others.

A `Selection` is a theme id and a `Mode` (`System`, `Light` or `Dark`). The suite has at most one
selection, stored in the suite folder's `selection.json`. No `selection.json` means no suite
selection: nobody has picked one yet, and neither `Selection` nor `AppChoice` has a default that
could invent one.

Each app keeps an `AppChoice` in its own preferences: `FollowSuite`, or `Own(Selection)` ("Use a
different theme in this app"). Each app also states its `AppDefaults` (a theme and a mode): what it
shows while it follows the suite and the suite has no selection. Apps normally use `SUITE_DEFAULT`
(`craft.artcraft`); the pilot apps (PdfCraft, PhotoCraft, the launcher) use ArtCraft with
`Mode::Dark`, and office apps may ship Light. `AppChoice::effective(suite, &defaults)` returns the
selection that applies:

| `AppChoice` | Suite selection | Effective selection |
|---|---|---|
| `Own(sel)` | any | `sel` |
| `FollowSuite` | `Some(suite)` | `suite` |
| `FollowSuite` | none | the app's `AppDefaults` |

An app writes `selection.json` only when the user picks a suite theme or mode, never on first
start.

## Core roles

Every variant defines all 13 roles. A user theme that leaves roles out gets them from Graphite of
the same variant, and `from_maps` lists each one it filled.

| Role | Painted as |
|---|---|
| `background` | behind panels: the window, pasteboard or canvas surround |
| `surface` | panels, sidebars, controls at rest |
| `raised` | cards, popups, menus, fields |
| `text`, `text_muted` | primary and secondary text |
| `line` | decorative dividers (no contrast requirement) |
| `accent`, `on_accent` | accent fills and the text or icons on them |
| `selection` | selected rows and items, with `text` on top |
| `focus` | keyboard focus rings |
| `good`, `warn`, `bad` | status marks |

## Derived states

`derive(&roles, variant)` computes the interaction and secondary colours, mixed in encoded sRGB
(`Rgba::lerp_gamma`). Apps may overwrite any field.

| State | Rule |
|---|---|
| `hover`, `pressed` | `surface` 6% / 12% toward `text` in light, 8% / 14% in dark |
| `selected_hover` | `selection` 8% toward `text` |
| `accent_hover`, `accent_pressed` | `accent` moved away from `on_accent`'s luminance in 2% steps, up to 10% / 22%, stopping before a step that takes `on_accent` below 4.5:1 |
| `accent_text` | `accent` toward `text` until it reaches 4.5:1 on background, surface and raised |
| `accent_soft` | `accent` at 14% over `surface` |
| `text_faint` | `text_muted` toward `surface` while it stays at 4.5:1 on background, surface and raised |
| `control_edge` | `line` toward `text` until it reaches 3:1 on background, surface and raised |

Each state keeps the painted pairs it is part of at their requirement. When a step toward `text`
would break one, the state steps away from `text` by the same amount instead. For example,
ArtCraft's dark selection is mid grey, so its `selected_hover` is darker than the selection: a
lighter one would leave `text` below 4.5:1 and `text_muted` below 3.5:1. Built-in colours are never changed to suit a
derived state.

## Contrast

`contrast::painted_pairs()` lists every pair the apps paint from the roles and derived states.
Ratios are WCAG 2; a translucent foreground is composited over the background first. Text needs
4.5:1 (WCAG AA). Muted text needs 4.5:1 on the resting backgrounds and 3.5:1 on the hover, pressed
and selected state backgrounds, where it sits only while the pointer or the selection is there.

| Foreground | Background | Need |
|---|---|---|
| `text` | `background`, `surface`, `raised`, `hover`, `pressed`, `selection`, `selected_hover` | 4.5:1 |
| `text_muted` | `background`, `surface`, `raised` | 4.5:1 |
| `text_muted` | `hover`, `pressed`, `selection`, `selected_hover` | 3.5:1 |
| `text_faint` | `background`, `surface`, `raised` | 4.5:1 |
| `on_accent` | `accent`, `accent_hover`, `accent_pressed` | 4.5:1 |
| `accent_text` | `background`, `surface`, `raised` | 4.5:1 |
| `focus`, `control_edge`, `accent` | `surface`, `raised`, `background` | 3:1 |
| `good`, `warn`, `bad` | `background`, `surface` | 3:1 |

`contrast::diagnose(&theme)` returns the pairs that miss their need. Diagnostics are advisory: a
user or imported theme keeps its colours and can still be imported and selected, and the app shows
the diagnostics. The built-in themes have none; a test enforces that. `contrast::check` tests pairs
an app paints outside this table.

## Resolution

`resolve(&selection, system, fallback, &lookup)` is pure. `system` is the OS's light/dark answer
(`None` when it gives none), `fallback` is the app's variant for that case, and `lookup` finds a
theme by id (the built-ins, or the store's library).

| Selection | Result | `Fallback` note |
|---|---|---|
| `Mode::System`, OS answers | the OS's variant | none |
| `Mode::System`, OS silent | the app's fallback variant | `SystemSilent` |
| `Mode::Light` or `Mode::Dark` | that variant | none |
| `craft.app-default` | `Resolution::AppDefault` in the variant above | as above |
| an id `lookup` doesn't know | `Resolution::AppDefault` in the variant above, never another named theme | `UnknownTheme` |
| a theme with only the other variant | `Resolution::Theme` in the variant it has | `OnlyVariant` |

A `Resolved` theme carries the roles of the chosen variant and their derived states.

## Adopting suite themes in an app

There is no shared trait and no shared context key. Each app adds to its own theme module:

1. `Tokens::app_default(variant)`: its existing palette, unchanged.
2. `Tokens::from_resolved(&Resolved)`: maps the core roles and derived states onto its token fields.
3. `install(ctx, light: Tokens, dark: Tokens)`: builds its own `Style` for each token set and calls
   `craft_ui::theme::install(ctx, light_style, dark_style)`, which sets both of egui's styles and
   `ThemePreference::System`. With a fixed mode both slots get the chosen variant; with
   `Mode::System` each slot gets its own variant, and egui switches when the OS does, without a
   rebuild. The app stores both token sets in its own state, so `Tokens::get` returns the set for
   `ctx.theme()`.

On Linux and the BSDs, the portal's answer from `craft_platform::appearance::Watcher::current()`
replaces winit's in `raw.system_theme` (in the raw-input hook); `None` from the portal clears it.
Each app sets `Options::fallback_theme`. The shared crates store nothing in egui's context.

An app's saved light/dark preference becomes `Own(craft.app-default, same mode)` on adoption, so
its look doesn't change until the user picks a named theme or turns on "Follow suite". A fresh
install follows the suite and shows the app's `AppDefaults` until the user picks a suite theme.
Each app sets these defaults explicitly for a fresh install and for a legacy preference file, so a
`#[serde(default)]` never moves existing users to the suite.

Each adapter PR carries an exact style comparison (see
[theme-infrastructure.md](theme-infrastructure.md#migration-and-validation)): with
`craft.app-default` the app's `Style` and `Tokens` stay identical.

## File format and storage

`craft-theme-store` (coming) defines the theme file format, the machine-local theme library, the
suite selection and how apps see each other's changes.
