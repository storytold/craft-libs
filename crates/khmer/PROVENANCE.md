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
| `public/dictionaries/km_frequency_dictionary.json` | SIL khmerlbdict combined with Nathan Wells's original SBBIC work | JSON to `frequency.tsv` | Historical SIL revision, included source lists and inherited notices; especially the CLDR version/notice |
| `public/dictionaries/km_symspell_dictionary.txt` | Nathan Wells's original SBBIC work ("the rest was me") | Unchanged text | Maintainer review of this attestation and the MIT grant |
| `public/dictionaries/khmer-multiple-spellings.txt` | Nathan Wells's original SBBIC work ("the rest was me") | Unchanged text | Maintainer review of this attestation and the MIT grant |
| `lib/khmer-titles.json` | Nathan Wells's original SBBIC work ("the rest was me") | JSON array to `titles.txt` | Maintainer review of this attestation and the MIT grant |
| `lib/khmer-affixes.ts` | Nathan Wells's original SBBIC work ("the rest was me") | Configuration parsed as data to `affixes.tsv`; never executed | Maintainer decision covering converted configuration data |
| `public/dictionaries/km_kcc_tagger.json` | Nathan Wells's original SBBIC model; internal training documents, with permission to train reported by SBBIC | JSON weights to `kcc-weights.tsv` | Maintainer decision covering trained models and this redistribution evidence |

The Rust runtime also uses frequency features. An original model does not establish
the licence of the separate frequency dictionary it consumes.

## SIL and inherited sources

The evidence snapshot is khmerlbdict revision
[`54abc6b87f4ffa6a0c95c09836c7c86648002273`](https://github.com/silnrsi/khmerlbdict/tree/54abc6b87f4ffa6a0c95c09836c7c86648002273).
This pins the evidence inspected, not the historical revision used by Aksara.
Its [LICENSE](https://github.com/silnrsi/khmerlbdict/blob/54abc6b87f4ffa6a0c95c09836c7c86648002273/LICENSE)
is MIT, Copyright (c) 2015 SIL NRSI; its exact bytes are pinned in craft-i18n.

Its [README](https://github.com/silnrsi/khmerlbdict/blob/54abc6b87f4ffa6a0c95c09836c7c86648002273/README.md)
lists SEAlang frequencies, village/region names, CLDR names, personal names,
word lists from two Bible versions and three Khmer books. In particular:

> places.txt. Language, script, territory and exemplar city names taken from CLDR.

The exact CLDR release and its accompanying notice have not been established for
the input Wells used. Unicode-3.0 is an allowed Embeddable licence under ADR 0004,
but that alone does not identify the historical file's licence or prove its notice
has been retained. The repository-level MIT grant does not replace inherited
source notices. The other listed inputs also need their scope confirmed for the
combined frequency artifact.

## SEAlang notice

Wells supplied a screenshot and the full MIT notice for SEAlang's lists,
Copyright (c) 2010, Doug Cooper. The source is
http://sealang.net/project/list/; the record is contributor-supplied because no
pinnable raw licence file is available. The full supplied notice is retained in
the km licence entry in craft-i18n's manifest and travels as `LICENCES/NOTICES.txt`
when staging or archiving. [LICENSE-AKSARA](LICENSE-AKSARA) retains the three MIT
copyright holders: SBBIC Khmer, SIL NRSI and Doug Cooper.
Whether SEAlang also entered directly, or only through SIL, is unconfirmed;
the per-file statement identifies SIL as the external frequency input.

## Classification and release gate

Under craft-i18n ADR 0004, unresolved inherited evidence remains Download-only.
The six-file pack is Download-only and hidden pending the frequency evidence,
maintainer Embeddable sign-off and the maintainer's ADR decision covering model
and configuration conversion. The original pinned files can be inspected in upstream or the verified cache;
`stage` rejects the pack while its class is Download-only. It may
not be converted or embedded in a distributed binary in this state.

`source-pins.json` records Download-only and `build.rs` rejects any supplied
`CRAFT_KHMER_DIR`. Builds without data still work. There is no environment-variable
override. Once resolved, a maintainer-reviewed change must record the exact source
evidence, required notices, sign-off and corresponding class in both repositories
before enabling embedding again.
