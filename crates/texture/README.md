# craft-texture

Linear float image planes and texture sampling. An `Image` is a gray (1 channel) or RGBA plane of
`f32`, row-major with the top row first, shared copy-on-write through an `Arc`. Values are stored
as given, so HDR values above 1 and negative values are kept. The crate has no UI, does no file or
network I/O and has no `unsafe` code.

- **Construction.** `try_filled`, `try_generate`, `try_generate_uv` and `from_data` check the size
  against `Limits` (longest side, pixel count, bytes) before allocating and return an
  `ImageError`. `from_data` also checks that the buffer holds width × height × channels floats.
  `filled`, `generate`, `generate_uv` and `gray` clamp each side to `1..=MAX_SIDE` (8192).
  A plane is never larger than `isize::MAX` bytes (2 GiB minus one byte on `wasm32`), even with
  raised limits: the `try_` forms return `ImageError::TooLarge` for a larger one, and `map`,
  `to_color` and `to_kind` clamp each side to `1..=MAX_SIDE` and map the top-left part instead.
  `try_map`, `try_to_kind`, `try_resize` and `try_to_rgba8_display` reserve their pixel buffer
  first and return `ImageError::OutOfMemory` when that reservation fails. They still make small
  allocations that are not fallible: the `Arc` around a new image's buffer, and the boxed
  resampler of `try_resize`. The forms without `try_` allocate like `Vec`. A failed infallible
  allocation ends the process.
- **Sampling.** `sample` (bilinear) and `sample_nearest` take normalized `(u, v)` with `(0, 0)` at
  the upper-left corner and `v` increasing downward, as glTF 2.0 section 3.8.3 defines them
  (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html>). `tile` picks repeat (glTF
  `REPEAT`) or clamp to edge (`CLAMP_TO_EDGE`) for both axes. The texel coordinate, the wrapping
  and the bilinear weights follow the Vulkan specification, sections 20.4.1, 20.6.1 and 20.6.3
  (<https://registry.khronos.org/vulkan/specs/latest/html/vkspec.html#textures-texel-filtering>).
  Repeat keeps its period at any finite coordinate. A NaN or infinite coordinate gives
  `[0, 0, 0, 1]`. Bilinear sums are taken in `f64` and clamped to the `f32` range, so finite
  texels give a finite sample. `sample_axes`, `sample_nearest_axes` and `get_axes` take
  `tiling: [bool; 2]`, the wrap mode of `u` and of `v` on their own, as glTF's `wrapS` and
  `wrapT`; with equal entries they give the same results as the forms with one `tile`.
- **Stored sampling.** `sample_stored` is the texture read of MaterialCraft's function graphs. It
  rounds the bilinear weights to multiples of 1/256 and blends the texels by linear interpolation
  in `f32`. When the image's integer-sampling flag is on (`with_integer_sampling(true)`, read with
  `integer_sampling()`), it then clamps each channel to `0..=1` and rounds it to a multiple of
  1/65535. A channel whose interpolation is not finite is taken from the `f64` weighted sum
  instead, so finite texels give a finite sample. New images start with the flag off, clones keep
  it, and two images are equal only when their flags are equal.
