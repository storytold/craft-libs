# craft-pub

Reader for Microsoft Publisher 2002 and later files (`.pub`). The file is a compound file
([`craft-cfb`](../cfb/README.md)) holding a `Contents` stream (pages, masters, objects, palette), a
Quill text store and Escher drawing records. This crate decodes the `Contents` structure, the Quill
text and its runs, the Escher drawing records and table geometry into plain data. It has no
document model: mapping to pages, frames and styles belongs to the application (DesignCraft keeps
it). Import only.

- Detection lives in [`craft-file-formats`](../file-formats/README.md) (`Format::Publisher`, behind
  its `publisher` feature), which reads the first bytes of the root `Contents` stream through
  `craft_cfb::CompoundFile::read_path_prefix`.
- Every size and count read from the file is checked; recursion and work are bounded. Errors carry a
  stable `ErrorCode` (`NotPublisher`, `Unsupported`, `Truncated`, `Malformed`).
- Publisher 97, 98 and 2000 files use a different layout and are outside this crate.

## What is and is not decoded

Decoded: page size, page order, masters and the objects on each page, text links, the colour palette,
Quill text with its character and paragraph runs and default styles, Escher shapes, groups, fills,
lines, picture records, and table dimensions, cell rectangles and cell text boundaries. Fields no
sample explains are not guessed; the importer reports them.

## Provenance

The code moved here from the DesignCraft Publisher importer (storytold/designcraft pull request
2126), unchanged in behaviour. Its provenance record is `docs/publisher-import.md`, section
"Provenance", and `docs/pub-format/` in that change:

- The importer, its tests and the format notes were written by AI coding assistants working under the
  direction of the author, srt8hellcat2-ui, from the bytes of open sample `.pub` files and the public
  Microsoft specifications (MS-CFB, MS-ODRAW). No other Publisher reader was used, and no
  third-party reader's source or behaviour description was read by anyone working on it.
- The `contents`, `quill` and `read` modules were rewritten from the format notes alone by a
  different assistant than the one that wrote the notes; the notes' author had been allowed to read an
  earlier reader of the same project (recorded in that change).
- No Publisher-authored file is committed. Tests that need samples are in the application.

A maintainer has not yet confirmed that this provenance is sufficient; that decision belongs to
the maintainers of storytold/designcraft and this repository.
