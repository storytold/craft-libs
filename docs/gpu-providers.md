# GPU providers

Provider ids:

- `craft.*` — shared ops in **craft-gpu-providers** (no app crate deps)
- `craft.noop`, `craft.test.echo`, `craft.mask` — registered by [`register_all()`](../../crates/gpu-providers/src/lib.rs) for worker/runtime init (`craft.mask` is a CPU stand-in for FilmCraft mask composite until WGSL lands on wgpu)
- `{app}.*` — registered by application wasm at init

Generic worker code must only call `ProviderRegistry::dispatch` — never import app gpu crates.

Dependency lint: **craft-gpu-providers** must not depend on `effectcraft-*`, `filmcraft-*`, or `*-engine`.
