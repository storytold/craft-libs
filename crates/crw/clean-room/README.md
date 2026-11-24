# Canon CRW clean-room derivation record

What `craft-crw` decodes a CRW from, and the evidence that it is right. The full record was
produced in LightCraft (`clean-room/crw/`, branch `clean-room-crw-format-decode`, revision
`6ca439b5155820d2befeacd81ec0c6a80b2bd3bc`; the record is identical at `70c755e4`, where the decoder was ported) between 2026-10-12 and 2026-10-25; the documents below moved here
**verbatim** when the decoder became this shared crate, and the derivation trail stays at that revision. The documents are the evidence: they are
not rewritten after the fact, so paths they mention are LightCraft's paths of that time (the
mapping is below).

The derivation trail and frozen table-generation materials remain in
[LightCraft PR #3021](https://github.com/storytold/lightcraft/pull/3021). This crate retains the
format specification, verification summary, colour metadata notes, source log and public-facing
decoder documentation.

## Documents

| File | What it is |
|---|---|
| [`sources.md`](sources.md) | Every URL opened and search run, with the time, what was asked and what was taken. The rules in force are at its top. |
| [`02-crw-format.md`](02-crw-format.md) | **The frozen format** (revision 2) and its reopening rule. The decoder implements this document. |
| [`03-verification.md`](03-verification.md) | Full-frame exact comparison against the reference, zero tuning: 28 of 29 files, then 1,516 of 1,516 D60 files. |
| [`04-color.md`](04-color.md) | CFA phase, active area, black and white levels, white-balance records, orientation and time: what the decoder reports beyond the samples. §3's colour matrices concern LightCraft (below). |
| [`05-private-corpus.md`](05-private-corpus.md) | A summary of private-corpus decoder verification; no image data or identifying file list is included. |

## What stayed local

- The Python reference model and probe harness (`plan/crw/model/` in LightCraft, gitignored),
  by the maintainer's decision #4. Its findings are the entropy-coding derivation, which is not
  part of this record.
- The reference decoder: LibRaw 0.22.2, official pre-built Windows binaries installed by the
  maintainer; only `unprocessed_raw.exe` was ever run, as a black box. The source tree shipped
  alongside it was never opened, read or searched.
- Canon's CIFF specification PDF (copyrighted; archive URL and SHA-256 in `sources.md`).
- The reference TIFFs (`<name>.crw.tiff`) the verification compared against; `CRAFT_CRW_ORACLE`
  points the test at a local copy.

## Path mapping (LightCraft at the time → craft-libs)

| In the documents | Now |
|---|---|
| `clean-room/crw/…` | `crates/crw/clean-room/…` |
| `crates/raw/src/vendor/crw.rs` | `crates/crw/src/lib.rs` |
| `crates/raw/src/vendor/crw_tables.rs` | `crates/crw/src/tables.rs` |
| `crates/raw/tests/crw_oracle.rs` | `crates/crw/tests/samples.rs` (`CRAFT_CRW_ORACLE`, formerly `LIGHTCRAFT_CRW_ORACLE`) |
| `white_from_data` (LightCraft's shared estimate) | `saturation_plateau` in `crates/crw/src/lib.rs` |
| `xtask/src/main.rs` `RAW_SAMPLES` (corpus URLs) | `crates/crw/tests/samples.json` + `scripts/fetch-crw-samples.py` |
| `plan/crw/model/gen_tables.py` | LightCraft PR #3021 |
| `docs/arw6-compression.md` (the house model for a derivation) | LightCraft's document; the method is summarised in `docs/clean-room.md` of this repository |
| `crates/raw/src/chart_table.rs`, `spectral_table.rs`, `camera_preview`, `assets/camera-profiles/` | LightCraft (colour stays in the app) |

## Rules that were in force

From `sources.md`: no raw-decoder source may be opened, fetched, searched inside
or requested (dcraw, LibRaw, rawspeed, rawloader/rawler, dnglab, darktable, RawTherapee,
libopenraw, ExifTool's Perl code, including licence headers, comments and mirrors). Allowed:
published format documentation in prose, published standards, the sample files, and a reference
decoder's binary run as a black box with every probe logged. No tuning after the freeze: a
mismatch reopens the derivation explicitly, with its cause and a new freeze commit recorded
(`02-crw-format.md`, revision 2, is the one reopening). The general standard this followed is
[`docs/clean-room.md`](../../../docs/clean-room.md).
