# Lossless embedded data experiment

Measured 2026-11-02 on the ARM64 macOS development host. Generated canonical files are SHA-checked against `source-pins.json`, preserving the uncompressed baseline. No dictionary
entries, variant rules, titles, affixes, model features, or weight precision were
removed. Full app executable and downloadable package measurements are separate
from this embedded-data experiment.

## Production choice: separate Brotli q11 streams

`build.rs` uses pinned Brotli 8.0.4, quality 11 and window 22, to produce six
reproducible `.br` streams in `OUT_DIR`. Canonical text is generated in OUT_DIR from pinned raw pack files; raw assets are not committed here.
Only compressed payloads enter production binaries; original text is included
for byte-identity assertions in test binaries. Runtime uses the existing pure
Rust `brotli-decompressor` 5.0.3 and `crc32fast` 1.5.2 dependencies.

| Canonical file | Raw bytes | Brotli q11 bytes |
|---|---:|---:|
| frequency.tsv | 698,847 | 140,695 |
| km_symspell_dictionary.txt | 2,329,978 | 349,190 |
| khmer-multiple-spellings.txt | 72,509 | 13,529 |
| kcc-weights.tsv | 1,126,467 | 210,329 |
| titles.txt | 1,184 | 337 |
| affixes.tsv | 1,017 | 263 |
| **Total** | **4,230,002** | **714,343** |

The text payload saves **3,515,659 bytes (83.11%)** compared with raw embedding,
and **166,356 bytes** compared with the initial zlib implementation. These
figures are not installer download savings: installer ZIPs already compress raw
text. Full app/package measurement is necessary to judge download benefits.

## Alternatives measured

All formats use the identical six files. Brotli uses `lgwin=22`.

| Format | Total compressed bytes |
|---|---:|
| Rust miniz_oxide zlib level 10 | 880,699 |
| Python zlib level 9 | 879,850 |
| Brotli q9, separate files | 810,341 |
| Brotli q9, concatenated files | 787,629 |
| **Brotli q11, separate files** | **714,343** |
| Brotli q11, concatenated files | 678,587 |
| Python bzip2 | 680,063 |
| Python LZMA | 688,328 |

Concatenation saves another 35,756 bytes at q11 but would inflate all 4.23 MB on
any first asset use. Separate streams preserve lazy loading. Bzip2/LZMA require
additional decoder choices; Brotli already belongs to the app's dependency tree.
Quality11 costs approximately 4.46 seconds of encoding for all six files on this
host, versus 0.36 seconds for q9; encoding happens during the build only.

A standalone optimized probe checked every inflated byte and ran 30 decoding
repetitions. Median decoding of all six files, including expanded CRC32 and UTF-8
validation: Brotli q9 separate 12.214 ms, q11 separate 12.798 ms, q11 concatenated
11.551 ms; zlib 13.272 ms with its built-in checksum. The production path adds
compressed CRC32 validation before decoding; full API measurements below include
that work and parsing.

## Full API benchmark

`examples/benchmark.rs` is shared between the untouched raw baseline and compressed
implementation, both optimized (`opt-level=3`, thin LTO, 16 codegen units). Fresh
processes are interleaved, reversing order each round. Each initializes all six
assets through normal APIs, then runs 500 warm segmentations and 100 warm
suggestion requests. Fresh processes are required to measure first-use cost.

The initial zlib experiment used 20 process pairs and returned behavior checksum
8,828 every time:

| Workload, median | Raw baseline | Initial zlib |
|---|---:|---:|
| Cold combined initialization/query | 20.066 ms | 33.674 ms |
| 500 warm segmentations | 55.264 ms | 55.160 ms |
| 100 warm suggestion requests | 629.480 ms | 636.949 ms |

Initial zlib median maximum RSS across five additional runs was 16,105,472 bytes,
compared with 15,204,352 bytes for raw embedding. Final Brotli API/RSS measurements
are recorded separately from these historical zlib results.

The final Brotli run waited until full application builds/tests finished, then
ran 20 new interleaved raw/Brotli process pairs and five RSS runs per variant.
Every behavior checksum again equaled 8,828:

| Workload, median | Raw baseline | Final Brotli q11 |
|---|---:|---:|
| Cold combined initialization/query | 20.189 ms | 32.678 ms |
| 500 warm segmentations | 55.516 ms | 54.959 ms |
| 100 warm suggestion requests | 625.587 ms | 637.226 ms |
| Peak RSS, five-run median | 15,237,120 bytes | 17,072,128 bytes |

Final first-use overhead is **12.490 ms once**. Warm segmentation is effectively
unchanged. The suggestion batch is 1.86% slower, with overlapping observed ranges
(raw 605.072–661.289 ms; Brotli 614.560–672.344 ms). Cold ranges were
raw 19.782–58.966 ms and Brotli 32.448–50.258 ms. Median peak RSS increases by
**1,835,008 bytes**. These local microbenchmarks are not cross-platform guarantees.
The optimized external baseline/compressed comparison also found zero differences
across all 8,020 synthetic segmentation cases and all 76,847 spelling entries.
Historical zlib and final Brotli results are stored separately.

After all assets are used, inflated UTF-8 buffers retain **4,230,002 logical
bytes**; baseline strings instead reside in mapped executable pages. Parsed maps
remain the same. Actual RSS includes mapped executable pages, decoder code,
allocation/page rounding, transient decoder buffers, and workload allocations.
Compression does not reduce parsed-map memory requirements. Only Khmer use loads
the lazy buffers; no network or filesystem requests are made.

## Correctness and failure behavior

Eleven crate tests pass, including corrected KCC extraction and 16 exact upstream
differential fixtures. New checks compare **every inflated byte of all six
assets**, all 29,599 frequency entries, all 76,847 spelling entries, all 1,824 final
preferred-spelling mappings (duplicate source alternatives retain last-rule wins),
all 53 titles, all 38 affixes, and **all 39,127 f64 weight bit patterns**. No lossy
weight quantization is used.

Build-generated descriptors retain exact expanded length and both packed/raw
CRC32 values. Runtime checks packed CRC32 **before invoking the decoder**, so
bit-corrupted window headers cannot cause unexpected decoder allocations.
Canonical streams are built with window 22. A bounded writer prohibits expansion
past each exact expected length, and an independent 3 MiB ceiling rejects oversized
descriptors. Raw CRC32, exact expanded length, and UTF-8 are checked afterward.
CRC32 detects accidental corruption; it is not an authenticity signature.

Malformed Brotli, corrupt first/middle/last bytes, truncation (also with an updated
packed CRC), wrong expanded lengths, excessive output, wrong raw CRC, and invalid
UTF-8 return unavailable data without panicking. `data_available()` reports
whether all assets loaded. An unavailable spelling dictionary fails open rather
than flagging every word; unavailable grammar/model rules make no corrections.
Each asset initializes once through `OnceLock<String>`.

Verified `cargo test -p wordcraft-khmer`, Clippy on all targets with `-D warnings`,
formatting, and an actual `wasm32-unknown-unknown` compilation. Reproduce full API
samples with `cargo run --release -p wordcraft-khmer --example benchmark`.
