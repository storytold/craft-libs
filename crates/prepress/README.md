# craft-prepress

Prepress for the Craft apps: separations, ink limits and the ink manager.

- `ink`: the ink model the writers and previews share. `InkSet` holds the four process inks
  (always first, in plate order) and the spot inks of a job, and is the job's ink manager
  (aliases, spots to process); `InkColor` is a color as ink tints (`Separation`, `DeviceN`,
  registration or none); `Overprint` holds `op`, `OP` and `OPM`. Spot names are compared as
  PDF names: `/Gold#20Foil` and `Gold Foil` are one ink. `to_pdf_color` writes an `InkColor`
  as a krilla color for export.
- `plates`: one 8-bit plane per ink (255 = 100% ink) rendered from a PDF page.
- `coverage`: ink coverage at a pixel and over a page, total area coverage (TAC) per pixel,
  and the pixels and regions over a TAC limit.
- `composite`: an sRGB preview of chosen plates.
- `cms`: process color conversion into a CMYK printing condition through craft-cms.

- `flatten` (feature `flatten`): the display list a transparency flattener reads and writes:
  fills, text runs, images and transparency groups in page space, with ink colors and overprint
  per item. Text runs keep their font (bytes, face index, a dedup key), glyph ids and positions,
  and Unicode text, so text without transparency stays live text; outlines are kept as the
  fallback for fonts that can't be embedded again. `flatten` turns the transparent parts into
  opaque region fills, compositing one value per ink (process and spot) with blend modes, group
  opacity, isolated and knockout groups, and overprint per plate. Items without transparency
  come back unchanged (JPEG images in DeviceGray or DeviceCMYK keep their DCT data; other images
  are written as 8-bit Flate samples); regions a gradient, image or soft mask reaches are listed
  for rasterizing. `flatten::raster` rasterizes them: each region's items are written alone,
  separated with `plates` and written back as a DeviceN image of the page's inks, clipped to the
  region, snapped to the device grid and a pixel larger than the clip so no paper shows at its
  edges. `flatten_document` flattens every page of a PDF, or a range of pages, into one PDF with
  the source's output intent, page boxes, title, author, creation date and modification date; a
  font used on several pages is embedded once. The caller sets the PDF version and the PDF/X
  version and conformance keys (for PDF/X-1a:2003, 1.4, `PDF/X-1:2003` and `PDF/X-1a:2003`);
  krilla has no PDF/X validator, so the app keeps its own PDF/X checks. `flatten_page` is its
  one-page case.

## Plates from an exported PDF

An app exports its PDF as usual, then separates a page of it:

```rust
use craft_prepress::{plates, InkId, Options, Process};

let p = plates(&pdf, 0, 150.0, &Options::default())?;
for (id, ink) in p.inks.iter() {
    let plane = p.plane(id); // p.width × p.height tints, row-major
    println!("{}: {} pixels inked", ink.name(), plane.map_or(0, |t| t.iter().filter(|v| **v > 0).count()));
}
let black = p.tint(InkId::process(Process::Black), 10, 10);
let gold = p.inks.lookup("Gold").and_then(|id| p.tint(id, 10, 10));
```

The plates are the PDF as printed, so a preview built on them shows what the writer produced,
not what the document model intended.

How a plate is made: the page renders once per ink through craft-pdf's `raster` with
hayro-interpret's plate mode (`InterpreterSettings::plate`). Every paint draws its tint of
that ink as gray film. A paint that overprints (`op`/`OP`, and `OPM 1` for zero DeviceCMYK
components) without naming the ink leaves the film as it is; with `Options::overprint` off,
every paint knocks out. Clipping, anti-aliasing, soft masks, transparency and knockout groups
stay the renderer's own. `Separation` and `DeviceN` colorants paint their own plates (`All`
every plate, `None` none), DeviceCMYK its components, DeviceGray black only.

Inks are found by a low-resolution pass that records each spot ink a paint uses, in the order
first painted. Spot inks used only inside soft masks are left out. Each spot gets a Lab
alternate measured from its sRGB appearance at 100%, so out-of-sRGB spots are clipped.

### Process color

RGB, CIE-based and ICCBased colors become CMYK through, in order:

1. the CMYK profile of the page's or the document's output intent (a `GTS_PDFX` intent first),
   when craft-cms can convert into it;
