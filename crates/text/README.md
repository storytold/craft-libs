# Craft Text

`craft-text` holds the text processing that sits below each app's composer: functions that take
plain text and a language and return what layout needs before it shapes and places glyphs.

| Module | Feature | Provides |
| --- | --- | --- |
| `hyphen` | `hyphen` (default) | Hyphenation points |
| `segment` | `segment` (default) | Line-break opportunities with kinsoku; Thai, Lao, Khmer and Burmese words |
| `bidi` | `bidi` (default) | Bidirectional text: embedding levels, line reordering, mirroring |
| `linebreak` | `linebreak` (default) | Line breaking on a box/glue/penalty model |

Language features (`spanish`, `german`, …, `all-languages`) add the data for one language to every
module that uses it, and turn those modules on. Build with `default-features = false` to leave
out all modules and data.

## Hyphenation

`craft_text::hyphen` finds hyphenation points. It returns byte offsets into the word: a hyphen
may go before the char that starts at each offset. Line breaking, discretionary hyphens and
exception lists stay in each application.

### English

English is always available with `hyphen` and needs no language feature. Each word is looked up
in this order:

1. the bundled dictionary (`Dictionary::en_us`): the Moby Hyphenator II word list, about 160,000
   words. Simple inflections (`'s`, `-s`, `-es`, `-ed`, `-d`) inherit the breaks of their stem;
2. the bundled Liang patterns (`assets/hyphen/en-us.pat`), trained from the same list by the crate's
   patgen-style trainer;
3. a small syllable heuristic for words outside the pattern alphabet.

Case, apostrophes (`don't`, `’s`), surrounding punctuation and compounds are handled; no break is
placed next to an apostrophe or a non-letter. `Dictionary::en_us()` also serves as an English word
list, for example for spelling checks.

### Other languages

