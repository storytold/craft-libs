# craft-hwpx

Reader for HWPX, the XML format of Hancom Hangul documents (OWPML, KS X 6101: a ZIP package of
XML parts), into the `craft-wordmodel` document model. It is WordCraft's reader, on `craft-zip`
and `craft-xml`. HWP 5.0, Hangul's binary format, is a separate crate.

> 본 제품은 한글과컴퓨터의 한글 문서 파일(.hwp) 공개 문서를 참고하여 개발하였습니다.
>
> (This product was developed with reference to Hancom's public document on the Hangul document
> file (.hwp) format. Hancom's terms for that document require this sentence, word for word, in the
> product's interface, manual, help and source.) Hangul and HWP are trademarks of Hancom Inc. This
> crate is not affiliated with, sponsored by or endorsed by Hancom Inc.

```rust
// Not an HWPX package: an error, never a panic.
assert!(!craft_hwpx::sniff(b"not a package"));
assert!(craft_hwpx::read(b"not a package").is_err());
```

- `sniff(bytes)`, `read(bytes)`.
- Character and paragraph shapes and styles, numbering and bullets, sections (page size,
  margins, orientation, columns, vertical text), headers and footers (both, odd and even, hidden
  on the first page), page numbers, footnotes and endnotes, memos (as comments), hyperlinks,
  bookmarks, input fields, tracked deletions, tables (merged cells, borders and fills, cell
  margins, captions, repeated header rows), pictures (cropped, turned), shapes (rectangles,
  ellipses, lines, polygons and curves, with fills, gradients, dashes and arrowheads), text in
  shapes, groups, form objects (as boxes showing their caption), the master page (a part of
  its own) and page background (in every header of their section, behind the text), and page
  borders.
- Korean words wrap whole where a paragraph says so (`ParaProps::keep_hangul_words`).
- Where Hancom lays out differently from the model, the reader keeps what the file's layout cache
  says (line heights, a table's last row on its page); each such rule says in the source what it
  was checked against.
- For saving back: the file read is kept in `Document::passthrough` under `PACKAGE_KEY`, and each
  paragraph's source paragraph is filed in `Document::kept` under `KEPT_FORMAT` while the
  paragraph stays as read.
- What isn't read is logged (`log::warn!`), once per kind: equations show their script as
  italic text, arcs are drawn as their frame, cells lose their diagonal lines, and OLE objects,
  charts, text art and video are left out, as is a page background whose picture the package
  doesn't hold.

## Sources

| Document | Used for |
|---|---|
| Hancom's OWPML model, [`hwpx-owpml-model`](https://github.com/hancom-io/hwpx-owpml-model) at commit 1453388 (Apache-2.0) | element and attribute names, enumeration values, namespace URIs and package paths; names only, no code taken |
| Hancom's "한글 문서 파일 형식 5.0" (Hangul document file format 5.0), revision 1.3 | what HWPX shares with HWP 5.0, such as its unit (HWPUNIT, 1/7200 inch); the attribution sentence above comes with it |
| Hancom's free viewer, compared from outside (a document opened, its window looked at) | layout rules the documents don't state; no program was disassembled or analysed |

The rules settled by comparing with the viewer were found before this crate existed, in WordCraft;
each is listed with its date and what it was checked on in WordCraft's [clean-room
record](https://github.com/WhyEichiO/wordcraft-hwp-hwpx-ViewerEditor/blob/f68cfa0b4b948e07173e23703d4260d268f361a6/docs/hwp/cleanroom.md)
(section 3 says how comparisons were made, section 4 what was checked, section 7 what is still a
guess). This crate adds no new ones.

No other HWP or HWPX implementation was read. Test documents are written by hand in the tests;
real documents are never committed.

The code was moved from WordCraft (`crates/hwp`, storytold/wordcraft#6915, branch
`hangul-documents` at `f68cfa0b4b948e07173e23703d4260d268f361a6`), where it wrote a DOCX package
for WordCraft's DOCX reader. Here it fills the model directly, with the same decisions; on the
documents it was checked on, both give the same document.

## Tests

`cargo test -p craft-hwpx`. A folder of real documents can be read too:
`CRAFT_HWP_SAMPLES=/path/to/folder cargo test -p craft-hwpx --test it local_samples`. Without the
variable that test says it was skipped and passes; `CRAFT_HWP_SAMPLES_REQUIRED=1` makes a missing
folder fail.

## Limits

Each part or picture is read up to 256 MiB, the whole archive up to 1 GiB uncompressed and 65,536
entries. XML is followed 256 elements deep and up to 8,000,000 elements per part. Tables, text in
shapes and notes nest at most 64 deep, groups 8; a table has at most 32,767 rows and 63 columns
(a span past them ends at the edge), a section at most 45 text columns, and at most 200,000 cells the file declares without writing are made in all. Page sizes and
margins are capped at 14,400,000 HWPUNIT, and a tiled page background lays at most 10,000
tiles, and all of a document's 50,000 (more: the picture stretched once, logged). The constants are in the source next to what
they bound. Damaged or hostile input gives an error or a best-effort document, never a panic
(`tests/it/main.rs` sweeps truncations and mutations).
