# craft-gpu-yuv

RGBA to 4:2:0 Y'CbCr for the Craft apps' encoders and scopes: NV12, P010 and I420, scaled to the
output size, in the BT.601, BT.709 and BT.2020 matrices at limited or full range. `YuvProvider` is
the `craft.yuv` provider of the `craft-gpu` job layer. `Converter` records the same conversion as
render passes into a caller's own encoder, and `cpu::convert` is the same conversion on the CPU.

Licensed under MIT OR Apache-2.0.

## Contents

| Item | What |
|---|---|
| `YuvProvider` | the `craft.yuv` `GpuProvider`; `new()` converts on the CPU on any `GpuContext`, `with_device(device, queue)` converts with the render passes on that device (not on `wasm32`) |
| `YuvJob` | one job's conversion, source size and output size; `params()` writes them, `parse` reads them, `job(input)` makes the `GpuJob` |
| `PROVIDER_ID` | `"craft.yuv"` |
| `Format` | `Nv12` (8-bit luma, then interleaved Cb and Cr), `P010` (10-bit values in the high bits of little-endian 16-bit words, same planes as NV12) and `I420` (8-bit luma, Cb, then Cr) |
| `Matrix`, `Range` | BT.601, BT.709 and BT.2020 non-constant luminance; limited range (luma 16 to 235, chroma 16 to 240, times 4 at 10 bits) or full range (every code, chroma centered on 128 or 512) |
| `Conversion` | one format, matrix and range |
| `Converter::new(device, conversion)` | the two render pipelines of one conversion; no per-frame state |
| `Converter::bind(device, view)` | a source texture view bound once, as a `Source` |
| `Converter::planes(device, width, height)` | the plane textures of one output size, as `Planes` |
| `Converter::draw(encoder, source, planes)` | records a luma pass and a chroma pass |
| `Planes::copy_to_buffer(encoder, buffer, offset)` | records the copies of every plane into a read-back buffer |
| `FrameLayout` | each plane's size, its offset and row pitch in a read-back buffer (`buffer_size`), and the size of a packed frame (`frame_bytes`); `unpack` copies a mapped buffer into a packed frame |
| `cpu::convert(rgba, src_width, src_height, conversion, width, height)` | the conversion of an RGBA8 picture on the CPU, as one packed frame |

## Jobs

A host registers `YuvProvider` in a `ProviderRegistry` and dispatches jobs made by `YuvJob::job`:

```rust
let mut registry = craft_gpu::ProviderRegistry::new();
registry.register(Box::new(craft_gpu_yuv::YuvProvider::new()));
let input = ctx.upload_buffer("rgba", &rgba)?;
let job = craft_gpu_yuv::YuvJob::new(conversion, 1920, 1080, 1280, 720).job(input);
let outcome = registry.dispatch(&mut ctx, &job)?;
let frame = ctx.read_buffer(outcome.outputs[0])?;
```

A job has one input, an RGBA8 buffer of exactly `src_width × src_height × 4` bytes, and no
outputs; its outcome has one output, a new buffer that holds the packed frame. The parameters are
ASCII `key:value` fields separated by `;`:
`fmt:nv12;matrix:bt709;range:limited;sw:1920;sh:1080;w:1280;h:720`. `fmt` is `nv12`, `p010` or
`i420`, `matrix` is `bt601`, `bt709` or `bt2020`, `range` is `limited` or `full`, and the four sides
are decimal digits.

The provider checks the job against the context's `DeviceCaps` on every context, the null one
included: each source and output side must be at most `max_texture_dimension_2d`, and the
read-back layout with its padded rows (`FrameLayout::buffer_size`) at most `max_buffer_size`
bytes. It checks the input's length before reading it. A
malformed, missing, repeated or unknown parameter, a wrong input count or length, and a size the
caps or the conversion do not allow are `GpuFault::InvalidJob`. With a device, the provider also
checks the device's own limits, and an error the device reports during the job is
`GpuFault::Operation`. Each conversion's pipelines are made once per provider.

`YuvProvider::new()` runs `cpu::convert`, so a consumer's tests and CI run on `craft-gpu-null`
without a graphics adapter. `YuvProvider::with_device` runs the render passes on the given device
and waits for each job to finish; the input and the output still move through the context's
buffers.

## Plane layout

A packed frame of `w × h` (both even) holds its planes one after another, each row without
padding:

| Format | Plane | Offset | Row bytes | Rows |
|---|---|---|---|---|
| NV12 | Y' | 0 | `w` | `h` |
| NV12 | Cb, Cr interleaved | `w·h` | `w` | `h/2` |
| P010 | Y' | 0 | `2w` | `h` |
| P010 | Cb, Cr interleaved | `2w·h` | `2w` | `h/2` |
| I420 | Y' | 0 | `w` | `h` |
| I420 | Cb | `w·h` | `w/2` | `h/2` |
| I420 | Cr | `w·h + w·h/4` | `w/2` | `h/2` |

