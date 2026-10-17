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
    let (i18n, problems) = craft_i18n::I18n::new(CRAFT_I18N);
    problems.iter().for_each(|p| log::warn!("translation catalog: {p}; that string shows in English"));
    i18n
});

thread_local! { static CURRENT: Cell<craft_i18n::Lang> = const { Cell::new(craft_i18n::Lang::EN) }; }

macro_rules! tl { ($s:expr) => { $crate::I18N.t(CURRENT.get(), $s) }; }
```

The app keeps its `tl!` macro, its language setting (`"auto"` → `I18N.system_lang()`, otherwise
`I18N.from_code`) and its picker (`I18N.languages()`, `I18N.name(lang)`).

## Behaviour

- **Fallback:** `pt-br` app rows → `pt-br` shared rows → `pt` → the English source text. App rows
  override shared ones in their own locale. Only a region falls back (`sr-latn` never falls back to
  Cyrillic `sr`). A missing translation never errors.
- **Lookups:** `t` (plain), `tc` (with a context such as `menu` or `@msg`, falling back to plain),
  `tid` (an `@id` command id, falling back to the English label), `trn` (a count, `@plural` rows,
  `{n}` filled), `get` (one context, no fallback: a key cap's name under `key`, or "is there a
  translation?"), and `fmt` for `{name}` placeholders, filled in one pass.
- **Bad rows** (malformed, an unknown `@` context, a wrong plural form count) are skipped and
  returned by `I18n::new` for the app to log; craft-i18n's CI rejects them before they ship.
- **Plurals:** forms are listed in CLDR order (zero, one, two, few, many, other) for the categories
  whole numbers 0 to 1000 reach, from `intl_pluralrules`; a count in any other category takes the
  last form. Keep `intl_pluralrules` on the same version as craft-i18n's checker.
- **Names:** a language's name comes from its shared catalog's `@id language.name` row, else its
  code; English is "English".
- **Right to left:** `rtl` is true for `ar`, `fa`, `he` and `ur`; putting text into display order
  is the app's job.
- **System locale:** `system_lang` matches `sys-locale`'s list (`LANG`, macOS and Windows preferred
  languages, the browser's on the web): `fr_CA.UTF-8` → `fr`, `zh-TW` → `zh-hant`, `pt-PT` →
  `pt-br` when that is the only Portuguese, `C`/`POSIX` → English.
- **Pseudo locale** (`pseudo`, test-only, never in `languages()`): every lookup comes back as
  `⟦…⟧`, line by line, so text drawn without the marks never went through translation.
