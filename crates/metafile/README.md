# craft-metafile

Windows metafiles for the Craft apps: one reader for WMF, EMF and EMF+, and writers for EMF and WMF.

- `parse(bytes) -> Picture` plays a file's records on a model of the GDI device context and
  returns what was drawn: paths, pattern fills, bitmaps and text, each with its clip.
- `size(bytes)` gives the picture's size from the header alone when the header decides it.
- `sniff(bytes) -> Option<Kind>` and `is_metafile(bytes)` recognise the formats.
- `Picture::fit` and `Picture::place` fit a picture (with crop fractions) into a rectangle for
  callers that draw it; `crop_window` is the crop rule they share with bitmaps (a negative crop
  side pads the picture).
- `write::emf` and `write::wmf` (feature `write`) write a list of drawing operations.

The crate depends on `kurbo`, `thiserror`, `encoding_rs` (the code pages of 8-bit text) and `image`
(PNG, JPEG, GIF, TIFF and BMP decoders, for the image files that EMF+ and `BI_JPEG`/`BI_PNG`
bitmaps carry). It knows no fonts, no file system
and no application.

## The model

A `Picture` has a size (points when the file states one: the EMF frame, the placeable WMF box;
device or logical units otherwise) and `items` in paint order, in `0..width` by `0..height`, y down:

| Item | What it holds |
|---|---|
| `Item::Path` | Segments (`Seg`: move, line, cubic, close), a fill colour, a stroke colour and width (0 is a hairline), the stroke's `LineStyle` (cap, join, miter limit, dashes), the fill rule. |
| `Item::Pattern` | A path filled with a repeating RGBA `Tile`: hatch and bitmap brushes, EMF+ gradients and textures. Fills with the same brush share one tile. |
| `Item::Bitmap` | RGBA8 pixels (straight alpha, rows top-down), the rectangle they fill and a `transform` that also holds a turn or a skew (then the rectangle is the box around it). |
| `Item::Text` | The string, the font the file asks for (family, height, weight, italic, underline, strike, character set), per-character advances, colours, alignment and a transform from text space. |

Every item carries the clip it was drawn under: `region` (the GDI clip region as rectangles) and
`clips` (clip paths from `SELECTCLIPPATH`, outermost first; items under the same clip share one
`Arc<ClipPath>`). An item shows inside its region and inside each of its clip paths.

`Picture::skipped_records` counts records of a type the reader does not play, so a caller can say
that part of a file was not read.

### Why this model

The three apps had two output models. WordCraft's and DeckCraft's readers produced a picture of
paths, bitmaps and text to draw. VectorCraft's reader built VectorCraft document nodes directly,
and kept what an editor needs and a renderer can lose: the pen's caps, joins and dashes, clip
paths as clips, and the turn of an image. The shared model is the picture, extended with exactly
those three things (`LineStyle`, `ClipPath`, `Item::Bitmap::transform`), so an editor can rebuild
objects from it and a renderer can draw it. Bitmaps are decoded RGBA rather than raw DIB bytes
because every caller needs pixels (two draw them, one re-encodes them as PNG), and because the
decoder is where the size, pixel and work limits have to be enforced.

## Records read

### WMF (MS-WMF)

