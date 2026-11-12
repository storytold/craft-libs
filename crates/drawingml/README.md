# craft-drawingml

The parts of DrawingML (ECMA-376 part 1, chapter 20) that presentations, word-processing
documents and spreadsheets share. A `.pptx` slide, a `.docx` drawing and an `.xlsx` drawing all
hold the same `a:` elements; this crate is the one place that knows what they mean.

Nothing here opens a package. Readers take a `craft_xml::El` and writers a `craft_xml::Writer`,
so the caller decides which part the markup comes from. Lengths are points (1/72 inch) as `f64`;
files store EMU (12 700 per point).

| Module | What | Markup |
|---|---|---|
| crate root | `emu_to_pt`, `pt_to_emu`, `Xfrm` (offset, extent, rotation, flips, as an affine), `group_child_affine` | `a:xfrm`, `a:chOff`, `a:chExt` |
| `color` | `Rgba`, `SchemeSlot` and `ColorScheme` (the twelve theme colours and their `bg1`/`tx1` aliases), `ColorTransform` with `apply`, the preset and system colour names | `a:srgbClr`, `a:schemeClr`, `a:prstClr`, `a:sysClr`, `a:lumMod`, `a:tint`, `a:alpha` … |
| `preset` | `build(name, w, h, adjusts)`: the outline of a preset shape as fill and stroke sub-paths, its text rectangle, adjust handles and connection sites; `CATALOG` with labels, categories and default adjusts | `a:prstGeom` |
| `custgeom` | `read_cust_geom` / `write_cust_geom` between `a:custGeom` and `CustomPath` (an `M L C Q Z` path string per sub-path), and `Guides`, the shape guide formula evaluator | `a:custGeom`, `a:gd` |
| `warp` | `warp(name, adjusts, rect)`: the bands and guide curves of a preset text warp, and the mapping of laid-out lines onto them | `a:prstTxWarp` |

## The model

These modules hold types only; their readers and writers are not here yet.

| Module | Types | Markup they stand for |
|---|---|---|
| `style` | `ColorRef` (a base colour plus transforms, resolved against a `ColorScheme`), `Fill`, `Gradient`, `PictureFill`, `Line`, `Effects`, `ShapeStyle` | `a:solidFill`, `a:gradFill`, `a:blipFill`, `a:pattFill`, `a:ln`, `a:effectLst`, `p:style` |
| `text` | `TextBody`, `Paragraph`, `ParaProps`, `Run`, `RunProps`, `ListStyle`, hyperlinks and actions | `a:txBody`, `a:bodyPr`, `a:lstStyle`, `a:p`, `a:r` |
| `table` | `Table`, `Row`, `Cell`, `TableStyleDef` and the built-in table styles | `a:tbl`, `a:tblStyleLst` |
| `chart` | `Chart`, `Series`, axes, data labels, trendlines and error bars, with the values they compute | `c:chartSpace`, `cx:chartSpace` |
| `numfmt` | `format(code, value, lang)`: number format codes as charts and cells use them | `c:numFmt` |
| `diagram` | `Diagram`, `DiagramNode` | `dgm:dataModel` |

A picture or sound is a `MediaId`; whoever owns the package keeps the bytes. Themes are not here yet.

## Preset shapes and text warps

Preset names are the file's (`roundRect`, `rightArrow`, `textArchUp`) so files round-trip. The
outlines are constructed in Rust from what each shape looks like and from the meaning of its
adjust values; no geometry table is copied. Adjust values use the file convention: ratios in
1/100 000 of the shorter side (or of the width or height), angles in 1/60 000 degree.

```rust
let g = craft_drawingml::preset::build("roundRect", 200.0, 100.0, &[25_000.0]).unwrap();
assert!(!g.paths.is_empty());
assert!(g.text_rect.width() < 200.0);
```

## Hostile input

`read_cust_geom` reads at most 1 000 paths per geometry, 100 000 commands per path and 5 000
guides per list, keeps at most 10 000 guide values, and evaluates the guide lists once per
coordinate space, for at most 16 spaces and 4 MiB of guide names. Every evaluated value is finite and within
±1e15; a division by zero is 0. An arc that sweeps a full turn or more draws one turn. `preset::build` and
`warp::warp` take any `f64` and return `None` or a finite outline. Each limit has a test.

The types derive `serde` traits with the field names the apps' own document files already use.
