# craft-develop

The raw develop pipeline, CPU path: a camera raw (bytes, or a [`craft-raw`](../raw/README.md) `RawImage`) or an
already-decoded linear picture, plus [`craft-develop-model`](../develop-model/README.md) settings, in; the rendered
picture out. No `unsafe`, no panics on hostile input, no file I/O, and it builds for `wasm32-unknown-unknown`.

```toml
craft-develop = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

```rust
use craft_develop::load::{LoadOptions, load_raw};
use craft_develop::{OutputDepth, OutputSpace, Quality, RenderRequest, StageCache, render_cached};
use craft_develop_model::DevelopSettings;

// bytes → linear Rec.2020 (D65), upright, plus what the settings are read against
let source = load_raw(&bytes, &LoadOptions::full())?;          // LoadOptions::fit(2560) for a binned preview
let settings = DevelopSettings::for_raw(source.info.as_shot_temp, source.info.as_shot_tint);
let src = std::sync::Arc::new(source.image);
let cache = StageCache::default();                              // one per view; reuses unchanged stages
let req = RenderRequest {
    space: OutputSpace::ProPhoto,                               // Srgb (default), DisplayP3, AdobeRgb, Rec2020
    depth: OutputDepth::U16,                                    // U8 (default), F32Linear, F32Hdr
    quality: Quality::Full,                                     // Draft while a slider is dragged
    window: None,                                               // Some(PixelWindow { .. }): a tile of the output
    ..RenderRequest::fit(4000, 4000)
};
let out = render_cached(&src, &source.info, &settings, &req, &cache);
// out.image: 8-bit RGBA (display-encoded), out.histogram, out.deep: Some(DeepImage) for U16 / F32
```

It is the third of the crates planned in craft-libs#587 (`craft-raw`, `craft-develop-model`, `craft-develop`). The
base is LightCraft's `lightcraft-pipeline`, the develop half of `lightcraft-raw` that craft-raw left out, the
`lightcraft-color`, `lightcraft-raster` and `lightcraft-inpaint` crates it builds on, and the raw load step and
camera look fit of `lightcraft-engine`, all at `a75497d6`. Issue numbers in comments are LightCraft's
(`storytold/lightcraft#N`).

## What is here

| Module | From LightCraft | Contents |
|---|---|---|
| crate root (`render`, `render_cached`, `plan`, `RenderRequest`, `SourceInfo`, `StageCache`, …) and `geometry`, `optics`, `upright`, `straighten`, `transform`, `local`, `local_tone`, `light_v3`, `tone`, `curve`, `colorops`, `finish`, `crystal`, `nrc_joint`, `masks`, `mask_space`, `spots`, `blemish`, `dust`, `redeye`, `lens_blur`, `lut`, `output`, `profiles`, `auto`, `cull`, `visualize` | `crates/pipeline` | The pipeline: geometry and lens corrections, white balance, exposure and light, local tone, tone mapping, colour, detail and noise reduction, masks (brush, linear, radial, range, AI mattes), spot removal, red eye, lens blur, grain and vignette, output spaces and depths, soft proofing, windows (`PixelWindow`) and the stage cache. Scene-linear Rec.2020 float throughout. |
| `load` | `crates/engine` `files/load.rs`, `files.rs`, `files/colour.rs`, `files/probe.rs` (`embedded_lens`) | Raw → the pipeline's source: `load_raw` (bytes) and `develop_raw` (a decoded `RawImage`). Binned development for previews, demosaic for full size, highlight rebuild, the camera colour model, the white-balance scale (`WbScaleOf`: Kelvin or relative), the `SourceInfo` with the camera's colour, tone, lens corrections, mattes and gain table map. |
| `look` | `crates/engine` `camera_preview.rs`, `camera_preview/`, `camera_profiles.rs` | The starting look of a raw without colour matrices (most non-DNG raws), fitted to its own camera JPEG: matrix, hue/saturation table, tone and chroma curves, with the bundled camera profiles (`look::profiles`). |
| `sensor` | `crates/raw` (the parts craft-raw left out) | Normalisation (black, white, active area, `OpcodeList1/2`), demosaic (bilinear, PPG, AHD, X-Trans), binned development, highlight reconstruction, Remove Hot Pixels, samples stuck below the white level, the DNG colour model, spectral and chart-measured camera matrices, Kelvin from a camera's white-balance presets, the ERF / MEF white-balance estimate, DCP profiles, applying opcode lists, profile tables, gain table maps and semantic masks. `sensor::prepare` does what LightCraft's decoders did to the samples. |
| `color` | `crates/color` | `Mat3`, RGB spaces, Bradford, CCT and temperature / tint, transfer curves, OkLab / OkLCh, splines. |
| `raster` | `crates/raster` | `Image<T>` (`Rgb32f`, `Rgba8`, `Plane`), resampling, blur, morphology, histograms. |
| `inpaint` | `crates/inpaint` | PatchMatch completion and membrane fill (spot removal). |
| `preview` | new | The raw's embedded camera JPEG decoded to linear Rec.2020 for the look fit and the white-balance estimate. |

`color`, `raster` and `inpaint` are modules, not crates of their own: nothing else in craft-libs uses them yet, and
`craft-cms` (ICC) covers colour management between profiles, which this crate does not do. The output of a render
is in the requested `OutputSpace`; converting to a document's ICC profile is the caller's, with `craft-cms`.

## Output

