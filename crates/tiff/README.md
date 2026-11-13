# craft-tiff

The TIFF / IFD reader and writer underneath raw decoding (DNG, CR2, NEF, ARW…) and Exif. Moved from
LightCraft's `lightcraft-tiff` as the first step of shared RAW support (craft-libs#783, #587). It depends
on no other Craft crate.

## Features

- Classic TIFF and BigTIFF, little- and big-endian (`II`/`MM`). Also accepts the TIFF-like magic numbers of
  some raw formats (ORF `IIRO`/`IIRS`, RW2 `IIU`) when `ParseOptions::accept_raw_magic` is set (default).
- Every field type: BYTE, ASCII, SHORT, LONG, RATIONAL, SBYTE, UNDEFINED, SSHORT, SLONG, SRATIONAL, FLOAT,
  DOUBLE, IFD, LONG8, SLONG8, IFD8.
- IFD chain + `SubIFDs` (330) + Exif (34665) / GPS (34853) / Interoperability (40965) pointer IFDs, resolved
  into a tree (`Ifd::sub_ifds`, `exif`, `gps`, `interop`). `Entry::offset` records the absolute position of
  each value (needed for maker notes and embedded blobs).
- `Tiff::parse_with_status` returns the best-effort tree with a completeness flag for callers that must
  distinguish a clean absence from an IFD, entry, or pointer that could not be inspected.
- `parse_ifd_at(data, offset, order, base, bigtiff, opts)` parses an IFD whose offsets are relative to an
  arbitrary base; `makernote::parse_makernote` knows the header / byte-order / offset-base conventions of
  Canon, Nikon (types 1 and 3), Sony, Fujifilm, Olympus (old and new), Panasonic and Pentax notes.
- `image::ImageInfo` — width/height, bits, samples, compression, photometric, planar config, predictor,
  sample format and the strip or tile chunk list (`chunks()`, `chunk_bytes()`); missing byte counts are
  inferred.
- `directory_entries`, `entry_span`, `ParseOptions::budget_for` and `decode_value` are the reader's own rules
  (directory layout and entry limit, which entries it reads, its value budget, value decoding), for callers that
  must select entries exactly as `parse_ifd_at` would without decoding the whole directory (e.g. a raw decoder's
  Sony `SR2Private` lookup).
- `TiffWriter` writes IFD chains with strips or tiles, `SubIFDs` and pointer IFDs, in either byte order,
  classic or BigTIFF; `writer::rational` / `srational` approximate floats as rationals.

## Robustness

The reader never panics on malformed input (proptests feed random and mutated files): offsets are
bounds-checked; out-of-range or unknown-type entries are skipped; IFD loops are detected; IFD count,
nesting depth and entries per IFD are limited; at most 65,536 directory entry slots
(`ParseOptions::max_entries_total`) are accepted across the main IFD chain and all child IFDs, even when tables
overlap at distinct offsets, and an IFD past that bound ends the chain or is skipped like any other bad IFD after
the first; a next-IFD pointer is followed only when every declared entry was read, with the counts compared as
64-bit numbers (a BigTIFF declaring 4,294,967,297 entries but holding one can't pass for a whole directory on a
32-bit target); the total bytes of decoded values are capped at
`8 × input + 1 MiB` so a file cannot make us allocate far more than its own size. `parse_with_status` reports
truncated tables, skipped values, unresolved pointers and incomplete IFD chains without changing the parser's
best-effort tree.

## Sources

Public specifications only: TIFF 6.0 (Adobe, 1992), the TIFF 6.0 supplements (IFD type, SubIFDs), the
BigTIFF format description (AWare Systems), Adobe DNG Specification 1.7 (tag numbers), CIPA DC-008 Exif 2.32,
and the ExifTool tag-name documentation (maker-note header conventions, prose only). No third-party code
was consulted.
