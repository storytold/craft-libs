# craft-i18n

Translations for the Craft apps: lookup with fallback, plurals, language names and the system
locale, over the catalogs of [craft-i18n](https://github.com/benandez/craft-i18n) (the data
repository, same name).

The catalogs are a fully optional build input. An app's `build.rs` embeds them from
`CRAFT_I18N_DIR` as a static `CRAFT_I18N: &[(locale, shared catalog, app catalog)]`, empty when the
variable is unset; the recipe is in craft-i18n's
[`docs/integration.md`](https://github.com/benandez/craft-i18n/blob/main/docs/integration.md).
This crate never reads files.

## Wiring an app

```rust
use std::cell::Cell;
use std::sync::LazyLock;

include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs")); // pub static CRAFT_I18N: &craft_i18n::Embedded

pub static I18N: LazyLock<craft_i18n::I18n> = LazyLock::new(|| {
    let offered: Vec<&str> = LANGUAGES.iter().map(|l| l.code).collect(); // the app's own list
    let (i18n, problems) = craft_i18n::I18n::new(CRAFT_I18N, &offered); // parses nothing yet
    problems.iter().for_each(|p| log::warn!("translation catalog: {p}"));
    i18n
});

thread_local! { static CURRENT: Cell<craft_i18n::Lang> = const { Cell::new(craft_i18n::Lang::EN) }; }

macro_rules! tl { ($s:expr) => { $crate::I18N.t($crate::CURRENT.get(), $s) }; }
```

The app keeps its `tl!` macro, its language setting (`"auto"` → `I18N.system_lang()`, otherwise
`I18N.from_code`) and its picker (`I18N.languages()`, `I18N.name(lang)`).

## Behaviour

- **Offered locales:** `I18n::new` takes the codes the app offers (its own language list), so an
  app never offers a language its interface isn't ready for (Arabic without right-to-left layout,
  say). `languages()`, `from_code`, `match_tag` and `system_lang` only return offered locales; an
  embedded locale that isn't offered still serves as a fallback (`pt` for an offered `pt-br`).
- **Fallback:** locale by locale along the chain, `pt-br` → `pt` → the English source text. In
  each locale, app rows override shared ones, and `tc`/`tid` try their context or `@id` row before
  the plain row, so `pt-br`'s plain row beats `pt`'s `menu` row. Only a region falls back (`pt-br`
  → `pt`, `zh-hant-hk` → `zh-hant`); `sr-latn` never falls back to Cyrillic `sr`. A missing
  translation never errors.
- **Lookups:** `t` (plain), `tc` (with a context such as `menu` or `@msg`, else plain), `tid` (an
  `@id` command id, else the English label), `trn` and `trn_decimal` (a count, `@plural` rows,
  `{n}` filled), `get` (one context along the locale chain, so `pt-br` can return a `pt` row, but never the plain row
  or English: a key cap's name under `key`, or "is there a translation?"), and `fmt` for `{name}`
  placeholders, filled in one pass.
- **Isolation:** when a message or a value inserted into it has a right-to-left letter (bidi
  class R or AL), or a value has a bidi control (LRE, RLE, PDF, LRO, RLO, LRI, RLI, FSI, PDI,
  LRM, RLM, ALM), `fmt` wraps each inserted value in a first-strong isolate (FSI…PDI), and `trn`,
  `file_size` and `measure` wrap `{n}` in a left-to-right one (LRI…PDI), so a Latin file name in an Arabic sentence, a
  Hebrew name in an English one, or an ASCII file name with a right-to-left override can't
  reorder the words around it. Embeddings, overrides and isolates a value opens are closed inside
  its isolate, and a stray PDF or PDI in it is dropped. Wrap numbers, paths and codes in `ltr`
  before passing them; a value that already is one isolate keeps it. Other text comes out
  without marks, as if `fmt` didn't isolate at all. egui draws the marks as nothing. For text
  that leaves the interface (file names, logs, terminals, text parsed later), use `fmt_plain` or
  `strip_isolates`.
- **Bad rows** (malformed, an unknown `@` context, a wrong plural form count, a row defined twice,
  of which the first is kept) and bad locales (`en`, `pseudo`, a code embedded twice in any case)
  are skipped; craft-i18n's CI rejects them before they ship. `I18n::new` parses nothing: it
  returns the bad locales, and each locale's rows are parsed (and its bad rows logged with
  `log::warn!`) on the first lookup that reaches it, including as a fallback. `I18n::check_all()`
  parses every locale and returns all problems (`<locale>/<common|app>: line N: …`), for a test.
