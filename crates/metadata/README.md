# craft-metadata

Photo metadata: EXIF, IPTC-IIM, XMP packets and GPX track logs, the blocks that carry them in image
files, and one `Metadata` model they all read into. From LightCraft (`lightcraft-meta`). TIFF
structure comes from `craft-tiff`.

One module per format; they share `Metadata`, `DateTime`, `Gps` and `Orientation`, which is why they
are one crate.

| Function | Purpose |
|---|---|
| `read_exif(&[u8]) -> Metadata` | A TIFF-structured Exif block: a whole TIFF/DNG/raw file, or a JPEG APP1 payload with or without `Exif\0\0`. `try_read_exif` reports header errors, `try_read_exif_with` takes `craft_tiff::ParseOptions`. |
| `from_tiff(&Tiff) -> Metadata` | The same from an already parsed stream. |
| `write_exif(&Metadata) -> Vec<u8>` | An Exif block for an exported file. |
| `tags::tag_rows`, `tags::file_tag_rows` | Every tag with its name and value as text, maker notes included. |
| `focus::read`, `focus::read_header` | Where the camera focused, from the maker note (Sony), on the upright photo. `read_header` reads only the first kilobytes of a file through a callback. |
| `recipe_from_exif`, `recipe_from_bytes`, `read_recipe_file` | The Fujifilm in-camera recipe (film simulation, grain, colour chrome, tone, white balance shift and the rest) from the maker note of an Exif block, a whole TIFF, JPEG or RAF, or a file read only as far as its metadata. |
| `parse_iptc(&[u8]) -> Metadata` | IIM record 2: title (2:5), keywords (2:25), by-line (2:80), copyright (2:116), caption (2:120), date and time created (2:55, 2:60). |
| `parse_xmp(&str) -> Result<XmpData>` | `Metadata`, every property (`prefix:name`, structs flattened as `a:b/c:d`) and the structured values. `parse_xmp_with` also canonicalises the caller's own namespaces. |
| `write_xmp(&Metadata) -> String` | A complete `<?xpacket?>` packet. `write_xmp_with` sets the toolkit name and adds simple properties in the caller's namespace. |
| `XmpData::has_location_metadata`, `XmpData::properties_truncated` | Whether a packet names GPS or a place anywhere (array items and repeated structs included), and whether the flattened map is complete, for a caller that must prove a file carries no location. `parse_gps_position` reads an XMP coordinate pair. |
| `embedded_xmp(&Embedded)`, `parse_xmp_pair` | A JPEG's standard XMP with the Extended XMP it links (`xmpNote:HasExtendedXMP`) read as one packet; an `EmbeddedXmpError` (keeping the standard packet) when the link or the extension cannot be read whole. `extract` and `from_embedded` use it best-effort. Extended XMP: 16 MiB, 4096 chunks, only the linked GUID's chunks counted. |
| `merge_xmp(existing, fresh, MergeRules)` | Updates a packet another program wrote: owned properties are replaced, everything else stays byte for byte. |
| `parse_gpx(&str) -> Result<Tracklog>` | GPX track points with their track and segment; `Tracklog::locate` gives the position at a capture time, `Tracklog::only_track` one track. |
| `embedded(&[u8]) -> Embedded` | Sniffs JPEG, PNG, WebP, TIFF-based files, Fujifilm RAF, Canon CR3 and JPEG XL containers and returns the Exif, XMP (and extended XMP), ICC and IPTC blocks. `jpeg_segments`, `png_chunks`, `webp_chunks` are the walkers. |
| `cr3::parse_cr3` | The Canon CR3 container: `CMT1`…`CMT4`, the XMP `uuid` box, the thumbnail, the tracks. |
| `jxl::metadata` | The `Exif` and `xml ` boxes of a JPEG XL container, plain or Brotli-wrapped in `brob`. |
| `edit::upright_exif`, `edit::upright_xmp`, `edit::exif_with_resolution`, `edit::xmp_with_resolution`, `edit::export_exif`, `edit::export_xmp` | Byte-preserving edits for export: Orientation set to 1, the resolution set (in place, malformed entries removed, or appended when missing), the source thumbnail dropped. Every other byte stays. `edit::exif_orientation`, `edit::exif_resolution`, `edit::xmp_resolution`, `edit::photoshop_resolution` and `edit::exif_color_space` read the same values with the editors' strict rules. |
| `mpf::images`, `mpf::write` | The Multi-Picture Format index (CIPA DC-007) of a JPEG: the images stored after the primary one (a gain map, a preview), and writing an index plus appending the images. `mpf::markers` lists the marker segments before the first scan. |
| `extract(&[u8]) -> Metadata` | All of it for a whole file. Precedence: EXIF, then IPTC fills gaps, then XMP overrides the user fields (rating, label, title, caption, artist, copyright, keywords, GPS) and fills the rest. `from_embedded` does the same for blocks already located. |

