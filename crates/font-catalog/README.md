# craft-font-catalog

The installed fonts for the Craft apps: discovery on every platform, lookup by family and style or by any other name, and loading shared between threads.

## What it finds

A `Catalog` reads, in this order:

| Where | Source | Platforms |
|---|---|---|
| The platform's font folders (`font_dirs`) | `System` | all |
| The suite's font store (`font_store_dir`) | `Store` | all |
| The app's own folders (`Catalog::set_app_dirs`, for example `office_font_dirs`) | `App` | all |
| Folders the user or an automation caller picked (`Catalog::set_confined_dirs`) | `User` | all |
| Files the platform's font service knows (`system` feature, through fontique) | `System` | macOS, Windows, Linux, FreeBSD |
| The registry's font list | `System` | Windows |
| Files from injected listers (`Catalog::with_lister`) | `System` | all |

The font folders:

- macOS: `/System/Library/Fonts`, `/Library/Fonts`, `/Network/Library/Fonts`, `~/Library/Fonts`, `/System/Library/Assets` and the font assets macOS downloads on demand (`AssetsV2/com_apple_MobileAsset_Font*`).
- Windows: `%WINDIR%\Fonts` and `%LOCALAPPDATA%\Microsoft\Windows\Fonts`.
- Linux and the BSDs: `/usr/share/fonts`, `/usr/local/share/fonts`, the Flatpak host folders (`/run/host/fonts`, `/run/host/local-fonts`, `/run/host/user-fonts`), `~/.local/share/fonts`, `~/.fonts`, `$XDG_DATA_HOME/fonts`, `$XDG_DATA_DIRS/*/fonts`, and the `<dir>` and `<remap-dir>` entries of the fontconfig configuration with its includes and the user's own. Inside a Flatpak sandbox, also the host's folders from `/run/host/font-dirs.xml` and from the host's configuration under `/run/host/etc/fonts`.
- Haiku: the system's and the user's `data/fonts` folders.

The font service covers fonts outside those folders:

- macOS: CoreText's available fonts, which include fonts that Adobe Fonts, RightFont and other font managers activate from their own folders.
- Windows: DirectWrite's system font collection, which includes per-user fonts and the fonts that font services such as Adobe Fonts load at run time. The registry adds fonts that programs register by full path.
- Linux and FreeBSD: fontconfig, loaded at run time (`fontconfig-dlopen`), so a system without it still runs.

`FontEnv` holds the environment all of this depends on (`HOME`, the XDG variables, `FONTCONFIG_FILE`, `WINDIR`, `LOCALAPPDATA`, `CRAFT_FONT_STORE`, a file system root), so tests can describe any system without touching the process environment.

## The font store

`font_store_dir` is the one per-user folder that the suite's font store installs fonts into and every app's catalog reads. No other program reads it.

| Platform | Folder |
|---|---|
| macOS | `~/Library/Application Support/Craft Fonts` |
| Windows | `%LOCALAPPDATA%\Craft Fonts` |
| Linux, BSDs, Haiku | `$XDG_DATA_HOME/craft-fonts` (`~/.local/share/craft-fonts`) |

`CRAFT_FONT_STORE` replaces it, for portable installs and tests.

## Scanning

The scan reads each file's table directory and its `name`, `OS/2` and `fvar` tables, at most 1 MiB per table, never the whole file. Faces without outline tables (`glyf`, `CFF `, `CFF2`, `VARC`) are left out. Files in folders count when they have a font extension (`.ttf`, `.otf`, `.ttc`, `.otc`); files a font service or lister names count whatever their name. Only regular files are read (after following links), and on Unix the open doesn't wait, so a FIFO or device named `x.ttf` can't block the scan. The scan follows links, except in the folders given to `set_confined_dirs`: there a link whose target is outside the folder is skipped. Mac suitcase fonts are read from their resource fork. Damaged files give no faces, never a panic.

Each face (`FaceInfo`) has its family and style, legacy family and style, family names in other languages, full name, PostScript name, weight, width, italic, a named instance's axis settings, `fsType`, the OS/2 code page bits, its file and index, and its source. Each named instance of a variable font is a face of its own.

## Lookup

- `families()` and `styles(family)`.
- `family(name)` and `resolve(name, style)` accept any name of a family: its own name ignoring ASCII case, its names in other languages and its legacy names ignoring case and anything but letters and digits. `resolve` also accepts a legacy family with its legacy style (`Roboto Light` / `Regular`), a full name, and a PostScript name.
- Within a family, `resolve` matches the style by name (any of its names). Otherwise it picks the nearest weight, with italic differing counting as 1000, then the nearest width. It returns `Match::Exact` or `Match::NearestStyle`, so the app decides about substitutes.
- `by_postscript_name(name)`.

## Loading and threads

- The first scan runs on first use, or on a background thread after `scan_in_background`. Lookups wait for it.
- Lookups read an immutable snapshot, so a rescan never blocks them.
- `load_family` reads a family's files once. Other threads asking for the same family wait and share the result; other families load at the same time. Faces of one file share its bytes (`Arc<[u8]>`).
- `add_bytes` and `add_file` add fonts from downloads, the user or the app's bundle, within `Limits`, for every lookup. Added faces win over installed faces of the same family and style. `remove_added` removes a source's faces. `add_file` refuses a path that isn't a regular file.

## Document scopes

A document's own fonts (embedded, or in a folder beside it) go into a scope, so they change only that document's lookups.

- `new_scope()` returns a `Scope`. `add_bytes_in(scope, …)` and `add_file_in(scope, …)` add faces to it.
- `resolve_in`, `by_postscript_name_in`, `family_in`, `styles_in`, `families_in` and `load_family_in` look up in a scope. Inside a scope, its own faces come first, then the faces added without a scope, then the installed faces. A family the scope has faces of keeps its other styles from the installed fonts.
- Other scopes and the unscoped lookups never see a scope's faces.
- `remove_scope(scope)` removes its faces when the document closes. Other scopes keep theirs, and `remove_added` doesn't touch scoped faces.
- `Limits::max_added_faces` counts the added faces of every scope together.

## Changes

- `rescan` scans again, reading only new and changed files.
- `installed_fonts_changed` compares the modification times of every folder the scan read (and the folders of files a font service named) and, on Windows, the registry's font list. It costs a few hundred `stat` calls, not a scan.
- `rescan_if_changed` does both; apps call it when their window is activated.
- `watch(interval, on_change)` polls on a background thread.
- `generation` counts the changes unscoped lookups see: scans, and faces added or removed without a scope. A scope's changes leave it alone, so other documents don't lay out again. `generation_in(scope)` adds the scope's own changes and never repeats a value, even after `remove_scope`; a document lays out again when it changes.

## One enumeration for the UI and documents

With the `system` feature, `system::collection()` returns the process's fontique `Collection`: the system's fonts and the font store's. The catalog lists its files, and craft-ui's font fallback can query the same collection instead of building its own, so the system's fonts are enumerated once per process. A rescan builds a new collection; `system::generation()` tells holders of an older clone to take a new one.

## Try it

```sh
cargo run -p craft-font-catalog --example fonts             # counts, folders, faces outside them
cargo run -p craft-font-catalog --example fonts -- Arial     # a family's faces
cargo run -p craft-font-catalog --example fonts -- --watch   # report fonts installed or removed
```
