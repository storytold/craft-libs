# craft-hwp

Reader for HWP 5.0, the binary format of Hancom Hangul documents (a compound file of record
streams), on the `craft-wordmodel` document model. It is WordCraft's HWP reader, on `craft-cfb`.
The records are rebuilt as the OWPML elements an HWPX holds and read through `craft-hwpx`
(`craft_hwpx::owpml::read`), so both formats read the same way and a rule fixed for one holds for
the other.

> 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
>
> (This product was developed with reference to Hancom's public document on the Hangul document
> file (.hwp) format. Hancom's terms for that document require this sentence, word for word, in the
> product's interface, manual, help and source.) Hangul and HWP are trademarks of Hancom Inc. This
> crate is not affiliated with, sponsored by or endorsed by Hancom Inc.

```rust
// Not an HWP file: an error, never a panic.
assert!(!craft_hwp::sniff(b"not a compound file"));
assert!(craft_hwp::read(b"not a compound file").is_err());
```

- `sniff(bytes)`, `read(bytes)`, `HwpError`, `FILE_KEY`.

Reading:

- DocInfo becomes the header: fonts, border fills, character and paragraph shapes, tab
  definitions, numbering and bullets, styles and the pictures stored in the file.
- Each BodyText section becomes a section: paragraphs and their character runs, the section
  definition (page size, margins, columns, page borders and backgrounds, the master page),
  headers and footers, page numbers and page hiding, footnotes and endnotes, memos, fields
  (hyperlinks, click-here fields, bookmarks, dates, formulas and the others the format names),
  tables (merged cells, cell zones, margins), pictures, shapes (lines, rectangles, ellipses, arcs,
  polygons and curves), groups, form objects and equations.
- From there the document is read as `craft-hwpx` reads an HWPX; see its README for what the
  model holds and what is logged.
- Compressed documents are inflated stream by stream. Password-protected and distribution-only
  documents (Hancom: 배포용) give `HwpError::Protected`.
- The file read is kept in `Document::passthrough` under `FILE_KEY`, and each paragraph's source
  paragraph in `Document::kept` (`craft_hwpx::KEPT_FORMAT`), for a writer to come.

Not yet: writing HWP 5.0 (a new file, or edits saved back into the file read).

## Sources

| Document | Used for |
|---|---|
| Hancom's "한글 문서 파일 형식 5.0" (Hangul document file format 5.0), revision 1.3 | the record layouts of DocInfo and BodyText, the controls and their ids, compression, the file header flags; the attribution sentence above comes with it |
| [MS-CFB] Compound File Binary File Format, through `craft-cfb` | the container |
| Hancom's free viewer, compared from outside (a document opened, its window looked at) | where the document and real files disagree (a line type table shifted by one, margins stored doubled); no program was disassembled or analysed |

Where the format document and real files disagree, the comment in the source says what the files
showed. Those comparisons were made before this crate existed, in WordCraft, and are listed with
their dates in WordCraft's [clean-room
record](https://github.com/WhyEichiO/wordcraft-hwp-hwpx-ViewerEditor/blob/f68cfa0b4b948e07173e23703d4260d268f361a6/docs/hwp/cleanroom.md).
This crate adds no new ones.

No other HWP or HWPX implementation was read. Test files are written record by record in the
tests; real documents are never committed.

The code was moved from WordCraft (`crates/hwp/src/hwp5.rs`, storytold/wordcraft#6915, branch
`hangul-documents` at `f68cfa0b4b948e07173e23703d4260d268f361a6`), where the rebuilt elements went
through the same converter as HWPX. On the documents it was checked on, both give the same
document.

## Tests

`cargo test -p craft-hwp`. A folder of real documents can be read too:
`CRAFT_HWP_SAMPLES=/path/to/folder cargo test -p craft-hwp --test it local_samples`. Without the
variable the test says it was skipped and passes; `CRAFT_HWP_SAMPLES_REQUIRED=1` makes a missing
folder fail.

## Limits

Each stream is read up to 256 MiB, all streams up to 1 GiB, as stored and once inflated (each
counts), and a file holds at most 65,536 streams. DocInfo and all sections together hold at most
2,000,000 records. Paragraphs in tables, text boxes, notes and groups are followed 64 deep. The
limits of `craft-hwpx` apply to the rebuilt document. The constants are in the source next to
what they bound. Damaged or hostile input gives an error or a best-effort document, never a panic
(`tests/it/main.rs` sweeps truncations and mutations of the file and of its record streams).
