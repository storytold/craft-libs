# Settings storage across the Craft apps

How every Craft app stores its settings, and what goes where. The shared code is
[`craft-settings`](../crates/settings/README.md); this page is the policy it implements.
Discussion: [storytold/craft-libs#12](https://github.com/storytold/craft-libs/issues/12).

## What goes where

| Kind | Where | Why |
|---|---|---|
| Settings people might edit | `config.toml` in the config folder | One readable file with comments, atomic saves, backup, recovery, migrations |
| Window and UI state | `ui.ron` / `app.ron` beside `config.toml` | Changes every session; not worth comments or hand edits |
| Caches and models | Platform cache folder (`paths::cache_dir`); `%LOCALAPPDATA%` on Windows so it does not roam | Disposable, large |
| Recovery and autosave files | Platform data folder (`paths::data_dir`) | Not settings; must survive a settings reset |
| Settings that belong to a library or project (LightCraft) | Inside that folder, via `SettingsStore::open_at` | Travels with the library |
| API keys and tokens | The OS keychain | Never in a plain-text file |
| Suite themes: the suite's theme choice and the user's theme library | `ArtCraft/themes` in the local config folder, via `craft-theme-store` ([suite-themes.md](suite-themes.md#file-format-and-storage)) | Shared by every Craft app and machine-local; writes take a lock and merge, unlike `config.toml` |

## The config folder

First match wins. The store reports which rule applied, so apps can show the path in their
about or settings screen.

1. An override passed by the app, such as a `--config-dir` flag
2. `<APP>_CONFIG_DIR`, for example `PHOTOCRAFT_CONFIG_DIR` (one per app, so redirecting one
   app does not move the others). Set but **empty** means isolated: nothing is read or written
   anywhere, the same as DesignCraft's existing behaviour, so tests and agents can never touch a
   user's real settings
3. Portable mode: a marker file next to the executable (next to the `.app` on macOS, since
   writing inside a signed bundle breaks its signature). The default is a file named `portable`
   with settings in `config/` beside it; apps that already ship portable builds keep their own
   names (PhotoCraft: `portable.txt` or `PhotoCraft.portable`, folder `PhotoCraftData`)
4. The platform default:

Apps can also open a store with `Location::Memory` (for a switch like `DESIGNCRAFT_NO_PREFS`):
settings work in memory and nothing touches the disk.

| Platform | Folder |
|---|---|
| Linux / BSD | `$XDG_CONFIG_HOME/<app>/`, or `~/.config/<app>/` if unset |
| macOS | `~/Library/Application Support/<app>/` |
| Windows | `%APPDATA%\<app>\` (Roaming) |

`<app>` is the app's lowercase id (`photocraft`, `designcraft`, …) on every platform, macOS
included (decided in #12). Existing Linux folder names do not change; the Launcher moves from
`ai.storyteller.artcraft-launcher` to `artcraft-launcher`.

If the folder cannot be created or written (a portable marker in `/Applications`, an AppImage
mount), the store opens read-only and says so at startup.

## Files in the folder

- `config.toml`: starts with `schema_version = N`, then one `[section]` per settings area.
  Written on first launch so people can find it. `0600` on Unix.
- `config.toml.bak`: the previous version, refreshed before every save.
- `config.broken-<unix-ts>.toml`: a file that was not valid TOML, moved aside and never deleted.

## Saving and recovery

- Saves happen during use, half a second after the last change, atomically through
  `craft-atomic`. A failed save leaves the old file intact and is retried with backoff; the app
  shows the error.
- A file with a TOML syntax error is moved aside and defaults are used. A file that parses but
  has a value that no longer fits (a renamed theme, a typo in a hand edit) is not treated as
  broken: only that `[section]` falls back to defaults and the app is told which one.
- A file written by a newer build is shown but never overwritten.
- Comments, key order and keys the app does not know survive saves. If two instances of an app
  are open, each re-reads the file before writing so those survive; known keys are
  last-writer-wins.

## Migration

- Schema changes: `Settings::migrate` upgrades the document one version at a time before it is
  parsed. A failed step leaves the file untouched and the store read-only.
- Older per-app files (`prefs.json`, `ui.json`, `*.ron`): imported on the first run that finds no
  `config.toml`, using the app's existing parser. The old file is **left in place**, so rolling
  back to an older build keeps working; apps can remove it a few releases later.

## Where the apps are today

From the audit in #12 (Linux defaults, from source; overrides and portable mode change them).
Before an app moves over, check where it saves on macOS and Windows so the importer finds the old
files there too.

| App | Today | Moving to `craft-settings` |
|---|---|---|
| PhotoCraft | `~/.config/photocraft/preferences.json` + eframe `ui.ron`; atomic saves with retries; portable via `portable.txt`/`PhotoCraft.portable` → `PhotoCraftData/` | Import `preferences.json`; keep `ui.ron`; keep its portable names via `PORTABLE_MARKERS`/`PORTABLE_DIR` |
| DesignCraft | `~/.config/designcraft/prefs.json` + `ui.json`; saves on exit, ignores failures; empty `DESIGNCRAFT_CONFIG_DIR` and `DESIGNCRAFT_NO_PREFS` touch nothing | Import `prefs.json`; keep `ui.json`; gain save-during-use and error reporting; `DESIGNCRAFT_NO_PREFS` → `Location::Memory` |
| EffectCraft | `~/.config/effectcraft/prefs.json`, `shortcuts.json`, `script_settings.json`; atomic | Import all three into sections of one `config.toml` |
| FilmCraft | `~/.local/share/filmcraft/preferences.json`; atomic | Import from the data folder; settings move to the config folder |
| GridCraft | `~/.config/gridcraft/prefs.json` + `ui.json`; saves on exit, ignores failures | Same as DesignCraft |
| LightCraft | global `ui.json`; library `prefs.json`, `presets.json`, `view.json`; atomic, backups, overwrite protection | Global settings via `open`; library settings via `open_at(library_dir)` |
| VectorCraft | `~/.config/vectorcraft/ui.json` with engine preferences inside; atomic, ignores failures | Split engine preferences out into `config.toml`; keep UI state in `ui.json` |
| ArtCraft Launcher | `~/.local/share/ai.storyteller.artcraft-launcher/app.ron` (eframe) | Import from `app.ron`; needs its own tick timer while in the tray |

Adoption order: PhotoCraft and DesignCraft first (closest to and furthest from the target
behaviour), then the rest.

## Not yet

- Reloading settings when the file changes on disk while the app runs
- Removing imported legacy files
- Settings on the web (`wasm32`), where only the trait, document helpers and error types exist
