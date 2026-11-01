# craft-blitter

CPU raster blending and Porter-Duff compositing for the Craft apps. It provides in-place slice
blitting and single-pixel compositing across `RGBA8` (`&mut [u8]`), `RGBA16` (`&mut [u16]`), and
`RGBA32F` (`&mut [[f32; 4]]`). No `unsafe`, no panics on malformed or mismatched slices, zero heap
allocations on serial paths (and on parallel paths once Rayon's global thread pool is initialized),
and it builds for `wasm32-unknown-unknown`.

```toml
craft-blitter = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## Who uses it

Six Craft applications currently maintain separate scalar blend-mode implementations for CPU
document rendering, layer previews, export flattening, and tile compositing:

- **PhotoCraft** (`crates/color/src/blend.rs`, `crates/compose`)
- **VectorCraft** (`crates/color/src/blend.rs`)
- **DesignCraft** (`crates/color/src/blend.rs`)
- **EffectCraft** (`crates/color/src/blend.rs`)
- **FigCraft** (`crates/color/src/blend.rs`)
- **FilmCraft** (`crates/render/src/blend.rs`)

`egui`'s painter only draws GPU vertex-tinted premultiplied `Normal` UI primitives; it does not
composite document raster tiles across 32 blend modes or `RGBA8` / `RGBA16` / `RGBA32F` buffers.
`craft-blitter` provides the shared CPU raster kernel layer below each app's document compositor.

## Blend modes and specification sources

Every blend mode in `BlendMode` is implemented solely from published specifications:

| Variants | Formula (`cb`, `cs` in `[0, 1]`, or unclamped where noted) | Public specification |
|---|---|---|
| `Normal` | `cs` | [W3C Compositing-1 §10.1.1](https://www.w3.org/TR/compositing-1/#blendingnormal) |
| `Multiply` | `cb * cs` | [W3C Compositing-1 §10.1.2](https://www.w3.org/TR/compositing-1/#blendingmultiply) |
| `Screen` | `cb + cs - cb * cs` | [W3C Compositing-1 §10.1.3](https://www.w3.org/TR/compositing-1/#blendingscreen) |
| `Overlay` | `HardLight(cs, cb)` | [W3C Compositing-1 §10.1.4](https://www.w3.org/TR/compositing-1/#blendingoverlay) |
| `Darken` | `min(cb, cs)` | [W3C Compositing-1 §10.1.5](https://www.w3.org/TR/compositing-1/#blendingdarken) |
| `Lighten` | `max(cb, cs)` | [W3C Compositing-1 §10.1.6](https://www.w3.org/TR/compositing-1/#blendinglighten) |
| `ColorDodge` | `0` if `cb <= 0`; `1` if `cs >= 1`; else `min(1, cb / (1 - cs))` | [W3C Compositing-1 §10.1.7](https://www.w3.org/TR/compositing-1/#blendingcolordodge) |
| `ColorBurn` | `1` if `cb >= 1`; `0` if `cs <= 0`; else `1 - min(1, (1 - cb) / cs)` | [W3C Compositing-1 §10.1.8](https://www.w3.org/TR/compositing-1/#blendingcolorburn) |
| `HardLight` | `2 * cb * cs` if `cs <= 0.5`; else `Screen(cb, 2 * cs - 1)` | [W3C Compositing-1 §10.1.9](https://www.w3.org/TR/compositing-1/#blendinghardlight) |
| `SoftLight` | W3C/PDF curve with cubic `((16 * cb - 12) * cb + 4) * cb` for `cb <= 0.25` and `sqrt(cb)` above `0.25` | [W3C Compositing-1 §10.1.10](https://www.w3.org/TR/compositing-1/#blendingsoftlight), [Adobe PDF Blend Modes Addendum (2006)](https://www.adobe.com/content/dam/acom/en/devnet/pdf/pdf_reference_archive/blend_modes.pdf) |
| `Difference` | `\|cb - cs\|` | [W3C Compositing-1 §10.1.11](https://www.w3.org/TR/compositing-1/#blendingdifference) |
| `Exclusion` | `cb + cs - 2 * cb * cs` | [W3C Compositing-1 §10.1.12](https://www.w3.org/TR/compositing-1/#blendingexclusion) |
| `Hue`, `Saturation`, `Color`, `Luminosity` | `SetLum` / `SetSat` / `ClipColor` with `Lum(C) = 0.30 R + 0.59 G + 0.11 B` (`top = 1.0` in SDR, `f32::MAX` in HDR) | [W3C Compositing-1 §10.2](https://www.w3.org/TR/compositing-1/#blendingnonseparable), [Adobe PDF Blend Modes Addendum (2006)](https://www.adobe.com/content/dam/acom/en/devnet/pdf/pdf_reference_archive/blend_modes.pdf) |
| `LinearBurn` | `max(cb + cs - 1, 0)` | [Khronos `GL_NV_blend_equation_advanced` Table X.2 (`LINEARBURN_NV`)](https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt), [OpenVG `VG_KHR_advanced_blending` Table 14](https://registry.khronos.org/OpenVG/extensions/KHR/advanced_blending.txt) |
| `LinearDodge` | `min(cb + cs, hi)` (`hi = 1.0` in SDR, `f32::MAX` in HDR) | [Khronos `GL_NV_blend_equation_advanced` Table X.2 (`LINEARDODGE_NV`, `PLUS_NV`)](https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt) |
| `VividLight` | `0` if `cs <= 0`; `1 - min(1, (1 - cb) / (2 * cs))` if `0 < cs < 0.5`; `min(1, cb / (2 * (1 - cs)))` if `0.5 <= cs < 1`; `1` if `cs >= 1` | [Khronos `GL_NV_blend_equation_advanced` Table X.2 (`VIVIDLIGHT_NV`)](https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt) |
| `LinearLight` | `clamp(cb + 2 * cs - 1, 0, 1)` | [Khronos `GL_NV_blend_equation_advanced` Table X.2 (`LINEARLIGHT_NV`)](https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt) |
| `PinLight` | `min(cb, 2 * cs)` if `cs <= 0.5`; else `max(cb, 2 * cs - 1)` | [OpenVG `VG_KHR_advanced_blending` Table 14 (`VG_BLEND_PINLIGHT_KHR`)](https://registry.khronos.org/OpenVG/extensions/KHR/advanced_blending.txt), [Adobe Blending Modes](https://helpx.adobe.com/photoshop/using/blending-modes.html) |
| `HardMix` | `0` if `cb + cs < 1`; else `1` | [Khronos `GL_NV_blend_equation_advanced` Table X.2 (`HARDMIX_NV`)](https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt) |
| `Subtract` | `max(cb - cs, 0)` | [Khronos `GL_NV_blend_equation_advanced` Table X.2 (`MINUS_CLAMPED_NV`)](https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt), [OpenVG `VG_KHR_advanced_blending` Table 14 (`VG_BLEND_SUBTRACT_KHR`)](https://registry.khronos.org/OpenVG/extensions/KHR/advanced_blending.txt) |
| `Divide` | `min(cb / cs, hi)` for `cs > 0`; finite guard `cs <= 0 -> 0` if `cb <= 0` else `1` | [Adobe Blending Modes (`Divide`)](https://helpx.adobe.com/photoshop/using/blending-modes.html) |
| `DarkerColor`, `LighterColor` | Compares RGB channel sum `R + G + B` and keeps the whole pixel with the lower or higher sum | [Adobe Blending Modes (`Darker Color`, `Lighter Color`)](https://helpx.adobe.com/photoshop/using/blending-modes.html) |
| `ClassicColorBurn`, `ClassicColorDodge` | PDF 1.4 / ISO 32000-1 Table 136 ColorBurn (`cs <= 0 -> 0`, else `max(0, 1 - (1 - cb) / cs)`) and ColorDodge (`cs >= 1 -> 1`, else `min(1, cb / (1 - cs))`) | [OpenVG `VG_KHR_advanced_blending` Table 13](https://registry.khronos.org/OpenVG/extensions/KHR/advanced_blending.txt), [Khronos `GL_NV_blend_equation_advanced` Issue 34](https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt), [Paint.NET Blend Modes](https://www.getpaint.net/doc/latest/BlendModes.html) |
| `Reflect`, `Glow`, `Negation` | `min(1, cb^2 / (1 - cs))`, `Reflect(cs, cb)`, `1 - \|1 - cb - cs\|` | [Pegtop Blend Modes](http://www.pegtop.net/delphi/articles/blendmodes/), [Paint.NET Blend Modes](https://www.getpaint.net/doc/latest/BlendModes.html) |
| `Xor` | Bitwise `cb ^ cs` (`RGBA8` / `RGBA16`; quantized to 8-bit in `RGBA32F`) | [Paint.NET Blend Modes (`Xor`)](https://www.getpaint.net/doc/latest/BlendModes.html) |

## Alpha conventions and Porter-Duff operators

- **`AlphaMode::Straight`** (default): color channels store un-premultiplied values.
- **`AlphaMode::Premultiplied`**: color channels store `color * alpha`.
- **`CompositeOp::SourceOver`** (default; `(X, Y, Z) = (1, 1, 1)` in
  [W3C Compositing-1 §9.1.2 / §10](https://www.w3.org/TR/compositing-1/) and
  [Khronos Table X.1](https://registry.khronos.org/OpenGL/extensions/NV/NV_blend_equation_advanced.txt)):
  - `ao = as + ab * (1 - as)`
  - `ao * Co = (1 - as) * ab * Cb + (1 - ab) * as * Cs + as * ab * B(Cb, Cs)`
- **`CompositeOp::SourceAtop`** (`(X, Y, Z) = (1, 0, 1)` in
  [W3C Compositing-1 §9.1.6](https://www.w3.org/TR/compositing-1/#porterduffcompositingoperators_srcatop)
  and [W3C SVG Compositing](https://www.w3.org/TR/SVGCompositing/); used for alpha-locked painting
  and clipping masks):
  - `ao = ab`
  - `ao * Co = (1 - as) * ab * Cb + as * ab * B(Cb, Cs)` (for `ab > 0`, straight-alpha
    `Co = (1 - as) * Cb + as * B(Cb, Cs)`).

## Rounding rules and determinism

- **`div255(x)`**: `((x + 128) * 257) >> 16`, which equals exact nearest-integer `round(x / 255) = (x + 127) / 255` for every integer `x` in `0..=65535` (255 is odd, so half-way ties never exist).
- **`div65535(x)`**: `(((x + 32768) * 65537) >> 32) as u32`, which equals exact nearest-integer `round(x / 65535) = (x + 32767) / 65535` for every integer `x` in `0..=65535^2`.
- **General integer ratios** (`num / denom`): rounded half-up via `(num + denom / 2) / denom`.
- **Float-to-integer quantization** (`SoftLight`, `Reflect`, `Glow`, and `Hue` / `Saturation` / `Color` / `Luminosity` in `fixed.rs`, plus `opacity`): clamped to `[0.0, 1.0]` and quantized half-up via `(v * MAX + 0.5) as u8` / `as u16`.
- **SIMD chunking vs scalar fallback**: `craft-blitter` obeys the workspace `#![forbid(unsafe_code)]` rule on stable Rust. `simd.rs` transposes 8-pixel blocks (`slice::as_chunks::<8>()`) into branchless `[f32; 8]` Structure-of-Arrays arrays on the stack for LLVM auto-vectorization (`SSE`/`AVX`, `NEON`, `wasm32` `simd128`), using the exact same IEEE-754 expression tree as `float::composite_f32` (no FMA fusion). `float::blit_rgba32f_scalar` exposes the pure scalar fallback directly, and `tests/slice_blitting.rs` verifies `to_bits()` bit-identity between the two across chunk boundaries.

## Boundaries

- **In scope**: CPU slice blitting and single-pixel blending/compositing for the 32 documented modes above across `RGBA8`, `RGBA16`, and `RGBA32F`.
- **Out of scope**: spatial dithering (`Dissolve`), layer-tree group scoping (`PassThrough`), ICC color space conversion (`craft-cms`), GPU shader pipelines, and application-specific non-standard modes.

## Testing

- `cargo test -p craft-blitter --locked`:
  - `tests/blend_modes.rs`: analytical checks for all 32 modes against the published W3C, ISO 32000, Khronos, Adobe, Pegtop, and Paint.NET equations.
  - `tests/fixed_point.rs`: exhaustive `div255` (`0..=65535`), `div65535` boundary checks, exhaustive 65,536-pair `(cb, cs)` verification of all 26 separable `RGBA8` kernels (`<= 1` LSB), and sampled `RGBA16` / non-separable RGB checks.
  - `tests/slice_blitting.rs`: `to_bits()` bit-identity between `simd` 8-lane spans, parallel `blit_rgba32f`, and `float::blit_rgba32f_scalar` across span lengths `0..=5000`; `SourceOver` and `SourceAtop` in straight and premultiplied alpha; mismatched/empty/`NaN` inputs.
  - `tests/proptests.rs`: property tests across all 32 `BlendMode` variants, both `AlphaMode`s, and both `CompositeOp`s.
- `cargo test --manifest-path crates/blitter/alloc-check/Cargo.toml --locked -- --test-threads=1`:
  - Runs a `#[global_allocator]` counting allocator asserting `0` heap allocations across cold serial blits (all 32 modes, all depths) and steady-state parallel blits (`>= 4096` pixels).

`.github/workflows/blitter.yml` runs `rustfmt`, `clippy`, the crate tests, the counting-allocator harness, and `wasm32-unknown-unknown` compilation on Linux and Windows.

## Provenance

The scalar reference structure in `src/blend.rs` and the 8-lane `normal_lanes` SoA transposition in `src/simd.rs` are adapted from PhotoCraft (`crates/color/src/blend.rs` and `crates/compose/src/psblend.rs`, MIT OR Apache-2.0; see root `NOTICE`), reduced strictly to formulas published in the specifications cited above.
