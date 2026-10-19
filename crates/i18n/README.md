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
    let (i18n, problems) = craft_i18n::I18n::new(CRAFT_I18N, &offered);
    problems.iter().for_each(|p| log::warn!("translation catalog: {p}; that string shows in English"));
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
  `@id` command id, else the English label), `trn` (a count, `@plural` rows, `{n}` filled), `get`
  (one context along the locale chain, so `pt-br` can return a `pt` row, but never the plain row
  or English: a key cap's name under `key`, or "is there a translation?"), and `fmt` for `{name}`
  placeholders, filled in one pass.
- **Bad rows** (malformed, an unknown `@` context, a wrong plural form count, a row defined twice,
  of which the first is kept) and bad locales (`en`, `pseudo`, a code embedded twice in any case)
  are skipped and returned by `I18n::new` for the app to log; craft-i18n's CI rejects them before
  they ship.
- **Plurals:** forms are listed in CLDR order (zero, one, two, few, many, other) for the categories
  whole numbers 0 to 1000 reach, from `intl_pluralrules`; a count in any other category takes the
  last form. CLDR's `pt` is Brazilian, so `pt` uses `pt-PT`'s rules (0 is plural) and `pt-br`
  `pt`'s. `intl_pluralrules` is pinned to the version craft-i18n's checker uses, and
  `plural_form_counts` holds the same counts.
- **Names:** a language's name comes from its shared catalog's `@id language.name` row, else its
  code; English is "English".
- **Right to left:** `rtl` is true for `ar`, `ckb`, `dv`, `fa`, `he`, `ps`, `sd`, `ug`, `ur` and
  `yi`; putting text into display order is the app's job.
- **System locale:** `system_lang` matches the system's list once and caches it: on desktop Unix
  `LANGUAGE`, `LC_ALL`, `LC_MESSAGES` and `LANG`, read raw; elsewhere `sys-locale`'s (macOS and
  Windows preferred languages, the browser's on the web). `fr_CA.UTF-8` → `fr`, `zh-TW` →
  `zh-hant`, `pt-PT` → `pt-br` when that is the only Portuguese, `C`/`POSIX` → English. A script
  is never crossed: `sr_RS@latin` and `sr-Latn-RS` pick `sr-latn`, never `sr`, while `sr-Cyrl-RS`
  picks `sr`.
- **Pseudo locale** (`pseudo`, debug builds only, never in `languages()`): every lookup, `get`
  included, comes back as `⟦…⟧`, line by line, so text drawn without the marks never went through
  translation. `looked_up()` lists the source texts looked up so far, for an audit against a
  complete catalog.
