# Aksara input provenance and pending licence review

Recorded 2026-11-04. This is an evidence record, not an Embeddable approval.
The six source artifacts and their exact hashes are in [source-pins.json](source-pins.json),
at Aksara revision `31740e9d51d38171ae6119fc2d18a4a18fbfb2f1`.

## Contributor statements

Nathan Wells (@sungkhum), working as SBBIC, supplied the following statements in
the WordCraft #5357 implementation discussion. These are contributor attestations;
they are not statements found in the pinned Aksara repository:

> Frequency dictionary was combined from SIL and myself, and the rest was me (working as SBBIC)

The question named the frequency dictionary, SymSpell dictionary, multiple-spellings
list, titles, affix rules and tagger model. Wells then identified the SIL input:

> This is SIL frequency stuff https://github.com/silnrsi/khmerlbdict

An earlier statement identified SBBIC internal work, SEAlang lists and SIL
khmerlbdict as the overall resource lineage. The later per-file statement above
narrows the external-list dependency to the frequency dictionary. SBBIC also
reported permission to train the tagger on its internal documents. Those private
documents are not redistributed, and permission to train is recorded separately
from the model's redistribution licence.

## Per-file record

All paths below refer to the pinned Aksara revision. The direct upstream grant is
its root [MIT LICENSE](https://github.com/sbbic-khmer/aksara-khmer-word-processor/blob/31740e9d51d38171ae6119fc2d18a4a18fbfb2f1/LICENSE),
Copyright (c) 2025 SBBIC Khmer. The quoted statement above is the file-level
authorship evidence; no separate per-file provenance headers were found.

| Aksara file | Reported lineage | Conversion | Outstanding evidence |
|---|---|---|---|
| `public/dictionaries/km_frequency_dictionary.json` | SIL khmerlbdict combined with Nathan Wells's original SBBIC work | JSON to `frequency.tsv` | Maintainer classification/ADR sign-off; the exact SIL input and permission confirmation are recorded below |
| `public/dictionaries/km_symspell_dictionary.txt` | Nathan Wells's original SBBIC work ("the rest was me") | Unchanged text | Maintainer review of this attestation and the MIT grant |
| `public/dictionaries/khmer-multiple-spellings.txt` | Nathan Wells/SBBIC original work; explicit MIT permission confirmed 2026-11-04 | Unchanged text | Maintainer classification/ADR sign-off |
| `lib/khmer-titles.json` | Nathan Wells's original SBBIC work ("the rest was me") | JSON array to `titles.txt` | Maintainer review of this attestation and the MIT grant |
| `lib/khmer-affixes.ts` | Nathan Wells's original SBBIC work ("the rest was me") | Configuration parsed as data to `affixes.tsv`; never executed | Maintainer decision covering converted configuration data |
| `public/dictionaries/km_kcc_tagger.json` | Nathan Wells's original SBBIC model; internal training documents, with permission to train reported by SBBIC | JSON weights to `kcc-weights.tsv` | Maintainer decision covering trained models and this redistribution evidence |

The Rust runtime also uses frequency features. An original model does not establish
the licence of the separate frequency dictionary it consumes.

## SIL and inherited sources

The evidence snapshot is khmerlbdict revision
[`54abc6b87f4ffa6a0c95c09836c7c86648002273`](https://github.com/silnrsi/khmerlbdict/tree/54abc6b87f4ffa6a0c95c09836c7c86648002273).
Wells confirms this exact revision was the individual-file input downloaded
on 2026-01-08 as `khmerlbdict-master.zip`; no changes to the SIL files were made.
Its [LICENSE](https://github.com/silnrsi/khmerlbdict/blob/54abc6b87f4ffa6a0c95c09836c7c86648002273/LICENSE)
is MIT, Copyright (c) 2015 SIL NRSI; its exact bytes are pinned in craft-i18n.

Its [README](https://github.com/silnrsi/khmerlbdict/blob/54abc6b87f4ffa6a0c95c09836c7c86648002273/README.md)
lists SEAlang frequencies, village/region names, CLDR names, personal names,
word lists from two Bible versions and three Khmer books. In particular:

> places.txt. Language, script, territory and exemplar city names taken from CLDR.

The original SIL places file is now verified as an exact multiset match to
CLDR 28: 996 nonempty name occurrences (including duplicates) and one blank line.
The matching publication's full historical **Unicode-DFS-2015** notice is retained,
with a modification statement. This establishes matching content and its notice,
not the precise CLDR checkout SIL originally extracted. It does not replace that notice
with today's Unicode-3.0 terms. See the [inherited-source audit](https://github.com/sungkhum/craft-i18n/blob/feat/khmer-language-data/docs/khmer-inherited-sources.md)
for immutable hashes, the complete nine-input table and reproducible checks.
Wells confirms SBBIC had permission for its contributed word/frequency lists and
SIL had permission for its lists. The January import is independently reproduced
for all 10,000 word/frequency pairs. This closes the previously open factual
questions; maintainer classification/ADR sign-off is still required.

## SEAlang notice

Wells supplied a screenshot and the MIT notice, Copyright (c) 2010, Doug Cooper.
The same terms were independently retrieved on 2026-11-04 from
http://sealang.net/project/list/license.txt, linked by the project's menu.
The retrieved file is 1,070 bytes, SHA256
`2a98708d01f8a98765501f2cc5af607c00dcef85479518876a7b5338eaa78fd6`.
The manifest preserves the terms in `LICENCES/NOTICES.txt`; it does not support
non-GitHub raw URLs as pinned entries. Wells identifies the individual SIL files
as the imported external source;
that includes SIL's SEAlang-derived list. The supplied terms match the retrieved
file except for trailing blank lines.

## Classification and release gate

Under craft-i18n ADR 0004, maintainers must approve the permitted class.
The six-file pack is Download-only and hidden pending maintainer Embeddable
sign-off and the ADR decision covering model/configuration conversion. The original
pinned files can be inspected upstream or in the verified cache;
`stage` rejects the pack while its class is Download-only. It may
not be converted or embedded in a distributed binary in this state.

`source-pins.json` records Download-only and `build.rs` rejects any supplied
`CRAFT_KHMER_DIR`. Builds without data still work. There is no environment-variable
override. The source evidence and required notices are now recorded. A
maintainer-reviewed change must record sign-off and the corresponding class in both repositories
before enabling embedding again.
