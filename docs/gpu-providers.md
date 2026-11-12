# GPU providers

Provider ids:

- `craft.*`: shared ops in **craft-gpu-providers** (no app crate deps)
- `craft.noop`, `craft.test.echo`, `craft.mask`, `craft.composite.premul`, `craft.readback`: registered by [`register_all()`](../crates/gpu-providers/src/lib.rs) for worker/runtime init
  - `craft.mask`: CPU stand-in for FilmCraft mask composite until WGSL lands on wgpu
  - `craft.composite.premul`: CPU premultiplied "over" (Normal path from `composite.wgsl`) until WGSL lands on wgpu
  - `craft.readback`: CPU decode of staging buffers (`f16` RGBA → `f32`, or `fmt:u8` passthrough); uses [`chunk_buffer`](../crates/gpu/src/chunk.rs) against device caps and [`f16_to_f32*`](../crates/gpu/src/readback.rs) from **craft-gpu**
- `{app}.*`: registered by application wasm at init

Generic worker code must only call `ProviderRegistry::dispatch`: never import app gpu crates.

Dependency lint: **craft-gpu-providers** must not depend on `effectcraft-*`, `filmcraft-*`, or `*-engine`.

## Readback is async underneath; these providers don't see it

`GpuContext::read_buffer`, the method every provider above calls, is a blocking convenience over the trait's real primitives, `request_read` + `poll_reads` (see [gpu-backends.md](./gpu-backends.md)). Nothing in this crate changed for that. `craft-gpu-null` still resolves `read_buffer` immediately. `craft-gpu-wgpu`'s native path now does too, blocking on the device between polls. `crates/gpu-wgpu/src/tests.rs` runs these same providers (`craft.test.echo`, `craft.composite.premul`, `craft.readback`) against a real `WgpuContext` as well as the null one, when a real adapter is available (RFC-0022 requirement 2). A provider dispatching a real WGSL job will be the one that actually needs the non-blocking `request_read`/`poll_reads` shape directly. On `wasm32` today `read_buffer` serves the staging read, the only backing on that target. A read that did not resolve synchronously faults. A future real device backing there will refuse blocking outright, per `gpu-backends.md`.