2. `Options::fallback`: craft-cms's Coated FOGRA39 profile (the default), a profile the caller
   gives, or the ISO 32000-2 §10.4.2.4 device formulas.

`Plates::conversion` says which was used. Conversions are relative colorimetric without black
point compensation. DeviceCMYK, and ICCBased CMYK in the printing condition's own profile,
separate as they are.

### Caps

| Constant | Value | Why |
|---|---|---|
| `MAX_SPOT_INKS` | 32 spot inks per page | A `DeviceN` space has at most 32 colorants, and each plate costs a render and a byte per pixel. Further spots are listed in `Plates::unseparated`. |
| `MAX_PLATE_PIXELS` | 4 MP per plate (2000 × 2000) | A page with many inks stays within a few hundred megabytes. The resolution is lowered to fit; `Plates::dpi` reports the one used. |

The renderer's side limit (`craft_pdf::raster::MAX_SIDE`) also lowers the resolution.

### Approximations

- Non-separable blend modes and groups that blend in RGB are approximated per ink.
- The graphics state's rendering intent is not read; transfer functions and halftones are not
  applied.

## Coverage and TAC

Every reading takes `enabled`, indexed by `InkId::index` like a Separations Preview's
toggles: inks past its end count, so `&[]` counts every ink. Values are percent of one ink at
100%, so TAC runs to 100 × the number of inks.

| Call | Gives |
|---|---|
| `coverage_at(x, y, enabled)` | `Coverage`: each ink's tint and the TAC of the enabled inks at a pixel |
| `pixel_at(u, v)` | The pixel under a point given as fractions of the page, for a pointer readout |
| `tac(enabled)` | TAC per pixel, row-major |
| `stats(enabled)` | `Stats`: per ink `InkStats { max, mean, area }`; the page's highest TAC, where, and its mean |
| `above(limit, enabled)` | `Above`: a mask of the pixels over `limit` and their `count`; `regions()` lists connected regions with bounding boxes, largest first (at most `MAX_REGIONS`); `paint(rgb, color)` marks them on a preview |

## Composite preview

`Plates::composite(enabled, &cms)` returns sRGB8 (three bytes per pixel, row-major) of the
enabled inks:

- the process plates convert through `cms` (a `CmsConversion` for the plates' printing
  condition, `Plates::conversion` names it), relative colorimetric with black point
  compensation, or through the bundled CMYK profile when `cms` can't convert;
- each enabled spot plate multiplies over them by the spot's appearance at its tint: a Lab
  alternate interpolated from paper white to the solid, or a CMYK alternate scaled by the tint.

Overprint is already in the plates (the renderer decides per plate what overprints and what
knocks out), so the preview shows it without rewriting any paint as a blend mode.

```rust
use craft_prepress::{CmsConversion, InkId};

let cms = CmsConversion::fogra39();
let mut enabled = vec![true; p.inks.len()];
enabled[InkId::process(craft_prepress::Process::Black).index()] = false; // hide Black
let mut rgb = p.composite(&enabled, &cms);
let over = p.above(300.0, &enabled);
over.paint(&mut rgb, [255, 0, 255]); // TAC warning
let worst = over.regions().first().copied();
```

## Ink manager

`InkSet` carries the job's ink settings next to its inks:

| Call | Effect |
|---|---|
| `alias(from, to)` | Spot `from` prints on `to`'s plate (a process ink or another spot, chains followed); `alias(from, from)` removes it. Loops and process `from` are refused. |
| `set_to_process(ink, on)` | Spot `ink` prints as process color, from its alternate (`spot_cmyk`). |
| `set_all_to_process(on)` | All Spots to Process; aliases still apply first. |
| `plate_of(ink)` | The plate a tint of `ink` lands on, `None` when it prints as process. |
| `resolve(&color, &cms)` | `color` with both applied. Tints landing on one plate combine as `1 − (1 − a)(1 − b)`; a converted color lists the four process inks first, and `InkColor::as_cmyk` gives the CMYK when no spot is left. |

`resolve` takes a `CmykConversion` (such as `CmsConversion`) because a spot with a Lab
alternate needs the printing condition to become CMYK.

Apply it in two places:

