# craft-develop-model

Raw develop settings as data: the second of the three crates planned in craft-libs#587
(`craft-raw`, `craft-develop-model`, `craft-develop`). No rendering, no GPU, no I/O. An app can
store, copy, merge, migrate and serialise a photo's edits without depending on the pipeline.

```toml
craft-develop-model = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

The base is LightCraft's `lightcraft-develop` and the `crs:` import from `lightcraft-engine`
(`crs.rs`, `crs_masks.rs`) at `a75497d6`. Issue numbers in comments are LightCraft's
(`storytold/lightcraft#N`).

## What is here

| Module | Contents |
|---|---|
| `settings` | `DevelopSettings`, the complete, serialisable description of a photo's look: white balance (Kelvin or relative scale), light, tone curve, HSL / mixer / point colour, B&W mix, colour grading, calibration, effects (vignette, grain), detail, optics (lens profile, embedded corrections, defringe), geometry and Upright, crop, masks (brush, linear, radial, range, AI segmentation), spots, red eye, lens blur, enhance, HDR. `ProcessVersion` / `Process`: which formulas an edit renders with, so later formula changes leave existing edits alone ([process versions](docs/process-versions.md)). |
| `controls` | `CONTROLS`: every numeric slider by id, with its label, section, range, default, step and track gradient. UIs and agents address sliders through it. |
| `presets` | `Preset`, `SettingsGroup` (the groups a copy, paste, sync or preset covers), `extract_groups`, `apply_partial` (a partial settings object merged at an amount), `deep_merge`. |
| `segmask` | `SegMask`: a compressed AI segmentation mask stored inside the settings. |
| `crs`, `crs_masks` | Camera Raw / Lightroom `crs:` properties (from `craft_metadata::parse_xmp`) mapped to a *partial* `DevelopSettings` JSON object, which is merged like a preset: global adjustments, tone curves, white balance (`Target` decides how a `crs:` temperature is read), crop, Upright, local corrections and retouch spots, with a report of what was read and what was left out. `preset_from_xmp` reads an XMP preset. The field table is in [xmp-interop](docs/xmp-interop.md). |

Reading the `crs:` packet itself (names, structures, Camera Raw's number formats) is
[`craft-crs`](../crs/README.md)'s job; this crate decides what those settings mean for the edit
model. The mapping is a best-effort approximation of the other application's look, not a pixel match.

## Different from LightCraft

Fixes made here that LightCraft `a75497d6` does not have. It gets them when it switches to this crate.

- `to_json_for_merge` states every setting the stored JSON leaves out at its default (HDR, the profile and its camera
  local tone, the curve, Lens Blur's and Denoise's optional settings), not only Remove Hot Pixels. Copying a group
  turns them off on the target, and a preset at an amount starts from them.
- `reset_section` resets the sliders of point colours and red eyes, Upright with its guides, and the lens profile and
  Remove Chromatic Aberration switches. Settings from outside the app keep those sliders in range.
- `is_unedited` counts a chosen white balance as an edit. As shot, the numbers stay the photo's own.
- A preset at no amount adds no mask, and its Lens Blur focus mask follows its mask to the id the photo gives it.
- `crs:` import: a brush dab that is not a finite number is dropped, brush runs that differ in `crs:MaskInverted` stay
  two components, `crs:RetouchInfo` is read only without `crs:RetouchAreas`, and an XMP preset keeps its masks.
- `Homography::from_quads` (`craft-geom`) solves a mapping that sends the origin to infinity.

## Not here yet

- Writing `crs:` back from `DevelopSettings`. LightCraft writes its own `lc:settings` in a sidecar; PhotoCraft's
  Camera Raw writes `crs:`. That comes with the PhotoCraft switch, together with the migration of PhotoCraft's
  current Camera Raw settings.
- The tests that render a mask or a spot to check its sense (inside or outside a radial, where a spot lands)
  need the pipeline. They come back with `craft-develop`. The mapping half of each is kept here.
- LightCraft-only paths: `Target` from a catalog photo, and `.lrtemplate` (Lua) presets.
