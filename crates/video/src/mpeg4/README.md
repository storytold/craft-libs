# MPEG-4 Part 2 video encoder (`craft_video::mpeg4`, feature `mpeg4`)

Clean-room MPEG-4 Visual (ISO/IEC 14496-2) encoder in pure Rust: the format Xvid and DivX write.
It writes Simple profile streams. Depends on `craft-media` (`bitstream`, `CodecParams`) and
`thiserror`. No `unsafe`, no panics. No Xvid, DivX, FFmpeg or other MPEG-4 implementation was
read. ffmpeg and ffprobe are used only as external decoders in the tests.

There is no MPEG-4 Part 2 decoder in this crate. `craft-media` names the stream (`CodecId::Mpeg4Video`)
in MP4, Matroska, AVI and MXF.

## Specifications

| Document | Used for |
|---|---|
| ISO/IEC 14496-2 | visual object sequence, visual object, VOL and VOP headers (§6.2), macroblock and block syntax, the VLC tables of Annex B, intra DC prediction (§7.4.3), the second inverse quantisation method (§7.4.2), the IDCT (Annex A) and its saturation, motion vector coding and prediction (§7.6), the intra refresh rule (§7.4.5) |

## API

```rust
use craft_video::mpeg4::{Encoder, EncoderConfig, RateControl, YuvFrame};

let mut cfg = EncoderConfig::new(1280, 720, 30000, 1001);
cfg.rate = RateControl::Bitrate { kbps: 2500 }; // or RateControl::Quant(1..=31)
cfg.keyint = 250;
cfg.headers_in_band = false;                     // MP4 / Matroska: headers in the track
let mut enc = Encoder::new(cfg)?;
let params = enc.codec_params();                 // Mpeg4Video, `mp4v`, size, extradata
for frame in frames {
    let packet = enc.encode(&YuvFrame { y, u, v, y_stride, uv_stride })?;
    // packet.data: one VOP; packet.keyframe; packet.pts (frame number); packet.quant
}
```

- Input is 8-bit 4:2:0. Width and height are even, 2 to 8190.
- One picture in, one VOP out, in order. There are no B-VOPs and no delay.
- `extradata()` holds `VisualObjectSequence` through `VideoObjectLayer`. That is the MP4 `esds`
  `DecoderSpecificInfo` and the Matroska `CodecPrivate`. With `headers_in_band` (the default),
  the extradata also starts every I-VOP packet. A raw `.m4v` stream or an AVI file needs this.
- `reconstruction()` returns the picture a decoder reconstructs from the last packet.
- `request_keyframe()` codes the next picture as an I-VOP.

## Coding

- Headers:
  - Simple Object Type.
  - `profile_and_level_indication` is the smallest Simple profile level whose picture size holds
    the picture (L3, L4a, L5, L6). Bitrate and VBV limits are not checked. Pictures over 3600
    macroblocks are signalled as L6, which is larger than that level allows.
  - Rectangular, progressive, 4:2:0, `low_delay`, H.263 quantisation (`quant_type` 0).
  - No resync markers, no data partitioning.
  - Fixed VOP rate when the reduced frame rate allows it.
  - Pixel aspect ratio: a table code where one fits, otherwise the extended form.
  - Optional colour description.
- Time code: `vop_time_increment_resolution` is the reduced frame rate numerator (16 bits at
  most). `modulo_time_base` counts the seconds between VOPs.
- I-VOPs: every macroblock is intra. DC is predicted from the left or upper block (§7.4.3) and
  coded with the DC size tables. AC prediction is off (`ac_pred_flag` 0), so the scan is always
  zigzag.
- P-VOPs:
  - One half-sample vector per 16×16 macroblock (no 4MV), with `fcode` 1 or 2 per VOP.
  - The motion search tries the median predictor, the left and upper vectors and zero, then
    runs a diamond search in whole samples (cost: SAD plus quantiser × vector bits), then
    refines to the 8 half-sample positions around the best.
  - Vectors are limited so that the part of a macroblock inside the picture reads only reference
    samples inside the picture. The prediction therefore does not depend on how a decoder pads
    its reference.
  - `vop_rounding_type` alternates between P-VOPs.
- Mode decision in P-VOPs:
  - Intra when the macroblock's deviation from its mean, plus 500, is below the best SAD.
  - Skipped (`not_coded`) when the vector is zero and nothing quantises to non-zero.
  - Every macroblock is coded intra again after 128 inter codings. §7.4.5 requires this within
    132, to bound IDCT mismatch with the decoder.
- Quantisation:
  - Intra AC: `|c| / 2q`.
  - Inter: `(|c| - q/2) / 2q`.
  - Intra DC: rounded to `dc_scaler` (Table 7-1).
  - Levels are capped at 2047.
- Coefficients use Tables B-16 (intra) and B-17 (inter). Escape modes 1 (level offset) and 2
  (run offset) are used where they fit; otherwise the fixed-length mode 3.
- DCT and IDCT are separable in `f64`. The IDCT rounds and saturates to [-256, 255].
- Rate control:
  - `Quant(q)`: every VOP at quantiser `q`.
  - `Bitrate`: a 1-pass average. Each VOP's quantiser (2–31) comes from a bits × quantiser model
    kept separately for I and P, aiming at the frame budget minus 1/25 of the running overshoot.
    I-VOPs aim at 4 frame budgets. The quantiser changes by at most 4 between VOPs of a type.

## Not supported

- B-VOPs, quarter-sample motion, global motion compensation (Advanced Simple profile).
- AC prediction, 4MV, `dquant` within a VOP.
- MPEG quantisation matrices.
- Interlace, shapes, sprites, scalability.
- Resync markers, data partitioning, reversible VLCs.

## Tests

- Unit tests:
  - The coefficient tables are prefix codes and the intra and inter tables use the same codes.
  - Every other VLC table is prefix-free.
  - DC scaler, dequantisation, chroma vector rounding, motion vector wrapping, and the predictor
    rules at picture edges.
  - DCT round trip.
- Code table oracle (`encode/tests.rs`): every table entry, every escape mode 1 and mode 2
  event, intra and inter. The table-coded stream must decode in ffmpeg byte for byte like the
  same coefficients sent with the fixed-length escape. Swapping two table entries fails it.
- Integration tests (`tests/mpeg4_encode`) decode with ffmpeg and check:
  - no diagnostics and the right frame count;
  - every frame within 48 dB of the encoder's reconstruction, and above a PSNR floor against the
    source;
  - quantisers 1 to 31, partial macroblocks (18×34, 98×66, 2×2);
  - a 150-frame GOP, which exercises the intra refresh and the time code across seconds;
  - extreme content at quantiser 1;
  - out-of-band headers given as extradata;
  - the bitrate mode within 15% of its target.
- ffprobe reads the size, aspect ratio, colour description and profile.

The oracle tests skip when ffmpeg is absent, unless `CRAFT_MEDIA_REQUIRE_ORACLES=1` is set.
