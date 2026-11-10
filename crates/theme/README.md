# craft-theme

The Craft suite's theme contract: theme identity, the 13 core colour roles of a light and a dark
variant, derived interaction states, contrast checks on the colour pairs apps paint, the built-in
themes and resolution of a selection against the OS's light/dark answer.

Pure data and functions: no egui, no I/O, no threads, and it builds for wasm. Theme files, the
suite folder and change watching belong to `craft-theme-store`; `craft_ui::theme` converts colours
for egui and installs styles. [docs/suite-themes.md](../../docs/suite-themes.md) describes the
contract and how an app adopts it.

```rust
use craft_theme::{AppChoice, AppDefaults, Mode, Resolution, ThemeId, Variant, lookup_builtin, resolve};

// What this app shows while it follows the suite and nobody has picked a suite theme.
let defaults = AppDefaults { theme: ThemeId::suite_default(), mode: Mode::Dark };
let choice = AppChoice::FollowSuite; // from the app's own preferences
let suite = None; // from craft-theme-store: no selection.json yet
let sel = choice.effective(suite, &defaults);
match resolve(&sel, Some(Variant::Dark), Variant::Light, &lookup_builtin) {
    Resolution::Theme(r) => { /* map r.roles and r.derived onto the app's tokens */ }
    Resolution::AppDefault { variant, .. } => { /* the app's own palette in `variant` */ }
}
```

| Item | What it is |
|---|---|
| `Rgba` | sRGB with unmultiplied alpha; `parse_hex`, `to_hex`, `lerp_gamma`, `over` |
| `ThemeId`, `APP_DEFAULT`, `SUITE_DEFAULT` | ids `[a-z0-9][a-z0-9._-]{1,63}`; `craft.` is reserved for built-ins, and user ids can't start with a Windows device name |
| `Role`, `CoreRoles`, `Variant`, `Mode` | the 13 roles of one variant; light/dark; System/Light/Dark |
| `SuiteTheme::from_maps` | a user theme from role maps; missing roles come from Graphite and are listed |
| `builtins()`, `builtin`, `lookup_builtin` | ArtCraft, Graphite, High contrast |
| `Selection`, `AppChoice`, `AppDefaults` | a theme and mode; an app's "follow suite" or own choice; what a following app shows before a suite theme is picked (no `Default` for any of them) |
| `resolve`, `Resolution`, `Resolved`, `Fallback` | pure resolution; unknown ids give the app default, never another theme |
| `derive`, `Derived` | hover, pressed and the other derived states, guarded to keep their contrast pairs |
| `contrast` | `ratio`, `painted_pairs`, advisory `diagnose`, `check` for app-painted pairs |

Feature `serde` adds `Serialize`/`Deserialize` to the data types: colours as `"#rrggbb[aa]"`, ids
as strings, enums in snake_case.

ArtCraft's colours come from PhotoCraft's Pro Light and Pro Medium themes, Graphite's from
PdfCraft's Light gray and Dark gray, and High contrast follows DesignCraft's high-contrast
brightness. Checking contrast on the pairs apps actually paint follows
[pdfcraft#1090](https://github.com/storytold/pdfcraft/pull/1090).
