# Craft plate test

`craft-plate-test` holds print-color test scenes for the Craft apps and the tools to check PDFs
against them. A scene is an app-neutral JSON description of one page (spot inks, CMYK fills,
transparency, overprint, soft masks, placed PDFs), the ink each plate must carry at sample points,
and the PDF structure a correct print export has.

- `scene`: serde types for the scene files in `scenes/`, and `Scene::validate` for the checks a
  schema can't make (declared inks, geometry fields per object type, plates named by points).
- `reference`: writes a scene to PDF with krilla through craft-pdf. It tests the shared writer on
  its own. A feature the writer can't express yet returns `Error::Unsupported`.
- `structure`: summarizes a PDF with hayro-syntax (page and form transparency groups with `/S /CS
  /I /K`; graphics states with `OP op OPM BM SMask`; color space families; DeviceRGB use;
  output intents) and checks the summary against a scene's `structure` block.
- `gs`, `tiff`, `judge`, `report`: the local plate check (below).

`tests/scenes.rs` parses every scene, writes every scene without `pending`, and checks the
structure. It runs in CI in seconds, with the unit tests of the TIFF reader and the judge.

## Scenes

The file names say what each scene tests; `title` and `source` say more.

| IDs | Cases |
|---|---|
| `S01`–`S17` | Print-color research cases: overprint, spots, blending spaces, knockout and isolated groups, masks, placed PDFs, gradients |
| `D01`–`D10` | DesignCraft print cases (its DC list) and a GWG idea (D04) |
| `M1`, `M2`, `N1` | Luminosity mask backdrop and transfer; DeviceN overprint |
| `F01` | Flattener: a semi-transparent image over a flat fill; where the image's alpha is 0 the fill prints unchanged (vectorcraft#2160) |
| `F02` | Flattener: art hidden under opaque fills, with transparency over them, leaves no trace (vectorcraft#2161) |

## Scene format

Coordinates are points from the page's top-left corner; rectangles are `[x, y, width, height]`.
Ink values are percentages (0 = no ink, 100 = solid). Plates use separation names: `Cyan`,
`Magenta`, `Yellow`, `Black` and the spot names.

| Field | Meaning |
|---|---|
| `id`, `title`, `source`, `note` | Identity and origin (research case, app regression) |
| `page` | `[width, height]` in pt |
| `inks` | Spot inks, each `{"name", "cmyk": [c, m, y, k]}` or `"lab": [L, a, b]`; their order is the DeviceN colorant order after the process inks |
| `settings` | `group_cs` (`"rgb"` or `"cmyk"`: the transparency blending space), `output_intent` (`"fogra39"`: writes the scene with craft-pdf `write::print` settings and craft-cms's `COATED_FOGRA39_ICC`; `tests/scenes.rs` also writes every non-pending scene this way) |
| `objects` | Drawn in order (below) |
| `expect` | `plates`: every plate the separation must produce, process plates included, and no others. `points`: `{at, kind: "area" or "text", region (text), plate (text, default Black), inks: {plate: value or [lo, hi]}, tol, info, note}`; unlisted plates expect 0, `info` points are measured but not judged |
| `structure` | Expected PDF structure (below) |
| `pending` | The task that makes the scene writable, e.g. `"T4"`; pending scenes are parsed but not written |

Objects have a `type`: `rect` and `ellipse` (`rect`), `text` (`at` = baseline start, `size`,
`text`), `image` (`rect`, `image`), `pdf` (`rect`, `scene` = id of a scene placed as a page),
`group` (`objects`, `isolated`, `knockout`). Any object can carry `opacity` (whole object, as a
transparency group), `blend` (`multiply`, `color`, … in kebab case), `overprint` (`{fill, stroke,
mode}`, mode = OPM, default 1) and `mask` (`{kind: "luminosity" or "alpha", content: [objects],
backdrop, transfer}`; `backdrop` is a process or gray paint, `transfer` a list of samples 0–1). Shapes and text take `fill` and `stroke` (`{paint, width}`).

Paints:

| Paint | Example |
|---|---|
| Process | `{"cmyk": [100, 0, 0, 0]}` |
| Gray (black ink %, written as DeviceGray) | `{"gray": 50}` |
| RGB (0–255) | `{"rgb": [255, 0, 0]}` |
| Spot | `{"spot": "Gold", "tint": 50}` (tint defaults to 100) |
| Several inks (DeviceN) | `{"inks": {"Cyan": 60, "Black": 40}}` |
| Registration (`/All`) | `"registration"` |
| Linear gradient | `{"linear": {"from": [x, y], "to": [x, y], "stops": [{"at": 0, "paint": …}, …]}}` |
| Mesh gradient (one Coons patch with straight edges) | `{"mesh": {"rect": [x, y, w, h], "corners": [paint, paint, paint, paint]}}` (top left, top right, bottom right, bottom left) |

Images are synthetic: `{"px": [w, h], "ramp": [paint, paint]}` (left to right),
`"split": [paint, paint]` (left and right halves) or `"feather": pt` (a gray soft-mask image,
white inside and fading to black at the edges). Shadows and feathers are luminosity masks over
such an image. A ramp or split image can carry `"alpha": {"rect": [x, y, w, h], "opacity": 0.6}`,
written as its `/SMask` like a PNG with transparency: pixels whose centers lie in `rect` (page
pt) have that opacity, the rest 0.

Tolerances: ±2 for device, spot and overprint values; ±6 for blends and ICC; ±10 for shadows and
gradients.

### The `structure` block

Each field is optional; only the fields present are checked.

| Field | Check |
|---|---|
| `page_group` | The page group's `/CS` family; `"none"` = no page group, `"no-cs"` = a group without `/CS` |
| `group_cs` | The set of `/CS` values of form and soft-mask groups (`"none"` = no `/CS`) |
| `knockout`, `isolated` | Some group has `/K true` / `/I true` (page groups excluded for `isolated`) |
| `overprint`, `smask` | Some graphics state sets `OP` or `op` true / a soft mask |
| `blend_modes` | The set of blend modes other than Normal |
| `ext_g_states` | Each entry (keys `OP op OPM BM SMask BC TR`) matches some graphics state |
| `color_spaces`, `color_spaces_include` | The exact set / a subset of color space families, e.g. `DeviceCMYK`, `Separation(Gold, alt DeviceCMYK)` |
| `device_rgb` | Some painting color space, alternates included, is DeviceRGB |
| `output_intents` | The `/S` of each output intent |

A failing check prints the full summary as JSON, which is also the quickest way to write a new
block.

## Adding a scene

1. Add `scenes/<ID>-<slug>.json`. Reuse the 360 × 216 pt page and the `Gold` spot unless the
   case needs otherwise. Use fonts and images that the harness makes; never commit fonts, GWG
   files or other third-party test files.
2. Add `pending` when the writer can't express the scene yet, and remove it in the change that
   adds the feature.
3. Run `cargo test -p craft-plate-test`.

Text uses `Inter-Regular.ttf` from `CRAFT_PDF_TEST_FONT_DIR`
(`python scripts/fetch-pdf-test-fonts.py <dir outside the checkout>`). Without it, text objects
are skipped with a printed note.

## Use from an app

Add the crate as a dev-dependency and write an adapter in the app's tests: build each scene as an
app document, export it through the app's own print path, and check the result with
`structure::summarize` and the scene's `structure` block. Scenes the app can't build yet are
skipped by the adapter, not edited.

```rust
let scenes = craft_plate_test::scene::load_dir(&craft_plate_test::scene::scenes_dir())?;
for scene in scenes.iter().filter(|s| s.pending.is_none()) {
    let pdf = my_app_export(scene)?; // the app's adapter
    let summary = craft_plate_test::structure::summarize(&pdf)?;
    assert!(scene.structure.check(&summary).is_empty());
}
```

## Plate check (local only)

```sh
scripts/plate-check.sh
```

The check writes every scene, separates it with Ghostscript's `tiffsep` device at 72 dpi with
overprint simulated, and judges the plates:

- Area points: the median of the 5 × 5 pixels around the point, per plate.
- Text points: the median over the glyph's pixels, those in `region` with at least half the
  darkest ink on the point's `plate`.
- Each point's plates within `tol` of the expected value (or inside the `[lo, hi]` range).
- The separation produces exactly the scene's `plates`.

Pending scenes are written without the features they wait for (`reference::write_partial`) and
are expected to fail; the report shows what the writer prints today. A pending scene that passes
is reported as such: its plates don't show the missing feature under Ghostscript, so its
`structure` block is the check that does. The test fails when a scene without `pending` fails or
the harness errors.

`scripts/plate-check.sh --flatten` runs the same check on each scene's page flattened by
craft-prepress (`flatten_page` at 300 dpi; the test is `flatten_plates` in craft-prepress, since
craft-prepress dev-depends on this crate). The flattened page must also have no transparency
left (`structure::transparency`: transparency groups, soft masks in graphics states, blend modes
other than Normal, `ca`/`CA` below 1). Its report goes to `plate-test/flattened/` and shows both
pages side by side, with the flattener's time per scene.

The results go to `plate-test/report.md` and `results.json` in Cargo's target directory
(`CARGO_TARGET_DIR` is respected). The script sets `CRAFT_PDF_TEST_FONT_DIR` to
`~/.cache/craft-pdf-test-fonts` when the font is there; without the font, text points are skipped.

Ghostscript is AGPL. It runs only as an external program on developer machines (`gs` on `PATH`, or
`CRAFT_PLATE_TEST_GS`), never in CI. Don't link it, copy its code, or commit its output. Without
Ghostscript the test returns with a note.