- **Plurals:** forms are listed in CLDR order (zero, one, two, few, many, other) for the categories
  whole numbers 0 to 1000 reach, from ICU4X's CLDR rules; `plural_form_counts` holds the counts
  craft-i18n's checker expects. `trn_decimal` picks the form from the digits shown (ICU4X plural
  operands), so English `1.0` is plural and French `1,5` singular. A count in a category integers
  never reach takes the `few` form if there is one (Russian and Ukrainian decimals, `1,5 файла`),
  else the last. Rules are per language, so `sr-latn` and `zh-hant` take `sr`'s and `zh`'s. CLDR's
  `pt` is Brazilian, so `pt` uses `pt-PT`'s rules (0 is plural) and `pt-br` `pt`'s. A language
  CLDR has no plural rules for shows English forms.
- **Names:** a language's name comes from the app catalog's `@id language.name` row, else the
  shared catalog's, else its code; English is "English".
- **Scripts and direction:** for any language tag, `scripts` lists the ISO 15924 scripts its
  interface text is written in (`ja` → Hani, Hira, Kana; `sr-latn` → Latn; `pa-PK` → Arab),
  `direction` is right to left when the main script is (Arabic, Hebrew, Syriac, Thaana), and `han`
  gives the regional forms of its Han characters for picking a CJK face: `zh-CN`/`zh-SG`/`zh-Hans` →
  SC, `zh-TW`/`zh-US`/`zh-Hant` → TC, `zh-HK`/`zh-MO` → HK, `ja` → JP, `ko` → KR. `I18n::rtl` is
  `direction` of the language's code; putting text into display order is the app's job.
- **Reading numbers:** `Format::parse(text)` reads what the format writes (its digits, decimal
  and grouping separators, the system's included) and what people type or paste: any Unicode
  decimal digits (ASCII, Arabic-Indic, Persian, Devanagari, Bengali, Thai, full-width, …) from one
  set; `٫` as a decimal mark everywhere; grouping by the locale's other mark of `.` and `,`, spaces
  (NBSP, narrow NBSP), `'`, `’` or `٬`, in threes or the Indian 2-2-3; a leading `-`, `−` or `+`;
  bidi marks ignored. A lone `.` in a comma locale is decimal when it can't be grouping (`1.5`,
  `0.250`, `1234.5`), while `1.234` stays 1234 in German; the same holds for `,` in a point
  locale. Percent signs and units (`50 %`, `12 px`) are not read: strip them first. Other input
  that doesn't fit is an error, never a guess. `parse_number(text, locale)` is the same for
  `Format::from_tag(locale)`.
- **System locale:** `system_lang` matches the system's list once and caches it: on desktop Unix
  `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` and `LANG`, read raw; elsewhere `sys-locale`'s (macOS and
  Windows preferred languages, the browser's on the web). `fr_CA.UTF-8` → `fr`, `zh-TW` → `zh-hant`,
  `pt-PT` → `pt-br` when that is the only Portuguese, `C`/`POSIX` → English. A script is never
  crossed: `sr_RS@latin` and `sr-Latn-RS` pick `sr-latn`, never `sr`, while `sr-Cyrl-RS` picks `sr`.
  A region that implies a script counts as that script: `pa-PK` picks `pa-arab`, not Gurmukhi `pa`.
- **Sizes and units:** `file_size(lang, bytes, style)` writes `1.5 MB` or `512 bytes`; `FileSize`
  picks steps of 1024 named KB (`BINARY`), 1024 named KiB (`IEC`) or 1000 named KB (`DECIMAL`),
  the decimals per unit, no decimals from a given value up, the smallest and largest unit (below
  the smallest, sizes round up), and `B` or words for bytes. `measure(lang, value, decimals,
  "{n} px")` writes any other measure. Unit names are rows in context `unit`, keyed by the English
  template (`{n} MB` → `{n} Mo`, `{n}%` → `{n} %`) and read with `get`, so a plain `B` row (bold)
  is never used; bytes in words are the `@plural` row `{n} byte|{n} bytes`. Numbers are written
  by `I18n::format`.
