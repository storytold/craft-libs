# craft-markdown

Markdown, read and written: a CommonMark subset plus GitHub pipe tables and strikethrough, to and
from the [craft-flow](../flow/README.md) model and a craft-wordmodel `Document`.

- Read (`import`, `import_on`, `parse` for the flow): ATX and setext headings, paragraphs with
  soft and hard breaks, block quotes, fenced and indented code, thematic breaks, bullet and
  ordered lists nested by indentation (with their start number), GFM tables, link reference
  definitions; inlines by the CommonMark delimiter algorithm: emphasis and strong, `~~strike~~`,
  code spans, inline, reference and autolinks, images (`data:` URIs become pictures, others their
  alt text), backslash escapes, character references, HTML comments, `<br>` and `<a id>` anchors.
- Write (`export`): the same constructs; pictures become `![alt](name)`.

```rust
let doc = craft_markdown::import("# Title\n\nSome *emphasis* and a [link](https://example.com).\n");
assert_eq!(craft_markdown::export(&doc).trim_end(), "# Title\n\nSome *emphasis* and a [link](https://example.com).");
```

Input is hostile. Nesting is bounded, and table cells are capped (`MAX_TABLE_CELLS` in all, plus
the model's row and column limits), with one warning per kind of thing left out.

Moved from WordCraft (`crates/formats/src/markdown`, `07f97737`) with its tests; character
references come from craft-html.
