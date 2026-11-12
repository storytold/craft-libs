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
attributes and the unnamed ones kept: sheet views (panes, selections), print setup (options,
margins, page setup, header and footer, page breaks), data validation (also read from the `x14`
extension), conditional formats (colour scales, data bars, icon sets, rule extensions kept),
hyperlinks (with their URLs), the legacy drawing. Comments (`Comments`, from the comments part)
and the note shapes of the VML drawing (`NoteShape`). A sheet that gains comments and has no VML
drawing gets one (`notes_vml`, in a shape block no kept drawing uses); an existing VML drawing is
kept as read. Extensions in `extLst` are carried through unchanged.

The parts this crate writes use the transitional namespaces; a strict file's kept parts stay as
they were. A binary workbook is not written.

A binary workbook gives the same `Workbook`, strings, rows and merged cells, with saved values
only: its formulas are token streams that are not decoded, and its styles are not read (each
cell keeps its format index).

## Hostile input

- `Limits.package`: ZIP entries, bytes per part and in all (`craft-opc`).
- `Limits.xml`: one XML part. Rows and shared strings are streamed, so this bounds one row, one
  string and what is left of a part, not a sheet's length.
- `Limits.binary_records`, `binary_cells`, `binary_text`: a binary workbook, over all its parts
  (40 million records, 4 million cells and as many shared strings, 256 MiB of text, as
  `craft-xls` allows). Fixed caps: 1024 sheets, 100,000 merged ranges per sheet, 64 nested
  extension blocks.
- Cells and rows outside the 1,048,576 × 16,384 grid are dropped and counted.

SpreadsheetML is read leniently: a damaged optional part is a warning, a damaged worksheet keeps
the rows before the damage. A binary part that does not parse fails the read, since skipping a
record would lose data without a trace.

Shared with `craft-xls` and `craft-ods` through `craft-sheet`: `CellRef`, `Range`, `Value`,
`CellError`, `Visibility`, sheet-name quoting, built-in number formats and the palette.
