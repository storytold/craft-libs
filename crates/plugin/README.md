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
  for [`craft-update`](../update)'s `Http` (native only).
- No app code: an app is an id string, an API range and a list of kinds (`AppInfo`), a `Host`
  implementation and a `Policy`. It depends on `wasmi`, `serde`, `serde_json`, `sha2`,
  `thiserror` and, in native builds, [`craft-atomic`](../atomic).

## Platforms

The sandbox, manifest, permissions, bundle, catalog and GitHub modules build everywhere,
`wasm32-unknown-unknown` included (wasmi is an interpreter, no JIT). `store` is native only: the
web builds keep plug-ins in browser storage.

## Modules

| Module | What it holds |
|---|---|
| (root) | The sandbox: `Instance`, `run`, `check_module`, `Host`, `Limits`, `RunOutput`, `UiChange`, `PluginError`; the ABI names `IMPORT_MODULE` (`craft`), `LEGACY_IMPORT_MODULE` (`figcraft`), `ENTRY` (`craft_run`), `LEGACY_ENTRY` (`figcraft_run`). `AppInfo` and `Target`. |
| `manifest` | `Manifest` (`parse`, `from_value`, `command`, `target`), `AppTarget`, `Permissions`, `Access`, `PluginCommand`, `Param`, `ParamKind`, `Panel`; `MANIFEST_FILE`, `LEGACY_MANIFEST_FILE`, `KIND_COMMAND`, `API_VERSION` (6). |
| `permissions` | `check_call`, `check_read_only`, `Policy`, `ALWAYS_DENIED`, `pattern_matches`, `changed`. |
| `bundle` | `Bundle` (`encode`, `decode`, `check`, `check_with`), `open`, `open_with`, `sha256_hex`, `parse_version`, `compare_versions`, `newer`; `BUNDLE_EXTENSION`, `MAX_BUNDLE_BYTES`. |
| `catalog` | `parse_catalog`, `Catalog`, `Package` (`latest_for`, `release`, `check_manifest`), `Release` (`target`), `check_url`, `check_redirect`, `file_url_path`, `HOSTS`. |
| `github` | `parse_repo_spec`, `RepoRef` (`releases_api_url`), `parse_release`, `Release` (`bundle_asset`, `sums_asset`), `parse_sha256sums`, `plan_download`, `RepoDownload`, `verify_sha256`. |
| `fetch` | `Fetch`, `fetch_catalog`, `download_release`, `resolve_repo`, `download_repo`. |
| `store` | `Store` (`install`, `install_with`, `load`, `list`, `remove`), `Source`, `SourceRecord`, `Installed`, `plugins_dir`, `folder_name`. |
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

`kind` defaults to `command`. `apps.<id>.api` is compared with the API range the app reports
(for command plug-ins, the command ABI, `1..=API_VERSION`). A manifest without `apps` is a legacy
one: an app that sets `AppInfo::accept_legacy` (FigCraft) runs it by its top-level `api`; others
refuse it. Unknown fields are ignored, so manifests written for later versions still read.

```rust,ignore
use craft_plugin::{API_VERSION, AppInfo, Manifest};

const APP: AppInfo<'static> =
    AppInfo { id: "designcraft", version: env!("CARGO_PKG_VERSION"), api: 1..=API_VERSION, kinds: &["command"], accept_legacy: false };

let manifest = Manifest::parse(&text)?;
let target = manifest.target(&APP)?; // Err: a sentence to show
```

## Running a plug-in

The app implements `Host` (run a command, return the selection as JSON, notify, log, settings,
`save_file`) and calls `Instance::new(&wasm, &Limits::default())`, then `instance.call(&input, &mut host)`
once per run or panel callback. The input is `{"api", "command", "params", "selection"}` (plus
`event` and `values` from a panel). A trap, a refusal, a non-zero status or an exhausted budget is
an `Err`, and the app rolls the document back.

## Permissions

Inside `Host::call`, before running anything:

```rust,ignore
use craft_plugin::{Policy, check_call, check_read_only};

// Once: the app's own refused commands, and exact exceptions to them.
let policy = Policy::new(&["doc.*", "version.*", "collab.*", "command.batch"], &["doc.list", "version.list"]);

check_call(manifest, &policy, command)?; // ALWAYS_DENIED, the app's list, the manifest's allowlist
check_read_only(manifest, command, registry_says_it_edits(command))?;
```

`ALWAYS_DENIED` (`plugin.*`, `plugins.*`, `edit.undo`, `edit.redo`, `file.*`, `app.*`, `pref.*`,
`window.*`) applies in every app; exceptions never reopen it. Patterns are exact ids or prefixes
ending in `*`.

**Read access is enforced after the call too.** Registries differ in what they record, so the
crate can't know whether a command edited. Run each call as its own nested undo group, compare the
document before and after (undo history length, a revision counter, or a pointer to an immutable
document), and when it changed, undo that group and return
`check_read_only(manifest, command, true)`'s error to the plug-in. Refuse a call that opens a dialog
the same way. Parameters that name a file (`path`) are the app's to refuse.

## Installing

Three sources, the same checks:

```rust,ignore
use craft_plugin::{bundle, catalog, fetch, github, store::{Source, Store, plugins_dir}};

let store = Store::new(plugins_dir(&settings_dir));

// A catalog.
let (cat, warnings) = fetch::fetch_catalog(&mut http, url)?;
let package = cat.package(id).ok_or("not in the catalog")?;
let release = package.latest_for(&APP).ok_or("no release for this app")?;
let bytes = fetch::download_release(&mut http, url, package, release)?; // size and SHA-256 pinned
let (_, manifest) = bundle::open(&bytes)?;
package.check_manifest(release, &manifest)?;
manifest.target(&APP)?;
// … show the confirmation (permissions, kind, source, SHA-256), then:
store.install(&bytes, Source::Catalog { catalog: url.into(), package: package.id.clone() })?;

// A repository: owner/repo or owner/repo@tag.
let repo = github::parse_repo_spec(typed)?;
let plan = fetch::resolve_repo(&mut http, &repo)?; // the API answer and SHA256SUMS only
let bytes = fetch::download_repo(&mut http, &plan)?;
let (_, manifest) = bundle::open(&bytes)?;
// … confirmation, saying "not in any catalog you use" when it isn't, then:
store.install(&bytes, Source::Github { repo: plan.repo.slug(), tag: plan.repo.tag.clone().unwrap_or_default(), asset: plan.asset })?;
```

Nothing is written before `Store::install`. Downloads are `https://` on GitHub's hosts only
(`catalog::HOSTS`); a release URL must be an asset of the package's `repo`; a local (`file://`)
catalog may name local files. A repository release must publish its bundle's SHA-256 in
`SHA256SUMS` (or GitHub's own asset digest); when both exist they must agree. On an update, show
the confirmation again when `permissions::changed(old, new)`.

The store keeps `<root>/<first 32 hex digits of SHA-256(lower-case id)>/` with the bundle as
`<sha256>.craftplugin` and `source.json`. No name on disk comes from a plug-in. `load` and `list`
check each record against its folder and bundle; `list` reports damaged folders separately.

An app's own kinds use `bundle::open_with`, `Bundle::check_with` and `Store::install_with` with
their own module check.

## Writing a plug-in

Export `memory` and `craft_run(input_len: i32) -> i32`; import host functions from `craft`. The
ABI table is in the `sandbox` module's documentation (`cargo doc -p craft-plugin`). Publish a
GitHub release with one `.craftplugin` file and a `SHA256SUMS` file listing it.
