# Suite themes

A suite theme is a named set of colours that every Craft app can paint its interface with, so that
one choice in one app can restyle the whole suite. This page is the contract the apps share and
how an app adopts it. It follows skylartaylor's brief on
[pdfcraft#1090](https://github.com/storytold/pdfcraft/pull/1090). PdfCraft, PhotoCraft and the
ArtCraft Launcher are its first consumers.

The contract is deliberately small. Apps keep their own `Tokens`, `egui::Visuals`, layout, fonts,
document and canvas colours and saved preferences. They map the 13 core roles below into their own
tokens through an adapter.

| Crate | Contents |
|---|---|
| [`craft-theme`](../crates/theme/README.md) | Theme ids, roles, themes, derived states, contrast, built-ins, resolution. No egui, no I/O; builds for wasm. |
| [`craft-theme-store`](../crates/theme-store/README.md) | Theme files, the suite folder, the suite selection and its lock/revision protocol, outside rendering. |
| [`craft_ui::theme`](../crates/ui/README.md#theme-mechanisms) | `color32` and `install`: the bridge to egui. |

## Themes and selection

A theme has a stable `ThemeId` (`[a-z0-9][a-z0-9._-]{1,63}`), a display name, a version and a light
variant, a dark variant or both. Renaming a theme keeps its id. Ids starting with `craft.` belong to
the built-ins. User theme ids become file names, so `ThemeId::parse_user` and
`SuiteTheme::from_maps` refuse `craft.` ids and ids whose first dot-separated part is a Windows
device name (`con`, `nul`, `aux`, `prn`, `com1`-`com9`, `lpt1`-`lpt9`).

| Id | Name | Light | Dark |
|---|---|---|---|
| `craft.app-default` | App default | the app's own palette | the app's own palette |
| `craft.artcraft` | ArtCraft | PhotoCraft Pro Light | PhotoCraft Pro Medium |
| `craft.graphite` | Graphite | PdfCraft Light gray | PdfCraft Dark gray |
| `craft.high-contrast` | High contrast | black on white, dark blue accent | white on black, yellow accent |

`builtins()` returns the last three. `craft.app-default` is not a theme with colours: it stands for
each app's current palette, and the settings UI lists it beside the others.

A `Selection` is a theme id and a `Mode` (`System`, `Light` or `Dark`). The suite has at most one
selection, stored in the suite folder's `selection.json`. No `selection.json` means no suite
selection: nobody has picked one yet, and neither `Selection` nor `AppChoice` has a default that
could invent one.

Each app keeps an `AppChoice` in its own preferences: `FollowSuite`, or `Own(Selection)` ("Use a
different theme in this app"). Each app also states its `AppDefaults` (a theme and a mode): what it
shows while it follows the suite and the suite has no selection. Apps normally use `SUITE_DEFAULT`
(`craft.artcraft`); the pilot apps (PdfCraft, PhotoCraft, the launcher) use ArtCraft with
`Mode::Dark`, and office apps may ship Light. `AppChoice::effective(suite, &defaults)` returns the
selection that applies:

| `AppChoice` | Suite selection | Effective selection |
|---|---|---|
| `Own(sel)` | any | `sel` |
| `FollowSuite` | `Some(suite)` | `suite` |
| `FollowSuite` | none | the app's `AppDefaults` |

An app writes `selection.json` only when the user picks a suite theme or mode, never on first
start.

## Core roles

Every variant defines all 13 roles. A user theme that leaves roles out gets them from Graphite of
the same variant, and `from_maps` lists each one it filled.

| Role | Painted as |
|---|---|
| `background` | behind panels: the window, pasteboard or canvas surround |
| `surface` | panels, sidebars, controls at rest |
| `raised` | cards, popups, menus, fields |
| `text`, `text_muted` | primary and secondary text |
| `line` | decorative dividers (no contrast requirement) |
| `accent`, `on_accent` | accent fills and the text or icons on them |
| `selection` | selected rows and items, with `text` on top |
| `focus` | keyboard focus rings |
| `good`, `warn`, `bad` | status marks |

## Derived states

`derive(&roles, variant)` computes the interaction and secondary colours, mixed in encoded sRGB
(`Rgba::lerp_gamma`). Apps may overwrite any field.

| State | Rule |
|---|---|
| `hover`, `pressed` | `surface` 6% / 12% toward `text` in light, 8% / 14% in dark |
| `selected_hover` | `selection` 8% toward `text` |
| `accent_hover`, `accent_pressed` | `accent` moved away from `on_accent`'s luminance in 2% steps, up to 10% / 22%, stopping before a step that takes `on_accent` below 4.5:1 |
| `accent_text` | `accent` toward `text` until it reaches 4.5:1 on background, surface and raised |
| `accent_soft` | `accent` at 14% over `surface` |
| `text_faint` | `text_muted` toward `surface` while it stays at 4.5:1 on background, surface and raised |
| `control_edge` | `line` toward `text` until it reaches 3:1 on background, surface and raised |

Each state keeps the painted pairs it is part of at their requirement. When a step toward `text`
would break one, the state steps away from `text` by the same amount instead. For example,
ArtCraft's dark selection is mid grey, so its `selected_hover` is darker than the selection: a
lighter one would leave `text` below 4.5:1 and `text_muted` below 3.5:1. Built-in colours are never changed to suit a
derived state.

## Contrast

`contrast::painted_pairs()` lists every pair the apps paint from the roles and derived states.
Ratios are WCAG 2; a translucent foreground is composited over the background first. Text needs
4.5:1 (WCAG AA). Muted text needs 4.5:1 on the resting backgrounds and 3.5:1 on the hover, pressed
and selected state backgrounds, where it sits only while the pointer or the selection is there.

| Foreground | Background | Need |
|---|---|---|
| `text` | `background`, `surface`, `raised`, `hover`, `pressed`, `selection`, `selected_hover` | 4.5:1 |
| `text_muted` | `background`, `surface`, `raised` | 4.5:1 |
| `text_muted` | `hover`, `pressed`, `selection`, `selected_hover` | 3.5:1 |
| `text_faint` | `background`, `surface`, `raised` | 4.5:1 |
| `on_accent` | `accent`, `accent_hover`, `accent_pressed` | 4.5:1 |
| `accent_text` | `background`, `surface`, `raised` | 4.5:1 |
| `focus`, `control_edge`, `accent` | `surface`, `raised`, `background` | 3:1 |
| `good`, `warn`, `bad` | `background`, `surface` | 3:1 |

`contrast::diagnose(&theme)` returns the pairs that miss their need. Diagnostics are advisory: a
user or imported theme keeps its colours and can still be imported and selected, and the app shows
the diagnostics. The built-in themes have none; a test enforces that. `contrast::check` tests pairs
an app paints outside this table.

## Resolution

`resolve(&selection, system, fallback, &lookup)` is pure. `system` is the OS's light/dark answer
(`None` when it gives none), `fallback` is the app's variant for that case, and `lookup` finds a
theme by id (the built-ins, or the store's library).

| Selection | Result | `Fallback` note |
|---|---|---|
| `Mode::System`, OS answers | the OS's variant | none |
| `Mode::System`, OS silent | the app's fallback variant | `SystemSilent` |
| `Mode::Light` or `Mode::Dark` | that variant | none |
| `craft.app-default` | `Resolution::AppDefault` in the variant above | as above |
| an id `lookup` doesn't know | `Resolution::AppDefault` in the variant above, never another named theme | `UnknownTheme` |
| a theme with only the other variant | `Resolution::Theme` in the variant it has | `OnlyVariant` |

A `Resolved` theme carries the roles of the chosen variant and their derived states.

## Adopting suite themes in an app

There is no shared trait and no shared context key. Each app adds to its own theme module:

1. `Tokens::app_default(variant)`: its existing palette, unchanged.
2. `Tokens::from_resolved(&Resolved)`: maps the core roles and derived states onto its token fields.
3. `install(ctx, light: Tokens, dark: Tokens)`: builds its own `Style` for each token set and calls
   `craft_ui::theme::install(ctx, light_style, dark_style)`, which sets both of egui's styles and
   `ThemePreference::System`. With a fixed mode both slots get the chosen variant; with
   `Mode::System` each slot gets its own variant, and egui switches when the OS does, without a
   rebuild. The app stores both token sets in its own state, so `Tokens::get` returns the set for
   `ctx.theme()`.

On Linux and the BSDs, the portal's answer from `craft_platform::appearance::Watcher::current()`
replaces winit's in `raw.system_theme` (in the raw-input hook); `None` from the portal clears it.
Each app sets `Options::fallback_theme`. The shared crates store nothing in egui's context.

An app's saved light/dark preference becomes `Own(craft.app-default, same mode)` on adoption, so
its look doesn't change until the user picks a named theme or turns on "Follow suite". A fresh
install follows the suite and shows the app's `AppDefaults` until the user picks a suite theme.
Each app sets these defaults explicitly for a fresh install and for a legacy preference file, so a
`#[serde(default)]` never moves existing users to the suite.

Each adapter PR carries an exact style comparison (see
[theme-infrastructure.md](theme-infrastructure.md#migration-and-validation)): with
`craft.app-default` the app's `Style` and `Tokens` stay identical.

## File format and storage

`craft-theme-store` keeps the suite selection and the user's theme library in one machine-local
folder that every Craft app reads.

### Folder

`SuiteDir::resolve(app_isolated)` picks it in this order:

1. `CRAFT_SUITE_DIR`. Set but empty: in memory, nothing on disk is read or written.
2. In memory when the app runs without a disk store (`app_isolated`): app constructors, tests,
   portable installs. Only the desktop shells open the folder, so tests and agents never touch it.
3. `ArtCraft/themes` under the local config folder: `~/.config/ArtCraft/themes` on Linux and the
   BSDs, `~/Library/Application Support/ArtCraft/themes` on macOS, `%LOCALAPPDATA%\ArtCraft\themes`
   on Windows.

Web builds are always in memory. An in-memory store runs the same operations as one on disk.

```text
ArtCraft/themes/selection.json                     suite selection and revisions
ArtCraft/themes/selection.lock                     lock file, never deleted
ArtCraft/themes/library/<id>.json                  one user theme per file
ArtCraft/themes/selection.broken-<unix-ts>.json    a damaged selection, moved aside on the next write
```

Reading never creates anything. The folder and `selection.json` appear with the first write: the
user choosing a suite theme or mode, or importing a theme.

### Theme files

```json
{ "schema": "craft-theme", "schema_version": 1,
  "id": "user.navy-cream", "name": "Navy and cream", "version": 3,
  "light": { "background": "#F2F1EE", "surface": "#FBFAF7", "text": "#1B2340", "…": "…" },
  "dark": { "…": "…" },
  "apps": { "pdfcraft": { "tokens_version": 1, "titlebar": "#101010" } },
  "family": { } }
```

- JSON, at most 256 KiB. `schema_version` is the format; `version` is the author's revision.
- `id` follows the id rules and can't start with `craft.`. A library file is named `<id>.json`.
- At least one of `light` and `dark`. Colours are `#RRGGBB` or `#RRGGBBAA`.
- A role that is missing, or whose colour doesn't parse, is filled from Graphite of the same
  variant and listed as a `MissingRole`. The stored file keeps the original value.
- `apps` and `family` are reserved and not applied yet. They and every other key the app doesn't
  know stay in the file byte for byte.
- A file with a newer `schema_version` is refused, never changed.

A theme picker's diagnostic count for a theme is its `MissingRole` count plus its contrast
`Diagnostic` count. Neither blocks an import or a selection.

### Selection file

```json
{ "schema_version": 1, "revision": 17, "library_revision": 4,
  "suite": { "theme": "craft.artcraft", "mode": "system" }, "written_by": "pdfcraft 0.15.0" }
```

`suite.theme` and `suite.mode` are each optional: a field is present once a user has chosen it in
some app. `Accepted::suite` is a `SuiteChoice { theme: Option<ThemeId>, mode: Option<Mode> }`,
empty while there is no file, no `suite` or neither field. An app that follows the suite shows
`suite.with_defaults(&app_defaults)`: the chosen fields, and its own fresh-install default for
the rest. A write contains only fields a user chose, so one app's default never reaches the
others: picking a theme on a fresh install writes no mode. Keys the app doesn't know are kept on
every write.

With feature `serde`, `Accepted` and `SuiteChoice` serialize, for the copy each app keeps in its
preferences (`last_suite`).

### Writes: lock, merge, revision

Reads take no lock: every write replaces the file with an atomic rename. Each write:

1. Opens `selection.lock` and takes an exclusive lock (`File::try_lock`), retrying every 20 ms
   for up to 2 s, then gives up with `Busy`. On a file system without locks (`ENOLCK`,
   `Unsupported`) it writes without one and relies on the re-check in step 4.
2. Reads `selection.json`. A damaged file, or one over 64 KiB, is replaced by this write if it
   writes anything: it is moved to `selection.broken-<unix-ts>.json` just before the new file is
   written (and put back if that fails). The caller's base stands in for it, with the damaged
   file's `revision` and `library_revision` when it still parses as JSON, so they never go
   backwards. A file from a newer build gives `TooNew`, is never changed, and the store stays
   read-only.
3. Merges (below). A conflict returns `Conflict { disk }` and writes nothing.
4. Writes the result with `revision` one above the file's, carrying the file's unknown keys,
   through craft-atomic with private permissions. Just before the rename it reads the file again;
   if it changed (a writer that ignored the lock, possible on network file systems), the write is
   abandoned with `Busy`.
5. Releases the lock and returns the new `Accepted`.

`Store::update(base, change)` takes the state the user's change was made on and a
`SelectionChange`: the fields the user changed (`theme`, `mode`; their new values are "mine"
below). Only these are written, and only they can conflict. Per field:

| Field | Disk | Result |
|---|---|---|
| not changed | anything | disk's value, chosen or not |
| changed | not chosen | mine |
| changed | equal to mine | disk's value (nothing to write for it) |
| changed | another value, equal to base's | mine |
| changed | another value, not base's | conflict: another app chose this field after the caller read |

A mode-only change therefore keeps the theme on disk, and two apps on a fresh install that pick
a theme and a mode at the same time both land. A change naming a library theme that no longer
exists is `NotFound`. On a conflict the app shows the suite's choice with "Changed in another
Craft app; your choice wasn't applied" and "Apply mine", which sends the same change again
(returned in `Conflict { disk, mine }`) with `disk` as base.

Library operations take the same lock. They set `library_revision` to the file's value plus one,
so concurrent imports never collapse into one bump, and leave `suite` as it is (absent stays
absent).

- **Import** validates the file, writes its original bytes to `library/<id>.json` without ever
  replacing a file, and bumps `library_revision`. A taken id gives `IdExists`; "import as copy"
  stores it as the first free `<id>-2`, `<id>-3`, … and changes nothing but the `id` value.
- **Delete** refuses the suite selection's theme (`InUse`) and built-ins. It renames the file to
  `library/.<id>.json.deleting` (listings skip hidden names), writes `selection.json`, then
  removes it; a failed write renames it back. Per-app choices naming a deleted theme fall back to
  the app default, as unknown ids always do.
- **Export** returns a library theme's stored bytes (at most 256 KiB), or a built-in written out
  as a theme file.

Ids a user couldn't create (the `craft.` prefix, Windows device names such as `con`) are
`NotFound` for every library operation and never reach the file system.

When a write fails, everything it changed is undone (an imported file is removed, a deleted one
restored) and the error comes back; the app returns to `Store::accepted()` and offers Retry.

### Last accepted state

The store keeps the last state it read or wrote. A damaged, too new or unreadable
`selection.json` comes back from `load_selection` as a `Problem` and leaves that state in place.
Each app also copies it into its own preferences and passes it to `StoreOptions::last_accepted`,
so a bad file at startup still shows the last look.

### Watching for changes

Apps don't call `Store` directly. `SuiteWatch::open(dir, options, wake)` (or `start(dir, wake)`
with default options) owns the store on a worker thread named `craft-theme-watch` and publishes
what it finds as an immutable `Snapshot`:

| Field | What it holds |
|---|---|
| `accepted` | the selection as last read or written; problems never replace it |
| `index` | the library themes (id, name, variants, diagnostic count, stamp), sorted by file name |
| `active` | the suite selection's library theme, the last version that parsed; `None` for built-ins |
| `app_theme` | the library theme named with `Op::AppTheme`, for an app's own choice; `None` once its file is deleted, so the app falls back to its default |
| `problems` | damaged, too new or unreadable files; each listed once |
| `results` | the last 16 operation results by request id |
| `generation` | goes up by one with each new snapshot |

What the worker reads, and when:

- `selection.json`: its stamp (length, modification time, file identity on Unix) every 500 ms,
  parsed only when the stamp changed; every 10 s its bytes are read and compared, which catches a
  replacement that kept length and modification time. Unchanged bytes are never parsed again.
- The library: listed only when `library_revision` changes or on `Op::Rescan`, which an app sends
  when a theme picker opens. Only files whose stamp changed are read; the index of the rest is
  kept. Library files are written once, so a hand edit in place that keeps length and
  modification time goes unnoticed.
- The suite's theme and the app's own library theme: their stamps on every 500 ms check, parsed
  only when they changed. A deleted or broken file becomes a problem; the suite's theme keeps its
  last good version, the app's own theme clears when deleted.
- I/O errors (another process holding a file for a moment) are retried on the next check. An
  unreadable `selection.json` is reported only when the retry fails too.

Each change publishes a new snapshot and calls `wake` (the app's `request_repaint`) once; a check
that finds nothing new does neither. `wake` runs on the worker and must not block or call back
into the watch.

Each frame the app calls `generation()`, one atomic load, and compares it with its cache key
`(generation, effective selection, effective variant)`. Only when the key changed does it take
`snapshot()`, resolve with `Snapshot::theme` as the lookup, and rebuild its tokens and styles.

Changes go through `request(Op)`, which queues the operation and returns its id at once:
`Update { base, change }`, `Import`, `Delete`, `Export` (the bytes come back as
`OpOutput::Exported`), `Rescan` and `AppTheme`. The app applies the change to its own state
optimistically, then looks for `snapshot().result(id)`: on `Conflict` the snapshot already holds
the disk state; on another error it holds the accepted state from before. `Rescan` and
`AppTheme` have no result; their effect is the next snapshot, published only if something
changed. Send `AppTheme` when the app's own choice changes.

`SuiteWatch::memory()` and in-memory folders run without a thread: `request` applies the
operation before it returns, with the same snapshots and generations but no `wake` call, since
the caller is already in its frame. On `wasm32` that is the only form. If the worker thread
can't start, the same form runs on the folder with a `Problem::Watcher`: it reads the folder
once and never polls. A check or request that fails inside the worker is reported (a problem,
or an error result) and the worker keeps going.

Dropping a `SuiteWatch` lets queued operations finish and waits at most 100 ms for the worker; a
worker still blocked on the lock then finishes on its own. A queued update that hasn't run when
the process exits is lost, which is one reason apps also keep the user's choice in their own
preferences (`last_suite`).

The lock file and revision protocol, the re-check before the rename, the failure handling and the
stamp-plus-bytes change detection follow the theme store in
[pdfcraft#1090](https://github.com/storytold/pdfcraft/pull/1090).
