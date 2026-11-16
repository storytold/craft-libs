# craft-render-parity

Test support for the shared develop pipeline ([#587](https://github.com/storytold/craft-libs/issues/587)):
how far two renders of the same photo are apart, and whether that is within a stated tolerance.
A develop stage that moves from an app, or an app that switches to the shared pipeline, renders
the same fixture both ways and compares the two here.

```toml
[dev-dependencies]
craft-render-parity = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

```rust
use craft_render_parity::{Image, Tolerance, compare};

let expected = Image::new(width, height, &app_render)?;
let actual = Image::new(width, height, &shared_render)?;
let report = compare(&expected, &actual)?;
assert!(report.within(&Tolerance::BASELINE), "{report}: {:?}", report.exceeded(&Tolerance::BASELINE));
```

## Input

Scene-linear RGB with Rec. 709 / sRGB primaries and a D65 white, three interleaved `f32` samples
per pixel, row by row, with 1.0 as diffuse white. Values above 1.0 are compared as they are. A NaN
or infinite sample is an error, never a pass.

## Measures

| Measure | Meaning |
|---|---|
| `max_delta_e`, `mean_delta_e` | CIEDE2000 (kL = kC = kH = 1) in CIELAB relative to D65, from `craft_cms::math` |
| `max_channel`, `mean_channel` | Absolute difference of the linear samples, overall and per channel |
| `worst` | The first pixel with the largest CIEDE2000 difference, or the first that differs at all |

`Tolerance::BASELINE`, the acceptance bar of #587, is CIEDE2000 at most 0.5 at any pixel and 0.05
on average (1.0 is about the smallest visible difference), and at most 1/1024 in any linear sample.
