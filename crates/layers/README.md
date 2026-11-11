# craft-layers

The Layers panel for Craft apps. The app implements `LayerTree`, a read-only view of its layers; `LayersPanel` shows it and reports what the user did as events in the order they happened. The app applies them with its own commands and undo.

| Module (`craft_layers::ui::…`) | What it gives the app |
| --- | --- |
| `tree` | `LayerTree`, the one trait an app implements: children and parent, one `LayerInfo` per row (name, kind, visibility, label colour, locks, clipping, link, blend label, masks, effects, warning), the selection and target, sub-rows, the active layer's properties, the blend modes (`Entry<String>` lists), the footer's state and the drop rules. |
| `panel` | `LayersPanel`: the filter bar, the blend and opacity row, the lock and fill row, the rows, the footer; one focus stop, keyboard, rename in place (double-click on the name, F2 or `start_rename`), drag and drop, AccessKit. `LayersEvent`s out; `panel_menu` and `run_command` for the panel menu; `LayersPanel::style(&UiTokens)` gives the look. |
| `rows` | `BlendRows`: Blend mode, Opacity, Lock and Fill as one block, usable alone (a Properties panel). |
| `filter` | `LayerFilter` and `LayerFilterBar`: filter by kind, name or blend mode; the matching rule is a plain function. |
| `options` | `LayersSettings` (the persisted section), `LayersOptions` and `LayersOptionsDialog` (Layers Panel Options: thumbnail size and contents, the five switches). |
| `thumbs` | `LayerThumb`, `LayerThumbKind`, `ThumbContents`: the key the panel gives `craft_ui::primitives::textures::Thumbnails` for each thumbnail of a row; the app implements `ThumbSource` for it. |

Its words are the `words!` structs of the parts: `LayersPanelWords`, `BlendRowsWords`, `LayerFilterWords` and `LayersOptionsWords`.

The crate is UI-only: egui and craft-ui are plain dependencies, and `screen-testing` is its only feature.
