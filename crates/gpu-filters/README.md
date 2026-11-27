# craft-gpu-filters

Image filters for the Craft apps' layer effects, on premultiplied RGBA8 images: a separable
Gaussian blur, per-channel lookup tables on straight colour (levels and curves), and premultiplied
source-over compositing with an opacity. `FilterProvider` is the `craft.filters` provider of the
`craft-gpu` job layer (RFC-0022). `GpuFilters` runs the filters as wgpu compute passes on a device
the caller holds, and `cpu` has the same filters on the CPU, the reference the passes are tested
against.

Licensed under MIT OR Apache-2.0.

## Contents

| Item | What |
|---|---|
| `Filter` | `GaussianBlur { sigma }`, `Lut(Box<Lut>)` or `Composite { opacity }`; `inputs()` is how many images it reads, `validate()` checks its parameters |
| `Lut` | `tables: [[u8; 256]; 4]`, red, green, blue and alpha; `Lut::identity()` changes nothing |
| `FilterProvider` | the `craft.filters` `GpuProvider`; `new()` filters on the CPU on any `GpuContext`, `with_device(device, queue)` filters with the compute passes on that device (not on `wasm32`) |
| `FilterJob` | one job's filter and image size; `params()` writes them, `parse` reads them, `job(inputs)` makes the `GpuJob` |
| `PROVIDER_ID` | `"craft.filters"` |
| `GpuFilters::new(device, queue)` | the compute pipelines on one device, made on first use; `apply(filter, width, height, inputs)` runs a filter and waits for the result (not on `wasm32`); `buffer_limit()` is the largest buffer the device lets it bind |
| `cpu::apply`, `cpu::gaussian_blur`, `cpu::lut_apply`, `cpu::composite` | the same filters on the CPU |
| `kernel(sigma)` | the blur weights both paths use |
| `MAX_SIDE`, `MAX_PIXELS`, `MAX_SIGMA` | 16384 pixels a side, 64 Mi pixels an image, sigma 200 |
| `Error` | why a filter did not run |

## Images

An image is `width × height` pixels, tightly packed, four bytes a pixel in the order red, green,
blue, alpha, with the colour premultiplied by alpha. A colour channel above its pixel's alpha is
read as the alpha, so no output is ever out of range, whatever the input bytes. Both sides run from
1 to `MAX_SIDE`, and an image holds at most `MAX_PIXELS` pixels.

## Filters

**Gaussian blur.** The kernel reaches `r = ceil(3 sigma)` pixels to each side, with weights
`exp(-i² / (2 sigma²))` scaled to sum to 1 (`kernel`); a sigma of 0 changes nothing. The blur runs
on the premultiplied channels, a horizontal pass and then a vertical one, and pixels outside the
image are transparent black, so the edges of a layer fade. The horizontal pass keeps 16 bits a
channel; the vertical pass rounds to 8 bits and holds each colour channel to its alpha. Sigma must
be a finite number from 0 to `MAX_SIGMA` (200); NaN, a negative or a larger sigma is
`Error::Sigma`.

**Lookup tables.** The tables apply to straight colour. For each pixel, with premultiplied colour
`c` and alpha `a`:

1. the straight colour is `s = (c·255 + a/2) / a` in integers, and 0 when `a` is 0;
2. the new alpha is `a' = tables[3][a]`;
3. the new premultiplied colour is `(tables[ch][s] · a' + 127) / 255`.

The identity tables give back every valid premultiplied pixel unchanged. A pixel with no alpha has
straight colour 0, so an alpha table that raises 0 shows `tables[ch][0]` there.

**Composite.** The layer, scaled by `opacity`, over the backdrop. The opacity, a finite number from
0 to 1, is quantized to `o = round(opacity · 255)`. With `x·y/255` rounded as `(x·y + 127) / 255`,
the layer is `s = s·o/255` and each output channel, alpha included, is `s + d·(255 − s_alpha)/255`.

## Tolerances

The compute passes follow the CPU reference step for step, with the same weights in the same order.

| Filter | Largest difference from `cpu` in any channel |
|---|---|
| Gaussian blur | 1 (a device may fuse a multiply and an add, so a sum on a half may round either way) |
| Lookup tables | 0 (integer arithmetic) |
| Composite | 0 (integer arithmetic) |

`oracle_tests.rs` checks these on a real device at sizes that do and do not fill a 16 × 16
workgroup, with valid and with hostile pixels, and sigmas from 0 to a kernel wider than the image.
When the machine has no graphics adapter, those tests print a note and pass without running: the
no-adapter path RFC-0022 defines. Everything else runs on `craft-gpu-null`.

## Jobs

```rust
let mut registry = craft_gpu::ProviderRegistry::new();
registry.register(Box::new(craft_gpu_filters::FilterProvider::new()));
let input = ctx.upload_buffer("layer", &rgba)?;
let job = craft_gpu_filters::FilterJob::new(Filter::GaussianBlur { sigma: 4.0 }, 1920, 1080).job(&[input]);
let outcome = registry.dispatch(&mut ctx, &job)?;
let blurred = ctx.read_buffer(outcome.outputs[0])?;
```

A job's inputs are RGBA8 buffers of exactly `width × height × 4` bytes: one image, or for a
composite the backdrop and then the layer. It names no outputs; its outcome has one, a new buffer
holding the result. The parameters are ASCII `key:value` fields separated by `;`: `op` (`blur`,
`lut` or `over`), `w` and `h` in decimal digits, then `sigma` for `blur`, `opacity` for `over`, or
`r`, `g`, `b` and `a` for `lut`, each table as 512 hexadecimal digits. For example
`op:blur;w:1920;h:1080;sigma:2.5`.

On every context, the null one included, the provider holds the filter's largest buffer to the
smaller of the context's `max_buffer_size` and `max_storage_buffer_binding_size`: the image for the
lookup tables and the composite, the blur's 8-bytes-a-pixel intermediate image for the blur. It
checks every input's length before reading any. A malformed, missing, repeated, unknown or
inapplicable parameter, a value out of range, a wrong input count or length, and a size the caps or
the limits do not allow are `GpuFault::InvalidJob`. An error the device reports and a failed
allocation are `GpuFault::Operation`. A fault from the context itself, such as
`GpuFault::DeviceUnavailable` or `GpuFault::DeviceLost`, comes back as it is.

`FilterProvider::new()` runs `cpu::apply`, so a consumer's tests and CI run on `craft-gpu-null`
without a graphics adapter. `FilterProvider::with_device` runs the compute passes on the given
device and waits for each job; the inputs and the output still move through the context's buffers.
Every device call runs inside error scopes, so a device error is an `Error::Device`, never a panic
from wgpu's uncaptured-error handler, and pipelines that failed are not reused.

## Not here

The crate turns on no wgpu backend: the caller's device brings its own. The passes read back to
the host after each filter; recording them into a caller's own encoder over its own buffers, and
other filters (sharpen, colour matrices, displacement), are left for when an app needs them.
