# craft-xls

A clean-room reader for Excel 97-2003 workbooks (`.xls`, BIFF8). It turns the bytes of a file
into a `Workbook` that keeps the file's own terms: cells, formulas as text, the format table,
column and row records. Mapping that onto an application's document is the caller's work.
No `unsafe`. Dependencies: `craft-cfb` (the compound file container), `craft-sheet` (cell positions,
values and tables shared with the other spreadsheet formats), `md-5` and `sha1` (RC4 keys),
`miniz_oxide` (compressed metafiles) and `thiserror`.
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
- The sheet the workbook was saved on (Window1).
- Cell formats: one `CellFormat` per XF record with its font, number format code, alignment,
  borders, fill and protection, and palette colours resolved to RGB; the Normal font;
  user-defined cell styles.
- Merged cells, ColInfo and Row records (width in 1/256 character, height in twips, hidden,
  outline, default format), the default row height, the Window2 display flags.
- Defined names with their scope, print areas and print titles.
- Data validation (DVal and Dv records): type, operator, formulas as text (relative references
  from the top left cell of the rule's ranges, a literal list as `"a,b"`), messages, alert style
  and flags, in SpreadsheetML's terms. Exactly the DVal's count of rules is read, as Excel does;
  a rule Excel would refuse is skipped with a warning, and at most 65,536 are kept per sheet.

- Cell comments: each Note's cell, author and visibility, with its text object's text (Obj,
  TxO and its Continue records).
- Conditional formats: CondFmt ranges with their "cell value is" and "formula is" CF rules,
  formulas relative to the bounding range, and the DXFN as a `DifferentialFormat` whose unset
  parts are `None`.
- Pictures: the drawing group's BLIP store and each sheet's picture shapes with their client
  anchors (cells and fractions): PNG, JPEG, EMF and WMF (inflated, a WMF given its placeable
  header back), and DIB as BMP. Shapes that show one picture share its bytes.
- The AutoFilter range (AutoFilterInfo with the `_FilterDatabase` name); not its criteria.

Limits for these: 256 MiB of drawing records for the workbook and for each sheet, 4096 stored
pictures, 256 MiB of inflated metafiles in all (64 MiB each), 65,536 pictures, comments and
conditional formats per sheet. A damaged comment, object or conditional-format record is
skipped with a warning.

Not read: charts, other shapes, newer conditional formats (CF12), hyperlinks,
PivotTables, macros. Each is named once in `Read::warnings`. BIFF5 and older is refused
(`Error::Biff5`).

## Encrypted workbooks

A workbook encrypted with RC4 (version 1.1, keyed through MD5) or RC4 CryptoAPI (versions 2.2,
3.2 and 4.2, keyed through SHA-1 with a 40- to 128-bit key, which current Excel writes) is read
with `read_with_password` ([MS-OFFCRYPTO] 2.3.5, 2.3.6). Without a password, Excel's default one
(`VelvetSweatshop`) is tried, and `Error::Encrypted` says a password is needed. A wrong password
is `Error::WrongPassword`; XOR obfuscation and CryptoAPI with AES, another cipher or hash, or an
external provider are `Error::UnsupportedEncryption`. Every size in the FilePass record is
checked before it is used. There is no writer
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
