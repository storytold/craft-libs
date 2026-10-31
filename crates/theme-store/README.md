# craft-theme-store

The Craft suite's theme files, the machine-local theme library and the shared suite selection,
outside rendering. Several apps can change the selection and the library at the same time
without losing a change: every write takes a lock, re-reads the file, merges and bumps a
revision. No egui; the theme contract (ids, roles, built-ins, resolution) is
[`craft-theme`](../theme/README.md). [docs/suite-themes.md](../../docs/suite-themes.md#file-format-and-storage)
describes the file format, the folder and the protocol.

```rust,ignore
use craft_theme::ThemeId;
use craft_theme_store::{SelectionChange, Store, StoreError, StoreOptions, SuiteDir};

// Desktop shells only; app constructors, tests and web builds use SuiteDir::memory().
let mut store = Store::open(&SuiteDir::resolve(false), StoreOptions::new("pdfcraft 0.15.0"));
let _ = store.load_selection(); // a problem keeps the last accepted state
// What the app shows while following the suite: chosen fields, its own defaults for the rest.
let shown = store.accepted().suite.with_defaults(&app_defaults);

// The user picks a theme: only that field is written. base is the accepted state the UI showed.
let base = store.accepted().clone();
match store.update(&base, SelectionChange::theme(ThemeId::parse("craft.graphite")?)) {
    Ok(accepted) => { /* show accepted.suite.with_defaults(&app_defaults) */ }
    Err(StoreError::Conflict { disk, mine }) => { /* "Changed in another Craft app"; "Apply mine" sends `mine` with `disk` as base */ }
    Err(e) => { /* back to store.accepted(), show e with Retry */ }
}
```

| Item | What it is |
|---|---|
| `SuiteDir` | `CRAFT_SUITE_DIR` (empty: in memory), in memory for isolated apps, else `ArtCraft/themes` in the local config folder |
| `Store` | `load_selection`, `update`, `import`, `delete`, `export`, `library`, `load_theme`, `theme` (a `resolve` lookup) |
| `SelectionChange` | the fields the user changed (`theme`, `mode`); only these are written |
| `SuiteChoice` | the suite's `theme` and `mode`, each `None` until a user chooses it; `with_defaults` fills in the app's own |
| `Accepted` | `suite: SuiteChoice`, `revision`, `library_revision`; serde with feature `serde`, for the copy apps keep in their preferences |
| `StoreError` | `Busy`, `Conflict { disk }`, `IdExists`, `InUse`, … ; nothing on disk changed |
| `Problem` | damaged, too new or unreadable files; the store keeps its last accepted state |
| `parse_theme`, `theme_to_json`, `ThemeRecord` | theme file v1; unknown keys stay in the stored bytes |
| `SuiteWatch`, `Snapshot`, `Op`, `OpOutput` | the store on a worker thread: snapshots by generation, queued operations with results |
| `FileStamp`, `library_files`, `selection_stamp`, `theme_stamp`, `parse_counts` | cheap change checks for a watcher |

`Store` methods block on file I/O and on the lock (up to 2 s). Apps use `SuiteWatch`, which owns
the store on a worker thread, instead of calling them on the UI thread:

```rust,ignore
use craft_theme_store::{Op, StoreOptions, SuiteDir, SuiteWatch};

let options = StoreOptions::new("pdfcraft 0.15.0").last_accepted(prefs.last_suite.clone());
let watch = SuiteWatch::open(&SuiteDir::resolve(false), options, move || ctx.request_repaint());

// Each frame: one atomic load; rebuild only when the key changed. `effective` is
// `choice.effective(Some(&snapshot.accepted.suite.with_defaults(&defaults)), &defaults)`,
// recomputed when the generation or the app's own choice changes.
let key = (watch.generation(), effective.clone(), ctx.theme());
if key != self.theme_key {
    let snapshot = watch.snapshot();
    let resolution = craft_theme::resolve(&effective, system, fallback, &|id| snapshot.theme(id));
    // tokens and styles from `resolution`
}

// The user picks a theme: apply it locally, then send it.
let id = watch.request(Op::Update { base: snapshot.accepted.clone(), change });
// Later frames: snapshot.result(id) is Ok, or an error with the accepted state to go back to.
```

The worker stats `selection.json` every 500 ms and compares its bytes every 10 s, parses only
what changed, re-lists the library only when its revision changes or on `Op::Rescan`, and calls
`wake` only when the snapshot changes. `SuiteWatch::memory()` runs without a thread and without
`wake` calls (the caller is in its frame already); on `wasm32` that is the only form. [docs/suite-themes.md](../../docs/suite-themes.md#watching-for-changes)
has the details.

In memory, each `Store` has its own state: open one per process and share it.

Feature `testing` adds `SAMPLE_THEME_JSON` with its expected colours (`sample_roles`), for each
app's adapter test, and `FailingFs`, a file system that fails one step of a write.

The lock file and revision protocol, the re-check before the rename, the failure handling and the
stamp-plus-bytes change detection follow the theme store in [pdfcraft#1090](https://github.com/storytold/pdfcraft/pull/1090) by
mjshfrvstj-sys.
