# craft-rtf

Rich Text Format, read and written. It replaces the RTF code each Craft app carried: the
tokenizer, code pages, `\u` handling, dates and escaping come from WordCraft's reader and
writer; the small model covers what DesignCraft's text import, DeckCraft's outline import and
export and PdfCraft's export read and write.

```rust
use craft_rtf::{Block, CharFmt, Document, Para, Run, read, write};

let bold = CharFmt { bold: true, ..CharFmt::default() };
let doc = Document {
    blocks: vec![Block::Para(Para {
        runs: vec![Run { text: "Plan {Q3} ".into(), fmt: bold }, Run { text: "café 😀".into(), fmt: CharFmt::default() }],
        outline_level: Some(0),
        ..Para::default()
    })],
    pictures_left_out: 0,
};
let rtf = write(&doc, "Liberation Sans");
assert!(rtf.contains(r"\outlinelevel0 {\b Plan \{Q3\} }caf\u233?"));
assert_eq!(read(rtf.as_bytes()).unwrap(), doc);
```

## Layers

| Layer | What it is | For |
| --- | --- | --- |
| `Tokens` | The tokens of a file: `{`, `}`, control words with their parameter, `\*`, `\'hh`, control symbols, text runs, `\bin` data. | Any reader. |
| `TextDecoder`, `codepage` | 8-bit text in a code page, `\uN` and its fallback characters. | Any reader. |
| `esc`, `esc_font`, `date_words`, `iso_to_dttm` | Escaping and dates. | Any writer. |
| `read`, `write`, `Document` | A file to and from a small model. | An app that needs text with basic formatting. |

A reader that keeps more than the model holds (styles, lists, fields, notes, tracked changes,
pictures) uses the first two layers and its own interpretation of the control words.

## Text

- **Code pages.** 8-bit text is in the current font's code page (`\fcharset`, `\cpg`), else
  the default font's (`\deff`), else the document's (`\ansicpg`, `\mac`, `\pc`, `\pca`), else
  Windows-1252. Single-byte: Windows 874 and 1250 to 1258, DOS 437, 737, 850, 852, 857, 866,
  Mac OS Roman. Multibyte: 932 (Shift-JIS), 936 (GBK), 949 (UHC), 950 (Big5), streamed through
  one decoder so a lead byte and its trail byte decode together whatever falls between them.
  Raw bytes above 0x7F are text in the code page, as `\'hh` is. They are not read as UTF-8.
- **`\uN`.** One UTF-16 code unit, taken modulo 65,536. A high surrogate pairs only with a low
  surrogate in the very next `\u`; a surrogate that does not pair is U+FFFD, as in Word. The
  `\ucN` characters after a `\u` are its fallback and are skipped: a text byte, a `\'hh`, a
  control word and a control symbol each count as one, a following `\u` does not, and the
  skipping ends with the group.
- **Malformed input.** `\'` without two hex digits is dropped with the hex digits it has; a
  `}` or `\` after it keeps its meaning. A control word is at most 32 letters and a parameter
  at most 10 digits. A parameter outside `i32` is clamped.

## The model

`Document { blocks, pictures_left_out }`, `Block::{Para, Table}`, `Para { runs, outline_level,
left_indent, right_indent, first_line_indent, space_before, space_after, align }`, `Run { text, fmt }`, `CharFmt { bold, italic, underline, strike,
vert_align, half_points, color, font }`, `Table { rows: Vec<Vec<Cell>> }`, `Cell { right, paras }`.

In `Run::text`, a tab is `\t`, and `LINE_BREAK` (`\n`), `PAGE_BREAK` (U+000C) and
`COLUMN_BREAK` (U+000E) are `\line`, `\page` and `\column`. Lengths are in twips and sizes in
half-points, as the file has them.

`read` leaves out headers, footers, footnotes, comments, field instructions (a field's result
is kept), list labels (`\listtext`, `\pntext`), the stylesheet, pictures and objects (counted),
and every destination marked `\*`. Paragraph properties end with their group, as character
properties do. A trailing `\par` does not leave an empty last paragraph; empty paragraphs
elsewhere are kept. `\sect` ends a paragraph.

### Limits

| Limit | Value | Past it |
| --- | --- | --- |
| `MAX_GROUPS` | 512 nested groups | deeper groups are read in the state of the group around them |
| `MAX_FONTS` | 4096 font table entries | later fonts are not kept |
| `MAX_COLORS` | 4096 colour table entries | later colours are not kept |
| `MAX_FONT_NAME` | 256 bytes | the font is not kept |

The reader does not recurse, and what it returns is never larger than a small multiple of the
file: each character of text costs the file at least one byte.

## Outlook bodies (feature `outlook`)

`outlook::decompress_rtf` undoes the compression of `PidTagRtfCompressed` ([MS-OXRTFCP]: `LZFu`
with its CRC checked, or `MELA` stored as is), never allocating past the caller's cap.
`outlook::rtf_text` gives the text a mail body shows (font charsets and `\ansicpg` decoded through
`craft_mime::charset`, `\uN` with its fallback skipped, destinations dropped), and
`outlook::rtf_html` the HTML of an encapsulated-HTML body ([MS-OXRTFEX], `\fromhtml1`). Groups nest
at most `MAX_RTF_DEPTH` (256) deep. From MailCraft `crates/pst/src/rtf.rs` at
[a44a450](https://github.com/UCS-Care-Group/mailcraft/commit/a44a45013d1c6396229c40ab60172d1882b368d5),
by Martin Ball and the MailCraft contributors.

## The word model (`doc` feature)

`craft_rtf::doc` reads RTF into the word model (`craft_wordmodel::Document`) and writes it
back, through the text formats' flow (`craft_flow`): styles, lists, fields, bookmarks,
pictures (PNG, JPEG, metafiles; other formats re-encoded as PNG within a bounded decode),
tables, notes, comments, tracked changes, sections, headers and footers. It is WordCraft's RTF
reader and writer, on this crate's code pages, dates and escaping.

- `doc::import(bytes) -> Result<Document, String>`, `doc::parse(bytes) -> Result<Flow, String>`
- `doc::export(&Document) -> String`

Limits: group nesting (512), picture data (64 MiB a picture), notes (5,000; 500 blocks each),
sections (10,000), revision authors (10,000), fonts (4,096).

## Not here yet

`doc` keeps its own tokenizing loop, which has rules the shared `Tokens` does not (`\bin` data
and fallback characters inside picture data, the destinations a `\uN` fallback is skipped in).
Moving it onto `Tokens` is a follow-up.
