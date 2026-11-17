# craft-ease

Easing curves, keyed tracks, constant-speed motion paths and damped springs for animation in the
Craft apps. The crate depends only on the standard library, with `serde` as an optional feature
(on by default). It builds for `wasm32-unknown-unknown`.

Moved from StudioCraft's `craft-ease`
([`crates/craft-ease`](https://github.com/blamejs/StudioCraft/tree/b1ead5a9db0d478bf7171ce9e942e600ca500fea/crates/craft-ease)).
Parts of it started in other Craft apps:

- `Easing`, `CustomEase`, `SampledEase` and the curve editing came from AniCraft's `anicraft-doc`
  (`crates/doc/src/easing.rs`). They now use `f64` and the crate's own `Vec2`, the Back and
  Elastic In Out curves follow the published equations, and `Hold` and `Spring` are new
  variants.
- Speed and influence eases, hold keys and the constant-speed motion paths came from
  EffectCraft's `effectcraft-keyframe` (`crates/keyframe/src/lib.rs`). `Track` takes any
  `Animatable` value instead of the app's value type, and `MotionPath` continues in a straight
  line past either end.
- `Spring` replaces AniCraft's stepped spring (`crates/doc/src/spring.rs`) with the exact
  solution of the damped spring equation, so a step costs O(1) and any step length is stable.

Licensed under MIT OR Apache-2.0.

## Contents

- `Easing` maps linear progress `t` in `0..=1` to eased progress. The variants are no ease
  (`Linear`), `Hold`, the Classic slider (−100 to +100), nine named families (Quad, Cubic, Quart,
  Quint, Sine, Back, Circ, Bounce, Elastic) each In, Out and In Out, `CubicBezier` as the CSS
  Easing Functions specification defines `cubic-bezier()`, an editable `CustomEase`, a
  `SampledEase` table and a physical `SpringEase`. `Easing::apply(t)` returns exactly 0 for
  `t <= 0` and NaN, and exactly 1 for `t >= 1`. It does not allocate.
- `Easing::from_temporal` builds the cubic Bezier for a segment from the speed and influence of
  its two sides (`TemporalEase`), as keyframe velocity dialogs give them. `Easing::temporal`
  reads them back.
- `Track<T>` holds keys of any `Animatable` value (`f32`, `f64`, `Vec2`, `[f32; N]`,
  `[f64; N]`), each with the ease to the next key. `Track::sample` is a binary search and does
  not allocate.
- `MotionPath` is a chain of cubic Bezier segments with an arc-length table per segment, built
  once. `point_at` and `point_at_length` move at constant speed along it and continue in a
  straight line past either end. `MotionPath::through` sets the handles of a smooth path through
  a list of anchors.
- `Spring` and `SpringState` move a value toward a target. `SpringState::step` applies the exact
  solution of the damped spring equation over the step, so a step costs O(1), any step length is
  stable, and many short steps give the same value as one long one.

## Checked and lenient constructors

Every type has a checked constructor that returns `EaseError` for a value that is not finite or
is out of range: `Easing::cubic_bezier`, `Easing::from_temporal`, `Easing::spring`,
`CustomEase::try_new`, `SampledEase::try_new`, `Track::from_keys`, `MotionPath::new`,
`Spring::new` and `Spring::from_physical`. `CustomEase::new` and `SampledEase::new` repair what
they are given instead, and files are read through the same repairs. Reading a track refuses a
file with more than `MAX_TRACK_KEYS` keys while it reads.

## Limits

| Constant | Value |
| --- | --- |
| `MAX_EASE_Y` | 10 (eased values stay within ±10) |
| `MAX_EASE_POINTS` | 64 points in a custom ease |
| `MAX_EASE_SAMPLES` | 1,025 samples in a sampled ease |
| `MAX_TRACK_KEYS` | 100,000 keys in a track |
| `MAX_PATH_SEGMENTS` | 10,000 segments in a motion path |
| `MAX_SPRING_HZ` | 10,000 swings a second |
| `MAX_DAMPING_RATIO` | 1,000,000 |
| `MIN_INFLUENCE` | 0.001 |

## Sources

- The named families are the easing equations published by Robert Penner. Back In Out and
  Elastic In Out use the published In Out equations, which scale Back's overshoot by 1.525 and
  give Elastic a longer period.
- `CubicBezier` follows the CSS Easing Functions Level 1 specification.

## Details

- Values are `f64` throughout.
- The `serde` feature reads and writes `Easing` (spring eases included) and `Track`.
  `MotionPath`, `Spring` and `SpringState` are not serialized.
- Serialized eases keep their form: presets are strings such as `"QuadIn"`, and `Classic`,
  `CubicBezier`, `Custom` and `Sampled` are one-key objects. `Hold` and `Spring` are the last
  variants. A track is read from its fields by name or in order.
- `Easing::apply` returns exactly 0 and 1 at the ends for every variant, and `+∞` gives 1.
- `CubicBezier` keeps y1 and y2 within ±10 when it is evaluated. Its solver inverts the curve
  with both handles at full reach in closed form, and otherwise stops within 1e-12 of the span
  in at most 64 steps.
- A sampled ease's name drops control characters, bidirectional overrides and invisible format
  characters such as zero-width spaces. A sampled ease with no name left is named "Sampled ease".
  Reading a file keeps the first 65,536 entries of a custom ease's points or a sampled ease's
  values and skips the rest.
- Without the `serde` feature the crate has no dependencies, and its tests that read and write
  files are left out.
- `Track` takes `Easing::Hold` for held keys and treats −0 and +0 as one time. It samples keys
  at opposite ends of the `f64` range without overflow, and returns `EaseError::Overflow` when
  an overshooting ease carries a value past what its type holds.
- Speed and influence become a `CubicBezier` (`Easing::from_temporal`), with the influence
  allowed from 0.001 to 1 on each side, including a sum above 1.
- Motion paths are measured by Gauss-Legendre quadrature per table cell with Newton refinement.
  `MotionPath::through` sets each handle to a sixth of the line between the anchor's neighbors.
  A segment whose handles lie on its chord is treated as a straight line.
- `Spring::from_physical(mass, stiffness, damping)` takes a mass, a stiffness and a damping
  coefficient. A spring ease that is still moving after 10 seconds has the distance it is still
  off taken out evenly over the ease, so it reaches 1 without a jump. Any other spring ease is
  within about 0.1 % of 1 just before it reaches 1.
