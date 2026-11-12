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

A cell keeps the file's terms: a shared-string index, a format index, the formula text as the
file spells it, value and cell metadata indexes. `CellValue::resolve` gives the value it shows.
Shared formulas are not expanded and formulas are not parsed: that needs a formula grammar.

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
