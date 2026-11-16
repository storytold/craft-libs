# craft-xlsx

Excel workbooks: `.xlsx`, `.xlsm`, `.xltx`, `.xltm` (SpreadsheetML in an OPC package,
transitional and strict) and `.xlsb` (the same package with binary parts). Clean-room, from
ECMA-376 Part 1 (SpreadsheetML), [MS-XLSB] and observed files. No `unsafe`.

This is the part layer. `Package::open` opens the container through `craft-opc` and reads:

| Call | Gives |
|---|---|
| `workbook()` | `Workbook`: sheets in tab order (name, id, visibility, kind, part), the 1904 date flag, defined names, the active tab, and the whole part as a tree |
| `shared_strings()` | `StringItem`s: text, formatted runs, phonetic readings |
| `styles()` | `Styles`: number formats, fonts, fills, borders, cell and cell-style formats, named styles, differential formats, the indexed palette, as indexed tables |
| `sheet(entry, each)` | each `Row` of `Cell`s to a callback, then a `SheetPart`: the rest of the worksheet as a tree, its relationships, columns and merged cells |
| `opc()`, `read_xml()`, `optional_xml()`, `rels()` | any other part |

Every part is read with fixed prefixes (`ns::CANONICAL`): the SpreadsheetML main namespace
(transitional or strict) unprefixed, the others by `ns::NAMESPACES` (`r:id`, `mc:AlternateContent`,
`x14:sparklineGroups`, `xm:sqref`), and the prefixes in `mc:Ignorable` and `mc:Choice/@Requires`
rewritten to match. A tree kept from a part goes back out with `ns::declare`, which declares what
it uses. A name in a namespace that is not listed reads as `?:name` and is not written.

A cell keeps the file's terms: a shared-string index, a format index, the formula text as the
file spells it, value and cell metadata indexes. `CellValue::resolve` gives the value it shows.
Shared formulas are not expanded and formulas are not parsed: that needs a formula grammar.

## Writing

`Document::read` holds a whole SpreadsheetML workbook: the `Workbook`, shared strings, `Styles`,
each worksheet's `Row`s and `SheetPart`, and every other part as a `KeptPart` (bytes and content
type: drawings, charts, the theme, document properties, a VBA project, their relationships).
`Document::write` regenerates the workbook part (sheets, defined names, date system, active tab),
the worksheets (dimension, columns, merged cells, rows), the shared strings and the styles tables
from those fields, keeps the rest of each part's tree in place, and writes the kept parts as they
were. Part names and relationship ids are kept, so a file read and written without changes points
where it did, and writing it twice gives the same bytes. The calculation chain is dropped (Excel
rebuilds it). `XlsxKind` is the workbook part's content type (`.xlsx`, `.xlsm`, `.xltx`, `.xltm`).
Typed worksheet features (`SheetPart::features`), each mirroring its element with optional
attributes and the unnamed ones kept: sheet views (panes, selections), the default sizes
(`sheetFormatPr`), print setup (options, margins, page setup, header and footer, page breaks),
data validation (also read from the `x14`
extension), conditional formats (colour scales, data bars, icon sets, rule extensions kept),
hyperlinks (with their URLs), the legacy drawing. Comments (`Comments`, from the comments part)
and the note shapes of the VML drawing (`NoteShape`). A sheet that gains comments and has no VML
drawing gets one (`notes_vml`, in a shape block no kept drawing uses); an existing VML drawing is
kept as read. Extensions in `extLst` are carried through unchanged.

Drawings and charts, on `craft-drawingml`: `Package::drawing` reads a sheet's `xdr:wsDr` into a
`Drawing` of `Anchor`s, each with a typed `Placement` (two-cell, one-cell, absolute, with EMU
offsets) and its object kept as written; `Anchor` names the object's kind, its `cNvPr`, its chart
or image relationship and its `a:xfrm`, and `craft_drawingml::read` reads its fill, line and text.
`mc:AlternateContent` items (Office 2016 charts, ink) are kept in place, and
`Drawing::alternate_anchors` reads the anchors inside them. `Package::chart` reads a chart part
(`c:` or `cx:`) into `craft_drawingml::chart::Chart`; `Package::theme` reads the theme. A
`Document` worksheet holds its `DrawingPart` (regenerated, its relationships kept by id);
`Document::add_chart` and `Document::add_picture` add a chart part written by `craft-drawingml`
(a `chartEx` part behind `mc:AlternateContent` for the Office 2016 types) or an image, with its
anchor. Chart parts and media are kept parts.

Tables: `Package::tables` reads a sheet's table parts (in `tableParts` order) into `Table`
(every `CT_Table` attribute, `TableColumn`s with calculated and totals formulas,
`TableStyleInfo`; `autoFilter`, `sortState` and extensions kept as written). A `Document`
worksheet holds them as `TablePart`s, regenerated on writing with their relationships and
`tableParts` entries; a removed table takes both with it. Every part name a document keeps is
reserved before new parts are named, so a new part never takes a name another sheet's part
keeps.

