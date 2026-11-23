# Changelog

All crates share the workspace version. See [the versioning policy](docs/versioning.md).

## v1.0.5

- **Release Notes:** [GitHub Release v1.0.5](https://github.com/ussoewwin/craft-libs/releases/tag/v1.0.5)

### 2026-11-23 — Upstream Merge (`next` + `main`) and Picker Sample Fix

- **Merged `storytold/craft-libs` into the fork's `next`:** `next` at `a4e08cdaf`, on top of a
  fork line that already carried `upstream/main` (the Camera Raw filter merge, `c2a11e702`), so one
  checkout satisfies every revision `photocraft` pins — `8d436f486a` for the shared crates and the
  `craft-develop` pair from the `main` line.
- **Fixed: picker samples skip the private-use areas** ([`ab0873b44`](https://github.com/ussoewwin/craft-libs/commit/ab0873b44)):
  a patched face such as a Nerd Font maps its icons to `U+E000..U+F8FF` first, so the "first glyphs
  the face has" sample became icon codepoints, and laying it out in the interface face panicked
  epaint (`No glyph for U+F030 (U+F030) in Proportional`). Samples now take only characters a face
  shares with the interface face.
- **Merge resolutions that keep both sides:**
  - `craft-font-manager/picker/rank.rs`: the fork's kana sample now runs after upstream's
    `inked` glyph test, so a face that lists kana codepoints without outlines no longer presents
    itself as a kana face.
  - workspace `Cargo.toml`: upstream's new members (`craft-png`, `craft-cdr`) and the fork's
    `craft-win32` both stay registered.
  - `craft-audio/src/track.rs`: upstream's E-AC-3 and unsupported-codec decoder tests adopted on
    top of the fork's changes.

---

## [v1.0.4] — 2026-11-22

- **Release Notes:** [GitHub Release v1.0.4](https://github.com/ussoewwin/craft-libs/releases/tag/v1.0.4)

### Fork Technical Specification & Automated Preservation Infrastructure

- **Comprehensive Fork Specification Architecture (`md/fork-technical-specification.md`):**
  - Documented all 6 core fork pillars: Space key Hand tool isolation (`activate_on_space`, `with_space_activation`), ZIP 64-bit EOCD64 parser (`crates/zip`), PSD layer record and mask preservation (`craft-psd`), centralized CJK font classification (`craft-font-manager`), Windows directory flush durability (`craft-atomic`), and Lama Cleaner CUDA integration.
  - Formulated strict preservation invariants across all format decoders and UI runtime components.
- **Automated Upstream Synchronization & Merge Protection Protocol:**
  - Codified upstream merge rules ensuring custom fork crates (`crates/zip`) are never overwritten or deleted by automated synchronization.
  - Mandated automated x64 test suite verification (`craft-ui-canvas`, `craft-ui-widgets`, `craft-ui-panels`, `craft-psd`, `craft-zip`) as non-negotiable gates for upstream merges.
- **Upstream UI Breaking Change Harmonization & egui Synchronization:**
  - Verified runtime compatibility with upstream UI memoization contracts (`MemoProps`) and accessibility tree validation (`Context::with_accesskit_parents` via `storytold/egui:8a3c5ecb29`).
  - Validated zero-regression downstream compilation and release build parity with `photocraft-ui-egui`.

---

## [v1.0.3] — 2026-11-22

- **Release Notes:** [GitHub Release v1.0.3](https://github.com/ussoewwin/craft-libs/releases/tag/v1.0.3)

### Architectural Harmonization & Upstream Overlap Resolution

- **Title Bar & Menu Bar Layout Alignment (`craft-ui-shell`, `craft-menu`, `craft-ui-widgets`):**
  - Upstream (`storytold/craft-libs:next`) modernized the title bar layout pipeline via `TitleBar::show_titled`, making title width allocation automatic within the layout container.
  - Retired custom fork methods (`TitleBar::leading_width`, `MenuBar::own_width`, `MenuBarUi::width`) and obsolete regression test `menu_bar_room.rs`, restoring 100% code parity with upstream layout contracts and eliminating merge conflicts.
- **Content-Sized Container Inline Header Sizing (`craft-ui-contract`):**
  - Reverted custom inline header reservation hooks in `body/size.rs`, `body/regions.rs`, and `body/hooks.rs` to clean upstream baseline, utilizing upstream's unified container sizing pass.
- **Tool Rail Localization Architecture Evolution (`craft-ui-panels`):**
  - Downstream PhotoCraft transitioned tool command translation directly to the official `CommandState::title` system in `framework.rs`.
  - Retired custom `RailLabels` closures (`ToolRail::labels`, `FlyoutButton::labels`) in `craft-ui-panels`.
  - Preserved critical keyboard policy: `FlyoutButton` retains explicit `with_space_activation()`, keeping the `Space` key strictly dedicated to the canvas Hand/Pan tool while allowing focused buttons to activate cleanly.
- **Consolidated CJK Font Classification Single Source of Truth (`craft-font-manager`):**
  - Redundant classification heuristics formerly maintained in `photocraft/crates/text` (`font_class.rs`) were completely consolidated into `craft-font-manager/rank.rs`.
  - Added `localized_families` fallback: automatically evaluates native Japanese font names (e.g. `ＭＳ 明朝`, `游ゴシック`) when ASCII family names do not trigger classification rules.
  - Ensures workspace-wide single source of truth for CJK classification (Mincho, Gothic, Maru Gothic, Kaisho, Pop) with Photoshop 1:1 Japanese name parity.
- **Upstream Synchronization & Verification:**
  - Synchronized latest upstream changes (including PRs #2756, #2838).
  - All test suites passing with zero warnings: `craft-font-manager` (60 passed), `craft-print` (51 passed), `craft-ui-contract` (396 passed), `craft-ui-shell` (128 passed), `craft-menu` (3 passed), `craft-ui-panels` (234 passed).

---

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
