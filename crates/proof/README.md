# craft-proof

Proofing for the Craft apps, per language: spelling, suggestions, grammar checks and hyphenation,
a Hunspell `.aff`/`.dic` reader, user words, ignored words and a dictionaries folder. It moved here
from WordCraft's `wordcraft-proof` with its history and the same public API. No `unsafe`, no
panics on hostile input, and it builds for `wasm32-unknown-unknown`.

```toml
craft-proof = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## Languages

| Language | Spelling | Grammar | Hyphenation |
|---|---|---|---|
| English (en-US, en-GB and the British-spelling Englishes, en-CA) | Moby word list + `en-extra.txt`, dialect variants (`en-variants.txt`); without them the installed Hunspell English, else the built-in Moby list ([below](#english-is-always-checked)) | yes | Moby list, our patterns, heuristic |
| Polish, Portuguese, Italian | the installed Hunspell dictionary | yes | TeX patterns |
| Croatian, Turkish, Russian, Czech, Indonesian, Spanish (es, es-MX, es-419, es-UY) | the installed Hunspell dictionary (Russian also `ru-extra.txt`) | yes | Turkish and Russian rules; English fallback otherwise |
| German | — | — | TeX patterns |
| Bulgarian | — | — | syllable rules |

Grammar rules, suggestion tuning and the rule-based hyphenation are code: they work in every
build.

## Use

```rust
use craft_proof::{ProofLang, check_text_with, suggest_in, Options};
use craft_proof::hyphen::{hyphen_points_in, Limits};

// A paragraph whose runs are in different languages (byte ranges of `text`).
let issues = check_text_with(text, &[(0..text.len(), ProofLang::EnGb)], &Options::default(), true, false);
let fixes = suggest_in("colour", 5, ProofLang::En);
let breaks = hyphen_points_in("hyphenation", &Limits::default(), ProofLang::En);
```

Hunspell dictionaries are not bundled; they are read from, in order: the folder given to
`set_dictionaries_folder`, `WORDCRAFT_DICTIONARIES`, WordCraft's own folder under the user's
application data (`%LOCALAPPDATA%\WordCraft\dictionaries` and `%APPDATA%\WordCraft\dictionaries`,
`~/Library/Application Support/WordCraft/dictionaries`, `$XDG_DATA_HOME/wordcraft/dictionaries`),
then the usual Hunspell and LibreOffice locations. None on the web.

State is process-wide: `add_word`, `ignore_word`, `set_dictionaries_folder`,
`load_user_dictionary`/`save_user_dictionary`. The crate reads no document model; callers pass
plain text and language spans.

## Language data

The word lists and patterns live in [craft-i18n](https://github.com/storytold/craft-i18n)
(`language-data/`), not here. `build.rs` embeds them at build time:

- `CRAFT_I18N_DIR=<absolute path to a craft-i18n checkout>` embeds `language-data/`.
- Unset (a fork, docs.rs, offline): the crate builds with a warning and no embedded Language
  data. English spelling then falls back as below, hyphenation that needs the data is off, and
  grammar and the Hunspell languages work as before.
- `CRAFT_I18N_REQUIRED=1` (releases) makes a missing directory or file an error.

Tests that pin results of the data run only when every file is embedded (`craft_proof_data`);
`src/no_data.rs` runs every public entry point when none is (`craft_proof_no_data`). A partial
data folder runs neither set.

### English is always checked

English spelling uses the first of these that is there; `english_source(lang)` says which.

1. **Embedded:** the Moby word list from `CRAFT_I18N_DIR` (above). Nothing below is used then.
2. **System Hunspell:** the run's own English `.aff`/`.dic` pair (`en_US`, `en_GB` or `en_CA`;
   each name in the dictionaries folder and then the system locations before the next name).
   Never another English's: an `en_US` dictionary would mark British spellings, so en-GB and
   en-CA without their own use the built-in list, which has both spellings. A file that can't be
   read is skipped for the next name or folder; one file is read once for every English that uses
   it. Compounding (`COMPOUNDMIN`, `COMPOUNDRULE`…, used by English dictionaries only for
   ordinals) and `OCONV` are ignored for English rather than refused. None on the web.
3. **Built-in list:** `data/en-words.dic`, the whole Moby word list (about 165,000 words, 344 KB
   compressed, words only) built into the crate, with `data/en-extra.txt` (WordCraft's supplement
   of modern words, a copy of craft-i18n's) and the same inflection rules as the embedded list. A
   word it doesn't know is misspelled. Suggestions are the words one edit away (swap, missing,
   added or changed letter), looked up in the list, for words of up to 32 letters: swaps first,
   then common words, neighbouring keys, the same first letter.

A dictionary that is there but can't be read (encoding, size, parse error, no words) is skipped,
and `dictionary_load_error(lang)` gives the reason; it never panics. Text in other languages is
never checked against English: a language without its dictionary has no spelling marks.

`data/en-words.dic` is public domain: the Moby Hyphenator II list by Grady Ward (Project Gutenberg
eBook #3204), from craft-i18n's `language-data/en-us/en-us.dic` with its break points dropped
and a few entries that are far more often typos left out (`teh`, `abd`, `nad`, `ur`, `u`, `r`;
the list is in the generator).
Regenerate with
`cargo run -p craft-proof --example en_words -- <craft-i18n>/language-data/en-us/en-us.dic crates/proof/data/en-words.dic`.
`data/en-extra.txt` is WordCraft contributors' own list (MIT OR Apache-2.0); keep it in step with
craft-i18n's copy.

## Check

```sh
cargo fmt -p craft-proof --check
cargo clippy -p craft-proof --all-targets -- -D warnings
CRAFT_I18N_DIR=/abs/path/craft-i18n cargo test -p craft-proof
cargo test -p craft-proof   # without Language data
cargo check -p craft-proof --target wasm32-unknown-unknown
```

CI: `.github/workflows/proof.yml`.

## Credits

storytold/craft-libs#30/#31 by brzuchal (Michał Marcin Brzuchalski) started the shared crate.

## Owners

WordCraft maintainers: echelon, dylanmtaylor, aiongg.

## Licence

MIT OR Apache-2.0. The Language data carries its own licences, recorded in craft-i18n's
`ATTRIBUTION.md`.
