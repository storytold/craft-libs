# craft-affinity

An independent, bounded Rust reader for native Affinity documents: `.af` from Affinity 3 and
`.afdesign`, `.afphoto` and `.afpub` from Affinity 1 and 2 (container versions 8 to 12). It reads
the archive, the tagged object stream in `doc.dat` and the part of the document model listed below,
and returns a neutral `Document` in pixels with every transform composed. Mapping that document
onto an application's own model stays in the application. It does not write Affinity files.

```rust
match craft_affinity::read(bytes, craft_affinity::Limits::default()) {
    Ok(document) => { /* spreads, nodes, warnings */ }
    // The flattened PNG preview every document embeds, when the native document can't be read.
    Err(reason) => { let preview = craft_affinity::preview(bytes); }
}
```

`is_affinity` recognises the container by its signature. `Document::warnings` lists what was
left out or approximated, one line per kind with a count.

## What is read

| Affinity | Model |
|---|---|
| Pages, Publisher spreads (one or two pages) | `Document::spreads`, each with its page rectangles |
| Master pages (`MpCh`) shown on pages through master page instances (`MPIN`: master, page offset, master page, count) | `Document::masters` and `Kind::MasterInstance` nodes |
| Artboards (legacy `ABEn` or Affinity 3 `phrp`/`aprp`), with name, paint and any transform | `Kind::Artboard`: export rectangle, the actual clipping outline, background and strokes |
| Layers (`Scop`), groups (`Grup`), pass-through or isolated | `Kind::Layer`, `Kind::Group`, `Blend::PassThrough` |
| Curves (`PCrv`): cubic subpaths, closed flags, live corners (`CnrD`) | `Kind::Shape` paths (multi-subpath curves fill even-odd) |
| Rectangles with corner radii (relative or absolute, locked or per corner) and round or straight corners, ellipses, polygons (smooth too), stars, square stars, pies, triangles, trapezoids, diamonds | `Kind::Shape` paths |
| Clouds, hearts, cogs (with their hole), rectangular and elliptical callouts, arrows, double stars, tears, crescents and segments | `Kind::Shape` paths drawn from their parameters (approximate outlines, not Affinity's exact curves) |
| Compound shapes (`Comp`, add and subtract) | one even-odd path |
| A shape, curve or text with children | the node keeps its children: it clips them |
| Vector masks (`AdCh` curves and shapes) | `Node::mask` |
| Pixel masks (`MRst`, attached or as a layer in a group) | `Node::pixel_mask` |
| Solid fills: RGBA, HSLA, CMYK, grey, Lab (D50) | `Paint::Solid` in the colour's own model |
| Linear, elliptical and radial gradients with Affinity's midpoint bias | `Paint::Gradient` (each stop's bias is the midpoint where the blend is half way) |
| Strokes: weight, scale with object, dash pattern and phase, alignment, miter limit, behind the fill; a stroke whose style is None (every shape stores one, black and 1 px) is left out | `paint::Stroke` |
| A stroke's pressure curve (`LDeP`: position along the path against width) | `Stroke::profile`, the curve sampled at 2 to 8 points per segment (256 segments at most), widths up to the full weight |
| Fill layers (`FRst`) | `Kind::Fill` |
| Artistic and frame text: Unicode scalar-indexed runs, stored fallback font, family/weight/style, boolean OpenType features, all caps, underline, strikethrough, size, tracking, horizontal scale, fixed leading, colour, outlined characters, first baseline; each paragraph's alignment, indents, space before and after, fixed and automatic leading; frame text sized by its text scale (`FTxS`), not its frame's transform | `Kind::Text`: runs with their character and paragraph formats, anchor, frame and the transforms of the node and of its type |
| Tables (`TxtT`): grid, cell text, merged cells, cell insets and vertical alignment, the line on every cell edge | `Kind::Table` |
| Placed images (`ImgN`): the original JPEG/PNG embedded in the file | `Pixels::Encoded` |
| Pixel layers (`Rstr`): RGBA 8/16-bit and CMYK 8-bit tiles, cropped to their content; source-backed RGBA8 JPEG/PNG tiles with matching dimensions and zero origin; the document's CMYK colour profile (`CMYP`) | `Pixels::Rgba8`; CMYK layers as `Pixels::Cmyka8` (inks kept) with `Document::cmyk_profile` to show them through, or `Pixels::cmyka8_to_rgba8` without colour management |
| Embedded Affinity documents (`EmbN` with an `EmDc` entry), four deep at most | their first page's content as the node's children (their artboards only clip; their master pages are left out); other embedded documents and symbols as the picture Affinity cached of them |
| Layer effects: drop shadows, outer glows, inner shadows; outlines; bevels; colour and gradient overlays in any blend mode | `Node::effects`; `overlay::apply` bakes colour and gradient overlays into fill, stroke, text colours and pixels for a caller without such an effect |
| Opacity, visibility, lock, names, blend modes | the same on every `Node` |