- **Export.** The writer calls `resolve` on every color it emits, then
  `to_pdf_color(&resolved, &inks, &cms)` for the krilla color to paint with (see
  [Export](#export)). An overprinting color that became CMYK is written with `OPM 1`, so its
  zero components leave the plates under it as they are.
- **Preview.** The preview separates that exported PDF with `plates` and shows
  `composite` and the coverage readings. Plates are the truth: an ink manager change shows
  after the next export, and the preview never recomputes overprint from the document.

## Export

`to_pdf_color(&color, &inks, &cms)` gives the krilla color (`craft_pdf::write::backend::color::Color`)
for an `InkColor`, choosing the color space from the inks it names:

| Color | Written as |
|---|---|
| Tints of process inks only | DeviceCMYK; unlisted process inks are 0 |
| One spot ink | `Separation` with the spot's alternate as DeviceCMYK fallback |
| Any other mix | `DeviceN`: process inks first in plate order, then spots in `InkSet` order |
| `All` | `Separation /All` (registration) |
| `None` | `Separation /None` (marks no plate) |

A spot's alternate is `spot_cmyk`: a CMYK alternate as given, a Lab one converted through
`cms` into the printing condition, so a spot has one alternate in every space that names it.
`None` when the color names an ink not in the set or a `DeviceN` space would need more than
32 colorants.

```rust
use craft_prepress::{CmsConversion, InkColor, to_pdf_color};

let cms = CmsConversion::fogra39();
let printed = inks.resolve(&InkColor::Tints(vec![(gold, 1.0), (black, 0.5)]), &cms);
if let Some(color) = to_pdf_color(&printed, &inks, &cms) {
    surface.set_fill(Some(Fill { paint: color.into(), ..Default::default() })); // DeviceN [Black Gold]
}
```

## In DesignCraft and VectorCraft

- **Separations Preview**: export the spread to PDF
  in memory, `plates(pdf, 0, dpi, &Options::default())` at the zoom's resolution, then
  `composite(enabled, &cms)` for the canvas. The panel lists `p.inks` with a toggle per row
  feeding `enabled`, each row's percentage from `coverage_at` under the pointer
  (`pixel_at`), and the TAC mode from `above(limit, enabled)` with `paint`. Single-plate view
  shows `p.plane(id)` as gray film. It would replace VectorCraft's `proof.rs`, which
  recolors the document and draws overprint as Darken.
- **Ink Manager** (DesignCraft's `InkManager` aliases and convert-to-process, VectorCraft's
  `inks.rs` plates): keep one `InkSet` per document, add each spot swatch with its Lab or
  CMYK alternate, and map the dialog's rows onto `alias`, `set_to_process` and
  `set_all_to_process`. The PDF writer resolves swatch colors through it; the Separations
  Preview then lists the plates the export produced.
- **Preflight**: `stats(&[])` gives per-ink max and mean coverage and the page's highest TAC
  and its position for a report line such as "TAC 340% at …, limit 300%".

## Credits

- Plate mode, ink discovery, the output-intent handling and the coverage readings and
  composite (`Separations::above`, `stats`, `composite`) come from
  [storytold/pdfcraft#2770](https://github.com/storytold/pdfcraft/pull/2770) by Guillem Hermida
  (guillehr2), with craft-cms in place of moxcms.
- Knockout groups in the renderer come from
  [storytold/pdfcraft#2974](https://github.com/storytold/pdfcraft/pull/2974) by mtlcl410.
- The flattener's planar regions (`flatten::geom`) are ported from VectorCraft at commit
  `c2b15982` by the VectorCraft contributors: `crates/pathops/src/pathfinder.rs`,
  `crates/pathops/src/boolean.rs` and `crates/engine/src/cmd/flatten.rs`.
- The flattener's compositor (`flatten::compose`) is ported from VectorCraft at commit
  `c2b15982` by the VectorCraft contributors: `Composer`, `region_fill` and the raster/vector
  balance in `crates/engine/src/cmd/flatten.rs`, and the blend formulas in
  `crates/color/src/blend.rs`.
- The flattener's raster step (`flatten::raster`) follows VectorCraft's `render_raster` in
  `crates/engine/src/cmd/flatten.rs` at commit `c2b15982`: the image reaching a pixel past its
  area, and the coverage of the art drawn opaque as the image's alpha.

All MIT OR Apache-2.0, as this crate.
