# Sony ARW lens corrections

> Moved from LightCraft (`docs/sony-ilce7rm6-lens-corrections.md`) with the decoder (`src/arw_lens.rs`, `src/arw_lens_rm6.rs`). It records how the tables were derived and verified; mentions of LightCraft's settings and UI describe that app.

Sony cameras record three lens-correction tables in the raw IFD of every ARW: distortion, vignetting and lateral
chromatic aberration (CA). LightCraft reads them and turns distortion into a DNG `WarpRectilinear` opcode, so it
reaches the same **Optics ▸ Enable Lens Corrections** (`optics.lens_profile`) as a DNG's embedded warp and Panasonic's
RW2 distortion data. Lateral CA is parsed and converts to the warp's red and blue planes, but it is **off by default**:
applied after demosaic it made the render's CA worse (Verification). Vignetting is read but not applied (below). The
conversion is verified on the **ILCE-7RM6** and, for distortion, the **ILCE-6700** ([ILCE-6700](#ilce-6700)); the
ILCE-7RM4A has its own conversion, with other conventions
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
  0.052 px. Swapping the halves or an absolute form (`v · norm / U`) is clearly worse. **Not applied by default**
  (Verification).
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
   `G(r) = S(r / z)`. Red and blue, when the CA planes are on: `G(r) · (1 + c(G(r)) / 3.36·10⁶)`.
2. Each map is sampled at `u = i / 64`, i = 0…64 (65 samples of `G(u · m) / m`) and fitted by least squares with
   DNG's 4-term odd polynomial `u · (k0 + k1 u² + k2 u⁴ + k3 u⁶)` (`vendor::fit_radial_warp`, shared with RW2).
3. **Fit gate:** a plane is refused when its fit is worse than 0.25 px at a 1440 px long edge at a sample the crop
   shows: the samples up to the first one at or beyond the crop corner. The lossless files' padded active area
   (10240 × 7168 for a 9984 × 6656 crop) reaches radii that are never shown (`u` up to 0.925 is shown there).
   Over all 54 photos the fit's worst error at all 65 samples was 0.220 px (a lossless file; ARW6 files at most
   0.175 px). Between samples the error is somewhat larger: 0.071 against 0.049 px for the full-frame 16–35 mm
   example in the tests.
4. **By default one plane, G**, applied to all three channels: `lens_opcodes` calls `warp_opcode(…, lateral_ca:
   false)`. With `lateral_ca: true` (a code change) the planes are `[R, G, B]`, unless the CA table is absent or
   malformed, a CA value is implausible (|c| above 2·10⁻³ of the radius, i.e. 6720; the largest value in the 54
   photos' tables is 1724) or a colour plane fails the gate: then G alone again. The G plane is the same either way.
   No opcode when the distortion table is absent or all zero (CA alone is never applied: it is relative to green),
   the zoom has no solution in [0.5, 2] or the G fit fails. A refused table leaves the photo uncorrected rather than
   wrongly warped.

The opcode then goes through the same pipeline stage as a DNG's (`crates/pipeline/src/optics.rs`): it rotates with
the photo (portrait files are covered) and is carried by DNG export, as RW2's is.

## Which bodies get it

Only bodies on one of two allowlists in `arw_lens_rm6.rs` get an opcode: `FULL_FRAME_CROP` (today `ILCE-7RM6`,
9984 × 6656) and `APSC_BODIES` (today `ILCE-6700`, below). The unit and the knot spacing are verified per body; a body
with another sensor size or another unit would be warped wrongly. Other bodies get no correction from this conversion;
the ILCE-7RM4A gets its own, with other conventions (below). To add a body: collect photos of it shot with distortion
correction on (several lenses and focal lengths), repeat the camera-JPEG registration below against the uncorrected
render, confirm the unit, the knot spacing and the zoom rule (or measure the body's own). Then add a full-frame body's
default crop to `FULL_FRAME_CROP`, or an APS-C body whose knots span its own crop (as measured for the ILCE-6700) to
`APSC_BODIES`, with a test; an APS-C body also needs its fit gate checked against the camera JPEG (see *ILCE-6700*).

For bodies in `APSC_BODIES` (today `ILCE-6700`), the knots span the file's own default crop: spacing = its half
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
| Aspect crops | applied; the zoom for the recorded crop is unverified | refused (native 3:2 only) |
| Tables | distortion (16 knots; 11 in APS-C); lateral CA parsed, off by default; vignetting read, not applied | distortion, 16 entries only |

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

**Distortion, photos corrected in camera** (our render with the correction on vs the camera JPEG; budget 0.75 px).
"Corners + midpoints" is the 2-D displacement at the four corners and four edge midpoints, global translation
included; "bins" the radial residual over 32 radial bins. A photo counts as measurable as in the method above.

| Lens | Photos (measurable) | Focal lengths | Corners + midpoints max / rms | Bins max / rms | Uncorrected, corners (per-photo max) |
|---|---|---|---|---|---|
| FE 24–70 mm F2.8 GM II | 26 (23) | 24–70 mm | 0.553 / 0.125 | 0.503 / 0.066 | 1.1–17.3 |
| FE 100 mm F2.8 Macro GM OSS | 9 (7) | 100 mm | 0.409 / 0.111 | 0.317 / 0.097 | 0.6–4.8 |
| FE 200–600 mm F5.6–6.3 G OSS | 6 (3) | 594–600 mm | 0.679 / 0.321 | 0.498 / 0.274 | 7.5–10.8 |
| **All** | **41 (33)** | | **0.679** / 0.150 (206 points) | 0.503 / 0.109 | 0.6–17.3 |

No photo is over budget; the global translation is at most 0.056 px. The 8 not measurable are black, near-black or
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

**Lateral CA in the render** (radial R-vs-G and B-vs-G shift, median of the 12 blocks nearest each corner; 43 photos
with enough detail at all four corners; budget 0.3 px):

| Lens | Photos | Off: max R / B | Off: rms | On: max R / B | On: rms | Camera JPEG: max / rms |
|---|---|---|---|---|---|---|
| FE 24–70 mm F2.8 GM II | 23 | 0.089 / 0.071 | 0.028 | 0.219 / 0.148 | 0.069 | 0.150 / 0.036 |
| FE 16–35 mm F2.8 GM II | 12 | 0.125 / 0.054 | 0.041 | 0.030 / 0.051 | 0.020 | 0.058 / 0.016 |
| FE 100 mm F2.8 Macro GM OSS | 6 | 0.020 / 0.050 | 0.013 | 0.061 / 0.157 | 0.044 | 0.021 / 0.004 |
| FE 200–600 mm F5.6–6.3 G OSS | 2 | 0.033 / 0.101 | 0.032 | 0.146 / 0.075 | 0.070 | 0.113 / 0.039 |
| **All** | **43** | 0.125 / 0.101 | 0.032 | **0.219** / 0.157 | 0.052 | 0.150 / 0.028 |

Within budget everywhere, but **the correction makes the render's own CA larger on average** (rms 0.032 → 0.052 px;
smaller only for the 16–35 mm). The reason is where it runs: over the radial bins beyond 0.3 of the half diagonal
(418 bins), our demosaiced render shows only 0.29 × the lens CA measured on the raw CFA (correlation 0.85), while the
warp after demosaic moves R and B by 0.84 × the table (correlation 0.96). On the raw data the table halves the CA (rms
0.062 → 0.035 px); applied after a demosaic that already hid most of it, it overshoots (rms 0.021 → 0.035 px over the
same bins). All of this is far below one pixel at 1440 px.

**Ruling: the lateral CA planes are off by default.** `lens_opcodes` emits the G plane alone; the conversion stays in
the code and its unit tests (`warp_opcode(…, lateral_ca: true)`), so it can be turned on once CA is corrected where
the table applies. **Follow-up:** warp the raw CFA's red and blue sites before demosaic (the table halves the raw CA,
rms 0.062 → 0.035 px), then turn the planes on and repeat this check. Optics ▸ Remove Chromatic Aberration
(`optics.remove_ca`, an automatic estimate from the image) remains available for visible fringes. The distortion
numbers above were measured with the three-plane warp, whose G plane is the one the default emits.

**Fit gate margins** (worst of R, G and B at the gated samples; gate 0.25 px): the 4 lossless files 0.160 px (margin
0.090; 0.220 px at all 65 samples, the padded radii never shown), the 46 full-frame ARW6 files 0.175 px (margin 0.075),
the 4 APS-C ARW6 files 0.035 px. The largest CA value in the 54 tables is 1724 (FE 24–70 mm), a quarter of the 6720
bound. So every photo passes: with the CA planes on (the verification renders) each got the three-plane warp, and the
corrected and uncorrected renders differ on all 54; by default each gets the one-plane distortion warp.

## Performance

`LIGHTCRAFT_PROFILE=1` renders of the full-frame lossless file and the full-frame Compressed HQ file (corpus
`arw-sony-ilce7rm6-ff-land-lossless.arw`, `arw6-sony-ilce7rm6-ff-land-hq.arw`), on a 4-core laptop shared with other
builds (so absolute times are high); the fastest of 3–5 runs. The warp runs in the `sample` stage:

| Render | Off | One plane (the default; measured with manual Distortion 1) | Three planes (CA planes on) |
|---|---|---|---|
| Full size 9984 × 6656, lossless | 267 ms | — | 3098 ms |
| Full size 9984 × 6656, Compressed HQ | 403 ms | 1187 ms | 4014 ms |
| 1920 px long edge (2496 × 1664 source), Compressed HQ | 23.7 ms | 45.4 ms | 112.0 ms |

The default one-plane warp takes the sample stage from 24 to 45 ms at 1920 px (inferred from the manual Distortion
run: the same single-plane path, one plane for all three channels). The three colour planes cost about 2.5–3× that,
as for an RW2 or DNG with three planes (the same path): the price of turning the CA planes on. The stage cache keys
the resampled source by the source, framing and output size, so tone, colour and exposure drags don't re-run it.

## Limits

- **ILCE-7RM6 and ILCE-6700 only.** Other Sony bodies record the same tags; their knot spacing (defined here by the
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
- **APS-C distortion is unverified in camera on the ILCE-7RM6:** no APS-C photo with distortion correction on was available; the APS-C
  knot spacing is verified on CA only, and the APS-C zoom follows the full-frame rule.
- **Aspect-ratio crops** (in-camera 4:3, 16:9, 1:1) were not measured; the correction uses the crop the file records.
- **One distortion unit for every lens.** Fitted per lens the unit is 14826 (24–70 mm, 23 photos, very consistent),
  15659 (200–600 mm, 3 photos) and 16017 (100 mm, tiny tables); with the single 15000 the 200–600 mm keeps a systematic
  residual of up to 0.59 px (within budget, above the method's 0.03 px noise floor). Something lens-dependent is not
  captured; per-lens units would need per-lens verification.
- **Lateral CA off by default.** Its unit is provisional (above), and a warp after demosaic, as for RW2 and DNG,
  overshoots: our demosaic already hides about 70 % of the lens CA, and with the planes on the rms corner CA rose
  from 0.032 to 0.052 px (Verification). Correcting CA before demosaic is the follow-up.
- **Vignetting not applied** (above).
- **DNG export** carries the warp in `OpcodeList3`, as for RW2: a DNG reader applies it like any embedded lens
  correction.
- **Existing imports** need Photo → Reload from Disk to pick up the lens data.
- The camera-JPEG colour fit still matches its pairs without the correction (docs/camera-preview-colour.md).

## Sources

- [ExifTool EXIF tag names](https://exiftool.org/TagNames/EXIF.html): the names and types of tags `0x7031`–`0x7037`
  (ExifTool 13.59 was used to read the tag values). Their meaning, units and conventions were measured as above.
- [dnglab RawDB, Sony ILCE-7RM6](https://rawdb.dnglab.org/sets/Sony/ILCE-7RM6): the 12 CC0 samples (data only;
  dnglab's code was not consulted).
