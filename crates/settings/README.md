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
- No application code: it depends only on `craft-atomic`, `serde`, `toml_edit`, `web-time`,
  `log`, `dirs` (native) and `web-sys` (browser), and
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

// Values read from another file at startup (a custom dictionary's words) count as loaded: no save is
// scheduled, and a later save does not write them over another instance's edit.
store.update_loaded(|p| p.spelling.words = words);

// Every frame: saves 500 ms after the last change, retries failures with backoff.
// `web_time::Instant` is `std::time::Instant` on native targets and also works in a browser.
let tick = store.tick(web_time::Instant::now());
if let SaveStatus::Failed(err) = &tick.status {
    // show it; the store keeps retrying
}
if let Some(deadline) = tick.next_deadline {
    // eframe only repaints on input, so ask for one at the deadline or an idle app never saves:
    ctx.request_repaint_after(deadline.saturating_duration_since(web_time::Instant::now()));
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

   If the portable folder cannot be created or written, the platform folder is used and
   `LoadReport::warning` says why.
4. The platform config folder: `$XDG_CONFIG_HOME/<app>/` or `~/.config/<app>/` on Linux,
   `~/Library/Application Support/<app>/` on macOS, `%APPDATA%\<app>\` on Windows. `Settings::FOLDER_NAME` renames `<app>` (default `APP_ID`)
   for an app whose existing folder is spelled differently, such as `Photocraft`.

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

When another process changed the file since this store last read or wrote it, only this store's
own changes are merged into the file on disk, item by item. `Settings::merge_rule(path)` names
the rule for each key path; the default is `MergeRule::ByKey`:

| Rule | When both processes changed the item |
|---|---|
| `ByKey` | A table merges key by key, each key by its own rule; an array or a scalar takes this store's value. |
| `Whole` | This store's value replaces the other's whole, so a window's place and size never come from two processes. |
| `Recent { cap, key }` | A most-recently-used list: entries this store added or used again lead, entries it removed stay removed, the other's follow in their order, cut to `cap`. |
| `Entries { key }` | A list of user entries: additions from both are kept, removals from both hold, an entry both changed takes this store's version, and an added entry follows the entry before it. |

`key` names a list entry; an entry it answers `None` for is dropped, so a value the settings type
cannot read is not carried into the merged list.

```rust,ignore
fn merge_rule(path: &[&str]) -> MergeRule {
    match path {
        ["recent"] => MergeRule::Recent { cap: 10, key: |v| v.as_str().map(str::to_owned) },
        ["window"] => MergeRule::Whole,
        _ => MergeRule::ByKey,
    }
}
```

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

Each save holds an advisory lock, `.config.toml.lock` beside the file
(`SettingsFiles::lock`), from the disk check to the write, so two processes saving at the same
moment (logging out closes every window together) take turns: the later one sees and merges what
the earlier one wrote. A save waits at most 2 seconds for the lock; past that, or when the lock
file cannot be opened, it logs a warning and saves without it, so a stuck process never stops
another from saving. Hand edits and a second instance's unknown keys and comments are preserved
as above. A read (`SettingsStore::read`) takes no lock and writes nothing. Files with nothing to
lock, such as browser storage, keep the default `lock`, which holds none.

## Validation

Before merging (from `AGENTS.md`), plus the wasm check:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --target wasm32-unknown-unknown -p craft-settings
```

The native tests in `tests/` write only to temporary folders and never touch the real config
folder. `tests/local_storage.rs` runs only in a browser: `cargo test -p craft-settings --target
wasm32-unknown-unknown` with a matching `wasm-bindgen-test-runner` as the target's runner.

On `wasm32` the whole store is available: `SettingsStore`, `OpenOptions`, `SharedSettings`,
`paths` (`Location`, `Probe`, `Resolved`), `migrate` and `LegacyFile`, the `Settings` trait, the
document helpers, the error types, `Remembered`, `presets` (`PresetFolder`, with entries kept in
the app's settings sections) and `recovery::backup_path`. Native only: `DiskFiles`, `profiles`,
the rest of `recovery` and `testing`.

In a browser `OpenOptions::default()` stores files in localStorage through
`LocalStorageFiles::new(4 << 20)`: each file is one item keyed by its path, so
`Location::Dir(dir)` names the key prefix, and `Location::Auto` keeps settings in memory, as a
browser has no folders. A stored item over the cap reads as absent (the store uses defaults and
the next save replaces it); a write over the cap fails and leaves the old item. A damaged item is
moved aside to `config.aside-<n>.toml`, as `std::time::SystemTime` does not work in a browser
to give the native timestamp name. A `LegacyFile` is read through the same files, so an app's old localStorage item imports
into a fresh store.