HEIF, PSD and MRW files are read by `craft-heif`, `craft-psd` and `craft-raw`. Put the blocks they
return into an `Embedded` and call `from_embedded`.

## Validation

Only well-formed values become typed metadata; the raw property stays in `XmpData::properties`. XMP orientation must be an integral code 1 to 8; ISO and pixel dimensions exact whole numbers in range (compared as decimal digits); GPS minutes and seconds under 60, the sign from the axis letter. IPTC Time Created counts only as a whole `HHMMSS` or `HHMMSS±HHMM`. A carriage return in written text is `&#13;`.

## Capture time

EXIF candidates are tried in order: DateTimeOriginal, DateTimeDigitized, then IFD0 DateTime. A clock
with more than three colon-separated components is rejected as a whole, so the next candidate or the
XMP supplies the time. ISO 8601 times in XMP need a complete clock. Calendar checks go through
`chrono`. Invalid GPX point times count as untimed.

## XMP namespaces

Written: `dc`, `xmp`, `xmpRights`, `photoshop`, `exif`, `exifEX`, `tiff`, `aux`, `lr`,
`Iptc4xmpCore`, and the one namespace the caller passes. Read as well: `crs`, `mwg-rs` regions
(`stArea`, `stDim`). Reading canonicalises prefixes by namespace URI and handles attribute and element
forms, `rdf:Seq`/`Bag`/`Alt` (x-default first), `rdf:parseType="Resource"` and nested-description
structs, predefined and numeric entities.

A foreign namespace that uses a recognised prefix cannot supply recognised properties. Its keys are
`Q{hex-encoded namespace URI}local`. Merging keeps the original foreign XML.

## Limits

XMP: 64 levels, 200 000 nodes, 512 namespace declarations in scope, 1 MiB of flattened keys and 8 MiB of opaque names per packet
(`XmpError::TooComplex`). GPX: 64 levels, 5 000 000 points. MPF: 16 index entries. CR3 and JPEG XL: 4096 boxes; a `brob` box inflates to at most 32 MiB. JPEG ICC is returned only when its APP2 chunks form a
complete, consistent, uniquely numbered sequence. TIFF limits are `craft-tiff`'s. No input panics:
proptests feed random bytes to every container walker and random text to the XMP parser, and a sample
JPEG is cut at every byte.

## Sources

CIPA DC-008 (Exif 2.32), TIFF 6.0, Adobe DNG 1.7 (tag numbers), ISO 16684-1 and the Adobe XMP
Specification Parts 1 to 3, IPTC-IIM 4.2 and the IPTC Photo Metadata Standard, the MWG Region
Guidelines, ITU-T T.81 (JPEG markers), ICC.1 (APP2 embedding), the PNG Specification 3rd ed. (`eXIf`,
`iTXt`), the WebP RIFF container specification, ISO/IEC 14496-12 (box structure), GPX 1.1. XML parsing:
`quick-xml` (MIT). No third-party metadata code was consulted.