Other languages use the patterns compiled into [hypher](https://crates.io/crates/hypher) 0.1.8.
Each has a cargo feature named as in hypher (`spanish`, `german`, `norwegian`, …); `all-languages`
enables all 47. Patterns for languages whose feature is off are not compiled in.

```rust
use craft_text::hyphen::{Lang, Limits, hyphen_points_in};

fn points(word: &str, language: &str) -> Vec<usize> {
    match Lang::for_language(language) {
        Some(lang) => hyphen_points_in(word, &Limits::default(), lang),
        None => Vec::new(), // unknown language or feature off: don't hyphenate
    }
}
```

`Lang::for_language` accepts English language names as InDesign and IDML write them (`Spanish`,
`German: 2006 Reform`, `$ID/Norwegian: Bokmal`) and locale codes (`es`, `es_ES`, `pt-BR`,
`de_DE_2006`). It returns `None` for unknown languages and for languages not compiled in.
`Lang` is `Copy + Eq + Hash`, so it can key a cache of computed points. Its `Patterns` variant
holds an opaque `PatternLang`; build one with `Lang::for_language`.

### Limits

`Limits` holds the paragraph's settings: minimum word length, letters before the first and after
the last hyphen, and whether capitalized words are hyphenated. Words in all capitals are never
hyphenated. Limit checks are linear in word length.

### Regenerating the English data

`examples/hyphgen.rs` rebuilds `assets/hyphen/en-us.dic` and `assets/hyphen/en-us.pat` from the
Moby list. It needs the `generator` feature, which exposes the dictionary writer (`hyphen::dict`)
and the pattern trainer (`hyphen::patgen`); applications don't need it.

```sh
curl -LO https://www.gutenberg.org/files/3204/files/mhyph.txt
cargo run --release -p craft-text --features generator --example hyphgen -- mhyph.txt crates/text/assets/hyphen --validate
```

## Segmentation

`craft_text::segment` finds where a line may break. It takes plain text and returns byte offsets;
measuring, choosing breaks and hanging punctuation stay in each application's composer.
Consolidated from vectorcraft, wordcraft, deckcraft and designcraft.

```rust
use craft_text::segment::{Kinsoku, Options, line_breaks};

let options = Options { kinsoku: Kinsoku::Soft, ..Options::default() };
for b in line_breaks("日本語のテキスト。", &options) {
    // A line may end before byte b.offset; it must if b.mandatory.
}
```

`line_breaks` starts from UAX #14 (the `unicode-linebreak` crate) and adjusts it:

- `Options::kinsoku` applies the Japanese and Chinese line-start and line-end rules (JLREQ,
  CLREQ) at breaks next to an East Asian character. Text with no East Asian characters gets
  UAX #14's breaks unchanged.
  - `Hard` (default): UAX #14's breaks, minus those kinsoku forbids.
  - `Soft`: as `Hard`, and 々, ー and small kana may start a line.
  - `None`: UAX #14's breaks plus a break between any two East Asian characters.
- `Options::korean_words` sets Korean word by word: no break between two Hangul letters or Han
  ideographs. Off, Korean breaks between syllables as UAX #14 does.
- Mandatory breaks (newlines and the end of the text) are always kept.

`kinsoku_allows(before, after, mode)` is the same rule for composers that build their own
opportunity lists.

### Each app's kinsoku as a `Kinsoku` value

| App | App setting | `Kinsoku` | Rules the app keeps |
| --- | --- | --- | --- |
| vectorcraft | Kinsoku set None, Hard, Soft | `None`, `Hard`, `Soft` | none |
| wordcraft | `kinsoku` on, off | `Hard`, `None` | `word_wrap` off: Latin words break at any character |
| designcraft | no set (the breaker's own rules); Hard Kinsoku | `Hard` | custom document tables, inseparable pairs, bunri-kinshi, rensuuji, roman word break |
| designcraft | Soft, Simplified Chinese, Traditional Chinese, Korean Kinsoku | `Soft` | as above |
| designcraft | Kinsoku set None | `None` | as above |

Push-in and push-out priority and hanging punctuation decide how a line fits, not where it may
break, and stay in the composer. deckcraft has no kinsoku of its own (see below).

### Kinsoku sets for adopters

The sets are vectorcraft's (`no_line_start`, `soft_line_start`, `no_line_end`), plus these from
the other apps:

- Can't start a line: 〞 (closing double prime quote), ゛ ゜ ﾞ ﾟ (spacing sound marks), ％ ￠
  (postfixed abbreviations, JLREQ cl-13). From wordcraft.
- Can't end a line: ＄ ￡ ￥ (prefixed abbreviations, JLREQ cl-12). From wordcraft.

What changes for each app:

- vectorcraft: the additions above. Breaks come from UAX #14 rather than "after any CJK
  character", so `Hard` and `Soft` also keep every break UAX #14 forbids. Kinsoku no longer
  applies between two non-East-Asian characters (a space before an en dash in Latin text may
  break).
- wordcraft: its kinsoku on is `Hard`, which also keeps the prolonged sound mark, small kana,
  hyphens (‐ – 〜 ゠), ‼ ⁇ ⁈ ⁉ and the Latin closing marks next to East Asian text off the line
  start, and « ‘ “ ( [ { off the line end. Its kinsoku off is `None`. Breaking Latin words
  anywhere (`word_wrap` off) and the no-break-after-slash rule stay in wordcraft.
- designcraft: its breaker's set is `Hard` without the additions above and without ’ ” 〛 〟 ｠
  〜 ～ ゠ ‐ – ‼ ⁇ ⁈ ⁉ », ゕ ゖ, ㇰ–ㇿ and the half-width forms (can't start), « ‘ “ 〚 〝 ｟ ｢
  (can't end). Its Soft set also lets ゝ ゞ ヽ ヾ 〻 start a line; here they stay forbidden, as in
  InDesign's Soft set. Document kinsoku tables (`Kinsoku` in `doc/cjk.rs`) stay in designcraft.
  `korean_char_breaks: false` is `korean_words: true`.
- deckcraft: `Hard` is its UAX #14 breaks minus those kinsoku forbids. `None` adds breaks between
  East Asian characters, so no mode returns plain UAX #14 for East Asian text.

### Thai, Lao, Khmer and Burmese

UAX #14 gives these scripts class SA and leaves breaking inside them to a dictionary. Without the
script's language feature, `line_breaks` treats a run of them as one word: it never breaks inside
the run, only at its edges where UAX #14 allows a break (after a space, for example). A long Thai
sentence with no spaces is one unbreakable piece, so the composer needs an emergency break for it.
The features `thai`, `lao`, `khmer` and `burmese` split runs of that
script into words with ICU4X's dictionary segmenter (`icu_segmenter`). Any one of them links the
dictionaries for all four, about 1.8 MB; the default build has no ICU data. The data is under the
Unicode License v3 (see NOTICE).
## Bidi

`craft_text::bidi` runs the Unicode Bidirectional Algorithm (UAX #9, through
[unicode-bidi](https://crates.io/crates/unicode-bidi)) on document text. It works on plain text and
needs no fonts; shaping, glyph mirroring and caret placement stay in each application.

The pipeline is the one UAX #9 prescribes:

1. `Paragraph::new(text, Direction::Rtl)` resolves embedding levels once for the logical
   paragraph. `Direction::Auto` takes the direction from the first strong character.
   `Paragraph::with_overrides` also forces byte ranges to one direction (as LRO/RLO would),
   keeping offsets into the original text. A paragraph separator (`\n`, U+2029) in the text starts
   a new paragraph with its own level: `paragraphs()` lists each one's byte range and level, and
   `level()` is the first one's.
2. The application shapes and breaks lines in logical order.
3. Each line is reordered on its own: `Paragraph::visual_runs(line)` returns its runs of one level
   (byte range and level) left to right as displayed, with rule L1 applied (trailing whitespace,
   tabs and separators at the level of the paragraph they are in). Zero-width joiners and tag
   characters keep the level of the character before them, so no run splits a grapheme cluster.
   A run with an odd level displays its characters from end to start. Lines should not cross a
   paragraph separator.

Lines made of other units (clusters, glyphs, cells) use the free functions on level slices:
`line_levels(chars, levels, para_level)` for rule L1 and `visual_order(levels)` for rule L2.
`mirror(c)` returns the mirrored character for rule L4 (`(` for `)`), or `None` when the font has
to supply a mirrored glyph.

Offsets are bytes into the paragraph's `&str`: `Paragraph::levels()` holds one level per byte,
as unicode-bidi resolves them, and line ranges and runs are byte ranges. This matches the shaper
cluster offsets and line ranges the apps already keep, so no char-index table is needed;
`char_levels()` gives one level per character for code that works on `char` slices. Levels are
plain `u8` (even left to right, odd right to left), so unicode-bidi's types don't leak into
callers.

Text with nothing right to left in a left-to-right or `Auto` paragraph skips the algorithm.

The module started from DeckCraft's `text::bidi` (levels, rule L1 and L2 over chars). The
per-byte levels, the run-level reordering test cases and the hostile-input checks come from
WordCraft's layout; overrides and mirroring follow DesignCraft's composer.
## Line breaking

`craft_text::linebreak` breaks paragraphs into lines on Knuth and Plass's box/glue/penalty model.
It knows nothing about fonts: the composer turns shaped glyphs into `Item`s, calls a breaker and
places the glyphs of each returned `Line`. Widths are in the caller's units, and `width(k)` gives
the measure of line `k`, so first-line indents and shaped frames work. The core comes from
vectorcraft's every-line composer; tiered justification, emergency breaks, the hyphenation zone,
greedy and balanced lines come from designcraft's breaker.

- **Items.** Boxes, glue and penalties. Glue has a natural width and stretch and shrink in three
  tiers (word space, letter space, glyph scale) that are used in that order, as in InDesign's
  justification settings. Boxes may carry letter and glyph tiers too. Penalties have a cost and a
  flag (hyphens); ±10 000 forbids or forces the break. `Item::fill()` before the final
  `Item::forced()` leaves the last line unjustified. Optional fields, set on top of a constructor
  (`Item { hang, ..Item::glue(w, st, sh) }`): `hang` (optical margin), `edge` (an `Edge` of
  stretch and shrink gained at a line end or start, such as mojikumi), `hyphenation`,
  `emergency`.
- **Breakers.** `knuth_plass` runs one total-fit pass. `break_paragraph` runs passes at rising
  tolerances, then falls back to `greedy`; the returned `Breaks` says in `method` which one
  produced the lines. `greedy_with` resolves position-dependent widths (tabs). `balanced` evens
  the lines (meant for ragged text) and sets `Breaks::balanced` when it did. `overlong` finds
  words wider than the measure, where the composer adds `Item::emergency()` breaks.
- **Params.** Demerits, fitness classes, tolerance, hyphen limit, tier costs, `uniform_from`,
  and `Rag` for ragged text: rag stretch, short-last-line rating and hyphenation zone.
  `uniform_from` is the first line index from which `width(k)` stays the same. The default 0 is
  right for a constant measure; a caller with a first-line indent, drop cap or shaped frame sets
  it to the first line of the constant tail, or its breaks may not be the best ones. Negative or
  NaN demerits count as 0, and a NaN measure as 0.
- **Lines.** Each `Line` gives its item range, natural width, adjustment ratio, the fraction of
  each tier used and the width left over, so the composer can distribute the space.

```rust
use craft_text::linebreak::{Item, Params, Rag, break_paragraph};

let space = Item::glue(0.5, 0.0, 0.0);
let items = [Item::boxed(3.0), space, Item::boxed(3.0), space, Item::boxed(5.0), Item::fill(), Item::forced()];
let ragged = Params { rag: Some(Rag::default()), ..Params::default() };
let lines = break_paragraph(&items, &|_line| 8.0, &ragged, &[1.0, f64::INFINITY]).lines;
```

### Adopting

- **vectorcraft** `composer::compose`: a box per glyph (`hang` = `start_credit(i)` for `i > 0`),
  glue per space (justified: stretch 0.5 and shrink 0.2 of its width; ragged: none), a penalty per
  `Breakpoint` (width = its hyphen, flagged when it has one, cost 50 justified or 25 ragged), and
  `fill` + `forced` at the end unless Justify All. `Params`: `first_fitness: None`,
  `max_badness: 10_000`, `fitness_demerits` 10 000 or 1 000, `fitness` bounds
  `[-0.5, 0.5, 1.0]` or `[0.15, 0.4, 0.75]`, `uniform_from` as `layout.rs` computes it (the
  first index from which `widths` stay at the last width); ragged adds
  `Rag { stretch: 0, width_fraction: 1/6, short_last: 1/3, short_last_unit: 1/3 }`. Its two
  `compose` calls become two `knuth_plass` calls (tolerance 1, then 4 or ∞); `None` still falls
  back to the single-line composer.
- **designcraft** `breaker::items` builds `linebreak::Item`s: `hang`, `edge` (mojikumi
  end/start elasticity), `overflow` (`space_overflow`), `hyphenation` (`auto`), `emergency`.
  Ragged text uses plain spaces and `Rag { stretch: ragged_stretch, width_fraction: 0,
  hyphenation_zone: hyph_zone }` instead of glue/penalty/glue triples. `Spacing` maps to
  `Params { hyphen_limit, overrun: FIT_EPS, uniform_from, .. }` with `uniform_from` the number
  of drop-cap lines, else 1 with a first-line indent, else 0. Kinsoku priority 1 or 2 maps
  to `stretch_weight` or `shrink_weight` 4; `TIER_COST` is `tier_cost`. `knuth_plass` becomes
  `overlong` + emergency items + `break_paragraph(.., &[2.5, 12.0, f64::INFINITY])`; `greedy`
  becomes `greedy_with` (tabs through `advance`; `Item::no_break()` where an emergency split is
  not allowed); `balanced` becomes `balanced`. Mapping lines back to glyphs
  (`breaks_from_positions`) stays in the app.
- **effectcraft** `every_line`: a box per segment between line-break opportunities, glue for its
  trailing whitespace, `Rag { width_fraction: 1.0, .. }`, `uniform_from: 1` (the first line's
  measure differs when it is indented) and `break_paragraph(.., &[f64::INFINITY])`.

Differences from the seeds:

- vectorcraft: a Justify All last line is accepted at any ratio, like every line that ends at a
  forced break.
- designcraft: fill glue stretches infinitely instead of by 10 000 units. In ragged text a line
  ending at a forced break keeps its neighbour's fitness class. Greedy counts every flagged break
  toward the hyphen limit, and splits overlong words at the composer's emergency breaks instead
  of scanning for kinsoku. Past 4096 active breaks a pass fails and `break_paragraph` falls back
  to greedy, reported as `Method::Greedy`. Lines before `uniform_from` (first-line indent, drop
  cap) are kept apart by line number, where designcraft treats every line alike.
- effectcraft: lines are rated by Knuth's cubed ratio, not squared slack.

## Licenses

The word list is public domain; the trained patterns are CC0-1.0. hypher's patterns come from
hyph-utf8 under their own permissive licenses (LPPL, MPL, MIT, BSD-3); see the repository's
[NOTICE](../../NOTICE) and [ATTRIBUTION.md](../../ATTRIBUTION.md).

## Checks

Some tests depend on language features, so run the suite with them on:

```sh
cargo test -p craft-text --all-features
```
