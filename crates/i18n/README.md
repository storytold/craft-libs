# craft-i18n

Translations for the Craft apps: lookup with fallback, plurals, language names and the system
locale, over the catalogs of [craft-i18n](https://github.com/benandez/craft-i18n) (the data
repository, same name).

The catalogs are a fully optional build input. An app's `build.rs` embeds them from
`CRAFT_I18N_DIR` as a static `CRAFT_I18N: &[(locale, shared catalog, app catalog)]`, empty when the
variable is unset; the recipe is in craft-i18n's
[`docs/integration.md`](https://github.com/benandez/craft-i18n/blob/main/docs/integration.md),
and [`craft-i18n-build`](../i18n-build/README.md) writes that static from `build.rs`.
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
- **Ordinals:** `tord(lang, n, [one, two, few, other])` is the wording of an ordinal from the `@ordinal` row keyed by the four
  English forms (`Move to the {n}st place|…nd…|…rd…|…th place`: 1st, 2nd, 3rd, 4th, but 11th, 12th, 13th and 21st). A translation
  lists the forms of its locale's ordinal categories in CLDR order, counted from the CLDR data (one for German, Polish, Arabic,
  Japanese, Spanish and Chinese, two for French: `one` against `other`), or is a selector message. English's own rule is the
  fallback, and needs no data.
- **Typed messages and selector messages:** `tm(lang, context, "Delete this {item :noun}?", &[("item", Arg::Noun("page"))])`
  fills a message from typed values: `Arg::Text`, `Arg::Name` (a person's text, cut at 1,024 characters), `Arg::Count`,
  `Arg::Number { value, decimals }`, `Arg::Noun` and, with `dates`, `Arg::Date`. A placeholder in the source may carry its type
  (`{n :number}`, `{n :integer}`, `{when :date}`, `{item :noun}`; English shows it without the annotation). The row is the
  (context, source) of the catalogs along the locale chain, else the plain row; its translation is plain text with `{name}`
  placeholders, or **a selector message**, a small subset of MessageFormat 2 that begins with `.input`, `.local` or `.match`
  and chooses its words by a number, an ordinal or a noun's gender (`select`, below). It is allowed in a plain row (any
  context that does not start with `@`) and in a `@plural` or `@ordinal` row, not in `@id`, `@msg`, `@msgplural`, `@noun` or
  `@grammar` rows. `trn`, `trn_decimal` and `tord` take a selector message as their row's translation too, with `$n` the count. A message the language cannot show (a noun it has no row
  for, a value of the wrong type) shows in English, whole.
- **Nouns:** `@grammar ⇥ genders ⇥ masculine feminine neuter` and `@grammar ⇥ forms ⇥ accusative genitive` declare what a language
  has (from either of its catalogs, in any order); `@noun ⇥ page ⇥ feminine: strona; accusative=stronę` gives a noun's
  translation, gender and named forms (a language without gender writes the word alone). `{$item}` is the base form,
  `{$item :noun form=accusative}` a named form, and a form a noun does not list is its base form. A plain row's `{item}` takes
  the base form. The nouns are looked up in the locale that holds the row.
- **Select (`craft_i18n::select`):** `Message::parse` reads the subset: declarations `.input {$x :number}`, `:integer`,
  `:number select=ordinal`, `:noun` and `.local $y = {$x :noun form=genitive}`; one `.match` of one to three declared variables;
  variants (a key for each selector: a plural or ordinal category, an exact whole number, a gender, or `*`, then a quoted pattern
  `{{…}}`); patterns of text, `{$x}`, `{$x :noun form=<name>}` and the escapes `\{`, `\}`, `\|`, `\\`. No markup, no other
  function, no option beyond `select` and `form`. Of the variants that match, the best wins, comparing the selectors from the
  left: an exact number beats a category, and a category or a gender beats `*`. `other` is never written as a key: `*` is the
  `other` category. `check_message(kind, source, translation, locale)` applies the rules a translation must meet, which the runtime applies
  to every row it reads and a checker can apply to a file: variables are placeholders of the source or declared, a selector's
  function fits the source's type, keys are the language's categories or genders, a variant of only `*` exists, **every
  category whole numbers 0 to 1,000 reach (other than `other`) has a variant of its own**, or else an exact variant for each of
  its whole numbers (German `one` is only 1); a Polish message without `many` is an error, not a wrong form, every placeholder is in every variant (a selector's own number may be left out for an exact number
  or a category that holds exactly one such number: Arabic `zero`, `one` and `two`, Polish `one`, English `one`, so a
  translation can say "one shape" in words), forms are declared, a trailing `…` agrees with the source, and no bidi embedding,
  override or isolate is in the text (the runtime adds the isolates; the marks LRM and RLM are free). `LocaleRules::for_locale(code)`
  gives the categories, the whole numbers of each (`whole_numbers`) and the single-valued ones from the same CLDR data, so a
  checker and the runtime cannot disagree. The classic rows keep the runtime's own rules: a plain or `@plural` translation with
  a bidi control is read as it always was.