Everything else is reported in the warnings, never dropped silently: blur and other layer
effects, adjustment layers and live filters, brush strokes, pressure curves that can't be read or
that go past the million points one document may hold, transparency gradients, fill opacity,
bitmap fills, master pages based on other master pages, conical gradients (as radial), parametric
shapes of an unknown class (as their bounding ellipse), corner types other than round or straight,
stars with rounded points, several fills or strokes on one object (the active one is used), skewed
or flipped text, characters of different transparency in one text, text fields such as page
numbers, text threading, frames with columns or a curved outline, table cell fills, unsupported
font selectors, grey/Lab/32-bit pixels and blend modes outside `Blend`.

## Where it came from

The reader was written in VectorCraft and PhotoCraft from one common start (AF.1 to AF.3 by
callmeBalloch, 2026-10) and then grew apart in both. This crate is VectorCraft's copy
(`crates/affinity` at `6e873b80`: text and paragraph formats, tables, master pages, effects and
overlays, embedded documents, CMYK and large pixel layers, pressure curves, one budget for the
whole read) with what only PhotoCraft's copy had (`crates/affinity` at `63b641b5`): the special
shapes, null array slots charged before a list is scanned, colour records decoded only as far as
they are read, the table-based 16-bit plane conversion, and a caller-set pixel allowance
(`Limits::max_pixel_bytes`).

## Provenance and clean-room

Affinity has no published file-format specification, so this reader is built only from a public
description of the format and from public files, never from Affinity itself.

* **No Affinity software.** Affinity was never downloaded, installed, run, scripted, screenshotted or
  disassembled for this work, and nothing from an Affinity installation (program code, resources,
  presets, fonts, colour profiles) was read. No file was made with Affinity for it: every sample
  is a document its author published.
