# Craft Apps patch to wgpu-hal 30.0.1

Source: https://crates.io/crates/wgpu-hal/30.0.1 (MIT OR Apache-2.0; upstream licence files are retained here).

Unpatched `wgpu-hal-30.0.1.crate` SHA-256:
`b6b7fb58561a792bc237628ba0792e332de418fefe145f13b5ed8201e6d52f58`.
The exact source difference is [`dx12-window-scaling.patch`](dx12-window-scaling.patch).
Run `python scripts/verify_wgpu_hal.py PATH/TO/wgpu-hal-30.0.1.crate` from the
craft-libs root to check the archive hash, every vendored archive file and the
patch. The script tolerates Git's CRLF checkout conversion but no source changes.

The `Vendored wgpu-hal source` workflow also runs the vendored library's tests
with only the `dx12` backend enabled and builds its `halmark` window example on
`windows-latest`. It targets this manifest directly because the vendor crate is
excluded from the workspace; the workspace's existing Windows jobs do not test
this copy. Reproduce the checks with:

```sh
cargo test --manifest-path vendor/wgpu-hal/Cargo.toml --locked --no-default-features --features dx12 --lib
cargo build --manifest-path vendor/wgpu-hal/Cargo.toml --locked --no-default-features --features dx12 --example halmark
```

These checks compile the HWND and composition paths and run upstream unit tests.
They do not launch a window or measure presentation sharpness. The consumer
measurements below remain separate evidence; live mouse-drag resizing,
mixed-DPI monitor transitions and composition presentation still need validation
on an interactive Windows desktop.

For an actual framebuffer/presentation regression, use the small
[egui visual test](../../scripts/wgpu-hal-visual/README.md). It automatically
builds the published and patched backends, captures four window states and
marks differences with a translucent red overlay. A non-reproducing baseline
is reported as inconclusive rather than passing. The workflow's opt-in
`visual_check` run retains screenshots as artifacts.

The DX12 swap-chain description uses `DXGI_SCALING_NONE` for HWND presentation
to avoid resampling the windowed UI. Composition swap chains retain
`DXGI_SCALING_STRETCH`, which Microsoft requires for
[`CreateSwapChainForComposition`](https://learn.microsoft.com/windows/win32/api/dxgi1_2/nf-dxgi1_2-idxgifactory2-createswapchainforcomposition).
This affects any Craft App using the patched `wgpu-hal` version. FigCraft
[issue #778](https://github.com/storytold/figcraft/issues/778) has the
reproduction and measurements. PhotoCraft's matching
[PR #7177](https://github.com/storytold/photocraft/pull/7177) reports the same
restored-window blur and a pixel-for-pixel match after this DX12 change at
100%, 125% and 150% Windows scale. Its comparison before the patch was measured
at 100%; the higher-scale samples checked only the patched build.

Upstream tracking: [gfx-rs/wgpu#10196](https://github.com/gfx-rs/wgpu/issues/10196)
identifies DX12's hard-coded scaling and discusses making it configurable. Its
reported Vulkan symptom is different from the DX12 blur here.

Remove this copy when a released `wgpu-hal` supports non-stretch DX12 HWND
presentation (or exposes a scaling mode Craft Apps can select), and affected
apps have upgraded to and verified that release. Revisit the patch on every
`wgpu-hal` version bump: a Cargo patch for 30.0.1 can silently stop applying
when the dependency graph moves to a newer version.

The proposed upstream `SurfaceScalingMode::Auto` currently prefers `Stretch`.
If that API ships unchanged, FigCraft and PhotoCraft must explicitly select
`OneToOne` and verify sharp restored windows before removing this patch;
upgrading wgpu alone would restore the blur.

While an app pins this source through `[patch.crates-io]`, its lockfile no longer
contains the registry `source` and `checksum` for `wgpu-hal`. The archive hash
and verification command above provide an explicit integrity check for the
vendored copy until the patch is removed.
