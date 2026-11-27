# Changelog

All crates share the workspace version. See [the versioning policy](docs/versioning.md).

## v1.1.0

- **Release Notes:** [GitHub Release v1.1.0](https://github.com/ussoewwin/craft-libs/releases/tag/v1.1.0)

### 2026-11-27 - The Second Merge (`next`), Space on the Release, and the Listing's Exit

- **Merged `storytold/craft-libs` into the fork's `next`:** `next` at `a46af76c7`, 13 upstream
  commits over `ea14416aa`, merged as `b3e81eb46` with no conflict, the fork's own crates, their
  trials and its Space rule kept.
- **Space activates on the release, and never waits.** `PressRoute::Space` holds the stop
  (`space_held::hold`) and the pass that carries the release activates the held record once, as a
  mouse release clicks; the input hook no longer defers a press for its release, which had returned
  the same pair at the head of the queue on every pass and stalled that viewport's input. Enter stays
  immediate, and the canvas keeps its Space.
- **A failed CUPS listing is a failure, not a system without printers.** The Unix runner returns the
  `lpstat` exit status and its stderr (`output_within_full`); a failed exit is `None` unless it is the
  supported driverless-only answer (`No destinations added`), and a successful empty listing stays an
  empty list, so the dialog reports `DiscoveryFailed` instead of clearing the remembered destination.
- **Upstream's own design-lint red is exempt where its precedent exempts it.** `4062c6aa1` asserts a
  literal `4.0` in `crates/ui-panels/src/dialog/trials.rs`, the shape the two trial rooms already
  exempt: a `test/literal-size` rule in `design-lint.toml` and one exception entry in
  `design/exceptions/craft-ui-panels/dialog.toml`, after which the lint reports 28 passed.
- **The round is written down.** `md/2026-11-27-round-2-merge.md` holds the merge, the three-way
  audit and each finding with its measurement, and section 7 of
  `md/fork-technical-specification.md` states the round's rules and numbers.
- **Suites**: the workspace runs **18,729 tests, 18,728 passed, 63 skipped, 1 failed** - upstream's own
  `craft-xlsx` stack test, measured and named in section 4; format and clippy exit 0.
- See [Release Notes v1.1.0](https://github.com/ussoewwin/craft-libs/releases/tag/v1.1.0) for details.

## v1.0.9

- **Release Notes:** [GitHub Release v1.0.9](https://github.com/ussoewwin/craft-libs/releases/tag/v1.0.9)

### 2026-11-27 - Upstream Merge (`next`), and the macOS Menu Exemption Registered Once

- **Merged `storytold/craft-libs` into the fork's `next`:** `next` at `ea14416aa`, 39 upstream
  commits over `e17227177` (merged in two steps as upstream moved), with the fork's own crates, their
  trials and its Space rule kept.
- **The macOS menu driver's exemption is upstream's, registered once.** Upstream `76322b729` keeps
  `std::time::Instant` behind a `contract/motion` marker and lists the site in
  `design/exceptions/craft-menu/crate.toml`; the fork's own entry named the same site, so
  `design/exceptions/craft-menu/mac.toml` is removed. With both, the lint reported
  `exception/stale` for each file; with one, it reports 28 passed.
- **`Prompt::layer_role` carries upstream's visibility.** `#[prop(vis = "pub(super)")]` includes the
  fork's plain `#[prop]`, and the hand-written setter stays gone.
- **The single suite failure is upstream's own, measured.** `craft-xlsx xlsb::formula::tests::a_long_chain_of_strings_stays_within_the_depth_bound`
  builds a 256 KiB thread for 4,096 chained strings; it aborted in 2 of 5 `cargo nextest run` runs
  here, passed 5 of 5 under `cargo test`, and passed 5 of 5 with the stack raised to 2 MiB in a probe
  that was reverted. The fork holds no change to `crates/xlsx`, and upstream's own Workspace runs on
  `next` (`45864885952`, `5d2c8bdcf`) are red as well.
- **Suites**: the workspace runs **18,723 passed / 1 failed (the test above) / 63 skipped**; the
  design lint reports 28 passed, and clippy and format exit 0.
- See [Release Notes v1.0.9](https://github.com/ussoewwin/craft-libs/releases/tag/v1.0.9) for details.

## v1.0.8

- **Release Notes:** [GitHub Release v1.0.8](https://github.com/ussoewwin/craft-libs/releases/tag/v1.0.8)

### 2026-11-26 - Upstream Merge (`next`), Space on the Controls That Ask, and the Design Lint

- **Merged `storytold/craft-libs` into the fork's `next`:** `next` at `e17227177`, 403 upstream
  commits over `75866f6b1` (merged in five steps as upstream moved), with the fork's own crates, their
  trials and its Space rule kept.
- **Space is routed by the stop that asks for it.** `PressRoute::Space` holds the stop whose Space is
  down (`space_held`, one hold per context) and activates it on the release, as a click does, even
  when the focus moved in between; a stop asks by name or among its activation keys, and every other
  stop leaves Space to the application, where it is the canvas's Hand key.
- **A focused icon button opens its menu from the keyboard.** `IconButton` declares Space among its
  activation keys, as `Button` does, so upstream's menu trials pass with the fork's rule rather than
  against it.
- **The router's Space decision is one function.** `space_release` carries the clear and the
  activation, because the guard's complexity sat over the workspace's clippy limit when the code
  lived inside `begin`.
- **The design lint's four findings on upstream's own tree are answered.** `Prompt::layer_role` is a
  `#[prop]` whose setter the derive writes; the bezier overlay's trial counter and the macOS menu
  driver's retry clock are registered exemptions with their reasons, in
  `design/exceptions/craft-draw/bezier.toml` and `design/exceptions/craft-menu/mac.toml`.
- **The font picker's classification is callable from outside.** `FontClass::classify` exposes the
  name, `sFamilyClass`, PANOSE and fixed-pitch reading, and `FamilyCodec::shown` passes the localized
  names the signature now takes.
- **Suites**: the workspace runs **18,710 passed / 0 failed / 63 skipped**; clippy, format and the
  design lint exit 0.
- See [Release Notes v1.0.8](https://github.com/ussoewwin/craft-libs/releases/tag/v1.0.8) for details.

## v1.0.7

- **Release Notes:** [GitHub Release v1.0.7](https://github.com/ussoewwin/craft-libs/releases/tag/v1.0.7)

### 2026-11-25 - Upstream Merge (`next`) and the Design-Lint Repairs

- **Merged `storytold/craft-libs` into the fork's `next`:** `next` at `6fcf61c81`, 213 upstream
  commits on top of the line this fork follows (444 files), with the fork's own crates, their
  trials and its Space rule kept.
- **Space stays the canvas's Hand in the activation harness.** Upstream's trial loop now walks
  every activation key; the fork's `keys` array still holds Enter alone, so Space is never a path,
  while upstream's sampling, disabled-part filter and shared fixture are taken as they are.
- **The page picture's paper and its document dimensions are declared exemptions.** `Tile::page`
  takes the document's own width and height, in any one unit, and its paper is the blank page of a
  document rather than a theme surface; both carry a `design: allow` marker and an entry in
  `design/exceptions`.
- **A start-screen story's links name catalog icons.** The column story used two names the catalog
  does not carry (`message-circle`, `git-branch`); it now names `message_square` and
  `external_link`, which the catalog's generated list has.
- **Consumer-exposed space allocation goes through `runtime::egui::alloc`.** `alloc::space` is
  public, so a crate outside the contract allocates through the one door the design lint names.
- **Small clippy repairs in the fork's own crates.** Three range loops index a slice they can
  iterate instead (`develop`), a test helper iterates its matrix (`color-transform`), a test module
  sits last in `font-manager/src/picker/rank.rs`, and the reading-view Space trial upstream left
  unregistered is registered and passes (116 trials in that binary).
- The committed components index is regenerated from the registrations.
- **Suites**: `cargo nextest run --workspace` reports **18,485 passed / 0 failed / 63 skipped**
  (exit 0, 1,542 s); `cargo clippy --workspace --all-targets -- -D warnings` exits 0; `cargo fmt --
  --check` exits 0; `craft-design-lint`'s workspace check passes.

## v1.0.6

- **Release Notes:** [GitHub Release v1.0.6](https://github.com/ussoewwin/craft-libs/releases/tag/v1.0.6)

### 2026-11-25 - Upstream Merge (`next`) and Five Repairs

- **Merged `storytold/craft-libs` into the fork's `next`:** `next` at `9f2961402`, 88 upstream
  commits on top of the line this fork follows, with the fork's own crates and their trials kept.
  The workspace-test harness takes upstream's copy whole.
- **A printer listing that fails is no longer read as an empty list.** The platform answers `None`
  when the spooler cannot list its queues; that answer was flattened into an empty list, so a
  transient failure looked like "the printer is gone" and the Print dialog dropped the chosen
  queue. The listing now fails instead: the panel keeps the remembered printer and says the listing
  failed, while a listing that answers - even with no printer in it - still replaces a destination
  it does not name.
- **Without an installed host the library writes English dates and numbers.** Its words were
  English while its date and number formats followed the machine's region, so a calendar named its
  weekdays in the machine's language. Both fall back to `en-US`, which is what the trials assume.
- **A caption's literal backslash survives ffmpeg's ASS reader.** `{}` is plain text to ffmpeg, not
  an override block, so a literal `\N` came back as `\{}N`. The writer emits `\{\i0}`, an override
  block ffmpeg strips, and the crate's own reader reads the same two characters back.
- **Every egui memory access goes through `runtime::egui::memory`.** The held-Space record read and
  wrote the context's memory directly; the design lint names that as a contract violation.
- **`Option::is_some_and` takes its closure by value**, as clippy's
  `needless_borrows_for_generic_args` asks.
- **The browser-POST trial sends the end of the request head**, as a browser does: a host HTTP
  inspection holds an incomplete head and never passes it on. What the trial asserts is unchanged.
- **Suites:** the whole workspace runs **18,373 passed / 0 failed / 57 skipped**; the corpus samples
  run 670 passed / 0 failed / 11 ignored; the design lint's 28 trials run clean.

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
