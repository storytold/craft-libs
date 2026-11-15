# craft-doc-binary

Reader for the Word 97-2003 binary file format (`.doc`, [MS-DOC]) into the `craft-wordmodel`
document model. It is WordCraft's reader, on `craft-cfb` for the compound file and
`craft-codepage` for 8-bit text.

```rust
// Not a compound file: an error, never a panic.
assert!(craft_doc_binary::read(b"not a .doc").is_err());
assert!(!craft_doc_binary::is_encrypted(b"not a .doc"));
```

- `read(bytes)`, `read_with_password(bytes, Option<&str>)`, `is_encrypted(bytes)`.
- Text in pieces (8-bit and UTF-16), paragraph and character formatting, styles, lists,
  tables, sections, headers and footers, notes, comments, fields, bookmarks, pictures and
  OfficeArt shapes, document properties.
- OfficeArt WordArt text (shape types 136–175) as unfilled, borderless text boxes, including
  grouped shapes, with the named font and literal fill colour (or line colour when unfilled).
- XOR obfuscation, RC4 and RC4 CryptoAPI password protection.

WordArt is a text fallback, not a reproduction of its warp, outline or other text effects.
Font size is estimated from the displayed frame; zero text margins and auto-fit let the box
expand rather than clip wrapped lines. Empty WordArt is omitted. At most 200 Unicode characters
and 100 font-name characters are kept per shape; complex strings decode at most 4096 UTF-16
units. WordArt lines share the document's paragraph budget, and text parts share its part cap.
WordArt text is charged to the document's character budget but may use only half of it, so the
document's own text is still read; WordArt past that share is left out.

There is no writer.

## Limits

Streams (1 GiB each, refused by `craft-cfb` before anything is allocated), paragraphs, pieces,
property counts, table nesting and drawing records are capped; the constants are in the source
next to what they bound, and each has a test. Hostile input gives an error or a best-effort
document, never a panic.
