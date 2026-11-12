# craft-eps

Encapsulated PostScript files as a container: what a file is, where its PostScript and its
previews are, and what its header comments say, without running its program. It depends on
nothing.

| | |
|---|---|
| `sniff`, `is_eps` | EPS (binary header, or `%!PS-Adobe-… EPSF-…`), plain PostScript or PDF, and whether it is an Illustrator file: PostScript-based (`%AI…` comments, or Illustrator as `%%Creator`) or PDF-based (the `AIPrivateData` / `AIPDFPrivateData` keys near the start or end of the PDF, a hint only) |
| `sections` | The parts behind the binary header ("DOS EPS"): the PostScript, the TIFF preview, the Windows metafile preview. A header that points outside the file is an error |
| `wrap` | PostScript and its previews behind a binary header, as `sections` reads them back |
| `Comments::read` | `%%BoundingBox` and `%%HiResBoundingBox` (the first in the header; `(atend)` taken from the trailer, outside embedded documents), `%%Pages`, `%%LanguageLevel`, `%%Creator`, `%%Title` |
| `epsi` | The device-independent preview (`%%BeginPreview`, 1, 2, 4 or 8 bits) as an 8-bit grey bitmap |
| `ascii85::encode`, `ascii85::decode` | The ASCII85 data encoding; lines never start with `%`, decoding is bounded |

```rust
let parts = craft_eps::sections(bytes)?;
let comments = craft_eps::Comments::read(parts.postscript);
if let Some([x0, y0, x1, y1]) = comments.page_box() { /* points */ }
```

## What it is not

There is no PostScript interpreter here, and no writer of drawings. This crate is the layer every
EPS reader and writer in the Craft apps had its own copy of; what the program draws is read and
written elsewhere:

- VectorCraft's `crates/eps` runs the program into its own document and writes EPS, PostScript
  print jobs and the legacy Illustrator format from it. That code speaks VectorCraft's document
  model throughout and has not moved.
- A PostScript interpreter that returns what a file paints as a PDF is proposed in
  craft-libs#159 (`craft-ps`, from DesignCraft).
- Decoding the TIFF or metafile preview is the caller's (`craft-metafile` reads the latter).
- The editing copy of the art inside a PDF-based Illustrator file is in the PDF's `PieceInfo`,
  which a PDF reader finds.

## Where it came from

The Craft apps' own code (MIT OR Apache-2.0), merged so that no caller loses a case:

- the binary header reader and writer, the EPSI preview and ASCII85 from VectorCraft
  (`crates/eps/src/lib.rs`, `import/preview.rs`, `ps.rs` at `6e873b80`);
- the header comments from DesignCraft (`crates/images/src/eps.rs` at `513f77b8`: bounded header
  and trailer windows, `(atend)` boxes outside embedded documents, boxes of a finite size) and
  VectorCraft (`import.rs`: `%%Pages`, the Illustrator markers);
- the same header and box reading also existed in EffectCraft (`crates/pdf/src/ps/eps.rs`) and,
  as a signature check, in PhotoCraft (`crates/io/src/vector_preview.rs`).

## Clean room

Written from Adobe's published specifications only: the PostScript Language Document Structuring
Conventions 3.0 (Technical Note 5001), the Encapsulated PostScript File Format Specification 3.0
(Technical Note 5002, which defines the binary header and the EPSI preview), and the PostScript
Language Reference, third edition (ASCII85). The Illustrator markers are the header comments the
Adobe Illustrator File Format Specification, version 7.0 (1998), documents (listed by PRONOM at
<https://www.nationalarchives.gov.uk/PRONOM/fmt/423>), and the `PieceInfo` key names PDF-based
files carry in plain sight. No Adobe application was run or used as an oracle, no Adobe prolog or
other Adobe-authored PostScript is included, and no GPL, LGPL or AGPL code (Ghostscript, Inkscape,
Scribus…) was read. The test files are written by hand in the tests.

## Limits

The header is read from the first 64 KiB of the PostScript and a deferred bounding box from the
last 64 KiB (`MAX_HEADER`, `MAX_TRAILER`); a bounding box is four finite numbers that make a box
of a finite size; the binary header's offsets are checked against the file; an EPSI preview is
at most 8192 pixels a side and must have all its samples; `ascii85::decode` takes the most bytes
it may produce; a PDF is scanned for the Illustrator keys in its first and last mebibyte
(`MAX_PDF_SCAN`). Each limit has a test. No `unsafe`, no panics outside tests.
