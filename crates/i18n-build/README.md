# craft-i18n-build

Embeds the optional [craft-i18n](https://github.com/storytold/craft-i18n) translation catalogs
from an app's `build.rs`, as the `CRAFT_I18N` table that
[`craft-i18n`](../i18n/README.md)'s `I18n::new` takes. No dependencies.

Apps never list the craft-i18n catalog repository in `Cargo.toml`: Cargo would fetch it for every
build. The build reads a local checkout named by `CRAFT_I18N_DIR` instead, following craft-i18n's
[`docs/integration.md`](https://github.com/storytold/craft-i18n/blob/main/docs/integration.md).

| Environment | Result |
|---|---|
| `CRAFT_I18N_DIR` unset or empty | Only the app's own catalogs (`app_dir`), without shared rows; none without them. |
| `CRAFT_I18N_DIR` = a craft-i18n checkout | Every locale with the app's catalog, with its `common.tsv`, and the locale each regional one falls back to. |
| `CRAFT_I18N_DIR` relative, without `locales/`, or with a catalog that can't be read | `cargo::warning`; the checkout is not used. |
| `CRAFT_I18N_REQUIRED=1` (exactly `1`; other values are ignored) | Error instead when the checkout is unset or can't be used. |

Give `CRAFT_I18N_DIR` as an absolute path: Cargo runs build scripts in the package's directory,
and `include_str!` resolves a relative path against the generated file.

## Using it

`Cargo.toml` (same `rev` as the app's other craft-libs crates):

```toml
[dependencies]
craft-i18n = { git = "https://github.com/storytold/craft-libs", rev = "…" }
log = "0.4"

[build-dependencies]
craft-fonts-build = { git = "https://github.com/storytold/craft-libs", rev = "…", features = ["subset"] }
craft-i18n-build = { git = "https://github.com/storytold/craft-libs", rev = "…" }
```

`build.rs`, with the interface faces cut to the embedded catalogs by
[`craft-fonts-build`](../fonts-build/README.md):

```rust
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let locales = match craft_i18n_build::Catalogs::new("wordcraft").write("craft_i18n.rs") {
        Ok(locales) => locales,
        Err(e) => {
            println!("cargo::error={e}");
            Vec::new()
        }
    };
    let catalogs: Vec<std::path::PathBuf> = locales.iter().flat_map(|l| l.files()).map(Into::into).collect();
    if let Err(e) = craft_fonts_build::Embed::new().subset_to(catalogs).write("craft_fonts.rs") {
        println!("cargo::error={e}");
    }
}
```

The crate:

```rust
include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

pub static I18N: std::sync::LazyLock<craft_i18n::I18n> = std::sync::LazyLock::new(|| {
    let (i18n, problems) = craft_i18n::I18n::new(CRAFT_I18N, CRAFT_I18N_OFFERED);
    problems.iter().for_each(|p| log::warn!("translation catalog: {p}"));
    i18n
});
```

The generated file defines two statics:

- `CRAFT_I18N: &[(&str, &str, &str)]`, the same type as `craft_i18n::Embedded`: (locale, shared
  catalog, the app's catalog) for each embedded locale, `""` for a missing catalog, each embedded
  with `include_str!`.
- `CRAFT_I18N_OFFERED: &[&str]`: the locales with the app's own catalog. An app that keeps its
  own language list passes that to `I18n::new` instead.

`Catalogs::new(app)` reads `locales/<locale>/<app>.tsv` and `common.tsv`; `app` is the catalog
name, lowercase letters, digits and `-`. The builder takes:

- `app_dir(path)`: the app's own catalogs, `<path>/<locale>/<app>.tsv`, for an app that keeps
  some in its repository. Their locales are offered even when the checkout lacks them. When the
  checkout has the same locale's app catalog, the two are merged into one file under `OUT_DIR`:
  the rows here, then the checkout's rows whose context and source text are not defined here by
  a row that craft-i18n accepts. A row here that craft-i18n skips (a trailing `…` or placeholders
  that differ from the source text, an `@plural` row with another number of forms than the
  checkout's) leaves the checkout's row in place. Comments and blank lines are left out of the
  merged file, so a problem `check_all` reports in it gives that file's line numbers. A relative
  path is under the package directory. A catalog here that can't be read is always an error.
- `dir(path)` and `required(bool)` override the two environment variables.

`write` returns the embedded `Locale`s (`code`, `shared`, `app`, `offered()`, `files()`).

## Rules

- Locale folders are matched in lowercase. `en` (the English source rows), `pseudo` and names
  starting with `.` are never embedded. Two folders that differ only in case are an error, and so
  is an entry of `locales/` whose type can't be read.
- A locale with only `common.tsv` is embedded only as the parent of a regional locale that has
  the app's catalog: `pt` for `pt-br`, `zh-hant` for `zh-hant-hk`. Only a last subtag of two
  letters or three digits is a region, so `sr-latn` never brings in Cyrillic `sr`. Without the
  parent, `pt-br` would show English where only `pt` has a row.
- Each catalog must be a regular file of UTF-8 text of at most `MAX_CATALOG_BYTES` (8 MiB), with
  a UTF-8 path. Anything else under a catalog's name (a folder, a device, a FIFO) counts as a
  catalog that can't be read, and is never opened.
- Paths are used as given and never canonicalized: on Windows that gives `\\?\C:\…`, which
  `include_str!` can't read.
- `cargo::rerun-if-changed` names the checkout's `locales/` folder and the app's catalog folder,
  which Cargo scans for changes, and `cargo::rerun-if-env-changed` both variables. A path with a
  line break is an error. A checkout without `locales/` is not watched: after adding the folder,
  change `build.rs` or run `cargo clean -p <package>` to rebuild.
- The table is written on every run, empty on an error. Files whose bytes did not change are not
  rewritten, so Cargo does not rebuild the crate that includes them.
