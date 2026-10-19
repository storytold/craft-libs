# GPU backends

| Crate | Role |
|-------|------|
| `craft-gpu` | Traits, `GpuJob`, `ProviderRegistry` |
| `craft-gpu-wgpu` | wgpu → WebGPU (wasm) / Vulkan·DX12·Metal (native) |
| `craft-gpu-null` | In-memory context for CI |

Feature flags on `craft-gpu-wgpu`:

- `adapter-probe` — native only; uses pollster to `request_adapter` and fill real [`DeviceCaps`](../crates/gpu/src/caps.rs) (falls back when no GPU, e.g. CI).
- `native-vulkan`, `native-dx12`, `native-metal`, `native-gles` — enable the matching wgpu backend.

On `wasm32`, wgpu is built with the `webgpu` feature; caps probing stays browser-default until async init lands in the worker.
