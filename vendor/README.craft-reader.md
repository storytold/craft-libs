# Shared Hayro reader snapshot

The `hayro`, `hayro-interpret`, `hayro-syntax` and `hayro-jbig2` directories are transferred
from [PdfCraft revision ec0a965074f376a5bf610435948e4e5184bf3299](https://github.com/storytold/pdfcraft/tree/ec0a965074f376a5bf610435948e4e5184bf3299/vendor)
(`main`, 2026-10-19). The upstream [patch ledger](https://github.com/storytold/pdfcraft/blob/ec0a965074f376a5bf610435948e4e5184bf3299/vendor/README.md)
describes the inherited patches. Source manifests record every transferred Git blob and
SHA-256 alongside the local hash. The source manifests themselves are not self-hashed.

The first transfer was from PdfCraft revision 7437bde378f43856eada40af25b983812e615e8a.
The snapshot was then re-synced to ec0a965 by a three-way merge (base 7437bde, PdfCraft's
current tree, this tree), so every local patch described below is carried unchanged. The
re-sync brings these PdfCraft changes:

| PdfCraft PR | Crate | Change |
|---|---|---|
| #1691 | hayro-interpret | Patch 28: images with few possible pixel values convert each value once (spot-color images) |
| #1903, #3316 | hayro-syntax | CCITT images that declare `/Rows 0`, or fewer `/Rows` than `/Height`, decode to their height |
| #1954 | hayro-syntax | ASCII85 decoding stops at the stream's decode limit (here the per-document limit) |
| #1982 | hayro-syntax | `Page::base_dimensions` keeps a one-point floor after `/UserUnit` |
| #2027 | hayro-interpret | Reads a transparency group's `/I` and `/K`; a group that is neither isolated nor knockout composites directly onto its backdrop |
| #2132 | hayro | A soft mask fades a gradient fill |
| #1557 | hayro-interpret, hayro | Patch 29: devices are told where annotation drawing begins and ends |
| 13e3154 | hayro-interpret | Annex D.2 alias codes in standard, TrueType, Type 1 and Type 3 fonts read as the glyphs they draw |
| #2436 | hayro-interpret | Patches 31–33: faster sampled functions, image decodes reused across zoom levels, patch meshes interpolated once per grid point |
| #2812 | hayro-interpret | Patch 34: a per-thread tint-transform counter for deterministic tests |
| #2035 | hayro-syntax | Flate back-references are copied in runs |
| #3356 | hayro | A tiling pattern drawn once shows its cell at full scale |
| #3378 | hayro-syntax | Patch 20: spaces or tabs between `stream` and its line break are accepted |
| #3363 | hayro-syntax | A JP2 image in a color space the decoder refuses is drawn by its component count |
| #3412 | hayro-interpret | Type 3 glyph bounding boxes are transformed to device space before shading bounds are intersected |

PdfCraft's vendored `vello_common` patch (clip-intersection performance) is not part of this
reader snapshot.

`hayro-cmap` 0.1.0 is transferred from [PdfCraft revision a2ec27b4775f53c40cad3456936d9406611e3a8d](https://github.com/storytold/pdfcraft/tree/a2ec27b4775f53c40cad3456936d9406611e3a8d/vendor/hayro-cmap)
with its malformed-CMap patch (PdfCraft #3177): bad `bfchar` or `bfrange` entries, an
unresolvable `usecmap` and an unparseable tail are skipped instead of discarding the whole
CMap. Its tests `tests::bad_bfchar_entry_is_skipped` and `tests::unresolvable_usecmap_is_skipped`
come with it. One local patch: a binary `usecmap` counts against the 16-level nesting limit
as a text one does, so a self-referencing binary CMap stops instead of overflowing the stack.
Consuming workspaces need a `hayro-cmap` patch entry beside the other Hayro entries.

`hayro-postscript` 0.1.0 is the crates.io package (upstream revision
a2354aebaf86f5ab13e5fd6b6b0521306488bf9d), the PostScript token scanner `hayro-cmap` uses
to parse the CMap streams PDFs embed. One local patch: the scanner, which does not parse
dictionaries yet, skips runs of `<<` and `>>` in a loop and returns the next object, instead
of recursing once per delimiter, so a long run of `<` or `>` bytes no longer overflows the
stack. Tests: `object::tests::long_open_dict_run_does_not_overflow`,
`object::tests::long_close_dict_run_does_not_overflow`, `object::tests::dict_delimiters_are_skipped`
and `object::tests::nested_dict_delimiters_are_skipped`. Its `SOURCE.json` records the
registry hashes. Consuming workspaces need a `hayro-postscript` patch entry beside the other
Hayro entries.

Changes ported from PdfCraft pull requests that were open when they were ported, under the
same MIT OR Apache-2.0 license. Code comments keep the pull request's patch numbers, except
where noted below.

| PdfCraft PR | Author | Source commit | Files | Change |
|---|---|---|---|---|
| [#2770](https://github.com/storytold/pdfcraft/pull/2770) | Guillem Hermida (guillehr2) | [6fcf2a4](https://github.com/guillehr2/pdfcraft/commit/6fcf2a4aca29e943e28cb69017ff8f87d00656c6) | hayro-interpret `color.rs`, `interpret/mod.rs`, `interpret/path.rs`, `interpret/state.rs`, `pattern.rs`, `shading.rs`, `soft_mask.rs`, `x_object.rs` | Patches 35 and 36, plate mode: `InterpreterSettings::plate` draws one ink's separation as a gray film (`color::Plate`, `ColorSpace::ink_use`/`ink_tint`); the graphics state reads `OP`, `op` and `OPM` and an overprinting paint that doesn't specify the plate's ink leaves the plate as it is (ISO 32000-2 §8.6.7); `InterpreterSettings::ink_sink` reports spot inks; `color::CmykSimulation` converts process color to an output intent's CMYK. Only the vendored interpreter part is ported; PdfCraft's `separations.rs` and Output Preview UI are not |
| [#2974](https://github.com/storytold/pdfcraft/pull/2974) | mtlcl410 | [bae6981](https://github.com/mtlcl410/pdfcraft/commit/bae69819594a6385a027fec1f67e6b3c36c259b4) | hayro-interpret `device.rs`, `x_object.rs`; hayro `renderer.rs` | Knockout groups (`/K true`) knock out: while one is innermost, fills, strokes, glyph outlines and shadings are drawn with `Compose::Copy` (weighted by coverage); an unmasked image's rectangle (`Device::knockout_image`) and a nested group's shape (an erase pass between `Device::begin_knockout_erase` and `end_knockout_erase`) are cleared before their layer composites; a transparency group drawn in a knockout group always gets a layer of its own. Masked images and stencils composite as before |

Code comments label #2974's changes `Knockout patch (pdfcraft #2974)` instead of a number: the
pull request numbers them hayro-interpret patch 39 and hayro patch 19, and PdfCraft may
renumber them when it merges. `crates/pdf/tests/knockout.rs` adapts its test; a plate-mode
test in `crates/pdf/tests/plates.rs` checks that a knockout group separates with only the top
object where two objects overlap. The erase pass resolves the group's resources with
`Resources::from_scope`, as the group's own draw does in this tree.

A craft-libs patch, labeled `Craft patch (CMYK masks)` in code comments,
takes a luminosity soft mask whose group is DeviceCMYK from the group's inks: ISO 32000-2
§11.6.5.2 and §10.4.2.2 give `1 - min(1, 0.3c + 0.59m + 0.11y + k)`, where the colors as
drawn through the CMYK profile leave K 100% at about a tenth. hayro's `renderer.rs` draws
the group's four process plates over the backdrop's film (`SoftMask::plate`,
`SoftMask::interpret_plate`, `Plate::tint`) and applies the formula. On those plates spot
colorants paint through their alternate space (`Plate::spots_as_process`) and nothing
overprints. Masks in other color spaces are unchanged. A test in
`crates/pdf/tests/plates.rs` checks process, spot, RGB and backdrop colors.

The output-intent conversion of patch 36 goes through a `color::CmykConversion` trait, so
that callers can convert with their own color management. `CmykSimulation::new` keeps
#2770's moxcms conversion as the default, and `CmykSimulation::with_conversion` takes a
caller's. Lab and ICCBased colors reach the conversion as CIE XYZ relative to the D50 PCS
white (`color::PCS_WHITE`); the moxcms conversion takes them on through #2770's ProPhoto hub.
`crates/pdf/tests/plates.rs` adapts #2770's plate tests to the raster path, with its
synthetic ICC profiles in `crates/pdf/tests/support/synthetic_icc.rs`.

The shared snapshot adds immutable per-document ordinary decode limits, threaded through
initial xrefs, object streams, fallback and repair. Existing constructors retain their
previous default. Known-size image budgets and streaming content budgets remain distinct.
ContentReader now reports omitted or failed whole streams and incomplete Flate blocks as
known incomplete content. The interpreter also reports failed or budget-reaching nested
forms, patterns and Type 3 procedures. A stream exposes its immutable document limit so
callers can use the same conservative completion check. This diagnostic does not cover
structural repair or every omitted object and is not a rendering-completeness guarantee.
The syntax manifest pins JBIG2 exactly to 0.3.0 so a newer registry release cannot bypass
the transferred decoder patches.
Synthetic unit tests cover these boundaries; two version tests and the introductory doctest
use synthetic PDFs so they do not rely on the absent upstream corpus. No corpus is copied.

Compatibility patches follow the resource scopes and text-showing order in
[Hayro 0.8.0, revision ea9c81dc70cde40c64205986fdcfcd481fe934d8](https://github.com/LaurenzV/hayro/tree/ea9c81dc70cde40c64205986fdcfcd481fe934d8),
under the same MIT OR Apache-2.0 licence. Explicit page and nested-object resource
dictionaries replace the enclosing dictionary as a whole. Missing Form resources inherit
the caller; tiling patterns use only their own resources. Direct arrays and dictionaries
are limited to 64 levels in raw and decoded content. Page-tree traversal is iterative,
retains cycle detection and inheritance, and fails closed when its 100,000-entry work
budget or a fallible frame/page allocation fails. Text-showing operations fill the
whole run before stroking it, including strings separated by adjustments in `TJ`.
The implementation replays bounded source operands without allocating a glyph buffer.
The existing Type 3 single-paint behavior and incomplete-content diagnostics remain.
Raw array iteration now ends safely if validation and object parsing disagree;
standalone sign tokens retain the reader's zero tolerance.
Byte transfer-function samples use rounded 8-bit values; failed or non-finite evaluations
leave the sample unchanged. Historical source hashes remain in each `SOURCE.json`,
alongside local hashes and the compatibility reference.

The JBIG2 segment parser caps the long-form referred-to-segment count at `u16::MAX`
and rejects a larger claim as an invalid count before any retention-flag or
referred-to-number bytes are consumed, so a crafted header cannot turn
`Vec::with_capacity` into a multi-gigabyte reservation that aborts on allocation
failure.

Hostile-input bounds (CodeRabbit findings):

- An xref-stream subsection must be backed by its decoded bytes: all-zero `/W` widths
  are refused, and a declared count longer than the data is refused before any entry is
  inserted, so a tiny stream cannot declare billions of entries.
- Each object stream's `/N` header pairs are parsed once per document and cached beside its
  decoded data, instead of on every compressed-object lookup.
- An embedded JBIG2 page over 2^28 pixels is refused before the output buffer is allocated,
  and that buffer is reserved fallibly (`Bitmap::reinitialize` allocates before touching the
  existing bitmap, so a failed reservation leaves it intact). A `JBIG2Globals` stream with an image filter is
  ignored, so globals cannot recurse through further globals.
- At most 4096 graphics states, holding at most 64 MiB together, are saved; a `q` past
  either limit is counted but not copied, and its `Q` still removes the clips added since.
- An xref-stream `/W` type field wider than one byte is read at its full width (big-endian)
  instead of only its first byte, and `/W[0]` over 8 bytes is refused like `/W[1]`.
  A dash array over 256 entries strokes solid.
- An xref-stream subsection running past object number `i32::MAX` is refused instead of
  overflowing or turning negative.
- A CID font's `/W2` run that starts past CID 65535 is skipped as in `/W`, instead of
  overflowing and discarding the whole table.
- CCITT images: `ccitt_size_ok` (inherited PdfCraft ledger item 2) allows at most 2^20
  columns and 2^28 pixels (columns × rows) before the decoder allocates. A `/Columns` past
  `u32::MAX` is refused rather than wrapped to a small width. `/Rows` follows PdfCraft (17)
  and (19): `/Rows 0` or a value below `/Height` decodes to the image height, and decoding
  still stops where the data ends.

Reader limits from the [PdfCraft revision a2ec27b4775f53c40cad3456936d9406611e3a8d ledger](https://github.com/storytold/pdfcraft/blob/a2ec27b4775f53c40cad3456936d9406611e3a8d/vendor/README.md),
by its item ids: hayro-interpret (31) comes with the ec0a965 re-sync (patches 31–33 above):
its flat sample array computes the largest 32-bit Type 0 sample in 64 bits instead of
overflowing `2_u32.pow(32)`. hayro-syntax (7), (22) and (25) are covered by the local bounds
above: ASCII85 output stops at the decode limit, object-stream indexes are parsed once with
checked offsets, and direct objects nest at most 64 levels, stricter than PdfCraft's 128. Test:
`function::type0::tests::thirty_two_bit_samples_do_not_overflow`.

Fixes from the same [PdfCraft revision a2ec27b4775f53c40cad3456936d9406611e3a8d ledger](https://github.com/storytold/pdfcraft/blob/a2ec27b4775f53c40cad3456936d9406611e3a8d/vendor/README.md),
by its item ids, each marked `PdfCraft patch` in the source. The ec0a965 re-sync brings
hayro-syntax (17) and (19), CCITT `/Rows 0` or fewer `/Rows` than `/Height` decode to the
image height; (20), spaces or tabs between `stream` and its line break; the `base_dimensions`
one-point floor after `/UserUnit`; and in hayro-interpret (30), the Annex D.2 alias codes
(WinAnsi 0xA0 and 0xAD, MacRoman 0xCA) read as an ordinary space and hyphen unless
`/Differences` names them, and (P2), a Type 3 glyph's `/FontBBox` is taken to device space
before it bounds the glyph's clip and shadings. Ported from the ledger on top of it: in
hayro-syntax, the `EI` extension of (4), `EI` followed by a delimiter ends an inline image;
(23), bytes above 0x7F after an `EI` candidate don't reject it; the unledgered #3565 fix, a
doubled sign (`--280`) is one number in both read and skip, beside the local zero-tolerance
sign tokens; in hayro-interpret, (44), a `/ToUnicode /Identity-H` or `/Identity-V` name reads
two-byte codes as Unicode; (P6), glyph names outside the Adobe Glyph List (`f_i`,
`hyphen.case`, `uniXXXXXXXX`) read by the glyph list specification's full mapping; (P8), a
substitute without a spacing character draws its ordinary space; and the extension of (26),
comma-below letters (ș ț ķ ļ ņ ŗ Ģ) in the standard-font stand-ins are built from the font's
comma. The unit tests of all these items come with them; the CCITT, Type 3 and comma-below
raster tests are adapted in `tests/upstream_raster.rs` and the text contracts in
`tests/upstream_text.rs`. (P8)'s PdfCraft test needs a Japanese substitute font and is not
ported.
- A CCITT image with `/Rows 0` (unknown height) decodes at most the rows that fit the stream's
  output limit (counted in 1-bit rows) and 2^28 pixels; a known height is clamped the same way.
  Before, it decoded to the end of the data, and a bit of 2D code can repeat a whole row.
- A JPEG 2000 image over 2^28 pixels, or over its output limit (at least 64 MiB), by the
  codestream's own dimensions is refused before decoding, and its buffer is reserved fallibly:
  the image dictionary's smaller size no longer stands in for the size the decoder allocates.

Paint fixes from the [PdfCraft revision 0b670caf4a090e9d83a7f41a84b4cb2144a2d33e ledger](https://github.com/storytold/pdfcraft/blob/0b670caf4a090e9d83a7f41a84b4cb2144a2d33e/vendor/README.md),
each marked `PdfCraft patch` in the source: hayro-interpret (46) with its hayro counterpart
(PdfCraft #2977), a stencil mask is painted with the fill paint, not the stroke paint, at the
fill opacity once, and a pattern paint drops the opacity it captured at `scn`
(`Pattern::without_opacity`); the unledgered PdfCraft #3246, a shading pattern takes the alpha
current when it is painted (`Pattern::set_alpha`, also on a plate), so a `gs` after `scn` or
`SCN` still applies; (36), marked `PdfCraft patch (#3035)` because the plate-mode code uses
(35) and (36) for #2770's own numbers, a mesh pixel whose corner no triangle holds takes the
triangle covering its center (`Triangle::covers`), so shared edges leave no seam; and (51),
the saved-state byte and count sums saturate, so a huge dash array or clip list stays over
the bound on 32-bit targets instead of wrapping. The PdfCraft raster tests for (46), #3246 and
(36) are adapted in `tests/upstream_raster.rs`.

Font and text fixes from the [PdfCraft revision 0b670caf4a090e9d83a7f41a84b4cb2144a2d33e ledger](https://github.com/storytold/pdfcraft/blob/0b670caf4a090e9d83a7f41a84b4cb2144a2d33e/vendor/README.md),
each marked `PdfCraft patch` in the source: hayro-interpret (49) (PdfCraft #2949 and its
follow-up), `OpenTypeFontBlob::new` accepts a `post` table version 1.0, 2.0 or 3.0 written
with its 16-bit halves swapped (`00 00 00 03`, as ScanSoft PDF Create! embeds every font), and
`glyph_names` reads such a table from a copy with the version written correctly, so a 2.0
table's names serve `/Differences`; any other unknown version is still refused; (50)
(PdfCraft #3958), `CffFontBlob::glyph_index_by_name` looks names up in a map built on first
use and shared between clones instead of searching the charset for every character code, and
the first glyph with a name still wins; and the unledgered PdfCraft #3116 (#3109), marked
`PdfCraft patch (#3116)`, text in mode 7 (clip only) installs its clip and reaches the device
as `GlyphDrawMode::Clip`, which hayro does not paint, so extraction that leaves out invisible
OCR text (mode 3) keeps a heading drawn through its clip. The unit tests
`font::blob::tests::{a_post_version_with_swapped_halves_is_read_as_the_version_it_means,
glyph_names_resolve_to_the_first_glyph_with_that_name}` come with (49) and (50); the #3116
contract is in `tests/upstream_text.rs`.

Fixes from the [PdfCraft revision 0b670caf4a090e9d83a7f41a84b4cb2144a2d33e ledger](https://github.com/storytold/pdfcraft/blob/0b670caf4a090e9d83a7f41a84b4cb2144a2d33e/vendor/README.md),
by its item ids, each marked `PdfCraft patch` in the source. hayro-syntax (24), PdfCraft
#2400: `xref::prev_offset` reads a trailer `/Prev` written as a reference `N G R` as the
offset that object holds when the section's own entries place it (iText's
`indirectPrev.pdf`), and otherwise as offset N (tiff2pdf), so the earlier sections' pages
are seen. Each section's entries are read before its `Prev` is followed; the local
xref-stream bounds above apply to them unchanged. Tests:
`xref::tests::prev_as_reference_reaches_the_earlier_section` and
`xref::tests::prev_as_reference_to_an_object_holding_the_offset_reads_that_offset`.
hayro-syntax (P5), PdfCraft #3184: `filter::jpx::decode` applies a JP2 palette (`pclr`) when
the image dictionary has no `/ColorSpace`, or one that does not resolve, since the colors then
come from the JP2 header (ISO 32000-2 §7.4.9). With a `/ColorSpace` the raw indices are kept,
as an `/Indexed` space needs. Resolved palette colors are 8-bit samples whatever the
precision of the indices; an image without a palette keeps its own. Tests:
`filter::jpx::tests::jpx_palette_is_applied_without_a_color_space`,
`filter::jpx::tests::jpx_palette_indices_stay_raw_for_an_indexed_color_space` and
`filter::jpx::tests::palette_colours_are_eight_bit_and_other_samples_keep_their_precision`.
PdfCraft's raster test `a_jp2_palette_keeps_its_colours_whatever_the_index_precision` is not
ported: the unit tests show the same behavior.

From [PdfCraft revision 0b670caf4a090e9d83a7f41a84b4cb2144a2d33e](https://github.com/storytold/pdfcraft/tree/0b670caf4a090e9d83a7f41a84b4cb2144a2d33e/vendor),
unledgered and marked `PdfCraft patch` in the source: hayro-syntax #3438 (issue #3107), with
its look-ahead bound from 01af6cba3. An inline image without a filter ends at the byte
length its `/W`, `/H`, `/CS`, `/BPC` and `/IM` give (rows padded to whole bytes) when white
space and `EI` follow there, so an `EI` inside its samples does not end it. Conflicting
long and short keys, filters and other color spaces keep the `EI` look-ahead. When the
computed end lies past the content window, the tokenizer asks for more data only while the
image, its header and the terminator fit within `MAX_DECODED_STREAM`, and otherwise falls
back to the look-ahead. Its unit tests in `content::tests` come with it. PdfCraft's raster
test is not ported: the windowed tokenizer tests cover the same content streams.

Rendering fixes from the [PdfCraft revision 0b670caf4a090e9d83a7f41a84b4cb2144a2d33e ledger](https://github.com/storytold/pdfcraft/blob/0b670caf4a090e9d83a7f41a84b4cb2144a2d33e/vendor/README.md),
each marked `PdfCraft patch` in the source: hayro-interpret (40) (PdfCraft #2987), an optional
content membership dictionary with a visibility expression (`/VE`: `/And`, `/Or` and `/Not`
over groups the document lists in `/OCProperties /OCGs`) shows its content when the
expression is true, ahead of `/OCGs` and `/P`; a malformed expression, or one nested deeper
than 32 levels, is ignored and `/OCGs` and `/P` decide; and hayro (18) (PdfCraft #2972), the
page renders as one transparency group over a transparent backdrop that is then put on the
background, so a blend mode over an empty part of the page keeps its color; with a color
replacement (reading colors) the background stays inside the group as its backdrop. A plate
render (`InterpreterSettings::plate`) also keeps the background inside, marked `Craft patch`,
so blend modes on a separation's film blend with the paper. The PdfCraft raster tests of both
are adapted in `tests/upstream_raster.rs`.

Default color spaces from the [PdfCraft revision 0b670caf4a090e9d83a7f41a84b4cb2144a2d33e ledger](https://github.com/storytold/pdfcraft/blob/0b670caf4a090e9d83a7f41a84b4cb2144a2d33e/vendor/README.md):
hayro-interpret (38) (PdfCraft #3018), marked `PdfCraft patch (#3018)` because the plate-mode
code labels #2770's changes by that pull request's own patch numbers. The `/DefaultGray`,
`/DefaultRGB` and `/DefaultCMYK` color spaces of the resources in scope (ISO 32000-2
§8.6.5.6) replace the device space that `g`/`G`, `rg`/`RG`, `k`/`K` and `cs`/`CS` select (by
name, or through a `/ColorSpace` resource that is a device space), and the device
`/ColorSpace` of an image XObject, an inline image or a shading painted by `sh` or as a
shading pattern (`DefaultColorSpaces`, looked up once per content window). A default that is
a device, Lab, Indexed or Pattern space, or has the wrong number of components, is ignored.
Decoded images and shadings are cached per default. Two local differences: a form XObject
with `/Resources` of its own sees only its own defaults, as it sees only its own other
resources here (`Resources::from_scope`), where PdfCraft falls back to the page's; and on a
plate (patch 35) no default applies, so DeviceCMYK still separates as its components with
the nonzero overprint mode, and DeviceGray on black alone, rather than through a DefaultCMYK
or DefaultGray profile and the CMYK simulation of patch 36. PdfCraft's eight raster tests are
adapted in `tests/upstream_raster.rs` (`default_color_spaces`); the plate case is
`default_color_spaces_leave_device_color_on_its_plates` in `tests/plates.rs`.

A rendering speedup from the [PdfCraft revision 0b670caf4a090e9d83a7f41a84b4cb2144a2d33e ledger](https://github.com/storytold/pdfcraft/blob/0b670caf4a090e9d83a7f41a84b4cb2144a2d33e/vendor/README.md),
hayro-interpret (35) with hayro (19), PdfCraft #3296, marked `PdfCraft patch (#3296)` because
the plate-mode code uses (35) for #2770's own numbers. `EncodedShadingPattern::extent` is the
device-space rectangle of the pixels a mesh shading's triangles touched when it was encoded
(`None` for function-based, axial and radial shadings, and for a mesh with a `/Background`).
`Renderer::set_paint` returns `PaintSetup`: a mesh texture covers that extent (and a
transparent pixel around it, padded instead of repeated) rather than the whole path, and a
mesh with no triangle on the canvas returns `PaintSetup::Nothing`, which the fill, stroke,
rect and stencil callers skip. The rendered output is the same. The knockout erase pass
returns before the extent is used, so it still clears under the whole path. Tests:
`mesh_extent_tests::{a_mesh_paints_its_triangles_alone, a_mesh_keeps_the_graphics_state_alpha,
a_mesh_off_the_canvas_paints_nothing}` in hayro.

An ICC profile cache fix from the [PdfCraft revision 0b670caf4a090e9d83a7f41a84b4cb2144a2d33e ledger](https://github.com/storytold/pdfcraft/blob/0b670caf4a090e9d83a7f41a84b4cb2144a2d33e/vendor/README.md):
hayro-interpret (48) (PdfCraft #3020), marked `PdfCraft patch (#3020)`. The color space read
from an `/ICCBased` profile stream is cached by `IccProfileKey`, not by the stream
dictionary's bytes alone, so two profiles with the same `/N` and `/Length` no longer share
whichever was read first. A stream object is looked up by its object identifier without
reading it, and on a miss by its dictionary key and raw (decrypted, undecoded) bytes, so
copies of a profile are read once; a stream written directly in its array has only the
content key. `InterpreterCache::icc_profile_reads` counts the reads. The CMYK simulation of
patch 36 doesn't use this cache: it is built from the output intent's bytes by the caller,
and the digest that tells its profile from a document profile is still a hash of the
decoded profile bytes. PdfCraft's two raster tests are adapted in `tests/upstream_raster.rs`
(`icc_profile_cache`).

Local fixes on top of these ports, marked `Craft patch` or in the comments of the ported
code: an inflated patch-mesh triangle is rebuilt with `Triangle::new`, so its stored dot
products match its vertices (moving the points alone skewed barycentric coordinates by about
5 %, in `covers` and in interpolation); a stencil image puts the fill alpha back itself, since
`restore_state` doesn't when the saved-state bound skipped the save; and a resolved JP2
palette decodes to 8 bits whatever `/BitsPerComponent` the dictionary gives.

This remains the patched 0.7 backend. It retains the existing Bradford Lab conversion,
floating component precision and codec versions, rather than claiming full 0.8
equivalence. The independent color oracles, backend tests and facade contracts cover
these retained policies separately from the compatibility changes.

All other source and asset files remain unchanged. Upstream licence texts, Foxit/PDFium
standard-14 fonts and the CMYK ICC profile are retained. Attribution is in the root NOTICE
and ATTRIBUTION.md. The removed LAB.icc profile remains absent. Lopdf is not part of this
snapshot: applications that need its editing or inspection compatibility keep owning it.

Root Cargo patch entries are required in consuming workspaces. The writer-only feature
remains independent; vendoring reader sources does not make them a writer dependency.

The transferred snapshot includes the latest Lab Bradford adaptation, Type 3 rendering-mode
handling and filtered shrinking of oversized soft masks. `tests/upstream_raster.rs` adapts
the corresponding portable PdfCraft rendering contracts for native and WASM.
The snapshot also preserves PdfCraft #1477 (WinAnsi Unicode fallback for TrueType
fonts without Encoding) and #1499 (byte-aligned packed TIFF predictor rows).
Synthetic tests cover 1/2/4/8-bit RGB rows and rendering, plus embedded TrueType
text with absent or partial ToUnicode maps and explicit encoding precedence.

## Craft patches for the flattener

Local changes for craft-prepress's PDF-in adapter (`craft_prepress::flatten::pdf`), which reads
pages through a `Device` that keeps colors as inks and embeds the source fonts again. Code
comments label them `Craft patch (flatten)`.

| Files | Change |
|---|---|
| hayro-interpret `font/outline.rs`, `font/cid.rs` | `OutlineFontData::embedded`: true when the bytes come from the PDF's `FontFile*`, false for a non-embedded CID font's substitute from the font resolver |
| hayro-interpret `font/outline.rs`, `font/cid.rs`, `font/blob.rs` | `OutlineFontData::index`: the face index of the bytes in a font collection, as the resolver returned it |
| hayro-interpret `font/outline.rs`, `font/type1.rs`, `font/mod.rs` | `OutlineGlyph::font_data` returns the bare CFF bytes of a simple `FontFile3/Type1C` font, with `embedded: true`; its glyph ids index the CFF. The descriptive fields (`postscript_name`, `weight`, flags) are left empty |
| hayro-interpret `device.rs` | `Device::set_overprint(fill, stroke, mode)`: the `op`, `OP` and `OPM` for the paints that follow. `Device::set_isolated`: the group just pushed is isolated (`/I true`). Both default to no-ops |
| hayro-interpret `interpret/path.rs`, `interpret/text.rs`, `x_object.rs` | `set_overprint` is called where `set_blend_mode` is for fills, strokes (the `sh` operator included), glyphs and images; `set_isolated` after an isolated group is pushed |
| hayro-interpret `color.rs` | `Color::ink_paint`: the inks a color paints, as plate mode separates it (`color::InkPaint`). `Color::spot_alternate`: a spot ink at 100% (`color::SpotAlternate`), CMYK when the alternate space is DeviceCMYK. `Color::opacity` and `Color::is_device_cmyk` |
| hayro-interpret `color.rs` | `color::InkSpace`, `ColorSpace::ink_space` and `ColorSpace::ink_values`: how a space's colors are kept as inks (gray, CMYK, its colorants, `All`, `None`) and colors converted in batches, process color as `Color::ink_paint` converts it. `ColorSpace::components`; `ColorSpace::is_device_cmyk` and `ColorSpace::spot_alternate` public |
| hayro-interpret `types.rs`, `x_object.rs` | `RasterImage::with_inks`: an image's samples in its own space's inks (`InkImage`), `/Decode` and `/Matte` applied, transfer functions left out, with its alpha at the alpha's own size |
| hayro-interpret `interpret/mod.rs` | The `sh` operator drops the shading's `/Background`, which only a shading pattern paints (ISO 32000-2 §8.7.4.5.1) |
