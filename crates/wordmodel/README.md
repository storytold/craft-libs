# craft-wordmodel

A word-processing document model: stories, paragraphs, runs, tables, sections, styles, numbering,
fields, content controls, notes, comments, revisions, floating objects, drawings and math. It is
what the DOCX, RTF, ODT and other text-format crates read into and write from.

It is WordCraft's document model (`crates/doc`), moved without its editing code. Module paths,
type names and serde attributes are as they were, so an app that re-exports the crate keeps its
native file format.

```rust
use craft_wordmodel::{Block, Document};

let doc = Document::from_text("One\nTwo");
assert_eq!(doc.body.len(), 2);
assert!(matches!(&*doc.body[0], Block::Para(p) if p.text == "One"));
```

## What is here

- `Document`: the body story, secondary stories (`parts`: headers, footers, notes, comments,
  text boxes), the style sheet, list definitions, comments, revisions, settings, document
  properties, media and parts kept from the source file.
- `Block`, `Paragraph`, `Run`, `InlineObject`, `Table`: the content. Paragraph text is UTF-8;
  an inline object stands in the text as U+FFFC (`para::OBJ`).
- `props`, `styles`, `resolve`, `numbering`, `section`: formatting and its resolution.
- `math`, `math_latex`, `math_linear`, `math_symbols`: the equation tree (`MNode`) and its
  linear and LaTeX forms.
- `chart`, `smart_art`, `graphic`, `fill`, `connector`, `freeform`, `wordart`, `effects`,
  `picfx`, `line_dash`: drawing objects as the model holds them. Reading and writing their
  DrawingML is `craft-drawingml`'s job, in the format crates.
- `kept`: markup kept from the file a document was read from, so what is not edited is written
  back as it was.
- `walk`, `work`, `geom`, `bidi`, `cluster`, `collate`, `locale_number`, `symbol_fonts`.

## Not here

Range editing, copy and paste, IME text and equation editing stay with the app that edits.
Code pages are `craft-rtf`'s `codepage`, the ZIP entry-count check is `craft-opc`'s.

## Limits

The model carries the limits its readers rely on: nesting of tables (`table::MAX_TABLE_DEPTH`),
text boxes (`MAX_TEXT_BOX_DEPTH`, `BoxBudget`), math (`math::MAX_DEPTH`, matrix sizes), group
children, alt text length, linked boxes and the walk depth (`PARA_WALK_DEPTH`). Their tests
moved with them.
