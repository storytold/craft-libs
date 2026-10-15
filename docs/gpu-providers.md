# GPU providers

Provider ids:

- `craft.*` — shared ops in **craft-gpu-providers** (no app crate deps)
- `craft.noop`, `craft.test.echo`, `craft.mask`, `craft.composite.premul`, `craft.readback` — registered by [`register_all()`](../../crates/gpu-providers/src/lib.rs) for worker/runtime init
  - `craft.mask` — CPU stand-in for FilmCraft mask composite until WGSL lands on wgpu
  - `craft.composite.premul` — CPU premultiplied "over" (Normal path from `composite.wgsl`) until WGSL lands on wgpu
  - `craft.readback` — CPU decode of staging buffers (`f16` RGBA → `f32`, or `fmt:u8` passthrough); uses [`chunk_buffer`](../../crates/gpu/src/chunk.rs) against device caps and [`f16_to_f32*`](../../crates/gpu/src/readback.rs) from **craft-gpu**
- `{app}.*` — registered by application wasm at init

Generic worker code must only call `ProviderRegistry::dispatch` — never import app gpu crates.

Dependency lint: **craft-gpu-providers** must not depend on `effectcraft-*`, `filmcraft-*`, or `*-engine`.
