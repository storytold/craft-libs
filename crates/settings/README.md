# Craft Settings

`craft-settings` stores an application's settings in `config.toml`: it picks the folder, loads
without ever failing, saves atomically while the application runs, keeps a backup, moves damaged
files aside and runs schema migrations. Each application keeps its own settings type and
defaults. The suite-wide policy (what belongs where) is in
[docs/settings-storage.md](../../docs/settings-storage.md).

Atomic writes come from [`craft-atomic`](../atomic/README.md).

## Boundaries

- Never crashes: every error comes back in a `LoadReport`, `TickResult` or `Result`; the crate
  denies `unwrap`, `expect`, `panic` and friends, and the workspace forbids `unsafe`.
- No application code: it depends only on `craft-atomic`, `serde`, `toml_edit` and `dirs`, and
  knows nothing about any app's settings, defaults, UI or file formats. Apps supply their type
  and their own parsers for older files.
- No UI: it reports; the application decides what to show.

## Defining settings

```rust
use craft_settings::{DocumentMut, MigrateError, Settings};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
struct Prefs {
    general: General,
    appearance: Appearance,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
struct General { language: String, autosave_minutes: u32 }

#[derive(Serialize, Deserialize, Default, Clone, PartialEq)]
#[serde(default)]
struct Appearance { theme: String, ui_scale: f64 }

impl Settings for Prefs {
    const APP_ID: &'static str = "photocraft";
    const SCHEMA_VERSION: u32 = 1;

    fn migrate(from: u32, doc: &mut DocumentMut) -> Result<(), MigrateError> {
        // Called once per step for files older than SCHEMA_VERSION, before typed parsing.
        Ok(())
    }
}
```

Rules for the type:

- Put `#[serde(default)]` on every struct, so a missing key falls back to its default.
- Do not use `#[serde(deny_unknown_fields)]`: keys the application does not know are kept in
  the file on purpose.
- Each top-level field becomes a `[section]`. A value that does not fit resets only its section.

## Using the store

```rust,ignore
use craft_settings::{Location, SaveStatus, SettingsStore};

// Once at startup. Never fails; look at the report for anything to show the user.
let (mut store, report) = SettingsStore::<Prefs>::open(Location::Auto);
if report.has_problems() {
    // report.broken_moved_to, report.section_errors, report.read_only
}

// Every frame: read is in-memory.
let scale = store.get().appearance.ui_scale;

// On change: only marks the store dirty if the value actually changed.
store.update(|p| p.appearance.theme = "dark".into());

// Every frame: saves 500 ms after the last change, retries failures with backoff.
let tick = store.tick(std::time::Instant::now());
if let SaveStatus::Failed(err) = &tick.status {
    // show it; the store keeps retrying
}
if let Some(deadline) = tick.next_deadline {
    // eframe only repaints on input, so ask for one at the deadline or an idle app never saves:
    ctx.request_repaint_after(deadline.saturating_duration_since(std::time::Instant::now()));
}

// On exit.
let _ = store.flush();
```

`SettingsStore::open_at(dir)` opens a store in a given folder, for settings that travel with a
library or project. `SettingsStore::open(Location::Memory)` keeps settings in memory only and
reads and writes nothing, for tests, demos and switches like `DESIGNCRAFT_NO_PREFS`; the app
uses the same `get`/`update`/`tick`/`flush` calls either way. `OpenOptions` adds older files to
import, a different debounce, and a `Probe` for tests.

## Where the file goes

`Location::Auto` applies these rules in order; the report says which one won:

1. `Location::Dir(path)`: an application override, such as a `--config-dir` flag.
2. `<APP>_CONFIG_DIR` (`PHOTOCRAFT_CONFIG_DIR`). Set but **empty** means isolated: the store
   is in memory only and nothing is read or written, so tests and agents never touch the real
   profile.
