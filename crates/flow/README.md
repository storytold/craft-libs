# craft-flow

The model every text format maps to and from: a `Flow` is blocks, a block a paragraph (`Para`:
a `Kind` — normal, heading, title, quote, code line, rule — list membership, alignment, spacing
and formatted `Inline` spans: text, pictures, links, bookmarks, fields, notes, equations, ruby,
charts, shapes) or a table (`FTable` of `Cell`s holding blocks), with the document's `Meta`.

`to_doc` and `from_doc` convert a flow to and from a `craft_wordmodel::Document`. A format reader
builds a flow and calls `to_doc`; a writer calls `from_doc` (or `from_doc_with` and `FlowOpts` for
what that format keeps: fields, boxes, paragraph formatting, tracked changes) and walks the flow.
Formats that keep more than the flow holds (ODT) carry it in the optional fields (`extra`,
`kept`, …), which the plain formats leave empty.

```rust
use craft_flow::{FBlock, Flow, Fmt, Inline, Kind, Para, from_doc, to_doc};

let p = Para { kind: Kind::Heading(1), inlines: vec![Inline::Text("Title".into(), Fmt::default())], ..Default::default() };
let doc = to_doc(&Flow { blocks: vec![FBlock::Para(p)], ..Default::default() });
let back = from_doc(&doc);
assert!(matches!(&back.blocks[..], [FBlock::Para(p)] if p.kind == Kind::Heading(1) && p.text() == "Title"));
```

Also here: picture sniffing and sizing (`sniff_image`, `image_px`, `make_img`), CSS colours
(`parse_color`), base64 and `data:` URIs, list labels (`ListCounter`), and the one-warning-per-kind
report of what an importer leaves out to stay within a limit (`Dropped`, `begin_import`,
`dropped`).

## Provenance

WordCraft's `crates/formats/src/model` (at `07f97737`), moved as it is onto craft-wordmodel.
Image decoding for re-encoding (`decode_bounded`) stays with the RTF writer that needs it.
