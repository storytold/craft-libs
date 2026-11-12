# craft-pdn

Paint.NET documents (`.pdn`), read. Only the PDN3 layout (Paint.NET 3 and 4): the `PDN3`
signature, an XML header (skipped), a .NET binary serialization stream holding a
`PaintDotNet.Document`, then one block of pixels per surface in numbered chunks, each a gzip
stream or raw bytes. The layout follows the reader of the pypdn project (MIT); the serialization
records follow Microsoft's public MS-NRBF specification.

- `is_pdn`: the signature.
- `Document::parse`: canvas size, bitmap layers (name, visibility, opacity 0 to 255, Paint.NET's
  own `BlendMode`, from the current `blendMode` field or the older `UserBlendOps` class names)
  and which surface each layer shows. Every allocation the pixels need is checked first.
- `Document::read_surfaces`: decodes the pixel blocks in file order, one `Surface` (rows of
  B, G, R, A bytes, straight alpha) at a time, with a `keep_going` callback.
- `nrbf`: the data-only MS-NRBF reader on its own: a `Graph` of `Value`s with references kept as
  ids. No .NET type is instantiated or run.

Paint.NET's Color Burn and Color Dodge are its own formulas; what an application maps them to is
its own choice. There is no writer.

## Limits

| Limit | Value |
| --- | --- |
| Canvas, and all pixel blocks together | 1 GiB (`MAX_PIXEL_BYTES`) |
| Layers | 1 to 1024 (`MAX_LAYERS`) |
| A layer name; all names | 64 KiB (`MAX_NAME_BYTES`); 4 MiB (`MAX_TOTAL_NAME_BYTES`) |
| Pixel chunks in one block | 1,000,000 |
| NRBF records, items and members | 1,000,000 (`nrbf::MAX_ITEMS`) |
| NRBF string bytes | 64 MiB (`nrbf::MAX_STRINGS`) |
| NRBF nesting and reference chains | 64 (`nrbf::MAX_DEPTH`) |

A raw pixel block shorter than it declares is refused before its buffer is allocated.