* **A public, permissively licensed description.** The container and object-stream layout was
  learned from [VMDevCpp/afread](https://github.com/VMDevCpp/afread) (MIT, at
  `04b672334a43e3e37ded6b5ffc57af231d589774`, written for container versions 7–11) and re-described
  in our own words before this Rust code was written; no code was translated. No GPL/AGPL code (such
  as Inkscape's Affinity extension) was read.
* **Every structure checked against public files.** afread doesn't say how its author learned the
  format, so nothing here rests on it alone: each structure the reader relies on is confirmed by
  the public documents. Every archive entry carries a CRC-32 that must match, and every field in
  the object stream names its own type, so a misread layout fails loudly instead of producing
  plausible wrong data.
* **Meaning fitted to Affinity's own pictures.** What shapes, paints, text and pixel data mean was
  worked out by comparing our render with the thumbnail every Affinity document embeds (Affinity's
  render of itself), on 176 public documents saved by Affinity 1.x, 2.x and 3.0/3.1 on Windows,
  macOS and iPad. The collection has since grown to 189 distinct documents, and the reader opens
  all of them. Most are published under CC0, MIT, BSD, Apache-2.0 or CC BY (-SA); the others
  (no licence stated, or GPL or non-commercial terms) were only opened locally to compare
  pictures, never committed or redistributed. The corpus test keeps the comparison for the pinned
  files, which are CC0, MIT or Apache-2.0 only.
* **Nothing of Affinity's is in this repository**: no program code, assets or documents. The test
  files are fetched at pinned commits and sha256-verified, never committed.
* **Read-only.** Nothing is written in Affinity's format; export waits until someone can check
  written files in Affinity itself.

Tables, paragraph formats, text scale, master pages, layer effects, overlays, embedded documents,
large pixel layers and HEIC photos were added in DesignCraft
([storytold/designcraft#291](https://github.com/storytold/designcraft/pull/291)) and brought into
VectorCraft's copy, from the same public fixtures and the thumbnails of documents their owners
made, used only locally. Automatic leading (the font size plus the font's line gap, for artistic and frame text)
was fitted on the thumbnails of one such local document with 56 artboards and on the public Patchy
text-frame fixture.


## Validation

* Unit tests on synthetic containers built by the `synth` feature (stored, zlib and zstd entries,
  checksums, budgets, cycles, hostile streams, every truncation) and property tests of random
  mutations. `tests/it/import_limits.rs` and `tests/it/outline_limits.rs` build hostile documents
  against the limits below: grids of 16 million cells, grids and stories shared by many layers,
  200 000 paragraphs, a document embedded hundreds of times, compounds that fan out or contain
  themselves, lists of a million null slots linked many times.
* `scripts/fetch-affinity-corpus.sh` fetches 51 public CC0/MIT/Apache-2.0 documents at pinned
  commits, SHA-256 verified against `tests/support/corpus.tsv` (which names each file's source and
  licence), with the upstream licence notices. Of these, 33 are current `.af`: four CC0 vector
  illustrations and 28 original Affinity 3.2.3 feature probes plus one separately identified
  derived regression input from [SethRobinson/Patchy](https://github.com/SethRobinson/Patchy)
  (MIT). Only document data and provenance notices were inspected, never that project's importer
  or authoring scripts. The files are never committed.

  ```sh
  scripts/fetch-affinity-corpus.sh
  AFFINITY_CORPUS=target/fixtures/affinity cargo test -p craft-affinity --all-features
  ```

  Without `AFFINITY_CORPUS` the corpus tests are skipped. `AFFINITY_SAMPLES` names any other
  folder of documents to parse.
* `tests/it/real_files.rs` parses every file and freezes the modelled features of all 33 current
  `.af` files (`tests/support/affinity3-signatures.txt`: counts, fonts, artboards, warnings and a
  CRC of the whole model). Native raster tests compare decoded pixels with independent embedded
  images; text tests assert Unicode run boundaries, fallback fonts and feature retention.
* `fuzz/` has `cargo-fuzz` targets for the whole reader (`container`) and the object stream
  (`stream`):

  ```sh
  cd crates/affinity
  cargo +nightly fuzz run stream -- -max_total_time=600
  ```

What has not been verified: files from Affinity builds or platforms outside the public set, files
written by other applications, rotated or skewed images against Affinity's render, mask polarity
against a render with and without the mask, the special shapes against Affinity's own outlines,
and anything listed as a warning above. The `synth` builders write only what this reader accepts;
Affinity has never opened their output, so they are test fixtures, not an exporter.

## Safety limits

Every size is checked before it is allocated: 256 MiB per archive entry and 1 GiB per import by
default (`Limits`), a 64 MiB zstd window, 4096 saved revisions, array lengths no longer than the
bytes left, 16 777 216 decoded values per document stream (fields and array elements together, counted before
anything is allocated: a value takes about 40 bytes in memory however few it took in the file; the largest of
those 189 public documents uses 3.3 million), 384 levels of object nesting, 128 levels of layers, 500 000 layers, four million curve
nodes per curve; compound outlines also cap recursion at 128 levels, operand visits at 100 000
and cumulative geometry work at four million, rejecting cycles and partial results on limits.
Pie sweeps normalize finite angles in constant time. Pixel layers are decoded at no more than 64
megapixels each (cropped to their content first; larger ones are read at a half, a quarter… of
their resolution) and `Limits::max_pixel_bytes` for the whole document (2 GiB as RGBA, 512
megapixels, by default); pixel masks are read only over what
they mask (all of it under artistic text, whose glyphs only the layout places). One budget covers
the whole read: layers, pixels, 16 million characters of stories and 262 144 table cells, so
content that many layers link, or a document embedded many times, can't multiply them. Embedded
documents nest four deep at most; both of their readings are charged to that budget and to the
importing document's archive limits, also when they fail part of the way. Tables have at most
4096 rows and columns and 65 536 cells, and 64 layer effects per node are read. Gradients are
limited to 1024 stops. Text collection is bounded to a million characters per node before
allocation, with a cumulative 16 MiB budget for owned font names (256 bytes at most each),
features and gradient stops; paragraph formats are matched to runs by binary search. Exhaustion
warns and uses default attributes for remaining text, preserving its characters. Colour and
gradient overlays share one work budget per import (`overlay::MAX_WORK`: the nodes and pixels
each is baked into); past it they are left out, and said. Null slots of object lists are
charged to the document's work before a list is scanned, on every visit of a shared object.
Every archive entry's CRC-32 and size must match. Malformed input returns an `Error`; the
crate has no panics outside tests.


## Privacy

Affinity documents can hold the folder they were saved in, user names, original image paths and
XMP metadata. This reader never imports those fields.
