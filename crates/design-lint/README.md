# craft-design-lint

Design-system lint for Craft apps. Two halves:

- **clippy**: `clippy.template.toml` lists the stock egui widgets and raw colour constructors apps must not call. `check-clippy` fails unless the app's `clippy.toml` carries every entry and no other `clippy.toml` (a package directory or a parent below the root) or `CLIPPY_CONF_DIR` makes clippy read a different file. `clippy --write` merges the template in.
- **syntax**: `check` parses the configured directories with `syn` (each file once) and reports literal sizes, named colour constants, style mutation and crate-wide clippy allows.

```
craft-design-lint check [--config design-lint.toml] [--summary] [--deny-unused] [--baseline FILE | --write-baseline FILE]
craft-design-lint clippy [--write] [--clippy-toml PATH]
craft-design-lint check-clippy [--clippy-toml PATH]
```

Exit code: 0 clean, 1 findings (or a rise over the baseline, or clippy drift or shadowing), 2 usage, configuration or I/O error.

craft-libs itself: `cargo run --release -p craft-design-lint -- check --config design-lint.toml --summary` (the root config lists the framework and family UI paths).

## Config

```toml
[lint]
paths = ["src"]          # relative to the config; tests are skipped unless include-tests = true
deny-unused = true       # marker/unused and exemption/unused become errors

[[exempt]]               # path-wide allowance
path = "src/canvas"
rules = ["literal-size"] # ids or families; omit for every rule except parse
reason = "document-space drawing"
```

`warn = ["props/shadow"]` in `[lint]` reports the named rules (ids or families) as warnings, so an app's config can phase a rule in; craft-libs leaves it empty, so `props/shadow` is an error there.

`parse` findings are never exempt. An exemption that matches no checked file or silences no finding is reported as `exemption/unused`. The report lists every exemption with its rules and reason.

## Rules

| Rule | Flags |
|---|---|
| `literal-size/vec2` | `vec2`, `Vec2::new/splat`, `Vec2 { x, y }` with a literal |
| `literal-size/add-space` | `.add_space(lit)` |
| `literal-size/corner-radius` | `CornerRadius::same/from`, struct form, `.corner_radius`, `theme::corner_radius`, the radius argument of `.rect`, `.rect_filled`, `.rect_stroke`, `Shape::rect_*`, `RectShape::new/filled/stroke` |
| `literal-size/stroke` | `Stroke::new`, `PathStroke::new`, struct form, `theme::stroke` (width); `(width, colour)` tuples passed to `.stroke`, `.rect*`, `.line_segment`, `.circle*`, `Stroke::from`, or `.into()` |
| `literal-size/margin` | `Margin::same/symmetric/from`, struct form, `.inner_margin`, `.outer_margin`, `theme::margin`, `theme::margin_x` |
| `literal-size/font-id` | `FontId::proportional/monospace/new` (size); `.size(lit)` on a `RichText::..` chain |
| `raw-colour/const` | `Color32::WHITE` and the other named constants; `Rgba::{WHITE, BLACK, RED, GREEN, BLUE}` |
| `style-mutation` | `style_mut`, `spacing_mut`, `visuals_mut`, `set_style`, `set_visuals`, `set_zoom_factor`, `override_*` |
| `tooltip/stock` | `on_hover_text`, `on_disabled_hover_text`, `on_hover_ui`, `on_disabled_hover_ui`, `on_hover_text_at_pointer`, `on_hover_ui_at_pointer`, `show_tooltip_text`, `show_tooltip_ui`, `egui::Tooltip::for_enabled/for_disabled/for_widget/always_open`; the message names the `craft_ui::contract` call that replaces it (`Props::tooltip`, or `Show::show_with_tip` for a tooltip with a body). craft-ui's own tooltip module is not in the paths an app lints |
| `clippy/inner-allow` | `#![allow(clippy::disallowed_*)]` or `#![expect(..)]` in any scope; list the file in an `[[exempt]]` to accept it. Outer `#[allow]`/`#[expect]` of those lints are counted in the summary line. |

## Egui-ban and API rules

