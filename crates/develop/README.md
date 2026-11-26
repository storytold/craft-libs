# craft-develop

The raw develop pipeline, CPU path: the shared pipeline of
[#587](https://github.com/storytold/craft-libs/issues/587), moved from LightCraft one module at a time. No `unsafe`,
no panics on hostile input, no file I/O, and it builds for `wasm32-unknown-unknown`.

```toml
craft-develop = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## What is here

| Module | From LightCraft (`a75497d6`) | Contents |
|---|---|---|
| `auto` | `crates/pipeline` | Auto tone (`AutoTone`, `auto_tone`, `scene_log_mean`), auto B&W mix (`auto_bw_mix`), and auto white balance (`auto_wb`, `auto_wb_with`). |
| `color::spline` | `crates/color/src/spline.rs` | Re-exports `craft_color_transform::curve` (`MonotoneCurve`, `Lut1`). |
| `colorops` | `crates/pipeline` | OkLCh colour tools (`ColorOps`, `WheelK`, `PointK`, `band_hues`, `band_weights`, `calibration_matrix`, `calibrate`). |
| `crystal` | `crates/pipeline` | Process V7+ crystal-layer film grain (`CrystalGrain`, `CrystalLook`, `CrystalTile`). |
| `curve` | `crates/pipeline` | The parametric tone curve and its regions. |
| `finish` | `crates/pipeline` | Pure-Rust finishing helpers: sharpening masks/limits, vignette parameters, value grain, gamut mapping, and curve saturation refinement. |
| `light_v3` / `light_v3_params` | `crates/pipeline` | Process V4+ calibrated Contrast and Blacks operators (`LightOps`, `PreviewOps`, `Operator`, `Fit`, `Mode`). |
| `local` | `crates/pipeline` | White balance (`wb_matrix_for`, `checked_wb`, `effective_wb`, `legacy_white`), guided filters, median/detail-free base, noise reduction (`denoise`, `apply_nr`), and spatial plane sigmas. |
| `local_tone` | `crates/pipeline` | DNG `ProfileGainTableMap` camera local tone mapping (`LocalTone`, `apply`). |
| `lut` | `crates/pipeline` | 3D lookup tables and the `.cube` reader. |
| `nrc_joint` | `crates/pipeline` | Process V6+ joint-guided linear colour noise reduction (`JointNoise`, `denoise`). |
| `profiles` | `crates/pipeline` | Base rendering profiles (`effective`). |
| `raster` | `crates/raster` | `Image<T>` (`Rgb32f`, `Rgba8`, `Plane`), separable blur, resampling, point sampling, morphology, and `Histogram`. |
| `sensor` | `crates/raw` | Sensor normalization, DNG opcodes (`OpcodeList1/2/3`), Bayer (AHD/PPG/Bilinear) & X-Trans demosaicing, highlight reconstruction, DNG colour matrices (`color`), `ProfileGainTableMap` evaluation (`gaintable`), and binned preview development. |
| `tone` | `crates/pipeline` | Global tone mapping (`ToneMap` and `CameraTone`) from scene luminance to display-linear luminance, SDR and HDR. |
| `inpaint` | `crates/inpaint` | Content-aware hole filling for spot removal: `complete` (multi-scale patch completion with PatchMatch) and `membrane_fill` (a smooth harmonic fill, its seed and fallback). Interleaved `f32` buffers of any channel count, a fixed-seed RNG, the same result on any number of threads. |

## Parity with LightCraft

Each module comes with a test that its output is LightCraft's at the base commit, compared with
[`craft-render-parity`](../render-parity/README.md) at `Tolerance::BASELINE`. The expected pictures in
`tests/fixtures/` were written by LightCraft's own crate at that commit, on the scene the test builds.

Differences from LightCraft (ordinary-range output is covered by the baseline fixtures):

- `inpaint::complete_with` returns `None` for a patch radius wider than the image before computing with it (LightCraft
  reaches the same `None`, or overflows on a huge radius), returns an image with no pixels at once, and reports
  `Cancelled` when any one poll said so, including a poll made on a worker thread.

- `membrane_fill` scales extreme finite boundaries for the harmonic solve and bounds the result by those
  boundaries, avoiding overflow in means and neighbour sums. Known pixels remain unchanged; ordinary
  values take the original arithmetic path. This does not promise arbitrary extreme-value safety for
  PatchMatch completion, which also computes squared costs and downsamples its input.
- Membrane cell indices retain `usize` width. PatchMatch rejects grids whose signed search offsets,
  summed-area counts or padded table cannot be represented; patch-area arithmetic no longer overflows
  `i32`. An already-cancelled oversized-patch request returns `Cancelled`, not an unavailable-patch result.

## Limits

This crate includes the sensor stage and consumes decoded images from `craft-raw`. Raw loading
and end-to-end render orchestration remain outside it.

For integer DNGs with a `LinearizationTable` and a nonempty `OpcodeList1`, `craft-raw` returns the
table unapplied. Shared sensor development and binned previews apply it after `OpcodeList1`,
before black/white normalisation and `OpcodeList2`. Already-linearised and floating-point
samples are not remapped.
