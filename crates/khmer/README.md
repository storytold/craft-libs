# craft-khmer

Shared offline Khmer segmentation and proofing primitives. The default build contains no Khmer language data; enable an explicit language build using [DATA.md](DATA.md). No runtime network or JavaScript is required.

Derived from SBBIC Khmer's MIT-licensed Aksara at commit
`31740e9d51d38171ae6119fc2d18a4a18fbfb2f1`:
https://github.com/sbbic-khmer/aksara-khmer-word-processor
The upstream copyright and permission notice is retained in `LICENSE-AKSARA`.
All source data transformations are textual: JSON frequency objects and model
weights become TSV; title arrays become newline-delimited text; affix definitions
become TSV. No records or numeric precision are intentionally discarded.

An explicitly enabled Khmer pack uses the same feature-set v3 boundary perceptron and all 39,127
weights as Aksara's editor, with the full default 29,599-entry frequency
lexicon. The model's local orthography, frequency bands, longest dictionary
spans, and straddling-word features are ported directly. Khmer COENG clusters,
dependent vowels, signs, repetition sign, connectors and word joiners protect
break positions. Existing ZWSP remains an explicit opportunity. Sixteen pinned
fixtures were produced by running the upstream TypeScript with its production
lexicon and model; the Rust test compares exact word spans for those fixtures.
The original protected-phrase list is empty at this revision.

The additive frequency fallback retains the best score per cluster position
using dynamic programming rather than a width-12 whole-path beam. It ports
frequency penalties and bounded unknown chunks, and includes productive fused
affixes. The loaded v3 model decides multi-cluster Khmer runs, as upstream does.

Spelling uses the complete bundled upstream SymSpell word/frequency list.
Suggestions use bounded Unicode Levenshtein distance (two edits), ranked by edit
distance then frequency, with deterministic lexical ties. Inputs over 32 Unicode
characters receive no suggestions; the result limit is capped at 16. This uses
a bounded dictionary scan rather than SymSpell's deletion index. Preferred
spelling alternatives and phrase replacements reuse all upstream rules.
`grammar_matches` reports these preferred spelling rules, not general syntax.
`is_title` supports the consuming proofing layer's proper-name suppression.

## Limits

This is not a claim of complete Aksara parity. Browser `Intl.Segmenter` hints,
user-configured protected phrases, ignored/added word mutation, doubled-consonant
fuzzy lookup, all beam compound scoring exceptions, and SymSpell's
Damerau-Levenshtein transposition ranking are not ported. Non-Khmer text and digits
are intentionally excluded from `word_ranges`; punctuation stays outside returned
word spans. Word joiners protect adjacent opportunities but do not change text.
The full copyrighted upstream evaluation book corpus is not redistributed; the
16 synthetic differential fixtures do not establish corpus-wide accuracy parity.


## Exact-behavior performance revision

Frequency, spelling and variant indexes initialize independently. The boundary
scorer reuses scratch buffers and retains feature order; tests compare score
bits with the frozen implementation. Preferred forms use exact lookups at valid
word ends. Suggestions retain the full dictionary and ranking while using banded
stack-based edit distance. There is no dictionary pruning or alternative engine.
The fourth implementation borrows contiguous original-text spans for probes, caches overlapping frequency queries in a fixed seven-row ring, and reuses feature buffers across a run. Spelling uses a sorted UTF-8-offset index inspired by touchde’s WordCraft #3110, retaining every count and deterministic suggestion ranking. The full dictionary and contextual scorer differential retains exact f64 score bits.

## Human-reviewed regression examples

Selected visual review by @sungkhum on 2026-11-03 is preserved in
[review-2026-11-03.json](review-2026-11-03.json). `tests/human_review.rs`
excludes 13 distinct rejected positions across eight public/synthetic strings
and preserves one explicitly accepted finer phrase segmentation. A `craft-proof`
test demonstrates `កសីករ` remaining whole inside unspaced text, being flagged,
receiving `កសិករ` as its first suggestion, and checking cleanly after correction.
These selected examples are not a representative accuracy estimate; unmarked
rows are not endorsements and candidate breaks are not necessarily rendered wraps.
