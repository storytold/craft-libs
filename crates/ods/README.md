# craft-ods

OpenDocument spreadsheets (`.ods`, `.ots`): the tables of `content.xml` (ODF 1.3 Part 3,
section 9), on `craft-odf` for the package. Clean-room. No `unsafe`.

`read(bytes, &Options)` gives a `Workbook` of `Sheet`s in document order. Each sheet has its
`table:name` as written, its non-blank `Cell`s by position (repeated rows and columns expanded)
and its merged ranges. A cell keeps the file's terms:

| Field | From |
|---|---|
| `value_type` | `office:value-type`, or `calcext:value-type="error"` |
| `raw`, `currency` | the value attribute for the type, `office:currency` |
| `text`, `paragraphs` | the displayed paragraphs, white space collapsed as ODF says, annotations left out |
| `formula` | `table:formula` as written (`of:=…`), not translated |
| `style` | `table:style-name` |
| `covered` | a `table:covered-table-cell` |

`Cell::value()` gives the shown value: numbers, booleans, strings, dates as serial numbers in
the 1900 date system (`date_serial`) and durations in days (`duration_days`). It is `None` when
the type's value is missing or unreadable, or for a formula without a cached result, so the
caller chooses what to show.

## The whole document

`Document::open(bytes, password, &Options)` reads a package, a password-protected package
(`Error::Encrypted` without a password, `Error::Password` with the wrong one) or a flat `.fods`.
It keeps, as the file has them and without interpreting them:

| Where | What |
|---|---|
| `Cell` | `annotation` (the note's tree), `drawings` (`draw:` trees anchored to it), `link` (its first `text:a`), `runs` (styled stretches of `text`), `validation`, `matrix` (the size of its matrix formula) |
| `Sheet` | `attrs` (the `table:table` element's), `columns` and `rows` (runs that state a style, a default cell style or a visibility), `column_groups`, `row_groups`, `header_columns`, `header_rows`, `blanks` (blank cells with a style or a validation, as ranges), `elements` (the table's other children: shapes, conditional formats, local names, …) |
| `Workbook` | `attrs` (the `office:spreadsheet` element's), `body` (its other children: names, database ranges, validations, data pilot tables, calculation settings, …), `font_faces`, `automatic_styles` |
| `Document` | `styles`, `settings`, `meta` (the parts' trees); `file` and `xml` read pictures and embedded objects by `package_path` |

Trees name every element and attribute with a fixed prefix (`table:`, `style:`, `draw:`,
`loext:`, …), whatever the file declared. A damaged or oversized optional part is left out and
named in `Document::warnings`; `content.xml` must be readable.

`read_content` reads a `content.xml` on its own (a flat `.fods` goes through
`craft_odf::flat::to_package` first). An encrypted package is `Error::Encrypted`; decrypt it with
`craft_odf::crypt::decrypt` and read the result.

## Hostile input

`Options` bounds the package (`craft-odf`), cells (1,000,000), merged ranges (100,000), text
(64 MiB, repeats counted) and one cell's text or attribute (1 MiB), all over the whole
workbook. Repeats and spans are checked against the 1,048,576 × 16,384 grid before anything is
expanded; blank padding past the grid's edge is clipped, data past it is an error. The XML is read strictly
(unbalanced markup, a document type declaration or an undefined entity fails the read), with
`craft-xml`'s depth and attribute limits. At most 1024 tables.

`Options::max_kept` (256 MiB) bounds what is kept besides cells: trees, rows, columns, groups
and blank ranges, with every copy of a repeated cell's note or drawing counted. Styled blank
cells, rows and columns are kept as runs and ranges, so a sheet padded with formatting to the
grid's edge stays small; a cell's styled stretches stop at 4,096.

`Options::only_sheet` keeps one sheet's contents and lists the others by name and attributes.

`Options::truncate_text` keeps the cells read before the text passes `max_text`, leaves out
every cell after that point and sets `Workbook::text_truncated`, so the caller can warn instead
of failing. Rows and cells after the cell that passes it are not read. The one-cell limit still
applies to every cell read, that cell included; the cell and merged-range limits count the cells
kept.

## Features

- `test-support` enables `testing`, which builds small `.ods` packages in code (`spreadsheet`,
  `content`, `package`, `files`) for the tests of crates that read spreadsheets. It is not a general
  spreadsheet writer.
