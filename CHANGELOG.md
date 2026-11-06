# Changelog

All crates share the workspace version. See [the versioning policy](docs/versioning.md).

## v0.1.0 — 2026-10-12

First coordinated Git release of craft-libs. Applications can pin a named baseline instead of
following development commits. This is an initial 0.x release: compatibility is maintained
within `0.1.x`; changes requiring consumer migration use a new minor version.

### Included crates

- **craft-atomic:** same-directory staged file replacement, exclusive creation, streaming
  writes, transient rename retries and fault-injection support, extracted from PhotoCraft.
- **craft-settings:** app-owned TOML settings with platform/portable paths, atomic saves,
  backups, damaged-file recovery, schema migrations and preservation of unknown fields.
- **craft-quit:** native macOS termination, optional egui close handling and an optional
  application menu. Save decisions and prompts stay with the app.
- **craft-print:** CUPS and native Windows PDF submission, printer discovery, copies, duplex,
  colour and scaling options. Includes bounded PDF sheet handling and the pinned Windows
  completion fix. PDF generation stays with the app.
- **craft-ui:** egui icon buttons, tabs, splits, resize handles, toolbar overflow, transactional
  docking, theme helpers and print-settings controls. Commands, content, localization and
  application state stay with the consumer. Docking serialization is opt-in through `serde`.

### Requirements and adoption

- Rust **1.95+**, edition 2024. Public UI integrations use **egui 0.36**.
- Use `git = "https://github.com/storytold/craft-libs", tag = "v0.1.0"` for each required crate;
  use the same tag everywhere. The [README](README.md#use-a-released-version) has a complete
  manifest example and a commit-pin alternative. Commit application lockfiles.
- No automatic migration from an arbitrary earlier Git revision is implied. Compare the API
  you used with this release and test affected workflows before changing pins.
- For `craft-quit`, run the close hook last in `eframe::App::logic`, after the app's unsaved-work
  decision. Finish asynchronous saves before requesting close; termination does not execute
  code after `run_native`. See the crate's API documentation before adoption.
- This release is distributed through Git only. No crates.io packages are published by it.

### Platform coverage and known limits

The release gate checks the workspace on Linux, macOS and Windows with default and all features,
plus Linux minimum-compiler and UI/settings WASM checks. UI tests include offscreen visual
regressions. Windows print tests exercise a temporary paused spooler queue; those checks do
not establish physical printer fidelity or every application's native GUI behavior.

- `craft-ui` supports native and WASM use. Font coverage, bidirectional text and screen-reader
  behavior still depend on the application and upstream stack; see [#17](https://github.com/storytold/craft-libs/issues/17).
- `craft-settings` persistence is native-only. WASM exposes settings traits, document helpers
  and error types. Concurrent writers are best-effort, last-writer-wins; strict coordination
  needs an application-owned lock.
- `craft-atomic` does not preserve all filesystem metadata during replacement. Directory sync
  is best effort; tests do not prove physical power-loss durability. See its README's contract.
- `craft-quit`'s native behavior is macOS-specific; other platforms keep their normal close
  path. Earlier hardware testing of the close hook is recorded in
  [#3](https://github.com/storytold/craft-libs/issues/3); it does not qualify every consuming app.
- `craft-print` requires CUPS command-line tools on Linux/macOS or the native Windows spooler.
  Browser submission is unsupported. Windows accepts a bounded subset of print-ready PDFs;
  unsupported inputs and driver paper/scaling mismatches return errors. Printing is synchronous,
  so call it from a worker. Physical output needs hardware testing.
- Windows printing retains a pinned Git dependency on the maintained `winprint` fork. Its
  build dependencies require the license/advisory policy exceptions documented in
  [craft-print's README](crates/print/README.md). Registry publication is deferred.

Planned crates and open feature PRs are not part of this release. Existing commit pins remain
valid, and consumers do not need to update simply because `main` changes.
