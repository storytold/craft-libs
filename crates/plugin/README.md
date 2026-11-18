# Craft Plugin

`craft-plugin` is the one WebAssembly plug-in runtime of the Craft apps
([RFC-0046](https://github.com/storytold/craft-contrib/blob/main/rfcs/0046-shared-plugin-runtime.md)):
the sandbox and its limits, the manifest (`craft-plugin.json`), permissions checked against an
app's command registry, the `.craftplugin` bundle, the per-user plug-in store, catalogs and
installing from GitHub releases. It was extracted from FigCraft's `figcraft-plugin` with its tests;
FigCraft's plug-ins (ABI 6, `figcraft-plugin.json`, `figcraft` imports) keep working unchanged.

Each app keeps what is its own: plug-in kinds that touch its document model (pixel filters, live
effects, widgets…), its list of refused commands, running a plug-in as one undo step and rolling
back on failure, and its menus, dialogs and Plug-ins window.

## Boundaries

- Never crashes: the crate denies `unwrap`, `expect`, `panic` and friends, and the workspace
  forbids `unsafe`. Everything a plug-in, a bundle, a catalog or GitHub hands over is untrusted:
  sizes, counts, nesting and string lengths are bounded before anything is allocated or kept.
- No network I/O in the core. Catalogs and GitHub are pure functions over bytes; `fetch` runs
  them over a `Fetch` trait the app implements. The optional `http` feature implements `Fetch`
  for [`craft-update`](../update)'s `Http` (native only; on wasm32 the feature does nothing, so an
  app may enable it for every target). That impl is for the craft-update of the same craft-libs
  revision: an app whose own craft-update comes from another revision has two copies and
  implements `Fetch` on its client itself until it pins one revision for both, when the
  impl applies as it is.
- No app code: an app is an id string, API ranges and a list of kinds (`AppInfo`), a `Host`
  implementation and a `Policy`. It depends on `wasmi`, `serde`, `serde_json`, `sha2`,
  `thiserror` and, in native builds, [`craft-atomic`](../atomic).

## Platforms

The sandbox, manifest, permissions, bundle, source, catalog, GitHub and fetch modules build
everywhere, `wasm32-unknown-unknown` included (wasmi is an interpreter, no JIT;
`cargo check -p craft-plugin --target wasm32-unknown-unknown` checks it). `store` is native only:
the web builds keep plug-ins in browser storage, and record where each came from with the same
`Source` and `SourceRecord` (`craft_plugin::Source`, from the `source` module) that the native
store writes as `source.json`.

## Modules

| Module | What it holds |
|---|---|
| (root) | The sandbox: `Instance`, `run`, `check_module`, `Host`, `Limits`, `RunOutput`, `UiChange`, `PluginError`; the ABI names `IMPORT_MODULE` (`craft`), `LEGACY_IMPORT_MODULE` (`figcraft`), `ENTRY` (`craft_run`), `LEGACY_ENTRY` (`figcraft_run`). `AppInfo` (`api_for`), `KindApi` and `Target`. |
| `manifest` | `Manifest` (`parse`, `from_value`, `command`, `target`), `AppTarget`, `Permissions`, `Access`, `PluginCommand`, `Param`, `ParamKind`, `Panel`; `MANIFEST_FILE`, `LEGACY_MANIFEST_FILE`, `KIND_COMMAND`, `API_VERSION` (6). |
| `permissions` | `check_call`, `check_read_only`, `Policy` (`refuses`, `excepts`), `ALWAYS_DENIED`, `pattern_matches`, `changed`. |
| `bundle` | `Bundle` (`encode`, `decode`, `check`, `check_with`), `open`, `open_with`, `sha256_hex`, `parse_version`, `compare_versions`, `newer`; `BUNDLE_EXTENSION`, `MAX_BUNDLE_BYTES`. |
| `catalog` | `parse_catalog`, `Catalog`, `Package` (`latest_for`, `release`, `check_manifest`), `Release` (`target`), `check_url`, `check_redirect`, `file_url_path`, `HOSTS`. |
| `github` | `parse_repo_spec`, `RepoRef` (`releases_api_url`), `parse_release`, `Release` (`bundle_asset`, `sums_asset`), `parse_sha256sums`, `plan_download`, `RepoDownload`, `verify_sha256`. |
| `fetch` | `Fetch` (`get`, `read_local`), `fetch_catalog`, `download_release`, `resolve_repo`, `download_repo`, `read_location`, `read_local_file`, `follow_redirects`, `MAX_REDIRECTS`. |
| `source` | `Source` (`File`, `Catalog`, `Github`, `Other`; `check`), `SourceRecord`: on every target. |
| `store` | `Store` (`install_for`, `install_with_for`, `load_for`, `list_for`; `install`, `install_with`, `load`, `list`, `remove`), `Listing`, `Installed`, `Problem`, `plugins_dir`, `folder_name`; `Source` and `SourceRecord` again. |
| `widget` | FigCraft's widget descriptions (`WidgetRender`), which share the ABI. |

## The manifest

```json
{
  "id": "io.github.example.palette-tools",
  "name": "Palette tools",
  "version": "1.2.0",
  "author": "Example",
  "license": "MIT OR Apache-2.0",
  "repo": "https://github.com/example/palette-tools",
  "main": "palette_tools.wasm",
  "api": 6,
  "kind": "command",
  "apps": {
    "figcraft": { "api": 6 },
    "designcraft": { "api": 6, "minApp": "0.30.0" }
  },
  "permissions": { "access": "write", "commands": ["select.*", "swatch.*"] },
  "commands": [{ "id": "grid", "label": "Swatch grid…" }]
}
```

`kind` defaults to `command`. `apps.<id>.api` is compared with the API range the app reports for
that kind (`AppInfo::api_for`): `AppInfo::api` for command plug-ins (the command ABI,
`1..=API_VERSION`) and for every kind `AppInfo::kind_api` doesn't list; an app's own kinds with
their own ABI (PhotoCraft's filters, EffectCraft's effects) list it there, so one `AppInfo`
describes the whole app. A manifest without `apps` is a legacy one: an app that sets
`AppInfo::accept_legacy` (FigCraft) runs it by its top-level `api`; others refuse it. Unknown
fields are ignored, so manifests written for later versions still read.

```rust,ignore
use craft_plugin::{API_VERSION, AppInfo, KIND_COMMAND, KindApi, Manifest};

const APP: AppInfo<'static> = AppInfo {
    id: "photocraft",
    version: env!("CARGO_PKG_VERSION"),
    api: 1..=API_VERSION,                 // command plug-ins
    kinds: &[KIND_COMMAND, "filter"],
    accept_legacy: false,
    kind_api: &[KindApi { kind: "filter", api: 1..=1 }], // `[]` when every kind uses `api`
};

let manifest = Manifest::parse(&text)?;
let target = manifest.target(&APP)?; // Err: a sentence to show
```

The store's `install_for`, `load_for` and `list_for` call `target` themselves; call it directly
for a manifest that isn't installed yet (an install confirmation, a catalog's release with
`Release::target` or `Package::latest_for`).

