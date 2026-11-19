# craft-ppt-binary

Reader for the PowerPoint 97-2003 binary file format (`.ppt`, `.pot` and `.pps`) into the
`craft-pptx` presentation model. It is DeckCraft's reader, on `craft-cfb` for the compound file.
Import only: there is no `.ppt` writer.

| Document | Used for |
|---|---|
| [MS-PPT] PowerPoint (.ppt) Binary File Format (Microsoft Open Specifications) | the Current User stream, the edit chain and persist directories, documents, masters, slides, notes, text and its styles, headers and footers |
| [MS-ODRAW] Office Drawing Binary File Formats | shapes, their property tables and geometry, the BLIP store and its pictures |
| [MS-CFB] Compound File Binary File Format | through `craft-cfb` |

```rust
// Not a compound file: an error, never a panic.
assert!(!craft_ppt_binary::sniff(b"not a .ppt"));
assert!(craft_ppt_binary::import(b"not a .ppt", &craft_pptx::Plain).is_err());
```

- `sniff(bytes)`, `import(bytes, &defaults)` and `import_checked(bytes, &defaults)`, which also
  names the slides the file lists but whose objects are gone.
- Main masters become masters, with their colour scheme, fonts, text styles, background and
  decorative shapes. Title masters become layouts of their main master.
- Slides keep their shapes (preset and freeform geometry, groups, rotation and flips), fills,
  lines, shadows, pictures (PNG, JPEG, DIB, TIFF, EMF and WMF), tables (as groups of shapes, the
  way the file stores them), and text with the master's styles resolved into it, along with
  fields, numbering, headers and footers, notes, and hidden slides.
- Damaged files read as far as they can: a broken edit chain falls back to a scan of the edits,
  and unknown or damaged records are skipped. Sizes, counts, nesting and picture decoding are
  bounded.
- Password-protected files are refused (`PptError::Encrypted`), as are PowerPoint 95 and earlier
  files (`PptError::TooOld`).

The caller's `craft_pptx::Defaults` supplies the master each of the file's masters starts from:
its layouts and placeholders. The file's masters then place and style the placeholders that
master has. With `craft_pptx::Plain` (one blank layout, no placeholders), the slides still read
with their own boxes.

The reader was written for DeckCraft (storytold/deckcraft#1137); the history of
`crates/ppt-binary` names its authors. Its tests build `.ppt` files from records; no file made
by PowerPoint is used.
