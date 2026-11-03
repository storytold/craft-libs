# craft-ecproj-format

The envelope of EffectCraft `.ecproj` project files: the members around the project that
EffectCraft writes and other apps read. The project model, the saved view and the schema 1
conversion stay in EffectCraft. No `unsafe`, no panics, and it builds for
`wasm32-unknown-unknown`. The dependencies are `serde`, `serde_json` and `thiserror`.

```toml
craft-ecproj-format = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## The file

A project file is one UTF-8 JSON object with no newline at the end: `savedBy`, then the saved
view's members (`activeComp`, `openComps`, `compTimes`, each left out when empty), then the
project's members, starting with `schema`. Projects saved before `savedBy` was added start with
`schema`. EffectCraft indents the object with two spaces, or writes it on one line when Settings ▸
Project ▸ Save Compact Project Files is on. The crate writes and reads both forms.

## Use

```rust
use craft_ecproj_format::{File, Head, check_schema};

// Writing
check_schema(project.schema)?;
let text = File::new(APP_VERSION, &view, &project).to_json()?;

// Reading
let head = Head::read(&text)?;
```

- `SCHEMA_VERSION` is the newest project schema, 2.
- `check_schema(schema) -> Result<(), TooNew>` refuses a schema newer than `SCHEMA_VERSION`.
  `TooNew` reads "project schema 3 is newer than this version supports (2)". It is
  `#[non_exhaustive]`: other crates read its `schema` and `supported` fields.
- `File { saved_by, view, project }` serializes as one object, with `view` and `project`
  flattened into it. `to_json()` gives the file's text indented with two spaces, and
  `to_json_compact()` gives it on one line. Neither checks the values: `serde_json` writes NaN and
  infinity as `null`, so the caller checks its values before saving.
- `Head::read(text) -> Result<Head, ReadError>` reads `savedBy` and `schema` from one JSON
  object, and skips other members. `savedBy` is `None` when it is absent, is not a string, or
  appears more than once. Text that is not a JSON object, has no `schema` that is a `u32`, or has
  two `schema` members gives `ReadError::Json`. A schema newer than `SCHEMA_VERSION` gives
  `ReadError::TooNew { too_new, saved_by, .. }`, which reads as `TooNew` does and holds the file's
  `savedBy`, so a message can name the version that wrote it. `ReadError` and its `TooNew`
  variant are `#[non_exhaustive]`: match them with a wildcard arm and `..`.

`.ecprojx` files hold the same object encoded as XML, which this crate does not read or write.
