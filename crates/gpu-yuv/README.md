# craft-gpu-yuv

RGBA to 4:2:0 Y'CbCr on the GPU for the Craft apps' encoders and scopes: NV12, P010 and I420,
scaled to the output size, in the BT.601, BT.709 and BT.2020 matrices at limited or full range.
`cpu::convert` is the same conversion on the CPU. The crate depends only on `wgpu` 30.

Licensed under MIT OR Apache-2.0.

## Contents

| Item | What |
|---|---|
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

## Sampling

The source is scaled to the output size; the caller keeps the aspect ratio. Each output texel
(luma at the output size, chroma at half the output size in both directions) takes the average
of up to 4 × 4 bilinear taps spread over the part of the source it covers: 1 tap per axis while
the source is at most twice the plane's size, and one more for each further 2:1 of reduction.
At the same size, each chroma sample is the average of the 2 × 2 pixels it covers, and its
position is the center of those pixels. A reduction of more than 8:1 along an axis skips part of
the source between taps.

The passes write unsigned-integer textures (`R8Uint`, `Rg8Uint`, `R16Uint`, `Rg16Uint`). The shader rounds
each value half to even. P010 does not need wgpu's 16-bit norm feature.

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

The CPU tests check standard code values (BT.709 limited-range red is 63, 102, 240), each
format's plane order, the area average of a reduction and the read-back layout. The GPU tests
convert pictures with hard edges at the same size, and smooth pictures at five scales, in every
format, and compare each sample with `cpu::convert`. The tolerance is 2 code values at 8 bits and
6 at 10 bits. A machine without a graphics
adapter skips the GPU tests and prints that it did so.
