# Sony ARW lens corrections

> Moved from LightCraft (`docs/sony-ilce7rm6-lens-corrections.md`) with the decoder (`src/arw_lens.rs`, `src/arw_lens_rm6.rs`). It records how the tables were derived and verified; mentions of LightCraft's settings and UI describe that app.

Sony cameras record three lens-correction tables in the raw IFD of every ARW: distortion, vignetting and lateral
chromatic aberration (CA). LightCraft reads them and turns distortion into a DNG `WarpRectilinear` opcode, so it
reaches the same **Optics ▸ Enable Lens Corrections** (`optics.lens_profile`) as a DNG's embedded warp and Panasonic's
RW2 distortion data. **Lateral CA is corrected on the raw mosaic, before demosaic** ([Lateral CA before
demosaic](#lateral-ca-before-demosaic)), for Bayer ILCE-7RM6 files with 3:2 image and reference crops and a verified
lens. Enable Lens Corrections governs distortion only, as Lightroom applies a raw's built-in corrections whatever
its profile switch says (inferred, as for imported presets' `LensProfileEnable=0`). The warp's red and blue
planes stay off because applying them after demosaic made the render's CA worse (Verification). Vignetting is
read but not applied (below). The conversion is verified on the **ILCE-7RM6** and, for distortion only, the
**ILCE-6700** ([ILCE-6700](#ilce-6700)) and **ZV-E10** ([ZV-E10](#zv-e10)). The ILCE-7RM4A has its own
conversion, with other conventions
([Two conventions](#two-conventions)), and other Sony bodies' tables give no correction until they are verified.

The correction is on by default for new imports, as for any file with embedded lens data (`Photo::camera_defaults`).
Photos imported before this change keep their settings: **Photo → Reload from Disk** adds the lens data and, when the
photo is still at its import settings, turns the correction on as one undoable step; the automatic reload after an
external editor saves the file does the same. Edited photos keep their settings. The Profile Distortion slider
scales the warp as for DNG and RW2.

## The tables

ExifTool's names (EXIF table, `SubIFD` group); each table is a count-prefixed array of signed 16-bit values
(`SSHORT`, 17 / 17 / 33 values), and each has an in-camera on/off tag:

| Table | Tag | Flag tag | Count (full frame / APS-C crop mode) | Flag values seen |
|---|---|---|---|---|
| Distortion (`DistortionCorrParams`) | `0x7037` | `0x7036` | 16 / 11 | 0 (off) or 1 (on) |
| Vignetting (`VignettingCorrParams`) | `0x7032` | `0x7031` | 16 / 11 | 2 on all 54 photos |
| Lateral CA (`ChromaticAberrationCorrParams`) | `0x7035` | `0x7034` | 32 / 22 (red, then blue) | 1 on all 54 photos |

The first value is the knot count; only the first `count` values are used. Full-frame files carry 16 / 16 / 32 knots;
APS-C crop-mode files 11 / 11 / 22, padded to the same array length (distortion with its last value repeated,
vignetting and CA with zeros). The CA table holds red's `count / 2` knots, then blue's. The corrections are built
whatever the flags say: the flag only records what the camera did to its own JPEG. Tables with a count outside
2–16 (CA: an odd count or outside 4–32), a count that doesn't fit the array, or non-integer values are ignored.

## Measured conventions

None of this is documented; it was measured against the camera's own JPEGs (method below). Residuals are in px at a
1440 px long edge.

- **Knots and centre.** Knot `i` sits at sensor radius `i · s` from the default-crop centre, with
  `s = hypot(9984, 6656) / 30` = 399.98 px: the full-frame default crop's half diagonal divided by 15, so knot 15 is
  the full-frame crop corner. The spacing is fixed on the sensor: APS-C files use the same pixel spacing (verified on
  CA, rms 0.004–0.010 px against 0.011–0.014 px for `i/15` of the APS-C half diagonal; `i/10` of it, 392 px, fits as
  well). Values are interpolated linearly between
  knots; beyond the last knot the last segment is extended (it matters only in the outermost ~0.4 knot spacing at
  the corners of a zoomed-out barrel correction). The centre is the default-crop centre: the measured translation
  between our render and the camera JPEG has median 0.013 px, max 0.216 px.
- **Distortion** is relative and indexed at the corrected radius, in units of 1/15000: a corrected radius `r`
  reads the source radius `S(r) = r · (1 + v(r) / 15000)` (positive values: pincushion; negative: barrel). The camera
  then zooms so the output fills the default crop: output radius = `z` · corrected radius, with `z` chosen so that
  the largest `S(ρ / z) / ρ` over the crop's border radii `ρ` is 1 (binding at the corner for pincushion, at the
  short-edge midpoint for barrel). `z` was 0.971–1.035 on these photos. Without the zoom every hypothesis fails
  (24.6 px); with it, the worst residual over the 34 measurable corrected photos was 0.596 px (rms 0.143; corners and
  edge midpoints 0.570 px), within the 0.75 px budget for a unit between about 14700 and 15350. A global
  least-squares fit gives 15000.7.
- **Lateral CA** is relative to the green source radius, at that radius: red's source radius is
  `g · (1 + c_R(g) / 3.36·10⁶)`, with `g` the green source radius (after the distortion correction above) and `c_R`
  the red half on the same knots; blue likewise with the second half. Positive values: that colour's image is larger
  than green's. **The unit is provisional.** The measurement is borderline: on the 15 photos with CA above 0.15 px
  that a single unit explains, the best (minimax) unit 2.26·10⁶ leaves 0.305 px against a 0.3 px budget; per-photo
  best units scatter from 1.5·10⁶ to 7.2·10⁶ (about ±30 % scene bias: the RawDB landscape and portrait shots of one
  scene, with nearly identical tables, measure 3.4–3.5·10⁶ and 4.5·10⁶). The chosen 3.36·10⁶ is the median of the 20
  cleanest photos: worst 0.399 px on the 15, and the rms raw CA over all 46 measured photos falls from 0.079 to
  0.052 px. Swapping the halves or an absolute form (`v · norm / U`) is clearly worse. `g` is a sensor radius, so the
  table applies to the raw mosaic as it is, with no distortion ([Lateral CA before
  demosaic](#lateral-ca-before-demosaic)).
- **Vignetting: not applied.** The camera JPEGs show no vignetting compensation relative to our uncorrected render:
  over the 39 photos whose measurement reaches the corners, the corner/centre ratio JPEG / render has median 0.996
  (p10 0.907, p90 1.087), while the tables would predict a corner gain with median +32 % for `1 + v / 2¹⁴` (+16 %
  for `v / 2¹⁵`, +63 % for `v / 2¹³`). Regressing the measured corner EV on the predicted one gives slope 0.009 ±
  0.063. No model (`1 + v/U`, `U/(U − v)`, `2^(v/U)`) at any unit from 2¹³ to 2¹⁸ reaches 3 %; the identity is the
  best of all. The raw data isn't pre-compensated either: the read noise of three dark high-ISO frames is flat from
  centre to corner, where a digital gain from these tables would raise it by 60–80 %. So the table probably describes
  the lens's natural falloff, but this camera's JPEGs don't apply it, and nothing shows its unit or model. Options:
  (a) skip it, so renders match the camera JPEG (the current ruling); (b) apply it with an unverified model for a
  Lightroom-like look; (c) shoot flat fields (a white wall at several apertures, one lens) and measure the model.

## Conversion to `WarpRectilinear`

`src/arw_lens_rm6.rs` builds one `WarpRectilinear` in `OpcodeList3`, centred on the default-crop centre
(DNG pixel-index units relative to the active area; radius 1 = the farthest active-area corner, `m`):

1. `S` from the distortion table and the zoom `z` (bisection on [0.5, 2], 4097 border radii); the green map is
   `G(r) = S(r / z)`.
2. The map is sampled at `u = i / 64`, i = 0…64 (65 samples of `G(u · m) / m`) and fitted by least squares with
   DNG's 4-term odd polynomial `u · (k0 + k1 u² + k2 u⁴ + k3 u⁶)` (`vendor::fit_radial_warp`, shared with RW2).
3. **Fit gate:** the plane is refused when its fit is worse than 0.25 px at a 1440 px long edge at a sample the crop
   shows: the samples up to the first one at or beyond the crop corner. The lossless files' padded active area
   (10240 × 7168 for a 9984 × 6656 crop) reaches radii that are never shown (`u` up to 0.925 is shown there).
   Over all 54 photos the fit's worst error at all 65 samples was 0.220 px (a lossless file; ARW6 files at most
   0.175 px). Between samples the error is somewhat larger: 0.071 against 0.049 px for the full-frame 16–35 mm
   example in the tests.
4. **One plane, G**, applied to all three channels, always: the warp carries no CA (lateral CA is corrected on the
   mosaic, below). No opcode when the distortion table is absent or all zero, the zoom has no solution in [0.5, 2],
   the G fit fails, the frame isn't 3:2 or the file is linear (the downsized lossless M and S sizes). A refused
   table leaves the photo uncorrected rather than wrongly warped.

The opcode then goes through the same pipeline stage as a DNG's (`crates/pipeline/src/optics.rs`): it rotates with
the photo (portrait files are covered) and is carried by DNG export, as RW2's is.

## Lateral CA before demosaic

`arw_lens_rm6::lateral_ca` turns the CA table into a `LateralCa` map (`src/lateral_ca.rs`), carried in
`OpcodeLists::lateral_ca` next to the DNG opcode lists. It is not a DNG opcode and is never written to a DNG. This
crate only builds the map; the kernel and the stages below are LightCraft's (`lightcraft-raw`), which corrects it.

- **The map** is sensor to sensor and needs no distortion. Where green's image of a point sits at sensor radius `ρ`
  about the default-crop centre, red's sits at `ρ · (1 + c_R(ρ) / 3.36·10⁶)`, and blue's likewise. The knots are at
  `i · 399.98` px ([Measured conventions](#measured-conventions)), linear in between, with the last segment extended.
  Each knot is bounded by `MAX_SCALE` = 2·10⁻³, which is 6720 in table units; the largest value in the 54 photos'
  tables is 1724. A table with a value beyond the bound, a malformed table or an all-zero one gives no map. The map
  applies even when the distortion table is absent.
- **The kernel:** every red (blue) site of the mosaic is resampled from red (blue) sites only, at the position the map
  gives. It uses a 4 × 4 Catmull-Rom (a = −0.5) on that colour's half-resolution plane, clamped to the [min, max] of
  the central 2 × 2 samples, so there is no ringing and no new extreme. Green is untouched. A site whose 4 × 4 support
  holds a clipped or non-finite sample keeps its own value, so a blown highlight never turns into a below-clip value
  that highlight reconstruction would take for colour.
- **Where it runs:** LightCraft's `RawImage::normalized()`, right after `OpcodeList2`. Every demosaic (AHD, PPG, bilinear) and AI
  Denoise therefore see the corrected mosaic. Binned previews and thumbnails bin the mosaic without the map and then
  resample the binned red and blue with the same kernel (the clip mask stands in for the clip level). No path
  corrects twice. The map applies to Bayer ILCE-7RM6 files with 3:2 image and reference crops and a lens it
  measurably improves ([Limits](#limits)). Linear files (the downsized lossless M and S sizes) and in-camera
  aspect crops get neither the map nor the distortion warp.
- **Why there:** the colour-difference demosaic (AHD) borrows green's edges for red and blue, so a full-size render
  shows only 0.29 × the lens CA measured on the raw mosaic. Applied after demosaic, the table (the earlier design: a
  warp with three colour planes) therefore overshot. A binned preview averages each colour's own sites, so it showed
  about twice the CA of a full-size render (0.54–0.60 × the raw CA), and preview and export disagreed. On the mosaic,
  the table acts on the CA it describes, and every level gets the same correction ([Verification](#verification)).
- **Engine:** the camera-look colour fit bins its proxies without the map, as it was tuned, so looks don't move.
  Pooled camera profiles see no lens corrections either. AI Denoise products of ARW photos are keyed apart once, so
  they are made again from the corrected mosaic.

## Which bodies get it

Only bodies on one of two allowlists in `arw_lens_rm6.rs` get an opcode: `FULL_FRAME_CROP` (today `ILCE-7RM6`,
9984 × 6656) and `APSC_BODIES` (today `ILCE-6700` and `ZV-E10`, below, each with its distortion unit).
Only `FULL_FRAME_CROP` bodies get the CA map. The unit and the knot spacing are verified per body; a body
with another sensor size or another unit would be warped wrongly. Other bodies get no correction from this conversion;
the ILCE-7RM4A gets its own, with other conventions (below). To add a body: collect photos of it shot with distortion
correction on (several lenses and focal lengths), repeat the camera-JPEG registration below against the uncorrected
render, confirm the unit, the knot spacing and the zoom rule (or measure the body's own). Then add a full-frame body's
default crop to `FULL_FRAME_CROP`, or an APS-C body whose knots span its own crop (as measured for the ILCE-6700) to
`APSC_BODIES`, with a test; an APS-C body also needs its fit gate checked against the camera JPEG (see *ILCE-6700*).

For bodies in `APSC_BODIES` (today `ILCE-6700` and `ZV-E10`), the knots span the file's own default crop: spacing = its half
diagonal / (knot count − 1). This crop-based rule applies only to that list. The ILCE-7RM6 keeps its sensor-fixed
400 px in both modes; in full frame (16 knots) the crop-based rule gives the same 400 px, and in its APS-C mode
(11 knots) it would give 392 px, which was only an alternative measurement there (it fits as well, above).

## ILCE-6700

The ILCE-6700 (APS-C, 6192 × 4128 default crop) writes 11-knot tables like the ILCE-7RM6's APS-C mode. Measured as
above on 30 photos shot with distortion correction Auto: Sigma 10–18 mm F2.8 DC DN (9 photos, 10–18 mm), Tamron
17–70 mm F2.8 B070 (13, 17–70 mm) and Sony E 70–350 mm G (8, 70–350 mm), all private, only the summary numbers here.
The convention is the ILCE-7RM6's (corrected-radius index, unit 15000, frame-filling zoom), with the knot spacing
the APS-C half diagonal / 10 = 372.09 px:

| | p95 residual at 1440 px |
|---|---|
| Uncorrected | 3.8–5.6 px (19 photos; on the other 11 the shift exceeds the method's ±20 px search) |
| Sensor-fixed 400 px (the ILCE-7RM6's pixel spacing) | 0.56–3.71 px (passes only at 10 mm) |
| **Half diagonal / 10 = 372 px, unit 15000** (exact table) | **0.23–0.68 px on 27 photos (median 0.43); 1.34–1.42 px on the three 10 mm photos** |
| The same as LightCraft renders it (`lightcraft-cli render`, Enable Lens Corrections on, fitted polynomial) | **0.22–0.73 px on 27 photos (median 0.44); 1.36–1.38 px at 10 mm** |

Units 14000–17000 and a reading that drops the first table value as a non-knot (10 knots over the half diagonal,
413 px) were also scanned; none keeps all 30 within 0.75 px, and 15000 gives the lowest worst case at 372 px. The
residual is radial (tangential median ≤ 0.02 px, so the centre is right) and has the same shape on every lens,
positive at mid radii and negative at the corners, growing with the table's magnitude (10 mm: last knot −1333, 1.3 px
at the corners). Fitted per photo, the best spacing and unit trade off against each other and drift with the lens
(10 mm about 400 px, 18 mm about 365 px): the same lens-dependent effect as the per-lens units above. The 10 mm end
stays within the 0.5–1.5 px accepted for Nikon Z lenses (`docs/nikon-lens-corrections.md`) and removes about 75 % of
the uncorrected 5.6 px.

**Fit gate.** The 11 knots over the smaller crop bend more sharply than `WarpRectilinear`'s polynomial follows: on the
decoded geometry (active area 6656 × 4608) it fits the 30 tables within 0.08–0.44 px, and 13 of them exceed the
full-frame 0.25 px gate. Measured against the camera JPEG, the fitted polynomial is as good as the exact table (p95
0.22–0.73 px against 0.23–0.68 px on the 27 photos, 1.35–1.38 px at 10 mm), so APS-C bodies use a 0.5 px gate
(`APSC_FIT_ERROR_PX_AT_1440`).

**Lens exception: Sigma 10–18 mm F2.8 DC DN at 10–10.5 mm** (`LENS_EXCEPTIONS`, matched on Exif `LensModel`
`10-18mm F2.8 DC DN | Contemporary 023` and `FocalLength` ≤ 10.5 mm). Its wide end keeps 1.36–1.38 px under the
body's rule. On 115 further private photos of this lens (97 at 10 mm, 2 at 10.4–10.5 mm, 16 at 11.7–12.5 mm), a grid
over spacing 372–426 px and unit 14000–16500 was measured on the exact table. The photos at 10 mm were split by
shooting location: 63 to choose the values, 34 to check them. A larger spacing with a smaller unit fits about as well
(a ridge in the grid): 420 px / 14000 was best on the 63 choice photos (0.445 px worst) but put 2 of the 34 check photos
above 0.75 px, so the pick is **416 px / 14250**, within 0.51 px on both sets.
As rendered (`lightcraft-cli render`, Enable Lens Corrections on):

| Sigma 10–18 mm | p95 at 1440 px |
|---|---|
| 10 mm, 97 photos (one night photo has too little detail to measure) | 0.38–0.55 px, median 0.44 (body's rule: about 1.4 px) |
| 10.4–10.5 mm, 2 photos | 0.36 px |
| 11.7–12.5 mm, 16 photos (body's rule, the exception does not apply) | 0.48–0.72 px, median 0.57 |

At 11.7 mm and longer an exception of this kind fits worse than the body's rule (412 px / 14500 on the grid: 0.73–1.19 px against 0.48–0.69 px), so it stops at
10.5 mm, the longest measured focal length where it helps; 10.6–11.6 mm is unmeasured and takes the body's rule.

## ZV-E10

The ZV-E10 (APS-C, 6000 × 4000 default crop at (12, 12) in 6048 × 4024 storage) writes 11-knot tables like the
ILCE-6700, zero-padded to the same 17 values. Measured as above on 4 photos shot with distortion correction Auto (the
E PZ 16-50 mm F3.5-5.6 OSS, which forces it; its owner's photos, only the summary numbers here): 2 at 16 mm, 1 at 29 mm
(portrait) and 1 at 32 mm. Feature matches between the uncorrected render and the camera JPEG
(LightCraft's `tools/sony_distortion_check.py`, SIFT agreement after the best similarity), the camera's map applied to the render's
points in a scan of knot spacing 340–380 px and unit 15000–17800:

| | p95 residual at 1440 px |
|---|---|
| Uncorrected | 2.7–18.7 px (16 mm: 18.6–18.7 px) |
| The ILCE-6700's rule (half diagonal / 10 = 360.56 px, unit 15000), as rendered | 0.45–2.18 px: the 16 mm photos over-corrected by about 9 % |
| Sensor-fixed 400 px, unit 16384 | 0.43–3.21 px |
| **Half diagonal / 10 = 360.56 px, unit 16384** (exact table) | **0.27–0.61 px** |
| The same as LightCraft renders it (`lightcraft-cli render`, Enable Lens Corrections on, fitted polynomial) | **0.26–0.64 px** (median residual 0.07–0.36 px) |

At 360.56 px every unit from 16300 to 17000 lands within 0.60–0.64 px worst (the measurement's floor on these scenes);
spacing and unit trade off along a ridge as on the ILCE-6700 (375 px / 15700 and 380 px / 15400 reach 0.59 px), so the
body keeps the ILCE-6700's crop-based spacing and takes its own unit, 16384, in `APSC_BODIES`. The fitted polynomial
passes the 0.5 px APS-C fit gate on all four tables, the strongest (16 mm, −1905 at the last knot) included. An HDR
merge of a ZV-E10 bracket keeps the warp in the merged DNG's `OpcodeList3`, as for any raw with embedded corrections.

## Two conventions

A second Sony decoder, `src/arw_lens.rs`
([Sony embedded distortion corrections](sony-lens-corrections.md), PR #498), reads the same `0x7037` table on the
**ILCE-7RM4A**, validated against Sony Imaging Edge exports (distortion On/Off pairs at 200 and 600 mm with the
FE 200–600 mm; corrected TIFFs at 24 mm with the FE 24–105 mm). Its conventions differ from the ones above
(`r_o` = output radius about the crop centre, `v` = the table, linear between knots):

| | ILCE-7RM6 (this page, `arw_lens_rm6.rs`) | ILCE-7RM4A (`arw_lens.rs`) |
|---|---|---|
| Knots | `i · 399.98` px, fixed on the sensor; last segment extended | `i / 15` of the crop's half diagonal; radius clamped to 1 |
| Table indexed at | the corrected radius `r_o / z` | the output radius `r_o` |
| Unit | 15000 | 16384 |
| Zoom | fill: the largest `S(ρ / z) / ρ` over the border is 1 | `1 / max f` over the border radii |
| Aspect crops | refused (3:2 frames only; no warp and no CA map) | refused (native 3:2 only) |
| Tables | distortion (16 knots; 11 in APS-C); lateral CA on the mosaic before demosaic, always; vignetting read, not applied | distortion, 16 entries only |

`arw::decode` passes both the same active area and crop (the DNG-style default crop, on the ILCE-7RM6 the same as
Sony's crop tags); their model lists don't overlap, so a photo gets at most one of them: an ILCE-7RM6 this page's
conversion, an ILCE-7RM4A the other, every other body neither.

**Cross-check.** The ILCE-7RM4A formula, scored on the 34 measurable corrected ILCE-7RM6 photos exactly as in
Measured conventions (radial bins, px at 1440, max / rms; "c" = the worst corner or edge-midpoint point):

| Variant | Unit | All photos | Corners max / median | 24–70 GM II (23) | 100 mm Macro (8) | 200–600 G (3) |
|---|---|---|---|---|---|---|
| **A**: this page | 15000 | **0.596 / 0.143** | 0.570 / 0.099 | 0.596 / 0.100, c 0.474 | 0.361 / 0.135, c 0.336 | 0.588 / 0.325, c 0.570 |
| **B1**: the ILCE-7RM4A formula | 16384 | **1.429 / 0.319** | 1.913 / 0.204 | 1.429 / 0.385, c 1.913 | 0.289 / 0.049, c 0.234 | 0.264 / 0.115, c 0.320 |
| **B2**: B1 at its best single unit | 15680 | 0.940 / 0.223 | 1.202 / 0.098 | 0.940 / 0.215, c 1.202 | 0.327 / 0.100, c 0.278 | 0.683 / 0.428, c 0.637 |
| **B3**: A's indexing and zoom at 16384 | 16384 | 1.450 / 0.455 | 1.845 / 0.350 | 1.450 / 0.539, c 1.845 | 0.284 / 0.062, c 0.238 | 0.654 / 0.329, c 0.586 |

- B1 misses the 0.75 px budget on these photos, but only on the 24–70 GM II: at 24–28 mm it under-corrects the barrel
  in the outer half (up to 1.43 px at 0.7–0.85 of the half diagonal), at 45–70 mm the pincushion by a nearly uniform
  0.2–0.4 px. On the 100 mm and the 200–600 it fits better than A, and on the 200–600, the one lens both validations
  share, its unit agrees with this body's (best unit under its indexing 16524–16602). So the data does not show the
  two bodies differing.
- The zoom rules agree (within 0.0035 on every photo); the unit and the table index make the difference. Under A's
  indexing the 24–70 GM II's barrel and pincushion ends want the same unit (14749 / 14860, 0.7 % apart); under B's
  they need 14792 / 15705 (6 % apart).
- APS-C: B refuses 11-entry tables. Its knots read literally on the APS-C crop (`i / 15` of its half diagonal,
  261.5 px) fit the APS-C RawDB files' lateral CA 4 × worse on 3 of 6 photos than the sensor-fixed 400 px; `i / 10`
  (392 px) can't be told from 400 px.

**Open question: no single unit fits every lens under either convention.** The 200–600 wants a 5–7 % larger unit (a
weaker correction) than the 24–70 GM II's pincushion end with similar table values (+409 … +561 against +406 … +454
at the corner), under both indexings and every knot spacing tried (0.94–1.12 ×: the ratio of the two lenses' best
units stays 1.053–1.073). A's 15000 is the compromise that keeps every photo within 0.6 px; 16384 is the 200–600's
value. That rests on 3 measurable 200–600 photos (594–600 mm). The ILCE-7RM4A's own wide-angle evidence is its
weakest: its On/Off pairs are all 200–600, and its 24 mm check (LightCraft export against Sony's TIFF) is median 0.35 /
95th percentile 1.04 px at 2376 px. On its `WIDE` unit-test table the two formulas differ by up to 2.49 px at 1440 at
the corner (computed from the formulas, not measured).

**What would settle it:** Sony Imaging Edge On/Off exports of the 24–70 GM II (or any wide zoom's wide end) on either
body, which would show whether the wide-end disagreement is the body or the formula; and more lenses, and more
200–600 photos, on the ILCE-7RM6, to tell a per-lens unit from a convention. Until then each body keeps the
conventions it was validated with.

## Method

Photos: 54 ILCE-7RM6 ARWs: 42 private, and the 12 CC0 samples of the
[dnglab RawDB ILCE-7RM6 set](https://rawdb.dnglab.org/sets/Sony/ILCE-7RM6) (`ILCE-7RM6_{FF,APSC}_{land,port}_RAW{,-C,-C-HD}.ARW`:
full frame and APS-C, landscape and portrait, lossless, Compressed and Compressed HQ). `cargo xtask corpus --download`
fetches three of them: the APS-C lossless file as `arw-sony-ilce7rm6-apsc-land-lossless.arw`, and the ARW 6.0
decoder's full-frame Compressed and APS-C Compressed HQ files (`arw6-sony-ilce7rm6-ff-land-c.arw`,
`arw6-sony-ilce7rm6-apsc-land-hq.arw`). The corpus test `corpus_sony_lens_tables_become_opcodes` checks all three, and
the full-frame lossless file where it is present.
Four lenses: FE 24–70 mm F2.8 GM II (27 photos), FE 16–35 mm F2.8 GM II (12, all RawDB), FE 100 mm F2.8 Macro GM OSS
(9), FE 200–600 mm F5.6–6.3 G OSS (6). 41 photos have in-camera distortion correction on (all private); 13 have it
off (the 12 RawDB files and one private photo). Only aggregate numbers per lens are reported. The scripts are local
(the gitignored `plan/arw6/lens/`), not in the repository; the steps are described so they can be redone.

- **Oracle:** the camera JPEG inside each ARW (`JpgFromRaw`, 9984 × 6656 / 6528 × 4352, stored unrotated, turned to
  the photo's orientation). Tag values were read with ExifTool used as a tool. Clean room: no lens-correction code of
  any other project and no Adobe data was read.
- **Distortion (camera-JPEG block registration).** Our render at half the JPEG size and the JPEG (2 × 2 box-reduced)
  are both reduced to a quarter of the JPEG size; on the green channel, every 40 px a 64 px JPEG block is located in
  the render by normalised cross-correlation with parabolic sub-pixel refinement. Pass 1 searches coarse-to-fine
  (±28 px at 1/16 size, then ±6 px); pass 2 (used for every number) searches ±6 px around pass 1's smooth radial fit
  (data-derived, needed for a macro of a periodic screen). Blocks with low contrast (std < 2), NCC < 0.8 or a
  tangential displacement above 4 px are dropped. A robust fit of `p_render = g(r) · p_jpeg + t` (`g` an even
  4-term polynomial) gives the translation `t`; the measured radius ratio is the median of 32 radial bins (≥ 10
  blocks each). A photo is measurable with ≥ 300 blocks and ≥ 15 bins reaching 0.7 of the half diagonal: 34 of the
  41 corrected photos (two black frames, a near-black night frame, two low-detail frames and two whose blocks reach
  only 0.54 of the half diagonal are not). 108 hypotheses (radius norm, knot placement, relative or absolute, table
  indexed at the corrected or the source radius, zoom rule), each with its best unit, were compared.
- **Lateral CA (raw CFA).** The demosaiced render shows only 25–60 % of the lens's CA and the JPEG's 4:2:2 chroma
  and noise reduction hide it, so CA was measured on the raw: from LightCraft's uncompressed CFA DNG export, the
  red (blue) plane against green interpolated at the same sites (4-neighbour mean), square-root encoded, 2 × 2
  binned; 40 px blocks every 24 px, ±3 px NCC search, radial component, median per 16 radial bins.
- **Vignetting (exposure brackets).** The JPEG is resampled into the render's geometry; per pixel and channel, the
  exposure at which eight renders bracketed −0.3 … +1.8 EV equal the JPEG value is interpolated (smooth areas, JPEG
  values 12–240, monotone brackets). That cancels the fitted tone curve, since exposure is a scene-linear gain before
  it. Medians in 16 annuli give the JPEG's falloff relative to our uncorrected render.

## Verification

Every photo rendered with Enable Lens Corrections on and off at half the JPEG size (50 of the 54 are Compressed or
Compressed HQ ARW 6.0 files, so the renders used a local build of this code merged with the ARW6 decoder), then the same block
registration (passes 1 and 2) against the camera JPEG, and R and B against G inside each render (80 px blocks every
128 full-resolution px, ±4 px search; per corner the median radial shift of the 12 nearest blocks). px at a 1440 px
long edge; aggregates only.

An experimental build applied the lateral CA map before demosaic to all four measured lenses. Its results
were compared with the uncorrected renders in the same way. Every photo was rendered with Enable Lens Corrections
on at half the JPEG size (Full level: AHD)
and at 2496 px (Preview level: the mosaic binned 4×; APS-C 2×, then fitted). The uncorrected 2496 px renders come
from a build without lens corrections, because the CA map no longer follows the switch. On a synthetic sub-pixel
shift of a render's red plane, the block measurement reads 0.86–1.10 × the shift at both sizes.

**Distortion, photos corrected in camera** (our render with the correction on vs the camera JPEG; budget 0.75 px).
"Corners + midpoints" is the 2-D displacement at the four corners and four edge midpoints, global translation
included; "bins" the radial residual over 32 radial bins. A photo counts as measurable as in the method above.

| Lens | Photos (measurable) | Focal lengths | Corners + midpoints max / rms | Bins max / rms | Uncorrected, corners (per-photo max) |
|---|---|---|---|---|---|
| FE 24–70 mm F2.8 GM II | 26 (23) | 24–70 mm | 0.553 / 0.125 | 0.503 / 0.066 | 1.1–17.3 |
| FE 100 mm F2.8 Macro GM OSS | 9 (7) | 100 mm | 0.409 / 0.111 | 0.317 / 0.097 | 0.6–4.8 |
| FE 200–600 mm F5.6–6.3 G OSS | 6 (3) | 594–600 mm | 0.679 / 0.321 | 0.498 / 0.274 | 7.5–10.8 |
| **All** | **41 (33)** | | **0.679** / 0.150 (206 points) | 0.503 / 0.109 | 0.6–17.3 |

No photo is over budget in these renders (the re-measure on the CA-corrected renders is below); the global
translation is at most 0.056 px. The 8 not measurable are black, near-black or
low-detail frames, or frames whose blocks don't reach 0.7 of the half diagonal. That is one more than in the
measurement (33 measurable against 34): one 100 mm photo keeps only 255 of the 300 blocks needed when it is matched on
the corrected render, as block coverage differs slightly between the two renders. The 200–600 mm keeps the systematic
residual the measurement found for it (its own best unit is 15659, see Limits).

**Controls, correction off in camera** (13 photos: the 12 RawDB files, full frame and APS-C, and one private 24–70 mm
at 58 mm). Their JPEG keeps the lens's distortion, so our corrected render must differ from it by the model's own shift:
it does, by up to 10.5 px (full frame) and 11.4 px (APS-C), and matches that shift within 0.639 px at the corners and
midpoints (bins 0.259 px; full-frame RawDB files ≤ 0.218 px, APS-C ≤ 0.639 px). This checks that the pipeline renders
the model (also for APS-C, barrel, zoom 0.984), not that the APS-C model is right.

**Portrait photos:** 8 corrected (orientation 6 and 8) within 0.232 px, 6 controls within 0.369 px of the model's
shift: the warp turns with the photo.

**Lateral CA in the render, Full level** (radial R-vs-G and B-vs-G shift, median of the 12 blocks nearest each
corner; 43 photos with enough detail at all four corners; budget 0.3 px). *Off*: no CA correction. *Three planes*:
the earlier design, the table applied after demosaic as the warp's red and blue planes. *Before demosaic*:
the experimental map, including the 200–600 mm. All-lens aggregates in this section and the Preview section
summarize that experiment.

| Lens | Photos | Off: max R / B | Off: rms | Three planes: max R / B | Three planes: rms | Before demosaic: max R / B | Before demosaic: rms | Camera JPEG: max / rms |
|---|---|---|---|---|---|---|---|---|
| FE 24–70 mm F2.8 GM II | 23 | 0.089 / 0.071 | 0.028 | 0.219 / 0.148 | 0.069 | 0.070 / 0.098 | 0.024 | 0.150 / 0.036 |
| FE 16–35 mm F2.8 GM II | 12 | 0.125 / 0.054 | 0.041 | 0.030 / 0.051 | 0.020 | 0.058 / 0.056 | 0.025 | 0.058 / 0.016 |
| FE 100 mm F2.8 Macro GM OSS | 6 | 0.020 / 0.050 | 0.013 | 0.061 / 0.157 | 0.044 | 0.013 / 0.037 | 0.009 | 0.021 / 0.004 |
| FE 200–600 mm F5.6–6.3 G OSS | 2 | 0.033 / 0.101 | 0.032 | 0.146 / 0.075 | 0.070 | 0.099 / 0.181 | 0.068 | 0.113 / 0.039 |
| **All** | **43** | 0.125 / 0.101 | 0.032 | 0.219 / 0.157 | 0.052 | 0.099 / **0.181** | **0.026** | 0.150 / 0.028 |

- **The correction lowers the corner CA on three of the four lenses** (all photos: rms 0.032 → 0.026 px). Three
  planes after demosaic had raised it to 0.052 px.
- **It doesn't reach the spec's target** of rms ≤ 0.020 / max ≤ 0.08 px.
- **The 200–600 mm (2 photos) gets worse:** rms 0.032 → 0.068 px. The 0.181 px maximum is one corner of one 600 mm
  photo. There the raw mosaic shows almost no blue CA (−0.04 to +0.08 px in the outer radial bins), while the table
  asks for −0.05 to −0.12 px.
- **Photos:** 7 of the 43 have a corner rms more than 0.005 px above uncorrected (six 24–70 mm, one 200–600 mm).
- **Radial bins:** the largest radial-bin median is 0.109 px uncorrected, 0.226 px with three planes and 0.100 px
  corrected.
- **Against the raw mosaic:** over the 418 radial bins beyond 0.3 of the half diagonal, the corrected render shows
  0.12 × the lens CA measured on the raw mosaic (correlation 0.60). The bins' rms is 0.012 px, against 0.021 px
  uncorrected and 0.035 px with the three planes.

All of this is far below one pixel at 1440 px.

**Preview level** (2496 px renders, 26 photos with enough detail at all four corners at that size). Each photo's
global red and blue translation is removed first. Binning leaves the colour planes offset by a fraction of a binned
pixel (`binned.rs`; median 0.03 / 0.04 px at 1440 after correction, 0.03 / 0.10 px before), and a corner measurement
would read that offset as CA of alternating sign:

| Lens | Photos | Off: max R / B | Off: rms | Before demosaic: max R / B | Before demosaic: rms | Full level, same photos: rms |
|---|---|---|---|---|---|---|
| FE 24–70 mm F2.8 GM II | 11 | 0.121 / 0.398 | 0.097 | 0.034 / 0.053 | 0.026 | 0.023 |
| FE 16–35 mm F2.8 GM II | 11 | 0.106 / 0.192 | 0.066 | 0.095 / 0.185 | 0.055 | 0.023 |
| FE 100 mm F2.8 Macro GM OSS | 3 | 0.033 / 0.073 | 0.027 | 0.023 / 0.053 | 0.018 | 0.011 |
| FE 200–600 mm F5.6–6.3 G OSS | 1 | 0.194 / 0.228 | 0.198 | 0.051 / 0.077 | 0.040 | 0.024 |
| **All** | **26** | 0.194 / 0.398 | 0.084 | 0.095 / 0.185 | **0.043** | 0.022 |

- **Uncorrected previews showed about twice the CA of a full-size render:** corner rms 2.5 × that of the uncorrected
  full-size renders of the same photos, and 0.54 × the raw CA over the radial bins against 0.29 × at full size.
- **Corrected, both levels show 0.12–0.14 × the raw CA** (preview: 0.13 × with the translation removed, as the
  table; 0.14 × as measured).
- **Preview vs full-size render:** on the same photo, the rms over the corners of (preview − full size) exceeds
  0.05 px on 6 of these 26 photos, the largest 0.100 px; uncorrected, it did on 9, the largest 0.185 px. The six are
  all APS-C 16–35 mm files, whose uncorrected previews disagreed with the full-size renders by as much. Counting every
  photo with at least one corner measured at both sizes (sparser, so noisier): 10 of 41 corrected photos exceed
  0.05 px (largest 0.213, median 0.019 px), against 12 of 36 uncorrected (largest 0.185, median 0.032 px).
- **With the translation left in,** the corrected preview's corner rms is 0.054 px (0.100 uncorrected).
- **Spec criterion not met:** it asked for ≤ 0.025 px at the corners and ≤ 0.05 px per photo (see
  [Limits](#limits)).

**Distortion re-measured on the corrected renders:** the G plane is the same opcode.
- **Per block:** the G displacement against the camera JPEG moves by a median 0.002 px (no photo's median above
  0.007 px).
- **Radial bins:** max / rms 0.503 / 0.111 px on the 33 measurable corrected photos, against 0.503 / 0.109 px.
- **Block flips:** 0.1 % of the blocks (0.4 % on the controls) flip their correlation peak by more than 0.1 px
  between the two renders. One such flip moves a corner point of a 200–600 mm photo from 0.679 to 1.243 px (over
  the 0.75 px budget by this point measure; per block the G plane moves 0.002 px, so this is the correlation flip,
  not the geometry); that photo has 3 measured points from 305 blocks.

**Shipped correction.** The map runs before demosaic on eligible ILCE-7RM6 files for the three verified lenses.
Their measured full-render corner RMS went from 0.028 to 0.024 px (24–70 mm), from 0.041 to 0.025 px
(16–35 mm), and from 0.013 to 0.009 px (100 mm Macro). The 200–600 mm stays at the uncorrected CA baseline
while keeping its distortion warp. Enable Lens Corrections governs distortion only. The emitted warp carries the
G plane only. Optics ▸ Remove Chromatic Aberration (`optics.remove_ca`, an automatic estimate from the image) and the
manual CA sliders still work after demosaic and now see only the residual. The distortion numbers above were
measured with the three-plane warp, whose G plane is the one emitted today.

**Fit gate margins** (worst of R, G and B at the gated samples; gate 0.25 px): the 4 lossless files 0.160 px (margin
0.090; 0.220 px at all 65 samples, the padded radii never shown), the 46 full-frame ARW6 files 0.175 px (margin 0.075),
the 4 APS-C ARW6 files 0.035 px. The largest CA value in the 54 tables is 1724 (FE 24–70 mm), a quarter of the 6720
bound. So every photo passes (these are the three planes' numbers; the G plane alone is within them), and the
corrected and uncorrected renders differ on all 54.

## Performance

**Decode, where the CA map runs.** LightCraft's `render_bench` (`ONLY=source N=9`, `crates/engine/examples/render_bench.rs`) times
`normalized()` and the 4× binned Preview decode with and without the map. These are the minimum of 9 runs on a 4-core
laptop with nothing else running:

| File (mosaic) | Full decode, `normalized()`: without → with the map | Map cost per 24 MP | Preview decode (binned ×4, 2496 × 1664): without → with |
|---|---|---|---|
| Full-frame lossless (10240 × 7168) | 98 → 424 ms wall (363 → 1532 ms CPU) | +106 ms wall (+382 ms CPU) | 41 → 101 ms wall (152 → 377 ms CPU) |
| Full-frame Compressed HQ (10016 × 6672) | 113 → 420 ms wall (397 → 1472 ms CPU) | +110 ms wall (+386 ms CPU) | 48 → 107 ms wall (173 → 409 ms CPU) |

- **Full size:** the map adds about 0.31–0.33 s to these 67–73 MP decodes. AHD alone takes 2.2 s, and a full-size
  render end to end takes 9.5–9.8 s (`LIGHTCRAFT_PROFILE=1`, fastest of 3; the `develop` stage 2.65–3.0 s).
- **Preview:** the map adds about 60 ms wall (224–235 ms CPU).
- **Budgets:** +0.15 s per 24 MP for a full decode, and +80 ms wall (about 300 ms CPU) for a Preview decode on 4 cores.
  The Preview budget was first 30 ms, which a 4 × 4 cubic kernel can't reach.

**Rendering is unchanged.** The `sample` stage warps with the one G plane it used before, from the same opcode, so
tone, colour and exposure drags re-run nothing new. The stage cache keys the resampled source by the source, framing
and output size.

| Render (Compressed HQ, fastest of 3) | Off | One plane, manual Distortion 1 | One plane, embedded (the default) | Three planes (the earlier CA design) |
|---|---|---|---|---|
| Full size 9984 × 6656 | 403 ms | 1187 ms | 1109 ms | 4014 ms |
| 1920 px long edge (2496 × 1664 source) | 22.1 ms | 41.6 ms | 56.7 ms | 112.0 ms |

Both three-plane numbers and the full-size Off and manual numbers come from the earlier verification, on a machine
shared with other builds; the rest are from this round. The three planes, had they been turned on, would have cost
about 55 ms more per 1920 px render and about 2.9 s more per full-size render (numbers from different runs).

## Limits

- **ILCE-7RM6, ILCE-6700 and ZV-E10 only.** Other Sony bodies record the same tags; their knot spacing (defined here by the
  full-frame crop, or an APS-C body's own crop), unit and zoom rule are unverified, so they get no correction from this conversion (the ILCE-7RM4A's distortion has
  its own: [Two conventions](#two-conventions)).
- **ILCE-6700 at strong barrel distortion:** the body's rule leaves 1.36–1.38 px at the Sigma 10–18 mm's 10 mm end;
  that lens has a measured exception (above). The two public raw.pixls.us ILCE-6700 files (one scene, E 16–55 mm
  F2.8 G at 16 mm, not used for fitting) both carry the same table (−1829 at the last knot). On the compressed file
  the correction leaves p95 3.1 px (uncorrected 5.1 px, exact table 3.4 px); with the same spacing this lens fits at
  unit 16000 (0.43 px), another lens-dependent unit (below). On the lossless file the table, on its padded
  6656 × 4608 frame, misses the 0.5 px fit gate, so that photo stays uncorrected (5.3 px). Other wide lenses with
  similarly strong tables are unmeasured. Its vignetting and lateral CA tables are read, not applied: measured in
  #2801 (the camera JPEG doesn't compensate vignetting, as on the ILCE-7RM6).
- **ZV-E10: one lens, four photos.** Only the E PZ 16-50 mm (16–32 mm) is measured. Its unit, 16384, differs from
  the ILCE-6700's 15000; the ILCE-6700's own E 16–55 mm at 16 mm also fits a higher unit (16000, above), so the unit
  may belong to the lens rather than the body. Other lenses on the ZV-E10 are unmeasured and take 16384. The
  ZV-E10M2 (another sensor) is not covered.
- **APS-C distortion is unverified in camera on the ILCE-7RM6:** no APS-C photo with distortion correction on was available; the APS-C
  knot spacing is verified on CA only, and the APS-C zoom follows the full-frame rule.
- **Aspect-ratio crops** (in-camera 4:3, 16:9, 1:1) were not measured, so they get no correction: neither the
  distortion warp nor the CA map. Neither do linear (downsized lossless M and S) files, which have no mosaic.
- **One distortion unit for every lens.** Fitted per lens the unit is 14826 (24–70 mm, 23 photos, very consistent),
  15659 (200–600 mm, 3 photos) and 16017 (100 mm, tiny tables); with the single 15000 the 200–600 mm keeps a systematic
  residual of up to 0.59 px (within budget, above the method's 0.03 px noise floor). Something lens-dependent is not
  captured; per-lens units would need per-lens verification.
- **Lateral CA: the unit is still provisional** (above). With it, the corrected render beats the uncorrected one on
  three lenses but not on the 200–600 mm (2 photos; [Verification](#verification)). A per-lens unit would need more
  photos per lens. Since the map is applied whatever Lens Corrections says, it is limited to those three lenses
  (`CA_IMPROVES` in `arw_lens_rm6.rs`: FE 24–70 mm F2.8 GM II, FE 16–35 mm F2.8 GM II, FE 100 mm F2.8 Macro GM OSS, by
  Exif `LensModel`). The 200–600 mm, any lens not measured, and a file without a lens name get no map; their distortion
  warp is unchanged.
- **Lateral CA beside blown highlights stays uncorrected:** a site whose support holds a clipped sample keeps its
  value. Sites within a few pixels of the mosaic's edge, whose source lies beyond it, repeat the edge sample; on
  ILCE-7RM6 files they lie outside the default crop (inset 8–16 px).
- **Previews:** binning keeps the red and blue planes offset by a fraction of a binned pixel, as before (median
  0.03 / 0.04 px at 1440 after correction). On the APS-C 16–35 mm files, a preview and a full-size render still
  differ by up to 0.1 px at a corner, as they did uncorrected (the cause is not known).
- **Enable Lens Corrections governs distortion only.** Lateral CA correction remains on for this body's Bayer files
  with 3:2 image and reference crops and a verified lens.
- **DNG export, Photo → Convert to DNG and import's Copy as DNG carry distortion only.** The warp goes into
  `OpcodeList3`, as for RW2, and a DNG reader applies it like any embedded lens correction. No DNG opcode resamples
  the CFA before demosaic, so the CA correction is lost. Three-plane CFA DNGs from other converters stay corrected
  after demosaic. Converting their warp into a map before demosaic is a follow-up.
- **Vignetting not applied** (above).
- **Existing imports** need Photo → Reload from Disk to pick up the distortion data. They get the CA correction
  without it, because the map comes from the file at decode (the render cache refreshes). Smart previews built
  before this change keep the uncorrected CA while the original is offline. AI Denoise results of ARW photos are
  made again once.
- The camera-JPEG colour fit still matches its pairs without the corrections (distortion and CA;
  docs/camera-preview-colour.md).

## Sources

- [ExifTool EXIF tag names](https://exiftool.org/TagNames/EXIF.html): the names and types of tags `0x7031`–`0x7037`
  (ExifTool 13.59 was used to read the tag values). Their meaning, units and conventions were measured as above.
- [dnglab RawDB, Sony ILCE-7RM6](https://rawdb.dnglab.org/sets/Sony/ILCE-7RM6): the 12 CC0 samples (data only;
  dnglab's code was not consulted).
