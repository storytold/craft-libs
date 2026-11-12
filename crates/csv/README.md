# craft-csv

Delimited text (CSV, TSV and their relatives) and SYLK (`.slk`). No `unsafe`; the only
dependency is `thiserror`.

Fields are text. Reading a field as a number or a date, deciding how a value is written, and
mapping a SYLK sheet onto a document are the caller's work.

## Delimited text

| Function | What it does |
|---|---|
| `decode_text(bytes)` | UTF-8 and UTF-16 byte order marks; without one, valid UTF-8 is kept; other bytes are Windows-1256 when they read as Arabic, else Windows-1252 |
| `sniff_delimiter(text, quote)` | comma, semicolon or tab: the one with the most consistent count per line over the first 20 non-blank lines, outside quotes |
| `parse_records(text, delim, quote, max_fields, on_record)` | records of fields, one callback per record (return `false` to stop); CR, LF and CRLF line ends; quoted fields with doubled quotes and line breaks; a quote in the middle of a field is text |
| `records(text, delim, quote, max_records, max_fields)` | the same, collected |
| `push_field(out, field, delim)`, `field_len(field, delim)` | one field as written, quoted only when it holds the delimiter, a quote or a line break; its exact length, to enforce a size cap before writing |

`max_fields` bounds what one record keeps, so a row of endless delimiters cannot allocate a
string per field. The delimiter between fields and the line end are the writer's own.

## SYLK

`sylk::read(bytes)` gives a `sylk::Sheet`: cells by (row, column) with a value, the formula in A1
notation (the file's R1C1 text with its references rewritten, not checked for syntax) and the
number format code; column widths in characters; and the number of cell records skipped because
their position is outside the grid. `sylk::r1c1_formula_to_a1` is the rewrite on its own.

Files over 512 MB are refused, positions outside 1,048,576 rows by 16,384 columns are skipped,
and at most 1,024 column-width records are kept.

## Origin

Moved from GridCraft: `crates/xlsx/src/csv.rs` (decoding, sniffing, the record parser and the
quoting of its writer) and `crates/engine/src/io/sylk.rs`, at `5cf52c5f`. GridCraft keeps the
parts that depend on its model: typed values and dates from fields, and the sheet it builds.