A frame is `1.5·w·h` bytes in NV12 and I420 and `3·w·h` bytes in P010. Each P010 word holds the
10-bit value in its high bits, with the 6 low bits zero. NV12 and I420 follow the W3C WebCodecs
definitions (<https://www.w3.org/TR/webcodecs/#enumdef-videopixelformat>), and NV12 and P010 follow
the Linux media API's sample images
(<https://docs.kernel.org/userspace-api/media/v4l/pixfmt-yuv-planar.html>).

In a read-back buffer, `FrameLayout` pads each row to a multiple of
`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT` (256) bytes, as a texture-to-buffer copy needs, and puts each
plane right after the previous plane's padded rows. `FrameLayout::unpack` removes the padding.

## Code values

Luma and chroma follow BT.601-7 section 2.5, BT.709-6 items 3.2 to 3.5 and BT.2020-2 tables 4 and
5 at limited range, and BT.2100-3 table 9 at full range. A half rounds away from zero, as `INT` in
BT.709 and BT.2020 and `Round` in BT.2100 do, so a full-range Cb or Cr that computes to 0.5 is
code 1.

## Sampling

The source is scaled to the output size; the caller keeps the aspect ratio. Each output texel
(luma at the output size, chroma at half the output size in both directions) takes the average
of up to 4 × 4 bilinear taps spread over the part of the source it covers: 1 tap per axis while
the source is at most twice the plane's size, and one more for each further 2:1 of reduction.
At the same size, each chroma sample is the average of the 2 × 2 pixels it covers, and its
position is the center of those pixels. A reduction of more than 8:1 along an axis skips part of
the source between taps.

`cpu::convert` scales with these same taps rather than with a resampling crate such as
`fast_image_resize`, because it is the reference the shader is tested against, and a different
kernel would differ from the shader at every edge. A caller that wants another filter on the CPU
can resize first with such a crate and convert at the same size.

The passes write unsigned-integer textures (`R8Uint`, `Rg8Uint`, `R16Uint`, `Rg16Uint`), so P010
does not need wgpu's 16-bit norm feature.

## Limits

- The matrix applies to the stored values: the source must already be in the output's primaries
  and transfer function. An sRGB view of the source gives linear values, which the matrix does
  not expect; bind a non-sRGB view. Alpha is ignored, so premultiplied pixels convert as if they
  were over black.
- Output sides must be even and not zero (`Error::OutputSize`), and no larger than the device's
  largest 2D texture (`Error::TooLarge`). A layout whose byte sizes do not fit in `u32` rows, `u64`
  offsets or a `usize` frame is `Error::Overflow`. `Converter::planes` refuses a read-back layout
  larger than the device's largest buffer (`Error::BufferLimit`), and `cpu::convert` refuses an
  output side over `cpu::MAX_SIDE` (16384).
- The source view must be a 2D view of a filterable float format, such as `Rgba8Unorm`,
  `Bgra8Unorm` or `Rgba16Float`.
- `copy_to_buffer` needs an offset that is a multiple of `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`
  (`Error::Offset`) and a buffer of at least `buffer_size` bytes from it (`Error::BufferSize`).
  `draw` refuses planes made for another format (`Error::Mismatch`).

## Tests

- `oracle_tests.rs` checks the conversion against sources outside the crate. It computes the
  code values of 729 colors in `f64` from each recommendation's Kr, Kb and quantization, and
  checks the nominal levels the recommendations state (white, black, neutral chroma and the
  chroma extremes). It compares every format, matrix and range with the `yuv` crate's conversion
  of the same pictures (`yuv` 0.8, a dev-dependency), within 1 code value at 8 bits and 2 at
  10 bits. It checks the packed plane sizes against the planes `yuv` allocates, and a 4 × 4 frame
  against the byte offsets of the public format definitions. The GPU passes are checked against
  the same equations, within 1 code value.
- `job_tests.rs` runs `craft.yuv` through a `ProviderRegistry` on `craft-gpu-null`, and with a
  device. It checks the parameters, the inputs and outputs, the caps limits, and a device whose
  limits are smaller than the context's.
- `tests.rs` checks standard code values, each format's plane order, the area average of a
  reduction and the read-back layout. It also compares the GPU passes with `cpu::convert` on
  pictures with hard edges at the same size and on smooth pictures at five scales, within 2 code
  values at 8 bits and 6 at 10 bits.

A machine without a graphics adapter skips the GPU tests and prints that it did so.
