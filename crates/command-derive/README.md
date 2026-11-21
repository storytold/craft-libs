# craft-command-derive

The `#[command]` attribute macro for [`craft-command`](../command/README.md). It turns one handler function into a command row, a schema and a run wrapper.

```rust
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
struct NestParams { name: String, parent: Option<u32> }

#[command(id = "clip.nest", label = "Nest", doc = r#"{"name":str,"parent":binId?}"#, menu = ["Clip"])]
fn nest(session: &mut Session, params: NestParams) -> Result<serde_json::Value, AppError> { /* ... */ }
```

The attribute takes `id`, `label`, `doc`, `menu`, `shortcut`, `alt_shortcuts`, `enabled`, `bad_params` and `journal`. `doc` is required. The handler has the shape `fn(&mut Session, Params) -> Result<Value, AppError>`.

For a handler named `nest` the macro generates:

- `NEST_COMMAND`, a `CommandSpec` row.
- `nest_schema()`, the schema from `schemars`, marked `x-craft-source: derived`.
- `NEST_SCHEMA`, a pair of the id and that function.
- The wrapper that reads the params from JSON and calls the handler. Whether unknown keys are refused is the choice of the params type (`deny_unknown_fields`).

`CommandSpec.params` is a `&'static str` and a typed schema needs run time code, so the row keeps the doc string. The registry still serves the schema from the doc string.

The crate that uses the macro needs `craft-command`, `serde_json` and `schemars` 1.x. There are no compile-fail tests. Macro errors are plain `syn::Error` values.
