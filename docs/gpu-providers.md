# GPU providers

Provider ids:

- `craft.*` — shared ops in **craft-gpu-providers** (no app crate deps)
- `craft.noop`, `craft.test.echo` — registered by [`register_all()`](../../crates/gpu-providers/src/lib.rs) for worker/runtime init
- `{app}.*` — registered by application wasm at init

Generic worker code must only call `ProviderRegistry::dispatch` — never import app gpu crates.

Dependency lint: **craft-gpu-providers** must not depend on `effectcraft-*`, `filmcraft-*`, or `*-engine`.
