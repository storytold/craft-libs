# craft-hwpx

Reader and writer for HWPX, the XML format of Hancom Hangul documents (OWPML, KS X 6101: a ZIP
package of XML parts), on the `craft-wordmodel` document model. It is WordCraft's reader and
writer, on `craft-zip` and `craft-xml`. HWP 5.0, Hangul's binary format, is `craft-hwp`, which
rebuilds its records as OWPML element trees and reads them through this crate (`owpml::read`).

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

// A document written as a new package reads back.
let doc = craft_wordmodel::Document::from_text("한글 문서");
let bytes = craft_hwpx::write(&doc).unwrap();
let back = craft_hwpx::read(&bytes).unwrap();
assert!(back.body.iter().filter_map(|b| b.as_para()).any(|p| p.text == "한글 문서"));
```

- `sniff(bytes)`, `read(bytes)`, `write(doc)`, `write_with(doc, &WriteOptions)`,
  `set_application_name(name)`.
- `owpml::read(header, sections, binaries)`: a document another Hangul format rebuilt as OWPML
  element trees (`owpml::Element`), read as an HWPX package's parts are. No file is kept for
  saving back, so `write` makes a new package of it.

Reading:

- Character and paragraph shapes and styles, numbering and bullets, sections (page size,
  margins, orientation, columns, vertical text), headers and footers (both, odd and even, hidden
  on the first page), page numbers, footnotes and endnotes, memos (as comments), hyperlinks,
  bookmarks, input fields, tracked deletions, tables (merged cells, borders and fills, cell
  margins, captions, repeated header rows), pictures (cropped, turned), shapes (rectangles,
  ellipses, lines, polygons and curves, with fills, gradients, dashes and arrowheads), text in
  shapes, groups, form objects (as boxes showing their caption), the master page and page
  background (in the headers, behind the text), and page borders.
- Korean words wrap whole where a paragraph says so (`ParaProps::keep_hangul_words`).
- Where Hancom lays out differently from the model, the reader keeps what the file's layout cache
  says (line heights, a table's last row on its page); each such rule says in the source what it
  was checked against.
- For saving back: the file read is kept in `Document::passthrough` under `PACKAGE_KEY`, and each
  paragraph's source paragraph is filed in `Document::kept` under `KEPT_FORMAT` while the
  paragraph stays as read.
- What isn't read is logged (`log::warn!`), once per kind: equations show their script as
  italic text, and OLE objects, charts, text art and video are left out.

Writing:

- A document read from an HWPX is written back into a copy of that file: the paragraphs whose
  text or formatting changed get their runs rebuilt (typed text takes its neighbour's character
  shape, the layout cache is dropped for Hancom to lay them out again), new paragraphs copy the
  shape of the one before, deleted ones go, tables whose structure, borders or fill changed are
  written again whole, and new, replaced, resized or removed pictures are written. Everything
  else keeps its XML byte for byte, and the other parts of the package their content.
- Which paragraph of the edited document is which paragraph of the file: the file is read again
  and the stories are lined up block by block. Unchanged blocks are found by
  `craft_wordmodel::kept::fingerprint`; the ones between two of them are paired, the most alike
  first; an unchanged block out of order was moved (its new place isn't written); a paragraph
  gone whose text starts or ends its neighbour's was merged into it.
- Only paragraphs whose text the writer can rewrite are edited: text runs not interleaved with
  fields, notes or objects. An edit elsewhere, a moved paragraph, and a comment added or deleted
  are left as they were in the file and logged.
- Any other document becomes a new package: text and character shapes, paragraph shapes, page
  setup per section, tables, headers and footers with page numbers, notes, pictures, text boxes,
  rectangles, ellipses, lines, freeforms, groups, lists (as paragraph headings), paragraph style
  names and comments (as memos). Other drawn shapes, charts, diagrams and hyperlink destinations
  are left out and logged. `WriteOptions::hangul_font` names the font for Hangul beside a font
  without it (an app answers from the fonts it has); without it the run's own font is named.
- `version.xml` names the application as `set_application_name` gave it ("WordCraft" until
  then).

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

`cargo test -p craft-hwpx`. A folder of real documents can be read, and written back unedited
(every part as it was), too:
`CRAFT_HWP_SAMPLES=/path/to/folder cargo test -p craft-hwpx --test it local_samples`. Without the
variable those tests say they were skipped and pass; `CRAFT_HWP_SAMPLES_REQUIRED=1` makes a
missing folder fail.

## Limits

Each part or picture is read up to 256 MiB, the whole archive up to 1 GiB uncompressed and 65,536
entries. XML is followed 256 elements deep and up to 8,000,000 elements per part. Tables, text in
shapes and notes nest at most 64 deep, groups 8; a table has at most 32,767 rows and 63 columns,
and at most 200,000 cells the file declares without writing are made in all. Page sizes and
margins are capped at 14,400,000 HWPUNIT, and a tiled page background lays at most 10,000
tiles (more: the picture stretched once, logged). The constants are in the source next to what
they bound. Damaged or hostile input gives an error or a best-effort document, never a panic
(`tests/it/main.rs` sweeps truncations and mutations, and writes what they read).

Writing follows tables 32 deep and groups 8 deep; deeper ones are left out and logged. Text
compared for an edit takes at most 50,000,000 steps or 2,000 single-character edits, past which
the changed middle is written as one block. Lining up a story weighs at most 1,000,000 pairs of
blocks between two unchanged ones (past it they pair in order) and splits it 64 times.
