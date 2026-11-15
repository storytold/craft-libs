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
  texels give a finite sample.
- **Other operations.** `get` reads one texel, `map`, `to_gray`, `to_color` and `to_kind` convert,
  `resize` box-averages whole-factor shrinks and samples bilinearly otherwise, and
  `to_rgba8_display` encodes 8-bit sRGB RGBA. `linear_to_srgb`, `srgb_to_linear` and `luminance`
  (Rec. 709) are scalar helpers. `resource_key` and `split_resource_key` tag a bitmap name with
  its color space.

Mirrored repeat, mip chains, height to normal, channel packing, decoding and texture-set
recognition are not in the crate yet.

## Cargo features

No default features.

| Feature | Provides | Pulls in |
|---|---|---|
| `parallel` | generated images and display conversion fill rows on several threads; the results are the same | `rayon` |

Without `parallel` the crate builds for `wasm32-unknown-unknown`.

## Limits

| Field | Default | Checked by |
|---|---|---|
| `max_side` | 8192 | every `try_` constructor and `from_data` |
| `max_pixels` | 2^26 (8192 × 8192) | the same |
| `max_bytes` | 1 GiB (an 8192 × 8192 RGBA plane) | the same |

## Origin

| Part | Source |
|---|---|
| `Image`, `Kind`, sampling, resizing, conversions, sRGB helpers, resource keys and the six tests `sample_survives_huge_coordinates`, `resource_keys_round_trip`, `sample_wraps_and_clamps`, `resize_box_and_bilinear`, `hostile_sizes_are_clamped`, `conversions` | MaterialCraft `crates/image/src/lib.rs` at [3133062](https://github.com/storytold/materialcraft/commit/3133062968b8a1566eda12a357cb06062db16687) |
| `Limits`, the `try_` constructors, `from_data`, private fields with getters, the `parallel` feature | written here |

## Migration

| App | Status |
|---|---|
| MaterialCraft | `materialcraft-image` is the source. The app switches to this crate in its own pull request. |
| EffectCraft | Its texture sampling moves into this crate later, with mirrored repeat. |

For MaterialCraft:

| `materialcraft-image` | craft-texture |
| --- | --- |
| `image.w`, `image.h`, `image.kind`, `image.data` | `width()`, `height()`, `kind()`, `data()` and `shared_data()` |
| `Image { w, h, kind, data }` built by hand | `Image::from_data(w, h, kind, data, &Limits::default())` |
| `rayon` always on | the `parallel` feature |
| `linear_to_srgb(NaN)` returned −0.055 | it returns NaN |