| Group | Records |
|---|---|
| Headers | Placeable header, memory and disk metafile headers. |
| Mapping | `SETMAPMODE`, `SETWINDOWORG`, `SETWINDOWEXT`, `SETVIEWPORTORG`, `SETVIEWPORTEXT`, all map modes (text, metric, isotropic, anisotropic). A placeable file is played into its box the way Office plays it. |
| State | `SAVEDC`, `RESTOREDC`, `SETPOLYFILLMODE`, `SETROP2`, `SETBKMODE`, `SETBKCOLOR`, `SETTEXTCOLOR`, `SETTEXTALIGN`. |
| Objects | `CREATEPENINDIRECT` (style, cap and join bits), `CREATEBRUSHINDIRECT` (solid, null, hatched), `DIBCREATEPATTERNBRUSH`, `CREATEFONTINDIRECT`, `SELECTOBJECT`, `DELETEOBJECT`, stock objects. `CREATEPALETTE`, `CREATEPATTERNBRUSH` and `CREATEREGION` take their handle slot. |
| Drawing | `MOVETO`, `LINETO`, `POLYLINE`, `POLYGON`, `POLYPOLYGON`, `RECTANGLE`, `ROUNDRECT`, `ELLIPSE`, `ARC`, `PIE`, `CHORD`, `PATBLT`. |
| Bitmaps | `STRETCHDIB`, `DIBSTRETCHBLT`, `DIBBITBLT` (with or without a source). |
| Text | `EXTTEXTOUT` (rectangle, opaque, clipped, dx), `TEXTOUT`, decoded in the selected font's character set: code pages 1250 to 1258 and 874, Shift_JIS, UHC, GBK and Big5 (a double-byte character takes the advances of both its bytes), and the symbol character set (also the Symbol font under `DEFAULT_CHARSET`), whose codes stay in the private-use area at U+F000. Face names are read in the font's code page. Johab pairs are blanks. |
| Clipping | `INTERSECTCLIPRECT`, `EXCLUDECLIPRECT`. |
| Not read | `OFFSETWINDOWORG`, `OFFSETVIEWPORTORG`, `SCALEWINDOWEXT`, `SCALEVIEWPORTEXT`, flood fills, `SETPIXEL`, region records (`FILLREGION`, `FRAMEREGION`, `PAINTREGION`, `INVERTREGION`, `SELECTCLIPREGION`, `OFFSETCLIPRGN`), the device-dependent `BITBLT`, `STRETCHBLT` and `SETDIBTODEV`. They are counted in `skipped_records`. |

### EMF (MS-EMF)

| Group | Records |
|---|---|
| Header | Frame, device size in pixels, millimetres and (when present) micrometres. |
| Mapping | `SETMAPMODE`, window and viewport origin and extent, `SCALEVIEWPORTEXTEX`, `SCALEWINDOWEXTEX`, `SETWORLDTRANSFORM`, `MODIFYWORLDTRANSFORM`. |
| State | `SAVEDC`, `RESTOREDC`, `SETPOLYFILLMODE`, `SETROP2`, `SETBKMODE`, `SETBKCOLOR`, `SETTEXTCOLOR`, `SETTEXTALIGN`, `SETMITERLIMIT`, `SETARCDIRECTION`. |
| Objects | `CREATEPEN`, `EXTCREATEPEN` (cosmetic and geometric, cap, join, user-style dashes), `CREATEBRUSHINDIRECT`, `CREATEMONOBRUSH`, `CREATEDIBPATTERNBRUSHPT`, `EXTCREATEFONTINDIRECTW`, `SELECTOBJECT`, `DELETEOBJECT`, stock objects. |
| Drawing | `MOVETOEX`, `LINETO`, `POLYLINE(16)`, `POLYLINETO(16)`, `POLYGON(16)`, `POLYBEZIER(16)`, `POLYBEZIERTO(16)`, `POLYPOLYLINE(16)`, `POLYPOLYGON(16)`, `RECTANGLE`, `ROUNDRECT`, `ELLIPSE`, `ARC`, `CHORD`, `PIE`. |
| Paths | `BEGINPATH`, `ENDPATH`, `CLOSEFIGURE`, `FILLPATH`, `STROKEPATH`, `STROKEANDFILLPATH`, `ABORTPATH`, `SELECTCLIPPATH` (`RGN_AND`, `RGN_COPY`). |
| Bitmaps | `BITBLT`, `STRETCHBLT`, `STRETCHDIBITS`, `SETDIBITSTODEVICE`, `ALPHABLEND` (constant and per-pixel alpha); source-less raster operations fill with the brush, black or white; `SRCAND`, `SRCPAINT` and `SRCINVERT` are drawn as they look over white. |
| Text | `EXTTEXTOUTA`, `EXTTEXTOUTW`, `SMALLTEXTOUT`; 8-bit strings are decoded as WMF text is; glyph-index strings are left out. |
| Clipping | `INTERSECTCLIPRECT`, `EXCLUDECLIPRECT`, `EXTSELECTCLIPRGN` (all five modes, on rectangles), `SELECTCLIPPATH`. |
| Not read | `ANGLEARC`, `ARCTO`, `POLYDRAW(16)`, `SETPIXELV`, flood fills, region paints, `MASKBLT`, `PLGBLT`, `TRANSPARENTBLT`, `GRADIENTFILL`, `POLYTEXTOUT`, `WIDENPATH`, `SETMETARGN`, `OFFSETCLIPRGN`, `SETLAYOUT`, `SELECTCLIPPATH` with `RGN_OR`, `RGN_XOR` or `RGN_DIFF` (the clip stays as it was). Unknown types are counted in `skipped_records`. |

