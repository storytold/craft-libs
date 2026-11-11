# craft-plugin-ui

Host-drawn plugin panels, the grants prompt and the plug-in manager windows. Plugins never touch egui.

## Architecture

- `src/tree.rs`: `Widget` (serde, tag `type`, `deny_unknown_fields`) and `WidgetTree`. Only validated trees exist. Validation walks with an explicit stack, never recursion.
- `src/render.rs`: `show(ui, &mut WidgetTree) -> Vec<UiEvent>`. Edits change the tree in place and emit events. Parameter edits use the id `<form>.<param>`.
- `src/grants.rs`: prompt and review model plus views. `Permission`, `PromptLevel` and `DocScope` are the types of `craft-plugin-abi`, re-exported. Do not add a second parser here.
- `src/manager.rs` (feature `host`, native only): `UiState`, `ManagerView`, `Act` and `draw`. The app builds the view, `draw` returns acts, the app carries them out against `PluginManager`.

## Gotchas

- Node count: a `param_form` counts 1 plus one per parameter. Change `MAX_NODES` and the count rule together.
- `draw` calls the view closure only while the manager is open. Keep the closure cheap anyway.
- Panel answers come in as `Result<Value, String>` through `open_panel_window` and `apply_panel_answer`. A missing `tree` key keeps the old tree.
- `frames.read` parses in the ABI but the prompt refuses it (`GrantError::Reserved`).
- Feature `host` must not build for wasm32 (`craft-plugin-host` lifecycle is native only). The `manager` module is gated on both.
- Tests use `egui_kittest` without `wgpu`. `tests/manager.rs` has `required-features = ["host"]`.
