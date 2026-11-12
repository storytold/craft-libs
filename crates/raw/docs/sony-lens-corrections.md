# Sony embedded lens corrections

> Moved from LightCraft (`docs/sony-lens-corrections.md`) with the decoder (`src/arw_lens.rs`, `src/arw_lens_rm6.rs`). It records how the tables were derived and verified; mentions of LightCraft's settings and UI describe that app.

The **ILCE-7RM6** (A7R VI) has its own conversion, with other conventions measured against its camera JPEGs:
[Sony ARW lens corrections](sony-ilce7rm6-lens-corrections.md). On ILCE-7RM6 photos the formula below misses the 24–70 GM II by
up to 1.43 px at 1440 px while fitting the 100 mm and 200–600 better, and no single unit fits every lens under either
convention; that page's *Two conventions* section has the comparison and what would settle it.

Sony ARWs record three lens corrections per shot in the raw image IFD: shading (vignetting), lateral chromatic
aberration (CA) and distortion, each as a camera setting and a radial table, with copies in the encrypted
`SR2SubIFD`. LightCraft **decodes all three** wherever a file carries them and applies distortion here on three validated
bodies: **ILCE-7RM4A (A7R IVA)**, **ILCE-7M3 (A7 III)** and **ILCE-7CR (A7CR)**. The A7R IVA keeps its existing Bayer 3:2 conversion,
validated with the FE 24-105mm F4 G OSS and FE 200-600mm F5.6-6.3 G OSS; the A7 III uses the same conversion, validated
with the FE 16-35mm F4 ZA OSS, FE 28-70mm F3.5-5.6 OSS and FE 50mm F1.8 ([ILCE-7M3 validation](#ilce-7m3-validation)). The A7CR uses a separate conversion,
limited to full-frame Bayer 3:2 files with distortion Auto or forced Auto. Apart from the separate ILCE-7RM6 converter, other models and shading and CA on every model are
decoded only: no warp, vignetting or CA correction is added for them. The decoded record stays with the image
(`RawInfo::lens_metadata` / `RawImage::lens_metadata`, types in `src/lens_metadata.rs`, decoder in
`src/arw_lens.rs`) for diagnostics and for the validation work described below.

The applied distortion is converted into `OpcodeList3` / `WarpRectilinear`, so the existing profile correction
control, CPU/GPU optics path, EXIF orientation handling, and DNG export use the same correction. Linear YCbCr ARWs
and non-3:2 camera crops are excluded pending validation. An aspect crop may retain the full-frame radial
normalization; the decoder does not guess that relationship. Tables of other lengths and rejected tables remain
uncorrected, and a table found only in the encrypted `SR2SubIFD` is never applied. The camera's distortion Off
setting does not erase its table. A7CR Off, missing, unavailable and unknown settings produce no warp. The A7R IVA's
existing behavior is unchanged. Newly imported photos use LightCraft's existing embedded-profile default
(enabled); the profile correction control can disable or adjust it.

## What is decoded

| Component | Raw-IFD setting / table | `SR2SubIFD` setting / table | Accepted layout |
|---|---|---|---|
| Shading | `0x7031` / `0x7032` | `0x797c` / `0x797d` | SHORT\[1\]; SSHORT\[17\]: 16, then 16 samples |
| Lateral CA | `0x7034` / `0x7035` | `0x797f` / `0x7980` | SHORT\[1\]; SSHORT\[33\]: 32, then two 16-sample curves |
| Distortion | `0x7036` / `0x7037` | `0x7981` / `0x7982` | SHORT\[1\]; SSHORT\[17\]: 16, then 16 samples |

- **Bounded, exact reading.** A raw-IFD field is the one the TIFF reader parsed, the copy the image itself uses:
  its first occurrence, within the reader's limits (8,192 entries per directory, a total value budget). The directory
  is also scanned, in either byte order and in BigTIFF, for diagnostics only: a tag that occurs more than once is
  flagged (`raw_ifd_repeated`, `sr2_repeated`), and an entry the reader left out is reported with its reason (values
  outside the file, an unknown type, or past the reader's limits) rather than treated as absent. `SR2SubIFD` fields
  are read straight from their first entry. A setting may also be a nonnegative SSHORT (the documented editor type
  differs from the type the cameras store). Anything else is refused with its reason (`SonyFieldError`: field type,
  count, leading sample count, values outside the file or block, past the reader's limits, a negative setting).
  Nothing is allocated from a count in the file.
- **Raw IFD first, `SR2SubIFD` as a checked fallback.** The raw IFD's copy wins. The `SR2SubIFD`'s copy is used only
  when the raw IFD lacks the tag; a malformed raw-IFD copy is reported, never silently replaced. When both copies are
  well-formed their agreement is recorded (`copies_agree`), and on disagreement the raw IFD's copy is used. The block
  is decrypted (by the existing `vendor/sr2.rs`) for every ARW that has one, so the check always runs; a block that
  can't be found or decrypted gives no copies. Locating it selects the `SR2Private` directory's offset, length and
  key exactly as the TIFF reader would (the reader's own directory layout, 8,192-entry limit, readable-entry rules
  and value budget, then the first readable occurrence of each), but decodes only each field's first value, so the
  directory's other entries cost no memory however large they claim to be; the block itself is limited to 1 MiB. The
  plain tag numbers are not looked up inside the `SR2SubIFD`.
- **Settings keep their number and a documented meaning:** shading 256 Off, 257 and 272 Auto, 511 unavailable; CA
  0 Off, 1 Auto, 255 unavailable; distortion 0 Off, 1 Auto, 17 Auto fixed by the lens, 255 unavailable. Any other value
  is `Unknown(value)`, never Off (the A7CR's Low shading setting has no documented number). Values are not read as
  bit masks. Off does not mean absent: the table is still recorded.
- **Only the 16-sample (CA: 2 × 16) layout is interpreted.** Many bodies store 11 (CA: 22) samples padded into the
  same 17 (33) values: older compacts and bodies such as the DSC-RX100 III and ILCE-3500, and the ILCE-7M4's APS-C
  captures (its full-frame captures carry 16). Those tables are refused as `CountPrefix(11)` and stay
  uninterpreted. An all-zero table is kept: it is a valid no-op, not a sign of an unsupported lens.
- **Framing evidence travels with the tables:** the stored frame (padding included), Sony's crop tags and the DNG
  default crop as recorded, the image crop, the reference rectangle the distortion conversion is centred on and
  normalized to (the default crop when present, else the image crop), compression, whether the data is linear
  YCbCr (lossless M/S), and whether both crops are native 3:2.
- **Per-component status.** `Applied` (converted into an opcode), `NoTable`, `NotValidatedComponent` (shading and CA
  on every body), `NotValidatedModel`, `NotValidatedMode` (linear YCbCr, non-3:2 framing, or a distortion table found
  only in the `SR2SubIFD`), `Neutral` (all-zero table in the validated scope) and `Rejected` (the validated
  conversion refused the table). A readable table never implies a usable correction; only `Applied` components
  become opcodes.

For the A7CR, `CameraCorrectionDisabled` records an Off, missing, unavailable or unknown setting. APS-C,
aspect crops and lossless M/S remain `NotValidatedMode`. The raw IFD's correction table is required.

The A7R IVA's applied path is unchanged: its conversion still receives the raw IFD's `0x7037` table exactly as the
TIFF reader parsed it, with the same model, framing and fit checks, and produces the same warp. A repeated tag keeps
the reader's choice and is only flagged, and a table the reader never reads (past its entry limit) is reported but
never applied, so already-catalogued photos, their previews and their stored lens data stay consistent with a fresh
decode.

## What each component still needs

The following summarizes an investigation on the ILCE-7CR (A7CR): an owner's collection of 131 raws with four
lenses (Voigtländer APO-LANTHAR 28mm F2 and 50mm F2, both manual focus; Sony FE 24–50mm F2.8 G; Tamron 35–150mm
F2–2.8), plus the three public raw.pixls.us A7CR samples. No private photographs, names or derived images are
part of this repository.

- **Inventory.** All 131 raws carry all three tables in both the raw IFD and the `SR2SubIFD`, and the copies agree
  in every file; all four lenses provide tables, the manual-focus ones included. Shading is 272 (Auto) and CA 1
  (Auto) everywhere; distortion is 17 (Auto, fixed by the lens) on the nine FE 24–50 files and 0 (Off) on the rest.
  Every image crop is 9504 × 6336 at (32, 20), in uncompressed, compressed and lossless-L (padded to 9728 × 6656)
  storage. The decoder reproduces this inventory exactly (see *Local inventory check*).
- **Distortion.** The A7R IVA interpretation misses the A7CR camera JPEG at the wide end. The A7CR now uses the
  separately measured corrected-radius convention below. It uses the raw IFD table, not the MakerNote array.
- **Shading.** The layout decodes, but no public specification gives the sample-to-gain function, its radius
  convention on the A7CR, or whether the raw samples already contain part of the correction. Brightness ratios on six
  natural scenes show no consistent missing peripheral brightening. Needed: evenly lit flat fields with shading
  Off / Auto / Low at several apertures and focal lengths, raw CFA ratios between them, and Imaging Edge On/Off
  exports, to fix the gain function and processing stage and rule out double correction.
- **Lateral CA.** A public analysis reads the table as a red, then a blue curve relative to green, at a 2⁻²¹
  relative radial scale. On three distortion-Off files that candidate reduced red and blue feature residuals
  modestly (for example 0.35 → 0.29 px median for red with the 28 mm), which supports investigating it but certifies
  neither scale, interpolation nor processing stage. Two product changes come before it: embedded CA must be
  independent of the distortion amount (today `pipeline::optics` scales the whole embedded per-plane warp with the
  profile distortion control), and the red and blue planes must be resampled in camera RGB, before the camera colour
  model mixes them (`engine::files::load_bytes_now` applies the colour model before the optics stage). Then
  validation on full-resolution achromatic edges.
- **Other modes.** APS-C, aspect crops and lossless M/S (linear YCbCr) captures were not measured and stay
  excluded.

## A7CR camera-JPEG validation

The A7CR conversion substantially reduces the corrected camera JPEG's geometry mismatch. Its empirical table unit is
`15559.300457825264`, fixed by the earlier four-file FE 24-50mm fit before checking other files and lenses.
This is a measured approximation for the tested scope, not a Sony specification or a universal Sony constant.

For output radius `rho` in native crop pixels:

```
spacing = hypot(9504, 6336) / 30
S(r) = r * (1 + interpolate(table, r / spacing) / 15559.300457825264)
source_radius(rho) = S(rho / zoom)
```

Interpolation is linear. The final segment extends beyond the last knot. Positive samples increase the source
radius before the fill zoom; negative samples reduce it.

The fill zoom makes the maximum of
`S(rho / zoom) / rho` on the rectangular crop border equal to one. The optical center is the default crop's
pixel-center midpoint. The measured crop is 9504 by 6336 at (32, 20).

Storage padding affects DNG normalization
only, so 9600 by 6376 and 9728 by 6656 storage produce the same physical warp.

A four-term odd polynomial represents the map as `WarpRectilinear`. A dense 32769-point check rejects a polynomial
that folds or differs from the table by more than 0.25 px at a 1440 px long edge. This approximation check is
separate from agreement with the camera JPEG. The converter applies no shading or lateral CA.

Validation uses all 131 owner files numerically, nine public raw.pixls.us codings, one independent public FE 50mm
file and the public issue #1300 FE 16-25mm file. Private pixels stay in memory. No private images, names or source
identities are saved in the new measurement artifacts.

The nine private correction-on files cover the FE 24-50mm
at 24, 49 and 50 mm. The other 122 private files have correction Off and cover the Tamron 35-150mm and the
Voigtlander APO-LANTHAR 28mm and 50mm. Off references validate preservation, not correction-on optics for those lenses.

Green-channel SIFT (Scale-Invariant Feature Transform) matches at 2376 px are filtered by an independent radial fit. Residuals remove median translation
only. They fit no scale, center or lens coefficient to the candidate.

High-pass correlation uses held-out pixels
at 1200 px. File checks hold out the second file and then every fifth file in each lens/focal/setting group, and
both public correction-on files. The private checks were also examined in the earlier study, so they are file
checks rather than a fresh independent capture campaign.

The public second-lens checks do not tune the unit.

The A7R VI study uses a 0.75 px camera validation budget at 1440 px. The reporter has support to 0.933 half
diagonals. Its direct table's largest radial-bin residual is 0.590 px at 1200 px (0.708 px at 1440 px).

The implemented polynomial reaches 0.638 px in a radial bin and 0.692 px in a supported edge region at 1200 px
in source coordinates. Actual corrected-raster matches give 0.173/0.605 px median/p95, a 0.763 px radial-bin
maximum and a 0.969 px edge-region maximum at 1200 px. That edge region is 1.162 px at 1440 px, so this does not
pass a strict 0.75 px budget everywhere measured.

The unit test against the independently fitted smooth feature curve uses the combined 1.0 px table and
polynomial approximation budget; its observed maximum is 0.895 px at 1440 px. These are different metrics.

The held-out private 24 mm file has weak outer-radius support. No unsupported ideal corner is counted as measured.

APS-C, aspect crops and lossless M/S are excluded. Portrait captures use the same sensor-coordinate map and the
existing EXIF orientation conversion. Other lenses and the 16-25mm's tele end need more correction-on references.

No Imaging Edge On/Off pairs were measured for this body. Camera JPEGs are the reference for this change.

This branch supplies the warp for camera-look pairing in PR #1586. It does not duplicate that pairing code.

`LOOK_VERSION` 12 refreshes smart previews that stored no A7CR lens data; `RENDER_CACHE_VERSION` 69 redraws cached
renders.

Existing edits are preserved. Reload from Disk reads the new lens data for old imports. Enable the
profile correction control when an existing edit has it disabled.

## Local inventory check

`crates/raw/tests/it/sony_lens_inventory.rs` is opt-in and does nothing in CI. With `LIGHTCRAFT_SONY_LENS_DIR` naming a
directory of ARWs it decodes each from its headers and prints its record (settings, sources, copies' agreement,
sample ranges, statuses); `LIGHTCRAFT_SONY_LENS_FULL=1` also compares header and full decode. With
`LIGHTCRAFT_SONY_LENS_INVENTORY` naming a CSV inventory it checks every file's table counts, sizes, sample counts,
minima and maxima, raw-IFD versus `SR2SubIFD` agreement, settings (numbers and labels), stored size, compression and
crop tags. A file with a row must decode, and every file must have a row, whether it decodes or not. Any decode
error fails the check except an unsupported variant, which is counted and must equal `LIGHTCRAFT_SONY_LENS_UNSUPPORTED`
(default 0). On the 131 A7CR raws above, all 131 inventory rows reproduce and every file decodes. On 133 public Sony
ARWs (127 raw.pixls.us samples from 100 bodies, four corpus copies and two files attached to a public issue), 110 carry
a record, 14 decode without one, and 9 are unsupported variants (eight packed ARWs of older bodies and one ARW
without a CFA image); header and full decode agree on the 124 that decode, and no record has disagreeing copies.

## Clean-room evidence

The [ExifTool EXIF tag-name documentation](https://exiftool.org/TagNames/EXIF.html) names the raw-IFD tags
`0x7031` to `0x7037` (VignettingCorrection / VignettingCorrParams, ChromaticAberrationCorrection /
ChromaticAberrationCorrParams, DistortionCorrection / DistortionCorrParams) with their setting values, and gives the
tables as 17 or 33 signed 16-bit integers whose first value is the count. The
[Sony SR2SubIFD documentation](https://exiftool.org/TagNames/Sony.html#SR2SubIFD) names the encrypted copies
`0x797d`, `0x7980` and `0x7982`; the setting tags next to them (`0x797c`, `0x797f`, `0x7981`) are unnamed there and
were identified by inspecting files, where they equal the raw-IFD settings. The corpus test's expected tables are the
values ExifTool, run as a black-box tool, prints for the public samples. No external photo decoder implementation,
lens database or Adobe lens profile was consulted. Sony Imaging Edge Edit 4.1 was used only as a black-box reference
on local scratch copies.

The observed inverse mapping, with radius `r` normalized to the default crop's half diagonal, is:

```
f(r) = 1 + interpolate(table, r) / 16384
source = center + (output - center) * zoom * f(r)
zoom = 1 / max(f(r)) over radii on the rectangular output boundary
```

The border radii range from the shorter half edge / half diagonal to 1. Table knots span 0 through 1.
Linear interpolation approximates the observed map closely; we do not claim to reproduce Sony's
internal interpolator. A fixed-size weighted least-squares fit expresses source radius as
`r * (k0 + k1*r² + k2*r⁴ + k3*r⁶)`. The fit is rejected if radial error exceeds 0.0005 of the
half diagonal (about 0.43 px at a 1440 px 3:2 preview), or the mapping folds. Table values are bounded
and the input must have precisely the supported signed-short layout. Crop offset and normalization
are converted into DNG's active-area coordinate system before creating the opcode. The warp's centre
and normalization come from the standard `DefaultCropOrigin` / `DefaultCropSize` when the file has them,
not from Sony's crop tags: on the tested A7R IVA files the former start at raw x=32 and the latter at
x=0, and the standard origin is the one that aligns with Sony's exports. The image itself keeps the
decoder's crop precedence for every Sony body (Sony's crop tags, then the default crop, then the
recorded image size), so the framing is unchanged; the opcode's centre is in active-area coordinates,
so the warp stays on the optical centre either way. On the A7R IVA that framing still sits 32 raw
pixels left of Sony's exports (eight pixels in a 2376-pixel-wide export).

The main raw-IFD table is important: the similarly named MakerNote table has different values and
must not be substituted into this formula. Main raw-IFD and SR2SubIFD values agreed in the test files, and the
`SR2SubIFD` copy is never converted.

## Validation

At 2376×1584, independent SIFT correspondences between Sony distortion On and Off TIFF exports gave:

| Lens / focal length | Median residual | 95th percentile | Maximum matched radius |
|---|---:|---:|---:|
| FE 200–600 / 200 mm | 0.043 px | 0.191 px | 0.97 half diagonals |
| FE 200–600 / 600 mm | 0.055 px | 0.231 px | 0.95 half diagonals |

These use the fixed 16384 scale and automatic border framing, not fitted per-image scale or zoom.
A median translation below 0.004 px was removed from the residuals. Correspondences do not cover
all pixels; these are geometric feature residuals, not photometric equality or a worst-case guarantee.
Across 20 sampled RAWs (16 at 24–105 mm, four at 200–600 mm), the polynomial's maximum deviation
from the interpolated file table was below 0.12 px at 1440 px. That measures approximation error,
not agreement with an independent renderer. Camera JPEG and Apple Core Image comparisons support
the wide-lens interpretation but are weaker evidence than same-renderer On/Off pairs.

Historical validation before restoring main's image crop precedence: LightCraft exports with the
standard default crop framing were compared directly with Sony's corrected
TIFFs. At 2376 px wide, median / 95th-percentile feature residuals were 0.35 / 1.04 px at 24 mm,
0.15 / 0.40 px at 200 mm, and 0.13 / 0.56 px at 600 mm (median translation below 0.024 px).
The uncorrected medians at 200 and 600 mm were 8.63 and 10.82 px respectively. Different colour,
sharpening, and demosaicing affect feature localization; these are geometry checks, not colour parity.
An isolated desktop run on Apple M1 Metal detected the profile, rendered without GPU fallback or
warnings, and exposed the existing Optics controls. Existing library edits are not reset: once the
source has loaded, enable lens corrections if they were previously disabled.

After restoring main's crop precedence, fresh 2376×1584 exports on 2026-10-10
(main `b70ae174` plus the aspect-framing guard) measured:

| Focal length | Corrected median / p95 residual | Uncorrected median | Removed translation x / y | Matches / maximum radius |
|---|---|---|---|---|
| 24 mm | 0.36 / 1.12 px | 5.62 px | 7.98 / −0.01 px | 1,442 / 0.95 |
| 200 mm | 0.13 / 0.35 px | 8.65 px | 8.01 / 0.02 px | 11,606 / 0.98 |
| 600 mm | 0.10 / 0.37 px | 10.33 px | 8.02 / 0.00 px | 12,268 / 0.97 |

Residuals subtract only the median feature translation; no scale or lens coefficients are fitted to
these reference pairs. The roughly eight-pixel horizontal translation is the retained crop-origin
difference described above, not a claim of exact Sony framing. Maximum radius is in half diagonals.
The distortion geometry remains aligned while every body's image framing follows main's crop order.

Tests cover signedness/length rejection, zero and malformed tables, independent 600 mm reference
geometry, barrel framing, offset crop normalization, both TIFF byte orders, header/full consistency,
and preservation through DNG export. The decoder's synthetic tests (`src/arw_lens.rs`) add both
byte orders and BigTIFF, exact SHORT / SSHORT types, negative samples, missing, short, long, huge and zero counts,
wrong leading counts, values outside the file, a table past the TIFF reader's entry limit (not applied), repeated
tags (the reader's copy kept, as before), unknown and negative settings, raw-IFD versus
`SR2SubIFD` precedence and agreement, the correct `SR2SubIFD` tag numbers, a wrong decryption key, hostile entries
inside the decrypted block, linear YCbCr, framing evidence (Sony's crop at x = 0 against the default crop, agreeing
crops, padded storage, aspect crops, crop tags outside the frame), all eight EXIF orientations leaving the record
unchanged, and neutral, out-of-range and folding tables. On a small classic-TIFF fixture in both byte orders, every
truncation and four mutations of each byte outside the pixel data (set to 0x00 and 0xff, top and low bit flipped)
decode or fail without a panic. `tests/sr2_heap.rs` measures the peak heap of decoding a 16 MiB file whose
`SR2Private` directory holds eight fields aliasing the whole file: 37 kB, against 134 MB when those fields were
decoded. No private photographs or reference TIFFs are committed.

## ILCE-7M3 validation

Checked on 2026-10-12 against the camera's own JPEG of the same exposure, which is an independent renderer
that applied the correction (RAW+JPEG, Lens Comp. ▸ Distortion Comp.: Auto). RAW+JPEG pairs from one
ILCE-7M3 (26 in the table below, plus two left out as explained under it): a checkerboard filling the frame on a TV (handheld) and on a desktop monitor (tripod), compressed and
uncompressed raws, apertures from wide open to f/8, plus one textured wall. Tables covered strong barrel (16 mm,
−690 at the corner), strong pincushion (70 mm, +1161) and the sign change between them.

Method (`tools/sony_distortion_check.py`): LightCraft renders each ARW at full size with the profile correction
on and off; sub-pixel board corners (Hessian saddle points refined with `cornerSubPix`, kept only where the four
quadrants alternate, labelled into rows and columns) are matched with the JPEG's by grid position, with the
match fixed by the central corners before any fit, because a board is periodic. A similarity (zoom, rotation,
shift) is fitted, as the camera fills its frame with its own zoom; no lens coefficient is fitted. The textured wall
uses SIFT matches instead. The script was checked on synthetic boards with a known radial distortion.

| Lens / focal length | Shots | Corrected vs JPEG: median / p95 (px at 6000) | Uncorrected median | Corner (r > 0.9) | Camera / LightCraft correction |
|---|---:|---|---:|---:|---:|
| FE 16–35 F4 ZA / 16 mm | 10 | 0.33–0.38 / 1.01–1.15 | 16.7–17.0 | 1.40–1.56 px | 1.02 |
| FE 16–35 F4 ZA / 20 mm | 4 | 0.16–0.24 / 0.34–0.50 | 3.9–4.4 | 0.15–0.24 px | 0.98 |
| FE 16–35 F4 ZA / 35 mm | 2 | 0.18–0.21 / 0.39–0.44 | 11.0–11.1 | 0.18–0.26 px | 1.01–1.02 |
| FE 28–70 / 28 mm | 2 | 0.24–0.25 / 0.59–0.64 | 13.2–13.4 | 0.92 px | 1.02 |
| FE 28–70 / 33 mm | 1 | 0.07 / 0.16 | 0.8 | 0.07 px | 1.00 |
| FE 28–70 / 50 mm | 1 | 0.30 / 0.69 | 20.8 | 1.24 px | 1.01 |
| FE 28–70 / 70 mm | 1 | 0.44 / 0.93 | 26.4 | 1.49 px | 1.01 |
| FE 50 F1.8 / 50 mm | 4 | 0.06–0.12 / 0.15–0.31 | 2.8–3.0 | 0.08–0.14 px | 1.00–1.01 |
| Wall, 16 mm (SIFT) | 1 | 0.37 / 1.06 | 5.6 | 1.22 px | — |

Residuals are in pixels of the 6000×4000 JPEG; at the 2376 px exports used above for the ILCE-7RM4A, the
board medians are 0.02–0.17 px and the p95 at most 0.46 px. The last column fits JPEG ≈ uncorrected +
α·(corrected − uncorrected) radially: LightCraft applies the camera's amount of correction within 2 %, slightly
less for strong barrel and slightly more at 20 mm, which is what remains at the 16 mm corners (about 1.5 px of a
35 px correction). Rows and columns of corrected boards are as straight as in the JPEG in every shot (largest
bow 0.8–7.8 px in both, from the screens), against 7–55 px uncorrected. Compressed and uncompressed raws,
the TV and the monitor, and wide open and f/8 agree. Two 16 mm board shots whose board stopped short of the
corners (reach 0.8 of the half diagonal) gave 0.85 px medians and are left out of the table.

On this body Sony's crop tags and the default crop agree (origin 12, 12, 6000×4000 of 6048×4024), so the
warp sits on the image centre; fitted against the camera, the centre is within 0.3 px. Unit tests keep the warp from
three of these tables within 0.0008 of the half diagonal (2.9 px at 6000 px) of the camera's fitted geometry.
The photographs stay with their owner; none is committed.

## Camera coverage and regression boundary

The model check is exact: `ILCE-7RM4` (A7R IV without the A suffix), unknown/missing
model names, and every other model remain unsupported by this distortion decoder. Matching tag
numbers and table lengths do not establish the scale, interpolation or crop normalization on another
body. Support can expand after independent geometry validation; do not add model-name prefix matching.

The current `cargo xtask corpus --download` manifest contains 25 Sony CC0 samples across 18 bodies,
including samples 4822 (A7R IVA / 24–105 mm) and 3989 (A9 II / 200–600 mm), each with a pinned SHA-256 that the
download checks. `sony_embedded_distortion_is_limited_to_validated_models` checks 23 of them (the two ILCE-7CM2
codings have their own test), and `sony_lens_records_of_the_public_samples` verifies all 25 against their pins
(`SONY_SAMPLES` in `crates/raw/tests/it/corpus.rs`, itself anchored by a pinned digest and compared with the manifest)
before decoding. **No CI job fetches the corpus**: these fixture tests run only where it has been downloaded, and
otherwise skip each absent file. `LIGHTCRAFT_CORPUS_REQUIRED=1` makes `sony_lens_records_of_the_public_samples`
fail on any absent pinned file instead.

| Camera | Corpus samples | Decoded record: tables, settings (shading / CA / distortion) | Applied |
|---|---|---|---|
| ILCE-7RM4A | Compressed (4822) | All three tables; 257 / 1 / 17 | One distortion warp |
| ILCE-9M2 | Compressed (3989) | All three; 257 / 1 / 0 | None |
| ILCE-7M3 | Compressed, uncompressed | All three; 257 / 1 / 0 | One distortion warp |
| ILCE-7M4 | 14-bit, lossless L/M/S | L/M/S: all three, 272 / 1 / 17; the 14-bit file (APS-C framing, 4608 × 3072): settings only, its 11-sample tables are not interpreted | None |
| ILCE-7RM2 | 12-bit uncompressed | All three; 256 / 0 / 1 | None |
| DSC-RX100 | One | Shading, from the `SR2SubIFD` (the raw IFD has no tables); CA and distortion in the 11-sample layout | None |
| DSC-RX100M3 | One | Shading; CA and distortion in the 11-sample layout | None |
| ILCE-7CR | Compressed, lossless L/M | All three; 272 / 1 / 0 | None |
| ILCE-7RM4 | 14-bit compressed | All three; 257 / 1 / 17 | None |
| ILCE-3500 | 5600 K | Settings only (unavailable) | None |
| ILCE-7S | Shade | An all-zero CA table; settings unavailable | None |
| ILCE-7SM2 | 16:9 framing | All-zero shading and CA tables; settings unavailable | None |
| DSLR-A500, DSLR-A700, SLT-A33, DSLR-A580, DSLR-A900 | One each | No record | None |

These are **model-boundary regression checks, not new geometric validation of those models**.
`sony_lens_records_of_the_public_samples` also checks the exact tables, settings, provenance and framing evidence of
the three A7CR codings and the A7R IVA sample, and that across the Sony corpus only the A7R IVA's distortion is ever
applied, besides the A7 III's. The A7 III tables were refused at first because their corner warp was unverified;
the [ILCE-7M3 validation](#ilce-7m3-validation) measured it against the camera's own JPEGs and enabled the body. The lossless M/S
files are linear YCbCr and remain excluded independently of the model check. Header and full decode
must agree for every sample. Synthetic tests also verify both TIFF byte orders, exact model matching,
that the warp is centred on the standard default crop while the image keeps Sony's crop, and that DNG export preserves an accepted warp without adding one to rejected
models. A non-3:2 Sony image crop is also rejected when the standard default crop still describes
3:2 geometry; the regression fails without the image-framing guard. The existing independent Sony
geometry tests continue to cover the enabled A7R IVA path.
Before restoring main's crop precedence, re-exporting the 24, 200 and 600 mm reference files after
the model restriction produced byte-identical corrected PNGs to the earlier validated exports. Isolated headless app checks showed the A7R IVA profile
controls present and the A7 III profile controls absent, with no notices and completed RAW renders.

Before enabling another body, compare corrected and uncorrected renders against independent references
across multiple lenses/focal lengths and photographs with features near the edges and corners. Record
residuals and spatial coverage, and check crop/aspect and compression variants. Embedded JPEGs are
useful when the camera actually applied distortion correction; an Off JPEG cannot certify an On warp.
Prefer same-renderer Sony On/Off exports when the camera setting or geometry is ambiguous. Table-fit
error alone only checks our polynomial approximation, not whether the table's interpretation is correct.
