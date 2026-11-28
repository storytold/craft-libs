# Olympus and OM System lossless ORF

The compressed decoder in `src/orfc.rs` and its selection in `src/orf.rs` are ported
from [LightCraft #3964](https://github.com/storytold/lightcraft/pull/3964), revision
`225282d856868c4cb9063c72dd331c5bbcdadd1e`.
This is the implementation extracted from [LightCraft #1162](https://github.com/storytold/lightcraft/pull/1162),
not the implementation withdrawn from [LightCraft #3775](https://github.com/storytold/lightcraft/pull/3775).
The original branch records public Olympus patent prose
(US7333035B2 and US7715639B2, Takashi Ishikawa), CC0 raw.pixls.us samples, and
sensor output observed from the LibRaw 0.22.2 `unprocessed_raw` executable as a
black box. Neither patent specifies the ORF bit layout. Exact sensor agreement
validates output; it does not independently prove historical derivation or settle
provenance acceptance. The withdrawal of the other decoder remains recorded in
[orf-compression.md](orf-compression.md).

The stream starts with `00 00 00 00 01 00 00`, then MSB-first signed adaptive Rice
codes without row alignment. Even and odd columns have independent magnitude,
correction and flatness state, reset at each row. Same-colour neighbours two
pixels away predict each sample. Truncated codes and values outside the 12-bit
range return errors. Checked dimensions, the caller's `Limits`, the six-bit
minimum per sample, and fallible sensor-buffer reservation bound allocation.
The bit layout, prediction and reference methodology are recorded in
[LightCraft's implementation record](https://github.com/storytold/lightcraft/blob/225282d856868c4cb9063c72dd331c5bbcdadd1e/docs/orf.md).

The existing uncompressed reader, `WB_GLevel` handling, level provenance and
strip-table checks are preserved. Every matching single-strip stream is fully
validated even during a header probe. Invalid Rice codes retain a valid word or
packed interpretation. If both interpretations fit, decoding returns an explicit
unsupported-layout error: neither Compression=1 nor the prefix distinguishes them.
`.ORI` directory discovery,
colour calibration, lens corrections and library/proxy upgrades belong to apps.

## Verification

`tests/orf_corpus.rs` pins the SHA-256 of each input and every unscaled sensor
sample for 23 CC0 files, including compressed and uncompressed 80 MP captures.
No media is committed. Download the fixtures with LightCraft's corpus downloader,
then set `CRAFT_RAW_ORF_CORPUS` to its `corpus` directory. With that variable set,
every fixture is required; absence is an error. Without it, the external-corpus
test reports that it is skipped. The synthetic unit tests always run.

```sh
cargo test -p craft-raw --lib orf
CRAFT_RAW_ORF_CORPUS=/path/to/lightcraft/corpus cargo test -p craft-raw --test orf_corpus -- --nocapture
cargo test -p craft-raw --test it vendor_robust::
```

The optional [reference checker](verify_orf_reference.py) reads the same 23-case
manifest in `tests/orf_corpus.rs`. It requires Python 3.11+ and an explicitly
selected, installed `unprocessed_raw` executable; it does not fetch or build one.
It verifies the input hashes, then compares every unscaled sensor sample with the
pinned hashes. PGM output is streamed in bounded blocks from a temporary directory
and removed even on failure. Inputs and expected hashes are never rewritten.

```sh
python3 -m unittest discover -s crates/raw/docs -p 'test_verify_orf_reference.py' -v
python3 crates/raw/docs/verify_orf_reference.py --corpus /path/to/lightcraft/corpus --oracle /path/to/unprocessed_raw
```

The reference check records output agreement, not independent derivation or
provenance acceptance. [LightCraft's pinned acquisition manifest](https://github.com/storytold/lightcraft/blob/225282d856868c4cb9063c72dd331c5bbcdadd1e/xtask/src/raw_samples.rs)
lists the raw.pixls.us source URLs and input SHA-256 values for all 23 files.
