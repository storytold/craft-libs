# craft-theme-store

The Craft suite's theme files, the machine-local theme library and the shared suite selection,
outside rendering. Several apps can change the selection and the library at the same time
without losing a change: every write takes a lock, re-reads the file, merges and bumps a
revision. No egui; the theme contract (ids, the theme model and file format, built-ins, the
selection and its resolution) is
[`craft-theme`](../theme/README.md). [docs/suite-themes.md](../../docs/suite-themes.md#file-format-and-storage)
describes the file format, the folder and the protocol.

```rust,ignore
use craft_theme_store::{SelectionChange, Store, StoreError, StoreOptions, SuiteDir};

// Desktop shells only; app constructors, tests and web builds use SuiteDir::memory().
let mut store = Store::open(&SuiteDir::resolve(false), StoreOptions::new("pdfcraft 0.15.0"));
let _ = store.load_selection(); // a problem keeps the last accepted state
// What the app shows while following the suite: chosen fields, its own defaults for the rest.
let shown = store.accepted().suite.with_defaults(&app_defaults);

// The user picks a theme: only its slots are written (both for a two-variant theme, else its own
// mode's). base is the accepted state the UI showed.
let base = store.accepted().clone();
match store.update(&base, SelectionChange::pick_theme(&picked)) {
    Ok(accepted) => { /* show accepted.suite.with_defaults(&app_defaults) */ }
    Err(StoreError::Conflict { disk, mine }) => { /* "Changed in another Craft app"; "Apply mine" sends `mine` with `disk` as base */ }
    Err(e) => { /* back to store.accepted(), show e with Retry */ }
}
```

| Item | What it is |
|---|---|
| `SuiteDir` | `CRAFT_SUITE_DIR` (empty: in memory), in memory for isolated apps, else `ArtCraft/themes` in the local config folder |
| `Store` | `load_selection`, `update`, `import`, `delete`, `export`, `library`, `load_theme`, `theme` (a `resolve` lookup) |
| `SelectionChange` | the fields the user changed (`theme_light`, `theme_dark`, `mode`, `contrast`, `size`, `text_size`); only these are written, each merged on its own |
| `Accepted` | `suite` (craft-theme's `SuiteChoice`: each field `None` until a user chooses it), `revision`, `library_revision`; serde, for the copy apps keep in their preferences |
| `StoreError` | `Busy`, `Conflict { disk }`, `IdExists`, `InUse`, … ; nothing on disk changed |
| `Problem` | damaged, too new or unreadable files; the store keeps its last accepted state |
| `ThemeRecord`, `ThemeEntry`, `format` | a parsed user theme file (`craft-design-theme` v1, craft-theme's `format`) with its stored bytes, unknown keys included; the library skips files in another format, such as the pilot's `craft-theme` |
| `SuiteClient` | what an app calls: per-frame `sync`, the optimistic `shown` choice, `change`, conflict and failure notices with `apply_mine` and `retry`, the `theme` lookup, `preview` for any theme's picker card |
| `SuiteWatch`, `Snapshot`, `Op`, `OpOutput` | the store on a worker thread: snapshots by generation, queued operations with results |
| `ui` (feature `ui`) | the Preferences appearance page `AppearancePage` (modes, light and dark theme rows of `ThemeChip`s with favorites, Size and UI Font Size; tokens `theme_chip.*`); feature `screen-testing` adds egui_kittest input tests of the page |
| `FileStamp`, `library_files`, `selection_stamp`, `theme_stamp`, `parse_counts` | cheap change checks for a watcher |

`Store` methods block on file I/O and on the lock (up to 2 s). Apps use `SuiteClient` over a
`SuiteWatch`, which owns the store on a worker thread, instead of calling them on the UI thread:

```rust,ignore
use craft_theme_store::{SelectionChange, StoreOptions, SuiteClient, SuiteDir, SuiteWatch};

let options = StoreOptions::new("pdfcraft 0.15.0").last_accepted(prefs.last_suite.clone());
let watch = SuiteWatch::open(&SuiteDir::resolve(false), options, move || ctx.request_repaint());
let mut client = SuiteClient::new(watch); // SuiteClient::memory() in constructors, tests, web

// Each frame: one atomic load unless something changed.
if client.sync() || choice_changed {
    let selection = app_choice.apply(&client.shown().with_defaults(&defaults));
    let shown = resolve_selection(&selection, system, fallback, &defaults, Some(&app), &|id| client.theme(id));
    // tokens from `shown.resolved`; save client.accepted() as prefs.last_suite
}

// The user picks a theme, mode, contrast, size or text size; shown() reflects it at once.
client.change(SelectionChange::pick_theme(&theme));
// Notices: client.conflict() with client.apply_mine(), client.failure() with client.retry()
// (StoreError::Lost when more than 16 other results pushed the answer out between two syncs),
// client.problems() for files the suite can't use.
// Picker cards: client.preview(id) for built-ins and every library theme, no export needed.
```

The worker stats `selection.json` every 500 ms and compares its bytes every 10 s, parses only
what changed, re-lists the library only when its revision changes or on `Op::Rescan`, and calls
`wake` only when the snapshot changes. `SuiteWatch::memory()` runs without a thread and without
`wake` calls (the caller is in its frame already); on `wasm32` that is the only form. [docs/suite-themes.md](../../docs/suite-themes.md#watching-for-changes)
has the details.

In memory, each `Store` has its own state: open one per process and share it.

The `testing` module has `SAMPLE_THEME_JSON` with its pinned colours (`sample_colors`), for each
app's adapter test, and `FailingFs`, a file system that fails one step of a write.

The lock file and revision protocol, the re-check before the rename, the failure handling and the
stamp-plus-bytes change detection follow the theme store in [pdfcraft#1090](https://github.com/storytold/pdfcraft/pull/1090) by
mjshfrvstj-sys.
