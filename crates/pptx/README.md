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

## Reading

`import(bytes, &defaults)` reads a `.pptx`, `.potx` or `.ppsx` into a `Presentation`, on
`craft-opc` for the package and `craft-drawingml` for the DrawingML. It is lenient: unknown parts
are skipped, a malformed part is logged and left out, and a slide the presentation lists but the
package lacks is left out and named by `import_checked`. Everything is bounded: entries, part and
package sizes, inflate work, XML depth and nodes, shapes per part, group nesting, chart points and
decoded chart bytes, kept chart parts, and add-in tag bytes (`MAX_TAG_BYTES`).

What the file leaves out comes from the caller's `Defaults`: the master for a file without a
readable one, and the theme of a master that names none. `Plain` has no application's look in it.
`import_theme` reads the theme of a `.thmx`, or of a presentation's first master. `sniff` tells a
package from other bytes.

## Writing

`export(&presentation, &defaults)` writes a package PowerPoint opens without repair; `export_as`
writes a `.potx` or `.ppsx` instead. Masters, layouts and slides with their shapes, pictures,
media, tables and table styles, charts (an unchanged chart's workbook and other parts written
back), SmartArt, 3-D models, ink (as freeform lines), objects kept as XML (with their parts),
notes and handout masters, sections, custom shows, show settings, drawing guides, embedded fonts,
tags, kept parts and document properties. A slide whose layout is missing goes on the first one;
a presentation with no master gets the caller's. `Defaults` also gives the text styles written
where a master or the presentation has none, and the program name for `docProps/app.xml`.

Transitions (`p:transition`, with the PowerPoint 2010 and later effects) and animations
(`p:timing`, flattened into `anim::Animation` on reading and built back into the timing tree on
writing, with media volume and playback settings) are read and written.

Speaker notes (`p:notes`, text with its formatting) and comments are read and written. Comments
go out twice, as PowerPoint writes them: the modern threaded part (`p188:`, with replies and the
resolved state) and the flattened legacy part with its authors; on reading, a slide's modern part
wins over its legacy copy.
