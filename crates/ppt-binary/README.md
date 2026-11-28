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
- Password-protected files are refused (`PptError::Encrypted`). Formats older than PowerPoint 95
  aren't read: a compound file whose current user record has neither of [MS-PPT]'s tokens and
  that has no edit record is refused as too old (`PptError::TooOld`), and a file that isn't a
  compound file is `PptError::NotPpt`. No file from an earlier PowerPoint was tried.

## PowerPoint 95

PowerPoint 95 (7.0) files use the same compound file and many of the same record numbers, but a
16-byte record header and a single record tree with no edit chain. A file whose `PowerPoint
Document` stream starts with that tree is read on its own path; anything else takes the
PowerPoint 97 path. The tree's Document must hold its DocumentAtom, which counts the slides;
`import_checked` names the slides a cut-short file is missing ("slide 3"): those the count says it
should hold, and any cut off after its header, which is left out. What
was learned of the format came from files PowerPoint 95 itself wrote,
one feature per file, and the public [MS-PPT] numbering where the records keep it.

So far a PowerPoint 95 file imports with its slide size, its masters (each the caller's default
master wearing the file's colour scheme, and its background when that isn't the scheme's), and
its slides in order with their master, layout type and, when a slide doesn't follow the master's,
its own background: a solid colour, or none when the slide's background has no fill. A slide also
says whether it shows the master's objects and whether it uses the master's colour scheme or its
own. Slides hold
their placeholders (title, body, centred title, subtitle, and the date, footer and slide number)
and text boxes, each with its box and its text: the text's anchoring (top, middle or bottom, and
whether it's centred across) and word wrap, paragraphs with their level, alignment (justified
included), line spacing, space before and after (in lines or points), bullet character and the
indents of the text's ruler, and runs with their font, size, bold, italic, underline and colour
(an RGB or a colour scheme slot). The text is Windows-1252. The master's placeholders set where the
default master's placeholders go, and its date, footer and slide number those of every layout. The
master's text styles become its title, body and other styles, level by level. Shapes and lines
read too: all 24 of PowerPoint 95's shapes (rectangle, rounded rectangle, ellipse, diamond, the
two triangles, parallelogram, trapezoid, hexagon, octagon, plus, five-point star, the arrow and
the thick arrow in their four directions, home plate, cube, balloon and seal) as DrawingML presets
with their adjust values (the trapezoid, wide at the top in PowerPoint 95, as its own outline, so
its text stays upright), each with its fill (or none), its line (or none) with its colour, its
width (the six line styles, 1 to 10 pt) and its dots or dashes, and its shadow with the shadow's
colour, offset and transparency; and lines from their start to their end, with their arrowheads.
A shape code no probe showed reads as a rectangle. PowerPoint 95 measures angles
counter-clockwise, DrawingML clockwise, so each is negated. A shape's geometry holds its outline
as drawn, quarter-turns and mirror included, so its angle and flips come from there (a box turned
by an odd number of quarter-turns is turned back); the rectangle, rounded rectangle and ellipse
take the angle the user set. A shape that draws nothing but text is a text box; a shape's text is
read as a text box's is. PowerPoint 95 lays it out in the box the geometry holds, turned by the
text's own angle, so the text of a shape turned before it was typed stays upright: such text is
`upright`, at its own angle, with its insets on that box's edges (a balloon's measured from its
body). A preset with a text rectangle of its own, such as an arrow's shaft, applies the insets
inside that rectangle, where PowerPoint 95 measures them from the whole box, so such text can sit
off from where PowerPoint 95 puts it. Freeforms read as custom paths through their points, in
their box, with
their format's fill, line and shadow, and open ones with their arrowheads. Groups read as groups
of their members, which keep their places on the slide (PowerPoint 95 can't turn or flip a
group), nested up to 32 deep. Pictures read as picture shapes in their box, with their crop and
the line and fill PowerPoint 95 draws them with. PowerPoint 95 keeps every picture as a Windows
metafile, whatever was inserted; each becomes `image/x-wmf` media as stored, as the PowerPoint 97
path keeps its metafiles, read once however many shapes show it, and all of them within the
PowerPoint 97 path's 2 GiB budget. A picture whose picture can't be read is left out. Each slide's
notes page gives the slide its notes: the text of the page's notes placeholder, its paragraphs
and runs formatted as a slide's are.

The caller's `craft_pptx::Defaults` supplies the master each of the file's masters starts from:
its layouts and placeholders. The file's masters then place and style the placeholders that
master has. With `craft_pptx::Plain` (one blank layout, no placeholders), the slides still read
with their own boxes.

The reader was written for DeckCraft (storytold/deckcraft#1137); the history of
`crates/ppt-binary` names its authors. Its tests build `.ppt` files from records; no file made
by PowerPoint is used. One optional test reads real PowerPoint 95 files from the directory
`CRAFT_PPT95_SAMPLES` names, and does nothing when it is unset; no such file is in the repository.
