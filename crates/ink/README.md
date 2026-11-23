# craft-ink

Neutral ink for the drawing apps: a stroke and brush model, a bounded reader and writer for the
Office InkML part behind a drawing, lasso selection, replay ordering and scheduling, a geometric
ink-to-shape recogniser, and the handwriting-recogniser seam. No application code, no UI toolkit,
no inference runtime.

The document stays in the application. A host maps its own shapes, anchors, sheets and slides onto
`Stroke`; the package wiring around an ink part — the `mc:AlternateContent` choice, the
`xdr:contentPart`, its `customXml` relationship, the `[Content_Types].xml` declaration, the
`a16:creationId` extension and the mandatory PNG fallback picture — is the host's too. This crate
reads and writes the ink part itself.

```rust
use craft_ink::stroke::{Brush, Color, Stroke, StrokePoint, Tool};
use craft_ink::{Ink, Limits, inkml};

let stroke = Stroke::new(
    vec![StrokePoint::new(0.0, 0.0), StrokePoint::new(2.5, 1.0)],
    Brush::new(Tool::Pen),
    Color::from_hex("#1F5FC9").unwrap(),
);
let part = inkml::write(&Ink { strokes: vec![stroke], timestamp: None }, &Limits::default())?;
let back = inkml::read(part.as_bytes(), &Limits::default())?;
assert_eq!(back.strokes.len(), 1);
# Ok::<(), craft_ink::InkmlError>(())
```

## Modules

- `stroke` — points, pressure, brush kind (`Tool`), texture (`Effect`), colour, width and height
  in centimetres, draw order and a canonical `a16:creationId`-style key. Coordinates are
  centimetres at the format's 1/1000 cm resolution; `px_to_cm` and `cm_to_px` convert a host's
  96 dpi sheet points.
- `inkml` — the codec. Writes `<inkml:ink>` → `<inkml:definitions>` (context with
  `inkSource`/`traceFormat`/`channelProperties`, brush with `brushProperty`) and one
  `<inkml:trace>` per stroke, using the W3C difference-order grammar. Two private brush
  properties carry what Office cannot express: `craftTool` and `craftOrder`; the names GridCraft
  wrote before this crate (`gridcraftTool`, `gridcraftOrder`) are read as aliases. Unknown brush
  properties are preserved; unknown elements are refused rather than guessed.
- `select` — lasso containment: even–odd rule, at most 64 samples spread along a stroke, and at
  least 60 % of them inside. The loop is bounded and thinned.
- `replay` — the order strokes are replayed in (recorded `order`, else document order, stable)
  and a schedule that reveals a stroke along its path from a caller-owned clock.
- `shape` — a line, rectangle/square, ellipse/circle, triangle or polygon of up to eight
  corners, or an arrow drawn shaft-and-head in one stroke; everything else stays ink.
- `recognize` — the `Recognizer` trait, a fixed-answer double, a bounded `LineImage`, and a model
  descriptor with provenance, sizes and SHA-256 hashes. Verifying and running a model belongs to
  a runtime crate the host installs.

## Specifications

- W3C, *Ink Markup Language (InkML) 1.0 Recommendation*, 20 September 2011,
  <https://www.w3.org/TR/InkML/> — trace format, channels, brushes and the difference encoding.
- Microsoft, *[MS-ODRAWXML]: Office Drawing Extensions to Office Open XML Structure*,
  <https://learn.microsoft.com/openspecs/office_standards/ms-odrawxml/> — the `xdr:contentPart`,
  its `customXml` relationship and the `a16:creationId` extension the host owns.

## Bounds

`inkml::Limits` (16 MiB, 1,024 strokes, 20,000 points, 8 levels of `traceGroup`) is checked
before anything is trusted; strokes are capped at `MAX_POINTS` points and `MAX_COORD_CM`
centimetres, pressure at `MAX_PRESSURE`, brush sizes at 0.001–100 cm. XML is read by `craft-xml`,
which does not process a DTD or expand entities and bounds nesting, nodes and attributes.
Non-finite numbers are refused, arithmetic is checked or saturating, and no function panics on
hostile input.

## Provenance

Moved from the open app PRs that developed it, all MIT OR Apache-2.0: GridCraft's InkML workbook
part, pen gallery, lasso selection, replay and ink-to-shape work, and its Action Pen recogniser
seam — the runtime (`ink-rten`) stays in the application. Ink to shape originates in WordCraft's
`crates/engine/src/cmd/ink_shape.rs`. The document model, command layer, drawing and package
container stay in each application.
