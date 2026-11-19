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
| `color::spline` | `crates/color/src/spline.rs` | `MonotoneCurve` (a tone curve through points that does not overshoot) and `Lut1`. |
| `curve` | `crates/pipeline` | The parametric tone curve and its regions. |
| `lut` | `crates/pipeline` | 3D lookup tables and the `.cube` reader. |
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

## Not here yet

The rest of the pipeline: the rest of colour, raster, the sensor stage, raw loading and the render stages. No app uses this crate
yet.
