# craft-docx

DOCX (ECMA-376 / ISO/IEC 29500 WordprocessingML) read and written, on the `craft-wordmodel`
document model. Also Word XML Documents (Flat OPC, one `.xml` file holding the whole package)
and Word 2003 XML, password-protected packages and Restrict Editing password hashes.

It is WordCraft's DOCX crate, moved onto the shared crates: `craft-opc` for the package,
`craft-xml` for parsing, `craft-offcrypto` for encryption, `craft-wordmodel` for the model.

```rust
use craft_wordmodel::Document;

let doc = Document::from_text("Hello\nWorld");
let bytes = craft_docx::write(&doc).unwrap();
let back = craft_docx::read(&bytes).unwrap();
assert_eq!(back.plain_text(craft_wordmodel::StoryRef::Body), doc.plain_text(craft_wordmodel::StoryRef::Body));
```

## Reading

- `read(bytes)`, `read_with(bytes, &ReadOptions { password, chunks })`, `read_with_password`.
- `read_xml(bytes)`: a Word XML Document or a Word 2003 XML Document, told apart by its root.
- `package_title(reader, max_part)`: the title in `docProps/core.xml`, inflating only that part.

The reader is lenient (unknown markup is skipped, bad numbers fall back to defaults) and bounded:

| Limit | Value |
| --- | --- |
| ZIP entries | 20,000 (checked before the archive is indexed) |
| One part | 256 MiB |
| All parts | 512 MiB, counting entries that fail their checksum |
| XML nesting | 256 open elements |
| XML elements | 8,000,000 in one part |
| Attributes | 1,024 on one element |
| Word XML file | 1 GiB |

Markup the model does not hold is kept in `Document::passthrough` and the `kept` markup, and
written back as it was read.

## Writing

`write(doc)`, `write_as(doc, Flavor)` for `.docx`, `.docm`, `.dotx` and `.dotm`; `write_flat`
and `write_wordml2003` for the XML forms; `encrypt` for a password.

Equations (OMML) are read and written by `craft_math::omml`.

## Not here yet

- DrawingML (`a:`) shape fills, lines, custom geometry and colours are still read with this
  crate's own code; they move to `craft-drawingml`.
