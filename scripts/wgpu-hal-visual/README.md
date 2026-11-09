# DX12 presentation regression test

This Windows-only test opens a small window with **real egui text** at several
sizes and one-pixel lines. It captures the internal GPU framebuffer and the
Windows presentation, then repeats the same scene with the patched backend.
No full Craft App build, Python, Pillow or external screenshot program is needed.

The renderer versions match the original FigCraft case: eframe/egui **0.36.2**,
winit **0.30.13**, wgpu/wgpu-hal **30.0.1**, with DX12 enabled. The renderer is
deliberately pinned: a simpler raw-wgpu window did not reproduce the blur.

## Run locally

On an interactive Windows desktop with Rust and the native C++ linker installed:

```powershell
./scripts/wgpu-hal-visual/run.ps1
```

For an existing GNU Rust toolchain, put its MinGW compiler on `PATH` and use:

```powershell
./scripts/wgpu-hal-visual/run.ps1 -Toolchain stable-x86_64-pc-windows-gnu
```

Optional `-TargetDirectory` chooses the Cargo cache/build location (for example,
another drive). `-OutputDirectory` chooses a **new or empty** results directory.
Defaults use the checkout's ignored `target/wgpu-hal-visual` directory and a new
unique results folder; the script prints its location. No settings or projects
from installed Craft Apps are read, and Windows display settings are not changed.

The script builds two isolated temporary copies of the same harness:

1. `before`: the locked, published crates.io wgpu-hal 30.0.1.
2. `after`: the same dependency graph, selecting this checkout's
   `vendor/wgpu-hal` through a temporary Cargo patch.

Before launching either executable, the script verifies that every other
dependency version/source and all harness source files match, and that Cargo
actually selected the intended backend. Neither repository manifest is edited.
The Cargo lockfile pins the test's published baseline; Cargo's offline resolution
replaces only the second temporary lockfile's wgpu-hal source, with the resulting
dependency graph checked before capture.

Both executables automatically resize, maximize, restore and close their own
window. Capture has a bounded timeout and rejects incomplete or stale runs.

## Images and verdict

Each of the four state folders contains:

- `01-before.png`: Windows output without the patch.
- `02-after.png`: Windows output with the patch.
- `03-difference.png`: the after image with changed RGB pixels marked in
  **red at about 41% opacity**.

The top-level three images show the initial restored window. `before/` and
`after/` retain the original internal and Windows captures. `results.json`,
`RESULTS.md` and `provenance.json` record numerical results and what was tested.

Comparison uses the Win32-reported integer client offset and a 40-pixel edge
margin. It does **not** resize, interpolate, search for a favorable alignment or
ignore small color differences. The two internal framebuffers must be identical.

- **PASS (exit 0):** the unpatched restored/resized states reproduce the blur,
  and every patched Windows capture matches its framebuffer exactly.
- **INCONCLUSIVE (exit 2):** the unpatched bug did not reproduce in a required
  state. A zero/zero comparison is **not** evidence that this fix works.
- **FAIL (exit 1):** the patched output differs, the A/B scenes differ, capture
  failed or inputs are invalid. Inspect the images before assigning a cause.

The PowerShell runner returns a failure for either non-passing verdict, while
preserving images and reports. The maximized baseline is allowed to be sharp;
the reported issue concerns restored windows.

## GitHub Actions

The ordinary Windows job compiles the harness and runs the comparison unit tests.
The same non-graphical checks can be run locally with
`cargo test --manifest-path scripts/wgpu-hal-visual/Cargo.toml --locked`.
For an actual capture, open **Actions → Vendored wgpu-hal source → Run workflow**
and enable **visual_check**. A workflow must be available on the default branch
for manual dispatch; upstream fork runs may also require a maintainer's approval.

Images, logs and the numerical report are uploaded as **dx12-window-captures**,
including on failed or inconclusive runs, and the verdict is placed in the job
summary. Hosted runners may lack the display/driver behavior that reproduces
this bug. Such a run must not be presented as a successful regression test.

## Scope and sample result

See [the sample report](example/RESULTS.md) and [red overlay](example/03-difference.png).
The sample is a local Windows/NVIDIA RTX 3060 run at one pixel per point, not
a hosted Windows result or a universal guarantee across drivers.

This test covers settled restored/resized/maximized windows and restoration.
It does not cover live mouse-drag resizing, mixed-DPI monitor transitions or
composition targets. `PrintWindow` runs on a worker thread and captures only the
harness's own HWND; the Win32 FFI in `src/capture.rs` is test-only and is never
linked into a Craft App.

The harness and generated sample images are original test material under the
repository's MIT OR Apache-2.0 licence. No font files or app assets are added;
egui's default test font comes from its existing dependency.