- **Numbers:** `Format` writes numbers for a `Region` on ICU4X's decimal formatter: `integer`
  (`1.234.567`, Indian `12,34,567`), `number(value, decimals)` and `percent(ratio, decimals)`
  (`50 %`, Turkish `%50`). It follows the region, not the interface language, with the locale's
  digits (`١٬٢٣٤٫٥` in Egypt, `۱٬۲۳۴` in Iran) or, with `Digits::Latin`, `0`–`9`. With Latin
  digits, a decimal or grouping separator the system sets (Windows' and macOS's own settings)
  replaces the locale's. `I18n::format` is the format `trn`, `file_size` and `measure` use: the
  system region's until `I18n::set_format` (the user's region setting or digit choice), and again
  after `I18n::reset_format`. A count in `trn` is therefore grouped and in the region's digits
  (`1.234 Dateien`, `١٬٢٣٤ ملفًا`); for names written into documents or files (`Copy 1234`), use
  `trn_plain`, which writes `0`–`9` without grouping or isolate marks.
- **CLDR data:** decimal symbols, digits and plural rules (`data/numbers.postcard`, about 10 KB) and
  percent patterns (`data/locales.rs`) cover every CLDR locale; `data/generate.sh` rebuilds both
  from CLDR. The ICU4X crates are pinned to one minor version (`~2.3`), the one the data is
  generated for. The ICU4X code and data add about 90 KB to a stripped release build and 80 KB to
  wasm.
- **Formatting region:** `Region` holds how numbers, times and measurements are written, apart
  from the interface language: the number and time locales (BCP 47), metric or US measurement, A4
  or Letter, and the decimal and grouping separators (or no grouping) and clock when the system
  sets them; otherwise the number locale's CLDR data decides. `Region::system()` reads it once:
  Windows' regional format with the user's changes (`HKCU\Control Panel\International`), Apple's
  current locale (`en_PL`, `en_GB@rg=uszzzz`), `LC_ALL` or else `LC_NUMERIC`, `LC_TIME`,
  `LC_MEASUREMENT` and `LC_PAPER` each on its own over `LANG` on Unix (`C` there is English on A4,
  metric, with a `.` decimal and no grouping; `LANG=C` gives way to the preferred language), the
  preferred language on the web and Android. `Region::from_setting` takes the app's saved choice
  (`auto` or a tag like `pl-PL`). Paper and measurement follow CLDR's territory lists; tags may
  carry `-u-ms-`, `-u-hc-`, `-u-rg-` and `-u-nu-` (Apple's `@numbers=`), and a chosen digit
  system stays in the number locale.
  Ported from WordCraft's `Region`.
- **Pseudo locale** (`pseudo`, debug builds only, never in `languages()`): every lookup, `get`
  included, comes back as `⟦…⟧`, line by line, so text drawn without the marks never went through
  translation. `looked_up()` lists the source texts looked up so far, for an audit against a
  complete catalog.
- **Search** (`Folder`): `Folder::new(locale).fold(text)` gives a key for case- and
  accent-insensitive matching (`keep_accents()` makes accents count) and a map back to the original
  text, so `Folded::find` returns the byte range to highlight. Compatibility forms fold to their
  plain letters (`ﬁ`, full-width `Ａ`), `ß` to `ss`, `ς` to `σ`, digits of every script to ASCII
  (`Layer ١٢` finds `layer 12`); ignoring accents, Persian and Arabic letter forms match (`ک` and
  `ك`, `ی` and `ي`, `ة` and `ه`, `أ` and `ا`); invisible format characters (soft hyphens, joiners,
  bidi isolates) are dropped. Turkish and Azerbaijani fold `I` to `ı` when accents count; ignoring
  accents, `I`, `i`, `İ` and `ı` all match.
- **Sorting** (`Collator`, feature `collate`): ICU4X collation for the locales craft-i18n ships,
  Persian and Hindi, with digits compared by value (`Layer 2` before `Layer 10`) and `-u-co-pinyin`
  / `-u-co-stroke` for Chinese. The data (754 KB) is regenerated by `data/generate-collation.sh`;
  the feature adds about 1 MB to a release binary or wasm module (320 KB gzipped).
