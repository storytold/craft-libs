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
- Password-protected files are refused (`PptError::Encrypted`). Formats older than PowerPoint 95
  aren't read: a compound file whose current user record has neither of [MS-PPT]'s tokens and
  that has no edit record is refused as too old (`PptError::TooOld`), and a file that isn't a
  compound file is `PptError::NotPpt`. No file from an earlier PowerPoint was tried.

## PowerPoint 95

PowerPoint 95 (7.0) files use the same compound file and many of the same record numbers, but a
16-byte record header and a single record tree with no edit chain. A file whose `PowerPoint
Document` stream starts with that tree is read on its own path; anything else takes the
PowerPoint 97 path. The tree's Document must hold its DocumentAtom, which counts the slides;
`import_checked` names the slides a cut-short file is missing ("slide 3"): those the count says it
should hold, and any cut off after its header, which is left out. What
was learned of the format came from files PowerPoint 95 itself wrote,
one feature per file, and the public [MS-PPT] numbering where the records keep it.

So far a PowerPoint 95 file imports with its slide size, its masters (each the caller's default
master wearing the file's colour scheme, and its background when that isn't the scheme's), and
its slides in order with their master, layout type and, when a slide doesn't follow the master's,
its own background: a solid colour, or none when the slide's background has no fill. A slide also
says whether it shows the master's objects and whether it uses the master's colour scheme or its
own. The slides are empty: shapes, placeholders and their text, pictures and notes come later.

The caller's `craft_pptx::Defaults` supplies the master each of the file's masters starts from:
its layouts and placeholders. The file's masters then place and style the placeholders that
master has. With `craft_pptx::Plain` (one blank layout, no placeholders), the slides still read
with their own boxes.

The reader was written for DeckCraft (storytold/deckcraft#1137); the history of
`crates/ppt-binary` names its authors. Its tests build `.ppt` files from records; no file made
by PowerPoint is used. One optional test reads real PowerPoint 95 files from the directory
`CRAFT_PPT95_SAMPLES` names, and does nothing when it is unset; no such file is in the repository.
