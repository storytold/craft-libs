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
- `blip`: the BLIP store and its pictures (EMF and WMF inflated, PNG, JPEG, TIFF and DIBs as BMP
  files) within a byte budget, locating an FBSE's picture record (embedded or in a delay stream),
  and a decode budget charged before inflating or decoding.
- `dib`: a DIB decoded within limits and re-encoded as a PNG, or kept as a BMP placeholder.
- `wmf`: a WMF's placeable header, which the BLIP store leaves out, rebuilt from the BLIP's
  metafile header.

Each reader still keeps some picture handling and limits of its own; making them one set is
storytold/craft-libs#2484's next step.