- **Mip chains.** `mip::MipChain::try_build` makes every level below an image down to 1 × 1, each
  side halved and rounded down to at least 1, with `floor(log2(max(w, h))) + 1` levels, as
  section 12.4.4 of the Vulkan specification and `ARB_texture_non_power_of_two`
  (<https://registry.khronos.org/OpenGL/extensions/ARB/ARB_texture_non_power_of_two.txt>) size
  them. A level averages 2 × 2 texels of the one before it in `f64`; on an odd side longer than
  1 the last texel of the level averages the last three, and a side of 1 stays 1. Every channel,
  alpha included, is averaged on its own as linear data, without premultiplication.
  `from_levels` takes levels made elsewhere and checks their sizes. `lod` takes the derivatives
  of `(u, v)` along the two screen axes and returns `log2` of the longer scaled footprint plus
  `lod_bias`, clamped to `min_lod..=max_lod`, as section 20.3.7 defines it. `select` picks one
  level, where a tie takes the lower-numbered level (1.5 selects level 1), or two levels with a
  blend. `sample_grad` and `sample_lod` sample them with the wrap mode of each axis applied in
  each level, nearest or bilinear within a level as section 20.6.3 describes.
  `MipMinFilter::from_gltf` and `TexelFilter::from_gltf` read glTF 2.0 `minFilter` and
  `magFilter` codes. NaN or infinite derivatives, LODs, biases and bounds, and a `min_lod` above
  `max_lod`, return a `MipError`. Each level is checked against `MipLimits::image`, and all
  levels together against `max_chain_bytes`, before anything is allocated; `required_bytes`
  makes the same check for a size alone.
- **Other operations.** `get` reads one texel, `map`, `to_gray`, `to_color` and `to_kind` convert,
  `resize` box-averages whole-factor shrinks and samples bilinearly otherwise, and
  `to_rgba8_display` encodes 8-bit sRGB RGBA. `linear_to_srgb`, `srgb_to_linear` and `luminance`
  (Rec. 709) are scalar helpers. `resource_key` and `split_resource_key` tag a bitmap name with
  its color space.
- **Height to normal and ORM packing.** Both read a `ScalarSource`: a plane of linear scalar
  samples that gives its size and one row at a time, so an adapter can decode a tiled or encoded
  surface in place. Besides the output, each worker keeps at most three rows for the normal map
  and one row for each given plane for ORM packing, never a full `f32` copy of the input. Both
  check the output size against `Limits` before reading. An `Image` is a
  `ScalarSource` of its gray value, or its Rec. 709 luminance for a color image.
  - `normal::try_height_to_normal` returns an RGBA tangent-space normal map. It takes the 3×3
    Sobel gradient of the height, scales it by `strength` and normalizes `(-gx × s, gy × s, 1)`,
    then stores `0.5 × n + 0.5` with alpha 1, as glTF 2.0 encodes normal textures. OpenGL
    normals have green up the image; `NormalConvention::DirectX` negates green. Neighbors clamp
    to the image, or with `BoundaryMode::ClampToBlock` to the `block × block` UV tile of the
    center sample. A neighbor that `ScalarSource::covered` reports as not covered takes the
    center height. `HeightNormalOptions::uv_tiles(height_strength, tile_res, block_scale)` gives
    the strength `height_strength × tile_res × block_scale / 16` and clamps to UV tiles of
    `tile_res`. A strength of 0 gives flat normals `[0.5, 0.5, 1]`.
  - `packing::try_pack_orm` returns RGBA `[occlusion, roughness, metallic, 1]`, the channels
    glTF 2.0 reads occlusion from (red) and metallic-roughness from (green and blue). A plane
    that is not given takes 1, 0.5 or 0. `occlusion_multiplier` multiplies a second occlusion
    plane, such as baked occlusion, into red. Every plane must have the same size, and `size`
    gives it when no plane is given.
  - Values are linear data with no transfer curve. A NaN or infinite strength, sample or block
    scale, a block of 0, an occlusion product that overflows `f32`, mismatched sizes and a
    failed row read return a `TextureOpError`.

Mirrored repeat, decoding and texture-set recognition are not in the crate yet.

## Cargo features

No default features.

| Feature | Provides | Pulls in |
|---|---|---|
| `parallel` | generated images, display conversion, height to normal and ORM packing fill rows on several threads; the results are the same | `rayon` |

Without `parallel` the crate builds for `wasm32-unknown-unknown`.

## Limits

| Field | Default | Checked by |
|---|---|---|
| `max_side` | 8192 | every `try_` constructor, `from_data`, `try_height_to_normal` and `try_pack_orm` |
| `max_pixels` | 2^26 (8192 × 8192) | the same |
| `max_bytes` | 1 GiB (an 8192 × 8192 RGBA plane) | the same |
| `MipLimits::max_chain_bytes` | 1.5 GiB (a chain over an 8192 × 8192 RGBA plane needs about 1.33 GiB) | `MipChain::try_build`, `from_levels` and `required_bytes`, with `MipLimits::image` for each level |

## Origin

| Part | Source |
|---|---|
| `Image`, `Kind`, sampling, resizing, conversions, sRGB helpers, resource keys and the six tests `sample_survives_huge_coordinates`, `resource_keys_round_trip`, `sample_wraps_and_clamps`, `resize_box_and_bilinear`, `hostile_sizes_are_clamped`, `conversions` | MaterialCraft `crates/image/src/lib.rs` at [3133062](https://github.com/storytold/materialcraft/commit/3133062968b8a1566eda12a357cb06062db16687) |
| `get_axes`, `sample_axes`, `sample_nearest_axes`, `sample_stored` and the integer-sampling flag | MaterialCraft `crates/image/src/lib.rs` at [6c3b35c](https://github.com/storytold/materialcraft/commit/6c3b35c39632feafcda3d7a90ff18242f3ad910f), on this crate's coordinate reduction and `f64` bilinear sum |
| `Limits`, the `try_` constructors, `from_data`, private fields with getters, the `parallel` feature, `with_integer_sampling` and `integer_sampling` | written here |
| The Sobel normal, its strength scale, UV-tile clamp and uncovered-neighbor rule, the ORM channels and defaults, and the tests `gltf_png_matches_the_composite`, `directx_flips_green`, `height_becomes_normal_detail`, `ao_mixes_baked_and_channel`, `converted_maps_invert_and_derive_normals` and `uv_tiles_export_one_file_per_tile_without_padding_across`, with assertions on the new functions | TextureCraft `crates/formats/src/export/mod.rs`, `presets.rs` and `tests.rs` and `crates/core/src/channel.rs` at [b89badc](https://github.com/storytold/texturecraft/commit/b89badcd6f883751f79960f93c554fb77f063885) |
| `ScalarSource`, row-at-a-time reading, the limit and non-finite checks, and the tiled-input tests | written here |
| `mip`: mip chains, LOD and level selection, and their tests | written here |

## Migration

| App | Status |
|---|---|
| MaterialCraft | `materialcraft-image` is the source. The app switches to this crate in its own pull request. |
| EffectCraft | Its texture sampling moves into this crate later, with mirrored repeat. |
| TextureCraft | Its export's height normal and ORM packing are the source of `normal` and `packing`. The app switches to them in its own pull request. |

For MaterialCraft:

| `materialcraft-image` | craft-texture |
| --- | --- |
| `image.w`, `image.h`, `image.kind`, `image.data` | `width()`, `height()`, `kind()`, `data()` and `shared_data()` |
| `Image { w, h, kind, data }` built by hand | `Image::from_data(w, h, kind, data, &Limits::default())` |
| `image.integer_sampling = on` and `Image { integer_sampling, .. }` | `image.with_integer_sampling(on)`; read it with `integer_sampling()` |
| `sample_axes` blends with `a + (b − a)t` in `f32` | it sums the weighted texels in `f64`, as `sample` does; the results differ by rounding |
| `rayon` always on | the `parallel` feature |
| `linear_to_srgb(NaN)` returned −0.055 | it returns NaN |