- **Caps (`craft_i18n::limits`):** a catalog file is at most 2 MiB (`MAX_CATALOG_BYTES`), 20,000 rows (`MAX_ROWS`; the rest are
  skipped) of 16 KiB each (`MAX_ROW_BYTES`); a selector message at most 4 KiB, 3 selectors, 64 variants, 8 declarations and 64
  parts in a pattern; 2,000 nouns a locale and 16 forms a noun; a person's text 1,024 characters and a filled message 4,096
  (cut, ending with `…`); 64 locales; a language tag 35 bytes and 16 preferred languages. Past a cap the row, the rest of the
  file, or the file is skipped and reported (`check_all`), and the English shows. Work is linear in a catalog's bytes and a
  message's parts.
- **Balance:** a message of `tm`, or a selector message of `trn` or `tord`, never ends with a bidi isolate, embedding or override
  open, whatever its values or its translation open and wherever the 4,096-character cap cuts it: a cut inside an isolated value
  ends with `…` and the closers of what the kept text opens (within the cap), so the text after the message in a window is
  not reordered by it.
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
- **Bad rows** (malformed, an unknown `@` context, a wrong plural or ordinal form count, a selector message that breaks a rule of
  `select`, a noun that breaks its language's `@grammar`, a row defined twice, of which the first is kept) and bad locales (`en`, `pseudo`, a code embedded twice in any case)
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
- **CLDR data:** decimal symbols, digits and plural rules, cardinal and ordinal (`data/numbers.postcard`, about 11 KB),
  percent patterns (`data/locales.rs`) and date data (`data/dates.postcard`, about 427 KB) cover
  every CLDR locale; `data/generate.sh` rebuilds all three from CLDR 49.0.0 with `icu4x-datagen`
  2.4.0, and writes both releases to `data/source.txt` for a table built from the same CLDR to read.
  The ICU4X crates are pinned to one minor version (`~2.4`), the one the data is generated for: a
  blob made for another minor version does not load, so the plural rules and dates would silently
  fall back to English forms and ISO dates (a test of the data fails then). The ICU4X code and the
  number data add about 90 KB to a stripped release build and 80 KB to wasm; the date data is
  extra.
- **Features:** `dates` and `local-time` are on by default. `dates` is everything below under
  Dates: ICU4X's date formatting and `data/dates.postcard` (about 427 KB), with jiff's civil types
  for the 400-year range extension (no time zone database and no system clock: only `jiff`'s
  `std`); `default-features = false` leaves it, and the date data, out. `local-time` (which
  implies `dates`) reads the system's UTC offset for `TimeZone::Local`: jiff's system zone, on the
  system's tz database (on Windows the one jiff bundles), and the browser's `Date` on wasm; without
  it `TimeZone::Local` reads as UTC. `collate` is off by default (Sorting, below). Dates went from
  `chrono` to `jiff` with these features: no `chrono` is in this crate's tree.
- **Dates** (feature `dates`): `DateFormat` (`I18n::dates`, `set_dates`, `reset_dates`, as `I18n::format` is for
  numbers) writes dates and times for the region's time locale and clock in the `Short`, `Medium`,
  `Long` and `Full` lengths, month and weekday names (`NameLength`), the first day of the week and
  `parse_date` for typed dates; `CalendarDate` and `DateTime` do the day arithmetic. Dates are
  always Gregorian for now: no other calendar's data is embedded, so th-TH and fa-IR write
  Gregorian years. The field order, names, digits and clock follow the region. CLDR 48 puts
  U+202F before AM and PM in English, so compare formatted strings after normalising it. If the
  data fails to load, `format` writes `YYYY-MM-DD hh:mm`. Relative times ("3 minutes ago") are
  craft-ui's `localized::relative_age`.
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
- **Pseudo locales** (`pseudo` and `pseudo-long`, debug builds only, never in `languages()`): every lookup, `get`
  included, comes back as `⟦…⟧`, line by line, so text drawn without the marks never went through
  translation; `pseudo-long` also accents the letters and makes the text 40% longer (`⟦Ṕáğé Šéţúṕ····⟧`), leaving `{name}`
  placeholders alone, so a layout that only just fits English is found. Counts, ordinals and typed messages are marked too.
  `looked_up()` lists the source texts looked up so far (plain lookups only), for an audit against a complete catalog.
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
  / `-u-co-stroke` for Chinese. The data (757 KB) is regenerated by `data/generate-collation.sh` (CLDR 49.0.0 and ICU
  release-79.1rc, ICU4X 2.4's tested ones);
  the feature adds about 1 MB to a release binary or wasm module (320 KB gzipped).
