# craft-doc-binary

Reader and writer for the Word 97-2003 binary file format (`.doc`, [MS-DOC]) and the
`craft-wordmodel` document model. It is WordCraft's reader and writer, on `craft-cfb` for the
compound file and `craft-codepage` for 8-bit text.

```rust
// Not a compound file: an error, never a panic.
assert!(craft_doc_binary::read(b"not a .doc").is_err());
assert!(!craft_doc_binary::is_encrypted(b"not a .doc"));

// A document written as a .doc reads back.
let bytes = craft_doc_binary::write(&craft_wordmodel::Document::from_text("Hello")).unwrap();
let doc = craft_doc_binary::read(&bytes).unwrap();
assert_eq!(doc.body[0].as_para().map(|p| p.text.as_str()), Some("Hello"));
```

- `read(bytes)`, `read_with_password(bytes, Option<&str>)`, `is_encrypted(bytes)`, `write(&document)`.
- Text in pieces (8-bit and UTF-16), paragraph and character formatting, styles, lists,
  tables, sections, headers and footers, notes, comments, fields, bookmarks, pictures and
  OfficeArt shapes, document properties.
- XOR obfuscation, RC4 and RC4 CryptoAPI password protection (the RC4 schemes through `craft-offcrypto`).
- Tracked insertions and deletions, with their author and time.

`write` produces a Word 97-2003 file (nFib 0x00C1, a version 3 compound file): text, character
and paragraph formatting, styles, fonts, lists, tables, sections with headers and footers,
footnotes and endnotes, fields and hyperlinks, bookmarks, inline pictures, tracked insertions
and deletions, and the document properties. Floating pictures are written in line, nested tables
become paragraphs of their cell, and shapes, charts, text boxes and comments are left out, each
kind logged once per save. It never writes a password-protected file.

## Limits

Streams (1 GiB each, refused by `craft-cfb` before anything is allocated), paragraphs, pieces,
property counts, table nesting and drawing records are capped; the constants are in the source
next to what they bound, and each has a test. Hostile input gives an error or a best-effort
document, never a panic.