### EMF+ (MS-EMFPLUS)

A file with EMF+ records is played from them, as GDI+ plays it. In a dual file (one that also
draws the picture with plain EMF records, as a fallback for GDI) the plain records are skipped,
except those between an `EmfPlusGetDC` record and the next EMF+ record. What the EMF+ reader does
not read (the table's last row) is therefore lost in a dual file even where its EMF records carry it.

| Feature | Read |
|---|---|
| Objects | Brushes (solid, hatch, texture, linear gradient with preset colours or blend factors, path gradient), pens (colour and width), paths, regions, images, fonts, string formats; objects continued over several records. |
| Drawing | `FillRects`, `DrawRects`, `FillPolygon`, `DrawLines`, `FillEllipse`, `DrawEllipse`, `FillPie`, `DrawPie`, `DrawArc`, `FillPath`, `DrawPath`, `DrawBeziers`, `DrawCurve`, `DrawClosedCurve`, `FillClosedCurve`, `FillRegion`, `Clear`. |
| Images | `DrawImage`, `DrawImagePoints`: raw pixel formats, PNG, JPEG, GIF, TIFF and BMP payloads, and nested metafiles (three levels). |
| Text | `DrawString` (alignment, wrapping to the layout width), `DrawDriverString` with character codes, and with glyph indices in the Windows core fonts. |
| State | World and page transforms, `Save`, `Restore`, containers. |
| Clipping | `SetClipRect`, `SetClipPath`, `SetClipRegion`, `ResetClip`, `OffsetClip`: rectangles and convex shapes cut the drawing itself; other shapes clip to their bounding rectangle; region nodes combine by intersect and union. |
| Not read | Pen caps, joins and dashes (EMF+ strokes have the default line style), image attributes and effects, glyph indices of other fonts, region XOR, exclude and complement, rendering hints, `SetRenderingOrigin`, the terminal-server records. |

### Bitmaps (DIB)

`BITMAPINFOHEADER` and larger headers at 1, 4, 8, 16, 24 and 32 bits with `BI_RGB` or
`BI_BITFIELDS`; `BITMAPCOREHEADER` at 1, 4, 8 and 24 bits; `BI_JPEG` and `BI_PNG`. Bottom-up and
top-down rows, source rectangles (only the rows and columns a record draws are decoded), mirrored
destinations. RLE compression is not decoded.

## Writers (feature `write`)

`write::emf(ops, region, app, title)` and `write::wmf(ops, region)` take `write::Op`s in points,
y down, in paint order: `Fill` (path, fill rule, colour), `Stroke` (path and `Pen`: colour, width,
cap, join, miter limit, dashes), `Image` (RGBA pixels, an affine placement, an opacity), `Clip`
and `Unclip`. `region` is the part of that space the picture covers.

- EMF keeps curves, writes geometric pens (`EXTCREATEPEN`; dashes as `PS_USERSTYLE` when the pen
  can draw them, else one line per dash), path clips inside `SAVEDC`/`RESTOREDC`, opaque images as
  `STRETCHDIBITS` and translucent ones as `ALPHABLEND`, each under a world transform.
- WMF flattens curves, writes 16-bit coordinates behind a placeable header (at most 1440 units an
  inch, fewer for large pictures), and returns what it could not hold in `write::Wmf`
  (`translucent_images`, `turned_images`, `clips_ignored`, `oversized_images`).

The reader reads back what the writers write; the tests round-trip both formats.

## Limits

Every size, count and offset from a file is checked before it is used, with `get` and checked
arithmetic. Past a limit the reader keeps what it has (a shorter picture) or leaves the item out;
it never panics and never allocates from an unchecked count. Each limit has a test.

| Limit | Value | Past it |
|---|---|---|
| Items in a picture | 200,000 | The parse stops. |
| Bytes of path points and pixels kept (pattern brushes, clip paths, EMF+ objects with the nested pictures they hold, and EMF+ clips included) | 256 MiB | The parse stops; an EMF+ object past it is not stored. |
| Points in one path bracket; points in one record | 2,000,000 | The bracket stops growing; the record is left out. |
| Object table slots | 4,096 | The object is not stored. |
| `SAVEDC` depth | 1,024 | The parse stops. |
| Clip paths nested on one device context | 32 | The parse stops. |
| Rectangles in a clip region | 64 | Further rectangles are dropped (this can only hide drawing). |
| Pen style entries | 16 | Further entries are dropped. |
| Pixels in one bitmap | 40,000,000 | The bitmap is left out. |
| Bitmap side drawn | 30,000 | The bitmap is left out before it is decoded. |
| JPEG or PNG side | 32,768 | The image is left out before it is decoded. |
| JPEG and PNG pixels decoded in one file | 2^28 | Further images are left out before they are decoded. |
| Pattern tile side | 256 | The brush is not made. |
| Characters in one string | 16,384 (EMF+: 32,768) | The string is cut. |
| EMF+ objects, save stack | 64, 64 | Ignored. |
| EMF+ points in one record; region nodes | 100,000 | The list is cut; the region is not made. |
| EMF+ region depth | 32 | The region is not made. |
| EMF+ gradient stops | 256 | The count is cut. |
| EMF+ continued object | 64 MiB | The size is cut; nothing is reserved ahead. |
| Nested metafiles | 3 levels | The inner picture is not read. |
| Clipping work per item, and per clip area | 1,000,000 and 4,000,000 steps | The item is left out; the clip keeps its previous area. |
| Masking a bitmap to a clip that is not a rectangle | 5,000,000 point tests | The bitmap is left out. |
| Placed path elements, clip regions included (`Picture::place`) | 500,000 | Placing stops. |
| Placed coordinates (`Picture::place`) | ±1,000,000 | The item is left out. |
| Placed pen width (`Picture::place`) | Twice the diagonal of the frame and the path together | The width is held there. |
| Writers: dashes cut from one stroke | 100,000 | The stroke is written solid. |
| Writers: segments from one curve | about 4,096 | The tolerance is raised. |

The tests parse every prefix of sample files, files with each byte replaced, and random bytes.

## What stays in the apps

- Mapping a `Picture` onto the app's document: VectorCraft builds paths, clipping groups, images
  and point type from the items (joining consecutive lines into one path, dropping a clip that
  covers the whole frame, turning skipped records and hatch and pattern fills into its warnings);
  WordCraft and DeckCraft keep a picture as a picture.
- Drawing: rasterising and PDF output of placed items, font lookup, shaping and glyph drawing
  (`Text` only names a font), caching of parsed pictures.
- For the writers: walking the document into `write::Op`s. Gradients, pattern fills, type as
  outlines, stroke alignment, width profiles, arrowheads, effects and opacity are decided there,
  as is scaling an image down before it is written and the wording of any warning.

## Origin

| Part | From |
|---|---|
| The reader (`canvas`, `emf`, `wmf`, `geom`, `place`, `bytes`) | DeckCraft `crates/metafile` at `cf0d7fbb`, itself ported from WordCraft at `05679497`. |
| EMF+ (`emfplus`), clip regions (`clip`, `cliprect`), text (`text`), pattern and hatch brushes | DeckCraft. |
| The cropped, depth-once DIB decoder (`dib`), the bitmap side cap, `size`, `crop_window`, the free-slot object table, capacity-based budget charging, the exact reservation for poly-polygon groups | WordCraft `crates/metafile` at `639b8133`, merged three ways over the common ancestor. |
| `LineStyle`, clip paths, `Item::Bitmap::transform`, `skipped_records`, the records behind them (`EXTCREATEPEN` entries, `SETMITERLIMIT`, `SETARCDIRECTION`, `SCALEVIEWPORTEXTEX`, `SCALEWINDOWEXTEX`, `SELECTCLIPPATH`, `ABORTPATH`), closing figures in `STROKEANDFILLPATH`, the micrometre device size, core-header, JPEG and PNG bitmaps, the per-file decode budget | VectorCraft `crates/metafile` at `3deafc24` (its reader's behaviour, written anew on this model). |
| The writers (`write`) | VectorCraft `crates/metafile` (`emf.rs`, `wmf.rs`, the encoding half of `dib.rs`, `vectorcraft_geom::flatten`). |

All three sources are MIT OR Apache-2.0. The tests build every file they read in code; the crate
bundles no sample files.

## Migration

### DeckCraft (`deckcraft-metafile`)

| Before | After |
|---|---|
| `deckcraft_metafile::{parse, is_metafile, Picture, Item, Seg, Tile, Text, Font, HAlign, VAlign, SYMBOL_CHARSET, PlacedItem, MAX_BITMAP_SIDE, Error}` | The same names in `craft_metafile`. |
| `Item::Path { segs, fill, stroke, even_odd, region }` | Adds `line` and `clips`. Draw `line` when stroking; clip to each of `clips` as to `region`. |
| `Item::Bitmap { rect, .. }`, `Item::Pattern { .. }`, `Text` | Add `clips`; `Item::Bitmap` adds `transform` (equal to `rect` unless the bitmap is turned). |
| `PlacedItem::*` | Add `clips` (`PlacedClip`: a path and its fill rule); `Path` adds `line`; `Bitmap` adds `transform`. |
| `Picture { width, height, items }` | Adds `skipped_records`. |
| `Picture::fit` with crops clamped to 95% a side | `crop_window`: each axis keeps at least 0.5%. |

### WordCraft (`wordcraft-metafile`)

| Before | After |
|---|---|
| `parse`, `size`, `is_metafile`, `Picture`, `Seg`, `Error`, `MAX_BITMAP_SIDE`, `crop_window`, `Picture::{has_size, fit, place}`, `Item::take_pixels` | The same names in `craft_metafile`. |
| `Item::Path`, `Item::Bitmap` | Add `region`, `clips`, `line`, `transform` as above; new `Item::Pattern` (hatch and pattern brushes now draw). |
| `Text { text, x, y, size, cell, font, bold, italic, color, angle, halign, valign, dx }` | `Text { transform, text, dx, font: Font { family, height, weight, italic, .. }, color, background, h_align, v_align, clip, region, clips }`. The reference point and angle are in `transform`; `size` is `font.height.abs()` in text units and `cell` is `font.height > 0`; `bold` is `font.weight >= 600`. A height of 0 and an empty family are the file's "default font": WordCraft's 16-unit Arial default moves to WordCraft. |
| `Text::layout`, `TextLayout` | Stay in WordCraft, over the shared `Text`. |
| `PlacedItem::Text { index, tf }` | `PlacedItem::Text { index, transform, clip, region, clips }`. |
| Text from `EXTTEXTOUTW` only | Also `EXTTEXTOUTA`, `SMALLTEXTOUT`, WMF text, opaque backgrounds, EMF+ strings. |

### VectorCraft (`vectorcraft-metafile`)

| Before | After |
|---|---|
| `sniff`, `Kind` | `craft_metafile::{sniff, Kind}`. |
| `import(bytes) -> Imported { document, kind, warnings }` | `craft_metafile::parse(bytes)`, then VectorCraft's own mapping of `Picture` to a `Document`: `Item::Path` to a path node (`line` to cap, join, miter limit and dash), `clips` to clipping groups (`Arc::ptr_eq` groups items), `region` to a rectangle clip, `Item::Bitmap` to an image placed by `transform`, `Item::Text` to point type, `Item::Pattern` to its tile or a flat colour, `skipped_records` to the warning. |
| `export(doc, region, kind) -> Output` | VectorCraft's `scene` walk produces `craft_metafile::write::Op`s, then `write::emf(&ops, region, "VectorCraft", &doc.title)` or `write::wmf(&ops, region)`; `Wmf`'s flags become the two WMF warnings. |
| `scene::Pen { cap: LineCap, join: LineJoin, dash: Option<Dash> }` | `write::Pen { cap: Cap, join: Join, dash: Option<write::Dash> }` (`Cap::Flat` is `LineCap::Butt`). A dash with `align_corners`, or one `Dash::emf_pen_draws` refuses, is cut by VectorCraft and passed as the path with no dash, to keep its own dash cutting. |
| `dib::Rgba::fitted`, `to_png` | Stay in VectorCraft. |
| EMF+ only files: a warning and nothing imported | They are read. |
