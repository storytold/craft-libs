# MPEG-4 Part 2 video (`craft_video::mpeg4`, features `mpeg4-decode` and `mpeg4-encode`)

Clean-room MPEG-4 Visual (ISO/IEC 14496-2) decoder and encoder in pure Rust: the format Xvid and
DivX write. The decoder reads the Simple and Advanced Simple profiles; the encoder writes Simple
profile streams. Feature `mpeg4` turns on both. Depends on `craft-media` (`bitstream`,
`CodecParams`) and `thiserror`. No `unsafe`, no panics. Both were written from the standard; no
Xvid, DivX, FFmpeg or other MPEG-4 source was read. ffmpeg is used only in the tests, as a
writer of test streams and as an external reference decoder.

`craft-media` names the stream (`CodecId::Mpeg4Video`) in MP4, Matroska, AVI and MXF.

The decoder and encoder share the Annex B code tables (`tables.rs`) and the IDCT (`dct.rs`).

## Specifications

| Document | Used for |
|---|---|
| ISO/IEC 14496-2 | visual object sequence, visual object, VOL, group of VOP and VOP headers (§6.2), video packets (§6.2.5), macroblock and block syntax, the VLC tables of Annex B, intra DC and AC prediction (§7.4.3), both inverse quantisation methods with mismatch control (§7.4.2), the IDCT (Annex A) and its saturation, motion vector coding and prediction (§7.6), half- and quarter-sample interpolation (§7.6.2), interlaced prediction and direct mode (§7.6.9), global motion compensation (§7.8), the intra refresh rule (§7.4.5) |

## Decoder

### API

```rust
use craft_video::mpeg4::Decoder;

let mut dec = Decoder::new();
dec.decode_headers(&extradata)?;              // MP4 esds / Matroska CodecPrivate, if any
for (chunk, pts) in packets {                   // container frames, in decode order
    for pic in dec.decode(&chunk, pts)? {       // display order
        // pic.y, pic.u, pic.v: tight 4:2:0 planes, pic.width × pic.height
        // pic.vop_type, pic.pts, pic.time (VOP time stamp), pic.quant, pic.info
    }
}
let rest = dec.flush();                         // the last reference picture
let info = craft_video::mpeg4::probe(&headers); // StreamInfo without decoding
```

- `decode` takes any chunk holding whole VOPs: a container frame (which may pack a P-VOP and a
  B-VOP, as DivX and Xvid store them in AVI), or several frames of a raw `.m4v` stream. It returns
  at most 16 pictures a call, fewer when the memory budget holds fewer beside the references
  (queued pictures count at their own size); the rest of a chunk past that limit is skipped and
  counted in `errors()`.
- Pictures come out in display order. With `low_delay` (no B-VOPs) each picture comes out as soon
  as it is decoded; otherwise a reference picture is held until the next one.
- An uncoded VOP (`vop_coded` 0: the placeholder after packed B-VOPs, or a dropped frame)
  produces no picture.
- A damaged VOP is still output. The damaged part is copied from the previous picture and
  decoding resumes at the next video packet. `errors()` counts such VOPs.
- `reset()` forgets the reference pictures and the time base, and keeps the configuration
  headers.
- A VOL that changes the picture size or turns on `low_delay` first outputs the picture held
  back for reordering. A VOL that is refused (`Unsupported`, `TooLarge`) ends the old layer: VOPs
  fail with the same error until a decodable VOL arrives.
- `set_memory_budget()` bounds the picture store (`craft_video::budget`); a larger VOL is
  refused with `DecodeError::TooLarge`.

### Coding tools

- I-, P-, B- and S-VOPs. Rectangular, 8-bit, 4:2:0.
- Intra DC and AC prediction, with the alternate scans.
- H.263 quantisation, and MPEG quantisation with the default or loaded matrices.
- One or four vectors a macroblock, half- or quarter-sample, `fcode` 1–7.
- B-VOPs: forward, backward, interpolated and direct macroblocks; skipped macroblocks where the
  future reference skipped them.
- Global motion compensation in S-VOPs with zero to three warping points at every accuracy,
  `mcsel` macroblocks, and the mean warped vector of a GMC macroblock as a neighbour's predictor.
