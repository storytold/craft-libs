# craft-pbr

Image-based lighting for physically based rendering, computed on the CPU. An `EnvImage` is a linear
RGB equirect (latitude-longitude) environment, row-major with the top row first. The crate turns it
into a GGX-prefiltered chain for specular light and nine spherical-harmonics coefficients for
diffuse light, and renders six procedural environments with no image assets. It also evaluates the
glTF 2.0 metallic-roughness BRDF for direct light and the environment BRDF term of the split sum.
It has no UI, does no file or network I/O and has no `unsafe` code.

- **Frame.** +Y is up. Column `u = φ / 2π` and row `v = θ / π` map to the direction
  `(sin θ cos φ, cos θ, sin θ sin φ)`: row 0 is straight up, `u = 0` points to +X and `u` increases
  toward +Z. Lookups wrap horizontally and clamp vertically. `dir_of` and `uv_of` convert between
  the two. An image in another frame is converted by the caller first: an OpenEXR
  latitude-longitude image puts longitude 0 at +Z
  (<https://openexr.com/en/latest/ReadingAndWritingImageFiles.html>).
- **Construction.** `EnvImage::try_from_linear` takes linear radiance and returns an error for a
  NaN or negative value or one above `MAX_RADIANCE` (1e28, which keeps every sum of samples
  within `f32`). `EnvImage::try_from_rgba` takes RGBA pixels, linear or sRGB
  (`ColorEncoding`), replaces NaN, infinite and negative values with 0, caps values at 65,000 and
  decodes sRGB with `craft_texture::srgb_to_linear`. Neither changes the size. `try_half` halves
  each side by box averaging; the last column or row of an odd side is kept in the average.
  `try_resampled` resamples by bilinear lookup at each pixel's direction. `sample` is a bilinear
  lookup by direction.
- **Specular.** `try_prefilter` returns `PrefilterOptions::levels` levels (default 6). Level 0 is
  a copy of the source; level `l` of `n` is filtered for roughness `l / (n - 1)` and halves each
  side of the level before, keeping at least 4 × 2 pixels and never growing. The filtering is the
  split-sum prefilter of Karis, *Real Shading in Unreal Engine 4* (SIGGRAPH 2013,
  <https://de45xmedrsdbp.cloudfront.net/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf>):
  view and reflection directions equal to the normal, GGX importance samples from Hammersley
  points (`samples`, default 128), each weighted by `n·l`. The GGX distribution uses
  `α = roughness²`, as glTF 2.0 section B.2.3 defines it
  (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#microfacet-surfaces>), and
  `ggx_distribution` evaluates the distribution as section B.3.2 writes it
  (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#specular-brdf>).
  `ggx_distribution_between` evaluates it between a normal and a half vector, taking `(n·h)²` and
  `|n × h|²` from the vectors in `f64`, so it stays exact where `n·h` is close to 1.
  Samples read a box-filtered pyramid of the source at a level chosen from each sample's density plus `lod_bias` (default 1). `firefly_cap` caps source radiance before
  filtering when set (default off); level 0 is never capped.
- **Environment BRDF.** `try_environment_brdf(n_dot_v, roughness)` returns the `[scale, bias]` of
  the split sum's second term: specular light is a prefiltered level's radiance times
  `f0 × scale + bias`. The values are Karis's analytic fit for mobile
  (<https://www.unrealengine.com/en-US/blog/physically-based-shading-on-mobile>), with `scale` and
  `bias` kept at least 0 and their sum at most 1. `n_dot_v` is clamped to `0..=1`.
- **Direct light.** `Material::try_new(base_color, metallic, roughness)` takes linear base color,
  metallic and perceptual roughness, each from 0 to 1 as glTF 2.0 defines them, and
  `try_with_weights(specular, diffuse)` sets two weights for direct light: `specular` (0 to
  `MAX_SPECULAR`, 25) scales the dielectric reflectance at normal incidence, 0.04, and `diffuse`
  (0 to 1) scales the diffuse lobe. Both are 1 for a glTF material. Other values, NaN and
  infinity give `PbrError::BadMaterial`. `try_direct(n, v, l)` returns the BRDF times `n·l` times
  π, the fraction of a light's irradiance reflected toward the viewer, per RGB channel. The BRDF
  is the final form of glTF 2.0 section B.3.5
  (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#metal-brdf-and-dielectric-brdf>):
  `f0` is 0.04 mixed toward the base color by metallic, `F` is Schlick's approximation at `v·h`
  (section B.3.4), the diffuse lobe is `(1 − F) (1 − metallic) base / π`, and the specular lobe
  is `F D V`, with the `D` of `ggx_distribution_between` from the normal and the half vector and
  the visibility `V = G / (4 (n·v) (n·l))`.
  - `Material::with_visibility` selects the geometry term, a `Visibility`:
    - `SmithSchlick`, the default and EffectCraft's term: the Smith-Schlick `G` with
      `k = (roughness + 1)² / 8`, which Karis gives for analytic lights in the 2013 notes above.
    - `SmithSeparable`: the `V` of section B.3.2, from the separable form of the Smith
      masking-shadowing function.
    - `SmithHeightCorrelated`: the height-correlated Smith form for GGX of Heitz, *Understanding
      the Masking-Shadowing Function in Microfacet-Based BRDFs* (JCGT 2014,
      <https://jcgt.org/published/0003/02/03/>), which section B.3.2 cites for its formulas.
      TextureCraft's mesh shader uses this form.

    The Smith-Schlick term equals the separable form at roughness 1; on smoother surfaces it is
    darker at grazing angles (below half of the separable form with roughness 0.25 and the view
    and light both about 84 degrees from the normal). The height-correlated form is at least as
    bright as the separable one. The visibility affects `try_direct` only.
  - Roughness is shaded as at least `MIN_DIRECT_ROUGHNESS` (0.032, `α` = 0.001024).
  - `n·v` is taken as at least `1e-4`, so a view at or below the surface sees it as at a
    grazing angle. `D` is exact where `n·h` is close to 1, so the highlight peaks at its head-on
    value. `D` is 0 where `n·h` is 0 or less, which happens when the view is at least as far
    below the surface as the light is above it.
  - With `SmithSchlick`, the specular denominator `4 (n·v) (n·l)` is taken as at least `1e-4`,
    as in EffectCraft. Where the product is smaller, at grazing angles, the specular lobe is below
    `F D G / (4 (n·v) (n·l))`: about 4% of it with `n·v` and `n·l` both 0.001. The other two
    forms divide by no such product and have no floor.
  - A light at or below the surface, or a view straight opposite the light, gives
    0. Directions are normalized; one that is not finite or has zero length gives
    `PbrError::BadDirection`.
  - A white metal reflects at most the light it receives, with each geometry term. As section
    B.3.6.3 notes, the Fresnel coupling of the two lobes does not conserve energy: with
    `SmithSchlick`, a smooth white dielectric seen about 80 degrees from the normal reflects about 9.8% more light than it
    receives.
- **Default environment.** `default_environment()` is an 8 × 4 gray studio gradient for an
  environment light that has no image: radiance proportional to `0.15 + 1.2 max(0, d·key)` with
  the key `(0.66, 0.75, 0)`, up and toward +X, scaled to a mean of 0.3 over the pixels weighted
  by `sin θ` at their centers.
- **Diffuse.** `try_sh9` projects every pixel of the source onto the nine real SH basis functions
  of `sh_basis`, each integrated over the area of each pixel. The coefficients are convolved with
  the clamped cosine lobe (band factors `π`, `2π/3`, `π/4`, Ramamoorthi and Hanrahan 2001,
  <https://graphics.stanford.edu/papers/envmap/envmap.pdf>) and divided by `π`.
  `irradiance` evaluates them: for a white surface the result is the diffuse radiance it
  reflects.
- **Procedural environments.** `Environment` lists Studio Soft, Studio Contrast, Overcast, Sunset,
  Warehouse and Night (`ENVIRONMENTS` holds the names). `try_render(width)` renders one at
  `width × width / 2`.
- **Limits.** `Limits` caps the side (2048), the pixels of one image (2^22), the bytes one call
  holds at once (256 MiB, counting the source copies, the pyramid, every level and the sample
  table of `try_prefilter`, and the column table of `try_sh9`, 48 bytes per column; a single
  level is only a copy of the source), the levels (8), the samples per pixel (1024) and the sample
  evaluations of one `try_prefilter` call (2^28). Every `try_` function checks its source against
  them and returns a `PbrError` before allocating over them, and a buffer is never larger than
  `isize::MAX` bytes even with raised limits. Pixel buffers are reserved fallibly
  (`PbrError::OutOfMemory`). Options out of range give
  `PbrError::BadOption`, and directions that are not finite or have zero length give
  `PbrError::BadDirection`.
- **Features.** `parallel` computes rows on several threads with rayon. The default build is
  serial and builds for `wasm32-unknown-unknown`. Results are the same either way.

On a 256 × 128 source, the default `try_prefilter` evaluates 1,396,736 samples (10,912 output
pixels in levels 1 to 5, 128 samples each).

## WGSL

`SHADE_WGSL` is the WGSL text of the shading functions (`src/shade.wgsl`). It declares no
bindings and no entry points, and every name in it starts with `pbr_`, `PBR_` or `Pbr`. An app
joins its own declarations, the text and its entry points into one module:

```rust
let source = [APP_BINDINGS, craft_pbr::SHADE_WGSL, APP_ENTRY_POINTS].join("\n");
```

and passes `source` to its shader compiler (with wgpu, `wgpu::ShaderSource::Wgsl(source.into())`).
The crate does not depend on wgpu or naga in its build.

- **Values.** `Material::gpu()` returns a `GpuMaterial`, laid out as the WGSL struct
  `PbrMaterial` (32 bytes: `base_color` at 0, `metallic` at 12, `roughness` at 16, `specular` at
  20, `diffuse` at 24). `GpuSh9::try_new` takes the coefficients of `try_sh9` and lays them out as
  `PbrSh9` (144 bytes, one `vec4<f32>` per coefficient). `to_bytes` on either returns the
  little-endian bytes for a uniform or storage buffer. `GpuSh9::try_new` returns
  `PbrError::BadOption` for a coefficient that is not finite or whose magnitude is over
  `GpuSh9::MAX_COEFFICIENT` (1e30), which keeps the `f32` sum of `pbr_irradiance` finite. Every
  result of `try_sh9` is accepted.
- **Bounds.** No function loops or recurses. The GPU cannot return an error, so where the Rust
  function returns one, the WGSL function returns a defined value: `pbr_direct` and
  `pbr_environment_light` clamp material values to the ranges `Material::try_new` and
  `try_with_weights` accept, a zero-length direction gives 0, and `pbr_environment_brdf` clamps
  roughness to 0 to 1. WGSL implementations may assume that values are finite, so an app uploads
  only checked values: a `GpuMaterial` comes from a checked `Material`.
- **Output.** Colors are linear. The text has no display encoding.

| Rust | WGSL | Notes |
|------|------|-------|
| `Material::try_new` | `pbr_material_clamped` | Clamps to the ranges the Rust constructors check, including `try_with_weights`. |
| `Material::f0` | `pbr_f0` | Same formula. |
| `Material::try_direct` | `pbr_direct` | Same floors: roughness at least 0.032, `n·v` at least `1e-4`, `n·h` at most 1 and `4 (n·v) (n·l)` at least `1e-4`. A light at or below the surface or a view straight opposite the light gives 0 in both. A zero-length direction gives 0 instead of an error, material values are clamped instead of checked, and `D` is evaluated in `f32` by `pbr_ggx_distribution`. |
| `fresnel_schlick` | `pbr_fresnel_schlick` | Same formula. |
| `smith_schlick_ggx` | `pbr_smith_schlick_ggx` | Same formula. |
| `ggx_distribution` | `pbr_ggx_distribution` | Same `α` floor, in `f32`; `(n·h)² (α² − 1) + 1` is computed as `(1 − n·h)(1 + n·h) + (n·h)² α²`. |
| `try_environment_brdf` | `pbr_environment_brdf` | Same fit and clamps; roughness is clamped instead of checked. |
| `dir_of` | `pbr_dir_of` | Same frame. |
| `uv_of` | `pbr_uv_of` | Same frame; a direction straight up or down has `u = 0`, as `atan2(0, 0)` gives in Rust. |
| `sh_basis` | `pbr_sh_basis` | Same basis; a zero-length direction gives nine zeros. |
| `irradiance` | `pbr_irradiance` | Sums in `f32` instead of `f64`. |
| `unit` | `pbr_unit` | Scales by the largest component first, as the Rust helper does; a zero-length direction gives the zero vector. |

Two WGSL functions have no Rust counterpart in this crate. `pbr_specular_levels(roughness,
levels)` returns the two levels of a `try_prefilter` chain to blend and the weight of the second.
`pbr_environment_light` combines irradiance, prefiltered radiance and `pbr_environment_brdf` by
the split sum, as EffectCraft's `shade.rs` does on the CPU.

The functions come from EffectCraft's `crates/gpu/src/shaders/advanced3d.wgsl` (at EffectCraft
`f4d45dfe`), which ports `shade.rs`. Changes from that shader:

- The direct roughness floor is 0.032 instead of 0.03, a view straight opposite the light gives 0
  instead of a fallback half vector, `n·h` is taken as at most 1 as well as at least 0, and the
  environment BRDF keeps `scale` and `bias` at least 0 with a sum of at most 1, as in the Rust
  functions (see Origin).
- A zero-length direction gives 0. EffectCraft's `norm3` replaced it with `(0, 0, −1)`.
- Material values are clamped to the Rust ranges. EffectCraft clamped roughness to 0.03 to 1 and
  read base color, metallic, specular and diffuse unchecked.
- `D` computes its inner term as `(1 − n·h)(1 + n·h) + (n·h)² α²` with an `α` floor of 0.001 and
  a denominator of at least `f32::MIN_POSITIVE`. EffectCraft computed `(n·h)² (α² − 1) + 1`,
  which loses precision in `f32` near `n·h = 1` for small `α`, with a denominator of at least
  `1e-12` and no `α` floor.
- Fresnel raises `1 − v·h` to the fifth power by multiplication instead of `pow`.
- The equirect mapping is in this crate's frame. For EffectCraft's frame, a direction
  `(x, y, z)` there is `(−z, −y, −x)` here, which gives the same `v` and the same `u` before
  EffectCraft adds its rotation.
- EffectCraft's `srgb_to_linear` is not carried: the shader does not call it.

Texture sampling, shadow maps, light attenuation and the unpacking of EffectCraft's buffers stay
in EffectCraft: they read its own bindings and packed layouts. The tests parse and validate the
text with naga, alone and with app bindings and `vs` and `fs` entry points, and check the
`PbrMaterial` and `PbrSh9` layouts against `to_bytes`. They do not run the WGSL on a GPU.

## Origin

The algorithms come from the `crates/render/src/ibl.rs` files of TextureCraft and MaterialCraft,
which held the same filtering code. With a firefly cap of 64, which both apps applied, the
prefiltered levels match theirs for sources with even sides that are at least twice as wide as
they are tall. Changes from those files:

- Halving an odd side keeps its last column or row instead of dropping it.
- The firefly cap is an option, off by default, and the source level bias is an option.
- Sizes, buffer lengths, radiance values, options and directions are checked, and work and memory
  are bounded, before anything is allocated.
- Level sizes never grow past the level before, so a source smaller than 4 × 2 is accepted.
- The source pyramid halves a source while it is wider than 8 or taller than 4, so a source 8 or
  fewer pixels wide and more than 4 tall also gets coarser levels. A source at least twice as wide
  as it is tall gets the same pyramid as in those files.
- The SH projection reads every pixel of the source instead of a source halved to 128 wide by
  plain averaging, which gave rows near the poles too much weight (a lit top row of a 256 × 128
  source counted twice). It integrates each basis function over the area of each pixel instead of
  taking its value and solid angle at the pixel center, so a uniform source of any size, down to
  1 × 1, gives uniform irradiance. It sums in `f64`. On the procedural environments the
  irradiance changes by less than 0.04% at 128 rows and about `1.3 / rows²` of its value on fewer
  rows (3.2e-4 on 64).
- Rows run on several threads only with the `parallel` feature.

The BRDF, the environment BRDF and the default environment come from EffectCraft's
`crates/render/src/three_d/adv/shade.rs` and `adv/scene.rs`. Changes from those files:

- Material values, weights and directions are checked; NaN, infinity, values out of range and
  zero-length directions give an error instead of a NaN or negative color. Directions are
  normalized.
- The distribution is `ggx_distribution_between`, evaluated in `f64` from the normal and the half
  vector. EffectCraft evaluated it in `f32` from `n·h`, which loses the narrow peak of smooth
  surfaces. From roughness 0.25 up, with the view and the light above the surface, the direct
  light matches within `1e-4` of `max(1, value)`.
- Roughness is shaded as at least 0.032 instead of 0.03, so `α` stays above the 0.001 floor of
  `ggx_distribution`.
- A view straight opposite the light gives 0 instead of using a fallback half vector.
- `D` is 0 where `n·h` is 0 or less, a view at least as far below the surface as the light is
  above it, so such a view sees no specular lobe. EffectCraft took `n·h` as at least 0 there and
  gave `D` its value at `n·h` = 0, `α² / π`. EffectCraft also did not keep `n·h` at most 1, so
  rounding after normalization could put `D` above its peak.
- The environment BRDF keeps `scale` and `bias` at least 0 with a sum of at most 1. The fit gives
  a negative scale for smooth surfaces at grazing angles (-0.044 with a bias of 1.044 at
  `n·v` = 0 and roughness 0), which made `f0 × scale + bias` greater than 1 and the diffuse
  weight `1 − fs` negative, and a negative bias of -0.0024 for rough surfaces at grazing angles.
- The default environment is in this crate's frame. An EffectCraft direction `(x, y, z)`, with +Y
  down and the image center at +Z, is `(−z, −y, −x)` here, so the key `(0, −0.75, −0.66)`
  becomes `(0.66, 0.75, 0)` and the pixels are the same within `1e-6`.

## Migration

- **TextureCraft.** Replace `texturecraft_render::ibl` with `craft_pbr`. Pass
  `PrefilterOptions` with `firefly_cap: Some(64.0)` to keep today's output. `EnvImage::from_rgba`
  becomes `EnvImage::try_from_rgba` followed by the app's import sizing: `try_half` while a side is
  over 2048, then `try_resampled` up to at least 128 × 64. The default `Limits` refuse a source
  with a side over 2048 (a 4096 × 2048 source gives `PbrError::BadSize`), so the import passes
  `Limits` with `max_side`, `max_pixels` and `max_bytes` raised to the largest source the app
  accepts, and passes the same limits to each `try_half` and to `try_resampled`. The
  `texturecraft_import` helper in `tests/it/moved.rs` uses `max_side` 2^16, `max_pixels` 2^26 and
  `max_bytes` 1 GiB.
- **MaterialCraft.** Replace `materialcraft_render::ibl` with `craft_pbr`, with the same
  `firefly_cap`. `by_id` stays in the app and maps view ids to `Environment::from_name`.
  `hammersley` returns `None` when `i` is not below `n`.
- **EffectCraft.** Replace `shade::brdf` with `Material::try_new(...)?.try_with_weights(specular_k,
  diffuse_k)?.try_direct(n, v, l)`, and `shade::env_brdf` with `try_environment_brdf`. The
  adapter keeps layer policy, texture sampling, lights, shadows and its box-filtered environment
  chain, and maps a material error to a render failure. `default_environment` replaces
  `scene::default_environment` once the adapter converts directions to this crate's frame.