`check_with_scope` runs these beside the rules above; `check` does not. Each rule is a module under `src/contract/`, listed once in `contract/mod.rs` (`REGISTRY` and `IDS`). "Allowed in" locations come from a `RuleScope` (the per-crate layer map implements it; `StaticScope` is the in-memory form). A permission key is a rule id, or `rule:sub` for a sub-permission (`contract/platform:target`, `contract/motion:clock`); a rule id does not cover its sub-permissions.

| Rule | Flags |
|---|---|
| `contract/egui-import` | egui paths into memory, layers, `Order`, AccessKit, options, input and pass control; `memory`, `options`, `input_mut` closures, `begin_pass`, `end_pass`, `add_plugin`, `move_focus` |
| `contract/runtime-egui` | a layer 5 or 6 module naming `runtime::egui` |
| `contract/text-edit` | `runtime::egui::text_edit` outside `primitives::text_adapter` (and the adapter's own module and the test kit) |
| `contract/allocate` | `allocate_*`, `interact`, `interact_opt`, `put`, `add_sized`, `new_child` with `max_rect` |
| `contract/egui-widget` | `button`, `checkbox`, `selectable_*`, `radio*`, `hyperlink*`, `separator`, `image`, `ui.label`, `heading`, `monospace`, `colored_label`, `code`, `small`, `text_edit_*` and egui `RichText`/`Label` in layer-mapped modules, egui `Button`, `TextEdit`, `DragValue`, `Slider`, `ComboBox` constructors, `add_space` with an argument that is neither a literal nor a token read |
| `contract/access` | `widget_info`, `labelled_by`, `accesskit_*`, `accessibility_*`, live settings |
| `contract/layers` | `Area`, `Popup`, `Modal` calls, `Order`, `move_to_top`, `set_sublayer`, `set_transform_layer`, `on_hover_text`, `menu_button`, `context_menu`; `swallow_outside_press` in every file |
| `contract/memory` | `get_temp`, `insert_temp`, `remove_temp*`, `data_mut`, `data`/`scope` closures, `make_persistent_id`, `next_auto_id`, `push_id`, `Id::unique`, `UiBuilder::id_salt`, `scope_builder` with an id salt |
| `contract/keys` | `key_pressed`, `key_down`, `key_released`, `consume_key`, `consume_shortcut`, egui `Event::Key`, `Event::Text`, `Event::Ime` |
| `contract/pointer` | drag and click queries, `interact_pointer_pos`, `press_origin`, `dragged_id` |
| `contract/motion` | `animate_*`, `request_repaint_after`, `animation_time`, reduced-motion reads; `Instant::now` and `SystemTime::now` (`contract/motion:clock`) |
| `contract/containers` | egui panels, `Window`, `CollapsingHeader`, `Resize`, `TableBuilder`, `show_rows`, `show_viewport`; `ScrollArea` in every file |
| `contract/platform` | `cfg(target_os ..)` and `cfg!` on the OS or wasm (`contract/platform:target`; an item-level `cfg(target_arch = "wasm32")` gate on what the web lacks is allowed); `copy_text`; `ctx.open_url` and `egui::OpenUrl` by path, so `Cx::open_url` is not flagged |
| `test/layout`, `test/kit`, `test/harness`, `test/cfg-test`, `test/literal-size`, `test/ignored` | integration tests outside `tests/it/` in an egui-linked crate; `egui_kittest` outside the kit; `#[test]` in a `harness = false` target; `#[cfg(test)]` modules in an egui-linked crate; float literals asserted in a test that shows a component or reads tokens (not in layer 0 or 1); `#[ignore]`d tests. They read test code, so the config sets `include-tests` |
| `api/non-exhaustive` | `pub` enums, struct-like variants and structs with `pub` fields without `#[non_exhaustive]`, unless on the scope's closed list; private modules, `#[doc(hidden)]` items and test code are not public API |
| `behavior/pipeline-only` | in layers 5 to 7, a `use` of the behaviours the pipeline runs: `PressMachine` and the other press-lifecycle types, `Gesture`, the focus-scope builders, `TypingRun`. Allowed in `craft_ui::{contract, primitives, edit}` |
| `props/reserved-setter`, `props/hand-setter` | in layers 5 to 8, an inherent builder method on a component named like a `Props` setter; one that only stores its argument in one field |
| `props/own-field`, `props/shadow`, `props/point-size` | a component's own `enabled`, `id_salt` or `tooltip` field; a `#[prop]` field whose setter is named like a `Props` setter (an error; `warn` makes it a warning); in layers 5 to 8, a size in points as a `#[prop]` field or a builder argument of a component: `f32`, `f64`, `Vec2` (also in an `Option`), a newtype named `Px`, `Points`, `Pt`, `Size`, `Width` or `Height`, an integer whose name has a size word (`width`, `height`, `size`, `side`, `gap`, ..), and an enum variant `Px`, `Points` or `Pt` that carries a number. A number that is not a size declares `#[prop(quantity = "..")]` |
| `style/state-pick`, `style/match-state`, `style/states-field`, `style/size-arm`, `style/token-read`, `style/instance-setter`, `style/to-egui` | in layers 5 to 8: an `if` on response state that picks a colour; a `match` arm on a `State` variant; `States<_>` or `*_hover`, `*_pressed`, `selected_*` fields in a `*Style` or `*Look` struct; a `match` arm on a `ControlSize` variant; `.tokens()`, `.token_group()`, `Tokens::of` outside a function that takes a `StyleCx` or is named `style`; a by-value builder named `style`, `look`, `colors` and the like or taking a `*Style` or `*Look`; `.to_egui()` inside such a style function. Allowed in `craft_ui::style` |
| `contract/entry` | in layers 5 and 6 (not layer 8): an inherent `pub fn show`, `show_at`, `show_in` or `ui` on a `pub` type that the file does not make a `Component` or `Pattern`; a `pub fn` taking `&mut Ui` and returning a `*Response` |
| `contract/shadow`, `contract/widget`, `contract/overlay-flag` | an inherent `show`, `show_at`, `show_in`, `show_with_tip` or `show_at_with_tip` on a component; `impl egui::Widget`; an `overlay` override on a type without `#[component(overlay)]`. Derive output is the only allowed place, so no file is |
| `contract/registration` | an `impl Component for` or `impl Pattern for` a type defined in the file that neither derives that trait nor appears in a `register_contract!(..)` invocation of the file. |

Egui paths are resolved through the file's `use` items (one table per file, grouped, renamed and glob imports), so a craft type that shares a name with an egui type is not flagged unless it is imported from egui.

Names are resolved through the file's `use` items, per module scope: grouped imports, renames (`use egui::Margin as M`), module aliases (`use crate::theme as t`) and glob imports (`use crate::theme::*`) all reach the rule table, so `margin(4.0)` is flagged after `use crate::theme::margin`. A bare name that a fn in the same module defines is not extended through globs. Imports in a module do not apply in sibling modules.

A literal is a number, optionally negated, parenthesised, cast or in arithmetic; zero is never flagged. A name is flagged when a `const` or `static` in the same file with that name has such a value. Consts are keyed by module and impl scope: a bare name takes the innermost scope that defines it, `Self::X`, `self::X`, `super::X`, `crate::X`, `Type::X` and `module::X` (last two segments) the scope they name, and a name with several candidates is flagged when any is non-zero.

## Markers

| Form | Covers |
|---|---|
| `// design: allow(reason)` at the end of a line | findings starting on that line |
| same, alone on a line | the next statement, match arm, field initialiser or item. For a `fn`, `impl`, `trait`, `mod`, `if`, `for`, `while`, `loop`, `match` or block it covers the header up to the opening brace, not the body |
| `//! design: allow(reason)` | the whole file |
| `allow[rule, family/..](reason)` | the same range, only for the listed rules |

`parse` and `clippy/inner-allow` findings cannot be covered by a marker. Markers with a short reason are `marker/empty`; unknown rules or an unclosed list are `marker/invalid`; a marker that covers nothing is `marker/unused`.

## Baseline

An existing code base can gate on new findings only:

```
craft-design-lint check --write-baseline design-lint-baseline.toml
craft-design-lint check --baseline design-lint-baseline.toml
```

The baseline stores error-finding counts per file and rule. The check fails when a count rises (a new file or rule counts from zero) and prints the findings of the rising files and rules. A file that does not parse always fails, and no baseline is written while one exists.

## Workspace check

`craft-design-lint structure --config design-lint.toml` from the repository root runs the structure checks and, over the config's paths, every rule of the egui-ban and API tables. The layer map is the rules' scope: a file's layer comes from the map, "allowed in" locations from spec 24.1 (`runtime::egui` and its submodules), egui linkage from `cargo metadata` dependencies, and `harness = false` targets from the `[[test]]` tables. Exit code 1 on any error. `cargo test -p craft-design-lint --test workspace` runs the same check.

## Structure checks

`craft-design-lint structure [--root DIR] [--config design-lint.toml]` runs the repository-wide checks of spec chapter 24 from `cargo metadata --no-deps` and the files under `design/`. Every file starts with `config_version = 1`. Findings are rustc-style lines; exit code 1 on any error.

| Rule | Reports |
|---|---|
| `layer/map` | a malformed layer file: module outside the crate's range, duplicate path, bad internal range, `row` misuse, a map for a crate that does not exist |
| `layer/unmapped` | a module of a multi-layer crate with no entry |
| `layer/import` | a `use` or qualified path whose defining module (through `pub use` re-exports and globs, across crates) is in a higher layer |
| `layer/internal` | a use from a layer above an internal module's range |
| `exception/unlisted`, `exception/stale`, `exception/expired`, `exception/invalid` | sites without an entry or beyond its count; entries listing more than exist (the ratchet: counts fall, never rise); entries whose `expires` milestone landed; malformed entries |
| `graph/layer`, `graph/tokens-services`, `graph/egui` | normal dependencies against the layer ranges; `graph/egui` covers layers 0 and 1 (egui family, winit, wgpu; `accesskit` in layer 0) |
| `features/gate` | a Cargo feature of a mapped crate that gates no optional dependency, directly or through a workspace crate's feature |
| `clone/baseline`, `clone/slack` | a family's duplicated tokens above or below its baseline |

`graph/*` and `features/gate` breaches are excused by exception entries with the same rule and the subject as `path` (`craft-window->egui`, `<crate>/<feature>`).

### Layer map: `design/layers/<crate>/`

```toml
# crate.toml
config_version = 1
lowest = 2          # with highest; or `outside = true` for a crate outside the layers
highest = 7
# row = "tokens"    # layer-0 crates only: "tokens" or "services"
```

```toml
# <family>.toml (any file name but crate.toml)
config_version = 1

[[module]]
path = "primitives::sense"   # relative to the crate root; `crate` is the root file only
layer = 3                    # or `outside = true`
internal = [2, 4]            # optional: layers above 4 may not use it
```

The longest matching path wins; a crate with `lowest == highest` needs no entries. The root file takes the highest layer unless `crate` is mapped.

### Exceptions: `design/exceptions/<crate>/<family>.toml`, `design/exceptions/milestones.toml`

```toml
config_version = 1

[[exception]]
name = "window-egui-dependency"   # unique across all files
kind = "dependency"               # marker, exempt, expect, probe, bridge, shim, pointer-only, canvas-tool, menu-entry, dependency, feature
rule = "graph/egui"               # rule id, family or `*`
path = "craft-window->egui"       # file, directory or subject; omit for the whole crate
reason = "native window code calls egui until the cut-over"
count = 1                         # default 1
expires = "cutover-3"             # required for bridge and shim
```

`milestones.toml` holds `config_version = 1` and `done = ["cutover-1"]`. Sites come from `// design: allow[rule](reason)` markers, `#[expect]`/`#[allow]` of `clippy::disallowed_*` lints, `[[exempt]]` entries of the `--config` file, and the graph and features checks.

### Clone baselines: `design/clones/<crate>/<family>.toml`

```toml
config_version = 1
paths = ["crates/ui/src/buttons"]
duplicated-tokens = 0   # default 0
```
