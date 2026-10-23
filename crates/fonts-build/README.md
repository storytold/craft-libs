# craft-fonts-build

Embeds the optional [craft-fonts](https://github.com/storytold/craft-fonts) faces from an app's
`build.rs`, and defines `EmbeddedFace`, the plain data type of the generated table. No
dependencies without the `subset` feature, and no egui, so crates below the UI layer (text engines,
exporters) can use it.

Apps never list craft-fonts in `Cargo.toml`: Cargo would fetch the font repository for every
build. The build reads a local checkout named by `CRAFT_FONTS_DIR` instead.

| Environment | Result |
|---|---|
| `CRAFT_FONTS_DIR` unset or empty | Empty table. |
| `CRAFT_FONTS_DIR` = a craft-fonts checkout | Every selected face of `fonts/manifest.txt`, embedded with `include_bytes!`. |
| `CRAFT_FONTS_DIR` set, but no manifest or a missing font file | `cargo::warning`, empty table. |
| `CRAFT_FONTS_REQUIRED=1` (exactly `1`; other values are ignored) | Error instead when the directory is unset, unreadable, has no selected face, or has no face for a script passed to `scripts`. |

Give `CRAFT_FONTS_DIR` as an absolute path: Cargo runs build scripts in the package's directory.

## Using it

`Cargo.toml` (same `rev` as the app's other craft-libs crates). The `subset` feature goes on the
build-dependency only: it brings in the subsetter ([skera](https://crates.io/crates/skera) and the
fontations crates), which the app itself never needs. Cargo's resolver 2 and later (edition 2021
and later) keep build-dependency features apart from the app's, so the dependency stays without
it.

```toml
[dependencies]
craft-fonts-build = { git = "https://github.com/storytold/craft-libs", rev = "…" }

[build-dependencies]
craft-fonts-build = { git = "https://github.com/storytold/craft-libs", rev = "…", features = ["subset"] }
```

`build.rs` (FilmCraft's `crates/text/build.rs`, in full):

```rust
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let wasm = craft_fonts_build::target_is_wasm();
    let embedded = craft_fonts_build::Embed::new()
        // CJK faces only as the interface faces, cut to the characters FilmCraft's catalogs use.
        .subset_to(catalogs("filmcraft"))
        // Web builds have a size budget: embed only the Japanese interface face there.
        .filter(move |face| !wasm || face.has_script("Jpan"))
        .write("craft_fonts.rs");
    if let Err(e) = embedded {
        println!("cargo::error={e}");
    }
}

/// Every locale's shared and app catalog in the craft-i18n checkout `CRAFT_I18N_DIR` names, the
/// files the app's translations embed; none when it is unset.
fn catalogs(app: &str) -> Vec<PathBuf> {
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_DIR");
    let Some(dir) = std::env::var_os("CRAFT_I18N_DIR").filter(|dir| !dir.is_empty()) else { return Vec::new() };
    let locales = PathBuf::from(dir).join("locales");
    println!("cargo::rerun-if-changed={}", locales.display());
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&locales)
        .into_iter()
        .flatten()
        .flatten()
        .flat_map(|locale| ["common.tsv".to_owned(), format!("{app}.tsv")].map(|file| locale.path().join(file)))
        .filter(|path| path.is_file())
        .collect();
    paths.sort();
    paths
}
```

The crate:

```rust
use craft_fonts_build::EmbeddedFace;

/// craft-fonts faces; empty unless built with `CRAFT_FONTS_DIR`.
pub static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));
```

`Embed::new()` selects every face except the sources of the interface faces (`role` `ui`, see
below), and `Embed::documents(&["Noto Sans", …])` only the families it names (see
[Document fonts](#document-fonts)). The builder narrows the set:

- `scripts(&["Jpan", "Arab", "Hant"])` keeps faces with any of these script tags. A tag matches
  itself and its region forms (`Hant` matches `Hant-HK`). With `CRAFT_FONTS_REQUIRED=1`, each tag
  must match at least one face.
- `filter(|entry| …)` keeps faces by any manifest field (family, style, licence, index).
- `for_ui()` keeps CJK faces only as the interface faces (see below).
- `subset_to(catalogs)` (feature `subset`) cuts the interface faces to the catalogs' characters;
  it implies `for_ui()`.
- `check_text(paths)` warns about characters in these files that no embedded face covers.
- `embedded_elsewhere(&["Noto Sans Math", …])` names families another table of the app embeds,
  so `check_text` and the catalog check count their characters as covered.
- `dir(path)` and `required(bool)` override the two environment variables.

`write` returns the embedded `ManifestEntry`s, for build-time checks such as "the interface face
is present". The generated file is a slice expression of `EmbeddedFace { family, style, scripts,
index, bytes }` literals; any struct with those fields and that name in scope works too.
`EmbeddedFace` itself has `name()`, `has_script(tag)`, `is_cjk()` and `weight()`.

## Document fonts

An app that draws document text (titles, layouts, vector text) embeds the families its documents
use, and only those, in a table of its own:

```rust
let embedded = craft_fonts_build::Embed::documents(&["Noto Sans", "Shippori Mincho"])
    // The interface table holds the CJK cuts and the symbol faces.
    .embedded_elsewhere(&["Noto Sans Math", "Noto Sans Symbols 2"])
    .write("document_fonts.rs");
```

Each named family brings all its styles, as whole files: a CJK document face takes 5–17 MB, so a
release embeds exactly the faces the app's documents need. A family named here is embedded even
when it is also an interface face's source (`BIZ UDPGothic` brings its Regular too). A name the
manifest lacks is an `Error::UnknownFamilies` listing the manifest's families: a build error with
`CRAFT_FONTS_REQUIRED=1`, and a `cargo::warning` with an empty table otherwise, as for any
checkout that can't be used. `scripts` and `filter` narrow the families further, for example to
their Regular faces.

## Interface faces

The full CJK faces take 5–17 MB each. For the interface, craft-fonts marks one Regular TrueType
face per CJK script with the role `ui`: `Noto Sans SC UI` (`Hans`), `Noto Sans TC UI` (`Hant`),
`Noto Sans HK UI` (`Hant-HK`), `Noto Sans KR UI` (`Kore`) and BIZ UDPGothic Regular (`Jpan`). The
app's build cuts them to the characters its translation catalogs use; for FilmCraft's catalogs
the five cuts take about 3 MB together.

- `for_ui()` drops every face with a CJK script tag (`Hans`, `Hant`, `Hant-HK`, `Jpan`, `Kore`,
  `Hani`) that is not an interface face, and keeps the rest. Document text embeds the full faces
  it names with `Embed::documents` instead. Without `for_ui()` or `subset_to`, the interface
  faces' sources are left out unless `Embed::documents` names their family.
- `subset_to(catalogs)` cuts each interface face to the characters the catalogs use from U+2E80 up
  (CJK and later blocks, without emoji, variation selectors and tags), plus CJK punctuation
  (U+3000–303F), kana (U+3040–30FF, U+31F0–31FF) and full-width forms (U+FF00–FFEF), and keeps
  every layout feature, so vertical forms (`vert`) still work. Every face gets the characters of
  every catalog, so a Japanese label in a Chinese catalog still draws. The cuts are written to
  `OUT_DIR/craft-fonts-ui/` and embedded from there; the same inputs give the same bytes, for
  reproducible releases. Cutting all five faces takes well under a second.
- Without `subset_to`, or with an empty catalog list, `for_ui()` leaves the interface faces out
  with a `cargo::warning`: a full CJK face is never embedded for the interface by accident. A
  checkout whose manifest has no interface face is a `cargo::warning` too. With `CRAFT_FONTS_REQUIRED=1` that
  makes `scripts(&["Hant"])` an error; with catalogs, an interface face satisfies it.
- An interface face with CFF outlines is an error (`Error::NotTrueType`), even when not required:
  the subsetter cuts TrueType (`glyf`) faces correctly, but not CFF ones.

The catalogs come from craft-i18n. An app's build reads them from the checkout `CRAFT_I18N_DIR`
names (craft-i18n's `docs/integration.md`); the fonts build script reads the same variable, as in
the example above. Catalogs kept in the app's own repository are passed by path, relative to the
package directory.

A character the faces lack still shows: craft-ui's system font provider draws it with an installed
font. The build says so: for each catalog, and each file passed to `check_text`, it prints one
`cargo::warning` with the count and the first few characters from U+2E80 up that no embedded face
covers. Since the cuts hold every catalog character their sources have, these are the characters
the sources lack. Faces the app embeds in another table count as covering their characters when
named with `embedded_elsewhere`; they are read whole from the checkout, so a face cut elsewhere is
exact only for the catalogs it was cut to. It also reruns the build script when a file changes.

## The manifest

One face per line, eight or nine fields separated by `" | "`; craft-fonts' `docs/integration.md`
defines them. Lines starting with `#` are comments. A file field ending in `#<n>` names face `n` of
a font collection (`fonts/noto-sans-cjk/NotoSansCJK.ttc#2`); without it the index is 0. The ninth
field, `role`, is `ui` on interface faces and absent elsewhere. Fields after the ninth are ignored.
