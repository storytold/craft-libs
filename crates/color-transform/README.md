# craft-color-transform

Color transforms for the Craft apps: RGB spaces and chromatic adaptation, white balance by
temperature and tint, OkLab and OkLCh, monotone tone curves and 1D lookup tables, and PQ tone
mapping. It builds on [`craft-color`](../color), which holds the frame color tags, the Y'CbCr
matrices and the sRGB, PQ and HLG transfer functions. It depends on `craft-color`, `craft-cms`
and `serde`, contains no `unsafe` code, and builds for `wasm32-unknown-unknown`. The per-pixel
functions take `f32` and do not allocate.

The matrix arithmetic comes from `craft_cms::math`. `Mat3` wraps its row-major `[[f64; 3]; 3]`
and converts to and from it with `From`. `mul`, `apply` and `determinant` call `math::mul`,
`math::apply` and `math::det`, `Mat3::inverse` calls `math::inverse`, `Xy::to_xyz` calls
`math::xy_to_xyz` after checking the chromaticity, `BRADFORD` is `math::BRADFORD`, and
`bradford_xyz` calls `math::bradford_checked`. `math::inverse` divides the matrix by its largest
entry before taking the determinant. `math::invert`, which `craft-cms` uses to validate ICC
matrices, keeps its absolute determinant floor of 1e-12. `RgbSpace::to_xyz` builds the
normalized primary matrix with `math::inverse` rather than with `math::rgb_to_xyz_matrix`, which
inverts with `math::invert`, and returns the identity when the normalized matrix has no inverse.

Moved from StudioCraft's `craft-color-transform`
([`crates/craft-color-transform`](https://github.com/blamejs/StudioCraft/tree/2abe1824aa0d849faf8d5f91557d68ebe577f495/crates/craft-color-transform)).
Parts of it started in other Craft apps:

- `cct`, `oklab` and `curve` came from LightCraft's `lightcraft-color` (`crates/color/src/cct.rs`,
  `perceptual.rs` and `spline.rs`). `MonotoneCurve` now loads a saved curve through its
  constructor, so a file cannot give it arrays of different lengths, and it bounds the points it
  keeps and the size of their coordinates. `OkLab::for_space` extends the BT.2020-only OkLab
  conversion to any `RgbSpace`.
- `tonemap` came from FilmCraft's `filmcraft-color` (`crates/color/src/transform.rs`,
  `bt2390_eetf` and its maxRGB operator). It now follows Report ITU-R BT.2408-8 Annex 5 with
  the black lift, passes values through when the target peak is at or above the source peak,
  and handles a target peak below a third of the source range (see `Eetf`).

License: MIT OR Apache-2.0.

## Contents

| Item | What it does | Source of the formula |
| --- | --- | --- |
| `Mat3`, `Xy`, `RgbSpace` | 3×3 matrices, CIE 1931 chromaticities, RGB spaces from primaries and a white point. `RgbSpace::to_xyz` is the normalized primary matrix. `RgbSpace::from(craft_color::Primaries)` gives the space of a frame tag. | SMPTE RP 177; primaries from ITU-R BT.709, BT.601, BT.2020 and SMPTE EG 432-1 |
| `bradford`, `bradford_xyz` | The Bradford chromatic adaptation matrix between two whites. `RgbSpace::to_space` converts between spaces with it. | The Bradford cone response matrix |
| `cct::temp_tint_to_xy`, `cct::xy_to_temp_tint` | A white at a correlated color temperature and tint, and back. Tint is Duv × 3000 along the locus normal in CIE 1960 (u, v); positive is towards green. | Kim et al., "Design of Advanced Color Temperature Control System for HDTV Applications", Journal of the Korean Physical Society 41 (2002) |
| `cct::wb_matrix`, `cct::white_balance` | The RGB matrix that makes a scene lit by one white look lit by another. | Bradford adaptation |
| `oklab` | OkLab from linear sRGB with the published matrices, from XYZ, and for any `RgbSpace` (`OkLab::for_space`, with `f32` matrices; `oklab::lms_matrices` gives the same matrices in `f64`); OkLCh. | B. Ottosson, "A perceptual color space for image processing" (2020) |
| `curve::MonotoneCurve` | A tone curve through points that does not overshoot between them. It saves as its list of points. `knots` gives each point with its tangent for evaluation in a shader. | Fritsch and Carlson, "Monotone Piecewise Cubic Interpolation", SIAM J. Numer. Anal. 17 (1980) |
| `curve::Lut1` | A 1D lookup table over 0..1 with linear interpolation. | |
| `tonemap::Eetf` | The reference EETF from a mastering display to a target display, with the black lift, on PQ values or nits. When the target peak is below a third of the source range in PQ, where the Annex's knee would be negative, the knee is 0 and the spline starts with a slope of 3 × maxLum. When minLum is above 0.25, where the Annex's lift minLum × (1 − E2)⁴ would make the output fall near black and raise the peak above the target peak, the lift is minLum × (1 − E2 / r)ᵏ with r = max(T, min(1, 2 − 4 minLum)) and k = min(4, r / minLum), where T is the smaller of maxLum and 1. This equals the Annex's lift up to minLum 0.25, keeps the output rising, changes continuously with the target black, and from minLum 0.5 reaches 0 at the top of the output. A target black above the source peak reads as the source peak. Both rules are the crate's own. | Report ITU-R BT.2408-8, Annex 5 |
| `tonemap::ToneMap` | The EETF applied to linear RGB by max(R, G, B) as a ratio, with 1.0 at the 203-nit reference white. | Report ITU-R BT.2408-8, Annex 5 (the maxRGB option) |

## Hostile input

- A chromaticity with `y` at or below 0, or with a coordinate that is not finite, is rejected by
  `Xy::to_xyz`. `bradford` then returns the identity, and so does `RgbSpace::to_xyz` for such a
  white, for collinear primaries and for a white that gives a primary no weight. `Mat3::inverse`
  and `bradford_xyz` return `None` or the identity instead of a matrix with entries beyond
  `f64`.
- Temperatures are clamped to 1000 K to 100 000 K, and NaN reads as 6500 K. A tint that is not
  finite reads as 0.
- `MonotoneCurve` keeps at most 1024 points and drops points with a coordinate that is not
  finite or is beyond ±1e6. A saved curve is read back through the same constructor, so a file
  cannot give it mismatched arrays.
- `Lut1::from_fn` and `MonotoneCurve::to_lut` build at most 65 536 entries.
- Luminances are clamped to 0..10 000 nits. A target peak that is NaN or not above 0 gives an
  EETF and a `ToneMap` that pass values through.

## Tests

`cargo test -p craft-color-transform` checks the code against published values: the sRGB matrix of
IEC 61966-2-1, the BT.709 and BT.2020 luminance weights, the BT.709 to BT.2020 matrix of
ITU-R BT.2087, the D65 to D50 Bradford matrix as Lindbloom's chromatic adaptation page prints it,
illuminant A and D65 from CIE 15:2004 (temperature, Duv, CIE 1960 coordinates), Ottosson's XYZ to
OkLab table, the OkLab value CSS Color Module Level 4 gives for `#7654CD`, the PQ levels of
BT.2408, and EETF values computed from the BT.2408 Annex 5 equations by a separate
implementation.