`Rendered::image` is always 8-bit RGBA, display-encoded in `RenderRequest::space` (or `RenderRequest::display`).
`RenderRequest::depth` adds `Rendered::deep`: `U16` display-encoded in the space's transfer curve, `F32Linear`
linear in the space's primaries (0..1), `F32Hdr` linear with HDR headroom. `load` gives linear Rec.2020 D65,
scene-referred (1.0 = sensor white after the baseline exposure; highlights above it are kept), upright.

## Seams: what the caller supplies

LightCraft fills these from its catalog, lens database and model downloads; here they are inputs.

| Input | Where |
|---|---|
| A lens profile for a file that embeds no correction (LightCraft: Lensfun database) | `LoadOptions::lens_fallback` |
| An imported camera profile (`.dcp`; read with `sensor::dcp::parse_dcp`) | `LoadOptions::dcp` |
| Remove Hot Pixels and its limit | `LoadOptions::hot_pixels` |
| User camera profiles for the look fit (LightCraft: a folder of JSON) | `look::profiles::register` |
| AI segmentation masks (LightCraft: SAM 3) | stored in the settings as `SegMask`s by the app |
| DNG semantic mattes | read by `load` from the file; other mattes through `SourceInfo::mattes` |
| The original's size when rendering from a smaller copy | `SourceInfo::native_long` |

## Not here

- The wgpu path (`lightcraft-gpu`) and its CPU-vs-GPU equivalence tests: the next step of #587.
- AI denoise (the denoised twin of a raw, `load_vec_pair`), SAM 3 segmentation, faces, the lens database, the
  catalog, the command registry, UI, export and print.
- Fitting camera profiles from many photos (`lightcraft-cli calibrate`, `camera_profiles::Pool`); `look::fit_profile`
  and `look::profile_pairs`, which it is built on, are here.
- Decoding other image formats (`lightcraft-codecs`): a decoded picture goes to `render` as linear Rec.2020 with
  `SourceInfo::default()`.
- The pipeline's examples (`develop_sheet`, `guided_memory`) and raster's `blur_bench`.

## Changes from the base

- LightCraft's methods on its own `RawImage` (`normalized`, `develop`, `develop_binned_masked`, `lift_clipped_samples`,
  `suppress_hot_pixels`, …) and on `HsvTable`, `ToneCurve`, `ProfileLook` and `GainTableMap` are extension traits on
  craft-raw's types (`use craft_develop::sensor::prelude::*`). Colour-tag matrices are craft-raw's `[[f64; 3]; 3]`
  and its white is `[x, y]`; `sensor::mat3`, `sensor::color::xy_of` and `sensor::orientation` convert.
- What LightCraft's decoders did and craft-raw does not is `sensor::prepare`, which `load` calls: lifting a DNG's
  samples stuck below the stated white, and the ERF / MEF white-balance estimate. The MEF estimate reads the
  embedded JPEG, not the uncompressed RGB preview LightCraft's MEF reader used.
- A Kelvin scale from a Fujifilm A / D65 pair (craft-raw keeps the pair) is placed at five temperatures here
  (`KelvinPresets::from_file`), as LightCraft's decoder did.
- The camera JPEG is decoded with `zune-jpeg` in the colour space the raw states for it (sRGB or Adobe RGB); an ICC
  profile embedded in the JPEG is not read (LightCraft's codec layer honours one).
- A raw variant craft-raw recognises but does not decode develops as its embedded JPEG only on
  `RawError::Unsupported`, oriented as the raw says; LightCraft also read the JPEG's own Exif orientation.
- Camera profiles: bundled, or registered by the app (`look::profiles::register`), instead of a folder.
- `craft-raw` makes `Cfa::valid` public and exports `dng_color_data` (the DCP reader's).
- The crate builds at `opt-level = 1` in dev (as LightCraft builds the pipeline), which also makes a debug build's
  floats match LightCraft's.

## Tests

`cargo test -p craft-develop`: 599 unit tests, 6 integration tests and one doc test pass; 16 are ignored (benches and
diagnostics LightCraft ignores too, and the one below). No sample files are committed: sources are procedural
(`scenes`, a test-only module from `lightcraft-scenes`) or DNGs written by craft-raw's writer.

- Moved with their modules: the pipeline's tests (with `tests/it/preview_save.rs`), color, raster, inpaint, and
  the develop-side tests of `lightcraft-raw` and of `camera_preview`.
- Back from craft-develop-model: the render halves of the `crs:` mask and spot tests (`tests_crs`).
- New: `tests/it/load.rs`, raw bytes to a 16-bit ProPhoto render, windows and draft quality.
- `tests_light::v1_and_v2_render_as_before_v3` is ignored: its golden hashes do not reproduce in LightCraft at
  `a75497d6` either (LightCraft's own run gives the hashes this crate gives).
- Left out, because they need files that are not in the repository or engine code that did not move: 17 of
  `camera_preview`'s tests (the `corpus_*` tests on real raws, the Photo Merge white balance, the DNG lens-proxy test
  built on the engine's XMP fixture), `gaintable::bench::gain_throughput` (a file named by `GTM_FILE`), the folder
  and cache-key tests of `camera_profiles` (replaced by a JSON and registry test), and `lightcraft-engine`'s
  `files/tests.rs` (the whole load path on corpus files, with the catalog and codecs).

## Sources

Moved from LightCraft (see `NOTICE`, *Source provenance*). The spectral camera matrices are derived from
rawtoaces-data (Apache-2.0), the chart matrices and camera profiles are LightCraft contributors' own measurements
(see `ATTRIBUTION.md`). No code from dcraw, LibRaw, rawspeed, darktable, RawTherapee or Adobe's SDKs.
