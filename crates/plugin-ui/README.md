# craft-plugin-ui

`craft-plugin-ui` draws the user interface of craft-abi 1 plugins with egui. A plugin never touches egui. It returns a JSON widget tree, and the host validates the tree and draws it. The crate depends on `craft-plugin-abi` and `egui`, on no app, and it builds for `wasm32`. The feature `host` adds the plug-in manager windows and depends on `craft-plugin-host`, which is native only.

## Boundaries

- Never crashes: no panics, no `unsafe`. A bad tree returns a `TreeError`.
- Only a validated tree can be drawn. `WidgetTree::from_json` and `WidgetTree::new` are the only constructors.
- The plugin owns the content. The host owns styling and fonts.
- Permission levels and wire forms come from `craft-plugin-abi`. This crate adds the words and the egui view.
- Meaning never rests on color alone. Every level has a text tag.

## Modules

- `tree`: `Widget` (serde, tag `type`) and `WidgetTree`. A widget is `column`, `row`, `group`, `label`, `button`, `checkbox`, `slider`, `number`, `text`, `dropdown`, `color`, `list`, `progress`, `separator` or `param_form`. Every widget has an `id`. Limits: 2000 nodes (a `param_form` counts 1 plus its parameters), depth 32, ids unique and at most 128 bytes, text at most 64 KiB, 10000 items per list or dropdown, 1024 parameters per form, finite numbers. Text with a control, bidi or zero-width character is refused. Newline and tab stay. Validation uses an explicit stack, so a deep tree cannot overflow the call stack.
- `render`: `show(ui, &mut WidgetTree) -> Vec<UiEvent>`. Edits apply to the tree in place and each one emits a `UiEvent`. A parameter edit in a `param_form` has the id `<form>.<param>`.
- `grants`: `parse_permission`, `describe`, `level_tag`, `GrantPrompt` (the install-time prompt as plain data), `GrantsReview` (the grants a plugin holds, each with Revoke), and the views `show_prompt` and `show_review`. `frames.read` parses in the ABI but no op uses it, so the prompt refuses it.
- `manager` (feature `host`, native only): `UiState`, `ManagerView` and `draw`. They show the plug-in list, the permission prompt window and the panel windows.

## Use

A panel call returns a tree. The app draws it and sends the events back to the plugin.

```rust
use craft_plugin_ui::{WidgetTree, show};

let mut tree = WidgetTree::from_json(r#"{"type":"column","id":"root","children":[
    {"type":"checkbox","id":"on","label":"Enabled","value":true}
]}"#)?;
// Inside an egui frame:
for event in show(ui, &mut tree) {
    // forward `event` to the plugin
}
```

With feature `host`, the app owns a `UiState`, calls `draw(ctx, &mut state, || view)` once per frame and carries out the returned `Act`s against its `PluginManager`. `draw` builds the `ManagerView` only while the manager window is open.

## Tests

Tests use `egui_kittest` without the `wgpu` feature, so they need no GPU and no display. They drive a headless context and check the events, the `Act`s and the text on screen. `tests/manager.rs` needs `--features host`. No test opens a real window.