3. A portable marker next to the executable (next to the `.app` bundle on macOS): by default a
   file named `portable`, with settings in `config/` beside it. An app with an existing portable
   layout keeps it by overriding `Settings::PORTABLE_MARKERS` and `Settings::PORTABLE_DIR`:

   ```rust,ignore
   const PORTABLE_MARKERS: &'static [&'static str] = &["portable.txt", "PhotoCraft.portable"];
   const PORTABLE_DIR: &'static str = "PhotoCraftData";
   ```
4. The platform config folder: `$XDG_CONFIG_HOME/<app>/` or `~/.config/<app>/` on Linux,
   `~/Library/Application Support/<app>/` on macOS, `%APPDATA%\<app>\` on Windows.

With no folder (isolated or `Location::Memory`), `store.dir()` and `store.path()` are `None`,
`is_persistent()` is false, legacy files are not imported, and `flush()` returns `Ok`.

Otherwise the folder is created at open. If it cannot be created or written, the store opens read-only
and the report says so, so the application can tell the user at startup rather than on the
first save. `paths::cache_dir` and `paths::data_dir` give the matching cache and data folders for
things that do not belong in `config.toml`.

## Files

- `config.toml`: the settings, starting with `schema_version = N`. Written on the first
  `flush()` even when nothing changed, so users can find it. Created `0600` on Unix; a mode the
  user changes by hand is kept on later saves.
- `config.toml.bak`: the previous version, refreshed before each save. On Unix it always has
  the same mode as `config.toml`; an older, broader backup is tightened on the next save.
- `config.broken-<unix-ts>.toml`: a file that was not valid TOML, moved aside. Never deleted.

## What happens when loading

| File state | Result |
|---|---|
| Missing | Defaults, or the first `LegacyFile` that parses. The old file is left in place so a rollback still works. |
| Not valid TOML | Moved aside; defaults; `report.broken_moved_to`. If it cannot be moved, read-only. |
| Valid TOML, a section does not fit | That section's defaults; the rest kept; `report.section_errors`. |
| Older `schema_version` | `Settings::migrate` once per step, then saved; `report.migrated_from`. |
| Newer `schema_version` | Values shown, store read-only, so an older build never overwrites a newer file. |
| Migration step fails | Store read-only, file untouched. |

## What happens when saving

Values are merged into the document that was loaded: tables merge key by key, arrays and
scalars are replaced whole, comments, key order, unknown keys and each replaced line's trailing
comment stay. Each write goes through `craft-atomic`, so a failed save leaves the old file intact.

Just before each write the store reads the file and compares its **contents** (not its mtime,
which can stay the same on FAT, HFS+ or network shares) with what it last read or wrote:

| File on disk | What the save does |
|---|---|
| Unchanged | Backs it up to `config.toml.bak`, then writes. |
| Changed, parses | Adopts it so its comments and unknown keys survive, backs it up, then writes this store's values over its known keys. |
| Changed, does not parse | Moves it to `config.broken-<ts>.toml` (the edit is kept; `take_moved_aside()` returns the path to show), leaves the previous backup alone, then writes. |
| Blank | Leaves the previous backup alone, then writes. |
| From a newer build, or its migration fails | Writes nothing; the store becomes read-only and the save returns `SettingsError::ReadOnly`, the same as at startup. |

### Concurrency contract

Saving is **best effort, last writer wins**, not multi-process locking. Hand edits and a
second instance's unknown keys and comments are preserved as above, but two processes saving
within the same moment can still race: the later write wins on known keys. Settings are small
and change rarely, so this is the contract apps should assume. An app that needs strict
coordination between processes (for example a launcher writing another app's settings) should
add its own lock around `flush()`.

## Validation

Before merging (from `AGENTS.md`), plus the wasm check:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --target wasm32-unknown-unknown -p craft-settings
```

The tests in `tests/` write only to temporary folders and never touch the real config folder.

On `wasm32` only the `Settings` trait, the document helpers and the error types are available.
