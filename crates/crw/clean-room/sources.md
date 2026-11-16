# CRW clean-room derivation — source log

Every URL opened and every web search run for this work, in order, with the time (local,
Europe-style 24 h), what was asked, and what was taken. If a result or page exposed details of an
existing decoder's algorithm that were not asked for, that is recorded too, with what it said.

Rules in force (from the task): no raw-decoder source may be opened, fetched, searched inside or
requested — dcraw, LibRaw, rawspeed, rawloader/rawler, dnglab, darktable, RawTherapee,
libopenraw, ExifTool's Perl code — including licence headers, comments and mirrors. Allowed:
published format documentation in prose (ExifTool's CIFF/CRW format and tag-name pages),
published standards, and the sample files. The branch `crw-decode` (an earlier attempt) is not
checked out, diffed, logged or read.

## Local files consulted (non-web)

- 2026-10-12 ~19:2x — `CLAUDE.md` (project instructions; in session context from the start) and
  `docs/arw6-compression.md` (the house model for deriving and documenting a format). Taken:
  process shape (assumptions → spec + samples → Python reference model → freeze → black-box
  verification), documentation conventions, corpus table format.
- 2026-10-12 ~19:2x — directory listing and SHA-256 of the maintainer's private
  `../RawImages/*.crw` (18 files, Canon EOS D60 era, 5.5–8.6 MB). Taken: file names, sizes,
  hashes only. No .THM files present. Contents not yet parsed.

## Web

- 2026-10-12 19:26 — **Fetch** `https://exiftool.org/TagNames/CanonRaw.html` (ExifTool's
  CanonRaw tag-name page; generated documentation, not Perl source). Asked: CIFF structure, tag
  IDs/names, compression info, spec links. Taken: tag IDs — 0x2005 RawData, 0x2007 JpgFromRaw,
  0x2008 ThumbnailImage, 0x1835 DecoderTable (subdir: DecoderTableNumber, CompressedDataOffset,
  CompressedDataLength), 0x1031 SensorInfo (subdir), 0x1810 ImageInfo (ImageWidth, ImageHeight,
  ComponentBitDepth, ColorBitDepth…), 0x1030 WhiteSample (BlackLevels at index 55), 0x10a9
  ColorBalance, 0x10ae ColorTemperature, 0x080a CanonRawMakeModel, 0x102a CanonShotInfo, 0x1834
  CanonModelID, 0x300a ImageProps, 0x2807 CameraObject, 0x300b ExifInformation, 0x0000
  NullRecord, 0x0001 FreeBytes, and others. No compression algorithm described. Links to
  `canon_raw.html` for structure. **No decoder algorithm details shown.**
- 2026-10-12 19:27 — **Fetch** `https://exiftool.org/canon_raw.html` (Phil Harvey's prose "Canon
  CRW file format" page). Taken: 26-byte header (ByteOrder `II`/`MM` @0, HeaderLength u32 @2 =
  0x1a, signature `HEAPCCDR` @6, CRWVersion 0x00010002 @14, 8 reserved bytes @18); block =
  ValueData, DirCount u16, N×10-byte entries (tag u16, size u32, offset u32), optional
  OtherData, u32 DirStart at BlockSize−4; offsets relative to the block's ValueData start;
  values 2-byte aligned; tag masks DataLocation 0xc000 (0x0000 = ValueData, 0x4000 = in-entry,
  0x8000/0xc000 unknown), DataFormat 0x3800 (0x0000 bytes, 0x0800 ASCII string, 0x1000 u16s,
  0x1800 u32s/floats, 0x2000 structure, 0x2800/0x3000 subdirectory), TagIndex 0x07ff;
  subdirectory = a complete block inside ValueData, its BlockSize = entry Size; root block from
  HeaderLength to EOF. RawData 0x2005 noted as "the bulk of the CRW file"; JpgFromRaw 0x2007;
  no compression description. Links the original Canon CIFF spec at `http://xyrion.org/ciff/`
  (dead) and lclevy's CR2/CR3 pages. **No decoder algorithm details shown.**
- 2026-10-12 19:28 — **Search** (WebSearch, standard): `Canon CIFF specification 1.0 rev 4 PDF
  "CIFF" camera image file format specification download`. Results included Wikipedia
  "Camera Image File Format", a freedesktop libopenraw wiki page "Canon CRW" (NOT opened — it is
  a decoder project's page; left for the maintainer to approve), an unpkg mirror of
  exiftool's canon_raw.html, PRONOM. Taken: pointer to Wikipedia for an archived spec URL.
  **No decoder algorithm details shown in the result snippets.**
- 2026-10-12 19:28 — **Fetch attempt** `http://xyrion.org/ciff/` — failed (site dead).
- 2026-10-12 19:29 — **Fetch** `https://en.wikipedia.org/wiki/Camera_Image_File_Format`. Taken:
  spec part names/dates (Image Data File v1.0 r4, 1997-12-24; File/Directory organization v1.0
  r3, 1998-02-09), archive URL for the PDF, camera list (EOS D30/D60/10D/300D, PowerShot Pro1,
  G1–G6, S30–S70).
- 2026-10-12 19:29 — **Download** (curl)
  `https://web.archive.org/web/20081230095207/http://xyrion.org/ciff/CIFFspecV1R04.pdf` →
  scratchpad `ciff-spec/CIFFspecV1R04.pdf`, 99 422 bytes, SHA-256
  `416d4e49ea3f6ad2ba5ccc76b48ca100605f4a6127816c145e6dd5d6c3b51cf4`, 34 pages. Text extracted
  with a local stdlib-Python script (`pdftext.py`, kept in scratchpad; extraction output
  `extracted.txt`). This is Canon's own published "CIFF Specification on Image Data File"
  v1.0 r4 (1997-12-24); local copies (PDF + text extraction) kept in the gitignored
  `plan/crw/` (Canon-copyrighted document, not committed; re-fetchable from the archive URL,
  hash above). Taken: the heap file header struct (byteOrder, headerLength, type
  "HEAP", subType "JPGM"/"CCDR"/"TIFP"/"ARCH", version 0x00010002), heap = heap space +
  offsetTbl, offsetTblOffset as last u32 of the heap, DC_RECORD_ENTRY {typeCode u16, length
  u32, offset u32}, numRecords u16, typeCode bit fields (bits 15-14 storage: 00 kStg_InHeapSpace,
  01 kStg_InRecordEntry, 10/11 reserved; bits 13-11 dataType: 000 byte, 001 ASCII, 010 word,
  011 dword, 100 structure, 101/110 heap record, 111 reserved; bits 10-0 IDCode), masks
  kStgFormatMask 0xc000 / kDataTypeMask 0x3800 / kIDCodeMask 0x07ff, null/free records, 2-byte
  alignment rule, ImageSpec/ImageFormat/CapturedTime/ModelName/… structs, appendix type-code
  defines. The spec documents the container and common metadata only — **it does not describe
  the CRW raw-data compression** (ImageFormat fileFormat value 2 is merely "Reserved (for
  CanonCRW format)"). **No decoder algorithm details shown.**
- 2026-10-12 19:31 — **Fetch** (curl) `https://raw.pixls.us/` and
  `https://raw.pixls.us/json/getrepository.php?set=all` (the site's own data endpoint found in
  its HTML; first tried `json/data.json`, 404). Taken: the full sample table (2016 rows);
  filtered locally for `.crw` (51 rows) and classic CIFF-era bodies; per-file licence, URL and
  SHA-256 as published by the site. CC0 CIFF-era bodies: EOS D30, D60, 10D (×3), 300D (×2),
  Digital Rebel, PowerShot G1, G5, G6. CC-BY-NC-SA only (NOT downloaded, per task rules):
  PowerShot G2, G3, Pro1, Pro70, S30, S40, S45, S50, S60, S70, S2 IS and others. Many other
  `.CRW` rows are CHDK sensor dumps from much later PowerShots (A-, SX-, ELPH-series), a
  different format. **No decoder algorithm details shown.**
- 2026-10-12 19:32 — **Download** (Python urllib, stdlib) the 11 CC0 files above from
  `https://raw.pixls.us/getfile.php/...` into the gitignored `corpus/crw/`; every file verified
  against the site's published SHA-256 (all OK; the hashes are pinned in
  `crates/crw/tests/samples.json`).

## Local analysis (scripts in this directory)

- 2026-10-12 19:35–19:45 — `ciff_dump.py` (written here, stdlib only; container layout
  implemented from the two documented sources above) run over the 11 public files and the 18
  private files (whose dumps may contain personal metadata and are never committed). Observed
  facts are summarised in the LightCraft derivation record. No raw-data decoding was attempted; statistics
  only. The script and its logs are not part of this record.
- 2026-10-12 ~20:0x — P0/P1 local analysis (post-approval): the container notes written from the
  already-logged sources + corpus observations. `p1_payload.py`, `p1b_markers.py`,
  `p1c_lowbits_bounds.py` run over all 29 files. Established statistically (no decoding): the
  three payload layouts, the
  514-byte zero block at the compressed section start of every file, JPEG-style ff00 stuffing
  of every entropy stream (zero exceptions), the PowerShot `ff d9` terminator, and the exact
  low-bits region bounds on G5/G6. Documented in the LightCraft derivation record. No external sources
  consulted in this step.

## Oracle (P2, from 2026-10-12)

- The maintainer installed LibRaw 0.22.2 (official pre-built Windows binaries) in
  a local `LibRaw-0.22.2\bin` folder. That directory tree also contains LibRaw's source
  code; **no source file there is ever opened, read or searched** — only
  `bin\unprocessed_raw.exe` is executed, as a black box, per the approved plan (§5a/§7).
  All oracle runs and probe campaigns are logged in the derivation log, which is not part of this
  record.

- 2026-10-13 — **Fetch** `https://raw.pixls.us/json/getrepository.php?set=all` (and the site
  root to find that endpoint): recovered the stable `getfile.php` URLs of the 11 CC0 corpus
  files by matching their SHA-256s, and re-confirmed each row's CC0 licence; URLs pinned in
  `xtask/src/main.rs` `RAW_SAMPLES`. No decoder content on these pages.
- 2026-10-13 — **Fetch** `https://exiftool.org/TagNames/CanonRaw.html` (again) and
  `https://exiftool.org/TagNames/Canon.html` (prose tag tables only): 0x10a9 ColorBalance →
  Canon ColorBalance table (Index2; WB_RGGBLevels slot order Auto/Daylight/Shade/Cloudy/
  Tungsten/Fluorescent/Flash/Custom/Kelvin, index-29 "black levels for the D60" note),
  0x1030 WhiteSample field names, CanonShotInfo index 7 = WhiteBalance and the Canon
  WhiteBalance value enumeration, CanonModelID values. Prose documentation, no code.

- 2026-10-25 — **Fetch** `https://www.xrite.com/service-support/new_color_specifications_for_colorchecker_sg_and_classic_charts`
  (prose page): ColorChecker Classic charts made after November 2014 use new reference data,
  published as CGATS files. The CGATS download itself was refused to scripts (HTTP 403) and
  not worked around.
- 2026-10-25 — **Fetch** `https://raw.githubusercontent.com/colour-science/colour/v0.4.2/colour/characterisation/datasets/colour_checkers/chromaticity_coordinates.py`
  (colour-science, BSD-3-Clause; also its rendered docs page): the 24 post-November-2014
  ColorChecker Classic L*a*b* values (D50), credited there to X-Rite's release. Reference
  data only, the chart's measured colours; no camera or decoder content. Used for the EOS D60
  colour matrices.
