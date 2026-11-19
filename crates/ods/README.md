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

`Options::only_sheet` keeps one sheet's cells and lists the others by name.
