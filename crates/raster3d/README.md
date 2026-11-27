# craft-raster3d

A CPU renderer for a prepared 3D scene. A `Scene` holds world-space triangles, their materials and
textures, lights, an optional environment light and shadow maps, as plain arrays. The crate clips,
rasterizes and shades the scene into premultiplied linear RGBA and camera depth, renders the shadow
maps of its lights, box-filters a supersampled result to output size and converts linear pixels to
the sRGB transfer curve. It has no UI, does no file or network I/O and has no `unsafe` code.
Building a scene from an app's document (layers, cameras, models, text) stays in the app.

- **Rendering.** `try_render(&scene, &limits)` returns a `Target` at the scene's raster size:
  premultiplied linear RGBA and the camera-space depth of the nearest opaque surface, infinity where
  nothing is drawn. `Scene::clip` maps world to clip space with reversed depth (a larger
  normalized device z is nearer); `Scene::view` maps world to camera space, and its third row gives
  the depth. Rows are rasterized one band of 16 rows at a time, or on several threads with the
  `parallel` feature.
- **Clipping.** Triangles are clipped against the depth range `0 ≤ z ≤ w` before the perspective
  divide, the clip volume WebGPU uses
  (<https://www.w3.org/TR/webgpu/#coordinate-systems>), and against a guard band of
  `GUARD_BAND × w` (1024) in x and y. A triangle inside the band is not clipped in x or y. When the
  depth clip in `f32` overflows between far-apart corners, the triangle is clipped against every
  plane in `f64` instead.
- **Coverage.** One sample at each pixel center. A pixel on an edge shared by two triangles belongs
  to the triangle for which the edge is a top or left edge, the rule Direct3D documents
  (<https://learn.microsoft.com/en-us/windows/win32/direct3d11/d3d10-graphics-programming-guide-rasterizer-stage-rules>).
  Positions, normals, texture coordinates and tangents are interpolated with perspective-correct
  weights (<https://www.w3.org/TR/webgpu/#coordinate-systems>).
- **Passes.** The first `Scene::opaque_count` indices are opaque triangles. Visibility is resolved
  first and each pixel is shaded once; where the visible fragment is discarded (an alpha test or the
  back face of a single-sided material), the nearest surviving fragment under the pixel is shaded
  instead. The remaining indices are transparent triangles, which the caller sorts back to front.
  Each is shaded per fragment, depth-tested against the opaque surfaces and blended over them. A
  triangle whose material is shadows-only belongs in the transparent range; `try_render` returns an
  error for one in the opaque range unless its material's `visible` is false.
  Valid surfaces with opacity below 1 use the transparent range, including opaque and mask alpha modes.
- **Facing.** A triangle whose screen-space winding is negative with y down
  (counterclockwise on the screen) faces the camera. A material with `double_sided` false does not
  draw back faces; a double-sided material shades a back face with its normal reversed.
- **Shading.** Direct light uses the Cook-Torrance BRDF with the GGX distribution, the
  Smith-Schlick geometry term with `k = (roughness + 1)² / 8` and Schlick's Fresnel, with a Lambert
  diffuse lobe weighted by `(1 − F)(1 − metallic)`, as in Karis, *Real Shading in Unreal Engine 4*
  (SIGGRAPH 2013,
  <https://de45xmedrsdbp.cloudfront.net/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf>).
  Roughness is shaded as at least 0.03. Light colors are the irradiance on a surface facing the
  light, so π is folded into the light intensity. Parallel, spot and point lights have no falloff,
  a smooth falloff or a clamped inverse-square falloff. The environment light reads an irradiance
  map for diffuse light and a chain of radiance levels by roughness for specular light, with
  Karis's analytic environment BRDF for mobile
  (<https://www.unrealengine.com/en-US/blog/physically-based-shading-on-mobile>). Materials take a
  base color, metallic-roughness, normal, occlusion and emissive texture, an alpha mode (opaque,
  mask or blend), and weights for the diffuse lobe, the dielectric reflectance and the ambient light.
  In mask mode a fragment whose material alpha (the base alpha times the base texture's) is below
  the cutoff is discarded, and a fragment that passes is drawn with the layer's opacity alone, so
  at an opacity of 1 it is fully opaque
  (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#alpha-coverage>). In blend mode the
  alpha is the material alpha times the opacity.
- **Textures.** `Scene::texels` holds linear straight RGBA, row-major from the top row, and each
  `TexInfo` gives a texture's offset, size and wrap modes (repeat, clamp or mirror). `sample` is a
  bilinear lookup with texel centers at +0.5.
- **Shadow maps.** `try_shadow_maps(&mut scene, draft, &limits)` renders the light-space depth of
  every triangle whose material casts shadows into one map per parallel or spot light, six per
  point light and one orthographic map for the environment light's dominant direction, for every
  light whose `shadow` is 0 or more. The shader compares each receiver with the nearest depth in the map
  (<https://developer.nvidia.com/gpugems/gpugems/part-ii-lighting-and-shadows/chapter-11-shadow-map-antialiasing>)
  over a jittered grid of 5 × 5 to 10 × 10 taps, each a bilinear comparison against the receiver's
  plane. A spot or point light's maps hold casters at a light-space depth of 0.001 or more, and a
  receiver at that depth or nearer is not shadowed. A spot light's map covers its outer cone's half
  angle plus 0.05 radians. A spot light whose half angle with that margin is over 1.45 radians
  (about 83°) gets six maps, as a point light does. A receiver of six maps reads the face of its
  direction's major axis, and a tap past that face's edge reads the adjacent face's texel along the
  tap's ray, with its depth converted to the receiver's face. A tap past the edge of a single map is unoccluded. A masked caster drops texels whose material alpha is below the
  material's cutoff, as shading does. A blended caster drops texels whose alpha times the opacity
  is below 0.5. A masked or blended caster without a base texture is tested the same way with its
  material's alpha. An opaque-mode or masked caster with an opacity below 0.5 casts no shadow.
  `shadow_map_size` gives one map texel per
  raster pixel (supersampled pixels when `ssaa` is above 1, as EffectCraft sizes them) to a power
  of two from 512 to 4096 (512 for a draft render), and
  `shadow_point_face_size` caps a cube face at 2048. `shadow_sizes` keeps a scene within
  `MAX_SHADOW_TEXELS` (26,000,000) by halving the largest map above its floor (512, or 128 for a
  cube face), the earliest light first on ties, and then dropping the last lights' maps.
  `try_env_shadow_map` builds the environment light's map. A second call of `try_shadow_maps`
  replaces the maps of the first, and gives the same maps and marks. Its depth tests (one per
  caster and map, plus each caster's bounding box in each map) are limited by `max_work`, and on
  an error it leaves the scene as it was. The bounding boxes of casters that are skipped for their
  material's alpha or opacity are not counted: a masked or blended caster without a base texture
  whose alpha fails its test, a masked or blended caster with a base texture whose base alpha
  times the opacity (the base alpha alone when masked) is 0 under a cutoff above 0, and an
  opaque-mode or masked caster with an opacity below 0.5.
- **Resolve and encoding.** `try_resolve(&target, k)` averages each `k × k` block of premultiplied
  pixels with equal weights, and takes the mean depth of the block's covered pixels, infinity where
  none is covered. Factors 1 and 2 use `f32` sums unless those sums overflow; higher factors use
  `f64` sums. `try_encode(&mut image)` divides each color by alpha, applies the sRGB transfer
  function of CSS Color 4 (<https://www.w3.org/TR/css-color-4/#color-conversion-code>), a linear
  segment up to a magnitude of 0.0031308 and a 1/2.4 power above it, with a negative color keeping
  its sign, multiplies by alpha again and leaves alpha linear. `srgb_to_linear` decodes a negative
  value the same way. Pixels with alpha of 1e-6 or less are left as they are. `try_render_cpu` renders and
  resolves by `Scene::ssaa`, and rejects a factor that does not divide the raster size before it
  renders.
- **Limits.** `Limits` caps the raster pixels (2^24), the triangles of a scene (2^22), its vertices, used or not (3 × 2^22), the lights
  (1024) and the coverage tests of the visibility, transparent and fallback passes (2^31). Preparation
  counts every clipped triangle's bounding-box pixels plus one per triangle; fallback tests count
  during rendering. `try_render`, `try_render_cpu` and
  `try_shadow_maps` check the scene first and return a `RenderError` before they allocate a target
  or a map: a zero side, a size over the limits, a side over 2^30, a buffer over `isize::MAX` bytes, a light whose shadow map does not
  exist (`try_render`; a mark of −1 means no shadow), an index list that is not whole triangles, an index, material
  or texture that does not exist, an environment light with no radiance level or with a level or
  irradiance map that does not exist, a texture or shadow-map span past its array, a value that is
  not finite, a material or texel alpha, a material opacity, metallic, roughness or occlusion
  strength, a shadow darkness or an environment
  light's key outside 0 to 1, a negative shadow diffusion or shadow-map radius, a parallel or spot light direction or an environment light direction
  (other than zero) whose length differs from 1 by more than 0.001, a spot light whose inner cone
  cosine is below its outer one, a light kind, light falloff
  or alpha mode other than 0 to 2. `try_render` and `try_render_cpu` also return `BadClipPosition`
  for a vertex whose clip-space position is past `f32`. `try_render` also returns
  `ShadowsOnlyOpaque` for a triangle in the opaque range whose material is shadows-only and
  visible, and
  `BadPixel` when a rendered color or depth overflows `f32` (finite inputs near the top of its
  range). `try_shadow_maps` also returns an error when a map's view or a caster's
  light-space corners would not fit `f32` (vertices far from the origin).
  `try_env_shadow_map` returns an error for inputs out of range and for a map that would hold a
  value that is not finite. `try_resolve` and `try_encode` take no `Scene` and check only their
  own inputs. `try_resolve` returns an error for a factor that is zero or does not divide both
  sides and for buffers of the wrong length; a target with a zero side resolves to an empty image.
  `try_encode` returns an error for an image whose buffer does not hold width × height pixels, a
  pixel that is not finite, an alpha below 0 or above 1 (beyond 1e-4 of rounding), or a color whose
  division by alpha overflows, and changes no pixel then; the error names the first such pixel. The renderer holds 40 bytes per raster
  pixel (color, depth and visibility) and up to seven clipped triangles of about 140 bytes per
  scene triangle. A band of 16 rows that has a discarded opaque pixel also holds 29 bytes per pixel
  (a discard flag and a candidate fragment) while its opaque pixels are shaded. Targets, images,
  shadow maps and the working buffers that grow with the raster or the vertex or triangle count
  are reserved fallibly: when a reservation fails, the `try_` function returns `OutOfMemory`. A
  triangle's clipped polygon is held inline, so clipping allocates nothing. The shadow pass's
  lists of lights and maps, a few entries per light, are allocated as usual. The search for a
  surviving fragment under discarded opaque pixels visits each opaque triangle's bounding box
  once more, so it runs no more coverage tests than the visibility pass. Its tests count toward
  `max_work`, and it can return `TooMuchWork` after the target is allocated.
- **Features.** `parallel` rasterizes bands of rows, resolves and encodes rows and pixels, and
  renders shadow maps on several threads with rayon. The default build is serial and builds for
  `wasm32-unknown-unknown`. Every output and error is the same either way, except the `needed`
  count of `TooMuchWork` when fallback or shadow maps pass the work limit, since each thread stops counting
  at its own point.
- **wasm32.** `tests/it/headless.rs` renders a scene lit by an environment from `craft-pbr`, with
  shadow maps, an alpha-masked card and a blended card, natively and as a wasm32 module under
  Node.js. The shadow texels, the depth and the encoded image at 8 bits per channel hash to the
  same constants on x86_64 Linux and in the module; other targets run only its coverage and
  behavior checks. Natively, Rust's `acos`, `atan2`, `exp2`, `log2`, `powf`, `sin` and `cos` call
  the platform's math library, and on wasm32 they call Rust's own, so the encoded `f32` colors
  can differ by a few units in the last place. With
  `wasm-bindgen-test-runner` of the `wasm-bindgen` version in `Cargo.lock` on the path:

  ```sh
  CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
    cargo test -p craft-raster3d --no-default-features --target wasm32-unknown-unknown --test it headless::
  ```

  The `headless_scene` example renders the same scene as a program. Built for wasm32 with the
  serial build, it is a module with no imports that exports `main` and its memory. An ignored
  test in `tests/it/headless.rs` checks its import list with `craft-wasm`, runs it in `wasmi`, and
  requires both hashes to match and the memory to stay at or under 128 pages (8 MiB; the release
  build grows to 118):

  ```sh
  cargo build -p craft-raster3d --no-default-features --release --target wasm32-unknown-unknown --example headless_scene
  CRAFT_RASTER3D_HEADLESS_WASM=target/wasm32-unknown-unknown/release/examples/headless_scene.wasm \
    cargo test -p craft-raster3d --test it headless:: -- --ignored
  ```

## Origin

The renderer comes from EffectCraft's `crates/render/src/three_d/adv.rs` (`resolve`, `encode`,
`render_cpu`), `adv/raster.rs`, `adv/shade.rs` and the scene types of `adv/scene.rs`, at
`2f16feb12f`. `tests/it/differential.rs` runs a copy of EffectCraft's code with the alpha-mask
change of storytold/effectcraft#4075 applied on the scenes in that module, and for those scenes
every shadow texel, raster pixel, depth, resolved pixel and encoded pixel matches EffectCraft's bit
for bit, with the rasters drawn without point lights' shadows. With them, every depth matches and
every pixel matches or is darker here. Each test prints the FNV-1a hashes of both sides' outputs. Other valid scenes can differ
through the changes listed below, for example where triangles leave the guard band or reach the
far plane, where depth-range crossings lie within 1/4096 of an edge from its inside corner, where
texture coordinates times the texture's width or height reach 2^23 in magnitude, where normals'
squared lengths, squared distances from fragments to point and spot lights, or casters'
light-space coordinates times a map's focal length overflow `f32`, where shadow-casting spot
lights' outer cones are over 1.40 radians wide, where a point light's shadow taps reach past a cube
face's edge, where resolve factors are greater than 2,
where shadows-only materials do not receive shadows, where fragments under discarded
opaque pixels leave the depth range, where blended casters have no base texture, where
opaque-mode and masked casters have an opacity below 0.5, where casters reach within 1 unit of
light-space depth of a spot or point light, where an encoded color is negative, where an
inverse-square light's radius is below 1, where a light lies straight opposite the view, where a
clip w makes 1/w subnormal, and where a
texture is sampled between large finite texels of opposite signs. Halfway between red values of
`3e38` and `−3e38`, for example, the sampler here returns red 0 and EffectCraft's interpolation
overflows `f32`.
Changes from those files:

- Scenes, resolve factors and pixels are checked. `Limits` bounds sizes, triangles, lights and
  coverage work. Scene and size checks precede target allocation. Preparation can allocate vertex
  and triangle buffers before a work-limit error. EffectCraft panicked on an index or a material
  reference that did not exist, an alpha-masked caster whose texture did not exist, an environment
  level range that overflowed, a zero raster width and a resolve factor of 0. It returned a cropped
  image for a factor that did not divide the size, NaN or infinite pixels for values that were not
  finite, and black environment light for levels that did not exist.
- Targets, images, shadow maps and the working buffers that grow with the raster or the vertex
  or triangle count are reserved fallibly, and a failed reservation gives `OutOfMemory`. EffectCraft's allocations
  stopped the process when memory ran out.
- Triangles are clipped against the guard band, and clipped in `f64` when the depth clip in `f32`
  overflows. EffectCraft clipped only the depth range, so a triangle with corners far past the
  viewport that crossed the depth range could be dropped. Triangles leaving the band can shade
  differently at their edges. A guard-band crossing is interpolated from the corner nearer the
  plane, and a depth-range or spot-map near-plane crossing within 1/4096 of an edge from its
  inside corner is interpolated from that corner. EffectCraft interpolated from the edge's first
  corner, which put the crossing on the inside corner when the other corner was far away, and
  dropped part of the triangle or the shadow caster.
- `try_shadow_maps` replaces the scene's maps and shadow texels, and reads any `shadow` of 0 or
  more as a request, so the indices one call writes request the same maps from the next.
  EffectCraft appended texels to those of an earlier call, which shifted every map's offset, and
  read only 0 as a request, so a second call kept maps only for the light whose first map was
  map 0.
- The search for a surviving fragment under a discarded opaque pixel visits the triangles in the
  same order over their bounding boxes. EffectCraft searched every triangle of the pixel's band of
  16 rows for each such pixel, which the work limit could not bound. The search takes only
  fragments inside the depth range, as the visibility pass does; EffectCraft also took a fragment
  whose interpolated depth rounded past 1.
- A shadows-only material that does not receive shadows draws nothing. EffectCraft drew the
  shadows on it.
- A triangle in the opaque range whose material is shadows-only and visible is an error.
  EffectCraft shaded it in the opaque pass, which wrote its shadow color and its depth and hid the
  surfaces behind it. Its scene builder puts shadows-only triangles in the transparent range, so its scenes pass.
- A texture coordinate whose value times the texture's width or height reaches 2^23 is scaled to
  texels in `f64`, after a repeated or mirrored axis reduces it by its period. EffectCraft scaled
  it in `f32`, which dropped the half-texel shift there, and from 2^24 saturated the texel index,
  which picked a texel off the wrap period. A shadow caster's texture coordinate whose
  perspective interpolation overflows `f32` (a large coordinate times a corner's 1/depth) is
  interpolated in `f64`; EffectCraft's was infinite, and its alpha test read a texel off the wrap
  period.
- A negative color is encoded on the sRGB curve with its magnitude choosing the segment and its
  sign kept, as CSS Color 4's conversion code does. EffectCraft put every negative value on the
  linear segment, so red −0.25 encoded to −3.23 instead of about −0.537.
- A normal whose squared length overflows `f32`, and the direction to a point or spot light or to
  a perspective eye whose difference from the fragment or its squared length overflows `f32`, are
  normalized in `f64`. EffectCraft divided them by an infinite length, so the surface took no
  direct light or shaded with a view direction that was not a number. A distance to such a light past `f32`'s
  range is kept in `f64` through the falloff, so a light 4.2e38 away with a radius of 3e38 and an
  inverse-square falloff passes half its light instead of none.
- A shadow tap past the edge of one of a point light's or wide spot light's six maps reads the
  adjacent face's texel along the tap's ray, with its depth converted to the receiver's face.
  EffectCraft read such a tap as unoccluded, so a receiver on a seam between two faces inside a
  shadow was about half lit.
- A caster corner or a receiver whose light-space coordinate times the map's focal length overflows
  `f32` is projected in `f64`. EffectCraft's map coordinates for it were infinite.
- A spot light whose outer cone's half angle plus 0.05 radians is over 1.45 radians gets six maps,
  as a point light does. EffectCraft gave it one map 1.45 radians wide, so receivers in the rest
  of the cone were not shadowed.
- Resolve blocks with factors above 2 are averaged in `f64`. EffectCraft's `f32` count of
  covered subsamples stopped at 2^24, which raised the mean depth.
- A clipped corner whose `1/w` overflows `f32` (a clip transform scaled near `f32`'s smallest
  values) is projected in `f64` instead of dropping its triangle, and a drawn pixel whose camera
  depth overflows is an error.
- An inverse-square light's attenuation is capped at 1. EffectCraft divided by the larger of the
  radius and 1, so a light with a radius below 1 grew brighter between its radius and a distance
  of 1 (about 1.78 at 0.75 for a radius of 0.5).
- A light straight opposite the view direction, which a normal-mapped fragment can face, gives no
  direct light, as `craft-pbr`'s `Material::try_direct` does. EffectCraft took a half vector of
  (0, 0, −1) there and could draw a sharp highlight.
- A clipped corner whose ndc depth `z × 1/w` rounds outside 0 to 1 (a subnormal `1/w`) takes
  `z / w` instead, so a triangle on the near plane with a huge w is drawn. The radiance level of
  the environment light is clamped to the last level of its chain, which `f32` rounding can pass
  above 2^24 levels.
- A texture lookup computes its texel offset in `i128`, so `sample` on a texture whose sides
  are near `u32::MAX` reads a missing texel as transparent black.
- A texture or shadow-map lookup at a coordinate past `i64` saturates instead of overflowing, and
  a texture coordinate that is not finite is taken as the largest finite one, so the sample stays
  finite. A channel whose bilinear interpolation between large finite
  texels overflows `f32` is interpolated in `f64`.
- A receiver more than its filter reach plus one texel outside a shadow map is unshadowed before
  the filter runs. EffectCraft's tap jitter scaled the map coordinate by 8, which overflowed past
  `f32::MAX / 8` and made the pixel NaN.
- A masked fragment that passes its cutoff is drawn with the layer's opacity alone and writes its
  depth, and the cutoff tests the material's alpha without the opacity. EffectCraft at
  `2f16feb12f` kept the fragment's alpha below 1, so the clear color showed through a masked
  surface in the opaque pass; storytold/effectcraft#4075 makes the same change there, and the copy
  in `tests/it/reference/` carries it.
- A masked or blended caster without a base texture tests its material's alpha as a textured
  caster tests a texel's, and an opaque-mode or masked caster with an opacity below 0.5 casts no
  shadow. EffectCraft at `2f16feb12f` tested every textured caster against 0.5 and drew
  untextured and opaque-mode casters whole, so a shadow could differ from the visible surface.
  With storytold/effectcraft#4075, EffectCraft's masked casters use the material's cutoff, and
  textured casters and untextured masked casters cast the same shadows as here. Two kinds of
  caster still differ: an opaque-mode or masked caster with an opacity below 0.5, and an
  untextured blended caster whose alpha times the opacity is below 0.5. EffectCraft draws both,
  and here they cast no shadow.
- A spot light whose inner cone cosine is below its outer one is `BadLight`. EffectCraft's lights
  always have the inner cosine at or above the outer one.
- A material whose metallic, roughness or occlusion strength is outside 0 to 1 is `BadMaterial`.
  EffectCraft clamps these for its own materials and passes a glTF model's factors as the file
  gives them, and the shader extrapolated past either end.
- A spot or point light's shadow maps clip casters at a light-space depth of 0.001, the depth
  below which the shader does not shadow a receiver. EffectCraft clipped them at 1, so a caster
  within 1 unit of the light cast no shadow and the receivers behind it were lit. A caster's
  crossing with that plane takes the plane's depth exactly, so a caster crossing it with
  far-apart corners stays in front of the light.
- A parallel or spot light's direction, and the environment light's direction when it is not
  zero, must be unit length within 0.001. EffectCraft shaded with the direction as given and
  normalized it for the shadow map, so a direction of another length scaled the light and changed
  a spot light's cone without changing its shadow.
- A triangle on the far end of the depth range (normalized device z of 0) is drawn. EffectCraft
  read a depth of 0 as an empty pixel and dropped it.
- `try_render` returns an error for a shadow mark that names a map that does not exist.
  EffectCraft drew that light unshadowed. Its scene builder runs the shadow maps or sets every
  mark to −1, so its scenes pass.
- A raster side over 2^30 is an error whatever the limits.
- A vertex that names a material that does not exist is an error. EffectCraft skipped its
  triangle in the raster and panicked on it in the shadow pass.
- The environment light's map is built by `try_env_shadow_map`, which returns an error for inputs
  that are out of range.
- A negative shadow diffusion on a light or the environment light is an error. EffectCraft
  subtracted it from the penumbra, so a shadow edge could be narrower than its minimum width or
  fall back to a reach of 2 texels.
- `render`, `setup`, `shadow_raster` and `shadow_maps` are split into smaller functions with the
  same order of operations.
- The camera basis of the shadow maps is computed in this crate from the same operations as
  EffectCraft's `camera::basis_view`.
- Bands, rows, pixels and shadow maps run on several threads only with the `parallel` feature.

## Migration

EffectCraft replaces `adv::raster::render` with `try_render`, `adv::resolve` with `try_resolve`,
`adv::encode` with `try_encode`, `adv::render_cpu` with `try_render_cpu`, and
`raster::shadow_maps` with `try_shadow_maps`, and maps a `RenderError` to a render failure. Its
scene builder fills `craft_raster3d::Scene`, whose fields are those of its `adv::scene::Scene`.
The BRDF and the environment BRDF stay as they are here until a separate change moves them to
`craft-pbr`, whose results differ (a roughness floor of 0.032 and a clamped environment BRDF).
