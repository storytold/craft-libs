# craft-jxl

JPEG XL decoding and encoding for still raster images.

The decoder reads a codestream incrementally, checks its dimensions before loading the remainder,
and applies a caller-selected compressed-input, pixel-count, and tracked-decoder-allocation budget.
`jxl-oxide`'s allocation tracker cannot account for every allocation made by the decoder, and the
input slice is already owned by the caller. The decoder returns the first frame as interleaved
`f32` samples together with source and rendered color metadata. It does not perform an external
ICC transform; callers that need color-managed output should use the returned profiles with their
CMS. CMYK output is rejected because this crate does not bundle an external CMS.

`decode_with_color` can request linear Rec. 2020 for enum-encoded images. XYB is rendered through
linear sRGB and converted to Rec. 2020 to avoid the known wide-gamut XYB rendering issue in
`jxl-oxide`. If a source embeds an ICC profile, the decoder keeps the source-profile samples and
returns the profile; it does not claim to transform ICC data without a CMS. `decode` retains the
decoder's default rendering target.

Brotli-compressed metadata boxes (`brob`: Exif, XMP) are inflated before the file reaches
`jxl-oxide`, which would inflate them outside its allocation tracker (a 54-byte box can expand to
64 MiB and more). All the `brob` boxes of one file share an 8 MiB budget; a box past it, a nested or
broken one, is dropped (the pixels still decode) and the others are handed over as the plain box
they wrap. A file without a `brob` box is not copied.

The encoder accepts interleaved 8-bit, 16-bit, or HDR floating-point samples. Floating-point input
is linear-light and is currently available only with the Rec. 2020 PQ option; one input unit maps
to 203 cd/m². Gray input is expanded to RGB, and opaque alpha is omitted. ICC data is embedded in
lossless output; Exif and XMP boxes are written when provided. Encoding is unavailable on
`wasm32` targets.

`jxl-oxide` is MIT OR Apache-2.0. `jixel` is BSD-3-Clause OR Apache-2.0.