PivotTables: `Workbook::pivot_caches` lists the caches by id; `Package::pivot_cache` reads a cache
definition (`PivotCacheDefinition`: source, fields with shared items and grouping),
`Package::pivot_records` streams its records, and `Package::pivot_tables` reads a sheet's
PivotTables (`PivotTableDefinition`: location, fields and items, axes, page and data fields,
style) with the cache each uses. `SharedItems::describe` gives the flags Excel writes for a field's
values, and `records_xml` writes a records part. A `Document` holds caches as `PivotCachePart`s
and a worksheet's tables as `PivotTablePart`s, regenerated with their relationships and
`pivotCaches` entries; an OLAP cache (and a table on one) stays a kept part.

Cell metadata: `Package::cell_pictures` (and `Document::cell_pictures`) follows a cell's `vm`
through the value and future metadata to its `_localImage` rich value and image, as
`CellPicture` (PNG or JPEG, alternative text). Every table is bounded (`MAX_RECORDS`, `MAX_PICTURE`,
`MAX_PICTURES_TOTAL`); what does not resolve is `None` with a warning, and the cell keeps its saved
value. `Document::add_cell_metadata` writes the metadata part, the rich-value parts and the images
for new pictures (`vm = i + 1`) and the `XLDAPR` dynamic-array metadata (`cm = 1`).

The parts this crate writes use the transitional namespaces; a strict file's kept parts stay as
they were. A binary workbook is not written.

A binary workbook gives the same `Workbook`, strings, rows (with their heights, outline levels,
hidden flags and styles), columns, default sizes and merged cells, and the same `Styles`:
number formats, fonts, fills, borders, cell and cell style formats, cell styles, differential
formats and the indexed colours, as Excel writes them in `styles.xml` (default fill colours and
empty settings left out, a cell style format's `apply*="0"`, a number format with a newer locale tag
in its Excel 2007 spelling, as `formatCode` has it). Custom table styles and recent colours are not
read; a damaged styles part is a warning and no styles, while one past a cap (65,536 entries in a
table, 1,024 gradient stops) or past `binary_text` is too large. The descent Excel records for rows
and the sheet (`x14ac:dyDescent`) is in alternate-content blocks, which are skipped. Its defined names are those of `definedNames` in the `.xlsx`
Excel writes: built-in names as `_xlnm.Print_Area`, sheet-scoped names with their `localSheetId`,
relative references as the cell they reach from A1, macro names with their `function`, `xlm` or
`vbProcedure`, `functionGroupId`, `shortcutKey`, `description` and `help`. Excel's hidden names for
newer functions and LET parameters are left out, as in `.xlsx`, and so is the formula-less
workbook-level built-in Excel writes beside a sheet's (`Database`), which formulas still spell
`_xlnm.Database`, and the formula-less name of a VBA function a cell calls (`.xlsm` lists none; the
cell spells it); a name whose formula can't be read is left out with a warning, and so is every name
whose formula uses a left-out name. A reference may name an XLM macro sheet (`Macro1!$A$1`, where
macro names point), and a 3-D span is spelled by its ends whatever chart sheets lie between them.
Its cell formulas are decoded from their tokens into the text an `.xlsx` file holds (`_xlfn.` and
`_xlpm.` names as written, apart from the four ISO/IEC 29500 functions `.xlsx` names bare, no
implicit `@`, the spaces and line breaks typed in the formula, numbers as Excel spells them), with
`FormulaKind::Shared` or `FormulaKind::Array` and their ranges, and `BrtCellMeta` as the cell
metadata index. Every cell of a shared formula gets its own text, decoded at the cell, since the
file keeps the tokens once. A formula whose decoding needs something the reader does not read keeps
only its saved result, with a warning saying why: a defined name left out, table references, other
workbooks, Python code stored outside the cell, data tables. The XLSB function table
(`xlsb/ftab.rs`) and the expression printer are this crate's own, apart from `craft-xls`'s.

## Hostile input

- `Limits.package`: ZIP entries, bytes per part and in all (`craft-opc`).
- `Limits.xml`: one XML part. Rows and shared strings are streamed, so this bounds one row, one
  string and what is left of a part, not a sheet's length.
- `Limits.binary_records`, `binary_cells`, `binary_text`: a binary workbook, over all its parts
  (40 million records, 4 million cells and as many shared strings, 256 MiB of text, as
  `craft-xls` allows). `binary_formula_text`: decoded formula text over the workbook (128 MiB);
  past it, formulas keep their saved results. A formula's tokens are bounded at 16 KB, its
  nesting at 256 levels and its array constants at 65,536 values in all, each charged before
  it is held; once the formula text allowance is spent, no formula is decoded. Fixed caps: 1024
  sheets, 100,000 merged ranges and 16,384 column records per sheet, 64 nested extension blocks. A
  row taller than 8,192 twips keeps the default height.
- Cells and rows outside the 1,048,576 × 16,384 grid are dropped and counted.

SpreadsheetML is read leniently: a damaged optional part is a warning, a damaged worksheet keeps
the rows before the damage. A binary part that does not parse fails the read, since skipping a
record would lose data without a trace.

Shared with `craft-xls` and `craft-ods` through `craft-sheet`: `CellRef`, `Range`, `Value`,
`CellError`, `Visibility`, sheet-name quoting, built-in number formats and the palette.