## Running a plug-in

The app implements `Host` (run a command, return the selection as JSON, notify, log, settings,
`save_file`) and calls `Instance::new(&wasm, &Limits::default())`, then `instance.call(&input, &mut host)`
once per run or panel callback. The input is `{"api", "command", "params", "selection"}` (plus
`event` and `values` from a panel). A trap, a refusal, a non-zero status or an exhausted budget is
an `Err`, and the app rolls the document back.

## Permissions
"The selection" (`Host::selection` and the input's `selection`) is the app's own selection, as
JSON in a shape the app documents: layer or node ids in FigCraft, object handles in CADCraft, A1
ranges in GridCraft, clip ids in FilmCraft. An app with no selection returns `[]`. The crate passes
it through unread; each app documents its shape in its `docs/plugins.md`, so plug-ins written for
several apps know what each hands them.


Inside `Host::call`, before running anything:

```rust,ignore
use craft_plugin::{Policy, check_call, check_read_only};

// Once: the app's own refused commands, and exceptions to them.
let policy = Policy::new(&["doc.*", "version.*", "collab.*", "command.batch"], &["doc.list", "version.list"]);

// An allowlist app refuses everything and lists what it allows, families included.
let allowlist = Policy::new(&["*"], &["select.*", "shape.*", "doc.list"]);
check_call(manifest, &policy, command)?; // ALWAYS_DENIED, the app's list, the manifest's allowlist
check_read_only(manifest, command, registry_says_it_edits(command))?;
```

`ALWAYS_DENIED` (`plugin.*`, `plugins.*`, `edit.undo`, `edit.redo`, `file.*`, `app.*`, `pref.*`,
`window.*`) applies in every app. Patterns, in the denied list, the exceptions and a manifest's
`commands`, are exact ids or prefixes ending in `*` (`pattern_matches`). `Policy::refuses` decides
in this order:

1. `ALWAYS_DENIED` refuses, and nothing reopens it: no exception, not a pattern, not a lone `*`.
2. Otherwise an exception that matches (exact or pattern) allows.
3. Otherwise the app's denied list refuses what it matches.

Then `check_call` applies the manifest's own `commands`. Exceptions were exact ids before they took
patterns; an exact id matches as it did.

**A command `ALWAYS_DENIED` refuses that a plug-in should reach.** Some apps name in-document
commands after a menu (FilmCraft's `file.newSequence`, `file.newBin`, `file.newColorMatte` create
items in the open project and touch no disk), and `file.*` refuses them. How to treat them is
RFC-0046's open question 6, so the shared list doesn't change per app, and an app can't reopen it.
Until it is settled: leave the command refused, list it in your `docs/plugins.md` as unavailable to
plug-ins and why, and report it on the tracking issue
([storytold/craft-libs#2139](https://github.com/storytold/craft-libs/issues/2139)) with the command
ids, what they do (no disk, no other documents) and whether an alias under another prefix would do.
Avoid working around it with an alias outside `ALWAYS_DENIED` that runs the same command: that
would decide the open question for one app.

**Read access is enforced after the call too.** Registries differ in what they record, so the
crate can't know whether a command edited. Run each call as its own nested undo group, compare the
document before and after (undo history length, a revision counter, or a pointer to an immutable
document), and when it changed, undo that group and return
`check_read_only(manifest, command, true)`'s error to the plug-in. Refuse a call that opens a dialog
the same way. Parameters that name a file (`path`) are the app's to refuse.

## Installing

Three sources, the same checks:

```rust,ignore
use craft_plugin::{Source, bundle, catalog, fetch, github, store::{Store, plugins_dir}};

let store = Store::new(plugins_dir(&settings_dir));

// A catalog.
let (cat, warnings) = fetch::fetch_catalog(&mut http, url)?;
let package = cat.package(id).ok_or("not in the catalog")?;
let release = package.latest_for(&APP).ok_or("no release for this app")?;
let bytes = fetch::download_release(&mut http, url, package, release)?; // size and SHA-256 pinned
let (_, manifest) = bundle::open(&bytes)?;
package.check_manifest(release, &manifest)?;
manifest.target(&APP)?; // before the confirmation; install_for checks it again
// … show the confirmation (permissions, kind, source, SHA-256), then:
store.install_for(&APP, &bytes, Source::Catalog { catalog: url.into(), package: package.id.clone() })?;

// A repository: owner/repo or owner/repo@tag.
let repo = github::parse_repo_spec(typed)?;
let plan = fetch::resolve_repo(&mut http, &repo)?; // the API answer and SHA256SUMS only
let bytes = fetch::download_repo(&mut http, &plan)?;
let (_, manifest) = bundle::open(&bytes)?;
// … confirmation, saying "not in any catalog you use" when it isn't, then:
store.install_for(&APP, &bytes, Source::Github { repo: plan.repo.slug(), tag: plan.repo.tag.clone().unwrap_or_default(), asset: plan.asset })?;

// The Plug-ins window: what this app runs, what it doesn't (and why), and damaged folders.
let listing = store.list_for(&APP);
```

Nothing is written before `Store::install_for`, and it refuses a plug-in `Manifest::target` refuses
for the app (another app's, a newer API, a newer `minApp`) before writing. `list_for` sorts what is
installed into `runnable` (with its `Target`), `not_runnable` (with the reason: show them, so they
can be removed) and `problems`; `load_for` loads one plug-in the app runs. `install`, `install_with`,
`load` and `list` don't look at the app and stay for tools that manage the store as files.

A source of the app's own (FigCraft's team server) is `Source::Other { kind, location }`: `kind` is
1–32 lower-case letters, digits and `-` (`figcraft-server`), `location` is plain text. It is
recorded as `{"type": "other", "kind": …, "location": …}`; `source.json` files written before it
existed read as they are. Downloads are `https://` on GitHub's hosts only
(`catalog::HOSTS`); a release URL must be an asset of the package's `repo`; a local (`file://`)
catalog may name local files. A repository release must publish its bundle's SHA-256 in
`SHA256SUMS` (or GitHub's own asset digest); when both exist they must agree. On an update, show
the confirmation again when `permissions::changed(old, new)`.

The store keeps `<root>/<first 32 hex digits of SHA-256(lower-case id)>/` with the bundle as
`<sha256>.craftplugin` and `source.json`. No name on disk comes from a plug-in. `load` and `list`
check each record against its folder and bundle; `list` reports damaged folders separately.

An app's own kinds use `bundle::open_with`, `Bundle::check_with` and `Store::install_with_for` with
their own module check (it gets the manifest, so one check can tell the kinds apart).

### Local catalogs and files

A `file://` catalog (an author testing an entry) and the local files it names are read through
`Fetch::read_local`. Its default reads the disk (`fetch::read_local_file`); an app that confines
file access (MCP automation roots) overrides it, checks the path and then calls `read_local_file`,
or refuses local catalogs:

```rust,ignore
impl craft_plugin::Fetch for AppFetch {
    fn get(&mut self, url: &str, max: usize) -> Result<Option<Vec<u8>>, String> { self.http.get(url, max) }
    fn read_local(&mut self, path: &std::path::Path, max: usize) -> Result<Vec<u8>, String> {
        file_access::allow(path, Access::Read).map_err(|e| e.to_string())?;
        craft_plugin::fetch::read_local_file(path, max)
    }
}
```

### Redirects

Every redirect should stay `https://` on `catalog::HOSTS` (`catalog::check_redirect`). A client
that can see a redirect before following it walks them with `fetch::follow_redirects` and GETs
where they end; the `http` feature's client does, with a HEAD request per hop
(`Http::redirect_target`). What it doesn't check: the GitHub API's own redirects (a renamed
repository; its answer is only read as release JSON, whose URLs are checked, and the token is
never sent on a redirect), and a server on GitHub's hosts that answers the final GET with another
redirect than it gave the HEAD (followed on HTTPS only). Bundles are pinned by SHA-256 and size
whatever served them.

### Threads

`fetch` blocks until its requests finish, and has no progress or cancelling of its own. Run the
whole install (fetch, download, `bundle::open`) on a worker thread and send the result back to
the UI, which shows the confirmation and then calls `install_for` (a quick local write). To
cancel, make the app's `Fetch` return an error once the person cancels; to show progress, report
it from `Fetch::get`.

## Writing a plug-in

Export `memory` and `craft_run(input_len: i32) -> i32`; import host functions from `craft`. The
ABI table is in the `sandbox` module's documentation (`cargo doc -p craft-plugin`). Publish a
GitHub release with one `.craftplugin` file and a `SHA256SUMS` file listing it.