- Interlace: field DCT, field prediction in P- and B-VOPs, field direct mode, the alternate
  vertical scan.
- Video packets (resync markers), with or without a header extension.
- Group of VOP headers, and the DivX and Xvid user data strings (named in `StreamInfo::encoder`).

### Interpretations

Where the text leaves room, the decoder does what Xvid and FFmpeg do, checked against ffmpeg's
output:

- A reference picture is padded from the edge of its whole macroblocks, not from the VOP's width
  and height. This differs only for pictures that are not a multiple of 16.
- A reference field is a view of the padded frame: a field line past the bottom of the picture
  repeats the frame's last line, which belongs to the bottom field.
- With quarter-sample field vectors, the chrominance vector divides the horizontal component by
  2 rounding towards zero and the vertical one rounding down.
- An uncoded VOP advances the time base but is not a reference for the temporal distances of
  direct mode.

Two cases where the decoder follows the standard and ffmpeg does not:

- Field direct mode scales the co-located macroblock's field vectors (§7.6.9.5). ffmpeg's decoder
  predicts these macroblocks from the vector difference alone. On ffmpeg's own interlaced
  streams with B-VOPs this decoder's B pictures are closer to the source, by up to 2.4 dB.
- The mean vector of a GMC macroblock is rounded to the nearest integer. With one warping point
  at half-sample accuracy, ffmpeg rounds a negative mean one lower.

### Not supported

Refused with `DecodeError::Unsupported`. Xvid and DivX write none of these.

- Data partitioning and reversible VLCs.
- Shapes, static sprites, sprite brightness change, scalability.
- NEWPRED, reduced resolution VOPs, complexity estimation headers, OBMC.
- Samples other than 8 bits, chroma formats other than 4:2:0.
- The short video header (H.263 baseline streams).

Workarounds for streams written by early Xvid and DivX builds with known bugs are not
implemented.

### Performance

One thread, no explicit SIMD; the interpolation loops are written so the compiler vectorises
them. On 1280×720 streams from ffmpeg's `mpeg4` encoder at 3 Mbit/s, the decoder takes about 2.5
times as long as ffmpeg's decoder on one thread for a Simple profile stream, and about 1.8 times
as long for an Advanced Simple one (B-VOPs, quarter sample, four vectors, MPEG quantisation).
The largest remaining cost is the shared `f64` IDCT, which is kept so that the decoder and the
encoder reconstruct the same samples.

`examples/mpeg4_m4vdec.rs` decodes a raw `.m4v` stream; `MPEG4_BENCH_ITERS=n` times it.

### Tests

- Unit tests: every VLC decodes to its table entry, the escape modes, the scans are
  permutations, the DC scaler, vector wrapping, the field and GMC interpolation on known input,
  header field widths, user data strings, and garbage input.
- Integration tests (`tests/mpeg4_decode`) encode streams with ffmpeg's `mpeg4` encoder and
  compare every picture with ffmpeg's decode (45 dB floor; IDCT mismatch between decoders
  accumulates over a GOP, a decoding error costs far more):
  - I/P at quantisers 1, 4 and 31, partial macroblocks (98×66, 50×38);
  - B-VOPs, four vectors, AC prediction, quarter sample and their combination;
  - MPEG quantisation with default and custom matrices;
  - video packets, large vectors (352×288, `me_range` 60);
  - clean content with skipped and uncoded macroblocks;
  - interlaced P-VOPs; interlaced B-VOPs are checked against the source instead (see above).
- GMC: hand-built S-VOPs with one, two and three points at each accuracy, with and without
  quarter sample, after an I-VOP of this crate's encoder, against ffmpeg.
- Packed B-VOPs with uncoded VOPs give the same pictures as the plain stream.
- The decoder reproduces this crate's encoder's reconstruction exactly.
- Corrupted streams (truncations, bit flips, overwrites) never panic; a garbled video packet is
  concealed.

## Encoder

### API

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

### Coding

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

### Not supported

- B-VOPs, quarter-sample motion, global motion compensation (Advanced Simple profile).
- AC prediction, 4MV, `dquant` within a VOP.
- MPEG quantisation matrices.
- Interlace, shapes, sprites, scalability.
- Resync markers, data partitioning, reversible VLCs.

### Tests

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
