# Clean-room format work in craft-libs

Several crates here read or write formats that other open-source projects also implement, often
under licences the Craft apps cannot take code from (GPL, LGPL, AGPL, CDDL) or under terms that
forbid it (Adobe's). This document is the standard a format crate follows so that every line of
it is ours, and so that a reviewer can check that claim from the repository alone. It is the
written form of what `craft-jpeg`, `craft-media` and `craft-crw` did.

## What "clean-room" means here

A clean-room crate is written only from:

- **published specifications and standards** (ITU-T, ISO/IEC, IETF RFCs, a vendor's own published
  format document, a registry's pages), read as documents;
- **prose documentation** of a format or its tags (ExifTool's tag-name and format pages, a wiki
  page that describes a container), never a project's code;
- **sample files** we may use: CC0, or owned by a contributor who consents, or produced by us.
  Not NC-licensed files, not files whose licence is unclear, never anything from inside a
  proprietary application's bundle;
- **a reference implementation run as a black box**: an official pre-built binary of another
  decoder, fed inputs and observed for outputs, with every run logged. Its source is never
  opened, read, searched, diffed or quoted, including comments, licence headers, mirrors and
  excerpts in forum posts or AI answers.

Never consulted, in any form: GPL/LGPL/AGPL/CDDL decoder and encoder sources (dcraw, LibRaw,
rawspeed, rawloader/rawler, dnglab, darktable, RawTherapee, ART, libopenraw, lensfun, ExifTool's
Perl, FFmpeg/libav, x264/x265 and the like), Adobe's SDKs, profiles (DCP, LCP), presets, fonts,
icons or binaries, and any earlier attempt of ours that was itself not clean.

## The process (what the record must show)

Modelled on LightCraft's ARW compression derivation and repeated for CRW
(`crates/crw/clean-room/`). A crate that derives an undocumented format commits a `clean-room/`
folder with:

1. **Starting assumptions**, written before any source is consulted, every item marked
   unverified, committed once and never edited. A reviewer diffs them against the result to see
   what was assumed and what was established.
2. **A plan** naming the sources that will be used, the sample corpus (public files with URL,
   licence and SHA-256; private files by name, size and hash only, never content), the phases,
   the freeze point, and the questions that are the maintainer's to decide. Work past the plan
   waits for the maintainer's recorded approval.
3. **A source log**: every URL opened and every search run, in order, with the time, what was
   asked and what was taken. If a page exposed implementation details that were not asked for,
   the log says so and what it said.
4. **Derivation documents** in which every fact cites the experiment (script and log) or the
   document that established it, and corrections are stated, not silently applied. Statistics
   before decoding; hypotheses tested against internal consistency first; black-box probes
   logged one by one.
5. **A freeze**: one document holding the complete derived format, with its tables as data. Its
   commit is the freeze point. Nothing in it is tuned afterwards. A mismatch found later reopens
   the derivation *explicitly*: the cause, the change and a new freeze revision are recorded in
   that document.
6. **Verification** after the freeze, with zero tuning: every sample compared exactly against the
   reference, every file's verdict recorded, failures recorded as failures. Cross-body checks
   (tables learned on one body, exact on another) are the strongest independent evidence.
7. **The implementation** from the frozen document, with: never-crash rules (`get()` for every
   file-derived offset, checked arithmetic, bounded recursion and allocation, no `unwrap`/`panic`),
   synthetic tests that need no sample files (a test-only encoder that inverts the model is the
   usual tool), hostile-input tests, and a sample-file test pinning the verified decode (a digest
   of the output per file) so the crate can prove it still matches without redistributing the
   references.
8. **Table data** lives in the record as JSON and is rendered into Rust by a committed script; the
   generated file says so and is never edited by hand.

When the derivation was done in another repository, the crate's `clean-room/` may hold only items
3, 5, 6 and 8, provided it pins that repository, path and revision (in its README and in `NOTICE`)
for the starting assumptions, the plan and the derivation documents, as `craft-crw` does.

Scripts in the record use the Python standard library only and run under `python -I`. Local-only
material (reference models, oracle binaries, copyrighted specification PDFs, dumps that may carry
personal metadata) is named in the record with how to obtain it, and is gitignored.

## Assets and attribution

Sample files are never committed (sizes aside, they are other people's photographs): tests
download them outside the checkout by pinned URL and SHA-256, licence checked (`scripts/
fetch-*.py`), and skip cleanly without them. Each manifest, each table derived from samples and
each log set has a row in `ATTRIBUTION.md`; code and records that came from an app are listed in
`NOTICE` under *Source provenance* with the repository, paths and revision.

## When something is not clean

Stop, record what was seen and where (the source log), and tell the maintainer. Code written
afterwards by the same person for the same part of the format is suspect; the maintainer decides
whether the part is re-derived by someone else, documented as derived from that source under its
licence terms, or dropped. Never quietly continue.
