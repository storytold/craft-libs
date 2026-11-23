# craft-sheet

What every spreadsheet file format says in the same words, kept once so that `craft-xls`,
`craft-xlsx` and `craft-ods` hand an application the same types:

- `CellRef` and `Range`: zero-based cell positions, and their A1 text both ways (`B3`,
  `$B$3`, `A1:C2`), inside Excel's 1,048,576 by 16,384 grid.
- `Value` and `CellError`: what a cell stores, and the error values with their text (`#DIV/0!`).
- `Visibility`: visible, hidden, very hidden.
- `sheet_needs_quotes`, `quote_sheet`: a sheet name as a reference spells it (`'My Sheet'!A1`).
- `HAlign`, `VAlign`, `Underline`, `Script`, `BorderStyle`, `Pattern`: the cell formatting choices
  BIFF8 and SpreadsheetML share.
- `builtin_format`, `builtin_format_id`: the number format codes behind the built-in ids
  (ECMA-376 Part 1, 18.8.30; [MS-XLS] 2.4.126).
- `DEFAULT_PALETTE`: the 64 indexed colours ([MS-XLS] 2.4.188; ECMA-376 Part 1, 18.8.27).

It is not a file format and has no dependencies. It holds no document model: sheets, styles
and formulas are each format's own.
