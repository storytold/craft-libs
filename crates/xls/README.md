# craft-xls

A clean-room reader for Excel 97-2003 workbooks (`.xls`, BIFF8). It turns the bytes of a file
into a `Workbook` that keeps the file's own terms: cells, formulas as text, the format table,
column and row records. Mapping that onto an application's document is the caller's work.
No `unsafe`; the only dependencies are `craft-cfb` (the compound file container) and `thiserror`.
No third-party implementation was consulted.

| Document | Used for |
|---|---|
| [MS-XLS] Excel Binary File Format (.xls) Structure (Microsoft Open Specifications) | the Workbook stream: record framing and Continue records, strings, the globals and worksheet substreams, cell records, parsed formulas (`Ptg` tokens, the `Ftab` function table), XF / Font / Palette / Style, defined names |
| [MS-CFB] Compound File Binary File Format | the container, through `craft-cfb` |

## What is read

- Worksheets in workbook order, with their visibility. Chart sheets, Excel 4.0 macro sheets,
  VBA modules and dialog sheets are skipped with a warning.
- Cell values: numbers (Number, RK, MulRk), shared and inline strings (LabelSst, Label, RString),
  booleans, errors, blanks that carry a format, and the saved results of formulas.
- Formulas as A1 text without the leading `=`, with English function names: cell formulas,
  shared formulas expanded per cell, array formulas (on their top left cell, with their range),
  3-D references, defined names, add-in and newer functions called by name.
- The 1904 date flag, "precision as displayed", calculation mode and iteration settings.
- Cell formats: one `CellFormat` per XF record with its font, number format code, alignment,
  borders, fill and protection, and palette colours resolved to RGB; the Normal font;
  user-defined cell styles.
- Merged cells, ColInfo and Row records (width in 1/256 character, height in twips, hidden,
  outline, default format), the default row height, the Window2 display flags.
- Defined names with their scope, print areas and print titles.

Not read: charts, pictures, comments, conditional formats, data validation, hyperlinks,
PivotTables, macros. Each is named once in `Read::warnings`. Encrypted workbooks
(`Error::Encrypted`) and BIFF5 and older (`Error::Biff5`) are refused. There is no writer
(`craft_xls::testing` builds small fixtures for tests).

A formula that cannot be put back into text (a link to another workbook, a macro command, an
unknown token) leaves its cell with the saved result, and `Warning::FormulaKept` says why.
Formulas are legacy (pre-dynamic-array) ones and are returned as stored: a caller whose formula
language has dynamic arrays adds Excel's implicit intersection itself.

## Use

```rust
let read = craft_xls::read(&bytes)?;
for sheet in &read.workbook.sheets {
    for (at, cell) in &sheet.cells {
        let format = cell.format.map(|i| &read.workbook.formats[usize::from(i)]);
        println!("{}!{} = {:?} {:?} {:?}", sheet.name, at.a1(), cell.value, cell.formula, format.map(|f| &f.number_format));
    }
}
for w in &read.warnings {
    eprintln!("{w}");
}
```

`read_with` takes `Limits` (records, cells, bytes of text); a caller that only wants a table of
values can lower them. `is_xls` tells whether bytes are a compound file with a workbook stream.

## Hostile input

Every count from the file is capped or clamped to what the record holds: records (one allowance
for the whole stream, so sheets pointing into one another do not multiply it), cells, text bytes,
shared strings, sheets, names, XFs, fonts, styles, merged ranges, Continue records per record,
characters per string, formula token bytes, array constant size and expression depth. Shared
formulas are expanded within the text allowance that is left after every sheet's own strings,
and past it cells keep their saved results. Nothing indexes by a number from the file.

## Origin

Moved from GridCraft's `gridcraft-xlsx` crate (`crates/xlsx/src/xls.rs` and `xls/`, at
`5cf52c5f`), where it read straight into GridCraft's workbook model. RString cells come from
WordCraft's mail-merge reader (`crates/formats/src/sheet/xls.rs`), as does the fixture writer
in `craft_xls::testing`.
