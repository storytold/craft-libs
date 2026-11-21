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
  the depth. Rows are rasterized on several threads with rayon.
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
  Each is shaded per fragment, depth-tested against the opaque surfaces and blended over them.
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
  angle plus 0.05 radians, up to 1.45 radians (about 83°), as EffectCraft's does; a receiver
  outside the map is not shadowed. A masked caster's alpha test applies
  to the map with the material's cutoff, as in shading. A blended caster drops texels whose alpha is below 0.5. A masked or blended caster without a base
  texture is tested the same way with its material's alpha, and an opaque-mode caster with its
  opacity. `shadow_map_size` gives one map texel per
  raster pixel (supersampled pixels when `ssaa` is above 1, as EffectCraft sizes them) to a power
  of two from 512 to 4096 (512 for a draft render), and
  `shadow_point_face_size` caps a cube face at 2048. `shadow_sizes` keeps a scene within
  `MAX_SHADOW_TEXELS` (26,000,000) by halving the largest map above its floor (512, or 128 for a
  cube face), the earliest light first on ties, and then dropping the last lights' maps.
  `try_env_shadow_map` builds the environment light's map. A second call of `try_shadow_maps`
  replaces the maps of the first, and gives the same maps and marks. Its depth tests (one per
  caster and map, plus each caster's bounding box in each map) are limited by `max_work`, and on
  an error it leaves the scene as it was.
- **Resolve and encoding.** `try_resolve(&target, k)` averages each `k × k` block of premultiplied
  pixels with equal weights, and takes the mean depth of the block's covered pixels, infinity where
  none is covered; a pixel whose `f32` sums overflow is averaged in `f64`. `try_encode(&mut image)` divides each color by alpha, applies the sRGB transfer
  function of CSS Color 4 (<https://www.w3.org/TR/css-color-4/#color-conversion-code>), a linear
  segment up to 0.0031308 and a 1/2.4 power above it, multiplies by alpha again and leaves alpha
  linear. Pixels with alpha of 1e-6 or less are left as they are. `try_render_cpu` renders and
  resolves by `Scene::ssaa`, and rejects a factor that does not divide the raster size before it
  renders.
- **Auxiliary outputs.** `try_render_with_aux`, `try_resolve_aux` and `try_render_cpu_with_aux`
  return the same color as `try_render`, `try_resolve` and `try_render_cpu`, bit for bit, and
  `AuxPlanes`: three planes at the color's size, row-major from the top row, that describe one
  surface per pixel. `depth` is the surface's camera depth (`f32`, the units of `Scene::view`),
  infinity where empty. `normal` is its unit normal in view space, `[0, 0, 0]` where empty: the
  interpolated vertex normal (one of length 1e-20 or less is read as `[0, 0, −1]`), reversed on a
  back face, turned by the inverse transpose of the upper-left 3 × 3 of `Scene::view` and
  normalized, so it stays perpendicular to the surface when the view scales axes unequally or
  shears; normal textures are not applied. `part_id` is the `Scene::part_ids` entry of the
  surface's triangle (`u32`), or `NO_PART` (`u32::MAX`) where empty; a triangle cut by clipping keeps its ID. Only a fragment that
  adds alpha above 0 can be the surface: the nearest opaque surface drawn at the pixel, including
  the one found under a discarded fragment, or a nearer transparent fragment; of several such
  fragments the nearest is kept. `depth` therefore differs from `Target::depth`, which holds opaque surfaces only. The
  resolve copies the depth, normal and part ID of one subsample of each block, the one with the
  smallest depth (the first in row-major order on ties), so it does not average depths across
  surfaces or mix IDs; a pixel holds one ID, not every surface that covers part of it. These
  functions need one part ID per triangle, none of them `NO_PART`, and, for a scene with a
  triangle, a view whose upper-left 3 × 3 is invertible. The other functions do not read
  `Scene::part_ids` and do not allocate the planes.
- **Limits.** `Limits` caps the raster pixels (2^24), the triangles of a scene (2^22), its vertices, used or not (3 × 2^22), the lights
  (1024) and the coverage tests of the visibility and transparent passes (2^31: the pixels of every
  clipped triangle's bounding box, plus one per triangle). Every `try_` function checks the scene
  first and returns a `RenderError` before it allocates the target: a zero side, a size over the
  limits, a side over 2^30, a buffer over `isize::MAX` bytes, a light whose shadow map does not
  exist (`try_render`; a mark of −1 means no shadow), an index list that is not whole triangles, an index, material
  or texture that does not exist, an environment light with no radiance level or with a level or
  irradiance map that does not exist, a texture or shadow-map span past its array, a value that is
  not finite, a material or texel alpha, a material opacity, metallic, roughness or occlusion
  strength, a shadow darkness or an environment
  light's key outside 0 to 1, a parallel or spot light direction or an environment light direction
  (other than zero) whose length differs from 1 by more than 0.001, a spot light whose inner cone
  cosine is below its outer one, a light kind
  or alpha mode other than 0 to 2, or a clip-space position past `f32`. `try_render` returns
  `BadPixel` when a rendered color or depth overflows `f32` (finite inputs near the top of its
  range). `try_shadow_maps` also returns an error when a map's view or a caster's
  corners in a map would not fit `f32` (vertices far from the origin).
  `try_env_shadow_map` returns an error for inputs out of range and for a map that would hold a
  value that is not finite. `try_resolve` returns an error for a factor that is zero or does
  not divide both sides and for buffers of the wrong length. `try_encode` returns an error for a
  pixel that is not finite, an alpha below 0 or above 1 (beyond 1e-4 of rounding), or a color whose
  division by alpha or encoding overflows, and changes no pixel then. The renderer holds 40 bytes per raster
  pixel and up to seven clipped triangles of about 140 bytes per scene triangle. With auxiliary
  outputs it holds 20 bytes more per raster pixel, and `try_render_cpu_with_aux` also 40 bytes per
  output pixel for the resolve; the sum must fit `isize::MAX` bytes, or the call returns
  `TooLarge` before it allocates. `try_render_with_aux` returns `BadPartIdCount`,
  `ReservedPartId` or `BadTransform` for the part IDs and view above, and `BadPixel` when an
  auxiliary depth overflows `f32`; `try_resolve_aux` returns `BadDataLength` for a plane of the
  wrong length. The search for a
  surviving fragment under discarded opaque pixels visits each opaque triangle's bounding box
  once more, so it runs no more coverage tests than the visibility pass.

## Origin

The renderer comes from EffectCraft's `crates/render/src/three_d/adv.rs` (`resolve`, `encode`,
`render_cpu`), `adv/raster.rs`, `adv/shade.rs` and the scene types of `adv/scene.rs`, at
`2f16feb12f`. For valid scenes whose triangles stay inside the guard band and off the far plane,
whose depth-range crossings lie more than 1/4096 of an edge from its inside corner, whose texture
coordinates on repeated and mirrored axes stay below 2^24, whose shadows-only materials receive
shadows, whose fragments under discarded opaque pixels stay inside the depth range, whose masked
and blended casters have a base texture and a cutoff of 0.5, whose opaque-mode casters have an
opacity of 0.5 or more, and whose casters do not reach within
1 unit of light-space depth of a spot or point light, every shadow texel, raster pixel, depth, resolved pixel and encoded pixel matches EffectCraft's bit for bit;
`tests/it/differential.rs` runs a copy of EffectCraft's code on the same scenes. Changes from those
files:

- Scenes, resolve factors and pixels are checked, and sizes, triangles, lights and coverage work
  are bounded, before anything is allocated. EffectCraft panicked on an index or a material
  reference that did not exist, an alpha-masked caster whose texture did not exist, an environment
  level range that overflowed, a zero raster width and a resolve factor of 0. It returned a cropped
  image for a factor that did not divide the size, NaN or infinite pixels for values that were not
  finite, and black environment light for levels that did not exist.
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
- A repeated or mirrored texture axis reduces a coordinate of 2^24 or more by its period before
  scaling it to texels. EffectCraft saturated the texel index, which picked a texel off the wrap
  period.
- A clipped corner whose `1/w` overflows `f32` (a clip transform scaled near `f32`'s smallest
  values) is projected in `f64` instead of dropping its triangle, and a drawn pixel whose camera
  depth overflows is an error.
- A texture lookup computes its texel offset in `i128`, so `sample` on a texture whose sides
  are near `u32::MAX` reads a missing texel as transparent black.
- A texture or shadow-map lookup at a coordinate past `i64` saturates instead of overflowing, and
  a texture coordinate that overflows `f32` when scaled to texels is taken as the largest finite
  one, so the sample stays finite. A channel whose bilinear interpolation between large finite
  texels overflows `f32` is interpolated in `f64`.
- A masked shadow caster uses the material's alpha cutoff, and a masked or blended caster
  without a base texture tests its material's alpha as a textured caster tests a texel's. An
  opaque-mode caster with an opacity below 0.5 casts no shadow. EffectCraft tested every textured
  caster against 0.5 and drew untextured and opaque-mode casters whole, so a shadow could differ
  from the visible surface. With a cutoff of 0.5, textured masked and blended casters cast the
  same shadows as EffectCraft's. Two kinds of caster still differ at that cutoff: an opaque-mode
  caster with an opacity below 0.5, and an untextured masked or blended caster whose alpha is
  below 0.5. EffectCraft drew both whole, and here they cast no shadow.
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
- `render`, `setup`, `shadow_raster` and `shadow_maps` are split into smaller functions with the
  same order of operations.
- The camera basis of the shadow maps is computed in this crate from the same operations as
  EffectCraft's `camera::basis_view`.

## Migration

EffectCraft replaces `adv::raster::render` with `try_render`, `adv::resolve` with `try_resolve`,
`adv::encode` with `try_encode`, `adv::render_cpu` with `try_render_cpu`, and
`raster::shadow_maps` with `try_shadow_maps`, and maps a `RenderError` to a render failure. Its
scene builder fills `craft_raster3d::Scene`, whose fields are those of its `adv::scene::Scene`.
The BRDF and the environment BRDF stay as they are here until a separate change moves them to
`craft-pbr`, whose results differ (a roughness floor of 0.032 and a clamped environment BRDF).
