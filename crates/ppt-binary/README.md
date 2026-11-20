# craft-ppt-binary

Reader for the PowerPoint 97-2003 binary file format (`.ppt`, `.pot` and `.pps`) into the
`craft-pptx` presentation model. It is DeckCraft's reader, on `craft-cfb` for the compound file.
Import only: there is no `.ppt` writer.

| Document | Used for |
|---|---|
| [MS-PPT] PowerPoint (.ppt) Binary File Format (Microsoft Open Specifications) | the Current User stream, the edit chain and persist directories, documents, masters, slides, notes, text and its styles, headers and footers |
| [MS-ODRAW] Office Drawing Binary File Formats | shapes, their property tables and geometry, the BLIP store and its pictures |
| [MS-CFB] Compound File Binary File Format | through `craft-cfb` |

```rust
// Not a compound file: an error, never a panic.
assert!(!craft_ppt_binary::sniff(b"not a .ppt"));
assert!(craft_ppt_binary::import(b"not a .ppt", &craft_pptx::Plain).is_err());
```

- `sniff(bytes)`, `import(bytes, &defaults)` and `import_checked(bytes, &defaults)`, which also
  names the slides the file lists but whose objects are gone.
- Main masters become masters, with their colour scheme, fonts, text styles, background and
  decorative shapes. Title masters become layouts of their main master.
- Slides keep their shapes (preset and freeform geometry, groups, rotation and flips), fills,
  lines, shadows, pictures (PNG, JPEG, DIB, TIFF, EMF and WMF), tables (as groups of shapes, the
  way the file stores them), and text with the master's styles resolved into it, along with
  fields, numbering, headers and footers, notes, and hidden slides.
- Damaged files read as far as they can: a broken edit chain falls back to a scan of the edits,
  and unknown or damaged records are skipped. Sizes, counts, nesting and picture decoding are
  bounded.
- Password-protected files are refused (`PptError::Encrypted`), as are PowerPoint 4.0 and earlier
  files (`PptError::TooOld`).

## PowerPoint 95

PowerPoint 95 (7.0) files use the same compound file and many of the same record numbers, but a
16-byte record header and a single record tree with no edit chain. A file whose `PowerPoint
Document` stream starts with that tree is read on its own path; anything else takes the
PowerPoint 97 path. What was learned of the format came from files PowerPoint 95 itself wrote,
one feature per file, and the public [MS-PPT] numbering where the records keep it.

So far a PowerPoint 95 file imports with its slide size, its masters (each the caller's default
master wearing the file's colour scheme, and its background when that isn't the scheme's), and
its slides in order with their master, layout type and, when a slide doesn't follow the master's,
its own background: a solid colour, or none when the slide's background has no fill. Slides hold
their placeholders (title, body, centred title, subtitle, and the date, footer and slide number)
and text boxes, each with its box and its text: paragraphs with their level, alignment, bullet
character and the indents of the text's ruler, and runs with their font, size, bold, italic,
underline and colour (an RGB or a colour scheme slot). The text is Windows-1252. The master's
placeholders set where the default master's placeholders go, and its date, footer and slide number
those of every layout. Shapes and lines read too: PowerPoint 95's twelve shapes (rectangle,
rounded rectangle, ellipse, diamond, the two triangles, parallelogram, trapezoid, hexagon,
octagon, plus and five-point star) as DrawingML presets with their adjust values, each with its
fill and line colour (or none), its shadow, its angle and a horizontal flip, and lines from their
start to their end. A shape that draws nothing but text is a text box. Line widths and styles,
vertical flips and shadow offsets are not read yet (lines draw 1 pt wide, as PowerPoint 95 drew
them, and shadows fall 6 pt right and down); groups and freeforms are skipped. Pictures, notes and the master's text styles come later.

The caller's `craft_pptx::Defaults` supplies the master each of the file's masters starts from:
its layouts and placeholders. The file's masters then place and style the placeholders that
master has. With `craft_pptx::Plain` (one blank layout, no placeholders), the slides still read
with their own boxes.

The reader was written for DeckCraft (storytold/deckcraft#1137); the history of
`crates/ppt-binary` names its authors. Its tests build `.ppt` files from records; no file made
by PowerPoint is used. One optional test reads real PowerPoint 95 files from the directory
`CRAFT_PPT95_SAMPLES` names, and does nothing when it is unset; no such file is in the repository.
