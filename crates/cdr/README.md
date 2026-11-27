# craft-cdr

A bounded Rust reader for `.cdr` drawings of versions 1 to 27 (and `.cdt` templates), and a
writer of version 15 drawings.
It reads the `WL` files of versions 1 and 2, the RIFF chunk tree of versions 3 and later with its
compressed lists, the ZIP packages that versions 14 and later save (with the data streams of
versions 16 and later) and the records listed below, and hands the engine a drawing in points,
with each object's transform. `write` writes a drawing back out as a `.cdr` file of version 15
(see [Writing](#writing)).

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

## Previews

`preview` returns the picture a `.cdr` file keeps of its drawing without reading the drawing (rows
R75 and R76). In versions 4 to 13 the picture is the `DISP` chunk, returned as a BMP file. In versions
14 to 16 it is the package member `metadata/thumbnails/thumbnail.bmp`, and from version 17 on it
is `previews/thumbnail.png`. The package's drawing stream is not read, so a package that holds
both members gives the PNG. Files of versions 1 and 3 keep no picture. A version 2 file keeps a
1-bit picture (R40) that `preview` does not read, so it refuses the file. `preview` does not check
the drawing's version, so a RIFF file that `read` refuses for its version, such as one that names a
version before 3, still gives its `DISP` picture.

A `DISP` bitmap must start with the 4 bytes `08 00 00 00`, have a 40-byte info header and be
uncompressed (compression 0, or 3 with color masks at 16 or 32 bits). A `DISP` chunk of another
kind, or a bitmap with another header length or compression, is passed over without checking the
rest of its bytes, and a file with no other `DISP` chunk is refused. A RIFF file without a `DISP` bitmap is refused as damaged when its last chunk does not end
where the form's length says the form ends, which includes a file cut short. The bytes are returned
still encoded, so the caller decodes them within its own image limits.

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

`write_cmx` writes a drawing as a `.cmx` file with 32-bit coordinates in the layouts `read_cmx`
reads, so that `read_cmx` leaves nothing out because of the `Limits` given: pages with their
sizes, layers with their names, groups, and curves with uniform fills and outlines. What the file
holds differently from the drawing is reported, as below. Rectangles and ellipses are
written as curves, and a clipping container as a group of its fill, the objects it clips
(unclipped) and its outline. A rounded rectangle scaled differently along its two sides gets round
corners, not stretched ones. Hidden layers, groups and shapes, guide and grid layers, guidelines
and master layers are left out; opacity, blend modes, fill rules and locked states are not
written, desktop layers are written as drawing layers, and a spot color with no name opens as a
process color; every use of a named spot color takes the look of its use at the strongest tint
among the uses whose look is known.

`write_cmx` returns an error, and writes nothing, for a coordinate, page side or outline width
past the 32-bit range, a negative or NaN outline width, a page side that rounds to 0, more than
65,535 pages, layers on a page or different colors, pens, line types, dash patterns or outlines, a
dash pattern of more than 32,765 lengths, groups nested deeper than the limits allow, more chunks,
objects, coordinates or dashes than a read within the limits takes, a file longer than their input
size, and a chunk past its 32-bit size. Otherwise the returned `Written` reports paths split or cut into several curves, dashes
rounded to whole outline widths, nonzero fills that may fill differently, names that open
shortened, spot colors at a tint too low to keep their look, with no name or with no known look
(written black), uses of one ink with different looks, and what was left out or written opaque,
unclipped or unlocked.

The header's index offsets, the index chunks (`LIST indx`), the instruction counts and offsets
inside page, layer and group records, and the bounds inside layer and group records are not
written, so a reader must find the chunks by their ids and walk the instructions in order, as
`read_cmx` does. The record of a page with a size holds the edges of its curves, or of the page
when it has none. `PROVENANCE.md`'s "Written
fields" table lists every field the `.cmx` writer sets.

## Writing

`write` writes a drawing as a `.cdr` file of version 15: a ZIP package whose first member,
`mimetype`, names the format and whose member `content/riffData.cdr` holds the RIFF file. Each
record is laid out as the reader reads it, by the `PROVENANCE.md` rows it cites. The reader reads
the written file back as the drawing it was written from, to the precision the file stores
(1/254000 inch for coordinates, 1/255 for color components), with the changes listed below the
table.

| In the drawing | Written as |
|---|---|
| Pages and their sizes; the drawing's page size | pages with a page size argument; `mcfg` |
| The master page's layers and the drawing's guidelines | the master page, with a guides, a desktop and a grid layer |
| Layers: name, kind, hidden and locked | layers with their flags and their kind as the object type |
| Guidelines of a layer | guideline objects in that layer |
| Groups, and clipping containers with the objects they clip | `LIST grp ` lists; containers naming groups of the document's `LIST clpt` |
| Paths, rectangles with each corner's radius, ellipses | curves, rectangles of the version 15 layout, ellipses |
| Uniform fills and outlines (width, color, caps, joins, dashes, behind the fill, scaled with the shape) | fill and outline records, one for each different fill or outline |
| RGB, CMYK, gray, registration and spot colors | color models 5, 3, 9 and 20; a spot color as its look with its tint and its name |
| The nonzero fill rule, opacity and blend modes | the winding argument and the uniform transparency argument |

Opacity is written as the reader reads argument 8000 (R47), and the fill rule as the reader reads
argument 14001 (R52). The uniform transparency of every public sample R47 cites is 50%, and the
specifications R52 cites do not say which fill rule each winding value means, so the direction of
the opacity value and the value written for nonzero rest on the reader's own reading. Each layer
gets an id (argument 40050), and every other page names the master page's guides, desktop and grid
layers by those ids (R70); the master page's drawing layers are not named from the other pages.

The drawing's version and warnings are not written. A group that holds no shape or container is
left out, a clipping container whose objects hold none is written as a plain shape, a subpath
without segments is left out, as the reader leaves it out, and a drawing without pages gets one
empty page. A closed subpath whose last segment ends away from its start gets a closing line to
its start, as the file closes subpaths with an explicit point. A drawing without a page size of
its own is written without `mcfg`. Guidelines on the master page's layers read back as the
drawing's guidelines. An empty layer of a page other than a drawing layer, and an empty layer of
the master page, are written and read back left out, as the reader leaves out such layers in any
file. A guideline on a hidden layer reads back hidden, and a guideline angle past the field's
range is written as the same angle within one turn. Hidden groups and objects carry the same hide
bit as hidden layers (R19); the specification's notes date that bit on groups and objects to a
later program release than version 15 files, so a program of that version may show them. A spot
color without a look is written with a black look. Dash lengths are written to whole outline
widths and spot color tints to whole percent, as the file stores them.

Writing refuses a drawing that the reader would not read whole within the same `Limits`
(objects, coordinates, dashes, arguments, chunks, clipped chunks, nesting depth, clip nesting,
members and sizes), and groups nested deeper than the nesting depth, groups that hold nothing
included. It also refuses what the file cannot hold or the reader would read back otherwise
changed: a coordinate past 32 bits of 1/254000 inch, a number that is not finite, a page width or
height that rounds to less than one coordinate unit, a negative outline width or a positive one
that rounds to 0, a corner radius below 0 or more than a billionth past half the rectangle's
shorter side (a radius within that is written as half), a color value, tint or opacity outside 0
to 1, more than 10 dash lengths, a negative one or one that rounds past 65,535, a pattern whose
lengths all round to 0, a name longer than 256 UTF-16 units or holding a NUL, a spot color without
a name, at a tint that rounds to 0 percent or at a tint too low for its look to read back within
one step of 1/255, objects on the master page's grid layer, and a guideline at the edge of the
coordinate range with no room for a second point on it whose rounded coordinates lie within 1
degree of its angle.

## Provenance

`PROVENANCE.md` lists every record layout the reader relies on, and the rows only the writers
need, with their public sources and status.
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
