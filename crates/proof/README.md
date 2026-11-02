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
| English (en-US, en-CA, en-GB and the British-spelling Englishes such as en-AU, en-NZ) | SCOWL 2026 Hunspell dictionary per English + `en-extra.txt`, dialect policy (`en-variants.txt`); a dictionaries-folder English first; without SCOWL the installed Hunspell English, else the built-in Moby list ([below](#english-is-always-checked)) | yes | Moby list, our patterns, heuristic |
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

1. **Dictionaries folder:** the English's own `.aff`/`.dic` pair (`en_US`, `en_GB` or `en_CA`)
   that the user put in the folder set with `set_dictionaries_folder`, in `WORDCRAFT_DICTIONARIES`
   or in WordCraft's own `dictionaries` folder. The user's choice comes first.
2. **Installed English pack:** reserved for Language packs (craft-libs#485, CP4.2).
3. **Embedded SCOWL:** the English's own SCOWL `rel-2026.02.25` Hunspell dictionary from
   `CRAFT_I18N_DIR` (`language-data/en-us`, `en-gb` (British `-ise`) or `en-ca`, `spelling.aff` +
   `spelling.dic`), read by the affix engine. en-AU, en-NZ, en-IE and the other British-spelling
   Englishes read en-GB; Canadian English reads en-CA. A word is right when the English's own
   dictionary knows it (or `en-extra.txt` does), never because another English's does
   (`colourblind` is wrong in en-US, `colorblind` in en-GB); suggestions come from the same
   dictionary. SCOWL's size-60 lists keep one spelling per dialect, and Word accepts more, so
   British English also takes the `-ize` spellings of its `-ise` words that American or Canadian
   English spell so (`organize`, `realization`; never `wize` or `advize`), and `en-variants.txt` flags the other dialect's spellings and accepts this
   dialect's other spellings of known words (`judgement` in en-US). English with no dialect
   (`is_correct`) takes a word any of the three knows. Suggestions offer a missing space only
   between two lower-case words of three letters or more, after every one-word candidate, and no
   name or abbreviation (`Col`) for a lower-case word. With SCOWL embedded the system locations aren't
   searched.
4. **System Hunspell:** the English's own pair in the system locations. Never another English's:
   an `en_US` dictionary would mark British spellings, so en-GB and en-CA without their own use
   the built-in list, which has both spellings. Each name is looked for in every folder before the
   next name; a file that can't be read is skipped for the next name or folder; one file is read
   once for every English that uses it. `COMPOUNDRULE`, `ONLYINCOMPOUND` and `COMPOUNDMIN`
   (ordinals such as `21st`) are read; flag-based compounding (`COMPOUNDFLAG`…) is ignored for
   English rather than refused; `OCONV` writes suggestions as the dictionary says (`don’t`), as for
   every language. None on the web.
5. **Built-in list:** `data/en-words.dic`, the whole Moby word list (about 165,000 words, 344 KB
   compressed, words only) built into the crate, with `data/en-extra.txt` (WordCraft's supplement
   of modern words, a copy of craft-i18n's) and the inflection rules. A word it doesn't know is
   misspelled. Suggestions are the words one edit away (swap, missing, added or changed letter),
   looked up in the list, for words of up to 32 letters: swaps first, then common words,
   neighbouring keys, the same first letter.

The embedded Moby list (`language-data/en-us/en-us.dic`) is no longer a spelling source: it is
the same list as the built-in one, and stays as the hyphenation word list. `en-variants.txt`
applies to every source above when it is embedded.

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

The SCOWL notice (SCOWL's licence, the Ispell BSD licence of the affix files and Benjamin Titze's
notice) is embedded with the dictionaries: `craft_proof::scowl::notice()`, for an About window.

Embedding SCOWL adds 1,542,396 bytes of Language data (the en-US, en-GB and en-CA `.aff`/`.dic`
and the notice); the Moby list (432,905 bytes) stays for hyphenation.

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
