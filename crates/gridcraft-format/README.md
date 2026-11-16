# craft-gridcraft-format

GridCraft's native workbook file is one pretty-printed UTF-8 JSON object, with a two-space indent
and no newline at the end. It is saved as `.json`, or as `.scjson` by older builds.

The workbook is a type parameter. GridCraft owns the model, and after a read it rebuilds the
model's indexes, drops cells outside the grid and reads the macros. This crate owns the bytes:

- `EXTENSIONS` and `is_extension` name the format.
- `read(bytes)` reads a workbook. A failure reads as "not a GridCraft JSON workbook: …".
- `write(workbook)` writes one with `serde_json`'s pretty printer.
- `write_with_kept(workbook, field, kept)` appends entries that a newer version wrote, and this
  build kept without reading, to the top-level list `field`, as the file held them. With nothing
  kept, the output is exactly `write`'s. Otherwise the workbook goes through a `serde_json::Value`
  first, so its object keys come out in map order, as GridCraft has always written them.

There is no `unsafe` and no panics, and it builds for `wasm32-unknown-unknown`.
