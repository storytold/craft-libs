# craft-officeart

Clean-room reader for the Office Drawing records (OfficeArt) that Word's `.doc`, PowerPoint's
`.ppt` and Excel's `.xls` binary formats all use for their shapes and pictures. It is shared by
`craft-doc-binary`, `craft-ppt-binary` and `craft-xls`. No `unsafe`.

| Document | Used for |
|---|---|
| [MS-ODRAW] Office Drawing Binary File Formats (Microsoft Open Specifications) | the record header, shape property tables, the BLIP store and its pictures |

```rust
use craft_officeart::props::{entries, prop};
use craft_officeart::record::Records;

// An OfficeArtFOPT holding one property: pib (the picture's 1-based index) = 3.
let fopt = [0x13, 0x00, 0x0B, 0xF0, 6, 0, 0, 0, 0x04, 0x01, 3, 0, 0, 0];
let table = Records::new(&fopt).next().unwrap();
let pib = entries(table.data, usize::from(table.instance)).find(|e| e.pid == prop::PIB);
assert_eq!(pib.map(|e| e.op), Some(3));
// Truncated input is clipped or ends the walk: never a panic.
assert!(Records::new(&fopt[..5]).next().is_none());
```

- `record`: the 8-byte record header (version, instance, type, length) and a body clipped to what
  holds it; `Records` walks records laid end to end. `craft-ppt-binary` frames PowerPoint's own
  records with it too.
- `rt`: the record types.
- `props`: a property table's entries (id, operand, complex bytes), and `Props`, a merged property
  set (a shape's tables, its master shape's and the drawing defaults') with boolean words merged
  flag by flag, at most 1024 properties and 4096 UTF-16 units a string.
- `record` also sets the limits of drawing walks: groups nested at most 12 deep, and at most
  200,000 records read from all of a document's drawings together.
- `blip`: BLIP records read (only the two instances [MS-ODRAW] gives each type; a metafile
  deflated or stored, cut to its saved size and at most 64 MiB), the BLIP store (at most 4096
  entries; an entry of size 0 holds nothing; its picture embedded or in a delay stream), and a
  budget charged before every byte a picture produces, inflated or copied. Each reader sets the
  budget's total.
- `dib`: a DIB decoded within limits and re-encoded as a PNG, or kept as a BMP placeholder.
- `wmf`: a WMF's placeable header, which the BLIP store leaves out, rebuilt from the BLIP's
  metafile header.

All three readers read BLIPs, property tables and drawings with these rules and limits, and make
the same files of them: a DIB decoded as a PNG (a BMP when it can't be), a WMF with its placeable
header back. Each sets its budget's total. `craft-doc-binary` also keeps a Macintosh PICT as a
placeholder its document model can show; the `.ppt` and `.xls` models have no such placeholder, so
those readers leave PICT out, and `craft-xls` leaves TIFF out too.
