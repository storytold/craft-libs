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

The encoder accepts interleaved 8-bit, 16-bit, or HDR floating-point samples. Floating-point input
is linear-light and is currently available only with the Rec. 2020 PQ option; one input unit maps
to 203 cd/m². Gray input is expanded to RGB, and opaque alpha is omitted. ICC data is embedded in
lossless output; Exif and XMP boxes are written when provided. Encoding is unavailable on
`wasm32` targets.

`jxl-oxide` is MIT OR Apache-2.0. `jixel` is BSD-3-Clause OR Apache-2.0.
