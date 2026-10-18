# craft-credits

The About window's credits for a Craft app: every contributor (GitHub login always; display and real
names only from the person's own verified `contributors/people.toml` entry) and the AI models named in
`Co-Authored-By` trailers (craftrules `standards/contributors.md`). The tables are baked into the
binary at build time; nothing is read at run time.

It replaces the `crates/ui-egui/build.rs` credits code (the same in all 13 apps) and the data model,
sorting and formatting half of each app's `credits.rs`. The window itself (layout, translation,
links, automation ids) stays in each app.

## Use

```toml
[dependencies]
craft-credits = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }

[build-dependencies]
craft-credits = { git = "https://github.com/storytold/craft-libs", rev = "<commit>", features = ["build"] }
```

`build.rs`:

```rust,ignore
fn main() {
    let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    craft_credits::build::write_credits(&manifest.join("../../contributors"));
}
```

The app's `credits.rs`:

```rust,ignore
use craft_credits::{Contributor, NameMode, SortKey};

include!(concat!(env!("OUT_DIR"), "/credits.rs")); // TOTAL_COMMITS, CONTRIBUTORS, MODELS

/// The contributors in the order the About window shows them.
pub fn in_order(names: NameMode, key: SortKey, ascending: bool) -> Vec<&'static Contributor> {
    craft_credits::sorted(CONTRIBUTORS, names, key, ascending)
}
```

- `Contributor::name`, `summary` / `summary_in` (a translated `SUMMARY` template), `lines_delta`.
- `SortKey::label`, `cell`, `numeric`, `default_ascending`, `id`; `NameMode::label`, `id`.
- `fmt::group`, `signed`, `day`, `fill`; `model_share`.
- `UI_STRINGS`: every English string the crate hands out, for the app's translation-coverage test.

A missing or malformed `contributors.json` or `people.toml` gives empty tables (or usernames only)
and a cargo warning, never a failed build.
