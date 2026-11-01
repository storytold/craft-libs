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
check_schema(head.schema)?;
```

- `SCHEMA_VERSION` is the newest project schema, 2.
- `check_schema(schema) -> Result<(), TooNew>` refuses a schema newer than `SCHEMA_VERSION`.
  `TooNew` reads "project schema 3 is newer than this version supports (2)".
- `File { saved_by, view, project }` serializes as one object, with `view` and `project`
  flattened into it. `to_json()` gives the file's text indented with two spaces, and
  `to_json_compact()` gives it on one line. Neither checks the values: `serde_json` writes NaN and
  infinity as `null`, so the caller checks its values before saving.
- `Head::read(text)` reads `savedBy` and `schema`, and skips other members. `savedBy` is `None`
  when it is absent, is not a string, or appears more than once. A JSON array whose first element
  is the schema also reads as a `Head`, with no `savedBy`.
- `saved_by(text) -> Option<String>` reads `savedBy` alone, and gives `None` when the text is not
  a JSON object or `savedBy` is not a string or appears more than once.

`.ecprojx` files hold the same object encoded as XML, which this crate does not read or write.
