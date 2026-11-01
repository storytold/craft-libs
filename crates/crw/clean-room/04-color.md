# CRW colour, geometry and metadata records (P3/P5)

Status: derived 2026-10-13, after the compression freeze. Everything here comes from the
approved sources: the corpus files themselves (public CC0 + the maintainer's private D60s),
the oracle's decoded sensor data, each file's own embedded JPEG, and ExifTool's prose tag
documentation (`TagNames/CanonRaw.html`, `TagNames/Canon.html` — tag names, table indices and
value enumerations only; fetches logged in `sources.md`). No decoder source was consulted.
Implemented in `crates/raw/src/vendor/crw.rs`.

## 1 CFA phase

Determined by two independent sample-derived tests on each body's oracle mosaic:

- **Chroma correlation**: bin the mosaic 2×2 into RGB under each of the four Bayer phase
  hypotheses, roughly gain-match to the body's own embedded JPEG (linearised ≈ γ2.2), and
  correlate the R−B chroma plane with the JPEG's. The true phase correlates strongly
  (+0.49…+0.94), its R/B swap strongly negatively (`cfa_check.py`, local model dir).
- **Green diagonal**: within 16×16 blocks the two green sites have near-equal sums; the
  diagonal with the smaller |difference| carries the greens.

Results: **RGGB at sensor (0, 0)** for EOS D30, D60, 10D, 300D (both tests decisive) and
PowerShot G6; **GBRG for the PowerShot G1** (greens on the main diagonal; chroma +0.77 GBRG
vs −0.67 GRBG). Unsampled PowerShots default to RGGB, unverified.

## 2 Active area, black level, white level

- **Active area**: SensorInfo (0x1031) values 5–8 are an inclusive crop (left, top, right,
  bottom); valid on every corpus body (G5/G6 included).
- **Black level**: measured from the masked borders. Observed (oracle data, per body class):
  the left border is dark up to 8 columns before the active area on the EOS bodies
  (≈127–129 DN on D60/10D, ≈128 on D30, ≈124–127 on 300D) but transitions bright earlier on
  the G6 (dark only to col 40 of 64), and on the G1 only the top border is dark
  (rows 0–7 ≈ 29 DN; its left border reads ≈98–160). The D60's first ~7 sensor rows are
  clamped to 0 (not a black reference). Rule adopted: candidate strips = 8-column groups of
  the left border and 4-row groups of the top border; black = per-CFA-site mean of the
  darkest strip and all strips within 8 DN of it, ignoring strips with mean ≤ 1 (the clamped
  rows). This lands on the true dark level on every corpus body. Cross-check: the trailing
  quadruple of ColorBalance (0x10a9) is (126–127)×4 on D60/10D and (124–125)×4 on 300D —
  matching the measured borders (ExifTool's prose notes this quadruple as "black levels").
- **Correction (2026-10-13):** the first implementation also averaged in the clamped strips
  it meant to ignore (mean ≤ 1 passes "within 8 DN of the darkest"), which put the D60's
  level at 117 instead of 128 and lifted every shadow. Fixed and tested; on 20 private D60
  files the left border's mean is within 2 DN of the record's black quadruple, per site
  (127–137, higher on the files with the larger calibration-row value).
- **White level**: saturation plateau of the decoded samples (shared `white_from_data`);
  without a plateau, the level at least 64 samples reach when that is within 3 % of the
  full range (`white_level`); else the full range, 4095 (12-bit) / 1023 (G1).
- **The sensor clips below 4095 at low ISO** (2026-10-13, 1,516 private D60 files by ISO
  from ShotInfo value 2, ISO = 100·2^(v/32 − 5)): at ISO 100, 200 and 400 no sample exceeds
  4009 and the maxima cluster at 4000–4008; at ISO 800 and 1000 saturated files reach 4095.
  The 10D and 300D samples stop at 4003–4008 as well. Before `white_level`, a photo with a
  small blown area had no plateau and got 4095 (373 of 925 files at ISO 100, 283 of 287 at
  ISO 400), leaving blown pixels at 0.977 of white, below the clip threshold.
- **Controlled frames from the maintainer** (lens cap and blown-out, ISO 100 and 1000; all
  four decode sample-exact). Cap, ISO 100: active area 129.2–130.1 per site, left border
  128.2–129.6, our level 128.9, record 127–129. Cap, ISO 1000: active area 129.5–130.5,
  but left border 134.5–135.7, our level 133.8, record 131–133: at high ISO the masked
  border and the record read about 4 DN above the image's true dark level. One frame, so
  no correction is applied; noise there is 15 DN. Blown-out, ISO 100: the common values
  are 3997–4000 and the maximum 4002 (our level 3987). Blown-out, ISO 1000: every active
  sample is 4095 (our level 4080).

## 3 As-shot white balance

- **0x10a9 ColorBalance** (D60/10D/300D only): a byte-length u16, then (R, G, G, B) level
  quadruples per white-balance setting, the last quadruple being black levels. ExifTool's
  prose gives the 10D/300D slot order — Auto, Daylight, Shade, Cloudy, Tungsten, Fluorescent,
  Flash, Custom, Kelvin, Black — at indices 1, 5, …, 37, which the record sizes confirm
  (82 bytes = 1 + 10×4 u16); the D60's 66-byte record holds 7 slots + black at index 29
  (= ExifTool's "D60 black levels" index), i.e. the same order without Shade and Kelvin.
- Multipliers are **level / green** (e.g. D60 slot 1: 1856, 784, 781, 945 → R 2.37, B 1.21).
  Confirmed against the cameras' own JPEGs by a white-preserving matrix fit (rows sum to 1,
  least squares from WB-corrected binned raw to the linearised JPEG): plausible slots score
  far better than implausible ones; among near-daylight slots the fit cannot fully separate
  (γ-model error), so the slot is chosen by metadata, not by fit.
- **Slot selection**: ShotInfo (0x102a) index 7 is the WhiteBalance setting (ExifTool prose:
  0 Auto, 1 Daylight, 2 Cloudy, 3 Tungsten, 4 Fluorescent, 5 Flash, 6 Custom, 8 Shade,
  9 Kelvin). Every corpus file outside the preset captures below reads 0 (Auto) → slot 0.
- **D60 preset mapping verified** (2026-10-14): the maintainer shot one overcast scene at
  each setting, Auto, Daylight, Cloudy, Tungsten, Fluorescent, Flash and Custom, ISO 100,
  within a minute (all seven decode sample-exact). Slots 1–6 hold the same multipliers in
  every file; slot 0 (Auto) varies slightly per shot. On the pixels each camera JPEG
  renders neutral, the multipliers the camera applied are nearest to slot 1 for Daylight,
  2 for Cloudy, 4 for Fluorescent and 5 for Flash, i.e. setting n uses slot n. Tungsten and
  Custom leave too few neutral pixels for that test (5 and 0), so they were checked by
  rendering: the overall tint of our render follows the camera JPEG for every setting
  (Tungsten JPEG R/G 0.57 B/G 1.41, ours 0.66 / 1.52; Custom 0.62 / 2.93, ours 0.80 /
  3.02), which no other slot would give. Shade and Kelvin (10D/300D only) remain untested.
- **Auto slot confirmed on the private D60 files** (2026-10-13): on the pixels each camera
  JPEG renders neutral (channel spread < 6 of 255, mid-tone, raw unclipped), the median raw
  G/R and G/B give the multipliers the camera applied. On 17 files with enough neutral
  pixels, slot 0 is within 0.005–0.036 (summed log error) of that on every one, flash
  shots included, and is the nearest slot on 16 (the 17th prefers Daylight by 0.005).
  Numbers only; no image content recorded.
- **0x10aa** (same three bodies): one (R, G, G, B) quadruple with G ≈ 1024 and R/B as
  *levels* (multiplier = G/level); consistent with a measured-white-balance record (Canon
  maker-note 0xaa MeasuredColor). On some files it fits the camera JPEG better than slot 0,
  on others worse; not used, kept as a documented alternative.
- **Not derived yet**: D30 and PowerShot white balance (no 0x10a9/0x10aa; the 0x1030
  WhiteSample record holds an 8×8 packed sample patch — constant filler on G1/G6, varying on
  D30 — undeciphered). These bodies render with the neutral fallback.
- **Colour matrices**: none of the CRW bodies is in `spectral_table.rs`. Rendering uses
  the engine's per-file starting look fitted to the file's own embedded JPEG
  (`camera_preview::file_local_look` lists CRW; `docs/camera-preview-colour.md`) — accepted
  on all 10 decodable public corpus files, holdout RMS 0.009–0.043, including the D30 and
  the PowerShots, whose missing white balance the fitted matrix absorbs.
  **Update 2026-10-25:** the EOS D60 has dual-illuminant matrices of our own, fitted to
  ColorChecker Classic shots under a tungsten bulb and direct sun (the fitting is part of
  LightCraft's colour work, `crates/raw/src/chart_table.rs`). They give its white balance a Kelvin scale and are its
  colour when the JPEG fit fails; the look stays the JPEG fit. The other CRW bodies have none.

## 4 Orientation and capture time

- ImageInfo (0x1810) value 3 is rotation in degrees. The two portrait 300D corpus files
  carry 270, and their scene is upright after rotating the stored image 90° counter-clockwise
  (checked visually against the embedded JPEG) → 270 maps to Exif orientation 8, hence
  90 → 6 and 180 → 3. All other corpus files carry 0.
- TimeStamp (0x180e) value 0 is a Unix time on the camera's local clock (cross-checked
  against the files' Exif-era siblings and plausibility); decoded as a civil time with no
  UTC offset.
- Make/model: 0x080a, two NUL-terminated strings ("Canon", "Canon EOS D60"); the G5
  refusal and the G1 CFA special-case key on the model string.
