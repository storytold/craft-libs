# craft-parquet

Flat Parquet tables: one primitive column per field. No `unsafe`; the decoder is Apache's
[`parquet`](https://github.com/apache/arrow-rs/tree/main/parquet) crate without its Arrow layer.
No Python, database, full Arrow stack or background worker is needed.

Values keep the file's own units. Turning a date into a spreadsheet serial, an integer beyond
binary64 precision into text, or a column into a header row is the caller's work.

## Reading

| Function | What it does |
|---|---|
| `is_parquet(bytes)` | `PAR1` or `PARE` at the start, whatever the file's extension |
| `read_rows(bytes, limits, on_row)` | one callback per row of `Value`s, after every metadata check has passed; an error the callback returns stops the read and comes back to the caller; returns the field names and the row count; copies the input |
| `read_owned_rows(bytes, limits, on_row)` | the same, handing the owned `Vec<u8>` to the decoder without a second full-file copy |
| `read(bytes, limits)`, `read_owned(bytes, limits)` | the same, collected into a `Table` (a second copy of the data) |

Supported input: flat optional or required numeric, Boolean and UTF-8 fields; decimals (as exact
digits); dates; millisecond and microsecond times and timestamps, as stored. Formula-looking
strings stay text. Decimals whose precision or scale passes 76 digits, decimal values longer than their precision needs, NaN and infinities, nested, repeated, list and map fields, duplicate field
names, binary values, unknown logical types, nanosecond and INT96 times and timestamps, encrypted
files and external column files are refused with a message, never truncated.

Uncompressed, Snappy, Gzip, Brotli and LZ4_RAW input work on native and web. ZSTD input works on
native builds; on wasm32 the decoder reports an unsupported codec.

## Writing

`write(root, names, rows, cell)` writes `rows` data rows under unique, nonempty field names,
Snappy-compressed, in row groups of 4,096 rows, so it never collects a whole column. `cell(row,
column)` gives each value as text, a finite number, a Boolean or empty, and an error it returns
stops the write. Each column holds one type; blanks become nulls and a column of blanks is text.
The callback runs twice per cell, and both passes check the value it returns.

## Resource bounds

`TableLimits` carries the caller's caps on data rows, columns and cells (field names plus data
rows, times columns). Before the decoder allocates, the crate preflights the compact-Thrift footer
and page headers:

| Bound | Value |
|---|---|
| footer metadata | 16 MiB |
| envelope nesting | 32 levels |
| pages | 1,000,000 |
| one page | 64 MiB |
| decoded page data | 512 MiB |
| decimal bytes read, counted per row | 512 MiB |
| text bytes read, counted per row | 512 MiB |
| one text value or field name | 1 MiB |
| text a `Table` keeps | 512 MiB |
| text written | 512 MiB |

These are expansion budgets, not a cap on file size. Input files still sit in memory; this is not
a streaming dataset browser.

For native profiling without printing cell contents, run
`cargo run -p craft-parquet --example inspect -- file.parquet --owned`. Omit `--owned` to time
the borrowed path.

Specification: https://parquet.apache.org/docs/file-format/
