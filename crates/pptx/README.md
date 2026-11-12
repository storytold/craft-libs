# craft-pptx

PresentationML (ECMA-376 part 1, chapter 19): what a `.pptx` presentation holds, as plain Rust
types with serde. The DrawingML inside it (fills, lines, text bodies, tables, charts, themes)
comes from `craft-drawingml`.

| Item | What | Markup |
|---|---|---|
| `Presentation` | slide size, masters, slides, sections, custom shows, show settings, header and footer, properties, media, embedded fonts, table styles, tags, kept parts, drawing guides | `p:presentation`, `docProps/*`, `p:viewPr` |
| `Master`, `Layout` | theme, shapes, background, title/body/other text styles, colour map | `p:sldMaster`, `p:sldLayout` |
| `Slide` | shapes, background, transition, animations, notes, comments, colour map override | `p:sld`, `p:notes`, comments parts |
| `Shape`, `ShapeKind` | shapes, pictures, groups, connectors, tables, charts, media, ink, 3-D models, and objects kept as XML | `p:sp`, `p:pic`, `p:grpSp`, `p:cxnSp`, `p:graphicFrame`, `p:contentPart` |
| `Placeholder`, `PhType`, `placeholder` | placeholders and the rule that matches a slide placeholder to its layout and master parents | `p:ph` |
| `anim` | transitions and the flat list of animations the timing tree stands for, with the effect catalogues | `p:transition`, `p:timing` |

Ids (`ShapeId`, `LayoutId`, `MasterId`, and `MediaId`, `SlideId` from `craft-drawingml`) are
unique across a presentation; `Presentation::alloc_id` hands out fresh ones and refuses once they
run out. `Presentation::default()` is an empty 16:9 deck with no masters: an application adds its
own starting masters and layouts.

The reader and writer come in later changes.
