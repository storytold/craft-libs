# craft-cdr

A bounded Rust reader for `.cdr` drawings of versions 1 to 27 (and `.cdt` templates).
It reads the `WL` files of versions 1 and 2, the RIFF chunk tree of versions 3 and later with its
compressed lists, the ZIP packages that versions 14 and later save (with the data streams of
versions 16 and later) and the records listed below, and hands the engine a drawing in points,
with each object's transform. It does not write `.cdr` files.

## What is read

| In the `.cdr` file | Read as |
|---|---|
| ZIP packages: `content/riffData.cdr` (versions 14 and 15), `content/root.dat` with the data streams `content/dataFileList.dat` names (16 on) | the RIFF chunk tree; a data stream is inflated only when a chunk the reader reads, or a list it splits, points into it |
| Pages, the page size in `mcfg` or a page's own size | pages, laid out by the engine as artboards side by side |
| Layers (drawing, desktop, guides and grid layers), the master page's among them | layers by name and kind |
| The hidden and locked flags of layers, groups and objects | flags |
| Guidelines (object type 0x16, and the version 7 `guid` chunk) | guidelines at their point and angle |
| Groups (`LIST grp `) | groups |
| Clipping containers and the groups of `LIST clpt` they clip | the container with the objects it clips |
| Curves (point lists of moves, lines and cubic curves) and paths (object type 0x25, version 10 on) | paths |
| Rectangles (one corner radius, four from version 9) | rectangles with each corner's radius |
| Ellipses, arcs and pies | ellipses, and paths for arcs and pies |
| Polygons and stars (a base side copied for each side) | paths |
| Uniform fills: CMYK, CMY, RGB, gray, registration, library colors | colors, spot colors with their tints |
| The fill's winding flag | the nonzero fill rule, or even-odd |
| Outlines: width, caps, joins, dashes, the no-outline and behind-the-fill flags, and from version 13 a library color named in their property list | outlines |
| Uniform transparency (version 13 on) and its merge mode | the shape's opacity and blend mode |
| Styles (`LIST stlt`, versions 7 to 15) that objects name | the fill and outline of an object that has none of its own |

Text, bitmaps, linked groups (blends, contours, brush strokes), B-spline curves, fountain,
pattern, texture and PostScript fills, outline arrowheads, lenses, transparency other than uniform
transparency of version 13 and later, and merge modes without a VectorCraft blend mode are not
read; the drawing's
`warnings` name each kind it held. Drawings of versions after 27 are refused, and so are packages
that are encrypted, ZIP64 archives or split across files.

VectorCraft reads a uniform transparency's stored value as the transparency in percent (0 opaque,
100 clear), and a winding flag other than 0 as the nonzero fill rule. These are VectorCraft's own
readings: no public source states either direction.

## Versions 1 to 4

Public specifications cover only parts of these versions, such as the version 3 fill and page
settings. For the rest, the reader follows the layouts of public sample files that
`PROVENANCE.md` lists (rows R31 to R44), and its tests read the synthetic files that
`src/synth.rs` builds from those rows.

| Version | Read | Left out or refused |
|---|---|---|
| 1 and 2 (`WL` files) | the page size; rectangles with one corner radius, whole ellipses and curves, each with its matrix; uniform fills and outlines with their width and color, in CMYK percent and the inks black and white. The objects come in on one page and one layer, front to back | text and objects of other types, and ellipses whose two angles differ (arcs and pies), with the unsupported-objects warning; fills of other types, with the unsupported-fills warning; inks of a numbered library, which read as spot colors without a look; groups and layers, whose records are passed over; dashes, caps, joins and arrowheads, which the reader does not look for; the records after a drawn object of another type, with the unsupported-objects warning |
| 3 (RIFF, form type `CDR `) | the page size from the page's corners; pages, layers and groups; rectangles, ellipses, arcs and curves with the matrices of the object and of the groups around it; uniform fills; outlines with their width, color, caps, joins and dashes | text and bitmaps; envelopes, with the envelopes warning (the objects and groups under one keep their outlines and matrices); fountain, pattern and texture fills; arrowheads |
| 4 (RIFF, form type `CDR4`) | as version 5, with the colors and the fixed-point transforms of version 4 | as version 5 |

A file is taken for version 1 or 2 by its content only when, after `WL`, its marker byte is one of
`e`, `l` and `m`, the next byte is zero, and the offsets of its header fit the file. The reader
reports version 1 for the marker `e` and version 2 for the other two. A RIFF file that names
a version before 3 is refused, and so is the version 4 template form type `CDST`, which reads as
not a `.cdr` drawing.

## `.cmx` files

`read_cmx` reads `.cmx` exchange files with 32-bit coordinates (rows R53 to R63) into the same
drawing model, with a version of 0. It reads the color, pen, line type, dash, outline and
arrowhead tables, then each page's instructions in drawing order:

| In the `.cmx` file | Read as |
|---|---|
| Pages, with the size the page gives, or else the size of the drawing's bounds that the page stores | pages; a page sized to the bounds is centered on them |
| Layers, by their UTF-16 or 8-bit name | drawing layers, unnamed when the file gives no name; a layer that holds nothing is left out |
| Groups | groups |
| Curves (moves, lines and cubic curves, as `.cdr` point lists) | paths |
| Uniform fills: CMYK in percent or 0 to 255, CMY, RGB, gray, and library colors with their names | colors, spot colors |
| Outlines: width, color, caps, joins, dashes, the no-outline, behind-the-fill and scaled-with flags | outlines |

Text, images, rectangles, ellipses, clipping paths, fills other than uniform ones, outline
arrowheads and lenses are not read; the drawing's `warnings` name each kind it held. A clipping
path is left out and the objects it clips are read without it. Files in big-endian byte order and
files with 16-bit coordinates are refused.

## Provenance

`PROVENANCE.md` lists every record layout the reader relies on, with its public source and status.
Code cites the rows by id (`R1` ...), and `tests/it/provenance.rs` fails when a cited id has no row.
The test inputs are synthetic drawings built by `src/synth.rs` and `src/synth_cmx.rs` (the
`synth` feature); no third-party file is committed.

## Limits

`Limits` bounds the input size, the bytes inflated from compressed lists and package members, the
size of one package member, the number of members, the nesting depth, the chunk count, the
objects, the coordinates and the outline dashes of one read. Past a limit the reader stops with an
error or leaves the rest out with a warning; it does not recurse and does not panic. Each record
of a version 1 or 2 file counts toward the chunk count, and the transform entries of a version 3
object are held to the arguments one record may have. A package's
data stream is read once however many lines of `content/dataFileList.dat` name it. A redirected
list whose bytes overlap those of an earlier redirected list is left out with the damage warning.
Other records may share bytes, and the bytes read again may add up to the member limit; past it
the records are left out with the limit warning.
